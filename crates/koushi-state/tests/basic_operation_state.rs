use koushi_state::{
    AppAction, AppState, BasicOperationRequest, BasicOperationState, ComposerMode, ComposerState,
    RoomSummary, RoomTags, SessionInfo, SessionState, TimelinePaneState, reduce,
};

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "http://127.0.0.1:6167".to_owned(),
            user_id: "@qa:localhost".to_owned(),
            device_id: "LOCALDEVICE".to_owned(),
        }),
        rooms: vec![RoomSummary {
            room_id: "!room:localhost".to_owned(),
            display_name: "QA Seed Room".to_owned(),
            display_label: "QA Seed Room".to_owned(),
            original_display_label: "QA Seed Room".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            marked_unread: false,
            recency_stamp: None,
            conversation_activity: None,
            latest_event: None,
            parent_space_ids: vec![],
            dm_space_ids: vec![],
            is_encrypted: false,
            joined_members: 0,
        }],
        timeline: TimelinePaneState {
            room_id: Some("!room:localhost".to_owned()),
            is_subscribed: true,
            is_paginating_backwards: false,
            composer: ComposerState::default(),
            submission_registry: Default::default(),
            scheduled_send_capability: Default::default(),
            scheduled_sends: Vec::new(),
            staged_uploads: Vec::new(),
            media_gallery: Vec::new(),
            media_downloads: Default::default(),
            continuity: Default::default(),
        },
        ..AppState::default()
    }
}

#[test]
fn composer_reply_target_is_rust_state() {
    let mut state = ready_state();

    reduce(
        &mut state,
        AppAction::ComposerReplyTargetSelected {
            room_id: "!room:localhost".to_owned(),
            event_id: "$root:localhost".to_owned(),
        },
    );

    assert_eq!(
        state.timeline.composer.mode,
        ComposerMode::Reply {
            in_reply_to_event_id: "$root:localhost".to_owned(),
        }
    );

    reduce(&mut state, AppAction::ComposerReplyCancelled);
    assert_eq!(state.timeline.composer.mode, ComposerMode::Plain);
}

#[test]
fn basic_operation_tracks_request_and_settles_on_matching_success() {
    let mut state = ready_state();

    reduce(
        &mut state,
        AppAction::BasicOperationRequested {
            request_id: 7,
            request: BasicOperationRequest::CreateRoom {
                name: "Local QA Room".to_owned(),
            },
        },
    );
    assert_eq!(
        state.basic_operation,
        BasicOperationState::CreatingRoom {
            request_id: 7,
            name: "Local QA Room".to_owned(),
        }
    );

    // A completion that does not correlate to the in-flight request is ignored.
    reduce(
        &mut state,
        AppAction::BasicOperationSucceeded { request_id: 999 },
    );
    assert!(!state.basic_operation.is_idle());

    // The correlated completion settles the operation back to Idle.
    reduce(
        &mut state,
        AppAction::BasicOperationSucceeded { request_id: 7 },
    );
    assert_eq!(state.basic_operation, BasicOperationState::Idle);
}

#[test]
fn basic_operation_rejects_a_second_request_while_in_flight() {
    let mut state = ready_state();

    reduce(
        &mut state,
        AppAction::BasicOperationRequested {
            request_id: 1,
            request: BasicOperationRequest::CreateRoom {
                name: "First".to_owned(),
            },
        },
    );
    // No-clobber: a second request while one is already pending is ignored.
    reduce(
        &mut state,
        AppAction::BasicOperationRequested {
            request_id: 2,
            request: BasicOperationRequest::CreateSpace {
                name: "Second".to_owned(),
            },
        },
    );

    assert_eq!(
        state.basic_operation,
        BasicOperationState::CreatingRoom {
            request_id: 1,
            name: "First".to_owned(),
        }
    );
}

#[test]
fn basic_operation_failure_requires_a_matching_in_flight_request() {
    let mut state = ready_state();

    // A failure while Idle correlates to nothing: ignored, records no error.
    reduce(
        &mut state,
        AppAction::BasicOperationFailed {
            request_id: 1,
            message: "boom".to_owned(),
        },
    );
    assert_eq!(state.basic_operation, BasicOperationState::Idle);
    assert!(state.errors.is_empty());

    // An in-flight failure with the matching id settles to Idle and records it.
    reduce(
        &mut state,
        AppAction::BasicOperationRequested {
            request_id: 5,
            request: BasicOperationRequest::CreateSpace {
                name: "QA".to_owned(),
            },
        },
    );
    reduce(
        &mut state,
        AppAction::BasicOperationFailed {
            request_id: 5,
            message: "boom".to_owned(),
        },
    );
    assert_eq!(state.basic_operation, BasicOperationState::Idle);
    assert_eq!(state.errors.len(), 1);
    assert_eq!(state.errors[0].code, "basic_operation_failed");
}

#[test]
fn basic_operation_request_requires_a_ready_session() {
    let mut state = AppState::default();

    reduce(
        &mut state,
        AppAction::BasicOperationRequested {
            request_id: 1,
            request: BasicOperationRequest::CreateRoom {
                name: "QA".to_owned(),
            },
        },
    );
    assert_eq!(state.basic_operation, BasicOperationState::Idle);
}
