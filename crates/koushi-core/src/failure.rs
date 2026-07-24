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
    SearchFailed {
        kind: SearchFailureKind,
    },
    ReportOperationFailed {
        kind: ReportFailureKind,
    },
    LocalEncryptionUnavailable,
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
    Forbidden,
    NotFound,
    Network,
    Sdk,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TimelineFailureKind {
    InvalidDirection,
    InvalidReactionTarget,
    InvalidReactionState,
    InvalidSendTarget,
    InvalidSendState,
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
