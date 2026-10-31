#![cfg(feature = "test-hooks")]

//! An in-room verification request whose room key arrives after the request
//! event is first stored as a UTD. Once the event cache resolves it, the
//! incoming-verification observer must still surface it (#1462).

use std::time::Duration;

use koushi_sdk::{MatrixClientSession, observe_incoming_verification_requests};
use koushi_state::{SessionAuthenticationMethod, SessionInfo};
use matrix_sdk::{
    Client,
    encryption::EncryptionSettings,
    ruma::{RoomVersionId, device_id, event_id, room_id, user_id},
    test_utils::mocks::MatrixMockServer,
};
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};
use serde_json::json;

fn session_for(server: &MatrixMockServer, client: &Client) -> MatrixClientSession {
    let info = SessionInfo {
        homeserver: server.server().uri(),
        user_id: client.user_id().unwrap().to_string(),
        device_id: client.device_id().unwrap().to_string(),
        authentication_method: SessionAuthenticationMethod::Unknown,
    };
    MatrixClientSession::from_client_for_testing(client.clone(), info)
}

#[tokio::test(flavor = "multi_thread")]
async fn late_decrypted_in_room_request_reaches_incoming_observer() {
    let room_id = room_id!("!verification:localhost");
    let alice_user_id = user_id!("@alice:localhost");
    let bob_user_id = user_id!("@bob:localhost");

    let server = MatrixMockServer::new().await;
    server.mock_crypto_endpoints_preset().await;

    let settings = || EncryptionSettings {
        auto_enable_cross_signing: true,
        ..Default::default()
    };
    let alice = server
        .client_builder_for_crypto_end_to_end(alice_user_id, device_id!("ALICEDEVICE"))
        .on_builder(move |builder| builder.with_encryption_settings(settings()))
        .build()
        .await;
    let bob = server
        .client_builder_for_crypto_end_to_end(bob_user_id, device_id!("BOBDEVICE"))
        .on_builder(move |builder| builder.with_encryption_settings(settings()))
        .build()
        .await;
    server.exchange_e2ee_identities(&alice, &bob).await;

    let factory = EventFactory::new().room(room_id).sender(alice_user_id);
    let room = JoinedRoomBuilder::new(room_id)
        .add_state_event(factory.create(alice_user_id, RoomVersionId::V1))
        .add_state_event(factory.room_encryption());
    server
        .mock_sync()
        .ok_and_run(&alice, |builder| {
            builder.add_joined_room(room.clone());
        })
        .await;
    server
        .mock_sync()
        .ok_and_run(&bob, |builder| {
            builder.add_joined_room(room);
        })
        .await;

    // Alice sends an encrypted in-room verification request; capture both the
    // room event and the to-device room key so they can reach Bob separately.
    let (event_receiver, send_mock) = server
        .mock_room_send()
        .ok_with_capture(event_id!("$request"), alice_user_id);
    let (_to_device_guard, room_key) = server.mock_capture_put_to_device(alice_user_id).await;
    {
        let _send_guard = send_mock.mock_once().mount_as_scoped().await;
        server
            .mock_get_members()
            .ok(vec![
                factory.member(alice_user_id).into_raw(),
                factory.member(bob_user_id).into_raw(),
            ])
            .mock_once()
            .mount()
            .await;
        alice
            .get_room(room_id)
            .expect("alice should know the room")
            .send_raw(
                "m.room.message",
                json!({
                    "body": "verification request",
                    "msgtype": "m.key.verification.request",
                    "to": bob_user_id,
                    "from_device": "ALICEDEVICE",
                    "methods": ["m.sas.v1"],
                }),
            )
            .await
            .expect("alice should send the request");
    }
    let event = event_receiver
        .await
        .expect("the request should be captured");
    let room_key = room_key.await;

    let bob_session = session_for(&server, &bob);
    let mut observer = observe_incoming_verification_requests(&bob_session).await;

    // The request event arrives first and is a UTD.
    server
        .mock_sync()
        .ok_and_run(&bob, |builder| {
            builder.add_joined_room(JoinedRoomBuilder::new(room_id).add_timeline_event(event));
        })
        .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(200), observer.recv())
            .await
            .is_err(),
        "an undecryptable request cannot be surfaced yet"
    );

    // The room key arrives afterwards and the event cache resolves the UTD.
    server
        .mock_sync()
        .ok_and_run(&bob, |builder| {
            builder.add_to_device_event(
                room_key
                    .deserialize_as()
                    .expect("the room key should deserialize"),
            );
        })
        .await;

    let request = tokio::time::timeout(Duration::from_secs(5), observer.recv())
        .await
        .expect("the late-decrypted request should be surfaced")
        .expect("the observer should stay open");
    assert_eq!(request.target().user_id, alice_user_id.as_str());
    assert_eq!(request.target().device_id, "ALICEDEVICE");

    assert!(
        tokio::time::timeout(Duration::from_millis(100), observer.recv())
            .await
            .is_err(),
        "late-decrypted verification must reach the product through exactly one typed delivery"
    );

    observer.shutdown().await;
}
