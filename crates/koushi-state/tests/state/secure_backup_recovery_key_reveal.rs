//! Secure Backup recovery-key reveal (#927): the generated key is shown on
//! screen, saving to a file is optional, and only the explicit "I saved the
//! recovery key" confirmation advances the gate and drops the key.

use koushi_state::{
    AppAction, AppState, RecoveryKeyDeliveryState, RecoveryKeyMaterial, SecureBackupGateState,
    SecureBackupPassphraseChangeState, SecureBackupSetupIntent, SecureBackupSetupState,
    SessionInfo, SessionState, encrypted_messaging_is_admitted, reduce,
};

// Synthetic, non-secret fixture; deliberately not shaped like a real key.
const SYNTHETIC_KEY: &str = "synthetic-recovery-key-fixture-927";

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.org".to_owned(),
        user_id: "@user-a:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    }
}

fn ready_state(gate: SecureBackupGateState) -> AppState {
    AppState {
        session: SessionState::Ready(session_info()),
        secure_backup_gate: gate,
        ..AppState::default()
    }
}

fn key() -> RecoveryKeyMaterial {
    RecoveryKeyMaterial::new(SYNTHETIC_KEY)
}

/// Drives InitialSetup to the reveal state the way AccountActor projects a
/// successful `recovery().enable()`.
fn revealed_setup_state() -> AppState {
    let mut state = ready_state(SecureBackupGateState::SetupRequired);
    reduce(
        &mut state,
        AppAction::SecureBackupSetupRequested {
            request_id: 7,
            intent: SecureBackupSetupIntent::InitialSetup,
        },
    );
    reduce(
        &mut state,
        AppAction::SecureBackupGateChanged(SecureBackupGateState::CreatingBackup),
    );
    reduce(
        &mut state,
        AppAction::SecureBackupRecoveryKeyReady {
            request_id: 7,
            recovery_key: key(),
        },
    );
    reduce(
        &mut state,
        AppAction::SecureBackupGateChanged(SecureBackupGateState::RecoveryKeyDeliveryRequired),
    );
    state
}

fn revealed_key(state: &AppState) -> Option<&str> {
    match &state.e2ee_trust.key_management.secure_backup_setup {
        SecureBackupSetupState::RecoveryKeyReady { recovery_key, .. } => {
            Some(recovery_key.expose_secret())
        }
        _ => None,
    }
}

#[test]
fn setup_success_reveals_the_key_without_a_file_and_keeps_the_gate_blocking() {
    let state = revealed_setup_state();

    assert_eq!(
        state.e2ee_trust.key_management.secure_backup_setup,
        SecureBackupSetupState::RecoveryKeyReady {
            request_id: 7,
            recovery_key: key(),
            delivery: RecoveryKeyDeliveryState::NotWritten,
            confirmation_failed: false,
        }
    );
    assert_eq!(revealed_key(&state), Some(SYNTHETIC_KEY));
    assert_eq!(
        state.secure_backup_gate,
        SecureBackupGateState::RecoveryKeyDeliveryRequired
    );
    assert!(!encrypted_messaging_is_admitted(&state));
}

#[test]
fn saving_to_a_file_records_delivery_but_does_not_advance_the_gate() {
    let mut state = revealed_setup_state();

    reduce(
        &mut state,
        AppAction::SecureBackupRecoveryKeySaved {
            reveal_request_id: 7,
            written: false,
        },
    );
    assert!(matches!(
        state.e2ee_trust.key_management.secure_backup_setup,
        SecureBackupSetupState::RecoveryKeyReady {
            delivery: RecoveryKeyDeliveryState::WriteFailed,
            ..
        }
    ));

    reduce(
        &mut state,
        AppAction::SecureBackupRecoveryKeySaved {
            reveal_request_id: 7,
            written: true,
        },
    );
    assert!(matches!(
        state.e2ee_trust.key_management.secure_backup_setup,
        SecureBackupSetupState::RecoveryKeyReady {
            delivery: RecoveryKeyDeliveryState::Written,
            ..
        }
    ));
    assert_eq!(revealed_key(&state), Some(SYNTHETIC_KEY));
    assert_eq!(
        state.secure_backup_gate,
        SecureBackupGateState::RecoveryKeyDeliveryRequired
    );
    assert!(!encrypted_messaging_is_admitted(&state));
}

#[test]
fn late_inspection_projection_does_not_drop_the_revealed_key() {
    let mut state = revealed_setup_state();

    reduce(
        &mut state,
        AppAction::SecureBackupGateChanged(SecureBackupGateState::RecoveryKeyDeliveryRequired),
    );

    assert_eq!(revealed_key(&state), Some(SYNTHETIC_KEY));
}

#[test]
fn stale_save_or_confirmation_is_a_no_op() {
    let mut state = revealed_setup_state();
    let before = state.clone();

    reduce(
        &mut state,
        AppAction::SecureBackupRecoveryKeySaved {
            reveal_request_id: 8,
            written: true,
        },
    );
    reduce(
        &mut state,
        AppAction::SecureBackupRecoveryKeyConfirmed {
            reveal_request_id: 8,
        },
    );

    assert_eq!(state, before);
}

#[test]
fn explicit_confirmation_advances_the_gate_and_drops_the_key() {
    let mut state = revealed_setup_state();

    reduce(
        &mut state,
        AppAction::SecureBackupRecoveryKeyConfirmed {
            reveal_request_id: 7,
        },
    );

    assert_eq!(
        state.e2ee_trust.key_management.secure_backup_setup,
        SecureBackupSetupState::Enabled { request_id: 7 }
    );
    assert_eq!(revealed_key(&state), None);
    assert_eq!(state.secure_backup_gate, SecureBackupGateState::Checking);
    let serialized = serde_json::to_string(&state).expect("state serializes");
    assert!(!serialized.contains(SYNTHETIC_KEY), "{serialized}");
}

#[test]
fn another_setup_request_cannot_replace_a_revealed_key() {
    let mut state = revealed_setup_state();
    let before = state.clone();

    reduce(
        &mut state,
        AppAction::SecureBackupSetupRequested {
            request_id: 9,
            intent: SecureBackupSetupIntent::InitialSetup,
        },
    );

    assert_eq!(state, before);
}

#[test]
fn only_one_recovery_key_is_revealed_at_a_time() {
    let mut setup_revealed = revealed_setup_state();
    setup_revealed.secure_backup_gate = SecureBackupGateState::Ready;
    let before = setup_revealed.clone();
    reduce(
        &mut setup_revealed,
        AppAction::SecureBackupPassphraseChangeRequested { request_id: 12 },
    );
    assert_eq!(setup_revealed, before);

    let mut change_revealed = ready_state(SecureBackupGateState::SetupRequired);
    change_revealed.e2ee_trust.key_management.passphrase_change =
        SecureBackupPassphraseChangeState::Changed {
            request_id: 11,
            recovery_key: key(),
            delivery: RecoveryKeyDeliveryState::NotWritten,
        };
    let before = change_revealed.clone();
    reduce(
        &mut change_revealed,
        AppAction::SecureBackupSetupRequested {
            request_id: 13,
            intent: SecureBackupSetupIntent::InitialSetup,
        },
    );
    reduce(
        &mut change_revealed,
        AppAction::SecureBackupPassphraseChangeRequested { request_id: 14 },
    );
    assert_eq!(change_revealed, before);
}

#[test]
fn passphrase_change_reveals_the_new_key_until_confirmation() {
    let mut state = ready_state(SecureBackupGateState::Ready);
    reduce(
        &mut state,
        AppAction::SecureBackupPassphraseChangeRequested { request_id: 11 },
    );
    reduce(
        &mut state,
        AppAction::SecureBackupPassphraseChanged {
            request_id: 11,
            recovery_key: key(),
        },
    );
    assert_eq!(
        state.e2ee_trust.key_management.passphrase_change,
        SecureBackupPassphraseChangeState::Changed {
            request_id: 11,
            recovery_key: key(),
            delivery: RecoveryKeyDeliveryState::NotWritten,
        }
    );

    reduce(
        &mut state,
        AppAction::SecureBackupRecoveryKeySaved {
            reveal_request_id: 11,
            written: true,
        },
    );
    assert!(matches!(
        state.e2ee_trust.key_management.passphrase_change,
        SecureBackupPassphraseChangeState::Changed {
            delivery: RecoveryKeyDeliveryState::Written,
            ..
        }
    ));
    assert_eq!(state.secure_backup_gate, SecureBackupGateState::Ready);

    reduce(
        &mut state,
        AppAction::SecureBackupRecoveryKeyConfirmed {
            reveal_request_id: 11,
        },
    );
    assert_eq!(
        state.e2ee_trust.key_management.passphrase_change,
        SecureBackupPassphraseChangeState::Idle
    );
    assert_eq!(state.secure_backup_gate, SecureBackupGateState::Ready);
}

#[test]
fn logout_and_account_switch_drop_a_revealed_key() {
    let mut logout = revealed_setup_state();
    reduce(&mut logout, AppAction::LogoutRequested);
    assert_eq!(revealed_key(&logout), None);

    let mut switch = revealed_setup_state();
    reduce(
        &mut switch,
        AppAction::SwitchAccountRequested {
            info: SessionInfo {
                user_id: "@user-b:example.invalid".to_owned(),
                device_id: "DEVICE-B".to_owned(),
                ..session_info()
            },
        },
    );
    assert_eq!(revealed_key(&switch), None);
}

#[test]
fn recovery_key_material_is_redacted_in_debug_output() {
    let state = revealed_setup_state();
    let action = AppAction::SecureBackupRecoveryKeyReady {
        request_id: 7,
        recovery_key: key(),
    };
    let passphrase_action = AppAction::SecureBackupPassphraseChanged {
        request_id: 11,
        recovery_key: key(),
    };

    for debug in [
        format!("{state:?}"),
        format!("{action:?}"),
        format!("{passphrase_action:?}"),
        format!("{:?}", key()),
        format!("{:?}", state.e2ee_trust.key_management),
    ] {
        assert!(!debug.contains(SYNTHETIC_KEY), "{debug}");
        assert!(!debug.contains("synthetic-recovery"), "{debug}");
    }
}

#[test]
fn recovery_key_material_serializes_as_the_plain_key_for_the_live_snapshot_only() {
    let state = revealed_setup_state();
    let value = serde_json::to_value(&state.e2ee_trust.key_management.secure_backup_setup)
        .expect("setup state serializes");

    assert_eq!(
        value,
        serde_json::json!({
            "kind": "recoveryKeyReady",
            "request_id": 7,
            "recovery_key": SYNTHETIC_KEY,
            "delivery": { "kind": "notWritten" },
            "confirmation_failed": false,
        })
    );
}

/// AccountActor holds a single revealed-key slot, so an in-flight setup and
/// an in-flight passphrase change must exclude each other before either
/// completes (#927 Astra P2).
#[test]
fn setup_and_passphrase_change_are_mutually_exclusive_while_in_flight() {
    let mut setting_up = ready_state(SecureBackupGateState::SetupRequired);
    reduce(
        &mut setting_up,
        AppAction::SecureBackupSetupRequested {
            request_id: 7,
            intent: SecureBackupSetupIntent::InitialSetup,
        },
    );
    assert_eq!(
        setting_up.e2ee_trust.key_management.secure_backup_setup,
        SecureBackupSetupState::SettingUp { request_id: 7 }
    );
    let before = setting_up.clone();
    reduce(
        &mut setting_up,
        AppAction::SecureBackupPassphraseChangeRequested { request_id: 8 },
    );
    assert_eq!(
        setting_up, before,
        "passphrase change admitted during setup"
    );

    let mut changing = ready_state(SecureBackupGateState::RecoveryKeyDeliveryRequired);
    reduce(
        &mut changing,
        AppAction::SecureBackupPassphraseChangeRequested { request_id: 8 },
    );
    assert_eq!(
        changing.e2ee_trust.key_management.passphrase_change,
        SecureBackupPassphraseChangeState::Changing { request_id: 8 }
    );
    let before = changing.clone();
    reduce(
        &mut changing,
        AppAction::SecureBackupSetupRequested {
            request_id: 9,
            intent: SecureBackupSetupIntent::ResetRecoveryKey { confirmed: true },
        },
    );
    assert_eq!(
        changing, before,
        "key reset admitted during passphrase change"
    );
}

/// When AccountActor cannot clear the persisted delivery marker after the
/// saved confirmation, it keeps its copy and restores the reveal so the key
/// the user just saved is not invalidated by a forced reset (#927).
#[test]
fn a_failed_marker_clear_restores_the_reveal_and_the_blocking_gate() {
    let mut state = revealed_setup_state();
    reduce(
        &mut state,
        AppAction::SecureBackupRecoveryKeySaved {
            reveal_request_id: 7,
            written: true,
        },
    );
    reduce(
        &mut state,
        AppAction::SecureBackupRecoveryKeyConfirmed {
            reveal_request_id: 7,
        },
    );
    assert_eq!(revealed_key(&state), None);

    // A stale restore is ignored.
    let before = state.clone();
    reduce(
        &mut state,
        AppAction::SecureBackupRecoveryKeyConfirmFailed {
            reveal_request_id: 8,
            recovery_key: key(),
            delivery: RecoveryKeyDeliveryState::Written,
        },
    );
    assert_eq!(state, before);

    reduce(
        &mut state,
        AppAction::SecureBackupRecoveryKeyConfirmFailed {
            reveal_request_id: 7,
            recovery_key: key(),
            delivery: RecoveryKeyDeliveryState::Written,
        },
    );
    assert_eq!(
        state.e2ee_trust.key_management.secure_backup_setup,
        SecureBackupSetupState::RecoveryKeyReady {
            request_id: 7,
            recovery_key: key(),
            delivery: RecoveryKeyDeliveryState::Written,
            confirmation_failed: true,
        }
    );
    assert_eq!(
        state.secure_backup_gate,
        SecureBackupGateState::RecoveryKeyDeliveryRequired
    );
    assert!(!encrypted_messaging_is_admitted(&state));
}
