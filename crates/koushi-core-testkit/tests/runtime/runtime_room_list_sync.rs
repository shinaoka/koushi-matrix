//! Runtime room-list projection tests.

use std::{
    collections::BTreeSet,
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use koushi_core::{
    SyncEvent,
    runtime::{CoreConnection, CoreRuntime},
};
use koushi_protocol::command::{AccountCommand, AppCommand, CoreCommand};
use koushi_state::{
    AppAction, AuthSecret, LoginRequest, RoomListFilter, RoomListReadiness, SessionState, SyncState,
};
use matrix_sdk::test_utils::mocks::MatrixMockServer;
use serde_json::{Map, Value, json};
use wiremock::{
    Mock, Request, Respond, ResponseTemplate,
    matchers::{method, path},
};

#[path = "../support/mod.rs"]
#[expect(
    clippy::duplicate_mod,
    reason = "suite aggregation preserves feature-local fixture scopes and registered helper tests"
)]
mod support;
use support::*;

#[test]
fn sync_event_wire_has_no_backend_or_mode_transition() {
    let started: SyncEvent = serde_json::from_value(json!({
        "Started": { "request_id": null }
    }))
    .expect("deserialize backend-free sync start");
    assert_eq!(
        serde_json::to_value(started).expect("serialize sync start"),
        json!({ "Started": { "request_id": null } })
    );

    for obsolete in [
        json!({ "Started": { "request_id": null, "backend": "SyncService" } }),
        json!({ "Started": { "request_id": null, "backend": "LegacySync" } }),
        json!({ "ModeChanged": { "mode": "legacy" } }),
        json!({ "ModeChanged": { "mode": "simplified" } }),
        json!({ "ModeChanged": { "mode": "transitioning" } }),
    ] {
        assert!(
            serde_json::from_value::<SyncEvent>(obsolete).is_err(),
            "obsolete sync wire state must be rejected"
        );
    }
}

const SLIDING_SYNC_PATH: &str = "/_matrix/client/unstable/org.matrix.simplified_msc3575/sync";
const INVITED_ROOM_ID: &str = "!invited:localhost";
const EXPECTED_JOINED_ROOMS: usize = 20;

#[derive(Clone)]
struct EchoRequestedLoginDevice;

impl Respond for EchoRequestedLoginDevice {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).expect("login request JSON");
        let device_id = body["device_id"]
            .as_str()
            .expect("fresh login generated device id");
        ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "synthetic-access-token",
            "device_id": device_id,
            "user_id": "@runtime-room-list:localhost"
        }))
    }
}

#[derive(Clone)]
struct RuntimeSlidingSyncResponder {
    room_list_requests: Arc<AtomicUsize>,
    encryption_requests: Arc<AtomicUsize>,
    request_bodies: Arc<Mutex<Vec<Value>>>,
    room_list_request_tx: tokio::sync::mpsc::UnboundedSender<usize>,
    first_response_release: Arc<Barrier>,
    complete_range_release: Arc<Barrier>,
}

impl Respond for RuntimeSlidingSyncResponder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).expect("sliding-sync request JSON");
        self.request_bodies
            .lock()
            .expect("request capture lock")
            .push(body.clone());

        match body["conn_id"].as_str() {
            Some("room-list") => {
                let request_index = self.room_list_requests.fetch_add(1, Ordering::AcqRel);
                let _ = self.room_list_request_tx.send(request_index);
                match request_index {
                    // The first committed response proves transport but not the complete
                    // loaded range: count 21 makes the service expand 0..=19 to 0..=20.
                    0 => {
                        self.first_response_release.wait();
                        room_list_response(&body, "room-pos-0", 19)
                    }
                    // Hold the complete-range response after request capture so the test can
                    // inspect the projection between commit and RoomActor reconciliation.
                    1 => {
                        self.complete_range_release.wait();
                        room_list_response(&body, "room-pos-1", 20)
                    }
                    // Exercise the SyncService's own reconnect loop without replacing it.
                    2 => ResponseTemplate::new(500).set_body_json(json!({
                        "errcode": "M_UNKNOWN",
                        "error": "synthetic reconnect"
                    })),
                    _ => room_list_response(&body, "room-pos-2", 20),
                }
            }
            Some("encryption") => {
                let request_index = self.encryption_requests.fetch_add(1, Ordering::AcqRel);
                response_with_request_transaction(
                    &body,
                    json!({ "pos": format!("encryption-pos-{request_index}") }),
                )
            }
            other => panic!("unexpected sliding-sync conn_id: {other:?}"),
        }
    }
}

fn room_list_response(request: &Value, pos: &str, last_room_index: usize) -> ResponseTemplate {
    let room_ids = (0..=last_room_index)
        .map(|index| {
            if index == EXPECTED_JOINED_ROOMS {
                INVITED_ROOM_ID.to_owned()
            } else {
                format!("!joined-{index}:localhost")
            }
        })
        .collect::<Vec<_>>();
    let mut rooms = Map::new();
    for (index, room_id) in room_ids.iter().enumerate() {
        let room = if room_id == INVITED_ROOM_ID {
            json!({
                "initial": true,
                "name": "Synthetic invite",
                "invite_state": [{
                    "type": "m.room.member",
                    "state_key": "@example:localhost",
                    "sender": "@inviter:localhost",
                    "content": { "membership": "invite" }
                }]
            })
        } else {
            json!({
                "initial": true,
                "required_state": [
                    {
                        "type": "m.room.name",
                        "state_key": "",
                        "sender": "@example:localhost",
                        "event_id": format!("$name-{index}:localhost"),
                        "origin_server_ts": index,
                        "content": { "name": format!("Synthetic room {index}") }
                    },
                    {
                        "type": "m.room.member",
                        "state_key": "@example:localhost",
                        "sender": "@example:localhost",
                        "event_id": format!("$member-{index}:localhost"),
                        "origin_server_ts": index,
                        "content": { "membership": "join" }
                    }
                ]
            })
        };
        rooms.insert(room_id.clone(), room);
    }

    response_with_request_transaction(
        request,
        json!({
            "pos": pos,
            "lists": {
                "all_rooms": {
                    "count": EXPECTED_JOINED_ROOMS + 1,
                    "ops": [{
                        "op": "SYNC",
                        "range": [0, last_room_index],
                        "room_ids": room_ids
                    }]
                }
            },
            "rooms": rooms,
            "extensions": {}
        }),
    )
}

fn response_with_request_transaction(request: &Value, mut response: Value) -> ResponseTemplate {
    if let Some(transaction_id) = request.get("txn_id") {
        response
            .as_object_mut()
            .expect("sliding-sync response object")
            .insert("txn_id".to_owned(), transaction_id.clone());
    }
    ResponseTemplate::new(200).set_body_json(response)
}

/// A synthetic homeserver that withholds the complete `all_rooms` range, plus
/// the verified runtime logged into it.
///
/// Room-list request 0 answers with a partial range (19 of the 21 entries the
/// `count` announces) and request 1, the complete range, is held on a barrier.
/// The RoomActor's reconciliation of the partial commit therefore cannot finish
/// until the test releases the barrier, which is the state both tests below
/// inspect.
struct HeldRoomListRange {
    server: MatrixMockServer,
    encryption_requests: Arc<AtomicUsize>,
    request_bodies: Arc<Mutex<Vec<Value>>>,
    room_list_request_rx: tokio::sync::mpsc::UnboundedReceiver<usize>,
    first_response_release: Arc<Barrier>,
    complete_range_release: Arc<Barrier>,
    runtime: CoreRuntime,
    connection: CoreConnection,
    // The runtime's stores live as long as it does.
    _data_dir: tempfile::TempDir,
    _credential_dir: tempfile::TempDir,
}

impl HeldRoomListRange {
    async fn start() -> Self {
        let server = MatrixMockServer::new().await;
        server
            .mock_versions()
            .with_feature("org.matrix.simplified_msc3575", true)
            .ok()
            .mount()
            .await;
        Mock::given(method("POST"))
            .and(path("/_matrix/client/v3/login"))
            .respond_with(EchoRequestedLoginDevice)
            .expect(1)
            .mount(server.server())
            .await;

        let room_list_requests = Arc::new(AtomicUsize::new(0));
        let encryption_requests = Arc::new(AtomicUsize::new(0));
        let request_bodies = Arc::new(Mutex::new(Vec::new()));
        let (room_list_request_tx, room_list_request_rx) = tokio::sync::mpsc::unbounded_channel();
        let first_response_release = Arc::new(Barrier::new(2));
        let complete_range_release = Arc::new(Barrier::new(2));
        Mock::given(method("POST"))
            .and(path(SLIDING_SYNC_PATH))
            .respond_with(RuntimeSlidingSyncResponder {
                room_list_requests: room_list_requests.clone(),
                encryption_requests: encryption_requests.clone(),
                request_bodies: request_bodies.clone(),
                room_list_request_tx,
                first_response_release: first_response_release.clone(),
                complete_range_release: complete_range_release.clone(),
            })
            .mount(server.server())
            .await;

        let data_dir = tempfile::tempdir().expect("runtime data directory");
        let credential_dir = tempfile::tempdir().expect("runtime credential directory");
        let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
            data_dir.path().to_path_buf(),
            credential_dir.path().to_path_buf(),
        );
        assert!(
            runtime
                .configure_trust_observation_for_testing(
                    koushi_sdk::CurrentDeviceTrustObservation {
                        current: koushi_state::CurrentDeviceTrustState::Verified,
                        updates: Box::pin(futures_util::stream::pending()),
                    }
                )
                .await
        );
        let connection = runtime.attach();
        let login_request_id = connection.next_request_id();
        connection
            .command(CoreCommand::Account(AccountCommand::LoginPassword {
                request_id: login_request_id,
                request: LoginRequest {
                    homeserver: server.uri(),
                    username: "runtime-room-list".to_owned(),
                    password: AuthSecret::new("synthetic-password"),
                    device_display_name: None,
                },
                platform: koushi_state::DisplayPlatform::Linux,
            }))
            .await
            .expect("submit runtime login");

        Self {
            server,
            encryption_requests,
            request_bodies,
            room_list_request_rx,
            first_response_release,
            complete_range_release,
            runtime,
            connection,
            _data_dir: data_dir,
            _credential_dir: credential_dir,
        }
    }
}

/// Wait for the next room-list request index, so a test observes the SDK at a
/// named point instead of on a clock.
async fn next_room_list_request(
    requests: &mut tokio::sync::mpsc::UnboundedReceiver<usize>,
    expected: usize,
    label: &str,
) {
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), requests.recv())
            .await
            .unwrap_or_else(|_| panic!("{label} deadline")),
        Some(expected),
        "{label}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn normal_runtime_waits_for_full_all_rooms_reconciliation_and_reuses_one_sync_engine() {
    let fixture = HeldRoomListRange::start().await;
    let HeldRoomListRange {
        server,
        encryption_requests,
        request_bodies,
        mut room_list_request_rx,
        first_response_release,
        complete_range_release,
        runtime,
        mut connection,
        // The runtime's stores live as long as it does, so bind them here.
        _data_dir,
        _credential_dir,
    } = fixture;

    next_room_list_request(&mut room_list_request_rx, 0, "first room-list request").await;
    let state_before_first_response = connection.snapshot().sync;
    first_response_release.wait();
    assert_ne!(
        state_before_first_response,
        SyncState::Running,
        "SDK Running before the first response is not a connected projection"
    );

    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), room_list_request_rx.recv())
            .await
            .expect("complete-range room-list request deadline"),
        Some(1)
    );
    let state_after_partial_commit = connection.snapshot();
    complete_range_release.wait();

    let reconciled = wait_for_state_event(&mut connection, |state| {
        matches!(state.sync, SyncState::Running)
            && state.rooms.len() == EXPECTED_JOINED_ROOMS
            && state.invites.len() == 1
            && state.invites[0].room_id == INVITED_ROOM_ID
    })
    .await;
    assert_eq!(reconciled.rooms.len(), EXPECTED_JOINED_ROOMS);
    assert_eq!(reconciled.invites[0].room_id, INVITED_ROOM_ID);
    assert_eq!(
        runtime.inspect_sync_owners_for_testing().await,
        (false, false, true),
        "normal account runtime owns exactly one SyncActor"
    );

    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), room_list_request_rx.recv())
            .await
            .expect("failing reconnect request deadline"),
        Some(2)
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), room_list_request_rx.recv())
            .await
            .expect("recovery request deadline"),
        Some(3)
    );
    wait_for_state_event(&mut connection, |state| {
        matches!(state.sync, SyncState::Running)
            && state.rooms.len() == EXPECTED_JOINED_ROOMS
            && state.invites.len() == 1
    })
    .await;
    assert_eq!(
        runtime.inspect_sync_owners_for_testing().await,
        (false, false, true),
        "reconnect must retain the one normal SyncActor owner"
    );

    let requests = server
        .received_requests()
        .await
        .expect("captured HTTP requests");
    assert!(
        requests
            .iter()
            .all(|request| request.url.path() != "/_matrix/client/v3/sync"),
        "normal runtime must never issue classic /v3/sync"
    );
    {
        let bodies = request_bodies.lock().expect("request capture lock");
        let conn_ids = bodies
            .iter()
            .filter_map(|body| body["conn_id"].as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(conn_ids, BTreeSet::from(["encryption", "room-list"]));
        assert!(encryption_requests.load(Ordering::Acquire) > 0);
        let first_room_list = bodies
            .iter()
            .find(|body| body["conn_id"] == "room-list")
            .expect("room-list request body");
        assert_eq!(
            first_room_list["lists"]
                .as_object()
                .expect("room-list lists object")
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["all_rooms"]
        );
        assert!(
            first_room_list["lists"]["all_rooms"]["filters"]
                .get("is_invite")
                .is_none_or(Value::is_null),
            "all_rooms must be unfiltered so joined and invited rooms share one list"
        );
    }
    drop(connection);

    tokio::time::timeout(Duration::from_secs(15), runtime.shutdown())
        .await
        .expect("ordered runtime shutdown must stop and join all actor owners");

    // #1634: this used to read `state_after_partial_commit.sync != SyncState::Running`.
    // `SyncState::Running` now means "this run committed a response" — the sync
    // lifecycle's own evidence — and no longer doubles as "the room list is
    // authoritative", so the intent is pinned on the signal that owns it.
    assert!(
        !matches!(
            state_after_partial_commit.room_list.readiness,
            RoomListReadiness::Ready { .. }
        ),
        "a committed partial all_rooms range must not present an authoritative room list \
         before the RoomActor reconciles the complete loaded range"
    );
}

/// #1634, the inverse of the nightly failure: a run that commits a response
/// while the room-list reconcile never answers must still reach `Running`.
///
/// The runtime's reconciliation round trip to the RoomActor is held open for the
/// whole test, which is the shape the nightly lane produced (the observer sat in
/// that round trip while the service kept committing). The sync lifecycle must
/// not wait for it: a committed response is this run's own evidence that the
/// service is running, and the room list keeps its separate readiness signal.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn committed_response_projects_running_while_the_room_list_reconcile_is_pending() {
    let HeldRoomListRange {
        server: _server,
        mut room_list_request_rx,
        first_response_release,
        complete_range_release,
        runtime,
        mut connection,
        _data_dir,
        _credential_dir,
        ..
    } = HeldRoomListRange::start().await;
    // Hold the reconciliation before the run can reach it.
    let release_reconcile = runtime.hold_live_room_list_reconcile_for_testing();

    next_room_list_request(&mut room_list_request_rx, 0, "first room-list request").await;
    first_response_release.wait();
    // The commit has landed; request 1 is the next sync request the service
    // issues after applying it.
    next_room_list_request(
        &mut room_list_request_rx,
        1,
        "complete-range room-list request",
    )
    .await;
    // Release the withheld range here: from this point only the reconciliation
    // round trip is pending, so a failed assertion cannot leave a responder
    // blocked on a barrier.
    complete_range_release.wait();

    let running = tokio::time::timeout(
        Duration::from_secs(10),
        wait_for_state_event(&mut connection, |state| {
            matches!(state.sync, SyncState::Running)
        }),
    )
    .await
    .expect("a committed response must project Running while the room-list reconcile is pending");
    assert!(
        !matches!(running.room_list.readiness, RoomListReadiness::Ready { .. }),
        "the sync lifecycle must be closed without claiming an authoritative room list"
    );

    // Release the held round trip so the run settles before ordered shutdown.
    let _ = release_reconcile.send(());
    wait_for_state_event(&mut connection, |state| {
        matches!(state.sync, SyncState::Running)
            && state.rooms.len() == EXPECTED_JOINED_ROOMS
            && matches!(state.room_list.readiness, RoomListReadiness::Ready { .. })
    })
    .await;

    drop(connection);
    tokio::time::timeout(Duration::from_secs(15), runtime.shutdown())
        .await
        .expect("ordered runtime shutdown must stop and join all actor owners");
}

#[tokio::test]
async fn select_room_list_filter_command_updates_projection_through_runtime() {
    let (runtime, _stores) = CoreRuntime::start_isolated();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(restore_ready_actions![AppAction::RoomListUpdated {
            spaces: vec![],
            rooms: vec![
                unread_room_summary("!room:example.test", 5),
                unread_room_summary("!dm:example.test", 0),
            ],
        },])
        .await;
    wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_)) && state.rooms.len() == 2
    })
    .await;

    let request_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::SelectRoomListFilter {
        request_id,
        filter: RoomListFilter::Unread,
    }))
    .await
    .expect("select room list filter command");

    let snapshot = wait_for_state(&mut conn, |state| {
        state.room_list.active_filter == RoomListFilter::Unread
            && state.room_list.items.len() == 1
            && state.room_list.items[0].room_id == "!room:example.test"
    })
    .await;
    assert_eq!(snapshot.room_list.active_filter, RoomListFilter::Unread);
    assert_eq!(
        snapshot
            .room_list
            .items
            .iter()
            .map(|item| item.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["!room:example.test"]
    );
}

#[tokio::test]
async fn mark_as_read_and_unread_success_actions_update_room_list_projection() {
    let (runtime, _stores) = CoreRuntime::start_isolated();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(restore_ready_actions![AppAction::RoomListUpdated {
            spaces: vec![],
            rooms: vec![unread_room_summary("!room:example.test", 3)],
        },])
        .await;
    wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_)) && state.rooms.len() == 1
    })
    .await;

    runtime
        .inject_actions(vec![AppAction::RoomMarkedAsUnreadSucceeded {
            request_id: 1,
            room_id: "!room:example.test".to_owned(),
            unread: true,
        }])
        .await;
    let snapshot = wait_for_state(&mut conn, |state| {
        state
            .rooms
            .iter()
            .any(|room| room.room_id == "!room:example.test" && room.marked_unread)
    })
    .await;
    assert!(
        snapshot
            .rooms
            .iter()
            .any(|room| room.room_id == "!room:example.test" && room.marked_unread)
    );

    runtime
        .inject_actions(vec![AppAction::RoomMarkedAsReadSucceeded {
            request_id: 2,
            room_id: "!room:example.test".to_owned(),
        }])
        .await;
    let snapshot = wait_for_state(&mut conn, |state| {
        state
            .rooms
            .iter()
            .any(|room| room.room_id == "!room:example.test" && !room.marked_unread)
    })
    .await;
    assert!(
        snapshot
            .rooms
            .iter()
            .any(|room| room.room_id == "!room:example.test" && !room.marked_unread)
    );
    assert_eq!(
        snapshot
            .rooms
            .iter()
            .find(|room| room.room_id == "!room:example.test")
            .unwrap()
            .unread_count,
        0
    );
}

// #1755: a first response that never commits is recorded once per bounded
// window with the last completed phase, and cleared once when it commits.
const FIRST_RESPONSE_STALL_STAGE: &str = "first_response_stalled";
const FIRST_RESPONSE_CLEARED_STAGE: &str = "first_response_stalled_cleared";

struct FirstResponseWatchdogHarness {
    _server: MatrixMockServer,
    _data_dir: tempfile::TempDir,
    _credential_dir: tempfile::TempDir,
    runtime: CoreRuntime,
    connection: Option<CoreConnection>,
    room_list_request_rx: tokio::sync::mpsc::UnboundedReceiver<usize>,
    first_response_release: Arc<Barrier>,
    complete_range_release: Arc<Barrier>,
}

/// Log in a normal account runtime whose first room-list response stays held
/// until the test releases `first_response_release`.
async fn first_response_watchdog_harness() -> FirstResponseWatchdogHarness {
    let server = MatrixMockServer::new().await;
    server
        .mock_versions()
        .with_feature("org.matrix.simplified_msc3575", true)
        .ok()
        .mount()
        .await;
    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/login"))
        .respond_with(EchoRequestedLoginDevice)
        .expect(1)
        .mount(server.server())
        .await;

    let (room_list_request_tx, room_list_request_rx) = tokio::sync::mpsc::unbounded_channel();
    let first_response_release = Arc::new(Barrier::new(2));
    let complete_range_release = Arc::new(Barrier::new(2));
    Mock::given(method("POST"))
        .and(path(SLIDING_SYNC_PATH))
        .respond_with(RuntimeSlidingSyncResponder {
            room_list_requests: Arc::new(AtomicUsize::new(0)),
            encryption_requests: Arc::new(AtomicUsize::new(0)),
            request_bodies: Arc::new(Mutex::new(Vec::new())),
            room_list_request_tx,
            first_response_release: first_response_release.clone(),
            complete_range_release: complete_range_release.clone(),
        })
        .mount(server.server())
        .await;

    let data_dir = tempfile::tempdir().expect("runtime data directory");
    let credential_dir = tempfile::tempdir().expect("runtime credential directory");
    let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    assert!(
        runtime
            .configure_trust_observation_for_testing(koushi_sdk::CurrentDeviceTrustObservation {
                current: koushi_state::CurrentDeviceTrustState::Verified,
                updates: Box::pin(futures_util::stream::pending()),
            })
            .await
    );
    let connection = runtime.attach();
    let login_request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Account(AccountCommand::LoginPassword {
            request_id: login_request_id,
            request: LoginRequest {
                homeserver: server.uri(),
                username: "first-response-watchdog".to_owned(),
                password: AuthSecret::new("synthetic-password"),
                device_display_name: None,
            },
            platform: koushi_state::DisplayPlatform::Linux,
        }))
        .await
        .expect("submit runtime login");

    FirstResponseWatchdogHarness {
        _server: server,
        _data_dir: data_dir,
        _credential_dir: credential_dir,
        runtime,
        connection: Some(connection),
        room_list_request_rx,
        first_response_release,
        complete_range_release,
    }
}

fn first_response_records(
    cursor: koushi_diagnostics::test_support::DetailCursor,
    stage: &str,
) -> Vec<koushi_diagnostics::DiagnosticRecord> {
    koushi_diagnostics::test_support::detail_records_since(cursor)
        .into_iter()
        .filter(|record| record.event.source == "core.sync" && record.event.stage == stage)
        .collect()
}

async fn wait_for_first_response_records(
    cursor: koushi_diagnostics::test_support::DetailCursor,
    stage: &'static str,
    deadline: Duration,
) -> Vec<koushi_diagnostics::DiagnosticRecord> {
    tokio::time::timeout(deadline, async {
        loop {
            let records = first_response_records(cursor, stage);
            if !records.is_empty() {
                return records;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{stage} must be recorded within {deadline:?}"))
}

fn diagnostic_value(
    record: &koushi_diagnostics::DiagnosticRecord,
    key: &str,
) -> koushi_diagnostics::DiagnosticValue {
    record
        .event
        .fields
        .iter()
        .find(|field| field.key == key)
        .unwrap_or_else(|| panic!("{key} field"))
        .value
        .clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn held_first_response_records_one_bounded_stall_then_one_clear() {
    use koushi_diagnostics::{DiagnosticLevel, DiagnosticValue};

    let _diagnostic_lock = koushi_diagnostics::test_support::lock_async().await;
    let diagnostic_start = koushi_diagnostics::test_support::detail_cursor();
    let mut harness = first_response_watchdog_harness().await;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), harness.room_list_request_rx.recv())
            .await
            .expect("first room-list request deadline"),
        Some(0),
        "the held response belongs to the run's first room-list request"
    );

    // A slow start inside the first window is not a stall.
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(
        first_response_records(diagnostic_start, FIRST_RESPONSE_STALL_STAGE).is_empty(),
        "the watchdog stays silent inside its first window"
    );

    let stalled = wait_for_first_response_records(
        diagnostic_start,
        FIRST_RESPONSE_STALL_STAGE,
        Duration::from_secs(45),
    )
    .await;
    assert_eq!(stalled.len(), 1, "exactly one record per bounded window");
    assert_eq!(stalled[0].event.level, DiagnosticLevel::Warn);
    assert_eq!(
        diagnostic_value(&stalled[0], "phase"),
        DiagnosticValue::Token("encryption_sync_requested"),
        "the record names the last completed phase"
    );
    let DiagnosticValue::Milliseconds(elapsed_ms) = diagnostic_value(&stalled[0], "elapsed_ms")
    else {
        panic!("elapsed_ms must be milliseconds")
    };
    assert!(
        elapsed_ms >= 30_000,
        "the record reports the run generation's elapsed wait, got {elapsed_ms}ms"
    );
    let DiagnosticValue::Count(generation) = diagnostic_value(&stalled[0], "generation") else {
        panic!("generation must be a count")
    };
    assert!(generation >= 1, "the record names its run generation");
    assert_eq!(
        stalled[0].event.fields.len(),
        3,
        "phase, elapsed_ms and generation are the whole privacy-safe record"
    );

    // The repeated window doubles, so the same stall cannot repeat at once.
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(
        first_response_records(diagnostic_start, FIRST_RESPONSE_STALL_STAGE).len(),
        1,
        "a repeated window must back off instead of repeating immediately"
    );

    // Releasing the held response clears the reported episode exactly once.
    harness.first_response_release.wait();
    let cleared = wait_for_first_response_records(
        diagnostic_start,
        FIRST_RESPONSE_CLEARED_STAGE,
        Duration::from_secs(15),
    )
    .await;
    assert_eq!(cleared.len(), 1, "one clear per reported stall");
    assert_eq!(cleared[0].event.level, DiagnosticLevel::Debug);
    assert_eq!(
        diagnostic_value(&cleared[0], "phase"),
        DiagnosticValue::Token("encryption_sync_requested")
    );

    drop(harness.connection.take());
    harness.complete_range_release.wait();
    tokio::time::timeout(Duration::from_secs(15), harness.runtime.shutdown())
        .await
        .expect("ordered runtime shutdown must stop and join all actor owners");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn committed_first_response_records_no_stall() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock_async().await;
    let diagnostic_start = koushi_diagnostics::test_support::detail_cursor();
    let mut harness = first_response_watchdog_harness().await;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), harness.room_list_request_rx.recv())
            .await
            .expect("first room-list request deadline"),
        Some(0)
    );

    harness.first_response_release.wait();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), harness.room_list_request_rx.recv())
            .await
            .expect("complete-range room-list request deadline"),
        Some(1)
    );
    harness.complete_range_release.wait();
    let connection = harness.connection.as_mut().expect("runtime connection");
    wait_for_state_event(connection, |state| {
        matches!(state.sync, SyncState::Running) && state.rooms.len() == EXPECTED_JOINED_ROOMS
    })
    .await;

    assert!(
        first_response_records(diagnostic_start, FIRST_RESPONSE_STALL_STAGE).is_empty(),
        "a start whose first response commits reports no stall"
    );
    assert!(
        first_response_records(diagnostic_start, FIRST_RESPONSE_CLEARED_STAGE).is_empty(),
        "nothing was reported, so nothing is cleared"
    );

    drop(harness.connection.take());
    tokio::time::timeout(Duration::from_secs(15), harness.runtime.shutdown())
        .await
        .expect("ordered runtime shutdown must stop and join all actor owners");
}
