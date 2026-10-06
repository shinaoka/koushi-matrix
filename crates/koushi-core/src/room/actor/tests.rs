use super::make_request_id;
use super::{RoomActor, RoomActorHandle, RoomMessage};

use koushi_protocol::command::RoomCommand;

use crate::executor;
use koushi_protocol::event::RoomEvent;

use koushi_sdk::MatrixClientSession;

use koushi_state::SessionInfo;
use koushi_state::{AppAction, RoomListSource};

#[cfg(any(test, feature = "test-hooks"))]
use std::sync::{Mutex, atomic::AtomicUsize};
use std::{sync::Arc, time::Duration};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

/// Bound for waits that must succeed: a regression hangs until it, while
/// scheduler load alone must never reach it.
const LIVENESS: Duration = Duration::from_secs(60);

#[tokio::test]
async fn room_actor_shutdown_aborts_when_its_mailbox_cannot_accept_shutdown() {
    let (tx, _rx) = mpsc::channel(1);
    tx.send(RoomMessage::Shutdown).await.expect("fill mailbox");
    let (timeline_residency, _timeline_residency_rx) = watch::channel(None);
    let (session, _session_rx) = watch::channel(None);
    let mut handle = RoomActorHandle {
        tx,
        timeline_residency,
        navigation_enrichment: crate::room::NavigationEnrichmentIngress::channel().0,
        session,
        #[cfg(any(test, feature = "test-hooks"))]
        room_operation_test_control: Arc::new(Mutex::new(None)),
        #[cfg(any(test, feature = "test-hooks"))]
        room_operation_test_reached_count: Arc::new(AtomicUsize::new(0)),
        task: Some(executor::spawn(std::future::pending())),
    };

    assert!(
        !handle
            .shutdown_with_timeouts(Duration::from_millis(10), Duration::from_millis(10))
            .await
    );
    assert!(handle.task.is_none());
}

#[tokio::test]
async fn routed_selection_is_never_projected_by_room_actor() {
    // #1060: AppActor owns room/Space selection. A selection command that
    // still reached this actor must not become a late, second selection.
    let (action_tx, mut action_rx) = mpsc::channel(16);
    let (event_tx, _event_rx) = broadcast::channel(16);
    let handle = RoomActor::spawn(
        action_tx,
        event_tx,
        crate::SlidingSyncDiagnostics::default(),
    );

    handle
        .send(RoomMessage::Command(RoomCommand::SelectSpace {
            request_id: make_request_id(1),
            space_id: Some("!space:example.test".to_owned()),
        }))
        .await;
    handle
        .send(RoomMessage::Command(RoomCommand::SelectRoom {
            request_id: make_request_id(2),
            room_id: "!room:example.test".to_owned(),
        }))
        .await;
    handle
        .send(RoomMessage::Command(RoomCommand::ReorderSpaces {
            request_id: make_request_id(3),
            space_ids: vec!["!space:example.test".to_owned()],
        }))
        .await;

    let actions = action_rx.recv().await.expect("actions");
    assert!(
        matches!(actions.as_slice(), [AppAction::ReorderSpaces { .. }]),
        "expected only the later ReorderSpaces projection, got {actions:?}"
    );
}

fn synthetic_session(
    server: &matrix_sdk::test_utils::mocks::MatrixMockServer,
    client: matrix_sdk::Client,
) -> Arc<MatrixClientSession> {
    Arc::new(MatrixClientSession::from_client_for_testing(
        client,
        SessionInfo {
            homeserver: server.uri(),
            user_id: "@alice:example.test".to_owned(),
            device_id: "ALICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    ))
}

async fn pinned_request_count(server: &matrix_sdk::test_utils::mocks::MatrixMockServer) -> usize {
    server.received_requests().await.map_or(0, |requests| {
        requests
            .iter()
            .filter(|request| request.url.path().contains("/state/m.room.pinned_events/"))
            .count()
    })
}

#[tokio::test]
async fn navigation_enrichment_refreshes_pins_for_the_current_session_only() {
    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    use wiremock::{
        Mock, ResponseTemplate,
        matchers::{method, path_regex},
    };

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = matrix_sdk::ruma::room_id!("!pinned:example.test");
    server.sync_joined_room(&client, room_id).await;
    Mock::given(method("GET"))
        .and(path_regex(
            r"^/_matrix/client/v3/rooms/.*/state/m.room.pinned_events/?$",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "pinned": [] })))
        .mount(&server)
        .await;
    let session = synthetic_session(&server, client);
    let session_key = crate::store::session_key_id_from_info(&session.info);
    let (action_tx, mut action_rx) = mpsc::channel(16);
    let (event_tx, _event_rx) = broadcast::channel(16);
    let handle = RoomActor::spawn(
        action_tx,
        event_tx,
        crate::SlidingSyncDiagnostics::default(),
    );
    let enrichment = handle.navigation_enrichment();

    // Demand retained before the session exists is replayed once it does.
    enrichment.admit(session_key.clone(), Some(room_id.to_string()), None, false);
    assert!(
        handle
            .send(RoomMessage::SessionEstablished { session })
            .await
    );
    let actions = tokio::time::timeout(Duration::from_secs(5), action_rx.recv())
        .await
        .expect("replayed pinned refresh projects")
        .expect("pinned projection");
    assert!(
        matches!(
            actions.as_slice(),
            [AppAction::RoomPinnedEventsUpdated { room_id: projected, .. }]
                if projected == room_id.as_str()
        ),
        "expected pinned projection, got {actions:?}"
    );
    assert_eq!(pinned_request_count(&server).await, 1);

    // Another account's demand never fetches under this session.
    enrichment.admit(
        koushi_protocol::SessionKeyId {
            user_id: "@mallory:example.test".to_owned(),
            ..session_key.clone()
        },
        Some(room_id.to_string()),
        None,
        false,
    );
    // Re-selecting the same room for this session refreshes again.
    enrichment.admit(session_key, Some(room_id.to_string()), None, false);
    let actions = tokio::time::timeout(Duration::from_secs(5), action_rx.recv())
        .await
        .expect("re-selection refreshes pins")
        .expect("pinned projection");
    assert!(matches!(
        actions.as_slice(),
        [AppAction::RoomPinnedEventsUpdated { .. }]
    ));
    assert_eq!(pinned_request_count(&server).await, 2);

    assert!(handle.send(RoomMessage::Shutdown).await);
    tokio::time::timeout(Duration::from_secs(1), handle.join())
        .await
        .expect("shutdown");
}

#[tokio::test]
async fn pinned_event_network_delay_does_not_block_later_room_work() {
    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    use wiremock::{
        Mock, ResponseTemplate,
        matchers::{method, path_regex},
    };

    let server = MatrixMockServer::new().await;
    // No client timeout: the delayed response keeps the pin fetch in flight for
    // the whole test, on real time, so nothing but the actor's own scheduling
    // decides whether later room work runs first.
    let client = server
        .client_builder()
        .on_builder(|builder| {
            builder.request_config(
                matrix_sdk::config::RequestConfig::new()
                    .disable_retry()
                    .timeout(None::<Duration>),
            )
        })
        .build()
        .await;
    let room_id = matrix_sdk::ruma::room_id!("!pinned:example.test");
    server.sync_joined_room(&client, room_id).await;
    Mock::given(method("GET"))
        .and(path_regex(
            r"^/_matrix/client/v3/rooms/.*/state/m.room.pinned_events/?$",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "pinned": [] }))
                .set_delay(Duration::from_secs(60 * 60)),
        )
        .expect(1)
        .mount(&server)
        .await;
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client,
        SessionInfo {
            homeserver: server.uri(),
            user_id: "@alice:example.test".to_owned(),
            device_id: "ALICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    ));
    let (action_tx, mut action_rx) = mpsc::channel(16);
    let (event_tx, _event_rx) = broadcast::channel(16);
    let handle = RoomActor::spawn(
        action_tx,
        event_tx,
        crate::SlidingSyncDiagnostics::default(),
    );
    assert!(
        handle
            .send(RoomMessage::SessionEstablished { session })
            .await
    );
    assert!(
        handle
            .send(RoomMessage::Command(RoomCommand::RefreshPinnedEvents {
                request_id: make_request_id(81),
                room_id: room_id.to_string(),
            }))
            .await
    );
    tokio::time::timeout(LIVENESS, async {
        while !server.received_requests().await.is_some_and(|requests| {
            requests
                .iter()
                .any(|request| request.url.path().contains("/state/m.room.pinned_events/"))
        }) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("pinned event request should start");
    assert!(
        action_rx.try_recv().is_err(),
        "pinned event fetch should still be pending"
    );

    assert!(
        handle
            .send(RoomMessage::Command(RoomCommand::ReorderSpaces {
                request_id: make_request_id(82),
                space_ids: vec!["!space:example.test".to_owned()],
            }))
            .await
    );
    let actions = tokio::time::timeout(LIVENESS, action_rx.recv())
        .await
        .expect("later room work must not wait for pin fetch")
        .expect("reorder action");
    assert!(matches!(
        actions.as_slice(),
        [AppAction::ReorderSpaces { .. }]
    ));
    assert!(handle.send(RoomMessage::Shutdown).await);
    tokio::time::timeout(LIVENESS, handle.join())
        .await
        .expect("shutdown");
    server.verify_and_reset().await;
}

#[tokio::test]
async fn reorder_spaces_projects_action() {
    let (action_tx, mut action_rx) = mpsc::channel(16);
    let (event_tx, _event_rx) = broadcast::channel(16);
    let handle = RoomActor::spawn(
        action_tx,
        event_tx,
        crate::SlidingSyncDiagnostics::default(),
    );

    handle
        .send(RoomMessage::Command(RoomCommand::ReorderSpaces {
            request_id: make_request_id(1),
            space_ids: vec![
                "!space-b:example.test".to_owned(),
                "!space-a:example.test".to_owned(),
            ],
        }))
        .await;

    let actions = action_rx.recv().await.expect("actions");
    assert!(
        matches!(
            actions.as_slice(),
            [AppAction::ReorderSpaces { space_ids }]
                if space_ids == &vec![
                        "!space-b:example.test".to_owned(),
                        "!space-a:example.test".to_owned()
                ]
        ),
        "expected ReorderSpaces action, got {actions:?}"
    );
}

#[test]
fn room_event_carries_request_id() {
    let request_id = make_request_id(10);
    let event = RoomEvent::RoomCreated {
        request_id,
        room_id: "!room:example.test".to_owned(),
    };
    match event {
        RoomEvent::RoomCreated {
            request_id: ev_id, ..
        } => assert_eq!(ev_id, request_id),
        other => panic!("unexpected event: {other:?}"),
    }
}

#[tokio::test]
async fn session_lifecycle_messages_without_session_complete_cleanly() {
    let (action_tx, _action_rx) = mpsc::channel(16);
    let (event_tx, _event_rx) = broadcast::channel(16);
    let handle = RoomActor::spawn(
        action_tx,
        event_tx,
        crate::SlidingSyncDiagnostics::default(),
    );

    // No session, no observation loop: both must be no-ops, and the
    // actor task must still exit on Shutdown.
    let (stop_ack_tx, stop_ack_rx) = oneshot::channel();
    assert!(
        handle
            .send(RoomMessage::StopSyncObservation {
                backend_generation: 1,
                ack: stop_ack_tx,
            })
            .await
    );
    stop_ack_rx.await.expect("stop acknowledgement");
    let (ack_tx, _ack_rx) = oneshot::channel();
    assert!(
        handle
            .send(RoomMessage::SessionCleared { ack: ack_tx })
            .await
    );
    assert!(handle.send(RoomMessage::Shutdown).await);
    tokio::time::timeout(std::time::Duration::from_secs(5), handle.join())
        .await
        .expect("actor task must exit after Shutdown");
}

#[tokio::test]
async fn stale_stop_generation_does_not_stop_replacement_observation() {
    use matrix_sdk::test_utils::mocks::MatrixMockServer;

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client.clone(),
        SessionInfo {
            homeserver: server.uri(),
            user_id: "@observer:example.invalid".to_owned(),
            device_id: "OBSERVER".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    ));
    let first_service = Arc::new(
        matrix_sdk_ui::room_list_service::RoomListService::new(client.clone())
            .await
            .expect("first room-list service"),
    );
    let replacement_service = Arc::new(
        matrix_sdk_ui::room_list_service::RoomListService::new(client)
            .await
            .expect("replacement room-list service"),
    );
    let (action_tx, _action_rx) = mpsc::channel(16);
    let (event_tx, _event_rx) = broadcast::channel(16);
    let handle = RoomActor::spawn(
        action_tx,
        event_tx,
        crate::SlidingSyncDiagnostics::default(),
    );

    assert!(
        handle
            .send(RoomMessage::SyncStarted {
                session: session.clone(),
                room_list_service: first_service,
                source: RoomListSource::Live,
                backend_generation: 1,
            })
            .await
    );
    assert!(
        handle
            .send(RoomMessage::SyncStarted {
                session,
                room_list_service: replacement_service,
                source: RoomListSource::Live,
                backend_generation: 2,
            })
            .await
    );

    let (stale_ack_tx, stale_ack_rx) = oneshot::channel();
    assert!(
        handle
            .send(RoomMessage::StopSyncObservation {
                backend_generation: 1,
                ack: stale_ack_tx,
            })
            .await
    );
    stale_ack_rx.await.expect("stale stop acknowledgement");

    let (inspect_tx, inspect_rx) = oneshot::channel();
    assert!(
        handle
            .send(RoomMessage::InspectObservationGeneration {
                response: inspect_tx,
            })
            .await
    );
    assert_eq!(inspect_rx.await.expect("observation generation"), Some(2));

    let (active_ack_tx, active_ack_rx) = oneshot::channel();
    assert!(
        handle
            .send(RoomMessage::StopSyncObservation {
                backend_generation: 2,
                ack: active_ack_tx,
            })
            .await
    );
    active_ack_rx.await.expect("active stop acknowledgement");

    let (inspect_tx, inspect_rx) = oneshot::channel();
    assert!(
        handle
            .send(RoomMessage::InspectObservationGeneration {
                response: inspect_tx,
            })
            .await
    );
    assert_eq!(inspect_rx.await.expect("stopped observation"), None);
    assert!(handle.send(RoomMessage::Shutdown).await);
    handle.join().await;
}
