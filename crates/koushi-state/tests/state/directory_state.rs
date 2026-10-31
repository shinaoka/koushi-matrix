use koushi_state::{
    AppAction, AppEffect, AppState, DirectoryJoinState, DirectoryPreviewJoinability,
    DirectoryPreviewMembership, DirectoryPreviewState, DirectoryQuery, DirectoryQueryState,
    DirectoryRoomPreview, DirectoryRoomSummary, DirectoryState, OperationFailureKind, SessionInfo,
    SessionState, UiEvent, reduce,
};

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.org".to_owned(),
        user_id: "@user-a:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    }
}

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(session_info()),
        ..AppState::default()
    }
}

fn query() -> DirectoryQuery {
    DirectoryQuery {
        term: Some("synthetic".to_owned()),
        server_name: Some("example.invalid".to_owned()),
        limit: Some(10),
        since: None,
    }
}

fn room(alias: &str) -> DirectoryRoomSummary {
    DirectoryRoomSummary {
        room_id: "!room:example.invalid".to_owned(),
        canonical_alias: Some(alias.to_owned()),
        room_type: None,
        name: "Synthetic Directory Room".to_owned(),
        topic: Some("Synthetic public topic".to_owned()),
        avatar_url: None,
        joined_members: 2,
        world_readable: true,
        guest_can_join: false,
    }
}

#[test]
fn directory_debug_output_redacts_private_directory_values() {
    let private_query = DirectoryQuery {
        term: Some("private directory search".to_owned()),
        server_name: Some("private.example.invalid".to_owned()),
        limit: Some(20),
        since: Some("private-page-token".to_owned()),
    };
    let private_room = DirectoryRoomSummary {
        room_id: "!private-room:example.invalid".to_owned(),
        canonical_alias: Some("#private-room:example.invalid".to_owned()),
        room_type: None,
        name: "Private Directory Room".to_owned(),
        topic: Some("Private directory topic".to_owned()),
        avatar_url: Some("mxc://example.invalid/private-avatar".to_owned()),
        joined_members: 3,
        world_readable: true,
        guest_can_join: false,
    };

    let debug_values = [
        format!("{private_query:?}"),
        format!(
            "{:?}",
            DirectoryQueryState::Results {
                request_id: 21,
                query: private_query.clone(),
                rooms: vec![private_room.clone()],
                next_batch: Some("private-next-page".to_owned()),
            }
        ),
        format!(
            "{:?}",
            DirectoryJoinState::Joining {
                request_id: 22,
                room_id_or_alias: "#private-room:example.invalid".to_owned(),
                via_servers: vec!["private.example.invalid".to_owned()],
            }
        ),
        format!(
            "{:?}",
            AppAction::DirectoryQuerySucceeded {
                request_id: 23,
                query: private_query.clone(),
                rooms: vec![private_room],
                next_batch: Some("private-next-page".to_owned()),
            }
        ),
        format!(
            "{:?}",
            AppAction::DirectoryJoinRequested {
                request_id: 24,
                room_id_or_alias: "#private-room:example.invalid".to_owned(),
                via_servers: vec!["private.example.invalid".to_owned()],
            }
        ),
        format!(
            "{:?}",
            AppAction::DirectoryJoinSucceeded {
                request_id: 25,
                room_id: "!private-room:example.invalid".to_owned(),
            }
        ),
        format!(
            "{:?}",
            AppAction::DirectoryJoinFailed {
                request_id: 26,
                room_id_or_alias: "#private-room:example.invalid".to_owned(),
                via_servers: vec!["private.example.invalid".to_owned()],
                kind: OperationFailureKind::Forbidden,
            }
        ),
    ];

    for debug in debug_values {
        for private_value in [
            "private directory search",
            "private.example.invalid",
            "private-page-token",
            "private-next-page",
            "!private-room:example.invalid",
            "#private-room:example.invalid",
            "Private Directory Room",
            "Private directory topic",
            "mxc://example.invalid/private-avatar",
        ] {
            assert!(
                !debug.contains(private_value),
                "debug leaked {private_value}: {debug}"
            );
        }
    }
}

#[test]
fn directory_query_lifecycle_is_request_correlated() {
    let mut state = ready_state();
    let query = query();

    let effects = reduce(
        &mut state,
        AppAction::DirectoryQueryRequested {
            request_id: 7,
            query: query.clone(),
        },
    );

    assert_eq!(
        state.directory,
        DirectoryState {
            query: DirectoryQueryState::Querying {
                request_id: 7,
                query: query.clone(),
            },
            preview: DirectoryPreviewState::Closed,
            join: DirectoryJoinState::Idle,
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::DirectoryChanged)]
    );

    assert_eq!(
        reduce(
            &mut state,
            AppAction::DirectoryQuerySucceeded {
                request_id: 8,
                query: query.clone(),
                rooms: vec![room("#synthetic:example.invalid")],
                next_batch: Some("stale".to_owned()),
            },
        ),
        Vec::new()
    );
    assert!(matches!(
        state.directory.query,
        DirectoryQueryState::Querying { request_id: 7, .. }
    ));

    reduce(
        &mut state,
        AppAction::DirectoryQuerySucceeded {
            request_id: 7,
            query: query.clone(),
            rooms: vec![room("#synthetic:example.invalid")],
            next_batch: Some("page-2".to_owned()),
        },
    );

    assert_eq!(
        state.directory.query,
        DirectoryQueryState::Results {
            request_id: 7,
            query,
            rooms: vec![room("#synthetic:example.invalid")],
            next_batch: Some("page-2".to_owned()),
        }
    );
}

#[test]
fn directory_query_failure_is_request_correlated() {
    let mut state = ready_state();
    let query = query();

    reduce(
        &mut state,
        AppAction::DirectoryQueryRequested {
            request_id: 9,
            query: query.clone(),
        },
    );
    assert_eq!(
        reduce(
            &mut state,
            AppAction::DirectoryQueryFailed {
                request_id: 10,
                query: query.clone(),
                kind: OperationFailureKind::Network,
            },
        ),
        Vec::new()
    );

    reduce(
        &mut state,
        AppAction::DirectoryQueryFailed {
            request_id: 9,
            query: query.clone(),
            kind: OperationFailureKind::Network,
        },
    );

    assert_eq!(
        state.directory.query,
        DirectoryQueryState::Failed {
            request_id: 9,
            query,
            kind: OperationFailureKind::Network,
        }
    );
}

#[test]
fn directory_join_by_alias_is_rust_owned_and_request_correlated() {
    let mut state = ready_state();
    let alias = "#synthetic:example.invalid".to_owned();

    let effects = reduce(
        &mut state,
        AppAction::DirectoryJoinRequested {
            request_id: 11,
            room_id_or_alias: alias.clone(),
            via_servers: vec!["example.invalid".to_owned()],
        },
    );

    assert_eq!(
        state.directory.join,
        DirectoryJoinState::Joining {
            request_id: 11,
            room_id_or_alias: alias.clone(),
            via_servers: vec!["example.invalid".to_owned()],
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::DirectoryChanged)]
    );

    assert_eq!(
        reduce(
            &mut state,
            AppAction::DirectoryJoinSucceeded {
                request_id: 12,
                room_id: "!stale:example.invalid".to_owned(),
            },
        ),
        Vec::new()
    );
    assert!(matches!(
        state.directory.join,
        DirectoryJoinState::Joining { request_id: 11, .. }
    ));

    let effects = reduce(
        &mut state,
        AppAction::DirectoryJoinSucceeded {
            request_id: 11,
            room_id: "!joined:example.invalid".to_owned(),
        },
    );

    assert_eq!(state.directory.join, DirectoryJoinState::Idle);
    assert_eq!(
        state.navigation.active_room_id.as_deref(),
        Some("!joined:example.invalid")
    );
    assert_eq!(
        state.timeline.room_id.as_deref(),
        Some("!joined:example.invalid")
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::DirectoryChanged),
            AppEffect::SubscribeTimeline {
                room_id: "!joined:example.invalid".to_owned(),
            },
            AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: "!joined:example.invalid".to_owned(),
            }),
        ]
    );
}

#[test]
fn directory_join_selects_joined_room_in_home_scope() {
    let mut state = ready_state();
    let alias = "#public:example.invalid".to_owned();
    state.navigation.active_space_id = Some("!space:example.invalid".to_owned());
    state.navigation.active_room_id = Some("!old:example.invalid".to_owned());

    reduce(
        &mut state,
        AppAction::DirectoryJoinRequested {
            request_id: 17,
            room_id_or_alias: alias.clone(),
            via_servers: vec!["example.invalid".to_owned()],
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::DirectoryJoinSucceeded {
            request_id: 17,
            room_id: "!joined:example.invalid".to_owned(),
        },
    );

    assert_eq!(state.navigation.active_space_id, None);
    assert_eq!(
        state.navigation.active_room_id.as_deref(),
        Some("!joined:example.invalid")
    );
    assert_eq!(
        state.timeline.room_id.as_deref(),
        Some("!joined:example.invalid")
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::DirectoryChanged),
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
            AppEffect::SubscribeTimeline {
                room_id: "!joined:example.invalid".to_owned(),
            },
            AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: "!joined:example.invalid".to_owned(),
            }),
        ]
    );
}

#[test]
fn directory_join_failure_preserves_alias_without_raw_sdk_error() {
    let mut state = ready_state();
    let alias = "#synthetic:example.invalid".to_owned();

    reduce(
        &mut state,
        AppAction::DirectoryJoinRequested {
            request_id: 13,
            room_id_or_alias: alias.clone(),
            via_servers: Vec::new(),
        },
    );
    assert_eq!(
        reduce(
            &mut state,
            AppAction::DirectoryJoinFailed {
                request_id: 13,
                room_id_or_alias: "#other:example.invalid".to_owned(),
                via_servers: Vec::new(),
                kind: OperationFailureKind::Forbidden,
            },
        ),
        Vec::new()
    );
    assert!(matches!(
        state.directory.join,
        DirectoryJoinState::Joining { request_id: 13, .. }
    ));

    reduce(
        &mut state,
        AppAction::DirectoryJoinFailed {
            request_id: 13,
            room_id_or_alias: alias.clone(),
            via_servers: Vec::new(),
            kind: OperationFailureKind::Forbidden,
        },
    );

    assert_eq!(
        state.directory.join,
        DirectoryJoinState::Failed {
            request_id: 13,
            room_id_or_alias: alias,
            via_servers: Vec::new(),
            kind: OperationFailureKind::Forbidden,
        }
    );
}

#[test]
fn directory_actions_require_ready_session_and_logout_clears_state() {
    let mut signed_out = AppState::default();

    assert_eq!(
        reduce(
            &mut signed_out,
            AppAction::DirectoryQueryRequested {
                request_id: 14,
                query: query(),
            },
        ),
        Vec::new()
    );
    assert_eq!(
        reduce(
            &mut signed_out,
            AppAction::DirectoryJoinRequested {
                request_id: 15,
                room_id_or_alias: "#synthetic:example.invalid".to_owned(),
                via_servers: Vec::new(),
            },
        ),
        Vec::new()
    );
    assert_eq!(signed_out.directory, DirectoryState::default());

    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::DirectoryJoinRequested {
            request_id: 16,
            room_id_or_alias: "#synthetic:example.invalid".to_owned(),
            via_servers: Vec::new(),
        },
    );
    reduce(&mut state, AppAction::LogoutFinished);

    assert_eq!(state.directory, DirectoryState::default());
}

fn preview() -> DirectoryRoomPreview {
    DirectoryRoomPreview {
        room_id: "!previewed:example.invalid".to_owned(),
        canonical_alias: Some("#previewed:example.invalid".to_owned()),
        room_type: Some("m.space".to_owned()),
        name: "Synthetic Space".to_owned(),
        topic: Some("Synthetic space topic".to_owned()),
        joined_members: 12,
        joinability: DirectoryPreviewJoinability::Open,
        membership: DirectoryPreviewMembership::None,
    }
}

#[test]
fn directory_preview_is_request_correlated_and_keeps_the_join_target() {
    let mut state = ready_state();
    let target = "!previewed:example.invalid".to_owned();
    let via = vec!["example.invalid".to_owned()];

    let effects = reduce(
        &mut state,
        AppAction::DirectoryPreviewRequested {
            request_id: 31,
            room_id_or_alias: target.clone(),
            via_servers: via.clone(),
        },
    );

    assert_eq!(
        state.directory.preview,
        DirectoryPreviewState::Loading {
            request_id: 31,
            room_id_or_alias: target.clone(),
            via_servers: via.clone(),
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::DirectoryChanged)]
    );

    assert_eq!(
        reduce(
            &mut state,
            AppAction::DirectoryPreviewLoaded {
                request_id: 32,
                room: preview(),
            },
        ),
        Vec::new()
    );
    assert!(matches!(
        state.directory.preview,
        DirectoryPreviewState::Loading { request_id: 31, .. }
    ));

    let effects = reduce(
        &mut state,
        AppAction::DirectoryPreviewLoaded {
            request_id: 31,
            room: preview(),
        },
    );

    // Joining must reuse the exact target/via that resolved the preview, so the
    // ready state carries them instead of making the GUI re-derive them.
    assert_eq!(
        state.directory.preview,
        DirectoryPreviewState::Ready {
            request_id: 31,
            room_id_or_alias: target,
            via_servers: via,
            room: preview(),
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::DirectoryChanged)]
    );
}

#[test]
fn directory_preview_failure_is_rust_owned_and_dismiss_closes_it() {
    let mut state = ready_state();
    let target = "#missing:example.invalid".to_owned();

    reduce(
        &mut state,
        AppAction::DirectoryPreviewRequested {
            request_id: 33,
            room_id_or_alias: target.clone(),
            via_servers: Vec::new(),
        },
    );
    let effects = reduce(
        &mut state,
        AppAction::DirectoryPreviewFailed {
            request_id: 33,
            room_id_or_alias: target.clone(),
            via_servers: Vec::new(),
            kind: OperationFailureKind::NotFound,
        },
    );

    assert_eq!(
        state.directory.preview,
        DirectoryPreviewState::Failed {
            request_id: 33,
            room_id_or_alias: target,
            via_servers: Vec::new(),
            kind: OperationFailureKind::NotFound,
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::DirectoryChanged)]
    );

    let effects = reduce(&mut state, AppAction::DirectoryPreviewDismissed);
    assert_eq!(state.directory.preview, DirectoryPreviewState::Closed);
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::DirectoryChanged)]
    );

    // Dismissing nothing is not a state change worth notifying the GUI about.
    assert_eq!(
        reduce(&mut state, AppAction::DirectoryPreviewDismissed),
        Vec::new()
    );
}

#[test]
fn joining_closes_the_preview_so_the_dialog_does_not_outlive_the_decision() {
    let mut state = ready_state();
    let target = "!previewed:example.invalid".to_owned();

    reduce(
        &mut state,
        AppAction::DirectoryPreviewRequested {
            request_id: 34,
            room_id_or_alias: target.clone(),
            via_servers: Vec::new(),
        },
    );
    reduce(
        &mut state,
        AppAction::DirectoryPreviewLoaded {
            request_id: 34,
            room: preview(),
        },
    );
    reduce(
        &mut state,
        AppAction::DirectoryJoinRequested {
            request_id: 35,
            room_id_or_alias: target,
            via_servers: Vec::new(),
        },
    );

    assert_eq!(state.directory.preview, DirectoryPreviewState::Closed);
}

#[test]
fn directory_preview_needs_a_ready_session_and_is_cleared_on_logout() {
    let mut signed_out = AppState::default();
    assert_eq!(
        reduce(
            &mut signed_out,
            AppAction::DirectoryPreviewRequested {
                request_id: 36,
                room_id_or_alias: "#synthetic:example.invalid".to_owned(),
                via_servers: Vec::new(),
            },
        ),
        Vec::new()
    );
    assert_eq!(signed_out.directory, DirectoryState::default());

    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::DirectoryPreviewRequested {
            request_id: 37,
            room_id_or_alias: "#synthetic:example.invalid".to_owned(),
            via_servers: Vec::new(),
        },
    );
    reduce(&mut state, AppAction::LogoutFinished);

    assert_eq!(state.directory, DirectoryState::default());
}

#[test]
fn directory_preview_debug_output_redacts_private_room_values() {
    let private_preview = DirectoryRoomPreview {
        room_id: "!private-preview:example.invalid".to_owned(),
        canonical_alias: Some("#private-preview:example.invalid".to_owned()),
        room_type: Some("m.space".to_owned()),
        name: "Private Preview Room".to_owned(),
        topic: Some("Private preview topic".to_owned()),
        joined_members: 5,
        joinability: DirectoryPreviewJoinability::InviteOnly,
        membership: DirectoryPreviewMembership::Invited,
    };

    let debug_values = [
        format!("{private_preview:?}"),
        format!(
            "{:?}",
            DirectoryPreviewState::Ready {
                request_id: 38,
                room_id_or_alias: "#private-preview:example.invalid".to_owned(),
                via_servers: vec!["private.example.invalid".to_owned()],
                room: private_preview.clone(),
            }
        ),
        format!(
            "{:?}",
            AppAction::DirectoryPreviewRequested {
                request_id: 39,
                room_id_or_alias: "#private-preview:example.invalid".to_owned(),
                via_servers: vec!["private.example.invalid".to_owned()],
            }
        ),
        format!(
            "{:?}",
            AppAction::DirectoryPreviewLoaded {
                request_id: 40,
                room: private_preview,
            }
        ),
    ];

    for debug in debug_values {
        for secret in [
            "private-preview",
            "Private Preview Room",
            "Private preview topic",
            "private.example.invalid",
        ] {
            assert!(
                !debug.contains(secret),
                "directory preview Debug output must not leak private room values"
            );
        }
    }
}
