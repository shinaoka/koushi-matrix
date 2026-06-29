use crate::{
    effect::{AppEffect, UiEvent},
    state::{AppState, NavigationState, RoomListFilter, TimelinePersistedViewport},
};

use super::{
    apply_space_order,
    avatar::{collect_known_avatar_thumbnails, preserve_avatar_thumbnail},
    clear_active_room_for_navigation, first_default_room_id, is_complete_space_order,
    is_session_ready, preferred_room_id_in_space, recompute_room_list_projection,
    remember_active_room_for_current_space, select_active_room_for_navigation,
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

    state.navigation = normalize_navigation_state(navigation);
    recompute_room_list_projection(state);
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

    let viewport = TimelinePersistedViewport::Anchored {
        event_id: anchor.event_id.clone(),
        edge: anchor.edge,
        offset_px: anchor.offset_px,
        updated_at_ms: anchor.updated_at_ms,
    };
    let should_update = state.navigation.room_scroll_anchors.get(&room_id) != Some(&anchor)
        || state.navigation.room_viewports.get(&room_id) != Some(&viewport);
    if !should_update {
        return Vec::new();
    }

    state
        .navigation
        .room_viewports
        .insert(room_id.clone(), viewport);
    state.navigation.room_scroll_anchors.insert(room_id, anchor);
    prune_room_scroll_anchors(&mut state.navigation.room_scroll_anchors);
    prune_room_viewports_and_legacy_anchors(&mut state.navigation);
    Vec::new()
}

pub(crate) fn handle_timeline_viewport_updated(
    state: &mut AppState,
    room_id: String,
    viewport: TimelinePersistedViewport,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let should_update = state.navigation.room_viewports.get(&room_id) != Some(&viewport);
    if !should_update {
        return Vec::new();
    }

    if let Some(anchor) = viewport.as_scroll_anchor() {
        state
            .navigation
            .room_scroll_anchors
            .insert(room_id.clone(), anchor);
    } else {
        state.navigation.room_scroll_anchors.remove(&room_id);
    }
    state.navigation.room_viewports.insert(room_id, viewport);
    prune_room_scroll_anchors(&mut state.navigation.room_scroll_anchors);
    prune_room_viewports_and_legacy_anchors(&mut state.navigation);
    Vec::new()
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
    for (room_id, anchor) in &navigation.room_scroll_anchors {
        navigation
            .room_viewports
            .entry(room_id.clone())
            .or_insert_with(|| TimelinePersistedViewport::Anchored {
                event_id: anchor.event_id.clone(),
                edge: anchor.edge,
                offset_px: anchor.offset_px,
                updated_at_ms: anchor.updated_at_ms,
            });
    }
    prune_room_viewports_and_legacy_anchors(&mut navigation);
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

fn prune_room_viewports(
    room_viewports: &mut std::collections::BTreeMap<String, TimelinePersistedViewport>,
) -> Vec<String> {
    if room_viewports.len() <= MAX_ROOM_SCROLL_ANCHORS {
        return Vec::new();
    }

    let mut ordered_room_ids: Vec<(String, u64)> = room_viewports
        .iter()
        .map(|(room_id, viewport)| (room_id.clone(), viewport.updated_at_ms()))
        .collect();
    ordered_room_ids.sort_by(
        |(left_room_id, left_updated_at_ms), (right_room_id, right_updated_at_ms)| {
            left_updated_at_ms
                .cmp(right_updated_at_ms)
                .then_with(|| left_room_id.cmp(right_room_id))
        },
    );
    let overflow = room_viewports.len().saturating_sub(MAX_ROOM_SCROLL_ANCHORS);

    let mut removed_room_ids = Vec::new();
    for (room_id, _) in ordered_room_ids.into_iter().take(overflow) {
        room_viewports.remove(&room_id);
        removed_room_ids.push(room_id);
    }
    removed_room_ids
}

fn prune_room_viewports_and_legacy_anchors(navigation: &mut NavigationState) {
    for room_id in prune_room_viewports(&mut navigation.room_viewports) {
        navigation.room_scroll_anchors.remove(&room_id);
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
