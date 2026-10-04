use std::fmt;

use koushi_state::{
    AvatarThumbnailState, CrossSigningStatus, IdentityResetState, KeyBackupStatus,
    LocalEncryptionHealth, VerificationFlowState,
};
use serde::{Deserialize, Serialize};

use super::ReportKind;
use crate::ids::{AccountKey, RequestId};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LocalEncryptionEvent {
    HealthChanged {
        health: LocalEncryptionHealth,
    },
    EventCacheStatus {
        encrypted_store: bool,
        subscribed: bool,
        subscribe_status: EventCacheSubscribeStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason_class: Option<EventCacheFailureReasonClass>,
    },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventCacheSubscribeStatus {
    Enabled,
    AlreadyEnabled,
    SubscribeFailed,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventCacheFailureReasonClass {
    SubscribeFailed,
}
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum AccountEvent {
    OidcAuthorizationCreated {
        request_id: RequestId,
        authorization_url: String,
        state: String,
    },
    AuthDiscoveryChanged {
        request_id: RequestId,
        homeserver: String,
    },
    /// A new login's authenticated session was admitted (provisional session
    /// installed). Emitted immediately, unlike `LoggedIn`, which is held until
    /// trust promotion; it settles the login request outcome.
    LoginAdmitted {
        request_id: RequestId,
        account_key: AccountKey,
    },
    LoggedIn {
        request_id: RequestId,
        account_key: AccountKey,
    },
    SessionRestored {
        request_id: RequestId,
        account_key: AccountKey,
    },
    /// Answer to `AccountCommand::QuerySavedSessions`. Carries identity data
    /// only (homeserver / user_id / device_id) — never tokens or secrets.
    SavedSessionsListed {
        request_id: RequestId,
        sessions: Vec<koushi_state::SessionInfo>,
    },
    RecoveryRequired {
        account_key: AccountKey,
    },
    RecoveryCompleted {
        request_id: RequestId,
        account_key: AccountKey,
    },
    LoggedOut {
        request_id: RequestId,
        account_key: AccountKey,
    },
    AccountSwitched {
        request_id: RequestId,
        account_key: AccountKey,
    },
    ProfileUpdated {
        request_id: RequestId,
        account_key: AccountKey,
    },
    AvatarThumbnailDownloaded {
        request_id: RequestId,
        mxc_uri: String,
        thumbnail: AvatarThumbnailState,
    },
    ReportCompleted {
        request_id: RequestId,
        kind: ReportKind,
    },
}
impl fmt::Debug for AccountEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OidcAuthorizationCreated { request_id, .. } => formatter
                .debug_struct("OidcAuthorizationCreated")
                .field("request_id", request_id)
                .field("authorization_url", &"AuthorizationUrl(..)")
                .field("state", &"CsrfState(..)")
                .finish(),
            Self::AuthDiscoveryChanged {
                request_id,
                homeserver: _,
            } => formatter
                .debug_struct("AuthDiscoveryChanged")
                .field("request_id", request_id)
                .field("homeserver", &"Homeserver(..)")
                .finish(),
            Self::LoginAdmitted {
                request_id,
                account_key,
            } => formatter
                .debug_struct("LoginAdmitted")
                .field("request_id", request_id)
                .field("account_key", account_key)
                .finish(),
            Self::LoggedIn {
                request_id,
                account_key,
            } => formatter
                .debug_struct("LoggedIn")
                .field("request_id", request_id)
                .field("account_key", account_key)
                .finish(),
            Self::SessionRestored {
                request_id,
                account_key,
            } => formatter
                .debug_struct("SessionRestored")
                .field("request_id", request_id)
                .field("account_key", account_key)
                .finish(),
            Self::SavedSessionsListed {
                request_id,
                sessions,
            } => formatter
                .debug_struct("SavedSessionsListed")
                .field("request_id", request_id)
                .field("session_count", &sessions.len())
                .finish(),
            Self::RecoveryRequired { account_key } => formatter
                .debug_struct("RecoveryRequired")
                .field("account_key", account_key)
                .finish(),
            Self::RecoveryCompleted {
                request_id,
                account_key,
            } => formatter
                .debug_struct("RecoveryCompleted")
                .field("request_id", request_id)
                .field("account_key", account_key)
                .finish(),
            Self::LoggedOut {
                request_id,
                account_key,
            } => formatter
                .debug_struct("LoggedOut")
                .field("request_id", request_id)
                .field("account_key", account_key)
                .finish(),
            Self::AccountSwitched {
                request_id,
                account_key,
            } => formatter
                .debug_struct("AccountSwitched")
                .field("request_id", request_id)
                .field("account_key", account_key)
                .finish(),
            Self::ProfileUpdated {
                request_id,
                account_key,
            } => formatter
                .debug_struct("ProfileUpdated")
                .field("request_id", request_id)
                .field("account_key", account_key)
                .finish(),
            Self::AvatarThumbnailDownloaded {
                request_id,
                mxc_uri: _,
                thumbnail,
            } => formatter
                .debug_struct("AvatarThumbnailDownloaded")
                .field("request_id", request_id)
                .field("mxc_uri", &"MxcUri(..)")
                .field("thumbnail", thumbnail)
                .finish(),
            Self::ReportCompleted { request_id, kind } => formatter
                .debug_struct("ReportCompleted")
                .field("request_id", request_id)
                .field("kind", kind)
                .finish(),
        }
    }
}
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum E2eeTrustEvent {
    VerificationProgress {
        account_key: AccountKey,
        state: VerificationFlowState,
    },
    CrossSigningChanged {
        account_key: AccountKey,
        status: CrossSigningStatus,
    },
    KeyBackupChanged {
        account_key: AccountKey,
        status: KeyBackupStatus,
    },
    IdentityResetChanged {
        account_key: AccountKey,
        state: IdentityResetState,
    },
}
impl fmt::Debug for E2eeTrustEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VerificationProgress { state, .. } => formatter
                .debug_struct("VerificationProgress")
                .field("account_key", &"AccountKey(..)")
                .field("state", &verification_state_name(state))
                .finish(),
            Self::CrossSigningChanged { status, .. } => formatter
                .debug_struct("CrossSigningChanged")
                .field("account_key", &"AccountKey(..)")
                .field("status", &cross_signing_status_name(status))
                .finish(),
            Self::KeyBackupChanged { status, .. } => formatter
                .debug_struct("KeyBackupChanged")
                .field("account_key", &"AccountKey(..)")
                .field("status", &key_backup_status_name(status))
                .finish(),
            Self::IdentityResetChanged { state, .. } => formatter
                .debug_struct("IdentityResetChanged")
                .field("account_key", &"AccountKey(..)")
                .field("state", &identity_reset_state_name(state))
                .finish(),
        }
    }
}
fn verification_state_name(state: &VerificationFlowState) -> &'static str {
    match state {
        VerificationFlowState::Idle => "Idle",
        VerificationFlowState::Requested { .. } => "Requested",
        VerificationFlowState::Accepted { .. } => "Accepted",
        VerificationFlowState::SasPresented { .. } => "SasPresented",
        VerificationFlowState::Confirming { .. } => "Confirming",
        VerificationFlowState::Done { .. } => "Done",
        VerificationFlowState::Failed { .. } => "Failed",
    }
}
fn cross_signing_status_name(status: &CrossSigningStatus) -> &'static str {
    match status {
        CrossSigningStatus::Unknown => "Unknown",
        CrossSigningStatus::Missing => "Missing",
        CrossSigningStatus::Bootstrapping { .. } => "Bootstrapping",
        CrossSigningStatus::Trusted => "Trusted",
        CrossSigningStatus::NotTrusted => "NotTrusted",
        CrossSigningStatus::Failed { .. } => "Failed",
    }
}
fn key_backup_status_name(status: &KeyBackupStatus) -> &'static str {
    match status {
        KeyBackupStatus::Unknown => "Unknown",
        KeyBackupStatus::Disabled => "Disabled",
        KeyBackupStatus::Enabling { .. } => "Enabling",
        KeyBackupStatus::Enabled { .. } => "Enabled",
        KeyBackupStatus::Restoring { .. } => "Restoring",
        KeyBackupStatus::Failed { .. } => "Failed",
    }
}
fn identity_reset_state_name(state: &IdentityResetState) -> &'static str {
    match state {
        IdentityResetState::Idle => "Idle",
        IdentityResetState::Resetting { .. } => "Resetting",
        IdentityResetState::AwaitingAuth { .. } => "AwaitingAuth",
        IdentityResetState::Failed { .. } => "Failed",
    }
}
