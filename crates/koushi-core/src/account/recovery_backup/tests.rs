use std::time::Duration;

use koushi_state::{AppAction, AuthFailureKind, VerificationCancelReason};

use tokio::sync::oneshot;

use super::{
    SecureBackupInspectionAdmission, apply_secure_backup_connectivity_edge,
    classify_e2ee_trust_auth_failure, classify_e2ee_trust_error, classify_recovery_error,
    project_bootstrap_cross_signing_result, project_enable_key_backup_result,
    project_reset_identity_auth_required, project_reset_identity_completed,
    project_restore_key_backup_result, recovery_result_is_current,
    secure_backup_inspection_admission, secure_backup_inspection_completion_action,
    secure_backup_monitor_wakeup_is_current, secure_backup_retry_delay,
};
use crate::account::actor::AccountMessage;
use crate::account::test_support::{
    acknowledge_next_verified_projection, inspect_session_runtime, inspect_sync_owners,
    login_gated_actor, shutdown_and_ack, spawn_actor_with_dirs, test_request_id,
};
use crate::account::verification::incoming_verification_request_id;
use koushi_protocol::command::AccountCommand;

use crate::executor;
use koushi_protocol::event::CoreEvent;

use koushi_protocol::failure::CoreFailure;
use koushi_protocol::ids::{AccountKey, RequestId, RuntimeConnectionId};

use tempfile::tempdir;

fn ready_secure_backup_inspection() -> koushi_sdk::MatrixSecureBackupInspection {
    koushi_sdk::MatrixSecureBackupInspection {
        server: koushi_sdk::MatrixSecureBackupServerState::Present,
        local: koushi_sdk::MatrixSecureBackupLocalState::Enabled,
        recovery: koushi_sdk::MatrixSecureBackupRecoveryState::Enabled,
        upload: koushi_sdk::MatrixSecureBackupUploadState::Settled,
        trust: koushi_sdk::MatrixSecureBackupTrustState::Trusted,
        recovery_key_delivery_pending: false,
    }
}

#[test]
fn secure_backup_completion_rejects_stale_or_unpromoted_sessions() {
    for (current_generation, promoted, completed_generation) in [(5, true, 4), (5, false, 5)] {
        assert!(
            secure_backup_inspection_completion_action(
                current_generation,
                promoted,
                false,
                completed_generation,
                Ok(ready_secure_backup_inspection()),
            )
            .is_none()
        );
    }

    assert!(matches!(
        secure_backup_inspection_completion_action(
            5,
            true,
            false,
            5,
            Ok(ready_secure_backup_inspection()),
        ),
        Some(AppAction::SecureBackupGateChanged(
            koushi_state::SecureBackupGateState::Ready
        ))
    ));
    assert!(matches!(
        secure_backup_inspection_completion_action(
            5,
            true,
            true,
            5,
            Err(koushi_state::SecureBackupGateFailureKind::Timeout),
        ),
        Some(AppAction::SecureBackupGateChanged(
            koushi_state::SecureBackupGateState::DegradedRetrying {
                failure: koushi_state::SecureBackupGateFailureKind::Timeout
            }
        ))
    ));
    assert!(matches!(
        secure_backup_inspection_completion_action(
            5,
            true,
            false,
            5,
            Err(koushi_state::SecureBackupGateFailureKind::Timeout),
        ),
        Some(AppAction::SecureBackupGateChanged(
            koushi_state::SecureBackupGateState::BlockedFailed {
                failure: koushi_state::SecureBackupGateFailureKind::Timeout
            }
        ))
    ));
}

#[test]
fn secure_backup_monitor_rejects_stale_generation_serial_and_locked_session_wakeups() {
    assert!(secure_backup_monitor_wakeup_is_current(7, 11, true, 7, 11));
    assert!(!secure_backup_monitor_wakeup_is_current(7, 11, true, 6, 11));
    assert!(!secure_backup_monitor_wakeup_is_current(7, 11, true, 7, 10));
    assert!(!secure_backup_monitor_wakeup_is_current(
        7, 11, false, 7, 11
    ));
}

#[test]
fn secure_backup_inspection_is_deferred_until_sync_connectivity_is_proven() {
    assert_eq!(
        secure_backup_inspection_admission(false, false),
        SecureBackupInspectionAdmission::Defer
    );
    assert_eq!(
        secure_backup_inspection_admission(false, true),
        SecureBackupInspectionAdmission::Coalesce
    );
    assert_eq!(
        secure_backup_inspection_admission(true, false),
        SecureBackupInspectionAdmission::Start
    );
}

#[test]
fn secure_backup_backoff_resets_only_once_across_a_flapping_recovery_epoch() {
    let mut attempt = 4;
    let mut epoch = false;
    let mut reset_consumed = false;

    assert!(!apply_secure_backup_connectivity_edge(
        false,
        &mut attempt,
        &mut epoch,
        &mut reset_consumed,
    ));
    assert!(apply_secure_backup_connectivity_edge(
        true,
        &mut attempt,
        &mut epoch,
        &mut reset_consumed,
    ));
    assert_eq!(attempt, 0);
    assert!(epoch);
    assert!(reset_consumed);

    attempt = 3;
    assert!(!apply_secure_backup_connectivity_edge(
        false,
        &mut attempt,
        &mut epoch,
        &mut reset_consumed,
    ));
    assert!(!apply_secure_backup_connectivity_edge(
        true,
        &mut attempt,
        &mut epoch,
        &mut reset_consumed,
    ));
    assert_eq!(attempt, 3, "a flap in the same epoch must preserve backoff");
}

#[test]
fn secure_backup_retry_delay_is_exponential_jittered_and_capped() {
    let delays = (0..10)
        .map(|attempt| secure_backup_retry_delay(attempt, 17))
        .collect::<Vec<_>>();

    assert!(delays.windows(2).all(|pair| pair[0] <= pair[1]));
    assert!(delays[0] >= Duration::from_secs(5));
    assert!(delays[0] <= Duration::from_secs(6));
    assert!(
        delays
            .iter()
            .all(|delay| *delay <= Duration::from_secs(300))
    );
    assert_eq!(delays.last(), Some(&Duration::from_secs(300)));
    assert_ne!(
        secure_backup_retry_delay(2, 17),
        secure_backup_retry_delay(2, 18),
        "monitor serial must contribute bounded jitter"
    );
}

#[test]
fn recovery_result_requires_current_generation_flow_request_and_session() {
    let current = test_request_id();
    let other = RequestId {
        connection_id: current.connection_id,
        sequence: current.sequence + 1,
    };

    assert!(recovery_result_is_current(
        4, 4, 9, 9, current, current, true
    ));
    assert!(!recovery_result_is_current(
        3, 4, 9, 9, current, current, true
    ));
    assert!(!recovery_result_is_current(
        4, 4, 8, 9, current, current, true
    ));
    assert!(!recovery_result_is_current(
        4, 4, 9, 9, other, current, true
    ));
    assert!(!recovery_result_is_current(
        4, 4, 9, 9, current, current, false
    ));
}

#[tokio::test]
async fn recovery_proof_success_waits_for_verified_trust_before_promotion() {
    let (handle, mut action_rx) = login_gated_actor().await;
    let flow_id = 81;
    let request_id = incoming_verification_request_id(flow_id);
    handle
        .send(AccountMessage::ConfigureSyntheticRecoveryTask {
            flow_id,
            pending: false,
        })
        .await;
    let (download_release, download) = oneshot::channel();
    handle
        .send(AccountMessage::ConfigureRecoveryDownload {
            completion: download,
        })
        .await;
    download_release
        .send(true)
        .expect("release recovery download");
    handle
        .send(AccountMessage::RecoveryFinished {
            generation: 2,
            flow_id,
            request_id,
            result: Ok(()),
        })
        .await;
    assert_eq!(
        inspect_session_runtime(&handle).await,
        (true, false, false, true),
        "accepted recovery proof must not promote until SDK current-device trust is Verified"
    );
    assert!(matches!(
        action_rx.recv().await.as_deref(),
        Some([AppAction::AuthoritativeDeviceTrustChanged {
            generation: 2,
            trust: koushi_state::CurrentDeviceTrustState::Unknown,
            ..
        }])
    ));

    handle
        .send(AccountMessage::CurrentDeviceTrustChanged {
            generation: 2,
            trust: koushi_state::CurrentDeviceTrustState::Verified,
        })
        .await;
    acknowledge_next_verified_projection(&handle, &mut action_rx).await;
    assert_eq!(
        inspect_session_runtime(&handle).await,
        (true, true, true, true),
        "Verified trust must complete promotion after recovery proof settlement"
    );
    loop {
        let actions = executor::timeout(Duration::from_secs(1), action_rx.recv())
            .await
            .expect("restore-key-backup request after recovery verification")
            .expect("account actions");
        if matches!(
            actions.as_slice(),
            [AppAction::RestoreKeyBackupRequested { .. }]
        ) {
            break;
        }
    }
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn recovery_submission_pauses_and_failure_resumes_the_single_provisional_owner() {
    let (handle, mut action_rx) = login_gated_actor().await;
    assert_eq!(inspect_sync_owners(&handle).await, (true, false, false));

    let (completion_tx, completion) = oneshot::channel();
    handle
        .send(AccountMessage::ConfigureRecoveryResult { completion })
        .await;
    handle
        .send(AccountMessage::Command(AccountCommand::SubmitRecovery {
            request_id: RequestId {
                connection_id: RuntimeConnectionId(1),
                sequence: 901,
            },
            request: koushi_state::RecoveryRequest {
                secret: koushi_state::AuthSecret::new("synthetic-recovery-secret"),
            },
        }))
        .await;
    assert_eq!(
        inspect_sync_owners(&handle).await,
        (false, false, false),
        "recovery submission must stop and join provisional encryption sync"
    );

    completion_tx
        .send(Err(koushi_sdk::E2eeRecoveryError::Sdk(
            "synthetic failure".to_owned(),
        )))
        .expect("release recovery result");
    loop {
        let actions = action_rx.recv().await.expect("recovery failure action");
        if matches!(actions.as_slice(), [AppAction::E2eeRecoveryFailed { .. }]) {
            break;
        }
    }
    assert_eq!(
        inspect_sync_owners(&handle).await,
        (true, false, false),
        "failed recovery must resume exactly one provisional encryption owner"
    );
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn recovery_trust_settlement_timeout_returns_to_recovery_failure() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
        .records
        .len();
    let (handle, mut action_rx) = login_gated_actor().await;
    let flow_id = 80;
    let request_id = incoming_verification_request_id(flow_id);
    handle
        .send(AccountMessage::ConfigureSyntheticRecoveryTask {
            flow_id,
            pending: false,
        })
        .await;
    handle
        .send(AccountMessage::RecoveryFinished {
            generation: 2,
            flow_id,
            request_id,
            result: Ok(()),
        })
        .await;
    assert!(matches!(
        action_rx.recv().await.as_deref(),
        Some([AppAction::AuthoritativeDeviceTrustChanged {
            generation: 2,
            trust: koushi_state::CurrentDeviceTrustState::Unknown,
            ..
        }])
    ));
    handle
        .send(AccountMessage::RecoveryTrustSettlementTimedOut {
            generation: 2,
            flow_id,
            request_id,
            trust: koushi_state::CurrentDeviceTrustState::Unknown,
        })
        .await;
    loop {
        let actions = executor::timeout(Duration::from_secs(1), action_rx.recv())
            .await
            .expect("recovery timeout failure projection")
            .expect("account actions");
        if matches!(actions.as_slice(), [AppAction::E2eeRecoveryFailed { .. }]) {
            break;
        }
    }
    assert_eq!(
        inspect_session_runtime(&handle).await,
        (true, false, false, true),
        "recovery trust timeout must not promote the session or leave normal runtime running"
    );
    assert!(
        koushi_diagnostics::test_support::detail_snapshot().records[diagnostic_start..]
            .iter()
            .any(|record| {
                record.event.source == "core.recovery_verification"
                    && record.event.stage == "trust_settlement_timeout_projected"
                    && record.event.fields.iter().any(|field| {
                        field.key == "failure_kind"
                            && field.value == koushi_diagnostics::DiagnosticValue::Token("timeout")
                    })
            }),
        "timeout projection must be visible in diagnostics"
    );
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn invalid_recovery_terminal_stays_gated_without_normal_runtime() {
    let (handle, mut action_rx) = login_gated_actor().await;
    let flow_id = 82;
    let request_id = incoming_verification_request_id(flow_id);
    handle
        .send(AccountMessage::ConfigureSyntheticRecoveryTask {
            flow_id,
            pending: false,
        })
        .await;
    handle
        .send(AccountMessage::RecoveryFinished {
            generation: 2,
            flow_id,
            request_id,
            result: Err(koushi_sdk::E2eeRecoveryError::Sdk(
                "invalid fixture secret".to_owned(),
            )),
        })
        .await;
    while !matches!(
        action_rx.recv().await.as_deref(),
        Some([AppAction::E2eeRecoveryFailed { .. }])
    ) {}
    assert_eq!(
        inspect_session_runtime(&handle).await,
        (true, false, false, true)
    );
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn recovery_cancel_is_processed_while_task_is_pending_and_stale_result_is_ignored() {
    let cred_dir = tempdir().expect("tempdir");
    let data_dir = tempdir().expect("tempdir");
    let (handle, mut action_rx, _event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    let flow_id = 71;
    assert!(
        handle
            .send(AccountMessage::ConfigureSyntheticRecoveryTask {
                flow_id,
                pending: true
            })
            .await
    );
    assert!(
        handle
            .send(AccountMessage::Command(
                AccountCommand::CancelVerification {
                    request_id: test_request_id(),
                    flow_id,
                    reason: VerificationCancelReason::User,
                },
            ))
            .await
    );
    let actions = tokio::time::timeout(std::time::Duration::from_secs(1), action_rx.recv())
        .await
        .expect("cancel projection timeout")
        .expect("cancel projection");
    assert_eq!(
        actions,
        vec![AppAction::VerificationGateAttemptFailed {
            flow_id,
            kind: koushi_state::VerificationGateFailureKind::Cancelled,
        }]
    );
    let (response, pending) = oneshot::channel();
    assert!(
        handle
            .send(AccountMessage::InspectRecoveryTask { response })
            .await
    );
    assert!(!pending.await.expect("recovery task inspection"));

    assert!(
        handle
            .send(AccountMessage::RecoveryFinished {
                generation: 0,
                flow_id,
                request_id: incoming_verification_request_id(flow_id),
                result: Ok(()),
            })
            .await
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), action_rx.recv())
            .await
            .is_err(),
        "stale recovery result must not project a second terminal"
    );
    shutdown_and_ack(&handle).await;
}

/// Verify classify_recovery_error maps SDK error text to coarse kinds
/// without leaking the raw message in any public type.
#[test]
fn recovery_error_classification_invalid_key() {
    let err = koushi_sdk::E2eeRecoveryError::Sdk("invalid recovery key".to_owned());
    assert_eq!(
        classify_recovery_error(&err),
        koushi_protocol::failure::RecoveryFailureKind::InvalidRecoveryKey,
        "SDK 'invalid' text must map to InvalidRecoveryKey"
    );
}

#[test]
fn recovery_error_classification_network() {
    let err = koushi_sdk::E2eeRecoveryError::Runtime("runtime error".to_owned());
    assert_eq!(
        classify_recovery_error(&err),
        koushi_protocol::failure::RecoveryFailureKind::Network,
        "Runtime error must map to Network"
    );
}

#[test]
fn recovery_error_classification_server_fallback() {
    let err = koushi_sdk::E2eeRecoveryError::Sdk("unexpected server error".to_owned());
    assert_eq!(
        classify_recovery_error(&err),
        koushi_protocol::failure::RecoveryFailureKind::Server,
        "Unknown SDK error must map to Server (conservative)"
    );
}

/// Verify that RecoveryRequest's Debug output does not leak the secret.
#[test]
fn recovery_request_debug_redacts_secret() {
    use koushi_state::AuthSecret;
    let req = koushi_state::RecoveryRequest {
        secret: AuthSecret::new("super-secret-recovery-key"),
    };
    let debug = format!("{req:?}");
    assert!(
        !debug.contains("super-secret-recovery-key"),
        "RecoveryRequest Debug must redact the secret: {debug}"
    );
}

/// Network-free: SubmitRecovery without an active session must emit
/// SessionRequired, not panic or crash.
#[tokio::test]
async fn submit_recovery_without_session_emits_session_required() {
    use koushi_state::AuthSecret;
    let cred_dir = tempdir().expect("tempdir");
    let data_dir = tempdir().expect("tempdir");
    let (handle, _action_rx, mut event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());

    let request_id = test_request_id();
    assert!(
        handle
            .send(AccountMessage::Command(AccountCommand::SubmitRecovery {
                request_id,
                request: koushi_state::RecoveryRequest {
                    secret: AuthSecret::new("some-key"),
                },
            }))
            .await
    );

    match event_rx.recv().await.expect("event") {
        CoreEvent::OperationFailed {
            request_id: ev_id,
            failure,
        } => {
            assert_eq!(ev_id, request_id);
            assert_eq!(failure, CoreFailure::SessionRequired);
        }
        other => panic!("expected OperationFailed(SessionRequired), got {other:?}"),
    }
}

/// Network-free: E2EE trust commands require an active store-backed
/// session. Runtime may allow recovery commands while AppState is
/// NeedsRecovery; without an actor session they must still fail as
/// SessionRequired, not as local-encryption unavailable.
#[tokio::test]
async fn e2ee_trust_commands_without_session_emit_session_required() {
    let cred_dir = tempdir().expect("tempdir");
    let data_dir = tempdir().expect("tempdir");
    let (handle, mut action_rx, mut event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());

    let request_id = test_request_id();
    assert!(
        handle
            .send(AccountMessage::Command(
                AccountCommand::BootstrapCrossSigning {
                    request_id,
                    auth: None,
                }
            ))
            .await
    );

    let actions = action_rx.recv().await.expect("trust failure action batch");
    assert_eq!(
        actions,
        vec![AppAction::BootstrapCrossSigningFailed {
            request_id: request_id.sequence,
            kind: koushi_state::TrustOperationFailureKind::Sdk,
        }]
    );

    match event_rx.recv().await.expect("event") {
        CoreEvent::OperationFailed {
            request_id: ev_id,
            failure,
        } => {
            assert_eq!(ev_id, request_id);
            assert_eq!(failure, CoreFailure::SessionRequired);
        }
        other => panic!("expected OperationFailed(SessionRequired), got {other:?}"),
    }
}

#[test]
fn e2ee_trust_error_classification_is_kind_only() {
    assert_eq!(
        classify_e2ee_trust_error(&koushi_sdk::E2eeTrustError::NoOlmMachine),
        koushi_state::TrustOperationFailureKind::Sdk
    );
    assert_eq!(
        classify_e2ee_trust_error(&koushi_sdk::E2eeTrustError::Sdk(
            "timeout while talking to @alice:example.test".to_owned()
        )),
        koushi_state::TrustOperationFailureKind::Timeout
    );
    assert_eq!(
        classify_e2ee_trust_error(&koushi_sdk::E2eeTrustError::Sdk("M_FORBIDDEN".to_owned())),
        koushi_state::TrustOperationFailureKind::Forbidden
    );
    let invalid_passphrase = koushi_sdk::E2eeTrustError::Sdk("invalid passphrase MAC".to_owned());
    assert_eq!(
        classify_e2ee_trust_error(&invalid_passphrase),
        koushi_state::TrustOperationFailureKind::InvalidPassphrase
    );
    assert_eq!(
        classify_e2ee_trust_auth_failure(&invalid_passphrase),
        AuthFailureKind::Sdk
    );
}

#[test]
fn e2ee_trust_sdk_results_project_actions_and_typed_events() {
    let request_id = test_request_id();
    let account_key = AccountKey("@alice:example.test".to_owned());

    let (actions, events) = project_bootstrap_cross_signing_result(
        request_id,
        account_key.clone(),
        Ok(koushi_state::CrossSigningStatus::Trusted),
    );
    assert_eq!(
        actions,
        vec![AppAction::CrossSigningStatusChanged {
            status: koushi_state::CrossSigningStatus::Trusted,
        }]
    );
    assert!(matches!(
        events.as_slice(),
        [CoreEvent::E2eeTrust(
            koushi_protocol::event::E2eeTrustEvent::CrossSigningChanged {
                status: koushi_state::CrossSigningStatus::Trusted,
                ..
            }
        )]
    ));

    let (actions, events) = project_bootstrap_cross_signing_result(
        request_id,
        account_key,
        Err(koushi_sdk::E2eeTrustError::Sdk(
            "timeout from @alice:example.test".to_owned(),
        )),
    );
    assert_eq!(
        actions,
        vec![AppAction::BootstrapCrossSigningFailed {
            request_id: request_id.sequence,
            kind: koushi_state::TrustOperationFailureKind::Timeout,
        }]
    );
    assert!(matches!(
        events.as_slice(),
        [CoreEvent::E2eeTrust(
            koushi_protocol::event::E2eeTrustEvent::CrossSigningChanged {
                status: koushi_state::CrossSigningStatus::Failed {
                    kind: koushi_state::TrustOperationFailureKind::Timeout,
                    ..
                },
                ..
            }
        )]
    ));
    let debug = format!("{events:?}");
    assert!(!debug.contains("@alice:example.test"));
    assert!(!debug.contains("timeout from"));

    let (actions, events) = project_enable_key_backup_result(
        request_id,
        AccountKey("@alice:example.test".to_owned()),
        Ok(koushi_state::KeyBackupStatus::Enabled {
            version: "available".to_owned(),
        }),
    );
    assert_eq!(
        actions,
        vec![AppAction::KeyBackupEnabled {
            request_id: request_id.sequence,
            version: "available".to_owned(),
        }]
    );
    assert!(matches!(
        events.as_slice(),
        [CoreEvent::E2eeTrust(
            koushi_protocol::event::E2eeTrustEvent::KeyBackupChanged {
                status: koushi_state::KeyBackupStatus::Enabled { .. },
                ..
            }
        )]
    ));

    let (actions, events) = project_restore_key_backup_result(
        request_id,
        AccountKey("@alice:example.test".to_owned()),
        Ok(koushi_sdk::KeyBackupRestoreSummary {
            scope: koushi_sdk::KeyBackupRestoreScope::JoinedRooms,
            version: Some("available".to_owned()),
            restored_rooms: 2,
            total_rooms: Some(3),
        }),
    );
    assert_eq!(
        actions,
        vec![
            AppAction::KeyBackupRestoreProgress {
                request_id: request_id.sequence,
                restored_rooms: 2,
                total_rooms: Some(3),
            },
            AppAction::KeyBackupRestored {
                request_id: request_id.sequence,
                version: Some("available".to_owned()),
            },
        ]
    );
    assert!(matches!(
        events.as_slice(),
        [
            CoreEvent::E2eeTrust(koushi_protocol::event::E2eeTrustEvent::KeyBackupChanged {
                status: koushi_state::KeyBackupStatus::Restoring {
                    restored_rooms: 2,
                    total_rooms: Some(3),
                    ..
                },
                ..
            }),
            CoreEvent::E2eeTrust(koushi_protocol::event::E2eeTrustEvent::KeyBackupChanged {
                status: koushi_state::KeyBackupStatus::Enabled { .. },
                ..
            })
        ]
    ));
}

#[test]
fn identity_reset_sdk_results_project_actions_and_typed_events() {
    let request_id = test_request_id();
    let account_key = AccountKey("@alice:example.test".to_owned());

    let (actions, events) = project_reset_identity_completed(request_id, account_key.clone());
    assert_eq!(
        actions,
        vec![AppAction::ResetIdentityCompleted {
            request_id: request_id.sequence,
        }]
    );
    assert!(matches!(
        events.as_slice(),
        [CoreEvent::E2eeTrust(
            koushi_protocol::event::E2eeTrustEvent::IdentityResetChanged {
                state: koushi_state::IdentityResetState::Idle,
                ..
            }
        )]
    ));

    let (actions, events) = project_reset_identity_auth_required(
        request_id,
        account_key,
        koushi_state::IdentityResetAuthType::Uiaa,
    );
    assert_eq!(
        actions,
        vec![AppAction::ResetIdentityAuthRequired {
            request_id: request_id.sequence,
            auth_type: koushi_state::IdentityResetAuthType::Uiaa,
        }]
    );
    assert!(matches!(
        events.as_slice(),
        [CoreEvent::E2eeTrust(
            koushi_protocol::event::E2eeTrustEvent::IdentityResetChanged {
                state: koushi_state::IdentityResetState::AwaitingAuth {
                    auth_type: koushi_state::IdentityResetAuthType::Uiaa,
                    ..
                },
                ..
            }
        )]
    ));

    let debug = format!("{events:?}");
    assert!(!debug.contains("@alice:example.test"));
}
