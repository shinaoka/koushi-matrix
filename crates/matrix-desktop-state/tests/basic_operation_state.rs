use matrix_desktop_state::{
    reduce, AppAction, AppState, BasicOperationState, ComposerMode,
    ComposerState, RoomSummary, SessionInfo, SessionState, TimelinePaneState,
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
            is_dm: false,
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            parent_space_ids: vec![],
        }],
        timeline: TimelinePaneState {
            room_id: Some("!room:localhost".to_owned()),
            is_subscribed: true,
            is_paginating_backwards: false,
            composer: ComposerState::default(),
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
fn room_creation_status_is_rust_state() {
    let mut state = ready_state();

    reduce(
        &mut state,
        AppAction::BasicOperationStarted {
            operation: BasicOperationState::CreatingRoom {
                name: "Local QA Room".to_owned(),
            },
        },
    );

    assert_eq!(
        state.basic_operation,
        BasicOperationState::CreatingRoom {
            name: "Local QA Room".to_owned(),
        }
    );

    reduce(&mut state, AppAction::BasicOperationFinished);
    assert_eq!(state.basic_operation, BasicOperationState::Idle);
}
