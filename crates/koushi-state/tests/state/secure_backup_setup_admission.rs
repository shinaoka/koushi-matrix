use koushi_state::{
    AppAction, AppState, SecureBackupGateState, SecureBackupSetupAdmission,
    SecureBackupSetupIntent, SecureBackupSetupState, SessionInfo, SessionState, reduce,
};

fn ready_state(gate: SecureBackupGateState) -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://server.example.invalid".to_owned(),
            user_id: "@alice:example.invalid".to_owned(),
            device_id: "DEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        secure_backup_gate: gate,
        ..Default::default()
    }
}

#[test]
fn secure_backup_setup_admission_covers_each_gate_and_intent() {
    let gates = [
        SecureBackupGateState::Inactive,
        SecureBackupGateState::Checking,
        SecureBackupGateState::ExistingBackupNeedsRecovery { failure: None },
        SecureBackupGateState::SecureStorageIncomplete,
        SecureBackupGateState::SetupRequired,
        SecureBackupGateState::ExplicitlyDisabledRequiresSetup,
        SecureBackupGateState::CreatingBackup,
        SecureBackupGateState::RecoveryKeyDeliveryRequired,
        SecureBackupGateState::UploadingExistingKeys {
            pending: koushi_state::PendingKeyCountBucket::One,
        },
        SecureBackupGateState::DegradedRetrying {
            failure: koushi_state::SecureBackupGateFailureKind::Network,
            detail: None,
        },
        SecureBackupGateState::BlockedFailed {
            failure: koushi_state::SecureBackupGateFailureKind::Sdk,
            detail: None,
        },
        SecureBackupGateState::Ready,
    ];

    for gate in gates {
        // #927: an existing backup is never re-exported; a lost reveal uses
        // the confirmed `ResetRecoveryKey` intent instead.
        let initial_allowed = gate == SecureBackupGateState::SetupRequired;
        let mut state = ready_state(gate.clone());
        let effects = reduce(
            &mut state,
            AppAction::SecureBackupSetupRequested {
                request_id: 1,
                intent: SecureBackupSetupIntent::InitialSetup,
            },
        );
        assert_eq!(
            !effects.is_empty(),
            initial_allowed,
            "initial setup admission for {gate:?}"
        );
        if initial_allowed {
            assert!(matches!(
                state.e2ee_trust.key_management.secure_backup_setup,
                SecureBackupSetupState::SettingUp { request_id: 1 }
            ));
        } else {
            assert_eq!(
                state.e2ee_trust.key_management.secure_backup_setup,
                SecureBackupSetupState::Idle
            );
        }

        let mut state = ready_state(gate.clone());
        let effects = reduce(
            &mut state,
            AppAction::SecureBackupSetupRequested {
                request_id: 2,
                intent: SecureBackupSetupIntent::Reenable { confirmed: false },
            },
        );
        assert!(effects.is_empty(), "unconfirmed re-enable for {gate:?}");
        assert_eq!(
            state.e2ee_trust.key_management.secure_backup_setup,
            SecureBackupSetupState::Idle
        );

        let mut state = ready_state(gate.clone());
        let effects = reduce(
            &mut state,
            AppAction::SecureBackupSetupRequested {
                request_id: 3,
                intent: SecureBackupSetupIntent::Reenable { confirmed: true },
            },
        );
        assert_eq!(
            !effects.is_empty(),
            gate == SecureBackupGateState::ExplicitlyDisabledRequiresSetup,
            "confirmed re-enable admission for {gate:?}"
        );

        let delivery_required = gate == SecureBackupGateState::RecoveryKeyDeliveryRequired;
        assert_eq!(
            SecureBackupSetupIntent::ResetRecoveryKey { confirmed: false }.admission(&gate),
            if delivery_required {
                SecureBackupSetupAdmission::ConfirmationRequired
            } else {
                SecureBackupSetupAdmission::FailedNoOp
            },
            "unconfirmed recovery-key reset admission for {gate:?}"
        );
        let mut state = ready_state(gate.clone());
        let effects = reduce(
            &mut state,
            AppAction::SecureBackupSetupRequested {
                request_id: 4,
                intent: SecureBackupSetupIntent::ResetRecoveryKey { confirmed: false },
            },
        );
        assert!(effects.is_empty(), "unconfirmed reset for {gate:?}");

        let mut state = ready_state(gate.clone());
        let effects = reduce(
            &mut state,
            AppAction::SecureBackupSetupRequested {
                request_id: 5,
                intent: SecureBackupSetupIntent::ResetRecoveryKey { confirmed: true },
            },
        );
        assert_eq!(
            !effects.is_empty(),
            delivery_required,
            "confirmed recovery-key reset admission for {gate:?}"
        );
        if delivery_required {
            assert!(matches!(
                state.e2ee_trust.key_management.secure_backup_setup,
                SecureBackupSetupState::SettingUp { request_id: 5 }
            ));
        }
    }
}

#[test]
fn duplicate_secure_backup_setup_is_a_no_op() {
    let mut state = ready_state(SecureBackupGateState::SetupRequired);
    reduce(
        &mut state,
        AppAction::SecureBackupSetupRequested {
            request_id: 1,
            intent: SecureBackupSetupIntent::InitialSetup,
        },
    );
    let before = state.clone();
    assert!(
        reduce(
            &mut state,
            AppAction::SecureBackupSetupRequested {
                request_id: 2,
                intent: SecureBackupSetupIntent::InitialSetup,
            },
        )
        .is_empty()
    );
    assert_eq!(state, before);
}

#[test]
fn secure_backup_setup_intent_is_private_safe() {
    let debug = format!(
        "{:?}",
        SecureBackupSetupIntent::Reenable { confirmed: true }
    );
    assert!(debug.contains("Reenable"));
    assert!(!debug.contains("secret"));
    assert!(!debug.contains("DEVICE"));
}
