use koushi_sdk::parse_login_discovery;
use koushi_state::LoginFlowKind;
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
fn oidc_login_flow_maps_to_desktop_oidc_without_tokens() {
    let flows = vec![koushi_sdk::MatrixLoginFlow {
        kind: koushi_sdk::MatrixLoginFlowKind::Oidc,
        delegated_oidc_compatibility: true,
        display_name: Some("Provider".to_owned()),
    }];

    let mapped = koushi_sdk::map_login_flows_to_desktop(flows);

    assert_eq!(mapped[0].kind, koushi_state::LoginFlowKind::Oidc);
    assert!(mapped[0].delegated_oidc_compatibility);
    assert_eq!(mapped[0].display_name.as_deref(), Some("Provider"));
}

#[test]
fn oauth_authorization_response_debug_redacts_url_and_state() {
    let authorization = koushi_sdk::OidcAuthorization {
        authorization_url: "https://issuer.example.test/auth?code=secret".to_owned(),
        state: "csrf-secret".to_owned(),
    };

    let debug = format!("{authorization:?}");

    assert!(debug.contains("OidcAuthorization"));
    assert!(!debug.contains("issuer.example.test"));
    assert!(!debug.contains("csrf-secret"));
}

#[test]
fn oauth_persistable_session_shape_is_tagged_and_secret_redacted() {
    let json = r#"{
        "auth_kind": "oauth",
        "homeserver": "https://matrix.example.test",
        "user_session": {
            "user_id": "@alice:example.test",
            "device_id": "DEVICEID",
            "access_token": "access-secret",
            "refresh_token": "refresh-secret"
        },
        "client_id": "koushi-test-client"
    }"#;

    let session = koushi_sdk::PersistableMatrixSession::from_json(json)
        .expect("tagged OAuth session should parse");

    assert_eq!(session.info.homeserver, "https://matrix.example.test");
    assert_eq!(session.info.user_id, "@alice:example.test");
    assert_eq!(session.info.device_id, "DEVICEID");
    assert_eq!(session.auth_kind(), koushi_sdk::PersistableAuthKind::OAuth);

    let debug = format!("{session:?}");
    assert!(!debug.contains("access-secret"));
    assert!(!debug.contains("refresh-secret"));
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
    let homeserver = koushi_sdk::Homeserver::parse("matrix.example.org")
        .expect("bare homeserver name should parse");

    assert_eq!(homeserver.normalized(), "https://matrix.example.org");
    assert_eq!(
        homeserver.login_discovery_url().as_str(),
        "https://matrix.example.org/_matrix/client/v3/login"
    );
}

#[test]
fn homeserver_input_allows_scheme_omission_and_explicit_port() {
    let homeserver = koushi_sdk::Homeserver::parse("matrix.example.org:8448")
        .expect("homeserver with explicit port should parse");

    assert_eq!(homeserver.normalized(), "https://matrix.example.org:8448");
    assert_eq!(
        homeserver.login_discovery_url().as_str(),
        "https://matrix.example.org:8448/_matrix/client/v3/login"
    );
}

#[test]
fn rejects_homeserver_url_with_unsupported_scheme() {
    let error = koushi_sdk::Homeserver::parse("file:///tmp/matrix")
        .expect_err("file homeserver URL should be rejected");

    assert_eq!(
        error.to_string(),
        "homeserver URL scheme must be http or https"
    );
}

#[test]
fn well_known_client_url_is_origin_root_based_regardless_of_base_path() {
    let homeserver = koushi_sdk::Homeserver::parse("https://matrix.example.org/matrix")
        .expect("base-path homeserver should parse");

    assert_eq!(
        homeserver.well_known_client_url().as_str(),
        "https://matrix.example.org/.well-known/matrix/client"
    );
    // The login discovery path stays relative to the base path.
    assert_eq!(
        homeserver.login_discovery_url().as_str(),
        "https://matrix.example.org/matrix/_matrix/client/v3/login"
    );
}

#[test]
fn discovered_urls_reject_embedded_credentials() {
    let homeserver = koushi_sdk::Homeserver::parse("https://user:pass@matrix.example.org")
        .expect_err("homeserver URL with credentials should be rejected");
    assert_eq!(
        homeserver.to_string(),
        "homeserver URL is invalid: homeserver URL must not include credentials"
    );

    let well_known = serde_json::json!({
        "m.authentication": {
            "account": "https://user:secret@account.example.test/account",
            "registration": "https://account.example.test/register"
        }
    });
    let links = koushi_sdk::parse_well_known_client(&well_known);
    assert_eq!(
        links.registration_url.as_deref(),
        Some("https://account.example.test/register")
    );
}

#[test]
fn well_known_debug_redacts_url_values() {
    let well_known = serde_json::json!({
        "m.authentication": {
            "registration": "https://account.example.test/register?token=secret"
        }
    });
    let links = koushi_sdk::parse_well_known_client(&well_known);
    let debug = format!("{links:?}");
    assert!(!debug.contains("account.example.test"));
    assert!(!debug.contains("secret"));
    assert!(debug.contains("Url(..)"));
}

#[test]
fn rejects_plain_http_for_non_loopback_homeserver() {
    let error = koushi_sdk::Homeserver::parse("http://matrix.example.org")
        .expect_err("non-loopback HTTP homeserver should be rejected");

    assert_eq!(
        error.to_string(),
        "homeserver URL must use https unless it is localhost or loopback"
    );
}

#[test]
fn maps_non_successful_http_response_to_discovery_error() {
    let error = koushi_sdk::parse_login_discovery_http_response(
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
fn parses_registration_url_from_well_known_delegated_auth() {
    let well_known = serde_json::json!({
        "m.homeserver": { "base_url": "https://matrix.example.test" },
        "m.authentication": {
            "issuer": "https://auth.example.test/",
            "account": "https://auth.example.test/account",
            "registration": "https://auth.example.test/register"
        }
    });

    let links = koushi_sdk::parse_well_known_client(&well_known);

    assert_eq!(
        links.registration_url.as_deref(),
        Some("https://auth.example.test/register")
    );
}

#[test]
fn parses_registration_url_from_msc2965_prefixed_well_known_key() {
    // matrix.org still serves the org.matrix.msc2965.authentication spelling.
    let well_known = serde_json::json!({
        "org.matrix.msc2965.authentication": {
            "issuer": "https://account.example.test/",
            "registration": "https://account.example.test/register/"
        }
    });

    let links = koushi_sdk::parse_well_known_client(&well_known);

    assert_eq!(
        links.registration_url.as_deref(),
        Some("https://account.example.test/register/")
    );
}

#[test]
fn well_known_without_delegated_auth_metadata_is_unavailable() {
    let well_known =
        serde_json::json!({ "m.homeserver": { "base_url": "https://matrix.example.test" } });

    let links = koushi_sdk::parse_well_known_client(&well_known);

    assert!(links.registration_url.is_none());
}

#[test]
fn malformed_or_unsupported_scheme_discovery_values_are_unavailable() {
    let well_known = serde_json::json!({
        "m.authentication": {
            "account": "not a url",
            "registration": "javascript:alert(1)"
        }
    });

    let links = koushi_sdk::parse_well_known_client(&well_known);

    assert!(links.registration_url.is_none());
}

#[test]
fn non_string_discovery_values_are_unavailable() {
    let well_known = serde_json::json!({
        "m.authentication": {
            "account": 42,
            "registration": ["https://auth.example.test/register"]
        }
    });

    let links = koushi_sdk::parse_well_known_client(&well_known);

    assert!(links.registration_url.is_none());
}

#[test]
fn login_discovery_fails_open_when_well_known_is_missing() {
    let homeserver = spawn_discovery_with_well_known_server(
        200,
        r#"{"flows":[{"type":"m.login.password"}]}"#,
        // The server returns 404 for the well-known path; login still works
        // and the optional registration link is simply unavailable.
        None,
    );

    let discovery =
        koushi_sdk::discover_login_flows(&homeserver).expect("discovery should succeed");

    assert!(discovery.delegated.registration_url.is_none());
}

#[test]
fn sso_completion_keeps_the_pre_auth_persistent_store_and_requested_device() {
    let homeserver = spawn_legacy_sso_server();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime should build");
    let store = tempfile::tempdir().expect("persistent SSO store");
    let config = koushi_sdk::MatrixClientStoreConfig::new(
        store.path(),
        koushi_sdk::MatrixClientStoreKey::new([55; 32]),
    );

    runtime.block_on(async {
        let (pending, _) = koushi_sdk::start_oidc_login_with_store(
            &homeserver,
            "koushi-desktop://auth/callback",
            Some(&config),
            Some("SSODEVICE"),
            false,
        )
        .await
        .expect("persistent SSO authorization");
        let session = koushi_sdk::finish_oidc_login(
            pending,
            "koushi-desktop://auth/callback?loginToken=synthetic",
        )
        .await
        .expect("persistent SSO completion");
        assert_eq!(session.info.device_id, "SSODEVICE");
        drop(session);
        assert_eq!(
            koushi_sdk::preflight_saved_crypto_store(
                &config,
                Some("@sso:example.invalid"),
                Some("SSODEVICE"),
            )
            .await,
            koushi_sdk::SavedCryptoStorePreflight::PresentMatching,
        );
    });
}

#[test]
fn discovers_login_flows_over_http() {
    let homeserver = spawn_login_discovery_server(
        200,
        r#"{"flows":[{"type":"m.login.password"},{"type":"m.login.sso"}]}"#,
    );

    let discovery =
        koushi_sdk::discover_login_flows(&homeserver).expect("discovery should succeed");

    assert_eq!(discovery.homeserver, homeserver);
    assert_eq!(discovery.flows[0].kind, LoginFlowKind::Password);
    assert_eq!(discovery.flows[1].kind, LoginFlowKind::Sso);
}

#[test]
fn starts_legacy_sso_login_when_discovery_has_plain_sso_flow() {
    let homeserver = spawn_legacy_sso_server();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime should build");

    let (_pending, authorization) = runtime
        .block_on(koushi_sdk::start_oidc_login(
            &homeserver,
            "koushi-desktop://auth/callback",
        ))
        .expect("legacy SSO should produce an authorization URL");

    assert!(authorization.state.is_empty());
    assert!(authorization.authorization_url.starts_with(&format!(
        "{homeserver}/_matrix/client/v3/login/sso/redirect"
    )));
    assert!(
        authorization
            .authorization_url
            .contains("redirectUrl=koushi-desktop%3A%2F%2Fauth%2Fcallback")
    );
}

fn spawn_login_discovery_server(status: u16, body: &'static str) -> String {
    spawn_discovery_with_well_known_server(status, body, None)
}

fn spawn_discovery_with_well_known_server(
    status: u16,
    login_body: &'static str,
    well_known_body: Option<&'static str>,
) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let addr = listener
        .local_addr()
        .expect("test server should have an address");

    thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener
                .accept()
                .expect("test server should accept a request");
            // Read until the request is complete: headers end at \r\n\r\n and
            // a body, if any, has the declared Content-Length (header names
            // are case-insensitive). A single read() can stop mid-request on
            // a split TCP segment.
            let mut request = Vec::new();
            let mut buffer = [0_u8; 2048];
            loop {
                let count = stream
                    .read(&mut buffer)
                    .expect("test server should read request");
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
                let text = String::from_utf8_lossy(&request);
                if let Some(end) = text.find("\r\n\r\n") {
                    let declared_length = text
                        .split("\r\n")
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + declared_length {
                        break;
                    }
                }
            }
            let request = String::from_utf8_lossy(&request);
            let (response_status, body): (u16, Vec<u8>) =
                if request.starts_with("GET /_matrix/client/v3/login HTTP/1.1") {
                    (status, login_body.as_bytes().to_vec())
                } else if request.starts_with("GET /.well-known/matrix/client HTTP/1.1") {
                    match well_known_body {
                        Some(well_known) => (200, well_known.as_bytes().to_vec()),
                        None => (
                            404,
                            b"{\"errcode\":\"M_NOT_FOUND\",\"error\":\"not found\"}".to_vec(),
                        ),
                    }
                } else {
                    (
                        404,
                        b"{\"errcode\":\"M_NOT_FOUND\",\"error\":\"not found\"}".to_vec(),
                    )
                };

            let response = format!(
                "HTTP/1.1 {response_status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let mut bytes = response.into_bytes();
            bytes.extend_from_slice(&body);
            stream
                .write_all(&bytes)
                .expect("test server should write response");
        }
    });

    format!("http://{addr}")
}

fn spawn_legacy_sso_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let addr = listener
        .local_addr()
        .expect("test server should have an address");

    thread::spawn(move || {
        for _ in 0..8 {
            let (mut stream, _) = listener
                .accept()
                .expect("test server should accept a request");
            let mut request = [0_u8; 2048];
            let bytes_read = stream
                .read(&mut request)
                .expect("test server should read request");
            let request = String::from_utf8_lossy(&request[..bytes_read]);
            let (status, body) = if request.starts_with("GET /_matrix/client/v3/login HTTP/1.1") {
                (200, r#"{"flows":[{"type":"m.login.sso"}]}"#.to_owned())
            } else if request.starts_with("GET /_matrix/client/versions HTTP/1.1") {
                (200, r#"{"versions":["v1.1","v1.2","v1.3"]}"#.to_owned())
            } else if request.starts_with("POST /_matrix/client/v3/login HTTP/1.1") {
                let device_id = request
                    .split_once("\r\n\r\n")
                    .and_then(|(_, body)| serde_json::from_str::<serde_json::Value>(body).ok())
                    .and_then(|body| body["device_id"].as_str().map(str::to_owned))
                    .unwrap_or_else(|| "SSODEVICE".to_owned());
                (
                    200,
                    format!(
                        r#"{{"access_token":"sso-token","device_id":"{device_id}","user_id":"@sso:example.invalid"}}"#
                    ),
                )
            } else {
                (
                    404,
                    r#"{"errcode":"M_NOT_FOUND","error":"not found"}"#.to_owned(),
                )
            };

            let response = format!(
                "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("test server should write response");
        }
    });

    format!("http://{addr}")
}
