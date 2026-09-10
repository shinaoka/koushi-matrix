use matrix_sdk::{
    ruma::{RoomVersionId, events::room::member::MembershipState},
    test_utils::mocks::MatrixMockServer,
};
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};

use super::{MatrixClientSession, SessionInfo};

async fn session_for(server: &MatrixMockServer) -> MatrixClientSession {
    let client = server.client_builder().build().await;
    let info = SessionInfo {
        homeserver: server.server().uri(),
        user_id: client
            .user_id()
            .expect("mock client has a user id")
            .to_string(),
        device_id: client
            .device_id()
            .expect("mock client has a device id")
            .to_string(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    };
    MatrixClientSession {
        client,
        info,
        diagnostic_counters: koushi_diagnostics::DiagnosticCounterContext::registered(),
    }
}

#[tokio::test]
async fn projection_uses_local_join_and_invite_filters_and_unions_child_joins() {
    let server = MatrixMockServer::new().await;
    let session = session_for(&server).await;
    let client = session.client();
    let own_user = client.user_id().expect("mock user");
    let space_id = matrix_sdk::ruma::room_id!("!space-facts:example.org");
    let child_a = matrix_sdk::ruma::room_id!("!child-a:example.org");
    let child_b = matrix_sdk::ruma::room_id!("!child-b:example.org");
    let joined = matrix_sdk::ruma::user_id!("@joined:example.org");
    let invited = matrix_sdk::ruma::user_id!("@invited:example.org");
    let both = matrix_sdk::ruma::user_id!("@both:example.org");
    let child_only = matrix_sdk::ruma::user_id!("@child-only:example.org");
    let second_only = matrix_sdk::ruma::user_id!("@second-only:example.org");
    let left = matrix_sdk::ruma::user_id!("@left:example.org");
    let banned = matrix_sdk::ruma::user_id!("@banned:example.org");
    let knocked = matrix_sdk::ruma::user_id!("@knocked:example.org");

    server
        .mock_sync()
        .ok_and_run(&session.client(), |builder| {
            builder.add_joined_room(
                JoinedRoomBuilder::new(space_id)
                    .add_state_event(
                        EventFactory::new()
                            .room(space_id)
                            .create(own_user, RoomVersionId::V1)
                            .with_space_type()
                            .into_raw_sync_state(),
                    )
                    .add_state_event(
                        EventFactory::new()
                            .sender(own_user)
                            .space_child(space_id.to_owned(), child_a.to_owned())
                            .into_raw_sync_state(),
                    )
                    .add_state_event(
                        EventFactory::new()
                            .sender(own_user)
                            .space_child(space_id.to_owned(), child_b.to_owned())
                            .into_raw_sync_state(),
                    )
                    .add_state_event(
                        EventFactory::new()
                            .room(space_id)
                            .member(joined)
                            .display_name("Joined")
                            .into_raw_sync_state(),
                    )
                    .add_state_event(
                        EventFactory::new()
                            .room(space_id)
                            .member(invited)
                            .membership(MembershipState::Invite)
                            .display_name("Invited")
                            .into_raw_sync_state(),
                    )
                    .add_state_event(
                        EventFactory::new()
                            .room(space_id)
                            .member(both)
                            .display_name("Both")
                            .into_raw_sync_state(),
                    ),
            );
            builder.add_joined_room(
                JoinedRoomBuilder::new(child_a)
                    .add_state_event(
                        EventFactory::new()
                            .room(child_a)
                            .member(child_only)
                            .display_name("Child only")
                            .into_raw_sync_state(),
                    )
                    .add_state_event(
                        EventFactory::new()
                            .room(child_a)
                            .member(both)
                            .display_name("Both in child")
                            .into_raw_sync_state(),
                    )
                    .add_state_event(
                        EventFactory::new()
                            .room(child_a)
                            .member(left)
                            .membership(MembershipState::Leave)
                            .into_raw_sync_state(),
                    )
                    .add_state_event(
                        EventFactory::new()
                            .room(child_a)
                            .member(banned)
                            .membership(MembershipState::Ban)
                            .into_raw_sync_state(),
                    )
                    .add_state_event(
                        EventFactory::new()
                            .room(child_a)
                            .member(knocked)
                            .membership(MembershipState::Knock)
                            .into_raw_sync_state(),
                    ),
            );
            builder.add_joined_room(
                JoinedRoomBuilder::new(child_b)
                    .add_state_event(
                        EventFactory::new()
                            .room(child_b)
                            .member(child_only)
                            .display_name("Child only again")
                            .into_raw_sync_state(),
                    )
                    .add_state_event(
                        EventFactory::new()
                            .room(child_b)
                            .member(second_only)
                            .display_name("Second only")
                            .into_raw_sync_state(),
                    )
                    .add_state_event(
                        EventFactory::new()
                            .room(child_b)
                            .member(invited)
                            .membership(MembershipState::Invite)
                            .into_raw_sync_state(),
                    ),
            );
        })
        .await;

    let projection = super::matrix_space_members_projection(&session, space_id.as_str())
        .await
        .expect("local Space projection");

    let ids = |entries: &[super::MatrixSpaceMemberEntry]| {
        entries
            .iter()
            .map(|entry| entry.user_id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ids(&projection.space_joined),
        vec![both.to_string(), joined.to_string()]
    );
    assert_eq!(ids(&projection.space_invited), vec![invited.to_string()]);
    assert_eq!(
        ids(&projection.child_room_only),
        vec![child_only.to_string(), second_only.to_string()]
    );
    assert!(
        projection
            .child_room_profiles
            .iter()
            .any(|entry| entry.user_id == both.to_string())
    );
    assert_eq!(
        projection.child_room_only[0].child_room_ids,
        vec![child_a.to_string(), child_b.to_string()]
    );
    assert_eq!(
        projection.child_room_ids,
        vec![child_a.to_string(), child_b.to_string()]
    );
    assert_eq!(projection.child_room_count, 2);
    assert_eq!(projection.incomplete_child_room_count, 2);
    assert_eq!(projection.space_joined_input_count, 2);
    assert_eq!(projection.space_invited_input_count, 1);
    assert_eq!(projection.child_join_input_count, 4);
    assert_eq!(projection.child_join_union_count, 3);
    assert_eq!(projection.duplicate_child_membership_count, 1);
}

#[test]
fn space_member_projection_debug_redacts_identifiers_and_profiles() {
    let entry = super::MatrixSpaceMemberEntry {
        user_id: "@private:example.invalid".to_owned(),
        display_name: Some("Private name".to_owned()),
        avatar_url: Some("mxc://example.invalid/avatar".to_owned()),
        power_level: Some(100),
        role: super::MatrixRoomMemberRole::Administrator,
        child_room_ids: vec!["!child:example.invalid".to_owned()],
        role_options: Vec::new(),
    };
    let projection = super::MatrixSpaceMembersProjection {
        space_id: "!space:example.invalid".to_owned(),
        child_room_ids: vec!["!child:example.invalid".to_owned()],
        space_joined: vec![entry.clone()],
        space_invited: Vec::new(),
        child_room_only: Vec::new(),
        child_room_profiles: vec![entry.clone()],
        space_joined_input_count: 1,
        space_invited_input_count: 0,
        child_join_input_count: 0,
        child_join_union_count: 0,
        duplicate_child_membership_count: 0,
        child_room_count: 1,
        complete_child_room_count: 1,
        incomplete_child_room_count: 0,
        power_levels_revision: None,
        can_edit_roles: false,
    };

    for debug in [format!("{entry:?}"), format!("{projection:?}")] {
        assert!(debug.contains("space_joined_count") || debug.contains("child_room_count"));
        assert!(!debug.contains("@private:example.invalid"));
        assert!(!debug.contains("Private name"));
        assert!(!debug.contains("mxc://example.invalid/avatar"));
        assert!(!debug.contains("!child:example.invalid"));
    }
}

#[test]
fn local_member_profile_debug_redacts_identifiers_names_and_mxc_uris() {
    let summary = super::MatrixRoomMemberSummary {
        membership: koushi_state::RoomMemberMembership::Joined,
        user_id: "@private:example.invalid".to_owned(),
        display_name: Some("Private member".to_owned()),
        avatar_url: Some("mxc://example.invalid/member-avatar".to_owned()),
        power_level: Some(50),
        role: super::MatrixRoomMemberRole::Moderator,
        role_options: Vec::new(),
        user_trust: Some(super::MatrixUserTrustState::Verified),
    };
    let snapshot = super::MatrixJoinedMemberSnapshot {
        members: vec![summary],
        complete: true,
        room_mention_allowed: Some(true),
    };
    let profile = super::MatrixUserProfile {
        user_id: "@private:example.invalid".to_owned(),
        display_name: Some("Private member".to_owned()),
        avatar_mxc_uri: Some("mxc://example.invalid/member-avatar".to_owned()),
    };

    for debug in [format!("{snapshot:?}"), format!("{profile:?}")] {
        assert!(!debug.contains("@private:example.invalid"), "{debug}");
        assert!(!debug.contains("Private member"), "{debug}");
        assert!(
            !debug.contains("mxc://example.invalid/member-avatar"),
            "{debug}"
        );
    }
}
