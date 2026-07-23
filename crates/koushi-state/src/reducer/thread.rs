use crate::SubmissionId;
use crate::{
    effect::{AppEffect, UiEvent},
    state::{
        AppError, AppState, FocusedContextState, PendingComposerSendKind, ThreadAttentionState,
        ThreadPaneState, ThreadsListState, sort_threads_list_items,
    },
};

use super::is_session_ready;

pub(crate) fn handle_thread_submission_accepted(
    state: &mut AppState,
    submission_id: SubmissionId,
    room_id: String,
    root_event_id: String,
    transaction_id: String,
    draft_revision: u64,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    if state
        .timeline
        .submission_registry
        .accepted_submission_ids
        .contains(&submission_id)
        || state
            .timeline
            .submission_registry
            .settled_submission_ids
            .contains(&submission_id)
    {
        return Vec::new();
    }
    state.timeline.submission_registry.remember_accepted(
        submission_id.clone(),
        transaction_id.clone(),
        crate::ComposerSubmissionTarget::Thread {
            room_id: room_id.clone(),
            root_event_id: root_event_id.clone(),
        },
    );
    let accepted_revision =
        state
            .composer_drafts
            .advance_thread_revision(&room_id, &root_event_id, draft_revision);
    let accepted_composer = state
        .composer_drafts
        .composer_for_thread(&room_id, &root_event_id);
    let ThreadPaneState::Open {
        room_id: open_room_id,
        root_event_id: open_root_event_id,
        composer,
        ..
    } = &mut state.thread
    else {
        return Vec::new();
    };
    if open_room_id != &room_id
        || open_root_event_id != &root_event_id
        || composer.pending_submission_id.is_some()
        || composer.pending_transaction_id.is_some()
        || composer.accepted_submission_ids.contains(&submission_id)
    {
        return Vec::new();
    }
    composer.remember_accepted_submission(submission_id.clone());
    composer.pending_submission_id = Some(submission_id);
    composer.pending_transaction_id = Some(transaction_id);
    composer.pending_send_kind = Some(PendingComposerSendKind::Reply {
        in_reply_to_event_id: root_event_id,
    });
    composer.draft = accepted_composer.draft;
    composer.draft_revision = accepted_revision;
    vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
}

pub(crate) fn handle_thread_composer_draft_changed(
    state: &mut AppState,
    room_id: String,
    root_event_id: String,
    draft: String,
    revision: u64,
) -> Vec<AppEffect> {
    if !is_session_ready(state) || !state.rooms.iter().any(|room| room.room_id == room_id) {
        return Vec::new();
    }
    if !state.composer_drafts.apply_thread_draft(
        room_id.clone(),
        root_event_id.clone(),
        draft.clone(),
        revision,
    ) {
        return Vec::new();
    }

    match &mut state.thread {
        ThreadPaneState::Open {
            room_id: open_room_id,
            root_event_id: open_root_event_id,
            composer,
            ..
        } if open_room_id == &room_id && open_root_event_id == &root_event_id => {
            composer.draft = draft;
            composer.draft_revision = revision;
            vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
        }
        _ => Vec::new(),
    }
}

pub(crate) fn handle_thread_reply_submitted(
    state: &mut AppState,
    room_id: String,
    root_event_id: String,
    transaction_id: String,
    draft_revision: u64,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    match &mut state.thread {
        ThreadPaneState::Open {
            room_id: open_room_id,
            root_event_id: open_root_event_id,
            composer,
            ..
        } if open_room_id == &room_id
            && open_root_event_id == &root_event_id
            && composer.pending_transaction_id.is_none() =>
        {
            composer.pending_transaction_id = Some(transaction_id);
            composer.pending_send_kind = Some(PendingComposerSendKind::Reply {
                in_reply_to_event_id: root_event_id.clone(),
            });
            let accepted_revision = state.composer_drafts.advance_thread_revision(
                &room_id,
                &root_event_id,
                draft_revision,
            );
            let accepted_composer = state
                .composer_drafts
                .composer_for_thread(&room_id, &root_event_id);
            composer.draft = accepted_composer.draft;
            composer.draft_revision = accepted_revision;
            vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
        }
        _ => Vec::new(),
    }
}

pub(crate) fn handle_thread_reply_finished(
    state: &mut AppState,
    room_id: String,
    root_event_id: String,
    transaction_id: String,
) -> Vec<AppEffect> {
    if matches!(
        &state.thread,
        ThreadPaneState::Open { composer, .. } if composer.pending_submission_id.is_some()
    ) {
        return Vec::new();
    }
    if !is_session_ready(state) {
        return Vec::new();
    }

    match &mut state.thread {
        ThreadPaneState::Open {
            room_id: open_room_id,
            root_event_id: open_root_event_id,
            composer,
            ..
        } if open_room_id == &room_id
            && open_root_event_id == &root_event_id
            && composer.pending_transaction_id.as_deref() == Some(transaction_id.as_str()) =>
        {
            composer.pending_transaction_id = None;
            composer.pending_send_kind = None;
            vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
        }
        _ => Vec::new(),
    }
}

pub(crate) fn handle_thread_reply_failed(
    state: &mut AppState,
    room_id: String,
    root_event_id: String,
    transaction_id: String,
    message: String,
) -> Vec<AppEffect> {
    if matches!(
        &state.thread,
        ThreadPaneState::Open { composer, .. } if composer.pending_submission_id.is_some()
    ) {
        return Vec::new();
    }
    if !is_session_ready(state) {
        return Vec::new();
    }

    match &mut state.thread {
        ThreadPaneState::Open {
            room_id: open_room_id,
            root_event_id: open_root_event_id,
            composer,
            ..
        } if open_room_id == &room_id
            && open_root_event_id == &root_event_id
            && composer.pending_transaction_id.as_deref() == Some(transaction_id.as_str()) =>
        {
            composer.pending_transaction_id = None;
            composer.pending_send_kind = None;
            state.errors.push(AppError {
                code: "send_text_failed".to_owned(),
                message,
                recoverable: true,
            });
            vec![
                AppEffect::EmitUiEvent(UiEvent::ThreadChanged),
                AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
            ]
        }
        _ => Vec::new(),
    }
}

pub(crate) fn handle_open_thread(
    state: &mut AppState,
    room_id: String,
    root_event_id: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state) || state.timeline.room_id.as_deref() != Some(room_id.as_str()) {
        return Vec::new();
    }

    state.upload_staging.clear_thread_targets_for_room(&room_id);
    state.thread = ThreadPaneState::Opening {
        room_id: room_id.clone(),
        root_event_id: root_event_id.clone(),
    };
    state.thread_attention = ThreadAttentionState::Tracking {
        room_id: room_id.clone(),
        root_event_id: root_event_id.clone(),
        notification_count: 0,
        highlight_count: 0,
        live_event_marker_count: 0,
    };
    vec![
        AppEffect::OpenThreadTimeline {
            room_id,
            root_event_id,
        },
        AppEffect::EmitUiEvent(UiEvent::ThreadChanged),
    ]
}

pub(crate) fn handle_thread_subscribed(
    state: &mut AppState,
    room_id: String,
    root_event_id: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state)
        || !matches!(
            &state.thread,
            ThreadPaneState::Opening {
                room_id: opening_room_id,
                root_event_id: opening_root_event_id,
            } if opening_room_id == &room_id && opening_root_event_id == &root_event_id
        )
    {
        return Vec::new();
    }

    let staged_uploads = state
        .upload_staging
        .items_for_target(&crate::ComposerTarget::Thread {
            room_id: room_id.clone(),
            root_event_id: root_event_id.clone(),
        });
    state.thread = ThreadPaneState::Open {
        composer: state
            .composer_drafts
            .composer_for_thread(&room_id, &root_event_id),
        room_id,
        root_event_id,
        is_subscribed: true,
        staged_uploads,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
}

pub(crate) fn handle_thread_subscription_failed(
    state: &mut AppState,
    room_id: String,
    root_event_id: String,
    _message: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state)
        || !matches!(
            &state.thread,
            ThreadPaneState::Opening {
                room_id: opening_room_id,
                root_event_id: opening_root_event_id,
            } if opening_room_id == &room_id && opening_root_event_id == &root_event_id
        )
    {
        return Vec::new();
    }

    state.thread = ThreadPaneState::Closed;
    state.thread_attention = ThreadAttentionState::Closed;
    state.errors.push(AppError {
        code: "thread_subscription_failed".to_owned(),
        message: "Matrix thread subscription failed".to_owned(),
        recoverable: true,
    });
    vec![
        AppEffect::EmitUiEvent(UiEvent::ThreadChanged),
        AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
    ]
}

pub(crate) fn handle_thread_attention_updated(
    state: &mut AppState,
    room_id: String,
    root_event_id: String,
    notification_count: u64,
    highlight_count: u64,
    live_event_marker_count: u64,
) -> Vec<AppEffect> {
    if !is_session_ready(state)
        || !matches!(
            &state.thread_attention,
            ThreadAttentionState::Tracking {
                room_id: tracking_room_id,
                root_event_id: tracking_root_event_id,
                ..
            } if tracking_room_id == &room_id
                && tracking_root_event_id == &root_event_id
        )
    {
        return Vec::new();
    }

    let next = ThreadAttentionState::Tracking {
        room_id,
        root_event_id,
        notification_count,
        highlight_count,
        live_event_marker_count,
    };
    if state.thread_attention == next {
        return Vec::new();
    }

    state.thread_attention = next;
    vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
}

pub(crate) fn handle_close_thread(state: &mut AppState) -> Vec<AppEffect> {
    if !is_session_ready(state) || state.thread == ThreadPaneState::Closed {
        return Vec::new();
    }

    if let ThreadPaneState::Open { room_id, .. } | ThreadPaneState::Opening { room_id, .. } =
        &state.thread
    {
        state.upload_staging.clear_thread_targets_for_room(room_id);
    }
    state.thread = ThreadPaneState::Closed;
    state.thread_attention = ThreadAttentionState::Closed;
    vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
}

pub(crate) fn handle_open_focused_context(
    state: &mut AppState,
    room_id: String,
    event_id: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state) || state.timeline.room_id.as_deref() != Some(room_id.as_str()) {
        return Vec::new();
    }

    state.focused_context = FocusedContextState::Opening {
        room_id: room_id.clone(),
        event_id: event_id.clone(),
    };
    vec![AppEffect::OpenFocusedTimeline { room_id, event_id }]
}

pub(crate) fn handle_focused_context_subscribed(
    state: &mut AppState,
    room_id: String,
    event_id: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state)
        || !matches!(
            &state.focused_context,
            FocusedContextState::Opening {
                room_id: opening_room_id,
                event_id: opening_event_id,
            } if opening_room_id == &room_id && opening_event_id == &event_id
        )
    {
        return Vec::new();
    }

    state.focused_context = FocusedContextState::Open {
        room_id,
        event_id,
        is_subscribed: true,
    };
    Vec::new()
}

pub(crate) fn handle_focused_context_subscription_failed(
    state: &mut AppState,
    room_id: String,
    event_id: String,
    _message: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state)
        || !matches!(
            &state.focused_context,
            FocusedContextState::Opening {
                room_id: opening_room_id,
                event_id: opening_event_id,
            } if opening_room_id == &room_id && opening_event_id == &event_id
        )
    {
        return Vec::new();
    }

    state.focused_context = FocusedContextState::Closed;
    state.errors.push(AppError {
        code: "focused_context_subscription_failed".to_owned(),
        message: "Matrix focused context subscription failed".to_owned(),
        recoverable: true,
    });
    vec![AppEffect::EmitUiEvent(UiEvent::ErrorChanged)]
}

pub(crate) fn handle_close_focused_context(state: &mut AppState) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    // #161: jump-to-date renders the focused timeline in the main pane, marked by
    // `main_timeline_anchor`. Closing the focused context therefore also returns
    // the main pane to the live timeline (live-edge control), independent of the
    // right-panel focused-context state. When leaving the anchored view, drop the
    // room's persisted scroll anchor so the live timeline pins to the live edge
    // (bottom) instead of restoring a stale pre-jump scroll position.
    if state.navigation.main_timeline_anchor.take().is_some()
        && let Some(room_id) = state.navigation.active_room_id.clone()
    {
        state.navigation.room_scroll_anchors.remove(&room_id);
    }

    if state.focused_context == FocusedContextState::Closed {
        return Vec::new();
    }
    state.focused_context = FocusedContextState::Closed;
    Vec::new()
}

pub(crate) fn handle_open_threads_list(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    if state.navigation.active_room_id.as_deref() != Some(room_id.as_str()) {
        return Vec::new();
    }
    if state.threads_list.room_id() == Some(room_id.as_str())
        && matches!(state.threads_list, ThreadsListState::Loading { .. })
    {
        return Vec::new();
    }
    state.threads_list = ThreadsListState::Loading {
        room_id: room_id.clone(),
        request_id,
    };
    vec![
        AppEffect::SubscribeThreadsList {
            request_id,
            room_id,
        },
        AppEffect::EmitUiEvent(UiEvent::ThreadsListChanged),
    ]
}

pub(crate) fn handle_threads_list_opened(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    mut items: Vec<crate::state::ThreadsListItem>,
    end_reached: bool,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    if state.threads_list.request_id() != Some(request_id) {
        return Vec::new();
    }
    sort_threads_list_items(&mut items, state.settings.values.thread_list_order);
    state.threads_list = ThreadsListState::Open {
        room_id,
        request_id,
        items,
        is_paginating: false,
        end_reached,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::ThreadsListChanged)]
}

pub(crate) fn handle_threads_list_updated(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    mut items: Vec<crate::state::ThreadsListItem>,
    is_paginating: bool,
    end_reached: bool,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    if state.threads_list.request_id() != Some(request_id) {
        return Vec::new();
    }
    sort_threads_list_items(&mut items, state.settings.values.thread_list_order);
    state.threads_list = ThreadsListState::Open {
        room_id,
        request_id,
        items,
        is_paginating,
        end_reached,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::ThreadsListChanged)]
}

pub(crate) fn handle_threads_list_pagination_completed(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    mut items: Vec<crate::state::ThreadsListItem>,
    end_reached: bool,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    if state.threads_list.request_id() != Some(request_id) {
        return Vec::new();
    }
    sort_threads_list_items(&mut items, state.settings.values.thread_list_order);
    state.threads_list = ThreadsListState::Open {
        room_id,
        request_id,
        items,
        is_paginating: false,
        end_reached,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::ThreadsListChanged)]
}

pub(crate) fn handle_threads_list_failed(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    failure_kind: crate::state::OperationFailureKind,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    if state.threads_list.request_id() != Some(request_id) {
        return Vec::new();
    }
    state.threads_list = ThreadsListState::Failed {
        room_id,
        request_id,
        failure_kind,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::ThreadsListChanged)]
}

pub(crate) fn handle_paginate_threads_list(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    match &mut state.threads_list {
        ThreadsListState::Open {
            room_id: open_room_id,
            request_id: open_request_id,
            is_paginating,
            end_reached,
            ..
        } if open_room_id == &room_id
            && request_id > *open_request_id
            && !*is_paginating
            && !*end_reached =>
        {
            *open_request_id = request_id;
            *is_paginating = true;
        }
        _ => return Vec::new(),
    }
    vec![
        AppEffect::PaginateThreadsList {
            request_id,
            room_id,
        },
        AppEffect::EmitUiEvent(UiEvent::ThreadsListChanged),
    ]
}

pub(crate) fn handle_close_threads_list(state: &mut AppState) -> Vec<AppEffect> {
    if state.threads_list == ThreadsListState::Closed {
        return Vec::new();
    }
    state.threads_list = ThreadsListState::Closed;
    vec![
        AppEffect::UnsubscribeThreadsList,
        AppEffect::EmitUiEvent(UiEvent::ThreadsListChanged),
    ]
}

pub(crate) fn handle_thread_root_projection_observed(
    state: &mut AppState,
    room_id: String,
    root_event_id: String,
    activity_event_id: String,
    activity_timestamp_ms: Option<u64>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    if !state.thread_root_projections.observe(
        room_id,
        root_event_id,
        activity_event_id,
        activity_timestamp_ms,
    ) {
        return Vec::new();
    }
    vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
}

pub(crate) fn handle_thread_root_projection_ready(
    state: &mut AppState,
    room_id: String,
    root_event_id: String,
    activity_event_id: String,
    activity_timestamp_ms: Option<u64>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    state.thread_root_projections.mark_ready(
        room_id,
        root_event_id,
        activity_event_id,
        activity_timestamp_ms,
    );
    vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
}

pub(crate) fn handle_thread_root_projection_failed(
    state: &mut AppState,
    room_id: String,
    root_event_id: String,
    activity_event_id: String,
    activity_timestamp_ms: Option<u64>,
    failure_kind: crate::state::OperationFailureKind,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    state.thread_root_projections.mark_failed(
        room_id,
        root_event_id,
        activity_event_id,
        activity_timestamp_ms,
        failure_kind,
    );
    vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
}

pub(crate) fn handle_thread_root_projections_reconciled(
    state: &mut AppState,
    room_id: String,
    activities: Vec<crate::state::ThreadRootProjectionActivity>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    let before = state.thread_root_projections.clone();
    state
        .thread_root_projections
        .reconcile_room(room_id, activities);
    (before != state.thread_root_projections)
        .then_some(AppEffect::EmitUiEvent(UiEvent::ThreadChanged))
        .into_iter()
        .collect()
}

pub(crate) fn handle_thread_root_projections_cleared(
    state: &mut AppState,
    room_id: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    state
        .thread_root_projections
        .clear_room(&room_id)
        .then_some(AppEffect::EmitUiEvent(UiEvent::ThreadChanged))
        .into_iter()
        .collect()
}
