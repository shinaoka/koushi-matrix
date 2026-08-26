//! Runtime room-list projection tests.

use std::{
    collections::BTreeSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use koushi_core::command::{AccountCommand, AppCommand, CoreCommand};
use koushi_core::{SyncEvent, runtime::CoreRuntime};
use koushi_state::{AppAction, AuthSecret, LoginRequest, RoomListFilter, SessionState, SyncState};
use matrix_sdk::test_utils::mocks::MatrixMockServer;
use serde_json::{Map, Value, json};
use wiremock::{
    Mock, Request, Respond, ResponseTemplate,
    matchers::{method, path},
};

mod support;
use support::*;

#[test]
fn production_runtime_requires_committed_all_rooms_readiness() {
    let sync_source = include_str!("../src/sync.rs");
    let production = sync_source
        .split("#[cfg(test)]\npub mod tests")
        .next()
        .expect("production sync source");

    assert!(production.contains("committed_all_rooms_response"));
    assert!(!production.contains("note_room_list_service_state"));
    assert!(!production.contains("probe_backend"));
    assert!(!production.contains("run_legacy_sync_loop"));
    assert!(production.contains("room_list_service: Arc<"));
    assert!(production.contains("room_list_service,"));
}

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

#[test]
fn production_core_has_no_legacy_or_mode_transition_vocabulary() {
    let sources = [
        include_str!("../src/event.rs"),
        include_str!("../src/state_delta.rs"),
        include_str!("../src/sync.rs"),
        include_str!("../src/room.rs"),
    ]
    .join("\n");

    for forbidden in [
        "SyncBackendKind",
        "LegacySync",
        "ModeChanged",
        "SyncMode",
        "sync_mode",
        "RoomListSource::Legacy",
        "RoomListSource::SyncService",
    ] {
        assert!(
            !sources.contains(forbidden),
            "production core still contains forbidden sync vocabulary: {forbidden}"
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
                    0 => room_list_response(&body, "room-pos-0", 19)
                        .set_delay(Duration::from_millis(300)),
                    // Hold the complete-range response after request capture so the test can
                    // inspect the projection between commit and RoomActor reconciliation.
                    1 => room_list_response(&body, "room-pos-1", 20)
                        .set_delay(Duration::from_millis(500)),
                    // Exercise the SyncService's own reconnect loop without replacing it.
                    2 => ResponseTemplate::new(500)
                        .set_body_json(json!({
                            "errcode": "M_UNKNOWN",
                            "error": "synthetic reconnect"
                        }))
                        .set_delay(Duration::from_millis(300)),
                    _ => room_list_response(&body, "room-pos-2", 20)
                        .set_delay(Duration::from_millis(500)),
                }
            }
            Some("encryption") => {
                let request_index = self.encryption_requests.fetch_add(1, Ordering::AcqRel);
                response_with_request_transaction(
                    &body,
                    json!({ "pos": format!("encryption-pos-{request_index}") }),
                )
                .set_delay(Duration::from_millis(500))
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

#[tokio::test]
async fn normal_runtime_waits_for_full_all_rooms_reconciliation_and_reuses_one_sync_engine() {
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
        .mount(&server.server())
        .await;

    let room_list_requests = Arc::new(AtomicUsize::new(0));
    let encryption_requests = Arc::new(AtomicUsize::new(0));
    let request_bodies = Arc::new(Mutex::new(Vec::new()));
    let (room_list_request_tx, mut room_list_request_rx) = tokio::sync::mpsc::unbounded_channel();
    Mock::given(method("POST"))
        .and(path(SLIDING_SYNC_PATH))
        .respond_with(RuntimeSlidingSyncResponder {
            room_list_requests: room_list_requests.clone(),
            encryption_requests: encryption_requests.clone(),
            request_bodies: request_bodies.clone(),
            room_list_request_tx,
        })
        .mount(&server.server())
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
    let mut connection = runtime.attach();
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

    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), room_list_request_rx.recv())
            .await
            .expect("first room-list request deadline"),
        Some(0)
    );
    assert_ne!(
        connection.snapshot().sync,
        SyncState::Running,
        "SDK Running before the first response is not a connected projection"
    );

    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), room_list_request_rx.recv())
            .await
            .expect("complete-range room-list request deadline"),
        Some(1)
    );
    let state_after_partial_commit = connection.snapshot().sync;

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
    drop(bodies);
    drop(connection);

    tokio::time::timeout(Duration::from_secs(15), runtime.shutdown())
        .await
        .expect("ordered runtime shutdown must stop and join all actor owners");

    assert_ne!(
        state_after_partial_commit,
        SyncState::Running,
        "a committed partial all_rooms range must not project Running before RoomActor reconciles the complete loaded range"
    );
}

#[tokio::test]
async fn select_room_list_filter_command_updates_projection_through_runtime() {
    let runtime = CoreRuntime::start();
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
    let runtime = CoreRuntime::start();
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
