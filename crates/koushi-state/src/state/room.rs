use std::fmt;

use serde::{Deserialize, Serialize};

use super::profile::AvatarImage;

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
    pub last_activity_ms: u64,
    pub parent_space_ids: Vec<String>,
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
            .field("last_activity_ms", &self.last_activity_ms)
            .field("parent_space_ids", &self.parent_space_ids.len())
            .field("is_encrypted", &self.is_encrypted)
            .field("joined_members", &self.joined_members)
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
