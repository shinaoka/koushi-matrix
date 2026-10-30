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
use koushi_mcp::wire::{
    ClientMessage, ErrorCode, MAX_FRAME_BYTES, PROTOCOL_VERSION, ServerMessage, WireError,
    decode_line, encode_line,
};

use super::{endpoint, events, handlers};

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

    let client = match read_hello(&mut reader, &token).await {
        Ok(Hello::Accepted(client)) => {
            shared.broker.note_current_launcher(&client.name);
            client
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
            protocol: PROTOCOL_VERSION,
            connection_id: format!("{}-{}", client.name, std::process::id()),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
        },
    )
    .await
    .is_err()
    {
        return;
    }

    loop {
        let line = tokio::select! {
            _ = shutdown.changed() => return,
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
                let dispatch = dispatch_method(&shared, &client, &method, params);
                tokio::select! {
                    biased;
                    _ = shutdown.changed() => return,
                    _ = closed(&mut reader) => return,
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

enum Hello {
    Accepted(ClientIdentity),
    Rejected(WireError),
    /// The token matched, but the launcher speaks another bridge protocol:
    /// a copy left over from an older Koushi (such as a Claude Desktop
    /// extension). Carries its name when that name is valid.
    Outdated(Option<String>),
}

/// Reads and validates the handshake.
async fn read_hello<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    expected_token: &str,
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
    }) = decode_line::<ClientMessage>(&line)
    else {
        return Ok(unauthorized());
    };
    if !constant_time_eq(token.as_bytes(), expected_token.as_bytes()) {
        return Ok(unauthorized());
    }
    // Checked after the token, so only a launcher that may connect at all can
    // put its name on the settings page as outdated.
    if protocol != PROTOCOL_VERSION {
        return Ok(Hello::Outdated(
            koushi_mcp::wire::valid_client_name(&client.name).then_some(client.name),
        ));
    }
    if !koushi_mcp::wire::valid_client_name(&client.name) {
        return Ok(Hello::Rejected(WireError::new(
            ErrorCode::Unsupported,
            koushi_mcp::wire::CLIENT_NAME_RULE,
        )));
    }
    let version = if koushi_mcp::wire::valid_client_version(&client.version) {
        client.version
    } else {
        String::new()
    };
    Ok(Hello::Accepted(ClientIdentity {
        name: client.name,
        version,
    }))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (a, b) in left.iter().zip(right) {
        difference |= a ^ b;
    }
    difference == 0
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

/// Dispatches one IPC request.
async fn dispatch_method(
    shared: &Shared,
    client: &ClientIdentity,
    method: &str,
    params: Value,
) -> Result<Value, WireError> {
    match method {
        "tools/call" => handlers::dispatch_tool_call(shared, client, params).await,
        "status" => Ok(serde_json::json!({
            "protocol": PROTOCOL_VERSION,
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

    async fn handshake(input: &str, expected_token: &str) -> Hello {
        let (mut client, server) = tokio::io::duplex(8 * 1024);
        client.write_all(input.as_bytes()).await.expect("write");
        client.shutdown().await.expect("shutdown");
        let mut reader = BufReader::new(server);
        read_hello(&mut reader, expected_token).await.expect("read")
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
            Hello::Accepted(_) => panic!("accepted"),
            Hello::Outdated(_) => panic!("outdated"),
        }
    }

    #[tokio::test]
    async fn accepts_the_matching_token_and_returns_client_info() {
        let Hello::Accepted(client) =
            handshake(&hello("secret", "codex", "0.147.0"), "secret").await
        else {
            panic!("rejected");
        };
        assert_eq!(client.name, "codex");
        assert_eq!(client.version, "0.147.0");
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
        let Hello::Accepted(client) =
            handshake(&hello("secret", "pi", "1.0\u{202E}"), "secret").await
        else {
            panic!("rejected");
        };
        assert_eq!(client.version, "");
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
