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
    SessionState, ThreadPaneState,
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
    /// The thread root the thread pane was opened on, if it was.
    thread_root: Option<String>,
}

const THREAD_ROOT: &str = "$ctx-thread-root:example.org";

fn thread_root_of(state: &AppState) -> Option<String> {
    match &state.thread {
        ThreadPaneState::Opening { root_event_id, .. }
        | ThreadPaneState::Open { root_event_id, .. } => Some(root_event_id.clone()),
        ThreadPaneState::Closed | ThreadPaneState::Failed { .. } => None,
    }
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

/// What the mock homeserver answers `/context` with, after `CONTEXT_DELAY`.
#[derive(Clone, Copy)]
enum ContextAnswer {
    /// The target with its neighbours.
    Usable,
    /// The target is a reply inside a thread, so the build's `/context` is what
    /// reveals the thread root.
    ThreadReply,
    /// `404 M_NOT_FOUND`: the server's authoritative "no such event".
    NotFound,
    /// `500`: a failure that says nothing about whether the event exists.
    ServerError,
}

async fn measure_activity_click(seed_room_cache: bool) -> Measurement {
    measure_click(
        seed_room_cache,
        ContextAnswer::Usable,
        EventNavigationSource::Activity,
        MissingTargetPolicy::LiveFallback,
    )
    .await
}

async fn measure_click(
    seed_room_cache: bool,
    answer: ContextAnswer,
    source: EventNavigationSource,
    missing_target_policy: MissingTargetPolicy,
) -> Measurement {
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
    builder = builder.add_timeline_event(factory.text_msg("after").event_id(event_id(AFTER)));
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
    if !matches!(answer, ContextAnswer::Usable | ContextAnswer::ThreadReply) {
        let (status, body) = match answer {
            ContextAnswer::NotFound => (
                404,
                serde_json::json!({"errcode": "M_NOT_FOUND", "error": "Event not found."}),
            ),
            _ => (
                500,
                serde_json::json!({"errcode": "M_UNKNOWN", "error": "synthetic failure"}),
            ),
        };
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path_regex(
                r"^/_matrix/client/.*/rooms/.*/context/.*$",
            ))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_json(body)
                    .set_delay(CONTEXT_DELAY),
            )
            .mount(server.server())
            .await;
    } else {
        server
            .mock_room_event_context()
            .with_delay(CONTEXT_DELAY)
            .ok(
                RoomContextResponseTemplate::new(if matches!(answer, ContextAnswer::ThreadReply) {
                    factory
                        .text_msg("target")
                        .event_id(event_id(TARGET))
                        .in_thread(event_id(THREAD_ROOT), event_id(TARGET))
                        .into_event()
                } else {
                    factory
                        .text_msg("target")
                        .event_id(event_id(TARGET))
                        .into_event()
                })
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
                .end("ctx-end"),
            )
            .mount()
            .await;
    }

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
            source,
            missing_target_policy,
        }))
        .await
        .expect("submit navigation");
    let state = wait_until(&mut connection, |state| {
        matches!(
            state.navigation.event_navigation,
            EventNavigationState::Anchored { .. }
                | EventNavigationState::LiveFallback { .. }
                | EventNavigationState::InPlace { .. }
                | EventNavigationState::Failed { .. }
        ) || thread_root_of(state).is_some()
    })
    .await;
    let click_to_anchor = clicked.elapsed();
    let (context_requests, event_requests) = room_requests(&server).await;
    let measurement = Measurement {
        click_to_anchor,
        context_requests,
        event_requests,
        terminal: state.navigation.event_navigation.clone(),
        thread_root: thread_root_of(&state),
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
/// The target is already local, so the click must neither wait for nor issue
/// any remote lookup.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn warm_cache_activity_click_anchors_without_remote_lookup() {
    let m = measure_activity_click(true).await;
    report("warm", &m);
    assert!(
        matches!(m.terminal, EventNavigationState::Anchored { .. }),
        "terminal: {:?}",
        m.terminal
    );
    assert_eq!(m.context_requests, 0, "no /context for a cached target");
    assert_eq!(m.event_requests, 0, "no /event for a cached target");
    assert!(
        m.click_to_anchor < Duration::from_millis(500),
        "click must not wait for the {:?} /context delay ({:?})",
        CONTEXT_DELAY,
        m.click_to_anchor
    );
}

/// Cold room cache: `/context` is the first and only remote lookup. No
/// `/event` precedes it, so the click costs one `/context` round trip.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cold_cache_activity_click_uses_context_as_first_lookup() {
    let m = measure_activity_click(false).await;
    report("cold", &m);
    assert_eq!(m.event_requests, 0, "no /event before /context");
    assert_eq!(m.context_requests, 1, "exactly one /context");
    assert!(
        m.click_to_anchor >= CONTEXT_DELAY,
        "the cold click waits for its one /context ({:?})",
        m.click_to_anchor
    );
    assert!(
        matches!(m.terminal, EventNavigationState::Anchored { .. }),
        "terminal: {:?}",
        m.terminal
    );
}

/// A cold thread reply: no `/event` precedes `/context`, and the thread root
/// `/context` revealed routes the click to the thread pane instead of a
/// thread-filtered main-pane window.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cold_cache_thread_reply_opens_the_thread_from_context() {
    let m = measure_click(
        false,
        ContextAnswer::ThreadReply,
        EventNavigationSource::Activity,
        MissingTargetPolicy::LiveFallback,
    )
    .await;
    report("cold-thread-reply", &m);
    assert_eq!(m.event_requests, 0, "no /event before /context");
    assert_eq!(m.thread_root.as_deref(), Some(THREAD_ROOT));
}

/// `/context` answers `M_NOT_FOUND`: the target is authoritatively absent, so
/// Activity falls back to the live room and Pinned reports the missing target.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cold_cache_context_not_found_is_absence() {
    let activity = measure_click(
        false,
        ContextAnswer::NotFound,
        EventNavigationSource::Activity,
        MissingTargetPolicy::LiveFallback,
    )
    .await;
    report("cold-not-found-activity", &activity);
    assert_eq!(activity.event_requests, 0);
    assert_eq!(activity.context_requests, 1);
    assert!(
        matches!(activity.terminal, EventNavigationState::LiveFallback { .. }),
        "terminal: {:?}",
        activity.terminal
    );

    let pinned = measure_click(
        false,
        ContextAnswer::NotFound,
        EventNavigationSource::Pinned,
        MissingTargetPolicy::Fail,
    )
    .await;
    report("cold-not-found-pinned", &pinned);
    assert_eq!(pinned.event_requests, 0);
    assert!(
        matches!(
            pinned.terminal,
            EventNavigationState::Failed {
                failure_kind: koushi_state::EventNavigationFailureKind::TargetMissing,
                ..
            }
        ),
        "terminal: {:?}",
        pinned.terminal
    );
}

/// A `/context` server error says nothing about whether the event exists, so it
/// is a timeline failure for every source, never a live fallback.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cold_cache_context_server_error_is_a_failure_not_absence() {
    let m = measure_click(
        false,
        ContextAnswer::ServerError,
        EventNavigationSource::Activity,
        MissingTargetPolicy::LiveFallback,
    )
    .await;
    report("cold-server-error-activity", &m);
    assert_eq!(m.event_requests, 0);
    assert!(
        matches!(
            m.terminal,
            EventNavigationState::Failed {
                failure_kind: koushi_state::EventNavigationFailureKind::Timeline,
                ..
            }
        ),
        "terminal: {:?}",
        m.terminal
    );
}
