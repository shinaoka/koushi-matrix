//! #1854: a navigation whose target is a reply inside a thread. Core routes that
//! target into the thread pane on the reply, because a main-pane focused
//! projection of it is thread-filtered by the SDK: the room messages around the
//! hit never arrive and the view's live tail is the thread's last reply.
//! Everything here is synthetic.

use super::*;
use koushi_protocol::event::IntentOutcome;
use koushi_state::{
    EventNavigationSource, EventNavigationState, FocusedContextState, ThreadOpenIntent,
    ThreadPaneState,
};

const ROOM: &str = "!search-thread-room:example.invalid";
const ROOT: &str = "$search-thread-root:example.invalid";
const TARGET: &str = "$search-thread-target:example.invalid";
const USER: &str = "@synthetic:example.invalid";

fn request(sequence: u64) -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(1854),
        sequence,
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

/// The thread pane's room, root, and open intent while it is opening or open.
fn thread_pane(state: &AppState) -> Option<(&str, &str, &ThreadOpenIntent)> {
    match &state.thread {
        ThreadPaneState::Opening {
            room_id,
            root_event_id,
            intent,
        }
        | ThreadPaneState::Open {
            room_id,
            root_event_id,
            intent,
            ..
        } => Some((room_id, root_event_id, intent)),
        ThreadPaneState::Closed | ThreadPaneState::Failed { .. } => None,
    }
}

struct Harness {
    command_tx: mpsc::Sender<CoreCommandEnvelope>,
    /// Kept alive: AppActor's action mailbox closes when every sender is
    /// dropped, and these tests inject no actions.
    _action_tx: mpsc::Sender<Vec<AppAction>>,
    snapshot_rx: watch::Receiver<VersionedAppStateSnapshot>,
    event_rx: broadcast::Receiver<CoreEvent>,
    /// The account stub's `Subscribe` keys, as the runtime sends them.
    subscribe_rx: mpsc::UnboundedReceiver<TimelineKey>,
    /// Every key drained from `subscribe_rx` so far.
    observed: Vec<TimelineKey>,
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

impl Harness {
    /// `thread_root_event_id` is what the event lookup reports for the target:
    /// `Some` when the target replies inside that thread, `None` when it is an
    /// ordinary room message.
    fn start(thread_root_event_id: Option<&'static str>) -> Self {
        let data_dir = tempfile::tempdir().expect("runtime data directory");
        let (
            actor,
            command_tx,
            action_tx,
            mut account_rx,
            event_rx,
            snapshot_rx,
            _navigation_projection_rx,
            _event_navigation_prepared_tx,
            _focused_projection_tx,
        ) = app_actor_event_navigation_fixture(data_dir.path(), selected_room_state());
        let (subscribe_tx, subscribe_rx) = mpsc::unbounded_channel();
        let account = tokio::spawn(async move {
            while let Some(message) = account_rx.recv().await {
                match message {
                    AccountMessage::EnsureRoomEventCached { response_tx, .. } => {
                        let _ = response_tx.send(crate::account::RoomEventLookupResult::Located {
                            thread_root_event_id: thread_root_event_id.map(ToOwned::to_owned),
                        });
                    }
                    AccountMessage::TimelineCommand(
                        koushi_protocol::command::TimelineCommand::Subscribe { key, .. },
                    )
                    | AccountMessage::TimelineCommandWithComposerFormatting {
                        command: koushi_protocol::command::TimelineCommand::Subscribe { key, .. },
                        ..
                    } => {
                        let _ = subscribe_tx.send(key);
                    }
                    _ => {}
                }
            }
        });
        let actor_task = tokio::spawn(async move {
            let _ = actor.run().await;
        });
        Harness {
            command_tx,
            _action_tx: action_tx,
            snapshot_rx,
            event_rx,
            subscribe_rx,
            observed: Vec::new(),
            tasks: vec![account, actor_task],
            _data_dir: data_dir,
        }
    }

    async fn navigate(&mut self, source: EventNavigationSource, request_id: RequestId) {
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
                    event_id: TARGET.to_owned(),
                    source,
                    missing_target_policy,
                }),
                composer_permit: None,
                admission: None,
            })
            .await
            .expect("navigation command");
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
                "{label}; last thread pane: {:?}, event navigation: {:?}, focused context: {:?}",
                self.snapshot_rx.borrow().state.thread,
                self.snapshot_rx.borrow().state.navigation.event_navigation,
                self.snapshot_rx.borrow().state.focused_context,
            ),
        }
    }

    /// The navigation's own terminal outcome, as published for its request.
    async fn wait_for_outcome(&mut self, request_id: RequestId) -> IntentOutcome {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match self.event_rx.recv().await {
                    Ok(CoreEvent::IntentLifecycle {
                        request_id: settled,
                        outcome,
                        ..
                    }) if settled == request_id => return outcome,
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(error) => panic!("event stream closed: {error:?}"),
                }
            }
        })
        .await
        .expect("the navigation must publish one terminal outcome")
    }

    /// Record subscriptions until one matches, so an assertion never races the
    /// effects that follow the published navigation terminal.
    async fn wait_for_subscription(
        &mut self,
        label: &str,
        matches: impl Fn(&TimelineKind) -> bool,
    ) {
        let found = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if self.observed.iter().any(|key| matches(&key.kind)) {
                    return;
                }
                match self.subscribe_rx.recv().await {
                    Some(key) => self.observed.push(key),
                    None => panic!("{label}: the timeline command stream closed"),
                }
            }
        })
        .await;
        if found.is_err() {
            panic!(
                "{label}: no matching timeline subscription; saw {:?}",
                self.observed
            );
        }
    }

    /// Every main-pane focused subscription `(room_id, event_id)`.
    fn focused_subscriptions(&self) -> Vec<(String, String)> {
        self.observed
            .iter()
            .map(|key| key.kind.clone())
            .filter_map(|kind| match kind {
                TimelineKind::Focused { room_id, event_id } => Some((room_id, event_id)),
                _ => None,
            })
            .collect()
    }

    /// Every `Thread` timeline subscription `(room_id, root_event_id)`.
    fn thread_subscriptions(&self) -> Vec<(String, String)> {
        self.observed
            .iter()
            .map(|key| key.kind.clone())
            .filter_map(|kind| match kind {
                TimelineKind::Thread {
                    room_id,
                    root_event_id,
                } => Some((room_id, root_event_id)),
                _ => None,
            })
            .collect()
    }
}

#[tokio::test]
async fn a_search_hit_in_a_thread_opens_the_thread_pane_on_the_reply() {
    let mut harness = Harness::start(Some(ROOT));
    harness
        .navigate(EventNavigationSource::Search, request(1))
        .await;

    let state = harness
        .wait_for_state(
            "the threaded search target must open the thread pane",
            |state| thread_pane(state).is_some(),
        )
        .await;
    match thread_pane(&state) {
        Some((room_id, root_event_id, ThreadOpenIntent::PinnedReply { event_id })) => assert_eq!(
            (room_id, root_event_id, event_id.as_str()),
            (ROOM, ROOT, TARGET),
            "the thread pane opens on the located root, pinned to the search hit"
        ),
        other => panic!("the thread pane must pin the located reply; got {other:?}"),
    }
    assert_eq!(
        harness.wait_for_outcome(request(1)).await,
        IntentOutcome::Committed,
        "the thread pane carries the target, so the navigation is committed"
    );
    assert_eq!(
        state.navigation.event_navigation,
        EventNavigationState::Idle,
        "a thread-routed navigation settles instead of waiting for its deadline"
    );
    assert_eq!(
        state.navigation.main_timeline_anchor, None,
        "the main pane stays on the live room timeline"
    );
    harness
        .wait_for_subscription("the thread timeline must be subscribed", |kind| {
            matches!(
                kind,
                TimelineKind::Thread { room_id, root_event_id }
                    if room_id == ROOM && root_event_id == ROOT
            )
        })
        .await;
    assert_eq!(
        harness.thread_subscriptions(),
        vec![(ROOM.to_owned(), ROOT.to_owned())],
        "the thread timeline is subscribed on the located root"
    );
    assert!(
        harness.focused_subscriptions().is_empty(),
        "the main pane must not receive a focused projection of a thread reply: the SDK \
         filters such a projection down to the thread, so it is neither the room nor a \
         stable anchor; got {:?}",
        harness.focused_subscriptions()
    );
}

/// The same contract for every other `NavigateToEvent` source: the routing
/// decision is Core's, not the renderer's, and it is the same one path.
#[tokio::test]
async fn every_other_navigate_to_event_source_routes_a_thread_reply_the_same_way() {
    for source in [
        EventNavigationSource::Activity,
        EventNavigationSource::Pinned,
        EventNavigationSource::Notification,
        EventNavigationSource::ReplyQuote,
    ] {
        let mut harness = Harness::start(Some(ROOT));
        harness.navigate(source, request(1)).await;
        let state = harness
            .wait_for_state(
                "a threaded navigation target must open the thread pane",
                |state| thread_pane(state).is_some(),
            )
            .await;
        match thread_pane(&state) {
            Some((room_id, root_event_id, ThreadOpenIntent::PinnedReply { event_id })) => {
                assert_eq!(
                    (room_id, root_event_id, event_id.as_str()),
                    (ROOM, ROOT, TARGET),
                    "{source:?}"
                );
            }
            other => panic!("{source:?} must pin the located reply; got {other:?}"),
        }
        assert_eq!(
            harness.wait_for_outcome(request(1)).await,
            IntentOutcome::Committed,
            "{source:?}"
        );
        harness
            .wait_for_subscription("the thread timeline must be subscribed", |kind| {
                matches!(kind, TimelineKind::Thread { .. })
            })
            .await;
        assert!(
            harness.focused_subscriptions().is_empty(),
            "{source:?} must not give the main pane a thread-filtered projection"
        );
    }
}

/// Regression: an ordinary room message keeps the main-pane focused contract.
#[tokio::test]
async fn a_search_hit_outside_a_thread_keeps_the_main_pane_focused_contract() {
    let mut harness = Harness::start(None);
    harness
        .navigate(EventNavigationSource::Search, request(1))
        .await;

    let state = harness
        .wait_for_state(
            "the room-shaped search target must stay in the main pane",
            |state| state.focused_context != FocusedContextState::Closed,
        )
        .await;
    assert_eq!(
        state.focused_context,
        FocusedContextState::Opening {
            room_id: ROOM.to_owned(),
            event_id: TARGET.to_owned(),
        },
        "a room message still anchors in the main pane"
    );
    assert_eq!(
        state.thread,
        ThreadPaneState::Closed,
        "no thread pane opens"
    );
    harness
        .wait_for_subscription(
            "the main pane must subscribe the focused timeline",
            |kind| {
                matches!(
                    kind,
                    TimelineKind::Focused { room_id, event_id }
                        if room_id == ROOM && event_id == TARGET
                )
            },
        )
        .await;
    assert_eq!(
        harness.focused_subscriptions(),
        vec![(ROOM.to_owned(), TARGET.to_owned())],
        "the main pane subscribes the focused timeline for the room message"
    );
    assert!(
        harness.thread_subscriptions().is_empty(),
        "a room message never opens a thread timeline"
    );
}
