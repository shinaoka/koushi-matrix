use super::{MatrixClientSession, MatrixRoomAliasAvailability, check_room_alias_availability};
use matrix_sdk::test_utils::mocks::MatrixMockServer;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{method, path_regex},
};

async fn session(server: &MatrixMockServer) -> MatrixClientSession {
    let client = server.client_builder().build().await;
    MatrixClientSession {
        info: koushi_state::SessionInfo {
            homeserver: server.server().uri(),
            user_id: client.user_id().unwrap().to_string(),
            device_id: client.device_id().unwrap().to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
        client,
        diagnostic_counters: koushi_diagnostics::DiagnosticCounterContext::registered(),
    }
}

#[tokio::test]
async fn a_resolving_alias_is_in_use() {
    let server = MatrixMockServer::new().await;
    server
        .mock_room_directory_resolve_alias()
        .ok("!room:example.invalid", vec!["example.invalid".to_owned()])
        .expect(1)
        .mount()
        .await;
    let session = session(&server).await;
    assert_eq!(
        check_room_alias_availability(&session, "#papers:example.invalid").await,
        MatrixRoomAliasAvailability::InUse
    );
}

#[tokio::test]
async fn a_not_found_alias_is_available() {
    let server = MatrixMockServer::new().await;
    server
        .mock_room_directory_resolve_alias()
        .not_found()
        .expect(1)
        .mount()
        .await;
    let session = session(&server).await;
    assert_eq!(
        check_room_alias_availability(&session, "#papers:example.invalid").await,
        MatrixRoomAliasAvailability::Available
    );
}

#[tokio::test]
async fn any_other_failure_is_unknown_never_available() {
    let server = MatrixMockServer::new().await;
    Mock::given(method("GET"))
        .and(path_regex(r"/_matrix/client/v3/directory/room/.*"))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
            "errcode": "M_FORBIDDEN", "error": "synthetic"
        })))
        .mount(server.server())
        .await;
    let session = session(&server).await;
    assert_eq!(
        check_room_alias_availability(&session, "#papers:example.invalid").await,
        MatrixRoomAliasAvailability::Unknown
    );
    assert_eq!(
        check_room_alias_availability(&session, "not an alias").await,
        MatrixRoomAliasAvailability::Unknown
    );
}
