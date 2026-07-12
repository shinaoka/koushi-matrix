use koushi_state::compute_room_list_projection;
use koushi_state::{
    AccountManagementOperation, AccountManagementState, AppAction, AppEffect, AppState,
    AuthDiscoveryState, AuthFailureKind, DelegatedAuthLinks, DeviceSessionListState,
    DeviceSessionSummary, E2eeKeyManagementState, LoginFlow, LoginFlowKind, OperationFailureKind,
    QrLoginState, RecoveryKeyDeliveryState, RoomKeyExportState, RoomKeyImportState,
    RoomListEntryKind, RoomListFilter, RoomListProjectionItem, RoomSummary, RoomTagInfo,
    SecureBackupPassphraseChangeState, SecureBackupSetupState, SessionInfo, SessionState,
    SoftLogoutReauthState, SyncMode, TrustOperationFailureKind, UiEvent, reduce,
};

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.org".to_owned(),
        user_id: "@user:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
    }
}

#[test]
fn auth_discovery_can_store_oidc_and_delegated_links_without_tokens() {
    let mut state = AppState {
        auth: AuthDiscoveryState::Discovering {
            homeserver: "https://example.test".to_owned(),
        },
        ..AppState::default()
    };
    let effects = reduce(
        &mut state,
        AppAction::LoginDiscoverySucceeded {
            homeserver: "https://example.test".to_owned(),
            flows: vec![LoginFlow {
                kind: LoginFlowKind::Oidc,
                delegated_oidc_compatibility: true,
                display_name: Some("Provider".to_owned()),
            }],
            delegated: DelegatedAuthLinks {
                registration_url: Some("https://example.test/register".to_owned()),
                account_management_url: Some("https://example.test/account".to_owned()),
            },
        },
    );

    assert!(
        effects
            .iter()
            .any(|effect| { matches!(effect, AppEffect::EmitUiEvent(UiEvent::AuthChanged)) })
    );
    assert_eq!(
        state.auth,
        AuthDiscoveryState::Ready {
            homeserver: "https://example.test".to_owned(),
            flows: vec![LoginFlow {
                kind: LoginFlowKind::Oidc,
                delegated_oidc_compatibility: true,
                display_name: Some("Provider".to_owned()),
            }],
            delegated: DelegatedAuthLinks {
                registration_url: Some("https://example.test/register".to_owned()),
                account_management_url: Some("https://example.test/account".to_owned()),
            },
        }
    );
}

#[test]
fn package_a_substates_start_secret_free_and_idle() {
    let state = AppState::default();
    assert!(matches!(
        state.e2ee_trust.key_management,
        E2eeKeyManagementState {
            room_key_export: RoomKeyExportState::Idle,
            room_key_import: RoomKeyImportState::Idle,
            secure_backup_setup: SecureBackupSetupState::Idle,
            passphrase_change: SecureBackupPassphraseChangeState::Idle,
        }
    ));
    assert_eq!(state.device_sessions, DeviceSessionListState::Idle);
    assert_eq!(state.account_management, AccountManagementState::Idle);
    assert_eq!(state.qr_login, QrLoginState::Idle);
}

#[test]
fn auth_failure_kind_is_coarse() {
    assert_eq!(format!("{:?}", AuthFailureKind::Network), "Network");
    assert_eq!(format!("{:?}", AuthFailureKind::Sdk), "Sdk");
}

#[test]
fn device_session_loading_is_ready_guarded_and_request_correlated() {
    let mut state = AppState::default();

    let effects = reduce(
        &mut state,
        AppAction::DeviceSessionsLoadRequested { request_id: 1 },
    );

    assert!(effects.is_empty());
    assert_eq!(state.device_sessions, DeviceSessionListState::Idle);

    state.session = SessionState::Ready(session_info());
    let effects = reduce(
        &mut state,
        AppAction::DeviceSessionsLoadRequested { request_id: 1 },
    );

    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::DeviceSessionsChanged)]
    );
    assert_eq!(
        state.device_sessions,
        DeviceSessionListState::Loading { request_id: 1 }
    );

    let effects = reduce(
        &mut state,
        AppAction::DeviceSessionsLoadRequested { request_id: 2 },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.device_sessions,
        DeviceSessionListState::Loading { request_id: 1 }
    );

    let effects = reduce(
        &mut state,
        AppAction::DeviceSessionsLoaded {
            request_id: 2,
            devices: Vec::new(),
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.device_sessions,
        DeviceSessionListState::Loading { request_id: 1 }
    );

    let devices = vec![DeviceSessionSummary {
        device_ordinal: 1,
        display_name: Some("Desktop".to_owned()),
        current: true,
        verified: true,
        inactive: false,
    }];
    let effects = reduce(
        &mut state,
        AppAction::DeviceSessionsLoaded {
            request_id: 1,
            devices: devices.clone(),
        },
    );

    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::DeviceSessionsChanged)]
    );
    assert_eq!(
        state.device_sessions,
        DeviceSessionListState::Loaded { devices }
    );
}

#[test]
fn account_management_is_ready_guarded_duplicate_guarded_and_request_correlated() {
    let mut state = AppState::default();
    let operation = AccountManagementOperation::ChangePassword;

    let effects = reduce(
        &mut state,
        AppAction::AccountManagementRequested {
            request_id: 10,
            operation,
        },
    );

    assert!(effects.is_empty());
    assert_eq!(state.account_management, AccountManagementState::Idle);

    state.session = SessionState::Ready(session_info());
    let effects = reduce(
        &mut state,
        AppAction::AccountManagementRequested {
            request_id: 10,
            operation,
        },
    );

    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::AccountManagementChanged)]
    );
    assert_eq!(
        state.account_management,
        AccountManagementState::Working {
            request_id: 10,
            operation,
        }
    );

    let effects = reduce(
        &mut state,
        AppAction::AccountManagementRequested {
            request_id: 11,
            operation: AccountManagementOperation::DeleteDevice,
        },
    );
    assert!(effects.is_empty());

    let effects = reduce(
        &mut state,
        AppAction::AccountManagementUiaRequired {
            request_id: 10,
            flow_id: 50,
            operation: AccountManagementOperation::DeleteDevice,
        },
    );
    assert!(effects.is_empty());

    let effects = reduce(
        &mut state,
        AppAction::AccountManagementUiaRequired {
            request_id: 10,
            flow_id: 50,
            operation,
        },
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::AccountManagementChanged)]
    );
    assert_eq!(
        state.account_management,
        AccountManagementState::AwaitingUia {
            request_id: 10,
            flow_id: 50,
            operation,
        }
    );

    let effects = reduce(
        &mut state,
        AppAction::AccountManagementSucceeded {
            request_id: 11,
            operation,
        },
    );
    assert!(effects.is_empty());

    let effects = reduce(
        &mut state,
        AppAction::AccountManagementSucceeded {
            request_id: 10,
            operation,
        },
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::AccountManagementChanged)]
    );
    assert_eq!(
        state.account_management,
        AccountManagementState::Succeeded {
            request_id: 10,
            operation,
        }
    );
}

#[test]
fn key_management_and_qr_login_are_duplicate_guarded_and_request_correlated() {
    let mut state = AppState::default();

    let effects = reduce(
        &mut state,
        AppAction::RoomKeyExportRequested { request_id: 20 },
    );

    assert!(effects.is_empty());
    assert_eq!(
        state.e2ee_trust.key_management.room_key_export,
        RoomKeyExportState::Idle
    );

    state.session = SessionState::Ready(session_info());
    let effects = reduce(
        &mut state,
        AppAction::RoomKeyExportRequested { request_id: 20 },
    );

    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged),
            AppEffect::EmitUiEvent(UiEvent::E2eeKeyManagementChanged),
        ]
    );
    assert_eq!(
        state.e2ee_trust.key_management.room_key_export,
        RoomKeyExportState::Exporting { request_id: 20 }
    );

    let effects = reduce(
        &mut state,
        AppAction::RoomKeyExportRequested { request_id: 21 },
    );
    assert!(effects.is_empty());

    let effects = reduce(
        &mut state,
        AppAction::RoomKeyExportFailed {
            request_id: 21,
            kind: TrustOperationFailureKind::Sdk,
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.e2ee_trust.key_management.room_key_export,
        RoomKeyExportState::Exporting { request_id: 20 }
    );

    let effects = reduce(
        &mut state,
        AppAction::RoomKeyExported {
            request_id: 20,
            exported_sessions: Some(3),
        },
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::E2eeTrustChanged),
            AppEffect::EmitUiEvent(UiEvent::E2eeKeyManagementChanged),
        ]
    );
    assert_eq!(
        state.e2ee_trust.key_management.room_key_export,
        RoomKeyExportState::Exported {
            request_id: 20,
            exported_sessions: Some(3),
        }
    );

    let effects = reduce(
        &mut state,
        AppAction::SecureBackupRecoveryKeyReady {
            request_id: 30,
            delivery: RecoveryKeyDeliveryState::Written,
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.e2ee_trust.key_management.secure_backup_setup,
        SecureBackupSetupState::Idle
    );

    let effects = reduce(
        &mut state,
        AppAction::QrLoginCapabilityCheckRequested { request_id: 40 },
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::QrLoginChanged)]
    );
    assert_eq!(
        state.qr_login,
        QrLoginState::CheckingCapability { request_id: 40 }
    );

    let effects = reduce(&mut state, AppAction::QrLoginScanStarted { request_id: 41 });
    assert!(effects.is_empty());
    assert_eq!(
        state.qr_login,
        QrLoginState::CheckingCapability { request_id: 40 }
    );
}

#[test]
fn soft_logout_reauth_requested_sets_authenticating_and_emits_ui_event() {
    let mut state = AppState::default();

    let effects = reduce(
        &mut state,
        AppAction::SoftLogoutReauthRequested { request_id: 1 },
    );
    assert!(effects.is_empty());
    assert_eq!(state.soft_logout_reauth, SoftLogoutReauthState::Idle);

    state.session = SessionState::Ready(session_info());
    let effects = reduce(
        &mut state,
        AppAction::SoftLogoutReauthRequested { request_id: 1 },
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::SoftLogoutReauthChanged)]
    );
    assert_eq!(
        state.soft_logout_reauth,
        SoftLogoutReauthState::Authenticating { request_id: 1 }
    );

    let mut locked = AppState::default();
    locked.session = SessionState::Locked(session_info());
    let effects = reduce(
        &mut locked,
        AppAction::SoftLogoutReauthRequested { request_id: 10 },
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::SoftLogoutReauthChanged)]
    );
    assert_eq!(
        locked.soft_logout_reauth,
        SoftLogoutReauthState::Authenticating { request_id: 10 }
    );

    let effects = reduce(
        &mut state,
        AppAction::SoftLogoutReauthRequested { request_id: 2 },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.soft_logout_reauth,
        SoftLogoutReauthState::Authenticating { request_id: 1 }
    );
}

#[test]
fn soft_logout_reauth_succeeds_and_fails_are_request_correlated() {
    let mut state = AppState::default();
    state.session = SessionState::Ready(session_info());
    reduce(
        &mut state,
        AppAction::SoftLogoutReauthRequested { request_id: 1 },
    );
    assert_eq!(
        state.soft_logout_reauth,
        SoftLogoutReauthState::Authenticating { request_id: 1 }
    );

    let effects = reduce(
        &mut state,
        AppAction::SoftLogoutReauthSucceeded { request_id: 2 },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.soft_logout_reauth,
        SoftLogoutReauthState::Authenticating { request_id: 1 }
    );

    let effects = reduce(
        &mut state,
        AppAction::SoftLogoutReauthFailed {
            request_id: 2,
            kind: AuthFailureKind::Forbidden,
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.soft_logout_reauth,
        SoftLogoutReauthState::Authenticating { request_id: 1 }
    );

    let effects = reduce(
        &mut state,
        AppAction::SoftLogoutReauthSucceeded { request_id: 1 },
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::SoftLogoutReauthChanged)]
    );
    assert_eq!(
        state.soft_logout_reauth,
        SoftLogoutReauthState::Succeeded { request_id: 1 }
    );
    state.session = SessionState::Authenticating {
        homeserver: session_info().homeserver,
    };
    let effects = reduce(&mut state, AppAction::LoginSucceeded(session_info()));
    assert!(matches!(state.session, SessionState::Provisional { .. }));
    assert!(effects.contains(&AppEffect::CheckCurrentDeviceTrust));

    let mut state = AppState::default();
    state.session = SessionState::Ready(session_info());
    reduce(
        &mut state,
        AppAction::SoftLogoutReauthRequested { request_id: 5 },
    );
    let effects = reduce(
        &mut state,
        AppAction::SoftLogoutReauthFailed {
            request_id: 5,
            kind: AuthFailureKind::Sdk,
        },
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::SoftLogoutReauthChanged)]
    );
    assert_eq!(
        state.soft_logout_reauth,
        SoftLogoutReauthState::Failed {
            request_id: 5,
            kind: AuthFailureKind::Sdk,
        }
    );
}

#[test]
fn soft_logout_reauth_is_cleared_on_logout() {
    let mut state = AppState::default();
    state.session = SessionState::Ready(session_info());
    state.soft_logout_reauth = SoftLogoutReauthState::Authenticating { request_id: 7 };

    let effects = reduce(&mut state, AppAction::LogoutRequested);

    assert!(effects.iter().any(|effect| matches!(
        effect,
        AppEffect::EmitUiEvent(UiEvent::SoftLogoutReauthChanged)
    )));
    assert_eq!(state.soft_logout_reauth, SoftLogoutReauthState::Idle);
    assert!(matches!(state.session, SessionState::LoggingOut));
}

#[test]
fn account_management_auth_submitted_transitions_awaiting_uia_to_working() {
    let mut state = AppState::default();
    state.session = SessionState::Ready(session_info());
    let operation = AccountManagementOperation::DeleteDevice;

    reduce(
        &mut state,
        AppAction::AccountManagementRequested {
            request_id: 10,
            operation,
        },
    );
    reduce(
        &mut state,
        AppAction::AccountManagementUiaRequired {
            request_id: 10,
            flow_id: 10,
            operation,
        },
    );
    assert_eq!(
        state.account_management,
        AccountManagementState::AwaitingUia {
            request_id: 10,
            flow_id: 10,
            operation,
        }
    );

    let effects = reduce(
        &mut state,
        AppAction::AccountManagementAuthSubmitted {
            request_id: 11,
            flow_id: 10,
        },
    );
    assert!(effects.is_empty());
    assert!(matches!(
        state.account_management,
        AccountManagementState::AwaitingUia { .. }
    ));

    let effects = reduce(
        &mut state,
        AppAction::AccountManagementAuthSubmitted {
            request_id: 10,
            flow_id: 11,
        },
    );
    assert!(effects.is_empty());

    let effects = reduce(
        &mut state,
        AppAction::AccountManagementAuthSubmitted {
            request_id: 10,
            flow_id: 10,
        },
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::AccountManagementChanged)]
    );
    assert_eq!(
        state.account_management,
        AccountManagementState::Working {
            request_id: 10,
            operation,
        }
    );
}
fn ready_state_with_rooms(rooms: Vec<RoomSummary>) -> AppState {
    AppState {
        session: SessionState::Ready(session_info()),
        sync_mode: SyncMode::Simplified,
        rooms,
        ..AppState::default()
    }
}

fn room_summary(
    room_id: &str,
    is_dm: bool,
    unread_count: u64,
    notification_count: u64,
    marked_unread: bool,
) -> RoomSummary {
    RoomSummary {
        room_id: room_id.to_owned(),
        display_name: room_id.to_owned(),
        display_label: room_id.to_owned(),
        original_display_label: room_id.to_owned(),
        avatar: None,
        is_dm,
        dm_user_ids: Vec::new(),
        tags: Default::default(),
        unread_count,
        notification_count,
        highlight_count: 0,
        marked_unread,
        last_activity_ms: 0,
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 0,
    }
}

#[test]
fn room_list_filter_selection_is_session_ready_guarded_and_recomputes_projection() {
    let rooms = vec![
        room_summary("!room1:example.invalid", false, 5, 0, false),
        room_summary("!dm1:example.invalid", true, 0, 0, false),
    ];

    let mut state = AppState::default();
    let effects = reduce(
        &mut state,
        AppAction::RoomListFilterSelected {
            filter: RoomListFilter::People,
        },
    );
    assert!(effects.is_empty());
    assert_eq!(state.room_list.active_filter, RoomListFilter::Rooms);

    state = ready_state_with_rooms(rooms);
    let effects = reduce(
        &mut state,
        AppAction::RoomListFilterSelected {
            filter: RoomListFilter::People,
        },
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
    );
    assert_eq!(state.room_list.active_filter, RoomListFilter::People);
    assert_eq!(
        state.room_list.items,
        vec![RoomListProjectionItem {
            room_id: "!dm1:example.invalid".to_owned(),
            kind: RoomListEntryKind::Room,
        }]
    );

    let effects = reduce(
        &mut state,
        AppAction::RoomListFilterSelected {
            filter: RoomListFilter::People,
        },
    );
    assert!(effects.is_empty());
}

#[test]
fn room_list_rooms_filter_excludes_favourites_and_low_priority_rooms() {
    let mut favourite = room_summary("!fav:example.invalid", false, 0, 0, false);
    favourite.tags.favourite = Some(RoomTagInfo {
        order: Some("0.1".to_owned()),
    });
    let ordinary = room_summary("!ordinary:example.invalid", false, 0, 0, false);
    let mut low_priority = room_summary("!low:example.invalid", false, 0, 0, false);
    low_priority.tags.low_priority = Some(RoomTagInfo {
        order: Some("0.9".to_owned()),
    });
    let dm = room_summary("!dm:example.invalid", true, 0, 0, false);

    let rooms = vec![favourite, ordinary, low_priority, dm];
    let projection = compute_room_list_projection(
        RoomListFilter::Rooms,
        Default::default(),
        None,
        &[],
        &rooms,
        &[],
    );

    assert_eq!(
        projection.items,
        vec![RoomListProjectionItem {
            room_id: "!ordinary:example.invalid".to_owned(),
            kind: RoomListEntryKind::Room,
        }]
    );
}

#[test]
fn room_list_filter_applied_updates_projection_when_changed() {
    let rooms = vec![room_summary("!room1:example.invalid", false, 0, 0, false)];
    let mut state = ready_state_with_rooms(rooms);

    let projection = state.room_list.clone();
    let effects = reduce(
        &mut state,
        AppAction::RoomListFilterApplied {
            projection: projection.clone(),
        },
    );
    assert!(effects.is_empty());

    let mut updated_projection = projection.clone();
    updated_projection.active_filter = RoomListFilter::Unread;
    let effects = reduce(
        &mut state,
        AppAction::RoomListFilterApplied {
            projection: updated_projection,
        },
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
    );
    assert_eq!(state.room_list.active_filter, RoomListFilter::Unread);
}

#[test]
fn mark_as_read_clears_unread_state_and_recomputes_room_list_projection() {
    let rooms = vec![room_summary("!room1:example.invalid", false, 3, 1, true)];
    let mut state = ready_state_with_rooms(rooms);
    state.room_list.active_filter = RoomListFilter::Unread;
    state.room_list = compute_room_list_projection(
        RoomListFilter::Unread,
        state.room_list.sort,
        state.navigation.active_space_id.as_deref(),
        &state.spaces,
        &state.rooms,
        &state.invites,
    );
    assert_eq!(state.room_list.items.len(), 1);

    let effects = reduce(
        &mut state,
        AppAction::RoomMarkedAsReadRequested {
            request_id: 1,
            room_id: "!room1:example.invalid".to_owned(),
            event_id: "$event1".to_owned(),
        },
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
    );

    let effects = reduce(
        &mut state,
        AppAction::RoomMarkedAsReadSucceeded {
            request_id: 1,
            room_id: "!room1:example.invalid".to_owned(),
        },
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
    );

    let room = state
        .rooms
        .iter()
        .find(|r| r.room_id == "!room1:example.invalid")
        .unwrap();
    assert!(!room.marked_unread);
    assert_eq!(room.unread_count, 0);
    assert_eq!(room.notification_count, 0);
    assert!(state.room_list.items.is_empty());
}

#[test]
fn mark_as_read_success_is_ignored_while_session_is_locked() {
    let rooms = vec![room_summary("!room1:example.invalid", false, 3, 1, true)];
    let mut state = ready_state_with_rooms(rooms);
    state.room_list.active_filter = RoomListFilter::Unread;
    state.room_list = compute_room_list_projection(
        RoomListFilter::Unread,
        state.room_list.sort,
        state.navigation.active_space_id.as_deref(),
        &state.spaces,
        &state.rooms,
        &state.invites,
    );
    state.session = SessionState::Locked(session_info());

    let effects = reduce(
        &mut state,
        AppAction::RoomMarkedAsReadSucceeded {
            request_id: 1,
            room_id: "!room1:example.invalid".to_owned(),
        },
    );

    assert!(effects.is_empty());
    let room = state
        .rooms
        .iter()
        .find(|r| r.room_id == "!room1:example.invalid")
        .unwrap();
    assert!(room.marked_unread);
    assert_eq!(room.unread_count, 3);
    assert_eq!(room.notification_count, 1);
    assert!(!state.room_list.items.is_empty());
}

#[test]
fn mark_as_read_failure_emits_error_event() {
    let rooms = vec![room_summary("!room1:example.invalid", false, 3, 0, false)];
    let mut state = ready_state_with_rooms(rooms);

    let effects = reduce(
        &mut state,
        AppAction::RoomMarkedAsReadFailed {
            request_id: 1,
            room_id: "!room1:example.invalid".to_owned(),
            kind: OperationFailureKind::Sdk,
        },
    );
    assert_eq!(effects, vec![AppEffect::EmitUiEvent(UiEvent::ErrorChanged)]);
}

#[test]
fn mark_as_unread_sets_unread_flag_and_recomputes_room_list_projection() {
    let rooms = vec![room_summary("!room1:example.invalid", false, 0, 0, false)];
    let mut state = ready_state_with_rooms(rooms);
    state.room_list.active_filter = RoomListFilter::Unread;
    state.room_list = compute_room_list_projection(
        RoomListFilter::Unread,
        state.room_list.sort,
        state.navigation.active_space_id.as_deref(),
        &state.spaces,
        &state.rooms,
        &state.invites,
    );
    assert!(state.room_list.items.is_empty());

    let effects = reduce(
        &mut state,
        AppAction::RoomMarkedAsUnreadRequested {
            request_id: 1,
            room_id: "!room1:example.invalid".to_owned(),
            unread: true,
        },
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
    );

    let effects = reduce(
        &mut state,
        AppAction::RoomMarkedAsUnreadSucceeded {
            request_id: 1,
            room_id: "!room1:example.invalid".to_owned(),
            unread: true,
        },
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
    );

    let room = state
        .rooms
        .iter()
        .find(|r| r.room_id == "!room1:example.invalid")
        .unwrap();
    assert!(room.marked_unread);
    assert_eq!(room.unread_count, 1);
    assert_eq!(state.room_list.items.len(), 1);
}

#[test]
fn mark_as_unread_clear_resets_unread_state() {
    let rooms = vec![room_summary("!room1:example.invalid", false, 5, 0, true)];
    let mut state = ready_state_with_rooms(rooms);

    let effects = reduce(
        &mut state,
        AppAction::RoomMarkedAsUnreadSucceeded {
            request_id: 1,
            room_id: "!room1:example.invalid".to_owned(),
            unread: false,
        },
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
    );

    let room = state
        .rooms
        .iter()
        .find(|r| r.room_id == "!room1:example.invalid")
        .unwrap();
    assert!(!room.marked_unread);
    assert_eq!(room.unread_count, 0);
}

#[test]
fn mark_read_and_unread_requests_are_ignored_for_unknown_rooms() {
    let rooms = vec![room_summary("!room1:example.invalid", false, 0, 0, false)];
    let mut state = ready_state_with_rooms(rooms);

    let effects = reduce(
        &mut state,
        AppAction::RoomMarkedAsReadRequested {
            request_id: 1,
            room_id: "!unknown:example.invalid".to_owned(),
            event_id: "$event".to_owned(),
        },
    );
    assert!(effects.is_empty());

    let effects = reduce(
        &mut state,
        AppAction::RoomMarkedAsUnreadRequested {
            request_id: 2,
            room_id: "!unknown:example.invalid".to_owned(),
            unread: true,
        },
    );
    assert!(effects.is_empty());
}
