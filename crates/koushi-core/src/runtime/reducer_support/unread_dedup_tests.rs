//! #1915: per-room unread traces are recorded only when a room's traced
//! values change, so repeated identical room-list snapshots cannot evict the
//! rest of the bounded diagnostics ring.

use super::super::tests::unread_diagnostic_room;
use super::*;
use koushi_diagnostics::test_support::{DetailCursor, detail_cursor, detail_records_since};
use koushi_diagnostics::{DiagnosticRecord, DiagnosticValue};
use koushi_state::{RoomSummary, SessionInfo, SessionState};

/// A count no other test uses, so concurrent tests that emit `core.unread`
/// without the diagnostics lock cannot be mistaken for this test's records.
const DEDUP_UNREAD_COUNT: u64 = 91_517;

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@synthetic:example.invalid".to_owned(),
            device_id: "SYNTHETIC".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        ..AppState::default()
    }
}

fn dedup_rooms() -> Vec<RoomSummary> {
    (0..3)
        .map(|index| {
            let mut room = unread_diagnostic_room(&format!("!dedup-{index}:example.invalid"));
            room.unread_count = DEDUP_UNREAD_COUNT;
            room
        })
        .collect()
}

fn is_dedup_record(record: &DiagnosticRecord) -> bool {
    record.event.source == "core.unread"
        && record.event.fields.iter().any(|field| {
            field.key == "unread" && field.value == DiagnosticValue::Count(DEDUP_UNREAD_COUNT)
        })
}

fn dedup_records_since(cursor: DetailCursor, stage: &str) -> usize {
    detail_records_since(cursor)
        .iter()
        .filter(|record| is_dedup_record(record) && record.event.stage == stage)
        .count()
}

#[test]
fn repeated_identical_room_list_snapshots_append_no_per_room_unread_traces() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let mut snapshot_ledger = unread_trace::UnreadTraceLedger::default();
    let mut applied_ledger = unread_trace::UnreadTraceLedger::default();
    let mut state = ready_state();
    let mut rooms = dedup_rooms();
    let mut apply = |state: &mut AppState, rooms: &[RoomSummary]| {
        unread_trace::trace_room_list_snapshot(&mut snapshot_ledger, rooms);
        reduce_with_unread_diagnostics(
            state,
            &mut applied_ledger,
            AppAction::RoomListUpdated {
                spaces: Vec::new(),
                rooms: rooms.to_vec(),
            },
        );
    };

    let first = detail_cursor();
    apply(&mut state, &rooms);
    assert_eq!(dedup_records_since(first, "room_list_snapshot"), 3);
    assert_eq!(dedup_records_since(first, "room_list_applied"), 3);

    let repeated = detail_cursor();
    for _ in 0..5 {
        apply(&mut state, &rooms);
    }
    let appended = detail_records_since(repeated)
        .iter()
        .filter(|record| is_dedup_record(record))
        .count();
    assert_eq!(
        appended, 0,
        "identical room-list snapshots must not append per-room unread traces"
    );

    // A change to one room's traced values records exactly that room again.
    rooms[1].notification_count += 1;
    let changed = detail_cursor();
    apply(&mut state, &rooms);
    assert_eq!(dedup_records_since(changed, "room_list_snapshot"), 1);
    assert_eq!(dedup_records_since(changed, "room_list_applied"), 1);

    // A room that becomes read and later unread again with the same values is
    // a new transition, so it is traced again.
    rooms[2].unread_count = 0;
    rooms[2].notification_count = 0;
    rooms[2].highlight_count = 0;
    rooms[2].marked_unread = false;
    apply(&mut state, &rooms);
    rooms[2] = dedup_rooms().remove(2);
    let unread_again = detail_cursor();
    apply(&mut state, &rooms);
    assert_eq!(dedup_records_since(unread_again, "room_list_snapshot"), 1);
    assert_eq!(dedup_records_since(unread_again, "room_list_applied"), 1);
}
