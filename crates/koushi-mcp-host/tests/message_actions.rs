//! `get_message` and the five message actions (#1628) at the transport
//! boundary: a call that can never succeed is refused before any approval, the
//! new `message_actions` grant is asked for as its own permission, and a
//! granted call reaches the account. `tools/list` is answered by the launcher
//! and is covered in `koushi-mcp`.
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

fn error_code(reply: &Value) -> String {
    reply["result"]["structuredContent"]["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn error_reason(reply: &Value) -> String {
    reply["result"]["structuredContent"]["error"]["reason"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn result_text(reply: &Value) -> String {
    reply["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// `message_actions` is its own grant: a read grant does not cover deleting a
/// message, and the refusal names the permission instead of the assistant's
/// prose.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn message_actions_is_a_separate_permission() {
    let mut harness = Harness::start(Some(Harness::account_grant(true, true, false))).await;
    let reply = harness
        .call(
            "delete_message",
            json!({ "account_id": ACCOUNT, "room_id": ROOM, "event_id": EVENT_ID }),
        )
        .await;
    assert_eq!(error_code(&reply), "consent_required");
    assert!(
        result_text(&reply).contains("message actions"),
        "{}",
        result_text(&reply)
    );
    harness.stop().await;
}

/// An argument that can never succeed fails before an approval is raised, so a
/// malformed call never produces a dialog.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_arguments_are_refused_before_any_approval() {
    let mut harness = Harness::start(None).await;

    let control_key = harness
        .call(
            "react_to_message",
            json!({
                "account_id": ACCOUNT, "room_id": ROOM, "event_id": EVENT_ID,
                "key": "ok\nand more"
            }),
        )
        .await;
    assert_eq!(error_code(&control_key), "unsupported");

    let long_key = "x".repeat(129);
    let long_key = harness
        .call(
            "react_to_message",
            json!({
                "account_id": ACCOUNT, "room_id": ROOM, "event_id": EVENT_ID, "key": long_key
            }),
        )
        .await;
    assert_eq!(error_code(&long_key), "unsupported");

    let bad_id = harness
        .call(
            "edit_message",
            json!({
                "account_id": ACCOUNT, "room_id": ROOM, "event_id": EVENT_ID,
                "body": "corrected", "client_request_id": "has spaces"
            }),
        )
        .await;
    assert_eq!(error_code(&bad_id), "unsupported");

    let empty_body = harness
        .call(
            "send_thread_reply",
            json!({
                "account_id": ACCOUNT, "room_id": ROOM, "thread_root_event_id": EVENT_ID,
                "body": "   ", "client_request_id": "uuid-1"
            }),
        )
        .await;
    assert_eq!(error_code(&empty_body), "unsupported");

    harness.stop().await;
}

/// A call that passes validation and holds the grant reaches the account, so a
/// tool that was never dispatched cannot be mistaken for an unready account.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_granted_read_reaches_the_account() {
    let mut harness = Harness::start(Some(Harness::account_grant(true, false, false))).await;
    let reply = harness
        .call(
            "get_message",
            json!({ "account_id": ACCOUNT, "room_id": ROOM, "event_id": EVENT_ID }),
        )
        .await;
    // This fixture has a reducer session but no SDK session, so the account
    // answers instead of the tool being unknown.
    assert_eq!(error_code(&reply), "account_unavailable");
    assert_eq!(error_reason(&reply), "");
    harness.stop().await;
}
