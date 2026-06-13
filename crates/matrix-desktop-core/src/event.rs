//! Public event boundary. Events carry the originating `RequestId` when one
//! exists; identifiers and visible bodies are allowed, secrets never.

use serde::{Deserialize, Serialize};

use crate::failure::{CoreFailure, TimelineFailureKind};
use crate::ids::{AccountKey, RequestId, TimelineBatchId, TimelineGeneration, TimelineKey};

/// Serializable UI snapshot. The full timeline item lists never live here
/// (Async rule 4); timeline data flows as diffs.
pub type AppStateSnapshot = matrix_desktop_state::AppState;

#[derive(Clone, Debug)]
pub enum CoreEvent {
    StateChanged(AppStateSnapshot),
    Account(AccountEvent),
    Sync(SyncEvent),
    Room(RoomEvent),
    Timeline(TimelineEvent),
    Search(SearchEvent),
    OperationFailed {
        request_id: RequestId,
        failure: CoreFailure,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AccountEvent {
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
        sessions: Vec<matrix_desktop_state::SessionInfo>,
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
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SyncEvent {
    Started {
        request_id: Option<RequestId>,
        backend: SyncBackendKind,
    },
    Running,
    Reconnecting,
    Failed,
    Stopped {
        request_id: Option<RequestId>,
    },
}

/// Selected sync backend, emitted so QA can assert server capability
/// (Async rule 9).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SyncBackendKind {
    SyncService,
    LegacySync,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RoomEvent {
    RoomCreated {
        request_id: RequestId,
        room_id: String,
    },
    SpaceCreated {
        request_id: RequestId,
        space_id: String,
    },
    SpaceChildSet {
        request_id: RequestId,
        space_id: String,
        child_room_id: String,
    },
    UserInvited {
        request_id: RequestId,
        room_id: String,
        user_id: String,
    },
    RoomJoined {
        request_id: RequestId,
        room_id: String,
    },
    RoomLeft {
        request_id: RequestId,
        room_id: String,
    },
    RoomForgotten {
        request_id: RequestId,
        room_id: String,
    },
    RoomListUpdated,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TimelineEvent {
    InitialItems {
        request_id: Option<RequestId>,
        key: TimelineKey,
        generation: TimelineGeneration,
        items: Vec<TimelineItem>,
    },
    ItemsUpdated {
        key: TimelineKey,
        generation: TimelineGeneration,
        batch_id: TimelineBatchId,
        diffs: Vec<TimelineDiff>,
    },
    PaginationStateChanged {
        request_id: Option<RequestId>,
        key: TimelineKey,
        direction: PaginationDirection,
        state: PaginationState,
    },
    SendCompleted {
        request_id: RequestId,
        key: TimelineKey,
        transaction_id: String,
        event_id: String,
    },
    ResyncRequired {
        key: TimelineKey,
        reason: TimelineResyncReason,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum PaginationDirection {
    Backward,
    Forward,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum PaginationState {
    Idle,
    Paginating,
    EndReached,
    Failed { kind: TimelineFailureKind },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TimelineResyncReason {
    QueueOverflow,
    SubscriptionRestarted,
}

/// Stable identity for every renderable item (Viewport/Scrollback contract):
/// remote event id when known, transaction id for local echo, synthetic ids
/// for separators/virtual items.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum TimelineItemId {
    Event { event_id: String },
    Transaction { transaction_id: String },
    Synthetic { synthetic_id: String },
}

/// Timeline item DTO. Phase 5 concretizes content kinds from the SDK
/// projection; the identity contract is stable from Phase 1.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineItem {
    pub id: TimelineItemId,
    pub sender: Option<String>,
    pub body: Option<String>,
    pub timestamp_ms: Option<u64>,
}

/// `VectorDiff`-shaped update preserving positional operations so the UI can
/// distinguish prepend pagination from live append/update/remove.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TimelineDiff {
    PushFront { item: TimelineItem },
    PushBack { item: TimelineItem },
    Insert { index: usize, item: TimelineItem },
    Set { index: usize, item: TimelineItem },
    Remove { index: usize },
    Truncate { length: usize },
    Clear,
    Reset { items: Vec<TimelineItem> },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SearchEvent {
    Results {
        request_id: RequestId,
        results: Vec<SearchResultItem>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchResultItem {
    pub room_id: String,
    pub event_id: String,
    pub snippet: String,
}
