use super::{account_command_projected_action, secure_backup_setup_projection_failure};
use crate::{CoreFailure, RequestId, RuntimeConnectionId};
use koushi_protocol::{AccountCommand, SecureBackupSetupRequest};
use koushi_state::{
    AppAction, AppState, SecureBackupGateState, SecureBackupSetupIntent, SecureBackupSetupState,
    SessionInfo, SessionState,
};

fn request(intent: SecureBackupSetupIntent) -> AccountCommand {
    AccountCommand::BootstrapSecureBackup {
        request_id: RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: 7,
        },
        request: SecureBackupSetupRequest {
            passphrase: None,
            recovery_key_destination_requested: true,
            intent,
        },
    }
}

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
fn projected_secure_backup_action_carries_the_closed_intent() {
    let command = request(SecureBackupSetupIntent::Reenable { confirmed: true });
    assert_eq!(
        account_command_projected_action(&command),
        Some(AppAction::SecureBackupSetupRequested {
            request_id: 7,
            intent: SecureBackupSetupIntent::Reenable { confirmed: true },
        })
    );
}

#[test]
fn secure_backup_projection_gate_returns_typed_private_safe_failures() {
    let mut state = ready_state(SecureBackupGateState::ExplicitlyDisabledRequiresSetup);
    assert_eq!(
        secure_backup_setup_projection_failure(
            &state,
            &request(SecureBackupSetupIntent::Reenable { confirmed: false }),
        ),
        Some(CoreFailure::SecureBackupSetupConfirmationRequired)
    );
    assert_eq!(
        secure_backup_setup_projection_failure(
            &state,
            &request(SecureBackupSetupIntent::InitialSetup),
        ),
        Some(CoreFailure::SecureBackupSetupFailedNoOp)
    );

    state.secure_backup_gate = SecureBackupGateState::SetupRequired;
    assert_eq!(
        secure_backup_setup_projection_failure(
            &state,
            &request(SecureBackupSetupIntent::Reenable { confirmed: true }),
        ),
        Some(CoreFailure::SecureBackupSetupFailedNoOp)
    );

    state.e2ee_trust.key_management.secure_backup_setup =
        SecureBackupSetupState::SettingUp { request_id: 3 };
    assert_eq!(
        secure_backup_setup_projection_failure(
            &state,
            &request(SecureBackupSetupIntent::InitialSetup),
        ),
        Some(CoreFailure::SecureBackupSetupFailedNoOp)
    );
}

#[test]
fn recovery_key_reset_requires_explicit_confirmation_before_routing() {
    // #927: a lost reveal is never re-exported; replacing the key needs the
    // confirmed reset intent because the previous key stops working.
    let state = ready_state(SecureBackupGateState::RecoveryKeyDeliveryRequired);
    assert_eq!(
        secure_backup_setup_projection_failure(
            &state,
            &request(SecureBackupSetupIntent::ResetRecoveryKey { confirmed: false }),
        ),
        Some(CoreFailure::SecureBackupSetupConfirmationRequired)
    );
    assert_eq!(
        secure_backup_setup_projection_failure(
            &state,
            &request(SecureBackupSetupIntent::InitialSetup),
        ),
        Some(CoreFailure::SecureBackupSetupFailedNoOp)
    );
    let mut projected = state.clone();
    let effects = koushi_state::reduce(
        &mut projected,
        account_command_projected_action(&request(SecureBackupSetupIntent::ResetRecoveryKey {
            confirmed: true,
        }))
        .expect("projected action"),
    );
    assert!(!effects.is_empty());
    assert!(matches!(
        projected.e2ee_trust.key_management.secure_backup_setup,
        SecureBackupSetupState::SettingUp { request_id: 7 }
    ));
}

fn request_id(sequence: u64) -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(1),
        sequence,
    }
}

#[test]
fn secure_backup_setup_and_passphrase_change_never_require_a_destination() {
    // #927: the key is revealed on screen, so neither setup nor passphrase
    // change consumes a native destination; only the optional save does.
    assert_eq!(
        crate::command_policy::native_artifact_for_account_command(&request(
            SecureBackupSetupIntent::InitialSetup
        )),
        None
    );
    assert_eq!(
        crate::command_policy::native_artifact_for_account_command(
            &AccountCommand::ChangeSecureBackupPassphrase {
                request_id: request_id(8),
                request: koushi_protocol::SecureBackupPassphraseChangeRequest {
                    old_secret: koushi_state::AuthSecret::new("old-synthetic-phrase"),
                    new_passphrase: koushi_state::AuthSecret::new("new-synthetic-phrase"),
                },
            }
        ),
        None
    );
    assert_eq!(
        crate::command_policy::native_artifact_for_account_command(
            &AccountCommand::SaveSecureBackupRecoveryKey {
                request_id: request_id(9),
                reveal_request_id: 7,
            }
        ),
        Some((
            request_id(9),
            crate::native_artifact::NativeArtifactKind::RecoveryKeyDestination
        ))
    );
}

#[test]
fn saved_confirmation_projects_the_reveal_exit_and_rejects_stale_reveals() {
    let command = AccountCommand::ConfirmSecureBackupRecoveryKeySaved {
        request_id: request_id(10),
        reveal_request_id: 7,
    };
    assert_eq!(
        account_command_projected_action(&command),
        Some(AppAction::SecureBackupRecoveryKeyConfirmed {
            reveal_request_id: 7,
        })
    );
    assert_eq!(
        secure_backup_setup_projection_failure(
            &ready_state(SecureBackupGateState::RecoveryKeyDeliveryRequired),
            &command,
        ),
        Some(CoreFailure::SecureBackupSetupFailedNoOp)
    );
    // Saving is validated by the actor against its held key and is never a
    // reducer transition by itself.
    assert_eq!(
        account_command_projected_action(&AccountCommand::SaveSecureBackupRecoveryKey {
            request_id: request_id(11),
            reveal_request_id: 7,
        }),
        None
    );
}

#[test]
fn setup_is_rejected_while_a_revealed_key_awaits_confirmation() {
    let mut state = ready_state(SecureBackupGateState::RecoveryKeyDeliveryRequired);
    state.e2ee_trust.key_management.secure_backup_setup =
        SecureBackupSetupState::RecoveryKeyReady {
            request_id: 3,
            recovery_key: koushi_state::RecoveryKeyMaterial::new("synthetic-admission-key"),
            delivery: koushi_state::RecoveryKeyDeliveryState::NotWritten,
            confirmation_failed: false,
        };
    let before = state.clone();
    let effects = koushi_state::reduce(
        &mut state,
        AppAction::SecureBackupSetupRequested {
            request_id: 7,
            intent: SecureBackupSetupIntent::InitialSetup,
        },
    );
    assert!(effects.is_empty());
    assert_eq!(state, before);

    // A passphrase change would rotate the key while the revealed one is
    // still unconfirmed: the projection rejects it with a typed no-op.
    let change = AccountCommand::ChangeSecureBackupPassphrase {
        request_id: request_id(12),
        request: koushi_protocol::SecureBackupPassphraseChangeRequest {
            old_secret: koushi_state::AuthSecret::new("old-synthetic-phrase"),
            new_passphrase: koushi_state::AuthSecret::new("new-synthetic-phrase"),
        },
    };
    let effects = koushi_state::reduce(
        &mut state,
        account_command_projected_action(&change).expect("projected action"),
    );
    assert!(effects.is_empty());
    assert_eq!(state, before);
    assert_eq!(
        secure_backup_setup_projection_failure(&state, &change),
        Some(CoreFailure::SecureBackupSetupFailedNoOp)
    );
    assert_eq!(
        secure_backup_setup_projection_failure(&AppState::default(), &change),
        None
    );
}

fn bootstrap_gate_session(awaiting_confirmation: bool) -> SessionState {
    let info = SessionInfo {
        homeserver: "https://server.example.invalid".to_owned(),
        user_id: "@alice:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    };
    let gate = koushi_state::VerificationGateState {
        methods: vec![koushi_state::VerificationMethodCapability::Bootstrap],
        account_kind: koushi_state::VerificationAccountKind::NewIdentity,
        failure: None,
    };
    if awaiting_confirmation {
        SessionState::AwaitingBootstrapConfirmation {
            info,
            gate,
            flow_id: 41,
        }
    } else {
        SessionState::AwaitingVerification { info, gate }
    }
}

#[test]
fn session_bootstrap_never_requires_a_destination_and_its_reveal_admits_the_optional_save() {
    // #1049: the identity bootstrap reveals the key like Secure Backup setup;
    // only the optional save consumes a native destination.
    let bootstrap = AccountCommand::StartSessionBootstrap {
        request_id: request_id(10),
        flow_id: 10,
        auth: None,
        passphrase: Some(koushi_state::AuthSecret::new("synthetic-phrase")),
    };
    assert_eq!(
        crate::command_policy::native_artifact_for_account_command(&bootstrap),
        None
    );
    let save = koushi_protocol::CoreCommand::Account(AccountCommand::SaveSecureBackupRecoveryKey {
        request_id: request_id(11),
        reveal_request_id: 41,
    });
    assert!(super::is_verification_gate_command(
        &save,
        &bootstrap_gate_session(true)
    ));
    assert!(!super::is_verification_gate_command(
        &save,
        &bootstrap_gate_session(false)
    ));
}

#[test]
fn session_bootstrap_confirmation_is_settled_by_the_actor_not_projected() {
    // The actor clears the persisted delivery marker first; only then may
    // the reveal drop the key and the gate re-check trust.
    assert_eq!(
        account_command_projected_action(&AccountCommand::ConfirmSessionBootstrapSaved {
            request_id: request_id(12),
            flow_id: 41,
        }),
        None
    );
}
