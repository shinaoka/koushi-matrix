//! #1803: marking a room read is a canonical clear (`Candidate --> Idle:
//! RoomMarkedRead`), so the unread counts, the badge and the candidate must move
//! in the same reduce. No room-list update may be needed to finish the clear.

use super::*;
use crate::state::{NativeAttentionDispatchState, RoomSummary};

const ACTIVE_ROOM: &str = "!active:example.invalid";
const NOTIFIED_ROOM: &str = "!other:example.invalid";
const STALE_ROOM: &str = "!stale:example.invalid";

fn active_room() -> RoomSummary {
    test_room(ACTIVE_ROOM, None)
}

fn read_room() -> RoomSummary {
    test_room(NOTIFIED_ROOM, None)
}

fn unread_room() -> RoomSummary {
    let mut room = read_room();
    room.unread_count = 1;
    room.notification_count = 1;
    room.recency_stamp = Some(43);
    room.latest_event = Some(latest_event("$notified:example.invalid", 43));
    room
}

fn room_list_update(state: &mut AppState, rooms: Vec<RoomSummary>) -> Vec<AppEffect> {
    reduce(
        state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms,
        },
    )
}

/// The reported precondition: a live candidate is published for a room that is
/// not the active one, and nobody has observed that room as read yet.
fn state_with_live_candidate() -> AppState {
    let mut state = ready_state();
    state.navigation.active_room_id = Some(ACTIVE_ROOM.to_owned());
    room_list_update(&mut state, vec![active_room(), read_room()]);
    room_list_update(&mut state, vec![active_room(), unread_room()]);

    assert!(
        state.native_attention.summary.candidate.is_some(),
        "precondition: the live room-list update raises a candidate"
    );
    assert_eq!(state.native_attention.summary.badge_count, 1);
    state
}

fn mark_room_read(state: &mut AppState) -> Vec<AppEffect> {
    reduce(
        state,
        AppAction::RoomMarkedAsReadRequested {
            request_id: 1,
            room_id: NOTIFIED_ROOM.to_owned(),
            event_id: "$notified:example.invalid".to_owned(),
        },
    );
    reduce(
        state,
        AppAction::RoomMarkedAsReadSucceeded {
            request_id: 1,
            room_id: NOTIFIED_ROOM.to_owned(),
        },
    )
}

#[test]
fn marking_the_alerting_room_read_clears_counts_badge_and_candidate_together() {
    let mut state = state_with_live_candidate();

    let effects = mark_room_read(&mut state);

    // No room-list update stands between the read and these assertions.
    assert_eq!(
        state
            .rooms
            .iter()
            .find(|room| room.room_id == NOTIFIED_ROOM)
            .map(|room| room.unread_count),
        Some(0),
        "the read room's unread count is cleared"
    );
    assert_eq!(
        state.native_attention.summary.candidate, None,
        "RoomMarkedRead clears the candidate in the same reduce"
    );
    assert_eq!(
        state.native_attention.notification, None,
        "a cleared candidate must not leave its payload behind"
    );
    assert_eq!(
        state.native_attention.dispatch,
        NativeAttentionDispatchState::Idle
    );
    assert_eq!(
        state.native_attention.summary.badge_count, 0,
        "the badge follows the cleared counts in the same reduce"
    );
    assert!(
        effects.contains(&AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged)),
        "the badge and banner surfaces must be refreshed by this reduce: {effects:?}"
    );
}

#[test]
fn an_unrelated_room_list_update_after_the_read_does_not_resurrect_the_candidate() {
    let mut state = state_with_live_candidate();
    mark_room_read(&mut state);

    // Activity in the active room: no unread count rises anywhere, so this
    // observation raises nothing of its own (#1799).
    let mut active = active_room();
    let mut own_event = latest_event("$own:example.invalid", 44);
    own_event.sender_id = Some("@alice:example.invalid".to_owned());
    active.latest_event = Some(own_event);
    active.recency_stamp = Some(44);
    room_list_update(&mut state, vec![active, read_room()]);

    assert_eq!(
        state.native_attention.summary.candidate, None,
        "the read room's candidate must not come back"
    );
    assert_eq!(state.native_attention.notification, None);
    assert_eq!(state.native_attention.summary.badge_count, 0);
}

/// The candidate belongs to `NOTIFIED_ROOM`, while `STALE_ROOM` simply holds
/// unread counts the client learned with the first snapshot and never raised.
fn state_with_an_unraised_unread_room() -> AppState {
    let mut state = ready_state();
    state.navigation.active_room_id = Some(ACTIVE_ROOM.to_owned());
    let mut stale = test_room(STALE_ROOM, None);
    stale.unread_count = 1;
    stale.notification_count = 1;
    stale.recency_stamp = Some(40);
    stale.latest_event = Some(latest_event("$stale:example.invalid", 40));
    room_list_update(
        &mut state,
        vec![active_room(), stale.clone(), unread_room()],
    );
    let mut raised = unread_room();
    raised.unread_count = 2;
    room_list_update(&mut state, vec![active_room(), stale, raised]);

    assert_eq!(
        state
            .native_attention
            .summary
            .candidate
            .as_ref()
            .map(|candidate| candidate.room_display_name.as_str()),
        Some(NOTIFIED_ROOM),
        "precondition: the raised candidate belongs to the alerted room"
    );
    state
}

#[test]
fn marking_another_room_read_keeps_the_published_candidate() {
    let mut state = state_with_an_unraised_unread_room();
    let candidate = state.native_attention.summary.candidate.clone();

    reduce(
        &mut state,
        AppAction::RoomMarkedAsReadSucceeded {
            request_id: 2,
            room_id: STALE_ROOM.to_owned(),
        },
    );

    assert_eq!(
        state.native_attention.summary.candidate, candidate,
        "reading another room leaves the alerted room's attention standing"
    );
    assert_eq!(state.native_attention.summary.badge_count, 2);
}
