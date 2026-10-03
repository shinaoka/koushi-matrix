//! #1060: local navigation is never gated by a RoomActor network operation.
//!
//! A real runtime (AppActor, AccountActor, RoomActor) holds an SDK session
//! bound to a mock homeserver. RoomActor starts a Space-children load whose
//! `/hierarchy` response the server delays for two seconds; RoomActor handles
//! that load serially, so it is busy for the whole delay. While it is held,
//! DM, room, and Space selections must each commit within the local
//! navigation budget, and the late response must not restore the older
//! selection. The 250 ms budget is a regression bound, not a latency claim.
//! All identifiers are synthetic.

use std::sync::Arc;
use std::time::{Duration, Instant};

use koushi_core::CoreCommand;
use koushi_core::runtime::{CoreConnection, CoreRuntime};
use koushi_protocol::command::RoomCommand;
use koushi_protocol::event::{CoreEvent, IntentOutcome, RoomEvent};
use koushi_protocol::ids::RequestId;
use koushi_sdk::MatrixClientSession;
use koushi_state::{AppAction, AppState, RoomSummary, SessionState, SpaceSummary};
use matrix_sdk::ruma::{RoomId, RoomVersionId, events::space::child::SpaceChildEventContent};
use matrix_sdk::test_utils::mocks::MatrixMockServer;
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};
use tokio::sync::mpsc;
use wiremock::{Request, Respond, ResponseTemplate};

mod support;
use support::*;

const ROOM_A: &str = "!network-room-a:example.org";
const ROOM_B: &str = "!network-room-b:example.org";
const DM: &str = "!network-dm:example.org";
const SPACE: &str = "!network-space:example.org";
const SPACE_CHILD: &str = "!network-space-child:example.org";
const OTHER_SPACE: &str = "!network-other-space:example.org";
const OTHER_SPACE_ROOM: &str = "!network-other-space-room:example.org";
const HIERARCHY_DELAY: Duration = Duration::from_secs(2);
const SELECTION_BUDGET: Duration = Duration::from_millis(250);

/// Answers `/hierarchy` for `SPACE` after `HIERARCHY_DELAY`, reporting each
/// request the moment the server receives it.
struct DelayedHierarchy {
    started: mpsc::UnboundedSender<()>,
}

impl Respond for DelayedHierarchy {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        let _ = self.started.send(());
        ResponseTemplate::new(200)
            .set_body_json(serde_json::json!({
                "rooms": [
                    {
                        "room_id": SPACE,
                        "room_type": "m.space",
                        "num_joined_members": 2,
                        "world_readable": false,
                        "guest_can_join": false,
                        "join_rule": "invite",
                        "children_state": [],
                    },
                    {
                        "room_id": SPACE_CHILD,
                        "name": "Synthetic child",
                        "num_joined_members": 2,
                        "world_readable": false,
                        "guest_can_join": false,
                        "join_rule": "public",
                        "children_state": [],
                    },
                ],
            }))
            .set_delay(HIERARCHY_DELAY)
    }
}

/// Sync `SPACE` (advertising `SPACE_CHILD`) into the SDK client so RoomActor
/// can build its Space room list.
async fn sync_space(server: &MatrixMockServer, client: &matrix_sdk::Client) {
    let own = client.user_id().expect("mock user id").to_owned();
    let space_id = RoomId::parse(SPACE).expect("space id");
    let factory = EventFactory::new().room(&space_id).sender(&own);
    server
        .sync_room(
            client,
            JoinedRoomBuilder::new(&space_id)
                .add_state_event(factory.create(&own, RoomVersionId::V11).with_space_type())
                .add_state_event(factory.member(&own))
                .add_state_event(
                    factory
                        .event(SpaceChildEventContent::new(vec![
                            "example.org".try_into().expect("server name"),
                        ]))
                        .state_key(SPACE_CHILD),
                ),
        )
        .await;
}

fn space(space_id: &str, child_room_ids: &[&str]) -> SpaceSummary {
    SpaceSummary {
        space_id: space_id.to_owned(),
        raw_name: None,
        display_name: "Synthetic space".to_owned(),
        avatar: None,
        join_rule: None,
        child_room_ids: child_room_ids.iter().map(|id| (*id).to_owned()).collect(),
        parent_side_child_room_ids: child_room_ids.iter().map(|id| (*id).to_owned()).collect(),
    }
}

fn dm_room(room_id: &str) -> RoomSummary {
    RoomSummary {
        is_dm: true,
        dm_user_ids: vec!["@peer:example.org".to_owned()],
        ..room_summary(room_id)
    }
}

fn space_room(room_id: &str, space_id: &str) -> RoomSummary {
    RoomSummary {
        parent_space_ids: vec![space_id.to_owned()],
        ..room_summary(room_id)
    }
}

/// Wait for `request_id`'s terminal selection outcome within the budget.
async fn selection_terminal(
    connection: &mut CoreConnection,
    request_id: RequestId,
) -> IntentOutcome {
    tokio::time::timeout(SELECTION_BUDGET, async {
        loop {
            if let Ok(CoreEvent::IntentLifecycle {
                request_id: settled,
                outcome,
                ..
            }) = connection.recv_event().await
                && settled == request_id
            {
                return outcome;
            }
        }
    })
    .await
    .expect("selection must settle while RoomActor waits on the homeserver")
}

async fn select_room(connection: &mut CoreConnection, room_id: &str) -> AppState {
    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Room(RoomCommand::SelectRoom {
            request_id,
            room_id: room_id.to_owned(),
        }))
        .await
        .expect("submit room selection");
    assert_eq!(
        selection_terminal(connection, request_id).await,
        IntentOutcome::Committed
    );
    connection.snapshot()
}

async fn select_space(connection: &mut CoreConnection, space_id: &str) -> AppState {
    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Room(RoomCommand::SelectSpace {
            request_id,
            space_id: Some(space_id.to_owned()),
        }))
        .await
        .expect("submit Space selection");
    let deadline = Instant::now() + SELECTION_BUDGET;
    loop {
        let state = connection.snapshot();
        if state.navigation.active_space_id.as_deref() == Some(space_id) {
            return state;
        }
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .expect("Space selection must commit while RoomActor waits on the homeserver");
        let _ = tokio::time::timeout(remaining, connection.next_versioned_snapshot()).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn selection_commits_while_a_room_actor_http_operation_is_held() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    sync_space(&server, &client).await;
    let (started_tx, mut started_rx) = mpsc::unbounded_channel();
    server
        .mock_get_hierarchy()
        .respond_with(DelayedHierarchy {
            started: started_tx,
        })
        .mount()
        .await;
    // Hold the own-identity keys query for the test's lifetime: the trust
    // recheck the restore requests must neither resolve nor fail mid-test,
    // since a failed recheck would make the session provisional.
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/_matrix/client/v3/keys/query"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({}))
                .set_delay(Duration::from_secs(60)),
        )
        .mount(server.server())
        .await;

    let data_dir = tempfile::tempdir().expect("data dir");
    let credential_dir = tempfile::tempdir().expect("credential dir");
    let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_owned(),
        credential_dir.path().to_owned(),
    );
    let mut connection = runtime.attach();
    runtime
        .inject_actions(restore_ready_actions![AppAction::RoomListUpdated {
            spaces: vec![
                space(SPACE, &[SPACE_CHILD]),
                space(OTHER_SPACE, &[OTHER_SPACE_ROOM]),
            ],
            rooms: vec![
                room_summary(ROOM_A),
                room_summary(ROOM_B),
                dm_room(DM),
                space_room(OTHER_SPACE_ROOM, OTHER_SPACE),
            ],
        },])
        .await;
    wait_for_state_event(&mut connection, |state| {
        matches!(state.session, SessionState::Ready(_)) && state.rooms.len() == 4
    })
    .await;
    assert!(
        runtime
            .install_account_session_for_testing(Arc::new(
                MatrixClientSession::from_client_for_testing(client, session_info()),
            ))
            .await,
        "install the mock-homeserver session"
    );

    // Select the Space and start its children load; RoomActor now waits on
    // the delayed `/hierarchy` response.
    let state = select_space(&mut connection, SPACE).await;
    assert_eq!(
        state.space_children.selected_space_id.as_deref(),
        Some(SPACE)
    );
    let load_request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Room(RoomCommand::LoadSpaceChildren {
            request_id: load_request_id,
            space_id: SPACE.to_owned(),
            generation: state.space_children.generation,
        }))
        .await
        .expect("submit Space children load");
    tokio::time::timeout(Duration::from_secs(5), started_rx.recv())
        .await
        .expect("RoomActor starts the /hierarchy request")
        .expect("hierarchy responder remains open");
    let held_since = Instant::now();

    // Every selection commits before the held response can arrive.
    let state = select_room(&mut connection, DM).await;
    assert_eq!(state.navigation.active_room_id.as_deref(), Some(DM));
    let state = select_room(&mut connection, ROOM_B).await;
    assert_eq!(state.navigation.active_room_id.as_deref(), Some(ROOM_B));
    let state = select_space(&mut connection, OTHER_SPACE).await;
    assert_eq!(
        state.navigation.active_room_id.as_deref(),
        Some(OTHER_SPACE_ROOM)
    );
    assert!(
        held_since.elapsed() < HIERARCHY_DELAY,
        "the selections completed while the response was still held"
    );

    // The late response settles its own request, and the old Space does not
    // come back.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match connection.recv_event().await {
                Ok(CoreEvent::Room(RoomEvent::SpaceChildrenLoaded { request_id, .. }))
                | Ok(CoreEvent::OperationFailed { request_id, .. })
                    if request_id == load_request_id =>
                {
                    return;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("the held Space children load settles after the delay");
    assert!(held_since.elapsed() >= HIERARCHY_DELAY);
    let state = connection.snapshot();
    assert_eq!(
        state.navigation.active_space_id.as_deref(),
        Some(OTHER_SPACE)
    );
    assert_eq!(
        state.navigation.active_room_id.as_deref(),
        Some(OTHER_SPACE_ROOM)
    );
    assert_eq!(
        state.space_children.selected_space_id.as_deref(),
        Some(OTHER_SPACE)
    );
    assert!(
        state
            .space_children
            .children
            .iter()
            .all(|child| child.room_id != SPACE_CHILD),
        "the old Space's late children must not be painted"
    );

    drop(connection);
    runtime.shutdown().await;
}
