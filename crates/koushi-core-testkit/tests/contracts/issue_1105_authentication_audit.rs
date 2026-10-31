//! End-to-end AppActor/AccountActor/store/outcome audit with HTTP fixtures.
use koushi_core::{
    AccountCommand, CoreCommand, CoreConnection,
    account_runtime_manager::AccountRuntimeManager,
    native_artifact::NativeArtifactRegistry,
    runtime::request_outcome::{OutcomeCorrelation, RequestOutcomeExpectation},
    settings::SettingsStore,
    store::StoreActor,
};
use koushi_key::InMemoryCredentialBackend;
use koushi_state::{AuthSecret, DisplayPlatform, LoginRequest, SessionState};
use std::{sync::Arc, time::Duration};
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path, path_regex},
};

async fn server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/_matrix/client/versions")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"versions":["v1.11"],"unstable_features":{"org.matrix.simplified_msc3575":true}}))).mount(&server).await;
    Mock::given(method("POST"))
        .and(path_regex("/_matrix/client/.*/keys/upload"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"one_time_key_counts":{}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex("/_matrix/client/.*/keys/query"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"device_keys":{},"failures":{}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex("/_matrix/client/.*/devices"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"devices":[]})))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path_regex("/_matrix/client/.*/devices/.*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path_regex("/_matrix/client/.*/sync")).respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(100)).set_body_json(serde_json::json!({"pos":"audit", "lists":{}, "rooms":{}, "extensions":{"to_device":{"next_batch":"audit","events":[]}, "e2ee":{"device_lists":{"changed":[],"left":[]},"device_one_time_keys_count":{}}}}))).mount(&server).await;
    Mock::given(method("POST"))
        .and(path_regex("/_matrix/client/.*/logout"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path_regex("/_matrix/client/.*/login")).respond_with(|r: &Request| {
        let body: serde_json::Value = r.body_json().unwrap();
        ResponseTemplate::new(200).set_body_json(serde_json::json!({"access_token":"synthetic-access","device_id":body["device_id"],"user_id":body["identifier"]["user"]}))
    }).with_priority(2).mount(&server).await;
    server
}

fn manager(data: &std::path::Path) -> AccountRuntimeManager {
    AccountRuntimeManager::new(
        StoreActor::with_os_backend(data, Arc::new(InMemoryCredentialBackend::default())),
        SettingsStore::new(data),
        Arc::new(|| Arc::new(NativeArtifactRegistry::new())),
    )
}

async fn login(
    connection: &mut CoreConnection,
    homeserver: String,
    username: &str,
) -> Result<(), koushi_core::runtime::request_outcome::RequestOutcomeError> {
    let request_id = connection.next_request_id();
    let generation = connection.state_generation();
    connection
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id,
            request: LoginRequest {
                homeserver,
                username: username.into(),
                password: AuthSecret::new("synthetic-password"),
                device_display_name: None,
            },
            platform: DisplayPlatform::Linux,
        }))
        .await
        .unwrap();
    connection
        .wait_for_request_outcome(
            OutcomeCorrelation::Request(request_id),
            RequestOutcomeExpectation::Authenticated {
                request_id,
                account_key: None,
            },
            generation,
            tokio::time::Instant::now() + Duration::from_secs(10),
        )
        .await
        .map(|_| ())
}

#[tokio::test]
async fn delegated_password_sign_in_settles_as_second_account() {
    second_account_probe(false).await;
}

#[tokio::test]
async fn second_account_does_not_resume_first_accounts_unverified_journal() {
    second_account_probe(true).await;
}

async fn second_account_probe(same_homeserver: bool) {
    let first_server = server().await;
    let second_server = server().await;
    let target_url = if same_homeserver {
        first_server.uri()
    } else {
        second_server.uri()
    };
    let discovery = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/matrix/client"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"m.homeserver":{"base_url":target_url}})),
        )
        .mount(&discovery)
        .await;
    let data = tempfile::tempdir().unwrap();
    let manager = manager(data.path());
    let first_tab = manager.selected_tab_id();
    let mut first = manager.tab_connection(&first_tab).unwrap();
    let first_outcome = login(&mut first, first_server.uri(), "@first:example.invalid").await;
    let first_phase = session_token(&first.snapshot().session);
    let info = match first.snapshot().session {
        SessionState::AwaitingVerification { info, .. }
        | SessionState::Provisional { info, .. }
        | SessionState::Ready(info)
        | SessionState::AwaitingBootstrapConfirmation { info, .. } => info,
        _ => panic!("unexpected first-account state"),
    };
    manager
        .bind_authenticated_session(&first_tab, &info)
        .await
        .unwrap();
    let second_tab = manager.add_account_tab().await.unwrap();
    let mut second = manager.tab_connection(&second_tab).unwrap();
    let outcome = login(&mut second, discovery.uri(), "@member:example.invalid").await;
    let first_preserved = !matches!(first.snapshot().session, SessionState::SignedOut);
    let second_phase = session_token(&second.snapshot().session);
    let second_gated = matches!(
        second.snapshot().session,
        SessionState::AwaitingVerification { .. }
            | SessionState::AwaitingBootstrapConfirmation { .. }
    );
    let request_count = if same_homeserver {
        first_server.received_requests().await.unwrap()
    } else {
        second_server.received_requests().await.unwrap()
    }
    .iter()
    .filter(|r| r.url.path().ends_with("/login"))
    .count();
    drop(first);
    drop(second);
    manager.shutdown_all_checked().await.unwrap();
    assert!(first_preserved);
    assert_eq!(
        outcome,
        Ok(()),
        "second account must settle: first_outcome={first_outcome:?}, first_phase={first_phase}, second_phase={second_phase}, second_gated={second_gated}, same_homeserver={same_homeserver}, login_requests={request_count}"
    );
}

#[tokio::test]
async fn password_retry_with_retained_pending_journal_reaches_server() {
    let server = server().await;
    Mock::given(method("POST"))
        .and(path_regex("/_matrix/client/.*/login"))
        .respond_with(ResponseTemplate::new(400).set_body_json(
            serde_json::json!({"errcode":"M_UNKNOWN","error":"Synthetic rejection"}),
        ))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    let data = tempfile::tempdir().unwrap();
    let manager = manager(data.path());
    let mut connection = manager.tab_connection(&manager.selected_tab_id()).unwrap();
    let first = login(&mut connection, server.uri(), "@member:example.invalid").await;
    // Core emits failure before projecting SignedOut; wait for the real reducer.
    tokio::time::timeout(Duration::from_secs(2), async {
        while !matches!(connection.snapshot().session, SessionState::SignedOut) {
            connection.next_versioned_snapshot().await.unwrap();
        }
    })
    .await
    .unwrap();
    let second = login(&mut connection, server.uri(), "@member:example.invalid").await;
    let count = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path().ends_with("/login"))
        .count();
    drop(connection);
    manager.shutdown_all_checked().await.unwrap();
    assert!(first.is_err());
    assert_eq!(
        second,
        Ok(()),
        "retained pre-auth allocation prevents retry; login_requests={count}"
    );
}

fn session_token(state: &SessionState) -> &'static str {
    match state {
        SessionState::Provisional {
            phase: koushi_state::ProvisionalPhase::CheckingTrust,
            ..
        } => "checking_trust",
        SessionState::Provisional {
            phase: koushi_state::ProvisionalPhase::DiscoveringMethods,
            ..
        } => "discovering_methods",
        SessionState::Provisional {
            phase: koushi_state::ProvisionalPhase::RecheckingTrust { failure: None },
            ..
        } => "rechecking_trust",
        SessionState::Provisional { .. } => "trust_failure",
        SessionState::AwaitingVerification { .. } => "awaiting_verification",
        SessionState::Ready(_) => "ready",
        SessionState::SignedOut => "signed_out",
        SessionState::Authenticating { .. } => "authenticating",
        SessionState::Rejecting { .. } => "rejecting",
        SessionState::CapabilityBlocked { .. } => "capability_blocked",
        _ => "other",
    }
}
