use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SessionLockReason {
    UnknownToken { soft_logout: bool },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SessionState {
    SignedOut,
    Restoring,
    SwitchingAccount {
        info: SessionInfo,
    },
    Authenticating {
        homeserver: String,
        attempt_id: LoginAttemptId,
    },
    Provisional {
        info: SessionInfo,
        phase: ProvisionalPhase,
    },
    AwaitingVerification {
        info: SessionInfo,
        gate: VerificationGateState,
    },
    Verifying {
        info: SessionInfo,
        gate: VerificationGateState,
        method: VerificationMethod,
        flow_id: u64,
        #[serde(default)]
        sas_emojis: Vec<crate::state::SasEmoji>,
    },
    /// The identity bootstrap succeeded and its recovery key is revealed
    /// (#1049). The key itself lives only in the reveal slot
    /// `SecureBackupSetupState::RecoveryKeyReady { request_id: flow_id }`,
    /// never in this widely observed session state.
    AwaitingBootstrapConfirmation {
        info: SessionInfo,
        gate: VerificationGateState,
        flow_id: u64,
    },
    Rejecting {
        info: SessionInfo,
        reason: VerificationGateRejectReason,
    },
    Ready(SessionInfo),
    Locked(SessionInfo),
    CapabilityBlocked {
        info: SessionInfo,
        failure: super::SlidingSyncCapabilityFailureKind,
    },
    LoggingOut,
}

#[derive(Clone, Copy, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct LoginAttemptId {
    connection_id: u64,
    sequence: u64,
}

impl LoginAttemptId {
    pub fn new(connection_id: u64, sequence: u64) -> Self {
        Self {
            connection_id,
            sequence,
        }
    }

    pub fn connection_id(self) -> u64 {
        self.connection_id
    }
    pub fn sequence(self) -> u64 {
        self.sequence
    }
}

impl fmt::Debug for LoginAttemptId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LoginAttemptId(..)")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CurrentDeviceTrustState {
    Unknown,
    Verified,
    Unverified,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ProvisionalPhase {
    CheckingTrust,
    DiscoveringMethods,
    RecheckingTrust {
        #[serde(default, rename = "failureKind")]
        failure: Option<VerificationGateFailureKind>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VerificationGateState {
    pub methods: Vec<VerificationMethodCapability>,
    pub account_kind: VerificationAccountKind,
    #[serde(default, rename = "failureKind")]
    pub failure: Option<VerificationGateFailureKind>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SecureBackupSetupIntent {
    InitialSetup,
    Reenable { confirmed: bool },
    // Replace a lost, unconfirmed recovery key of the existing backup with a
    // NEW one via `recovery().reset_key()` (#927). The previous key stops
    // working, so it requires explicit confirmation.
    ResetRecoveryKey { confirmed: bool },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecureBackupSetupAdmission {
    Allowed,
    ConfirmationRequired,
    FailedNoOp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SecureBackupGateFailureKind {
    Network,
    RateLimited,
    InvalidRecoveryKey,
    BackupKeyMismatch,
    SecretStorageIncomplete,
    ArtifactDelivery,
    Forbidden,
    Timeout,
    Sdk,
    /// The homeserver answered with an error response that no more specific
    /// kind covers. Unlike `Network`, the request did reach the server.
    ServerResponse,
    /// The homeserver rejected the authentication for this request (for
    /// example HTTP 401 / `M_UNKNOWN_TOKEN`). This records the request's
    /// outcome; it does not itself invalidate the session.
    Unauthorized,
}

/// Which Secure Backup operation or stage produced a failure. Closed,
/// privacy-safe vocabulary; never an SDK error string or endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SecureBackupFailureStage {
    /// `Backups::inspect_server_trust()`.
    InspectServerTrust,
    /// `Backups::room_key_counts()`.
    RoomKeyCounts,
    /// The `recovery_key_delivery_pending` marker read or write.
    RecoveryKeyDelivery,
    /// The local cross-signing completeness probe.
    CrossSigningStatus,
    /// The inspection did not finish inside its bounded deadline.
    InspectionDeadline,
    /// No structured stage could be attributed.
    Unknown,
}

/// How a Secure Backup exchange failed. Distinguishes an answer from the
/// server from a request that never received one.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SecureBackupFailureTransport {
    /// The request produced no response (connection, DNS, TLS, or I/O).
    NoResponse,
    /// The homeserver produced a response that rejected the request.
    HttpResponse,
    /// The client gave up waiting for a response.
    Timeout,
    /// A local SDK/crypto/state failure, not a network exchange.
    Local,
}

/// Allowlisted Matrix error categories (`errcode`) carried by a server
/// response. Anything outside this set is `Unknown`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SecureBackupMatrixErrorKind {
    Forbidden,
    UnknownToken,
    MissingToken,
    LimitExceeded,
    Unrecognized,
    BadJson,
    NotFound,
    Unknown,
}

/// Bounded, privacy-safe failure facts for the Secure Backup gate and
/// diagnostics. It never carries SDK error text, URLs, account or room
/// identifiers, response bodies, recovery material, or tokens.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SecureBackupFailureDetail {
    pub stage: SecureBackupFailureStage,
    pub transport: SecureBackupFailureTransport,
    #[serde(default, rename = "httpStatus")]
    pub http_status: Option<u16>,
    #[serde(default, rename = "matrixErrorKind")]
    pub matrix_error_kind: Option<SecureBackupMatrixErrorKind>,
    /// Whether retrying the same inspection could succeed.
    pub retryable: bool,
}

/// A coarse gate failure kind paired with its optional structured detail.
/// The kind drives catalog copy and gate policy; the detail drives the
/// specific explanation and the diagnostic report.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SecureBackupInspectionFailure {
    pub kind: SecureBackupGateFailureKind,
    pub detail: Option<SecureBackupFailureDetail>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingKeyCountBucket {
    Zero,
    One,
    TwoToTen,
    ElevenToOneHundred,
    OverOneHundred,
    Unknown,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SecureBackupGateState {
    #[default]
    Inactive,
    Checking,
    ExistingBackupNeedsRecovery {
        #[serde(default)]
        failure: Option<SecureBackupGateFailureKind>,
    },
    SecureStorageIncomplete,
    SetupRequired,
    ExplicitlyDisabledRequiresSetup,
    CreatingBackup,
    RecoveryKeyDeliveryRequired,
    UploadingExistingKeys {
        pending: PendingKeyCountBucket,
    },
    DegradedRetrying {
        failure: SecureBackupGateFailureKind,
        #[serde(default)]
        detail: Option<SecureBackupFailureDetail>,
    },
    BlockedFailed {
        failure: SecureBackupGateFailureKind,
        #[serde(default)]
        detail: Option<SecureBackupFailureDetail>,
    },
    Ready,
}

impl SecureBackupSetupIntent {
    pub fn admission(self, gate: &SecureBackupGateState) -> SecureBackupSetupAdmission {
        match (self, gate) {
            (Self::InitialSetup, SecureBackupGateState::SetupRequired) => {
                SecureBackupSetupAdmission::Allowed
            }
            (
                Self::ResetRecoveryKey { confirmed: true },
                SecureBackupGateState::RecoveryKeyDeliveryRequired,
            ) => SecureBackupSetupAdmission::Allowed,
            (
                Self::ResetRecoveryKey { confirmed: false },
                SecureBackupGateState::RecoveryKeyDeliveryRequired,
            ) => SecureBackupSetupAdmission::ConfirmationRequired,
            (
                Self::Reenable { confirmed: true },
                SecureBackupGateState::ExplicitlyDisabledRequiresSetup,
            ) => SecureBackupSetupAdmission::Allowed,
            (
                Self::Reenable { confirmed: false },
                SecureBackupGateState::ExplicitlyDisabledRequiresSetup,
            ) => SecureBackupSetupAdmission::ConfirmationRequired,
            _ => SecureBackupSetupAdmission::FailedNoOp,
        }
    }
}

impl SecureBackupGateState {
    pub fn backup_is_ready(&self) -> bool {
        matches!(
            self,
            Self::Ready | Self::UploadingExistingKeys { .. } | Self::DegradedRetrying { .. }
        )
    }

    /// The gate's blocking/degraded failure kind and its optional structured
    /// detail. `ExistingBackupNeedsRecovery` never carries structured detail
    /// because it is a trust verdict, not a failed exchange.
    pub fn failure(
        &self,
    ) -> Option<(
        SecureBackupGateFailureKind,
        Option<SecureBackupFailureDetail>,
    )> {
        match self {
            Self::ExistingBackupNeedsRecovery { failure } => failure.map(|kind| (kind, None)),
            Self::DegradedRetrying { failure, detail }
            | Self::BlockedFailed { failure, detail } => Some((*failure, *detail)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceCleanupAuthMode {
    Legacy,
    OAuth,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceCleanupOfferReason {
    RecoveryFailed,
    NoProofMethod,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceCleanupRemoteOutcome {
    Success,
    AlreadyAbsent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceCleanupFailureKind {
    Network,
    Forbidden,
    Timeout,
    Sdk,
    LocalData,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DeviceCleanupLocalMode {
    RemoteRemoved { outcome: DeviceCleanupRemoteOutcome },
    RemoteMayRemain,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DeviceCleanupState {
    #[default]
    Idle,
    Offered {
        reason: DeviceCleanupOfferReason,
    },
    ResolvingRemote {
        request_id: u64,
    },
    RemovingRemote {
        request_id: u64,
        auth_mode: DeviceCleanupAuthMode,
    },
    AwaitingUia {
        request_id: u64,
        flow_id: u64,
    },
    RemoteFailed {
        request_id: u64,
        auth_mode: DeviceCleanupAuthMode,
        #[serde(rename = "failureKind")]
        failure: DeviceCleanupFailureKind,
    },
    ResettingLocal {
        request_id: u64,
        mode: DeviceCleanupLocalMode,
    },
    LocalResetFailed {
        request_id: u64,
        mode: DeviceCleanupLocalMode,
        #[serde(rename = "failureKind")]
        failure: DeviceCleanupFailureKind,
    },
    ErasingLocalAnyway {
        request_id: u64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VerificationMethodCapability {
    ExistingDeviceSas,
    RecoveryKey,
    SecurityPhrase,
    Bootstrap,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VerificationMethod {
    ExistingDeviceSas,
    RecoveryKey,
    SecurityPhrase,
    Bootstrap,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VerificationAccountKind {
    ExistingIdentity,
    NewIdentity,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VerificationGateFailureKind {
    Network,
    Cancelled,
    Mismatch,
    Forbidden,
    Timeout,
    Sdk,
    NoProofMethod,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VerificationGateRejectReason {
    ExistingIdentityWithoutProof,
    UserRejected,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecoveryMethod {
    RecoveryKey,
    SecurityPhrase,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub homeserver: String,
    pub user_id: String,
    pub device_id: String,
    #[serde(default)]
    pub authentication_method: SessionAuthenticationMethod,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionAuthenticationMethod {
    Password,
    Sso,
    #[serde(rename = "oauth")]
    OAuth,
    Token,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AuthDiscoveryState {
    Unknown,
    Discovering {
        homeserver: String,
    },
    Ready {
        homeserver: String,
        flows: Vec<LoginFlow>,
        #[serde(default)]
        delegated: DelegatedAuthLinks,
    },
    Failed {
        homeserver: String,
        #[serde(rename = "failureKind")]
        kind: AuthFailureKind,
    },
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountManagementUrl(String);

impl AccountManagementUrl {
    /// Wrap an HTTP(S) destination already validated by the SDK boundary.
    pub fn from_validated(value: String) -> Self {
        Self(value)
    }
}

impl std::ops::Deref for AccountManagementUrl {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::fmt::Debug for AccountManagementUrl {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AccountManagementUrl(..)")
    }
}

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DelegatedAuthLinks {
    pub registration_url: Option<String>,
}

/// Redact the URL values from Debug: they cross the snapshot/browser boundary
/// and may contain sensitive query data even though credentials are rejected.
impl std::fmt::Debug for DelegatedAuthLinks {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DelegatedAuthLinks")
            .field(
                "registration_url",
                &self.registration_url.as_deref().map(|_| "Url(..)"),
            )
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuthFailureKind {
    Network,
    Unsupported,
    Cancelled,
    Forbidden,
    Timeout,
    Sdk,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LoginFlow {
    pub kind: LoginFlowKind,
    pub delegated_oidc_compatibility: bool,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LoginFlowKind {
    Password,
    Sso,
    Oidc,
    Token,
    Unknown(String),
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AccountManagementState {
    #[default]
    Idle,
    Working {
        request_id: u64,
        operation: AccountManagementOperation,
    },
    AwaitingUia {
        request_id: u64,
        flow_id: u64,
        operation: AccountManagementOperation,
    },
    Succeeded {
        request_id: u64,
        operation: AccountManagementOperation,
    },
    Failed {
        request_id: u64,
        operation: AccountManagementOperation,
        #[serde(rename = "failureKind")]
        kind: AuthFailureKind,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountManagementOperation {
    ChangePassword,
    DeactivateAccount,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CapabilityState {
    #[default]
    Unknown,
    Enabled,
    Disabled,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AccountManagementCapabilities {
    pub change_password: CapabilityState,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum QrLoginState {
    #[default]
    Idle,
    CheckingCapability {
        request_id: u64,
    },
    Unavailable,
    Displaying {
        request_id: u64,
    },
    Scanning {
        request_id: u64,
    },
    Verified {
        request_id: u64,
    },
    Failed {
        request_id: u64,
        #[serde(rename = "failureKind")]
        kind: AuthFailureKind,
    },
}

/// Rust-owned state machine for soft-logout re-authentication (MSC2697).
/// Product state contains only request ids and coarse failure kinds;
/// passwords and session secrets remain command-boundary values.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SoftLogoutReauthState {
    #[default]
    Idle,
    Authenticating {
        request_id: u64,
    },
    Succeeded {
        request_id: u64,
    },
    Failed {
        request_id: u64,
        #[serde(rename = "failureKind")]
        kind: AuthFailureKind,
    },
}
