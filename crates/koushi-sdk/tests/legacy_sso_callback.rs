//! Legacy SSO `loginToken` completion against a deterministic fixture.
//!
//! The fixture advertises only `m.login.sso`, so `start_oidc_login_with_store`
//! takes its legacy fallback: the authorization carries no OAuth CSRF state and
//! the homeserver callback returns only `loginToken`. This covers the SDK half
//! of #1266 (start -> browser callback -> authenticated session); the adapter
//! correlation half is covered by `koushi-core`/`koushi-desktop` tests.

use koushi_sdk::MatrixClientStoreConfig;
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

fn store_config(dir: &std::path::Path, key: u8) -> MatrixClientStoreConfig {
    MatrixClientStoreConfig::new(dir, koushi_sdk::MatrixClientStoreKey::new([key; 32]))
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime should build")
}

#[test]
fn legacy_sso_start_has_no_oauth_state_and_stateless_callback_authenticates() {
    let homeserver = spawn_legacy_sso_server();
    let store = tempfile::tempdir().expect("sso store");
    let runtime = runtime();

    runtime.block_on(async {
        let (pending, authorization) = koushi_sdk::start_oidc_login_with_store(
            &homeserver,
            "com.github.shinaoka.koushi-matrix:/auth/callback",
            Some(&store_config(store.path(), 11)),
            Some("SSODEVICE"),
            false,
        )
        .await
        .expect("legacy SSO authorization");

        // The legacy fallback mints no CSRF state; that empty state is what the
        // desktop adapter must correlate by "one pending legacy attempt".
        assert!(authorization.state.is_empty());
        assert!(authorization.authorization_url.starts_with(&format!(
            "{homeserver}/_matrix/client/v3/login/sso/redirect"
        )));

        let session = koushi_sdk::finish_oidc_login(
            pending,
            "com.github.shinaoka.koushi-matrix:/auth/callback?loginToken=synthetic",
        )
        .await
        .expect("stateless legacy callback completes");

        assert_eq!(session.info.device_id, "SSODEVICE");
        assert_eq!(
            session.info.authentication_method,
            koushi_state::SessionAuthenticationMethod::Sso
        );
        assert_eq!(session.info.homeserver, homeserver);
    });
}

#[test]
fn two_simultaneous_legacy_sso_tabs_complete_with_stateless_callbacks() {
    let homeserver = spawn_legacy_sso_server();
    let first_store = tempfile::tempdir().expect("first sso store");
    let second_store = tempfile::tempdir().expect("second sso store");
    let runtime = runtime();

    runtime.block_on(async {
        // Two tabs start before either callback arrives, so both pending
        // attempts coexist.
        let (first_pending, first_authorization) = koushi_sdk::start_oidc_login_with_store(
            &homeserver,
            "com.github.shinaoka.koushi-matrix:/auth/callback",
            Some(&store_config(first_store.path(), 21)),
            Some("SSODEVICEONE"),
            false,
        )
        .await
        .expect("first legacy SSO authorization");
        let (second_pending, second_authorization) = koushi_sdk::start_oidc_login_with_store(
            &homeserver,
            "com.github.shinaoka.koushi-matrix:/auth/callback",
            Some(&store_config(second_store.path(), 22)),
            Some("SSODEVICETWO"),
            false,
        )
        .await
        .expect("second legacy SSO authorization");
        assert!(first_authorization.state.is_empty());
        assert!(second_authorization.state.is_empty());

        let first_session = koushi_sdk::finish_oidc_login(
            first_pending,
            "com.github.shinaoka.koushi-matrix:/auth/callback?loginToken=synthetic-one",
        )
        .await
        .expect("first tab completes");
        let second_session = koushi_sdk::finish_oidc_login(
            second_pending,
            "com.github.shinaoka.koushi-matrix:/auth/callback?loginToken=synthetic-two",
        )
        .await
        .expect("second tab completes");

        assert_eq!(first_session.info.device_id, "SSODEVICEONE");
        assert_eq!(second_session.info.device_id, "SSODEVICETWO");
    });
}

#[test]
fn legacy_sso_callback_without_a_login_token_is_rejected() {
    let homeserver = spawn_legacy_sso_server();
    let store = tempfile::tempdir().expect("sso store");
    let runtime = runtime();

    runtime.block_on(async {
        let (pending, _) = koushi_sdk::start_oidc_login_with_store(
            &homeserver,
            "com.github.shinaoka.koushi-matrix:/auth/callback",
            Some(&store_config(store.path(), 31)),
            Some("SSODEVICE"),
            false,
        )
        .await
        .expect("legacy SSO authorization");

        // A callback that carries neither loginToken nor state is malformed and
        // must not produce a session.
        let outcome = koushi_sdk::finish_oidc_login(
            pending,
            "com.github.shinaoka.koushi-matrix:/auth/callback?code=synthetic",
        )
        .await;

        assert!(outcome.is_err());
    });
}

/// A disposable local homeserver advertising only `m.login.sso`.
///
/// Every request is synthetic: synthetic host, synthetic device IDs, synthetic
/// login tokens. Nothing here is logged or persisted.
fn spawn_legacy_sso_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let addr = listener
        .local_addr()
        .expect("test server should have an address");

    thread::spawn(move || {
        for _ in 0..64 {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut request = [0_u8; 4096];
            let bytes_read = stream.read(&mut request).unwrap_or(0);
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
            if stream.write_all(response.as_bytes()).is_err() {
                return;
            }
        }
    });

    format!("http://{addr}")
}
