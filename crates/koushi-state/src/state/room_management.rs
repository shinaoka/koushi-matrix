use std::fmt;

use serde::{Deserialize, Serialize};

use super::errors::OperationFailureKind;
use crate::{
    RoomAccessOutcome, RoomAccessResolveInput, RoomAccessViewerFacts, RoomDirectoryVisibility,
    resolve_room_access_outcome,
};

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomManagementState {
    pub selected_room_id: Option<String>,
    pub settings: Option<RoomSettingsSnapshot>,
    /// The Rust-owned access/history draft (#1177). React keeps only DOM/focus
    /// state; every rule, target and history selection lives here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<RoomAccessDraft>,
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
            .field("draft", &self.draft)
            .field("operation", &self.operation)
            .finish()
    }
}

/// Which editor a draft belongs to (#1177): the room being edited, or the
/// pending create session.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RoomAccessDraftScope {
    Room { room_id: String },
    Create { session_id: u64 },
}

impl fmt::Debug for RoomAccessDraftScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Room { .. } => formatter
                .debug_struct("Room")
                .field("room_id", &"RoomId(..)")
                .finish(),
            Self::Create { session_id } => formatter
                .debug_struct("Create")
                .field("session_id", session_id)
                .finish(),
        }
    }
}

impl RoomAccessDraftScope {
    pub fn room_id(&self) -> Option<&str> {
        match self {
            Self::Room { room_id } => Some(room_id),
            Self::Create { .. } => None,
        }
    }
}

/// The Rust-owned, serializable access/history draft (#1177).
///
/// `revision` increments on every accepted mutation and fences any preview
/// derived from the draft. Outcome notes are derived for the current draft,
/// never stored in it.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomAccessDraft {
    pub scope: RoomAccessDraftScope,
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<RoomJoinRule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<RoomHistoryVisibility>,
}

impl fmt::Debug for RoomAccessDraft {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RoomAccessDraft")
            .field("scope", &self.scope)
            .field("revision", &self.revision)
            .field("rule", &self.rule)
            .field("allow_target_count", &self.allow_targets.len())
            .field("history", &self.history)
            .finish()
    }
}

impl RoomAccessDraft {
    pub fn new(scope: RoomAccessDraftScope) -> Self {
        Self {
            scope,
            revision: 0,
            rule: None,
            allow_targets: Vec::new(),
            history: None,
        }
    }

    fn touch(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }

    pub fn set_rule(&mut self, rule: Option<RoomJoinRule>) {
        if self.rule != rule {
            self.rule = rule;
            self.touch();
        }
    }

    /// Replace the selected allow-target set; canonicalized so a reordered or
    /// duplicated selection is the same value.
    pub fn set_allow_targets(&mut self, allow_targets: Vec<String>) {
        let canonical =
            RoomAccessPolicy::new(self.rule.unwrap_or(RoomJoinRule::Restricted), allow_targets)
                .allow_targets;
        if self.allow_targets != canonical {
            self.allow_targets = canonical;
            self.touch();
        }
    }

    pub fn set_history(&mut self, history: Option<RoomHistoryVisibility>) {
        if self.history != history {
            self.history = history;
            self.touch();
        }
    }

    /// The effective policy the draft would submit, or `None` until a rule is
    /// chosen.
    pub fn policy(&self) -> Option<RoomAccessPolicy> {
        self.rule
            .map(|rule| RoomAccessPolicy::new(rule, self.allow_targets.clone()))
    }

    /// Canonical comparison against the confirmed policy (#1177): the full
    /// rule plus a sorted, deduplicated target set, so a reordered or
    /// duplicated server allow list is not a change.
    pub fn differs_from(&self, settings: &RoomSettingsSnapshot) -> bool {
        let Some(policy) = self.policy() else {
            return false;
        };
        policy != confirmed_access_policy(settings)
    }
}

/// The canonical confirmed policy of a settings snapshot (#1177).
pub fn confirmed_access_policy(settings: &RoomSettingsSnapshot) -> RoomAccessPolicy {
    let rule = settings.access.join_rule.unwrap_or(settings.join_rule);
    let targets = settings
        .access
        .allow_targets
        .iter()
        .map(|target| target.room_id.clone())
        .collect();
    RoomAccessPolicy::new(rule, targets)
}

/// The canonical policy of one observed access condition (#1177), or `None`
/// while the rule content is unavailable. Sorting and deduplicating the allow
/// targets makes a reordered or duplicated server list the same value.
pub fn canonical_access_policy(condition: &RoomAccessCondition) -> Option<RoomAccessPolicy> {
    condition.join_rule.map(|rule| {
        RoomAccessPolicy::new(
            rule,
            condition
                .allow_targets
                .iter()
                .map(|target| target.room_id.clone())
                .collect(),
        )
    })
}

/// The confirmed outcome of a room's access tuple (#1177).
pub fn confirmed_room_access_outcome(
    settings: &RoomSettingsSnapshot,
    encrypted: bool,
    directory: RoomDirectoryVisibility,
    route: Option<&str>,
    viewer: RoomAccessViewerFacts,
) -> RoomAccessOutcome {
    resolve_room_access_outcome(RoomAccessResolveInput {
        join_rule: settings.access.join_rule,
        restricted: settings.access.restricted,
        allow_targets: &settings.access.allow_targets,
        space_members_route: route,
        history: settings.history_visibility,
        encrypted,
        directory,
        viewer,
    })
}

impl RoomAccessDraft {
    /// The outcome of this draft's access values (#1177): the draft's rule and
    /// targets where set, otherwise the confirmed value. `history` may be an
    /// unsaved draft value, so callers label the result accordingly.
    pub fn outcome(
        &self,
        settings: &RoomSettingsSnapshot,
        encrypted: bool,
        directory: RoomDirectoryVisibility,
        route: Option<&str>,
        viewer: RoomAccessViewerFacts,
    ) -> RoomAccessOutcome {
        let (rule, restricted) = match self.rule {
            Some(RoomJoinRule::Restricted) | Some(RoomJoinRule::KnockRestricted) => (
                self.rule,
                Some(if self.allow_targets.is_empty() {
                    RestrictedConditions::ConfirmedEmpty
                } else {
                    RestrictedConditions::MembershipOnly
                }),
            ),
            Some(rule) => (Some(rule), None),
            None => (settings.access.join_rule, settings.access.restricted),
        };
        let targets: Vec<RoomAllowTarget> = self
            .allow_targets
            .iter()
            .map(|room_id| RoomAllowTarget {
                kind: RoomAllowTargetKind::Unknown,
                room_id: room_id.clone(),
            })
            .collect();
        resolve_room_access_outcome(RoomAccessResolveInput {
            join_rule: rule,
            restricted,
            allow_targets: &targets,
            space_members_route: route,
            history: self.history.unwrap_or(settings.history_visibility),
            encrypted,
            directory,
            viewer,
        })
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
    /// The verified access facts (#1220): the rule's availability, its
    /// restricted allow-condition completeness and its target kinds. The
    /// editor reads this instead of trusting the scalar `join_rule`. The raw
    /// target ids stay inside Rust (they are never part of the IPC wire shape).
    #[serde(default, skip)]
    pub access: RoomAccessCondition,
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
            .field("access_rule", &self.access.join_rule)
            .field("access_restricted", &self.access.restricted)
            .field("access_target_count", &self.access.allow_targets.len())
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

impl RoomSettingsSnapshot {
    /// Whether a settable access policy may be submitted against this
    /// snapshot's verified access facts (#1177). `None` means admissible.
    ///
    /// A restricted edit is rejected when the current allow content has entries
    /// this client does not model (rewriting would silently drop them) or could
    /// not be inspected at all. The two are kept apart so the renderer can say
    /// which condition it is.
    pub fn access_policy_rejection(
        &self,
        policy: &RoomAccessPolicy,
    ) -> Option<OperationFailureKind> {
        if !policy.is_submittable() {
            return Some(OperationFailureKind::Invalid);
        }
        match self.access.join_rule {
            None => Some(OperationFailureKind::PolicyNotVerified),
            Some(RoomJoinRule::Restricted | RoomJoinRule::KnockRestricted) => {
                match self.access.restricted {
                    Some(RestrictedConditions::MembershipOnly)
                    | Some(RestrictedConditions::ConfirmedEmpty) => None,
                    Some(RestrictedConditions::MembershipPlusUnsupported)
                    | Some(RestrictedConditions::UnsupportedOnly) => {
                        Some(OperationFailureKind::UnsupportedPolicyCondition)
                    }
                    Some(RestrictedConditions::NotInspected) | None => {
                        Some(OperationFailureKind::PolicyNotVerified)
                    }
                }
            }
            Some(_) => None,
        }
    }
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
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
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
            RoomSettingChange::JoinRule(_) | RoomSettingChange::AccessPolicy(_) => {
                self.can_change_join_rule
            }
            RoomSettingChange::Name(_)
            | RoomSettingChange::Topic(_)
            | RoomSettingChange::AvatarUrl(_)
            | RoomSettingChange::HistoryVisibility(_) => self.can_edit_settings,
        }
    }
}

/// A settable access policy: a join rule plus its canonical membership
/// allow-target set (#1177).
///
/// The targets are sorted and deduplicated so a reordered and/or duplicated
/// server allow list is not a change. `private` stays reserved and is never
/// produced as the Space route; the ordinary `public`/`invite`/`knock` rules
/// carry an empty target set and stay on `RoomSettingChange::JoinRule`.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomAccessPolicy {
    pub rule: RoomJoinRule,
    pub allow_targets: Vec<String>,
}

impl fmt::Debug for RoomAccessPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RoomAccessPolicy")
            .field("rule", &self.rule)
            .field("allow_target_count", &self.allow_targets.len())
            .finish()
    }
}

impl RoomAccessPolicy {
    /// Canonicalize a raw rule/target pair: drop empties, sort, deduplicate.
    pub fn new(rule: RoomJoinRule, allow_targets: Vec<String>) -> Self {
        let mut allow_targets = allow_targets;
        allow_targets.retain(|target| !target.is_empty());
        allow_targets.sort();
        allow_targets.dedup();
        Self {
            rule,
            allow_targets,
        }
    }

    /// Whether this policy may be submitted to the SDK. A restricted rule
    /// needs at least one verified membership target; a non-restricted rule
    /// carries none; `knock_restricted`, `private` and `unknown` are not
    /// settable through this path.
    pub fn is_submittable(&self) -> bool {
        match self.rule {
            RoomJoinRule::Restricted => !self.allow_targets.is_empty(),
            RoomJoinRule::Public | RoomJoinRule::Invite | RoomJoinRule::Knock => {
                self.allow_targets.is_empty()
            }
            RoomJoinRule::KnockRestricted | RoomJoinRule::Private | RoomJoinRule::Unknown => false,
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
    /// Set a restricted rule together with its membership allow list, or move
    /// to a rule that carries none (#1177). The allow list is canonical.
    AccessPolicy(RoomAccessPolicy),
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
            Self::AccessPolicy(policy) => formatter
                .debug_struct("AccessPolicy")
                .field("rule", &policy.rule)
                .field("allow_target_count", &policy.allow_targets.len())
                .finish(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::errors::OperationFailureKind;

    fn snapshot_with_access(access: RoomAccessCondition) -> RoomSettingsSnapshot {
        RoomSettingsSnapshot {
            room_id: "!room:example.invalid".to_owned(),
            name: None,
            topic: None,
            avatar_url: None,
            canonical_alias: None,
            alternate_aliases: Vec::new(),
            share_link: None,
            join_rule: access.join_rule.unwrap_or(RoomJoinRule::Invite),
            history_visibility: RoomHistoryVisibility::Shared,
            access,
            permissions: RoomPermissionFacts::default(),
            members: Vec::new(),
        }
    }

    fn restricted(completeness: RestrictedConditions) -> RoomAccessCondition {
        RoomAccessCondition {
            join_rule: Some(RoomJoinRule::Restricted),
            restricted: Some(completeness),
            allow_targets: vec![RoomAllowTarget {
                kind: RoomAllowTargetKind::Space,
                room_id: "!space:example.invalid".to_owned(),
            }],
        }
    }

    #[test]
    fn access_policy_is_canonical_and_submittable() {
        let policy = RoomAccessPolicy::new(
            RoomJoinRule::Restricted,
            vec![
                "!b:example.invalid".to_owned(),
                "!a:example.invalid".to_owned(),
                "!a:example.invalid".to_owned(),
                String::new(),
            ],
        );
        assert_eq!(
            policy.allow_targets,
            vec![
                "!a:example.invalid".to_owned(),
                "!b:example.invalid".to_owned()
            ]
        );
        assert!(policy.is_submittable());
        assert!(!RoomAccessPolicy::new(RoomJoinRule::Restricted, Vec::new()).is_submittable());
        assert!(
            !RoomAccessPolicy::new(RoomJoinRule::Public, vec!["!a:example.invalid".to_owned()])
                .is_submittable()
        );
        assert!(!RoomAccessPolicy::new(RoomJoinRule::Private, Vec::new()).is_submittable());
        assert!(!RoomAccessPolicy::new(RoomJoinRule::KnockRestricted, Vec::new()).is_submittable());
    }

    #[test]
    fn access_policy_rejection_separates_unsupported_from_unverified() {
        let policy = RoomAccessPolicy::new(
            RoomJoinRule::Restricted,
            vec!["!s:example.invalid".to_owned()],
        );

        assert_eq!(
            snapshot_with_access(restricted(RestrictedConditions::MembershipOnly))
                .access_policy_rejection(&policy),
            None
        );
        assert_eq!(
            snapshot_with_access(restricted(RestrictedConditions::ConfirmedEmpty))
                .access_policy_rejection(&policy),
            None
        );
        assert_eq!(
            snapshot_with_access(restricted(RestrictedConditions::MembershipPlusUnsupported))
                .access_policy_rejection(&policy),
            Some(OperationFailureKind::UnsupportedPolicyCondition)
        );
        assert_eq!(
            snapshot_with_access(restricted(RestrictedConditions::UnsupportedOnly))
                .access_policy_rejection(&policy),
            Some(OperationFailureKind::UnsupportedPolicyCondition)
        );
        assert_eq!(
            snapshot_with_access(RoomAccessCondition {
                join_rule: None,
                restricted: Some(RestrictedConditions::NotInspected),
                allow_targets: Vec::new(),
            })
            .access_policy_rejection(&policy),
            Some(OperationFailureKind::PolicyNotVerified)
        );
        assert_eq!(
            snapshot_with_access(RoomAccessCondition {
                join_rule: Some(RoomJoinRule::Public),
                restricted: None,
                allow_targets: Vec::new(),
            })
            .access_policy_rejection(&policy),
            None
        );
        assert_eq!(
            snapshot_with_access(restricted(RestrictedConditions::MembershipOnly))
                .access_policy_rejection(&RoomAccessPolicy::new(
                    RoomJoinRule::Restricted,
                    Vec::new()
                )),
            Some(OperationFailureKind::Invalid)
        );
    }

    #[test]
    fn draft_comparison_is_canonical_and_ignores_a_reordered_server_list() {
        let settings = snapshot_with_access(restricted(RestrictedConditions::MembershipOnly));
        let mut draft = RoomAccessDraft::new(RoomAccessDraftScope::Room {
            room_id: "!room:example.invalid".to_owned(),
        });
        assert!(
            !draft.differs_from(&settings),
            "an empty draft is not a change"
        );
        draft.set_rule(Some(RoomJoinRule::Restricted));
        draft.set_allow_targets(vec!["!space:example.invalid".to_owned()]);
        assert!(
            !draft.differs_from(&settings),
            "the same canonical policy is not a change"
        );
        draft.set_allow_targets(vec![
            "!space:example.invalid".to_owned(),
            "!space:example.invalid".to_owned(),
        ]);
        assert!(
            !draft.differs_from(&settings),
            "a duplicated target is not a change"
        );
        draft.set_allow_targets(vec!["!other:example.invalid".to_owned()]);
        assert!(
            draft.differs_from(&settings),
            "a changed target is a change"
        );
        let revision = draft.revision;
        draft.set_allow_targets(vec!["!other:example.invalid".to_owned()]);
        assert_eq!(
            draft.revision, revision,
            "a no-op mutation does not bump the revision"
        );
    }

    #[test]
    fn draft_outcome_uses_the_draft_history_with_the_confirmed_encryption() {
        let settings = snapshot_with_access(restricted(RestrictedConditions::MembershipOnly));
        let mut draft = RoomAccessDraft::new(RoomAccessDraftScope::Room {
            room_id: "!room:example.invalid".to_owned(),
        });
        draft.set_rule(Some(RoomJoinRule::Restricted));
        draft.set_allow_targets(vec!["!space:example.invalid".to_owned()]);
        draft.set_history(Some(RoomHistoryVisibility::Joined));
        let outcome = draft.outcome(
            &settings,
            true,
            RoomDirectoryVisibility::Private,
            Some("Design Team"),
            RoomAccessViewerFacts::default(),
        );
        assert_eq!(
            outcome.join.message_id,
            "room.accessOutcomeJoinSpaceMembers"
        );
        assert_eq!(
            outcome.history.message_id,
            "room.accessOutcomeHistoryJoined"
        );
        assert!(outcome.history_key_caveat.is_none());
        assert_eq!(
            confirmed_room_access_outcome(
                &settings,
                true,
                RoomDirectoryVisibility::Private,
                None,
                RoomAccessViewerFacts::default(),
            )
            .join
            .message_id,
            "room.accessOutcomeJoinMembershipRoute"
        );
    }

    #[test]
    fn access_policy_change_uses_the_join_rule_permission() {
        let change = RoomSettingChange::AccessPolicy(RoomAccessPolicy::new(
            RoomJoinRule::Restricted,
            vec!["!s:example.invalid".to_owned()],
        ));
        let mut permissions = RoomPermissionFacts {
            can_change_join_rule: true,
            ..RoomPermissionFacts::default()
        };
        assert!(permissions.allows_setting_change(&change));
        permissions.can_change_join_rule = false;
        assert!(!permissions.allows_setting_change(&change));
    }
}
