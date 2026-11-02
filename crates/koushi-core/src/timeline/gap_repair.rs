use std::collections::BTreeMap;
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
use crate::executor;
use crate::live_catchup::{LiveCatchupGate, classify_live_catchup_gate};
use crate::live_tail_freshness::{
    FOREGROUND_LIVE_TAIL_LIMIT, LiveTailFreshnessState, LiveTailSchedulerAction,
};
use koushi_protocol::event::{
    CoreEvent, TimelineEvent, TimelineGapId, TimelineGapPosition, TimelineItem, TimelineItemId,
};
use koushi_protocol::ids::{TimelineBatchId, TimelineGeneration, TimelineKey, TimelineKind};

// BEGIN GENERATED SIBLING IMPORTS
use super::actor::{TimelineActor, TimelineActorControl, TimelineActorMessage};
use super::diagnostics::{
    TimelineGapSelectionDiagnostic, record_live_catchup_gate, record_live_tail_cancellation,
    record_live_tail_commit, record_live_tail_queue, record_live_tail_reconciliation,
    record_live_tail_refresh, record_live_tail_state, record_timeline_gap_inspection_work,
    record_timeline_gap_projection, record_timeline_gap_projection_boundary,
    record_timeline_gap_repair, record_timeline_gap_selection,
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
        // #1737: this run now owns a committed response, so automatic read-state
        // writes may reflect the visible timeline again.
        self.read_state_authority = true;

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
    #[expect(
        clippy::too_many_arguments,
        reason = "actor message handler: parameters are the destructured message fields"
    )]
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
pub(super) struct GapProjectionRelayed {
    pub(super) actor_generation: u64,
    pub(super) timeline_generation: TimelineGeneration,
    pub(super) repair_generation: u64,
    pub(super) minimum_batch_id: TimelineBatchId,
}

pub(super) fn gap_projection_relay_is_current(
    actual: GapProjectionRelayed,
    actor_generation: u64,
    timeline_generation: TimelineGeneration,
    repair_generation: u64,
    minimum_batch_id: TimelineBatchId,
) -> bool {
    actual.actor_generation == actor_generation
        && actual.timeline_generation == timeline_generation
        && actual.repair_generation == repair_generation
        && actual.minimum_batch_id >= minimum_batch_id
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
        initial_projection_committed: bool,
    ) -> Option<(u64, TimelineGapRepairTrigger)> {
        if !initial_projection_committed || self.active_serial.is_some() {
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
            || self
                .live_tail_refresh
                .as_ref()
                .is_none_or(|(current, _, _)| *current != operation_generation)
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
        if !self.initial_projection_committed {
            "awaiting_initial_projection"
        } else if self.pagination_task.is_some() {
            "pagination"
        } else if self.gap_projection_correlation.is_pending()
            || self.pending_gap_projection.is_some()
        {
            "awaiting_relay"
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
            .begin_pending_inspection(self.initial_projection_committed)
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
        // Record the completed inspection's content-free counters right after
        // admission is confirmed, before the checkpoint-staleness gate below:
        // the SDK inspection already ran and incurred its reads, and a
        // superseded checkpoint must not hide that work from the counters.
        if let Ok(inspection) = &result {
            record_timeline_gap_inspection_work(
                timeline_gap_repair_trigger_token(trigger),
                serial,
                match inspection.continuity {
                    MatrixTimelineContinuity::Unknown => "unknown",
                    MatrixTimelineContinuity::Gapped => "gapped",
                    MatrixTimelineContinuity::Complete => "complete",
                },
                inspection.diagnostics.scanned_chunks,
                inspection.diagnostics.event_bodies_decoded,
                inspection.diagnostics.duration_ms,
            );
        }
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
                                if let Some(checkpoint) = committed_response.as_ref()
                                    && consume_room_subscription_checkpoint(
                                        &mut self.room_subscription_checkpoint,
                                        &mut self.deferred_room_subscription_checkpoint,
                                        checkpoint,
                                    )
                                {
                                    self.gap_repair
                                        .queue_inspection(TimelineGapRepairTrigger::LiveEdge);
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
        self.display_projection
            .display_items()
            .iter()
            .position(|item| {
                item.display_metadata.as_ref().is_some_and(|metadata| {
                    metadata.content_event_id.as_deref() == Some(event_id)
                        || metadata.activity_event_id.as_deref() == Some(event_id)
                }) || matches!(
                    &item.id,
                    TimelineItemId::Event { event_id: candidate } if candidate == event_id
                )
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
        let relayed = GapProjectionRelayed {
            actor_generation: self.actor_generation,
            timeline_generation: self.generation,
            repair_generation: pending.repair_generation,
            minimum_batch_id: batch_id,
        };
        // The load-bearing stale fence is the causal-projection correlation at
        // the relay call site. This local assertion pins the complete signal
        // shape before the actor advances its scheduler.
        if !gap_projection_relay_is_current(
            relayed,
            self.actor_generation,
            self.generation,
            pending.repair_generation,
            batch_id,
        ) {
            return;
        }
        self.gap_repair.queue_inspection(pending.trigger);
        record_timeline_gap_repair(
            "projection_relayed",
            timeline_gap_repair_trigger_token(pending.trigger),
            pending.repair_generation,
            pending.gap_count,
            pending.batches_processed,
            "accepted",
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
        self.start_pending_timeline_gap_inspection().await;
    }
}

#[cfg(test)]
mod tests;
