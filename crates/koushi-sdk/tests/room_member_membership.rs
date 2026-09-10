#![cfg(feature = "test-hooks")]

use koushi_sdk::{MatrixClientSession, get_room_settings_snapshot};
use koushi_state::{RoomMemberMembership, SessionAuthenticationMethod, SessionInfo};
use matrix_sdk::{
    ruma::events::room::member::MembershipState, test_utils::mocks::MatrixMockServer,
};
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};

#[tokio::test]
async fn room_member_status_and_mention_eligibility_follow_invite_then_join() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let info = SessionInfo {
        homeserver: server.server().uri(),
        user_id: client.user_id().unwrap().to_string(),
        device_id: client.device_id().unwrap().to_string(),
        authentication_method: SessionAuthenticationMethod::Unknown,
    };
    let session = MatrixClientSession::from_client_for_testing(client.clone(), info);
    let room_id = matrix_sdk::ruma::room_id!("!membership:example.org");
    let member_id = matrix_sdk::ruma::user_id!("@member:example.org");
    server
        .mock_sync()
        .ok_and_run(&client, |builder| {
            builder.add_joined_room(JoinedRoomBuilder::new(room_id));
        })
        .await;
    server
        .mock_get_members()
        .ok(vec![
            EventFactory::new()
                .room(room_id)
                .member(member_id)
                .membership(MembershipState::Invite)
                .display_name("Sample Member")
                .into_raw(),
        ])
        .mock_once()
        .mount()
        .await;

    let invited = get_room_settings_snapshot(&session, room_id.as_str())
        .await
        .unwrap();
    assert_eq!(invited.members.len(), 1);
    assert_eq!(invited.members[0].membership, RoomMemberMembership::Invited);
    let mentions = session
        .joined_member_snapshot_no_sync(room_id.as_str())
        .await
        .unwrap();
    assert!(mentions.complete);
    assert!(
        mentions.members.is_empty(),
        "an invitation must not establish mention eligibility"
    );

    server
        .mock_sync()
        .ok_and_run(&client, |builder| {
            builder.add_joined_room(
                JoinedRoomBuilder::new(room_id).add_state_event(
                    EventFactory::new()
                        .room(room_id)
                        .member(member_id)
                        .membership(MembershipState::Join)
                        .display_name("Sample Member")
                        .into_raw_sync_state(),
                ),
            );
        })
        .await;
    let joined = get_room_settings_snapshot(&session, room_id.as_str())
        .await
        .unwrap();
    assert_eq!(joined.members.len(), 1);
    assert_eq!(joined.members[0].membership, RoomMemberMembership::Joined);
    let mentions = session
        .joined_member_snapshot_no_sync(room_id.as_str())
        .await
        .unwrap();
    assert_eq!(mentions.members.len(), 1);
    assert_eq!(mentions.members[0].user_id, member_id.as_str());
}
