//! Wire types shared by the MCP launcher and the Koushi app's local socket.
//!
//! Both ends speak newline-delimited JSON: one object per line, no embedded
//! newlines. The launcher (`koushi-mcp`) is the only writer of MCP JSON-RPC on
//! stdout; the app is the only writer of IPC responses. Keeping the types in
//! one crate means a schema or error-code change cannot drift between the two
//! processes.

/// Protocol version carried by `hello` and `hello_ok`.
pub const PROTOCOL_VERSION: u32 = 1;

/// Maximum accepted line length on either side. A longer line is a protocol
/// violation, not a message: the bridge refuses it rather than buffering
/// unbounded input.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// Closed failure vocabulary. The launcher maps these to MCP tool errors and
/// never invents a code the app did not send.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Koushi is not running, or its socket could not be reached.
    KoushiUnavailable,
    /// The connection token did not match the running app.
    Unauthorized,
    /// The connection exists but the user has not granted it access yet.
    ConsentRequired,
    /// The request is outside the connection's grant.
    NotPermitted,
    /// The requested account is missing, signed out, locked, or replaced.
    AccountUnavailable,
    /// The room is unknown to the account or not joined.
    RoomNotFound,
    /// The event is unknown to the account's room.
    EventNotFound,
    /// The attachment exceeds the configured size limit.
    AttachmentTooLarge,
    /// The account already runs the maximum number of concurrent operations.
    Busy,
    /// The target already holds something the call must not overwrite, such
    /// as a composer draft the user typed (#1644). Nothing was changed.
    Conflict,
    /// The tool or argument shape is not supported by this build.
    Unsupported,
    /// A continuation cursor cannot be used (malformed, from a previous run,
    /// for another account, ahead of the feed, or older than its retained
    /// window). The message leads with the reason token; the caller starts a
    /// fresh baseline instead of assuming nothing happened.
    ResyncRequired,
    /// An unexpected internal failure; no detail crosses the boundary.
    Internal,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::KoushiUnavailable => "koushi_unavailable",
            Self::Unauthorized => "unauthorized",
            Self::ConsentRequired => "consent_required",
            Self::NotPermitted => "not_permitted",
            Self::AccountUnavailable => "account_unavailable",
            Self::RoomNotFound => "room_not_found",
            Self::EventNotFound => "event_not_found",
            Self::AttachmentTooLarge => "attachment_too_large",
            Self::Busy => "busy",
            Self::Conflict => "conflict",
            Self::Unsupported => "unsupported",
            Self::ResyncRequired => "resync_required",
            Self::Internal => "internal",
        }
    }
}

/// A failure travelling over the socket. `message` is private-safe: it never
/// carries message bodies, queries, tokens, or SDK errors.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WireError {
    pub code: ErrorCode,
    pub message: String,
    /// A stable, machine-readable refinement of `code`, such as why a cursor
    /// needs a resync. Absent for most failures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl WireError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            reason: None,
        }
    }

    /// A failure with a stable reason token the client can branch on.
    pub fn with_reason(code: ErrorCode, reason: &str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            reason: Some(reason.to_owned()),
        }
    }

    pub fn new_without_detail(code: ErrorCode) -> Self {
        Self::new(code, code.as_str())
    }
}

/// What the client tells the app about itself. Used for the consent surface's
/// label and for grant lookup, never for authorization on its own.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ClientInfo {
    pub name: String,
    #[serde(default)]
    pub version: String,
}

/// Why a client name was refused. The name is shown in Koushi's approval
/// dialog, so it must not be able to carry spacing, line breaks, or
/// bidirectional controls that could make it read as another assistant.
pub const CLIENT_NAME_RULE: &str = "client name must be 1-64 characters from A-Z a-z 0-9 . _ -";

/// Whether `name` is an acceptable `--client-name`.
pub fn valid_client_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// Whether a client version is shown as declared; anything else is dropped.
pub fn valid_client_version(version: &str) -> bool {
    version.len() <= 32
        && version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+'))
}

/// Messages sent by the launcher to the app.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Hello {
        protocol: u32,
        token: String,
        client: ClientInfo,
    },
    Request {
        id: u64,
        method: String,
        #[serde(default)]
        params: serde_json::Value,
    },
}

/// Messages sent by the app to the launcher.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    HelloOk {
        protocol: u32,
        connection_id: String,
        app_version: String,
    },
    Response {
        id: u64,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<WireError>,
    },
    /// Fatal pre-handshake failure; the app closes the socket afterwards.
    Error { code: ErrorCode, message: String },
}

impl ServerMessage {
    pub fn success(id: u64, result: serde_json::Value) -> Self {
        Self::Response {
            id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub fn failure(id: u64, error: WireError) -> Self {
        Self::Response {
            id,
            ok: false,
            result: None,
            error: Some(error),
        }
    }
}

/// `endpoint.json` contents. Written by the app on startup with mode 0600 and
/// removed on shutdown. The token is per launch and never persisted anywhere
/// else.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct EndpointFile {
    pub schema: u32,
    /// Unix socket path, or a Windows named-pipe name.
    pub endpoint: String,
    pub token: String,
    pub pid: u32,
    pub app_version: String,
}

/// Encodes one protocol message as a single line.
pub fn encode_line<T: serde::Serialize>(message: &T) -> Result<String, serde_json::Error> {
    let mut line = serde_json::to_string(message)?;
    line.push('\n');
    Ok(line)
}

/// Decodes one line. Trailing whitespace is tolerated; embedded newlines are
/// impossible because the caller splits on them first.
pub fn decode_line<T: serde::de::DeserializeOwned>(line: &str) -> Result<T, serde_json::Error> {
    serde_json::from_str(line.trim_end_matches(['\r', '\n']))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_message_round_trips() {
        let message = ClientMessage::Request {
            id: 7,
            method: "tools/list".to_owned(),
            params: serde_json::json!({}),
        };
        let line = encode_line(&message).expect("encode");
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        let decoded: ClientMessage = decode_line(&line).expect("decode");
        match decoded {
            ClientMessage::Request { id, method, .. } => {
                assert_eq!(id, 7);
                assert_eq!(method, "tools/list");
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }

    #[test]
    fn response_omits_empty_payloads() {
        let line = encode_line(&ServerMessage::success(1, serde_json::json!({"ok": true})))
            .expect("encode");
        assert!(line.contains("\"ok\":true"));
        assert!(!line.contains("error"));
        let failure = encode_line(&ServerMessage::failure(
            2,
            WireError::new_without_detail(ErrorCode::KoushiUnavailable),
        ))
        .expect("encode");
        assert!(failure.contains("koushi_unavailable"));
        assert!(!failure.contains("\"result\""));
    }

    /// Every published error code keeps a stable wire spelling, so a client can
    /// branch on it without parsing messages (#1524 adds `busy`).
    #[test]
    fn error_codes_have_distinct_and_stable_spellings() {
        let codes = [
            ErrorCode::KoushiUnavailable,
            ErrorCode::Unauthorized,
            ErrorCode::ConsentRequired,
            ErrorCode::NotPermitted,
            ErrorCode::AccountUnavailable,
            ErrorCode::RoomNotFound,
            ErrorCode::EventNotFound,
            ErrorCode::AttachmentTooLarge,
            ErrorCode::Busy,
            ErrorCode::Conflict,
            ErrorCode::Unsupported,
            ErrorCode::Internal,
        ];
        let mut seen = std::collections::HashSet::new();
        for code in codes {
            assert!(
                seen.insert(code.as_str()),
                "duplicate spelling: {}",
                code.as_str()
            );
            assert!(
                code.as_str()
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_')
            );
        }
        assert_eq!(ErrorCode::Busy.as_str(), "busy");
    }

    #[test]
    fn unknown_error_code_is_rejected() {
        let result: Result<ServerMessage, _> =
            decode_line(r#"{"type":"error","code":"whatever","message":"x"}"#);
        assert!(result.is_err());
    }
}
