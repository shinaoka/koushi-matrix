//! #1146: a focused SDK build must never wedge the TimelineManager loop.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use koushi_state::{AppAction, TimelineThreadRootOrder};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::executor;
use koushi_protocol::command::{InitialBackfillPolicy, TimelineCommand};
use koushi_protocol::event::CoreEvent;
use koushi_protocol::failure::{CoreFailure, TimelineFailureKind};
use koushi_protocol::ids::{AccountKey, RequestId, TimelineKey, TimelineKind};

use super::super::manager::{TimelineManagerControl, TimelineMessage};
use super::super::navigation::{
    NavigationProjectionCleanup, NavigationProjectionIngress, NavigationProjectionIntent,
};
use super::super::test_support::{fake_rid, focused_key, live_tail_test_manager};
use super::FocusedBuildProbe;

type BuildRelease = oneshot::Sender<Result<(), TimelineFailureKind>>;

struct Harness {
    msg_tx: mpsc::Sender<TimelineMessage>,
    control_tx: mpsc::Sender<TimelineManagerControl>,
    ingress: NavigationProjectionIngress,
    action_rx: mpsc::Receiver<Vec<AppAction>>,
    event_rx: broadcast::Receiver<CoreEvent>,
    started_rx: mpsc::UnboundedReceiver<BuildRelease>,
    _task: executor::JoinHandle<()>,
}

/// A running sessionless manager whose focused builds block at the SDK
/// `build()` boundary until the test releases them.
fn spawn_harness() -> Harness {
    spawn_harness_with(|_| {})
}

fn spawn_harness_with(
    configure: impl FnOnce(&mut super::super::manager::TimelineManagerActor),
) -> Harness {
    let mut manager = live_tail_test_manager(HashMap::new());
    configure(&mut manager);
    let (action_tx, action_rx) = mpsc::channel(64);
    manager.action_tx = action_tx;
    let (event_tx, event_rx) = broadcast::channel(64);
    manager.event_tx = event_tx;
    let (control_tx, control_rx) = mpsc::channel(1);
    manager.control_rx = Some(control_rx);
    let (ingress, navigation_rx) = NavigationProjectionIngress::channel();
    manager.navigation_projection_rx = Some(navigation_rx);
    let (started_tx, started_rx) = mpsc::unbounded_channel();
    manager.focused_builds.test_gate = Some(Arc::new(move || {
        let (release, released) = oneshot::channel();
        let _ = started_tx.send(release);
        Box::pin(async move { released.await.unwrap_or(Err(TimelineFailureKind::Sdk)) })
    }));
    let msg_tx = manager.msg_tx.clone();
    let task = executor::spawn(manager.run());
    Harness {
        msg_tx,
        control_tx,
        ingress,
        action_rx,
        event_rx,
        started_rx,
        _task: task,
    }
}

fn other_focused_key() -> TimelineKey {
    TimelineKey {
        account_key: AccountKey("@a:test".to_owned()),
        kind: TimelineKind::Focused {
            room_id: "!r:test".to_owned(),
            event_id: "$other:test".to_owned(),
        },
    }
}

fn home_room_key() -> TimelineKey {
    TimelineKey::room(AccountKey("@a:test".to_owned()), "!home:test")
}

impl Harness {
    async fn subscribe(&self, request_id: RequestId, key: TimelineKey) {
        assert!(
            self.msg_tx
                .send(TimelineMessage::Command(TimelineCommand::Subscribe {
                    request_id,
                    key,
                    initial_backfill: InitialBackfillPolicy::Disabled,
                }))
                .await
                .is_ok()
        );
    }

    async fn build_started(&mut self) -> BuildRelease {
        executor::timeout(Duration::from_secs(1), self.started_rx.recv())
            .await
            .expect("the focused build must reach the SDK boundary")
            .expect("build gate sender")
    }

    async fn probe(&self, key: &TimelineKey) -> FocusedBuildProbe {
        let (response, probe) = oneshot::channel();
        let _ = executor::timeout(
            Duration::from_secs(1),
            self.msg_tx.send(TimelineMessage::TestFocusedBuildProbe {
                key: key.clone(),
                response,
            }),
        )
        .await
        .expect("the manager mailbox must accept the probe");
        executor::timeout(Duration::from_secs(1), probe)
            .await
            .expect("the manager must answer while a focused build is in flight")
            .expect("probe response")
    }

    async fn next_action(&mut self) -> AppAction {
        let batch = executor::timeout(Duration::from_secs(1), self.action_rx.recv())
            .await
            .expect("manager action")
            .expect("action channel");
        assert_eq!(batch.len(), 1);
        batch.into_iter().next().expect("one action")
    }

    /// Wait until the manager has consumed every focused-build completion
    /// down to `remaining`, so a late result has provably been handled.
    async fn wait_for_in_flight(&self, key: &TimelineKey, remaining: usize) {
        executor::timeout(Duration::from_secs(5), async {
            while self.probe(key).await.in_flight != remaining {
                executor::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("focused-build completions must be consumed");
    }

    async fn wait_for_action(&mut self, wanted: impl Fn(&AppAction) -> bool) {
        executor::timeout(Duration::from_secs(10), async {
            loop {
                let batch = self.action_rx.recv().await.expect("action channel");
                if batch.iter().any(&wanted) {
                    return;
                }
            }
        })
        .await
        .expect("expected manager action");
    }

    fn no_pending_action(&mut self) {
        assert!(
            matches!(
                self.action_rx.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
            ),
            "no further manager action is expected"
        );
    }
}

#[tokio::test]
async fn focused_build_in_flight_does_not_block_control_or_newer_navigation() {
    let mut harness = spawn_harness();
    let key = focused_key();
    harness.subscribe(fake_rid(1_146), key.clone()).await;
    let release = harness.build_started().await;

    // Manager control completes while the SDK build is still held pending.
    let (acknowledged, acknowledgement) = oneshot::channel();
    harness
        .control_tx
        .send(TimelineManagerControl::DisplayPolicyChanged {
            thread_root_order: TimelineThreadRootOrder::LatestReply,
            hide_redacted: true,
            acknowledged,
        })
        .await
        .expect("control admission");
    executor::timeout(Duration::from_secs(1), acknowledgement)
        .await
        .expect("control must not wait for the focused build")
        .expect("control acknowledgement");

    // Newer navigation (a Home/room selection that no longer wants the
    // focused key) commits and retires the in-flight build first.
    assert!(harness.ingress.admit(
        NavigationProjectionIntent {
            generation: 1,
            key: home_room_key(),
            cause_request_id: fake_rid(1_147),
            replay_existing: false,
            cleanup: NavigationProjectionCleanup::default(),
        },
        None,
    ));
    assert!(matches!(
        harness.next_action().await,
        AppAction::TimelineSubscribed { room_id } if room_id == "!home:test"
    ));
    let probe = harness.probe(&key).await;
    assert_eq!(probe.pending_request_id, None);
    assert!(!probe.installed);
    assert_eq!(probe.room_leases, 0, "the retired build releases its lease");
    assert_eq!(probe.actor_generation, None);
    assert!(harness.probe(&home_room_key()).await.installed);

    assert!(
        release.is_closed(),
        "Home must settle the underlying SDK build"
    );
    harness.wait_for_in_flight(&key, 0).await;
    let probe = harness.probe(&key).await;
    assert!(!probe.installed);
    harness.no_pending_action();
}

#[tokio::test]
async fn superseded_focused_build_cannot_install_under_the_newer_owner() {
    let mut harness = spawn_harness();
    let a = focused_key();
    let b = other_focused_key();
    harness.subscribe(fake_rid(1), a.clone()).await;
    let release_a = harness.build_started().await;

    // AppActor admits the new desired foreground before subscribing it.
    harness.ingress.admit_focused(Some(b.clone()));
    harness.subscribe(fake_rid(2), b.clone()).await;
    let release_b = harness.build_started().await;

    let probe_a = harness.probe(&a).await;
    assert_eq!(probe_a.pending_request_id, None);
    assert!(!probe_a.installed);
    assert_eq!(
        harness.probe(&b).await.pending_request_id,
        Some(fake_rid(2))
    );
    assert_eq!(probe_a.room_leases, 1, "only B's lease remains");

    assert!(
        release_a.is_closed(),
        "supersession must settle A's SDK build"
    );
    harness.wait_for_in_flight(&a, 1).await;
    assert!(!harness.probe(&a).await.installed);
    assert_eq!(
        harness.probe(&b).await.pending_request_id,
        Some(fake_rid(2))
    );
    harness.no_pending_action();
    release_b.send(Ok(())).expect("B is still building");
    assert!(matches!(
        harness.next_action().await,
        AppAction::FocusedContextSubscribed { event_id, .. } if event_id == "$other:test"
    ));
    let probe_b = harness.probe(&b).await;
    assert!(probe_b.installed);
    assert_eq!(probe_b.room_leases, 1);
    assert!(!harness.probe(&a).await.installed);
    harness.no_pending_action();
}

#[tokio::test]
async fn repeated_subscribe_for_a_pending_focused_build_adopts_the_latest_request() {
    let mut harness = spawn_harness();
    let key = focused_key();
    harness.subscribe(fake_rid(10), key.clone()).await;
    let release = harness.build_started().await;
    harness.subscribe(fake_rid(11), key.clone()).await;

    let probe = harness.probe(&key).await;
    assert_eq!(probe.pending_request_id, Some(fake_rid(11)));
    assert_eq!(probe.room_leases, 1, "a coalesced subscribe takes no lease");
    assert!(
        matches!(
            harness.started_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ),
        "the in-flight SDK build is reused, not restarted"
    );

    release.send(Ok(())).expect("build is pending");
    assert!(matches!(
        harness.next_action().await,
        AppAction::FocusedContextSubscribed { .. }
    ));
    let probe = harness.probe(&key).await;
    assert!(probe.installed);
    assert_eq!(probe.pending_request_id, None);
    harness.no_pending_action();
}

#[tokio::test(start_paused = true)]
async fn blocked_focused_build_times_out_releases_ownership_and_retry_succeeds() {
    let mut harness = spawn_harness();
    let key = focused_key();
    harness.subscribe(fake_rid(20), key.clone()).await;
    let stuck = harness.build_started().await;

    let failed = executor::timeout(Duration::from_secs(60), async {
        loop {
            match harness.event_rx.recv().await {
                Ok(CoreEvent::OperationFailed {
                    request_id,
                    failure: CoreFailure::TimelineOperationFailed { kind },
                }) if request_id == fake_rid(20) => return kind,
                Ok(_) => {}
                Err(error) => panic!("event stream closed: {error:?}"),
            }
        }
    })
    .await
    .expect("a blocked focused build must reach a bounded failure");
    assert_eq!(failed, TimelineFailureKind::Timeout);
    assert!(matches!(
        harness.next_action().await,
        AppAction::FocusedContextSubscriptionFailed { .. }
    ));
    // Failure is published only after the underlying build has stopped;
    // merely detaching it leaves SDK cache guards held and can block retry.
    assert!(
        stuck.is_closed(),
        "timeout must drop the underlying SDK build"
    );
    let probe = harness.probe(&key).await;
    assert_eq!(probe.in_flight, 0);
    assert_eq!(probe.pending_request_id, None);
    assert!(!probe.installed);
    assert_eq!(probe.room_leases, 0);
    assert_eq!(probe.actor_generation, None);

    // Retry without restarting anything.
    harness.subscribe(fake_rid(21), key.clone()).await;
    harness
        .build_started()
        .await
        .send(Ok(()))
        .expect("retry build is pending");
    assert!(matches!(
        harness.next_action().await,
        AppAction::FocusedContextSubscribed { .. }
    ));
    let probe = harness.probe(&key).await;
    assert!(probe.installed);
    assert_eq!(probe.room_leases, 1);
}

async fn subscribe_directly(
    manager: &mut super::super::manager::TimelineManagerActor,
    request_id: RequestId,
    key: &TimelineKey,
) {
    manager
        .handle_subscribe(
            request_id,
            key.clone(),
            true,
            true,
            InitialBackfillPolicy::Disabled,
        )
        .await;
}

#[tokio::test]
async fn late_completion_of_a_cancelled_build_cannot_install_under_a_newer_same_key_owner() {
    // A -> Home -> A again for the same event: the first build's late result
    // must not install, announce, or take over the second owner's build.
    let mut manager = live_tail_test_manager(HashMap::new());
    let (action_tx, mut action_rx) = mpsc::channel(8);
    manager.action_tx = action_tx;
    manager.focused_builds.test_gate = Some(Arc::new(|| Box::pin(futures_util::future::pending())));
    let key = focused_key();
    subscribe_directly(&mut manager, fake_rid(30), &key).await;
    let stale_generation = manager.focused_builds.pending[&key].activation.generation;
    manager.unsubscribe_timeline(&key).await;
    manager.focused_builds.test_gate = None;
    subscribe_directly(&mut manager, fake_rid(31), &key).await;
    let current_generation = manager.focused_builds.pending[&key].activation.generation;
    assert_ne!(stale_generation, current_generation);

    manager
        .handle_focused_build_completion(super::FocusedBuildCompletion {
            key: key.clone(),
            actor_generation: stale_generation,
            result: Ok(super::PreparedFocusedTimeline::TestActor),
        })
        .await;
    let probe = manager.focused_build_probe(&key);
    assert!(!probe.installed, "a stale build must not install");
    assert_eq!(probe.pending_request_id, Some(fake_rid(31)));
    assert_eq!(probe.actor_generation, Some(current_generation));
    assert_eq!(probe.room_leases, 1);
    assert!(action_rx.try_recv().is_err(), "nothing is announced");

    manager
        .handle_focused_build_completion(super::FocusedBuildCompletion {
            key: key.clone(),
            actor_generation: current_generation,
            result: Ok(super::PreparedFocusedTimeline::TestActor),
        })
        .await;
    let probe = manager.focused_build_probe(&key);
    assert!(probe.installed);
    assert_eq!(probe.room_leases, 1);
    assert!(matches!(
        action_rx.try_recv().expect("subscribed action").as_slice(),
        [AppAction::FocusedContextSubscribed { .. }]
    ));
}

#[tokio::test]
async fn failed_focused_build_rolls_back_and_reports_once() {
    let mut manager = live_tail_test_manager(HashMap::new());
    let (action_tx, mut action_rx) = mpsc::channel(8);
    manager.action_tx = action_tx;
    let (event_tx, mut event_rx) = broadcast::channel(8);
    manager.event_tx = event_tx;
    manager.focused_builds.test_gate = Some(Arc::new(|| {
        Box::pin(async { Err(TimelineFailureKind::Sdk) })
    }));
    let key = focused_key();
    subscribe_directly(&mut manager, fake_rid(40), &key).await;
    let generation = manager.focused_builds.pending[&key].activation.generation;
    manager
        .handle_focused_build_completion(super::FocusedBuildCompletion {
            key: key.clone(),
            actor_generation: generation,
            result: Err(super::FocusedBuildFailure::Sdk),
        })
        .await;
    // A duplicate terminal for the same build is inert.
    manager
        .handle_focused_build_completion(super::FocusedBuildCompletion {
            key: key.clone(),
            actor_generation: generation,
            result: Err(super::FocusedBuildFailure::Interrupted),
        })
        .await;
    let probe = manager.focused_build_probe(&key);
    assert!(!probe.installed);
    assert_eq!(probe.pending_request_id, None);
    assert_eq!(probe.room_leases, 0);
    assert_eq!(probe.actor_generation, None);
    assert!(matches!(
        action_rx.try_recv().expect("failure action").as_slice(),
        [AppAction::FocusedContextSubscriptionFailed { .. }]
    ));
    assert!(action_rx.try_recv().is_err());
    assert!(matches!(
        event_rx.try_recv(),
        Ok(CoreEvent::OperationFailed { request_id, .. }) if request_id == fake_rid(40)
    ));
    assert!(event_rx.try_recv().is_err(), "exactly one failure terminal");
}

#[tokio::test]
async fn shutdown_settles_underlying_focused_build_before_acknowledgement() {
    let mut harness = spawn_harness();
    harness.subscribe(fake_rid(49), focused_key()).await;
    let release = harness.build_started().await;
    let (acknowledged, acknowledgement) = oneshot::channel();
    harness
        .control_tx
        .send(TimelineManagerControl::Shutdown { acknowledged })
        .await
        .expect("shutdown admission");
    executor::timeout(Duration::from_secs(1), acknowledgement)
        .await
        .expect("shutdown must settle")
        .expect("shutdown acknowledgement");
    assert!(
        release.is_closed(),
        "shutdown acknowledgement must follow SDK future destruction"
    );
    harness._task.await.expect("manager task settled");
}

#[tokio::test]
async fn sdk_focused_cache_context_error_does_not_poison_retry() {
    use matrix_sdk::event_cache::EventFocusThreadMode;
    use matrix_sdk::ruma::{event_id, room_id};
    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    use matrix_sdk_test::{ALICE, event_factory::EventFactory};
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, ResponseTemplate};

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    client
        .event_cache()
        .subscribe()
        .expect("event cache subscription");
    let room = room_id!("!focused-error:example.org");
    let target = event_id!("$focused-error:example.org");
    server.sync_joined_room(&client, room).await;
    Mock::given(method("GET"))
        .and(path_regex(r"/context/"))
        .respond_with(ResponseTemplate::new(404).set_body_json(
            serde_json::json!({ "errcode": "M_NOT_FOUND", "error": "synthetic missing event" }),
        ))
        .mount(server.server())
        .await;
    let mode = EventFocusThreadMode::Automatic {
        hide_threaded_events: false,
    };
    assert!(
        client
            .event_cache()
            .event_focused(room, target, mode, 20)
            .await
            .is_err()
    );

    let event = EventFactory::new()
        .room(room)
        .sender(&ALICE)
        .text_msg("target")
        .event_id(target)
        .into_event();
    Mock::given(method("GET"))
        .and(path_regex(r"/context/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "event": event.into_raw().json(), "events_before": [], "events_after": [], "state": []
        })))
        .with_priority(1)
        .mount(server.server())
        .await;
    let (cache, _handles) = client
        .event_cache()
        .event_focused(room, target, mode, 20)
        .await
        .expect("context failure must not strand registered cache state");
    assert!(
        cache
            .events()
            .await
            .expect("focused events")
            .iter()
            .any(|event| event.event_id() == Some(target))
    );
}

#[tokio::test]
async fn sdk_focused_build_waiting_on_context_is_cancelled_and_retry_starts_fresh() {
    // Production build path: the real SDK `TimelineFocus::Event` build waits
    // on a delayed `/context` response while the manager stays responsive;
    // retiring it releases the SDK focused-cache work so a retry completes.
    use koushi_sdk::MatrixClientSession;
    use matrix_sdk::ruma::{event_id, room_id};
    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    use matrix_sdk_test::{ALICE, event_factory::EventFactory};
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, ResponseTemplate};

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    client
        .event_cache()
        .subscribe()
        .expect("event cache subscription");
    let sdk_room_id = room_id!("!focused-build:example.org");
    let target = event_id!("$focused-target:example.org");
    server.sync_joined_room(&client, sdk_room_id).await;
    // `/context` is slow while sync stays healthy.
    let factory = EventFactory::new().room(sdk_room_id).sender(&ALICE);
    let target_event = factory.text_msg("target").event_id(target).into_event();
    let context = serde_json::json!({
        "event": target_event.into_raw().json(),
        "events_before": [],
        "events_after": [],
        "state": [],
    });
    Mock::given(method("GET"))
        .and(path_regex(r"/context/"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(context.clone())
                .set_delay(Duration::from_secs(60)),
        )
        .mount(server.server())
        .await;
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client,
        koushi_state::SessionInfo {
            homeserver: "http://example.invalid".to_owned(),
            user_id: ALICE.to_string(),
            device_id: "DEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    ));
    let mut harness = spawn_harness_with(|manager| manager.session = Some(session));
    let key = TimelineKey {
        account_key: AccountKey(ALICE.to_string()),
        kind: TimelineKind::Focused {
            room_id: sdk_room_id.to_string(),
            event_id: target.to_string(),
        },
    };
    harness.ingress.admit_focused(Some(key.clone()));
    harness.subscribe(fake_rid(50), key.clone()).await;

    // The build has reached the remote context request.
    executor::timeout(Duration::from_secs(10), async {
        loop {
            let requests = server
                .server()
                .received_requests()
                .await
                .unwrap_or_default();
            if requests
                .iter()
                .any(|request| request.url.path().contains("/context/"))
            {
                return;
            }
            executor::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the focused build must request remote context");

    let (acknowledged, acknowledgement) = oneshot::channel();
    harness
        .control_tx
        .send(TimelineManagerControl::DisplayPolicyChanged {
            thread_root_order: TimelineThreadRootOrder::LatestReply,
            hide_redacted: false,
            acknowledged,
        })
        .await
        .expect("control admission");
    executor::timeout(Duration::from_secs(1), acknowledgement)
        .await
        .expect("control must not wait for the SDK context request")
        .expect("control acknowledgement");
    assert_eq!(
        harness.probe(&key).await.pending_request_id,
        Some(fake_rid(50))
    );

    // Supersession/Home retires the in-flight build.
    harness.ingress.admit_focused(None);
    executor::timeout(Duration::from_secs(1), async {
        while harness.probe(&key).await.pending_request_id.is_some() {
            executor::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the retired build must be cancelled");
    let probe = harness.probe(&key).await;
    assert!(!probe.installed);
    assert_eq!(probe.room_leases, 0);
    assert_eq!(probe.actor_generation, None);

    // Do not release the original delayed response. A fresh request must
    // succeed before it, proving cancellation left no orphaned SDK state.
    Mock::given(method("GET"))
        .and(path_regex(r"/context/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(context))
        .with_priority(1)
        .mount(server.server())
        .await;
    harness.ingress.admit_focused(Some(key.clone()));
    harness.subscribe(fake_rid(51), key.clone()).await;
    let retry_generation = harness
        .probe(&key)
        .await
        .actor_generation
        .expect("retry activation");
    harness
        .wait_for_action(|action| {
            matches!(action, AppAction::FocusedContextSubscribed { event_id, .. }
                if event_id == target.as_str())
        })
        .await;
    let probe = harness.probe(&key).await;
    assert!(probe.installed);
    assert_eq!(probe.room_leases, 1);
    assert_eq!(probe.actor_generation, Some(retry_generation));
    let context_requests = server
        .server()
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|request| request.url.path().contains("/context/"))
        .count();
    assert_eq!(
        context_requests, 2,
        "the retry starts fresh instead of waiting for cancelled SDK work"
    );
    harness.no_pending_action();
}
