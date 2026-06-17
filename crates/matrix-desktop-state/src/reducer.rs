use crate::{
    action::AppAction,
    effect::{AppEffect, UiEvent},
    state::{
        AccountManagementState, ActivityMarkReadState, ActivityState, ActivityStream, ActivityTab,
        AppError, AppState, AttachmentScope, BasicOperationRequest, BasicOperationState,
        ComposerMode, CrossSigningStatus, DeviceSessionListState, DirectoryJoinState,
        DirectoryQueryState, DirectoryState, E2eeKeyManagementState, E2eeRecoveryState,
        E2eeTrustState, FilesViewScope, FilesViewState, FocusedContextState, IdentityResetState,
        KeyBackupStatus, LocalEncryptionState, NavigationState, OperationFailureKind,
        PendingComposerSendKind, PinOp, PinOperationState, QrLoginState, RoomKeyExportState,
        RoomKeyImportState, RoomListFilter, RoomManagementOperationKind,
        RoomManagementOperationState, RoomMemberRole, RoomModerationAction, SasEmoji, SearchState,
        SecureBackupPassphraseChangeState, SecureBackupSetupState, SessionState,
        SettingsPersistenceState, StagedUploadCompressionChoice, SyncState, ThreadAttentionState,
        ThreadPaneState, TimelinePaneState, TrustOperationFailureKind, VerificationCancelReason,
        VerificationFlowState, VerificationTarget, compute_room_list_projection,
    },
};

use std::collections::BTreeSet;

const TIMELINE_SUBSCRIPTION_FAILED_MESSAGE: &str = "Matrix timeline subscription failed";
const SETTINGS_LOAD_FAILED_MESSAGE: &str = "Settings could not be loaded";
const SETTINGS_PERSIST_FAILED_MESSAGE: &str = "Settings could not be saved";
const LOCAL_USER_ALIAS_UPDATE_FAILED_MESSAGE: &str = "Local user alias could not be saved";
const PIN_EVENT_FAILED_MESSAGE: &str = "Pinning the event failed";
const UNPIN_EVENT_FAILED_MESSAGE: &str = "Unpinning the event failed";

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
        AppAction::E2eeRecoveryRequired { info, methods } => {
            state.session = SessionState::NeedsRecovery {
                info: info.clone(),
                methods,
            };
            state.sync = SyncState::Starting;
            vec![
                AppEffect::PersistSession(info),
                AppEffect::StartSync,
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            ]
        }
        AppAction::E2eeRecoverySubmitted(request) => {
            let SessionState::NeedsRecovery { info, methods } = &state.session else {
                return Vec::new();
            };
            state.session = SessionState::Recovering {
                info: info.clone(),
                methods: methods.clone(),
            };
            vec![
                AppEffect::RecoverE2ee(request),
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            ]
        }
        AppAction::E2eeRecoverySucceeded => {
            let SessionState::Recovering { info, .. } = &state.session else {
                return Vec::new();
            };
            let info = info.clone();
            state.session = SessionState::Ready(info.clone());
            state.sync = SyncState::Starting;
            vec![
                AppEffect::PersistSession(info),
                AppEffect::StartSync,
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            ]
        }
        AppAction::E2eeRecoveryFailed { message } => {
            let SessionState::Recovering { info, methods } = &state.session else {
                return Vec::new();
            };
            state.session = SessionState::NeedsRecovery {
                info: info.clone(),
                methods: methods.clone(),
            };
            state.errors.push(AppError {
                code: "e2ee_recovery_failed".to_owned(),
                message,
                recoverable: true,
            });
            vec![
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
                AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
            ]
        }
        AppAction::E2eeRecoveryStateChanged {
            state: recovery_state,
            methods,
        } => match recovery_state {
            E2eeRecoveryState::Unknown => Vec::new(),
            E2eeRecoveryState::Incomplete => {
                let SessionState::Ready(info) = &state.session else {
                    return Vec::new();
                };
                let info = info.clone();
                state.session = SessionState::NeedsRecovery { info, methods };
                vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
            }
            E2eeRecoveryState::Enabled | E2eeRecoveryState::Disabled => {
                let info = match &state.session {
                    SessionState::NeedsRecovery { info, .. }
                    | SessionState::Recovering { info, .. } => info.clone(),
                    _ => return Vec::new(),
                };
                state.session = SessionState::Ready(info.clone());
                state.sync = SyncState::Starting;
                vec![
                    AppEffect::PersistSession(info),
                    AppEffect::StartSync,
                    AppEffect::EmitUiEvent(UiEvent::SessionChanged),
                ]
            }
        },
        AppAction::VerificationRequested { request_id, target } => {
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
        AppAction::VerificationAccepted { request_id } => {
            let VerificationFlowState::Requested { target, .. } = &state.e2ee_trust.verification
            else {
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
        AppAction::VerificationSasPresented { request_id, emojis } => {
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
        AppAction::VerificationConfirmed { request_id } => {
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
        AppAction::VerificationCancelled { request_id, reason } => {
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
        AppAction::VerificationCompleted { request_id } => {
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
        AppAction::VerificationFailed { request_id, kind } => {
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
        AppAction::CrossSigningStatusChanged { status } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            state.e2ee_trust.cross_signing = status;
            vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
        }
        AppAction::BootstrapCrossSigningRequested { request_id } => {
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
        AppAction::BootstrapCrossSigningFailed { request_id, kind } => {
            if state.e2ee_trust.cross_signing != (CrossSigningStatus::Bootstrapping { request_id })
            {
                return Vec::new();
            }

            state.e2ee_trust.cross_signing = CrossSigningStatus::Failed { request_id, kind };
            vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
        }
        AppAction::EnableKeyBackupRequested { request_id } => {
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
        AppAction::KeyBackupEnabled {
            request_id,
            version,
        } => {
            if state.e2ee_trust.key_backup != (KeyBackupStatus::Enabling { request_id }) {
                return Vec::new();
            }

            state.e2ee_trust.key_backup = KeyBackupStatus::Enabled { version };
            vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
        }
        AppAction::KeyBackupFailed { request_id, kind } => {
            if !key_backup_request_matches(&state.e2ee_trust.key_backup, request_id) {
                return Vec::new();
            }

            state.e2ee_trust.key_backup = KeyBackupStatus::Failed { request_id, kind };
            vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
        }
        AppAction::RestoreKeyBackupRequested {
            request_id,
            version,
        } => {
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
        AppAction::KeyBackupRestoreProgress {
            request_id,
            restored_rooms,
            total_rooms,
        } => {
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
        AppAction::KeyBackupRestored {
            request_id,
            version,
        } => {
            if !key_backup_restore_request_matches(&state.e2ee_trust.key_backup, request_id) {
                return Vec::new();
            }

            state.e2ee_trust.key_backup = match version {
                Some(version) => KeyBackupStatus::Enabled { version },
                None => KeyBackupStatus::Unknown,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
        }
        AppAction::ResetIdentityRequested { request_id } => {
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
        AppAction::ResetIdentityAuthRequired {
            request_id,
            auth_type,
        } => {
            if state.e2ee_trust.identity_reset != (IdentityResetState::Resetting { request_id }) {
                return Vec::new();
            }

            state.e2ee_trust.identity_reset = IdentityResetState::AwaitingAuth {
                request_id,
                auth_type,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
        }
        AppAction::ResetIdentityAuthSubmitted { request_id } => {
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
        AppAction::ResetIdentityCompleted { request_id } => {
            if !identity_reset_request_matches(&state.e2ee_trust.identity_reset, request_id) {
                return Vec::new();
            }

            state.e2ee_trust.identity_reset = IdentityResetState::Idle;
            state.e2ee_trust.verification = VerificationFlowState::Idle;
            state.e2ee_trust.cross_signing = CrossSigningStatus::Missing;
            state.e2ee_trust.key_backup = KeyBackupStatus::Disabled;
            vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
        }
        AppAction::ResetIdentityFailed { request_id, kind } => {
            if !identity_reset_request_matches(&state.e2ee_trust.identity_reset, request_id) {
                return Vec::new();
            }

            state.e2ee_trust.identity_reset = IdentityResetState::Failed { request_id, kind };
            state.e2ee_trust.cross_signing = CrossSigningStatus::Failed { request_id, kind };
            vec![AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged)]
        }
        AppAction::RoomKeyExportRequested { request_id } => {
            if !is_session_ready(state)
                || matches!(
                    state.e2ee_trust.key_management.room_key_export,
                    RoomKeyExportState::Exporting { .. }
                )
            {
                return Vec::new();
            }
            state.e2ee_trust.key_management.room_key_export =
                RoomKeyExportState::Exporting { request_id };
            e2ee_key_management_events()
        }
        AppAction::RoomKeyExported {
            request_id,
            exported_sessions,
        } => {
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
        AppAction::RoomKeyExportFailed { request_id, kind } => {
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
        AppAction::RoomKeyImportRequested { request_id } => {
            if !is_session_ready(state)
                || matches!(
                    state.e2ee_trust.key_management.room_key_import,
                    RoomKeyImportState::Importing { .. }
                )
            {
                return Vec::new();
            }
            state.e2ee_trust.key_management.room_key_import =
                RoomKeyImportState::Importing { request_id };
            e2ee_key_management_events()
        }
        AppAction::RoomKeyImported {
            request_id,
            imported_count,
            total_count,
        } => {
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
        AppAction::RoomKeyImportFailed { request_id, kind } => {
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
        AppAction::SecureBackupSetupRequested { request_id } => {
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
        AppAction::SecureBackupRecoveryKeyReady {
            request_id,
            delivery,
        } => {
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
        AppAction::SecureBackupSetupEnabled { request_id } => {
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
        AppAction::SecureBackupSetupFailed { request_id, kind } => {
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
        AppAction::SecureBackupPassphraseChangeRequested { request_id } => {
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
        AppAction::SecureBackupPassphraseChanged {
            request_id,
            delivery,
        } => {
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
        AppAction::SecureBackupPassphraseChangeFailed { request_id, kind } => {
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
        AppAction::QrLoginCapabilityCheckRequested { request_id } => {
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
        AppAction::QrLoginUnavailable { request_id } => {
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
        AppAction::QrLoginDisplayRequested { request_id } => {
            if matches!(
                state.qr_login,
                QrLoginState::Displaying { .. } | QrLoginState::Scanning { .. }
            ) {
                return Vec::new();
            }
            state.qr_login = QrLoginState::Displaying { request_id };
            vec![AppEffect::EmitUiEvent(UiEvent::QrLoginChanged)]
        }
        AppAction::QrLoginScanStarted { request_id } => {
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
        AppAction::QrLoginVerified { request_id } => {
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
        AppAction::QrLoginFailed { request_id, kind } => {
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
        AppAction::RestoreSessionNotFound => {
            state.session = SessionState::SignedOut;
            vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
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
        AppAction::SwitchAccountRequested { info } => {
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
        AppAction::LoginDiscoveryRequested { homeserver } => {
            state.auth = crate::state::AuthDiscoveryState::Discovering {
                homeserver: homeserver.clone(),
            };
            vec![
                AppEffect::DiscoverLogin { homeserver },
                AppEffect::EmitUiEvent(UiEvent::AuthChanged),
            ]
        }
        AppAction::LoginDiscoverySucceeded {
            homeserver,
            flows,
            delegated,
        } => {
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
        AppAction::LoginDiscoveryFailed { homeserver, kind } => {
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
        AppAction::DeviceSessionsLoadRequested { request_id } => {
            if !is_session_ready(state)
                || matches!(
                    state.device_sessions,
                    DeviceSessionListState::Loading { .. }
                )
            {
                return Vec::new();
            }
            state.device_sessions = DeviceSessionListState::Loading { request_id };
            vec![AppEffect::EmitUiEvent(UiEvent::DeviceSessionsChanged)]
        }
        AppAction::DeviceSessionsLoaded {
            request_id,
            devices,
        } => {
            if !matches!(
                state.device_sessions,
                DeviceSessionListState::Loading {
                    request_id: active
                } if active == request_id
            ) {
                return Vec::new();
            }
            state.device_sessions = DeviceSessionListState::Loaded { devices };
            vec![AppEffect::EmitUiEvent(UiEvent::DeviceSessionsChanged)]
        }
        AppAction::DeviceSessionsLoadFailed { request_id, kind } => {
            if !matches!(
                state.device_sessions,
                DeviceSessionListState::Loading {
                    request_id: active
                } if active == request_id
            ) {
                return Vec::new();
            }
            state.device_sessions = DeviceSessionListState::Failed { request_id, kind };
            vec![AppEffect::EmitUiEvent(UiEvent::DeviceSessionsChanged)]
        }
        AppAction::AccountManagementRequested {
            request_id,
            operation,
        } => {
            if !is_session_ready(state)
                || matches!(
                    state.account_management,
                    AccountManagementState::Working { .. }
                        | AccountManagementState::AwaitingUia { .. }
                )
            {
                return Vec::new();
            }
            state.account_management = AccountManagementState::Working {
                request_id,
                operation,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::AccountManagementChanged)]
        }
        AppAction::AccountManagementUiaRequired {
            request_id,
            flow_id,
            operation,
        } => {
            if !matches!(
                state.account_management,
                AccountManagementState::Working {
                    request_id: active,
                    operation: active_operation,
                } if active == request_id && active_operation == operation
            ) {
                return Vec::new();
            }
            state.account_management = AccountManagementState::AwaitingUia {
                request_id,
                flow_id,
                operation,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::AccountManagementChanged)]
        }
        AppAction::AccountManagementSucceeded {
            request_id,
            operation,
        } => {
            if !account_management_matches(&state.account_management, request_id, operation) {
                return Vec::new();
            }
            state.account_management = AccountManagementState::Succeeded {
                request_id,
                operation,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::AccountManagementChanged)]
        }
        AppAction::AccountManagementFailed {
            request_id,
            operation,
            kind,
        } => {
            if !account_management_matches(&state.account_management, request_id, operation) {
                return Vec::new();
            }
            state.account_management = AccountManagementState::Failed {
                request_id,
                operation,
                kind,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::AccountManagementChanged)]
        }
        AppAction::SettingsLoaded { values } => {
            state.settings.values = values;
            state.settings.persistence = SettingsPersistenceState::Idle;
            vec![AppEffect::EmitUiEvent(UiEvent::SettingsChanged)]
        }
        AppAction::SettingsLoadFailed { message: _ } => {
            state.settings.persistence = SettingsPersistenceState::Idle;
            state.errors.push(AppError {
                code: "settings_load_failed".to_owned(),
                message: SETTINGS_LOAD_FAILED_MESSAGE.to_owned(),
                recoverable: true,
            });
            vec![
                AppEffect::EmitUiEvent(UiEvent::SettingsChanged),
                AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
            ]
        }
        AppAction::SettingsUpdateRequested { request_id, patch } => {
            state.settings.values.apply_patch(patch);
            state.settings.persistence = SettingsPersistenceState::Saving { request_id };
            vec![
                AppEffect::PersistSettings {
                    request_id,
                    values: state.settings.values.clone(),
                },
                AppEffect::EmitUiEvent(UiEvent::SettingsChanged),
            ]
        }
        AppAction::SettingsPersisted { request_id } => {
            if state.settings.persistence != (SettingsPersistenceState::Saving { request_id }) {
                return Vec::new();
            }

            state.settings.persistence = SettingsPersistenceState::Idle;
            vec![AppEffect::EmitUiEvent(UiEvent::SettingsChanged)]
        }
        AppAction::SettingsPersistFailed {
            request_id,
            message: _,
        } => {
            if state.settings.persistence != (SettingsPersistenceState::Saving { request_id }) {
                return Vec::new();
            }

            state.settings.persistence = SettingsPersistenceState::Idle;
            state.errors.push(AppError {
                code: "settings_persist_failed".to_owned(),
                message: SETTINGS_PERSIST_FAILED_MESSAGE.to_owned(),
                recoverable: true,
            });
            vec![
                AppEffect::EmitUiEvent(UiEvent::SettingsChanged),
                AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
            ]
        }
        AppAction::OwnProfileUpdated { profile } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let own_user_id = session_user_id(state).map(str::to_owned);
            state.profile.own = profile;
            crate::state::refresh_profile_user_display_projection(
                &mut state.profile,
                own_user_id.as_deref(),
            );
            let room_members_changed =
                refresh_open_room_settings_member_display_projection(state, own_user_id.as_deref());
            let room_list_changed =
                refresh_open_room_summary_display_projection(state, own_user_id.as_deref());
            let native_attention_changed =
                room_list_changed && refresh_native_attention_candidate_display_projection(state);
            profile_changed_effects(
                room_members_changed,
                room_list_changed,
                native_attention_changed,
            )
        }
        AppAction::UserProfilesUpdated { profiles } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let own_user_id = session_user_id(state).map(str::to_owned);
            state.profile.users = profiles
                .into_iter()
                .map(|profile| (profile.user_id.clone(), profile))
                .collect();
            crate::state::refresh_profile_user_display_projection(
                &mut state.profile,
                own_user_id.as_deref(),
            );
            let room_members_changed =
                refresh_open_room_settings_member_display_projection(state, own_user_id.as_deref());
            let room_list_changed =
                refresh_open_room_summary_display_projection(state, own_user_id.as_deref());
            let native_attention_changed =
                room_list_changed && refresh_native_attention_candidate_display_projection(state);
            profile_changed_effects(
                room_members_changed,
                room_list_changed,
                native_attention_changed,
            )
        }
        AppAction::LocalUserAliasesLoaded { aliases } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let own_user_id = session_user_id(state).map(str::to_owned);
            state.profile.local_aliases = aliases
                .into_iter()
                .filter_map(|(user_id, alias)| {
                    crate::state::normalize_local_user_alias(Some(alias))
                        .map(|normalized| (user_id, normalized))
                })
                .collect();
            state.profile.local_alias_update = crate::state::LocalUserAliasUpdateState::Idle;
            crate::state::refresh_profile_user_display_projection(
                &mut state.profile,
                own_user_id.as_deref(),
            );
            let room_members_changed =
                refresh_open_room_settings_member_display_projection(state, own_user_id.as_deref());
            let room_list_changed =
                refresh_open_room_summary_display_projection(state, own_user_id.as_deref());
            let native_attention_changed =
                room_list_changed && refresh_native_attention_candidate_display_projection(state);
            profile_changed_effects(
                room_members_changed,
                room_list_changed,
                native_attention_changed,
            )
        }
        AppAction::LocalUserAliasUpdateRequested {
            request_id,
            user_id,
            alias,
        } => {
            if !is_session_ready(state) || !state.profile.local_alias_update.is_idle() {
                return Vec::new();
            }

            let own_user_id = session_user_id(state).map(str::to_owned);
            if let Some(alias) = crate::state::normalize_local_user_alias(alias) {
                state.profile.local_aliases.insert(user_id, alias);
            } else {
                state.profile.local_aliases.remove(&user_id);
            }
            state.profile.local_alias_update =
                crate::state::LocalUserAliasUpdateState::Saving { request_id };
            crate::state::refresh_profile_user_display_projection(
                &mut state.profile,
                own_user_id.as_deref(),
            );
            let room_members_changed =
                refresh_open_room_settings_member_display_projection(state, own_user_id.as_deref());
            let room_list_changed =
                refresh_open_room_summary_display_projection(state, own_user_id.as_deref());
            let native_attention_changed =
                room_list_changed && refresh_native_attention_candidate_display_projection(state);
            profile_changed_effects(
                room_members_changed,
                room_list_changed,
                native_attention_changed,
            )
        }
        AppAction::LocalUserAliasUpdateSucceeded { request_id } => {
            if state.profile.local_alias_update.request_id() != Some(request_id) {
                return Vec::new();
            }

            state.profile.local_alias_update = crate::state::LocalUserAliasUpdateState::Idle;
            vec![AppEffect::EmitUiEvent(UiEvent::ProfileChanged)]
        }
        AppAction::LocalUserAliasUpdateFailed {
            request_id,
            message: _,
        } => {
            if state.profile.local_alias_update.request_id() != Some(request_id) {
                return Vec::new();
            }

            state.profile.local_alias_update = crate::state::LocalUserAliasUpdateState::Idle;
            state.errors.push(AppError {
                code: "local_user_alias_update_failed".to_owned(),
                message: LOCAL_USER_ALIAS_UPDATE_FAILED_MESSAGE.to_owned(),
                recoverable: true,
            });
            vec![
                AppEffect::EmitUiEvent(UiEvent::ProfileChanged),
                AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
            ]
        }
        AppAction::ProfileUpdateRequested {
            request_id,
            request,
        } => {
            if !is_session_ready(state) || !state.profile.update.is_idle() {
                return Vec::new();
            }

            state.profile.update = match request {
                crate::state::ProfileUpdateRequest::SetDisplayName { display_name } => {
                    crate::state::ProfileUpdateState::SettingDisplayName {
                        request_id,
                        display_name,
                    }
                }
                crate::state::ProfileUpdateRequest::SetAvatar {
                    mime_type,
                    byte_count,
                } => crate::state::ProfileUpdateState::SettingAvatar {
                    request_id,
                    mime_type,
                    byte_count,
                },
            };
            vec![AppEffect::EmitUiEvent(UiEvent::ProfileChanged)]
        }
        AppAction::ProfileUpdateSucceeded {
            request_id,
            profile,
        } => {
            if state.profile.update.request_id() != Some(request_id) {
                return Vec::new();
            }

            let own_user_id = session_user_id(state).map(str::to_owned);
            state.profile.update = crate::state::ProfileUpdateState::Idle;
            state.profile.own = profile;
            crate::state::refresh_profile_user_display_projection(
                &mut state.profile,
                own_user_id.as_deref(),
            );
            let room_members_changed =
                refresh_open_room_settings_member_display_projection(state, own_user_id.as_deref());
            let room_list_changed =
                refresh_open_room_summary_display_projection(state, own_user_id.as_deref());
            let native_attention_changed =
                room_list_changed && refresh_native_attention_candidate_display_projection(state);
            profile_changed_effects(
                room_members_changed,
                room_list_changed,
                native_attention_changed,
            )
        }
        AppAction::ProfileUpdateFailed {
            request_id,
            message,
        } => {
            if state.profile.update.request_id() != Some(request_id) {
                return Vec::new();
            }

            state.profile.update = crate::state::ProfileUpdateState::Idle;
            state.errors.push(AppError {
                code: "profile_update_failed".to_owned(),
                message,
                recoverable: true,
            });
            vec![
                AppEffect::EmitUiEvent(UiEvent::ProfileChanged),
                AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
            ]
        }
        AppAction::LoginSubmitted(request) => {
            state.session = SessionState::Authenticating {
                homeserver: request.homeserver.clone(),
            };
            vec![
                AppEffect::Login(request),
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            ]
        }
        AppAction::LoginFailed { message } => {
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
        AppAction::SessionPersistenceFailed { message } => {
            state.errors.push(AppError {
                code: "session_persistence_failed".to_owned(),
                message,
                recoverable: true,
            });
            vec![AppEffect::EmitUiEvent(UiEvent::ErrorChanged)]
        }
        AppAction::SessionLocked => {
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
        AppAction::LogoutRequested => {
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
        AppAction::LogoutFinished => {
            *state = AppState::default();
            vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
        }
        AppAction::SyncStarted => {
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
        AppAction::SyncFailed { reason } => {
            if !is_session_ready(state) || matches!(state.sync, SyncState::Stopped) {
                return Vec::new();
            }

            state.sync = SyncState::Failed { reason };
            vec![
                AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
                AppEffect::StartSync,
            ]
        }
        AppAction::SyncReconnecting { reason } => {
            if !is_session_ready(state)
                || matches!(state.sync, SyncState::Stopped | SyncState::Running)
            {
                return Vec::new();
            }

            state.sync = SyncState::Reconnecting { reason };
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        }
        AppAction::SyncRecovered => {
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
        AppAction::SyncStopped => {
            if matches!(state.sync, SyncState::Stopped) {
                return Vec::new();
            }

            state.sync = SyncState::Stopped;
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        }
        AppAction::SyncModeChanged { mode } => {
            if state.sync_mode == mode {
                return Vec::new();
            }

            state.sync_mode = mode;
            vec![AppEffect::EmitUiEvent(UiEvent::SyncModeChanged)]
        }
        AppAction::RoomListUpdated { spaces, rooms } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let own_user_id = session_user_id(state).map(str::to_owned);
            let mut rooms = rooms;
            crate::state::refresh_room_summary_display_projection(
                &mut rooms,
                &state.profile,
                own_user_id.as_deref(),
            );
            let retained_room_ids = rooms
                .iter()
                .map(|room| room.room_id.clone())
                .collect::<BTreeSet<_>>();
            let had_active_room_before_update = state.navigation.active_room_id.is_some();
            state.spaces = spaces;
            state.rooms = rooms;
            state.room_list = compute_room_list_projection(
                state.room_list.active_filter,
                state.room_list.sort,
                &state.rooms,
                &state.invites,
            );
            state.composer_drafts.retain_rooms(&retained_room_ids);
            state.scheduled_sends.retain_rooms(&retained_room_ids);
            state.upload_staging.retain_rooms(&retained_room_ids);
            state.media_gallery.retain_rooms(&retained_room_ids);
            refresh_timeline_scheduled_sends(state);
            refresh_timeline_upload_staging(state);
            refresh_timeline_media_gallery(state);

            let mut effects = vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)];

            if state
                .navigation
                .active_space_id
                .as_deref()
                .is_some_and(|active_space_id| {
                    !state
                        .spaces
                        .iter()
                        .any(|space| space.space_id == active_space_id)
                })
            {
                state.navigation.active_space_id = None;
            }

            if let Some(active_room_id) = state.navigation.active_room_id.clone() {
                let room_still_exists = state
                    .rooms
                    .iter()
                    .any(|room| room.room_id == active_room_id);

                if !room_still_exists {
                    state.navigation.active_room_id = None;
                    let previous_room_id = state.timeline.room_id.clone().unwrap_or(active_room_id);
                    let had_thread = state.thread != ThreadPaneState::Closed
                        || state.thread_attention != ThreadAttentionState::Closed;

                    state.timeline = Default::default();
                    state.thread = ThreadPaneState::Closed;
                    state.thread_attention = ThreadAttentionState::Closed;

                    effects.push(AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                        room_id: previous_room_id,
                    }));
                    if had_thread {
                        effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadChanged));
                    }
                }
            }

            if had_active_room_before_update
                && state.navigation.active_room_id.is_none()
                && state.navigation.active_space_id.is_some()
                && let Some(room_id) = first_room_id_in_active_space(state)
            {
                select_active_room_after_room_list_update(state, &mut effects, room_id);
            }

            if let Some(active_room_id) = state.navigation.active_room_id.clone()
                && active_room_left_selected_space(state, &active_room_id)
            {
                retarget_active_room_for_selected_space(state, &mut effects, active_room_id);
            }

            if !had_active_room_before_update
                && state.navigation.active_room_id.is_none()
                && let Some(room_id) = first_default_room_id(state)
            {
                state.navigation.active_room_id = Some(room_id.clone());
                state.timeline = TimelinePaneState {
                    room_id: Some(room_id.clone()),
                    is_subscribed: false,
                    is_paginating_backwards: false,
                    composer: state.composer_drafts.composer_for_room(&room_id),
                    scheduled_send_capability: state.scheduled_sends.capability.clone(),
                    scheduled_sends: state.scheduled_sends.items_for_room(&room_id),
                    staged_uploads: state.upload_staging.items_for_room(&room_id),
                    media_gallery: state.media_gallery.items_for_room(&room_id),
                };
                effects.push(AppEffect::SubscribeTimeline {
                    room_id: room_id.clone(),
                });
                effects.push(AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }));
            }

            effects
        }
        AppAction::RoomListFilterSelected { filter } => {
            if !is_session_ready(state) || state.room_list.active_filter == filter {
                return Vec::new();
            }

            state.room_list = compute_room_list_projection(
                filter,
                state.room_list.sort,
                &state.rooms,
                &state.invites,
            );
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        }
        AppAction::RoomListFilterApplied { projection } => {
            if !is_session_ready(state) || state.room_list == projection {
                return Vec::new();
            }

            state.room_list = projection;
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        }
        AppAction::RoomTagsUpdated { room_id, tags } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let Some(room) = state.rooms.iter_mut().find(|room| room.room_id == room_id) else {
                return Vec::new();
            };

            if room.tags == tags {
                return Vec::new();
            }

            room.tags = tags;
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        }
        AppAction::RoomTagSet { room_id, tag, info } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let Some(room) = state.rooms.iter_mut().find(|room| room.room_id == room_id) else {
                return Vec::new();
            };

            let mut tags = room.tags.clone();
            tags.set(tag, info);
            if room.tags == tags {
                return Vec::new();
            }

            room.tags = tags;
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        }
        AppAction::RoomTagRemoved { room_id, tag } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let Some(room) = state.rooms.iter_mut().find(|room| room.room_id == room_id) else {
                return Vec::new();
            };

            let mut tags = room.tags.clone();
            tags.remove(tag);
            if room.tags == tags {
                return Vec::new();
            }

            room.tags = tags;
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        }
        AppAction::RoomPinnedEventsUpdated { room_id, pinned } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let entry = state.room_interactions.entry(room_id).or_default();
            if entry.pinned_events == pinned {
                return Vec::new();
            }

            entry.pinned_events = pinned;
            vec![AppEffect::EmitUiEvent(UiEvent::RoomInteractionsChanged)]
        }
        AppAction::PinEventRequested {
            request_id,
            room_id,
            event_id,
        } => {
            if !is_session_ready(state) || event_id.is_empty() || !room_exists(state, &room_id) {
                return Vec::new();
            }

            let entry = state.room_interactions.entry(room_id.clone()).or_default();
            if !entry.pin_operation.accepts_new_request() {
                return Vec::new();
            }

            entry.pin_operation = PinOperationState::Pending {
                request_id,
                room_id,
                event_id,
                op: PinOp::Pin,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::RoomInteractionsChanged)]
        }
        AppAction::PinEventCompleted {
            request_id,
            room_id,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let Some(entry) = state.room_interactions.get_mut(&room_id) else {
                return Vec::new();
            };
            if !matches!(
                entry.pin_operation,
                PinOperationState::Pending {
                    request_id: pending_request_id,
                    op: PinOp::Pin,
                    ..
                } if pending_request_id == request_id
            ) {
                return Vec::new();
            }

            entry.pin_operation = PinOperationState::Idle;
            vec![AppEffect::EmitUiEvent(UiEvent::RoomInteractionsChanged)]
        }
        AppAction::PinEventFailed {
            request_id,
            room_id,
            kind: _,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let Some(entry) = state.room_interactions.get_mut(&room_id) else {
                return Vec::new();
            };
            let PinOperationState::Pending {
                request_id: pending_request_id,
                event_id,
                op: PinOp::Pin,
                ..
            } = &entry.pin_operation
            else {
                return Vec::new();
            };
            if *pending_request_id != request_id {
                return Vec::new();
            };
            let event_id = event_id.clone();

            entry.pin_operation = PinOperationState::Failed {
                room_id,
                event_id,
                op: PinOp::Pin,
                recoverable: true,
            };
            state.errors.push(AppError {
                code: "pin_event_failed".to_owned(),
                message: PIN_EVENT_FAILED_MESSAGE.to_owned(),
                recoverable: true,
            });
            vec![
                AppEffect::EmitUiEvent(UiEvent::RoomInteractionsChanged),
                AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
            ]
        }
        AppAction::UnpinEventRequested {
            request_id,
            room_id,
            event_id,
        } => {
            if !is_session_ready(state) || event_id.is_empty() || !room_exists(state, &room_id) {
                return Vec::new();
            }

            let entry = state.room_interactions.entry(room_id.clone()).or_default();
            if !entry.pin_operation.accepts_new_request() {
                return Vec::new();
            }

            entry.pin_operation = PinOperationState::Pending {
                request_id,
                room_id,
                event_id,
                op: PinOp::Unpin,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::RoomInteractionsChanged)]
        }
        AppAction::UnpinEventCompleted {
            request_id,
            room_id,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let Some(entry) = state.room_interactions.get_mut(&room_id) else {
                return Vec::new();
            };
            if !matches!(
                entry.pin_operation,
                PinOperationState::Pending {
                    request_id: pending_request_id,
                    op: PinOp::Unpin,
                    ..
                } if pending_request_id == request_id
            ) {
                return Vec::new();
            }

            entry.pin_operation = PinOperationState::Idle;
            vec![AppEffect::EmitUiEvent(UiEvent::RoomInteractionsChanged)]
        }
        AppAction::UnpinEventFailed {
            request_id,
            room_id,
            kind: _,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let Some(entry) = state.room_interactions.get_mut(&room_id) else {
                return Vec::new();
            };
            let PinOperationState::Pending {
                request_id: pending_request_id,
                event_id,
                op: PinOp::Unpin,
                ..
            } = &entry.pin_operation
            else {
                return Vec::new();
            };
            if *pending_request_id != request_id {
                return Vec::new();
            };
            let event_id = event_id.clone();

            entry.pin_operation = PinOperationState::Failed {
                room_id,
                event_id,
                op: PinOp::Unpin,
                recoverable: true,
            };
            state.errors.push(AppError {
                code: "unpin_event_failed".to_owned(),
                message: UNPIN_EVENT_FAILED_MESSAGE.to_owned(),
                recoverable: true,
            });
            vec![
                AppEffect::EmitUiEvent(UiEvent::RoomInteractionsChanged),
                AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
            ]
        }
        AppAction::RoomMarkedAsReadRequested {
            request_id,
            room_id,
            event_id: _,
        } => {
            if !is_session_ready(state) || !room_exists(state, &room_id) {
                return Vec::new();
            }

            let _ = request_id;
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        }
        AppAction::RoomMarkedAsReadSucceeded {
            request_id,
            room_id,
        } => {
            if !is_session_ready(state) || !room_exists(state, &room_id) {
                return Vec::new();
            }

            let _ = request_id;
            if let Some(room) = state.rooms.iter_mut().find(|room| room.room_id == room_id) {
                room.marked_unread = false;
                room.unread_count = 0;
                room.notification_count = 0;
                room.highlight_count = 0;
                state.room_list = compute_room_list_projection(
                    state.room_list.active_filter,
                    state.room_list.sort,
                    &state.rooms,
                    &state.invites,
                );
            }
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        }
        AppAction::RoomMarkedAsReadFailed {
            request_id,
            room_id,
            kind: _,
        } => {
            if !is_session_ready(state) || !room_exists(state, &room_id) {
                return Vec::new();
            }

            let _ = request_id;
            vec![AppEffect::EmitUiEvent(UiEvent::ErrorChanged)]
        }
        AppAction::RoomMarkedAsUnreadRequested {
            request_id,
            room_id,
            unread,
        } => {
            if !is_session_ready(state) || !room_exists(state, &room_id) {
                return Vec::new();
            }

            let _ = (request_id, unread);
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        }
        AppAction::RoomMarkedAsUnreadSucceeded {
            request_id,
            room_id,
            unread,
        } => {
            if !is_session_ready(state) || !room_exists(state, &room_id) {
                return Vec::new();
            }

            let _ = request_id;
            if let Some(room) = state.rooms.iter_mut().find(|room| room.room_id == room_id) {
                room.marked_unread = unread;
                if unread && room.unread_count == 0 {
                    room.unread_count = 1;
                }
                if !unread {
                    room.unread_count = 0;
                }
                state.room_list = compute_room_list_projection(
                    state.room_list.active_filter,
                    state.room_list.sort,
                    &state.rooms,
                    &state.invites,
                );
            }
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        }
        AppAction::RoomMarkedAsUnreadFailed {
            request_id,
            room_id,
            kind: _,
        } => {
            if !is_session_ready(state) || !room_exists(state, &room_id) {
                return Vec::new();
            }

            let _ = request_id;
            vec![AppEffect::EmitUiEvent(UiEvent::ErrorChanged)]
        }
        AppAction::DirectoryQueryRequested { request_id, query } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            state.directory.query = DirectoryQueryState::Querying { request_id, query };
            vec![AppEffect::EmitUiEvent(UiEvent::DirectoryChanged)]
        }
        AppAction::DirectoryQuerySucceeded {
            request_id,
            query,
            rooms,
            next_batch,
        } => {
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
        AppAction::DirectoryQueryFailed {
            request_id,
            query,
            kind,
        } => {
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
        AppAction::DirectoryJoinRequested {
            request_id,
            alias,
            via_server,
        } => {
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
        AppAction::DirectoryJoinSucceeded {
            request_id,
            room_id: _,
        } => {
            if !matches!(
                &state.directory.join,
                DirectoryJoinState::Joining {
                    request_id: current_request_id,
                    ..
                } if *current_request_id == request_id
            ) {
                return Vec::new();
            }

            state.directory.join = DirectoryJoinState::Idle;
            vec![AppEffect::EmitUiEvent(UiEvent::DirectoryChanged)]
        }
        AppAction::DirectoryJoinFailed {
            request_id,
            alias,
            via_server,
            kind,
        } => {
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
        AppAction::RoomSettingsSnapshotLoaded { room_id, settings } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let own_user_id = session_user_id(state).map(str::to_owned);
            let mut settings = settings;
            crate::state::refresh_room_settings_member_display_projection(
                &mut settings,
                &state.profile,
                own_user_id.as_deref(),
            );
            state.room_management.selected_room_id = Some(room_id);
            state.room_management.settings = Some(settings);
            state.room_management.operation = RoomManagementOperationState::Idle;
            vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
        }
        AppAction::RoomSettingUpdateRequested {
            request_id,
            room_id,
            change: _,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            if !room_settings_permission_allows(state, &room_id) {
                state.room_management.operation = RoomManagementOperationState::Failed {
                    request_id,
                    room_id,
                    operation: RoomManagementOperationKind::Settings,
                    kind: OperationFailureKind::Forbidden,
                };
                return vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)];
            }

            state.room_management.operation = RoomManagementOperationState::Pending {
                request_id,
                room_id,
                operation: RoomManagementOperationKind::Settings,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
        }
        AppAction::RoomSettingUpdateSucceeded {
            request_id,
            room_id,
            settings,
        } => {
            if !room_management_operation_matches(
                state,
                request_id,
                &room_id,
                RoomManagementOperationKind::Settings,
            ) {
                return Vec::new();
            }

            let own_user_id = session_user_id(state).map(str::to_owned);
            let mut settings = settings;
            crate::state::refresh_room_settings_member_display_projection(
                &mut settings,
                &state.profile,
                own_user_id.as_deref(),
            );
            state.room_management.selected_room_id = Some(room_id);
            state.room_management.settings = Some(settings);
            state.room_management.operation = RoomManagementOperationState::Idle;
            vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
        }
        AppAction::RoomSettingUpdateFailed {
            request_id,
            room_id,
            kind,
        } => {
            if !room_management_operation_matches(
                state,
                request_id,
                &room_id,
                RoomManagementOperationKind::Settings,
            ) {
                return Vec::new();
            }

            state.room_management.operation = RoomManagementOperationState::Failed {
                request_id,
                room_id,
                operation: RoomManagementOperationKind::Settings,
                kind,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
        }
        AppAction::RoomModerationRequested {
            request_id,
            room_id,
            target_user_id: _,
            action,
            reason: _,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            if !room_moderation_permission_allows(state, &room_id, action) {
                state.room_management.operation = RoomManagementOperationState::Failed {
                    request_id,
                    room_id,
                    operation: RoomManagementOperationKind::Moderation,
                    kind: OperationFailureKind::Forbidden,
                };
                return vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)];
            }

            state.room_management.operation = RoomManagementOperationState::Pending {
                request_id,
                room_id,
                operation: RoomManagementOperationKind::Moderation,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
        }
        AppAction::RoomModerationSucceeded {
            request_id,
            room_id,
            target_user_id,
            action,
        } => {
            if !room_management_operation_matches(
                state,
                request_id,
                &room_id,
                RoomManagementOperationKind::Moderation,
            ) {
                return Vec::new();
            }

            if matches!(
                action,
                RoomModerationAction::Kick | RoomModerationAction::Ban
            ) && let Some(settings) = state.room_management.settings.as_mut()
                && settings.room_id == room_id
            {
                settings
                    .members
                    .retain(|member| member.user_id != target_user_id);
            }
            state.room_management.operation = RoomManagementOperationState::Idle;
            vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
        }
        AppAction::RoomModerationFailed {
            request_id,
            room_id,
            target_user_id: _,
            action: _,
            kind,
        } => {
            if !room_management_operation_matches(
                state,
                request_id,
                &room_id,
                RoomManagementOperationKind::Moderation,
            ) {
                return Vec::new();
            }

            state.room_management.operation = RoomManagementOperationState::Failed {
                request_id,
                room_id,
                operation: RoomManagementOperationKind::Moderation,
                kind,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
        }
        AppAction::RoomMemberRoleUpdateRequested {
            request_id,
            room_id,
            target_user_id: _,
            power_level: _,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            if !room_role_permission_allows(state, &room_id) {
                state.room_management.operation = RoomManagementOperationState::Failed {
                    request_id,
                    room_id,
                    operation: RoomManagementOperationKind::Roles,
                    kind: OperationFailureKind::Forbidden,
                };
                return vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)];
            }

            state.room_management.operation = RoomManagementOperationState::Pending {
                request_id,
                room_id,
                operation: RoomManagementOperationKind::Roles,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
        }
        AppAction::RoomMemberRoleUpdateSucceeded {
            request_id,
            room_id,
            target_user_id,
            power_level,
        } => {
            if !room_management_operation_matches(
                state,
                request_id,
                &room_id,
                RoomManagementOperationKind::Roles,
            ) {
                return Vec::new();
            }

            if let Some(settings) = state.room_management.settings.as_mut()
                && settings.room_id == room_id
                && let Some(member) = settings
                    .members
                    .iter_mut()
                    .find(|member| member.user_id == target_user_id)
            {
                member.power_level = Some(power_level);
                member.role = RoomMemberRole::from_power_level(Some(power_level));
            }
            state.room_management.operation = RoomManagementOperationState::Idle;
            vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
        }
        AppAction::RoomMemberRoleUpdateFailed {
            request_id,
            room_id,
            target_user_id: _,
            kind,
        } => {
            if !room_management_operation_matches(
                state,
                request_id,
                &room_id,
                RoomManagementOperationKind::Roles,
            ) {
                return Vec::new();
            }

            state.room_management.operation = RoomManagementOperationState::Failed {
                request_id,
                room_id,
                operation: RoomManagementOperationKind::Roles,
                kind,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
        }
        AppAction::ActivityOpened { request_id } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            state.activity = ActivityState::Opening {
                request_id,
                tab: ActivityTab::Recent,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::ActivityChanged)]
        }
        AppAction::ActivityClosed => {
            if state.activity == ActivityState::Closed {
                return Vec::new();
            }

            state.activity = ActivityState::Closed;
            vec![AppEffect::EmitUiEvent(UiEvent::ActivityChanged)]
        }
        AppAction::ActivitySnapshotLoaded {
            request_id,
            active_tab,
            recent,
            unread,
            excluded_room_ids,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let ActivityState::Opening {
                request_id: current_request_id,
                ..
            } = state.activity
            else {
                return Vec::new();
            };
            if current_request_id != request_id {
                return Vec::new();
            }

            let excluded_room_ids: BTreeSet<_> = excluded_room_ids.into_iter().collect();
            state.activity = ActivityState::Open {
                active_tab,
                recent: normalize_activity_stream(recent, &excluded_room_ids),
                unread: normalize_activity_stream(unread, &excluded_room_ids),
                mark_read: ActivityMarkReadState::Idle,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::ActivityChanged)]
        }
        AppAction::ActivityRowsObserved { .. } => Vec::new(),
        AppAction::ActivityRowsUpdated {
            recent,
            unread,
            excluded_room_ids,
        } => {
            let ActivityState::Open {
                recent: current_recent,
                unread: current_unread,
                ..
            } = &mut state.activity
            else {
                return Vec::new();
            };

            let excluded_room_ids: BTreeSet<_> = excluded_room_ids.into_iter().collect();
            *current_recent = normalize_activity_stream(recent, &excluded_room_ids);
            *current_unread = normalize_activity_stream(unread, &excluded_room_ids);
            vec![AppEffect::EmitUiEvent(UiEvent::ActivityChanged)]
        }
        AppAction::ActivityTabSelected { tab } => {
            let ActivityState::Open { active_tab, .. } = &mut state.activity else {
                return Vec::new();
            };
            if *active_tab == tab {
                return Vec::new();
            }

            *active_tab = tab;
            vec![AppEffect::EmitUiEvent(UiEvent::ActivityChanged)]
        }
        AppAction::ActivityMarkReadRequested { request_id, target } => {
            let ActivityState::Open { mark_read, .. } = &mut state.activity else {
                return Vec::new();
            };

            *mark_read = ActivityMarkReadState::Pending { request_id, target };
            vec![AppEffect::EmitUiEvent(UiEvent::ActivityChanged)]
        }
        AppAction::ActivityMarkReadSucceeded {
            request_id,
            cleared_event_ids,
        } => {
            let ActivityState::Open {
                unread, mark_read, ..
            } = &mut state.activity
            else {
                return Vec::new();
            };
            if !matches!(
                mark_read,
                ActivityMarkReadState::Pending {
                    request_id: current_request_id,
                    ..
                } if *current_request_id == request_id
            ) {
                return Vec::new();
            }

            let cleared_event_ids: BTreeSet<_> = cleared_event_ids.into_iter().collect();
            unread
                .rows
                .retain(|row| !cleared_event_ids.contains(&row.event_id));
            *mark_read = ActivityMarkReadState::Idle;
            vec![AppEffect::EmitUiEvent(UiEvent::ActivityChanged)]
        }
        AppAction::ActivityMarkReadFailed {
            request_id,
            target,
            kind,
        } => {
            let ActivityState::Open { mark_read, .. } = &mut state.activity else {
                return Vec::new();
            };
            if !matches!(
                mark_read,
                ActivityMarkReadState::Pending {
                    request_id: current_request_id,
                    ..
                } if *current_request_id == request_id
            ) {
                return Vec::new();
            }

            *mark_read = ActivityMarkReadState::Failed {
                target,
                failure_kind: kind,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::ActivityChanged)]
        }
        AppAction::LocalEncryptionProbeRequested { request_id } => {
            let next = LocalEncryptionState::Probing { request_id };
            if state.local_encryption == next {
                return Vec::new();
            }

            state.local_encryption = next;
            vec![AppEffect::EmitUiEvent(UiEvent::LocalEncryptionChanged)]
        }
        AppAction::LocalEncryptionHealthChanged { request_id, health } => {
            let LocalEncryptionState::Probing {
                request_id: current_request_id,
            } = state.local_encryption
            else {
                return Vec::new();
            };
            if current_request_id != request_id {
                return Vec::new();
            }

            let next = LocalEncryptionState::from(health);
            if state.local_encryption == next {
                return Vec::new();
            }

            state.local_encryption = next;
            vec![AppEffect::EmitUiEvent(UiEvent::LocalEncryptionChanged)]
        }
        AppAction::ResetLocalDataRequested { request_id } => {
            if !matches!(
                state.local_encryption,
                LocalEncryptionState::MissingCredential | LocalEncryptionState::ResetRequired
            ) {
                return Vec::new();
            }

            state.local_encryption = LocalEncryptionState::Resetting { request_id };
            vec![AppEffect::EmitUiEvent(UiEvent::LocalEncryptionChanged)]
        }
        AppAction::ResetLocalDataCompleted { request_id } => {
            let LocalEncryptionState::Resetting {
                request_id: current_request_id,
            } = state.local_encryption
            else {
                return Vec::new();
            };
            if current_request_id != request_id {
                return Vec::new();
            }

            state.local_encryption = LocalEncryptionState::Unknown;
            vec![AppEffect::EmitUiEvent(UiEvent::LocalEncryptionChanged)]
        }
        AppAction::ResetLocalDataFailed { request_id } => {
            let LocalEncryptionState::Resetting {
                request_id: current_request_id,
            } = state.local_encryption
            else {
                return Vec::new();
            };
            if current_request_id != request_id {
                return Vec::new();
            }

            state.local_encryption = LocalEncryptionState::ResetRequired;
            vec![AppEffect::EmitUiEvent(UiEvent::LocalEncryptionChanged)]
        }
        AppAction::NativeAttentionUpdated { attention } => {
            if state.native_attention == attention {
                return Vec::new();
            }

            state.native_attention = attention;
            vec![AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged)]
        }
        AppAction::JapaneseCatalogProfileChanged { profile } => {
            if state.cjk_text_policy.japanese_catalog == profile {
                return Vec::new();
            }

            state.cjk_text_policy.japanese_catalog = profile;
            vec![AppEffect::EmitUiEvent(UiEvent::CjkTextPolicyChanged)]
        }
        AppAction::InviteListUpdated { invites } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            state.invites = invites;
            if state.room_list.active_filter == RoomListFilter::Invites {
                state.room_list = compute_room_list_projection(
                    RoomListFilter::Invites,
                    state.room_list.sort,
                    &state.rooms,
                    &state.invites,
                );
            }
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        }
        AppAction::SelectSpace { space_id } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            state.navigation.active_space_id = space_id
                .filter(|space_id| state.spaces.iter().any(|space| space.space_id == *space_id));
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        }
        AppAction::SelectRoom { room_id } => {
            if !is_session_ready(state) || !state.rooms.iter().any(|room| room.room_id == room_id) {
                return Vec::new();
            }

            let had_thread = state.thread != ThreadPaneState::Closed
                || state.thread_attention != ThreadAttentionState::Closed;
            state.navigation.active_room_id = Some(room_id.clone());
            state.timeline = TimelinePaneState {
                room_id: Some(room_id.clone()),
                is_subscribed: false,
                is_paginating_backwards: false,
                composer: state.composer_drafts.composer_for_room(&room_id),
                scheduled_send_capability: state.scheduled_sends.capability.clone(),
                scheduled_sends: state.scheduled_sends.items_for_room(&room_id),
                staged_uploads: state.upload_staging.items_for_room(&room_id),
                media_gallery: state.media_gallery.items_for_room(&room_id),
            };
            state.thread = ThreadPaneState::Closed;
            state.thread_attention = ThreadAttentionState::Closed;
            state.focused_context = FocusedContextState::Closed;
            let mut effects = vec![
                AppEffect::SubscribeTimeline {
                    room_id: room_id.clone(),
                },
                AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }),
            ];
            if had_thread {
                effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadChanged));
            }
            effects
        }
        AppAction::TimelineSubscribed { room_id } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
            {
                return Vec::new();
            }

            state.timeline.is_subscribed = true;
            vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
        }
        AppAction::TimelineSubscriptionFailed {
            room_id,
            message: _,
        } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
            {
                return Vec::new();
            }

            state.errors.push(AppError {
                code: "timeline_subscription_failed".to_owned(),
                message: TIMELINE_SUBSCRIPTION_FAILED_MESSAGE.to_owned(),
                recoverable: true,
            });
            vec![
                AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }),
                AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
            ]
        }
        AppAction::TimelineBackPaginationRequested { room_id } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
                || state.timeline.is_paginating_backwards
            {
                return Vec::new();
            }

            state.timeline.is_paginating_backwards = true;
            vec![
                AppEffect::PaginateTimelineBackwards {
                    room_id: room_id.clone(),
                },
                AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }),
            ]
        }
        AppAction::TimelineBackPaginationFinished { room_id } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
                || !state.timeline.is_paginating_backwards
            {
                return Vec::new();
            }

            state.timeline.is_paginating_backwards = false;
            vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
        }
        AppAction::ScheduledSendCapabilityChanged { capability } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            if state.scheduled_sends.capability == capability {
                return Vec::new();
            }
            state.scheduled_sends.capability = capability;
            state.timeline.scheduled_send_capability = state.scheduled_sends.capability.clone();
            state
                .timeline
                .room_id
                .clone()
                .map(|room_id| vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })])
                .unwrap_or_default()
        }
        AppAction::ScheduledSendCreated { item } => {
            if !is_session_ready(state) || !room_exists(state, &item.room_id) {
                return Vec::new();
            }

            let room_id = item.room_id.clone();
            state.scheduled_sends.insert(item);
            state.composer_drafts.clear_room_draft(&room_id);
            if state.timeline.room_id.as_deref() == Some(room_id.as_str()) {
                state.timeline.composer = Default::default();
                refresh_timeline_scheduled_sends(state);
                return vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })];
            }
            Vec::new()
        }
        AppAction::ScheduledSendRescheduled {
            scheduled_id,
            send_at_ms,
            handle,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let Some(item) = state
                .scheduled_sends
                .reschedule(&scheduled_id, send_at_ms, handle)
            else {
                return Vec::new();
            };
            let room_id = item.room_id;
            if state.timeline.room_id.as_deref() == Some(room_id.as_str()) {
                refresh_timeline_scheduled_sends(state);
                return vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })];
            }
            Vec::new()
        }
        AppAction::ScheduledSendCancelled { scheduled_id }
        | AppAction::ScheduledSendDispatched { scheduled_id } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let Some(item) = state.scheduled_sends.remove(&scheduled_id) else {
                return Vec::new();
            };
            let room_id = item.room_id;
            if state.timeline.room_id.as_deref() == Some(room_id.as_str()) {
                refresh_timeline_scheduled_sends(state);
                return vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })];
            }
            Vec::new()
        }
        AppAction::UploadStagingChanged { room_id, items } => {
            if !is_session_ready(state) || !room_exists(state, &room_id) {
                return Vec::new();
            }

            state.upload_staging.replace_room_items(&room_id, items);
            if state.timeline.room_id.as_deref() == Some(room_id.as_str()) {
                refresh_timeline_upload_staging(state);
                return vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })];
            }
            Vec::new()
        }
        AppAction::UploadStagingCaptionChanged { staged_id, caption } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let Some(item) = state.upload_staging.update_caption(&staged_id, caption) else {
                return Vec::new();
            };
            let room_id = item.room_id;
            if state.timeline.room_id.as_deref() == Some(room_id.as_str()) {
                refresh_timeline_upload_staging(state);
                return vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })];
            }
            Vec::new()
        }
        AppAction::UploadStagingCompressionChanged {
            staged_id,
            compression_choice,
        } => {
            if !is_session_ready(state)
                || !staged_compression_choice_is_valid_for_item(
                    state.upload_staging.items.get(&staged_id),
                    compression_choice,
                )
            {
                return Vec::new();
            }

            let Some(item) = state
                .upload_staging
                .update_compression_choice(&staged_id, compression_choice)
            else {
                return Vec::new();
            };
            let room_id = item.room_id;
            if state.timeline.room_id.as_deref() == Some(room_id.as_str()) {
                refresh_timeline_upload_staging(state);
                return vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })];
            }
            Vec::new()
        }
        AppAction::UploadStagingCleared { room_id } => {
            if !is_session_ready(state) || !state.upload_staging.clear_room(&room_id) {
                return Vec::new();
            }
            if state.timeline.room_id.as_deref() == Some(room_id.as_str()) {
                refresh_timeline_upload_staging(state);
                return vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })];
            }
            Vec::new()
        }
        AppAction::MediaGalleryUpdated { room_id, items } => {
            if !is_session_ready(state) || !room_exists(state, &room_id) {
                return Vec::new();
            }

            state.media_gallery.replace_room_items(&room_id, items);
            if state.timeline.room_id.as_deref() == Some(room_id.as_str()) {
                refresh_timeline_media_gallery(state);
                return vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })];
            }
            Vec::new()
        }
        AppAction::ComposerDraftsLoaded { drafts } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            state.composer_drafts = drafts;
            let mut effects = Vec::new();
            if let Some(room_id) = state.timeline.room_id.clone()
                && state.timeline.composer.pending_transaction_id.is_none()
                && state.timeline.composer.draft.is_empty()
            {
                let composer = state.composer_drafts.composer_for_room(&room_id);
                if state.timeline.composer != composer {
                    state.timeline.composer = composer;
                    effects.push(AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }));
                }
            }
            if let ThreadPaneState::Open {
                room_id,
                root_event_id,
                composer,
                ..
            } = &mut state.thread
                && composer.pending_transaction_id.is_none()
                && composer.draft.is_empty()
            {
                let hydrated = state
                    .composer_drafts
                    .composer_for_thread(room_id, root_event_id);
                if *composer != hydrated {
                    *composer = hydrated;
                    effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadChanged));
                }
            }
            effects
        }
        AppAction::ComposerDraftChanged { room_id, draft } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
            {
                return Vec::new();
            }

            state.timeline.composer.draft = draft.clone();
            state.composer_drafts.set_room_draft(room_id.clone(), draft);
            vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
        }
        AppAction::SendTextSubmitted {
            room_id,
            transaction_id,
            body,
        } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
                || state.timeline.composer.pending_transaction_id.is_some()
            {
                return Vec::new();
            }

            state.timeline.composer.pending_transaction_id = Some(transaction_id.clone());
            state.timeline.composer.pending_send_kind = Some(match &state.timeline.composer.mode {
                ComposerMode::Plain => PendingComposerSendKind::Plain,
                ComposerMode::Reply {
                    in_reply_to_event_id,
                } => PendingComposerSendKind::Reply {
                    in_reply_to_event_id: in_reply_to_event_id.clone(),
                },
            });
            state.timeline.composer.draft.clear();
            state.composer_drafts.clear_room_draft(&room_id);
            vec![
                AppEffect::SendText {
                    room_id: room_id.clone(),
                    transaction_id,
                    body,
                },
                AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }),
            ]
        }
        AppAction::SendTextFinished {
            room_id,
            transaction_id,
        } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
                || state.timeline.composer.pending_transaction_id.as_deref()
                    != Some(transaction_id.as_str())
            {
                return Vec::new();
            }

            let pending_send_kind = state.timeline.composer.pending_send_kind.take();
            state.timeline.composer.pending_transaction_id = None;
            if let Some(PendingComposerSendKind::Reply {
                in_reply_to_event_id,
            }) = pending_send_kind
            {
                if state.timeline.composer.mode
                    == (ComposerMode::Reply {
                        in_reply_to_event_id,
                    })
                {
                    state.timeline.composer.mode = ComposerMode::Plain;
                }
            }
            vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
        }
        AppAction::SendTextFailed {
            room_id,
            transaction_id,
            message,
        } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
                || state.timeline.composer.pending_transaction_id.as_deref()
                    != Some(transaction_id.as_str())
            {
                return Vec::new();
            }

            state.timeline.composer.pending_transaction_id = None;
            state.timeline.composer.pending_send_kind = None;
            state.errors.push(AppError {
                code: "send_text_failed".to_owned(),
                message,
                recoverable: true,
            });
            vec![
                AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }),
                AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
            ]
        }
        AppAction::ThreadComposerDraftChanged {
            room_id,
            root_event_id,
            draft,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            match &mut state.thread {
                ThreadPaneState::Open {
                    room_id: open_room_id,
                    root_event_id: open_root_event_id,
                    composer,
                    ..
                } if open_room_id == &room_id && open_root_event_id == &root_event_id => {
                    composer.draft = draft.clone();
                    state
                        .composer_drafts
                        .set_thread_draft(room_id, root_event_id, draft);
                    vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
                }
                _ => Vec::new(),
            }
        }
        AppAction::ThreadReplySubmitted {
            room_id,
            root_event_id,
            transaction_id,
            body: _body,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            match &mut state.thread {
                ThreadPaneState::Open {
                    room_id: open_room_id,
                    root_event_id: open_root_event_id,
                    composer,
                    ..
                } if open_room_id == &room_id
                    && open_root_event_id == &root_event_id
                    && composer.pending_transaction_id.is_none() =>
                {
                    composer.pending_transaction_id = Some(transaction_id);
                    composer.pending_send_kind = Some(PendingComposerSendKind::Reply {
                        in_reply_to_event_id: root_event_id.clone(),
                    });
                    composer.draft.clear();
                    state
                        .composer_drafts
                        .clear_thread_draft(&room_id, &root_event_id);
                    vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
                }
                _ => Vec::new(),
            }
        }
        AppAction::ThreadReplyFinished {
            room_id,
            root_event_id,
            transaction_id,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            match &mut state.thread {
                ThreadPaneState::Open {
                    room_id: open_room_id,
                    root_event_id: open_root_event_id,
                    composer,
                    ..
                } if open_room_id == &room_id
                    && open_root_event_id == &root_event_id
                    && composer.pending_transaction_id.as_deref()
                        == Some(transaction_id.as_str()) =>
                {
                    composer.pending_transaction_id = None;
                    composer.pending_send_kind = None;
                    vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
                }
                _ => Vec::new(),
            }
        }
        AppAction::ThreadReplyFailed {
            room_id,
            root_event_id,
            transaction_id,
            message,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            match &mut state.thread {
                ThreadPaneState::Open {
                    room_id: open_room_id,
                    root_event_id: open_root_event_id,
                    composer,
                    ..
                } if open_room_id == &room_id
                    && open_root_event_id == &root_event_id
                    && composer.pending_transaction_id.as_deref()
                        == Some(transaction_id.as_str()) =>
                {
                    composer.pending_transaction_id = None;
                    composer.pending_send_kind = None;
                    state.errors.push(AppError {
                        code: "send_text_failed".to_owned(),
                        message,
                        recoverable: true,
                    });
                    vec![
                        AppEffect::EmitUiEvent(UiEvent::ThreadChanged),
                        AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
                    ]
                }
                _ => Vec::new(),
            }
        }
        AppAction::OpenThread {
            room_id,
            root_event_id,
        } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
            {
                return Vec::new();
            }

            state.thread = ThreadPaneState::Opening {
                room_id: room_id.clone(),
                root_event_id: root_event_id.clone(),
            };
            state.thread_attention = ThreadAttentionState::Tracking {
                room_id: room_id.clone(),
                root_event_id: root_event_id.clone(),
                notification_count: 0,
                highlight_count: 0,
                live_event_marker_count: 0,
            };
            vec![
                AppEffect::OpenThreadTimeline {
                    room_id,
                    root_event_id,
                },
                AppEffect::EmitUiEvent(UiEvent::ThreadChanged),
            ]
        }
        AppAction::ThreadSubscribed {
            room_id,
            root_event_id,
        } => {
            if !is_session_ready(state)
                || !matches!(
                    &state.thread,
                    ThreadPaneState::Opening {
                        room_id: opening_room_id,
                        root_event_id: opening_root_event_id,
                    } if opening_room_id == &room_id && opening_root_event_id == &root_event_id
                )
            {
                return Vec::new();
            }

            state.thread = ThreadPaneState::Open {
                composer: state
                    .composer_drafts
                    .composer_for_thread(&room_id, &root_event_id),
                room_id,
                root_event_id,
                is_subscribed: true,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
        }
        AppAction::ThreadAttentionUpdated {
            room_id,
            root_event_id,
            notification_count,
            highlight_count,
            live_event_marker_count,
        } => {
            if !is_session_ready(state)
                || !matches!(
                    &state.thread_attention,
                    ThreadAttentionState::Tracking {
                        room_id: tracking_room_id,
                        root_event_id: tracking_root_event_id,
                        ..
                    } if tracking_room_id == &room_id
                        && tracking_root_event_id == &root_event_id
                )
            {
                return Vec::new();
            }

            let next = ThreadAttentionState::Tracking {
                room_id,
                root_event_id,
                notification_count,
                highlight_count,
                live_event_marker_count,
            };
            if state.thread_attention == next {
                return Vec::new();
            }

            state.thread_attention = next;
            vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
        }
        AppAction::CloseThread => {
            if !is_session_ready(state) || state.thread == ThreadPaneState::Closed {
                return Vec::new();
            }

            state.thread = ThreadPaneState::Closed;
            state.thread_attention = ThreadAttentionState::Closed;
            vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
        }
        AppAction::OpenFocusedContext { room_id, event_id } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
            {
                return Vec::new();
            }

            state.focused_context = FocusedContextState::Opening {
                room_id: room_id.clone(),
                event_id: event_id.clone(),
            };
            vec![AppEffect::OpenFocusedTimeline { room_id, event_id }]
        }
        AppAction::FocusedContextSubscribed { room_id, event_id } => {
            if !is_session_ready(state)
                || !matches!(
                    &state.focused_context,
                    FocusedContextState::Opening {
                        room_id: opening_room_id,
                        event_id: opening_event_id,
                    } if opening_room_id == &room_id && opening_event_id == &event_id
                )
            {
                return Vec::new();
            }

            state.focused_context = FocusedContextState::Open {
                room_id,
                event_id,
                is_subscribed: true,
            };
            Vec::new()
        }
        AppAction::CloseFocusedContext => {
            if !is_session_ready(state) || state.focused_context == FocusedContextState::Closed {
                return Vec::new();
            }

            state.focused_context = FocusedContextState::Closed;
            Vec::new()
        }
        AppAction::SearchEdited { query, scope } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            state.search = SearchState::Editing { query, scope };
            vec![AppEffect::EmitUiEvent(UiEvent::SearchChanged)]
        }
        AppAction::SearchSubmitted {
            request_id,
            query,
            scope,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            state.search = SearchState::Searching {
                request_id,
                query: query.clone(),
                scope: scope.clone(),
            };
            vec![
                AppEffect::SearchMessages {
                    request_id,
                    query,
                    scope,
                },
                AppEffect::EmitUiEvent(UiEvent::SearchChanged),
            ]
        }
        AppAction::SearchSucceeded {
            request_id,
            results,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let (current_request_id, query, scope) = match &state.search {
                SearchState::Searching {
                    request_id,
                    query,
                    scope,
                } => (*request_id, query.clone(), scope.clone()),
                _ => return Vec::new(),
            };

            if current_request_id != request_id {
                return Vec::new();
            }

            state.search = SearchState::Results {
                request_id,
                query,
                scope,
                results,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::SearchChanged)]
        }
        AppAction::SearchFailed {
            request_id,
            message,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let (current_request_id, query, scope) = match &state.search {
                SearchState::Searching {
                    request_id,
                    query,
                    scope,
                } => (*request_id, query.clone(), scope.clone()),
                _ => return Vec::new(),
            };

            if current_request_id != request_id {
                return Vec::new();
            }

            state.search = SearchState::Failed {
                request_id,
                query,
                scope,
                message,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::SearchChanged)]
        }
        AppAction::FilesViewOpened {
            request_id,
            scope,
            filter,
            sort,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let scope = resolve_files_view_scope(state, scope);
            state.files_view = FilesViewState::Loading {
                request_id,
                scope: scope.clone(),
                filter: filter.clone(),
                sort,
            };
            vec![
                AppEffect::SearchAttachments {
                    request_id,
                    scope,
                    filter,
                    sort,
                },
                AppEffect::EmitUiEvent(UiEvent::FilesViewChanged),
            ]
        }
        AppAction::FilesViewClosed => {
            if state.files_view == FilesViewState::Closed {
                return Vec::new();
            }

            state.files_view = FilesViewState::Closed;
            vec![AppEffect::EmitUiEvent(UiEvent::FilesViewChanged)]
        }
        AppAction::FilesViewQueryRequested {
            request_id,
            scope,
            filter,
            sort,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            state.files_view = FilesViewState::Loading {
                request_id,
                scope: scope.clone(),
                filter: filter.clone(),
                sort,
            };
            vec![
                AppEffect::SearchAttachments {
                    request_id,
                    scope,
                    filter,
                    sort,
                },
                AppEffect::EmitUiEvent(UiEvent::FilesViewChanged),
            ]
        }
        AppAction::FilesViewQuerySucceeded { request_id, items } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let (current_request_id, scope, filter, sort) = match &state.files_view {
                FilesViewState::Loading {
                    request_id,
                    scope,
                    filter,
                    sort,
                } => (*request_id, scope.clone(), filter.clone(), *sort),
                _ => return Vec::new(),
            };

            if current_request_id != request_id {
                return Vec::new();
            }

            state.files_view = FilesViewState::Open {
                request_id,
                scope,
                filter,
                sort,
                items,
                selected_event_id: None,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::FilesViewChanged)]
        }
        AppAction::FilesViewQueryFailed {
            request_id,
            message,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let (current_request_id, scope, filter, sort) = match &state.files_view {
                FilesViewState::Loading {
                    request_id,
                    scope,
                    filter,
                    sort,
                } => (*request_id, scope.clone(), filter.clone(), *sort),
                _ => return Vec::new(),
            };

            if current_request_id != request_id {
                return Vec::new();
            }

            state.files_view = FilesViewState::Failed {
                request_id,
                scope,
                filter,
                sort,
                message,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::FilesViewChanged)]
        }
        AppAction::FilesViewSelectionChanged { event_id } => {
            if let FilesViewState::Open {
                selected_event_id, ..
            } = &mut state.files_view
            {
                if *selected_event_id == event_id {
                    return Vec::new();
                }
                *selected_event_id = event_id;
                vec![AppEffect::EmitUiEvent(UiEvent::FilesViewChanged)]
            } else {
                Vec::new()
            }
        }
        AppAction::ClearError { code } => {
            state.errors.retain(|error| error.code != code);
            vec![AppEffect::EmitUiEvent(UiEvent::ErrorChanged)]
        }
        AppAction::BasicOperationRequested {
            request_id,
            request,
        } => {
            // Start transition. Guarded like the composer's pending-transaction
            // rule: a new operation is accepted only from `Idle` and only with a
            // ready session, so an in-flight operation is never clobbered.
            if !is_session_ready(state) || !state.basic_operation.is_idle() {
                return Vec::new();
            }
            state.basic_operation = match request {
                BasicOperationRequest::CreateRoom { name } => {
                    BasicOperationState::CreatingRoom { request_id, name }
                }
                BasicOperationRequest::CreateSpace { name } => {
                    BasicOperationState::CreatingSpace { request_id, name }
                }
                BasicOperationRequest::LinkSpaceChild {
                    space_id,
                    child_room_id,
                } => BasicOperationState::LinkingSpaceChild {
                    request_id,
                    space_id,
                    child_room_id,
                },
            };
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        }
        AppAction::BasicOperationSucceeded { request_id } => {
            // Settle transition. Like search's `request_id` check, a completion is
            // applied only when it correlates to the in-flight operation; stale,
            // duplicate, or idle-state completions are ignored.
            if state.basic_operation.request_id() != Some(request_id) {
                return Vec::new();
            }
            state.basic_operation = BasicOperationState::Idle;
            vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
        }
        AppAction::BasicOperationFailed {
            request_id,
            message,
        } => {
            if state.basic_operation.request_id() != Some(request_id) {
                return Vec::new();
            }
            state.basic_operation = BasicOperationState::Idle;
            state.errors.push(AppError {
                code: "basic_operation_failed".to_owned(),
                message,
                recoverable: true,
            });
            vec![
                AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
                AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
            ]
        }
        AppAction::LiveRoomSignalsUpdated { room_id, update } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let own_user_id = session_user_id(state).map(str::to_owned);
            state.live_signals.rooms.insert(
                room_id,
                update.into_room_signals_with_profiles(&state.profile, own_user_id.as_deref()),
            );
            vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
        }
        AppAction::LiveRoomReceiptsUpdated {
            room_id,
            receipts_by_event,
        } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let own_user_id = session_user_id(state).map(str::to_owned);
            let room = state.live_signals.rooms.entry(room_id).or_default();
            let normalized = crate::state::LiveRoomSignalUpdate {
                receipts_by_event,
                fully_read_event_id: None,
                typing_user_ids: Vec::new(),
            }
            .into_room_signals_with_profiles(&state.profile, own_user_id.as_deref());
            for (event_id, receipts) in normalized.receipts_by_event {
                room.receipts_by_event.insert(event_id, receipts);
            }
            vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
        }
        AppAction::FullyReadMarkerUpdated { room_id, event_id } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            state
                .live_signals
                .rooms
                .entry(room_id)
                .or_default()
                .fully_read_event_id = event_id;
            vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
        }
        AppAction::TypingUsersUpdated { room_id, user_ids } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            let normalized = crate::state::LiveRoomSignalUpdate {
                receipts_by_event: Vec::new(),
                fully_read_event_id: None,
                typing_user_ids: user_ids,
            }
            .into_room_signals();
            state
                .live_signals
                .rooms
                .entry(room_id)
                .or_default()
                .typing_user_ids = normalized.typing_user_ids;
            vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
        }
        AppAction::PresenceUpdated { user_id, presence } => {
            if !is_session_ready(state) {
                return Vec::new();
            }

            state.live_signals.presence.insert(user_id, presence);
            vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
        }
        AppAction::ComposerReplyTargetSelected { room_id, event_id } => {
            if !is_session_ready(state)
                || state.timeline.room_id.as_deref() != Some(room_id.as_str())
            {
                return Vec::new();
            }
            state.timeline.composer.mode = ComposerMode::Reply {
                in_reply_to_event_id: event_id,
            };
            vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
        }
        AppAction::ComposerReplyCancelled => {
            let Some(room_id) = state.timeline.room_id.clone() else {
                return Vec::new();
            };
            if state.timeline.composer.mode == ComposerMode::Plain {
                return Vec::new();
            }
            state.timeline.composer.mode = ComposerMode::Plain;
            vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
        }
    }
}

fn is_session_ready(state: &AppState) -> bool {
    matches!(
        state.session,
        SessionState::Ready(_)
            | SessionState::NeedsRecovery { .. }
            | SessionState::Recovering { .. }
    )
}

fn resolve_files_view_scope(state: &AppState, scope: FilesViewScope) -> AttachmentScope {
    match scope {
        FilesViewScope::Room { room_id } => AttachmentScope::Room { room_id },
        FilesViewScope::Space { space_id } => {
            let child_room_ids = state
                .spaces
                .iter()
                .find(|space| space.space_id == space_id)
                .map(|space| space.child_room_ids.clone())
                .unwrap_or_default();
            AttachmentScope::Space {
                space_id,
                child_room_ids,
            }
        }
        FilesViewScope::Account => AttachmentScope::Account,
    }
}

fn session_user_id(state: &AppState) -> Option<&str> {
    match &state.session {
        SessionState::Ready(info)
        | SessionState::NeedsRecovery { info, .. }
        | SessionState::Recovering { info, .. } => Some(info.user_id.as_str()),
        _ => None,
    }
}

fn refresh_open_room_settings_member_display_projection(
    state: &mut AppState,
    own_user_id: Option<&str>,
) -> bool {
    let Some(settings) = state.room_management.settings.as_mut() else {
        return false;
    };
    crate::state::refresh_room_settings_member_display_projection(
        settings,
        &state.profile,
        own_user_id,
    )
}

fn refresh_open_room_summary_display_projection(
    state: &mut AppState,
    own_user_id: Option<&str>,
) -> bool {
    crate::state::refresh_room_summary_display_projection(
        &mut state.rooms,
        &state.profile,
        own_user_id,
    )
}

fn refresh_native_attention_candidate_display_projection(state: &mut AppState) -> bool {
    let Some(candidate) = state.native_attention.summary.candidate.as_mut() else {
        return false;
    };
    let Some(display_label) = state
        .rooms
        .iter()
        .filter(|room| room.tags.low_priority.is_none())
        .filter_map(|room| {
            crate::state::room_attention_summary(
                room.display_label.clone(),
                room.is_dm,
                room.notification_count,
                room.highlight_count,
                room.unread_count,
            )
        })
        .filter(|summary| {
            summary.kind == candidate.kind
                && summary.unread_count == candidate.unread_count
                && summary.highlight_count == candidate.highlight_count
        })
        .map(|summary| summary.room_display_name)
        .min()
    else {
        return false;
    };
    if candidate.room_display_name == display_label {
        return false;
    }
    candidate.room_display_name = display_label;
    true
}

fn profile_changed_effects(
    room_management_changed: bool,
    room_list_changed: bool,
    native_attention_changed: bool,
) -> Vec<AppEffect> {
    let mut effects = vec![AppEffect::EmitUiEvent(UiEvent::ProfileChanged)];
    if room_list_changed {
        effects.push(AppEffect::EmitUiEvent(UiEvent::RoomListChanged));
    }
    if room_management_changed {
        effects.push(AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged));
    }
    if native_attention_changed {
        effects.push(AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged));
    }
    effects
}

fn room_exists(state: &AppState, room_id: &str) -> bool {
    state.rooms.iter().any(|room| room.room_id == room_id)
}

fn room_settings_permission_allows(state: &AppState, room_id: &str) -> bool {
    state
        .room_management
        .settings
        .as_ref()
        .filter(|settings| settings.room_id == room_id)
        .is_some_and(|settings| settings.permissions.can_edit_settings)
}

fn room_role_permission_allows(state: &AppState, room_id: &str) -> bool {
    state
        .room_management
        .settings
        .as_ref()
        .filter(|settings| settings.room_id == room_id)
        .is_some_and(|settings| settings.permissions.can_edit_roles)
}

fn room_moderation_permission_allows(
    state: &AppState,
    room_id: &str,
    action: RoomModerationAction,
) -> bool {
    let Some(permissions) = state
        .room_management
        .settings
        .as_ref()
        .filter(|settings| settings.room_id == room_id)
        .map(|settings| settings.permissions)
    else {
        return false;
    };

    match action {
        RoomModerationAction::Kick => permissions.can_kick,
        RoomModerationAction::Ban => permissions.can_ban,
        RoomModerationAction::Unban => permissions.can_unban,
    }
}

fn room_management_operation_matches(
    state: &AppState,
    request_id: u64,
    room_id: &str,
    operation: RoomManagementOperationKind,
) -> bool {
    matches!(
        &state.room_management.operation,
        RoomManagementOperationState::Pending {
            request_id: current_request_id,
            room_id: current_room_id,
            operation: current_operation,
        } if *current_request_id == request_id
            && current_room_id == room_id
            && *current_operation == operation
    )
}

fn normalize_activity_stream(
    mut stream: ActivityStream,
    excluded_room_ids: &BTreeSet<String>,
) -> ActivityStream {
    stream
        .rows
        .retain(|row| !excluded_room_ids.contains(&row.room_id));
    stream.rows.sort_by(|left, right| {
        right
            .timestamp_ms
            .cmp(&left.timestamp_ms)
            .then_with(|| left.room_id.cmp(&right.room_id))
            .then_with(|| left.event_id.cmp(&right.event_id))
    });
    stream
}

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

fn account_management_matches(
    state: &AccountManagementState,
    request_id: u64,
    operation: crate::state::AccountManagementOperation,
) -> bool {
    matches!(
        state,
        AccountManagementState::Working {
            request_id: active,
            operation: active_operation,
        }
        | AccountManagementState::AwaitingUia {
            request_id: active,
            operation: active_operation,
            ..
        } if *active == request_id && *active_operation == operation
    )
}

fn e2ee_key_management_events() -> Vec<AppEffect> {
    vec![
        AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged),
        AppEffect::EmitUiEvent(UiEvent::E2eeKeyManagementChanged),
    ]
}

fn first_default_room_id(state: &AppState) -> Option<String> {
    state
        .rooms
        .iter()
        .find(|room| !room.is_dm)
        .or_else(|| state.rooms.first())
        .map(|room| room.room_id.clone())
}

fn first_room_id_in_active_space(state: &AppState) -> Option<String> {
    let active_space_id = state.navigation.active_space_id.as_deref()?;
    let active_space = state
        .spaces
        .iter()
        .find(|space| space.space_id == active_space_id)?;

    active_space
        .child_room_ids
        .iter()
        .find_map(|child_room_id| {
            state
                .rooms
                .iter()
                .find(|room| room.room_id == *child_room_id && !room.is_dm)
                .map(|room| room.room_id.clone())
        })
}

fn active_room_left_selected_space(state: &AppState, active_room_id: &str) -> bool {
    let Some(active_space_id) = state.navigation.active_space_id.as_deref() else {
        return false;
    };
    let Some(active_room) = state
        .rooms
        .iter()
        .find(|room| room.room_id == active_room_id)
    else {
        return false;
    };
    if active_room.is_dm {
        return false;
    }

    state
        .spaces
        .iter()
        .find(|space| space.space_id == active_space_id)
        .is_some_and(|space| {
            !space
                .child_room_ids
                .iter()
                .any(|room_id| room_id == active_room_id)
        })
}

fn retarget_active_room_for_selected_space(
    state: &mut AppState,
    effects: &mut Vec<AppEffect>,
    previous_room_id: String,
) {
    let next_room_id = first_room_id_in_active_space(state);
    let had_thread = state.thread != ThreadPaneState::Closed
        || state.thread_attention != ThreadAttentionState::Closed;

    match next_room_id {
        Some(room_id) => {
            select_active_room_after_room_list_update(state, effects, room_id);
        }
        None => {
            state.navigation.active_room_id = None;
            state.thread = ThreadPaneState::Closed;
            state.thread_attention = ThreadAttentionState::Closed;
            state.timeline = Default::default();
            effects.push(AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: previous_room_id,
            }));

            if had_thread {
                effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadChanged));
            }
        }
    }
}

fn select_active_room_after_room_list_update(
    state: &mut AppState,
    effects: &mut Vec<AppEffect>,
    room_id: String,
) {
    let had_thread = state.thread != ThreadPaneState::Closed
        || state.thread_attention != ThreadAttentionState::Closed;

    state.navigation.active_room_id = Some(room_id.clone());
    state.timeline = TimelinePaneState {
        room_id: Some(room_id.clone()),
        is_subscribed: false,
        is_paginating_backwards: false,
        composer: state.composer_drafts.composer_for_room(&room_id),
        scheduled_send_capability: state.scheduled_sends.capability.clone(),
        scheduled_sends: state.scheduled_sends.items_for_room(&room_id),
        staged_uploads: state.upload_staging.items_for_room(&room_id),
        media_gallery: state.media_gallery.items_for_room(&room_id),
    };
    state.thread = ThreadPaneState::Closed;
    state.thread_attention = ThreadAttentionState::Closed;
    effects.push(AppEffect::SubscribeTimeline {
        room_id: room_id.clone(),
    });
    effects.push(AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }));

    if had_thread {
        effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadChanged));
    }
}

fn refresh_timeline_scheduled_sends(state: &mut AppState) {
    state.timeline.scheduled_send_capability = state.scheduled_sends.capability.clone();
    state.timeline.scheduled_sends = state
        .timeline
        .room_id
        .as_deref()
        .map(|room_id| state.scheduled_sends.items_for_room(room_id))
        .unwrap_or_default();
}

fn refresh_timeline_upload_staging(state: &mut AppState) {
    state.timeline.staged_uploads = state
        .timeline
        .room_id
        .as_deref()
        .map(|room_id| state.upload_staging.items_for_room(room_id))
        .unwrap_or_default();
}

fn refresh_timeline_media_gallery(state: &mut AppState) {
    state.timeline.media_gallery = state
        .timeline
        .room_id
        .as_deref()
        .map(|room_id| state.media_gallery.items_for_room(room_id))
        .unwrap_or_default();
}

fn staged_compression_choice_is_valid_for_item(
    item: Option<&crate::state::StagedUploadItem>,
    compression_choice: StagedUploadCompressionChoice,
) -> bool {
    match (item, compression_choice) {
        (Some(item), StagedUploadCompressionChoice::NotApplicable) => {
            matches!(item.kind, crate::state::StagedUploadKind::File)
        }
        (Some(item), StagedUploadCompressionChoice::Original)
        | (Some(item), StagedUploadCompressionChoice::Compressed { .. }) => {
            matches!(item.kind, crate::state::StagedUploadKind::Image { .. })
        }
        (None, _) => false,
    }
}

fn current_session_info(state: &AppState) -> Option<crate::state::SessionInfo> {
    match &state.session {
        SessionState::NeedsRecovery { info, .. }
        | SessionState::Recovering { info, .. }
        | SessionState::Ready(info)
        | SessionState::Locked(info) => Some(info.clone()),
        SessionState::SignedOut
        | SessionState::Restoring
        | SessionState::SwitchingAccount { .. }
        | SessionState::Authenticating { .. }
        | SessionState::LoggingOut => None,
    }
}

fn clear_session_views(state: &mut AppState) -> Vec<AppEffect> {
    let previous_room_id = state.timeline.room_id.clone();
    let had_thread = state.thread != ThreadPaneState::Closed
        || state.thread_attention != ThreadAttentionState::Closed;
    let had_search = state.search != SearchState::Closed;
    let had_e2ee_trust = state.e2ee_trust != E2eeTrustState::default();
    let had_e2ee_key_management =
        state.e2ee_trust.key_management != E2eeKeyManagementState::default();
    let had_device_sessions = state.device_sessions != DeviceSessionListState::Idle;
    let had_account_management = state.account_management != AccountManagementState::Idle;
    let had_qr_login = state.qr_login != QrLoginState::Idle;
    let had_live_signals = state.live_signals != Default::default();
    let had_profile = state.profile != Default::default();
    let had_room_interactions = !state.room_interactions.is_empty();
    let had_directory = state.directory != DirectoryState::default();
    let had_activity = state.activity != ActivityState::Closed;
    let had_room_management = state.room_management != Default::default();
    let had_local_encryption = state.local_encryption != LocalEncryptionState::Unknown;
    let had_native_attention = state.native_attention != Default::default();
    let had_files_view = state.files_view != FilesViewState::Closed;

    state.navigation = NavigationState::default();
    state.spaces.clear();
    state.rooms.clear();
    state.invites.clear();
    state.room_interactions.clear();
    state.composer_drafts = Default::default();
    state.scheduled_sends = Default::default();
    state.upload_staging = Default::default();
    state.media_gallery = Default::default();
    state.directory = DirectoryState::default();
    state.activity = ActivityState::Closed;
    state.room_management = Default::default();
    state.profile = Default::default();
    state.timeline = Default::default();
    state.thread = ThreadPaneState::Closed;
    state.thread_attention = ThreadAttentionState::Closed;
    state.focused_context = FocusedContextState::Closed;
    state.search = SearchState::Closed;
    state.files_view = FilesViewState::Closed;
    state.e2ee_trust = E2eeTrustState::default();
    state.device_sessions = DeviceSessionListState::Idle;
    state.account_management = AccountManagementState::Idle;
    state.qr_login = QrLoginState::Idle;
    state.live_signals = Default::default();
    state.local_encryption = LocalEncryptionState::Unknown;
    state.native_attention = Default::default();

    let mut effects = vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)];
    if let Some(room_id) = previous_room_id {
        effects.push(AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }));
    }
    if had_thread {
        effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadChanged));
    }
    if had_search {
        effects.push(AppEffect::EmitUiEvent(UiEvent::SearchChanged));
    }
    if had_e2ee_trust {
        effects.push(AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged));
    }
    if had_e2ee_key_management {
        effects.push(AppEffect::EmitUiEvent(UiEvent::E2eeKeyManagementChanged));
    }
    if had_device_sessions {
        effects.push(AppEffect::EmitUiEvent(UiEvent::DeviceSessionsChanged));
    }
    if had_account_management {
        effects.push(AppEffect::EmitUiEvent(UiEvent::AccountManagementChanged));
    }
    if had_qr_login {
        effects.push(AppEffect::EmitUiEvent(UiEvent::QrLoginChanged));
    }
    if had_live_signals {
        effects.push(AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged));
    }
    if had_profile {
        effects.push(AppEffect::EmitUiEvent(UiEvent::ProfileChanged));
    }
    if had_room_interactions {
        effects.push(AppEffect::EmitUiEvent(UiEvent::RoomInteractionsChanged));
    }
    if had_directory {
        effects.push(AppEffect::EmitUiEvent(UiEvent::DirectoryChanged));
    }
    if had_activity {
        effects.push(AppEffect::EmitUiEvent(UiEvent::ActivityChanged));
    }
    if had_room_management {
        effects.push(AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged));
    }
    if had_local_encryption {
        effects.push(AppEffect::EmitUiEvent(UiEvent::LocalEncryptionChanged));
    }
    if had_native_attention {
        effects.push(AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged));
    }
    if had_files_view {
        effects.push(AppEffect::EmitUiEvent(UiEvent::FilesViewChanged));
    }
    effects
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{
        AvatarImage, AvatarThumbnailState, LiveEventReceiptSummary, LiveEventReceipts,
        LiveReadReceipt, LiveRoomSignalUpdate, PresenceKind, RoomLiveSignals, UserProfile,
    };

    fn ready_state() -> AppState {
        let mut state = AppState::default();
        state.session = SessionState::Ready(crate::state::SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@alice:example.invalid".to_owned(),
            device_id: "DEVICE".to_owned(),
        });
        state
    }

    #[test]
    fn live_signal_actions_update_rust_owned_state() {
        let mut state = ready_state();

        let effects = reduce(
            &mut state,
            AppAction::LiveRoomSignalsUpdated {
                room_id: "!room:example.invalid".to_owned(),
                update: LiveRoomSignalUpdate {
                    receipts_by_event: vec![LiveEventReceipts {
                        event_id: "$event:example.invalid".to_owned(),
                        receipts: vec![LiveReadReceipt {
                            user_id: "@bob:example.invalid".to_owned(),
                            display_name: None,
                            original_display_label: String::new(),
                            avatar: None,
                            timestamp_ms: Some(1_234),
                        }],
                    }],
                    fully_read_event_id: Some("$event:example.invalid".to_owned()),
                    typing_user_ids: vec![
                        "@carol:example.invalid".to_owned(),
                        "@bob:example.invalid".to_owned(),
                        "@bob:example.invalid".to_owned(),
                    ],
                },
            },
        );

        assert_eq!(
            effects,
            vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
        );
        assert_eq!(
            state.live_signals.rooms.get("!room:example.invalid"),
            Some(&RoomLiveSignals {
                receipts_by_event: [(
                    "$event:example.invalid".to_owned(),
                    LiveEventReceiptSummary {
                        readers: vec![LiveReadReceipt {
                            user_id: "@bob:example.invalid".to_owned(),
                            display_name: Some("@bob:example.invalid".to_owned()),
                            original_display_label: "@bob:example.invalid".to_owned(),
                            avatar: None,
                            timestamp_ms: Some(1_234),
                        }],
                        total_count: 1,
                        overflow_count: 0,
                    },
                )]
                .into(),
                fully_read_event_id: Some("$event:example.invalid".to_owned()),
                typing_user_ids: vec![
                    "@bob:example.invalid".to_owned(),
                    "@carol:example.invalid".to_owned(),
                ],
            })
        );

        let effects = reduce(
            &mut state,
            AppAction::PresenceUpdated {
                user_id: "@bob:example.invalid".to_owned(),
                presence: PresenceKind::Away,
            },
        );

        assert_eq!(
            effects,
            vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
        );
        assert_eq!(
            state.live_signals.presence.get("@bob:example.invalid"),
            Some(&PresenceKind::Away)
        );
    }

    #[test]
    fn live_signals_clear_with_session_views() {
        let mut state = ready_state();
        state.live_signals.rooms.insert(
            "!room:example.invalid".to_owned(),
            RoomLiveSignals {
                fully_read_event_id: Some("$event:example.invalid".to_owned()),
                ..RoomLiveSignals::default()
            },
        );
        state
            .live_signals
            .presence
            .insert("@bob:example.invalid".to_owned(), PresenceKind::Online);

        let effects = reduce(&mut state, AppAction::LogoutRequested);

        assert!(state.live_signals.rooms.is_empty());
        assert!(state.live_signals.presence.is_empty());
        assert!(
            effects.iter().any(|effect| matches!(
                effect,
                AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)
            ))
        );
    }

    #[test]
    fn live_read_receipts_project_reader_profiles_order_and_overflow() {
        let mut state = ready_state();
        state.profile.users.insert(
            "@alice:example.invalid".to_owned(),
            UserProfile {
                user_id: "@alice:example.invalid".to_owned(),
                display_name: Some("Alice".to_owned()),
                display_label: String::new(),
                original_display_label: String::new(),
                mention_search_terms: Vec::new(),
                avatar: Some(AvatarImage {
                    mxc_uri: "mxc://example.invalid/alice".to_owned(),
                    thumbnail: AvatarThumbnailState::NotRequested,
                }),
            },
        );
        state.profile.users.insert(
            "@bob:example.invalid".to_owned(),
            UserProfile {
                user_id: "@bob:example.invalid".to_owned(),
                display_name: Some("Bob".to_owned()),
                display_label: String::new(),
                original_display_label: String::new(),
                mention_search_terms: Vec::new(),
                avatar: Some(AvatarImage {
                    mxc_uri: "mxc://example.invalid/bob".to_owned(),
                    thumbnail: AvatarThumbnailState::NotRequested,
                }),
            },
        );
        state.profile.users.insert(
            "@carol:example.invalid".to_owned(),
            UserProfile {
                user_id: "@carol:example.invalid".to_owned(),
                display_name: Some("Carol".to_owned()),
                display_label: String::new(),
                original_display_label: String::new(),
                mention_search_terms: Vec::new(),
                avatar: None,
            },
        );

        let effects = reduce(
            &mut state,
            AppAction::LiveRoomReceiptsUpdated {
                room_id: "!room:example.invalid".to_owned(),
                receipts_by_event: vec![LiveEventReceipts {
                    event_id: "$event:example.invalid".to_owned(),
                    receipts: vec![
                        LiveReadReceipt {
                            user_id: "@alice:example.invalid".to_owned(),
                            display_name: None,
                            original_display_label: String::new(),
                            avatar: None,
                            timestamp_ms: Some(1_000),
                        },
                        LiveReadReceipt {
                            user_id: "@bob:example.invalid".to_owned(),
                            display_name: None,
                            original_display_label: String::new(),
                            avatar: None,
                            timestamp_ms: Some(3_000),
                        },
                        LiveReadReceipt {
                            user_id: "@carol:example.invalid".to_owned(),
                            display_name: None,
                            original_display_label: String::new(),
                            avatar: None,
                            timestamp_ms: Some(2_000),
                        },
                        LiveReadReceipt {
                            user_id: "@dana:example.invalid".to_owned(),
                            display_name: Some("Dana".to_owned()),
                            original_display_label: String::new(),
                            avatar: None,
                            timestamp_ms: Some(4_000),
                        },
                        LiveReadReceipt {
                            user_id: "@alice:example.invalid".to_owned(),
                            display_name: None,
                            original_display_label: String::new(),
                            avatar: None,
                            timestamp_ms: Some(5_000),
                        },
                    ],
                }],
            },
        );

        assert_eq!(
            effects,
            vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
        );
        let summary = state
            .live_signals
            .rooms
            .get("!room:example.invalid")
            .and_then(|room| room.receipts_by_event.get("$event:example.invalid"))
            .expect("receipt projection");
        assert_eq!(summary.total_count, 4);
        assert_eq!(summary.overflow_count, 1);
        assert_eq!(
            summary
                .readers
                .iter()
                .map(|receipt| (
                    receipt.user_id.as_str(),
                    receipt.display_name.as_deref(),
                    receipt.timestamp_ms,
                    receipt
                        .avatar
                        .as_ref()
                        .map(|avatar| avatar.mxc_uri.as_str()),
                ))
                .collect::<Vec<_>>(),
            vec![
                (
                    "@alice:example.invalid",
                    Some("Alice"),
                    Some(5_000),
                    Some("mxc://example.invalid/alice"),
                ),
                ("@dana:example.invalid", Some("Dana"), Some(4_000), None),
                (
                    "@bob:example.invalid",
                    Some("Bob"),
                    Some(3_000),
                    Some("mxc://example.invalid/bob"),
                ),
            ]
        );
    }
}
