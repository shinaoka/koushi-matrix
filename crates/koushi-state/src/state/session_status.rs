use std::fmt;

use serde::{Deserialize, Serialize};

use super::{CurrentDeviceTrustState, SessionAuthenticationMethod};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatusRefreshTrigger {
    Open,
    Manual,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurrentSessionSyncState {
    Stopped,
    Starting,
    Running,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnIdentityVerification {
    Missing,
    Unverified,
    Verified,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurrentSessionBackupState {
    Ready,
    Disabled,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurrentSessionStatusFailureKind {
    Sdk,
    TimedOut,
    Unavailable,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct CurrentSessionStatusDetails {
    pub device_display_name: Option<String>,
    pub device_id: String,
    pub authentication_method: SessionAuthenticationMethod,
    pub sync_state: CurrentSessionSyncState,
    pub is_cross_signed_by_owner: bool,
    pub own_identity_verification: OwnIdentityVerification,
    pub key_backup: CurrentSessionBackupState,
    pub verification: CurrentDeviceTrustState,
    pub checked_at_ms: u64,
}

impl CurrentSessionStatusDetails {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device_display_name: Option<String>,
        device_id: String,
        authentication_method: SessionAuthenticationMethod,
        sync_state: CurrentSessionSyncState,
        verification: CurrentDeviceTrustState,
        is_cross_signed_by_owner: bool,
        own_identity_verification: OwnIdentityVerification,
        key_backup: CurrentSessionBackupState,
        checked_at_ms: u64,
    ) -> Self {
        Self {
            device_display_name,
            device_id,
            authentication_method,
            sync_state,
            is_cross_signed_by_owner,
            own_identity_verification,
            key_backup,
            verification,
            checked_at_ms,
        }
    }
}

impl fmt::Debug for CurrentSessionStatusDetails {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CurrentSessionStatusDetails")
            .field(
                "device_display_name",
                &self.device_display_name.as_ref().map(|_| "DeviceName(..)"),
            )
            .field("device_id", &"DeviceId(..)")
            .field("authentication_method", &self.authentication_method)
            .field("sync_state", &self.sync_state)
            .field("is_cross_signed_by_owner", &self.is_cross_signed_by_owner)
            .field("own_identity_verification", &self.own_identity_verification)
            .field("key_backup", &self.key_backup)
            .field("verification", &self.verification)
            .field("checked_at_ms", &self.checked_at_ms)
            .finish()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CurrentSessionStatusState {
    #[default]
    Idle,
    Checking {
        request_id: u64,
        trigger: SessionStatusRefreshTrigger,
    },
    Ready {
        request_id: u64,
        details: CurrentSessionStatusDetails,
    },
    Failed {
        request_id: u64,
        kind: CurrentSessionStatusFailureKind,
        checked_at_ms: u64,
    },
}
