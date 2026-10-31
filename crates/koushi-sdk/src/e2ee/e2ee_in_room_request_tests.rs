//! Diagnostic regression for the sender-key-before-in-room-request ordering.
use std::time::Duration;

use super::{MatrixClientSession, observe_incoming_verification_requests};
use koushi_state::{SessionAuthenticationMethod, SessionInfo};
use matrix_sdk::{
    ruma::{RoomVersionId, device_id, event_id, room_id, user_id},
    test_utils::mocks::MatrixMockServer,
};
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};
use serde_json::json;

#[tokio::test]
async fn in_room_request_received_before_sender_device_keys_remains_actionable() {
    let server = MatrixMockServer::new().await;
    server.mock_crypto_endpoints_preset().await;
    let recipient_id = user_id!("@recipient:example.invalid");
    let sender_id = user_id!("@sender:example.invalid");
    let recipient = server
        .client_builder_for_crypto_end_to_end(recipient_id, device_id!("RECIPIENTDEVICE"))
        .build()
        .await;
    let sender = server
        .client_builder_for_crypto_end_to_end(sender_id, device_id!("SENDERDEVICE"))
        .build()
        .await;
    // Publish recipient keys, then teach only the sender about the recipient.
    // Receiving m.room.key does not require knowing the sender device keys.
    server.mock_sync().ok_and_run(&recipient, |_| {}).await;
    sender
        .update_tracked_users_for_testing([recipient_id])
        .await;
    server.mock_sync().ok_and_run(&sender, |_| {}).await;
    let room_id = room_id!("!request:example.invalid");
    let factory = EventFactory::new().room(room_id).sender(sender_id);
    let room = JoinedRoomBuilder::new(room_id)
        .add_state_event(factory.create(sender_id, RoomVersionId::V1))
        .add_state_event(factory.room_encryption());
    server
        .mock_sync()
        .ok_and_run(&sender, |builder| {
            builder.add_joined_room(room.clone());
        })
        .await;
    server
        .mock_sync()
        .ok_and_run(&recipient, |builder| {
            builder.add_joined_room(room);
        })
        .await;
    let session = MatrixClientSession {
        client: recipient.clone(),
        diagnostic_counters: koushi_diagnostics::DiagnosticCounterContext::registered(),
        info: SessionInfo {
            homeserver: server.server().uri(),
            user_id: recipient_id.to_string(),
            device_id: "RECIPIENTDEVICE".to_owned(),
            authentication_method: SessionAuthenticationMethod::Unknown,
        },
    };
    let mut observer = observe_incoming_verification_requests(&session).await;
    let (event_receiver, send_mock) = server
        .mock_room_send()
        .ok_with_capture(event_id!("$incoming-request:example.invalid"), sender_id);
    let (_key_guard, room_key) = server.mock_capture_put_to_device(sender_id).await;
    {
        let _send_guard = send_mock.mock_once().mount_as_scoped().await;
        server
            .mock_get_members()
            .ok(vec![
                factory.member(sender_id).into_raw(),
                factory.member(recipient_id).into_raw(),
            ])
            .mock_once()
            .mount()
            .await;
        sender.get_room(room_id).expect("sender room exists").send_raw("m.room.message", json!({
            "body": "synthetic verification request", "msgtype": "m.key.verification.request",
            "to": recipient_id, "from_device": "SENDERDEVICE", "methods": ["m.sas.v1"]
        })).await.expect("request should send");
    }
    let event = event_receiver
        .await
        .expect("encrypted event should be captured");
    let key = room_key.await;
    server
        .mock_sync()
        .ok_and_run(&recipient, |builder| {
            builder.add_to_device_event(key.deserialize_as().expect("room key should deserialize"));
        })
        .await;
    assert_eq!(
        recipient
            .encryption()
            .get_user_devices(sender_id)
            .await
            .expect("device query should read store")
            .devices()
            .count(),
        0,
        "recipient must not know sender device keys before receiving request"
    );
    server
        .mock_sync()
        .ok_and_run(&recipient, |builder| {
            builder
                .add_joined_room(JoinedRoomBuilder::new(room_id).add_timeline_event(event.clone()));
        })
        .await;
    let (cache, _cache_handles) = recipient
        .event_cache()
        .unwrap()
        .room(room_id)
        .await
        .expect("room cache should exist");
    let (_, mut updates) = cache
        .subscribe()
        .await
        .expect("room cache should subscribe");
    let cached = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(event) = cache
                .find_event(event_id!("$incoming-request:example.invalid"))
                .await
                .expect("cache lookup should succeed")
            {
                break event;
            }
            updates
                .recv()
                .await
                .expect("cache should publish the incoming request");
        }
    })
    .await
    .expect("incoming request must reach the room cache");
    assert!(
        matches!(
            cached.kind,
            matrix_sdk::deserialized_responses::TimelineEventKind::Decrypted(_)
        ),
        "this regression must decrypt immediately rather than reproduce a UTD"
    );
    // Make sender keys available after the first event was already handled.
    server.exchange_e2ee_identities(&recipient, &sender).await;
    let received = tokio::time::timeout(Duration::from_secs(2), observer.recv()).await;
    // Prove the event fixture is actionable once sender keys are known. This
    // control distinguishes an unknown-device loss from malformed test input.
    if received.is_err() {
        server
            .mock_sync()
            .ok_and_run(&recipient, |builder| {
                builder.add_joined_room(JoinedRoomBuilder::new(room_id).add_timeline_event(event));
            })
            .await;
        let replay = tokio::time::timeout(Duration::from_secs(2), observer.recv()).await;
        assert!(
            replay.is_ok(),
            "known-sender replay must be actionable before diagnosing delivery loss"
        );
    }
    observer.shutdown().await;
    assert!(
        received.is_ok(),
        "an in-room request must survive sender-key discovery after its first delivery"
    );
    assert_eq!(
        received.unwrap().unwrap().target().user_id,
        sender_id.as_str()
    );
}
