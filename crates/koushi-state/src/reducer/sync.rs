use crate::{
    effect::{AppEffect, UiEvent},
    state::{AppState, SyncMode, SyncState},
};

use super::is_session_ready;

pub(crate) fn handle_sync_started(state: &mut AppState) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    match state.sync {
        SyncState::Starting | SyncState::Failed { .. } | SyncState::Reconnecting { .. } => {
            state.sync = SyncState::Running;
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        }
        SyncState::Running | SyncState::Stopped => Vec::new(),
    }
}

pub(crate) fn handle_sync_failed(state: &mut AppState, reason: String) -> Vec<AppEffect> {
    if !is_session_ready(state) || matches!(state.sync, SyncState::Stopped) {
        return Vec::new();
    }

    state.sync = SyncState::Failed { reason };
    vec![
        AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
        AppEffect::StartSync,
    ]
}

pub(crate) fn handle_sync_reconnecting(state: &mut AppState, reason: String) -> Vec<AppEffect> {
    if !is_session_ready(state) || matches!(state.sync, SyncState::Stopped | SyncState::Running) {
        return Vec::new();
    }

    state.sync = SyncState::Reconnecting { reason };
    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
}

pub(crate) fn handle_sync_recovered(state: &mut AppState) -> Vec<AppEffect> {
    if !is_session_ready(state)
        || !matches!(
            state.sync,
            SyncState::Failed { .. } | SyncState::Reconnecting { .. }
        )
    {
        return Vec::new();
    }

    state.sync = SyncState::Running;
    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
}

pub(crate) fn handle_sync_stopped(state: &mut AppState) -> Vec<AppEffect> {
    if matches!(state.sync, SyncState::Stopped) {
        return Vec::new();
    }

    state.sync = SyncState::Stopped;
    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
}

pub(crate) fn handle_sync_mode_changed(state: &mut AppState, mode: SyncMode) -> Vec<AppEffect> {
    if state.sync_mode == mode {
        return Vec::new();
    }

    state.sync_mode = mode;
    vec![AppEffect::EmitUiEvent(UiEvent::SyncModeChanged)]
}
