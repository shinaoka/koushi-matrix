//! The CDP export conversation against a fake loopback DevTools server. No test
//! touches a real Element installation or any address outside 127.0.0.1.

use super::element_desktop::{ElementDesktopError, export_from_devtools};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tokio_tungstenite::{accept_async, tungstenite::Message};

/// Synthetic exported key, shaped like Element's own export.
const EXPORTED: &str = concat!(
    r#"[{"algorithm":"m.megolm.v1.aes-sha2","room_id":"!fixture:example.invalid","#,
    r#""sender_key":"Zml4dHVyZS1zZW5kZXIta2V5","session_id":"Zml4dHVyZS1zZXNzaW9u","#,
    r#""session_key":"Zml4dHVyZS1zZXNzaW9uLWtleQ==","#,
    r#""sender_claimed_keys":{"ed25519":"Zml4dHVyZS1lZDI1NTE5"},"#,
    r#""forwarding_curve25519_key_chain":[]}]"#
);

const WEBAPP: &str = "vector://vector/webapp/";
/// `{port}` stands for the fake server's own port.
const LOOPBACK_SOCKET: &str = "ws://127.0.0.1:{port}/devtools/page/1";

type Targets = dyn Fn(u16) -> Value + Send + Sync;
type Reply = dyn Fn() -> Value + Send + Sync;

struct FakeServer {
    port: u16,
    upgrades: Arc<AtomicUsize>,
}

fn page(url: &str, socket: &str) -> Value {
    json!([{"id": "page-1", "type": "page", "url": url, "webSocketDebuggerUrl": socket}])
}

fn exported_reply() -> Value {
    json!({"id": 1, "result": {"result": {"type": "string", "value": EXPORTED}}})
}

fn exception_reply() -> Value {
    json!({
        "id": 1,
        "result": {
            "result": {"type": "object", "subtype": "error", "description": "Error"},
            "exceptionDetails": {"text": "Uncaught (in promise)"},
        },
    })
}

/// Serves `/json/list` from `targets` and every `Runtime.evaluate` from
/// `evaluate`, one task per connection.
async fn serve<F, R>(targets: F, evaluate: R) -> FakeServer
where
    F: Fn(u16) -> Value + Send + Sync + 'static,
    R: Fn() -> Value + Send + Sync + 'static,
{
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let port = listener.local_addr().expect("local addr").port();
    let upgrades = Arc::new(AtomicUsize::new(0));
    let targets = Arc::new(targets);
    let evaluate = Arc::new(evaluate);
    let counter = Arc::clone(&upgrades);
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let targets = Arc::clone(&targets);
            let evaluate = Arc::clone(&evaluate);
            let counter = Arc::clone(&counter);
            tokio::spawn(
                async move { handle(stream, port, &*targets, &*evaluate, &counter).await },
            );
        }
    });
    FakeServer { port, upgrades }
}

/// One page target with `url`, whose socket URL is `socket` with `{port}` filled
/// in from the bound port.
async fn serve_page(url: &'static str, socket: &'static str, reply: fn() -> Value) -> FakeServer {
    serve(
        move |port| page(url, &socket.replace("{port}", &port.to_string())),
        reply,
    )
    .await
}

async fn handle(
    mut stream: TcpStream,
    port: u16,
    targets: &Targets,
    evaluate: &Reply,
    upgrades: &AtomicUsize,
) {
    // Peek first: the upgrade request must stay unread for `accept_async`.
    let mut buffer = [0_u8; 512];
    let peeked = stream.peek(&mut buffer).await.unwrap_or(0);
    if !String::from_utf8_lossy(&buffer[..peeked]).starts_with("GET /json/list") {
        upgrades.fetch_add(1, Ordering::SeqCst);
        let mut socket = accept_async(stream).await.expect("websocket upgrade");
        while let Some(Ok(Message::Text(_))) = socket.next().await {
            let reply = evaluate().to_string();
            socket
                .send(Message::Text(reply.into()))
                .await
                .expect("evaluate reply");
        }
        return;
    }
    // Drain the request head: closing with unread data can reset the socket.
    let mut request = [0_u8; 1024];
    let _ = stream.read(&mut request).await;
    let body = targets(port).to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .expect("http response");
}

#[tokio::test]
async fn an_exported_key_array_comes_back_from_the_webapp_target() {
    let server = serve_page(WEBAPP, LOOPBACK_SOCKET, exported_reply).await;
    let keys = export_from_devtools(server.port, Duration::from_secs(5))
        .await
        .expect("export");
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0]["room_id"], "!fixture:example.invalid");
    assert_eq!(keys[0]["algorithm"], "m.megolm.v1.aes-sha2");
    assert_eq!(server.upgrades.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_exception_details_reply_is_an_unavailable_export() {
    let server = serve_page(WEBAPP, LOOPBACK_SOCKET, exception_reply).await;
    let error = export_from_devtools(server.port, Duration::from_secs(5))
        .await
        .expect_err("unavailable");
    assert_eq!(error, ElementDesktopError::ExportUnavailable);
}

#[tokio::test]
async fn a_target_that_is_not_the_webapp_is_rejected() {
    let server = serve_page("https://example.invalid/", LOOPBACK_SOCKET, exported_reply).await;
    let error = export_from_devtools(server.port, Duration::from_millis(300))
        .await
        .expect_err("no target");
    assert_eq!(error, ElementDesktopError::NoWebappTarget);
    assert_eq!(server.upgrades.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_non_loopback_socket_url_is_never_dialled() {
    let server = serve_page(
        WEBAPP,
        "ws://192.0.2.1:{port}/devtools/page/1",
        exported_reply,
    )
    .await;
    let error = export_from_devtools(server.port, Duration::from_secs(5))
        .await
        .expect_err("invalid endpoint");
    assert_eq!(error, ElementDesktopError::InvalidEndpoint);
    // Give a stray upgrade time to arrive before asserting none did.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(server.upgrades.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_socket_url_on_another_port_is_rejected() {
    // Same host, one port up: only the port Koushi launched may be dialled.
    let server = serve(
        |port| {
            let socket = format!("ws://127.0.0.1:{}/devtools/page/1", port.wrapping_add(1));
            page(WEBAPP, &socket)
        },
        exported_reply,
    )
    .await;
    let error = export_from_devtools(server.port, Duration::from_secs(5))
        .await
        .expect_err("invalid endpoint");
    assert_eq!(error, ElementDesktopError::InvalidEndpoint);
    assert_eq!(server.upgrades.load(Ordering::SeqCst), 0);
}
