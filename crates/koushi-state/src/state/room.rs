use std::fmt;

use serde::{Deserialize, Serialize};

use super::profile::AvatarImage;
use super::settings::RoomNotificationMode;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SpaceSummary {
    pub space_id: String,
    pub display_name: String,
    #[serde(default)]
    pub avatar: Option<AvatarImage>,
    pub child_room_ids: Vec<String>,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomSummary {
    pub room_id: String,
    pub display_name: String,
    pub display_label: String,
    #[serde(default)]
    pub original_display_label: String,
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
            .field("display_label", &"DisplayLabel(..)")
            .field("original_display_label", &"OriginalDisplayLabel(..)")
            .field("avatar", &self.avatar.as_ref().map(|_| "AvatarImage(..)"))
            .field("is_dm", &self.is_dm)
            .field("dm_user_ids", &self.dm_user_ids.len())
            .field("tags", &self.tags)
            .field("unread_count", &self.unread_count)
            .field("notification_count", &self.notification_count)
            .field("highlight_count", &self.highlight_count)
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
    #[serde(default)]
    pub sender_id: Option<String>,
    #[serde(default)]
    pub sender_label: Option<String>,
    #[serde(default)]
    pub sender_avatar: Option<AvatarImage>,
    #[serde(default)]
    pub preview: Option<String>,
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
    #[serde(default)]
    pub avatar: Option<AvatarImage>,
    pub topic: Option<String>,
    pub inviter_display_name: Option<String>,
    #[serde(default)]
    pub inviter_user_id: Option<String>,
    pub is_dm: bool,
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

pub fn room_activity_unread_count(room: &RoomSummary) -> u64 {
    let count = room
        .unread_count
        .max(room.notification_count)
        .max(room.highlight_count);
    if count > 0 {
        count
    } else if room.marked_unread {
        1
    } else {
        0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RoomAttentionProjection {
    pub unread_count: u64,
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
    let has_unread_content = room.unread_count > 0
        || room.notification_count > 0
        || room.highlight_count > 0
        || room.marked_unread;
    let unread_count = room_activity_unread_count(room);
    let highlight_count = if is_muted { 0 } else { room.highlight_count };
    let has_unread_mention = !is_muted && room.highlight_count > 0;
    let is_attention_highlighted = !is_muted
        && (room.notification_count > 0 || room.highlight_count > 0 || room.marked_unread);
    let notification_count = if is_muted
        || (mode == Some(RoomNotificationMode::Mentions) && room.highlight_count == 0)
    {
        0
    } else {
        room.notification_count
    };
    let display_count = if is_muted {
        room.unread_count
    } else {
        notification_count
    };

    RoomAttentionProjection {
        unread_count,
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

    if projection.has_unread_mention {
        0
    } else if projection.notification_count > 0 {
        1
    } else if projection.has_unread_content {
        2
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
            sender_id: Some("@private-sender:example.invalid".to_owned()),
            sender_label: Some("Private Sender".to_owned()),
            sender_avatar: None,
            preview: Some("private body".to_owned()),
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
        ] {
            assert!(!debug.contains(private_value));
        }
    }
}
