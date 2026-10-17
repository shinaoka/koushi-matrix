use std::time::Duration;

use koushi_state::{AppAction, CurrentDeviceTrustState, SessionAuthenticationMethod, SessionInfo};
use tokio::sync::{mpsc, oneshot};

use crate::account::{
    actor::AccountMessage,
    session_lifecycle::SessionInvalidationReason,
    test_support::{
        acknowledge_next_verified_projection, consume_initial_unknown_trust_projection,
        inspect_session_runtime, login_gated_actor, shutdown_and_ack, test_request_id,
    },
};

async fn promoted_actor_with_blocked_discovery() -> (
    crate::account::actor::AccountActorHandle,
    mpsc::Receiver<Vec<AppAction>>,
    oneshot::Sender<Option<String>>,
) {
    let (handle, mut action_rx) = login_gated_actor().await;
    consume_initial_unknown_trust_projection(&mut action_rx).await;
    let (release, result) = oneshot::channel();
    assert!(
        handle
            .send(AccountMessage::ConfigureAccountManagementDiscovery { result })
            .await
    );
    assert!(
        handle
            .send(AccountMessage::CurrentDeviceTrustChanged {
                generation: 2,
                trust: CurrentDeviceTrustState::Verified,
            })
            .await
    );
    acknowledge_next_verified_projection(&handle, &mut action_rx).await;
    (handle, action_rx, release)
}

#[tokio::test]
async fn promoted_restored_session_starts_active_account_management_discovery() {
    let (handle, mut action_rx) = login_gated_actor().await;
    consume_initial_unknown_trust_projection(&mut action_rx).await;
    assert!(
        handle
            .send(AccountMessage::CurrentDeviceTrustChanged {
                generation: 2,
                trust: CurrentDeviceTrustState::Verified,
            })
            .await
    );
    acknowledge_next_verified_projection(&handle, &mut action_rx).await;

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if matches!(
                action_rx.recv().await.as_deref(),
                Some([AppAction::ActiveSessionAccountManagementUrlResolved { .. }])
            ) {
                break;
            }
        }
    })
    .await
    .expect("promotion must discover an active-session destination without login discovery");
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn trust_quarantine_aborts_active_account_management_discovery() {
    let (handle, mut action_rx, release) = promoted_actor_with_blocked_discovery().await;
    assert!(
        handle
            .send(AccountMessage::CurrentDeviceTrustChanged {
                generation: 2,
                trust: CurrentDeviceTrustState::Unverified,
            })
            .await
    );
    let (generation, transition_id) = loop {
        if let Some(
            [
                AppAction::AuthoritativeDeviceTrustChanged {
                    generation,
                    transition_id,
                    trust: CurrentDeviceTrustState::Unverified,
                },
            ],
        ) = action_rx.recv().await.as_deref()
        {
            break (*generation, *transition_id);
        }
    };
    assert!(
        handle
            .send(AccountMessage::TrustProjectionApplied {
                generation,
                transition_id,
                ready: false,
                locked: false,
            })
            .await
    );
    while inspect_session_runtime(&handle).await != (true, false, false, true) {
        crate::executor::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        release
            .send(Some("https://stale.example/devices".to_owned()))
            .is_err()
    );
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn authentication_lock_aborts_active_account_management_discovery() {
    let (handle, _action_rx, release) = promoted_actor_with_blocked_discovery().await;
    assert!(
        handle
            .send(AccountMessage::SessionInvalidated {
                reason: SessionInvalidationReason::UnknownToken { soft_logout: true },
            })
            .await
    );
    while inspect_session_runtime(&handle).await.1 {
        crate::executor::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        release
            .send(Some("https://stale.example/devices".to_owned()))
            .is_err()
    );
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn logout_aborts_active_account_management_discovery() {
    let (handle, _action_rx, release) = promoted_actor_with_blocked_discovery().await;
    assert!(
        handle
            .send(AccountMessage::Command(
                koushi_protocol::command::AccountCommand::ChangeHomeserver {
                    request_id: test_request_id(),
                },
            ))
            .await
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while !release.is_closed() {
            crate::executor::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("logout must abort discovery");
    let _ = handle.send(AccountMessage::Shutdown).await;
}

#[tokio::test]
async fn shutdown_aborts_active_account_management_discovery() {
    let (handle, _action_rx, release) = promoted_actor_with_blocked_discovery().await;
    shutdown_and_ack(&handle).await;
    assert!(
        release
            .send(Some("https://stale.example/devices".to_owned()))
            .is_err()
    );
}

#[tokio::test]
async fn wrong_session_destination_completion_is_ignored() {
    let (handle, mut action_rx, release) = promoted_actor_with_blocked_discovery().await;
    assert!(
        handle
            .send(AccountMessage::ActiveSessionAccountManagementUrlResolved {
                generation: 2,
                info: SessionInfo {
                    homeserver: "https://other.example".to_owned(),
                    user_id: "@other:example".to_owned(),
                    device_id: "OTHER".to_owned(),
                    authentication_method: SessionAuthenticationMethod::Password,
                },
                url: Some("https://stale.example/devices".to_owned()),
            })
            .await
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), async {
            loop {
                if matches!(
                    action_rx.recv().await.as_deref(),
                    Some([AppAction::ActiveSessionAccountManagementUrlResolved { .. }])
                ) {
                    break;
                }
            }
        })
        .await
        .is_err()
    );
    drop(release);
    let _ = handle.send(AccountMessage::Shutdown).await;
}

async fn deactivate_and_collect(password: Option<&str>) -> AppAction {
    let (handle, mut action_rx) = login_gated_actor().await;
    assert!(
        handle
            .send(AccountMessage::Command(
                koushi_protocol::command::AccountCommand::DeactivateAccount {
                    request_id: koushi_protocol::ids::RequestId {
                        connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
                        sequence: 77,
                    },
                    erase_data: false,
                    password: password.map(koushi_state::AuthSecret::new),
                },
            ))
            .await
    );
    let outcome = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let actions = action_rx.recv().await.expect("actor alive");
            if let Some(action) = actions.into_iter().find(|action| {
                matches!(
                    action,
                    AppAction::AccountManagementSucceeded { .. }
                        | AppAction::AccountManagementFailed { .. }
                        | AppAction::AccountManagementUiaRequired { .. }
                )
            }) {
                break action;
            }
        }
    })
    .await
    .expect("deactivation outcome");
    let _ = handle.send(AccountMessage::Shutdown).await;
    outcome
}

#[tokio::test]
async fn deactivation_with_password_answers_the_uia_challenge_once() {
    assert!(matches!(
        deactivate_and_collect(Some("synthetic-password")).await,
        AppAction::AccountManagementSucceeded {
            request_id: 77,
            operation: koushi_state::AccountManagementOperation::DeactivateAccount,
        }
    ));
}

#[tokio::test]
async fn deactivation_with_rejected_password_fails_as_forbidden() {
    assert!(matches!(
        deactivate_and_collect(Some("not-the-password")).await,
        AppAction::AccountManagementFailed {
            request_id: 77,
            operation: koushi_state::AccountManagementOperation::DeactivateAccount,
            kind: koushi_state::AuthFailureKind::Forbidden,
        }
    ));
}

#[tokio::test]
async fn deactivation_without_password_keeps_the_uia_prompt_path() {
    assert!(matches!(
        deactivate_and_collect(None).await,
        AppAction::AccountManagementUiaRequired {
            request_id: 77,
            flow_id: 77,
            ..
        }
    ));
}
