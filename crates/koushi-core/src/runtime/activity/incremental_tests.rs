//! #1915: open Activity is rebuilt from a per-room profile table, and only
//! when an Activity input changed.

use super::super::tests::unread_diagnostic_room;
use super::*;
use koushi_diagnostics::test_support::{DetailCursor, detail_cursor, detail_records_since};
use koushi_diagnostics::{DiagnosticRecord, DiagnosticValue};
use koushi_state::{
    ActivityMarkReadState, ConversationActivity, ConversationActivitySource,
    RoomLatestEventSummary, RoomNotificationMode, RoomSummary, RoomTagKind, SessionInfo,
    SessionState, SpaceSummary,
};

/// A count no other test uses, so concurrent tests that emit `core.unread`
/// without the diagnostics lock cannot be mistaken for this test's records.
const INCREMENTAL_UNREAD_COUNT: u64 = 73_219;

fn room_id(index: usize) -> String {
    format!("!incremental-{index}:example.invalid")
}

fn unread_room(index: usize) -> RoomSummary {
    let mut room = unread_diagnostic_room(&room_id(index));
    // `RoomListUpdated` re-derives the label from `display_name`, so the
    // fixture keeps all three in agreement for an update to be a true no-op.
    let label = format!("Synthetic room {index}");
    room.display_name = label.clone();
    room.display_label = label.clone();
    room.original_display_label = label;
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

fn recent_rows(state: &AppState) -> Vec<ActivityRow> {
    let ActivityState::Open { recent, .. } = &state.activity else {
        panic!("Activity should be open");
    };
    recent.rows.clone()
}

fn space_summary(display_name: &str) -> SpaceSummary {
    SpaceSummary {
        space_id: "!space:example.invalid".to_owned(),
        raw_name: None,
        display_name: display_name.to_owned(),
        avatar: None,
        join_rule: None,
        topic: None,
        parent_side_child_room_ids: Vec::new(),
        child_room_ids: Vec::new(),
    }
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

/// The hot path #1915 came from: while Activity is open, the live room list
/// publishes the same snapshot again and again. Ten identical
/// `RoomListUpdated` actions must not emit rows, must not rebuild either
/// stream, and must not record `activity_placeholder` for any room.
#[test]
fn repeated_identical_room_list_updates_do_not_rebuild_open_activity() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    const UNREAD_ROOMS: usize = 40;
    let mut state = open_activity_state((0..UNREAD_ROOMS).map(unread_room).collect());
    let mut projection = ActivityProjection::default();
    assert!(apply_update(&mut projection, &mut state));
    let rows_after_initial = unread_rows(&state);
    let rebuilds_after_initial = projection.stream_rebuilds();
    assert_eq!(
        placeholder_records_since(detail_cursor()),
        0,
        "the initial rebuild is already behind the cursor"
    );

    let repeated = detail_cursor();
    for round in 0..10 {
        let rooms = state.rooms.clone();
        koushi_state::reduce(
            &mut state,
            AppAction::RoomListUpdated {
                spaces: Vec::new(),
                rooms,
            },
        );
        assert!(
            projection.update_action_for_open_state(&state).is_none(),
            "identical RoomListUpdated round {round} must not emit Activity rows"
        );
    }
    assert_eq!(unread_rows(&state), rows_after_initial);
    assert_eq!(
        projection.stream_rebuilds(),
        rebuilds_after_initial,
        "identical RoomListUpdated must not rebuild the Activity streams"
    );
    assert_eq!(
        placeholder_records_since(repeated),
        0,
        "identical RoomListUpdated must not record activity_placeholder"
    );
    assert!(projection.last_rebuilt_room_rows().is_empty());
}

/// A room-list update that changes only a field no Activity input reads
/// (here: one room's member count) still rebuilds nothing.
#[test]
fn a_room_list_update_that_changes_no_activity_input_rebuilds_nothing() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let mut state = open_activity_state((0..4).map(unread_room).collect());
    let mut projection = ActivityProjection::default();
    assert!(apply_update(&mut projection, &mut state));
    let rows_after_initial = unread_rows(&state);
    let rebuilds_after_initial = projection.stream_rebuilds();

    state.rooms[2].joined_members += 1;
    let updated = state.rooms.clone();
    let cursor = detail_cursor();
    koushi_state::reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: updated,
        },
    );
    assert!(
        projection.update_action_for_open_state(&state).is_none(),
        "a member-count change is not an Activity input"
    );
    assert_eq!(projection.stream_rebuilds(), rebuilds_after_initial);
    assert_eq!(placeholder_records_since(cursor), 0);
    assert_eq!(unread_rows(&state), rows_after_initial);
}

/// The other half of the contract these tests pin: when an Activity input *did*
/// change, the runtime emits exactly once. Each case applies the change, expects
/// one update, then applies the same unchanged state again and expects silence.
///
/// A dropped `changes.updated` push in either the profile table or
/// `refresh_room_rows` leaves the room's row stale, which only these cases can
/// see: `snapshot()` always rebuilds everything on its first call.
fn assert_one_emit_then_none(
    state: &mut AppState,
    projection: &mut ActivityProjection,
    case: &str,
) {
    assert!(
        apply_update(projection, state),
        "{case} changed an Activity input and must emit once"
    );
    let rows = unread_rows(state);
    assert!(
        !apply_update(projection, state),
        "{case} must not emit again on an unchanged snapshot"
    );
    assert_eq!(
        unread_rows(state),
        rows,
        "{case}: a silent pass changes no rows"
    );
}

#[test]
fn every_activity_input_change_emits_exactly_once() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let mut state = open_activity_state((0..4).map(unread_room).collect());
    let mut projection = ActivityProjection::default();
    assert!(apply_update(&mut projection, &mut state));

    // A mute removes room 2 from Unread.
    koushi_state::reduce(
        &mut state,
        AppAction::RoomNotificationModeSet {
            request_id: 1,
            room_id: room_id(2),
            mode: RoomNotificationMode::Mute,
        },
    );
    assert_one_emit_then_none(&mut state, &mut projection, "mute");
    assert!(
        unread_rows(&state)
            .iter()
            .all(|row| row.room_id != room_id(2)),
        "a muted room has no Unread row"
    );

    // A low-priority tag removes room 3 the same way.
    koushi_state::reduce(
        &mut state,
        AppAction::RoomTagSet {
            room_id: room_id(3),
            tag: RoomTagKind::LowPriority,
            info: koushi_state::RoomTagInfo { order: None },
        },
    );
    assert_one_emit_then_none(&mut state, &mut projection, "low priority");
    assert!(
        unread_rows(&state)
            .iter()
            .all(|row| row.room_id != room_id(3)),
        "a low-priority room has no Unread row"
    );

    // A room that leaves the list loses its row.
    let mut rooms = state.rooms.clone();
    let removed = rooms.remove(1);
    koushi_state::reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms,
        },
    );
    assert_one_emit_then_none(&mut state, &mut projection, "room removal");
    assert!(
        unread_rows(&state)
            .iter()
            .all(|row| row.room_id != removed.room_id),
        "a removed room has no Unread row"
    );
}

/// The per-room narrowing must not lose a change that only the Space label
/// carries: a rename re-labels its rooms' rows and nothing else.
#[test]
fn a_space_rename_relabels_its_unread_rows_once() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let mut state = open_activity_state((0..2).map(unread_room).collect());
    let mut space = space_summary("Synthetic Workspace");
    space.child_room_ids = vec![room_id(0)];
    state.spaces = vec![space];
    let mut projection = ActivityProjection::default();
    assert!(apply_update(&mut projection, &mut state));
    let row_of = |rows: &[ActivityRow], room: &str| {
        rows.iter()
            .find(|row| row.room_id == room)
            .expect("row for the room")
            .clone()
    };
    let before = row_of(&unread_rows(&state), &room_id(0));
    assert_eq!(
        before.context_label, "Synthetic Workspace / Synthetic room 0",
        "the fixture must start from a resolved Space label"
    );

    let mut renamed = state.spaces[0].clone();
    renamed.display_name = "Renamed Workspace".to_owned();
    let rooms = state.rooms.clone();
    koushi_state::reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: vec![renamed],
            rooms,
        },
    );
    let outside_before = row_of(&unread_rows(&state), &room_id(1));
    assert_one_emit_then_none(&mut state, &mut projection, "Space rename");
    let rows = unread_rows(&state);
    assert_eq!(
        row_of(&rows, &room_id(0)).context_label,
        "Renamed Workspace / Synthetic room 0"
    );
    assert_eq!(
        row_of(&rows, &room_id(1)).context_label,
        outside_before.context_label,
        "a room outside the renamed Space keeps its label"
    );
}

/// A new message is the case the whole change exists for, and the one a dropped
/// `latest_event` comparison would silently freeze.
#[test]
fn a_new_message_on_a_read_room_adds_its_recent_row_once() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let mut rooms = (0..2).map(read_room).collect::<Vec<_>>();
    rooms.push(unread_room(2));
    let mut state = open_activity_state(rooms);
    let mut projection = ActivityProjection::default();
    assert!(apply_update(&mut projection, &mut state));
    let recent_before = recent_rows(&state);
    assert!(
        recent_before.iter().all(|row| row.room_id != room_id(0)),
        "a read room with no displayable event has no Recent row"
    );

    let mut room = state.rooms[0].clone();
    // In step with the fixture's conversation activity (1_000 ms), so the only
    // input this case moves is `latest_event` itself.
    room.latest_event = Some(RoomLatestEventSummary {
        event_id: "$synthetic-new-message:example.invalid".to_owned(),
        relation_type: None,
        relation_event_id: None,
        thread_root_event_id: None,
        sender_id: Some("@synthetic:example.invalid".to_owned()),
        sender_label: Some("Synthetic Sender".to_owned()),
        sender_avatar: None,
        preview: Some("synthetic preview".to_owned()),
        message_text: None,
        timestamp_ms: 1_000,
        is_redacted: false,
    });
    let mut rooms = state.rooms.clone();
    rooms[0] = room;
    koushi_state::reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms,
        },
    );
    assert!(
        apply_update(&mut projection, &mut state),
        "a new latest event must emit Activity rows"
    );
    let after = recent_rows(&state);
    assert!(
        after.iter().any(|row| row.room_id == room_id(0)),
        "the room's newest event becomes a Recent row: {after:?}"
    );
    assert!(
        !apply_update(&mut projection, &mut state),
        "an unchanged snapshot after the arrival must not emit"
    );
}
