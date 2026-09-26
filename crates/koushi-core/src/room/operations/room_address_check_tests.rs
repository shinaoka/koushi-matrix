//! Advisory create-room address checks through the Room actor (#1006).

use std::{sync::Arc, time::Duration};

use koushi_protocol::command::RoomCommand;
use koushi_sdk::MatrixClientSession;
use koushi_state::{AppAction, RoomAddressAvailability, SessionInfo};
use matrix_sdk::test_utils::mocks::MatrixMockServer;
use tokio::sync::{broadcast, mpsc};
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{method, path_regex},
};

use crate::room::actor::{RoomActor, RoomActorHandle, RoomMessage, make_request_id};

async fn actor(
    server: &MatrixMockServer,
) -> (RoomActorHandle, mpsc::Receiver<Vec<AppAction>>, String) {
    let client = server.client_builder().build().await;
    let server_name = client.user_id().unwrap().server_name().to_string();
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client.clone(),
        SessionInfo {
            homeserver: server.uri(),
            user_id: client.user_id().unwrap().to_string(),
            device_id: client.device_id().unwrap().to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    ));
    let (action_tx, action_rx) = mpsc::channel(64);
    let (event_tx, _event_rx) = broadcast::channel(64);
    let handle = RoomActor::spawn(
        action_tx,
        event_tx,
        crate::SlidingSyncDiagnostics::default(),
    );
    assert!(
        handle
            .send(RoomMessage::SessionEstablished { session })
            .await
    );
    (handle, action_rx, server_name)
}

async fn next_actions(action_rx: &mut mpsc::Receiver<Vec<AppAction>>) -> Vec<AppAction> {
    tokio::time::timeout(Duration::from_secs(10), action_rx.recv())
        .await
        .expect("reducer actions")
        .expect("action channel open")
}

async fn check(handle: &RoomActorHandle, sequence: u64, localpart: &str) {
    assert!(
        handle
            .send(RoomMessage::Command(
                RoomCommand::CheckRoomAddressAvailability {
                    request_id: make_request_id(sequence),
                    alias_localpart: localpart.to_owned(),
                }
            ))
            .await
    );
}

#[tokio::test]
async fn an_address_in_use_settles_with_an_unchecked_alternative() {
    let server = MatrixMockServer::new().await;
    server
        .mock_room_directory_resolve_alias()
        .ok("!taken:example.invalid", Vec::new())
        .mount()
        .await;
    let (handle, mut action_rx, server_name) = actor(&server).await;
    check(&handle, 5, "papers").await;

    let alias = format!("#papers:{server_name}");
    assert_eq!(
        next_actions(&mut action_rx).await,
        vec![AppAction::RoomAddressAvailabilityRequested {
            request_id: 5,
            full_alias: alias.clone(),
        }]
    );
    match next_actions(&mut action_rx).await.as_slice() {
        [
            AppAction::RoomAddressAvailabilitySettled {
                request_id: 5,
                full_alias,
                availability: RoomAddressAvailability::InUse,
                suggestion: Some(suggestion),
            },
        ] => {
            assert_eq!(full_alias, &alias);
            assert_eq!(suggestion.localpart, "papers-2");
            assert_eq!(suggestion.full_alias, format!("#papers-2:{server_name}"));
        }
        other => panic!("unexpected settlement: {other:?}"),
    }
}

#[tokio::test]
async fn an_invalid_draft_is_cleared_without_a_lookup() {
    let server = MatrixMockServer::new().await;
    server
        .mock_room_directory_resolve_alias()
        .not_found()
        .expect(0)
        .mount()
        .await;
    let (handle, mut action_rx, _) = actor(&server).await;
    check(&handle, 6, "has space").await;
    assert_eq!(
        next_actions(&mut action_rx).await,
        vec![AppAction::RoomAddressAvailabilityCleared]
    );
}

#[tokio::test]
async fn a_newer_check_cancels_the_slow_lookup_it_replaces() {
    let server = MatrixMockServer::new().await;
    Mock::given(method("GET"))
        .and(path_regex(r"/_matrix/client/v3/directory/room/%23slow.*"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(
                    serde_json::json!({ "room_id": "!r:example.invalid", "servers": [] }),
                )
                .set_delay(Duration::from_secs(60)),
        )
        .mount(server.server())
        .await;
    server
        .mock_room_directory_resolve_alias()
        .not_found()
        .mount()
        .await;
    let (handle, mut action_rx, _) = actor(&server).await;
    check(&handle, 7, "slow").await;
    assert!(matches!(
        next_actions(&mut action_rx).await.as_slice(),
        [AppAction::RoomAddressAvailabilityRequested { request_id: 7, .. }]
    ));
    check(&handle, 8, "fast").await;
    assert!(matches!(
        next_actions(&mut action_rx).await.as_slice(),
        [AppAction::RoomAddressAvailabilityRequested { request_id: 8, .. }]
    ));
    // The replaced lookup never settles; the newer one does.
    assert!(matches!(
        next_actions(&mut action_rx).await.as_slice(),
        [AppAction::RoomAddressAvailabilitySettled {
            request_id: 8,
            availability: RoomAddressAvailability::Available,
            ..
        }]
    ));

    assert!(
        handle
            .send(RoomMessage::Command(
                RoomCommand::ClearRoomAddressAvailability {
                    request_id: make_request_id(9),
                }
            ))
            .await
    );
    assert_eq!(
        next_actions(&mut action_rx).await,
        vec![AppAction::RoomAddressAvailabilityCleared]
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(200), action_rx.recv())
            .await
            .is_err(),
        "a cancelled lookup must not settle"
    );
}

/// A lookup the server never answers settles `unknown` at the 8-second bound,
/// never `available`. Time is paused, so the bound elapses without waiting.
#[tokio::test(start_paused = true)]
async fn an_unanswered_lookup_settles_unknown_at_the_bound() {
    let server = MatrixMockServer::new().await;
    Mock::given(method("GET"))
        .and(path_regex(r"/_matrix/client/v3/directory/room/.*"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(
                    serde_json::json!({ "room_id": "!r:example.invalid", "servers": [] }),
                )
                .set_delay(Duration::from_secs(3600)),
        )
        .mount(server.server())
        .await;
    let (handle, mut action_rx, _) = actor(&server).await;
    check(&handle, 11, "silent").await;
    assert!(matches!(
        next_actions(&mut action_rx).await.as_slice(),
        [AppAction::RoomAddressAvailabilityRequested { request_id: 11, .. }]
    ));
    let started = tokio::time::Instant::now();
    assert!(matches!(
        next_actions(&mut action_rx).await.as_slice(),
        [AppAction::RoomAddressAvailabilitySettled {
            request_id: 11,
            availability: RoomAddressAvailability::Unknown,
            suggestion: None,
            ..
        }]
    ));
    assert!(started.elapsed() >= Duration::from_secs(8));
}
