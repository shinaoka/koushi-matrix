use koushi_state::{
    AppAction, AppEffect, AppState, HistoryExportFailureKind, HistoryExportRange,
    HistoryExportRoom, HistoryExportRoomCounts, HistoryExportRoomFailureKind,
    HistoryExportRoomPhase, HistoryExportRoomSkipReason, HistoryExportScope, HistoryExportState,
    RoomSummary, RoomTags, SessionInfo, SessionState, SpaceSummary, UiEvent, reduce,
};

const ROOM: &str = "!history:example.invalid";
const SPACE: &str = "!space:example.invalid";

fn changed() -> Vec<AppEffect> {
    vec![AppEffect::EmitUiEvent(UiEvent::HistoryExportChanged)]
}

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.invalid".to_owned(),
        user_id: "@user-a:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    }
}

fn room_summary(room_id: &str) -> RoomSummary {
    RoomSummary {
        display_name_placeholder: None,
        display_label_placeholder: None,
        room_id: room_id.to_owned(),
        display_name: "Synthetic Room".to_owned(),
        display_label: "Synthetic Room".to_owned(),
        original_display_label: "Synthetic Room".to_owned(),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        thread_unread_count: 0,
        thread_highlight_count: 0,
        marked_unread: false,
        recency_stamp: None,
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: true,
        joined_members: 2,
    }
}

fn space_summary() -> SpaceSummary {
    serde_json::from_value(serde_json::json!({
        "space_id": SPACE,
        "raw_name": "Lab",
        "display_name": "Lab",
        "display_label": "Lab",
        "original_display_label": "Lab",
        "child_room_ids": [ROOM],
    }))
    .expect("space summary fixture")
}

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(session_info()),
        rooms: vec![room_summary(ROOM)],
        spaces: vec![space_summary()],
        ..AppState::default()
    }
}

fn room_scope() -> HistoryExportScope {
    HistoryExportScope::Room {
        room_id: ROOM.to_owned(),
    }
}

fn space_scope() -> HistoryExportScope {
    HistoryExportScope::Space {
        space_id: SPACE.to_owned(),
    }
}

fn export_room(room_id: &str, phase: HistoryExportRoomPhase) -> HistoryExportRoom {
    HistoryExportRoom {
        room_id: room_id.to_owned(),
        display_name: format!("name {room_id}"),
        phase,
        counts: HistoryExportRoomCounts::default(),
        skip_reason: (phase == HistoryExportRoomPhase::Skipped)
            .then_some(HistoryExportRoomSkipReason::NotJoined),
        failure_kind: None,
    }
}

fn request(state: &mut AppState, request_id: u64, scope: HistoryExportScope) -> Vec<AppEffect> {
    reduce(
        state,
        AppAction::HistoryExportRequested {
            request_id,
            scope,
            range: HistoryExportRange::AllAvailable,
        },
    )
}

fn prepared(
    state: &mut AppState,
    request_id: u64,
    rooms: Vec<HistoryExportRoom>,
) -> Vec<AppEffect> {
    reduce(
        state,
        AppAction::HistoryExportPrepared { request_id, rooms },
    )
}

fn progressed(
    state: &mut AppState,
    request_id: u64,
    room_id: &str,
    phase: HistoryExportRoomPhase,
    fetched: u64,
) -> Vec<AppEffect> {
    reduce(
        state,
        AppAction::HistoryExportRoomProgressed {
            request_id,
            room_id: room_id.to_owned(),
            phase,
            counts: HistoryExportRoomCounts {
                fetched_events: fetched,
                ..HistoryExportRoomCounts::default()
            },
        },
    )
}

fn rooms(state: &AppState) -> &[HistoryExportRoom] {
    match &state.history_export {
        HistoryExportState::Running { rooms, .. }
        | HistoryExportState::Completed { rooms, .. }
        | HistoryExportState::Stopped { rooms, .. }
        | HistoryExportState::Failed { rooms, .. } => rooms,
        other => panic!("expected rooms, got {other:?}"),
    }
}

fn running_space(state: &mut AppState) {
    request(state, 1, space_scope());
    prepared(
        state,
        1,
        vec![
            export_room("!a:x", HistoryExportRoomPhase::Pending),
            export_room("!b:x", HistoryExportRoomPhase::Pending),
            export_room("!c:x", HistoryExportRoomPhase::Skipped),
        ],
    );
}

#[test]
fn request_requires_ready_session_known_target_and_valid_range() {
    let mut signed_out = AppState {
        rooms: vec![room_summary(ROOM)],
        ..AppState::default()
    };
    assert!(request(&mut signed_out, 1, room_scope()).is_empty());
    assert_eq!(signed_out.history_export, HistoryExportState::Idle);

    let mut state = ready_state();
    let unknown_room = HistoryExportScope::Room {
        room_id: "!unknown:example.invalid".to_owned(),
    };
    assert!(request(&mut state, 1, unknown_room).is_empty());
    let unknown_space = HistoryExportScope::Space {
        space_id: "!other-space:example.invalid".to_owned(),
    };
    assert!(request(&mut state, 1, unknown_space).is_empty());
    let empty_period = reduce(
        &mut state,
        AppAction::HistoryExportRequested {
            request_id: 2,
            scope: room_scope(),
            range: HistoryExportRange::Period {
                start_ms: 10,
                end_exclusive_ms: 10,
                time_zone: "UTC".to_owned(),
            },
        },
    );
    assert!(empty_period.is_empty());
    assert_eq!(state.history_export, HistoryExportState::Idle);

    assert_eq!(request(&mut state, 3, space_scope()), changed());
    assert_eq!(
        state.history_export,
        HistoryExportState::Preparing {
            request_id: 3,
            scope: space_scope(),
            range: HistoryExportRange::AllAvailable,
            stop_requested: false,
        }
    );
}

#[test]
fn second_request_is_rejected_while_one_is_in_flight() {
    let mut state = ready_state();
    request(&mut state, 1, room_scope());
    assert!(request(&mut state, 2, space_scope()).is_empty());
    prepared(
        &mut state,
        1,
        vec![export_room(ROOM, HistoryExportRoomPhase::Pending)],
    );
    assert!(request(&mut state, 3, space_scope()).is_empty());
    assert_eq!(state.history_export.active_request_id(), Some(1));
}

#[test]
fn prepared_moves_to_running_with_rooms_and_ignores_stale_requests() {
    let mut state = ready_state();
    request(&mut state, 1, space_scope());
    assert!(prepared(&mut state, 9, Vec::new()).is_empty());
    assert_eq!(
        prepared(
            &mut state,
            1,
            vec![export_room("!a:x", HistoryExportRoomPhase::Pending)]
        ),
        changed()
    );
    assert!(matches!(
        state.history_export,
        HistoryExportState::Running {
            request_id: 1,
            stop_requested: false,
            ..
        }
    ));
    assert_eq!(rooms(&state).len(), 1);
    assert!(
        prepared(&mut state, 1, Vec::new()).is_empty(),
        "prepared twice"
    );
}

#[test]
fn room_progress_ignores_unknown_rooms_stale_requests_backward_phases_and_duplicates() {
    let mut state = ready_state();
    running_space(&mut state);
    assert_eq!(
        progressed(&mut state, 1, "!a:x", HistoryExportRoomPhase::Fetching, 10),
        changed()
    );
    assert!(progressed(&mut state, 1, "!a:x", HistoryExportRoomPhase::Fetching, 10).is_empty());
    assert!(progressed(&mut state, 2, "!a:x", HistoryExportRoomPhase::Fetching, 20).is_empty());
    assert!(progressed(&mut state, 1, "!zz:x", HistoryExportRoomPhase::Fetching, 1).is_empty());
    progressed(
        &mut state,
        1,
        "!a:x",
        HistoryExportRoomPhase::Attachments,
        30,
    );
    assert!(progressed(&mut state, 1, "!a:x", HistoryExportRoomPhase::Fetching, 40).is_empty());
    assert!(
        progressed(&mut state, 1, "!a:x", HistoryExportRoomPhase::Completed, 40).is_empty(),
        "progress cannot settle a room"
    );
    assert_eq!(rooms(&state)[0].phase, HistoryExportRoomPhase::Attachments);
    assert_eq!(rooms(&state)[0].counts.fetched_events, 30);
}

#[test]
fn rooms_settle_once_and_skipped_rooms_cannot_change() {
    let mut state = ready_state();
    running_space(&mut state);
    let settle = |state: &mut AppState, room_id: &str, phase, failure_kind| {
        reduce(
            state,
            AppAction::HistoryExportRoomSettled {
                request_id: 1,
                room_id: room_id.to_owned(),
                phase,
                counts: HistoryExportRoomCounts {
                    exported_events: 5,
                    ..HistoryExportRoomCounts::default()
                },
                failure_kind,
            },
        )
    };
    assert_eq!(
        settle(&mut state, "!a:x", HistoryExportRoomPhase::Completed, None),
        changed()
    );
    assert!(settle(&mut state, "!a:x", HistoryExportRoomPhase::Failed, None).is_empty());
    assert_eq!(
        settle(
            &mut state,
            "!b:x",
            HistoryExportRoomPhase::Failed,
            Some(HistoryExportRoomFailureKind::Network)
        ),
        changed()
    );
    assert!(settle(&mut state, "!c:x", HistoryExportRoomPhase::Completed, None).is_empty());
    assert!(
        settle(
            &mut state,
            "!c:x",
            HistoryExportRoomPhase::Attachments,
            None
        )
        .is_empty(),
        "only settled phases settle"
    );
    let rooms = rooms(&state);
    assert_eq!(rooms[0].phase, HistoryExportRoomPhase::Completed);
    assert_eq!(rooms[0].counts.exported_events, 5);
    assert_eq!(
        rooms[1].failure_kind,
        Some(HistoryExportRoomFailureKind::Network)
    );
    assert_eq!(rooms[2].phase, HistoryExportRoomPhase::Skipped);
}

#[test]
fn stop_is_accepted_once_while_preparing_or_running() {
    let mut state = ready_state();
    request(&mut state, 1, space_scope());
    assert!(
        reduce(
            &mut state,
            AppAction::HistoryExportStopRequested { request_id: 2 }
        )
        .is_empty()
    );
    assert_eq!(
        reduce(
            &mut state,
            AppAction::HistoryExportStopRequested { request_id: 1 }
        ),
        changed()
    );
    assert!(
        reduce(
            &mut state,
            AppAction::HistoryExportStopRequested { request_id: 1 }
        )
        .is_empty()
    );
    prepared(
        &mut state,
        1,
        vec![export_room("!a:x", HistoryExportRoomPhase::Pending)],
    );
    assert!(matches!(
        state.history_export,
        HistoryExportState::Running {
            stop_requested: true,
            ..
        }
    ));
    assert_eq!(
        reduce(
            &mut state,
            AppAction::HistoryExportStopped { request_id: 1 }
        ),
        changed()
    );
    assert!(matches!(
        state.history_export,
        HistoryExportState::Stopped { request_id: 1, .. }
    ));
    assert!(
        reduce(
            &mut state,
            AppAction::HistoryExportStopRequested { request_id: 1 }
        )
        .is_empty()
    );
}

#[test]
fn terminal_settlements_keep_rooms_and_ignore_stale_or_duplicate_ones() {
    let mut state = ready_state();
    running_space(&mut state);
    assert!(
        reduce(
            &mut state,
            AppAction::HistoryExportCompleted { request_id: 7 }
        )
        .is_empty()
    );
    assert_eq!(
        reduce(
            &mut state,
            AppAction::HistoryExportCompleted { request_id: 1 }
        ),
        changed()
    );
    assert!(matches!(
        state.history_export,
        HistoryExportState::Completed { request_id: 1, .. }
    ));
    assert_eq!(rooms(&state).len(), 3);
    assert!(
        reduce(
            &mut state,
            AppAction::HistoryExportCompleted { request_id: 1 }
        )
        .is_empty()
    );
    assert!(
        reduce(
            &mut state,
            AppAction::HistoryExportFailed {
                request_id: 1,
                kind: HistoryExportFailureKind::NoSpace
            }
        )
        .is_empty()
    );
}

#[test]
fn preparation_failure_settles_without_rooms() {
    let mut state = ready_state();
    request(&mut state, 1, space_scope());
    assert_eq!(
        reduce(
            &mut state,
            AppAction::HistoryExportFailed {
                request_id: 1,
                kind: HistoryExportFailureKind::ManifestMismatch
            }
        ),
        changed()
    );
    assert_eq!(
        state.history_export,
        HistoryExportState::Failed {
            request_id: 1,
            scope: space_scope(),
            range: HistoryExportRange::AllAvailable,
            rooms: Vec::new(),
            failure_kind: HistoryExportFailureKind::ManifestMismatch,
        }
    );
}

#[test]
fn retry_requires_the_matching_terminal_request_and_keeps_scope_and_range() {
    let mut state = ready_state();
    running_space(&mut state);
    let retry = |state: &mut AppState, request_id, target_request_id| {
        reduce(
            state,
            AppAction::HistoryExportRetryRequested {
                request_id,
                target_request_id,
            },
        )
    };
    assert!(retry(&mut state, 2, 1).is_empty(), "not while running");
    reduce(
        &mut state,
        AppAction::HistoryExportStopped { request_id: 1 },
    );
    assert!(retry(&mut state, 2, 5).is_empty(), "wrong target");
    assert_eq!(retry(&mut state, 2, 1), changed());
    assert_eq!(
        state.history_export,
        HistoryExportState::Preparing {
            request_id: 2,
            scope: space_scope(),
            range: HistoryExportRange::AllAvailable,
            stop_requested: false,
        }
    );
    assert!(retry(&mut AppState::default(), 3, 2).is_empty());
}

#[test]
fn a_new_request_replaces_a_terminal_state() {
    let mut state = ready_state();
    running_space(&mut state);
    reduce(
        &mut state,
        AppAction::HistoryExportCompleted { request_id: 1 },
    );
    assert_eq!(request(&mut state, 2, room_scope()), changed());
    assert_eq!(state.history_export.active_request_id(), Some(2));
}

#[test]
fn logout_and_lock_reset_the_export() {
    for action in [AppAction::LogoutRequested, AppAction::SessionLocked] {
        let mut state = ready_state();
        running_space(&mut state);
        let effects = reduce(&mut state, action);
        assert_eq!(state.history_export, HistoryExportState::Idle);
        assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::HistoryExportChanged)));
    }
}

#[test]
fn range_inclusion_is_start_inclusive_end_exclusive() {
    let range = HistoryExportRange::Period {
        start_ms: 1_700_000_000_000,
        end_exclusive_ms: 1_700_086_400_000,
        time_zone: "Asia/Tokyo".to_owned(),
    };
    assert!(!range.contains(1_699_999_999_999));
    assert!(range.contains(1_700_000_000_000));
    assert!(range.contains(1_700_086_399_999));
    assert!(!range.contains(1_700_086_400_000));
    assert!(HistoryExportRange::AllAvailable.contains(0));
}

#[test]
fn actions_debug_redacts_identifiers() {
    let action = AppAction::HistoryExportRoomProgressed {
        request_id: 1,
        room_id: "!private-room:example.invalid".to_owned(),
        phase: HistoryExportRoomPhase::Fetching,
        counts: HistoryExportRoomCounts::default(),
    };
    assert!(!format!("{action:?}").contains("private-room"));
    let action = AppAction::HistoryExportRequested {
        request_id: 1,
        scope: HistoryExportScope::Space {
            space_id: "!private-space:example.invalid".to_owned(),
        },
        range: HistoryExportRange::AllAvailable,
    };
    assert!(!format!("{action:?}").contains("private-space"));
}
