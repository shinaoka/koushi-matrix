//! Bounded, privacy-safe vocabulary for authentication failures (#1268).
//!
//! The SDK boundary classifies a password / OAuth / SSO failure into this
//! value, and Core derives the coarse `AuthFailureKind`, the cleanup evidence,
//! the visible guidance reason, and the diagnostic fields from it. Every field
//! is a boolean, a count-like enum token, or an allowlisted kind, so the value
//! can cross the command/event/snapshot boundary and be recorded without ever
//! carrying credentials, tokens, callback URLs, raw SDK errors, response
//! bodies, account identifiers, or server URLs.
//!
//! The serialized shape intentionally mirrors the bounded vocabulary that the
//! secure-backup diagnostics work (#1265) established:
//! `{"stage":"passwordLogin","method":"password","transport":"httpResponse",
//! "httpStatus":403,"matrixErrorKind":"forbidden","retryable":false}`.

use serde::{Deserialize, Serialize};

/// The authentication method whose attempt failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuthMethod {
    Password,
    #[serde(rename = "oauth")]
    OAuth,
    Sso,
}

impl AuthMethod {
    pub fn token(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::OAuth => "oauth",
            Self::Sso => "sso",
        }
    }
}

/// The bounded stage a failure originated from. The enum is deliberately
/// coarse: it names the operation, never an endpoint, host, or request body.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuthFailureStage {
    /// Resolving a typed server name / homeserver URL (well-known, scheme).
    ResolveHomeserver,
    /// The password login request itself.
    PasswordLogin,
    /// Creating the OAuth authorization-code request.
    OidcStart,
    /// Completing the OAuth callback / token exchange.
    OidcCallback,
    /// Creating the legacy SSO login URL.
    SsoStart,
    /// A local credential/crypto store failure.
    LocalStore,
}

impl AuthFailureStage {
    pub fn token(self) -> &'static str {
        match self {
            Self::ResolveHomeserver => "resolveHomeserver",
            Self::PasswordLogin => "passwordLogin",
            Self::OidcStart => "oidcStart",
            Self::OidcCallback => "oidcCallback",
            Self::SsoStart => "ssoStart",
            Self::LocalStore => "localStore",
        }
    }
}

/// Whether the failure produced a response at all. A received server response
/// is never reported as a transport failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuthFailureTransport {
    /// The request produced no response (connection, DNS, TLS, or I/O).
    NoResponse,
    /// The homeserver produced a response that rejected the request.
    HttpResponse,
    /// The client gave up waiting for a response.
    Timeout,
    /// A local SDK/store failure, not a network exchange.
    Local,
}

impl AuthFailureTransport {
    pub fn token(self) -> &'static str {
        match self {
            Self::NoResponse => "noResponse",
            Self::HttpResponse => "httpResponse",
            Self::Timeout => "timeout",
            Self::Local => "local",
        }
    }
}

/// Allowlisted Matrix error kinds. Anything outside the allowlist is reported
/// as `Unknown`; the raw server vocabulary is never carried across the
/// boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuthMatrixErrorKind {
    Forbidden,
    Unauthorized,
    LimitExceeded,
    UnknownToken,
    MissingToken,
    NotFound,
    Unrecognized,
    BadJson,
    Unknown,
}

impl AuthMatrixErrorKind {
    pub fn token(self) -> &'static str {
        match self {
            Self::Forbidden => "forbidden",
            Self::Unauthorized => "unauthorized",
            Self::LimitExceeded => "limitExceeded",
            Self::UnknownToken => "unknownToken",
            Self::MissingToken => "missingToken",
            Self::NotFound => "notFound",
            Self::Unrecognized => "unrecognized",
            Self::BadJson => "badJson",
            Self::Unknown => "unknown",
        }
    }
}

/// The bounded structured cause preserved from the SDK boundary through Core.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthFailureDetail {
    pub method: AuthMethod,
    pub stage: AuthFailureStage,
    pub transport: AuthFailureTransport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matrix_error_kind: Option<AuthMatrixErrorKind>,
    pub retryable: bool,
}

impl AuthFailureDetail {
    pub const fn local(method: AuthMethod, stage: AuthFailureStage) -> Self {
        Self {
            method,
            stage,
            transport: AuthFailureTransport::Local,
            http_status: None,
            matrix_error_kind: None,
            retryable: false,
        }
    }

    /// A local failure that a later retry can plausibly resolve (for example a
    /// transient local runtime setup failure).
    pub const fn local_retryable(method: AuthMethod, stage: AuthFailureStage) -> Self {
        Self {
            retryable: true,
            ..Self::local(method, stage)
        }
    }

    pub const fn http_response(
        method: AuthMethod,
        stage: AuthFailureStage,
        http_status: Option<u16>,
        matrix_error_kind: Option<AuthMatrixErrorKind>,
        retryable: bool,
    ) -> Self {
        Self {
            method,
            stage,
            transport: AuthFailureTransport::HttpResponse,
            http_status,
            matrix_error_kind,
            retryable,
        }
    }

    pub const fn no_response(method: AuthMethod, stage: AuthFailureStage) -> Self {
        Self {
            method,
            stage,
            transport: AuthFailureTransport::NoResponse,
            http_status: None,
            matrix_error_kind: None,
            retryable: true,
        }
    }

    pub const fn timeout(method: AuthMethod, stage: AuthFailureStage) -> Self {
        Self {
            method,
            stage,
            transport: AuthFailureTransport::Timeout,
            http_status: None,
            matrix_error_kind: None,
            retryable: true,
        }
    }
}

/// The user's bounded chosen method for a delegated (browser) sign-in start.
/// Only the two supported delegated methods are representable, so no server
/// vocabulary crosses the command boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DelegatedAuthMethod {
    #[serde(rename = "oauth")]
    OAuth,
    Sso,
}

impl DelegatedAuthMethod {
    pub fn token(self) -> &'static str {
        match self {
            Self::OAuth => "oauth",
            Self::Sso => "sso",
        }
    }

    /// The bounded method token used in the failure detail.
    pub fn auth_method(self) -> AuthMethod {
        match self {
            Self::OAuth => AuthMethod::OAuth,
            Self::Sso => AuthMethod::Sso,
        }
    }
}
