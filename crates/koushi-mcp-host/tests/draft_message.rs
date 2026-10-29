//! `draft_message` (#1644) at the transport boundary: `draft` is its own
//! permission, weaker than and separate from `send`; a revocation applies on
//! the next call; a granted draft reaches the account's Rust draft store and a
//! second draft into the same composer is a `conflict`.
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
        support::apply_actions(
            &core,
            vec![koushi_state::AppAction::RoomListUpdated {
                spaces: Vec::new(),
                rooms: vec![support::room(ROOM)],
            }],
        )
        .await;
        support::wait_for_room(&core, ROOM).await;
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

    fn account_grant(send: bool, draft: bool) -> Value {
        json!({
            "account_id": ACCOUNT,
            "read": false,
            "send": send,
            "attachments": false,
            "message_actions": false,
            "draft": draft,
        })
    }

    /// Rewrites the grant store, as the app does when the user changes a
    /// permission. Every call re-reads it.
    fn write_grant(&self, grant: Value) {
        std::fs::write(
            self.dir.path().join("app/mcp/connections.json"),
            json!({
                "schema": 3,
                "connections": [{ "client_name": "pi", "accounts": [grant] }]
            })
            .to_string(),
        )
        .expect("grants");
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

fn draft(body: &str) -> Value {
    json!({ "account_id": ACCOUNT, "room_id": ROOM, "body": body })
}

/// A send grant does not cover drafting: the refusal names the draft
/// permission.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn draft_is_a_separate_permission_from_send() {
    let mut harness = Harness::start(Some(Harness::account_grant(true, false))).await;
    let reply = harness.call("draft_message", draft("hello")).await;
    assert_eq!(error_code(&reply), "consent_required");
    assert!(
        result_text(&reply).contains("draft"),
        "{}",
        result_text(&reply)
    );
    harness.stop().await;
}

/// `draft` alone never lets the assistant send.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn draft_without_send_cannot_send() {
    let mut harness = Harness::start(Some(Harness::account_grant(false, true))).await;
    let reply = harness
        .call(
            "send_message",
            json!({
                "account_id": ACCOUNT, "room_id": ROOM, "body": "hello",
                "client_request_id": "uuid-1"
            }),
        )
        .await;
    assert_eq!(error_code(&reply), "consent_required");
    assert!(
        result_text(&reply).contains("send"),
        "{}",
        result_text(&reply)
    );
    harness.stop().await;
}

/// A granted draft reaches the account's draft store; a second one into the
/// same composer is a conflict and leaves the first in place; revoking `draft`
/// applies to the next call.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_granted_draft_is_staged_once_then_conflicts_and_revocation_applies() {
    let mut harness = Harness::start(Some(Harness::account_grant(false, true))).await;

    let first = wait_until_staged(&mut harness).await;
    assert_eq!(first["result"]["structuredContent"]["status"], "staged");
    assert_eq!(first["result"]["structuredContent"]["room_id"], ROOM);
    assert!(first["result"]["structuredContent"]["thread_root_event_id"].is_null());

    let second = harness.call("draft_message", draft("replacement")).await;
    assert_eq!(error_code(&second), "conflict");

    harness.write_grant(Harness::account_grant(false, false));
    let revoked = harness.call("draft_message", draft("after revoke")).await;
    assert_eq!(error_code(&revoked), "consent_required");
    harness.stop().await;
}

/// The account's draft store loads after sign-in; until then a draft answers
/// `busy` with `drafts_loading` and changes nothing.
async fn wait_until_staged(harness: &mut Harness) -> Value {
    for _ in 0..200 {
        let reply = harness
            .call("draft_message", draft("from the assistant"))
            .await;
        if error_reason(&reply) != "drafts_loading" {
            assert_eq!(error_code(&reply), "", "{reply}");
            return reply;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the draft store did not load");
}

/// Arguments that can never succeed fail before an approval is raised.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_drafts_are_refused_before_any_approval() {
    let mut harness = Harness::start(None).await;
    let empty = harness.call("draft_message", draft("   ")).await;
    assert_eq!(error_code(&empty), "unsupported");
    let long = harness
        .call("draft_message", draft(&"x".repeat(16 * 1024 + 1)))
        .await;
    assert_eq!(error_reason(&long), "body_too_long");
    harness.stop().await;
}

/// A room the account has not joined is reported after consent, so an
/// unapproved client cannot probe membership.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unjoined_room_is_reported_only_after_consent() {
    let mut harness = Harness::start(None).await;
    let unapproved = harness
        .call(
            "draft_message",
            json!({ "account_id": ACCOUNT, "room_id": "!elsewhere:example.invalid", "body": "x" }),
        )
        .await;
    assert_eq!(error_code(&unapproved), "consent_required");
    harness.write_grant(Harness::account_grant(false, true));
    let approved = harness
        .call(
            "draft_message",
            json!({ "account_id": ACCOUNT, "room_id": "!elsewhere:example.invalid", "body": "x" }),
        )
        .await;
    assert_eq!(error_code(&approved), "room_not_found");
    harness.stop().await;
}
