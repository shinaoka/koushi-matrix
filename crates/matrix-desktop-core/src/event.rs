//! Public event boundary. Events carry the originating `RequestId` when one
//! exists; identifiers and visible bodies are allowed, secrets never.

use std::fmt;

use matrix_desktop_state::{
    CrossSigningStatus, IdentityResetState, KeyBackupStatus, LiveRoomSignalUpdate, PresenceKind,
    VerificationFlowState,
};
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
    LiveSignals(LiveSignalsEvent),
    Search(SearchEvent),
    E2eeTrust(E2eeTrustEvent),
    OperationFailed {
        request_id: RequestId,
        failure: CoreFailure,
    },
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LiveSignalsEvent {
    RoomSignalsUpdated {
        room_id: String,
        update: LiveRoomSignalUpdate,
    },
    PresenceUpdated {
        user_id: String,
        presence: PresenceKind,
    },
    ReadReceiptSent {
        request_id: RequestId,
        key: TimelineKey,
        event_id: String,
    },
    FullyReadSet {
        request_id: RequestId,
        key: TimelineKey,
        event_id: String,
    },
    TypingSet {
        request_id: RequestId,
        key: TimelineKey,
        is_typing: bool,
    },
    PresenceSet {
        request_id: RequestId,
        presence: PresenceKind,
    },
}

impl fmt::Debug for LiveSignalsEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RoomSignalsUpdated { update, .. } => formatter
                .debug_struct("RoomSignalsUpdated")
                .field("room_id", &"RoomId(..)")
                .field("receipt_events", &update.receipts_by_event.len())
                .field(
                    "fully_read_event_id",
                    &update.fully_read_event_id.as_ref().map(|_| "EventId(..)"),
                )
                .field("typing_users", &update.typing_user_ids.len())
                .finish(),
            Self::PresenceUpdated { presence, .. } => formatter
                .debug_struct("PresenceUpdated")
                .field("user_id", &"UserId(..)")
                .field("presence", presence)
                .finish(),
            Self::ReadReceiptSent { request_id, .. } => formatter
                .debug_struct("ReadReceiptSent")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("event_id", &"EventId(..)")
                .finish(),
            Self::FullyReadSet { request_id, .. } => formatter
                .debug_struct("FullyReadSet")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("event_id", &"EventId(..)")
                .finish(),
            Self::TypingSet {
                request_id,
                is_typing,
                ..
            } => formatter
                .debug_struct("TypingSet")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("is_typing", is_typing)
                .finish(),
            Self::PresenceSet {
                request_id,
                presence,
            } => formatter
                .debug_struct("PresenceSet")
                .field("request_id", request_id)
                .field("presence", presence)
                .finish(),
        }
    }
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
    ProfileUpdated {
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
    InviteAccepted {
        request_id: RequestId,
        room_id: String,
    },
    InviteDeclined {
        request_id: RequestId,
        room_id: String,
    },
    DirectMessageStarted {
        request_id: RequestId,
        room_id: String,
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

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
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
    MediaUploadProgress {
        request_id: Option<RequestId>,
        key: TimelineKey,
        transaction_id: String,
        index: u64,
        progress: MediaTransferProgress,
        source: Option<TimelineMediaSource>,
    },
    MediaDownloadCompleted {
        request_id: RequestId,
        key: TimelineKey,
        event_id: String,
        byte_count: u64,
        mimetype: Option<String>,
    },
    ResyncRequired {
        key: TimelineKey,
        reason: TimelineResyncReason,
    },
}

impl fmt::Debug for TimelineEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InitialItems {
                request_id,
                generation,
                items,
                ..
            } => formatter
                .debug_struct("InitialItems")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("generation", generation)
                .field("items", items)
                .finish(),
            Self::ItemsUpdated {
                generation,
                batch_id,
                diffs,
                ..
            } => formatter
                .debug_struct("ItemsUpdated")
                .field("key", &"TimelineKey(..)")
                .field("generation", generation)
                .field("batch_id", batch_id)
                .field("diffs", diffs)
                .finish(),
            Self::PaginationStateChanged {
                request_id,
                direction,
                state,
                ..
            } => formatter
                .debug_struct("PaginationStateChanged")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("direction", direction)
                .field("state", state)
                .finish(),
            Self::SendCompleted {
                request_id,
                transaction_id,
                ..
            } => formatter
                .debug_struct("SendCompleted")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("transaction_id", transaction_id)
                .field("event_id", &"EventId(..)")
                .finish(),
            Self::MediaUploadProgress {
                request_id,
                transaction_id,
                index,
                progress,
                source,
                ..
            } => formatter
                .debug_struct("MediaUploadProgress")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("transaction_id", transaction_id)
                .field("index", index)
                .field("progress", progress)
                .field("source", source)
                .finish(),
            Self::MediaDownloadCompleted {
                request_id,
                byte_count,
                mimetype,
                ..
            } => formatter
                .debug_struct("MediaDownloadCompleted")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("event_id", &"EventId(..)")
                .field("byte_count", byte_count)
                .field("mimetype", mimetype)
                .finish(),
            Self::ResyncRequired { reason, .. } => formatter
                .debug_struct("ResyncRequired")
                .field("key", &"TimelineKey(..)")
                .field("reason", reason)
                .finish(),
        }
    }
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
#[derive(Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum TimelineItemId {
    Event { event_id: String },
    Transaction { transaction_id: String },
    Synthetic { synthetic_id: String },
}

impl fmt::Debug for TimelineItemId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Event { .. } => formatter
                .debug_struct("Event")
                .field("event_id", &"EventId(..)")
                .finish(),
            Self::Transaction { .. } => formatter
                .debug_struct("Transaction")
                .field("transaction_id", &"TransactionId(..)")
                .finish(),
            Self::Synthetic { .. } => formatter
                .debug_struct("Synthetic")
                .field("synthetic_id", &"SyntheticId(..)")
                .finish(),
        }
    }
}

/// Timeline item DTO. Phase 5 concretizes content kinds from the SDK
/// projection; the identity contract is stable from Phase 1.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<TimelineMedia>,
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

impl fmt::Debug for TimelineItem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TimelineItem")
            .field("id", &self.id)
            .field("sender", &self.sender)
            .field("body", &self.body.as_ref().map(|_| "MessageBody(..)"))
            .field("timestamp_ms", &self.timestamp_ms)
            .field("in_reply_to_event_id", &self.in_reply_to_event_id)
            .field("thread_root", &self.thread_root)
            .field(
                "thread_summary",
                &self.thread_summary.as_ref().map(|_| "ThreadSummary(..)"),
            )
            .field("media", &self.media)
            .field("reactions", &self.reactions)
            .field("can_react", &self.can_react)
            .field("is_redacted", &self.is_redacted)
            .field("can_redact", &self.can_redact)
            .field("is_edited", &self.is_edited)
            .field("can_edit", &self.can_edit)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineMedia {
    pub kind: TimelineMediaKind,
    pub filename: String,
    pub source: TimelineMediaSource,
    pub mimetype: Option<String>,
    pub size: Option<u64>,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub thumbnail: Option<TimelineMediaThumbnail>,
}

impl fmt::Debug for TimelineMedia {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TimelineMedia")
            .field("kind", &self.kind)
            .field("filename", &"MediaFilename(..)")
            .field("source", &self.source)
            .field("mimetype", &self.mimetype)
            .field("size", &self.size)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("thumbnail", &self.thumbnail)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TimelineMediaKind {
    Image,
    File,
    Audio,
    Video,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineMediaSource {
    pub mxc_uri: String,
    pub encrypted: bool,
    pub encryption_version: Option<String>,
}

impl fmt::Debug for TimelineMediaSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TimelineMediaSource")
            .field("mxc_uri", &"MxcUri(..)")
            .field("encrypted", &self.encrypted)
            .field("encryption_version", &self.encryption_version)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineMediaThumbnail {
    pub source: TimelineMediaSource,
    pub mimetype: Option<String>,
    pub size: Option<u64>,
    pub width: Option<u64>,
    pub height: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MediaTransferProgress {
    pub current: u64,
    pub total: u64,
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
            media: None,
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

    #[test]
    fn timeline_item_serializes_media_metadata_without_encryption_secrets() {
        let item = TimelineItem {
            id: TimelineItemId::Event {
                event_id: "$media:test".to_owned(),
            },
            sender: Some("@alice:example.invalid".to_owned()),
            body: Some("synthetic caption".to_owned()),
            timestamp_ms: Some(1_234),
            in_reply_to_event_id: None,
            thread_root: None,
            thread_summary: None,
            media: Some(TimelineMedia {
                kind: TimelineMediaKind::Image,
                filename: "synthetic-image.png".to_owned(),
                source: TimelineMediaSource {
                    mxc_uri: "mxc://example.invalid/media".to_owned(),
                    encrypted: true,
                    encryption_version: Some("v2".to_owned()),
                },
                mimetype: Some("image/png".to_owned()),
                size: Some(68),
                width: Some(2),
                height: Some(2),
                thumbnail: Some(TimelineMediaThumbnail {
                    source: TimelineMediaSource {
                        mxc_uri: "mxc://example.invalid/thumb".to_owned(),
                        encrypted: true,
                        encryption_version: Some("v2".to_owned()),
                    },
                    mimetype: Some("image/png".to_owned()),
                    size: Some(32),
                    width: Some(1),
                    height: Some(1),
                }),
            }),
            reactions: Vec::new(),
            can_react: true,
            is_redacted: false,
            can_redact: true,
            is_edited: false,
            can_edit: false,
        };

        let value = serde_json::to_value(&item).expect("timeline item serializes");

        assert_eq!(
            value["media"],
            json!({
                "kind": "Image",
                "filename": "synthetic-image.png",
                "source": {
                    "mxc_uri": "mxc://example.invalid/media",
                    "encrypted": true,
                    "encryption_version": "v2"
                },
                "mimetype": "image/png",
                "size": 68,
                "width": 2,
                "height": 2,
                "thumbnail": {
                    "source": {
                        "mxc_uri": "mxc://example.invalid/thumb",
                        "encrypted": true,
                        "encryption_version": "v2"
                    },
                    "mimetype": "image/png",
                    "size": 32,
                    "width": 1,
                    "height": 1
                }
            })
        );
        let serialized = serde_json::to_string(&item).expect("timeline item json");
        assert!(!serialized.contains("key"));
        assert!(!serialized.contains("hashes"));

        let debug = format!("{item:?}");
        assert!(!debug.contains("synthetic caption"), "{debug}");
        assert!(!debug.contains("synthetic-image.png"), "{debug}");
        assert!(!debug.contains("mxc://example.invalid"), "{debug}");
        assert!(!debug.contains("$media:test"), "{debug}");
    }

    #[test]
    fn media_timeline_event_debug_redacts_routing_and_media_identifiers() {
        let key = TimelineKey::room(
            AccountKey("@alice:example.invalid".to_owned()),
            "!room:example.invalid",
        );
        let event = TimelineEvent::MediaUploadProgress {
            request_id: Some(RequestId {
                connection_id: crate::ids::RuntimeConnectionId(1),
                sequence: 7,
            }),
            key,
            transaction_id: "txn-media".to_owned(),
            index: 0,
            progress: MediaTransferProgress {
                current: 4,
                total: 8,
            },
            source: Some(TimelineMediaSource {
                mxc_uri: "mxc://example.invalid/media".to_owned(),
                encrypted: true,
                encryption_version: Some("v2".to_owned()),
            }),
        };

        let debug = format!("{event:?}");
        assert!(debug.contains("MediaUploadProgress"), "{debug}");
        assert!(debug.contains("txn-media"), "{debug}");
        assert!(!debug.contains("!room:example.invalid"), "{debug}");
        assert!(!debug.contains("@alice:example.invalid"), "{debug}");
        assert!(!debug.contains("mxc://example.invalid"), "{debug}");
    }
}
