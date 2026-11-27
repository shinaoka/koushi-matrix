//! The push subscription, its lease and cursor acknowledgement (#2132, PR 2),
//! at the socket: per-account isolation, one subscriber per profile, resume
//! after the acknowledged cursor, backpressure instead of loss, and the typed
//! ends.
#![cfg(unix)]

#[path = "../support/mod.rs"]
#[expect(
    clippy::duplicate_mod,
    reason = "suite aggregation preserves feature-local fixture scopes and registered helper tests"
)]
mod support;

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use koushi_core::mention_feed::{MentionCandidate, MentionFeed, MentionKind};
use koushi_core::runtime::CoreRuntime;
use koushi_mcp_host::consent::{ConsentBroker, ConsentMode, ProfileId};
use koushi_mcp_host::{BridgeAccount, BridgeAccounts, ProfileGrants, TestLimits};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

const AGENT_A: &str = "account:@agent-a:example.invalid";
const AGENT_B: &str = "account:@agent-b:example.invalid";

struct Accounts {
    cores: Vec<(String, Arc<CoreRuntime>)>,
    feeds: std::sync::Mutex<HashMap<String, MentionFeed>>,
    media: PathBuf,
}

impl BridgeAccounts for Accounts {
    fn accounts(&self) -> Vec<BridgeAccount> {
        self.cores
            .iter()
            .map(|(id, core)| BridgeAccount {
                id: id.clone(),
                homeserver: None,
                connection: core.attach(),
                media_cache_dir: self.media.clone(),
            })
            .collect()
    }

    fn mention_feed(&self, account_id: &str) -> Option<MentionFeed> {
        self.feeds.lock().expect("feeds").get(account_id).cloned()
    }
}

struct World {
    _dir: tempfile::TempDir,
    data: PathBuf,
    accounts: Arc<Accounts>,
    broker: Arc<ConsentBroker>,
    /// While `true`, the feed of AGENT_A commits; clearing it makes it unhealthy.
    storage_ok: Arc<AtomicBool>,
    bridge: koushi_mcp_host::BridgeHandle,
}

fn limits(push_window: usize, ack_stall_ms: u64) -> TestLimits {
    TestLimits {
        push_window,
        ack_stall: Duration::from_millis(ack_stall_ms),
        write_stall: Duration::from_secs(5),
        recheck: Duration::from_millis(100),
    }
}

impl World {
    async fn start() -> Self {
        Self::start_with(limits(64, 60_000)).await
    }

    async fn start_with(limits: TestLimits) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let data = dir.path().join("app");
        let media = dir.path().join("media");
        std::fs::create_dir_all(&media).expect("media dir");
        let storage_ok = Arc::new(AtomicBool::new(true));
        let mut cores = Vec::new();
        let mut feeds = HashMap::new();
        for (index, id) in [AGENT_A, AGENT_B].into_iter().enumerate() {
            let core = Arc::new(CoreRuntime::start_with_data_dir(
                dir.path().join(format!("core-{index}")),
            ));
            let user = id.strip_prefix("account:").expect("account id");
            support::sign_in_as(&core, user).await;
            support::mark_agent_account(&core, user).await;
            let ok = Arc::clone(&storage_ok);
            let feed = if id == AGENT_A {
                MentionFeed::open(None, 0, move |_| ok.load(Ordering::SeqCst)).expect("feed")
            } else {
                MentionFeed::open(None, 0, |_| true).expect("feed")
            };
            feeds.insert(id.to_owned(), feed);
            cores.push((id.to_owned(), core));
        }
        let accounts = Arc::new(Accounts {
            cores,
            feeds: std::sync::Mutex::new(feeds),
            media,
        });
        let broker = ConsentBroker::new(data.clone(), ConsentMode::FailClosed);
        let bridge = koushi_mcp_host::start_with_limits(
            Arc::clone(&accounts) as Arc<dyn BridgeAccounts>,
            data.clone(),
            Arc::clone(&broker),
            limits,
        )
        .expect("bridge");
        Self {
            _dir: dir,
            data,
            accounts,
            broker,
            storage_ok,
            bridge,
        }
    }

    fn feed(&self, account: &str) -> MentionFeed {
        self.accounts.feeds.lock().expect("feeds")[account].clone()
    }

    /// Replaces the account's feed with a new log, as a rebuilt runtime would.
    fn recreate_feed(&self, account: &str) {
        let feed = MentionFeed::open(None, 0, |_| true).expect("feed");
        self.accounts
            .feeds
            .lock()
            .expect("feeds")
            .insert(account.to_owned(), feed);
    }

    fn core(&self, account: &str) -> &Arc<CoreRuntime> {
        &self
            .accounts
            .cores
            .iter()
            .find(|(id, _)| id == account)
            .expect("account")
            .1
    }

    async fn profile(&self, account: &str) -> (ProfileId, String) {
        let created = self
            .broker
            .create_profile(
                &*self.accounts,
                "Agent",
                account,
                ProfileGrants {
                    read: true,
                    ..ProfileGrants::default()
                },
            )
            .await
            .expect("profile created");
        let secret = std::fs::read_to_string(&created.secret_path).expect("secret file");
        (created.profile_id, secret.trim().to_owned())
    }

    async fn record(&self, account: &str, event: &str) {
        let outcome = self
            .feed(account)
            .record(MentionCandidate {
                room_id: "!room:example.invalid".to_owned(),
                event_id: Some(event.to_owned()),
                sender: "@someone:example.invalid".to_owned(),
                timestamp_ms: koushi_core::mention_feed::now_ms(),
                thread_root: None,
                kind: MentionKind::Mention,
            })
            .await;
        assert_eq!(outcome, koushi_core::mention_feed::RecordOutcome::Recorded);
    }

    fn state_file(&self, id: &ProfileId) -> Value {
        let path = self
            .data
            .join(format!("mcp/profile-state/{}.json", id.as_str()));
        serde_json::from_slice(&std::fs::read(path).expect("state file")).expect("json")
    }

    async fn shutdown(self) {
        self.bridge.shutdown().await;
    }
}

struct Conn {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
    next_id: u64,
    /// Frames that arrived while a response was awaited.
    buffered: VecDeque<Value>,
}

impl Conn {
    async fn connect(world: &World, protocol: u64, profile: Option<(&ProfileId, &str)>) -> Self {
        let endpoint = koushi_mcp::endpoint::read(&world.data.join("mcp/endpoint.json"))
            .expect("endpoint file");
        let stream = UnixStream::connect(&endpoint.endpoint)
            .await
            .expect("connect");
        let (read, writer) = stream.into_split();
        let mut conn = Self {
            reader: BufReader::new(read),
            writer,
            next_id: 0,
            buffered: VecDeque::new(),
        };
        let mut hello = json!({
            "type": "hello", "protocol": protocol, "token": endpoint.token,
            "client": {"name": "codex", "version": "1.0.0"},
        });
        if let Some((id, secret)) = profile {
            hello["profile"] = json!({"id": id.as_str(), "secret": secret});
        }
        conn.send(&hello).await;
        let reply = conn.read(Duration::from_secs(10)).await.expect("a reply");
        assert_eq!(reply["type"], "hello_ok", "{reply}");
        conn
    }

    async fn profiled(world: &World, id: &ProfileId, secret: &str) -> Self {
        Self::connect(world, 2, Some((id, secret))).await
    }

    async fn send(&mut self, value: &Value) {
        self.writer
            .write_all(format!("{value}\n").as_bytes())
            .await
            .expect("write");
    }

    async fn read(&mut self, wait: Duration) -> Option<Value> {
        let mut line = String::new();
        match tokio::time::timeout(wait, self.reader.read_line(&mut line)).await {
            Ok(Ok(read)) if read > 0 => Some(serde_json::from_str(line.trim_end()).expect("json")),
            _ => None,
        }
    }

    /// One request; pushes that arrive before its response are kept for
    /// `next_frame`.
    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({"type":"request","id":id,"method":method,"params":params}))
            .await;
        loop {
            let frame = self
                .read(Duration::from_secs(10))
                .await
                .expect("a response");
            if frame["type"] == "response" && frame["id"] == id {
                return frame;
            }
            self.buffered.push_back(frame);
        }
    }

    async fn next_frame(&mut self, wait: Duration) -> Option<Value> {
        match self.buffered.pop_front() {
            Some(frame) => Some(frame),
            None => self.read(wait).await,
        }
    }

    async fn subscribe(&mut self, after: Option<&str>) -> Value {
        let params = after.map_or_else(|| json!({}), |after| json!({"after": after}));
        self.request("subscribe", params).await
    }

    /// Reads pushes until `count` rows arrived; returns their event ids and the
    /// last cursor.
    async fn rows(&mut self, count: usize) -> (Vec<String>, String) {
        let mut events = Vec::new();
        let mut cursor = String::new();
        while events.len() < count {
            let frame = self
                .next_frame(Duration::from_secs(10))
                .await
                .expect("a push");
            assert_eq!(frame["type"], "push", "{frame}");
            for row in frame["rows"].as_array().expect("rows") {
                events.push(row["event_id"].as_str().expect("event").to_owned());
            }
            cursor = frame["cursor"].as_str().expect("cursor").to_owned();
        }
        (events, cursor)
    }

    async fn nothing_for(&mut self, wait: Duration) {
        if let Some(frame) = self.next_frame(wait).await {
            panic!("unexpected frame: {frame}");
        }
    }

    /// The `SubEnd` error's `(code, reason)`.
    async fn sub_end(&mut self) -> (String, String) {
        let frame = self
            .next_frame(Duration::from_secs(10))
            .await
            .expect("a SubEnd");
        assert_eq!(frame["type"], "sub_end", "{frame}");
        (
            frame["error"]["code"].as_str().expect("code").to_owned(),
            frame["error"]["reason"].as_str().unwrap_or("").to_owned(),
        )
    }
}

fn error_of(reply: &Value) -> (String, String) {
    assert_eq!(reply["ok"], false, "{reply}");
    (
        reply["error"]["code"].as_str().expect("code").to_owned(),
        reply["error"]["reason"].as_str().unwrap_or("").to_owned(),
    )
}

fn sub_of(reply: &Value) -> u64 {
    assert_eq!(reply["ok"], true, "{reply}");
    reply["result"]["sub"].as_u64().expect("sub")
}

/// Subscribes, retrying while the previous holder's connection is still
/// winding down.
async fn subscribe_when_free(conn: &mut Conn, after: Option<&str>) -> Value {
    for _ in 0..100 {
        let reply = conn.subscribe(after).await;
        if reply["ok"] == true || reply["error"]["code"] != "profile_busy" {
            return reply;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the lease was never released");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_profiles_on_different_accounts_never_see_each_others_rows() {
    let world = World::start().await;
    let (id_a, secret_a) = world.profile(AGENT_A).await;
    let (id_b, secret_b) = world.profile(AGENT_B).await;
    let mut a = Conn::profiled(&world, &id_a, &secret_a).await;
    let mut b = Conn::profiled(&world, &id_b, &secret_b).await;
    sub_of(&a.subscribe(None).await);
    sub_of(&b.subscribe(None).await);

    world.record(AGENT_A, "$a1").await;
    world.record(AGENT_B, "$b1").await;
    world.record(AGENT_A, "$a2").await;
    world.record(AGENT_B, "$b2").await;

    assert_eq!(a.rows(2).await.0, ["$a1", "$a2"]);
    assert_eq!(b.rows(2).await.0, ["$b1", "$b2"]);
    a.nothing_for(Duration::from_millis(300)).await;
    b.nothing_for(Duration::from_millis(300)).await;
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_subscriber_is_busy_and_the_lease_follows_the_connection() {
    let world = World::start().await;
    let (id, secret) = world.profile(AGENT_A).await;
    let mut first = Conn::profiled(&world, &id, &secret).await;
    let mut second = Conn::profiled(&world, &id, &secret).await;
    let sub = sub_of(&first.subscribe(None).await);

    // A typed error, and the loser keeps its request tools.
    assert_eq!(error_of(&second.subscribe(None).await).0, "profile_busy");
    let listed = second
        .request("tools/call", json!({"name":"list_accounts","arguments":{}}))
        .await;
    assert_eq!(listed["result"]["isError"], false, "{listed}");
    // The first keeps streaming.
    world.record(AGENT_A, "$e1").await;
    assert_eq!(first.rows(1).await.0, ["$e1"]);

    // `unsubscribe` frees the lease while the first connection stays.
    let ended = first.request("unsubscribe", json!({"sub": sub})).await;
    assert_eq!(ended["ok"], true, "{ended}");
    sub_of(&second.subscribe(None).await);
    assert_eq!(
        error_of(&first.subscribe(None).await).0,
        "profile_busy",
        "the new holder owns it"
    );

    // Closing the holder's socket without `unsubscribe` releases it.
    drop(second);
    sub_of(&subscribe_when_free(&mut first, None).await);
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dropped_connection_releases_the_lease() {
    let world = World::start().await;
    let (id, secret) = world.profile(AGENT_A).await;
    let mut holder = Conn::profiled(&world, &id, &secret).await;
    sub_of(&holder.subscribe(None).await);
    drop(holder);
    let mut next = Conn::profiled(&world, &id, &secret).await;
    sub_of(&subscribe_when_free(&mut next, None).await);
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resubscribing_resumes_after_the_acknowledged_cursor() {
    let world = World::start().await;
    let (id, secret) = world.profile(AGENT_A).await;
    for event in ["$e1", "$e2", "$e3"] {
        world.record(AGENT_A, event).await;
    }
    let mut conn = Conn::profiled(&world, &id, &secret).await;
    let sub = sub_of(&conn.subscribe(None).await);
    // A new profile starts at its creation baseline: it was created before the
    // rows, so all three are delivered.
    let (events, _) = conn.rows(3).await;
    assert_eq!(events, ["$e1", "$e2", "$e3"]);
    // Acknowledge the second row only, then vanish.
    let second = world.feed(AGENT_A).cursor_for(2);
    let acked = conn
        .request("ack", json!({"sub": sub, "cursor": second}))
        .await;
    assert_eq!(acked["ok"], true, "{acked}");
    drop(conn);

    let mut again = Conn::profiled(&world, &id, &secret).await;
    subscribe_when_free(&mut again, None).await;
    assert_eq!(again.rows(1).await.0, ["$e3"]);
    again.nothing_for(Duration::from_millis(300)).await;
    // The acknowledged cursor is in the profile's own state file.
    assert_eq!(world.state_file(&id)["acked_cursor"], json!(second));
    assert!(
        !std::fs::read_to_string(world.data.join("mcp/profiles.json"))
            .expect("profiles.json")
            .contains("acked_cursor"),
        "profiles.json is not written by an ack"
    );
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ack_is_cumulative_idempotent_and_never_ahead() {
    let world = World::start().await;
    let (id, secret) = world.profile(AGENT_A).await;
    for event in ["$e1", "$e2", "$e3"] {
        world.record(AGENT_A, event).await;
    }
    let mut conn = Conn::profiled(&world, &id, &secret).await;
    let sub = sub_of(&conn.subscribe(None).await);
    conn.rows(3).await;
    let feed = world.feed(AGENT_A);
    let ahead = conn
        .request("ack", json!({"sub": sub, "cursor": feed.cursor_for(4)}))
        .await;
    assert_eq!(error_of(&ahead), ("unsupported".into(), "ack_ahead".into()));
    let third = feed.cursor_for(3);
    assert_eq!(
        conn.request("ack", json!({"sub": sub, "cursor": third}))
            .await["ok"],
        true
    );
    // An older cursor is a no-op, not an error and not a step back.
    assert_eq!(
        conn.request("ack", json!({"sub": sub, "cursor": feed.cursor_for(1)}))
            .await["ok"],
        true
    );
    let unknown = conn
        .request("ack", json!({"sub": sub + 1, "cursor": third}))
        .await;
    assert_eq!(error_of(&unknown).1, "unknown_sub");
    conn.request("unsubscribe", json!({"sub": sub})).await;
    assert_eq!(world.state_file(&id)["acked_cursor"], json!(third));
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_consumer_that_never_acks_is_stopped_by_the_window_and_loses_nothing() {
    let world = World::start_with(limits(2, 400)).await;
    let (id, secret) = world.profile(AGENT_A).await;
    for event in ["$e1", "$e2", "$e3", "$e4", "$e5"] {
        world.record(AGENT_A, event).await;
    }
    let mut conn = Conn::profiled(&world, &id, &secret).await;
    let reply = conn.subscribe(None).await;
    assert_eq!(reply["result"]["window"], 2);
    let sub = sub_of(&reply);
    // Backpressure: two rows, then the pump stops reading the feed.
    assert_eq!(conn.rows(2).await.0, ["$e1", "$e2"]);
    // The stall ends the subscription with a typed reason, lease released.
    let end = conn.sub_end().await;
    assert_eq!(end, ("resync_required".into(), "ack_stalled".into()));
    let ack_after_end = conn
        .request(
            "ack",
            json!({"sub": sub, "cursor": world.feed(AGENT_A).cursor_for(2)}),
        )
        .await;
    assert_eq!(error_of(&ack_after_end).1, "unknown_sub");

    // Nothing was acknowledged, so a resubscribe redelivers from the start.
    sub_of(&conn.subscribe(None).await);
    let (events, cursor) = conn.rows(2).await;
    assert_eq!(events, ["$e1", "$e2"]);
    // Acknowledging frees the window and the stream continues in order.
    let sub = 2;
    conn.request("ack", json!({"sub": sub, "cursor": cursor}))
        .await;
    assert_eq!(conn.rows(2).await.0, ["$e3", "$e4"]);
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_acknowledged_window_keeps_the_stream_going_in_order() {
    let world = World::start_with(limits(3, 60_000)).await;
    let (id, secret) = world.profile(AGENT_A).await;
    let total = 40;
    for index in 1..=total {
        world.record(AGENT_A, &format!("$e{index}")).await;
    }
    let mut conn = Conn::profiled(&world, &id, &secret).await;
    let sub = sub_of(&conn.subscribe(None).await);
    let mut seen = Vec::new();
    while seen.len() < total {
        let frame = conn
            .next_frame(Duration::from_secs(10))
            .await
            .expect("a push");
        assert_eq!(frame["type"], "push");
        let rows = frame["rows"].as_array().expect("rows");
        assert!(rows.len() <= 3, "the window bounds a push");
        for row in rows {
            seen.push(row["event_id"].as_str().expect("event").to_owned());
        }
        let cursor = frame["cursor"].as_str().expect("cursor");
        let acked = conn
            .request("ack", json!({"sub": sub, "cursor": cursor}))
            .await;
        assert_eq!(acked["ok"], true, "{acked}");
    }
    let expected: Vec<String> = (1..=total).map(|index| format!("$e{index}")).collect();
    assert_eq!(seen, expected, "per-account order, no row skipped");
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_position_that_fell_out_of_retention_ends_expired_cursor() {
    let world = World::start_with(limits(1, 60_000)).await;
    let (id, secret) = world.profile(AGENT_A).await;
    let mut conn = Conn::profiled(&world, &id, &secret).await;
    let sub = sub_of(&conn.subscribe(None).await);
    world.record(AGENT_A, "$first").await;
    let (events, cursor) = conn.rows(1).await;
    assert_eq!(events, ["$first"]);
    // The window is full: while the consumer sits, more than the feed retains
    // arrives.
    for index in 0..(koushi_core::mention_feed::MAX_ENTRIES + 50) {
        world.record(AGENT_A, &format!("$bulk{index}")).await;
    }
    conn.request("ack", json!({"sub": sub, "cursor": cursor}))
        .await;
    // Rows were lost to retention: named, not skipped.
    let end = conn.sub_end().await;
    assert_eq!(end, ("resync_required".into(), "expired_cursor".into()));

    // `after:"retained"` is the documented recovery.
    sub_of(&subscribe_when_free(&mut conn, Some("retained")).await);
    let (events, _) = conn.rows(1).await;
    assert_eq!(events.len(), 1);
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_explicit_cursor_is_validated_by_the_feed() {
    let world = World::start().await;
    let (id, secret) = world.profile(AGENT_A).await;
    world.record(AGENT_A, "$e1").await;
    world.record(AGENT_A, "$e2").await;
    let mut conn = Conn::profiled(&world, &id, &secret).await;
    for (after, reason) in [
        ("garbage", "malformed_cursor"),
        (world.feed(AGENT_B).cursor_for(1).as_str(), "foreign_cursor"),
        (world.feed(AGENT_A).cursor_for(99).as_str(), "future_cursor"),
    ] {
        let reply = conn.subscribe(Some(after)).await;
        assert_eq!(
            error_of(&reply),
            ("resync_required".into(), reason.into()),
            "{after}"
        );
        // A refused subscribe holds no lease: the next one works.
    }
    // An `mf1` cursor replays after that row.
    let reply = conn
        .subscribe(Some(&world.feed(AGENT_A).cursor_for(1)))
        .await;
    sub_of(&reply);
    assert_eq!(conn.rows(1).await.0, ["$e2"]);
    // "retained" starts at the oldest retained row.
    let sub = reply["result"]["sub"].as_u64().expect("sub");
    conn.request("unsubscribe", json!({"sub": sub})).await;
    let reply = conn.subscribe(Some("retained")).await;
    assert_eq!(
        reply["result"]["start_cursor"],
        json!(world.feed(AGENT_A).cursor_for(0))
    );
    assert_eq!(conn.rows(2).await.0, ["$e1", "$e2"]);
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_feed_that_cannot_save_ends_the_subscription_account_unavailable() {
    let world = World::start().await;
    let (id, secret) = world.profile(AGENT_A).await;
    let mut conn = Conn::profiled(&world, &id, &secret).await;
    sub_of(&conn.subscribe(None).await);
    world.storage_ok.store(false, Ordering::SeqCst);
    let outcome = world
        .feed(AGENT_A)
        .record(MentionCandidate {
            room_id: "!r:example.invalid".to_owned(),
            event_id: Some("$lost".to_owned()),
            sender: "@s:example.invalid".to_owned(),
            timestamp_ms: koushi_core::mention_feed::now_ms(),
            thread_root: None,
            kind: MentionKind::Mention,
        })
        .await;
    assert_eq!(outcome, koushi_core::mention_feed::RecordOutcome::Failed);
    assert_eq!(conn.sub_end().await.0, "account_unavailable");
    // The lease went with it, and the new subscribe says why it cannot start.
    assert_eq!(
        error_of(&conn.subscribe(None).await).0,
        "account_unavailable"
    );
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_locked_account_ends_the_subscription_account_unavailable() {
    let world = World::start().await;
    let (id, secret) = world.profile(AGENT_A).await;
    let mut conn = Conn::profiled(&world, &id, &secret).await;
    sub_of(&conn.subscribe(None).await);
    let core = world.core(AGENT_A);
    support::apply_actions(
        core,
        vec![koushi_state::AppAction::SessionAuthenticationInvalidated { soft_logout: true }],
    )
    .await;
    support::wait_for_session(core, |session| {
        matches!(session, koushi_state::SessionState::Locked(_))
    })
    .await;
    assert_eq!(conn.sub_end().await.0, "account_unavailable");
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn losing_the_agent_role_ends_the_subscription_account_unavailable() {
    let world = World::start().await;
    let (id, secret) = world.profile(AGENT_A).await;
    let mut conn = Conn::profiled(&world, &id, &secret).await;
    sub_of(&conn.subscribe(None).await);
    let core = world.core(AGENT_A);
    support::apply_actions(
        core,
        vec![koushi_state::AppAction::AppSettingsSynchronized {
            values: koushi_state::AppSettingsValues::default(),
        }],
    )
    .await;
    let end = conn.sub_end().await;
    assert_eq!(
        end,
        ("account_unavailable".into(), "not_agent_account".into())
    );
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_protocol_1_connection_has_no_subscription() {
    let world = World::start().await;
    let mut conn = Conn::connect(&world, 1, None).await;
    for method in ["subscribe", "unsubscribe", "ack"] {
        let reply = conn.request(method, json!({"sub": 1, "cursor": "x"})).await;
        assert_eq!(error_of(&reply).0, "unsupported", "{method}");
    }
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_revoked_profile_ends_the_stream_with_unauthorized_not_eof() {
    let world = World::start().await;
    let (id, secret) = world.profile(AGENT_A).await;
    let mut conn = Conn::profiled(&world, &id, &secret).await;
    sub_of(&conn.subscribe(None).await);
    world.record(AGENT_A, "$e1").await;
    assert_eq!(conn.rows(1).await.0, ["$e1"]);
    world.broker.revoke_profile(&id).await.expect("revoked");
    // Every time, not only when the pump's own re-check wins a race.
    let (code, reason) = conn.sub_end().await;
    assert_eq!(
        (code.as_str(), reason.as_str()),
        ("unauthorized", "profile_revoked")
    );
    assert!(
        conn.next_frame(Duration::from_secs(5)).await.is_none(),
        "then EOF"
    );
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_revoke_during_a_request_answers_it_and_still_ends_the_stream() {
    let world = World::start().await;
    let (id, secret) = world.profile(AGENT_A).await;
    let mut conn = Conn::profiled(&world, &id, &secret).await;
    sub_of(&conn.subscribe(None).await);
    conn.send(
        &json!({"type":"request","id":900,"method":"tools/call","params":{
        "name":"wait_for_mentions","arguments":{"account_id": AGENT_A, "timeout_ms":30000,
        "cursor": world.feed(AGENT_A).cursor_for(0)}}}),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    world.broker.revoke_profile(&id).await.expect("revoked");
    let mut saw_end = false;
    while let Some(frame) = conn.next_frame(Duration::from_secs(5)).await {
        saw_end |= frame["type"] == "sub_end" && frame["error"]["code"] == "unauthorized";
    }
    assert!(saw_end, "the stream's end was told");
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ack_is_applied_while_a_request_is_still_running() {
    // Window of 2 and a stall of 1.5 s: if the ack waited for the request (up
    // to 6 s here), the subscription would die ack_stalled first.
    let world = World::start_with(limits(2, 1_500)).await;
    let (id, secret) = world.profile(AGENT_A).await;
    let mut conn = Conn::profiled(&world, &id, &secret).await;
    sub_of(&conn.subscribe(None).await);
    for event in ["$e1", "$e2", "$e3", "$e4"] {
        world.record(AGENT_A, event).await;
    }
    let (_, first) = conn.rows(2).await;
    conn.send(
        &json!({"type":"request","id":900,"method":"tools/call","params":{
        "name":"wait_for_mentions","arguments":{"account_id": AGENT_A, "timeout_ms":6000,
        "cursor": world.feed(AGENT_A).cursor_for(4)}}}),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    conn.send(&json!({"type":"request","id":901,"method":"ack","params":{"sub":1,"cursor":first}}))
        .await;
    // The ack's answer comes while request 900 is still running.
    let mut answered = false;
    let mut late_rows = 0;
    let started = std::time::Instant::now();
    while started.elapsed() < Duration::from_secs(4) {
        let Some(frame) = conn.next_frame(Duration::from_millis(500)).await else {
            continue;
        };
        match frame["type"].as_str() {
            Some("response") if frame["id"] == 901 => {
                assert_eq!(frame["ok"], true, "{frame}");
                answered = true;
            }
            Some("response") if frame["id"] == 900 => {
                assert!(
                    started.elapsed() > Duration::from_secs(2),
                    "the request was meant to run on: {frame}"
                );
                break;
            }
            Some("push") => {
                late_rows += frame["rows"].as_array().map_or(0, Vec::len);
                // Request 900 is still running (it has 6 s): that is the point.
                if answered && late_rows >= 2 {
                    break;
                }
            }
            Some("sub_end") => panic!("the subscription died: {frame}"),
            _ => {}
        }
    }
    assert!(answered, "the ack was answered during the request");
    assert!(late_rows >= 2, "the freed window let the next rows through");
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_recreated_feed_ends_the_stream_generation_changed() {
    let world = World::start().await;
    let (id, secret) = world.profile(AGENT_A).await;
    let mut conn = Conn::profiled(&world, &id, &secret).await;
    sub_of(&conn.subscribe(None).await);
    world.recreate_feed(AGENT_A);
    let (code, reason) = conn.sub_end().await;
    assert_eq!(
        (code.as_str(), reason.as_str()),
        ("resync_required", "generation_changed")
    );
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_resubscribe_never_erases_the_sub_end() {
    // Window 1, quick stall: the pump ends ack_stalled while the client is
    // already hammering subscribe.
    let world = World::start_with(limits(1, 150)).await;
    let (id, secret) = world.profile(AGENT_A).await;
    let mut conn = Conn::profiled(&world, &id, &secret).await;
    sub_of(&conn.subscribe(None).await);
    world.record(AGENT_A, "$e1").await;
    world.record(AGENT_A, "$e2").await;
    let mut resubscribed = false;
    for _ in 0..400 {
        tokio::time::sleep(Duration::from_millis(5)).await;
        let reply = conn.subscribe(None).await;
        if reply["ok"] == true {
            resubscribed = true;
            break;
        }
        assert_eq!(error_of(&reply).1, "already_subscribed", "{reply}");
    }
    assert!(resubscribed);
    // The new subscription's response came after the old one's end.
    let ended = conn
        .buffered
        .iter()
        .any(|frame| frame["type"] == "sub_end" && frame["error"]["reason"] == "ack_stalled");
    assert!(
        ended,
        "the end was queued before the new subscription: {:?}",
        conn.buffered
    );
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cursor_that_cannot_be_saved_is_reported_and_ends_the_stream() {
    let world = World::start().await;
    let (id, secret) = world.profile(AGENT_A).await;
    let mut conn = Conn::profiled(&world, &id, &secret).await;
    let sub = sub_of(&conn.subscribe(None).await);
    // A file where the state directory should be: every write fails.
    let dir = world.data.join("mcp/profile-state");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::write(&dir, b"not a directory").expect("block the directory");
    world.record(AGENT_A, "$e1").await;
    let (_, cursor) = conn.rows(1).await;
    let reply = conn
        .request("ack", json!({"sub": sub, "cursor": cursor}))
        .await;
    assert_eq!(reply["ok"], true, "{reply}");
    let (code, reason) = conn.sub_end().await;
    assert_eq!(
        (code.as_str(), reason.as_str()),
        ("internal", "ack_unsaved")
    );
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_settings_state_follows_the_lease_and_the_connection() {
    async fn row_where(
        world: &World,
        id: &ProfileId,
        check: impl Fn(&koushi_mcp_host::profile_view::ProfileView) -> bool,
    ) {
        for _ in 0..200 {
            let found = world
                .broker
                .subscribe()
                .borrow()
                .profiles
                .iter()
                .any(|row| row.profile_id == id.as_str() && check(row));
            if found {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the profile row never reached the expected state");
    }
    let world = World::start().await;
    let (id, secret) = world.profile(AGENT_A).await;
    let mut conn = Conn::profiled(&world, &id, &secret).await;
    row_where(&world, &id, |row| row.connected && !row.subscribed).await;

    let sub = sub_of(&conn.subscribe(None).await);
    row_where(&world, &id, |row| row.connected && row.subscribed).await;

    let ended = conn.request("unsubscribe", json!({"sub": sub})).await;
    assert_eq!(ended["ok"], true, "{ended}");
    row_where(&world, &id, |row| row.connected && !row.subscribed).await;

    sub_of(&conn.subscribe(None).await);
    row_where(&world, &id, |row| row.subscribed).await;
    drop(conn);
    row_where(&world, &id, |row| !row.connected && !row.subscribed).await;
    world.shutdown().await;
}
