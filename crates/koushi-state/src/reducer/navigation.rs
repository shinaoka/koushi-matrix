use crate::{
    effect::{AppEffect, UiEvent},
    state::{
        AppState, EventNavigationFailureKind, EventNavigationSource, EventNavigationState,
        NavigationState, RoomListFilter, SearchScope, SearchState, SpaceConversationSurface,
    },
};

use super::{
    apply_space_order_preference,
    avatar::{collect_known_avatar_thumbnails, preserve_avatar_thumbnail},
    clear_active_room_for_navigation, is_complete_space_order, is_session_ready,
    preferred_selection_in_space, recompute_room_list_projection,
    remember_active_room_for_current_space, reorder_visible_space_order,
    select_active_room_for_navigation,
};

const MAX_ROOM_SCROLL_ANCHORS: usize = 200;

pub(crate) fn handle_invite_list_updated(
    state: &mut AppState,
    mut invites: Vec<crate::state::InvitePreview>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    preserve_known_avatar_thumbnails(state, &mut invites);
    state.invites = invites;
    if state.room_list.active_filter == RoomListFilter::Invites {
        recompute_room_list_projection(state);
    }
    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
}

pub(crate) fn handle_navigation_loaded(
    state: &mut AppState,
    navigation: NavigationState,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let previous_active_space_id = state.navigation.active_space_id.clone();
    state.navigation = normalize_navigation_state(navigation);
    super::normalize_space_order_preference(&mut state.navigation.space_order);
    apply_space_order_preference(&mut state.spaces, &state.navigation.space_order);
    let space_members_changed = previous_active_space_id != state.navigation.active_space_id
        && super::space_members::handle_selected(state, state.navigation.active_space_id.clone());
    recompute_room_list_projection(state);
    let mut effects = vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)];
    if space_members_changed {
        effects.push(AppEffect::EmitUiEvent(UiEvent::SpaceMembersChanged));
    }
    effects
}

pub(crate) fn handle_navigation_preference_updated(
    state: &mut AppState,
    update: crate::state::NavigationPreferenceUpdate,
) -> Vec<AppEffect> {
    if !is_session_ready(state) || !state.navigation.apply_preference_update(update) {
        return Vec::new();
    }
    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
}

pub(crate) fn handle_timeline_scroll_anchor_updated(
    state: &mut AppState,
    room_id: String,
    anchor: crate::state::TimelineScrollAnchor,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let should_update = state.navigation.room_scroll_anchors.get(&room_id) != Some(&anchor);
    if !should_update {
        return Vec::new();
    }

    state.navigation.room_scroll_anchors.insert(room_id, anchor);
    prune_room_scroll_anchors(&mut state.navigation.room_scroll_anchors);
    Vec::new()
}

/// #161: enter event-anchored main-pane mode. Guarded: session must be ready
/// and `room_id` must be the active, known room. The main pane then renders the
/// event-focused timeline (core routes the `TimelineKind::Focused` subscription;
/// this reducer only owns the mode state).
pub(crate) fn handle_enter_anchored_timeline(
    state: &mut AppState,
    room_id: String,
    event_id: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    if state.navigation.active_room_id.as_deref() != Some(room_id.as_str()) {
        return Vec::new();
    }
    if !state.rooms.iter().any(|room| room.room_id == room_id) {
        return Vec::new();
    }

    let anchor = crate::state::MainTimelineAnchor { event_id };
    if state.navigation.main_timeline_anchor.as_ref() == Some(&anchor) {
        return Vec::new();
    }
    state.navigation.main_timeline_anchor = Some(anchor);
    Vec::new()
}

/// #161: return the main pane to the live timeline (live-edge control). Guarded:
/// session ready and `room_id` is the active room. No-op when already live.
pub(crate) fn handle_return_main_timeline_to_live(
    state: &mut AppState,
    room_id: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    if state.navigation.active_room_id.as_deref() != Some(room_id.as_str()) {
        return Vec::new();
    }
    let event_navigation_active = !matches!(
        state.navigation.event_navigation,
        EventNavigationState::Idle
    );
    if state.navigation.main_timeline_anchor.is_none() && !event_navigation_active {
        return Vec::new();
    }
    state.navigation.main_timeline_anchor = None;
    state.navigation.event_navigation = EventNavigationState::Idle;
    Vec::new()
}

pub(crate) fn handle_event_navigation_started(
    state: &mut AppState,
    source: EventNavigationSource,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let generation = state
        .navigation
        .event_navigation
        .generation()
        .saturating_add(1);
    state.navigation.event_navigation = EventNavigationState::Opening { generation, source };
    Vec::new()
}

pub(crate) fn handle_event_navigation_anchored(
    state: &mut AppState,
    generation: u64,
) -> Vec<AppEffect> {
    let Some(source) = event_navigation_opening_source(state, generation) else {
        return Vec::new();
    };
    state.navigation.event_navigation = EventNavigationState::Anchored { generation, source };
    Vec::new()
}

pub(crate) fn handle_event_navigation_live_fallback(
    state: &mut AppState,
    generation: u64,
) -> Vec<AppEffect> {
    let Some(source) = event_navigation_opening_source(state, generation) else {
        return Vec::new();
    };
    state.navigation.event_navigation = EventNavigationState::LiveFallback { generation, source };
    Vec::new()
}

pub(crate) fn handle_event_navigation_failed(
    state: &mut AppState,
    generation: u64,
    kind: EventNavigationFailureKind,
) -> Vec<AppEffect> {
    let Some(source) = event_navigation_opening_source(state, generation) else {
        return Vec::new();
    };
    state.navigation.event_navigation = EventNavigationState::Failed {
        generation,
        source,
        failure_kind: kind,
    };
    Vec::new()
}

pub(crate) fn handle_event_navigation_cleared(state: &mut AppState) -> Vec<AppEffect> {
    state.navigation.event_navigation = EventNavigationState::Idle;
    Vec::new()
}

fn event_navigation_opening_source(
    state: &AppState,
    generation: u64,
) -> Option<EventNavigationSource> {
    match state.navigation.event_navigation {
        EventNavigationState::Opening {
            generation: current,
            source,
        } if current == generation => Some(source),
        _ => None,
    }
}

fn preserve_known_avatar_thumbnails(
    state: &AppState,
    next_invites: &mut [crate::state::InvitePreview],
) {
    let known_thumbnails = collect_known_avatar_thumbnails(state, true);

    for invite in next_invites {
        preserve_avatar_thumbnail(&known_thumbnails, &mut invite.avatar);
    }
}

fn normalize_navigation_state(mut navigation: NavigationState) -> NavigationState {
    prune_room_scroll_anchors(&mut navigation.room_scroll_anchors);
    navigation.event_navigation = EventNavigationState::Idle;
    navigation
}

fn prune_room_scroll_anchors(
    room_scroll_anchors: &mut std::collections::BTreeMap<
        String,
        crate::state::TimelineScrollAnchor,
    >,
) {
    if room_scroll_anchors.len() <= MAX_ROOM_SCROLL_ANCHORS {
        return;
    }

    let mut ordered_room_ids: Vec<(String, u64)> = room_scroll_anchors
        .iter()
        .map(|(room_id, anchor)| (room_id.clone(), anchor.updated_at_ms))
        .collect();
    ordered_room_ids.sort_by(
        |(left_room_id, left_updated_at_ms), (right_room_id, right_updated_at_ms)| {
            left_updated_at_ms
                .cmp(right_updated_at_ms)
                .then_with(|| left_room_id.cmp(right_room_id))
        },
    );
    let overflow = room_scroll_anchors
        .len()
        .saturating_sub(MAX_ROOM_SCROLL_ANCHORS);

    for (room_id, _) in ordered_room_ids.into_iter().take(overflow) {
        room_scroll_anchors.remove(&room_id);
    }
}

fn close_current_room_search_for_room_change(
    state: &mut AppState,
    next_room_id: Option<&str>,
    effects: &mut Vec<AppEffect>,
) {
    let should_close = match &state.search {
        SearchState::Editing { scope, .. }
        | SearchState::TooShort { scope, .. }
        | SearchState::Searching { scope, .. }
        | SearchState::Results { scope, .. }
        | SearchState::Failed { scope, .. } => match scope {
            SearchScope::CurrentRoom { room_id } => next_room_id != Some(room_id.as_str()),
            SearchScope::CurrentSpace { .. } | SearchScope::AllRooms => false,
        },
        SearchState::Closed => false,
    };

    if should_close {
        state.search = SearchState::Closed;
        effects.push(AppEffect::EmitUiEvent(UiEvent::SearchChanged));
    }
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
    let selected_space_id = state.navigation.active_space_id.clone();
    // #445: restore the surface this Space was last on BEFORE projecting its room
    // list, so the projection is computed for the surface the user left rather
    // than whatever surface the previous Space happened to be showing. Only an
    // actual remembered entry may move the filter; a Space with no memory leaves
    // the current filter alone.
    let restored_selection = selected_space_id.as_deref().map(|space_id| {
        let remembered = state
            .navigation
            .last_selection_by_space_id
            .contains_key(space_id);
        (preferred_selection_in_space(state, space_id), remembered)
    });
    if let Some((selection, true)) = restored_selection.as_ref() {
        state.room_list.active_filter = match selection.surface {
            SpaceConversationSurface::Dms => RoomListFilter::People,
            SpaceConversationSurface::Rooms => RoomListFilter::Rooms,
        };
    }
    let space_members_changed = super::space_members::handle_selected(state, selected_space_id);
    recompute_room_list_projection(state);
    if state.navigation.active_space_id.is_none() {
        let mut effects = vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)];
        if space_members_changed {
            effects.push(AppEffect::EmitUiEvent(UiEvent::SpaceMembersChanged));
        }
        close_current_room_search_for_room_change(state, None, &mut effects);
        if let Some(previous_room_id) = previous_room_id {
            clear_active_room_for_navigation(state, &mut effects, previous_room_id);
        }
        return effects;
    }
    let target_room_id = restored_selection.and_then(|(selection, _)| selection.room_id);
    let mut effects = vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)];
    if space_members_changed {
        effects.push(AppEffect::EmitUiEvent(UiEvent::SpaceMembersChanged));
    }
    if target_room_id != state.navigation.active_room_id {
        match target_room_id {
            Some(room_id) => {
                close_current_room_search_for_room_change(
                    state,
                    Some(room_id.as_str()),
                    &mut effects,
                );
                select_active_room_for_navigation(state, &mut effects, room_id);
            }
            None => {
                close_current_room_search_for_room_change(state, None, &mut effects);
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

    if !reorder_visible_space_order(&mut state.navigation.space_order, &state.spaces, &space_ids) {
        return Vec::new();
    }
    apply_space_order_preference(&mut state.spaces, &state.navigation.space_order);
    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
}

pub(crate) fn handle_space_order_preference_removed(
    state: &mut AppState,
    space_id: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let previous_len = state.navigation.space_order.len();
    state
        .navigation
        .space_order
        .retain(|candidate| candidate != &space_id);
    if state.navigation.space_order.len() == previous_len {
        return Vec::new();
    }

    apply_space_order_preference(&mut state.spaces, &state.navigation.space_order);
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
        let space_members_changed =
            super::space_members::handle_selected(state, state.navigation.active_space_id.clone());
        recompute_room_list_projection(state);
        effects.push(AppEffect::EmitUiEvent(UiEvent::RoomListChanged));
        if space_members_changed {
            effects.push(AppEffect::EmitUiEvent(UiEvent::SpaceMembersChanged));
        }
    }
    close_current_room_search_for_room_change(state, Some(room_id.as_str()), &mut effects);
    if state.navigation.active_room_id.as_deref() == Some(room_id.as_str())
        && state.timeline.room_id.as_deref() == Some(room_id.as_str())
    {
        remember_active_room_for_current_space(state);
        return effects;
    }
    select_active_room_for_navigation(state, &mut effects, room_id);
    remember_active_room_for_current_space(state);
    effects
}
