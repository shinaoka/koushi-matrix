use koushi_state::{
    AppAction, AppEffect, AppState, OperationFailureKind, PinOp, PinOperationState, PinnedEvent,
    ReplyQuote, ReplyQuoteState, RoomSummary, RoomTags, SessionInfo, SessionState, UiEvent,
    UserProfile, reduce,
};

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://server.example.invalid".to_owned(),
            user_id: "@alice:example.invalid".to_owned(),
            device_id: "ALICEDEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        rooms: vec![RoomSummary {
            display_name_placeholder: None,
            display_label_placeholder: None,
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
        sender_label: None,
        body_preview: body_preview.map(str::to_owned),
        redacted: false,
        timestamp_ms: None,
        state: koushi_state::PinnedEventState::Ready,
        thread_root_event_id: None,
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
fn pin_completion_settles_in_ready_locked_and_switching_contexts() {
    let info = SessionInfo {
        homeserver: "https://server.example.invalid".to_owned(),
        user_id: "@alice:example.invalid".to_owned(),
        device_id: "ALICEDEVICE".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    };
    let sessions = vec![
        SessionState::Ready(info.clone()),
        SessionState::Locked(info.clone()),
        SessionState::SwitchingAccount { info: info.clone() },
        SessionState::CapabilityBlocked {
            info: info.clone(),
            failure: koushi_state::SlidingSyncCapabilityFailureKind::Unsupported,
        },
        SessionState::SignedOut,
    ];

    for session in sessions {
        let mut state = ready_state();
        reduce(
            &mut state,
            AppAction::PinEventRequested {
                request_id: 7,
                room_id: "!room:example.invalid".to_owned(),
                event_id: "$event:example.invalid".to_owned(),
            },
        );
        state.session = session.clone();

        let effects = reduce(
            &mut state,
            AppAction::PinEventCompleted {
                request_id: 7,
                room_id: "!room:example.invalid".to_owned(),
            },
        );

        if matches!(
            &session,
            SessionState::CapabilityBlocked { .. } | SessionState::SignedOut
        ) {
            assert_eq!(effects, Vec::new());
            assert!(matches!(
                state
                    .room_interactions
                    .get("!room:example.invalid")
                    .expect("room interaction state")
                    .pin_operation,
                PinOperationState::Pending { request_id: 7, .. }
            ));
        } else {
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
    }
}

#[test]
fn pin_and_unpin_failures_settle_in_locked_and_switching_contexts() {
    let info = SessionInfo {
        homeserver: "https://server.example.invalid".to_owned(),
        user_id: "@alice:example.invalid".to_owned(),
        device_id: "ALICEDEVICE".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    };
    for session in [
        SessionState::Locked(info.clone()),
        SessionState::SwitchingAccount { info: info.clone() },
    ] {
        let mut pin = ready_state();
        reduce(
            &mut pin,
            AppAction::PinEventRequested {
                request_id: 7,
                room_id: "!room:example.invalid".to_owned(),
                event_id: "$event:example.invalid".to_owned(),
            },
        );
        pin.session = session.clone();
        let effects = reduce(
            &mut pin,
            AppAction::PinEventFailed {
                request_id: 7,
                room_id: "!room:example.invalid".to_owned(),
                kind: OperationFailureKind::Network,
            },
        );
        assert!(matches!(
            pin.room_interactions["!room:example.invalid"].pin_operation,
            PinOperationState::Failed { op: PinOp::Pin, .. }
        ));
        assert_eq!(
            effects,
            vec![
                AppEffect::EmitUiEvent(UiEvent::RoomInteractionsChanged),
                AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
            ]
        );

        let mut unpin = ready_state();
        reduce(
            &mut unpin,
            AppAction::UnpinEventRequested {
                request_id: 8,
                room_id: "!room:example.invalid".to_owned(),
                event_id: "$event:example.invalid".to_owned(),
            },
        );
        unpin.session = session;
        let effects = reduce(
            &mut unpin,
            AppAction::UnpinEventFailed {
                request_id: 8,
                room_id: "!room:example.invalid".to_owned(),
                kind: OperationFailureKind::Network,
            },
        );
        assert!(matches!(
            unpin.room_interactions["!room:example.invalid"].pin_operation,
            PinOperationState::Failed {
                op: PinOp::Unpin,
                ..
            }
        ));
        assert_eq!(
            effects,
            vec![
                AppEffect::EmitUiEvent(UiEvent::RoomInteractionsChanged),
                AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
            ]
        );
    }
}

#[test]
fn pin_completion_stale_wrong_room_and_opposite_operation_are_inert_in_every_context() {
    let info = SessionInfo {
        homeserver: "https://server.example.invalid".to_owned(),
        user_id: "@alice:example.invalid".to_owned(),
        device_id: "ALICEDEVICE".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    };
    let sessions = vec![
        SessionState::Ready(info.clone()),
        SessionState::Locked(info.clone()),
        SessionState::SwitchingAccount { info: info.clone() },
        SessionState::CapabilityBlocked {
            info: info.clone(),
            failure: koushi_state::SlidingSyncCapabilityFailureKind::Unsupported,
        },
        SessionState::SignedOut,
    ];

    for session in sessions {
        let mut stale = ready_state();
        reduce(
            &mut stale,
            AppAction::PinEventRequested {
                request_id: 7,
                room_id: "!room:example.invalid".to_owned(),
                event_id: "$event:example.invalid".to_owned(),
            },
        );
        stale.session = session.clone();
        let before = stale.clone();
        assert!(
            reduce(
                &mut stale,
                AppAction::PinEventCompleted {
                    request_id: 8,
                    room_id: "!room:example.invalid".to_owned(),
                },
            )
            .is_empty()
        );
        assert_eq!(stale, before);

        let before = stale.clone();
        assert!(
            reduce(
                &mut stale,
                AppAction::PinEventCompleted {
                    request_id: 7,
                    room_id: "!other:example.invalid".to_owned(),
                },
            )
            .is_empty()
        );
        assert_eq!(stale, before);

        let mut opposite = ready_state();
        reduce(
            &mut opposite,
            AppAction::UnpinEventRequested {
                request_id: 7,
                room_id: "!room:example.invalid".to_owned(),
                event_id: "$event:example.invalid".to_owned(),
            },
        );
        opposite.session = session.clone();
        let before = opposite.clone();
        assert!(
            reduce(
                &mut opposite,
                AppAction::PinEventCompleted {
                    request_id: 7,
                    room_id: "!room:example.invalid".to_owned(),
                },
            )
            .is_empty()
        );
        assert_eq!(opposite, before);
    }
}

#[test]
fn pinned_projection_is_admitted_in_locked_and_switching_contexts() {
    for session in [
        SessionState::Locked(SessionInfo {
            homeserver: "https://server.example.invalid".to_owned(),
            user_id: "@alice:example.invalid".to_owned(),
            device_id: "ALICEDEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        SessionState::SwitchingAccount {
            info: SessionInfo {
                homeserver: "https://server.example.invalid".to_owned(),
                user_id: "@alice:example.invalid".to_owned(),
                device_id: "ALICEDEVICE".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            },
        },
    ] {
        let mut state = ready_state();
        state.session = session;
        let effects = reduce(
            &mut state,
            AppAction::RoomPinnedEventsUpdated {
                room_id: "!room:example.invalid".to_owned(),
                pinned: vec![pinned("$transient:example.invalid", Some("pinned"))],
            },
        );
        assert_eq!(
            state.room_interactions["!room:example.invalid"].pinned_events,
            vec![pinned("$transient:example.invalid", Some("pinned"))]
        );
        assert_eq!(
            effects,
            vec![AppEffect::EmitUiEvent(UiEvent::RoomInteractionsChanged)]
        );
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
fn pinned_projection_preserves_order_and_thread_relation_metadata() {
    let mut state = ready_state();
    let mut first = pinned("$first:example.invalid", Some("first"));
    first.timestamp_ms = Some(1_800_000_000_000);
    first.state = koushi_state::PinnedEventState::Ready;
    let mut reply = pinned("$reply:example.invalid", Some("reply"));
    reply.thread_root_event_id = Some("$root:example.invalid".to_owned());

    reduce(
        &mut state,
        AppAction::RoomPinnedEventsUpdated {
            room_id: "!room:example.invalid".to_owned(),
            pinned: vec![first.clone(), reply.clone()],
        },
    );

    assert_eq!(
        state.room_interactions["!room:example.invalid"].pinned_events,
        vec![first, reply]
    );
}

#[test]
fn pinned_state_projects_optional_friendly_sender_label() {
    let mut state = ready_state();
    state.profile.users.insert(
        "@bob:example.invalid".to_owned(),
        UserProfile {
            user_id: "@bob:example.invalid".to_owned(),
            display_name: Some("Bob".to_owned()),
            display_label: String::new(),
            original_display_label: String::new(),
            mention_search_terms: Vec::new(),
            avatar: None,
        },
    );
    let mut event = pinned("$friendly:example.invalid", Some("hello"));
    event.sender = Some("@bob:example.invalid".to_owned());

    reduce(
        &mut state,
        AppAction::RoomPinnedEventsUpdated {
            room_id: "!room:example.invalid".to_owned(),
            pinned: vec![event],
        },
    );

    assert_eq!(
        state.room_interactions["!room:example.invalid"].pinned_events[0]
            .sender_label
            .as_deref(),
        Some("Bob")
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

#[test]
fn reply_quote_state_wire_names_cover_hydration_lifecycle() {
    for (state, wire) in [
        (ReplyQuoteState::Loading, "loading"),
        (ReplyQuoteState::Ready, "ready"),
        (ReplyQuoteState::Redacted, "redacted"),
        (ReplyQuoteState::Missing, "missing"),
        (ReplyQuoteState::Unsupported, "unsupported"),
        (ReplyQuoteState::Failed, "failed"),
    ] {
        assert_eq!(state.as_str(), wire);
        assert_eq!(
            serde_json::to_value(state).expect("serialize reply quote state"),
            serde_json::Value::String(wire.to_owned())
        );
    }
}
