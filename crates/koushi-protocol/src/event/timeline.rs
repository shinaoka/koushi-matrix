use std::fmt;

use koushi_state::{
    AvatarImage, AvatarThumbnailState, ComposerDocument, JapaneseCatalogProfile,
    MediaTransferProgress, OperationFailureKind, ReplyQuote, SubmissionId, ThreadsListItem,
};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::failure::{ReadStateFailureKind, TimelineFailureKind};
use crate::ids::{RequestId, TimelineBatchId, TimelineGeneration, TimelineKey};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CjkTextPolicyEvent {
    JapaneseCatalogProfileChanged { profile: JapaneseCatalogProfile },
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ThreadsListEvent {
    Opened {
        request_id: RequestId,
        room_id: String,
        items: Vec<ThreadsListItem>,
        end_reached: bool,
    },
    Updated {
        request_id: RequestId,
        room_id: String,
        items: Vec<ThreadsListItem>,
        is_paginating: bool,
        end_reached: bool,
    },
    PaginationCompleted {
        request_id: RequestId,
        room_id: String,
        items: Vec<ThreadsListItem>,
        end_reached: bool,
    },
    Failed {
        request_id: RequestId,
        room_id: String,
        failure_kind: OperationFailureKind,
    },
}
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum TimelineEvent {
    InitialItems {
        /// Stable projection identity retained until the WebView acknowledges
        /// this actor generation's initial projection.
        request_id: Option<RequestId>,
        /// Exact command that caused this delivery. Recovery projections have
        /// no command cause and therefore use `None`.
        cause_request_id: Option<RequestId>,
        key: TimelineKey,
        /// Monotonic owner generation for actor replacement fencing.
        actor_generation: u64,
        generation: TimelineGeneration,
        items: Vec<TimelineItem>,
    },
    ItemsUpdated {
        key: TimelineKey,
        generation: TimelineGeneration,
        batch_id: TimelineBatchId,
        /// All numeric `TimelineDiff` indices are relative to the desktop
        /// display sequence immediately before that operation, never to
        /// Core's full navigation sequence.
        diffs: Vec<TimelineDiff>,
    },
    PaginationStateChanged {
        request_id: Option<RequestId>,
        key: TimelineKey,
        direction: PaginationDirection,
        state: PaginationState,
        /// Whether an accepted backward page changed the observable oldest edge.
        /// `None` is used for admission rejection, start, cancellation, and failure.
        prepend_expected: Option<bool>,
    },
    AnchorRestoreFinished {
        request_id: RequestId,
        key: TimelineKey,
        status: TimelineAnchorRestoreStatus,
    },
    NavigationUpdated {
        key: TimelineKey,
        snapshot: TimelineNavigationSnapshot,
    },
    GapPositionsUpdated {
        key: TimelineKey,
        /// Monotonic owner generation for actor replacement fencing.
        actor_generation: u64,
        generation: u64,
        positions: Vec<TimelineGapPosition>,
    },
    /// Gap work reached an idle scheduler after terminal processing. A UI
    /// pagination request rejected while repair was active may retry now.
    GapRepairReleased {
        key: TimelineKey,
        /// Monotonic owner generation for actor replacement fencing.
        actor_generation: u64,
        generation: u64,
    },
    SendCompleted {
        request_id: RequestId,
        key: TimelineKey,
        transaction_id: String,
        event_id: String,
    },
    MediaSendQueued {
        request_id: RequestId,
        key: TimelineKey,
        transaction_id: String,
    },
    SubmissionAccepted {
        request_id: RequestId,
        key: TimelineKey,
        submission_id: SubmissionId,
        transaction_id: String,
    },
    SubmissionRejected {
        request_id: RequestId,
        key: TimelineKey,
        submission_id: SubmissionId,
        kind: TimelineFailureKind,
    },
    MessageForwarded {
        request_id: RequestId,
        key: TimelineKey,
        destination_room_id: String,
        transaction_id: String,
        event_id: String,
    },
    MessageSourceLoaded {
        request_id: RequestId,
        key: TimelineKey,
        source: TimelineMessageSource,
    },
    MediaUploadProgress {
        request_id: Option<RequestId>,
        key: TimelineKey,
        transaction_id: String,
        index: u64,
        progress: MediaTransferProgress,
        source: Option<TimelineMediaSource>,
    },
    MediaDownloadProgress {
        request_id: RequestId,
        key: TimelineKey,
        event_id: String,
        progress: MediaTransferProgress,
    },
    MediaDownloadCompleted {
        request_id: RequestId,
        key: TimelineKey,
        event_id: String,
        source_url: String,
        byte_count: u64,
        mimetype: Option<String>,
        width: Option<u64>,
        height: Option<u64>,
    },
    MediaDownloadFailed {
        request_id: RequestId,
        key: TimelineKey,
        event_id: String,
        kind: TimelineFailureKind,
    },
    ResyncRequired {
        key: TimelineKey,
        reason: TimelineResyncReason,
    },
    DisplayPolicyUpdated {
        hide_redacted: bool,
    },
    DisplayLabelsUpdated {
        labels: Vec<TimelineDisplayLabelUpdate>,
    },
}
impl fmt::Debug for TimelineEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InitialItems {
                request_id,
                cause_request_id,
                generation,
                items,
                ..
            } => formatter
                .debug_struct("InitialItems")
                .field("request_id", request_id)
                .field("cause_request_id", cause_request_id)
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
            Self::AnchorRestoreFinished {
                request_id, status, ..
            } => formatter
                .debug_struct("AnchorRestoreFinished")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("status", status)
                .finish(),
            Self::NavigationUpdated { snapshot, .. } => formatter
                .debug_struct("NavigationUpdated")
                .field("key", &"TimelineKey(..)")
                .field("snapshot", snapshot)
                .finish(),
            Self::GapPositionsUpdated {
                actor_generation,
                generation,
                positions,
                ..
            } => formatter
                .debug_struct("GapPositionsUpdated")
                .field("key", &"TimelineKey(..)")
                .field("actor_generation", actor_generation)
                .field("generation", generation)
                .field("gap_count", &positions.len())
                .finish(),
            Self::GapRepairReleased {
                actor_generation,
                generation,
                ..
            } => formatter
                .debug_struct("GapRepairReleased")
                .field("key", &"TimelineKey(..)")
                .field("actor_generation", actor_generation)
                .field("generation", generation)
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
            Self::MediaSendQueued { request_id, .. } => formatter
                .debug_struct("MediaSendQueued")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("transaction_id", &"TransactionId(..)")
                .finish(),
            Self::SubmissionAccepted {
                request_id,
                submission_id,
                transaction_id,
                ..
            } => formatter
                .debug_struct("SubmissionAccepted")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("submission_id", submission_id)
                .field("transaction_id", transaction_id)
                .finish(),
            Self::SubmissionRejected {
                request_id,
                submission_id,
                kind,
                ..
            } => formatter
                .debug_struct("SubmissionRejected")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("submission_id", submission_id)
                .field("kind", kind)
                .finish(),
            Self::MessageForwarded { request_id, .. } => formatter
                .debug_struct("MessageForwarded")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("destination_room_id", &"RoomId(..)")
                .field("transaction_id", &"TransactionId(..)")
                .field("event_id", &"EventId(..)")
                .finish(),
            Self::MessageSourceLoaded { request_id, .. } => formatter
                .debug_struct("MessageSourceLoaded")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("source", &"TimelineMessageSource(..)")
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
            Self::MediaDownloadProgress {
                request_id,
                progress,
                ..
            } => formatter
                .debug_struct("MediaDownloadProgress")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("event_id", &"EventId(..)")
                .field("progress", progress)
                .finish(),
            Self::MediaDownloadCompleted {
                request_id,
                byte_count,
                mimetype,
                width,
                height,
                ..
            } => formatter
                .debug_struct("MediaDownloadCompleted")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("event_id", &"EventId(..)")
                .field("source_url", &"SourceUrl(..)")
                .field("byte_count", byte_count)
                .field("mimetype", mimetype)
                .field("width", width)
                .field("height", height)
                .finish(),
            Self::MediaDownloadFailed {
                request_id, kind, ..
            } => formatter
                .debug_struct("MediaDownloadFailed")
                .field("request_id", request_id)
                .field("key", &"TimelineKey(..)")
                .field("event_id", &"EventId(..)")
                .field("kind", kind)
                .finish(),
            Self::ResyncRequired { reason, .. } => formatter
                .debug_struct("ResyncRequired")
                .field("key", &"TimelineKey(..)")
                .field("reason", reason)
                .finish(),
            Self::DisplayPolicyUpdated { hide_redacted } => formatter
                .debug_struct("DisplayPolicyUpdated")
                .field("hide_redacted", hide_redacted)
                .finish(),
            Self::DisplayLabelsUpdated { labels } => formatter
                .debug_struct("DisplayLabelsUpdated")
                .field("label_count", &labels.len())
                .finish(),
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TimelineAnchorRestoreStatus {
    Found,
    EndReached,
    BudgetExhausted,
    Superseded,
    Failed { kind: TimelineFailureKind },
}
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineDisplayLabelUpdate {
    pub user_id: String,
    pub display_label: String,
}
impl fmt::Debug for TimelineDisplayLabelUpdate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TimelineDisplayLabelUpdate")
            .field("user_id", &"UserId(..)")
            .field("display_label", &"DisplayLabel(..)")
            .finish()
    }
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineViewportObservation {
    pub first_visible_event_id: Option<String>,
    pub last_visible_event_id: Option<String>,
    #[serde(default)]
    pub visible_gap_ids: Vec<TimelineGapId>,
    pub at_bottom: bool,
}
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct TimelineGapId {
    #[serde(with = "crate::u64_decimal_string")]
    pub topology_revision: u64,
    pub ordinal: u32,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineGapPosition {
    pub id: TimelineGapId,
    pub before_item_index: usize,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimelineReadStateSync {
    Synced,
    Pending,
    Failed { kind: ReadStateFailureKind },
    NotRequested,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineNavigationSnapshot {
    pub read_marker_event_id: Option<String>,
    pub read_marker_display_event_id: Option<String>,
    pub first_unread_event_id: Option<String>,
    pub unread_event_count: u64,
    pub unread_position: TimelineUnreadPosition,
    pub newer_event_count: u64,
    pub can_jump_to_bottom: bool,
    pub local_viewed_event_id: Option<String>,
    pub server_confirmed_read_event_id: Option<String>,
    pub read_state_sync: TimelineReadStateSync,
}
impl fmt::Debug for TimelineNavigationSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TimelineNavigationSnapshot")
            .field(
                "read_marker_event_id",
                &self.read_marker_event_id.as_ref().map(|_| "EventId(..)"),
            )
            .field(
                "read_marker_display_event_id",
                &self
                    .read_marker_display_event_id
                    .as_ref()
                    .map(|_| "EventId(..)"),
            )
            .field(
                "first_unread_event_id",
                &self.first_unread_event_id.as_ref().map(|_| "EventId(..)"),
            )
            .field(
                "local_viewed_event_id",
                &self.local_viewed_event_id.as_ref().map(|_| "EventId(..)"),
            )
            .field(
                "server_confirmed_read_event_id",
                &self
                    .server_confirmed_read_event_id
                    .as_ref()
                    .map(|_| "EventId(..)"),
            )
            .field("read_state_sync", &self.read_state_sync)
            .field("unread_event_count", &self.unread_event_count)
            .field("unread_position", &self.unread_position)
            .field("newer_event_count", &self.newer_event_count)
            .field("can_jump_to_bottom", &self.can_jump_to_bottom)
            .finish()
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimelineUnreadPosition {
    #[default]
    None,
    AboveViewport,
    InsideViewport,
    BelowViewport,
    Unknown,
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
    GapSettlementTimeout,
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
/// Rust-owned outbound send state for timeline local echoes.
///
/// This is a coarse public DTO: raw SDK errors stay in Rust logs/failures and
/// never cross the webview boundary.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TimelineSendState {
    Sending,
    NotSent { reason: TimelineSendFailureReason },
    Cancelled,
    Sent,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimelineSendFailureReason {
    Recoverable,
    Unrecoverable,
}
#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineMessageActions {
    pub can_copy: bool,
    pub can_forward: bool,
    #[serde(default)]
    pub can_reply: bool,
    pub can_permalink: bool,
    pub can_view_source: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permalink: Option<String>,
    /// Identity-bearing document for the shared inline edit surface. Plain-only
    /// legacy events remain text-only rather than guessing mention positions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editable_document: Option<ComposerDocument>,
}
impl fmt::Debug for TimelineMessageActions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TimelineMessageActions")
            .field("can_copy", &self.can_copy)
            .field("can_forward", &self.can_forward)
            .field("can_reply", &self.can_reply)
            .field("can_permalink", &self.can_permalink)
            .field("can_view_source", &self.can_view_source)
            .field(
                "permalink",
                &self.permalink.as_ref().map(|_| "Permalink(..)"),
            )
            .field(
                "editable_document",
                &self
                    .editable_document
                    .as_ref()
                    .map(|document| document.mention_intent().targets.len()),
            )
            .finish()
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimelineMegolmSessionReason {
    Initial,
    ExpiredTime,
    ExpiredMessageCount,
    MembershipOrDeviceChange,
    EncryptionSettingsChanged,
    ExplicitDiscard,
    FullMemberListReload,
    RoomSubscription,
    LimitedSyncResponse,
    KeyShareFailure,
    StoreMissing,
    Invalidated,
    Unknown,
    NotRetained,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineMessageSource {
    pub event_id: String,
    pub sender: Option<String>,
    pub timestamp_ms: Option<u64>,
    pub body: Option<String>,
    pub in_reply_to_event_id: Option<String>,
    pub thread_root: Option<String>,
    pub is_redacted: bool,
    pub is_edited: bool,
    pub has_media: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub megolm_session_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub megolm_message_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub megolm_session_rotation_reason: Option<TimelineMegolmSessionReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_json: Option<JsonValue>,
}
impl fmt::Debug for TimelineMessageSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TimelineMessageSource")
            .field("event_id", &"EventId(..)")
            .field("sender", &self.sender.as_ref().map(|_| "UserId(..)"))
            .field("timestamp_ms", &self.timestamp_ms)
            .field("body", &self.body.as_ref().map(|_| "MessageBody(..)"))
            .field(
                "in_reply_to_event_id",
                &self.in_reply_to_event_id.as_ref().map(|_| "EventId(..)"),
            )
            .field(
                "thread_root",
                &self.thread_root.as_ref().map(|_| "EventId(..)"),
            )
            .field("is_redacted", &self.is_redacted)
            .field("is_edited", &self.is_edited)
            .field("has_media", &self.has_media)
            .field(
                "megolm_session_fingerprint",
                &self
                    .megolm_session_fingerprint
                    .as_ref()
                    .map(|_| "MegolmSessionFingerprint(..)"),
            )
            .field("megolm_message_index", &self.megolm_message_index)
            .field(
                "megolm_session_rotation_reason",
                &self.megolm_session_rotation_reason,
            )
            .field(
                "original_json",
                &self.original_json.as_ref().map(|_| "OriginalEventJson(..)"),
            )
            .finish()
    }
}
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineCodeBlock {
    pub language: Option<String>,
    pub body: String,
}
impl fmt::Debug for TimelineCodeBlock {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TimelineCodeBlock")
            .field(
                "language",
                &self.language.as_ref().map(|_| "CodeBlockLanguage(..)"),
            )
            .field("body", &"CodeBlockBody(..)")
            .finish()
    }
}
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineFormattedBody {
    pub html: String,
    pub plain_text: String,
    pub code_blocks: Vec<TimelineCodeBlock>,
}
impl fmt::Debug for TimelineFormattedBody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TimelineFormattedBody")
            .field("html", &"FormattedHtml(..)")
            .field("plain_text", &"FormattedPlainText(..)")
            .field("code_blocks", &self.code_blocks.len())
            .finish()
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimelineMessageKind {
    #[default]
    Text,
    Emote,
    Notice,
}
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineSpoilerSpan {
    /// Start offset in JavaScript string units for the rendered text source.
    pub start_utf16: usize,
    /// End offset in JavaScript string units for the rendered text source.
    pub end_utf16: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
impl fmt::Debug for TimelineSpoilerSpan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TimelineSpoilerSpan")
            .field("start_utf16", &self.start_utf16)
            .field("end_utf16", &self.end_utf16)
            .field("reason", &self.reason.as_ref().map(|_| "SpoilerReason(..)"))
            .finish()
    }
}
/// Rust-owned plain-text link range. The URL itself is the authoritative,
/// Unicode-aware extraction from the message body; React renders anchors at
/// these UTF-16 offsets without re-parsing the text.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineLinkRange {
    pub url: String,
    /// Start offset in JavaScript string units for the rendered body text.
    pub start_utf16: usize,
    /// End offset in JavaScript string units for the rendered body text.
    pub end_utf16: usize,
}
impl fmt::Debug for TimelineLinkRange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TimelineLinkRange")
            .field("url", &"Url(..)")
            .field("start_utf16", &self.start_utf16)
            .field("end_utf16", &self.end_utf16)
            .finish()
    }
}
/// Timeline item DTO. Phase 5 concretizes content kinds from the SDK
/// projection; the identity contract is stable from Phase 1.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TimelineNoticeI18nKey {
    #[serde(rename = "timeline.notice.roomCreate")]
    RoomCreate,
    #[serde(rename = "timeline.notice.roomPowerLevels")]
    RoomPowerLevels,
    #[serde(rename = "timeline.notice.roomGuestAccess")]
    RoomGuestAccess,
    #[serde(rename = "timeline.notice.roomEncryption")]
    RoomEncryption,
    #[serde(rename = "timeline.notice.spaceParent")]
    SpaceParent,
    #[serde(rename = "timeline.notice.roomJoinRules")]
    RoomJoinRules,
    #[serde(rename = "timeline.notice.roomHistoryVisibility")]
    RoomHistoryVisibility,
    #[serde(rename = "timeline.notice.roomPinnedEvents")]
    RoomPinnedEvents,
    #[serde(rename = "timeline.notice.roomNameSet")]
    RoomNameSet,
    #[serde(rename = "timeline.notice.roomNameChanged")]
    RoomNameChanged,
    #[serde(rename = "timeline.notice.roomNameRemoved")]
    RoomNameRemoved,
    #[serde(rename = "timeline.notice.roomNameChangedGeneric")]
    RoomNameChangedGeneric,
}
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineNoticeI18n {
    pub key: TimelineNoticeI18nKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_name: Option<String>,
}
impl fmt::Debug for TimelineNoticeI18n {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TimelineNoticeI18n")
            .field("key", &self.key)
            .field("has_old_name", &self.old_name.is_some())
            .field("has_new_name", &self.new_name.is_some())
            .finish()
    }
}
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineItem {
    pub id: TimelineItemId,
    pub sender: Option<String>,
    #[serde(default)]
    pub sender_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_avatar: Option<AvatarImage>,
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice_i18n: Option<TimelineNoticeI18n>,
    #[serde(default)]
    pub message_kind: TimelineMessageKind,
    #[serde(default)]
    pub spoiler_spans: Vec<TimelineSpoilerSpan>,
    pub timestamp_ms: Option<u64>,
    pub in_reply_to_event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formatted: Option<TimelineFormattedBody>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_quote: Option<ReplyQuote>,
    #[serde(default)]
    pub thread_root: Option<String>,
    #[serde(default)]
    pub thread_summary: Option<ThreadSummaryDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<TimelineMedia>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_previews: Option<Vec<LinkPreview>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub link_ranges: Vec<TimelineLinkRange>,
    /// User ids this message's `m.mentions` named (#874). The renderer derives
    /// mention pills from the message instead of client-local profile labels,
    /// so the same event renders the same way for every viewer. Empty for
    /// items that are not messages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentioned_user_ids: Vec<String>,
    #[serde(default)]
    pub reactions: Vec<ReactionGroup>,
    #[serde(default)]
    pub can_react: bool,
    #[serde(default)]
    pub is_redacted: bool,
    #[serde(default)]
    pub is_hidden: bool,
    #[serde(default)]
    pub can_redact: bool,
    #[serde(default)]
    pub is_edited: bool,
    #[serde(default)]
    pub can_edit: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unable_to_decrypt: Option<TimelineUnableToDecrypt>,
    /// Room-key request presentation state (issue #460): closed stage + code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_state: Option<RoomKeyRequestStateDto>,
    #[serde(default)]
    pub actions: TimelineMessageActions,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_state: Option<TimelineSendState>,
    /// Rust-owned metadata for the bounded display projection. Canonical
    /// navigation items deliberately leave this unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_metadata: Option<TimelineDisplayMetadata>,
}
/// Rust-owned room-key request presentation state for a timeline item
/// (issue #460). Only closed tokens cross the wire.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomKeyRequestStage {
    Sent,
    Automatic,
    StillWaiting,
    Withheld,
    DecryptionRecovered,
    SendFailed,
}
/// Closed `m.room_key.withheld` codes correlatable from the SDK store
/// (issue #460). The SDK retains exactly these four codes; everything else
/// renders the generic refusal copy.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomKeyRequestWithheldCode {
    Blacklisted,
    Unverified,
    Unauthorised,
    Unavailable,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomKeyRequestStateDto {
    pub stage: RoomKeyRequestStage,
    pub withheld_code: Option<RoomKeyRequestWithheldCode>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineUnableToDecrypt {
    pub session_id: Option<String>,
    pub reason: TimelineUnableToDecryptReason,
    pub can_request_keys: bool,
    /// Closed recovery-stage token (issue #478), present while a standard-only
    /// recovery operation is running or settled for this session.
    pub recovery_stage: Option<String>,
    /// Closed terminal-guidance token (issue #478), present when automatic
    /// recovery is exhausted or impossible.
    pub recovery_guidance: Option<String>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimelineUnableToDecryptReason {
    MissingRoomKey,
    Withheld,
    Malformed,
    Unknown,
}
impl fmt::Debug for TimelineItem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TimelineItem")
            .field("id", &self.id)
            .field("sender", &self.sender.as_ref().map(|_| "UserId(..)"))
            .field(
                "sender_label",
                &self.sender_label.as_ref().map(|_| "SenderLabel(..)"),
            )
            .field(
                "sender_avatar",
                &self.sender_avatar.as_ref().map(|_| "AvatarImage(..)"),
            )
            .field("body", &self.body.as_ref().map(|_| "MessageBody(..)"))
            .field(
                "notice_i18n",
                &self.notice_i18n.as_ref().map(|notice| notice.key),
            )
            .field("message_kind", &self.message_kind)
            .field("spoiler_spans", &self.spoiler_spans.len())
            .field("timestamp_ms", &self.timestamp_ms)
            .field(
                "in_reply_to_event_id",
                &self.in_reply_to_event_id.as_ref().map(|_| "EventId(..)"),
            )
            .field(
                "formatted",
                &self.formatted.as_ref().map(|_| "TimelineFormattedBody(..)"),
            )
            .field(
                "reply_quote",
                &self.reply_quote.as_ref().map(|quote| quote.state.as_str()),
            )
            .field("thread_root", &self.thread_root)
            .field(
                "thread_summary",
                &self.thread_summary.as_ref().map(|_| "ThreadSummary(..)"),
            )
            .field("media", &self.media)
            .field(
                "link_previews",
                &self
                    .link_previews
                    .as_ref()
                    .map(|previews| format!("{} preview(s)", previews.len())),
            )
            .field("link_ranges", &self.link_ranges.len())
            .field("reactions", &self.reactions)
            .field("can_react", &self.can_react)
            .field("is_redacted", &self.is_redacted)
            .field("is_hidden", &self.is_hidden)
            .field("can_redact", &self.can_redact)
            .field("is_edited", &self.is_edited)
            .field("can_edit", &self.can_edit)
            .field("unable_to_decrypt", &self.unable_to_decrypt)
            .field("actions", &self.actions)
            .field("send_state", &self.send_state)
            .field(
                "display_metadata",
                &self.display_metadata.as_ref().map(|metadata| metadata.kind),
            )
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
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LinkPreviewState {
    #[default]
    Pending,
    Loading,
    Ready,
    Failed,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LinkPreviewImage {
    pub source: TimelineMediaSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u64>,
    #[serde(default)]
    pub thumbnail: AvatarThumbnailState,
}
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct LinkPreview {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<LinkPreviewImage>,
    #[serde(default)]
    pub state: LinkPreviewState,
}
impl fmt::Debug for LinkPreview {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LinkPreview")
            .field("state", &self.state)
            .field("has_image", &self.image.is_some())
            .finish()
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ThreadSummaryDto {
    pub reply_count: u32,
    pub latest_event_id: Option<String>,
    pub latest_sender: Option<String>,
    #[serde(default)]
    pub latest_sender_label: Option<String>,
    pub latest_body_preview: Option<String>,
    pub latest_timestamp_ms: Option<u64>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TimelineDisplayKind {
    Event,
    ThreadRoot,
    ThreadRootPending,
    ThreadRootFailed { failure_kind: OperationFailureKind },
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineDisplayMetadata {
    pub row_id: String,
    pub kind: TimelineDisplayKind,
    pub content_event_id: Option<String>,
    pub activity_event_id: Option<String>,
    pub display_timestamp_ms: Option<u64>,
}

impl fmt::Debug for TimelineDisplayMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TimelineDisplayMetadata")
            .field("row_id", &"RowId(..)")
            .field("kind", &self.kind)
            .field(
                "content_event_id",
                &self.content_event_id.as_ref().map(|_| "EventId(..)"),
            )
            .field(
                "activity_event_id",
                &self.activity_event_id.as_ref().map(|_| "EventId(..)"),
            )
            .field("display_timestamp_ms", &self.display_timestamp_ms)
            .finish()
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReactionGroup {
    pub key: String,
    pub count: u32,
    pub reacted_by_me: bool,
    pub my_reaction_event_id: Option<String>,
    pub sender_preview: Vec<ReactionSender>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReactionSender {
    pub user_id: String,
    pub display_label: Option<String>,
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
