use crate::{
    effect::{AppEffect, UiEvent},
    state::{
        AppState, OperationFailureKind, RoomAccessDraft, RoomAccessDraftScope,
        RoomHistoryVisibility, RoomJoinRule, RoomManagementOperationKind,
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
    state.room_management.selected_room_id = Some(room_id);
    state.room_management.settings = Some(settings);
    state.room_management.operation =
        pending_operation.unwrap_or(RoomManagementOperationState::Idle);
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
    // verified the current allow content can be inspected and safely rewritten.
    if let RoomSettingChange::AccessPolicy(policy) = change
        && let Some(kind) = state
            .room_management
            .settings
            .as_ref()
            .filter(|settings| settings.room_id == room_id)
            .and_then(|settings| settings.access_policy_rejection(policy))
    {
        state.room_management.operation = RoomManagementOperationState::Failed {
            request_id,
            room_id,
            operation: RoomManagementOperationKind::Settings,
            kind,
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
    state.room_management.selected_room_id = Some(room_id);
    state.room_management.settings = Some(settings);
    state.room_management.operation = RoomManagementOperationState::Idle;
    vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
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

pub(crate) fn handle_room_access_draft_reset(
    state: &mut AppState,
    scope: RoomAccessDraftScope,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    if state
        .room_management
        .draft
        .as_ref()
        .is_some_and(|draft| draft.scope == scope)
    {
        state.room_management.draft = None;
        return vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)];
    }
    Vec::new()
}

/// The draft for `scope`, created (or replaced) when the editor moves to a
/// different room or create session. A room scope is admitted only for the
/// loaded room, so a stale editor's command is ignored rather than applied to
/// another room.
fn room_access_draft_for_scope<'a>(
    state: &'a mut AppState,
    scope: &RoomAccessDraftScope,
) -> Option<&'a mut RoomAccessDraft> {
    if !is_session_ready(state) {
        return None;
    }
    if let Some(room_id) = scope.room_id() {
        let is_loaded = state.room_management.selected_room_id.as_deref() == Some(room_id)
            || state
                .room_management
                .settings
                .as_ref()
                .is_some_and(|settings| settings.room_id == room_id);
        if !is_loaded {
            return None;
        }
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
