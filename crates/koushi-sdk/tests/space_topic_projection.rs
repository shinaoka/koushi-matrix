#![cfg(feature = "test-hooks")]
#![recursion_limit = "256"]

//! #1661: the Space's confirmed description (`m.room.topic`) is part of the
//! room-list Space projection, so a change made by another member reaches
//! every surface that renders the Space without opening Space info.

use koushi_sdk::{
    MatrixClientSession, matrix_space_children_projection, room_list_snapshot_from_sdk_rooms,
};
use koushi_state::{SessionAuthenticationMethod, SessionInfo};
use matrix_sdk::{
    ruma::{RoomVersionId, events::space::child::SpaceChildEventContent, room_id},
    test_utils::mocks::MatrixMockServer,
};
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};
use wiremock::ResponseTemplate;

#[tokio::test]
async fn remote_space_topic_change_updates_the_projected_space() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let own = client.user_id().unwrap().to_owned();
    let other = matrix_sdk::ruma::user_id!("@admin:example.org");
    let space_id = room_id!("!space:example.org");
    let factory = EventFactory::new().room(space_id);

    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(space_id)
                .add_state_event(
                    factory
                        .create(other, RoomVersionId::V11)
                        .with_space_type()
                        .sender(other),
                )
                .add_state_event(factory.member(&own).sender(&own)),
        )
        .await;
    let projected = |snapshot: koushi_sdk::MatrixRoomListSnapshot| {
        snapshot
            .spaces
            .into_iter()
            .find(|space| space.space_id == space_id.as_str())
            .expect("the Space is projected")
            .topic
    };
    assert_eq!(
        projected(room_list_snapshot_from_sdk_rooms(client.joined_rooms()).await),
        None,
        "a Space without a topic has no description"
    );

    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(space_id).add_state_event(
                factory
                    .room_topic("Welcome\nPlease join the general room")
                    .sender(other),
            ),
        )
        .await;
    assert_eq!(
        projected(room_list_snapshot_from_sdk_rooms(client.joined_rooms()).await).as_deref(),
        Some("Welcome\nPlease join the general room"),
        "another member's topic change reaches the Space projection with its line breaks"
    );

    // Clearing the topic is an empty `m.room.topic`, which is no description.
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(space_id).add_state_event(factory.room_topic("").sender(other)),
        )
        .await;
    assert_eq!(
        projected(room_list_snapshot_from_sdk_rooms(client.joined_rooms()).await),
        None,
        "a cleared topic projects as no description"
    );
}

/// A sub-Space the account has not joined shows the description `/hierarchy`
/// reports for it, keeping its line breaks; a child without one has none.
#[tokio::test]
async fn not_joined_sub_space_carries_the_hierarchy_description() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let session = MatrixClientSession::from_client_for_testing(
        client.clone(),
        SessionInfo {
            homeserver: server.server().uri(),
            user_id: client.user_id().unwrap().to_string(),
            device_id: client.device_id().unwrap().to_string(),
            authentication_method: SessionAuthenticationMethod::Unknown,
        },
    );
    let own = client.user_id().unwrap().to_owned();
    let space_id = room_id!("!space:example.org");
    let factory = EventFactory::new().room(space_id).sender(&own);
    let mut space = JoinedRoomBuilder::new(space_id)
        .add_state_event(factory.create(&own, RoomVersionId::V11).with_space_type())
        .add_state_event(factory.member(&own));
    for child in ["!sub:example.org", "!plain:example.org"] {
        space = space.add_state_event(
            factory
                .event(SpaceChildEventContent::new(vec![
                    "example.org".try_into().unwrap(),
                ]))
                .state_key(child),
        );
    }
    server.sync_room(&client, space).await;
    let hierarchy_room = |room_id: &str, topic: Option<&str>| {
        let mut room = serde_json::json!({
            "room_id": room_id,
            "name": "Child",
            "room_type": "m.space",
            "num_joined_members": 2,
            "world_readable": false,
            "guest_can_join": false,
            "join_rule": "public",
            "children_state": [],
        });
        if let Some(topic) = topic {
            room["topic"] = topic.into();
        }
        room
    };
    server
        .mock_get_hierarchy()
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "rooms": [
                {
                    "room_id": space_id,
                    "room_type": "m.space",
                    "num_joined_members": 1,
                    "world_readable": false,
                    "guest_can_join": false,
                    "join_rule": "invite",
                    "children_state": [],
                },
                hierarchy_room("!sub:example.org", Some("Sub-Space guide\nSecond line")),
                hierarchy_room("!plain:example.org", None),
            ],
        })))
        .mount()
        .await;

    let entries = matrix_space_children_projection(&session, space_id.as_str())
        .await
        .unwrap();
    let topic_of = |room_id: &str| {
        entries
            .iter()
            .find(|entry| entry.room_id == room_id)
            .expect("the child is projected")
            .topic
            .clone()
    };
    assert_eq!(
        topic_of("!sub:example.org").as_deref(),
        Some("Sub-Space guide\nSecond line")
    );
    assert_eq!(topic_of("!plain:example.org"), None);
}
