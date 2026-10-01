use matrix_sdk::{ruma::room_id, test_utils::mocks::MatrixMockServer};
use matrix_sdk_test::InvitedRoomBuilder;

#[tokio::test]
async fn invited_room_projection_carries_the_sdk_empty_name_placeholder() {
    let room_id = room_id!("!empty-invite:example.invalid");
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room = server
        .sync_room(&client, InvitedRoomBuilder::new(room_id))
        .await;

    let invites = super::matrix_invite_previews_from_rooms([room]).await;

    assert_eq!(invites.len(), 1);
    assert_eq!(invites[0].display_name, "Empty Room");
    assert_eq!(
        invites[0].display_name_placeholder,
        Some(koushi_state::RoomNamePlaceholder::Empty)
    );
}
