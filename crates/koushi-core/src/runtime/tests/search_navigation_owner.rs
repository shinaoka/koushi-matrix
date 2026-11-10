//! #1465: a main-pane Search navigation keeps its completion owner while a
//! renderer recovery/replay `Subscribe` for the same focused key arrives
//! during the focused build. #1796: a repeat navigation to the event an
//! installed focused actor already shows settles from that actor's replay
//! instead of waiting for the event-navigation deadline. AppActor runs against
//! a real (sessionless) TimelineManager whose focused builds are held at the
//! SDK boundary. All identifiers are synthetic.

use super::*;
use crate::timeline::{GatedFocusedBuildManager, TimelineMessage};
use koushi_state::{EventNavigationFailureKind, EventNavigationSource, EventNavigationState};

const ROOM: &str = "!search-owner-room:example.invalid";
const EVENT: &str = "$search-owner-target:example.invalid";
const USER: &str = "@synthetic:example.invalid";

fn request(sequence: u64) -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(1465),
        sequence,
    }
}

fn focused_key() -> TimelineKey {
    TimelineKey {
        account_key: AccountKey(USER.to_owned()),
        kind: TimelineKind::Focused {
            room_id: ROOM.to_owned(),
            event_id: EVENT.to_owned(),
        },
    }
}

fn selected_room_state() -> AppState {
    let mut state = AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: USER.to_owned(),
            device_id: "SYNTHETIC".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        rooms: vec![unread_diagnostic_room(ROOM)],
        ..AppState::default()
    };
    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: ROOM.to_owned(),
        },
    );
    state
}

struct Harness {
    command_tx: mpsc::Sender<CoreCommandEnvelope>,
    snapshot_rx: watch::Receiver<VersionedAppStateSnapshot>,
    focused_projection_tx: mpsc::UnboundedSender<FocusedProjectionCommitted>,
    manager: GatedFocusedBuildManager,
    /// Request id of every timeline command after the manager accepted it.
    forwarded_rx: mpsc::UnboundedReceiver<RequestId>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    _data_dir: tempfile::TempDir,
}

impl Drop for Harness {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

fn timeline_command_request_id(command: &koushi_protocol::command::TimelineCommand) -> RequestId {
    match command {
        koushi_protocol::command::TimelineCommand::Subscribe { request_id, .. }
        | koushi_protocol::command::TimelineCommand::Unsubscribe { request_id, .. } => *request_id,
        _ => request(0),
    }
}

fn start() -> Harness {
    let data_dir = tempfile::tempdir().expect("runtime data directory");
    let (
        actor,
        command_tx,
        action_tx,
        mut account_rx,
        _event_rx,
        snapshot_rx,
        navigation_projection_rx,
        _event_navigation_prepared_tx,
        focused_projection_tx,
    ) = app_actor_event_navigation_fixture(data_dir.path(), selected_room_state());
    let mut manager = crate::timeline::spawn_gated_focused_build_manager(
        navigation_projection_rx,
        focused_projection_tx.clone(),
    );
    let mut manager_actions = std::mem::replace(&mut manager.action_rx, mpsc::channel(1).1);
    let msg_tx = manager.msg_tx();
    let (forwarded_tx, forwarded_rx) = mpsc::unbounded_channel();
    // Stand-in AccountActor: forwards timeline commands to the manager and
    // locates the target event.
    let account = tokio::spawn(async move {
        while let Some(message) = account_rx.recv().await {
            let command = match message {
                AccountMessage::EnsureRoomEventCached { response_tx, .. } => {
                    let _ = response_tx.send(crate::account::RoomEventLookupResult::Located);
                    continue;
                }
                AccountMessage::TimelineCommand(command)
                | AccountMessage::TimelineCommandWithComposerFormatting { command, .. } => command,
                _ => continue,
            };
            let request_id = timeline_command_request_id(&command);
            if msg_tx
                .send(TimelineMessage::Command(command))
                .await
                .is_err()
            {
                return;
            }
            let _ = forwarded_tx.send(request_id);
        }
    });
    // Manager actions reach the reducer through AppActor, as in production.
    let actions = tokio::spawn(async move {
        while let Some(batch) = manager_actions.recv().await {
            if action_tx.send(batch).await.is_err() {
                return;
            }
        }
    });
    let actor_task = tokio::spawn(async move {
        let _ = actor.run().await;
    });
    Harness {
        command_tx,
        snapshot_rx,
        focused_projection_tx,
        manager,
        forwarded_rx,
        tasks: vec![account, actions, actor_task],
        _data_dir: data_dir,
    }
}

impl Harness {
    async fn navigate(&mut self, request_id: RequestId) {
        self.navigate_from(EventNavigationSource::Search, request_id)
            .await;
    }

    async fn navigate_from(&mut self, source: EventNavigationSource, request_id: RequestId) {
        use koushi_protocol::command::EventNavigationMissingTargetPolicy as Policy;
        let missing_target_policy = match source {
            EventNavigationSource::Activity | EventNavigationSource::Search => Policy::LiveFallback,
            EventNavigationSource::Pinned
            | EventNavigationSource::Notification
            | EventNavigationSource::ReplyQuote => Policy::Fail,
        };
        self.command_tx
            .send(CoreCommandEnvelope::Public {
                command: CoreCommand::App(AppCommand::NavigateToEvent {
                    request_id,
                    room_id: ROOM.to_owned(),
                    event_id: EVENT.to_owned(),
                    source,
                    missing_target_policy,
                }),
                composer_permit: None,
                admission: None,
            })
            .await
            .expect("navigation command");
        self.wait_forwarded(request_id).await;
    }

    /// The pending banner's cancel: the existing `CloseFocusedContext` command.
    async fn close_focused_context(&mut self, request_id: RequestId) {
        self.command_tx
            .send(CoreCommandEnvelope::Public {
                command: CoreCommand::App(AppCommand::CloseFocusedContext { request_id }),
                composer_permit: None,
                admission: None,
            })
            .await
            .expect("close focused context command");
    }

    /// The renderer's recovery `ensure_timeline_subscribed` for the same key.
    async fn renderer_subscribe(&mut self, request_id: RequestId) {
        self.command_tx
            .send(CoreCommandEnvelope::Public {
                command: CoreCommand::Timeline(
                    koushi_protocol::command::TimelineCommand::Subscribe {
                        request_id,
                        key: focused_key(),
                        initial_backfill: koushi_protocol::command::InitialBackfillPolicy::Disabled,
                    },
                ),
                composer_permit: None,
                admission: None,
            })
            .await
            .expect("renderer subscribe command");
        self.wait_forwarded(request_id).await;
    }

    /// Wait until the manager accepted `request_id`'s timeline command; a
    /// later manager probe therefore observes it as handled.
    async fn wait_forwarded(&mut self, request_id: RequestId) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while self.forwarded_rx.recv().await != Some(request_id) {}
        })
        .await
        .expect("the timeline command must reach the manager");
    }

    async fn wait_for_state(
        &mut self,
        label: &str,
        predicate: impl Fn(&AppState) -> bool,
    ) -> AppState {
        let found = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let state = self.snapshot_rx.borrow_and_update().state.clone();
                if predicate(&state) {
                    return state;
                }
                if self.snapshot_rx.changed().await.is_err() {
                    future::pending::<()>().await;
                }
            }
        })
        .await;
        match found {
            Ok(state) => state,
            Err(_) => panic!(
                "{label}; last event navigation: {:?}, anchor: {:?}",
                self.snapshot_rx.borrow().state.navigation.event_navigation,
                self.snapshot_rx
                    .borrow()
                    .state
                    .navigation
                    .main_timeline_anchor,
            ),
        }
    }

    async fn wait_anchored(&mut self) -> AppState {
        self.wait_anchored_from(EventNavigationSource::Search).await
    }

    async fn wait_anchored_from(&mut self, source: EventNavigationSource) -> AppState {
        let state = self
            .wait_for_state("search navigation must anchor", |state| {
                matches!(
                    state.navigation.event_navigation,
                    EventNavigationState::Anchored {
                        source: anchored,
                        ..
                    } if anchored == source
                )
            })
            .await;
        assert_eq!(
            state
                .navigation
                .main_timeline_anchor
                .as_ref()
                .map(|anchor| anchor.event_id.as_str()),
            Some(EVENT),
            "the selected event renders in the main timeline"
        );
        state
    }

    fn is_opening(&self) -> bool {
        matches!(
            self.snapshot_rx.borrow().state.navigation.event_navigation,
            EventNavigationState::Opening {
                source: EventNavigationSource::Search,
                ..
            }
        )
    }

    /// The event navigation must leave `Opening` at a generation newer than
    /// `superseded`; report the settled state with the (paused-clock) time it
    /// took, so a settle that comes from the navigation deadline is visible.
    async fn settle_newer_than(&mut self, superseded: u64) -> (EventNavigationState, Duration) {
        let started = tokio::time::Instant::now();
        let settled = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let state = self
                    .snapshot_rx
                    .borrow_and_update()
                    .state
                    .navigation
                    .event_navigation;
                if state.generation() > superseded
                    && !matches!(state, EventNavigationState::Opening { .. })
                {
                    return state;
                }
                if self.snapshot_rx.changed().await.is_err() {
                    future::pending::<()>().await;
                }
            }
        })
        .await
        .expect("a repeat navigation must settle");
        (settled, started.elapsed())
    }
}

/// The reported failure: request A's build is held, the renderer recovery
/// subscribe B for the same key arrives, then the build completes. Navigation
/// A must still settle and anchor instead of waiting for its deadline.
#[tokio::test]
async fn search_navigation_survives_renderer_recovery_subscribe_during_focused_build() {
    let mut harness = start();
    let key = focused_key();
    harness.navigate(request(1)).await;
    let release = harness.manager.build_started().await;
    assert!(harness.is_opening());

    harness.renderer_subscribe(request(2)).await;
    let owner_after_recovery = harness.manager.pending_owner(&key).await;
    assert!(harness.manager.no_build_started(), "the build is reused");
    // Delayed completion: the build settles well after the recovery request.
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(harness.is_opening());

    release.send(Ok(())).expect("build is pending");
    harness.wait_anchored().await;
    assert!(harness.manager.installed(&key).await);
    assert_eq!(
        owner_after_recovery,
        Some(request(1)),
        "a renderer recovery subscribe must not take over the navigation owner"
    );
}

/// Fast completion: the build settles before any renderer request; a later
/// renderer replay subscribe leaves the anchored navigation untouched.
#[tokio::test]
async fn search_navigation_fast_completion_anchors_and_tolerates_a_later_replay() {
    let mut harness = start();
    let key = focused_key();
    harness.navigate(request(1)).await;
    harness
        .manager
        .build_started()
        .await
        .send(Ok(()))
        .expect("build is pending");
    harness.wait_anchored().await;

    harness.renderer_subscribe(request(2)).await;
    assert!(harness.manager.installed(&key).await);
    assert!(
        harness.manager.no_build_started(),
        "a replay never rebuilds"
    );
    harness.wait_anchored().await;
}

/// A commit correlated with the observer request is stale for navigation A
/// and must neither settle nor fail it; A's own commit still anchors.
#[tokio::test]
async fn observer_correlated_commit_is_ignored_and_the_owner_commit_anchors() {
    let mut harness = start();
    let key = focused_key();
    harness.navigate(request(1)).await;
    let release = harness.manager.build_started().await;
    harness.renderer_subscribe(request(2)).await;

    harness
        .focused_projection_tx
        .send(FocusedProjectionCommitted {
            projection_request_id: request(2),
            key: key.clone(),
            actor_generation: 1,
            timeline_generation: TimelineGeneration(0),
            item_count: 1,
            target_present: true,
        })
        .expect("stale commit");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(harness.is_opening(), "a stale commit must not settle A");

    release.send(Ok(())).expect("build is pending");
    harness.wait_anchored().await;
}

/// Repeated selection of the same result is a genuine supersession: the new
/// navigation owns the focused build and is the one that anchors.
#[tokio::test]
async fn repeated_search_selection_of_the_same_event_supersedes_the_owner() {
    let mut harness = start();
    let key = focused_key();
    harness.navigate(request(1)).await;
    let first = harness.manager.build_started().await;
    harness.renderer_subscribe(request(2)).await;

    harness.navigate(request(3)).await;
    assert_eq!(harness.manager.pending_owner(&key).await, Some(request(3)));
    harness.renderer_subscribe(request(4)).await;
    assert_eq!(
        harness.manager.pending_owner(&key).await,
        Some(request(3)),
        "the superseding navigation keeps ownership against a recovery subscribe"
    );

    if first.is_closed() {
        // The superseded build was retired; the new owner started its own.
        harness
            .manager
            .build_started()
            .await
            .send(Ok(()))
            .expect("replacement build is pending");
    } else {
        first.send(Ok(())).expect("build is pending");
    }
    harness.wait_anchored().await;
}

/// A failed focused build settles navigation A even when an observer
/// subscribe coalesced into it.
#[tokio::test]
async fn focused_build_failure_settles_search_navigation_with_an_observer() {
    let mut harness = start();
    harness.navigate(request(1)).await;
    let release = harness.manager.build_started().await;
    harness.renderer_subscribe(request(2)).await;

    release
        .send(Err(koushi_protocol::failure::TimelineFailureKind::Sdk))
        .expect("build is pending");
    harness
        .wait_for_state("search navigation must fail", |state| {
            matches!(
                state.navigation.event_navigation,
                EventNavigationState::Failed {
                    source: EventNavigationSource::Search,
                    failure_kind: EventNavigationFailureKind::Timeline,
                    ..
                }
            )
        })
        .await;
}

/// A focused build that never completes reaches the bounded build deadline
/// and settles navigation A as failed.
#[tokio::test(start_paused = true)]
async fn blocked_focused_build_with_an_observer_settles_at_the_deadline() {
    let mut harness = start();
    harness.navigate(request(1)).await;
    let _held = harness.manager.build_started().await;
    harness.renderer_subscribe(request(2)).await;

    // #1848: the focused budget (30 s) must settle this, before the 45 s
    // backstop could be what fails it.
    let failed = tokio::time::timeout(Duration::from_secs(40), async {
        loop {
            if matches!(
                harness
                    .snapshot_rx
                    .borrow_and_update()
                    .state
                    .navigation
                    .event_navigation,
                EventNavigationState::Failed {
                    source: EventNavigationSource::Search,
                    ..
                }
            ) {
                return;
            }
            if harness.snapshot_rx.changed().await.is_err() {
                future::pending::<()>().await;
            }
        }
    })
    .await;
    assert!(failed.is_ok(), "a blocked build must settle navigation A");
}

/// #1796: the reported failure. A repeat navigation to the event the installed
/// focused actor already shows takes the idempotent replay path instead of
/// starting a build, so the replay must settle the repeat navigation: at once,
/// and without the repeat waiting for the 15 s event-navigation deadline and
/// failing. The pane already shows the target, so the anchor must not move.
#[tokio::test(start_paused = true)]
async fn repeat_search_navigation_to_an_anchored_event_settles_from_the_replay() {
    let mut harness = start();
    let key = focused_key();
    harness.navigate(request(1)).await;
    harness
        .manager
        .build_started()
        .await
        .send(Ok(()))
        .expect("build is pending");
    let first = harness.wait_anchored().await;
    assert!(
        harness.manager.installed(&key).await,
        "the first navigation installed the focused actor"
    );

    harness.navigate(request(3)).await;
    assert!(
        harness.manager.no_build_started(),
        "a repeat navigation reuses the installed actor"
    );
    let (settled, elapsed) = harness
        .settle_newer_than(first.navigation.event_navigation.generation())
        .await;
    assert!(
        matches!(
            settled,
            EventNavigationState::Anchored {
                source: EventNavigationSource::Search,
                ..
            }
        ),
        "the repeat navigation must anchor again; settled {settled:?} after {elapsed:?}"
    );
    assert!(
        elapsed < crate::runtime::navigation::EVENT_NAVIGATION_TIMEOUT,
        "the repeat navigation must settle from the replay, not at the deadline: {elapsed:?}"
    );
    assert_eq!(
        harness
            .snapshot_rx
            .borrow()
            .state
            .navigation
            .main_timeline_anchor
            .as_ref()
            .map(|anchor| anchor.event_id.as_str()),
        Some(EVENT),
        "the repeat navigation leaves the displayed anchor in place"
    );
}

/// A repeat click in the Pinned messages panel is the reported source. It
/// shares the prepared/replay/commit steps with Search, so it must settle the
/// same way.
#[tokio::test(start_paused = true)]
async fn repeat_pinned_navigation_to_an_anchored_event_settles_from_the_replay() {
    let mut harness = start();
    let key = focused_key();
    harness
        .navigate_from(EventNavigationSource::Pinned, request(1))
        .await;
    harness
        .manager
        .build_started()
        .await
        .send(Ok(()))
        .expect("build is pending");
    let first = harness
        .wait_anchored_from(EventNavigationSource::Pinned)
        .await;
    assert!(harness.manager.installed(&key).await, "actor retained");

    harness
        .navigate_from(EventNavigationSource::Pinned, request(3))
        .await;
    assert!(harness.manager.no_build_started(), "the actor is reused");
    let (settled, elapsed) = harness
        .settle_newer_than(first.navigation.event_navigation.generation())
        .await;
    assert!(
        matches!(
            settled,
            EventNavigationState::Anchored {
                source: EventNavigationSource::Pinned,
                ..
            }
        ),
        "the repeat pinned navigation must anchor again; settled {settled:?} after {elapsed:?}"
    );
    assert!(
        elapsed < crate::runtime::navigation::EVENT_NAVIGATION_TIMEOUT,
        "the repeat pinned navigation must settle from the replay: {elapsed:?}"
    );
}

/// The fix correlates a repeat navigation with its own replay; it must not
/// accept any projection commit for the key. A commit carrying the superseded
/// navigation's identity settles nothing. The paused clock makes the barrier
/// deterministic: the delay completes only once no work is runnable, so a
/// wrongly accepted stale commit is always observed before the assertion.
#[tokio::test(start_paused = true)]
async fn a_commit_for_the_superseded_navigation_does_not_settle_the_repeat() {
    let mut harness = start();
    let key = focused_key();
    harness.navigate(request(1)).await;
    let release = harness.manager.build_started().await;
    harness.navigate(request(3)).await;
    assert_eq!(
        harness.manager.pending_owner(&key).await,
        Some(request(3)),
        "the repeat navigation owns the projection"
    );

    harness
        .focused_projection_tx
        .send(FocusedProjectionCommitted {
            projection_request_id: request(1),
            key: key.clone(),
            actor_generation: 1,
            timeline_generation: TimelineGeneration(0),
            item_count: 1,
            target_present: true,
        })
        .expect("stale commit");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        harness.is_opening(),
        "a commit for the superseded navigation must not settle the repeat"
    );

    if release.is_closed() {
        harness
            .manager
            .build_started()
            .await
            .send(Ok(()))
            .expect("replacement build is pending");
    } else {
        release.send(Ok(())).expect("build is pending");
    }
    harness.wait_anchored().await;
}

/// #1848: a slow old-history build that finishes inside its budget must anchor.
/// Under the former 10 s budget and 15 s backstop it was cut at 10 s.
#[tokio::test(start_paused = true)]
async fn slow_old_history_build_within_budget_anchors_without_failure() {
    let mut harness = start();
    harness.navigate(request(1)).await;
    let release = harness.manager.build_started().await;
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(20)).await;
    tokio::task::yield_now().await;
    release.send(Ok(())).expect("build is still pending");
    harness.wait_anchored().await;
}

/// #1848: the AppActor backstop must outlive the focused build it backs, plus
/// the up-to-5 s room-event cache lookup that runs before the build starts
/// (`account/routing.rs` `ROOM_EVENT_CACHE_TIMEOUT`).
#[test]
fn event_navigation_backstop_outlives_the_focused_build_budget() {
    assert!(
        crate::runtime::navigation::EVENT_NAVIGATION_TIMEOUT
            > crate::timeline::FOCUSED_TIMELINE_BUILD_TIMEOUT + Duration::from_secs(5)
    );
}

/// #1848: cancelling the pending navigation from its banner (`CloseFocusedContext`)
/// ends the wait without a failure: the held build is dropped, the owner is
/// cleared, and the superseded deadline cannot publish a failure later.
#[tokio::test(start_paused = true)]
async fn cancelling_a_pending_navigation_publishes_no_failure() {
    let mut harness = start();
    let key = focused_key();
    harness.navigate(request(1)).await;
    let release = harness.manager.build_started().await;

    harness.close_focused_context(request(2)).await;
    harness
        .wait_for_state("cancel must clear the pending navigation", |state| {
            matches!(
                state.navigation.event_navigation,
                EventNavigationState::Idle
            )
        })
        .await;
    assert!(
        release.is_closed(),
        "cancel must drop the held focused build"
    );
    assert_eq!(harness.manager.pending_owner(&key).await, None);

    tokio::time::advance(crate::runtime::navigation::EVENT_NAVIGATION_TIMEOUT).await;
    assert!(
        matches!(
            harness
                .snapshot_rx
                .borrow()
                .state
                .navigation
                .event_navigation,
            EventNavigationState::Idle
        ),
        "no late deadline may publish a failure after cancel"
    );
}
