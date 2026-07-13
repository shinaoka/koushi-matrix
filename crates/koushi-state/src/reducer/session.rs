use crate::{
    action::LoginRequest,
    effect::{AppEffect, UiEvent},
    state::{
        AppError, AppState, CurrentDeviceTrustState, LoginAttemptId, ProvisionalPhase,
        SessionState, SoftLogoutReauthState, SyncState, VerificationAccountKind,
        VerificationGateFailureKind, VerificationGateRejectReason, VerificationGateState,
        VerificationMethod, VerificationMethodCapability,
    },
};

use super::{
    clear_login_failed_errors, clear_session_views, current_session_info, is_session_ready,
};

pub(crate) fn handle_app_started(state: &mut AppState) -> Vec<AppEffect> {
    state.session = SessionState::Restoring;
    vec![AppEffect::RestoreSession]
}

pub(crate) fn handle_restore_session_requested(state: &mut AppState) -> Vec<AppEffect> {
    if !matches!(state.session, SessionState::SignedOut) {
        return Vec::new();
    }
    state.session = SessionState::Restoring;
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

fn install_provisional_session(
    state: &mut AppState,
    info: crate::state::SessionInfo,
) -> Vec<AppEffect> {
    let cleared_login_error = clear_login_failed_errors(state);
    state.session = SessionState::Provisional {
        info,
        phase: ProvisionalPhase::CheckingTrust,
    };
    state.sync = SyncState::Stopped;
    let mut effects = vec![
        AppEffect::CheckCurrentDeviceTrust,
        AppEffect::EmitUiEvent(UiEvent::SessionChanged),
    ];
    if cleared_login_error {
        effects.push(AppEffect::EmitUiEvent(UiEvent::ErrorChanged));
    }
    effects
}

pub(crate) fn handle_restore_session_succeeded(
    state: &mut AppState,
    info: crate::state::SessionInfo,
) -> Vec<AppEffect> {
    if !matches!(state.session, SessionState::Restoring) {
        return Vec::new();
    }
    install_provisional_session(state, info)
}

fn homeservers_match(expected: &str, actual: &str) -> bool {
    expected
        .trim_end_matches('/')
        .eq_ignore_ascii_case(actual.trim_end_matches('/'))
}

pub(crate) fn handle_login_succeeded(
    state: &mut AppState,
    attempt_id: LoginAttemptId,
    info: crate::state::SessionInfo,
) -> Vec<AppEffect> {
    let SessionState::Authenticating {
        homeserver,
        attempt_id: active_attempt_id,
    } = &state.session
    else {
        return Vec::new();
    };
    if *active_attempt_id != attempt_id || !homeservers_match(homeserver, &info.homeserver) {
        return Vec::new();
    }
    install_provisional_session(state, info)
}

fn promote_verified_session(
    state: &mut AppState,
    info: crate::state::SessionInfo,
) -> Vec<AppEffect> {
    state.session = SessionState::Ready(info.clone());
    state.sync = SyncState::Starting;
    vec![
        AppEffect::PersistSession(info),
        AppEffect::StartSync,
        AppEffect::EmitUiEvent(UiEvent::SessionChanged),
    ]
}

pub(crate) fn handle_current_device_trust_changed(
    state: &mut AppState,
    trust: CurrentDeviceTrustState,
) -> Vec<AppEffect> {
    if matches!(state.session, SessionState::Ready(_)) {
        return match trust {
            CurrentDeviceTrustState::Verified => Vec::new(),
            CurrentDeviceTrustState::Unknown | CurrentDeviceTrustState::Unverified => {
                handle_session_locked(state)
            }
        };
    }

    // Unknown/Unverified is the expected authoritative state while an active
    // proof attempt is in flight. Do not destroy its method, flow correlation,
    // or SAS projection; only Verified may complete the gate.
    if matches!(state.session, SessionState::Verifying { .. })
        && trust != CurrentDeviceTrustState::Verified
    {
        return Vec::new();
    }

    let info = match &state.session {
        SessionState::Provisional { info, .. }
        | SessionState::AwaitingVerification { info, .. }
        | SessionState::Verifying { info, .. } => info.clone(),
        _ => return Vec::new(),
    };
    match trust {
        CurrentDeviceTrustState::Verified => promote_verified_session(state, info),
        CurrentDeviceTrustState::Unverified => {
            state.session = SessionState::Provisional {
                info,
                phase: ProvisionalPhase::DiscoveringMethods,
            };
            vec![
                AppEffect::DiscoverVerificationMethods,
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            ]
        }
        CurrentDeviceTrustState::Unknown => {
            state.session = SessionState::Provisional {
                info,
                phase: ProvisionalPhase::RecheckingTrust {
                    failure: Some(VerificationGateFailureKind::Sdk),
                },
            };
            vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
        }
    }
}

pub(crate) fn handle_authoritative_device_trust_changed(
    state: &mut AppState,
    trust: CurrentDeviceTrustState,
) -> Vec<AppEffect> {
    if let SessionState::Locked(info) = &state.session
        && trust == CurrentDeviceTrustState::Verified
    {
        let info = info.clone();
        state.session = SessionState::Ready(info);
        state.sync = SyncState::Starting;
        return vec![
            AppEffect::StartSync,
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
        ];
    }

    let mut effects = handle_current_device_trust_changed(state, trust);
    if trust == CurrentDeviceTrustState::Verified {
        effects.retain(|effect| !matches!(effect, AppEffect::PersistSession(_)));
    }
    effects
}

pub(crate) fn handle_verification_methods_discovered(
    state: &mut AppState,
    gate: VerificationGateState,
) -> Vec<AppEffect> {
    let SessionState::Provisional {
        info,
        phase: ProvisionalPhase::DiscoveringMethods,
    } = &state.session
    else {
        return Vec::new();
    };
    let info = info.clone();
    if gate.account_kind == VerificationAccountKind::ExistingIdentity && gate.methods.is_empty() {
        state.session = SessionState::Rejecting {
            info,
            reason: VerificationGateRejectReason::ExistingIdentityWithoutProof,
        };
        return vec![AppEffect::RejectProvisionalSession];
    }
    state.session = SessionState::AwaitingVerification { info, gate };
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

pub(crate) fn handle_verification_method_discovery_failed(
    state: &mut AppState,
    kind: VerificationGateFailureKind,
) -> Vec<AppEffect> {
    let SessionState::Provisional {
        info,
        phase: ProvisionalPhase::DiscoveringMethods,
    } = &state.session
    else {
        return Vec::new();
    };

    state.session = SessionState::Provisional {
        info: info.clone(),
        phase: ProvisionalPhase::RecheckingTrust {
            failure: Some(kind),
        },
    };
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

pub(crate) fn handle_verification_method_discovery_retry_started(
    state: &mut AppState,
) -> Vec<AppEffect> {
    let SessionState::Provisional {
        info,
        phase: ProvisionalPhase::RecheckingTrust { failure: Some(_) },
    } = &state.session
    else {
        return Vec::new();
    };

    state.session = SessionState::Provisional {
        info: info.clone(),
        phase: ProvisionalPhase::DiscoveringMethods,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

fn method_capability(method: VerificationMethod) -> VerificationMethodCapability {
    match method {
        VerificationMethod::ExistingDeviceSas => VerificationMethodCapability::ExistingDeviceSas,
        VerificationMethod::RecoveryKey => VerificationMethodCapability::RecoveryKey,
        VerificationMethod::SecurityPhrase => VerificationMethodCapability::SecurityPhrase,
        VerificationMethod::Bootstrap => VerificationMethodCapability::Bootstrap,
    }
}

pub(crate) fn handle_verification_method_submitted(
    state: &mut AppState,
    method: VerificationMethod,
    flow_id: u64,
) -> Vec<AppEffect> {
    let SessionState::AwaitingVerification { info, gate } = &state.session else {
        return Vec::new();
    };
    if !gate.methods.contains(&method_capability(method)) {
        return Vec::new();
    }
    if matches!(method, VerificationMethod::Bootstrap)
        != matches!(gate.account_kind, VerificationAccountKind::NewIdentity)
    {
        return Vec::new();
    }
    let info = info.clone();
    let mut gate = gate.clone();
    gate.failure = None;
    state.session = SessionState::Verifying {
        info,
        gate,
        method,
        flow_id,
        sas_emojis: Vec::new(),
    };
    vec![
        AppEffect::BeginSessionVerification { method, flow_id },
        AppEffect::EmitUiEvent(UiEvent::SessionChanged),
    ]
}

pub(crate) fn handle_verification_gate_attempt_failed(
    state: &mut AppState,
    kind: VerificationGateFailureKind,
) -> Vec<AppEffect> {
    let SessionState::Verifying { info, gate, .. } = &state.session else {
        return Vec::new();
    };
    let mut gate = gate.clone();
    gate.failure = Some(kind);
    state.session = SessionState::AwaitingVerification {
        info: info.clone(),
        gate,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

pub(crate) fn handle_bootstrap_recovery_key_delivered(
    state: &mut AppState,
    flow_id: u64,
) -> Vec<AppEffect> {
    let SessionState::Verifying {
        info,
        gate,
        method: VerificationMethod::Bootstrap,
        flow_id: active_flow_id,
        ..
    } = &state.session
    else {
        return Vec::new();
    };
    if *active_flow_id != flow_id || gate.account_kind != VerificationAccountKind::NewIdentity {
        return Vec::new();
    }
    state.session = SessionState::AwaitingBootstrapConfirmation {
        info: info.clone(),
        gate: gate.clone(),
        flow_id,
        destination_written: true,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

pub(crate) fn handle_bootstrap_recovery_key_delivery_failed(
    state: &mut AppState,
    flow_id: u64,
    kind: VerificationGateFailureKind,
) -> Vec<AppEffect> {
    let SessionState::Verifying {
        info,
        gate,
        method: VerificationMethod::Bootstrap,
        flow_id: active_flow_id,
        ..
    } = &state.session
    else {
        return Vec::new();
    };
    if *active_flow_id != flow_id || gate.account_kind != VerificationAccountKind::NewIdentity {
        return Vec::new();
    }
    let mut gate = gate.clone();
    gate.failure = Some(kind);
    state.session = SessionState::AwaitingVerification {
        info: info.clone(),
        gate,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

pub(crate) fn handle_bootstrap_recovery_saved_confirmed(
    state: &mut AppState,
    flow_id: u64,
) -> Vec<AppEffect> {
    let SessionState::AwaitingBootstrapConfirmation {
        info,
        flow_id: active_flow_id,
        destination_written: true,
        ..
    } = &state.session
    else {
        return Vec::new();
    };
    if *active_flow_id != flow_id {
        return Vec::new();
    }
    state.session = SessionState::Provisional {
        info: info.clone(),
        phase: ProvisionalPhase::RecheckingTrust { failure: None },
    };
    vec![
        AppEffect::CheckCurrentDeviceTrust,
        AppEffect::EmitUiEvent(UiEvent::SessionChanged),
    ]
}

pub(crate) fn handle_verification_session_rejected(
    state: &mut AppState,
    reason: VerificationGateRejectReason,
) -> Vec<AppEffect> {
    let Some(info) = current_session_info(state) else {
        return Vec::new();
    };
    if !matches!(
        state.session,
        SessionState::Provisional { .. }
            | SessionState::AwaitingVerification { .. }
            | SessionState::Verifying { .. }
            | SessionState::AwaitingBootstrapConfirmation { .. }
    ) {
        return Vec::new();
    }
    state.session = SessionState::Rejecting { info, reason };
    state.sync = SyncState::Stopped;
    let mut effects = vec![AppEffect::RejectProvisionalSession];
    effects.extend(clear_session_views(state));
    effects
}

pub(crate) fn handle_provisional_session_discarded(state: &mut AppState) -> Vec<AppEffect> {
    if !matches!(state.session, SessionState::Rejecting { .. }) {
        return Vec::new();
    }
    *state = AppState::default();
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
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
    attempt_id: LoginAttemptId,
    request: LoginRequest,
) -> Vec<AppEffect> {
    if !admit_authentication_start(state, attempt_id, request.homeserver.clone()) {
        return Vec::new();
    }
    vec![
        AppEffect::Login {
            attempt_id,
            request,
        },
        AppEffect::EmitUiEvent(UiEvent::SessionChanged),
    ]
}

pub(crate) fn handle_authentication_started(
    state: &mut AppState,
    attempt_id: LoginAttemptId,
    homeserver: String,
) -> Vec<AppEffect> {
    if !admit_authentication_start(state, attempt_id, homeserver) {
        return Vec::new();
    }
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

fn admit_authentication_start(
    state: &mut AppState,
    attempt_id: LoginAttemptId,
    homeserver: String,
) -> bool {
    if !matches!(
        state.session,
        SessionState::SignedOut | SessionState::Authenticating { .. }
    ) {
        return false;
    }
    state.session = SessionState::Authenticating {
        homeserver,
        attempt_id,
    };
    true
}

pub(crate) fn handle_login_failed(
    state: &mut AppState,
    attempt_id: LoginAttemptId,
    message: String,
) -> Vec<AppEffect> {
    if !matches!(
        state.session,
        SessionState::Authenticating {
            attempt_id: active_attempt_id,
            ..
        } if active_attempt_id == attempt_id
    ) {
        return Vec::new();
    }
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

pub(crate) fn handle_verification_admission_preparation_failed(
    state: &mut AppState,
    kind: crate::state::VerificationGateFailureKind,
) -> Vec<AppEffect> {
    let SessionState::Provisional { info, .. } = &state.session else {
        return Vec::new();
    };
    state.session = SessionState::Provisional {
        info: info.clone(),
        phase: crate::state::ProvisionalPhase::RecheckingTrust {
            failure: Some(kind),
        },
    };
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

pub(crate) fn handle_session_locked(state: &mut AppState) -> Vec<AppEffect> {
    if let SessionState::Ready(info) = &state.session {
        let submission_registry = state.timeline.submission_registry.clone();
        state.session = SessionState::Locked(info.clone());
        state.sync = SyncState::Stopped;
        let mut effects = vec![
            AppEffect::StopSync,
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
        ];
        effects.extend(clear_session_views(state));
        state.timeline.submission_registry = submission_registry;
        return effects;
    }
    Vec::new()
}

pub(crate) fn handle_logout_requested(state: &mut AppState) -> Vec<AppEffect> {
    state.session = SessionState::LoggingOut;
    state.sync = SyncState::Stopped;
    let mut effects = vec![
        AppEffect::StopSync,
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
    let mut effects = vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)];
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
