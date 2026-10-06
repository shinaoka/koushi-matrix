//! #1060: selecting a known DM, room, or Space is purely local navigation.
//! AppActor must admit, reduce, publish, and settle it without a round trip
//! through the AccountActor/RoomActor network-operation mailboxes. Every test
//! holds the AccountActor mailbox full for its whole duration, and nothing
//! drains it until the test chooses to, so a regression that awaits the
//! mailbox never progresses at all. `LIVENESS_DEADLINE` therefore only has
//! to separate "blocked forever" from "slow scheduler"; it is not a latency
//! budget. All identifiers are synthetic.

use super::*;
use koushi_protocol::command::RoomCommand;

const USER: &str = "@synthetic:example.invalid";
const ROOM_A: &str = "!navigation-network-a:example.invalid";
const ROOM_B: &str = "!navigation-network-b:example.invalid";
const ROOM_C: &str = "!navigation-network-c:example.invalid";
const DM: &str = "!navigation-network-dm:example.invalid";
const SPACE: &str = "!navigation-network-space:example.invalid";
const SPACE_ROOM: &str = "!navigation-network-space-room:example.invalid";
const EMPTY_SPACE: &str = "!navigation-network-empty-space:example.invalid";
const EVENT: &str = "$navigation-network-event:example.invalid";
/// Liveness bound for every positive wait in this file. A blocked AppActor
/// hangs indefinitely on the held mailbox, so a generous bound loses no
/// detection power, while the former 250 ms budget failed on loaded 2-core CI
/// runners whenever the scheduler starved the test runtime.
const LIVENESS_DEADLINE: Duration = Duration::from_secs(10);

fn request(sequence: u64) -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(1060),
        sequence,
    }
}

fn dm_room(room_id: &str) -> RoomSummary {
    RoomSummary {
        is_dm: true,
        dm_user_ids: vec!["@peer:example.invalid".to_owned()],
        ..unread_diagnostic_room(room_id)
    }
}

/// Ready session on Home with an ordinary room selected, a DM, and one Space
/// whose remembered room is `SPACE_ROOM`.
fn navigation_state() -> AppState {
    let mut space_room = unread_diagnostic_room(SPACE_ROOM);
    space_room.parent_space_ids = vec![SPACE.to_owned()];
    let mut state = AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: USER.to_owned(),
            device_id: "SYNTHETIC".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        rooms: vec![
            unread_diagnostic_room(ROOM_A),
            unread_diagnostic_room(ROOM_B),
            unread_diagnostic_room(ROOM_C),
            dm_room(DM),
            space_room,
        ],
        spaces: vec![
            koushi_state::SpaceSummary {
                space_id: SPACE.to_owned(),
                raw_name: None,
                display_name: "Synthetic space".to_owned(),
                avatar: None,
                join_rule: None,
                child_room_ids: vec![SPACE_ROOM.to_owned()],
                parent_side_child_room_ids: vec![SPACE_ROOM.to_owned()],
            },
            koushi_state::SpaceSummary {
                space_id: EMPTY_SPACE.to_owned(),
                raw_name: None,
                display_name: "Synthetic empty space".to_owned(),
                avatar: None,
                join_rule: None,
                child_room_ids: Vec::new(),
                parent_side_child_room_ids: Vec::new(),
            },
        ],
        ..AppState::default()
    };
    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: ROOM_A.to_owned(),
        },
    );
    assert_eq!(state.navigation.active_room_id.as_deref(), Some(ROOM_A));
    state
}

struct BlockedMailbox {
    command_tx: mpsc::Sender<CoreCommandEnvelope>,
    action_tx: mpsc::Sender<Vec<AppAction>>,
    event_rx: broadcast::Receiver<CoreEvent>,
    snapshot_rx: watch::Receiver<VersionedAppStateSnapshot>,
    navigation_projection_rx: watch::Receiver<crate::timeline::NavigationProjectionDemand>,
    account_rx: mpsc::Receiver<AccountMessage>,
    account_actor: AccountActorHandle,
    initial: AppState,
    actor_task: executor::JoinHandle<()>,
    _data_dir: tempfile::TempDir,
    _event_navigation_prepared_tx: mpsc::UnboundedSender<EventNavigationPrepared>,
    _focused_projection_tx: mpsc::UnboundedSender<FocusedProjectionCommitted>,
}

impl BlockedMailbox {
    /// Start an AppActor whose one-slot AccountActor mailbox is already full
    /// and is never drained by the test until [`Self::finish`].
    async fn start(state: AppState) -> Self {
        Self::start_with(state, |_| {}).await
    }

    /// Like [`Self::start`], with white-box AppActor ownership installed
    /// before the actor runs (for an event navigation still in flight).
    async fn start_with(state: AppState, prepare: impl FnOnce(&mut AppActor)) -> Self {
        let data_dir = tempfile::tempdir().expect("runtime data directory");
        let (
            actor,
            command_tx,
            action_tx,
            account_rx,
            event_rx,
            snapshot_rx,
            navigation_projection_rx,
            event_navigation_prepared_tx,
            focused_projection_tx,
        ) = app_actor_fixture_with_account_capacity(data_dir.path(), state.clone(), 1);
        let mut actor = actor;
        prepare(&mut actor);
        assert!(
            actor
                .account_actor
                .send(AccountMessage::CancelActivityResolution)
                .await,
            "fill the AccountActor mailbox"
        );
        let account_actor = actor.account_actor.clone();
        let actor_task = executor::spawn(async move {
            let _ = actor.run().await;
        });
        Self {
            command_tx,
            action_tx,
            event_rx,
            snapshot_rx,
            navigation_projection_rx,
            account_rx,
            account_actor,
            initial: state,
            actor_task,
            _data_dir: data_dir,
            _event_navigation_prepared_tx: event_navigation_prepared_tx,
            _focused_projection_tx: focused_projection_tx,
        }
    }

    async fn submit(&self, command: CoreCommand) -> oneshot::Receiver<CoreCommandAdmission> {
        let (admission, admitted) = oneshot::channel();
        executor::timeout(
            LIVENESS_DEADLINE,
            self.command_tx.send(CoreCommandEnvelope::Public {
                command,
                composer_permit: None,
                admission: Some(admission),
            }),
        )
        .await
        .expect("AppActor command ingress must not wait for the AccountActor")
        .expect("AppActor command ingress remains open");
        admitted
    }

    async fn select_room(&self, request_id: RequestId, room_id: &str) {
        let _admitted = self
            .submit(CoreCommand::Room(RoomCommand::SelectRoom {
                request_id,
                room_id: room_id.to_owned(),
            }))
            .await;
    }

    /// Collect the terminal outcome for every request, asserting each arrives
    /// exactly once and only after the state generation it names was
    /// published. The returned state is the navigation/timeline the WebView
    /// had received when that terminal arrived, rebuilt from ordered deltas.
    async fn terminals(&mut self, requests: &[RequestId]) -> Vec<(IntentOutcome, AppState)> {
        let mut received = self.initial.clone();
        let mut received_generation = 0;
        let mut outcomes: HashMap<RequestId, (IntentOutcome, AppState)> = HashMap::new();
        executor::timeout(LIVENESS_DEADLINE, async {
            while outcomes.len() < requests.len() {
                match self
                    .event_rx
                    .recv()
                    .await
                    .expect("event stream remains open")
                {
                    CoreEvent::StateDelta(delta) => {
                        assert!(delta.generation > received_generation);
                        received_generation = delta.generation;
                        if let Some(navigation) = delta.changed.navigation {
                            received.navigation = navigation;
                        }
                        if let Some(timeline) = delta.changed.timeline {
                            received.timeline = timeline;
                        }
                    }
                    CoreEvent::IntentLifecycle {
                        request_id,
                        outcome,
                        published_generation,
                    } if requests.contains(&request_id) => {
                        assert!(
                            published_generation <= received_generation,
                            "terminal for {request_id:?} preceded its state publication"
                        );
                        assert!(
                            outcomes
                                .insert(request_id, (outcome, received.clone()))
                                .is_none(),
                            "request {request_id:?} settled twice"
                        );
                    }
                    _ => {}
                }
            }
        })
        .await
        .expect("room selection must settle while the AccountActor mailbox is full");
        requests
            .iter()
            .map(|request_id| outcomes.remove(request_id).expect("settled request"))
            .collect()
    }

    async fn wait_for_snapshot(&mut self, predicate: impl Fn(&AppState) -> bool) -> AppState {
        executor::timeout(LIVENESS_DEADLINE, async {
            loop {
                let state = self.snapshot_rx.borrow_and_update().state.clone();
                if predicate(&state) {
                    break state;
                }
                self.snapshot_rx
                    .changed()
                    .await
                    .expect("snapshot channel remains open");
            }
        })
        .await
        .expect("navigation must publish while the AccountActor mailbox is full")
    }

    /// The retained post-commit enrichment demand, as (room, space).
    fn enrichment(&self) -> Option<(u64, Option<String>, Option<String>)> {
        self.account_actor
            .latest_navigation_enrichment()
            .map(|demand| {
                assert_eq!(
                    demand.session_key.user_id, USER,
                    "enrichment is fenced to the committing session"
                );
                (
                    demand.generation,
                    demand.active_room_id,
                    demand.active_space_id,
                )
            })
    }

    fn retained_projection_key(&mut self) -> Option<TimelineKey> {
        self.navigation_projection_rx
            .borrow_and_update()
            .room
            .as_ref()
            .map(|intent| intent.key.clone())
    }

    /// Wait until every previously sent action batch has been fully reduced:
    /// the one-slot action channel only accepts a second empty batch after the
    /// loop has taken the first, which follows the complete earlier batch.
    async fn drain_action_batches(&self) {
        for _ in 0..2 {
            executor::timeout(LIVENESS_DEADLINE, self.action_tx.send(Vec::new()))
                .await
                .expect("action ingress must not wait for the AccountActor")
                .expect("action ingress remains open");
        }
    }

    /// The fill message is still queued: nothing drained the mailbox, so the
    /// assertions above really ran while it was full.
    fn finish(mut self) {
        self.actor_task.abort();
        assert!(
            matches!(
                self.account_rx.try_recv(),
                Ok(AccountMessage::CancelActivityResolution)
            ),
            "the AccountActor mailbox remained full throughout navigation"
        );
    }
}

fn room_key(room_id: &str) -> TimelineKey {
    TimelineKey::room(AccountKey(USER.to_owned()), room_id)
}

#[tokio::test]
async fn navigation_network_dm_selection_commits_while_account_mailbox_is_full() {
    let mut harness = BlockedMailbox::start(navigation_state()).await;
    harness.select_room(request(1), DM).await;

    let [(outcome, state)] = harness
        .terminals(&[request(1)])
        .await
        .try_into()
        .ok()
        .unwrap();
    assert_eq!(outcome, IntentOutcome::Committed);
    assert_eq!(state.navigation.active_room_id.as_deref(), Some(DM));
    assert_eq!(state.timeline.room_id.as_deref(), Some(DM));
    assert_eq!(harness.retained_projection_key(), Some(room_key(DM)));
    // Pinned-event refresh is scheduled after the commit, not awaited by it.
    assert_eq!(harness.enrichment(), Some((1, Some(DM.to_owned()), None)));
    harness.finish();
}

#[tokio::test]
async fn navigation_network_room_selection_commits_while_account_mailbox_is_full() {
    let mut harness = BlockedMailbox::start(navigation_state()).await;
    harness.select_room(request(1), ROOM_B).await;

    let [(outcome, state)] = harness
        .terminals(&[request(1)])
        .await
        .try_into()
        .ok()
        .unwrap();
    assert_eq!(outcome, IntentOutcome::Committed);
    assert_eq!(state.navigation.active_room_id.as_deref(), Some(ROOM_B));
    assert_eq!(state.timeline.room_id.as_deref(), Some(ROOM_B));
    assert_eq!(harness.retained_projection_key(), Some(room_key(ROOM_B)));
    harness.finish();
}

#[tokio::test]
async fn navigation_network_space_selection_commits_while_account_mailbox_is_full() {
    let mut harness = BlockedMailbox::start(navigation_state()).await;
    let admitted = harness
        .submit(CoreCommand::Room(RoomCommand::SelectSpace {
            request_id: request(1),
            space_id: Some(SPACE.to_owned()),
        }))
        .await;

    let state = harness
        .wait_for_snapshot(|state| {
            state.navigation.active_space_id.as_deref() == Some(SPACE)
                && state.navigation.active_room_id.as_deref() == Some(SPACE_ROOM)
        })
        .await;
    assert_eq!(state.timeline.room_id.as_deref(), Some(SPACE_ROOM));
    let admission = executor::timeout(LIVENESS_DEADLINE, admitted)
        .await
        .expect("Space selection admission must not wait for the AccountActor")
        .expect("admission sender retained");
    assert!(admission.admitted_generation > 0);
    assert_eq!(
        harness.retained_projection_key(),
        Some(room_key(SPACE_ROOM))
    );
    // Member hydration and the restored room's pins follow the commit.
    assert_eq!(
        harness.enrichment(),
        Some((1, Some(SPACE_ROOM.to_owned()), Some(SPACE.to_owned())))
    );

    // Returning Home clears the Space-restored room without member hydration.
    let _admitted = harness
        .submit(CoreCommand::Room(RoomCommand::SelectSpace {
            request_id: request(2),
            space_id: None,
        }))
        .await;
    harness
        .wait_for_snapshot(|state| {
            state.navigation.active_space_id.is_none() && state.navigation.active_room_id.is_none()
        })
        .await;
    // Home is retained as current demand too, superseding the Space.
    assert_eq!(harness.enrichment(), Some((2, None, None)));
    harness.finish();
}

#[tokio::test]
async fn navigation_network_rapid_selection_leaves_last_room_authoritative() {
    let mut harness = BlockedMailbox::start(navigation_state()).await;
    harness.select_room(request(1), ROOM_B).await;
    harness.select_room(request(2), ROOM_C).await;
    harness.select_room(request(3), DM).await;

    let terminals = harness
        .terminals(&[request(1), request(2), request(3)])
        .await;
    for ((outcome, state), room_id) in terminals.iter().zip([ROOM_B, ROOM_C, DM]) {
        // Each terminal follows the publication of its own committed room,
        // or reports that a later selection in the same batch replaced it.
        match outcome {
            IntentOutcome::Committed => {
                assert_eq!(state.navigation.active_room_id.as_deref(), Some(room_id));
            }
            IntentOutcome::FailedNoOp(IntentNoOpReason::Superseded) => {
                assert_ne!(room_id, DM, "the last selection cannot be superseded");
            }
            outcome => panic!("unexpected selection outcome {outcome:?}"),
        }
    }
    assert_eq!(terminals[2].0, IntentOutcome::Committed);

    // A late actor projection of an earlier selection has no request owner
    // left and must not restore the older room.
    executor::timeout(
        LIVENESS_DEADLINE,
        harness.action_tx.send(vec![AppAction::SelectRoom {
            room_id: ROOM_B.to_owned(),
        }]),
    )
    .await
    .expect("action ingress")
    .expect("action ingress remains open");
    harness.drain_action_batches().await;
    let state = harness.snapshot_rx.borrow().state.clone();
    assert_eq!(state.navigation.active_room_id.as_deref(), Some(DM));
    assert_eq!(state.timeline.room_id.as_deref(), Some(DM));
    assert_eq!(harness.retained_projection_key(), Some(room_key(DM)));
    // Only the latest selection remains as enrichment demand.
    assert_eq!(harness.enrichment(), Some((3, Some(DM.to_owned()), None)));
    harness.finish();
}

#[tokio::test]
async fn navigation_network_selection_from_home_commits_while_account_mailbox_is_full() {
    let mut state = navigation_state();
    reduce(&mut state, AppAction::SelectSpace { space_id: None });
    state.navigation.active_room_id = None;
    state.timeline.room_id = None;
    let mut harness = BlockedMailbox::start(state).await;
    harness.select_room(request(1), SPACE_ROOM).await;

    let [(outcome, state)] = harness
        .terminals(&[request(1)])
        .await
        .try_into()
        .ok()
        .unwrap();
    assert_eq!(outcome, IntentOutcome::Committed);
    assert_eq!(state.navigation.active_room_id.as_deref(), Some(SPACE_ROOM));
    // Selecting a Space child from Home moves navigation into its Space.
    assert_eq!(state.navigation.active_space_id.as_deref(), Some(SPACE));
    harness.finish();
}

#[tokio::test]
async fn navigation_network_selection_from_activity_commits_while_account_mailbox_is_full() {
    let mut state = navigation_state();
    reduce(&mut state, AppAction::ActivityOpened { request_id: 1 });
    assert!(!matches!(state.activity, ActivityState::Closed { .. }));
    let mut harness = BlockedMailbox::start(state).await;
    harness.select_room(request(1), ROOM_B).await;
    // A second selection proves that leaving Activity did not leave the
    // AppActor loop waiting on AccountActor cleanup admission.
    harness.select_room(request(2), ROOM_C).await;

    let terminals = harness.terminals(&[request(1), request(2)]).await;
    assert_eq!(terminals[1].0, IntentOutcome::Committed);
    assert_eq!(
        terminals[1].1.navigation.active_room_id.as_deref(),
        Some(ROOM_C)
    );
    harness.finish();
}

#[tokio::test]
async fn navigation_network_already_active_and_unknown_rooms_keep_their_outcomes() {
    let mut harness = BlockedMailbox::start(navigation_state()).await;
    harness.select_room(request(1), ROOM_A).await;
    harness
        .select_room(request(2), "!navigation-network-unknown:example.invalid")
        .await;

    let terminals = harness.terminals(&[request(1), request(2)]).await;
    assert_eq!(
        terminals[0].0,
        IntentOutcome::BenignNoOp(IntentNoOpReason::AlreadyActive)
    );
    assert_eq!(
        terminals[1].0,
        IntentOutcome::FailedNoOp(IntentNoOpReason::RoomNotInState)
    );
    assert_eq!(
        harness
            .snapshot_rx
            .borrow()
            .state
            .navigation
            .active_room_id
            .as_deref(),
        Some(ROOM_A)
    );
    // Re-selecting the active room still refreshes its pins; an unknown room
    // schedules nothing.
    assert_eq!(
        harness.enrichment(),
        Some((1, Some(ROOM_A.to_owned()), None))
    );
    harness.finish();
}

fn focused_key(room_id: &str) -> TimelineKey {
    TimelineKey {
        account_key: AccountKey(USER.to_owned()),
        kind: TimelineKind::Focused {
            room_id: room_id.to_owned(),
            event_id: EVENT.to_owned(),
        },
    }
}

/// `ROOM_A` anchored at `EVENT`: the focused timeline is already open.
fn anchored_state() -> AppState {
    let mut state = navigation_state();
    reduce(
        &mut state,
        AppAction::OpenFocusedContext {
            room_id: ROOM_A.to_owned(),
            event_id: EVENT.to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::EnterAnchoredTimeline {
            room_id: ROOM_A.to_owned(),
            event_id: EVENT.to_owned(),
        },
    );
    assert!(state.navigation.main_timeline_anchor.is_some());
    state
}

/// Start from a focused context whose timeline the manager currently owns.
async fn start_focused(state: AppState, prepare: impl FnOnce(&mut AppActor)) -> BlockedMailbox {
    BlockedMailbox::start_with(state, |actor| {
        actor
            .account_actor
            .admit_focused_foreground(Some(focused_key(ROOM_A)));
        prepare(actor);
    })
    .await
}

/// Two selections in a row: the second proves no post-commit focused cleanup
/// left the AppActor loop waiting on the full AccountActor mailbox. The old
/// focused owner is retired through the retained desired foreground.
async fn assert_two_selections_commit(harness: &mut BlockedMailbox) {
    harness.select_room(request(11), ROOM_B).await;
    harness.select_room(request(12), ROOM_C).await;
    let terminals = harness.terminals(&[request(11), request(12)]).await;
    assert_eq!(terminals[1].0, IntentOutcome::Committed);
    assert_eq!(
        terminals[1].1.navigation.active_room_id.as_deref(),
        Some(ROOM_C)
    );
    assert_eq!(terminals[1].1.timeline.room_id.as_deref(), Some(ROOM_C));
    assert_eq!(harness.navigation_projection_rx.borrow().focused, None);
    assert_eq!(harness.retained_projection_key(), Some(room_key(ROOM_C)));
}

#[tokio::test]
async fn navigation_network_selection_from_opening_focused_context_commits() {
    let mut harness = start_opening_focused().await;
    assert_two_selections_commit(&mut harness).await;
    harness.finish();
}

/// An event navigation into `ROOM_A` whose focused timeline is still opening.
async fn start_opening_focused() -> BlockedMailbox {
    let generation = 7;
    let mut state = navigation_state();
    state.focused_context = koushi_state::FocusedContextState::Open {
        room_id: ROOM_A.to_owned(),
        event_id: EVENT.to_owned(),
        is_subscribed: true,
    };
    state.navigation.event_navigation = koushi_state::EventNavigationState::Opening {
        generation,
        source: koushi_state::EventNavigationSource::Activity,
    };
    start_focused(state, |actor| {
        actor.pending_event_navigation = Some(PendingEventNavigation {
            request_id: request(1),
            select_request_id: request(2),
            room_id: ROOM_A.to_owned(),
            event_id: EVENT.to_owned(),
            source: koushi_state::EventNavigationSource::Activity,
            generation,
        });
        actor.pending_focused_navigation = Some(PendingFocusedNavigation {
            projection_request_id: request(1),
            key: focused_key(ROOM_A),
            room_id: ROOM_A.to_owned(),
            event_id: EVENT.to_owned(),
            allow_live_fallback: true,
            generation: Some(TimelineGeneration(generation)),
        });
    })
    .await
}

#[tokio::test]
async fn navigation_network_home_and_empty_space_from_opening_focused_context_commit() {
    for space_id in [None, Some(EMPTY_SPACE.to_owned())] {
        let mut harness = start_opening_focused().await;
        let _admitted = harness
            .submit(CoreCommand::Room(RoomCommand::SelectSpace {
                request_id: request(10),
                space_id: space_id.clone(),
            }))
            .await;
        harness
            .wait_for_snapshot(|state| {
                state.navigation.active_space_id == space_id
                    && state.navigation.active_room_id.is_none()
            })
            .await;
        assert_two_selections_commit(&mut harness).await;
        harness.finish();
    }
}

#[tokio::test]
async fn navigation_network_selection_from_anchored_focused_context_commits() {
    let mut harness = start_focused(anchored_state(), |_| {}).await;
    assert_two_selections_commit(&mut harness).await;
    harness.finish();
}

#[tokio::test]
async fn navigation_network_home_from_anchored_focused_context_commits() {
    let mut harness = start_focused(anchored_state(), |_| {}).await;
    let _admitted = harness
        .submit(CoreCommand::Room(RoomCommand::SelectSpace {
            request_id: request(10),
            space_id: None,
        }))
        .await;
    assert_two_selections_commit(&mut harness).await;
    harness.finish();
}

#[tokio::test]
async fn navigation_network_empty_space_from_anchored_focused_context_commits() {
    let mut harness = start_focused(anchored_state(), |_| {}).await;
    let _admitted = harness
        .submit(CoreCommand::Room(RoomCommand::SelectSpace {
            request_id: request(10),
            space_id: Some(EMPTY_SPACE.to_owned()),
        }))
        .await;
    harness
        .wait_for_snapshot(|state| {
            state.navigation.active_space_id.as_deref() == Some(EMPTY_SPACE)
                && state.navigation.active_room_id.is_none()
        })
        .await;
    assert_two_selections_commit(&mut harness).await;
    harness.finish();
}

#[tokio::test]
async fn navigation_network_event_navigation_room_selection_commits_while_account_mailbox_is_full()
{
    // #1060 step 6: navigating to an event in another room selects that room
    // through the same local commit; only the later event lookup may wait.
    for start in [navigation_state(), anchored_state()] {
        let mut harness = start_focused(start, |_| {}).await;
        let _admitted = harness
            .submit(CoreCommand::App(AppCommand::NavigateToEvent {
                request_id: request(20),
                room_id: ROOM_B.to_owned(),
                event_id: EVENT.to_owned(),
                source: koushi_state::EventNavigationSource::Activity,
                missing_target_policy:
                    koushi_protocol::command::EventNavigationMissingTargetPolicy::LiveFallback,
            }))
            .await;
        let state = harness
            .wait_for_snapshot(|state| {
                state.navigation.active_room_id.as_deref() == Some(ROOM_B)
                    && state.timeline.room_id.as_deref() == Some(ROOM_B)
                    && matches!(
                        state.navigation.event_navigation,
                        koushi_state::EventNavigationState::Opening { .. }
                    )
            })
            .await;
        assert_eq!(
            state.focused_context,
            koushi_state::FocusedContextState::Closed
        );
        assert_eq!(harness.retained_projection_key(), Some(room_key(ROOM_B)));
        assert_eq!(harness.navigation_projection_rx.borrow().focused, None);
        // A user selection that supersedes the in-flight event navigation also
        // commits without waiting.
        harness.select_room(request(21), ROOM_C).await;
        let [(outcome, state)] = harness
            .terminals(&[request(21)])
            .await
            .try_into()
            .ok()
            .unwrap();
        assert_eq!(outcome, IntentOutcome::Committed);
        assert_eq!(state.navigation.active_room_id.as_deref(), Some(ROOM_C));
        harness.finish();
    }
}

const UNRESOLVED_DM: &str = "!navigation-network-unresolved-dm:example.invalid";
const UNRESOLVED_DM_2: &str = "!navigation-network-unresolved-dm-2:example.invalid";

/// Activity is open on `navigation_state()`; nothing needs resolution yet.
/// The search crawler is paused so a live room-list update dispatches only
/// the Activity resolution.
fn activity_open_state() -> AppState {
    let mut state = AppState {
        activity: ActivityState::Open {
            active_tab: koushi_state::ActivityTab::Unread,
            recent: koushi_state::ActivityStream::default(),
            unread: koushi_state::ActivityStream::default(),
            mark_read: Default::default(),
        },
        ..navigation_state()
    };
    state.settings.values.search_crawler.speed = koushi_state::SearchCrawlerSpeed::Paused;
    state
}

fn unread_resolution(state: &AppState) -> Option<koushi_state::ActivityResolutionState> {
    match &state.activity {
        ActivityState::Open { unread, .. } => Some(unread.resolution),
        _ => None,
    }
}

impl BlockedMailbox {
    /// Apply a live room-list update that adds notified unread DMs without a
    /// resolvable latest event, and wait until its rooms are published.
    async fn live_unresolved_dms(&mut self, room_ids: &[&str]) {
        let mut rooms = self.initial.rooms.clone();
        rooms.extend(
            room_ids
                .iter()
                .enumerate()
                .map(|(index, room_id)| live_unresolved_activity_room(room_id, 200 + index as u64)),
        );
        executor::timeout(
            LIVENESS_DEADLINE,
            self.action_tx.send(vec![AppAction::RoomListUpdated {
                spaces: self.initial.spaces.clone(),
                rooms,
            }]),
        )
        .await
        .expect("action ingress must not wait for the AccountActor")
        .expect("action ingress remains open");
        self.wait_for_snapshot(|state| {
            room_ids
                .iter()
                .all(|room_id| state.rooms.iter().any(|room| room.room_id == *room_id))
        })
        .await;
    }

    /// Wait until open Activity reports a resolution in flight, as
    /// (generation, unresolved room count).
    async fn resolving_generation(&mut self) -> (u64, u32) {
        let state = self
            .wait_for_snapshot(|state| {
                matches!(
                    unread_resolution(state),
                    Some(koushi_state::ActivityResolutionState::Resolving { .. })
                )
            })
            .await;
        match unread_resolution(&state) {
            Some(koushi_state::ActivityResolutionState::Resolving {
                generation,
                unresolved_room_count,
            }) => (generation, unresolved_room_count),
            other => panic!("unexpected resolution state {other:?}"),
        }
    }
}

#[tokio::test]
async fn navigation_network_selection_commits_while_live_activity_resolution_waits_for_the_mailbox()
{
    // #1060 audit: an unresolved notified DM arriving while Activity is open
    // must not hold the AppActor loop in the ResolveActivity dispatch.
    let mut harness = BlockedMailbox::start(activity_open_state()).await;
    harness.live_unresolved_dms(&[UNRESOLVED_DM]).await;
    harness.select_room(request(1), ROOM_B).await;

    let [(outcome, state)] = harness
        .terminals(&[request(1)])
        .await
        .try_into()
        .ok()
        .unwrap();
    assert_eq!(outcome, IntentOutcome::Committed);
    assert_eq!(state.navigation.active_room_id.as_deref(), Some(ROOM_B));
    harness.finish();
}

#[tokio::test]
async fn navigation_network_deferred_activity_resolution_is_delivered_once_capacity_frees() {
    let mut harness = BlockedMailbox::start(activity_open_state()).await;
    harness.live_unresolved_dms(&[UNRESOLVED_DM]).await;
    let (generation, unresolved_room_count) = harness.resolving_generation().await;
    // A later live update while the dispatch is still deferred neither
    // restarts nor duplicates the in-flight generation.
    harness
        .live_unresolved_dms(&[UNRESOLVED_DM, UNRESOLVED_DM_2])
        .await;
    harness.drain_action_batches().await;

    // Free the mailbox: the fill message, then exactly the deferred request.
    assert!(matches!(
        harness.account_rx.recv().await,
        Some(AccountMessage::CancelActivityResolution)
    ));
    let (delivered_generation, rooms) =
        next_activity_resolution_request(&mut harness.account_rx, LIVENESS_DEADLINE)
            .await
            .expect("the deferred resolution is delivered once the mailbox has capacity");
    assert_eq!(delivered_generation, generation);
    // The fixture's other unread rooms are candidates too; the DM that
    // arrived while this generation was in flight is left for the next one.
    assert!(rooms.contains(UNRESOLVED_DM));
    assert!(!rooms.contains(UNRESOLVED_DM_2));
    assert_eq!(rooms.len(), unresolved_room_count as usize);
    assert!(
        next_activity_resolution_request(&mut harness.account_rx, Duration::from_millis(100))
            .await
            .is_none(),
        "one generation is dispatched exactly once"
    );
    assert_eq!(
        unread_resolution(&harness.snapshot_rx.borrow().state),
        Some(koushi_state::ActivityResolutionState::Resolving {
            generation,
            unresolved_room_count,
        })
    );
    harness.actor_task.abort();
}

#[tokio::test]
async fn navigation_network_deferred_activity_resolution_fails_retryably_when_the_mailbox_closes() {
    let mut harness = BlockedMailbox::start(activity_open_state()).await;
    harness.live_unresolved_dms(&[UNRESOLVED_DM]).await;
    let (generation, unresolved_room_count) = harness.resolving_generation().await;
    // The AccountActor goes away while the dispatch is deferred.
    let (_closed_tx, closed_rx) = mpsc::channel(1);
    drop(std::mem::replace(&mut harness.account_rx, closed_rx));

    let state = harness
        .wait_for_snapshot(|state| {
            matches!(
                unread_resolution(state),
                Some(koushi_state::ActivityResolutionState::Failed { .. })
            )
        })
        .await;
    assert_eq!(
        unread_resolution(&state),
        Some(koushi_state::ActivityResolutionState::Failed {
            generation,
            unresolved_room_count,
            failure_kind: koushi_state::OperationFailureKind::Sdk,
        })
    );
    // An undeliverable generation settles once; it does not loop.
    harness.drain_action_batches().await;
    assert_eq!(
        unread_resolution(&harness.snapshot_rx.borrow().state),
        unread_resolution(&state)
    );

    // The failure stays retryable: an explicit retry starts a new generation,
    // which fails again because the mailbox is still closed.
    let _admitted = harness
        .submit(CoreCommand::App(AppCommand::RetryActivityResolution {
            request_id: request(30),
        }))
        .await;
    let retried = harness
        .wait_for_snapshot(|state| {
            matches!(
                unread_resolution(state),
                Some(koushi_state::ActivityResolutionState::Failed { generation: retried, .. })
                    if retried > generation
            )
        })
        .await;
    assert!(matches!(
        unread_resolution(&retried),
        Some(koushi_state::ActivityResolutionState::Failed { .. })
    ));
    harness.actor_task.abort();
}

/// The next crawler room-availability notification, skipping other messages.
async fn next_crawler_rooms(
    account_rx: &mut mpsc::Receiver<AccountMessage>,
    within: Duration,
) -> Option<Vec<String>> {
    executor::timeout(within, async {
        loop {
            if let AccountMessage::NotifySearchCrawlerRoomsAvailable { room_ids, .. } =
                account_rx.recv().await?
            {
                return Some(room_ids);
            }
        }
    })
    .await
    .ok()
    .flatten()
}

#[tokio::test]
async fn navigation_network_selection_commits_after_a_live_room_list_update_with_the_crawler_running()
 {
    // #1060 audit: every live room-list update notifies the search crawler;
    // that notification must not hold the AppActor loop either.
    let state = navigation_state();
    assert_ne!(
        state.settings.values.search_crawler.speed,
        koushi_state::SearchCrawlerSpeed::Paused
    );
    let mut harness = BlockedMailbox::start(state).await;
    harness.live_unresolved_dms(&[UNRESOLVED_DM]).await;
    harness.select_room(request(1), ROOM_B).await;

    let [(outcome, state)] = harness
        .terminals(&[request(1)])
        .await
        .try_into()
        .ok()
        .unwrap();
    assert_eq!(outcome, IntentOutcome::Committed);
    assert_eq!(state.navigation.active_room_id.as_deref(), Some(ROOM_B));
    harness.finish();
}

#[tokio::test]
async fn navigation_network_deferred_crawler_notification_delivers_only_the_latest_rooms() {
    let mut harness = BlockedMailbox::start(navigation_state()).await;
    harness.live_unresolved_dms(&[UNRESOLVED_DM]).await;
    harness
        .live_unresolved_dms(&[UNRESOLVED_DM, UNRESOLVED_DM_2])
        .await;
    harness.drain_action_batches().await;

    assert!(matches!(
        harness.account_rx.recv().await,
        Some(AccountMessage::CancelActivityResolution)
    ));
    let rooms = next_crawler_rooms(&mut harness.account_rx, LIVENESS_DEADLINE)
        .await
        .expect("the deferred crawler notification is delivered once capacity frees");
    assert!(rooms.iter().any(|room_id| room_id == UNRESOLVED_DM_2));
    assert!(
        next_crawler_rooms(&mut harness.account_rx, Duration::from_millis(100))
            .await
            .is_none(),
        "deferred crawler notifications coalesce to the latest payload"
    );
    harness.actor_task.abort();
}

#[tokio::test]
async fn navigation_network_deferred_crawler_notification_is_dropped_after_the_session_ends() {
    let mut harness = BlockedMailbox::start(navigation_state()).await;
    harness.live_unresolved_dms(&[UNRESOLVED_DM]).await;
    executor::timeout(
        LIVENESS_DEADLINE,
        harness.action_tx.send(vec![AppAction::LogoutFinished]),
    )
    .await
    .expect("action ingress must not wait for the AccountActor")
    .expect("action ingress remains open");
    harness
        .wait_for_snapshot(|state| !matches!(state.session, SessionState::Ready(_)))
        .await;

    // Free the mailbox: the signed-out session's rooms never reach a crawler.
    assert!(matches!(
        harness.account_rx.recv().await,
        Some(AccountMessage::CancelActivityResolution)
    ));
    assert!(
        next_crawler_rooms(&mut harness.account_rx, Duration::from_millis(200))
            .await
            .is_none(),
        "a deferred crawler notification is fenced to its session"
    );
    harness.actor_task.abort();
}

#[tokio::test]
async fn navigation_network_selection_commits_after_a_live_leave_reloads_space_children() {
    // #1060 audit: leaving a child of the selected Space asks for a fresh
    // children projection inside the action-batch commit; that dispatch must
    // not hold the AppActor loop either, and is delivered once a slot frees.
    let mut state = navigation_state();
    state.settings.values.search_crawler.speed = koushi_state::SearchCrawlerSpeed::Paused;
    reduce(
        &mut state,
        AppAction::SelectSpace {
            space_id: Some(SPACE.to_owned()),
        },
    );
    assert_eq!(
        state.space_children.selected_space_id.as_deref(),
        Some(SPACE)
    );
    let mut harness = BlockedMailbox::start(state).await;
    executor::timeout(
        LIVENESS_DEADLINE,
        harness.action_tx.send(vec![AppAction::RoomLeftLocally {
            room_id: SPACE_ROOM.to_owned(),
        }]),
    )
    .await
    .expect("action ingress must not wait for the AccountActor")
    .expect("action ingress remains open");
    let left = harness
        .wait_for_snapshot(|state| {
            state.space_children.load == koushi_state::SpaceChildrenLoadState::Loading
        })
        .await;
    // A DM is global: selecting it keeps the Space and its pending reload.
    harness.select_room(request(1), DM).await;
    let [(outcome, selected)] = harness
        .terminals(&[request(1)])
        .await
        .try_into()
        .ok()
        .unwrap();
    assert_eq!(outcome, IntentOutcome::Committed);
    assert_eq!(selected.navigation.active_room_id.as_deref(), Some(DM));
    assert_eq!(selected.navigation.active_space_id.as_deref(), Some(SPACE));

    assert!(matches!(
        harness.account_rx.recv().await,
        Some(AccountMessage::CancelActivityResolution)
    ));
    let reload = executor::timeout(LIVENESS_DEADLINE, async {
        loop {
            if let Some(AccountMessage::RoomCommand(RoomCommand::LoadSpaceChildren {
                space_id,
                generation,
                ..
            })) = harness.account_rx.recv().await
            {
                return (space_id, generation);
            }
        }
    })
    .await
    .expect("the deferred reload is delivered once the mailbox has capacity");
    assert_eq!(reload, (SPACE.to_owned(), left.space_children.generation));
    harness.actor_task.abort();
}

type Observation = super::super::deferred_dispatch::DeferredDispatchObservation;

/// Start with the test-only deferred-dispatch observer installed. Each
/// observation follows one dispatch decision, so it is a causal fence for
/// "AppActor has handled this" without any sleep.
async fn start_observed(state: AppState) -> (BlockedMailbox, mpsc::UnboundedReceiver<Observation>) {
    let (observer, observations) = mpsc::unbounded_channel();
    let harness = BlockedMailbox::start_with(state, |actor| {
        actor.deferred_account_dispatch.observer = Some(observer);
    })
    .await;
    (harness, observations)
}

/// The first observation matching `predicate`, under one absolute deadline.
async fn observation(
    observations: &mut mpsc::UnboundedReceiver<Observation>,
    predicate: impl Fn(&Observation) -> bool,
) -> Observation {
    executor::timeout(LIVENESS_DEADLINE, async {
        loop {
            let observed = observations.recv().await.expect("observer remains open");
            if predicate(&observed) {
                return observed;
            }
        }
    })
    .await
    .expect("AppActor makes the expected dispatch decision while the mailbox is full")
}

fn crawler_label(settings: &koushi_state::SearchCrawlerSettings) -> String {
    super::super::deferred_dispatch::crawler_settings_label(settings)
}

/// A live room-list update holds a crawler notification with the current
/// settings; then the user changes crawler settings while the mailbox is
/// still full. Returns the held lane after the change and the crawler-lane
/// messages delivered once the mailbox frees, up to the last notification.
async fn crawler_settings_change_while_deferred(
    change: impl FnOnce(&mut koushi_state::SearchCrawlerSettings),
) -> (Vec<String>, Vec<String>) {
    let (mut harness, mut observations) = start_observed(navigation_state()).await;
    let before = harness.initial.settings.values.search_crawler.clone();
    harness.live_unresolved_dms(&[UNRESOLVED_DM]).await;
    observation(&mut observations, |observed| {
        observed.crawler_lane == [crawler_label(&before)]
    })
    .await;

    let mut settings = before;
    change(&mut settings);
    let _admitted = harness
        .submit(CoreCommand::App(AppCommand::UpdateSettings {
            request_id: request(40),
            patch: koushi_state::SettingsPatch {
                search_crawler: Some(settings.clone()),
                ..Default::default()
            },
        }))
        .await;
    // AppActor is now inside the settings command, so the run loop cannot
    // deliver the held notification until the command returns. The command
    // itself waits on the full mailbox for its settings-policy broadcasts, so
    // no settings snapshot can be awaited while the mailbox is held.
    observation(&mut observations, |observed| observed.event == "command").await;

    // Free the mailbox and keep draining it. The command's crawler dispatch
    // joins the held lane, and the lane ends with its notification, so every
    // lane message has arrived once that one has.
    let expected_last = crawler_label(&settings);
    let delivered = drain_crawler_lane(&mut harness.account_rx, &expected_last).await;
    // The final authoritative check of what the command left held: its last
    // crawler dispatch decision, made before anything could be delivered.
    let held = std::iter::from_fn(|| observations.try_recv().ok())
        .filter(|observed| observed.event == "crawler")
        .last()
        .expect("the settings change made a crawler dispatch decision")
        .crawler_lane;
    harness.actor_task.abort();
    (held, delivered)
}

/// Drain the freed mailbox until the crawler lane's last notification,
/// returning every crawler-lane message in delivery order.
async fn drain_crawler_lane(
    account_rx: &mut mpsc::Receiver<AccountMessage>,
    expected_last: &str,
) -> Vec<String> {
    executor::timeout(LIVENESS_DEADLINE, async {
        let mut delivered = Vec::new();
        loop {
            match account_rx.recv().await.expect("mailbox remains open") {
                AccountMessage::NotifySearchCrawlerRoomsAvailable { settings, .. } => {
                    delivered.push(crawler_label(&settings));
                    if delivered.last().map(String::as_str) == Some(expected_last) {
                        return delivered;
                    }
                }
                AccountMessage::InvalidateSearchCrawlerCache => {
                    delivered.push("invalidate".to_owned());
                }
                AccountMessage::RebuildSearchIndex => delivered.push("rebuild".to_owned()),
                _ => {}
            }
        }
    })
    .await
    .expect("the held crawler lane is delivered once the mailbox has capacity")
}
#[tokio::test]
async fn navigation_network_crawler_pause_supersedes_a_deferred_notification() {
    let (held, delivered) = crawler_settings_change_while_deferred(|settings| {
        settings.speed = koushi_state::SearchCrawlerSpeed::Paused;
    })
    .await;
    // The stale active notification was superseded while held, and nothing
    // else of the lane is delivered.
    let paused = "notify:Paused:captions=true:filenames=true".to_owned();
    assert_eq!(held, std::slice::from_ref(&paused));
    assert_eq!(delivered, [paused]);
}

#[tokio::test]
async fn navigation_network_caption_opt_out_supersedes_a_deferred_notification() {
    let (held, delivered) = crawler_settings_change_while_deferred(|settings| {
        settings.include_media_captions = false;
    })
    .await;
    // The cache is invalidated before the re-crawl, which uses the opted-out
    // settings; the stale caption-enabled notification never follows.
    let expected = [
        "invalidate".to_owned(),
        "notify:Standard:captions=false:filenames=true".to_owned(),
    ];
    assert_eq!(held, expected);
    assert_eq!(delivered, expected);
}

#[tokio::test]
async fn navigation_network_selection_commits_after_a_live_batch_closes_activity() {
    // #1060 review: an actor-projected batch that closes open Activity cancels
    // its resolution; that cancel must not hold the AppActor loop either.
    let mut harness = BlockedMailbox::start(activity_open_state()).await;
    executor::timeout(
        LIVENESS_DEADLINE,
        harness.action_tx.send(vec![AppAction::ActivityClosed]),
    )
    .await
    .expect("action ingress must not wait for the AccountActor")
    .expect("action ingress remains open");
    harness
        .wait_for_snapshot(|state| matches!(state.activity, ActivityState::Closed { .. }))
        .await;
    // The loop is free: an internal action batch is taken right away.
    harness.drain_action_batches().await;

    assert!(matches!(
        harness.account_rx.recv().await,
        Some(AccountMessage::CancelActivityResolution)
    ));
    let cancel = executor::timeout(LIVENESS_DEADLINE, async {
        loop {
            if let Some(AccountMessage::CancelActivityResolution) = harness.account_rx.recv().await
            {
                return;
            }
        }
    })
    .await;
    assert!(
        cancel.is_ok(),
        "the deferred cancel is delivered once capacity frees"
    );
    harness.actor_task.abort();
}

#[tokio::test]
async fn navigation_network_user_reload_replaces_a_held_live_leave_reload() {
    let mut state = navigation_state();
    state.settings.values.search_crawler.speed = koushi_state::SearchCrawlerSpeed::Paused;
    reduce(
        &mut state,
        AppAction::SelectSpace {
            space_id: Some(SPACE.to_owned()),
        },
    );
    let (mut harness, mut observations) = start_observed(state).await;
    executor::timeout(
        LIVENESS_DEADLINE,
        harness.action_tx.send(vec![AppAction::RoomLeftLocally {
            room_id: SPACE_ROOM.to_owned(),
        }]),
    )
    .await
    .expect("action ingress must not wait for the AccountActor")
    .expect("action ingress remains open");
    let generation = observation(&mut observations, |observed| {
        observed.event == "space_children_reload"
    })
    .await
    .space_children_reload
    .expect("the live leave's reload is held");

    // The user asks for the same Space and generation while it is held; the
    // held reload is dropped as AppActor admits the user's.
    let _admitted = harness
        .submit(CoreCommand::Room(RoomCommand::LoadSpaceChildren {
            request_id: request(50),
            space_id: SPACE.to_owned(),
            generation,
        }))
        .await;
    let admitted = observation(&mut observations, |observed| {
        observed.event == "user_space_children_reload"
    })
    .await;
    assert_eq!(
        admitted.space_children_reload, None,
        "nothing else holds a /hierarchy request for this generation"
    );

    // Once the mailbox frees, the only reload for this generation is the
    // user's own request.
    assert!(matches!(
        harness.account_rx.recv().await,
        Some(AccountMessage::CancelActivityResolution)
    ));
    let reload = executor::timeout(LIVENESS_DEADLINE, async {
        loop {
            if let Some(AccountMessage::RoomCommand(RoomCommand::LoadSpaceChildren {
                request_id,
                generation: loaded,
                ..
            })) = harness.account_rx.recv().await
            {
                return (request_id, loaded);
            }
        }
    })
    .await
    .expect("the user's reload is forwarded once the mailbox has capacity");
    assert_eq!(reload, (request(50), generation));
    harness.actor_task.abort();
}

#[tokio::test]
async fn navigation_network_rebuild_keeps_a_held_paused_notification() {
    // #1060 review: a rebuild changes no crawler settings, and its reducer
    // re-notifies only while crawling is active. A held paused notification
    // must survive it, or an older active notification could restart
    // crawling while settings say paused.
    let mut state = navigation_state();
    state.settings.values.search_crawler.speed = koushi_state::SearchCrawlerSpeed::Paused;
    let paused = state.settings.values.search_crawler.clone();
    let (observer, mut observations) = mpsc::unbounded_channel();
    let mut harness = BlockedMailbox::start_with(state, |actor| {
        let session_key = super::super::navigation::navigation_session_key(&actor.state);
        actor
            .deferred_account_dispatch
            .hold_crawler_notification_for_test(session_key, paused.clone());
        actor.deferred_account_dispatch.observer = Some(observer);
    })
    .await;

    let _admitted = harness
        .submit(CoreCommand::App(AppCommand::RebuildSearchIndex {
            request_id: request(60),
        }))
        .await;
    let held = observation(&mut observations, |observed| observed.event == "crawler")
        .await
        .crawler_lane;
    let paused_label = crawler_label(&paused);
    assert_eq!(held, ["rebuild".to_owned(), paused_label.clone()]);

    assert!(matches!(
        harness.account_rx.recv().await,
        Some(AccountMessage::CancelActivityResolution)
    ));
    let delivered = drain_crawler_lane(&mut harness.account_rx, &paused_label).await;
    assert_eq!(delivered, ["rebuild".to_owned(), paused_label]);
    harness.actor_task.abort();
}
