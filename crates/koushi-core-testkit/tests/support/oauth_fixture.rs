//! Disposable OAuth 2.0 / MSC3861-compatible authorization server and Matrix
//! homeserver fixture for the OAuth sign-in proof (#1326).
//!
//! This is a real TCP/HTTP server bound to `127.0.0.1:0`. The production code
//! under test makes real HTTP requests to it: authorization-server discovery
//! (`/_matrix/client/v1/auth_metadata`), dynamic client registration, the
//! authorization-code redirect, the token exchange over a real socket, and the
//! authenticated Matrix requests (`whoami`, `devices`, `keys/*`, sliding sync)
//! that admission and restore touch. Nothing is stubbed inside the SDK or
//! inside `koushi-core`; the only fixture is the server side of the wire.
//!
//! Every identity, code, token, key, and nonce below is synthetic and
//! private-data-free. No request body, code, verifier, state, or token is ever
//! logged: the observation log stores method plus path only.

#![allow(dead_code)]

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

/// The synthetic public client the fixture registers through RFC 7591 dynamic
/// client registration.
pub const OAUTH_CLIENT_ID: &str = "synthetic-oauth-client";
/// The synthetic Matrix user the exchanged access token authenticates as.
pub const OAUTH_USER_ID: &str = "@fixture-oauth-user:example.invalid";
/// The desktop's production OAuth redirect URI (`OIDC_REDIRECT_URI`).
pub const OAUTH_REDIRECT_URI: &str = "com.github.shinaoka.koushi-matrix:/auth/callback";

/// How the fixture answers the token endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenExchangeMode {
    /// Mint an access token bound to the code that was authorized.
    Issue,
    /// Reject the exchange with an RFC 6749 error response.
    Reject,
    /// Close the connection without a response (a transport failure).
    DropConnection,
}

/// What the fixture observed while serving the flow.
///
/// This is the server-side oracle: it proves which requests actually reached
/// the wire, in which order, and which secrets they carried.
#[derive(Default)]
pub struct OauthObservations {
    pub auth_metadata_requests: AtomicUsize,
    pub registration_requests: AtomicUsize,
    pub authorization_requests: AtomicUsize,
    pub token_requests: AtomicUsize,
    pub whoami_requests: AtomicUsize,
    pub versions_requests: AtomicUsize,
    /// `METHOD path` only, with the query string stripped.
    pub request_log: Mutex<Vec<String>>,
    /// Redirect URIs presented to the registration endpoint.
    pub registered_redirect_uris: Mutex<Vec<String>>,
    /// CSRF states minted by the authorization endpoint.
    pub authorization_states: Mutex<Vec<String>>,
    /// Device IDs carried in the authorization request scope.
    pub authorization_device_ids: Mutex<Vec<String>>,
    /// Authorization requests the fixture refused, with the refusal reason.
    pub rejected_authorizations: Mutex<Vec<String>>,
    /// Access tokens the token endpoint minted.
    pub issued_access_tokens: Mutex<Vec<String>>,
    /// Every bearer token presented on an authenticated Matrix request.
    pub bearer_tokens: Mutex<Vec<String>>,
    /// `(device_id, display_name)` written through `PUT /devices/{id}`.
    pub device_renames: Mutex<Vec<(String, String)>>,
    /// True once a token request presented a nonempty PKCE verifier.
    pub saw_code_verifier: AtomicBool,
    /// True once a code verifier was proven to derive the issued S256
    /// challenge, i.e. the exchange really was PKCE-bound.
    pub pkce_challenge_verified: AtomicBool,
}

impl OauthObservations {
    pub fn token_request_count(&self) -> usize {
        self.token_requests.load(Ordering::SeqCst)
    }

    pub fn registration_request_count(&self) -> usize {
        self.registration_requests.load(Ordering::SeqCst)
    }

    pub fn paths(&self) -> Vec<String> {
        self.request_log.lock_or_poison().clone()
    }

    pub fn bearer_tokens(&self) -> Vec<String> {
        self.bearer_tokens.lock_or_poison().clone()
    }

    /// Waits, up to `timeout`, for an authenticated request carrying `token` to
    /// be observed at index `start` or later, and reports whether one arrived.
    ///
    /// A restarted runtime restores its session from disk and reports it ready
    /// before the restored client's first request reaches the network, so a
    /// proof that the persisted token was really reused has to wait for that
    /// request instead of assuming it has already landed. `start` keeps the
    /// proof to requests the restored runtime itself made, so reuse cannot be
    /// credited to the runtime that has already been shut down.
    pub async fn wait_for_bearer_token_after(
        &self,
        start: usize,
        token: &str,
        timeout: Duration,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let observed = self
                .bearer_tokens
                .lock_or_poison()
                .get(start..)
                .is_some_and(|rest| rest.iter().any(|observed| observed == token));
            if observed {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

/// One authorized code, bound to the CSRF state, redirect URI, PKCE challenge,
/// client, and device the SDK presented at the authorization endpoint.
struct CodeBinding {
    code: String,
    state: String,
    code_challenge: String,
    redirect_uri: String,
    device_id: String,
}

pub struct OauthFixture {
    pub homeserver: String,
    pub authorization_endpoint: String,
    observations: Arc<OauthObservations>,
    token_mode: Arc<Mutex<TokenExchangeMode>>,
}

impl OauthFixture {
    /// Start the disposable server. The listener is owned by a background
    /// thread that ends when the process ends.
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("oauth fixture should bind");
        let addr = listener.local_addr().expect("oauth fixture address");
        let origin = format!("http://{addr}");

        let observations = Arc::new(OauthObservations::default());
        let token_mode = Arc::new(Mutex::new(TokenExchangeMode::Issue));
        let state = Arc::new(ServerState {
            origin: origin.clone(),
            observations: Arc::clone(&observations),
            token_mode: Arc::clone(&token_mode),
            codes: Mutex::new(Vec::new()),
            registered_client: Mutex::new(None),
            current_device_id: Mutex::new("FIXTUREOAUTHDEVICE".to_owned()),
        });

        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let state = Arc::clone(&state);
                std::thread::spawn(move || serve(stream, state));
            }
        });

        Self {
            homeserver: origin.clone(),
            authorization_endpoint: format!("{origin}/oauth2/authorize"),
            observations,
            token_mode,
        }
    }

    pub fn observations(&self) -> &Arc<OauthObservations> {
        &self.observations
    }

    pub fn set_token_exchange_mode(&self, mode: TokenExchangeMode) {
        *self.token_mode.lock_or_poison() = mode;
    }

    /// Play the browser/OS half of the flow: fetch the authorization URL the
    /// production code handed to the UI, exactly as an installed-app browser
    /// would, and follow the redirect back to the desktop's registered
    /// callback URI.
    ///
    /// Returns the callback URL (the OS deep link) or the fixture's refusal
    /// reason, so a malformed authorization request can never be mistaken for
    /// a completed sign-in.
    pub fn authorize(&self, authorization_url: &str) -> Result<String, String> {
        let (status, headers, body) = raw_get(authorization_url);
        if status != 302 {
            return Err(format!(
                "authorization endpoint refused the request ({status}): {body}"
            ));
        }
        let location = headers
            .into_iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("location"))
            .map(|(_, value)| value)
            .ok_or_else(|| "authorization response carried no redirect Location".to_owned())?;
        Ok(location)
    }

    /// The access token the fixture minted for the code in `callback_url`.
    pub fn expected_access_token(callback_url: &str) -> String {
        format!("synthetic-access-{}", code_from_callback(callback_url))
    }
}

struct ServerState {
    origin: String,
    observations: Arc<OauthObservations>,
    token_mode: Arc<Mutex<TokenExchangeMode>>,
    codes: Mutex<Vec<CodeBinding>>,
    registered_client: Mutex<Option<String>>,
    current_device_id: Mutex<String>,
}

/// A lock that survives a poisoned mutex, so a failed test can never turn a
/// fixture thread's lock into a non-unwinding panic at process exit.
trait LockExt<T> {
    fn lock_or_poison(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T> LockExt<T> for Mutex<T> {
    fn lock_or_poison(&self) -> std::sync::MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn serve(mut stream: TcpStream, state: Arc<ServerState>) {
    let Some(request) = read_request(&mut stream) else {
        return;
    };
    let (method, target) = request_line(&request);
    let body = request_body(&request);
    let path = target
        .split('?')
        .next()
        .unwrap_or(target.as_str())
        .to_owned();
    state
        .observations
        .request_log
        .lock_or_poison()
        .push(format!("{method} {path}"));

    let response = route(&method, &target, body, &state);
    match response {
        Some(response) => {
            let Response {
                status,
                location,
                body,
            } = response;
            // Only the whoami/devices/keys/sync surface is authenticated; the
            // fixture records the bearer token it was presented.
            if let Some(token) = bearer_token(&request) {
                state
                    .observations
                    .bearer_tokens
                    .lock_or_poison()
                    .push(token);
            }
            let location = location
                .as_deref()
                .map(|location| format!("Location: {location}\r\n"))
                .unwrap_or_default();
            let payload = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{location}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(payload.as_bytes());
        }
        None => {
            // Deliberate transport failure: close without a response.
        }
    }
    let _ = stream.flush();
    drop(stream);
}

struct Response {
    status: &'static str,
    location: Option<String>,
    body: String,
}

impl Response {
    fn ok(body: impl Into<String>) -> Option<Response> {
        Some(Response {
            status: "200 OK",
            location: None,
            body: body.into(),
        })
    }

    /// The installed-app redirect: the OS deep link is the `Location` header.
    fn redirect(location: &str) -> Option<Response> {
        Some(Response {
            status: "302 Found",
            location: Some(location.to_owned()),
            body: "{}".to_owned(),
        })
    }
}

fn route(method: &str, target: &str, body: &str, state: &ServerState) -> Option<Response> {
    let path = target.split('?').next().unwrap_or(target);
    if path == "/_matrix/client/versions" {
        state
            .observations
            .versions_requests
            .fetch_add(1, Ordering::SeqCst);
        return Response::ok(
            r#"{"versions":["v1.7"],"unstable_features":{"org.matrix.simplified_msc3575":true}}"#,
        );
    }
    if path == "/_matrix/client/v1/auth_metadata"
        || path == "/_matrix/client/unstable/org.matrix.msc2965/auth_metadata"
    {
        state
            .observations
            .auth_metadata_requests
            .fetch_add(1, Ordering::SeqCst);
        return Response::ok(auth_metadata(&state.origin));
    }
    if path == "/oauth2/register" {
        return register_client(body, state);
    }
    if path == "/oauth2/authorize" {
        return authorize(target, state);
    }
    if path == "/oauth2/token" {
        // Counted here, at the routing boundary: the oracle must observe a
        // request even when the fixture refuses it.
        state
            .observations
            .token_requests
            .fetch_add(1, Ordering::SeqCst);
        return exchange_code(body, state);
    }
    if path == "/_matrix/client/v3/account/whoami" {
        state
            .observations
            .whoami_requests
            .fetch_add(1, Ordering::SeqCst);
        return Response::ok(format!(r#"{{"user_id":"{OAUTH_USER_ID}"}}"#));
    }
    if path == "/_matrix/client/v3/devices" && method == "GET" {
        let device_id = state.current_device_id.lock_or_poison().clone();
        // An unnamed device, so admission must name *this* device.
        return Response::ok(format!(
            r#"{{"devices":[{{"device_id":"{device_id}","display_name":""}}]}}"#
        ));
    }
    if method == "PUT" && path.starts_with("/_matrix/client/v3/devices/") {
        let device_id = path
            .trim_start_matches("/_matrix/client/v3/devices/")
            .to_owned();
        let display_name = serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|body| body["display_name"].as_str().map(str::to_owned))
            .unwrap_or_default();
        state
            .observations
            .device_renames
            .lock_or_poison()
            .push((device_id, display_name));
        return Response::ok("{}");
    }
    if path.contains("/keys/upload") {
        return Response::ok(r#"{"one_time_key_counts":{}}"#);
    }
    if path.contains("/keys/query") {
        return Response::ok(r#"{"device_keys":{},"failures":{}}"#);
    }
    if path.contains("/room_keys/version") {
        return Some(Response {
            status: "404 Not Found",
            location: None,
            body: r#"{"errcode":"M_NOT_FOUND","error":"No current backup version"}"#.to_owned(),
        });
    }
    if path.contains("/sync") {
        // The sync engine's exact frame is not the subject of this proof; the
        // request itself (and its bearer token) is.
        return Response::ok(
            r#"{"next_batch":"synthetic-batch","pos":"synthetic-batch","rooms":{},"lists":{},"extensions":{"to_device":{"events":[]}},"device_one_time_keys_count":{}}"#,
        );
    }
    if path == "/_matrix/client/v3/login" {
        // OAuth-only discovery: the homeserver advertises delegated OIDC
        // through the legacy SSO flow and nothing else.
        return Response::ok(
            r#"{"flows":[{"type":"m.login.sso","org.matrix.msc3824.delegated_oidc_compatibility":true}]}"#,
        );
    }
    Some(Response {
        status: "404 Not Found",
        location: None,
        body: r#"{"errcode":"M_NOT_FOUND","error":"not found"}"#.to_owned(),
    })
}

fn auth_metadata(origin: &str) -> String {
    format!(
        r#"{{"issuer":"{origin}","authorization_endpoint":"{origin}/oauth2/authorize","token_endpoint":"{origin}/oauth2/token","registration_endpoint":"{origin}/oauth2/register","revocation_endpoint":"{origin}/oauth2/revoke","response_types_supported":["code"],"response_modes_supported":["query","fragment"],"grant_types_supported":["authorization_code","refresh_token"],"code_challenge_methods_supported":["S256"],"token_endpoint_auth_methods_supported":["none"]}}"#
    )
}

fn register_client(body: &str, state: &ServerState) -> Option<Response> {
    state
        .observations
        .registration_requests
        .fetch_add(1, Ordering::SeqCst);
    let metadata: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let redirect_uris = metadata["redirect_uris"].clone();
    let registered = redirect_uris
        .as_array()
        .map(|uris| {
            uris.iter()
                .filter_map(|uri| uri.as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !registered.iter().any(|uri| uri == OAUTH_REDIRECT_URI) {
        return Some(Response {
            status: "400 Bad Request",
            location: None,
            body: r#"{"error":"invalid_redirect_uri"}"#.to_owned(),
        });
    }
    *state.registered_client.lock_or_poison() = Some(OAUTH_CLIENT_ID.to_owned());
    state
        .observations
        .registered_redirect_uris
        .lock_or_poison()
        .extend(registered);
    Response::ok(format!(r#"{{"client_id":"{OAUTH_CLIENT_ID}"}}"#))
}

fn authorize(target: &str, state: &ServerState) -> Option<Response> {
    state
        .observations
        .authorization_requests
        .fetch_add(1, Ordering::SeqCst);
    let Some(query) = target.split_once('?').map(|(_, query)| query) else {
        return refuse(state, "missing_query");
    };
    let params = parse_form(query);
    let registered_client = state.registered_client.lock_or_poison().clone();
    let client_id = params.get("client_id").cloned().unwrap_or_default();
    if registered_client.as_deref() != Some(client_id.as_str()) {
        return refuse(state, "unregistered_client");
    }
    if params.get("response_type").map(String::as_str) != Some("code") {
        return refuse(state, "unsupported_response_type");
    }
    let redirect_uri = params.get("redirect_uri").cloned().unwrap_or_default();
    if redirect_uri != OAUTH_REDIRECT_URI {
        return refuse(state, "unregistered_redirect_uri");
    }
    let registered_redirect = state
        .observations
        .registered_redirect_uris
        .lock_or_poison()
        .clone();
    if !registered_redirect.contains(&redirect_uri.to_owned()) {
        return refuse(state, "redirect_uri_not_registered");
    }
    let state_param = params.get("state").cloned().unwrap_or_default();
    if state_param.is_empty() {
        return refuse(state, "missing_state");
    }
    if params.get("code_challenge_method").map(String::as_str) != Some("S256") {
        return refuse(state, "missing_s256_challenge");
    }
    let code_challenge = params.get("code_challenge").cloned().unwrap_or_default();
    if code_challenge.is_empty() {
        return refuse(state, "missing_code_challenge");
    }
    let device_id = device_id_from_scope(params.get("scope").map(String::as_str).unwrap_or(""))
        .unwrap_or_else(|| "FIXTUREOAUTHDEVICE".to_owned());
    *state.current_device_id.lock_or_poison() = device_id.clone();

    let code = {
        let mut codes = state.codes.lock_or_poison();
        let code = format!("synthetic-code-{}", codes.len() + 1);
        codes.push(CodeBinding {
            code: code.clone(),
            state: state_param.clone(),
            code_challenge,
            redirect_uri: redirect_uri.clone(),
            device_id: device_id.clone(),
        });
        code
    };
    state
        .observations
        .authorization_states
        .lock_or_poison()
        .push(state_param.clone());
    state
        .observations
        .authorization_device_ids
        .lock_or_poison()
        .push(device_id);

    let callback = format!("{redirect_uri}?code={code}&state={state_param}");
    Response::redirect(&callback)
}

fn refuse(state: &ServerState, reason: &str) -> Option<Response> {
    state
        .observations
        .rejected_authorizations
        .lock_or_poison()
        .push(reason.to_owned());
    Some(Response {
        status: "400 Bad Request",
        location: None,
        body: format!(r#"{{"error":"{reason}"}}"#),
    })
}

fn exchange_code(body: &str, state: &ServerState) -> Option<Response> {
    match *state.token_mode.lock_or_poison() {
        TokenExchangeMode::DropConnection => return None,
        TokenExchangeMode::Reject => {
            return Some(Response {
                status: "400 Bad Request",
                location: None,
                body: r#"{"error":"invalid_grant","error_description":"synthetic rejection"}"#
                    .to_owned(),
            });
        }
        TokenExchangeMode::Issue => {}
    }

    let params = parse_form(body);
    if params.get("grant_type").map(String::as_str) != Some("authorization_code") {
        return rejected_token("unsupported_grant_type");
    }
    let code = params.get("code").cloned().unwrap_or_default();
    let verifier = params.get("code_verifier").cloned().unwrap_or_default();
    if !verifier.is_empty() {
        state
            .observations
            .saw_code_verifier
            .store(true, Ordering::SeqCst);
    }
    let redirect_uri = params.get("redirect_uri").cloned().unwrap_or_default();

    let mut codes = state.codes.lock_or_poison();
    let Some(binding) = codes.iter().find(|binding| binding.code == code) else {
        return rejected_token("invalid_grant");
    };
    if verifier.is_empty() {
        return rejected_token("invalid_request");
    }
    if s256_code_challenge(&verifier) == binding.code_challenge {
        state
            .observations
            .pkce_challenge_verified
            .store(true, Ordering::SeqCst);
    } else {
        return rejected_token("invalid_grant_pkce");
    }
    if redirect_uri != binding.redirect_uri {
        return rejected_token("invalid_grant_redirect");
    }
    let access_token = format!("synthetic-access-{}", binding.code);
    let device_id = binding.device_id.clone();
    codes.retain(|binding| binding.code != code);
    drop(codes);

    state
        .observations
        .issued_access_tokens
        .lock_or_poison()
        .push(access_token.clone());
    Response::ok(format!(
        r#"{{"access_token":"{access_token}","token_type":"Bearer","refresh_token":"synthetic-refresh-token","expires_in":3600,"device_id":"{device_id}"}}"#
    ))
}

fn rejected_token(reason: &str) -> Option<Response> {
    Some(Response {
        status: "400 Bad Request",
        location: None,
        body: format!(r#"{{"error":"{reason}"}}"#),
    })
}

fn device_id_from_scope(scope: &str) -> Option<String> {
    scope
        .split_whitespace()
        .find_map(|value| value.strip_prefix("urn:matrix:org.matrix.msc2967.client:device:"))
        .or_else(|| {
            scope
                .split_whitespace()
                .find_map(|value| value.strip_prefix("urn:matrix:client:device:"))
        })
        .map(str::to_owned)
}

/// The `code` query parameter of a callback URL, without any URL crate.
pub fn code_from_callback(callback_url: &str) -> String {
    let query = callback_url
        .split_once('?')
        .map(|(_, query)| query)
        .unwrap_or("");
    parse_form(query).get("code").cloned().unwrap_or_default()
}

/// The `state` query parameter of a callback URL.
pub fn state_from_callback(callback_url: &str) -> String {
    let query = callback_url
        .split_once('?')
        .map(|(_, query)| query)
        .unwrap_or("");
    parse_form(query).get("state").cloned().unwrap_or_default()
}

/// Replace the `state` parameter of a callback URL, modelling a callback that
/// carries a state the client never minted.
pub fn with_state(callback_url: &str, state: &str) -> String {
    let (base, query) = callback_url
        .split_once('?')
        .expect("callback URL should carry a query");
    let mut params = parse_form(query);
    params.insert("state".to_owned(), state.to_owned());
    let rebuilt = params
        .into_pairs()
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    format!("{base}?{rebuilt}")
}

fn request_line(request: &str) -> (String, String) {
    let mut parts = request
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace();
    (
        parts.next().unwrap_or_default().to_owned(),
        parts.next().unwrap_or_default().to_owned(),
    )
}

fn request_body(request: &str) -> &str {
    request
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .unwrap_or("")
}

fn bearer_token(request: &str) -> Option<String> {
    request.lines().find_map(|line| {
        line.strip_prefix("authorization: ")
            .or_else(|| line.strip_prefix("Authorization: "))
            .and_then(|value| value.strip_prefix("Bearer "))
            .map(str::to_owned)
    })
}

fn read_request(stream: &mut TcpStream) -> Option<String> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => buffer.extend_from_slice(&chunk[..count]),
            Err(_) => break,
        }
        let text = String::from_utf8_lossy(&buffer);
        if let Some(header_end) = text.find("\r\n\r\n") {
            // HTTP header names are case-insensitive; hyper writes them
            // lowercase on the wire.
            let length = text[..header_end]
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if buffer.len() >= header_end + 4 + length {
                break;
            }
        }
        if buffer.len() > (1 << 20) {
            break;
        }
    }
    Some(String::from_utf8_lossy(&buffer).into_owned())
}

/// Issue one raw `GET` and return `(status, headers, body)`.
fn raw_get(url: &str) -> (u16, Vec<(String, String)>, String) {
    let rest = url.strip_prefix("http://").expect("fixture URLs are http");
    let (authority, target) = rest.split_once('/').expect("fixture URLs carry a path");
    let mut stream = TcpStream::connect(authority).expect("authorization endpoint connection");
    let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
    let request = format!(
        "GET /{target} HTTP/1.1\r\nHost: {authority}\r\nAccept: */*\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .expect("authorization request");
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    let (head, body) = response
        .split_once("\r\n\r\n")
        .expect("authorization response should carry a body separator");
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(0);
    let headers = lines
        .filter_map(|line| line.split_once(": "))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect::<Vec<_>>();
    let body = body
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_owned())
        .unwrap_or_else(|| body.to_owned());
    (status, headers, body.trim().to_owned())
}

/// Ordered form/query parameters.
#[derive(Default)]
struct Params(Vec<(String, String)>);

impl Params {
    fn get(&self, key: &str) -> Option<&String> {
        self.0
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }

    fn insert(&mut self, key: String, value: String) {
        match self.0.iter_mut().find(|(name, _)| *name == key) {
            Some(entry) => entry.1 = value,
            None => self.0.push((key, value)),
        }
    }

    fn into_pairs(self) -> Vec<(String, String)> {
        self.0
    }
}

fn parse_form(input: &str) -> Params {
    let mut params = Params::default();
    for pair in input.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        params.0.push((percent_decode(key), percent_decode(value)));
    }
    params
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        decoded.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        decoded.push(bytes[index]);
                        index += 1;
                    }
                }
            }
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// RFC 7636 `S256`: `BASE64URL-ENCODE(SHA256(ASCII(code_verifier)))`.
///
/// The digest is implemented here so that the fixture can prove the exchange
/// really was PKCE-bound without adding a dependency to the test crate. The
/// implementation is checked against the RFC 7636 appendix B vector by
/// `oauth_fixture_verifies_the_rfc7636_s256_vector` below.
pub fn s256_code_challenge(verifier: &str) -> String {
    base64_url_no_pad(&sha256(verifier.as_bytes()))
}

fn sha256(input: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let mut message = input.to_vec();
    let bit_length = (message.len() as u64) * 8;
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_length.to_be_bytes());

    for block in message.chunks_exact(64) {
        let mut w = [0_u32; 64];
        for (index, word) in block.chunks_exact(4).enumerate() {
            w[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut digest = [0_u8; 32];
    for (index, word) in h.iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

fn base64_url_no_pad(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut encoded = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let bytes = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let triple = ((bytes[0] as u32) << 16) | ((bytes[1] as u32) << 8) | bytes[2] as u32;
        encoded.push(ALPHABET[((triple >> 18) & 0x3f) as usize] as char);
        encoded.push(ALPHABET[((triple >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            encoded.push(ALPHABET[((triple >> 6) & 0x3f) as usize] as char);
        }
        if chunk.len() > 2 {
            encoded.push(ALPHABET[(triple & 0x3f) as usize] as char);
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oauth_fixture_verifies_the_rfc7636_s256_vector() {
        // RFC 7636 appendix B.
        assert_eq!(
            s256_code_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn oauth_fixture_percent_decodes_callback_urls() {
        let callback = format!("{OAUTH_REDIRECT_URI}?code=synthetic-code-1&state=synthetic-state");
        assert_eq!(code_from_callback(&callback), "synthetic-code-1");
        assert_eq!(state_from_callback(&callback), "synthetic-state");
        assert_eq!(
            with_state(&callback, "other"),
            format!("{OAUTH_REDIRECT_URI}?code=synthetic-code-1&state=other")
        );
    }
}
