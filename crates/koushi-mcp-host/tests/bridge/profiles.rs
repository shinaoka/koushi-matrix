//! Agent profiles (#2132, PR 1b): a profile is created by the owner, presented
//! in a protocol 2 hello, and then every grant, approval and request of that
//! connection is keyed on the profile, never on the name it declared.
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

use koushi_core::runtime::CoreRuntime;
use koushi_mcp_host::consent::{ConsentBroker, ConsentMode, GrantSubject, ProfileId};
use koushi_mcp_host::{BridgeAccount, BridgeAccounts, Need, ProfileGrants};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

const OWNER: &str = "account:@owner:example.invalid";
const AGENT_A: &str = "account:@agent-a:example.invalid";
const AGENT_B: &str = "account:@agent-b:example.invalid";
const NOBODY: &str = "account:@nobody:example.invalid";
const ROOM: &str = "!room:example.invalid";

struct Accounts {
    cores: Vec<(String, Arc<CoreRuntime>)>,
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
}

/// Three accounts, as the design's fixture: the owner's own, and agent A and B.
struct World {
    _dir: tempfile::TempDir,
    data: PathBuf,
    accounts: Arc<Accounts>,
    broker: Arc<ConsentBroker>,
    bridge: koushi_mcp_host::BridgeHandle,
}

impl World {
    async fn start(mode: ConsentMode) -> Self {
        Self::start_with(mode, None).await
    }

    async fn start_with(mode: ConsentMode, connections: Option<Value>) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let data = dir.path().join("app");
        if let Some(connections) = connections {
            std::fs::create_dir_all(data.join("mcp")).expect("mcp dir");
            std::fs::write(data.join("mcp/connections.json"), connections.to_string())
                .expect("grants");
        }
        let media = dir.path().join("media");
        std::fs::create_dir_all(&media).expect("media dir");
        let mut cores = Vec::new();
        for (index, (id, agent)) in [(OWNER, false), (AGENT_A, true), (AGENT_B, true)]
            .into_iter()
            .enumerate()
        {
            let core = Arc::new(CoreRuntime::start_with_data_dir(
                dir.path().join(format!("core-{index}")),
            ));
            let user = id.strip_prefix("account:").expect("account id");
            support::sign_in_as(&core, user).await;
            if agent {
                support::mark_agent_account(&core, user).await;
            }
            support::apply_actions(
                &core,
                vec![koushi_state::AppAction::RoomListUpdated {
                    spaces: Vec::new(),
                    rooms: vec![support::room(ROOM)],
                }],
            )
            .await;
            support::wait_for_room(&core, ROOM).await;
            cores.push((id.to_owned(), core));
        }
        let accounts = Arc::new(Accounts { cores, media });
        let broker = ConsentBroker::new(data.clone(), mode);
        let bridge = koushi_mcp_host::start(
            Arc::clone(&accounts) as Arc<dyn BridgeAccounts>,
            data.clone(),
            Arc::clone(&broker),
        )
        .expect("bridge");
        Self {
            _dir: dir,
            data,
            accounts,
            broker,
            bridge,
        }
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

    /// The owner path: creates a profile and returns its id and secret, read
    /// from the file the plugin would read.
    async fn create(
        &self,
        label: &str,
        account: &str,
        grants: ProfileGrants,
    ) -> (ProfileId, String) {
        let created = self
            .broker
            .create_profile(&*self.accounts, label, account, grants)
            .await
            .expect("profile created");
        let secret = std::fs::read_to_string(&created.secret_path).expect("secret file");
        (created.profile_id, secret.trim().to_owned())
    }

    fn profiles_bytes(&self) -> Vec<u8> {
        std::fs::read(self.data.join("mcp/profiles.json")).expect("profiles.json")
    }

    async fn shutdown(self) {
        self.bridge.shutdown().await;
    }
}

fn read_only() -> ProfileGrants {
    ProfileGrants {
        read: true,
        ..ProfileGrants::default()
    }
}

struct Conn {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
    next_id: u64,
}

impl Conn {
    async fn open(world: &World) -> (Self, String) {
        let endpoint = koushi_mcp::endpoint::read(&world.data.join("mcp/endpoint.json"))
            .expect("endpoint file");
        let stream = UnixStream::connect(&endpoint.endpoint)
            .await
            .expect("connect");
        let (read, writer) = stream.into_split();
        (
            Self {
                reader: BufReader::new(read),
                writer,
                next_id: 0,
            },
            endpoint.token,
        )
    }

    async fn send_raw(&mut self, line: &str) {
        self.writer
            .write_all(format!("{line}\n").as_bytes())
            .await
            .expect("write");
    }

    /// The next line verbatim, or `None` when the host closed.
    async fn read_raw(&mut self) -> Option<String> {
        let mut line = String::new();
        let read = tokio::time::timeout(Duration::from_secs(10), self.reader.read_line(&mut line))
            .await
            .expect("bounded read")
            .expect("read");
        (read > 0).then(|| line.trim_end().to_owned())
    }

    /// A profiled hello. `Ok` is an established connection, `Err` the raw
    /// refusal line.
    async fn profiled(
        world: &World,
        client_name: &str,
        id: &str,
        secret: &str,
    ) -> Result<Self, String> {
        let (mut conn, token) = Self::open(world).await;
        conn.send_raw(
            &json!({
                "type": "hello", "protocol": 2, "token": token,
                "client": {"name": client_name, "version": "1.0.0"},
                "profile": {"id": id, "secret": secret},
            })
            .to_string(),
        )
        .await;
        let line = conn.read_raw().await.expect("a reply");
        let reply: Value = serde_json::from_str(&line).expect("json");
        if reply["type"] == "hello_ok" {
            assert_eq!(reply["protocol"], 2);
            Ok(conn)
        } else {
            assert!(conn.read_raw().await.is_none(), "the host closes after it");
            Err(line)
        }
    }

    /// An unprofiled (protocol 1) connection.
    async fn unprofiled(world: &World, client_name: &str) -> Self {
        let (mut conn, token) = Self::open(world).await;
        conn.send_raw(
            &json!({
                "type": "hello", "protocol": 1, "token": token,
                "client": {"name": client_name, "version": "1.0.0"},
            })
            .to_string(),
        )
        .await;
        let line = conn.read_raw().await.expect("a reply");
        assert!(line.contains("hello_ok"), "{line}");
        conn
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        self.send_raw(
            &json!({"type":"request","id":self.next_id,"method":method,"params":params})
                .to_string(),
        )
        .await;
        let line = self.read_raw().await.expect("a response");
        serde_json::from_str(&line).expect("json")
    }

    async fn call(&mut self, tool: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name": tool, "arguments": arguments}))
            .await
    }
}

fn structured(reply: &Value) -> &Value {
    &reply["result"]["structuredContent"]
}

/// `(code, reason)` of a failed tool call.
fn failure(reply: &Value) -> (String, Option<String>) {
    assert_eq!(reply["result"]["isError"], true, "{reply}");
    let error = &structured(reply)["error"];
    (
        error["code"].as_str().expect("code").to_owned(),
        error["reason"].as_str().map(str::to_owned),
    )
}

fn account_ids(reply: &Value) -> Vec<String> {
    structured(reply)["accounts"]
        .as_array()
        .expect("accounts")
        .iter()
        .map(|account| account["account_id"].as_str().expect("id").to_owned())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_profile_is_created_and_used_end_to_end() {
    let world = World::start(ConsentMode::FailClosed).await;
    let (id, secret) = world.create("Reader", AGENT_A, read_only()).await;

    // The secret file is private, and the secret is in no stored byte.
    let path = world
        .data
        .join(format!("mcp/profiles/{}.secret", id.as_str()));
    let mode =
        std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&path).unwrap().permissions());
    assert_eq!(mode & 0o777, 0o600);
    assert!(!String::from_utf8_lossy(&world.profiles_bytes()).contains(&secret));

    // Connect with the secret: the connection is the profile's.
    let mut conn = Conn::profiled(&world, "Claude-Code", id.as_str(), &secret)
        .await
        .expect("the right secret connects");

    // `list_accounts` returns only the bound account, with the profile's grants.
    let listed = conn.call("list_accounts", json!({})).await;
    assert_eq!(account_ids(&listed), [AGENT_A]);
    let account = &structured(&listed)["accounts"][0];
    assert_eq!(account["consent"], "granted");
    assert_eq!(account["permissions"]["read"], true);
    assert_eq!(account["permissions"]["send"], false);

    // The bound account works under the profile's grant ...
    let rooms = conn
        .call("list_rooms", json!({"account_id": AGENT_A}))
        .await;
    assert_eq!(rooms["result"]["isError"], false, "{rooms}");
    assert_eq!(structured(&rooms)["rooms"][0]["room_id"], ROOM);
    // ... a permission it does not hold is refused ...
    let sent = conn
        .call(
            "send_message",
            json!({"account_id": AGENT_A, "room_id": ROOM, "body": "hi", "client_request_id": "r1"}),
        )
        .await;
    assert_eq!(failure(&sent).0, "consent_required");
    // ... and a foreign account is refused before anything else.
    for foreign in [AGENT_B, OWNER] {
        let reply = conn
            .call("list_rooms", json!({"account_id": foreign}))
            .await;
        assert_eq!(
            failure(&reply),
            (
                "not_permitted".to_owned(),
                Some("profile_account".to_owned())
            )
        );
    }

    // Revoke: the connection that was open is told, the row and the file go,
    // and the same secret no longer connects.
    assert!(world.broker.revoke_profile(&id).await.expect("revoked"));
    assert!(!path.exists(), "the secret file is removed");
    let after = tokio::time::timeout(Duration::from_secs(5), conn.read_raw()).await;
    assert!(
        matches!(after, Ok(None)),
        "the revoked connection is closed"
    );
    let refused = Conn::profiled(&world, "Claude-Code", id.as_str(), &secret)
        .await
        .err()
        .expect("the revoked secret is refused");
    assert!(refused.contains("profile_secret"), "{refused}");
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wrong_secret_and_an_unknown_id_are_byte_identical_and_never_lock_out() {
    let world = World::start(ConsentMode::FailClosed).await;
    let (id, secret) = world.create("Reader", AGENT_A, read_only()).await;
    let unknown = "prof_00000000000000000000000000000000";

    let wrong = Conn::profiled(&world, "codex", id.as_str(), "not-the-secret")
        .await
        .err()
        .expect("wrong secret");
    let missing = Conn::profiled(&world, "codex", unknown, "not-the-secret")
        .await
        .err()
        .expect("unknown id");
    let malformed = Conn::profiled(&world, "codex", "../../etc/passwd", "x")
        .await
        .err()
        .expect("malformed id");
    let empty = Conn::profiled(&world, "codex", id.as_str(), "")
        .await
        .err()
        .expect("empty secret");
    assert_eq!(
        wrong, missing,
        "an unknown id must not be told from a wrong secret"
    );
    assert_eq!(wrong, malformed);
    assert_eq!(wrong, empty);
    let error: Value = serde_json::from_str(&wrong).expect("json");
    assert_eq!(error["code"], "unauthorized");
    assert_eq!(error["reason"], "profile_secret");
    assert!(!wrong.contains("not-the-secret"));
    assert!(!wrong.contains(id.as_str()));

    // No lockout: many failures, then the real plugin still connects.
    for _ in 0..40 {
        Conn::profiled(&world, "codex", id.as_str(), "guess")
            .await
            .err()
            .expect("refused");
    }
    Conn::profiled(&world, "codex", id.as_str(), &secret)
        .await
        .expect("the real secret still connects");

    // The rejection time of a real profile is recorded for the settings page;
    // a guess at an unknown id leaves nothing behind.
    // The write happens after the connection closed, off its task: poll.
    let mut recorded = None;
    for _ in 0..100 {
        recorded = world.broker.profile_last_rejected_ms(&id);
        if recorded.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(recorded.is_some(), "last_rejected_ms is recorded");
    let unknown_id = ProfileId::parse(unknown).expect("shape");
    assert_eq!(world.broker.profile_last_rejected_ms(&unknown_id), None);
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rejection_does_not_rewrite_the_profile_store() {
    let world = World::start(ConsentMode::FailClosed).await;
    let (id, _secret) = world.create("Reader", AGENT_A, read_only()).await;
    let before = world.profiles_bytes();
    for _ in 0..5 {
        Conn::profiled(&world, "codex", id.as_str(), "guess")
            .await
            .err()
            .expect("refused");
    }
    assert_eq!(world.profiles_bytes(), before);
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_foreign_account_is_one_error_for_every_tool() {
    let world = World::start(ConsentMode::FailClosed).await;
    let all = ProfileGrants {
        read: true,
        send: true,
        attachments: true,
        message_actions: true,
        draft: true,
    };
    let (id, secret) = world.create("Everything", AGENT_A, all).await;
    let mut conn = Conn::profiled(&world, "codex", id.as_str(), &secret)
        .await
        .expect("connects");

    let tools = koushi_mcp_host::tool_names();
    assert!(
        tools.len() >= 19,
        "the table is generated from the dispatcher"
    );
    for tool in tools.iter().filter(|tool| **tool != "list_accounts") {
        let mut replies = Vec::new();
        for foreign in [AGENT_B, OWNER, NOBODY] {
            let reply = conn
                .call(
                    tool,
                    json!({
                        "account_id": foreign, "room_id": ROOM, "event_id": "$e",
                        "client_request_id": "r", "body": "b", "query": "q",
                        "space_id": "!s", "emoji": "x", "key": "x",
                    }),
                )
                .await;
            assert_eq!(
                failure(&reply),
                (
                    "not_permitted".to_owned(),
                    Some("profile_account".to_owned())
                ),
                "{tool} with {foreign}"
            );
            replies.push(reply["result"].to_string());
        }
        assert!(
            replies.windows(2).all(|pair| pair[0] == pair[1]),
            "{tool}: an existing and a missing account answer alike"
        );

        // `account_id` stays required: omitting it is invalid params, never a
        // resolution to the bound account.
        let omitted = conn.call(tool, json!({"room_id": ROOM})).await;
        let (code, reason) = failure(&omitted);
        assert_ne!(reason.as_deref(), Some("profile_account"), "{tool}");
        assert_eq!(code, "unsupported", "{tool}: {omitted}");
        // `read_recent_activity` validates its whole argument object first.
        if *tool != "read_recent_activity" {
            assert!(
                structured(&omitted)["error"]["message"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("account_id"),
                "{tool}: {omitted}"
            );
        }
    }
    world.shutdown().await;
}

fn connections_with_client_grant() -> Value {
    json!({
        "schema": 3,
        "connections": [{
            "client_name": "Claude-Code",
            "accounts": [{
                "account_id": AGENT_A, "read": true, "send": true,
                "attachments": false, "message_actions": false, "draft": false
            }]
        }]
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_client_name_grant_never_lets_a_profile_through() {
    let world = World::start_with(
        ConsentMode::FailClosed,
        Some(connections_with_client_grant()),
    )
    .await;
    // The grant is real: an unprofiled "Claude-Code" uses it.
    let mut named = Conn::unprofiled(&world, "Claude-Code").await;
    let rooms = named
        .call("list_rooms", json!({"account_id": AGENT_A}))
        .await;
    assert_eq!(rooms["result"]["isError"], false, "{rooms}");

    // A profile with no grants that also calls itself "Claude-Code" gets none.
    let (id, secret) = world
        .create("Empty", AGENT_A, ProfileGrants::default())
        .await;
    let mut profiled = Conn::profiled(&world, "Claude-Code", id.as_str(), &secret)
        .await
        .expect("connects");
    let listed = profiled.call("list_accounts", json!({})).await;
    assert_eq!(account_ids(&listed), [AGENT_A]);
    let account = &structured(&listed)["accounts"][0];
    assert_eq!(account["consent"], "required");
    for flag in ["read", "send", "attachments", "message_actions", "draft"] {
        assert_eq!(account["permissions"][flag], false, "{flag}");
    }
    for (tool, arguments) in [
        ("list_rooms", json!({"account_id": AGENT_A})),
        ("read_recent_activity", json!({"account_id": AGENT_A})),
        ("get_draft", json!({"account_id": AGENT_A, "room_id": ROOM})),
        (
            "send_message",
            json!({"account_id": AGENT_A, "room_id": ROOM, "body": "x", "client_request_id": "k"}),
        ),
    ] {
        let reply = profiled.call(tool, arguments).await;
        assert_eq!(failure(&reply).0, "consent_required", "{tool}");
    }
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_profile_grant_never_lets_a_client_name_through() {
    let world = World::start(ConsentMode::FailClosed).await;
    let (id, secret) = world.create("Reader-One", AGENT_A, read_only()).await;
    let mut profiled = Conn::profiled(&world, "whatever", id.as_str(), &secret)
        .await
        .expect("connects");
    let rooms = profiled
        .call("list_rooms", json!({"account_id": AGENT_A}))
        .await;
    assert_eq!(rooms["result"]["isError"], false, "{rooms}");

    // Connections that merely carry the profile's label, or its raw id, as
    // their name hold none of its grants and see no extra access.
    for name in ["Reader-One", id.as_str()] {
        let mut imposter = Conn::unprofiled(&world, name).await;
        let listed = imposter.call("list_accounts", json!({})).await;
        for account in structured(&listed)["accounts"]
            .as_array()
            .expect("accounts")
        {
            assert_eq!(account["consent"], "required", "{name}");
            assert_eq!(account["permissions"]["read"], false, "{name}");
        }
        let reply = imposter
            .call("list_rooms", json!({"account_id": AGENT_A}))
            .await;
        assert_eq!(failure(&reply).0, "consent_required", "{name}");
    }

    // Granting the profile changes no client's grants.
    let store = std::fs::read_to_string(world.data.join("mcp/connections.json"));
    assert!(
        store.is_err(),
        "no client grant was written for the profile"
    );
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_profiles_on_one_account_stay_distinct() {
    let world = World::start(ConsentMode::FailClosed).await;
    let (first, first_secret) = world.create("First", AGENT_A, read_only()).await;
    let (second, second_secret) = world.create("Second", AGENT_A, read_only()).await;
    world
        .broker
        .set_permission(
            &GrantSubject::Profile(first.clone()),
            AGENT_A,
            Need::Send,
            true,
        )
        .await
        .expect("grant");
    let mut one = Conn::profiled(&world, "same", first.as_str(), &first_secret)
        .await
        .expect("first");
    let mut two = Conn::profiled(&world, "same", second.as_str(), &second_secret)
        .await
        .expect("second");
    let one_view = one.call("list_accounts", json!({})).await;
    let two_view = two.call("list_accounts", json!({})).await;
    assert_eq!(
        structured(&one_view)["accounts"][0]["permissions"]["send"],
        true
    );
    assert_eq!(
        structured(&two_view)["accounts"][0]["permissions"]["send"],
        false
    );
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_draft_staged_by_a_client_is_not_the_profiles() {
    let world = World::start(ConsentMode::FailClosed).await;
    let draft_only = ProfileGrants {
        draft: true,
        ..ProfileGrants::default()
    };
    let (id, secret) = world.create("Drafter", AGENT_A, draft_only).await;
    // A draft an unprofiled "Claude-Code" staged, through the real reducer.
    support::apply_actions(
        world.core(AGENT_A),
        vec![koushi_state::AppAction::AssistantDraftStaged {
            target: koushi_state::ComposerTarget::Main {
                room_id: ROOM.to_owned(),
            },
            document: koushi_state::ComposerDocument::from_plain_text("from the client"),
            client_name: "Claude-Code".to_owned(),
            subject: GrantSubject::client("Claude-Code").key(),
            replace: false,
            reply_to_event_id: None,
        }],
    )
    .await;
    let mut profiled = Conn::profiled(&world, "Claude-Code", id.as_str(), &secret)
        .await
        .expect("connects");
    let read = json!({"account_id": AGENT_A, "room_id": ROOM});
    let mut status = Value::Null;
    let mut body = Value::Null;
    for _ in 0..100 {
        let reply = profiled.call("get_draft", read.clone()).await;
        status = structured(&reply)["status"].clone();
        body = structured(&reply)["body"].clone();
        if status != "empty" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        status, "user_edited",
        "another subject's draft is never reported as this profile's own"
    );
    assert_eq!(
        body, "",
        "another subject's draft text is not returned to this profile"
    );
    let replaced = profiled
        .call(
            "draft_message",
            json!({"account_id": AGENT_A, "room_id": ROOM, "body": "mine", "replace": true}),
        )
        .await;
    assert_eq!(failure(&replaced).0, "conflict", "{replaced}");
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn removing_the_agent_role_turns_a_ready_profile_into_not_agent_account() {
    let world = World::start(ConsentMode::FailClosed).await;
    let (id, secret) = world.create("Reader", AGENT_A, read_only()).await;
    let mut conn = Conn::profiled(&world, "codex", id.as_str(), &secret)
        .await
        .expect("connects");
    let ready = conn
        .call("list_rooms", json!({"account_id": AGENT_A}))
        .await;
    assert_eq!(ready["result"]["isError"], false, "{ready}");

    support::apply_actions(
        world.core(AGENT_A),
        vec![koushi_state::AppAction::AppSettingsSynchronized {
            values: koushi_state::AppSettingsValues::default(),
        }],
    )
    .await;
    let mut reply = Value::Null;
    for _ in 0..100 {
        reply = conn
            .call("list_rooms", json!({"account_id": AGENT_A}))
            .await;
        if reply["result"]["isError"] == true {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        failure(&reply),
        (
            "not_permitted".to_owned(),
            Some("not_agent_account".to_owned())
        )
    );
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_profile_can_be_bound_only_to_an_agent_account() {
    let world = World::start(ConsentMode::FailClosed).await;
    let owner = world
        .broker
        .create_profile(&*world.accounts, "Owner", OWNER, read_only())
        .await;
    assert_eq!(owner, Err(koushi_mcp_host::ProfileError::NotAgentAccount));
    let missing = world
        .broker
        .create_profile(&*world.accounts, "Nobody", NOBODY, read_only())
        .await;
    assert_eq!(
        missing,
        Err(koushi_mcp_host::ProfileError::AccountUnavailable)
    );
    assert!(!world.data.join("mcp/profiles.json").exists());
    let blank = world
        .broker
        .create_profile(&*world.accounts, "  \u{202E} ", AGENT_A, read_only())
        .await;
    assert_eq!(blank, Err(koushi_mcp_host::ProfileError::InvalidLabel));
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_revoke_ends_a_pending_approval_with_no_data() {
    let world = World::start(ConsentMode::Interactive).await;
    let (id, secret) = world
        .create("Waiting", AGENT_A, ProfileGrants::default())
        .await;
    let mut conn = Conn::profiled(&world, "codex", id.as_str(), &secret)
        .await
        .expect("connects");
    // The call needs `read`, which the profile lacks: it waits for the owner.
    conn.next_id += 1;
    let request_id = conn.next_id;
    conn.send_raw(
        &json!({"type":"request","id":request_id,"method":"tools/call",
                "params":{"name":"list_rooms","arguments":{"account_id": AGENT_A}}})
        .to_string(),
    )
    .await;
    for _ in 0..200 {
        if !world.broker.subscribe().borrow().pending.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let pending = world.broker.subscribe().borrow().pending.clone();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].client_name, "Waiting",
        "the dialog names the profile"
    );

    world.broker.revoke_profile(&id).await.expect("revoked");
    let line = conn.read_raw().await.expect("the waiting call is answered");
    let reply: Value = serde_json::from_str(&line).expect("json");
    assert_eq!(reply["ok"], false, "{reply}");
    assert_eq!(reply["error"]["code"], "unauthorized");
    assert_eq!(reply["error"]["reason"], "profile_revoked");
    assert!(reply.get("result").is_none(), "no data returns");
    assert!(world.broker.subscribe().borrow().pending.is_empty());
    world.shutdown().await;
}

/// No IPC method creates, edits or widens a profile: every method the
/// dispatcher knows, and every name that sounds like one, is called over a
/// profiled connection and the store's bytes and modification time stay put.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_ipc_method_changes_the_profile_store() {
    let world = World::start(ConsentMode::FailClosed).await;
    let (id, secret) = world.create("Reader", AGENT_A, read_only()).await;
    let store = world.data.join("mcp/profiles.json");
    let bytes = world.profiles_bytes();
    let modified = std::fs::metadata(&store).unwrap().modified().unwrap();

    let mut conn = Conn::profiled(&world, "codex", id.as_str(), &secret)
        .await
        .expect("connects");
    let hostile = json!({
        "account_id": AGENT_A, "room_id": ROOM, "event_id": "$e", "client_request_id": "r",
        "body": "b", "query": "q", "space_id": "!s", "profile_id": id.as_str(),
        "grants": {"read": true, "send": true, "attachments": true,
                   "message_actions": true, "draft": true},
        "label": "pwned", "secret": "x", "revoke": true,
        "timeout_s": 0, "timeout_ms": 0,
    });
    let mut calls = 0;
    for method in koushi_mcp_host::method_names() {
        if *method == "tools/call" {
            for tool in koushi_mcp_host::tool_names() {
                let reply = conn.call(tool, hostile.clone()).await;
                assert!(
                    !reply.to_string().contains("unknown tool"),
                    "{tool} is in the table but not served"
                );
                calls += 1;
            }
        } else {
            conn.request(method, hostile.clone()).await;
            calls += 1;
        }
    }
    for method in [
        "profile/create",
        "profile/revoke",
        "profile/update",
        "profiles/list",
        "grant",
        "approve",
        "subscribe",
        "unsubscribe",
        "ack",
    ] {
        let reply = conn.request(method, hostile.clone()).await;
        assert_eq!(reply["error"]["code"], "unsupported", "{method}");
        calls += 1;
    }
    assert!(calls > 25, "the table covered every tool and method");
    assert_eq!(world.profiles_bytes(), bytes, "the store's bytes changed");
    assert_eq!(
        std::fs::metadata(&store).unwrap().modified().unwrap(),
        modified,
        "the store was rewritten"
    );
    // The profile still holds exactly what the owner gave it.
    let listed = conn.call("list_accounts", json!({})).await;
    let permissions = &structured(&listed)["accounts"][0]["permissions"];
    assert_eq!(permissions["read"], true);
    assert_eq!(permissions["send"], false);
    world.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_secret_reaches_no_diagnostic_or_debug_output() {
    let _diagnostics = koushi_diagnostics::test_support::lock_async().await;
    let world = World::start(ConsentMode::FailClosed).await;
    let (id, secret) = world.create("Reader", AGENT_A, read_only()).await;
    Conn::profiled(&world, "codex", id.as_str(), "guess-with-a-marker-0xC0FFEE")
        .await
        .err()
        .expect("refused");
    let hello = koushi_mcp::wire::ProfileHello {
        id: id.as_str().to_owned(),
        secret: secret.clone(),
    };
    assert!(!format!("{hello:?}").contains(&secret));
    let state = format!("{:?}", world.broker.subscribe().borrow().clone());
    assert!(!state.contains(&secret));
    let diagnostics = format!("{:?}", koushi_diagnostics::snapshot());
    assert!(!diagnostics.contains(&secret));
    assert!(!diagnostics.contains("0xC0FFEE"));
    world.shutdown().await;
}
