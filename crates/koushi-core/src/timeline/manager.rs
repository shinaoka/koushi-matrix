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
use koushi_protocol::event::{CoreEvent, LiveSignalsEvent, TimelineEvent, TimelineItem};
use koushi_protocol::failure::{CoreFailure, TimelineFailureKind};
use koushi_protocol::ids::{RequestId, RuntimeConnectionId, TimelineKey, TimelineKind};

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
use super::focused_build::{
    FocusedBuildCompletion, FocusedBuildFailure, FocusedBuildSupervisor, PreparedFocusedTimeline,
    record_focused_build, record_focused_build_settled,
};
use super::gap_repair::{
    GlobalResponseCommit, LIVE_TAIL_CANCELLATION_DEADLINE, TimelineGapRepairTrigger,
};
use super::navigation::{
    INITIAL_EMPTY_ROOM_BACKFILL_EVENT_COUNT, TimelineActorGenerationGate,
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
use super::thread_projection::{
    ThreadRootProjectionFetchRegistry, ThreadSummaryActivityObservation,
};
use super::typing::{TypingLane, TypingLaneSettlement};
// END GENERATED SIBLING IMPORTS

/// Bounded diff queue capacity per subscribed timeline (overview.md, Async rule 10).
pub const TIMELINE_DIFF_QUEUE_CAPACITY: usize = 128;

/// Export-time snapshot of the resident Room timeline set (#1394).
///
/// Counts only: no Room, thread, event or user identifiers and no message
/// content. `resident_event_items` sums the position-evidence entry count of
/// every resident actor, so it measures distinct event-ID-backed canonical
/// depth; transaction-backed local echoes, date separators and receipt-only
/// items are not part of that evidence.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TimelineResidencyDiagnostics {
    /// Session-resident desired Room subscriptions.
    pub session_resident_rooms: u64,
    /// Room subscriptions actually held by the SDK Sliding Sync session, which
    /// expiry can empty while the logical subscription set is unchanged.
    pub actual_room_subscriptions: u64,
    /// Resident timeline actors, and their split by timeline kind.
    pub resident_actors: u64,
    pub resident_room_actors: u64,
    pub resident_thread_actors: u64,
    pub resident_focused_actors: u64,
    /// Canonical event items held by all resident actors, and the deepest one.
    pub resident_event_items: u64,
    pub deepest_actor_event_items: u64,
    /// Lower-bound bytes of the resident projection: `size_of::<TimelineItem>()`
    /// per item plus the message text each actor holds (#1394).
    pub resident_item_struct_bytes: u64,
    pub resident_item_text_bytes: u64,
}

fn initial_thread_backfill_is_authoritative(end_reached: bool, item_count: usize) -> bool {
    end_reached || item_count > 0
}

/// Fold one `(kind, canonical event-item count, struct bytes, text bytes)` row
/// per resident actor into a [`TimelineResidencyDiagnostics`] snapshot (#1394).
/// An actor without position evidence contributes an actor but no depth.
pub(super) fn fold_residency_actors<'a>(
    session_resident_rooms: usize,
    actual_room_subscriptions: usize,
    actors: impl IntoIterator<Item = (&'a TimelineKind, Option<(usize, u64, u64)>)>,
) -> TimelineResidencyDiagnostics {
    let mut diagnostics = TimelineResidencyDiagnostics {
        session_resident_rooms: session_resident_rooms as u64,
        actual_room_subscriptions: actual_room_subscriptions as u64,
        ..TimelineResidencyDiagnostics::default()
    };
    for (kind, evidence) in actors {
        diagnostics.resident_actors += 1;
        match kind {
            TimelineKind::Room { .. } => diagnostics.resident_room_actors += 1,
            TimelineKind::Thread { .. } => diagnostics.resident_thread_actors += 1,
            TimelineKind::Focused { .. } => diagnostics.resident_focused_actors += 1,
        }
        let (event_items, struct_bytes, text_bytes) = evidence.unwrap_or((0, 0, 0));
        let event_items = event_items as u64;
        diagnostics.resident_event_items += event_items;
        diagnostics.resident_item_struct_bytes += struct_bytes;
        diagnostics.resident_item_text_bytes += text_bytes;
        diagnostics.deepest_actor_event_items =
            diagnostics.deepest_actor_event_items.max(event_items);
    }
    diagnostics
}

async fn await_initial_thread_projection<S>(end_reached: bool, counts: S) -> bool
where
    S: futures_util::Stream<Item = usize>,
{
    if end_reached {
        return true;
    }
    record_subscribe_stage("initial_backfill_projection_wait", None);
    let wait = async move {
        futures_util::pin_mut!(counts);
        while let Some(count) = counts.next().await {
            if initial_thread_backfill_is_authoritative(false, count) {
                return true;
            }
        }
        record_subscribe_stage("initial_backfill_projection_closed", None);
        false
    };
    match tokio::time::timeout(std::time::Duration::from_secs(10), wait).await {
        Ok(ready) => ready,
        Err(_) => {
            record_subscribe_stage("initial_backfill_projection_deadline", None);
            false
        }
    }
}

async fn hydrate_initial_thread(
    timeline: &matrix_sdk_ui::timeline::Timeline,
    client: &matrix_sdk::Client,
    room_id: &matrix_sdk::ruma::RoomId,
    root_event_id: &matrix_sdk::ruma::EventId,
) -> Result<bool, TimelineFailureKind> {
    let hydrate = async {
        let (initial_items, initial_updates) = timeline.subscribe().await;
        if !initial_items.is_empty() {
            return Ok(true);
        }
        // A single UI pagination step can stop at a cached chunk containing only
        // edits/reactions. Consume a bounded raw-event window across such chunks.
        let (cache, _drop_handles) = client
            .event_cache()
            .thread(room_id, root_event_id)
            .await
            .map_err(|_| {
                record_subscribe_stage("initial_backfill_sdk_failed", None);
                TimelineFailureKind::Sdk
            })?;
        let outcome = cache
            .pagination()
            .run_backwards_until(INITIAL_EMPTY_ROOM_BACKFILL_EVENT_COUNT)
            .await
            .map_err(|error| {
                let kind = classify_thread_backfill_error(&error);
                record_subscribe_stage(
                    match kind {
                        TimelineFailureKind::Forbidden | TimelineFailureKind::NotFound => {
                            "initial_backfill_root_withheld"
                        }
                        _ => "initial_backfill_sdk_failed",
                    },
                    None,
                );
                kind
            })?;
        // Keep the pre-pagination subscription: the SDK relay is asynchronous.
        let counts = initial_updates.scan(initial_items, |items, diffs| {
            for diff in diffs {
                diff.apply(items);
            }
            futures_util::future::ready(Some(items.len()))
        });
        Ok(await_initial_thread_projection(outcome.reached_start, counts).await)
    };
    match tokio::time::timeout(std::time::Duration::from_secs(10), hydrate).await {
        Ok(result) => result,
        Err(_) => {
            record_subscribe_stage("initial_backfill_projection_deadline", None);
            Err(TimelineFailureKind::Timeout)
        }
    }
}

/// #1473: a server that withholds the thread root is a permanent visibility
/// outcome, not a transient SDK failure. Synapse answers `/relations` with
/// HTTP 404 `M_UNKNOWN` ("Unknown parent event."), tuwunel the root fetch with
/// 403 `M_FORBIDDEN`, so the HTTP status is authoritative when the errcode is
/// generic.
fn classify_thread_backfill_error(
    error: &matrix_sdk::event_cache::EventCacheError,
) -> TimelineFailureKind {
    use matrix_sdk::event_cache::EventCacheError;
    use matrix_sdk::ruma::api::error::ErrorKind;

    let EventCacheError::PaginationError(error) = error else {
        return TimelineFailureKind::Sdk;
    };
    if let Some(api_error) = error.as_client_api_error() {
        return match (api_error.error_kind(), api_error.status_code.as_u16()) {
            (Some(ErrorKind::Forbidden), _) | (_, 403) => TimelineFailureKind::Forbidden,
            (Some(ErrorKind::NotFound), _) | (_, 404) => TimelineFailureKind::NotFound,
            _ => TimelineFailureKind::Sdk,
        };
    }
    match error.as_ref() {
        matrix_sdk::Error::Http(http_error)
            if matches!(**http_error, matrix_sdk::HttpError::Reqwest(_)) =>
        {
            TimelineFailureKind::Network
        }
        _ => TimelineFailureKind::Sdk,
    }
}

/// Messages routed to the `TimelineManagerActor`.
pub(crate) enum TimelineMessage {
    BindHistoryRecovery(mpsc::Sender<crate::room::RoomMessage>),
    ReadReceiptWindow {
        source: koushi_protocol::view::ReceiptSourceRef,
        start: u64,
        limit: koushi_protocol::view::ReaderWindowLimit,
        response: oneshot::Sender<
            Result<
                super::receipt_endpoints::RawReceiptWindow,
                crate::view_scope_lifecycle::ScopeError,
            >,
        >,
    },
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
    /// #1259: a thread read advanced the shared thread-unread marker; ask the room's
    /// timeline actor to re-overlay its root items. Best-effort: a key without a
    /// retained timeline is a no-op.
    RefreshThreadUnread {
        key: TimelineKey,
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
    SettingsRoomObserved {
        core_generation: u64,
        room_id: OwnedRoomId,
        acknowledged: oneshot::Sender<()>,
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
    TestFocusedBuildProbe {
        key: TimelineKey,
        response: oneshot::Sender<super::focused_build::FocusedBuildProbe>,
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
        hide_redacted: bool,
        acknowledged: oneshot::Sender<()>,
    },
    Shutdown {
        acknowledged: oneshot::Sender<()>,
    },
    /// #1394: answer an export-time resident-set sample. The caller forwards the
    /// reply channel, so the manager never awaits and a dropped request settles
    /// the caller's bounded wait as unavailable.
    ResidencyDiagnostics {
        response: oneshot::Sender<Option<TimelineResidencyDiagnostics>>,
    },
}

impl TimelineManagerHandle {
    pub(crate) fn spawn(
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
        data_dir: Option<std::path::PathBuf>,
        account_work: AccountWorkScheduler,
        navigation_projection_rx: Option<watch::Receiver<super::NavigationProjectionDemand>>,
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

    #[expect(
        clippy::too_many_arguments,
        reason = "actor wiring: independent owned handles moved into one task"
    )]
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
        navigation_projection_rx: Option<watch::Receiver<super::NavigationProjectionDemand>>,
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
        hide_redacted: bool,
    ) -> bool {
        let (acknowledged, acknowledgement) = oneshot::channel();
        if self
            .control_tx
            .send(TimelineManagerControl::DisplayPolicyChanged {
                thread_root_order,
                hide_redacted,
                acknowledged,
            })
            .await
            .is_err()
        {
            return false;
        }
        acknowledgement.await.is_ok()
    }

    /// #1394: request an export-time resident-set snapshot. Enqueued without
    /// waiting; a full control mailbox drops the reply channel, which settles
    /// the caller's bounded wait as unavailable instead of blocking the export.
    pub(crate) fn residency_diagnostics(
        &self,
        response: oneshot::Sender<Option<TimelineResidencyDiagnostics>>,
    ) {
        let _ = self
            .control_tx
            .try_send(TimelineManagerControl::ResidencyDiagnostics { response });
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
    pub(super) history_recovery: Option<mpsc::Sender<crate::room::RoomMessage>>,
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
    /// Session/manager-owned room-scoped typing lane (#1481). Typing HTTP must
    /// never be awaited in an actor mailbox handler.
    pub(super) typing_lane: TypingLane,
    pub(super) mutation_lane: super::mutation::MutationLane,
    pub(super) action_tx: mpsc::Sender<Vec<AppAction>>,
    pub(super) event_tx: broadcast::Sender<CoreEvent>,
    pub(super) msg_tx: mpsc::Sender<TimelineMessage>,
    pub(super) msg_rx: mpsc::Receiver<TimelineMessage>,
    pub(super) control_rx: Option<mpsc::Receiver<TimelineManagerControl>>,
    pub(super) navigation_projection_rx: Option<watch::Receiver<super::NavigationProjectionDemand>>,
    pub(super) last_navigation_projection_generation: u64,
    /// The room projection last applied from the retained demand, so a
    /// focused-only update does not replay an unchanged room (#1060).
    pub(super) applied_room_projection: Option<u64>,
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
    /// #1110: the redaction display preference, part of the row-visibility
    /// policy the TimelineActors own.
    pub(super) hide_redacted: bool,
    pub(super) account_work: AccountWorkScheduler,
    /// Room-root hydration is shared across replacement actors so SyncStarted
    /// cannot restart a failed/pending bounded lookup.
    pub(super) thread_root_projection_service: Arc<Mutex<ThreadRootProjectionService>>,
    pub(super) thread_root_projection_fetches: ThreadRootProjectionFetchRegistry,
    pub(super) timeline_actor_generations: Arc<TimelineActorGenerationGate>,
    pub(super) live_tail_refreshes: LiveTailRefreshCoordinator<TimelineKey>,
    /// #1146: focused SDK builds in flight, polled by the manager loop.
    pub(super) focused_builds: FocusedBuildSupervisor,
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
        self.settle_typing_lane();
        self.send_enqueue_workers.cancel_all();
        self.focused_builds.abort_all();
        self.global_send_completion_observer_future.take();
    }
}

impl TimelineManagerActor {
    pub(crate) fn spawn(
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
        data_dir: Option<std::path::PathBuf>,
        account_work: AccountWorkScheduler,
        navigation_projection_rx: Option<watch::Receiver<super::NavigationProjectionDemand>>,
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
            history_recovery: None,
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
            typing_lane: TypingLane::unavailable(),
            mutation_lane: Default::default(),
            thread_root_order: TimelineThreadRootOrder::LatestReply,
            hide_redacted: false,
            action_tx,
            event_tx,
            msg_tx: tx.clone(),
            msg_rx,
            control_rx: Some(control_rx),
            navigation_projection_rx,
            last_navigation_projection_generation: 0,
            applied_room_projection: None,
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
            focused_builds: Default::default(),
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
    #[expect(
        clippy::too_many_arguments,
        reason = "actor wiring: independent owned handles moved into one task"
    )]
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
        navigation_projection_rx: Option<watch::Receiver<super::NavigationProjectionDemand>>,
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
                Some(session.client()),
            )) as GlobalSendCompletionObserverFuture);
        let typing_lane = TypingLane::matrix(Arc::clone(&session));
        let actor = TimelineManagerActor {
            history_recovery: None,
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
            typing_lane,
            mutation_lane: Default::default(),
            thread_root_order: TimelineThreadRootOrder::LatestReply,
            hide_redacted: false,
            action_tx,
            event_tx,
            msg_tx: tx.clone(),
            msg_rx,
            control_rx: Some(control_rx),
            navigation_projection_rx,
            last_navigation_projection_generation: 0,
            applied_room_projection: None,
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
            focused_builds: Default::default(),
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
        // #1554: each awaited step is boxed so this loop's frame holds pointers
        // instead of every select branch's temporary futures.
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
                            hide_redacted,
                            acknowledged,
                        }) => {
                            self.thread_root_order = thread_root_order;
                            self.hide_redacted = hide_redacted;
                            for actor in self.timelines.values() {
                                let _ = actor
                                    .send_control(TimelineActorControl::DisplayPolicyChanged {
                                        thread_root_order,
                                        hide_redacted,
                                    })
                                    .await;
                            }
                            let _ = acknowledged.send(());
                            continue;
                        }
                        Some(TimelineManagerControl::ResidencyDiagnostics { response }) => {
                            let _ = response.send(Some(self.residency_diagnostics()));
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
                        Box::pin(self.handle_navigation_projection(projection)).await;
                    }
                    continue;
                }
                completion = self.focused_builds.tasks.next(),
                    if !self.focused_builds.tasks.is_empty() => {
                    if let Some(completion) = completion {
                        Box::pin(self.handle_focused_build_completion(completion)).await;
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
                completion = self.typing_lane.tasks.next(),
                    if !self.typing_lane.tasks.is_empty() => {
                    if let Some(completion) = completion {
                        self.handle_typing_completion(completion);
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
                TimelineMessage::BindHistoryRecovery(tx) => {
                    self.history_recovery = Some(tx);
                }
                TimelineMessage::ReadReceiptWindow {
                    source,
                    start,
                    limit,
                    response,
                } => {
                    if let Some(handle) = self.timelines.get(&source.timeline.key) {
                        let _ = Box::pin(handle.send(TimelineActorMessage::ReadReceiptWindow {
                            source,
                            start,
                            limit,
                            response,
                        }))
                        .await;
                    } else {
                        let _ = response.send(Err(
                            crate::view_scope_lifecycle::ScopeError::SourceUnavailable,
                        ));
                    }
                }
                TimelineMessage::Shutdown { acknowledged } => {
                    shutdown_acknowledgement = acknowledged;
                    break;
                }
                TimelineMessage::SyncStarted {
                    room_list_service,
                    core_generation,
                } => {
                    Box::pin(self.handle_sync_started(room_list_service, core_generation)).await;
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
                    Box::pin(self.handle_visible_rooms_observed(core_generation, room_ids)).await;
                }
                TimelineMessage::SettingsRoomObserved {
                    core_generation,
                    room_id,
                    acknowledged,
                } => {
                    Box::pin(self.handle_settings_room_observed(core_generation, room_id)).await;
                    let _ = acknowledged.send(());
                }
                TimelineMessage::RoomMembershipObserved {
                    core_generation,
                    transitions,
                } => {
                    Box::pin(self.handle_room_membership_observed(core_generation, transitions))
                        .await;
                }
                TimelineMessage::RoomLeft {
                    room_id,
                    cause,
                    acknowledged,
                } => {
                    Box::pin(self.handle_room_left(room_id, cause)).await;
                    let _ = acknowledged.send(());
                }
                TimelineMessage::RoomRejoined {
                    room_id,
                    acknowledged,
                } => {
                    Box::pin(self.handle_room_rejoined(room_id)).await;
                    let _ = acknowledged.send(());
                }
                TimelineMessage::AllRoomsResponseCommitted {
                    core_generation,
                    response_sequence,
                } => {
                    Box::pin(
                        self.handle_all_rooms_response_committed(
                            core_generation,
                            response_sequence,
                        ),
                    )
                    .await;
                }
                TimelineMessage::RoomSubscriptionCheckpoint {
                    service_epoch,
                    checkpoint,
                } => {
                    Box::pin(self.handle_room_subscription_checkpoint(service_epoch, checkpoint))
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
                    Box::pin(self.handle_live_tail_refresh_completed(
                        key,
                        actor_generation,
                        epoch,
                        operation_generation,
                        outcome,
                        requested_limit,
                        returned_events,
                        duration_ms,
                    ))
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
                #[cfg(test)]
                TimelineMessage::TestFocusedBuildProbe { key, response } => {
                    let _ = response.send(self.focused_build_probe(&key));
                }
                TimelineMessage::IgnoredUsersUpdated { user_ids } => {
                    Box::pin(self.handle_ignored_users_updated(user_ids)).await;
                }
                TimelineMessage::ThreadRootProjectionFetchFinished {
                    key,
                    actor_generation,
                    activity,
                    result,
                } => {
                    Box::pin(self.handle_thread_root_projection_fetch_finished(
                        key,
                        actor_generation,
                        activity,
                        result,
                    ))
                    .await;
                }
                TimelineMessage::StartAggregateRefresh {
                    key,
                    actor_generation,
                    own_user_id,
                    refreshes,
                } => {
                    Box::pin(self.handle_aggregate_refresh_start(
                        key,
                        actor_generation,
                        own_user_id,
                        refreshes,
                    ))
                    .await;
                }
                TimelineMessage::AggregateRefreshFinished {
                    key,
                    actor_generation,
                    refresh,
                    result,
                } => {
                    Box::pin(self.handle_aggregate_refresh_finished(
                        key,
                        actor_generation,
                        refresh,
                        result,
                    ))
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
                    Box::pin(self.handle_authoritative_read_state_observed(
                        &key,
                        actor_generation,
                        read_key,
                        event_id,
                    ))
                    .await;
                }
                TimelineMessage::LocalReadBoundaryObserved {
                    key,
                    actor_generation,
                    target,
                } => {
                    Box::pin(self.handle_local_read_boundary_observed(
                        key,
                        actor_generation,
                        target,
                    ))
                    .await;
                }
                TimelineMessage::RefreshThreadUnread { key } => {
                    if let Some(handle) = self.timelines.get(&key) {
                        let deadline = executor::Instant::now() + LIVE_TAIL_CANCELLATION_DEADLINE;
                        let _ = Box::pin(executor::timeout_at(
                            deadline,
                            handle.send_control(TimelineActorControl::RefreshThreadUnread),
                        ))
                        .await;
                    }
                }
                TimelineMessage::Command(command) => {
                    Box::pin(self.handle_command(command)).await;
                }
                TimelineMessage::CommandWithComposerFormatting {
                    command,
                    formatting_options,
                } => {
                    Box::pin(
                        self.handle_command_with_formatting_options(command, formatting_options),
                    )
                    .await;
                }
                TimelineMessage::LeasedCommand {
                    command,
                    composer_permit,
                } => {
                    Box::pin(self.handle_command_with_permit(command, Some(composer_permit))).await;
                }
                TimelineMessage::LeasedCommandWithComposerFormatting {
                    command,
                    composer_permit,
                    formatting_options,
                } => {
                    Box::pin(self.handle_command_with_formatting_context(
                        command,
                        Some(composer_permit),
                        formatting_options,
                    ))
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
        self.mutation_lane.begin_shutdown();
        Box::pin(self.focused_builds.cancel_all()).await;
        self.read_workers.cancel_all();
        self.read_workers.publish_persistence();
        self.settle_typing_lane();
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
        Box::pin(self.join_send_enqueue_workers()).await;
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
            Box::pin(handle.stop()).await;
        }
        Box::pin(self.mutation_lane.stop()).await;
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
        while let Some(terminal) = Box::pin(self.terminal_rx.recv()).await {
            Box::pin(self.handle_send_terminal_handoff(terminal)).await;
        }
        for key in room_keys {
            Box::pin(self.clear_thread_root_projections_for_room(&key)).await;
        }
        Box::pin(self.thread_root_projection_fetches.abort_all()).await;
        if let Some(task) = self.room_subscription_checkpoint_task.take() {
            task.abort();
        }
        if let Some(acknowledged) = shutdown_acknowledgement {
            let _ = acknowledged.send(());
        }
    }
    pub(super) async fn handle_command(&mut self, command: TimelineCommand) {
        Box::pin(self.handle_command_with_permit(command, None)).await;
    }
    async fn handle_command_with_formatting_options(
        &mut self,
        command: TimelineCommand,
        formatting_options: ComposerFormattingOptions,
    ) {
        Box::pin(self.handle_command_with_formatting_context(command, None, formatting_options))
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
        Box::pin(self.handle_command_with_permit(command, composer_permit)).await;
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
                Box::pin(self.handle_subscribe(request_id, key, true, true, initial_backfill))
                    .await;
            }
            TimelineCommand::EnsureSubscribed {
                request_id,
                key,
                replay_existing,
            } => {
                trace_timeline_route("manager_received", "ensure_subscribed", request_id, &key);
                if matches!(key.kind, TimelineKind::Room { .. }) {
                    Box::pin(self.handle_committed_room_selection(
                        request_id,
                        key,
                        replay_existing,
                        true,
                    ))
                    .await;
                } else if matches!(key.kind, TimelineKind::Thread { .. })
                    && !self.timelines.contains_key(&key)
                {
                    // #1473: thread actors are built only by the Rust-owned
                    // `OpenThread` subscribe, which owns the initial-hydration
                    // gate. A renderer recovery that raced a failed open must
                    // not leave an unhydrated actor whose replay would later
                    // report a false success; it only replays a retained one.
                    record_subscribe_stage("thread_recovery_without_actor", None);
                    self.emit_failure(
                        request_id,
                        CoreFailure::TimelineOperationFailed {
                            kind: TimelineFailureKind::NotSubscribed,
                        },
                    );
                } else {
                    Box::pin(self.handle_subscribe(
                        request_id,
                        key,
                        replay_existing,
                        true,
                        InitialBackfillPolicy::Disabled,
                    ))
                    .await;
                }
            }
            TimelineCommand::ReplaySubscribed { request_id } => {
                Box::pin(self.handle_replay_subscribed(request_id)).await;
            }
            TimelineCommand::Unsubscribe { request_id, key } => {
                trace_timeline_route("manager_received", "unsubscribe", request_id, &key);
                Box::pin(self.unsubscribe_timeline(&key)).await;
            }
            TimelineCommand::Paginate {
                request_id,
                key,
                direction,
                event_count,
            } => {
                trace_timeline_route("manager_received", "paginate", request_id, &key);
                Box::pin(self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::Paginate {
                        request_id,
                        direction,
                        event_count,
                    },
                ))
                .await;
            }
            TimelineCommand::CancelPagination { request_id, key } => {
                trace_timeline_route("manager_received", "cancel_pagination", request_id, &key);
                if let Some(handle) = self.timelines.get(&key) {
                    let _ = Box::pin(
                        handle.send(TimelineActorMessage::CancelPagination { request_id }),
                    )
                    .await;
                }
            }
            TimelineCommand::CancelLinkPreviews { request_id, key } => {
                trace_timeline_route("manager_received", "cancel_link_previews", request_id, &key);
                if let Some(handle) = self.timelines.get(&key) {
                    let _ = Box::pin(
                        handle.send(TimelineActorMessage::CancelLinkPreviews { request_id }),
                    )
                    .await;
                }
            }
            TimelineCommand::ReturnToLive { request_id, key } => {
                Box::pin(self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::ReturnToLive { request_id },
                ))
                .await;
            }
            TimelineCommand::ObserveViewport {
                request_id,
                key,
                observation,
            } => {
                Box::pin(self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::ObserveViewport { observation },
                ))
                .await;
            }
            TimelineCommand::RepairGaps { request_id, key } => {
                record_timeline_gap_repair("requested", "manual", 0, 0, 0, "queued");
                Box::pin(self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::InspectTimelineGaps {
                        trigger: TimelineGapRepairTrigger::Manual,
                    },
                ))
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
                Box::pin(self.route_send_to_worker_or_fail(
                    request_id,
                    &key,
                    transaction_id.clone(),
                    body.clone(),
                    SendComposerProjection::None,
                    TimelineSendEnqueuePayload::Text {
                        document,
                        formatting_options: self.composer_formatting_options,
                    },
                ))
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
                Box::pin(self.route_submission_to_worker(
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
                ))
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
                Box::pin(self.route_send_to_worker_or_fail(
                    request_id,
                    &key,
                    transaction_id.clone(),
                    body.clone(),
                    SendComposerProjection::None,
                    TimelineSendEnqueuePayload::Reply {
                        in_reply_to_event_id,
                        document,
                        formatting_options: self.composer_formatting_options,
                    },
                ))
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
                Box::pin(self.route_submission_to_worker(
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
                ))
                .await;
            }
            TimelineCommand::ForwardMessage {
                request_id,
                key,
                source_event_id,
                destination_room_id,
                transaction_id,
            } => {
                Box::pin(self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::ForwardMessage {
                        request_id,
                        source_event_id,
                        destination_room_id,
                        transaction_id,
                    },
                ))
                .await;
            }
            TimelineCommand::LoadMessageSource {
                request_id,
                key,
                event_id,
            } => {
                Box::pin(self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::LoadMessageSource {
                        request_id,
                        event_id,
                    },
                ))
                .await;
            }
            TimelineCommand::RequestRoomKey {
                request_id,
                key,
                event_id,
                origin,
            } => {
                Box::pin(self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::RequestRoomKey {
                        request_id: Some(request_id),
                        event_id,
                        origin,
                    },
                ))
                .await;
            }
            TimelineCommand::RequestLateDecryption { request_id, key } => {
                Box::pin(self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::RequestLateDecryption {
                        request_id: Some(request_id),
                        trigger: crate::room_key_receive::RECEIVE_SUMMARY_TRIGGER_MANUAL,
                    },
                ))
                .await;
            }
            TimelineCommand::RetrySend {
                request_id,
                key,
                transaction_id,
            } => {
                Box::pin(self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::RetrySend {
                        request_id,
                        transaction_id,
                    },
                ))
                .await;
            }
            TimelineCommand::CancelSend {
                request_id,
                key,
                transaction_id,
            } => {
                Box::pin(self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::CancelSend {
                        request_id,
                        transaction_id,
                    },
                ))
                .await;
            }
            TimelineCommand::UploadAndSendMedia {
                request_id,
                key,
                transaction_id,
                request,
                ..
            } => {
                Box::pin(self.route_media_send_to_worker_or_fail(
                    request_id,
                    &key,
                    transaction_id.clone(),
                    TimelineSendEnqueuePayload::Media {
                        request_id,
                        client_transaction_id: transaction_id,
                        request,
                    },
                ))
                .await;
            }
            TimelineCommand::DownloadMedia {
                request_id,
                key,
                event_id,
                selection,
            } => {
                Box::pin(self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::DownloadMedia {
                        request_id,
                        event_id,
                        selection,
                    },
                ))
                .await;
            }
            TimelineCommand::EditText {
                request_id,
                key,
                event_id,
                document,
            } => {
                self.route_mutation(
                    request_id,
                    &key,
                    super::mutation::Mutation::Edit {
                        event_id,
                        document,
                        formatting: self.composer_formatting_options,
                    },
                );
            }
            TimelineCommand::Redact {
                request_id,
                key,
                event_id,
            } => {
                self.route_mutation(
                    request_id,
                    &key,
                    super::mutation::Mutation::Redact { event_id },
                );
            }
            TimelineCommand::SendReaction {
                request_id,
                key,
                event_id,
                reaction_key,
            } => {
                trace_timeline_route("manager_received", "send_reaction", request_id, &key);
                self.route_mutation(
                    request_id,
                    &key,
                    super::mutation::Mutation::AddReaction {
                        event_id,
                        reaction_key,
                    },
                );
            }
            TimelineCommand::RedactReaction {
                request_id,
                key,
                event_id,
                reaction_key,
                reaction_event_id,
            } => {
                trace_timeline_route("manager_received", "redact_reaction", request_id, &key);
                self.route_mutation(
                    request_id,
                    &key,
                    super::mutation::Mutation::RemoveReaction {
                        event_id,
                        reaction_key,
                        reaction_event_id,
                    },
                );
            }
            TimelineCommand::SendReadReceipt {
                request_id,
                key,
                event_id,
            } => {
                trace_timeline_route("manager_received", "send_read_receipt", request_id, &key);
                Box::pin(self.route_read_command(
                    request_id,
                    key,
                    event_id,
                    ReadCommandKind::Receipt,
                ))
                .await;
            }
            TimelineCommand::SetFullyRead {
                request_id,
                key,
                event_id,
            } => {
                trace_timeline_route("manager_received", "set_fully_read", request_id, &key);
                Box::pin(self.route_read_command(
                    request_id,
                    key,
                    event_id,
                    ReadCommandKind::FullyRead,
                ))
                .await;
            }
            TimelineCommand::SetTyping {
                request_id,
                key,
                is_typing,
            } => {
                trace_timeline_route("manager_received", "set_typing", request_id, &key);
                self.handle_set_typing(request_id, key, is_typing);
            }
            TimelineCommand::LoadLinkPreviews {
                request_id,
                key,
                event_id,
            } => {
                trace_timeline_route("manager_received", "load_link_previews", request_id, &key);
                Box::pin(self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::LoadLinkPreviews {
                        request_id,
                        event_id,
                    },
                ))
                .await;
            }
            TimelineCommand::HideLinkPreview {
                request_id,
                key,
                event_id,
            } => {
                Box::pin(self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::HideLinkPreview {
                        request_id,
                        event_id,
                    },
                ))
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
                    let _ = Box::pin(handle.send(TimelineActorMessage::LinkPreviewPolicyChanged {
                        unencrypted_global_enabled,
                        encrypted_global_enabled,
                        room_enabled,
                    }))
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
    /// Full unsubscribe bookkeeping for one key: generation invalidation and
    /// quiescence, local-read correlation cleanup, retained send-completion
    /// cleanup, and actor-resource lease release. Session residency is
    /// intentionally independent and is never removed here.
    pub(super) async fn unsubscribe_timeline(&mut self, key: &TimelineKey) {
        // #1146: cancel and settle SDK construction before rolling back its
        // lease/activation, without publishing a cancellation failure terminal.
        if let Some(pending) = self.focused_builds.cancel(key).await {
            if pending.lease_added
                && let Some(room_id) = &pending.lease_room_id
            {
                self.release_room_lease(room_id);
            }
            self.timeline_actor_generations
                .restore_failed_activation(key, pending.activation);
        }
        // Drop the actor handle, which cancels its relay task and drops
        // the SDK Timeline handle — no dedicated success event per spec.
        if matches!(key.kind, TimelineKind::Room { .. }) {
            self.clear_thread_root_projections_for_room(key).await;
            // #1494: the generation is invalidated, so the retired observer
            // cannot project after this.
            let _ = self
                .emit_action_reliable(AppAction::SenderDeviceAdvisoriesCleared {
                    room_id: key.room_id().to_owned(),
                })
                .await;
        } else {
            self.timeline_actor_generations
                .invalidate_and_quiesce(key)
                .await;
        }
        let removed_actor = self.timelines.remove(key);
        if removed_actor.is_some() {
            self.read_workers.remove_local_read_correlation(key);
        }
        self.send_completion
            .lock()
            .expect("send completion coordinator lock must not be poisoned")
            .drop_direct_retained_for_key(key);
        // Release the actor-resource lease only when an actor was
        // actually removed.
        if removed_actor.is_some()
            && let Ok(room_id) = key.room_id().parse::<OwnedRoomId>()
        {
            self.release_room_lease(&room_id);
        }
    }

    /// #1060: retire every main-pane focused timeline AppActor no longer
    /// desires. Driven by the retained navigation demand rather than a
    /// mailbox `Unsubscribe`, so AppActor never waits for admission and a
    /// delayed key-only cleanup cannot remove a newer owner of the same key.
    pub(super) async fn retire_undesired_focused_timelines(
        &mut self,
        desired: Option<&TimelineKey>,
    ) {
        let mut obsolete = self
            .timelines
            .keys()
            .filter(|key| matches!(key.kind, TimelineKind::Focused { .. }))
            .filter(|key| Some(*key) != desired)
            .cloned()
            .collect::<Vec<_>>();
        // #1146: an in-flight focused build is retired with its owner.
        for key in self.focused_builds.pending_keys() {
            if Some(key) != desired && !obsolete.contains(key) {
                obsolete.push(key.clone());
            }
        }
        for key in obsolete {
            self.unsubscribe_timeline(&key).await;
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
        // A selection command may already be queued when a leave is acknowledged.
        // Do not let it recreate an actor or room subscription until rejoin.
        if key
            .room_id()
            .parse::<OwnedRoomId>()
            .ok()
            .is_some_and(|room_id| self.room_leave_states.contains_key(&room_id))
        {
            self.emit_subscription_failure(
                request_id,
                &key,
                TimelineFailureKind::NotSubscribed,
                emit_failure_terminal,
            )
            .await;
            return;
        }
        // #1146: a repeated subscribe while the focused SDK build is in flight
        // reuses that build instead of restarting it. #1465: only the admitted
        // navigation owner of the key takes over the projection correlation; a
        // renderer recovery/replay subscribe is an observer.
        let navigation_owner = self
            .navigation_projection_rx
            .as_ref()
            .is_some_and(|demand| demand.borrow().owns_focused(&key, request_id));
        if self
            .focused_builds
            .coalesce(&key, request_id, emit_failure_terminal, navigation_owner)
        {
            return;
        }

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
        // #1146: a focused SDK build may wait on remote context or the
        // focused-cache lock; the manager waits for it only through a bounded,
        // fenced completion handled in `handle_focused_build_completion`.
        if matches!(key.kind, TimelineKind::Focused { .. }) {
            match self.focused_build_future(&key, activation.generation) {
                Ok(build) => self.focused_builds.start(
                    key,
                    activation,
                    request_id,
                    emit_failure_terminal,
                    lease_room_id,
                    lease_added,
                    reconcile_trigger,
                    build,
                ),
                Err(kind) => {
                    self.roll_back_failed_subscribe(
                        request_id,
                        &key,
                        activation,
                        lease_added.then_some(lease_room_id).flatten(),
                        reconcile_trigger,
                        kind,
                        emit_failure_terminal,
                    )
                    .await;
                }
            }
            return;
        }
        let subscription_generation = self.current_subscription_generation();
        match Box::pin(self.build_timeline_actor_handle(
            request_id,
            &key,
            activation.generation,
            subscription_generation,
            initial_backfill,
        ))
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
                if lease_added && let Some(room_id) = &lease_room_id {
                    self.release_room_lease(room_id);
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

    /// #1394: content-free snapshot of the resident timeline set, sampled by
    /// the diagnostics export. Reads the manager's own handles only and never
    /// awaits an actor, so a busy mailbox cannot hold the export.
    pub(super) fn residency_diagnostics(&self) -> TimelineResidencyDiagnostics {
        fold_residency_actors(
            self.session_subscribed_rooms.len(),
            self.room_list_service
                .as_ref()
                .map_or(0, |service| service.actual_subscribed_rooms().len()),
            self.timelines.iter().map(|(key, handle)| {
                (
                    &key.kind,
                    handle.position_rx.as_ref().map(|position| {
                        let position = position.borrow();
                        (
                            position.ranks.len(),
                            position.resident_item_struct_bytes,
                            position.resident_item_text_bytes,
                        )
                    }),
                )
            }),
        )
    }

    fn current_subscription_generation(&self) -> Option<u64> {
        self.room_list_service
            .as_ref()
            .map(|service| service.subscription_generation().get())
    }

    /// The focused-build counterpart of `handle_subscribe`'s success arm.
    async fn install_subscribed_actor(&mut self, key: &TimelineKey, handle: TimelineActorHandle) {
        self.emit_timeline_subscribed_action(key).await;
        if let Some(previous) = self.timelines.insert(key.clone(), handle) {
            previous.stop().await;
        }
        self.replay_retained_room_subscription_checkpoint(key).await;
        self.retry_pending_send_hydrations(key);
        record_subscribe_stage("subscribed_done", None);
    }

    /// The focused-build counterpart of `handle_subscribe`'s failure arm.
    #[expect(
        clippy::too_many_arguments,
        reason = "the admitted subscribe state is rolled back as one unit"
    )]
    async fn roll_back_failed_subscribe(
        &mut self,
        request_id: RequestId,
        key: &TimelineKey,
        activation: super::navigation::TimelineActorGenerationActivation,
        added_lease_room_id: Option<OwnedRoomId>,
        reconcile_trigger: SubscriptionReconcileTrigger,
        kind: TimelineFailureKind,
        emit_failure_terminal: bool,
    ) {
        if let Some(room_id) = &added_lease_room_id {
            self.release_room_lease(room_id);
        }
        // Keep session residency after a failed actor build; only the
        // actor-resource lease is rolled back.
        self.reconcile_subscriptions(reconcile_trigger).await;
        self.timeline_actor_generations
            .restore_failed_activation(key, activation);
        self.emit_subscription_failure(request_id, key, kind, emit_failure_terminal)
            .await;
    }

    /// Prepare the SDK `TimelineFocus::Event` build as an owned future. Only
    /// synchronous validation happens here; the SDK work runs in the
    /// supervised task.
    fn focused_build_future(
        &self,
        key: &TimelineKey,
        actor_generation: u64,
    ) -> Result<
        futures_util::future::BoxFuture<
            'static,
            Result<PreparedFocusedTimeline, FocusedBuildFailure>,
        >,
        TimelineFailureKind,
    > {
        let (room_id, focus) = focused_timeline_focus(key)?;
        #[cfg(any(test, feature = "test-hooks"))]
        if self.session.is_none() {
            #[cfg(test)]
            let gate = self.focused_builds.test_gate.clone();
            return Ok(Box::pin(async move {
                #[cfg(test)]
                if let Some(gate) = gate {
                    gate().await.map_err(|_| FocusedBuildFailure::Sdk)?;
                }
                Ok(PreparedFocusedTimeline::TestActor)
            }));
        }
        let Some(session) = &self.session else {
            return Err(TimelineFailureKind::NotSubscribed);
        };
        let Some(room) = session.client().get_room(&room_id) else {
            return Err(TimelineFailureKind::Sdk);
        };
        let cache_key = match &focus {
            TimelineFocus::Event {
                target,
                num_context_events,
                thread_mode,
                ..
            } => Some((target.clone(), *num_context_events, *thread_mode)),
            _ => None,
        };
        let client = session.client().clone();
        Ok(Box::pin(async move {
            record_subscribe_stage("build_begin", None);
            let build_started = Some(startup_trace::now());
            let sdk_started = executor::Instant::now();
            let observer: matrix_sdk::event_cache::focused_diagnostics::FocusedBuildObserver =
                Arc::new(move |stage, outcome, duration_ms| {
                    koushi_diagnostics::record(
                        koushi_diagnostics::DiagnosticEvent::new(
                            koushi_diagnostics::DiagnosticLevel::Debug,
                            "core.focused_build",
                            stage,
                        )
                        .field(koushi_diagnostics::DiagnosticField::count(
                            "build",
                            actor_generation,
                        ))
                        .field(koushi_diagnostics::DiagnosticField::token(
                            "outcome", outcome,
                        ))
                        .field(koushi_diagnostics::DiagnosticField::count(
                            "duration_ms",
                            duration_ms,
                        )),
                    );
                });
            let timeline_result = Box::pin(
                matrix_sdk::event_cache::focused_diagnostics::observe_focused_build(
                    observer,
                    koushi_timeline_builder(&room, focus).build(),
                ),
            )
            .await;
            startup_trace::trace_phase(StartupPhase::TimelineBuild, build_started);
            record_subscribe_stage("build_done", None);
            // The actual SDK-internal wait for a completed build. Cancellation
            // is recorded by the supervisor when the underlying future is dropped.
            record_focused_build(
                if timeline_result.is_ok() {
                    "sdk_build_done"
                } else {
                    "sdk_build_failed"
                },
                actor_generation,
                Some(sdk_started.elapsed().as_millis()),
            );
            let timeline = timeline_result.map_err(|_| FocusedBuildFailure::Sdk)?;
            // #1676: the build just memoized this cache under the same key, so
            // the lookup returns it without another `/context` request.
            let focused_cache = match cache_key {
                Some((target, num_context_events, thread_mode)) => client
                    .event_cache()
                    .event_focused(&room_id, &target, thread_mode.into(), num_context_events)
                    .await
                    .ok()
                    .map(|(cache, _drop_handles)| cache),
                None => None,
            };
            Ok(PreparedFocusedTimeline::Sdk(
                Arc::new(timeline),
                focused_cache,
            ))
        }))
    }

    /// Finish or roll back one focused build. Only the build still pending
    /// for its exact activation may install; anything else is discarded.
    pub(super) async fn handle_focused_build_completion(
        &mut self,
        completion: FocusedBuildCompletion,
    ) {
        let Some(pending) = self.focused_builds.take_current(&completion).await else {
            return;
        };
        let key = completion.key;
        if self.timeline_actor_generations.current_generation(&key)
            != Some(pending.activation.generation)
        {
            // Defensive: ownership moved without cancelling this build.
            if pending.lease_added
                && let Some(room_id) = &pending.lease_room_id
            {
                self.release_room_lease(room_id);
            }
            record_focused_build("stale_discarded", pending.activation.generation, None);
            return;
        }
        let result = match completion.result {
            Ok(PreparedFocusedTimeline::Sdk(timeline, focused_cache)) => match self.session.clone()
            {
                Some(session) => {
                    let subscription_generation = self.current_subscription_generation();
                    let handle = Box::pin(self.spawn_timeline_actor(
                        session,
                        pending.request_id,
                        &key,
                        timeline,
                        pending.activation.generation,
                        subscription_generation,
                    ))
                    .await;
                    if let Some(cache) = focused_cache {
                        let _ = handle
                            .tx
                            .send(TimelineActorMessage::BindFocusedCache(cache))
                            .await;
                    }
                    Ok(handle)
                }
                None => Err(FocusedBuildFailure::Sdk),
            },
            #[cfg(any(test, feature = "test-hooks"))]
            Ok(PreparedFocusedTimeline::TestActor) => {
                Ok(Self::room_subscription_residency_test_actor_handle())
            }
            Err(failure) => Err(failure),
        };
        record_focused_build_settled(&pending, result.as_ref().map(|_| ()).map_err(|f| *f));
        match result {
            Ok(handle) => {
                self.install_subscribed_actor(&key, handle).await;
                #[cfg(test)]
                self.publish_test_actor_focused_projection(
                    &key,
                    pending.activation.generation,
                    pending.request_id,
                );
            }
            Err(failure) => {
                // #1465: each observer that coalesced into the failed build
                // still receives its own failure terminal.
                for (observer, emit_failure_terminal) in &pending.observers {
                    if *emit_failure_terminal {
                        self.emit_failure(
                            *observer,
                            CoreFailure::TimelineOperationFailed {
                                kind: failure.timeline_failure_kind(),
                            },
                        );
                    }
                }
                self.roll_back_failed_subscribe(
                    pending.request_id,
                    &key,
                    pending.activation,
                    pending
                        .lease_added
                        .then_some(pending.lease_room_id)
                        .flatten(),
                    pending.reconcile_trigger,
                    failure.timeline_failure_kind(),
                    pending.emit_failure_terminal,
                )
                .await;
            }
        }
    }
    /// Await `future` inline while still polling every manager-owned worker
    /// set.
    ///
    /// Send-enqueue workers, focused-build completions, read workers and their
    /// retry wakes, and the global send-completion observer have no poller
    /// other than this manager. Some take SDK locks (send-enqueue workers reach
    /// the event cache through `load_or_fetch_event`). Tokio locks hand a
    /// released permit to the next queued waiter even when nobody polls it, so
    /// an inline manager await on the same lock would wait forever on a permit
    /// owned by a parked worker. Any inline manager await that can block on an
    /// SDK lock must go through this helper. Completions are applied after
    /// `future` settles, in arrival order.
    pub(super) async fn await_driving_manager_workers<F: std::future::Future>(
        &mut self,
        future: F,
    ) -> F::Output {
        #[expect(
            clippy::large_enum_variant,
            reason = "short-lived value moved once; boxing would add an allocation per completion"
        )]
        enum Driven {
            SendEnqueue(super::outbound_send::SendEnqueueWorkerCompletion),
            FocusedBuild(super::focused_build::FocusedBuildCompletion),
            Read(super::read_state::ReadWorkerCompletion),
            Typing(super::typing::TypingCompletion),
        }
        // Boxed: the wrapped SDK futures (timeline build, actor spawn) are large,
        // and nesting them inline in this state machine overflows the stack.
        let mut future = Box::pin(future);
        let mut driven = Vec::new();
        let output = loop {
            tokio::select! {
                biased;
                output = &mut future => break output,
                worker = self.send_enqueue_workers.tasks.next(),
                    if !self.send_enqueue_workers.tasks.is_empty() => {
                    driven.extend(worker.map(Driven::SendEnqueue));
                }
                completion = self.focused_builds.tasks.next(),
                    if !self.focused_builds.tasks.is_empty() => {
                    driven.extend(completion.map(Driven::FocusedBuild));
                }
                completion = self.read_workers.tasks.next(),
                    if !self.read_workers.tasks.is_empty() => {
                    driven.extend(completion.map(Driven::Read));
                }
                retry = self.read_workers.retry_tasks.next(),
                    if !self.read_workers.retry_tasks.is_empty() => {
                    driven.extend(retry.map(Driven::Read));
                }
                completion = self.typing_lane.tasks.next(),
                    if !self.typing_lane.tasks.is_empty() => {
                    driven.extend(completion.map(Driven::Typing));
                }
                _ = super::outbound_send::poll_global_send_completion_observer(
                    &mut self.global_send_completion_observer_future,
                ) => {
                    self.global_send_completion_observer_future = None;
                }
            }
        };
        for completion in driven {
            match completion {
                Driven::SendEnqueue(completion) => {
                    self.handle_send_enqueue_worker_completion(completion).await;
                }
                Driven::FocusedBuild(completion) => {
                    // Installing a focused build spawns its actor through this
                    // helper again; box the cycle.
                    Box::pin(self.handle_focused_build_completion(completion)).await;
                }
                Driven::Read(completion) => self.handle_read_worker_completion(completion).await,
                Driven::Typing(completion) => self.handle_typing_completion(completion),
            }
        }
        output
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
            TimelineKind::Focused { .. } => focused_timeline_focus(key)?.1,
        };

        #[cfg(any(test, feature = "test-hooks"))]
        if self.session.is_none() {
            return Ok(Self::room_subscription_residency_test_actor_handle());
        }

        let Some(session) = self.session.clone() else {
            return Err(TimelineFailureKind::NotSubscribed);
        };
        let client = session.client();
        let room = match client.get_room(&room_id) {
            Some(room) => room,
            None => return Err(TimelineFailureKind::Sdk),
        };

        trace("build_begin");
        let build_started = Some(startup_trace::now());
        let timeline_result = Box::pin(
            self.await_driving_manager_workers(koushi_timeline_builder(&room, focus).build()),
        )
        .await;
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
            let (initial_items, _) = self
                .await_driving_manager_workers(timeline.subscribe())
                .await;
            if initial_items.is_empty() {
                let _permit = self
                    .account_work
                    .acquire(AccountWorkKind::ExplicitPagination)
                    .await;
                let TimelineKind::Thread { root_event_id, .. } = &key.kind else {
                    unreachable!()
                };
                let root_event_id = matrix_sdk::ruma::EventId::parse(root_event_id)
                    .map_err(|_| TimelineFailureKind::Sdk)?;
                if !self
                    .await_driving_manager_workers(hydrate_initial_thread(
                        &timeline,
                        &client,
                        &room_id,
                        &root_event_id,
                    ))
                    .await?
                {
                    return Err(TimelineFailureKind::Sdk);
                }
            }
        }

        Ok(Box::pin(self.spawn_timeline_actor(
            session,
            request_id,
            key,
            timeline,
            actor_generation,
            subscription_generation,
        ))
        .await)
    }

    async fn spawn_timeline_actor(
        &mut self,
        session: Arc<MatrixClientSession>,
        request_id: RequestId,
        key: &TimelineKey,
        timeline: Arc<matrix_sdk_ui::timeline::Timeline>,
        actor_generation: u64,
        subscription_generation: Option<u64>,
    ) -> TimelineActorHandle {
        record_subscribe_stage("spawn_begin", None);
        // The actor's initial load subscribes the event cache and may paginate.
        let spawn = TimelineActor::spawn(
            key.clone(),
            timeline,
            session,
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
            self.hide_redacted,
            Arc::clone(&self.timeline_actor_generations),
            actor_generation,
            subscription_generation,
            Arc::clone(&self.send_completion),
            self.terminal_ingress.clone(),
            self.msg_tx.clone(),
        );
        let handle = Box::pin(self.await_driving_manager_workers(spawn)).await;
        if let Some(tx) = &self.history_recovery {
            let _ = handle
                .tx
                .send(TimelineActorMessage::BindHistoryRecovery(tx.clone()))
                .await;
        }
        record_subscribe_stage("spawn_done", None);
        handle
    }
    /// Admit a room-scoped typing notice into the session-owned lane (#1481).
    /// Typing never enters a `TimelineActor` mailbox, so a slow typing request
    /// cannot stall that actor's pending-send publication or diff batches.
    fn handle_set_typing(&mut self, request_id: RequestId, key: TimelineKey, is_typing: bool) {
        if let Err(kind) = self.typing_lane.admit(request_id, key, is_typing) {
            self.emit_failure(request_id, CoreFailure::TimelineOperationFailed { kind });
        }
    }

    pub(super) fn handle_typing_completion(&mut self, completion: super::typing::TypingCompletion) {
        for settlement in self.typing_lane.settle(completion) {
            self.emit_typing_settlement(settlement);
        }
    }

    /// One correlated terminal per admitted typing request: `TypingSet` on
    /// success, `OperationFailed` otherwise.
    fn emit_typing_settlement(&self, settlement: TypingLaneSettlement) {
        let (request_id, key, is_typing, failure) = settlement.into_parts();
        match failure {
            None => self.emit(CoreEvent::LiveSignals(LiveSignalsEvent::TypingSet {
                request_id,
                key,
                is_typing,
            })),
            Some(kind) => {
                self.emit_failure(request_id, CoreFailure::TimelineOperationFailed { kind })
            }
        }
    }

    /// Cancel every in-flight typing task and settle every admitted request.
    /// Used by ordered shutdown and by the fail-safe drop, so no admitted
    /// request is left without a terminal.
    pub(super) fn settle_typing_lane(&mut self) {
        for settlement in self.typing_lane.settle_all() {
            self.emit_typing_settlement(settlement);
        }
    }
    fn route_mutation(
        &mut self,
        request_id: RequestId,
        key: &TimelineKey,
        mutation: super::mutation::Mutation,
    ) {
        let context =
            self.timelines
                .get(key)
                .and_then(|handle| match handle.enqueue_context.as_ref() {
                    Some(super::outbound_send::TimelineSendEnqueueContext::Matrix(context)) => {
                        Some(super::mutation::MutationContext {
                            key: key.clone(),
                            generation: context.actor_generation?,
                            gate: self.timeline_actor_generations.clone(),
                            timeline: context.timeline.clone()?,
                            account_work: self.account_work.clone(),
                            sent_event_txns: context.sent_event_txns.clone(),
                        })
                    }
                    _ => None,
                });
        if let Some(context) = context {
            self.mutation_lane
                .admit(context, request_id, mutation, self.event_tx.clone());
        } else {
            self.emit_failure(
                request_id,
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::NotSubscribed,
                },
            );
        }
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
        if let Some(action) = timeline_subscription_failed_action(key, kind) {
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

/// Parse a Focused key's room and SDK event focus without touching the
/// session.
fn focused_timeline_focus(
    key: &TimelineKey,
) -> Result<(OwnedRoomId, TimelineFocus), TimelineFailureKind> {
    let TimelineKind::Focused { room_id, event_id } = &key.kind else {
        return Err(TimelineFailureKind::Sdk);
    };
    let room_id = OwnedRoomId::try_from(room_id.as_str()).map_err(|_| TimelineFailureKind::Sdk)?;
    let target = matrix_sdk::ruma::EventId::parse(event_id.as_str())
        .map_err(|_| TimelineFailureKind::Sdk)?;
    Ok((
        room_id,
        TimelineFocus::Event {
            target,
            num_context_events: 20,
            thread_mode: matrix_sdk_ui::timeline::TimelineEventFocusThreadMode::Automatic {
                hide_threaded_events: false,
            },
        },
    ))
}

pub(super) fn internal_timeline_request_id() -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(0),
        sequence: 0,
    }
}

#[cfg(test)]
mod thread_mention_projection_tests;

#[cfg(test)]
mod thread_hydration_tests;

#[cfg(test)]
mod return_to_live_tests;
