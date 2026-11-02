//! #1735: drive the real SDK and account Retry against a silent local server.
use std::time::Duration;

use koushi_protocol::command::{AccountCommand, SyncCommand};
use koushi_state::{
    AppAction, AppEffect, AppState, CurrentDeviceTrustState, SecureBackupGateState,
    SyncLifecycleStatus, reduce,
};
use tokio::sync::mpsc;

use crate::account::{
    actor::{AccountActorHandle, AccountMessage},
    test_support::*,
};
#[path = "stalled_server.rs"]
mod stalled_server;
use stalled_server::StalledServer;

async fn stalled_actor() -> (
    StalledServer,
    AccountActorHandle,
    mpsc::Receiver<Vec<AppAction>>,
) {
    let server = StalledServer::start().await;
    let (handle, mut actions) = login_gated_actor_at(server.uri.clone()).await;
    handle
        .send(AccountMessage::ConfigureSecureBackupDeferWait {
            wait: Duration::from_millis(100),
        })
        .await;
    consume_initial_unknown_trust_projection(&mut actions).await;
    handle
        .send(AccountMessage::CurrentDeviceTrustChanged {
            generation: 2,
            trust: CurrentDeviceTrustState::Verified,
        })
        .await;
    acknowledge_next_verified_projection(&handle, &mut actions).await;
    // As in AppActor, the promoted account starts its one normal sync owner.
    handle
        .send(AccountMessage::SyncCommand(SyncCommand::Start {
            request_id: test_request_id(),
        }))
        .await;
    (server, handle, actions)
}

async fn failed_gate(
    handle: &AccountActorHandle,
    actions: &mut mpsc::Receiver<Vec<AppAction>>,
) -> koushi_state::SecureBackupFailureStage {
    handle.send(AccountMessage::InspectSecureBackup).await;
    let stage = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let batch = actions.recv().await.expect("account actions");
            assert!(
                !batch.iter().any(|action| matches!(
                    action,
                    AppAction::SyncStatusChanged {
                        status: SyncLifecycleStatus::Running,
                        ..
                    }
                )),
                "silent server cannot prove connectivity"
            );
            if let Some(detail) = batch.iter().find_map(|action| match action {
                AppAction::SecureBackupGateChanged(SecureBackupGateState::BlockedFailed {
                    failure: koushi_state::SecureBackupGateFailureKind::Timeout,
                    detail: Some(detail),
                }) => Some(detail),
                _ => None,
            }) {
                assert!(
                    detail.retryable,
                    "#860: unproven connectivity must stay retryable"
                );
                break detail.stage;
            }
        }
    })
    .await
    .expect("bounded connectivity deadline must fail retryably");
    assert!(
        !inspect_secure_backup_owners(handle)
            .await
            .has_inspection_task
    );
    stage
}

async fn retry(handle: &AccountActorHandle) {
    assert!(
        handle
            .send(AccountMessage::Command(
                AccountCommand::RetrySecureBackupInspection {
                    request_id: test_request_id()
                }
            ))
            .await
    );
}

#[tokio::test]
async fn retry_replaces_stalled_first_sync_request() {
    let (mut server, handle, mut actions) = stalled_actor().await;
    let _first = server.next_attempt().await;
    assert_eq!(
        failed_gate(&handle, &mut actions).await,
        koushi_state::SecureBackupFailureStage::FirstSync
    );
    retry(&handle).await;
    let result = tokio::time::timeout(Duration::from_secs(3), server.attempts.recv()).await;
    shutdown_and_ack(&handle).await;
    server.shutdown().await;
    assert!(
        matches!(result, Ok(Some(_))),
        "Retry must issue a new sliding-sync request, not wait on the stalled run"
    );
}

#[tokio::test]
async fn repeated_retry_fences_late_response_then_admits_inspection() {
    let (mut server, handle, mut actions) = stalled_actor().await;
    let first = server.next_attempt().await;
    failed_gate(&handle, &mut actions).await;
    retry(&handle).await;
    let second = server.next_attempt().await;
    // A second explicit Retry while still Starting must replace that run too.
    retry(&handle).await;
    let third = server.next_attempt().await;
    first.answer().await;
    second.answer().await;
    // Neither replaced HTTP response can publish Running for the new run.
    let quiet = tokio::time::timeout(Duration::from_millis(200), async {
        loop {
            for action in actions.recv().await.expect("account actions") {
                assert!(
                    !matches!(
                        action,
                        AppAction::SyncStatusChanged {
                            status: SyncLifecycleStatus::Running,
                            ..
                        }
                    ),
                    "late replaced response proved new connectivity"
                );
            }
        }
    })
    .await;
    assert!(quiet.is_err());
    assert!(
        !inspect_secure_backup_owners(&handle)
            .await
            .has_inspection_task
    );
    assert!(server.backup_requests.try_recv().is_err());

    third.answer().await;
    // Run the real reducer's connectivity effect, as AppActor does.
    let mut state = AppState {
        session: koushi_state::SessionState::Ready(koushi_state::SessionInfo {
            homeserver: server.uri.clone(),
            user_id: "@fixture-user:example.invalid".into(),
            device_id: "SYNTHETIC".into(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Password,
        }),
        ..AppState::default()
    };
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let batch = actions.recv().await.expect("account actions");
            let mut running = false;
            for action in batch {
                running |= matches!(
                    action,
                    AppAction::SyncStatusChanged {
                        status: SyncLifecycleStatus::Running,
                        ..
                    }
                );
                for effect in reduce(&mut state, action) {
                    if let AppEffect::SyncConnectivityChanged { proven } = effect {
                        handle
                            .send(AccountMessage::SyncConnectivityChanged { proven })
                            .await;
                    }
                }
            }
            if running {
                break;
            }
        }
        server
            .backup_requests
            .recv()
            .await
            .expect("Running must start the actual Secure Backup inspection");
        loop {
            if actions
                .recv()
                .await
                .expect("account actions")
                .iter()
                .any(|action| {
                    matches!(
                        action,
                        AppAction::SecureBackupGateChanged(SecureBackupGateState::SetupRequired)
                    )
                })
            {
                break;
            }
        }
    })
    .await
    .expect("replacement response must resume inspection and settle the gate");
    shutdown_and_ack(&handle).await;
    server.shutdown().await;
}

#[tokio::test]
async fn retry_after_sign_out_does_not_revive_sync_or_arm_inspection() {
    let (mut server, handle, mut actions) = stalled_actor().await;
    let first = server.next_attempt().await;
    failed_gate(&handle, &mut actions).await;
    handle
        .send(AccountMessage::Command(AccountCommand::Logout {
            request_id: test_request_id(),
        }))
        .await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if actions
                .recv()
                .await
                .expect("account actions")
                .iter()
                .any(|action| matches!(action, AppAction::LogoutFinished))
            {
                break;
            }
        }
    })
    .await
    .expect("logout settles");
    retry(&handle).await;
    let owners = inspect_secure_backup_owners(&handle).await;
    assert!(
        !owners.inspection_pending && !owners.has_defer_deadline && !owners.has_inspection_task
    );
    assert_eq!(inspect_sync_owners(&handle).await, (false, false, false));
    first.answer().await;
    assert!(server.attempts.try_recv().is_err());
    shutdown_and_ack(&handle).await;
    server.shutdown().await;
}

#[tokio::test]
async fn retry_without_session_reports_session_required() {
    let credentials = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let (handle, _actions, mut events) = spawn_actor_with_dirs(credentials.path(), data.path());
    retry(&handle).await;
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(3), events.recv())
            .await
            .expect("Retry failure"),
        Ok(koushi_protocol::event::CoreEvent::OperationFailed {
            failure: koushi_protocol::failure::CoreFailure::SessionRequired,
            ..
        })
    ));
    let owners = inspect_secure_backup_owners(&handle).await;
    assert!(!owners.inspection_pending && !owners.has_defer_deadline);
    shutdown_and_ack(&handle).await;
}

#[tokio::test]
async fn retry_after_account_replacement_uses_only_the_new_session() {
    let (mut old_server, handle, mut actions) = stalled_actor().await;
    let old_response = old_server.next_attempt().await;
    failed_gate(&handle, &mut actions).await;
    let old_owners = inspect_secure_backup_owners(&handle).await;
    let mut new_server = StalledServer::start_named("@other-user:example.invalid").await;
    handle
        .send(AccountMessage::Command(AccountCommand::LoginPassword {
            request_id: test_request_id(),
            request: koushi_state::LoginRequest {
                homeserver: new_server.uri.clone(),
                username: "other-user".into(),
                password: koushi_state::AuthSecret::new("synthetic-password"),
                device_display_name: None,
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let batch = recv_account_action_with_sliding_sync_effects(&handle, &mut actions).await;
            if batch
                .iter()
                .any(|action| matches!(action, AppAction::LoginSucceeded { .. }))
            {
                break;
            }
        }
    })
    .await
    .expect("replacement login");
    consume_initial_unknown_trust_projection(&mut actions).await;
    retry(&handle).await;
    let owners = inspect_secure_backup_owners(&handle).await;
    assert!(owners.trust_generation > old_owners.trust_generation);
    assert!(
        !owners.inspection_pending && !owners.has_defer_deadline && !owners.has_inspection_task
    );
    old_response.answer().await;
    // A stale deadline from the old account cannot clear or fail the new one.
    handle
        .send(AccountMessage::SecureBackupDeferredDeadlineExpired {
            generation: old_owners.trust_generation,
            deadline_serial: old_owners.defer_serial,
        })
        .await;
    handle
        .send(AccountMessage::CurrentDeviceTrustChanged {
            generation: owners.trust_generation,
            trust: CurrentDeviceTrustState::Verified,
        })
        .await;
    acknowledge_next_verified_projection(&handle, &mut actions).await;
    retry(&handle).await;
    let _new_response = new_server.next_attempt().await;
    assert!(old_server.attempts.try_recv().is_err());
    assert_eq!(
        failed_gate(&handle, &mut actions).await,
        koushi_state::SecureBackupFailureStage::FirstSync
    );
    shutdown_and_ack(&handle).await;
    old_server.shutdown().await;
    new_server.shutdown().await;
}

#[tokio::test]
async fn trust_gate_reentry_keeps_the_sessions_prior_connectivity_fact() {
    let (mut server, handle, mut actions) = stalled_actor().await;
    let _first = server.next_attempt().await;
    handle
        .send(AccountMessage::SyncConnectivityChanged { proven: true })
        .await;
    handle
        .send(AccountMessage::CurrentDeviceTrustChanged {
            generation: 2,
            trust: CurrentDeviceTrustState::Unverified,
        })
        .await;
    let (generation, transition_id) = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            for action in actions.recv().await.expect("account actions") {
                if let AppAction::AuthoritativeDeviceTrustChanged {
                    generation,
                    transition_id,
                    trust: CurrentDeviceTrustState::Unverified,
                } = action
                {
                    return (generation, transition_id);
                }
            }
        }
    })
    .await
    .expect("trust demotion");
    handle
        .send(AccountMessage::TrustProjectionApplied {
            generation,
            transition_id,
            ready: false,
            locked: false,
        })
        .await;
    handle
        .send(AccountMessage::CurrentDeviceTrustChanged {
            generation: 2,
            trust: CurrentDeviceTrustState::Verified,
        })
        .await;
    tokio::time::timeout(
        Duration::from_secs(3),
        acknowledge_next_verified_projection(&handle, &mut actions),
    )
    .await
    .expect("re-promotion");
    retry(&handle).await;
    let _replacement = server.next_attempt().await;
    assert_eq!(
        failed_gate(&handle, &mut actions).await,
        koushi_state::SecureBackupFailureStage::InspectionDeadline
    );
    shutdown_and_ack(&handle).await;
    server.shutdown().await;
}

#[tokio::test]
async fn deferred_deadline_after_prior_connectivity_is_not_a_first_sync_failure() {
    let (server, handle, mut actions) = stalled_actor().await;
    handle
        .send(AccountMessage::SyncConnectivityChanged { proven: true })
        .await;
    handle
        .send(AccountMessage::SyncConnectivityChanged { proven: false })
        .await;
    assert_eq!(
        failed_gate(&handle, &mut actions).await,
        koushi_state::SecureBackupFailureStage::InspectionDeadline
    );
    shutdown_and_ack(&handle).await;
    server.shutdown().await;
}
