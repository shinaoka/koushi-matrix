use std::fmt;

use serde::{Deserialize, Serialize};

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
    },
    Rejecting {
        info: SessionInfo,
        reason: VerificationGateRejectReason,
    },
    Ready(SessionInfo),
    Locked(SessionInfo),
    LoggingOut,
}

#[derive(Clone, Copy, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LoginAttemptId(u64);

impl LoginAttemptId {
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    pub fn get(self) -> u64 {
        self.0
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

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DelegatedAuthLinks {
    pub registration_url: Option<String>,
    pub account_management_url: Option<String>,
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
pub enum DeviceSessionListState {
    #[default]
    Idle,
    Loading {
        request_id: u64,
    },
    Loaded {
        devices: Vec<DeviceSessionSummary>,
    },
    Failed {
        request_id: u64,
        #[serde(rename = "failureKind")]
        kind: AuthFailureKind,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeviceSessionSummary {
    pub device_ordinal: u64,
    pub display_name: Option<String>,
    pub current: bool,
    pub verified: bool,
    pub inactive: bool,
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
    RenameDevice,
    DeleteDevice,
    DeleteOtherDevices,
    ChangePassword,
    DeactivateAccount,
    ThreePid,
    IdentityServer,
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
