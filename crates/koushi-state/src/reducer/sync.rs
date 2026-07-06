use crate::{
    effect::{AppEffect, UiEvent},
    state::{AppError, AppState, SessionState, SyncLifecycleStatus, SyncMode, SyncState},
};

use super::{current_session_info, is_session_ready};

pub(crate) fn handle_sync_status_changed(
    state: &mut AppState,
    generation: u64,
    status: SyncLifecycleStatus,
) -> Vec<AppEffect> {
    if generation <= state.sync_generation {
        return Vec::new();
    }

    state.sync_generation = generation;
    let next = if is_session_ready(state) {
        sync_state_from_status(status)
    } else {
        SyncState::Stopped
    };

    if state.sync == next {
        return Vec::new();
    }

    state.sync = next;
    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
}

fn sync_state_from_status(status: SyncLifecycleStatus) -> SyncState {
    match status {
        SyncLifecycleStatus::Stopped => SyncState::Stopped,
        SyncLifecycleStatus::Starting => SyncState::Starting,
        SyncLifecycleStatus::Running => SyncState::Running,
        SyncLifecycleStatus::Failed { reason } => SyncState::Failed { reason },
        SyncLifecycleStatus::Reconnecting { reason } => SyncState::Reconnecting { reason },
    }
}

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

    // Auth failures are terminal: the session token is invalid and retrying
    // will loop forever. Leave the sync in Failed state without scheduling a
    // restart so the GUI can surface an error and prompt the user to log in.
    let auth_failure = reason == "sync_failed_auth";
    let retry = !auth_failure;

    state.sync = SyncState::Failed { reason };
    let mut effects = vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)];
    if auth_failure {
        if let Some(info) = current_session_info(state) {
            state.session = SessionState::Locked(info);
        }
        state.errors.push(AppError {
            code: "sync_auth_required".to_owned(),
            message: "sign-in required".to_owned(),
            recoverable: true,
        });
        effects.push(AppEffect::EmitUiEvent(UiEvent::SessionChanged));
        effects.push(AppEffect::EmitUiEvent(UiEvent::ErrorChanged));
    }
    if retry {
        effects.push(AppEffect::StartSync);
    }
    effects
}

pub(crate) fn handle_sync_reconnecting(state: &mut AppState, reason: String) -> Vec<AppEffect> {
    if !is_session_ready(state) || matches!(state.sync, SyncState::Stopped) {
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
