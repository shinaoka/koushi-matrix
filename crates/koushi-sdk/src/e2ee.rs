#[cfg(test)]
use crate::client_session::{
    PersistableMatrixSession, desktop_client_builder_defaults, restore_session,
};
use crate::room_projection::matrix_room;
use crate::{MatrixClientSession, MatrixRoomOperationError};
use futures_util::{Stream, StreamExt, stream};
use koushi_diagnostics::{
    DiagnosticCounterContext, DiagnosticEvent, DiagnosticField, DiagnosticLevel, record,
};
#[cfg(test)]
use koushi_state::SessionInfo;
use koushi_state::{
    AuthSecret, CrossSigningStatus, CurrentDeviceTrustState, CurrentSessionBackupState,
    DeviceCleanupAuthMode, DeviceCleanupFailureKind, DeviceCleanupRemoteOutcome, E2eeRecoveryState,
    IdentityResetAuthRequest, IdentityResetAuthType, KeyBackupStatus, OwnIdentityVerification,
    PendingKeyCountBucket, RecoveryRequest, SasEmoji, SecureBackupGateFailureKind,
    SecureBackupGateState, VerificationAccountKind, VerificationGateState,
    VerificationMethodCapability, VerificationTarget,
};
use matrix_sdk::ruma::{events::AnySyncTimelineEvent, serde::Raw};
use matrix_sdk_base::crypto::CollectStrategy;
use serde::{Deserialize, Serialize};
use std::{fmt, path::PathBuf, pin::Pin, sync::Arc};
use thiserror::Error;
use zeroize::Zeroizing;

pub type CurrentDeviceTrustStream = Pin<Box<dyn Stream<Item = CurrentDeviceTrustState> + Send>>;

pub struct CurrentDeviceTrustObservation {
    pub current: CurrentDeviceTrustState,
    pub updates: CurrentDeviceTrustStream,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixRoomKeyRotationReason {
    Initial,
    ExpiredTime,
    ExpiredMessageCount,
    MembershipOrDeviceChange,
    EncryptionSettingsChanged,
    ExplicitDiscard,
    FullMemberListReload,
    RoomSubscription,
    LimitedSyncResponse,
    KeyShareFailure,
    StoreMissing,
    Invalidated,
    Unknown,
}

pub type SecureBackupStateStream = Pin<Box<dyn Stream<Item = MatrixSecureBackupState> + Send>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MatrixSecureBackupState {
    pub backup: MatrixSecureBackupLocalState,
    pub recovery: MatrixSecureBackupRecoveryState,
}

pub struct MatrixSecureBackupStateObservation {
    pub current: MatrixSecureBackupState,
    pub updates: SecureBackupStateStream,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("current-device trust recheck failed")]
pub enum CurrentDeviceTrustRecheckError {
    Authentication,
    Network,
    Server,
    Sdk,
}

fn classify_current_device_trust_recheck_error(
    error: &matrix_sdk::Error,
) -> CurrentDeviceTrustRecheckError {
    match error {
        matrix_sdk::Error::AuthenticationRequired => CurrentDeviceTrustRecheckError::Authentication,
        matrix_sdk::Error::Timeout => CurrentDeviceTrustRecheckError::Network,
        matrix_sdk::Error::Http(http_error) => {
            classify_current_device_trust_recheck_http_error(http_error)
        }
        _ => CurrentDeviceTrustRecheckError::Sdk,
    }
}

fn classify_current_device_trust_recheck_http_error(
    error: &matrix_sdk::HttpError,
) -> CurrentDeviceTrustRecheckError {
    use matrix_sdk::ruma::api::error::ErrorKind;

    let authentication = matches!(
        error.client_api_error_kind(),
        Some(ErrorKind::UnknownToken(_)) | Some(ErrorKind::MissingToken)
    ) || error
        .as_client_api_error()
        .is_some_and(|error| matches!(error.status_code.as_u16(), 401 | 403));
    if authentication {
        return CurrentDeviceTrustRecheckError::Authentication;
    }

    match error {
        matrix_sdk::HttpError::Reqwest(_) => CurrentDeviceTrustRecheckError::Network,
        matrix_sdk::HttpError::Api(_) => CurrentDeviceTrustRecheckError::Server,
        matrix_sdk::HttpError::Cached(inner) => {
            classify_current_device_trust_recheck_http_error(inner)
        }
        _ => CurrentDeviceTrustRecheckError::Sdk,
    }
}

fn current_device_trust_recheck_failure_token(
    error: CurrentDeviceTrustRecheckError,
) -> &'static str {
    match error {
        CurrentDeviceTrustRecheckError::Authentication => "authentication",
        CurrentDeviceTrustRecheckError::Network => "network",
        CurrentDeviceTrustRecheckError::Server => "server",
        CurrentDeviceTrustRecheckError::Sdk => "sdk",
    }
}

fn record_current_device_trust_recheck_finished(
    outcome: &'static str,
    failure_kind: Option<CurrentDeviceTrustRecheckError>,
) {
    let mut event = DiagnosticEvent::new(
        DiagnosticLevel::Info,
        "sdk.current_device_trust_recheck",
        "finished",
    )
    .field(DiagnosticField::token("outcome", outcome));
    if let Some(error) = failure_kind {
        event = event.field(DiagnosticField::token(
            "failure_kind",
            current_device_trust_recheck_failure_token(error),
        ));
    }
    record(event);
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct MatrixCurrentSessionInspection {
    pub device_display_name: Option<String>,
    pub verification: CurrentDeviceTrustState,
    pub is_cross_signed_by_owner: bool,
    pub own_identity_verification: OwnIdentityVerification,
    pub key_backup: CurrentSessionBackupState,
}

impl std::fmt::Debug for MatrixCurrentSessionInspection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MatrixCurrentSessionInspection")
            .field(
                "device_display_name",
                &self.device_display_name.as_ref().map(|_| "DeviceName(..)"),
            )
            .field("verification", &self.verification)
            .field("is_cross_signed_by_owner", &self.is_cross_signed_by_owner)
            .field("own_identity_verification", &self.own_identity_verification)
            .field("key_backup", &self.key_backup)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
#[error("current-session inspection failed")]
pub enum MatrixCurrentSessionInspectionError {
    Unavailable,
    DeviceRequest,
    CurrentDeviceMissing,
    IdentityRequest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatrixSecureBackupServerState {
    Unknown,
    Absent,
    Present,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatrixSecureBackupLocalState {
    Unknown,
    Disabled,
    Creating,
    Enabling,
    Resuming,
    Downloading,
    Disabling,
    Enabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatrixSecureBackupRecoveryState {
    Unknown,
    Disabled,
    Incomplete,
    Enabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatrixSecureBackupUploadState {
    Unknown,
    Pending(PendingKeyCountBucket),
    Failed,
    Settled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatrixSecureBackupTrustState {
    Unknown,
    Mismatch,
    Trusted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MatrixSecureBackupInspection {
    pub server: MatrixSecureBackupServerState,
    pub local: MatrixSecureBackupLocalState,
    pub recovery: MatrixSecureBackupRecoveryState,
    pub upload: MatrixSecureBackupUploadState,
    pub trust: MatrixSecureBackupTrustState,
    pub recovery_key_delivery_pending: bool,
}

impl MatrixSecureBackupInspection {
    pub fn recommended_gate_state(&self) -> SecureBackupGateState {
        use MatrixSecureBackupLocalState as Local;
        use MatrixSecureBackupRecoveryState as Recovery;
        use MatrixSecureBackupServerState as Server;
        use MatrixSecureBackupTrustState as Trust;
        use MatrixSecureBackupUploadState as Upload;

        if self.server == Server::Unknown {
            return SecureBackupGateState::Checking;
        }

        if self.trust == Trust::Mismatch {
            return SecureBackupGateState::ExistingBackupNeedsRecovery {
                failure: Some(SecureBackupGateFailureKind::BackupKeyMismatch),
            };
        }

        if self.recovery == Recovery::Incomplete {
            return SecureBackupGateState::SecureStorageIncomplete;
        }

        match self.server {
            Server::Present => {
                if self.recovery_key_delivery_pending {
                    return SecureBackupGateState::RecoveryKeyDeliveryRequired;
                }
                match self.local {
                    Local::Unknown
                    | Local::Creating
                    | Local::Enabling
                    | Local::Resuming
                    | Local::Downloading
                    | Local::Disabling => return SecureBackupGateState::Checking,
                    Local::Disabled => {
                        return SecureBackupGateState::ExistingBackupNeedsRecovery {
                            failure: None,
                        };
                    }
                    Local::Enabled => {}
                }

                if matches!(self.recovery, Recovery::Unknown | Recovery::Disabled) {
                    return SecureBackupGateState::ExistingBackupNeedsRecovery { failure: None };
                }

                match self.upload {
                    Upload::Settled if self.trust == Trust::Trusted => SecureBackupGateState::Ready,
                    Upload::Pending(pending) => {
                        SecureBackupGateState::UploadingExistingKeys { pending }
                    }
                    Upload::Failed => SecureBackupGateState::DegradedRetrying {
                        failure: SecureBackupGateFailureKind::Network,
                    },
                    Upload::Unknown | Upload::Settled => SecureBackupGateState::Checking,
                }
            }
            Server::Absent => match self.recovery {
                Recovery::Disabled => SecureBackupGateState::ExplicitlyDisabledRequiresSetup,
                Recovery::Unknown | Recovery::Enabled => SecureBackupGateState::SetupRequired,
                Recovery::Incomplete => SecureBackupGateState::SecureStorageIncomplete,
            },
            Server::Unknown => SecureBackupGateState::Checking,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatrixDeviceNameOutcome {
    Present,
    Renamed,
    CurrentDeviceMissing,
    InspectionFailed,
    RenameFailed,
}

pub async fn ensure_device_display_name(
    session: &MatrixClientSession,
    display_name: &str,
) -> MatrixDeviceNameOutcome {
    let response = match session.client().devices().await {
        Ok(response) => response,
        Err(_) => return MatrixDeviceNameOutcome::InspectionFailed,
    };
    let Some(current_device) = response
        .devices
        .into_iter()
        .find(|device| device.device_id.as_str() == session.info.device_id)
    else {
        return MatrixDeviceNameOutcome::CurrentDeviceMissing;
    };
    if current_device
        .display_name
        .as_deref()
        .is_some_and(|name| !name.trim().is_empty())
    {
        return MatrixDeviceNameOutcome::Present;
    }

    match session
        .client()
        .rename_device(&current_device.device_id, display_name)
        .await
    {
        Ok(_) => MatrixDeviceNameOutcome::Renamed,
        Err(_) => MatrixDeviceNameOutcome::RenameFailed,
    }
}

fn classify_own_identity_verification(
    identity_present: bool,
    identity_verified: bool,
) -> OwnIdentityVerification {
    if !identity_present {
        OwnIdentityVerification::Missing
    } else if identity_verified {
        OwnIdentityVerification::Verified
    } else {
        OwnIdentityVerification::Unverified
    }
}

fn classify_current_session_backup(
    local_state: matrix_sdk::encryption::backups::BackupState,
    server_probe: Result<bool, ()>,
) -> CurrentSessionBackupState {
    use matrix_sdk::encryption::backups::BackupState;

    if local_state == BackupState::Enabled {
        CurrentSessionBackupState::Ready
    } else if server_probe.is_ok() {
        CurrentSessionBackupState::Disabled
    } else {
        CurrentSessionBackupState::Unknown
    }
}

fn map_secure_backup_local_state(
    state: matrix_sdk::encryption::backups::BackupState,
) -> MatrixSecureBackupLocalState {
    use matrix_sdk::encryption::backups::BackupState;

    match state {
        BackupState::Enabled => MatrixSecureBackupLocalState::Enabled,
        BackupState::Unknown => MatrixSecureBackupLocalState::Unknown,
        BackupState::Creating => MatrixSecureBackupLocalState::Creating,
        BackupState::Enabling => MatrixSecureBackupLocalState::Enabling,
        BackupState::Resuming => MatrixSecureBackupLocalState::Resuming,
        BackupState::Downloading => MatrixSecureBackupLocalState::Downloading,
        BackupState::Disabling => MatrixSecureBackupLocalState::Disabling,
    }
}

fn map_secure_backup_recovery_state(
    state: matrix_sdk::encryption::recovery::RecoveryState,
) -> MatrixSecureBackupRecoveryState {
    use matrix_sdk::encryption::recovery::RecoveryState;

    match state {
        RecoveryState::Unknown => MatrixSecureBackupRecoveryState::Unknown,
        RecoveryState::Disabled => MatrixSecureBackupRecoveryState::Disabled,
        RecoveryState::Incomplete => MatrixSecureBackupRecoveryState::Incomplete,
        RecoveryState::Enabled => MatrixSecureBackupRecoveryState::Enabled,
    }
}

fn classify_secure_backup_upload(
    counts: Result<matrix_sdk_base::crypto::store::types::RoomKeyCounts, ()>,
    upload_state: matrix_sdk::encryption::backups::UploadState,
) -> MatrixSecureBackupUploadState {
    if matches!(
        upload_state,
        matrix_sdk::encryption::backups::UploadState::Error
    ) {
        return MatrixSecureBackupUploadState::Failed;
    }
    let Ok(counts) = counts else {
        return MatrixSecureBackupUploadState::Failed;
    };
    let pending = counts.total.saturating_sub(counts.backed_up);
    if pending == 0 {
        return MatrixSecureBackupUploadState::Settled;
    }
    let bucket = match pending {
        1 => PendingKeyCountBucket::One,
        2..=10 => PendingKeyCountBucket::TwoToTen,
        11..=100 => PendingKeyCountBucket::ElevenToOneHundred,
        _ => PendingKeyCountBucket::OverOneHundred,
    };
    MatrixSecureBackupUploadState::Pending(bucket)
}

enum SecureBackupStateUpdate {
    Backup(MatrixSecureBackupLocalState),
    Recovery(MatrixSecureBackupRecoveryState),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IdentityFact {
    Existing,
    Missing,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecoveryFact {
    Available,
    Unavailable,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct VerificationMethodFacts {
    identity: IdentityFact,
    verified_other_device_count: u64,
    recovery: RecoveryFact,
}

fn map_sdk_verification_state(
    state: matrix_sdk::encryption::VerificationState,
) -> CurrentDeviceTrustState {
    match state {
        matrix_sdk::encryption::VerificationState::Unknown => CurrentDeviceTrustState::Unknown,
        matrix_sdk::encryption::VerificationState::Verified => CurrentDeviceTrustState::Verified,
        matrix_sdk::encryption::VerificationState::Unverified => {
            CurrentDeviceTrustState::Unverified
        }
    }
}

fn current_device_trust_state_token(state: CurrentDeviceTrustState) -> &'static str {
    match state {
        CurrentDeviceTrustState::Unknown => "unknown",
        CurrentDeviceTrustState::Verified => "verified",
        CurrentDeviceTrustState::Unverified => "unverified",
    }
}

fn map_verification_method_facts(facts: VerificationMethodFacts) -> VerificationGateState {
    let (account_kind, methods) = match facts.identity {
        IdentityFact::Unknown => (VerificationAccountKind::Unknown, Vec::new()),
        IdentityFact::Missing => (
            VerificationAccountKind::NewIdentity,
            vec![VerificationMethodCapability::Bootstrap],
        ),
        IdentityFact::Existing
            if matches!(facts.recovery, RecoveryFact::Unknown)
                && facts.verified_other_device_count == 0 =>
        {
            (VerificationAccountKind::Unknown, Vec::new())
        }
        IdentityFact::Existing => {
            let mut methods = Vec::new();
            if facts.verified_other_device_count > 0 {
                methods.push(VerificationMethodCapability::ExistingDeviceSas);
            }
            if matches!(facts.recovery, RecoveryFact::Available) {
                methods.push(VerificationMethodCapability::RecoveryKey);
                methods.push(VerificationMethodCapability::SecurityPhrase);
            }
            (VerificationAccountKind::ExistingIdentity, methods)
        }
    };
    VerificationGateState {
        methods,
        account_kind,
        failure: None,
    }
}

fn is_eligible_own_user_proof_device(
    current_device_id: &str,
    candidate_device_id: &str,
    cross_signed_by_owner: bool,
    blocked: bool,
) -> bool {
    is_own_user_verification_recipient(
        current_device_id,
        candidate_device_id,
        cross_signed_by_owner,
    ) && !blocked
}

fn is_own_user_verification_recipient(
    current_device_id: &str,
    candidate_device_id: &str,
    cross_signed_by_owner: bool,
) -> bool {
    candidate_device_id != current_device_id && cross_signed_by_owner
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OwnUserSasDeviceFact {
    is_current: bool,
    cross_signed_by_owner: bool,
    blocked: bool,
    dehydrated: bool,
    curve_key_present: bool,
    ed25519_key_present: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct OwnUserSasRecipientDiagnostics {
    other_device_count: u64,
    recipient_count: u64,
    eligible_device_count: u64,
    sender_device_query_visible: bool,
    sender_curve_key_present: bool,
    sender_ed25519_key_present: bool,
    interactive_recipient_count: u64,
    dehydrated_recipient_count: u64,
}

fn own_user_sas_recipient_diagnostics(
    devices: impl IntoIterator<Item = OwnUserSasDeviceFact>,
) -> OwnUserSasRecipientDiagnostics {
    let mut diagnostics = OwnUserSasRecipientDiagnostics::default();
    for device in devices {
        if device.is_current {
            diagnostics.sender_device_query_visible = true;
            diagnostics.sender_curve_key_present |= device.curve_key_present;
            diagnostics.sender_ed25519_key_present |= device.ed25519_key_present;
            continue;
        }

        diagnostics.other_device_count += 1;
        if !device.cross_signed_by_owner {
            continue;
        }

        diagnostics.recipient_count += 1;
        if !device.blocked {
            diagnostics.eligible_device_count += 1;
        }
        if device.dehydrated {
            diagnostics.dehydrated_recipient_count += 1;
        } else if device.curve_key_present && device.ed25519_key_present {
            diagnostics.interactive_recipient_count += 1;
        }
    }
    diagnostics
}

fn sas_delivery_event(stage: &'static str, flow_id: u64) -> DiagnosticEvent {
    DiagnosticEvent::new(DiagnosticLevel::Info, "core.sas_verification", stage)
        .field(DiagnosticField::count("flow_id", flow_id))
}

fn sas_delivery_waiting_event(flow_id: u64, waiting_for: &'static str) -> DiagnosticEvent {
    sas_delivery_event("waiting", flow_id).field(DiagnosticField::token("waiting_for", waiting_for))
}

fn sas_recipients_resolved_event(
    flow_id: u64,
    diagnostics: OwnUserSasRecipientDiagnostics,
) -> DiagnosticEvent {
    sas_delivery_event("recipients_resolved", flow_id)
        .field(DiagnosticField::count(
            "other_device_count",
            diagnostics.other_device_count,
        ))
        .field(DiagnosticField::count(
            "recipient_count",
            diagnostics.recipient_count,
        ))
        .field(DiagnosticField::count(
            "eligible_device_count",
            diagnostics.eligible_device_count,
        ))
        .field(DiagnosticField::boolean(
            "sender_device_query_visible",
            diagnostics.sender_device_query_visible,
        ))
        .field(DiagnosticField::boolean(
            "sender_curve_key_present",
            diagnostics.sender_curve_key_present,
        ))
        .field(DiagnosticField::boolean(
            "sender_ed25519_key_present",
            diagnostics.sender_ed25519_key_present,
        ))
        .field(DiagnosticField::count(
            "interactive_recipient_count",
            diagnostics.interactive_recipient_count,
        ))
        .field(DiagnosticField::count(
            "dehydrated_recipient_count",
            diagnostics.dehydrated_recipient_count,
        ))
}

fn record_sas_delivery_event(event: DiagnosticEvent) {
    koushi_diagnostics::record_and_stderr(event);
}

fn recovery_verification_event(stage: &'static str) -> DiagnosticEvent {
    DiagnosticEvent::new(DiagnosticLevel::Info, "sdk.recovery_verification", stage)
        .field(DiagnosticField::token("flow_type", "recovery_key"))
}

fn record_recovery_verification_event(event: DiagnosticEvent) {
    koushi_diagnostics::record_and_stderr(event);
}

pub(super) fn has_stale_authoritative_device_signature(
    inspection: &matrix_sdk::encryption::recovery::RecoveryDeviceSignatureInspection,
) -> bool {
    inspection.authoritative_self_signing_signature_present
        && inspection.authoritative_self_signing_signature_parseable
        && !inspection.authoritative_self_signing_signature_valid
        && inspection.cached_self_signing_key_matches_authoritative
        && inspection.cached_signed_content_matches_authoritative
}

fn secret_storage_error_kind(
    error: &matrix_sdk::encryption::secret_storage::SecretStorageError,
) -> &'static str {
    use matrix_sdk::encryption::{
        identities::ManualVerifyError,
        secret_storage::{ImportError, SecretStorageError},
    };

    match error {
        SecretStorageError::Sdk(_) => "sdk",
        SecretStorageError::Json(_) => "json",
        SecretStorageError::SecretStorageKey(_) => "secret_storage_key",
        SecretStorageError::MissingKeyInfo { .. } => "missing_key_info",
        SecretStorageError::ImportError { error, .. } => match error {
            ImportError::Sdk(_) => "import_sdk",
            ImportError::Json(_) => "import_json",
            ImportError::Key(_) => "import_key",
            ImportError::MismatchedPublicKeys => "import_mismatched_public_keys",
            ImportError::Decryption(_) => "import_decryption",
        },
        SecretStorageError::Storage(_) => "storage",
        SecretStorageError::Verification(error) => match error {
            ManualVerifyError::Http(_) => "verification_http",
            ManualVerifyError::Signature(_) => "verification_signature",
            ManualVerifyError::SignatureUploadFailures { .. } => "signature_upload_failures",
        },
        SecretStorageError::Decryption(_) => "decryption",
        SecretStorageError::InconsistentBackupDecryptionKey => "inconsistent_backup_decryption_key",
        SecretStorageError::MissingOrInvalidBackupDecryptionKey => {
            "missing_or_invalid_backup_decryption_key"
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SignatureUploadFailureDiagnostics {
    signed_target_count: usize,
    signed_key_count: usize,
    failure_user_count: usize,
    failure_key_count: usize,
    invalid_signature_count: usize,
    other_failure_count: usize,
    unknown_failure_count: usize,
}

fn secret_storage_signature_upload_failure_diagnostics(
    error: &matrix_sdk::encryption::secret_storage::SecretStorageError,
) -> Option<SignatureUploadFailureDiagnostics> {
    match error {
        matrix_sdk::encryption::secret_storage::SecretStorageError::Verification(
            matrix_sdk::encryption::identities::ManualVerifyError::SignatureUploadFailures {
                signed_target_count,
                signed_key_count,
                failure_user_count,
                failure_key_count,
                invalid_signature_count,
                other_failure_count,
                unknown_failure_count,
            },
        ) => Some(SignatureUploadFailureDiagnostics {
            signed_target_count: *signed_target_count,
            signed_key_count: *signed_key_count,
            failure_user_count: *failure_user_count,
            failure_key_count: *failure_key_count,
            invalid_signature_count: *invalid_signature_count,
            other_failure_count: *other_failure_count,
            unknown_failure_count: *unknown_failure_count,
        }),
        _ => None,
    }
}

fn recovery_error_kind(error: &matrix_sdk::encryption::recovery::RecoveryError) -> &'static str {
    match error {
        matrix_sdk::encryption::recovery::RecoveryError::BackupExistsOnServer => {
            "backup_exists_on_server"
        }
        matrix_sdk::encryption::recovery::RecoveryError::Sdk(_) => "sdk",
        matrix_sdk::encryption::recovery::RecoveryError::SecretStorage(error) => {
            secret_storage_error_kind(error)
        }
    }
}

fn recovery_signature_upload_failure_diagnostics(
    error: &matrix_sdk::encryption::recovery::RecoveryError,
) -> Option<SignatureUploadFailureDiagnostics> {
    match error {
        matrix_sdk::encryption::recovery::RecoveryError::SecretStorage(error) => {
            secret_storage_signature_upload_failure_diagnostics(error)
        }
        _ => None,
    }
}

fn with_signature_upload_failure_diagnostics(
    event: DiagnosticEvent,
    diagnostics: SignatureUploadFailureDiagnostics,
) -> DiagnosticEvent {
    event
        .field(DiagnosticField::count(
            "signed_target_count",
            diagnostics.signed_target_count as u64,
        ))
        .field(DiagnosticField::count(
            "signed_key_count",
            diagnostics.signed_key_count as u64,
        ))
        .field(DiagnosticField::count(
            "failure_user_count",
            diagnostics.failure_user_count as u64,
        ))
        .field(DiagnosticField::count(
            "failure_key_count",
            diagnostics.failure_key_count as u64,
        ))
        .field(DiagnosticField::count(
            "invalid_signature_count",
            diagnostics.invalid_signature_count as u64,
        ))
        .field(DiagnosticField::count(
            "other_failure_count",
            diagnostics.other_failure_count as u64,
        ))
        .field(DiagnosticField::count(
            "unknown_failure_count",
            diagnostics.unknown_failure_count as u64,
        ))
}

async fn record_recovery_cross_signing_status(
    encryption: &matrix_sdk::encryption::Encryption,
    stage: &'static str,
) {
    match encryption.cross_signing_status().await {
        Some(status) => record_recovery_verification_event(
            recovery_verification_event(stage)
                .field(DiagnosticField::token("outcome", "found"))
                .field(DiagnosticField::boolean("has_master", status.has_master))
                .field(DiagnosticField::boolean(
                    "has_self_signing",
                    status.has_self_signing,
                ))
                .field(DiagnosticField::boolean(
                    "has_user_signing",
                    status.has_user_signing,
                ))
                .field(DiagnosticField::boolean("complete", status.is_complete())),
        ),
        None => record_recovery_verification_event(
            recovery_verification_event(stage).field(DiagnosticField::token("outcome", "missing")),
        ),
    }
}

pub type E2eeRecoveryStateStream = Pin<Box<dyn Stream<Item = E2eeRecoveryState> + Send>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MatrixCrossSigningStatus {
    pub has_master: bool,
    pub has_self_signing: bool,
    pub has_user_signing: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixIdentityResetAuthType {
    Uiaa,
    OAuth,
    Unknown,
}

pub struct MatrixIdentityResetHandle {
    inner: matrix_sdk::encryption::recovery::IdentityResetHandle,
    auth_type: MatrixIdentityResetAuthType,
}

impl fmt::Debug for MatrixIdentityResetHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixIdentityResetHandle")
            .field("auth_type", &self.auth_type)
            .finish_non_exhaustive()
    }
}

impl MatrixIdentityResetHandle {
    pub fn auth_type(&self) -> MatrixIdentityResetAuthType {
        self.auth_type
    }

    pub fn desktop_auth_type(&self) -> IdentityResetAuthType {
        map_identity_reset_auth_type_to_desktop(self.auth_type)
    }

    pub async fn cancel(&self) {
        self.inner.cancel().await;
    }

    pub async fn reset(
        &self,
        session: &MatrixClientSession,
        request: &IdentityResetAuthRequest,
    ) -> Result<(), E2eeTrustError> {
        let auth = match request {
            IdentityResetAuthRequest::OAuthApproved => None,
            IdentityResetAuthRequest::UiaaPassword { password } => {
                let matrix_sdk::encryption::CrossSigningResetAuthType::Uiaa(uiaa) =
                    self.inner.auth_type()
                else {
                    return Err(E2eeTrustError::Sdk(
                        "identity reset auth type mismatch".to_owned(),
                    ));
                };
                let identifier = matrix_sdk::ruma::api::client::uiaa::UserIdentifier::Matrix(
                    matrix_sdk::ruma::api::client::uiaa::MatrixUserIdentifier::new(
                        session.info.user_id.clone(),
                    ),
                );
                let mut password_auth = matrix_sdk::ruma::api::client::uiaa::Password::new(
                    identifier,
                    password.expose_secret().to_owned(),
                );
                password_auth.session.clone_from(&uiaa.session);
                Some(matrix_sdk::ruma::api::client::uiaa::AuthData::Password(
                    password_auth,
                ))
            }
        };

        self.inner.reset(auth).await?;
        Ok(())
    }
}

pub enum IdentityResetOutcome {
    Completed,
    AuthRequired(MatrixIdentityResetHandle),
}

impl fmt::Debug for IdentityResetOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Completed => formatter.write_str("Completed"),
            Self::AuthRequired(handle) => formatter
                .debug_tuple("AuthRequired")
                .field(&handle.auth_type())
                .finish(),
        }
    }
}

#[derive(Clone)]
pub struct MatrixVerificationRequestHandle {
    inner: matrix_sdk::encryption::verification::VerificationRequest,
}

impl fmt::Debug for MatrixVerificationRequestHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixVerificationRequestHandle")
            .field("flow_id", &"FlowId(..)")
            .finish_non_exhaustive()
    }
}

impl MatrixVerificationRequestHandle {
    pub fn flow_id(&self) -> &str {
        self.inner.flow_id()
    }

    pub fn state(&self) -> MatrixVerificationRequestState {
        map_sdk_verification_request_state(self.inner.state())
    }

    pub fn changes(&self) -> MatrixVerificationRequestStateStream {
        Box::pin(self.inner.changes().map(map_sdk_verification_request_state))
    }
}

#[derive(Clone)]
pub struct MatrixOwnUserVerificationHandle {
    request: MatrixVerificationRequestHandle,
    eligible_device_count: u64,
}

impl fmt::Debug for MatrixOwnUserVerificationHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixOwnUserVerificationHandle")
            .field("eligible_device_count", &self.eligible_device_count)
            .finish_non_exhaustive()
    }
}

impl MatrixOwnUserVerificationHandle {
    pub fn eligible_device_count(&self) -> u64 {
        self.eligible_device_count
    }

    pub fn state(&self) -> MatrixVerificationRequestState {
        self.request.state()
    }

    pub fn changes(&self) -> MatrixVerificationRequestStateStream {
        self.request.changes()
    }
}

#[derive(Clone)]
pub struct MatrixIncomingVerificationRequest {
    target: VerificationTarget,
    handle: MatrixVerificationRequestHandle,
}

impl MatrixIncomingVerificationRequest {
    pub fn target(&self) -> &VerificationTarget {
        &self.target
    }

    pub fn handle(&self) -> &MatrixVerificationRequestHandle {
        &self.handle
    }

    pub fn into_parts(self) -> (VerificationTarget, MatrixVerificationRequestHandle) {
        (self.target, self.handle)
    }
}

impl fmt::Debug for MatrixIncomingVerificationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixIncomingVerificationRequest")
            .field("target", &"VerificationTarget(..)")
            .field("handle", &self.handle)
            .finish()
    }
}

pub struct MatrixIncomingVerificationRequestObserver {
    client: matrix_sdk::Client,
    receiver: Option<tokio::sync::mpsc::Receiver<MatrixIncomingVerificationRequest>>,
    handlers: Vec<matrix_sdk::event_handler::EventHandlerHandle>,
    incoming_request_task: Option<tokio::task::JoinHandle<()>>,
}

impl MatrixIncomingVerificationRequestObserver {
    pub async fn recv(&mut self) -> Option<MatrixIncomingVerificationRequest> {
        self.receiver.as_mut()?.recv().await
    }

    pub fn take_receiver(
        &mut self,
    ) -> Option<tokio::sync::mpsc::Receiver<MatrixIncomingVerificationRequest>> {
        self.receiver.take()
    }

    /// Stop the typed delivery owner and retain its JoinHandle until abort settles.
    pub async fn shutdown(&mut self) {
        if let Some(task) = self.incoming_request_task.as_mut() {
            task.abort();
            let _ = task.await;
        }
        self.incoming_request_task = None;
        self.remove_handlers();
    }

    fn remove_handlers(&mut self) {
        for handler in self.handlers.drain(..) {
            self.client.remove_event_handler(handler);
        }
    }
}

impl fmt::Debug for MatrixIncomingVerificationRequestObserver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixIncomingVerificationRequestObserver")
            .field("pending", &"Receiver(..)")
            .finish_non_exhaustive()
    }
}

impl Drop for MatrixIncomingVerificationRequestObserver {
    fn drop(&mut self) {
        if let Some(task) = self.incoming_request_task.take() {
            task.abort();
        }
        self.remove_handlers();
    }
}

#[derive(Clone)]
pub struct MatrixSasVerificationHandle {
    inner: matrix_sdk::encryption::verification::SasVerification,
}

impl fmt::Debug for MatrixSasVerificationHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixSasVerificationHandle")
            .field("flow_id", &"FlowId(..)")
            .finish_non_exhaustive()
    }
}

impl MatrixSasVerificationHandle {
    pub fn state(&self) -> MatrixSasState {
        map_sdk_sas_state(self.inner.state())
    }

    pub fn changes(&self) -> MatrixSasStateStream {
        Box::pin(self.inner.changes().map(map_sdk_sas_state))
    }

    pub fn emojis(&self) -> Option<Vec<SasEmoji>> {
        self.inner.emoji().map(map_sdk_sas_emojis_to_desktop)
    }
}

pub type MatrixVerificationRequestStateStream =
    Pin<Box<dyn Stream<Item = MatrixVerificationRequestState> + Send>>;

pub type MatrixSasStateStream = Pin<Box<dyn Stream<Item = MatrixSasState> + Send>>;

#[derive(Clone, Debug)]
pub enum MatrixVerificationRequestState {
    Created,
    Requested,
    Ready,
    SasStarted(MatrixSasVerificationHandle),
    Done,
    Cancelled {
        kind: MatrixVerificationCancelKind,
        cancelled_by_us: bool,
    },
    UnsupportedMethod,
}

fn verification_request_state_token(state: &MatrixVerificationRequestState) -> &'static str {
    match state {
        MatrixVerificationRequestState::Created => "created",
        MatrixVerificationRequestState::Requested => "requested",
        MatrixVerificationRequestState::Ready => "ready",
        MatrixVerificationRequestState::SasStarted(_) => "sas_started",
        MatrixVerificationRequestState::Done => "done",
        MatrixVerificationRequestState::Cancelled { .. } => "cancelled",
        MatrixVerificationRequestState::UnsupportedMethod => "unsupported_method",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixVerificationCancelKind {
    UnknownMethod,
    KeyMismatch,
    User,
    Timeout,
    AcceptedElsewhere,
    Other,
}

fn map_verification_cancel_kind(code: &str) -> MatrixVerificationCancelKind {
    match code {
        "m.unknown_method" => MatrixVerificationCancelKind::UnknownMethod,
        "m.key_mismatch" => MatrixVerificationCancelKind::KeyMismatch,
        "m.user" => MatrixVerificationCancelKind::User,
        "m.timeout" => MatrixVerificationCancelKind::Timeout,
        "m.accepted" => MatrixVerificationCancelKind::AcceptedElsewhere,
        _ => MatrixVerificationCancelKind::Other,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MatrixSasState {
    Created,
    Started,
    Accepted,
    SasPresented {
        emojis: Vec<SasEmoji>,
    },
    Confirmed,
    Done,
    Cancelled {
        kind: MatrixVerificationCancelKind,
        cancelled_by_us: bool,
    },
    UnsupportedShortAuth,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyBackupRestoreScope {
    JoinedRooms,
}

#[derive(Clone, Eq, PartialEq)]
pub struct KeyBackupRestoreSummary {
    pub scope: KeyBackupRestoreScope,
    pub version: Option<String>,
    pub restored_rooms: u64,
    pub total_rooms: Option<u64>,
}

impl fmt::Debug for KeyBackupRestoreSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KeyBackupRestoreSummary")
            .field("scope", &self.scope)
            .field(
                "version",
                &self.version.as_ref().map(|_| "BackupVersion(..)"),
            )
            .field("restored_rooms", &self.restored_rooms)
            .field("total_rooms", &self.total_rooms)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoomKeyExportSummary {
    pub exported_sessions: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoomKeyImportSummary {
    pub imported_count: u64,
    pub total_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SecureBackupSetupSummary {
    pub recovery_key_written: bool,
}

#[derive(Clone, Eq, Error, PartialEq)]
pub enum E2eeTrustError {
    #[error("Matrix encryption is not initialized")]
    NoOlmMachine,
    #[error("secure backup inspection is inconclusive")]
    SecureBackupInspectionInconclusive,
    #[error("a secure backup already exists on the server")]
    SecureBackupAlreadyExists,
    #[error("explicit secure-backup re-enable confirmation is required")]
    SecureBackupReenableConfirmationRequired,
    #[error("secure backup upload did not settle")]
    SecureBackupUploadFailed,
    #[error("secure backup recovery key delivery failed")]
    SecureBackupRecoveryKeyDeliveryFailed,
    #[error("Matrix encryption operation failed")]
    Classified(E2eeTrustFailureKind),
    #[error("Matrix SDK trust operation failed")]
    Sdk(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum E2eeTrustFailureKind {
    Network,
    Forbidden,
    InvalidBackup,
    Timeout,
    Sdk,
}

#[derive(Clone, Eq, PartialEq)]
pub enum MatrixDeviceCleanupOutcome {
    Settled(DeviceCleanupRemoteOutcome),
    UiaaRequired { session: Option<String> },
}

impl fmt::Debug for MatrixDeviceCleanupOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Settled(outcome) => formatter.debug_tuple("Settled").field(outcome).finish(),
            Self::UiaaRequired { session } => formatter
                .debug_struct("UiaaRequired")
                .field("session", &session.as_ref().map(|_| "SessionId(..)"))
                .finish(),
        }
    }
}

impl fmt::Debug for E2eeTrustError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoOlmMachine => formatter.write_str("NoOlmMachine"),
            Self::SecureBackupInspectionInconclusive => {
                formatter.write_str("SecureBackupInspectionInconclusive")
            }
            Self::SecureBackupAlreadyExists => formatter.write_str("SecureBackupAlreadyExists"),
            Self::SecureBackupReenableConfirmationRequired => {
                formatter.write_str("SecureBackupReenableConfirmationRequired")
            }
            Self::SecureBackupUploadFailed => formatter.write_str("SecureBackupUploadFailed"),
            Self::SecureBackupRecoveryKeyDeliveryFailed => {
                formatter.write_str("SecureBackupRecoveryKeyDeliveryFailed")
            }
            Self::Classified(kind) => formatter.debug_tuple("Classified").field(kind).finish(),
            Self::Sdk(_) => formatter.write_str("Sdk(..)"),
        }
    }
}

impl From<matrix_sdk::Error> for E2eeTrustError {
    fn from(error: matrix_sdk::Error) -> Self {
        match error {
            matrix_sdk::Error::NoOlmMachine => Self::NoOlmMachine,
            other => Self::Sdk(other.to_string()),
        }
    }
}

fn e2ee_trust_failure_kind(error: &matrix_sdk::Error) -> E2eeTrustFailureKind {
    match error {
        matrix_sdk::Error::Http(error)
            if error
                .as_client_api_error()
                .is_some_and(|error| error.status_code.as_u16() == 403)
                || matches!(
                    error.client_api_error_kind(),
                    Some(matrix_sdk::ruma::api::error::ErrorKind::Forbidden)
                ) =>
        {
            E2eeTrustFailureKind::Forbidden
        }
        matrix_sdk::Error::Http(_)
        | matrix_sdk::Error::Io(_)
        | matrix_sdk::Error::ConcurrentRequestFailed => E2eeTrustFailureKind::Network,
        matrix_sdk::Error::Timeout => E2eeTrustFailureKind::Timeout,
        matrix_sdk::Error::BackupNotEnabled | matrix_sdk::Error::SecureBackupRequired => {
            E2eeTrustFailureKind::InvalidBackup
        }
        _ => E2eeTrustFailureKind::Sdk,
    }
}

impl From<matrix_sdk::encryption::recovery::RecoveryError> for E2eeTrustError {
    fn from(error: matrix_sdk::encryption::recovery::RecoveryError) -> Self {
        Self::Sdk(error.to_string())
    }
}

pub fn map_cross_signing_status_to_desktop(
    status: Option<MatrixCrossSigningStatus>,
) -> CrossSigningStatus {
    match status {
        None => CrossSigningStatus::Missing,
        Some(status) if status.has_master && status.has_self_signing && status.has_user_signing => {
            CrossSigningStatus::Trusted
        }
        Some(_) => CrossSigningStatus::NotTrusted,
    }
}

pub fn map_identity_reset_auth_type_to_desktop(
    auth_type: MatrixIdentityResetAuthType,
) -> IdentityResetAuthType {
    match auth_type {
        MatrixIdentityResetAuthType::Uiaa => IdentityResetAuthType::Uiaa,
        MatrixIdentityResetAuthType::OAuth => IdentityResetAuthType::OAuth,
        MatrixIdentityResetAuthType::Unknown => IdentityResetAuthType::Unknown,
    }
}

fn map_sdk_identity_reset_auth_type(
    auth_type: &matrix_sdk::encryption::CrossSigningResetAuthType,
) -> MatrixIdentityResetAuthType {
    match auth_type {
        matrix_sdk::encryption::CrossSigningResetAuthType::Uiaa(_) => {
            MatrixIdentityResetAuthType::Uiaa
        }
        matrix_sdk::encryption::CrossSigningResetAuthType::OAuth(_) => {
            MatrixIdentityResetAuthType::OAuth
        }
    }
}

fn map_sdk_verification_request_state(
    state: matrix_sdk::encryption::verification::VerificationRequestState,
) -> MatrixVerificationRequestState {
    use matrix_sdk::encryption::verification::{
        Verification, VerificationRequestState as SdkVerificationRequestState,
    };

    match state {
        SdkVerificationRequestState::Created { .. } => MatrixVerificationRequestState::Created,
        SdkVerificationRequestState::Requested { .. } => MatrixVerificationRequestState::Requested,
        SdkVerificationRequestState::Ready { .. } => MatrixVerificationRequestState::Ready,
        SdkVerificationRequestState::Transitioned { verification } => match verification {
            Verification::SasV1(inner) => {
                MatrixVerificationRequestState::SasStarted(MatrixSasVerificationHandle { inner })
            }
            #[allow(unreachable_patterns)]
            _ => MatrixVerificationRequestState::UnsupportedMethod,
        },
        SdkVerificationRequestState::Done => MatrixVerificationRequestState::Done,
        SdkVerificationRequestState::Cancelled(info) => MatrixVerificationRequestState::Cancelled {
            kind: map_verification_cancel_kind(info.cancel_code().as_str()),
            cancelled_by_us: info.cancelled_by_us(),
        },
    }
}

fn map_sdk_sas_state(state: matrix_sdk::encryption::verification::SasState) -> MatrixSasState {
    use matrix_sdk::encryption::verification::SasState as SdkSasState;

    match state {
        SdkSasState::Created { .. } => MatrixSasState::Created,
        SdkSasState::Started { .. } => MatrixSasState::Started,
        SdkSasState::Accepted { .. } => MatrixSasState::Accepted,
        SdkSasState::KeysExchanged { emojis, .. } => match emojis {
            Some(emojis) => MatrixSasState::SasPresented {
                emojis: map_sdk_sas_emojis_to_desktop(emojis.emojis),
            },
            None => MatrixSasState::UnsupportedShortAuth,
        },
        SdkSasState::Confirmed => MatrixSasState::Confirmed,
        SdkSasState::Done { .. } => MatrixSasState::Done,
        SdkSasState::Cancelled(info) => {
            map_sas_cancellation(info.cancel_code().as_str(), info.cancelled_by_us())
        }
    }
}

fn map_sas_cancellation(code: &str, cancelled_by_us: bool) -> MatrixSasState {
    MatrixSasState::Cancelled {
        kind: map_verification_cancel_kind(code),
        cancelled_by_us,
    }
}

pub fn map_sdk_sas_emojis_to_desktop(
    emojis: [matrix_sdk::encryption::verification::Emoji; 7],
) -> Vec<SasEmoji> {
    emojis
        .into_iter()
        .map(|emoji| SasEmoji {
            symbol: emoji.symbol.to_owned(),
            description: emoji.description.to_owned(),
        })
        .collect()
}

pub async fn cross_signing_status(
    session: &MatrixClientSession,
) -> Result<CrossSigningStatus, E2eeTrustError> {
    let status = session
        .client()
        .encryption()
        .cross_signing_status()
        .await
        .map(|status| MatrixCrossSigningStatus {
            has_master: status.has_master,
            has_self_signing: status.has_self_signing,
            has_user_signing: status.has_user_signing,
        });
    Ok(map_cross_signing_status_to_desktop(status))
}

pub async fn bootstrap_cross_signing(
    session: &MatrixClientSession,
    auth_secret: Option<&AuthSecret>,
) -> Result<CrossSigningStatus, E2eeTrustError> {
    let encryption = session.client().encryption();
    match encryption.bootstrap_cross_signing(None).await {
        Ok(()) => {}
        Err(error) => {
            let Some(auth_secret) = auth_secret else {
                return Err(error.into());
            };
            let Some(uiaa_session) = error
                .as_uiaa_response()
                .and_then(|response| response.session.clone())
            else {
                return Err(error.into());
            };
            let identifier = matrix_sdk::ruma::api::client::uiaa::UserIdentifier::Matrix(
                matrix_sdk::ruma::api::client::uiaa::MatrixUserIdentifier::new(
                    session.info.user_id.clone(),
                ),
            );
            let mut password_auth = matrix_sdk::ruma::api::client::uiaa::Password::new(
                identifier,
                auth_secret.expose_secret().to_owned(),
            );
            password_auth.session = Some(uiaa_session);
            encryption
                .bootstrap_cross_signing(Some(
                    matrix_sdk::ruma::api::client::uiaa::AuthData::Password(password_auth),
                ))
                .await?;
        }
    }
    cross_signing_status(session).await
}

pub async fn enable_key_backup(
    session: &MatrixClientSession,
    passphrase: Option<&AuthSecret>,
) -> Result<KeyBackupStatus, E2eeTrustError> {
    let encryption = session.client().encryption();
    if let Some(passphrase) = passphrase {
        let _recovery_key = encryption
            .recovery()
            .enable()
            .wait_for_backups_to_upload()
            .with_passphrase(passphrase.expose_secret())
            .await?;
    } else {
        encryption.recovery().enable_backup().await?;
    }
    Ok(map_backup_state_to_desktop(encryption.backups().state()))
}

pub async fn restore_key_backup(
    session: &MatrixClientSession,
    request: &RecoveryRequest,
    version: Option<&str>,
) -> Result<KeyBackupRestoreSummary, E2eeTrustError> {
    let encryption = session.client().encryption();
    encryption
        .recovery()
        .recover(request.secret.expose_secret())
        .await?;

    download_joined_room_keys_from_backup(session, version).await
}

pub async fn download_joined_room_keys_from_backup(
    session: &MatrixClientSession,
    version: Option<&str>,
) -> Result<KeyBackupRestoreSummary, E2eeTrustError> {
    let encryption = session.client().encryption();
    let backup_state = encryption.backups().state();
    if !matches!(
        backup_state,
        matrix_sdk::encryption::backups::BackupState::Enabled
            | matrix_sdk::encryption::backups::BackupState::Downloading
    ) {
        return Err(E2eeTrustError::Sdk(
            "key backup unavailable after recovery".to_owned(),
        ));
    }

    let rooms = session.client().joined_rooms();
    let total_rooms = rooms.len() as u64;
    let mut restored_rooms = 0;
    for room in rooms {
        encryption
            .backups()
            .download_room_keys_for_room(room.room_id())
            .await?;
        restored_rooms += 1;
    }

    let backup_status = map_backup_state_to_desktop(encryption.backups().state());
    let backup_version = version.map(str::to_owned).or_else(|| match backup_status {
        KeyBackupStatus::Enabled { version } => Some(version),
        KeyBackupStatus::Restoring { version, .. } => version,
        _ => None,
    });

    Ok(KeyBackupRestoreSummary {
        scope: KeyBackupRestoreScope::JoinedRooms,
        version: backup_version,
        restored_rooms,
        total_rooms: Some(total_rooms),
    })
}

/// Closed token for an `m.room_key.withheld` code observed for a session
/// (issue #460). Only the codes retained by the SDK store are correlatable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixRoomKeyWithheldCode {
    Blacklisted,
    Unverified,
    Unauthorised,
    Unavailable,
    Other,
}

impl MatrixRoomKeyWithheldCode {
    pub fn token(self) -> &'static str {
        match self {
            Self::Blacklisted => "blacklisted",
            Self::Unverified => "unverified",
            Self::Unauthorised => "unauthorised",
            Self::Unavailable => "unavailable",
            Self::Other => "custom",
        }
    }
}

/// Live stream of `m.room_key.withheld` codes (issue #460), mapped to closed
/// app-owned tokens. Never exposes raw SDK content. The broadcast
/// subscription is established eagerly (before the caller reads the stored
/// snapshot) so observations cannot fall into a snapshot/subscription gap.
pub async fn room_key_withheld_stream(
    session: &MatrixClientSession,
) -> impl futures_util::Stream<Item = Vec<(String, String, MatrixRoomKeyWithheldCode)>> + use<> {
    use futures_util::StreamExt;

    let client = session.client();
    let stream = client
        .encryption()
        .room_keys_withheld_received_stream()
        .await;
    let stream = futures_util::stream::iter(stream).flatten();
    stream.map(move |infos| {
        infos
            .into_iter()
            .filter_map(|info| {
                let code = withheld_code_from_sdk(info.withheld_event.content)?;
                Some((info.room_id.to_string(), info.session_id, code))
            })
            .collect()
    })
}

fn withheld_code_from_sdk(
    content: matrix_sdk::encryption::RoomKeyWithheldContent,
) -> Option<MatrixRoomKeyWithheldCode> {
    use matrix_sdk::encryption::RoomKeyWithheldContent;
    match content {
        RoomKeyWithheldContent::MegolmV1AesSha2(content) => Some(match content.withheld_code() {
            matrix_sdk::encryption::WithheldCode::Blacklisted => {
                MatrixRoomKeyWithheldCode::Blacklisted
            }
            matrix_sdk::encryption::WithheldCode::Unverified => {
                MatrixRoomKeyWithheldCode::Unverified
            }
            matrix_sdk::encryption::WithheldCode::Unauthorised => {
                MatrixRoomKeyWithheldCode::Unauthorised
            }
            matrix_sdk::encryption::WithheldCode::Unavailable => {
                MatrixRoomKeyWithheldCode::Unavailable
            }
            _ => MatrixRoomKeyWithheldCode::Other,
        }),
        _ => Some(MatrixRoomKeyWithheldCode::Other),
    }
}

/// Stored `m.room_key.withheld` codes for a room (issue #460), mapped to
/// closed tokens keyed by session id.
pub async fn room_key_withheld_codes(
    session: &MatrixClientSession,
    room_id: &str,
) -> Vec<(String, MatrixRoomKeyWithheldCode)> {
    let Ok(room_id) = matrix_sdk::ruma::RoomId::parse(room_id) else {
        return Vec::new();
    };
    session
        .client()
        .encryption()
        .room_key_withheld_codes(room_id.as_ref())
        .await
        .into_iter()
        .map(|(session, code)| {
            use matrix_sdk::encryption::WithheldCode;
            let code = match code {
                WithheldCode::Blacklisted => MatrixRoomKeyWithheldCode::Blacklisted,
                WithheldCode::Unverified => MatrixRoomKeyWithheldCode::Unverified,
                WithheldCode::Unauthorised => MatrixRoomKeyWithheldCode::Unauthorised,
                WithheldCode::Unavailable => MatrixRoomKeyWithheldCode::Unavailable,
                _ => MatrixRoomKeyWithheldCode::Other,
            };
            (session, code)
        })
        .collect()
}

/// Whether the local crypto store already holds an inbound group session for
/// the given room + Megolm session (issue #478 local recovery source).
pub async fn has_inbound_group_session(
    session: &MatrixClientSession,
    room_id: &str,
    session_id: &str,
) -> Result<bool, MatrixRoomOperationError> {
    let room_id = matrix_sdk::ruma::RoomId::parse(room_id)
        .map_err(|_| MatrixRoomOperationError::InvalidRoomId)?;
    session
        .client()
        .encryption()
        .has_inbound_group_session(room_id.as_ref(), session_id)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn download_room_key_from_backup(
    session: &MatrixClientSession,
    room_id: &str,
    session_id: &str,
) -> Result<bool, E2eeTrustError> {
    let room_id = matrix_sdk::ruma::RoomId::parse(room_id)
        .map_err(|_| E2eeTrustError::Sdk("invalid room id".to_owned()))?;
    session
        .client()
        .encryption()
        .backups()
        .download_room_key(room_id.as_ref(), session_id)
        .await
        .map_err(|error| E2eeTrustError::Classified(e2ee_trust_failure_kind(&error)))
}

#[cfg(not(target_family = "wasm"))]
pub async fn export_room_keys_to_file(
    session: &MatrixClientSession,
    path: PathBuf,
    passphrase: &AuthSecret,
) -> Result<RoomKeyExportSummary, E2eeTrustError> {
    session
        .client()
        .encryption()
        .export_room_keys(path, passphrase.expose_secret(), |_| true)
        .await?;
    Ok(RoomKeyExportSummary {
        exported_sessions: None,
    })
}

#[cfg(not(target_family = "wasm"))]
pub async fn import_room_keys_from_file(
    session: &MatrixClientSession,
    path: PathBuf,
    passphrase: &AuthSecret,
) -> Result<RoomKeyImportSummary, E2eeTrustError> {
    let result = session
        .client()
        .encryption()
        .import_room_keys(path, passphrase.expose_secret())
        .await
        .map_err(|error| E2eeTrustError::Sdk(error.to_string()))?;
    Ok(RoomKeyImportSummary {
        imported_count: result.imported_count as u64,
        total_count: result.total_count as u64,
    })
}

pub async fn bootstrap_secure_backup(
    session: &MatrixClientSession,
    passphrase: Option<&AuthSecret>,
    recovery_key_destination_path: Option<PathBuf>,
) -> Result<SecureBackupSetupSummary, E2eeTrustError> {
    let recovery = session.client().encryption().recovery();
    let recovery_key = match passphrase {
        Some(passphrase) => {
            recovery
                .enable()
                .wait_for_backups_to_upload()
                .with_passphrase(passphrase.expose_secret())
                .await?
        }
        None => recovery.enable().wait_for_backups_to_upload().await?,
    };
    let recovery_key_written =
        write_recovery_key_if_requested(recovery_key, recovery_key_destination_path)?;
    Ok(SecureBackupSetupSummary {
        recovery_key_written,
    })
}

pub async fn change_secure_backup_passphrase(
    session: &MatrixClientSession,
    old_secret: &AuthSecret,
    new_passphrase: &AuthSecret,
    recovery_key_destination_path: Option<PathBuf>,
) -> Result<SecureBackupSetupSummary, E2eeTrustError> {
    let recovery_key = session
        .client()
        .encryption()
        .recovery()
        .recover_and_reset(old_secret.expose_secret())
        .with_passphrase(new_passphrase.expose_secret())
        .await?;
    let recovery_key_written =
        write_recovery_key_if_requested(recovery_key, recovery_key_destination_path)?;
    Ok(SecureBackupSetupSummary {
        recovery_key_written,
    })
}

fn write_recovery_key_if_requested(
    recovery_key: String,
    destination_path: Option<PathBuf>,
) -> Result<bool, E2eeTrustError> {
    let recovery_key = Zeroizing::new(recovery_key);
    write_recovery_key_material(&recovery_key, destination_path)
}

fn write_recovery_key_material(
    recovery_key: &str,
    destination_path: Option<PathBuf>,
) -> Result<bool, E2eeTrustError> {
    use std::io::Write as _;

    let Some(destination_path) = destination_path else {
        return Ok(false);
    };

    let mut options = std::fs::OpenOptions::new();
    // The native save dialog is expected to return a new artifact path. Refuse
    // to follow or overwrite an existing file (including a symlink).
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(&destination_path)
        .map_err(|_| E2eeTrustError::SecureBackupRecoveryKeyDeliveryFailed)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|_| E2eeTrustError::SecureBackupRecoveryKeyDeliveryFailed)?;
    }
    if file.write_all(recovery_key.as_bytes()).is_err() || file.sync_all().is_err() {
        drop(file);
        let _ = std::fs::remove_file(&destination_path);
        return Err(E2eeTrustError::SecureBackupRecoveryKeyDeliveryFailed);
    }
    Ok(true)
}

pub async fn reset_identity(
    session: &MatrixClientSession,
) -> Result<IdentityResetOutcome, E2eeTrustError> {
    let outcome = session
        .client()
        .encryption()
        .recovery()
        .reset_identity()
        .await?;

    Ok(match outcome {
        Some(handle) => {
            let auth_type = map_sdk_identity_reset_auth_type(handle.auth_type());
            IdentityResetOutcome::AuthRequired(MatrixIdentityResetHandle {
                inner: handle,
                auth_type,
            })
        }
        None => IdentityResetOutcome::Completed,
    })
}

pub async fn complete_identity_reset(
    session: &MatrixClientSession,
    handle: &MatrixIdentityResetHandle,
    request: &IdentityResetAuthRequest,
) -> Result<(), E2eeTrustError> {
    handle.reset(session, request).await
}

pub async fn cleanup_current_device(
    session: &MatrixClientSession,
    password: Option<&AuthSecret>,
    uiaa_session: Option<&str>,
) -> Result<MatrixDeviceCleanupOutcome, DeviceCleanupFailureKind> {
    if session.device_cleanup_auth_mode() == DeviceCleanupAuthMode::OAuth {
        return cleanup_oauth_session(session.client().oauth()).await;
    }

    let raw_device_id = session.info.device_id.as_str();
    let device_ids = [matrix_sdk::ruma::OwnedDeviceId::from(raw_device_id)];
    let auth_data = device_cleanup_auth_data(session, password, uiaa_session);
    match session
        .client()
        .delete_devices(&device_ids, auth_data)
        .await
    {
        Ok(_) => Ok(MatrixDeviceCleanupOutcome::Settled(
            DeviceCleanupRemoteOutcome::Success,
        )),
        Err(error) => {
            if let Some(uiaa) = error.as_uiaa_response() {
                return Ok(MatrixDeviceCleanupOutcome::UiaaRequired {
                    session: uiaa.session.clone(),
                });
            }
            classify_device_cleanup_http_fact(
                error.client_api_error_kind(),
                matches!(error, matrix_sdk::HttpError::Reqwest(_)),
            )
            .map(MatrixDeviceCleanupOutcome::Settled)
        }
    }
}

async fn cleanup_oauth_session(
    oauth: matrix_sdk::authentication::oauth::OAuth,
) -> Result<MatrixDeviceCleanupOutcome, DeviceCleanupFailureKind> {
    match oauth.logout().await {
        Ok(()) => Ok(MatrixDeviceCleanupOutcome::Settled(
            DeviceCleanupRemoteOutcome::Success,
        )),
        Err(matrix_sdk::authentication::oauth::OAuthError::NotAuthenticated) => Ok(
            MatrixDeviceCleanupOutcome::Settled(DeviceCleanupRemoteOutcome::AlreadyAbsent),
        ),
        Err(_) => Err(DeviceCleanupFailureKind::Sdk),
    }
}

fn classify_device_cleanup_http_fact(
    kind: Option<&matrix_sdk::ruma::api::error::ErrorKind>,
    network_failure: bool,
) -> Result<DeviceCleanupRemoteOutcome, DeviceCleanupFailureKind> {
    use matrix_sdk::ruma::api::error::ErrorKind;

    match kind {
        Some(ErrorKind::Forbidden) | Some(ErrorKind::UnknownToken(_)) => {
            Err(DeviceCleanupFailureKind::Forbidden)
        }
        _ if network_failure => Err(DeviceCleanupFailureKind::Network),
        _ => Err(DeviceCleanupFailureKind::Sdk),
    }
}

fn device_cleanup_auth_data(
    session: &MatrixClientSession,
    password: Option<&AuthSecret>,
    uiaa_session: Option<&str>,
) -> Option<matrix_sdk::ruma::api::client::uiaa::AuthData> {
    let password = password?;
    let identifier = matrix_sdk::ruma::api::client::uiaa::UserIdentifier::Matrix(
        matrix_sdk::ruma::api::client::uiaa::MatrixUserIdentifier::new(
            session.info.user_id.clone(),
        ),
    );
    let mut password_auth = matrix_sdk::ruma::api::client::uiaa::Password::new(
        identifier,
        password.expose_secret().to_owned(),
    );
    password_auth.session = uiaa_session.map(str::to_owned);
    Some(matrix_sdk::ruma::api::client::uiaa::AuthData::Password(
        password_auth,
    ))
}

#[cfg(test)]
mod device_cleanup_tests {
    use matrix_sdk::ruma::api::error::{ErrorKind, UnknownTokenErrorData};
    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    use serde_json::json;
    use wiremock::{
        Mock, ResponseTemplate,
        matchers::{body_json, method, path_regex},
    };

    use super::{
        DeviceCleanupAuthMode, DeviceCleanupFailureKind, DeviceCleanupRemoteOutcome,
        MatrixClientSession, MatrixDeviceCleanupOutcome, SessionInfo,
        classify_device_cleanup_http_fact, cleanup_current_device, cleanup_oauth_session,
    };

    async fn session_for(server: &MatrixMockServer) -> MatrixClientSession {
        let client = server.client_builder().build().await;
        MatrixClientSession::from_client_for_testing(
            client.clone(),
            SessionInfo {
                homeserver: server.server().uri(),
                user_id: client
                    .user_id()
                    .expect("mock client has a user id")
                    .to_string(),
                device_id: client
                    .device_id()
                    .expect("mock client has a device id")
                    .to_string(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            },
        )
    }

    #[tokio::test]
    async fn device_cleanup_auth_mode_is_legacy_without_an_oauth_full_session() {
        let server = MatrixMockServer::new().await;
        let session = session_for(&server).await;

        assert_eq!(
            session.device_cleanup_auth_mode(),
            DeviceCleanupAuthMode::Legacy
        );
    }

    #[tokio::test]
    async fn device_cleanup_auth_mode_is_oauth_for_an_oauth_full_session() {
        let server = MatrixMockServer::new().await;
        let client = matrix_sdk::Client::builder()
            .homeserver_url(server.server().uri())
            .build()
            .await
            .expect("OAuth test client");
        client
            .oauth()
            .restore_session(
                matrix_sdk::test_utils::client::oauth::mock_session(
                    matrix_sdk::test_utils::client::mock_session_tokens(),
                ),
                matrix_sdk_base::store::RoomLoadSettings::default(),
            )
            .await
            .expect("synthetic OAuth session");
        let session = MatrixClientSession::from_client_for_testing(
            client.clone(),
            SessionInfo {
                homeserver: client.homeserver().to_string(),
                user_id: client.user_id().expect("OAuth user").to_string(),
                device_id: client.device_id().expect("OAuth device").to_string(),
                authentication_method: koushi_state::SessionAuthenticationMethod::OAuth,
            },
        );
        assert_eq!(
            session.device_cleanup_auth_mode(),
            DeviceCleanupAuthMode::OAuth
        );
    }

    #[tokio::test]
    async fn oauth_device_cleanup_revokes_tokens_without_matrix_uiaa() {
        let server = MatrixMockServer::new().await;
        let oauth_server = server.oauth();
        oauth_server
            .mock_server_metadata()
            .ok_https()
            .expect(1..)
            .named("server_metadata")
            .mount()
            .await;
        oauth_server
            .mock_revocation()
            .ok()
            .expect(1)
            .named("revocation")
            .mount()
            .await;
        let client = server.client_builder().unlogged().build().await;
        client
            .oauth()
            .restore_session(
                matrix_sdk::test_utils::client::oauth::mock_session(
                    matrix_sdk::test_utils::client::mock_session_tokens_with_refresh(),
                ),
                matrix_sdk_base::store::RoomLoadSettings::default(),
            )
            .await
            .expect("synthetic OAuth session");
        let session = MatrixClientSession::from_client_for_testing(
            client.clone(),
            SessionInfo {
                homeserver: client.homeserver().to_string(),
                user_id: client.user_id().expect("OAuth user").to_string(),
                device_id: client.device_id().expect("OAuth device").to_string(),
                authentication_method: koushi_state::SessionAuthenticationMethod::OAuth,
            },
        );
        client
            .oauth()
            .server_metadata()
            .await
            .expect("OAuth server metadata");
        assert_eq!(
            session.device_cleanup_auth_mode(),
            DeviceCleanupAuthMode::OAuth
        );

        assert_eq!(
            cleanup_oauth_session(client.oauth().insecure_rewrite_https_to_http()).await,
            Ok(MatrixDeviceCleanupOutcome::Settled(
                DeviceCleanupRemoteOutcome::Success
            ))
        );
    }

    #[tokio::test]
    async fn oauth_device_cleanup_maps_an_absent_session_without_uiaa() {
        let server = MatrixMockServer::new().await;
        let client = server.client_builder().unlogged().build().await;

        assert_eq!(
            cleanup_oauth_session(client.oauth()).await,
            Ok(MatrixDeviceCleanupOutcome::Settled(
                DeviceCleanupRemoteOutcome::AlreadyAbsent
            ))
        );
    }

    #[tokio::test]
    async fn legacy_device_cleanup_deletes_the_authoritative_current_device_and_returns_uiaa() {
        let server = MatrixMockServer::new().await;
        let session = session_for(&server).await;
        let expected_device_id = session.info.device_id.clone();
        Mock::given(method("POST"))
            .and(path_regex(r"^/_matrix/client/(?:v3|r0)/delete_devices$"))
            .and(body_json(json!({ "devices": [expected_device_id] })))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "session": "opaque-uiaa-session",
                "flows": [{ "stages": ["m.login.password"] }],
                "params": {},
                "completed": []
            })))
            .expect(1)
            .mount(server.server())
            .await;

        let outcome = cleanup_current_device(&session, None, None)
            .await
            .expect("UIAA is an expected continuation, not a failure");
        assert_eq!(
            outcome,
            MatrixDeviceCleanupOutcome::UiaaRequired {
                session: Some("opaque-uiaa-session".to_owned()),
            }
        );
    }

    #[tokio::test]
    async fn legacy_device_cleanup_keeps_unknown_token_retryable() {
        let server = MatrixMockServer::new().await;
        let session = session_for(&server).await;
        Mock::given(method("POST"))
            .and(path_regex(r"^/_matrix/client/(?:v3|r0)/delete_devices$"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "errcode": "M_UNKNOWN_TOKEN",
                "error": "expired",
                "soft_logout": false
            })))
            .expect(1)
            .mount(server.server())
            .await;

        assert_eq!(
            cleanup_current_device(&session, None, None).await,
            Err(DeviceCleanupFailureKind::Forbidden)
        );
    }

    #[test]
    fn device_cleanup_uiaa_debug_redacts_the_opaque_session() {
        let outcome = MatrixDeviceCleanupOutcome::UiaaRequired {
            session: Some("opaque-uiaa-session".to_owned()),
        };

        let debug = format!("{outcome:?}");
        assert!(debug.contains("SessionId(..)"));
        assert!(!debug.contains("opaque-uiaa-session"));
    }

    #[test]
    fn device_cleanup_http_classification_requires_authoritative_absence() {
        assert_eq!(
            classify_device_cleanup_http_fact(
                Some(&ErrorKind::UnknownToken(UnknownTokenErrorData::new())),
                false,
            ),
            Err(DeviceCleanupFailureKind::Forbidden)
        );
        assert_eq!(
            classify_device_cleanup_http_fact(Some(&ErrorKind::NotFound), false),
            Err(DeviceCleanupFailureKind::Sdk)
        );
        assert_eq!(
            classify_device_cleanup_http_fact(Some(&ErrorKind::Forbidden), false),
            Err(DeviceCleanupFailureKind::Forbidden)
        );
        assert_eq!(
            classify_device_cleanup_http_fact(None, true),
            Err(DeviceCleanupFailureKind::Network)
        );
        assert_eq!(
            classify_device_cleanup_http_fact(None, false),
            Err(DeviceCleanupFailureKind::Sdk)
        );
    }
}

#[derive(thiserror::Error)]
pub enum AccountManagementError {
    #[error("interactive authentication required")]
    UiaaChallenge { session: Option<String> },
    #[error("Matrix SDK account management failed")]
    Sdk(String),
}

impl fmt::Debug for AccountManagementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UiaaChallenge { session } => formatter
                .debug_struct("UiaaChallenge")
                .field("session", &session.as_ref().map(|_| "SessionId(..)"))
                .finish(),
            Self::Sdk(_) => formatter.write_str("Sdk(..)"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccountManagementCapabilities {
    pub change_password: bool,
}

pub async fn account_management_capabilities(
    session: &MatrixClientSession,
) -> AccountManagementCapabilities {
    let change_password = session
        .client()
        .homeserver_capabilities()
        .can_change_password()
        .await
        .ok()
        .unwrap_or(true);
    AccountManagementCapabilities { change_password }
}

pub async fn change_password(
    session: &MatrixClientSession,
    new_password: &AuthSecret,
    auth: Option<&IdentityResetAuthRequest>,
    uiaa_session: Option<&str>,
) -> Result<(), AccountManagementError> {
    let auth_data = account_management_auth_data(session, auth, uiaa_session);
    match session
        .client()
        .account()
        .change_password(new_password.expose_secret(), auth_data)
        .await
    {
        Ok(_) => Ok(()),
        Err(error) => {
            if let Some(uiaa) = error.as_uiaa_response() {
                Err(AccountManagementError::UiaaChallenge {
                    session: uiaa.session.clone(),
                })
            } else {
                Err(AccountManagementError::Sdk(error.to_string()))
            }
        }
    }
}

pub async fn deactivate_account(
    session: &MatrixClientSession,
    erase_data: bool,
    auth: Option<&IdentityResetAuthRequest>,
    uiaa_session: Option<&str>,
) -> Result<(), AccountManagementError> {
    let auth_data = account_management_auth_data(session, auth, uiaa_session);
    match session
        .client()
        .account()
        .deactivate(None, auth_data, erase_data)
        .await
    {
        Ok(_) => Ok(()),
        Err(error) => {
            if let Some(uiaa) = error.as_uiaa_response() {
                Err(AccountManagementError::UiaaChallenge {
                    session: uiaa.session.clone(),
                })
            } else {
                Err(AccountManagementError::Sdk(error.to_string()))
            }
        }
    }
}

fn account_management_auth_data(
    session: &MatrixClientSession,
    auth: Option<&IdentityResetAuthRequest>,
    uiaa_session: Option<&str>,
) -> Option<matrix_sdk::ruma::api::client::uiaa::AuthData> {
    let IdentityResetAuthRequest::UiaaPassword { password } = auth? else {
        return None;
    };
    let identifier = matrix_sdk::ruma::api::client::uiaa::UserIdentifier::Matrix(
        matrix_sdk::ruma::api::client::uiaa::MatrixUserIdentifier::new(
            session.info.user_id.clone(),
        ),
    );
    let mut password_auth = matrix_sdk::ruma::api::client::uiaa::Password::new(
        identifier,
        password.expose_secret().to_owned(),
    );
    password_auth.session = uiaa_session.map(str::to_owned);
    Some(matrix_sdk::ruma::api::client::uiaa::AuthData::Password(
        password_auth,
    ))
}

pub async fn request_device_verification(
    session: &MatrixClientSession,
    target: &VerificationTarget,
) -> Result<MatrixVerificationRequestHandle, E2eeTrustError> {
    let user_id = matrix_sdk::ruma::OwnedUserId::try_from(target.user_id.as_str())
        .map_err(|_| E2eeTrustError::Sdk("invalid verification user id".to_owned()))?;
    let device_id = matrix_sdk::ruma::OwnedDeviceId::from(target.device_id.as_str());
    let device = session
        .client()
        .encryption()
        .get_device(&user_id, &device_id)
        .await
        .map_err(|error| E2eeTrustError::Sdk(error.to_string()))?
        .ok_or_else(|| E2eeTrustError::Sdk("verification target device not found".to_owned()))?;

    let inner = device
        .request_verification_with_methods(vec![
            matrix_sdk::ruma::events::key::verification::VerificationMethod::SasV1,
        ])
        .await?;
    Ok(MatrixVerificationRequestHandle { inner })
}

pub async fn discover_current_session_verification_methods(
    session: &MatrixClientSession,
) -> VerificationGateState {
    let Ok(user_id) = matrix_sdk::ruma::OwnedUserId::try_from(session.info.user_id.as_str()) else {
        return map_verification_method_facts(VerificationMethodFacts {
            identity: IdentityFact::Unknown,
            verified_other_device_count: 0,
            recovery: RecoveryFact::Unknown,
        });
    };
    let encryption = session.client().encryption();
    let identity = match encryption.request_user_identity(&user_id).await {
        Ok(Some(_)) => IdentityFact::Existing,
        Ok(None) => IdentityFact::Missing,
        Err(_) => IdentityFact::Unknown,
    };
    if !matches!(identity, IdentityFact::Existing) {
        return map_verification_method_facts(VerificationMethodFacts {
            identity,
            verified_other_device_count: 0,
            recovery: if matches!(identity, IdentityFact::Unknown) {
                RecoveryFact::Unknown
            } else {
                RecoveryFact::Unavailable
            },
        });
    }
    let verified_other_device_count = match encryption.get_user_devices(&user_id).await {
        Ok(devices) => devices
            .devices()
            .filter(|device| {
                // A provisional device does not trust the owner identity yet,
                // so local cross-signing trust creates a chicken-and-egg
                // dependency. Proof eligibility is the authoritative owner
                // signature on a distinct, non-blocked device.
                is_eligible_own_user_proof_device(
                    &session.info.device_id,
                    device.device_id().as_str(),
                    device.is_cross_signed_by_owner(),
                    device.is_blacklisted(),
                )
            })
            .count() as u64,
        Err(_) => {
            return map_verification_method_facts(VerificationMethodFacts {
                identity: IdentityFact::Unknown,
                verified_other_device_count: 0,
                recovery: RecoveryFact::Unknown,
            });
        }
    };
    let recovery = match session.e2ee_recovery_state() {
        E2eeRecoveryState::Enabled | E2eeRecoveryState::Incomplete => RecoveryFact::Available,
        E2eeRecoveryState::Disabled => RecoveryFact::Unavailable,
        E2eeRecoveryState::Unknown => RecoveryFact::Unknown,
    };
    map_verification_method_facts(VerificationMethodFacts {
        identity,
        verified_other_device_count,
        recovery,
    })
}

pub async fn request_own_user_sas_verification(
    session: &MatrixClientSession,
    flow_id: u64,
) -> Result<MatrixOwnUserVerificationHandle, E2eeTrustError> {
    record_sas_delivery_event(sas_delivery_event("request_started", flow_id));
    let user_id = match matrix_sdk::ruma::OwnedUserId::try_from(session.info.user_id.as_str()) {
        Ok(user_id) => user_id,
        Err(_) => {
            record_sas_delivery_event(
                sas_delivery_event("request_send_finished", flow_id)
                    .field(DiagnosticField::token("outcome", "failed"))
                    .field(DiagnosticField::token("failure_stage", "invalid_user_id")),
            );
            return Err(E2eeTrustError::Sdk(
                "invalid verification user id".to_owned(),
            ));
        }
    };
    let encryption = session.client().encryption();
    let identity = match encryption.request_user_identity(&user_id).await {
        Ok(Some(identity)) => identity,
        Ok(None) => {
            record_sas_delivery_event(
                sas_delivery_event("request_send_finished", flow_id)
                    .field(DiagnosticField::token("outcome", "failed"))
                    .field(DiagnosticField::token("failure_stage", "identity_missing")),
            );
            return Err(E2eeTrustError::Sdk(
                "verification identity unavailable".to_owned(),
            ));
        }
        Err(_) => {
            record_sas_delivery_event(
                sas_delivery_event("request_send_finished", flow_id)
                    .field(DiagnosticField::token("outcome", "failed"))
                    .field(DiagnosticField::token("failure_stage", "identity_query")),
            );
            return Err(E2eeTrustError::Sdk(
                "verification identity unavailable".to_owned(),
            ));
        }
    };
    record_sas_delivery_event(sas_delivery_waiting_event(flow_id, "recipient_devices"));
    let devices = match encryption.get_user_devices(&user_id).await {
        Ok(devices) => devices,
        Err(_) => {
            record_sas_delivery_event(
                sas_delivery_event("request_send_finished", flow_id)
                    .field(DiagnosticField::token("outcome", "failed"))
                    .field(DiagnosticField::token("failure_stage", "device_query")),
            );
            return Err(E2eeTrustError::Sdk(
                "verification devices unavailable".to_owned(),
            ));
        }
    };
    let recipient_diagnostics =
        own_user_sas_recipient_diagnostics(devices.devices().map(|device| OwnUserSasDeviceFact {
            is_current: device.device_id().as_str() == session.info.device_id,
            cross_signed_by_owner: device.is_cross_signed_by_owner(),
            blocked: device.is_blacklisted(),
            dehydrated: device.is_dehydrated(),
            curve_key_present: device.curve25519_key().is_some(),
            ed25519_key_present: device.ed25519_key().is_some(),
        }));
    let eligible_device_count = recipient_diagnostics.eligible_device_count;
    record_sas_delivery_event(sas_recipients_resolved_event(
        flow_id,
        recipient_diagnostics,
    ));
    if eligible_device_count == 0 {
        record_sas_delivery_event(
            sas_delivery_event("request_send_finished", flow_id)
                .field(DiagnosticField::token("outcome", "failed"))
                .field(DiagnosticField::token(
                    "failure_stage",
                    "no_eligible_device",
                )),
        );
        return Err(E2eeTrustError::Sdk(
            "verification device unavailable".to_owned(),
        ));
    }
    record_sas_delivery_event(sas_delivery_waiting_event(flow_id, "to_device_delivery"));
    let inner = match identity
        .request_verification_with_methods(vec![
            matrix_sdk::ruma::events::key::verification::VerificationMethod::SasV1,
        ])
        .await
    {
        Ok(inner) => inner,
        Err(_) => {
            record_sas_delivery_event(
                sas_delivery_event("request_send_finished", flow_id)
                    .field(DiagnosticField::token("outcome", "failed"))
                    .field(DiagnosticField::token("failure_stage", "send")),
            );
            return Err(E2eeTrustError::Sdk(
                "verification request failed".to_owned(),
            ));
        }
    };
    let request = MatrixVerificationRequestHandle { inner };
    record_sas_delivery_event(
        sas_delivery_event("request_send_finished", flow_id)
            .field(DiagnosticField::token("outcome", "success"))
            .field(DiagnosticField::token(
                "initial_state",
                verification_request_state_token(&request.state()),
            )),
    );
    Ok(MatrixOwnUserVerificationHandle {
        request,
        eligible_device_count,
    })
}

pub async fn start_own_user_sas_verification(
    handle: &MatrixOwnUserVerificationHandle,
) -> Result<Option<MatrixSasVerificationHandle>, E2eeTrustError> {
    start_sas_verification(&handle.request).await
}

pub async fn cancel_own_user_sas_verification(
    handle: &MatrixOwnUserVerificationHandle,
) -> Result<(), E2eeTrustError> {
    cancel_verification_request(&handle.request).await
}

pub async fn observe_incoming_verification_requests(
    session: &MatrixClientSession,
) -> MatrixIncomingVerificationRequestObserver {
    let client = session.client();
    let (sender, receiver) = tokio::sync::mpsc::channel(32);

    // All normal and recovered to-device requests use the same typed lease stream. Generic raw
    // SDK handlers remain compatibility fanout only and do not own Koushi product delivery.
    let incoming_request_task = client
        .encryption()
        .subscribe_to_incoming_verification_requests()
        .await
        .map(|incoming_requests| {
            let sender = sender.clone();
            tokio::spawn(forward_incoming_verification_requests(
                incoming_requests,
                sender,
            ))
        });

    let room_client = client.clone();
    let room_sender = sender;
    let room_handler = client.add_event_handler(
        move |event: matrix_sdk::ruma::events::room::message::OriginalSyncRoomMessageEvent| {
            let client = room_client.clone();
            let sender = room_sender.clone();
            async move {
                if !matches!(
                    &event.content.msgtype,
                    matrix_sdk::ruma::events::room::message::MessageType::VerificationRequest(_)
                ) {
                    return;
                }
                if let Some(request) = incoming_verification_request_for_flow(
                    &client,
                    &event.sender,
                    event.event_id.as_str(),
                )
                .await
                {
                    let _ = sender.send(request).await;
                }
            }
        },
    );

    MatrixIncomingVerificationRequestObserver {
        client,
        receiver: Some(receiver),
        handlers: vec![room_handler],
        incoming_request_task,
    }
}

async fn forward_incoming_verification_requests(
    incoming_requests: impl Stream<Item = matrix_sdk::encryption::IncomingVerificationRequestDelivery>,
    sender: tokio::sync::mpsc::Sender<MatrixIncomingVerificationRequest>,
) {
    forward_incoming_verification_deliveries(
        incoming_requests,
        sender,
        |delivery| incoming_verification_request_from_handle(delivery.request().clone()),
        matrix_sdk::encryption::IncomingVerificationRequestDelivery::commit,
    )
    .await;
}

async fn forward_incoming_verification_deliveries<D, P>(
    deliveries: impl Stream<Item = D>,
    sender: tokio::sync::mpsc::Sender<P>,
    mut project: impl FnMut(&D) -> Option<P>,
    mut commit: impl FnMut(D),
) {
    futures_util::pin_mut!(deliveries);
    while let Some(delivery) = deliveries.next().await {
        let Some(product) = project(&delivery) else {
            // Terminal/non-actionable heads are consumed so they cannot starve later requests.
            commit(delivery);
            continue;
        };
        if sender.send(product).await.is_err() {
            // Dropping without commit releases the SDK lease for a later observer.
            break;
        }
        commit(delivery);
    }
}

async fn incoming_verification_request_for_flow(
    client: &matrix_sdk::Client,
    sender: &matrix_sdk::ruma::UserId,
    flow_id: &str,
) -> Option<MatrixIncomingVerificationRequest> {
    let request = client
        .encryption()
        .get_verification_request(sender, flow_id)
        .await?;
    incoming_verification_request_from_handle(request)
}

fn incoming_verification_request_from_handle(
    request: matrix_sdk::encryption::verification::VerificationRequest,
) -> Option<MatrixIncomingVerificationRequest> {
    let matrix_sdk::encryption::verification::VerificationRequestState::Requested {
        other_device_data,
        ..
    } = request.state()
    else {
        return None;
    };

    Some(MatrixIncomingVerificationRequest {
        target: VerificationTarget {
            user_id: request.other_user_id().to_string(),
            device_id: other_device_data.device_id().to_string(),
        },
        handle: MatrixVerificationRequestHandle { inner: request },
    })
}

pub async fn accept_verification_request(
    handle: &MatrixVerificationRequestHandle,
) -> Result<(), E2eeTrustError> {
    handle
        .inner
        .accept_with_methods(vec![
            matrix_sdk::ruma::events::key::verification::VerificationMethod::SasV1,
        ])
        .await?;
    Ok(())
}

pub async fn start_sas_verification(
    handle: &MatrixVerificationRequestHandle,
) -> Result<Option<MatrixSasVerificationHandle>, E2eeTrustError> {
    Ok(handle
        .inner
        .start_sas()
        .await?
        .map(|inner| MatrixSasVerificationHandle { inner }))
}

pub async fn accept_sas_verification(
    handle: &MatrixSasVerificationHandle,
) -> Result<(), E2eeTrustError> {
    handle.inner.accept().await?;
    Ok(())
}

pub async fn confirm_sas_verification(
    handle: &MatrixSasVerificationHandle,
) -> Result<(), E2eeTrustError> {
    handle.inner.confirm().await?;
    Ok(())
}

pub async fn mismatch_sas_verification(
    handle: &MatrixSasVerificationHandle,
) -> Result<(), E2eeTrustError> {
    handle.inner.mismatch().await?;
    Ok(())
}

pub async fn cancel_verification_request(
    handle: &MatrixVerificationRequestHandle,
) -> Result<(), E2eeTrustError> {
    handle.inner.cancel().await?;
    Ok(())
}

pub async fn cancel_sas_verification(
    handle: &MatrixSasVerificationHandle,
) -> Result<(), E2eeTrustError> {
    handle.inner.cancel().await?;
    Ok(())
}

pub fn map_backup_state_to_desktop(
    state: matrix_sdk::encryption::backups::BackupState,
) -> KeyBackupStatus {
    use matrix_sdk::encryption::backups::BackupState;

    match state {
        BackupState::Unknown => KeyBackupStatus::Unknown,
        BackupState::Creating | BackupState::Enabling | BackupState::Resuming => {
            KeyBackupStatus::Enabling { request_id: 0 }
        }
        BackupState::Enabled => KeyBackupStatus::Enabled {
            version: "available".to_owned(),
        },
        BackupState::Downloading => KeyBackupStatus::Restoring {
            request_id: 0,
            version: None,
            restored_rooms: 0,
            total_rooms: None,
        },
        BackupState::Disabling => KeyBackupStatus::Disabled,
    }
}

#[cfg(test)]
mod secure_backup_inspection_tests {
    use koushi_state::{PendingKeyCountBucket, SecureBackupGateFailureKind, SecureBackupGateState};

    use super::{
        E2eeTrustError, MatrixSecureBackupInspection, MatrixSecureBackupLocalState,
        MatrixSecureBackupRecoveryState, MatrixSecureBackupServerState, MatrixSecureBackupState,
        MatrixSecureBackupStateObservation, MatrixSecureBackupTrustState,
        MatrixSecureBackupUploadState, SecureBackupStateStream, classify_secure_backup_upload,
    };

    #[test]
    fn secure_backup_upload_snapshot_classifies_without_waiting_for_settlement() {
        use matrix_sdk::encryption::backups::UploadState;
        use matrix_sdk_base::crypto::store::types::RoomKeyCounts;

        assert_eq!(
            classify_secure_backup_upload(
                Ok(RoomKeyCounts {
                    total: 125,
                    backed_up: 20,
                }),
                UploadState::Uploading(RoomKeyCounts {
                    total: 125,
                    backed_up: 20,
                }),
            ),
            MatrixSecureBackupUploadState::Pending(PendingKeyCountBucket::OverOneHundred)
        );
        assert_eq!(
            classify_secure_backup_upload(
                Ok(RoomKeyCounts {
                    total: 125,
                    backed_up: 125,
                }),
                UploadState::Done,
            ),
            MatrixSecureBackupUploadState::Settled
        );
        assert_eq!(
            classify_secure_backup_upload(
                Ok(RoomKeyCounts {
                    total: 125,
                    backed_up: 20,
                }),
                UploadState::Error,
            ),
            MatrixSecureBackupUploadState::Failed
        );
    }

    fn inspection(
        server: MatrixSecureBackupServerState,
        local: MatrixSecureBackupLocalState,
        recovery: MatrixSecureBackupRecoveryState,
        upload: MatrixSecureBackupUploadState,
        trust: MatrixSecureBackupTrustState,
    ) -> MatrixSecureBackupInspection {
        MatrixSecureBackupInspection {
            server,
            local,
            recovery,
            upload,
            trust,
            recovery_key_delivery_pending: false,
        }
    }

    #[test]
    fn secure_backup_inspection_classifies_required_cartesian_cases() {
        assert_eq!(
            inspection(
                MatrixSecureBackupServerState::Present,
                MatrixSecureBackupLocalState::Enabled,
                MatrixSecureBackupRecoveryState::Enabled,
                MatrixSecureBackupUploadState::Settled,
                MatrixSecureBackupTrustState::Trusted,
            )
            .recommended_gate_state(),
            SecureBackupGateState::Ready
        );

        assert_eq!(
            inspection(
                MatrixSecureBackupServerState::Present,
                MatrixSecureBackupLocalState::Disabled,
                MatrixSecureBackupRecoveryState::Enabled,
                MatrixSecureBackupUploadState::Unknown,
                MatrixSecureBackupTrustState::Unknown,
            )
            .recommended_gate_state(),
            SecureBackupGateState::ExistingBackupNeedsRecovery { failure: None }
        );

        assert_eq!(
            inspection(
                MatrixSecureBackupServerState::Absent,
                MatrixSecureBackupLocalState::Disabled,
                MatrixSecureBackupRecoveryState::Unknown,
                MatrixSecureBackupUploadState::Unknown,
                MatrixSecureBackupTrustState::Unknown,
            )
            .recommended_gate_state(),
            SecureBackupGateState::SetupRequired
        );

        assert_eq!(
            inspection(
                MatrixSecureBackupServerState::Absent,
                MatrixSecureBackupLocalState::Disabled,
                MatrixSecureBackupRecoveryState::Disabled,
                MatrixSecureBackupUploadState::Unknown,
                MatrixSecureBackupTrustState::Unknown,
            )
            .recommended_gate_state(),
            SecureBackupGateState::ExplicitlyDisabledRequiresSetup
        );

        assert_eq!(
            inspection(
                MatrixSecureBackupServerState::Unknown,
                MatrixSecureBackupLocalState::Enabled,
                MatrixSecureBackupRecoveryState::Enabled,
                MatrixSecureBackupUploadState::Settled,
                MatrixSecureBackupTrustState::Trusted,
            )
            .recommended_gate_state(),
            SecureBackupGateState::Checking
        );

        assert_eq!(
            inspection(
                MatrixSecureBackupServerState::Present,
                MatrixSecureBackupLocalState::Enabled,
                MatrixSecureBackupRecoveryState::Enabled,
                MatrixSecureBackupUploadState::Settled,
                MatrixSecureBackupTrustState::Mismatch,
            )
            .recommended_gate_state(),
            SecureBackupGateState::ExistingBackupNeedsRecovery {
                failure: Some(SecureBackupGateFailureKind::BackupKeyMismatch),
            }
        );

        assert_eq!(
            inspection(
                MatrixSecureBackupServerState::Present,
                MatrixSecureBackupLocalState::Enabled,
                MatrixSecureBackupRecoveryState::Incomplete,
                MatrixSecureBackupUploadState::Settled,
                MatrixSecureBackupTrustState::Trusted,
            )
            .recommended_gate_state(),
            SecureBackupGateState::SecureStorageIncomplete
        );

        assert_eq!(
            inspection(
                MatrixSecureBackupServerState::Present,
                MatrixSecureBackupLocalState::Enabled,
                MatrixSecureBackupRecoveryState::Enabled,
                MatrixSecureBackupUploadState::Failed,
                MatrixSecureBackupTrustState::Trusted,
            )
            .recommended_gate_state(),
            SecureBackupGateState::DegradedRetrying {
                failure: SecureBackupGateFailureKind::Network,
            }
        );
    }

    #[test]
    fn pending_recovery_key_delivery_survives_inspection_and_keeps_gate_closed() {
        let mut inspection = inspection(
            MatrixSecureBackupServerState::Present,
            MatrixSecureBackupLocalState::Enabled,
            MatrixSecureBackupRecoveryState::Enabled,
            MatrixSecureBackupUploadState::Settled,
            MatrixSecureBackupTrustState::Trusted,
        );
        inspection.recovery_key_delivery_pending = true;

        assert_eq!(
            inspection.recommended_gate_state(),
            koushi_state::SecureBackupGateState::RecoveryKeyDeliveryRequired
        );
    }

    #[test]
    fn secure_backup_inspection_requires_typed_trust_evidence() {
        assert_eq!(
            inspection(
                MatrixSecureBackupServerState::Present,
                MatrixSecureBackupLocalState::Enabled,
                MatrixSecureBackupRecoveryState::Enabled,
                MatrixSecureBackupUploadState::Settled,
                MatrixSecureBackupTrustState::Unknown,
            )
            .recommended_gate_state(),
            SecureBackupGateState::Checking
        );

        assert_eq!(
            inspection(
                MatrixSecureBackupServerState::Present,
                MatrixSecureBackupLocalState::Enabled,
                MatrixSecureBackupRecoveryState::Enabled,
                MatrixSecureBackupUploadState::Settled,
                MatrixSecureBackupTrustState::Mismatch,
            )
            .recommended_gate_state(),
            SecureBackupGateState::ExistingBackupNeedsRecovery {
                failure: Some(SecureBackupGateFailureKind::BackupKeyMismatch),
            }
        );
    }

    #[test]
    fn secure_backup_state_observation_is_public_and_private_data_free() {
        let state = MatrixSecureBackupState {
            backup: MatrixSecureBackupLocalState::Enabled,
            recovery: MatrixSecureBackupRecoveryState::Enabled,
        };
        let serialized = serde_json::to_string(&state).expect("state is serializable");
        let debug = format!("{state:?}");

        assert!(serialized.contains("backup"));
        assert!(serialized.contains("recovery"));
        for forbidden in [
            "backup-version-42",
            "recovery-key-secret",
            "@alice:example.invalid",
            "!room:example.invalid",
            "/tmp/recovery-key.txt",
            "raw SDK failure",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "serialized state leaked {forbidden}"
            );
            assert!(!debug.contains(forbidden), "debug state leaked {forbidden}");
        }

        let _observation: Option<MatrixSecureBackupStateObservation> = None;
        let _stream: Option<SecureBackupStateStream> = None;
        let _observe: fn(&super::MatrixClientSession) -> MatrixSecureBackupStateObservation =
            super::MatrixClientSession::observe_secure_backup_state;
    }

    #[test]
    fn secure_backup_inspection_has_no_secret_or_identifier_surface() {
        let inspection = inspection(
            MatrixSecureBackupServerState::Present,
            MatrixSecureBackupLocalState::Enabled,
            MatrixSecureBackupRecoveryState::Enabled,
            MatrixSecureBackupUploadState::Pending(PendingKeyCountBucket::One),
            MatrixSecureBackupTrustState::Trusted,
        );
        let serialized = serde_json::to_string(&inspection).expect("inspection is serializable");
        let debug = format!("{inspection:?}");
        for forbidden in [
            "backup-version-42",
            "recovery-key-secret",
            "@alice:example.invalid",
            "!room:example.invalid",
            "/tmp/recovery-key.txt",
            "raw SDK failure",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "serialized inspection leaked {forbidden}"
            );
            assert!(
                !debug.contains(forbidden),
                "debug inspection leaked {forbidden}"
            );
        }
        assert!(!serialized.contains("version"));
        assert!(!debug.contains("version"));

        let error = E2eeTrustError::Sdk("raw SDK failure with a recovery-key-secret".to_owned());
        assert!(!format!("{error:?}").contains("raw SDK failure"));
        assert!(!format!("{error:?}").contains("recovery-key-secret"));
    }
}

#[cfg(test)]
mod e2ee_trust_tests {
    use super::{
        E2eeTrustError, IdentityFact, KeyBackupRestoreScope, KeyBackupRestoreSummary,
        MatrixCrossSigningStatus, MatrixIdentityResetAuthType, MatrixIncomingVerificationRequest,
        MatrixIncomingVerificationRequestObserver, PersistableMatrixSession, RecoveryFact,
        RoomKeyExportSummary, RoomKeyImportSummary, SecureBackupSetupSummary,
        VerificationMethodFacts, accept_sas_verification, accept_verification_request,
        bootstrap_cross_signing, bootstrap_secure_backup, cancel_sas_verification,
        cancel_verification_request, change_secure_backup_passphrase, complete_identity_reset,
        confirm_sas_verification, cross_signing_status, enable_key_backup,
        export_room_keys_to_file, forward_incoming_verification_deliveries,
        import_room_keys_from_file, map_backup_state_to_desktop,
        map_cross_signing_status_to_desktop, map_identity_reset_auth_type_to_desktop,
        map_sdk_sas_emojis_to_desktop, map_sdk_verification_state, map_verification_method_facts,
        mismatch_sas_verification, observe_incoming_verification_requests,
        request_device_verification, reset_identity, restore_key_backup, restore_session,
        start_sas_verification, write_recovery_key_if_requested,
    };
    use futures_util::stream;
    use koushi_state::{
        AuthSecret, CrossSigningStatus, CurrentDeviceTrustState, IdentityResetAuthType,
        KeyBackupStatus, SasEmoji, SessionInfo, VerificationAccountKind,
        VerificationMethodCapability,
    };
    use matrix_sdk::encryption::backups::BackupState;
    use matrix_sdk::{
        ruma::{owned_device_id, owned_user_id},
        test_utils::mocks::MatrixMockServer,
    };
    use serde_json::json;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    use std::time::Duration;
    const MATRIX_KEY_EXPORT_HEADER: &str = "-----BEGIN MEGOLM SESSION DATA-----";
    const MATRIX_KEY_EXPORT_FOOTER: &str = "-----END MEGOLM SESSION DATA-----";
    struct FakeIncomingDelivery {
        id: u8,
        product: Option<u8>,
        committed: bool,
        commits: Arc<Mutex<Vec<u8>>>,
        uncommitted_drops: Arc<Mutex<Vec<u8>>>,
    }
    impl Drop for FakeIncomingDelivery {
        fn drop(&mut self) {
            if !self.committed {
                self.uncommitted_drops.lock().unwrap().push(self.id);
            }
        }
    }
    const ELEMENT_COMPATIBLE_KEY_EXPORT: &str = "\
-----BEGIN MEGOLM SESSION DATA-----\n\
Af7mGhlzQ+eGvHu93u0YXd3D/+vYMs3E7gQqOhuCtkvGAAAAASH7pEdWvFyAP1JUisAcpEo\n\
Xke2Q7Kr9hVl/SCc6jXBNeJCZcrUbUV4D/tRQIl3E9L4fOk928YI1J+3z96qiH0uE7hpsCI\n\
CkHKwjPU+0XTzFdIk1X8H7sZ+MD/2Sg/q3y8rtUjz7uEj4GUTnb+9SCOTVmJsRfqgUpM1CU\n\
bDLytHf1JkohY4tWEgpsCc67xdzgodjr12qYrfg/zNm3LGpxlrffJknw4rk5QFTj4kMbqbD\n\
ZZgDTni+HxRTDGge2J620lMOiznvXX+H09Rwruqx5aJvvaaKd86jWRpiO2oSFqHn4u5ONl9\n\
41uzm62Sj0eIm6ZbA9NQs87jQw4LxsejhZVL+NdjIg80zVSBTWhTdo0DTnbFSNP4ReOiz0U\n\
XosOF8A5T8Vdx2nvA0GXltfcHKVKQYh/LJAkNQ7P9UYL4ae/5TtQZkhB1KxCLTRWqADCl53\n\
uBMGpG53EMgY6G6K2DEIOkcv7sdXQF5WpemiSWZqJRWj+cjfs9BpCTbkp/rszWFl2TniWpR\n\
RqIbT2jORlN4rTvdtF0F4z1pqP4qWyR3sLNTkXm9CFRzWADNG0RDZKxbCoo6RPvtaCTfaHo\n\
SwfvzBS6CjfAG+FOugpV48o7+XetaUUPZ6/tZSPhCdeV8eP9q5r0QwWeXFogzoNzWt4HYx9\n\
MdXxzD+f0mtg5gzehrrEEARwI2bCvPpHxlt/Na9oW/GBpkjwR1LSKgg4CtpRyWngPjdEKpZ\n\
GYW19pdjg0qdXNk/eqZsQTsNWVo6A\n\
-----END MEGOLM SESSION DATA-----";
    #[tokio::test]
    async fn incoming_verification_observer_shutdown_joins_typed_delivery_task() {
        let persistable = PersistableMatrixSession::from_json(
            r#"{"homeserver":"https://matrix.example.invalid","user_id":"@alice:example.invalid","device_id":"ALICEDEVICE","access_token":"synthetic-access"}"#,
        )
        .expect("synthetic session should deserialize");
        let session = restore_session(&persistable)
            .await
            .expect("synthetic session should restore");
        let mut observer = observe_incoming_verification_requests(&session).await;
        let abort_handle = observer
            .incoming_request_task
            .as_ref()
            .expect("a restored session has a typed incoming-request subscription")
            .abort_handle();

        observer.shutdown().await;

        assert!(abort_handle.is_finished());
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_incoming_observer_shutdown_retains_inner_task_ownership() {
        struct TaskAlive(Arc<AtomicBool>);
        impl Drop for TaskAlive {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }

        let persistable = PersistableMatrixSession::from_json(
            r#"{"homeserver":"https://matrix.example.invalid","user_id":"@alice:example.invalid","device_id":"ALICEDEVICE","access_token":"synthetic-access"}"#,
        )
        .expect("synthetic session should deserialize");
        let session = restore_session(&persistable)
            .await
            .expect("synthetic session should restore");
        let mut observer = observe_incoming_verification_requests(&session).await;
        let original = observer
            .incoming_request_task
            .take()
            .expect("a restored session has a typed incoming-request subscription");
        original.abort();
        let _ = original.await;

        let alive = Arc::new(AtomicBool::new(true));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        observer.incoming_request_task = Some(tokio::spawn({
            let alive = Arc::clone(&alive);
            async move {
                let _alive = TaskAlive(alive);
                let _ = started_tx.send(());
                let _ = release_rx.recv();
                std::future::pending::<()>().await;
            }
        }));
        started_rx.await.expect("noncooperative inner task started");

        assert!(
            tokio::time::timeout(Duration::from_millis(20), observer.shutdown())
                .await
                .is_err(),
            "the fixture must cancel shutdown while the inner task cannot settle"
        );
        assert!(
            observer.incoming_request_task.is_some(),
            "shutdown cancellation must leave the JoinHandle with its observer owner"
        );
        assert!(alive.load(Ordering::SeqCst));

        release_tx
            .send(())
            .expect("release noncooperative inner task");
        observer.shutdown().await;
        assert!(!alive.load(Ordering::SeqCst));
    }
    #[tokio::test]
    async fn terminal_incoming_head_is_committed_before_actionable_tail() {
        let commits = Arc::new(Mutex::new(Vec::new()));
        let uncommitted_drops = Arc::new(Mutex::new(Vec::new()));
        let delivery = |id, product| FakeIncomingDelivery {
            id,
            product,
            committed: false,
            commits: commits.clone(),
            uncommitted_drops: uncommitted_drops.clone(),
        };
        let (sender, mut receiver) = tokio::sync::mpsc::channel(2);

        forward_incoming_verification_deliveries(
            stream::iter([delivery(1, None), delivery(2, Some(42))]),
            sender,
            |delivery| delivery.product,
            |mut delivery| {
                delivery.committed = true;
                delivery.commits.lock().unwrap().push(delivery.id);
            },
        )
        .await;

        assert_eq!(receiver.recv().await, Some(42));
        assert_eq!(*commits.lock().unwrap(), vec![1, 2]);
        assert!(uncommitted_drops.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn actionable_incoming_delivery_commits_only_after_product_send_success() {
        let commits = Arc::new(Mutex::new(Vec::new()));
        let uncommitted_drops = Arc::new(Mutex::new(Vec::new()));
        let delivery = FakeIncomingDelivery {
            id: 1,
            product: Some(42),
            committed: false,
            commits: commits.clone(),
            uncommitted_drops: uncommitted_drops.clone(),
        };
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        drop(receiver);

        forward_incoming_verification_deliveries(
            stream::iter([delivery]),
            sender,
            |delivery| delivery.product,
            |mut delivery| {
                delivery.committed = true;
                delivery.commits.lock().unwrap().push(delivery.id);
            },
        )
        .await;

        assert!(commits.lock().unwrap().is_empty());
        assert_eq!(*uncommitted_drops.lock().unwrap(), vec![1]);
    }
    #[tokio::test]
    async fn verification_raw_redelivery_reuses_the_same_product_flow_identity() {
        let server = MatrixMockServer::new().await;
        server.mock_crypto_endpoints_preset().await;

        let alice_user_id = owned_user_id!("@alice:example.org");
        let alice_device_id = owned_device_id!("ALICEDEVICE");
        let alice = server
            .client_builder_for_crypto_end_to_end(&alice_user_id, &alice_device_id)
            .build()
            .await;
        let bob_user_id = owned_user_id!("@bob:example.org");
        let bob_device_id = owned_device_id!("BOBDEVICE");
        let bob = server
            .client_builder_for_crypto_end_to_end(&bob_user_id, &bob_device_id)
            .build()
            .await;

        // Publish Bob's device keys without teaching Alice about Bob. The first
        // request delivery must therefore use the passive unknown-sender
        // recovery path after its key query completes.
        server.mock_sync().ok_and_run(&bob, |_| {}).await;
        let session = super::MatrixClientSession {
            client: alice.clone(),
            diagnostic_counters: koushi_diagnostics::DiagnosticCounterContext::registered(),
            info: SessionInfo {
                homeserver: server.server().uri(),
                user_id: alice_user_id.to_string(),
                device_id: alice_device_id.to_string(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            },
        };
        let mut observer = observe_incoming_verification_requests(&session).await;
        let request_timestamp = matrix_sdk::ruma::MilliSecondsSinceUnixEpoch::now().get();
        let request = json!({
            "sender": bob_user_id,
            "type": "m.key.verification.request",
            "content": {
                "from_device": bob_device_id,
                "transaction_id": "sender-key-recovery-flow",
                "methods": ["m.sas.v1"],
                "timestamp": request_timestamp,
            },
        });

        server
            .mock_sync()
            .ok_and_run(&alice, |builder| {
                builder.add_to_device_event(request.clone());
            })
            .await;
        let first = tokio::time::timeout(std::time::Duration::from_secs(5), observer.recv())
            .await
            .expect("typed sender-key recovery should publish without polling delay")
            .expect("typed sender-key recovery should yield the request");
        assert_eq!(first.handle().flow_id(), "sender-key-recovery-flow");

        server
            .mock_sync()
            .ok_and_run(&alice, |builder| {
                builder.add_to_device_event(request);
            })
            .await;
        let repeated = tokio::time::timeout(std::time::Duration::from_secs(5), observer.recv())
            .await
            .expect("at-least-once transport should forward raw redelivery without polling delay")
            .expect("raw redelivery should remain observable");
        assert_eq!(repeated.handle().flow_id(), first.handle().flow_id());
    }
    #[test]
    fn cross_signing_status_maps_to_private_data_free_desktop_status() {
        assert_eq!(
            map_cross_signing_status_to_desktop(None),
            CrossSigningStatus::Missing
        );
        assert_eq!(
            map_cross_signing_status_to_desktop(Some(MatrixCrossSigningStatus {
                has_master: true,
                has_self_signing: true,
                has_user_signing: true,
            })),
            CrossSigningStatus::Trusted
        );
        assert_eq!(
            map_cross_signing_status_to_desktop(Some(MatrixCrossSigningStatus {
                has_master: true,
                has_self_signing: false,
                has_user_signing: true,
            })),
            CrossSigningStatus::NotTrusted
        );
    }
    #[test]
    fn current_device_trust_maps_all_sdk_verification_states() {
        use matrix_sdk::encryption::VerificationState;

        assert_eq!(
            map_sdk_verification_state(VerificationState::Unknown),
            CurrentDeviceTrustState::Unknown
        );
        assert_eq!(
            map_sdk_verification_state(VerificationState::Verified),
            CurrentDeviceTrustState::Verified
        );
        assert_eq!(
            map_sdk_verification_state(VerificationState::Unverified),
            CurrentDeviceTrustState::Unverified
        );
    }
    #[test]
    fn verification_method_discovery_distinguishes_identity_facts() {
        let existing_with_sas = map_verification_method_facts(VerificationMethodFacts {
            identity: IdentityFact::Existing,
            verified_other_device_count: 2,
            recovery: RecoveryFact::Unavailable,
        });
        assert_eq!(
            existing_with_sas.account_kind,
            VerificationAccountKind::ExistingIdentity
        );
        assert_eq!(
            existing_with_sas.methods,
            vec![VerificationMethodCapability::ExistingDeviceSas]
        );

        let new_identity = map_verification_method_facts(VerificationMethodFacts {
            identity: IdentityFact::Missing,
            verified_other_device_count: 0,
            recovery: RecoveryFact::Unavailable,
        });
        assert_eq!(
            new_identity.account_kind,
            VerificationAccountKind::NewIdentity
        );
        assert_eq!(
            new_identity.methods,
            vec![VerificationMethodCapability::Bootstrap]
        );

        let unknown = map_verification_method_facts(VerificationMethodFacts {
            identity: IdentityFact::Unknown,
            verified_other_device_count: 0,
            recovery: RecoveryFact::Available,
        });
        assert_eq!(unknown.account_kind, VerificationAccountKind::Unknown);
        assert!(unknown.methods.is_empty());

        let existing_with_recovery = map_verification_method_facts(VerificationMethodFacts {
            identity: IdentityFact::Existing,
            verified_other_device_count: 0,
            recovery: RecoveryFact::Available,
        });
        assert_eq!(
            existing_with_recovery.methods,
            vec![
                VerificationMethodCapability::RecoveryKey,
                VerificationMethodCapability::SecurityPhrase,
            ]
        );

        let existing_without_proof = map_verification_method_facts(VerificationMethodFacts {
            identity: IdentityFact::Existing,
            verified_other_device_count: 0,
            recovery: RecoveryFact::Unavailable,
        });
        assert_eq!(
            existing_without_proof.account_kind,
            VerificationAccountKind::ExistingIdentity
        );
        assert!(existing_without_proof.methods.is_empty());

        let sas_survives_unknown_recovery =
            map_verification_method_facts(VerificationMethodFacts {
                identity: IdentityFact::Existing,
                verified_other_device_count: 1,
                recovery: RecoveryFact::Unknown,
            });
        assert_eq!(
            sas_survives_unknown_recovery.methods,
            vec![VerificationMethodCapability::ExistingDeviceSas]
        );

        let unknown_without_known_proof = map_verification_method_facts(VerificationMethodFacts {
            identity: IdentityFact::Existing,
            verified_other_device_count: 0,
            recovery: RecoveryFact::Unknown,
        });
        assert_eq!(
            unknown_without_known_proof.account_kind,
            VerificationAccountKind::Unknown
        );
    }
    #[test]
    fn own_user_proof_eligibility_requires_distinct_owner_signed_unblocked_device() {
        assert!(super::is_eligible_own_user_proof_device(
            "CURRENT", "OTHER", true, false
        ));
        assert!(!super::is_eligible_own_user_proof_device(
            "CURRENT", "CURRENT", true, false
        ));
        assert!(!super::is_eligible_own_user_proof_device(
            "CURRENT", "OTHER", false, false
        ));
        assert!(!super::is_eligible_own_user_proof_device(
            "CURRENT", "OTHER", true, true
        ));
    }
    #[test]
    fn own_user_request_recipient_requires_a_distinct_owner_signed_device() {
        assert!(super::is_own_user_verification_recipient(
            "CURRENT", "OTHER", true
        ));
        assert!(!super::is_own_user_verification_recipient(
            "CURRENT", "CURRENT", true
        ));
        assert!(!super::is_own_user_verification_recipient(
            "CURRENT", "OTHER", false
        ));
    }
    #[test]
    fn own_user_sas_recipient_diagnostics_distinguish_sender_and_interactive_targets() {
        use super::OwnUserSasDeviceFact as Fact;

        let diagnostics = super::own_user_sas_recipient_diagnostics([
            Fact {
                is_current: true,
                cross_signed_by_owner: false,
                blocked: false,
                dehydrated: false,
                curve_key_present: true,
                ed25519_key_present: true,
            },
            Fact {
                is_current: false,
                cross_signed_by_owner: true,
                blocked: false,
                dehydrated: false,
                curve_key_present: true,
                ed25519_key_present: true,
            },
            Fact {
                is_current: false,
                cross_signed_by_owner: true,
                blocked: false,
                dehydrated: true,
                curve_key_present: true,
                ed25519_key_present: true,
            },
            Fact {
                is_current: false,
                cross_signed_by_owner: true,
                blocked: true,
                dehydrated: false,
                curve_key_present: false,
                ed25519_key_present: true,
            },
            Fact {
                is_current: false,
                cross_signed_by_owner: false,
                blocked: false,
                dehydrated: false,
                curve_key_present: true,
                ed25519_key_present: true,
            },
        ]);

        assert!(diagnostics.sender_device_query_visible);
        assert!(diagnostics.sender_curve_key_present);
        assert!(diagnostics.sender_ed25519_key_present);
        assert_eq!(diagnostics.other_device_count, 4);
        assert_eq!(diagnostics.recipient_count, 3);
        assert_eq!(diagnostics.eligible_device_count, 2);
        assert_eq!(diagnostics.interactive_recipient_count, 1);
        assert_eq!(diagnostics.dehydrated_recipient_count, 1);
    }
    #[test]
    fn sas_delivery_event_contains_only_closed_private_safe_fields() {
        let event = super::sas_delivery_event("recipients_resolved", 41)
            .field(koushi_diagnostics::DiagnosticField::count(
                "other_device_count",
                3,
            ))
            .field(koushi_diagnostics::DiagnosticField::count(
                "recipient_count",
                1,
            ));
        assert_eq!(event.source, "core.sas_verification");
        assert_eq!(
            koushi_diagnostics::format_event(&event),
            "stage=recipients_resolved flow_id=41 other_device_count=3 recipient_count=1"
        );
    }
    #[test]
    fn sas_delivery_waiting_event_identifies_private_safe_wait_state() {
        let event = super::sas_delivery_waiting_event(43, "to_device_delivery");

        assert_eq!(
            koushi_diagnostics::format_event(&event),
            "stage=waiting flow_id=43 waiting_for=to_device_delivery"
        );
    }
    #[test]
    fn sas_recipients_resolved_event_includes_sender_readiness_without_identifiers() {
        let event = super::sas_recipients_resolved_event(
            42,
            super::OwnUserSasRecipientDiagnostics {
                other_device_count: 9,
                recipient_count: 6,
                eligible_device_count: 6,
                sender_device_query_visible: true,
                sender_curve_key_present: true,
                sender_ed25519_key_present: true,
                interactive_recipient_count: 5,
                dehydrated_recipient_count: 1,
            },
        );

        assert_eq!(
            koushi_diagnostics::format_event(&event),
            "stage=recipients_resolved flow_id=42 other_device_count=9 recipient_count=6 eligible_device_count=6 sender_device_query_visible=true sender_curve_key_present=true sender_ed25519_key_present=true interactive_recipient_count=5 dehydrated_recipient_count=1"
        );
    }
    #[test]
    fn verification_cancel_codes_map_to_closed_private_safe_categories() {
        use super::MatrixVerificationCancelKind as Kind;

        assert_eq!(
            super::map_verification_cancel_kind("m.unknown_method"),
            Kind::UnknownMethod
        );
        assert_eq!(
            super::map_verification_cancel_kind("m.key_mismatch"),
            Kind::KeyMismatch
        );
        assert_eq!(super::map_verification_cancel_kind("m.user"), Kind::User);
        assert_eq!(
            super::map_verification_cancel_kind("m.timeout"),
            Kind::Timeout
        );
        assert_eq!(
            super::map_verification_cancel_kind("m.accepted"),
            Kind::AcceptedElsewhere
        );
        assert_eq!(
            super::map_verification_cancel_kind("m.future_code"),
            Kind::Other
        );
    }
    #[test]
    fn sas_cancellation_maps_to_closed_private_safe_projection() {
        use super::{MatrixSasState as SasState, MatrixVerificationCancelKind as CancelKind};

        let state = super::map_sas_cancellation("m.timeout", false);

        assert_eq!(
            state,
            SasState::Cancelled {
                kind: CancelKind::Timeout,
                cancelled_by_us: false,
            }
        );
        let debug = format!("{state:?}");
        assert_eq!(debug, "Cancelled { kind: Timeout, cancelled_by_us: false }");
        assert!(!debug.contains("m.timeout"));

        let unknown = super::map_sas_cancellation("m.future_private_code", true);
        assert_eq!(
            unknown,
            SasState::Cancelled {
                kind: CancelKind::Other,
                cancelled_by_us: true,
            }
        );
        assert!(!format!("{unknown:?}").contains("future_private_code"));
    }
    #[test]
    fn own_user_sas_api_returns_only_an_opaque_adapter_handle() {
        let _ = super::request_own_user_sas_verification;
        let _opaque: Option<super::MatrixOwnUserVerificationHandle> = None;
        assert!(!std::any::type_name::<super::MatrixOwnUserVerificationHandle>().contains('@'));
    }
    #[test]
    fn key_backup_state_maps_to_private_data_free_desktop_status() {
        assert_eq!(
            map_backup_state_to_desktop(BackupState::Unknown),
            KeyBackupStatus::Unknown
        );
        assert_eq!(
            map_backup_state_to_desktop(BackupState::Enabled),
            KeyBackupStatus::Enabled {
                version: "available".to_owned(),
            }
        );
        assert_eq!(
            map_backup_state_to_desktop(BackupState::Downloading),
            KeyBackupStatus::Restoring {
                request_id: 0,
                version: None,
                restored_rooms: 0,
                total_rooms: None,
            }
        );
    }
    #[test]
    fn e2ee_trust_error_debug_redacts_sdk_details() {
        let error = E2eeTrustError::Sdk("raw matrix sdk error with @alice:example.test".to_owned());
        let debug = format!("{error:?}");

        assert!(!debug.contains("@alice:example.test"));
        assert!(!debug.contains("raw matrix sdk error"));
        assert!(debug.contains("Sdk"));
    }
    #[test]
    fn key_backup_restore_summary_declares_joined_room_scope() {
        let summary = KeyBackupRestoreSummary {
            scope: KeyBackupRestoreScope::JoinedRooms,
            version: Some("available".to_owned()),
            restored_rooms: 2,
            total_rooms: Some(3),
        };

        let debug = format!("{summary:?}");
        assert!(debug.contains("JoinedRooms"));
        assert!(!debug.contains("BackupWide"));
        assert!(!debug.contains("AllRooms"));
    }
    #[test]
    fn room_key_file_transfer_summaries_are_private_data_free() {
        let export_summary = RoomKeyExportSummary {
            exported_sessions: None,
        };
        let import_summary = RoomKeyImportSummary {
            imported_count: 1,
            total_count: 1,
        };

        assert_eq!(export_summary.exported_sessions, None);
        assert_eq!(import_summary.imported_count, 1);
        assert_eq!(import_summary.total_count, 1);
        assert!(!format!("{export_summary:?}").contains("MEGOLM"));
        assert!(!format!("{import_summary:?}").contains("MEGOLM"));
    }
    #[test]
    fn secure_backup_setup_summary_is_private_data_free() {
        let summary = SecureBackupSetupSummary {
            recovery_key_written: true,
        };

        let debug = format!("{summary:?}");
        assert!(debug.contains("recovery_key_written"));
        assert!(!debug.contains("RecoveryKey("));
    }
    #[test]
    fn recovery_key_delivery_writes_native_artifact_without_debugging_material() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("recovery-artifact.txt");
        let artifact_payload = String::from("fixture-artifact-material");

        let written = write_recovery_key_if_requested(artifact_payload.clone(), Some(path.clone()))
            .expect("artifact write should succeed");

        assert!(written);
        assert_eq!(
            std::fs::read_to_string(&path).expect("read artifact"),
            artifact_payload
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(path)
                    .expect("artifact metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
    #[test]
    fn recovery_key_delivery_refuses_to_overwrite_an_existing_artifact() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("existing-artifact.txt");
        std::fs::write(&path, "keep-me").expect("write existing artifact");

        let error = write_recovery_key_if_requested(
            "fixture-artifact-material".to_owned(),
            Some(path.clone()),
        )
        .expect_err("existing artifact must not be overwritten");

        assert_eq!(error, E2eeTrustError::SecureBackupRecoveryKeyDeliveryFailed);
        assert_eq!(
            std::fs::read_to_string(path).expect("read artifact"),
            "keep-me"
        );
    }
    #[tokio::test]
    async fn room_key_import_accepts_element_compatible_key_export_envelope() {
        assert!(ELEMENT_COMPATIBLE_KEY_EXPORT.starts_with(MATRIX_KEY_EXPORT_HEADER));
        assert!(ELEMENT_COMPATIBLE_KEY_EXPORT.ends_with(MATRIX_KEY_EXPORT_FOOTER));

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("element-compatible-room-keys.txt");
        std::fs::write(&path, ELEMENT_COMPATIBLE_KEY_EXPORT).expect("write fixture");
        let persistable = PersistableMatrixSession::from_json(
            r#"{"homeserver":"https://matrix.example.invalid","user_id":"@alice:example.invalid","device_id":"ALICEDEVICE","access_token":"synthetic-access"}"#,
        )
        .expect("synthetic session should deserialize");
        let session = restore_session(&persistable)
            .await
            .expect("synthetic session should restore");

        let summary = import_room_keys_from_file(&session, path, &AuthSecret::new("1234"))
            .await
            .expect("Matrix/Element key export envelope should import");

        assert_eq!(summary.total_count, 1);
    }
    #[test]
    fn e2ee_trust_public_async_api_is_exposed() {
        let _ = cross_signing_status;
        let _ = bootstrap_cross_signing;
        let _ = enable_key_backup;
        let _ = restore_key_backup;
        let _ = reset_identity;
        let _ = complete_identity_reset;
        let _ = request_device_verification;
        let _ = accept_verification_request;
        let _ = start_sas_verification;
        let _ = accept_sas_verification;
        let _ = confirm_sas_verification;
        let _ = mismatch_sas_verification;
        let _ = cancel_verification_request;
        let _ = cancel_sas_verification;
        let _ = observe_incoming_verification_requests;
        let _ = export_room_keys_to_file;
        let _ = import_room_keys_from_file;
        let _ = bootstrap_secure_backup;
        let _ = change_secure_backup_passphrase;
        let _: Option<MatrixIncomingVerificationRequest> = None;
        let _: Option<MatrixIncomingVerificationRequestObserver> = None;
    }
    #[test]
    fn sas_emojis_map_to_desktop_dto_without_sdk_types() {
        let emojis = [
            matrix_sdk::encryption::verification::Emoji {
                symbol: "🐶",
                description: "Dog",
            },
            matrix_sdk::encryption::verification::Emoji {
                symbol: "🐱",
                description: "Cat",
            },
            matrix_sdk::encryption::verification::Emoji {
                symbol: "🦁",
                description: "Lion",
            },
            matrix_sdk::encryption::verification::Emoji {
                symbol: "🐎",
                description: "Horse",
            },
            matrix_sdk::encryption::verification::Emoji {
                symbol: "🦄",
                description: "Unicorn",
            },
            matrix_sdk::encryption::verification::Emoji {
                symbol: "🐷",
                description: "Pig",
            },
            matrix_sdk::encryption::verification::Emoji {
                symbol: "🐘",
                description: "Elephant",
            },
        ];

        assert_eq!(
            map_sdk_sas_emojis_to_desktop(emojis),
            vec![
                SasEmoji {
                    symbol: "🐶".to_owned(),
                    description: "Dog".to_owned(),
                },
                SasEmoji {
                    symbol: "🐱".to_owned(),
                    description: "Cat".to_owned(),
                },
                SasEmoji {
                    symbol: "🦁".to_owned(),
                    description: "Lion".to_owned(),
                },
                SasEmoji {
                    symbol: "🐎".to_owned(),
                    description: "Horse".to_owned(),
                },
                SasEmoji {
                    symbol: "🦄".to_owned(),
                    description: "Unicorn".to_owned(),
                },
                SasEmoji {
                    symbol: "🐷".to_owned(),
                    description: "Pig".to_owned(),
                },
                SasEmoji {
                    symbol: "🐘".to_owned(),
                    description: "Elephant".to_owned(),
                },
            ]
        );
    }
    #[test]
    fn identity_reset_auth_type_maps_to_private_data_free_desktop_status() {
        assert_eq!(
            map_identity_reset_auth_type_to_desktop(MatrixIdentityResetAuthType::Uiaa),
            IdentityResetAuthType::Uiaa
        );
        assert_eq!(
            map_identity_reset_auth_type_to_desktop(MatrixIdentityResetAuthType::OAuth),
            IdentityResetAuthType::OAuth
        );
    }
}

pub(super) const DESKTOP_SQLITE_STORE_POOL_MAX_SIZE: usize = 4;

impl MatrixClientSession {
    pub fn e2ee_recovery_state(&self) -> E2eeRecoveryState {
        map_sdk_recovery_state(self.client().encryption().recovery().state())
    }
    pub fn e2ee_recovery_state_stream(&self) -> E2eeRecoveryStateStream {
        Box::pin(
            self.client()
                .encryption()
                .recovery()
                .state_stream()
                .map(map_sdk_recovery_state),
        )
    }
    /// Update the SDK send-queue admission latch for encrypted events. The
    /// durability fence remains enabled independently for the whole session.
    pub fn set_secure_backup_send_admitted(&self, admitted: bool) {
        self.client()
            .send_queue()
            .set_secure_backup_send_admitted(admitted);
    }
    pub fn observe_secure_backup_state(&self) -> MatrixSecureBackupStateObservation {
        let encryption = self.client().encryption();
        let backups = encryption.backups();
        let recovery = encryption.recovery();
        let current = MatrixSecureBackupState {
            backup: map_secure_backup_local_state(backups.state()),
            recovery: map_secure_backup_recovery_state(recovery.state()),
        };

        let backup_updates = backups.state_stream().map(|state| {
            SecureBackupStateUpdate::Backup(
                state
                    .map(map_secure_backup_local_state)
                    .unwrap_or(MatrixSecureBackupLocalState::Unknown),
            )
        });
        let recovery_updates = recovery.state_stream().map(|state| {
            SecureBackupStateUpdate::Recovery(map_secure_backup_recovery_state(state))
        });
        let updates = stream::select(backup_updates, recovery_updates).scan(
            (current.backup, current.recovery),
            |state, update| {
                match update {
                    SecureBackupStateUpdate::Backup(backup) => state.0 = backup,
                    SecureBackupStateUpdate::Recovery(recovery) => state.1 = recovery,
                }
                futures_util::future::ready(Some(MatrixSecureBackupState {
                    backup: state.0,
                    recovery: state.1,
                }))
            },
        );

        MatrixSecureBackupStateObservation {
            current,
            updates: Box::pin(updates),
        }
    }
    pub fn current_device_trust(&self) -> CurrentDeviceTrustState {
        let subscriber = self.client().encryption().verification_state();
        map_sdk_verification_state(subscriber.get())
    }
    pub async fn recheck_current_device_trust(
        &self,
    ) -> Result<CurrentDeviceTrustState, CurrentDeviceTrustRecheckError> {
        // Subscribe before the request so the returned value belongs to the
        // same observation that sees the own-user keys-query settlement.
        let subscriber = self.client().encryption().verification_state();
        let client = self.client();
        let Some(user_id) = client.user_id() else {
            let error = CurrentDeviceTrustRecheckError::Sdk;
            record_current_device_trust_recheck_finished("failed", Some(error));
            return Err(error);
        };
        match client.encryption().request_user_identity(user_id).await {
            Ok(_) => {
                record_current_device_trust_recheck_finished("success", None);
                Ok(map_sdk_verification_state(subscriber.get()))
            }
            Err(error) => {
                let error = classify_current_device_trust_recheck_error(&error);
                record_current_device_trust_recheck_finished("failed", Some(error));
                Err(error)
            }
        }
    }
    pub async fn inspect_current_session(
        &self,
    ) -> Result<MatrixCurrentSessionInspection, MatrixCurrentSessionInspectionError> {
        let client = self.client();
        let verification = client.encryption().verification_state();
        let user_id = client
            .user_id()
            .ok_or(MatrixCurrentSessionInspectionError::Unavailable)?;
        let device_id = client
            .device_id()
            .ok_or(MatrixCurrentSessionInspectionError::Unavailable)?;

        let devices = client
            .devices()
            .await
            .map_err(|_| MatrixCurrentSessionInspectionError::DeviceRequest)?;
        let current_device = devices
            .devices
            .into_iter()
            .find(|device| device.device_id == device_id)
            .ok_or(MatrixCurrentSessionInspectionError::CurrentDeviceMissing)?;

        let encryption = client.encryption();
        let own_identity = encryption
            .request_user_identity(user_id)
            .await
            .map_err(|_| MatrixCurrentSessionInspectionError::IdentityRequest)?;
        let current_crypto_device = encryption
            .get_device(user_id, device_id)
            .await
            .map_err(|_| MatrixCurrentSessionInspectionError::IdentityRequest)?;
        let is_cross_signed_by_owner = current_crypto_device
            .as_ref()
            .is_some_and(|device| device.is_cross_signed_by_owner());
        let own_identity_verification = classify_own_identity_verification(
            own_identity.is_some(),
            own_identity
                .as_ref()
                .is_some_and(|identity| identity.is_verified()),
        );

        let backups = encryption.backups();
        let local_backup_state = backups.state();
        let server_probe =
            if local_backup_state == matrix_sdk::encryption::backups::BackupState::Enabled {
                Ok(true)
            } else {
                backups.fetch_exists_on_server().await.map_err(|_| ())
            };

        Ok(MatrixCurrentSessionInspection {
            device_display_name: current_device.display_name,
            verification: map_sdk_verification_state(verification.get()),
            is_cross_signed_by_owner,
            own_identity_verification,
            key_backup: classify_current_session_backup(local_backup_state, server_probe),
        })
    }
    pub async fn inspect_secure_backup(
        &self,
    ) -> Result<MatrixSecureBackupInspection, E2eeTrustError> {
        let encryption = self.client().encryption();
        let backups = encryption.backups();
        let (server, trust) = match backups.inspect_server_trust().await {
            Ok(matrix_sdk::encryption::backups::ServerBackupTrust::Absent) => (
                MatrixSecureBackupServerState::Absent,
                MatrixSecureBackupTrustState::Unknown,
            ),
            Ok(matrix_sdk::encryption::backups::ServerBackupTrust::Trusted) => (
                MatrixSecureBackupServerState::Present,
                MatrixSecureBackupTrustState::Trusted,
            ),
            Ok(matrix_sdk::encryption::backups::ServerBackupTrust::Mismatch) => (
                MatrixSecureBackupServerState::Present,
                MatrixSecureBackupTrustState::Mismatch,
            ),
            Ok(
                matrix_sdk::encryption::backups::ServerBackupTrust::MissingLocalKey
                | matrix_sdk::encryption::backups::ServerBackupTrust::Untrusted,
            ) => (
                MatrixSecureBackupServerState::Present,
                MatrixSecureBackupTrustState::Unknown,
            ),
            Err(_) => (
                MatrixSecureBackupServerState::Unknown,
                MatrixSecureBackupTrustState::Unknown,
            ),
        };
        let local_sdk_state = backups.state();
        let local = map_secure_backup_local_state(local_sdk_state);
        let recovery_api = encryption.recovery();
        let recovery = match map_secure_backup_recovery_state(recovery_api.state()) {
            MatrixSecureBackupRecoveryState::Disabled => {
                match recovery_api.is_explicitly_disabled().await {
                    Ok(true) => MatrixSecureBackupRecoveryState::Disabled,
                    Ok(false) | Err(_) => MatrixSecureBackupRecoveryState::Unknown,
                }
            }
            state => state,
        };
        let upload = if local_sdk_state == matrix_sdk::encryption::backups::BackupState::Enabled
            && trust == MatrixSecureBackupTrustState::Trusted
        {
            classify_secure_backup_upload(
                backups.room_key_counts().await.map_err(|_| ()),
                backups.upload_state(),
            )
        } else {
            MatrixSecureBackupUploadState::Unknown
        };
        Ok(MatrixSecureBackupInspection {
            server,
            local,
            recovery,
            upload,
            trust,
            recovery_key_delivery_pending: self.recovery_key_delivery_pending().await?,
        })
    }
    pub async fn recover_secure_backup(
        &self,
        request: &RecoveryRequest,
    ) -> Result<(), E2eeTrustError> {
        self.client()
            .encryption()
            .recovery()
            .recover(request.secret.expose_secret())
            .await?;
        self.wait_for_secure_backup_steady_state().await
    }
    pub async fn setup_secure_backup(
        &self,
        passphrase: Option<&AuthSecret>,
        recovery_key_destination_path: Option<PathBuf>,
    ) -> Result<SecureBackupSetupSummary, E2eeTrustError> {
        self.setup_secure_backup_with_confirmation(passphrase, recovery_key_destination_path, false)
            .await
    }
    pub async fn reenable_secure_backup(
        &self,
        passphrase: Option<&AuthSecret>,
        recovery_key_destination_path: Option<PathBuf>,
    ) -> Result<SecureBackupSetupSummary, E2eeTrustError> {
        self.setup_secure_backup_with_confirmation(passphrase, recovery_key_destination_path, true)
            .await
    }
    async fn setup_secure_backup_with_confirmation(
        &self,
        passphrase: Option<&AuthSecret>,
        recovery_key_destination_path: Option<PathBuf>,
        explicit_reenable_confirmed: bool,
    ) -> Result<SecureBackupSetupSummary, E2eeTrustError> {
        let inspection = self.inspect_secure_backup().await?;
        if inspection.recommended_gate_state()
            == SecureBackupGateState::ExplicitlyDisabledRequiresSetup
            && !explicit_reenable_confirmed
        {
            return Err(E2eeTrustError::SecureBackupReenableConfirmationRequired);
        }
        match inspection.server {
            MatrixSecureBackupServerState::Absent => {}
            MatrixSecureBackupServerState::Present => {
                if inspection.local == MatrixSecureBackupLocalState::Enabled
                    && inspection.trust == MatrixSecureBackupTrustState::Trusted
                {
                    let recovery_key = self
                        .client()
                        .encryption()
                        .backups()
                        .local_recovery_key()
                        .await?
                        .ok_or(E2eeTrustError::SecureBackupInspectionInconclusive)?;
                    let summary = SecureBackupSetupSummary {
                        recovery_key_written: write_recovery_key_material(
                            &recovery_key,
                            recovery_key_destination_path,
                        )?,
                    };
                    self.set_recovery_key_delivery_pending(false).await?;
                    return Ok(summary);
                }
                return Err(E2eeTrustError::SecureBackupAlreadyExists);
            }
            MatrixSecureBackupServerState::Unknown => {
                return Err(E2eeTrustError::SecureBackupInspectionInconclusive);
            }
        }

        self.set_recovery_key_delivery_pending(true).await?;
        let summary =
            bootstrap_secure_backup(self, passphrase, recovery_key_destination_path).await?;
        self.set_recovery_key_delivery_pending(false).await?;
        self.wait_for_secure_backup_steady_state().await?;
        Ok(summary)
    }
    async fn recovery_key_delivery_pending(&self) -> Result<bool, E2eeTrustError> {
        const KEY: &[u8] = b"koushi.secure_backup.recovery_key_delivery_pending.v1";
        self.client()
            .state_store()
            .get_custom_value(KEY)
            .await
            .map(|value| value.as_deref() == Some(b"1"))
            .map_err(|_| E2eeTrustError::SecureBackupInspectionInconclusive)
    }
    async fn set_recovery_key_delivery_pending(&self, pending: bool) -> Result<(), E2eeTrustError> {
        const KEY: &[u8] = b"koushi.secure_backup.recovery_key_delivery_pending.v1";
        let client = self.client();
        let store = client.state_store();
        if pending {
            store.set_custom_value(KEY, b"1".to_vec()).await.map(|_| ())
        } else {
            store.remove_custom_value(KEY).await.map(|_| ())
        }
        .map_err(|_| E2eeTrustError::SecureBackupInspectionInconclusive)
    }
    pub async fn wait_for_secure_backup_steady_state(&self) -> Result<(), E2eeTrustError> {
        self.client()
            .encryption()
            .backups()
            .wait_for_steady_state()
            .await
            .map_err(|_| E2eeTrustError::SecureBackupUploadFailed)
    }
    pub fn observe_current_device_trust(&self) -> CurrentDeviceTrustObservation {
        // Subscribe first, then read from the same subscriber so an update
        // cannot be lost between the current-value probe and stream creation.
        let subscriber = self.client().encryption().verification_state();
        let current = map_sdk_verification_state(subscriber.get());
        let updates = Box::pin(subscriber.map(map_sdk_verification_state));
        CurrentDeviceTrustObservation { current, updates }
    }
}

#[cfg(test)]
mod current_device_trust_recheck_tests {
    use matrix_sdk::test_utils::mocks::MatrixMockServer;

    use super::{CurrentDeviceTrustState, MatrixClientSession, SessionInfo};

    #[tokio::test]
    async fn recheck_current_device_trust_queries_own_identity() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
            .records
            .len();
        let server = MatrixMockServer::new().await;
        let client = server.client_builder().build().await;
        let info = SessionInfo {
            homeserver: server.server().uri(),
            user_id: client
                .user_id()
                .expect("mock client has a user id")
                .to_string(),
            device_id: client
                .device_id()
                .expect("mock client has a device id")
                .to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        };
        let session = MatrixClientSession::from_client_for_testing(client, info);
        let _query = server
            .mock_query_keys()
            .ok()
            .expect(1)
            .named("authoritative current-device trust recheck")
            .mount_as_scoped()
            .await;

        let trust = session
            .recheck_current_device_trust()
            .await
            .expect("empty authoritative response still settles");

        assert_eq!(trust, CurrentDeviceTrustState::Unverified);
        assert!(
            koushi_diagnostics::test_support::detail_snapshot().records[diagnostic_start..]
                .iter()
                .any(|record| {
                    koushi_diagnostics::format_event(&record.event)
                        == "stage=finished outcome=success"
                        && record.event.source == "sdk.current_device_trust_recheck"
                }),
            "successful trust rechecks must record a closed completion diagnostic"
        );
    }
}

#[cfg(test)]
mod current_session_status_tests {
    use matrix_sdk::{
        encryption::backups::BackupState,
        ruma::{CanonicalJsonValue, owned_user_id},
        test_utils::mocks::MatrixMockServer,
    };
    use matrix_sdk_test::{
        ruma_response_to_json, test_json::keys_query_sets::KeyQueryResponseTemplate,
    };
    use serde_json::json;
    use vodozemac::Ed25519SecretKey;
    use wiremock::{
        Mock, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    use super::{
        CurrentDeviceTrustState, CurrentSessionBackupState, MatrixClientSession,
        MatrixCurrentSessionInspectionError, MatrixDeviceNameOutcome, OwnIdentityVerification,
        SessionInfo, classify_current_session_backup, classify_own_identity_verification,
        ensure_device_display_name,
    };

    async fn session(server: &MatrixMockServer) -> MatrixClientSession {
        let client = server.client_builder().build().await;
        let info = SessionInfo {
            homeserver: server.server().uri(),
            user_id: client.user_id().expect("mock user id").to_string(),
            device_id: client.device_id().expect("mock device id").to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::OAuth,
        };
        MatrixClientSession::from_client_for_testing(client, info)
    }

    async fn mount_device(
        server: &MatrixMockServer,
        display_name: Option<&str>,
    ) -> wiremock::MockGuard {
        server
            .mock_devices()
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "devices": [{
                    "device_id": "DEVICEID",
                    "display_name": display_name,
                    "last_seen_ip": "private.invalid",
                    "last_seen_ts": 1_u64,
                    "user_id": "@example:localhost"
                }]
            })))
            .expect(1)
            .mount_as_scoped()
            .await
    }

    fn sign_json_for_test(
        value: &mut serde_json::Value,
        signing_key: &Ed25519SecretKey,
        user_id: &str,
        key_identifier: &str,
    ) {
        let mut unsigned = value.clone();
        let object = unsigned.as_object_mut().expect("device JSON object");
        object.remove("signatures");
        object.remove("unsigned");
        let canonical: CanonicalJsonValue = unsigned.try_into().expect("canonical device JSON");
        let signature = signing_key.sign(canonical.to_string().as_bytes());
        value["signatures"][user_id][format!("ed25519:{key_identifier}")] =
            signature.to_base64().into();
    }

    #[tokio::test]
    async fn current_session_status_finds_current_device_display_name() {
        let server = MatrixMockServer::new().await;
        let session = session(&server).await;
        let _devices = mount_device(&server, Some("Koushi Workstation")).await;
        let _identity = server
            .mock_query_keys()
            .ok()
            .expect(1)
            .mount_as_scoped()
            .await;
        let _backup = server
            .mock_room_keys_version()
            .none()
            .expect(1)
            .mount_as_scoped()
            .await;

        let status = session
            .inspect_current_session()
            .await
            .expect("authoritative inspection");

        assert_eq!(
            status.device_display_name.as_deref(),
            Some("Koushi Workstation")
        );
        assert!(!status.is_cross_signed_by_owner);
        assert_eq!(
            status.own_identity_verification,
            OwnIdentityVerification::Missing
        );
        assert_eq!(status.key_backup, CurrentSessionBackupState::Disabled);
        assert!(!format!("{status:?}").contains("Koushi Workstation"));
        let serialized = serde_json::to_string(&status).expect("serialize coarse status");
        assert!(!serialized.contains("1234"));
        assert!(!serialized.contains("private.invalid"));
    }

    #[tokio::test]
    async fn current_session_status_rejects_an_absent_current_device() {
        let server = MatrixMockServer::new().await;
        let session = session(&server).await;
        let _devices = server
            .mock_devices()
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "devices": [] })))
            .expect(1)
            .mount_as_scoped()
            .await;

        assert_eq!(
            session.inspect_current_session().await,
            Err(MatrixCurrentSessionInspectionError::CurrentDeviceMissing)
        );
    }

    #[tokio::test]
    async fn current_session_status_maps_device_and_identity_failures_coarsely() {
        let device_server = MatrixMockServer::new().await;
        let device_session = session(&device_server).await;
        let _devices = device_server
            .mock_devices()
            .error500()
            .expect(1)
            .mount_as_scoped()
            .await;
        assert_eq!(
            device_session.inspect_current_session().await,
            Err(MatrixCurrentSessionInspectionError::DeviceRequest)
        );

        let identity_server = MatrixMockServer::new().await;
        let identity_session = session(&identity_server).await;
        let _devices = mount_device(&identity_server, None).await;
        let _identity = identity_server
            .mock_query_keys()
            .error500()
            .expect(1)
            .mount_as_scoped()
            .await;
        assert_eq!(
            identity_session.inspect_current_session().await,
            Err(MatrixCurrentSessionInspectionError::IdentityRequest)
        );
    }

    #[tokio::test]
    async fn current_session_status_reads_owner_cross_signing_and_unverified_own_identity() {
        let server = MatrixMockServer::new().await;
        let session = session(&server).await;
        let _devices = mount_device(&server, Some("Signed device")).await;
        let client = session.client();
        let user_id = client.user_id().expect("mock user id");
        let device_id = client.device_id().expect("mock device id");
        let current_device = client
            .encryption()
            .get_device(user_id, device_id)
            .await
            .expect("read current device")
            .expect("mock client stores its own device");
        let self_signing_key = Ed25519SecretKey::from_slice(b"self1234self1234self1234self1234");
        let response = KeyQueryResponseTemplate::new(owned_user_id!("@example:localhost"))
            .with_cross_signing_keys(
                Ed25519SecretKey::from_slice(b"master12master12master12master12"),
                Ed25519SecretKey::from_slice(b"self1234self1234self1234self1234"),
                Ed25519SecretKey::from_slice(b"user1234user1234user1234user1234"),
            )
            .build_response();
        let mut response_json = ruma_response_to_json(response);
        let device_keys = current_device
            .keys()
            .iter()
            .map(|(key_id, key)| (key_id.to_string(), key.to_base64()))
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut current_device_json = json!({
            "user_id": user_id,
            "device_id": device_id,
            "algorithms": current_device.algorithms(),
            "keys": device_keys,
            "signatures": current_device.signatures(),
        });
        sign_json_for_test(
            &mut current_device_json,
            &self_signing_key,
            user_id.as_str(),
            &self_signing_key.public_key().to_base64(),
        );
        response_json["device_keys"][user_id.as_str()][device_id.as_str()] = current_device_json;
        let _identity = server
            .mock_query_keys()
            .respond_with(ResponseTemplate::new(200).set_body_json(response_json))
            .expect(1)
            .mount_as_scoped()
            .await;
        let _backup = server
            .mock_room_keys_version()
            .error500()
            .expect(1)
            .mount_as_scoped()
            .await;

        let status = session
            .inspect_current_session()
            .await
            .expect("authoritative inspection");

        assert_eq!(
            session.current_device_trust(),
            CurrentDeviceTrustState::Verified,
            "the SDK current-device verdict is authoritative even while own-identity verification is supplemental"
        );
        assert!(status.is_cross_signed_by_owner);
        assert_eq!(
            status.own_identity_verification,
            OwnIdentityVerification::Unverified
        );
        assert_eq!(status.key_backup, CurrentSessionBackupState::Unknown);
    }

    #[tokio::test]
    async fn oauth_device_name_renames_only_an_empty_authoritative_name() {
        let server = MatrixMockServer::new().await;
        let session = session(&server).await;
        let _devices = mount_device(&server, Some("   ")).await;
        let _rename = Mock::given(method("PUT"))
            .and(path("/_matrix/client/v3/devices/DEVICEID"))
            .and(body_json(json!({ "display_name": "Koushi on Linux" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount_as_scoped(server.server())
            .await;

        assert_eq!(
            ensure_device_display_name(&session, "Koushi on Linux").await,
            MatrixDeviceNameOutcome::Renamed
        );
    }

    #[tokio::test]
    async fn oauth_device_name_preserves_existing_name_and_maps_failures_coarsely() {
        let named_server = MatrixMockServer::new().await;
        let named_session = session(&named_server).await;
        let _devices = mount_device(&named_server, Some("Custom device")).await;
        assert_eq!(
            ensure_device_display_name(&named_session, "Koushi on Linux").await,
            MatrixDeviceNameOutcome::Present
        );
        assert!(
            named_server
                .received_requests()
                .await
                .expect("request history")
                .iter()
                .all(|request| request.method.as_str() != "PUT")
        );

        let failed_server = MatrixMockServer::new().await;
        let failed_session = session(&failed_server).await;
        let _devices = mount_device(&failed_server, None).await;
        let _rename = Mock::given(method("PUT"))
            .and(path("/_matrix/client/v3/devices/DEVICEID"))
            .respond_with(ResponseTemplate::new(500).set_body_string("private raw failure"))
            .expect(1)
            .mount_as_scoped(failed_server.server())
            .await;
        let outcome = ensure_device_display_name(&failed_session, "Koushi on Linux").await;
        assert_eq!(outcome, MatrixDeviceNameOutcome::RenameFailed);
        assert!(!format!("{outcome:?}").contains("private raw failure"));
    }

    #[test]
    fn current_session_status_classifies_identity_and_backup_without_secrets() {
        assert_eq!(
            classify_own_identity_verification(false, true),
            OwnIdentityVerification::Missing
        );
        assert_eq!(
            classify_own_identity_verification(true, false),
            OwnIdentityVerification::Unverified
        );
        assert_eq!(
            classify_own_identity_verification(true, true),
            OwnIdentityVerification::Verified
        );
        assert_eq!(
            classify_current_session_backup(BackupState::Enabled, Ok(true)),
            CurrentSessionBackupState::Ready
        );
        assert_eq!(
            classify_current_session_backup(BackupState::Unknown, Ok(false)),
            CurrentSessionBackupState::Disabled
        );
        assert_eq!(
            classify_current_session_backup(BackupState::Unknown, Err(())),
            CurrentSessionBackupState::Unknown
        );

        let error = MatrixCurrentSessionInspectionError::IdentityRequest;
        assert_eq!(
            serde_json::to_string(&error).expect("serialize coarse error"),
            "\"identity_request\""
        );
        assert!(!format!("{error:?}").contains("private"));
    }
}

#[derive(Debug, Error)]
pub enum E2eeRecoveryError {
    #[error("E2EE recovery runtime failed: {0}")]
    Runtime(String),
    #[error("E2EE recovery failed: {0}")]
    Sdk(String),
}

pub fn recover_e2ee_blocking(
    session: &MatrixClientSession,
    request: &RecoveryRequest,
) -> Result<(), E2eeRecoveryError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| E2eeRecoveryError::Runtime(error.to_string()))?;

    runtime.block_on(recover_e2ee(session, request))
}

pub async fn room_key_rotation_reason(
    session: &MatrixClientSession,
    room_id: &str,
    session_id: &str,
) -> Option<MatrixRoomKeyRotationReason> {
    let room_id = matrix_sdk::ruma::RoomId::parse(room_id).ok()?;
    session
        .client
        .encryption()
        .room_key_rotation_reason(&room_id, session_id)
        .await
        .map(map_room_key_rotation_reason)
}

fn map_room_key_rotation_reason(
    reason: matrix_sdk::encryption::RoomKeyRotationReason,
) -> MatrixRoomKeyRotationReason {
    use matrix_sdk::encryption::RoomKeyRotationReason as Reason;
    match reason {
        Reason::Initial => MatrixRoomKeyRotationReason::Initial,
        Reason::ExpiredTime => MatrixRoomKeyRotationReason::ExpiredTime,
        Reason::ExpiredMessageCount => MatrixRoomKeyRotationReason::ExpiredMessageCount,
        Reason::MembershipOrDeviceChange => MatrixRoomKeyRotationReason::MembershipOrDeviceChange,
        Reason::EncryptionSettingsChanged => MatrixRoomKeyRotationReason::EncryptionSettingsChanged,
        Reason::ExplicitDiscard => MatrixRoomKeyRotationReason::ExplicitDiscard,
        Reason::FullMemberListReload => MatrixRoomKeyRotationReason::FullMemberListReload,
        Reason::RoomSubscription => MatrixRoomKeyRotationReason::RoomSubscription,
        Reason::LimitedSyncResponse => MatrixRoomKeyRotationReason::LimitedSyncResponse,
        Reason::KeyShareFailure => MatrixRoomKeyRotationReason::KeyShareFailure,
        Reason::StoreMissing => MatrixRoomKeyRotationReason::StoreMissing,
        Reason::Invalidated => MatrixRoomKeyRotationReason::Invalidated,
        Reason::Unknown => MatrixRoomKeyRotationReason::Unknown,
    }
}

pub(super) async fn install_room_key_diagnostic_observer(
    client: &matrix_sdk::Client,
) -> Arc<DiagnosticCounterContext> {
    let counters = DiagnosticCounterContext::registered();
    koushi_diagnostics::reset_rotation_ledger();
    let observer_counters = Arc::clone(&counters);
    client
        .encryption()
        .set_room_key_diagnostic_observer(Some(Arc::new(move |event| {
            record_room_key_diagnostic(&observer_counters, event);
        })))
        .await;
    counters
}

fn record_room_key_diagnostic(
    counters: &DiagnosticCounterContext,
    event: matrix_sdk::encryption::RoomKeyDiagnosticEvent,
) {
    use matrix_sdk::encryption::RoomKeyDiagnosticEvent;

    match event {
        RoomKeyDiagnosticEvent::IncomingRequest(event) => {
            record_incoming_room_key_diagnostic(counters, event)
        }
        RoomKeyDiagnosticEvent::Rotation(event) => {
            record_room_key_rotation_diagnostic(counters, event)
        }
        RoomKeyDiagnosticEvent::MemberReload(event) => {
            record_room_key_member_reload_diagnostic(counters, event)
        }
        RoomKeyDiagnosticEvent::Receive(event) => {
            record_room_key_receive_diagnostic(counters, event)
        }
        RoomKeyDiagnosticEvent::OlmRecovery(event) => {
            record_olm_recovery_diagnostic(counters, event)
        }
        RoomKeyDiagnosticEvent::InitialShare(event) => {
            record_initial_share_diagnostic(counters, event)
        }
        RoomKeyDiagnosticEvent::InitialShareSession(event) => {
            record_initial_share_session_diagnostic(counters, event)
        }
        RoomKeyDiagnosticEvent::Index0Reshare(event) => {
            record_index0_reshare_diagnostic(counters, event)
        }
        RoomKeyDiagnosticEvent::InitialShareRepair(event) => {
            record_initial_share_repair_diagnostic(counters, event)
        }
        RoomKeyDiagnosticEvent::EncryptionReadiness(event) => {
            record_encryption_readiness_diagnostic(counters, event)
        }
    }
}

fn record_encryption_readiness_diagnostic(
    counters: &DiagnosticCounterContext,
    event: matrix_sdk::encryption::EncryptionReadinessDiagnostic,
) {
    use matrix_sdk::encryption::{
        EncryptionReadinessOutcome as Outcome, EncryptionReadinessQueryState as Query,
        EncryptionReadinessSyncState as Sync,
    };

    let sync = match event.sync {
        Sync::NotStarted => "not_started",
        Sync::Pending => "pending",
        Sync::Received => "received",
        Sync::Failed => "failed",
        Sync::Cancelled => "cancelled",
    };
    let query = match event.query {
        Query::NotStarted => "not_started",
        Query::InProgress => "in_progress",
        Query::Accepted => "accepted",
        Query::Failed => "failed",
    };
    let outcome = match event.outcome {
        Outcome::Ready => "ready",
        Outcome::Sync => "sync",
        Outcome::KeyQuery => "key_query",
        Outcome::SecondShare => "second_share",
        Outcome::SessionChanged => "session_changed",
        Outcome::Deadline => "deadline",
        Outcome::Cancelled => "cancelled",
    };
    counters.increment(match event.outcome {
        Outcome::Ready => "encryption_readiness_ready",
        Outcome::Sync => "encryption_readiness_sync",
        Outcome::KeyQuery => "encryption_readiness_key_query",
        Outcome::SecondShare => "encryption_readiness_second_share",
        Outcome::SessionChanged => "encryption_readiness_session_changed",
        Outcome::Deadline => "encryption_readiness_deadline",
        Outcome::Cancelled => "encryption_readiness_cancelled",
    });
    koushi_diagnostics::record(
        DiagnosticEvent::new(
            if event.outcome == Outcome::Ready {
                DiagnosticLevel::Info
            } else {
                DiagnosticLevel::Warn
            },
            "core.encryption_readiness",
            outcome,
        )
        .field(DiagnosticField::ordinal_alias(
            "room_alias",
            "room",
            event.room.ordinal(),
        ))
        .field(DiagnosticField::ordinal_alias(
            "session_alias",
            "session",
            event.session.ordinal(),
        ))
        .field(DiagnosticField::count("generation", event.generation))
        .field(DiagnosticField::token("sync", sync))
        .field(DiagnosticField::token("query", query))
        .field(DiagnosticField::count(
            "active_members_bucket",
            event.active_members_bucket.into(),
        ))
        .field(DiagnosticField::count(
            "returned_devices_bucket",
            event.returned_devices_bucket.into(),
        ))
        .field(DiagnosticField::count(
            "eligible_devices_bucket",
            event.eligible_devices_bucket.into(),
        ))
        .field(DiagnosticField::count(
            "accepted_devices_bucket",
            event.accepted_devices_bucket.into(),
        ))
        .field(DiagnosticField::count(
            "message_index_bucket",
            event.message_index_bucket.into(),
        ))
        .field(DiagnosticField::count(
            "registry_evictions",
            event.registry_evictions,
        ))
        .field(DiagnosticField::boolean("retryable", event.retryable)),
    );
}

fn record_olm_recovery_diagnostic(
    counters: &DiagnosticCounterContext,
    event: matrix_sdk::encryption::OlmRecoveryDiagnostic,
) {
    use matrix_sdk::encryption::{OlmRecoveryReshareOutcome, OlmRecoverySignalOutcome};

    let signal_token = match event.signal {
        OlmRecoverySignalOutcome::Observed => "observed",
        OlmRecoverySignalOutcome::IgnoredUnknownDevice => "ignored_unknown_device",
        OlmRecoverySignalOutcome::IgnoredDehydrated => "ignored_dehydrated",
        OlmRecoverySignalOutcome::Failed => "failed",
    };
    let reshare_token = event.reshare.map(|outcome| match outcome {
        OlmRecoveryReshareOutcome::Queued => "queued",
        OlmRecoveryReshareOutcome::AlreadyPending => "already_pending",
        OlmRecoveryReshareOutcome::NoMatchingSession => "no_matching_session",
        OlmRecoveryReshareOutcome::PolicyBlocked => "policy_blocked",
        OlmRecoveryReshareOutcome::Failed => "failed",
    });

    counters.increment("olm_recovery_signal");
    if let Some(reshare) = reshare_token {
        counters.increment("olm_recovery_reshare");
        let mut diagnostic =
            DiagnosticEvent::new(DiagnosticLevel::Info, "core.olm_recovery", "reshare")
                .field(DiagnosticField::token("signal", signal_token))
                .field(DiagnosticField::token("reshare", reshare))
                .field(DiagnosticField::count(
                    "matching_sessions_bucket",
                    event.matching_sessions_bucket as u64,
                ));
        if let Some(device) = event.device {
            diagnostic = diagnostic.field(DiagnosticField::ordinal_alias(
                "device_alias",
                "device",
                device.ordinal(),
            ));
        }
        koushi_diagnostics::record(diagnostic);
    } else {
        let mut diagnostic =
            DiagnosticEvent::new(DiagnosticLevel::Info, "core.olm_recovery", "signal")
                .field(DiagnosticField::token("signal", signal_token));
        if let Some(device) = event.device {
            diagnostic = diagnostic.field(DiagnosticField::ordinal_alias(
                "device_alias",
                "device",
                device.ordinal(),
            ));
        }
        koushi_diagnostics::record(diagnostic);
    }
}

fn record_room_key_receive_diagnostic(
    counters: &DiagnosticCounterContext,
    event: matrix_sdk::encryption::RoomKeyReceiveDiagnostic,
) {
    use matrix_sdk::encryption::{
        ForwardedRoomKeyAuthOutcome as ForwardOutcome, RoomKeyIngressKind as IngressKind,
        RoomKeyMergeDecision as MergeDecision, RoomKeyReceiveDiagnosticKind as Kind,
    };

    let (token, counter) = match event.kind {
        Kind::RoomKeyIngress {
            kind: IngressKind::Direct,
        } => ("ingress_direct", "receive_ingress_direct"),
        Kind::RoomKeyIngress {
            kind: IngressKind::Forwarded,
        } => ("ingress_forwarded", "receive_ingress_forwarded"),
        Kind::ToDeviceOlmFailed => ("olm_failed", "receive_olm_failed"),
        Kind::ToDeviceOlmWedged => ("olm_wedged", "receive_olm_wedged"),
        Kind::ToDeviceDehydratedRejected => ("dehydrated_rejected", "receive_dehydrated_rejected"),
        Kind::ToDeviceMalformed => ("malformed", "receive_malformed"),
        Kind::RoomKeyUnsupportedAlgorithm => {
            ("unsupported_algorithm", "receive_unsupported_algorithm")
        }
        Kind::ForwardedRoomKeyAuth {
            outcome: ForwardOutcome::RejectedNoMatchingRequest,
        } => (
            "forwarded_no_matching_request",
            "receive_forwarded_no_matching_request",
        ),
        Kind::ForwardedRoomKeyAuth {
            outcome: ForwardOutcome::RejectedUntrustedSender,
        } => (
            "forwarded_untrusted_sender",
            "receive_forwarded_untrusted_sender",
        ),
        Kind::ForwardedRoomKeyAuth {
            outcome: ForwardOutcome::UnsupportedAlgorithm,
        } => ("forwarded_unsupported", "receive_forwarded_unsupported"),
        Kind::ForwardedRoomKeyAuth {
            outcome: ForwardOutcome::Accepted,
        } => ("forwarded_accepted", "receive_forwarded_accepted"),
        Kind::Merge {
            decision: MergeDecision::AcceptedNew,
        } => ("merge_accepted_new", "receive_merge_accepted_new"),
        Kind::Merge {
            decision: MergeDecision::AcceptedImproved,
        } => ("merge_accepted_improved", "receive_merge_accepted_improved"),
        Kind::Merge {
            decision: MergeDecision::DuplicateIgnored,
        } => ("merge_duplicate_ignored", "receive_merge_duplicate_ignored"),
        Kind::Merge {
            decision: MergeDecision::WorseIgnored,
        } => ("merge_worse_ignored", "receive_merge_worse_ignored"),
        Kind::Merge {
            decision: MergeDecision::UnconnectedRejected,
        } => (
            "merge_unconnected_rejected",
            "receive_merge_unconnected_rejected",
        ),
        Kind::Merge {
            decision: MergeDecision::InvalidSessionKey,
        } => (
            "merge_invalid_session_key",
            "receive_merge_invalid_session_key",
        ),
        Kind::Merge {
            decision: MergeDecision::StoreFailed,
        } => ("merge_store_failed", "receive_merge_store_failed"),
    };
    counters.increment(counter);
    koushi_diagnostics::record(
        DiagnosticEvent::new(DiagnosticLevel::Info, "core.room_key_receive", "outcome")
            .field(DiagnosticField::token("outcome", token)),
    );
}

fn record_initial_share_diagnostic(
    counters: &DiagnosticCounterContext,
    event: matrix_sdk::encryption::InitialShareDeviceDiagnostic,
) {
    use matrix_sdk::encryption::{InitialShareDeviceClass as Class, InitialShareStage as Stage};

    let device_class = match event.device_class {
        Class::VerifiedOwn => "verified_own",
        Class::UnverifiedOwn => "unverified_own",
        Class::VerifiedPeer => "verified_peer",
        Class::UnverifiedPeer => "unverified_peer",
        Class::Dehydrated => "dehydrated",
        Class::Unknown => "unknown",
    };
    let (stage, counter) = match event.stage {
        Stage::Eligible => ("eligible", "initial_share_eligible"),
        Stage::OlmMissing => ("olm_missing", "initial_share_olm_missing"),
        Stage::OlmEncrypted => ("olm_encrypted", "initial_share_olm_encrypted"),
        Stage::OlmEncryptionFailed => (
            "olm_encryption_failed",
            "initial_share_olm_encryption_failed",
        ),
        Stage::Withheld => ("withheld", "initial_share_withheld"),
        Stage::RequestQueued => ("request_queued", "initial_share_request_queued"),
        Stage::HomeserverAccepted => ("homeserver_accepted", "initial_share_homeserver_accepted"),
        Stage::RequestFailed => ("request_failed", "initial_share_request_failed"),
        Stage::ShareStateCommitted { message_index } => {
            if message_index == 0 {
                (
                    "share_state_committed",
                    "initial_share_share_committed_index0",
                )
            } else {
                (
                    "share_state_committed",
                    "initial_share_share_committed_after_index0",
                )
            }
        }
    };
    counters.increment(counter);
    match event.stage {
        Stage::Eligible => match event.device_class {
            Class::VerifiedOwn | Class::UnverifiedOwn => {
                counters.increment("initial_share_eligible_own")
            }
            Class::VerifiedPeer | Class::UnverifiedPeer => {
                counters.increment("initial_share_eligible_peer")
            }
            Class::Dehydrated | Class::Unknown => {}
        },
        _ => {}
    }

    let mut diagnostic = DiagnosticEvent::new(DiagnosticLevel::Info, "core.initial_share", "stage")
        .field(DiagnosticField::ordinal_alias(
            "session_alias",
            "session",
            event.session.ordinal(),
        ))
        .field(DiagnosticField::ordinal_alias(
            "device_alias",
            "device",
            event.device.ordinal(),
        ))
        .field(DiagnosticField::token("device_class", device_class))
        .field(DiagnosticField::token("stage", stage))
        .field(DiagnosticField::milliseconds(
            "elapsed_ms",
            event.elapsed_ms.into(),
        ));
    if let Stage::ShareStateCommitted { message_index } = event.stage {
        diagnostic = diagnostic.field(DiagnosticField::count(
            "message_index",
            message_index as u64,
        ));
    }
    koushi_diagnostics::record(diagnostic);
}

fn record_initial_share_session_diagnostic(
    counters: &DiagnosticCounterContext,
    event: matrix_sdk::encryption::InitialShareSessionDiagnostic,
) {
    koushi_diagnostics::mark_rotation_first_send_correlation(event.session.ordinal());
    counters.increment(if event.all_initial_shares_settled_first {
        "initial_share_first_event_all_settled"
    } else {
        "initial_share_first_event_pending"
    });
    counters.increment(if event.created_at_index0 {
        "initial_share_sessions_at_index0"
    } else {
        "initial_share_sessions_after_index0"
    });
    koushi_diagnostics::record(
        DiagnosticEvent::new(DiagnosticLevel::Info, "core.initial_share", "first_event")
            .field(DiagnosticField::ordinal_alias(
                "session_alias",
                "session",
                event.session.ordinal(),
            ))
            .field(DiagnosticField::count(
                "first_event_message_index",
                event.first_event_message_index as u64,
            ))
            .field(DiagnosticField::boolean(
                "all_initial_shares_settled_first",
                event.all_initial_shares_settled_first,
            ))
            .field(DiagnosticField::count(
                "pending_requests_bucket",
                event.pending_requests_bucket as u64,
            ))
            .field(DiagnosticField::count(
                "eligible_own_devices",
                event.eligible_own_devices as u64,
            ))
            .field(DiagnosticField::count(
                "eligible_peer_devices",
                event.eligible_peer_devices as u64,
            ))
            .field(DiagnosticField::count(
                "index0_shares_committed",
                event.index0_shares_committed as u64,
            ))
            .field(DiagnosticField::count(
                "after_index0_shares_committed",
                event.after_index0_shares_committed as u64,
            ))
            .field(DiagnosticField::count(
                "homeserver_accepted_devices",
                event.homeserver_accepted_devices as u64,
            ))
            .field(DiagnosticField::boolean(
                "created_at_index0",
                event.created_at_index0,
            ))
            .field(DiagnosticField::milliseconds(
                "elapsed_ms",
                event.elapsed_ms.into(),
            )),
    );
}

fn record_index0_reshare_diagnostic(
    counters: &DiagnosticCounterContext,
    event: matrix_sdk::encryption::Index0ReshareDiagnostic,
) {
    use matrix_sdk::encryption::{
        Index0InitialShareState as Share, Index0ReshareOutcome as Outcome,
    };

    let reshare = match event.reshare {
        Outcome::Sent => "sent",
        Outcome::Deadline => "deadline",
        Outcome::Cancelled => "cancelled",
        Outcome::PolicyBlocked => "policy_blocked",
        Outcome::Failed => "failed",
        Outcome::NotNeeded => "not_needed",
    };
    let initial_share = match event.initial_share {
        Share::Accepted => "accepted",
        Share::Failed => "failed",
        Share::Withheld => "withheld",
        Share::NoRecipients => "no_recipients",
    };
    let reshare_counter = match event.reshare {
        Outcome::Sent => "index0_reshare_sent",
        Outcome::Deadline => "index0_reshare_deadline",
        Outcome::Cancelled => "index0_reshare_cancelled",
        Outcome::PolicyBlocked => "index0_reshare_policy_blocked",
        Outcome::Failed => "index0_reshare_failed",
        Outcome::NotNeeded => "index0_reshare_not_needed",
    };
    let initial_share_counter = match event.initial_share {
        Share::Accepted => "index0_initial_share_accepted",
        Share::Failed => "index0_initial_share_failed",
        Share::Withheld => "index0_initial_share_withheld",
        Share::NoRecipients => "index0_initial_share_no_recipients",
    };
    counters.increment(reshare_counter);
    counters.increment(initial_share_counter);
    koushi_diagnostics::record(
        DiagnosticEvent::new(DiagnosticLevel::Info, "core.index0_reshare", "outcome")
            .field(DiagnosticField::ordinal_alias(
                "session_alias",
                "session",
                event.session.ordinal(),
            ))
            .field(DiagnosticField::token("initial_share", initial_share))
            .field(DiagnosticField::token("reshare", reshare))
            .field(DiagnosticField::count(
                "eligible_own_bucket",
                event.eligible_own_bucket as u64,
            ))
            .field(DiagnosticField::count(
                "eligible_peer_bucket",
                event.eligible_peer_bucket as u64,
            ))
            .field(DiagnosticField::milliseconds(
                "elapsed_ms",
                event.elapsed_ms.into(),
            )),
    );
}

fn record_initial_share_repair_diagnostic(
    counters: &DiagnosticCounterContext,
    event: matrix_sdk::encryption::InitialShareRepairDiagnostic,
) {
    use matrix_sdk::encryption::{
        InitialShareRepairClaimOutcome as Claim, InitialShareRepairOlmState as Olm,
        InitialShareRepairOutcome as Repair,
    };

    let initial_olm = match event.initial_olm {
        Olm::Missing => "missing",
        Olm::Present => "present",
        Olm::Unknown => "unknown",
    };
    let claim = match event.claim {
        Claim::NotNeeded => "not_needed",
        Claim::Requested => "requested",
        Claim::Accepted => "accepted",
        Claim::Empty => "empty",
        Claim::Invalid => "invalid",
        Claim::NetworkFailed => "network_failed",
        Claim::SdkFailed => "sdk_failed",
    };
    let repair = match event.repair {
        Repair::Settled => "settled",
        Repair::WaitingWake => "waiting_wake",
        Repair::Deadline => "deadline",
        Repair::Cancelled => "cancelled",
        Repair::NoRecipients => "no_recipients",
        Repair::Failed => "failed",
    };
    let claim_counter = match event.claim {
        Claim::NotNeeded => "initial_repair_claim_not_needed",
        Claim::Requested => "initial_repair_claim_requested",
        Claim::Accepted => "initial_repair_claim_accepted",
        Claim::Empty => "initial_repair_claim_empty",
        Claim::Invalid => "initial_repair_claim_invalid",
        Claim::NetworkFailed => "initial_repair_claim_network_failed",
        Claim::SdkFailed => "initial_repair_claim_sdk_failed",
    };
    let repair_counter = match event.repair {
        Repair::Settled => "initial_repair_settled",
        Repair::WaitingWake => "initial_repair_waiting_wake",
        Repair::Deadline => "initial_repair_deadline",
        Repair::Cancelled => "initial_repair_cancelled",
        Repair::NoRecipients => "initial_repair_no_recipients",
        Repair::Failed => "initial_repair_failed",
    };
    if event.first_event_message_index.is_none() {
        counters.increment(claim_counter);
        counters.increment(repair_counter);
    }
    let mut diagnostic = DiagnosticEvent::new(
        DiagnosticLevel::Info,
        "core.initial_share_repair",
        "outcome",
    )
    .field(DiagnosticField::ordinal_alias(
        "session_alias",
        "session",
        event.session.ordinal(),
    ))
    .field(DiagnosticField::token("initial_olm", initial_olm))
    .field(DiagnosticField::token("claim", claim))
    .field(DiagnosticField::token("repair", repair))
    .field(DiagnosticField::count(
        "own_coverage_bucket",
        event.own_coverage_bucket as u64,
    ))
    .field(DiagnosticField::count(
        "peer_users_covered_bucket",
        event.peer_users_covered_bucket as u64,
    ))
    .field(DiagnosticField::count(
        "peer_users_zero_coverage_bucket",
        event.peer_users_zero_coverage_bucket as u64,
    ))
    .field(DiagnosticField::count(
        "missing_devices_bucket",
        event.missing_devices_bucket as u64,
    ))
    .field(DiagnosticField::boolean("same_session", event.same_session))
    .field(DiagnosticField::boolean(
        "first_event_index_known",
        event.first_event_message_index.is_some(),
    ))
    .field(DiagnosticField::milliseconds(
        "elapsed_ms",
        event.elapsed_ms.into(),
    ));
    if let Some(fence) = match (event.first_event_message_index, event.repair) {
        (Some(_), Repair::Settled) => Some("settled"),
        (Some(_), Repair::NoRecipients) => Some("no_recipients"),
        (Some(_), Repair::Failed) => Some("failed"),
        (Some(_), Repair::Deadline) => Some("deadline"),
        (_, Repair::Cancelled) => Some("cancelled"),
        _ => None,
    } {
        diagnostic = diagnostic.field(DiagnosticField::token("first_event_fence", fence));
    }
    if let Some(index) = event.first_event_message_index {
        diagnostic = diagnostic.field(DiagnosticField::count(
            "first_event_message_index",
            index as u64,
        ));
    }
    koushi_diagnostics::record(diagnostic);
}

fn record_incoming_room_key_diagnostic(
    counters: &DiagnosticCounterContext,
    event: matrix_sdk::encryption::IncomingRoomKeyRequestDiagnostic,
) {
    use matrix_sdk::encryption::{
        IncomingRoomKeyRequestOutcome as Outcome, IncomingRoomKeyRequestStage as Stage,
        RequestedRoomKeySession as SessionKind, RoomKeyRefusalReason as Refusal,
        RoomKeyRequestAction as Action, RoomKeyRequesterDeviceState as DeviceState,
        RoomKeyRequesterScope as Scope,
    };

    let stage = match event.stage {
        Stage::Received => "received",
        Stage::Classified => "classified",
        Stage::SessionLookup => "session_lookup",
        Stage::AuthorizationDecided => "authorization_decided",
        Stage::Outcome => "outcome",
    };
    let action = match event.action {
        Action::Request => "request",
        Action::Cancellation => "cancellation",
        Action::Unknown => "unknown",
    };
    let scope = match event.requester_scope {
        Scope::Own => "own",
        Scope::Peer => "peer",
        Scope::Unknown => "unknown",
    };
    let device_state = match event.requester_device_state {
        DeviceState::Current => "current",
        DeviceState::VerifiedOwn => "verified_own",
        DeviceState::UnverifiedOwn => "unverified_own",
        DeviceState::KnownPeer => "known_peer",
        DeviceState::Unknown => "unknown",
    };
    let session_kind = match event.requested_session_kind {
        SessionKind::Current => "current",
        SessionKind::Historical => "historical",
        SessionKind::Unknown => "unknown",
    };
    let outcome = match event.outcome {
        Outcome::None => "none",
        Outcome::Forwarded => "forwarded",
        Outcome::QueuedForOlm => "queued_for_olm",
        Outcome::Cancelled => "cancelled",
        Outcome::IgnoredSelf => "ignored_self",
        Outcome::Refused => "refused",
        Outcome::MissingSession => "missing_session",
        Outcome::UnsupportedAlgorithm => "unsupported_algorithm",
        Outcome::ForwardingDisabled => "forwarding_disabled",
        Outcome::SdkError => "sdk_error",
    };
    let refusal = match event.refusal_reason {
        Refusal::None => "none",
        Refusal::MissingOldOutboundProof => "missing_old_outbound_proof",
        Refusal::NotOriginalRecipient => "not_original_recipient",
        Refusal::UntrustedOwnDevice => "untrusted_own_device",
        Refusal::ChangedSenderKey => "changed_sender_key",
        Refusal::UnknownDevice => "unknown_device",
        Refusal::UnsupportedAlgorithm => "unsupported_algorithm",
        Refusal::ForwardingDisabled => "forwarding_disabled",
        Refusal::MissingInboundSession => "missing_inbound_session",
        Refusal::MissingOlmSession => "missing_olm_session",
        Refusal::SdkError => "sdk_error",
    };

    let mut diagnostic =
        DiagnosticEvent::new(DiagnosticLevel::Info, "core.room_key_request", stage)
            .field(DiagnosticField::token("action", action))
            .field(DiagnosticField::ordinal_alias(
                "request_alias",
                "request",
                event.request.ordinal(),
            ))
            .field(DiagnosticField::token("requester_scope", scope))
            .field(DiagnosticField::ordinal_alias(
                "requester_device_alias",
                "device",
                event.requester_device.ordinal(),
            ))
            .field(DiagnosticField::token(
                "requester_device_state",
                device_state,
            ))
            .field(DiagnosticField::token("requested_session", session_kind))
            .field(DiagnosticField::token(
                "inbound_session_present",
                optional_bool_token(event.inbound_session_present),
            ))
            .field(DiagnosticField::token(
                "matching_outbound_proof_present",
                optional_bool_token(event.matching_outbound_proof_present),
            ))
            .field(DiagnosticField::token("outcome", outcome))
            .field(DiagnosticField::token("refusal_reason", refusal))
            .field(DiagnosticField::token(
                "response_created",
                optional_bool_token(event.response_created),
            ))
            .field(DiagnosticField::milliseconds(
                "elapsed_ms",
                event.elapsed_ms.into(),
            ));
    if let Some(peer) = event.requester_user {
        diagnostic = diagnostic.field(DiagnosticField::ordinal_alias(
            "requester_user_alias",
            "peer",
            peer.ordinal(),
        ));
    }
    if let Some(room) = event.room {
        diagnostic = diagnostic.field(DiagnosticField::ordinal_alias(
            "room_alias",
            "room",
            room.ordinal(),
        ));
    }
    if let Some(session) = event.requested_session {
        diagnostic = diagnostic.field(DiagnosticField::ordinal_alias(
            "requested_session_alias",
            "session",
            session.ordinal(),
        ));
    }
    record(diagnostic);

    if event.stage == Stage::Received {
        counters.increment("received_requests");
        if event.action == Action::Cancellation {
            counters.increment("cancellations");
        }
    }
    if event.stage == Stage::Outcome {
        match event.outcome {
            Outcome::Forwarded => counters.increment("forwarded"),
            Outcome::QueuedForOlm => counters.increment("queued"),
            Outcome::Refused => counters.increment("refused"),
            Outcome::MissingSession => counters.increment("missing_sessions"),
            Outcome::SdkError => counters.increment("sdk_errors"),
            _ => {}
        }
    }
}

fn record_room_key_rotation_diagnostic(
    counters: &DiagnosticCounterContext,
    event: matrix_sdk::encryption::RoomKeyRotationDiagnostic,
) {
    use matrix_sdk::encryption::{
        RoomKeyCreationOutcome as Creation, RoomKeyFirstShareOutcome as Share,
        RoomKeyRotationReason as Reason,
    };

    let reason = match event.reason {
        Reason::Initial => "initial",
        Reason::ExpiredTime => "expired_time",
        Reason::ExpiredMessageCount => "expired_message_count",
        Reason::MembershipOrDeviceChange => "membership_or_device_change",
        Reason::EncryptionSettingsChanged => "encryption_settings_changed",
        Reason::ExplicitDiscard => "explicit_discard",
        Reason::FullMemberListReload => "full_member_list_reload",
        Reason::RoomSubscription => "room_subscription",
        Reason::LimitedSyncResponse => "limited_sync_response",
        Reason::KeyShareFailure => "key_share_failure",
        Reason::StoreMissing => "store_missing",
        Reason::Invalidated => "invalidated",
        Reason::Unknown => "unknown",
    };
    let creation = match event.creation_outcome {
        Creation::Created => "created",
        Creation::Reused => "reused",
        Creation::Failed => "failed",
    };
    let share = match event.first_share_outcome {
        Share::Pending => "pending",
        Share::Sent => "sent",
        Share::Failed => "failed",
        Share::Unknown => "unknown",
    };
    koushi_diagnostics::record_rotation_boundary(koushi_diagnostics::RotationBoundaryDiagnostic {
        room_alias: event.room.ordinal(),
        previous_session_alias: event.previous_session.map(|alias| alias.ordinal()),
        new_session_alias: event.new_session.map(|alias| alias.ordinal()),
        reason,
        creation_outcome: creation,
        first_share_outcome: share,
        first_send_correlation_present: event.first_send_correlation_present,
        discard_elapsed_ms: event.discard_elapsed_ms,
        elapsed_ms: event.elapsed_ms,
    });

    match event.reason {
        Reason::Initial => counters.increment("initial_session_creations"),
        Reason::ExpiredTime => counters.increment("rotations_expired_time"),
        Reason::ExpiredMessageCount => counters.increment("rotations_expired_message_count"),
        Reason::MembershipOrDeviceChange => {
            counters.increment("rotations_membership_or_device_change")
        }
        Reason::EncryptionSettingsChanged => {
            counters.increment("rotations_encryption_settings_changed")
        }
        Reason::ExplicitDiscard => counters.increment("rotations_explicit_discard"),
        Reason::FullMemberListReload => counters.increment("rotations_full_member_list_reload"),
        Reason::RoomSubscription => counters.increment("rotations_room_subscription"),
        Reason::LimitedSyncResponse => counters.increment("rotations_limited_sync_response"),
        Reason::KeyShareFailure => counters.increment("rotations_key_share_failure"),
        Reason::StoreMissing => counters.increment("rotations_store_missing"),
        Reason::Invalidated => counters.increment("rotations_invalidated"),
        Reason::Unknown => counters.increment("rotations_unknown"),
    }
    if event.creation_outcome == Creation::Failed {
        counters.increment("rotation_creation_failures");
    }
    if event.first_share_outcome == Share::Failed {
        counters.increment("rotation_share_failures");
    }
}

fn record_room_key_member_reload_diagnostic(
    counters: &DiagnosticCounterContext,
    event: matrix_sdk::encryption::RoomKeyMemberReloadDiagnostic,
) {
    use matrix_sdk::encryption::{
        RoomKeyMemberReloadDiscardOutcome as Outcome, RoomKeyRotationReason as Reason,
    };

    let reason = match event.reason {
        Reason::RoomSubscription => "room_subscription",
        Reason::LimitedSyncResponse => "limited_sync_response",
        Reason::MembershipOrDeviceChange => "membership_or_device_change",
        Reason::FullMemberListReload => "full_member_list_reload",
        _ => "unknown",
    };
    let discard_outcome = match event.discard_outcome {
        Outcome::Discarded => "discarded",
        Outcome::NoActiveSession => "no_active_session",
        Outcome::SdkError => "sdk_error",
    };
    let mut diagnostic = DiagnosticEvent::new(
        DiagnosticLevel::Info,
        "core.room_member_reload",
        "completed",
    )
    .field(DiagnosticField::ordinal_alias(
        "room_alias",
        "room",
        event.room.ordinal(),
    ))
    .field(DiagnosticField::token("reason", reason))
    .field(DiagnosticField::count(
        "invalidation_count_bucket",
        event.invalidation_count_bucket.into(),
    ))
    .field(DiagnosticField::count(
        "response_member_count_bucket",
        event.response_member_count_bucket.into(),
    ))
    .field(DiagnosticField::milliseconds(
        "processing_elapsed_ms",
        event.processing_elapsed_ms.into(),
    ))
    .field(DiagnosticField::token("discard_outcome", discard_outcome));
    if let Some(were_synced) = event.members_were_synced_before_invalidation {
        diagnostic = diagnostic.field(DiagnosticField::boolean(
            "members_synced_before_invalidation",
            were_synced,
        ));
    }
    if let Some(invalidation_age_ms) = event.invalidation_age_ms {
        diagnostic = diagnostic.field(DiagnosticField::milliseconds(
            "invalidation_age_ms",
            invalidation_age_ms.into(),
        ));
    }
    if let Some(request_elapsed_ms) = event.request_elapsed_ms {
        diagnostic = diagnostic.field(DiagnosticField::milliseconds(
            "request_elapsed_ms",
            request_elapsed_ms.into(),
        ));
    }
    record(diagnostic);

    match event.reason {
        Reason::RoomSubscription => counters.increment("member_reloads_room_subscription"),
        Reason::LimitedSyncResponse => counters.increment("member_reloads_limited_sync_response"),
        Reason::MembershipOrDeviceChange => {
            counters.increment("member_reloads_membership_or_device_change")
        }
        Reason::FullMemberListReload => {
            counters.increment("member_reloads_full_member_list_reload")
        }
        _ => counters.increment("member_reloads_unknown"),
    }
    match event.discard_outcome {
        Outcome::Discarded => counters.increment("member_reload_discarded"),
        Outcome::NoActiveSession => counters.increment("member_reload_no_active_session"),
        Outcome::SdkError => counters.increment("member_reload_sdk_error"),
    }
}

fn optional_bool_token(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "unknown",
    }
}

#[cfg(test)]
mod megolm_send_parity_tests {
    use std::sync::{Arc, Mutex};

    use matrix_sdk::{
        encryption::{RoomKeyDiagnosticEvent, RoomKeyDiagnosticObserver},
        test_utils::mocks::MatrixMockServer,
    };
    use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory, test_json};
    use ruma::{
        RoomVersionId, device_id, events::room::message::RoomMessageEventContent, room_id, user_id,
    };
    use wiremock::{
        Mock, ResponseTemplate,
        matchers::{method, path_regex},
    };

    const TO_DEVICE_PATH: &str = r"^/_matrix/client/.*/sendToDevice/m.room.encrypted/.*";
    const ROOM_SEND_PATH: &str = r"^/_matrix/client/.*/rooms/.*/send/m.room.encrypted/.*";

    #[tokio::test]
    async fn koushi_default_builder_does_not_enable_index0_duplicate_share() {
        let server = MatrixMockServer::new().await;
        server.mock_crypto_endpoints_preset().await;
        let alice_user_id = user_id!("@alice:example.org");
        let bob_user_id = user_id!("@bob:example.org");
        let alice_device_id = device_id!("ALICEDEVICE");
        let bob_device_id = device_id!("BOBDEVICE");
        let alice = server
            .client_builder_for_crypto_end_to_end(alice_user_id, alice_device_id)
            .on_builder(super::desktop_client_builder_defaults)
            .build()
            .await;
        let bob = server
            .client_builder_for_crypto_end_to_end(bob_user_id, bob_device_id)
            .build()
            .await;
        server.exchange_e2ee_identities(&alice, &bob).await;

        let room_id = room_id!("!koushi-megolm-default:example.org");
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let observer: RoomKeyDiagnosticObserver = Arc::new(move |event| {
            captured.lock().unwrap().push(event);
        });
        alice
            .encryption()
            .set_room_key_diagnostic_observer(Some(observer))
            .await;

        let event_factory = EventFactory::new().sender(alice_user_id).room(room_id);
        server
            .mock_sync()
            .ok_and_run(&alice, |builder| {
                builder.add_joined_room(
                    JoinedRoomBuilder::new(room_id)
                        .add_state_event(event_factory.create(alice_user_id, RoomVersionId::V1))
                        .add_state_event(event_factory.room_encryption())
                        .add_state_event(event_factory.member(alice_user_id).into_raw())
                        .add_state_event(event_factory.member(bob_user_id).into_raw()),
                );
            })
            .await;
        server
            .mock_get_members()
            .ok(vec![
                event_factory.member(alice_user_id).into_raw(),
                event_factory.member(bob_user_id).into_raw(),
            ])
            .mount()
            .await;
        Mock::given(method("PUT"))
            .and(path_regex(TO_DEVICE_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(&*test_json::EMPTY))
            .mount(server.server())
            .await;
        Mock::given(method("PUT"))
            .and(path_regex(ROOM_SEND_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(&*test_json::EVENT_ID))
            .mount(server.server())
            .await;

        let mut encryption_generation = alice
            .begin_encryption_sync_generation()
            .expect("desktop readiness enabled");
        encryption_generation.mark_received();
        let room = alice.get_room(&room_id).unwrap();
        matrix_sdk::room::futures::ensure_room_encryption_ready_with_index0_duplicate_share_for_testing(
            &room,
        )
        .await
        .unwrap();
        room.send(RoomMessageEventContent::text_plain("first"))
            .await
            .unwrap();

        let requests = server.server().received_requests().await.unwrap();
        let encrypted_to_device = requests
            .iter()
            .filter(|request| {
                request
                    .url
                    .path()
                    .contains("/sendToDevice/m.room.encrypted/")
            })
            .count();
        assert_eq!(
            encrypted_to_device, 1,
            "Koushi defaults must keep standard pre-share only"
        );
        let events = events.lock().unwrap();
        assert!(events.iter().any(|event| {
            matches!(
                event,
                RoomKeyDiagnosticEvent::InitialShareSession(record)
                    if record.first_event_message_index == 0
            )
        }));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, RoomKeyDiagnosticEvent::Index0Reshare(_)))
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, RoomKeyDiagnosticEvent::InitialShareRepair(_)))
        );
    }
}

pub(super) fn desktop_room_key_recipient_strategy() -> CollectStrategy {
    CollectStrategy::AllDevices
}

pub async fn recover_e2ee(
    session: &MatrixClientSession,
    request: &RecoveryRequest,
) -> Result<(), E2eeRecoveryError> {
    let client = session.client();
    let encryption = client.encryption();
    let own_user_id = client
        .user_id()
        .ok_or_else(|| E2eeRecoveryError::Sdk("missing own user id".to_owned()))?
        .to_owned();
    record_recovery_cross_signing_status(&encryption, "cross_signing_before_recovery").await;
    record_recovery_verification_event(recovery_verification_event(
        "recover_and_fix_backup_started",
    ));
    encryption
        .recovery()
        .recover_and_fix_backup(request.secret.expose_secret())
        .await
        .map_err(|error| {
            let mut event = recovery_verification_event("recover_and_fix_backup_finished")
                .field(DiagnosticField::token("outcome", "failed"))
                .field(DiagnosticField::token(
                    "error_kind",
                    recovery_error_kind(&error),
                ));
            if let Some(diagnostics) = recovery_signature_upload_failure_diagnostics(&error) {
                event = with_signature_upload_failure_diagnostics(event, diagnostics);
            }
            record_recovery_verification_event(event);
            E2eeRecoveryError::Sdk(error.to_string())
        })?;
    record_recovery_verification_event(
        recovery_verification_event("recover_and_fix_backup_finished")
            .field(DiagnosticField::token("outcome", "success")),
    );
    record_recovery_cross_signing_status(&encryption, "cross_signing_after_recovery").await;

    record_recovery_verification_event(recovery_verification_event(
        "authoritative_signature_inspection_started",
    ));
    let signature_inspection = encryption
        .recovery()
        .inspect_current_device_signature_state()
        .await
        .map_err(|error| {
            record_recovery_verification_event(
                recovery_verification_event("authoritative_signature_inspection_finished")
                    .field(DiagnosticField::token("outcome", "failed")),
            );
            E2eeRecoveryError::Sdk(error.to_string())
        })?;
    record_recovery_verification_event(
        recovery_verification_event("authoritative_signature_inspection_finished")
            .field(DiagnosticField::token("outcome", "success"))
            .field(DiagnosticField::count(
                "query_failure_count",
                signature_inspection.query_failure_count as u64,
            ))
            .field(DiagnosticField::count(
                "server_device_count",
                signature_inspection.server_device_count as u64,
            ))
            .field(DiagnosticField::boolean(
                "authoritative_device_present",
                signature_inspection.authoritative_device_present,
            ))
            .field(DiagnosticField::boolean(
                "authoritative_device_deserialized",
                signature_inspection.authoritative_device_deserialized,
            ))
            .field(DiagnosticField::count(
                "authoritative_signature_count",
                signature_inspection.authoritative_signature_count as u64,
            ))
            .field(DiagnosticField::boolean(
                "self_signing_key_present",
                signature_inspection.self_signing_key_present,
            ))
            .field(DiagnosticField::boolean(
                "self_signing_key_deserialized",
                signature_inspection.self_signing_key_deserialized,
            ))
            .field(DiagnosticField::boolean(
                "authoritative_self_signing_signature_valid",
                signature_inspection.authoritative_self_signing_signature_valid,
            ))
            .field(DiagnosticField::boolean(
                "authoritative_self_signing_signature_present",
                signature_inspection.authoritative_self_signing_signature_present,
            ))
            .field(DiagnosticField::boolean(
                "authoritative_self_signing_signature_parseable",
                signature_inspection.authoritative_self_signing_signature_parseable,
            ))
            .field(DiagnosticField::boolean(
                "cached_self_signing_key_present",
                signature_inspection.cached_self_signing_key_present,
            ))
            .field(DiagnosticField::boolean(
                "cached_self_signing_key_matches_authoritative",
                signature_inspection.cached_self_signing_key_matches_authoritative,
            ))
            .field(DiagnosticField::boolean(
                "cached_device_present",
                signature_inspection.cached_device_present,
            ))
            .field(DiagnosticField::boolean(
                "cached_keys_match_authoritative",
                signature_inspection.cached_keys_match_authoritative,
            ))
            .field(DiagnosticField::boolean(
                "cached_signed_content_matches_authoritative",
                signature_inspection.cached_signed_content_matches_authoritative,
            ))
            .field(DiagnosticField::count(
                "cached_signature_count",
                signature_inspection.cached_signature_count as u64,
            ))
            .field(DiagnosticField::boolean(
                "cached_cross_signed_by_owner",
                signature_inspection.cached_cross_signed_by_owner,
            )),
    );

    let identity = encryption
        .get_user_identity(&own_user_id)
        .await
        .map_err(|error| {
            record_recovery_verification_event(
                recovery_verification_event("post_recovery_identity_inspected")
                    .field(DiagnosticField::token("outcome", "failed")),
            );
            E2eeRecoveryError::Sdk(error.to_string())
        })?;
    let projected_trust = map_sdk_verification_state(encryption.verification_state().get());
    record_recovery_verification_event(
        recovery_verification_event("post_recovery_identity_inspected")
            .field(DiagnosticField::token("outcome", "success"))
            .field(DiagnosticField::boolean(
                "identity_found",
                identity.is_some(),
            ))
            .field(DiagnosticField::boolean(
                "identity_verified",
                identity
                    .as_ref()
                    .is_some_and(|identity| identity.is_verified()),
            ))
            .field(DiagnosticField::boolean(
                "identity_previously_verified",
                identity
                    .as_ref()
                    .is_some_and(|identity| identity.was_previously_verified()),
            ))
            .field(DiagnosticField::token(
                "projected_trust",
                current_device_trust_state_token(projected_trust),
            )),
    );
    record_recovery_cross_signing_status(&encryption, "cross_signing_after_recovery_inspection")
        .await;

    let own_device = encryption.get_own_device().await.map_err(|error| {
        record_recovery_verification_event(
            recovery_verification_event("post_recovery_own_device_inspected")
                .field(DiagnosticField::token("outcome", "failed")),
        );
        E2eeRecoveryError::Sdk(error.to_string())
    })?;
    let Some(own_device) = own_device else {
        record_recovery_verification_event(
            recovery_verification_event("post_recovery_own_device_inspected")
                .field(DiagnosticField::token("outcome", "missing")),
        );
        return Err(E2eeRecoveryError::Sdk(
            "own device is missing after recovery".to_owned(),
        ));
    };

    let cross_signed_by_owner = own_device.is_cross_signed_by_owner();
    record_recovery_verification_event(
        recovery_verification_event("post_recovery_own_device_inspected")
            .field(DiagnosticField::token("outcome", "found"))
            .field(DiagnosticField::boolean(
                "verified",
                own_device.is_verified(),
            ))
            .field(DiagnosticField::boolean(
                "verified_with_cross_signing",
                own_device.is_verified_with_cross_signing(),
            ))
            .field(DiagnosticField::boolean(
                "cross_signed_by_owner",
                cross_signed_by_owner,
            )),
    );
    if !cross_signed_by_owner {
        if has_stale_authoritative_device_signature(&signature_inspection) {
            record_recovery_verification_event(
                recovery_verification_event("server_signature_conflict_detected")
                    .field(DiagnosticField::token("outcome", "confirmed"))
                    .field(DiagnosticField::token(
                        "cause",
                        "stale_authoritative_device_signature",
                    ))
                    .field(DiagnosticField::token(
                        "remediation",
                        "new_device_id_required",
                    )),
            );
            return Err(E2eeRecoveryError::Sdk(
                "the homeserver retained an invalid cross-signing signature for this device; \
                 sign in as a new device to recover"
                    .to_owned(),
            ));
        }
        return Err(E2eeRecoveryError::Sdk(
            "own device is not cross-signed after recovery".to_owned(),
        ));
    }

    Ok(())
}

pub async fn reshare_room_key(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<MatrixRoomKeyReshareOutcome, MatrixRoomOperationError> {
    force_reshare_room_key(
        session,
        room_id,
        None,
        MatrixRoomKeyReshareTarget::AllEligible,
    )
    .await
}

#[derive(Clone, Eq, Hash, PartialEq)]
pub struct MatrixOutboundGroupSessionToken(matrix_sdk::room::OutboundGroupSessionToken);

impl fmt::Debug for MatrixOutboundGroupSessionToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MatrixOutboundGroupSessionToken(<redacted>)")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixRoomKeyReshareTarget {
    OwnOtherDevices,
    PeerDevices,
    AllEligible,
}

impl From<MatrixRoomKeyReshareTarget> for matrix_sdk::room::RoomKeyReshareTarget {
    fn from(value: MatrixRoomKeyReshareTarget) -> Self {
        match value {
            MatrixRoomKeyReshareTarget::OwnOtherDevices => Self::OwnOtherDevices,
            MatrixRoomKeyReshareTarget::PeerDevices => Self::PeerDevices,
            MatrixRoomKeyReshareTarget::AllEligible => Self::AllEligible,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixRoomKeyReshareOutcome {
    Sent {
        request_count: usize,
        recipient_count: usize,
        /// Eligible devices whose share could not be Olm-encrypted (e.g. no
        /// Olm session and no usable claimed key). Kept visible so failures
        /// are not collapsed away.
        failed_recipient_count: usize,
    },
    NoSession,
    NoRecipients,
    StaleSession,
}

pub async fn current_outbound_group_session_token(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<Option<MatrixOutboundGroupSessionToken>, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    room.current_outbound_group_session_token()
        .await
        .map(|token| token.map(MatrixOutboundGroupSessionToken))
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

pub async fn force_reshare_room_key(
    session: &MatrixClientSession,
    room_id: &str,
    expected: Option<&MatrixOutboundGroupSessionToken>,
    target: MatrixRoomKeyReshareTarget,
) -> Result<MatrixRoomKeyReshareOutcome, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    room.force_reshare_room_key(expected.map(|token| &token.0), target.into())
        .await
        .map(|outcome| match outcome {
            matrix_sdk::room::RoomKeyReshareResult::Sent {
                request_count,
                recipient_count,
                failed_recipient_count,
            } => MatrixRoomKeyReshareOutcome::Sent {
                request_count,
                recipient_count,
                failed_recipient_count,
            },
            matrix_sdk::room::RoomKeyReshareResult::UnableToEncrypt { .. } => {
                MatrixRoomKeyReshareOutcome::NoRecipients
            }
            matrix_sdk::room::RoomKeyReshareResult::NoSession => {
                MatrixRoomKeyReshareOutcome::NoSession
            }
            matrix_sdk::room::RoomKeyReshareResult::NoRecipients => {
                MatrixRoomKeyReshareOutcome::NoRecipients
            }
            matrix_sdk::room::RoomKeyReshareResult::StaleSession => {
                MatrixRoomKeyReshareOutcome::StaleSession
            }
        })
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

/// Closed outcome of a manual index-0 room-key share (issue #538).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatrixIndex0ShareOutcome {
    Completed,
    RefusedNotEncrypted,
    RefusedIndexAdvanced,
    NoSession,
    NoRecipients,
    PolicyBlocked,
    CancelledStale,
    Deadline,
    Failed,
}

/// Closed outcome of the keys-claim step of a manual index-0 share
/// (issue #538).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatrixIndex0ClaimOutcome {
    NotNeeded,
    Succeeded,
    Failed,
    Deadline,
}

/// Closed aggregate summary of a manual index-0 share (issue #538). Counts
/// are buckets only; no identifiers or key material cross the boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MatrixIndex0ShareSummary {
    pub outcome: MatrixIndex0ShareOutcome,
    pub message_index_before: Option<u32>,
    pub message_index_after: Option<u32>,
    pub own_eligible: usize,
    pub own_accepted: usize,
    pub own_missing: usize,
    pub peer_eligible: usize,
    pub peer_accepted: usize,
    pub peer_missing: usize,
    pub peer_users_with_zero_accepted: usize,
    pub claim: MatrixIndex0ClaimOutcome,
    pub elapsed_ms: u64,
    pub room_event_sent: bool,
    pub index0_consumed: bool,
}

/// Closed outcome of a manual current-session index-0 recovery resend (issue #541).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatrixIndex0ResendOutcome {
    Completed,
    RefusedNotEncrypted,
    NoSession,
    InboundSessionMissing,
    InboundIndexAdvanced,
    OriginalLedgerMissing,
    NoRecipients,
    PolicyBlocked,
    StaleIdentityRefused,
    CancelledStale,
    Deadline,
    Failed,
}

/// Closed aggregate summary of a manual current-session index-0 resend.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MatrixIndex0ResendSummary {
    pub outcome: MatrixIndex0ResendOutcome,
    pub message_index_before: Option<u32>,
    pub message_index_after: Option<u32>,
    pub peer_ledger: usize,
    pub peer_sender_key_changed: usize,
    pub peer_eligible: usize,
    pub peer_accepted: usize,
    pub peer_missing: usize,
    pub policy_blocked: usize,
    pub inbound_first_known_index: Option<u32>,
    pub claim: MatrixIndex0ClaimOutcome,
    pub elapsed_ms: u64,
    pub room_event_sent: bool,
    pub index0_consumed: bool,
}

/// Closed outcome of a manual force-new-outbound-session (issue #538).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatrixForceNewSessionOutcome {
    Completed,
    RefusedNotEncrypted,
    CancelledStale,
    Failed,
    Deadline,
}

/// Closed summary of a manual force-new-outbound-session (issue #538).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MatrixForceNewSessionSummary {
    pub outcome: MatrixForceNewSessionOutcome,
    pub previous_session_exists: bool,
    pub fresh_session_created: bool,
    pub message_index: Option<u32>,
    pub elapsed_ms: u64,
}

/// Discard the current outbound Megolm session for a room (issue #538).
pub async fn discard_outbound_group_session(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    room.discard_room_key()
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

/// Create the outbound Megolm session if needed and pre-share it with all
/// active members (issue #538). This is the normal preshare path, run with
/// the per-room transport lock held.
pub async fn preshare_outbound_group_session(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<(), MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    room.preshare_room_key()
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

/// Return the current outbound session's message index, if a session exists
/// (issue #538).
pub async fn current_outbound_group_session_index(
    session: &MatrixClientSession,
    room_id: &str,
) -> Result<Option<u32>, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    room.current_outbound_group_session_message_index()
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

/// Manually share the current outbound session's index-0 room key to every
/// eligible recipient device (issue #538 diagnostic control). `cancellation`
/// and `validate` are checked before every HTTP effect.
pub async fn share_index0_room_key(
    session: &MatrixClientSession,
    room_id: &str,
    cancellation: &mut tokio::sync::broadcast::Receiver<()>,
    validate: impl Fn() -> bool + Send + Sync,
) -> Result<MatrixIndex0ShareSummary, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let summary = room
        .share_index0_room_key(cancellation, validate)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    Ok(MatrixIndex0ShareSummary {
        outcome: match summary.outcome {
            matrix_sdk_base::crypto::ManualIndex0ShareOutcome::Completed => {
                MatrixIndex0ShareOutcome::Completed
            }
            matrix_sdk_base::crypto::ManualIndex0ShareOutcome::RefusedNotEncrypted => {
                MatrixIndex0ShareOutcome::RefusedNotEncrypted
            }
            matrix_sdk_base::crypto::ManualIndex0ShareOutcome::RefusedIndexAdvanced => {
                MatrixIndex0ShareOutcome::RefusedIndexAdvanced
            }
            matrix_sdk_base::crypto::ManualIndex0ShareOutcome::NoSession => {
                MatrixIndex0ShareOutcome::NoSession
            }
            matrix_sdk_base::crypto::ManualIndex0ShareOutcome::NoRecipients => {
                MatrixIndex0ShareOutcome::NoRecipients
            }
            matrix_sdk_base::crypto::ManualIndex0ShareOutcome::PolicyBlocked => {
                MatrixIndex0ShareOutcome::PolicyBlocked
            }
            matrix_sdk_base::crypto::ManualIndex0ShareOutcome::CancelledStale => {
                MatrixIndex0ShareOutcome::CancelledStale
            }
            matrix_sdk_base::crypto::ManualIndex0ShareOutcome::Deadline => {
                MatrixIndex0ShareOutcome::Deadline
            }
            matrix_sdk_base::crypto::ManualIndex0ShareOutcome::Failed => {
                MatrixIndex0ShareOutcome::Failed
            }
        },
        message_index_before: summary.message_index_before,
        message_index_after: summary.message_index_after,
        own_eligible: summary.own_eligible,
        own_accepted: summary.own_accepted,
        own_missing: summary.own_missing,
        peer_eligible: summary.peer_eligible,
        peer_accepted: summary.peer_accepted,
        peer_missing: summary.peer_missing,
        peer_users_with_zero_accepted: summary.peer_users_with_zero_accepted,
        claim: match summary.claim {
            matrix_sdk_base::crypto::ManualClaimOutcome::NotNeeded => {
                MatrixIndex0ClaimOutcome::NotNeeded
            }
            matrix_sdk_base::crypto::ManualClaimOutcome::Succeeded => {
                MatrixIndex0ClaimOutcome::Succeeded
            }
            matrix_sdk_base::crypto::ManualClaimOutcome::Failed => MatrixIndex0ClaimOutcome::Failed,
            matrix_sdk_base::crypto::ManualClaimOutcome::Deadline => {
                MatrixIndex0ClaimOutcome::Deadline
            }
        },
        elapsed_ms: summary.elapsed_ms,
        room_event_sent: summary.room_event_sent,
        index0_consumed: summary.index0_consumed,
    })
}

/// Manually resend index-0 recovery material for the current outbound session
/// (issue #541 diagnostic control).
pub async fn resend_index0_room_key(
    session: &MatrixClientSession,
    room_id: &str,
    cancellation: &mut tokio::sync::broadcast::Receiver<()>,
    validate: impl Fn() -> bool + Send + Sync,
) -> Result<MatrixIndex0ResendSummary, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let summary = room
        .resend_index0_room_key(cancellation, validate)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    Ok(MatrixIndex0ResendSummary {
        outcome: match summary.outcome {
            matrix_sdk_base::crypto::ManualIndex0ResendOutcome::Completed => {
                MatrixIndex0ResendOutcome::Completed
            }
            matrix_sdk_base::crypto::ManualIndex0ResendOutcome::RefusedNotEncrypted => {
                MatrixIndex0ResendOutcome::RefusedNotEncrypted
            }
            matrix_sdk_base::crypto::ManualIndex0ResendOutcome::NoSession => {
                MatrixIndex0ResendOutcome::NoSession
            }
            matrix_sdk_base::crypto::ManualIndex0ResendOutcome::InboundSessionMissing => {
                MatrixIndex0ResendOutcome::InboundSessionMissing
            }
            matrix_sdk_base::crypto::ManualIndex0ResendOutcome::InboundIndexAdvanced => {
                MatrixIndex0ResendOutcome::InboundIndexAdvanced
            }
            matrix_sdk_base::crypto::ManualIndex0ResendOutcome::OriginalLedgerMissing => {
                MatrixIndex0ResendOutcome::OriginalLedgerMissing
            }
            matrix_sdk_base::crypto::ManualIndex0ResendOutcome::NoRecipients => {
                MatrixIndex0ResendOutcome::NoRecipients
            }
            matrix_sdk_base::crypto::ManualIndex0ResendOutcome::PolicyBlocked => {
                MatrixIndex0ResendOutcome::PolicyBlocked
            }
            matrix_sdk_base::crypto::ManualIndex0ResendOutcome::StaleIdentityRefused => {
                MatrixIndex0ResendOutcome::StaleIdentityRefused
            }
            matrix_sdk_base::crypto::ManualIndex0ResendOutcome::CancelledStale => {
                MatrixIndex0ResendOutcome::CancelledStale
            }
            matrix_sdk_base::crypto::ManualIndex0ResendOutcome::Deadline => {
                MatrixIndex0ResendOutcome::Deadline
            }
            matrix_sdk_base::crypto::ManualIndex0ResendOutcome::Failed => {
                MatrixIndex0ResendOutcome::Failed
            }
        },
        message_index_before: summary.message_index_before,
        message_index_after: summary.message_index_after,
        peer_ledger: summary.peer_ledger,
        peer_sender_key_changed: summary.peer_sender_key_changed,
        peer_eligible: summary.peer_eligible,
        peer_accepted: summary.peer_accepted,
        peer_missing: summary.peer_missing,
        policy_blocked: summary.policy_blocked,
        inbound_first_known_index: summary.inbound_first_known_index,
        claim: match summary.claim {
            matrix_sdk_base::crypto::ManualClaimOutcome::NotNeeded => {
                MatrixIndex0ClaimOutcome::NotNeeded
            }
            matrix_sdk_base::crypto::ManualClaimOutcome::Succeeded => {
                MatrixIndex0ClaimOutcome::Succeeded
            }
            matrix_sdk_base::crypto::ManualClaimOutcome::Failed => MatrixIndex0ClaimOutcome::Failed,
            matrix_sdk_base::crypto::ManualClaimOutcome::Deadline => {
                MatrixIndex0ClaimOutcome::Deadline
            }
        },
        elapsed_ms: summary.elapsed_ms,
        room_event_sent: summary.room_event_sent,
        index0_consumed: summary.index0_consumed,
    })
}

/// Manually rotate the outbound Megolm session and confirm the fresh session
/// is at message index 0 (issue #538 diagnostic control).
pub async fn force_new_outbound_session(
    session: &MatrixClientSession,
    room_id: &str,
    cancellation: &mut tokio::sync::broadcast::Receiver<()>,
    validate: impl Fn() -> bool + Send + Sync,
) -> Result<MatrixForceNewSessionSummary, MatrixRoomOperationError> {
    let room = matrix_room(session, room_id)?;
    let summary = room
        .force_new_outbound_session(cancellation, validate)
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)?;
    Ok(MatrixForceNewSessionSummary {
        outcome: match summary.outcome {
            matrix_sdk_base::crypto::ManualForceNewOutcome::Completed => {
                MatrixForceNewSessionOutcome::Completed
            }
            matrix_sdk_base::crypto::ManualForceNewOutcome::RefusedNotEncrypted => {
                MatrixForceNewSessionOutcome::RefusedNotEncrypted
            }
            matrix_sdk_base::crypto::ManualForceNewOutcome::CancelledStale => {
                MatrixForceNewSessionOutcome::CancelledStale
            }
            matrix_sdk_base::crypto::ManualForceNewOutcome::Failed => {
                MatrixForceNewSessionOutcome::Failed
            }
            matrix_sdk_base::crypto::ManualForceNewOutcome::Deadline => {
                MatrixForceNewSessionOutcome::Deadline
            }
        },
        previous_session_exists: summary.previous_session_exists,
        fresh_session_created: summary.fresh_session_created,
        message_index: summary.message_index,
        elapsed_ms: summary.elapsed_ms,
    })
}

pub async fn request_room_key_for_event(
    session: &MatrixClientSession,
    room_id: &str,
    event: &Raw<AnySyncTimelineEvent>,
) -> Result<(), MatrixRoomOperationError> {
    let room_id = matrix_sdk::ruma::RoomId::parse(room_id)
        .map_err(|_| MatrixRoomOperationError::InvalidRoomId)?;
    session
        .client()
        .request_room_key_for_event(event, room_id.as_ref())
        .await
        .map_err(MatrixRoomOperationError::from_sdk_error)
}

/// Privacy-safe snapshot of the receive-side room-key handling state: the
/// crypto-machine counters plus the event-cache late-decryption counters and
/// health. Contains only counts, booleans, and closed tokens.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixRoomKeyReceiveDiagnostics {
    /// Crypto-machine receive counters (ingress, Olm, merge decisions).
    pub crypto: matrix_sdk::encryption::RoomKeyReceiveCounters,
    /// Event-cache late-decryption counters and health.
    pub late_decryption: matrix_sdk::event_cache::RoomKeyLateDecryptionDiagnostics,
}

/// Snapshot the privacy-safe receive-side room-key diagnostics for a session.
pub async fn room_key_receive_diagnostics(
    session: &MatrixClientSession,
) -> MatrixRoomKeyReceiveDiagnostics {
    let client = session.client();
    let crypto = client.encryption().room_key_receive_counters().await;
    let late_decryption = client.event_cache().room_key_receive_diagnostics();
    MatrixRoomKeyReceiveDiagnostics {
        crypto,
        late_decryption,
    }
}

/// Issue a bounded local late-decryption retry for the given room and session
/// IDs, using the SDK event-cache redecryptor. This requests no new keys and
/// redistributes nothing; it only asks the redecryptor to re-attempt decryption
/// of the events it already holds for those sessions.
pub fn request_late_decryption(
    session: &MatrixClientSession,
    room_id: &str,
    utd_session_ids: impl IntoIterator<Item = String>,
) {
    use matrix_sdk::event_cache::DecryptionRetryRequest;

    let Ok(room_id) = matrix_sdk::ruma::OwnedRoomId::try_from(room_id) else {
        return;
    };
    let request = DecryptionRetryRequest {
        room_id,
        utd_session_ids: utd_session_ids.into_iter().collect(),
        refresh_info_session_ids: Default::default(),
    };
    session.client().event_cache().request_decryption(request);
}

/// Subscribe to the SDK event-cache redecryptor reports (Lagging,
/// BackupAvailable, ResolvedUtds). Used by the runtime to drive bounded local
/// late-decryption retries.
pub fn late_decryption_report_stream(
    session: &MatrixClientSession,
) -> impl futures_util::Stream<
    Item = std::result::Result<
        matrix_sdk::event_cache::RedecryptorReport,
        matrix_sdk::event_cache::BroadcastStreamRecvError,
    >,
> + use<> {
    let client = session.client();
    client.event_cache().subscribe_to_decryption_reports()
}

fn map_sdk_recovery_state(
    state: matrix_sdk::encryption::recovery::RecoveryState,
) -> E2eeRecoveryState {
    match state {
        matrix_sdk::encryption::recovery::RecoveryState::Unknown => E2eeRecoveryState::Unknown,
        matrix_sdk::encryption::recovery::RecoveryState::Enabled => E2eeRecoveryState::Enabled,
        matrix_sdk::encryption::recovery::RecoveryState::Disabled => E2eeRecoveryState::Disabled,
        matrix_sdk::encryption::recovery::RecoveryState::Incomplete => {
            E2eeRecoveryState::Incomplete
        }
    }
}

#[cfg(test)]
mod room_key_receive_diagnostics_tests {
    use super::record_room_key_receive_diagnostic;
    use koushi_diagnostics::DiagnosticCounterContext;
    use matrix_sdk::encryption::{
        ForwardedRoomKeyAuthOutcome, RoomKeyIngressKind, RoomKeyMergeDecision,
        RoomKeyReceiveDiagnostic, RoomKeyReceiveDiagnosticKind,
    };

    #[test]
    fn receive_diagnostic_records_closed_tokens_only() {
        let cases = [
            RoomKeyReceiveDiagnosticKind::RoomKeyIngress {
                kind: RoomKeyIngressKind::Direct,
            },
            RoomKeyReceiveDiagnosticKind::RoomKeyIngress {
                kind: RoomKeyIngressKind::Forwarded,
            },
            RoomKeyReceiveDiagnosticKind::ToDeviceOlmFailed,
            RoomKeyReceiveDiagnosticKind::ToDeviceOlmWedged,
            RoomKeyReceiveDiagnosticKind::ToDeviceDehydratedRejected,
            RoomKeyReceiveDiagnosticKind::ToDeviceMalformed,
            RoomKeyReceiveDiagnosticKind::RoomKeyUnsupportedAlgorithm,
            RoomKeyReceiveDiagnosticKind::ForwardedRoomKeyAuth {
                outcome: ForwardedRoomKeyAuthOutcome::RejectedNoMatchingRequest,
            },
            RoomKeyReceiveDiagnosticKind::ForwardedRoomKeyAuth {
                outcome: ForwardedRoomKeyAuthOutcome::RejectedUntrustedSender,
            },
            RoomKeyReceiveDiagnosticKind::ForwardedRoomKeyAuth {
                outcome: ForwardedRoomKeyAuthOutcome::UnsupportedAlgorithm,
            },
            RoomKeyReceiveDiagnosticKind::ForwardedRoomKeyAuth {
                outcome: ForwardedRoomKeyAuthOutcome::Accepted,
            },
            RoomKeyReceiveDiagnosticKind::Merge {
                decision: RoomKeyMergeDecision::AcceptedNew,
            },
            RoomKeyReceiveDiagnosticKind::Merge {
                decision: RoomKeyMergeDecision::AcceptedImproved,
            },
            RoomKeyReceiveDiagnosticKind::Merge {
                decision: RoomKeyMergeDecision::DuplicateIgnored,
            },
            RoomKeyReceiveDiagnosticKind::Merge {
                decision: RoomKeyMergeDecision::WorseIgnored,
            },
            RoomKeyReceiveDiagnosticKind::Merge {
                decision: RoomKeyMergeDecision::UnconnectedRejected,
            },
            RoomKeyReceiveDiagnosticKind::Merge {
                decision: RoomKeyMergeDecision::InvalidSessionKey,
            },
            RoomKeyReceiveDiagnosticKind::Merge {
                decision: RoomKeyMergeDecision::StoreFailed,
            },
        ];
        // Hold the diagnostic lock and use the detail ring only (no
        // synthesized aggregate counter records) so parallel tests cannot
        // perturb the count.
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
            .records
            .len();
        let counters = DiagnosticCounterContext::new();
        for kind in cases {
            record_room_key_receive_diagnostic(&counters, RoomKeyReceiveDiagnostic { kind });
        }

        let snapshot = koushi_diagnostics::test_support::detail_snapshot();
        let receive_records: Vec<_> = snapshot
            .records
            .iter()
            .skip(diagnostic_start)
            .filter(|record| record.event.source == "core.room_key_receive")
            .collect();
        assert_eq!(receive_records.len(), cases.len());
        for record in receive_records {
            let text = format!("{:?}", record.event);
            assert!(
                !text.contains('@') && !text.contains('!') && !text.contains("http"),
                "privacy leak in receive diagnostic: {text}"
            );
        }
    }
}

#[cfg(test)]
mod room_key_member_reload_diagnostics_tests {
    use super::{
        MatrixRoomKeyRotationReason, map_room_key_rotation_reason,
        record_room_key_member_reload_diagnostic, record_room_key_rotation_diagnostic,
    };
    use koushi_diagnostics::{DiagnosticCounterContext, DiagnosticValue, test_support};
    use matrix_sdk::encryption::{
        RoomKeyCreationOutcome, RoomKeyDiagnosticAlias, RoomKeyFirstShareOutcome,
        RoomKeyMemberReloadDiagnostic, RoomKeyMemberReloadDiscardOutcome,
        RoomKeyRotationDiagnostic, RoomKeyRotationReason,
    };

    #[test]
    fn every_sdk_rotation_reason_maps_to_the_closed_desktop_reason() {
        use RoomKeyRotationReason as Sdk;
        for (sdk, expected) in [
            (Sdk::Initial, MatrixRoomKeyRotationReason::Initial),
            (Sdk::ExpiredTime, MatrixRoomKeyRotationReason::ExpiredTime),
            (
                Sdk::ExpiredMessageCount,
                MatrixRoomKeyRotationReason::ExpiredMessageCount,
            ),
            (
                Sdk::MembershipOrDeviceChange,
                MatrixRoomKeyRotationReason::MembershipOrDeviceChange,
            ),
            (
                Sdk::EncryptionSettingsChanged,
                MatrixRoomKeyRotationReason::EncryptionSettingsChanged,
            ),
            (
                Sdk::ExplicitDiscard,
                MatrixRoomKeyRotationReason::ExplicitDiscard,
            ),
            (
                Sdk::FullMemberListReload,
                MatrixRoomKeyRotationReason::FullMemberListReload,
            ),
            (
                Sdk::RoomSubscription,
                MatrixRoomKeyRotationReason::RoomSubscription,
            ),
            (
                Sdk::LimitedSyncResponse,
                MatrixRoomKeyRotationReason::LimitedSyncResponse,
            ),
            (
                Sdk::KeyShareFailure,
                MatrixRoomKeyRotationReason::KeyShareFailure,
            ),
            (Sdk::StoreMissing, MatrixRoomKeyRotationReason::StoreMissing),
            (Sdk::Invalidated, MatrixRoomKeyRotationReason::Invalidated),
            (Sdk::Unknown, MatrixRoomKeyRotationReason::Unknown),
        ] {
            assert_eq!(map_room_key_rotation_reason(sdk), expected);
        }
    }

    #[test]
    fn member_reload_and_rotation_records_are_privacy_safe_and_correlated() {
        let _guard = test_support::lock();
        let counters = DiagnosticCounterContext::new();
        koushi_diagnostics::reset_rotation_ledger();
        let diagnostic_start = test_support::detail_snapshot().records.len();

        record_room_key_member_reload_diagnostic(
            &counters,
            RoomKeyMemberReloadDiagnostic {
                room: RoomKeyDiagnosticAlias::new(3),
                reason: RoomKeyRotationReason::RoomSubscription,
                members_were_synced_before_invalidation: Some(true),
                invalidation_count_bucket: 2,
                invalidation_age_ms: Some(42),
                request_elapsed_ms: Some(17),
                response_member_count_bucket: 4,
                processing_elapsed_ms: 3,
                discard_outcome: RoomKeyMemberReloadDiscardOutcome::Discarded,
            },
        );
        record_room_key_rotation_diagnostic(
            &counters,
            RoomKeyRotationDiagnostic {
                room: RoomKeyDiagnosticAlias::new(3),
                previous_session: Some(RoomKeyDiagnosticAlias::new(7)),
                new_session: Some(RoomKeyDiagnosticAlias::new(8)),
                reason: RoomKeyRotationReason::RoomSubscription,
                creation_outcome: RoomKeyCreationOutcome::Created,
                first_share_outcome: RoomKeyFirstShareOutcome::Pending,
                first_send_correlation_present: false,
                discard_elapsed_ms: Some(9),
                elapsed_ms: 1,
            },
        );

        let snapshot = test_support::detail_snapshot();
        let records: Vec<_> = snapshot.records.iter().skip(diagnostic_start).collect();
        let reload = records
            .iter()
            .find(|record| record.event.source == "core.room_member_reload")
            .expect("member reload diagnostic");
        let rotation_snapshot = test_support::rotation_snapshot();
        let rotation = rotation_snapshot
            .records
            .iter()
            .find(|record| record.event.source == "core.room_key_rotation")
            .expect("rotation diagnostic");
        for record in [*reload, rotation] {
            assert!(record.event.fields.iter().any(|field| {
                field.key == "room_alias"
                    && field.value
                        == DiagnosticValue::OrdinalAlias {
                            kind: "room",
                            ordinal: 3,
                        }
            }));
            let debug = format!("{:?}", record.event);
            assert!(!debug.contains('@') && !debug.contains('!') && !debug.contains("http"));
        }
        assert!(rotation.event.fields.iter().any(|field| {
            field.key == "discard_elapsed_ms" && field.value == DiagnosticValue::Milliseconds(9)
        }));
    }
}

#[cfg(test)]
mod initial_share_diagnostics_tests {
    use super::{
        record_initial_share_diagnostic, record_initial_share_session_diagnostic,
        record_room_key_rotation_diagnostic,
    };
    use koushi_diagnostics::{DiagnosticCounterContext, test_support};
    use matrix_sdk::encryption::{
        InitialShareDeviceClass as Class, InitialShareDeviceDiagnostic,
        InitialShareSessionDiagnostic, InitialShareStage as Stage, RoomKeyCreationOutcome,
        RoomKeyDiagnosticAlias, RoomKeyFirstShareOutcome, RoomKeyRotationDiagnostic,
        RoomKeyRotationReason,
    };

    #[test]
    fn first_event_updates_only_the_matching_rotation_boundary() {
        let _guard = test_support::lock();
        let counters = DiagnosticCounterContext::new();
        koushi_diagnostics::reset_rotation_ledger();
        for session in [8, 9] {
            record_room_key_rotation_diagnostic(
                &counters,
                RoomKeyRotationDiagnostic {
                    room: RoomKeyDiagnosticAlias::new(session - 7),
                    previous_session: None,
                    new_session: Some(RoomKeyDiagnosticAlias::new(session)),
                    reason: RoomKeyRotationReason::Initial,
                    creation_outcome: RoomKeyCreationOutcome::Created,
                    first_share_outcome: RoomKeyFirstShareOutcome::Pending,
                    first_send_correlation_present: false,
                    discard_elapsed_ms: None,
                    elapsed_ms: 1,
                },
            );
        }

        record_initial_share_session_diagnostic(
            &counters,
            InitialShareSessionDiagnostic {
                session: RoomKeyDiagnosticAlias::new(8),
                first_event_message_index: 0,
                all_initial_shares_settled_first: true,
                pending_requests_bucket: 0,
                eligible_own_devices: 1,
                eligible_peer_devices: 1,
                index0_shares_committed: 2,
                after_index0_shares_committed: 0,
                homeserver_accepted_devices: 2,
                created_at_index0: true,
                elapsed_ms: 2,
            },
        );

        let snapshot = test_support::rotation_snapshot();
        assert_eq!(snapshot.records.len(), 2);
        for (record, expected) in snapshot.records.iter().zip([true, false]) {
            assert!(record.event.fields.iter().any(|field| {
                field.key == "first_send_correlation_present"
                    && field.value == koushi_diagnostics::DiagnosticValue::Boolean(expected)
            }));
        }
    }

    fn counter_value(counters: &DiagnosticCounterContext, name: &'static str) -> u64 {
        let snapshot = counters.snapshot();
        snapshot
            .records
            .iter()
            .find(|record| {
                record.event.source == "core.room_key_summary"
                    && record.event.fields.iter().any(|field| {
                        field.key == "name"
                            && field.value == koushi_diagnostics::DiagnosticValue::Token(name)
                    })
            })
            .and_then(|record| {
                record
                    .event
                    .fields
                    .iter()
                    .find_map(|field| match field.value {
                        koushi_diagnostics::DiagnosticValue::Count(count)
                            if field.key == "count" =>
                        {
                            Some(count)
                        }
                        _ => None,
                    })
            })
            .unwrap_or(0)
    }

    fn device_event(class: Class, stage: Stage) -> InitialShareDeviceDiagnostic {
        InitialShareDeviceDiagnostic {
            session: RoomKeyDiagnosticAlias::new(7),
            device: RoomKeyDiagnosticAlias::new(3),
            device_class: class,
            stage,
            elapsed_ms: 12,
        }
    }

    #[test]
    fn initial_share_diagnostic_records_closed_tokens_and_counters() {
        let _guard = test_support::lock();
        let counters = DiagnosticCounterContext::new();
        let diagnostic_start = test_support::detail_snapshot().records.len();

        record_initial_share_diagnostic(
            &counters,
            device_event(Class::VerifiedPeer, Stage::Eligible),
        );
        record_initial_share_diagnostic(&counters, device_event(Class::Unknown, Stage::OlmMissing));
        record_initial_share_diagnostic(
            &counters,
            device_event(Class::Unknown, Stage::OlmEncrypted),
        );
        record_initial_share_diagnostic(
            &counters,
            device_event(Class::Unknown, Stage::OlmEncryptionFailed),
        );
        record_initial_share_diagnostic(&counters, device_event(Class::Unknown, Stage::Withheld));
        record_initial_share_diagnostic(
            &counters,
            device_event(Class::Unknown, Stage::RequestQueued),
        );
        record_initial_share_diagnostic(
            &counters,
            device_event(Class::Unknown, Stage::HomeserverAccepted),
        );
        record_initial_share_diagnostic(
            &counters,
            device_event(Class::Unknown, Stage::RequestFailed),
        );
        record_initial_share_diagnostic(
            &counters,
            device_event(
                Class::Unknown,
                Stage::ShareStateCommitted { message_index: 0 },
            ),
        );
        record_initial_share_diagnostic(
            &counters,
            device_event(
                Class::Unknown,
                Stage::ShareStateCommitted { message_index: 4 },
            ),
        );
        record_initial_share_session_diagnostic(
            &counters,
            InitialShareSessionDiagnostic {
                session: RoomKeyDiagnosticAlias::new(7),
                first_event_message_index: 0,
                all_initial_shares_settled_first: true,
                pending_requests_bucket: 0,
                eligible_own_devices: 0,
                eligible_peer_devices: 1,
                index0_shares_committed: 1,
                after_index0_shares_committed: 1,
                homeserver_accepted_devices: 1,
                created_at_index0: true,
                elapsed_ms: 12,
            },
        );

        assert_eq!(counter_value(&counters, "initial_share_eligible_peer"), 1);
        assert_eq!(counter_value(&counters, "initial_share_eligible_own"), 0);
        assert_eq!(counter_value(&counters, "initial_share_olm_missing"), 1);
        assert_eq!(counter_value(&counters, "initial_share_olm_encrypted"), 1);
        assert_eq!(
            counter_value(&counters, "initial_share_olm_encryption_failed"),
            1
        );
        assert_eq!(counter_value(&counters, "initial_share_withheld"), 1);
        assert_eq!(counter_value(&counters, "initial_share_request_queued"), 1);
        assert_eq!(
            counter_value(&counters, "initial_share_homeserver_accepted"),
            1
        );
        assert_eq!(counter_value(&counters, "initial_share_request_failed"), 1);
        assert_eq!(
            counter_value(&counters, "initial_share_share_committed_index0"),
            1
        );
        assert_eq!(
            counter_value(&counters, "initial_share_share_committed_after_index0"),
            1
        );
        assert_eq!(
            counter_value(&counters, "initial_share_first_event_all_settled"),
            1
        );
        assert_eq!(
            counter_value(&counters, "initial_share_first_event_pending"),
            0
        );
        assert_eq!(
            counter_value(&counters, "initial_share_sessions_at_index0"),
            1
        );
        assert_eq!(
            counter_value(&counters, "initial_share_sessions_after_index0"),
            0
        );

        let snapshot = test_support::detail_snapshot();
        let stage_records: Vec<_> = snapshot
            .records
            .iter()
            .skip(diagnostic_start)
            .filter(|record| record.event.source == "core.initial_share")
            .collect();
        // 10 device stages + 1 session summary.
        assert_eq!(stage_records.len(), 11);
        let stage_tokens: Vec<_> = stage_records
            .iter()
            .filter(|record| record.event.stage == "stage")
            .map(|record| {
                record
                    .event
                    .fields
                    .iter()
                    .find(|field| field.key == "stage")
                    .and_then(|field| match &field.value {
                        koushi_diagnostics::DiagnosticValue::Token(token) => Some(*token),
                        _ => None,
                    })
                    .expect("stage token")
            })
            .collect();
        for token in [
            "eligible",
            "olm_missing",
            "olm_encrypted",
            "olm_encryption_failed",
            "withheld",
            "request_queued",
            "homeserver_accepted",
            "request_failed",
            "share_state_committed",
            "share_state_committed",
        ] {
            assert!(stage_tokens.contains(&token), "missing stage token {token}");
        }
    }

    #[test]
    fn initial_share_diagnostics_never_expose_private_values() {
        let _guard = test_support::lock();
        let counters = DiagnosticCounterContext::new();
        let diagnostic_start = test_support::detail_snapshot().records.len();

        record_initial_share_diagnostic(
            &counters,
            device_event(Class::VerifiedPeer, Stage::Eligible),
        );
        record_initial_share_diagnostic(
            &counters,
            device_event(
                Class::Unknown,
                Stage::ShareStateCommitted { message_index: 0 },
            ),
        );
        record_initial_share_session_diagnostic(
            &counters,
            InitialShareSessionDiagnostic {
                session: RoomKeyDiagnosticAlias::new(7),
                first_event_message_index: 0,
                all_initial_shares_settled_first: true,
                pending_requests_bucket: 0,
                eligible_own_devices: 1,
                eligible_peer_devices: 2,
                index0_shares_committed: 1,
                after_index0_shares_committed: 0,
                homeserver_accepted_devices: 1,
                created_at_index0: true,
                elapsed_ms: 12,
            },
        );

        let snapshot = test_support::detail_snapshot();
        for record in snapshot.records.iter().skip(diagnostic_start) {
            let text = format!("{:?}", record.event);
            assert!(
                !text.contains('@') && !text.contains('!') && !text.contains("http"),
                "privacy leak in initial-share diagnostic: {text}"
            );
            assert!(!text.contains("session_key"), "privacy leak: {text}");
            assert!(!text.contains("ciphertext"), "privacy leak: {text}");
        }
    }

    #[test]
    fn initial_share_counters_survive_detail_ring_eviction() {
        let _guard = test_support::lock();
        let counters = DiagnosticCounterContext::new();

        // The aggregate counter lives outside the bounded detail ring: emit
        // without recording any detail and confirm the counter still exports.
        let detail_before = test_support::detail_snapshot().records.len();
        counters.increment("initial_share_olm_encrypted");
        assert_eq!(
            test_support::detail_snapshot().records.len(),
            detail_before,
            "the counter must not consume detail-ring capacity"
        );
        assert_eq!(counter_value(&counters, "initial_share_olm_encrypted"), 1);
    }
}

#[cfg(test)]
mod index0_reshare_diagnostics_tests {
    use super::record_index0_reshare_diagnostic;
    use koushi_diagnostics::{DiagnosticCounterContext, test_support};
    use matrix_sdk::encryption::{
        Index0InitialShareState as Share, Index0ReshareDiagnostic, Index0ReshareOutcome as Outcome,
        RoomKeyDiagnosticAlias,
    };

    fn counter_value(counters: &DiagnosticCounterContext, name: &'static str) -> u64 {
        let snapshot = counters.snapshot();
        snapshot
            .records
            .iter()
            .find(|record| {
                record.event.source == "core.room_key_summary"
                    && record.event.fields.iter().any(|field| {
                        field.key == "name"
                            && field.value == koushi_diagnostics::DiagnosticValue::Token(name)
                    })
            })
            .and_then(|record| {
                record
                    .event
                    .fields
                    .iter()
                    .find_map(|field| match field.value {
                        koushi_diagnostics::DiagnosticValue::Count(count)
                            if field.key == "count" =>
                        {
                            Some(count)
                        }
                        _ => None,
                    })
            })
            .unwrap_or(0)
    }

    fn record(counters: &DiagnosticCounterContext, outcome: Outcome, initial_share: Share) {
        record_index0_reshare_diagnostic(
            counters,
            Index0ReshareDiagnostic {
                session: RoomKeyDiagnosticAlias::new(7),
                initial_share,
                reshare: outcome,
                eligible_own_bucket: 0,
                eligible_peer_bucket: 1,
                elapsed_ms: 12,
            },
        );
    }

    #[test]
    fn index0_reshare_diagnostic_records_closed_tokens_and_counters() {
        let _guard = test_support::lock();
        let counters = DiagnosticCounterContext::new();
        let diagnostic_start = test_support::detail_snapshot().records.len();

        record(&counters, Outcome::Sent, Share::Accepted);
        record(&counters, Outcome::Failed, Share::Failed);
        record(&counters, Outcome::Deadline, Share::Failed);
        record(&counters, Outcome::Cancelled, Share::Failed);
        record(&counters, Outcome::PolicyBlocked, Share::Failed);
        record(&counters, Outcome::NotNeeded, Share::Withheld);
        record(&counters, Outcome::NotNeeded, Share::NoRecipients);

        assert_eq!(counter_value(&counters, "index0_reshare_sent"), 1);
        assert_eq!(counter_value(&counters, "index0_reshare_failed"), 1);
        assert_eq!(counter_value(&counters, "index0_reshare_deadline"), 1);
        assert_eq!(counter_value(&counters, "index0_reshare_cancelled"), 1);
        assert_eq!(counter_value(&counters, "index0_reshare_policy_blocked"), 1);
        assert_eq!(counter_value(&counters, "index0_reshare_not_needed"), 2);
        assert_eq!(counter_value(&counters, "index0_initial_share_accepted"), 1);
        assert_eq!(counter_value(&counters, "index0_initial_share_failed"), 4);
        assert_eq!(counter_value(&counters, "index0_initial_share_withheld"), 1);
        assert_eq!(
            counter_value(&counters, "index0_initial_share_no_recipients"),
            1
        );

        let snapshot = test_support::detail_snapshot();
        let records: Vec<_> = snapshot
            .records
            .iter()
            .skip(diagnostic_start)
            .filter(|record| record.event.source == "core.index0_reshare")
            .collect();
        assert_eq!(records.len(), 7);
        for record in records {
            let text = format!("{:?}", record.event);
            assert!(
                !text.contains('@') && !text.contains('!') && !text.contains("http"),
                "privacy leak in index0 reshare diagnostic: {text}"
            );
        }
    }

    #[test]
    fn index0_reshare_counters_survive_detail_ring_eviction() {
        let _guard = test_support::lock();
        let counters = DiagnosticCounterContext::new();
        let detail_before = test_support::detail_snapshot().records.len();
        counters.increment("index0_reshare_sent");
        assert_eq!(
            test_support::detail_snapshot().records.len(),
            detail_before,
            "the counter must not consume detail-ring capacity"
        );
        assert_eq!(counter_value(&counters, "index0_reshare_sent"), 1);
    }
}

#[cfg(test)]
mod initial_share_repair_diagnostics_tests {
    use super::record_initial_share_repair_diagnostic;
    use koushi_diagnostics::{DiagnosticCounterContext, test_support};
    use matrix_sdk::encryption::{
        InitialShareRepairClaimOutcome as Claim, InitialShareRepairDiagnostic,
        InitialShareRepairOlmState as Olm, InitialShareRepairOutcome as Repair,
        RoomKeyDiagnosticAlias,
    };

    fn counter_value(counters: &DiagnosticCounterContext, name: &'static str) -> u64 {
        let snapshot = counters.snapshot();
        snapshot
            .records
            .iter()
            .find(|record| {
                record.event.source == "core.room_key_summary"
                    && record.event.fields.iter().any(|field| {
                        field.key == "name"
                            && field.value == koushi_diagnostics::DiagnosticValue::Token(name)
                    })
            })
            .and_then(|record| {
                record
                    .event
                    .fields
                    .iter()
                    .find_map(|field| match field.value {
                        koushi_diagnostics::DiagnosticValue::Count(count)
                            if field.key == "count" =>
                        {
                            Some(count)
                        }
                        _ => None,
                    })
            })
            .unwrap_or(0)
    }

    fn record(counters: &DiagnosticCounterContext, claim: Claim, repair: Repair) {
        record_initial_share_repair_diagnostic(
            counters,
            InitialShareRepairDiagnostic {
                session: RoomKeyDiagnosticAlias::new(11),
                initial_olm: Olm::Missing,
                claim,
                repair,
                own_coverage_bucket: 1,
                peer_users_covered_bucket: 1,
                peer_users_zero_coverage_bucket: 1,
                missing_devices_bucket: 1,
                first_event_message_index: None,
                same_session: true,
                elapsed_ms: 9,
            },
        );
    }

    #[test]
    fn issue_523_initial_share_repair_records_closed_tokens_without_identifiers() {
        let _guard = test_support::lock();
        let counters = DiagnosticCounterContext::new();
        let start = test_support::detail_snapshot().records.len();
        record(&counters, Claim::NotNeeded, Repair::Settled);
        record(&counters, Claim::Requested, Repair::WaitingWake);
        record(&counters, Claim::Accepted, Repair::Deadline);
        record(&counters, Claim::Empty, Repair::Cancelled);
        record(&counters, Claim::Invalid, Repair::NoRecipients);
        record(&counters, Claim::NetworkFailed, Repair::Failed);
        record(&counters, Claim::SdkFailed, Repair::Failed);

        for counter in [
            "initial_repair_claim_not_needed",
            "initial_repair_claim_requested",
            "initial_repair_claim_accepted",
            "initial_repair_claim_empty",
            "initial_repair_claim_invalid",
            "initial_repair_claim_network_failed",
            "initial_repair_claim_sdk_failed",
            "initial_repair_settled",
            "initial_repair_waiting_wake",
            "initial_repair_deadline",
            "initial_repair_cancelled",
            "initial_repair_no_recipients",
            "initial_repair_failed",
        ] {
            let expected = if counter == "initial_repair_failed" {
                2
            } else {
                1
            };
            assert_eq!(
                counter_value(&counters, counter),
                expected,
                "missing counter {counter}"
            );
        }
        for record in test_support::detail_snapshot().records.iter().skip(start) {
            let text = format!("{:?}", record.event);
            assert!(!text.contains('@') && !text.contains('!') && !text.contains("http"));
        }
    }
}

#[cfg(test)]
mod encryption_debug_dto_privacy_tests {
    use super::{
        MatrixForceNewSessionOutcome, MatrixForceNewSessionSummary, MatrixIndex0ClaimOutcome,
        MatrixIndex0ResendOutcome, MatrixIndex0ResendSummary, MatrixIndex0ShareOutcome,
        MatrixIndex0ShareSummary,
    };

    fn banned_fragments() -> &'static [&'static str] {
        &[
            "room_id",
            "user_id",
            "device_id",
            "session_id",
            "sender_key",
            "session_key",
            "ciphertext",
            "event_id",
            "txn",
            "identity_key",
            "homeserver",
        ]
    }

    #[test]
    fn index0_share_summary_serializes_without_identifiers_or_key_material() {
        let summary = MatrixIndex0ShareSummary {
            outcome: MatrixIndex0ShareOutcome::Completed,
            message_index_before: Some(0),
            message_index_after: Some(0),
            own_eligible: 1,
            own_accepted: 1,
            own_missing: 0,
            peer_eligible: 2,
            peer_accepted: 1,
            peer_missing: 1,
            peer_users_with_zero_accepted: 1,
            claim: MatrixIndex0ClaimOutcome::Succeeded,
            elapsed_ms: 12,
            room_event_sent: false,
            index0_consumed: false,
        };
        let text = serde_json::to_string(&summary).unwrap();
        for fragment in banned_fragments() {
            assert!(
                !text.contains(fragment),
                "privacy leak: {fragment} in {text}"
            );
        }
        assert!(
            !text.contains('@') && !text.contains('!'),
            "identifier leak: {text}"
        );
    }

    #[test]
    fn force_new_summary_serializes_without_identifiers_or_key_material() {
        let summary = MatrixForceNewSessionSummary {
            outcome: MatrixForceNewSessionOutcome::Completed,
            previous_session_exists: true,
            fresh_session_created: true,
            message_index: Some(0),
            elapsed_ms: 9,
        };
        let text = serde_json::to_string(&summary).unwrap();
        for fragment in banned_fragments() {
            assert!(
                !text.contains(fragment),
                "privacy leak: {fragment} in {text}"
            );
        }
        assert!(
            !text.contains('@') && !text.contains('!'),
            "identifier leak: {text}"
        );
    }

    #[test]
    fn index0_resend_summary_serializes_without_identifiers_or_key_material() {
        let summary = MatrixIndex0ResendSummary {
            outcome: MatrixIndex0ResendOutcome::Completed,
            message_index_before: Some(8),
            message_index_after: Some(8),
            peer_ledger: 2,
            peer_sender_key_changed: 0,
            peer_eligible: 2,
            peer_accepted: 2,
            peer_missing: 0,
            policy_blocked: 0,
            inbound_first_known_index: Some(0),
            claim: MatrixIndex0ClaimOutcome::NotNeeded,
            elapsed_ms: 12,
            room_event_sent: false,
            index0_consumed: false,
        };
        let text = serde_json::to_string(&summary).unwrap();
        for fragment in banned_fragments()
            .iter()
            .filter(|fragment| **fragment != "sender_key")
        {
            assert!(
                !text.contains(fragment),
                "privacy leak: {fragment} in {text}"
            );
        }
        assert!(
            !text.contains('@') && !text.contains('!'),
            "identifier leak: {text}"
        );
    }

    #[test]
    fn index0_share_outcome_and_claim_tokens_are_closed() {
        for outcome in [
            MatrixIndex0ShareOutcome::Completed,
            MatrixIndex0ShareOutcome::RefusedNotEncrypted,
            MatrixIndex0ShareOutcome::RefusedIndexAdvanced,
            MatrixIndex0ShareOutcome::NoSession,
            MatrixIndex0ShareOutcome::NoRecipients,
            MatrixIndex0ShareOutcome::PolicyBlocked,
            MatrixIndex0ShareOutcome::CancelledStale,
            MatrixIndex0ShareOutcome::Deadline,
            MatrixIndex0ShareOutcome::Failed,
        ] {
            let text = serde_json::to_string(&outcome).unwrap();
            assert!(
                text.starts_with('"') && text.ends_with('"'),
                "open token: {text}"
            );
            assert!(
                !text.contains('@') && !text.contains('!'),
                "identifier leak: {text}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::has_stale_authoritative_device_signature;

    #[test]
    fn recovery_key_path_uses_sdk_signature_publication_only() {
        let source = include_str!("e2ee.rs");
        let recovery_body = crate::test_source::item_body(source, "pub async fn recover_e2ee");

        assert!(
            !recovery_body.contains("prepare_current_device_registration"),
            "recovery must never republish identity keys for an existing device ID"
        );
        assert!(
            !recovery_body.contains("force_upload_device_keys"),
            "recovery must not replace device identity keys out of band"
        );
        assert!(recovery_body.contains("recover_and_fix_backup"));
        assert!(!recovery_body.contains(".recover(request.secret.expose_secret())"));
        assert!(
            !recovery_body.contains("republish_current_device_keys_after_recovery"),
            "SDK recovery already publishes the cross-signature through /keys/signatures/upload"
        );
        assert!(
            !recovery_body.contains("post_recovery_device_republish"),
            "recovery must not mutate device keys after SDK signature publication"
        );
        assert!(
            recovery_body.contains("get_own_device"),
            "recovery key proof must inspect current device signing state"
        );
        assert!(
            recovery_body.contains("post_recovery_own_device_inspected"),
            "recovery must diagnose the SDK-refreshed own-device projection"
        );
        assert!(
            recovery_body.contains("inspect_current_device_signature_state"),
            "recovery must compare authoritative device signatures with the local projection"
        );
        assert!(
            recovery_body.contains("is_cross_signed_by_owner"),
            "recovery must require the SDK-refreshed owner cross-signature"
        );
        assert!(
            recovery_body.contains("record_recovery_verification_event"),
            "recovery key proof must emit stderr diagnostics before UI diagnostics are available"
        );
    }
    #[test]
    fn recovery_detects_a_stale_authoritative_device_signature() {
        use matrix_sdk::encryption::recovery::RecoveryDeviceSignatureInspection;

        let stale = RecoveryDeviceSignatureInspection {
            authoritative_self_signing_signature_present: true,
            authoritative_self_signing_signature_parseable: true,
            authoritative_self_signing_signature_valid: false,
            cached_self_signing_key_matches_authoritative: true,
            cached_signed_content_matches_authoritative: true,
            ..Default::default()
        };
        assert!(has_stale_authoritative_device_signature(&stale));

        let repaired = RecoveryDeviceSignatureInspection {
            authoritative_self_signing_signature_valid: true,
            ..stale
        };
        assert!(!has_stale_authoritative_device_signature(&repaired));
    }
    #[test]
    fn recovery_sdk_records_standard_signature_round_trip_diagnostics() {
        let devices_source = include_str!(
            "../../../vendor/matrix-rust-sdk/crates/matrix-sdk/src/encryption/identities/devices.rs"
        );
        let secret_store_source = include_str!(
            "../../../vendor/matrix-rust-sdk/crates/matrix-sdk/src/encryption/secret_storage/secret_store.rs"
        );

        assert!(
            devices_source.contains("verify_with_diagnostics"),
            "the exact signed device target must be retained across the standard upload"
        );
        assert!(
            secret_store_source.contains("standard_signature_round_trip_finished"),
            "the standard recovery path must compare its upload target with the refreshed device"
        );
        assert!(
            secret_store_source.contains("preupload_self_signing_signature_valid"),
            "diagnostics must distinguish invalid local signing from server-side mutation"
        );
        assert!(
            secret_store_source.contains("signed_content_matches_refreshed"),
            "diagnostics must compare the canonical signed content before and after upload"
        );
        assert!(
            secret_store_source.contains("self_signing_key_id_matches_refreshed"),
            "diagnostics must distinguish a stale self-signing key generation"
        );
        assert!(
            secret_store_source.contains("preupload_signature_matches_refreshed"),
            "diagnostics must distinguish server-side signature replacement"
        );
        assert!(
            secret_store_source.contains("preupload_signature_valid_with_refreshed_key"),
            "diagnostics must cross-check the upload with the authoritative key generation"
        );
        assert!(
            !secret_store_source.contains("preupload_signature_value"),
            "diagnostics must never expose raw signatures"
        );
    }
    #[test]
    fn recovery_diagnostics_classify_signature_upload_failures_inside_secret_storage() {
        let error = matrix_sdk::encryption::recovery::RecoveryError::SecretStorage(
            matrix_sdk::encryption::secret_storage::SecretStorageError::Verification(
                matrix_sdk::encryption::identities::ManualVerifyError::SignatureUploadFailures {
                    signed_target_count: 1,
                    signed_key_count: 1,
                    failure_user_count: 1,
                    failure_key_count: 2,
                    invalid_signature_count: 1,
                    other_failure_count: 1,
                    unknown_failure_count: 0,
                },
            ),
        );

        assert_eq!(
            super::recovery_error_kind(&error),
            "signature_upload_failures"
        );
        assert_eq!(
            super::recovery_signature_upload_failure_diagnostics(&error),
            Some(super::SignatureUploadFailureDiagnostics {
                signed_target_count: 1,
                signed_key_count: 1,
                failure_user_count: 1,
                failure_key_count: 2,
                invalid_signature_count: 1,
                other_failure_count: 1,
                unknown_failure_count: 0,
            })
        );
    }
    #[test]
    fn typed_peer_policy_is_all_devices_not_only_trusted() {
        assert!(matches!(
            super::desktop_room_key_recipient_strategy(),
            matrix_sdk_base::crypto::CollectStrategy::AllDevices
        ));
    }
}

#[cfg(test)]
mod encryption_readiness_diagnostics_tests {
    use super::record_encryption_readiness_diagnostic;
    use koushi_diagnostics::{DiagnosticCounterContext, test_support};
    use matrix_sdk::encryption::{
        EncryptionReadinessDiagnostic, EncryptionReadinessOutcome, EncryptionReadinessQueryState,
        EncryptionReadinessSyncState, RoomKeyDiagnosticAlias,
    };

    #[test]
    fn readiness_diagnostic_contains_only_closed_aliases_counts_and_tokens() {
        let _guard = test_support::lock();
        let counters = DiagnosticCounterContext::new();
        let start = test_support::detail_snapshot().records.len();
        record_encryption_readiness_diagnostic(
            &counters,
            EncryptionReadinessDiagnostic {
                room: RoomKeyDiagnosticAlias::new(4),
                session: RoomKeyDiagnosticAlias::new(9),
                generation: 3,
                sync: EncryptionReadinessSyncState::Received,
                query: EncryptionReadinessQueryState::Accepted,
                outcome: EncryptionReadinessOutcome::Ready,
                active_members_bucket: 2,
                returned_devices_bucket: 2,
                eligible_devices_bucket: 1,
                accepted_devices_bucket: 1,
                message_index_bucket: 0,
                registry_evictions: 0,
                retryable: false,
            },
        );

        let snapshot = test_support::detail_snapshot();
        let record = snapshot.records[start..]
            .iter()
            .find(|record| record.event.source == "core.encryption_readiness")
            .expect("readiness record");
        let text = format!("{:?}", record.event);
        for forbidden in [
            "@alice:example.org",
            "DEVICE",
            "!room:example.org",
            "$event",
            "session-secret",
            "https://example.org",
            "curve25519",
            "sync-position",
        ] {
            assert!(
                !text.contains(forbidden),
                "privacy leak: {forbidden}: {text}"
            );
        }
        assert!(text.contains("kind: \"room\""));
        assert!(text.contains("kind: \"session\""));
        assert!(text.contains("received"));
        assert!(text.contains("accepted"));
        assert!(text.contains("ready"));
    }
}

#[cfg(test)]
mod current_device_trust_recheck_classifier_tests {
    use super::{
        CurrentDeviceTrustRecheckError, MatrixClientSession,
        classify_current_device_trust_recheck_error,
    };
    use koushi_state::{SessionAuthenticationMethod, SessionInfo};
    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    use serde_json::json;
    use wiremock::ResponseTemplate;

    async fn session(server: &MatrixMockServer) -> MatrixClientSession {
        let client = server.client_builder().build().await;
        MatrixClientSession::from_client_for_testing(
            client.clone(),
            SessionInfo {
                homeserver: server.server().uri(),
                user_id: client.user_id().expect("mock user id").to_string(),
                device_id: client.device_id().expect("mock device id").to_string(),
                authentication_method: SessionAuthenticationMethod::Unknown,
            },
        )
    }

    #[test]
    fn structured_facts_classify_without_display_parsing() {
        assert_eq!(
            classify_current_device_trust_recheck_error(&matrix_sdk::Error::Timeout),
            CurrentDeviceTrustRecheckError::Network
        );
        assert_eq!(
            classify_current_device_trust_recheck_error(&matrix_sdk::Error::NoOlmMachine),
            CurrentDeviceTrustRecheckError::Sdk
        );
    }

    #[tokio::test]
    async fn unknown_token_keys_query_is_authentication() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
            .records
            .len();
        let server = MatrixMockServer::new().await;
        let session = session(&server).await;
        let _guard = server
            .mock_query_keys()
            .error_unknown_token(false)
            .expect(1)
            .mount_as_scoped()
            .await;
        assert_eq!(
            session.recheck_current_device_trust().await,
            Err(CurrentDeviceTrustRecheckError::Authentication)
        );
        assert!(
            koushi_diagnostics::test_support::detail_snapshot().records[diagnostic_start..]
                .iter()
                .any(|record| {
                    record.event.source == "sdk.current_device_trust_recheck"
                        && koushi_diagnostics::format_event(&record.event)
                            == "stage=finished outcome=failed failure_kind=authentication"
                }),
            "authentication rechecks must record their closed failure kind"
        );
    }

    #[tokio::test]
    async fn structured_forbidden_status_is_authentication_diagnostic() {
        let server = MatrixMockServer::new().await;
        let session = session(&server).await;
        let _guard = server
            .mock_query_keys()
            .respond_with(ResponseTemplate::new(403).set_body_json(json!({
                "errcode": "M_FORBIDDEN",
                "error": "synthetic"
            })))
            .expect(1)
            .mount_as_scoped()
            .await;

        assert_eq!(
            session.recheck_current_device_trust().await,
            Err(CurrentDeviceTrustRecheckError::Authentication)
        );
    }

    #[tokio::test]
    async fn server_keys_query_failure_is_server() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
            .records
            .len();
        let server = MatrixMockServer::new().await;
        let session = session(&server).await;
        let _guard = server
            .mock_query_keys()
            .respond_with(ResponseTemplate::new(500).set_body_json(json!({
                "errcode": "M_UNKNOWN",
                "error": "synthetic"
            })))
            .expect(1)
            .mount_as_scoped()
            .await;
        assert_eq!(
            session.recheck_current_device_trust().await,
            Err(CurrentDeviceTrustRecheckError::Server)
        );
        assert!(
            koushi_diagnostics::test_support::detail_snapshot().records[diagnostic_start..]
                .iter()
                .any(|record| {
                    record.event.source == "sdk.current_device_trust_recheck"
                        && koushi_diagnostics::format_event(&record.event)
                            == "stage=finished outcome=failed failure_kind=server"
                }),
            "server rechecks must record their closed failure kind"
        );
    }
}
