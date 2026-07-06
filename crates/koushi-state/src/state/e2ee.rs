use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum E2eeRecoveryState {
    Unknown,
    Enabled,
    Disabled,
    Incomplete,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct E2eeTrustState {
    pub verification: VerificationFlowState,
    pub cross_signing: CrossSigningStatus,
    pub key_backup: KeyBackupStatus,
    pub identity_reset: IdentityResetState,
    #[serde(default)]
    pub key_management: E2eeKeyManagementState,
    pub devices: Vec<DeviceTrustSummary>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct E2eeKeyManagementState {
    pub room_key_export: RoomKeyExportState,
    pub room_key_import: RoomKeyImportState,
    pub secure_backup_setup: SecureBackupSetupState,
    pub passphrase_change: SecureBackupPassphraseChangeState,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RoomKeyExportState {
    #[default]
    Idle,
    Exporting {
        request_id: u64,
    },
    Exported {
        request_id: u64,
        exported_sessions: Option<u64>,
    },
    Failed {
        request_id: u64,
        #[serde(rename = "failureKind")]
        kind: TrustOperationFailureKind,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RoomKeyImportState {
    #[default]
    Idle,
    Importing {
        request_id: u64,
    },
    Imported {
        request_id: u64,
        imported_count: u64,
        total_count: u64,
    },
    Failed {
        request_id: u64,
        #[serde(rename = "failureKind")]
        kind: TrustOperationFailureKind,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SecureBackupSetupState {
    #[default]
    Idle,
    SettingUp {
        request_id: u64,
    },
    RecoveryKeyReady {
        request_id: u64,
        delivery: RecoveryKeyDeliveryState,
    },
    Enabled {
        request_id: u64,
    },
    Failed {
        request_id: u64,
        #[serde(rename = "failureKind")]
        kind: TrustOperationFailureKind,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RecoveryKeyDeliveryState {
    #[default]
    NotWritten,
    Written,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SecureBackupPassphraseChangeState {
    #[default]
    Idle,
    Changing {
        request_id: u64,
    },
    Changed {
        request_id: u64,
        delivery: RecoveryKeyDeliveryState,
    },
    Failed {
        request_id: u64,
        #[serde(rename = "failureKind")]
        kind: TrustOperationFailureKind,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum VerificationFlowState {
    #[default]
    Idle,
    Requested {
        request_id: u64,
        target: VerificationTarget,
    },
    Accepted {
        request_id: u64,
        target: VerificationTarget,
    },
    SasPresented {
        request_id: u64,
        target: VerificationTarget,
        emojis: Vec<SasEmoji>,
    },
    Confirming {
        request_id: u64,
        target: VerificationTarget,
        emojis: Vec<SasEmoji>,
    },
    Done {
        request_id: u64,
        target: VerificationTarget,
    },
    Failed {
        request_id: u64,
        target: VerificationTarget,
        #[serde(rename = "failureKind")]
        kind: TrustOperationFailureKind,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VerificationTarget {
    pub user_id: String,
    pub device_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SasEmoji {
    pub symbol: String,
    pub description: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CrossSigningStatus {
    #[default]
    Unknown,
    Missing,
    Bootstrapping {
        request_id: u64,
    },
    Trusted,
    NotTrusted,
    Failed {
        request_id: u64,
        #[serde(rename = "failureKind")]
        kind: TrustOperationFailureKind,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum KeyBackupStatus {
    #[default]
    Unknown,
    Disabled,
    Enabling {
        request_id: u64,
    },
    Enabled {
        version: String,
    },
    Restoring {
        request_id: u64,
        version: Option<String>,
        restored_rooms: u64,
        total_rooms: Option<u64>,
    },
    Failed {
        request_id: u64,
        #[serde(rename = "failureKind")]
        kind: TrustOperationFailureKind,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum IdentityResetState {
    #[default]
    Idle,
    Resetting {
        request_id: u64,
    },
    AwaitingAuth {
        request_id: u64,
        auth_type: IdentityResetAuthType,
    },
    Failed {
        request_id: u64,
        #[serde(rename = "failureKind")]
        kind: TrustOperationFailureKind,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IdentityResetAuthType {
    Uiaa,
    #[serde(rename = "oauth")]
    OAuth,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeviceTrustSummary {
    pub user_id: String,
    pub device_id: String,
    pub trust_level: DeviceTrustLevel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceTrustLevel {
    Unknown,
    Unverified,
    Verified,
    Blocked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrustOperationFailureKind {
    Cancelled,
    Mismatch,
    InvalidPassphrase,
    Network,
    Forbidden,
    Timeout,
    Sdk,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VerificationCancelReason {
    User,
    Mismatch,
}
