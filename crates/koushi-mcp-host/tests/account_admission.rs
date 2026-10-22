//! Account admission for account-scoped tools (#1580): an account that is not
//! ready (signed out, locked, or still verifying) fails the request with
//! `account_unavailable` before any approval exists, and an already-granted
//! permission never turns it into an empty success (no rooms, no events).
//!
//! The runtime behind these accounts is signed out until the test drives the
//! real reducer; the fake exposes it regardless, which is what the runtime
//! manager does for a signed-out tab it still keeps (the account key survives
//! until the user removes the tab).
#![cfg(unix)]

mod support;

use std::sync::Arc;
use std::time::Duration;

use koushi_core::runtime::CoreRuntime;
use koushi_mcp_host::consent::{ConsentBroker, ConsentMode};
use koushi_mcp_host::{BridgeAccount, BridgeAccounts, Need};
use koushi_state::{AppAction, SessionState};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

const ACCOUNT: &str = "account:@member1:example.invalid";
const ROOM: &str = "!room:example.invalid";
const EVENT: &str = "$event:example.invalid";

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

/// The session a test starts its account in.
#[derive(Clone, Copy)]
enum Session {
    SignedOut,
    Ready,
    /// A lock screen or an invalidated session: the account is signed in.
    Locked,
    /// The sign-in flow's re-gate after a lock; still not a usable account.
    Verifying,
}

struct Harness {
    dir: tempfile::TempDir,
    core: Arc<CoreRuntime>,
    bridge: koushi_mcp_host::BridgeHandle,
    broker: Arc<ConsentBroker>,
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
    next_id: u64,
}

impl Harness {
    async fn start(grant: Option<Value>, mode: ConsentMode, session: Session) -> Self {
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
        let media = dir.path().join("media");
        std::fs::create_dir_all(&media).expect("media dir");
        let accounts: Arc<dyn BridgeAccounts> = Arc::new(OneAccount {
            core: Arc::clone(&core),
            media,
        });
        let broker = ConsentBroker::new(data_dir.clone(), mode);
        let bridge = koushi_mcp_host::start(accounts, data_dir.clone(), Arc::clone(&broker))
            .expect("bridge");

        let endpoint =
            koushi_mcp::endpoint::read(&data_dir.join("mcp/endpoint.json")).expect("endpoint");
        let stream = UnixStream::connect(&endpoint.endpoint)
            .await
            .expect("connect");
        let (read, writer) = stream.into_split();
        let mut harness = Self {
            dir,
            core,
            bridge,
            broker,
            reader: BufReader::new(read),
            writer,
            next_id: 0,
        };
        harness
            .send(json!({
                "type": "hello", "protocol": 1, "token": endpoint.token,
                "client": { "name": "pi", "version": "1.0.0" }
            }))
            .await;
        let hello = harness.line().await;
        assert!(hello.contains("hello_ok"), "{hello}");
        harness.enter(session).await;
        harness
    }

    /// Drives the runtime into `session` through the real reducer.
    async fn enter(&self, session: Session) {
        match session {
            Session::SignedOut => {}
            Session::Ready => support::sign_in(&self.core).await,
            Session::Locked => {
                support::sign_in(&self.core).await;
                support::apply_actions(
                    &self.core,
                    vec![AppAction::SessionAuthenticationInvalidated { soft_logout: true }],
                )
                .await;
                support::wait_for_session(&self.core, |session| {
                    matches!(session, SessionState::Locked(_))
                })
                .await;
            }
            Session::Verifying => {
                support::sign_in(&self.core).await;
                support::apply_actions(&self.core, vec![AppAction::SessionLocked]).await;
                support::wait_for_session(&self.core, |session| {
                    matches!(session, SessionState::Provisional { .. })
                })
                .await;
            }
        }
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

    /// Starts one `tools/call` and returns the response id.
    async fn request(&mut self, tool: &str, arguments: Value) -> u64 {
        let id = self.next_id();
        self.send(json!({
            "type": "request", "id": id, "method": "tools/call",
            "params": { "name": tool, "arguments": arguments }
        }))
        .await;
        id
    }

    async fn call(&mut self, tool: &str, arguments: Value) -> String {
        self.request(tool, arguments).await;
        self.line().await
    }

    fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn pending(&self) -> usize {
        self.broker.subscribe().borrow().pending.len()
    }

    /// Waits for an approval of `need`, so an admission regression fails this
    /// test instead of hanging it.
    async fn wait_for_pending(&self, need: Need) {
        let waited = tokio::time::timeout(Duration::from_secs(5), async {
            while self.pending_for(need) == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        assert!(waited.is_ok(), "no {need:?} approval was raised");
    }

    fn pending_for(&self, need: Need) -> usize {
        self.broker
            .subscribe()
            .borrow()
            .pending
            .iter()
            .filter(|pending| pending.need == need)
            .count()
    }

    fn file(&self, name: &str) -> String {
        let path = self.dir.path().join(name);
        std::fs::write(&path, b"synthetic notes").expect("write fixture");
        path.to_string_lossy().into_owned()
    }

    async fn stop(self) {
        self.bridge.shutdown().await;
    }
}

fn granted(need: &str) -> Value {
    let mut grant = json!({
        "account_id": ACCOUNT, "read": false, "send": false, "attachments": false
    });
    grant[need] = json!(true);
    grant
}

/// Every account-scoped tool, with the arguments it needs to get as far as the
/// account lookup.
fn account_scoped_calls(attachment: String) -> Vec<(&'static str, Value)> {
    vec![
        ("list_rooms", json!({ "account_id": ACCOUNT })),
        (
            "search_messages",
            json!({ "account_id": ACCOUNT, "query": "hello" }),
        ),
        (
            "read_room_messages",
            json!({ "account_id": ACCOUNT, "room_id": ROOM }),
        ),
        ("read_recent_activity", json!({ "account_id": ACCOUNT })),
        (
            "wait_for_events",
            json!({ "account_id": ACCOUNT, "timeout_s": 1 }),
        ),
        (
            "send_message",
            json!({ "account_id": ACCOUNT, "room_id": ROOM, "body": "Synthetic reply" }),
        ),
        (
            "send_message",
            json!({
                "account_id": ACCOUNT, "room_id": ROOM,
                "attachments": [{ "path": attachment }]
            }),
        ),
        (
            "get_attachment",
            json!({ "account_id": ACCOUNT, "room_id": ROOM, "event_id": EVENT }),
        ),
    ]
}

async fn assert_refused_without_an_approval(session: Session) {
    let mut harness = Harness::start(None, ConsentMode::Interactive, session).await;
    let attachment = harness.file("fixture_notes.txt");
    for (tool, arguments) in account_scoped_calls(attachment) {
        let reply = harness.call(tool, arguments).await;
        assert!(reply.contains("account_unavailable"), "{tool}: {reply}");
        assert!(!reply.contains("consent_required"), "{tool}: {reply}");
        assert_eq!(harness.pending(), 0, "{tool} opened an approval: {reply}");
    }
    harness.stop().await;
}

async fn assert_no_empty_success(session: Session) {
    let mut harness = Harness::start(
        Some(json!({
            "account_id": ACCOUNT, "read": true, "send": true, "attachments": true
        })),
        ConsentMode::FailClosed,
        session,
    )
    .await;
    let attachment = harness.file("fixture_notes.txt");
    for (tool, arguments) in account_scoped_calls(attachment) {
        let reply = harness.call(tool, arguments).await;
        assert!(reply.contains("account_unavailable"), "{tool}: {reply}");
    }
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_signed_out_account_is_refused_without_an_approval() {
    assert_refused_without_an_approval(Session::SignedOut).await;
    assert_no_empty_success(Session::SignedOut).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_locked_account_is_refused_without_an_approval() {
    assert_refused_without_an_approval(Session::Locked).await;
    assert_no_empty_success(Session::Locked).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_account_still_verifying_is_refused_without_an_approval() {
    assert_refused_without_an_approval(Session::Verifying).await;
    assert_no_empty_success(Session::Verifying).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_ready_account_is_served() {
    let mut harness = Harness::start(
        Some(granted("read")),
        ConsentMode::FailClosed,
        Session::Ready,
    )
    .await;
    let reply = harness
        .call("list_rooms", json!({ "account_id": ACCOUNT }))
        .await;
    assert!(reply.contains("\"isError\":false"), "{reply}");
    assert!(reply.contains("\"rooms\":[]"), "{reply}");
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_account_locked_while_its_approval_waited_is_refused() {
    let mut harness = Harness::start(None, ConsentMode::Interactive, Session::Ready).await;
    let id = harness
        .request("list_rooms", json!({ "account_id": ACCOUNT }))
        .await;
    harness.wait_for_pending(Need::Read).await;
    support::apply_actions(
        &harness.core,
        vec![AppAction::SessionAuthenticationInvalidated { soft_logout: false }],
    )
    .await;
    support::wait_for_session(&harness.core, |session| {
        matches!(session, SessionState::Locked(_))
    })
    .await;
    let request_id = harness.broker.subscribe().borrow().pending[0].request_id;
    harness.broker.answer(request_id, true).await;
    let reply = harness.line().await;
    assert!(reply.contains(&format!("\"id\":{id}")), "{reply}");
    assert!(reply.contains("account_unavailable"), "{reply}");
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_send_never_asks_for_attachments_after_the_account_was_locked() {
    let mut harness = Harness::start(None, ConsentMode::Interactive, Session::Ready).await;
    let path = harness.file("fixture_notes.txt");
    let id = harness
        .request(
            "send_message",
            json!({
                "account_id": ACCOUNT, "room_id": ROOM, "body": "Synthetic reply",
                "attachments": [{ "path": path }]
            }),
        )
        .await;
    harness.wait_for_pending(Need::Send).await;
    support::apply_actions(
        &harness.core,
        vec![AppAction::SessionAuthenticationInvalidated { soft_logout: false }],
    )
    .await;
    support::wait_for_session(&harness.core, |session| {
        matches!(session, SessionState::Locked(_))
    })
    .await;
    let request_id = harness.broker.subscribe().borrow().pending[0].request_id;
    harness.broker.answer(request_id, true).await;
    let reply = harness.line().await;
    assert!(reply.contains(&format!("\"id\":{id}")), "{reply}");
    assert!(reply.contains("account_unavailable"), "{reply}");
    assert_eq!(
        harness.pending_for(Need::Attachments),
        0,
        "the locked account was asked for the attachments permission"
    );
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_accounts_still_reports_the_sign_in_status() {
    let mut harness = Harness::start(None, ConsentMode::FailClosed, Session::SignedOut).await;
    let reply = harness.call("list_accounts", json!({})).await;
    assert!(reply.contains(ACCOUNT), "{reply}");
    assert!(reply.contains("signed_out"), "{reply}");
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_ready_account_is_listed_as_ready() {
    let mut harness = Harness::start(None, ConsentMode::FailClosed, Session::Ready).await;
    let reply = harness.call("list_accounts", json!({})).await;
    assert!(reply.contains(ACCOUNT), "{reply}");
    assert!(reply.contains("\"status\":\"ready\""), "{reply}");
    harness.stop().await;
}
