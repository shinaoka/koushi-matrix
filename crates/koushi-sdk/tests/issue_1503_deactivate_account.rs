//! #1503: deactivation capability and password-UIA retry against disposable
//! HTTP fixtures.
use koushi_sdk::{AccountManagementError, MatrixClientStoreConfig, MatrixClientStoreKey};
use koushi_state::{AuthSecret, IdentityResetAuthRequest, LoginRequest};
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path, path_regex},
};

async fn logged_in_session(
    server: &MockServer,
    data: &tempfile::TempDir,
) -> koushi_sdk::MatrixClientSession {
    Mock::given(method("GET")).and(path("/_matrix/client/versions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"versions":["v1.11"],"unstable_features":{"org.matrix.simplified_msc3575":true}}))).mount(server).await;
    Mock::given(method("POST"))
        .and(path_regex("/_matrix/client/.*/keys/upload"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"one_time_key_counts":{}})),
        )
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex("/_matrix/client/.*/keys/query"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"device_keys":{},"failures":{}})),
        )
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex("/_matrix/client/.*/login"))
        .respond_with(|request: &Request| {
            let body: serde_json::Value = request.body_json().unwrap();
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"access_token":"synthetic-access", "device_id":body["device_id"], "user_id":"@member:example.invalid"}))
        })
        .mount(server)
        .await;
    let config = MatrixClientStoreConfig::new(data.path(), MatrixClientStoreKey::new([23; 32]));
    let request = LoginRequest {
        homeserver: server.uri(),
        username: "@member:example.invalid".into(),
        password: AuthSecret::new("synthetic-password"),
        device_display_name: None,
    };
    koushi_sdk::login_with_password_with_new_device(&request, &config, "SYNTHETIC")
        .await
        .expect("synthetic login")
}

fn uia_challenge() -> ResponseTemplate {
    ResponseTemplate::new(401).set_body_json(serde_json::json!({
        "session": "synthetic-uia-session",
        "flows": [{"stages": ["m.login.password"]}],
        "params": {},
    }))
}

#[tokio::test]
async fn password_session_offers_deactivation_capability() {
    let server = MockServer::start().await;
    let data = tempfile::tempdir().unwrap();
    let session = logged_in_session(&server, &data).await;
    let capabilities = koushi_sdk::account_management_capabilities(&session).await;
    assert!(capabilities.deactivate_account);
}

#[tokio::test]
async fn deactivation_retry_sends_password_auth_with_uia_session() {
    let server = MockServer::start().await;
    let data = tempfile::tempdir().unwrap();
    let session = logged_in_session(&server, &data).await;
    Mock::given(method("POST"))
        .and(path_regex("/_matrix/client/.*/account/deactivate"))
        .respond_with(|request: &Request| {
            let body: serde_json::Value = request.body_json().unwrap();
            if body.get("auth").is_some() {
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"id_server_unbind_result":"no-support"}))
            } else {
                uia_challenge()
            }
        })
        .mount(&server)
        .await;

    let first = koushi_sdk::deactivate_account(&session, true, None, None).await;
    let Err(AccountManagementError::UiaaChallenge {
        session: uia_session,
    }) = first
    else {
        panic!("unauthenticated deactivation must surface the UIA challenge");
    };
    assert_eq!(uia_session.as_deref(), Some("synthetic-uia-session"));

    let auth = IdentityResetAuthRequest::UiaaPassword {
        password: AuthSecret::new("synthetic-password"),
    };
    koushi_sdk::deactivate_account(&session, true, Some(&auth), uia_session.as_deref())
        .await
        .expect("password retry succeeds");

    let requests = server.received_requests().await.unwrap();
    let retry = requests
        .iter()
        .filter(|r| r.url.path().ends_with("/account/deactivate"))
        .nth(1)
        .expect("retry request");
    let body = retry.body_json::<serde_json::Value>().unwrap();
    assert_eq!(body["erase"], true);
    assert_eq!(body["auth"]["type"], "m.login.password");
    assert_eq!(body["auth"]["session"], "synthetic-uia-session");
    assert_eq!(
        body["auth"]["identifier"]["user"],
        "@member:example.invalid"
    );
}
