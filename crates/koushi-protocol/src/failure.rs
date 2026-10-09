//! Redacted public failures (overview.md Security Model: coarse public
//! failures with non-secret kinds; never raw SDK errors).

use serde::{Deserialize, Serialize};

use koushi_state::AuthFailureKind;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CoreFailure {
    SessionRequired,
    /// The credential store is healthy but holds no stored session for the
    /// requested account (restore / switch target). UI: go to login quietly.
    SessionNotFound,
    LoginFailed {
        kind: LoginFailureKind,
    },
    RecoveryFailed {
        kind: RecoveryFailureKind,
    },
    SyncFailed {
        kind: SyncFailureKind,
    },
    RoomOperationFailed {
        kind: RoomFailureKind,
    },
    TimelineOperationFailed {
        kind: TimelineFailureKind,
    },
    ProfileOperationFailed {
        kind: ProfileFailureKind,
    },
    AccountOperationFailed {
        kind: AuthFailureKind,
    },
    SecureBackupSetupConfirmationRequired,
    SecureBackupSetupFailedNoOp,
    /// A verification request was refused because another verification flow
    /// is still in progress (#1024).
    VerificationInProgress,
    SearchFailed {
        kind: SearchFailureKind,
    },
    ReportOperationFailed {
        kind: ReportFailureKind,
    },
    LocalEncryptionUnavailable,
    PreferenceRejected,
    StoreUnavailable,
    ShutdownFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum LoginFailureKind {
    InvalidCredentials,
    Network,
    RateLimited,
    Server,
    Store,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RecoveryFailureKind {
    InvalidRecoveryKey,
    Network,
    Server,
    Timeout,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SyncFailureKind {
    Http,
    Auth,
    Store,
    Internal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RoomFailureKind {
    AliasInUse,
    Forbidden,
    InvalidInvite,
    NotFound,
    Network,
    Sdk,
    /// The current join-rule content has allow conditions this client does not
    /// model, so rewriting it would drop them (#1177).
    UnsupportedPolicyCondition,
    /// The current join-rule policy could not be read from the store before
    /// the write, so the edit was not attempted (#1177).
    PolicyNotVerified,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadStateFailureKind {
    Timeout,
    Transport,
    RateLimited,
    Authentication,
    Server,
    Capacity,
    Sdk,
}

impl ReadStateFailureKind {
    pub fn token(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Transport => "transport",
            Self::RateLimited => "rate_limited",
            Self::Authentication => "authentication",
            Self::Server => "server",
            Self::Capacity => "capacity",
            Self::Sdk => "sdk",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TimelineFailureKind {
    InvalidDirection,
    InvalidReactionTarget,
    InvalidReactionState,
    InvalidSendTarget,
    InvalidSendState,
    SecureBackupRequired,
    ComposerRevisionExhausted,
    UnsupportedSlashCommand,
    NotSubscribed,
    Forbidden,
    Network,
    Timeout,
    Sdk,
    QueueOverflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ProfileFailureKind {
    Forbidden,
    Network,
    InvalidMimeType,
    Sdk,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SearchFailureKind {
    IndexUnavailable,
    Query,
    Internal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ReportFailureKind {
    Forbidden,
    Network,
    InvalidUserId,
    InvalidRoomId,
    InvalidEventId,
    Sdk,
}
