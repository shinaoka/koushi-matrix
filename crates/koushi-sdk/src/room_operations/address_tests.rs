use super::{
    MatrixClientSession, MatrixCreateRoomOptions, MatrixCreateRoomVisibility, create_room,
};
use matrix_sdk::test_utils::mocks::MatrixMockServer;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{method, path_regex},
};

#[tokio::test]
async fn room_alias_collision_is_distinct_from_network_failure_and_redacts_server_text() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let session = MatrixClientSession {
        info: koushi_state::SessionInfo {
            homeserver: server.server().uri(),
            user_id: client.user_id().unwrap().to_string(),
            device_id: client.device_id().unwrap().to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
        client,
        diagnostic_counters: koushi_diagnostics::DiagnosticCounterContext::registered(),
    };
    Mock::given(method("POST"))
        .and(path_regex(r"/_matrix/client/.*/createRoom"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "errcode": "M_ROOM_IN_USE", "error": "private server text: synthetic-secret"
        })))
        .expect(1)
        .mount(server.server())
        .await;
    let error = create_room(
        &session,
        MatrixCreateRoomOptions {
            name: "Example Room".into(),
            topic: None,
            alias_localpart: Some("example-room".into()),
            encrypted: false,
            visibility: MatrixCreateRoomVisibility::Public,
            parent_space: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.failure_kind().map(|kind| kind.to_string()).as_deref(),
        Some("alias_in_use")
    );
    assert!(!format!("{error:?} {error}").contains("synthetic-secret"));
}
