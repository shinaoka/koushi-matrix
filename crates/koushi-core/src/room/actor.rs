use super::list_observer::{
    RoomListObservation, RoomListObservationCommand, room_stop_matches_generation,
};
use super::mentions::MentionDemand;
use super::operations::SpaceChildLinkKey;
#[cfg(any(test, feature = "test-hooks"))]
use super::operations::{RoomOperationTestControl, RoomOperationTestControlSlot};
use super::space_members::{SpaceMemberDemand, SpaceMemberRefreshFence};
use crate::account_work::AccountWorkScheduler;
use crate::executor;
use crate::timeline::TimelineSubscriptionResidencyHandle;
#[cfg(any(test, feature = "test-hooks"))]
use crate::timeline::{RoomMembershipTransition, VisibleRoomObservation};
use koushi_protocol::command::RoomCommand;
use koushi_protocol::event::CoreEvent;
use koushi_protocol::failure::CoreFailure;
use koushi_protocol::ids::RequestId;
#[cfg(test)]
use koushi_protocol::ids::RuntimeConnectionId;
use koushi_sdk::{
    MatrixClientSession, MatrixJoinedMemberSnapshot, MatrixRoomOperationError,
    MatrixSpaceMembersProjection,
};
use koushi_state::{AppAction, MentionSurface, RoomListFailureKind, RoomListSource};
#[cfg(any(test, feature = "test-hooks"))]
use std::sync::{Mutex, atomic::AtomicUsize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

const ROOM_ACTOR_SHUTDOWN_SEND_TIMEOUT: Duration = Duration::from_secs(1);

pub(super) const ROOM_ACTOR_SHUTDOWN_JOIN_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) const ROOM_OBSERVATION_SHUTDOWN_JOIN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MissingSpaceChildLink {
    pub(super) space_id: String,
    pub(super) child_room_id: String,
    pub(super) via_server: String,
}

/// Messages sent to the RoomActor from AccountActor / SyncActor.
pub enum RoomMessage {
    /// Route a `RoomCommand` to the actor.
    Command(RoomCommand),
    #[cfg(any(test, feature = "test-hooks"))]
    TestCommand {
        command: RoomCommand,
        processed: oneshot::Sender<()>,
    },
    /// A store-backed session was established (login/restore/switch).
    /// Enables room operations; does NOT start the room-list observation —
    /// that starts on `SyncStarted` when its live `RoomListService` is known.
    SessionEstablished { session: Arc<MatrixClientSession> },
    /// Sync started. Sent by `SyncActor` after the backend is launched.
    /// `room_list_service` is the ONE live service owned by the running
    /// `SyncService`. Ad-hoc `RoomListService` instances are prohibited.
    SyncStarted {
        session: Arc<MatrixClientSession>,
        room_list_service: Arc<matrix_sdk_ui::room_list_service::RoomListService>,
        source: RoomListSource,
        backend_generation: u64,
    },
    /// A committed response must not become connected until the matching
    /// response has been projected reliably. Empty accounts may omit the SDK
    /// room count, so that response is treated as a complete empty projection.
    ReconcileCommittedRange {
        source: RoomListSource,
        backend_generation: u64,
        response_sequence: u64,
        ack: oneshot::Sender<RoomListReconcileAck>,
    },
    /// Re-project the current entries after a committed Sliding Sync response.
    /// The source remains the single live `RoomListService`; this wake only
    /// closes the response/store publication window for membership changes.
    RefreshCommittedProjection {
        source: RoomListSource,
        backend_generation: u64,
    },
    /// Re-project after the base client reports an invite/leave membership
    /// update. The dynamic room-list stream can observe that update before
    /// the client room store publishes the corresponding room, so the actor
    /// performs the same bounded live-service refresh used by local room
    /// mutations.
    RefreshMembershipProjection {
        source: RoomListSource,
        room_generation: u64,
    },
    /// Authoritative room-list removal invalidated in-flight room operations.
    /// Stop only the observation owned by this runtime generation and
    /// acknowledge after its task has joined.
    StopSyncObservation {
        backend_generation: u64,
        ack: oneshot::Sender<()>,
    },
    /// A backend task ended. The source/generation fence prevents a delayed
    /// stop from failing a replacement backend that already started.
    BackendSyncStopped {
        source: RoomListSource,
        backend_generation: u64,
    },
    /// The active account is logging out/switching/resetting while the
    /// RoomActor stays alive for future sessions. The oneshot acknowledges
    /// that the actor completed the encryption-debug cancel/join/settle
    /// sequence before clearing the session (issue #538).
    SessionCleared { ack: oneshot::Sender<()> },
    /// Observer relay: parent-only space links discovered in a room-list
    /// snapshot. RoomActor owns dedupe, server writes, and retry policy.
    MissingSpaceChildLinks { links: Vec<MissingSpaceChildLink> },
    /// Account-data aliases are identity presentation inputs, never mention
    /// eligibility inputs. Reproject only already-demanded joined members.
    LocalUserAliasesUpdated { aliases: BTreeMap<String, String> },
    MentionMembersRefreshed {
        room_id: String,
        session_generation: u64,
        refresh_generation: u64,
        result: Result<MatrixJoinedMemberSnapshot, MatrixRoomOperationError>,
    },
    /// Base-client room updates can include membership state changes. Only
    /// demanded rooms are recomputed; `None` means the broadcast lagged and
    /// every demanded room must be self-healed.
    MentionMembershipChanged { room_ids: Option<BTreeSet<String>> },
    /// Completion of a local-only sync-driven Space-member projection.
    SpaceMembersProjectionRefreshed {
        request_id: RequestId,
        session_generation: u64,
        demand_generation: u64,
        refresh_generation: u64,
        space_id: String,
        generation: u64,
        result: Result<MatrixSpaceMembersProjection, MatrixRoomOperationError>,
    },
    /// A sync update changed the authoritative `m.room.pinned_events` state.
    /// The actor reloads only the affected rooms so external pin/unpin actions
    /// become visible without polling every room.
    PinnedEventsChanged { room_ids: BTreeSet<String> },
    #[cfg(any(test, feature = "test-hooks"))]
    TestVisibleRoomsObserved {
        core_generation: u64,
        reconciliation_is_complete: bool,
        room_ids: Vec<VisibleRoomObservation>,
        forwarded: oneshot::Sender<bool>,
    },
    #[cfg(any(test, feature = "test-hooks"))]
    TestMembershipObserved {
        core_generation: u64,
        transitions: Vec<RoomMembershipTransition>,
        forwarded: oneshot::Sender<bool>,
    },
    #[cfg(any(test, feature = "test-hooks"))]
    TestKnownRooms {
        room_ids: BTreeSet<String>,
        forwarded: oneshot::Sender<()>,
    },
    #[cfg(test)]
    InspectObservationGeneration {
        response: oneshot::Sender<Option<u64>>,
    },
    /// Ordered shutdown.
    Shutdown,
}

#[derive(Clone)]
pub(super) struct TimelineResidencyBinding {
    pub(super) session: Arc<MatrixClientSession>,
    pub(super) handle: TimelineSubscriptionResidencyHandle,
}

/// Handle to the RoomActor background task (owned by AccountActor).
pub struct RoomActorHandle {
    pub(crate) tx: mpsc::Sender<RoomMessage>,
    timeline_residency: watch::Sender<Option<TimelineResidencyBinding>>,
    #[cfg(any(test, feature = "test-hooks"))]
    session: watch::Sender<Option<Arc<MatrixClientSession>>>,
    #[cfg(any(test, feature = "test-hooks"))]
    room_operation_test_control: RoomOperationTestControlSlot,
    #[cfg(any(test, feature = "test-hooks"))]
    room_operation_test_reached_count: Arc<AtomicUsize>,
    task: Option<executor::JoinHandle<()>>,
}

impl RoomActorHandle {
    pub(crate) fn spawn_with_account_work(
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
        sliding_sync_diagnostics: crate::SlidingSyncDiagnostics,
        _account_work: AccountWorkScheduler,
    ) -> Self {
        RoomActor::spawn(action_tx, event_tx, sliding_sync_diagnostics)
    }

    pub(crate) fn sender(&self) -> mpsc::Sender<RoomMessage> {
        self.tx.clone()
    }

    pub(crate) fn bind_timeline_residency(
        &self,
        session: Arc<MatrixClientSession>,
        handle: TimelineSubscriptionResidencyHandle,
    ) {
        self.timeline_residency
            .send_replace(Some(TimelineResidencyBinding { session, handle }));
    }

    pub(crate) fn clear_timeline_residency(&self) {
        self.timeline_residency.send_replace(None);
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn operation_test_reached_count(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.room_operation_test_reached_count)
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn install_room_operation_test_control(
        &self,
        control: RoomOperationTestControl,
    ) -> bool {
        let mut slot = self
            .room_operation_test_control
            .lock()
            .expect("room operation test control lock");
        if slot.is_some() {
            return false;
        }
        *slot = Some(control);
        true
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) async fn install_known_rooms_for_test(&self, room_ids: BTreeSet<String>) -> bool {
        let (forwarded_tx, forwarded_rx) = oneshot::channel();
        if !self
            .send(RoomMessage::TestKnownRooms {
                room_ids,
                forwarded: forwarded_tx,
            })
            .await
        {
            return false;
        }
        forwarded_rx.await.is_ok()
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn timeline_residency_snapshot(
        &self,
    ) -> Option<(
        Arc<MatrixClientSession>,
        TimelineSubscriptionResidencyHandle,
    )> {
        self.timeline_residency
            .borrow()
            .as_ref()
            .map(|binding| (binding.session.clone(), binding.handle.clone()))
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn session_snapshot(&self) -> Option<Arc<MatrixClientSession>> {
        self.session.borrow().clone()
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) async fn wait_for_session(&self, expected: &Arc<MatrixClientSession>) -> bool {
        let mut session = self.session.subscribe();
        session
            .wait_for(|current| {
                current
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, expected))
            })
            .await
            .is_ok()
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) async fn room_subscription_residency_test_observe_visible(
        &self,
        core_generation: u64,
        reconciliation_is_complete: bool,
        room_ids: Vec<VisibleRoomObservation>,
    ) -> bool {
        let (forwarded_tx, forwarded_rx) = oneshot::channel();
        if !self
            .send(RoomMessage::TestVisibleRoomsObserved {
                core_generation,
                reconciliation_is_complete,
                room_ids,
                forwarded: forwarded_tx,
            })
            .await
        {
            return false;
        }
        forwarded_rx.await.unwrap_or(false)
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) async fn room_subscription_residency_test_observe_membership(
        &self,
        core_generation: u64,
        transitions: Vec<RoomMembershipTransition>,
    ) -> bool {
        let (forwarded_tx, forwarded_rx) = oneshot::channel();
        if !self
            .send(RoomMessage::TestMembershipObserved {
                core_generation,
                transitions,
                forwarded: forwarded_tx,
            })
            .await
        {
            return false;
        }
        forwarded_rx.await.unwrap_or(false)
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn clear_room_operation_test_control(&self) {
        self.room_operation_test_control
            .lock()
            .expect("room operation test control lock")
            .take();
    }

    pub async fn send(&self, msg: RoomMessage) -> bool {
        self.tx.send(msg).await.is_ok()
    }

    /// Wait for the actor task to complete (used in ordered shutdown).
    pub async fn shutdown(&mut self) -> bool {
        self.shutdown_with_timeouts(
            ROOM_ACTOR_SHUTDOWN_SEND_TIMEOUT,
            ROOM_ACTOR_SHUTDOWN_JOIN_TIMEOUT,
        )
        .await
    }

    async fn shutdown_with_timeouts(
        &mut self,
        send_timeout: Duration,
        join_timeout: Duration,
    ) -> bool {
        let sent = matches!(
            executor::timeout(send_timeout, self.tx.send(RoomMessage::Shutdown)).await,
            Ok(Ok(()))
        );
        let Some(mut task) = self.task.take() else {
            return sent;
        };
        if sent && executor::timeout(join_timeout, &mut task).await.is_ok() {
            return true;
        }
        task.abort();
        let _ = task.await;
        false
    }

    pub async fn join(mut self) {
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

pub enum RoomListReconcileAck {
    Projected {
        backend_generation: u64,
        room_generation: u64,
        response_sequence: u64,
    },
    Reconciled {
        backend_generation: u64,
        room_generation: u64,
        response_sequence: u64,
    },
    Superseded {
        backend_generation: u64,
        room_generation: u64,
        response_sequence: u64,
    },
}

pub struct RoomActor {
    pub(super) session: Option<Arc<MatrixClientSession>>,
    pub(super) timeline_residency: watch::Receiver<Option<TimelineResidencyBinding>>,
    session_slot: watch::Sender<Option<Arc<MatrixClientSession>>>,
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) room_operation_test_control: RoomOperationTestControlSlot,
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) room_operation_test_reached_count: Arc<AtomicUsize>,
    pub(super) observation: Option<RoomListObservation>,
    room_list_generation: u64,
    room_list_source: Option<RoomListSource>,
    room_list_backend_generation: Option<u64>,
    pub(super) known_room_ids: Arc<RwLock<BTreeSet<String>>>,
    pub(super) known_dm_rooms: Arc<RwLock<Vec<koushi_state::RoomSummary>>>,
    pub(super) attempted_space_child_repairs: Arc<RwLock<BTreeSet<SpaceChildLinkKey>>>,
    pub(super) mention_demands: HashMap<(String, MentionSurface), MentionDemand>,
    pub(super) mention_member_snapshots: HashMap<String, MatrixJoinedMemberSnapshot>,
    pub(super) mention_refresh_generations: HashMap<String, u64>,
    pub(super) mention_refresh_sequence: u64,
    pub(super) mention_session_generation: u64,
    pub(super) mention_local_aliases: BTreeMap<String, String>,
    pub(super) space_member_demand: Option<SpaceMemberDemand>,
    pub(super) space_member_demand_generation: u64,
    pub(super) space_member_refresh_sequence: u64,
    pub(super) space_member_session_generation: u64,
    pub(super) space_member_refresh_in_flight: Option<SpaceMemberRefreshFence>,
    pub(super) space_member_refresh_pending: bool,
    pub(super) action_tx: mpsc::Sender<Vec<AppAction>>,
    pub(super) event_tx: broadcast::Sender<CoreEvent>,
    pub(super) sliding_sync_diagnostics: crate::SlidingSyncDiagnostics,
    pub(super) self_tx: mpsc::Sender<RoomMessage>,
    command_rx: mpsc::Receiver<RoomMessage>,
}

impl RoomActor {
    pub fn spawn(
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
        sliding_sync_diagnostics: crate::SlidingSyncDiagnostics,
    ) -> RoomActorHandle {
        Self::spawn_with_account_work(
            action_tx,
            event_tx,
            sliding_sync_diagnostics,
            AccountWorkScheduler::default(),
        )
    }

    pub(crate) fn spawn_with_account_work(
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
        sliding_sync_diagnostics: crate::SlidingSyncDiagnostics,
        _account_work: AccountWorkScheduler,
    ) -> RoomActorHandle {
        let (tx, command_rx) = mpsc::channel(crate::ACTOR_MESSAGE_QUEUE_CAPACITY);
        let (timeline_residency, timeline_residency_rx) = watch::channel(None);
        let (session_slot, _session_rx) = watch::channel(None);
        #[cfg(any(test, feature = "test-hooks"))]
        let room_operation_test_control = Arc::new(Mutex::new(None));
        #[cfg(any(test, feature = "test-hooks"))]
        let room_operation_test_reached_count = Arc::new(AtomicUsize::new(0));
        let actor = RoomActor {
            session: None,
            timeline_residency: timeline_residency_rx,
            session_slot: session_slot.clone(),
            #[cfg(any(test, feature = "test-hooks"))]
            room_operation_test_control: room_operation_test_control.clone(),
            #[cfg(any(test, feature = "test-hooks"))]
            room_operation_test_reached_count: room_operation_test_reached_count.clone(),
            observation: None,
            room_list_generation: 0,
            room_list_source: None,
            room_list_backend_generation: None,
            known_room_ids: Arc::new(RwLock::new(BTreeSet::new())),
            known_dm_rooms: Arc::new(RwLock::new(Vec::new())),
            attempted_space_child_repairs: Arc::new(RwLock::new(BTreeSet::new())),
            mention_demands: HashMap::new(),
            mention_member_snapshots: HashMap::new(),
            mention_refresh_generations: HashMap::new(),
            mention_refresh_sequence: 0,
            mention_session_generation: 0,
            mention_local_aliases: BTreeMap::new(),
            space_member_demand: None,
            space_member_demand_generation: 0,
            space_member_refresh_sequence: 0,
            space_member_session_generation: 0,
            space_member_refresh_in_flight: None,
            space_member_refresh_pending: false,
            action_tx,
            event_tx,
            sliding_sync_diagnostics,
            self_tx: tx.clone(),
            command_rx,
        };
        let task = executor::spawn(actor.run());
        RoomActorHandle {
            tx,
            timeline_residency,
            #[cfg(any(test, feature = "test-hooks"))]
            session: session_slot,
            #[cfg(any(test, feature = "test-hooks"))]
            room_operation_test_control,
            #[cfg(any(test, feature = "test-hooks"))]
            room_operation_test_reached_count,
            task: Some(task),
        }
    }

    async fn run(mut self) {
        while let Some(msg) = self.command_rx.recv().await {
            match msg {
                RoomMessage::Shutdown => {
                    self.stop_observation().await;
                    break;
                }
                RoomMessage::Command(command) => {
                    self.handle_command(command).await;
                }
                #[cfg(any(test, feature = "test-hooks"))]
                RoomMessage::TestCommand { command, processed } => {
                    self.handle_command(command).await;
                    let _ = processed.send(());
                }
                RoomMessage::SessionEstablished { session } => {
                    // Room operations become available; observation starts
                    // later on SyncStarted (backend then known).
                    self.reset_space_member_session();
                    self.session = Some(session.clone());
                    self.session_slot.send_replace(Some(session));
                    self.clear_known_rooms();
                    self.clear_space_child_repair_attempts();
                    self.clear_mention_candidates();
                }
                RoomMessage::SyncStarted {
                    session,
                    room_list_service,
                    source,
                    backend_generation,
                } => {
                    // Guard against two observation loops running: a previous
                    // loop (from an earlier SyncStarted) is stopped before the
                    // replacement is spawned.
                    self.stop_observation().await;
                    self.reset_space_member_session();
                    self.session = Some(session.clone());
                    self.session_slot.send_replace(Some(session.clone()));
                    self.room_list_generation = self.room_list_generation.wrapping_add(1).max(1);
                    self.room_list_source = Some(source);
                    self.room_list_backend_generation = Some(backend_generation);
                    // Keep the actor-known room book across backend handoff so
                    // cached rows remain actionable while the new generation
                    // is still loading. SessionEstablished/SessionCleared own
                    // the account-bound reset of this book.
                    self.clear_space_child_repair_attempts();
                    self.reduce_reliable(vec![AppAction::RoomListBootstrapStarted {
                        generation: self.room_list_generation,
                        source,
                    }])
                    .await;
                    // Its first diff batch (Reset with the current entries)
                    // provides the initial snapshot, so no separate initial
                    // refresh is needed. Cached rows remain available until
                    // that generation settles.
                    let timeline_residency = self
                        .timeline_residency
                        .borrow()
                        .as_ref()
                        .filter(|binding| Arc::ptr_eq(&binding.session, &session))
                        .map(|binding| binding.handle.clone());
                    self.start_live_observation(
                        session,
                        room_list_service,
                        self.room_list_generation,
                        source,
                        timeline_residency,
                    );
                }
                RoomMessage::ReconcileCommittedRange {
                    source,
                    backend_generation,
                    response_sequence,
                    ack,
                } => {
                    if self.room_list_source == Some(source)
                        && self.room_list_backend_generation == Some(backend_generation)
                        && let Some(observation) = &self.observation
                        && observation.source == source
                        && observation.generation == self.room_list_generation
                    {
                        let _ = observation
                            .command_tx
                            .send(RoomListObservationCommand::Reconcile {
                                backend_generation,
                                response_sequence,
                                ack,
                            })
                            .await;
                    }
                }
                RoomMessage::RefreshCommittedProjection {
                    source,
                    backend_generation,
                } => {
                    if self.room_list_source == Some(source)
                        && self.room_list_backend_generation == Some(backend_generation)
                        && let Some(observation) = &self.observation
                        && observation.source == source
                        && observation.generation == self.room_list_generation
                    {
                        // This is a single post-commit wake. Operation
                        // refreshes use bounded retries because the local
                        // mutation may settle asynchronously; a sync response
                        // must not spawn retry work for every response.
                        let _ = observation
                            .command_tx
                            .try_send(RoomListObservationCommand::Refresh);
                    }
                }
                RoomMessage::RefreshMembershipProjection {
                    source,
                    room_generation,
                } => {
                    if self.room_list_source == Some(source)
                        && self.room_list_generation == room_generation
                    {
                        // Membership updates are sparse and can precede the
                        // SDK room-store publication. Use the existing
                        // bounded refresh path so the single live service is
                        // re-read after that publication window closes.
                        self.refresh_room_list();
                    }
                }
                RoomMessage::StopSyncObservation {
                    backend_generation,
                    ack,
                } => {
                    if room_stop_matches_generation(
                        self.room_list_backend_generation,
                        backend_generation,
                    ) {
                        self.stop_observation().await;
                        self.reset_space_member_session();
                        self.clear_known_rooms();
                        self.clear_space_child_repair_attempts();
                        self.room_list_source = None;
                        self.room_list_backend_generation = None;
                    }
                    let _ = ack.send(());
                }
                RoomMessage::BackendSyncStopped {
                    source,
                    backend_generation,
                } => {
                    if self.room_list_source == Some(source)
                        && self.room_list_backend_generation == Some(backend_generation)
                    {
                        self.stop_observation().await;
                        self.reduce_reliable(vec![AppAction::RoomListBootstrapFailed {
                            generation: self.room_list_generation,
                            source,
                            kind: RoomListFailureKind::Stopped,
                        }])
                        .await;
                        self.room_list_source = None;
                        self.room_list_backend_generation = None;
                    }
                }
                RoomMessage::SessionCleared { ack } => {
                    self.stop_observation().await;
                    self.reset_space_member_session();
                    self.session = None;
                    self.session_slot.send_replace(None);
                    self.clear_known_rooms();
                    self.clear_space_child_repair_attempts();
                    self.clear_mention_candidates();
                    let _ = ack.send(());
                }

                RoomMessage::MissingSpaceChildLinks { links } => {
                    self.handle_missing_space_child_links(links).await;
                }
                RoomMessage::LocalUserAliasesUpdated { aliases } => {
                    self.handle_mention_local_aliases_updated(aliases).await;
                }
                RoomMessage::MentionMembersRefreshed {
                    room_id,
                    session_generation,
                    refresh_generation,
                    result,
                } => {
                    self.handle_mention_members_refreshed(
                        room_id,
                        session_generation,
                        refresh_generation,
                        result,
                    )
                    .await;
                }
                RoomMessage::MentionMembershipChanged { room_ids } => {
                    self.handle_mention_membership_changed(room_ids).await;
                }
                RoomMessage::SpaceMembersProjectionRefreshed {
                    request_id,
                    session_generation,
                    demand_generation,
                    refresh_generation,
                    space_id,
                    generation,
                    result,
                } => {
                    self.handle_space_members_projection_refreshed(
                        request_id,
                        session_generation,
                        demand_generation,
                        refresh_generation,
                        space_id,
                        generation,
                        result,
                    )
                    .await;
                }
                RoomMessage::PinnedEventsChanged { room_ids } => {
                    self.handle_pinned_events_changed(room_ids).await;
                }
                #[cfg(any(test, feature = "test-hooks"))]
                RoomMessage::TestVisibleRoomsObserved {
                    core_generation,
                    reconciliation_is_complete,
                    room_ids,
                    forwarded,
                } => {
                    self.handle_test_visible_rooms_observed(
                        core_generation,
                        reconciliation_is_complete,
                        room_ids,
                        forwarded,
                    )
                    .await;
                }
                #[cfg(any(test, feature = "test-hooks"))]
                RoomMessage::TestMembershipObserved {
                    core_generation,
                    transitions,
                    forwarded,
                } => {
                    self.handle_test_membership_observed(core_generation, transitions, forwarded)
                        .await;
                }
                #[cfg(any(test, feature = "test-hooks"))]
                RoomMessage::TestKnownRooms {
                    room_ids,
                    forwarded,
                } => {
                    *self.known_room_ids.write().expect("known room ids lock") = room_ids;
                    let _ = forwarded.send(());
                }
                #[cfg(test)]
                RoomMessage::InspectObservationGeneration { response } => {
                    let _ = response.send(self.room_list_backend_generation);
                }
            }
        }
    }

    async fn handle_command(&mut self, command: RoomCommand) {
        match command {
            RoomCommand::CreateRoom {
                request_id,
                options,
            } => {
                self.handle_create_room(request_id, options).await;
            }
            RoomCommand::CreatePublicDirectoryRoom {
                request_id,
                name,
                alias_localpart,
            } => {
                self.handle_create_public_directory_room(request_id, name, alias_localpart)
                    .await;
            }
            RoomCommand::CreateSpace { request_id, name } => {
                self.handle_create_space(request_id, name).await;
            }
            RoomCommand::SetSpaceChild {
                request_id,
                space_id,
                child_room_id,
                via_server,
            } => {
                self.handle_set_space_child(request_id, space_id, child_room_id, via_server)
                    .await;
            }
            RoomCommand::InviteUser {
                request_id,
                room_id,
                user_id,
            } => {
                self.handle_invite_user(request_id, room_id, user_id).await;
            }
            RoomCommand::LoadSpaceMembers {
                request_id,
                space_id,
                generation,
            } => {
                self.handle_load_space_members(request_id, space_id, generation)
                    .await;
            }
            RoomCommand::InviteUserToSpace {
                request_id,
                space_id,
                user_id,
                generation,
            } => {
                self.handle_invite_user_to_space(request_id, space_id, user_id, generation)
                    .await;
            }
            RoomCommand::CancelSpaceInvite {
                request_id,
                space_id,
                user_id,
                generation,
            } => {
                self.handle_cancel_space_invite(request_id, space_id, user_id, generation)
                    .await;
            }
            RoomCommand::InviteTargets {
                request_id,
                room_id,
                user_ids,
                scope,
            } => {
                self.handle_invite_targets(request_id, room_id, user_ids, scope)
                    .await;
            }
            RoomCommand::AcceptInvite {
                request_id,
                room_id,
            } => {
                self.handle_accept_invite(request_id, room_id).await;
            }
            RoomCommand::DeclineInvite {
                request_id,
                room_id,
            } => {
                self.handle_decline_invite(request_id, room_id).await;
            }
            RoomCommand::StartDirectMessage {
                request_id,
                user_id,
            } => {
                self.handle_start_direct_message(request_id, user_id).await;
            }
            RoomCommand::JoinRoom {
                request_id,
                room_id,
            } => {
                self.handle_join_room(request_id, room_id).await;
            }
            RoomCommand::LeaveRoom {
                request_id,
                room_id,
            } => {
                self.handle_leave_room(request_id, room_id).await;
            }
            RoomCommand::ForgetRoom {
                request_id,
                room_id,
            } => {
                self.handle_forget_room(request_id, room_id).await;
            }
            RoomCommand::SetTag {
                request_id,
                room_id,
                tag,
                order,
            } => {
                self.handle_set_tag(request_id, room_id, tag, order).await;
            }
            RoomCommand::RemoveTag {
                request_id,
                room_id,
                tag,
            } => {
                self.handle_remove_tag(request_id, room_id, tag).await;
            }
            RoomCommand::PinEvent {
                request_id,
                room_id,
                event_id,
            } => {
                self.handle_pin_event(request_id, room_id, event_id).await;
            }
            RoomCommand::UnpinEvent {
                request_id,
                room_id,
                event_id,
            } => {
                self.handle_unpin_event(request_id, room_id, event_id).await;
            }
            RoomCommand::RefreshPinnedEvents {
                request_id,
                room_id,
            } => {
                self.handle_refresh_pinned_events(request_id, room_id).await;
            }
            RoomCommand::QueryDirectory { request_id, query } => {
                self.handle_query_directory(request_id, query).await;
            }
            RoomCommand::PreviewJoinTarget {
                request_id,
                room_id_or_alias,
                via_servers,
            } => {
                self.handle_preview_join_target(request_id, room_id_or_alias, via_servers)
                    .await;
            }
            RoomCommand::DismissDirectoryPreview { request_id: _ } => {
                self.reduce_reliable(vec![AppAction::DirectoryPreviewDismissed])
                    .await;
            }
            RoomCommand::JoinDirectoryRoom {
                request_id,
                room_id_or_alias,
                via_servers,
            } => {
                self.handle_join_directory_room(request_id, room_id_or_alias, via_servers)
                    .await;
            }
            RoomCommand::LoadRoomSettings {
                request_id,
                room_id,
            } => {
                self.handle_load_room_settings(request_id, room_id).await;
            }
            RoomCommand::QueryMentionCandidates {
                request_id,
                account_key,
                room_id,
                surface,
                query,
            } => {
                self.handle_query_mention_candidates(
                    request_id,
                    account_key,
                    room_id,
                    surface,
                    query,
                )
                .await;
            }
            RoomCommand::UpdateRoomSetting {
                request_id,
                room_id,
                change,
            } => {
                self.handle_update_room_setting(request_id, room_id, change)
                    .await;
            }
            RoomCommand::ModerateRoomMember {
                request_id,
                room_id,
                target_user_id,
                action,
                reason,
            } => {
                self.handle_moderate_room_member(
                    request_id,
                    room_id,
                    target_user_id,
                    action,
                    reason,
                )
                .await;
            }
            RoomCommand::UpdateRoomMemberRole {
                request_id,
                room_id,
                target_user_id,
                power_level,
            } => {
                self.handle_update_room_member_role(
                    request_id,
                    room_id,
                    target_user_id,
                    power_level,
                )
                .await;
            }
            RoomCommand::UpdateSpaceMemberRole {
                request_id,
                space_id,
                user_id,
                generation,
                expected_power_levels_revision,
                expected_power_level,
                power_level,
                confirmed,
            } => {
                self.handle_update_space_member_role(
                    request_id,
                    space_id,
                    user_id,
                    generation,
                    expected_power_levels_revision,
                    expected_power_level,
                    power_level,
                    confirmed,
                )
                .await;
            }
            RoomCommand::SelectSpace {
                request_id: _,
                space_id,
            } => {
                // Navigation commits before membership hydration; the room-list
                // observer owns the SDK await and subsequent reprojection.
                let hydration_space_id = space_id.clone();
                self.reduce_reliable(vec![AppAction::SelectSpace { space_id }])
                    .await;
                if let Some(space_id) = hydration_space_id
                    && let Some(observation) = &self.observation
                {
                    let _ = observation
                        .command_tx
                        .send(RoomListObservationCommand::HydrateSpaceMembers { space_id })
                        .await;
                }
            }
            RoomCommand::ReorderSpaces {
                request_id: _,
                space_ids,
            } => {
                // Pure navigation preference: project to reducer; no domain event.
                // One-shot navigation MUST be delivered reliably (see reduce_reliable).
                self.reduce_reliable(vec![AppAction::ReorderSpaces { space_ids }])
                    .await;
            }
            RoomCommand::SelectRoom {
                request_id: _,
                room_id,
            } => {
                // Pure navigation: project to reducer; no domain event.
                // Core updates navigation state here and does not consume
                // reducer effects in this actor. One-shot navigation MUST be
                // delivered reliably: a dropped SelectRoom is the large-account
                // "room selection did not complete" bug (see reduce_reliable).
                self.reduce_reliable(vec![AppAction::SelectRoom { room_id }])
                    .await;
            }
            RoomCommand::MarkRoomAsRead {
                request_id,
                room_id,
                event_id,
            } => {
                self.handle_mark_room_as_read(request_id, room_id, event_id)
                    .await;
            }
            RoomCommand::MarkRoomAsUnread {
                request_id,
                room_id,
                unread,
            } => {
                self.handle_mark_room_as_unread(request_id, room_id, unread)
                    .await;
            }
            RoomCommand::ForceRotateOutboundSession {
                request_id,
                room_id,
            } => {
                self.handle_force_rotate_outbound_session(request_id, room_id)
                    .await;
            }
            RoomCommand::SetRoomNotificationMode {
                request_id,
                room_id,
                mode,
            } => {
                self.handle_set_room_notification_mode(request_id, room_id, mode)
                    .await;
            }
            RoomCommand::ReportContent {
                request_id,
                room_id,
                event_id,
                reason,
            } => {
                self.handle_report_content(request_id, room_id, event_id, reason)
                    .await;
            }
            RoomCommand::ReportRoom {
                request_id,
                room_id,
                reason,
            } => {
                self.handle_report_room(request_id, room_id, reason).await;
            }
        }
    }

    fn clear_known_rooms(&self) {
        if let Ok(mut known_room_ids) = self.known_room_ids.write() {
            known_room_ids.clear();
        }
        if let Ok(mut known_dm_rooms) = self.known_dm_rooms.write() {
            known_dm_rooms.clear();
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

    /// Reliable projection for one-shot, non-re-projected actions (navigation,
    /// command results) that MUST NOT be dropped under large-account sync load.
    /// Backpressures instead of dropping; the AppActor drains the action inbox
    /// continuously, so this does not deadlock.
    pub(super) async fn reduce_reliable(&self, actions: Vec<AppAction>) {
        let _ = self.action_tx.send(actions).await;
    }
}

#[cfg(test)]
pub(super) fn make_request_id(seq: u64) -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(1),
        sequence: seq,
    }
}

#[cfg(test)]
mod tests;
