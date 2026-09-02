//! `recovery_backup` ownership for AccountActor.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_sdk::MatrixClientSession;
use koushi_state::{
    AppAction, AuthFailureKind, CrossSigningStatus, IdentityResetAuthType, IdentityResetState,
    RecoveryKeyDeliveryState, RecoveryRequest, SecureBackupSetupIntent, TrustOperationFailureKind,
};

use crate::executor;
use crate::native_artifact::NativeArtifactKind;
use koushi_protocol::command::{
    RoomKeyExportRequest, RoomKeyImportRequest, SecureBackupPassphraseChangeRequest,
    SecureBackupSetupRequest,
};
use koushi_protocol::event::{AccountEvent, CoreEvent, E2eeTrustEvent};
use koushi_protocol::failure::{CoreFailure, RecoveryFailureKind};
use koushi_protocol::ids::{AccountKey, RequestId};

use super::actor::{AccountActor, AccountMessage};
use super::local_data_cleanup::record_device_cleanup_offer;
use super::trust_gate::{current_device_trust_token, verification_gate_failure_kind};
use super::verification::recovery_failure_token;

const RECOVERY_TRUST_SETTLEMENT_TIMEOUT: Duration = Duration::from_secs(20);

const RECOVERY_TRUST_SETTLEMENT_POLL_INTERVAL: Duration = Duration::from_millis(250);

const SECURE_BACKUP_INSPECTION_TIMEOUT: Duration = Duration::from_secs(30);

const SECURE_BACKUP_RETRY_BASE: Duration = Duration::from_secs(5);

const SECURE_BACKUP_RETRY_MAX: Duration = Duration::from_secs(5 * 60);

const SECURE_BACKUP_MONITOR_INTERVAL: Duration = Duration::from_secs(60);

pub(super) fn secure_backup_retry_delay(attempt: u32, jitter_seed: u64) -> Duration {
    let exponent = attempt.min(6);
    let base_ms = SECURE_BACKUP_RETRY_BASE
        .as_millis()
        .saturating_mul(1_u128 << exponent)
        .min(SECURE_BACKUP_RETRY_MAX.as_millis());
    let jitter_max_ms = base_ms / 5;
    let mut mixed =
        jitter_seed ^ u64::from(attempt.wrapping_add(1)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    mixed ^= mixed >> 30;
    mixed = mixed.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    mixed ^= mixed >> 27;
    let jitter_ms = if jitter_max_ms == 0 {
        0
    } else {
        u128::from(mixed) % (jitter_max_ms + 1)
    };
    Duration::from_millis(
        (base_ms + jitter_ms)
            .min(SECURE_BACKUP_RETRY_MAX.as_millis())
            .try_into()
            .unwrap_or(u64::MAX),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SecureBackupInspectionAdmission {
    Start,
    Coalesce,
    Defer,
}

pub(super) fn secure_backup_inspection_admission(
    connectivity_proven: bool,
    inspection_active: bool,
) -> SecureBackupInspectionAdmission {
    if inspection_active {
        SecureBackupInspectionAdmission::Coalesce
    } else if connectivity_proven {
        SecureBackupInspectionAdmission::Start
    } else {
        SecureBackupInspectionAdmission::Defer
    }
}

pub(super) fn apply_secure_backup_connectivity_edge(
    proven: bool,
    retry_attempt: &mut u32,
    recovery_epoch: &mut bool,
    reset_consumed: &mut bool,
) -> bool {
    if !proven {
        if !*recovery_epoch {
            *recovery_epoch = true;
            *reset_consumed = false;
        }
    } else if *recovery_epoch && !*reset_consumed {
        *retry_attempt = 0;
        *reset_consumed = true;
        return true;
    }
    false
}

pub(super) fn secure_backup_monitor_wakeup_is_current(
    current_generation: u64,
    current_monitor_serial: u64,
    session_promoted: bool,
    wake_generation: u64,
    wake_monitor_serial: u64,
) -> bool {
    session_promoted
        && wake_generation == current_generation
        && wake_monitor_serial == current_monitor_serial
}

pub(super) struct PendingRecoveryTask {
    pub(super) generation: u64,
    pub(super) flow_id: u64,
    pub(super) request_id: RequestId,
    pub(super) task: crate::executor::JoinHandle<()>,
}

pub(super) struct PendingRecoveryCompletion {
    pub(super) generation: u64,
    pub(super) flow_id: u64,
    pub(super) request_id: RequestId,
    pub(super) account_key: AccountKey,
}

pub(super) fn recovery_verification_event(stage: &'static str, flow_id: u64) -> DiagnosticEvent {
    DiagnosticEvent::new(DiagnosticLevel::Info, "core.recovery_verification", stage)
        .field(DiagnosticField::count("flow_id", flow_id))
        .field(DiagnosticField::token("flow_type", "recovery_key"))
}

pub(super) fn record_recovery_verification_event(event: DiagnosticEvent) {
    koushi_diagnostics::record_and_stderr(event);
}

fn secure_backup_inspection_completion_action(
    current_generation: u64,
    session_promoted: bool,
    was_admitted: bool,
    generation: u64,
    result: Result<
        koushi_sdk::MatrixSecureBackupInspection,
        koushi_state::SecureBackupGateFailureKind,
    >,
) -> Option<AppAction> {
    if generation != current_generation || !session_promoted {
        return None;
    }
    let gate = match result {
        Ok(inspection) => inspection.recommended_gate_state(),
        Err(
            failure @ (koushi_state::SecureBackupGateFailureKind::Network
            | koushi_state::SecureBackupGateFailureKind::RateLimited
            | koushi_state::SecureBackupGateFailureKind::Timeout),
        ) if was_admitted => koushi_state::SecureBackupGateState::DegradedRetrying { failure },
        Err(failure) => koushi_state::SecureBackupGateState::BlockedFailed { failure },
    };
    Some(AppAction::SecureBackupGateChanged(gate))
}

fn secure_backup_gate_token(gate: &koushi_state::SecureBackupGateState) -> &'static str {
    use koushi_state::SecureBackupGateState;
    match gate {
        SecureBackupGateState::Inactive => "inactive",
        SecureBackupGateState::Checking => "checking",
        SecureBackupGateState::ExistingBackupNeedsRecovery { .. } => "recovery_required",
        SecureBackupGateState::SecureStorageIncomplete => "secure_storage_incomplete",
        SecureBackupGateState::SetupRequired => "setup_required",
        SecureBackupGateState::ExplicitlyDisabledRequiresSetup => "explicitly_disabled",
        SecureBackupGateState::CreatingBackup => "creating",
        SecureBackupGateState::RecoveryKeyDeliveryRequired => "delivery_required",
        SecureBackupGateState::UploadingExistingKeys { .. } => "uploading",
        SecureBackupGateState::DegradedRetrying { .. } => "degraded",
        SecureBackupGateState::BlockedFailed { .. } => "blocked_failed",
        SecureBackupGateState::Ready => "ready",
    }
}

fn recovery_result_is_current(
    generation: u64,
    current_generation: u64,
    flow_id: u64,
    current_flow_id: u64,
    request_id: RequestId,
    current_request_id: RequestId,
    has_session: bool,
) -> bool {
    has_session
        && generation == current_generation
        && flow_id == current_flow_id
        && request_id == current_request_id
}

/// Map an `E2eeRecoveryError` to a coarse `RecoveryFailureKind` without
/// exposing raw SDK error text in public events or error messages.
/// Conservative classification: prefer InvalidRecoveryKey for auth-type SDK
/// errors, Network for network errors, Server for anything else.
fn classify_recovery_error(
    error: &koushi_sdk::E2eeRecoveryError,
) -> koushi_protocol::failure::RecoveryFailureKind {
    use koushi_protocol::failure::RecoveryFailureKind;
    use koushi_sdk::E2eeRecoveryError;
    match error {
        E2eeRecoveryError::Runtime(_) => RecoveryFailureKind::Network,
        E2eeRecoveryError::Sdk(message) => {
            // Classify by error text fragments — these fragments come from the
            // SDK/server and are used only for kind selection, never emitted.
            if message.contains("invalid")
                || message.contains("Invalid")
                || message.contains("M_FORBIDDEN")
                || message.contains("401")
                || message.contains("403")
            {
                RecoveryFailureKind::InvalidRecoveryKey
            } else if message.contains("network")
                || message.contains("timeout")
                || message.contains("connection")
                || message.contains("connect")
            {
                RecoveryFailureKind::Network
            } else {
                RecoveryFailureKind::Server
            }
        }
    }
}

pub(super) fn classify_e2ee_trust_error(
    error: &koushi_sdk::E2eeTrustError,
) -> TrustOperationFailureKind {
    match error {
        koushi_sdk::E2eeTrustError::Classified(kind) => match kind {
            koushi_sdk::E2eeTrustFailureKind::Network => TrustOperationFailureKind::Network,
            koushi_sdk::E2eeTrustFailureKind::Forbidden => TrustOperationFailureKind::Forbidden,
            koushi_sdk::E2eeTrustFailureKind::InvalidBackup => TrustOperationFailureKind::Mismatch,
            koushi_sdk::E2eeTrustFailureKind::Timeout => TrustOperationFailureKind::Timeout,
            koushi_sdk::E2eeTrustFailureKind::Sdk => TrustOperationFailureKind::Sdk,
        },
        koushi_sdk::E2eeTrustError::NoOlmMachine
        | koushi_sdk::E2eeTrustError::SecureBackupInspectionInconclusive
        | koushi_sdk::E2eeTrustError::SecureBackupAlreadyExists
        | koushi_sdk::E2eeTrustError::SecureBackupReenableConfirmationRequired
        | koushi_sdk::E2eeTrustError::SecureBackupUploadFailed
        | koushi_sdk::E2eeTrustError::SecureBackupRecoveryKeyDeliveryFailed => {
            TrustOperationFailureKind::Sdk
        }
        koushi_sdk::E2eeTrustError::Sdk(message) => {
            let lower = message.to_ascii_lowercase();
            if lower.contains("passphrase")
                || lower.contains("mac")
                || lower.contains("decrypt")
                || lower.contains("recovery key")
                || lower.contains("invalid key")
            {
                TrustOperationFailureKind::InvalidPassphrase
            } else if lower.contains("timeout") {
                TrustOperationFailureKind::Timeout
            } else if lower.contains("forbidden")
                || lower.contains("m_forbidden")
                || lower.contains("401")
                || lower.contains("403")
            {
                TrustOperationFailureKind::Forbidden
            } else if lower.contains("network")
                || lower.contains("connection")
                || lower.contains("connect")
            {
                TrustOperationFailureKind::Network
            } else {
                TrustOperationFailureKind::Sdk
            }
        }
    }
}

fn classify_secure_backup_gate_failure(
    error: &koushi_sdk::E2eeTrustError,
) -> koushi_state::SecureBackupGateFailureKind {
    use koushi_sdk::E2eeTrustError;
    use koushi_state::SecureBackupGateFailureKind;

    match error {
        E2eeTrustError::SecureBackupUploadFailed => SecureBackupGateFailureKind::Network,
        E2eeTrustError::SecureBackupRecoveryKeyDeliveryFailed => {
            SecureBackupGateFailureKind::ArtifactDelivery
        }
        E2eeTrustError::SecureBackupInspectionInconclusive => SecureBackupGateFailureKind::Sdk,
        E2eeTrustError::SecureBackupAlreadyExists => SecureBackupGateFailureKind::BackupKeyMismatch,
        E2eeTrustError::NoOlmMachine | E2eeTrustError::SecureBackupReenableConfirmationRequired => {
            SecureBackupGateFailureKind::Sdk
        }
        E2eeTrustError::Classified(_) => match classify_e2ee_trust_error(error) {
            TrustOperationFailureKind::Timeout => SecureBackupGateFailureKind::Timeout,
            TrustOperationFailureKind::Forbidden => SecureBackupGateFailureKind::Forbidden,
            TrustOperationFailureKind::Network => SecureBackupGateFailureKind::Network,
            TrustOperationFailureKind::Mismatch => SecureBackupGateFailureKind::BackupKeyMismatch,
            _ => SecureBackupGateFailureKind::Sdk,
        },
        E2eeTrustError::Sdk(_) => match classify_e2ee_trust_error(error) {
            TrustOperationFailureKind::InvalidPassphrase => {
                SecureBackupGateFailureKind::InvalidRecoveryKey
            }
            TrustOperationFailureKind::Timeout => SecureBackupGateFailureKind::Timeout,
            TrustOperationFailureKind::Forbidden => SecureBackupGateFailureKind::Forbidden,
            TrustOperationFailureKind::Network => SecureBackupGateFailureKind::Network,
            TrustOperationFailureKind::Mismatch => SecureBackupGateFailureKind::BackupKeyMismatch,
            TrustOperationFailureKind::Cancelled | TrustOperationFailureKind::Sdk => {
                SecureBackupGateFailureKind::Sdk
            }
        },
    }
}

pub(super) fn classify_e2ee_trust_auth_failure(
    error: &koushi_sdk::E2eeTrustError,
) -> AuthFailureKind {
    match classify_e2ee_trust_error(error) {
        TrustOperationFailureKind::Network => AuthFailureKind::Network,
        TrustOperationFailureKind::Forbidden => AuthFailureKind::Forbidden,
        TrustOperationFailureKind::Timeout => AuthFailureKind::Timeout,
        TrustOperationFailureKind::Cancelled
        | TrustOperationFailureKind::Mismatch
        | TrustOperationFailureKind::InvalidPassphrase
        | TrustOperationFailureKind::Sdk => AuthFailureKind::Sdk,
    }
}

fn project_bootstrap_cross_signing_result(
    request_id: RequestId,
    account_key: AccountKey,
    result: Result<koushi_state::CrossSigningStatus, koushi_sdk::E2eeTrustError>,
) -> (Vec<AppAction>, Vec<CoreEvent>) {
    match result {
        Ok(status) => (
            vec![AppAction::CrossSigningStatusChanged {
                status: status.clone(),
            }],
            vec![CoreEvent::E2eeTrust(E2eeTrustEvent::CrossSigningChanged {
                account_key,
                status,
            })],
        ),
        Err(error) => {
            let kind = classify_e2ee_trust_error(&error);
            let status = koushi_state::CrossSigningStatus::Failed {
                request_id: request_id.sequence,
                kind,
            };
            (
                vec![AppAction::BootstrapCrossSigningFailed {
                    request_id: request_id.sequence,
                    kind,
                }],
                vec![CoreEvent::E2eeTrust(E2eeTrustEvent::CrossSigningChanged {
                    account_key,
                    status,
                })],
            )
        }
    }
}

fn project_enable_key_backup_result(
    request_id: RequestId,
    account_key: AccountKey,
    result: Result<koushi_state::KeyBackupStatus, koushi_sdk::E2eeTrustError>,
) -> (Vec<AppAction>, Vec<CoreEvent>) {
    match result {
        Ok(koushi_state::KeyBackupStatus::Enabled { version }) => {
            let status = koushi_state::KeyBackupStatus::Enabled {
                version: version.clone(),
            };
            (
                vec![AppAction::KeyBackupEnabled {
                    request_id: request_id.sequence,
                    version,
                }],
                vec![CoreEvent::E2eeTrust(E2eeTrustEvent::KeyBackupChanged {
                    account_key,
                    status,
                })],
            )
        }
        Ok(status) => (
            vec![AppAction::KeyBackupFailed {
                request_id: request_id.sequence,
                kind: TrustOperationFailureKind::Sdk,
            }],
            vec![CoreEvent::E2eeTrust(E2eeTrustEvent::KeyBackupChanged {
                account_key,
                status,
            })],
        ),
        Err(error) => {
            let kind = classify_e2ee_trust_error(&error);
            let status = koushi_state::KeyBackupStatus::Failed {
                request_id: request_id.sequence,
                kind,
            };
            (
                vec![AppAction::KeyBackupFailed {
                    request_id: request_id.sequence,
                    kind,
                }],
                vec![CoreEvent::E2eeTrust(E2eeTrustEvent::KeyBackupChanged {
                    account_key,
                    status,
                })],
            )
        }
    }
}

fn project_restore_key_backup_result(
    request_id: RequestId,
    account_key: AccountKey,
    result: Result<koushi_sdk::KeyBackupRestoreSummary, koushi_sdk::E2eeTrustError>,
) -> (Vec<AppAction>, Vec<CoreEvent>) {
    match result {
        Ok(summary) => {
            let progress_status = koushi_state::KeyBackupStatus::Restoring {
                request_id: request_id.sequence,
                version: summary.version.clone(),
                restored_rooms: summary.restored_rooms,
                total_rooms: summary.total_rooms,
            };
            let restored_status = match summary.version.clone() {
                Some(version) => koushi_state::KeyBackupStatus::Enabled { version },
                None => koushi_state::KeyBackupStatus::Unknown,
            };
            (
                vec![
                    AppAction::KeyBackupRestoreProgress {
                        request_id: request_id.sequence,
                        restored_rooms: summary.restored_rooms,
                        total_rooms: summary.total_rooms,
                    },
                    AppAction::KeyBackupRestored {
                        request_id: request_id.sequence,
                        version: summary.version,
                    },
                ],
                vec![
                    CoreEvent::E2eeTrust(E2eeTrustEvent::KeyBackupChanged {
                        account_key: account_key.clone(),
                        status: progress_status,
                    }),
                    CoreEvent::E2eeTrust(E2eeTrustEvent::KeyBackupChanged {
                        account_key,
                        status: restored_status,
                    }),
                ],
            )
        }
        Err(error) => {
            let kind = classify_e2ee_trust_error(&error);
            let status = koushi_state::KeyBackupStatus::Failed {
                request_id: request_id.sequence,
                kind,
            };
            (
                vec![AppAction::KeyBackupFailed {
                    request_id: request_id.sequence,
                    kind,
                }],
                vec![CoreEvent::E2eeTrust(E2eeTrustEvent::KeyBackupChanged {
                    account_key,
                    status,
                })],
            )
        }
    }
}

pub(super) fn project_reset_identity_completed(
    request_id: RequestId,
    account_key: AccountKey,
) -> (Vec<AppAction>, Vec<CoreEvent>) {
    (
        vec![AppAction::ResetIdentityCompleted {
            request_id: request_id.sequence,
        }],
        vec![CoreEvent::E2eeTrust(E2eeTrustEvent::IdentityResetChanged {
            account_key,
            state: IdentityResetState::Idle,
        })],
    )
}

fn project_reset_identity_auth_required(
    request_id: RequestId,
    account_key: AccountKey,
    auth_type: IdentityResetAuthType,
) -> (Vec<AppAction>, Vec<CoreEvent>) {
    let state = IdentityResetState::AwaitingAuth {
        request_id: request_id.sequence,
        auth_type,
    };
    (
        vec![AppAction::ResetIdentityAuthRequired {
            request_id: request_id.sequence,
            auth_type,
        }],
        vec![CoreEvent::E2eeTrust(E2eeTrustEvent::IdentityResetChanged {
            account_key,
            state,
        })],
    )
}

pub(super) fn project_reset_identity_error(
    request_id: RequestId,
    account_key: AccountKey,
    error: koushi_sdk::E2eeTrustError,
) -> (Vec<AppAction>, Vec<CoreEvent>) {
    let kind = classify_e2ee_trust_error(&error);
    let state = IdentityResetState::Failed {
        request_id: request_id.sequence,
        kind,
    };
    (
        vec![AppAction::ResetIdentityFailed {
            request_id: request_id.sequence,
            kind,
        }],
        vec![
            CoreEvent::E2eeTrust(E2eeTrustEvent::CrossSigningChanged {
                account_key: account_key.clone(),
                status: CrossSigningStatus::Failed {
                    request_id: request_id.sequence,
                    kind,
                },
            }),
            CoreEvent::E2eeTrust(E2eeTrustEvent::IdentityResetChanged { account_key, state }),
        ],
    )
}

pub(super) fn project_identity_reset_failed_event(
    request_id: u64,
    account_key: AccountKey,
    kind: TrustOperationFailureKind,
) -> Vec<CoreEvent> {
    vec![
        CoreEvent::E2eeTrust(E2eeTrustEvent::CrossSigningChanged {
            account_key: account_key.clone(),
            status: CrossSigningStatus::Failed { request_id, kind },
        }),
        CoreEvent::E2eeTrust(E2eeTrustEvent::IdentityResetChanged {
            account_key,
            state: IdentityResetState::Failed { request_id, kind },
        }),
    ]
}

impl AccountActor {
    pub(super) async fn handle_bootstrap_cross_signing(
        &self,
        request_id: RequestId,
        auth: Option<koushi_state::AuthSecret>,
    ) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.send_actions(vec![AppAction::BootstrapCrossSigningFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };
        let account_key = AccountKey(session.info.user_id.clone());
        let result = koushi_sdk::bootstrap_cross_signing(&session, auth.as_ref()).await;
        let (actions, events) =
            project_bootstrap_cross_signing_result(request_id, account_key, result);
        self.send_actions(actions).await;
        for event in events {
            self.emit(event);
        }
    }

    pub(super) async fn handle_enable_key_backup(
        &self,
        request_id: RequestId,
        passphrase: Option<koushi_state::AuthSecret>,
    ) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.send_actions(vec![AppAction::KeyBackupFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };
        let account_key = AccountKey(session.info.user_id.clone());
        let result = koushi_sdk::enable_key_backup(&session, passphrase.as_ref()).await;
        drop(passphrase);
        let (actions, events) = project_enable_key_backup_result(request_id, account_key, result);
        self.send_actions(actions).await;
        for event in events {
            self.emit(event);
        }
    }

    pub(super) async fn handle_restore_key_backup(
        &self,
        request_id: RequestId,
        version: Option<String>,
        request: RecoveryRequest,
    ) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.send_actions(vec![AppAction::KeyBackupFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };
        let account_key = AccountKey(session.info.user_id.clone());
        let result = koushi_sdk::restore_key_backup(&session, &request, version.as_deref()).await;
        drop(request);

        let (actions, events) = project_restore_key_backup_result(request_id, account_key, result);
        self.send_actions(actions).await;
        for event in events {
            self.emit(event);
        }
    }

    pub(super) async fn handle_export_room_keys(
        &self,
        request_id: RequestId,
        request: RoomKeyExportRequest,
    ) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.native_artifacts
                    .unregister(request_id, NativeArtifactKind::RoomKeyExportDestination);
                self.send_actions(vec![AppAction::RoomKeyExportFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };

        let RoomKeyExportRequest { passphrase } = request;
        let destination_path = match self
            .native_artifacts
            .take(request_id, NativeArtifactKind::RoomKeyExportDestination)
        {
            Ok(path) => path,
            Err(_) => {
                self.send_actions(vec![AppAction::RoomKeyExportFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }])
                .await;
                self.emit_failure(
                    request_id,
                    CoreFailure::AccountOperationFailed {
                        kind: AuthFailureKind::Sdk,
                    },
                );
                return;
            }
        };
        let result =
            koushi_sdk::export_room_keys_to_file(&session, destination_path, &passphrase).await;
        drop(passphrase);
        match result {
            Ok(summary) => {
                self.send_actions(vec![AppAction::RoomKeyExported {
                    request_id: request_id.sequence,
                    exported_sessions: summary.exported_sessions,
                }])
                .await;
            }
            Err(error) => {
                let kind = classify_e2ee_trust_error(&error);
                self.send_actions(vec![AppAction::RoomKeyExportFailed {
                    request_id: request_id.sequence,
                    kind,
                }])
                .await;
                self.emit_failure(
                    request_id,
                    CoreFailure::AccountOperationFailed {
                        kind: classify_e2ee_trust_auth_failure(&error),
                    },
                );
            }
        }
    }

    pub(super) async fn handle_import_room_keys(
        &self,
        request_id: RequestId,
        request: RoomKeyImportRequest,
    ) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.native_artifacts
                    .unregister(request_id, NativeArtifactKind::RoomKeyImportSource);
                self.send_actions(vec![AppAction::RoomKeyImportFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };

        let RoomKeyImportRequest { passphrase } = request;
        let source_path = match self
            .native_artifacts
            .take(request_id, NativeArtifactKind::RoomKeyImportSource)
        {
            Ok(path) => path,
            Err(_) => {
                self.send_actions(vec![AppAction::RoomKeyImportFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }])
                .await;
                self.emit_failure(
                    request_id,
                    CoreFailure::AccountOperationFailed {
                        kind: AuthFailureKind::Sdk,
                    },
                );
                return;
            }
        };
        let result =
            koushi_sdk::import_room_keys_from_file(&session, source_path, &passphrase).await;
        drop(passphrase);
        match result {
            Ok(summary) => {
                self.send_actions(vec![AppAction::RoomKeyImported {
                    request_id: request_id.sequence,
                    imported_count: summary.imported_count,
                    total_count: summary.total_count,
                }])
                .await;
            }
            Err(error) => {
                let kind = classify_e2ee_trust_error(&error);
                self.send_actions(vec![AppAction::RoomKeyImportFailed {
                    request_id: request_id.sequence,
                    kind,
                }])
                .await;
                self.emit_failure(
                    request_id,
                    CoreFailure::AccountOperationFailed {
                        kind: classify_e2ee_trust_auth_failure(&error),
                    },
                );
            }
        }
    }

    pub(super) async fn handle_bootstrap_secure_backup(
        &mut self,
        request_id: RequestId,
        request: SecureBackupSetupRequest,
    ) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.native_artifacts
                    .unregister(request_id, NativeArtifactKind::RecoveryKeyDestination);
                self.send_actions(vec![AppAction::SecureBackupSetupFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };

        let SecureBackupSetupRequest {
            passphrase,
            recovery_key_destination_requested,
            intent,
        } = request;
        if !recovery_key_destination_requested {
            self.native_artifacts
                .unregister(request_id, NativeArtifactKind::RecoveryKeyDestination);
            self.send_actions(vec![AppAction::SecureBackupGateChanged(
                koushi_state::SecureBackupGateState::RecoveryKeyDeliveryRequired,
            )])
            .await;
            self.emit_failure(
                request_id,
                CoreFailure::AccountOperationFailed {
                    kind: AuthFailureKind::Sdk,
                },
            );
            return;
        }
        let recovery_key_destination_path = match self
            .native_artifacts
            .take(request_id, NativeArtifactKind::RecoveryKeyDestination)
        {
            Ok(path) => Some(path),
            Err(_) => {
                self.send_actions(vec![AppAction::SecureBackupGateChanged(
                    koushi_state::SecureBackupGateState::RecoveryKeyDeliveryRequired,
                )])
                .await;
                self.emit_failure(
                    request_id,
                    CoreFailure::AccountOperationFailed {
                        kind: AuthFailureKind::Sdk,
                    },
                );
                return;
            }
        };
        self.send_actions(vec![AppAction::SecureBackupGateChanged(
            koushi_state::SecureBackupGateState::CreatingBackup,
        )])
        .await;
        let result = match intent {
            SecureBackupSetupIntent::InitialSetup => {
                session
                    .setup_secure_backup(passphrase.as_ref(), recovery_key_destination_path)
                    .await
            }
            SecureBackupSetupIntent::Reenable { confirmed: true } => {
                session
                    .reenable_secure_backup(passphrase.as_ref(), recovery_key_destination_path)
                    .await
            }
            SecureBackupSetupIntent::Reenable { confirmed: false } => {
                Err(koushi_sdk::E2eeTrustError::SecureBackupReenableConfirmationRequired)
            }
        };
        drop(passphrase);
        match result {
            Ok(summary) => {
                self.recovery_key_delivery_pending = false;
                let delivery = if summary.recovery_key_written {
                    RecoveryKeyDeliveryState::Written
                } else {
                    RecoveryKeyDeliveryState::NotWritten
                };
                self.send_actions(vec![
                    AppAction::SecureBackupRecoveryKeyReady {
                        request_id: request_id.sequence,
                        delivery,
                    },
                    AppAction::SecureBackupSetupEnabled {
                        request_id: request_id.sequence,
                    },
                ])
                .await;
                self.start_secure_backup_inspection();
            }
            Err(error) => {
                let kind = classify_e2ee_trust_error(&error);
                let mut actions = vec![AppAction::SecureBackupSetupFailed {
                    request_id: request_id.sequence,
                    kind,
                }];
                if matches!(
                    error,
                    koushi_sdk::E2eeTrustError::SecureBackupRecoveryKeyDeliveryFailed
                ) {
                    self.recovery_key_delivery_pending = true;
                    self.set_secure_backup_send_admitted(false);
                    actions.push(AppAction::SecureBackupGateChanged(
                        koushi_state::SecureBackupGateState::RecoveryKeyDeliveryRequired,
                    ));
                }
                self.send_actions(actions).await;
                self.emit_failure(
                    request_id,
                    CoreFailure::AccountOperationFailed {
                        kind: classify_e2ee_trust_auth_failure(&error),
                    },
                );
            }
        }
    }

    pub(super) async fn handle_recover_secure_backup(
        &mut self,
        request_id: RequestId,
        request: RecoveryRequest,
    ) {
        let Some(session) = self.session.clone().filter(|_| self.session_promoted) else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        self.send_actions(vec![AppAction::SecureBackupGateChanged(
            koushi_state::SecureBackupGateState::Checking,
        )])
        .await;
        match session.recover_secure_backup(&request).await {
            Ok(()) => self.start_secure_backup_inspection(),
            Err(error) => {
                self.send_actions(vec![AppAction::SecureBackupGateChanged(
                    koushi_state::SecureBackupGateState::ExistingBackupNeedsRecovery {
                        failure: Some(classify_secure_backup_gate_failure(&error)),
                    },
                )])
                .await;
                self.emit_failure(
                    request_id,
                    CoreFailure::AccountOperationFailed {
                        kind: classify_e2ee_trust_auth_failure(&error),
                    },
                );
            }
        }
    }

    pub(super) async fn handle_change_secure_backup_passphrase(
        &self,
        request_id: RequestId,
        request: SecureBackupPassphraseChangeRequest,
    ) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.native_artifacts
                    .unregister(request_id, NativeArtifactKind::RecoveryKeyDestination);
                self.send_actions(vec![AppAction::SecureBackupPassphraseChangeFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };

        let SecureBackupPassphraseChangeRequest {
            old_secret,
            new_passphrase,
            recovery_key_destination_requested,
        } = request;
        let recovery_key_destination_path = if recovery_key_destination_requested {
            match self
                .native_artifacts
                .take(request_id, NativeArtifactKind::RecoveryKeyDestination)
            {
                Ok(path) => Some(path),
                Err(_) => {
                    self.send_actions(vec![AppAction::SecureBackupPassphraseChangeFailed {
                        request_id: request_id.sequence,
                        kind: TrustOperationFailureKind::Sdk,
                    }])
                    .await;
                    self.emit_failure(
                        request_id,
                        CoreFailure::AccountOperationFailed {
                            kind: AuthFailureKind::Sdk,
                        },
                    );
                    return;
                }
            }
        } else {
            self.native_artifacts
                .unregister(request_id, NativeArtifactKind::RecoveryKeyDestination);
            None
        };
        let result = koushi_sdk::change_secure_backup_passphrase(
            &session,
            &old_secret,
            &new_passphrase,
            recovery_key_destination_path,
        )
        .await;
        drop(old_secret);
        drop(new_passphrase);
        match result {
            Ok(summary) => {
                let delivery = if summary.recovery_key_written {
                    RecoveryKeyDeliveryState::Written
                } else {
                    RecoveryKeyDeliveryState::NotWritten
                };
                self.send_actions(vec![AppAction::SecureBackupPassphraseChanged {
                    request_id: request_id.sequence,
                    delivery,
                }])
                .await;
            }
            Err(error) => {
                let kind = classify_e2ee_trust_error(&error);
                self.send_actions(vec![AppAction::SecureBackupPassphraseChangeFailed {
                    request_id: request_id.sequence,
                    kind,
                }])
                .await;
                self.emit_failure(
                    request_id,
                    CoreFailure::AccountOperationFailed {
                        kind: classify_e2ee_trust_auth_failure(&error),
                    },
                );
            }
        }
    }

    pub(super) async fn handle_reset_identity(&mut self, request_id: RequestId) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.cancel_identity_reset_handle().await;
                self.send_actions(vec![AppAction::ResetIdentityFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };
        let account_key = AccountKey(session.info.user_id.clone());
        match koushi_sdk::reset_identity(&session).await {
            Ok(koushi_sdk::IdentityResetOutcome::Completed) => {
                self.cancel_identity_reset_handle().await;
                let (actions, events) = project_reset_identity_completed(request_id, account_key);
                self.send_actions(actions).await;
                for event in events {
                    self.emit(event);
                }
            }
            Ok(koushi_sdk::IdentityResetOutcome::AuthRequired(handle)) => {
                let auth_type = handle.desktop_auth_type();
                self.cancel_identity_reset_handle().await;
                self.identity_reset_flow_id = Some(request_id.sequence);
                self.spawn_identity_reset_auth_timeout(request_id.sequence);
                self.identity_reset_handle = Some(handle);
                let (actions, events) =
                    project_reset_identity_auth_required(request_id, account_key, auth_type);
                self.send_actions(actions).await;
                for event in events {
                    self.emit(event);
                }
            }
            Err(error) => {
                self.cancel_identity_reset_handle().await;
                let (actions, events) =
                    project_reset_identity_error(request_id, account_key, error);
                self.send_actions(actions).await;
                for event in events {
                    self.emit(event);
                }
            }
        }
    }

    pub(super) async fn handle_start_session_bootstrap(
        &mut self,
        request_id: RequestId,
        flow_id: u64,
        auth: Option<koushi_state::AuthSecret>,
        request: SecureBackupSetupRequest,
    ) {
        let Some(session) = self.session.clone() else {
            self.native_artifacts
                .unregister(request_id, NativeArtifactKind::RecoveryKeyDestination);
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        if !request.recovery_key_destination_requested {
            self.native_artifacts
                .unregister(request_id, NativeArtifactKind::RecoveryKeyDestination);
            self.send_actions(vec![AppAction::BootstrapRecoveryKeyDeliveryFailed {
                flow_id,
                kind: koushi_state::VerificationGateFailureKind::Sdk,
            }])
            .await;
            return;
        }
        if let Err(error) = koushi_sdk::bootstrap_cross_signing(&session, auth.as_ref()).await {
            drop(auth);
            self.native_artifacts
                .unregister(request_id, NativeArtifactKind::RecoveryKeyDestination);
            self.send_actions(vec![AppAction::BootstrapRecoveryKeyDeliveryFailed {
                flow_id,
                kind: verification_gate_failure_kind(&error),
            }])
            .await;
            return;
        }
        drop(auth);
        let SecureBackupSetupRequest {
            passphrase,
            recovery_key_destination_requested: _,
            intent: _,
        } = request;
        let recovery_key_destination_path = match self
            .native_artifacts
            .take(request_id, NativeArtifactKind::RecoveryKeyDestination)
        {
            Ok(path) => Some(path),
            Err(_) => {
                self.send_actions(vec![AppAction::BootstrapRecoveryKeyDeliveryFailed {
                    flow_id,
                    kind: koushi_state::VerificationGateFailureKind::Sdk,
                }])
                .await;
                return;
            }
        };
        let result = koushi_sdk::bootstrap_secure_backup(
            &session,
            passphrase.as_ref(),
            recovery_key_destination_path,
        )
        .await;
        drop(passphrase);
        match result {
            Ok(summary) if summary.recovery_key_written => {
                self.send_actions(vec![AppAction::BootstrapRecoveryKeyDelivered { flow_id }])
                    .await;
            }
            Ok(_) => {
                self.send_actions(vec![AppAction::BootstrapRecoveryKeyDeliveryFailed {
                    flow_id,
                    kind: koushi_state::VerificationGateFailureKind::Sdk,
                }])
                .await;
            }
            Err(error) => {
                self.send_actions(vec![AppAction::BootstrapRecoveryKeyDeliveryFailed {
                    flow_id,
                    kind: verification_gate_failure_kind(&error),
                }])
                .await;
            }
        }
    }

    /// Submit a recovery secret. Calls the auth crate's `recover_e2ee`
    /// primitive. On success, the accepted proof is not enough to enter Ready:
    /// wait until the SDK reports the current device as Verified. On failure:
    /// classify conservatively to
    /// InvalidRecoveryKey/Network/Server (never raw error text) and emit
    /// OperationFailed with RecoveryFailed.
    ///
    /// The recovery secret is NEVER logged, included in error messages, or
    /// stored in any event/snapshot.
    pub(super) async fn handle_submit_recovery(
        &mut self,
        request_id: RequestId,
        request: RecoveryRequest,
    ) {
        let session = match &self.session {
            Some(s) => s.clone(),
            None => {
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };

        self.stop_recovery_task().await;
        self.stop_recovery_trust_settlement_task().await;
        self.pending_recovery_completion = None;
        let generation = self.trust_generation;
        let flow_id = request_id.sequence;
        let provisional_encryption_sync_was_active = self.provisional_encryption_sync.is_some();
        self.stop_provisional_encryption_sync().await;
        record_recovery_verification_event(
            recovery_verification_event("provisional_encryption_sync_paused", flow_id).field(
                DiagnosticField::boolean("was_active", provisional_encryption_sync_was_active),
            ),
        );
        record_recovery_verification_event(recovery_verification_event("submitted", flow_id));
        let tx = self.self_tx.clone();
        #[cfg(test)]
        let recovery_result_override = self
            .recovery_result_override
            .lock()
            .expect("recovery result lock")
            .take();
        let task = crate::executor::spawn(async move {
            #[cfg(test)]
            let result = if let Some(completion) = recovery_result_override {
                completion.await.unwrap_or_else(|_| {
                    Err(koushi_sdk::E2eeRecoveryError::Runtime(
                        "synthetic recovery result channel closed".to_owned(),
                    ))
                })
            } else {
                koushi_sdk::recover_e2ee(&session, &request).await
            };
            #[cfg(not(test))]
            let result = koushi_sdk::recover_e2ee(&session, &request).await;
            drop(request);
            let _ = tx
                .send(AccountMessage::RecoveryFinished {
                    generation,
                    flow_id,
                    request_id,
                    result,
                })
                .await;
        });
        self.recovery_task = Some(PendingRecoveryTask {
            generation,
            flow_id,
            request_id,
            task,
        });
    }

    pub(super) async fn handle_recovery_finished(
        &mut self,
        generation: u64,
        flow_id: u64,
        request_id: RequestId,
        result: Result<(), koushi_sdk::E2eeRecoveryError>,
    ) {
        let is_current = self.recovery_task.as_ref().is_some_and(|pending| {
            recovery_result_is_current(
                generation,
                self.trust_generation,
                flow_id,
                pending.flow_id,
                request_id,
                pending.request_id,
                self.session.is_some(),
            ) && pending.generation == generation
        });
        if !is_current {
            return;
        }
        if let Some(pending) = self.recovery_task.take() {
            let _ = pending.task.await;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        let account_key = AccountKey(session.info.user_id.clone());
        match result {
            Ok(()) => {
                record_recovery_verification_event(
                    recovery_verification_event("settled", flow_id)
                        .field(DiagnosticField::token("terminal", "success")),
                );
                let trust_after_recovery = session.current_device_trust();
                record_recovery_verification_event(
                    recovery_verification_event("post_recovery_trust_read", flow_id)
                        .field(DiagnosticField::count("generation", generation))
                        .field(DiagnosticField::request_id(
                            "request_id",
                            request_id.connection_id.0,
                            request_id.sequence,
                        ))
                        .field(DiagnosticField::token(
                            "trust",
                            current_device_trust_token(trust_after_recovery),
                        )),
                );
                if trust_after_recovery != koushi_state::CurrentDeviceTrustState::Verified {
                    self.resume_provisional_encryption_sync_after_recovery(
                        session.clone(),
                        generation,
                        flow_id,
                    );
                    self.pending_recovery_completion = Some(PendingRecoveryCompletion {
                        generation,
                        flow_id,
                        request_id,
                        account_key,
                    });
                    record_recovery_verification_event(
                        recovery_verification_event("trust_pending", flow_id)
                            .field(DiagnosticField::count("generation", generation))
                            .field(DiagnosticField::request_id(
                                "request_id",
                                request_id.connection_id.0,
                                request_id.sequence,
                            ))
                            .field(DiagnosticField::token(
                                "trust",
                                current_device_trust_token(trust_after_recovery),
                            )),
                    );
                    let transition_id = self.next_trust_transition_id();
                    self.send_actions(vec![AppAction::AuthoritativeDeviceTrustChanged {
                        generation,
                        transition_id,
                        trust: trust_after_recovery,
                    }])
                    .await;
                    self.start_recovery_trust_settlement_poll(
                        generation, flow_id, request_id, session,
                    )
                    .await;
                    return;
                }
                if !self
                    .promote_recovered_session_runtime(generation, flow_id, request_id)
                    .await
                {
                    return;
                }
                self.send_actions(vec![AppAction::E2eeRecoverySucceeded])
                    .await;
                self.complete_recovery_after_verified(request_id, account_key, session)
                    .await;
            }
            Err(error) => {
                self.resume_provisional_encryption_sync_after_recovery(
                    session.clone(),
                    generation,
                    flow_id,
                );
                self.pending_recovery_completion = None;
                let kind = classify_recovery_error(&error);
                record_recovery_verification_event(
                    recovery_verification_event("settled", flow_id)
                        .field(DiagnosticField::token("terminal", "failed"))
                        .field(DiagnosticField::token(
                            "failure_kind",
                            recovery_failure_token(kind),
                        )),
                );
                // Project failure: Recovering → NeedsRecovery.
                record_device_cleanup_offer("recovery_failed");
                self.send_actions(vec![AppAction::E2eeRecoveryFailed {
                    message: "recovery failed".to_owned(),
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::RecoveryFailed { kind });
            }
        }
    }

    pub(super) async fn complete_recovery_after_verified(
        &mut self,
        request_id: RequestId,
        account_key: AccountKey,
        session: Arc<MatrixClientSession>,
    ) {
        self.send_actions(vec![AppAction::RestoreKeyBackupRequested {
            request_id: request_id.sequence,
            version: None,
        }])
        .await;
        #[cfg(test)]
        let recovery_download_override = self
            .recovery_download_override
            .lock()
            .expect("recovery download lock")
            .take();
        #[cfg(test)]
        let restore_result = if let Some(completion) = recovery_download_override {
            if completion.await.unwrap_or(false) {
                Ok(koushi_sdk::KeyBackupRestoreSummary {
                    scope: koushi_sdk::KeyBackupRestoreScope::JoinedRooms,
                    version: None,
                    restored_rooms: 0,
                    total_rooms: Some(0),
                })
            } else {
                Err(koushi_sdk::E2eeTrustError::Sdk(
                    "controlled recovery download failure".to_owned(),
                ))
            }
        } else {
            koushi_sdk::download_joined_room_keys_from_backup(&session, None).await
        };
        #[cfg(not(test))]
        let restore_result =
            koushi_sdk::download_joined_room_keys_from_backup(&session, None).await;
        let (actions, events) =
            project_restore_key_backup_result(request_id, account_key.clone(), restore_result);
        self.send_actions(actions).await;
        for event in events {
            self.emit(event);
        }
        self.emit(CoreEvent::Account(AccountEvent::RecoveryCompleted {
            request_id,
            account_key,
        }));
    }

    async fn start_recovery_trust_settlement_poll(
        &mut self,
        generation: u64,
        flow_id: u64,
        request_id: RequestId,
        session: Arc<MatrixClientSession>,
    ) {
        self.stop_recovery_trust_settlement_task().await;
        let tx = self.self_tx.clone();
        record_recovery_verification_event(
            recovery_verification_event("trust_settlement_wait_started", flow_id)
                .field(DiagnosticField::count("generation", generation))
                .field(DiagnosticField::request_id(
                    "request_id",
                    request_id.connection_id.0,
                    request_id.sequence,
                )),
        );
        self.recovery_trust_settlement_task = Some(executor::spawn(async move {
            let started = Instant::now();
            let mut trust = session.current_device_trust();
            while started.elapsed() < RECOVERY_TRUST_SETTLEMENT_TIMEOUT {
                if trust == koushi_state::CurrentDeviceTrustState::Verified {
                    record_recovery_verification_event(
                        recovery_verification_event("trust_settlement_wait_finished", flow_id)
                            .field(DiagnosticField::count("generation", generation))
                            .field(DiagnosticField::token("outcome", "verified"))
                            .field(DiagnosticField::milliseconds(
                                "elapsed_ms",
                                started.elapsed().as_millis(),
                            )),
                    );
                    let _ = tx
                        .send(AccountMessage::CurrentDeviceTrustChanged { generation, trust })
                        .await;
                    return;
                }
                executor::sleep(RECOVERY_TRUST_SETTLEMENT_POLL_INTERVAL).await;
                trust = session.current_device_trust();
            }
            record_recovery_verification_event(
                recovery_verification_event("trust_settlement_wait_finished", flow_id)
                    .field(DiagnosticField::count("generation", generation))
                    .field(DiagnosticField::token("outcome", "timeout"))
                    .field(DiagnosticField::token(
                        "trust",
                        current_device_trust_token(trust),
                    ))
                    .field(DiagnosticField::milliseconds(
                        "elapsed_ms",
                        started.elapsed().as_millis(),
                    )),
            );
            let _ = tx
                .send(AccountMessage::RecoveryTrustSettlementTimedOut {
                    generation,
                    flow_id,
                    request_id,
                    trust,
                })
                .await;
        }));
    }

    pub(super) async fn handle_recovery_trust_settlement_timed_out(
        &mut self,
        generation: u64,
        flow_id: u64,
        request_id: RequestId,
        trust: koushi_state::CurrentDeviceTrustState,
    ) {
        let is_current = self
            .pending_recovery_completion
            .as_ref()
            .is_some_and(|pending| {
                pending.generation == generation
                    && pending.flow_id == flow_id
                    && pending.request_id == request_id
                    && generation == self.trust_generation
                    && self.session.is_some()
            });
        if !is_current {
            record_recovery_verification_event(
                recovery_verification_event("trust_settlement_timeout_ignored", flow_id)
                    .field(DiagnosticField::count("generation", generation))
                    .field(DiagnosticField::request_id(
                        "request_id",
                        request_id.connection_id.0,
                        request_id.sequence,
                    ))
                    .field(DiagnosticField::token(
                        "trust",
                        current_device_trust_token(trust),
                    )),
            );
            return;
        }
        self.pending_recovery_completion = None;
        self.recovery_trust_settlement_task = None;
        record_recovery_verification_event(
            recovery_verification_event("trust_settlement_timeout_projected", flow_id)
                .field(DiagnosticField::count("generation", generation))
                .field(DiagnosticField::request_id(
                    "request_id",
                    request_id.connection_id.0,
                    request_id.sequence,
                ))
                .field(DiagnosticField::token(
                    "trust",
                    current_device_trust_token(trust),
                ))
                .field(DiagnosticField::token("failure_kind", "timeout")),
        );
        record_device_cleanup_offer("recovery_failed");
        self.send_actions(vec![AppAction::E2eeRecoveryFailed {
            message: "session verification timed out".to_owned(),
        }])
        .await;
        self.emit_failure(
            request_id,
            CoreFailure::RecoveryFailed {
                kind: RecoveryFailureKind::Timeout,
            },
        );
    }

    pub(super) async fn stop_recovery_task(&mut self) -> Option<u64> {
        let pending = self.recovery_task.take()?;
        let flow_id = pending.flow_id;
        pending.task.abort();
        let _ = pending.task.await;
        Some(flow_id)
    }

    pub(super) async fn stop_recovery_trust_settlement_task(&mut self) {
        if let Some(task) = self.recovery_trust_settlement_task.take() {
            task.abort();
            let _ = task.await;
        }
    }

    pub(super) fn start_secure_backup_inspection(&mut self) {
        match secure_backup_inspection_admission(
            self.sync_connectivity_proven,
            self.secure_backup_inspection_task.is_some(),
        ) {
            SecureBackupInspectionAdmission::Coalesce => {
                self.secure_backup_inspection_pending = true;
                return;
            }
            SecureBackupInspectionAdmission::Defer => {
                self.retire_secure_backup_monitor();
                self.secure_backup_inspection_pending = true;
                record(
                    DiagnosticEvent::new(
                        DiagnosticLevel::Info,
                        "core.secure_backup",
                        "inspection_deferred",
                    )
                    .field(DiagnosticField::token("reason", "connectivity_unproven")),
                );
                return;
            }
            SecureBackupInspectionAdmission::Start => {}
        }
        self.retire_secure_backup_monitor();
        let Some(session) = self.session.clone().filter(|_| self.session_promoted) else {
            return;
        };
        let generation = self.trust_generation;
        record(DiagnosticEvent::new(
            DiagnosticLevel::Debug,
            "core.secure_backup",
            "inspection_started",
        ));
        let tx = self.self_tx.clone();
        let started_at = Instant::now();
        self.secure_backup_inspection_task = Some(executor::spawn(async move {
            let result = match executor::timeout(
                SECURE_BACKUP_INSPECTION_TIMEOUT,
                session.inspect_secure_backup(),
            )
            .await
            {
                Ok(Ok(inspection)) => Ok(inspection),
                Ok(Err(error)) => Err(classify_secure_backup_gate_failure(&error)),
                Err(_) => Err(koushi_state::SecureBackupGateFailureKind::Timeout),
            };
            let _ = tx
                .send(AccountMessage::SecureBackupInspectionFinished {
                    generation,
                    started_at,
                    result,
                })
                .await;
        }));
    }

    pub(super) async fn finish_secure_backup_inspection(
        &mut self,
        generation: u64,
        started_at: Instant,
        result: Result<
            koushi_sdk::MatrixSecureBackupInspection,
            koushi_state::SecureBackupGateFailureKind,
        >,
    ) {
        self.secure_backup_inspection_task = None;
        if std::mem::take(&mut self.secure_backup_inspection_pending) {
            self.start_secure_backup_inspection();
            return;
        }
        let Some(mut action) = secure_backup_inspection_completion_action(
            self.trust_generation,
            self.session_promoted,
            self.secure_backup_ready,
            generation,
            result,
        ) else {
            if self.session_promoted {
                self.start_secure_backup_inspection();
            }
            return;
        };
        if self.recovery_key_delivery_pending
            && matches!(
                action,
                AppAction::SecureBackupGateChanged(koushi_state::SecureBackupGateState::Ready)
            )
        {
            action = AppAction::SecureBackupGateChanged(
                koushi_state::SecureBackupGateState::RecoveryKeyDeliveryRequired,
            );
        }
        if let AppAction::SecureBackupGateChanged(gate) = &action {
            let admitted = gate.backup_is_ready();
            self.set_secure_backup_send_admitted(admitted);
            let retrying = matches!(
                gate,
                koushi_state::SecureBackupGateState::DegradedRetrying { .. }
            );
            record(
                DiagnosticEvent::new(
                    DiagnosticLevel::Info,
                    "core.secure_backup",
                    "inspection_settled",
                )
                .field(DiagnosticField::token(
                    "gate",
                    secure_backup_gate_token(gate),
                ))
                .field(DiagnosticField::milliseconds(
                    "elapsed_ms",
                    started_at.elapsed().as_millis(),
                )),
            );
            if retrying {
                self.schedule_secure_backup_monitor(generation, true);
            } else if !matches!(
                gate,
                koushi_state::SecureBackupGateState::BlockedFailed { .. }
            ) {
                self.secure_backup_retry_attempt = 0;
                self.secure_backup_recovery_epoch = false;
                self.secure_backup_recovery_reset_consumed = false;
                self.schedule_secure_backup_monitor(generation, false);
            }
        }
        self.send_actions(vec![action]).await;
    }

    fn schedule_secure_backup_monitor(&mut self, generation: u64, retrying: bool) {
        self.retire_secure_backup_monitor();
        let monitor_serial = self.secure_backup_monitor_serial;
        let (delay, cadence, attempt) = if retrying {
            let attempt = self.secure_backup_retry_attempt;
            self.secure_backup_retry_attempt = self.secure_backup_retry_attempt.saturating_add(1);
            (
                secure_backup_retry_delay(attempt, monitor_serial),
                "retry_exponential",
                Some(attempt),
            )
        } else {
            (SECURE_BACKUP_MONITOR_INTERVAL, "periodic_60s", None)
        };
        record(
            DiagnosticEvent::new(
                DiagnosticLevel::Debug,
                "core.secure_backup",
                "monitor_scheduled",
            )
            .field(DiagnosticField::token("cadence", cadence))
            .field(DiagnosticField::milliseconds("delay_ms", delay.as_millis()))
            .field(DiagnosticField::count(
                "attempt",
                attempt.map(u64::from).unwrap_or(0),
            )),
        );
        let tx = self.self_tx.clone();
        self.secure_backup_monitor_task = Some(executor::spawn(async move {
            executor::sleep(delay).await;
            let _ = tx
                .send(AccountMessage::RetrySecureBackupInspection {
                    generation,
                    monitor_serial,
                })
                .await;
        }));
    }

    pub(super) async fn handle_sync_connectivity_changed(&mut self, proven: bool) {
        if self.sync_connectivity_proven == proven {
            return;
        }
        self.sync_connectivity_proven = proven;
        record(
            DiagnosticEvent::new(
                DiagnosticLevel::Info,
                "core.secure_backup",
                "connectivity_changed",
            )
            .field(DiagnosticField::boolean("proven", proven)),
        );
        let recovery_reset = apply_secure_backup_connectivity_edge(
            proven,
            &mut self.secure_backup_retry_attempt,
            &mut self.secure_backup_recovery_epoch,
            &mut self.secure_backup_recovery_reset_consumed,
        );
        if !proven {
            self.secure_backup_inspection_pending |= self.session_promoted
                || self.secure_backup_inspection_task.is_some()
                || self.secure_backup_monitor_task.is_some();
            if let Some(task) = self.secure_backup_inspection_task.take() {
                task.abort();
                let _ = task.await;
            }
            self.retire_secure_backup_monitor();
            return;
        }
        if self.session_promoted {
            self.secure_backup_inspection_pending = false;
            if !self.secure_backup_recovery_epoch || recovery_reset {
                self.start_secure_backup_inspection();
            } else {
                self.schedule_secure_backup_monitor(self.trust_generation, true);
            }
        }
    }

    fn retire_secure_backup_monitor(&mut self) {
        self.secure_backup_monitor_serial =
            self.secure_backup_monitor_serial.wrapping_add(1).max(1);
        if let Some(task) = self.secure_backup_monitor_task.take() {
            task.abort();
        }
    }

    pub(super) async fn cancel_secure_backup_inspection(&mut self) {
        self.secure_backup_inspection_pending = false;
        self.sync_connectivity_proven = false;
        self.secure_backup_retry_attempt = 0;
        self.secure_backup_recovery_epoch = false;
        self.secure_backup_recovery_reset_consumed = false;
        if let Some(task) = self.secure_backup_inspection_task.take() {
            task.abort();
            let _ = task.await;
        }
        self.secure_backup_monitor_serial =
            self.secure_backup_monitor_serial.wrapping_add(1).max(1);
        if let Some(task) = self.secure_backup_monitor_task.take() {
            task.abort();
            let _ = task.await;
        }
    }

    pub(super) fn start_secure_backup_observer(&mut self, session: Arc<MatrixClientSession>) {
        if let Some(task) = self.secure_backup_observer.take() {
            task.abort();
        }
        let generation = self.trust_generation;
        let mut observation = session.observe_secure_backup_state();
        let tx = self.self_tx.clone();
        self.secure_backup_observer = Some(executor::spawn(async move {
            while let Some(state) = observation.updates.next().await {
                if tx
                    .send(AccountMessage::SecureBackupStateChanged { generation, state })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    pub(super) async fn handle_secure_backup_state_changed(
        &mut self,
        generation: u64,
        state: koushi_sdk::MatrixSecureBackupState,
    ) {
        if generation != self.trust_generation || !self.session_promoted {
            return;
        }
        if state.backup != koushi_sdk::MatrixSecureBackupLocalState::Enabled
            || state.recovery != koushi_sdk::MatrixSecureBackupRecoveryState::Enabled
        {
            self.set_secure_backup_send_admitted(false);
        }
        self.send_actions(vec![AppAction::SecureBackupGateChanged(
            koushi_state::SecureBackupGateState::Checking,
        )])
        .await;
        self.start_secure_backup_inspection();
    }

    pub(super) async fn stop_secure_backup_observer(&mut self) {
        if let Some(task) = self.secure_backup_observer.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

#[cfg(test)]
mod tests;
