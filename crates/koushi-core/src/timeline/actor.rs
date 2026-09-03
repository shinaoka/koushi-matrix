#[cfg(test)]
use std::collections::BTreeSet;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};

use futures_util::StreamExt;
use koushi_sdk::{
    MatrixClientSession, MatrixCommittedRoomTimelineCheckpoint as MatrixRoomSubscriptionCheckpoint,
    MatrixLiveTailRefreshCancellation, MatrixLiveTailRefreshResult, MatrixTimelineGapError,
    MatrixTimelineGapInspection, MatrixTimelineGapRepairResult,
};
use koushi_state::{
    AppAction, ComposerDocument, TimelineMediaGalleryItem, TimelineThreadRootOrder,
};

use matrix_sdk::send_queue::{RoomSendQueueUpdate, SendHandle};
use matrix_sdk_ui::timeline::Timeline;
#[cfg(test)]
use matrix_sdk_ui::timeline::TimelineItem as SdkTimelineItem;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::account_work::{AccountWorkKind, AccountWorkScheduler};
#[cfg(test)]
use crate::causal_projection::CausalProjectionId;
use crate::executor;
use crate::link_preview::LinkPreviewContext;
use crate::search::SearchIndexMessage;
use crate::startup_trace::{self, StartupPhase};
use crate::threads_list::ThreadRootProjectionService;
use koushi_protocol::command::MediaDownloadSelection;
#[cfg(test)]
use koushi_protocol::event::TimelineAnchorRestoreStatus;
use koushi_protocol::event::{
    CoreEvent, LinkPreview, PaginationDirection, TimelineDiff, TimelineItem, TimelineItemId,
    TimelineNavigationSnapshot, TimelineReadStateSync, TimelineResyncReason, TimelineSendState,
    TimelineViewportObservation,
};
use koushi_protocol::failure::{CoreFailure, TimelineFailureKind};
use koushi_protocol::ids::{
    RequestId, RuntimeConnectionId, TimelineBatchId, TimelineGeneration, TimelineKey, TimelineKind,
};

// BEGIN GENERATED SIBLING IMPORTS
use super::diagnostics::{
    event_cache_origin_trace_token, record_live_catchup_gate, record_subscribe_stage,
    record_timeline_gap_demand, record_timeline_gap_projection_boundary,
    record_timeline_gap_repair, record_timeline_gap_repair_evaluation, trace_event_cache_diffs,
    trace_event_cache_items, trace_timeline_items, trace_timeline_paginate,
};
use super::display_projection::{DisplayProjectionContext, DisplayProjectionState};
use super::gap_repair::{
    GapRepairEvaluationDiagnosticSignature, GapRepairViewportWakeDecision, GlobalCommitDecision,
    GlobalCommitFence, GlobalResponseCommit, PendingLiveTailRefreshCompletion,
    PendingTimelineGapProjection, RestoreCausalProjectionBuffer, TimelineGapProjectionCorrelation,
    TimelineGapRepairTracker, TimelineGapRepairTrigger, historical_causal_projection_operation,
    projected_gaps_contain_id, rendered_live_edge_target, retain_room_subscription_checkpoint,
    room_checkpoint_advances_global_fence, should_record_gap_repair_evaluation,
    timeline_gap_repair_trigger_token,
};
use super::item_projection::{
    ReceiptObservationTarget, apply_ignored_sender_suppression, apply_link_previews_to_item,
    cache_sdk_item_media_source, emit_receipt_observation_actions,
    live_event_receipts_from_sdk_items, remember_local_echo, sdk_item_to_timeline_item,
    thread_auto_requestable_event_id, timeline_room_id, withheld_update_should_publish,
};
use super::manager::TimelineMessage;
use super::media::{
    MediaDownloadOutcome, PrivateMediaEntry, media_gallery_items_from_timeline_items,
    media_gallery_updated_action,
};
use super::navigation::{
    ActivePaginationTask, INITIAL_EMPTY_ROOM_BACKFILL_EVENT_COUNT, InitialItemsRequestIdentity,
    PaginationCompletion, RestoreTimelineAnchorState, TimelineActorGenerationGate,
    activity_rows_from_timeline_items, emit_initial_items_for_generation,
    emit_timeline_events_for_generation, send_generation_fenced,
    should_hydrate_empty_initial_room_timeline,
};
use super::outbound_send::{
    MatrixTimelineSendEnqueueContext, PendingSendProjection, SharedSendCompletionCoordinator,
    TimelineSendEnqueueContext, TimelineSendTerminalIngress, run_send_queue_monitor,
    thread_activity_observed_action, thread_attention_action,
};
use super::read_state::{ReadActorApplyKind, run_typing_notifications};
use super::relay::{
    RELAY_RESTART_BASE_DELAY, RELAY_RESTART_MAX_DELAY, RelayRestartBackoff, TimelineRelayBatch,
    TimelineRelayControl, accepted_relay_batch, run_diff_relay,
};
use super::room_key_recovery::{
    DecryptRetryController, DecryptRetrySettledResult, KeyRequestUiState,
    decrypt_retry_settlement_operation,
};
use super::thread_projection::{
    ThreadAttentionCounters, ThreadAttentionTracker, thread_root_item_with_authoritative_aggregate,
    thread_summary_observations_for_windows,
};
// END GENERATED SIBLING IMPORTS
#[cfg(test)]
use super::gap_repair::TestGapRepairCompletionPause;
#[cfg(test)]
use super::thread_projection::ThreadAttentionBatchProvenance;

const TIMELINE_ACTOR_CONTROL_QUEUE_CAPACITY: usize = 16;

fn should_fetch_members(kind: &TimelineKind) -> bool {
    matches!(
        kind,
        TimelineKind::Room { .. } | TimelineKind::Focused { .. }
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ThreadSummaryProjectionWake {
    Updated {
        root_event_id: String,
        activity_revision: u64,
        summary_revision: u64,
    },
    Cleared {
        root_event_id: String,
        activity_revision: u64,
        summary_revision: u64,
    },
}

#[derive(Clone)]
pub(super) struct ThreadSummaryProjectionIngress {
    tx: watch::Sender<BTreeMap<String, ThreadSummaryProjectionWake>>,
}

impl ThreadSummaryProjectionIngress {
    pub(super) fn channel() -> (
        Self,
        watch::Receiver<BTreeMap<String, ThreadSummaryProjectionWake>>,
    ) {
        let (tx, rx) = watch::channel(BTreeMap::new());
        (Self { tx }, rx)
    }

    /// Latest-wins, bounded publication. The watch value remains owned until
    /// the Room actor atomically drains it, so manager publication never waits
    /// for the ordinary actor mailbox.
    pub(super) fn publish(&self, wake: ThreadSummaryProjectionWake) {
        let (root_event_id, activity_revision, summary_revision) = match &wake {
            ThreadSummaryProjectionWake::Updated {
                root_event_id,
                activity_revision,
                summary_revision,
            }
            | ThreadSummaryProjectionWake::Cleared {
                root_event_id,
                activity_revision,
                summary_revision,
            } => (root_event_id.clone(), *activity_revision, *summary_revision),
        };
        self.tx.send_modify(|pending| {
            assert!(
                pending.contains_key(&root_event_id)
                    || pending.len() < crate::threads_list::THREAD_SUMMARY_PROJECTION_MAX_ROOTS,
                "thread-summary projection wake slots exceeded"
            );
            let newer = pending.get(&root_event_id).is_none_or(|current| {
                let current_revisions = match current {
                    ThreadSummaryProjectionWake::Updated {
                        activity_revision,
                        summary_revision,
                        ..
                    }
                    | ThreadSummaryProjectionWake::Cleared {
                        activity_revision,
                        summary_revision,
                        ..
                    } => (*activity_revision, *summary_revision),
                };
                (activity_revision, summary_revision) >= current_revisions
            });
            if newer {
                pending.insert(root_event_id.clone(), wake);
            }
        });
    }

    fn drain(
        &self,
        receiver: &mut watch::Receiver<BTreeMap<String, ThreadSummaryProjectionWake>>,
    ) -> Vec<ThreadSummaryProjectionWake> {
        let mut drained = BTreeMap::new();
        loop {
            self.tx.send_modify(|pending| drained.append(pending));
            // Mark the clear (or a racing publication) observed. If a publisher
            // won before this borrow, the non-empty check loops and drains it;
            // if it wins after the check, `changed()` remains ready.
            receiver.borrow_and_update();
            if self.tx.borrow().is_empty() {
                break;
            }
        }
        drained.into_values().collect()
    }
}

pub(super) enum TimelineActorMessage {
    RoomSubscriptionCheckpoint(MatrixRoomSubscriptionCheckpoint),
    /// Ordered generation advance for a retained room whose subscription set
    /// changed because another room was added/removed (issue #518). The actor
    /// must accept new-generation checkpoints after this message is processed.
    UpdateSubscriptionGeneration(u64),
    GlobalResponseCommitted(GlobalResponseCommit),
    StartLiveTailRefresh {
        epoch: u64,
        operation_generation: u64,
        limit: u16,
    },
    CancelLiveTailNetwork {
        operation_generation: u64,
        acknowledged: oneshot::Sender<()>,
    },
    LiveTailRefreshFinished {
        actor_generation: u64,
        epoch: u64,
        operation_generation: u64,
        requested_limit: u16,
        result: MatrixLiveTailRefreshResult,
        duration_ms: u128,
    },
    InspectTimelineGaps {
        trigger: TimelineGapRepairTrigger,
    },
    TimelineGapInspectionFinished {
        serial: u64,
        trigger: TimelineGapRepairTrigger,
        committed_response: Option<MatrixRoomSubscriptionCheckpoint>,
        global_commit: Option<GlobalResponseCommit>,
        result: Result<MatrixTimelineGapInspection, MatrixTimelineGapError>,
    },
    TimelineGapRepairFinished {
        serial: u64,
        trigger: TimelineGapRepairTrigger,
        repaired_live_edge_fallback: bool,
        result: Result<MatrixTimelineGapRepairResult, MatrixTimelineGapError>,
    },
    TimelineGapRelaySettlementDue {
        actor_generation: u64,
        repair_generation: u64,
        trigger: TimelineGapRepairTrigger,
    },
    Paginate {
        request_id: RequestId,
        direction: PaginationDirection,
        event_count: u16,
    },
    CancelPagination {
        request_id: RequestId,
    },
    CancelLinkPreviews {
        request_id: RequestId,
    },
    PaginationFinished {
        serial: u64,
        request_id: RequestId,
        direction: PaginationDirection,
        completion: PaginationCompletion,
    },
    OwnReadReceiptChanged,
    RestoreTimelineAnchor {
        request_id: RequestId,
        event_id: String,
        max_batches: u16,
        event_count: u16,
    },
    RestoreTimelineAnchorContinue {
        serial: u64,
    },
    ObserveViewport {
        observation: TimelineViewportObservation,
    },
    BeginGapRepairDemand,
    EndGapRepairDemand,
    ForwardMessage {
        request_id: RequestId,
        source_event_id: String,
        destination_room_id: String,
        transaction_id: String,
    },
    LoadMessageSource {
        request_id: RequestId,
        event_id: String,
    },
    ReplyDetailsFetchFinished {
        event_id: String,
    },
    RequestRoomKey {
        request_id: Option<RequestId>,
        event_id: String,
        origin: koushi_protocol::command::KeyRequestOrigin,
    },
    RequestLateDecryption {
        request_id: Option<RequestId>,
        trigger: &'static str,
    },
    RoomKeyRecoveryTick {
        session_id: String,
        attempt: u32,
        actor_generation: u64,
    },
    RoomKeyWithheldObserved {
        room_id: String,
        session_id: String,
        code: &'static str,
    },
    DecryptRetryTimeout {
        operation: u64,
        actor_generation: u64,
    },
    RetrySend {
        request_id: RequestId,
        transaction_id: String,
    },
    CancelSend {
        request_id: RequestId,
        transaction_id: String,
    },
    DownloadMedia {
        request_id: RequestId,
        event_id: String,
        selection: MediaDownloadSelection,
    },
    MediaDownloadFinished {
        request_id: RequestId,
        event_id: String,
        outcome: MediaDownloadOutcome,
    },
    EditText {
        request_id: RequestId,
        event_id: String,
        document: ComposerDocument,
    },
    Redact {
        request_id: RequestId,
        event_id: String,
    },
    ToggleReaction {
        request_id: RequestId,
        event_id: String,
        reaction_key: String,
    },
    SendReaction {
        request_id: RequestId,
        event_id: String,
        reaction_key: String,
    },
    RedactReaction {
        request_id: RequestId,
        event_id: String,
        reaction_key: String,
        reaction_event_id: String,
    },
    ApplyReadSuccess {
        kind: ReadActorApplyKind,
        event_id: String,
        acknowledged: oneshot::Sender<bool>,
    },
    ReadStateProjection {
        local_viewed_event_id: Option<String>,
        server_confirmed_read_event_id: Option<String>,
        sync: TimelineReadStateSync,
    },
    ReadStatePolicyChanged {
        send_read_receipts: bool,
    },
    DisplayPolicyChanged {
        thread_root_order: TimelineThreadRootOrder,
    },
    RefreshPendingSendProjection {
        actor_generation: u64,
        projections: Vec<PendingSendProjection>,
        acknowledged: oneshot::Sender<bool>,
    },
    SetTyping {
        request_id: RequestId,
        is_typing: bool,
    },
    TypingUsersUpdated(Vec<String>),
    IgnoredUsersUpdated(std::collections::BTreeSet<String>),
    LoadLinkPreviews {
        request_id: RequestId,
        event_id: String,
    },
    LinkPreviewsFetched {
        request_id: RequestId,
        event_id: String,
        previews: Vec<LinkPreview>,
        pending_count: usize,
        ready_count: usize,
        failed_count: usize,
        elapsed_ms: u128,
    },
    HideLinkPreview {
        request_id: RequestId,
        event_id: String,
    },
    LinkPreviewPolicyChanged {
        unencrypted_global_enabled: bool,
        encrypted_global_enabled: bool,
        room_enabled: Option<bool>,
    },
    /// Internal: send completed (from send queue monitor task).
    SendQueueUpdate(RoomSendQueueUpdate),
    /// Internal: send queue broadcast lagged and the actor must resync the
    /// SDK-owned local echo snapshot before projecting outbound send states.
    SendQueueLagged,
    /// Internal: re-emit the current navigation_items as InitialItems without
    /// tearing down the SDK subscription. A user-command Subscribe supplies
    /// its exact cause; internal recovery replay is causeless.
    ReplayInitialItems {
        cause_request_id: Option<RequestId>,
    },
    #[cfg(test)]
    TestBeginRestore {
        request_id: RequestId,
        event_id: String,
        acknowledged: oneshot::Sender<()>,
    },
    #[cfg(test)]
    TestInjectRestoreDiff {
        diffs: Vec<eyeball_im::VectorDiff<Arc<SdkTimelineItem>>>,
        projections: BTreeSet<CausalProjectionId>,
        acknowledged: oneshot::Sender<()>,
    },
    #[cfg(test)]
    TestRestoreCausalState(oneshot::Sender<(bool, bool, usize, BTreeSet<CausalProjectionId>)>),
    #[cfg(test)]
    TestFinishRestore {
        request_id: RequestId,
        response: oneshot::Sender<bool>,
    },
    #[cfg(test)]
    TestArmGapRepairCompletionPause {
        pause: TestGapRepairCompletionPause,
        acknowledged: oneshot::Sender<()>,
    },
    #[cfg(test)]
    Barrier(oneshot::Sender<()>),
}

pub(super) enum TimelineActorControl {
    ReplayInitialItems {
        cause_request_id: RequestId,
    },
    StartLiveTailRefresh {
        epoch: u64,
        operation_generation: u64,
        limit: u16,
    },
    CancelLiveTailNetwork {
        operation_generation: u64,
        acknowledged: oneshot::Sender<()>,
    },
    ApplyReadSuccess {
        kind: ReadActorApplyKind,
        event_id: String,
        acknowledged: oneshot::Sender<bool>,
    },
    ReadStateProjection {
        local_viewed_event_id: Option<String>,
        server_confirmed_read_event_id: Option<String>,
        sync: TimelineReadStateSync,
    },
    ReadStatePolicyChanged {
        send_read_receipts: bool,
    },
    DisplayPolicyChanged {
        thread_root_order: TimelineThreadRootOrder,
    },
    BeginGapRepairDemand,
    EndGapRepairDemand,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct TimelineActorCleanupState {
    end_gap_demand_serial: u64,
    cancel_network_serial: u64,
    cancel_network_operation_generation: u64,
    pub(super) cancel_pagination_serial: u64,
    pub(super) cancel_link_previews_serial: u64,
}

#[derive(Clone)]
pub(super) struct TimelineActorCleanupIngress {
    tx: watch::Sender<TimelineActorCleanupState>,
}

impl TimelineActorCleanupIngress {
    pub(super) fn channel() -> (Self, watch::Receiver<TimelineActorCleanupState>) {
        let (tx, rx) = watch::channel(TimelineActorCleanupState::default());
        (Self { tx }, rx)
    }

    fn end_gap_repair_demand(&self) {
        self.tx.send_modify(|state| {
            state.end_gap_demand_serial = state.end_gap_demand_serial.wrapping_add(1).max(1);
        });
    }

    fn cancel_live_tail_network(&self, operation_generation: u64) {
        self.tx.send_modify(|state| {
            state.cancel_network_serial = state.cancel_network_serial.wrapping_add(1).max(1);
            state.cancel_network_operation_generation = operation_generation;
        });
    }

    fn cancel_pagination(&self) {
        self.tx.send_modify(|state| {
            state.cancel_pagination_serial = state.cancel_pagination_serial.wrapping_add(1).max(1);
        });
    }

    fn cancel_link_previews(&self) {
        self.tx.send_modify(|state| {
            state.cancel_link_previews_serial =
                state.cancel_link_previews_serial.wrapping_add(1).max(1);
        });
    }
}

impl From<TimelineActorControl> for TimelineActorMessage {
    fn from(control: TimelineActorControl) -> Self {
        match control {
            TimelineActorControl::ReplayInitialItems { cause_request_id } => {
                Self::ReplayInitialItems {
                    cause_request_id: Some(cause_request_id),
                }
            }
            TimelineActorControl::StartLiveTailRefresh {
                epoch,
                operation_generation,
                limit,
            } => Self::StartLiveTailRefresh {
                epoch,
                operation_generation,
                limit,
            },
            TimelineActorControl::CancelLiveTailNetwork {
                operation_generation,
                acknowledged,
            } => Self::CancelLiveTailNetwork {
                operation_generation,
                acknowledged,
            },
            TimelineActorControl::ApplyReadSuccess {
                kind,
                event_id,
                acknowledged,
            } => Self::ApplyReadSuccess {
                kind,
                event_id,
                acknowledged,
            },
            TimelineActorControl::ReadStateProjection {
                local_viewed_event_id,
                server_confirmed_read_event_id,
                sync,
            } => Self::ReadStateProjection {
                local_viewed_event_id,
                server_confirmed_read_event_id,
                sync,
            },
            TimelineActorControl::ReadStatePolicyChanged { send_read_receipts } => {
                Self::ReadStatePolicyChanged { send_read_receipts }
            }
            TimelineActorControl::DisplayPolicyChanged { thread_root_order } => {
                Self::DisplayPolicyChanged { thread_root_order }
            }
            TimelineActorControl::BeginGapRepairDemand => Self::BeginGapRepairDemand,
            TimelineActorControl::EndGapRepairDemand => Self::EndGapRepairDemand,
        }
    }
}

pub(super) fn canonical_activity_window_action(
    key: &TimelineKey,
    items: &[TimelineItem],
) -> Option<AppAction> {
    let TimelineKind::Room { room_id } = &key.kind else {
        return None;
    };
    let mut redacted_event_ids = Vec::new();
    let mut hidden_event_ids = Vec::new();
    for item in items {
        let TimelineItemId::Event { event_id } = &item.id else {
            continue;
        };
        if item.is_redacted {
            redacted_event_ids.push(event_id.clone());
        }
        if item.is_hidden {
            hidden_event_ids.push(event_id.clone());
        }
    }
    Some(AppAction::CanonicalActivityWindowReconciled {
        room_id: room_id.clone(),
        rows: activity_rows_from_timeline_items(key, items),
        redacted_event_ids,
        hidden_event_ids,
    })
}

pub(super) async fn reserve_canonical_activity_action(
    action_tx: &mpsc::Sender<Vec<AppAction>>,
    key: &TimelineKey,
) -> Option<mpsc::OwnedPermit<Vec<AppAction>>> {
    if !matches!(key.kind, TimelineKind::Room { .. }) {
        return None;
    }
    action_tx.clone().reserve_owned().await.ok()
}

/// Ordered delivery for projection state-machine actions. These transitions
/// must wait for reducer capacity; `try_send` would let Core/frontend retain a
/// root snapshot while AppState permanently misses its matching transition.
pub(super) async fn emit_app_action_reliable(
    action_tx: &mpsc::Sender<Vec<AppAction>>,
    action: AppAction,
) -> bool {
    action_tx.send(vec![action]).await.is_ok()
}

pub(super) struct TimelineActorHandle {
    pub(super) tx: mpsc::Sender<TimelineActorMessage>,
    pub(super) control_tx: Option<mpsc::Sender<TimelineActorControl>>,
    pub(super) thread_summary_projection: ThreadSummaryProjectionIngress,
    pub(super) position_rx: Option<watch::Receiver<Arc<TimelinePositionIndex>>>,
    pub(super) task: Option<executor::JoinHandle<()>>,
    pub(super) auxiliary_tasks: Vec<executor::JoinHandle<()>>,
    pub(super) subscription_generation: Option<u64>,
    pub(super) enqueue_context: Option<TimelineSendEnqueueContext>,
}

pub(super) struct TimelinePositionIndex {
    pub(super) generation: u128,
    pub(super) ranks: HashMap<String, u64>,
}

impl TimelinePositionIndex {
    pub(super) fn from_items(
        actor_generation: u64,
        timeline_generation: TimelineGeneration,
        items: &[TimelineItem],
    ) -> Self {
        let generation = (u128::from(actor_generation) << 64) | u128::from(timeline_generation.0);
        let ranks = items
            .iter()
            .enumerate()
            .filter_map(|(rank, item)| {
                let TimelineItemId::Event { event_id } = &item.id else {
                    return None;
                };
                Some((event_id.clone(), rank.try_into().unwrap_or(u64::MAX)))
            })
            .collect();
        Self { generation, ranks }
    }

    pub(super) fn evidence(
        &self,
        event_id: &str,
    ) -> Option<crate::read_state::ReadPositionEvidence> {
        self.ranks
            .get(event_id)
            .copied()
            .map(|rank| crate::read_state::ReadPositionEvidence {
                generation: self.generation,
                rank,
            })
    }

    pub(super) fn actor_generation(&self) -> u64 {
        (self.generation >> 64) as u64
    }
}

impl TimelineActorHandle {
    pub(super) fn thread_summary_projection(&self) -> &ThreadSummaryProjectionIngress {
        &self.thread_summary_projection
    }

    pub(super) async fn send(&self, msg: TimelineActorMessage) -> bool {
        self.tx.send(msg).await.is_ok()
    }

    pub(super) fn try_send(&self, msg: TimelineActorMessage) -> bool {
        self.tx.try_send(msg).is_ok()
    }

    pub(super) async fn send_control(&self, control: TimelineActorControl) -> bool {
        match &self.control_tx {
            Some(tx) => tx.send(control).await.is_ok(),
            None => self.send(control.into()).await,
        }
    }

    fn try_send_control(&self, control: TimelineActorControl) -> bool {
        match &self.control_tx {
            Some(tx) => tx.try_send(control).is_ok(),
            None => self.tx.try_send(control.into()).is_ok(),
        }
    }

    pub(super) fn end_gap_repair_demand(&self) {
        if let Some(cleanup) = self.cleanup_ingress() {
            cleanup.end_gap_repair_demand();
        } else {
            let _ = self.try_send_control(TimelineActorControl::EndGapRepairDemand);
        }
    }

    pub(super) fn cancel_live_tail_network(&self, operation_generation: u64) -> bool {
        if let Some(cleanup) = self.cleanup_ingress() {
            cleanup.cancel_live_tail_network(operation_generation);
            true
        } else {
            let (acknowledged, _acknowledgement) = oneshot::channel();
            self.try_send_control(TimelineActorControl::CancelLiveTailNetwork {
                operation_generation,
                acknowledged,
            })
        }
    }

    pub(super) fn cancel_pagination_after_commit(&self) {
        if let Some(cleanup) = self.cleanup_ingress() {
            cleanup.cancel_pagination();
        }
    }

    pub(super) fn cancel_link_previews_after_commit(&self) {
        if let Some(cleanup) = self.cleanup_ingress() {
            cleanup.cancel_link_previews();
        }
    }

    fn cleanup_ingress(&self) -> Option<&TimelineActorCleanupIngress> {
        match self.enqueue_context.as_ref() {
            Some(TimelineSendEnqueueContext::Matrix(context)) => Some(&context.cleanup),
            #[cfg(test)]
            Some(TimelineSendEnqueueContext::Synthetic { .. }) => None,
            #[cfg(test)]
            Some(TimelineSendEnqueueContext::CleanupProbe { cleanup }) => Some(cleanup),
            None => None,
        }
    }

    pub(super) fn read_position(
        &self,
        event_id: &str,
    ) -> Option<crate::read_state::ReadPositionEvidence> {
        self.position_rx
            .as_ref()
            .and_then(|receiver| receiver.borrow().evidence(event_id))
    }

    pub(super) fn read_position_index(&self) -> Option<Arc<TimelinePositionIndex>> {
        self.position_rx
            .as_ref()
            .map(|receiver| Arc::clone(&receiver.borrow()))
    }

    pub(super) async fn stop(mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
        for task in self.auxiliary_tasks.drain(..) {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for TimelineActorHandle {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        for task in &self.auxiliary_tasks {
            task.abort();
        }
    }
}

pub(super) struct LocalViewedBoundary {
    pub(super) event_id: String,
    pub(super) position: crate::read_state::ReadPositionEvidence,
}

pub(super) struct TimelineActor {
    pub(super) key: TimelineKey,
    pub(super) timeline: Arc<Timeline>,
    pub(super) session: Arc<MatrixClientSession>,
    pub(super) action_tx: mpsc::Sender<Vec<AppAction>>,
    pub(super) event_tx: broadcast::Sender<CoreEvent>,
    pub(super) msg_tx: mpsc::Sender<TimelineActorMessage>,
    msg_rx: mpsc::Receiver<TimelineActorMessage>,
    control_rx: mpsc::Receiver<TimelineActorControl>,
    cleanup_rx: watch::Receiver<TimelineActorCleanupState>,
    last_end_gap_demand_cleanup_serial: u64,
    last_cancel_network_cleanup_serial: u64,
    last_cancel_pagination_cleanup_serial: u64,
    last_cancel_link_previews_cleanup_serial: u64,
    pub(super) position_tx: watch::Sender<Arc<TimelinePositionIndex>>,
    pub(super) relay_control_tx: mpsc::Sender<TimelineRelayControl>,
    relay_control_rx: mpsc::Receiver<TimelineRelayControl>,
    pub(super) relay_data_rx: Option<mpsc::Receiver<TimelineRelayBatch>>,
    pub(super) relay_task: Option<executor::JoinHandle<()>>,
    pub(super) relay_restart_backoff: RelayRestartBackoff,
    pub(super) relay_restart_task: Option<executor::JoinHandle<()>>,
    pub(super) generation: TimelineGeneration,
    /// Stable identity of the actor-owned InitialItems projection. Replays
    /// preserve this id so a remounted consumer acknowledges the same owner.
    pub(super) projection_request_id: RequestId,
    pub(super) initial_projection_committed: bool,
    pub(super) next_batch_id: TimelineBatchId,
    /// Correlates send queue completions across the enqueue / SentEvent race.
    pub(super) send_completion: SharedSendCompletionCoordinator,
    /// SDK transaction id -> Rust-owned outbound send state.
    pub(super) send_statuses: HashMap<String, TimelineSendState>,
    /// SDK transaction id -> SDK send handle used for retry/cancel.
    pub(super) send_handles: HashMap<String, SendHandle>,
    /// Manager-owned sends retained even when the SDK local echo is omitted.
    pub(super) pending_send_projections: Vec<PendingSendProjection>,
    /// Current account user id, used to project reaction ownership.
    pub(super) own_user_id: Option<matrix_sdk::ruma::OwnedUserId>,
    /// event_id → SDK transaction id for events this actor sent. Used to
    /// address local-echo items whose remote echo has not arrived (e.g.
    /// some Sliding Sync implementations do not echo own events into the timeline),
    /// so edit/redact by event id can fall back to the transaction identity.
    pub(super) sent_event_txns: HashMap<String, matrix_sdk::ruma::OwnedTransactionId>,
    /// event_id -> SDK media source. This cache may contain encrypted media
    /// keys/hashes and must never be serialized or logged.
    pub(super) media_sources: HashMap<String, PrivateMediaEntry>,
    /// Event IDs for which a download is currently in flight. Prevents duplicate
    /// concurrent downloads when the user clicks an attachment repeatedly.
    pub(super) media_downloads_in_progress: HashSet<String>,
    /// In-flight media download workers keyed by event id; aborted on actor drop.
    pub(super) media_download_tasks: HashMap<String, executor::JoinHandle<()>>,
    /// Search index mutation sender (Phase 6). `None` when no search index is
    /// configured (pre-session or pre-Phase-6 builds). Fire-and-forget: if the
    /// channel is full, we drop the mutation rather than block the diff relay.
    pub(super) search_index_tx: Option<mpsc::Sender<crate::search::SearchIndexMessage>>,
    /// Rust-owned pane-level thread attention read-state tracker. Only thread
    /// timelines update it, and React reads its projection through
    /// `AppState.thread_attention`.
    pub(super) thread_attention: ThreadAttentionTracker,
    /// Rust-owned navigation projection source. The webview reports viewport
    /// facts; item ordering, unread marker semantics, and counts stay here.
    pub(super) navigation_items: Vec<TimelineItem>,
    /// Canonical-slot membership plus the normalized bounded sequence last
    /// emitted to the Room UI. SDK indices are translated through this state;
    /// raw navigation indices never reach `ItemsUpdated`.
    pub(super) display_projection: DisplayProjectionState,
    pub(super) media_gallery_items: Vec<TimelineMediaGalleryItem>,
    pub(super) fully_read_event_id: Option<String>,
    pub(super) server_confirmed_read_event_id: Option<String>,
    pub(super) local_viewed_boundary: Option<LocalViewedBoundary>,
    pub(super) read_state_sync: TimelineReadStateSync,
    pub(super) send_read_receipts: bool,
    pub(super) viewport_observation: TimelineViewportObservation,
    pub(super) last_navigation_snapshot: Option<TimelineNavigationSnapshot>,
    pub(super) ignored_user_ids: std::collections::BTreeSet<String>,
    /// URL preview policy for this timeline.
    pub(super) link_preview_policy: LinkPreviewContext,
    /// In-flight URL preview fetch workers keyed by event_id.
    pub(super) link_preview_fetches: HashMap<String, executor::JoinHandle<()>>,
    /// In-flight reply detail fetch workers keyed by the reply event_id.
    pub(super) reply_detail_fetches: HashMap<String, executor::JoinHandle<()>>,
    /// Manager-owned bounded hydration state shared by replacement Room
    /// actors. This is not a `Timeline` and cannot paginate.
    pub(super) thread_root_projection_service: Arc<Mutex<ThreadRootProjectionService>>,
    pub(super) thread_summary_projection: ThreadSummaryProjectionIngress,
    thread_summary_projection_rx: watch::Receiver<BTreeMap<String, ThreadSummaryProjectionWake>>,
    pub(super) thread_root_order: TimelineThreadRootOrder,
    /// Manager-owned serial fence for display events and their actor generation.
    pub(super) timeline_actor_generations: Arc<TimelineActorGenerationGate>,
    pub(super) actor_generation: u64,
    pub(super) subscription_generation: Option<u64>,
    pub(super) room_subscription_checkpoint: Option<MatrixRoomSubscriptionCheckpoint>,
    pub(super) deferred_room_subscription_checkpoint: Option<MatrixRoomSubscriptionCheckpoint>,
    pub(super) global_commit_fence: GlobalCommitFence,
    pub(super) missing_committed_response_retry: Option<(u64, u64)>,
    /// Bounded root hydration workers are manager-owned so their completion is
    /// ordered with unsubscribe/shutdown lifecycle commands.
    pub(super) manager_tx: mpsc::Sender<TimelineMessage>,
    /// Stable manager-owned FIFO for send terminal ownership transfer. Unlike
    /// `manager_tx`, admission never awaits actor mailbox capacity.
    pub(super) terminal_ingress: TimelineSendTerminalIngress,
    /// Reply event IDs already handed to the SDK for replied-to details during
    /// this actor lifetime. This avoids retry loops on every viewport tick.
    pub(super) reply_detail_fetch_attempted_event_ids: HashSet<String>,
    pub(super) pagination_task: Option<ActivePaginationTask>,
    pub(super) next_pagination_serial: u64,
    /// Application data directory for cached preview images.
    pub(super) data_dir: Option<std::path::PathBuf>,
    pub(super) account_work: AccountWorkScheduler,
    pub(super) restore_anchor: Option<RestoreTimelineAnchorState>,
    pub(super) next_restore_anchor_serial: u64,
    /// Buffered `TimelineDiff`s accumulated during a restore walk. While
    /// `restore_anchor.is_some()`, each `handle_diff_batch` call appends its
    /// projected display-space diffs here instead of emitting `ItemsUpdated`
    /// per chunk. Canonical state advances immediately; the
    /// buffer is flushed as ONE `ItemsUpdated` when the restore terminates
    /// (Found/EndReached/BudgetExhausted/Failed/Superseded), so React receives
    /// a single settled update rather than O(chunks) intermediate renders.
    pub(super) restore_emit_buffer: Vec<TimelineDiff>,
    /// Causal SDK projection tags for the diffs above. They must be observed
    /// only after the coalesced restore batch is published to the UI.
    pub(super) restore_causal_projections: RestoreCausalProjectionBuffer,
    /// `maybe_hydrate_missing_thread_roots` is deliberately deferred until a
    /// buffered restore window has emitted its final canonical `ItemsUpdated`.
    /// Otherwise a newly observed Pending projection is pruned by the store
    /// before the reply that owns it is visible.
    pub(super) hydrate_after_restore_flush: bool,
    /// Monotonically increasing counter, incremented at the start of every
    /// `handle_diff_batch` call (restore or not).
    pub(super) diff_batch_seq: u64,
    pub(super) gap_repair: TimelineGapRepairTracker,
    pub(super) gap_projection_correlation: TimelineGapProjectionCorrelation,
    pub(super) pending_gap_projection: Option<PendingTimelineGapProjection>,
    /// Historical projection released by a coalesced restore publication.
    /// The synchronous flush records the exact published batch; the actor loop
    /// performs the existing async scheduler handoff immediately afterwards.
    pub(super) ready_restore_gap_projection_batch: Option<TimelineBatchId>,
    pub(super) live_tail_projection_correlation: TimelineGapProjectionCorrelation,
    pub(super) pending_live_tail_projection: Option<PendingLiveTailRefreshCompletion>,
    pub(super) live_tail_refresh: Option<(
        u64,
        MatrixLiveTailRefreshCancellation,
        executor::JoinHandle<()>,
    )>,
    pub(super) gap_work_task: Option<executor::JoinHandle<()>>,
    pub(super) gap_relay_settlement_task: Option<executor::JoinHandle<()>>,
    #[cfg(test)]
    pub(super) test_gap_repair_completion_pause: Option<TestGapRepairCompletionPause>,
    last_gap_repair_evaluation_diagnostic: Option<GapRepairEvaluationDiagnosticSignature>,
    /// Diagnostic-only evidence that this actor received foreground repair demand.
    /// It deliberately does not affect repair admission or actor lifecycle.
    pub(super) foreground_gap_demand_active: bool,
    pub(super) decrypt_retry: DecryptRetryController,
    pub(super) decrypt_retry_timeout_task: Option<executor::JoinHandle<()>>,
    /// Standard-only room-key recovery operations (issue #478), keyed by the
    /// Megolm session id internally (never exported).
    pub(super) room_key_recovery:
        std::collections::BTreeMap<String, super::recovery_model::RecoveryOperation>,
    /// Per-event room-key request presentation state (issue #460), keyed by
    /// event id (already visible timeline correlation).
    pub(super) key_request_states: std::collections::BTreeMap<String, KeyRequestUiState>,
    /// Issue #460: automatic key-request candidates whose try_send hit a full
    /// mailbox; retried on the next actor loop iteration.
    pub(super) pending_auto_key_requests: Vec<String>,
    /// Closed `m.room_key.withheld` codes per (room, session) observed in this
    /// timeline's room (issue #460).
    pub(super) withheld_codes: std::collections::BTreeMap<(String, String), &'static str>,
    pub(super) next_session_alias: u64,
    pub(super) recovery_tick_tasks: std::collections::BTreeMap<String, executor::JoinHandle<()>>,
}

impl Drop for TimelineActor {
    fn drop(&mut self) {
        if let Some(task) = self.relay_task.take() {
            task.abort();
        }
        if let Some(task) = self.relay_restart_task.take() {
            task.abort();
        }
        for task in self.link_preview_fetches.values() {
            task.abort();
        }
        for task in self.reply_detail_fetches.values() {
            task.abort();
        }
        for task in self.media_download_tasks.values() {
            task.abort();
        }
        if let Some(active) = self.pagination_task.take() {
            active.task.abort();
        }
        if let Some(task) = self.gap_work_task.take() {
            task.abort();
        }
        if let Some(task) = self.gap_relay_settlement_task.take() {
            task.abort();
        }
        if let Some((_, cancellation, task)) = self.live_tail_refresh.take() {
            cancellation.cancel();
            drop(task);
        }
        if let Some(task) = self.decrypt_retry_timeout_task.take() {
            task.abort();
        }
    }
}

fn canonical_pending_event_ids(
    projections: &[PendingSendProjection],
    canonical_items: &[TimelineItem],
) -> Vec<String> {
    projections
        .iter()
        .filter_map(|projection| projection.terminal_event_id.as_deref())
        .filter(|event_id| {
            canonical_items.iter().any(|item| {
                matches!(&item.id, TimelineItemId::Event { event_id: canonical } if canonical == event_id)
            })
        })
        .map(str::to_owned)
        .collect()
}

impl TimelineActor {
    pub(super) fn display_projection_context(&self) -> DisplayProjectionContext {
        DisplayProjectionContext::for_timeline(
            &self.key.kind,
            &self.viewport_observation,
            self.restore_anchor.is_some(),
        )
        .with_thread_roots(
            self.thread_root_order,
            self.thread_root_projection_service
                .lock()
                .expect("thread-root projection service lock must not be poisoned")
                .display_data_for_room(self.key.room_id()),
        )
    }

    pub(super) fn reproject_display_items(&mut self) -> Vec<TimelineDiff> {
        let context = self.display_projection_context();
        self.display_projection.reproject(&context)
    }

    pub(super) async fn refresh_pending_send_projection(
        &mut self,
        actor_generation: u64,
        mut projections: Vec<PendingSendProjection>,
    ) -> bool {
        if actor_generation != self.actor_generation
            || self
                .timeline_actor_generations
                .current_generation(&self.key)
                != Some(self.actor_generation)
        {
            return false;
        }
        let converged_event_ids = canonical_pending_event_ids(&projections, &self.navigation_items);
        if !converged_event_ids.is_empty() {
            let mut coordinator = self
                .send_completion
                .lock()
                .expect("send completion coordinator lock must not be poisoned");
            for event_id in converged_event_ids {
                coordinator.reconcile_remote_event(self.key.room_id(), &event_id);
            }
            projections = coordinator.projections_for_key(&self.key);
        }
        let settled_transaction_ids = self
            .send_completion
            .lock()
            .expect("send completion coordinator lock must not be poisoned")
            .settled_transaction_ids(self.key.room_id());
        self.pending_send_projections = projections;
        let pending_items = self
            .pending_send_projections
            .iter()
            .map(|projection| projection.item.clone())
            .collect();
        let mut suppressed = self
            .pending_send_projections
            .iter()
            .filter(|projection| {
                matches!(
                    projection.phase,
                    super::outbound_send::PendingSendPhase::SentAwaitingRemote
                        | super::outbound_send::PendingSendPhase::HydratedSent
                )
            })
            .filter_map(|projection| projection.sdk_transaction_id.clone())
            .collect::<std::collections::HashSet<_>>();
        suppressed.extend(settled_transaction_ids);
        let diffs = self.display_projection.replace_pending(
            pending_items,
            suppressed,
            &self.display_projection_context(),
        );
        if !diffs.is_empty() {
            let batch_id = self.next_batch_id;
            if super::navigation::emit_items_updated_for_generation(
                &self.event_tx,
                &self.timeline_actor_generations,
                &self.key,
                self.actor_generation,
                self.generation,
                batch_id,
                diffs,
            ) {
                self.next_batch_id = TimelineBatchId(batch_id.0 + 1);
            } else {
                return false;
            }
        }
        true
    }

    fn drain_thread_summary_projection_wakes(&mut self) {
        if self
            .timeline_actor_generations
            .current_generation(&self.key)
            != Some(self.actor_generation)
        {
            return;
        }
        for wake in self
            .thread_summary_projection
            .drain(&mut self.thread_summary_projection_rx)
        {
            let (root_event_id, activity_revision, summary_revision, cleared) = match wake {
                ThreadSummaryProjectionWake::Updated {
                    root_event_id,
                    activity_revision,
                    summary_revision,
                } => (root_event_id, activity_revision, summary_revision, false),
                ThreadSummaryProjectionWake::Cleared {
                    root_event_id,
                    activity_revision,
                    summary_revision,
                } => (root_event_id, activity_revision, summary_revision, true),
            };
            let service = self
                .thread_root_projection_service
                .lock()
                .expect("thread-root projection service lock must not be poisoned");
            let current = service.display_data_at_revision(
                self.key.room_id(),
                &root_event_id,
                activity_revision,
                summary_revision,
            );
            drop(service);
            if cleared && current.is_some() || !cleared && current.is_none() {
                continue;
            }

            let mut canonical_set = None;
            if let Some(aggregate) = self
                .thread_root_projection_service
                .lock()
                .expect("thread-root projection service lock must not be poisoned")
                .aggregate_at_revision(
                    self.key.room_id(),
                    &root_event_id,
                    activity_revision,
                    summary_revision,
                )
                && let Some((index, item)) =
                    self.navigation_items.iter().enumerate().find(|(_, item)| {
                        matches!(
                            &item.id,
                            TimelineItemId::Event { event_id }
                                if event_id == &root_event_id && item.thread_root.is_none()
                        )
                    })
            {
                let next = thread_root_item_with_authoritative_aggregate(item, &aggregate);
                if *item != next {
                    self.navigation_items[index] = next.clone();
                    canonical_set = Some((index, next));
                }
            }

            let canonical_set_applied = canonical_set.is_some();
            let display_diffs = if let Some((index, item)) = canonical_set {
                let context = self.display_projection_context();
                super::display_projection::apply_non_sdk_item_set_diffs_to_display_items(
                    &mut self.display_projection,
                    &[TimelineDiff::Set { index, item }],
                    &context,
                )
            } else {
                self.reproject_display_items()
            };
            if display_diffs.is_empty() {
                continue;
            }
            let batch_id = self.next_batch_id;
            if super::navigation::emit_items_updated_for_generation(
                &self.event_tx,
                &self.timeline_actor_generations,
                &self.key,
                self.actor_generation,
                self.generation,
                batch_id,
                display_diffs,
            ) {
                self.next_batch_id = TimelineBatchId(batch_id.0 + 1);
                if canonical_set_applied {
                    self.emit_navigation_if_changed();
                }
            }
        }
    }

    pub(super) async fn publish_thread_summary_window_observations(
        &self,
        before: &[TimelineItem],
        after: &[TimelineItem],
    ) -> bool {
        for observation in thread_summary_observations_for_windows(&self.key, before, after) {
            if self
                .manager_tx
                .send(TimelineMessage::ThreadSummaryActivityObserved {
                    key: self.key.clone(),
                    actor_generation: self.actor_generation,
                    observation,
                })
                .await
                .is_err()
            {
                return false;
            }
        }
        true
    }

    async fn publish_current_canonical_activity(&self) {
        let Some(activity_permit) =
            reserve_canonical_activity_action(&self.action_tx, &self.key).await
        else {
            return;
        };
        let Some(commit_lease) = self
            .timeline_actor_generations
            .try_acquire(&self.key, self.actor_generation)
        else {
            drop(activity_permit);
            return;
        };
        if let Some(action) = canonical_activity_window_action(&self.key, &self.navigation_items) {
            activity_permit.send(vec![action]);
        }
        drop(commit_lease);
    }

    /// Spawn the actor, emit InitialItems, and return the handle.
    pub(super) async fn spawn(
        key: TimelineKey,
        timeline: Arc<Timeline>,
        session: Arc<MatrixClientSession>,
        subscribe_request_id: RequestId,
        send_read_receipts: bool,
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
        search_index_tx: Option<mpsc::Sender<crate::search::SearchIndexMessage>>,
        ignored_user_ids: std::collections::BTreeSet<String>,
        data_dir: Option<std::path::PathBuf>,
        link_preview_policy: LinkPreviewContext,
        account_work: AccountWorkScheduler,
        thread_root_projection_service: Arc<Mutex<ThreadRootProjectionService>>,
        thread_root_order: TimelineThreadRootOrder,
        timeline_actor_generations: Arc<TimelineActorGenerationGate>,
        actor_generation: u64,
        subscription_generation: Option<u64>,
        send_completion: SharedSendCompletionCoordinator,
        terminal_ingress: TimelineSendTerminalIngress,
        manager_tx: mpsc::Sender<TimelineMessage>,
    ) -> TimelineActorHandle {
        let mut auxiliary_tasks: Vec<executor::JoinHandle<()>> = Vec::new();

        // Subscribe the event cache BEFORE the timeline load so the initial
        // load's provenance (store=cache vs network) is observed. The helper
        // decides whether to mirror the observation to stderr.
        {
            if let Ok(parsed_room_id) = matrix_sdk::ruma::RoomId::parse(key.room_id()) {
                if let Some(observer_room) = session.client().get_room(&parsed_room_id) {
                    if let Ok((cache, drop_guards)) = observer_room.event_cache().await {
                        if let Ok((initial, mut updates)) = cache.subscribe().await {
                            if !initial.is_empty() {
                                // Cache already had events at restore: warm initial state.
                                startup_trace::trace_origin("cache");
                            }
                            trace_event_cache_items("cache_initial", &key, &initial);
                            let trace_key = key.clone();
                            auxiliary_tasks.push(executor::spawn(async move {
                                let _event_cache_drop_guards = drop_guards;
                                use matrix_sdk::event_cache::RoomEventCacheUpdate;
                                loop {
                                    match updates.recv().await {
                                        Ok(RoomEventCacheUpdate::UpdateTimelineEvents(diffs)) => {
                                            startup_trace::trace_origin(
                                                event_cache_origin_trace_token(&diffs.origin),
                                            );
                                            trace_event_cache_diffs(
                                                "cache_update",
                                                &trace_key,
                                                &diffs.origin,
                                                &diffs.diffs,
                                            );
                                        }
                                        Ok(_) => {}
                                        // Broadcast lagged or channel closed — stop the observer.
                                        Err(_) => break,
                                    }
                                }
                            }));
                        }
                    }
                }
            }
        }

        // Subscribe to the SDK timeline to get initial items + diff stream.
        let subscribe_started = Some(startup_trace::now());
        let (mut initial_sdk_items, mut diff_stream) = timeline.subscribe().await;
        startup_trace::trace_phase_items(
            StartupPhase::TimelineSubscribe,
            subscribe_started,
            initial_sdk_items.len(),
        );
        if should_hydrate_empty_initial_room_timeline(&key.kind, initial_sdk_items.len()) {
            let gate_started = Some(std::time::Instant::now());
            let hydrate_result = {
                let _permit = account_work
                    .acquire(AccountWorkKind::ExplicitPagination)
                    .await;
                let gate_wait = gate_started.map(|started| started.elapsed());
                trace_timeline_paginate(
                    "initial_hydrate_gate_acquired",
                    subscribe_request_id,
                    &key,
                    PaginationDirection::Backward,
                    INITIAL_EMPTY_ROOM_BACKFILL_EVENT_COUNT,
                    None,
                    gate_wait.map(|duration| duration.as_millis()),
                    None,
                );
                let paginate_started = Some(startup_trace::now());
                let trace_started = Some(std::time::Instant::now());
                let outcome = timeline
                    .paginate_backwards(INITIAL_EMPTY_ROOM_BACKFILL_EVENT_COUNT)
                    .await;
                let outcome_token = match &outcome {
                    Ok(true) => "end_reached",
                    Ok(false) => "idle",
                    Err(_) => "failed",
                };
                trace_timeline_paginate(
                    "initial_hydrate_sdk_finish",
                    subscribe_request_id,
                    &key,
                    PaginationDirection::Backward,
                    INITIAL_EMPTY_ROOM_BACKFILL_EVENT_COUNT,
                    trace_started.map(|started| started.elapsed().as_millis()),
                    gate_wait.map(|duration| duration.as_millis()),
                    Some(outcome_token),
                );
                startup_trace::trace_paginate(
                    paginate_started,
                    gate_wait,
                    matches!(outcome, Ok(true)),
                );
                outcome
            };
            if hydrate_result.is_ok() {
                let resubscribe_started = Some(startup_trace::now());
                let (hydrated_items, hydrated_stream) = timeline.subscribe().await;
                startup_trace::trace_phase_items(
                    StartupPhase::TimelineSubscribe,
                    resubscribe_started,
                    hydrated_items.len(),
                );
                initial_sdk_items = hydrated_items;
                diff_stream = hydrated_stream;
            }
        }
        let own_user_id = session.client().user_id().map(|user_id| user_id.to_owned());
        let mut initial_read_receipt_changes = if own_user_id.is_some() {
            Some(timeline.subscribe_own_user_read_receipts_changed().await)
        } else {
            None
        };
        let initial_read_receipt_event_id = match own_user_id.as_deref() {
            Some(own_user_id) => timeline
                .latest_user_read_receipt_timeline_event_id(own_user_id)
                .await
                .map(|event_id| event_id.to_string()),
            None => None,
        };
        let room_id = key.room_id().to_owned();

        let mut media_sources = HashMap::new();
        for item in &initial_sdk_items {
            cache_sdk_item_media_source(&mut media_sources, item);
        }

        let initial_items: Vec<TimelineItem> = initial_sdk_items
            .iter()
            .map(|item| sdk_item_to_timeline_item(&key, item, own_user_id.as_deref()))
            .map(|mut item| {
                apply_ignored_sender_suppression(&mut item, &ignored_user_ids);
                item
            })
            .collect();
        let mut initial_items = initial_items;
        for item in &mut initial_items {
            apply_link_previews_to_item(&mut *item, &room_id, &link_preview_policy, &session).await;
        }
        trace_timeline_items("initial", &key, &initial_items);
        let navigation_items = initial_items.clone();
        for item in &navigation_items {
            super::thread_projection::seed_thread_summary_item(
                &thread_root_projection_service,
                &key,
                item,
            );
        }
        let mut display_projection = DisplayProjectionState::from_canonical_window(
            &navigation_items,
            0..navigation_items.len(),
        );
        let initial_display_context = DisplayProjectionContext::for_timeline(
            &key.kind,
            &TimelineViewportObservation::default(),
            false,
        )
        .with_thread_roots(
            thread_root_order,
            thread_root_projection_service
                .lock()
                .expect("thread-root projection service lock must not be poisoned")
                .display_data_for_room(key.room_id()),
        );
        let (pending_send_projections, settled_transaction_ids) = {
            let coordinator = send_completion
                .lock()
                .expect("send completion coordinator lock must not be poisoned");
            (
                coordinator.projections_for_key(&key),
                coordinator.settled_transaction_ids(key.room_id()),
            )
        };
        let pending_items = pending_send_projections
            .iter()
            .map(|projection| projection.item.clone())
            .collect();
        let mut suppressed = pending_send_projections
            .iter()
            .filter(|projection| {
                matches!(
                    projection.phase,
                    super::outbound_send::PendingSendPhase::SentAwaitingRemote
                        | super::outbound_send::PendingSendPhase::HydratedSent
                )
            })
            .filter_map(|projection| projection.sdk_transaction_id.clone())
            .collect::<std::collections::HashSet<_>>();
        suppressed.extend(settled_transaction_ids);
        display_projection.replace_pending(pending_items, suppressed, &initial_display_context);
        let initial_media_gallery_items =
            media_gallery_items_from_timeline_items(&key, &initial_items);
        let initial_receipts = live_event_receipts_from_sdk_items(initial_sdk_items.iter());

        let (actor_tx, actor_rx) = mpsc::channel(256);
        let (actor_control_tx, actor_control_rx) =
            mpsc::channel(TIMELINE_ACTOR_CONTROL_QUEUE_CAPACITY);
        let (thread_summary_projection, thread_summary_projection_rx) =
            ThreadSummaryProjectionIngress::channel();
        let (actor_cleanup_tx, actor_cleanup_rx) = TimelineActorCleanupIngress::channel();
        let (relay_control_tx, relay_control_rx) = mpsc::channel(1);
        let (relay_data_tx, relay_data_rx) = mpsc::channel(256);
        let mut send_statuses = HashMap::new();
        let mut send_handles = HashMap::new();
        if let Some(mut receipt_changes) = initial_read_receipt_changes.take() {
            use futures_util::StreamExt;
            let receipt_tx = actor_tx.clone();
            auxiliary_tasks.push(executor::spawn(async move {
                while receipt_changes.next().await.is_some() {
                    if receipt_tx
                        .send(TimelineActorMessage::OwnReadReceiptChanged)
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }));
        }

        // Room-key withheld observer (issue #460): capture standard
        // m.room_key.withheld codes for this room into closed presentation
        // tokens, so the UI can show localized refusal copy.
        {
            let withheld_tx = actor_tx.clone();
            let withheld_room = key.room_id().to_owned();
            let withheld_session = session.clone();
            auxiliary_tasks.push(executor::spawn(async move {
                // Establish the broadcast subscription BEFORE reading the
                // stored snapshot: the underlying channel does not replay, so
                // an observation between the snapshot and the subscription
                // would otherwise be lost from both sources. The wrapper
                // subscribes eagerly; no timeout is needed (the task is
                // detached and never awaited by actor construction).
                let mut stream =
                    Box::pin(koushi_sdk::room_key_withheld_stream(&withheld_session).await);
                // Seed from stored withheld state (duplicates are idempotent
                // in the observer handler).
                for (session_id, code) in
                    koushi_sdk::room_key_withheld_codes(&withheld_session, &withheld_room).await
                {
                    let _ = withheld_tx
                        .send(TimelineActorMessage::RoomKeyWithheldObserved {
                            room_id: withheld_room.clone(),
                            session_id,
                            code: code.token(),
                        })
                        .await;
                }
                while let Some(batch) = stream.next().await {
                    for (room_id, session_id, code) in batch {
                        if room_id != withheld_room {
                            continue;
                        }
                        if withheld_tx
                            .send(TimelineActorMessage::RoomKeyWithheldObserved {
                                room_id,
                                session_id,
                                code: code.token(),
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }));
        }

        // Late-decryption reports observer (#476): when the event-cache
        // redecryptor reports lag or backup availability, drive the bounded
        // local retry for this timeline's visible UTD sessions.
        {
            use futures_util::StreamExt;
            let late_decryption_tx = actor_tx.clone();
            let late_decryption_reports = koushi_sdk::late_decryption_report_stream(&session);
            auxiliary_tasks.push(executor::spawn(async move {
                use matrix_sdk::event_cache::RedecryptorReport;
                let mut reports = late_decryption_reports;
                let mut last_sent = std::time::Instant::now()
                    - crate::room_key_receive::LATE_DECRYPTION_RETRY_COALESCE_WINDOW;
                while let Some(report) = reports.next().await {
                    let Ok(report) = report else { continue };
                    let trigger = match &report {
                        RedecryptorReport::Lagging => {
                            crate::room_key_receive::RECEIVE_SUMMARY_TRIGGER_STREAM_LAGGED
                        }
                        RedecryptorReport::BackupAvailable => {
                            crate::room_key_receive::RECEIVE_SUMMARY_TRIGGER_BACKUP_AVAILABLE
                        }
                        RedecryptorReport::ResolvedUtds { .. } => continue,
                    };
                    if last_sent.elapsed()
                        < crate::room_key_receive::LATE_DECRYPTION_RETRY_COALESCE_WINDOW
                    {
                        continue;
                    }
                    last_sent = std::time::Instant::now();
                    if late_decryption_tx
                        .send(TimelineActorMessage::RequestLateDecryption {
                            request_id: None,
                            trigger,
                        })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }));
        }

        // Emit InitialItems (generation 0) from the actor-owned display
        // projection. Navigation items remain canonical and are used for the
        // Activity mirror below.
        let generation = TimelineGeneration(0);
        let initial_display_items = display_projection.display_items().to_vec();
        let initial_emitted = emit_initial_items_for_generation(
            &event_tx,
            &timeline_actor_generations,
            &key,
            actor_generation,
            InitialItemsRequestIdentity::fresh(subscribe_request_id),
            generation,
            initial_display_items,
            Vec::new(),
        );
        record_subscribe_stage(
            if initial_emitted {
                "initial_emitted"
            } else {
                "initial_rejected_stale_generation"
            },
            Some(initial_items.len()),
        );
        if initial_emitted
            && let Some(activity_permit) = reserve_canonical_activity_action(&action_tx, &key).await
        {
            if let Some(action) = canonical_activity_window_action(&key, &navigation_items) {
                activity_permit.send(vec![action]);
            }
        }
        if initial_emitted
            && let Some(action) = thread_activity_observed_action(&key, &navigation_items)
        {
            let _ = action_tx.send(vec![action]).await;
        }
        if let Some(action) =
            media_gallery_updated_action(&key, initial_media_gallery_items.clone())
        {
            let _ = action_tx.send(vec![action]).await;
        }

        // Spawn the diff relay task: converts SDK VectorDiff stream into actor messages.
        let initial_items: Vec<_> = initial_sdk_items.iter().cloned().collect();
        let relay_task = Some(executor::spawn(run_diff_relay(
            relay_data_tx,
            relay_control_tx.clone(),
            generation,
            actor_generation,
            diff_stream,
            initial_items,
        )));
        if should_fetch_members(&key.kind) {
            let timeline = Arc::clone(&timeline);
            auxiliary_tasks.push(executor::spawn(async move {
                timeline.fetch_members().await;
            }));
        }

        // Spawn the send queue monitor task: forwards RoomSendQueueUpdate to actor.
        let room_id_str = match &key.kind {
            TimelineKind::Room { room_id }
            | TimelineKind::Thread { room_id, .. }
            | TimelineKind::Focused { room_id, .. } => room_id.clone(),
        };
        let mut initial_fully_read_event_id = None;
        if let Ok(room_id) = matrix_sdk::ruma::RoomId::parse(&room_id_str) {
            if let Some(room) = session.client().get_room(&room_id) {
                let sq_tx = actor_tx.clone();
                if let Ok((local_echoes, update_rx)) = room.send_queue().subscribe().await {
                    for echo in &local_echoes {
                        remember_local_echo(&mut send_statuses, &mut send_handles, echo);
                    }
                    auxiliary_tasks.push(executor::spawn(run_send_queue_monitor(sq_tx, update_rx)));
                }

                let (typing_guard, typing_rx) = room.subscribe_to_typing_notifications();
                let typing_tx = actor_tx.clone();
                auxiliary_tasks.push(executor::spawn(run_typing_notifications(
                    typing_tx,
                    typing_guard,
                    typing_rx,
                )));

                let room_id = room_id_str.clone();
                if initial_emitted && !initial_receipts.is_empty() {
                    let _ = emit_receipt_observation_actions(
                        session.as_ref(),
                        &action_tx,
                        &timeline_actor_generations,
                        &key,
                        actor_generation,
                        &room_id,
                        initial_receipts,
                        ReceiptObservationTarget::Live,
                    )
                    .await;
                }
                let _ = action_tx
                    .send(vec![AppAction::FullyReadMarkerUpdated {
                        room_id,
                        event_id: {
                            initial_fully_read_event_id = room
                                .fully_read_event_id()
                                .map(|event_id| event_id.to_string());
                            initial_fully_read_event_id.clone()
                        },
                    }])
                    .await;
            }
        }

        let initial_server_confirmed_read_event_id = match &key.kind {
            TimelineKind::Thread { .. } => initial_read_receipt_event_id.clone(),
            TimelineKind::Room { .. } | TimelineKind::Focused { .. } => {
                initial_fully_read_event_id.clone()
            }
        };
        let thread_attention = ThreadAttentionTracker::hydrate(
            &key,
            &navigation_items,
            own_user_id.as_ref().map(|user_id| user_id.as_str()),
            initial_read_receipt_event_id,
        );
        if thread_attention.counts != ThreadAttentionCounters::default() {
            if let Some(action) = thread_attention_action(thread_attention.counts, &key) {
                let _ = action_tx.send(vec![action]).await;
            }
        }
        let mut gap_repair = TimelineGapRepairTracker::default();
        if matches!(key.kind, TimelineKind::Room { .. }) {
            gap_repair.observe_live_edge_target(rendered_live_edge_target(&navigation_items));
        }
        let enqueue_context =
            TimelineSendEnqueueContext::Matrix(MatrixTimelineSendEnqueueContext {
                key: key.clone(),
                timeline: Arc::clone(&timeline),
                session: Arc::clone(&session),
                cleanup: actor_cleanup_tx,
                diagnostic_trace: None,
            });
        let (position_tx, position_rx) = watch::channel(Arc::new(
            TimelinePositionIndex::from_items(actor_generation, generation, &navigation_items),
        ));
        let mut actor = TimelineActor {
            key: key.clone(),
            timeline,
            session,
            action_tx,
            event_tx,
            msg_tx: actor_tx.clone(),
            msg_rx: actor_rx,
            control_rx: actor_control_rx,
            cleanup_rx: actor_cleanup_rx,
            last_end_gap_demand_cleanup_serial: 0,
            last_cancel_network_cleanup_serial: 0,
            last_cancel_pagination_cleanup_serial: 0,
            last_cancel_link_previews_cleanup_serial: 0,
            position_tx,
            relay_control_tx,
            relay_control_rx,
            relay_data_rx: Some(relay_data_rx),
            relay_task,
            relay_restart_backoff: RelayRestartBackoff::new(
                RELAY_RESTART_BASE_DELAY,
                RELAY_RESTART_MAX_DELAY,
            ),
            relay_restart_task: None,
            generation,
            projection_request_id: subscribe_request_id,
            initial_projection_committed: initial_emitted,
            next_batch_id: TimelineBatchId(0),
            send_completion: Arc::clone(&send_completion),
            send_statuses,
            send_handles,
            pending_send_projections,
            own_user_id,
            sent_event_txns: HashMap::new(),
            media_sources,
            media_downloads_in_progress: HashSet::new(),
            media_download_tasks: HashMap::new(),
            search_index_tx,
            thread_attention,
            navigation_items,
            display_projection,
            media_gallery_items: initial_media_gallery_items,
            fully_read_event_id: initial_fully_read_event_id,
            server_confirmed_read_event_id: initial_server_confirmed_read_event_id,
            local_viewed_boundary: None,
            read_state_sync: TimelineReadStateSync::Synced,
            send_read_receipts,
            viewport_observation: TimelineViewportObservation::default(),
            last_navigation_snapshot: None,
            ignored_user_ids,
            link_preview_policy,
            link_preview_fetches: HashMap::new(),
            reply_detail_fetches: HashMap::new(),
            thread_root_projection_service,
            thread_summary_projection: thread_summary_projection.clone(),
            thread_summary_projection_rx,
            thread_root_order,
            timeline_actor_generations,
            actor_generation,
            subscription_generation,
            room_subscription_checkpoint: None,
            deferred_room_subscription_checkpoint: None,
            global_commit_fence: GlobalCommitFence::default(),
            missing_committed_response_retry: None,
            manager_tx,
            terminal_ingress,
            reply_detail_fetch_attempted_event_ids: HashSet::new(),
            pagination_task: None,
            next_pagination_serial: 0,
            data_dir,
            account_work,
            restore_anchor: None,
            next_restore_anchor_serial: 0,
            restore_emit_buffer: Vec::new(),
            restore_causal_projections: RestoreCausalProjectionBuffer::default(),
            hydrate_after_restore_flush: false,
            diff_batch_seq: 0,
            gap_repair,
            gap_projection_correlation: TimelineGapProjectionCorrelation::default(),
            pending_gap_projection: None,
            ready_restore_gap_projection_batch: None,
            live_tail_projection_correlation: TimelineGapProjectionCorrelation::default(),
            pending_live_tail_projection: None,
            live_tail_refresh: None,
            gap_work_task: None,
            gap_relay_settlement_task: None,
            #[cfg(test)]
            test_gap_repair_completion_pause: None,
            last_gap_repair_evaluation_diagnostic: None,
            foreground_gap_demand_active: false,
            decrypt_retry: DecryptRetryController::default(),
            decrypt_retry_timeout_task: None,
            room_key_recovery: Default::default(),
            next_session_alias: 0,
            recovery_tick_tasks: Default::default(),
            key_request_states: Default::default(),
            pending_auto_key_requests: Vec::new(),
            withheld_codes: Default::default(),
        };

        actor
            .forward_initial_items_to_search(initial_sdk_items.iter().cloned())
            .await;
        // Issue #460: automatic one-shot key requests for Thread timelines at
        // subscription time — existing UTD rows are delivered as InitialItems,
        // not as diffs, so the diff-batch scanner never sees them. The actor
        // processes these after startup; mailbox-full candidates are retained
        // in the pending set and retried by the run loop. The automatic guard
        // in handle_request_room_key refuses repeats for already-requested
        // events.
        if matches!(key.kind, TimelineKind::Thread { .. }) {
            let initial_candidates: Vec<String> = initial_sdk_items
                .iter()
                .filter_map(thread_auto_requestable_event_id)
                .collect();
            actor.dispatch_auto_key_requests(initial_candidates);
        }
        let task = executor::spawn(actor.run());

        TimelineActorHandle {
            tx: actor_tx,
            control_tx: Some(actor_control_tx),
            thread_summary_projection,
            position_rx: Some(position_rx),
            task: Some(task),
            auxiliary_tasks,
            subscription_generation,
            enqueue_context: Some(enqueue_context),
        }
    }
    async fn run(mut self) {
        // This must run only after `spawn` has returned the handle to the
        // manager. Publishing through the manager mailbox while construction
        // is still awaited can self-deadlock when that bounded mailbox is full.
        self.publish_authoritative_read_state().await;
        if matches!(self.key.kind, TimelineKind::Room { .. }) {
            self.maybe_hydrate_missing_thread_roots(None).await;
        }
        if matches!(self.key.kind, TimelineKind::Thread { .. }) {
            let initial_items = self.navigation_items.clone();
            let _ = self
                .publish_thread_summary_window_observations(&[], &initial_items)
                .await;
        }
        let initial_trigger = if self.gap_repair.has_live_edge_target() {
            TimelineGapRepairTrigger::LiveEdge
        } else {
            TimelineGapRepairTrigger::Automatic
        };
        self.gap_repair.queue_inspection(initial_trigger);
        loop {
            // Issue #460: retry automatic key-request candidates that were
            // deferred when the mailbox was full (deduplicated by the
            // key_request_states guard inside dispatch_auto_key_requests).
            if !self.pending_auto_key_requests.is_empty() {
                let pending = std::mem::take(&mut self.pending_auto_key_requests);
                self.dispatch_auto_key_requests(pending);
            }
            self.drain_thread_summary_projection_wakes();
            tokio::select! {
                biased;
                summary_projection = self.thread_summary_projection_rx.changed() => {
                    if summary_projection.is_err() {
                        break;
                    }
                    self.drain_thread_summary_projection_wakes();
                }
                cleanup = self.cleanup_rx.changed() => {
                    if cleanup.is_err() {
                        break;
                    }
                    let cleanup = *self.cleanup_rx.borrow_and_update();
                    self.handle_cleanup(cleanup);
                }
                control = self.control_rx.recv() => {
                    let Some(control) = control else { break };
                    self.handle_msg(control.into()).await;
                }
                control = self.relay_control_rx.recv() => {
                    let Some(control) = control else { break };
                    self.handle_relay_control(control).await;
                }
                batch = async {
                    match self.relay_data_rx.as_mut() {
                        Some(receiver) => receiver.recv().await,
                        None => futures_util::future::pending().await,
                    }
                } => {
                    if let Some(TimelineRelayBatch {
                        generation,
                        diffs,
                        thread_attention_provenance,
                        gap_repair_projections,
                    }) = batch {
                        if let Some(diffs) = accepted_relay_batch(self.generation, generation, diffs) {
                            self.relay_restart_backoff.reset_after_live_batch();
                            self.handle_diff_batch(
                                diffs,
                                thread_attention_provenance,
                                gap_repair_projections,
                            ).await;
                        }
                    }
                }
                msg = self.msg_rx.recv() => {
                    let Some(msg) = msg else { break };
                    self.handle_msg(msg).await;
                }
            }
            self.finish_ready_causal_projection_handoffs().await;
        }
    }
    async fn finish_ready_causal_projection_handoffs(&mut self) {
        if let Some(batch_id) = self.ready_restore_gap_projection_batch.take() {
            self.finish_pending_gap_projection(batch_id).await;
        }
        if self.pending_live_tail_projection.is_some()
            && !self.live_tail_projection_correlation.is_pending()
        {
            if self.finish_pending_live_tail_projection().await {
                self.request_timeline_gap_inspection(TimelineGapRepairTrigger::LiveTailSnapshot)
                    .await;
            }
        }
    }
    fn handle_cleanup(&mut self, cleanup: TimelineActorCleanupState) {
        if cleanup.end_gap_demand_serial != self.last_end_gap_demand_cleanup_serial {
            self.last_end_gap_demand_cleanup_serial = cleanup.end_gap_demand_serial;
            self.foreground_gap_demand_active = false;
            self.gap_repair.pending_trigger = None;
        }
        if cleanup.cancel_network_serial != self.last_cancel_network_cleanup_serial {
            self.last_cancel_network_cleanup_serial = cleanup.cancel_network_serial;
            if let Some((current_generation, cancellation, _)) = self.live_tail_refresh.as_ref()
                && *current_generation == cleanup.cancel_network_operation_generation
            {
                cancellation.cancel();
            }
        }
        let cleanup_request_id = RequestId {
            connection_id: RuntimeConnectionId(0),
            sequence: 0,
        };
        if cleanup.cancel_pagination_serial != self.last_cancel_pagination_cleanup_serial {
            self.last_cancel_pagination_cleanup_serial = cleanup.cancel_pagination_serial;
            self.handle_cancel_pagination(cleanup_request_id);
        }
        if cleanup.cancel_link_previews_serial != self.last_cancel_link_previews_cleanup_serial {
            self.last_cancel_link_previews_cleanup_serial = cleanup.cancel_link_previews_serial;
            self.handle_cancel_link_previews(cleanup_request_id);
        }
    }
    async fn handle_msg(&mut self, msg: TimelineActorMessage) {
        match msg {
            TimelineActorMessage::StartLiveTailRefresh {
                epoch,
                operation_generation,
                limit,
            } => {
                self.start_live_tail_refresh(epoch, operation_generation, limit);
            }
            TimelineActorMessage::CancelLiveTailNetwork {
                operation_generation,
                acknowledged,
            } => {
                if let Some((current_generation, cancellation, _)) = self.live_tail_refresh.as_ref()
                    && *current_generation == operation_generation
                {
                    cancellation.cancel();
                }
                let _ = acknowledged.send(());
            }
            TimelineActorMessage::LiveTailRefreshFinished {
                actor_generation,
                epoch,
                operation_generation,
                requested_limit,
                result,
                duration_ms,
            } => {
                self.handle_live_tail_refresh_finished(
                    actor_generation,
                    epoch,
                    operation_generation,
                    requested_limit,
                    result,
                    duration_ms,
                )
                .await;
            }
            TimelineActorMessage::UpdateSubscriptionGeneration(generation) => {
                // The subscription set changed because another room was
                // added/removed; this room stayed retained, so its expected
                // generation advances. Ordered before any new checkpoint, so
                // the actor accepts them.
                self.subscription_generation = Some(generation);
            }
            TimelineActorMessage::RoomSubscriptionCheckpoint(checkpoint) => {
                if self.key.room_id() == checkpoint.room_id()
                    && self.subscription_generation == Some(checkpoint.generation())
                {
                    let checkpoint_response_sequence = checkpoint.response_sequence();
                    let advances_global_fence = room_checkpoint_advances_global_fence(
                        self.room_subscription_checkpoint.as_ref(),
                        self.deferred_room_subscription_checkpoint.as_ref(),
                        &checkpoint,
                    );
                    let changed = retain_room_subscription_checkpoint(
                        &mut self.room_subscription_checkpoint,
                        &mut self.deferred_room_subscription_checkpoint,
                        checkpoint,
                    );
                    if advances_global_fence {
                        self.global_commit_fence
                            .note_room_checkpoint_advanced(checkpoint_response_sequence);
                    }
                    if changed
                        && self
                            .room_subscription_checkpoint
                            .as_ref()
                            .is_some_and(|current| current.has_inserted_gap())
                    {
                        self.gap_repair
                            .queue_inspection(TimelineGapRepairTrigger::LiveEdge);
                    }
                    record_live_catchup_gate(
                        self.live_catchup_gate(),
                        self.subscription_generation,
                        self.room_subscription_checkpoint.as_ref(),
                        self.gap_repair_scheduler_phase(),
                        self.gap_repair.batches_processed,
                    );
                    self.start_pending_timeline_gap_inspection().await;
                    self.publish_authoritative_read_state().await;
                }
            }
            TimelineActorMessage::GlobalResponseCommitted(commit) => {
                if matches!(
                    self.global_commit_fence.observe(commit),
                    GlobalCommitDecision::InspectNewestLiveEdge
                ) {
                    self.gap_repair
                        .queue_inspection(TimelineGapRepairTrigger::LiveEdge);
                    self.start_pending_timeline_gap_inspection().await;
                }
            }
            TimelineActorMessage::InspectTimelineGaps { trigger } => {
                if matches!(trigger, TimelineGapRepairTrigger::Manual) {
                    self.gap_repair.begin_explicit_demand();
                }
                self.request_timeline_gap_inspection(trigger).await;
            }
            TimelineActorMessage::TimelineGapInspectionFinished {
                serial,
                trigger,
                committed_response,
                global_commit,
                result,
            } => {
                self.handle_timeline_gap_inspection_finished(
                    serial,
                    trigger,
                    committed_response,
                    global_commit,
                    result,
                )
                .await;
            }
            TimelineActorMessage::TimelineGapRepairFinished {
                serial,
                trigger,
                repaired_live_edge_fallback,
                result,
            } => {
                self.handle_timeline_gap_repair_finished(
                    serial,
                    trigger,
                    repaired_live_edge_fallback,
                    result,
                )
                .await;
            }
            TimelineActorMessage::TimelineGapRelaySettlementDue {
                actor_generation,
                repair_generation,
                trigger,
            } => {
                if self.gap_projection_correlation.operation
                    == Some((
                        actor_generation,
                        historical_causal_projection_operation(repair_generation),
                    ))
                {
                    let operation = historical_causal_projection_operation(repair_generation);
                    record_timeline_gap_projection_boundary(
                        "relay_timeout",
                        "authoritative_replay",
                        actor_generation,
                        self.generation,
                        operation,
                        None,
                        None,
                        self.gap_projection_correlation
                            .expected_last_projection_batch,
                        self.gap_projection_correlation.observed_batches.len(),
                    );
                    if let Some(task) = self.gap_relay_settlement_task.take() {
                        task.abort();
                    }
                    self.handle_relay_overflow(
                        self.generation,
                        TimelineResyncReason::GapSettlementTimeout,
                    )
                    .await;
                } else {
                    let _ = trigger;
                }
            }
            TimelineActorMessage::Paginate {
                request_id,
                direction,
                event_count,
            } => {
                self.handle_paginate(request_id, direction, event_count)
                    .await;
            }
            TimelineActorMessage::CancelPagination { request_id } => {
                self.handle_cancel_pagination(request_id);
            }
            TimelineActorMessage::CancelLinkPreviews { request_id } => {
                self.handle_cancel_link_previews(request_id);
            }
            TimelineActorMessage::PaginationFinished {
                serial,
                request_id,
                direction,
                completion,
            } => {
                if self
                    .pagination_task
                    .as_ref()
                    .is_some_and(|active| active.serial == serial)
                {
                    self.pagination_task = None;
                    self.emit_pagination_completion(request_id, direction, completion);
                    self.start_pending_timeline_gap_inspection().await;
                }
            }
            TimelineActorMessage::OwnReadReceiptChanged => {
                self.handle_own_read_receipt_changed().await;
            }
            TimelineActorMessage::RestoreTimelineAnchor {
                request_id,
                event_id,
                max_batches,
                event_count,
            } => {
                self.handle_restore_timeline_anchor(request_id, event_id, max_batches, event_count)
                    .await;
            }
            TimelineActorMessage::RestoreTimelineAnchorContinue { serial } => {
                self.handle_restore_timeline_anchor_continue(serial).await;
            }
            TimelineActorMessage::ObserveViewport { observation } => {
                self.viewport_observation = observation;
                if let Some(target) = self.observe_local_viewed_boundary() {
                    let _ = self
                        .manager_tx
                        .send(TimelineMessage::LocalReadBoundaryObserved {
                            key: self.key.clone(),
                            actor_generation: self.actor_generation,
                            target,
                        })
                        .await;
                }
                self.maybe_fetch_visible_reply_details();
                self.emit_navigation_if_changed();
                let viewport_range = self.viewport_item_range();
                let decision = self.gap_repair.evaluate_viewport_wake(
                    viewport_range,
                    &self.viewport_observation.visible_gap_ids,
                );
                let projected_gap_count = self.gap_repair.projected_gaps.len();
                let visible_gap_count = self.viewport_observation.visible_gap_ids.len();
                let visible_gap_validated = self
                    .viewport_observation
                    .visible_gap_ids
                    .iter()
                    .any(|id| projected_gaps_contain_id(&self.gap_repair.projected_gaps, *id));
                let scheduler_phase = self.gap_repair_scheduler_phase();
                let (decision_token, candidate_changed) = match decision {
                    GapRepairViewportWakeDecision::Wake { .. } => ("wake", true),
                    GapRepairViewportWakeDecision::WakeStaleVisibleDemand => {
                        ("wake_stale_visible", false)
                    }
                    GapRepairViewportWakeDecision::IdleNoCandidate => ("idle_no_candidate", false),
                    GapRepairViewportWakeDecision::IdleUnchangedCandidate { .. } => {
                        ("idle_unchanged", false)
                    }
                };
                let diagnostic_signature = GapRepairEvaluationDiagnosticSignature {
                    decision: decision_token,
                    projected_gap_count,
                    visible_gap_count,
                    visible_gap_validated,
                    candidate_changed,
                    scheduler_phase,
                };
                if should_record_gap_repair_evaluation(
                    &mut self.last_gap_repair_evaluation_diagnostic,
                    diagnostic_signature,
                ) {
                    record_timeline_gap_repair_evaluation(
                        decision_token,
                        projected_gap_count,
                        visible_gap_count,
                        visible_gap_validated,
                        candidate_changed,
                        scheduler_phase,
                    );
                }
                if matches!(
                    decision,
                    GapRepairViewportWakeDecision::Wake { .. }
                        | GapRepairViewportWakeDecision::WakeStaleVisibleDemand
                ) {
                    self.request_timeline_gap_inspection(TimelineGapRepairTrigger::Automatic)
                        .await;
                }
            }
            TimelineActorMessage::BeginGapRepairDemand => {
                let reason = if self.foreground_gap_demand_active {
                    "room_reselected"
                } else {
                    "room_selected"
                };
                let foreground_demand_epoch = self.gap_repair.begin_explicit_demand();
                self.foreground_gap_demand_active = true;
                let inspection_requested = !self.viewport_observation.visible_gap_ids.is_empty();
                record_timeline_gap_demand(
                    foreground_demand_epoch,
                    self.gap_repair.projected_gaps.len(),
                    self.viewport_observation.visible_gap_ids.len(),
                    inspection_requested,
                    reason,
                    self.gap_repair_scheduler_phase(),
                );
                if inspection_requested {
                    self.request_timeline_gap_inspection(TimelineGapRepairTrigger::Automatic)
                        .await;
                }
            }
            TimelineActorMessage::EndGapRepairDemand => {
                self.foreground_gap_demand_active = false;
                self.gap_repair.pending_trigger = None;
            }
            TimelineActorMessage::ForwardMessage {
                request_id,
                source_event_id,
                destination_room_id,
                transaction_id,
            } => {
                self.handle_forward_message(
                    request_id,
                    source_event_id,
                    destination_room_id,
                    transaction_id,
                )
                .await;
            }
            TimelineActorMessage::LoadMessageSource {
                request_id,
                event_id,
            } => {
                self.handle_load_message_source(request_id, event_id).await;
            }
            TimelineActorMessage::ReplyDetailsFetchFinished { event_id } => {
                self.reply_detail_fetches.remove(&event_id);
            }
            TimelineActorMessage::RequestRoomKey {
                request_id,
                event_id,
                origin,
            } => {
                self.handle_request_room_key(request_id, event_id, origin)
                    .await;
            }
            TimelineActorMessage::RequestLateDecryption {
                request_id,
                trigger,
            } => {
                self.handle_request_late_decryption(request_id, trigger)
                    .await;
            }
            TimelineActorMessage::RoomKeyRecoveryTick {
                session_id,
                attempt,
                actor_generation,
            } => {
                self.handle_room_key_recovery_tick(session_id, attempt, actor_generation)
                    .await;
            }
            TimelineActorMessage::RoomKeyWithheldObserved {
                room_id,
                session_id,
                code,
            } => {
                self.withheld_codes
                    .insert((room_id.clone(), session_id.clone()), code);
                // Issue #460: a withheld observation for a session that a
                // pending request is waiting on settles that presentation and
                // publishes immediately (the to-device event carries no
                // timeline diff, so without this the refusal stays invisible).
                let pending_event_ids: Vec<String> = self
                    .key_request_states
                    .iter()
                    .filter(|(_, state)| state.session_id.as_deref() == Some(session_id.as_str()))
                    .map(|(event_id, _)| event_id.clone())
                    .collect();
                for event_id in pending_event_ids {
                    // Settle the active retry for this event first (if any):
                    // its settle path transitions the stage and publishes, and
                    // prevents the pending timeout from later downgrading the
                    // refusal to still_waiting.
                    if let Some(operation) = decrypt_retry_settlement_operation(
                        &self.decrypt_retry,
                        self.actor_generation,
                        &event_id,
                    ) {
                        self.settle_decrypt_retry(operation, DecryptRetrySettledResult::Withheld);
                        continue;
                    }
                    // Non-current requests (already timed out / settled) still
                    // surface the refusal when the observation arrives late.
                    // Terminal stages are not regressed: a recovered event stays
                    // recovered and a send failure stays failed. A stage already
                    // settled `withheld` by a diff still gains the typed code
                    // when the independent observation arrives later.
                    let should_publish =
                        self.key_request_states.get(&event_id).is_some_and(|state| {
                            withheld_update_should_publish(state.stage, state.withheld_code, code)
                        });
                    if !should_publish {
                        continue;
                    }
                    if let Some(state) = self.key_request_states.get_mut(&event_id) {
                        state.stage = "withheld";
                        state.withheld_code = Some(code);
                    }
                    if let Some(state) = self.key_request_states.get(&event_id) {
                        self.publish_key_request_state(&event_id, state);
                    }
                }
            }
            TimelineActorMessage::DecryptRetryTimeout {
                operation,
                actor_generation,
            } => {
                if self.actor_generation == actor_generation
                    && self.decrypt_retry.is_current(operation, actor_generation)
                {
                    self.settle_decrypt_retry(operation, DecryptRetrySettledResult::Timeout);
                }
            }
            TimelineActorMessage::RetrySend {
                request_id,
                transaction_id,
            } => {
                self.handle_retry_send(request_id, transaction_id).await;
            }
            TimelineActorMessage::CancelSend {
                request_id,
                transaction_id,
            } => {
                self.handle_cancel_send(request_id, transaction_id).await;
            }
            TimelineActorMessage::DownloadMedia {
                request_id,
                event_id,
                selection,
            } => {
                self.handle_download_media(request_id, event_id, selection)
                    .await;
            }
            TimelineActorMessage::MediaDownloadFinished {
                request_id,
                event_id,
                outcome,
            } => {
                self.handle_media_download_finished(request_id, event_id, outcome)
                    .await;
            }
            TimelineActorMessage::EditText {
                request_id,
                event_id,
                document,
            } => {
                self.handle_edit_text(request_id, event_id, document).await;
            }
            TimelineActorMessage::Redact {
                request_id,
                event_id,
            } => {
                self.handle_redact(request_id, event_id).await;
            }
            TimelineActorMessage::ToggleReaction {
                request_id,
                event_id,
                reaction_key,
            } => {
                self.handle_toggle_reaction(request_id, event_id, reaction_key)
                    .await;
            }
            TimelineActorMessage::SendReaction {
                request_id,
                event_id,
                reaction_key,
            } => {
                self.handle_send_reaction(request_id, event_id, reaction_key)
                    .await;
            }
            TimelineActorMessage::RedactReaction {
                request_id,
                event_id,
                reaction_key,
                reaction_event_id,
            } => {
                self.handle_redact_reaction(request_id, event_id, reaction_key, reaction_event_id)
                    .await;
            }
            TimelineActorMessage::ApplyReadSuccess {
                kind,
                event_id,
                acknowledged,
            } => {
                let applied = self.handle_read_success(kind, event_id).await;
                let _ = acknowledged.send(applied);
            }
            TimelineActorMessage::ReadStateProjection {
                local_viewed_event_id,
                server_confirmed_read_event_id,
                sync,
            } => {
                self.handle_read_state_projection(
                    local_viewed_event_id,
                    server_confirmed_read_event_id,
                    sync,
                );
            }
            TimelineActorMessage::ReadStatePolicyChanged { send_read_receipts } => {
                self.send_read_receipts = send_read_receipts;
                if !send_read_receipts && matches!(self.key.kind, TimelineKind::Thread { .. }) {
                    self.read_state_sync = TimelineReadStateSync::NotRequested;
                    self.emit_navigation_if_changed();
                }
            }
            TimelineActorMessage::RefreshPendingSendProjection {
                actor_generation,
                projections,
                acknowledged,
            } => {
                let accepted = self
                    .refresh_pending_send_projection(actor_generation, projections)
                    .await;
                let _ = acknowledged.send(accepted);
            }
            TimelineActorMessage::DisplayPolicyChanged { thread_root_order } => {
                if self.thread_root_order != thread_root_order {
                    self.thread_root_order = thread_root_order;
                    let diffs = self.reproject_display_items();
                    if !diffs.is_empty() {
                        let batch_id = self.next_batch_id;
                        if super::navigation::emit_items_updated_for_generation(
                            &self.event_tx,
                            &self.timeline_actor_generations,
                            &self.key,
                            self.actor_generation,
                            self.generation,
                            batch_id,
                            diffs,
                        ) {
                            self.next_batch_id = TimelineBatchId(batch_id.0 + 1);
                        }
                    }
                }
            }
            TimelineActorMessage::SetTyping {
                request_id,
                is_typing,
            } => {
                self.handle_set_typing(request_id, is_typing).await;
            }
            TimelineActorMessage::TypingUsersUpdated(user_ids) => {
                self.emit_typing_users_action(user_ids);
            }
            TimelineActorMessage::IgnoredUsersUpdated(user_ids) => {
                self.handle_ignored_users_updated(user_ids).await;
            }
            TimelineActorMessage::LoadLinkPreviews {
                request_id,
                event_id,
            } => {
                self.handle_load_link_previews(request_id, event_id).await;
            }
            TimelineActorMessage::LinkPreviewsFetched {
                request_id,
                event_id,
                previews,
                pending_count,
                ready_count,
                failed_count,
                elapsed_ms,
            } => {
                self.handle_link_previews_fetched(
                    request_id,
                    event_id,
                    previews,
                    pending_count,
                    ready_count,
                    failed_count,
                    elapsed_ms,
                )
                .await;
            }
            TimelineActorMessage::HideLinkPreview {
                request_id,
                event_id,
            } => {
                self.handle_hide_link_preview(request_id, event_id).await;
            }
            TimelineActorMessage::LinkPreviewPolicyChanged {
                unencrypted_global_enabled,
                encrypted_global_enabled,
                room_enabled,
            } => {
                self.handle_link_preview_policy_changed(
                    unencrypted_global_enabled,
                    encrypted_global_enabled,
                    room_enabled,
                )
                .await;
            }
            TimelineActorMessage::SendQueueUpdate(update) => {
                self.handle_send_queue_update(update).await;
            }
            TimelineActorMessage::SendQueueLagged => {
                self.handle_send_queue_lagged().await;
                self.publish_current_canonical_activity().await;
            }
            TimelineActorMessage::ReplayInitialItems { cause_request_id } => {
                self.handle_replay_initial_items(cause_request_id);
                self.publish_current_canonical_activity().await;
            }
            #[cfg(test)]
            TimelineActorMessage::TestBeginRestore {
                request_id,
                event_id,
                acknowledged,
            } => {
                self.restore_anchor = Some(RestoreTimelineAnchorState {
                    request_id,
                    event_id,
                    max_batches_remaining: 1,
                    event_count: 1,
                    in_flight: false,
                    awaiting_diff_batch: false,
                    continuation_scheduled: false,
                    continuation_serial: None,
                    anchor_relay_wait: None,
                });
                let _ = acknowledged.send(());
            }
            #[cfg(test)]
            TimelineActorMessage::TestInjectRestoreDiff {
                diffs,
                projections,
                acknowledged,
            } => {
                let provenance = ThreadAttentionBatchProvenance::from_sdk_diffs(&diffs);
                self.handle_diff_batch(diffs, provenance, projections).await;
                let _ = acknowledged.send(());
            }
            #[cfg(test)]
            TimelineActorMessage::TestRestoreCausalState(response) => {
                let _ = response.send((
                    self.live_tail_projection_correlation.is_pending(),
                    self.pending_live_tail_projection.is_some(),
                    self.restore_emit_buffer.len(),
                    self.restore_causal_projections.projections.clone(),
                ));
            }
            #[cfg(test)]
            TimelineActorMessage::TestFinishRestore {
                request_id,
                response,
            } => {
                let had_buffered_diffs = !self.restore_emit_buffer.is_empty();
                self.finish_anchor_restore(request_id, TimelineAnchorRestoreStatus::EndReached);
                let _ = response.send(had_buffered_diffs);
            }
            #[cfg(test)]
            TimelineActorMessage::TestArmGapRepairCompletionPause {
                pause,
                acknowledged,
            } => {
                self.test_gap_repair_completion_pause = Some(pause);
                let _ = acknowledged.send(());
            }
            #[cfg(test)]
            TimelineActorMessage::Barrier(response) => {
                let _ = response.send(());
            }
        }
        if self.hydrate_after_restore_flush && self.restore_anchor.is_none() {
            self.hydrate_after_restore_flush = false;
            self.maybe_hydrate_missing_thread_roots(None).await;
        }
    }
    pub(super) fn emit(&self, event: CoreEvent) {
        match event {
            CoreEvent::Timeline(event) => {
                let _ = emit_timeline_events_for_generation(
                    &self.event_tx,
                    &self.timeline_actor_generations,
                    &self.key,
                    self.actor_generation,
                    vec![event],
                );
            }
            event => {
                let _ = self.event_tx.send(event);
            }
        }
    }
    /// Reliably deliver an `AppAction` to the reducer.  Uses `send` (not
    /// `try_send`) so the action is not silently dropped when the channel is
    /// momentarily full.  Required for state-machine transitions where a
    /// dropped action would leave the UI stuck in a pending/inconsistent state
    /// (REPOSITORY_RULES L124-128).
    pub(super) async fn emit_action_reliable(&self, action: AppAction) -> bool {
        send_generation_fenced(
            &self.action_tx,
            &self.timeline_actor_generations,
            &self.key,
            self.actor_generation,
            vec![action],
        )
        .await
    }
    pub(super) async fn emit_search_messages_reliable(
        &self,
        messages: Vec<SearchIndexMessage>,
    ) -> bool {
        let Some(tx) = &self.search_index_tx else {
            return self
                .timeline_actor_generations
                .try_acquire(&self.key, self.actor_generation)
                .is_some();
        };
        for message in messages {
            if !send_generation_fenced(
                tx,
                &self.timeline_actor_generations,
                &self.key,
                self.actor_generation,
                message,
            )
            .await
            {
                return false;
            }
        }
        self.timeline_actor_generations
            .try_acquire(&self.key, self.actor_generation)
            .is_some()
    }
    pub(super) fn emit_failure(&self, request_id: RequestId, failure: CoreFailure) {
        self.emit(CoreEvent::OperationFailed {
            request_id,
            failure,
        });
    }
    pub(super) fn emit_timeline_failure(&self, request_id: RequestId, kind: TimelineFailureKind) {
        self.emit_failure(request_id, CoreFailure::TimelineOperationFailed { kind });
    }
    fn emit_typing_users_action(&self, user_ids: Vec<String>) {
        let Some(room_id) = timeline_room_id(&self.key) else {
            return;
        };
        let _ = self
            .action_tx
            .try_send(vec![AppAction::TypingUsersUpdated { room_id, user_ids }]);
    }
}

#[cfg(test)]
mod tests;
