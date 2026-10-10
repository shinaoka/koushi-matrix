//! Issue #1255: focused and thread receipt-summary changes cross the state
//! delta as their own scoped slices, never folded into the main-scope slice.
//!
//! All identifiers are synthetic (`example.invalid`).

use std::collections::BTreeMap;

use crate::build_state_delta;
use koushi_state::{AppState, LiveEventReceiptSummary, RoomLiveSignals};

const ROOM_ID: &str = "!room:example.invalid";
const ROOT_EVENT_ID: &str = "$root:example.invalid";
const TARGET_EVENT_ID: &str = "$target:example.invalid";
const REPLY_EVENT_ID: &str = "$reply:example.invalid";

fn summary(total_count: u64) -> LiveEventReceiptSummary {
    LiveEventReceiptSummary {
        total_count,
        ..Default::default()
    }
}

fn state_with_room(room: RoomLiveSignals) -> AppState {
    let mut state = AppState::default();
    state.live_signals.rooms.insert(ROOM_ID.to_owned(), room);
    state
}

fn room_mut(state: &mut AppState) -> &mut RoomLiveSignals {
    state
        .live_signals
        .rooms
        .get_mut(ROOM_ID)
        .expect("synthetic room is present")
}

#[test]
fn focused_receipt_change_uses_the_focused_scoped_delta() {
    let previous = state_with_room(RoomLiveSignals {
        focused_receipts_by_event: BTreeMap::from([(
            TARGET_EVENT_ID.to_owned(),
            BTreeMap::from([(REPLY_EVENT_ID.to_owned(), summary(1))]),
        )]),
        thread_receipts_by_event: BTreeMap::from([(
            ROOT_EVENT_ID.to_owned(),
            BTreeMap::from([(REPLY_EVENT_ID.to_owned(), summary(5))]),
        )]),
        ..RoomLiveSignals::default()
    });
    let mut next = previous.clone();
    room_mut(&mut next)
        .focused_receipts_by_event
        .get_mut(TARGET_EVENT_ID)
        .unwrap()
        .insert(REPLY_EVENT_ID.to_owned(), summary(9));

    let delta = build_state_delta(1, &previous, &next).expect("focused receipt change");

    assert!(delta.changed.live_signals.is_none());
    assert!(delta.changed.live_signals_rooms.is_none());
    assert!(delta.changed.live_signals_receipts_by_room_event.is_none());
    assert!(
        delta
            .changed
            .live_signals_thread_receipts_by_room_event
            .is_none()
    );
    let focused = delta
        .changed
        .live_signals_focused_receipts_by_room_event
        .expect("focused scoped slice");
    assert_eq!(
        focused[ROOM_ID][TARGET_EVENT_ID][REPLY_EVENT_ID]
            .as_ref()
            .unwrap()
            .total_count,
        9
    );
    assert!(
        !focused[ROOM_ID].contains_key(ROOT_EVENT_ID),
        "an unchanged thread scope must not appear in the focused slice"
    );
}

#[test]
fn thread_receipt_removal_uses_the_thread_scoped_delta_with_null_replacements() {
    let previous = state_with_room(RoomLiveSignals {
        thread_receipts_by_event: BTreeMap::from([(
            ROOT_EVENT_ID.to_owned(),
            BTreeMap::from([(REPLY_EVENT_ID.to_owned(), summary(3))]),
        )]),
        ..RoomLiveSignals::default()
    });
    let mut next = previous.clone();
    room_mut(&mut next)
        .thread_receipts_by_event
        .remove(ROOT_EVENT_ID);

    let delta = build_state_delta(2, &previous, &next).expect("thread receipt removal");

    assert!(delta.changed.live_signals_rooms.is_none());
    assert!(delta.changed.live_signals_receipts_by_room_event.is_none());
    assert!(
        delta
            .changed
            .live_signals_focused_receipts_by_room_event
            .is_none()
    );
    let thread = delta
        .changed
        .live_signals_thread_receipts_by_room_event
        .expect("thread scoped slice");
    assert!(
        thread[ROOM_ID][ROOT_EVENT_ID][REPLY_EVENT_ID].is_none(),
        "a removed thread-scope entry crosses as a null replacement"
    );
}

#[test]
fn main_and_thread_scope_changes_are_reported_independently() {
    let previous = state_with_room(RoomLiveSignals {
        receipts_by_event: BTreeMap::from([(REPLY_EVENT_ID.to_owned(), summary(1))]),
        ..RoomLiveSignals::default()
    });
    let mut next = previous.clone();
    room_mut(&mut next).thread_receipts_by_event.insert(
        ROOT_EVENT_ID.to_owned(),
        BTreeMap::from([(REPLY_EVENT_ID.to_owned(), summary(4))]),
    );

    let delta = build_state_delta(3, &previous, &next).expect("thread scope added");

    assert!(delta.changed.live_signals_rooms.is_none());
    assert!(delta.changed.live_signals_receipts_by_room_event.is_none());
    let thread = delta
        .changed
        .live_signals_thread_receipts_by_room_event
        .expect("thread scoped slice");
    assert_eq!(
        thread[ROOM_ID][ROOT_EVENT_ID][REPLY_EVENT_ID]
            .as_ref()
            .unwrap()
            .total_count,
        4
    );
}
