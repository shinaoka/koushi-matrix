use koushi_state::{
    ActivityMarkReadState, ActivityMarkReadTarget, ActivityRow, ActivityState, ActivityStream,
    ActivityTab, AppAction, AppEffect, AppState, OperationFailureKind, SessionInfo, SessionState,
    UiEvent, reduce,
};

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.org".to_owned(),
        user_id: "@user-a:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
    }
}

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(session_info()),
        ..AppState::default()
    }
}

fn row(room_id: &str, event_id: &str, timestamp_ms: u64) -> ActivityRow {
    ActivityRow {
        room_id: room_id.to_owned(),
        event_id: event_id.to_owned(),
        room_label: format!("Room {room_id}"),
        sender_label: Some("@sender:example.invalid".to_owned()),
        preview: Some(format!("body for {event_id}")),
        timestamp_ms,
        unread: true,
        highlight: false,
    }
}

fn stream(rows: Vec<ActivityRow>, next_batch: Option<&str>) -> ActivityStream {
    ActivityStream {
        rows,
        next_batch: next_batch.map(str::to_owned),
    }
}

#[test]
fn activity_open_lifecycle_is_request_correlated_and_sorts_rows_newest_first() {
    let mut state = ready_state();

    let effects = reduce(&mut state, AppAction::ActivityOpened { request_id: 7 });

    assert_eq!(
        state.activity,
        ActivityState::Opening {
            request_id: 7,
            tab: ActivityTab::Recent,
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::ActivityChanged)]
    );

    assert_eq!(
        reduce(
            &mut state,
            AppAction::ActivitySnapshotLoaded {
                request_id: 8,
                active_tab: ActivityTab::Recent,
                recent: stream(vec![row("!a", "$new", 30)], Some("stale-recent")),
                unread: stream(vec![row("!b", "$old", 10)], Some("stale-unread")),
                excluded_room_ids: Vec::new(),
            },
        ),
        Vec::new()
    );
    assert!(matches!(
        state.activity,
        ActivityState::Opening { request_id: 7, .. }
    ));

    reduce(
        &mut state,
        AppAction::ActivitySnapshotLoaded {
            request_id: 7,
            active_tab: ActivityTab::Recent,
            recent: stream(
                vec![
                    row("!a", "$older-recent", 20),
                    row("!b", "$newer-recent", 40),
                ],
                Some("recent-page-2"),
            ),
            unread: stream(
                vec![
                    row("!c", "$older-unread", 5),
                    row("!d", "$newer-unread", 35),
                ],
                Some("unread-page-2"),
            ),
            excluded_room_ids: Vec::new(),
        },
    );

    assert_eq!(
        state.activity,
        ActivityState::Open {
            active_tab: ActivityTab::Recent,
            recent: stream(
                vec![
                    row("!b", "$newer-recent", 40),
                    row("!a", "$older-recent", 20)
                ],
                Some("recent-page-2"),
            ),
            unread: stream(
                vec![
                    row("!d", "$newer-unread", 35),
                    row("!c", "$older-unread", 5)
                ],
                Some("unread-page-2"),
            ),
            mark_read: ActivityMarkReadState::Idle,
        }
    );
}

#[test]
fn activity_unread_stream_keeps_stale_unreads_separate_from_recent_bounds() {
    let mut state = ready_state();
    reduce(&mut state, AppAction::ActivityOpened { request_id: 9 });

    reduce(
        &mut state,
        AppAction::ActivitySnapshotLoaded {
            request_id: 9,
            active_tab: ActivityTab::Unread,
            recent: stream(
                vec![row("!recent", "$recent-message", 100)],
                Some("recent-next"),
            ),
            unread: stream(
                vec![
                    row("!stale", "$stale-unread", 1),
                    row("!recent", "$recent-unread", 90),
                ],
                Some("unread-next"),
            ),
            excluded_room_ids: Vec::new(),
        },
    );

    let ActivityState::Open { recent, unread, .. } = &state.activity else {
        panic!("activity should be open");
    };

    assert_eq!(
        recent.rows,
        vec![row("!recent", "$recent-message", 100)],
        "Recent keeps its recency-bounded rows"
    );
    assert_eq!(
        unread.rows,
        vec![
            row("!recent", "$recent-unread", 90),
            row("!stale", "$stale-unread", 1),
        ],
        "Unread keeps stale unread rows even when they are outside Recent"
    );
    assert_eq!(recent.next_batch.as_deref(), Some("recent-next"));
    assert_eq!(unread.next_batch.as_deref(), Some("unread-next"));
}

#[test]
fn selecting_unread_tab_does_not_clear_unread_rows() {
    let mut state = ready_state();
    reduce(&mut state, AppAction::ActivityOpened { request_id: 10 });
    reduce(
        &mut state,
        AppAction::ActivitySnapshotLoaded {
            request_id: 10,
            active_tab: ActivityTab::Recent,
            recent: stream(vec![row("!room", "$recent", 20)], None),
            unread: stream(vec![row("!room", "$unread", 10)], None),
            excluded_room_ids: Vec::new(),
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::ActivityTabSelected {
            tab: ActivityTab::Unread,
        },
    );

    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::ActivityChanged)]
    );
    assert_eq!(
        state.activity,
        ActivityState::Open {
            active_tab: ActivityTab::Unread,
            recent: stream(vec![row("!room", "$recent", 20)], None),
            unread: stream(vec![row("!room", "$unread", 10)], None),
            mark_read: ActivityMarkReadState::Idle,
        }
    );
}

#[test]
fn activity_mark_read_settles_only_matching_request_and_cleared_event_ids() {
    let mut state = ready_state();
    reduce(&mut state, AppAction::ActivityOpened { request_id: 11 });
    reduce(
        &mut state,
        AppAction::ActivitySnapshotLoaded {
            request_id: 11,
            active_tab: ActivityTab::Unread,
            recent: stream(Vec::new(), None),
            unread: stream(
                vec![
                    row("!a", "$a1", 30),
                    row("!a", "$a2", 20),
                    row("!b", "$b1", 10),
                ],
                None,
            ),
            excluded_room_ids: Vec::new(),
        },
    );

    let target = ActivityMarkReadTarget::Room {
        room_id: "!a".to_owned(),
        up_to_event_id: "$a1".to_owned(),
    };
    reduce(
        &mut state,
        AppAction::ActivityMarkReadRequested {
            request_id: 40,
            target: target.clone(),
        },
    );

    assert!(matches!(
        state.activity,
        ActivityState::Open {
            mark_read: ActivityMarkReadState::Pending { request_id: 40, .. },
            ..
        }
    ));

    assert_eq!(
        reduce(
            &mut state,
            AppAction::ActivityMarkReadSucceeded {
                request_id: 41,
                cleared_event_ids: vec!["$a1".to_owned(), "$a2".to_owned()],
            },
        ),
        Vec::new()
    );

    reduce(
        &mut state,
        AppAction::ActivityMarkReadSucceeded {
            request_id: 40,
            cleared_event_ids: vec!["$a1".to_owned(), "$a2".to_owned()],
        },
    );

    assert_eq!(
        state.activity,
        ActivityState::Open {
            active_tab: ActivityTab::Unread,
            recent: stream(Vec::new(), None),
            unread: stream(vec![row("!b", "$b1", 10)], None),
            mark_read: ActivityMarkReadState::Idle,
        }
    );
}

#[test]
fn activity_snapshot_filters_excluded_rooms_before_rendering() {
    let mut state = ready_state();
    reduce(&mut state, AppAction::ActivityOpened { request_id: 12 });

    reduce(
        &mut state,
        AppAction::ActivitySnapshotLoaded {
            request_id: 12,
            active_tab: ActivityTab::Recent,
            recent: stream(
                vec![
                    row("!normal", "$normal-recent", 30),
                    row("!low", "$low-recent", 20),
                ],
                None,
            ),
            unread: stream(
                vec![
                    row("!muted", "$muted-unread", 25),
                    row("!normal", "$normal-unread", 10),
                ],
                None,
            ),
            excluded_room_ids: vec!["!low".to_owned(), "!muted".to_owned()],
        },
    );

    let ActivityState::Open { recent, unread, .. } = &state.activity else {
        panic!("activity should be open");
    };
    assert_eq!(recent.rows, vec![row("!normal", "$normal-recent", 30)]);
    assert_eq!(unread.rows, vec![row("!normal", "$normal-unread", 10)]);
}

#[test]
fn activity_debug_output_redacts_private_values() {
    let private_row = ActivityRow {
        room_id: "!private-room:example.invalid".to_owned(),
        event_id: "$private-event:example.invalid".to_owned(),
        room_label: "Private Room".to_owned(),
        sender_label: Some("Private Sender".to_owned()),
        preview: Some("private message body".to_owned()),
        timestamp_ms: 42,
        unread: true,
        highlight: true,
    };
    let target = ActivityMarkReadTarget::Room {
        room_id: "!private-room:example.invalid".to_owned(),
        up_to_event_id: "$private-event:example.invalid".to_owned(),
    };

    let debug_values = [
        format!("{private_row:?}"),
        format!(
            "{:?}",
            stream(vec![private_row.clone()], Some("private-page-token"))
        ),
        format!(
            "{:?}",
            ActivityState::Open {
                active_tab: ActivityTab::Unread,
                recent: stream(vec![private_row.clone()], Some("private-recent-token")),
                unread: stream(vec![private_row.clone()], Some("private-unread-token")),
                mark_read: ActivityMarkReadState::Pending {
                    request_id: 1,
                    target: target.clone(),
                },
            }
        ),
        format!(
            "{:?}",
            AppAction::ActivitySnapshotLoaded {
                request_id: 2,
                active_tab: ActivityTab::Recent,
                recent: stream(vec![private_row.clone()], None),
                unread: stream(vec![private_row], None),
                excluded_room_ids: vec!["!private-room:example.invalid".to_owned()],
            }
        ),
        format!(
            "{:?}",
            AppAction::ActivityMarkReadFailed {
                request_id: 3,
                target,
                kind: OperationFailureKind::Forbidden,
            }
        ),
    ];

    for debug in debug_values {
        for private_value in [
            "!private-room:example.invalid",
            "$private-event:example.invalid",
            "Private Room",
            "Private Sender",
            "private message body",
            "private-page-token",
            "private-recent-token",
            "private-unread-token",
        ] {
            assert!(
                !debug.contains(private_value),
                "debug leaked {private_value}: {debug}"
            );
        }
    }
}
