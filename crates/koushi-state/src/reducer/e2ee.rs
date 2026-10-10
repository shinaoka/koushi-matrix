use crate::{
    effect::{AppEffect, UiEvent},
    state::{
        AppState, AuthFailureKind, CrossSigningStatus, DeviceCleanupOfferReason,
        DeviceCleanupState, IdentityResetState, KeyBackupStatus, ProvisionalPhase, QrLoginState,
        RoomKeyExportState, RoomKeyImportState, SasEmoji, SecureBackupPassphraseChangeState,
        SecureBackupSetupState, SessionState, SyncState, TrustOperationFailureKind,
        VerificationAccountKind, VerificationCancelReason, VerificationFlowState,
        VerificationGateFailureKind, VerificationGateState, VerificationMethod,
        VerificationMethodCapability, VerificationTarget,
    },
};

use super::{
    clear_login_failed_errors, clear_session_views, clear_stale_verification_flow,
    has_verification_gate_projection_context, is_session_ready,
};

fn recovery_gate(
    methods: Vec<crate::state::RecoveryMethod>,
    failure: Option<VerificationGateFailureKind>,
) -> VerificationGateState {
    VerificationGateState {
        methods: methods
            .into_iter()
            .map(|method| match method {
                crate::state::RecoveryMethod::RecoveryKey => {
                    VerificationMethodCapability::RecoveryKey
                }
                crate::state::RecoveryMethod::SecurityPhrase => {
                    VerificationMethodCapability::SecurityPhrase
                }
            })
            .collect(),
        account_kind: VerificationAccountKind::ExistingIdentity,
        failure,
    }
}

pub(crate) fn handle_e2ee_recovery_required(
    state: &mut AppState,
    info: crate::state::SessionInfo,
    methods: Vec<crate::state::RecoveryMethod>,
) -> Vec<AppEffect> {
    if !matches!(
        &state.session,
        SessionState::Provisional {
            info: current,
            phase: ProvisionalPhase::DiscoveringMethods,
        } if current == &info
    ) {
        return Vec::new();
    }
    let cleared_login_error = clear_login_failed_errors(state);
    state.session = SessionState::AwaitingVerification {
        info,
        gate: recovery_gate(methods, None),
    };
    state.sync = crate::state::SyncState::Stopped;
    let mut effects = vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)];
    effects.extend(clear_session_views(state));
    if cleared_login_error {
        effects.push(AppEffect::EmitUiEvent(UiEvent::ErrorChanged));
    }
    effects
}

pub(crate) fn handle_gate_sas_presented(
    state: &mut AppState,
    flow_id: u64,
    emojis: Vec<SasEmoji>,
) -> Vec<AppEffect> {
    if emojis.len() != 7 {
        return Vec::new();
    }
    let SessionState::Verifying {
        method: VerificationMethod::ExistingDeviceSas,
        flow_id: active_flow_id,
        sas_emojis,
        ..
    } = &mut state.session
    else {
        return Vec::new();
    };
    if *active_flow_id != flow_id {
        return Vec::new();
    }
    *sas_emojis = emojis;
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

pub(crate) fn handle_e2ee_recovery_submitted(
    state: &mut AppState,
    flow_id: u64,
    request: crate::action::RecoveryRequest,
) -> Vec<AppEffect> {
    let SessionState::AwaitingVerification { info, gate } = &state.session else {
        return Vec::new();
    };
    let method = if gate
        .methods
        .contains(&VerificationMethodCapability::RecoveryKey)
    {
        VerificationMethod::RecoveryKey
    } else if gate
        .methods
        .contains(&VerificationMethodCapability::SecurityPhrase)
    {
        VerificationMethod::SecurityPhrase
    } else {
        return Vec::new();
    };
    state.device_cleanup = DeviceCleanupState::Idle;
    state.session = SessionState::Verifying {
        info: info.clone(),
        gate: gate.clone(),
        method,
        flow_id,
        sas_emojis: Vec::new(),
    };
    let verification_cleared = clear_stale_verification_flow(state);
    let mut effects = vec![
        AppEffect::RecoverE2ee(request),
        AppEffect::EmitUiEvent(UiEvent::SessionChanged),
    ];
    if verification_cleared {
        effects.push(AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged));
    }
    effects
}

pub(crate) fn handle_e2ee_recovery_succeeded(state: &mut AppState) -> Vec<AppEffect> {
    let SessionState::Verifying { info, method, .. } = &state.session else {
        return Vec::new();
    };
    if !matches!(
        method,
        VerificationMethod::RecoveryKey | VerificationMethod::SecurityPhrase
    ) {
        return Vec::new();
    }
    let info = info.clone();
    state.session = SessionState::Ready(info.clone());
    state.sync = SyncState::Starting;
    vec![
        AppEffect::PersistSession(info),
        AppEffect::StartSync,
        AppEffect::EmitUiEvent(UiEvent::SessionChanged),
    ]
}

pub(crate) fn handle_e2ee_recovery_failed(state: &mut AppState, message: String) -> Vec<AppEffect> {
    let SessionState::Verifying {
        info, gate, method, ..
    } = &state.session
    else {
        return Vec::new();
    };
    if !matches!(
        method,
        VerificationMethod::RecoveryKey | VerificationMethod::SecurityPhrase
    ) {
        return Vec::new();
    }
    let mut gate = gate.clone();
    gate.failure = Some(VerificationGateFailureKind::Sdk);
    state.device_cleanup = DeviceCleanupState::Offered {
        reason: DeviceCleanupOfferReason::RecoveryFailed,
    };
    state.session = SessionState::AwaitingVerification {
        info: info.clone(),
        gate,
    };
    state.errors.push(crate::state::AppError {
        code: "e2ee_recovery_failed".to_owned(),
        message,
        recoverable: true,
        reason: None,
    });
    vec![
        AppEffect::EmitUiEvent(UiEvent::SessionChanged),
        AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
    ]
}

pub(crate) fn handle_e2ee_recovery_state_changed(
    state: &mut AppState,
    recovery_state: crate::state::E2eeRecoveryState,
    methods: Vec<crate::state::RecoveryMethod>,
) -> Vec<AppEffect> {
    match recovery_state {
        crate::state::E2eeRecoveryState::Unknown => Vec::new(),
        crate::state::E2eeRecoveryState::Incomplete => {
            let Some(info) = super::current_session_info(state) else {
                return Vec::new();
            };
            if !has_verification_gate_projection_context(state) {
                return Vec::new();
            }
            state.session = SessionState::AwaitingVerification {
                info,
                gate: recovery_gate(methods, None),
            };
            vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
        }
        crate::state::E2eeRecoveryState::Enabled | crate::state::E2eeRecoveryState::Disabled => {
            if matches!(state.session, SessionState::Verifying { .. }) {
                return Vec::new();
            }
            let info = match &state.session {
                SessionState::AwaitingVerification { info, .. } => info.clone(),
                _ => return Vec::new(),
            };
            state.session = SessionState::Provisional {
                info,
                phase: ProvisionalPhase::RecheckingTrust { failure: None },
            };
            vec![
                AppEffect::CheckCurrentDeviceTrust,
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            ]
        }
    }
}

pub(crate) fn handle_verification_requested(
    state: &mut AppState,
    request_id: u64,
    target: VerificationTarget,
    initiator: crate::state::VerificationInitiator,
) -> Vec<AppEffect> {
    if !is_session_ready(state) || state.e2ee_trust.verification.is_in_progress() {
        return Vec::new();
    }

    state.e2ee_trust.verification = VerificationFlowState::Requested {
        request_id,
        target: target.clone(),
        initiator,
    };
    vec![
        AppEffect::RequestVerification { request_id, target },
        AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged),
    ]
}

pub(crate) fn handle_verification_accepted(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    let VerificationFlowState::Requested {
        target, initiator, ..
    } = &state.e2ee_trust.verification
    else {
        return Vec::new();
    };
    if verification_request_id(&state.e2ee_trust.verification) != Some(request_id) {
        return Vec::new();
    }

    // For our own request this is the other side's acceptance, projected by
    // the account actor from the SDK request state.
    state.e2ee_trust.verification = VerificationFlowState::Accepted {
        request_id,
        target: target.clone(),
        initiator: *initiator,
    };
    vec![
        AppEffect::AcceptVerification { request_id },
        AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged),
    ]
}

pub(crate) fn handle_verification_sas_presented(
    state: &mut AppState,
    request_id: u64,
    emojis: Vec<SasEmoji>,
) -> Vec<AppEffect> {
    if !matches!(
        state.e2ee_trust.verification,
        VerificationFlowState::Requested { .. }
            | VerificationFlowState::Accepted { .. }
            | VerificationFlowState::SasPresented { .. }
    ) {
        return Vec::new();
    }
    let Some(target) = verification_target(&state.e2ee_trust.verification) else {
        return Vec::new();
    };
    if verification_request_id(&state.e2ee_trust.verification) != Some(request_id) {
        return Vec::new();
    }

    state.e2ee_trust.verification = VerificationFlowState::SasPresented {
        request_id,
        target,
        emojis,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
}

pub(crate) fn handle_verification_confirmed(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    let VerificationFlowState::SasPresented { .. } = &state.e2ee_trust.verification else {
        return Vec::new();
    };
    let Some(target) = verification_target(&state.e2ee_trust.verification) else {
        return Vec::new();
    };
    if verification_request_id(&state.e2ee_trust.verification) != Some(request_id) {
        return Vec::new();
    }
    let emojis = verification_emojis(&state.e2ee_trust.verification);

    state.e2ee_trust.verification = VerificationFlowState::Confirming {
        request_id,
        target,
        emojis,
    };
    vec![
        AppEffect::ConfirmSasVerification { request_id },
        AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged),
    ]
}

pub(crate) fn handle_verification_cancelled(
    state: &mut AppState,
    request_id: u64,
    reason: VerificationCancelReason,
) -> Vec<AppEffect> {
    if let SessionState::Verifying {
        info,
        gate,
        flow_id,
        ..
    } = &state.session
        && *flow_id == request_id
    {
        let mut gate = gate.clone();
        gate.failure = Some(match reason {
            VerificationCancelReason::User => VerificationGateFailureKind::Cancelled,
            VerificationCancelReason::Mismatch => VerificationGateFailureKind::Mismatch,
        });
        state.session = SessionState::AwaitingVerification {
            info: info.clone(),
            gate,
        };
        return vec![
            AppEffect::CancelVerification { request_id, reason },
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
        ];
    }
    if !verification_is_active(&state.e2ee_trust.verification)
        || verification_request_id(&state.e2ee_trust.verification) != Some(request_id)
    {
        return Vec::new();
    }

    state.e2ee_trust.verification = match reason {
        VerificationCancelReason::User => VerificationFlowState::Idle,
        VerificationCancelReason::Mismatch => {
            if !matches!(
                state.e2ee_trust.verification,
                VerificationFlowState::SasPresented { .. }
                    | VerificationFlowState::Confirming { .. }
            ) {
                return Vec::new();
            }
            let Some(target) = verification_target(&state.e2ee_trust.verification) else {
                return Vec::new();
            };
            VerificationFlowState::Failed {
                request_id,
                target,
                kind: TrustOperationFailureKind::Mismatch,
            }
        }
    };
    vec![
        AppEffect::CancelVerification { request_id, reason },
        AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged),
    ]
}

pub(crate) fn handle_verification_completed(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    if let SessionState::Verifying {
        info,
        method: VerificationMethod::ExistingDeviceSas,
        flow_id,
        ..
    } = &state.session
        && *flow_id == request_id
    {
        state.session = SessionState::Provisional {
            info: info.clone(),
            phase: ProvisionalPhase::RecheckingTrust { failure: None },
        };
        return vec![
            AppEffect::CheckCurrentDeviceTrust,
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
        ];
    }
    if !verification_is_active(&state.e2ee_trust.verification)
        || verification_request_id(&state.e2ee_trust.verification) != Some(request_id)
    {
        return Vec::new();
    }
    let Some(target) = verification_target(&state.e2ee_trust.verification) else {
        return Vec::new();
    };

    state.e2ee_trust.verification = VerificationFlowState::Done { request_id, target };
    vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
}

pub(crate) fn handle_verification_failed(
    state: &mut AppState,
    request_id: u64,
    kind: TrustOperationFailureKind,
) -> Vec<AppEffect> {
    if let SessionState::Verifying {
        info,
        gate,
        method: VerificationMethod::ExistingDeviceSas,
        flow_id,
        ..
    } = &state.session
        && *flow_id == request_id
    {
        let mut gate = gate.clone();
        gate.failure = Some(match kind {
            TrustOperationFailureKind::Cancelled => VerificationGateFailureKind::Cancelled,
            TrustOperationFailureKind::Mismatch => VerificationGateFailureKind::Mismatch,
            TrustOperationFailureKind::InvalidPassphrase => VerificationGateFailureKind::Sdk,
            TrustOperationFailureKind::Network => VerificationGateFailureKind::Network,
            TrustOperationFailureKind::Forbidden => VerificationGateFailureKind::Forbidden,
            TrustOperationFailureKind::Timeout => VerificationGateFailureKind::Timeout,
            TrustOperationFailureKind::Sdk => VerificationGateFailureKind::Sdk,
        });
        state.session = SessionState::AwaitingVerification {
            info: info.clone(),
            gate,
        };
        return vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)];
    }
    if !verification_is_active(&state.e2ee_trust.verification)
        || verification_request_id(&state.e2ee_trust.verification) != Some(request_id)
    {
        return Vec::new();
    }
    let Some(target) = verification_target(&state.e2ee_trust.verification) else {
        return Vec::new();
    };

    state.e2ee_trust.verification = VerificationFlowState::Failed {
        request_id,
        target,
        kind,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
}

pub(crate) fn handle_cross_signing_status_changed(
    state: &mut AppState,
    status: CrossSigningStatus,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    if matches!(
        state.e2ee_trust.cross_signing,
        CrossSigningStatus::Bootstrapping { .. }
    ) && !matches!(status, CrossSigningStatus::Trusted)
    {
        return Vec::new();
    }

    state.e2ee_trust.cross_signing = status;
    vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
}

pub(crate) fn handle_bootstrap_cross_signing_requested(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    if !is_session_ready(state)
        || matches!(
            state.e2ee_trust.cross_signing,
            CrossSigningStatus::Bootstrapping { .. }
        )
    {
        return Vec::new();
    }

    state.e2ee_trust.cross_signing = CrossSigningStatus::Bootstrapping { request_id };
    vec![
        AppEffect::BootstrapCrossSigning { request_id },
        AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged),
    ]
}

pub(crate) fn handle_bootstrap_cross_signing_failed(
    state: &mut AppState,
    request_id: u64,
    kind: TrustOperationFailureKind,
) -> Vec<AppEffect> {
    if state.e2ee_trust.cross_signing != (CrossSigningStatus::Bootstrapping { request_id }) {
        return Vec::new();
    }

    state.e2ee_trust.cross_signing = CrossSigningStatus::Failed { request_id, kind };
    vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
}

pub(crate) fn handle_enable_key_backup_requested(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    if !is_session_ready(state)
        || matches!(
            state.e2ee_trust.key_backup,
            KeyBackupStatus::Enabling { .. } | KeyBackupStatus::Restoring { .. }
        )
    {
        return Vec::new();
    }

    state.e2ee_trust.key_backup = KeyBackupStatus::Enabling { request_id };
    vec![
        AppEffect::EnableKeyBackup { request_id },
        AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged),
    ]
}

pub(crate) fn handle_key_backup_enabled(
    state: &mut AppState,
    request_id: u64,
    version: String,
) -> Vec<AppEffect> {
    if state.e2ee_trust.key_backup != (KeyBackupStatus::Enabling { request_id }) {
        return Vec::new();
    }

    state.e2ee_trust.key_backup = KeyBackupStatus::Enabled { version };
    vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
}

pub(crate) fn handle_key_backup_failed(
    state: &mut AppState,
    request_id: u64,
    kind: TrustOperationFailureKind,
) -> Vec<AppEffect> {
    if !key_backup_request_matches(&state.e2ee_trust.key_backup, request_id) {
        return Vec::new();
    }

    state.e2ee_trust.key_backup = KeyBackupStatus::Failed { request_id, kind };
    vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
}

pub(crate) fn handle_restore_key_backup_requested(
    state: &mut AppState,
    request_id: u64,
    version: Option<String>,
) -> Vec<AppEffect> {
    if !is_session_ready(state)
        || matches!(
            state.e2ee_trust.key_backup,
            KeyBackupStatus::Enabling { .. } | KeyBackupStatus::Restoring { .. }
        )
    {
        return Vec::new();
    }

    state.e2ee_trust.key_backup = KeyBackupStatus::Restoring {
        request_id,
        version: version.clone(),
        restored_rooms: 0,
        total_rooms: None,
    };
    vec![
        AppEffect::RestoreKeyBackup {
            request_id,
            version,
        },
        AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged),
    ]
}

pub(crate) fn handle_key_backup_restore_progress(
    state: &mut AppState,
    request_id: u64,
    restored_rooms: u64,
    total_rooms: Option<u64>,
) -> Vec<AppEffect> {
    let KeyBackupStatus::Restoring { version, .. } = &state.e2ee_trust.key_backup else {
        return Vec::new();
    };
    if !key_backup_request_matches(&state.e2ee_trust.key_backup, request_id) {
        return Vec::new();
    }

    state.e2ee_trust.key_backup = KeyBackupStatus::Restoring {
        request_id,
        version: version.clone(),
        restored_rooms,
        total_rooms,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
}

pub(crate) fn handle_key_backup_restored(
    state: &mut AppState,
    request_id: u64,
    version: Option<String>,
) -> Vec<AppEffect> {
    if !key_backup_restore_request_matches(&state.e2ee_trust.key_backup, request_id) {
        return Vec::new();
    }

    state.e2ee_trust.key_backup = match version {
        Some(version) => KeyBackupStatus::Enabled { version },
        None => KeyBackupStatus::Unknown,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
}

pub(crate) fn handle_reset_identity_requested(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    if !is_session_ready(state)
        || matches!(
            state.e2ee_trust.identity_reset,
            IdentityResetState::Resetting { .. } | IdentityResetState::AwaitingAuth { .. }
        )
    {
        return Vec::new();
    }

    state.e2ee_trust.identity_reset = IdentityResetState::Resetting { request_id };
    vec![
        AppEffect::ResetIdentity { request_id },
        AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged),
    ]
}

pub(crate) fn handle_reset_identity_auth_required(
    state: &mut AppState,
    request_id: u64,
    auth_type: crate::state::IdentityResetAuthType,
) -> Vec<AppEffect> {
    if state.e2ee_trust.identity_reset != (IdentityResetState::Resetting { request_id }) {
        return Vec::new();
    }

    state.e2ee_trust.identity_reset = IdentityResetState::AwaitingAuth {
        request_id,
        auth_type,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
}

pub(crate) fn handle_reset_identity_auth_submitted(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    if !matches!(
        state.e2ee_trust.identity_reset,
        IdentityResetState::AwaitingAuth {
            request_id: current_request_id,
            ..
        } if current_request_id == request_id
    ) {
        return Vec::new();
    }

    state.e2ee_trust.identity_reset = IdentityResetState::Resetting { request_id };
    vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
}

pub(crate) fn handle_reset_identity_cancelled(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    handle_reset_identity_failed(state, request_id, TrustOperationFailureKind::Cancelled)
}

pub(crate) fn handle_reset_identity_timed_out(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    handle_reset_identity_failed(state, request_id, TrustOperationFailureKind::Timeout)
}

pub(crate) fn handle_reset_identity_completed(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    if !identity_reset_request_matches(&state.e2ee_trust.identity_reset, request_id) {
        return Vec::new();
    }

    state.e2ee_trust.identity_reset = IdentityResetState::Idle;
    state.e2ee_trust.verification = VerificationFlowState::Idle;
    state.e2ee_trust.cross_signing = CrossSigningStatus::Missing;
    state.e2ee_trust.key_backup = KeyBackupStatus::Disabled;
    vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
}

pub(crate) fn handle_reset_identity_failed(
    state: &mut AppState,
    request_id: u64,
    kind: TrustOperationFailureKind,
) -> Vec<AppEffect> {
    if !identity_reset_request_matches(&state.e2ee_trust.identity_reset, request_id) {
        return Vec::new();
    }

    state.e2ee_trust.identity_reset = IdentityResetState::Failed { request_id, kind };
    state.e2ee_trust.cross_signing = CrossSigningStatus::Failed { request_id, kind };
    vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
}

pub(crate) fn handle_room_key_export_requested(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    if !is_session_ready(state)
        || matches!(
            state.e2ee_trust.key_management.room_key_export,
            RoomKeyExportState::Exporting { .. }
        )
    {
        return Vec::new();
    }
    state.e2ee_trust.key_management.room_key_export = RoomKeyExportState::Exporting { request_id };
    e2ee_key_management_events()
}

pub(crate) fn handle_room_key_exported(
    state: &mut AppState,
    request_id: u64,
    exported_sessions: Option<u64>,
) -> Vec<AppEffect> {
    if !matches!(
        state.e2ee_trust.key_management.room_key_export,
        RoomKeyExportState::Exporting {
            request_id: active
        } if active == request_id
    ) {
        return Vec::new();
    }
    state.e2ee_trust.key_management.room_key_export = RoomKeyExportState::Exported {
        request_id,
        exported_sessions,
    };
    e2ee_key_management_events()
}

pub(crate) fn handle_room_key_export_failed(
    state: &mut AppState,
    request_id: u64,
    kind: TrustOperationFailureKind,
) -> Vec<AppEffect> {
    if !matches!(
        state.e2ee_trust.key_management.room_key_export,
        RoomKeyExportState::Exporting {
            request_id: active
        } if active == request_id
    ) {
        return Vec::new();
    }
    state.e2ee_trust.key_management.room_key_export =
        RoomKeyExportState::Failed { request_id, kind };
    e2ee_key_management_events()
}

pub(crate) fn handle_room_key_import_requested(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    if !is_session_ready(state)
        || matches!(
            state.e2ee_trust.key_management.room_key_import,
            RoomKeyImportState::Importing { .. }
        )
    {
        return Vec::new();
    }
    state.e2ee_trust.key_management.room_key_import = RoomKeyImportState::Importing { request_id };
    e2ee_key_management_events()
}

pub(crate) fn handle_room_key_imported(
    state: &mut AppState,
    request_id: u64,
    imported_count: u64,
    total_count: u64,
) -> Vec<AppEffect> {
    if !matches!(
        state.e2ee_trust.key_management.room_key_import,
        RoomKeyImportState::Importing {
            request_id: active
        } if active == request_id
    ) {
        return Vec::new();
    }
    state.e2ee_trust.key_management.room_key_import = RoomKeyImportState::Imported {
        request_id,
        imported_count,
        total_count,
    };
    e2ee_key_management_events()
}

pub(crate) fn handle_room_key_import_failed(
    state: &mut AppState,
    request_id: u64,
    kind: TrustOperationFailureKind,
) -> Vec<AppEffect> {
    if !matches!(
        state.e2ee_trust.key_management.room_key_import,
        RoomKeyImportState::Importing {
            request_id: active
        } if active == request_id
    ) {
        return Vec::new();
    }
    state.e2ee_trust.key_management.room_key_import =
        RoomKeyImportState::Failed { request_id, kind };
    e2ee_key_management_events()
}

/// At most one recovery key flow runs at a time (#927): AccountActor holds a
/// single revealed-key copy, so setup (including re-enable and key reset) and
/// passphrase change exclude each other from admission until the revealed
/// key is confirmed, and also while either is still in flight.
fn recovery_key_flow_busy(state: &AppState) -> bool {
    matches!(
        state.e2ee_trust.key_management.secure_backup_setup,
        SecureBackupSetupState::SettingUp { .. } | SecureBackupSetupState::RecoveryKeyReady { .. }
    ) || matches!(
        state.e2ee_trust.key_management.passphrase_change,
        SecureBackupPassphraseChangeState::Changing { .. }
            | SecureBackupPassphraseChangeState::Changed { .. }
    )
}

pub(crate) fn handle_secure_backup_setup_requested(
    state: &mut AppState,
    request_id: u64,
    intent: crate::state::SecureBackupSetupIntent,
) -> Vec<AppEffect> {
    if !is_session_ready(state)
        || recovery_key_flow_busy(state)
        || !matches!(
            intent.admission(&state.secure_backup_gate),
            crate::state::SecureBackupSetupAdmission::Allowed
        )
    {
        return Vec::new();
    }
    state.e2ee_trust.key_management.secure_backup_setup =
        SecureBackupSetupState::SettingUp { request_id };
    e2ee_key_management_events()
}

pub(crate) fn handle_secure_backup_recovery_key_ready(
    state: &mut AppState,
    request_id: u64,
    recovery_key: crate::state::RecoveryKeyMaterial,
) -> Vec<AppEffect> {
    if !matches!(
        state.e2ee_trust.key_management.secure_backup_setup,
        SecureBackupSetupState::SettingUp {
            request_id: active
        } if active == request_id
    ) {
        return Vec::new();
    }
    state.e2ee_trust.key_management.secure_backup_setup =
        SecureBackupSetupState::RecoveryKeyReady {
            request_id,
            recovery_key,
            delivery: crate::state::RecoveryKeyDeliveryState::NotWritten,
            confirmation_failed: false,
        };
    e2ee_key_management_events()
}

pub(crate) fn handle_secure_backup_setup_failed(
    state: &mut AppState,
    request_id: u64,
    kind: TrustOperationFailureKind,
) -> Vec<AppEffect> {
    if !matches!(
        state.e2ee_trust.key_management.secure_backup_setup,
        SecureBackupSetupState::SettingUp {
            request_id: active
        } if active == request_id
    ) {
        return Vec::new();
    }
    state.e2ee_trust.key_management.secure_backup_setup =
        SecureBackupSetupState::Failed { request_id, kind };
    e2ee_key_management_events()
}

pub(crate) fn handle_secure_backup_passphrase_change_requested(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    if !is_session_ready(state) || recovery_key_flow_busy(state) {
        return Vec::new();
    }
    state.e2ee_trust.key_management.passphrase_change =
        SecureBackupPassphraseChangeState::Changing { request_id };
    e2ee_key_management_events()
}

pub(crate) fn handle_secure_backup_passphrase_changed(
    state: &mut AppState,
    request_id: u64,
    recovery_key: crate::state::RecoveryKeyMaterial,
) -> Vec<AppEffect> {
    if !matches!(
        state.e2ee_trust.key_management.passphrase_change,
        SecureBackupPassphraseChangeState::Changing {
            request_id: active
        } if active == request_id
    ) {
        return Vec::new();
    }
    state.e2ee_trust.key_management.passphrase_change =
        SecureBackupPassphraseChangeState::Changed {
            request_id,
            recovery_key,
            delivery: crate::state::RecoveryKeyDeliveryState::NotWritten,
        };
    e2ee_key_management_events()
}

/// Records the optional "Save to file" outcome. Saving never leaves the
/// reveal state and never changes the Secure Backup gate.
pub(crate) fn handle_secure_backup_recovery_key_saved(
    state: &mut AppState,
    reveal_request_id: u64,
    written: bool,
) -> Vec<AppEffect> {
    let outcome = if written {
        crate::state::RecoveryKeyDeliveryState::Written
    } else {
        crate::state::RecoveryKeyDeliveryState::WriteFailed
    };
    let key_management = &mut state.e2ee_trust.key_management;
    let delivery = match (
        &mut key_management.secure_backup_setup,
        &mut key_management.passphrase_change,
    ) {
        (
            SecureBackupSetupState::RecoveryKeyReady {
                request_id,
                delivery,
                ..
            },
            _,
        ) if *request_id == reveal_request_id => delivery,
        (
            _,
            SecureBackupPassphraseChangeState::Changed {
                request_id,
                delivery,
                ..
            },
        ) if *request_id == reveal_request_id => delivery,
        _ => return Vec::new(),
    };
    // A failed retry must not erase an earlier successful write.
    if *delivery == outcome || *delivery == crate::state::RecoveryKeyDeliveryState::Written {
        return Vec::new();
    }
    *delivery = outcome;
    e2ee_key_management_events()
}

/// The explicit "I saved the recovery key" confirmation. It drops the key
/// and, for setup, hands the gate back to authoritative inspection.
pub(crate) fn handle_secure_backup_recovery_key_confirmed(
    state: &mut AppState,
    reveal_request_id: u64,
) -> Vec<AppEffect> {
    let key_management = &mut state.e2ee_trust.key_management;
    if matches!(
        key_management.secure_backup_setup,
        SecureBackupSetupState::RecoveryKeyReady { request_id, .. }
            if request_id == reveal_request_id
    ) {
        key_management.secure_backup_setup = SecureBackupSetupState::Enabled {
            request_id: reveal_request_id,
        };
        let mut effects = e2ee_key_management_events();
        if state.secure_backup_gate
            == crate::state::SecureBackupGateState::RecoveryKeyDeliveryRequired
        {
            state.secure_backup_gate = crate::state::SecureBackupGateState::Checking;
            effects.push(AppEffect::EmitUiEvent(UiEvent::SessionChanged));
        }
        return effects;
    }
    if matches!(
        key_management.passphrase_change,
        SecureBackupPassphraseChangeState::Changed { request_id, .. }
            if request_id == reveal_request_id
    ) {
        key_management.passphrase_change = SecureBackupPassphraseChangeState::Idle;
        return e2ee_key_management_events();
    }
    Vec::new()
}

/// Restores the setup reveal after AccountActor failed to clear the
/// persisted delivery marker. Applies only while the matching confirmation
/// is the latest setup transition and no other key flow has started.
pub(crate) fn handle_secure_backup_recovery_key_confirm_failed(
    state: &mut AppState,
    reveal_request_id: u64,
    recovery_key: crate::state::RecoveryKeyMaterial,
    delivery: crate::state::RecoveryKeyDeliveryState,
) -> Vec<AppEffect> {
    if !is_session_ready(state)
        || recovery_key_flow_busy(state)
        || !matches!(
            state.e2ee_trust.key_management.secure_backup_setup,
            SecureBackupSetupState::Enabled { request_id } if request_id == reveal_request_id
        )
    {
        return Vec::new();
    }
    state.e2ee_trust.key_management.secure_backup_setup =
        SecureBackupSetupState::RecoveryKeyReady {
            request_id: reveal_request_id,
            recovery_key,
            delivery,
            confirmation_failed: true,
        };
    let mut effects = e2ee_key_management_events();
    if state.secure_backup_gate != crate::state::SecureBackupGateState::RecoveryKeyDeliveryRequired
    {
        state.secure_backup_gate = crate::state::SecureBackupGateState::RecoveryKeyDeliveryRequired;
        effects.push(AppEffect::EmitUiEvent(UiEvent::SessionChanged));
    }
    effects
}

pub(crate) fn handle_secure_backup_passphrase_change_failed(
    state: &mut AppState,
    request_id: u64,
    kind: TrustOperationFailureKind,
) -> Vec<AppEffect> {
    if !matches!(
        state.e2ee_trust.key_management.passphrase_change,
        SecureBackupPassphraseChangeState::Changing {
            request_id: active
        } if active == request_id
    ) {
        return Vec::new();
    }
    state.e2ee_trust.key_management.passphrase_change =
        SecureBackupPassphraseChangeState::Failed { request_id, kind };
    e2ee_key_management_events()
}

pub(crate) fn handle_qr_login_capability_check_requested(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    if matches!(
        state.qr_login,
        QrLoginState::CheckingCapability { .. }
            | QrLoginState::Displaying { .. }
            | QrLoginState::Scanning { .. }
    ) {
        return Vec::new();
    }
    state.qr_login = QrLoginState::CheckingCapability { request_id };
    vec![AppEffect::EmitUiEvent(UiEvent::QrLoginChanged)]
}

pub(crate) fn handle_qr_login_unavailable(state: &mut AppState, request_id: u64) -> Vec<AppEffect> {
    if !matches!(
        state.qr_login,
        QrLoginState::CheckingCapability {
            request_id: active
        } if active == request_id
    ) {
        return Vec::new();
    }
    state.qr_login = QrLoginState::Unavailable;
    vec![AppEffect::EmitUiEvent(UiEvent::QrLoginChanged)]
}

pub(crate) fn handle_qr_login_display_requested(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    if matches!(
        state.qr_login,
        QrLoginState::Displaying { .. } | QrLoginState::Scanning { .. }
    ) {
        return Vec::new();
    }
    state.qr_login = QrLoginState::Displaying { request_id };
    vec![AppEffect::EmitUiEvent(UiEvent::QrLoginChanged)]
}

pub(crate) fn handle_qr_login_scan_started(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    if !matches!(
        state.qr_login,
        QrLoginState::Displaying {
            request_id: active
        } if active == request_id
    ) {
        return Vec::new();
    }
    state.qr_login = QrLoginState::Scanning { request_id };
    vec![AppEffect::EmitUiEvent(UiEvent::QrLoginChanged)]
}

pub(crate) fn handle_qr_login_verified(state: &mut AppState, request_id: u64) -> Vec<AppEffect> {
    if !matches!(
        state.qr_login,
        QrLoginState::Displaying {
            request_id: active
        }
        | QrLoginState::Scanning {
            request_id: active
        } if active == request_id
    ) {
        return Vec::new();
    }
    state.qr_login = QrLoginState::Verified { request_id };
    vec![AppEffect::EmitUiEvent(UiEvent::QrLoginChanged)]
}

pub(crate) fn handle_qr_login_failed(
    state: &mut AppState,
    request_id: u64,
    kind: AuthFailureKind,
) -> Vec<AppEffect> {
    if !matches!(
        state.qr_login,
        QrLoginState::CheckingCapability {
            request_id: active
        }
        | QrLoginState::Displaying {
            request_id: active
        }
        | QrLoginState::Scanning {
            request_id: active
        } if active == request_id
    ) {
        return Vec::new();
    }
    state.qr_login = QrLoginState::Failed { request_id, kind };
    vec![AppEffect::EmitUiEvent(UiEvent::QrLoginChanged)]
}

// --- Private helpers ---

fn verification_request_id(verification: &VerificationFlowState) -> Option<u64> {
    match verification {
        VerificationFlowState::Idle => None,
        VerificationFlowState::Requested { request_id, .. }
        | VerificationFlowState::Accepted { request_id, .. }
        | VerificationFlowState::SasPresented { request_id, .. }
        | VerificationFlowState::Confirming { request_id, .. }
        | VerificationFlowState::Done { request_id, .. }
        | VerificationFlowState::Failed { request_id, .. } => Some(*request_id),
    }
}

fn verification_target(verification: &VerificationFlowState) -> Option<VerificationTarget> {
    match verification {
        VerificationFlowState::Idle => None,
        VerificationFlowState::Requested { target, .. }
        | VerificationFlowState::Accepted { target, .. }
        | VerificationFlowState::SasPresented { target, .. }
        | VerificationFlowState::Confirming { target, .. }
        | VerificationFlowState::Done { target, .. }
        | VerificationFlowState::Failed { target, .. } => Some(target.clone()),
    }
}

fn verification_emojis(verification: &VerificationFlowState) -> Vec<SasEmoji> {
    match verification {
        VerificationFlowState::SasPresented { emojis, .. }
        | VerificationFlowState::Confirming { emojis, .. } => emojis.clone(),
        VerificationFlowState::Idle
        | VerificationFlowState::Requested { .. }
        | VerificationFlowState::Accepted { .. }
        | VerificationFlowState::Done { .. }
        | VerificationFlowState::Failed { .. } => Vec::new(),
    }
}

fn verification_is_active(verification: &VerificationFlowState) -> bool {
    matches!(
        verification,
        VerificationFlowState::Requested { .. }
            | VerificationFlowState::Accepted { .. }
            | VerificationFlowState::SasPresented { .. }
            | VerificationFlowState::Confirming { .. }
    )
}

fn key_backup_request_matches(key_backup: &KeyBackupStatus, request_id: u64) -> bool {
    matches!(
        key_backup,
        KeyBackupStatus::Enabling {
            request_id: current_request_id,
        } | KeyBackupStatus::Restoring {
            request_id: current_request_id,
            ..
        } if *current_request_id == request_id
    )
}

fn key_backup_restore_request_matches(key_backup: &KeyBackupStatus, request_id: u64) -> bool {
    matches!(
        key_backup,
        KeyBackupStatus::Restoring {
            request_id: current_request_id,
            ..
        } if *current_request_id == request_id
    )
}

fn identity_reset_request_matches(identity_reset: &IdentityResetState, request_id: u64) -> bool {
    matches!(
        identity_reset,
        IdentityResetState::Resetting {
            request_id: current_request_id,
        } | IdentityResetState::AwaitingAuth {
            request_id: current_request_id,
            ..
        } if *current_request_id == request_id
    )
}

fn e2ee_key_management_events() -> Vec<AppEffect> {
    vec![
        AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged),
        AppEffect::EmitUiEvent(UiEvent::E2eeKeyManagementChanged),
    ]
}
