//! Space child linking through the Room actor (#1007): routing is derived by
//! the SDK, and every linking attempt settles visibly.

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use koushi_protocol::command::{
    CreateRoomOptions, CreateRoomParentSpace, CreateRoomVisibility, RoomCommand,
};
use koushi_protocol::event::{CoreEvent, RoomEvent};
use koushi_protocol::failure::{CoreFailure, RoomFailureKind};
use koushi_sdk::MatrixClientSession;
use koushi_state::{
    AppAction, BasicOperationRequest, OperationFailureKind, SessionInfo, SpaceChildLinkOutcome,
};
use matrix_sdk::{
    Client,
    ruma::{Int, OwnedRoomId, OwnedUserId, RoomId, RoomVersionId},
    test_utils::mocks::MatrixMockServer,
};
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};
use tokio::sync::{broadcast, mpsc};
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{body_json, method, path_regex},
};

use crate::room::actor::{RoomActor, RoomActorHandle, RoomMessage, make_request_id};

const SPACE: &str = "!space:example.test";
/// Room version 12 room IDs carry no server name.
const DOMAINLESS: &str = "!31hneApxJ_1o-63DmFrpeqnkFfWppnzWso1JvH3ogLM";

async fn sync_room(
    server: &MatrixMockServer,
    client: &Client,
    room_id: &str,
    power: i32,
    space: bool,
) {
    let own = client.user_id().unwrap().to_owned();
    let room_id: OwnedRoomId = RoomId::parse(room_id).unwrap();
    let factory = EventFactory::new().room(&room_id);
    let create = factory.create(&own, RoomVersionId::V11);
    let create = if space {
        create.with_space_type()
    } else {
        create
    };
    let mut levels: BTreeMap<OwnedUserId, Int> = BTreeMap::from([(own.clone(), Int::from(power))]);
    server
        .sync_room(
            client,
            JoinedRoomBuilder::new(&room_id)
                .add_state_event(create.sender(&own))
                .add_state_event(factory.member(&own))
                .add_state_event(factory.power_levels(&mut levels).state_key("").sender(&own)),
        )
        .await;
}

async fn actor_with_session(
    server: &MatrixMockServer,
    client: Client,
) -> (
    RoomActorHandle,
    mpsc::Receiver<Vec<AppAction>>,
    broadcast::Receiver<CoreEvent>,
) {
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
    let (event_tx, event_rx) = broadcast::channel(64);
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
    (handle, action_rx, event_rx)
}

async fn expect_child_put(server: &MatrixMockServer, via: &str, times: u64) {
    Mock::given(method("PUT"))
        .and(path_regex(
            r"^/_matrix/client/v3/rooms/.*/state/m\.space\.child/",
        ))
        .and(body_json(serde_json::json!({ "via": [via] })))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "event_id": "$c" })),
        )
        .expect(times)
        .mount(server.server())
        .await;
}

/// Collect reducer actions until one matches, bounded in time.
async fn actions_until(
    action_rx: &mut mpsc::Receiver<Vec<AppAction>>,
    done: impl Fn(&AppAction) -> bool,
) -> Vec<AppAction> {
    let mut seen = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(batch) = action_rx.recv().await {
            let finished = batch.iter().any(&done);
            seen.extend(batch);
            if finished {
                return;
            }
        }
    })
    .await
    .expect("expected reducer actions");
    seen
}

fn settlements(actions: &[AppAction]) -> Vec<(u64, String, String, SpaceChildLinkOutcome)> {
    actions
        .iter()
        .filter_map(|action| match action {
            AppAction::SpaceChildLinkSettled {
                request_id,
                space_id,
                child_room_id,
                outcome,
            } => Some((
                *request_id,
                space_id.clone(),
                child_room_id.clone(),
                *outcome,
            )),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn adding_a_domainless_room_writes_a_routed_child_and_settles_linked() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let own_server = client.user_id().unwrap().server_name().to_string();
    server.mock_room_state_encryption().plain().mount().await;
    sync_room(&server, &client, SPACE, 100, true).await;
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(&RoomId::parse(DOMAINLESS).unwrap()),
        )
        .await;
    expect_child_put(&server, &own_server, 1).await;
    Mock::given(method("PUT"))
        .and(path_regex(
            r"^/_matrix/client/v3/rooms/.*/state/m\.space\.parent/",
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "event_id": "$p" })),
        )
        .mount(server.server())
        .await;
    let (handle, mut action_rx, mut event_rx) = actor_with_session(&server, client).await;

    let request_id = make_request_id(21);
    assert!(
        handle
            .send(RoomMessage::Command(RoomCommand::SetSpaceChild {
                request_id,
                space_id: SPACE.to_owned(),
                child_room_id: DOMAINLESS.to_owned(),
            }))
            .await
    );

    let actions = actions_until(&mut action_rx, |action| {
        matches!(
            action,
            AppAction::BasicOperationSucceeded { request_id: 21 }
        )
    })
    .await;
    assert!(actions.iter().any(|action| matches!(
        action,
        AppAction::BasicOperationRequested {
            request_id: 21,
            request: BasicOperationRequest::LinkSpaceChild { .. },
        }
    )));
    assert_eq!(
        settlements(&actions),
        vec![(
            21,
            SPACE.to_owned(),
            DOMAINLESS.to_owned(),
            SpaceChildLinkOutcome::Linked
        )]
    );
    let event = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let CoreEvent::Room(RoomEvent::SpaceChildSet { request_id: id, .. }) =
                event_rx.recv().await.expect("event")
            {
                return id;
            }
        }
    })
    .await
    .expect("SpaceChildSet");
    assert_eq!(event, request_id);
}

#[tokio::test]
async fn a_denied_space_settles_failed_forbidden_without_a_request() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let own_server = client.user_id().unwrap().server_name().to_string();
    server.mock_room_state_encryption().plain().mount().await;
    sync_room(&server, &client, SPACE, 0, true).await;
    sync_room(&server, &client, DOMAINLESS, 100, false).await;
    expect_child_put(&server, &own_server, 0).await;
    let (handle, mut action_rx, mut event_rx) = actor_with_session(&server, client).await;

    assert!(
        handle
            .send(RoomMessage::Command(RoomCommand::SetSpaceChild {
                request_id: make_request_id(22),
                space_id: SPACE.to_owned(),
                child_room_id: DOMAINLESS.to_owned(),
            }))
            .await
    );

    let actions = actions_until(&mut action_rx, |action| {
        matches!(
            action,
            AppAction::BasicOperationFailed { request_id: 22, .. }
        )
    })
    .await;
    assert_eq!(
        settlements(&actions),
        vec![(
            22,
            SPACE.to_owned(),
            DOMAINLESS.to_owned(),
            SpaceChildLinkOutcome::Failed {
                reason: OperationFailureKind::Forbidden
            }
        )]
    );
    let failure = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let CoreEvent::OperationFailed { failure, .. } =
                event_rx.recv().await.expect("event")
            {
                return failure;
            }
        }
    })
    .await
    .expect("OperationFailed");
    assert_eq!(
        failure,
        CoreFailure::RoomOperationFailed {
            kind: RoomFailureKind::Forbidden
        }
    );
}

async fn create_domainless_room_in_space(child_status: u16) -> (Vec<AppAction>, Vec<CoreEvent>) {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let own_server = client.user_id().unwrap().server_name().to_string();
    server.mock_room_state_encryption().plain().mount().await;
    sync_room(&server, &client, SPACE, 100, true).await;
    let expected_parent_via = own_server.clone();
    Mock::given(method("POST"))
        .and(path_regex(r"^/_matrix/client/v3/createRoom"))
        // The new room's `m.space.parent` is routed by the SDK as well.
        .and(move |request: &wiremock::Request| {
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            body["initial_state"].as_array().is_some_and(|events| {
                events.iter().any(|event| {
                    event["type"] == "m.space.parent"
                        && event["state_key"] == SPACE
                        && event["content"]["via"] == serde_json::json!([expected_parent_via])
                })
            })
        })
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "room_id": DOMAINLESS })),
        )
        .expect(1)
        .mount(server.server())
        .await;
    Mock::given(method("PUT"))
        .and(path_regex(
            r"^/_matrix/client/v3/rooms/.*/state/m\.space\.child/",
        ))
        .and(body_json(serde_json::json!({ "via": [own_server] })))
        .respond_with(
            ResponseTemplate::new(child_status)
                .set_body_json(serde_json::json!({ "event_id": "$c", "errcode": "M_UNKNOWN" })),
        )
        .expect(1)
        .mount(server.server())
        .await;
    Mock::given(method("PUT"))
        .and(path_regex(
            r"^/_matrix/client/v3/rooms/.*/state/m\.space\.parent/",
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "event_id": "$p" })),
        )
        .mount(server.server())
        .await;
    let (handle, mut action_rx, mut event_rx) = actor_with_session(&server, client).await;

    assert!(
        handle
            .send(RoomMessage::Command(RoomCommand::CreateRoom {
                request_id: make_request_id(31),
                options: CreateRoomOptions {
                    name: "papers".to_owned(),
                    topic: None,
                    alias_localpart: None,
                    encrypted: false,
                    invited_only: false,
                    visibility: CreateRoomVisibility::Private,
                    parent_space: Some(CreateRoomParentSpace {
                        space_id: SPACE.to_owned(),
                    }),
                },
            }))
            .await
    );
    let actions = actions_until(&mut action_rx, |action| {
        matches!(
            action,
            AppAction::BasicOperationSucceeded { request_id: 31 }
        )
    })
    .await;
    let mut events = Vec::new();
    while let Ok(event) = event_rx.try_recv() {
        events.push(event);
    }
    (actions, events)
}

#[tokio::test]
async fn creating_a_domainless_room_in_a_space_links_it_through_sdk_routing() {
    let (actions, events) = create_domainless_room_in_space(200).await;
    assert_eq!(
        settlements(&actions),
        vec![(
            31,
            SPACE.to_owned(),
            DOMAINLESS.to_owned(),
            SpaceChildLinkOutcome::Linked
        )]
    );
    assert!(events.iter().any(|event| matches!(
        event,
        CoreEvent::Room(RoomEvent::RoomCreated { room_id, .. }) if room_id == DOMAINLESS
    )));
}

#[tokio::test]
async fn a_failed_link_after_creation_is_settled_without_failing_the_creation() {
    let (actions, events) = create_domainless_room_in_space(500).await;
    let settled = settlements(&actions);
    assert_eq!(settled.len(), 1);
    assert!(matches!(settled[0].3, SpaceChildLinkOutcome::Failed { .. }));
    // The link settles before the creation succeeds, so a settled creation
    // snapshot always carries the result.
    let settle_at = actions
        .iter()
        .position(|action| matches!(action, AppAction::SpaceChildLinkSettled { .. }))
        .unwrap();
    let success_at = actions
        .iter()
        .position(|action| matches!(action, AppAction::BasicOperationSucceeded { .. }))
        .unwrap();
    assert!(settle_at < success_at);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, CoreEvent::Room(RoomEvent::RoomCreated { .. })))
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, CoreEvent::OperationFailed { .. })),
        "the create request must not be settled as failed"
    );
}
