//! #1910: a `send_message` whose `reply_to_event_id` points at an event inside
//! a thread must land in that thread — the contract #1808 promised at the MCP
//! host boundary. The handler only maps the reply target onto
//! `TimelineCommand::SendReply`; whether the send lands in the thread is decided
//! inside the account runtime against the room's own cache. This test drives
//! both halves through the bridge, so a change at either side of that seam
//! fails here.
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
use koushi_sdk::MatrixClientSession;
use matrix_sdk::ruma::{event_id, room_id, user_id};
use matrix_sdk::test_utils::mocks::MatrixMockServer;
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};
use serde_json::{Value, json};
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

struct Harness {
    dir: tempfile::TempDir,
    bridge: koushi_mcp_host::BridgeHandle,
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
}

impl Harness {
    /// A bridge over a runtime whose SDK session is `client`, so a granted
    /// send actually reaches `server`'s homeserver and its content is
    /// inspectable.
    async fn start(client: matrix_sdk::Client) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let data_dir = dir.path().join("app");
        std::fs::create_dir_all(data_dir.join("mcp")).expect("mcp dir");
        std::fs::write(
            data_dir.join("mcp/connections.json"),
            json!({
                "schema": 3,
                "connections": [{
                    "client_name": "pi",
                    "accounts": [{
                        "account_id": ACCOUNT,
                        "read": true,
                        "send": true,
                        "attachments": false,
                        "message_actions": false,
                    }]
                }]
            })
            .to_string(),
        )
        .expect("grants");
        let core = Arc::new(CoreRuntime::start_with_data_dir(dir.path().join("core")));
        support::sign_in(&core).await;
        assert!(
            core.install_account_session_for_testing(Arc::new(
                MatrixClientSession::from_client_for_testing(client, support::session_info()),
            ))
            .await,
            "install the mock-homeserver session"
        );
        // A user-content send needs a promoted session, and a ready session
        // also needs Secure Backup admission (#1737). Both are set on the
        // account actor after the install, so the install cannot clear them.
        assert!(
            core.promote_account_session_for_testing().await,
            "promote the installed mock session"
        );
        assert!(
            core.project_secure_backup_gate_for_testing(koushi_state::SecureBackupGateState::Ready)
                .await,
            "project the Secure Backup gate that admits sends"
        );
        let media = dir.path().join("media");
        std::fs::create_dir_all(&media).expect("media dir");
        let accounts: Arc<dyn BridgeAccounts> = Arc::new(OneAccount {
            core: core.clone(),
            media,
        });
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
        tokio::time::timeout(Duration::from_secs(30), self.reader.read_line(&mut line))
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

    async fn stop(self) {
        let _ = &self.dir;
        self.bridge.shutdown().await;
    }
}

/// #1910 / #1808: replying to an event that lives in a thread stays in that
/// thread. The exact relation is pinned, so a main-timeline reply that only
/// quotes the in-thread event fails here.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reply_to_an_in_thread_event_lands_in_that_thread() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room = room_id!("!threaded:example.invalid");
    let root = event_id!("$root:example.invalid");
    let in_thread = event_id!("$in-thread:example.invalid");
    let alice = user_id!("@alice:example.invalid");
    let factory = EventFactory::new().room(room).sender(alice);
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(room)
                .add_timeline_event(
                    factory
                        .text_msg("Synthetic root")
                        .event_id(root)
                        .into_raw_sync(),
                )
                .add_timeline_event(
                    factory
                        .text_msg("Synthetic in-thread message")
                        .event_id(in_thread)
                        .in_thread(root, root)
                        .into_raw_sync(),
                ),
        )
        .await;
    // The target must be resolved from the room's own cache: this is the MCP
    // case, where nothing is displayed and no request is wanted.
    server
        .mock_room_event()
        .match_event_id()
        .ok(factory
            .text_msg("Synthetic in-thread message")
            .event_id(in_thread)
            .in_thread(root, root)
            .into_event())
        .never()
        .mount()
        .await;
    server.mock_room_state_encryption().plain().mount().await;
    server
        .mock_room_send()
        .ok(event_id!("$sent:example.invalid"))
        .expect(1)
        .mount()
        .await;
    let mut harness = Harness::start(client).await;
    let reply = harness
        .call(
            "send_message",
            json!({
                "account_id": ACCOUNT, "room_id": room,
                "body": "Synthetic answer", "reply_to_event_id": in_thread,
                "client_request_id": "uuid-1910",
            }),
        )
        .await;
    assert_eq!(
        reply["result"]["structuredContent"]["event_id"],
        json!("$sent:example.invalid"),
        "{reply}"
    );
    harness.stop().await;

    let sent = server
        .received_requests()
        .await
        .expect("the mock recorded requests")
        .into_iter()
        .find(|request| {
            request.method.as_str() == "PUT" && request.url.path().contains("/send/m.room.message/")
        })
        .expect("the reply was sent");
    let content: Value = serde_json::from_slice(&sent.body).expect("message content");
    // A genuine in-thread reply omits `is_falling_back`, so the exact relation
    // below pins the thread root and the replied-to event at once.
    assert_eq!(
        content["m.relates_to"],
        json!({
            "rel_type": "m.thread",
            "event_id": root,
            "m.in_reply_to": { "event_id": in_thread },
        }),
        "an MCP reply to an in-thread event must be sent into that thread"
    );
    assert_eq!(content["body"], json!("Synthetic answer"));
}
