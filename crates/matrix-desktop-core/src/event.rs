//! Public event boundary. Events carry the originating `RequestId` when one
//! exists; identifiers and visible bodies are allowed, secrets never.

use std::fmt;

use matrix_desktop_state::{
    ActivityStream, ActivityTab, AppState, CrossSigningStatus, DirectoryQuery,
    DirectoryRoomSummary, IdentityResetState, JapaneseCatalogProfile, KeyBackupStatus,
    LiveRoomSignalUpdate, LocalEncryptionHealth, NativeAttentionSummary, PinnedEvent, PresenceKind,
    ProfileState, ReplyQuote, RoomModerationAction, RoomSettingsSnapshot, RoomTagKind,
    SessionState, VerificationFlowState, resolve_user_display_name,
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
    Activity(ActivityEvent),
    LocalEncryption(LocalEncryptionEvent),
    NativeAttention(NativeAttentionEvent),
    CjkTextPolicy(CjkTextPolicyEvent),
    OperationFailed {
        request_id: RequestId,
        failure: CoreFailure,
    },
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum ActivityEvent {
    Opened {
        request_id: RequestId,
    },
    Closed {
        request_id: RequestId,
    },
    SnapshotLoaded {
        request_id: RequestId,
        active_tab: ActivityTab,
        recent: ActivityStream,
        unread: ActivityStream,
    },
    TabSelected {
        request_id: RequestId,
        tab: ActivityTab,
    },
    MarkedRead {
        request_id: RequestId,
        cleared_event_ids: Vec<String>,
    },
}

impl fmt::Debug for ActivityEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Opened { request_id } => formatter
                .debug_struct("ActivityOpened")
                .field("request_id", request_id)
                .finish(),
            Self::Closed { request_id } => formatter
                .debug_struct("ActivityClosed")
                .field("request_id", request_id)
                .finish(),
            Self::SnapshotLoaded {
                request_id,
                active_tab,
                recent,
                unread,
            } => formatter
                .debug_struct("ActivitySnapshotLoaded")
                .field("request_id", request_id)
                .field("active_tab", active_tab)
                .field("recent", recent)
                .field("unread", unread)
                .finish(),
            Self::TabSelected { request_id, tab } => formatter
                .debug_struct("ActivityTabSelected")
                .field("request_id", request_id)
                .field("tab", tab)
                .finish(),
            Self::MarkedRead {
                request_id,
                cleared_event_ids,
            } => formatter
                .debug_struct("ActivityMarkedRead")
                .field("request_id", request_id)
                .field(
                    "cleared_event_ids",
                    &format_args!("{} event id(s)", cleared_event_ids.len()),
                )
                .finish(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LocalEncryptionEvent {
    HealthChanged { health: LocalEncryptionHealth },
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum NativeAttentionEvent {
    SummaryUpdated { summary: NativeAttentionSummary },
}

impl fmt::Debug for NativeAttentionEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SummaryUpdated { summary } => formatter
                .debug_struct("SummaryUpdated")
                .field("unread_count", &summary.unread_count)
                .field("highlight_count", &summary.highlight_count)
                .field("badge_count", &summary.badge_count)
                .field(
                    "candidate",
                    &summary.candidate.as_ref().map(|_| "AttentionCandidate(..)"),
                )
                .finish(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CjkTextPolicyEvent {
    JapaneseCatalogProfileChanged { profile: JapaneseCatalogProfile },
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

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
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
    RoomTagSet {
        request_id: RequestId,
        room_id: String,
        tag: RoomTagKind,
    },
    RoomTagRemoved {
        request_id: RequestId,
        room_id: String,
        tag: RoomTagKind,
    },
    PinnedEventsUpdated {
        room_id: String,
        pinned: Vec<PinnedEvent>,
    },
    PinEventCompleted {
        request_id: RequestId,
        room_id: String,
    },
    UnpinEventCompleted {
        request_id: RequestId,
        room_id: String,
    },
    DirectoryQueryCompleted {
        request_id: RequestId,
        query: DirectoryQuery,
        rooms: Vec<DirectoryRoomSummary>,
        next_batch: Option<String>,
    },
    RoomSettingsLoaded {
        request_id: RequestId,
        settings: RoomSettingsSnapshot,
    },
    RoomSettingUpdated {
        request_id: RequestId,
        settings: RoomSettingsSnapshot,
    },
    RoomMemberModerated {
        request_id: RequestId,
        room_id: String,
        target_user_id: String,
        action: RoomModerationAction,
    },
    RoomMemberRoleUpdated {
        request_id: RequestId,
        room_id: String,
        target_user_id: String,
        power_level: i64,
    },
    RoomListUpdated,
}

impl fmt::Debug for RoomEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RoomCreated { request_id, .. } => formatter
                .debug_struct("RoomCreated")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::SpaceCreated { request_id, .. } => formatter
                .debug_struct("SpaceCreated")
                .field("request_id", request_id)
                .field("space_id", &"RoomId(..)")
                .finish(),
            Self::SpaceChildSet { request_id, .. } => formatter
                .debug_struct("SpaceChildSet")
                .field("request_id", request_id)
                .field("space_id", &"RoomId(..)")
                .field("child_room_id", &"RoomId(..)")
                .finish(),
            Self::UserInvited { request_id, .. } => formatter
                .debug_struct("UserInvited")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("user_id", &"UserId(..)")
                .finish(),
            Self::InviteAccepted { request_id, .. } => formatter
                .debug_struct("InviteAccepted")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::InviteDeclined { request_id, .. } => formatter
                .debug_struct("InviteDeclined")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::DirectMessageStarted { request_id, .. } => formatter
                .debug_struct("DirectMessageStarted")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::RoomJoined { request_id, .. } => formatter
                .debug_struct("RoomJoined")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::RoomLeft { request_id, .. } => formatter
                .debug_struct("RoomLeft")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::RoomForgotten { request_id, .. } => formatter
                .debug_struct("RoomForgotten")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::RoomTagSet {
                request_id, tag, ..
            } => formatter
                .debug_struct("RoomTagSet")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("tag", tag)
                .finish(),
            Self::RoomTagRemoved {
                request_id, tag, ..
            } => formatter
                .debug_struct("RoomTagRemoved")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("tag", tag)
                .finish(),
            Self::PinnedEventsUpdated { pinned, .. } => formatter
                .debug_struct("PinnedEventsUpdated")
                .field("room_id", &"RoomId(..)")
                .field("pinned_count", &pinned.len())
                .finish(),
            Self::PinEventCompleted { request_id, .. } => formatter
                .debug_struct("PinEventCompleted")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::UnpinEventCompleted { request_id, .. } => formatter
                .debug_struct("UnpinEventCompleted")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::DirectoryQueryCompleted {
                request_id, rooms, ..
            } => formatter
                .debug_struct("DirectoryQueryCompleted")
                .field("request_id", request_id)
                .field("query", &"DirectoryQuery(..)")
                .field("rooms_count", &rooms.len())
                .finish(),
            Self::RoomSettingsLoaded { request_id, .. } => formatter
                .debug_struct("RoomSettingsLoaded")
                .field("request_id", request_id)
                .field("settings", &"RoomSettingsSnapshot(..)")
                .finish(),
            Self::RoomSettingUpdated { request_id, .. } => formatter
                .debug_struct("RoomSettingUpdated")
                .field("request_id", request_id)
                .field("settings", &"RoomSettingsSnapshot(..)")
                .finish(),
            Self::RoomMemberModerated {
                request_id, action, ..
            } => formatter
                .debug_struct("RoomMemberModerated")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("target_user_id", &"UserId(..)")
                .field("action", action)
                .finish(),
            Self::RoomMemberRoleUpdated {
                request_id,
                power_level,
                ..
            } => formatter
                .debug_struct("RoomMemberRoleUpdated")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("target_user_id", &"UserId(..)")
                .field("power_level", power_level)
                .finish(),
            Self::RoomListUpdated => formatter.write_str("RoomListUpdated"),
        }
    }
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
    NavigationUpdated {
        key: TimelineKey,
        snapshot: TimelineNavigationSnapshot,
    },
    SendCompleted {
        request_id: RequestId,
        key: TimelineKey,
        transaction_id: String,
        event_id: String,
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
            Self::NavigationUpdated { snapshot, .. } => formatter
                .debug_struct("NavigationUpdated")
                .field("key", &"TimelineKey(..)")
                .field("snapshot", snapshot)
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
    pub at_bottom: bool,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineNavigationSnapshot {
    pub read_marker_event_id: Option<String>,
    pub first_unread_event_id: Option<String>,
    pub unread_event_count: u64,
    pub unread_position: TimelineUnreadPosition,
    pub newer_event_count: u64,
    pub can_jump_to_bottom: bool,
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
                "first_unread_event_id",
                &self.first_unread_event_id.as_ref().map(|_| "EventId(..)"),
            )
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
    pub can_permalink: bool,
    pub can_view_source: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permalink: Option<String>,
}

impl fmt::Debug for TimelineMessageActions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TimelineMessageActions")
            .field("can_copy", &self.can_copy)
            .field("can_forward", &self.can_forward)
            .field("can_permalink", &self.can_permalink)
            .field("can_view_source", &self.can_view_source)
            .field(
                "permalink",
                &self.permalink.as_ref().map(|_| "Permalink(..)"),
            )
            .finish()
    }
}

pub fn message_actions_for_timeline_item(
    room_id: &str,
    item_id: &TimelineItemId,
    body: Option<&str>,
    _has_media: bool,
    is_redacted: bool,
) -> TimelineMessageActions {
    let TimelineItemId::Event { event_id } = item_id else {
        return TimelineMessageActions::default();
    };

    let has_body = body.map(|body| !body.is_empty()).unwrap_or(false);
    let permalink = matrix_to_event_permalink(room_id, event_id);

    TimelineMessageActions {
        can_copy: has_body && !is_redacted,
        can_forward: has_body && !is_redacted,
        can_permalink: permalink.is_some(),
        can_view_source: !event_id.trim().is_empty(),
        permalink,
    }
}

pub fn matrix_to_event_permalink(room_id: &str, event_id: &str) -> Option<String> {
    if room_id.trim().is_empty() || event_id.trim().is_empty() {
        return None;
    }

    Some(format!(
        "https://matrix.to/#/{}/{}",
        percent_encode_matrix_to_component(room_id),
        percent_encode_matrix_to_component(event_id)
    ))
}

fn percent_encode_matrix_to_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'!') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(hex_digit(byte >> 4));
            encoded.push(hex_digit(byte & 0x0f));
        }
    }
    encoded
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'A' + (value - 10)) as char,
        _ => unreachable!("hex digit nibble"),
    }
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
            .finish()
    }
}

pub fn message_source_for_timeline_item(item: &TimelineItem) -> Option<TimelineMessageSource> {
    let TimelineItemId::Event { event_id } = &item.id else {
        return None;
    };

    Some(TimelineMessageSource {
        event_id: event_id.clone(),
        sender: item.sender.clone(),
        timestamp_ms: item.timestamp_ms,
        body: item.body.clone(),
        in_reply_to_event_id: item.in_reply_to_event_id.clone(),
        thread_root: item.thread_root.clone(),
        is_redacted: item.is_redacted,
        is_edited: item.is_edited,
        has_media: item.media.is_some(),
    })
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

/// Timeline item DTO. Phase 5 concretizes content kinds from the SDK
/// projection; the identity contract is stable from Phase 1.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineItem {
    pub id: TimelineItemId,
    pub sender: Option<String>,
    #[serde(default)]
    pub sender_label: Option<String>,
    pub body: Option<String>,
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
    #[serde(default)]
    pub actions: TimelineMessageActions,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_state: Option<TimelineSendState>,
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
            .field("body", &self.body.as_ref().map(|_| "MessageBody(..)"))
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
            .field("reactions", &self.reactions)
            .field("can_react", &self.can_react)
            .field("is_redacted", &self.is_redacted)
            .field("is_hidden", &self.is_hidden)
            .field("can_redact", &self.can_redact)
            .field("is_edited", &self.is_edited)
            .field("can_edit", &self.can_edit)
            .field("actions", &self.actions)
            .field("send_state", &self.send_state)
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
    #[serde(default)]
    pub latest_sender_label: Option<String>,
    pub latest_body_preview: Option<String>,
    pub latest_timestamp_ms: Option<u64>,
}

pub fn project_timeline_event_display_labels(event: &mut TimelineEvent, state: &AppState) {
    match event {
        TimelineEvent::InitialItems { items, .. } => {
            for item in items {
                project_timeline_item_display_labels(item, state);
            }
        }
        TimelineEvent::ItemsUpdated { diffs, .. } => {
            for diff in diffs {
                project_timeline_diff_display_labels(diff, state);
            }
        }
        TimelineEvent::PaginationStateChanged { .. }
        | TimelineEvent::SendCompleted { .. }
        | TimelineEvent::MessageSourceLoaded { .. }
        | TimelineEvent::MessageForwarded { .. }
        | TimelineEvent::MediaUploadProgress { .. }
        | TimelineEvent::MediaDownloadCompleted { .. }
        | TimelineEvent::ResyncRequired { .. }
        | TimelineEvent::NavigationUpdated { .. }
        | TimelineEvent::DisplayPolicyUpdated { .. }
        | TimelineEvent::DisplayLabelsUpdated { .. } => {}
    }
}

pub fn project_room_event_display_labels(event: &mut RoomEvent, state: &AppState) {
    match event {
        RoomEvent::RoomSettingsLoaded { settings, .. }
        | RoomEvent::RoomSettingUpdated { settings, .. } => {
            matrix_desktop_state::refresh_room_settings_member_display_projection(
                settings,
                &state.profile,
                timeline_projection_own_user_id(state),
            );
        }
        RoomEvent::RoomListUpdated
        | RoomEvent::RoomCreated { .. }
        | RoomEvent::SpaceCreated { .. }
        | RoomEvent::SpaceChildSet { .. }
        | RoomEvent::RoomJoined { .. }
        | RoomEvent::RoomLeft { .. }
        | RoomEvent::RoomForgotten { .. }
        | RoomEvent::UserInvited { .. }
        | RoomEvent::InviteAccepted { .. }
        | RoomEvent::InviteDeclined { .. }
        | RoomEvent::DirectMessageStarted { .. }
        | RoomEvent::RoomTagSet { .. }
        | RoomEvent::RoomTagRemoved { .. }
        | RoomEvent::PinnedEventsUpdated { .. }
        | RoomEvent::PinEventCompleted { .. }
        | RoomEvent::UnpinEventCompleted { .. }
        | RoomEvent::DirectoryQueryCompleted { .. }
        | RoomEvent::RoomMemberModerated { .. }
        | RoomEvent::RoomMemberRoleUpdated { .. } => {}
    }
}

pub fn project_timeline_item_display_labels(item: &mut TimelineItem, state: &AppState) {
    item.sender_label = timeline_sender_label(item.sender.as_deref(), state);
    item.is_hidden = state.settings.values.display.hide_redacted && item.is_redacted;
    if let Some(reply_quote) = item.reply_quote.as_mut() {
        reply_quote.sender_label = timeline_sender_label(reply_quote.sender.as_deref(), state);
    }
    if let Some(thread_summary) = item.thread_summary.as_mut() {
        thread_summary.latest_sender_label =
            timeline_sender_label(thread_summary.latest_sender.as_deref(), state);
    }
}

fn project_timeline_diff_display_labels(diff: &mut TimelineDiff, state: &AppState) {
    match diff {
        TimelineDiff::PushFront { item }
        | TimelineDiff::PushBack { item }
        | TimelineDiff::Insert { item, .. }
        | TimelineDiff::Set { item, .. } => project_timeline_item_display_labels(item, state),
        TimelineDiff::Reset { items } => {
            for item in items {
                project_timeline_item_display_labels(item, state);
            }
        }
        TimelineDiff::Remove { .. } | TimelineDiff::Truncate { .. } | TimelineDiff::Clear => {}
    }
}

fn timeline_sender_label(sender: Option<&str>, state: &AppState) -> Option<String> {
    let sender = sender?;
    Some(resolve_user_display_name(
        &state.profile,
        sender,
        None,
        timeline_projection_own_user_id(state),
    ))
}

pub fn timeline_projection_own_user_id(state: &AppState) -> Option<&str> {
    match &state.session {
        SessionState::Ready(info) => Some(info.user_id.as_str()),
        SessionState::SignedOut
        | SessionState::Restoring
        | SessionState::Authenticating { .. }
        | SessionState::NeedsRecovery { .. }
        | SessionState::Recovering { .. }
        | SessionState::LoggingOut
        | SessionState::Locked(_)
        | SessionState::SwitchingAccount { .. } => None,
    }
}

pub fn derive_display_label_updates(
    profile: &ProfileState,
    own_user_id: Option<&str>,
) -> Vec<TimelineDisplayLabelUpdate> {
    derive_display_label_updates_for_user_ids(profile, own_user_id, std::iter::empty::<&str>())
}

pub fn derive_display_label_updates_for_user_ids<'a>(
    profile: &ProfileState,
    own_user_id: Option<&str>,
    additional_user_ids: impl IntoIterator<Item = &'a str>,
) -> Vec<TimelineDisplayLabelUpdate> {
    let mut seen = std::collections::BTreeSet::new();
    let mut updates = Vec::new();

    let mut push = |user_id: &str| {
        if !seen.insert(user_id.to_owned()) {
            return;
        }
        let label = resolve_user_display_name(profile, user_id, None, own_user_id);
        updates.push(TimelineDisplayLabelUpdate {
            user_id: user_id.to_owned(),
            display_label: label,
        });
    };

    for uid in profile.local_aliases.keys() {
        push(uid);
    }
    for uid in profile.users.keys() {
        push(uid);
    }
    if let Some(uid) = own_user_id {
        push(uid);
    }
    for uid in additional_user_ids {
        push(uid);
    }

    updates
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

    fn fake_rid(sequence: u64) -> RequestId {
        RequestId {
            connection_id: crate::ids::RuntimeConnectionId(7),
            sequence,
        }
    }

    fn activity_row(
        room_id: &str,
        event_id: &str,
        timestamp_ms: u64,
    ) -> matrix_desktop_state::ActivityRow {
        matrix_desktop_state::ActivityRow {
            room_id: room_id.to_owned(),
            event_id: event_id.to_owned(),
            room_label: "Private Room".to_owned(),
            sender_label: Some("Private Sender".to_owned()),
            preview: Some("private message body".to_owned()),
            timestamp_ms,
            unread: true,
            highlight: false,
        }
    }

    fn activity_stream(
        rows: Vec<matrix_desktop_state::ActivityRow>,
    ) -> matrix_desktop_state::ActivityStream {
        matrix_desktop_state::ActivityStream {
            rows,
            next_batch: Some("private-page-token".to_owned()),
        }
    }

    #[test]
    fn activity_events_debug_redacts_rows_targets_and_page_tokens() {
        let snapshot = ActivityEvent::SnapshotLoaded {
            request_id: fake_rid(1),
            active_tab: matrix_desktop_state::ActivityTab::Recent,
            recent: activity_stream(vec![activity_row(
                "!private-room:example.invalid",
                "$private-event:example.invalid",
                20,
            )]),
            unread: activity_stream(vec![activity_row(
                "!private-room:example.invalid",
                "$private-unread:example.invalid",
                10,
            )]),
        };
        let marked = ActivityEvent::MarkedRead {
            request_id: fake_rid(2),
            cleared_event_ids: vec!["$private-event:example.invalid".to_owned()],
        };

        for debug in [format!("{snapshot:?}"), format!("{marked:?}")] {
            assert!(!debug.contains("!private-room:example.invalid"), "{debug}");
            assert!(!debug.contains("$private-event:example.invalid"), "{debug}");
            assert!(
                !debug.contains("$private-unread:example.invalid"),
                "{debug}"
            );
            assert!(!debug.contains("Private Room"), "{debug}");
            assert!(!debug.contains("Private Sender"), "{debug}");
            assert!(!debug.contains("private message body"), "{debug}");
            assert!(!debug.contains("private-page-token"), "{debug}");
        }
    }

    #[test]
    fn timeline_item_serializes_thread_fields_reactions_and_redaction_affordances() {
        let item = TimelineItem {
            id: TimelineItemId::Event {
                event_id: "$event:test".to_owned(),
            },
            sender: Some("@alice:example.invalid".to_owned()),
            sender_label: None,
            body: Some("hello".to_owned()),
            message_kind: Default::default(),
            spoiler_spans: Vec::new(),
            timestamp_ms: Some(1_234),
            in_reply_to_event_id: None,
            formatted: None,
            reply_quote: None,
            thread_root: Some("$root:test".to_owned()),
            thread_summary: Some(ThreadSummaryDto {
                reply_count: 2,
                latest_sender: Some("@bob:example.invalid".to_owned()),
                latest_sender_label: None,
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
            is_hidden: false,
            can_redact: true,
            is_edited: true,
            can_edit: true,
            actions: TimelineMessageActions::default(),
            send_state: None,
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
                "latest_sender_label": null,
                "latest_body_preview": "latest reply",
                "latest_timestamp_ms": 1456
            })
        );
    }

    #[test]
    fn timeline_item_serializes_reply_quote_without_debugging_body() {
        let item = TimelineItem {
            id: TimelineItemId::Event {
                event_id: "$reply:test".to_owned(),
            },
            sender: Some("@alice:example.invalid".to_owned()),
            sender_label: None,
            body: Some("reply body".to_owned()),
            message_kind: Default::default(),
            spoiler_spans: Vec::new(),
            timestamp_ms: Some(1_234),
            in_reply_to_event_id: Some("$root:test".to_owned()),
            formatted: None,
            reply_quote: Some(matrix_desktop_state::ReplyQuote {
                event_id: "$root:test".to_owned(),
                sender: Some("@bob:example.invalid".to_owned()),
                sender_label: None,
                body_preview: Some("quoted body".to_owned()),
                state: matrix_desktop_state::ReplyQuoteState::Ready,
            }),
            thread_root: None,
            thread_summary: None,
            media: None,
            reactions: Vec::new(),
            can_react: true,
            is_redacted: false,
            is_hidden: false,
            can_redact: true,
            is_edited: false,
            can_edit: false,
            actions: TimelineMessageActions::default(),
            send_state: None,
        };

        let value = serde_json::to_value(&item).expect("timeline item serializes");

        assert_eq!(
            value["reply_quote"],
            json!({
                "event_id": "$root:test",
                "sender": "@bob:example.invalid",
                "sender_label": null,
                "body_preview": "quoted body",
                "state": "ready"
            })
        );
        let debug = format!("{item:?}");
        assert!(debug.contains("reply_quote"));
        assert!(!debug.contains("quoted body"), "{debug}");
        assert!(!debug.contains("$root:test"), "{debug}");
    }

    #[test]
    fn timeline_item_serializes_formatted_body_without_debugging_content() {
        let item = TimelineItem {
            id: TimelineItemId::Event {
                event_id: "$formatted:test".to_owned(),
            },
            sender: Some("@alice:example.invalid".to_owned()),
            sender_label: None,
            body: Some("plain fallback".to_owned()),
            message_kind: TimelineMessageKind::Emote,
            spoiler_spans: vec![TimelineSpoilerSpan {
                start_utf16: 0,
                end_utf16: 13,
                reason: Some("reason".to_owned()),
            }],
            timestamp_ms: Some(1_234),
            in_reply_to_event_id: None,
            formatted: Some(TimelineFormattedBody {
                html: "<strong>private html</strong><pre><code class=\"language-rust\">private_code()</code></pre>"
                    .to_owned(),
                plain_text: "private htmlprivate_code()".to_owned(),
                code_blocks: vec![TimelineCodeBlock {
                    language: Some("rust".to_owned()),
                    body: "private_code()".to_owned(),
                }],
            }),
            reply_quote: None,
            thread_root: None,
            thread_summary: None,
            media: None,
            reactions: Vec::new(),
            can_react: true,
            is_redacted: false,
            is_hidden: false,
            can_redact: true,
            is_edited: false,
            can_edit: true,
            actions: TimelineMessageActions::default(),
            send_state: None,
        };

        let value = serde_json::to_value(&item).expect("timeline item serializes");

        assert_eq!(
            value["formatted"],
            json!({
                "html": "<strong>private html</strong><pre><code class=\"language-rust\">private_code()</code></pre>",
                "plain_text": "private htmlprivate_code()",
                "code_blocks": [
                    {
                        "language": "rust",
                        "body": "private_code()"
                    }
                ]
            })
        );
        assert_eq!(value["message_kind"], json!("emote"));
        assert_eq!(
            value["spoiler_spans"],
            json!([
                {
                    "start_utf16": 0,
                    "end_utf16": 13,
                    "reason": "reason"
                }
            ])
        );
        let debug = format!("{item:?}");
        assert!(debug.contains("TimelineFormattedBody"));
        assert!(!debug.contains("private html"), "{debug}");
        assert!(!debug.contains("private_code"), "{debug}");
        assert!(!debug.contains("language-rust"), "{debug}");
        assert!(!debug.contains("reason"), "{debug}");
        assert!(!debug.contains("$formatted:test"), "{debug}");
    }

    #[test]
    fn timeline_item_serializes_rust_owned_message_actions() {
        let item = TimelineItem {
            id: TimelineItemId::Event {
                event_id: "$event:test".to_owned(),
            },
            sender: Some("@alice:example.invalid".to_owned()),
            sender_label: None,
            body: Some("copyable body".to_owned()),
            message_kind: Default::default(),
            spoiler_spans: Vec::new(),
            timestamp_ms: Some(1_234),
            in_reply_to_event_id: None,
            formatted: None,
            reply_quote: None,
            thread_root: None,
            thread_summary: None,
            media: None,
            reactions: Vec::new(),
            can_react: true,
            is_redacted: false,
            is_hidden: false,
            can_redact: true,
            is_edited: false,
            can_edit: true,
            actions: message_actions_for_timeline_item(
                "!room:test",
                &TimelineItemId::Event {
                    event_id: "$event:test".to_owned(),
                },
                Some("copyable body"),
                false,
                false,
            ),
            send_state: None,
        };

        let value = serde_json::to_value(&item).expect("timeline item serializes");

        assert_eq!(
            value["actions"],
            json!({
                "can_copy": true,
                "can_forward": true,
                "can_permalink": true,
                "can_view_source": true,
                "permalink": "https://matrix.to/#/!room%3Atest/%24event%3Atest"
            })
        );
        let debug = format!("{item:?}");
        assert!(debug.contains("actions"), "{debug}");
        assert!(!debug.contains("https://matrix.to"), "{debug}");
        assert!(!debug.contains("$event:test"), "{debug}");
        assert!(!debug.contains("!room:test"), "{debug}");

        let redacted = message_actions_for_timeline_item(
            "!room:test",
            &TimelineItemId::Event {
                event_id: "$redacted:test".to_owned(),
            },
            Some("redacted body"),
            true,
            true,
        );
        assert!(!redacted.can_copy);
        assert!(!redacted.can_forward);
        assert!(redacted.can_permalink);
        assert!(redacted.can_view_source);

        let media_without_body = message_actions_for_timeline_item(
            "!room:test",
            &TimelineItemId::Event {
                event_id: "$media:test".to_owned(),
            },
            None,
            true,
            false,
        );
        assert!(!media_without_body.can_copy);
        assert!(!media_without_body.can_forward);
        assert!(media_without_body.can_permalink);
        assert!(media_without_body.can_view_source);

        let local_echo = message_actions_for_timeline_item(
            "!room:test",
            &TimelineItemId::Transaction {
                transaction_id: "txn:test".to_owned(),
            },
            Some("local echo"),
            false,
            false,
        );
        assert_eq!(local_echo, TimelineMessageActions::default());
    }

    #[test]
    fn message_source_and_forward_events_are_typed_and_redacted_in_debug() {
        let key = TimelineKey::room(AccountKey("@alice:test".to_owned()), "!room:test");
        let source = TimelineMessageSource {
            event_id: "$event:test".to_owned(),
            sender: Some("@alice:test".to_owned()),
            timestamp_ms: Some(1234),
            body: Some("private source body".to_owned()),
            in_reply_to_event_id: Some("$root:test".to_owned()),
            thread_root: Some("$thread:test".to_owned()),
            is_redacted: false,
            is_edited: true,
            has_media: false,
        };
        let loaded = TimelineEvent::MessageSourceLoaded {
            request_id: fake_rid(30),
            key: key.clone(),
            source: source.clone(),
        };
        let forwarded = TimelineEvent::MessageForwarded {
            request_id: fake_rid(31),
            key,
            destination_room_id: "!destination:test".to_owned(),
            transaction_id: "txn-forward-private".to_owned(),
            event_id: "$forwarded:test".to_owned(),
        };

        let value = serde_json::to_value(&loaded).expect("source event serializes");
        assert_eq!(
            value,
            json!({
                "MessageSourceLoaded": {
                    "request_id": { "connection_id": 7, "sequence": 30 },
                    "key": {
                        "account_key": "@alice:test",
                        "kind": { "Room": { "room_id": "!room:test" } }
                    },
                    "source": {
                        "event_id": "$event:test",
                        "sender": "@alice:test",
                        "timestamp_ms": 1234,
                        "body": "private source body",
                        "in_reply_to_event_id": "$root:test",
                        "thread_root": "$thread:test",
                        "is_redacted": false,
                        "is_edited": true,
                        "has_media": false
                    }
                }
            })
        );

        for debug in [
            format!("{source:?}"),
            format!("{loaded:?}"),
            format!("{forwarded:?}"),
        ] {
            assert!(!debug.contains("private source body"), "{debug}");
            assert!(!debug.contains("$event:test"), "{debug}");
            assert!(!debug.contains("$root:test"), "{debug}");
            assert!(!debug.contains("$thread:test"), "{debug}");
            assert!(!debug.contains("$forwarded:test"), "{debug}");
            assert!(!debug.contains("!destination:test"), "{debug}");
            assert!(!debug.contains("txn-forward-private"), "{debug}");
        }
    }

    #[test]
    fn timeline_item_serializes_outbound_send_state_without_raw_error() {
        let item = TimelineItem {
            id: TimelineItemId::Transaction {
                transaction_id: "txn-send-state".to_owned(),
            },
            sender: Some("@alice:example.invalid".to_owned()),
            sender_label: None,
            body: Some("hello".to_owned()),
            message_kind: Default::default(),
            spoiler_spans: Vec::new(),
            timestamp_ms: Some(1_234),
            in_reply_to_event_id: None,
            formatted: None,
            reply_quote: None,
            thread_root: None,
            thread_summary: None,
            media: None,
            reactions: Vec::new(),
            can_react: false,
            is_redacted: false,
            is_hidden: false,
            can_redact: false,
            is_edited: false,
            can_edit: false,
            actions: TimelineMessageActions::default(),
            send_state: Some(TimelineSendState::NotSent {
                reason: TimelineSendFailureReason::Recoverable,
            }),
        };

        let value = serde_json::to_value(&item).expect("timeline item serializes");

        assert_eq!(
            value["send_state"],
            json!({
                "kind": "notSent",
                "reason": "recoverable"
            })
        );
        let debug = format!("{item:?}");
        assert!(debug.contains("NotSent"), "{debug}");
        assert!(!debug.contains("hello"), "{debug}");
    }

    #[test]
    fn timeline_item_serializes_media_metadata_without_encryption_secrets() {
        let item = TimelineItem {
            id: TimelineItemId::Event {
                event_id: "$media:test".to_owned(),
            },
            sender: Some("@alice:example.invalid".to_owned()),
            sender_label: None,
            body: Some("synthetic caption".to_owned()),
            message_kind: Default::default(),
            spoiler_spans: Vec::new(),
            timestamp_ms: Some(1_234),
            in_reply_to_event_id: None,
            formatted: None,
            reply_quote: None,
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
            is_hidden: false,
            can_redact: true,
            is_edited: false,
            can_edit: false,
            actions: TimelineMessageActions::default(),
            send_state: None,
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

    #[test]
    fn room_member_role_event_debug_redacts_room_and_user_ids() {
        let event = RoomEvent::RoomMemberRoleUpdated {
            request_id: fake_rid(44),
            room_id: "!private-room:example.invalid".to_owned(),
            target_user_id: "@private-target:example.invalid".to_owned(),
            power_level: 50,
        };

        let debug = format!("{event:?}");
        assert!(debug.contains("RoomMemberRoleUpdated"), "{debug}");
        assert!(debug.contains("power_level"), "{debug}");
        assert!(!debug.contains("!private-room:example.invalid"), "{debug}");
        assert!(
            !debug.contains("@private-target:example.invalid"),
            "{debug}"
        );
    }

    #[test]
    fn display_labels_updated_event_serializes_and_redacts_debug() {
        let labels = vec![
            TimelineDisplayLabelUpdate {
                user_id: "@alice:example.invalid".to_owned(),
                display_label: "Alice Alias".to_owned(),
            },
            TimelineDisplayLabelUpdate {
                user_id: "@bob:example.invalid".to_owned(),
                display_label: "Bobby".to_owned(),
            },
        ];
        let event = TimelineEvent::DisplayLabelsUpdated { labels };

        let value = serde_json::to_value(&event).expect("DisplayLabelsUpdated serializes");
        assert_eq!(
            value,
            json!({
                "DisplayLabelsUpdated": {
                    "labels": [
                        { "user_id": "@alice:example.invalid", "display_label": "Alice Alias" },
                        { "user_id": "@bob:example.invalid", "display_label": "Bobby" }
                    ]
                }
            })
        );

        let debug = format!("{event:?}");
        assert!(debug.contains("DisplayLabelsUpdated"), "{debug}");
        assert!(!debug.contains("@alice:example.invalid"), "{debug}");
        assert!(!debug.contains("@bob:example.invalid"), "{debug}");
        assert!(!debug.contains("Alice Alias"), "{debug}");
        assert!(!debug.contains("Bobby"), "{debug}");
    }

    #[test]
    fn timeline_items_project_redacted_visibility_from_settings() {
        let mut state = AppState::default();
        state.settings.values.display.hide_redacted = true;
        let key = TimelineKey::room(
            AccountKey("@me:example.invalid".to_owned()),
            "!room:example.invalid",
        );
        let mut event = TimelineEvent::InitialItems {
            request_id: None,
            key,
            generation: TimelineGeneration(0),
            items: vec![
                timeline_item_fixture("$redacted:example.invalid", true),
                timeline_item_fixture("$visible:example.invalid", false),
            ],
        };

        project_timeline_event_display_labels(&mut event, &state);

        let TimelineEvent::InitialItems { items, .. } = event else {
            panic!("expected InitialItems");
        };
        assert!(items[0].is_redacted);
        assert!(items[0].is_hidden);
        assert!(!items[1].is_redacted);
        assert!(!items[1].is_hidden);
    }

    #[test]
    fn timeline_display_policy_update_serializes_and_redacts_debug() {
        let event = TimelineEvent::DisplayPolicyUpdated {
            hide_redacted: true,
        };

        let value = serde_json::to_value(&event).expect("DisplayPolicyUpdated serializes");
        assert_eq!(
            value,
            json!({
                "DisplayPolicyUpdated": {
                    "hide_redacted": true
                }
            })
        );

        let debug = format!("{event:?}");
        assert!(debug.contains("DisplayPolicyUpdated"), "{debug}");
        assert!(debug.contains("hide_redacted"), "{debug}");
    }

    fn timeline_item_fixture(event_id: &str, is_redacted: bool) -> TimelineItem {
        TimelineItem {
            id: TimelineItemId::Event {
                event_id: event_id.to_owned(),
            },
            sender: Some("@alice:example.invalid".to_owned()),
            sender_label: None,
            body: if is_redacted {
                None
            } else {
                Some("visible body".to_owned())
            },
            message_kind: Default::default(),
            spoiler_spans: Vec::new(),
            timestamp_ms: Some(1),
            in_reply_to_event_id: None,
            formatted: None,
            reply_quote: None,
            thread_root: None,
            thread_summary: None,
            media: None,
            reactions: Vec::new(),
            can_react: !is_redacted,
            is_redacted,
            is_hidden: false,
            can_redact: !is_redacted,
            is_edited: false,
            can_edit: false,
            actions: TimelineMessageActions::default(),
            send_state: None,
        }
    }

    #[test]
    fn room_settings_events_project_member_display_labels_from_profile_state() {
        let mut state = AppState::default();
        state.session = SessionState::Ready(matrix_desktop_state::SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@me:example.invalid".to_owned(),
            device_id: "DEVICE".to_owned(),
        });
        state.profile.local_aliases.insert(
            "@member:example.invalid".to_owned(),
            "Local Remark".to_owned(),
        );

        let mut event = RoomEvent::RoomSettingsLoaded {
            request_id: fake_rid(70),
            settings: RoomSettingsSnapshot {
                room_id: "!room:example.invalid".to_owned(),
                name: Some("Room".to_owned()),
                topic: None,
                avatar_url: None,
                join_rule: matrix_desktop_state::RoomJoinRule::Invite,
                history_visibility: matrix_desktop_state::RoomHistoryVisibility::Shared,
                permissions: matrix_desktop_state::RoomPermissionFacts::default(),
                members: vec![matrix_desktop_state::RoomMemberSummary {
                    user_id: "@member:example.invalid".to_owned(),
                    display_name: Some("Upstream Member".to_owned()),
                    display_label: "Upstream Member".to_owned(),
                    original_display_label: "Upstream Member".to_owned(),
                    avatar_url: None,
                    power_level: Some(0),
                    role: matrix_desktop_state::RoomMemberRole::User,
                }],
            },
        };

        project_room_event_display_labels(&mut event, &state);

        let RoomEvent::RoomSettingsLoaded { settings, .. } = event else {
            panic!("expected room settings event");
        };
        assert_eq!(settings.members[0].display_label, "Local Remark");
        assert_eq!(
            settings.members[0].display_name.as_deref(),
            Some("Upstream Member")
        );
    }

    #[test]
    fn derive_display_label_updates_resolves_from_profile_state() {
        let mut state = AppState::default();
        state.profile.own.display_name = Some("My Name".to_owned());
        state.profile.local_aliases.insert(
            "@alice:example.invalid".to_owned(),
            "Alice Alias".to_owned(),
        );
        state.profile.local_aliases.insert(
            "@bob:example.invalid".to_owned(),
            "".to_owned(), // empty alias = cleared, falls through
        );
        state.profile.users.insert(
            "@carol:example.invalid".to_owned(),
            matrix_desktop_state::UserProfile {
                user_id: "@carol:example.invalid".to_owned(),
                display_name: Some("Carol Upstream".to_owned()),
                display_label: String::new(),
                original_display_label: String::new(),
                mention_search_terms: Vec::new(),
                avatar: None,
            },
        );
        // own user id for resolve_user_display_name own-user fallback
        let own_user_id = Some("@me:example.invalid");

        let updates = derive_display_label_updates(&state.profile, own_user_id);

        // Alice: alias present -> label = alias
        let alice = updates
            .iter()
            .find(|u| u.user_id == "@alice:example.invalid")
            .expect("alice in updates");
        assert_eq!(alice.display_label, "Alice Alias");

        // Bob: alias is empty -> falls through to MXID since no upstream
        let bob = updates
            .iter()
            .find(|u| u.user_id == "@bob:example.invalid")
            .expect("bob in updates");
        assert_eq!(bob.display_label, "@bob:example.invalid");

        // Carol: upstream display_name in users, no alias -> label = upstream
        let carol = updates
            .iter()
            .find(|u| u.user_id == "@carol:example.invalid")
            .expect("carol in updates");
        assert_eq!(carol.display_label, "Carol Upstream");

        // Own user is included when own display_name is set
        let me = updates
            .iter()
            .find(|u| u.user_id == "@me:example.invalid")
            .expect("own user in updates");
        assert_eq!(me.display_label, "My Name");

        let updates = derive_display_label_updates_for_user_ids(
            &state.profile,
            own_user_id,
            ["@unknown:example.invalid"].into_iter(),
        );
        let unknown = updates
            .iter()
            .find(|u| u.user_id == "@unknown:example.invalid")
            .expect("additional user id in updates");
        assert_eq!(unknown.display_label, "@unknown:example.invalid");
    }
}
