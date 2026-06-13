use matrix_desktop_sdk::parse_login_discovery;
use matrix_desktop_state::LoginFlowKind;
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

#[test]
fn parses_password_sso_and_token_flows() {
    let response = serde_json::json!({
        "flows": [
            { "type": "m.login.password" },
            {
                "type": "m.login.sso",
                "org.matrix.msc3824.delegated_oidc_compatibility": true
            },
            { "type": "m.login.token" }
        ]
    });

    let flows = parse_login_discovery(&response).expect("discovery should parse");

    assert_eq!(flows[0].kind, LoginFlowKind::Password);
    assert_eq!(flows[1].kind, LoginFlowKind::Sso);
    assert!(flows[1].delegated_oidc_compatibility);
    assert_eq!(flows[2].kind, LoginFlowKind::Token);
}

#[test]
fn keeps_unknown_flow_type_without_failing() {
    let response = serde_json::json!({
        "flows": [
            { "type": "com.example.login.custom" }
        ]
    });

    let flows = parse_login_discovery(&response).expect("unknown flow should parse");

    assert_eq!(
        flows[0].kind,
        LoginFlowKind::Unknown("com.example.login.custom".to_owned())
    );
}

#[test]
fn rejects_response_without_flows_array() {
    let response = serde_json::json!({
        "not_flows": []
    });

    let error = parse_login_discovery(&response).expect_err("missing flows should fail");

    assert_eq!(
        error.to_string(),
        "login discovery response is missing flows"
    );
}

#[test]
fn builds_discovery_url_from_bare_homeserver_name() {
    let homeserver = matrix_desktop_sdk::Homeserver::parse("matrix.example.org")
        .expect("bare homeserver name should parse");

    assert_eq!(homeserver.normalized(), "https://matrix.example.org");
    assert_eq!(
        homeserver.login_discovery_url().as_str(),
        "https://matrix.example.org/_matrix/client/v3/login"
    );
}

#[test]
fn homeserver_input_allows_scheme_omission_and_explicit_port() {
    let homeserver = matrix_desktop_sdk::Homeserver::parse("matrix.example.org:8448")
        .expect("homeserver with explicit port should parse");

    assert_eq!(homeserver.normalized(), "https://matrix.example.org:8448");
    assert_eq!(
        homeserver.login_discovery_url().as_str(),
        "https://matrix.example.org:8448/_matrix/client/v3/login"
    );
}

#[test]
fn rejects_homeserver_url_with_unsupported_scheme() {
    let error = matrix_desktop_sdk::Homeserver::parse("file:///tmp/matrix")
        .expect_err("file homeserver URL should be rejected");

    assert_eq!(
        error.to_string(),
        "homeserver URL scheme must be http or https"
    );
}

#[test]
fn rejects_plain_http_for_non_loopback_homeserver() {
    let error = matrix_desktop_sdk::Homeserver::parse("http://matrix.example.org")
        .expect_err("non-loopback HTTP homeserver should be rejected");

    assert_eq!(
        error.to_string(),
        "homeserver URL must use https unless it is localhost or loopback"
    );
}

#[test]
fn maps_non_successful_http_response_to_discovery_error() {
    let error = matrix_desktop_sdk::parse_login_discovery_http_response(
        404,
        r#"{"errcode":"M_UNRECOGNIZED","error":"OAuth 2.0 authentication is in use on this homeserver."}"#,
    )
    .expect_err("non-200 discovery should fail");

    assert_eq!(
        error.to_string(),
        "login discovery failed with HTTP 404: OAuth 2.0 authentication is in use on this homeserver."
    );
}

#[test]
fn discovers_login_flows_over_http() {
    let homeserver = spawn_login_discovery_server(
        200,
        r#"{"flows":[{"type":"m.login.password"},{"type":"m.login.sso"}]}"#,
    );

    let discovery =
        matrix_desktop_sdk::discover_login_flows(&homeserver).expect("discovery should succeed");

    assert_eq!(discovery.homeserver, homeserver);
    assert_eq!(discovery.flows[0].kind, LoginFlowKind::Password);
    assert_eq!(discovery.flows[1].kind, LoginFlowKind::Sso);
}

fn spawn_login_discovery_server(status: u16, body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let addr = listener
        .local_addr()
        .expect("test server should have an address");

    thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("test server should accept a request");
        let mut request = [0_u8; 2048];
        let bytes_read = stream
            .read(&mut request)
            .expect("test server should read request");
        let request = String::from_utf8_lossy(&request[..bytes_read]);
        assert!(request.starts_with("GET /_matrix/client/v3/login HTTP/1.1"));

        let response = format!(
            "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .expect("test server should write response");
    });

    format!("http://{addr}")
}
