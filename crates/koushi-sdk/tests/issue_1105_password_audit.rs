//! Real SDK/store probes against disposable HTTP fixtures.
use koushi_sdk::{MatrixClientStoreConfig, MatrixClientStoreKey};
use koushi_state::{AuthSecret, LoginRequest};
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path, path_regex},
};

async fn homeserver() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/_matrix/client/versions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"versions":["v1.11"],"unstable_features":{"org.matrix.simplified_msc3575":true}}))).mount(&server).await;
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
    server
}

fn accepted(request: &Request) -> ResponseTemplate {
    let body: serde_json::Value = request.body_json().unwrap();
    ResponseTemplate::new(200).set_body_json(serde_json::json!({"access_token":"synthetic-access", "device_id":body["device_id"], "user_id":"@member:example.invalid"}))
}

fn credentials(homeserver: String) -> LoginRequest {
    LoginRequest {
        homeserver,
        username: "@member:example.invalid".into(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: None,
    }
}

#[tokio::test]
async fn delegated_full_matrix_id_logs_in_on_fresh_persistent_store() {
    let server = homeserver().await;
    let discovery = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/matrix/client"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"m.homeserver":{"base_url":server.uri()}})),
        )
        .mount(&discovery)
        .await;
    Mock::given(method("POST"))
        .and(path_regex("/_matrix/client/.*/login"))
        .respond_with(accepted)
        .mount(&server)
        .await;
    let data = tempfile::tempdir().unwrap();
    let config = MatrixClientStoreConfig::new(data.path(), MatrixClientStoreKey::new([17; 32]));
    let resolved = koushi_sdk::resolve_homeserver(&discovery.uri())
        .await
        .unwrap();
    let result = koushi_sdk::login_with_password_with_new_device(
        &credentials(resolved.normalized()),
        &config,
        "SYNTHETIC",
    )
    .await;
    assert!(
        result.is_ok(),
        "delegated login should work on a fresh persistent SDK store"
    );
    let session = result.unwrap();
    assert_eq!(session.info.user_id, "@member:example.invalid");
    assert_eq!(session.info.homeserver, server.uri());
    let requests = server.received_requests().await.unwrap();
    let login = requests
        .iter()
        .find(|r| r.url.path().ends_with("/login"))
        .unwrap();
    assert_eq!(
        login.body_json::<serde_json::Value>().unwrap()["identifier"]["user"],
        "@member:example.invalid"
    );
}

#[tokio::test]
async fn retry_after_unclassified_rejection_reaches_login_endpoint() {
    let server = homeserver().await;
    Mock::given(method("POST"))
        .and(path_regex("/_matrix/client/.*/login"))
        .respond_with(ResponseTemplate::new(400).set_body_json(
            serde_json::json!({"errcode":"M_UNKNOWN","error":"Synthetic rejection"}),
        ))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex("/_matrix/client/.*/login"))
        .respond_with(accepted)
        .with_priority(2)
        .mount(&server)
        .await;
    let data = tempfile::tempdir().unwrap();
    let config = MatrixClientStoreConfig::new(data.path(), MatrixClientStoreKey::new([18; 32]));
    let request = credentials(server.uri());
    assert!(
        koushi_sdk::login_with_password_with_new_device(&request, &config, "SYNTHETIC")
            .await
            .is_err()
    );
    let second =
        koushi_sdk::login_with_password_with_new_device(&request, &config, "SYNTHETIC").await;
    let failure = match &second {
        Err(koushi_sdk::PasswordLoginError::SavedCryptoStore(
            koushi_sdk::SavedCryptoStorePreflight::Empty,
        )) => "empty_pre_auth_store",
        Err(_) => "other",
        Ok(_) => "none",
    };
    let count = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path().ends_with("/login"))
        .count();
    assert!(
        second.is_ok(),
        "retry failed before login: stage={failure}, login_requests={count}"
    );
}
