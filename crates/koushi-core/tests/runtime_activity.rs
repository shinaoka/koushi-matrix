//! Runtime activity integration tests.

mod support;

use std::time::Duration;

use koushi_core::event::{ActivityEvent, CoreEvent};
use koushi_core::{AppCommand, CoreCommand, CoreRuntime};
use koushi_state::{
    ActivityMarkReadState, ActivityMarkReadTarget, ActivityRowKind, ActivityState, AppAction,
    RoomSummary, SessionState, SpaceSummary,
};
use support::{activity_row, room_summary, unread_room_summary, wait_for_state};

fn dm_room_summary(room_id: &str, dm_user_id: &str) -> RoomSummary {
    RoomSummary {
        is_dm: true,
        dm_user_ids: vec![dm_user_id.to_owned()],
        unread_count: 1,
        ..room_summary(room_id)
    }
}

fn room_in_space_summary(room_id: &str, space_id: &str) -> RoomSummary {
    RoomSummary {
        parent_space_ids: vec![space_id.to_owned()],
        unread_count: 1,
        ..room_summary(room_id)
    }
}

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
            .map(|row| row.event_id.as_deref())
            .collect::<Vec<_>>(),
        [
            Some("$marker-unread:example.test"),
            Some("$marker-read:example.test"),
            Some("$recent:example.test"),
            Some("$stale:example.test")
        ]
    );
    assert!(
        unread
            .rows
            .iter()
            .any(|row| row.event_id.as_deref() == Some("$stale:example.test")),
        "stale unread rows must remain visible"
    );
    assert!(
        unread
            .rows
            .iter()
            .any(|row| row.event_id.as_deref() == Some("$marker-unread:example.test")),
        "rows after the Rust-owned fully-read marker must remain unread"
    );
    assert!(
        unread
            .rows
            .iter()
            .all(|row| row.event_id.as_deref() != Some("$marker-read:example.test")),
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

#[tokio::test]
async fn activity_context_label_reflects_dm_or_space_room() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(vec![
            AppAction::RestoreSessionSucceeded(support::session_info()),
            AppAction::RoomListUpdated {
                spaces: vec![SpaceSummary {
                    space_id: "!space:example.test".to_owned(),
                    display_name: "QA Space".to_owned(),
                    avatar: None,
                    child_room_ids: vec!["!room-in-space:example.test".to_owned()],
                }],
                rooms: vec![
                    dm_room_summary("!dm:example.test", "@dm:example.test"),
                    room_in_space_summary("!room-in-space:example.test", "!space:example.test"),
                    unread_room_summary("!room-home:example.test", 1),
                ],
            },
            AppAction::ActivityRowsObserved {
                rows: vec![
                    activity_row("!dm:example.test", "$dm:example.test", 30),
                    activity_row("!room-in-space:example.test", "$space:example.test", 20),
                    activity_row("!room-home:example.test", "$home:example.test", 10),
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
    let ActivityState::Open { recent, .. } = snapshot.activity else {
        panic!("activity should be open");
    };
    let labels_by_room: std::collections::HashMap<String, String> = recent
        .rows
        .iter()
        .map(|row| (row.room_id.clone(), row.context_label.clone()))
        .collect();
    assert_eq!(
        labels_by_room.get("!dm:example.test"),
        Some(&"DM".to_owned())
    );
    assert_eq!(
        labels_by_room.get("!room-in-space:example.test"),
        Some(&"Room · QA Space / QA Room".to_owned())
    );
    assert_eq!(
        labels_by_room.get("!room-home:example.test"),
        Some(&"Room".to_owned())
    );
}

#[tokio::test]
async fn activity_room_mark_read_suppresses_placeholder_only_for_cleared_room() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(vec![
            AppAction::RestoreSessionSucceeded(support::session_info()),
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![
                    unread_room_summary("!room-a:example.test", 1),
                    unread_room_summary("!room-b:example.test", 1),
                ],
            },
            AppAction::ActivityRowsObserved {
                rows: vec![
                    activity_row("!room-a:example.test", "$event-a:example.test", 10),
                    activity_row("!room-b:example.test", "$event-b:example.test", 20),
                ],
            },
        ])
        .await;
    wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_)) && state.rooms.len() == 2
    })
    .await;

    let open_request_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::OpenActivity {
        request_id: open_request_id,
    }))
    .await
    .expect("open activity command");
    wait_for_state(&mut conn, |state| {
        matches!(state.activity, ActivityState::Open { .. })
    })
    .await;

    let mark_request_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::MarkActivityRead {
        request_id: mark_request_id,
        target: ActivityMarkReadTarget::Room {
            room_id: "!room-a:example.test".to_owned(),
            up_to_event_id: "$event-a:example.test".to_owned(),
        },
    }))
    .await
    .expect("mark activity room read command");

    wait_for_state(&mut conn, |state| {
        let room_a_cleared = state
            .rooms
            .iter()
            .find(|room| room.room_id == "!room-a:example.test")
            .is_some_and(|room| room.unread_count == 0);
        let room_b_unread = state
            .rooms
            .iter()
            .find(|room| room.room_id == "!room-b:example.test")
            .is_some_and(|room| room.unread_count == 1);
        matches!(
            &state.activity,
            ActivityState::Open { unread, mark_read, .. }
                if matches!(mark_read, ActivityMarkReadState::Idle)
                    && room_a_cleared
                    && room_b_unread
                    && unread.rows.iter().all(|row| row.room_id != "!room-a:example.test")
                    && unread.rows.iter().any(|row| {
                        row.room_id == "!room-b:example.test"
                            && row.kind == ActivityRowKind::Event
                            && row.event_id.as_deref() == Some("$event-b:example.test")
                    })
        )
    })
    .await;

    runtime
        .inject_actions(vec![AppAction::FullyReadMarkerUpdated {
            room_id: "!room-b:example.test".to_owned(),
            event_id: Some("$event-b:example.test".to_owned()),
        }])
        .await;

    let snapshot = wait_for_state(&mut conn, |state| {
        let marker_applied = state
            .live_signals
            .rooms
            .get("!room-b:example.test")
            .and_then(|signals| signals.fully_read_event_id.as_deref())
            == Some("$event-b:example.test");
        matches!(
            &state.activity,
            ActivityState::Open { unread, .. }
                if marker_applied
                    && unread.rows.iter().any(|row| {
                        row.room_id == "!room-b:example.test"
                            && row.kind == ActivityRowKind::RoomUnread
                            && row.event_id.is_none()
                    })
                    && unread.rows.iter().all(|row| {
                        row.event_id.as_deref() != Some("$event-b:example.test")
                    })
        )
    })
    .await;
    let ActivityState::Open { unread, .. } = snapshot.activity else {
        panic!("activity should stay open");
    };
    assert!(
        unread.rows.iter().any(|row| {
            row.room_id == "!room-b:example.test"
                && row.kind == ActivityRowKind::RoomUnread
                && row.event_id.is_none()
        }),
        "unrelated room placeholders must not be suppressed by another room's mark-read"
    );
}

#[tokio::test]
async fn activity_unread_uses_room_summary_placeholder_and_mark_all_does_not_emit_synthetic_event_id()
 {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(vec![
            AppAction::RestoreSessionSucceeded(support::session_info()),
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![
                    unread_room_summary("!placeholder:example.test", 1),
                    unread_room_summary("!with-row:example.test", 1),
                ],
            },
            AppAction::ActivityRowsObserved {
                rows: vec![activity_row(
                    "!with-row:example.test",
                    "$with-row-event:example.test",
                    20,
                )],
            },
        ])
        .await;
    wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_)) && state.rooms.len() == 2
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
    assert!(
        recent
            .rows
            .iter()
            .all(|row| row.kind != ActivityRowKind::RoomUnread),
        "Recent must stay event-backed; room-unread placeholders are Unread-only"
    );
    let placeholder = unread
        .rows
        .iter()
        .find(|row| row.room_id == "!placeholder:example.test");
    assert!(
        placeholder.is_some(),
        "unread room with no observed row must appear as a typed placeholder"
    );
    let placeholder = placeholder.unwrap();
    assert_eq!(placeholder.kind, ActivityRowKind::RoomUnread);
    assert_eq!(placeholder.event_id, None);
    assert!(placeholder.unread);
    assert!(
        unread.rows.iter().any(|row| {
            row.room_id == "!with-row:example.test"
                && row.event_id.as_deref() == Some("$with-row-event:example.test")
        }),
        "observed event rows remain preferred over placeholders"
    );
    assert!(
        !unread
            .rows
            .iter()
            .any(|row| row.room_id == "!with-row:example.test"
                && row.kind == ActivityRowKind::RoomUnread),
        "rooms with observed unread rows must not also get a placeholder"
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
                    && state.rooms.iter().all(|room| room.unread_count == 0)
        )
    })
    .await;
    assert!(
        snapshot
            .live_signals
            .rooms
            .get("!placeholder:example.test")
            .and_then(|signals| signals.fully_read_event_id.as_deref())
            .is_none(),
        "placeholder mark-all must not write a synthetic fully-read marker"
    );

    let mut cleared_event_ids = None;
    for _ in 0..50 {
        match tokio::time::timeout(Duration::from_secs(1), conn.recv_event()).await {
            Ok(Ok(CoreEvent::Activity(ActivityEvent::MarkedRead {
                request_id,
                cleared_event_ids: ids,
            }))) if request_id == mark_request_id => {
                cleared_event_ids = Some(ids);
                break;
            }
            Ok(Ok(_)) => continue,
            Ok(Err(_)) => break,
            Err(_) => break,
        }
    }
    let cleared = cleared_event_ids.expect("MarkedRead event not received");
    assert_eq!(
        cleared,
        vec!["$with-row-event:example.test"],
        "MarkedRead must contain only real event ids, no synthetic placeholder ids"
    );
}
