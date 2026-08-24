use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel};
use koushi_sdk::{
    MatrixCommittedRoomTimelineCheckpoint as MatrixRoomSubscriptionCheckpoint,
    MatrixLiveTailRefreshCancellation, MatrixLiveTailRefreshOutcome, MatrixLiveTailRefreshResult,
    MatrixTimelineContinuity, MatrixTimelineGapError, MatrixTimelineGapHandle,
    MatrixTimelineGapInspection, MatrixTimelineGapRepairBudget, MatrixTimelineGapRepairOutcome,
    MatrixTimelineGapRepairResult,
};
use koushi_state::{AppAction, TimelineContinuityInspection, TimelineGapRepairFailureKind};

use matrix_sdk_ui::timeline::GapRepairProjectionId;
#[cfg(test)]
use tokio::sync::oneshot;

use crate::account_work::AccountWorkKind;
use crate::causal_projection::{
    CausalProjectionDomain, CausalProjectionId, CausalProjectionOperationId,
    next_causal_projection_serial,
};
use crate::event::{
    CoreEvent, TimelineEvent, TimelineGapId, TimelineGapPosition, TimelineItem, TimelineItemId,
};
use crate::executor;
use crate::ids::{TimelineBatchId, TimelineGeneration, TimelineKey, TimelineKind};
use crate::live_catchup::{LiveCatchupGate, classify_live_catchup_gate};
use crate::live_tail_freshness::{
    FOREGROUND_LIVE_TAIL_LIMIT, LiveTailFreshnessState, LiveTailSchedulerAction,
};

// BEGIN GENERATED SIBLING IMPORTS
use super::actor::{TimelineActor, TimelineActorControl, TimelineActorMessage};
use super::diagnostics::{
    TimelineGapSelectionDiagnostic, record_live_catchup_gate, record_live_tail_cancellation,
    record_live_tail_commit, record_live_tail_queue, record_live_tail_reconciliation,
    record_live_tail_refresh, record_live_tail_state, record_timeline_gap_projection,
    record_timeline_gap_projection_boundary, record_timeline_gap_repair,
    record_timeline_gap_selection,
};
use super::manager::{TimelineManagerActor, TimelineMessage};
use super::read_state::ReadRetrySource;
// END GENERATED SIBLING IMPORTS

/// One absolute foreground bound for delivering a live-tail cancellation and
/// receiving the actor acknowledgement. The scheduler invalidates the
/// operation generation before entering this wait, so expiry is safe: a late
/// actor completion is stale and room navigation may continue.

pub(super) const LIVE_TAIL_CANCELLATION_DEADLINE: Duration = Duration::from_millis(100);

impl TimelineManagerActor {
    pub(super) async fn invalidate_live_tail_epoch_for_existing_rooms(
        &mut self,
        service_epoch: u64,
    ) -> Vec<LiveTailSchedulerAction<TimelineKey>> {
        let keys = self
            .timelines
            .keys()
            .filter(|key| matches!(key.kind, TimelineKind::Room { .. }))
            .filter(|key| self.live_tail_refreshes.freshness(key).is_some())
            .cloned()
            .collect::<Vec<_>>();
        let mut pending_start = None;
        for key in keys {
            let from = self.live_tail_refreshes.freshness(&key);
            let actions = self
                .live_tail_refreshes
                .invalidate_epoch(key.clone(), service_epoch);
            record_live_tail_state(
                from,
                self.live_tail_refreshes.freshness(&key),
                service_epoch,
            );
            record_live_tail_queue("foreground", &actions);
            for action in actions {
                match action {
                    LiveTailSchedulerAction::Start { .. } => {
                        debug_assert!(pending_start.is_none());
                        pending_start = Some(action);
                    }
                    LiveTailSchedulerAction::CancelNetwork {
                        key,
                        operation_generation,
                    } => {
                        let cancels_pending = pending_start.as_ref().is_some_and(|pending| {
                            matches!(
                                pending,
                                LiveTailSchedulerAction::Start {
                                    key: pending_key,
                                    operation_generation: pending_operation,
                                    ..
                                } if pending_key == &key
                                    && *pending_operation == operation_generation
                            )
                        });
                        if cancels_pending {
                            pending_start = None;
                        } else {
                            self.apply_live_tail_scheduler_actions(vec![
                                LiveTailSchedulerAction::CancelNetwork {
                                    key,
                                    operation_generation,
                                },
                            ])
                            .await;
                        }
                    }
                }
            }
        }
        pending_start
            .filter(|pending| {
                matches!(
                    pending,
                    LiveTailSchedulerAction::Start {
                        key,
                        epoch,
                        operation_generation,
                        ..
                    } if matches!(
                        self.live_tail_refreshes.freshness(key),
                        Some(LiveTailFreshnessState::Refreshing {
                            epoch: state_epoch,
                            operation_generation: state_operation,
                            ..
                        }) if state_epoch == *epoch && state_operation == *operation_generation
                    )
                )
            })
            .into_iter()
            .collect()
    }
    pub(super) async fn handle_room_subscription_checkpoint(
        &mut self,
        service_epoch: u64,
        checkpoint: MatrixRoomSubscriptionCheckpoint,
    ) {
        if service_epoch != self.room_subscription_service_epoch {
            return;
        }
        self.wake_desired_reads_for_room(checkpoint.room_id(), ReadRetrySource::Checkpoint)
            .await;
        let matching_keys = self
            .timelines
            .iter()
            .filter(|(key, handle)| {
                matches!(key.kind, TimelineKind::Room { .. })
                    && key.room_id() == checkpoint.room_id()
                    && handle.subscription_generation == Some(checkpoint.generation())
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in matching_keys {
            let from = self.live_tail_refreshes.freshness(&key);
            let actions = self
                .live_tail_refreshes
                .mark_fresh(key.clone(), service_epoch);
            record_live_tail_state(
                from,
                self.live_tail_refreshes.freshness(&key),
                service_epoch,
            );
            record_live_tail_queue("foreground", &actions);
            self.apply_live_tail_scheduler_actions(actions).await;
            if let Some(handle) = self.timelines.get(&key) {
                let _ = handle
                    .send(TimelineActorMessage::RoomSubscriptionCheckpoint(
                        checkpoint.clone(),
                    ))
                    .await;
            }
        }
    }
    pub(super) async fn handle_all_rooms_response_committed(
        &mut self,
        core_generation: u64,
        response_sequence: u64,
    ) {
        let commit = GlobalResponseCommit::new(core_generation, response_sequence);
        let Some(current) = self.global_response_commit else {
            return;
        };
        if core_generation != current.core_generation || commit <= current {
            return;
        }
        self.global_response_commit = Some(commit);

        // The SDK publishes room-subscription checkpoints before the global
        // response commit. Replay the retained values through the manager
        // first so an updated active room suppresses the omission-only probe.
        if let Some(service) = self.room_list_service.clone() {
            let retained = service.room_subscription_checkpoints().get();
            for checkpoint in retained.values() {
                self.handle_room_subscription_checkpoint(
                    self.room_subscription_service_epoch,
                    MatrixRoomSubscriptionCheckpoint::from_room_subscription(checkpoint),
                )
                .await;
            }
        }

        let targets = self
            .timelines
            .iter()
            .filter(|(key, _)| is_global_commit_inspection_target(&key.kind))
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in targets {
            if let Some(handle) = self.timelines.get(&key) {
                let _ = handle
                    .send(TimelineActorMessage::GlobalResponseCommitted(commit))
                    .await;
            }
        }
    }
    pub(super) async fn apply_live_tail_scheduler_actions(
        &mut self,
        actions: Vec<LiveTailSchedulerAction<TimelineKey>>,
    ) {
        for action in actions {
            match action {
                LiveTailSchedulerAction::CancelNetwork {
                    key,
                    operation_generation,
                } => {
                    let Some(handle) = self.timelines.get(&key) else {
                        continue;
                    };
                    let started = Instant::now();
                    let outcome = if handle.cancel_live_tail_network(operation_generation) {
                        "admitted"
                    } else {
                        "actor_closed"
                    };
                    record_live_tail_cancellation(
                        outcome,
                        operation_generation,
                        started.elapsed().as_millis(),
                    );
                }
                LiveTailSchedulerAction::Start {
                    key,
                    epoch,
                    operation_generation,
                    limit,
                } => {
                    debug_assert_eq!(limit, FOREGROUND_LIVE_TAIL_LIMIT);
                    if let Some(handle) = self.timelines.get(&key) {
                        let deadline = executor::Instant::now() + LIVE_TAIL_CANCELLATION_DEADLINE;
                        let _ = executor::timeout_at(
                            deadline,
                            handle.send_control(TimelineActorControl::StartLiveTailRefresh {
                                epoch,
                                operation_generation,
                                limit,
                            }),
                        )
                        .await;
                    }
                }
            }
        }
    }
    pub(super) async fn handle_live_tail_refresh_completed(
        &mut self,
        key: TimelineKey,
        actor_generation: u64,
        epoch: u64,
        operation_generation: u64,
        outcome: MatrixLiveTailRefreshOutcome,
        requested_limit: u16,
        returned_events: usize,
        duration_ms: u128,
    ) {
        let Some(actor_lease) = self
            .timeline_actor_generations
            .try_acquire(&key, actor_generation)
        else {
            return;
        };
        drop(actor_lease);
        let from = self.live_tail_refreshes.freshness(&key);
        if !matches!(
            from,
            Some(LiveTailFreshnessState::Refreshing {
                epoch: running_epoch,
                operation_generation: running_operation,
                ..
            }) if running_epoch == epoch && running_operation == operation_generation
        ) {
            return;
        }
        let historical_gap_remaining = matches!(
            outcome,
            MatrixLiveTailRefreshOutcome::Detached {
                historical_gap_remaining: true,
                ..
            }
        );
        record_live_tail_refresh(
            outcome,
            requested_limit,
            returned_events,
            historical_gap_remaining,
            operation_generation,
            duration_ms,
        );
        let actions =
            self.live_tail_refreshes
                .finish(key.clone(), epoch, operation_generation, outcome);
        record_live_tail_state(from, self.live_tail_refreshes.freshness(&key), epoch);
        record_live_tail_queue("delayed", &actions);
        self.apply_live_tail_scheduler_actions(actions).await;
    }
    pub(super) async fn replay_retained_room_subscription_checkpoint(&mut self, key: &TimelineKey) {
        if !matches!(key.kind, TimelineKind::Room { .. }) {
            return;
        }
        let Some(service) = self.room_list_service.clone() else {
            return;
        };
        let Ok(room_id) = matrix_sdk::ruma::RoomId::parse(key.room_id()) else {
            return;
        };
        let checkpoints = service.room_subscription_checkpoints();
        let retained = checkpoints.get();
        let Some(checkpoint) = retained.get(&room_id) else {
            return;
        };
        self.handle_room_subscription_checkpoint(
            self.room_subscription_service_epoch,
            MatrixRoomSubscriptionCheckpoint::from_room_subscription(checkpoint),
        )
        .await;
    }
}

fn is_global_commit_inspection_target(kind: &TimelineKind) -> bool {
    matches!(kind, TimelineKind::Room { .. })
}

#[cfg(test)]
pub(super) struct TestGapRepairCompletionPause {
    reached: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
    forwarded: oneshot::Sender<bool>,
}

const MAX_TIMELINE_GAP_REPAIR_BATCHES: u32 = 32;

const MAX_LIVE_EDGE_GAP_REPAIR_BATCHES: u32 = 4;

const TIMELINE_GAP_OBSERVABLE_SETTLEMENT_TIMEOUT: Duration = Duration::from_secs(5);

const TIMELINE_GAP_RELAY_SETTLEMENT_TIMEOUT: Duration = Duration::from_secs(5);

const TIMELINE_GAP_RENDER_SETTLEMENT_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) fn historical_causal_projection_operation(serial: u64) -> CausalProjectionOperationId {
    CausalProjectionOperationId::new(CausalProjectionDomain::HistoricalGap, serial)
        .expect("historical projection serial must stay within its 63-bit domain")
}

pub(super) fn live_tail_causal_projection_operation(serial: u64) -> CausalProjectionOperationId {
    CausalProjectionOperationId::new(CausalProjectionDomain::LiveTail, serial)
        .expect("live-tail projection serial must stay within its 63-bit domain")
}

impl CausalProjectionId {
    /// Decode the SDK/UI transport tag once, at the relay boundary. Downstream
    /// Core code routes only this typed identity and never reinterprets the
    /// raw numeric generation.
    pub(super) fn decode_transport(projection: GapRepairProjectionId) -> Self {
        Self {
            actor_generation: projection.actor_generation,
            operation: CausalProjectionOperationId::decode_transport(projection.repair_generation),
            projection_batch: projection.projection_batch,
        }
    }

    fn encode_transport(self) -> GapRepairProjectionId {
        GapRepairProjectionId {
            actor_generation: self.actor_generation,
            repair_generation: self.operation.encode_transport(),
            projection_batch: self.projection_batch,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TimelineGapObservableSettlement {
    Observable,
    NoProjection,
    TimedOut,
}

async fn wait_for_gap_repair_projection_with_timeout<F>(
    timeout: Duration,
    projection: F,
) -> TimelineGapObservableSettlement
where
    F: std::future::Future<Output = bool>,
{
    match executor::timeout(timeout, projection).await {
        Ok(true) => TimelineGapObservableSettlement::Observable,
        Ok(false) => TimelineGapObservableSettlement::NoProjection,
        Err(_) => TimelineGapObservableSettlement::TimedOut,
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum TimelineGapRepairTrigger {
    Automatic,
    LiveEdge,
    /// Publish the current gap topology after a detached live-tail refresh
    /// without consuming its continuation token through automatic repair.
    LiveTailSnapshot,
    Manual,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TimelineGapRenderFence {
    pub(super) actor_generation: u64,
    pub(super) timeline_generation: TimelineGeneration,
    pub(super) repair_generation: u64,
    pub(super) minimum_batch_id: TimelineBatchId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TimelineGapProjectionCompletion {
    NoDiff,
    Pending,
    Ready(TimelineBatchId),
}

#[derive(Debug, Default)]
pub(super) struct TimelineGapProjectionCorrelation {
    pub(super) operation: Option<(u64, CausalProjectionOperationId)>,
    pub(super) observed_batches: BTreeMap<u32, TimelineBatchId>,
    pub(super) expected_last_projection_batch: Option<u32>,
}

impl TimelineGapProjectionCorrelation {
    pub(super) fn begin(&mut self, actor_generation: u64, operation: CausalProjectionOperationId) {
        self.operation = Some((actor_generation, operation));
        self.observed_batches.clear();
        self.expected_last_projection_batch = None;
    }

    pub(super) fn complete(
        &mut self,
        actor_generation: u64,
        operation: CausalProjectionOperationId,
        last_projection_batch: Option<u32>,
    ) -> TimelineGapProjectionCompletion {
        if self.operation != Some((actor_generation, operation)) {
            return TimelineGapProjectionCompletion::NoDiff;
        }
        let Some(expected) = last_projection_batch else {
            self.clear(actor_generation, operation);
            return TimelineGapProjectionCompletion::NoDiff;
        };
        self.expected_last_projection_batch = Some(expected);
        if let Some(batch_id) = self.observed_batches.get(&expected).copied() {
            self.clear(actor_generation, operation);
            TimelineGapProjectionCompletion::Ready(batch_id)
        } else {
            TimelineGapProjectionCompletion::Pending
        }
    }

    pub(super) fn observe(
        &mut self,
        projection: CausalProjectionId,
        batch_id: TimelineBatchId,
    ) -> Option<TimelineBatchId> {
        if self.operation != Some((projection.actor_generation, projection.operation)) {
            return None;
        }
        self.observed_batches
            .insert(projection.projection_batch, batch_id);
        if self.expected_last_projection_batch != Some(projection.projection_batch) {
            return None;
        }
        self.clear(projection.actor_generation, projection.operation);
        Some(batch_id)
    }

    fn clear(&mut self, actor_generation: u64, operation: CausalProjectionOperationId) {
        if self.operation == Some((actor_generation, operation)) {
            self.operation = None;
            self.observed_batches.clear();
            self.expected_last_projection_batch = None;
        }
    }

    pub(super) fn is_pending(&self) -> bool {
        self.operation.is_some()
    }

    pub(super) fn accepts(&self, projection: CausalProjectionId) -> bool {
        self.operation == Some((projection.actor_generation, projection.operation))
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct CausalProjectionObservation {
    pub(super) historical_gap_batch_id: Option<TimelineBatchId>,
    pub(super) live_tail_batch_id: Option<TimelineBatchId>,
}

pub(super) fn observe_causal_projection(
    historical_gap: &mut TimelineGapProjectionCorrelation,
    live_tail: &mut TimelineGapProjectionCorrelation,
    projection: CausalProjectionId,
    batch_id: TimelineBatchId,
) -> CausalProjectionObservation {
    match projection.operation.domain {
        CausalProjectionDomain::HistoricalGap => CausalProjectionObservation {
            historical_gap_batch_id: historical_gap.observe(projection, batch_id),
            live_tail_batch_id: None,
        },
        CausalProjectionDomain::LiveTail => CausalProjectionObservation {
            historical_gap_batch_id: None,
            live_tail_batch_id: live_tail.observe(projection, batch_id),
        },
    }
}

#[derive(Debug, Default)]
pub(super) struct RestoreCausalProjectionBuffer {
    pub(super) projections: BTreeSet<CausalProjectionId>,
}

impl RestoreCausalProjectionBuffer {
    pub(super) fn buffer_batch(&mut self, projections: BTreeSet<CausalProjectionId>) {
        self.projections.extend(projections);
    }

    pub(super) fn observe_after_publication(
        &mut self,
        historical_gap: &mut TimelineGapProjectionCorrelation,
        live_tail: &mut TimelineGapProjectionCorrelation,
        published_batch_id: TimelineBatchId,
    ) -> CausalProjectionObservation {
        std::mem::take(&mut self.projections).into_iter().fold(
            CausalProjectionObservation::default(),
            |mut ready, projection| {
                let observation = observe_causal_projection(
                    historical_gap,
                    live_tail,
                    projection,
                    published_batch_id,
                );
                ready.historical_gap_batch_id = ready
                    .historical_gap_batch_id
                    .or(observation.historical_gap_batch_id);
                ready.live_tail_batch_id =
                    ready.live_tail_batch_id.or(observation.live_tail_batch_id);
                ready
            },
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct PendingTimelineGapProjection {
    pub(super) trigger: TimelineGapRepairTrigger,
    repair_generation: u64,
    gap_count: u32,
    batches_processed: u32,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct PendingLiveTailRefreshCompletion {
    actor_generation: u64,
    epoch: u64,
    operation_generation: u64,
    outcome: MatrixLiveTailRefreshOutcome,
    requested_limit: u16,
    returned_events: usize,
    duration_ms: u128,
}

fn recover_obsolete_gap_settlement(
    correlation: &mut TimelineGapProjectionCorrelation,
    pending_projection: &mut Option<PendingTimelineGapProjection>,
    tracker: &mut TimelineGapRepairTracker,
    actor_generation: u64,
    repair_generation: u64,
    trigger: TimelineGapRepairTrigger,
) -> bool {
    let operation = historical_causal_projection_operation(repair_generation);
    if correlation.operation != Some((actor_generation, operation)) {
        return false;
    }
    correlation.clear(actor_generation, operation);
    if pending_projection
        .as_ref()
        .is_some_and(|pending| pending.repair_generation == repair_generation)
    {
        pending_projection.take();
    }
    let _ = tracker.finish_work(repair_generation);
    tracker.queue_inspection(trigger);
    true
}

/// One bounded batch per scheduler permit. The event bound comes from the work
/// policy so the batch size has a single owner.
fn timeline_gap_repair_budget(
    trigger: TimelineGapRepairTrigger,
    work_kind: AccountWorkKind,
) -> MatrixTimelineGapRepairBudget {
    MatrixTimelineGapRepairBudget {
        event_limit: work_kind.policy().batch_limit,
        cached_chunk_limit: match trigger {
            TimelineGapRepairTrigger::LiveTailSnapshot => 0,
            TimelineGapRepairTrigger::Automatic
            | TimelineGapRepairTrigger::LiveEdge
            | TimelineGapRepairTrigger::Manual => 1,
        },
    }
}

pub(super) fn timeline_gap_repair_trigger_token(trigger: TimelineGapRepairTrigger) -> &'static str {
    match trigger {
        TimelineGapRepairTrigger::Automatic => "cache_gap",
        TimelineGapRepairTrigger::LiveEdge => "live_edge",
        TimelineGapRepairTrigger::LiveTailSnapshot => "live_tail_snapshot",
        TimelineGapRepairTrigger::Manual => "manual",
    }
}

/// Pick the only inspection that may follow one published SDK diff batch.
///
/// Live-tail refreshes can publish several causally tagged batches. They are
/// not historical-gap repairs: intermediate batches must not wake automatic
/// or live-edge repair, and only the exact final batch that released the
/// refresh completion may publish the observe-only snapshot.
pub(super) fn post_diff_gap_inspection_trigger(
    has_live_tail_projection: bool,
    live_tail_completion_published: bool,
    live_edge_target_changed: bool,
) -> Option<TimelineGapRepairTrigger> {
    if live_tail_completion_published {
        Some(TimelineGapRepairTrigger::LiveTailSnapshot)
    } else if has_live_tail_projection {
        None
    } else if live_edge_target_changed {
        Some(TimelineGapRepairTrigger::LiveEdge)
    } else {
        Some(TimelineGapRepairTrigger::Automatic)
    }
}

fn live_tail_completion_requires_snapshot(outcome: MatrixLiveTailRefreshOutcome) -> bool {
    matches!(
        outcome,
        MatrixLiveTailRefreshOutcome::Unchanged
            | MatrixLiveTailRefreshOutcome::Advanced { .. }
            | MatrixLiveTailRefreshOutcome::Detached { .. }
    )
}

fn timeline_gap_repair_made_progress(outcome: &MatrixTimelineGapRepairOutcome) -> bool {
    match outcome {
        MatrixTimelineGapRepairOutcome::Deferred {
            cached_chunks_loaded,
        } => *cached_chunks_loaded > 0,
        MatrixTimelineGapRepairOutcome::Progress { events } => *events > 0,
        MatrixTimelineGapRepairOutcome::BoundariesJoined { .. }
        | MatrixTimelineGapRepairOutcome::StartReached { .. } => true,
        MatrixTimelineGapRepairOutcome::Stale | MatrixTimelineGapRepairOutcome::Failed => false,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TimelineGapRepairResultDiagnostic {
    outcome: &'static str,
    events: usize,
    cached_chunks_loaded: usize,
    has_projection_batch: bool,
    made_progress: bool,
}

fn timeline_gap_repair_result_diagnostic(
    result: &Result<MatrixTimelineGapRepairResult, MatrixTimelineGapError>,
) -> TimelineGapRepairResultDiagnostic {
    let (outcome, events, cached_chunks_loaded, made_progress) = match result {
        Ok(result) => match result.outcome {
            MatrixTimelineGapRepairOutcome::Deferred {
                cached_chunks_loaded,
            } => (
                "deferred",
                0,
                cached_chunks_loaded,
                cached_chunks_loaded > 0,
            ),
            MatrixTimelineGapRepairOutcome::Progress { events } => {
                ("progress", events, 0, events > 0)
            }
            MatrixTimelineGapRepairOutcome::BoundariesJoined { events } => {
                ("boundaries_joined", events, 0, true)
            }
            MatrixTimelineGapRepairOutcome::StartReached { events } => {
                ("start_reached", events, 0, true)
            }
            MatrixTimelineGapRepairOutcome::Stale => ("stale", 0, 0, false),
            MatrixTimelineGapRepairOutcome::Failed => ("failed", 0, 0, false),
        },
        Err(_) => ("error", 0, 0, false),
    };
    TimelineGapRepairResultDiagnostic {
        outcome,
        events,
        cached_chunks_loaded,
        has_projection_batch: result
            .as_ref()
            .is_ok_and(|result| result.last_projection_batch.is_some()),
        made_progress,
    }
}

fn record_timeline_gap_repair_attempt(
    admission: TimelineGapAttemptAdmission,
    demand_revision: u64,
) {
    koushi_diagnostics::record_and_stderr(
        DiagnosticEvent::new(
            DiagnosticLevel::Info,
            "core.timeline_gap_repair",
            "attempt_admitted",
        )
        .field(DiagnosticField::count(
            "attempt_number",
            admission.attempt_number,
        ))
        .field(DiagnosticField::token(
            "reset_reason",
            admission.reason.as_str(),
        ))
        .field(DiagnosticField::boolean(
            "topology_changed",
            admission.topology_changed,
        ))
        .field(DiagnosticField::boolean(
            "ordinal_changed",
            admission.ordinal_changed,
        ))
        .field(DiagnosticField::boolean(
            "demand_changed",
            admission.demand_changed,
        ))
        .field(DiagnosticField::count("demand_revision", demand_revision)),
    );
}

fn admit_and_record_timeline_gap_repair_attempt(
    tracker: &mut TimelineGapRepairTracker,
    id: TimelineGapId,
    demand_revision: u64,
) -> bool {
    let Some(admission) = tracker.admit_gap_attempt(id, demand_revision) else {
        return false;
    };
    record_timeline_gap_repair_attempt(admission, demand_revision);
    true
}

fn record_timeline_gap_repair_budget(
    attempt_number: u64,
    demand_revision: u64,
    consecutive_no_progress_batches: u32,
    cached_chunks_loaded: usize,
) {
    let budget_remaining =
        MAX_TIMELINE_GAP_REPAIR_BATCHES.saturating_sub(consecutive_no_progress_batches);
    koushi_diagnostics::record_and_stderr(
        DiagnosticEvent::new(
            DiagnosticLevel::Info,
            "core.timeline_gap_repair",
            "budget_updated",
        )
        .field(DiagnosticField::count("attempt_number", attempt_number))
        .field(DiagnosticField::count("demand_revision", demand_revision))
        .field(DiagnosticField::count(
            "consecutive_no_progress_batches",
            consecutive_no_progress_batches.into(),
        ))
        .field(DiagnosticField::count(
            "budget_remaining",
            budget_remaining.into(),
        ))
        .field(DiagnosticField::count(
            "cached_chunks_loaded",
            cached_chunks_loaded.try_into().unwrap_or(u64::MAX),
        )),
    );
}

fn record_timeline_gap_repair_result(
    tracker: &mut TimelineGapRepairTracker,
    serial: u64,
    trigger: TimelineGapRepairTrigger,
    result: &Result<MatrixTimelineGapRepairResult, MatrixTimelineGapError>,
) {
    let diagnostic = timeline_gap_repair_result_diagnostic(result);
    koushi_diagnostics::record_and_stderr(
        DiagnosticEvent::new(DiagnosticLevel::Info, "core.timeline_gap_repair", "result")
            .field(DiagnosticField::token(
                "trigger",
                timeline_gap_repair_trigger_token(trigger),
            ))
            .field(DiagnosticField::count("generation", serial))
            .field(DiagnosticField::token("outcome", diagnostic.outcome))
            .field(DiagnosticField::count(
                "events",
                diagnostic.events.try_into().unwrap_or(u64::MAX),
            ))
            .field(DiagnosticField::count(
                "cached_chunks_loaded",
                diagnostic
                    .cached_chunks_loaded
                    .try_into()
                    .unwrap_or(u64::MAX),
            ))
            .field(DiagnosticField::boolean(
                "has_projection_batch",
                diagnostic.has_projection_batch,
            ))
            .field(DiagnosticField::boolean(
                "made_progress",
                diagnostic.made_progress,
            )),
    );
    let cached_chunks_loaded = match result {
        Ok(result) => {
            tracker.record_batch_outcome(&result.outcome);
            match result.outcome {
                MatrixTimelineGapRepairOutcome::Deferred {
                    cached_chunks_loaded,
                } => cached_chunks_loaded,
                MatrixTimelineGapRepairOutcome::Progress { .. }
                | MatrixTimelineGapRepairOutcome::BoundariesJoined { .. }
                | MatrixTimelineGapRepairOutcome::StartReached { .. }
                | MatrixTimelineGapRepairOutcome::Stale
                | MatrixTimelineGapRepairOutcome::Failed => 0,
            }
        }
        Err(_) => {
            tracker.record_batch_error();
            0
        }
    };
    record_timeline_gap_repair_budget(
        tracker.attempt_number,
        tracker
            .attempt_demand_revision
            .unwrap_or(tracker.demand_revision),
        tracker.consecutive_no_progress_batches,
        cached_chunks_loaded,
    );
}

fn checkpoint_is_strictly_newer(
    incoming: &MatrixRoomSubscriptionCheckpoint,
    existing: &MatrixRoomSubscriptionCheckpoint,
) -> bool {
    if incoming.same_response_as(existing) {
        return false;
    }
    incoming.generation() > existing.generation()
        || (incoming.generation() == existing.generation()
            && incoming.response_sequence() > existing.response_sequence())
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct GlobalResponseCommit {
    core_generation: u64,
    response_sequence: u64,
}

impl GlobalResponseCommit {
    pub(super) fn new(core_generation: u64, response_sequence: u64) -> Self {
        Self {
            core_generation,
            response_sequence,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GlobalCommitDecision {
    IgnoredStaleOrDuplicate,
    CoveredByRoomCheckpoint,
    InspectNewestLiveEdge,
}

#[derive(Default)]
pub(super) struct GlobalCommitFence {
    latest: Option<GlobalResponseCommit>,
    latest_room_checkpoint_response_sequence: Option<u64>,
    pending_inspection: Option<GlobalResponseCommit>,
}

impl GlobalCommitFence {
    pub(super) fn note_room_checkpoint_advanced(&mut self, response_sequence: u64) {
        if self
            .latest_room_checkpoint_response_sequence
            .is_none_or(|latest| response_sequence > latest)
        {
            self.latest_room_checkpoint_response_sequence = Some(response_sequence);
        }
    }

    pub(super) fn observe(&mut self, commit: GlobalResponseCommit) -> GlobalCommitDecision {
        if self.latest.is_some_and(|latest| commit <= latest) {
            return GlobalCommitDecision::IgnoredStaleOrDuplicate;
        }
        self.latest = Some(commit);
        if self.latest_room_checkpoint_response_sequence == Some(commit.response_sequence) {
            return GlobalCommitDecision::CoveredByRoomCheckpoint;
        }
        self.pending_inspection = Some(commit);
        GlobalCommitDecision::InspectNewestLiveEdge
    }

    fn take_pending_inspection(&mut self) -> Option<GlobalResponseCommit> {
        self.pending_inspection.take()
    }

    fn has_pending_inspection(&self) -> bool {
        self.pending_inspection.is_some()
    }

    fn restore_pending_inspection(&mut self, commit: GlobalResponseCommit) {
        if self.latest == Some(commit) && self.pending_inspection.is_none() {
            self.pending_inspection = Some(commit);
        }
    }
}

pub(super) fn retain_room_subscription_checkpoint(
    current: &mut Option<MatrixRoomSubscriptionCheckpoint>,
    deferred: &mut Option<MatrixRoomSubscriptionCheckpoint>,
    incoming: MatrixRoomSubscriptionCheckpoint,
) -> bool {
    if let Some(existing) = current.as_ref() {
        if !checkpoint_is_strictly_newer(&incoming, existing) {
            return false;
        }
        if existing.has_inserted_gap() {
            if deferred
                .as_ref()
                .is_none_or(|pending| checkpoint_is_strictly_newer(&incoming, pending))
            {
                *deferred = Some(incoming);
            }
            return false;
        }
    }

    *current = Some(incoming);
    // Any deferred checkpoint arrived before the new current checkpoint. It
    // must never be promoted after the newer current checkpoint is consumed.
    *deferred = None;
    true
}

pub(super) fn room_checkpoint_advances_global_fence(
    current: Option<&MatrixRoomSubscriptionCheckpoint>,
    deferred: Option<&MatrixRoomSubscriptionCheckpoint>,
    incoming: &MatrixRoomSubscriptionCheckpoint,
) -> bool {
    current.is_none_or(|existing| checkpoint_is_strictly_newer(incoming, existing))
        && deferred.is_none_or(|existing| checkpoint_is_strictly_newer(incoming, existing))
}

fn global_commit_gap_selection(gap_count: usize) -> GapRepairSelection {
    gap_count
        .checked_sub(1)
        .map_or(GapRepairSelection::None, |ordinal| {
            GapRepairSelection::Unprojected {
                ordinal,
                reason: UnprojectedGapReason::LiveEdge,
            }
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MissingCommittedGapDecision {
    Noop,
    Retry,
    CloseStale,
}

fn missing_committed_gap_decision(
    checkpoint_has_gap: bool,
    previous_retry: Option<(u64, u64)>,
    retry_key: (u64, u64),
) -> MissingCommittedGapDecision {
    if !checkpoint_has_gap {
        MissingCommittedGapDecision::Noop
    } else if previous_retry == Some(retry_key) {
        MissingCommittedGapDecision::CloseStale
    } else {
        MissingCommittedGapDecision::Retry
    }
}

fn consume_room_subscription_checkpoint(
    current: &mut Option<MatrixRoomSubscriptionCheckpoint>,
    deferred: &mut Option<MatrixRoomSubscriptionCheckpoint>,
    consumed: &MatrixRoomSubscriptionCheckpoint,
) -> bool {
    if !current
        .as_ref()
        .is_some_and(|checkpoint| checkpoint.same_response_as(consumed))
    {
        return false;
    }
    *current = None;
    if let Some(next) = deferred
        .take()
        .filter(|next| checkpoint_is_strictly_newer(next, consumed))
    {
        *current = Some(next);
        return true;
    }
    false
}

fn gap_repair_continuation_trigger(
    trigger: TimelineGapRepairTrigger,
    repaired_live_edge_fallback: bool,
    outcome: &MatrixTimelineGapRepairOutcome,
) -> TimelineGapRepairTrigger {
    if matches!(trigger, TimelineGapRepairTrigger::LiveEdge)
        && repaired_live_edge_fallback
        && matches!(
            outcome,
            MatrixTimelineGapRepairOutcome::BoundariesJoined { .. }
                | MatrixTimelineGapRepairOutcome::StartReached { .. }
        )
    {
        TimelineGapRepairTrigger::Automatic
    } else {
        trigger
    }
}

fn projected_gap_insertion_index(
    newer_position: Option<usize>,
    older_position: Option<usize>,
) -> Option<usize> {
    newer_position.or_else(|| older_position.map(|index| index.saturating_add(1)))
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct GapBoundaryPresenceCounts {
    pub(super) both: usize,
    pub(super) one: usize,
    pub(super) none: usize,
    pub(super) projected: usize,
}

fn summarize_gap_boundary_presence(
    boundary_presence: impl IntoIterator<Item = (bool, bool)>,
) -> GapBoundaryPresenceCounts {
    boundary_presence.into_iter().fold(
        GapBoundaryPresenceCounts::default(),
        |mut counts, (newer_present, older_present)| {
            match (newer_present, older_present) {
                (true, true) => counts.both += 1,
                (true, false) | (false, true) => counts.one += 1,
                (false, false) => counts.none += 1,
            }
            if newer_present || older_present {
                counts.projected += 1;
            }
            counts
        },
    )
}

fn projected_gap_id(topology_revision: u64, ordinal: usize) -> TimelineGapId {
    TimelineGapId {
        topology_revision,
        ordinal: ordinal.try_into().unwrap_or(u32::MAX),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ProjectedGapCandidate {
    id: TimelineGapId,
    relation: ProjectedGapRelation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProjectedGapRelation {
    ExplicitVisible,
    IntersectsViewport,
    NearestLiveEdge,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GapRepairViewportWakeDecision {
    Wake { candidate: ProjectedGapCandidate },
    WakeStaleVisibleDemand,
    IdleNoCandidate,
    IdleUnchangedCandidate { candidate: ProjectedGapCandidate },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct GapRepairEvaluationDiagnosticSignature {
    pub(super) decision: &'static str,
    pub(super) projected_gap_count: usize,
    pub(super) visible_gap_count: usize,
    pub(super) visible_gap_validated: bool,
    pub(super) candidate_changed: bool,
    pub(super) scheduler_phase: &'static str,
}

pub(super) fn projected_gaps_contain_id(
    projected: &[(usize, TimelineGapPosition)],
    id: TimelineGapId,
) -> bool {
    projected.iter().any(|(_, position)| position.id == id)
}

pub(super) fn should_record_gap_repair_evaluation(
    previous: &mut Option<GapRepairEvaluationDiagnosticSignature>,
    next: GapRepairEvaluationDiagnosticSignature,
) -> bool {
    if *previous == Some(next) {
        return false;
    }
    *previous = Some(next);
    true
}

/// Classify one gap-repair batch for the account-wide scheduler.
///
/// A gap the viewport reported as visible, and an explicitly requested repair,
/// are foreground work. Live-edge and nearest-live-edge repair for the selected
/// room is background: it must not delay a send or visible pagination.
/// Events the batch actually projected, for scheduler diagnostics only.
fn gap_repair_batch_events(
    result: &Result<MatrixTimelineGapRepairResult, MatrixTimelineGapError>,
) -> u64 {
    match result {
        Ok(result) => match result.outcome {
            MatrixTimelineGapRepairOutcome::Progress { events }
            | MatrixTimelineGapRepairOutcome::BoundariesJoined { events }
            | MatrixTimelineGapRepairOutcome::StartReached { events } => events as u64,
            MatrixTimelineGapRepairOutcome::Deferred { .. }
            | MatrixTimelineGapRepairOutcome::Stale
            | MatrixTimelineGapRepairOutcome::Failed => 0,
        },
        Err(_) => 0,
    }
}

fn gap_repair_work_kind(
    trigger: TimelineGapRepairTrigger,
    candidate: Option<ProjectedGapCandidate>,
) -> AccountWorkKind {
    if matches!(trigger, TimelineGapRepairTrigger::Manual) {
        return AccountWorkKind::VisibleGapRepair;
    }
    match candidate.map(|candidate| candidate.relation) {
        Some(ProjectedGapRelation::ExplicitVisible | ProjectedGapRelation::IntersectsViewport) => {
            AccountWorkKind::VisibleGapRepair
        }
        Some(ProjectedGapRelation::NearestLiveEdge) | None => AccountWorkKind::OffscreenGapRepair,
    }
}

fn select_projected_gap_candidate(
    projected: &[(usize, TimelineGapPosition)],
    viewport_range: Option<(usize, usize)>,
    visible_gap_ids: &[TimelineGapId],
) -> Option<ProjectedGapCandidate> {
    if !visible_gap_ids.is_empty() {
        return projected
            .iter()
            .filter(|(_, position)| visible_gap_ids.contains(&position.id))
            .map(|(_, position)| ProjectedGapCandidate {
                id: position.id,
                relation: ProjectedGapRelation::ExplicitVisible,
            })
            .next_back();
    }
    let in_viewport = viewport_range.and_then(|(first, last)| {
        let start = first.min(last);
        let end = first.max(last).saturating_add(1);
        projected
            .iter()
            .filter(|(_, position)| (start..=end).contains(&position.before_item_index))
            .map(|(_, position)| ProjectedGapCandidate {
                id: position.id,
                relation: ProjectedGapRelation::IntersectsViewport,
            })
            .next_back()
    });
    in_viewport.or_else(|| {
        projected.last().map(|(_, position)| ProjectedGapCandidate {
            id: position.id,
            relation: ProjectedGapRelation::NearestLiveEdge,
        })
    })
}

fn evaluate_gap_repair_viewport_wake(
    projected: &[(usize, TimelineGapPosition)],
    viewport_range: Option<(usize, usize)>,
    visible_gap_ids: &[TimelineGapId],
    previous: Option<ProjectedGapCandidate>,
) -> GapRepairViewportWakeDecision {
    if visible_gap_ids
        .iter()
        .any(|visible_id| !projected_gaps_contain_id(projected, *visible_id))
    {
        return GapRepairViewportWakeDecision::WakeStaleVisibleDemand;
    }
    let Some(candidate) =
        select_projected_gap_candidate(projected, viewport_range, visible_gap_ids)
    else {
        return GapRepairViewportWakeDecision::IdleNoCandidate;
    };
    if previous == Some(candidate) {
        GapRepairViewportWakeDecision::IdleUnchangedCandidate { candidate }
    } else {
        GapRepairViewportWakeDecision::Wake { candidate }
    }
}

#[cfg(test)]
fn select_projected_gap_id(
    projected: &[(usize, TimelineGapPosition)],
    viewport_range: Option<(usize, usize)>,
) -> Option<TimelineGapId> {
    select_projected_gap_candidate(projected, viewport_range, &[]).map(|candidate| candidate.id)
}

fn projected_gap_identity_matches_descriptor(
    id: TimelineGapId,
    descriptor_ordinal: usize,
    descriptor_topology_revision: u64,
) -> bool {
    usize::try_from(id.ordinal).ok() == Some(descriptor_ordinal)
        && id.topology_revision == descriptor_topology_revision
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GapRepairSelection {
    None,
    Projected {
        id: TimelineGapId,
    },
    DirectCommittedResponse,
    Unprojected {
        ordinal: usize,
        reason: UnprojectedGapReason,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UnprojectedGapReason {
    LiveEdge,
    Foreground,
    Manual,
}

fn gap_selection_diagnostic_decision(
    selection: GapRepairSelection,
    projected_candidate: Option<ProjectedGapCandidate>,
    foreground_demand_active: bool,
    gap_count: usize,
    projected_gap_count: usize,
) -> &'static str {
    if let GapRepairSelection::Projected { id } = selection {
        return match projected_candidate.filter(|candidate| candidate.id == id) {
            Some(ProjectedGapCandidate {
                relation: ProjectedGapRelation::ExplicitVisible,
                ..
            }) => "explicit_visible",
            Some(ProjectedGapCandidate {
                relation: ProjectedGapRelation::IntersectsViewport,
                ..
            }) => "viewport",
            Some(ProjectedGapCandidate {
                relation: ProjectedGapRelation::NearestLiveEdge,
                ..
            }) => "nearest_live_edge",
            None => "blocked",
        };
    }
    if matches!(
        selection,
        GapRepairSelection::DirectCommittedResponse | GapRepairSelection::Unprojected { .. }
    ) {
        return "nearest_live_edge";
    }
    if foreground_demand_active && gap_count > 0 && projected_gap_count == 0 {
        "foreground_unlocated"
    } else {
        "blocked"
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UnlocatedGapAction {
    None,
    QueueAutomatic,
    RepairNewest { ordinal: usize },
}

fn unlocated_gap_action(
    foreground_demand_active: bool,
    trigger: TimelineGapRepairTrigger,
    gap_count: usize,
    projected_gap_count: usize,
) -> UnlocatedGapAction {
    if !foreground_demand_active || projected_gap_count > 0 {
        return UnlocatedGapAction::None;
    }
    let Some(ordinal) = gap_count.checked_sub(1) else {
        return UnlocatedGapAction::None;
    };
    match trigger {
        TimelineGapRepairTrigger::Automatic => UnlocatedGapAction::RepairNewest { ordinal },
        TimelineGapRepairTrigger::LiveTailSnapshot => UnlocatedGapAction::QueueAutomatic,
        TimelineGapRepairTrigger::LiveEdge | TimelineGapRepairTrigger::Manual => {
            UnlocatedGapAction::None
        }
    }
}

fn select_gap_repair_candidate(
    trigger: TimelineGapRepairTrigger,
    projected: &[(usize, TimelineGapPosition)],
    viewport_range: Option<(usize, usize)>,
    visible_gap_ids: &[TimelineGapId],
    gap_count: usize,
    has_live_edge_target: bool,
) -> GapRepairSelection {
    if matches!(trigger, TimelineGapRepairTrigger::LiveTailSnapshot) {
        return GapRepairSelection::None;
    }
    if let Some(candidate) =
        select_projected_gap_candidate(projected, viewport_range, visible_gap_ids)
    {
        let id = candidate.id;
        return GapRepairSelection::Projected { id };
    }
    if !visible_gap_ids.is_empty() && matches!(trigger, TimelineGapRepairTrigger::Automatic) {
        return GapRepairSelection::None;
    }
    let Some(ordinal) = gap_count.checked_sub(1) else {
        return GapRepairSelection::None;
    };
    match trigger {
        TimelineGapRepairTrigger::Automatic => GapRepairSelection::None,
        TimelineGapRepairTrigger::LiveEdge if has_live_edge_target => {
            GapRepairSelection::Unprojected {
                ordinal,
                reason: UnprojectedGapReason::LiveEdge,
            }
        }
        TimelineGapRepairTrigger::LiveEdge => GapRepairSelection::None,
        TimelineGapRepairTrigger::LiveTailSnapshot => GapRepairSelection::None,
        TimelineGapRepairTrigger::Manual => GapRepairSelection::Unprojected {
            ordinal,
            reason: UnprojectedGapReason::Manual,
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LiveEdgeGapSelection {
    topology_revision: u64,
    ordinal: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LiveEdgeSelectionDecision {
    Repair,
    NoProgress,
}

pub(super) fn rendered_live_edge_target(items: &[TimelineItem]) -> Option<String> {
    items.iter().rev().find_map(|item| match &item.id {
        TimelineItemId::Event { event_id } => Some(event_id.clone()),
        TimelineItemId::Transaction { .. } | TimelineItemId::Synthetic { .. } => None,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TimelineGapAttemptResetReason {
    Initial,
    Topology,
    Ordinal,
    Demand,
}

impl TimelineGapAttemptResetReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Initial => "initial",
            Self::Topology => "topology",
            Self::Ordinal => "ordinal",
            Self::Demand => "demand",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TimelineGapAttemptAdmission {
    attempt_number: u64,
    reason: TimelineGapAttemptResetReason,
    topology_changed: bool,
    ordinal_changed: bool,
    demand_changed: bool,
}

#[derive(Default)]
pub(super) struct TimelineGapRepairTracker {
    next_serial: u64,
    pub(super) active_serial: Option<u64>,
    pub(super) pending_trigger: Option<TimelineGapRepairTrigger>,
    pub(super) awaiting_projection: Option<TimelineGapRenderFence>,
    pub(super) gap_count: u32,
    attempt_gap_id: Option<TimelineGapId>,
    attempt_demand_revision: Option<u64>,
    attempt_number: u64,
    demand_revision: u64,
    pub(super) batches_processed: u32,
    consecutive_no_progress_batches: u32,
    pub(super) projected_gaps: Vec<(usize, TimelineGapPosition)>,
    last_projected_candidate: Option<ProjectedGapCandidate>,
    live_edge_target: Option<String>,
    live_edge_batches_processed: u32,
    last_live_edge_selection: Option<LiveEdgeGapSelection>,
}

impl TimelineGapRepairTracker {
    #[cfg(test)]
    fn begin_inspection(&mut self) -> Option<u64> {
        self.begin_work()
    }

    fn begin_repair(&mut self, gap_count: u32) -> Option<u64> {
        let serial = self.begin_work()?;
        self.gap_count = gap_count;
        Some(serial)
    }

    pub(super) fn queue_inspection(&mut self, trigger: TimelineGapRepairTrigger) {
        self.pending_trigger = Some(
            self.pending_trigger
                .map_or(trigger, |pending| pending.max(trigger)),
        );
    }

    fn replace_projected_gaps(
        &mut self,
        projected_gaps: Vec<(usize, TimelineGapPosition)>,
        viewport_range: Option<(usize, usize)>,
        visible_gap_ids: &[TimelineGapId],
    ) {
        self.last_projected_candidate =
            select_projected_gap_candidate(&projected_gaps, viewport_range, visible_gap_ids);
        self.projected_gaps = projected_gaps;
    }

    pub(super) fn clear_projected_gaps(&mut self) {
        self.projected_gaps.clear();
        self.last_projected_candidate = None;
    }

    pub(super) fn observe_live_edge_target(&mut self, target: Option<String>) -> bool {
        if self.live_edge_target == target {
            return false;
        }
        self.live_edge_target = target;
        self.live_edge_batches_processed = 0;
        self.last_live_edge_selection = None;
        true
    }

    pub(super) fn has_live_edge_target(&self) -> bool {
        self.live_edge_target.is_some()
    }

    fn evaluate_live_edge_selection(
        &mut self,
        selection: LiveEdgeGapSelection,
    ) -> LiveEdgeSelectionDecision {
        if self.live_edge_batches_processed > 0 && self.last_live_edge_selection == Some(selection)
        {
            return LiveEdgeSelectionDecision::NoProgress;
        }
        self.last_live_edge_selection = Some(selection);
        LiveEdgeSelectionDecision::Repair
    }

    pub(super) fn evaluate_viewport_wake(
        &mut self,
        viewport_range: Option<(usize, usize)>,
        visible_gap_ids: &[TimelineGapId],
    ) -> GapRepairViewportWakeDecision {
        let decision = evaluate_gap_repair_viewport_wake(
            &self.projected_gaps,
            viewport_range,
            visible_gap_ids,
            self.last_projected_candidate,
        );
        if matches!(
            decision,
            GapRepairViewportWakeDecision::Wake {
                candidate: ProjectedGapCandidate {
                    relation: ProjectedGapRelation::ExplicitVisible,
                    ..
                }
            }
        ) {
            self.advance_demand_revision();
        }
        self.last_projected_candidate = match decision {
            GapRepairViewportWakeDecision::Wake { candidate }
            | GapRepairViewportWakeDecision::IdleUnchangedCandidate { candidate } => {
                Some(candidate)
            }
            GapRepairViewportWakeDecision::WakeStaleVisibleDemand
            | GapRepairViewportWakeDecision::IdleNoCandidate => None,
        };
        decision
    }

    pub(super) fn begin_explicit_demand(&mut self) -> u64 {
        let revision = self.advance_demand_revision();
        self.last_projected_candidate = None;
        revision
    }

    fn advance_demand_revision(&mut self) -> u64 {
        self.demand_revision = self.demand_revision.wrapping_add(1);
        self.demand_revision
    }

    fn begin_pending_inspection(
        &mut self,
        projection_acknowledged: bool,
    ) -> Option<(u64, TimelineGapRepairTrigger)> {
        if !projection_acknowledged
            || self.active_serial.is_some()
            || self.awaiting_projection.is_some()
        {
            return None;
        }
        let trigger = self.pending_trigger?;
        let serial = self.begin_work()?;
        self.pending_trigger = None;
        Some((serial, trigger))
    }

    #[cfg(test)]
    fn has_pending_inspection(&self) -> bool {
        self.pending_trigger.is_some()
    }

    fn await_projection(&mut self, fence: TimelineGapRenderFence) {
        self.awaiting_projection = Some(fence);
    }

    pub(super) fn acknowledge_projection(&mut self, actual: TimelineGapRenderFence) -> bool {
        let Some(required) = self.awaiting_projection else {
            return false;
        };
        if actual.actor_generation != required.actor_generation
            || actual.timeline_generation != required.timeline_generation
            || actual.repair_generation != required.repair_generation
            || actual.minimum_batch_id < required.minimum_batch_id
        {
            return false;
        }
        self.awaiting_projection = None;
        true
    }

    fn recover_projection_timeout(
        &mut self,
        expired: TimelineGapRenderFence,
        trigger: TimelineGapRepairTrigger,
    ) -> bool {
        if self.awaiting_projection != Some(expired) {
            return false;
        }
        self.awaiting_projection = None;
        self.queue_inspection(trigger);
        true
    }

    fn begin_work(&mut self) -> Option<u64> {
        if self.active_serial.is_some() {
            return None;
        }
        self.next_serial = next_causal_projection_serial(self.next_serial)?;
        self.active_serial = Some(self.next_serial);
        Some(self.next_serial)
    }

    fn finish_work(&mut self, serial: u64) -> bool {
        if self.active_serial != Some(serial) {
            return false;
        }
        self.active_serial = None;
        true
    }

    fn admit_gap_attempt(
        &mut self,
        id: TimelineGapId,
        demand_revision: u64,
    ) -> Option<TimelineGapAttemptAdmission> {
        if self.attempt_gap_id == Some(id) && self.attempt_demand_revision == Some(demand_revision)
        {
            return None;
        }
        let topology_changed = self
            .attempt_gap_id
            .is_some_and(|previous| previous.topology_revision != id.topology_revision);
        let ordinal_changed = self
            .attempt_gap_id
            .is_some_and(|previous| previous.ordinal != id.ordinal);
        let demand_changed = self
            .attempt_demand_revision
            .is_some_and(|previous| previous != demand_revision);
        let reason = if self.attempt_gap_id.is_none() {
            TimelineGapAttemptResetReason::Initial
        } else if topology_changed {
            TimelineGapAttemptResetReason::Topology
        } else if ordinal_changed {
            TimelineGapAttemptResetReason::Ordinal
        } else {
            TimelineGapAttemptResetReason::Demand
        };
        self.attempt_number = self.attempt_number.saturating_add(1);
        self.attempt_gap_id = Some(id);
        self.attempt_demand_revision = Some(demand_revision);
        self.batches_processed = 0;
        self.consecutive_no_progress_batches = 0;
        self.live_edge_batches_processed = 0;
        self.last_live_edge_selection = None;
        Some(TimelineGapAttemptAdmission {
            attempt_number: self.attempt_number,
            reason,
            topology_changed,
            ordinal_changed,
            demand_changed,
        })
    }

    fn record_batch(&mut self, trigger: TimelineGapRepairTrigger) -> Option<u32> {
        if !self.can_start_batch(trigger) {
            return None;
        }
        self.batches_processed = self.batches_processed.saturating_add(1);
        if matches!(trigger, TimelineGapRepairTrigger::LiveEdge) {
            self.live_edge_batches_processed = self.live_edge_batches_processed.saturating_add(1);
        }
        Some(self.batches_processed)
    }

    fn record_batch_error(&mut self) {
        self.consecutive_no_progress_batches =
            self.consecutive_no_progress_batches.saturating_add(1);
    }

    fn record_batch_outcome(&mut self, outcome: &MatrixTimelineGapRepairOutcome) {
        if timeline_gap_repair_made_progress(outcome) {
            self.consecutive_no_progress_batches = 0;
        } else {
            self.record_batch_error();
        }
    }

    fn can_start_batch(&self, trigger: TimelineGapRepairTrigger) -> bool {
        self.consecutive_no_progress_batches < MAX_TIMELINE_GAP_REPAIR_BATCHES
            && (!matches!(trigger, TimelineGapRepairTrigger::LiveEdge)
                || self.live_edge_batches_processed < MAX_LIVE_EDGE_GAP_REPAIR_BATCHES)
    }
}

impl TimelineActor {
    pub(super) fn start_live_tail_refresh(
        &mut self,
        epoch: u64,
        operation_generation: u64,
        limit: u16,
    ) {
        if !matches!(self.key.kind, TimelineKind::Room { .. })
            || self
                .live_tail_refresh
                .as_ref()
                .is_some_and(|(current, _, _)| *current == operation_generation)
        {
            return;
        }
        if let Some((_, cancellation, task)) = self.live_tail_refresh.take() {
            cancellation.cancel();
            drop(task);
        }

        let cancellation = MatrixLiveTailRefreshCancellation::new();
        let task_cancellation = cancellation.clone();
        let session = self.session.clone();
        let actor_tx = self.msg_tx.clone();
        let room_id = self.key.room_id().to_owned();
        let actor_generation = self.actor_generation;
        let projection_operation = live_tail_causal_projection_operation(operation_generation);
        self.live_tail_projection_correlation
            .begin(actor_generation, projection_operation);
        record_live_tail_commit("started", operation_generation);
        let task = executor::spawn(async move {
            let started = Instant::now();
            let result = session
                .refresh_room_live_tail(
                    &room_id,
                    limit,
                    actor_generation,
                    projection_operation.encode_transport(),
                    task_cancellation,
                )
                .await;
            let _ = actor_tx
                .send(TimelineActorMessage::LiveTailRefreshFinished {
                    actor_generation,
                    epoch,
                    operation_generation,
                    requested_limit: limit,
                    result,
                    duration_ms: started.elapsed().as_millis(),
                })
                .await;
        });
        self.live_tail_refresh = Some((operation_generation, cancellation, task));
    }
    pub(super) async fn handle_live_tail_refresh_finished(
        &mut self,
        actor_generation: u64,
        epoch: u64,
        operation_generation: u64,
        requested_limit: u16,
        result: MatrixLiveTailRefreshResult,
        duration_ms: u128,
    ) {
        if actor_generation != self.actor_generation
            || !self
                .live_tail_refresh
                .as_ref()
                .is_some_and(|(current, _, _)| *current == operation_generation)
        {
            return;
        }
        let _ = self.live_tail_refresh.take();
        record_live_tail_commit("completed", operation_generation);
        record_live_tail_reconciliation(result.diagnostics, operation_generation);
        let outcome = result.outcome;
        let completion = PendingLiveTailRefreshCompletion {
            actor_generation,
            epoch,
            operation_generation,
            outcome,
            requested_limit,
            returned_events: result.returned_events,
            duration_ms,
        };
        match self.live_tail_projection_correlation.complete(
            actor_generation,
            live_tail_causal_projection_operation(operation_generation),
            result.last_projection_batch,
        ) {
            TimelineGapProjectionCompletion::NoDiff | TimelineGapProjectionCompletion::Ready(_) => {
                if self.publish_live_tail_refresh_completion(completion).await {
                    self.request_timeline_gap_inspection(
                        TimelineGapRepairTrigger::LiveTailSnapshot,
                    )
                    .await;
                }
            }
            TimelineGapProjectionCompletion::Pending => {
                self.pending_live_tail_projection = Some(completion);
            }
        }
    }
    pub(super) async fn finish_pending_live_tail_projection(&mut self) -> bool {
        if let Some(completion) = self.pending_live_tail_projection.take() {
            self.publish_live_tail_refresh_completion(completion).await
        } else {
            false
        }
    }
    async fn publish_live_tail_refresh_completion(
        &self,
        completion: PendingLiveTailRefreshCompletion,
    ) -> bool {
        let snapshot_required = live_tail_completion_requires_snapshot(completion.outcome);
        let _ = self
            .manager_tx
            .send(TimelineMessage::LiveTailRefreshCompleted {
                key: self.key.clone(),
                actor_generation: completion.actor_generation,
                epoch: completion.epoch,
                operation_generation: completion.operation_generation,
                outcome: completion.outcome,
                requested_limit: completion.requested_limit,
                returned_events: completion.returned_events,
                duration_ms: completion.duration_ms,
            })
            .await;
        snapshot_required
    }
    pub(super) fn viewport_item_range(&self) -> Option<(usize, usize)> {
        self.viewport_observation
            .first_visible_event_id
            .as_deref()
            .and_then(|event_id| self.timeline_event_position(event_id))
            .zip(
                self.viewport_observation
                    .last_visible_event_id
                    .as_deref()
                    .and_then(|event_id| self.timeline_event_position(event_id)),
            )
    }
    pub(super) fn gap_repair_scheduler_phase(&self) -> &'static str {
        if !self.projection_acknowledged {
            "awaiting_projection_ack"
        } else if self.pagination_task.is_some() {
            "pagination"
        } else if self.restore_anchor.is_some() {
            "anchor_restore"
        } else if self.gap_projection_correlation.is_pending()
            || self.pending_gap_projection.is_some()
        {
            "awaiting_relay"
        } else if self.gap_repair.awaiting_projection.is_some() {
            "awaiting_render_ack"
        } else if self.gap_repair.active_serial.is_some() {
            "active"
        } else if self.gap_repair.pending_trigger.is_some() {
            "queued"
        } else {
            "idle"
        }
    }
    fn record_gap_selection_diagnostic(
        &self,
        trigger: TimelineGapRepairTrigger,
        decision: &'static str,
        repair_started: bool,
        gap_count: usize,
        projected_gap_count: usize,
    ) {
        record_timeline_gap_selection(TimelineGapSelectionDiagnostic {
            trigger: timeline_gap_repair_trigger_token(trigger),
            decision,
            repair_started,
            gap_count,
            projected_gap_count,
            visible_gap_count: self.viewport_observation.visible_gap_ids.len(),
            foreground_demand_active: self.foreground_gap_demand_active,
            foreground_demand_epoch: self.gap_repair.demand_revision,
            has_live_edge_target: self.gap_repair.has_live_edge_target(),
            scheduler_phase: self.gap_repair_scheduler_phase(),
        });
    }
    pub(super) async fn request_timeline_gap_inspection(
        &mut self,
        trigger: TimelineGapRepairTrigger,
    ) {
        if !matches!(self.key.kind, TimelineKind::Room { .. }) {
            return;
        }
        self.gap_repair.queue_inspection(trigger);
        self.start_pending_timeline_gap_inspection().await;
    }
    pub(super) async fn start_pending_timeline_gap_inspection(&mut self) {
        if self.pagination_task.is_some()
            || self.restore_anchor.is_some()
            || self.gap_projection_correlation.is_pending()
            || self.pending_gap_projection.is_some()
        {
            return;
        }
        if matches!(
            self.gap_repair.pending_trigger,
            Some(TimelineGapRepairTrigger::LiveEdge)
        ) && matches!(
            self.live_catchup_gate(),
            LiveCatchupGate::AwaitingCheckpoint | LiveCatchupGate::Stale
        ) && self.gap_repair.live_edge_batches_processed == 0
        {
            record_live_catchup_gate(
                self.live_catchup_gate(),
                self.subscription_generation,
                self.room_subscription_checkpoint.as_ref(),
                self.gap_repair_scheduler_phase(),
                self.gap_repair.batches_processed,
            );
            return;
        }
        let Some((serial, trigger)) = self
            .gap_repair
            .begin_pending_inspection(self.projection_acknowledged)
        else {
            return;
        };
        let room_id = self.key.room_id().to_owned();
        let global_commit = matches!(trigger, TimelineGapRepairTrigger::LiveEdge)
            .then(|| self.global_commit_fence.take_pending_inspection())
            .flatten();
        let committed_response = (matches!(trigger, TimelineGapRepairTrigger::LiveEdge)
            && global_commit.is_none())
        .then(|| self.room_subscription_checkpoint.clone())
        .flatten();
        record_timeline_gap_repair(
            "inspection",
            timeline_gap_repair_trigger_token(trigger),
            serial,
            self.gap_repair.gap_count,
            self.gap_repair.batches_processed,
            "started",
        );
        if !self
            .emit_action_reliable(AppAction::TimelineContinuityInspectionStarted {
                room_id: room_id.clone(),
                generation: serial,
            })
            .await
        {
            self.gap_repair.finish_work(serial);
            if let Some(global_commit) = global_commit {
                self.global_commit_fence
                    .restore_pending_inspection(global_commit);
            }
            self.gap_repair.queue_inspection(trigger);
            return;
        }
        let session = self.session.clone();
        let actor_tx = self.msg_tx.clone();
        self.gap_work_task = Some(executor::spawn(async move {
            let result = session.inspect_room_timeline_gaps(&room_id).await;
            let _ = actor_tx
                .send(TimelineActorMessage::TimelineGapInspectionFinished {
                    serial,
                    trigger,
                    committed_response,
                    global_commit,
                    result,
                })
                .await;
        }));
    }
    pub(super) fn live_catchup_gate(&self) -> LiveCatchupGate {
        if self.global_commit_fence.has_pending_inspection() {
            return LiveCatchupGate::InspectCommittedLiveEdge;
        }
        classify_live_catchup_gate(
            self.subscription_generation,
            self.room_subscription_checkpoint
                .as_ref()
                .map(|checkpoint| {
                    (
                        checkpoint.generation(),
                        checkpoint.has_timeline_update(),
                        checkpoint.has_inserted_gap(),
                    )
                }),
        )
    }
    pub(super) async fn handle_timeline_gap_inspection_finished(
        &mut self,
        serial: u64,
        trigger: TimelineGapRepairTrigger,
        committed_response: Option<MatrixRoomSubscriptionCheckpoint>,
        global_commit: Option<GlobalResponseCommit>,
        result: Result<MatrixTimelineGapInspection, MatrixTimelineGapError>,
    ) {
        if !self.gap_repair.finish_work(serial) {
            return;
        }
        self.gap_work_task = None;
        if matches!(trigger, TimelineGapRepairTrigger::LiveEdge)
            && committed_response.as_ref().is_some_and(|inspected| {
                self.room_subscription_checkpoint
                    .as_ref()
                    .is_none_or(|current| !current.same_response_as(inspected))
            })
        {
            self.gap_repair
                .queue_inspection(TimelineGapRepairTrigger::LiveEdge);
            self.start_pending_timeline_gap_inspection().await;
            return;
        }
        let room_id = self.key.room_id().to_owned();
        match result {
            Ok(inspection) => {
                record_timeline_gap_repair(
                    "inspection",
                    timeline_gap_repair_trigger_token(trigger),
                    serial,
                    inspection.gaps.len().try_into().unwrap_or(u32::MAX),
                    self.gap_repair.batches_processed,
                    match inspection.continuity {
                        MatrixTimelineContinuity::Unknown => "unknown",
                        MatrixTimelineContinuity::Gapped => "incomplete",
                        MatrixTimelineContinuity::Complete => "healthy",
                    },
                );
                let projected_gaps = self.emit_gap_positions(serial, &inspection.gaps);
                let viewport_range = self.viewport_item_range();
                self.gap_repair.replace_projected_gaps(
                    projected_gaps.clone(),
                    viewport_range,
                    &self.viewport_observation.visible_gap_ids,
                );
                let known_gap_count = inspection.gaps.len().try_into().unwrap_or(u32::MAX);
                let state_inspection = match inspection.continuity {
                    MatrixTimelineContinuity::Unknown => TimelineContinuityInspection::Unknown,
                    MatrixTimelineContinuity::Gapped => TimelineContinuityInspection::Gapped {
                        gap_count: known_gap_count,
                    },
                    MatrixTimelineContinuity::Complete => TimelineContinuityInspection::Complete,
                };
                let _ = self
                    .emit_action_reliable(AppAction::TimelineContinuityInspected {
                        room_id,
                        generation: serial,
                        inspection: state_inspection,
                    })
                    .await;
                match inspection.continuity {
                    MatrixTimelineContinuity::Gapped => {
                        self.gap_repair.gap_count = known_gap_count;
                        let mut committed_descriptor = None;
                        let mut selection = if global_commit.is_some() {
                            // A global commit proves that event-cache mutation finished for
                            // this response. It permits only the newest persisted gap to enter
                            // the existing bounded live-edge chain; viewport and foreground
                            // demand cannot redirect this omission-only repair.
                            global_commit_gap_selection(inspection.gaps.len())
                        } else if matches!(trigger, TimelineGapRepairTrigger::LiveEdge) {
                            match self.live_catchup_gate() {
                                LiveCatchupGate::RepairCheckpointGap => {
                                    committed_descriptor = self
                                        .room_subscription_checkpoint
                                        .as_ref()
                                        .filter(|current| {
                                            committed_response.as_ref().is_some_and(|inspected| {
                                                current.same_response_as(inspected)
                                            })
                                        })
                                        .and_then(|checkpoint| checkpoint.inserted_gap_handle());
                                    committed_descriptor
                                        .as_ref()
                                        .map_or(GapRepairSelection::None, |_| {
                                            GapRepairSelection::DirectCommittedResponse
                                        })
                                }
                                LiveCatchupGate::AwaitingCheckpoint
                                | LiveCatchupGate::Stale
                                | LiveCatchupGate::NoTimelineUpdate
                                | LiveCatchupGate::NoGap
                                    if self.gap_repair.live_edge_batches_processed > 0 =>
                                {
                                    select_gap_repair_candidate(
                                        trigger,
                                        &projected_gaps,
                                        viewport_range,
                                        &self.viewport_observation.visible_gap_ids,
                                        inspection.gaps.len(),
                                        true,
                                    )
                                }
                                LiveCatchupGate::AwaitingCheckpoint
                                | LiveCatchupGate::Stale
                                | LiveCatchupGate::NoTimelineUpdate
                                | LiveCatchupGate::NoGap
                                | LiveCatchupGate::InspectCommittedLiveEdge => {
                                    GapRepairSelection::None
                                }
                            }
                        } else {
                            select_gap_repair_candidate(
                                trigger,
                                &projected_gaps,
                                viewport_range,
                                &self.viewport_observation.visible_gap_ids,
                                inspection.gaps.len(),
                                self.gap_repair.has_live_edge_target(),
                            )
                        };
                        let unlocated_action = unlocated_gap_action(
                            self.foreground_gap_demand_active,
                            trigger,
                            inspection.gaps.len(),
                            projected_gaps.len(),
                        );
                        if let UnlocatedGapAction::RepairNewest { ordinal } = unlocated_action {
                            selection = GapRepairSelection::Unprojected {
                                ordinal,
                                reason: UnprojectedGapReason::Foreground,
                            };
                            record_timeline_gap_repair(
                                "selection",
                                timeline_gap_repair_trigger_token(trigger),
                                serial,
                                known_gap_count,
                                self.gap_repair.batches_processed,
                                "foreground_unlocated_repair",
                            );
                        }
                        let projected_candidate = select_projected_gap_candidate(
                            &projected_gaps,
                            viewport_range,
                            &self.viewport_observation.visible_gap_ids,
                        );
                        let selection_decision = gap_selection_diagnostic_decision(
                            selection,
                            projected_candidate,
                            self.foreground_gap_demand_active,
                            inspection.gaps.len(),
                            projected_gaps.len(),
                        );
                        let selected_projected_gap_id = match selection {
                            GapRepairSelection::Projected { id } => Some(id),
                            GapRepairSelection::None
                            | GapRepairSelection::DirectCommittedResponse
                            | GapRepairSelection::Unprojected { .. } => None,
                        };
                        let (ordinal, outcome, repaired_live_edge_fallback) = match selection {
                            GapRepairSelection::None => {
                                self.record_gap_selection_diagnostic(
                                    trigger,
                                    selection_decision,
                                    false,
                                    inspection.gaps.len(),
                                    projected_gaps.len(),
                                );
                                if let Some(checkpoint) = committed_response.as_ref() {
                                    let retry_key =
                                        (checkpoint.generation(), checkpoint.response_sequence());
                                    match missing_committed_gap_decision(
                                        checkpoint.has_inserted_gap(),
                                        self.missing_committed_response_retry,
                                        retry_key,
                                    ) {
                                        MissingCommittedGapDecision::Retry => {
                                            self.missing_committed_response_retry = Some(retry_key);
                                            self.gap_repair.queue_inspection(
                                                TimelineGapRepairTrigger::LiveEdge,
                                            );
                                            self.start_pending_timeline_gap_inspection().await;
                                            return;
                                        }
                                        MissingCommittedGapDecision::CloseStale => {
                                            self.missing_committed_response_retry = None;
                                            if consume_room_subscription_checkpoint(
                                                &mut self.room_subscription_checkpoint,
                                                &mut self.deferred_room_subscription_checkpoint,
                                                checkpoint,
                                            ) {
                                                self.gap_repair.queue_inspection(
                                                    TimelineGapRepairTrigger::LiveEdge,
                                                );
                                            }
                                        }
                                        MissingCommittedGapDecision::Noop => {}
                                    }
                                }
                                record_timeline_gap_repair(
                                    "inspection",
                                    timeline_gap_repair_trigger_token(trigger),
                                    serial,
                                    known_gap_count,
                                    self.gap_repair.batches_processed,
                                    "offscreen",
                                );
                                if matches!(unlocated_action, UnlocatedGapAction::QueueAutomatic) {
                                    self.gap_repair
                                        .queue_inspection(TimelineGapRepairTrigger::Automatic);
                                }
                                self.start_pending_timeline_gap_inspection().await;
                                self.emit_gap_repair_released_if_idle(serial);
                                return;
                            }
                            GapRepairSelection::Projected { id } => {
                                (usize::try_from(id.ordinal).ok(), "projected", false)
                            }
                            GapRepairSelection::DirectCommittedResponse => {
                                self.missing_committed_response_retry = None;
                                if let Some(checkpoint) = committed_response.as_ref() {
                                    if consume_room_subscription_checkpoint(
                                        &mut self.room_subscription_checkpoint,
                                        &mut self.deferred_room_subscription_checkpoint,
                                        checkpoint,
                                    ) {
                                        self.gap_repair
                                            .queue_inspection(TimelineGapRepairTrigger::LiveEdge);
                                    }
                                }
                                (None, "committed_response", true)
                            }
                            GapRepairSelection::Unprojected { ordinal, reason } => match reason {
                                UnprojectedGapReason::LiveEdge => {
                                    (Some(ordinal), "live_edge_fallback", true)
                                }
                                UnprojectedGapReason::Foreground | UnprojectedGapReason::Manual => {
                                    (Some(ordinal), "manual_fallback", false)
                                }
                            },
                        };
                        record_timeline_gap_repair(
                            "selection",
                            timeline_gap_repair_trigger_token(trigger),
                            serial,
                            known_gap_count,
                            self.gap_repair.batches_processed,
                            outcome,
                        );
                        let descriptor = if let Some(descriptor) = committed_descriptor.take() {
                            descriptor
                        } else {
                            let projected_descriptor = selected_projected_gap_id.and_then(|id| {
                                inspection
                                    .gaps
                                    .iter()
                                    .enumerate()
                                    .find(|(ordinal, descriptor)| {
                                        projected_gap_identity_matches_descriptor(
                                            id,
                                            *ordinal,
                                            descriptor.topology_revision(),
                                        )
                                    })
                                    .map(|(_, descriptor)| descriptor)
                            });
                            let fallback_descriptor = selected_projected_gap_id
                                .is_none()
                                .then(|| ordinal.and_then(|ordinal| inspection.gaps.get(ordinal)))
                                .flatten();
                            let Some(descriptor) =
                                projected_descriptor.or(fallback_descriptor).cloned()
                            else {
                                self.record_gap_selection_diagnostic(
                                    trigger,
                                    selection_decision,
                                    false,
                                    inspection.gaps.len(),
                                    projected_gaps.len(),
                                );
                                self.start_pending_timeline_gap_inspection().await;
                                self.emit_gap_repair_released_if_idle(serial);
                                return;
                            };
                            descriptor
                        };
                        let selected_gap_id = selected_projected_gap_id.or_else(|| {
                            let ordinal = ordinal.or_else(|| {
                                committed_response.as_ref().and_then(|checkpoint| {
                                    inspection
                                        .gaps
                                        .iter()
                                        .position(|gap| checkpoint.matches_gap(gap))
                                })
                            })?;
                            Some(projected_gap_id(descriptor.topology_revision(), ordinal))
                        });
                        if let Some(id) = selected_gap_id {
                            let demand_revision = self.gap_repair.demand_revision;
                            admit_and_record_timeline_gap_repair_attempt(
                                &mut self.gap_repair,
                                id,
                                demand_revision,
                            );
                        }
                        if matches!(trigger, TimelineGapRepairTrigger::LiveEdge) {
                            if !self.gap_repair.can_start_batch(trigger) {
                                self.record_gap_selection_diagnostic(
                                    trigger,
                                    selection_decision,
                                    false,
                                    inspection.gaps.len(),
                                    projected_gaps.len(),
                                );
                                record_timeline_gap_repair(
                                    "selection",
                                    timeline_gap_repair_trigger_token(trigger),
                                    serial,
                                    known_gap_count,
                                    self.gap_repair.batches_processed,
                                    "budget_exhausted",
                                );
                                self.start_pending_timeline_gap_inspection().await;
                                self.emit_gap_repair_released_if_idle(serial);
                                return;
                            }
                            let fingerprint = LiveEdgeGapSelection {
                                topology_revision: descriptor.topology_revision(),
                                ordinal: ordinal.unwrap_or(usize::MAX),
                            };
                            if matches!(
                                self.gap_repair.evaluate_live_edge_selection(fingerprint),
                                LiveEdgeSelectionDecision::NoProgress
                            ) {
                                self.record_gap_selection_diagnostic(
                                    trigger,
                                    selection_decision,
                                    false,
                                    inspection.gaps.len(),
                                    projected_gaps.len(),
                                );
                                record_timeline_gap_repair(
                                    "selection",
                                    timeline_gap_repair_trigger_token(trigger),
                                    serial,
                                    known_gap_count,
                                    self.gap_repair.batches_processed,
                                    "no_progress",
                                );
                                self.start_pending_timeline_gap_inspection().await;
                                self.emit_gap_repair_released_if_idle(serial);
                                return;
                            }
                        }
                        self.record_gap_selection_diagnostic(
                            trigger,
                            selection_decision,
                            true,
                            inspection.gaps.len(),
                            projected_gaps.len(),
                        );
                        self.start_timeline_gap_repair(
                            trigger,
                            repaired_live_edge_fallback,
                            descriptor,
                            known_gap_count,
                        )
                        .await;
                    }
                    MatrixTimelineContinuity::Unknown | MatrixTimelineContinuity::Complete => {
                        self.gap_repair.gap_count = 0;
                        self.gap_repair.live_edge_batches_processed = 0;
                        self.gap_repair.last_live_edge_selection = None;
                    }
                }
            }
            Err(_) => {
                record_timeline_gap_repair(
                    "inspection",
                    timeline_gap_repair_trigger_token(trigger),
                    serial,
                    self.gap_repair.gap_count,
                    self.gap_repair.batches_processed,
                    "failed",
                );
                let known_gap_count = self.gap_repair.gap_count;
                if known_gap_count == 0 {
                    let _ = self
                        .emit_action_reliable(AppAction::TimelineContinuityInspected {
                            room_id,
                            generation: serial,
                            inspection: TimelineContinuityInspection::Unknown,
                        })
                        .await;
                } else {
                    let repair_serial = self
                        .gap_repair
                        .begin_repair(known_gap_count)
                        .expect("completed inspection leaves scheduler idle");
                    let _ = self
                        .emit_action_reliable(AppAction::TimelineGapRepairStarted {
                            room_id: room_id.clone(),
                            generation: repair_serial,
                            gap_count: known_gap_count,
                        })
                        .await;
                    self.gap_repair.finish_work(repair_serial);
                    let _ = self
                        .emit_action_reliable(AppAction::TimelineGapRepairFailed {
                            room_id,
                            generation: repair_serial,
                            gap_count: known_gap_count,
                            batches_processed: self.gap_repair.batches_processed,
                            kind: TimelineGapRepairFailureKind::Sdk,
                        })
                        .await;
                }
            }
        }
        self.start_pending_timeline_gap_inspection().await;
        self.emit_gap_repair_released_if_idle(serial);
    }
    fn emit_gap_positions(
        &self,
        generation: u64,
        gaps: &[MatrixTimelineGapHandle],
    ) -> Vec<(usize, TimelineGapPosition)> {
        let boundary_presence = gaps
            .iter()
            .map(|gap| {
                let newer_present = gap
                    .newer_boundary_event_id()
                    .is_some_and(|event_id| self.timeline_event_position(event_id).is_some());
                let older_present = gap
                    .older_boundary_event_id()
                    .is_some_and(|event_id| self.timeline_event_position(event_id).is_some());
                (newer_present, older_present)
            })
            .collect::<Vec<_>>();
        let boundary_counts = summarize_gap_boundary_presence(boundary_presence.iter().copied());
        let projected = gaps
            .iter()
            .enumerate()
            .filter_map(|(ordinal, gap)| {
                let newer_position = gap
                    .newer_boundary_event_id()
                    .and_then(|event_id| self.timeline_event_position(event_id));
                let older_position = gap
                    .older_boundary_event_id()
                    .and_then(|event_id| self.timeline_event_position(event_id));
                projected_gap_insertion_index(newer_position, older_position).map(
                    |before_item_index| {
                        (
                            ordinal,
                            TimelineGapPosition {
                                id: projected_gap_id(gap.topology_revision(), ordinal),
                                before_item_index,
                            },
                        )
                    },
                )
            })
            .collect::<Vec<_>>();
        debug_assert_eq!(boundary_counts.projected, projected.len());
        if !gaps.is_empty() {
            let navigation_event_count = self
                .navigation_items
                .iter()
                .filter(|item| matches!(&item.id, TimelineItemId::Event { .. }))
                .count();
            record_timeline_gap_projection(
                gaps.len(),
                boundary_counts,
                navigation_event_count,
                self.foreground_gap_demand_active,
                self.gap_repair.demand_revision,
                self.gap_repair_scheduler_phase(),
            );
        }
        let positions = projected.iter().map(|(_, position)| *position).collect();
        self.emit(CoreEvent::Timeline(TimelineEvent::GapPositionsUpdated {
            key: self.key.clone(),
            actor_generation: self.actor_generation,
            generation,
            positions,
        }));
        projected
    }
    fn timeline_event_position(&self, event_id: &str) -> Option<usize> {
        self.navigation_items.iter().position(|item| {
            matches!(&item.id, TimelineItemId::Event { event_id: candidate } if candidate == event_id)
        })
    }
    async fn start_timeline_gap_repair(
        &mut self,
        trigger: TimelineGapRepairTrigger,
        repaired_live_edge_fallback: bool,
        descriptor: MatrixTimelineGapHandle,
        gap_count: u32,
    ) {
        let Some(serial) = self.gap_repair.begin_repair(gap_count) else {
            return;
        };
        let room_id = self.key.room_id().to_owned();
        if !self
            .emit_action_reliable(AppAction::TimelineGapRepairStarted {
                room_id: room_id.clone(),
                generation: serial,
                gap_count,
            })
            .await
        {
            self.gap_repair.finish_work(serial);
            return;
        }
        if self.gap_repair.record_batch(trigger).is_none() {
            self.gap_repair.finish_work(serial);
            let _ = self
                .emit_action_reliable(AppAction::TimelineGapRepairFailed {
                    room_id,
                    generation: serial,
                    gap_count,
                    batches_processed: self.gap_repair.batches_processed,
                    kind: TimelineGapRepairFailureKind::Timeout,
                })
                .await;
            return;
        }
        let session = self.session.clone();
        let timeline = self.timeline.clone();
        let actor_tx = self.msg_tx.clone();
        let work_kind = gap_repair_work_kind(trigger, self.gap_repair.last_projected_candidate);
        let account_work = self.account_work.clone();
        let budget = timeline_gap_repair_budget(trigger, work_kind);
        let actor_generation = self.actor_generation;
        let timeline_generation = self.generation;
        let projection_operation = historical_causal_projection_operation(serial);
        self.gap_projection_correlation
            .begin(actor_generation, projection_operation);
        #[cfg(test)]
        let completion_pause = self.test_gap_repair_completion_pause.take();
        self.gap_work_task = Some(executor::spawn(async move {
            // One bounded batch per permit: the slot is released before local
            // projection settlement so a send or visible pagination does not
            // wait for it, and the next batch re-enters scheduling.
            let mut result = {
                let permit = account_work.acquire(work_kind).await;
                let outcome = session
                    .repair_room_timeline_gap(
                        &descriptor,
                        budget,
                        actor_generation,
                        projection_operation.encode_transport(),
                    )
                    .await;
                permit.record_yield(1, gap_repair_batch_events(&outcome));
                outcome
            };
            if let Some(projection_batch) = result
                .as_ref()
                .ok()
                .and_then(|result| result.last_projection_batch)
            {
                let settlement = wait_for_gap_repair_projection_with_timeout(
                    TIMELINE_GAP_OBSERVABLE_SETTLEMENT_TIMEOUT,
                    timeline.wait_for_gap_repair_projection(
                        CausalProjectionId {
                            actor_generation,
                            operation: projection_operation,
                            projection_batch,
                        }
                        .encode_transport(),
                    ),
                )
                .await;
                let settlement_outcome = match settlement {
                    TimelineGapObservableSettlement::Observable => "observable",
                    TimelineGapObservableSettlement::NoProjection => "no_projection",
                    TimelineGapObservableSettlement::TimedOut => "timed_out",
                };
                record_timeline_gap_projection_boundary(
                    "sdk_settled",
                    settlement_outcome,
                    actor_generation,
                    timeline_generation,
                    projection_operation,
                    Some(projection_batch),
                    None,
                    Some(projection_batch),
                    0,
                );
                match settlement {
                    TimelineGapObservableSettlement::Observable => {}
                    TimelineGapObservableSettlement::NoProjection => {
                        if let Ok(result) = &mut result {
                            result.last_projection_batch = None;
                        }
                    }
                    TimelineGapObservableSettlement::TimedOut => {
                        result = Err(MatrixTimelineGapError::Sdk);
                    }
                }
            }
            #[cfg(test)]
            let forwarded = if let Some(TestGapRepairCompletionPause {
                reached,
                release,
                forwarded,
            }) = completion_pause
            {
                let _ = reached.send(());
                let _ = release.await;
                Some(forwarded)
            } else {
                None
            };
            let _completion_forwarded = actor_tx
                .send(TimelineActorMessage::TimelineGapRepairFinished {
                    serial,
                    trigger,
                    repaired_live_edge_fallback,
                    result,
                })
                .await
                .is_ok();
            #[cfg(test)]
            if let Some(forwarded) = forwarded {
                let _ = forwarded.send(_completion_forwarded);
            }
        }));
    }
    pub(super) async fn handle_timeline_gap_repair_finished(
        &mut self,
        serial: u64,
        trigger: TimelineGapRepairTrigger,
        repaired_live_edge_fallback: bool,
        result: Result<MatrixTimelineGapRepairResult, MatrixTimelineGapError>,
    ) {
        if !self.gap_repair.finish_work(serial) {
            return;
        }
        self.gap_work_task = None;
        let room_id = self.key.room_id().to_owned();
        let gap_count = self.gap_repair.gap_count;
        record_timeline_gap_repair_result(&mut self.gap_repair, serial, trigger, &result);
        let Ok(result) = result else {
            self.gap_projection_correlation.clear(
                self.actor_generation,
                historical_causal_projection_operation(serial),
            );
            self.emit_gap_repair_failure_and_resume(
                room_id,
                serial,
                gap_count,
                TimelineGapRepairFailureKind::Sdk,
            )
            .await;
            return;
        };
        let batches_processed = self.gap_repair.batches_processed;
        if result.outcome == MatrixTimelineGapRepairOutcome::Failed {
            self.gap_projection_correlation.clear(
                self.actor_generation,
                historical_causal_projection_operation(serial),
            );
            self.emit_gap_repair_failure_and_resume(
                room_id,
                serial,
                gap_count,
                TimelineGapRepairFailureKind::Sdk,
            )
            .await;
            return;
        }
        if matches!(trigger, TimelineGapRepairTrigger::LiveEdge)
            && !timeline_gap_repair_made_progress(&result.outcome)
        {
            self.gap_projection_correlation.clear(
                self.actor_generation,
                historical_causal_projection_operation(serial),
            );
            record_timeline_gap_repair(
                "repair",
                timeline_gap_repair_trigger_token(trigger),
                serial,
                gap_count,
                self.gap_repair.batches_processed,
                "no_progress",
            );
            self.emit_gap_repair_failure_and_resume(
                room_id,
                serial,
                gap_count,
                TimelineGapRepairFailureKind::UnsupportedAnchor,
            )
            .await;
            return;
        }
        let continuation_trigger =
            gap_repair_continuation_trigger(trigger, repaired_live_edge_fallback, &result.outcome);
        let operation = historical_causal_projection_operation(serial);
        let observed_projection_count = self.gap_projection_correlation.observed_batches.len();
        let completion = self.gap_projection_correlation.complete(
            self.actor_generation,
            operation,
            result.last_projection_batch,
        );
        let (completion_outcome, timeline_batch_id) = match completion {
            TimelineGapProjectionCompletion::Ready(batch_id) => ("ready", Some(batch_id)),
            TimelineGapProjectionCompletion::Pending => ("pending", None),
            TimelineGapProjectionCompletion::NoDiff => ("no_diff", None),
        };
        record_timeline_gap_projection_boundary(
            "actor_completed",
            completion_outcome,
            self.actor_generation,
            self.generation,
            operation,
            result.last_projection_batch,
            timeline_batch_id,
            result.last_projection_batch,
            observed_projection_count,
        );
        match completion {
            TimelineGapProjectionCompletion::Ready(batch_id) => {
                self.pending_gap_projection = Some(PendingTimelineGapProjection {
                    trigger: continuation_trigger,
                    repair_generation: serial,
                    gap_count,
                    batches_processed,
                });
                self.finish_pending_gap_projection(batch_id).await;
            }
            TimelineGapProjectionCompletion::Pending => {
                self.pending_gap_projection = Some(PendingTimelineGapProjection {
                    trigger: continuation_trigger,
                    repair_generation: serial,
                    gap_count,
                    batches_processed,
                });
                self.schedule_gap_relay_settlement(serial, continuation_trigger);
                record_timeline_gap_repair(
                    "awaiting_relay",
                    timeline_gap_repair_trigger_token(trigger),
                    serial,
                    gap_count,
                    batches_processed,
                    "pending",
                );
            }
            TimelineGapProjectionCompletion::NoDiff => {
                let _ = self
                    .emit_action_reliable(AppAction::TimelineGapRepairProgressed {
                        room_id,
                        generation: serial,
                        gap_count,
                        batches_processed,
                        minimum_batch_id: None,
                    })
                    .await;
                self.request_timeline_gap_inspection(continuation_trigger)
                    .await;
            }
        }
    }
    fn schedule_gap_relay_settlement(
        &mut self,
        repair_generation: u64,
        trigger: TimelineGapRepairTrigger,
    ) {
        if let Some(task) = self.gap_relay_settlement_task.take() {
            task.abort();
        }
        let actor_generation = self.actor_generation;
        let actor_tx = self.msg_tx.clone();
        self.gap_relay_settlement_task = Some(executor::spawn(async move {
            executor::sleep(TIMELINE_GAP_RELAY_SETTLEMENT_TIMEOUT).await;
            let _ = actor_tx
                .send(TimelineActorMessage::TimelineGapRelaySettlementDue {
                    actor_generation,
                    repair_generation,
                    trigger,
                })
                .await;
        }));
    }
    pub(super) async fn release_gap_relay_settlement(
        &mut self,
        actor_generation: u64,
        repair_generation: u64,
        trigger: TimelineGapRepairTrigger,
    ) {
        let gap_count = self
            .pending_gap_projection
            .as_ref()
            .map_or(self.gap_repair.gap_count, |pending| pending.gap_count);
        if !recover_obsolete_gap_settlement(
            &mut self.gap_projection_correlation,
            &mut self.pending_gap_projection,
            &mut self.gap_repair,
            actor_generation,
            repair_generation,
            trigger,
        ) {
            return;
        }
        if let Some(task) = self.gap_relay_settlement_task.take() {
            task.abort();
        }
        record_timeline_gap_repair(
            "relay_settlement_recovered",
            timeline_gap_repair_trigger_token(trigger),
            repair_generation,
            gap_count,
            self.gap_repair.batches_processed,
            "authoritative_replay",
        );
        self.emit_gap_repair_failure_and_resume(
            self.key.room_id().to_owned(),
            repair_generation,
            gap_count,
            TimelineGapRepairFailureKind::Timeout,
        )
        .await;
    }
    pub(super) async fn recover_gap_render_settlement(
        &mut self,
        fence: TimelineGapRenderFence,
        trigger: TimelineGapRepairTrigger,
    ) {
        if !self.gap_repair.recover_projection_timeout(fence, trigger) {
            return;
        }
        if let Some(task) = self.gap_render_settlement_task.take() {
            task.abort();
        }
        record_timeline_gap_repair(
            "render_settlement_recovered",
            timeline_gap_repair_trigger_token(trigger),
            fence.repair_generation,
            self.gap_repair.gap_count,
            self.gap_repair.batches_processed,
            "timeout",
        );
        self.emit_gap_repair_failure_and_resume(
            self.key.room_id().to_owned(),
            fence.repair_generation,
            self.gap_repair.gap_count,
            TimelineGapRepairFailureKind::Timeout,
        )
        .await;
    }
    async fn emit_gap_repair_failure_and_resume(
        &mut self,
        room_id: String,
        serial: u64,
        gap_count: u32,
        kind: TimelineGapRepairFailureKind,
    ) {
        let _ = self
            .emit_action_reliable(AppAction::TimelineGapRepairFailed {
                room_id,
                generation: serial,
                gap_count,
                batches_processed: self.gap_repair.batches_processed,
                kind,
            })
            .await;
        self.start_pending_timeline_gap_inspection().await;
        self.emit_gap_repair_released_if_idle(serial);
    }
    fn emit_gap_repair_released_if_idle(&self, generation: u64) {
        if self.gap_repair.active_serial.is_some()
            || self.gap_repair.pending_trigger.is_some()
            || self.gap_repair.awaiting_projection.is_some()
            || self.gap_projection_correlation.is_pending()
            || self.pending_gap_projection.is_some()
        {
            return;
        }
        self.emit(CoreEvent::Timeline(TimelineEvent::GapRepairReleased {
            key: self.key.clone(),
            actor_generation: self.actor_generation,
            generation,
        }));
    }
    pub(super) async fn finish_pending_gap_projection(&mut self, batch_id: TimelineBatchId) {
        if let Some(task) = self.gap_relay_settlement_task.take() {
            task.abort();
        }
        let Some(pending) = self.pending_gap_projection.take() else {
            return;
        };
        self.gap_repair.queue_inspection(pending.trigger);
        let fence = TimelineGapRenderFence {
            actor_generation: self.actor_generation,
            timeline_generation: self.generation,
            repair_generation: pending.repair_generation,
            minimum_batch_id: batch_id,
        };
        self.gap_repair.await_projection(fence);
        if let Some(task) = self.gap_render_settlement_task.take() {
            task.abort();
        }
        let actor_tx = self.msg_tx.clone();
        let trigger = pending.trigger;
        self.gap_render_settlement_task = Some(executor::spawn(async move {
            executor::sleep(TIMELINE_GAP_RENDER_SETTLEMENT_TIMEOUT).await;
            let _ = actor_tx
                .send(TimelineActorMessage::TimelineGapRenderSettlementDue { fence, trigger })
                .await;
        }));
        record_timeline_gap_repair(
            "awaiting_render",
            timeline_gap_repair_trigger_token(pending.trigger),
            pending.repair_generation,
            pending.gap_count,
            pending.batches_processed,
            "pending",
        );
        let _ = self
            .emit_action_reliable(AppAction::TimelineGapRepairProgressed {
                room_id: self.key.room_id().to_owned(),
                generation: pending.repair_generation,
                gap_count: pending.gap_count,
                batches_processed: pending.batches_processed,
                minimum_batch_id: Some(batch_id.0),
            })
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_source::item_body;

    use std::collections::{BTreeSet, HashMap};

    use std::sync::Arc;

    use std::time::Duration;

    use koushi_sdk::{
        MatrixClientSession, MatrixLiveTailRefreshOutcome, MatrixTimelineGapError,
        MatrixTimelineGapRepairBudget, MatrixTimelineGapRepairOutcome,
        MatrixTimelineGapRepairResult,
    };

    use koushi_state::AppAction;

    use tokio::sync::{broadcast, mpsc, oneshot};

    use crate::account_work::AccountWorkKind;
    #[cfg(test)]
    use crate::causal_projection::CAUSAL_PROJECTION_SERIAL_MAX;
    use crate::causal_projection::CausalProjectionId;
    use crate::command::TimelineCommand;
    use crate::event::{
        CoreEvent, TimelineDiff, TimelineEvent, TimelineGapId, TimelineGapPosition, TimelineItem,
        TimelineItemId, TimelineMessageActions,
    };

    #[cfg(any(test, feature = "test-hooks"))]
    use crate::ids::AccountKey;
    use crate::ids::{TimelineBatchId, TimelineGeneration, TimelineKey, TimelineKind};

    use koushi_state::SessionInfo;

    use super::super::actor::TimelineActorMessage;
    use super::super::diagnostics::{
        TimelineGapSelectionDiagnostic, record_timeline_gap_demand, record_timeline_gap_projection,
        record_timeline_gap_projection_boundary, record_timeline_gap_repair_evaluation,
        record_timeline_gap_selection,
    };
    use super::super::display_projection::apply_timeline_diffs_to_display_items;
    use super::super::manager::TimelineMessage;
    use super::super::relay::TimelineRelayBatch;
    use super::super::test_support::{fake_rid, live_tail_test_manager};
    use super::super::thread_projection::ThreadAttentionBatchProvenance;
    use super::{
        GapBoundaryPresenceCounts, GapRepairEvaluationDiagnosticSignature, GapRepairSelection,
        GapRepairViewportWakeDecision, GlobalCommitDecision, GlobalCommitFence,
        GlobalResponseCommit, LiveEdgeGapSelection, LiveEdgeSelectionDecision,
        MAX_LIVE_EDGE_GAP_REPAIR_BATCHES, MAX_TIMELINE_GAP_REPAIR_BATCHES,
        MissingCommittedGapDecision, PendingTimelineGapProjection, ProjectedGapCandidate,
        ProjectedGapRelation, TestGapRepairCompletionPause, TimelineGapAttemptResetReason,
        TimelineGapObservableSettlement, TimelineGapProjectionCompletion,
        TimelineGapProjectionCorrelation, TimelineGapRenderFence, TimelineGapRepairTracker,
        TimelineGapRepairTrigger, UnlocatedGapAction, UnprojectedGapReason,
        admit_and_record_timeline_gap_repair_attempt, evaluate_gap_repair_viewport_wake,
        gap_repair_continuation_trigger, gap_repair_work_kind, gap_selection_diagnostic_decision,
        global_commit_gap_selection, historical_causal_projection_operation,
        is_global_commit_inspection_target, live_tail_completion_requires_snapshot,
        missing_committed_gap_decision, post_diff_gap_inspection_trigger, projected_gap_id,
        projected_gap_identity_matches_descriptor, projected_gap_insertion_index,
        record_timeline_gap_repair_result, recover_obsolete_gap_settlement,
        rendered_live_edge_target, select_gap_repair_candidate, select_projected_gap_candidate,
        select_projected_gap_id, should_record_gap_repair_evaluation,
        summarize_gap_boundary_presence, timeline_gap_repair_budget,
        timeline_gap_repair_made_progress, timeline_gap_repair_result_diagnostic,
        timeline_gap_repair_trigger_token, unlocated_gap_action,
        wait_for_gap_repair_projection_with_timeout,
    };

    fn event_item(event_id: &str, body: &str) -> TimelineItem {
        TimelineItem {
            request_state: None,
            id: TimelineItemId::Event {
                event_id: event_id.to_owned(),
            },
            sender: None,
            sender_label: None,
            sender_avatar: None,
            body: Some(body.to_owned()),
            notice_i18n: None,
            message_kind: Default::default(),
            spoiler_spans: Vec::new(),
            timestamp_ms: None,
            in_reply_to_event_id: None,
            formatted: None,
            reply_quote: None,
            thread_root: None,
            thread_summary: None,
            media: None,
            link_previews: None,
            link_ranges: Vec::new(),
            reactions: Vec::new(),
            can_react: false,
            is_redacted: false,
            is_hidden: false,
            can_redact: false,
            is_edited: false,
            can_edit: false,
            actions: TimelineMessageActions::default(),
            send_state: None,
            unable_to_decrypt: None,
        }
    }
    fn projected_gap_position(
        topology_revision: u64,
        ordinal: usize,
        before_item_index: usize,
    ) -> TimelineGapPosition {
        TimelineGapPosition {
            id: projected_gap_id(topology_revision, ordinal),
            before_item_index,
        }
    }
    fn timeline_gap_repair_diagnostic_count_since(
        diagnostic_start: usize,
        stage: &str,
        demand_revision: u64,
    ) -> usize {
        koushi_diagnostics::test_support::detail_snapshot().records[diagnostic_start..]
            .iter()
            .filter(|record| {
                record.event.source == "core.timeline_gap_repair"
                    && record.event.stage == stage
                    && record.event.fields.iter().any(|field| {
                        field.key == "demand_revision"
                            && field.value
                                == koushi_diagnostics::DiagnosticValue::Count(demand_revision)
                    })
            })
            .count()
    }

    #[test]
    fn global_commit_fence_admits_one_omitted_room_inspection_per_new_commit() {
        let mut fence = GlobalCommitFence::default();
        let covered = GlobalResponseCommit::new(7, 10);
        let omitted = GlobalResponseCommit::new(7, 11);

        fence.note_room_checkpoint_advanced(10);
        assert_eq!(
            fence.observe(covered),
            GlobalCommitDecision::CoveredByRoomCheckpoint
        );
        assert_eq!(fence.take_pending_inspection(), None);

        assert_eq!(
            fence.observe(omitted),
            GlobalCommitDecision::InspectNewestLiveEdge
        );
        assert_eq!(fence.take_pending_inspection(), Some(omitted));
        assert_eq!(
            fence.take_pending_inspection(),
            None,
            "one global commit permits only one bounded inspection"
        );
        assert_eq!(
            fence.observe(omitted),
            GlobalCommitDecision::IgnoredStaleOrDuplicate
        );
        assert_eq!(
            fence.observe(GlobalResponseCommit::new(6, 99)),
            GlobalCommitDecision::IgnoredStaleOrDuplicate,
            "a retired core generation cannot reopen live-edge work"
        );
    }

    #[test]
    fn room_checkpoint_covers_only_its_exact_global_response() {
        let mut fence = GlobalCommitFence::default();

        fence.note_room_checkpoint_advanced(12);
        assert_eq!(
            fence.observe(GlobalResponseCommit::new(7, 11)),
            GlobalCommitDecision::InspectNewestLiveEdge,
            "an N+1 room checkpoint cannot cover an omitted room in response N",
        );
        assert_eq!(
            fence.take_pending_inspection(),
            Some(GlobalResponseCommit::new(7, 11)),
        );
        assert_eq!(
            fence.observe(GlobalResponseCommit::new(7, 12)),
            GlobalCommitDecision::CoveredByRoomCheckpoint,
        );
    }

    #[test]
    fn global_commit_selects_only_the_newest_gap_for_bounded_live_edge_repair() {
        assert_eq!(global_commit_gap_selection(0), GapRepairSelection::None);
        assert_eq!(
            global_commit_gap_selection(4),
            GapRepairSelection::Unprojected {
                ordinal: 3,
                reason: UnprojectedGapReason::LiveEdge,
            },
        );
    }

    #[test]
    fn global_commit_messages_preserve_engine_neutral_identity() {
        let commit = GlobalResponseCommit::new(7, 11);
        let manager = TimelineMessage::AllRoomsResponseCommitted {
            core_generation: commit.core_generation,
            response_sequence: commit.response_sequence,
        };
        assert!(matches!(
            manager,
            TimelineMessage::AllRoomsResponseCommitted {
                core_generation: 7,
                response_sequence: 11,
            }
        ));
        assert!(matches!(
            TimelineActorMessage::GlobalResponseCommitted(commit),
            TimelineActorMessage::GlobalResponseCommitted(GlobalResponseCommit {
                core_generation: 7,
                response_sequence: 11,
            })
        ));
    }

    #[test]
    fn global_commit_inspection_targets_only_active_room_timelines() {
        assert!(is_global_commit_inspection_target(&TimelineKind::Room {
            room_id: "!room:example.org".to_owned(),
        }));
        assert!(!is_global_commit_inspection_target(&TimelineKind::Thread {
            room_id: "!room:example.org".to_owned(),
            root_event_id: "$root:example.org".to_owned(),
        }));
        assert!(!is_global_commit_inspection_target(
            &TimelineKind::Focused {
                room_id: "!room:example.org".to_owned(),
                event_id: "$event:example.org".to_owned(),
            }
        ));
    }

    #[test]
    fn missing_committed_gap_is_reinspected_once_then_closed() {
        let retry_key = (7, 11);
        assert_eq!(
            missing_committed_gap_decision(true, None, retry_key),
            MissingCommittedGapDecision::Retry
        );
        assert_eq!(
            missing_committed_gap_decision(true, Some(retry_key), retry_key),
            MissingCommittedGapDecision::CloseStale
        );
        assert_eq!(
            missing_committed_gap_decision(false, Some(retry_key), retry_key),
            MissingCommittedGapDecision::Noop
        );
    }

    #[tokio::test]
    async fn lagged_observable_projection_wait_is_bounded() {
        assert_eq!(
            wait_for_gap_repair_projection_with_timeout(
                Duration::from_millis(1),
                std::future::pending(),
            )
            .await,
            TimelineGapObservableSettlement::TimedOut
        );
    }

    #[test]
    fn unlocated_gap_has_no_projection_position() {
        assert_eq!(projected_gap_insertion_index(None, None), None);
        assert_eq!(projected_gap_insertion_index(Some(7), None), Some(7));
        assert_eq!(projected_gap_insertion_index(None, Some(7)), Some(8));
    }

    #[test]
    fn projected_gap_identity_is_stable_only_within_the_same_topology_revision() {
        let select = |topology_revision| {
            let projected = [(1, projected_gap_position(topology_revision, 1, 18))];
            select_projected_gap_candidate(&projected, Some((15, 20)), &[])
                .expect("the projected gap intersects the viewport")
        };

        let first = select(7);
        let repeated = select(7);
        let revised = select(8);

        assert_eq!(first.id, repeated.id);
        assert_ne!(first.id, revised.id);
    }

    #[test]
    fn timeline_gap_id_wire_preserves_full_range_projected_identity() {
        let id = projected_gap_id(14_695_981_039_346_656_037, 1);

        let encoded = serde_json::to_string(&id).expect("projected gap id serializes");
        assert_eq!(
            encoded,
            r#"{"topology_revision":"14695981039346656037","ordinal":1}"#
        );
        assert_eq!(
            serde_json::from_str::<TimelineGapId>(&encoded).expect("projected gap id deserializes"),
            id
        );
    }

    #[test]
    fn projected_gap_identity_validates_revision_and_ordinal_before_descriptor_lookup() {
        let selected = projected_gap_id(7, 1);

        assert!(projected_gap_identity_matches_descriptor(selected, 1, 7));
        assert!(!projected_gap_identity_matches_descriptor(selected, 1, 8));
        assert!(!projected_gap_identity_matches_descriptor(selected, 0, 7));
    }

    #[test]
    fn gap_projection_counts_unlocated_sdk_descriptors() {
        let counts = summarize_gap_boundary_presence([
            (false, false),
            (false, false),
            (false, false),
            (false, false),
        ]);

        assert_eq!(
            counts,
            GapBoundaryPresenceCounts {
                both: 0,
                one: 0,
                none: 4,
                projected: 0,
            }
        );
    }

    #[test]
    fn foreground_unlocated_selection_is_distinguished_from_blocked_selection() {
        assert_eq!(
            gap_selection_diagnostic_decision(GapRepairSelection::None, None, true, 4, 0,),
            "foreground_unlocated"
        );
        assert_eq!(
            gap_selection_diagnostic_decision(GapRepairSelection::None, None, false, 4, 0,),
            "blocked"
        );
    }

    #[test]
    fn foreground_unlocated_gap_has_one_action_policy() {
        assert_eq!(
            unlocated_gap_action(true, TimelineGapRepairTrigger::Automatic, 2, 0),
            UnlocatedGapAction::RepairNewest { ordinal: 1 }
        );
        assert_eq!(
            unlocated_gap_action(true, TimelineGapRepairTrigger::LiveTailSnapshot, 4, 0),
            UnlocatedGapAction::QueueAutomatic
        );
        for action in [
            unlocated_gap_action(false, TimelineGapRepairTrigger::Automatic, 2, 0),
            unlocated_gap_action(false, TimelineGapRepairTrigger::LiveTailSnapshot, 4, 0),
            unlocated_gap_action(true, TimelineGapRepairTrigger::LiveTailSnapshot, 0, 0),
            unlocated_gap_action(true, TimelineGapRepairTrigger::Automatic, 4, 1),
            unlocated_gap_action(true, TimelineGapRepairTrigger::LiveTailSnapshot, 4, 1),
        ] {
            assert_eq!(action, UnlocatedGapAction::None);
        }
    }

    #[test]
    fn projected_selection_diagnostic_preserves_candidate_relation() {
        let id = projected_gap_id(7, 1);
        for (relation, expected) in [
            (ProjectedGapRelation::ExplicitVisible, "explicit_visible"),
            (ProjectedGapRelation::IntersectsViewport, "viewport"),
            (ProjectedGapRelation::NearestLiveEdge, "nearest_live_edge"),
        ] {
            assert_eq!(
                gap_selection_diagnostic_decision(
                    GapRepairSelection::Projected { id },
                    Some(ProjectedGapCandidate { id, relation }),
                    true,
                    1,
                    1,
                ),
                expected
            );
        }
    }

    #[test]
    fn unlocated_gap_diagnostics_are_private_safe() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        record_timeline_gap_projection(
            4,
            GapBoundaryPresenceCounts {
                both: 0,
                one: 0,
                none: 4,
                projected: 0,
            },
            19,
            true,
            3,
            "idle",
        );
        record_timeline_gap_demand(3, 0, 0, false, "room_selected", "idle");
        record_timeline_gap_selection(TimelineGapSelectionDiagnostic {
            trigger: "cache_gap",
            decision: "foreground_unlocated",
            repair_started: false,
            gap_count: 4,
            projected_gap_count: 0,
            visible_gap_count: 0,
            foreground_demand_active: true,
            foreground_demand_epoch: 3,
            has_live_edge_target: false,
            scheduler_phase: "idle",
        });

        let snapshot = koushi_diagnostics::test_support::detail_snapshot();
        for source in [
            "core.timeline_gap_projection",
            "core.timeline_gap_demand",
            "core.timeline_gap_selection",
        ] {
            let event = &snapshot
                .records
                .iter()
                .rev()
                .find(|record| record.event.source == source)
                .expect("new gap diagnostic")
                .event;
            let debug = format!("{event:?}");
            for forbidden in [
                "room_id",
                "event_id",
                "user_id",
                "gap_id",
                "transaction_id",
                "message",
                "ordinal",
            ] {
                assert!(!debug.contains(forbidden), "{source} leaked {forbidden}");
            }
        }
    }

    #[test]
    fn gap_projection_boundary_diagnostics_correlate_without_private_identifiers() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        record_timeline_gap_projection_boundary(
            "relay_received",
            "accepted",
            41,
            TimelineGeneration(7),
            historical_causal_projection_operation(13),
            Some(3),
            Some(TimelineBatchId(19)),
            Some(3),
            1,
        );

        let event = koushi_diagnostics::test_support::detail_snapshot()
            .records
            .into_iter()
            .rev()
            .find(|record| {
                record.event.source == "core.timeline_gap_projection"
                    && record.event.stage == "relay_received"
            })
            .expect("projection boundary diagnostic")
            .event;
        let keys = event
            .fields
            .iter()
            .map(|field| field.key)
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            vec![
                "outcome",
                "domain",
                "actor_generation",
                "timeline_generation",
                "operation_generation",
                "projection_batch",
                "timeline_batch_id",
                "expected_projection_batch",
                "observed_projection_count",
            ]
        );
        let debug = format!("{event:?}");
        for forbidden in ["room_id", "event_id", "user_id", "gap_id", "message"] {
            assert!(!debug.contains(forbidden));
        }
    }

    #[test]
    fn automatic_repair_prefers_a_gap_intersecting_the_viewport() {
        let projected = vec![
            (0, projected_gap_position(7, 0, 3)),
            (1, projected_gap_position(7, 1, 18)),
            (2, projected_gap_position(7, 2, 40)),
        ];
        assert_eq!(
            select_projected_gap_id(&projected, Some((15, 20))),
            Some(projected_gap_id(7, 1))
        );
        assert_eq!(
            select_projected_gap_id(&projected, Some((25, 30))),
            Some(projected_gap_id(7, 2))
        );
    }

    #[test]
    fn visible_gap_demand_is_preferred_over_inferred_event_bounds() {
        let projected = vec![
            (0, projected_gap_position(7, 0, 3)),
            (1, projected_gap_position(7, 1, 18)),
        ];
        let visible_gap_id = projected_gap_id(7, 0);

        assert_eq!(
            select_gap_repair_candidate(
                TimelineGapRepairTrigger::Automatic,
                &projected,
                Some((15, 20)),
                &[visible_gap_id],
                2,
                false,
            ),
            GapRepairSelection::Projected { id: visible_gap_id }
        );
    }

    #[test]
    fn visible_gap_without_event_bounds_wakes_foreground_repair() {
        let projected = vec![(0, projected_gap_position(7, 0, 3))];
        let visible_gap_id = projected_gap_id(7, 0);

        assert_eq!(
            evaluate_gap_repair_viewport_wake(&projected, None, &[visible_gap_id], None,),
            GapRepairViewportWakeDecision::Wake {
                candidate: ProjectedGapCandidate {
                    id: visible_gap_id,
                    relation: ProjectedGapRelation::ExplicitVisible,
                }
            }
        );
    }

    #[test]
    fn stale_visible_gap_is_ignored_and_requests_fresh_inspection() {
        let projected = vec![(0, projected_gap_position(7, 0, 3))];
        let stale_visible_gap_id = projected_gap_id(8, 0);

        assert_eq!(
            evaluate_gap_repair_viewport_wake(
                &projected,
                Some((1, 5)),
                &[stale_visible_gap_id],
                None,
            ),
            GapRepairViewportWakeDecision::WakeStaleVisibleDemand
        );
        assert_eq!(
            select_gap_repair_candidate(
                TimelineGapRepairTrigger::Automatic,
                &projected,
                Some((1, 5)),
                &[stale_visible_gap_id],
                1,
                false,
            ),
            GapRepairSelection::None
        );
    }

    #[test]
    fn stale_visible_gap_does_not_suppress_independent_live_edge_fallback() {
        let projected = vec![(0, projected_gap_position(7, 0, 3))];

        assert_eq!(
            select_gap_repair_candidate(
                TimelineGapRepairTrigger::LiveEdge,
                &projected,
                Some((1, 5)),
                &[projected_gap_id(8, 0)],
                2,
                true,
            ),
            GapRepairSelection::Unprojected {
                ordinal: 1,
                reason: UnprojectedGapReason::LiveEdge,
            }
        );
    }

    #[test]
    fn viewport_wake_requests_inspection_when_projected_candidate_changes() {
        let projected = vec![
            (0, projected_gap_position(7, 0, 3)),
            (1, projected_gap_position(7, 1, 18)),
        ];

        assert_eq!(
            evaluate_gap_repair_viewport_wake(&projected, Some((15, 20)), &[], None),
            GapRepairViewportWakeDecision::Wake {
                candidate: ProjectedGapCandidate {
                    id: projected_gap_id(7, 1),
                    relation: ProjectedGapRelation::IntersectsViewport,
                },
            }
        );
    }

    #[test]
    fn viewport_wake_ignores_repeated_observation_for_same_candidate() {
        let projected = vec![(0, projected_gap_position(7, 0, 8))];
        let previous = ProjectedGapCandidate {
            id: projected_gap_id(7, 0),
            relation: ProjectedGapRelation::IntersectsViewport,
        };

        assert_eq!(
            evaluate_gap_repair_viewport_wake(&projected, Some((5, 10)), &[], Some(previous)),
            GapRepairViewportWakeDecision::IdleUnchangedCandidate {
                candidate: previous,
            }
        );
    }

    #[test]
    fn viewport_wake_requests_again_when_viewport_selects_another_gap() {
        let projected = vec![
            (0, projected_gap_position(7, 0, 3)),
            (1, projected_gap_position(7, 1, 18)),
        ];
        let previous = ProjectedGapCandidate {
            id: projected_gap_id(7, 1),
            relation: ProjectedGapRelation::IntersectsViewport,
        };

        assert_eq!(
            evaluate_gap_repair_viewport_wake(&projected, Some((1, 5)), &[], Some(previous)),
            GapRepairViewportWakeDecision::Wake {
                candidate: ProjectedGapCandidate {
                    id: projected_gap_id(7, 0),
                    relation: ProjectedGapRelation::IntersectsViewport,
                },
            }
        );
    }

    #[test]
    fn viewport_wake_preserves_pending_trigger_while_render_ack_is_outstanding() {
        let projected = vec![(0, projected_gap_position(7, 0, 8))];
        let mut tracker = TimelineGapRepairTracker::default();
        tracker.await_projection(TimelineGapRenderFence {
            actor_generation: 9,
            timeline_generation: TimelineGeneration(3),
            repair_generation: 11,
            minimum_batch_id: TimelineBatchId(5),
        });

        let decision = evaluate_gap_repair_viewport_wake(&projected, Some((5, 10)), &[], None);
        assert!(matches!(
            decision,
            GapRepairViewportWakeDecision::Wake { .. }
        ));
        tracker.queue_inspection(TimelineGapRepairTrigger::Automatic);

        assert_eq!(tracker.begin_pending_inspection(true), None);
        assert!(tracker.has_pending_inspection());
    }

    #[test]
    fn observe_viewport_wakes_only_after_projected_candidate_changes() {
        let projected = vec![
            (0, projected_gap_position(7, 0, 3)),
            (1, projected_gap_position(7, 1, 18)),
        ];
        let mut tracker = TimelineGapRepairTracker::default();
        tracker.replace_projected_gaps(projected, Some((15, 20)), &[]);

        assert!(matches!(
            tracker.evaluate_viewport_wake(Some((15, 20)), &[]),
            GapRepairViewportWakeDecision::IdleUnchangedCandidate { .. }
        ));
        assert_eq!(
            tracker.evaluate_viewport_wake(Some((1, 5)), &[]),
            GapRepairViewportWakeDecision::Wake {
                candidate: ProjectedGapCandidate {
                    id: projected_gap_id(7, 0),
                    relation: ProjectedGapRelation::IntersectsViewport,
                }
            }
        );
    }

    #[test]
    fn viewport_wake_evaluation_diagnostics_are_private_safe() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        record_timeline_gap_repair_evaluation("wake", 2, 1, true, true, "awaiting_render_ack");

        let record = koushi_diagnostics::test_support::detail_snapshot()
            .records
            .into_iter()
            .rev()
            .find(|record| {
                record.event.source == "core.timeline_gap_repair"
                    && record.event.stage == "evaluation"
            })
            .expect("viewport wake evaluation diagnostic");
        let keys = record
            .event
            .fields
            .iter()
            .map(|field| field.key)
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            vec![
                "trigger",
                "decision",
                "projected_gap_count",
                "visible_gap_count",
                "visible_gap_validated",
                "candidate_changed",
                "scheduler_phase",
            ]
        );
        let debug = format!("{:?}", record.event);
        for forbidden in ["room_id", "event_id", "user_id", "gap_id", "message"] {
            assert!(!debug.contains(forbidden));
        }
    }

    #[test]
    fn gap_repair_wake_is_retained_across_ack_and_inspection_order() {
        let projected = vec![
            (0, projected_gap_position(7, 0, 3)),
            (1, projected_gap_position(7, 1, 18)),
        ];
        let mut tracker = TimelineGapRepairTracker::default();
        tracker.replace_projected_gaps(projected, Some((15, 20)), &[]);

        assert!(matches!(
            tracker.evaluate_viewport_wake(Some((1, 5)), &[]),
            GapRepairViewportWakeDecision::Wake { .. }
        ));
        tracker.queue_inspection(TimelineGapRepairTrigger::Automatic);
        assert_eq!(tracker.begin_pending_inspection(false), None);
        let (first_serial, _) = tracker
            .begin_pending_inspection(true)
            .expect("projection ACK releases the queued viewport wake");

        assert!(matches!(
            tracker.evaluate_viewport_wake(Some((15, 20)), &[]),
            GapRepairViewportWakeDecision::Wake { .. }
        ));
        tracker.queue_inspection(TimelineGapRepairTrigger::Automatic);
        assert_eq!(tracker.begin_pending_inspection(true), None);
        assert!(tracker.finish_work(first_serial));
        let (second_serial, _) = tracker
            .begin_pending_inspection(true)
            .expect("active inspection completion releases the changed candidate");
        assert!(tracker.finish_work(second_serial));

        let fence = TimelineGapRenderFence {
            actor_generation: 9,
            timeline_generation: TimelineGeneration(3),
            repair_generation: 11,
            minimum_batch_id: TimelineBatchId(5),
        };
        tracker.await_projection(fence);
        assert!(matches!(
            tracker.evaluate_viewport_wake(Some((1, 5)), &[]),
            GapRepairViewportWakeDecision::Wake { .. }
        ));
        tracker.queue_inspection(TimelineGapRepairTrigger::Automatic);
        assert_eq!(tracker.begin_pending_inspection(true), None);
        assert!(tracker.acknowledge_projection(fence));
        assert!(tracker.begin_pending_inspection(true).is_some());

        assert!(matches!(
            tracker.evaluate_viewport_wake(Some((1, 5)), &[]),
            GapRepairViewportWakeDecision::IdleUnchangedCandidate { .. }
        ));
        assert_eq!(
            timeline_gap_repair_budget(
                TimelineGapRepairTrigger::Automatic,
                AccountWorkKind::OffscreenGapRepair
            )
            .cached_chunk_limit,
            1
        );
    }

    #[test]
    fn terminal_gap_repair_failures_resume_queued_candidate_inspection() {
        let source = include_str!("gap_repair.rs");
        let handler = item_body(source, "async fn handle_timeline_gap_repair_finished");
        let helper = item_body(source, "async fn emit_gap_repair_failure_and_resume");
        assert!(
            handler
                .matches("emit_gap_repair_failure_and_resume")
                .count()
                >= 3,
            "SDK failure, failed outcome, and batch exhaustion must all release queued work"
        );
        assert!(
            helper.contains("start_pending_timeline_gap_inspection().await"),
            "the common terminal helper must restart a candidate-change inspection queued during repair"
        );
        let resume_offset = helper
            .find("start_pending_timeline_gap_inspection().await")
            .expect("terminal repair failure must release queued scheduler work");
        let wake_offset = helper
            .find("emit_gap_repair_released_if_idle")
            .expect("an idle scheduler must wake a UI request rejected during gap repair");
        assert!(
            resume_offset < wake_offset,
            "the retry wake must be emitted only after queued scheduler work has had a chance to restart"
        );
    }

    #[test]
    fn terminal_gap_inspection_paths_resume_queued_work_before_release_wake() {
        let handler = item_body(
            include_str!("gap_repair.rs"),
            "async fn handle_timeline_gap_inspection_finished",
        );
        let resume_offset = handler
            .rfind("start_pending_timeline_gap_inspection().await")
            .expect("inspection completion must restart candidate work queued during inspection");
        let wake_offset = handler
            .rfind("emit_gap_repair_released_if_idle")
            .expect("inspection completion must wake UI retries when the scheduler becomes idle");
        assert!(
            resume_offset < wake_offset,
            "inspection completion must restart queued work before deciding that the scheduler is released"
        );
    }

    #[test]
    fn candidate_wake_queued_during_repair_is_available_after_terminal_release() {
        let projected = vec![
            (0, projected_gap_position(7, 0, 3)),
            (1, projected_gap_position(7, 1, 18)),
        ];
        let mut tracker = TimelineGapRepairTracker::default();
        tracker.replace_projected_gaps(projected, Some((1, 5)), &[]);
        let repair_serial = tracker
            .begin_repair(2)
            .expect("the initial repair should own the scheduler");

        assert!(matches!(
            tracker.evaluate_viewport_wake(Some((15, 20)), &[]),
            GapRepairViewportWakeDecision::Wake { .. }
        ));
        tracker.queue_inspection(TimelineGapRepairTrigger::Automatic);
        assert_eq!(tracker.begin_pending_inspection(true), None);

        assert!(tracker.finish_work(repair_serial));
        assert!(tracker.begin_pending_inspection(true).is_some());
    }

    #[test]
    fn repeated_gap_repair_evaluation_signature_is_deduplicated() {
        let signature = GapRepairEvaluationDiagnosticSignature {
            decision: "idle_unchanged",
            projected_gap_count: 2,
            visible_gap_count: 1,
            visible_gap_validated: true,
            candidate_changed: false,
            scheduler_phase: "idle",
        };
        let mut previous = None;

        assert!(should_record_gap_repair_evaluation(
            &mut previous,
            signature
        ));
        assert!(!should_record_gap_repair_evaluation(
            &mut previous,
            signature
        ));
        assert!(should_record_gap_repair_evaluation(
            &mut previous,
            GapRepairEvaluationDiagnosticSignature {
                scheduler_phase: "active",
                ..signature
            }
        ));
    }

    #[test]
    fn automatic_and_manual_repair_use_separate_cache_budgets() {
        // The event bound comes from the work policy; only the cache budget
        // varies by trigger.
        for (trigger, work_kind) in [
            (
                TimelineGapRepairTrigger::Automatic,
                AccountWorkKind::OffscreenGapRepair,
            ),
            (
                TimelineGapRepairTrigger::LiveEdge,
                AccountWorkKind::OffscreenGapRepair,
            ),
            (
                TimelineGapRepairTrigger::Manual,
                AccountWorkKind::VisibleGapRepair,
            ),
        ] {
            assert_eq!(
                timeline_gap_repair_budget(trigger, work_kind),
                MatrixTimelineGapRepairBudget {
                    event_limit: work_kind.policy().batch_limit,
                    cached_chunk_limit: 1,
                }
            );
        }
        assert_eq!(
            timeline_gap_repair_budget(
                TimelineGapRepairTrigger::LiveTailSnapshot,
                AccountWorkKind::OffscreenGapRepair
            )
            .cached_chunk_limit,
            0,
            "live-tail snapshots must not load cached chunks"
        );
    }

    #[test]
    fn gap_repair_takes_a_scheduler_permit_around_one_bounded_batch() {
        let source = include_str!("gap_repair.rs");
        // Split on an assembled literal so this test's own source cannot match
        // the anchor ahead of the production function.
        let repair_source = source
            .split(concat!("async fn start", "_timeline", "_gap", "_repair"))
            .nth(1)
            .and_then(|section| {
                section
                    .split(concat!(
                        "async fn handle",
                        "_timeline",
                        "_gap",
                        "_repair",
                        "_finished"
                    ))
                    .next()
            })
            .expect("gap repair starter should exist");
        let acquire_offset = repair_source
            .find("account_work.acquire(work_kind)")
            .expect("gap repair must take a permit for its work kind");
        let repair_offset = repair_source
            .find("repair_room_timeline_gap(")
            .expect("gap repair must still call the SDK repair");
        let yield_offset = repair_source
            .find("permit.record_yield(1,")
            .expect("gap repair must report the bounded batch it yielded after");
        assert!(
            acquire_offset < repair_offset && repair_offset < yield_offset,
            "gap repair must acquire, run one bounded batch, then yield"
        );
        let settlement_offset = repair_source
            .find("wait_for_gap_repair_projection_with_timeout")
            .expect("gap repair must still wait for projection settlement");
        assert!(
            yield_offset < settlement_offset,
            "the permit must be released before local projection settlement"
        );
    }

    #[test]
    fn gap_repair_work_kind_follows_reported_visibility() {
        use super::{ProjectedGapCandidate, ProjectedGapRelation};
        let gap_id = TimelineGapId {
            topology_revision: 1,
            ordinal: 0,
        };
        for relation in [
            ProjectedGapRelation::ExplicitVisible,
            ProjectedGapRelation::IntersectsViewport,
        ] {
            assert_eq!(
                gap_repair_work_kind(
                    TimelineGapRepairTrigger::Automatic,
                    Some(ProjectedGapCandidate {
                        id: gap_id,
                        relation
                    })
                ),
                AccountWorkKind::VisibleGapRepair
            );
        }
        assert_eq!(
            gap_repair_work_kind(
                TimelineGapRepairTrigger::Automatic,
                Some(ProjectedGapCandidate {
                    id: gap_id,
                    relation: ProjectedGapRelation::NearestLiveEdge
                })
            ),
            AccountWorkKind::OffscreenGapRepair
        );
        assert_eq!(
            gap_repair_work_kind(TimelineGapRepairTrigger::LiveEdge, None),
            AccountWorkKind::OffscreenGapRepair,
            "live-edge repair for the selected room stays background"
        );
        assert_eq!(
            gap_repair_work_kind(TimelineGapRepairTrigger::Manual, None),
            AccountWorkKind::VisibleGapRepair,
            "an explicitly requested repair is foreground even without a candidate"
        );
        // Background repair must never outrank a send or visible pagination.
        assert!(
            AccountWorkKind::OffscreenGapRepair.policy().priority
                > AccountWorkKind::ExplicitPagination.policy().priority
        );
        assert!(
            AccountWorkKind::VisibleGapRepair.policy().priority
                > AccountWorkKind::MessageSend.policy().priority
        );
    }

    #[test]
    fn trigger_priority_keeps_live_edge_between_viewport_and_manual() {
        let mut tracker = TimelineGapRepairTracker::default();
        tracker.queue_inspection(TimelineGapRepairTrigger::Automatic);
        tracker.queue_inspection(TimelineGapRepairTrigger::LiveEdge);
        assert!(matches!(
            tracker.begin_pending_inspection(true),
            Some((_, TimelineGapRepairTrigger::LiveEdge))
        ));

        let mut tracker = TimelineGapRepairTracker::default();
        tracker.queue_inspection(TimelineGapRepairTrigger::LiveEdge);
        tracker.queue_inspection(TimelineGapRepairTrigger::Manual);
        assert!(matches!(
            tracker.begin_pending_inspection(true),
            Some((_, TimelineGapRepairTrigger::Manual))
        ));

        let mut tracker = TimelineGapRepairTracker::default();
        tracker.queue_inspection(TimelineGapRepairTrigger::LiveEdge);
        tracker.queue_inspection(TimelineGapRepairTrigger::LiveTailSnapshot);
        assert!(matches!(
            tracker.begin_pending_inspection(true),
            Some((_, TimelineGapRepairTrigger::LiveTailSnapshot))
        ));

        let mut tracker = TimelineGapRepairTracker::default();
        tracker.queue_inspection(TimelineGapRepairTrigger::LiveTailSnapshot);
        tracker.queue_inspection(TimelineGapRepairTrigger::Manual);
        assert!(matches!(
            tracker.begin_pending_inspection(true),
            Some((_, TimelineGapRepairTrigger::Manual))
        ));
    }

    #[test]
    fn live_tail_snapshot_observes_projected_gaps_without_repairing_them() {
        let projected = vec![(0, projected_gap_position(7, 0, 0))];
        assert_eq!(
            select_gap_repair_candidate(
                TimelineGapRepairTrigger::LiveTailSnapshot,
                &projected,
                Some((0, 0)),
                &[],
                1,
                true,
            ),
            GapRepairSelection::None,
        );
    }

    #[test]
    fn final_live_tail_projection_batch_queues_one_snapshot_instead_of_live_edge() {
        assert_eq!(
            post_diff_gap_inspection_trigger(true, true, true),
            Some(TimelineGapRepairTrigger::LiveTailSnapshot),
            "the exact final live-tail batch must publish one observation instead of leaving a repair-capable LiveEdge request behind"
        );
        assert_eq!(
            post_diff_gap_inspection_trigger(true, false, true),
            None,
            "an intermediate live-tail batch must not queue automatic or live-edge repair",
        );
        assert_eq!(
            post_diff_gap_inspection_trigger(
                true,
                live_tail_completion_requires_snapshot(MatrixLiveTailRefreshOutcome::Failed),
                true,
            ),
            None,
            "a failed live-tail completion must not create an observation snapshot",
        );
        assert_eq!(
            post_diff_gap_inspection_trigger(false, false, true),
            Some(TimelineGapRepairTrigger::LiveEdge),
        );
        assert_eq!(
            post_diff_gap_inspection_trigger(false, false, false),
            Some(TimelineGapRepairTrigger::Automatic),
        );
    }

    #[test]
    fn live_edge_fallback_selects_only_the_newest_unprojected_gap() {
        assert_eq!(
            select_gap_repair_candidate(
                TimelineGapRepairTrigger::Automatic,
                &[],
                None,
                &[],
                4,
                true,
            ),
            GapRepairSelection::None,
        );
        assert_eq!(
            select_gap_repair_candidate(
                TimelineGapRepairTrigger::LiveEdge,
                &[],
                None,
                &[],
                4,
                false,
            ),
            GapRepairSelection::None,
        );
        assert_eq!(
            select_gap_repair_candidate(
                TimelineGapRepairTrigger::LiveEdge,
                &[],
                None,
                &[],
                4,
                true,
            ),
            GapRepairSelection::Unprojected {
                ordinal: 3,
                reason: UnprojectedGapReason::LiveEdge,
            },
        );
    }

    #[test]
    fn live_edge_target_change_rearms_a_bounded_attempt() {
        let mut tracker = TimelineGapRepairTracker::default();
        assert!(tracker.observe_live_edge_target(Some("$owner-a".to_owned())));
        assert!(!tracker.observe_live_edge_target(Some("$owner-a".to_owned())));

        for _ in 0..MAX_LIVE_EDGE_GAP_REPAIR_BATCHES {
            assert!(
                tracker
                    .record_batch(TimelineGapRepairTrigger::LiveEdge)
                    .is_some()
            );
        }
        assert!(!tracker.can_start_batch(TimelineGapRepairTrigger::LiveEdge));

        assert!(tracker.observe_live_edge_target(Some("$owner-b".to_owned())));
        assert!(tracker.can_start_batch(TimelineGapRepairTrigger::LiveEdge));
    }

    #[test]
    fn unchanged_live_edge_topology_after_a_batch_is_no_progress() {
        let mut tracker = TimelineGapRepairTracker::default();
        let selection = LiveEdgeGapSelection {
            topology_revision: 17,
            ordinal: 3,
        };

        assert_eq!(
            tracker.evaluate_live_edge_selection(selection),
            LiveEdgeSelectionDecision::Repair,
        );
        assert!(
            tracker
                .record_batch(TimelineGapRepairTrigger::LiveEdge)
                .is_some()
        );
        assert_eq!(
            tracker.evaluate_live_edge_selection(selection),
            LiveEdgeSelectionDecision::NoProgress,
        );
    }

    #[test]
    fn live_edge_zero_progress_outcomes_terminate() {
        for outcome in [
            MatrixTimelineGapRepairOutcome::Stale,
            MatrixTimelineGapRepairOutcome::Deferred {
                cached_chunks_loaded: 0,
            },
            MatrixTimelineGapRepairOutcome::Progress { events: 0 },
        ] {
            assert!(!timeline_gap_repair_made_progress(&outcome));
        }
        for outcome in [
            MatrixTimelineGapRepairOutcome::Deferred {
                cached_chunks_loaded: 1,
            },
            MatrixTimelineGapRepairOutcome::Progress { events: 1 },
            MatrixTimelineGapRepairOutcome::BoundariesJoined { events: 0 },
            MatrixTimelineGapRepairOutcome::StartReached { events: 0 },
        ] {
            assert!(timeline_gap_repair_made_progress(&outcome));
        }
    }

    #[test]
    fn gap_repair_result_diagnostics_preserve_sdk_outcome_and_progress_counts() {
        let cases = [
            (
                Ok(MatrixTimelineGapRepairResult {
                    outcome: MatrixTimelineGapRepairOutcome::Deferred {
                        cached_chunks_loaded: 3,
                    },
                    last_projection_batch: Some(2),
                }),
                ("deferred", 0, 3, true, true),
            ),
            (
                Ok(MatrixTimelineGapRepairResult {
                    outcome: MatrixTimelineGapRepairOutcome::Progress { events: 17 },
                    last_projection_batch: Some(1),
                }),
                ("progress", 17, 0, true, true),
            ),
            (
                Ok(MatrixTimelineGapRepairResult {
                    outcome: MatrixTimelineGapRepairOutcome::BoundariesJoined { events: 5 },
                    last_projection_batch: None,
                }),
                ("boundaries_joined", 5, 0, false, true),
            ),
            (
                Ok(MatrixTimelineGapRepairResult {
                    outcome: MatrixTimelineGapRepairOutcome::StartReached { events: 4 },
                    last_projection_batch: None,
                }),
                ("start_reached", 4, 0, false, true),
            ),
            (
                Ok(MatrixTimelineGapRepairResult {
                    outcome: MatrixTimelineGapRepairOutcome::Stale,
                    last_projection_batch: None,
                }),
                ("stale", 0, 0, false, false),
            ),
            (
                Ok(MatrixTimelineGapRepairResult {
                    outcome: MatrixTimelineGapRepairOutcome::Failed,
                    last_projection_batch: None,
                }),
                ("failed", 0, 0, false, false),
            ),
            (
                Err(MatrixTimelineGapError::Sdk),
                ("error", 0, 0, false, false),
            ),
        ];

        for (result, expected) in cases {
            let diagnostic = timeline_gap_repair_result_diagnostic(&result);
            assert_eq!(
                (
                    diagnostic.outcome,
                    diagnostic.events,
                    diagnostic.cached_chunks_loaded,
                    diagnostic.has_projection_batch,
                    diagnostic.made_progress,
                ),
                expected,
            );
        }
    }

    #[test]
    fn repaired_live_edge_does_not_continue_into_an_unrelated_historical_gap() {
        assert_eq!(
            gap_repair_continuation_trigger(
                TimelineGapRepairTrigger::LiveEdge,
                true,
                &MatrixTimelineGapRepairOutcome::BoundariesJoined { events: 3 },
            ),
            TimelineGapRepairTrigger::Automatic,
        );
        assert_eq!(
            gap_repair_continuation_trigger(
                TimelineGapRepairTrigger::LiveEdge,
                true,
                &MatrixTimelineGapRepairOutcome::Progress { events: 3 },
            ),
            TimelineGapRepairTrigger::LiveEdge,
        );
        assert_eq!(
            gap_repair_continuation_trigger(
                TimelineGapRepairTrigger::LiveEdge,
                false,
                &MatrixTimelineGapRepairOutcome::BoundariesJoined { events: 3 },
            ),
            TimelineGapRepairTrigger::LiveEdge,
            "repairing a projected gap must preserve the live-edge intent"
        );
    }

    #[test]
    fn actor_fixture_recovers_relation_bounded_live_edge_after_exact_render_ack() {
        // The raw newest boundary is an edit/reaction and therefore has no
        // standalone projected row. The rendered owner still supplies the
        // actor-private live-edge target.
        let actor_generation = 7;
        let timeline_generation = TimelineGeneration(3);
        let projection_batch = 1;
        let rendered_batch_id = TimelineBatchId(41);
        let older = event_item("$older:test", "older");
        let missing = event_item("$missing:test", "missing");
        let newer_owner = event_item("$owner:test", "newer");
        let mut rendered_items = vec![older.clone(), newer_owner.clone()];
        let mut tracker = TimelineGapRepairTracker::default();
        let mut correlation = TimelineGapProjectionCorrelation::default();

        assert!(tracker.observe_live_edge_target(rendered_live_edge_target(&rendered_items)));
        tracker.queue_inspection(TimelineGapRepairTrigger::LiveEdge);
        assert_eq!(
            tracker.begin_pending_inspection(false),
            None,
            "the initial projection must be acknowledged before inspection"
        );
        let (inspection_serial, trigger) = tracker
            .begin_pending_inspection(true)
            .expect("initial render ACK releases live-edge inspection");
        assert_eq!(trigger, TimelineGapRepairTrigger::LiveEdge);
        assert!(tracker.finish_work(inspection_serial));

        let projected_relation_boundaries = Vec::new();
        assert_eq!(
            select_gap_repair_candidate(
                trigger,
                &projected_relation_boundaries,
                None,
                &[],
                3,
                tracker.has_live_edge_target(),
            ),
            GapRepairSelection::Unprojected {
                ordinal: 2,
                reason: UnprojectedGapReason::LiveEdge,
            },
        );

        let repair_serial = tracker.begin_repair(3).expect("repair owns scheduler");
        let repair_operation = historical_causal_projection_operation(repair_serial);
        correlation.begin(actor_generation, repair_operation);

        // Model the SDK relay publication carrying the repair correlation tag.
        // A duplicate delivery is included deliberately: the same display
        // normalization used by TimelineActor/WebView must retain one row.
        apply_timeline_diffs_to_display_items(
            &mut rendered_items,
            &[
                TimelineDiff::Insert {
                    index: 1,
                    item: missing.clone(),
                },
                TimelineDiff::Insert {
                    index: 1,
                    item: missing.clone(),
                },
            ],
        );
        assert_eq!(
            correlation.observe(
                CausalProjectionId {
                    actor_generation,
                    operation: repair_operation,
                    projection_batch,
                },
                rendered_batch_id,
            ),
            None,
            "publication alone cannot continue before SDK completion"
        );
        assert_eq!(
            correlation.complete(actor_generation, repair_operation, Some(projection_batch)),
            TimelineGapProjectionCompletion::Ready(rendered_batch_id),
        );
        assert!(tracker.finish_work(repair_serial));
        assert_eq!(
            tracker.record_batch(trigger),
            Some(1),
            "one bounded live-edge repair batch is recorded"
        );

        // Once that newest gap joins, reinspection uses ordinary automatic
        // policy, so the two unrelated unprojected historical gaps stay idle.
        let continuation = gap_repair_continuation_trigger(
            trigger,
            true,
            &MatrixTimelineGapRepairOutcome::BoundariesJoined { events: 1 },
        );
        assert_eq!(continuation, TimelineGapRepairTrigger::Automatic);
        tracker.queue_inspection(continuation);
        let fence = TimelineGapRenderFence {
            actor_generation,
            timeline_generation,
            repair_generation: repair_serial,
            minimum_batch_id: rendered_batch_id,
        };
        tracker.await_projection(fence);
        assert!(!tracker.acknowledge_projection(TimelineGapRenderFence {
            minimum_batch_id: TimelineBatchId(rendered_batch_id.0 - 1),
            ..fence
        }));
        assert_eq!(
            tracker.begin_pending_inspection(true),
            None,
            "an unrelated or older render ACK cannot release continuation"
        );
        assert!(tracker.acknowledge_projection(fence));
        let (continuation_serial, continuation) = tracker
            .begin_pending_inspection(true)
            .expect("the exact render ACK releases continuation");
        assert_eq!(
            select_gap_repair_candidate(
                continuation,
                &projected_relation_boundaries,
                None,
                &[],
                2,
                true,
            ),
            GapRepairSelection::None,
        );
        assert!(tracker.finish_work(continuation_serial));

        assert_eq!(rendered_items, vec![older, missing.clone(), newer_owner]);
        assert_eq!(
            rendered_items
                .iter()
                .filter(|item| item.id == missing.id)
                .count(),
            1,
            "the repaired interval is projected exactly once"
        );
    }

    #[test]
    fn live_edge_diagnostic_trigger_is_private_safe() {
        assert_eq!(
            timeline_gap_repair_trigger_token(TimelineGapRepairTrigger::LiveEdge),
            "live_edge"
        );
    }

    #[test]
    fn subscription_inspection_waits_for_initial_projection_ack() {
        let mut tracker = TimelineGapRepairTracker::default();
        tracker.queue_inspection(TimelineGapRepairTrigger::Automatic);
        assert_eq!(tracker.begin_pending_inspection(false), None);
        assert!(tracker.has_pending_inspection());
        assert!(matches!(
            tracker.begin_pending_inspection(true),
            Some((_, TimelineGapRepairTrigger::Automatic))
        ));
    }

    #[test]
    fn repair_continuation_requires_the_matching_render_fence() {
        let mut tracker = TimelineGapRepairTracker::default();
        let fence = TimelineGapRenderFence {
            actor_generation: 9,
            timeline_generation: TimelineGeneration(3),
            repair_generation: 11,
            minimum_batch_id: TimelineBatchId(5),
        };
        tracker.await_projection(fence);

        assert!(!tracker.acknowledge_projection(TimelineGapRenderFence {
            repair_generation: 10,
            ..fence
        }));
        assert!(!tracker.acknowledge_projection(TimelineGapRenderFence {
            minimum_batch_id: TimelineBatchId(4),
            ..fence
        }));
        assert!(tracker.acknowledge_projection(TimelineGapRenderFence {
            minimum_batch_id: TimelineBatchId(6),
            ..fence
        }));
    }

    #[test]
    fn render_ack_timeout_clears_fence_and_requeues_live_edge() {
        let mut tracker = TimelineGapRepairTracker::default();
        let fence = TimelineGapRenderFence {
            actor_generation: 9,
            timeline_generation: TimelineGeneration(3),
            repair_generation: 11,
            minimum_batch_id: TimelineBatchId(5),
        };
        tracker.await_projection(fence);

        assert!(tracker.recover_projection_timeout(fence, TimelineGapRepairTrigger::LiveEdge,));
        let (_, trigger) = tracker
            .begin_pending_inspection(true)
            .expect("the matching timeout must release and requeue LiveEdge");
        assert_eq!(trigger, TimelineGapRepairTrigger::LiveEdge);
        assert!(!tracker.recover_projection_timeout(fence, TimelineGapRepairTrigger::Manual,));
    }

    #[test]
    fn relay_overflow_clears_obsolete_gap_correlation_and_requeues_live_edge() {
        let actor_generation = 9;
        let mut tracker = TimelineGapRepairTracker::default();
        let repair_generation = tracker.begin_repair(1).expect("repair owns scheduler");
        let mut correlation = TimelineGapProjectionCorrelation::default();
        correlation.begin(
            actor_generation,
            historical_causal_projection_operation(repair_generation),
        );
        let mut pending = Some(PendingTimelineGapProjection {
            trigger: TimelineGapRepairTrigger::LiveEdge,
            repair_generation,
            gap_count: 1,
            batches_processed: 1,
        });

        assert!(recover_obsolete_gap_settlement(
            &mut correlation,
            &mut pending,
            &mut tracker,
            actor_generation,
            repair_generation,
            TimelineGapRepairTrigger::LiveEdge,
        ));
        assert!(!correlation.is_pending());
        assert!(pending.is_none());
        let (_, trigger) = tracker
            .begin_pending_inspection(true)
            .expect("overflow recovery must release and requeue LiveEdge");
        assert_eq!(trigger, TimelineGapRepairTrigger::LiveEdge);
    }

    #[test]
    fn stale_prior_actor_gap_projection_is_removed_from_every_relay_batch() {
        let current_actor_generation = 9;
        let stale = CausalProjectionId {
            actor_generation: current_actor_generation - 1,
            operation: historical_causal_projection_operation(11),
            projection_batch: 1,
        };
        let current = CausalProjectionId {
            actor_generation: current_actor_generation,
            operation: historical_causal_projection_operation(12),
            projection_batch: 1,
        };

        for _ in 0..3 {
            let mut batch = TimelineRelayBatch {
                generation: TimelineGeneration(4),
                diffs: Vec::new(),
                thread_attention_provenance: ThreadAttentionBatchProvenance::default(),
                gap_repair_projections: BTreeSet::from([stale, current]),
            };
            batch.retain_gap_repair_projections_for_actor(current_actor_generation);
            assert_eq!(
                batch.gap_repair_projections,
                BTreeSet::from([current]),
                "every relay batch must drop superseded descriptors and retain current identity"
            );
        }
    }

    #[test]
    fn timeline_gap_repair_tracker_coalesces_and_rejects_stale_completions() {
        let mut tracker = TimelineGapRepairTracker::default();
        let first = tracker.begin_inspection().expect("first inspection");
        assert!(tracker.begin_inspection().is_none());
        assert!(!tracker.finish_work(first.wrapping_add(1)));
        assert!(tracker.finish_work(first));

        let repair = tracker.begin_repair(2).expect("repair starts");
        assert_eq!(tracker.gap_count, 2);
        assert!(tracker.begin_repair(2).is_none());
        assert!(tracker.finish_work(repair));
    }

    #[test]
    fn historical_projection_serial_exhaustion_never_reuses_one() {
        let actor_generation = 9;
        let mut correlation = TimelineGapProjectionCorrelation::default();
        let prior_operation = historical_causal_projection_operation(CAUSAL_PROJECTION_SERIAL_MAX);
        correlation.begin(actor_generation, prior_operation);
        assert_eq!(
            correlation.complete(actor_generation, prior_operation, Some(1)),
            TimelineGapProjectionCompletion::Pending,
        );

        let mut tracker = TimelineGapRepairTracker {
            next_serial: CAUSAL_PROJECTION_SERIAL_MAX,
            ..TimelineGapRepairTracker::default()
        };
        assert_eq!(tracker.begin_repair(1), None);
        assert_eq!(tracker.begin_repair(1), None, "exhaustion cannot busy-loop");
        assert!(tracker.active_serial.is_none());
        assert_eq!(tracker.next_serial, CAUSAL_PROJECTION_SERIAL_MAX);
        assert_eq!(
            correlation.observe(
                CausalProjectionId {
                    actor_generation,
                    operation: historical_causal_projection_operation(1),
                    projection_batch: 1,
                },
                TimelineBatchId(8),
            ),
            None,
            "serial one from a hypothetical wrap cannot cross-complete the prior identity",
        );
        assert!(correlation.is_pending());
    }

    #[test]
    fn gap_repair_progress_budget_allows_cache_reveal_beyond_total_batch_count() {
        let mut tracker = TimelineGapRepairTracker::default();
        let id = projected_gap_id(7, 1);
        let demand_revision = 11;
        assert!(tracker.admit_gap_attempt(id, demand_revision).is_some());

        for expected in 1..=MAX_TIMELINE_GAP_REPAIR_BATCHES + 1 {
            let outcome = MatrixTimelineGapRepairOutcome::Deferred {
                cached_chunks_loaded: 1,
            };
            assert!(timeline_gap_repair_made_progress(&outcome));
            assert_eq!(tracker.attempt_gap_id, Some(id));
            assert_eq!(tracker.attempt_demand_revision, Some(demand_revision));
            assert!(
                tracker.can_start_batch(TimelineGapRepairTrigger::Automatic),
                "cache reveal batch {expected} must remain admissible"
            );
            assert_eq!(
                tracker.record_batch(TimelineGapRepairTrigger::Automatic),
                Some(expected)
            );
            tracker.record_batch_outcome(&outcome);
            assert_eq!(tracker.consecutive_no_progress_batches, 0);
        }
        assert_eq!(
            tracker.batches_processed,
            MAX_TIMELINE_GAP_REPAIR_BATCHES + 1
        );
    }

    #[test]
    fn gap_repair_attempt_diagnostics_classify_attempt_resets() {
        let mut tracker = TimelineGapRepairTracker::default();

        let initial = tracker
            .admit_gap_attempt(projected_gap_id(7, 1), 11)
            .expect("initial gap attempt is admitted");
        assert_eq!(initial.attempt_number, 1);
        assert_eq!(initial.reason, TimelineGapAttemptResetReason::Initial);
        assert!(!initial.topology_changed);
        assert!(!initial.ordinal_changed);
        assert!(!initial.demand_changed);
        assert_eq!(initial.reason.as_str(), "initial");

        let topology = tracker
            .admit_gap_attempt(projected_gap_id(8, 1), 11)
            .expect("topology change is admitted");
        assert_eq!(topology.attempt_number, 2);
        assert_eq!(topology.reason, TimelineGapAttemptResetReason::Topology);
        assert!(topology.topology_changed);
        assert!(!topology.ordinal_changed);
        assert!(!topology.demand_changed);
        assert_eq!(topology.reason.as_str(), "topology");

        let ordinal = tracker
            .admit_gap_attempt(projected_gap_id(8, 2), 11)
            .expect("ordinal change is admitted");
        assert_eq!(ordinal.attempt_number, 3);
        assert_eq!(ordinal.reason, TimelineGapAttemptResetReason::Ordinal);
        assert!(!ordinal.topology_changed);
        assert!(ordinal.ordinal_changed);
        assert!(!ordinal.demand_changed);
        assert_eq!(ordinal.reason.as_str(), "ordinal");

        let demand = tracker
            .admit_gap_attempt(projected_gap_id(8, 2), 12)
            .expect("explicit demand change is admitted");
        assert_eq!(demand.attempt_number, 4);
        assert_eq!(demand.reason, TimelineGapAttemptResetReason::Demand);
        assert!(!demand.topology_changed);
        assert!(!demand.ordinal_changed);
        assert!(demand.demand_changed);
        assert_eq!(demand.reason.as_str(), "demand");

        assert!(
            tracker
                .admit_gap_attempt(projected_gap_id(8, 2), 12)
                .is_none()
        );
    }

    #[test]
    fn gap_repair_attempt_diagnostics_emit_once_per_changed_admission() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
            .records
            .len();
        let demand_revision = 9_004_001;
        let mut tracker = TimelineGapRepairTracker::default();

        assert!(admit_and_record_timeline_gap_repair_attempt(
            &mut tracker,
            projected_gap_id(7, 1),
            demand_revision,
        ));
        assert!(admit_and_record_timeline_gap_repair_attempt(
            &mut tracker,
            projected_gap_id(8, 1),
            demand_revision,
        ));
        assert!(admit_and_record_timeline_gap_repair_attempt(
            &mut tracker,
            projected_gap_id(8, 2),
            demand_revision,
        ));
        assert!(admit_and_record_timeline_gap_repair_attempt(
            &mut tracker,
            projected_gap_id(8, 2),
            demand_revision + 1,
        ));
        assert!(!admit_and_record_timeline_gap_repair_attempt(
            &mut tracker,
            projected_gap_id(8, 2),
            demand_revision + 1,
        ));

        let records = koushi_diagnostics::test_support::detail_snapshot().records;
        let admissions = records[diagnostic_start..]
            .iter()
            .filter(|record| {
                record.event.source == "core.timeline_gap_repair"
                    && record.event.stage == "attempt_admitted"
                    && record.event.fields.iter().any(|field| {
                        field.key == "demand_revision"
                            && matches!(
                                field.value,
                                koushi_diagnostics::DiagnosticValue::Count(value)
                                    if value == demand_revision || value == demand_revision + 1
                            )
                    })
            })
            .collect::<Vec<_>>();
        assert_eq!(admissions.len(), 4);
        for reason in ["initial", "topology", "ordinal", "demand"] {
            assert_eq!(
                admissions
                    .iter()
                    .filter(|record| {
                        record.event.fields.iter().any(|field| {
                            field.key == "reset_reason"
                                && field.value == koushi_diagnostics::DiagnosticValue::Token(reason)
                        })
                    })
                    .count(),
                1,
                "changed admission must emit reset reason {reason} exactly once",
            );
        }
    }

    #[test]
    fn gap_repair_attempt_diagnostics_emit_one_budget_update_per_sdk_result() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
            .records
            .len();
        let demand_revision = 9_004_101;
        let mut tracker = TimelineGapRepairTracker::default();
        tracker
            .admit_gap_attempt(projected_gap_id(7, 1), demand_revision)
            .expect("initial gap attempt is admitted");
        let outcomes = [
            MatrixTimelineGapRepairOutcome::Stale,
            MatrixTimelineGapRepairOutcome::Deferred {
                cached_chunks_loaded: 0,
            },
            MatrixTimelineGapRepairOutcome::Deferred {
                cached_chunks_loaded: 1,
            },
            MatrixTimelineGapRepairOutcome::Failed,
            MatrixTimelineGapRepairOutcome::Progress { events: 0 },
            MatrixTimelineGapRepairOutcome::Progress { events: 1 },
            MatrixTimelineGapRepairOutcome::BoundariesJoined { events: 0 },
            MatrixTimelineGapRepairOutcome::StartReached { events: 0 },
        ];

        for (index, outcome) in outcomes.into_iter().enumerate() {
            record_timeline_gap_repair_result(
                &mut tracker,
                index as u64 + 1,
                TimelineGapRepairTrigger::Automatic,
                &Ok(MatrixTimelineGapRepairResult {
                    outcome,
                    last_projection_batch: None,
                }),
            );
            assert_eq!(
                timeline_gap_repair_diagnostic_count_since(
                    diagnostic_start,
                    "budget_updated",
                    demand_revision,
                ),
                index + 1,
                "each successful SDK result must emit exactly one budget update",
            );
        }

        record_timeline_gap_repair_result(
            &mut tracker,
            outcomes.len() as u64 + 1,
            TimelineGapRepairTrigger::Automatic,
            &Err(MatrixTimelineGapError::Sdk),
        );
        assert_eq!(
            timeline_gap_repair_diagnostic_count_since(
                diagnostic_start,
                "budget_updated",
                demand_revision,
            ),
            outcomes.len() + 1,
            "the SDK error result must emit exactly one budget update",
        );
    }

    #[test]
    fn gap_repair_progress_budget_rejects_thirty_third_consecutive_noop() {
        let mut tracker = TimelineGapRepairTracker::default();
        let id = projected_gap_id(7, 1);
        let demand_revision = 11;
        assert!(tracker.admit_gap_attempt(id, demand_revision).is_some());

        for expected in 1..=MAX_TIMELINE_GAP_REPAIR_BATCHES {
            let outcome = MatrixTimelineGapRepairOutcome::Deferred {
                cached_chunks_loaded: 0,
            };
            assert!(!timeline_gap_repair_made_progress(&outcome));
            assert!(tracker.can_start_batch(TimelineGapRepairTrigger::Automatic));
            assert_eq!(
                tracker.record_batch(TimelineGapRepairTrigger::Automatic),
                Some(expected)
            );
            tracker.record_batch_outcome(&outcome);
            assert_eq!(tracker.consecutive_no_progress_batches, expected);
        }
        assert!(!tracker.can_start_batch(TimelineGapRepairTrigger::Automatic));
        assert_eq!(
            tracker.record_batch(TimelineGapRepairTrigger::Automatic),
            None
        );
        assert_eq!(tracker.attempt_gap_id, Some(id));
        assert_eq!(tracker.attempt_demand_revision, Some(demand_revision));
        assert_eq!(
            tracker.consecutive_no_progress_batches,
            MAX_TIMELINE_GAP_REPAIR_BATCHES
        );
    }

    #[test]
    fn gap_repair_sdk_error_budget_rejects_thirty_third_consecutive_error() {
        let mut tracker = TimelineGapRepairTracker::default();
        let id = projected_gap_id(7, 1);
        let demand_revision = 11;
        assert!(tracker.admit_gap_attempt(id, demand_revision).is_some());

        for expected in 1..=MAX_TIMELINE_GAP_REPAIR_BATCHES {
            assert_eq!(
                tracker.record_batch(TimelineGapRepairTrigger::Automatic),
                Some(expected)
            );
            tracker.record_batch_error();
            assert_eq!(tracker.consecutive_no_progress_batches, expected);
        }
        assert_eq!(
            tracker.record_batch(TimelineGapRepairTrigger::Automatic),
            None
        );
        assert_eq!(tracker.attempt_gap_id, Some(id));
        assert_eq!(tracker.attempt_demand_revision, Some(demand_revision));

        assert!(
            tracker
                .admit_gap_attempt(projected_gap_id(8, 1), demand_revision)
                .is_some()
        );
        tracker.batches_processed = u32::MAX;
        assert_eq!(
            tracker.record_batch(TimelineGapRepairTrigger::Automatic),
            Some(u32::MAX)
        );
        assert_eq!(tracker.batches_processed, u32::MAX);
    }

    #[test]
    fn gap_repair_budget_is_scoped_without_resetting_repeated_demand() {
        let mut tracker = TimelineGapRepairTracker::default();
        let id = projected_gap_id(7, 1);

        assert!(tracker.admit_gap_attempt(id, 11).is_some());
        assert_eq!(
            tracker.record_batch(TimelineGapRepairTrigger::Automatic),
            Some(1)
        );

        assert!(tracker.admit_gap_attempt(id, 11).is_none());
        assert_eq!(tracker.attempt_gap_id, Some(id));
        assert_eq!(tracker.batches_processed, 1);
    }

    #[test]
    fn gap_repair_budget_is_scoped_to_topology_revision() {
        let mut tracker = TimelineGapRepairTracker::default();
        let first = projected_gap_id(7, 1);
        let revised = projected_gap_id(8, 1);

        assert!(tracker.admit_gap_attempt(first, 11).is_some());
        assert!(
            tracker
                .record_batch(TimelineGapRepairTrigger::LiveEdge)
                .is_some()
        );

        assert!(tracker.admit_gap_attempt(revised, 11).is_some());
        assert_eq!(tracker.attempt_gap_id, Some(revised));
        assert_eq!(tracker.batches_processed, 0);
        assert_eq!(tracker.live_edge_batches_processed, 0);
    }

    #[test]
    fn gap_repair_budget_is_scoped_to_gap_ordinal() {
        let mut tracker = TimelineGapRepairTracker::default();
        let first = projected_gap_id(7, 1);
        let another = projected_gap_id(7, 2);

        assert!(tracker.admit_gap_attempt(first, 11).is_some());
        assert!(
            tracker
                .record_batch(TimelineGapRepairTrigger::Automatic)
                .is_some()
        );

        assert!(tracker.admit_gap_attempt(another, 11).is_some());
        assert_eq!(tracker.attempt_gap_id, Some(another));
        assert_eq!(tracker.batches_processed, 0);
    }

    #[test]
    fn gap_repair_budget_is_scoped_to_explicit_demand_revision() {
        let mut tracker = TimelineGapRepairTracker::default();
        let id = projected_gap_id(7, 1);

        assert!(tracker.admit_gap_attempt(id, 11).is_some());
        assert!(
            tracker
                .record_batch(TimelineGapRepairTrigger::Automatic)
                .is_some()
        );

        assert!(tracker.admit_gap_attempt(id, 12).is_some());
        assert_eq!(tracker.attempt_gap_id, Some(id));
        assert_eq!(tracker.batches_processed, 0);
    }

    #[test]
    fn gap_repair_budget_is_scoped_to_room_reselection_demand() {
        let projected = vec![(0, projected_gap_position(7, 1, 8))];
        let id = projected_gap_id(7, 1);
        let mut tracker = TimelineGapRepairTracker::default();
        tracker.replace_projected_gaps(projected, Some((5, 10)), &[id]);
        let initial_demand = tracker.begin_explicit_demand();
        assert!(tracker.admit_gap_attempt(id, initial_demand).is_some());
        assert_eq!(
            tracker.record_batch(TimelineGapRepairTrigger::Automatic),
            Some(1)
        );

        let reselection_demand = tracker.begin_explicit_demand();

        assert_ne!(initial_demand, reselection_demand);
        assert_eq!(tracker.batches_processed, 1);
        assert!(matches!(
            tracker.evaluate_viewport_wake(Some((5, 10)), &[id]),
            GapRepairViewportWakeDecision::Wake { .. }
        ));
        assert!(tracker.admit_gap_attempt(id, reselection_demand).is_some());
        assert_eq!(tracker.batches_processed, 0);
    }

    #[test]
    fn gap_repair_budget_is_scoped_to_newly_visible_demand() {
        let projected = vec![(0, projected_gap_position(7, 1, 8))];
        let id = projected_gap_id(7, 1);
        let mut tracker = TimelineGapRepairTracker::default();
        let initial_demand = tracker.begin_explicit_demand();
        tracker.replace_projected_gaps(projected, None, &[]);
        assert!(tracker.admit_gap_attempt(id, initial_demand).is_some());
        assert_eq!(
            tracker.record_batch(TimelineGapRepairTrigger::Automatic),
            Some(1)
        );

        assert!(matches!(
            tracker.evaluate_viewport_wake(None, &[id]),
            GapRepairViewportWakeDecision::Wake {
                candidate: ProjectedGapCandidate {
                    relation: ProjectedGapRelation::ExplicitVisible,
                    ..
                }
            }
        ));
        let visible_demand = tracker.demand_revision;

        assert_ne!(initial_demand, visible_demand);
        assert!(tracker.admit_gap_attempt(id, visible_demand).is_some());
        assert_eq!(tracker.batches_processed, 0);
        assert!(matches!(
            tracker.evaluate_viewport_wake(None, &[id]),
            GapRepairViewportWakeDecision::IdleUnchangedCandidate { .. }
        ));
        assert_eq!(tracker.demand_revision, visible_demand);
    }

    #[test]
    fn repair_projection_waits_for_the_exact_tagged_batch() {
        let mut correlation = TimelineGapProjectionCorrelation::default();
        let operation = historical_causal_projection_operation(11);
        correlation.begin(9, operation);

        // An unrelated live diff can consume batch 5 without satisfying the repair.
        assert_eq!(
            correlation.complete(9, operation, Some(1)),
            TimelineGapProjectionCompletion::Pending
        );
        assert_eq!(
            correlation.observe(
                CausalProjectionId {
                    actor_generation: 9,
                    operation: historical_causal_projection_operation(10),
                    projection_batch: 1,
                },
                TimelineBatchId(5),
            ),
            None
        );
        assert_eq!(
            correlation.observe(
                CausalProjectionId {
                    actor_generation: 9,
                    operation,
                    projection_batch: 1,
                },
                TimelineBatchId(6),
            ),
            Some(TimelineBatchId(6))
        );
    }

    #[test]
    fn repair_projection_uses_the_last_sdk_projection_batch() {
        let mut correlation = TimelineGapProjectionCorrelation::default();
        let operation = historical_causal_projection_operation(7);
        correlation.begin(4, operation);
        assert_eq!(
            correlation.observe(
                CausalProjectionId {
                    actor_generation: 4,
                    operation,
                    projection_batch: 1,
                },
                TimelineBatchId(20),
            ),
            None
        );
        assert_eq!(
            correlation.complete(4, operation, Some(2)),
            TimelineGapProjectionCompletion::Pending
        );
        assert_eq!(
            correlation.observe(
                CausalProjectionId {
                    actor_generation: 4,
                    operation,
                    projection_batch: 2,
                },
                TimelineBatchId(21),
            ),
            Some(TimelineBatchId(21))
        );
    }

    #[test]
    fn gap_only_cache_reveal_requires_no_render_fence() {
        let mut correlation = TimelineGapProjectionCorrelation::default();
        let operation = historical_causal_projection_operation(3);
        correlation.begin(2, operation);
        assert_eq!(
            correlation.complete(2, operation, None),
            TimelineGapProjectionCompletion::NoDiff
        );
        assert!(!correlation.is_pending());
    }

    #[tokio::test]
    async fn gap_repair_room_switch_cancels_completion() {
        use matrix_sdk::{
            linked_chunk::{ChunkIdentifier, LinkedChunkId, Position, Update},
            test_utils::mocks::{MatrixMockServer, RoomMessagesResponseTemplate},
        };
        use matrix_sdk_base::event_cache::Gap;
        use matrix_sdk_test::{ALICE, event_factory::EventFactory};

        let server = MatrixMockServer::new().await;
        let client = server.client_builder().build().await;
        let room_id = matrix_sdk::ruma::room_id!("!cancel-gap:example.org");
        let older_id = matrix_sdk::ruma::event_id!("$cancel-older:example.org");
        let newer_id = matrix_sdk::ruma::event_id!("$cancel-newer:example.org");
        let missing_id = matrix_sdk::ruma::event_id!("$cancel-missing:example.org");
        let factory = EventFactory::new().room(room_id).sender(&ALICE);
        {
            let store = client
                .event_cache_store()
                .lock()
                .await
                .expect("cache store");
            store
                .as_clean()
                .expect("clean cache store")
                .handle_linked_chunk_updates(
                    LinkedChunkId::Room(room_id),
                    vec![
                        Update::NewItemsChunk {
                            previous: None,
                            new: ChunkIdentifier::new(0),
                            next: None,
                        },
                        Update::PushItems {
                            at: Position::new(ChunkIdentifier::new(0), 0),
                            items: vec![factory.text_msg("older").event_id(older_id).into_event()],
                        },
                        Update::NewGapChunk {
                            previous: Some(ChunkIdentifier::new(0)),
                            new: ChunkIdentifier::new(1),
                            next: None,
                            gap: Gap {
                                token: "cancel-gap-token".to_owned(),
                            },
                        },
                        Update::NewItemsChunk {
                            previous: Some(ChunkIdentifier::new(1)),
                            new: ChunkIdentifier::new(2),
                            next: None,
                        },
                        Update::PushItems {
                            at: Position::new(ChunkIdentifier::new(2), 0),
                            items: vec![factory.text_msg("newer").event_id(newer_id).into_event()],
                        },
                    ],
                )
                .await
                .expect("seed persisted gap");
        }
        client
            .event_cache()
            .subscribe()
            .expect("event cache subscribe");
        server.sync_joined_room(&client, room_id).await;
        server
            .mock_room_messages()
            .match_from("cancel-gap-token")
            .match_limit(64)
            .ok(RoomMessagesResponseTemplate::default().events(vec![
                factory.text_msg("missing").event_id(missing_id),
                factory.text_msg("older").event_id(older_id),
            ]))
            .expect(1)
            .named("old-actor-real-gap-repair")
            .mount()
            .await;

        let session = Arc::new(MatrixClientSession::from_client_for_testing(
            client,
            SessionInfo {
                homeserver: "http://example.invalid".to_owned(),
                user_id: ALICE.to_string(),
                device_id: "DEVICE".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            },
        ));
        let key = TimelineKey::room(
            AccountKey("@cancel-gap:example.org".to_owned()),
            room_id.to_string(),
        );
        let projection_request_id = fake_rid(27_500);
        let (action_tx, mut action_rx) = mpsc::channel(128);
        let (event_tx, mut event_rx) = broadcast::channel(128);
        let (manager_tx, _manager_rx) = mpsc::channel(16);
        let mut manager = live_tail_test_manager(HashMap::new());
        manager.session = Some(session);
        manager.action_tx = action_tx;
        manager.event_tx = event_tx;
        manager.msg_tx = manager_tx;
        manager.test_session_available = false;
        manager
            .handle_subscribe(projection_request_id, key.clone(), true, true)
            .await;

        let (old_actor_generation, timeline_generation) =
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if let CoreEvent::Timeline(TimelineEvent::InitialItems {
                        request_id: Some(request_id),
                        key: emitted_key,
                        actor_generation,
                        generation,
                        ..
                    }) = event_rx.recv().await.expect("initial actor event")
                        && request_id == projection_request_id
                        && emitted_key == key
                    {
                        break (actor_generation, generation);
                    }
                }
            })
            .await
            .expect("real actor initial projection");
        let old_actor_tx = manager
            .timelines
            .get(&key)
            .expect("old room actor")
            .tx
            .clone();

        let (projection_ack_tx, projection_ack_rx) = oneshot::channel();
        assert!(
            manager
                .timelines
                .get(&key)
                .expect("old room actor")
                .send(TimelineActorMessage::AcknowledgeProjection {
                    projection_request_id,
                    generation: timeline_generation,
                    response: projection_ack_tx,
                })
                .await
        );
        assert!(
            projection_ack_rx
                .await
                .expect("initial projection acknowledgement")
                .accepted
        );

        let (reached_tx, reached_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let (forwarded_tx, forwarded_rx) = oneshot::channel();
        let (armed_tx, armed_rx) = oneshot::channel();
        assert!(
            manager
                .timelines
                .get(&key)
                .expect("old room actor")
                .send(TimelineActorMessage::TestArmGapRepairCompletionPause {
                    pause: TestGapRepairCompletionPause {
                        reached: reached_tx,
                        release: release_rx,
                        forwarded: forwarded_tx,
                    },
                    acknowledged: armed_tx,
                })
                .await
        );
        armed_rx.await.expect("completion pause armed");
        assert!(
            manager
                .timelines
                .get(&key)
                .expect("old room actor")
                .send(TimelineActorMessage::InspectTimelineGaps {
                    trigger: TimelineGapRepairTrigger::Manual,
                })
                .await
        );

        let started_generation = tokio::time::timeout(Duration::from_secs(5), async {
            'started: loop {
                for action in action_rx.recv().await.expect("gap repair action channel") {
                    if let AppAction::TimelineGapRepairStarted {
                        room_id: started_room_id,
                        generation,
                        ..
                    } = action
                        && started_room_id == room_id.as_str()
                    {
                        break 'started generation;
                    }
                }
            }
        })
        .await
        .expect("real SDK gap repair started");
        tokio::time::timeout(Duration::from_secs(5), reached_rx)
            .await
            .expect("real SDK repair reached the session-to-actor completion boundary")
            .expect("completion pause sender");

        let (old_barrier_tx, old_barrier_rx) = oneshot::channel();
        assert!(
            manager
                .timelines
                .get(&key)
                .expect("old room actor")
                .send(TimelineActorMessage::Barrier(old_barrier_tx))
                .await
        );
        old_barrier_rx.await.expect("old actor pre-switch barrier");
        while action_rx.try_recv().is_ok() {}
        while event_rx.try_recv().is_ok() {}

        manager
            .handle_command(TimelineCommand::Unsubscribe {
                request_id: fake_rid(27_501),
                key: key.clone(),
            })
            .await;
        assert!(!manager.timelines.contains_key(&key));
        let replacement_generation = manager
            .timeline_actor_generations
            .activate_after_quiescence(&key)
            .await
            .generation;
        assert_ne!(old_actor_generation, replacement_generation);
        while action_rx.try_recv().is_ok() {}
        while event_rx.try_recv().is_ok() {}

        let _ = release_tx.send(());
        let completion_forwarded = match tokio::time::timeout(Duration::from_secs(1), forwarded_rx)
            .await
            .expect("paused repair worker must settle after old actor drop")
        {
            Ok(forwarded) => forwarded,
            Err(_) => false,
        };
        let old_actor_closed =
            tokio::time::timeout(Duration::from_millis(100), old_actor_tx.closed())
                .await
                .is_ok();
        if !old_actor_closed {
            let (barrier_tx, barrier_rx) = oneshot::channel();
            if old_actor_tx
                .send(TimelineActorMessage::Barrier(barrier_tx))
                .await
                .is_ok()
            {
                let _ = tokio::time::timeout(Duration::from_secs(1), barrier_rx).await;
            }
        }

        let mut stale_actions = Vec::new();
        while let Ok(actions) = action_rx.try_recv() {
            for action in actions {
                let label = match action {
                    AppAction::TimelineGapRepairProgressed { .. } => Some("Progressed"),
                    AppAction::TimelineGapRepairFailed { .. } => Some("Failed"),
                    AppAction::TimelineContinuityInspectionStarted { .. } => {
                        Some("inspection continuation")
                    }
                    _ => None,
                };
                if let Some(label) = label {
                    stale_actions.push(label);
                }
            }
        }
        let mut stale_core_event_count = 0;
        while event_rx.try_recv().is_ok() {
            stale_core_event_count += 1;
        }

        assert!(
            !completion_forwarded,
            "the released old-generation repair completion reached its actor mailbox"
        );
        assert!(
            old_actor_closed,
            "the unsubscribed actor channel stayed open"
        );
        assert!(
            stale_actions.is_empty(),
            "old generation {old_actor_generation} published reducer work after replacement generation {replacement_generation}: {stale_actions:?}; repair generation {started_generation}"
        );
        assert_eq!(
            stale_core_event_count, 0,
            "old generation {old_actor_generation} published CoreEvent output after replacement generation {replacement_generation}"
        );
    }
}
