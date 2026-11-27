//! `wait_for_mentions` (#2110) at the transport boundary: an exact feed with
//! cursors, thread roots and `resync_required` for a cursor of another log.
#![cfg(unix)]

#[path = "../support/mod.rs"]
#[expect(
    clippy::duplicate_mod,
    reason = "suite aggregation preserves feature-local fixture scopes and registered helper tests"
)]
mod support;

use std::sync::Arc;
use std::time::Duration;

use koushi_core::mention_feed::{MentionCandidate, MentionFeed, MentionKind};
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
    feed: MentionFeed,
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

    fn mention_feed(&self, account_id: &str) -> Option<MentionFeed> {
        (account_id == ACCOUNT).then(|| self.feed.clone())
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
    async fn start(grant: Option<Value>, feed: MentionFeed) -> Self {
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
        let accounts: Arc<dyn BridgeAccounts> = Arc::new(OneAccount { core, media, feed });
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

fn error_reason(reply: &Value) -> String {
    reply["result"]["structuredContent"]["error"]["reason"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn error_code(reply: &Value) -> String {
    reply["result"]["structuredContent"]["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn candidate(event: &str, thread_root: Option<&str>) -> MentionCandidate {
    MentionCandidate {
        room_id: ROOM.to_owned(),
        event_id: Some(event.to_owned()),
        sender: "@member2:example.invalid".to_owned(),
        timestamp_ms: koushi_core::mention_feed::now_ms(),
        thread_root: thread_root.map(str::to_owned),
        kind: MentionKind::Mention,
    }
}

fn feed() -> MentionFeed {
    MentionFeed::open(None, 0, |_| true).expect("open")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mentions_arrive_one_per_event_and_resume_from_the_cursor() {
    let feed = feed();
    for (event, root) in [("$a", None), ("$b", Some("$root")), ("$c", None)] {
        feed.record(candidate(event, root)).await;
    }
    let mut harness = Harness::start(Some(Harness::account_grant(true, false, false)), feed).await;

    let first = harness
        .call(
            "wait_for_mentions",
            json!({ "account_id": ACCOUNT, "timeout_ms": 0, "limit": 2 }),
        )
        .await;
    let first = &first["result"]["structuredContent"];
    let mentions = first["mentions"].as_array().expect("mentions");
    assert_eq!(mentions.len(), 2, "{first}");
    assert_eq!(mentions[0]["event_id"], "$a");
    assert_eq!(mentions[1]["event_id"], "$b");
    assert_eq!(mentions[1]["thread_root_event_id"], "$root");
    assert_eq!(first["coverage"], "exact_mentions");

    let rest = harness
        .call(
            "wait_for_mentions",
            json!({ "account_id": ACCOUNT, "timeout_ms": 0, "cursor": first["cursor"] }),
        )
        .await;
    let rest = &rest["result"]["structuredContent"];
    assert_eq!(rest["mentions"].as_array().map(Vec::len), Some(1), "{rest}");
    assert_eq!(rest["mentions"][0]["event_id"], "$c");

    let quiet = harness
        .call(
            "wait_for_mentions",
            json!({ "account_id": ACCOUNT, "timeout_ms": 0, "cursor": rest["cursor"] }),
        )
        .await;
    let quiet = &quiet["result"]["structuredContent"];
    assert!(quiet["mentions"].as_array().expect("mentions").is_empty());
    assert_eq!(quiet["cursor"], rest["cursor"]);
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cursor_of_another_log_needs_a_resync() {
    let other = feed();
    other.record(candidate("$a", None)).await;
    let foreign = other.cursor_for(1);
    let mine = feed();
    mine.record(candidate("$a", None)).await;
    let mut harness = Harness::start(
        Some(Harness::account_grant(true, false, false)),
        mine.clone(),
    )
    .await;
    let reply = harness
        .call(
            "wait_for_mentions",
            json!({ "account_id": ACCOUNT, "timeout_ms": 0, "cursor": foreign }),
        )
        .await;
    assert_eq!(error_code(&reply), "resync_required", "{reply}");
    assert_eq!(error_reason(&reply), "foreign_cursor");
    let future = harness
        .call(
            "wait_for_mentions",
            json!({ "account_id": ACCOUNT, "timeout_ms": 0, "cursor": mine.cursor_for(9) }),
        )
        .await;
    assert_eq!(error_reason(&future), "future_cursor");
    let malformed = harness
        .call(
            "wait_for_mentions",
            json!({ "account_id": ACCOUNT, "timeout_ms": 0, "cursor": "nonsense" }),
        )
        .await;
    assert_eq!(error_reason(&malformed), "malformed_cursor");
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_read_grant_the_feed_is_refused() {
    let mut harness = Harness::start(None, feed()).await;
    let reply = harness
        .call(
            "wait_for_mentions",
            json!({ "account_id": ACCOUNT, "timeout_ms": 0 }),
        )
        .await;
    assert_eq!(error_code(&reply), "consent_required", "{reply}");
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cursor_that_is_not_a_string_is_rejected_not_treated_as_absent() {
    let feed = feed();
    feed.record(candidate("$a", None)).await;
    let mut harness = Harness::start(Some(Harness::account_grant(true, false, false)), feed).await;
    for bad in [json!(7), json!({ "seq": 1 }), json!(["x"])] {
        let reply = harness
            .call(
                "wait_for_mentions",
                json!({ "account_id": ACCOUNT, "timeout_ms": 0, "cursor": bad }),
            )
            .await;
        assert_eq!(error_reason(&reply), "invalid_params", "{reply}");
        assert!(reply["result"]["structuredContent"]["mentions"].is_null());
    }
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_feed_that_could_not_save_answers_account_unavailable() {
    let feed = MentionFeed::open(
        Some(koushi_core::mention_feed::MentionLogData::fresh(
            "0123456789abcdef".to_owned(),
            0,
        )),
        0,
        |_| false,
    )
    .expect("open");
    feed.record(candidate("$a", None)).await;
    let mut harness = Harness::start(Some(Harness::account_grant(true, false, false)), feed).await;
    let reply = harness
        .call(
            "wait_for_mentions",
            json!({ "account_id": ACCOUNT, "timeout_ms": 0 }),
        )
        .await;
    assert_eq!(error_code(&reply), "account_unavailable", "{reply}");
    harness.stop().await;
}

/// #2154: a bad `mentions` entry is `invalid_params` before anything is sent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_invalid_mentions_argument_is_invalid_params_and_nothing_is_sent() {
    let mut harness = Harness::start(Some(Harness::account_grant(true, true, true)), feed()).await;
    for bad in [
        json!(["not-a-user"]),
        json!(["@ok:example.invalid", ""]),
        json!("@a:b"),
        json!([7]),
    ] {
        let reply = harness
            .call(
                "send_message",
                json!({ "account_id": ACCOUNT, "room_id": ROOM, "body": "hi", "mentions": bad }),
            )
            .await;
        assert_eq!(error_reason(&reply), "invalid_params", "{reply}");
        assert!(reply["result"]["structuredContent"]["event_id"].is_null());
        let reply = harness
            .call(
                "send_thread_reply",
                json!({
                    "account_id": ACCOUNT, "room_id": ROOM, "thread_root_event_id": "$root",
                    "body": "hi", "client_request_id": "r1", "mentions": bad
                }),
            )
            .await;
        assert_eq!(error_reason(&reply), "invalid_params", "{reply}");
    }
    harness.stop().await;
}
