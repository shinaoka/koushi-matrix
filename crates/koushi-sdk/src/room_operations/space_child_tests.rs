use super::{
    MatrixClientSession, MatrixRoomOperationError, MatrixRoomOperationFailureKind,
    MatrixSpaceChildLinkOutcome, MatrixSpaceParentLinkOutcome, set_space_child,
};
use matrix_sdk::{
    Client,
    ruma::{
        Int, OwnedRoomId, OwnedServerName, OwnedUserId, RoomId, RoomVersionId, UserId, event_id,
        events::space::{child::SpaceChildEventContent, parent::SpaceParentEventContent},
        owned_user_id,
    },
    test_utils::mocks::MatrixMockServer,
};
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};
use std::collections::BTreeMap;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{body_json, method, path_regex},
};

async fn expect_state_put(
    server: &MatrixMockServer,
    event_type: &str,
    body: serde_json::Value,
    times: u64,
) {
    Mock::given(method("PUT"))
        .and(path_regex(format!(
            r"^/_matrix/client/v3/rooms/.*/state/{event_type}/"
        )))
        .and(body_json(body))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "event_id": "$state" })),
        )
        .expect(times)
        .mount(server.server())
        .await;
}

/// A room version 12 room ID: an opaque hash with no `:server` component.
const DOMAINLESS_ROOM_ID: &str = "!31hneApxJ_1o-63DmFrpeqnkFfWppnzWso1JvH3ogLM";

fn session(server: &MatrixMockServer, client: Client) -> MatrixClientSession {
    MatrixClientSession {
        info: koushi_state::SessionInfo {
            homeserver: server.server().uri(),
            user_id: client.user_id().unwrap().to_string(),
            device_id: client.device_id().unwrap().to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
        client,
        diagnostic_counters: koushi_diagnostics::DiagnosticCounterContext::registered(),
    }
}

struct RoomFixture<'a> {
    room_id: OwnedRoomId,
    is_space: bool,
    own_power_level: i64,
    /// Joined members besides the session user, with their power levels.
    other_members: Vec<(OwnedUserId, i64)>,
    child_event: Option<(&'a RoomId, Vec<OwnedServerName>)>,
    parent_event: Option<(&'a RoomId, Vec<OwnedServerName>)>,
}

async fn sync_fixture(server: &MatrixMockServer, client: &Client, fixture: RoomFixture<'_>) {
    let own = client.user_id().unwrap().to_owned();
    let factory = EventFactory::new().room(&fixture.room_id);
    // Room version 11 keeps authorization in `users` power levels so a test
    // can deny a state event; routing is independent of the room version.
    let create = factory.create(&own, RoomVersionId::V11);
    let create = if fixture.is_space {
        create.with_space_type()
    } else {
        create
    };
    let mut builder = JoinedRoomBuilder::new(&fixture.room_id)
        .add_state_event(create.sender(&own))
        .add_state_event(factory.member(&own));
    let mut power_levels: BTreeMap<OwnedUserId, Int> =
        BTreeMap::from([(own.clone(), Int::from(fixture.own_power_level as i32))]);
    for (user, level) in &fixture.other_members {
        builder = builder.add_state_event(factory.member(user));
        power_levels.insert(user.clone(), Int::from(*level as i32));
    }
    builder = builder.add_state_event(
        factory
            .power_levels(&mut power_levels)
            .state_key("")
            .sender(&own),
    );
    if let Some((child, via)) = fixture.child_event {
        builder = builder.add_state_event(
            factory
                .event(SpaceChildEventContent::new(via))
                .state_key(child.as_str())
                .sender(&own),
        );
    }
    if let Some((parent, via)) = fixture.parent_event {
        builder = builder.add_state_event(
            factory
                .event(SpaceParentEventContent::new(via))
                .state_key(parent.as_str())
                .sender(&own),
        );
    }
    server.sync_room(client, builder).await;
}

fn plain_room(room_id: &str) -> RoomFixture<'static> {
    RoomFixture {
        room_id: RoomId::parse(room_id).unwrap(),
        is_space: false,
        own_power_level: 100,
        other_members: Vec::new(),
        child_event: None,
        parent_event: None,
    }
}

fn space(room_id: &str) -> RoomFixture<'static> {
    RoomFixture {
        is_space: true,
        ..plain_room(room_id)
    }
}

#[tokio::test]
async fn domainless_child_without_synced_members_is_routed_through_own_server() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let own_server = client.user_id().unwrap().server_name().to_string();
    server.mock_room_state_encryption().plain().mount().await;
    sync_fixture(&server, &client, space("!space:example.org")).await;
    // A freshly created room: joined, but no member list loaded yet.
    let child_id = RoomId::parse(DOMAINLESS_ROOM_ID).unwrap();
    server
        .sync_room(&client, JoinedRoomBuilder::new(&child_id))
        .await;
    assert!(child_id.server_name().is_none());

    expect_state_put(
        &server,
        "m.space.child",
        serde_json::json!({ "via": [own_server] }),
        1,
    )
    .await;
    server
        .mock_set_space_parent()
        .ok(event_id!("$parent").to_owned())
        .mount()
        .await;

    let outcome = set_space_child(
        &session(&server, client),
        "!space:example.org",
        DOMAINLESS_ROOM_ID,
    )
    .await
    .unwrap();

    assert!(outcome.child_written);
}

#[tokio::test]
async fn child_route_comes_from_joined_members_and_the_inverse_parent_is_written() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let own_server = client.user_id().unwrap().server_name().to_string();
    server.mock_room_state_encryption().plain().mount().await;
    sync_fixture(&server, &client, space("!space:example.org")).await;
    let admin: OwnedUserId = owned_user_id!("@admin:remote.example");
    sync_fixture(
        &server,
        &client,
        RoomFixture {
            own_power_level: 50,
            other_members: vec![(admin, 100)],
            ..plain_room(DOMAINLESS_ROOM_ID)
        },
    )
    .await;

    // The highest-power member's server leads, then servers by population.
    expect_state_put(
        &server,
        "m.space.child",
        serde_json::json!({ "via": ["remote.example", own_server] }),
        1,
    )
    .await;
    expect_state_put(
        &server,
        "m.space.parent",
        serde_json::json!({ "via": [own_server] }),
        1,
    )
    .await;

    let outcome = set_space_child(
        &session(&server, client),
        "!space:example.org",
        DOMAINLESS_ROOM_ID,
    )
    .await
    .unwrap();

    assert_eq!(
        outcome,
        MatrixSpaceChildLinkOutcome {
            child_written: true,
            parent: MatrixSpaceParentLinkOutcome::Written,
        }
    );
}

#[tokio::test]
async fn missing_space_child_permission_fails_before_any_request() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    server.mock_room_state_encryption().plain().mount().await;
    sync_fixture(
        &server,
        &client,
        RoomFixture {
            own_power_level: 0,
            ..space("!space:example.org")
        },
    )
    .await;
    sync_fixture(&server, &client, plain_room(DOMAINLESS_ROOM_ID)).await;
    server
        .mock_set_space_child()
        .ok(event_id!("$child").to_owned())
        .expect(0)
        .mount()
        .await;

    let error = set_space_child(
        &session(&server, client),
        "!space:example.org",
        DOMAINLESS_ROOM_ID,
    )
    .await
    .unwrap_err();

    assert_eq!(
        error,
        MatrixRoomOperationError::Sdk(MatrixRoomOperationFailureKind::Forbidden)
    );
}

#[tokio::test]
async fn a_parent_only_relationship_gains_the_child_and_keeps_the_existing_parent() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let own_server: OwnedServerName = client.user_id().unwrap().server_name().to_owned();
    server.mock_room_state_encryption().plain().mount().await;
    let space_id = RoomId::parse("!space:example.org").unwrap();
    sync_fixture(&server, &client, space(space_id.as_str())).await;
    sync_fixture(
        &server,
        &client,
        RoomFixture {
            parent_event: Some((&space_id, vec![own_server.clone()])),
            ..plain_room(DOMAINLESS_ROOM_ID)
        },
    )
    .await;
    server
        .mock_set_space_child()
        .ok(event_id!("$child").to_owned())
        .expect(1)
        .mount()
        .await;
    server
        .mock_set_space_parent()
        .ok(event_id!("$parent").to_owned())
        .expect(0)
        .mount()
        .await;

    let outcome = set_space_child(
        &session(&server, client),
        space_id.as_str(),
        DOMAINLESS_ROOM_ID,
    )
    .await
    .unwrap();

    assert_eq!(
        outcome,
        MatrixSpaceChildLinkOutcome {
            child_written: true,
            parent: MatrixSpaceParentLinkOutcome::AlreadyPresent,
        }
    );
}

#[tokio::test]
async fn an_already_routed_child_is_not_written_again() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let own_server: OwnedServerName = client.user_id().unwrap().server_name().to_owned();
    server.mock_room_state_encryption().plain().mount().await;
    let space_id = RoomId::parse("!space:example.org").unwrap();
    let child_id = RoomId::parse(DOMAINLESS_ROOM_ID).unwrap();
    sync_fixture(
        &server,
        &client,
        RoomFixture {
            child_event: Some((&child_id, vec![own_server.clone()])),
            ..space(space_id.as_str())
        },
    )
    .await;
    sync_fixture(
        &server,
        &client,
        RoomFixture {
            parent_event: Some((&space_id, vec![own_server])),
            ..plain_room(DOMAINLESS_ROOM_ID)
        },
    )
    .await;
    server
        .mock_set_space_child()
        .ok(event_id!("$child").to_owned())
        .expect(0)
        .mount()
        .await;

    let outcome = set_space_child(
        &session(&server, client),
        space_id.as_str(),
        DOMAINLESS_ROOM_ID,
    )
    .await
    .unwrap();

    assert!(!outcome.child_written);
    assert_eq!(outcome.parent, MatrixSpaceParentLinkOutcome::AlreadyPresent);
}

#[test]
fn an_empty_route_falls_back_to_the_session_server_only() {
    let own = UserId::parse("@member:home.example").unwrap();
    assert_eq!(
        super::space_child::route_or_own_server(Vec::new(), Some(own.server_name())),
        vec![own.server_name().to_owned()]
    );
    let remote: OwnedServerName = "remote.example".try_into().unwrap();
    assert_eq!(
        super::space_child::route_or_own_server(vec![remote.clone()], Some(own.server_name())),
        vec![remote]
    );
}

/// A child event with an empty `via` (how a child is removed) cannot route:
/// it is not projected as a child, and adding the room writes a routed one.
#[tokio::test]
async fn an_empty_via_child_is_not_a_child_and_is_rewritten() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let own_server = client.user_id().unwrap().server_name().to_string();
    server.mock_room_state_encryption().plain().mount().await;
    let space_id = RoomId::parse("!space:example.org").unwrap();
    let child_id = RoomId::parse(DOMAINLESS_ROOM_ID).unwrap();
    sync_fixture(
        &server,
        &client,
        RoomFixture {
            child_event: Some((&child_id, Vec::new())),
            ..space(space_id.as_str())
        },
    )
    .await;
    sync_fixture(&server, &client, plain_room(DOMAINLESS_ROOM_ID)).await;

    let space_room = client.get_room(&space_id).unwrap();
    assert!(
        crate::room_projection::matrix_space_child_room_ids(&space_room)
            .await
            .is_empty()
    );

    expect_state_put(
        &server,
        "m.space.child",
        serde_json::json!({ "via": [own_server] }),
        1,
    )
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
    let outcome = set_space_child(
        &session(&server, client),
        space_id.as_str(),
        DOMAINLESS_ROOM_ID,
    )
    .await
    .unwrap();
    assert!(outcome.child_written);
}
