use std::collections::BTreeSet;
#[cfg(any(test, feature = "test-hooks"))]
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel};
use koushi_sdk::MatrixCommittedRoomTimelineCheckpoint as MatrixRoomSubscriptionCheckpoint;
#[cfg(any(test, feature = "test-hooks"))]
use koushi_state::ComposerFormattingOptions;

use matrix_sdk::ruma::OwnedRoomId;
#[cfg(any(test, feature = "test-hooks"))]
use tokio::sync::broadcast;
use tokio::sync::{mpsc, oneshot, watch};

#[cfg(any(test, feature = "test-hooks"))]
use crate::account_work::AccountWorkScheduler;
use crate::executor;
#[cfg(any(test, feature = "test-hooks"))]
use crate::link_preview::LinkPreviewContext;
#[cfg(any(test, feature = "test-hooks"))]
use crate::live_tail_freshness::LiveTailRefreshCoordinator;
#[cfg(any(test, feature = "test-hooks"))]
use crate::threads_list::ThreadRootProjectionService;
#[cfg(any(test, feature = "test-hooks"))]
use koushi_protocol::command::TimelineCommand;
#[cfg(any(test, feature = "test-hooks"))]
use koushi_protocol::ids::AccountKey;
use koushi_protocol::ids::{TimelineKey, TimelineKind};

// BEGIN GENERATED SIBLING IMPORTS
#[cfg(any(test, feature = "test-hooks"))]
use super::actor::TimelineActorHandle;
use super::actor::TimelineActorMessage;
use super::diagnostics::{
    record_residency_intent, record_subscription_reconcile, record_subscription_room_coverage,
    subscription_count_bucket,
};
use super::gap_repair::GlobalResponseCommit;
use super::manager::{TimelineManagerActor, TimelineMessage, internal_timeline_request_id};
#[cfg(any(test, feature = "test-hooks"))]
use super::navigation::TimelineActorGenerationGate;
#[cfg(any(test, feature = "test-hooks"))]
use super::outbound_send::{
    SendEnqueueWorkerSupervisor, SharedSendCompletionCoordinator, SubmissionAdmissionLedger,
    TimelineSendTerminalIngress,
};
use super::read_state::ReadRetrySource;
#[cfg(any(test, feature = "test-hooks"))]
use super::read_state::ReadWorkerSupervisor;
#[cfg(any(test, feature = "test-hooks"))]
use super::thread_projection::ThreadRootProjectionFetchRegistry;
// END GENERATED SIBLING IMPORTS

/// The only successful local room-removal causes that may mutate session
/// residency. Session teardown does not send a live removal message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RoomRemovalCause {
    DirectLeave,
    InviteDecline,
}

impl RoomRemovalCause {
    fn token(self) -> &'static str {
        match self {
            Self::DirectLeave => "room_left",
            Self::InviteDecline => "invite_declined",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RoomMembershipTransitionKind {
    Left,
    Joined,
    Invited,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RoomMembershipTransition {
    pub(crate) room_id: OwnedRoomId,
    pub(crate) kind: RoomMembershipTransitionKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VisibleRoomObservation {
    pub(crate) room_id: String,
    pub(crate) non_left: bool,
}

struct MembershipOperationGateState {
    accepting: bool,
    active_count: usize,
}

#[derive(Clone)]
pub(super) struct MembershipOperationGate {
    state: Arc<Mutex<MembershipOperationGateState>>,
    active_count: watch::Sender<usize>,
}

pub(crate) struct TimelineSubscriptionResidencyPermit {
    gate: MembershipOperationGate,
}

impl MembershipOperationGate {
    pub(super) fn new() -> Self {
        let (active_count, _) = watch::channel(0_usize);
        Self {
            state: Arc::new(Mutex::new(MembershipOperationGateState {
                accepting: true,
                active_count: 0,
            })),
            active_count,
        }
    }

    fn begin_operation(&self) -> Option<TimelineSubscriptionResidencyPermit> {
        let mut state = self.state.lock().expect("membership operation gate lock");
        if !state.accepting {
            return None;
        }
        state.active_count += 1;
        self.active_count.send_replace(state.active_count);
        Some(TimelineSubscriptionResidencyPermit { gate: self.clone() })
    }

    async fn close_and_drain(&self) {
        {
            let mut state = self.state.lock().expect("membership operation gate lock");
            state.accepting = false;
        }
        let mut active_count = self.active_count.subscribe();
        let _ = active_count.wait_for(|count| *count == 0).await;
    }

    #[cfg(any(test, feature = "test-hooks"))]
    fn snapshot(&self) -> (bool, usize) {
        let state = self.state.lock().expect("membership operation gate lock");
        (state.accepting, state.active_count)
    }
}

impl Drop for TimelineSubscriptionResidencyPermit {
    fn drop(&mut self) {
        let mut state = self
            .gate
            .state
            .lock()
            .expect("membership operation gate lock");
        debug_assert!(state.active_count > 0);
        state.active_count = state.active_count.saturating_sub(1);
        self.gate.active_count.send_replace(state.active_count);
    }
}

/// Cloneable, narrow residency ingress shared by the RoomActor and the
/// session-owned timeline manager. It is intentionally not a generic timeline
/// command authority.
#[derive(Clone)]
pub(crate) struct TimelineSubscriptionResidencyHandle {
    pub(super) tx: mpsc::Sender<TimelineMessage>,
    pub(super) gate: MembershipOperationGate,
}

impl TimelineSubscriptionResidencyHandle {
    pub(crate) fn begin_operation(&self) -> Option<TimelineSubscriptionResidencyPermit> {
        if self.tx.is_closed() {
            return None;
        }
        self.gate.begin_operation()
    }

    pub(crate) async fn close_and_drain(&self) {
        self.gate.close_and_drain().await;
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn gate_snapshot(&self) -> (bool, usize) {
        self.gate.snapshot()
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) async fn gate_probe_for_testing(&self) -> (bool, usize, bool, bool) {
        let permit = self
            .begin_operation()
            .expect("test gate must initially accept");
        let gate = self.clone();
        let drain = executor::spawn(async move {
            gate.close_and_drain().await;
            gate.gate_snapshot()
        });
        // The final drop intentionally happens before the waiter is polled.
        drop(permit);
        let (accepting, active_count) = drain.await.expect("gate drain task");
        let rejected = self.begin_operation().is_none();
        (accepting, active_count, rejected, active_count == 0)
    }

    pub(crate) async fn visible_rooms_observed(
        &self,
        core_generation: u64,
        room_ids: Vec<VisibleRoomObservation>,
    ) -> bool {
        self.tx
            .send(TimelineMessage::VisibleRoomsObserved {
                core_generation,
                room_ids,
            })
            .await
            .is_ok()
    }

    pub(crate) async fn membership_observed(
        &self,
        core_generation: u64,
        transitions: Vec<RoomMembershipTransition>,
    ) -> bool {
        self.tx
            .send(TimelineMessage::RoomMembershipObserved {
                core_generation,
                transitions,
            })
            .await
            .is_ok()
    }

    pub(crate) async fn room_left(
        &self,
        _permit: &TimelineSubscriptionResidencyPermit,
        room_id: OwnedRoomId,
        cause: RoomRemovalCause,
    ) -> bool {
        let (acknowledged, acknowledgement) = oneshot::channel();
        if self
            .tx
            .send(TimelineMessage::RoomLeft {
                room_id,
                cause,
                acknowledged,
            })
            .await
            .is_err()
        {
            return false;
        }
        acknowledgement.await.is_ok()
    }

    pub(crate) async fn room_rejoined(
        &self,
        _permit: &TimelineSubscriptionResidencyPermit,
        room_id: OwnedRoomId,
    ) -> bool {
        let (acknowledged, acknowledgement) = oneshot::channel();
        if self
            .tx
            .send(TimelineMessage::RoomRejoined {
                room_id,
                acknowledged,
            })
            .await
            .is_err()
        {
            return false;
        }
        acknowledgement.await.is_ok()
    }
}

/// Session-local leave state. Visibility and restore evidence never clear it;
/// only an ordered SDK left→joined/invited observation or an admitted local
/// rejoin may do so.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RoomLeaveState {
    PendingLeftObservation,
    LeftObserved,
}

/// Why one subscription reconciliation was requested (issue #518).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SubscriptionReconcileTrigger {
    RoomSelected,
    ThreadOpened,
    FocusedOpened,
    TimelineRebuild,
    SyncStarted,
    VisibleRange,
    Restore,
    RoomLeft,
    RoomRejoined,
    Membership,
}

impl SubscriptionReconcileTrigger {
    fn token(self) -> &'static str {
        match self {
            Self::RoomSelected
            | Self::ThreadOpened
            | Self::FocusedOpened
            | Self::TimelineRebuild => "opened",
            Self::SyncStarted => "session_restart",
            Self::VisibleRange => "visible_range",
            Self::Restore => "restore",
            Self::RoomLeft => "room_left",
            Self::RoomRejoined => "room_rejoined",
            Self::Membership => "membership",
        }
    }
}

impl TimelineManagerActor {
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) fn room_subscription_residency_test_actor_handle() -> TimelineActorHandle {
        let (tx, mut rx) = mpsc::channel(1);
        let task = executor::spawn(async move { while rx.recv().await.is_some() {} });
        TimelineActorHandle {
            tx,
            control_tx: None,
            thread_summary_projection:
                crate::timeline::actor::ThreadSummaryProjectionIngress::channel().0,
            position_rx: None,
            task: Some(task),
            auxiliary_tasks: Vec::new(),
            subscription_generation: None,
            enqueue_context: None,
        }
    }
    #[cfg(any(test, feature = "test-hooks"))]
    #[doc(hidden)]
    pub(crate) fn room_subscription_residency_test_manager(
        room_list_service: Arc<matrix_sdk_ui::room_list_service::RoomListService>,
    ) -> Self {
        let (action_tx, _action_rx) = mpsc::channel(8);
        let (event_tx, _event_rx) = broadcast::channel(8);
        let (msg_tx, msg_rx) = mpsc::channel(8);
        let (terminal_ingress, terminal_rx) = TimelineSendTerminalIngress::channel();
        Self {
            session: None,
            room_list_service: Some(room_list_service),
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
            action_tx,
            event_tx,
            msg_tx,
            msg_rx,
            control_rx: None,
            navigation_projection_rx: None,
            last_navigation_projection_generation: 0,
            applied_room_projection: None,
            terminal_ingress,
            terminal_rx,
            search_index_tx: None,
            ignored_user_ids: BTreeSet::new(),
            data_dir: None,
            link_preview_policy: LinkPreviewContext::default(),
            composer_formatting_options: ComposerFormattingOptions::default(),
            thread_root_order: koushi_state::TimelineThreadRootOrder::LatestReply,
            hide_redacted: false,
            account_work: AccountWorkScheduler::default(),
            thread_root_projection_service: Arc::new(Mutex::new(
                ThreadRootProjectionService::default(),
            )),
            thread_root_projection_fetches: ThreadRootProjectionFetchRegistry::default(),
            timeline_actor_generations: Arc::new(TimelineActorGenerationGate::default()),
            live_tail_refreshes: LiveTailRefreshCoordinator::new(),
            focused_builds: Default::default(),
            #[cfg(any(test, feature = "test-hooks"))]
            test_session_available: true,
        }
    }
    #[cfg(any(test, feature = "test-hooks"))]
    #[doc(hidden)]
    pub(crate) fn room_subscription_residency_test_handle(
        &self,
    ) -> TimelineSubscriptionResidencyHandle {
        TimelineSubscriptionResidencyHandle {
            tx: self.msg_tx.clone(),
            gate: MembershipOperationGate::new(),
        }
    }
    #[cfg(any(test, feature = "test-hooks"))]
    #[doc(hidden)]
    pub(crate) async fn room_subscription_residency_test_gate_probe() -> (bool, usize, bool, bool) {
        let (tx, _rx) = mpsc::channel(1);
        let handle = TimelineSubscriptionResidencyHandle {
            tx,
            gate: MembershipOperationGate::new(),
        };
        handle.gate_probe_for_testing().await
    }
    #[cfg(any(test, feature = "test-hooks"))]
    #[doc(hidden)]
    pub(crate) async fn room_subscription_residency_test_admit_key(&mut self, key: TimelineKey) {
        self.handle_subscribe(
            internal_timeline_request_id(),
            key,
            false,
            false,
            koushi_protocol::command::InitialBackfillPolicy::Disabled,
        )
        .await;
    }
    #[cfg(any(test, feature = "test-hooks"))]
    #[doc(hidden)]
    pub(crate) async fn room_subscription_residency_test_admit_build_failure(
        &mut self,
        room_id: OwnedRoomId,
    ) {
        let key = TimelineKey {
            account_key: AccountKey("@resident:example.invalid".to_owned()),
            kind: TimelineKind::Thread {
                room_id: room_id.to_string(),
                root_event_id: "not-an-event-id".to_owned(),
            },
        };
        self.handle_subscribe(
            internal_timeline_request_id(),
            key,
            false,
            false,
            koushi_protocol::command::InitialBackfillPolicy::Disabled,
        )
        .await;
    }
    #[cfg(any(test, feature = "test-hooks"))]
    #[doc(hidden)]
    pub(crate) async fn room_subscription_residency_test_unsubscribe(&mut self, key: TimelineKey) {
        self.handle_command(TimelineCommand::Unsubscribe {
            request_id: internal_timeline_request_id(),
            key,
        })
        .await;
    }
    #[cfg(any(test, feature = "test-hooks"))]
    #[doc(hidden)]
    pub(crate) fn room_subscription_residency_test_snapshot(
        &self,
    ) -> (Vec<String>, Vec<String>, Vec<String>, usize, usize, u64) {
        let desired_rooms = self
            .session_subscribed_rooms
            .iter()
            .map(ToString::to_string)
            .collect();
        let active_rooms = self
            .room_list_service
            .as_ref()
            .map(|service| {
                service
                    .active_room_subscriptions()
                    .into_iter()
                    .map(|room_id| room_id.to_string())
                    .collect()
            })
            .unwrap_or_default();
        let lease_count = self.subscribed_room_leases.values().sum();
        let sdk_generation = self
            .room_list_service
            .as_ref()
            .map(|service| service.subscription_generation().get())
            .unwrap_or_default();
        let tombstoned_rooms = self
            .room_leave_states
            .keys()
            .map(ToString::to_string)
            .collect();
        (
            desired_rooms,
            active_rooms,
            tombstoned_rooms,
            self.timelines.len(),
            lease_count,
            sdk_generation,
        )
    }
    #[cfg(any(test, feature = "test-hooks"))]
    #[doc(hidden)]
    pub(crate) async fn room_subscription_residency_test_seed_sdk_subscriptions(
        &mut self,
        room_ids: &[&matrix_sdk::ruma::RoomId],
    ) {
        if let Some(service) = self.room_list_service.clone() {
            let _ = service
                .reconcile_room_subscriptions_with_generation(room_ids)
                .await;
        }
    }
    #[cfg(any(test, feature = "test-hooks"))]
    #[doc(hidden)]
    pub(crate) async fn room_subscription_residency_test_expire_sdk_subscriptions(&mut self) {
        if let Some(service) = self.room_list_service.clone() {
            let empty: Vec<&matrix_sdk::ruma::RoomId> = Vec::new();
            let _ = service
                .reconcile_room_subscriptions_with_generation(&empty)
                .await;
        }
    }
    #[cfg(any(test, feature = "test-hooks"))]
    #[doc(hidden)]
    pub(crate) async fn room_subscription_residency_test_pump_next_ingress(&mut self) {
        loop {
            let message = self.msg_rx.recv().await.expect("residency manager mailbox");
            match message {
                TimelineMessage::VisibleRoomsObserved {
                    core_generation,
                    room_ids,
                } => {
                    self.handle_visible_rooms_observed(core_generation, room_ids)
                        .await;
                    return;
                }
                TimelineMessage::RoomMembershipObserved {
                    core_generation,
                    transitions,
                } => {
                    self.handle_room_membership_observed(core_generation, transitions)
                        .await;
                    return;
                }
                TimelineMessage::RoomLeft {
                    room_id,
                    cause,
                    acknowledged,
                } => {
                    self.handle_room_left(room_id, cause).await;
                    let _ = acknowledged.send(());
                    return;
                }
                TimelineMessage::RoomRejoined {
                    room_id,
                    acknowledged,
                } => {
                    self.handle_room_rejoined(room_id).await;
                    let _ = acknowledged.send(());
                    return;
                }
                TimelineMessage::RoomSubscriptionCheckpoint {
                    service_epoch,
                    checkpoint,
                } => {
                    self.handle_room_subscription_checkpoint(service_epoch, checkpoint)
                        .await;
                }
                _ => panic!("unexpected residency manager mailbox message"),
            }
        }
    }
    #[cfg(any(test, feature = "test-hooks"))]
    #[doc(hidden)]
    pub(crate) async fn room_subscription_residency_test_sync_started(
        &mut self,
        core_generation: u64,
    ) {
        self.current_core_generation = Some(core_generation);
        if let Some(service) = self.room_list_service.clone() {
            self.handle_sync_started(service, core_generation).await;
        }
    }
    #[cfg(any(test, feature = "test-hooks"))]
    #[doc(hidden)]
    pub(crate) async fn room_subscription_residency_test_offer_restore(
        &mut self,
        core_generation: u64,
        room_ids: &[&str],
        proven: bool,
    ) {
        let restored = room_ids
            .iter()
            .map(|room_id| room_id.parse().expect("synthetic room id"))
            .collect();
        self.restored_room_subscription_probe = Some((proven, restored));
        self.current_core_generation = Some(core_generation);
        if let Some(service) = self.room_list_service.clone() {
            self.handle_sync_started(service, core_generation).await;
        }
    }
    pub(super) async fn handle_sync_started(
        &mut self,
        room_list_service: Arc<matrix_sdk_ui::room_list_service::RoomListService>,
        core_generation: u64,
    ) {
        self.wake_all_desired_reads(ReadRetrySource::Reconnect)
            .await;
        self.room_subscription_service_epoch =
            self.room_subscription_service_epoch.wrapping_add(1).max(1);
        let service_epoch = self.room_subscription_service_epoch;
        if let Some(task) = self.room_subscription_checkpoint_task.take() {
            task.abort();
        }
        self.room_list_service = Some(room_list_service.clone());
        self.current_core_generation = Some(core_generation);
        self.global_response_commit = Some(GlobalResponseCommit::new(core_generation, 0));
        let replacement_starts = self
            .invalidate_live_tail_epoch_for_existing_rooms(service_epoch)
            .await;
        let mut checkpoints = room_list_service.room_subscription_checkpoints();
        let manager_tx = self.msg_tx.clone();
        self.room_subscription_checkpoint_task = Some(executor::spawn(async move {
            loop {
                let retained = checkpoints.get();
                for checkpoint in retained.values() {
                    if manager_tx
                        .send(TimelineMessage::RoomSubscriptionCheckpoint {
                            service_epoch,
                            checkpoint: MatrixRoomSubscriptionCheckpoint::from_room_subscription(
                                checkpoint,
                            ),
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                if checkpoints.next().await.is_none() {
                    return;
                }
            }
        }));

        self.subscribe_existing_timeline_rooms(&room_list_service)
            .await;
        self.rebuild_existing_room_timelines_after_sync_started()
            .await;
        self.apply_live_tail_scheduler_actions(replacement_starts)
            .await;
    }
    pub(super) async fn handle_visible_rooms_observed(
        &mut self,
        core_generation: u64,
        room_ids: Vec<VisibleRoomObservation>,
    ) {
        if self.current_core_generation != Some(core_generation) {
            record_residency_intent("visible_range", "stale", 0, 0);
            return;
        }
        let mut accepted = BTreeSet::new();
        let mut rejected = 0_usize;
        for observation in room_ids {
            let Ok(room_id) = observation.room_id.parse::<OwnedRoomId>() else {
                rejected += 1;
                continue;
            };
            if !observation.non_left || self.room_leave_states.contains_key(&room_id) {
                rejected += 1;
                continue;
            }
            accepted.insert(room_id);
        }
        let accepted_count = accepted.len();
        self.session_subscribed_rooms.extend(accepted);
        record_residency_intent(
            "visible_range",
            if rejected == 0 {
                "accepted"
            } else {
                "rejected"
            },
            accepted_count,
            rejected,
        );
        // A valid current-generation observation is also the recovery wake
        // after UnknownPos/session expiry, even when it adds no new intent.
        self.reconcile_subscriptions(SubscriptionReconcileTrigger::VisibleRange)
            .await;
    }
    pub(super) async fn handle_room_membership_observed(
        &mut self,
        core_generation: u64,
        transitions: Vec<RoomMembershipTransition>,
    ) {
        if self.current_core_generation != Some(core_generation) {
            record_residency_intent("membership", "stale", 0, transitions.len());
            return;
        }
        let mut changed = 0_usize;
        for transition in transitions {
            match transition.kind {
                RoomMembershipTransitionKind::Left => {
                    if self.room_leave_states.get(&transition.room_id)
                        == Some(&RoomLeaveState::PendingLeftObservation)
                    {
                        self.room_leave_states
                            .insert(transition.room_id, RoomLeaveState::LeftObserved);
                        changed += 1;
                    }
                }
                RoomMembershipTransitionKind::Joined | RoomMembershipTransitionKind::Invited => {
                    if self.room_leave_states.get(&transition.room_id)
                        == Some(&RoomLeaveState::LeftObserved)
                    {
                        self.room_leave_states.remove(&transition.room_id);
                        self.session_subscribed_rooms.insert(transition.room_id);
                        changed += 1;
                    }
                }
            }
        }
        record_residency_intent("membership", "accepted", changed, 0);
        self.reconcile_subscriptions(SubscriptionReconcileTrigger::Membership)
            .await;
    }
    pub(super) async fn handle_room_left(&mut self, room_id: OwnedRoomId, cause: RoomRemovalCause) {
        self.room_leave_states
            .insert(room_id.clone(), RoomLeaveState::PendingLeftObservation);
        self.session_subscribed_rooms.remove(&room_id);
        record_residency_intent("room_left", cause.token(), 0, 1);
        self.reconcile_subscriptions(SubscriptionReconcileTrigger::RoomLeft)
            .await;
    }
    pub(super) async fn handle_room_rejoined(&mut self, room_id: OwnedRoomId) {
        self.room_leave_states.remove(&room_id);
        self.session_subscribed_rooms.insert(room_id);
        record_residency_intent("room_rejoined", "acknowledged", 1, 0);
        self.reconcile_subscriptions(SubscriptionReconcileTrigger::RoomRejoined)
            .await;
    }
    /// Stable process-local ordinal for a room's coverage-correlation records.
    fn room_ordinal_for(&mut self, room_id: OwnedRoomId) -> u64 {
        if let Some(ordinal) = self.subscription_room_ordinals.get(&room_id) {
            return *ordinal;
        }
        self.next_subscription_room_ordinal += 1;
        let ordinal = self.next_subscription_room_ordinal;
        self.subscription_room_ordinals.insert(room_id, ordinal);
        ordinal
    }
    /// Add a room-ID lease for a live Timeline actor.
    pub(super) fn lease_room(&mut self, room_id: OwnedRoomId) {
        *self.subscribed_room_leases.entry(room_id).or_default() += 1;
    }
    /// Release one room-ID lease; returns true when the last lease was dropped.
    pub(super) fn release_room_lease(&mut self, room_id: &OwnedRoomId) -> bool {
        let Some(count) = self.subscribed_room_leases.get_mut(room_id) else {
            return false;
        };
        *count = count.saturating_sub(1);
        if *count == 0 {
            self.subscribed_room_leases.remove(room_id);
            true
        } else {
            false
        }
    }
    /// Whether a room currently has live lease coverage.
    pub(super) fn room_is_leased(&self, room_id: &str) -> bool {
        matrix_sdk::ruma::RoomId::parse(room_id)
            .ok()
            .is_some_and(|room_id| self.subscribed_room_leases.contains_key(&room_id))
    }
    /// Atomically reconcile the live Sliding Sync room-subscription set to the
    /// session-resident desired set. Exact-set reconciles are true no-ops:
    /// retained rooms are never invalidated and presentation-only timeline
    /// changes never replace a live subscription.
    pub(super) async fn reconcile_subscriptions(&mut self, trigger: SubscriptionReconcileTrigger) {
        let Some(service) = &self.room_list_service else {
            return;
        };
        let generation_before = service.subscription_generation().get();
        let previous_active_count = service.active_room_subscriptions().len();
        let desired_count = self.session_subscribed_rooms.len();
        let desired: Vec<&matrix_sdk::ruma::RoomId> = self
            .session_subscribed_rooms
            .iter()
            .map(|room_id| room_id.as_ref())
            .collect();
        let result = service
            .reconcile_room_subscriptions_with_generation(&desired)
            .await;
        if !result.noop {
            // Rotation correlation: per-room continuous-coverage tokens
            // derived from the SDK's ATOMIC reconciliation result (a session
            // expiry clears the real map, so such rooms are classified as
            // added, not retained). A retained room kept coverage; an added
            // room that was seen before is a security-required re-add
            // (coverage lost); a first-time add has unknown prior coverage.
            for room_id in &result.retained_rooms {
                koushi_diagnostics::increment_counter("subscription_room_continuous");
                record_subscription_room_coverage(
                    self.room_ordinal_for(room_id.clone()),
                    "continuous_coverage",
                    "true",
                );
            }
            for room_id in &result.added_rooms {
                let readded = self.subscription_room_seen.contains(room_id);
                if readded {
                    koushi_diagnostics::increment_counter("subscription_room_readded");
                    record_subscription_room_coverage(
                        self.room_ordinal_for(room_id.clone()),
                        "continuous_coverage",
                        "false",
                    );
                } else {
                    koushi_diagnostics::increment_counter("subscription_room_added_new");
                    record_subscription_room_coverage(
                        self.room_ordinal_for(room_id.clone()),
                        "continuous_coverage",
                        "unknown",
                    );
                }
                self.subscription_room_seen.insert(room_id.clone());
            }
        }
        if !result.noop {
            // A generation change caused by another room must not make a
            // retained room falsely stale: advance every retained Room actor's
            // expected subscription generation so checkpoint matching keeps
            // working (issue #518 generation ownership). The ordered update
            // message reaches the actor before any new checkpoint the manager
            // forwards afterwards (same FIFO channel).
            let generation = result.generation.get();
            let retained_room_keys = self
                .timelines
                .iter()
                .filter(|(actor_key, _)| matches!(actor_key.kind, TimelineKind::Room { .. }))
                .map(|(actor_key, _)| actor_key.clone())
                .collect::<Vec<_>>();
            for actor_key in retained_room_keys {
                if let Some(handle) = self.timelines.get_mut(&actor_key) {
                    handle.subscription_generation = Some(generation);
                    let _ = handle
                        .send(TimelineActorMessage::UpdateSubscriptionGeneration(
                            generation,
                        ))
                        .await;
                }
            }
        }
        record_subscription_reconcile(
            trigger.token(),
            previous_active_count,
            desired_count,
            generation_before,
            &result,
        );
    }
    async fn subscribe_existing_timeline_rooms(
        &mut self,
        service: &Arc<matrix_sdk_ui::room_list_service::RoomListService>,
    ) {
        self.room_list_service = Some(service.clone());
        // The SDK's actual room map is importable only when it was restored
        // with the matching non-empty Sliding Sync position. UnknownPos and
        // expiry clear that proof, so those rooms are deliberately ignored.
        #[cfg(any(test, feature = "test-hooks"))]
        let (restored_from_shared_position, restored_rooms) = self
            .restored_room_subscription_probe
            .take()
            .unwrap_or_else(|| {
                (
                    service.has_restored_room_subscriptions(),
                    service.actual_subscribed_rooms(),
                )
            });
        #[cfg(not(any(test, feature = "test-hooks")))]
        let (restored_from_shared_position, restored_rooms) = (
            service.has_restored_room_subscriptions(),
            service.actual_subscribed_rooms(),
        );
        if restored_from_shared_position {
            let imported = restored_rooms
                .into_iter()
                .filter(|room_id| !self.room_leave_states.contains_key(room_id));
            self.session_subscribed_rooms.extend(imported);
            koushi_diagnostics::increment_counter("subscription_restore_proven");
            koushi_diagnostics::record(
                DiagnosticEvent::new(DiagnosticLevel::Info, "core.subscription", "restore")
                    .field(DiagnosticField::token("outcome", "proven"))
                    .field(DiagnosticField::count(
                        "restored_room_count_bucket",
                        subscription_count_bucket(self.session_subscribed_rooms.len()),
                    )),
            );
        } else if !restored_rooms.is_empty() {
            koushi_diagnostics::increment_counter("subscription_restore_unproven");
            koushi_diagnostics::record(
                DiagnosticEvent::new(DiagnosticLevel::Info, "core.subscription", "restore")
                    .field(DiagnosticField::token("outcome", "unproven"))
                    .field(DiagnosticField::count(
                        "restored_room_count_bucket",
                        subscription_count_bucket(restored_rooms.len()),
                    )),
            );
        }
        // SyncStarted performs one deduplicated reconcile of the complete
        // session-resident set; per-actor rebuilds must not re-subscribe.
        self.reconcile_subscriptions(if restored_from_shared_position {
            SubscriptionReconcileTrigger::Restore
        } else {
            SubscriptionReconcileTrigger::SyncStarted
        })
        .await;
    }
    async fn rebuild_existing_room_timelines_after_sync_started(&mut self) {
        let keys = self
            .timelines
            .keys()
            .filter(|key| matches!(key.kind, TimelineKind::Room { .. }))
            .cloned()
            .collect::<Vec<_>>();
        for key in keys {
            self.replace_existing_room_timeline_after_sync_started(key)
                .await;
        }
    }
    async fn replace_existing_room_timeline_after_sync_started(&mut self, key: TimelineKey) {
        let request_id = internal_timeline_request_id();
        // The activation fences the previous actor before the replacement can
        // spawn and refresh the shared replay-known registry.
        let activation = self
            .timeline_actor_generations
            .activate_after_quiescence(&key)
            .await;
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
                koushi_protocol::command::InitialBackfillPolicy::Disabled,
            )
            .await
        {
            Ok(handle) => {
                self.emit_timeline_subscribed_action(&key).await;
                if let Some(previous) = self.timelines.insert(key.clone(), handle) {
                    previous.stop().await;
                }
                self.restore_foreground_gap_demand(&key).await;
                self.replay_retained_room_subscription_checkpoint(&key)
                    .await;
            }
            Err(kind) => {
                self.timeline_actor_generations
                    .restore_failed_activation(&key, activation);
                self.emit_subscription_failure(request_id, &key, kind, true)
                    .await;
            }
        }
    }
}

#[cfg(test)]
mod tests;
