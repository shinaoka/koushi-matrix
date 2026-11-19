//! #1981: a reply quote whose original is already a row of the live room
//! timeline settles in place. Opening a focused timeline for it would swap the
//! main pane away and, because that focused view starts at the room's live
//! tail, straight back again: a visible flash with no emphasis. Everything here
//! is synthetic.

use super::*;
use koushi_protocol::event::IntentOutcome;
use koushi_state::{EventNavigationSource, EventNavigationState, FocusedContextState};

const ROOM: &str = "!reply-in-place-room:example.invalid";
const TARGET: &str = "$reply-in-place-original:example.invalid";
const USER: &str = "@synthetic:example.invalid";

fn request(sequence: u64) -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(1981),
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

/// What the account stub observed from the runtime.
#[derive(Debug)]
enum Observed {
    /// The live-room display query `(room_id, event_id)`.
    DisplayQuery(String, String),
    /// The network-backed event lookup.
    Lookup,
    Subscribe(TimelineKind),
}

struct Harness {
    command_tx: mpsc::Sender<CoreCommandEnvelope>,
    _action_tx: mpsc::Sender<Vec<AppAction>>,
    snapshot_rx: watch::Receiver<VersionedAppStateSnapshot>,
    event_rx: broadcast::Receiver<CoreEvent>,
    observed_rx: mpsc::UnboundedReceiver<Observed>,
    observed: Vec<Observed>,
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
    /// `shown` is the live room timeline's answer: whether its current display
    /// projection holds the target as a row.
    fn start(state: AppState, shown: bool) -> Self {
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
        ) = app_actor_event_navigation_fixture(data_dir.path(), state);
        let (observed_tx, observed_rx) = mpsc::unbounded_channel();
        let account = tokio::spawn(async move {
            while let Some(message) = account_rx.recv().await {
                match message {
                    AccountMessage::RoomTimelineShowsEvent {
                        room_id,
                        event_id,
                        response_tx,
                    } => {
                        let _ = observed_tx.send(Observed::DisplayQuery(room_id, event_id));
                        let _ = response_tx.send(shown);
                    }
                    AccountMessage::EnsureRoomEventCached { response_tx, .. } => {
                        let _ = observed_tx.send(Observed::Lookup);
                        let _ = response_tx.send(crate::account::RoomEventLookupResult::Located {
                            thread_root_event_id: None,
                        });
                    }
                    AccountMessage::TimelineCommand(
                        koushi_protocol::command::TimelineCommand::Subscribe { key, .. },
                    )
                    | AccountMessage::TimelineCommandWithComposerFormatting {
                        command: koushi_protocol::command::TimelineCommand::Subscribe { key, .. },
                        ..
                    } => {
                        let _ = observed_tx.send(Observed::Subscribe(key.kind));
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
            observed_rx,
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
                "{label}; event navigation: {:?}, focused context: {:?}, anchor: {:?}",
                self.snapshot_rx.borrow().state.navigation.event_navigation,
                self.snapshot_rx.borrow().state.focused_context,
                self.snapshot_rx
                    .borrow()
                    .state
                    .navigation
                    .main_timeline_anchor,
            ),
        }
    }

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

    /// Drain what the account stub saw so far, waiting briefly for stragglers
    /// sent by effects that follow the published terminal.
    async fn settle_observed(&mut self) {
        while let Ok(Some(observed)) =
            tokio::time::timeout(Duration::from_millis(100), self.observed_rx.recv()).await
        {
            self.observed.push(observed);
        }
    }

    async fn wait_for_focused_subscription(&mut self) {
        let found =
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if self.observed.iter().any(|seen| {
                        matches!(seen, Observed::Subscribe(TimelineKind::Focused { .. }))
                    }) {
                        return;
                    }
                    match self.observed_rx.recv().await {
                        Some(observed) => self.observed.push(observed),
                        None => panic!("the account stub stream closed"),
                    }
                }
            })
            .await;
        if found.is_err() {
            panic!("no focused subscription; saw {:?}", self.observed);
        }
    }

    fn display_queries(&self) -> Vec<(String, String)> {
        self.observed
            .iter()
            .filter_map(|seen| match seen {
                Observed::DisplayQuery(room_id, event_id) => {
                    Some((room_id.clone(), event_id.clone()))
                }
                _ => None,
            })
            .collect()
    }

    fn lookups(&self) -> usize {
        self.observed
            .iter()
            .filter(|seen| matches!(seen, Observed::Lookup))
            .count()
    }

    fn focused_subscriptions(&self) -> usize {
        self.observed
            .iter()
            .filter(|seen| matches!(seen, Observed::Subscribe(TimelineKind::Focused { .. })))
            .count()
    }
}

#[tokio::test]
async fn a_reply_quote_whose_original_is_a_live_row_settles_in_place() {
    let mut harness = Harness::start(selected_room_state(), true);
    harness
        .navigate(EventNavigationSource::ReplyQuote, request(1))
        .await;

    let state = harness
        .wait_for_state("the reply-quote navigation must settle in place", |state| {
            matches!(
                state.navigation.event_navigation,
                EventNavigationState::InPlace { .. }
            )
        })
        .await;
    assert_eq!(
        state.navigation.event_navigation,
        EventNavigationState::InPlace {
            generation: 1,
            source: EventNavigationSource::ReplyQuote,
            event_id: TARGET.to_owned(),
        }
    );
    assert_eq!(
        harness.wait_for_outcome(request(1)).await,
        IntentOutcome::Committed
    );
    assert_eq!(
        state.navigation.main_timeline_anchor, None,
        "the main pane stays on the live room timeline"
    );
    assert_eq!(
        state.focused_context,
        FocusedContextState::Closed,
        "no focused context opens for a row the live timeline already shows"
    );
    harness.settle_observed().await;
    assert_eq!(
        harness.display_queries(),
        vec![(ROOM.to_owned(), TARGET.to_owned())],
        "Core asks the live room timeline once, for the quoted original"
    );
    assert_eq!(
        harness.focused_subscriptions(),
        0,
        "no focused timeline is subscribed; saw {:?}",
        harness.observed
    );
    assert_eq!(
        harness.lookups(),
        0,
        "a shown row needs no network-backed event lookup"
    );
}

/// Regression: an original outside the loaded live window keeps #1592's
/// focused-context path.
#[tokio::test]
async fn a_reply_quote_whose_original_is_not_shown_keeps_the_focused_path() {
    let mut harness = Harness::start(selected_room_state(), false);
    harness
        .navigate(EventNavigationSource::ReplyQuote, request(1))
        .await;

    let state = harness
        .wait_for_state(
            "the original outside the window must open focused",
            |state| state.focused_context != FocusedContextState::Closed,
        )
        .await;
    assert_eq!(
        state.focused_context,
        FocusedContextState::Opening {
            room_id: ROOM.to_owned(),
            event_id: TARGET.to_owned(),
        }
    );
    harness.wait_for_focused_subscription().await;
    assert_eq!(harness.lookups(), 1, "the event lookup still runs");
}

/// The in-place route belongs to reply quotes. Other sources keep their
/// documented focused contract and never ask the live timeline.
#[tokio::test]
async fn other_navigation_sources_do_not_settle_in_place() {
    for source in [
        EventNavigationSource::Activity,
        EventNavigationSource::Search,
        EventNavigationSource::Pinned,
        EventNavigationSource::Notification,
    ] {
        let mut harness = Harness::start(selected_room_state(), true);
        harness.navigate(source, request(1)).await;
        harness
            .wait_for_state("the navigation must open focused", |state| {
                state.focused_context != FocusedContextState::Closed
            })
            .await;
        harness.wait_for_focused_subscription().await;
        assert!(
            harness.display_queries().is_empty(),
            "{source:?} must not consult the live room timeline"
        );
    }
}

/// While the main pane shows an anchored history window the live room timeline
/// is not on screen, so a reply quote there keeps the focused path.
#[tokio::test]
async fn a_reply_quote_from_an_anchored_main_pane_keeps_the_focused_path() {
    let mut state = selected_room_state();
    reduce(
        &mut state,
        AppAction::EnterAnchoredTimeline {
            room_id: ROOM.to_owned(),
            event_id: "$earlier-anchor:example.invalid".to_owned(),
        },
    );
    assert!(state.navigation.main_timeline_anchor.is_some());
    let mut harness = Harness::start(state, true);
    harness
        .navigate(EventNavigationSource::ReplyQuote, request(1))
        .await;
    harness
        .wait_for_state("the anchored pane must open focused", |state| {
            matches!(
                &state.focused_context,
                FocusedContextState::Opening { event_id, .. } if event_id == TARGET
            )
        })
        .await;
    harness.wait_for_focused_subscription().await;
    assert!(
        harness.display_queries().is_empty(),
        "an anchored main pane does not show the live room timeline"
    );
}
