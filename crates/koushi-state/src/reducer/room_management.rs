use crate::{
    effect::{AppEffect, UiEvent},
    state::{
        AppState, OperationFailureKind, RoomManagementOperationKind, RoomManagementOperationState,
        RoomMemberRole, RoomModerationAction,
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
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    if !room_settings_permission_allows(state, &room_id) {
        state.room_management.operation = RoomManagementOperationState::Failed {
            request_id,
            room_id,
            operation: RoomManagementOperationKind::Settings,
            kind: OperationFailureKind::Forbidden,
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

// --- Private helpers ---

fn room_settings_permission_allows(state: &AppState, room_id: &str) -> bool {
    state
        .room_management
        .settings
        .as_ref()
        .filter(|settings| settings.room_id == room_id)
        .is_some_and(|settings| settings.permissions.can_edit_settings)
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
