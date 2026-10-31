use koushi_sdk::{
    DiscoveryResponseFailureKind, DiscoverySource, DiscoveryTransportFailureKind, HttpStatusClass,
    SlidingSyncDiscoveryResult, discover_sliding_sync_support,
};
use std::net::TcpListener;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

async fn discover_with_response(
    status: u16,
    body: &str,
) -> (SlidingSyncDiscoveryResult, MockServer) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/_matrix/client/versions"))
        .respond_with(ResponseTemplate::new(status).set_body_raw(body, "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let result = discover_sliding_sync_support(&server.uri()).await;
    (result, server)
}

#[tokio::test]
async fn advertised_simplified_sliding_sync_is_supported_via_one_unauthenticated_versions_request()
{
    let (result, server) = discover_with_response(
        200,
        r#"{"versions":[],"unstable_features":{"org.matrix.simplified_msc3575":true}}"#,
    )
    .await;

    assert_eq!(
        result,
        SlidingSyncDiscoveryResult::Supported {
            source: DiscoverySource::Versions,
            advertised: true,
            http_status_class: Some(HttpStatusClass::Success),
        }
    );
    let requests = server.received_requests().await.expect("captured requests");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.path(), "/_matrix/client/versions");
    assert!(requests[0].headers.get("authorization").is_none());
    assert!(!requests[0].url.path().contains("sync"));
}

#[tokio::test]
async fn missing_or_false_advertisement_is_unsupported_without_server_fingerprinting() {
    for (body, advertised) in [
        (r#"{"versions":["v1.12"],"unstable_features":{}}"#, false),
        (
            r#"{"versions":["v1.12"],"unstable_features":{"org.matrix.simplified_msc3575":false}}"#,
            true,
        ),
        (r#"{"versions":["v1.12"]}"#, false),
    ] {
        let (result, _server) = discover_with_response(200, body).await;
        assert_eq!(
            result,
            SlidingSyncDiscoveryResult::Unsupported {
                advertised,
                http_status_class: Some(HttpStatusClass::Success),
            }
        );
        let rendered = format!("{result:?}");
        assert!(!rendered.contains("Synapse"));
        assert!(!rendered.contains("Tuwunel"));
        assert!(!rendered.contains("Conduit"));
    }
}

#[tokio::test]
async fn redirect_is_invalid_and_is_not_followed_as_a_second_request() {
    let destination = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/_matrix/client/versions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{"versions":[],"unstable_features":{"org.matrix.simplified_msc3575":true}}"#,
            "application/json",
        ))
        .expect(0)
        .mount(&destination)
        .await;

    let origin = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/_matrix/client/versions"))
        .respond_with(ResponseTemplate::new(302).insert_header(
            "Location",
            format!("{}/_matrix/client/versions", destination.uri()),
        ))
        .expect(1)
        .mount(&origin)
        .await;

    let result = discover_sliding_sync_support(&origin.uri()).await;

    assert_eq!(
        result,
        SlidingSyncDiscoveryResult::InvalidResponse {
            failure: DiscoveryResponseFailureKind::HttpStatus,
            http_status_class: Some(HttpStatusClass::Other),
        }
    );
    assert_eq!(
        origin
            .received_requests()
            .await
            .expect("origin requests")
            .len(),
        1
    );
    assert!(
        destination
            .received_requests()
            .await
            .expect("destination requests")
            .is_empty()
    );
}

#[tokio::test]
async fn transport_failure_is_unreachable_without_leaking_the_url() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve local port");
    let url = format!("http://{}", listener.local_addr().expect("local address"));
    drop(listener);

    let result = discover_sliding_sync_support(&url).await;

    assert!(matches!(
        result,
        SlidingSyncDiscoveryResult::Unreachable {
            failure: DiscoveryTransportFailureKind::Connection
                | DiscoveryTransportFailureKind::Other
        }
    ));
    assert!(!format!("{result:?}").contains(&url));
}

#[tokio::test]
async fn malformed_and_non_success_responses_are_invalid_without_leaking_body_or_url() {
    let malformed_marker = "private-response-marker";
    let (malformed, malformed_server) = discover_with_response(200, malformed_marker).await;
    assert_eq!(
        malformed,
        SlidingSyncDiscoveryResult::InvalidResponse {
            failure: DiscoveryResponseFailureKind::Malformed,
            http_status_class: Some(HttpStatusClass::Success),
        }
    );
    let malformed_debug = format!("{malformed:?}");
    assert!(!malformed_debug.contains(malformed_marker));
    assert!(!malformed_debug.contains(&malformed_server.uri()));

    let response_marker = "private-error-marker";
    let (non_success, error_server) = discover_with_response(503, response_marker).await;
    assert_eq!(
        non_success,
        SlidingSyncDiscoveryResult::InvalidResponse {
            failure: DiscoveryResponseFailureKind::HttpStatus,
            http_status_class: Some(HttpStatusClass::ServerError),
        }
    );
    let non_success_debug = format!("{non_success:?}");
    assert!(!non_success_debug.contains(response_marker));
    assert!(!non_success_debug.contains(&error_server.uri()));
}
