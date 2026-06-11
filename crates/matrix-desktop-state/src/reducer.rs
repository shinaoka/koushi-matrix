use crate::{
    action::AppAction,
    effect::{AppEffect, UiEvent},
    state::{AppError, AppState, SearchState, SessionState, SyncState, ThreadPaneState},
};

pub fn reduce(state: &mut AppState, action: AppAction) -> Vec<AppEffect> {
    match action {
        AppAction::AppStarted => {
            state.session = SessionState::Restoring;
            vec![AppEffect::RestoreSession]
        }
        AppAction::RestoreSessionSucceeded(info) | AppAction::LoginSucceeded(info) => {
            state.session = SessionState::Ready(info.clone());
            state.sync = SyncState::Starting;
            vec![
                AppEffect::PersistSession(info),
                AppEffect::StartSync,
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            ]
        }
        AppAction::RestoreSessionFailed { message } => {
            state.session = SessionState::SignedOut;
            state.errors.push(AppError {
                code: "restore_failed".to_owned(),
                message,
                recoverable: true,
            });
            vec![
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
                AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
            ]
        }
        AppAction::LoginSubmitted {
            homeserver,
            username,
        } => {
            state.session = SessionState::Authenticating {
                homeserver: homeserver.clone(),
            };
            vec![AppEffect::Login {
                homeserver,
                username,
            }]
        }
        AppAction::LoginFailed { message } => {
            state.session = SessionState::SignedOut;
            state.errors.push(AppError {
                code: "login_failed".to_owned(),
                message,
                recoverable: true,
            });
            vec![AppEffect::EmitUiEvent(UiEvent::ErrorChanged)]
        }
        AppAction::SessionLocked => {
            if let SessionState::Ready(info) = &state.session {
                state.session = SessionState::Locked(info.clone());
            }
            vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
        }
        AppAction::LogoutRequested => {
            state.session = SessionState::LoggingOut;
            state.sync = SyncState::Stopped;
            state.timeline = Default::default();
            state.thread = ThreadPaneState::Closed;
            state.search = SearchState::Closed;
            vec![AppEffect::StopSync, AppEffect::ClearSession]
        }
        AppAction::LogoutFinished => {
            *state = AppState::default();
            vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
        }
        AppAction::SyncStarted => {
            state.sync = SyncState::Running;
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        }
        AppAction::SyncFailed { reason } => {
            state.sync = SyncState::Recovering { reason };
            vec![AppEffect::StartSync]
        }
        AppAction::SyncRecovered => {
            state.sync = SyncState::Running;
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        }
        AppAction::SyncStopped => {
            state.sync = SyncState::Stopped;
            vec![AppEffect::StopSync]
        }
        AppAction::ClearError { code } => {
            state.errors.retain(|error| error.code != code);
            vec![AppEffect::EmitUiEvent(UiEvent::ErrorChanged)]
        }
        _ => Vec::new(),
    }
}
