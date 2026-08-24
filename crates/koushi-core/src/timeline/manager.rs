use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use futures_util::StreamExt;
use koushi_sdk::{
    MatrixClientSession, MatrixCommittedRoomTimelineCheckpoint as MatrixRoomSubscriptionCheckpoint,
    MatrixLiveTailRefreshOutcome, MatrixOutboundGroupSessionToken, MatrixRoomKeyReshareTarget,
};
use koushi_state::{AppAction, ComposerFormattingOptions, OperationFailureKind};

use matrix_sdk::ruma::OwnedRoomId;
use matrix_sdk_ui::timeline::TimelineFocus;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::account_work::AccountWorkScheduler;
use crate::command::TimelineCommand;
use crate::event::{CoreEvent, TimelineAnchorRestoreStatus, TimelineEvent, TimelineItem};
use crate::executor;
use crate::failure::{CoreFailure, TimelineFailureKind};
use crate::ids::{
    RequestId, RuntimeConnectionId, TimelineBatchId, TimelineGeneration, TimelineKey, TimelineKind,
};
use crate::link_preview::LinkPreviewContext;
#[cfg(test)]
use crate::live_tail_freshness::LiveTailFreshnessState;
use crate::live_tail_freshness::LiveTailRefreshCoordinator;
use crate::read_state::{ReadPersistenceSnapshot, ReadStateKey};
use crate::runtime::ForwardedComposerDraftPermit;
use crate::search::SearchIndexMessage;
use crate::startup_trace::{self, StartupPhase};
use crate::threads_list::{
    AggregateRefresh, ThreadRootProjectionActivity, ThreadRootProjectionRefreshResult,
    ThreadRootProjectionService,
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
    NavigationProjectionIntent, TimelineActorGenerationGate, TimelineProjectionAcknowledgement,
    receive_navigation_projection,
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
use super::room_key_recovery::RoomKeyReshareCompletion;
use super::thread_projection::{
    ReplayKnownThreadRootProjectionRegistry, ThreadRootProjectionFetchRegistry,
};
// END GENERATED SIBLING IMPORTS

/// Bounded diff queue capacity per subscribed timeline (overview.md, Async rule 10).

pub const TIMELINE_DIFF_QUEUE_CAPACITY: usize = 128;

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
    AcknowledgeProjection {
        projection_request_id: RequestId,
        key: TimelineKey,
        generation: TimelineGeneration,
        response: oneshot::Sender<TimelineProjectionAcknowledgement>,
    },
    AcknowledgeBatchRendered {
        key: TimelineKey,
        actor_generation: u64,
        timeline_generation: TimelineGeneration,
        repair_generation: u64,
        batch_id: TimelineBatchId,
    },
    /// Sync started: carries the one live `RoomListService`. Subscribing a timeline must also
    /// subscribe its room with the live service so the server streams that
    /// room's new timeline events (canon: TimelineActor description; without
    /// this on servers that only deliver the initial window).
    SyncStarted {
        room_list_service: Arc<matrix_sdk_ui::room_list_service::RoomListService>,
        core_generation: u64,
    },
    #[cfg(feature = "test-hooks")]
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
    /// A Room actor observed an absent thread root and has already committed
    /// its pending state transition. The manager owns the resulting worker so
    /// unsubscribe/shutdown can cancel it deterministically.
    #[cfg(test)]
    StartThreadRootProjectionFetch {
        key: TimelineKey,
        actor_generation: u64,
        own_user_id: Option<matrix_sdk::ruma::OwnedUserId>,
        activities: Vec<ThreadRootProjectionActivity>,
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
    RoomKeyReshareCompleted {
        key: TimelineKey,
        actor_generation: u64,
        expected_session: MatrixOutboundGroupSessionToken,
        target: MatrixRoomKeyReshareTarget,
        attempt: u8,
        outcome: RoomKeyReshareCompletion,
    },
    RunRoomKeyReshare {
        key: TimelineKey,
        actor_generation: u64,
        expected_session: MatrixOutboundGroupSessionToken,
        target: MatrixRoomKeyReshareTarget,
        attempt: u8,
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
    Shutdown {
        acknowledged: oneshot::Sender<()>,
    },
}

impl TimelineManagerHandle {
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

    #[cfg(feature = "test-hooks")]
    pub(crate) async fn residency_snapshot_for_testing(
        &self,
        response: oneshot::Sender<(Vec<String>, Vec<String>)>,
    ) -> bool {
        self.send(TimelineMessage::ResidencyTestSnapshot { response })
            .await
    }

    #[cfg(feature = "test-hooks")]
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
    #[cfg(feature = "test-hooks")]
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
    pub(super) account_work: AccountWorkScheduler,
    /// Room-root hydration is shared across replacement actors so SyncStarted
    /// cannot restart a failed/pending bounded lookup.
    pub(super) thread_root_projection_service: Arc<Mutex<ThreadRootProjectionService>>,
    pub(super) thread_root_projection_fetches: ThreadRootProjectionFetchRegistry,
    /// Ready snapshots copied from bounded replays, tracked separately so
    /// unsubscribe/shutdown can explicitly clear retained frontend rows.
    pub(super) replay_known_thread_root_projections:
        Arc<Mutex<ReplayKnownThreadRootProjectionRegistry>>,
    /// Serializes ownership transfer between replacement actors before either
    /// generation may touch the shared replay-known registry.
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
    ) -> TimelineManagerHandle {
        let (tx, msg_rx) = mpsc::channel(crate::runtime::ACTOR_MESSAGE_QUEUE_CAPACITY);
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
            #[cfg(feature = "test-hooks")]
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
            replay_known_thread_root_projections: Arc::new(Mutex::new(
                ReplayKnownThreadRootProjectionRegistry::default(),
            )),
            timeline_actor_generations: Arc::new(TimelineActorGenerationGate::default()),
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
    ) -> TimelineManagerHandle {
        let (tx, msg_rx) = mpsc::channel(crate::runtime::ACTOR_MESSAGE_QUEUE_CAPACITY);
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
            #[cfg(feature = "test-hooks")]
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
            replay_known_thread_root_projections: Arc::new(Mutex::new(
                ReplayKnownThreadRootProjectionRegistry::default(),
            )),
            timeline_actor_generations: Arc::new(TimelineActorGenerationGate::default()),
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
                        self.handle_send_enqueue_worker_completion(completion);
                    }
                    continue;
                }
                _ = self.send_enqueue_workers.diagnostic_tasks.next(),
                    if !self.send_enqueue_workers.diagnostic_tasks.is_empty() => {
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
                #[cfg(feature = "test-hooks")]
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
                #[cfg(test)]
                TimelineMessage::StartThreadRootProjectionFetch {
                    key,
                    actor_generation,
                    own_user_id,
                    activities,
                } => {
                    self.handle_thread_root_projection_fetch_start(
                        key,
                        actor_generation,
                        own_user_id,
                        activities,
                    )
                    .await;
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
                TimelineMessage::RoomKeyReshareCompleted {
                    key,
                    actor_generation,
                    expected_session,
                    target,
                    attempt,
                    outcome,
                } => {
                    self.handle_room_key_reshare_completed(
                        key,
                        actor_generation,
                        expected_session,
                        target,
                        attempt,
                        outcome,
                    )
                    .await;
                }
                TimelineMessage::RunRoomKeyReshare {
                    key,
                    actor_generation,
                    expected_session,
                    target,
                    attempt,
                } => {
                    self.handle_room_key_reshare(
                        key,
                        actor_generation,
                        expected_session,
                        target,
                        attempt,
                    );
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
                TimelineMessage::AcknowledgeProjection {
                    projection_request_id,
                    key,
                    generation,
                    response,
                } => {
                    let accepted = self
                        .acknowledge_projection(projection_request_id, &key, generation)
                        .await;
                    let _ = response.send(accepted);
                }
                TimelineMessage::AcknowledgeBatchRendered {
                    key,
                    actor_generation,
                    timeline_generation,
                    repair_generation,
                    batch_id,
                } => {
                    self.acknowledge_batch_rendered(
                        &key,
                        actor_generation,
                        timeline_generation,
                        repair_generation,
                        batch_id,
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
        self.send_enqueue_workers.room_key_reshares.clear();
        self.send_enqueue_workers.cancel_diagnostics();
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
        self.thread_root_projection_fetches.abort_all();
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
            TimelineCommand::Subscribe { request_id, key } => {
                trace_timeline_route("manager_received", "subscribe", request_id, &key);
                self.handle_subscribe(request_id, key, true, true).await;
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
                    self.handle_subscribe(request_id, key, replay_existing, true)
                        .await;
                }
            }
            TimelineCommand::ReplaySubscribed { request_id } => {
                self.handle_replay_subscribed(request_id).await;
            }
            TimelineCommand::Unsubscribe { request_id, key } => {
                trace_timeline_route("manager_received", "unsubscribe", request_id, &key);
                self.send_enqueue_workers.room_key_reshares.remove(&key);
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
    async fn acknowledge_projection(
        &mut self,
        projection_request_id: RequestId,
        key: &TimelineKey,
        generation: TimelineGeneration,
    ) -> TimelineProjectionAcknowledgement {
        let Some(handle) = self.timelines.get(key) else {
            return TimelineProjectionAcknowledgement::default();
        };
        let (response, accepted) = oneshot::channel();
        if !handle
            .send(TimelineActorMessage::AcknowledgeProjection {
                projection_request_id,
                generation,
                response,
            })
            .await
        {
            return TimelineProjectionAcknowledgement::default();
        }
        accepted.await.unwrap_or_default()
    }
    async fn acknowledge_batch_rendered(
        &mut self,
        key: &TimelineKey,
        actor_generation: u64,
        timeline_generation: TimelineGeneration,
        repair_generation: u64,
        batch_id: TimelineBatchId,
    ) {
        let Some(handle) = self.timelines.get(key) else {
            return;
        };
        let _ = handle
            .send(TimelineActorMessage::AcknowledgeBatchRendered {
                actor_generation,
                timeline_generation,
                repair_generation,
                batch_id,
            })
            .await;
    }
    pub(super) async fn handle_subscribe(
        &mut self,
        request_id: RequestId,
        key: TimelineKey,
        replay_existing: bool,
        emit_failure_terminal: bool,
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
            )
            .await
        {
            Ok(handle) => {
                self.emit_timeline_subscribed_action(&key).await;
                self.send_enqueue_workers.room_key_reshares.remove(&key);
                if let Some(previous) = self.timelines.insert(key.clone(), handle) {
                    previous.stop().await;
                }
                self.replay_retained_room_subscription_checkpoint(&key)
                    .await;
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

        #[cfg(feature = "test-hooks")]
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
            Arc::clone(&self.replay_known_thread_root_projections),
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
    fn room_subscribe_success_reduces_timeline_subscribed_action() {
        let source = include_str!("manager.rs");
        let fn_offset = source
            .find("async fn handle_subscribe")
            .expect("handle_subscribe should exist");
        let rest = &source[fn_offset..];
        let end = rest
            .find("async fn route_to_actor_or_fail")
            .expect("next helper should exist");
        let handle_subscribe_source = &rest[..end];
        let spawn_token = concat!("TimelineActor::", "spawn");
        let action_token = concat!("emit_timeline", "_subscribed_action");
        let room_token = concat!("TimelineKind::", "Room");
        let build_helper_source = source
            .split("async fn build_timeline_actor_handle")
            .nth(1)
            .expect("subscribe build helper should exist")
            .split("async fn route_to_actor_or_fail")
            .next()
            .expect("route helper should follow subscribe build helper");
        let success_path = handle_subscribe_source
            .split("Ok(handle) =>")
            .nth(1)
            .expect("subscribe success should handle the built timeline actor");
        let action_offset = success_path
            .find(action_token)
            .expect("subscribe success should reduce TimelineSubscribed");

        assert!(
            build_helper_source.contains(spawn_token),
            "subscribe success should spawn the timeline actor"
        );
        assert!(
            action_offset > 0,
            "TimelineSubscribed should be reduced only after subscribe succeeds"
        );
        assert!(
            source.contains(room_token),
            "main timeline subscription state should only be marked for room timelines"
        );
    }

    #[test]
    fn timeline_subscribe_settles_use_reliable_reducer_actions() {
        let subscribed_helper = item_body(
            include_str!("manager.rs"),
            "async fn emit_timeline_subscribed_action",
        );
        let failure_helper = item_body(
            include_str!("diagnostics.rs"),
            "fn timeline_subscription_failed_action",
        );
        let subscribe_body = item_body(include_str!("manager.rs"), "async fn handle_subscribe");
        assert!(
            subscribed_helper.contains("self.action_tx.send(vec![action]).await"),
            "timeline subscribe success must enqueue reducer settle actions reliably"
        );
        assert!(
            !subscribed_helper.contains("try_send(vec![action])"),
            "timeline subscribe success must not use drop-on-full try_send"
        );
        assert!(
            failure_helper.contains("TimelineKind::Room { .. } => None"),
            "main room subscription failures use the existing main timeline failure path"
        );
        assert!(
            failure_helper.contains("AppAction::ThreadSubscriptionFailed"),
            "thread subscription failures must settle the thread opening state"
        );
        assert!(
            failure_helper.contains("AppAction::FocusedContextSubscriptionFailed"),
            "focused subscription failures must settle the focused opening state"
        );
        assert!(
            subscribe_body.contains("self.emit_subscription_failure"),
            "subscribe failure branches must emit reducer failure actions"
        );
    }

    #[test]
    fn thread_timeline_focus_uses_sdk_thread_pagination() {
        let source = include_str!("manager.rs");
        let focus_source = source
            .split("let focus = match &key.kind")
            .nth(1)
            .expect("subscribe focus match should exist")
            .split("let timeline_result")
            .next()
            .expect("timeline build should follow focus selection");
        let thread_focus = focus_source
            .split("TimelineKind::Thread")
            .nth(1)
            .expect("thread timeline focus arm should exist")
            .split("TimelineKind::Focused")
            .next()
            .expect("focused timeline focus arm should follow thread arm");

        assert!(
            thread_focus.contains("TimelineFocus::Thread"),
            "thread panes should use SDK thread timelines so pagination follows thread relations"
        );
        assert!(
            !thread_focus.contains("TimelineFocus::Event"),
            "thread panes must not use event-context focus because later thread replies can be outside the context window"
        );
    }

    /// Contract: re-ensuring an already-subscribed identical key takes the
    /// cheap path — ask the existing actor to replay InitialItems for the new
    /// request_id and return early, so NO `subscribe_to_rooms` / timeline
    /// teardown happens on snapshot churn. Only when the cheap replay cannot be
    /// delivered (mailbox full/closed) does it fall back to a full rebuild so a
    /// re-mounted view is still guaranteed InitialItems. A different (new) key
    /// always falls through to the full subscribe path.
    #[test]
    fn timeline_subscribe_is_idempotent_for_existing_key() {
        let source = include_str!("manager.rs");
        let handle_subscribe_source = source
            .split("async fn handle_subscribe")
            .nth(1)
            .expect("handle_subscribe should exist")
            .split("async fn route_to_actor_or_fail")
            .next()
            .expect("route helper should follow handle_subscribe");

        // The existing-key branch must be present and must end with an early
        // return — proving the full SDK rebuild is skipped.
        let existing_key_branch = handle_subscribe_source
            .split("let replay_result = if let Some(handle) = self.timelines.get(&key)")
            .nth(1)
            .expect("handle_subscribe must detect an already-subscribed key via timelines.get")
            .split("let client = session.client()")
            .next()
            .expect("existing-key branch must precede the new-key SDK path");

        assert!(
            existing_key_branch.contains("ReplayInitialItems"),
            "re-ensuring an already subscribed timeline must send ReplayInitialItems to the existing actor (no SDK teardown on the success path)"
        );
        assert!(
            existing_key_branch.contains("return;"),
            "the cheap replay path must return early, skipping subscribe_to_rooms and the full SDK rebuild"
        );
        // The success (Ok) arm does the cheap replay and returns; the existing-key
        // branch never re-runs the SDK `subscribe_to_rooms` (which lives after
        // `let client = session.client()`). An undeliverable replay (full/closed
        // mailbox) intentionally falls back to a full rebuild via
        // `self.timelines.remove(&key)`, so no "must-not-remove" assertion here.

        // The new-key (full subscribe) path must still delegate to the actor
        // builder, which subscribes the room with a generation checkpoint and
        // builds a fresh timeline.
        let build_helper_source = source
            .split("async fn build_timeline_actor_handle")
            .nth(1)
            .expect("timeline actor build helper should exist")
            .split("async fn route_to_actor_or_fail")
            .next()
            .expect("route helper should follow timeline actor build helper");

        assert!(
            !build_helper_source.contains("subscribe_to_rooms_with_generation"),
            "building a Timeline actor must not mutate the room-subscription set (issue #518)"
        );
        assert!(
            handle_subscribe_source.contains("lease_room"),
            "a new (not yet subscribed) key must lease its room before building"
        );
        assert!(
            handle_subscribe_source.contains("reconcile_subscriptions"),
            "a new (not yet subscribed) key must reconcile the room-subscription set"
        );
    }

    #[test]
    fn sync_started_subscribes_existing_timeline_rooms_with_live_room_list_service() {
        let run_source = item_body(include_str!("manager.rs"), "async fn run(mut self)");
        let sync_started_handler =
            item_body(include_str!("residency.rs"), "async fn handle_sync_started");
        let rebuild_handler = item_body(
            include_str!("residency.rs"),
            "async fn rebuild_existing_room_timelines_after_sync_started",
        );
        assert!(
            run_source.contains("self.handle_sync_started(room_list_service, core_generation)"),
            "SyncStarted must subscribe already-open timeline rooms with the live RoomListService; otherwise room summaries can update while existing timeline actors miss remote events"
        );
        assert!(
            sync_started_handler
                .contains("self.room_list_service = Some(room_list_service.clone());"),
            "the live RoomListService handle must still be retained for future timeline subscriptions"
        );
        assert!(
            sync_started_handler
                .contains("self.subscribe_existing_timeline_rooms(&room_list_service)"),
            "already-open timeline actors must have their rooms subscribed when SyncStarted arrives after actor creation"
        );
        assert!(
            sync_started_handler.contains("rebuild_existing_room_timelines_after_sync_started")
                && sync_started_handler.contains(".await"),
            "late SyncStarted must rebuild already-open room live timelines so fresh InitialItems repair events missed before the live RoomListService handoff"
        );
        assert!(
            include_str!("residency.rs").contains("session_subscribed_rooms"),
            "existing timeline room subscriptions must be derived from session residency"
        );
        assert!(
            include_str!("residency.rs").contains("reconcile_room_subscriptions_with_generation"),
            "existing timeline rooms must be reconciled atomically with the live RoomListService"
        );
        assert!(
            rebuild_handler.contains("matches!(key.kind, TimelineKind::Room { .. })"),
            "only room live timelines should be rebuilt on SyncStarted; focused/thread contexts should not be reset"
        );
        assert!(
            !rebuild_handler.contains("self.timelines.remove(&key);"),
            "sync-start rebuild must not drop the existing room timeline before the replacement subscribe succeeds"
        );
        assert!(
            rebuild_handler.contains("replace_existing_room_timeline_after_sync_started"),
            "sync-start rebuild must build a replacement actor and swap it in only after success"
        );
    }

    #[test]
    fn timeline_ensure_subscribed_can_skip_existing_actor_replay() {
        let source = include_str!("manager.rs");
        let handle_command = source
            .split("async fn handle_command(&mut self, command: TimelineCommand)")
            .nth(1)
            .expect("handle_command should exist")
            .split("async fn handle_command_with_permit")
            .next()
            .expect("command-with-permit helper should follow handle_command");
        let handle_command_with_permit = source
            .split("async fn handle_command_with_permit")
            .nth(1)
            .expect("handle_command_with_permit should exist")
            .split("async fn route_send_to_worker_or_fail")
            .next()
            .expect("send routing should follow command handling");
        let handle_subscribe_source = source
            .split("async fn handle_subscribe")
            .nth(1)
            .expect("handle_subscribe should exist")
            .split("let client = session.client()")
            .next()
            .expect("existing-key branch should precede the SDK subscribe path");

        assert!(
            handle_command.contains("self.handle_command_with_permit(command, None).await"),
            "plain timeline commands should delegate through the permit-aware command helper"
        );
        assert!(
            handle_command_with_permit.contains("TimelineCommand::EnsureSubscribed"),
            "timeline manager should expose an explicit ensure-subscription path for callers that do not need item replay"
        );
        assert!(
            handle_command_with_permit.contains("replay_existing"),
            "ensure-subscription routing must pass through whether an existing actor should replay InitialItems"
        );
        assert!(
            handle_subscribe_source.contains("if replay_existing"),
            "existing actors should only replay InitialItems when the caller explicitly requests replay"
        );
    }

    #[test]
    fn replay_subscribed_recovery_replays_initial_items_causeless_for_all_timelines() {
        let source = include_str!("manager.rs");
        let handle_command_with_permit = source
            .split("async fn handle_command_with_permit")
            .nth(1)
            .expect("handle_command_with_permit should exist")
            .split("async fn route_send_to_worker_or_fail")
            .next()
            .expect("send routing should follow command handling");
        let replay_handler = source
            .split("async fn handle_replay_subscribed")
            .nth(1)
            .expect("TimelineManagerActor should expose subscribed-timeline replay")
            .split("async fn handle_subscribe")
            .next()
            .expect("handle_subscribe should follow replay handler");

        assert!(
            handle_command_with_permit.contains("TimelineCommand::ReplaySubscribed { request_id }")
                && handle_command_with_permit
                    .contains("self.handle_replay_subscribed(request_id).await"),
            "TimelineManagerActor must route replay-subscribed commands to the replay helper"
        );
        assert!(
            replay_handler.contains("for handle in self.timelines.values()")
                && replay_handler.contains("TimelineActorMessage::ReplayInitialItems")
                && replay_handler.contains("cause_request_id: None")
                && replay_handler.contains(".await"),
            "recovery must ask every subscribed TimelineActor for a causeless InitialItems replay"
        );
    }
}
