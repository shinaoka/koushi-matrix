//! AccountActor production-path tests for contact security details (#1024)
//! against the matrix-sdk crypto mock homeserver.

use koushi_protocol::{
    command::{AccountCommand, ContactSecurityRequest},
    ids::{RequestId, RuntimeConnectionId},
};
use koushi_state::{
    AppAction, ContactDevicesStatus, ContactIdentityVerification, ContactSecurityFailureKind,
    ContactSecuritySummary, SessionInfo, VerificationCancelReason,
};
use matrix_sdk::{
    Client,
    ruma::{DeviceId, UserId, device_id, user_id},
    test_utils::mocks::MatrixMockServer,
};
use serde_json::json;
use tempfile::tempdir;
use tokio::sync::oneshot;
use tokio::time::{Duration, timeout};
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{method, path_regex},
};

use super::{
    actor::{AccountActorHandle, AccountMessage},
    test_support::{shutdown_and_ack, spawn_actor_with_dirs},
};

async fn crypto_client(
    server: &MatrixMockServer,
    user_id: &UserId,
    device_id: &DeviceId,
) -> Client {
    let client = server
        .client_builder_for_crypto_end_to_end(user_id, device_id)
        .build()
        .await;
    server.mock_sync().ok_and_run(&client, |_| {}).await;
    client
}

async fn cross_signed_client(
    server: &MatrixMockServer,
    user_id: &UserId,
    device_id: &DeviceId,
) -> Client {
    let client = crypto_client(server, user_id, device_id).await;
    client
        .encryption()
        .bootstrap_cross_signing(None)
        .await
        .expect("cross-signing bootstrap");
    client
}

async fn install_session(server: &MatrixMockServer, handle: &AccountActorHandle, client: &Client) {
    let session = koushi_sdk::MatrixClientSession::from_client_for_testing(
        client.clone(),
        SessionInfo {
            homeserver: server.uri(),
            user_id: client.user_id().expect("user id").to_string(),
            device_id: client.device_id().expect("device id").to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Password,
        },
    );
    assert!(
        handle
            .install_residency_test_session(std::sync::Arc::new(session))
            .await
    );
}

fn request(sequence: u64) -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(3),
        sequence,
    }
}

async fn send(handle: &AccountActorHandle, request_id: RequestId, request: ContactSecurityRequest) {
    assert!(
        handle
            .send(AccountMessage::Command(AccountCommand::ContactSecurity {
                request_id,
                request,
            }))
            .await
    );
}

async fn load(handle: &AccountActorHandle, sequence: u64, user_id: &UserId) {
    send(
        handle,
        request(sequence),
        ContactSecurityRequest::Load {
            user_id: user_id.to_string(),
        },
    )
    .await;
}

async fn wait_for_keys_query_after(server: &MatrixMockServer, baseline: usize) {
    timeout(Duration::from_secs(3), async {
        loop {
            if server.received_requests().await.is_some_and(|requests| {
                requests.len() > baseline
                    && requests
                        .iter()
                        .skip(baseline)
                        .any(|request| request.url.path().ends_with("/keys/query"))
            }) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("contact security key query started");
}

async fn delay_next_keys_query(server: &MatrixMockServer) {
    Mock::given(method("POST"))
        .and(path_regex(r"^/_matrix/client/.*/keys/query$"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(700))
                .set_body_json(json!({ "device_keys": {}, "failures": {} })),
        )
        .with_priority(1)
        .up_to_n_times(1)
        .mount(server.server())
        .await;
}

async fn actor_responds_during_contact_security_request(handle: &AccountActorHandle) -> bool {
    let (response, response_rx) = oneshot::channel();
    if !handle
        .send(AccountMessage::InspectSessionRuntime { response })
        .await
    {
        return false;
    }
    matches!(
        timeout(Duration::from_millis(300), response_rx).await,
        Ok(Ok((true, _, _, _)))
    )
}

async fn next_contact_action(
    action_rx: &mut tokio::sync::mpsc::Receiver<Vec<AppAction>>,
    wait: Duration,
) -> Option<AppAction> {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let actions = timeout(remaining, action_rx.recv()).await.ok()??;
        for action in actions {
            if matches!(
                action,
                AppAction::ContactSecurityLoaded { .. }
                    | AppAction::ContactSecurityLoadFailed { .. }
                    | AppAction::ContactSecurityRefreshed { .. }
            ) {
                return Some(action);
            }
        }
    }
}

fn expect_loaded(
    action: Option<AppAction>,
    sequence: u64,
    user: &UserId,
) -> ContactSecuritySummary {
    match action {
        Some(AppAction::ContactSecurityLoaded {
            request_id,
            user_id,
            summary,
        }) => {
            assert_eq!(request_id, sequence);
            assert_eq!(user_id, user.as_str());
            summary
        }
        other => panic!("expected load for request {sequence}, got {other:?}"),
    }
}

#[tokio::test]
async fn contact_security_load_does_not_block_account_actor_during_key_query() {
    let server = MatrixMockServer::new().await;
    server.mock_crypto_endpoints_preset().await;
    let alice = cross_signed_client(
        &server,
        user_id!("@alice:example.test"),
        device_id!("ALICE1"),
    )
    .await;
    let bob_id = user_id!("@bob:example.test");
    let _bob = cross_signed_client(&server, bob_id, device_id!("BOB1")).await;
    let cred_dir = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let (handle, _action_rx, _event_rx) = spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    install_session(&server, &handle, &alice).await;
    let baseline = server.received_requests().await.unwrap_or_default().len();
    delay_next_keys_query(&server).await;

    load(&handle, 1, bob_id).await;
    wait_for_keys_query_after(&server, baseline).await;
    let actor_responded = actor_responds_during_contact_security_request(&handle).await;
    shutdown_and_ack(&handle).await;

    assert!(
        actor_responded,
        "AccountActor waited for contact key-query I/O"
    );
}

#[tokio::test]
async fn contact_verification_does_not_block_account_actor_during_key_query() {
    let server = MatrixMockServer::new().await;
    server.mock_crypto_endpoints_preset().await;
    let alice = cross_signed_client(
        &server,
        user_id!("@alice:example.test"),
        device_id!("ALICE1"),
    )
    .await;
    let bob_id = user_id!("@bob:example.test");
    let _bob = cross_signed_client(&server, bob_id, device_id!("BOB1")).await;
    let cred_dir = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let (handle, mut action_rx, _event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    install_session(&server, &handle, &alice).await;
    let baseline = server.received_requests().await.unwrap_or_default().len();
    delay_next_keys_query(&server).await;

    send(
        &handle,
        request(2),
        ContactSecurityRequest::RequestVerification {
            user_id: bob_id.to_string(),
        },
    )
    .await;
    wait_for_keys_query_after(&server, baseline).await;
    let actor_responded = actor_responds_during_contact_security_request(&handle).await;
    assert!(
        handle
            .send(AccountMessage::Command(
                AccountCommand::CancelVerification {
                    request_id: request(3),
                    flow_id: 2,
                    reason: VerificationCancelReason::User,
                }
            ))
            .await
    );
    let cancelled = timeout(Duration::from_millis(300), async {
        loop {
            let Some(actions) = action_rx.recv().await else {
                return false;
            };
            if actions.iter().any(|action| {
                matches!(
                    action,
                    AppAction::VerificationCancelled {
                        request_id: 2,
                        reason: VerificationCancelReason::User,
                    }
                )
            }) {
                return true;
            }
        }
    })
    .await
    .unwrap_or(false);
    shutdown_and_ack(&handle).await;

    assert!(
        actor_responded,
        "AccountActor waited for verification key-query I/O"
    );
    assert!(
        cancelled,
        "in-flight contact verification should be cancellable"
    );
}

#[tokio::test]
async fn load_then_store_changes_refresh_until_user_info_closes() {
    let server = MatrixMockServer::new().await;
    server.mock_crypto_endpoints_preset().await;
    let alice = cross_signed_client(
        &server,
        user_id!("@alice:example.test"),
        device_id!("ALICE1"),
    )
    .await;
    let bob_id = user_id!("@bob:example.test");
    let _bob = cross_signed_client(&server, bob_id, device_id!("BOB1")).await;
    let cred_dir = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let (handle, mut action_rx, _event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    install_session(&server, &handle, &alice).await;

    load(&handle, 1, bob_id).await;
    let summary = expect_loaded(
        next_contact_action(&mut action_rx, Duration::from_secs(10)).await,
        1,
        bob_id,
    );
    assert_eq!(summary.devices, ContactDevicesStatus::AllOwnerSigned);
    assert_eq!(
        summary.identity,
        ContactIdentityVerification::NotVerifiedByYou
    );

    // Bob adds a device; the SDK store learns it (here through an explicit
    // key query; in production usually through sync) and the open User info
    // refreshes without another retrieval command.
    let _bob_second = crypto_client(&server, bob_id, device_id!("BOB2")).await;
    alice
        .encryption()
        .request_user_identity(bob_id)
        .await
        .expect("query");
    match next_contact_action(&mut action_rx, Duration::from_secs(10)).await {
        Some(AppAction::ContactSecurityRefreshed { user_id, summary }) => {
            assert_eq!(user_id, bob_id.as_str());
            assert_eq!(summary.devices, ContactDevicesStatus::SomeNotOwnerSigned);
            assert_eq!(summary.device_counts.total, 2);
        }
        other => panic!("expected refresh, got {other:?}"),
    }

    // After closing, later store changes are not projected.
    send(&handle, request(2), ContactSecurityRequest::Close).await;
    let _bob_third = crypto_client(&server, bob_id, device_id!("BOB3")).await;
    alice
        .encryption()
        .request_user_identity(bob_id)
        .await
        .expect("query");
    assert!(
        next_contact_action(&mut action_rx, Duration::from_millis(500))
            .await
            .is_none()
    );
    shutdown_and_ack(&handle).await;
}

#[tokio::test]
async fn switching_contacts_stops_refreshing_the_previous_contact() {
    let server = MatrixMockServer::new().await;
    server.mock_crypto_endpoints_preset().await;
    let alice = cross_signed_client(
        &server,
        user_id!("@alice:example.test"),
        device_id!("ALICE1"),
    )
    .await;
    let bob_id = user_id!("@bob:example.test");
    let carol_id = user_id!("@carol:example.test");
    let _bob = cross_signed_client(&server, bob_id, device_id!("BOB1")).await;
    let _carol = crypto_client(&server, carol_id, device_id!("CAROL1")).await;
    let cred_dir = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let (handle, mut action_rx, _event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    install_session(&server, &handle, &alice).await;
    let baseline = server.received_requests().await.unwrap_or_default().len();
    delay_next_keys_query(&server).await;

    load(&handle, 1, bob_id).await;
    wait_for_keys_query_after(&server, baseline).await;
    load(&handle, 2, carol_id).await;
    let carol = expect_loaded(
        next_contact_action(&mut action_rx, Duration::from_secs(10)).await,
        2,
        carol_id,
    );
    assert_eq!(carol.devices, ContactDevicesStatus::OwnerIdentityMissing);
    assert_eq!(carol.identity, ContactIdentityVerification::Unknown);
    assert!(
        next_contact_action(&mut action_rx, Duration::from_secs(1))
            .await
            .is_none(),
        "stale Bob load must not overtake the newer Carol load"
    );

    // A change to Bob's devices re-reads only the open contact (Carol), whose
    // answer is unchanged, so nothing is projected for Bob.
    let _bob_second = crypto_client(&server, bob_id, device_id!("BOB2")).await;
    alice
        .encryption()
        .request_user_identity(bob_id)
        .await
        .expect("query");
    assert!(
        next_contact_action(&mut action_rx, Duration::from_millis(500))
            .await
            .is_none()
    );
    shutdown_and_ack(&handle).await;
}

#[tokio::test]
async fn load_without_a_session_fails_as_unavailable() {
    let cred_dir = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let (handle, mut action_rx, _event_rx) =
        spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
    load(&handle, 5, user_id!("@bob:example.test")).await;
    match next_contact_action(&mut action_rx, Duration::from_secs(5)).await {
        Some(AppAction::ContactSecurityLoadFailed {
            request_id,
            failure_kind,
            ..
        }) => {
            assert_eq!(request_id, 5);
            assert_eq!(failure_kind, ContactSecurityFailureKind::SessionRequired);
        }
        other => panic!("expected failure, got {other:?}"),
    }
    shutdown_and_ack(&handle).await;
}
