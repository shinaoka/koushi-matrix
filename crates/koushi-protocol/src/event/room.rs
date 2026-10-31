use std::fmt;

use super::{ReportKind, RoomKeyRequestStage, RoomKeyRequestWithheldCode};
use crate::ids::{RequestId, TimelineKey};
use koushi_state::{
    DirectoryQuery, DirectoryRoomPreview, DirectoryRoomSummary, InviteDestinationResult,
    PinnedEvent, RoomModerationAction, RoomSettingsSnapshot, RoomTagKind, SpaceMemberInviteOutcome,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum RoomEvent {
    HistoryKeysRequested {
        request_id: RequestId,
        room_id: String,
        recipients: u16,
    },
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
    /// Issue #961: the Space's advertised children settled. Only a count
    /// travels: room identities belong to the state snapshot.
    SpaceChildrenLoaded {
        request_id: RequestId,
        generation: u64,
        child_count: usize,
    },
    SpaceMembersLoaded {
        request_id: RequestId,
        generation: u64,
        joined_count: usize,
        invited_count: usize,
        child_room_only_count: usize,
        incomplete_child_room_count: usize,
    },
    SpaceMemberInviteSettled {
        request_id: RequestId,
        space_id: String,
        user_id: String,
        generation: u64,
        outcome: SpaceMemberInviteOutcome,
    },
    SpaceMemberInviteCancellationSettled {
        request_id: RequestId,
        space_id: String,
        user_id: String,
        generation: u64,
        outcome: SpaceMemberInviteOutcome,
    },
    InviteBatchCompleted {
        request_id: RequestId,
        room_id: String,
        results: Vec<InviteDestinationResult>,
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
    /// #1494: `StartDirectMessage { missing: Report }` found no reusable
    /// direct chat and created nothing.
    DirectMessageMissing {
        request_id: RequestId,
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
        request_id: Option<RequestId>,
        room_id: String,
        pinned: Vec<PinnedEvent>,
    },
    PinEventCompleted {
        request_id: RequestId,
        room_id: String,
        event_id: String,
    },
    UnpinEventCompleted {
        request_id: RequestId,
        room_id: String,
        event_id: String,
    },
    DirectoryQueryCompleted {
        request_id: RequestId,
        query: DirectoryQuery,
        rooms: Vec<DirectoryRoomSummary>,
        next_batch: Option<String>,
    },
    DirectoryPreviewLoaded {
        request_id: RequestId,
        room: DirectoryRoomPreview,
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
    SpaceMemberRoleUpdateSettled {
        request_id: RequestId,
        space_id: String,
        user_id: String,
        generation: u64,
        outcome: koushi_state::SpaceMemberRoleUpdateOutcome,
    },
    RoomKeyRequestStateChanged {
        key: TimelineKey,
        event_id: String,
        request_id: Option<RequestId>,
        stage: RoomKeyRequestStage,
        withheld_code: Option<RoomKeyRequestWithheldCode>,
    },
    /// Issue #450: a recognized-but-unavailable slash command (/join, /invite)
    /// was rejected for a composer target; the key names which composer should
    /// show the localized notice and the request id lets the Tauri submission
    /// waiter settle immediately instead of waiting out its timeout.
    ComposerSlashCommandRejected {
        key: TimelineKey,
        request_id: RequestId,
    },
    MarkedAsRead {
        request_id: RequestId,
        room_id: String,
    },
    MarkedAsUnread {
        request_id: RequestId,
        room_id: String,
        unread: bool,
    },
    OutboundSessionRotationForced {
        request_id: RequestId,
        room_id: String,
    },
    RoomListUpdated,
    ReportCompleted {
        request_id: RequestId,
        kind: ReportKind,
    },
}
impl fmt::Debug for RoomEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HistoryKeysRequested {
                request_id,
                recipients,
                ..
            } => formatter
                .debug_struct("HistoryKeysRequested")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("recipients", recipients)
                .finish(),
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
            Self::SpaceChildrenLoaded {
                request_id,
                generation,
                child_count,
            } => formatter
                .debug_struct("SpaceChildrenLoaded")
                .field("request_id", request_id)
                .field("generation", generation)
                .field("child_count", child_count)
                .finish(),
            Self::SpaceMembersLoaded {
                request_id,
                generation,
                joined_count,
                invited_count,
                child_room_only_count,
                incomplete_child_room_count,
            } => formatter
                .debug_struct("SpaceMembersLoaded")
                .field("request_id", request_id)
                .field("generation", generation)
                .field("joined_count", joined_count)
                .field("invited_count", invited_count)
                .field("child_room_only_count", child_room_only_count)
                .field("incomplete_child_room_count", incomplete_child_room_count)
                .finish(),
            Self::SpaceMemberInviteSettled {
                request_id,
                generation,
                outcome,
                ..
            } => formatter
                .debug_struct("SpaceMemberInviteSettled")
                .field("request_id", request_id)
                .field("generation", generation)
                .field("outcome", outcome)
                .finish(),
            Self::SpaceMemberInviteCancellationSettled {
                request_id,
                generation,
                outcome,
                ..
            } => formatter
                .debug_struct("SpaceMemberInviteCancellationSettled")
                .field("request_id", request_id)
                .field("generation", generation)
                .field("outcome", outcome)
                .finish(),
            Self::InviteBatchCompleted {
                request_id,
                results,
                ..
            } => formatter
                .debug_struct("InviteBatchCompleted")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("result_count", &results.len())
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
            Self::DirectMessageMissing { request_id } => formatter
                .debug_struct("DirectMessageMissing")
                .field("request_id", request_id)
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
            Self::DirectoryPreviewLoaded { request_id, room } => formatter
                .debug_struct("DirectoryPreviewLoaded")
                .field("request_id", request_id)
                .field("room", room)
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
            Self::SpaceMemberRoleUpdateSettled {
                request_id,
                generation,
                outcome,
                ..
            } => formatter
                .debug_struct("SpaceMemberRoleUpdateSettled")
                .field("request_id", request_id)
                .field("generation", generation)
                .field("outcome", outcome)
                .finish(),
            Self::RoomKeyRequestStateChanged {
                key: _,
                event_id: _,
                request_id: _,
                stage,
                withheld_code,
            } => formatter
                .debug_struct("RoomKeyRequestStateChanged")
                .field("key", &"TimelineKey(..)")
                .field("event_id", &"EventId(..)")
                .field("stage", stage)
                .field("withheld_code", withheld_code)
                .finish(),
            Self::ComposerSlashCommandRejected { key: _, request_id } => formatter
                .debug_struct("ComposerSlashCommandRejected")
                .field("key", &"TimelineKey(..)")
                .field("request_id", request_id)
                .finish(),
            Self::MarkedAsRead { request_id, .. } => formatter
                .debug_struct("MarkedAsRead")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::MarkedAsUnread {
                request_id,
                room_id,
                unread,
            } => formatter
                .debug_struct("MarkedAsUnread")
                .field("request_id", request_id)
                .field("room_id", room_id)
                .field("unread", unread)
                .finish(),
            Self::OutboundSessionRotationForced { request_id, .. } => formatter
                .debug_struct("OutboundSessionRotationForced")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::RoomListUpdated => formatter.write_str("RoomListUpdated"),
            Self::ReportCompleted { request_id, kind } => formatter
                .debug_struct("ReportCompleted")
                .field("request_id", request_id)
                .field("kind", kind)
                .finish(),
        }
    }
}

/// One page of a room's recent timeline for a caller outside the GUI (#1530).
///
/// The page is read straight from the account's session, so it works for a room
/// that is not open and never becomes an app state update.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomMessagesPage {
    pub messages: Vec<RoomMessageItem>,
    /// Opaque token for the next older page. Absent at the start of history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

/// One message-like event from a room's timeline.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomMessageItem {
    pub event_id: String,
    pub sender_id: String,
    pub timestamp_ms: u64,
    /// The message text, when this device could read it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// True exactly when `body` is present, as in a direct message read.
    pub content_available: bool,
    #[serde(default)]
    pub kind: RoomMessageKind,
    /// The event arrived encrypted, whichever way it was decrypted.
    #[serde(default)]
    pub is_encrypted: bool,
    #[serde(default)]
    pub is_redacted: bool,
}

impl fmt::Debug for RoomMessageItem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RoomMessageItem")
            .field("event_id", &"EventId(..)")
            .field("sender_id", &"UserId(..)")
            .field("timestamp_ms", &self.timestamp_ms)
            .field("has_body", &self.body.is_some())
            .field("kind", &self.kind)
            .field("is_encrypted", &self.is_encrypted)
            .field("is_redacted", &self.is_redacted)
            .finish()
    }
}

/// The message type of one timeline event, reduced to what a reader needs.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomMessageKind {
    #[default]
    Other,
    Text,
    Emote,
    Notice,
    Image,
    Video,
    Audio,
    File,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forced_rotation_debug_redacts_room_id() {
        let event = RoomEvent::OutboundSessionRotationForced {
            request_id: RequestId {
                connection_id: crate::RuntimeConnectionId(1),
                sequence: 2,
            },
            room_id: "!private-room:example.invalid".to_owned(),
        };
        let debug = format!("{event:?}");
        assert!(debug.contains("RoomId(..)"), "{debug}");
        assert!(!debug.contains("!private-room:example.invalid"), "{debug}");
    }
}
