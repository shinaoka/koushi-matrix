use matrix_desktop_state::{
    AccountManagementOperation, AccountManagementState, AppAction, AppEffect, AppState,
    AuthDiscoveryState, AuthFailureKind, DelegatedAuthLinks, DeviceSessionListState,
    DeviceSessionSummary, E2eeKeyManagementState, LoginFlow, LoginFlowKind, QrLoginState,
    RecoveryKeyDeliveryState, RoomKeyExportState, RoomKeyImportState,
    SecureBackupPassphraseChangeState, SecureBackupSetupState, SessionInfo, SessionState,
    TrustOperationFailureKind, UiEvent, reduce,
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
            exported_sessions: 3,
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
            exported_sessions: 3,
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
