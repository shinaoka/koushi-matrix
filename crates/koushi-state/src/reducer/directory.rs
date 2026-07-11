use crate::{
    effect::{AppEffect, UiEvent},
    state::{
        AppState, DirectoryJoinState, DirectoryQuery, DirectoryQueryState, FocusedContextState,
        ThreadAttentionState, ThreadPaneState, ThreadsListState, TimelinePaneState,
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

pub(crate) fn handle_directory_join_requested(
    state: &mut AppState,
    request_id: u64,
    alias: String,
    via_server: Option<String>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    state.directory.join = DirectoryJoinState::Joining {
        request_id,
        alias,
        via_server,
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
    state.navigation.active_space_id = None;
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
    state.thread = ThreadPaneState::Closed;
    state.thread_attention = ThreadAttentionState::Closed;
    state.threads_list = ThreadsListState::Closed;
    state.focused_context = FocusedContextState::Closed;

    let mut effects = vec![AppEffect::EmitUiEvent(UiEvent::DirectoryChanged)];
    if previous_active_space_id.is_some() {
        effects.push(AppEffect::EmitUiEvent(UiEvent::RoomListChanged));
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
    alias: String,
    via_server: Option<String>,
    kind: crate::state::OperationFailureKind,
) -> Vec<AppEffect> {
    if !matches!(
        &state.directory.join,
        DirectoryJoinState::Joining {
            request_id: current_request_id,
            alias: current_alias,
            via_server: current_via_server,
        } if *current_request_id == request_id
            && *current_alias == alias
            && *current_via_server == via_server
    ) {
        return Vec::new();
    }

    state.directory.join = DirectoryJoinState::Failed {
        request_id,
        alias,
        via_server,
        kind,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::DirectoryChanged)]
}
