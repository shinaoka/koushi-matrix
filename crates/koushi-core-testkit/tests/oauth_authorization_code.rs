//! #1326: the complete OAuth/MAS sign-in path through the SDK and Core.
//!
//! Every assertion here runs against the disposable authorization server and
//! homeserver fixture in `support::oauth_fixture` over real TCP. The
//! production code under test is unchanged: `koushi_sdk::start_oidc_login_with_store`
//! / `finish_oidc_login` for the browser half, and `CoreRuntime`'s real
//! `AccountCommand::StartOidcLogin` / `CompleteOidcLogin` admission for the
//! Core half. No Tauri command, IPC mock, or store-preflight report is used as
//! OAuth completion evidence.
//!
//! Fixture identities, codes, tokens, and keys are synthetic. Token values are
//! never printed: assertions on them compare booleans, so a red CI log stays
//! token-free.

mod support;

use std::{sync::Arc, time::Duration};

use koushi_core::runtime::{CoreConnection, CoreRuntime};
use koushi_core::{AccountCommand, CoreCommand, CoreEvent};
use koushi_protocol::event::AccountEvent;
use koushi_state::{
    AuthFailureStage, AuthFailureTransport, AuthMethod, DelegatedAuthMethod, DisplayPlatform,
    LoginFlowKind, SessionAuthenticationMethod, SessionState,
};
use support::oauth_fixture::{
    OAUTH_CLIENT_ID, OAUTH_REDIRECT_URI, OAUTH_USER_ID, OauthFixture, TokenExchangeMode,
    code_from_callback, state_from_callback, with_state,
};

const OAUTH_DEVICE_ID: &str = "OAUTHFIXTUREDEVICE";

/// Bound on waiting for the restarted runtime to authenticate with the token
/// the code exchange minted. The restarted runtime restores the persisted
/// session locally and reports it ready before the restored client reaches the
/// network, so the proof waits for that request; expiring the wait still fails
/// it.
const RESTORE_TOKEN_REUSE_TIMEOUT: Duration = Duration::from_secs(10);

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime should build")
}

fn store_config(dir: &std::path::Path) -> koushi_sdk::MatrixClientStoreConfig {
    koushi_sdk::MatrixClientStoreConfig::new(dir, koushi_sdk::MatrixClientStoreKey::new([0x13; 32]))
}

/// Start one OAuth attempt against the fixture, exactly as the desktop's
/// adapter does, and return the pending allocation plus the authorization the
/// UI would open.
async fn start_oauth(
    fixture: &OauthFixture,
    store: &std::path::Path,
    device_id: &str,
) -> (koushi_sdk::PendingOidcLogin, koushi_sdk::OidcAuthorization) {
    koushi_sdk::start_oidc_login_with_store(
        &fixture.homeserver,
        OAUTH_REDIRECT_URI,
        Some(&store_config(store)),
        Some(device_id),
        false,
        DelegatedAuthMethod::OAuth,
    )
    .await
    .unwrap_or_else(|error| {
        panic!(
            "OAuth authorization should be created: {error:?}; requests: {:?}",
            fixture.observations().paths()
        )
    })
}

/// The production discovery call reports the OAuth-only homeserver's delegated
/// method rather than a password flow.
#[test]
fn oauth_only_discovery_reports_the_delegated_method() {
    let fixture = OauthFixture::start();

    let discovery = koushi_sdk::discover_login_flows(&fixture.homeserver)
        .expect("login discovery against the fixture");

    assert_eq!(discovery.homeserver, fixture.homeserver);
    assert_eq!(discovery.flows.len(), 1);
    assert_eq!(discovery.flows[0].kind, LoginFlowKind::Oidc);
    assert!(discovery.flows[0].delegated_oidc_compatibility);
}

/// The happy path: authorization creation carries the registered redirect URI
/// and a nonempty CSRF state, the callback the fixture redirects to is
/// exchanged for tokens over a real socket with a PKCE-bound code, and the
/// result is an OAuth-authenticated session on the same persistent client.
#[test]
fn oauth_authorization_code_exchange_authenticates_the_persistent_client() {
    let fixture = OauthFixture::start();
    let store = tempfile::tempdir().expect("oauth store");
    let runtime = runtime();

    runtime.block_on(async {
        let (pending, authorization) = start_oauth(&fixture, store.path(), OAUTH_DEVICE_ID).await;
        assert_eq!(authorization.method, AuthMethod::OAuth);
        assert!(!authorization.legacy_sso_fallback);
        assert!(
            !authorization.state.is_empty(),
            "authorization creation must mint a nonempty CSRF state"
        );
        assert!(
            authorization
                .authorization_url
                .contains(&format!("state={}", authorization.state)),
            "the authorization URL must carry the minted CSRF state"
        );
        assert!(
            authorization
                .authorization_url
                .contains("code_challenge_method=S256"),
            "the authorization URL must request the S256 PKCE challenge"
        );

        // The browser half: fetch the authorization URL and follow its redirect.
        let callback_url = fixture
            .authorize(&authorization.authorization_url)
            .expect("authorization should redirect to the registered callback");
        assert!(callback_url.starts_with(OAUTH_REDIRECT_URI));
        let expected_token = OauthFixture::expected_access_token(&callback_url);

        let session = koushi_sdk::finish_oidc_login(pending, &callback_url)
            .await
            .expect("the authorization code should be exchanged for a session");

        assert_eq!(session.info.homeserver, fixture.homeserver);
        assert_eq!(session.info.user_id, OAUTH_USER_ID);
        assert_eq!(session.info.device_id, OAUTH_DEVICE_ID);
        assert_eq!(
            session.info.authentication_method,
            SessionAuthenticationMethod::OAuth
        );

        let observations = fixture.observations();
        // The code really was exchanged: the fixture minted a token, verified
        // the PKCE verifier against the issued S256 challenge, and saw the
        // token come back on the authenticated `whoami` the SDK performs to
        // authenticate the new session.
        assert_eq!(observations.token_request_count(), 1);
        let issued = observations
            .issued_access_tokens
            .lock()
            .expect("issued access tokens")
            .clone();
        assert_eq!(issued.len(), 1);
        assert!(issued[0] == expected_token, "the fixture mints one token");
        assert!(
            observations
                .pkce_challenge_verified
                .load(std::sync::atomic::Ordering::SeqCst),
            "the token exchange must be PKCE-bound"
        );
        assert!(observations.bearer_tokens().contains(&expected_token));

        // Same allocation: the session owns the client that started the attempt.
        let client = session.client();
        assert_eq!(
            client.oauth().client_id().map(|id| id.as_str().to_owned()),
            Some(OAUTH_CLIENT_ID.to_owned())
        );
        let oauth_session = client
            .oauth()
            .full_session()
            .expect("the client must hold an OAuth session");
        assert_eq!(oauth_session.user.meta.user_id.as_str(), OAUTH_USER_ID);
        assert!(
            oauth_session.user.meta.device_id.as_str() == OAUTH_DEVICE_ID,
            "the OAuth session must carry the requested device"
        );
        assert!(
            oauth_session.user.tokens.access_token == expected_token,
            "the session must hold the token the code exchange minted"
        );

        // The registered redirect URI is the one the fixture accepted and the
        // one the callback came back on.
        let registered = observations
            .registered_redirect_uris
            .lock()
            .expect("registered redirect uris")
            .clone();
        assert_eq!(registered, vec![OAUTH_REDIRECT_URI.to_owned()]);
    });
}

/// A restart restores the same OAuth session from the same persistent store and
/// keeps authenticating with the token the code exchange minted.
#[test]
fn oauth_session_restores_from_the_same_persistent_store() {
    let fixture = OauthFixture::start();
    let store = tempfile::tempdir().expect("oauth store");
    let runtime = runtime();

    runtime.block_on(async {
        let (pending, authorization) = start_oauth(&fixture, store.path(), OAUTH_DEVICE_ID).await;
        let callback_url = fixture
            .authorize(&authorization.authorization_url)
            .expect("authorization should redirect to the registered callback");
        let expected_token = OauthFixture::expected_access_token(&callback_url);
        let session = koushi_sdk::finish_oidc_login(pending, &callback_url)
            .await
            .expect("code exchange");
        let persistable = session.persistable_session().expect("persistable session");
        assert!(persistable.oauth_session().is_some());
        let info = session.info.clone();
        drop(session);

        // Restart: a fresh client on the same store root restores the OAuth
        // session without registering again or exchanging a code again.
        let restored =
            koushi_sdk::restore_session_with_store(&persistable, Some(&store_config(store.path())))
                .await
                .expect("the persisted OAuth session should restore");
        assert_eq!(restored.info, info);
        assert_eq!(
            restored.info.authentication_method,
            SessionAuthenticationMethod::OAuth
        );
        restored
            .client()
            .whoami()
            .await
            .expect("the restored client authenticates with the persisted token");

        let observations = fixture.observations();
        assert_eq!(observations.token_request_count(), 1);
        assert_eq!(observations.registration_request_count(), 1);
        assert!(
            observations
                .bearer_tokens()
                .iter()
                .filter(|token| **token == expected_token)
                .count()
                >= 2,
            "the restored client must reuse the persisted access token"
        );
    });
}

/// A callback with no code or no state never reaches the token endpoint.
#[test]
fn callback_without_code_or_state_is_rejected_without_token_exchange() {
    for callback in [
        format!("{OAUTH_REDIRECT_URI}?state=synthetic-state"),
        format!("{OAUTH_REDIRECT_URI}?code=synthetic-code"),
        OAUTH_REDIRECT_URI.to_owned(),
    ] {
        let fixture = OauthFixture::start();
        let store = tempfile::tempdir().expect("oauth store");
        let runtime = runtime();

        runtime.block_on(async {
            let (pending, _authorization) =
                start_oauth(&fixture, store.path(), OAUTH_DEVICE_ID).await;

            let error = match koushi_sdk::finish_oidc_login(pending, &callback).await {
                Ok(_session) => panic!("callback {callback} must not create a session"),
                Err(error) => error,
            };

            // The load-bearing criterion, observed on the server: the code never
            // reached the token endpoint.
            assert_eq!(
                fixture.observations().token_request_count(),
                0,
                "callback {callback} must not reach the token endpoint"
            );
            let detail = error
                .typed_detail()
                .unwrap_or_else(|| panic!("callback {callback} must fail with a typed detail"));
            assert_eq!(detail.method, AuthMethod::OAuth, "callback {callback}");
            assert_eq!(
                detail.stage,
                AuthFailureStage::OidcCallback,
                "callback {callback}"
            );
            assert!(
                !detail.retryable,
                "callback {callback} must not be retryable"
            );
            // The rejection is local: nothing reached the token endpoint and
            // nothing answered, so the failure must not be reported as a
            // server response or a transport failure that never happened.
            assert_eq!(
                detail.transport,
                AuthFailureTransport::Local,
                "callback {callback}"
            );
            assert_eq!(detail.http_status, None, "callback {callback}");
        });
    }
}

/// A callback whose state the client never minted is rejected before the token
/// endpoint sees the code.
#[test]
fn mismatched_state_is_rejected_without_token_exchange() {
    let fixture = OauthFixture::start();
    let store = tempfile::tempdir().expect("oauth store");
    let runtime = runtime();

    runtime.block_on(async {
        let (pending, authorization) = start_oauth(&fixture, store.path(), OAUTH_DEVICE_ID).await;
        let callback_url = fixture
            .authorize(&authorization.authorization_url)
            .expect("authorization should redirect");
        let tampered = with_state(&callback_url, "synthetic-foreign-state");

        let error = koushi_sdk::finish_oidc_login(pending, &tampered)
            .await
            .expect_err("a mismatched state must not complete the login");

        let detail = error.typed_detail().expect("typed failure detail");
        assert_eq!(detail.method, AuthMethod::OAuth);
        assert_eq!(detail.stage, AuthFailureStage::OidcCallback);
        assert!(!detail.retryable);
        assert_eq!(fixture.observations().token_request_count(), 0);
        assert!(!format!("{error:?}").contains("synthetic-foreign-state"));
    });
}

/// Replaying the already-consumed callback against the same authenticated
/// client fails and does not exchange the code a second time.
#[test]
fn replayed_callback_does_not_exchange_the_code_again() {
    let fixture = OauthFixture::start();
    let store = tempfile::tempdir().expect("oauth store");
    let runtime = runtime();

    runtime.block_on(async {
        let (pending, authorization) = start_oauth(&fixture, store.path(), OAUTH_DEVICE_ID).await;
        let callback_url = fixture
            .authorize(&authorization.authorization_url)
            .expect("authorization should redirect");
        let session = koushi_sdk::finish_oidc_login(pending, &callback_url)
            .await
            .expect("code exchange");
        assert_eq!(fixture.observations().token_request_count(), 1);

        let replay = session
            .client()
            .oauth()
            .finish_login(matrix_sdk::utils::UrlOrQuery::Url(
                callback_url.parse().expect("callback URL"),
            ))
            .await
            .expect_err("a replayed callback must not re-authenticate");
        assert!(
            !replay
                .to_string()
                .contains(&code_from_callback(&callback_url))
        );
        assert_eq!(fixture.observations().token_request_count(), 1);
        assert_eq!(
            session.info.authentication_method,
            SessionAuthenticationMethod::OAuth
        );
    });
}

/// A stale authorization from a replaced browser attempt cannot complete
/// against the replacement allocation, and nothing is exchanged.
#[test]
fn stale_authorization_from_a_replaced_attempt_cannot_complete() {
    let fixture = OauthFixture::start();
    let first_store = tempfile::tempdir().expect("first oauth store");
    let second_store = tempfile::tempdir().expect("second oauth store");
    let runtime = runtime();

    runtime.block_on(async {
        let (first_pending, first_authorization) =
            start_oauth(&fixture, first_store.path(), "OAUTHFIRSTDEVICE").await;
        let stale_callback = fixture
            .authorize(&first_authorization.authorization_url)
            .expect("first authorization should redirect");
        // The user retries: a fresh allocation replaces the retained attempt.
        let (second_pending, second_authorization) =
            start_oauth(&fixture, second_store.path(), "OAUTHSECONDDEVICE").await;
        assert_ne!(first_authorization.state, second_authorization.state);
        drop(first_pending);

        let error = koushi_sdk::finish_oidc_login(second_pending, &stale_callback)
            .await
            .expect_err("a stale callback must not complete the replacement attempt");

        let detail = error.typed_detail().expect("typed failure detail");
        assert_eq!(detail.stage, AuthFailureStage::OidcCallback);
        assert_eq!(fixture.observations().token_request_count(), 0);
    });
}

/// A rejected token endpoint is a typed, redacted, non-retryable failure and
/// leaves no session behind.
#[test]
fn token_endpoint_rejection_is_a_typed_redacted_failure() {
    let fixture = OauthFixture::start();
    let store = tempfile::tempdir().expect("oauth store");
    let runtime = runtime();

    runtime.block_on(async {
        let (pending, authorization) = start_oauth(&fixture, store.path(), OAUTH_DEVICE_ID).await;
        let callback_url = fixture
            .authorize(&authorization.authorization_url)
            .expect("authorization should redirect");
        fixture.set_token_exchange_mode(TokenExchangeMode::Reject);

        let error = koushi_sdk::finish_oidc_login(pending, &callback_url)
            .await
            .expect_err("a rejected token exchange must not create a session");

        let detail = error.typed_detail().expect("typed failure detail");
        assert_eq!(detail.method, AuthMethod::OAuth);
        assert_eq!(detail.stage, AuthFailureStage::OidcCallback);
        assert_eq!(detail.transport, AuthFailureTransport::HttpResponse);
        assert!(!detail.retryable);
        assert_eq!(fixture.observations().token_request_count(), 1);
        let debug = format!("{error:?}");
        assert!(!debug.contains(&code_from_callback(&callback_url)));
        assert!(!debug.contains(&state_from_callback(&callback_url)));
        assert!(!debug.contains("synthetic rejection"));
        assert!(!debug.contains("synthetic-access"));
    });
}

/// A token endpoint that never answers is a typed, retryable transport failure.
#[test]
fn token_transport_failure_is_a_typed_retryable_failure() {
    let fixture = OauthFixture::start();
    let store = tempfile::tempdir().expect("oauth store");
    let runtime = runtime();

    runtime.block_on(async {
        let (pending, authorization) = start_oauth(&fixture, store.path(), OAUTH_DEVICE_ID).await;
        let callback_url = fixture
            .authorize(&authorization.authorization_url)
            .expect("authorization should redirect");
        fixture.set_token_exchange_mode(TokenExchangeMode::DropConnection);

        let error = koushi_sdk::finish_oidc_login(pending, &callback_url)
            .await
            .expect_err("a transport failure must not create a session");

        let detail = error.typed_detail().expect("typed failure detail");
        assert_eq!(detail.method, AuthMethod::OAuth);
        assert_eq!(detail.stage, AuthFailureStage::OidcCallback);
        assert_eq!(detail.transport, AuthFailureTransport::NoResponse);
        assert!(detail.retryable);
        assert_eq!(fixture.observations().token_request_count(), 1);
        assert!(!format!("{error:?}").contains(&code_from_callback(&callback_url)));
    });
}

// ---------------------------------------------------------------------------
// Core admission: the real runtime, the real store, the real restart.
// ---------------------------------------------------------------------------

async fn submit_oauth_start(
    connection: &mut CoreConnection,
    homeserver: &str,
) -> (koushi_core::RequestId, String, String) {
    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Account(AccountCommand::StartOidcLogin {
            request_id,
            homeserver: homeserver.to_owned(),
            method: DelegatedAuthMethod::OAuth,
        }))
        .await
        .expect("submit OAuth start");

    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match connection.recv_event().await.expect("core event stream") {
                CoreEvent::Account(AccountEvent::OidcAuthorizationCreated {
                    request_id: event_id,
                    authorization_url,
                    state,
                }) if event_id == request_id => return (request_id, authorization_url, state),
                CoreEvent::OperationFailed {
                    request_id: event_id,
                    failure,
                } if event_id == request_id => panic!("OAuth start failed: {failure:?}"),
                _ => {}
            }
        }
    })
    .await
    .expect("OAuth authorization should be created")
}

async fn submit_oauth_callback(
    connection: &CoreConnection,
    callback_url: &str,
) -> koushi_core::RequestId {
    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Account(AccountCommand::CompleteOidcLogin {
            request_id,
            callback_url: callback_url.to_owned(),
            platform: DisplayPlatform::Linux,
        }))
        .await
        .expect("submit OAuth callback");
    request_id
}

/// The end-to-end proof: real authorization creation, real callback, real code
/// exchange through the SDK, real Core admission, and a real restart/restore
/// that reuses the persisted OAuth session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn core_admits_a_real_oauth_exchange_and_restores_it_after_restart() {
    let fixture = OauthFixture::start();
    let observations = Arc::clone(fixture.observations());
    let data_dir = tempfile::tempdir().expect("runtime data dir");
    let credential_dir = tempfile::tempdir().expect("runtime credential dir");
    let data_path = data_dir.path().to_path_buf();
    let credential_path = credential_dir.path().to_path_buf();
    let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_path.clone(),
        credential_path.clone(),
    );
    assert!(
        runtime
            .configure_trust_observation_for_testing(koushi_sdk::CurrentDeviceTrustObservation {
                current: koushi_state::CurrentDeviceTrustState::Verified,
                updates: Box::pin(futures_util::stream::pending()),
            })
            .await
    );
    let mut connection = runtime.attach();

    let (start_request_id, authorization_url, state) =
        submit_oauth_start(&mut connection, &fixture.homeserver).await;
    assert!(
        !state.is_empty(),
        "Core must expose a nonempty CSRF state to the adapter"
    );
    let callback_url = fixture
        .authorize(&authorization_url)
        .expect("the production authorization URL must be acceptable to the server");
    assert!(
        state_from_callback(&callback_url) == state,
        "the callback must carry the state Core minted"
    );
    let expected_token = OauthFixture::expected_access_token(&callback_url);
    let completion_request_id = submit_oauth_callback(&connection, &callback_url).await;
    assert_ne!(completion_request_id, start_request_id);

    let state = wait_for_ready_account(&mut connection, OAUTH_USER_ID).await;
    let SessionState::Ready(info) = &state.session else {
        panic!("the OAuth login never reached a ready session");
    };
    assert_eq!(info.homeserver, fixture.homeserver);
    assert_eq!(info.user_id, OAUTH_USER_ID);
    assert_eq!(
        info.authentication_method,
        SessionAuthenticationMethod::OAuth,
        "Core must record the OAuth authentication method, not a password login"
    );
    // Device/store identity: the device the SDK authenticated with is the one
    // Core's pending allocation created, and the same one it now serves.
    let authorized_device = observations
        .authorization_device_ids
        .lock()
        .expect("authorization device ids")
        .last()
        .cloned()
        .expect("an authorization device id");
    assert_eq!(info.device_id, authorized_device);

    // The account-management surface Core drives after admission reached the
    // device Core admitted, with the platform's OAuth device name.
    let renames = observations
        .device_renames
        .lock()
        .expect("device renames")
        .clone();
    assert!(!renames.is_empty(), "Core must name the admitted device");
    assert!(
        renames.iter().all(|(device, _)| *device == info.device_id),
        "Core must only ever name the device the OAuth flow authenticated"
    );
    assert!(
        renames
            .iter()
            .any(|(_, name)| name == DisplayPlatform::Linux.oauth_device_display_name())
    );

    assert_eq!(observations.token_request_count(), 1);
    assert_eq!(observations.registration_request_count(), 1);
    assert!(observations.bearer_tokens().contains(&expected_token));
    drop(connection);
    runtime.shutdown().await;

    // Restart on the same directories: the persisted OAuth session must be
    // restored and used, without a second registration or code exchange.
    let restarted =
        CoreRuntime::start_with_data_dir_and_file_credentials(data_path, credential_path);
    assert!(
        restarted
            .configure_trust_observation_for_testing(koushi_sdk::CurrentDeviceTrustObservation {
                current: koushi_state::CurrentDeviceTrustState::Verified,
                updates: Box::pin(futures_util::stream::pending()),
            })
            .await
    );
    let mut connection = restarted.attach();
    // Everything the restarted runtime sends is counted from here on, so the
    // reuse of the persisted token cannot be credited to the old runtime.
    let bearers_before_restart = observations.bearer_tokens().len();
    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Account(AccountCommand::RestoreLastSession {
            request_id,
        }))
        .await
        .expect("submit restore");
    let restored = wait_for_ready_account(&mut connection, OAUTH_USER_ID).await;
    let SessionState::Ready(restored_restored_info) = &restored.session else {
        panic!("the persisted session never restored");
    };
    assert_eq!(restored_restored_info, info);
    assert_eq!(
        restored_restored_info.authentication_method,
        SessionAuthenticationMethod::OAuth
    );

    assert_eq!(
        observations.token_request_count(),
        1,
        "restore must reuse the persisted OAuth session, not exchange a new code"
    );
    assert_eq!(
        observations.registration_request_count(),
        1,
        "restore must reuse the persisted OAuth client registration"
    );
    assert!(
        observations
            .wait_for_bearer_token_after(
                bearers_before_restart,
                &expected_token,
                RESTORE_TOKEN_REUSE_TIMEOUT
            )
            .await,
        "the restored runtime must authenticate with the token the code exchange minted"
    );
    drop(connection);
    restarted.shutdown().await;
}

/// The post-login verification gate holds the OAuth session: the exchange
/// admits the account, but the authenticated session is not promoted until the
/// device is verified.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oauth_login_is_admitted_but_held_until_the_device_is_verified() {
    let fixture = OauthFixture::start();
    let (runtime, stores) = CoreRuntime::start_isolated();
    let mut connection = runtime.attach();

    let (start_request_id, authorization_url, _state) =
        submit_oauth_start(&mut connection, &fixture.homeserver).await;
    let callback_url = fixture
        .authorize(&authorization_url)
        .expect("authorization should redirect");
    let completion_request_id = submit_oauth_callback(&connection, &callback_url).await;
    assert_ne!(completion_request_id, start_request_id);

    let admitted = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match connection.recv_event().await.expect("core event stream") {
                CoreEvent::Account(AccountEvent::LoginAdmitted { account_key, .. }) => {
                    return account_key;
                }
                CoreEvent::OperationFailed { failure, .. } => {
                    panic!("OAuth login failed: {failure:?}")
                }
                _ => {}
            }
        }
    })
    .await
    .expect("the exchanged OAuth session must be admitted");
    assert!(admitted.0 == OAUTH_USER_ID);

    let logged_in = tokio::time::timeout(Duration::from_millis(400), async {
        loop {
            match connection.recv_event().await.expect("core event stream") {
                CoreEvent::Account(AccountEvent::LoggedIn { .. }) => return,
                CoreEvent::OperationFailed { failure, .. } => {
                    panic!("OAuth login failed: {failure:?}")
                }
                _ => {}
            }
        }
    })
    .await;
    assert!(
        logged_in.is_err(),
        "an unverified device must not promote the OAuth session to LoggedIn"
    );

    drop(connection);
    runtime.shutdown().await;
    drop(stores);
}

async fn wait_for_ready_account(
    connection: &mut CoreConnection,
    user_id: &str,
) -> koushi_state::AppState {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let snapshot = connection.snapshot();
            if matches!(
                &snapshot.session,
                SessionState::Ready(info) if info.user_id == user_id
            ) {
                return snapshot;
            }
            connection
                .recv_event()
                .await
                .expect("core event stream must remain open");
        }
    })
    .await
    .unwrap_or_else(|_| {
        let snapshot = connection.snapshot();
        panic!(
            "the OAuth session never reached Ready; session: {:?}; capability: {:?}",
            snapshot.session, snapshot.sliding_sync_capability
        )
    })
}
