//! Direct operations must not acquire display actors or settle a GUI composer.
use std::{collections::HashMap, sync::Arc, time::Duration};

use futures_util::StreamExt;
use koushi_protocol::{
    command::TimelineCommand,
    event::{CoreEvent, TimelineEvent},
    ids::{AccountKey, RequestId, TimelineKey},
};
use koushi_sdk::MatrixClientSession;
use koushi_state::{ComposerDocument, SessionAuthenticationMethod, SessionInfo};
use matrix_sdk::{
    ruma::{event_id, room_id, user_id},
    test_utils::mocks::MatrixMockServer,
};
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};
use tokio::sync::{broadcast, mpsc};

use super::super::manager::TimelineManagerActor;
use super::super::test_support::{fake_rid, live_tail_test_manager, test_timeline_actor_handle};
use super::run_global_send_completion_observer;
use super::{MatrixTimelineSendEnqueueContext, TimelineSendEnqueueContext};

#[tokio::test]
async fn direct_send_completes_in_unviewed_room_without_composer_actions() {
    for reply in [false, true] {
        let server = MatrixMockServer::new().await;
        let client = server.client_builder().build().await;
        let room = server
            .sync_joined_room(&client, room_id!("!unviewed:example.invalid"))
            .await;
        server.mock_room_state_encryption().plain().mount().await;
        let mut expected_body =
            serde_json::json!({ "body": "Synthetic background message", "msgtype": "m.text" });
        if reply {
            expected_body["m.relates_to"] =
                serde_json::json!({ "m.in_reply_to": { "event_id": "$original:example.invalid" } });
        }
        server
            .mock_room_send()
            .body_matches_partial_json(expected_body)
            .ok(event_id!("$sent:example.invalid"))
            .expect(1)
            .mount()
            .await;
        let session = Arc::new(MatrixClientSession::from_client_for_testing(
            client.clone(),
            SessionInfo {
                homeserver: server.server().uri(),
                user_id: client.user_id().unwrap().to_string(),
                device_id: client.device_id().unwrap().to_string(),
                authentication_method: SessionAuthenticationMethod::Unknown,
            },
        ));
        let key = TimelineKey::room(
            AccountKey(session.info.user_id.clone()),
            room.room_id().to_string(),
        );
        let mut manager = live_tail_test_manager(HashMap::new());
        manager.session = Some(session);
        let (action_tx, mut action_rx) = mpsc::channel(32);
        manager.action_tx = action_tx;
        let mut events = manager.event_tx.subscribe();
        let observer = tokio::spawn(run_global_send_completion_observer(
            client.send_queue().subscribe(),
            Arc::clone(&manager.send_completion),
            manager.terminal_ingress.clone(),
            Some(client),
        ));
        let request_id = fake_rid(9100);
        let document = ComposerDocument::from_plain_text("Synthetic background message");
        let command = if reply {
            TimelineCommand::SendReply {
                request_id,
                key: key.clone(),
                transaction_id: "background-reply".into(),
                in_reply_to_event_id: "$original:example.invalid".into(),
                document,
            }
        } else {
            TimelineCommand::SendText {
                request_id,
                key: key.clone(),
                transaction_id: "background-text".into(),
                document,
            }
        };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        manager.handle_command(command).await;
        assert!(
            events.try_recv().is_err(),
            "send must not fail admission without a display actor"
        );
        if let Some(completion) =
            tokio::time::timeout_at(deadline, manager.send_enqueue_workers.tasks.next())
                .await
                .unwrap()
        {
            manager
                .handle_send_enqueue_worker_completion(completion)
                .await;
        }
        let terminal = tokio::time::timeout_at(deadline, manager.terminal_rx.recv())
            .await
            .unwrap()
            .unwrap();
        manager.handle_send_terminal_handoff(terminal).await;
        assert!(
            matches!(events.try_recv().unwrap(), CoreEvent::Timeline(TimelineEvent::SendCompleted { request_id: completed, event_id, .. }) if completed == request_id && event_id == "$sent:example.invalid")
        );
        assert!(
            manager.timelines.is_empty(),
            "send must not create a display subscription"
        );
        assert!(
            action_rx.try_recv().is_err(),
            "direct send must leave GUI composer untouched"
        );
        observer.abort();
        let _ = observer.await;
    }
}

/// Drives one direct send to its terminal and returns the message content the
/// send queue actually PUT, so each reply test asserts what the server gets.
async fn direct_send_content(
    manager: &mut TimelineManagerActor,
    events: &mut broadcast::Receiver<CoreEvent>,
    server: &MatrixMockServer,
    request_id: RequestId,
    command: TimelineCommand,
) -> serde_json::Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    manager.handle_command(command).await;
    if let Some(completion) =
        tokio::time::timeout_at(deadline, manager.send_enqueue_workers.tasks.next())
            .await
            .unwrap()
    {
        manager
            .handle_send_enqueue_worker_completion(completion)
            .await;
    }
    let terminal = tokio::time::timeout_at(deadline, manager.terminal_rx.recv())
        .await
        .unwrap()
        .unwrap();
    manager.handle_send_terminal_handoff(terminal).await;
    assert!(
        matches!(events.try_recv().unwrap(), CoreEvent::Timeline(TimelineEvent::SendCompleted { request_id: completed, .. }) if completed == request_id)
    );
    let sent = server
        .server()
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .find(|request| {
            request.method.as_str() == "PUT" && request.url.path().contains("/send/m.room.message/")
        })
        .expect("the message was sent");
    serde_json::from_slice(&sent.body).expect("message content")
}

/// #1808: a direct (MCP) reply to an event that lives in a thread must stay in
/// that thread, as the thread composer's reply does, instead of posting a
/// main-timeline reply that quotes an in-thread event.
#[tokio::test]
async fn direct_reply_to_in_thread_event_stays_in_the_thread() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!threaded:example.invalid");
    let root_id = event_id!("$root:example.invalid");
    let in_thread_id = event_id!("$in-thread:example.invalid");
    let alice = user_id!("@alice:example.invalid");
    let factory = EventFactory::new().room(room_id).sender(alice);
    let room = server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(room_id)
                .add_timeline_event(
                    factory
                        .text_msg("Synthetic root")
                        .event_id(root_id)
                        .into_raw_sync(),
                )
                .add_timeline_event(
                    factory
                        .text_msg("Synthetic in-thread message")
                        .event_id(in_thread_id)
                        .in_thread(root_id, root_id)
                        .into_raw_sync(),
                ),
        )
        .await;
    // The synced target must be resolved from the room's own cache: this test
    // is the MCP case, where nothing is displayed and no request is wanted.
    server
        .mock_room_event()
        .match_event_id()
        .ok(factory
            .text_msg("Synthetic in-thread message")
            .event_id(in_thread_id)
            .in_thread(root_id, root_id)
            .into_event())
        .never()
        .mount()
        .await;
    server.mock_room_state_encryption().plain().mount().await;
    server
        .mock_room_send()
        .ok(event_id!("$sent:example.invalid"))
        .expect(1)
        .mount()
        .await;
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client.clone(),
        SessionInfo {
            homeserver: server.server().uri(),
            user_id: client.user_id().unwrap().to_string(),
            device_id: client.device_id().unwrap().to_string(),
            authentication_method: SessionAuthenticationMethod::Unknown,
        },
    ));
    let key = TimelineKey::room(
        AccountKey(session.info.user_id.clone()),
        room.room_id().as_str(),
    );
    let mut manager = live_tail_test_manager(HashMap::new());
    manager.session = Some(session);
    let (action_tx, _action_rx) = mpsc::channel(32);
    manager.action_tx = action_tx;
    let mut events = manager.event_tx.subscribe();
    let observer = tokio::spawn(run_global_send_completion_observer(
        client.send_queue().subscribe(),
        Arc::clone(&manager.send_completion),
        manager.terminal_ingress.clone(),
        Some(client),
    ));
    let request_id = fake_rid(9150);
    let content = direct_send_content(
        &mut manager,
        &mut events,
        &server,
        request_id,
        TimelineCommand::SendReply {
            request_id,
            key,
            transaction_id: "background-thread-reply".into(),
            in_reply_to_event_id: in_thread_id.to_string(),
            document: ComposerDocument::from_plain_text("Synthetic answer"),
        },
    )
    .await;
    observer.abort();
    let _ = observer.await;

    // A genuine in-thread reply is `is_falling_back: false`, which ruma omits
    // from the wire as its default, so the exact relation below pins it.
    assert_eq!(
        content["m.relates_to"],
        serde_json::json!({
            "rel_type": "m.thread",
            "event_id": root_id.as_str(),
            "m.in_reply_to": { "event_id": in_thread_id.as_str() },
        }),
        "a direct reply to an in-thread event must be sent into that thread"
    );
    assert_eq!(
        content["m.mentions"]["user_ids"],
        serde_json::json!([alice.as_str()]),
        "the quoted sender is mentioned through the same resolution"
    );
}

/// The other half of the same path (#1808): a room that is on screen can still
/// miss the reply target from its loaded window, and the reply must land in the
/// target's thread all the same.
#[tokio::test]
async fn direct_reply_in_a_viewed_room_stays_in_the_thread_outside_the_loaded_window() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!viewed:example.invalid");
    let root_id = event_id!("$root:example.invalid");
    let in_thread_id = event_id!("$in-thread:example.invalid");
    let alice = user_id!("@alice:example.invalid");
    let factory = EventFactory::new().room(room_id).sender(alice);
    let room = server
        .sync_room(&client, JoinedRoomBuilder::new(room_id))
        .await;
    // The window is empty, so the target can only come from its own read.
    server
        .mock_room_event()
        .match_event_id()
        .ok(factory
            .text_msg("Synthetic in-thread message")
            .event_id(in_thread_id)
            .in_thread(root_id, root_id)
            .into_event())
        .mock_once()
        .mount()
        .await;
    server.mock_room_state_encryption().plain().mount().await;
    server
        .mock_room_send()
        .ok(event_id!("$sent:example.invalid"))
        .expect(1)
        .mount()
        .await;
    let timeline = Arc::new(
        matrix_sdk_ui::timeline::TimelineBuilder::new(&room)
            .build()
            .await
            .expect("a room timeline"),
    );
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client.clone(),
        SessionInfo {
            homeserver: server.server().uri(),
            user_id: client.user_id().unwrap().to_string(),
            device_id: client.device_id().unwrap().to_string(),
            authentication_method: SessionAuthenticationMethod::Unknown,
        },
    ));
    let key = TimelineKey::room(
        AccountKey(session.info.user_id.clone()),
        room.room_id().as_str(),
    );
    let mut manager = live_tail_test_manager(HashMap::new());
    manager.session = Some(Arc::clone(&session));
    let (action_tx, _action_rx) = mpsc::channel(32);
    manager.action_tx = action_tx;
    let mut handle = test_timeline_actor_handle();
    handle.enqueue_context = Some(TimelineSendEnqueueContext::Matrix(Box::new(
        MatrixTimelineSendEnqueueContext {
            key: key.clone(),
            room: room.clone(),
            timeline: Some(timeline),
            actor_generation: None,
            session,
            cleanup: None,
            diagnostic_trace: None,
            sent_event_txns: Default::default(),
        },
    )));
    manager.timelines.insert(key.clone(), handle);
    let mut events = manager.event_tx.subscribe();
    let observer = tokio::spawn(run_global_send_completion_observer(
        client.send_queue().subscribe(),
        Arc::clone(&manager.send_completion),
        manager.terminal_ingress.clone(),
        Some(client),
    ));
    let request_id = fake_rid(9160);
    let content = direct_send_content(
        &mut manager,
        &mut events,
        &server,
        request_id,
        TimelineCommand::SendReply {
            request_id,
            key,
            transaction_id: "viewed-thread-reply".into(),
            in_reply_to_event_id: in_thread_id.to_string(),
            document: ComposerDocument::from_plain_text("Synthetic answer"),
        },
    )
    .await;
    observer.abort();
    let _ = observer.await;

    assert_eq!(
        content["m.relates_to"],
        serde_json::json!({
            "rel_type": "m.thread",
            "event_id": root_id.as_str(),
            "m.in_reply_to": { "event_id": in_thread_id.as_str() },
        }),
        "an unloaded in-thread target must still keep the reply in its thread"
    );
}

/// A direct reply to an event outside any thread stays a plain reply, and still
/// mentions the quoted sender.
#[tokio::test]
async fn direct_reply_outside_any_thread_stays_a_plain_reply() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!plain:example.invalid");
    let original_id = event_id!("$original:example.invalid");
    let alice = user_id!("@alice:example.invalid");
    let factory = EventFactory::new().room(room_id).sender(alice);
    let room = server
        .sync_room(&client, JoinedRoomBuilder::new(room_id))
        .await;
    server
        .mock_room_event()
        .match_event_id()
        .ok(factory
            .text_msg("Synthetic original")
            .event_id(original_id)
            .into_event())
        .mock_once()
        .mount()
        .await;
    server.mock_room_state_encryption().plain().mount().await;
    server
        .mock_room_send()
        .ok(event_id!("$sent:example.invalid"))
        .expect(1)
        .mount()
        .await;
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client.clone(),
        SessionInfo {
            homeserver: server.server().uri(),
            user_id: client.user_id().unwrap().to_string(),
            device_id: client.device_id().unwrap().to_string(),
            authentication_method: SessionAuthenticationMethod::Unknown,
        },
    ));
    let key = TimelineKey::room(
        AccountKey(session.info.user_id.clone()),
        room.room_id().as_str(),
    );
    let mut manager = live_tail_test_manager(HashMap::new());
    manager.session = Some(session);
    let (action_tx, _action_rx) = mpsc::channel(32);
    manager.action_tx = action_tx;
    let mut events = manager.event_tx.subscribe();
    let observer = tokio::spawn(run_global_send_completion_observer(
        client.send_queue().subscribe(),
        Arc::clone(&manager.send_completion),
        manager.terminal_ingress.clone(),
        Some(client),
    ));
    let request_id = fake_rid(9170);
    let content = direct_send_content(
        &mut manager,
        &mut events,
        &server,
        request_id,
        TimelineCommand::SendReply {
            request_id,
            key,
            transaction_id: "plain-reply".into(),
            in_reply_to_event_id: original_id.to_string(),
            document: ComposerDocument::from_plain_text("Synthetic answer"),
        },
    )
    .await;
    observer.abort();
    let _ = observer.await;

    assert_eq!(
        content["m.relates_to"],
        serde_json::json!({ "m.in_reply_to": { "event_id": original_id.as_str() } }),
        "a reply to an event outside any thread is not a thread reply"
    );
    assert_eq!(
        content["m.mentions"]["user_ids"],
        serde_json::json!([alice.as_str()])
    );
}

/// A target nobody can resolve keeps the reply it had before #1808: a send that
/// succeeds as a plain reply rather than a failed or thread-assumed send.
#[tokio::test]
async fn direct_reply_to_an_unresolvable_event_is_unchanged() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room = server
        .sync_joined_room(&client, room_id!("!unknown-target:example.invalid"))
        .await;
    server.mock_room_state_encryption().plain().mount().await;
    server
        .mock_room_send()
        .ok(event_id!("$sent:example.invalid"))
        .expect(1)
        .mount()
        .await;
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client.clone(),
        SessionInfo {
            homeserver: server.server().uri(),
            user_id: client.user_id().unwrap().to_string(),
            device_id: client.device_id().unwrap().to_string(),
            authentication_method: SessionAuthenticationMethod::Unknown,
        },
    ));
    let key = TimelineKey::room(
        AccountKey(session.info.user_id.clone()),
        room.room_id().as_str(),
    );
    let mut manager = live_tail_test_manager(HashMap::new());
    manager.session = Some(session);
    let (action_tx, _action_rx) = mpsc::channel(32);
    manager.action_tx = action_tx;
    let mut events = manager.event_tx.subscribe();
    let observer = tokio::spawn(run_global_send_completion_observer(
        client.send_queue().subscribe(),
        Arc::clone(&manager.send_completion),
        manager.terminal_ingress.clone(),
        Some(client),
    ));
    let request_id = fake_rid(9180);
    let content = direct_send_content(
        &mut manager,
        &mut events,
        &server,
        request_id,
        TimelineCommand::SendReply {
            request_id,
            key,
            transaction_id: "unknown-target-reply".into(),
            in_reply_to_event_id: "$missing:example.invalid".into(),
            document: ComposerDocument::from_plain_text("Synthetic answer"),
        },
    )
    .await;
    observer.abort();
    let _ = observer.await;

    assert_eq!(
        content["m.relates_to"],
        serde_json::json!({ "m.in_reply_to": { "event_id": "$missing:example.invalid" } }),
        "an unresolvable target keeps the existing plain reply"
    );
    assert_eq!(content.get("m.mentions"), None);
}

#[tokio::test]
async fn direct_send_rejects_foreign_missing_left_and_leaving_rooms() {
    use super::super::residency::RoomLeaveState;
    use koushi_protocol::failure::{CoreFailure, TimelineFailureKind};
    use matrix_sdk_test::LeftRoomBuilder;

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let joined = server
        .sync_joined_room(&client, room_id!("!joined:example.invalid"))
        .await;
    server
        .sync_room(
            &client,
            LeftRoomBuilder::new(room_id!("!left:example.invalid")),
        )
        .await;
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client.clone(),
        SessionInfo {
            homeserver: server.server().uri(),
            user_id: client.user_id().unwrap().to_string(),
            device_id: client.device_id().unwrap().to_string(),
            authentication_method: SessionAuthenticationMethod::Unknown,
        },
    ));
    let account = AccountKey(session.info.user_id.clone());
    let mut manager = live_tail_test_manager(HashMap::new());
    manager.session = Some(session);
    manager.room_leave_states.insert(
        joined.room_id().to_owned(),
        RoomLeaveState::PendingLeftObservation,
    );
    let (action_tx, mut actions) = mpsc::channel(32);
    manager.action_tx = action_tx;
    let mut events = manager.event_tx.subscribe();
    for (key, expected) in [
        (
            TimelineKey::room(
                AccountKey("@foreign:example.invalid".into()),
                joined.room_id().to_string(),
            ),
            TimelineFailureKind::InvalidSendTarget,
        ),
        (
            TimelineKey::room(account.clone(), "!missing:example.invalid"),
            TimelineFailureKind::NotFound,
        ),
        (
            TimelineKey::room(account.clone(), "!left:example.invalid"),
            TimelineFailureKind::Forbidden,
        ),
        (
            TimelineKey::room(account, joined.room_id().to_string()),
            TimelineFailureKind::Forbidden,
        ),
    ] {
        manager
            .handle_command(TimelineCommand::SendText {
                request_id: fake_rid(9200),
                key,
                transaction_id: "rejected-send".into(),
                document: ComposerDocument::from_plain_text("Synthetic rejected message"),
            })
            .await;
        assert!(
            matches!(events.try_recv().unwrap(), CoreEvent::OperationFailed { failure: CoreFailure::TimelineOperationFailed { kind }, .. } if kind == expected)
        );
        assert!(manager.send_enqueue_workers.tasks.is_empty());
    }
    assert!(actions.try_recv().is_err());
    assert!(manager.timelines.is_empty());
}
