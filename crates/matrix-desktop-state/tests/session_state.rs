use matrix_desktop_state::{
    AppAction, AppEffect, AppState, AuthDiscoveryState, AuthFailureKind, AuthSecret,
    DelegatedAuthLinks, E2eeRecoveryState, LoginFlow, LoginFlowKind, LoginRequest,
    NativeAttentionCandidate, NativeAttentionCapabilities, NativeAttentionCapability,
    NativeAttentionState, NativeAttentionSummary, NavigationState, RecoveryMethod, RecoveryRequest,
    RoomAttentionKind, RoomSummary, RoomTags, SearchScope, SearchState, SessionInfo, SessionState,
    SpaceSummary, SyncState, ThreadAttentionState, ThreadPaneState, TimelinePaneState, UiEvent,
    reduce,
};

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.org".to_owned(),
        user_id: "@user-a:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
    }
}

fn alternate_session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.org".to_owned(),
        user_id: "@user-b:example.invalid".to_owned(),
        device_id: "DEVICE-B".to_owned(),
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
            display_name: None,
        },
        LoginFlow {
            kind: LoginFlowKind::Sso,
            delegated_oidc_compatibility: true,
            display_name: None,
        },
    ];

    let effects = reduce(
        &mut state,
        AppAction::LoginDiscoverySucceeded {
            homeserver: "https://matrix.example.org".to_owned(),
            flows: flows.clone(),
            delegated: DelegatedAuthLinks::default(),
        },
    );

    assert_eq!(
        state.auth,
        AuthDiscoveryState::Ready {
            homeserver: "https://matrix.example.org".to_owned(),
            flows,
            delegated: DelegatedAuthLinks::default(),
        }
    );
    assert_eq!(effects, vec![AppEffect::EmitUiEvent(UiEvent::AuthChanged)]);
}

#[test]
fn login_discovery_ignores_stale_completions_for_previous_homeserver() {
    let mut state = AppState {
        auth: AuthDiscoveryState::Discovering {
            homeserver: "https://new.example.org".to_owned(),
        },
        ..AppState::default()
    };

    let success_effects = reduce(
        &mut state,
        AppAction::LoginDiscoverySucceeded {
            homeserver: "https://old.example.org".to_owned(),
            flows: vec![LoginFlow {
                kind: LoginFlowKind::Password,
                delegated_oidc_compatibility: false,
                display_name: None,
            }],
            delegated: DelegatedAuthLinks::default(),
        },
    );

    assert!(success_effects.is_empty());
    assert_eq!(
        state.auth,
        AuthDiscoveryState::Discovering {
            homeserver: "https://new.example.org".to_owned(),
        }
    );

    let failure_effects = reduce(
        &mut state,
        AppAction::LoginDiscoveryFailed {
            homeserver: "https://old.example.org".to_owned(),
            kind: AuthFailureKind::Network,
        },
    );

    assert!(failure_effects.is_empty());
    assert_eq!(
        state.auth,
        AuthDiscoveryState::Discovering {
            homeserver: "https://new.example.org".to_owned(),
        }
    );
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
fn session_persistence_failure_records_error_without_leaving_ready_session() {
    let info = session_info();
    let mut state = AppState {
        session: SessionState::Ready(info.clone()),
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::SessionPersistenceFailed {
            message: "session was not saved".to_owned(),
        },
    );

    assert_eq!(state.session, SessionState::Ready(info));
    assert_eq!(state.errors[0].code, "session_persistence_failed");
    assert!(state.errors[0].recoverable);
    assert_eq!(effects, vec![AppEffect::EmitUiEvent(UiEvent::ErrorChanged)]);
}

#[test]
fn account_switch_request_stops_sync_clears_views_and_restores_target_session() {
    let current = session_info();
    let target = alternate_session_info();
    let mut state = AppState {
        session: SessionState::Ready(current),
        sync: SyncState::Running,
        navigation: NavigationState {
            active_space_id: Some("space-a".to_owned()),
            active_room_id: Some("room-a".to_owned()),
        },
        spaces: vec![SpaceSummary {
            space_id: "space-a".to_owned(),
            display_name: "Space A".to_owned(),
            avatar: None,
            child_room_ids: vec!["room-a".to_owned()],
        }],
        rooms: vec![RoomSummary {
            room_id: "room-a".to_owned(),
            display_name: "Room A".to_owned(),
            display_label: "Room A".to_owned(),
            original_display_label: "Room A".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            marked_unread: false,
            last_activity_ms: 0,
            parent_space_ids: vec!["space-a".to_owned()],
        }],
        timeline: TimelinePaneState {
            room_id: Some("room-a".to_owned()),
            is_subscribed: true,
            is_paginating_backwards: false,
            composer: Default::default(),
            scheduled_send_capability: Default::default(),
            scheduled_sends: Vec::new(),
            staged_uploads: Vec::new(),
            media_gallery: Vec::new(),
        },
        thread: ThreadPaneState::Open {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            is_subscribed: true,
            composer: Default::default(),
        },
        thread_attention: ThreadAttentionState::Tracking {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            notification_count: 2,
            highlight_count: 1,
            live_event_marker_count: 2,
        },
        search: SearchState::Editing {
            query: "hello".to_owned(),
            scope: SearchScope::AllRooms,
        },
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::SwitchAccountRequested {
            info: target.clone(),
        },
    );

    assert_eq!(
        state.session,
        SessionState::SwitchingAccount {
            info: target.clone()
        }
    );
    assert_eq!(state.sync, SyncState::Stopped);
    assert_eq!(state.navigation, NavigationState::default());
    assert!(state.spaces.is_empty());
    assert!(state.rooms.is_empty());
    assert_eq!(state.timeline, TimelinePaneState::default());
    assert_eq!(state.thread, ThreadPaneState::Closed);
    assert_eq!(state.thread_attention, ThreadAttentionState::Closed);
    assert_eq!(state.search, SearchState::Closed);
    assert_eq!(
        effects,
        vec![
            AppEffect::StopSync,
            AppEffect::ClearSession,
            AppEffect::RestoreSessionFor(target),
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
fn e2ee_recovery_required_after_login_stays_post_login_and_starts_sync() {
    let mut state = AppState {
        session: SessionState::Authenticating {
            homeserver: "https://matrix.example.org".to_owned(),
        },
        ..AppState::default()
    };
    let info = session_info();
    let methods = vec![RecoveryMethod::RecoveryKey, RecoveryMethod::SecurityPhrase];

    let effects = reduce(
        &mut state,
        AppAction::E2eeRecoveryRequired {
            info: info.clone(),
            methods: methods.clone(),
        },
    );

    assert_eq!(
        state.session,
        SessionState::NeedsRecovery {
            info: info.clone(),
            methods: methods.clone(),
        }
    );
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
fn e2ee_recovery_submission_emits_recover_effect_without_exposing_secret() {
    let info = session_info();
    let methods = vec![RecoveryMethod::RecoveryKey, RecoveryMethod::SecurityPhrase];
    let mut state = AppState {
        session: SessionState::NeedsRecovery {
            info: info.clone(),
            methods: methods.clone(),
        },
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::E2eeRecoverySubmitted(RecoveryRequest {
            secret: AuthSecret::new("synthetic-recovery-secret"),
        }),
    );

    assert_eq!(
        state.session,
        SessionState::Recovering {
            info: info.clone(),
            methods
        }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::RecoverE2ee(RecoveryRequest {
                secret: AuthSecret::new("synthetic-recovery-secret"),
            }),
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
        ]
    );
    assert!(!format!("{effects:?}").contains("synthetic-recovery-secret"));
}

#[test]
fn e2ee_recovery_success_enters_ready_and_starts_sync() {
    let info = session_info();
    let mut state = AppState {
        session: SessionState::Recovering {
            info: info.clone(),
            methods: vec![RecoveryMethod::RecoveryKey],
        },
        ..AppState::default()
    };

    let effects = reduce(&mut state, AppAction::E2eeRecoverySucceeded);

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
fn unknown_e2ee_recovery_state_does_not_prompt_or_stop_sync() {
    let info = session_info();
    let mut state = AppState {
        session: SessionState::Ready(info.clone()),
        sync: SyncState::Running,
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::E2eeRecoveryStateChanged {
            state: E2eeRecoveryState::Unknown,
            methods: vec![RecoveryMethod::RecoveryKey],
        },
    );

    assert_eq!(state.session, SessionState::Ready(info));
    assert_eq!(state.sync, SyncState::Running);
    assert!(effects.is_empty());
}

#[test]
fn incomplete_e2ee_recovery_state_prompts_without_stopping_sync() {
    let info = session_info();
    let mut state = AppState {
        session: SessionState::Ready(info.clone()),
        sync: SyncState::Running,
        navigation: NavigationState {
            active_space_id: Some("space-a".to_owned()),
            active_room_id: Some("room-a".to_owned()),
        },
        spaces: vec![SpaceSummary {
            space_id: "space-a".to_owned(),
            display_name: "Space A".to_owned(),
            avatar: None,
            child_room_ids: vec!["room-a".to_owned()],
        }],
        rooms: vec![RoomSummary {
            room_id: "room-a".to_owned(),
            display_name: "Room A".to_owned(),
            display_label: "Room A".to_owned(),
            original_display_label: "Room A".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 3,
            notification_count: 3,
            highlight_count: 0,
            marked_unread: false,
            last_activity_ms: 0,
            parent_space_ids: vec!["space-a".to_owned()],
        }],
        timeline: TimelinePaneState {
            room_id: Some("room-a".to_owned()),
            is_subscribed: true,
            is_paginating_backwards: false,
            composer: Default::default(),
            scheduled_send_capability: Default::default(),
            scheduled_sends: Vec::new(),
            staged_uploads: Vec::new(),
            media_gallery: Vec::new(),
        },
        ..AppState::default()
    };

    let methods = vec![RecoveryMethod::RecoveryKey, RecoveryMethod::SecurityPhrase];
    let effects = reduce(
        &mut state,
        AppAction::E2eeRecoveryStateChanged {
            state: E2eeRecoveryState::Incomplete,
            methods: methods.clone(),
        },
    );

    assert_eq!(
        state.session,
        SessionState::NeedsRecovery {
            info: info.clone(),
            methods
        }
    );
    assert_eq!(state.sync, SyncState::Running);
    assert_eq!(
        state.navigation,
        NavigationState {
            active_space_id: Some("space-a".to_owned()),
            active_room_id: Some("room-a".to_owned()),
        }
    );
    assert_eq!(state.spaces.len(), 1);
    assert_eq!(state.rooms.len(), 1);
    assert!(state.timeline.is_subscribed);
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
    );
}

#[test]
fn enabled_e2ee_recovery_state_releases_recovery_prompt() {
    let info = session_info();
    let mut state = AppState {
        session: SessionState::NeedsRecovery {
            info: info.clone(),
            methods: vec![RecoveryMethod::RecoveryKey],
        },
        sync: SyncState::Stopped,
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::E2eeRecoveryStateChanged {
            state: E2eeRecoveryState::Enabled,
            methods: vec![RecoveryMethod::RecoveryKey],
        },
    );

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
            avatar: None,
            child_room_ids: vec!["room-a".to_owned()],
        }],
        rooms: vec![RoomSummary {
            room_id: "room-a".to_owned(),
            display_name: "Room A".to_owned(),
            display_label: "Room A".to_owned(),
            original_display_label: "Room A".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 3,
            notification_count: 3,
            highlight_count: 0,
            marked_unread: false,
            last_activity_ms: 0,
            parent_space_ids: vec!["space-a".to_owned()],
        }],
        timeline: TimelinePaneState {
            room_id: Some("room-a".to_owned()),
            is_subscribed: true,
            is_paginating_backwards: true,
            composer: Default::default(),
            scheduled_send_capability: Default::default(),
            scheduled_sends: Vec::new(),
            staged_uploads: Vec::new(),
            media_gallery: Vec::new(),
        },
        thread: ThreadPaneState::Open {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            is_subscribed: true,
            composer: Default::default(),
        },
        thread_attention: ThreadAttentionState::Tracking {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            notification_count: 2,
            highlight_count: 1,
            live_event_marker_count: 2,
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
    assert_eq!(state.thread_attention, ThreadAttentionState::Closed);
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
fn logout_clears_native_attention_state_and_notifies_ui() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        sync: SyncState::Running,
        native_attention: NativeAttentionState {
            summary: NativeAttentionSummary {
                unread_count: 4,
                highlight_count: 1,
                badge_count: 4,
                candidate: Some(NativeAttentionCandidate {
                    room_display_name: "Announcements".to_owned(),
                    kind: RoomAttentionKind::Mention,
                    unread_count: 4,
                    highlight_count: 1,
                }),
                capabilities: NativeAttentionCapabilities {
                    notifications: NativeAttentionCapability::Available,
                    badge: NativeAttentionCapability::Available,
                    overlay_icon: NativeAttentionCapability::Available,
                    sound: NativeAttentionCapability::Available,
                    tray: NativeAttentionCapability::Available,
                    activation: NativeAttentionCapability::Available,
                },
            },
            dispatch: Default::default(),
        },
        ..AppState::default()
    };

    let effects = reduce(&mut state, AppAction::LogoutRequested);

    assert_eq!(state.native_attention, NativeAttentionState::default());
    assert_eq!(
        effects,
        vec![
            AppEffect::StopSync,
            AppEffect::ClearSession,
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
            AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged),
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
            avatar: None,
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
fn sync_failure_enters_failed_state_before_retry() {
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
        SyncState::Failed {
            reason: "limited network".to_owned(),
        }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
            AppEffect::StartSync,
        ]
    );
}

#[test]
fn sync_retry_enters_reconnecting_state() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        sync: SyncState::Failed {
            reason: "limited network".to_owned(),
        },
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::SyncReconnecting {
            reason: "limited network".to_owned(),
        },
    );

    assert_eq!(
        state.sync,
        SyncState::Reconnecting {
            reason: "limited network".to_owned(),
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
    );
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
    assert_eq!(
        reduce(
            &mut state,
            AppAction::SyncReconnecting {
                reason: "late reconnect".to_owned(),
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
