//! #1915: open Activity is rebuilt from a per-room profile table, and only
//! when an Activity input changed.

use super::super::tests::unread_diagnostic_room;
use super::*;
use koushi_diagnostics::test_support::{DetailCursor, detail_cursor, detail_records_since};
use koushi_diagnostics::{DiagnosticRecord, DiagnosticValue};
use koushi_state::{
    ActivityMarkReadState, ConversationActivity, ConversationActivitySource, RoomSummary,
    SessionInfo, SessionState,
};

/// A count no other test uses, so concurrent tests that emit `core.unread`
/// without the diagnostics lock cannot be mistaken for this test's records.
const INCREMENTAL_UNREAD_COUNT: u64 = 73_219;

fn room_id(index: usize) -> String {
    format!("!incremental-{index}:example.invalid")
}

fn unread_room(index: usize) -> RoomSummary {
    let mut room = unread_diagnostic_room(&room_id(index));
    room.display_label = format!("Synthetic room {index}");
    room.unread_count = INCREMENTAL_UNREAD_COUNT;
    room.conversation_activity = Some(ConversationActivity {
        timestamp_ms: 1_000 + index as u64,
        source: ConversationActivitySource::Message,
    });
    room
}

fn read_room(index: usize) -> RoomSummary {
    let mut room = unread_room(index);
    room.unread_count = 0;
    room.notification_count = 0;
    room.highlight_count = 0;
    room.marked_unread = false;
    room
}

fn open_activity_state(rooms: Vec<RoomSummary>) -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@synthetic:example.invalid".to_owned(),
            device_id: "SYNTHETIC".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        rooms,
        activity: ActivityState::Open {
            active_tab: ActivityTab::Recent,
            recent: ActivityStream::default(),
            unread: ActivityStream::default(),
            mark_read: ActivityMarkReadState::Idle,
        },
        ..AppState::default()
    }
}

fn is_placeholder_record(record: &DiagnosticRecord) -> bool {
    record.event.source == "core.unread"
        && record.event.stage == "activity_placeholder"
        && record.event.fields.iter().any(|field| {
            field.key == "unread"
                && matches!(field.value, DiagnosticValue::Count(count) if count >= INCREMENTAL_UNREAD_COUNT)
        })
}

fn placeholder_records_since(cursor: DetailCursor) -> usize {
    detail_records_since(cursor)
        .iter()
        .filter(|record| is_placeholder_record(record))
        .count()
}

fn unread_rows(state: &AppState) -> Vec<ActivityRow> {
    let ActivityState::Open { unread, .. } = &state.activity else {
        panic!("Activity should be open");
    };
    unread.rows.clone()
}

/// Apply the projection's update the way the runtime does after an action.
fn apply_update(projection: &mut ActivityProjection, state: &mut AppState) -> bool {
    match projection.update_action_for_open_state(state) {
        Some(action) => {
            koushi_state::reduce(state, action);
            true
        }
        None => false,
    }
}

#[test]
fn open_activity_is_not_rebuilt_by_an_action_that_touches_no_activity_input() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    const UNREAD_ROOMS: usize = 40;
    let mut state = open_activity_state((0..UNREAD_ROOMS).map(unread_room).collect());
    let mut projection = ActivityProjection::default();

    let initial = detail_cursor();
    assert!(apply_update(&mut projection, &mut state));
    assert_eq!(unread_rows(&state).len(), UNREAD_ROOMS);
    assert_eq!(placeholder_records_since(initial), UNREAD_ROOMS);
    let rebuilds_after_initial = projection.stream_rebuilds();

    // Switching tabs changes Activity state, but no input the streams read.
    let unrelated = detail_cursor();
    koushi_state::reduce(
        &mut state,
        AppAction::ActivityTabSelected {
            tab: ActivityTab::Unread,
        },
    );
    for _ in 0..10 {
        assert!(
            projection.update_action_for_open_state(&state).is_none(),
            "an action that touches no Activity input must not emit rows"
        );
    }
    assert_eq!(
        projection.stream_rebuilds(),
        rebuilds_after_initial,
        "an action that touches no Activity input must not rebuild the streams"
    );
    assert_eq!(
        placeholder_records_since(unrelated),
        0,
        "an unchanged rebuild must not record activity_placeholder traces"
    );
}

#[test]
fn one_room_unread_change_updates_exactly_that_rooms_row() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let mut rooms = (0..4).map(unread_room).collect::<Vec<_>>();
    rooms.push(read_room(4));
    let mut state = open_activity_state(rooms);
    let mut projection = ActivityProjection::default();
    assert!(apply_update(&mut projection, &mut state));
    let before = unread_rows(&state);
    assert_eq!(before.len(), 4);
    let revisions_before = (0..5)
        .map(|index| projection.room_profile_revision(&room_id(index)))
        .collect::<Vec<_>>();

    // Room 4 becomes unread: exactly its row appears, every other row stays.
    state.rooms[4].unread_count = INCREMENTAL_UNREAD_COUNT;
    let became_unread = detail_cursor();
    assert!(apply_update(&mut projection, &mut state));
    let after = unread_rows(&state);
    assert_eq!(after.len(), 5);
    let added = after
        .iter()
        .filter(|row| !before.contains(row))
        .collect::<Vec<_>>();
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].room_id, room_id(4));
    assert!(before.iter().all(|row| after.contains(row)));
    assert_eq!(placeholder_records_since(became_unread), 1);
    assert_eq!(projection.last_rebuilt_room_rows(), [room_id(4)]);
    for (index, revision_before) in revisions_before.iter().enumerate().take(4) {
        assert_eq!(
            projection.room_profile_revision(&room_id(index)),
            *revision_before,
            "room {index} did not change"
        );
    }
    assert_ne!(
        projection.room_profile_revision(&room_id(4)),
        revisions_before[4]
    );

    // A count change on room 0 refreshes only room 0. Its placeholder row
    // carries no count, so the emitted streams stay as they are.
    state.rooms[0].unread_count = INCREMENTAL_UNREAD_COUNT + 2;
    let count_changed = detail_cursor();
    assert!(!apply_update(&mut projection, &mut state));
    assert_eq!(unread_rows(&state), after);
    assert_eq!(placeholder_records_since(count_changed), 1);
    assert_eq!(projection.last_rebuilt_room_rows(), [room_id(0)]);
}
