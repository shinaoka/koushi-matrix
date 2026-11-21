//! Element Desktop's own room-key export over the Chrome DevTools Protocol.
//!
//! Element Desktop keeps its E2EE room keys in Chromium's IndexedDB, which
//! cannot be read from outside the app. Its webapp can export them, so Koushi
//! relaunches Element with a loopback debugging port, calls the export over
//! CDP, and closes that port again. Nothing here writes key material to disk,
//! and no error carries a path, a token, or a key.

use super::MigrationElementDesktop;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::{
    net::Ipv4Addr,
    path::Path,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};

const BUNDLE: &str = "/Applications/Element.app/Contents/MacOS/Element";
/// Part of the bundle executable's argv, as `pgrep -f` sees it.
const PROCESS_MATCH: &str = "Element.app/Contents/MacOS/Element";
/// The Linux package's executable name, as `pgrep -f` sees it.
const LINUX_PROCESS_MATCH: &str = "element-desktop";
/// Where the Linux packages install the executable, in package order.
const LINUX_BUNDLES: [&str; 4] = [
    "/usr/bin/element-desktop",
    "/usr/local/bin/element-desktop",
    "/opt/Element/element-desktop",
    "/snap/bin/element-desktop",
];
/// Opt-in for the Element Desktop import until it has run in the field. The
/// detection answers with `import_enabled` so the UI hides the route, and
/// `stage_element_desktop` refuses on its own.
pub(crate) const IMPORT_FLAG_ENV: &str = "KOUSHI_ENABLE_ELEMENT_DESKTOP_IMPORT";
/// Page URLs that mean "the Element webapp is loaded".
const WEBAPP_URLS: [&str; 2] = ["vector://vector/webapp", "https://app.element.io/"];
/// Element's export entry points, current SDK first.
const EXPRESSIONS: [&str; 2] = [
    "mxMatrixClientPeg.get().getCrypto().exportRoomKeys().then(k => JSON.stringify(k))",
    "mxMatrixClientPeg.get().exportRoomKeys().then(k => JSON.stringify(k))",
];
const EXPORT_DEADLINE: Duration = Duration::from_secs(90);
const QUIT_DEADLINE: Duration = Duration::from_secs(10);
const HTTP_TIMEOUT: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(250);

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ElementDesktopError {
    NotInstalled,
    ExportUnavailable,
    QuitFailed,
    LaunchFailed,
    InvalidEndpoint,
    /// No page target with the Element webapp URL appeared before the deadline.
    NoWebappTarget,
    Timeout,
}

impl ElementDesktopError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::NotInstalled => "migration_element_desktop_not_installed",
            Self::ExportUnavailable => "migration_element_desktop_export_unavailable",
            Self::QuitFailed => "migration_element_desktop_quit_failed",
            Self::LaunchFailed => "migration_element_desktop_launch_failed",
            Self::InvalidEndpoint => "migration_element_desktop_invalid_endpoint",
            Self::NoWebappTarget => "migration_element_desktop_no_webapp_target",
            Self::Timeout => "migration_element_desktop_timeout",
        }
    }
}

/// What detection found, before it is turned into the DTO. Split out so the
/// installed/running/unavailable combinations are testable off the platform.
#[derive(Clone, Copy, Debug)]
pub(super) struct DetectionFacts {
    pub(super) installed: bool,
    pub(super) running: bool,
    pub(super) import_supported: bool,
    pub(super) import_enabled: bool,
}

pub(super) fn describe(facts: DetectionFacts) -> MigrationElementDesktop {
    let installed = facts.installed;
    let importable = installed && facts.import_supported && facts.import_enabled;
    MigrationElementDesktop {
        installed,
        // A process without the application is not a source to offer.
        running: installed && facts.running,
        import_supported: facts.import_supported,
        import_enabled: facts.import_enabled,
        unavailable_reason: if importable || !installed {
            None
        } else if !facts.import_supported {
            Some("migration_element_desktop_platform_unsupported")
        } else {
            Some("migration_element_desktop_import_disabled")
        },
    }
}

pub(super) fn detect() -> MigrationElementDesktop {
    let installed = install_path().is_some_and(|path| path.exists());
    describe(DetectionFacts {
        installed,
        running: installed && is_running(),
        import_supported: cfg!(target_os = "macos"),
        import_enabled: import_flag_enabled(std::env::var(IMPORT_FLAG_ENV).ok().as_deref()),
    })
}

pub(super) fn import_flag_enabled(value: Option<&str>) -> bool {
    matches!(value, Some("1"))
}

/// The executable this platform would launch, whether or not it exists.
fn install_path() -> Option<std::path::PathBuf> {
    if cfg!(target_os = "macos") {
        return Some(std::path::PathBuf::from(BUNDLE));
    }
    if cfg!(target_os = "windows") {
        return windows_install_candidates(std::env::var("LOCALAPPDATA").ok().as_deref())
            .into_iter()
            .find(|path| path.exists());
    }
    linux_install_path(|path| path.exists())
}

/// `Element.exe` under the per-user install root. Split from `install_path` so
/// the Windows layout is checked from any host.
pub(super) fn windows_install_candidates(local_appdata: Option<&str>) -> Vec<std::path::PathBuf> {
    let mut candidates = vec![std::path::PathBuf::from(
        r"C:\Program Files\Element\Element.exe",
    )];
    if let Some(root) = local_appdata {
        candidates.push(
            std::path::Path::new(root)
                .join("Programs")
                .join("element-desktop")
                .join("Element.exe"),
        );
    }
    candidates
}

/// First Linux bundle that exists, in package-manager order.
pub(super) fn linux_install_path(exists: impl Fn(&Path) -> bool) -> Option<std::path::PathBuf> {
    LINUX_BUNDLES
        .iter()
        .map(Path::new)
        .find(|path| exists(path))
        .map(Path::to_path_buf)
}

/// `tasklist` prints one line per process; anything naming `Element.exe` is the
/// desktop app. Matched case-insensitively because Windows report the name as
/// the image was launched.
pub(super) fn tasklist_lists_element(output: &str) -> bool {
    output
        .lines()
        .any(|line| line.to_ascii_lowercase().contains("element.exe"))
}

/// `pgrep` answers in milliseconds, so this stays one synchronous path instead
/// of a second async copy for the polling loops.
fn is_running() -> bool {
    if cfg!(target_os = "windows") {
        return std::process::Command::new("tasklist")
            .output()
            .is_ok_and(|output| tasklist_lists_element(&String::from_utf8_lossy(&output.stdout)));
    }
    let pattern = if cfg!(target_os = "macos") {
        PROCESS_MATCH
    } else {
        LINUX_PROCESS_MATCH
    };
    std::process::Command::new("pgrep")
        .args(["-f", pattern])
        .output()
        .is_ok_and(|output| output.status.success())
}

pub(super) async fn export_room_keys() -> Result<Vec<Value>, ElementDesktopError> {
    if !cfg!(target_os = "macos") || !Path::new(BUNDLE).is_file() {
        return Err(ElementDesktopError::NotInstalled);
    }
    // The debugging port only exists on a started instance, so a running
    // Element has to go first; it comes back open once the export is done.
    let was_running = is_running();
    if was_running {
        quit().await?;
    }
    let port = free_port().await.ok_or(ElementDesktopError::LaunchFailed)?;
    let _element = tokio::process::Command::new(BUNDLE)
        .arg(format!("--remote-debugging-port={port}"))
        .spawn()
        .map_err(|_| ElementDesktopError::LaunchFailed)?;
    let keys = export_from_devtools(port, EXPORT_DEADLINE).await;
    // Close the port again even when the export failed.
    let _ = quit().await;
    if was_running {
        relaunch().await;
    }
    keys
}

async fn quit() -> Result<(), ElementDesktopError> {
    let _ = tokio::process::Command::new("osascript")
        .args(["-e", "tell application \"Element\" to quit"])
        .output()
        .await;
    let deadline = Instant::now() + QUIT_DEADLINE;
    while is_running() {
        if Instant::now() >= deadline {
            // Never force-kill: an unanswered quit is reported, not escalated.
            return Err(ElementDesktopError::QuitFailed);
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    Ok(())
}

async fn relaunch() {
    let _ = tokio::process::Command::new("open")
        .args(["-a", "Element"])
        .status()
        .await;
    let deadline = Instant::now() + QUIT_DEADLINE;
    while !is_running() {
        if Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Binds port 0 for the debugging port: the OS hands out an ephemeral port from
/// 49152 upwards, and the listener is dropped so Element can take it.
async fn free_port() -> Option<u16> {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .ok()?;
    listener.local_addr().ok().map(|address| address.port())
}

pub(super) async fn export_from_devtools(
    port: u16,
    timeout: Duration,
) -> Result<Vec<Value>, ElementDesktopError> {
    let deadline = Instant::now() + timeout;
    let url = webapp_socket_url(port, deadline).await?;
    let mut socket = connect(check_endpoint(&url, port)?, deadline).await?;
    for expression in EXPRESSIONS {
        let request = evaluate_request(expression);
        socket
            .send(request)
            .await
            .map_err(|_| ElementDesktopError::ExportUnavailable)?;
        if let Some(keys) = parse_keys(reply(&mut socket, deadline).await?) {
            return Ok(keys);
        }
    }
    Err(ElementDesktopError::ExportUnavailable)
}

/// The target URL comes from Element's own HTTP surface, so only the loopback
/// endpoint Koushi launched may be dialled.
fn check_endpoint(url: &str, port: u16) -> Result<&str, ElementDesktopError> {
    let authority = url
        .strip_prefix("ws://")
        .and_then(|rest| rest.split('/').next())
        .ok_or(ElementDesktopError::InvalidEndpoint)?;
    let (host, socket_port) = authority
        .rsplit_once(':')
        .ok_or(ElementDesktopError::InvalidEndpoint)?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let loopback = matches!(host, "127.0.0.1" | "::1" | "localhost");
    let same_port = socket_port.parse::<u16>().ok() == Some(port);
    if !loopback || !same_port {
        return Err(ElementDesktopError::InvalidEndpoint);
    }
    Ok(url)
}

async fn connect(url: &str, deadline: Instant) -> Result<Socket, ElementDesktopError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(ElementDesktopError::Timeout);
    }
    tokio::time::timeout(remaining, tokio_tungstenite::connect_async(url))
        .await
        .map_err(|_| ElementDesktopError::Timeout)?
        .map(|(socket, _)| socket)
        .map_err(|_| ElementDesktopError::ExportUnavailable)
}

fn evaluate_request(expression: &str) -> Message {
    let request = serde_json::json!({
        "id": 1,
        "method": "Runtime.evaluate",
        "params": {
            "expression": expression,
            "awaitPromise": true,
            "returnByValue": true,
        },
    });
    Message::Text(request.to_string().into())
}

/// Reads frames until the response to our own request: CDP events on the same
/// socket carry no `id`.
async fn reply(
    socket: &mut Socket,
    deadline: Instant,
) -> Result<Option<String>, ElementDesktopError> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(ElementDesktopError::Timeout);
        }
        let frame = match tokio::time::timeout(remaining, socket.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => text.to_string(),
            Ok(Some(Ok(Message::Close(_)))) | Ok(None) | Ok(Some(Err(_))) => return Ok(None),
            Ok(Some(Ok(_))) => continue,
            Err(_) => return Err(ElementDesktopError::Timeout),
        };
        let Ok(message) = serde_json::from_str::<Value>(&frame) else {
            continue;
        };
        if message.get("id").and_then(Value::as_u64) != Some(1) {
            continue;
        }
        let Some(result) = message.get("result") else {
            return Ok(None);
        };
        if result.get("exceptionDetails").is_some() {
            return Ok(None);
        }
        return Ok(result
            .get("result")
            .and_then(|value| value.get("value"))
            .and_then(Value::as_str)
            .map(str::to_owned));
    }
}

/// An empty or partial export is never a success: staging nothing must not look
/// like a completed import.
fn parse_keys(raw: Option<String>) -> Option<Vec<Value>> {
    let keys = serde_json::from_str::<Vec<Value>>(&raw?).ok()?;
    (!keys.is_empty() && keys.iter().all(Value::is_object)).then_some(keys)
}

async fn webapp_socket_url(port: u16, deadline: Instant) -> Result<String, ElementDesktopError> {
    loop {
        let targets = http_get(port, "/json/list")
            .await
            .and_then(|body| serde_json::from_str::<Value>(&body).ok());
        if let Some(url) = targets.as_ref().and_then(webapp_target) {
            return Ok(url);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            // Element is still booting when the port is up but no page is.
            return Err(ElementDesktopError::NoWebappTarget);
        }
        tokio::time::sleep(remaining.min(POLL_INTERVAL)).await;
    }
}

fn webapp_target(targets: &Value) -> Option<String> {
    targets.as_array()?.iter().find_map(|target| {
        let kind = target.get("type")?.as_str()?;
        let url = target.get("url")?.as_str()?;
        let socket = target.get("webSocketDebuggerUrl")?.as_str()?;
        let webapp = WEBAPP_URLS.iter().any(|prefix| url.starts_with(prefix));
        (kind == "page" && webapp && !socket.is_empty()).then(|| socket.to_owned())
    })
}

/// One `GET`, `Connection: close`, body read to EOF: the single endpoint CDP
/// exposes does not justify an HTTP dependency.
async fn http_get(port: u16, path: &str) -> Option<String> {
    tokio::time::timeout(HTTP_TIMEOUT, async {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await.ok()?;
        let request =
            format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await.ok()?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.ok()?;
        let response = String::from_utf8(response).ok()?;
        let (head, body) = response.split_once("\r\n\r\n")?;
        head.lines()
            .next()?
            .contains(" 200 ")
            .then(|| body.to_owned())
    })
    .await
    .ok()?
}
