//! #2135: cancelling an unsent message while the session-wide send queue is
//! closed (Secure Backup lock) must remove exactly that request, keep its
//! successors persisted, keep the queue closed, and send nothing.

use std::{sync::Arc, time::Duration};

use koushi_protocol::{
    event::CoreEvent,
    ids::{AccountKey, TimelineKey},
};
use koushi_sdk::MatrixClientSession;
use koushi_state::SessionInfo;
use matrix_sdk::{
    ruma::{events::room::message::RoomMessageEventContent, room_id, user_id},
    test_utils::mocks::MatrixMockServer,
};
use matrix_sdk_ui::timeline::TimelineFocus;
use tokio::sync::{broadcast, mpsc};

use super::super::{
    actor::{TimelineActor, TimelineActorMessage},
    relay::koushi_timeline_builder,
    test_support::{fake_rid, live_tail_test_manager},
};
use crate::{executor, link_preview::LinkPreviewContext};

#[tokio::test]
async fn cancelling_while_the_send_queue_is_closed_removes_only_that_request() {
    let server = MatrixMockServer::new().await;
    // The queue is closed: no request may reach the network.
    server
        .mock_room_send()
        .ok(matrix_sdk::ruma::event_id!("$never:example.invalid"))
        .expect(0)
        .mount()
        .await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!issue-2135:example.invalid");
    let alice = user_id!("@alice:example.invalid");
    let room = server.sync_joined_room(&client, room_id).await;

    // Close admission session-wide, then persist two requests behind it.
    client.send_queue().set_enabled(false).await;
    room.send_queue().set_enabled(false);
    let first = room
        .send_queue()
        .send(RoomMessageEventContent::text_plain("first").into())
        .await
        .expect("persist first");
    let second = room
        .send_queue()
        .send(RoomMessageEventContent::text_plain("second").into())
        .await
        .expect("persist second");
    let first_txn = first.transaction_id().to_string();
    let second_txn = second.transaction_id().to_string();

    let timeline = Arc::new(
        koushi_timeline_builder(
            &room,
            TimelineFocus::Live {
                hide_threaded_events: false,
            },
        )
        .build()
        .await
        .expect("room timeline"),
    );
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client.clone(),
        SessionInfo {
            homeserver: "https://example.invalid".into(),
            user_id: alice.to_string(),
            device_id: "SYNTHETIC".into(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    ));
    let key = TimelineKey::room(AccountKey(alice.to_string()), room_id.to_string());
    let mut manager = live_tail_test_manager(Default::default());
    let (action_tx, mut action_rx) = mpsc::channel(64);
    manager.action_tx = action_tx;
    let _action_drain = executor::spawn(async move { while action_rx.recv().await.is_some() {} });
    manager.event_tx = broadcast::channel(1024).0;
    let mut events = manager.event_tx.subscribe();
    let actor_generation = manager
        .timeline_actor_generations
        .activate_after_quiescence(&key)
        .await
        .generation;
    let actor = TimelineActor::spawn(
        key.clone(),
        timeline,
        session,
        fake_rid(1),
        true,
        manager.action_tx.clone(),
        manager.event_tx.clone(),
        None,
        Default::default(),
        None,
        LinkPreviewContext::default(),
        manager.account_work.clone(),
        manager.thread_root_projection_service.clone(),
        koushi_state::TimelineThreadRootOrder::RootEvent,
        false,
        true,
        manager.timeline_actor_generations.clone(),
        actor_generation,
        None,
        Default::default(),
        manager.terminal_ingress.clone(),
        manager.msg_tx.clone(),
        None,
    )
    .await;

    // Let the actor settle: it must hold both requests as Sending (replayed
    // by `subscribe`) before the command names one of them.
    while let Ok(Ok(_)) = tokio::time::timeout(Duration::from_millis(300), events.recv()).await {}

    actor
        .send(TimelineActorMessage::CancelSend {
            request_id: fake_rid(2),
            transaction_id: first_txn.clone(),
        })
        .await;
    let mut failures = Vec::new();
    while let Ok(Ok(event)) = tokio::time::timeout(Duration::from_millis(500), events.recv()).await
    {
        if let CoreEvent::OperationFailed { failure, .. } = event {
            failures.push(format!("{failure:?}"));
        }
    }
    assert!(failures.is_empty(), "cancel failed: {failures:?}");

    let (remaining, _updates) = room.send_queue().subscribe().await.expect("subscribe");
    let remaining: Vec<String> = remaining
        .iter()
        .map(|echo| echo.transaction_id.to_string())
        .collect();
    assert_eq!(
        remaining,
        vec![second_txn],
        "the cancelled request is gone and its successor stays persisted"
    );
    assert!(
        !client.send_queue().is_enabled() && !room.send_queue().is_enabled(),
        "cancelling must not re-open a closed queue (global={}, room={})",
        client.send_queue().is_enabled(),
        room.send_queue().is_enabled()
    );
    // `.expect(0)` on the send mock is verified when `server` drops.
}
