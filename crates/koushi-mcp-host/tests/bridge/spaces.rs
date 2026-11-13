//! `list_spaces` / `list_space_rooms` (#1883) at the transport boundary: an
//! assistant holds two rooms named `all` in different Spaces and resolves the
//! right one through a read-only Space listing, without the app changing the
//! account, Space or room the GUI shows. A sub-Space expands only as deep as
//! `max_depth` asks, and an id that is not a Space this account sees is an
//! error rather than an empty listing.
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
const SPACE_A: &str = "!space-a:example.invalid";
const SPACE_B: &str = "!space-b:example.invalid";
const SUB_SPACE: &str = "!space-sub:example.invalid";
const ALL_A: &str = "!all-a:example.invalid";
const ALL_B: &str = "!all-b:example.invalid";
const DEEP_ROOM: &str = "!deep-room:example.invalid";
const UNKNOWN_SPACE: &str = "!no-such-space:example.invalid";

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
    /// A fail-closed bridge holding a read grant. Two Spaces each contain a
    /// room displayed as `all`, and Space A also contains a sub-Space.
    async fn start() -> Self {
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
                        "send": false,
                        "attachments": false,
                        "message_actions": false,
                        "draft": false,
                    }]
                }]
            })
            .to_string(),
        )
        .expect("grants");
        let core = Arc::new(CoreRuntime::start_with_data_dir(dir.path().join("core")));
        support::sign_in(&core).await;
        support::apply_actions(
            &core,
            vec![koushi_state::AppAction::RoomListUpdated {
                spaces: vec![
                    support::space(SPACE_A, "Space A", &[ALL_A, SUB_SPACE]),
                    support::space(SPACE_B, "Space B", &[ALL_B]),
                    support::space(SUB_SPACE, "Space A / sub", &[DEEP_ROOM]),
                ],
                rooms: vec![
                    support::named_room(ALL_A, "all", &[SPACE_A]),
                    support::named_room(ALL_B, "all", &[SPACE_B]),
                    support::named_room(DEEP_ROOM, "deep", &[SUB_SPACE]),
                ],
            }],
        )
        .await;
        support::wait_for_room(&core, ALL_A).await;
        let media = dir.path().join("media");
        std::fs::create_dir_all(&media).expect("media dir");
        let accounts: Arc<dyn BridgeAccounts> = Arc::new(OneAccount {
            core: Arc::clone(&core),
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

    async fn stop(self) {
        let _ = &self.dir;
        self.bridge.shutdown().await;
    }
}

fn structured(reply: &Value) -> &Value {
    &reply["result"]["structuredContent"]
}

fn error_code(reply: &Value) -> String {
    structured(reply)["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn children<'a>(reply: &'a Value, key: &str) -> &'a Vec<Value> {
    structured(reply)[key]
        .as_array()
        .unwrap_or_else(|| panic!("{key} in {reply}"))
}

fn names(entries: &[Value]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| {
            entry["name"]
                .as_str()
                .unwrap_or_else(|| panic!("name in {entry}"))
                .to_owned()
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_rooms_named_all_resolve_through_their_space() {
    let mut harness = Harness::start().await;

    let spaces_reply = harness
        .call("list_spaces", json!({ "account_id": ACCOUNT }))
        .await;
    assert_eq!(
        structured(&spaces_reply)["truncated"],
        false,
        "{spaces_reply}"
    );
    let spaces = children(&spaces_reply, "spaces");
    let space_a = spaces
        .iter()
        .find(|space| space["space_id"] == SPACE_A)
        .unwrap_or_else(|| panic!("Space A in {spaces_reply}"));
    assert_eq!(space_a["name"], "Space A");
    assert_eq!(space_a["raw_name"], "Space A");
    assert_eq!(space_a["child_count"], 2);
    assert_eq!(space_a["joined"], true);
    assert_eq!(space_a["is_subspace"], false);
    assert_eq!(space_a["parent_space_ids"], json!([]));
    assert!(
        space_a["alias"].is_null(),
        "Koushi keeps no Space alias, so none is invented: {space_a}"
    );
    let sub_space = spaces
        .iter()
        .find(|space| space["space_id"] == SUB_SPACE)
        .unwrap_or_else(|| panic!("sub-Space in {spaces_reply}"));
    assert_eq!(sub_space["is_subspace"], true);
    assert_eq!(sub_space["parent_space_ids"], json!([SPACE_A]));

    let a_children = harness
        .call(
            "list_space_rooms",
            json!({ "account_id": ACCOUNT, "space_id": SPACE_A }),
        )
        .await;
    assert_eq!(structured(&a_children)["truncated"], false, "{a_children}");
    let a_entries = children(&a_children, "children");
    let all_rooms: Vec<&Value> = a_entries
        .iter()
        .filter(|entry| entry["name"] == "all")
        .collect();
    assert_eq!(
        all_rooms.len(),
        1,
        "one room named all per Space: {a_children}"
    );
    assert_eq!(all_rooms[0]["room_id"], ALL_A);
    assert_eq!(all_rooms[0]["is_space"], false);
    assert_eq!(all_rooms[0]["joined"], true);
    assert_eq!(all_rooms[0]["parent_space_ids"], json!([SPACE_A]));
    assert_eq!(all_rooms[0]["depth"], 1);
    assert!(
        all_rooms[0]["suggested"].is_null(),
        "the Space's child flags were never loaded, so suggested is unknown: {a_children}"
    );
    let sub_entry = a_entries
        .iter()
        .find(|entry| entry["room_id"] == SUB_SPACE)
        .unwrap_or_else(|| panic!("sub-Space in {a_children}"));
    assert_eq!(sub_entry["is_space"], true);
    assert_eq!(sub_entry["name"], "Space A / sub");

    let b_children = harness
        .call(
            "list_space_rooms",
            json!({ "account_id": ACCOUNT, "space_id": SPACE_B }),
        )
        .await;
    let b_entries = children(&b_children, "children");
    assert_eq!(names(b_entries), vec!["all".to_owned()]);
    assert_eq!(b_entries[0]["room_id"], ALL_B);

    let rooms_reply = harness
        .call(
            "list_rooms",
            json!({ "account_id": ACCOUNT, "space_id": SPACE_B }),
        )
        .await;
    let rooms = children(&rooms_reply, "rooms");
    assert_eq!(names(rooms), vec!["all".to_owned()]);
    assert_eq!(rooms[0]["room_id"], ALL_B);
    assert_eq!(rooms[0]["parent_space_ids"], json!([SPACE_B]));

    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn max_depth_expands_sub_spaces_and_an_unknown_space_is_an_error() {
    let mut harness = Harness::start().await;

    let shallow = harness
        .call(
            "list_space_rooms",
            json!({ "account_id": ACCOUNT, "space_id": SPACE_A }),
        )
        .await;
    assert!(
        !names(children(&shallow, "children")).contains(&"deep".to_owned()),
        "depth 1 stops at the sub-Space: {shallow}"
    );

    let deep = harness
        .call(
            "list_space_rooms",
            json!({ "account_id": ACCOUNT, "space_id": SPACE_A, "max_depth": 2 }),
        )
        .await;
    let entries = children(&deep, "children");
    let deep_room = entries
        .iter()
        .find(|entry| entry["room_id"] == DEEP_ROOM)
        .unwrap_or_else(|| panic!("deep room in {deep}"));
    assert_eq!(deep_room["depth"], 2);
    assert_eq!(deep_room["name"], "deep");
    assert_eq!(deep_room["parent_space_ids"], json!([SUB_SPACE]));

    let clamped = harness
        .call(
            "list_space_rooms",
            json!({ "account_id": ACCOUNT, "space_id": SPACE_A, "max_depth": 9 }),
        )
        .await;
    assert_eq!(
        children(&clamped, "children").len(),
        entries.len(),
        "max_depth clamps to the deepest level that exists: {clamped}"
    );

    let unknown = harness
        .call(
            "list_space_rooms",
            json!({ "account_id": ACCOUNT, "space_id": UNKNOWN_SPACE }),
        )
        .await;
    assert_eq!(error_code(&unknown), "room_not_found", "{unknown}");

    harness.stop().await;
}
