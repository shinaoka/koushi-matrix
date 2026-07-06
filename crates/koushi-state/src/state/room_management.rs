use std::fmt;

use serde::{Deserialize, Serialize};

use super::errors::OperationFailureKind;

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomManagementState {
    pub selected_room_id: Option<String>,
    pub settings: Option<RoomSettingsSnapshot>,
    pub operation: RoomManagementOperationState,
}

impl fmt::Debug for RoomManagementState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RoomManagementState")
            .field(
                "selected_room_id",
                &self.selected_room_id.as_ref().map(|_| "RoomId(..)"),
            )
            .field(
                "settings",
                &self.settings.as_ref().map(|_| "RoomSettingsSnapshot(..)"),
            )
            .field("operation", &self.operation)
            .finish()
    }
}

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RoomManagementOperationState {
    #[default]
    Idle,
    Pending {
        request_id: u64,
        room_id: String,
        operation: RoomManagementOperationKind,
    },
    Failed {
        request_id: u64,
        room_id: String,
        operation: RoomManagementOperationKind,
        #[serde(rename = "failureKind")]
        kind: OperationFailureKind,
    },
}

impl fmt::Debug for RoomManagementOperationState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => formatter.write_str("Idle"),
            Self::Pending {
                request_id,
                operation,
                ..
            } => formatter
                .debug_struct("Pending")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("operation", operation)
                .finish(),
            Self::Failed {
                request_id,
                operation,
                kind,
                ..
            } => formatter
                .debug_struct("Failed")
                .field("request_id", request_id)
                .field("room_id", &"RoomId(..)")
                .field("operation", operation)
                .field("kind", kind)
                .finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomManagementOperationKind {
    Settings,
    Moderation,
    Roles,
    Permissions,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomSettingsSnapshot {
    pub room_id: String,
    pub name: Option<String>,
    pub topic: Option<String>,
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub canonical_alias: Option<String>,
    #[serde(default)]
    pub alternate_aliases: Vec<String>,
    #[serde(default)]
    pub share_link: Option<String>,
    pub join_rule: RoomJoinRule,
    pub history_visibility: RoomHistoryVisibility,
    pub permissions: RoomPermissionFacts,
    pub members: Vec<RoomMemberSummary>,
}

impl fmt::Debug for RoomSettingsSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RoomSettingsSnapshot")
            .field("room_id", &"RoomId(..)")
            .field("name", &self.name.as_ref().map(|_| "RoomName(..)"))
            .field("topic", &self.topic.as_ref().map(|_| "RoomTopic(..)"))
            .field(
                "avatar_url",
                &self.avatar_url.as_ref().map(|_| "MxcUri(..)"),
            )
            .field(
                "canonical_alias",
                &self.canonical_alias.as_ref().map(|_| "RoomAlias(..)"),
            )
            .field("alternate_aliases", &self.alternate_aliases.len())
            .field(
                "share_link",
                &self.share_link.as_ref().map(|_| "MatrixToLink(..)"),
            )
            .field("join_rule", &self.join_rule)
            .field("history_visibility", &self.history_visibility)
            .field("permissions", &self.permissions)
            .field("members", &self.members.len())
            .finish()
    }
}

pub fn room_settings_share_link(
    room_id: &str,
    canonical_alias: Option<&str>,
    alternate_aliases: &[String],
) -> Option<String> {
    canonical_alias
        .and_then(non_empty_trimmed)
        .or_else(|| {
            alternate_aliases
                .iter()
                .find_map(|alias| non_empty_trimmed(alias))
        })
        .or_else(|| non_empty_trimmed(room_id))
        .map(|identifier| {
            format!(
                "https://matrix.to/#/{}",
                percent_encode_matrix_to_component(identifier)
            )
        })
}

fn non_empty_trimmed(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
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
        _ => unreachable!("hex digit nibble is <= 15"),
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomMemberSummary {
    pub user_id: String,
    pub display_name: Option<String>,
    pub display_label: String,
    #[serde(default)]
    pub original_display_label: String,
    pub avatar_url: Option<String>,
    pub power_level: Option<i64>,
    pub role: RoomMemberRole,
    #[serde(default)]
    pub user_trust: Option<UserTrustState>,
}

impl fmt::Debug for RoomMemberSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RoomMemberSummary")
            .field("user_id", &"UserId(..)")
            .field(
                "display_name",
                &self.display_name.as_ref().map(|_| "DisplayName(..)"),
            )
            .field("display_label", &"DisplayLabel(..)")
            .field("original_display_label", &"OriginalDisplayLabel(..)")
            .field(
                "avatar_url",
                &self.avatar_url.as_ref().map(|_| "MxcUri(..)"),
            )
            .field("power_level", &self.power_level)
            .field("role", &self.role)
            .field("user_trust", &self.user_trust)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UserTrustState {
    Unverified,
    Verified,
    IdentityReset,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomMemberRole {
    Creator,
    Administrator,
    Moderator,
    User,
}

impl RoomMemberRole {
    pub fn from_power_level(power_level: Option<i64>) -> Self {
        match power_level {
            None => Self::Creator,
            Some(level) if level >= 100 => Self::Administrator,
            Some(level) if level >= 50 => Self::Moderator,
            Some(_) => Self::User,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomJoinRule {
    Public,
    Invite,
    Knock,
    Restricted,
    Private,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomHistoryVisibility {
    WorldReadable,
    Shared,
    Invited,
    Joined,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomPermissionFacts {
    pub can_edit_settings: bool,
    pub can_edit_roles: bool,
    pub can_kick: bool,
    pub can_ban: bool,
    pub can_unban: bool,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomSettingChange {
    Name(Option<String>),
    Topic(Option<String>),
    AvatarUrl(Option<String>),
    JoinRule(RoomJoinRule),
    HistoryVisibility(RoomHistoryVisibility),
}

impl fmt::Debug for RoomSettingChange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name(value) => formatter
                .debug_tuple("Name")
                .field(&value.as_ref().map(|_| "RoomName(..)"))
                .finish(),
            Self::Topic(value) => formatter
                .debug_tuple("Topic")
                .field(&value.as_ref().map(|_| "RoomTopic(..)"))
                .finish(),
            Self::AvatarUrl(value) => formatter
                .debug_tuple("AvatarUrl")
                .field(&value.as_ref().map(|_| "MxcUri(..)"))
                .finish(),
            Self::JoinRule(rule) => formatter.debug_tuple("JoinRule").field(rule).finish(),
            Self::HistoryVisibility(visibility) => formatter
                .debug_tuple("HistoryVisibility")
                .field(visibility)
                .finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomModerationAction {
    Kick,
    Ban,
    Unban,
}
