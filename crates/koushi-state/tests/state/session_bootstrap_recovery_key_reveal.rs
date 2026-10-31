//! Verification-gate identity bootstrap reveals its recovery key on screen
//! (#1049), reusing the #927 Secure Backup reveal. The key lives only in the
//! narrow `SecureBackupSetupState::RecoveryKeyReady` reveal slot; the widely
//! observed `SessionState::AwaitingBootstrapConfirmation` stays coarse.

use koushi_state::{
    AppAction, AppEffect, AppState, ProvisionalPhase, RecoveryKeyDeliveryState,
    RecoveryKeyMaterial, SecureBackupSetupState, SessionInfo, SessionState, UiEvent,
    VerificationAccountKind, VerificationGateRejectReason, VerificationGateState,
    VerificationMethod, VerificationMethodCapability, reduce,
};

// Synthetic, non-secret fixture; deliberately not shaped like a real key.
const SYNTHETIC_KEY: &str = "synthetic-bootstrap-recovery-key-fixture-1049";

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.org".to_owned(),
        user_id: "@user-a:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    }
}

fn bootstrap_gate() -> VerificationGateState {
    VerificationGateState {
        methods: vec![VerificationMethodCapability::Bootstrap],
        account_kind: VerificationAccountKind::NewIdentity,
        failure: None,
    }
}

fn key() -> RecoveryKeyMaterial {
    RecoveryKeyMaterial::new(SYNTHETIC_KEY)
}

fn bootstrapping_state(flow_id: u64) -> AppState {
    AppState {
        session: SessionState::Verifying {
            info: session_info(),
            gate: bootstrap_gate(),
            method: VerificationMethod::Bootstrap,
            flow_id,
            sas_emojis: vec![],
        },
        ..AppState::default()
    }
}

fn revealed_bootstrap_state(flow_id: u64) -> AppState {
    let mut state = bootstrapping_state(flow_id);
    reduce(
        &mut state,
        AppAction::BootstrapRecoveryKeyReady {
            flow_id,
            recovery_key: key(),
        },
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
fn bootstrap_reveals_the_key_without_a_destination_and_holds_the_gate() {
    let mut state = bootstrapping_state(41);
    let stale = reduce(
        &mut state,
        AppAction::BootstrapRecoveryKeyReady {
            flow_id: 40,
            recovery_key: key(),
        },
    );
    assert!(stale.is_empty());
    assert_eq!(state, bootstrapping_state(41));

    let effects = reduce(
        &mut state,
        AppAction::BootstrapRecoveryKeyReady {
            flow_id: 41,
            recovery_key: key(),
        },
    );
    assert_eq!(
        state.session,
        SessionState::AwaitingBootstrapConfirmation {
            info: session_info(),
            gate: bootstrap_gate(),
            flow_id: 41,
        }
    );
    assert_eq!(
        state.e2ee_trust.key_management.secure_backup_setup,
        SecureBackupSetupState::RecoveryKeyReady {
            request_id: 41,
            recovery_key: key(),
            delivery: RecoveryKeyDeliveryState::NotWritten,
            confirmation_failed: false,
        }
    );
    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::SessionChanged)));
    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::E2eeKeyManagementChanged)));
}

#[test]
fn saving_to_a_file_records_delivery_but_never_leaves_the_bootstrap_gate() {
    let mut state = revealed_bootstrap_state(41);
    reduce(
        &mut state,
        AppAction::SecureBackupRecoveryKeySaved {
            reveal_request_id: 41,
            written: true,
        },
    );
    assert!(matches!(
        state.session,
        SessionState::AwaitingBootstrapConfirmation { flow_id: 41, .. }
    ));
    assert!(matches!(
        state.e2ee_trust.key_management.secure_backup_setup,
        SecureBackupSetupState::RecoveryKeyReady {
            delivery: RecoveryKeyDeliveryState::Written,
            ..
        }
    ));
    assert_eq!(revealed_key(&state), Some(SYNTHETIC_KEY));
}

#[test]
fn explicit_confirmation_leaves_the_gate_and_drops_the_key() {
    let mut state = revealed_bootstrap_state(41);
    let before = state.clone();
    assert!(
        reduce(
            &mut state,
            AppAction::BootstrapRecoverySavedConfirmed { flow_id: 40 }
        )
        .is_empty()
    );
    assert_eq!(state, before);

    let effects = reduce(
        &mut state,
        AppAction::BootstrapRecoverySavedConfirmed { flow_id: 41 },
    );
    assert_eq!(
        state.session,
        SessionState::Provisional {
            info: session_info(),
            phase: ProvisionalPhase::RecheckingTrust { failure: None },
        }
    );
    assert_eq!(
        state.e2ee_trust.key_management.secure_backup_setup,
        SecureBackupSetupState::Enabled { request_id: 41 }
    );
    assert_eq!(effects.first(), Some(&AppEffect::CheckCurrentDeviceTrust));
    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::SessionChanged)));
    let serialized = serde_json::to_string(&state).expect("state serializes");
    assert!(!serialized.contains(SYNTHETIC_KEY));
}

#[test]
fn a_failed_confirmation_keeps_the_reveal_and_flags_it() {
    let mut state = revealed_bootstrap_state(41);
    assert!(
        reduce(
            &mut state,
            AppAction::BootstrapRecoverySavedConfirmFailed { flow_id: 40 }
        )
        .is_empty()
    );
    reduce(
        &mut state,
        AppAction::BootstrapRecoverySavedConfirmFailed { flow_id: 41 },
    );
    assert!(matches!(
        state.session,
        SessionState::AwaitingBootstrapConfirmation { flow_id: 41, .. }
    ));
    assert!(matches!(
        state.e2ee_trust.key_management.secure_backup_setup,
        SecureBackupSetupState::RecoveryKeyReady {
            request_id: 41,
            confirmation_failed: true,
            ..
        }
    ));
    assert_eq!(revealed_key(&state), Some(SYNTHETIC_KEY));
}

#[test]
fn rejecting_or_logging_out_of_the_bootstrap_gate_drops_the_key() {
    let mut rejected = revealed_bootstrap_state(41);
    reduce(
        &mut rejected,
        AppAction::VerificationSessionRejected {
            reason: VerificationGateRejectReason::ExistingIdentityWithoutProof,
        },
    );
    assert_eq!(revealed_key(&rejected), None);

    let mut logged_out = revealed_bootstrap_state(41);
    reduce(&mut logged_out, AppAction::LogoutRequested);
    assert_eq!(revealed_key(&logged_out), None);
}

#[test]
fn bootstrap_recovery_key_is_absent_from_the_session_surface_and_debug_output() {
    let state = revealed_bootstrap_state(41);
    assert_eq!(revealed_key(&state), Some(SYNTHETIC_KEY));
    let action = AppAction::BootstrapRecoveryKeyReady {
        flow_id: 41,
        recovery_key: key(),
    };
    // `SessionState` feeds QA window titles, gate diagnostics, and gate tests;
    // it must never carry the key in any rendering.
    let session_json = serde_json::to_string(&state.session).expect("session serializes");
    for rendered in [
        format!("{state:?}"),
        format!("{:?}", state.session),
        format!("{action:?}"),
        session_json,
    ] {
        assert!(!rendered.contains(SYNTHETIC_KEY), "{rendered}");
    }
}
