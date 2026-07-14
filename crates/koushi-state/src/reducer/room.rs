use std::collections::BTreeSet;

use crate::{
    effect::{AppEffect, UiEvent},
    state::{
        AppError, AppState, OperationFailureKind, PinOp, PinOperationState, PinnedEvent,
        RoomListFilter, RoomSummary, RoomTagInfo, RoomTagKind, SpaceSummary, ThreadAttentionState,
        ThreadPaneState, ThreadsListState, TimelinePaneState,
    },
};

use super::{
    active_room_left_selected_space, apply_space_order,
    avatar::{collect_known_avatar_thumbnails, preserve_avatar_thumbnail},
    first_default_room_id, has_session_projection_context, is_session_ready,
    preferred_room_id_in_active_space, recompute_room_list_projection, reconcile_space_order,
    refresh_timeline_media_gallery, refresh_timeline_scheduled_sends,
    refresh_timeline_upload_staging, retain_navigation_room_memory,
    retarget_active_room_for_selected_space, room_exists,
    select_active_room_after_room_list_update, session_user_id,
};

const PIN_EVENT_FAILED_MESSAGE: &str = "Pinning the event failed";
const UNPIN_EVENT_FAILED_MESSAGE: &str = "Unpinning the event failed";

pub(crate) fn handle_room_list_updated(
    state: &mut AppState,
    spaces: Vec<crate::state::SpaceSummary>,
    rooms: Vec<crate::state::RoomSummary>,
) -> Vec<AppEffect> {
    if !has_session_projection_context(state) {
        return Vec::new();
    }

    let own_user_id = session_user_id(state).map(str::to_owned);
    let mut rooms = rooms;
    let mut spaces = spaces;
    preserve_known_avatar_thumbnails(state, &mut spaces, &mut rooms);
    suppress_stale_unread_after_local_read(state, &mut rooms);
    crate::state::refresh_room_summary_display_projection(
        &mut rooms,
        &state.profile,
        own_user_id.as_deref(),
    );
    let retained_room_ids = rooms
        .iter()
        .map(|room| room.room_id.clone())
        .collect::<BTreeSet<_>>();
    let had_active_room_before_update = state.navigation.active_room_id.is_some();
    reconcile_space_order(&mut state.navigation.space_order, &spaces);
    apply_space_order(&mut spaces, &state.navigation.space_order);
    state.spaces = spaces;
    state.rooms = rooms;
    retain_navigation_room_memory(state);
    recompute_room_list_projection(state);
    state.composer_drafts.retain_rooms(&retained_room_ids);
    state.scheduled_sends.retain_rooms(&retained_room_ids);
    state.upload_staging.retain_rooms(&retained_room_ids);
    state.media_gallery.retain_rooms(&retained_room_ids);
    refresh_timeline_scheduled_sends(state);
    refresh_timeline_upload_staging(state);
    refresh_timeline_media_gallery(state);

    let mut effects = vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)];

    // Notify the search crawler of all current joined rooms on every
    // RoomListUpdate so it can idempotently start/resume any missing
    // crawls. The actor is responsible for deduplication.
    {
        use crate::state::SearchCrawlerSpeed;
        let crawler_settings = &state.settings.values.search_crawler;
        if crawler_settings.speed != SearchCrawlerSpeed::Paused {
            let room_ids: Vec<String> = state.rooms.iter().map(|r| r.room_id.clone()).collect();
            if !room_ids.is_empty() {
                effects.push(AppEffect::NotifySearchCrawlerRoomsAvailable {
                    room_ids,
                    settings: crawler_settings.clone(),
                });
            }
        }
    }

    if state
        .navigation
        .active_space_id
        .as_deref()
        .is_some_and(|active_space_id| {
            !state
                .spaces
                .iter()
                .any(|space| space.space_id == active_space_id)
        })
    {
        state.navigation.active_space_id = None;
    }

    if let Some(active_room_id) = state.navigation.active_room_id.clone() {
        let room_still_exists = state
            .rooms
            .iter()
            .any(|room| room.room_id == active_room_id);

        if !room_still_exists {
            state.navigation.active_room_id = None;
            let previous_room_id = state.timeline.room_id.clone().unwrap_or(active_room_id);
            let had_thread = state.thread != ThreadPaneState::Closed
                || state.thread_attention != ThreadAttentionState::Closed;
            let had_threads_list = state.threads_list != ThreadsListState::Closed;

            state.timeline = Default::default();
            state.thread = ThreadPaneState::Closed;
            state.thread_attention = ThreadAttentionState::Closed;
            state.threads_list = ThreadsListState::Closed;

            effects.push(AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: previous_room_id,
            }));
            if had_thread {
                effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadChanged));
            }
            if had_threads_list {
                effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadsListChanged));
            }
        }
    }

    if had_active_room_before_update
        && state.navigation.active_room_id.is_none()
        && state.navigation.active_space_id.is_some()
        && let Some(room_id) = preferred_room_id_in_active_space(state)
    {
        select_active_room_after_room_list_update(state, &mut effects, room_id);
    }

    if let Some(active_room_id) = state.navigation.active_room_id.clone()
        && active_room_left_selected_space(state, &active_room_id)
    {
        retarget_active_room_for_selected_space(state, &mut effects, active_room_id);
    }

    if let Some(active_room_id) = state.navigation.active_room_id.clone()
        && state.timeline.room_id.as_deref() != Some(active_room_id.as_str())
        && room_exists(state, &active_room_id)
    {
        select_active_room_after_room_list_update(state, &mut effects, active_room_id);
    }

    if !had_active_room_before_update && state.navigation.active_room_id.is_none() {
        let next_room_id = if state.navigation.active_space_id.is_some() {
            preferred_room_id_in_active_space(state)
        } else {
            first_default_room_id(state)
        };
        if let Some(room_id) = next_room_id {
            state.navigation.active_room_id = Some(room_id.clone());
            state.timeline = TimelinePaneState {
                room_id: Some(room_id.clone()),
                is_subscribed: false,
                is_paginating_backwards: false,
                composer: state.composer_drafts.composer_for_room(&room_id),
                submission_registry: state.timeline.submission_registry.clone(),
                scheduled_send_capability: state.scheduled_sends.capability.clone(),
                scheduled_sends: state.scheduled_sends.items_for_room(&room_id),
                staged_uploads: state.upload_staging.items_for_room(&room_id),
                media_gallery: state.media_gallery.items_for_room(&room_id),
                media_downloads: Default::default(),
            };
            effects.push(AppEffect::SubscribeTimeline {
                room_id: room_id.clone(),
            });
            effects.push(AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }));
        }
    }

    recompute_room_list_projection(state);
    effects
}

fn suppress_stale_unread_after_local_read(state: &AppState, rooms: &mut [RoomSummary]) {
    for room in rooms {
        if !room_has_unread_metrics(room) {
            continue;
        }
        let Some(existing_room) = state
            .rooms
            .iter()
            .find(|candidate| candidate.room_id == room.room_id)
        else {
            continue;
        };
        if room_has_unread_metrics(existing_room) {
            continue;
        }
        let fully_read_event_id_present = state
            .live_signals
            .rooms
            .get(&room.room_id)
            .and_then(|signals| signals.fully_read_event_id.as_deref())
            .is_some();
        if !fully_read_event_id_present || !same_room_activity(existing_room, room) {
            continue;
        }
        room.marked_unread = false;
        room.unread_count = 0;
        room.notification_count = 0;
        room.highlight_count = 0;
    }
}

fn room_has_unread_metrics(room: &RoomSummary) -> bool {
    room.unread_count > 0
        || room.notification_count > 0
        || room.highlight_count > 0
        || room.marked_unread
}

fn same_room_activity(left: &RoomSummary, right: &RoomSummary) -> bool {
    left.recency_stamp == right.recency_stamp
        && left.conversation_activity == right.conversation_activity
        && latest_event_id(left) == latest_event_id(right)
}

fn latest_event_id(room: &RoomSummary) -> Option<&str> {
    room.latest_event
        .as_ref()
        .map(|event| event.event_id.as_str())
}

fn preserve_known_avatar_thumbnails(
    state: &AppState,
    spaces: &mut [SpaceSummary],
    rooms: &mut [crate::state::RoomSummary],
) {
    let known_thumbnails = collect_known_avatar_thumbnails(state, false);

    for room in rooms {
        preserve_avatar_thumbnail(&known_thumbnails, &mut room.avatar);
    }
    for space in spaces {
        preserve_avatar_thumbnail(&known_thumbnails, &mut space.avatar);
    }
}

pub(crate) fn handle_room_list_filter_selected(
    state: &mut AppState,
    filter: RoomListFilter,
) -> Vec<AppEffect> {
    if !is_session_ready(state) || state.room_list.active_filter == filter {
        return Vec::new();
    }

    state.room_list.active_filter = filter;
    recompute_room_list_projection(state);
    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
}

pub(crate) fn handle_room_list_filter_applied(
    state: &mut AppState,
    projection: crate::state::RoomListProjection,
) -> Vec<AppEffect> {
    if !is_session_ready(state) || state.room_list == projection {
        return Vec::new();
    }

    state.room_list = projection;
    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
}

pub(crate) fn handle_room_tags_updated(
    state: &mut AppState,
    room_id: String,
    tags: crate::state::RoomTags,
) -> Vec<AppEffect> {
    if !has_session_projection_context(state) {
        return Vec::new();
    }

    let Some(room) = state.rooms.iter_mut().find(|room| room.room_id == room_id) else {
        return Vec::new();
    };

    if room.tags == tags {
        return Vec::new();
    }

    room.tags = tags;
    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
}

pub(crate) fn handle_room_tag_set(
    state: &mut AppState,
    room_id: String,
    tag: RoomTagKind,
    info: RoomTagInfo,
) -> Vec<AppEffect> {
    if !has_session_projection_context(state) {
        return Vec::new();
    }

    let Some(room) = state.rooms.iter_mut().find(|room| room.room_id == room_id) else {
        return Vec::new();
    };

    let mut tags = room.tags.clone();
    tags.set(tag, info);
    if room.tags == tags {
        return Vec::new();
    }

    room.tags = tags;
    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
}

pub(crate) fn handle_room_tag_removed(
    state: &mut AppState,
    room_id: String,
    tag: RoomTagKind,
) -> Vec<AppEffect> {
    if !has_session_projection_context(state) {
        return Vec::new();
    }

    let Some(room) = state.rooms.iter_mut().find(|room| room.room_id == room_id) else {
        return Vec::new();
    };

    let mut tags = room.tags.clone();
    tags.remove(tag);
    if room.tags == tags {
        return Vec::new();
    }

    room.tags = tags;
    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
}

pub(crate) fn handle_room_pinned_events_updated(
    state: &mut AppState,
    room_id: String,
    pinned: Vec<PinnedEvent>,
) -> Vec<AppEffect> {
    if !has_session_projection_context(state) {
        return Vec::new();
    }

    let entry = state.room_interactions.entry(room_id).or_default();
    if entry.pinned_events == pinned {
        return Vec::new();
    }

    entry.pinned_events = pinned;
    vec![AppEffect::EmitUiEvent(UiEvent::RoomInteractionsChanged)]
}

pub(crate) fn handle_pin_event_requested(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    event_id: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state) || event_id.is_empty() || !room_exists(state, &room_id) {
        return Vec::new();
    }

    let entry = state.room_interactions.entry(room_id.clone()).or_default();
    if !entry.pin_operation.accepts_new_request() {
        return Vec::new();
    }

    entry.pin_operation = PinOperationState::Pending {
        request_id,
        room_id,
        event_id,
        op: PinOp::Pin,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::RoomInteractionsChanged)]
}

pub(crate) fn handle_pin_event_completed(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
) -> Vec<AppEffect> {
    if !has_session_projection_context(state) {
        return Vec::new();
    }

    let Some(entry) = state.room_interactions.get_mut(&room_id) else {
        return Vec::new();
    };
    if !matches!(
        entry.pin_operation,
        PinOperationState::Pending {
            request_id: pending_request_id,
            op: PinOp::Pin,
            ..
        } if pending_request_id == request_id
    ) {
        return Vec::new();
    }

    entry.pin_operation = PinOperationState::Idle;
    vec![AppEffect::EmitUiEvent(UiEvent::RoomInteractionsChanged)]
}

pub(crate) fn handle_pin_event_failed(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    _kind: crate::state::OperationFailureKind,
) -> Vec<AppEffect> {
    if !has_session_projection_context(state) {
        return Vec::new();
    }

    let Some(entry) = state.room_interactions.get_mut(&room_id) else {
        return Vec::new();
    };
    let PinOperationState::Pending {
        request_id: pending_request_id,
        event_id,
        op: PinOp::Pin,
        ..
    } = &entry.pin_operation
    else {
        return Vec::new();
    };
    if *pending_request_id != request_id {
        return Vec::new();
    };
    let event_id = event_id.clone();

    entry.pin_operation = PinOperationState::Failed {
        room_id,
        event_id,
        op: PinOp::Pin,
        recoverable: true,
    };
    state.errors.push(AppError {
        code: "pin_event_failed".to_owned(),
        message: PIN_EVENT_FAILED_MESSAGE.to_owned(),
        recoverable: true,
    });
    vec![
        AppEffect::EmitUiEvent(UiEvent::RoomInteractionsChanged),
        AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
    ]
}

pub(crate) fn handle_unpin_event_requested(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    event_id: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state) || event_id.is_empty() || !room_exists(state, &room_id) {
        return Vec::new();
    }

    let entry = state.room_interactions.entry(room_id.clone()).or_default();
    if !entry.pin_operation.accepts_new_request() {
        return Vec::new();
    }

    entry.pin_operation = PinOperationState::Pending {
        request_id,
        room_id,
        event_id,
        op: PinOp::Unpin,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::RoomInteractionsChanged)]
}

pub(crate) fn handle_unpin_event_completed(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
) -> Vec<AppEffect> {
    if !has_session_projection_context(state) {
        return Vec::new();
    }

    let Some(entry) = state.room_interactions.get_mut(&room_id) else {
        return Vec::new();
    };
    if !matches!(
        entry.pin_operation,
        PinOperationState::Pending {
            request_id: pending_request_id,
            op: PinOp::Unpin,
            ..
        } if pending_request_id == request_id
    ) {
        return Vec::new();
    }

    entry.pin_operation = PinOperationState::Idle;
    vec![AppEffect::EmitUiEvent(UiEvent::RoomInteractionsChanged)]
}

pub(crate) fn handle_unpin_event_failed(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    _kind: crate::state::OperationFailureKind,
) -> Vec<AppEffect> {
    if !has_session_projection_context(state) {
        return Vec::new();
    }

    let Some(entry) = state.room_interactions.get_mut(&room_id) else {
        return Vec::new();
    };
    let PinOperationState::Pending {
        request_id: pending_request_id,
        event_id,
        op: PinOp::Unpin,
        ..
    } = &entry.pin_operation
    else {
        return Vec::new();
    };
    if *pending_request_id != request_id {
        return Vec::new();
    };
    let event_id = event_id.clone();

    entry.pin_operation = PinOperationState::Failed {
        room_id,
        event_id,
        op: PinOp::Unpin,
        recoverable: true,
    };
    state.errors.push(AppError {
        code: "unpin_event_failed".to_owned(),
        message: UNPIN_EVENT_FAILED_MESSAGE.to_owned(),
        recoverable: true,
    });
    vec![
        AppEffect::EmitUiEvent(UiEvent::RoomInteractionsChanged),
        AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
    ]
}

pub(crate) fn handle_room_marked_as_read_requested(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    event_id: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state) || !room_exists(state, &room_id) {
        return Vec::new();
    }

    let _ = (request_id, event_id);
    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
}

pub(crate) fn handle_room_marked_as_read_succeeded(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
) -> Vec<AppEffect> {
    if !has_session_projection_context(state) || !room_exists(state, &room_id) {
        return Vec::new();
    }

    let _ = request_id;
    if let Some(room) = state.rooms.iter_mut().find(|room| room.room_id == room_id) {
        room.marked_unread = false;
        room.unread_count = 0;
        room.notification_count = 0;
        room.highlight_count = 0;
        recompute_room_list_projection(state);
    }
    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
}

pub(crate) fn handle_room_marked_as_read_failed(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    kind: OperationFailureKind,
) -> Vec<AppEffect> {
    if !has_session_projection_context(state) || !room_exists(state, &room_id) {
        return Vec::new();
    }

    let _ = (request_id, kind);
    vec![AppEffect::EmitUiEvent(UiEvent::ErrorChanged)]
}

pub(crate) fn handle_room_marked_as_unread_requested(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    unread: bool,
) -> Vec<AppEffect> {
    if !is_session_ready(state) || !room_exists(state, &room_id) {
        return Vec::new();
    }

    let _ = (request_id, unread);
    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
}

pub(crate) fn handle_room_marked_as_unread_succeeded(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    unread: bool,
) -> Vec<AppEffect> {
    if !has_session_projection_context(state) || !room_exists(state, &room_id) {
        return Vec::new();
    }

    let _ = request_id;
    if let Some(room) = state.rooms.iter_mut().find(|room| room.room_id == room_id) {
        room.marked_unread = unread;
        if unread && room.unread_count == 0 {
            room.unread_count = 1;
        }
        if !unread {
            room.unread_count = 0;
        }
        recompute_room_list_projection(state);
    }
    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
}

pub(crate) fn handle_room_marked_as_unread_failed(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    kind: OperationFailureKind,
) -> Vec<AppEffect> {
    if !has_session_projection_context(state) || !room_exists(state, &room_id) {
        return Vec::new();
    }

    let _ = (request_id, kind);
    vec![AppEffect::EmitUiEvent(UiEvent::ErrorChanged)]
}
