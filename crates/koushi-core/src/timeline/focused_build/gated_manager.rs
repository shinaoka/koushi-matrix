//! #1465: a running sessionless TimelineManager whose focused builds are held
//! at the SDK `build()` boundary, for AppActor-level navigation tests.
//!
//! A completed test build installs a test actor and, like a real focused
//! actor, publishes its first focused `InitialItems` for the build's
//! projection owner (containing the target event) and answers later replays,
//! so the private focused-projection commit lane reaches AppActor exactly as in
//! production.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use koushi_protocol::command::TimelineCommand;
use koushi_protocol::event::CoreEvent;
use koushi_protocol::failure::TimelineFailureKind;
use koushi_protocol::ids::{RequestId, TimelineGeneration, TimelineKey, TimelineKind};
use koushi_state::AppAction;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::executor;

use super::super::actor::{TimelineActorControl, TimelineActorHandle};
use super::super::manager::{TimelineManagerActor, TimelineMessage};
use super::super::navigation::{
    FocusedProjectionCommitted, InitialItemsRequestIdentity, NavigationProjectionDemand,
    TimelineActorGenerationGate, emit_initial_items_for_generation,
};
use super::super::test_support::{live_tail_test_manager, timeline_item};

/// Releases one held focused build with its SDK result.
pub(crate) type FocusedBuildRelease = oneshot::Sender<Result<(), TimelineFailureKind>>;

pub(crate) struct GatedFocusedBuildManager {
    msg_tx: mpsc::Sender<TimelineMessage>,
    pub(crate) action_rx: mpsc::Receiver<Vec<AppAction>>,
    pub(crate) event_rx: broadcast::Receiver<CoreEvent>,
    started_rx: mpsc::UnboundedReceiver<FocusedBuildRelease>,
    _task: executor::JoinHandle<()>,
}

/// Spawn a manager wired to AppActor's navigation demand and private
/// focused-projection commit lane.
pub(crate) fn spawn_gated_focused_build_manager(
    navigation_projection_rx: watch::Receiver<NavigationProjectionDemand>,
    focused_projection_tx: mpsc::UnboundedSender<FocusedProjectionCommitted>,
) -> GatedFocusedBuildManager {
    let mut manager = live_tail_test_manager(HashMap::new());
    let (action_tx, action_rx) = mpsc::channel(64);
    manager.action_tx = action_tx;
    let (event_tx, event_rx) = broadcast::channel(256);
    manager.event_tx = event_tx;
    manager.navigation_projection_rx = Some(navigation_projection_rx);
    manager.timeline_actor_generations = Arc::new(
        TimelineActorGenerationGate::with_focused_projection_commits(Some(focused_projection_tx)),
    );
    manager.focused_builds.test_actor_projection = true;
    let (started_tx, started_rx) = mpsc::unbounded_channel();
    manager.focused_builds.test_gate = Some(Arc::new(move || {
        let (release, released) = oneshot::channel();
        let _ = started_tx.send(release);
        Box::pin(async move { released.await.unwrap_or(Err(TimelineFailureKind::Sdk)) })
    }));
    let msg_tx = manager.msg_tx.clone();
    let task = executor::spawn(manager.run());
    GatedFocusedBuildManager {
        msg_tx,
        action_rx,
        event_rx,
        started_rx,
        _task: task,
    }
}

impl GatedFocusedBuildManager {
    pub(crate) fn msg_tx(&self) -> mpsc::Sender<TimelineMessage> {
        self.msg_tx.clone()
    }

    pub(crate) async fn command(&self, command: TimelineCommand) {
        assert!(
            self.msg_tx
                .send(TimelineMessage::Command(command))
                .await
                .is_ok(),
            "the manager mailbox must stay open"
        );
    }

    pub(crate) async fn build_started(&mut self) -> FocusedBuildRelease {
        executor::timeout(Duration::from_secs(2), self.started_rx.recv())
            .await
            .expect("the focused build must reach the SDK boundary")
            .expect("build gate sender")
    }

    pub(crate) fn no_build_started(&mut self) -> bool {
        matches!(
            self.started_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        )
    }

    /// The projection owner of the pending build for `key`, observed through
    /// the manager mailbox so every earlier command has been handled.
    pub(crate) async fn pending_owner(&self, key: &TimelineKey) -> Option<RequestId> {
        self.probe(key).await.pending_request_id
    }

    pub(crate) async fn installed(&self, key: &TimelineKey) -> bool {
        self.probe(key).await.installed
    }

    async fn probe(&self, key: &TimelineKey) -> super::FocusedBuildProbe {
        let (response, probe) = oneshot::channel();
        self.msg_tx
            .send(TimelineMessage::TestFocusedBuildProbe {
                key: key.clone(),
                response,
            })
            .await
            .expect("the manager mailbox must accept the probe");
        executor::timeout(Duration::from_secs(2), probe)
            .await
            .expect("the manager must answer while a focused build is in flight")
            .expect("probe response")
    }
}

impl TimelineManagerActor {
    /// The handle installed for a completed sessionless focused build. #1796:
    /// the gated harness actor answers `ReplayInitialItems` the way a real
    /// focused actor does — it re-emits its retained window for the subscribing
    /// cause — so a repeat navigation is exercised end to end instead of being
    /// dropped by a mailbox-only stand-in.
    pub(in crate::timeline) fn installed_focused_test_actor_handle(
        &self,
        key: &TimelineKey,
        projection_request_id: RequestId,
        actor_generation: u64,
    ) -> TimelineActorHandle {
        let TimelineKind::Focused { event_id, .. } = &key.kind else {
            return Self::room_subscription_residency_test_actor_handle();
        };
        if !self.focused_builds.test_actor_projection {
            return Self::room_subscription_residency_test_actor_handle();
        }
        let items = vec![timeline_item(event_id, Some("target"), "@bob:test", false)];
        let (control_tx, mut control_rx) = mpsc::channel(1);
        let (tx, mut message_rx) = mpsc::channel(1);
        let event_tx = self.event_tx.clone();
        let timeline_actor_generations = Arc::clone(&self.timeline_actor_generations);
        let replay_key = key.clone();
        let task = executor::spawn(async move {
            loop {
                tokio::select! {
                    control = control_rx.recv() => match control {
                        Some(TimelineActorControl::ReplayInitialItems {
                            cause_request_id,
                        }) => {
                            emit_initial_items_for_generation(
                                &event_tx,
                                &timeline_actor_generations,
                                &replay_key,
                                actor_generation,
                                InitialItemsRequestIdentity::replay(
                                    projection_request_id,
                                    Some(cause_request_id),
                                ),
                                TimelineGeneration(0),
                                items.clone(),
                                Vec::new(),
                            );
                        }
                        Some(_) => {}
                        None => return,
                    },
                    message = message_rx.recv() => {
                        if message.is_none() {
                            return;
                        }
                    }
                }
            }
        });
        TimelineActorHandle {
            tx,
            control_tx: Some(control_tx),
            thread_summary_projection:
                crate::timeline::actor::ThreadSummaryProjectionIngress::channel().0,
            position_rx: None,
            task: Some(task),
            auxiliary_tasks: Vec::new(),
            subscription_generation: None,
            enqueue_context: None,
        }
    }

    /// Publish the first focused projection of an installed test actor, as a
    /// real focused actor does for its projection owner.
    pub(in crate::timeline) fn publish_test_actor_focused_projection(
        &self,
        key: &TimelineKey,
        actor_generation: u64,
        projection_request_id: RequestId,
    ) {
        if self.session.is_some() || !self.focused_builds.test_actor_projection {
            return;
        }
        let TimelineKind::Focused { event_id, .. } = &key.kind else {
            return;
        };
        emit_initial_items_for_generation(
            &self.event_tx,
            &self.timeline_actor_generations,
            key,
            actor_generation,
            InitialItemsRequestIdentity::fresh(projection_request_id),
            TimelineGeneration(0),
            vec![timeline_item(event_id, Some("target"), "@bob:test", false)],
            Vec::new(),
        );
    }
}
