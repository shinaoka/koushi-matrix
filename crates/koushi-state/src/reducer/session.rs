use crate::{
    action::LoginRequest,
    effect::{AppEffect, UiEvent},
    state::{
        AppError, AppState, CurrentDeviceTrustState, DeviceCleanupAuthMode,
        DeviceCleanupFailureKind, DeviceCleanupLocalMode, DeviceCleanupOfferReason,
        DeviceCleanupRemoteOutcome, DeviceCleanupState, LoginAttemptId, ProvisionalPhase,
        SessionLockReason, SessionState, SlidingSyncCapabilityState, SoftLogoutReauthState,
        SyncState, VerificationAccountKind, VerificationGateFailureKind,
        VerificationGateRejectReason, VerificationGateState, VerificationMethod,
        VerificationMethodCapability,
    },
};

use super::{
    clear_login_failed_errors, clear_session_views, clear_stale_verification_flow,
    current_session_info, is_session_ready,
};

fn reset_app_state_preserving_account_epoch(state: &mut AppState) {
    let account_epoch = state.sliding_sync_account_epoch;
    *state = AppState::default();
    state.sliding_sync_account_epoch = account_epoch;
}

pub(crate) fn handle_app_started(state: &mut AppState) -> Vec<AppEffect> {
    state.session = SessionState::Restoring;
    state.session_lock_reason = None;
    state.sliding_sync_capability = SlidingSyncCapabilityState::Unknown;
    vec![AppEffect::RestoreSession]
}

pub(crate) fn handle_restore_session_requested(state: &mut AppState) -> Vec<AppEffect> {
    if !matches!(state.session, SessionState::SignedOut) {
        return Vec::new();
    }
    state.session = SessionState::Restoring;
    state.session_lock_reason = None;
    state.sliding_sync_capability = SlidingSyncCapabilityState::Unknown;
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

fn install_provisional_session(
    state: &mut AppState,
    info: crate::state::SessionInfo,
) -> Vec<AppEffect> {
    let cleared_login_error = clear_login_failed_errors(state);
    state.session_lock_reason = None;
    state.device_cleanup = DeviceCleanupState::Idle;
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
    let expected = match &state.session {
        SessionState::Restoring => true,
        SessionState::SwitchingAccount { info: target } => {
            target.homeserver == info.homeserver
                && target.user_id == info.user_id
                && target.device_id == info.device_id
        }
        _ => false,
    };
    if !expected {
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
    state.session_lock_reason = None;
    state.secure_backup_gate = crate::state::SecureBackupGateState::Checking;
    state.device_cleanup = DeviceCleanupState::Idle;
    state.sync = SyncState::Starting;
    vec![
        AppEffect::PersistSession(info),
        AppEffect::StartSync,
        AppEffect::InspectSecureBackup,
        AppEffect::EmitUiEvent(UiEvent::SessionChanged),
    ]
}

pub(crate) fn handle_secure_backup_gate_changed(
    state: &mut AppState,
    gate: crate::state::SecureBackupGateState,
) -> Vec<AppEffect> {
    if !matches!(state.session, SessionState::Ready(_)) || state.secure_backup_gate == gate {
        return Vec::new();
    }
    state.secure_backup_gate = gate;
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

pub(crate) fn handle_current_device_trust_changed(
    state: &mut AppState,
    trust: CurrentDeviceTrustState,
) -> Vec<AppEffect> {
    if matches!(state.session, SessionState::Ready(_)) {
        return match trust {
            CurrentDeviceTrustState::Verified => Vec::new(),
            CurrentDeviceTrustState::Unverified => {
                gate_ready_session(state, ProvisionalPhase::DiscoveringMethods)
            }
            CurrentDeviceTrustState::Unknown => {
                gate_ready_session(state, ProvisionalPhase::RecheckingTrust { failure: None })
            }
        };
    }

    // Unknown/Unverified is the expected authoritative state while a proof
    // attempt is available or in flight. Do not destroy the discovered method
    // list, selected method, flow correlation, or SAS projection; only Verified
    // may complete the gate.
    if matches!(
        state.session,
        SessionState::AwaitingVerification { .. } | SessionState::Verifying { .. }
    ) && trust != CurrentDeviceTrustState::Verified
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
            state.device_cleanup = DeviceCleanupState::Idle;
            vec![
                AppEffect::DiscoverVerificationMethods,
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            ]
        }
        CurrentDeviceTrustState::Unknown => {
            state.session = SessionState::Provisional {
                info,
                phase: ProvisionalPhase::RecheckingTrust { failure: None },
            };
            state.device_cleanup = DeviceCleanupState::Idle;
            vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
        }
    }
}

pub(crate) fn handle_authoritative_device_trust_changed(
    state: &mut AppState,
    trust: CurrentDeviceTrustState,
) -> Vec<AppEffect> {
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
    let mut gate = gate;
    if gate.account_kind == VerificationAccountKind::ExistingIdentity && gate.methods.is_empty() {
        gate.failure = Some(VerificationGateFailureKind::NoProofMethod);
        state.device_cleanup = DeviceCleanupState::Offered {
            reason: DeviceCleanupOfferReason::NoProofMethod,
        };
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
    state.device_cleanup = DeviceCleanupState::Idle;
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
    state.device_cleanup = DeviceCleanupState::Idle;
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
    state.device_cleanup = DeviceCleanupState::Idle;
    state.session = SessionState::Verifying {
        info,
        gate,
        method,
        flow_id,
        sas_emojis: Vec::new(),
    };
    let verification_cleared = clear_stale_verification_flow(state);
    let mut effects = vec![
        AppEffect::BeginSessionVerification { method, flow_id },
        AppEffect::EmitUiEvent(UiEvent::SessionChanged),
    ];
    if verification_cleared {
        effects.push(AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged));
    }
    effects
}

pub(crate) fn handle_verification_gate_attempt_failed(
    state: &mut AppState,
    flow_id: u64,
    kind: VerificationGateFailureKind,
) -> Vec<AppEffect> {
    let SessionState::Verifying {
        info,
        gate,
        flow_id: active_flow_id,
        ..
    } = &state.session
    else {
        return Vec::new();
    };
    if *active_flow_id != flow_id {
        return Vec::new();
    }
    let mut gate = gate.clone();
    gate.failure = Some(kind);
    state.device_cleanup = DeviceCleanupState::Offered {
        reason: DeviceCleanupOfferReason::RecoveryFailed,
    };
    state.session = SessionState::AwaitingVerification {
        info: info.clone(),
        gate,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

pub(crate) fn handle_device_cleanup_start_requested(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    if !matches!(
        state.session,
        SessionState::AwaitingVerification { .. }
            | SessionState::Provisional {
                phase: ProvisionalPhase::RecheckingTrust { .. },
                ..
            }
    ) {
        return Vec::new();
    }
    state.device_cleanup = match state.device_cleanup {
        DeviceCleanupState::Offered { .. } | DeviceCleanupState::RemoteFailed { .. } => {
            DeviceCleanupState::ResolvingRemote { request_id }
        }
        DeviceCleanupState::LocalResetFailed { mode, .. } => match mode {
            DeviceCleanupLocalMode::RemoteRemoved { .. } => {
                DeviceCleanupState::ResettingLocal { request_id, mode }
            }
            DeviceCleanupLocalMode::RemoteMayRemain => {
                DeviceCleanupState::ErasingLocalAnyway { request_id }
            }
        },
        _ => return Vec::new(),
    };
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

pub(crate) fn handle_device_cleanup_remote_started(
    state: &mut AppState,
    request_id: u64,
    auth_mode: DeviceCleanupAuthMode,
) -> Vec<AppEffect> {
    if !matches!(
        state.device_cleanup,
        DeviceCleanupState::ResolvingRemote {
            request_id: active_request_id
        } if active_request_id == request_id
    ) {
        return Vec::new();
    }
    state.device_cleanup = DeviceCleanupState::RemovingRemote {
        request_id,
        auth_mode,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

pub(crate) fn handle_device_cleanup_uia_required(
    state: &mut AppState,
    request_id: u64,
    flow_id: u64,
) -> Vec<AppEffect> {
    if !matches!(
        state.device_cleanup,
        DeviceCleanupState::RemovingRemote {
            request_id: active_request_id,
            auth_mode: DeviceCleanupAuthMode::Legacy,
        } if active_request_id == request_id
    ) {
        return Vec::new();
    }
    state.device_cleanup = DeviceCleanupState::AwaitingUia {
        request_id,
        flow_id,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

pub(crate) fn handle_device_cleanup_uia_submitted(
    state: &mut AppState,
    request_id: u64,
    flow_id: u64,
) -> Vec<AppEffect> {
    if !matches!(
        state.device_cleanup,
        DeviceCleanupState::AwaitingUia {
            request_id: active_request_id,
            flow_id: active_flow_id,
        } if active_request_id == request_id && active_flow_id == flow_id
    ) {
        return Vec::new();
    }
    state.device_cleanup = DeviceCleanupState::RemovingRemote {
        request_id,
        auth_mode: DeviceCleanupAuthMode::Legacy,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

pub(crate) fn handle_device_cleanup_remote_settled(
    state: &mut AppState,
    request_id: u64,
    outcome: DeviceCleanupRemoteOutcome,
) -> Vec<AppEffect> {
    if !matches!(
        state.device_cleanup,
        DeviceCleanupState::RemovingRemote {
            request_id: active_request_id,
            ..
        } if active_request_id == request_id
    ) {
        return Vec::new();
    }
    state.device_cleanup = DeviceCleanupState::ResettingLocal {
        request_id,
        mode: DeviceCleanupLocalMode::RemoteRemoved { outcome },
    };
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

pub(crate) fn handle_device_cleanup_remote_failed(
    state: &mut AppState,
    request_id: u64,
    auth_mode: DeviceCleanupAuthMode,
    kind: DeviceCleanupFailureKind,
) -> Vec<AppEffect> {
    if !matches!(
        state.device_cleanup,
        DeviceCleanupState::RemovingRemote {
            request_id: active_request_id,
            auth_mode: active_auth_mode,
        } if active_request_id == request_id && active_auth_mode == auth_mode
    ) {
        return Vec::new();
    }
    state.device_cleanup = DeviceCleanupState::RemoteFailed {
        request_id,
        auth_mode,
        failure: kind,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

pub(crate) fn handle_device_cleanup_erase_local_anyway_requested(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    if !matches!(
        state.device_cleanup,
        DeviceCleanupState::RemoteFailed { .. }
    ) {
        return Vec::new();
    }
    state.device_cleanup = DeviceCleanupState::ErasingLocalAnyway { request_id };
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

pub(crate) fn handle_device_cleanup_local_reset_failed(
    state: &mut AppState,
    request_id: u64,
    kind: DeviceCleanupFailureKind,
) -> Vec<AppEffect> {
    let mode = match state.device_cleanup {
        DeviceCleanupState::ResettingLocal {
            request_id: active_request_id,
            mode,
        } if active_request_id == request_id => mode,
        DeviceCleanupState::ErasingLocalAnyway {
            request_id: active_request_id,
        } if active_request_id == request_id => DeviceCleanupLocalMode::RemoteMayRemain,
        _ => return Vec::new(),
    };
    state.device_cleanup = DeviceCleanupState::LocalResetFailed {
        request_id,
        mode,
        failure: kind,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

pub(crate) fn handle_device_cleanup_completed(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    if !matches!(
        state.device_cleanup,
        DeviceCleanupState::ResettingLocal {
            request_id: active_request_id,
            ..
        } | DeviceCleanupState::ErasingLocalAnyway {
            request_id: active_request_id,
        } if active_request_id == request_id
    ) {
        return Vec::new();
    }
    reset_app_state_preserving_account_epoch(state);
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
    state.device_cleanup = DeviceCleanupState::Idle;
    state.sync = SyncState::Stopped;
    let mut effects = vec![AppEffect::RejectProvisionalSession];
    effects.extend(clear_session_views(state));
    effects
}

pub(crate) fn handle_provisional_session_discarded(state: &mut AppState) -> Vec<AppEffect> {
    if !matches!(state.session, SessionState::Rejecting { .. }) {
        return Vec::new();
    }
    reset_app_state_preserving_account_epoch(state);
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

pub(crate) fn handle_restore_session_not_found(state: &mut AppState) -> Vec<AppEffect> {
    state.session = SessionState::SignedOut;
    state.session_lock_reason = None;
    state.sliding_sync_capability = SlidingSyncCapabilityState::Unknown;
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

pub(crate) fn handle_restore_session_failed(
    state: &mut AppState,
    message: String,
) -> Vec<AppEffect> {
    state.session = SessionState::SignedOut;
    state.session_lock_reason = None;
    state.sliding_sync_capability = SlidingSyncCapabilityState::Unknown;
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
    state.session_lock_reason = None;
    state.sliding_sync_capability = SlidingSyncCapabilityState::Unknown;
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
    state.session_lock_reason = None;
    state.sliding_sync_capability = SlidingSyncCapabilityState::Unknown;
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

fn gate_ready_session(state: &mut AppState, phase: ProvisionalPhase) -> Vec<AppEffect> {
    if let SessionState::Ready(info) = &state.session {
        let submission_registry = state.timeline.submission_registry.clone();
        state.session = SessionState::Provisional {
            info: info.clone(),
            phase,
        };
        state.session_lock_reason = None;
        state.device_cleanup = DeviceCleanupState::Idle;
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

fn lock_ready_session(state: &mut AppState, reason: SessionLockReason) -> Vec<AppEffect> {
    if let SessionState::Ready(info) = &state.session {
        let submission_registry = state.timeline.submission_registry.clone();
        state.session = SessionState::Locked(info.clone());
        state.session_lock_reason = Some(reason);
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

pub(crate) fn handle_session_locked(state: &mut AppState) -> Vec<AppEffect> {
    gate_ready_session(state, ProvisionalPhase::DiscoveringMethods)
}

pub(crate) fn handle_session_authentication_invalidated(
    state: &mut AppState,
    soft_logout: bool,
) -> Vec<AppEffect> {
    lock_ready_session(state, SessionLockReason::UnknownToken { soft_logout })
}

pub(crate) fn handle_logout_requested(state: &mut AppState) -> Vec<AppEffect> {
    state.session = SessionState::LoggingOut;
    state.session_lock_reason = None;
    state.sliding_sync_capability = SlidingSyncCapabilityState::Unknown;
    state.device_cleanup = DeviceCleanupState::Idle;
    state.sync = SyncState::Stopped;
    let mut effects = vec![
        AppEffect::StopSync,
        AppEffect::EmitUiEvent(UiEvent::SessionChanged),
    ];
    effects.extend(clear_session_views(state));
    effects
}

pub(crate) fn handle_logout_finished(state: &mut AppState) -> Vec<AppEffect> {
    reset_app_state_preserving_account_epoch(state);
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
    state.session_lock_reason = None;
    state.sliding_sync_capability = SlidingSyncCapabilityState::Unknown;
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
