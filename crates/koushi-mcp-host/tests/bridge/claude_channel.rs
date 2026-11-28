//! The Claude Code channel adapter against a real host (#2126, stage 4): a
//! wake row from the owner becomes exactly one `notifications/claude/channel`,
//! the cursor is acknowledged after the write, and a second launcher with the
//! same profile is told `profile_busy` while keeping its request tools.
#![cfg(unix)]

#[path = "../support/mod.rs"]
#[expect(
    clippy::duplicate_mod,
    reason = "suite aggregation preserves feature-local fixture scopes and registered helper tests"
)]
mod support;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use koushi_core::mention_feed::{MentionCandidate, MentionFeed, MentionKind};
use koushi_core::runtime::CoreRuntime;
use koushi_mcp::channel::{ChannelFeed, ChannelRunner};
use koushi_mcp::server::{AppBackend, serve_with_channel};
use koushi_mcp::wire::{ClientInfo, ProfileHello};
use koushi_mcp_host::consent::{ConsentBroker, ConsentMode};
use koushi_mcp_host::{BridgeAccount, BridgeAccounts, ProfileGrants};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};

const AGENT: &str = "account:@agent:example.invalid";
const DM: &str = "!dm:example.invalid";
const OWNER: &str = "@owner:example.invalid";

struct Accounts {
    core: Arc<CoreRuntime>,
    feed: MentionFeed,
    media: PathBuf,
}

impl BridgeAccounts for Accounts {
    fn accounts(&self) -> Vec<BridgeAccount> {
        vec![BridgeAccount {
            id: AGENT.to_owned(),
            homeserver: None,
            connection: self.core.attach(),
            media_cache_dir: self.media.clone(),
        }]
    }

    fn mention_feed(&self, account_id: &str) -> Option<MentionFeed> {
        (account_id == AGENT).then(|| self.feed.clone())
    }
}

struct World {
    _dir: tempfile::TempDir,
    data: PathBuf,
    accounts: Arc<Accounts>,
    profile: ProfileHello,
    bridge: koushi_mcp_host::BridgeHandle,
}

impl World {
    async fn start() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let data = dir.path().join("app");
        let media = dir.path().join("media");
        std::fs::create_dir_all(&media).expect("media dir");
        let core = Arc::new(CoreRuntime::start_with_data_dir(dir.path().join("core")));
        let user = AGENT.strip_prefix("account:").expect("account id");
        support::sign_in_as(&core, user).await;
        support::mark_agent_account(&core, user).await;
        let accounts = Arc::new(Accounts {
            core,
            feed: MentionFeed::open(None, 0, |_| true).expect("feed"),
            media,
        });
        let broker = ConsentBroker::new(data.clone(), ConsentMode::FailClosed);
        let created = broker
            .create_profile(
                &*accounts,
                "Agent",
                AGENT,
                ProfileGrants {
                    read: true,
                    ..ProfileGrants::default()
                },
            )
            .await
            .expect("profile created");
        let secret = koushi_mcp::profile_secret::read(&created.secret_path).expect("secret");
        let bridge = koushi_mcp_host::start(
            Arc::clone(&accounts) as Arc<dyn BridgeAccounts>,
            data.clone(),
            broker,
        )
        .expect("bridge");
        Self {
            _dir: dir,
            data,
            accounts,
            profile: ProfileHello {
                id: created.profile_id.as_str().to_owned(),
                secret,
            },
            bridge,
        }
    }

    /// A message from `sender` to the agent's DM, as the wake feed records it.
    async fn message(&self, event: &str, sender: &str) {
        self.accounts
            .feed
            .record(MentionCandidate {
                room_id: DM.to_owned(),
                event_id: Some(event.to_owned()),
                sender: sender.to_owned(),
                timestamp_ms: koushi_core::mention_feed::now_ms(),
                thread_root: None,
                kind: MentionKind::Mention,
                message_kind: None,
                mime_type: None,
                size_bytes: None,
            })
            .await;
    }

    /// Starts a launcher the way `main` does with `--profile --claude-channel`.
    fn launch(&self) -> Launcher {
        let endpoint_path = self.data.join("mcp/endpoint.json");
        let client_info = ClientInfo {
            name: "claude-code".to_owned(),
            version: "1.0.0".to_owned(),
        };
        let backend = Arc::new(
            AppBackend::new(endpoint_path.clone(), client_info.clone())
                .with_profile(self.profile.clone()),
        );
        let channel = Box::new(ChannelRunner {
            endpoint_path,
            client_info,
            profile: self.profile.clone(),
            backend: backend.clone(),
        }) as Box<dyn ChannelFeed>;
        let (stdin, server_in) = tokio::io::duplex(64 * 1024);
        let (server_out, stdout) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            serve_with_channel(server_in, server_out, backend, Some(channel))
                .await
                .expect("serve");
        });
        Launcher {
            stdin: Some(stdin),
            stdout: BufReader::new(stdout),
            task,
            next_id: 0,
        }
    }
}

struct Launcher {
    stdin: Option<DuplexStream>,
    stdout: BufReader<DuplexStream>,
    task: tokio::task::JoinHandle<()>,
    next_id: u64,
}

impl Launcher {
    async fn send(&mut self, value: Value) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        stdin
            .write_all(format!("{value}\n").as_bytes())
            .await
            .expect("write");
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await;
        loop {
            let line = self.line(Duration::from_secs(10)).await.expect("a reply");
            if line["id"] == id {
                return line;
            }
        }
    }

    async fn initialize(&mut self) -> Value {
        let reply = self.request("initialize", json!({})).await;
        self.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .await;
        reply
    }

    async fn line(&mut self, wait: Duration) -> Option<Value> {
        let mut line = String::new();
        match tokio::time::timeout(wait, self.stdout.read_line(&mut line)).await {
            Ok(Ok(read)) if read > 0 => Some(serde_json::from_str(line.trim_end()).expect("json")),
            _ => None,
        }
    }

    /// The next channel notification.
    async fn notification(&mut self, wait: Duration) -> Option<Value> {
        let line = self.line(wait).await?;
        assert_eq!(line["method"], "notifications/claude/channel", "{line}");
        Some(line)
    }

    /// Closes stdin, as a session ending does, and waits for the loop to stop.
    async fn close(mut self) {
        drop(self.stdin.take());
        self.task.await.expect("serve ended");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_message_from_the_owner_is_exactly_one_notification_and_the_cursor_moves() {
    let world = World::start().await;
    let mut launcher = world.launch();
    let init = launcher.initialize().await;
    assert_eq!(
        init["result"]["capabilities"]["experimental"]["claude/channel"],
        json!({})
    );
    assert!(
        init["result"]["instructions"]
            .as_str()
            .expect("instructions")
            .contains("client_request_id")
    );

    world.message("$e1", OWNER).await;
    let event = launcher
        .notification(Duration::from_secs(15))
        .await
        .expect("a channel notification");
    let meta = &event["params"]["meta"];
    assert_eq!(
        meta,
        &json!({"room_id": DM, "event_id": "$e1", "sender": OWNER, "kind": "mention"})
    );
    // The fixture room is not readable, so the text falls back to a pointer
    // that carries no message body and no filename.
    assert!(
        event["params"]["content"]
            .as_str()
            .expect("content")
            .contains("get_message")
    );
    assert!(
        launcher
            .notification(Duration::from_millis(700))
            .await
            .is_none(),
        "exactly one notification"
    );
    // Give the one-second ack coalescing a chance, then end the session.
    launcher.close().await;

    // A new session with the same profile resumes after the acknowledged
    // cursor, so the delivered row is not delivered again.
    let mut again = None;
    for _ in 0..100 {
        let mut next = world.launch();
        next.initialize().await;
        match next.line(Duration::from_millis(700)).await {
            Some(line) if line["params"]["meta"]["kind"] == "profile_busy" => {
                next.close().await;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Some(line) => panic!("the delivered row came again: {line}"),
            None => {
                again = Some(next);
                break;
            }
        }
    }
    again.expect("the lease was released").close().await;
    world.bridge.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_launcher_with_the_same_profile_gets_profile_busy_and_keeps_its_tools() {
    let world = World::start().await;
    let mut first = world.launch();
    first.initialize().await;
    // The first launcher holds the lease once a row reaches it.
    world.message("$e1", OWNER).await;
    first
        .notification(Duration::from_secs(15))
        .await
        .expect("the holder is subscribed");

    let mut second = world.launch();
    second.initialize().await;
    let busy = second
        .notification(Duration::from_secs(15))
        .await
        .expect("a visible profile_busy note");
    assert_eq!(busy["params"]["meta"], json!({"kind": "profile_busy"}));
    assert!(
        busy["params"]["content"]
            .as_str()
            .expect("content")
            .contains("request tools keep working")
    );

    let tools = second.request("tools/list", json!({})).await;
    assert!(
        tools["result"]["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .any(|tool| tool["name"] == "send_message")
    );
    let call = second
        .request(
            "tools/call",
            json!({"name": "list_accounts", "arguments": {}}),
        )
        .await;
    assert_ne!(call["result"]["isError"], true, "{call}");

    // The holder is undisturbed.
    world.message("$e2", OWNER).await;
    let next = first
        .notification(Duration::from_secs(15))
        .await
        .expect("the holder still receives");
    assert_eq!(next["params"]["meta"]["event_id"], "$e2");
    first.close().await;
    second.close().await;
    world.bridge.shutdown().await;
}
