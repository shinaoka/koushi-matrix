//! Runtime integration tests for E2EE trust command projection.

use std::{
    sync::atomic::{AtomicUsize, Ordering},
    sync::{Arc, Mutex},
    time::Duration,
};

use koushi_core::executor;
use koushi_core::runtime::CoreRuntime;
use koushi_protocol::command::{AccountCommand, CoreCommand};
use koushi_state::{AuthSecret, CrossSigningStatus, LoginRequest, SessionState};
use matrix_sdk::test_utils::mocks::MatrixMockServer;
use serde_json::json;
use wiremock::{
    Mock, Request, Respond, ResponseTemplate,
    matchers::{method, path},
};

mod support;
use support::*;

#[derive(Clone)]
struct NotifyFirstSlidingSync {
    first_request: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    request_count: Arc<AtomicUsize>,
}

#[derive(Clone)]
struct EchoRequestedLoginDevice;

impl Respond for EchoRequestedLoginDevice {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: serde_json::Value =
            serde_json::from_slice(&request.body).expect("login request JSON");
        let device_id = body["device_id"]
            .as_str()
            .expect("fresh login requests its generated device id");
        ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "synthetic-access-token",
            "device_id": device_id,
            "user_id": "@provisional-owner:localhost"
        }))
    }
}

impl Respond for NotifyFirstSlidingSync {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        let request_index = self.request_count.fetch_add(1, Ordering::AcqRel);
        if request_index == 0
            && let Some(sender) = self
                .first_request
                .lock()
                .expect("request probe lock")
                .take()
        {
            let _ = sender.send(());
        }
        let response = ResponseTemplate::new(200).set_body_json(json!({ "pos": "0" }));
        if request_index == 0 {
            response
        } else {
            ResponseTemplate::new(500).set_body_json(json!({
                "errcode": "M_UNKNOWN",
                "error": "synthetic failure"
            }))
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provisional_verification_hands_one_encryption_sync_owner_to_normal_runtime() {
    let server = MatrixMockServer::new().await;
    server
        .mock_versions()
        .with_feature("org.matrix.simplified_msc3575", true)
        .ok()
        .mount()
        .await;
    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/login"))
        .respond_with(EchoRequestedLoginDevice)
        .expect(1)
        .mount(&server.server())
        .await;
    let (first_request_tx, first_request_rx) = tokio::sync::oneshot::channel();
    Mock::given(method("POST"))
        .and(path(
            "/_matrix/client/unstable/org.matrix.simplified_msc3575/sync",
        ))
        .respond_with(NotifyFirstSlidingSync {
            first_request: Arc::new(Mutex::new(Some(first_request_tx))),
            request_count: Arc::new(AtomicUsize::new(0)),
        })
        .mount(&server.server())
        .await;

    let data_dir = tempfile::tempdir().expect("runtime data directory");
    let credential_dir = tempfile::tempdir().expect("runtime credential directory");
    let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    let mut connection = runtime.attach();
    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id,
            request: LoginRequest {
                homeserver: server.uri(),
                username: "provisional-owner".to_owned(),
                password: AuthSecret::new("synthetic-password"),
                device_display_name: None,
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await
        .expect("submit login");

    wait_for_state_event(&mut connection, |state| {
        matches!(
            state.session,
            SessionState::Provisional { .. } | SessionState::AwaitingVerification { .. }
        )
    })
    .await;
    // The probe is installed before login and the oneshot buffers a request that
    // wins this state observation, so splitting the phase fences loses no event.
    executor::timeout(Duration::from_secs(5), first_request_rx)
        .await
        .expect("provisional encryption request deadline")
        .expect("provisional encryption request probe");
    assert_eq!(
        runtime.inspect_sync_owners_for_testing().await,
        (true, false, false),
        "verification owns only provisional encryption sync"
    );
    let provisional_requests = server.received_requests().await.expect("captured requests");
    assert!(
        provisional_requests
            .iter()
            .all(|request| request.url.path() != "/_matrix/client/v3/sync"),
        "provisional runtime must never issue classic /sync"
    );
    assert!(provisional_requests.iter().any(|request| {
        request.url.path() == "/_matrix/client/unstable/org.matrix.simplified_msc3575/sync"
            && serde_json::from_slice::<serde_json::Value>(&request.body)
                .is_ok_and(|body| body["conn_id"] == "encryption")
    }));

    assert!(
        runtime
            .set_current_device_trust_for_testing(koushi_state::CurrentDeviceTrustState::Verified,)
            .await
    );
    wait_for_state_event(&mut connection, |state| {
        matches!(state.session, SessionState::Ready(_))
    })
    .await;
    assert_eq!(
        runtime.inspect_sync_owners_for_testing().await,
        (false, false, true),
        "provisional owner must be stopped and joined before normal SyncActor starts"
    );
}

#[tokio::test]
async fn e2ee_trust_account_command_settles_without_an_sdk_session() {
    let runtime = CoreRuntime::start();
    let mut connection = runtime.attach();

    runtime.inject_actions(restore_ready_actions()).await;

    wait_for_state_event(&mut connection, |state| {
        matches!(state.session, SessionState::Ready(_))
    })
    .await;

    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Account(
            AccountCommand::BootstrapCrossSigning {
                request_id,
                auth: None,
            },
        ))
        .await
        .expect("submit bootstrap cross-signing");

    let snapshot = wait_for_state_event(&mut connection, |state| {
        matches!(
            state.e2ee_trust.cross_signing,
            CrossSigningStatus::Failed { request_id: observed, .. }
                if observed == request_id.sequence
        )
    })
    .await;
    assert!(matches!(
        snapshot.e2ee_trust.cross_signing,
        CrossSigningStatus::Failed { request_id: observed, .. }
            if observed == request_id.sequence
    ));
}
