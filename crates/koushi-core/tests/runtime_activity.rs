//! Runtime activity integration tests.

mod support;

use std::time::Duration;

use koushi_core::event::{ActivityEvent, CoreEvent};
use koushi_core::{AppCommand, CoreCommand, CoreRuntime};
use koushi_state::{
    ActivityMarkReadState, ActivityMarkReadTarget, ActivityRowKind, ActivityState, AppAction,
    AvatarImage, AvatarThumbnailState, RoomLatestEventSummary, RoomNotificationMode, RoomSummary,
    SessionState, SpaceSummary,
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

fn notification_room_summary(room_id: &str, unread_count: u64) -> RoomSummary {
    RoomSummary {
        notification_count: unread_count,
        ..unread_room_summary(room_id, unread_count)
    }
}

#[tokio::test]
async fn app_command_opens_activity_from_observed_rows_and_mark_read_settles() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(restore_ready_actions![
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![
                    notification_room_summary("!recent:example.test", 1),
                    notification_room_summary("!stale:example.test", 1),
                    notification_room_summary("!marker:example.test", 2),
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
            .all(|row| row.kind == ActivityRowKind::RoomUnread && row.event_id.is_none()),
        "Activity/Unread is a room list, not a mixed event stream"
    );
    assert_eq!(
        unread
            .rows
            .iter()
            .map(|row| row.room_id.as_str())
            .collect::<Vec<_>>(),
        [
            "!marker:example.test",
            "!recent:example.test",
            "!stale:example.test"
        ]
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
        Some("$marker-read:example.test"),
        "room-list unread entries must not invent a newer fully-read event"
    );
    assert_eq!(
        snapshot
            .live_signals
            .rooms
            .get("!stale:example.test")
            .and_then(|signals| signals.fully_read_event_id.as_deref()),
        None,
        "room-list unread entries without event ids cannot update fully-read markers"
    );
}

#[tokio::test]
async fn activity_context_label_reflects_dm_or_space_room() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(restore_ready_actions![
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
        Some(&"QA Space / QA Room".to_owned())
    );
    assert_eq!(
        labels_by_room.get("!room-home:example.test"),
        Some(&"QA Room".to_owned())
    );
}

#[tokio::test]
async fn activity_recent_preserves_observed_sender_avatar_without_profile_cache() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    let mut row = activity_row("!room:example.test", "$avatar:example.test", 10);
    row.sender_id = Some("@alice:example.test".to_owned());
    row.sender_avatar = Some(AvatarImage {
        mxc_uri: "mxc://example.test/alice-avatar".to_owned(),
        thumbnail: AvatarThumbnailState::NotRequested,
    });

    runtime
        .inject_actions(restore_ready_actions![
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![room_summary("!room:example.test")],
            },
            AppAction::ActivityRowsObserved { rows: vec![row] },
        ])
        .await;
    wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_)) && state.rooms.len() == 1
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
    let row = recent.rows.first().expect("recent row");
    assert_eq!(
        row.sender_avatar
            .as_ref()
            .map(|avatar| avatar.mxc_uri.as_str()),
        Some("mxc://example.test/alice-avatar")
    );
}

#[tokio::test]
async fn activity_recent_includes_room_list_latest_event_for_unopened_read_dm() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    let mut dm = dm_room_summary("!dm:example.test", "@terasaki:example.test");
    dm.unread_count = 0;
    dm.notification_count = 0;
    dm.highlight_count = 0;
    dm.latest_event = Some(RoomLatestEventSummary {
        event_id: "$latest-dm:example.test".to_owned(),
        sender_id: Some("@terasaki:example.test".to_owned()),
        sender_label: Some("Satoshi Terasaki".to_owned()),
        sender_avatar: None,
        preview: Some("already read but never opened".to_owned()),
        timestamp_ms: 120,
    });

    runtime
        .inject_actions(restore_ready_actions![AppAction::RoomListUpdated {
            spaces: vec![],
            rooms: vec![dm],
        },])
        .await;
    wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_)) && state.rooms.len() == 1
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
        unread.rows.is_empty(),
        "read DMs should not appear in Activity/Unread"
    );
    let row = recent
        .rows
        .first()
        .expect("latest room event should populate Recent");
    assert_eq!(row.event_id.as_deref(), Some("$latest-dm:example.test"));
    assert_eq!(row.sender_id.as_deref(), Some("@terasaki:example.test"));
    assert_eq!(row.sender_label.as_deref(), Some("Satoshi Terasaki"));
    assert_eq!(
        row.preview.as_deref(),
        Some("already read but never opened")
    );
    assert_eq!(row.context_label, "DM");
    assert!(!row.unread);
}

#[tokio::test]
async fn activity_room_mark_read_suppresses_unread_room_entry_only_for_cleared_room() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(restore_ready_actions![
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![
                    notification_room_summary("!room-a:example.test", 1),
                    notification_room_summary("!room-b:example.test", 1),
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
                            && row.kind == ActivityRowKind::RoomUnread
                            && row.event_id.is_none()
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
        "unrelated unread room entries must not be suppressed by another room's mark-read"
    );
}

#[tokio::test]
async fn activity_unread_uses_room_summary_rows_and_mark_all_does_not_emit_synthetic_event_id() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(restore_ready_actions![
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![
                    notification_room_summary("!placeholder:example.test", 1),
                    notification_room_summary("!with-row:example.test", 1),
                    unread_room_summary("!plain-unread:example.test", 1),
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
                && row.kind == ActivityRowKind::RoomUnread
                && row.event_id.is_none()
        }),
        "Activity/Unread must remain a room list even when recent has observed events"
    );
    assert!(
        unread
            .rows
            .iter()
            .all(|row| row.room_id != "!plain-unread:example.test"),
        "plain unread message counts must not create Activity/Unread rows"
    );
    assert!(
        unread.rows.iter().all(|row| row.event_id.is_none()),
        "Activity/Unread must not mix event rows with room rows"
    );

    let mark_request_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::MarkActivityRead {
        request_id: mark_request_id,
        target: ActivityMarkReadTarget::All,
    }))
    .await
    .expect("mark activity read command");

    let snapshot = wait_for_state(&mut conn, |state| {
        let notification_rooms_cleared = ["!placeholder:example.test", "!with-row:example.test"]
            .iter()
            .all(|room_id| {
                state
                    .rooms
                    .iter()
                    .find(|room| room.room_id == *room_id)
                    .is_some_and(|room| {
                        room.unread_count == 0
                            && room.notification_count == 0
                            && room.highlight_count == 0
                            && !room.marked_unread
                    })
            });
        let plain_unread_preserved = state
            .rooms
            .iter()
            .find(|room| room.room_id == "!plain-unread:example.test")
            .is_some_and(|room| room.unread_count == 1 && room.notification_count == 0);
        matches!(
            &state.activity,
            ActivityState::Open { unread, mark_read, .. }
                if unread.rows.is_empty()
                    && matches!(mark_read, ActivityMarkReadState::Idle)
                    && notification_rooms_cleared
                    && plain_unread_preserved
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
        Vec::<String>::new(),
        "room-list mark-all has no event ids to report"
    );
}

#[tokio::test]
async fn activity_unread_removes_rooms_when_notification_mode_is_mute() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(restore_ready_actions![
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![
                    notification_room_summary("!normal:example.test", 1),
                    notification_room_summary("!muted-with-row:example.test", 1),
                    notification_room_summary("!muted-placeholder:example.test", 1),
                ],
            },
            AppAction::ActivityRowsObserved {
                rows: vec![
                    activity_row("!normal:example.test", "$normal:example.test", 30),
                    activity_row(
                        "!muted-with-row:example.test",
                        "$muted-with-row:example.test",
                        20,
                    ),
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
    wait_for_state(&mut conn, |state| {
        matches!(state.activity, ActivityState::Open { .. })
    })
    .await;

    runtime
        .inject_actions(vec![
            AppAction::RoomNotificationModeSet {
                request_id: 1,
                room_id: "!muted-with-row:example.test".to_owned(),
                mode: RoomNotificationMode::Mute,
            },
            AppAction::RoomNotificationModeSet {
                request_id: 2,
                room_id: "!muted-placeholder:example.test".to_owned(),
                mode: RoomNotificationMode::Mute,
            },
        ])
        .await;

    let snapshot = wait_for_state(&mut conn, |state| {
        matches!(
            &state.activity,
            ActivityState::Open { unread, .. }
                if unread.rows.iter().any(|row| row.room_id == "!normal:example.test")
                    && unread
                        .rows
                        .iter()
                        .all(|row| !row.room_id.starts_with("!muted-"))
        )
    })
    .await;
    let ActivityState::Open { unread, .. } = snapshot.activity else {
        panic!("activity should stay open");
    };
    assert!(
        unread.rows.iter().any(|row| {
            row.room_id == "!normal:example.test"
                && row.kind == ActivityRowKind::RoomUnread
                && row.event_id.is_none()
        }),
        "unmuted unread room rows must remain visible"
    );
    assert!(
        unread
            .rows
            .iter()
            .all(|row| !row.room_id.starts_with("!muted-")),
        "muted event rows and muted unread placeholders must be hidden"
    );
}
