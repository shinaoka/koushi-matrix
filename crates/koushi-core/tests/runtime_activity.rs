//! Runtime activity integration tests.

mod support;

use koushi_core::{AppCommand, CoreCommand, CoreRuntime};
use koushi_state::{
    ActivityMarkReadState, ActivityMarkReadTarget, ActivityState, AppAction, SessionState,
};
use support::{activity_row, unread_room_summary, wait_for_state};

#[tokio::test]
async fn app_command_opens_activity_from_observed_rows_and_mark_read_settles() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(vec![
            AppAction::RestoreSessionSucceeded(support::session_info()),
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![
                    unread_room_summary("!recent:example.test", 1),
                    unread_room_summary("!stale:example.test", 1),
                    unread_room_summary("!marker:example.test", 2),
                ],
            },
            AppAction::FullyReadMarkerUpdated {
                room_id: "!marker:example.test".to_owned(),
                event_id: Some("$marker-read:example.test".to_owned()),
            },
            AppAction::ActivityRowsObserved {
                rows: vec![
                    activity_row("!recent:example.test", "$recent:example.test", 20),
                    activity_row("!stale:example.test", "$stale:example.test", 1),
                    activity_row("!marker:example.test", "$marker-read:example.test", 40),
                    activity_row("!marker:example.test", "$marker-unread:example.test", 60),
                ],
            },
        ])
        .await;
    wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_)) && state.rooms.len() == 3
    })
    .await;

    let open_request_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::OpenActivity {
        request_id: open_request_id,
    }))
    .await
    .expect("open activity command");

    let snapshot = wait_for_state(&mut conn, |state| {
        matches!(state.activity, ActivityState::Open { .. })
    })
    .await;
    let ActivityState::Open { recent, unread, .. } = snapshot.activity else {
        panic!("activity should be open");
    };
    assert_eq!(
        recent
            .rows
            .iter()
            .map(|row| row.event_id.as_str())
            .collect::<Vec<_>>(),
        [
            "$marker-unread:example.test",
            "$marker-read:example.test",
            "$recent:example.test",
            "$stale:example.test"
        ]
    );
    assert!(
        unread
            .rows
            .iter()
            .any(|row| row.event_id == "$stale:example.test"),
        "stale unread rows must remain visible"
    );
    assert!(
        unread
            .rows
            .iter()
            .any(|row| row.event_id == "$marker-unread:example.test"),
        "rows after the Rust-owned fully-read marker must remain unread"
    );
    assert!(
        unread
            .rows
            .iter()
            .all(|row| row.event_id != "$marker-read:example.test"),
        "rows at or before the Rust-owned fully-read marker must be excluded"
    );

    let mark_request_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::MarkActivityRead {
        request_id: mark_request_id,
        target: ActivityMarkReadTarget::All,
    }))
    .await
    .expect("mark activity read command");

    let snapshot = wait_for_state(&mut conn, |state| {
        matches!(
            &state.activity,
            ActivityState::Open { unread, mark_read, .. }
                if unread.rows.is_empty()
                    && matches!(mark_read, ActivityMarkReadState::Idle)
                    && state
                        .live_signals
                        .rooms
                        .get("!marker:example.test")
                        .and_then(|signals| signals.fully_read_event_id.as_deref())
                        == Some("$marker-unread:example.test")
                    && state
                        .live_signals
                        .rooms
                        .get("!stale:example.test")
                        .and_then(|signals| signals.fully_read_event_id.as_deref())
                        == Some("$stale:example.test")
        )
    })
    .await;
    let ActivityState::Open { unread, .. } = snapshot.activity else {
        panic!("activity should stay open");
    };
    assert!(unread.rows.is_empty());
    assert!(
        snapshot.rooms.iter().all(|room| room.unread_count == 0),
        "activity mark-all-read must clear room unread counts so sidebar badges agree"
    );
    assert_eq!(
        snapshot
            .live_signals
            .rooms
            .get("!marker:example.test")
            .and_then(|signals| signals.fully_read_event_id.as_deref()),
        Some("$marker-unread:example.test")
    );
    assert_eq!(
        snapshot
            .live_signals
            .rooms
            .get("!stale:example.test")
            .and_then(|signals| signals.fully_read_event_id.as_deref()),
        Some("$stale:example.test")
    );
}
