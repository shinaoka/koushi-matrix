use std::{collections::BTreeSet, sync::Arc, time::Duration};

use koushi_protocol::{AccountKey, RequestId, RoomCommand, RuntimeConnectionId};
use koushi_state::{AppAction, MentionCandidatesCompleteness, MentionSurface};
use matrix_sdk::{
    ruma::events::room::member::MembershipState, test_utils::mocks::MatrixMockServer,
};
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};
use tokio::sync::{broadcast, mpsc};

use crate::room::{RoomActor, RoomMessage};

#[tokio::test]
async fn demanded_mention_candidates_include_member_after_invitation_is_accepted() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let session = Arc::new(koushi_sdk::MatrixClientSession::from_client_for_testing(
        client.clone(),
        koushi_state::SessionInfo {
            homeserver: server.server().uri(),
            user_id: client.user_id().unwrap().to_string(),
            device_id: client.device_id().unwrap().to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    ));
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
                .into_raw(),
        ])
        .mock_once()
        .mount()
        .await;
    session
        .refresh_joined_member_snapshot(room_id.as_str())
        .await
        .unwrap();

    let (action_tx, mut action_rx) = mpsc::channel(16);
    let (event_tx, _) = broadcast::channel(16);
    let handle = RoomActor::spawn(
        action_tx,
        event_tx,
        crate::SlidingSyncDiagnostics::default(),
    );
    assert!(
        handle
            .send(RoomMessage::SessionEstablished {
                session: session.clone()
            })
            .await
    );
    assert!(
        handle
            .send(RoomMessage::Command(RoomCommand::QueryMentionCandidates {
                request_id: RequestId {
                    connection_id: RuntimeConnectionId(1),
                    sequence: 1
                },
                account_key: AccountKey(session.info.user_id.clone()),
                room_id: room_id.to_string(),
                surface: MentionSurface::Main,
                query: String::new(),
            }))
            .await
    );
    assert!(next_complete_candidates(&mut action_rx).await.is_empty());

    server
        .mock_sync()
        .ok_and_run(&client, |builder| {
            builder.add_joined_room(
                JoinedRoomBuilder::new(room_id).add_state_event(
                    EventFactory::new()
                        .room(room_id)
                        .member(member_id)
                        .membership(MembershipState::Join)
                        .into_raw_sync_state(),
                ),
            );
        })
        .await;
    assert!(
        handle
            .send(RoomMessage::MentionMembershipChanged {
                room_ids: Some(BTreeSet::from([room_id.to_string()])),
            })
            .await
    );
    let candidates = next_complete_candidates(&mut action_rx).await;
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].user_id, member_id.as_str());
    assert!(handle.send(RoomMessage::Shutdown).await);
    handle.join().await;
}

async fn next_complete_candidates(
    action_rx: &mut mpsc::Receiver<Vec<AppAction>>,
) -> Vec<koushi_state::MentionCandidate> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            for action in action_rx.recv().await.expect("actor action") {
                if let AppAction::MentionCandidatesProjected {
                    completeness: MentionCandidatesCompleteness::Complete,
                    candidates,
                    ..
                } = action
                {
                    return candidates;
                }
            }
        }
    })
    .await
    .expect("complete mention projection")
}
