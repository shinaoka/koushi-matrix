use crate::client_session::{build_client, oidc_client_registration_data};
use crate::e2ee::install_room_key_diagnostic_observer;
use crate::{MatrixClientSession, MatrixClientStoreConfig, logout};
use koushi_state::{
    AuthSecret, DelegatedAuthLinks, LoginFlow, LoginFlowKind, LoginRequest, SessionInfo,
};
use matrix_sdk::utils::UrlOrQuery;
use serde::Deserialize;
use std::{fmt, net::IpAddr, time::Duration};
use thiserror::Error;
use url::Url;

const LOGIN_DISCOVERY_PATH: &str = "_matrix/client/v3/login";

const WELL_KNOWN_CLIENT_PATH: &str = ".well-known/matrix/client";

/// The well-known document is a nicety (account-management links); it must
/// never delay login, so it gets a much shorter budget than flow discovery.
const WELL_KNOWN_CLIENT_TIMEOUT: Duration = Duration::from_secs(3);

const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) const SYNC_INVITE_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

pub(super) const SYNC_INVITE_PROBE_CONNECTION_ID: &str = "koushi-invite";

pub(super) const SYNC_INVITE_PROBE_LIST_KEY: &str = "koushi_invites";

pub(super) const MATRIX_ROOM_LIST_SNAPSHOT_LIMIT: usize = 4096;

pub const LOCAL_USER_ALIASES_ACCOUNT_DATA_TYPE: &str = "app.koushi.local_aliases";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoginDiscovery {
    pub homeserver: String,
    pub flows: Vec<LoginFlow>,
    pub delegated: DelegatedAuthLinks,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixLoginDiscovery {
    pub homeserver: String,
    pub flows: Vec<MatrixLoginFlow>,
    pub delegated: DelegatedAuthLinks,
}

#[derive(Clone, Eq, PartialEq)]
pub struct OidcAuthorization {
    pub authorization_url: String,
    pub state: String,
}

impl fmt::Debug for OidcAuthorization {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OidcAuthorization")
            .field("authorization_url", &"AuthorizationUrl(..)")
            .field("state", &"CsrfState(..)")
            .finish()
    }
}

#[derive(Clone)]
pub enum PendingOidcLogin {
    OAuth {
        client: matrix_sdk::Client,
        homeserver: String,
    },
    Sso {
        client: matrix_sdk::Client,
        homeserver: String,
    },
}

impl PendingOidcLogin {
    pub fn homeserver(&self) -> &str {
        match self {
            Self::OAuth { homeserver, .. } | Self::Sso { homeserver, .. } => homeserver,
        }
    }
}

impl fmt::Debug for PendingOidcLogin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = formatter.debug_struct("PendingOidcLogin");
        match self {
            Self::OAuth { .. } => debug.field("kind", &"OAuth"),
            Self::Sso { .. } => debug.field("kind", &"Sso"),
        }
        .field("client", &"MatrixClient(..)")
        .field("homeserver", &"Homeserver(..)")
        .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixLoginFlow {
    pub kind: MatrixLoginFlowKind,
    pub delegated_oidc_compatibility: bool,
    pub display_name: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MatrixLoginFlowKind {
    Password,
    Sso,
    Oidc,
    Token,
    Unknown(String),
}

#[derive(Clone, Debug)]
pub struct Homeserver {
    base_url: Url,
}

impl Homeserver {
    pub fn parse(input: &str) -> Result<Self, LoginDiscoveryError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(LoginDiscoveryError::InvalidHomeserver(
                "homeserver is empty".to_owned(),
            ));
        }

        let candidate = if trimmed.contains("://") {
            trimmed.to_owned()
        } else {
            format!("https://{trimmed}")
        };
        let mut base_url = Url::parse(&candidate)
            .map_err(|error| LoginDiscoveryError::InvalidHomeserver(error.to_string()))?;

        if !matches!(base_url.scheme(), "http" | "https") {
            return Err(LoginDiscoveryError::UnsupportedHomeserverScheme);
        }
        if base_url.host_str().is_none() {
            return Err(LoginDiscoveryError::InvalidHomeserver(
                "homeserver URL is missing a host".to_owned(),
            ));
        }
        if !base_url.username().is_empty() || base_url.password().is_some() {
            return Err(LoginDiscoveryError::InvalidHomeserver(
                "homeserver URL must not include credentials".to_owned(),
            ));
        }
        if base_url.scheme() == "http" && !is_loopback_homeserver(&base_url) {
            return Err(LoginDiscoveryError::InsecureHomeserverScheme);
        }
        if base_url.query().is_some() || base_url.fragment().is_some() {
            return Err(LoginDiscoveryError::InvalidHomeserver(
                "homeserver URL must not include query or fragment".to_owned(),
            ));
        }

        if !base_url.path().ends_with('/') {
            let mut path = base_url.path().to_owned();
            path.push('/');
            base_url.set_path(&path);
        }

        Ok(Self { base_url })
    }

    pub fn normalized(&self) -> String {
        let mut normalized = self.base_url.to_string();
        if normalized.ends_with('/') {
            normalized.pop();
        }
        normalized
    }

    pub fn login_discovery_url(&self) -> Url {
        self.base_url
            .join(LOGIN_DISCOVERY_PATH)
            .expect("login discovery path should be relative")
    }

    /// `/.well-known/matrix/client` at the origin root (the homeserver
    /// scheme+host, not the client-server base path).
    pub fn well_known_client_url(&self) -> Url {
        let mut origin = self.base_url.clone();
        origin.set_path("");
        origin.set_query(None);
        origin.set_fragment(None);
        origin
            .join(WELL_KNOWN_CLIENT_PATH)
            .expect("well-known client path should be relative")
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum LoginDiscoveryError {
    #[error("homeserver URL is invalid: {0}")]
    InvalidHomeserver(String),
    #[error("homeserver URL scheme must be http or https")]
    UnsupportedHomeserverScheme,
    #[error("homeserver URL must use https unless it is localhost or loopback")]
    InsecureHomeserverScheme,
    #[error("login discovery request failed: {0}")]
    RequestFailed(String),
    #[error("login discovery failed with HTTP {status}: {message}")]
    HttpStatus { status: u16, message: String },
    #[error("login discovery response is missing flows")]
    MissingFlows,
    #[error("login discovery response is invalid: {0}")]
    InvalidResponse(String),
}

#[derive(Debug, Error)]
pub enum PasswordLoginError {
    #[error(transparent)]
    InvalidHomeserver(#[from] LoginDiscoveryError),
    #[error("password login runtime failed: {0}")]
    Runtime(String),
    #[error("password login failed: {0}")]
    Sdk(String),
    #[error("SDK session is not available")]
    MissingSession,
    #[error("session serialization failed: {0}")]
    Serialization(String),
}

#[derive(Deserialize)]
struct LoginDiscoveryResponse {
    flows: Vec<RawLoginFlow>,
}

#[derive(Deserialize)]
struct RawLoginFlow {
    #[serde(rename = "type")]
    flow_type: String,
    #[serde(default, rename = "org.matrix.msc3824.delegated_oidc_compatibility")]
    delegated_oidc_compatibility: bool,
    #[serde(default)]
    display_name: Option<String>,
}

#[derive(Deserialize)]
struct MatrixErrorResponse {
    error: Option<String>,
}

pub fn discover_login_flows(homeserver: &str) -> Result<LoginDiscovery, LoginDiscoveryError> {
    let homeserver = Homeserver::parse(homeserver)?;
    let response = reqwest::blocking::Client::builder()
        .timeout(DISCOVERY_TIMEOUT)
        .user_agent("matrix-desktop-prelogin/0.1")
        .build()
        .map_err(|error| LoginDiscoveryError::RequestFailed(error.to_string()))?
        .get(homeserver.login_discovery_url())
        .send()
        .map_err(|error| LoginDiscoveryError::RequestFailed(error.to_string()))?;

    let status = response.status().as_u16();
    let body = response
        .text()
        .map_err(|error| LoginDiscoveryError::RequestFailed(error.to_string()))?;
    let flows = parse_login_discovery_http_response(status, &body)?;

    Ok(LoginDiscovery {
        homeserver: homeserver.normalized(),
        flows,
        // #475: delegated account-management/registration links come from the
        // well-known client document; failure to fetch or parse it must never
        // block login (the links are a nicety, so this fails open to empty).
        delegated: discover_delegated_auth_links(&homeserver),
    })
}

/// Fetch and parse the `/.well-known/matrix/client` delegated-auth metadata.
/// Any error (network, status, malformed body, missing metadata, unsupported
/// scheme) yields empty links so login discovery never depends on it.
fn discover_delegated_auth_links(homeserver: &Homeserver) -> DelegatedAuthLinks {
    match fetch_well_known_client(homeserver) {
        Some(links) => links,
        None => DelegatedAuthLinks::default(),
    }
}

fn fetch_well_known_client(homeserver: &Homeserver) -> Option<DelegatedAuthLinks> {
    let response = reqwest::blocking::Client::builder()
        .timeout(WELL_KNOWN_CLIENT_TIMEOUT)
        .user_agent("matrix-desktop-prelogin/0.1")
        .build()
        .ok()?
        .get(homeserver.well_known_client_url())
        .send()
        .ok()?;
    if response.status().as_u16() != 200 {
        return None;
    }
    let body = response.text().ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&body).ok()?;
    Some(parse_well_known_client(&value))
}

/// Resolve the server-owned account/device-management destination for the
/// authenticated session. Absence and every discovery failure are a missing
/// capability, never a session failure.
pub async fn resolve_active_session_account_management_url(
    session: &MatrixClientSession,
) -> Option<String> {
    if session.client().oauth().full_session().is_some() {
        use matrix_sdk::ruma::api::client::discovery::get_authorization_server_metadata::v1::AccountManagementActionData;

        let metadata = session.client().oauth().server_metadata().await.ok()?;
        let url = metadata
            .account_management_url_with_action(AccountManagementActionData::DevicesList)?;
        return parse_discovered_http_url_str(url.as_str());
    }

    let homeserver = Homeserver::parse(&session.info.homeserver).ok()?;
    let response = reqwest::Client::builder()
        .timeout(WELL_KNOWN_CLIENT_TIMEOUT)
        .user_agent("matrix-desktop-active-session/0.1")
        .build()
        .ok()?
        .get(homeserver.well_known_client_url())
        .send()
        .await
        .ok()?;
    if response.status().as_u16() != 200 {
        return None;
    }
    let body = response.text().await.ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&body).ok()?;
    parse_account_management_url_from_well_known(&value)
}

pub fn login_with_password_blocking(
    request: &LoginRequest,
) -> Result<MatrixClientSession, PasswordLoginError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| PasswordLoginError::Runtime(error.to_string()))?;

    runtime.block_on(login_with_password(request))
}

pub fn logout_blocking(session: &MatrixClientSession) -> Result<(), PasswordLoginError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| PasswordLoginError::Runtime(error.to_string()))?;

    runtime.block_on(logout(session))
}

pub async fn login_with_password(
    request: &LoginRequest,
) -> Result<MatrixClientSession, PasswordLoginError> {
    login_with_password_with_store(request, None).await
}

pub async fn login_with_password_with_store(
    request: &LoginRequest,
    store_config: Option<&MatrixClientStoreConfig>,
) -> Result<MatrixClientSession, PasswordLoginError> {
    let homeserver = Homeserver::parse(&request.homeserver)?;
    let client = build_client(&homeserver, store_config).await?;

    let mut login = client
        .matrix_auth()
        .login_username(&request.username, request.password.expose_secret());
    if let Some(device_display_name) = request.device_display_name.as_deref() {
        login = login.initial_device_display_name(device_display_name);
    }

    let response = login
        .send()
        .await
        .map_err(|error| PasswordLoginError::Sdk(error.to_string()))?;
    let user_id = response.user_id.to_string();
    let device_id = response.device_id.to_string();
    client
        .send_queue()
        .require_secure_backup_for_encrypted_sends(false);
    let diagnostic_counters = install_room_key_diagnostic_observer(&client).await;

    Ok(MatrixClientSession {
        client,
        diagnostic_counters,
        info: SessionInfo {
            homeserver: homeserver.normalized(),
            user_id,
            device_id,
            authentication_method: koushi_state::SessionAuthenticationMethod::Password,
        },
    })
}

/// Re-authenticate an existing soft-logged-out session with the same device id.
/// The returned storeless session must be persisted and restored into the
/// existing per-account store by the caller so crypto/cached data is preserved.
pub async fn login_with_existing_device(
    homeserver: &str,
    user_id: &str,
    device_id: &str,
    password: &AuthSecret,
) -> Result<MatrixClientSession, PasswordLoginError> {
    let homeserver = Homeserver::parse(homeserver)?;
    let client = build_client(&homeserver, None).await?;

    let response = client
        .matrix_auth()
        .login_username(user_id, password.expose_secret())
        .device_id(device_id)
        .send()
        .await
        .map_err(|error| PasswordLoginError::Sdk(error.to_string()))?;
    client
        .send_queue()
        .require_secure_backup_for_encrypted_sends(false);
    let diagnostic_counters = install_room_key_diagnostic_observer(&client).await;

    Ok(MatrixClientSession {
        client,
        diagnostic_counters,
        info: SessionInfo {
            homeserver: homeserver.normalized(),
            user_id: response.user_id.to_string(),
            device_id: response.device_id.to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Password,
        },
    })
}

pub async fn start_oidc_login(
    homeserver: &str,
    redirect_uri: &str,
) -> Result<(PendingOidcLogin, OidcAuthorization), PasswordLoginError> {
    let homeserver = Homeserver::parse(homeserver)?;
    let redirect_uri =
        Url::parse(redirect_uri).map_err(|error| PasswordLoginError::Sdk(error.to_string()))?;
    let client = build_client(&homeserver, None).await?;

    match client
        .oauth()
        .login(
            redirect_uri.clone(),
            None,
            Some(oidc_client_registration_data(redirect_uri.clone())),
            None,
        )
        .build()
        .await
    {
        Ok(authorization) => Ok((
            PendingOidcLogin::OAuth {
                client,
                homeserver: homeserver.normalized(),
            },
            OidcAuthorization {
                authorization_url: authorization.url.to_string(),
                state: authorization.state.secret().to_owned(),
            },
        )),
        Err(_) => {
            let authorization_url = client
                .matrix_auth()
                .get_sso_login_url(redirect_uri.as_str(), None)
                .await
                .map_err(|error| PasswordLoginError::Sdk(error.to_string()))?;
            Ok((
                PendingOidcLogin::Sso {
                    client,
                    homeserver: homeserver.normalized(),
                },
                OidcAuthorization {
                    authorization_url,
                    state: String::new(),
                },
            ))
        }
    }
}

pub async fn finish_oidc_login(
    pending: PendingOidcLogin,
    callback_url: &str,
) -> Result<MatrixClientSession, PasswordLoginError> {
    let callback_url =
        Url::parse(callback_url).map_err(|error| PasswordLoginError::Sdk(error.to_string()))?;
    let (client, homeserver, authentication_method) = match pending {
        PendingOidcLogin::OAuth { client, homeserver } => {
            client
                .oauth()
                .finish_login(callback_url.into())
                .await
                .map_err(|error| PasswordLoginError::Sdk(error.to_string()))?;
            (
                client,
                homeserver,
                koushi_state::SessionAuthenticationMethod::OAuth,
            )
        }
        PendingOidcLogin::Sso { client, homeserver } => {
            client
                .matrix_auth()
                .login_with_sso_callback(UrlOrQuery::Url(callback_url))
                .map_err(|error| PasswordLoginError::Sdk(error.to_string()))?
                .initial_device_display_name("Koushi")
                .request_refresh_token()
                .send()
                .await
                .map_err(|error| PasswordLoginError::Sdk(error.to_string()))?;
            (
                client,
                homeserver,
                koushi_state::SessionAuthenticationMethod::Sso,
            )
        }
    };

    let user_id = client
        .user_id()
        .ok_or(PasswordLoginError::MissingSession)?
        .to_string();
    let device_id = client
        .device_id()
        .ok_or(PasswordLoginError::MissingSession)?
        .to_string();
    client
        .send_queue()
        .require_secure_backup_for_encrypted_sends(false);
    let diagnostic_counters = install_room_key_diagnostic_observer(&client).await;

    Ok(MatrixClientSession {
        client,
        diagnostic_counters,
        info: SessionInfo {
            homeserver,
            user_id,
            device_id,
            authentication_method,
        },
    })
}

pub fn parse_login_discovery_http_response(
    status: u16,
    body: &str,
) -> Result<Vec<LoginFlow>, LoginDiscoveryError> {
    if status != 200 {
        return Err(LoginDiscoveryError::HttpStatus {
            status,
            message: matrix_error_message(body),
        });
    }

    let value = serde_json::from_str::<serde_json::Value>(body)
        .map_err(|error| LoginDiscoveryError::InvalidResponse(error.to_string()))?;
    parse_login_discovery(&value)
}

pub fn parse_login_discovery(
    value: &serde_json::Value,
) -> Result<Vec<LoginFlow>, LoginDiscoveryError> {
    Ok(map_login_flows_to_desktop(parse_matrix_login_flows(value)?))
}

/// Parse the pre-login registration link from a `/.well-known/matrix/client`
/// document (#475). Both the finalized `m.authentication` key and the older
/// `org.matrix.msc2965.authentication` key are accepted. Authenticated account
/// management is deliberately resolved by the active-session path instead.
pub fn parse_well_known_client(value: &serde_json::Value) -> DelegatedAuthLinks {
    let authentication = value
        .get("m.authentication")
        .or_else(|| value.get("org.matrix.msc2965.authentication"));
    let Some(authentication) = authentication else {
        return DelegatedAuthLinks::default();
    };
    DelegatedAuthLinks {
        registration_url: parse_discovered_http_url(authentication.get("registration")),
    }
}

fn parse_account_management_url_from_well_known(value: &serde_json::Value) -> Option<String> {
    let authentication = value
        .get("m.authentication")
        .or_else(|| value.get("org.matrix.msc2965.authentication"))?;
    parse_discovered_http_url(authentication.get("account"))
}

/// A discovered URL is usable only when it parses and uses http/https.
/// Anything else (missing, malformed, `javascript:`, `file:`, …) is None so
/// the UI never renders a broken or dangerous link.
fn parse_discovered_http_url(value: Option<&serde_json::Value>) -> Option<String> {
    parse_discovered_http_url_str(value?.as_str()?)
}

fn parse_discovered_http_url_str(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let url = Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    // Never let embedded credentials cross the discovery/snapshot boundary.
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    Some(url.to_string())
}

pub fn parse_matrix_login_flows(
    value: &serde_json::Value,
) -> Result<Vec<MatrixLoginFlow>, LoginDiscoveryError> {
    if !value.get("flows").is_some_and(serde_json::Value::is_array) {
        return Err(LoginDiscoveryError::MissingFlows);
    }

    let response = serde_json::from_value::<LoginDiscoveryResponse>(value.clone())
        .map_err(|error| LoginDiscoveryError::InvalidResponse(error.to_string()))?;

    Ok(response
        .flows
        .into_iter()
        .map(|flow| MatrixLoginFlow {
            kind: parse_flow_kind(flow.flow_type),
            delegated_oidc_compatibility: flow.delegated_oidc_compatibility,
            display_name: flow.display_name,
        })
        .collect())
}

pub fn map_login_flows_to_desktop(flows: Vec<MatrixLoginFlow>) -> Vec<LoginFlow> {
    flows
        .into_iter()
        .map(|flow| LoginFlow {
            kind: match flow.kind {
                MatrixLoginFlowKind::Password => LoginFlowKind::Password,
                MatrixLoginFlowKind::Sso => LoginFlowKind::Sso,
                MatrixLoginFlowKind::Oidc => LoginFlowKind::Oidc,
                MatrixLoginFlowKind::Token => LoginFlowKind::Token,
                MatrixLoginFlowKind::Unknown(value) => LoginFlowKind::Unknown(value),
            },
            delegated_oidc_compatibility: flow.delegated_oidc_compatibility,
            display_name: flow.display_name,
        })
        .collect()
}

fn parse_flow_kind(flow_type: String) -> MatrixLoginFlowKind {
    match flow_type.as_str() {
        "m.login.password" => MatrixLoginFlowKind::Password,
        "m.login.sso" => MatrixLoginFlowKind::Sso,
        "m.login.oidc" | "m.login.oauth2" => MatrixLoginFlowKind::Oidc,
        "m.login.token" => MatrixLoginFlowKind::Token,
        _ => MatrixLoginFlowKind::Unknown(flow_type),
    }
}

fn is_loopback_homeserver(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };

    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn matrix_error_message(body: &str) -> String {
    serde_json::from_str::<MatrixErrorResponse>(body)
        .ok()
        .and_then(|response| response.error)
        .filter(|message| !message.trim().is_empty())
        .unwrap_or_else(|| "homeserver did not return login flows".to_owned())
}

#[cfg(test)]
mod active_session_account_management_tests {
    use matrix_sdk::test_utils::{client::oauth, mocks::MatrixMockServer};
    use serde_json::json;
    use wiremock::{
        Mock, ResponseTemplate,
        matchers::{method, path},
    };

    use super::{MatrixClientSession, resolve_active_session_account_management_url};
    use koushi_state::{SessionAuthenticationMethod, SessionInfo};

    async fn password_session(server: &MatrixMockServer) -> MatrixClientSession {
        let client = server.client_builder().unlogged().build().await;
        MatrixClientSession::from_client_for_testing(
            client,
            SessionInfo {
                homeserver: server.server().uri(),
                user_id: "@alice:example.test".to_owned(),
                device_id: "PASSWORDDEVICE".to_owned(),
                authentication_method: SessionAuthenticationMethod::Password,
            },
        )
    }

    #[tokio::test]
    async fn password_session_resolves_finalized_and_matrix_org_legacy_well_known() {
        for key in ["m.authentication", "org.matrix.msc2965.authentication"] {
            let server = MatrixMockServer::new().await;
            Mock::given(method("GET"))
                .and(path("/.well-known/matrix/client"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    key: { "account": "https://account.example.test/devices" }
                })))
                .expect(1)
                .mount(server.server())
                .await;

            assert_eq!(
                resolve_active_session_account_management_url(&password_session(&server).await)
                    .await
                    .as_deref(),
                Some("https://account.example.test/devices")
            );
        }
    }

    #[tokio::test]
    async fn password_session_treats_missing_malformed_and_unsafe_metadata_as_unavailable() {
        for body in [
            json!({}),
            json!({ "m.authentication": { "account": 7 } }),
            json!({ "m.authentication": { "account": "javascript:alert(1)" } }),
            json!({ "m.authentication": { "account": "https://user:secret@example.test" } }),
        ] {
            let server = MatrixMockServer::new().await;
            Mock::given(method("GET"))
                .and(path("/.well-known/matrix/client"))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .expect(1)
                .mount(server.server())
                .await;

            assert_eq!(
                resolve_active_session_account_management_url(&password_session(&server).await)
                    .await,
                None
            );
        }
    }

    #[tokio::test]
    async fn password_session_treats_http_and_json_failures_as_unavailable() {
        for response in [
            ResponseTemplate::new(500),
            ResponseTemplate::new(200).set_body_string("not-json"),
        ] {
            let server = MatrixMockServer::new().await;
            Mock::given(method("GET"))
                .and(path("/.well-known/matrix/client"))
                .respond_with(response)
                .expect(1)
                .mount(server.server())
                .await;

            assert_eq!(
                resolve_active_session_account_management_url(&password_session(&server).await)
                    .await,
                None
            );
        }
    }

    #[tokio::test]
    async fn password_session_treats_network_failure_as_unavailable() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve test port");
        let address = listener.local_addr().expect("reserved address");
        drop(listener);
        let server = MatrixMockServer::new().await;
        let mut session = password_session(&server).await;
        session.info.homeserver = format!("http://{address}");

        assert_eq!(
            resolve_active_session_account_management_url(&session).await,
            None
        );
    }

    #[tokio::test]
    async fn oauth_session_treats_metadata_failure_as_unavailable() {
        let server = MatrixMockServer::new().await;
        server
            .oauth()
            .mock_server_metadata()
            .error500()
            .expect(1)
            .mount()
            .await;
        let client = server.client_builder().unlogged().build().await;
        client
            .oauth()
            .restore_session(
                oauth::mock_session(matrix_sdk::test_utils::client::mock_session_tokens()),
                matrix_sdk_base::store::RoomLoadSettings::default(),
            )
            .await
            .expect("restore synthetic OAuth session");
        let session = MatrixClientSession::from_client_for_testing(
            client,
            SessionInfo {
                homeserver: server.server().uri(),
                user_id: "@alice:example.test".to_owned(),
                device_id: "OAUTHDEVICE".to_owned(),
                authentication_method: SessionAuthenticationMethod::OAuth,
            },
        );

        assert_eq!(
            resolve_active_session_account_management_url(&session).await,
            None
        );
    }

    #[tokio::test]
    async fn oauth_session_uses_public_server_metadata_devices_list_action() {
        let server = MatrixMockServer::new().await;
        server
            .oauth()
            .mock_server_metadata()
            .ok_https()
            .expect(1)
            .mount()
            .await;
        let client = server.client_builder().unlogged().build().await;
        client
            .oauth()
            .restore_session(
                oauth::mock_session(matrix_sdk::test_utils::client::mock_session_tokens()),
                matrix_sdk_base::store::RoomLoadSettings::default(),
            )
            .await
            .expect("restore synthetic OAuth session");
        let session = MatrixClientSession::from_client_for_testing(
            client,
            SessionInfo {
                homeserver: server.server().uri(),
                user_id: "@alice:example.test".to_owned(),
                device_id: "OAUTHDEVICE".to_owned(),
                authentication_method: SessionAuthenticationMethod::OAuth,
            },
        );

        let destination = resolve_active_session_account_management_url(&session)
            .await
            .expect("mock metadata advertises account management");
        assert!(destination.starts_with("https://"));
        assert!(
            destination.contains("org.matrix.sessions_list"),
            "{destination}"
        );
    }
}
