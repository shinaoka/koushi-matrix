use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use futures_util::StreamExt;
use koushi_sdk::{
    MatrixClientSession, MatrixCommittedRoomTimelineCheckpoint as MatrixRoomSubscriptionCheckpoint,
    MatrixLiveTailRefreshOutcome,
};
use koushi_state::{
    AppAction, ComposerFormattingOptions, OperationFailureKind, TimelineThreadRootOrder,
};

use matrix_sdk::ruma::OwnedRoomId;
use matrix_sdk_ui::timeline::TimelineFocus;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::account_work::{AccountWorkKind, AccountWorkScheduler};
use crate::composer_draft_lifecycle::ForwardedComposerDraftPermit;
use crate::executor;
use crate::link_preview::LinkPreviewContext;
#[cfg(test)]
use crate::live_tail_freshness::LiveTailFreshnessState;
use crate::live_tail_freshness::LiveTailRefreshCoordinator;
use crate::read_state::{ReadPersistenceSnapshot, ReadStateKey};
use crate::search::SearchIndexMessage;
use crate::startup_trace::{self, StartupPhase};
use crate::threads_list::{
    AggregateRefresh, ThreadRootProjectionActivity, ThreadRootProjectionRefreshResult,
    ThreadRootProjectionService,
};
use koushi_protocol::command::{InitialBackfillPolicy, TimelineCommand};
use koushi_protocol::event::{CoreEvent, TimelineAnchorRestoreStatus, TimelineEvent, TimelineItem};
use koushi_protocol::failure::{CoreFailure, TimelineFailureKind};
use koushi_protocol::ids::{
    RequestId, RuntimeConnectionId, TimelineBatchId, TimelineGeneration, TimelineKey, TimelineKind,
};

// BEGIN GENERATED SIBLING IMPORTS
use super::actor::{
    TimelineActor, TimelineActorControl, TimelineActorHandle, TimelineActorMessage,
    emit_app_action_reliable,
};
use super::composer::validate_composer_body_for_timeline_send;
use super::diagnostics::{
    record_subscribe_stage, record_timeline_gap_repair, timeline_subscription_failed_action,
    trace_timeline_route,
};
use super::gap_repair::{
    GlobalResponseCommit, LIVE_TAIL_CANCELLATION_DEADLINE, TimelineGapRepairTrigger,
};
use super::navigation::{
    INITIAL_EMPTY_ROOM_BACKFILL_EVENT_COUNT, NavigationProjectionIntent,
    TimelineActorGenerationGate, receive_navigation_projection,
};
use super::outbound_send::{
    GlobalSendCompletionObserverFuture, SendComposerProjection, SendEnqueueWorkerSupervisor,
    SharedSendCompletionCoordinator, SubmissionAdmissionLedger, TimelineSendEnqueuePayload,
    TimelineSendTerminalHandoff, TimelineSendTerminalIngress,
    apply_send_completion_observation_loss_and_handoff, poll_global_send_completion_observer,
    run_global_send_completion_observer,
};
use super::read_state::{ReadCommandKind, ReadPersistenceIngress, ReadWorkerSupervisor};
use super::relay::koushi_timeline_builder;
use super::residency::{
    MembershipOperationGate, RoomLeaveState, RoomMembershipTransition, RoomRemovalCause,
    SubscriptionReconcileTrigger, TimelineSubscriptionResidencyHandle, VisibleRoomObservation,
};
use super::thread_projection::{
    ThreadRootProjectionFetchRegistry, ThreadSummaryActivityObservation,
};
// END GENERATED SIBLING IMPORTS

/// Bounded diff queue capacity per subscribed timeline (overview.md, Async rule 10).

pub const TIMELINE_DIFF_QUEUE_CAPACITY: usize = 128;

fn initial_thread_backfill_is_authoritative(end_reached: bool, item_count: usize) -> bool {
    end_reached || item_count > 0
}

/// Messages routed to the `TimelineManagerActor`.
pub(crate) enum TimelineMessage {
    Command(TimelineCommand),
    CommandWithComposerFormatting {
        command: TimelineCommand,
        formatting_options: ComposerFormattingOptions,
    },
    LeasedCommand {
        command: TimelineCommand,
        composer_permit: ForwardedComposerDraftPermit,
    },
    LeasedCommandWithComposerFormatting {
        command: TimelineCommand,
        composer_permit: ForwardedComposerDraftPermit,
        formatting_options: ComposerFormattingOptions,
    },
    /// Sync started: carries the one live `RoomListService`. Subscribing a timeline must also
    /// subscribe its room with the live service so the server streams that
    /// room's new timeline events (canon: TimelineActor description; without
    /// this on servers that only deliver the initial window).
    SyncStarted {
        room_list_service: Arc<matrix_sdk_ui::room_list_service::RoomListService>,
        core_generation: u64,
    },
    #[cfg(any(test, feature = "test-hooks"))]
    ResidencyTestSnapshot {
        response: oneshot::Sender<(Vec<String>, Vec<String>)>,
    },
    VisibleRoomsObserved {
        core_generation: u64,
        room_ids: Vec<VisibleRoomObservation>,
    },
    RoomMembershipObserved {
        core_generation: u64,
        transitions: Vec<RoomMembershipTransition>,
    },
    RoomLeft {
        room_id: OwnedRoomId,
        cause: RoomRemovalCause,
        acknowledged: oneshot::Sender<()>,
    },
    RoomRejoined {
        room_id: OwnedRoomId,
        acknowledged: oneshot::Sender<()>,
    },
    AllRoomsResponseCommitted {
        core_generation: u64,
        response_sequence: u64,
    },
    RoomSubscriptionCheckpoint {
        service_epoch: u64,
        checkpoint: MatrixRoomSubscriptionCheckpoint,
    },
    LiveTailRefreshCompleted {
        key: TimelineKey,
        actor_generation: u64,
        epoch: u64,
        operation_generation: u64,
        outcome: MatrixLiveTailRefreshOutcome,
        requested_limit: u16,
        returned_events: usize,
        duration_ms: u128,
    },
    #[cfg(test)]
    TestLiveTailDispatchState {
        key: TimelineKey,
        epoch: u64,
        response: oneshot::Sender<(bool, usize, Option<usize>)>,
    },
    IgnoredUsersUpdated {
        user_ids: std::collections::BTreeSet<String>,
    },
    /// Terminal result of the manager-owned bounded root lookup. It returns to
    /// the manager mailbox rather than publishing state/frontend changes from
    /// an unowned detached task.
    ThreadRootProjectionFetchFinished {
        key: TimelineKey,
        actor_generation: u64,
        activity: ThreadRootProjectionActivity,
        result: Result<TimelineItem, OperationFailureKind>,
    },
    /// Start aggregate refreshes after an accepted Room-window commit. A
    /// refresh whose root item is still pending starts the existing bounded
    /// root hydration first; its aggregate worker starts from that terminal.
    StartAggregateRefresh {
        key: TimelineKey,
        actor_generation: u64,
        own_user_id: Option<matrix_sdk::ruma::OwnedUserId>,
        refreshes: Vec<AggregateRefresh>,
    },
    /// Terminal result of one manager-owned exact aggregate refresh.
    AggregateRefreshFinished {
        key: TimelineKey,
        actor_generation: u64,
        refresh: AggregateRefresh,
        result: Result<ThreadRootProjectionRefreshResult, OperationFailureKind>,
    },
    /// Reliable observation from a current Thread actor. The manager routes
    /// the projection to the exact current Room actor without awaiting its
    /// ordinary mailbox.
    ThreadSummaryActivityObserved {
        key: TimelineKey,
        actor_generation: u64,
        observation: ThreadSummaryActivityObservation,
    },
    AuthoritativeReadStateObserved {
        key: TimelineKey,
        actor_generation: u64,
        read_key: ReadStateKey,
        event_id: Option<String>,
    },
    LocalReadBoundaryObserved {
        key: TimelineKey,
        actor_generation: u64,
        target: crate::read_state::ReadTarget,
    },
    #[cfg_attr(not(test), allow(dead_code))]
    Shutdown {
        acknowledged: Option<tokio::sync::oneshot::Sender<()>>,
    },
}

/// Handle to the timeline manager task (owned by `AccountActor`).
pub struct TimelineManagerHandle {
    tx: mpsc::Sender<TimelineMessage>,
    control_tx: mpsc::Sender<TimelineManagerControl>,
    residency: TimelineSubscriptionResidencyHandle,
    #[cfg(test)]
    terminal_ingress: TimelineSendTerminalIngress,
}

pub(super) enum TimelineManagerControl {
    ReadStatePolicyChanged {
        session_generation: u64,
        send_read_receipts: bool,
        acknowledged: oneshot::Sender<()>,
    },
    DisplayPolicyChanged {
        thread_root_order: TimelineThreadRootOrder,
        acknowledged: oneshot::Sender<()>,
    },
    Shutdown {
        acknowledged: oneshot::Sender<()>,
    },
}

impl TimelineManagerHandle {
    pub(crate) fn spawn(
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
        data_dir: Option<std::path::PathBuf>,
        account_work: AccountWorkScheduler,
        navigation_projection_rx: Option<watch::Receiver<Option<NavigationProjectionIntent>>>,
        focused_projection_tx: Option<mpsc::UnboundedSender<super::FocusedProjectionCommitted>>,
    ) -> Self {
        TimelineManagerActor::spawn(
            action_tx,
            event_tx,
            data_dir,
            account_work,
            navigation_projection_rx,
            focused_projection_tx,
        )
    }

    pub(crate) fn spawn_with_session(
        session: Arc<MatrixClientSession>,
        read_session_generation: u64,
        restored_read_state: ReadPersistenceSnapshot,
        read_persistence: ReadPersistenceIngress,
        send_read_receipts: bool,
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
        search_index_tx: mpsc::Sender<SearchIndexMessage>,
        data_dir: Option<std::path::PathBuf>,
        link_preview_policy: LinkPreviewContext,
        account_work: AccountWorkScheduler,
        navigation_projection_rx: Option<watch::Receiver<Option<NavigationProjectionIntent>>>,
        focused_projection_tx: Option<mpsc::UnboundedSender<super::FocusedProjectionCommitted>>,
    ) -> Self {
        TimelineManagerActor::spawn_with_session(
            session,
            read_session_generation,
            restored_read_state,
            read_persistence,
            send_read_receipts,
            action_tx,
            event_tx,
            search_index_tx,
            data_dir,
            link_preview_policy,
            account_work,
            navigation_projection_rx,
            focused_projection_tx,
        )
    }

    pub(crate) async fn send(&self, msg: TimelineMessage) -> bool {
        self.tx.send(msg).await.is_ok()
    }

    pub(crate) fn sender(&self) -> mpsc::Sender<TimelineMessage> {
        self.tx.clone()
    }

    pub(crate) fn residency_handle(&self) -> TimelineSubscriptionResidencyHandle {
        self.residency.clone()
    }

    pub(crate) async fn close_membership_operations(&self) {
        self.residency.close_and_drain().await;
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) async fn residency_snapshot_for_testing(
        &self,
        response: oneshot::Sender<(Vec<String>, Vec<String>)>,
    ) -> bool {
        self.send(TimelineMessage::ResidencyTestSnapshot { response })
            .await
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn residency_gate_snapshot_for_testing(&self) -> (bool, usize) {
        self.residency.gate_snapshot()
    }

    pub(crate) async fn set_read_state_policy(
        &self,
        session_generation: u64,
        send_read_receipts: bool,
    ) -> bool {
        let (acknowledged, acknowledgement) = oneshot::channel();
        if self
            .control_tx
            .send(TimelineManagerControl::ReadStatePolicyChanged {
                session_generation,
                send_read_receipts,
                acknowledged,
            })
            .await
            .is_err()
        {
            return false;
        }
        acknowledgement.await.is_ok()
    }

    pub(crate) async fn set_display_policy(
        &self,
        thread_root_order: TimelineThreadRootOrder,
    ) -> bool {
        let (acknowledged, acknowledgement) = oneshot::channel();
        if self
            .control_tx
            .send(TimelineManagerControl::DisplayPolicyChanged {
                thread_root_order,
                acknowledged,
            })
            .await
            .is_err()
        {
            return false;
        }
        acknowledgement.await.is_ok()
    }

    pub(crate) async fn shutdown(&self) -> bool {
        self.close_membership_operations().await;
        let (acknowledged, acknowledgement) = oneshot::channel();
        if self
            .control_tx
            .send(TimelineManagerControl::Shutdown { acknowledged })
            .await
            .is_err()
        {
            return false;
        }
        acknowledgement.await.is_ok()
    }

    #[cfg(test)]
    pub(super) fn terminal_sender(&self) -> TimelineSendTerminalIngress {
        self.terminal_ingress.clone()
    }
}

/// Manages the `HashMap<TimelineKey, TimelineActorHandle>`.
/// Colocated as a child task under `AccountActor` (spec: "actor deployment
/// is flexible; boundaries define ownership not one task per actor").
pub struct TimelineManagerActor {
    pub(super) session: Option<Arc<MatrixClientSession>>,
    pub(super) room_list_service: Option<Arc<matrix_sdk_ui::room_list_service::RoomListService>>,
    pub(super) room_subscription_checkpoint_task: Option<executor::JoinHandle<()>>,
    pub(super) room_subscription_service_epoch: u64,
    pub(super) current_core_generation: Option<u64>,
    pub(super) room_leave_states: BTreeMap<OwnedRoomId, RoomLeaveState>,
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) restored_room_subscription_probe: Option<(bool, BTreeSet<OwnedRoomId>)>,
    /// Session-resident desired room subscriptions. This set outlives every
    /// presentation actor and is dropped with the manager/session.
    pub(super) session_subscribed_rooms: BTreeSet<OwnedRoomId>,
    /// Refcounted room-ID leases for live Timeline actor resources (issue #518).
    /// Room, Thread, and Focused timelines all contribute a lease for their
    /// room; leases do not own session residency.
    pub(super) subscribed_room_leases: BTreeMap<OwnedRoomId, usize>,
    /// Rooms that were ever subscribed this runtime (used to distinguish a
    /// security-required re-add from a first-time add for rotation
    /// correlation diagnostics).
    pub(super) subscription_room_seen: BTreeSet<OwnedRoomId>,
    /// Process-local room ordinals for identifier-free correlation records.
    pub(super) subscription_room_ordinals: BTreeMap<OwnedRoomId, u64>,
    pub(super) next_subscription_room_ordinal: u64,
    pub(super) global_response_commit: Option<GlobalResponseCommit>,
    pub(super) timelines: HashMap<TimelineKey, TimelineActorHandle>,
    pub(super) accepted_submissions: SubmissionAdmissionLedger,
    pub(super) send_completion: SharedSendCompletionCoordinator,
    pub(super) global_send_completion_observer_future: Option<GlobalSendCompletionObserverFuture>,
    pub(super) send_enqueue_workers: SendEnqueueWorkerSupervisor,
    pub(super) read_workers: ReadWorkerSupervisor,
    pub(super) action_tx: mpsc::Sender<Vec<AppAction>>,
    pub(super) event_tx: broadcast::Sender<CoreEvent>,
    pub(super) msg_tx: mpsc::Sender<TimelineMessage>,
    pub(super) msg_rx: mpsc::Receiver<TimelineMessage>,
    pub(super) control_rx: Option<mpsc::Receiver<TimelineManagerControl>>,
    pub(super) navigation_projection_rx:
        Option<watch::Receiver<Option<NavigationProjectionIntent>>>,
    pub(super) last_navigation_projection_generation: u64,
    /// Non-evicting active terminal-delivery state. Admission is synchronous
    /// under the shared send tracker lock, so its FIFO is bounded logically by
    /// already-admitted/outstanding sends (at most one failure and one final
    /// handoff per tracked send), not by arbitrary background work.
    pub(super) terminal_ingress: TimelineSendTerminalIngress,
    pub(super) terminal_rx: mpsc::UnboundedReceiver<TimelineSendTerminalHandoff>,
    /// Search index mutation sender. Forwarded to individual `TimelineActor`s
    /// so they can push `SearchIndexMessage`s on each diff. `None` when there
    /// is no active search index (pre-session or pre-Phase-6 builds).
    pub(super) search_index_tx: Option<mpsc::Sender<SearchIndexMessage>>,
    pub(super) ignored_user_ids: std::collections::BTreeSet<String>,
    /// Application data directory for cached preview images.
    pub(super) data_dir: Option<std::path::PathBuf>,
    /// URL preview policy broadcast from AppState.
    pub(super) link_preview_policy: LinkPreviewContext,
    pub(super) composer_formatting_options: ComposerFormattingOptions,
    pub(super) thread_root_order: TimelineThreadRootOrder,
    pub(super) account_work: AccountWorkScheduler,
    /// Room-root hydration is shared across replacement actors so SyncStarted
    /// cannot restart a failed/pending bounded lookup.
    pub(super) thread_root_projection_service: Arc<Mutex<ThreadRootProjectionService>>,
    pub(super) thread_root_projection_fetches: ThreadRootProjectionFetchRegistry,
    pub(super) timeline_actor_generations: Arc<TimelineActorGenerationGate>,
    pub(super) live_tail_refreshes: LiveTailRefreshCoordinator<TimelineKey>,
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) test_session_available: bool,
}

impl Drop for TimelineManagerActor {
    fn drop(&mut self) {
        // Ordered shutdown has already drained or synchronously cancelled the
        // directly-polled futures. On cancellation or panic, close admission,
        // then settle worker registrations before stopping the observer.
        self.terminal_ingress.stop_accepting();
        self.read_workers.cancel_all();
        self.send_enqueue_workers.cancel_all();
        self.global_send_completion_observer_future.take();
    }
}

impl TimelineManagerActor {
    pub(crate) fn spawn(
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
        data_dir: Option<std::path::PathBuf>,
        account_work: AccountWorkScheduler,
        navigation_projection_rx: Option<watch::Receiver<Option<NavigationProjectionIntent>>>,
        focused_projection_tx: Option<mpsc::UnboundedSender<super::FocusedProjectionCommitted>>,
    ) -> TimelineManagerHandle {
        let (tx, msg_rx) = mpsc::channel(crate::ACTOR_MESSAGE_QUEUE_CAPACITY);
        let (control_tx, control_rx) = mpsc::channel(1);
        let (terminal_ingress, terminal_rx) = TimelineSendTerminalIngress::channel();
        let residency = TimelineSubscriptionResidencyHandle {
            tx: tx.clone(),
            gate: MembershipOperationGate::new(),
        };
        let actor = TimelineManagerActor {
            session: None,
            room_list_service: None,
            room_subscription_checkpoint_task: None,
            room_subscription_service_epoch: 0,
            current_core_generation: None,
            room_leave_states: BTreeMap::new(),
            #[cfg(any(test, feature = "test-hooks"))]
            restored_room_subscription_probe: None,
            session_subscribed_rooms: BTreeSet::new(),
            subscribed_room_leases: BTreeMap::new(),
            subscription_room_seen: BTreeSet::new(),
            subscription_room_ordinals: BTreeMap::new(),
            next_subscription_room_ordinal: 0,
            global_response_commit: None,
            timelines: HashMap::new(),
            accepted_submissions: SubmissionAdmissionLedger::default(),
            send_completion: SharedSendCompletionCoordinator::default(),
            global_send_completion_observer_future: None,
            send_enqueue_workers: SendEnqueueWorkerSupervisor::new(terminal_ingress.clone()),
            read_workers: ReadWorkerSupervisor::unavailable(),
            thread_root_order: TimelineThreadRootOrder::LatestReply,
            action_tx,
            event_tx,
            msg_tx: tx.clone(),
            msg_rx,
            control_rx: Some(control_rx),
            navigation_projection_rx,
            last_navigation_projection_generation: 0,
            terminal_ingress: terminal_ingress.clone(),
            terminal_rx,
            search_index_tx: None,
            ignored_user_ids: std::collections::BTreeSet::new(),
            data_dir,
            link_preview_policy: LinkPreviewContext::default(),
            composer_formatting_options: ComposerFormattingOptions::default(),
            account_work,
            thread_root_projection_service: Arc::new(Mutex::new(
                ThreadRootProjectionService::default(),
            )),
            thread_root_projection_fetches: ThreadRootProjectionFetchRegistry::default(),
            timeline_actor_generations: Arc::new(
                TimelineActorGenerationGate::with_focused_projection_commits(focused_projection_tx),
            ),
            live_tail_refreshes: LiveTailRefreshCoordinator::new(),
            #[cfg(any(test, feature = "test-hooks"))]
            test_session_available: false,
        };
        executor::spawn(actor.run());
        TimelineManagerHandle {
            tx,
            control_tx,
            residency,
            #[cfg(test)]
            terminal_ingress,
        }
    }
    /// Spawn with a session and a search index mutation sender.
    /// Called by `AccountActor::spawn_sync_actor` (Phase 6 wiring).
    pub(crate) fn spawn_with_session(
        session: Arc<MatrixClientSession>,
        read_session_generation: u64,
        restored_read_state: ReadPersistenceSnapshot,
        read_persistence: ReadPersistenceIngress,
        send_read_receipts: bool,
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
        search_index_tx: mpsc::Sender<SearchIndexMessage>,
        data_dir: Option<std::path::PathBuf>,
        link_preview_policy: LinkPreviewContext,
        account_work: AccountWorkScheduler,
        navigation_projection_rx: Option<watch::Receiver<Option<NavigationProjectionIntent>>>,
        focused_projection_tx: Option<mpsc::UnboundedSender<super::FocusedProjectionCommitted>>,
    ) -> TimelineManagerHandle {
        let (tx, msg_rx) = mpsc::channel(crate::ACTOR_MESSAGE_QUEUE_CAPACITY);
        let (control_tx, control_rx) = mpsc::channel(1);
        let (terminal_ingress, terminal_rx) = TimelineSendTerminalIngress::channel();
        let residency = TimelineSubscriptionResidencyHandle {
            tx: tx.clone(),
            gate: MembershipOperationGate::new(),
        };
        let send_completion = SharedSendCompletionCoordinator::default();
        let global_send_completion_observer_future =
            Some(Box::pin(run_global_send_completion_observer(
                session.client().send_queue().subscribe(),
                Arc::clone(&send_completion),
                terminal_ingress.clone(),
            )) as GlobalSendCompletionObserverFuture);
        let actor = TimelineManagerActor {
            session: Some(Arc::clone(&session)),
            room_list_service: None,
            room_subscription_checkpoint_task: None,
            room_subscription_service_epoch: 0,
            current_core_generation: None,
            room_leave_states: BTreeMap::new(),
            #[cfg(any(test, feature = "test-hooks"))]
            restored_room_subscription_probe: None,
            session_subscribed_rooms: BTreeSet::new(),
            subscribed_room_leases: BTreeMap::new(),
            subscription_room_seen: BTreeSet::new(),
            subscription_room_ordinals: BTreeMap::new(),
            next_subscription_room_ordinal: 0,
            global_response_commit: None,
            timelines: HashMap::new(),
            accepted_submissions: SubmissionAdmissionLedger::default(),
            send_completion,
            global_send_completion_observer_future,
            send_enqueue_workers: SendEnqueueWorkerSupervisor::new(terminal_ingress.clone()),
            read_workers: ReadWorkerSupervisor::matrix(
                session,
                read_session_generation,
                restored_read_state,
                read_persistence,
                send_read_receipts,
            ),
            thread_root_order: TimelineThreadRootOrder::LatestReply,
            action_tx,
            event_tx,
            msg_tx: tx.clone(),
            msg_rx,
            control_rx: Some(control_rx),
            navigation_projection_rx,
            last_navigation_projection_generation: 0,
            terminal_ingress: terminal_ingress.clone(),
            terminal_rx,
            search_index_tx: Some(search_index_tx),
            ignored_user_ids: std::collections::BTreeSet::new(),
            data_dir,
            link_preview_policy,
            composer_formatting_options: ComposerFormattingOptions::default(),
            account_work,
            thread_root_projection_service: Arc::new(Mutex::new(
                ThreadRootProjectionService::default(),
            )),
            thread_root_projection_fetches: ThreadRootProjectionFetchRegistry::default(),
            timeline_actor_generations: Arc::new(
                TimelineActorGenerationGate::with_focused_projection_commits(focused_projection_tx),
            ),
            live_tail_refreshes: LiveTailRefreshCoordinator::new(),
            #[cfg(any(test, feature = "test-hooks"))]
            test_session_available: false,
        };
        executor::spawn(actor.run());
        TimelineManagerHandle {
            tx,
            control_tx,
            residency,
            #[cfg(test)]
            terminal_ingress,
        }
    }
    pub(super) async fn run(mut self) {
        let mut shutdown_acknowledgement = None;
        #[cfg(test)]
        let mut live_tail_completion_dispatches = 0;
        #[cfg(test)]
        let mut navigation_projection_completion_dispatches = None;
        loop {
            let msg = tokio::select! {
                biased;
                control = async {
                    match self.control_rx.as_mut() {
                        Some(receiver) => receiver.recv().await,
                        None => futures_util::future::pending().await,
                    }
                } => {
                    match control {
                        Some(TimelineManagerControl::ReadStatePolicyChanged {
                            session_generation,
                            send_read_receipts,
                            acknowledged,
                        }) => {
                            self.handle_read_state_policy_changed(
                                session_generation,
                                send_read_receipts,
                            )
                            .await;
                            let _ = acknowledged.send(());
                            continue;
                        }
                        Some(TimelineManagerControl::DisplayPolicyChanged {
                            thread_root_order,
                            acknowledged,
                        }) => {
                            self.thread_root_order = thread_root_order;
                            for actor in self.timelines.values() {
                                let _ = actor
                                    .send_control(TimelineActorControl::DisplayPolicyChanged {
                                        thread_root_order,
                                    })
                                    .await;
                            }
                            let _ = acknowledged.send(());
                            continue;
                        }
                        Some(TimelineManagerControl::Shutdown { acknowledged }) => {
                            shutdown_acknowledgement = Some(acknowledged);
                            break;
                        }
                        None => break,
                    }
                }
                projection = receive_navigation_projection(
                    &mut self.navigation_projection_rx
                ) => {
                    if let Some(projection) = projection {
                        #[cfg(test)]
                        {
                            navigation_projection_completion_dispatches =
                                Some(live_tail_completion_dispatches);
                        }
                        self.handle_navigation_projection(projection).await;
                    }
                    continue;
                }
                completion = self.read_workers.tasks.next(),
                    if !self.read_workers.tasks.is_empty() => {
                    if let Some(completion) = completion {
                        self.handle_read_worker_completion(completion).await;
                    }
                    continue;
                }
                retry = self.read_workers.retry_tasks.next(),
                    if !self.read_workers.retry_tasks.is_empty() => {
                    if let Some(retry) = retry {
                        self.handle_read_worker_completion(retry).await;
                    }
                    continue;
                }
                terminal = self.terminal_rx.recv() => {
                    let Some(terminal) = terminal else { break };
                    self.handle_send_terminal_handoff(terminal).await;
                    continue;
                }
                worker = self.send_enqueue_workers.tasks.next(), if !self.send_enqueue_workers.tasks.is_empty() => {
                    if let Some(completion) = worker {
                        self.handle_send_enqueue_worker_completion(completion).await;
                    }
                    continue;
                }
                _ = poll_global_send_completion_observer(&mut self.global_send_completion_observer_future) => {
                    self.global_send_completion_observer_future = None;
                    continue;
                }
                msg = self.msg_rx.recv() => msg,
            };
            let Some(msg) = msg else { break };
            match msg {
                TimelineMessage::Shutdown { acknowledged } => {
                    shutdown_acknowledgement = acknowledged;
                    break;
                }
                TimelineMessage::SyncStarted {
                    room_list_service,
                    core_generation,
                } => {
                    self.handle_sync_started(room_list_service, core_generation)
                        .await;
                }
                #[cfg(any(test, feature = "test-hooks"))]
                TimelineMessage::ResidencyTestSnapshot { response } => {
                    let (desired, active, ..) = self.room_subscription_residency_test_snapshot();
                    let _ = response.send((desired, active));
                }
                TimelineMessage::VisibleRoomsObserved {
                    core_generation,
                    room_ids,
                } => {
                    self.handle_visible_rooms_observed(core_generation, room_ids)
                        .await;
                }
                TimelineMessage::RoomMembershipObserved {
                    core_generation,
                    transitions,
                } => {
                    self.handle_room_membership_observed(core_generation, transitions)
                        .await;
                }
                TimelineMessage::RoomLeft {
                    room_id,
                    cause,
                    acknowledged,
                } => {
                    self.handle_room_left(room_id, cause).await;
                    let _ = acknowledged.send(());
                }
                TimelineMessage::RoomRejoined {
                    room_id,
                    acknowledged,
                } => {
                    self.handle_room_rejoined(room_id).await;
                    let _ = acknowledged.send(());
                }
                TimelineMessage::AllRoomsResponseCommitted {
                    core_generation,
                    response_sequence,
                } => {
                    self.handle_all_rooms_response_committed(core_generation, response_sequence)
                        .await;
                }
                TimelineMessage::RoomSubscriptionCheckpoint {
                    service_epoch,
                    checkpoint,
                } => {
                    self.handle_room_subscription_checkpoint(service_epoch, checkpoint)
                        .await;
                }
                TimelineMessage::LiveTailRefreshCompleted {
                    key,
                    actor_generation,
                    epoch,
                    operation_generation,
                    outcome,
                    requested_limit,
                    returned_events,
                    duration_ms,
                } => {
                    #[cfg(test)]
                    {
                        live_tail_completion_dispatches += 1;
                    }
                    let hydration_key = key.clone();
                    self.handle_live_tail_refresh_completed(
                        key,
                        actor_generation,
                        epoch,
                        operation_generation,
                        outcome,
                        requested_limit,
                        returned_events,
                        duration_ms,
                    )
                    .await;
                    self.retry_pending_send_hydrations(&hydration_key);
                }
                #[cfg(test)]
                TimelineMessage::TestLiveTailDispatchState {
                    key,
                    epoch,
                    response,
                } => {
                    let _ = response.send((
                        self.live_tail_refreshes.freshness(&key)
                            == Some(LiveTailFreshnessState::Fresh { epoch }),
                        live_tail_completion_dispatches,
                        navigation_projection_completion_dispatches,
                    ));
                }
                TimelineMessage::IgnoredUsersUpdated { user_ids } => {
                    self.handle_ignored_users_updated(user_ids).await;
                }
                TimelineMessage::ThreadRootProjectionFetchFinished {
                    key,
                    actor_generation,
                    activity,
                    result,
                } => {
                    self.handle_thread_root_projection_fetch_finished(
                        key,
                        actor_generation,
                        activity,
                        result,
                    )
                    .await;
                }
                TimelineMessage::StartAggregateRefresh {
                    key,
                    actor_generation,
                    own_user_id,
                    refreshes,
                } => {
                    self.handle_aggregate_refresh_start(
                        key,
                        actor_generation,
                        own_user_id,
                        refreshes,
                    )
                    .await;
                }
                TimelineMessage::AggregateRefreshFinished {
                    key,
                    actor_generation,
                    refresh,
                    result,
                } => {
                    self.handle_aggregate_refresh_finished(key, actor_generation, refresh, result)
                        .await;
                }
                TimelineMessage::ThreadSummaryActivityObserved {
                    key,
                    actor_generation,
                    observation,
                } => {
                    self.handle_thread_summary_activity_observed(
                        key,
                        actor_generation,
                        observation,
                    );
                }
                TimelineMessage::AuthoritativeReadStateObserved {
                    key,
                    actor_generation,
                    read_key,
                    event_id,
                } => {
                    self.handle_authoritative_read_state_observed(
                        &key,
                        actor_generation,
                        read_key,
                        event_id,
                    )
                    .await;
                }
                TimelineMessage::LocalReadBoundaryObserved {
                    key,
                    actor_generation,
                    target,
                } => {
                    self.handle_local_read_boundary_observed(key, actor_generation, target)
                        .await;
                }
                TimelineMessage::Command(command) => {
                    self.handle_command(command).await;
                }
                TimelineMessage::CommandWithComposerFormatting {
                    command,
                    formatting_options,
                } => {
                    self.handle_command_with_formatting_options(command, formatting_options)
                        .await;
                }
                TimelineMessage::LeasedCommand {
                    command,
                    composer_permit,
                } => {
                    self.handle_command_with_permit(command, Some(composer_permit))
                        .await;
                }
                TimelineMessage::LeasedCommandWithComposerFormatting {
                    command,
                    composer_permit,
                    formatting_options,
                } => {
                    self.handle_command_with_formatting_context(
                        command,
                        Some(composer_permit),
                        formatting_options,
                    )
                    .await;
                }
            }
        }
        let room_keys = self
            .timelines
            .keys()
            .filter(|key| matches!(key.kind, TimelineKind::Room { .. }))
            .cloned()
            .collect::<Vec<_>>();
        // Stop accepting commands, then join session-owned enqueue workers
        // while the sole global terminal observer remains live. A worker may
        // still bind a durably saved SDK transaction during this phase.
        self.read_workers.cancel_all();
        self.read_workers.publish_persistence();
        let abandoned_read_waiters = self
            .read_workers
            .waiters
            .drain()
            .map(|(_, waiter)| waiter)
            .collect::<Vec<_>>();
        for waiter in abandoned_read_waiters {
            self.emit_failure(
                waiter.request_id,
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::Sdk,
                },
            );
        }
        self.join_send_enqueue_workers().await;
        // Once enqueue workers are quiescent, stop the observer before actor
        // presentation producers. Every remaining bound request then receives
        // one explicit observation-loss settlement before ingress closes.
        self.global_send_completion_observer_future.take();
        let timeline_actors = self
            .timelines
            .drain()
            .map(|(_, handle)| handle)
            .collect::<Vec<_>>();
        for handle in timeline_actors {
            handle.stop().await;
        }
        apply_send_completion_observation_loss_and_handoff(
            &self.send_completion,
            &self.terminal_ingress,
            None,
        );
        // Reject later actor admissions, then drain every handoff accepted before
        // the close. Shutdown acknowledgement is deliberately held until the
        // reducer channel has accepted every action-required terminal in FIFO order.
        self.terminal_ingress
            .close_for_shutdown(&mut self.terminal_rx);
        while let Some(terminal) = self.terminal_rx.recv().await {
            self.handle_send_terminal_handoff(terminal).await;
        }
        for key in room_keys {
            self.clear_thread_root_projections_for_room(&key).await;
        }
        self.thread_root_projection_fetches.abort_all().await;
        if let Some(task) = self.room_subscription_checkpoint_task.take() {
            task.abort();
        }
        if let Some(acknowledged) = shutdown_acknowledgement {
            let _ = acknowledged.send(());
        }
    }
    pub(super) async fn handle_command(&mut self, command: TimelineCommand) {
        self.handle_command_with_permit(command, None).await;
    }
    async fn handle_command_with_formatting_options(
        &mut self,
        command: TimelineCommand,
        formatting_options: ComposerFormattingOptions,
    ) {
        self.handle_command_with_formatting_context(command, None, formatting_options)
            .await;
    }
    async fn handle_command_with_formatting_context(
        &mut self,
        command: TimelineCommand,
        composer_permit: Option<ForwardedComposerDraftPermit>,
        formatting_options: ComposerFormattingOptions,
    ) {
        let previous_options = self.composer_formatting_options;
        self.composer_formatting_options = formatting_options;
        self.handle_command_with_permit(command, composer_permit)
            .await;
        self.composer_formatting_options = previous_options;
    }
    pub(super) async fn handle_command_with_permit(
        &mut self,
        command: TimelineCommand,
        mut composer_permit: Option<ForwardedComposerDraftPermit>,
    ) {
        match command {
            TimelineCommand::Subscribe {
                request_id,
                key,
                initial_backfill,
            } => {
                trace_timeline_route("manager_received", "subscribe", request_id, &key);
                self.handle_subscribe(request_id, key, true, true, initial_backfill)
                    .await;
            }
            TimelineCommand::EnsureSubscribed {
                request_id,
                key,
                replay_existing,
            } => {
                trace_timeline_route("manager_received", "ensure_subscribed", request_id, &key);
                if matches!(key.kind, TimelineKind::Room { .. }) {
                    self.handle_committed_room_selection(request_id, key, replay_existing, true)
                        .await;
                } else {
                    self.handle_subscribe(
                        request_id,
                        key,
                        replay_existing,
                        true,
                        InitialBackfillPolicy::Disabled,
                    )
                    .await;
                }
            }
            TimelineCommand::ReplaySubscribed { request_id } => {
                self.handle_replay_subscribed(request_id).await;
            }
            TimelineCommand::Unsubscribe { request_id, key } => {
                trace_timeline_route("manager_received", "unsubscribe", request_id, &key);
                // Drop the actor handle, which cancels its relay task and drops
                // the SDK Timeline handle — no dedicated success event per spec.
                if matches!(key.kind, TimelineKind::Room { .. }) {
                    self.clear_thread_root_projections_for_room(&key).await;
                } else {
                    self.timeline_actor_generations
                        .invalidate_and_quiesce(&key)
                        .await;
                }
                let removed_actor = self.timelines.remove(&key);
                if removed_actor.is_some() {
                    self.read_workers.remove_local_read_correlation(&key);
                }
                // Release the actor-resource lease only when an actor was
                // actually removed. Session residency is intentionally
                // independent and is never removed by unsubscribe.
                if removed_actor.is_some() {
                    if let Ok(room_id) = key.room_id().parse::<OwnedRoomId>() {
                        self.release_room_lease(&room_id);
                    }
                }
            }
            TimelineCommand::Paginate {
                request_id,
                key,
                direction,
                event_count,
            } => {
                trace_timeline_route("manager_received", "paginate", request_id, &key);
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::Paginate {
                        request_id,
                        direction,
                        event_count,
                    },
                )
                .await;
            }
            TimelineCommand::CancelPagination { request_id, key } => {
                trace_timeline_route("manager_received", "cancel_pagination", request_id, &key);
                if let Some(handle) = self.timelines.get(&key) {
                    let _ = handle
                        .send(TimelineActorMessage::CancelPagination { request_id })
                        .await;
                }
            }
            TimelineCommand::CancelLinkPreviews { request_id, key } => {
                trace_timeline_route("manager_received", "cancel_link_previews", request_id, &key);
                if let Some(handle) = self.timelines.get(&key) {
                    let _ = handle
                        .send(TimelineActorMessage::CancelLinkPreviews { request_id })
                        .await;
                }
            }
            TimelineCommand::RestoreTimelineAnchor {
                request_id,
                key,
                event_id,
                max_batches,
                event_count,
            } => {
                if matches!(&key.kind, TimelineKind::Room { .. }) {
                    self.route_to_actor_or_fail(
                        request_id,
                        &key,
                        TimelineActorMessage::RestoreTimelineAnchor {
                            request_id,
                            event_id,
                            max_batches,
                            event_count,
                        },
                    )
                    .await;
                } else {
                    self.emit(CoreEvent::Timeline(TimelineEvent::AnchorRestoreFinished {
                        request_id,
                        key,
                        status: TimelineAnchorRestoreStatus::Failed {
                            kind: TimelineFailureKind::NotSubscribed,
                        },
                    }));
                }
            }
            TimelineCommand::ObserveViewport {
                request_id,
                key,
                observation,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::ObserveViewport { observation },
                )
                .await;
            }
            TimelineCommand::RepairGaps { request_id, key } => {
                record_timeline_gap_repair("requested", "manual", 0, 0, 0, "queued");
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::InspectTimelineGaps {
                        trigger: TimelineGapRepairTrigger::Manual,
                    },
                )
                .await;
            }
            TimelineCommand::SendText {
                request_id,
                key,
                transaction_id,
                document,
            } => {
                let body = document.plain_body();
                if let Err(kind) = validate_composer_body_for_timeline_send(&body) {
                    self.emit_failure(request_id, CoreFailure::TimelineOperationFailed { kind });
                    return;
                }
                self.route_send_to_worker_or_fail(
                    request_id,
                    &key,
                    transaction_id.clone(),
                    body.clone(),
                    SendComposerProjection::for_send_text(&key),
                    TimelineSendEnqueuePayload::Text {
                        document,
                        formatting_options: self.composer_formatting_options,
                    },
                )
                .await;
            }
            TimelineCommand::SubmitText {
                request_id,
                submission_id,
                key,
                transaction_id,
                document,
                draft_revision,
                ..
            } => {
                let body = document.plain_body();
                if let Err(kind) = validate_composer_body_for_timeline_send(&body) {
                    self.emit(CoreEvent::Timeline(TimelineEvent::SubmissionRejected {
                        request_id,
                        key,
                        submission_id,
                        kind,
                    }));
                    return;
                }
                self.route_submission_to_worker(
                    request_id,
                    submission_id.clone(),
                    &key,
                    transaction_id.clone(),
                    body.clone(),
                    draft_revision,
                    SendComposerProjection::for_send_text(&key),
                    TimelineSendEnqueuePayload::Text {
                        document,
                        formatting_options: self.composer_formatting_options,
                    },
                    composer_permit.take(),
                )
                .await;
            }
            TimelineCommand::SendReply {
                request_id,
                key,
                transaction_id,
                in_reply_to_event_id,
                document,
            } => {
                let body = document.plain_body();
                if let Err(kind) = validate_composer_body_for_timeline_send(&body) {
                    self.emit_failure(request_id, CoreFailure::TimelineOperationFailed { kind });
                    return;
                }
                self.route_send_to_worker_or_fail(
                    request_id,
                    &key,
                    transaction_id.clone(),
                    body.clone(),
                    SendComposerProjection::for_send_reply(&key),
                    TimelineSendEnqueuePayload::Reply {
                        in_reply_to_event_id,
                        document,
                        formatting_options: self.composer_formatting_options,
                    },
                )
                .await;
            }
            TimelineCommand::SubmitReply {
                request_id,
                submission_id,
                key,
                transaction_id,
                in_reply_to_event_id,
                document,
                draft_revision,
                ..
            } => {
                let body = document.plain_body();
                if let Err(kind) = validate_composer_body_for_timeline_send(&body) {
                    self.emit(CoreEvent::Timeline(TimelineEvent::SubmissionRejected {
                        request_id,
                        key,
                        submission_id,
                        kind,
                    }));
                    return;
                }
                self.route_submission_to_worker(
                    request_id,
                    submission_id.clone(),
                    &key,
                    transaction_id.clone(),
                    body.clone(),
                    draft_revision,
                    SendComposerProjection::for_send_reply(&key),
                    TimelineSendEnqueuePayload::Reply {
                        in_reply_to_event_id,
                        document,
                        formatting_options: self.composer_formatting_options,
                    },
                    composer_permit.take(),
                )
                .await;
            }
            TimelineCommand::ForwardMessage {
                request_id,
                key,
                source_event_id,
                destination_room_id,
                transaction_id,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::ForwardMessage {
                        request_id,
                        source_event_id,
                        destination_room_id,
                        transaction_id,
                    },
                )
                .await;
            }
            TimelineCommand::LoadMessageSource {
                request_id,
                key,
                event_id,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::LoadMessageSource {
                        request_id,
                        event_id,
                    },
                )
                .await;
            }
            TimelineCommand::RequestRoomKey {
                request_id,
                key,
                event_id,
                origin,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::RequestRoomKey {
                        request_id: Some(request_id),
                        event_id,
                        origin,
                    },
                )
                .await;
            }
            TimelineCommand::RequestLateDecryption { request_id, key } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::RequestLateDecryption {
                        request_id: Some(request_id),
                        trigger: crate::room_key_receive::RECEIVE_SUMMARY_TRIGGER_MANUAL,
                    },
                )
                .await;
            }
            TimelineCommand::RetrySend {
                request_id,
                key,
                transaction_id,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::RetrySend {
                        request_id,
                        transaction_id,
                    },
                )
                .await;
            }
            TimelineCommand::CancelSend {
                request_id,
                key,
                transaction_id,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::CancelSend {
                        request_id,
                        transaction_id,
                    },
                )
                .await;
            }
            TimelineCommand::UploadAndSendMedia {
                request_id,
                key,
                transaction_id,
                request,
                ..
            } => {
                self.route_media_send_to_worker_or_fail(
                    request_id,
                    &key,
                    transaction_id.clone(),
                    TimelineSendEnqueuePayload::Media {
                        request_id,
                        client_transaction_id: transaction_id,
                        request,
                    },
                )
                .await;
            }
            TimelineCommand::DownloadMedia {
                request_id,
                key,
                event_id,
                selection,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::DownloadMedia {
                        request_id,
                        event_id,
                        selection,
                    },
                )
                .await;
            }
            TimelineCommand::EditText {
                request_id,
                key,
                event_id,
                document,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::EditText {
                        request_id,
                        event_id,
                        document,
                    },
                )
                .await;
            }
            TimelineCommand::Redact {
                request_id,
                key,
                event_id,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::Redact {
                        request_id,
                        event_id,
                    },
                )
                .await;
            }
            TimelineCommand::ToggleReaction {
                request_id,
                key,
                event_id,
                reaction_key,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::ToggleReaction {
                        request_id,
                        event_id,
                        reaction_key,
                    },
                )
                .await;
            }
            TimelineCommand::SendReaction {
                request_id,
                key,
                event_id,
                reaction_key,
            } => {
                trace_timeline_route("manager_received", "send_reaction", request_id, &key);
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::SendReaction {
                        request_id,
                        event_id,
                        reaction_key,
                    },
                )
                .await;
            }
            TimelineCommand::RedactReaction {
                request_id,
                key,
                event_id,
                reaction_key,
                reaction_event_id,
            } => {
                trace_timeline_route("manager_received", "redact_reaction", request_id, &key);
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::RedactReaction {
                        request_id,
                        event_id,
                        reaction_key,
                        reaction_event_id,
                    },
                )
                .await;
            }
            TimelineCommand::SendReadReceipt {
                request_id,
                key,
                event_id,
            } => {
                trace_timeline_route("manager_received", "send_read_receipt", request_id, &key);
                self.route_read_command(request_id, key, event_id, ReadCommandKind::Receipt)
                    .await;
            }
            TimelineCommand::SetFullyRead {
                request_id,
                key,
                event_id,
            } => {
                trace_timeline_route("manager_received", "set_fully_read", request_id, &key);
                self.route_read_command(request_id, key, event_id, ReadCommandKind::FullyRead)
                    .await;
            }
            TimelineCommand::SetTyping {
                request_id,
                key,
                is_typing,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::SetTyping {
                        request_id,
                        is_typing,
                    },
                )
                .await;
            }
            TimelineCommand::LoadLinkPreviews {
                request_id,
                key,
                event_id,
            } => {
                trace_timeline_route("manager_received", "load_link_previews", request_id, &key);
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::LoadLinkPreviews {
                        request_id,
                        event_id,
                    },
                )
                .await;
            }
            TimelineCommand::HideLinkPreview {
                request_id,
                key,
                event_id,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::HideLinkPreview {
                        request_id,
                        event_id,
                    },
                )
                .await;
            }
            TimelineCommand::BroadcastLinkPreviewPolicy {
                unencrypted_global_enabled,
                encrypted_global_enabled,
                room_overrides,
            } => {
                self.link_preview_policy.unencrypted_global_enabled = unencrypted_global_enabled;
                self.link_preview_policy.encrypted_global_enabled = encrypted_global_enabled;
                self.link_preview_policy.room_overrides = room_overrides;
                for (key, handle) in &self.timelines {
                    let room_enabled = self
                        .link_preview_policy
                        .room_overrides
                        .get(key.room_id())
                        .copied();
                    let _ = handle
                        .send(TimelineActorMessage::LinkPreviewPolicyChanged {
                            unencrypted_global_enabled,
                            encrypted_global_enabled,
                            room_enabled,
                        })
                        .await;
                }
            }
        }
    }
    async fn handle_replay_subscribed(&mut self, _request_id: RequestId) {
        for handle in self.timelines.values() {
            let _ = handle
                .send(TimelineActorMessage::ReplayInitialItems {
                    cause_request_id: None,
                })
                .await;
        }
    }
    pub(super) async fn handle_subscribe(
        &mut self,
        request_id: RequestId,
        key: TimelineKey,
        replay_existing: bool,
        emit_failure_terminal: bool,
        initial_backfill: InitialBackfillPolicy,
    ) {
        let trace = |stage: &str| {
            record_subscribe_stage(stage, None);
        };
        trace("start");
        #[cfg(any(test, feature = "test-hooks"))]
        let session_missing = self.session.is_none() && !self.test_session_available;
        #[cfg(not(any(test, feature = "test-hooks")))]
        let session_missing = self.session.is_none();
        if session_missing {
            self.emit_subscription_failure(
                request_id,
                &key,
                TimelineFailureKind::NotSubscribed,
                emit_failure_terminal,
            )
            .await;
            return;
        };

        // Issue #518: the retained actor's room must be proven present in the
        // live Sliding Sync room-subscription set before the cheap replay path
        // is trusted. A presentation-only rebuild elsewhere must never leave a
        // retained actor subscribed to a room the live set no longer covers.
        // The coverage check only applies to an already-retained actor: a
        // genuinely new key is leased and reconciled by the build path below.
        let existing_key = self.timelines.contains_key(&key);
        let subscribed_room_id = existing_key
            .then(|| key.room_id().parse::<OwnedRoomId>().ok())
            .flatten()
            .filter(|room_id| {
                // Verify against the ACTUAL Sliding Sync map, not the logical
                // active set: a session expiry clears the real map without
                // touching the logical set, so the retained actor must restore
                // real coverage before replaying.
                let present = self
                    .room_list_service
                    .as_ref()
                    .is_some_and(|service| service.actual_subscribed_rooms().contains(room_id));
                if present {
                    koushi_diagnostics::increment_counter("subscription_coverage_present");
                } else {
                    koushi_diagnostics::increment_counter("subscription_coverage_missing");
                }
                !present
            });
        if let Some(room_id) = &subscribed_room_id {
            // Restore the missing room coverage through the lease owner before
            // replaying. The key already holds its lease (acquired at actor
            // creation); never double-lease on a coverage recovery.
            self.session_subscribed_rooms.insert(room_id.clone());
            if !self.room_is_leased(key.room_id()) {
                self.lease_room(room_id.clone());
            }
            self.reconcile_subscriptions(SubscriptionReconcileTrigger::TimelineRebuild)
                .await;
        }

        // Idempotency: if the identical key is already subscribed, do NOT drop
        // and rebuild the SDK subscription.  The full rebuild was 4-8 expensive
        // `subscribe_to_rooms` / timeline-build cycles per room on snapshot
        // churn (issue #116).  Callers that need to populate an empty
        // TimelineView can request an InitialItems replay; room-selection
        // effects with an already-retained App store can skip that full replay.
        let replay_result = if let Some(handle) = self.timelines.get(&key) {
            if replay_existing {
                let deadline = executor::Instant::now() + LIVE_TAIL_CANCELLATION_DEADLINE;
                Some(
                    executor::timeout_at(
                        deadline,
                        handle.send_control(TimelineActorControl::ReplayInitialItems {
                            cause_request_id: request_id,
                        }),
                    )
                    .await
                    .map_err(|_| ())
                    .and_then(|sent| sent.then_some(()).ok_or(())),
                )
            } else {
                Some(Ok(()))
            }
        } else {
            None
        };
        match replay_result {
            Some(Ok(())) => {
                // Re-emit the subscribed action so the reducer re-confirms
                // `is_subscribed = true` (idempotent in the reducer).
                self.emit_timeline_subscribed_action(&key).await;
                if !replay_existing {
                    trace("replay_initial_skipped");
                }
                self.retry_pending_send_hydrations(&key);
                trace("subscribed_done");
                return;
            }
            Some(Err(_)) => {
                // Mailbox full or closed: the cheap replay could not be
                // delivered. Fall through to a full rebuild, but keep the old
                // handle until the replacement actor is built successfully.
                trace("replay_initial_failed");
            }
            None => {}
        }

        let activation = self
            .timeline_actor_generations
            .activate_after_quiescence(&key)
            .await;

        // Admit session residency before reconciling. The actor-resource lease
        // is separate and is rolled back if actor construction fails.
        let reconcile_trigger = match &key.kind {
            TimelineKind::Room { .. } => SubscriptionReconcileTrigger::RoomSelected,
            TimelineKind::Thread { .. } => SubscriptionReconcileTrigger::ThreadOpened,
            TimelineKind::Focused { .. } => SubscriptionReconcileTrigger::FocusedOpened,
        };
        let lease_room_id = key.room_id().parse::<OwnedRoomId>().ok();
        if let Some(room_id) = &lease_room_id {
            self.session_subscribed_rooms.insert(room_id.clone());
        }
        // The lease is per TimelineKey, not per actor instance: a replacement
        // (replay-failure rebuild) transfers the existing key lease, so only a
        // genuinely new key acquires one.
        let lease_added = lease_room_id.as_ref().is_some_and(|room_id| {
            if self.timelines.contains_key(&key) {
                false
            } else {
                self.lease_room(room_id.clone());
                true
            }
        });
        self.reconcile_subscriptions(reconcile_trigger).await;
        let subscription_generation = self
            .room_list_service
            .as_ref()
            .map(|service| service.subscription_generation().get());
        match self
            .build_timeline_actor_handle(
                request_id,
                &key,
                activation.generation,
                subscription_generation,
                initial_backfill,
            )
            .await
        {
            Ok(handle) => {
                self.emit_timeline_subscribed_action(&key).await;
                if let Some(previous) = self.timelines.insert(key.clone(), handle) {
                    previous.stop().await;
                }
                self.replay_retained_room_subscription_checkpoint(&key)
                    .await;
                self.retry_pending_send_hydrations(&key);
                trace("subscribed_done");
            }
            Err(kind) => {
                if lease_added {
                    if let Some(room_id) = &lease_room_id {
                        self.release_room_lease(room_id);
                    }
                }
                // Keep session residency after a failed actor build; only the
                // actor-resource lease is rolled back.
                self.reconcile_subscriptions(reconcile_trigger).await;
                self.timeline_actor_generations
                    .restore_failed_activation(&key, activation);
                self.emit_subscription_failure(request_id, &key, kind, emit_failure_terminal)
                    .await;
            }
        }
    }
    pub(super) async fn build_timeline_actor_handle(
        &mut self,
        request_id: RequestId,
        key: &TimelineKey,
        actor_generation: u64,
        subscription_generation: Option<u64>,
        initial_backfill: InitialBackfillPolicy,
    ) -> Result<TimelineActorHandle, TimelineFailureKind> {
        let trace = |stage: &str| {
            record_subscribe_stage(stage, None);
        };
        let room_id_str = match &key.kind {
            TimelineKind::Room { room_id } => room_id.clone(),
            TimelineKind::Thread { room_id, .. } => room_id.clone(),
            TimelineKind::Focused { room_id, .. } => room_id.clone(),
        };

        let room_id = match matrix_sdk::ruma::RoomId::parse(&room_id_str) {
            Ok(id) => id,
            Err(_) => return Err(TimelineFailureKind::Sdk),
        };

        // Issue #518: building a Timeline actor must NOT mutate the live
        // room-subscription set. The room was already subscribed through the
        // room-ID lease + reconciliation in `handle_subscribe` (or, for sync
        // rebuilds, by `handle_sync_started`). The caller passes the current
        // subscription generation for checkpoint matching.
        let focus = match &key.kind {
            TimelineKind::Room { .. } => TimelineFocus::Live {
                hide_threaded_events: false,
            },
            TimelineKind::Thread { root_event_id, .. } => {
                match matrix_sdk::ruma::EventId::parse(root_event_id.as_str()) {
                    Ok(event_id) => TimelineFocus::Thread {
                        root_event_id: event_id,
                    },
                    Err(_) => return Err(TimelineFailureKind::Sdk),
                }
            }
            TimelineKind::Focused { event_id, .. } => {
                match matrix_sdk::ruma::EventId::parse(event_id.as_str()) {
                    Ok(eid) => TimelineFocus::Event {
                        target: eid,
                        num_context_events: 20,
                        thread_mode:
                            matrix_sdk_ui::timeline::TimelineEventFocusThreadMode::Automatic {
                                hide_threaded_events: false,
                            },
                    },
                    Err(_) => return Err(TimelineFailureKind::Sdk),
                }
            }
        };

        #[cfg(any(test, feature = "test-hooks"))]
        if self.session.is_none() {
            return Ok(Self::room_subscription_residency_test_actor_handle());
        }

        let Some(session) = &self.session else {
            return Err(TimelineFailureKind::NotSubscribed);
        };
        let client = session.client();
        let room = match client.get_room(&room_id) {
            Some(room) => room,
            None => return Err(TimelineFailureKind::Sdk),
        };

        trace("build_begin");
        let build_started = Some(startup_trace::now());
        let timeline_result = koushi_timeline_builder(&room, focus).build().await;
        startup_trace::trace_phase(StartupPhase::TimelineBuild, build_started);
        trace("build_done");

        let timeline = match timeline_result {
            Ok(t) => Arc::new(t),
            Err(_) => return Err(TimelineFailureKind::Sdk),
        };

        if matches!(
            initial_backfill,
            InitialBackfillPolicy::RequiredForExistingThread
        ) && matches!(key.kind, TimelineKind::Thread { .. })
        {
            let (initial_items, _) = timeline.subscribe().await;
            if initial_items.is_empty() {
                let _permit = self
                    .account_work
                    .acquire(AccountWorkKind::ExplicitPagination)
                    .await;
                let end_reached = timeline
                    .paginate_backwards(INITIAL_EMPTY_ROOM_BACKFILL_EVENT_COUNT)
                    .await
                    .map_err(|_| TimelineFailureKind::Sdk)?;
                let (settled_items, _) = timeline.subscribe().await;
                if !initial_thread_backfill_is_authoritative(end_reached, settled_items.len()) {
                    return Err(TimelineFailureKind::Sdk);
                }
            }
        }

        trace("spawn_begin");
        let handle = TimelineActor::spawn(
            key.clone(),
            timeline,
            session.clone(),
            request_id,
            self.read_workers.send_read_receipts_enabled(),
            self.action_tx.clone(),
            self.event_tx.clone(),
            self.search_index_tx.clone(),
            self.ignored_user_ids.clone(),
            self.data_dir.clone(),
            self.link_preview_policy.for_room(key.room_id()),
            self.account_work.clone(),
            Arc::clone(&self.thread_root_projection_service),
            self.thread_root_order,
            Arc::clone(&self.timeline_actor_generations),
            actor_generation,
            subscription_generation,
            Arc::clone(&self.send_completion),
            self.terminal_ingress.clone(),
            self.msg_tx.clone(),
        )
        .await;
        trace("spawn_done");

        Ok(handle)
    }
    async fn route_to_actor_or_fail(
        &mut self,
        request_id: RequestId,
        key: &TimelineKey,
        msg: TimelineActorMessage,
    ) {
        match self.timelines.get(key) {
            Some(handle) => {
                let _ = handle.send(msg).await;
            }
            None => {
                self.emit_failure(
                    request_id,
                    CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::NotSubscribed,
                    },
                );
            }
        }
    }
    pub(super) fn emit(&self, event: CoreEvent) {
        let _ = self.event_tx.send(event);
    }
    pub(super) fn emit_failure(&self, request_id: RequestId, failure: CoreFailure) {
        self.emit(CoreEvent::OperationFailed {
            request_id,
            failure,
        });
    }
    pub(super) async fn emit_subscription_failure(
        &mut self,
        request_id: RequestId,
        key: &TimelineKey,
        kind: TimelineFailureKind,
        emit_failure_terminal: bool,
    ) {
        if let Some(action) = timeline_subscription_failed_action(key) {
            if emit_failure_terminal {
                let _ = self.action_tx.send(vec![action]).await;
            } else {
                let _ = self.action_tx.try_send(vec![action]);
            }
        }
        if emit_failure_terminal {
            self.emit_failure(request_id, CoreFailure::TimelineOperationFailed { kind });
        }
    }
    pub(super) async fn emit_timeline_subscribed_action(&mut self, key: &TimelineKey) {
        let action = match &key.kind {
            TimelineKind::Room { room_id } => AppAction::TimelineSubscribed {
                room_id: room_id.clone(),
            },
            TimelineKind::Thread {
                room_id,
                root_event_id,
            } => AppAction::ThreadSubscribed {
                room_id: room_id.clone(),
                root_event_id: root_event_id.clone(),
            },
            TimelineKind::Focused { room_id, event_id } => AppAction::FocusedContextSubscribed {
                room_id: room_id.clone(),
                event_id: event_id.clone(),
            },
        };
        let _ = self.action_tx.send(vec![action]).await;
    }
    pub(super) async fn emit_action_reliable(&mut self, action: AppAction) -> bool {
        emit_app_action_reliable(&self.action_tx, action).await
    }
}

pub(super) fn internal_timeline_request_id() -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(0),
        sequence: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_source::item_body;

    use futures_util::StreamExt;

    #[test]
    fn existing_thread_initial_backfill_requires_items_or_authoritative_end() {
        assert!(super::initial_thread_backfill_is_authoritative(true, 0));
        assert!(super::initial_thread_backfill_is_authoritative(false, 1));
        assert!(!super::initial_thread_backfill_is_authoritative(false, 0));
    }
}
