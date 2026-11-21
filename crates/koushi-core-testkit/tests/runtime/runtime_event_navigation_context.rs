//! #2039: Activity → Recent navigation to a target that the room event cache
//! already holds.
//!
//! A real runtime (AppActor, AccountActor, TimelineManager) holds an SDK
//! session bound to a mock homeserver. The room cache is seeded through sync
//! with a contiguous window that contains the synthetic target, and no focused
//! cache exists for it. `/context` answers after a fixed delay, so a navigation
//! that waits for remote context shows the delay on its critical path.
//!
//! This is a measurement harness. It prints request counts and click-to-anchor
//! timing (`#2039 measurement ...` lines, read with `--nocapture`) and asserts
//! only the properties that identify the critical path. No duration is a
//! correctness budget beyond "at least the fixed delay". All identifiers are
//! synthetic.

use std::sync::Arc;
use std::time::{Duration, Instant};

use koushi_core::CoreCommand;
use koushi_core::runtime::{CoreConnection, CoreRuntime};
use koushi_protocol::command::{AppCommand, RoomCommand};
use koushi_sdk::MatrixClientSession;
use koushi_state::{
    AppAction, AppState, EventNavigationSource, EventNavigationState, MissingTargetPolicy,
    SessionState,
};
use matrix_sdk::ruma::{EventId, RoomId, user_id};
use matrix_sdk::test_utils::mocks::{MatrixMockServer, RoomContextResponseTemplate};
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};
use wiremock::ResponseTemplate;

#[path = "../support/mod.rs"]
#[expect(
    clippy::duplicate_mod,
    reason = "suite aggregation preserves feature-local fixture scopes and registered helper tests"
)]
mod support;
use support::*;

const ROOM: &str = "!ctx-room:example.org";
const OTHER_ROOM: &str = "!ctx-other-room:example.org";
const BEFORE: &str = "$ctx-before:example.org";
const TARGET: &str = "$ctx-target:example.org";
const AFTER: &str = "$ctx-after:example.org";
/// The fixed `/context` latency that exposes the critical path.
const CONTEXT_DELAY: Duration = Duration::from_millis(1_500);
const HANG_GUARD: Duration = Duration::from_secs(30);
const TRUST_RECHECK_HOLD: Duration = Duration::from_secs(3_600);

struct Measurement {
    click_to_anchor: Duration,
    context_requests: usize,
    event_requests: usize,
    terminal: EventNavigationState,
}

fn event_id(id: &str) -> &EventId {
    <&EventId>::try_from(id).expect("synthetic event id")
}

/// Counts `/context` and `/event` requests the homeserver received for ROOM.
async fn room_requests(server: &MatrixMockServer) -> (usize, usize) {
    let requests = server
        .server()
        .received_requests()
        .await
        .expect("request recording is enabled");
    let count = |needle: &str| {
        requests
            .iter()
            .filter(|request| request.url.path().contains(needle))
            .count()
    };
    (count("/context/"), count("/event/"))
}

async fn measure_activity_click(seed_room_cache: bool) -> Measurement {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let own = user_id!("@alice:example.test").to_owned();
    let room_id = RoomId::parse(ROOM).expect("room id");
    let factory = EventFactory::new().room(&room_id).sender(&own);

    let mut builder = JoinedRoomBuilder::new(&room_id);
    if seed_room_cache {
        builder = builder
            .add_timeline_event(factory.text_msg("before").event_id(event_id(BEFORE)))
            .add_timeline_event(factory.text_msg("target").event_id(event_id(TARGET)));
    }
    builder =
        builder.add_timeline_event(factory.text_msg("after").event_id(event_id(AFTER)));
    server.sync_room(&client, builder).await;

    // Hold the own-identity trust recheck for the whole test (see
    // runtime_navigation_network.rs for the reason).
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
    server
        .mock_room_event_context()
        .with_delay(CONTEXT_DELAY)
        .ok(RoomContextResponseTemplate::new(
            factory
                .text_msg("target")
                .event_id(event_id(TARGET))
                .into_event(),
        )
        .events_before(vec![
            factory
                .text_msg("before")
                .event_id(event_id(BEFORE))
                .into_event(),
        ])
        .events_after(vec![
            factory
                .text_msg("after")
                .event_id(event_id(AFTER))
                .into_event(),
        ])
        .start("ctx-start")
        .end("ctx-end"))
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
            spaces: vec![],
            rooms: vec![room_summary(OTHER_ROOM), room_summary(ROOM)],
        },])
        .await;
    wait_until(&mut connection, |state| {
        matches!(state.session, SessionState::Ready(_)) && state.rooms.len() == 2
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
    // The user is looking at another room (Activity), as in the report.
    let select = connection.next_request_id();
    connection
        .command(CoreCommand::Room(RoomCommand::SelectRoom {
            request_id: select,
            room_id: OTHER_ROOM.to_owned(),
        }))
        .await
        .expect("select other room");
    wait_until(&mut connection, |state| {
        state.navigation.active_room_id.as_deref() == Some(OTHER_ROOM)
    })
    .await;

    let request_id = connection.next_request_id();
    let clicked = Instant::now();
    connection
        .command(CoreCommand::App(AppCommand::NavigateToEvent {
            request_id,
            room_id: ROOM.to_owned(),
            event_id: TARGET.to_owned(),
            source: EventNavigationSource::Activity,
            missing_target_policy: MissingTargetPolicy::LiveFallback,
        }))
        .await
        .expect("submit Activity navigation");
    let state = wait_until(&mut connection, |state| {
        matches!(
            state.navigation.event_navigation,
            EventNavigationState::Anchored { .. }
                | EventNavigationState::LiveFallback { .. }
                | EventNavigationState::InPlace { .. }
                | EventNavigationState::Failed { .. }
        )
    })
    .await;
    let click_to_anchor = clicked.elapsed();
    let (context_requests, event_requests) = room_requests(&server).await;
    let measurement = Measurement {
        click_to_anchor,
        context_requests,
        event_requests,
        terminal: state.navigation.event_navigation.clone(),
    };
    drop(connection);
    runtime.shutdown().await;
    measurement
}

async fn wait_until<F>(connection: &mut CoreConnection, predicate: F) -> AppState
where
    F: Fn(&AppState) -> bool,
{
    tokio::time::timeout(HANG_GUARD, async {
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
    .await
    .expect("navigation state never settled")
}

fn report(label: &str, m: &Measurement) {
    eprintln!(
        "#2039 measurement {label}: click_to_anchor_ms={} context_requests={} \
         event_requests={} terminal={:?} context_delay_ms={}",
        m.click_to_anchor.as_millis(),
        m.context_requests,
        m.event_requests,
        m.terminal,
        CONTEXT_DELAY.as_millis()
    );
}

/// Warm room cache, no focused cache, `/context` delayed by `CONTEXT_DELAY`.
/// Stage 1 characterises the current code: the click waits for remote context.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn warm_cache_activity_click_waits_for_delayed_context() {
    let m = measure_activity_click(true).await;
    report("warm", &m);
    assert!(
        matches!(m.terminal, EventNavigationState::Anchored { .. }),
        "terminal: {:?}",
        m.terminal
    );
    assert!(
        m.click_to_anchor >= CONTEXT_DELAY,
        "the current code anchors only after remote context ({:?})",
        m.click_to_anchor
    );
    assert_eq!(m.context_requests, 1);
}

/// Cold room cache: records the `/event` and `/context` request counts.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cold_cache_activity_click_request_counts() {
    let m = measure_activity_click(false).await;
    report("cold", &m);
}
