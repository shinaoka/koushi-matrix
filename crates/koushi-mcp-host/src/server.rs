//! Local socket server hosted by the Tauri adapter.
//!
//! One connection per MCP client session. The launcher authenticates with the
//! per-launch token from `endpoint.json`; this module never reads Matrix
//! credentials and never hands one to a caller.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::watch;

use crate::BridgeAccounts;
use crate::consent::{ClientIdentity, ConsentBroker};
use crate::endpoint::constant_time_eq;
use koushi_mcp::wire::{
    ClientMessage, ErrorCode, LEGACY_PROTOCOL, MAX_FRAME_BYTES, SUPPORTED_PROTOCOLS, ServerMessage,
    WireError, decode_line, encode_line,
};

use super::{endpoint, events, handlers, profiles};

#[cfg(unix)]
type Listener = tokio::net::UnixListener;
#[cfg(unix)]
type Stream = tokio::net::UnixStream;
#[cfg(windows)]
type Listener = NamedPipeListener;
#[cfg(windows)]
type Stream = tokio::net::windows::named_pipe::NamedPipeServer;

/// How often expired approvals are closed while the bridge runs.
const SWEEP_INTERVAL: Duration = Duration::from_secs(5);
/// How long a graceful shutdown waits for connections to wind down.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// Owns the listener task and the files it published.
pub struct BridgeHandle {
    shutdown: watch::Sender<bool>,
    stopped: AtomicBool,
    accept_task: Option<tokio::task::JoinHandle<()>>,
    broker: Arc<ConsentBroker>,
    data_dir: PathBuf,
    socket_path: PathBuf,
}

impl BridgeHandle {
    /// Signals the listener and every connection to stop and removes the
    /// published files. Waiting tool calls answer `koushi_unavailable`.
    /// Idempotent, so a later `Drop` cannot remove files a newer bridge wrote.
    pub fn stop(&self) {
        if self.stopped.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = self.shutdown.send(true);
        self.broker.reset();
        self.remove_files();
    }

    /// Stops the bridge and waits for the listener and its connections to
    /// finish, so a bridge started afterwards owns the socket path alone.
    /// The broker is reset only after the connections are gone, so no call
    /// can open an approval after the reset.
    pub async fn shutdown(mut self) {
        if !self.stopped.swap(true, Ordering::SeqCst) {
            let _ = self.shutdown.send(true);
            if let Some(mut task) = self.accept_task.take()
                && tokio::time::timeout(SHUTDOWN_GRACE, &mut task)
                    .await
                    .is_err()
            {
                task.abort();
                let _ = task.await;
            }
            self.broker.reset();
            self.remove_files();
        }
    }

    fn remove_files(&self) {
        endpoint::remove(&self.data_dir);
        // Harmless on Windows: the pipe vanishes with the process, and the
        // path is not a filesystem entry there.
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

impl Drop for BridgeHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Starts the bridge: binds the socket, writes `endpoint.json`, and accepts
/// connections until shutdown. Tool calls are authorized by `broker`.
pub fn start(
    accounts: Arc<dyn BridgeAccounts>,
    data_dir: PathBuf,
    broker: Arc<ConsentBroker>,
) -> Result<BridgeHandle, &'static str> {
    endpoint::ensure_dir(&data_dir).map_err(|_| "mcp_dir")?;
    // A secret file with no row, left by a crash between the two, authenticates
    // nothing; remove it.
    profiles::sweep_orphans(&data_dir);
    let transport = endpoint::transport_path(&data_dir);
    let token = endpoint::new_token().map_err(|_| "mcp_token")?;
    let socket_path = PathBuf::from(&transport);

    #[cfg(unix)]
    let listener = {
        // A stale socket from a crashed run must not block binding.
        let _ = std::fs::remove_file(&socket_path);
        let listener = Listener::bind(&socket_path).map_err(|_| "mcp_bind")?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))
            .map_err(|_| "mcp_socket_mode")?;
        listener
    };
    #[cfg(windows)]
    let listener = Listener::bind(&transport, true).map_err(|_| "mcp_pipe")?;

    endpoint::write(&data_dir, &transport, &token, env!("CARGO_PKG_VERSION"))
        .map_err(|_| "mcp_endpoint_file")?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let log = Arc::new(events::EventLog::new());
    events::spawn_poller(Arc::clone(&accounts), Arc::clone(&log), shutdown_rx.clone());
    spawn_sweeper(Arc::clone(&broker), shutdown_rx.clone());
    let accept_task = tokio::spawn(accept_loop(
        listener,
        Shared {
            accounts,
            log,
            activity_pages: Arc::new(crate::recent_activity::ActivityPages::default()),
            broker: Arc::clone(&broker),
        },
        token,
        shutdown_rx,
    ));
    Ok(BridgeHandle {
        shutdown: shutdown_tx,
        stopped: AtomicBool::new(false),
        accept_task: Some(accept_task),
        broker,
        data_dir,
        socket_path,
    })
}

/// What every connection task shares.
#[derive(Clone)]
pub(crate) struct Shared {
    pub(crate) accounts: Arc<dyn BridgeAccounts>,
    pub(crate) log: Arc<events::EventLog>,
    pub(crate) activity_pages: Arc<crate::recent_activity::ActivityPages>,
    pub(crate) broker: Arc<ConsentBroker>,
}

fn spawn_sweeper(broker: Arc<ConsentBroker>, mut shutdown: watch::Receiver<bool>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        return;
                    }
                }
                _ = ticker.tick() => broker.sweep(),
            }
        }
    });
}

async fn accept_loop(
    #[allow(unused_mut)] mut listener: Listener,
    shared: Shared,
    token: String,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut connections = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    break;
                }
            }
            // Reap finished connections so the set does not grow.
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
            accepted = listener.accept() => {
                match accepted {
                    #[cfg(unix)]
                    Ok((stream, _addr)) => {
                        connections.spawn(serve_connection(
                            stream,
                            shared.clone(),
                            token.clone(),
                            shutdown.clone(),
                        ));
                    }
                    #[cfg(windows)]
                    Ok(stream) => {
                        connections.spawn(serve_connection(
                            stream,
                            shared.clone(),
                            token.clone(),
                            shutdown.clone(),
                        ));
                    }
                    Err(_) => {
                        // Transient accept failure; give the runtime a moment
                        // rather than spinning on a broken listener.
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
            }
        }
    }
    // Stop accepting at once, then wait for every connection, which watches
    // the same signal, to close.
    drop(listener);
    while connections.join_next().await.is_some() {}
}

async fn write_message(
    writer: &mut (impl AsyncWriteExt + Unpin),
    message: &ServerMessage,
) -> std::io::Result<()> {
    let Ok(encoded) = encode_line(message) else {
        return Err(std::io::Error::other("encode"));
    };
    writer.write_all(encoded.as_bytes()).await?;
    writer.flush().await
}

async fn serve_connection(
    stream: Stream,
    shared: Shared,
    token: String,
    mut shutdown: watch::Receiver<bool>,
) {
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);

    let (client, protocol, hello_name) = match read_hello(
        &mut reader,
        &token,
        shared.broker.data_dir(),
    )
    .await
    {
        Ok(Hello::Accepted {
            identity,
            protocol,
            declared_name,
        }) => {
            shared.broker.note_current_launcher(&declared_name);
            (identity, protocol, declared_name)
        }
        Ok(Hello::ProfileRejected(presented_id)) => {
            // One reply for an unknown id, a wrong secret and an unreadable
            // store, written before anything that differs between them. The
            // rejection is recorded after the stream is closed and off this
            // task, so the peer does not wait for it. (The store is read for
            // every presented id; the remaining timing difference is not
            // claimed to be zero.)
            let _ = write_message(
                &mut write_half,
                &ServerMessage::Error {
                    code: ErrorCode::Unauthorized,
                    message: PROFILE_REJECTED_MESSAGE.to_owned(),
                    reason: Some("profile_secret".to_owned()),
                    supported: None,
                },
            )
            .await;
            let _ = tokio::io::AsyncWriteExt::shutdown(&mut write_half).await;
            drop(write_half);
            let data_dir = shared.broker.data_dir().to_owned();
            tokio::task::spawn_blocking(move || profiles::note_rejected(&data_dir, &presented_id));
            return;
        }
        Ok(Hello::Outdated(name)) => {
            if let Some(name) = name {
                shared.broker.note_outdated_launcher(&name);
            }
            let _ = write_message(
                &mut write_half,
                &ServerMessage::Error {
                    code: ErrorCode::Unsupported,
                    message: "this koushi-mcp launcher is older than the running Koushi; register it again"
                        .to_owned(),
                    reason: Some("protocol_version".to_owned()),
                    supported: Some(SUPPORTED_PROTOCOLS.to_vec()),
                },
            )
            .await;
            return;
        }
        Ok(Hello::Rejected(error)) => {
            let _ = write_message(
                &mut write_half,
                &ServerMessage::Error {
                    code: error.code,
                    message: error.message,
                    reason: error.reason,
                    supported: None,
                },
            )
            .await;
            return;
        }
        Err(_) => return,
    };

    if write_message(
        &mut write_half,
        &ServerMessage::HelloOk {
            protocol,
            connection_id: format!("{}-{}", hello_name, std::process::id()),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
        },
    )
    .await
    .is_err()
    {
        return;
    }

    // A profiled connection ends when its profile is revoked. Subscribed
    // before the first request, and every request re-checks the row anyway, so
    // a missed signal cannot keep a revoked profile working.
    let mut revoked = client
        .subject()
        .profile_id()
        .map(|id| shared.broker.watch_revocation(id));

    loop {
        let line = tokio::select! {
            _ = shutdown.changed() => return,
            _ = revoked_signal(&mut revoked) => return,
            read = koushi_mcp::ipc::read_frame(&mut reader) => match read {
                Ok(line) => line,
                Err(_) => return,
            },
        };
        if line.is_empty() {
            return;
        }
        if line.len() > MAX_FRAME_BYTES {
            let _ = write_message(
                &mut write_half,
                &ServerMessage::Error {
                    code: ErrorCode::Internal,
                    message: "frame too large".to_owned(),
                    reason: None,
                    supported: None,
                },
            )
            .await;
            return;
        }
        let response = match decode_line::<ClientMessage>(&line) {
            Ok(ClientMessage::Request { id, method, params }) => {
                // The launcher sends one request at a time, so while a call
                // runs, the only thing to watch for is the client going away
                // (it cancelled, or was killed) or the bridge stopping. Either
                // drops the call before it can act, including a send that is
                // still waiting for approval.
                let dispatch = dispatch_method(&shared, &client, protocol, &method, params);
                tokio::select! {
                    biased;
                    _ = shutdown.changed() => return,
                    _ = closed(&mut reader) => return,
                    // A revocation answers the call that was waiting, with no
                    // data, and ends the connection.
                    _ = revoked_signal(&mut revoked) => {
                        let _ = write_message(
                            &mut write_half,
                            &ServerMessage::failure(id, crate::consent::profile_revoked()),
                        )
                        .await;
                        return;
                    }
                    result = dispatch => {
                        // Stopping signals the connection and then releases
                        // waiting approvals, and one poll can observe the
                        // second without the first. A stopped bridge closes
                        // the connection; it never answers.
                        if *shutdown.borrow() {
                            return;
                        }
                        match result {
                            Ok(value) => ServerMessage::success(id, value),
                            Err(error) => ServerMessage::failure(id, error),
                        }
                    }
                }
            }
            Ok(ClientMessage::Hello { .. }) => ServerMessage::failure(
                0,
                WireError::new(ErrorCode::Internal, "duplicate handshake"),
            ),
            Err(_) => {
                ServerMessage::failure(0, WireError::new(ErrorCode::Internal, "malformed request"))
            }
        };
        if write_message(&mut write_half, &response).await.is_err() {
            return;
        }
    }
}

/// Resolves when the connection's profile was revoked; never for an
/// unprofiled connection.
async fn revoked_signal(revoked: &mut Option<watch::Receiver<bool>>) {
    match revoked {
        Some(receiver) => {
            // A sent value and a dropped sender both mean the profile is gone.
            let _ = receiver.wait_for(|revoked| *revoked).await;
        }
        None => std::future::pending::<()>().await,
    }
}

/// Resolves when the peer closed its side. Data that arrives meanwhile stays
/// buffered for the next read, and this future then never resolves.
async fn closed<R: tokio::io::AsyncRead + Unpin>(reader: &mut BufReader<R>) {
    let pending_data = match reader.fill_buf().await {
        Ok(buffer) => !buffer.is_empty(),
        Err(_) => false,
    };
    if pending_data {
        std::future::pending::<()>().await;
    }
}

/// The one message for every failed profile authentication.
const PROFILE_REJECTED_MESSAGE: &str = "agent profile id or secret rejected";

enum Hello {
    /// The identity and the protocol version negotiated for this connection.
    /// `declared_name` is the unverified name the launcher sent; it clears the
    /// "outdated launcher" note and nothing else.
    Accepted {
        identity: ClientIdentity,
        protocol: u32,
        declared_name: String,
    },
    Rejected(WireError),
    /// The token matched and a profile was presented, but it did not verify.
    /// Carries the presented id only to record the rejection.
    ProfileRejected(String),
    /// The token matched, but the launcher speaks another bridge protocol:
    /// a copy left over from an older Koushi (such as a Claude Desktop
    /// extension). Carries its name when that name is valid.
    Outdated(Option<String>),
}

/// Reads and validates the handshake.
async fn read_hello<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    expected_token: &str,
    data_dir: &std::path::Path,
) -> std::io::Result<Hello> {
    let unauthorized = || {
        Hello::Rejected(WireError::new(
            ErrorCode::Unauthorized,
            "connection token rejected",
        ))
    };
    let line = tokio::time::timeout(Duration::from_secs(5), koushi_mcp::ipc::read_frame(reader))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "handshake timeout"))??;
    if line.is_empty() {
        return Ok(unauthorized());
    }
    let Ok(ClientMessage::Hello {
        protocol,
        token,
        client,
        profile,
    }) = decode_line::<ClientMessage>(&line)
    else {
        return Ok(unauthorized());
    };
    if !constant_time_eq(token.as_bytes(), expected_token.as_bytes()) {
        return Ok(unauthorized());
    }
    // Checked after the token, so only a launcher that may connect at all can
    // put its name on the settings page as outdated.
    if !SUPPORTED_PROTOCOLS.contains(&protocol) {
        return Ok(Hello::Outdated(
            koushi_mcp::wire::valid_client_name(&client.name).then_some(client.name),
        ));
    }
    if !koushi_mcp::wire::valid_client_name(&client.name) {
        return Ok(Hello::Rejected(WireError::with_reason(
            ErrorCode::Unsupported,
            "client_name",
            koushi_mcp::wire::CLIENT_NAME_RULE,
        )));
    }
    let version = if koushi_mcp::wire::valid_client_version(&client.version) {
        client.version.clone()
    } else {
        String::new()
    };
    let identity = match (protocol, profile) {
        // A v1 peer cannot know the field, so a profile is never accepted on
        // the legacy path.
        (LEGACY_PROTOCOL, Some(_)) => {
            return Ok(Hello::Rejected(WireError::with_reason(
                ErrorCode::Unsupported,
                "profile_requires_protocol_2",
                "a profile needs protocol 2",
            )));
        }
        (LEGACY_PROTOCOL, None) => ClientIdentity::unprofiled(&client.name, &version),
        (_, None) => {
            return Ok(Hello::Rejected(WireError::with_reason(
                ErrorCode::Unsupported,
                "profile_required",
                "protocol 2 needs an agent profile",
            )));
        }
        // An unknown id, a malformed id, a wrong secret and an unreadable
        // store are one outcome, so neither the reply nor its timing says
        // which profile ids exist. The profile's label, set by the owner,
        // names the connection; the name the launcher declared never keys
        // anything.
        (_, Some(presented)) => {
            match profiles::authenticate(data_dir, &presented.id, &presented.secret) {
                Some(row) => ClientIdentity::profiled(
                    row.id.clone(),
                    &row.label,
                    &version,
                    row.account_id.clone(),
                ),
                None => return Ok(Hello::ProfileRejected(presented.id)),
            }
        }
    };
    Ok(Hello::Accepted {
        identity,
        protocol,
        declared_name: client.name,
    })
}

/// Windows named-pipe listener. Kept small: one instance is prepared before the
/// current one is handed to a connection task.
#[cfg(windows)]
struct NamedPipeListener {
    name: String,
    server: tokio::net::windows::named_pipe::NamedPipeServer,
}

#[cfg(windows)]
impl NamedPipeListener {
    /// `first` is true only for the very first instance; a later instance may
    /// not claim `first_pipe_instance`, or Windows rejects the bind.
    fn bind(name: &str, first: bool) -> Result<Self, String> {
        use tokio::net::windows::named_pipe::ServerOptions;
        let security = koushi_mcp::windows_security::PrivateSecurity::new()
            .map_err(|_| "mcp pipe ACL".to_owned())?;
        let mut attributes = security.attributes();
        // SAFETY: descriptor and attributes stay alive through synchronous
        // CreateNamedPipe; Windows copies the descriptor before returning.
        let server = unsafe {
            ServerOptions::new()
                .first_pipe_instance(first)
                .reject_remote_clients(true)
                .create_with_security_attributes_raw(
                    name,
                    std::ptr::from_mut(&mut attributes).cast(),
                )
        }
        .map_err(|_| "mcp pipe".to_owned())?;
        Ok(Self {
            name: name.to_owned(),
            server,
        })
    }

    async fn accept(&mut self) -> std::io::Result<Stream> {
        self.server.connect().await?;
        // Install the next instance before handing this one to a connection
        // task, so a client that connects immediately still finds a pipe.
        let next = Self::bind(&self.name, false).map_err(std::io::Error::other)?;
        let connected = std::mem::replace(&mut self.server, next.server);
        Ok(connected)
    }
}

/// Every method `dispatch_method` serves. The table is the gate: an arm that is
/// not listed here is never reached, so no method can be served without its row
/// in the no-write test, which is generated from this list.
pub(crate) const METHODS: &[&str] = &["tools/call", "status"];

/// Dispatches one IPC request.
async fn dispatch_method(
    shared: &Shared,
    client: &ClientIdentity,
    protocol: u32,
    method: &str,
    params: Value,
) -> Result<Value, WireError> {
    match method {
        other if !METHODS.contains(&other) => Err(WireError::new(
            ErrorCode::Unsupported,
            format!("unsupported bridge method: {other}"),
        )),
        "tools/call" => handlers::dispatch_tool_call(shared, client, params).await,
        "status" => Ok(serde_json::json!({
            "protocol": protocol,
            "app_version": env!("CARGO_PKG_VERSION")
        })),
        other => Err(WireError::new(
            ErrorCode::Unsupported,
            format!("unsupported bridge method: {other}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consent::GrantSubject;

    async fn handshake(input: &str, expected_token: &str) -> Hello {
        let (mut client, server) = tokio::io::duplex(8 * 1024);
        client.write_all(input.as_bytes()).await.expect("write");
        client.shutdown().await.expect("shutdown");
        let mut reader = BufReader::new(server);
        read_hello(
            &mut reader,
            expected_token,
            std::path::Path::new("/nonexistent-koushi-test"),
        )
        .await
        .expect("read")
    }

    fn hello(token: &str, name: &str, version: &str) -> String {
        format!(
            "{}\n",
            serde_json::json!({
                "type": "hello", "protocol": 1, "token": token,
                "client": { "name": name, "version": version }
            })
        )
    }

    fn rejected(hello: Hello) -> ErrorCode {
        match hello {
            Hello::Rejected(error) => error.code,
            Hello::Accepted { .. } => panic!("accepted"),
            Hello::ProfileRejected(_) => panic!("profile rejected"),
            Hello::Outdated(_) => panic!("outdated"),
        }
    }

    #[tokio::test]
    async fn accepts_the_matching_token_and_returns_client_info() {
        let Hello::Accepted {
            identity,
            declared_name,
            ..
        } = handshake(&hello("secret", "codex", "0.147.0"), "secret").await
        else {
            panic!("rejected");
        };
        assert_eq!(declared_name, "codex");
        assert_eq!(identity.label().to_string(), "codex");
        assert_eq!(identity.subject(), &GrantSubject::client("codex"));
        assert_eq!(identity.version(), "0.147.0");
    }

    #[tokio::test]
    async fn rejects_a_wrong_token() {
        let result = handshake(&hello("guess", "codex", "0"), "secret").await;
        assert_eq!(rejected(result), ErrorCode::Unauthorized);
    }

    #[tokio::test]
    async fn an_authenticated_launcher_with_another_protocol_is_reported_outdated() {
        let input = concat!(
            r#"{"type":"hello","protocol":9,"token":"secret","client":{"name":"claude-desktop","version":"0"}}"#,
            "\n"
        );
        assert!(matches!(
            handshake(input, "secret").await,
            Hello::Outdated(Some(name)) if name == "claude-desktop"
        ));
        let unauthenticated = concat!(
            r#"{"type":"hello","protocol":9,"token":"guess","client":{"name":"claude-desktop","version":"0"}}"#,
            "\n"
        );
        assert_eq!(
            rejected(handshake(unauthenticated, "secret").await),
            ErrorCode::Unauthorized
        );
    }

    #[tokio::test]
    async fn rejects_a_non_hello_first_message() {
        let input = r#"{"type":"request","id":1,"method":"tools/call"}"#.to_owned() + "\n";
        assert_eq!(
            rejected(handshake(&input, "secret").await),
            ErrorCode::Unauthorized
        );
    }

    #[tokio::test]
    async fn rejects_a_client_name_that_could_mislead_the_dialog() {
        for name in [
            "",
            "claude\u{202E}edoc",
            "a b",
            "line\nbreak",
            &"x".repeat(65),
        ] {
            let result = handshake(&hello("secret", name, "1"), "secret").await;
            assert_eq!(rejected(result), ErrorCode::Unsupported, "{name:?}");
        }
    }

    #[tokio::test]
    async fn drops_an_unusable_version_but_keeps_the_client() {
        let Hello::Accepted { identity, .. } =
            handshake(&hello("secret", "pi", "1.0\u{202E}"), "secret").await
        else {
            panic!("rejected");
        };
        assert_eq!(identity.version(), "");
    }

    #[tokio::test]
    async fn closed_resolves_on_eof_but_not_on_buffered_data() {
        let (client, server) = tokio::io::duplex(1024);
        let mut reader = BufReader::new(server);
        drop(client);
        tokio::time::timeout(Duration::from_secs(1), closed(&mut reader))
            .await
            .expect("eof seen");

        let (mut client, server) = tokio::io::duplex(1024);
        client.write_all(b"next\n").await.expect("write");
        let mut reader = BufReader::new(server);
        assert!(
            tokio::time::timeout(Duration::from_millis(50), closed(&mut reader))
                .await
                .is_err()
        );
        let mut line = String::new();
        reader.read_line(&mut line).await.expect("read");
        assert_eq!(line, "next\n", "buffered data is kept for the next read");
    }

    #[test]
    fn constant_time_comparison_is_length_aware() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
