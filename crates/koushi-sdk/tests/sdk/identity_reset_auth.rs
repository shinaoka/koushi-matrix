//! Keep identity-reset continuations on the SDK's authenticated UIAA helpers.

use koushi_sdk::{
    IdentityResetOutcome, MatrixClientSession, complete_identity_reset, reset_identity,
};
use koushi_state::{
    AuthSecret, IdentityResetAuthRequest, SessionAuthenticationMethod, SessionInfo,
};
use matrix_sdk::{encryption::EncryptionSettings, test_utils::mocks::MatrixMockServer};
use serde_json::{Value, json};
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{body_partial_json, method, path_regex},
};

async fn complete_reset(challenge: Value, request: IdentityResetAuthRequest, expected_auth: Value) {
    let server = MatrixMockServer::new().await;
    let client = server
        .client_builder()
        .on_builder(|builder| {
            builder.with_encryption_settings(EncryptionSettings {
                auto_enable_backups: false,
                ..Default::default()
            })
        })
        .build()
        .await;
    let session = MatrixClientSession::from_client_for_testing(
        client.clone(),
        SessionInfo {
            homeserver: server.uri(),
            user_id: client.user_id().expect("fixture user").to_string(),
            device_id: client.device_id().expect("fixture device").to_string(),
            authentication_method: SessionAuthenticationMethod::Unknown,
        },
    );
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "errcode": "M_NOT_FOUND", "error": "No recovery configured",
        })))
        .mount(server.server())
        .await;
    Mock::given(method("PUT"))
        .and(path_regex(
            r"^/_matrix/client/(r0|v3)/user/.*/account_data/",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(server.server())
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r"^/_matrix/client/(r0|v3)/keys/upload$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "one_time_key_counts": {"signed_curve25519": 50},
        })))
        .mount(server.server())
        .await;
    let initial_upload = Mock::given(method("POST"))
        .and(path_regex(
            r"^/_matrix/client/(unstable|v3)/keys/device_signing/upload$",
        ))
        .respond_with(ResponseTemplate::new(401).set_body_json(challenge))
        .expect(1)
        .mount_as_scoped(server.server())
        .await;
    let IdentityResetOutcome::AuthRequired(handle) = reset_identity(&session)
        .await
        .expect("begin identity reset")
    else {
        panic!("the fixture requires authentication");
    };
    drop(initial_upload);

    let wrong_request = match &request {
        IdentityResetAuthRequest::OAuthApproved => IdentityResetAuthRequest::UiaaPassword {
            password: AuthSecret::new("synthetic-password"),
        },
        IdentityResetAuthRequest::UiaaPassword { .. } => IdentityResetAuthRequest::OAuthApproved,
    };
    assert!(
        complete_identity_reset(&session, &handle, &wrong_request)
            .await
            .is_err()
    );
    assert_eq!(
        server
            .server()
            .received_requests()
            .await
            .expect("fixture requests")
            .iter()
            .filter(|request| request.url.path().ends_with("/keys/device_signing/upload"))
            .count(),
        1,
        "mismatched authentication must not send another identity-reset upload"
    );
    Mock::given(method("POST"))
        .and(path_regex(
            r"^/_matrix/client/(unstable|v3)/keys/device_signing/upload$",
        ))
        .and(body_partial_json(json!({"auth": expected_auth})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(server.server())
        .await;
    Mock::given(method("POST"))
        .and(path_regex(
            r"^/_matrix/client/(unstable|v3)/keys/signatures/upload$",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(server.server())
        .await;
    complete_identity_reset(&session, &handle, &request)
        .await
        .expect("continue reset with the SDK's session-bound auth data");
}

#[tokio::test]
async fn password_reset_continuation_keeps_the_uiaa_session() {
    complete_reset(
        json!({
            "flows": [{"stages": ["m.login.password"]}],
            "params": {}, "session": "synthetic-reset-session",
        }),
        IdentityResetAuthRequest::UiaaPassword {
            password: AuthSecret::new("synthetic-password"),
        },
        json!({
            "type": "m.login.password", "session": "synthetic-reset-session",
            "password": "synthetic-password",
        }),
    )
    .await;
}

#[tokio::test]
async fn oauth_reset_continuation_keeps_the_uiaa_session() {
    complete_reset(
        json!({
            "flows": [{"stages": ["org.matrix.cross_signing_reset"]}],
            "params": {"org.matrix.cross_signing_reset": {"url": "https://example.invalid/approve"}},
            "session": "synthetic-reset-session",
        }),
        IdentityResetAuthRequest::OAuthApproved,
        json!({
            "type": "m.oauth", "session": "synthetic-reset-session",
        }),
    )
    .await;
}
