//! The explicitly opened Home/Space scheduled-sends projection (#1160).
//!
//! This slice is the one surface deliberately allowed to carry future message
//! bodies for rooms the webview is not currently showing, and only while a
//! Ready session keeps it open. It is derived synchronously in the reducer from
//! the account store, so there is no actor, request correlation, pagination or
//! Loading/Failed state.

use std::collections::BTreeSet;

use crate::{
    effect::AppEffect,
    state::{
        AppState, ScheduledSendsListState, ScheduledSendsScope, sorted_scheduled_sends_for_rooms,
    },
};

use super::is_session_ready;

pub(crate) fn handle_open_scheduled_sends_list(
    state: &mut AppState,
    scope: ScheduledSendsScope,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    // The captured scope must match the navigation the panel opened from. An
    // unknown or inactive Space is rejected rather than silently broadening to
    // the sidebar's all-non-DM fallback.
    match &scope {
        ScheduledSendsScope::Home if state.navigation.active_space_id.is_some() => {
            return Vec::new();
        }
        ScheduledSendsScope::Space { space_id }
            if state.navigation.active_space_id.as_deref() != Some(space_id.as_str())
                || !state.spaces.iter().any(|space| space.space_id == *space_id) =>
        {
            return Vec::new();
        }
        _ => {}
    }
    state.scheduled_sends_list = ScheduledSendsListState::Open {
        scope,
        capability: state.scheduled_sends.capability.clone(),
        items: Vec::new(),
    };
    Vec::new()
}

pub(crate) fn handle_close_scheduled_sends_list(state: &mut AppState) -> Vec<AppEffect> {
    state.scheduled_sends_list = ScheduledSendsListState::Closed;
    Vec::new()
}

/// Narrow post-reducer invariant (#1160).
///
/// One place closes the projection whenever the session is no longer Ready,
/// which covers the teardown, `sync_failed_auth` and unsupported-revalidation
/// transitions without three independent call sites. It also enforces the Space
/// scope guard: a captured Space that no longer exists or is no longer the
/// active scope closes the projection *before* membership is derived, so an
/// automatic Space removal cannot broaden the list. Otherwise the list is
/// re-derived here so create/reschedule/cancel/dispatch and membership changes
/// are reflected while the panel stays open.
pub(crate) fn reconcile_scheduled_sends_list(state: &mut AppState) {
    if matches!(state.scheduled_sends_list, ScheduledSendsListState::Closed) {
        return;
    }
    if !is_session_ready(state) {
        state.scheduled_sends_list = ScheduledSendsListState::Closed;
        return;
    }
    if let ScheduledSendsListState::Open {
        scope: ScheduledSendsScope::Space { space_id },
        ..
    } = &state.scheduled_sends_list
    {
        let space_is_active =
            state.navigation.active_space_id.as_deref() == Some(space_id.as_str());
        let space_exists = state.spaces.iter().any(|space| space.space_id == *space_id);
        if !space_is_active || !space_exists {
            state.scheduled_sends_list = ScheduledSendsListState::Closed;
            return;
        }
    }
    refresh_scheduled_sends_list(state);
}

fn refresh_scheduled_sends_list(state: &mut AppState) {
    let ScheduledSendsListState::Open { scope, .. } = &state.scheduled_sends_list else {
        return;
    };
    let room_ids = match scope {
        ScheduledSendsScope::Home => None,
        ScheduledSendsScope::Space { space_id } => {
            Some(scoped_membership_room_ids(state, space_id))
        }
    };
    let items = sorted_scheduled_sends_for_rooms(&state.scheduled_sends, room_ids.as_ref());
    let capability = state.scheduled_sends.capability.clone();
    let next = ScheduledSendsListState::Open {
        scope: scope.clone(),
        capability,
        items,
    };
    if state.scheduled_sends_list != next {
        state.scheduled_sends_list = next;
    }
}

/// The sidebar's `space_rooms ∪ global_dms` room set for `space_id`.
///
/// This consumes the sidebar projection rather than recomputing membership, so
/// the panel matches what the sidebar shows: joined immediate children from
/// `SpaceSummary.child_room_ids` resolved against `state.rooms` plus the DMs
/// `RoomSummary.dm_space_ids` assigns, regardless of collapsed sections or a
/// filter box. Callers have already checked that `space_id` exists and is the
/// active scope, so the sidebar's all-non-DM fallback cannot apply.
///
/// ponytail: the whole sidebar is composed per reducer turn while the panel is
/// open; a room-set-only helper would be cheaper if that ever measures hot.
fn scoped_membership_room_ids(state: &AppState, space_id: &str) -> BTreeSet<String> {
    let sidebar = crate::compose_sidebar_for_state(state);
    debug_assert_eq!(sidebar.active_space_id.as_deref(), Some(space_id));
    sidebar
        .space_rooms
        .iter()
        .chain(sidebar.global_dms.iter())
        .map(|room| room.room_id.clone())
        .collect()
}
