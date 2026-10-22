//! `send_message` with local-file attachments (#1533): the advertised
//! `attachments` argument is honoured, never silently dropped. An
//! attachment-only call is a valid send, sending files needs the attachments
//! permission as well as send, and arguments that can never succeed fail with a
//! typed error before any approval is raised.
#![cfg(unix)]

mod support;

use std::sync::Arc;
use std::time::Duration;

use koushi_core::runtime::CoreRuntime;
use koushi_mcp_host::consent::{ConsentBroker, ConsentMode};
use koushi_mcp_host::{BridgeAccount, BridgeAccounts};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

const ACCOUNT: &str = "account:@member1:example.invalid";
const ROOM: &str = "!room:example.invalid";

struct OneAccount {
    core: Arc<CoreRuntime>,
    media: std::path::PathBuf,
}

impl BridgeAccounts for OneAccount {
    fn accounts(&self) -> Vec<BridgeAccount> {
        vec![BridgeAccount {
            id: ACCOUNT.to_owned(),
            homeserver: None,
            connection: self.core.attach(),
            media_cache_dir: self.media.clone(),
        }]
    }
}

struct Harness {
    dir: tempfile::TempDir,
    data_dir: std::path::PathBuf,
    bridge: koushi_mcp_host::BridgeHandle,
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
}

impl Harness {
    /// A fail-closed bridge: a missing permission answers `consent_required`
    /// at once, naming the permission, so the test sees which one was asked.
    async fn start(grant: Option<Value>) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let data_dir = dir.path().join("app");
        if let Some(grant) = grant {
            std::fs::create_dir_all(data_dir.join("mcp")).expect("mcp dir");
            std::fs::write(
                data_dir.join("mcp/connections.json"),
                json!({
                    "schema": 2,
                    "connections": [{ "client_name": "pi", "accounts": [grant] }]
                })
                .to_string(),
            )
            .expect("grants");
        }
        let core = Arc::new(CoreRuntime::start_with_data_dir(dir.path().join("core")));
        // A call that passes argument validation must reach the permissions
        // under test, so the account is ready (#1580).
        support::sign_in(&core).await;
        let media = dir.path().join("media");
        std::fs::create_dir_all(&media).expect("media dir");
        let accounts: Arc<dyn BridgeAccounts> = Arc::new(OneAccount { core, media });
        let broker = ConsentBroker::new(data_dir.clone(), ConsentMode::FailClosed);
        let bridge = koushi_mcp_host::start(accounts, data_dir.clone(), broker).expect("bridge");

        let endpoint =
            koushi_mcp::endpoint::read(&data_dir.join("mcp/endpoint.json")).expect("endpoint");
        let stream = UnixStream::connect(&endpoint.endpoint)
            .await
            .expect("connect");
        let (read, writer) = stream.into_split();
        let mut harness = Self {
            dir,
            data_dir,
            bridge,
            reader: BufReader::new(read),
            writer,
        };
        harness
            .send(json!({
                "type": "hello", "protocol": 1, "token": endpoint.token,
                "client": { "name": "pi", "version": "1.0.0" }
            }))
            .await;
        let hello = harness.line().await;
        assert!(hello.contains("hello_ok"), "{hello}");
        harness
    }

    async fn send(&mut self, value: Value) {
        self.writer
            .write_all(format!("{value}\n").as_bytes())
            .await
            .expect("write");
    }

    async fn line(&mut self) -> String {
        let mut line = String::new();
        tokio::time::timeout(Duration::from_secs(5), self.reader.read_line(&mut line))
            .await
            .expect("reply in time")
            .expect("read");
        line
    }

    /// The tool result text of one `send_message` call.
    async fn send_message(&mut self, arguments: Value) -> String {
        self.send(json!({
            "type": "request", "id": 1, "method": "tools/call",
            "params": { "name": "send_message", "arguments": arguments }
        }))
        .await;
        self.line().await
    }

    fn file(&self, name: &str, contents: &[u8]) -> String {
        let path = self.dir.path().join(name);
        std::fs::write(&path, contents).expect("write fixture");
        path.to_string_lossy().into_owned()
    }

    async fn stop(self) {
        let _ = &self.data_dir;
        self.bridge.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_attachment_only_send_is_not_refused_as_an_empty_body() {
    let mut harness = Harness::start(None).await;
    let path = harness.file("fixture_notes.txt", b"synthetic notes");
    let reply = harness
        .send_message(json!({
            "account_id": ACCOUNT, "room_id": ROOM, "client_request_id": "req-1",
            "attachments": [{ "path": path }]
        }))
        .await;
    assert!(!reply.contains("body is empty"), "{reply}");
    assert!(reply.contains("consent_required"), "{reply}");
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sending_files_also_needs_the_attachments_permission() {
    let mut harness = Harness::start(Some(json!({
        "account_id": ACCOUNT, "read": true, "send": true, "attachments": false
    })))
    .await;
    let path = harness.file("fixture_notes.txt", b"synthetic notes");
    let reply = harness
        .send_message(json!({
            "account_id": ACCOUNT, "room_id": ROOM, "client_request_id": "req-2",
            "body": "Synthetic caption", "attachments": [{ "path": path }]
        }))
        .await;
    assert!(reply.contains("consent_required"), "{reply}");
    assert!(reply.contains("attachment access"), "{reply}");
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unusable_attachment_arguments_fail_before_consent() {
    let mut harness = Harness::start(None).await;
    let absolute = harness.file("fixture_notes.txt", b"synthetic notes");
    let missing = harness
        .dir
        .path()
        .join("missing.txt")
        .to_string_lossy()
        .into_owned();
    let cases = [
        (
            json!([{ "path": "fixture_notes.txt" }]),
            None,
            "attachment paths must be absolute",
        ),
        (
            json!([{ "path": missing }]),
            None,
            "an attachment file cannot be read",
        ),
        (
            json!([{ "path": absolute }]),
            Some("$parent:example.invalid"),
            "reply_to_event_id cannot be combined with attachments",
        ),
    ];
    for (attachments, reply_to, expected) in cases {
        let mut arguments = json!({
            "account_id": ACCOUNT, "room_id": ROOM, "client_request_id": "req-3",
            "body": "Synthetic caption", "attachments": attachments
        });
        if let Some(reply_to) = reply_to {
            arguments["reply_to_event_id"] = json!(reply_to);
        }
        let reply = harness.send_message(arguments).await;
        assert!(reply.contains("unsupported"), "{reply}");
        assert!(reply.contains(expected), "{reply}");
        assert!(!reply.contains("consent_required"), "{reply}");
    }
    harness.stop().await;
}
