use crate::{
    effect::{AppEffect, UiEvent},
    state::{
        AppState, DirectoryJoinState, DirectoryPreviewState, DirectoryQuery, DirectoryQueryState,
        FocusedContextState, ScheduledSendsListState, ThreadAttentionState, ThreadPaneState,
        ThreadsListState, TimelinePaneState,
    },
};

use super::is_session_ready;

pub(crate) fn handle_directory_query_requested(
    state: &mut AppState,
    request_id: u64,
    query: DirectoryQuery,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    state.directory.query = DirectoryQueryState::Querying { request_id, query };
    vec![AppEffect::EmitUiEvent(UiEvent::DirectoryChanged)]
}

pub(crate) fn handle_directory_query_succeeded(
    state: &mut AppState,
    request_id: u64,
    query: DirectoryQuery,
    rooms: Vec<crate::state::DirectoryRoomSummary>,
    next_batch: Option<String>,
) -> Vec<AppEffect> {
    if !matches!(
        &state.directory.query,
        DirectoryQueryState::Querying {
            request_id: current_request_id,
            query: current_query,
        } if *current_request_id == request_id && *current_query == query
    ) {
        return Vec::new();
    }

    state.directory.query = DirectoryQueryState::Results {
        request_id,
        query,
        rooms,
        next_batch,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::DirectoryChanged)]
}

pub(crate) fn handle_directory_query_failed(
    state: &mut AppState,
    request_id: u64,
    query: DirectoryQuery,
    kind: crate::state::OperationFailureKind,
) -> Vec<AppEffect> {
    if !matches!(
        &state.directory.query,
        DirectoryQueryState::Querying {
            request_id: current_request_id,
            query: current_query,
        } if *current_request_id == request_id && *current_query == query
    ) {
        return Vec::new();
    }

    state.directory.query = DirectoryQueryState::Failed {
        request_id,
        query,
        kind,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::DirectoryChanged)]
}

pub(crate) fn handle_directory_preview_requested(
    state: &mut AppState,
    request_id: u64,
    room_id_or_alias: String,
    via_servers: Vec<String>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    state.directory.preview = DirectoryPreviewState::Loading {
        request_id,
        room_id_or_alias,
        via_servers,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::DirectoryChanged)]
}

pub(crate) fn handle_directory_preview_loaded(
    state: &mut AppState,
    request_id: u64,
    room: crate::state::DirectoryRoomPreview,
) -> Vec<AppEffect> {
    let DirectoryPreviewState::Loading {
        request_id: current_request_id,
        room_id_or_alias,
        via_servers,
    } = &state.directory.preview
    else {
        return Vec::new();
    };
    if *current_request_id != request_id {
        return Vec::new();
    }

    state.directory.preview = DirectoryPreviewState::Ready {
        request_id,
        room_id_or_alias: room_id_or_alias.clone(),
        via_servers: via_servers.clone(),
        room,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::DirectoryChanged)]
}

pub(crate) fn handle_directory_preview_failed(
    state: &mut AppState,
    request_id: u64,
    room_id_or_alias: String,
    via_servers: Vec<String>,
    kind: crate::state::OperationFailureKind,
) -> Vec<AppEffect> {
    if !matches!(
        &state.directory.preview,
        DirectoryPreviewState::Loading {
            request_id: current_request_id,
            room_id_or_alias: current_target,
            via_servers: current_via_servers,
        } if *current_request_id == request_id
            && *current_target == room_id_or_alias
            && *current_via_servers == via_servers
    ) {
        return Vec::new();
    }

    state.directory.preview = DirectoryPreviewState::Failed {
        request_id,
        room_id_or_alias,
        via_servers,
        kind,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::DirectoryChanged)]
}

pub(crate) fn handle_directory_preview_dismissed(state: &mut AppState) -> Vec<AppEffect> {
    if state.directory.preview == DirectoryPreviewState::Closed {
        return Vec::new();
    }

    state.directory.preview = DirectoryPreviewState::Closed;
    vec![AppEffect::EmitUiEvent(UiEvent::DirectoryChanged)]
}

pub(crate) fn handle_directory_join_requested(
    state: &mut AppState,
    request_id: u64,
    room_id_or_alias: String,
    via_servers: Vec<String>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    // The decision has been made; the preview must not outlive it.
    state.directory.preview = DirectoryPreviewState::Closed;
    state.directory.join = DirectoryJoinState::Joining {
        request_id,
        room_id_or_alias,
        via_servers,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::DirectoryChanged)]
}

pub(crate) fn handle_directory_join_succeeded(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
) -> Vec<AppEffect> {
    if !matches!(
        &state.directory.join,
        DirectoryJoinState::Joining {
            request_id: current_request_id,
            ..
        } if *current_request_id == request_id
    ) {
        return Vec::new();
    }

    let previous_active_space_id = state.navigation.active_space_id.clone();
    let had_thread = state.thread != ThreadPaneState::Closed
        || state.thread_attention != ThreadAttentionState::Closed;
    let had_threads_list = state.threads_list != ThreadsListState::Closed;
    state.directory.join = DirectoryJoinState::Idle;
    // #1160: a successful join leaves the directory and selects the joined room,
    // so it closes the scoped scheduled-sends panel like a room selection.
    state.scheduled_sends_list = ScheduledSendsListState::Closed;
    state.navigation.active_space_id = None;
    let space_members_changed = super::space_members::handle_selected(state, None);
    let space_children_changed = super::space_children::handle_selected(state, None);
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
        continuity: Default::default(),
    };
    state.thread = ThreadPaneState::Closed;
    state.thread_attention = ThreadAttentionState::Closed;
    state.threads_list = ThreadsListState::Closed;
    state.focused_context = FocusedContextState::Closed;

    let mut effects = vec![AppEffect::EmitUiEvent(UiEvent::DirectoryChanged)];
    if previous_active_space_id.is_some() {
        effects.push(AppEffect::EmitUiEvent(UiEvent::RoomListChanged));
    }
    if space_members_changed {
        effects.push(AppEffect::EmitUiEvent(UiEvent::SpaceMembersChanged));
    }
    if space_children_changed {
        effects.push(AppEffect::EmitUiEvent(UiEvent::SpaceChildrenChanged));
    }
    effects.push(AppEffect::SubscribeTimeline {
        room_id: room_id.clone(),
    });
    effects.push(AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }));
    if had_thread {
        effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadChanged));
    }
    if had_threads_list {
        effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadsListChanged));
    }
    effects
}

pub(crate) fn handle_directory_join_failed(
    state: &mut AppState,
    request_id: u64,
    room_id_or_alias: String,
    via_servers: Vec<String>,
    kind: crate::state::OperationFailureKind,
) -> Vec<AppEffect> {
    if !matches!(
        &state.directory.join,
        DirectoryJoinState::Joining {
            request_id: current_request_id,
            room_id_or_alias: current_target,
            via_servers: current_via_servers,
        } if *current_request_id == request_id
            && *current_target == room_id_or_alias
            && *current_via_servers == via_servers
    ) {
        return Vec::new();
    }

    state.directory.join = DirectoryJoinState::Failed {
        request_id,
        room_id_or_alias,
        via_servers,
        kind,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::DirectoryChanged)]
}
