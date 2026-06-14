//! Public event boundary. Events carry the originating `RequestId` when one
//! exists; identifiers and visible bodies are allowed, secrets never.

use std::fmt;

use matrix_desktop_state::{CrossSigningStatus, KeyBackupStatus, VerificationFlowState};
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
    E2eeTrust(E2eeTrustEvent),
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
        request_id: Option<RequestId>,
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
            Self::IdentityResetChanged { request_id, .. } => formatter
                .debug_struct("IdentityResetChanged")
                .field("account_key", &"AccountKey(..)")
                .field("request_id", request_id)
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
    pub in_reply_to_event_id: Option<String>,
    #[serde(default)]
    pub thread_root: Option<String>,
    #[serde(default)]
    pub thread_summary: Option<ThreadSummaryDto>,
    #[serde(default)]
    pub reactions: Vec<ReactionGroup>,
    #[serde(default)]
    pub can_react: bool,
    #[serde(default)]
    pub is_redacted: bool,
    #[serde(default)]
    pub can_redact: bool,
    #[serde(default)]
    pub is_edited: bool,
    #[serde(default)]
    pub can_edit: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ThreadSummaryDto {
    pub reply_count: u32,
    pub latest_sender: Option<String>,
    pub latest_body_preview: Option<String>,
    pub latest_timestamp_ms: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReactionGroup {
    pub key: String,
    pub count: u32,
    pub reacted_by_me: bool,
    pub my_reaction_event_id: Option<String>,
    pub sender_preview: Vec<String>,
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
    /// The encrypted search index applied a document mutation for this event.
    /// Carries only app-owned visible-state identifiers (room/event ids) so
    /// pollers can wake on indexing progress instead of sleeping; the message
    /// body is never included (Security Model — Search).
    IndexUpdated { room_id: String, event_id: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchResultItem {
    pub room_id: String,
    pub event_id: String,
    pub snippet: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn timeline_item_serializes_thread_fields_reactions_and_redaction_affordances() {
        let item = TimelineItem {
            id: TimelineItemId::Event {
                event_id: "$event:test".to_owned(),
            },
            sender: Some("@alice:example.invalid".to_owned()),
            body: Some("hello".to_owned()),
            timestamp_ms: Some(1_234),
            in_reply_to_event_id: None,
            thread_root: Some("$root:test".to_owned()),
            thread_summary: Some(ThreadSummaryDto {
                reply_count: 2,
                latest_sender: Some("@bob:example.invalid".to_owned()),
                latest_body_preview: Some("latest reply".to_owned()),
                latest_timestamp_ms: Some(1_456),
            }),
            reactions: vec![ReactionGroup {
                key: "👍".to_owned(),
                count: 2,
                reacted_by_me: true,
                my_reaction_event_id: Some("$reaction:test".to_owned()),
                sender_preview: vec!["@alice:example.invalid".to_owned()],
            }],
            can_react: true,
            is_redacted: false,
            can_redact: true,
            is_edited: true,
            can_edit: true,
        };

        let value = serde_json::to_value(&item).expect("timeline item serializes");

        assert_eq!(
            value["reactions"],
            json!([
                {
                    "key": "👍",
                    "count": 2,
                    "reacted_by_me": true,
                    "my_reaction_event_id": "$reaction:test",
                    "sender_preview": ["@alice:example.invalid"]
                }
            ])
        );
        assert_eq!(value["can_react"], json!(true));
        assert_eq!(value["is_redacted"], json!(false));
        assert_eq!(value["can_redact"], json!(true));
        assert_eq!(value["is_edited"], json!(true));
        assert_eq!(value["can_edit"], json!(true));
        assert_eq!(value["thread_root"], json!("$root:test"));
        assert_eq!(
            value["thread_summary"],
            json!({
                "reply_count": 2,
                "latest_sender": "@bob:example.invalid",
                "latest_body_preview": "latest reply",
                "latest_timestamp_ms": 1456
            })
        );
    }
}
