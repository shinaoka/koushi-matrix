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

/// #1915 regression: the ledgers are owned by the session that projects the
/// room list. `AccountRuntimeManager` runs several accounts in one process, so a
/// process-wide ledger let one account's traces suppress another's for the same
/// room, and one account's session start clear another's state. Two sessions
/// here are two ledger values, exactly as two runtimes hold them.
#[test]
fn two_sessions_record_the_same_room_independently() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let rooms = dedup_rooms();
    let mut session_a = unread_trace::UnreadTraceLedger::default();
    let mut session_b = unread_trace::UnreadTraceLedger::default();

    let first = detail_cursor();
    unread_trace::trace_room_list_snapshot(&mut session_a, &rooms);
    unread_trace::trace_room_list_snapshot(&mut session_b, &rooms);
    assert_eq!(
        dedup_records_since(first, "room_list_snapshot"),
        6,
        "each session records its own first snapshot: 3 rooms x 2 sessions"
    );

    // Dedup still applies inside one session.
    let repeated = detail_cursor();
    unread_trace::trace_room_list_snapshot(&mut session_a, &rooms);
    assert_eq!(dedup_records_since(repeated, "room_list_snapshot"), 0);

    // B starting a new session neither records into A's ledger nor clears it.
    let restarted = detail_cursor();
    let mut session_b_restarted = unread_trace::UnreadTraceLedger::default();
    unread_trace::trace_room_list_snapshot(&mut session_b_restarted, &rooms);
    unread_trace::trace_room_list_snapshot(&mut session_a, &rooms);
    assert_eq!(
        dedup_records_since(restarted, "room_list_snapshot"),
        3,
        "session B's restart records B's rooms and leaves session A's state alone"
    );
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

/// The actor-owned ledgers (`room_list_applied`, `activity_placeholder`) live
/// as long as the runtime, but their meaning is per session. A lock-screen
/// gate and its unlock in the same runtime change `state.session`, and the
/// session's first room-list application after that must be recorded instead
/// of being suppressed by the values traced before the gate.
#[tokio::test]
async fn a_session_change_clears_the_actor_owned_trace_history() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock_async().await;
    let data = tempfile::tempdir().expect("tempdir");
    let rooms = dedup_rooms();
    let (mut actor, ..) =
        super::super::tests::app_actor_event_navigation_fixture(data.path(), ready_state());

    let apply_room_list = |actor: &mut crate::runtime::AppActor| {
        actor.reduce_app_action_state(AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: rooms.clone(),
        });
    };
    let first = detail_cursor();
    apply_room_list(&mut actor);
    assert_eq!(
        dedup_records_since(first, "room_list_applied"),
        3,
        "the first session records every unread room"
    );

    // Same session: identical, so nothing is recorded again.
    let repeated = detail_cursor();
    apply_room_list(&mut actor);
    assert_eq!(dedup_records_since(repeated, "room_list_applied"), 0);

    // A session gate and its unlock put the session through two other values
    // and back to ready. That is a new session lifetime, not more of the old
    // one, so the next identical application is recorded again.
    let SessionState::Ready(info) = actor.state.session.clone() else {
        panic!("the fixture starts ready");
    };
    actor.reduce_app_action_state(AppAction::SessionLocked);
    assert!(!matches!(actor.state.session, SessionState::Ready(_)));
    actor.reduce_app_action_state(AppAction::RestoreSessionSucceeded(info.clone()));
    actor.reduce_app_action_state(AppAction::CurrentDeviceTrustChanged(
        koushi_state::CurrentDeviceTrustState::Verified,
    ));
    assert_eq!(actor.state.session, SessionState::Ready(info));
    let new_session = detail_cursor();
    apply_room_list(&mut actor);
    assert_eq!(
        dedup_records_since(new_session, "room_list_applied"),
        3,
        "the unlocked session records its own first room-list application"
    );
}
