//! Wire protocol negotiation of the MCP bridge (#2132, PR 1a): a protocol 1
//! session is served exactly as before, and protocol 2 is refused with typed
//! reasons until profiles can be verified.
#![cfg(unix)]

use std::path::Path;
use std::sync::Arc;

use koushi_mcp_host::consent::{ConsentBroker, ConsentMode};
use koushi_mcp_host::{BridgeAccount, BridgeAccounts};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// Raw lines of what the host wrote before the protocol 2 change, with
/// `connection_id` and `app_version` masked by string replacement. The file is
/// an oracle: nothing in this test regenerates it, so it can only change by a
/// reviewed edit. `<closed>` means the host closed without a reply.
///
/// MEASURED, not reasoned: every line below was printed by a temporary
/// integration test run against the base build (commit 19e4280a, before PR 1a),
/// using the same helpers as this file. All ten lines were measured; none is
/// derived. Compared against this branch on the raw line, the only differences
/// are the `reason`/`supported` keys appended to the three handshake errors
/// (`client_name`; `protocol_version` + `"supported":[1,2]` for protocol 0 and 3).
///
/// ```text
/// hello_ok      {"type":"hello_ok","protocol":1,"connection_id":"<masked>","app_version":"<masked>"}
/// list_accounts {"type":"response","id":1,"ok":true,"result":{"content":[{"text":"{\"accounts\":[]}","type":"text"}],"isError":false,"structuredContent":{"accounts":[]}}}
/// status        {"type":"response","id":2,"ok":true,"result":{"app_version":"<masked>","protocol":1}}
/// unknown       {"type":"response","id":3,"ok":false,"error":{"code":"unsupported","message":"unsupported bridge method: nope"}}
/// not json      {"type":"response","id":0,"ok":false,"error":{"code":"internal","message":"malformed request"}}
/// frame too big <closed>
/// wrong token   {"type":"error","code":"unauthorized","message":"connection token rejected"}
/// bad name      {"type":"error","code":"unsupported","message":"client name must be 1-64 characters from A-Z a-z 0-9 . _ -"}
/// protocol 0    {"type":"error","code":"unsupported","message":"this koushi-mcp launcher is older than the running Koushi; register it again"}
/// protocol 3    (same line as protocol 0)
/// ```
const GOLDEN: &str = include_str!("golden/v1_session.jsonl");

struct NoAccounts;
impl BridgeAccounts for NoAccounts {
    fn accounts(&self) -> Vec<BridgeAccount> {
        Vec::new()
    }
}

struct Peer {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
}

impl Peer {
    async fn open(data: &Path) -> (Self, String) {
        let endpoint =
            koushi_mcp::endpoint::read(&data.join("mcp/endpoint.json")).expect("endpoint file");
        let stream = UnixStream::connect(&endpoint.endpoint)
            .await
            .expect("connect");
        let (read, writer) = stream.into_split();
        (
            Self {
                reader: BufReader::new(read),
                writer,
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

    /// The next line, verbatim, or `None` when the host closed.
    async fn read_raw(&mut self) -> Option<String> {
        let mut line = String::new();
        let read = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.reader.read_line(&mut line),
        )
        .await
        .expect("bounded read")
        .expect("read");
        (read > 0).then(|| line.trim_end().to_owned())
    }
}

fn hello(protocol: u64, token: &str, name: &str, profile: Option<Value>) -> String {
    let mut value = json!({
        "type": "hello", "protocol": protocol, "token": token,
        "client": {"name": name, "version": "1.0.0"}
    });
    if let Some(profile) = profile {
        value["profile"] = profile;
    }
    value.to_string()
}

/// Replaces the value of a string field by string replacement on the raw
/// line, so key order and whitespace stay exactly what the host wrote.
fn mask_field(line: &str, key: &str) -> String {
    let needle = format!("\"{key}\":\"");
    let Some(start) = line.find(&needle).map(|at| at + needle.len()) else {
        return line.to_owned();
    };
    let end = start + line[start..].find('"').expect("closing quote");
    format!("{}<masked>{}", &line[..start], &line[end..])
}

/// The two fields that legitimately differ between runs: the connection id
/// carries a pid and the app version moves with every release.
fn mask(line: &str) -> String {
    mask_field(&mask_field(line, "connection_id"), "app_version")
}

/// Splits the keys protocol 2 added (`reason`, `supported`) off the end of a
/// raw handshake error, so what is left can be compared with the old build.
fn without_new_keys(line: &str) -> (String, String) {
    let at = line.find(",\"reason\":").expect("a reason key");
    (
        format!("{}}}", &line[..at]),
        line[at..line.len() - 1].to_owned(),
    )
}

async fn start() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    koushi_mcp_host::BridgeHandle,
) {
    let dir = tempfile::tempdir().expect("tempdir");
    let data = dir.path().join("app");
    let broker = ConsentBroker::new(data.clone(), ConsentMode::FailClosed);
    let bridge =
        koushi_mcp_host::start(Arc::new(NoAccounts), data.clone(), broker).expect("bridge");
    (dir, data, bridge)
}

async fn handshake_error(data: &Path, line: impl FnOnce(&str) -> String) -> Value {
    let (mut peer, token) = Peer::open(data).await;
    peer.send_raw(&line(&token)).await;
    let reply = peer.read_raw().await.expect("an error reply");
    assert!(peer.read_raw().await.is_none(), "the host closes after it");
    serde_json::from_str(&reply).expect("json")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_protocol_1_session_matches_the_recorded_transcript() {
    let (_dir, data, _bridge) = start().await;
    let mut transcript = Vec::new();

    let (mut peer, token) = Peer::open(&data).await;
    for request in [
        hello(1, &token, "golden", None),
        json!({"type":"request","id":1,"method":"tools/call",
               "params":{"name":"list_accounts","arguments":{}}})
        .to_string(),
        json!({"type":"request","id":2,"method":"status"}).to_string(),
        json!({"type":"request","id":3,"method":"nope"}).to_string(),
        "not json".to_owned(),
    ] {
        peer.send_raw(&request).await;
        transcript.push(mask(&peer.read_raw().await.expect("reply")));
    }
    // An oversized frame: the host closes without a reply.
    peer.send_raw(&"x".repeat(koushi_mcp::wire::MAX_FRAME_BYTES + 1))
        .await;
    transcript.push(
        peer.read_raw()
            .await
            .unwrap_or_else(|| "<closed>".to_owned()),
    );

    // A wrong token on a fresh connection.
    let (mut other, _) = Peer::open(&data).await;
    other
        .send_raw(&hello(1, "not-the-token", "golden", None))
        .await;
    transcript.push(mask(&other.read_raw().await.expect("reply")));

    // Handshake errors: the old line plus exactly the new keys, nothing else.
    let mut added = Vec::new();
    for line in [
        hello(1, &token, "bad name", None),
        hello(0, &token, "golden", None),
        hello(3, &token, "golden", None),
    ] {
        let (mut peer, _) = Peer::open(&data).await;
        peer.send_raw(&line).await;
        let reply = peer.read_raw().await.expect("an error reply");
        let (old, new_keys) = without_new_keys(&reply);
        transcript.push(old);
        added.push(new_keys);
    }
    assert_eq!(
        added,
        [
            r#","reason":"client_name""#,
            r#","reason":"protocol_version","supported":[1,2]"#,
            r#","reason":"protocol_version","supported":[1,2]"#,
        ]
    );

    let golden: Vec<&str> = GOLDEN.lines().collect();
    assert_eq!(transcript, golden);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_v1_hello_is_answered_1_and_status_reports_1() {
    let (_dir, data, _bridge) = start().await;
    let (mut peer, token) = Peer::open(&data).await;
    peer.send_raw(&hello(1, &token, "codex", None)).await;
    let ok: Value = serde_json::from_str(&peer.read_raw().await.expect("hello_ok")).expect("json");
    assert_eq!(ok["type"], "hello_ok");
    assert_eq!(ok["protocol"], 1);
    peer.send_raw(&json!({"type":"request","id":1,"method":"status"}).to_string())
        .await;
    let status: Value =
        serde_json::from_str(&peer.read_raw().await.expect("status")).expect("json");
    assert_eq!(status["result"]["protocol"], 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_v2_hello_needs_a_profile_and_a_profile_cannot_be_verified_yet() {
    let (_dir, data, _bridge) = start().await;
    let missing = handshake_error(&data, |t| hello(2, t, "codex", None)).await;
    assert_eq!(missing["type"], "error");
    assert_eq!(missing["code"], "unsupported");
    assert_eq!(missing["reason"], "profile_required");

    let profile = json!({"id": "p1", "secret": "s3cret"});
    let presented = handshake_error(&data, |t| hello(2, t, "codex", Some(profile.clone()))).await;
    assert_eq!(presented["code"], "unsupported");
    assert_eq!(presented["reason"], "profile_rejected");
    assert!(!presented.to_string().contains("s3cret"));

    // An unknown id and a wrong secret must be indistinguishable, byte for
    // byte, or the host becomes an oracle for which profile ids exist.
    let other = handshake_error(&data, |t| {
        hello(
            2,
            t,
            "codex",
            Some(json!({"id": "p2", "secret": "different"})),
        )
    })
    .await;
    assert_eq!(presented.to_string(), other.to_string());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_profile_on_a_v1_hello_is_not_a_token_rejection() {
    let (_dir, data, _bridge) = start().await;
    let profile = json!({"id": "p1", "secret": "s"});
    let error = handshake_error(&data, |t| hello(1, t, "codex", Some(profile.clone()))).await;
    assert_eq!(error["code"], "unsupported");
    assert_eq!(error["reason"], "profile_requires_protocol_2");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handshake_errors_carry_a_reason_that_tells_name_from_version() {
    let (_dir, data, _bridge) = start().await;
    let name = handshake_error(&data, |t| hello(1, t, "bad name", None)).await;
    assert_eq!(name["code"], "unsupported");
    assert_eq!(name["reason"], "client_name");
    for protocol in [0, 3] {
        let version = handshake_error(&data, |t| hello(protocol, t, "codex", None)).await;
        assert_eq!(version["code"], "unsupported");
        assert_eq!(version["reason"], "protocol_version");
        assert_eq!(version["supported"], json!([1, 2]));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wrong_token_is_unauthorized_before_any_version_disclosure() {
    let (_dir, data, _bridge) = start().await;
    for line in [
        hello(1, "wrong", "codex", None),
        hello(3, "wrong", "codex", None),
        hello(2, "wrong", "codex", Some(json!({"id":"p","secret":"s"}))),
    ] {
        let error = handshake_error(&data, |_| line.clone()).await;
        assert_eq!(error["code"], "unauthorized");
        assert!(error.get("reason").is_none());
        assert!(error.get("supported").is_none());
    }
}
