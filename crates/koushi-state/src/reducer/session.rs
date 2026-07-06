use crate::{
    action::LoginRequest,
    effect::{AppEffect, UiEvent},
    state::{AppError, AppState, SessionState, SoftLogoutReauthState, SyncState},
};

use super::{
    clear_login_failed_errors, clear_session_views, current_session_info, is_session_ready,
};

pub(crate) fn handle_app_started(state: &mut AppState) -> Vec<AppEffect> {
    state.session = SessionState::Restoring;
    vec![AppEffect::RestoreSession]
}

pub(crate) fn handle_restore_or_login_succeeded(
    state: &mut AppState,
    info: crate::state::SessionInfo,
) -> Vec<AppEffect> {
    let cleared_login_error = clear_login_failed_errors(state);
    state.session = SessionState::Ready(info.clone());
    state.sync = SyncState::Starting;
    let mut effects = vec![
        AppEffect::PersistSession(info),
        AppEffect::StartSync,
        AppEffect::EmitUiEvent(UiEvent::SessionChanged),
    ];
    if cleared_login_error {
        effects.push(AppEffect::EmitUiEvent(UiEvent::ErrorChanged));
    }
    effects
}

pub(crate) fn handle_restore_session_not_found(state: &mut AppState) -> Vec<AppEffect> {
    state.session = SessionState::SignedOut;
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

pub(crate) fn handle_restore_session_failed(
    state: &mut AppState,
    message: String,
) -> Vec<AppEffect> {
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

pub(crate) fn handle_login_submitted(
    state: &mut AppState,
    request: LoginRequest,
) -> Vec<AppEffect> {
    state.session = SessionState::Authenticating {
        homeserver: request.homeserver.clone(),
    };
    vec![
        AppEffect::Login(request),
        AppEffect::EmitUiEvent(UiEvent::SessionChanged),
    ]
}

pub(crate) fn handle_login_failed(state: &mut AppState, message: String) -> Vec<AppEffect> {
    state.session = SessionState::SignedOut;
    state.errors.push(AppError {
        code: "login_failed".to_owned(),
        message,
        recoverable: true,
    });
    vec![
        AppEffect::EmitUiEvent(UiEvent::SessionChanged),
        AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
    ]
}

pub(crate) fn handle_login_discovery_requested(
    state: &mut AppState,
    homeserver: String,
) -> Vec<AppEffect> {
    state.auth = crate::state::AuthDiscoveryState::Discovering {
        homeserver: homeserver.clone(),
    };
    vec![
        AppEffect::DiscoverLogin { homeserver },
        AppEffect::EmitUiEvent(UiEvent::AuthChanged),
    ]
}

pub(crate) fn handle_login_discovery_succeeded(
    state: &mut AppState,
    homeserver: String,
    flows: Vec<crate::state::LoginFlow>,
    delegated: crate::state::DelegatedAuthLinks,
) -> Vec<AppEffect> {
    if !matches!(
        &state.auth,
        crate::state::AuthDiscoveryState::Discovering {
            homeserver: active
        } if active == &homeserver
    ) {
        return Vec::new();
    }
    state.auth = crate::state::AuthDiscoveryState::Ready {
        homeserver,
        flows,
        delegated,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::AuthChanged)]
}

pub(crate) fn handle_login_discovery_failed(
    state: &mut AppState,
    homeserver: String,
    kind: crate::state::AuthFailureKind,
) -> Vec<AppEffect> {
    if !matches!(
        &state.auth,
        crate::state::AuthDiscoveryState::Discovering {
            homeserver: active
        } if active == &homeserver
    ) {
        return Vec::new();
    }
    state.auth = crate::state::AuthDiscoveryState::Failed { homeserver, kind };
    vec![AppEffect::EmitUiEvent(UiEvent::AuthChanged)]
}

pub(crate) fn handle_session_persistence_failed(
    state: &mut AppState,
    message: String,
) -> Vec<AppEffect> {
    state.errors.push(AppError {
        code: "session_persistence_failed".to_owned(),
        message,
        recoverable: true,
    });
    vec![AppEffect::EmitUiEvent(UiEvent::ErrorChanged)]
}

pub(crate) fn handle_session_locked(state: &mut AppState) -> Vec<AppEffect> {
    if let SessionState::Ready(info) = &state.session {
        state.session = SessionState::Locked(info.clone());
        state.sync = SyncState::Stopped;
        let mut effects = vec![
            AppEffect::StopSync,
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
        ];
        effects.extend(clear_session_views(state));
        return effects;
    }
    Vec::new()
}

pub(crate) fn handle_logout_requested(state: &mut AppState) -> Vec<AppEffect> {
    state.session = SessionState::LoggingOut;
    state.sync = SyncState::Stopped;
    let mut effects = vec![
        AppEffect::StopSync,
        AppEffect::ClearSession,
        AppEffect::EmitUiEvent(UiEvent::SessionChanged),
    ];
    effects.extend(clear_session_views(state));
    effects
}

pub(crate) fn handle_logout_finished(state: &mut AppState) -> Vec<AppEffect> {
    *state = AppState::default();
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

pub(crate) fn handle_switch_account_requested(
    state: &mut AppState,
    info: crate::state::SessionInfo,
) -> Vec<AppEffect> {
    if current_session_info(state).as_ref() == Some(&info) {
        return Vec::new();
    }

    state.session = SessionState::SwitchingAccount { info: info.clone() };
    state.sync = SyncState::Stopped;
    let mut effects = vec![
        AppEffect::StopSync,
        AppEffect::ClearSession,
        AppEffect::RestoreSessionFor(info),
        AppEffect::EmitUiEvent(UiEvent::SessionChanged),
    ];
    effects.extend(clear_session_views(state));
    effects
}

pub(crate) fn handle_soft_logout_reauth_requested(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    let can_reauth = is_session_ready(state) || matches!(state.session, SessionState::Locked(_));
    if !can_reauth || !matches!(state.soft_logout_reauth, SoftLogoutReauthState::Idle) {
        return Vec::new();
    }
    state.soft_logout_reauth = SoftLogoutReauthState::Authenticating { request_id };
    vec![AppEffect::EmitUiEvent(UiEvent::SoftLogoutReauthChanged)]
}

pub(crate) fn handle_soft_logout_reauth_succeeded(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    if !matches!(
        state.soft_logout_reauth,
        SoftLogoutReauthState::Authenticating {
            request_id: active
        } if active == request_id
    ) {
        return Vec::new();
    }
    state.soft_logout_reauth = SoftLogoutReauthState::Succeeded { request_id };
    vec![AppEffect::EmitUiEvent(UiEvent::SoftLogoutReauthChanged)]
}

pub(crate) fn handle_soft_logout_reauth_failed(
    state: &mut AppState,
    request_id: u64,
    kind: crate::state::AuthFailureKind,
) -> Vec<AppEffect> {
    if !matches!(
        state.soft_logout_reauth,
        SoftLogoutReauthState::Authenticating {
            request_id: active
        } if active == request_id
    ) {
        return Vec::new();
    }
    state.soft_logout_reauth = SoftLogoutReauthState::Failed { request_id, kind };
    vec![AppEffect::EmitUiEvent(UiEvent::SoftLogoutReauthChanged)]
}
