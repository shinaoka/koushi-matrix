use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::errors::OperationFailureKind;
use crate::{
    CreateRoomVisibility, RoomAccessOutcome, RoomAccessResolveInput, RoomAccessViewerFacts,
    RoomDirectoryVisibility, resolve_room_access_outcome,
};

use super::AppState;

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomManagementState {
    pub selected_room_id: Option<String>,
    pub settings: Option<RoomSettingsSnapshot>,
    /// The Rust-owned access/history draft (#1177). React keeps only DOM/focus
    /// state; every rule, target and history selection lives here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<RoomAccessDraft>,
    /// Where the open room is published in the queried directory (#1177), read
    /// with `get_room_visibility` for confirmed rooms. Kept off the wire; the
    /// preview resolves it into a confirmed outcome line.
    #[serde(skip)]
    pub directory: RoomDirectoryVisibility,
    /// The room whose access/history editor lifetime is currently admitted
    /// (#1177). A mutation from any other room's editor is rejected.
    #[serde(skip)]
    pub active_room_editor: Option<String>,
    /// The create session whose editor lifetime is currently admitted (#1177).
    /// A mutation or reset from a retired session is rejected.
    #[serde(skip)]
    pub active_create_session: Option<u64>,
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
            .field("directory", &self.directory)
            .field(
                "active_room_editor",
                &self.active_room_editor.as_ref().map(|_| "RoomId(..)"),
            )
            .field("active_create_session", &self.active_create_session)
            .field("operation", &self.operation)
            .finish()
    }
}

/// Which editor a draft belongs to (#1177): the room being edited, or the
/// pending create session.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
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

/// The create dialog facts used to seed the effective selection when its editor
/// opens (#1177). These are the user's own dialog choices, not trusted Matrix
/// facts; the preview resolves authoritative facts itself.
#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRoomAccessSeed {
    pub visibility: CreateRoomVisibility,
    #[serde(default)]
    pub invited_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_space_id: Option<String>,
}

impl fmt::Debug for CreateRoomAccessSeed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreateRoomAccessSeed")
            .field("visibility", &self.visibility)
            .field("invited_only", &self.invited_only)
            .field(
                "parent_space_id",
                &self.parent_space_id.as_ref().map(|_| "RoomId(..)"),
            )
            .finish()
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
    /// Whether the user has made an explicit choice in this editor (#1177). An
    /// untouched creation draft leaves the legacy preset path to Create. The
    /// initial legacy selection is seeded without touching it.
    #[serde(default)]
    pub touched: bool,
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
            .field("touched", &self.touched)
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
            touched: false,
        }
    }

    /// Establish the effective creation selection from the legacy preset before
    /// any target editing (#1177), without marking it as a user choice. A
    /// private room in a Space starts restricted to that attachment Space, so a
    /// target edit carries a rule and the attachment is never silently kept
    /// while another target is added.
    pub fn seed_create_selection(
        &mut self,
        visibility: CreateRoomVisibility,
        invited_only: bool,
        parent_space_id: Option<&str>,
    ) {
        let rule = if visibility == CreateRoomVisibility::Public {
            RoomJoinRule::Public
        } else if invited_only || parent_space_id.is_none() {
            RoomJoinRule::Invite
        } else {
            RoomJoinRule::Restricted
        };
        self.rule = Some(rule);
        self.allow_targets = if rule == RoomJoinRule::Restricted {
            RoomAccessPolicy::new(
                rule,
                parent_space_id.map(str::to_owned).into_iter().collect(),
            )
            .allow_targets
        } else {
            Vec::new()
        };
        self.history = None;
        self.touched = false;
    }

    fn touch(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }

    pub fn set_rule(&mut self, rule: Option<RoomJoinRule>) {
        if self.rule != rule {
            self.rule = rule;
            self.touch();
            self.touched = true;
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
            self.touched = true;
        }
    }

    pub fn set_history(&mut self, history: Option<RoomHistoryVisibility>) {
        if self.history != history {
            self.history = history;
            self.touch();
            self.touched = true;
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

/// Which property a Rust preview describes (#1177). One enum keeps an access
/// preview result out of the history panel and vice versa.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomAccessPreviewContext {
    Access,
    History,
}

/// A stateless Rust preview of one panel's effective tuple (#1177).
///
/// It is computed from the current snapshot and returned, never stored: the
/// `confirmed` flag says whether every shown policy value is the confirmed one
/// (no unsaved draft value is part of the outcome).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomAccessPreview {
    pub scope: RoomAccessDraftScope,
    pub context: RoomAccessPreviewContext,
    pub confirmed: bool,
    pub outcome: RoomAccessOutcome,
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
    /// The outcome of one panel (#1177): the access panel combines the draft
    /// rule and targets with the **confirmed** history, the history panel the
    /// **confirmed** access with the draft history. The panel that is not being
    /// edited therefore never shows an unsaved value.
    pub fn outcome_for_context(
        &self,
        context: RoomAccessPreviewContext,
        settings: &RoomSettingsSnapshot,
        encrypted: bool,
        directory: RoomDirectoryVisibility,
        route: Option<&str>,
        viewer: RoomAccessViewerFacts,
    ) -> RoomAccessOutcome {
        let draft_targets: Vec<RoomAllowTarget> = self
            .allow_targets
            .iter()
            .map(|room_id| RoomAllowTarget {
                kind: RoomAllowTargetKind::Unknown,
                room_id: room_id.clone(),
            })
            .collect();
        let (rule, restricted, targets, history) = match context {
            RoomAccessPreviewContext::Access => match self.rule {
                Some(RoomJoinRule::Restricted) | Some(RoomJoinRule::KnockRestricted) => (
                    self.rule,
                    Some(if self.allow_targets.is_empty() {
                        RestrictedConditions::ConfirmedEmpty
                    } else {
                        RestrictedConditions::MembershipOnly
                    }),
                    draft_targets.as_slice(),
                    settings.history_visibility,
                ),
                Some(rule) => (
                    Some(rule),
                    None,
                    draft_targets.as_slice(),
                    settings.history_visibility,
                ),
                // No draft rule: the access panel shows the confirmed policy,
                // including its confirmed allow targets, never the draft's
                // (possibly edited) target list.
                None => (
                    settings.access.join_rule,
                    settings.access.restricted,
                    settings.access.allow_targets.as_slice(),
                    settings.history_visibility,
                ),
            },
            RoomAccessPreviewContext::History => (
                settings.access.join_rule,
                settings.access.restricted,
                settings.access.allow_targets.as_slice(),
                self.history.unwrap_or(settings.history_visibility),
            ),
        };
        resolve_room_access_outcome(RoomAccessResolveInput {
            join_rule: rule,
            restricted,
            allow_targets: targets,
            space_members_route: route,
            history,
            encrypted,
            directory,
            viewer,
        })
    }

    /// Whether one panel's shown policy is entirely the confirmed value (#1177).
    pub fn context_is_confirmed(
        &self,
        context: RoomAccessPreviewContext,
        settings: &RoomSettingsSnapshot,
    ) -> bool {
        match context {
            RoomAccessPreviewContext::Access => self.rule.is_none() || !self.differs_from(settings),
            RoomAccessPreviewContext::History => {
                self.history.is_none() || self.history == Some(settings.history_visibility)
            }
        }
    }
}

/// A stateless Rust preview of one Room Info panel (#1177).
///
/// A draft applies only when its scope matches the request, so a stale editor
/// previews the confirmed value instead of another room's draft. The returned
/// `scope`/`context`/`confirmed` plus the outcome lines are the full identity
/// the caller fences stale results against.
pub fn preview_room_access_draft(
    state: &AppState,
    scope: &RoomAccessDraftScope,
    context: RoomAccessPreviewContext,
) -> RoomAccessPreview {
    let room_id = scope.room_id();
    let settings = room_id.and_then(|id| {
        state
            .room_management
            .settings
            .as_ref()
            .filter(|settings| settings.room_id == id)
    });
    let draft = state
        .room_management
        .draft
        .as_ref()
        .filter(|draft| &draft.scope == scope);
    let encrypted = room_id
        .and_then(|id| state.rooms.iter().find(|room| room.room_id == id))
        .is_some_and(|room| room.is_encrypted);
    // The confirmed directory publication is read from the SDK when the room's
    // settings load; an open room without that read yet is "loading", and a
    // room that is not loaded makes no claim at all.
    let directory = if settings.is_some() {
        state.room_management.directory
    } else {
        RoomDirectoryVisibility::Unavailable
    };
    let viewer = RoomAccessViewerFacts::default();
    let (outcome, confirmed) = match (settings, draft) {
        (Some(settings), Some(draft)) => (
            draft.outcome_for_context(
                context,
                settings,
                encrypted,
                directory,
                room_access_route_name(state, context, settings, draft).as_deref(),
                viewer,
            ),
            draft.context_is_confirmed(context, settings),
        ),
        (Some(settings), None) => (
            confirmed_room_access_outcome(
                settings,
                encrypted,
                directory,
                confirmed_room_access_route_name(state, settings).as_deref(),
                viewer,
            ),
            true,
        ),
        (None, _) => (
            resolve_room_access_outcome(RoomAccessResolveInput {
                join_rule: None,
                restricted: None,
                allow_targets: &[],
                space_members_route: None,
                history: RoomHistoryVisibility::Joined,
                encrypted: false,
                directory,
                viewer,
            }),
            true,
        ),
    };
    RoomAccessPreview {
        scope: scope.clone(),
        context,
        confirmed,
        outcome,
    }
}

/// The effective create inputs the Rust preview normalizes with the same rules
/// Create applies (#1177). `parent_space_id` is the attachment Space, not a
/// trusted assertion.
#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRoomAccessPreviewInput {
    pub visibility: CreateRoomVisibility,
    #[serde(default)]
    pub invited_only: bool,
    #[serde(default)]
    pub encrypted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_space_id: Option<String>,
}

impl fmt::Debug for CreateRoomAccessPreviewInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreateRoomAccessPreviewInput")
            .field("visibility", &self.visibility)
            .field("invited_only", &self.invited_only)
            .field("encrypted", &self.encrypted)
            .field(
                "parent_space_id",
                &self.parent_space_id.as_ref().map(|_| "RoomId(..)"),
            )
            .finish()
    }
}

/// Why a create proposal cannot be submitted (#1177), mirroring the typed
/// rejections Create's own normalization raises.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CreateRoomAccessRejection {
    /// A published public room cannot also be membership-restricted.
    PublicWithRestrictedAccess,
    /// The explicit policy wins only while the legacy flags are at defaults.
    ExplicitPolicyWithInvitedOnly,
    /// A restricted rule with no membership route is not a route.
    EmptyAccessTargets,
    /// A public room is never invite-only.
    PublicWithInvitedOnly,
}

/// A stateless preview of the create dialog's effective proposed tuple (#1177).
/// A create proposal is never "confirmed", so `confirmed` is always false.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRoomAccessPreview {
    pub scope: RoomAccessDraftScope,
    pub confirmed: bool,
    pub outcome: RoomAccessOutcome,
    /// The effective join rule Create would submit, after its own normalization
    /// (the legacy private-in-Space preset included).
    pub effective_rule: Option<RoomJoinRule>,
    /// The effective history visibility Create would submit.
    pub effective_history: RoomHistoryVisibility,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejection: Option<CreateRoomAccessRejection>,
    /// An explicit restricted rule pins room version V9, as does the legacy
    /// private-in-Space preset.
    pub room_version_pinned: bool,
}

/// A stateless Rust preview of the create dialog's effective proposed tuple
/// (#1177). The explicit draft policy wins; absent it the legacy
/// visibility/invite-only/parent-Space preset is reproduced.
pub fn preview_create_room_access(
    state: &AppState,
    scope: &RoomAccessDraftScope,
    input: CreateRoomAccessPreviewInput,
) -> CreateRoomAccessPreview {
    let draft = state
        .room_management
        .draft
        .as_ref()
        .filter(|draft| &draft.scope == scope);
    let public = input.visibility == CreateRoomVisibility::Public;
    let parent_id = input.parent_space_id.as_deref();
    let draft_rule = draft.and_then(|draft| draft.rule);
    let (rule, restricted, target_ids, route) = if let Some(rule) = draft_rule {
        let targets = draft
            .map(|draft| draft.allow_targets.clone())
            .unwrap_or_default();
        let restricted = matches!(
            rule,
            RoomJoinRule::Restricted | RoomJoinRule::KnockRestricted
        )
        .then(|| {
            if targets.is_empty() {
                RestrictedConditions::ConfirmedEmpty
            } else {
                RestrictedConditions::MembershipOnly
            }
        });
        let route = single_space_route_name(state, &targets);
        (Some(rule), restricted, targets, route)
    } else if public {
        (Some(RoomJoinRule::Public), None, Vec::new(), None)
    } else if input.invited_only || parent_id.is_none() {
        (Some(RoomJoinRule::Invite), None, Vec::new(), None)
    } else {
        // The legacy private-in-Space preset names the attachment Space.
        let id = parent_id.unwrap_or_default().to_owned();
        let route = single_space_route_name(state, std::slice::from_ref(&id));
        (
            Some(RoomJoinRule::Restricted),
            Some(RestrictedConditions::MembershipOnly),
            vec![id],
            route,
        )
    };
    let draft_targets: Vec<RoomAllowTarget> = target_ids
        .iter()
        .map(|room_id| RoomAllowTarget {
            kind: RoomAllowTargetKind::Unknown,
            room_id: room_id.clone(),
        })
        .collect();
    let encrypted = input.encrypted && !public;
    // Mirror Create's own history normalization: an explicit value wins, else a
    // private room in a Space takes the legacy `Invited` default.
    let history = draft
        .and_then(|draft| draft.history)
        .or_else(|| (!public && parent_id.is_some()).then_some(RoomHistoryVisibility::Invited))
        .unwrap_or(RoomHistoryVisibility::Shared);
    let rejection = if draft_rule.is_some() {
        if input.invited_only {
            Some(CreateRoomAccessRejection::ExplicitPolicyWithInvitedOnly)
        } else if public && draft_rule == Some(RoomJoinRule::Restricted) {
            Some(CreateRoomAccessRejection::PublicWithRestrictedAccess)
        } else if draft_rule == Some(RoomJoinRule::Restricted) && target_ids.is_empty() {
            Some(CreateRoomAccessRejection::EmptyAccessTargets)
        } else {
            None
        }
    } else if public && input.invited_only {
        Some(CreateRoomAccessRejection::PublicWithInvitedOnly)
    } else {
        None
    };
    let room_version_pinned = if draft_rule.is_some() {
        matches!(
            draft_rule,
            Some(RoomJoinRule::Restricted | RoomJoinRule::KnockRestricted)
        )
    } else {
        !public && parent_id.is_some()
    };
    let outcome = resolve_room_access_outcome(RoomAccessResolveInput {
        join_rule: rule,
        restricted,
        allow_targets: &draft_targets,
        space_members_route: route.as_deref(),
        history,
        encrypted,
        directory: if public {
            RoomDirectoryVisibility::ProposedPublic
        } else {
            RoomDirectoryVisibility::ProposedPrivate
        },
        viewer: RoomAccessViewerFacts::default(),
    });
    CreateRoomAccessPreview {
        scope: scope.clone(),
        confirmed: false,
        outcome,
        effective_rule: rule,
        effective_history: history,
        rejection,
        room_version_pinned,
    }
}

fn room_access_route_name(
    state: &AppState,
    context: RoomAccessPreviewContext,
    settings: &RoomSettingsSnapshot,
    draft: &RoomAccessDraft,
) -> Option<String> {
    let ids: Vec<String> = match context {
        RoomAccessPreviewContext::Access => match draft.rule {
            Some(RoomJoinRule::Restricted) | Some(RoomJoinRule::KnockRestricted) => {
                draft.allow_targets.clone()
            }
            Some(_) => Vec::new(),
            None => confirmed_route_ids(settings),
        },
        RoomAccessPreviewContext::History => confirmed_route_ids(settings),
    };
    single_space_route_name(state, &ids)
}

fn confirmed_room_access_route_name(
    state: &AppState,
    settings: &RoomSettingsSnapshot,
) -> Option<String> {
    single_space_route_name(state, &confirmed_route_ids(settings))
}

fn confirmed_route_ids(settings: &RoomSettingsSnapshot) -> Vec<String> {
    let target = settings.access.allow_targets.as_slice();
    if settings.access.restricted == Some(RestrictedConditions::MembershipOnly)
        && target.len() == 1
        && target[0].kind == RoomAllowTargetKind::Space
    {
        vec![target[0].room_id.clone()]
    } else {
        Vec::new()
    }
}

/// The one distinct Space-route name for a target set, or `None` unless it is
/// exactly one id that names a joined Space with a safe display name (#1220).
fn single_space_route_name(state: &AppState, ids: &[String]) -> Option<String> {
    if ids.len() != 1 {
        return None;
    }
    let id = ids[0].as_str();
    state
        .spaces
        .iter()
        .find(|space| space.space_id == id)
        .and_then(|space| {
            let name = space
                .raw_name
                .as_deref()
                .unwrap_or(space.display_name.as_str())
                .trim();
            (!name.is_empty() && name != id).then(|| name.to_owned())
        })
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

/// Whether a requested access policy may be admitted for its room (#1177).
///
/// The requested policy is admitted only when every newly selected allow target
/// is a joined, verified Space (the predicate is authoritative: the SDK client
/// locally or the projected room list in the reducer). An existing non-Space
/// condition, which this editor cannot model, is preserved or its removal is
/// rejected explicitly, never silently dropped. `None` means admissible.
pub fn access_policy_target_rejection(
    settings: &RoomSettingsSnapshot,
    policy: &RoomAccessPolicy,
    is_joined_verified_space: impl Fn(&str) -> bool,
) -> Option<OperationFailureKind> {
    let confirmed_ids: BTreeSet<&str> = settings
        .access
        .allow_targets
        .iter()
        .map(|target| target.room_id.as_str())
        .collect();
    let has_non_space_confirmed = settings
        .access
        .allow_targets
        .iter()
        .any(|target| target.kind != RoomAllowTargetKind::Space);
    if has_non_space_confirmed {
        let policy_ids: BTreeSet<&str> = policy.allow_targets.iter().map(String::as_str).collect();
        if !confirmed_ids.is_subset(&policy_ids) {
            return Some(OperationFailureKind::UnsupportedPolicyCondition);
        }
    }
    for target in &policy.allow_targets {
        if confirmed_ids.contains(target.as_str()) {
            continue;
        }
        if !is_joined_verified_space(target) {
            return Some(OperationFailureKind::PolicyNotVerified);
        }
    }
    None
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

/// One room's complete observed access/history tuple (#1177).
///
/// The shared access projection (`AppState::room_access`) carries only the
/// condition for the room list; the history visibility rides the same
/// observation so an external history change reaches an open Room Info editor
/// through the one existing observation/reconciliation owner. The observed
/// tuple is kept apart from the immediately displayed (possibly locally
/// accepted) value so an unchanged old observation is distinguishable from a
/// value this client just saved.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomAccessObservation {
    pub access: RoomAccessCondition,
    pub history_visibility: RoomHistoryVisibility,
}

impl From<RoomAccessCondition> for RoomAccessObservation {
    fn from(access: RoomAccessCondition) -> Self {
        Self {
            access,
            history_visibility: RoomHistoryVisibility::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomHistoryVisibility {
    WorldReadable,
    #[default]
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
    use crate::SpaceSummary;
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

    #[test]
    fn preview_combines_each_panel_with_the_confirmed_other_property() {
        let scope = RoomAccessDraftScope::Room {
            room_id: "!room:example.invalid".to_owned(),
        };
        let mut state = AppState::default();
        state.room_management.settings = Some(snapshot_with_access(restricted(
            RestrictedConditions::MembershipOnly,
        )));
        let mut draft = RoomAccessDraft::new(scope.clone());
        draft.set_rule(Some(RoomJoinRule::Public));
        draft.set_history(Some(RoomHistoryVisibility::Invited));
        state.room_management.draft = Some(draft);

        let access = preview_room_access_draft(&state, &scope, RoomAccessPreviewContext::Access);
        assert_eq!(
            access.outcome.join.message_id,
            "room.accessOutcomeJoinPublic"
        );
        assert_eq!(
            access.outcome.history.message_id,
            "room.accessOutcomeHistoryShared"
        );
        assert!(!access.confirmed, "the access panel shows an unsaved rule");

        let history = preview_room_access_draft(&state, &scope, RoomAccessPreviewContext::History);
        assert_eq!(
            history.outcome.join.message_id,
            "room.accessOutcomeJoinMembershipRoute"
        );
        assert_eq!(
            history.outcome.history.message_id,
            "room.accessOutcomeHistoryInvited"
        );
        assert!(
            !history.confirmed,
            "the history panel shows an unsaved value"
        );
    }

    #[test]
    fn preview_of_a_stale_scope_uses_the_confirmed_value() {
        let mut state = AppState::default();
        state.room_management.settings = Some(snapshot_with_access(restricted(
            RestrictedConditions::MembershipOnly,
        )));
        state.room_management.draft = Some(RoomAccessDraft::new(RoomAccessDraftScope::Room {
            room_id: "!other:example.invalid".to_owned(),
        }));
        let scope = RoomAccessDraftScope::Room {
            room_id: "!room:example.invalid".to_owned(),
        };
        let preview = preview_room_access_draft(&state, &scope, RoomAccessPreviewContext::Access);
        assert!(preview.confirmed);
        assert_eq!(
            preview.outcome.join.message_id,
            "room.accessOutcomeJoinMembershipRoute"
        );
    }

    #[test]
    fn access_panel_uses_confirmed_targets_without_a_draft_rule() {
        let scope = RoomAccessDraftScope::Room {
            room_id: "!room:example.invalid".to_owned(),
        };
        let mut state = AppState::default();
        state.room_management.settings = Some(snapshot_with_access(restricted(
            RestrictedConditions::MembershipOnly,
        )));
        state.spaces = vec![SpaceSummary {
            space_id: "!space:example.invalid".to_owned(),
            raw_name: Some("Design".to_owned()),
            display_name: "Design".to_owned(),
            avatar: None,
            join_rule: None,
            child_room_ids: Vec::new(),
            parent_side_child_room_ids: Vec::new(),
        }];
        // A history-only draft must not blank the access panel's confirmed
        // targets or its verified single-Space route.
        let mut draft = RoomAccessDraft::new(scope.clone());
        draft.set_history(Some(RoomHistoryVisibility::Invited));
        state.room_management.draft = Some(draft);

        let preview = preview_room_access_draft(&state, &scope, RoomAccessPreviewContext::Access);
        assert_eq!(
            preview.outcome.join.message_id,
            "room.accessOutcomeJoinSpaceMembers"
        );
        assert_eq!(
            preview.outcome.join.substitutions,
            vec!["Design".to_owned()]
        );
        assert!(preview.confirmed);
    }

    #[test]
    fn create_preview_reports_the_effective_rule_and_history() {
        let scope = RoomAccessDraftScope::Create { session_id: 1 };
        let state = AppState::default();
        // A private room in a Space keeps the legacy restricted preset and its
        // `invited` history, and it pins room version V9.
        let legacy = preview_create_room_access(
            &state,
            &scope,
            CreateRoomAccessPreviewInput {
                visibility: CreateRoomVisibility::Private,
                invited_only: false,
                encrypted: true,
                parent_space_id: Some("!space:example.invalid".to_owned()),
            },
        );
        assert_eq!(legacy.effective_rule, Some(RoomJoinRule::Restricted));
        assert_eq!(legacy.effective_history, RoomHistoryVisibility::Invited);
        assert_eq!(legacy.rejection, None);
        assert!(legacy.room_version_pinned);

        // A private room at Home is invite-only with the shared-history default.
        let home =
            preview_create_room_access(&state, &scope, CreateRoomAccessPreviewInput::default());
        assert_eq!(home.effective_rule, Some(RoomJoinRule::Invite));
        assert_eq!(home.effective_history, RoomHistoryVisibility::Shared);
        assert!(!home.room_version_pinned);
    }

    #[test]
    fn create_preview_reports_rejections_and_room_version() {
        let scope = RoomAccessDraftScope::Create { session_id: 1 };
        let mut state = AppState::default();
        let mut draft = RoomAccessDraft::new(scope.clone());
        draft.set_rule(Some(RoomJoinRule::Restricted));
        draft.set_allow_targets(vec!["!space:example.invalid".to_owned()]);
        state.room_management.draft = Some(draft);

        let public = preview_create_room_access(
            &state,
            &scope,
            CreateRoomAccessPreviewInput {
                visibility: CreateRoomVisibility::Public,
                invited_only: false,
                encrypted: true,
                parent_space_id: None,
            },
        );
        assert_eq!(
            public.rejection,
            Some(CreateRoomAccessRejection::PublicWithRestrictedAccess)
        );
        assert!(public.room_version_pinned);
        assert!(!public.confirmed);

        let invited = preview_create_room_access(
            &state,
            &scope,
            CreateRoomAccessPreviewInput {
                visibility: CreateRoomVisibility::Private,
                invited_only: true,
                encrypted: true,
                parent_space_id: None,
            },
        );
        assert_eq!(
            invited.rejection,
            Some(CreateRoomAccessRejection::ExplicitPolicyWithInvitedOnly)
        );
    }

    #[test]
    fn preview_reports_confirmed_and_proposed_directory_publication() {
        let scope = RoomAccessDraftScope::Room {
            room_id: "!room:example.invalid".to_owned(),
        };
        let mut state = AppState::default();
        state.room_management.settings = Some(snapshot_with_access(restricted(
            RestrictedConditions::MembershipOnly,
        )));

        state.room_management.directory = RoomDirectoryVisibility::Public;
        assert_eq!(
            preview_room_access_draft(&state, &scope, RoomAccessPreviewContext::Access)
                .outcome
                .directory
                .message_id,
            "room.accessOutcomeDirectoryPublic"
        );
        state.room_management.directory = RoomDirectoryVisibility::Private;
        assert_eq!(
            preview_room_access_draft(&state, &scope, RoomAccessPreviewContext::Access)
                .outcome
                .directory
                .message_id,
            "room.accessOutcomeDirectoryPrivate"
        );
        for (visibility, expected) in [
            (
                RoomDirectoryVisibility::Unavailable,
                "room.accessOutcomeDirectoryUnavailable",
            ),
            (
                RoomDirectoryVisibility::Loading,
                "room.accessOutcomeDirectoryLoading",
            ),
            (
                RoomDirectoryVisibility::Failed,
                "room.accessOutcomeDirectoryFailed",
            ),
        ] {
            state.room_management.directory = visibility;
            assert_eq!(
                preview_room_access_draft(&state, &scope, RoomAccessPreviewContext::Access)
                    .outcome
                    .directory
                    .message_id,
                expected
            );
        }

        // Creation describes the proposal, never a confirmed listing.
        let create_scope = RoomAccessDraftScope::Create { session_id: 1 };
        let public = preview_create_room_access(
            &state,
            &create_scope,
            CreateRoomAccessPreviewInput {
                visibility: CreateRoomVisibility::Public,
                ..CreateRoomAccessPreviewInput::default()
            },
        );
        assert_eq!(
            public.outcome.directory.message_id,
            "room.accessOutcomeDirectoryWillBePublic"
        );
        let private = preview_create_room_access(
            &state,
            &create_scope,
            CreateRoomAccessPreviewInput::default(),
        );
        assert_eq!(
            private.outcome.directory.message_id,
            "room.accessOutcomeDirectoryWillBePrivate"
        );
    }
}
