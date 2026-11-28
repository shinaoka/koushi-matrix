//! #2165: while send is not admitted (Secure Backup not ready), every room
//! writing tool answers the documented refusal `not_permitted` /
//! `send_not_admitted`, never `internal` and never a Rust `Debug` rendering.
//! The fixture signs in without an SDK session, so no homeserver is reachable.
#![cfg(unix)]

#[path = "../support/mod.rs"]
#[expect(
    clippy::duplicate_mod,
    reason = "suite aggregation preserves feature-local fixture scopes and registered helper tests"
)]
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
const EVENT_ID: &str = "$event:example.invalid";

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
    bridge: koushi_mcp_host::BridgeHandle,
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
}

impl Harness {
    /// A fail-closed bridge: a missing permission answers `consent_required`
    /// at once, naming the permission, instead of waiting for a dialog.
    async fn start(grant: Option<Value>) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let data_dir = dir.path().join("app");
        if let Some(grant) = grant {
            std::fs::create_dir_all(data_dir.join("mcp")).expect("mcp dir");
            std::fs::write(
                data_dir.join("mcp/connections.json"),
                json!({
                    "schema": 3,
                    "connections": [{ "client_name": "pi", "accounts": [grant] }]
                })
                .to_string(),
            )
            .expect("grants");
        }
        let core = Arc::new(CoreRuntime::start_with_data_dir(dir.path().join("core")));
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

    async fn call(&mut self, name: &str, arguments: Value) -> Value {
        self.send(json!({
            "type": "request", "id": 1, "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        }))
        .await;
        let line = self.line().await;
        serde_json::from_str(line.trim()).expect("json reply")
    }

    fn account_grant(read: bool, send: bool, message_actions: bool) -> Value {
        json!({
            "account_id": ACCOUNT,
            "read": read,
            "send": send,
            "attachments": false,
            "message_actions": message_actions,
        })
    }

    async fn stop(self) {
        let _ = &self.dir;
        self.bridge.shutdown().await;
    }
}

const REFUSAL: &str = "sending is unavailable until Secure Backup is ready";

fn error(reply: &Value) -> &Value {
    &reply["result"]["structuredContent"]["error"]
}

fn assert_send_not_admitted(reply: &Value) {
    assert_eq!(error(reply)["code"], json!("not_permitted"), "{reply}");
    assert_eq!(
        error(reply)["reason"],
        json!("send_not_admitted"),
        "{reply}"
    );
    assert_eq!(error(reply)["message"], json!(REFUSAL), "{reply}");
    assert_no_rust_debug(reply);
}

fn assert_no_rust_debug(reply: &Value) {
    let text = reply.to_string();
    for leaked in [
        "TimelineOperationFailed",
        "SecureBackupRequired",
        "kind:",
        "{ ",
    ] {
        assert!(!text.contains(leaked), "`{leaked}` leaked into {text}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_message_reports_the_lock_as_not_permitted() {
    let mut harness = Harness::start(Some(Harness::account_grant(true, true, true))).await;
    let reply = harness
        .call(
            "send_message",
            json!({
                "account_id": ACCOUNT, "room_id": ROOM,
                "body": "Synthetic text", "client_request_id": "uuid-2165-a",
            }),
        )
        .await;
    assert_send_not_admitted(&reply);
    harness.stop().await;
}

/// The homeserver is unreachable (no SDK session is installed), so a thread
/// root lookup cannot succeed: the refusal proves admission is checked first.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_thread_reply_reports_the_lock_before_any_lookup() {
    let mut harness = Harness::start(Some(Harness::account_grant(true, true, true))).await;
    let reply = harness
        .call(
            "send_thread_reply",
            json!({
                "account_id": ACCOUNT, "room_id": ROOM,
                "thread_root_event_id": EVENT_ID,
                "body": "Synthetic text", "client_request_id": "uuid-2165-b",
            }),
        )
        .await;
    assert_send_not_admitted(&reply);
    harness.stop().await;
}
