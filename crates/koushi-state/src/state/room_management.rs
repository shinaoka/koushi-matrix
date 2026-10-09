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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomMemberRoleOption {
    pub power_level: i64,
    pub role: RoomMemberRole,
    pub requires_confirmation: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomMemberMembership {
    Joined,
    Invited,
    #[default]
    Unknown,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomMemberSummary {
    #[serde(default)]
    pub membership: RoomMemberMembership,
    pub user_id: String,
    pub display_name: Option<String>,
    pub display_label: String,
    #[serde(default)]
    pub original_display_label: String,
    pub avatar_url: Option<String>,
    pub power_level: Option<i64>,
    pub role: RoomMemberRole,
    #[serde(default)]
    pub role_options: Vec<RoomMemberRoleOption>,
    #[serde(default)]
    pub user_trust: Option<UserTrustState>,
}

impl fmt::Debug for RoomMemberSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RoomMemberSummary")
            .field("membership", &self.membership)
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
            .field("role_option_count", &self.role_options.len())
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
    KnockRestricted,
    Private,
    /// A rule this client does not model. Shown as-is and never sent back.
    Unknown,
}

impl RoomJoinRule {
    /// Whether a `RoomSettingChange::JoinRule` may carry this rule. The others
    /// need content the command does not carry (a restricted allow list) or
    /// could not be written back faithfully.
    pub fn is_settable(self) -> bool {
        matches!(
            self,
            Self::Public | Self::Invite | Self::Knock | Self::Private
        )
    }
}

/// What the client could determine about a `restricted`/`knock_restricted`
/// rule's allow conditions (#1220). Mirrors the SDK's five-way completeness.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RestrictedConditions {
    /// The join-rule content is unavailable (unsynced or hidden): nothing is
    /// claimed about the rule or its allow list.
    NotInspected,
    /// The rule is restricted and its allow list is empty.
    ConfirmedEmpty,
    /// Every allow entry is a room-membership rule, and there is at least one.
    MembershipOnly,
    /// At least one room-membership entry beside at least one unmodelled entry.
    MembershipPlusUnsupported,
    /// At least one allow entry, all of them unmodelled.
    UnsupportedOnly,
}

/// The kind of one restricted-rule allow target (#1220), verified from the
/// local room's create event.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomAllowTargetKind {
    Space,
    Room,
    Unknown,
}

/// One distinct allow target of a restricted rule (#1220).
///
/// The id stays inside Rust: every distinct target is counted before unnamed
/// ones are dropped, and the sidebar resolves the id to a display label rather
/// than exposing it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomAllowTarget {
    pub kind: RoomAllowTargetKind,
    pub room_id: String,
}

/// One room's projected access condition (#1166, #1220).
///
/// The join rule is `None` when the rule content is unavailable, so an unsynced
/// rule is never defaulted to `Invite` and reported as inspected. The
/// restricted facts are carried only when the rule is
/// `restricted`/`knock_restricted`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomAccessCondition {
    pub join_rule: Option<RoomJoinRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restricted: Option<RestrictedConditions>,
    /// Rooms and Spaces a restricted rule names as membership routes (#1220).
    /// Rust resolves these to display labels before the renderer sees anything.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_targets: Vec<RoomAllowTarget>,
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
    /// Whether the account may send `m.room.join_rules`. Changing who can join
    /// needs only this, so a member who may not rename the room can still
    /// hold it (#935).
    #[serde(default)]
    pub can_change_join_rule: bool,
    pub can_edit_roles: bool,
    #[serde(default)]
    pub can_invite: bool,
    pub can_kick: bool,
    pub can_ban: bool,
    pub can_unban: bool,
}

impl RoomPermissionFacts {
    /// The one permission check for a settings change, shared by the Core
    /// guard before the state event is sent and the reducer guard that admits
    /// the pending operation, so the two can never disagree.
    pub fn allows_setting_change(&self, change: &RoomSettingChange) -> bool {
        match change {
            RoomSettingChange::JoinRule(_) => self.can_change_join_rule,
            RoomSettingChange::Name(_)
            | RoomSettingChange::Topic(_)
            | RoomSettingChange::AvatarUrl(_)
            | RoomSettingChange::HistoryVisibility(_) => self.can_edit_settings,
        }
    }
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
