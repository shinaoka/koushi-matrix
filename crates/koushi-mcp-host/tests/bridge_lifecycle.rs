//! Connection lifecycle of the MCP bridge with in-app approval (#1536):
//! stopping the bridge closes live connections, and a client that goes away
//! while its call waits for approval leaves nothing running.
#![cfg(unix)]

mod support;

use std::sync::Arc;
use std::time::Duration;

use koushi_core::runtime::CoreRuntime;
use koushi_mcp_host::consent::{ConsentBroker, ConsentMode};
use koushi_mcp_host::{BridgeAccount, BridgeAccounts};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

const ACCOUNT: &str = "account:@member1:example.invalid";

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

struct Client {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
}

impl Client {
    async fn connect(data_dir: &std::path::Path, name: &str) -> Self {
        let endpoint =
            koushi_mcp::endpoint::read(&data_dir.join("mcp/endpoint.json")).expect("endpoint file");
        let stream = UnixStream::connect(&endpoint.endpoint)
            .await
            .expect("connect");
        let (read, writer) = stream.into_split();
        let mut client = Self {
            reader: BufReader::new(read),
            writer,
        };
        client
            .send(serde_json::json!({
                "type": "hello", "protocol": 1, "token": endpoint.token,
                "client": { "name": name, "version": "1.0.0" }
            }))
            .await;
        let reply = client.line().await.expect("hello reply");
        assert!(reply.contains("hello_ok"), "{reply}");
        client
    }

    async fn send(&mut self, value: serde_json::Value) {
        self.writer
            .write_all(format!("{value}\n").as_bytes())
            .await
            .expect("write");
    }

    async fn call(&mut self, id: u64, tool: &str, arguments: serde_json::Value) {
        self.send(serde_json::json!({
            "type": "request", "id": id, "method": "tools/call",
            "params": { "name": tool, "arguments": arguments }
        }))
        .await;
    }

    async fn line(&mut self) -> Option<String> {
        let mut line = String::new();
        let read = tokio::time::timeout(Duration::from_secs(5), self.reader.read_line(&mut line))
            .await
            .expect("reply in time")
            .ok()?;
        (read > 0).then_some(line)
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    data_dir: std::path::PathBuf,
    broker: Arc<ConsentBroker>,
}

async fn fixture() -> (Fixture, Arc<dyn BridgeAccounts>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let data_dir = dir.path().join("app");
    let core = Arc::new(CoreRuntime::start_with_data_dir(dir.path().join("core")));
    // The calls under test must reach the approval lifecycle: only a ready
    // account is admitted (#1580).
    support::sign_in(&core).await;
    let media = dir.path().join("media");
    std::fs::create_dir_all(&media).expect("media dir");
    let accounts: Arc<dyn BridgeAccounts> = Arc::new(OneAccount { core, media });
    let broker = ConsentBroker::new(data_dir.clone(), ConsentMode::Interactive);
    (
        Fixture {
            _dir: dir,
            data_dir,
            broker,
        },
        accounts,
    )
}

async fn until(
    broker: &ConsentBroker,
    predicate: impl Fn(&koushi_mcp_host::consent::McpAssistantsState) -> bool,
) {
    let mut state = broker.subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if predicate(&state.borrow_and_update()) {
                return;
            }
            state.changed().await.expect("broker alive");
        }
    })
    .await
    .expect("state reached in time");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_client_that_disconnects_while_waiting_never_sends() {
    let (fixture, accounts) = fixture().await;
    let bridge = koushi_mcp_host::start(
        accounts,
        fixture.data_dir.clone(),
        Arc::clone(&fixture.broker),
    )
    .expect("bridge");

    let mut client = Client::connect(&fixture.data_dir, "pi").await;
    client
        .call(
            1,
            "send_message",
            serde_json::json!({
                "account_id": ACCOUNT, "room_id": "!room:example.invalid",
                "body": "Synthetic reply"
            }),
        )
        .await;
    until(&fixture.broker, |state| {
        state.pending.first().is_some_and(|pending| pending.waiting)
    })
    .await;

    drop(client);
    // The call was dropped with its connection: nobody waits, and the
    // message it would have sent is gone from the approval.
    until(&fixture.broker, |state| {
        state
            .pending
            .first()
            .is_some_and(|pending| !pending.waiting && pending.message_preview.is_none())
    })
    .await;

    bridge.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopping_the_bridge_closes_connected_clients() {
    let (fixture, accounts) = fixture().await;
    let bridge = koushi_mcp_host::start(
        accounts,
        fixture.data_dir.clone(),
        Arc::clone(&fixture.broker),
    )
    .expect("bridge");

    let mut client = Client::connect(&fixture.data_dir, "pi").await;
    client.call(1, "list_accounts", serde_json::json!({})).await;
    let reply = client.line().await.expect("reply");
    assert!(reply.contains(ACCOUNT), "{reply}");

    // A second client waits for approval when the bridge stops.
    let mut waiting = Client::connect(&fixture.data_dir, "codex").await;
    waiting
        .call(
            1,
            "list_rooms",
            serde_json::json!({ "account_id": ACCOUNT }),
        )
        .await;
    until(&fixture.broker, |state| !state.pending.is_empty()).await;

    bridge.shutdown().await;
    assert!(!fixture.data_dir.join("mcp/endpoint.json").exists());
    assert!(fixture.broker.subscribe().borrow().pending.is_empty());

    // Both connections are closed: the waiting call ends without a reply,
    // and the other client can no longer get one either.
    assert_eq!(waiting.line().await, None);
    let request = serde_json::json!({
        "type": "request", "id": 2, "method": "tools/call",
        "params": { "name": "list_accounts", "arguments": {} }
    });
    let wrote = client
        .writer
        .write_all(format!("{request}\n").as_bytes())
        .await;
    assert!(wrote.is_err() || client.line().await.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_account_is_refused_without_an_approval() {
    let (fixture, accounts) = fixture().await;
    let bridge = koushi_mcp_host::start(
        accounts,
        fixture.data_dir.clone(),
        Arc::clone(&fixture.broker),
    )
    .expect("bridge");

    let mut client = Client::connect(&fixture.data_dir, "pi").await;
    client
        .call(
            1,
            "list_rooms",
            serde_json::json!({ "account_id": "account:@nobody:example.invalid" }),
        )
        .await;
    let reply = client.line().await.expect("reply");
    assert!(reply.contains("account_unavailable"), "{reply}");
    assert!(fixture.broker.subscribe().borrow().pending.is_empty());

    bridge.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restarted_bridge_keeps_its_files_when_the_old_handle_drops() {
    let (fixture, accounts) = fixture().await;
    let first = koushi_mcp_host::start(
        Arc::clone(&accounts),
        fixture.data_dir.clone(),
        Arc::clone(&fixture.broker),
    )
    .expect("bridge");
    first.shutdown().await;
    let second = koushi_mcp_host::start(
        accounts,
        fixture.data_dir.clone(),
        Arc::clone(&fixture.broker),
    )
    .expect("restarted bridge");
    assert!(fixture.data_dir.join("mcp/endpoint.json").exists());
    let _client = Client::connect(&fixture.data_dir, "pi").await;

    second.shutdown().await;
}
