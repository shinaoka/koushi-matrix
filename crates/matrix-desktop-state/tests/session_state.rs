use matrix_desktop_state::{
    AppAction, AppEffect, AppState, AuthDiscoveryState, AuthSecret, LoginFlow, LoginFlowKind,
    LoginRequest, NavigationState, RoomSummary, SearchScope, SearchState, SessionInfo,
    SessionState, SpaceSummary, SyncState, ThreadPaneState, TimelinePaneState, UiEvent, reduce,
};

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.org".to_owned(),
        user_id: "@user-a:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
    }
}

#[test]
fn app_started_requests_session_restore() {
    let mut state = AppState::default();

    let effects = reduce(&mut state, AppAction::AppStarted);

    assert_eq!(state.session, SessionState::Restoring);
    assert_eq!(effects, vec![AppEffect::RestoreSession]);
}

#[test]
fn restore_success_marks_ready_and_starts_sync() {
    let mut state = AppState {
        session: SessionState::Restoring,
        ..AppState::default()
    };
    let info = session_info();

    let effects = reduce(&mut state, AppAction::RestoreSessionSucceeded(info.clone()));

    assert_eq!(state.session, SessionState::Ready(info.clone()));
    assert_eq!(state.sync, SyncState::Starting);
    assert_eq!(
        effects,
        vec![
            AppEffect::PersistSession(info),
            AppEffect::StartSync,
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
        ]
    );
}

#[test]
fn restore_not_found_enters_signed_out_without_error() {
    let mut state = AppState {
        session: SessionState::Restoring,
        ..AppState::default()
    };

    let effects = reduce(&mut state, AppAction::RestoreSessionNotFound);

    assert_eq!(state.session, SessionState::SignedOut);
    assert!(state.errors.is_empty());
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
    );
}

#[test]
fn login_discovery_requests_homeserver_flows() {
    let mut state = AppState::default();

    let effects = reduce(
        &mut state,
        AppAction::LoginDiscoveryRequested {
            homeserver: "https://matrix.example.org".to_owned(),
        },
    );

    assert_eq!(
        state.auth,
        AuthDiscoveryState::Discovering {
            homeserver: "https://matrix.example.org".to_owned()
        }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::DiscoverLogin {
                homeserver: "https://matrix.example.org".to_owned(),
            },
            AppEffect::EmitUiEvent(UiEvent::AuthChanged),
        ]
    );
}

#[test]
fn login_discovery_success_records_supported_flows() {
    let mut state = AppState {
        auth: AuthDiscoveryState::Discovering {
            homeserver: "https://matrix.example.org".to_owned(),
        },
        ..AppState::default()
    };
    let flows = vec![
        LoginFlow {
            kind: LoginFlowKind::Password,
            delegated_oidc_compatibility: false,
        },
        LoginFlow {
            kind: LoginFlowKind::Sso,
            delegated_oidc_compatibility: true,
        },
    ];

    let effects = reduce(
        &mut state,
        AppAction::LoginDiscoverySucceeded {
            homeserver: "https://matrix.example.org".to_owned(),
            flows: flows.clone(),
        },
    );

    assert_eq!(
        state.auth,
        AuthDiscoveryState::Ready {
            homeserver: "https://matrix.example.org".to_owned(),
            flows
        }
    );
    assert_eq!(effects, vec![AppEffect::EmitUiEvent(UiEvent::AuthChanged)]);
}

#[test]
fn login_submitted_enters_authenticating_and_emits_session_event() {
    let mut state = AppState::default();

    let effects = reduce(
        &mut state,
        AppAction::LoginSubmitted(LoginRequest {
            homeserver: "https://matrix.example.org".to_owned(),
            username: "user-a".to_owned(),
            password: AuthSecret::new("synthetic-password"),
            device_display_name: Some("Matrix Desktop Test".to_owned()),
        }),
    );

    assert_eq!(
        state.session,
        SessionState::Authenticating {
            homeserver: "https://matrix.example.org".to_owned()
        }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::Login(LoginRequest {
                homeserver: "https://matrix.example.org".to_owned(),
                username: "user-a".to_owned(),
                password: AuthSecret::new("synthetic-password"),
                device_display_name: Some("Matrix Desktop Test".to_owned()),
            }),
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
        ]
    );
}

#[test]
fn login_request_debug_redacts_password() {
    let action = AppAction::LoginSubmitted(LoginRequest {
        homeserver: "https://matrix.example.org".to_owned(),
        username: "user-a".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    });

    let debug = format!("{action:?}");

    assert!(debug.contains("AuthSecret(..)"));
    assert!(!debug.contains("synthetic-password"));
}

#[test]
fn login_failure_returns_to_signed_out_and_records_error() {
    let mut state = AppState {
        session: SessionState::Authenticating {
            homeserver: "https://matrix.example.org".to_owned(),
        },
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::LoginFailed {
            message: "invalid password".to_owned(),
        },
    );

    assert_eq!(state.session, SessionState::SignedOut);
    assert_eq!(state.errors[0].code, "login_failed");
    assert!(state.errors[0].recoverable);
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
        ]
    );
}

#[test]
fn logout_stops_sync_and_clears_session() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        sync: SyncState::Running,
        ..AppState::default()
    };

    let effects = reduce(&mut state, AppAction::LogoutRequested);

    assert_eq!(state.session, SessionState::LoggingOut);
    assert_eq!(state.sync, SyncState::Stopped);
    assert_eq!(
        effects,
        vec![
            AppEffect::StopSync,
            AppEffect::ClearSession,
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
        ]
    );
}

#[test]
fn logout_clears_session_views_and_notifies_ui() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        sync: SyncState::Running,
        navigation: NavigationState {
            active_space_id: Some("space-a".to_owned()),
            active_room_id: Some("room-a".to_owned()),
        },
        spaces: vec![SpaceSummary {
            space_id: "space-a".to_owned(),
            display_name: "Space A".to_owned(),
            child_room_ids: vec!["room-a".to_owned()],
        }],
        rooms: vec![RoomSummary {
            room_id: "room-a".to_owned(),
            display_name: "Room A".to_owned(),
            is_dm: false,
            unread_count: 3,
            parent_space_ids: vec!["space-a".to_owned()],
        }],
        timeline: TimelinePaneState {
            room_id: Some("room-a".to_owned()),
            is_subscribed: true,
            is_paginating_backwards: true,
            composer: Default::default(),
        },
        thread: ThreadPaneState::Open {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            is_subscribed: true,
            composer: Default::default(),
        },
        search: SearchState::Editing {
            query: "アンケート".to_owned(),
            scope: SearchScope::AllRooms,
        },
        ..AppState::default()
    };

    let effects = reduce(&mut state, AppAction::LogoutRequested);

    assert_eq!(state.navigation, NavigationState::default());
    assert!(state.spaces.is_empty());
    assert!(state.rooms.is_empty());
    assert_eq!(state.timeline, TimelinePaneState::default());
    assert_eq!(state.thread, ThreadPaneState::Closed);
    assert_eq!(state.search, SearchState::Closed);
    assert_eq!(
        effects,
        vec![
            AppEffect::StopSync,
            AppEffect::ClearSession,
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
            AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: "room-a".to_owned(),
            }),
            AppEffect::EmitUiEvent(UiEvent::ThreadChanged),
            AppEffect::EmitUiEvent(UiEvent::SearchChanged),
        ]
    );
}

#[test]
fn session_locked_stops_sync_and_clears_session_views() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        sync: SyncState::Running,
        spaces: vec![SpaceSummary {
            space_id: "space-a".to_owned(),
            display_name: "Space A".to_owned(),
            child_room_ids: vec![],
        }],
        ..AppState::default()
    };

    let effects = reduce(&mut state, AppAction::SessionLocked);

    assert_eq!(state.session, SessionState::Locked(session_info()));
    assert_eq!(state.sync, SyncState::Stopped);
    assert!(state.spaces.is_empty());
    assert_eq!(
        effects,
        vec![
            AppEffect::StopSync,
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
        ]
    );
}

#[test]
fn sync_failure_enters_recovering_state() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        sync: SyncState::Running,
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::SyncFailed {
            reason: "limited network".to_owned(),
        },
    );

    assert_eq!(
        state.sync,
        SyncState::Recovering {
            reason: "limited network".to_owned(),
        }
    );
    assert_eq!(effects, vec![AppEffect::StartSync]);
}

#[test]
fn late_sync_signals_after_logout_are_ignored() {
    let mut state = AppState {
        session: SessionState::LoggingOut,
        sync: SyncState::Stopped,
        ..AppState::default()
    };

    assert_eq!(reduce(&mut state, AppAction::SyncStarted), Vec::new());
    assert_eq!(
        reduce(
            &mut state,
            AppAction::SyncFailed {
                reason: "late failure".to_owned(),
            },
        ),
        Vec::new()
    );
    assert_eq!(reduce(&mut state, AppAction::SyncRecovered), Vec::new());
    assert_eq!(state.sync, SyncState::Stopped);
}

#[test]
fn sync_stopped_is_a_completion_signal() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        sync: SyncState::Running,
        ..AppState::default()
    };

    let effects = reduce(&mut state, AppAction::SyncStopped);

    assert_eq!(state.sync, SyncState::Stopped);
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
    );
}
