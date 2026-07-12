use koushi_backend::{
    DEFAULT_HOMESERVER, DesktopRoomListRoom, DesktopRoomListSpace, DesktopRoomListUpdate,
    E2eeRecoveryMode, FakeDesktopBackend, FakeDesktopBackendConfig, LoginMode, SyncMode,
    compose_room_list_update,
};
use koushi_search::SearchCandidate;
use koushi_state::{
    AppAction, AppEffect, AuthDiscoveryState, AuthFailureKind, AuthSecret, E2eeRecoveryState,
    LoginAttemptId, LoginFlowKind, LoginRequest, RecoveryRequest, SearchMatchField, SearchScope,
    SearchState, SessionState, SyncState, ThreadPaneState, compose_sidebar,
};

fn login_submitted(request: LoginRequest) -> AppAction {
    AppAction::LoginSubmitted {
        attempt_id: LoginAttemptId::new(0, 1),
        request,
    }
}
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

#[test]
fn fake_backend_boots_into_ready_session_with_rooms_and_thread() {
    let backend = FakeDesktopBackend::booted();

    let snapshot = backend.snapshot();

    let SessionState::Ready(session_info) = &snapshot.state.session else {
        panic!("expected ready session");
    };
    assert_eq!(session_info.homeserver, DEFAULT_HOMESERVER);
    assert_eq!(snapshot.state.sync, SyncState::Running);
    assert_eq!(
        snapshot.state.navigation.active_space_id.as_deref(),
        Some("!space-alpha:example.invalid")
    );
    assert_eq!(
        snapshot.state.navigation.active_room_id.as_deref(),
        Some("!room-alpha:example.invalid")
    );
    assert!(snapshot.state.timeline.is_subscribed);
    assert!(matches!(
        snapshot.state.thread,
        ThreadPaneState::Open { .. }
    ));
    assert!(
        snapshot
            .timeline
            .iter()
            .any(|message| message.event_id == "$alpha-update")
    );
    assert_eq!(
        snapshot
            .thread
            .as_ref()
            .map(|thread| thread.root_event_id.as_str()),
        Some("$alpha-update")
    );
    assert!(
        snapshot
            .sidebar
            .global_dms
            .iter()
            .any(|room| room.display_name == "Member 1")
    );
}

#[test]
fn room_list_composition_keeps_dms_global_and_preserves_multi_parent_rooms() {
    let action = compose_room_list_update(DesktopRoomListUpdate {
        spaces: vec![
            DesktopRoomListSpace {
                space_id: "!space-a:example.invalid".to_owned(),
                display_name: "Alpha".to_owned(),
            },
            DesktopRoomListSpace {
                space_id: "!space-b:example.invalid".to_owned(),
                display_name: "Beta".to_owned(),
            },
        ],
        rooms: vec![
            DesktopRoomListRoom {
                room_id: "!shared:example.invalid".to_owned(),
                display_name: "Shared".to_owned(),
                is_dm: false,
                unread_count: 4,
                notification_count: 4,
                highlight_count: 0,
                parent_space_ids: vec![
                    "!space-a:example.invalid".to_owned(),
                    "!space-b:example.invalid".to_owned(),
                ],
                joined_members: 0,
            },
            DesktopRoomListRoom {
                room_id: "!loose:example.invalid".to_owned(),
                display_name: "Loose".to_owned(),
                is_dm: false,
                unread_count: 2,
                notification_count: 2,
                highlight_count: 0,
                parent_space_ids: Vec::new(),
                joined_members: 0,
            },
            DesktopRoomListRoom {
                room_id: "!dm-a:example.invalid".to_owned(),
                display_name: "Direct".to_owned(),
                is_dm: true,
                unread_count: 3,
                notification_count: 3,
                highlight_count: 0,
                parent_space_ids: vec!["!space-a:example.invalid".to_owned()],
                joined_members: 0,
            },
        ],
    });

    let AppAction::RoomListUpdated { spaces, rooms } = action else {
        panic!("expected room list action");
    };

    assert_eq!(spaces[0].child_room_ids, vec!["!shared:example.invalid"]);
    assert_eq!(spaces[1].child_room_ids, vec!["!shared:example.invalid"]);
    assert_eq!(rooms[0].parent_space_ids.len(), 2);
    assert!(rooms[2].parent_space_ids.is_empty());

    let alpha_sidebar = compose_sidebar(Some("!space-a:example.invalid"), &spaces, &rooms);
    assert_eq!(
        alpha_sidebar
            .space_rooms
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["!shared:example.invalid"]
    );
    assert_eq!(
        alpha_sidebar
            .global_dms
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["!dm-a:example.invalid"]
    );
    assert_eq!(alpha_sidebar.space_unread_count, 4);
    assert_eq!(alpha_sidebar.dm_unread_count, 3);

    let account_home = compose_sidebar(None, &spaces, &rooms);
    assert_eq!(account_home.account_home.unread_count, 9);
    assert_eq!(
        account_home
            .space_rooms
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["!shared:example.invalid", "!loose:example.invalid"]
    );
}

#[test]
fn fake_backend_keeps_homeserver_configurable() {
    let backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        homeserver: "https://matrix.example.org".into(),
        user_id: "@user-a:example.invalid".into(),
        device_id: "DEVICE_A".into(),
        ..FakeDesktopBackendConfig::default()
    });

    let SessionState::Ready(session_info) = &backend.snapshot().state.session else {
        panic!("expected ready session");
    };
    assert_eq!(session_info.homeserver, "https://matrix.example.org");
    assert_eq!(
        backend.session_key_id().homeserver,
        "https://matrix.example.org"
    );
}

#[test]
fn fake_backend_can_boot_without_saved_session() {
    let backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        restore_session: false,
        ..FakeDesktopBackendConfig::default()
    });

    let snapshot = backend.snapshot();

    assert_eq!(snapshot.state.session, SessionState::SignedOut);
    assert_eq!(snapshot.state.sync, SyncState::Stopped);
    assert!(snapshot.state.rooms.is_empty());
    assert!(snapshot.state.spaces.is_empty());
    assert!(snapshot.state.errors.is_empty());
}

#[test]
fn fake_backend_discovers_password_and_sso_login_methods() {
    let mut backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        restore_session: false,
        ..FakeDesktopBackendConfig::default()
    });

    backend.dispatch(AppAction::LoginDiscoveryRequested {
        homeserver: "https://matrix.example.org".to_owned(),
    });

    let AuthDiscoveryState::Ready {
        homeserver, flows, ..
    } = &backend.snapshot().state.auth
    else {
        panic!("expected discovered login flows");
    };

    assert_eq!(homeserver, "https://matrix.example.org");
    assert!(
        flows
            .iter()
            .any(|flow| flow.kind == LoginFlowKind::Password)
    );
    assert!(
        flows
            .iter()
            .any(|flow| { flow.kind == LoginFlowKind::Sso && flow.delegated_oidc_compatibility })
    );
}

#[test]
fn http_backend_discovers_login_methods_from_homeserver() {
    let homeserver = spawn_login_discovery_server(
        200,
        r#"{"flows":[{"type":"m.login.password"},{"type":"m.login.token"}]}"#,
    );
    let mut backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        restore_session: false,
        login_discovery: koushi_backend::LoginDiscoveryMode::Http,
        ..FakeDesktopBackendConfig::default()
    });

    backend.dispatch(AppAction::LoginDiscoveryRequested {
        homeserver: homeserver.clone(),
    });

    let AuthDiscoveryState::Ready {
        homeserver: discovered_homeserver,
        flows,
        ..
    } = &backend.snapshot().state.auth
    else {
        panic!("expected discovered login flows");
    };

    assert_eq!(discovered_homeserver, &homeserver);
    assert_eq!(flows[0].kind, LoginFlowKind::Password);
    assert_eq!(flows[1].kind, LoginFlowKind::Token);
}

#[test]
fn http_backend_records_login_discovery_failure() {
    let homeserver = spawn_login_discovery_server(
        404,
        r#"{"errcode":"M_UNRECOGNIZED","error":"OAuth 2.0 authentication is in use on this homeserver."}"#,
    );
    let mut backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        restore_session: false,
        login_discovery: koushi_backend::LoginDiscoveryMode::Http,
        ..FakeDesktopBackendConfig::default()
    });

    backend.dispatch(AppAction::LoginDiscoveryRequested {
        homeserver: homeserver.clone(),
    });

    let AuthDiscoveryState::Failed {
        homeserver: failed_homeserver,
        kind,
    } = &backend.snapshot().state.auth
    else {
        panic!("expected login discovery failure");
    };

    assert_eq!(failed_homeserver, &homeserver);
    assert_eq!(*kind, AuthFailureKind::Sdk);
}

#[test]
fn fake_backend_login_boundary_fails_explicitly_before_real_sdk_wiring() {
    let mut backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        restore_session: false,
        ..FakeDesktopBackendConfig::default()
    });

    backend.dispatch(login_submitted(LoginRequest {
        homeserver: "https://matrix.example.org".to_owned(),
        username: "demo-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    }));

    let snapshot = backend.snapshot();

    assert_eq!(snapshot.state.session, SessionState::SignedOut);
    assert!(snapshot.state.rooms.is_empty());
    assert_eq!(snapshot.state.errors.len(), 1);
    assert_eq!(snapshot.state.errors[0].code, "login_failed");
    assert!(
        snapshot.state.errors[0]
            .message
            .contains("real Matrix login is not wired")
    );
    assert!(!format!("{snapshot:?}").contains("synthetic-password"));
}

#[test]
fn sdk_login_backend_enters_ready_session_without_exposing_secret() {
    let homeserver = spawn_password_login_server(200);
    let mut backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        restore_session: false,
        login: koushi_backend::LoginMode::MatrixSdk,
        ..FakeDesktopBackendConfig::default()
    });

    backend.dispatch(login_submitted(LoginRequest {
        homeserver: homeserver.clone(),
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    }));

    let snapshot = backend.snapshot();
    let SessionState::Ready(session_info) = &snapshot.state.session else {
        panic!("expected ready session");
    };

    assert_eq!(session_info.homeserver, homeserver);
    assert_eq!(session_info.user_id, "@fixture-user:example.invalid");
    assert_eq!(session_info.device_id, "FIXTUREDEVICE");
    assert!(!format!("{snapshot:?}").contains("synthetic-password"));
}

#[test]
fn deferred_sdk_login_returns_login_effect_without_holding_backend_execution() {
    let homeserver = spawn_password_login_server(200);
    let mut backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        restore_session: false,
        login: koushi_backend::LoginMode::Deferred,
        ..FakeDesktopBackendConfig::default()
    });

    let effects = backend.dispatch(login_submitted(LoginRequest {
        homeserver: homeserver.clone(),
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    }));

    assert!(matches!(
        backend.snapshot().state.session,
        SessionState::Authenticating { .. }
    ));
    assert!(effects.iter().any(|effect| {
        matches!(
            effect,
            AppEffect::Login { request, .. } if request.homeserver == homeserver
        )
    }));
    assert!(!format!("{effects:?}").contains("synthetic-password"));
}

#[test]
fn deferred_sdk_login_completion_stores_session_and_enters_ready_state() {
    let homeserver = spawn_password_login_server(200);
    let mut backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        restore_session: false,
        login: koushi_backend::LoginMode::Deferred,
        ..FakeDesktopBackendConfig::default()
    });
    backend.dispatch(login_submitted(LoginRequest {
        homeserver: homeserver.clone(),
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    }));
    let request = LoginRequest {
        homeserver: homeserver.clone(),
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };
    let session =
        koushi_sdk::login_with_password_blocking(&request).expect("password login should succeed");

    backend.complete_matrix_login(session);

    let snapshot = backend.snapshot();
    let SessionState::Ready(session_info) = &snapshot.state.session else {
        panic!("expected ready session");
    };
    assert_eq!(session_info.homeserver, homeserver);
    assert_eq!(session_info.user_id, "@fixture-user:example.invalid");
    assert_eq!(session_info.device_id, "FIXTUREDEVICE");
    assert!(!format!("{snapshot:?}").contains("synthetic-password"));
}

#[test]
fn deferred_sdk_login_completion_can_enter_e2ee_recovery_step() {
    let homeserver = spawn_password_login_server(200);
    let mut backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        restore_session: false,
        login: koushi_backend::LoginMode::Deferred,
        e2ee_recovery: E2eeRecoveryMode::RequiredDeferred,
        ..FakeDesktopBackendConfig::default()
    });
    backend.dispatch(login_submitted(LoginRequest {
        homeserver: homeserver.clone(),
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    }));
    let request = LoginRequest {
        homeserver: homeserver.clone(),
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };
    let session =
        koushi_sdk::login_with_password_blocking(&request).expect("password login should succeed");

    let effects = backend.complete_matrix_login(session);

    let snapshot = backend.snapshot();
    let SessionState::AwaitingVerification { info, gate } = &snapshot.state.session else {
        panic!("expected e2ee recovery step");
    };
    assert_eq!(info.homeserver, homeserver);
    assert_eq!(gate.methods.len(), 2);
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, AppEffect::StartSync))
    );
    assert_eq!(snapshot.state.sync, SyncState::Stopped);
    assert!(snapshot.state.rooms.is_empty());
}

#[test]
fn sdk_state_recovery_mode_does_not_prompt_before_sdk_reports_incomplete() {
    let homeserver = spawn_password_login_server(200);
    let mut backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        restore_session: false,
        login: koushi_backend::LoginMode::Deferred,
        e2ee_recovery: E2eeRecoveryMode::SdkState,
        ..FakeDesktopBackendConfig::default()
    });
    backend.dispatch(login_submitted(LoginRequest {
        homeserver: homeserver.clone(),
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    }));
    let request = LoginRequest {
        homeserver: homeserver.clone(),
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };
    let session =
        koushi_sdk::login_with_password_blocking(&request).expect("password login should succeed");

    backend.complete_matrix_login(session);

    let snapshot = backend.snapshot();
    assert!(matches!(snapshot.state.session, SessionState::Ready(_)));
    assert_eq!(snapshot.state.sync, SyncState::Running);
}

#[test]
fn deferred_sync_mode_emits_start_sync_without_fixture_room_updates() {
    let homeserver = spawn_password_login_server(200);
    let mut backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        restore_session: false,
        login: LoginMode::Deferred,
        sync: SyncMode::Deferred,
        ..FakeDesktopBackendConfig::default()
    });
    backend.dispatch(login_submitted(LoginRequest {
        homeserver: homeserver.clone(),
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    }));
    let request = LoginRequest {
        homeserver,
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };
    let session =
        koushi_sdk::login_with_password_blocking(&request).expect("password login should succeed");

    let effects = backend.complete_matrix_login(session);
    let snapshot = backend.snapshot();

    assert!(matches!(snapshot.state.session, SessionState::Ready(_)));
    assert_eq!(snapshot.state.sync, SyncState::Starting);
    assert!(snapshot.state.rooms.is_empty());
    assert!(snapshot.state.spaces.is_empty());
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::StartSync))
    );
    assert!(!effects.iter().any(|effect| {
        matches!(effect, AppEffect::SubscribeTimeline { .. })
            || matches!(effect, AppEffect::OpenThreadTimeline { .. })
    }));
}

#[test]
fn deferred_sync_mode_does_not_seed_fixture_rooms_after_sync_failure() {
    let homeserver = spawn_password_login_server(200);
    let mut backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        restore_session: false,
        login: LoginMode::Deferred,
        sync: SyncMode::Deferred,
        ..FakeDesktopBackendConfig::default()
    });
    backend.dispatch(login_submitted(LoginRequest {
        homeserver: homeserver.clone(),
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    }));
    let request = LoginRequest {
        homeserver,
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };
    let session =
        koushi_sdk::login_with_password_blocking(&request).expect("password login should succeed");
    backend.complete_matrix_login(session);

    let effects = backend.dispatch(AppAction::SyncFailed {
        reason: "synthetic network failure".to_owned(),
    });
    let snapshot = backend.snapshot();

    assert_eq!(
        snapshot.state.sync,
        SyncState::Failed {
            reason: "synthetic network failure".to_owned()
        }
    );
    assert!(snapshot.state.rooms.is_empty());
    assert!(snapshot.state.spaces.is_empty());
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::StartSync))
    );
}

#[test]
fn deferred_sync_mode_defers_timeline_subscription_to_sdk_boundary() {
    let mut backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        sync: SyncMode::Deferred,
        ..FakeDesktopBackendConfig::default()
    });
    let room_id = "!sdk-room:example.invalid".to_owned();

    let effects = backend.dispatch(compose_room_list_update(DesktopRoomListUpdate {
        spaces: vec![DesktopRoomListSpace {
            space_id: "!sdk-space:example.invalid".to_owned(),
            display_name: "SDK Space".to_owned(),
        }],
        rooms: vec![DesktopRoomListRoom {
            room_id: room_id.clone(),
            display_name: "SDK Room".to_owned(),
            is_dm: false,
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            parent_space_ids: vec!["!sdk-space:example.invalid".to_owned()],
            joined_members: 0,
        }],
    }));

    assert!(effects.iter().any(|effect| {
        matches!(
            effect,
            AppEffect::SubscribeTimeline { room_id: effect_room_id }
                if effect_room_id == &room_id
        )
    }));
    assert_eq!(
        backend.snapshot().state.timeline.room_id.as_deref(),
        Some(room_id.as_str())
    );
    assert!(!backend.snapshot().state.timeline.is_subscribed);
}

#[test]
fn deferred_sync_mode_defers_timeline_pagination_to_sdk_boundary() {
    let mut backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        sync: SyncMode::Deferred,
        ..FakeDesktopBackendConfig::default()
    });
    let room_id = "!sdk-room:example.invalid".to_owned();

    backend.dispatch(compose_room_list_update(DesktopRoomListUpdate {
        spaces: vec![DesktopRoomListSpace {
            space_id: "!sdk-space:example.invalid".to_owned(),
            display_name: "SDK Space".to_owned(),
        }],
        rooms: vec![DesktopRoomListRoom {
            room_id: room_id.clone(),
            display_name: "SDK Room".to_owned(),
            is_dm: false,
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            parent_space_ids: vec!["!sdk-space:example.invalid".to_owned()],
            joined_members: 0,
        }],
    }));
    backend.dispatch(AppAction::SelectRoom {
        room_id: room_id.clone(),
    });

    let before = backend
        .snapshot()
        .timeline
        .iter()
        .map(|message| message.event_id.clone())
        .collect::<Vec<_>>();

    let effects = backend.dispatch(AppAction::TimelineBackPaginationRequested {
        room_id: room_id.clone(),
    });

    let snapshot = backend.snapshot();
    let after = snapshot
        .timeline
        .iter()
        .map(|message| message.event_id.clone())
        .collect::<Vec<_>>();

    assert_eq!(before, after);
    assert!(snapshot.state.timeline.is_paginating_backwards);
    assert!(effects.iter().any(|effect| {
        matches!(
            effect,
            AppEffect::PaginateTimelineBackwards { room_id }
                if room_id == "!sdk-room:example.invalid"
        )
    }));
}

#[test]
fn deferred_sync_mode_defers_send_text_to_sdk_boundary() {
    let mut backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        sync: SyncMode::Deferred,
        ..FakeDesktopBackendConfig::default()
    });
    let room_id = "!sdk-room:example.invalid".to_owned();

    backend.dispatch(compose_room_list_update(DesktopRoomListUpdate {
        spaces: vec![DesktopRoomListSpace {
            space_id: "!sdk-space:example.invalid".to_owned(),
            display_name: "SDK Space".to_owned(),
        }],
        rooms: vec![DesktopRoomListRoom {
            room_id: room_id.clone(),
            display_name: "SDK Room".to_owned(),
            is_dm: false,
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            parent_space_ids: vec!["!sdk-space:example.invalid".to_owned()],
            joined_members: 0,
        }],
    }));
    backend.dispatch(AppAction::SelectRoom {
        room_id: room_id.clone(),
    });

    let effects = backend.dispatch(AppAction::SendTextSubmitted {
        room_id: room_id.clone(),
        transaction_id: "txn-sdk".to_owned(),
        body: "hello from sdk boundary".to_owned(),
    });
    let snapshot = backend.snapshot();

    assert!(effects.iter().any(|effect| {
        matches!(
            effect,
            AppEffect::SendText { transaction_id, .. } if transaction_id == "txn-sdk"
        )
    }));
    assert_eq!(
        snapshot
            .state
            .timeline
            .composer
            .pending_transaction_id
            .as_deref(),
        Some("txn-sdk")
    );
    assert!(
        snapshot
            .timeline
            .iter()
            .all(|message| message.event_id != "$local-txn-sdk")
    );
}

#[test]
fn logout_clears_backend_matrix_session_handle() {
    let homeserver = spawn_password_login_server(200);
    let mut backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        restore_session: false,
        login: koushi_backend::LoginMode::Deferred,
        ..FakeDesktopBackendConfig::default()
    });
    backend.dispatch(login_submitted(LoginRequest {
        homeserver: homeserver.clone(),
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    }));
    let request = LoginRequest {
        homeserver,
        username: "fixture-user".to_owned(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: Some("Matrix Desktop Test".to_owned()),
    };
    let session =
        koushi_sdk::login_with_password_blocking(&request).expect("password login should succeed");
    backend.complete_matrix_login(session);

    backend.dispatch(AppAction::LogoutRequested);
    backend.dispatch(AppAction::LogoutFinished);

    assert!(backend.matrix_session().is_none());
    assert!(matches!(
        backend.snapshot().state.session,
        SessionState::SignedOut
    ));
}

#[test]
fn observed_incomplete_recovery_does_not_reopen_a_gate_after_verified_ready() {
    let mut backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        restore_session: true,
        e2ee_recovery: E2eeRecoveryMode::SdkState,
        ..FakeDesktopBackendConfig::default()
    });

    backend.observe_e2ee_recovery_state(E2eeRecoveryState::Incomplete);

    let snapshot = backend.snapshot();
    assert!(matches!(snapshot.state.session, SessionState::Ready(_)));
    assert_eq!(snapshot.state.sync, SyncState::Running);
    assert!(!snapshot.state.rooms.is_empty());
}

#[test]
fn observed_unknown_e2ee_recovery_state_does_not_interrupt_running_session() {
    let mut backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        restore_session: true,
        e2ee_recovery: E2eeRecoveryMode::SdkState,
        ..FakeDesktopBackendConfig::default()
    });

    backend.observe_e2ee_recovery_state(E2eeRecoveryState::Unknown);

    let snapshot = backend.snapshot();
    assert!(matches!(snapshot.state.session, SessionState::Ready(_)));
    assert_eq!(snapshot.state.sync, SyncState::Running);
    assert!(!snapshot.state.rooms.is_empty());
}

#[test]
fn e2ee_recovery_submission_can_be_deferred_without_exposing_secret() {
    let mut backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        restore_session: true,
        e2ee_recovery: E2eeRecoveryMode::RequiredDeferred,
        ..FakeDesktopBackendConfig::default()
    });

    let effects = backend.dispatch(AppAction::E2eeRecoverySubmitted {
        flow_id: 77,
        request: RecoveryRequest {
            secret: AuthSecret::new("synthetic-recovery-secret"),
        },
    });

    assert!(matches!(
        backend.snapshot().state.session,
        SessionState::Verifying { .. }
    ));
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::RecoverE2ee(_)))
    );
    assert!(!format!("{effects:?}").contains("synthetic-recovery-secret"));
}

#[test]
fn e2ee_recovery_fixture_success_enters_ready_session() {
    let mut backend = FakeDesktopBackend::booted_with_config(FakeDesktopBackendConfig {
        restore_session: true,
        e2ee_recovery: E2eeRecoveryMode::RequiredFixtureSuccess,
        ..FakeDesktopBackendConfig::default()
    });

    backend.dispatch(AppAction::E2eeRecoverySubmitted {
        flow_id: 77,
        request: RecoveryRequest {
            secret: AuthSecret::new("synthetic-recovery-secret"),
        },
    });

    let snapshot = backend.snapshot();
    assert!(matches!(snapshot.state.session, SessionState::Ready(_)));
    assert_eq!(snapshot.state.sync, SyncState::Running);
    assert!(!format!("{snapshot:?}").contains("synthetic-recovery-secret"));
}

#[test]
fn fake_backend_keeps_dms_global_when_switching_spaces() {
    let mut backend = FakeDesktopBackend::booted();

    backend.dispatch(AppAction::SelectSpace {
        space_id: Some("!space-beta:example.invalid".into()),
    });
    let snapshot = backend.snapshot();

    assert_eq!(
        snapshot
            .sidebar
            .space_rooms
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["!room-search:example.invalid"]
    );
    assert!(snapshot.sidebar.global_dms.iter().any(|room| {
        room.room_id == "!dm-member-1:example.invalid" && room.display_name == "Member 1"
    }));
    assert!(
        snapshot
            .sidebar
            .space_rooms
            .iter()
            .all(|room| !room.room_id.starts_with("!dm-"))
    );
}

#[test]
fn fake_backend_backward_pagination_prepends_history_and_finishes() {
    let mut backend = FakeDesktopBackend::booted();
    let before = backend
        .snapshot()
        .timeline
        .iter()
        .map(|message| message.event_id.clone())
        .collect::<Vec<_>>();

    assert!(!before.iter().any(|event_id| event_id == "$alpha-history"));

    let effects = backend.dispatch(AppAction::TimelineBackPaginationRequested {
        room_id: "!room-alpha:example.invalid".to_owned(),
    });

    let snapshot = backend.snapshot();
    let after = snapshot
        .timeline
        .iter()
        .map(|message| message.event_id.clone())
        .collect::<Vec<_>>();

    assert!(!snapshot.state.timeline.is_paginating_backwards);
    assert_eq!(after.first().map(String::as_str), Some("$alpha-history"));
    assert_eq!(after.get(1), before.first());
    assert!(effects.iter().any(|effect| {
        matches!(
            effect,
            AppEffect::PaginateTimelineBackwards { room_id }
                if room_id == "!room-alpha:example.invalid"
        )
    }));
}

#[test]
fn fake_backend_upserts_sdk_timeline_messages_and_indexes_search() {
    let mut backend = FakeDesktopBackend::booted();

    backend.upsert_timeline_messages(vec![koushi_backend::TimelineMessage {
        room_id: "!room-alpha:example.invalid".to_owned(),
        event_id: "$sdk-visible".to_owned(),
        sender: "@sdk-user:example.invalid".to_owned(),
        timestamp_ms: 1_830_000_000_000,
        body: "SDK visible timeline body".to_owned(),
        attachment_filename: None,
        reply_count: 0,
    }]);

    let snapshot = backend.snapshot();
    assert!(snapshot.timeline.iter().any(|message| {
        message.event_id == "$sdk-visible" && message.body == "SDK visible timeline body"
    }));

    let results = backend.submit_search("SDK visible", SearchScope::AllRooms);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].event_id, "$sdk-visible");
}

#[test]
fn fake_backend_applies_sdk_timeline_remove_to_timeline_and_search() {
    let mut backend = FakeDesktopBackend::booted();

    backend.apply_timeline_updates(vec![
        koushi_backend::TimelineUpdate::Upsert(koushi_backend::TimelineMessage {
            room_id: "!room-alpha:example.invalid".to_owned(),
            event_id: "$sdk-redacted".to_owned(),
            sender: "@sdk-user:example.invalid".to_owned(),
            timestamp_ms: 1_830_000_000_000,
            body: "SDK redaction candidate body".to_owned(),
            attachment_filename: Some("sdk-redaction-candidate.txt".to_owned()),
            reply_count: 0,
        }),
        koushi_backend::TimelineUpdate::Remove {
            room_id: "!room-alpha:example.invalid".to_owned(),
            event_id: "$sdk-redacted".to_owned(),
        },
    ]);

    let snapshot = backend.snapshot();
    assert!(
        snapshot
            .timeline
            .iter()
            .all(|message| message.event_id != "$sdk-redacted")
    );
    assert!(
        backend
            .submit_search("SDK redaction candidate", SearchScope::AllRooms)
            .is_empty()
    );
    assert!(
        backend
            .submit_search("sdk-redaction-candidate.txt", SearchScope::AllRooms)
            .is_empty()
    );
}

#[test]
fn fake_backend_search_drops_ngram_false_positive() {
    let mut backend = FakeDesktopBackend::booted();

    let results = backend.submit_search("Alpha", SearchScope::AllRooms);

    assert_eq!(
        results
            .iter()
            .map(|result| result.event_id.as_str())
            .collect::<Vec<_>>(),
        vec!["$alpha-update"]
    );
    assert_eq!(results[0].match_field, SearchMatchField::MessageBody);
    assert_eq!(results[0].highlights[0].start_utf16, 0);
    assert!(matches!(
        backend.snapshot().state.search,
        SearchState::Results { .. }
    ));
}

#[test]
fn fake_backend_searches_attachment_filenames() {
    let mut backend = FakeDesktopBackend::booted();

    let results = backend.submit_search("fixture_budget.xlsx", SearchScope::AllRooms);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].event_id, "$budget-file");
    assert_eq!(results[0].match_field, SearchMatchField::AttachmentFileName);
    assert_eq!(results[0].snippet, "fixture_budget.xlsx");
}

#[test]
fn fake_backend_search_uses_visible_edited_message_body() {
    let mut backend = FakeDesktopBackend::booted();

    let results = backend.submit_search("checklist", SearchScope::AllRooms);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].event_id, "$late-original");
    assert_eq!(results[0].snippet, "Final synthetic checklist");
}

// #162: the SDK ngram candidate list is an accelerator, not the authority. A
// supplied candidate that does not match the query is still dropped, but store
// messages matching the query are found via the direct scan even when they were
// not supplied as candidates (the reported "0 results despite a visible match").
#[test]
fn fake_backend_search_unions_store_scan_with_supplied_candidates() {
    let mut backend = FakeDesktopBackend::booted();
    let budget_file_candidate = SearchCandidate {
        room_id: "!room-alpha:example.invalid".to_owned(),
        event_id: "$budget-file".to_owned(),
        score_millis: 950,
    };

    // Only a non-matching candidate is supplied, yet the store's "Alpha"
    // message is still found via the direct scan (previously returned empty).
    let store_scan_results = backend.submit_search_candidates(
        "Alpha",
        SearchScope::AllRooms,
        vec![budget_file_candidate.clone()],
    );

    assert!(
        store_scan_results
            .iter()
            .any(|result| result.event_id == "$alpha-update"),
        "store scan should surface the matching message without a supplied candidate"
    );

    let exact_results = backend.submit_search_candidates(
        "fixture_budget.xlsx",
        SearchScope::AllRooms,
        vec![budget_file_candidate],
    );

    assert_eq!(exact_results.len(), 1);
    assert_eq!(exact_results[0].event_id, "$budget-file");
    assert_eq!(
        exact_results[0].match_field,
        SearchMatchField::AttachmentFileName
    );
}

#[test]
fn fake_backend_edit_message_updates_visible_timeline_and_search_body() {
    let mut backend = FakeDesktopBackend::booted();

    backend.edit_message(
        "!room-alpha:example.invalid",
        "$alpha-update",
        "Edited synthetic update",
    );

    let snapshot = backend.snapshot();
    let edited = snapshot
        .timeline
        .iter()
        .find(|message| message.event_id == "$alpha-update")
        .expect("edited message should remain visible");
    assert_eq!(edited.body, "Edited synthetic update");

    let results = backend.submit_search("Edited synthetic", SearchScope::AllRooms);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].event_id, "$alpha-update");
    assert_eq!(results[0].snippet, "Edited synthetic update");
    assert!(
        backend
            .submit_search("Alpha keyword", SearchScope::AllRooms)
            .is_empty()
    );
}

#[test]
fn fake_backend_redact_message_removes_visible_timeline_and_search_result() {
    let mut backend = FakeDesktopBackend::booted();

    backend.redact_message("!room-alpha:example.invalid", "$budget-file");

    let snapshot = backend.snapshot();
    assert!(
        snapshot
            .timeline
            .iter()
            .all(|message| message.event_id != "$budget-file")
    );
    assert!(
        backend
            .submit_search("fixture_budget.xlsx", SearchScope::AllRooms)
            .is_empty()
    );
}

fn spawn_login_discovery_server(status: u16, body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let addr = listener
        .local_addr()
        .expect("test server should have an address");

    thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("test server should accept a request");
        let mut request = [0_u8; 2048];
        let bytes_read = stream
            .read(&mut request)
            .expect("test server should read request");
        let request = String::from_utf8_lossy(&request[..bytes_read]);
        assert!(request.starts_with("GET /_matrix/client/v3/login HTTP/1.1"));

        let response = format!(
            "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .expect("test server should write response");
    });

    format!("http://{addr}")
}

fn spawn_password_login_server(status: u16) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let addr = listener
        .local_addr()
        .expect("test server should have an address");

    thread::spawn(move || {
        for _ in 0..4 {
            let (mut stream, _) = listener
                .accept()
                .expect("test server should accept a request");
            let request = read_http_request(&mut stream);

            if request.starts_with("GET /_matrix/client/versions ") {
                write_json(
                    &mut stream,
                    200,
                    r#"{"versions":["r0.6.0","v1.1","v1.2","v1.3","v1.4","v1.5","v1.6","v1.7"]}"#,
                );
                continue;
            }

            if request.starts_with("POST /_matrix/client/")
                && request.contains("fixture-user")
                && request.contains("synthetic-password")
                && request.contains("Matrix Desktop Test")
            {
                if status == 200 {
                    write_json(
                        &mut stream,
                        200,
                        r#"{"access_token":"fixture-access-token","device_id":"FIXTUREDEVICE","user_id":"@fixture-user:example.invalid"}"#,
                    );
                } else {
                    write_json(
                        &mut stream,
                        status,
                        r#"{"errcode":"M_FORBIDDEN","error":"Invalid credentials"}"#,
                    );
                }
                return;
            }

            write_json(
                &mut stream,
                404,
                r#"{"errcode":"M_NOT_FOUND","error":"Unexpected test request"}"#,
            );
        }
    });

    format!("http://{addr}")
}

fn read_http_request(stream: &mut std::net::TcpStream) -> String {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];

    loop {
        let bytes_read = stream
            .read(&mut buffer)
            .expect("test server should read request");
        if bytes_read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..bytes_read]);

        let request_text = String::from_utf8_lossy(&request);
        let Some(header_end) = request_text.find("\r\n\r\n") else {
            continue;
        };
        let content_length = request_text
            .lines()
            .find_map(|line| {
                line.strip_prefix("Content-Length: ")
                    .and_then(|value| value.parse::<usize>().ok())
            })
            .unwrap_or(0);
        if request.len() >= header_end + 4 + content_length {
            break;
        }
    }

    String::from_utf8(request).expect("test request should be UTF-8")
}

fn write_json(stream: &mut std::net::TcpStream, status: u16, body: &str) {
    let response = format!(
        "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .expect("test server should write response");
}
