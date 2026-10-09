use crate::{
    RoomDirectoryVisibility,
    effect::{AppEffect, UiEvent},
    state::{
        AppState, CreateRoomAccessSeed, OperationFailureKind, RoomAccessDraft,
        RoomAccessDraftScope, RoomHistoryVisibility, RoomJoinRule, RoomManagementOperationKind,
        RoomManagementOperationState, RoomMemberRole, RoomModerationAction, RoomSettingChange,
    },
};

use super::{is_session_ready, session_user_id};

pub(crate) fn handle_room_settings_snapshot_loaded(
    state: &mut AppState,
    room_id: String,
    settings: crate::state::RoomSettingsSnapshot,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let own_user_id = session_user_id(state).map(str::to_owned);
    let mut settings = settings;
    crate::state::refresh_room_settings_member_display_projection(
        &mut settings,
        &state.profile,
        own_user_id.as_deref(),
    );
    let pending_operation = match &state.room_management.operation {
        RoomManagementOperationState::Pending {
            room_id: pending_room_id,
            ..
        } if pending_room_id == &room_id => Some(state.room_management.operation.clone()),
        _ => None,
    };
    // #1177: this read may be a pre-send read that still carries the values a
    // property had before a write this client just accepted but has not yet
    // observed. Keep the accepted local access/history for such a property; a
    // fresh load with no pending local value (or an observation that has since
    // advanced) replaces it.
    if let Some(current) = state
        .room_management
        .settings
        .as_ref()
        .filter(|current| current.room_id == room_id)
    {
        let observed = state.room_access_observed.get(&room_id);
        if let Some(accepted) = state.room_access.get(&room_id)
            && accepted != &settings.access
            && observed.map(|observed| &observed.access) != Some(accepted)
        {
            settings.access = accepted.clone();
            settings.join_rule = accepted.join_rule.unwrap_or(settings.join_rule);
        }
        let observed_history = observed.map(|observed| observed.history_visibility);
        if settings.history_visibility != current.history_visibility
            && observed_history != Some(current.history_visibility)
        {
            settings.history_visibility = current.history_visibility;
        }
    }
    state.room_management.selected_room_id = Some(room_id.clone());
    // #1177: loading another room's settings synchronously invalidates the
    // previous room's editor lifetime and draft.
    let room_changed =
        state.room_management.active_room_editor.as_deref() != Some(room_id.as_str());
    if room_changed
        && state
            .room_management
            .draft
            .as_ref()
            .and_then(|draft| draft.scope.room_id())
            .is_some_and(|draft_room_id| draft_room_id != room_id)
    {
        state.room_management.draft = None;
    }
    state.room_management.active_room_editor = Some(room_id);
    state.room_management.settings = Some(settings);
    // A pre-send read of the same room keeps the confirmed publication it
    // already read; a room switch starts over until the read lands.
    if room_changed {
        state.room_management.directory = RoomDirectoryVisibility::Loading;
    }
    state.room_management.operation =
        pending_operation.unwrap_or(RoomManagementOperationState::Idle);
    vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
}

/// Install a confirmed room's directory publication for the open room (#1177).
pub(crate) fn handle_room_directory_visibility_observed(
    state: &mut AppState,
    room_id: String,
    visibility: RoomDirectoryVisibility,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    if state.room_management.selected_room_id.as_deref() != Some(room_id.as_str()) {
        return Vec::new();
    }
    if state.room_management.directory == visibility {
        return Vec::new();
    }
    state.room_management.directory = visibility;
    vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
}

pub(crate) fn handle_room_setting_update_requested(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    change: &RoomSettingChange,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    if !room_settings_permission_allows(state, &room_id, change) {
        state.room_management.operation = RoomManagementOperationState::Failed {
            request_id,
            room_id,
            operation: RoomManagementOperationKind::Settings,
            kind: OperationFailureKind::Forbidden,
        };
        return vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)];
    }

    // #1177: a restricted policy edit is admitted only when the pre-send read
    // verified the current allow content can be inspected and safely rewritten,
    // and every newly selected target is a joined, verified Space.
    if let RoomSettingChange::AccessPolicy(policy) = change
        && let Some(rejection) = room_access_policy_admission(state, &room_id, policy)
    {
        state.room_management.operation = RoomManagementOperationState::Failed {
            request_id,
            room_id,
            operation: RoomManagementOperationKind::Settings,
            kind: rejection,
        };
        return vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)];
    }

    state.room_management.operation = RoomManagementOperationState::Pending {
        request_id,
        room_id,
        operation: RoomManagementOperationKind::Settings,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
}

pub(crate) fn handle_room_setting_update_succeeded(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    change: &RoomSettingChange,
    settings: crate::state::RoomSettingsSnapshot,
) -> Vec<AppEffect> {
    if !room_management_operation_matches(
        state,
        request_id,
        &room_id,
        RoomManagementOperationKind::Settings,
    ) {
        return Vec::new();
    }

    let own_user_id = session_user_id(state).map(str::to_owned);
    let mut settings = settings;
    crate::state::refresh_room_settings_member_display_projection(
        &mut settings,
        &state.profile,
        own_user_id.as_deref(),
    );
    // #1177: install the accepted change as a delta, not a level copy. The
    // pre-send read behind `settings` can still lag the SDK echo of the *other*
    // property this client just saved, so that property keeps its locally
    // accepted value until the shared access observation agrees.
    let preserved = state
        .room_management
        .settings
        .as_ref()
        .filter(|current| current.room_id == room_id)
        .map(|current| {
            (
                current.history_visibility,
                current.access.clone(),
                current.join_rule,
            )
        });
    let access_is_observed = state.room_access.get(&room_id) == Some(&settings.access);
    match change {
        RoomSettingChange::AccessPolicy(_) => {
            if let Some((history, _, _)) = preserved {
                settings.history_visibility = history;
            }
        }
        RoomSettingChange::JoinRule(rule) => {
            if let Some((history, _, _)) = preserved {
                settings.history_visibility = history;
            }
            // A settable scalar rule carries no allow policy; keep the access
            // projection consistent with the new rule.
            settings.access = crate::state::RoomAccessCondition {
                join_rule: Some(*rule),
                restricted: None,
                allow_targets: Vec::new(),
            };
        }
        RoomSettingChange::HistoryVisibility(_) => {
            if !access_is_observed && let Some((_, access, join_rule)) = preserved {
                settings.access = access;
                settings.join_rule = join_rule;
            }
        }
        _ => {
            if !access_is_observed && let Some((_, access, join_rule)) = preserved.as_ref() {
                settings.access = access.clone();
                settings.join_rule = *join_rule;
            }
            if let Some((history, _, _)) = preserved {
                settings.history_visibility = history;
            }
        }
    }

    state.room_management.selected_room_id = Some(room_id.clone());
    state.room_management.operation = RoomManagementOperationState::Idle;
    let mut effects = Vec::new();
    // A successful local access change reaches the shared projection without
    // waiting for the SDK echo; the next authoritative observation replaces it.
    if matches!(
        change,
        RoomSettingChange::AccessPolicy(_) | RoomSettingChange::JoinRule(_)
    ) {
        state.room_access.insert(room_id, settings.access.clone());
        effects.push(AppEffect::EmitUiEvent(UiEvent::RoomListChanged));
    }
    state.room_management.settings = Some(settings);
    effects.push(AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged));
    effects
}

pub(crate) fn handle_room_setting_update_failed(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    kind: OperationFailureKind,
) -> Vec<AppEffect> {
    if !room_management_operation_matches(
        state,
        request_id,
        &room_id,
        RoomManagementOperationKind::Settings,
    ) {
        return Vec::new();
    }

    state.room_management.operation = RoomManagementOperationState::Failed {
        request_id,
        room_id,
        operation: RoomManagementOperationKind::Settings,
        kind,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
}

pub(crate) fn handle_room_moderation_requested(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    action: RoomModerationAction,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    if !room_moderation_permission_allows(state, &room_id, action) {
        state.room_management.operation = RoomManagementOperationState::Failed {
            request_id,
            room_id,
            operation: RoomManagementOperationKind::Moderation,
            kind: OperationFailureKind::Forbidden,
        };
        return vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)];
    }

    state.room_management.operation = RoomManagementOperationState::Pending {
        request_id,
        room_id,
        operation: RoomManagementOperationKind::Moderation,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
}

pub(crate) fn handle_room_moderation_succeeded(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    target_user_id: String,
    action: RoomModerationAction,
) -> Vec<AppEffect> {
    if !room_management_operation_matches(
        state,
        request_id,
        &room_id,
        RoomManagementOperationKind::Moderation,
    ) {
        return Vec::new();
    }

    if matches!(
        action,
        RoomModerationAction::Kick | RoomModerationAction::Ban
    ) && let Some(settings) = state.room_management.settings.as_mut()
        && settings.room_id == room_id
    {
        settings
            .members
            .retain(|member| member.user_id != target_user_id);
    }
    state.room_management.operation = RoomManagementOperationState::Idle;
    vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
}

pub(crate) fn handle_room_moderation_failed(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    kind: OperationFailureKind,
) -> Vec<AppEffect> {
    if !room_management_operation_matches(
        state,
        request_id,
        &room_id,
        RoomManagementOperationKind::Moderation,
    ) {
        return Vec::new();
    }

    state.room_management.operation = RoomManagementOperationState::Failed {
        request_id,
        room_id,
        operation: RoomManagementOperationKind::Moderation,
        kind,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
}

pub(crate) fn handle_room_member_role_update_requested(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    if !room_role_permission_allows(state, &room_id) {
        state.room_management.operation = RoomManagementOperationState::Failed {
            request_id,
            room_id,
            operation: RoomManagementOperationKind::Roles,
            kind: OperationFailureKind::Forbidden,
        };
        return vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)];
    }

    state.room_management.operation = RoomManagementOperationState::Pending {
        request_id,
        room_id,
        operation: RoomManagementOperationKind::Roles,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
}

pub(crate) fn handle_room_member_role_update_succeeded(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    target_user_id: String,
    power_level: i64,
) -> Vec<AppEffect> {
    if !room_management_operation_matches(
        state,
        request_id,
        &room_id,
        RoomManagementOperationKind::Roles,
    ) {
        return Vec::new();
    }

    if let Some(settings) = state.room_management.settings.as_mut()
        && settings.room_id == room_id
        && let Some(member) = settings
            .members
            .iter_mut()
            .find(|member| member.user_id == target_user_id)
    {
        member.power_level = Some(power_level);
        member.role = RoomMemberRole::from_power_level(Some(power_level));
    }
    state.room_management.operation = RoomManagementOperationState::Idle;
    vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
}

pub(crate) fn handle_room_member_role_update_failed(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    kind: OperationFailureKind,
) -> Vec<AppEffect> {
    if !room_management_operation_matches(
        state,
        request_id,
        &room_id,
        RoomManagementOperationKind::Roles,
    ) {
        return Vec::new();
    }

    state.room_management.operation = RoomManagementOperationState::Failed {
        request_id,
        room_id,
        operation: RoomManagementOperationKind::Roles,
        kind,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
}

pub(crate) fn handle_room_access_draft_rule_set(
    state: &mut AppState,
    scope: RoomAccessDraftScope,
    rule: Option<RoomJoinRule>,
) -> Vec<AppEffect> {
    let Some(draft) = room_access_draft_for_scope(state, &scope) else {
        return Vec::new();
    };
    draft.set_rule(rule);
    vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
}

pub(crate) fn handle_room_access_draft_allow_targets_set(
    state: &mut AppState,
    scope: RoomAccessDraftScope,
    allow_targets: Vec<String>,
) -> Vec<AppEffect> {
    let Some(draft) = room_access_draft_for_scope(state, &scope) else {
        return Vec::new();
    };
    draft.set_allow_targets(allow_targets);
    vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
}

pub(crate) fn handle_room_access_draft_history_set(
    state: &mut AppState,
    scope: RoomAccessDraftScope,
    history: Option<RoomHistoryVisibility>,
) -> Vec<AppEffect> {
    let Some(draft) = room_access_draft_for_scope(state, &scope) else {
        return Vec::new();
    };
    draft.set_history(history);
    vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
}

pub(crate) fn handle_room_access_draft_opened(
    state: &mut AppState,
    scope: RoomAccessDraftScope,
    create: Option<CreateRoomAccessSeed>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    let mut draft = RoomAccessDraft::new(scope.clone());
    match &scope {
        RoomAccessDraftScope::Room { room_id } => {
            let is_loaded = state.room_management.selected_room_id.as_deref() == Some(room_id)
                || state
                    .room_management
                    .settings
                    .as_ref()
                    .is_some_and(|settings| settings.room_id == *room_id);
            if !is_loaded {
                return Vec::new();
            }
            state.room_management.active_room_editor = Some(room_id.clone());
        }
        RoomAccessDraftScope::Create { session_id } => {
            if *session_id == 0 {
                // Create editor lifetimes are never session zero (#1177).
                return Vec::new();
            }
            state.room_management.active_create_session = Some(*session_id);
            if let Some(create) = create {
                draft.seed_create_selection(
                    create.visibility,
                    create.invited_only,
                    create.parent_space_id.as_deref(),
                );
            }
        }
    }
    state.room_management.draft = Some(draft);
    vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
}

pub(crate) fn handle_room_access_draft_reset(
    state: &mut AppState,
    scope: RoomAccessDraftScope,
) -> Vec<AppEffect> {
    if !is_session_ready(state) || !active_editor_matches(state, &scope) {
        return Vec::new();
    }
    if state
        .room_management
        .draft
        .as_ref()
        .is_some_and(|draft| draft.scope == scope)
    {
        state.room_management.draft = None;
    }
    // A create session is retired so a later old mutation cannot recreate it.
    // A room editor stays admitted for its room until another room loads.
    if matches!(scope, RoomAccessDraftScope::Create { .. }) {
        state.room_management.active_create_session = None;
    }
    vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
}

/// Whether `scope` names the editor lifetime Rust has admitted (#1177).
fn active_editor_matches(state: &AppState, scope: &RoomAccessDraftScope) -> bool {
    match scope {
        RoomAccessDraftScope::Room { room_id } => {
            state.room_management.active_room_editor.as_deref() == Some(room_id.as_str())
        }
        RoomAccessDraftScope::Create { session_id } => {
            *session_id != 0 && state.room_management.active_create_session == Some(*session_id)
        }
    }
}

/// The draft for `scope`. A room scope is admitted only for the loaded room and
/// the active editor; a create scope only for the session the Open admitted, so
/// a retired editor's command is ignored rather than applied or recreated.
fn room_access_draft_for_scope<'a>(
    state: &'a mut AppState,
    scope: &RoomAccessDraftScope,
) -> Option<&'a mut RoomAccessDraft> {
    if !is_session_ready(state) || !active_editor_matches(state, scope) {
        return None;
    }
    if scope.room_id().is_none() {
        // A create draft is established by its Open, never lazily recreated.
        return state
            .room_management
            .draft
            .as_mut()
            .filter(|draft| &draft.scope == scope);
    }
    let room_id = scope.room_id()?;
    let is_loaded = state.room_management.selected_room_id.as_deref() == Some(room_id)
        || state
            .room_management
            .settings
            .as_ref()
            .is_some_and(|settings| settings.room_id == room_id);
    if !is_loaded {
        return None;
    }
    let needs_new = state
        .room_management
        .draft
        .as_ref()
        .is_none_or(|draft| draft.scope != *scope);
    if needs_new {
        state.room_management.draft = Some(RoomAccessDraft::new(scope.clone()));
    }
    state.room_management.draft.as_mut()
}

// --- Private helpers ---

/// The combined admission verdict for a requested access policy (#1177): the
/// confirmed condition must be inspectable and rewrite-safe, and every newly
/// selected target must be a joined, verified Space.
fn room_access_policy_admission(
    state: &AppState,
    room_id: &str,
    policy: &crate::state::RoomAccessPolicy,
) -> Option<OperationFailureKind> {
    let settings = state
        .room_management
        .settings
        .as_ref()
        .filter(|settings| settings.room_id == room_id)?;
    settings.access_policy_rejection(policy).or_else(|| {
        let joined_space_ids: std::collections::BTreeSet<&str> = state
            .spaces
            .iter()
            .map(|space| space.space_id.as_str())
            .collect();
        crate::state::access_policy_target_rejection(settings, policy, |target| {
            joined_space_ids.contains(target)
        })
    })
}

fn room_settings_permission_allows(
    state: &AppState,
    room_id: &str,
    change: &RoomSettingChange,
) -> bool {
    state
        .room_management
        .settings
        .as_ref()
        .filter(|settings| settings.room_id == room_id)
        .is_some_and(|settings| settings.permissions.allows_setting_change(change))
}

fn room_role_permission_allows(state: &AppState, room_id: &str) -> bool {
    state
        .room_management
        .settings
        .as_ref()
        .filter(|settings| settings.room_id == room_id)
        .is_some_and(|settings| settings.permissions.can_edit_roles)
}

fn room_moderation_permission_allows(
    state: &AppState,
    room_id: &str,
    action: RoomModerationAction,
) -> bool {
    let Some(permissions) = state
        .room_management
        .settings
        .as_ref()
        .filter(|settings| settings.room_id == room_id)
        .map(|settings| settings.permissions)
    else {
        return false;
    };

    match action {
        RoomModerationAction::Kick => permissions.can_kick,
        RoomModerationAction::Ban => permissions.can_ban,
        RoomModerationAction::Unban => permissions.can_unban,
    }
}

fn room_management_operation_matches(
    state: &AppState,
    request_id: u64,
    room_id: &str,
    operation: RoomManagementOperationKind,
) -> bool {
    matches!(
        &state.room_management.operation,
        RoomManagementOperationState::Pending {
            request_id: current_request_id,
            room_id: current_room_id,
            operation: current_operation,
        } if *current_request_id == request_id
            && current_room_id == room_id
            && *current_operation == operation
    )
}
