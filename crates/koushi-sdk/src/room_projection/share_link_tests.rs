use super::matrix_room_settings_snapshot;
use matrix_sdk::{
    ruma::{events::room::canonical_alias::RoomCanonicalAliasEventContent, room_id},
    test_utils::mocks::MatrixMockServer,
};
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};

#[tokio::test]
async fn settings_share_link_tracks_the_sdk_permalink_after_alias_changes() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!share:example.org");
    for (alias, alternate) in [
        (
            Some("#original:example.org"),
            Some("#alternate:example.org"),
        ),
        (Some("#updated:example.org"), None),
        (None, Some("#alternate:example.org")),
        (None, None),
    ] {
        let mut content = RoomCanonicalAliasEventContent::new();
        content.alias = alias.map(|value| value.try_into().unwrap());
        content.alt_aliases = alternate
            .into_iter()
            .map(|value| value.try_into().unwrap())
            .collect();
        let event = EventFactory::new()
            .room(room_id)
            .sender(client.user_id().unwrap())
            .event(content)
            .state_key("")
            .into_raw_sync_state();
        server
            .mock_sync()
            .ok_and_run(&client, |builder| {
                builder.add_joined_room(JoinedRoomBuilder::new(room_id).add_state_event(event));
            })
            .await;
        let room = client.get_room(room_id).unwrap();
        let expected = room.matrix_to_permalink().await.unwrap().to_string();
        let snapshot = matrix_room_settings_snapshot(&room).await;
        assert_eq!(snapshot.share_link.as_deref(), Some(expected.as_str()));
        assert_eq!(snapshot.canonical_alias.as_deref(), alias);
    }
}
