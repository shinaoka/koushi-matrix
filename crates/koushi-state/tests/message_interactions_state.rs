use koushi_state::{
    AppAction, AppEffect, AppState, OperationFailureKind, PinOp, PinOperationState, PinnedEvent,
    ReplyQuote, ReplyQuoteState, RoomSummary, RoomTags, SessionInfo, SessionState, UiEvent, reduce,
};

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://server.example.invalid".to_owned(),
            user_id: "@alice:example.invalid".to_owned(),
            device_id: "ALICEDEVICE".to_owned(),
        }),
        rooms: vec![RoomSummary {
            room_id: "!room:example.invalid".to_owned(),
            display_name: "Room".to_owned(),
            display_label: "Room".to_owned(),
            original_display_label: "Room".to_owned(),
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
            parent_space_ids: Vec::new(),
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 0,
        }],
        ..AppState::default()
    }
}

fn pinned(event_id: &str, body_preview: Option<&str>) -> PinnedEvent {
    PinnedEvent {
        event_id: event_id.to_owned(),
        sender: Some("Alice".to_owned()),
        body_preview: body_preview.map(str::to_owned),
        redacted: false,
    }
}

#[test]
fn pin_request_enters_pending_when_session_is_ready() {
    let mut state = ready_state();

    let effects = reduce(
        &mut state,
        AppAction::PinEventRequested {
            request_id: 7,
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$event:example.invalid".to_owned(),
        },
    );

    assert_eq!(
        state
            .room_interactions
            .get("!room:example.invalid")
            .expect("room interaction state")
            .pin_operation,
        PinOperationState::Pending {
            request_id: 7,
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$event:example.invalid".to_owned(),
            op: PinOp::Pin,
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomInteractionsChanged)]
    );
}

#[test]
fn second_pin_in_same_room_is_ignored_while_pending() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::PinEventRequested {
            request_id: 7,
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$first:example.invalid".to_owned(),
        },
    );

    assert_eq!(
        reduce(
            &mut state,
            AppAction::PinEventRequested {
                request_id: 8,
                room_id: "!room:example.invalid".to_owned(),
                event_id: "$second:example.invalid".to_owned(),
            },
        ),
        Vec::new()
    );
    assert_eq!(
        state
            .room_interactions
            .get("!room:example.invalid")
            .expect("room interaction state")
            .pin_operation,
        PinOperationState::Pending {
            request_id: 7,
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$first:example.invalid".to_owned(),
            op: PinOp::Pin,
        }
    );
}

#[test]
fn stale_pin_completion_is_ignored() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::PinEventRequested {
            request_id: 7,
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$event:example.invalid".to_owned(),
        },
    );

    assert_eq!(
        reduce(
            &mut state,
            AppAction::PinEventCompleted {
                request_id: 8,
                room_id: "!room:example.invalid".to_owned(),
            },
        ),
        Vec::new()
    );
    assert!(matches!(
        state
            .room_interactions
            .get("!room:example.invalid")
            .expect("room interaction state")
            .pin_operation,
        PinOperationState::Pending { request_id: 7, .. }
    ));
}

#[test]
fn pin_completion_is_ignored_after_session_leaves_ready() {
    for session in [
        SessionState::Locked(SessionInfo {
            homeserver: "https://server.example.invalid".to_owned(),
            user_id: "@alice:example.invalid".to_owned(),
            device_id: "ALICEDEVICE".to_owned(),
        }),
        SessionState::SwitchingAccount {
            info: SessionInfo {
                homeserver: "https://server.example.invalid".to_owned(),
                user_id: "@alice:example.invalid".to_owned(),
                device_id: "ALICEDEVICE".to_owned(),
            },
        },
    ] {
        let mut state = ready_state();
        reduce(
            &mut state,
            AppAction::PinEventRequested {
                request_id: 7,
                room_id: "!room:example.invalid".to_owned(),
                event_id: "$event:example.invalid".to_owned(),
            },
        );
        state.session = session;

        let effects = reduce(
            &mut state,
            AppAction::PinEventCompleted {
                request_id: 7,
                room_id: "!room:example.invalid".to_owned(),
            },
        );

        assert_eq!(
            state
                .room_interactions
                .get("!room:example.invalid")
                .expect("room interaction state")
                .pin_operation,
            PinOperationState::Pending {
                request_id: 7,
                room_id: "!room:example.invalid".to_owned(),
                event_id: "$event:example.invalid".to_owned(),
                op: koushi_state::PinOp::Pin,
            }
        );
        assert!(effects.is_empty());
    }
}

#[test]
fn pin_failure_sets_failed_state_and_error_for_matching_request() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::PinEventRequested {
            request_id: 7,
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$event:example.invalid".to_owned(),
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::PinEventFailed {
            request_id: 7,
            room_id: "!room:example.invalid".to_owned(),
            kind: OperationFailureKind::Network,
        },
    );

    assert_eq!(
        state
            .room_interactions
            .get("!room:example.invalid")
            .expect("room interaction state")
            .pin_operation,
        PinOperationState::Failed {
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$event:example.invalid".to_owned(),
            op: PinOp::Pin,
            recoverable: true,
        }
    );
    assert_eq!(
        state.errors.last().expect("pin failure error").code,
        "pin_event_failed"
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::RoomInteractionsChanged),
            AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
        ]
    );
}

#[test]
fn recoverable_pin_failure_can_be_retried() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::PinEventRequested {
            request_id: 7,
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$event:example.invalid".to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::PinEventFailed {
            request_id: 7,
            room_id: "!room:example.invalid".to_owned(),
            kind: OperationFailureKind::Network,
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::PinEventRequested {
            request_id: 8,
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$event:example.invalid".to_owned(),
        },
    );

    assert_eq!(
        state
            .room_interactions
            .get("!room:example.invalid")
            .expect("room interaction state")
            .pin_operation,
        PinOperationState::Pending {
            request_id: 8,
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$event:example.invalid".to_owned(),
            op: PinOp::Pin,
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomInteractionsChanged)]
    );
}

#[test]
fn unpin_request_completes_only_matching_unpin_operation() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::UnpinEventRequested {
            request_id: 9,
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$event:example.invalid".to_owned(),
        },
    );

    assert_eq!(
        reduce(
            &mut state,
            AppAction::PinEventCompleted {
                request_id: 9,
                room_id: "!room:example.invalid".to_owned(),
            },
        ),
        Vec::new()
    );

    let effects = reduce(
        &mut state,
        AppAction::UnpinEventCompleted {
            request_id: 9,
            room_id: "!room:example.invalid".to_owned(),
        },
    );

    assert_eq!(
        state
            .room_interactions
            .get("!room:example.invalid")
            .expect("room interaction state")
            .pin_operation,
        PinOperationState::Idle
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomInteractionsChanged)]
    );
}

#[test]
fn invalid_pin_inputs_do_not_create_room_interaction_state() {
    let mut state = ready_state();

    assert_eq!(
        reduce(
            &mut state,
            AppAction::PinEventRequested {
                request_id: 7,
                room_id: "!missing:example.invalid".to_owned(),
                event_id: "$event:example.invalid".to_owned(),
            },
        ),
        Vec::new()
    );
    assert_eq!(
        reduce(
            &mut state,
            AppAction::PinEventRequested {
                request_id: 8,
                room_id: "!room:example.invalid".to_owned(),
                event_id: String::new(),
            },
        ),
        Vec::new()
    );

    assert!(state.room_interactions.is_empty());
}

#[test]
fn pinned_state_update_replaces_room_pinned_list() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomPinnedEventsUpdated {
            room_id: "!room:example.invalid".to_owned(),
            pinned: vec![pinned("$one:example.invalid", Some("one"))],
        },
    );

    reduce(
        &mut state,
        AppAction::RoomPinnedEventsUpdated {
            room_id: "!room:example.invalid".to_owned(),
            pinned: vec![pinned("$two:example.invalid", None)],
        },
    );

    assert_eq!(
        state
            .room_interactions
            .get("!room:example.invalid")
            .expect("room interaction state")
            .pinned_events,
        vec![pinned("$two:example.invalid", None)]
    );
}

#[test]
fn logout_clears_room_interactions() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomPinnedEventsUpdated {
            room_id: "!room:example.invalid".to_owned(),
            pinned: vec![pinned("$one:example.invalid", Some("one"))],
        },
    );

    reduce(&mut state, AppAction::LogoutRequested);

    assert!(state.room_interactions.is_empty());
}

#[test]
fn reply_quote_dto_can_represent_absent_non_reply_quote() {
    let reply_quote: Option<ReplyQuote> = None;

    assert!(reply_quote.is_none());
    assert_eq!(ReplyQuoteState::Ready.as_str(), "ready");
}
