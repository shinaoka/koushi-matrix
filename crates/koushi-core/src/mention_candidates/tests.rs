use std::{collections::BTreeMap, sync::Arc, time::Duration};

use matrix_sdk::test_utils::mocks::MatrixMockServer;
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};
use tokio::sync::{broadcast, mpsc};

use koushi_state::RoomMentionPermission;

use super::{MentionMemberInput, project_candidates};
use crate::{
    RequestId, RuntimeConnectionId,
    room::{RoomActor, RoomMessage},
};
use koushi_protocol::{AccountKey, RoomCommand};

fn member(
    user_id: &str,
    room_display_name: Option<&str>,
    profile_display_name: Option<&str>,
    local_alias: Option<&str>,
) -> MentionMemberInput {
    MentionMemberInput {
        user_id: user_id.to_owned(),
        room_display_name: room_display_name.map(str::to_owned),
        profile_display_name: profile_display_name.map(str::to_owned),
        local_alias: local_alias.map(str::to_owned),
        avatar_mxc_uri: None,
    }
}

#[test]
fn exact_alias_then_prefix_then_substring_order_is_deterministic() {
    let projection = project_candidates(
        "ali",
        vec![
            member("@substring:test", Some("Malika"), None, None),
            member("@prefix:test", Some("Alice"), None, None),
            member("@exact:test", Some("Elsewhere"), None, Some("Ali")),
            member("@prefix-b:test", Some("Alina"), None, None),
        ],
        RoomMentionPermission::Denied,
    );

    let ids: Vec<_> = projection
        .candidates
        .iter()
        .map(|candidate| candidate.user_id.as_str())
        .collect();
    assert_eq!(
        ids,
        [
            "@exact:test",
            "@prefix:test",
            "@prefix-b:test",
            "@substring:test"
        ]
    );
}

#[test]
fn full_mxid_localpart_name_tokens_and_cjk_substrings_match() {
    let members = vec![
        member("@mxid-match:example.org", None, None, None),
        member("@hiroshi:example.org", Some("Hiroshi Shinaoka"), None, None),
        member("@jp:example.org", Some("品岡浩司"), None, None),
    ];

    for (query, expected) in [
        ("@mxid-match:example.org", "@mxid-match:example.org"),
        ("mxid-match", "@mxid-match:example.org"),
        ("shina", "@hiroshi:example.org"),
        ("hiro", "@hiroshi:example.org"),
        ("品岡", "@jp:example.org"),
        ("浩司", "@jp:example.org"),
    ] {
        let projection = project_candidates(query, members.clone(), RoomMentionPermission::Denied);
        assert_eq!(projection.candidates.len(), 1, "query={query}");
        assert_eq!(projection.candidates[0].user_id, expected, "query={query}");
    }
}

#[test]
fn alias_changes_the_label_and_terms_without_changing_eligibility() {
    let projection = project_candidates(
        "personal",
        vec![member(
            "@joined:test",
            Some("Room Name"),
            Some("Profile Name"),
            Some("Personal Alias"),
        )],
        RoomMentionPermission::Denied,
    );
    assert_eq!(projection.candidates.len(), 1);
    assert_eq!(
        projection.candidates[0].display_label.as_deref(),
        Some("Personal Alias")
    );
    assert_eq!(
        projection.candidates[0].original_display_label.as_deref(),
        Some("Room Name")
    );

    let absent = project_candidates("global-only", Vec::new(), RoomMentionPermission::Denied);
    assert!(absent.candidates.is_empty());
}

#[test]
fn missing_friendly_label_remains_none_and_user_id_stays_identity_only() {
    let projection = project_candidates(
        "unknown",
        vec![member("@unknown:test", None, None, None)],
        RoomMentionPermission::Denied,
    );
    let candidate = &projection.candidates[0];
    assert_eq!(candidate.display_label, None);
    assert_eq!(candidate.original_display_label, None);
    assert_eq!(candidate.user_id, "@unknown:test");
}

#[test]
fn room_mention_is_projected_only_when_matched_and_allowed() {
    for (query, permission, expected) in [
        ("ro", RoomMentionPermission::Allowed, true),
        ("room", RoomMentionPermission::Allowed, true),
        ("other", RoomMentionPermission::Allowed, false),
        ("room", RoomMentionPermission::Denied, false),
        ("room", RoomMentionPermission::Unknown, false),
    ] {
        let projection = project_candidates(query, Vec::new(), permission);
        assert_eq!(projection.room_mention_included, expected);
    }
}

async fn session_for(server: &MatrixMockServer) -> Arc<koushi_sdk::MatrixClientSession> {
    let client = server.client_builder().build().await;
    Arc::new(koushi_sdk::MatrixClientSession::from_client_for_testing(
        client.clone(),
        koushi_state::SessionInfo {
            homeserver: server.server().uri(),
            user_id: client.user_id().unwrap().to_string(),
            device_id: client.device_id().unwrap().to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    ))
}

#[tokio::test]
async fn room_actor_publishes_cached_joined_members_before_refresh() {
    let server = MatrixMockServer::new().await;
    let session = session_for(&server).await;
    let room_id = matrix_sdk::ruma::room_id!("!actor-mentions:example.org");
    let joined = matrix_sdk::ruma::user_id!("@joined:example.org");
    server
        .mock_sync()
        .ok_and_run(&session.client(), |builder| {
            builder.add_joined_room(
                JoinedRoomBuilder::new(room_id).add_state_event(
                    EventFactory::new()
                        .room(room_id)
                        .member(joined)
                        .display_name("Joined Person")
                        .into_raw_sync_state(),
                ),
            );
        })
        .await;

    let (action_tx, mut action_rx) = mpsc::channel(8);
    let (event_tx, _) = broadcast::channel(8);
    let handle = RoomActor::spawn(
        action_tx,
        event_tx,
        crate::SlidingSyncDiagnostics::default(),
    );
    assert!(
        handle
            .send(RoomMessage::SessionEstablished {
                session: session.clone(),
            })
            .await
    );
    assert!(
        handle
            .send(RoomMessage::Command(RoomCommand::QueryMentionCandidates {
                request_id: RequestId {
                    connection_id: RuntimeConnectionId(7),
                    sequence: 41,
                },
                account_key: AccountKey(session.info.user_id.clone()),
                room_id: room_id.to_string(),
                surface: koushi_state::MentionSurface::Main,
                query: "joined".to_owned(),
            },))
            .await
    );

    let demanded = tokio::time::timeout(Duration::from_secs(2), action_rx.recv())
        .await
        .expect("demand timeout")
        .expect("demand action");
    assert!(matches!(
        demanded.as_slice(),
        [koushi_state::AppAction::MentionCandidatesDemanded {
            request_id: 41,
            generation: 1,
            ..
        }]
    ));

    let projected = tokio::time::timeout(Duration::from_secs(2), action_rx.recv())
        .await
        .expect("projection timeout")
        .expect("projection action");
    let [
        koushi_state::AppAction::MentionCandidatesProjected {
            completeness,
            candidates,
            ..
        },
    ] = projected.as_slice()
    else {
        panic!("expected mention projection, got {projected:?}");
    };
    assert_eq!(
        *completeness,
        koushi_state::MentionCandidatesCompleteness::Partial
    );
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].user_id, joined.as_str());

    assert!(handle.send(RoomMessage::Shutdown).await);
    handle.join().await;
}

#[tokio::test]
async fn room_actor_settles_complete_cached_members_without_another_request() {
    let server = MatrixMockServer::new().await;
    let session = session_for(&server).await;
    let room_id = matrix_sdk::ruma::room_id!("!complete-mentions:example.org");
    let joined = matrix_sdk::ruma::user_id!("@complete:example.org");
    server
        .mock_sync()
        .ok_and_run(&session.client(), |builder| {
            builder.add_joined_room(JoinedRoomBuilder::new(room_id));
        })
        .await;
    server
        .mock_get_members()
        .ok(vec![
            EventFactory::new()
                .room(room_id)
                .member(joined)
                .display_name("Complete Person")
                .into_raw(),
        ])
        .mock_once()
        .mount()
        .await;
    session
        .refresh_joined_member_snapshot(room_id.as_str())
        .await
        .expect("prime complete member cache");

    let (action_tx, mut action_rx) = mpsc::channel(8);
    let (event_tx, _) = broadcast::channel(8);
    let handle = RoomActor::spawn(
        action_tx,
        event_tx,
        crate::SlidingSyncDiagnostics::default(),
    );
    assert!(
        handle
            .send(RoomMessage::SessionEstablished {
                session: session.clone(),
            })
            .await
    );
    assert!(
        handle
            .send(RoomMessage::Command(RoomCommand::QueryMentionCandidates {
                request_id: RequestId {
                    connection_id: RuntimeConnectionId(7),
                    sequence: 42,
                },
                account_key: AccountKey(session.info.user_id.clone()),
                room_id: room_id.to_string(),
                surface: koushi_state::MentionSurface::Main,
                query: "complete".to_owned(),
            }))
            .await
    );
    let _demanded = action_rx.recv().await.expect("demand action");
    let projected = action_rx.recv().await.expect("projection action");
    assert!(matches!(
        projected.as_slice(),
        [koushi_state::AppAction::MentionCandidatesProjected {
            completeness: koushi_state::MentionCandidatesCompleteness::Complete,
            candidates,
            ..
        }] if candidates.len() == 1 && candidates[0].user_id == joined.as_str()
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(100), action_rx.recv())
            .await
            .is_err(),
        "complete cache must not start a refresh"
    );

    assert!(handle.send(RoomMessage::Shutdown).await);
    handle.join().await;
}

#[tokio::test]
async fn room_actor_applies_local_aliases_to_candidate_matching_and_labels() {
    let server = MatrixMockServer::new().await;
    let session = session_for(&server).await;
    let room_id = matrix_sdk::ruma::room_id!("!aliased-mentions:example.org");
    let joined = matrix_sdk::ruma::user_id!("@joined:example.org");
    server
        .mock_sync()
        .ok_and_run(&session.client(), |builder| {
            builder.add_joined_room(
                JoinedRoomBuilder::new(room_id).add_state_event(
                    EventFactory::new()
                        .room(room_id)
                        .member(joined)
                        .display_name("Room Name")
                        .into_raw_sync_state(),
                ),
            );
        })
        .await;

    let (action_tx, mut action_rx) = mpsc::channel(8);
    let (event_tx, _) = broadcast::channel(8);
    let handle = RoomActor::spawn(
        action_tx,
        event_tx,
        crate::SlidingSyncDiagnostics::default(),
    );
    assert!(
        handle
            .send(RoomMessage::SessionEstablished {
                session: session.clone(),
            })
            .await
    );
    assert!(
        handle
            .send(RoomMessage::LocalUserAliasesUpdated {
                aliases: BTreeMap::from([(joined.to_string(), "Personal Alias".to_owned(),)]),
            })
            .await
    );
    assert!(
        handle
            .send(RoomMessage::Command(RoomCommand::QueryMentionCandidates {
                request_id: RequestId {
                    connection_id: RuntimeConnectionId(7),
                    sequence: 45,
                },
                account_key: AccountKey(session.info.user_id.clone()),
                room_id: room_id.to_string(),
                surface: koushi_state::MentionSurface::Main,
                query: "personal".to_owned(),
            }))
            .await
    );

    let _demanded = action_rx.recv().await.expect("demand action");
    let projected = action_rx.recv().await.expect("projection action");
    assert!(matches!(
        projected.as_slice(),
        [koushi_state::AppAction::MentionCandidatesProjected {
            candidates,
            ..
        }] if candidates.len() == 1
            && candidates[0].user_id == joined.as_str()
            && candidates[0].display_label.as_deref() == Some("Personal Alias")
            && candidates[0].original_display_label.as_deref() == Some("Room Name")
    ));

    assert!(handle.send(RoomMessage::Shutdown).await);
    handle.join().await;
}

#[tokio::test]
async fn room_actor_shares_refresh_and_settles_main_and_thread_demands() {
    let server = MatrixMockServer::new().await;
    let session = session_for(&server).await;
    let room_id = matrix_sdk::ruma::room_id!("!shared-mentions:example.org");
    let joined = matrix_sdk::ruma::user_id!("@shared:example.org");
    server
        .mock_sync()
        .ok_and_run(&session.client(), |builder| {
            builder.add_joined_room(JoinedRoomBuilder::new(room_id));
        })
        .await;
    server
        .mock_get_members()
        .ok(vec![
            EventFactory::new()
                .room(room_id)
                .member(joined)
                .display_name("Shared Person")
                .into_raw(),
        ])
        .mock_once()
        .mount()
        .await;

    let (action_tx, mut action_rx) = mpsc::channel(16);
    let (event_tx, _) = broadcast::channel(8);
    let handle = RoomActor::spawn(
        action_tx,
        event_tx,
        crate::SlidingSyncDiagnostics::default(),
    );
    assert!(
        handle
            .send(RoomMessage::SessionEstablished {
                session: session.clone(),
            })
            .await
    );
    for (sequence, surface) in [
        (43, koushi_state::MentionSurface::Main),
        (44, koushi_state::MentionSurface::Thread),
        (45, koushi_state::MentionSurface::Edit),
    ] {
        assert!(
            handle
                .send(RoomMessage::Command(RoomCommand::QueryMentionCandidates {
                    request_id: RequestId {
                        connection_id: RuntimeConnectionId(7),
                        sequence,
                    },
                    account_key: AccountKey(session.info.user_id.clone()),
                    room_id: room_id.to_string(),
                    surface,
                    query: "shared".to_owned(),
                }))
                .await
        );
    }

    let mut main_complete = false;
    let mut thread_complete = false;
    let mut edit_complete = false;
    for _ in 0..12 {
        let actions = tokio::time::timeout(Duration::from_secs(2), action_rx.recv())
            .await
            .expect("mention lifecycle timeout")
            .expect("mention lifecycle action");
        for action in actions {
            if let koushi_state::AppAction::MentionCandidatesProjected {
                surface,
                completeness: koushi_state::MentionCandidatesCompleteness::Complete,
                candidates,
                ..
            } = action
            {
                assert_eq!(candidates.len(), 1);
                match surface {
                    koushi_state::MentionSurface::Main => main_complete = true,
                    koushi_state::MentionSurface::Thread => thread_complete = true,
                    koushi_state::MentionSurface::Edit => edit_complete = true,
                }
            }
        }
        if main_complete && thread_complete && edit_complete {
            break;
        }
    }
    assert!(main_complete && thread_complete && edit_complete);

    assert!(handle.send(RoomMessage::Shutdown).await);
    handle.join().await;
}
