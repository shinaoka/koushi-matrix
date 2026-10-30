use std::fmt;

use serde::{Deserialize, Serialize};

use super::profile::AvatarImage;
use super::settings::RoomNotificationMode;

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct SpaceSummary {
    pub space_id: String,
    /// The Space's canonical `m.room.name`. `None` means the Space has no
    /// name event: an alias or a computed name is never promoted into it,
    /// and snapshots written before this field existed load as `None`.
    #[serde(default)]
    pub raw_name: Option<String>,
    pub display_name: String,
    #[serde(default)]
    pub avatar: Option<AvatarImage>,
    /// The Space's join rule as last synced. `None` until a room-list update
    /// has projected it (snapshots written before this field existed load as
    /// `None`); readers show it as not yet known, never as a guessed rule.
    #[serde(default)]
    pub join_rule: Option<super::room_management::RoomJoinRule>,
    /// The Space's confirmed description (`m.room.topic`) as last synced
    /// (#1661): the read-only value every surface outside Space info renders.
    /// `None` means no description, never an empty string.
    #[serde(default)]
    pub topic: Option<String>,
    pub child_room_ids: Vec<String>,
    /// The Space's own parent-side `m.space.child` children: what other
    /// clients list for this Space. `child_room_ids` is the display union of
    /// this list with rooms that only claim the Space through `m.space.parent`
    /// (the sidebar shows a child as soon as one side of the relationship has
    /// synced); add-existing eligibility (#1007) must use this parent-side
    /// list, so a parent-only room stays eligible.
    #[serde(default)]
    pub parent_side_child_room_ids: Vec<String>,
}

impl fmt::Debug for SpaceSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SpaceSummary")
            .field("space_id", &"RoomId(..)")
            .field("raw_name", &self.raw_name.as_ref().map(|_| "RoomName(..)"))
            .field("display_name", &"RoomName(..)")
            .field("avatar", &self.avatar.as_ref().map(|_| "AvatarImage(..)"))
            .field("join_rule", &self.join_rule)
            .field("topic", &self.topic.as_ref().map(|_| "RoomTopic(..)"))
            .field("child_room_ids", &self.child_room_ids.len())
            .field(
                "parent_side_child_room_ids",
                &self.parent_side_child_room_ids.len(),
            )
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomSummary {
    pub room_id: String,
    pub display_name: String,
    /// Set when `display_name` is the SDK's calculated empty-room name
    /// (#1050); `display_name` keeps the SDK's English text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name_placeholder: Option<RoomNamePlaceholder>,
    pub display_label: String,
    #[serde(default)]
    pub original_display_label: String,
    /// Set when `display_label` is that placeholder (#1050). `display_label`
    /// then still carries the SDK's English text as caller data, and the GUI
    /// renders this structured value through the message catalog instead. A
    /// single-member DM never uses it: its label resolves to the member.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_label_placeholder: Option<RoomNamePlaceholder>,
    #[serde(default)]
    pub avatar: Option<AvatarImage>,
    pub is_dm: bool,
    #[serde(default)]
    pub dm_user_ids: Vec<String>,
    #[serde(default)]
    pub tags: RoomTags,
    pub unread_count: u64,
    pub notification_count: u64,
    pub highlight_count: u64,
    /// #1238: the room's summed per-root SDK thread-cache unread replies.
    ///
    /// This is the thread half of the proven client-side decomposition
    /// (`RoomReadReceiptEventFilter` excludes thread replies from
    /// `unread_count`, each `ThreadReadReceiptEventFilter` counts one thread);
    /// it is deliberately a separate field from the main-only `unread_count`,
    /// which stays the navigation value for read markers and first-unread.
    #[serde(default)]
    pub thread_unread_count: u64,
    /// #1238: the room's summed per-root SDK thread-cache unread mentions, so a
    /// mention inside a thread renders mention styling instead of a plain count.
    #[serde(default)]
    pub thread_highlight_count: u64,
    #[serde(default)]
    pub marked_unread: bool,
    #[serde(default)]
    pub recency_stamp: Option<u64>,
    #[serde(default)]
    pub conversation_activity: Option<ConversationActivity>,
    #[serde(default)]
    pub latest_event: Option<RoomLatestEventSummary>,
    pub parent_space_ids: Vec<String>,
    #[serde(default)]
    pub dm_space_ids: Vec<String>,
    #[serde(default)]
    pub is_encrypted: bool,
    #[serde(default)]
    pub joined_members: u64,
}

impl fmt::Debug for RoomSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RoomSummary")
            .field("room_id", &"RoomId(..)")
            .field("display_name", &"RoomName(..)")
            .field("display_name_placeholder", &self.display_name_placeholder)
            .field("display_label", &"DisplayLabel(..)")
            .field("original_display_label", &"OriginalDisplayLabel(..)")
            .field("display_label_placeholder", &self.display_label_placeholder)
            .field("avatar", &self.avatar.as_ref().map(|_| "AvatarImage(..)"))
            .field("is_dm", &self.is_dm)
            .field("dm_user_ids", &self.dm_user_ids.len())
            .field("tags", &self.tags)
            .field("unread_count", &self.unread_count)
            .field("notification_count", &self.notification_count)
            .field("highlight_count", &self.highlight_count)
            .field("thread_unread_count", &self.thread_unread_count)
            .field("thread_highlight_count", &self.thread_highlight_count)
            .field("marked_unread", &self.marked_unread)
            .field("has_recency_stamp", &self.recency_stamp.is_some())
            .field("conversation_activity", &self.conversation_activity)
            .field(
                "latest_event",
                &self.latest_event.as_ref().map(|_| "LatestEvent(..)"),
            )
            .field("parent_space_ids", &self.parent_space_ids.len())
            .field("dm_space_ids", &self.dm_space_ids.len())
            .field("is_encrypted", &self.is_encrypted)
            .field("joined_members", &self.joined_members)
            .finish()
    }
}

/// The SDK's calculated room name when no name, alias, or other member exists
/// (the Matrix "Empty Room" rule, `matrix_sdk_base::RoomDisplayName::Empty` and
/// `EmptyWas`). Product text for it is owned by the GUI message catalog.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RoomNamePlaceholder {
    /// No useful name could ever be calculated.
    Empty,
    /// The room is empty now; `previous_names` is the SDK's former member
    /// name list, caller-owned data rendered with `dir="auto"`.
    EmptyWas { previous_names: String },
}

impl fmt::Debug for RoomNamePlaceholder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("Empty"),
            Self::EmptyWas { .. } => formatter
                .debug_struct("EmptyWas")
                .field("previous_names", &"DisplayNames(..)")
                .finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConversationActivitySource {
    Message,
    EncryptedMessage,
    ThreadReply,
}

#[derive(Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConversationActivity {
    pub timestamp_ms: u64,
    pub source: ConversationActivitySource,
}

impl fmt::Debug for ConversationActivity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConversationActivity")
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

impl RoomSummary {
    pub(crate) fn compare_attention_activity(
        left: Option<&Self>,
        left_mode: Option<RoomNotificationMode>,
        right: Option<&Self>,
        right_mode: Option<RoomNotificationMode>,
    ) -> std::cmp::Ordering {
        room_attention_sort_rank(left, left_mode)
            .cmp(&room_attention_sort_rank(right, right_mode))
            .then_with(|| compare_conversation_activity(left, right))
    }
}

pub(crate) fn compare_conversation_activity(
    left: Option<&RoomSummary>,
    right: Option<&RoomSummary>,
) -> std::cmp::Ordering {
    let left_activity = left.and_then(|room| room.conversation_activity);
    let right_activity = right.and_then(|room| room.conversation_activity);
    right_activity
        .is_some()
        .cmp(&left_activity.is_some())
        .then_with(|| {
            right_activity
                .map(|activity| activity.timestamp_ms)
                .cmp(&left_activity.map(|activity| activity.timestamp_ms))
        })
        // #1364: a room whose newest event is not a conversation event (sticker,
        // poll, call, reaction, state change) has no conversation timestamp to
        // compare, and the room-list Event Cache window can be a single event
        // deep. Keep the server's own room order inside that group instead of
        // collapsing every activity-less room into case-folded label order. The
        // recency stamp is consumed only here, below every room that has real
        // conversation activity, so a newly joined room cannot outrank an
        // existing conversation (state-machine.md, Sidebar Sections And Low
        // Priority).
        .then_with(|| {
            let left_stamp = left.and_then(|room| room.recency_stamp);
            let right_stamp = right.and_then(|room| room.recency_stamp);
            right_stamp
                .is_some()
                .cmp(&left_stamp.is_some())
                .then_with(|| right_stamp.cmp(&left_stamp))
        })
        .then_with(|| {
            let left_label = left
                .map(|room| room.display_label.to_lowercase())
                .unwrap_or_default();
            let right_label = right
                .map(|room| room.display_label.to_lowercase())
                .unwrap_or_default();
            left_label.cmp(&right_label)
        })
        .then_with(|| {
            left.map(|room| room.room_id.as_str())
                .unwrap_or_default()
                .cmp(right.map(|room| room.room_id.as_str()).unwrap_or_default())
        })
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomLatestEventSummary {
    pub event_id: String,
    #[serde(default)]
    pub relation_type: Option<String>,
    #[serde(default)]
    pub relation_event_id: Option<String>,
    /// Thread root of the latest event, when it is a thread reply. Carried so
    /// Activity rows built from the room-summary fallback can route to the
    /// Thread panel before the room timeline has been reconciled (#965).
    #[serde(default)]
    pub thread_root_event_id: Option<String>,
    #[serde(default)]
    pub sender_id: Option<String>,
    #[serde(default)]
    pub sender_label: Option<String>,
    #[serde(default)]
    pub sender_avatar: Option<AvatarImage>,
    #[serde(default)]
    pub preview: Option<String>,
    /// #1427: the decrypted body of the latest conversation event when it is a plain
    /// text message this device can already read, for the device-local delivery
    /// prefix. It is the latest event's body, which may be newer than the unread
    /// event that made the room qualify.
    ///
    /// Never serialized. The room-list `preview` above is part of the sidebar row and
    /// is already in the app-state snapshot; this field is the prefix source the
    /// delivery channel adds, and it must not enter a snapshot, a state delta, an
    /// event, or a diagnostic. It is `None` for a redacted, undecryptable, or
    /// non-text event, so a room with nothing readable delivers without a prefix.
    #[serde(skip)]
    pub message_text: Option<String>,
    pub timestamp_ms: u64,
    #[serde(default)]
    pub is_redacted: bool,
}

impl fmt::Debug for RoomLatestEventSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let relation_kind = match self.relation_type.as_deref() {
            Some("m.replace") => Some("replace"),
            Some("m.annotation") => Some("annotation"),
            Some(_) => Some("other"),
            None => None,
        };
        formatter
            .debug_struct("RoomLatestEventSummary")
            .field("event_id", &"EventId(..)")
            .field("relation_type", &relation_kind)
            .field(
                "relation_event_id",
                &self.relation_event_id.as_ref().map(|_| "EventId(..)"),
            )
            .field(
                "thread_root_event_id",
                &self.thread_root_event_id.as_ref().map(|_| "EventId(..)"),
            )
            .field("sender_id", &self.sender_id.as_ref().map(|_| "UserId(..)"))
            .field(
                "sender_label",
                &self.sender_label.as_ref().map(|_| "SenderLabel(..)"),
            )
            .field(
                "sender_avatar",
                &self.sender_avatar.as_ref().map(|_| "AvatarImage(..)"),
            )
            .field("preview", &self.preview.as_ref().map(|_| "Preview(..)"))
            .field(
                "message_text",
                &self.message_text.as_ref().map(|_| "MessageText(..)"),
            )
            .field("timestamp_ms", &self.timestamp_ms)
            .field("is_redacted", &self.is_redacted)
            .finish()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomTags {
    pub favourite: Option<RoomTagInfo>,
    pub low_priority: Option<RoomTagInfo>,
}

impl RoomTags {
    pub fn set(&mut self, tag: RoomTagKind, info: RoomTagInfo) {
        match tag {
            RoomTagKind::Favourite => {
                self.favourite = Some(info);
                self.low_priority = None;
            }
            RoomTagKind::LowPriority => {
                self.low_priority = Some(info);
                self.favourite = None;
            }
        }
    }

    pub fn remove(&mut self, tag: RoomTagKind) {
        match tag {
            RoomTagKind::Favourite => self.favourite = None,
            RoomTagKind::LowPriority => self.low_priority = None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomTagInfo {
    pub order: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomTagKind {
    Favourite,
    LowPriority,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InvitePreview {
    pub room_id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name_placeholder: Option<RoomNamePlaceholder>,
    #[serde(default)]
    pub avatar: Option<AvatarImage>,
    pub topic: Option<String>,
    pub inviter_display_name: Option<String>,
    #[serde(default)]
    pub inviter_user_id: Option<String>,
    pub is_dm: bool,
    #[serde(default)]
    pub is_space: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoomAttentionKind {
    Mention,
    Dm,
    Message,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomAttentionSummary {
    pub room_display_name: String,
    pub kind: RoomAttentionKind,
    pub notification_count: u64,
    pub highlight_count: u64,
    pub unread_count: u64,
}

pub fn room_attention_kind(
    is_dm: bool,
    notification_count: u64,
    highlight_count: u64,
    unread_count: u64,
) -> Option<RoomAttentionKind> {
    if highlight_count > 0 {
        return Some(RoomAttentionKind::Mention);
    }

    if notification_count == 0 && unread_count == 0 {
        return None;
    }

    if is_dm {
        Some(RoomAttentionKind::Dm)
    } else {
        Some(RoomAttentionKind::Message)
    }
}

/// The room activity a successful local mark-read cleared (#1683): the
/// room-summary latest event and last-activity timestamps at that moment. A later
/// unread room-list snapshot is stale only while it still shows this activity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoomReadActivity {
    latest_event_id: Option<String>,
    recency_stamp: Option<u64>,
    conversation_activity: Option<ConversationActivity>,
}

impl RoomReadActivity {
    pub fn of(room: &RoomSummary) -> Self {
        Self {
            latest_event_id: room
                .latest_event
                .as_ref()
                .map(|event| event.event_id.clone()),
            recency_stamp: room.recency_stamp,
            conversation_activity: room.conversation_activity,
        }
    }
}

/// Persistent numeric unread shared by room/sidebar, account and native badges.
///
/// #1238: the badge is the only place the main-timeline and thread counts meet.
/// Thread replies are excluded from `unread_count` by the SDK's room read-receipt
/// filter and counted per thread by the thread filters, so
/// `unread_count + thread_unread_count` is the non-overlapping client
/// decomposition. A homeserver's `notification_count` has no event IDs and
/// cannot make a message locatable, so it cannot add to a numeric unread badge.
/// `unread_count` itself stays main-only for navigation.
pub fn room_activity_unread_count(room: &RoomSummary) -> u64 {
    room.unread_count.saturating_add(room.thread_unread_count)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RoomAttentionProjection {
    pub unread_count: u64,
    pub main_unread_count: u64,
    pub thread_unread_count: u64,
    pub notification_count: u64,
    pub highlight_count: u64,
    pub has_unread_content: bool,
    pub is_attention_highlighted: bool,
    pub has_unread_mention: bool,
    pub is_muted: bool,
    pub display_count: u64,
}

pub fn room_attention_projection(
    room: &RoomSummary,
    mode: Option<RoomNotificationMode>,
) -> RoomAttentionProjection {
    let is_muted = mode == Some(RoomNotificationMode::Mute);
    let thread_highlight_count = room.thread_highlight_count;
    let highlight_total = room.highlight_count.saturating_add(thread_highlight_count);
    let has_unread_content = room_activity_unread_count(room) > 0 || room.marked_unread;
    let unread_count = room_activity_unread_count(room);
    let highlight_count = if is_muted { 0 } else { highlight_total };
    let has_unread_mention = !is_muted && highlight_total > 0;
    let is_attention_highlighted =
        !is_muted && (room.notification_count > 0 || highlight_total > 0 || room.marked_unread);
    let notification_count =
        if is_muted || (mode == Some(RoomNotificationMode::Mentions) && highlight_total == 0) {
            0
        } else {
            room.notification_count
        };
    let display_count = unread_count;

    RoomAttentionProjection {
        unread_count,
        main_unread_count: room.unread_count,
        thread_unread_count: room.thread_unread_count,
        notification_count,
        highlight_count,
        has_unread_content,
        is_attention_highlighted,
        has_unread_mention,
        is_muted,
        display_count,
    }
}

fn room_attention_sort_rank(room: Option<&RoomSummary>, mode: Option<RoomNotificationMode>) -> u8 {
    let Some(room) = room else {
        return 3;
    };
    let projection = room_attention_projection(room, mode);

    // Server notification/highlight counters cannot prove a visible unread
    // message. They must not promote a room that has no SDK-local unread.
    if !projection.is_muted && projection.unread_count > 0 {
        0
    } else if !projection.is_muted && room.marked_unread {
        1
    } else {
        3
    }
}

pub fn room_attention_summary(
    room_display_name: String,
    is_dm: bool,
    notification_count: u64,
    highlight_count: u64,
    unread_count: u64,
) -> Option<RoomAttentionSummary> {
    let kind = room_attention_kind(is_dm, notification_count, highlight_count, unread_count)?;

    Some(RoomAttentionSummary {
        room_display_name: private_safe_room_display_name(room_display_name),
        kind,
        notification_count,
        highlight_count,
        unread_count,
    })
}

fn private_safe_room_display_name(room_display_name: String) -> String {
    if room_display_name.trim().is_empty() {
        "Room".to_owned()
    } else {
        room_display_name
    }
}

#[cfg(test)]
mod tests {
    use super::RoomLatestEventSummary;
    use serde_json::json;

    #[test]
    fn room_latest_event_summary_defaults_redaction_and_keeps_debug_private() {
        let restored: RoomLatestEventSummary = serde_json::from_value(json!({
            "event_id": "$event:example.invalid",
            "timestamp_ms": 42,
        }))
        .expect("legacy room latest summary");
        assert!(!restored.is_redacted);

        let summary = RoomLatestEventSummary {
            event_id: "$private-event:example.invalid".to_owned(),
            relation_type: None,
            relation_event_id: None,
            thread_root_event_id: None,
            sender_id: Some("@private-sender:example.invalid".to_owned()),
            sender_label: Some("Private Sender".to_owned()),
            sender_avatar: None,
            preview: Some("private body".to_owned()),
            message_text: Some("private message text".to_owned()),
            timestamp_ms: 42,
            is_redacted: true,
        };
        let debug = format!("{summary:?}");

        assert!(debug.contains("is_redacted"));
        assert!(debug.contains("true"));
        for private_value in [
            "$private-event:example.invalid",
            "@private-sender:example.invalid",
            "Private Sender",
            "private body",
            "private message text",
        ] {
            assert!(!debug.contains(private_value));
        }
    }

    /// #1427: the delivery prefix source is a fact this device holds about a room and
    /// never a field of the app state. The room-list `preview` is serialized for the
    /// sidebar row and stays that way; the text this feature adds must not join it.
    #[test]
    fn the_delivery_prefix_source_never_serializes_into_the_app_state() {
        let mut state = koushi_state_app_state_with_room_latest_text("private delivery text");
        let serialized = serde_json::to_string(&state).expect("the app state serializes");
        assert!(
            !serialized.contains("private delivery text"),
            "the prefix source reached the app-state snapshot"
        );

        // The same state does carry the room-list preview, which is pre-existing
        // sidebar behavior and is not what this test protects.
        assert!(serialized.contains("private preview"));
        // Dropping the room also drops the text, so the negative assertion above is
        // about the field and not about an empty room list.
        state.rooms.clear();
        assert!(
            !serde_json::to_string(&state)
                .expect("the app state serializes")
                .contains("private delivery text")
        );
    }

    fn koushi_state_app_state_with_room_latest_text(text: &str) -> crate::AppState {
        let mut room: super::RoomSummary = serde_json::from_value(json!({
            "room_id": "!room:example.invalid",
            "display_name": "Room",
            "display_label": "Room",
            "is_dm": false,
            "unread_count": 1,
            "notification_count": 1,
            "highlight_count": 0,
            "parent_space_ids": [],
            "latest_event": {
                "event_id": "$event:example.invalid",
                "preview": "private preview",
                "timestamp_ms": 42,
            },
        }))
        .expect("a room summary with a latest event");
        let latest_event = room.latest_event.as_mut().expect("the latest event");
        // `message_text` is skipped on deserialize, like every other field the app
        // state must not carry, so it is set here where the test can hold it.
        assert!(latest_event.message_text.is_none());
        latest_event.message_text = Some(text.to_owned());

        crate::AppState {
            rooms: vec![room],
            ..crate::AppState::default()
        }
    }
}
