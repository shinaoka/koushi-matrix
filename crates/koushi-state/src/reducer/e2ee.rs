use crate::{
    effect::{AppEffect, UiEvent},
    state::{
        AppState, AuthFailureKind, CrossSigningStatus, IdentityResetState, KeyBackupStatus,
        ProvisionalPhase, QrLoginState, RoomKeyExportState, RoomKeyImportState, SasEmoji,
        SecureBackupPassphraseChangeState, SecureBackupSetupState, SessionState,
        TrustOperationFailureKind, VerificationAccountKind, VerificationCancelReason,
        VerificationFlowState, VerificationGateFailureKind, VerificationGateState,
        VerificationMethod, VerificationMethodCapability, VerificationTarget,
    },
};

use super::{
    clear_login_failed_errors, clear_session_views, has_verification_gate_projection_context,
    is_session_ready,
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

pub(crate) fn handle_e2ee_recovery_submitted(
    state: &mut AppState,
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
    state.session = SessionState::Verifying {
        info: info.clone(),
        gate: gate.clone(),
        method,
        flow_id: 0,
    };
    vec![
        AppEffect::RecoverE2ee(request),
        AppEffect::EmitUiEvent(UiEvent::SessionChanged),
    ]
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
    state.session = SessionState::Provisional {
        info,
        phase: ProvisionalPhase::RecheckingTrust { failure: None },
    };
    vec![
        AppEffect::CheckCurrentDeviceTrust,
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
    state.session = SessionState::AwaitingVerification {
        info: info.clone(),
        gate,
    };
    state.errors.push(crate::state::AppError {
        code: "e2ee_recovery_failed".to_owned(),
        message,
        recoverable: true,
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
            let info = match &state.session {
                SessionState::AwaitingVerification { info, .. }
                | SessionState::Verifying { info, .. } => info.clone(),
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
) -> Vec<AppEffect> {
    if !is_session_ready(state)
        || !matches!(
            state.e2ee_trust.verification,
            VerificationFlowState::Idle
                | VerificationFlowState::Done { .. }
                | VerificationFlowState::Failed { .. }
        )
    {
        return Vec::new();
    }

    state.e2ee_trust.verification = VerificationFlowState::Requested {
        request_id,
        target: target.clone(),
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
    let VerificationFlowState::Requested { target, .. } = &state.e2ee_trust.verification else {
        return Vec::new();
    };
    if verification_request_id(&state.e2ee_trust.verification) != Some(request_id) {
        return Vec::new();
    }

    state.e2ee_trust.verification = VerificationFlowState::Accepted {
        request_id,
        target: target.clone(),
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

pub(crate) fn handle_secure_backup_setup_requested(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    if !is_session_ready(state)
        || matches!(
            state.e2ee_trust.key_management.secure_backup_setup,
            SecureBackupSetupState::SettingUp { .. }
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
    delivery: crate::state::RecoveryKeyDeliveryState,
) -> Vec<AppEffect> {
    if !matches!(
        state.e2ee_trust.key_management.secure_backup_setup,
        SecureBackupSetupState::SettingUp {
            request_id: active
        }
        | SecureBackupSetupState::RecoveryKeyReady {
            request_id: active,
            ..
        } if active == request_id
    ) {
        return Vec::new();
    }
    state.e2ee_trust.key_management.secure_backup_setup =
        SecureBackupSetupState::RecoveryKeyReady {
            request_id,
            delivery,
        };
    e2ee_key_management_events()
}

pub(crate) fn handle_secure_backup_setup_enabled(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    if !matches!(
        state.e2ee_trust.key_management.secure_backup_setup,
        SecureBackupSetupState::SettingUp {
            request_id: active
        }
        | SecureBackupSetupState::RecoveryKeyReady {
            request_id: active,
            ..
        } if active == request_id
    ) {
        return Vec::new();
    }
    state.e2ee_trust.key_management.secure_backup_setup =
        SecureBackupSetupState::Enabled { request_id };
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
        }
        | SecureBackupSetupState::RecoveryKeyReady {
            request_id: active,
            ..
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
    if !is_session_ready(state)
        || matches!(
            state.e2ee_trust.key_management.passphrase_change,
            SecureBackupPassphraseChangeState::Changing { .. }
        )
    {
        return Vec::new();
    }
    state.e2ee_trust.key_management.passphrase_change =
        SecureBackupPassphraseChangeState::Changing { request_id };
    e2ee_key_management_events()
}

pub(crate) fn handle_secure_backup_passphrase_changed(
    state: &mut AppState,
    request_id: u64,
    delivery: crate::state::RecoveryKeyDeliveryState,
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
            delivery,
        };
    e2ee_key_management_events()
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
