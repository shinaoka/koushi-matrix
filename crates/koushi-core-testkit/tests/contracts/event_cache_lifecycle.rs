//! Exercise automatic SDK cache startup through the real Core login/restore path.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use koushi_core::runtime::{CoreConnection, CoreRuntime};
use koushi_protocol::command::{AccountCommand, CoreCommand, RoomCommand};
use koushi_protocol::event::{CoreEvent, TimelineDiff, TimelineEvent, TimelineItemId};
use koushi_state::{AppState, AuthSecret, LoginRequest, SessionState, SyncState};
use matrix_sdk::test_utils::mocks::MatrixMockServer;
use serde_json::{Value, json};
use wiremock::{
    Mock, Request, Respond, ResponseTemplate,
    matchers::{method, path},
};

const USER: &str = "@cache-lifecycle:localhost";
const ROOM: &str = "!cache-lifecycle:localhost";
const FIRST_EVENT: &str = "$cache-first:localhost";
const RESTORED_EVENT: &str = "$cache-restored:localhost";
const SYNC_PATH: &str = "/_matrix/client/unstable/org.matrix.simplified_msc3575/sync";

// This reads after the timeout, so the store may have changed since the failure.
// The SDK's store opener checkpoints and migrates; use SQLite read-only instead.
fn persisted_event_after_timeout(
    data: &std::path::Path,
    credentials: &std::path::Path,
) -> Result<bool, &'static str> {
    let backend = koushi_store::CredentialStoreBackend::FileDir(
        koushi_store::FileCredentialStore::new(credentials),
    );
    let session = backend
        .load_last_session()
        .map_err(|_| "session lookup failed")?
        .ok_or("session missing")?;
    let store_id = backend
        .load_local_store_id(&session)
        .map_err(|_| "store id lookup failed")?;
    let secret = backend
        .load(&session)
        .map_err(|_| "store key lookup failed")?;
    let key = secret.derive_sdk_store_key();
    let path = data
        .join("accounts/v2")
        .join(store_id.as_str())
        .join("cache/matrix-sdk-event-cache.sqlite3");
    read_cached_event(&path, key.as_bytes())
}

fn read_cached_event(path: &std::path::Path, key: &[u8]) -> Result<bool, &'static str> {
    let db =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| "read-only open failed")?;
    let version: Vec<u8> = db
        .query_row("SELECT value FROM kv WHERE key = 'version'", [], |row| {
            row.get(0)
        })
        .map_err(|_| "schema version unavailable")?;
    if version != [18] {
        return Err("unsupported cache schema");
    }
    let encrypted_cipher: Vec<u8> = db
        .query_row("SELECT value FROM kv WHERE key = 'cipher'", [], |row| {
            row.get(0)
        })
        .map_err(|_| "cache cipher unavailable")?;
    let cipher = matrix_sdk_store_encryption::StoreCipher::import_with_key(key, &encrypted_cipher)
        .map_err(|_| "cache cipher import failed")?;
    let room = cipher.hash_key("events", ROOM.as_bytes());
    let event = cipher.hash_key("events", FIRST_EVENT.as_bytes());
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM events WHERE event_id = ?1 AND room_id = ?2)",
        rusqlite::params![event.as_slice(), room.as_slice()],
        |row| row.get(0),
    )
    .map_err(|_| "cache lookup failed")
}

#[test]
fn cache_failure_diagnostic_reads_hashed_event_without_writing() {
    let dir = tempfile::tempdir().expect("isolated database");
    let path = dir.path().join("cache.sqlite3");
    let key = [7_u8; 32];
    let cipher = matrix_sdk_store_encryption::StoreCipher::new().expect("cipher");
    let db = rusqlite::Connection::open(&path).expect("database");
    db.execute_batch("CREATE TABLE kv (key TEXT PRIMARY KEY, value BLOB); CREATE TABLE events (room_id BLOB, event_id BLOB);").expect("schema");
    db.execute("INSERT INTO kv VALUES ('version', ?1)", [&[18_u8][..]])
        .expect("version");
    db.execute(
        "INSERT INTO kv VALUES ('cipher', ?1)",
        [cipher.export_with_key(&key).expect("export")],
    )
    .expect("cipher");
    assert_eq!(read_cached_event(&path, &key), Ok(false));
    db.execute(
        "INSERT INTO events VALUES (?1, ?2)",
        rusqlite::params![
            cipher.hash_key("events", ROOM.as_bytes()).as_slice(),
            cipher.hash_key("events", FIRST_EVENT.as_bytes()).as_slice()
        ],
    )
    .expect("event");
    assert_eq!(read_cached_event(&path, &key), Ok(true));
    db.execute(
        "UPDATE kv SET value = ?1 WHERE key = 'version'",
        [&[19_u8][..]],
    )
    .expect("version");
    assert_eq!(
        read_cached_event(&path, &key),
        Err("unsupported cache schema")
    );
    assert_eq!(
        read_cached_event(&dir.path().join("missing"), &key),
        Err("read-only open failed")
    );
}

struct LoginResponse;

impl Respond for LoginResponse {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).expect("login JSON");
        ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "synthetic-cache-token",
            "device_id": body["device_id"],
            "user_id": USER,
        }))
    }
}

struct CacheSyncResponse {
    phase: Arc<AtomicUsize>,
    sequence: AtomicUsize,
}

impl Respond for CacheSyncResponse {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).expect("sync JSON");
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        let mut response =
            json!({"pos": format!("cache-pos-{sequence}"), "txn_id": body["txn_id"]});
        if body["conn_id"] == "room-list" {
            let phase = self.phase.load(Ordering::Acquire);
            let events = if phase == 1 {
                // Restore must find the first message on disk, not in this response.
                vec![]
            } else {
                vec![json!({
                    "type": "m.room.message",
                    "event_id": if phase == 0 { FIRST_EVENT } else { RESTORED_EVENT },
                    "sender": USER,
                    "origin_server_ts": 1_700_000_000_000_u64 + phase as u64,
                    "content": {"msgtype": "m.text", "body": "synthetic cache message"},
                })]
            };
            response["lists"] = json!({"all_rooms": {"count": 1, "ops": [{
                "op": "SYNC", "range": [0, 0], "room_ids": [ROOM],
            }]}});
            response["rooms"] = json!({ROOM: {
                "initial": true,
                "required_state": [
                    {"type": "m.room.create", "state_key": "", "sender": USER,
                     "event_id": "$cache-create:localhost", "origin_server_ts": 1,
                     "content": {"creator": USER, "room_version": "10"}},
                    {"type": "m.room.member", "state_key": USER, "sender": USER,
                     "event_id": "$cache-member:localhost", "origin_server_ts": 2,
                     "content": {"membership": "join"}},
                    {"type": "m.room.name", "state_key": "", "sender": USER,
                     "event_id": "$cache-name:localhost", "origin_server_ts": 3,
                     "content": {"name": "Synthetic cache room"}},
                ],
                "timeline": events,
            }});
        }
        ResponseTemplate::new(200).set_body_json(response)
    }
}

async fn verified_runtime(data: &std::path::Path, credentials: &std::path::Path) -> CoreRuntime {
    let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
        data.to_owned(),
        credentials.to_owned(),
    );
    assert!(
        runtime
            .configure_trust_observation_for_testing(koushi_sdk::CurrentDeviceTrustObservation {
                current: koushi_state::CurrentDeviceTrustState::Verified,
                updates: Box::pin(futures_util::stream::pending()),
            },)
            .await
    );
    runtime
}

async fn wait_for_state(
    connection: &mut CoreConnection,
    deadline: tokio::time::Instant,
    predicate: impl Fn(&AppState) -> bool,
) -> AppState {
    tokio::time::timeout_at(deadline, async {
        loop {
            if predicate(&connection.snapshot()) {
                break;
            }
            if connection.next_versioned_snapshot().await.is_none() {
                break;
            }
        }
    })
    .await
    .expect("cache lifecycle state deadline");
    let final_state = connection.snapshot();
    assert!(
        predicate(&final_state),
        "cache lifecycle state did not converge"
    );
    final_state
}

fn has_latest(state: &AppState, event_id: &str) -> bool {
    matches!(state.session, SessionState::Ready(_))
        && state.sync == SyncState::Running
        && state.rooms.iter().any(|room| {
            room.room_id == ROOM
                && room
                    .latest_event
                    .as_ref()
                    .is_some_and(|event| event.event_id == event_id)
        })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_cache_is_rejected_before_diagnostics_without_killing_the_account_actor() {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    tokio::time::timeout_at(deadline, async {
        let server = MatrixMockServer::new().await;
        let session = |client: matrix_sdk::Client| {
            let info = koushi_state::SessionInfo {
                homeserver: server.uri(),
                user_id: client.user_id().expect("authenticated fixture").to_string(),
                device_id: client.device_id().expect("fixture device").to_string(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            };
            Arc::new(koushi_sdk::MatrixClientSession::from_client_for_testing(
                client, info,
            ))
        };
        let (runtime, _stores) = CoreRuntime::start_isolated();
        assert!(
            runtime
                .install_account_session_for_testing(session(server.client_builder().build().await))
                .await
        );
        let owners = runtime.inspect_sync_owners_for_testing().await;
        let unavailable = server
            .client_builder()
            .on_builder(|builder| builder.with_event_cache(false))
            .build()
            .await;
        assert!(
            !runtime
                .install_account_session_for_testing(session(unavailable))
                .await
        );
        assert_eq!(runtime.inspect_sync_owners_for_testing().await, owners);
        assert!(
            runtime
                .install_account_session_for_testing(session(server.client_builder().build().await))
                .await,
            "a rejected cache must not panic or disable later account admission"
        );
        runtime.shutdown().await;
    })
    .await
    .expect("disabled-cache admission deadline");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn automatic_cache_delivers_first_sync_persisted_restore_and_new_updates() {
    let started = tokio::time::Instant::now();
    let deadline = started + Duration::from_secs(30);
    let mut phases = vec![("start", started)];
    let server = MatrixMockServer::new().await;
    server
        .mock_versions()
        .with_feature("org.matrix.simplified_msc3575", true)
        .ok()
        .mount()
        .await;
    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/login"))
        .respond_with(LoginResponse)
        .expect(1)
        .mount(server.server())
        .await;
    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/logout"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(server.server())
        .await;
    let phase = Arc::new(AtomicUsize::new(0));
    Mock::given(method("POST"))
        .and(path(SYNC_PATH))
        .respond_with(CacheSyncResponse {
            phase: phase.clone(),
            sequence: AtomicUsize::new(0),
        })
        .mount(server.server())
        .await;
    let data = tempfile::tempdir().expect("isolated runtime data");
    let credentials = tempfile::tempdir().expect("isolated runtime credentials");
    let runtime = verified_runtime(data.path(), credentials.path()).await;
    phases.push(("login_start", tokio::time::Instant::now()));
    let mut connection = runtime.attach();
    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id,
            request: LoginRequest {
                homeserver: server.uri(),
                username: "cache-lifecycle".to_owned(),
                password: AuthSecret::new("synthetic-password"),
                device_display_name: None,
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await
        .expect("submit login");
    phases.push(("first_sync_wait_start", tokio::time::Instant::now()));
    let initial = wait_for_state(&mut connection, deadline, |state| {
        has_latest(state, FIRST_EVENT)
    })
    .await;
    phases.push(("first_sync_done", tokio::time::Instant::now()));
    drop(connection);
    tokio::time::timeout_at(deadline, runtime.shutdown())
        .await
        .expect("first runtime settles");
    phases.push(("ordered_shutdown_done", tokio::time::Instant::now()));

    phase.store(1, Ordering::Release);
    let restored = verified_runtime(data.path(), credentials.path()).await;
    let mut connection = restored.attach();
    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Account(AccountCommand::RestoreLastSession {
            request_id,
        }))
        .await
        .expect("submit restore");
    let restored_state = wait_for_state(&mut connection, deadline, |state| {
        has_latest(state, FIRST_EVENT)
    })
    .await;
    phases.push(("restore_done", tokio::time::Instant::now()));
    assert_eq!(
        restored_state.session, initial.session,
        "restore keeps the same device"
    );
    // RoomInfo also persists previews; require the actual cached timeline, not only its summary.
    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Room(RoomCommand::SelectRoom {
            request_id,
            room_id: ROOM.to_owned(),
        }))
        .await
        .expect("select restored room");
    phases.push(("cached_delivery_wait_start", tokio::time::Instant::now()));
    let mut initial_counts = Vec::new();
    let mut delivered_in_updates = false;
    let result = tokio::time::timeout_at(deadline, async {
        loop {
            match connection.recv_event().await.expect("restored Core event") {
                CoreEvent::Timeline(TimelineEvent::InitialItems { key, items, .. }) if key.room_id() == ROOM => {
                    let found = items.iter().any(|item| matches!(&item.id, TimelineItemId::Event { event_id } if event_id == FIRST_EVENT));
                    initial_counts.push((items.len(), found));
                    if found { break; }
                }
                CoreEvent::Timeline(TimelineEvent::ItemsUpdated { key, diffs, .. }) if key.room_id() == ROOM => {
                    delivered_in_updates |= diffs.iter().any(|diff| match diff {
                        TimelineDiff::PushFront { item } | TimelineDiff::PushBack { item } | TimelineDiff::Insert { item, .. } | TimelineDiff::Set { item, .. } => matches!(&item.id, TimelineItemId::Event { event_id } if event_id == FIRST_EVENT),
                        TimelineDiff::Reset { items } => items.iter().any(|item| matches!(&item.id, TimelineItemId::Event { event_id } if event_id == FIRST_EVENT)),
                        _ => false,
                    });
                }
                _ => {}
            }
        }
    }).await;
    if result.is_err() {
        let failed_at = tokio::time::Instant::now();
        phases.push(("cached_delivery_timeout", failed_at));
        let timing: Vec<_> = phases
            .windows(2)
            .map(|pair| {
                (
                    pair[1].0,
                    pair[1].1.duration_since(pair[0].1),
                    deadline.saturating_duration_since(pair[1].1),
                )
            })
            .collect();
        let observation_started = tokio::time::Instant::now();
        let cache = persisted_event_after_timeout(data.path(), credentials.path());
        panic!(
            "restored timeline reads persisted cache without server replay: timeout; phases=(phase, elapsed, remaining) {timing:?}; logout=not reached; initial_items=(count, contains_event) {initial_counts:?}; delivered_in_updates={delivered_in_updates}; persisted_event_after_timeout={cache:?}; observation_started_after_timeout={:?}; observation_duration={:?}; store may have changed since timeout",
            observation_started.duration_since(failed_at),
            observation_started.elapsed()
        );
    }
    phase.store(2, Ordering::Release);
    wait_for_state(&mut connection, deadline, |state| {
        has_latest(state, RESTORED_EVENT)
    })
    .await;

    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Account(AccountCommand::Logout { request_id }))
        .await
        .expect("submit logout");
    wait_for_state(&mut connection, deadline, |state| {
        matches!(state.session, SessionState::SignedOut) && state.rooms.is_empty()
    })
    .await;
    drop(connection);
    tokio::time::timeout_at(deadline, restored.shutdown())
        .await
        .expect("restored runtime settles");
}
