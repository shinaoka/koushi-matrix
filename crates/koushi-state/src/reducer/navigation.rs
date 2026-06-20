use crate::{
    effect::{AppEffect, UiEvent},
    state::{AppState, RoomListFilter},
};

use super::{
    apply_space_order, clear_active_room_for_navigation, first_default_room_id,
    is_complete_space_order, is_session_ready, preferred_room_id_in_space,
    recompute_room_list_projection, remember_active_room_for_current_space,
    select_active_room_for_navigation,
};

pub(crate) fn handle_invite_list_updated(
    state: &mut AppState,
    invites: Vec<crate::state::InvitePreview>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    state.invites = invites;
    if state.room_list.active_filter == RoomListFilter::Invites {
        recompute_room_list_projection(state);
    }
    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
}

pub(crate) fn handle_select_space(
    state: &mut AppState,
    space_id: Option<String>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    remember_active_room_for_current_space(state);
    let previous_room_id = state.navigation.active_room_id.clone();
    state.navigation.active_space_id =
        space_id.filter(|space_id| state.spaces.iter().any(|space| space.space_id == *space_id));
    recompute_room_list_projection(state);
    let target_room_id = match state.navigation.active_space_id.as_deref() {
        Some(space_id) => preferred_room_id_in_space(state, space_id),
        None => first_default_room_id(state),
    };
    let mut effects = vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)];
    if target_room_id != state.navigation.active_room_id {
        match target_room_id {
            Some(room_id) => {
                select_active_room_for_navigation(state, &mut effects, room_id);
            }
            None => {
                if let Some(previous_room_id) = previous_room_id {
                    clear_active_room_for_navigation(state, &mut effects, previous_room_id);
                }
            }
        }
    }
    remember_active_room_for_current_space(state);
    effects
}

pub(crate) fn handle_reorder_spaces(
    state: &mut AppState,
    space_ids: Vec<String>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    if !is_complete_space_order(&state.spaces, &space_ids) {
        return Vec::new();
    }

    state.navigation.space_order = space_ids;
    apply_space_order(&mut state.spaces, &state.navigation.space_order);
    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
}

pub(crate) fn handle_select_room(state: &mut AppState, room_id: String) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let Some(selected_room) = state
        .rooms
        .iter()
        .find(|room| room.room_id == room_id)
        .cloned()
    else {
        return Vec::new();
    };

    remember_active_room_for_current_space(state);
    let previous_active_space_id = state.navigation.active_space_id.clone();
    if !selected_room.is_dm {
        let active_space_contains_selected_room = state
            .navigation
            .active_space_id
            .as_ref()
            .is_some_and(|space_id| selected_room.parent_space_ids.contains(space_id));
        if !active_space_contains_selected_room {
            state.navigation.active_space_id = selected_room.parent_space_ids.first().cloned();
        }
    }
    let mut effects = Vec::new();
    if previous_active_space_id != state.navigation.active_space_id {
        recompute_room_list_projection(state);
        effects.push(AppEffect::EmitUiEvent(UiEvent::RoomListChanged));
    }
    select_active_room_for_navigation(state, &mut effects, room_id);
    remember_active_room_for_current_space(state);
    effects
}
