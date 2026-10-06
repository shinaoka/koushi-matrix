//! #1060 / #1113: local navigation is never gated by a RoomActor network
//! operation.
//!
//! A real runtime (AppActor, AccountActor, RoomActor) holds an SDK session
//! bound to a mock homeserver. RoomActor starts a Space-children load; its
//! `/hierarchy` responder reports that the request arrived and then blocks on a
//! release channel the test owns, so the server decides nothing about timing.
//! RoomActor handles that load serially, so it stays busy until the test
//! releases the response. While the gate is closed, DM, room, and Space
//! selections must each commit, and the late response must not restore the
//! older selection.
//!
//! Correctness is an ordering, not a duration: the gate can only be opened by
//! the test, after the assertions that require it to be closed. The bounded
//! timeouts below are hang guards for event-driven waits, never correctness
//! budgets. All identifiers are synthetic.

use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

/// Deadlock guard for event-driven waits. Every wait it bounds is released by
/// an explicit gate signal or a streamed actor transition; a wait that reaches
/// this deadline is a broken invariant, not a slow machine.
const HANG_GUARD: Duration = Duration::from_secs(30);

/// Fixture hold for the own-identity trust recheck (see the test body). It must
/// outlast any bounded failure path in this file, and no assertion reads it.
const TRUST_RECHECK_HOLD: Duration = Duration::from_secs(3_600);

/// Answers `/hierarchy` for `SPACE` only after the test releases it.
///
/// `started` reports that the server received the request. The responder then
/// blocks the mock server thread until the test sends a release token, so the
/// held window is determined by the assertions rather than by elapsed time.
/// Dropping the release sender (for example while unwinding a failed
/// assertion) disconnects the channel and frees the responder immediately.
struct GatedHierarchy {
    started: mpsc::UnboundedSender<()>,
    release: Mutex<Option<std_mpsc::Receiver<()>>>,
}

impl GatedHierarchy {
    fn new(started: mpsc::UnboundedSender<()>) -> (Self, std_mpsc::Sender<()>) {
        let (release_tx, release_rx) = std_mpsc::channel();
        (
            Self {
                started,
                release: Mutex::new(Some(release_rx)),
            },
            release_tx,
        )
    }
}

impl Respond for GatedHierarchy {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        let _ = self.started.send(());
        // Only the first request is gated: a retry must not wait for a token
        // that the test already spent.
        if let Some(release) = self.release.lock().expect("hierarchy gate").take() {
            let _ = release.recv_timeout(HANG_GUARD);
        }
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
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

/// Private-data-free phase and navigation diagnostic for failure messages.
/// Every identifier here is a synthetic constant from this file.
fn navigation_diagnostic(phase: &str, state: &AppState) -> String {
    format!(
        "navigation phase `{phase}` did not settle: active_room={:?} active_space={:?} \
         selected_space={:?} known_rooms={} space_children={}",
        state.navigation.active_room_id,
        state.navigation.active_space_id,
        state.space_children.selected_space_id,
        state.rooms.len(),
        state.space_children.children.len(),
    )
}

/// Wait for a navigation predicate, rechecked only after the runtime streams an
/// observable transition. `HANG_GUARD` is a deadlock guard.
async fn wait_for_navigation<F>(
    connection: &mut CoreConnection,
    phase: &str,
    predicate: F,
) -> AppState
where
    F: Fn(&AppState) -> bool,
{
    let waited = tokio::time::timeout(HANG_GUARD, async {
        loop {
            let snapshot = connection.snapshot();
            if predicate(&snapshot) {
                return snapshot;
            }
            connection
                .next_versioned_snapshot()
                .await
                .expect("runtime snapshot stream must remain open");
        }
    })
    .await;
    match waited {
        Ok(state) => state,
        Err(_) => panic!("{}", navigation_diagnostic(phase, &connection.snapshot())),
    }
}

/// Wait for `request_id`'s terminal selection outcome, driven by the runtime
/// event stream.
async fn selection_terminal(
    connection: &mut CoreConnection,
    phase: &str,
    request_id: RequestId,
) -> IntentOutcome {
    let waited = tokio::time::timeout(HANG_GUARD, async {
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
    .await;
    match waited {
        Ok(outcome) => outcome,
        Err(_) => panic!("{}", navigation_diagnostic(phase, &connection.snapshot())),
    }
}

async fn select_room(connection: &mut CoreConnection, phase: &str, room_id: &str) -> AppState {
    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Room(RoomCommand::SelectRoom {
            request_id,
            room_id: room_id.to_owned(),
        }))
        .await
        .expect("submit room selection");
    assert_eq!(
        selection_terminal(connection, phase, request_id).await,
        IntentOutcome::Committed,
        "{}",
        navigation_diagnostic(phase, &connection.snapshot())
    );
    connection.snapshot()
}

/// Space selection has no `IntentLifecycle` of its own, so its authoritative
/// commitment is observed on the runtime snapshot stream.
async fn select_space(connection: &mut CoreConnection, phase: &str, space_id: &str) -> AppState {
    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Room(RoomCommand::SelectSpace {
            request_id,
            space_id: Some(space_id.to_owned()),
        }))
        .await
        .expect("submit Space selection");
    wait_for_navigation(connection, phase, |state| {
        state.navigation.active_space_id.as_deref() == Some(space_id)
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn selection_commits_while_a_room_actor_http_operation_is_held() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    sync_space(&server, &client).await;
    // The own-identity trust recheck must stay unresolved for the whole test.
    // A `CheckCurrentDeviceTrust` effect can reach AccountActor after this test
    // installs its mock-homeserver session, and any settlement then (verified,
    // unverified, or failed) replaces the authoritative session with
    // provisional encryption sync and clears navigation.
    //
    // This hold cannot be a release-gated responder: wiremock serves every mock
    // from one single-threaded runtime, so a responder that blocks would also
    // stall the `/hierarchy` request this test has to observe. It is not a
    // correctness budget either: nothing asserts on it, and every wait below
    // is bounded by `HANG_GUARD`, which fires long before this can elapse.
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path_regex(
            r"^/_matrix/client/.*/keys/query$",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({}))
                .set_delay(TRUST_RECHECK_HOLD),
        )
        .mount(server.server())
        .await;
    // The Space children load is the operation under test: it reports that it
    // arrived and stays held until this test releases it.
    let (started_tx, mut started_rx) = mpsc::unbounded_channel();
    let (hierarchy, release_hierarchy) = GatedHierarchy::new(started_tx);
    server
        .mock_get_hierarchy()
        .respond_with(hierarchy)
        .mount()
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
    // Observable fixture readiness: the session is Ready and every synthetic
    // room is known before the invariant is exercised.
    wait_for_navigation(&mut connection, "fixture-ready", |state| {
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

    // Select the Space and start its children load. RoomActor is now held on
    // the `/hierarchy` response and can only resume when this test says so.
    let state = select_space(&mut connection, "initial-space-selection", SPACE).await;
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
    tokio::time::timeout(HANG_GUARD, started_rx.recv())
        .await
        .unwrap_or_else(|_| {
            panic!(
                "hierarchy-request-start: the gated /hierarchy request never arrived; {}",
                navigation_diagnostic("hierarchy-request-start", &connection.snapshot())
            )
        })
        .expect("hierarchy responder remains open");

    // Every selection commits while the gate is still closed. The response
    // cannot be produced before `release_hierarchy` is sent, so these
    // assertions cannot race the held operation.
    let state = select_room(&mut connection, "dm-selection", DM).await;
    assert_eq!(state.navigation.active_room_id.as_deref(), Some(DM));
    let state = select_room(&mut connection, "room-selection", ROOM_B).await;
    assert_eq!(state.navigation.active_room_id.as_deref(), Some(ROOM_B));
    let state = select_space(&mut connection, "other-space-selection", OTHER_SPACE).await;
    assert_eq!(
        state.navigation.active_room_id.as_deref(),
        Some(OTHER_SPACE_ROOM)
    );

    // Opening the gate is the only way the held operation can complete.
    release_hierarchy
        .send(())
        .expect("release the gated /hierarchy response");

    // The late response settles its own request, and the older Space does not
    // come back.
    tokio::time::timeout(HANG_GUARD, async {
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
    .unwrap_or_else(|_| {
        panic!(
            "late-hierarchy-settlement: the released Space children load never settled; {}",
            navigation_diagnostic("late-hierarchy-settlement", &connection.snapshot())
        )
    });
    let state = connection.snapshot();
    let diagnostic = navigation_diagnostic("final-authority", &state);
    assert_eq!(
        state.navigation.active_space_id.as_deref(),
        Some(OTHER_SPACE),
        "{diagnostic}"
    );
    assert_eq!(
        state.navigation.active_room_id.as_deref(),
        Some(OTHER_SPACE_ROOM),
        "{diagnostic}"
    );
    assert_eq!(
        state.space_children.selected_space_id.as_deref(),
        Some(OTHER_SPACE),
        "{diagnostic}"
    );
    assert!(
        state
            .space_children
            .children
            .iter()
            .all(|child| child.room_id != SPACE_CHILD),
        "the old Space's late children must not be painted; {diagnostic}"
    );

    drop(connection);
    runtime.shutdown().await;
}
