//! #2030 B2: a confirmed main-room read boundary settles the room list.
//!
//! Core proves the boundary is the newest event the room's main timeline holds
//! and publishes it as `ConfirmedReadBoundarySettled`. The reducer publishes it
//! as the room's read position whatever room is in the foreground, and drops the
//! main unread and attention only while the room list still shows that event. A
//! list that moved on keeps its counts, and an explicit mark-unread or a thread's
//! unread is a different scope that reading the room does not touch.

use super::*;
use crate::state::{RoomAttentionProjection, RoomSummary, room_attention_projection};

const READ_ROOM: &str = "!read:example.invalid";
const OTHER_ROOM: &str = "!other:example.invalid";
const FOREGROUND_ROOM: &str = "!foreground:example.invalid";

fn unread_room(latest_event_id: &str) -> RoomSummary {
    let mut room = test_room(READ_ROOM, None);
    room.unread_count = 3;
    room.notification_count = 2;
    room.highlight_count = 1;
    room.recency_stamp = Some(43);
    room.latest_event = Some(latest_event(latest_event_id, 43_000));
    room
}

/// Unread too, so "the settlement touched no other room" is a real assertion.
fn foreground_room() -> RoomSummary {
    let mut room = test_room(FOREGROUND_ROOM, None);
    room.unread_count = 5;
    room.notification_count = 4;
    room.highlight_count = 2;
    room.recency_stamp = Some(50);
    room.latest_event = Some(latest_event("$foreground:example.invalid", 50_000));
    room
}

fn unrelated_unread_room() -> RoomSummary {
    let mut room = test_room(OTHER_ROOM, None);
    room.unread_count = 2;
    room.notification_count = 1;
    room.recency_stamp = Some(40);
    room.latest_event = Some(latest_event("$other:example.invalid", 40_000));
    room
}

/// The reader is looking at another room, so nothing here depends on the
/// foreground room.
fn state_watching_another_room(rooms: Vec<RoomSummary>) -> AppState {
    let mut state = ready_state();
    state.navigation.active_room_id = Some(FOREGROUND_ROOM.to_owned());
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms,
        },
    );
    state
}

fn room<'a>(state: &'a AppState, room_id: &str) -> &'a RoomSummary {
    state
        .rooms
        .iter()
        .find(|room| room.room_id == room_id)
        .expect("room in the projection")
}

fn attention(state: &AppState, room_id: &str) -> RoomAttentionProjection {
    room_attention_projection(room(state, room_id), None)
}

fn settle(state: &mut AppState, room_id: &str, event_id: &str) -> Vec<AppEffect> {
    reduce(
        state,
        AppAction::ConfirmedReadBoundarySettled {
            room_id: room_id.to_owned(),
            event_id: event_id.to_owned(),
        },
    )
}

#[test]
fn a_confirmed_boundary_clears_a_room_the_reader_is_not_in() {
    let mut state = state_watching_another_room(vec![
        unread_room("$newest:example.invalid"),
        foreground_room(),
        unrelated_unread_room(),
    ]);

    let effects = settle(&mut state, READ_ROOM, "$newest:example.invalid");

    let read = room(&state, READ_ROOM);
    assert_eq!(read.unread_count, 0, "the main badge drops without an echo");
    assert_eq!(read.notification_count, 0);
    assert_eq!(read.highlight_count, 0);
    assert!(
        !attention(&state, READ_ROOM).is_attention_highlighted,
        "attention follows the counts in the same reduce"
    );
    assert_eq!(
        state
            .live_signals
            .rooms
            .get(READ_ROOM)
            .and_then(|signals| signals.fully_read_event_id.as_deref()),
        Some("$newest:example.invalid"),
        "the confirmed boundary is the room's published read position"
    );
    assert!(
        state.locally_read_activity.contains_key(READ_ROOM),
        "the covered activity is recorded so a stale snapshot cannot revive it"
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::EmitUiEvent(UiEvent::RoomListChanged))),
        "the room list is republished"
    );
    // The foreground room and an unrelated room keep whatever they had.
    assert_eq!(room(&state, FOREGROUND_ROOM).unread_count, 5);
    assert_eq!(room(&state, FOREGROUND_ROOM).notification_count, 4);
    assert_eq!(room(&state, OTHER_ROOM).unread_count, 2);
    assert_eq!(room(&state, OTHER_ROOM).notification_count, 1);
}

#[test]
fn a_newer_activity_keeps_the_unread_counts() {
    // The list's newest event is newer than the boundary: a message arrived
    // before the read was confirmed, so it is still unread.
    let mut state =
        state_watching_another_room(vec![unread_room("$arrived-later:example.invalid")]);

    settle(&mut state, READ_ROOM, "$read:example.invalid");

    let read = room(&state, READ_ROOM);
    assert_eq!(read.unread_count, 3, "an uncovered message stays unread");
    assert_eq!(read.notification_count, 2);
    assert_eq!(read.highlight_count, 1);
    assert!(attention(&state, READ_ROOM).is_attention_highlighted);
    assert!(
        !state.locally_read_activity.contains_key(READ_ROOM),
        "nothing was covered, so nothing is recorded as read"
    );
    assert_eq!(
        state
            .live_signals
            .rooms
            .get(READ_ROOM)
            .and_then(|signals| signals.fully_read_event_id.as_deref()),
        Some("$read:example.invalid"),
        "the boundary is still published as the read position"
    );
}

#[test]
fn a_manual_mark_unread_survives_untouched() {
    let mut unread = unread_room("$newest:example.invalid");
    unread.marked_unread = true;
    let mut state = state_watching_another_room(vec![unread, foreground_room()]);

    settle(&mut state, READ_ROOM, "$newest:example.invalid");

    // An explicit mark-unread is the reader's own intent, so the automatic
    // boundary leaves the counts under it alone: clearing them would leave a dot,
    // and the next snapshot of the same activity would restore them as stale
    // counts.
    let read = room(&state, READ_ROOM);
    assert_eq!(read.unread_count, 3);
    assert_eq!(read.notification_count, 2);
    assert!(read.marked_unread);
    assert!(attention(&state, READ_ROOM).is_attention_highlighted);
    assert!(
        !state.locally_read_activity.contains_key(READ_ROOM),
        "no activity is recorded as read, so nothing can clear that intent later"
    );
}

#[test]
fn a_thread_unread_keeps_the_room_attention_counters() {
    // Room-level notification and highlight counters can include thread replies
    // this device has not observed, and a thread highlight cannot be restored once
    // dropped (#1238), so only the main unread is settled.
    let mut state = state_watching_another_room(vec![
        unread_room("$newest:example.invalid"),
        foreground_room(),
    ]);
    reduce(
        &mut state,
        AppAction::ThreadUnreadObserved {
            room_id: READ_ROOM.to_owned(),
            root_event_id: "$root:example.invalid".to_owned(),
            unread: 4,
            highlight: 1,
        },
    );
    assert_eq!(room(&state, READ_ROOM).thread_unread_count, 4);

    settle(&mut state, READ_ROOM, "$newest:example.invalid");

    let read = room(&state, READ_ROOM);
    assert_eq!(read.unread_count, 0, "the main badge drops without an echo");
    assert_eq!(
        read.notification_count, 2,
        "the echo owns the thread counters"
    );
    assert_eq!(read.highlight_count, 1);
    assert_eq!(
        read.thread_unread_count, 4,
        "a thread reply is never part of the main-room boundary"
    );
    assert_eq!(read.thread_highlight_count, 1);
}

#[test]
fn an_old_snapshot_cannot_restore_a_settled_count() {
    let mut state = state_watching_another_room(vec![
        unread_room("$newest:example.invalid"),
        foreground_room(),
    ]);
    settle(&mut state, READ_ROOM, "$newest:example.invalid");
    assert_eq!(room(&state, READ_ROOM).unread_count, 0);

    // The Sliding Sync echo of the pre-read state arrives after the settlement.
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![unread_room("$newest:example.invalid"), foreground_room()],
        },
    );

    let read = room(&state, READ_ROOM);
    assert_eq!(
        read.unread_count, 0,
        "a snapshot showing the activity the boundary already covered is stale"
    );
    assert_eq!(read.notification_count, 0);
    assert_eq!(read.highlight_count, 0);
}

#[test]
fn a_settlement_for_an_unknown_room_keeps_the_projection() {
    let mut state = state_watching_another_room(vec![unread_room("$newest:example.invalid")]);

    let effects = settle(
        &mut state,
        "!absent:example.invalid",
        "$newest:example.invalid",
    );

    // The read position is kept for the room list's next update; there is no
    // projection to settle, so nothing is republished.
    assert_eq!(
        state
            .live_signals
            .rooms
            .get("!absent:example.invalid")
            .and_then(|signals| signals.fully_read_event_id.as_deref()),
        Some("$newest:example.invalid")
    );
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::EmitUiEvent(UiEvent::RoomListChanged)))
    );
    assert_eq!(room(&state, READ_ROOM).unread_count, 3);
}

#[test]
fn a_settlement_that_clears_nothing_still_records_the_covered_activity() {
    // The echo already settled the counts. The boundary still proves this
    // activity read, so a snapshot that keeps showing it unread is stale.
    let mut settled = unread_room("$newest:example.invalid");
    settled.unread_count = 0;
    settled.notification_count = 0;
    settled.highlight_count = 0;
    let mut state = state_watching_another_room(vec![settled]);

    let effects = settle(&mut state, READ_ROOM, "$newest:example.invalid");

    assert!(
        state.locally_read_activity.contains_key(READ_ROOM),
        "the covered activity is recorded even without counts to clear"
    );
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::EmitUiEvent(UiEvent::RoomListChanged))),
        "an unchanged projection is not republished"
    );

    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![unread_room("$newest:example.invalid")],
        },
    );
    assert_eq!(room(&state, READ_ROOM).unread_count, 0);
}

#[test]
fn a_settlement_never_moves_an_existing_read_position() {
    // The marker path owns `fully_read_event_id`, and the read divider falls back
    // to it: a settlement for an older boundary must not pull the divider back.
    let mut state = state_watching_another_room(vec![unread_room("$newest:example.invalid")]);
    reduce(
        &mut state,
        AppAction::FullyReadMarkerUpdated {
            room_id: READ_ROOM.to_owned(),
            event_id: Some("$newest:example.invalid".to_owned()),
        },
    );

    let effects = settle(&mut state, READ_ROOM, "$older:example.invalid");

    assert_eq!(
        state
            .live_signals
            .rooms
            .get(READ_ROOM)
            .and_then(|signals| signals.fully_read_event_id.as_deref()),
        Some("$newest:example.invalid"),
        "the newer read position survives"
    );
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged))),
        "nothing was written, so nothing is republished"
    );
    assert_eq!(
        room(&state, READ_ROOM).unread_count,
        3,
        "an older boundary covers nothing the list shows"
    );
}
