use std::collections::BTreeSet;

use crate::{
    effect::{AppEffect, UiEvent},
    state::{ActivityMarkReadState, ActivityState, ActivityStream, ActivityTab, AppState},
};

use super::is_session_ready;

pub(crate) fn handle_activity_opened(state: &mut AppState, request_id: u64) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    state.activity = ActivityState::Opening {
        request_id,
        tab: ActivityTab::Recent,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::ActivityChanged)]
}

pub(crate) fn handle_activity_closed(state: &mut AppState) -> Vec<AppEffect> {
    if state.activity == ActivityState::Closed {
        return Vec::new();
    }

    state.activity = ActivityState::Closed;
    vec![AppEffect::EmitUiEvent(UiEvent::ActivityChanged)]
}

pub(crate) fn handle_activity_snapshot_loaded(
    state: &mut AppState,
    request_id: u64,
    active_tab: ActivityTab,
    recent: ActivityStream,
    unread: ActivityStream,
    excluded_room_ids: Vec<String>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let ActivityState::Opening {
        request_id: current_request_id,
        ..
    } = state.activity
    else {
        return Vec::new();
    };
    if current_request_id != request_id {
        return Vec::new();
    }

    let excluded_room_ids: BTreeSet<_> = excluded_room_ids.into_iter().collect();
    state.activity = ActivityState::Open {
        active_tab,
        recent: normalize_activity_stream(recent, &excluded_room_ids),
        unread: normalize_activity_stream(unread, &excluded_room_ids),
        mark_read: ActivityMarkReadState::Idle,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::ActivityChanged)]
}

pub(crate) fn handle_activity_rows_observed(_state: &mut AppState) -> Vec<AppEffect> {
    Vec::new()
}

pub(crate) fn handle_activity_rows_updated(
    state: &mut AppState,
    recent: ActivityStream,
    unread: ActivityStream,
    excluded_room_ids: Vec<String>,
) -> Vec<AppEffect> {
    let ActivityState::Open {
        recent: current_recent,
        unread: current_unread,
        ..
    } = &mut state.activity
    else {
        return Vec::new();
    };

    let excluded_room_ids: BTreeSet<_> = excluded_room_ids.into_iter().collect();
    *current_recent = normalize_activity_stream(recent, &excluded_room_ids);
    *current_unread = normalize_activity_stream(unread, &excluded_room_ids);
    vec![AppEffect::EmitUiEvent(UiEvent::ActivityChanged)]
}

pub(crate) fn handle_activity_tab_selected(
    state: &mut AppState,
    tab: ActivityTab,
) -> Vec<AppEffect> {
    let ActivityState::Open { active_tab, .. } = &mut state.activity else {
        return Vec::new();
    };
    if *active_tab == tab {
        return Vec::new();
    }

    *active_tab = tab;
    vec![AppEffect::EmitUiEvent(UiEvent::ActivityChanged)]
}

pub(crate) fn handle_activity_mark_read_requested(
    state: &mut AppState,
    request_id: u64,
    target: crate::state::ActivityMarkReadTarget,
) -> Vec<AppEffect> {
    let ActivityState::Open { mark_read, .. } = &mut state.activity else {
        return Vec::new();
    };

    *mark_read = ActivityMarkReadState::Pending { request_id, target };
    vec![AppEffect::EmitUiEvent(UiEvent::ActivityChanged)]
}

pub(crate) fn handle_activity_mark_read_succeeded(
    state: &mut AppState,
    request_id: u64,
    cleared_event_ids: Vec<String>,
) -> Vec<AppEffect> {
    let ActivityState::Open {
        unread, mark_read, ..
    } = &mut state.activity
    else {
        return Vec::new();
    };
    if !matches!(
        mark_read,
        ActivityMarkReadState::Pending {
            request_id: current_request_id,
            ..
        } if *current_request_id == request_id
    ) {
        return Vec::new();
    }

    let cleared_event_ids: BTreeSet<_> = cleared_event_ids.into_iter().collect();
    unread
        .rows
        .retain(|row| !cleared_event_ids.contains(&row.event_id));
    *mark_read = ActivityMarkReadState::Idle;
    vec![AppEffect::EmitUiEvent(UiEvent::ActivityChanged)]
}

pub(crate) fn handle_activity_mark_read_failed(
    state: &mut AppState,
    request_id: u64,
    target: crate::state::ActivityMarkReadTarget,
    kind: crate::state::OperationFailureKind,
) -> Vec<AppEffect> {
    let ActivityState::Open { mark_read, .. } = &mut state.activity else {
        return Vec::new();
    };
    if !matches!(
        mark_read,
        ActivityMarkReadState::Pending {
            request_id: current_request_id,
            ..
        } if *current_request_id == request_id
    ) {
        return Vec::new();
    }

    *mark_read = ActivityMarkReadState::Failed {
        target,
        failure_kind: kind,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::ActivityChanged)]
}

// --- Private helpers ---

fn normalize_activity_stream(
    mut stream: ActivityStream,
    excluded_room_ids: &BTreeSet<String>,
) -> ActivityStream {
    stream
        .rows
        .retain(|row| !excluded_room_ids.contains(&row.room_id));
    stream.rows.sort_by(|left, right| {
        right
            .timestamp_ms
            .cmp(&left.timestamp_ms)
            .then_with(|| left.room_id.cmp(&right.room_id))
            .then_with(|| left.event_id.cmp(&right.event_id))
    });
    stream
}
