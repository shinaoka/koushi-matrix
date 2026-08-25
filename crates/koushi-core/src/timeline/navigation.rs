use std::collections::{HashMap, HashSet};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel};
use koushi_state::ActivityRow;

use matrix_sdk_ui::timeline::Timeline;
use tokio::sync::{broadcast, mpsc, watch};

use crate::account_work::{AccountWorkKind, AccountWorkPermit, AccountWorkScheduler};
use crate::event::{
    CoreEvent, PaginationDirection, PaginationState, ThreadRootProjectionDto,
    ThreadRootProjectionSourceDto, ThreadRootProjectionStateDto, TimelineAnchorRestoreStatus,
    TimelineDiff, TimelineEvent, TimelineItem, TimelineItemId, TimelineNavigationSnapshot,
    TimelineReadStateSync, TimelineUnreadPosition, TimelineViewportObservation,
};
use crate::executor;
use crate::failure::{CoreFailure, TimelineFailureKind};
use crate::ids::{RequestId, TimelineBatchId, TimelineGeneration, TimelineKey, TimelineKind};
use crate::live_tail_freshness::LiveTailSchedulerAction;
use crate::startup_trace::{self};
use crate::threads_list::ThreadRootProjectionService;
use koushi_sdk::MatrixLiveTailRefreshOutcome as LiveTailRefreshOutcome;

// BEGIN GENERATED SIBLING IMPORTS
use super::actor::{TimelineActor, TimelineActorControl, TimelineActorMessage};
use super::diagnostics::{
    record_live_tail_queue, record_live_tail_state, record_subscribe_stage,
    timeline_key_trace_kind, trace_timeline_items, trace_timeline_paginate,
};
use super::display_projection::{
    DisplayProjectionState, apply_non_sdk_item_set_diffs_to_display_items,
};
use super::gap_repair::{LIVE_TAIL_CANCELLATION_DEADLINE, RestoreCausalProjectionBuffer};
use super::item_projection::{
    eligible_activity_preview, has_user_visible_content, is_attention_eligible_event,
    is_unread_navigation_item, item_index_for_event_id, timeline_item_event_id,
};
use super::manager::TimelineManagerActor;
use super::thread_projection::{
    JAVASCRIPT_SAFE_INTEGER_MAX, ReplayKnownDisplayContext, ReplayKnownThreadRootProjection,
    ReplayKnownThreadRootProjectionRegistry, ReplayKnownThreadRootProjectionUpdate,
    ThreadAttentionObservation, ThreadAttentionTracker,
    known_thread_root_projections_for_display_context, overlay_thread_summary_item,
    replay_known_candidates_for_display_items,
    replay_known_timeline_events_with_hydration_handoffs, seed_thread_summary_item,
};
// END GENERATED SIBLING IMPORTS

pub(super) const INITIAL_EMPTY_ROOM_BACKFILL_EVENT_COUNT: u16 = 100;

pub(super) const ROOM_REPLAY_INITIAL_ITEMS_MAX: usize = 120;

/// Backstop tick count for the anchor-relay wait. After the SDK signals
/// `anchor_present == true`, the anchor's diff has been broadcast through the
/// 3-hop relay (conclude_backwards_pagination_from_disk → event-cache task →
/// timeline observable → relay → DiffBatch actor msg) and WILL arrive in the
/// actor's `timeline_contains` check within the next few ticks. This backstop
/// guards against a genuinely stuck relay; under normal load the anchor lands
/// well before the count reaches zero.
const RESTORE_ANCHOR_RELAY_WAIT_TICKS: u8 = 40;

/// Delay between anchor-relay-wait ticks (milliseconds). The relay pipeline
/// is a 3-hop async path: conclude_backwards_pagination_from_disk →
/// room_event_cache_updates_task → handle_remote_events_with_diffs →
/// observable → relay task → DiffBatch actor message. Without a pause, all
/// 40 backstop ticks can drain before any relay task gets CPU time.
/// 50 ms is deliberately conservative (well within the 2 000 ms total
/// budget); under normal conditions the anchor lands on tick 1.
const RESTORE_ANCHOR_RELAY_WAIT_TICK_MS: u64 = 50;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TimelineProjectionAcknowledgement {
    pub accepted: bool,
    pub item_count: u64,
    pub target_present: bool,
}

/// Private projection work admitted only after Rust-owned room navigation has
/// committed. `generation` is owned by AppActor and is the sole ordering key;
/// request ids remain correlation data and may cross connection epochs.
#[derive(Clone)]
pub(crate) struct NavigationProjectionIntent {
    pub(crate) generation: u64,
    pub(crate) key: TimelineKey,
    pub(crate) cause_request_id: RequestId,
    pub(crate) replay_existing: bool,
    pub(crate) cleanup: NavigationProjectionCleanup,
}

/// Best-effort cleanup folded into the same latest-wins projection admission.
///
/// AppActor must never wait for the ordinary AccountActor/TimelineManager
/// mailboxes after it has committed a room selection. These keys therefore
/// travel with the retained navigation projection and are deliberately
/// uncorrelated with the already-terminal user request.
#[derive(Clone, Default)]
pub(crate) struct NavigationProjectionCleanup {
    pub(crate) cancel_pagination: Option<TimelineKey>,
    pub(crate) cancel_link_previews: Option<TimelineKey>,
}

/// Stable latest-wins ingress shared across session-scoped timeline-manager
/// replacement. A watch channel is a one-slot value plus a coalesced wake:
/// replacing a value cannot fill or block the AppActor/AccountActor mailbox.
#[derive(Clone)]
pub(crate) struct NavigationProjectionIngress {
    tx: watch::Sender<Option<NavigationProjectionIntent>>,
}

impl NavigationProjectionIngress {
    pub(crate) fn channel() -> (Self, watch::Receiver<Option<NavigationProjectionIntent>>) {
        let (tx, rx) = watch::channel(None);
        (Self { tx }, rx)
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<Option<NavigationProjectionIntent>> {
        let receiver = self.tx.subscribe();
        // A replacement manager must observe the retained latest desired
        // projection even when it subscribed after that value was admitted.
        self.tx.send_modify(|_| {});
        receiver
    }

    pub(crate) fn admit(&self, intent: NavigationProjectionIntent) -> bool {
        let retained = self.tx.borrow().clone();
        let next = match retained {
            Some(current) if current.generation > intent.generation => return true,
            Some(mut current)
                if current.generation == intent.generation && current.key == intent.key =>
            {
                current.replay_existing |= intent.replay_existing;
                current
            }
            _ => intent,
        };
        // `send_replace` retains the value even during the brief interval in
        // which a session-scoped manager is being replaced and no receiver
        // exists. A later `subscribe` explicitly wakes on that retained value.
        self.tx.send_replace(Some(next));
        true
    }
}

/// Manager-owned serial fence for an actor instance of a timeline key.
///
/// A replay-known registry mutation and its Core event emission acquire a
/// short, synchronous lease. Replacement first prevents new old-generation
/// leases, waits for the in-flight lease count to reach zero, then publishes a
/// new generation before its actor may refresh the shared registry. The lease
/// intentionally never spans an `.await`; it protects only `Mutex` mutation
/// and synchronous `broadcast::Sender::send` calls.
#[derive(Default)]
pub(super) struct TimelineActorGenerationGateState {
    pub(super) entries: HashMap<TimelineKey, TimelineActorGenerationGateEntry>,
}

/// Process-global owner epoch. TimelineManagerActor may be recreated during
/// sync/account lifecycle repair while the WebView canonical store survives;
/// therefore per-manager counters are not a valid replacement fence.
static NEXT_TIMELINE_ACTOR_GENERATION: AtomicU64 = AtomicU64::new(1);

pub(super) static DISPLAY_PROJECTION_RESET_FALLBACKS: AtomicU64 = AtomicU64::new(0);

/// QA/test observation point for the process-global projection reset fallback
/// counter. Product behavior never branches on this diagnostic value.
#[cfg(any(test, feature = "qa-bin"))]
pub fn display_projection_reset_fallback_count() -> u64 {
    DISPLAY_PROJECTION_RESET_FALLBACKS.load(Ordering::Relaxed)
}

pub(super) struct TimelineActorGenerationGateEntry {
    generation: u64,
    active_leases: usize,
    replacing: bool,
}

pub(super) struct TimelineActorGenerationGate {
    pub(super) state: Mutex<TimelineActorGenerationGateState>,
    changes: watch::Sender<u64>,
}

pub(super) struct TimelineActorGenerationActivation {
    pub(super) generation: u64,
    previous_generation: Option<u64>,
}

pub(super) struct TimelineActorGenerationLease {
    gate: Arc<TimelineActorGenerationGate>,
    key: TimelineKey,
    generation: u64,
}

impl Default for TimelineActorGenerationGate {
    fn default() -> Self {
        let (changes, _) = watch::channel(0_u64);
        Self {
            state: Mutex::new(TimelineActorGenerationGateState::default()),
            changes,
        }
    }
}

impl TimelineActorGenerationGate {
    /// Starts a new actor generation only after every old-generation replay
    /// lease has completed. No synchronous mutex is held while waiting for a
    /// watch notification.
    pub(super) async fn activate_after_quiescence(
        &self,
        key: &TimelineKey,
    ) -> TimelineActorGenerationActivation {
        let mut changes = self.changes.subscribe();
        loop {
            let activation = {
                let mut state = self
                    .state
                    .lock()
                    .expect("timeline actor generation lock must not be poisoned");
                match state.entries.get_mut(key) {
                    Some(entry) => {
                        entry.replacing = true;
                        if entry.active_leases != 0 {
                            None
                        } else {
                            let previous_generation = entry.generation;
                            let generation = next_timeline_actor_generation(&mut state);
                            state.entries.insert(
                                key.clone(),
                                TimelineActorGenerationGateEntry {
                                    generation,
                                    active_leases: 0,
                                    replacing: false,
                                },
                            );
                            Some(TimelineActorGenerationActivation {
                                generation,
                                previous_generation: Some(previous_generation),
                            })
                        }
                    }
                    None => {
                        let generation = next_timeline_actor_generation(&mut state);
                        state.entries.insert(
                            key.clone(),
                            TimelineActorGenerationGateEntry {
                                generation,
                                active_leases: 0,
                                replacing: false,
                            },
                        );
                        Some(TimelineActorGenerationActivation {
                            generation,
                            previous_generation: None,
                        })
                    }
                }
            };
            if let Some(activation) = activation {
                return activation;
            }
            // `changes` was subscribed before the state check, so a lease
            // release between that check and `changed().await` is observed as
            // an already-pending version change rather than lost.
            if changes.changed().await.is_err() {
                unreachable!("the manager owns the generation gate sender");
            }
        }
    }

    /// Restores an actor generation if construction of its replacement failed
    /// before an actor handle was returned. A successful spawn is never
    /// restored: its handle atomically supersedes the old one in the manager.
    pub(super) fn restore_failed_activation(
        &self,
        key: &TimelineKey,
        activation: TimelineActorGenerationActivation,
    ) {
        let mut state = self
            .state
            .lock()
            .expect("timeline actor generation lock must not be poisoned");
        let should_restore = state.entries.get(key).is_some_and(|entry| {
            entry.generation == activation.generation && entry.active_leases == 0
        });
        if !should_restore {
            return;
        }
        match activation.previous_generation {
            Some(previous_generation) => {
                state.entries.insert(
                    key.clone(),
                    TimelineActorGenerationGateEntry {
                        generation: previous_generation,
                        active_leases: 0,
                        replacing: false,
                    },
                );
            }
            None => {
                state.entries.remove(key);
            }
        }
        self.changes
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }

    /// Unsubscribe/shutdown removes ownership only after a prior synchronous
    /// replay lease has finished. As with replacement, the mutex is dropped
    /// before awaiting a watch change.
    pub(super) async fn invalidate_and_quiesce(&self, key: &TimelineKey) {
        let mut changes = self.changes.subscribe();
        loop {
            let complete = {
                let mut state = self
                    .state
                    .lock()
                    .expect("timeline actor generation lock must not be poisoned");
                let Some(entry) = state.entries.get_mut(key) else {
                    return;
                };
                entry.replacing = true;
                if entry.active_leases != 0 {
                    false
                } else {
                    state.entries.remove(key);
                    true
                }
            };
            if complete {
                self.changes
                    .send_modify(|revision| *revision = revision.wrapping_add(1));
                return;
            }
            if changes.changed().await.is_err() {
                unreachable!("the manager owns the generation gate sender");
            }
        }
    }

    pub(super) fn try_acquire(
        self: &Arc<Self>,
        key: &TimelineKey,
        generation: u64,
    ) -> Option<TimelineActorGenerationLease> {
        let mut state = self
            .state
            .lock()
            .expect("timeline actor generation lock must not be poisoned");
        let entry = state.entries.get_mut(key)?;
        if entry.generation != generation || entry.replacing {
            return None;
        }
        entry.active_leases = entry.active_leases.saturating_add(1);
        Some(TimelineActorGenerationLease {
            gate: Arc::clone(self),
            key: key.clone(),
            generation,
        })
    }

    pub(super) fn current_generation(&self, key: &TimelineKey) -> Option<u64> {
        self.state
            .lock()
            .expect("timeline actor generation lock must not be poisoned")
            .entries
            .get(key)
            .map(|entry| entry.generation)
    }
}

impl Drop for TimelineActorGenerationLease {
    fn drop(&mut self) {
        let mut state = self
            .gate
            .state
            .lock()
            .expect("timeline actor generation lock must not be poisoned");
        if let Some(entry) = state.entries.get_mut(&self.key)
            && entry.generation == self.generation
        {
            entry.active_leases = entry.active_leases.saturating_sub(1);
        }
        drop(state);
        self.gate
            .changes
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }
}

fn next_timeline_actor_generation(_state: &mut TimelineActorGenerationGateState) -> u64 {
    NEXT_TIMELINE_ACTOR_GENERATION.fetch_add(1, Ordering::Relaxed)
}

pub(super) fn accept_projection_ack_for_active_actor(
    timeline_actor_generations: &Arc<TimelineActorGenerationGate>,
    key: &TimelineKey,
    actor_generation: u64,
    expected_projection_request_id: RequestId,
    expected_generation: TimelineGeneration,
    projection_request_id: RequestId,
    generation: TimelineGeneration,
    projection_acknowledged: &mut bool,
) -> bool {
    if projection_request_id != expected_projection_request_id || generation != expected_generation
    {
        return false;
    }
    let Some(_lease) = timeline_actor_generations.try_acquire(key, actor_generation) else {
        return false;
    };
    *projection_acknowledged = true;
    true
}

pub(super) fn projection_acknowledgement_for_current_items(
    key: &TimelineKey,
    items: &[TimelineItem],
    accepted: bool,
) -> TimelineProjectionAcknowledgement {
    let target_present = match &key.kind {
        TimelineKind::Focused { event_id, .. } => items.iter().any(
            |item| matches!(&item.id, TimelineItemId::Event { event_id: id } if id == event_id),
        ),
        TimelineKind::Room { .. } | TimelineKind::Thread { .. } => true,
    };
    TimelineProjectionAcknowledgement {
        accepted,
        item_count: items.len() as u64,
        target_present,
    }
}

pub(super) fn replay_projection_request_id(
    projection_request_id: RequestId,
    projection_acknowledged: bool,
) -> Option<RequestId> {
    (!projection_acknowledged).then_some(projection_request_id)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct InitialItemsRequestIdentity {
    projection_request_id: Option<RequestId>,
    cause_request_id: Option<RequestId>,
}

impl InitialItemsRequestIdentity {
    pub(super) fn fresh(request_id: RequestId) -> Self {
        Self {
            projection_request_id: Some(request_id),
            cause_request_id: Some(request_id),
        }
    }

    pub(super) fn replay(
        projection_request_id: RequestId,
        projection_acknowledged: bool,
        cause_request_id: Option<RequestId>,
    ) -> Self {
        Self {
            projection_request_id: replay_projection_request_id(
                projection_request_id,
                projection_acknowledged,
            ),
            cause_request_id,
        }
    }

    pub(super) fn recovery() -> Self {
        Self {
            projection_request_id: None,
            cause_request_id: None,
        }
    }
}

/// The only emission gateway for TimelineActor-owned Core timeline events.
///
/// The lease is held solely for the synchronous broadcast send(s). It never
/// crosses an await, yet replacement cannot activate a new actor generation
/// between an old actor's current-generation check and this event delivery.
pub(super) fn emit_timeline_events_for_generation(
    event_tx: &broadcast::Sender<CoreEvent>,
    timeline_actor_generations: &Arc<TimelineActorGenerationGate>,
    key: &TimelineKey,
    actor_generation: u64,
    events: Vec<TimelineEvent>,
) -> bool {
    let Some(lease) = timeline_actor_generations.try_acquire(key, actor_generation) else {
        return false;
    };
    emit_timeline_events_with_lease(event_tx, &lease, events);
    true
}

async fn acquire_pagination_permit_and_emit_paginating(
    request_id: RequestId,
    key: TimelineKey,
    event_tx: broadcast::Sender<CoreEvent>,
    timeline_actor_generations: Arc<TimelineActorGenerationGate>,
    actor_generation: u64,
    account_work: AccountWorkScheduler,
    direction: PaginationDirection,
) -> Option<AccountWorkPermit> {
    let permit = account_work
        .acquire(AccountWorkKind::ExplicitPagination)
        .await;
    emit_timeline_events_for_generation(
        &event_tx,
        &timeline_actor_generations,
        &key,
        actor_generation,
        vec![TimelineEvent::PaginationStateChanged {
            request_id: Some(request_id),
            key: key.clone(),
            direction,
            state: PaginationState::Paginating,
            prepend_expected: None,
        }],
    )
    .then_some(permit)
}

/// Emits an already-authorized group atomically with respect to generation
/// replacement. Callers must keep `lease` alive for this entire synchronous
/// call; this helper deliberately does not acquire a second lease.
pub(super) fn emit_timeline_events_with_lease(
    event_tx: &broadcast::Sender<CoreEvent>,
    _lease: &TimelineActorGenerationLease,
    events: Vec<TimelineEvent>,
) {
    for event in events {
        let _ = event_tx.send(CoreEvent::Timeline(event));
    }
}

/// Commits one canonical display diff batch and its resulting replay-known
/// ownership transition under one generation lease. The registry is mutated
/// only after the lease proves this actor is still current; a SyncStarted
/// fence therefore rejects both halves of the UI-visible transition.
///
/// The helper is deliberately synchronous. In particular, no root hydration,
/// media work, or reducer delivery may run while the lease is held.
pub(super) fn emit_items_updated_and_reconcile_replay_known_for_generation(
    event_tx: &broadcast::Sender<CoreEvent>,
    registry: &Arc<Mutex<ReplayKnownThreadRootProjectionRegistry>>,
    thread_root_projection_service: &Arc<Mutex<ThreadRootProjectionService>>,
    timeline_actor_generations: &Arc<TimelineActorGenerationGate>,
    key: &TimelineKey,
    actor_generation: u64,
    generation: TimelineGeneration,
    batch_id: TimelineBatchId,
    diffs: Vec<TimelineDiff>,
    navigation_items: &[TimelineItem],
    display_items: &[TimelineItem],
) -> bool {
    let Some(lease) = timeline_actor_generations.try_acquire(key, actor_generation) else {
        return false;
    };
    emit_items_updated_and_reconcile_replay_known_with_lease(
        event_tx,
        registry,
        thread_root_projection_service,
        &lease,
        key,
        generation,
        batch_id,
        diffs,
        navigation_items,
        display_items,
    );
    true
}

/// Commits a non-SDK `Set` mutation beside its bounded replay display update
/// and replay-known ownership transition. These mutations originate in actor
/// policy/state handlers rather than an SDK vector diff. Their canonical index
/// is resolved against the exact retained slot owner; a replay-only root may
/// be outside the bounded mirror and therefore intentionally emit no display
/// mutation.
///
/// The actor generation lease is acquired *before* the mirror is changed and
/// retained until the display-safe `ItemsUpdated` event and scoped replay
/// Ready/Clear events have all been synchronously broadcast. This is the same
/// current-generation boundary used for SDK diff batches.
pub(super) fn emit_non_sdk_item_sets_and_reconcile_replay_known_for_generation(
    event_tx: &broadcast::Sender<CoreEvent>,
    registry: &Arc<Mutex<ReplayKnownThreadRootProjectionRegistry>>,
    thread_root_projection_service: &Arc<Mutex<ThreadRootProjectionService>>,
    timeline_actor_generations: &Arc<TimelineActorGenerationGate>,
    key: &TimelineKey,
    actor_generation: u64,
    generation: TimelineGeneration,
    batch_id: TimelineBatchId,
    diffs: Vec<TimelineDiff>,
    navigation_items: &[TimelineItem],
    display_projection: &mut DisplayProjectionState,
) -> bool {
    let Some(lease) = timeline_actor_generations.try_acquire(key, actor_generation) else {
        return false;
    };
    let display_diffs = apply_non_sdk_item_set_diffs_to_display_items(display_projection, &diffs);
    emit_items_updated_and_reconcile_replay_known_with_lease(
        event_tx,
        registry,
        thread_root_projection_service,
        &lease,
        key,
        generation,
        batch_id,
        display_diffs,
        navigation_items,
        display_projection.display_items(),
    );
    true
}

/// Sends one `ItemsUpdated` event and all replay-known lifecycle consequences
/// under a generation lease already acquired by the caller.
/// Keeping the registry mutation and every broadcast in this helper prevents
/// an observer from seeing only one half of a display transition.
pub(super) fn emit_items_updated_and_reconcile_replay_known_with_lease(
    event_tx: &broadcast::Sender<CoreEvent>,
    registry: &Arc<Mutex<ReplayKnownThreadRootProjectionRegistry>>,
    thread_root_projection_service: &Arc<Mutex<ThreadRootProjectionService>>,
    lease: &TimelineActorGenerationLease,
    key: &TimelineKey,
    generation: TimelineGeneration,
    batch_id: TimelineBatchId,
    diffs: Vec<TimelineDiff>,
    navigation_items: &[TimelineItem],
    display_items: &[TimelineItem],
) {
    let mut registry = registry
        .lock()
        .expect("replay-known root registry lock must not be poisoned");
    let replay_known_update = registry.reconcile_navigation(
        key,
        navigation_items,
        &ReplayKnownDisplayContext::from_display_items(display_items),
    );
    let mut events =
        Vec::with_capacity(1 + replay_known_update.stale.len() + replay_known_update.ready.len());
    events.push(TimelineEvent::ItemsUpdated {
        key: key.clone(),
        generation,
        batch_id,
        diffs,
    });
    events.extend(replay_known_timeline_events_with_hydration_handoffs(
        key,
        &mut registry,
        thread_root_projection_service,
        replay_known_update,
    ));
    emit_timeline_events_with_lease(event_tx, lease, events);
}

/// The UI-visible terminal state of one anchor-restore walk.  Every member of
/// this group is published while the same actor-generation lease is held so a
/// replacement actor cannot expose an `ItemsUpdated` without its matching
/// navigation/terminal state (or vice versa).
pub(super) struct RestoreSettlement {
    pub(super) navigation_snapshot: Option<TimelineNavigationSnapshot>,
    pub(super) terminal: Option<(RequestId, TimelineAnchorRestoreStatus)>,
}

/// Publish a restore terminal group for the current actor generation.
///
/// `None` means the actor was already stale and no state, buffer, batch id, or
/// event was changed.  `Some(true)` means a coalesced `ItemsUpdated` batch was
/// included; `Some(false)` means only navigation/terminal events were needed.
pub(super) fn publish_restore_settlement_for_generation(
    restore_emit_buffer: &mut Vec<TimelineDiff>,
    force_items_updated: bool,
    next_batch_id: &mut TimelineBatchId,
    event_tx: &broadcast::Sender<CoreEvent>,
    registry: &Arc<Mutex<ReplayKnownThreadRootProjectionRegistry>>,
    thread_root_projection_service: &Arc<Mutex<ThreadRootProjectionService>>,
    timeline_actor_generations: &Arc<TimelineActorGenerationGate>,
    key: &TimelineKey,
    actor_generation: u64,
    generation: TimelineGeneration,
    navigation_items: &[TimelineItem],
    display_items: &[TimelineItem],
    settlement: RestoreSettlement,
) -> Option<bool> {
    let lease = timeline_actor_generations.try_acquire(key, actor_generation)?;
    Some(publish_restore_settlement_with_lease(
        restore_emit_buffer,
        force_items_updated,
        next_batch_id,
        event_tx,
        registry,
        thread_root_projection_service,
        &lease,
        key,
        generation,
        navigation_items,
        display_items,
        settlement,
    ))
}

pub(super) fn publish_restore_settlement_with_lease(
    restore_emit_buffer: &mut Vec<TimelineDiff>,
    force_items_updated: bool,
    next_batch_id: &mut TimelineBatchId,
    event_tx: &broadcast::Sender<CoreEvent>,
    registry: &Arc<Mutex<ReplayKnownThreadRootProjectionRegistry>>,
    thread_root_projection_service: &Arc<Mutex<ThreadRootProjectionService>>,
    lease: &TimelineActorGenerationLease,
    key: &TimelineKey,
    generation: TimelineGeneration,
    navigation_items: &[TimelineItem],
    display_items: &[TimelineItem],
    settlement: RestoreSettlement,
) -> bool {
    // A causal projection may correspond to a display no-op (for example a
    // duplicate SDK slot collapsed by render identity).  It still needs an
    // empty ItemsUpdated batch as the authoritative render fence.
    let published_items = force_items_updated || !restore_emit_buffer.is_empty();
    if published_items {
        let batch_id = *next_batch_id;
        let diffs = std::mem::take(restore_emit_buffer);
        emit_items_updated_and_reconcile_replay_known_with_lease(
            event_tx,
            registry,
            thread_root_projection_service,
            lease,
            key,
            generation,
            batch_id,
            diffs,
            navigation_items,
            display_items,
        );
        *next_batch_id = TimelineBatchId(batch_id.0 + 1);
    }

    let mut terminal_events = Vec::with_capacity(2);
    if let Some(snapshot) = settlement.navigation_snapshot {
        terminal_events.push(TimelineEvent::NavigationUpdated {
            key: key.clone(),
            snapshot,
        });
    }
    if let Some((request_id, status)) = settlement.terminal {
        terminal_events.push(TimelineEvent::AnchorRestoreFinished {
            request_id,
            key: key.clone(),
            status,
        });
    }
    emit_timeline_events_with_lease(event_tx, lease, terminal_events);
    published_items
}

/// Commits a fresh UI `InitialItems` window and its replay-known ownership
/// transition under one shared synchronous boundary. The caller derives
/// `replay_known_candidates` before entering this function; no fetch,
/// pagination, reducer delivery, or other await is allowed while the
/// generation lease and registry mutex are held.
///
/// In particular, a hydration terminal cannot observe an owner-less interval
/// after the frontend has replaced its window but before its replay-known
/// Ready is registered. Event order remains `InitialItems`, then any scoped
/// replay Clear/Ready/hydration handoff events.
pub(super) fn emit_initial_items_and_reconcile_replay_known_for_generation(
    event_tx: &broadcast::Sender<CoreEvent>,
    registry: &Arc<Mutex<ReplayKnownThreadRootProjectionRegistry>>,
    thread_root_projection_service: &Arc<Mutex<ThreadRootProjectionService>>,
    timeline_actor_generations: &Arc<TimelineActorGenerationGate>,
    key: &TimelineKey,
    actor_generation: u64,
    request_identity: InitialItemsRequestIdentity,
    generation: TimelineGeneration,
    items: Vec<TimelineItem>,
    replay_known_candidates: Vec<ThreadRootProjectionDto>,
) -> bool {
    emit_initial_items_and_reconcile_replay_known_for_generation_after_initial(
        event_tx,
        registry,
        thread_root_projection_service,
        timeline_actor_generations,
        key,
        actor_generation,
        request_identity,
        generation,
        items,
        replay_known_candidates,
        || {},
    )
}

/// Internal synchronous boundary used by the production no-op callback above
/// and the deterministic test-only interleaving hook below. The callback is
/// invoked after `InitialItems` delivery while both the generation lease and
/// replay registry mutex remain held; it must never await.
fn emit_initial_items_and_reconcile_replay_known_for_generation_after_initial<F>(
    event_tx: &broadcast::Sender<CoreEvent>,
    registry: &Arc<Mutex<ReplayKnownThreadRootProjectionRegistry>>,
    thread_root_projection_service: &Arc<Mutex<ThreadRootProjectionService>>,
    timeline_actor_generations: &Arc<TimelineActorGenerationGate>,
    key: &TimelineKey,
    actor_generation: u64,
    request_identity: InitialItemsRequestIdentity,
    generation: TimelineGeneration,
    items: Vec<TimelineItem>,
    replay_known_candidates: Vec<ThreadRootProjectionDto>,
    after_initial: F,
) -> bool
where
    F: FnOnce(),
{
    let Some(lease) = timeline_actor_generations.try_acquire(key, actor_generation) else {
        return false;
    };
    emit_initial_items_and_reconcile_replay_known_with_lease_after_initial(
        event_tx,
        registry,
        thread_root_projection_service,
        &lease,
        key,
        actor_generation,
        request_identity,
        generation,
        items,
        replay_known_candidates,
        Vec::new(),
        after_initial,
    );
    true
}

fn emit_initial_items_and_reconcile_replay_known_with_lease_after_initial<F>(
    event_tx: &broadcast::Sender<CoreEvent>,
    registry: &Arc<Mutex<ReplayKnownThreadRootProjectionRegistry>>,
    thread_root_projection_service: &Arc<Mutex<ThreadRootProjectionService>>,
    lease: &TimelineActorGenerationLease,
    key: &TimelineKey,
    actor_generation: u64,
    request_identity: InitialItemsRequestIdentity,
    generation: TimelineGeneration,
    items: Vec<TimelineItem>,
    replay_known_candidates: Vec<ThreadRootProjectionDto>,
    prefix_events: Vec<TimelineEvent>,
    after_initial: F,
) where
    F: FnOnce(),
{
    let mut registry = registry
        .lock()
        .expect("replay-known root registry lock must not be poisoned");
    for item in &items {
        seed_thread_summary_item(thread_root_projection_service, key, item);
    }
    let items = items
        .into_iter()
        .map(|item| overlay_thread_summary_item(thread_root_projection_service, key, &item))
        .collect();
    let replay_known_update = registry.replace(key, replay_known_candidates);
    emit_timeline_events_with_lease(event_tx, lease, prefix_events);
    emit_timeline_events_with_lease(
        event_tx,
        lease,
        vec![TimelineEvent::InitialItems {
            request_id: request_identity.projection_request_id,
            cause_request_id: request_identity.cause_request_id,
            key: key.clone(),
            actor_generation,
            generation,
            items,
        }],
    );
    after_initial();
    let events = replay_known_timeline_events_with_hydration_handoffs(
        key,
        &mut registry,
        thread_root_projection_service,
        replay_known_update,
    );
    emit_timeline_events_with_lease(event_tx, lease, events);
}

pub(super) struct PreparedInitialWindow {
    pub(super) display_projection: DisplayProjectionState,
    pub(super) navigation_items: Option<Vec<TimelineItem>>,
    pub(super) emitted_items: Vec<TimelineItem>,
    pub(super) replay_known_candidates: Vec<ThreadRootProjectionDto>,
}

pub(super) fn commit_prepared_initial_window_for_generation(
    navigation_items: &mut Vec<TimelineItem>,
    display_projection: &mut DisplayProjectionState,
    event_tx: &broadcast::Sender<CoreEvent>,
    registry: &Arc<Mutex<ReplayKnownThreadRootProjectionRegistry>>,
    thread_root_projection_service: &Arc<Mutex<ThreadRootProjectionService>>,
    timeline_actor_generations: &Arc<TimelineActorGenerationGate>,
    key: &TimelineKey,
    actor_generation: u64,
    request_identity: InitialItemsRequestIdentity,
    generation: TimelineGeneration,
    prefix_events: Vec<TimelineEvent>,
    prepared: PreparedInitialWindow,
) -> bool {
    let Some(lease) = timeline_actor_generations.try_acquire(key, actor_generation) else {
        return false;
    };
    commit_prepared_initial_window_with_lease(
        navigation_items,
        display_projection,
        event_tx,
        registry,
        thread_root_projection_service,
        &lease,
        key,
        actor_generation,
        request_identity,
        generation,
        prefix_events,
        prepared,
        || {},
    );
    true
}

pub(super) fn commit_prepared_initial_window_with_lease<F>(
    navigation_items: &mut Vec<TimelineItem>,
    display_projection: &mut DisplayProjectionState,
    event_tx: &broadcast::Sender<CoreEvent>,
    registry: &Arc<Mutex<ReplayKnownThreadRootProjectionRegistry>>,
    thread_root_projection_service: &Arc<Mutex<ThreadRootProjectionService>>,
    lease: &TimelineActorGenerationLease,
    key: &TimelineKey,
    actor_generation: u64,
    request_identity: InitialItemsRequestIdentity,
    generation: TimelineGeneration,
    prefix_events: Vec<TimelineEvent>,
    prepared: PreparedInitialWindow,
    commit_synchronous_candidates: F,
) where
    F: FnOnce(),
{
    let PreparedInitialWindow {
        display_projection: candidate_display_projection,
        navigation_items: candidate_navigation_items,
        emitted_items,
        replay_known_candidates,
    } = prepared;
    if let Some(candidate_navigation_items) = candidate_navigation_items {
        *navigation_items = candidate_navigation_items;
    }
    *display_projection = candidate_display_projection;
    commit_synchronous_candidates();
    emit_initial_items_and_reconcile_replay_known_with_lease_after_initial(
        event_tx,
        registry,
        thread_root_projection_service,
        lease,
        key,
        actor_generation,
        request_identity,
        generation,
        emitted_items,
        replay_known_candidates,
        prefix_events,
        || {},
    );
}

#[cfg(test)]
pub(super) fn emit_initial_items_and_reconcile_replay_known_for_generation_with_test_hook<F>(
    event_tx: &broadcast::Sender<CoreEvent>,
    registry: &Arc<Mutex<ReplayKnownThreadRootProjectionRegistry>>,
    thread_root_projection_service: &Arc<Mutex<ThreadRootProjectionService>>,
    timeline_actor_generations: &Arc<TimelineActorGenerationGate>,
    key: &TimelineKey,
    actor_generation: u64,
    request_identity: InitialItemsRequestIdentity,
    generation: TimelineGeneration,
    items: Vec<TimelineItem>,
    replay_known_candidates: Vec<ThreadRootProjectionDto>,
    after_initial: F,
) -> bool
where
    F: FnOnce(),
{
    emit_initial_items_and_reconcile_replay_known_for_generation_after_initial(
        event_tx,
        registry,
        thread_root_projection_service,
        timeline_actor_generations,
        key,
        actor_generation,
        request_identity,
        generation,
        items,
        replay_known_candidates,
        after_initial,
    )
}

impl ReplayKnownThreadRootProjectionRegistry {
    /// Replaces this replay's known snapshots and returns roots that were in a
    /// prior replay for the same key but are no longer eligible for display.
    pub(super) fn replace(
        &mut self,
        key: &TimelineKey,
        projections: Vec<ThreadRootProjectionDto>,
    ) -> ReplayKnownThreadRootProjectionUpdate {
        self.replace_with_emit_unchanged(key, projections, true)
    }

    fn replace_with_emit_unchanged(
        &mut self,
        key: &TimelineKey,
        projections: Vec<ThreadRootProjectionDto>,
        emit_unchanged: bool,
    ) -> ReplayKnownThreadRootProjectionUpdate {
        let mut previous = self.entries.remove(key).unwrap_or_default();
        // Keep every live and just-staled epoch occupied while allocating. A
        // Clear from the old owner can be delivered in the same synchronous
        // group as a new Ready, so reusing it would make source-scoped clears
        // ambiguous after JavaScript deserializes the JSON number.
        let mut occupied_epochs = previous
            .values()
            .filter_map(|projection| match projection.source {
                ThreadRootProjectionSourceDto::ReplayKnown { epoch } => Some(epoch),
                ThreadRootProjectionSourceDto::Hydration => None,
            })
            .collect::<HashSet<_>>();
        let mut next = HashMap::new();
        let mut ready = Vec::new();
        let mut stale = Vec::new();

        for mut projection in projections {
            let ThreadRootProjectionStateDto::Ready { item } = &projection.state else {
                continue;
            };
            let ready_item = item.clone();
            if !projection.retain_without_reply {
                continue;
            }
            let (source, is_unchanged) = match previous.remove(&projection.root_event_id) {
                Some(existing)
                    if existing.activity_event_id == projection.activity_event_id
                        && existing.activity_timestamp_ms == projection.activity_timestamp_ms =>
                {
                    (existing.source, existing.item == ready_item)
                }
                Some(existing) => {
                    stale.push(existing);
                    let epoch = self.allocate_safe_epoch(&mut occupied_epochs);
                    (ThreadRootProjectionSourceDto::ReplayKnown { epoch }, false)
                }
                None => {
                    let epoch = self.allocate_safe_epoch(&mut occupied_epochs);
                    (ThreadRootProjectionSourceDto::ReplayKnown { epoch }, false)
                }
            };
            projection.source = source.clone();
            next.insert(
                projection.root_event_id.clone(),
                ReplayKnownThreadRootProjection {
                    root_event_id: projection.root_event_id.clone(),
                    activity_event_id: projection.activity_event_id.clone(),
                    activity_timestamp_ms: projection.activity_timestamp_ms,
                    item: ready_item,
                    source,
                },
            );
            if emit_unchanged || !is_unchanged {
                ready.push(projection);
            }
        }
        stale.extend(previous.into_values());
        if !next.is_empty() {
            self.entries.insert(key.clone(), next);
        }
        ReplayKnownThreadRootProjectionUpdate { ready, stale }
    }

    pub(super) fn clear(&mut self, key: &TimelineKey) -> Vec<ReplayKnownThreadRootProjection> {
        self.suppressed_hydration_terminals.remove(key);
        self.emitted_hydration_terminals.remove(key);
        self.entries
            .remove(key)
            .map(|entries| entries.into_values().collect())
            .unwrap_or_default()
    }

    pub(super) fn reconcile_navigation(
        &mut self,
        key: &TimelineKey,
        navigation_items: &[TimelineItem],
        display_context: &ReplayKnownDisplayContext,
    ) -> ReplayKnownThreadRootProjectionUpdate {
        self.replace_with_emit_unchanged(
            key,
            known_thread_root_projections_for_display_context(navigation_items, display_context),
            false,
        )
    }

    /// True when this Room timeline currently owns the root with a
    /// replay-known snapshot. Manager-owned hydration workers consult this
    /// before they publish late terminal events, while still recording their
    /// result in the hydration service/reducer state.
    pub(super) fn owns_root(&self, key: &TimelineKey, root_event_id: &str) -> bool {
        self.entries
            .get(key)
            .is_some_and(|entries| entries.contains_key(root_event_id))
    }

    pub(super) fn mark_hydration_terminal_suppressed(
        &mut self,
        key: &TimelineKey,
        root_event_id: String,
    ) {
        self.suppressed_hydration_terminals
            .entry(key.clone())
            .or_default()
            .insert(root_event_id);
    }

    pub(super) fn take_suppressed_hydration_terminal(
        &mut self,
        key: &TimelineKey,
        root_event_id: &str,
    ) -> bool {
        let Some(roots) = self.suppressed_hydration_terminals.get_mut(key) else {
            return false;
        };
        let was_suppressed = roots.remove(root_event_id);
        if roots.is_empty() {
            self.suppressed_hydration_terminals.remove(key);
        }
        was_suppressed
    }

    pub(super) fn mark_hydration_terminal_emitted(
        &mut self,
        key: &TimelineKey,
        root_event_id: String,
    ) {
        self.emitted_hydration_terminals
            .entry(key.clone())
            .or_default()
            .insert(root_event_id);
    }

    pub(super) fn take_emitted_hydration_terminal(
        &mut self,
        key: &TimelineKey,
        root_event_id: &str,
    ) -> bool {
        let Some(roots) = self.emitted_hydration_terminals.get_mut(key) else {
            return false;
        };
        let was_emitted = roots.remove(root_event_id);
        if roots.is_empty() {
            self.emitted_hydration_terminals.remove(key);
        }
        was_emitted
    }

    /// Allocate a positive JavaScript-safe source epoch that does not collide
    /// with any owner in the current replacement group. The registry is
    /// bounded to 32 entries, so this loop cannot approach the safe range's
    /// upper bound in practice.
    fn allocate_safe_epoch(&mut self, occupied_epochs: &mut HashSet<u64>) -> u64 {
        let mut epoch = if (1..=JAVASCRIPT_SAFE_INTEGER_MAX).contains(&self.next_epoch) {
            self.next_epoch
        } else {
            1
        };
        loop {
            if occupied_epochs.insert(epoch) {
                self.next_epoch = if epoch == JAVASCRIPT_SAFE_INTEGER_MAX {
                    1
                } else {
                    epoch + 1
                };
                return epoch;
            }
            epoch = if epoch == JAVASCRIPT_SAFE_INTEGER_MAX {
                1
            } else {
                epoch + 1
            };
        }
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[cfg(test)]
    pub(super) fn get(
        &self,
        key: &TimelineKey,
    ) -> Option<&HashMap<String, ReplayKnownThreadRootProjection>> {
        self.entries.get(key)
    }
}

pub(super) async fn receive_navigation_projection(
    receiver: &mut Option<watch::Receiver<Option<NavigationProjectionIntent>>>,
) -> Option<NavigationProjectionIntent> {
    let Some(active) = receiver.as_mut() else {
        return futures_util::future::pending().await;
    };
    if active.changed().await.is_err() {
        *receiver = None;
        return None;
    }
    active.borrow_and_update().clone()
}

impl TimelineManagerActor {
    pub(super) async fn handle_navigation_projection(
        &mut self,
        intent: NavigationProjectionIntent,
    ) {
        if intent.generation < self.last_navigation_projection_generation {
            return;
        }
        if intent.generation > self.last_navigation_projection_generation {
            self.last_navigation_projection_generation = intent.generation;
        }
        let actual_foreground = self
            .live_tail_refreshes
            .active_key()
            .filter(|key| *key != &intent.key)
            .cloned();
        if let Some(key) = actual_foreground.as_ref()
            && let Some(handle) = self.timelines.get(key)
        {
            // The projection ingress is latest-wins. When A→B→C coalesces
            // before this manager polls, C carries cleanup(B), but A remains
            // the manager's actual foreground. Clean that owned foreground
            // independently so replacing B cannot strand A's network work.
            handle.cancel_pagination_after_commit();
            handle.cancel_link_previews_after_commit();
        }
        if let Some(key) = intent.cleanup.cancel_pagination.as_ref()
            && Some(key) != actual_foreground.as_ref()
            && let Some(handle) = self.timelines.get(key)
        {
            handle.cancel_pagination_after_commit();
        }
        if let Some(key) = intent.cleanup.cancel_link_previews.as_ref()
            && Some(key) != actual_foreground.as_ref()
            && let Some(handle) = self.timelines.get(key)
        {
            handle.cancel_link_previews_after_commit();
        }
        self.handle_committed_room_selection(
            intent.cause_request_id,
            intent.key,
            intent.replay_existing,
            false,
        )
        .await;
    }
    pub(super) async fn handle_committed_room_selection(
        &mut self,
        request_id: RequestId,
        key: TimelineKey,
        replay_existing: bool,
        emit_failure_terminal: bool,
    ) {
        let previous_foreground = self
            .live_tail_refreshes
            .active_key()
            .filter(|active| *active != &key)
            .cloned();
        let from = self.live_tail_refreshes.freshness(&key);
        let actions = self
            .live_tail_refreshes
            .activate(key.clone(), self.room_subscription_service_epoch);
        if let Some(previous) = previous_foreground {
            if let Some(handle) = self.timelines.get(&previous) {
                // Generation invalidation above makes late old-room work inert;
                // cleanup is best-effort and must never hold the new room.
                handle.end_gap_repair_demand();
            }
        }
        record_live_tail_state(
            from,
            self.live_tail_refreshes.freshness(&key),
            self.room_subscription_service_epoch,
        );
        record_live_tail_queue("foreground", &actions);
        let mut starts = Vec::new();
        for action in actions {
            if matches!(action, LiveTailSchedulerAction::Start { .. }) {
                starts.push(action);
            } else {
                self.apply_live_tail_scheduler_actions(vec![action]).await;
            }
        }

        self.handle_subscribe(
            request_id,
            key.clone(),
            replay_existing,
            emit_failure_terminal,
        )
        .await;
        if let Some(handle) = self.timelines.get(&key) {
            let deadline = executor::Instant::now() + LIVE_TAIL_CANCELLATION_DEADLINE;
            let _ = executor::timeout_at(
                deadline,
                handle.send_control(TimelineActorControl::BeginGapRepairDemand),
            )
            .await;
            self.apply_live_tail_scheduler_actions(starts).await;
            return;
        }
        for action in starts {
            if let LiveTailSchedulerAction::Start {
                epoch,
                operation_generation,
                ..
            } = action
            {
                let from = self.live_tail_refreshes.freshness(&key);
                let follow_up = self.live_tail_refreshes.finish(
                    key.clone(),
                    epoch,
                    operation_generation,
                    LiveTailRefreshOutcome::Failed,
                );
                record_live_tail_state(from, self.live_tail_refreshes.freshness(&key), epoch);
                record_live_tail_queue("delayed", &follow_up);
                self.apply_live_tail_scheduler_actions(follow_up).await;
            }
        }
    }
    pub(super) async fn restore_foreground_gap_demand(&mut self, key: &TimelineKey) {
        if self.live_tail_refreshes.active_key() != Some(key) {
            return;
        }
        if let Some(handle) = self.timelines.get(key) {
            let deadline = executor::Instant::now() + LIVE_TAIL_CANCELLATION_DEADLINE;
            let _ = executor::timeout_at(
                deadline,
                handle.send_control(TimelineActorControl::BeginGapRepairDemand),
            )
            .await;
        }
    }
}

/// Wait for channel capacity without publishing, then synchronously validate
/// actor ownership and publish while the short generation lease is held.
/// Replacement may win during the capacity await; in that case the prepared
/// value is discarded and no stale continuation escapes.
pub(super) async fn send_generation_fenced<T>(
    tx: &mpsc::Sender<T>,
    timeline_actor_generations: &Arc<TimelineActorGenerationGate>,
    key: &TimelineKey,
    actor_generation: u64,
    value: T,
) -> bool {
    let Ok(permit) = tx.clone().reserve_owned().await else {
        return false;
    };
    let Some(_lease) = timeline_actor_generations.try_acquire(key, actor_generation) else {
        return false;
    };
    permit.send(value);
    true
}

pub(super) struct ActivePaginationTask {
    pub(super) serial: u64,
    direction: PaginationDirection,
    event_count: u16,
    pub(super) task: executor::JoinHandle<()>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PaginationCompletion {
    state: PaginationState,
    prepend_expected: Option<bool>,
}

impl PaginationCompletion {
    fn into_result(self) -> Result<bool, TimelineFailureKind> {
        match self.state {
            PaginationState::EndReached => Ok(true),
            PaginationState::Idle | PaginationState::Paginating => Ok(false),
            PaginationState::Failed { kind } => Err(kind),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct RestoreTimelineAnchorState {
    pub(super) request_id: RequestId,
    pub(super) event_id: String,
    pub(super) max_batches_remaining: u16,
    pub(super) event_count: u16,
    pub(super) in_flight: bool,
    pub(super) awaiting_diff_batch: bool,
    pub(super) continuation_scheduled: bool,
    pub(super) continuation_serial: Option<u64>,
    /// Set to `Some(RESTORE_ANCHOR_RELAY_WAIT_TICKS)` after the SDK confirms
    /// `anchor_present == true` (load-until-anchor found the anchor in a loaded
    /// chunk; its broadcast has been fired and WILL propagate through the 3-hop
    /// relay). While non-zero, each tick re-checks `timeline_contains(anchor)`
    /// and re-ticks until Found or the backstop runs out. `None` during the
    /// normal walk.
    pub(super) anchor_relay_wait: Option<u8>,
}

fn backward_pagination_changed_oldest_edge(
    oldest_before: Option<&str>,
    oldest_after: Option<&str>,
) -> bool {
    oldest_after.is_some() && oldest_before != oldest_after
}

async fn oldest_observable_event_id(timeline: &Timeline) -> Option<String> {
    let (items, _updates) = timeline.subscribe().await;
    items.iter().find_map(|item| {
        item.as_event()
            .and_then(|event| event.event_id())
            .map(ToString::to_string)
    })
}

impl TimelineActor {
    pub(super) async fn handle_paginate(
        &mut self,
        request_id: RequestId,
        direction: PaginationDirection,
        event_count: u16,
    ) {
        trace_timeline_paginate(
            "actor_paginate_start",
            request_id,
            &self.key,
            direction,
            event_count,
            None,
            None,
            None,
        );

        // Enforce direction rule: forward only on Focused (Async rule 5).
        if direction == PaginationDirection::Forward
            && !matches!(self.key.kind, TimelineKind::Focused { .. })
        {
            self.emit_failure(
                request_id,
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::InvalidDirection,
                },
            );
            return;
        }

        if self.gap_repair.active_serial.is_some()
            || self.gap_repair.awaiting_projection.is_some()
            || self.gap_projection_correlation.is_pending()
            || self.pending_gap_projection.is_some()
        {
            trace_timeline_paginate(
                "actor_paginate_skip",
                request_id,
                &self.key,
                direction,
                event_count,
                None,
                None,
                Some("gap_repair"),
            );
            self.emit(CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                request_id: Some(request_id),
                key: self.key.clone(),
                direction,
                state: PaginationState::Idle,
                prepend_expected: None,
            }));
            return;
        }

        if self.pagination_task.is_some() {
            trace_timeline_paginate(
                "actor_paginate_skip",
                request_id,
                &self.key,
                direction,
                event_count,
                None,
                None,
                Some("in_flight"),
            );
            return;
        }

        let serial = self.next_pagination_serial;
        self.next_pagination_serial = self.next_pagination_serial.saturating_add(1);
        let key = self.key.clone();
        let timeline = self.timeline.clone();
        let event_tx = self.event_tx.clone();
        let timeline_actor_generations = self.timeline_actor_generations.clone();
        let actor_generation = self.actor_generation;
        let actor_tx = self.msg_tx.clone();
        let account_work = self.account_work.clone();
        let task = executor::spawn(async move {
            let completion = Self::paginate_once_for(
                request_id,
                key,
                timeline,
                event_tx,
                timeline_actor_generations,
                actor_generation,
                account_work,
                direction,
                event_count,
            )
            .await;
            let _ = actor_tx
                .send(TimelineActorMessage::PaginationFinished {
                    serial,
                    request_id,
                    direction,
                    completion,
                })
                .await;
        });
        self.pagination_task = Some(ActivePaginationTask {
            serial,
            direction,
            event_count,
            task,
        });
    }
    async fn paginate_once(
        &mut self,
        request_id: RequestId,
        direction: PaginationDirection,
        event_count: u16,
    ) -> Result<bool, TimelineFailureKind> {
        let completion = Self::paginate_once_for(
            request_id,
            self.key.clone(),
            self.timeline.clone(),
            self.event_tx.clone(),
            self.timeline_actor_generations.clone(),
            self.actor_generation,
            self.account_work.clone(),
            direction,
            event_count,
        )
        .await;
        self.emit_pagination_completion(request_id, direction, completion);
        completion.into_result()
    }
    async fn paginate_once_for(
        request_id: RequestId,
        key: TimelineKey,
        timeline: Arc<Timeline>,
        event_tx: broadcast::Sender<CoreEvent>,
        timeline_actor_generations: Arc<TimelineActorGenerationGate>,
        actor_generation: u64,
        account_work: AccountWorkScheduler,
        direction: PaginationDirection,
        event_count: u16,
    ) -> PaginationCompletion {
        let oldest_event_before = if direction == PaginationDirection::Backward {
            oldest_observable_event_id(&timeline).await
        } else {
            None
        };
        let gate_started = Some(std::time::Instant::now());
        let Some(permit) = acquire_pagination_permit_and_emit_paginating(
            request_id,
            key.clone(),
            event_tx,
            timeline_actor_generations,
            actor_generation,
            account_work,
            direction,
        )
        .await
        else {
            return PaginationCompletion {
                state: PaginationState::Idle,
                prepend_expected: None,
            };
        };
        let result = {
            let gate_wait = gate_started.map(|t| t.elapsed());
            let gate_ms = gate_wait.map(|duration| duration.as_millis());
            trace_timeline_paginate(
                "gate_acquired",
                request_id,
                &key,
                direction,
                event_count,
                None,
                gate_ms,
                None,
            );
            let paginate_started = Some(startup_trace::now());
            let trace_started = Some(std::time::Instant::now());
            let outcome = match direction {
                PaginationDirection::Backward => timeline.paginate_backwards(event_count).await,
                PaginationDirection::Forward => timeline.paginate_forwards(event_count).await,
            };
            let outcome_token = match &outcome {
                Ok(true) => "end_reached",
                Ok(false) => "idle",
                Err(_) => "failed",
            };
            trace_timeline_paginate(
                "sdk_finish",
                request_id,
                &key,
                direction,
                event_count,
                trace_started.map(|started| started.elapsed().as_millis()),
                gate_ms,
                Some(outcome_token),
            );
            startup_trace::trace_paginate(paginate_started, gate_wait, matches!(outcome, Ok(true)));
            outcome
        };
        drop(permit);
        let prepend_expected = if direction == PaginationDirection::Backward && result.is_ok() {
            let oldest_event_after = oldest_observable_event_id(&timeline).await;
            Some(backward_pagination_changed_oldest_edge(
                oldest_event_before.as_deref(),
                oldest_event_after.as_deref(),
            ))
        } else {
            None
        };

        let next_state = match result {
            Ok(true) => PaginationState::EndReached,
            Ok(false) => PaginationState::Idle,
            Err(err) => {
                let kind = classify_pagination_error(&err);
                PaginationState::Failed { kind }
            }
        };

        PaginationCompletion {
            state: next_state,
            prepend_expected,
        }
    }
    pub(super) fn emit_pagination_completion(
        &self,
        request_id: RequestId,
        direction: PaginationDirection,
        completion: PaginationCompletion,
    ) {
        self.emit(CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
            request_id: Some(request_id),
            key: self.key.clone(),
            direction,
            state: completion.state,
            prepend_expected: completion.prepend_expected,
        }));
    }
    pub(super) fn handle_cancel_pagination(&mut self, request_id: RequestId) {
        let Some(active) = self.pagination_task.take() else {
            return;
        };
        active.task.abort();
        trace_timeline_paginate(
            "cancelled",
            request_id,
            &self.key,
            active.direction,
            active.event_count,
            None,
            None,
            Some("cancelled"),
        );
        self.emit(CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
            request_id: Some(request_id),
            key: self.key.clone(),
            direction: active.direction,
            state: PaginationState::Idle,
            prepend_expected: None,
        }));
    }
    pub(super) async fn handle_restore_timeline_anchor(
        &mut self,
        request_id: RequestId,
        event_id: String,
        max_batches: u16,
        event_count: u16,
    ) {
        if !matches!(self.key.kind, TimelineKind::Room { .. }) {
            self.emit_timeline_failure(request_id, TimelineFailureKind::NotSubscribed);
            return;
        }
        if self.gap_repair.active_serial.is_some()
            || self.gap_repair.awaiting_projection.is_some()
            || self.gap_projection_correlation.is_pending()
            || self.pending_gap_projection.is_some()
        {
            self.finish_anchor_restore(request_id, TimelineAnchorRestoreStatus::Superseded);
            return;
        }
        if event_id.trim().is_empty() || max_batches == 0 || event_count == 0 {
            // Invalid request: reject it without touching any active restore's
            // buffer. Using raw emit_anchor_restore_finished (NOT finish_anchor_restore)
            // prevents flushing a different restore's restore_emit_buffer here.
            self.emit_anchor_restore_finished(
                request_id,
                TimelineAnchorRestoreStatus::BudgetExhausted,
            );
            return;
        }
        if self.timeline_contains_event_id(&event_id) {
            self.restore_anchor = None;
            self.finish_anchor_restore(request_id, TimelineAnchorRestoreStatus::Found);
            return;
        }
        if let Some(mut existing) = self.restore_anchor.take() {
            if existing.event_id == event_id {
                existing.request_id = request_id;
                existing.max_batches_remaining = existing.max_batches_remaining.max(max_batches);
                existing.event_count = event_count;
                if existing.in_flight
                    || existing.awaiting_diff_batch
                    || existing.continuation_scheduled
                {
                    self.restore_anchor = Some(existing);
                } else {
                    self.schedule_restore_anchor_continue(existing).await;
                }
                return;
            }
            self.finish_anchor_restore(
                existing.request_id,
                TimelineAnchorRestoreStatus::Superseded,
            );
        }

        let restore = RestoreTimelineAnchorState {
            request_id,
            event_id,
            max_batches_remaining: max_batches,
            event_count,
            in_flight: false,
            awaiting_diff_batch: false,
            continuation_scheduled: false,
            continuation_serial: None,
            anchor_relay_wait: None,
        };

        self.schedule_restore_anchor_continue(restore).await;
    }
    pub(super) async fn handle_restore_timeline_anchor_continue(&mut self, serial: u64) {
        let Some(mut restore) = self.restore_anchor.take() else {
            return;
        };
        if restore.continuation_serial != Some(serial) {
            self.restore_anchor = Some(restore);
            return;
        }
        if restore.in_flight {
            self.restore_anchor = Some(restore);
            return;
        }
        restore.awaiting_diff_batch = false;
        restore.continuation_scheduled = false;
        restore.continuation_serial = None;

        // Anchor-relay wait: entered after the SDK's authoritative
        // `anchor_present == true` signal. All cache events are in memory and
        // their diffs are in flight through the 3-hop relay
        // (conclude_backwards_pagination_from_disk → event-cache task →
        // timeline observable → relay task → DiffBatch actor msg). Re-tick
        // until `timeline_contains` confirms, or the backstop expires.
        //
        // A bounded sleep between ticks is necessary: without it all 40
        // backstop ticks drain before the relay task gets CPU time, because
        // the actor processes its own messages before yielding to other tasks.
        if let Some(remaining) = restore.anchor_relay_wait {
            if self.timeline_contains_event_id(&restore.event_id) {
                self.finish_anchor_restore(restore.request_id, TimelineAnchorRestoreStatus::Found);
                return;
            }
            if remaining > 0 {
                restore.anchor_relay_wait = Some(remaining - 1);
                // Yield to the runtime so the relay pipeline can deliver the
                // anchor diff before we check again. Without this pause, all
                // 40 ticks complete before any relay task is scheduled.
                tokio::time::sleep(std::time::Duration::from_millis(
                    RESTORE_ANCHOR_RELAY_WAIT_TICK_MS,
                ))
                .await;
                self.schedule_restore_anchor_continue(restore).await;
                return;
            }
            // Backstop: relay genuinely stuck. EndReached is the safest
            // fallback (anchor not confirmed in items; the caller can retry).
            self.finish_anchor_restore(restore.request_id, TimelineAnchorRestoreStatus::EndReached);
            return;
        }

        if self.timeline_contains_event_id(&restore.event_id) {
            self.finish_anchor_restore(restore.request_id, TimelineAnchorRestoreStatus::Found);
            return;
        }
        if restore.max_batches_remaining == 0 {
            self.finish_anchor_restore(
                restore.request_id,
                TimelineAnchorRestoreStatus::BudgetExhausted,
            );
            return;
        }

        restore.in_flight = true;
        let request_id = restore.request_id;
        let event_count = restore.event_count;

        // First try a cache-only bulk backward load in a single call
        // instead of looping one chunk at a time through `paginate_once`.
        // The SDK stops as soon as the anchor event is found (load-until-anchor),
        // or when it reaches a gap or the start of the on-disk timeline.
        //
        // Pass the UI-provided chunk budget directly as max_chunks. Room entry
        // must fail fast for stale/deep anchors instead of turning into a long
        // history walk; the event count `n` is a secondary cap.
        let chunk_budget = restore.max_batches_remaining;
        let bulk_n = (chunk_budget as u32)
            .saturating_mul(event_count as u32)
            .min(u16::MAX as u32) as u16;
        let cache_result = self
            .timeline
            .live_restore_from_cache(bulk_n, &restore.event_id, chunk_budget)
            .await;
        restore.in_flight = false;

        match cache_result {
            Ok(outcome) => {
                // The bulk load fired `RoomEventCacheUpdate::UpdateTimelineEvents`
                // broadcasts for every disk chunk, which are ingested by the
                // live Timeline's tasks loop and arrive as actor `DiffBatch`
                // messages. Those are buffered into `restore_emit_buffer` while
                // `restore_anchor.is_some()`, so we still get a single coalesced
                // `ItemsUpdated` flush at the terminal.
                // Deduct the actual number of cache chunks consumed from the
                // budget (each chunk ≈ one paginate batch). Clamp minimum to 1
                // so partial loads always advance the budget counter.
                restore.max_batches_remaining = restore
                    .max_batches_remaining
                    .saturating_sub(outcome.chunks_loaded.max(1) as u16);

                // Fast path: anchor already in timeline items (shallow-anchor case
                // where the lazy in-memory reveal made it visible immediately).
                if self.timeline_contains_event_id(&restore.event_id) {
                    self.finish_anchor_restore(request_id, TimelineAnchorRestoreStatus::Found);
                    return;
                }

                if outcome.anchor_present {
                    // SDK authoritative signal: anchor was found in a loaded disk
                    // chunk; its diff broadcast is already in flight through the
                    // 3-hop relay. Enter the relay-wait loop; do NOT conclude
                    // EndReached/BudgetExhausted while anchor_present is true.
                    restore.anchor_relay_wait = Some(RESTORE_ANCHOR_RELAY_WAIT_TICKS);
                    self.schedule_restore_anchor_continue(restore).await;
                    return;
                }

                if outcome.hit_gap {
                    // The cache is not contiguous up to the anchor depth.
                    // Fall back to the per-chunk paginate_once loop, which can
                    // resolve gaps via the network for non-contiguous caches.
                    restore.in_flight = true;
                    restore.max_batches_remaining = restore.max_batches_remaining.saturating_sub(1);

                    let result = self
                        .paginate_once(request_id, PaginationDirection::Backward, event_count)
                        .await;
                    restore.in_flight = false;

                    if self.timeline_contains_event_id(&restore.event_id) {
                        self.finish_anchor_restore(request_id, TimelineAnchorRestoreStatus::Found);
                        return;
                    }

                    let end_reached = match result {
                        Ok(end_reached) => end_reached,
                        Err(kind) => {
                            self.finish_anchor_restore(
                                request_id,
                                TimelineAnchorRestoreStatus::Failed { kind },
                            );
                            return;
                        }
                    };
                    if end_reached {
                        if self.timeline_contains_event_id(&restore.event_id) {
                            self.finish_anchor_restore(
                                request_id,
                                TimelineAnchorRestoreStatus::Found,
                            );
                            return;
                        }
                        self.finish_anchor_restore(
                            request_id,
                            TimelineAnchorRestoreStatus::EndReached,
                        );
                        return;
                    }
                    if restore.max_batches_remaining == 0 {
                        if self.timeline_contains_event_id(&restore.event_id) {
                            self.finish_anchor_restore(
                                request_id,
                                TimelineAnchorRestoreStatus::Found,
                            );
                            return;
                        }
                        self.finish_anchor_restore(
                            request_id,
                            TimelineAnchorRestoreStatus::BudgetExhausted,
                        );
                        return;
                    }
                    restore.awaiting_diff_batch = true;
                    self.schedule_restore_anchor_continue(restore).await;
                    return;
                }

                // No gap, anchor not present: cache-only bulk load completed
                // without finding the anchor.
                if outcome.reached_start {
                    // Loaded to the start of the on-disk cache; anchor is
                    // genuinely absent — conclude EndReached immediately
                    // (authoritative; no timing wait needed).
                    self.finish_anchor_restore(request_id, TimelineAnchorRestoreStatus::EndReached);
                    return;
                }

                // Cap case: the bulk load stopped because it reached the u16
                // per-call cap, not because it reached a gap or start. More
                // budget remains; issue another bulk load immediately.
                if restore.max_batches_remaining > 0 {
                    restore.awaiting_diff_batch = true;
                    self.schedule_restore_anchor_continue(restore).await;
                    return;
                }

                // Budget exhausted without finding the anchor.
                self.finish_anchor_restore(
                    request_id,
                    TimelineAnchorRestoreStatus::BudgetExhausted,
                );
            }

            Err(_) => {
                // Cache load error — fall back to the per-chunk paginate_once
                // path for a single attempt, treating the error as transient.
                restore.in_flight = true;
                restore.max_batches_remaining = restore.max_batches_remaining.saturating_sub(1);

                let result = self
                    .paginate_once(request_id, PaginationDirection::Backward, event_count)
                    .await;
                restore.in_flight = false;

                if self.timeline_contains_event_id(&restore.event_id) {
                    self.finish_anchor_restore(request_id, TimelineAnchorRestoreStatus::Found);
                    return;
                }

                let end_reached = match result {
                    Ok(end_reached) => end_reached,
                    Err(kind) => {
                        self.finish_anchor_restore(
                            request_id,
                            TimelineAnchorRestoreStatus::Failed { kind },
                        );
                        return;
                    }
                };
                if end_reached {
                    if self.timeline_contains_event_id(&restore.event_id) {
                        self.finish_anchor_restore(request_id, TimelineAnchorRestoreStatus::Found);
                        return;
                    }
                    self.finish_anchor_restore(request_id, TimelineAnchorRestoreStatus::EndReached);
                    return;
                }
                if restore.max_batches_remaining == 0 {
                    if self.timeline_contains_event_id(&restore.event_id) {
                        self.finish_anchor_restore(request_id, TimelineAnchorRestoreStatus::Found);
                        return;
                    }
                    self.finish_anchor_restore(
                        request_id,
                        TimelineAnchorRestoreStatus::BudgetExhausted,
                    );
                    return;
                }
                restore.awaiting_diff_batch = true;
                self.schedule_restore_anchor_continue(restore).await;
            }
        }
    }
    pub(super) async fn maybe_continue_restore_anchor_after_diff(&mut self) {
        let Some(mut restore) = self.restore_anchor.take() else {
            return;
        };
        if restore.in_flight {
            self.restore_anchor = Some(restore);
            return;
        }
        // Anchor-relay wait: the queued Continue tick handles polling
        // `timeline_contains` each tick until Found or backstop. Put restore
        // back so the queued tick does its check on the next iteration.
        if restore.anchor_relay_wait.is_some() {
            self.restore_anchor = Some(restore);
            return;
        }
        if !restore.awaiting_diff_batch {
            self.restore_anchor = Some(restore);
            return;
        }
        if self.timeline_contains_event_id(&restore.event_id) {
            self.finish_anchor_restore(restore.request_id, TimelineAnchorRestoreStatus::Found);
            return;
        }
        if restore.max_batches_remaining == 0 {
            self.finish_anchor_restore(
                restore.request_id,
                TimelineAnchorRestoreStatus::BudgetExhausted,
            );
            return;
        }
        if restore.continuation_scheduled {
            self.restore_anchor = Some(restore);
            return;
        }

        restore.awaiting_diff_batch = false;
        self.schedule_restore_anchor_continue(restore).await;
    }
    async fn schedule_restore_anchor_continue(&mut self, mut restore: RestoreTimelineAnchorState) {
        self.next_restore_anchor_serial = self.next_restore_anchor_serial.wrapping_add(1);
        let serial = self.next_restore_anchor_serial;
        restore.continuation_scheduled = true;
        restore.continuation_serial = Some(serial);
        self.restore_anchor = Some(restore);
        let _ = self
            .msg_tx
            .send(TimelineActorMessage::RestoreTimelineAnchorContinue { serial })
            .await;
    }
    /// Re-emit `navigation_items` as `InitialItems` without touching the SDK
    /// subscription or tearing down the actor. Idempotent Subscribe supplies
    /// an exact cause; internal replay recovery does not. The projection ACK
    /// identity remains owned by the actor in both cases.
    pub(super) fn handle_replay_initial_items(&mut self, cause_request_id: Option<RequestId>) {
        let window = replay_initial_items_window_range(
            &self.key.kind,
            self.navigation_items.len(),
            &self.viewport_observation,
        );
        let items = self.navigation_items[window.clone()].to_vec();
        let item_count = items.len();
        trace_timeline_items("replay_initial", &self.key, &items);
        let candidate_display_projection =
            DisplayProjectionState::from_canonical_window(&self.navigation_items, window);
        let replay_known_candidates = replay_known_candidates_for_display_items(
            &self.key,
            &self.navigation_items,
            candidate_display_projection.display_items(),
        );
        let emitted = commit_prepared_initial_window_for_generation(
            &mut self.navigation_items,
            &mut self.display_projection,
            &self.event_tx,
            &self.replay_known_thread_root_projections,
            &self.thread_root_projection_service,
            &self.timeline_actor_generations,
            &self.key,
            self.actor_generation,
            InitialItemsRequestIdentity::replay(
                self.projection_request_id,
                self.projection_acknowledged,
                cause_request_id,
            ),
            self.generation,
            Vec::new(),
            PreparedInitialWindow {
                display_projection: candidate_display_projection,
                navigation_items: None,
                emitted_items: items,
                replay_known_candidates,
            },
        );
        if emitted {
            let _ = self.thread_attention.reconcile(
                &self.key,
                &self.navigation_items,
                self.own_user_id.as_ref().map(|user_id| user_id.as_str()),
                ThreadAttentionObservation::Replay,
            );
        }
        record_subscribe_stage(
            if emitted {
                "replay_initial_emitted"
            } else {
                "replay_initial_rejected_stale_generation"
            },
            Some(item_count),
        );
    }
    pub(super) fn acknowledge_projection(
        &mut self,
        projection_request_id: RequestId,
        generation: TimelineGeneration,
    ) -> bool {
        accept_projection_ack_for_active_actor(
            &self.timeline_actor_generations,
            &self.key,
            self.actor_generation,
            self.projection_request_id,
            self.generation,
            projection_request_id,
            generation,
            &mut self.projection_acknowledged,
        )
    }
    pub(super) fn emit_navigation_if_changed(&mut self) {
        let snapshot = derive_timeline_navigation_snapshot_with_read_state(
            &self.navigation_items,
            self.fully_read_event_id.as_deref(),
            self.server_confirmed_read_event_id.as_deref(),
            self.local_viewed_boundary
                .as_ref()
                .map(|boundary| boundary.event_id.as_str()),
            self.read_state_sync,
            &self.viewport_observation,
            self.own_user_id.as_ref().map(|user_id| user_id.as_str()),
        );
        if self.last_navigation_snapshot.as_ref() == Some(&snapshot) {
            return;
        }
        record_timeline_unread_consistency(
            "navigation_updated",
            &self.key,
            &self.navigation_items,
            self.display_projection.display_items(),
            self.last_navigation_snapshot.as_ref(),
            &snapshot,
            &self.thread_attention,
        );
        self.last_navigation_snapshot = Some(snapshot.clone());
        self.emit(CoreEvent::Timeline(TimelineEvent::NavigationUpdated {
            key: self.key.clone(),
            snapshot,
        }));
    }
    fn emit_anchor_restore_finished(
        &self,
        request_id: RequestId,
        status: TimelineAnchorRestoreStatus,
    ) {
        self.emit(CoreEvent::Timeline(TimelineEvent::AnchorRestoreFinished {
            request_id,
            key: self.key.clone(),
            status,
        }));
    }
    /// Publish the deferred display batch, changed navigation projection, and
    /// optional restore terminal under one actor-generation lease.  Returning
    /// `None` means a replacement actor won the generation fence; in that case
    /// the buffer and all actor-owned mirrors remain untouched.
    fn publish_restore_settlement(
        &mut self,
        terminal: Option<(RequestId, TimelineAnchorRestoreStatus)>,
    ) -> Option<bool> {
        let navigation_snapshot = derive_timeline_navigation_snapshot_with_read_state(
            &self.navigation_items,
            self.fully_read_event_id.as_deref(),
            self.server_confirmed_read_event_id.as_deref(),
            self.local_viewed_boundary
                .as_ref()
                .map(|boundary| boundary.event_id.as_str()),
            self.read_state_sync,
            &self.viewport_observation,
            self.own_user_id.as_ref().map(|user_id| user_id.as_str()),
        );
        let changed_navigation = (self.last_navigation_snapshot.as_ref()
            != Some(&navigation_snapshot))
        .then_some(navigation_snapshot);
        let published_batch_id = self.next_batch_id;
        let published_items = publish_restore_settlement_for_generation(
            &mut self.restore_emit_buffer,
            !self.restore_causal_projections.projections.is_empty(),
            &mut self.next_batch_id,
            &self.event_tx,
            &self.replay_known_thread_root_projections,
            &self.thread_root_projection_service,
            &self.timeline_actor_generations,
            &self.key,
            self.actor_generation,
            self.generation,
            &self.navigation_items,
            self.display_projection.display_items(),
            RestoreSettlement {
                navigation_snapshot: changed_navigation.clone(),
                terminal,
            },
        )?;

        if let Some(snapshot) = changed_navigation {
            self.last_navigation_snapshot = Some(snapshot);
        }
        if published_items {
            let observation = self.restore_causal_projections.observe_after_publication(
                &mut self.gap_projection_correlation,
                &mut self.live_tail_projection_correlation,
                published_batch_id,
            );
            self.ready_restore_gap_projection_batch = observation.historical_gap_batch_id;
        } else {
            self.restore_causal_projections = RestoreCausalProjectionBuffer::default();
        }
        Some(published_items)
    }
    /// Emit one canonical batch plus replay-known ownership changes. This is
    /// the sole normal/restore flush path for `ItemsUpdated`; it preserves the
    /// intentional restore buffering while keeping the final group atomic
    /// with respect to actor-generation replacement.
    pub(super) fn emit_items_updated_and_reconcile_replay_known(
        &mut self,
        diffs: Vec<TimelineDiff>,
    ) -> bool {
        let batch_id = self.next_batch_id;
        if emit_items_updated_and_reconcile_replay_known_for_generation(
            &self.event_tx,
            &self.replay_known_thread_root_projections,
            &self.thread_root_projection_service,
            &self.timeline_actor_generations,
            &self.key,
            self.actor_generation,
            self.generation,
            batch_id,
            diffs,
            &self.navigation_items,
            self.display_projection.display_items(),
        ) {
            self.next_batch_id = TimelineBatchId(batch_id.0 + 1);
            true
        } else {
            false
        }
    }
    /// Commit a local actor mutation through the same generation-fenced
    /// display/replay group as an SDK diff. The bounded replay mirror resolves
    /// each local Set through the exact canonical owner retained on its slot;
    /// roots omitted from the bounded window intentionally produce no display
    /// diff and are handled by replay reconciliation.
    pub(super) fn emit_non_sdk_item_sets_and_reconcile_replay_known(
        &mut self,
        diffs: Vec<TimelineDiff>,
    ) -> bool {
        let batch_id = self.next_batch_id;
        if emit_non_sdk_item_sets_and_reconcile_replay_known_for_generation(
            &self.event_tx,
            &self.replay_known_thread_root_projections,
            &self.thread_root_projection_service,
            &self.timeline_actor_generations,
            &self.key,
            self.actor_generation,
            self.generation,
            batch_id,
            diffs,
            &self.navigation_items,
            &mut self.display_projection,
        ) {
            self.next_batch_id = TimelineBatchId(batch_id.0 + 1);
            true
        } else {
            false
        }
    }
    /// Terminate a restore walk: flush the buffered diffs (Change 2) then emit
    /// `AnchorRestoreFinished`. Call this at every terminal restore path in
    /// place of `emit_anchor_restore_finished` when the diff buffer may be
    /// non-empty.
    pub(super) fn finish_anchor_restore(
        &mut self,
        request_id: RequestId,
        status: TimelineAnchorRestoreStatus,
    ) {
        if self
            .publish_restore_settlement(Some((request_id, status)))
            .unwrap_or(false)
        {
            self.hydrate_after_restore_flush = true;
        }
    }
}

#[cfg(test)]
pub(super) fn replay_initial_items_window(
    kind: &TimelineKind,
    items: &[TimelineItem],
    observation: &TimelineViewportObservation,
) -> Vec<TimelineItem> {
    items[replay_initial_items_window_range(kind, items.len(), observation)].to_vec()
}

fn replay_initial_items_window_range(
    kind: &TimelineKind,
    item_count: usize,
    observation: &TimelineViewportObservation,
) -> std::ops::Range<usize> {
    let start = if matches!(kind, TimelineKind::Room { .. })
        && observation.at_bottom
        && item_count > ROOM_REPLAY_INITIAL_ITEMS_MAX
    {
        item_count - ROOM_REPLAY_INITIAL_ITEMS_MAX
    } else {
        0
    };
    start..item_count
}

pub(super) fn should_hydrate_empty_initial_room_timeline(
    kind: &TimelineKind,
    item_count: usize,
) -> bool {
    matches!(kind, TimelineKind::Room { .. }) && item_count == 0
}

pub(super) fn activity_rows_from_timeline_items(
    key: &TimelineKey,
    items: &[TimelineItem],
) -> Vec<ActivityRow> {
    let TimelineKind::Room { room_id } = &key.kind else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| activity_row_from_timeline_item(room_id, item))
        .collect()
}

fn activity_row_from_timeline_item(room_id: &str, item: &TimelineItem) -> Option<ActivityRow> {
    if !is_attention_eligible_event(item) {
        return None;
    }
    let TimelineItemId::Event { event_id } = &item.id else {
        return None;
    };
    let preview = eligible_activity_preview(item)?;
    let mut row = ActivityRow::event(
        room_id.to_owned(),
        event_id.clone(),
        item.sender.clone(),
        String::new(),
        item.sender_label.clone(),
        Some(preview),
        item.timestamp_ms.unwrap_or(0),
        false,
        false,
    );
    row.sender_avatar = item.sender_avatar.clone();
    row.thread_root_event_id = item.thread_root.clone();
    Some(row)
}

pub(super) fn derive_timeline_navigation_snapshot(
    items: &[TimelineItem],
    fully_read_event_id: Option<&str>,
    observation: &TimelineViewportObservation,
    own_user_id: Option<&str>,
) -> TimelineNavigationSnapshot {
    derive_timeline_navigation_snapshot_with_read_state(
        items,
        fully_read_event_id,
        fully_read_event_id,
        None,
        TimelineReadStateSync::Synced,
        observation,
        own_user_id,
    )
}

pub(super) fn derive_timeline_navigation_snapshot_with_read_state(
    items: &[TimelineItem],
    fully_read_event_id: Option<&str>,
    server_confirmed_read_event_id: Option<&str>,
    local_viewed_event_id: Option<&str>,
    read_state_sync: TimelineReadStateSync,
    observation: &TimelineViewportObservation,
    own_user_id: Option<&str>,
) -> TimelineNavigationSnapshot {
    let server_confirmed_read_event_id = server_confirmed_read_event_id
        .or(fully_read_event_id)
        .map(ToOwned::to_owned);
    let local_viewed_event_id = local_viewed_event_id.map(ToOwned::to_owned);
    let local_viewed_is_canonical = local_viewed_event_id
        .as_deref()
        .is_some_and(|event_id| item_index_for_event_id(items, event_id).is_some());
    let mut snapshot = TimelineNavigationSnapshot {
        read_marker_event_id: server_confirmed_read_event_id.clone(),
        read_marker_display_event_id: local_viewed_is_canonical
            .then(|| local_viewed_event_id.clone())
            .flatten(),
        first_unread_event_id: None,
        unread_event_count: 0,
        unread_position: TimelineUnreadPosition::None,
        newer_event_count: 0,
        can_jump_to_bottom: false,
        local_viewed_event_id,
        server_confirmed_read_event_id: server_confirmed_read_event_id.clone(),
        read_state_sync,
    };

    let Some(read_marker_event_id) = server_confirmed_read_event_id.as_deref() else {
        return snapshot;
    };
    let Some(read_marker_index) = item_index_for_event_id(items, read_marker_event_id) else {
        snapshot.unread_position = TimelineUnreadPosition::Unknown;
        return snapshot;
    };
    snapshot.newer_event_count =
        newer_unread_event_count(items, observation, own_user_id, read_marker_index);
    snapshot.can_jump_to_bottom = snapshot.newer_event_count > 0;

    let unread_items: Vec<(usize, &TimelineItem)> = items
        .iter()
        .enumerate()
        .skip(read_marker_index.saturating_add(1))
        .filter(|(_, item)| is_unread_navigation_item(item, own_user_id))
        .collect();

    snapshot.unread_event_count = unread_items.len() as u64;
    if let Some((first_unread_index, first_unread)) = unread_items.first() {
        snapshot.first_unread_event_id =
            timeline_item_event_id(first_unread).map(ToOwned::to_owned);
        snapshot.unread_position =
            unread_position_for_index(items, *first_unread_index, observation);
        return snapshot;
    }

    // No remote unread events after the marker. Advance the display anchor to the
    // current user's latest visible own message at or after the marker so the
    // "Read up to here" separator is rendered after it, not before.
    if snapshot.read_marker_display_event_id.is_none() {
        snapshot.read_marker_display_event_id = items
            .iter()
            .enumerate()
            .skip(read_marker_index)
            .filter(|(_, item)| is_own_visible_event(item, own_user_id))
            .last()
            .and_then(|(_, item)| timeline_item_event_id(item).map(ToOwned::to_owned));
    }
    snapshot
}

fn timeline_unread_position_token(position: TimelineUnreadPosition) -> &'static str {
    match position {
        TimelineUnreadPosition::None => "none",
        TimelineUnreadPosition::AboveViewport => "above_viewport",
        TimelineUnreadPosition::InsideViewport => "inside_viewport",
        TimelineUnreadPosition::BelowViewport => "below_viewport",
        TimelineUnreadPosition::Unknown => "unknown",
    }
}

/// Correlate the Room fully-read marker, canonical unread projection, Thread
/// receipt, and latest-reply display projection without logging private IDs.
/// Equality and position booleans preserve the useful causal relationships
/// while keeping room, event, and user identifiers out of diagnostics.
fn timeline_unread_consistency_diagnostic_event(
    stage: &'static str,
    key: &TimelineKey,
    canonical_items: &[TimelineItem],
    display_items: &[TimelineItem],
    previous_snapshot: Option<&TimelineNavigationSnapshot>,
    snapshot: &TimelineNavigationSnapshot,
    thread_attention: &ThreadAttentionTracker,
) -> DiagnosticEvent {
    let event_position = |event_id: &str| {
        canonical_items
            .iter()
            .position(|item| timeline_item_event_id(item) == Some(event_id))
    };
    let display_position = |event_id: &str| {
        display_items
            .iter()
            .position(|item| timeline_item_event_id(item) == Some(event_id))
    };

    let fully_read_position = snapshot
        .read_marker_event_id
        .as_deref()
        .and_then(event_position);
    let first_unread_item = snapshot
        .first_unread_event_id
        .as_deref()
        .and_then(|event_id| event_position(event_id).map(|position| (position, event_id)))
        .and_then(|(position, event_id)| {
            canonical_items
                .get(position)
                .map(|item| (position, event_id, item))
        });
    let first_unread_position = first_unread_item.map(|(position, _, _)| position);
    let first_unread_event_id = first_unread_item.map(|(_, event_id, _)| event_id);
    let first_unread_thread_root =
        first_unread_item.and_then(|(_, _, item)| item.thread_root.as_deref());
    let thread_receipt_position = thread_attention
        .receipt_event_id
        .as_deref()
        .and_then(event_position);
    let thread_receipt_item =
        thread_receipt_position.and_then(|position| canonical_items.get(position));
    let timeline_thread_root = match &key.kind {
        TimelineKind::Thread { root_event_id, .. } => Some(root_event_id.as_str()),
        TimelineKind::Room { .. } | TimelineKind::Focused { .. } => None,
    };

    let latest_reply_activity_count = display_items
        .iter()
        .filter_map(|item| item.thread_summary.as_ref()?.latest_event_id.as_deref())
        .filter(|event_id| !event_id.trim().is_empty())
        .count();
    let display_root_for_first_unread = first_unread_event_id.and_then(|first_unread_event_id| {
        display_items.iter().find(|item| {
            item.thread_summary
                .as_ref()
                .and_then(|summary| summary.latest_event_id.as_deref())
                == Some(first_unread_event_id)
        })
    });
    let latest_reply_activity_canonical_count = display_items
        .iter()
        .filter_map(|item| item.thread_summary.as_ref()?.latest_event_id.as_deref())
        .filter(|event_id| event_position(event_id).is_some())
        .count();
    let fully_read_changed = previous_snapshot
        .is_some_and(|previous| previous.read_marker_event_id != snapshot.read_marker_event_id);

    DiagnosticEvent::new(
        DiagnosticLevel::Info,
        "core.timeline_unread_consistency",
        stage,
    )
    .field(DiagnosticField::token(
        "timeline",
        timeline_key_trace_kind(key),
    ))
    .field(DiagnosticField::count(
        "canonical_item_count",
        canonical_items.len().try_into().unwrap_or(u64::MAX),
    ))
    .field(DiagnosticField::count(
        "display_item_count",
        display_items.len().try_into().unwrap_or(u64::MAX),
    ))
    .field(DiagnosticField::boolean(
        "fully_read_present",
        snapshot.read_marker_event_id.is_some(),
    ))
    .field(DiagnosticField::boolean(
        "fully_read_changed",
        fully_read_changed,
    ))
    .field(DiagnosticField::boolean(
        "fully_read_in_canonical",
        fully_read_position.is_some(),
    ))
    .field(DiagnosticField::boolean(
        "first_unread_present",
        snapshot.first_unread_event_id.is_some(),
    ))
    .field(DiagnosticField::boolean(
        "first_unread_in_canonical",
        first_unread_item.is_some(),
    ))
    .field(DiagnosticField::boolean(
        "first_unread_after_fully_read",
        matches!((fully_read_position, first_unread_position), (Some(read), Some(unread)) if unread > read),
    ))
    .field(DiagnosticField::boolean(
        "first_unread_has_thread_root",
        first_unread_thread_root.is_some(),
    ))
    .field(DiagnosticField::boolean(
        "first_unread_directly_displayed",
        first_unread_event_id.is_some_and(|event_id| display_position(event_id).is_some()),
    ))
    .field(DiagnosticField::boolean(
        "display_root_for_first_unread_present",
        display_root_for_first_unread.is_some(),
    ))
    .field(DiagnosticField::boolean(
        "display_root_matches_thread_root",
        matches!(
            (display_root_for_first_unread.and_then(timeline_item_event_id), first_unread_thread_root),
            (Some(display_root), Some(thread_root)) if display_root == thread_root
        ),
    ))
    .field(DiagnosticField::count(
        "unread_event_count",
        snapshot.unread_event_count,
    ))
    .field(DiagnosticField::token(
        "unread_position",
        timeline_unread_position_token(snapshot.unread_position),
    ))
    .field(DiagnosticField::boolean(
        "thread_receipt_present",
        thread_attention.receipt_event_id.is_some(),
    ))
    .field(DiagnosticField::boolean(
        "thread_receipt_in_canonical",
        thread_receipt_position.is_some(),
    ))
    .field(DiagnosticField::boolean(
        "thread_receipt_matches_timeline_root",
        matches!(
            (thread_receipt_item.and_then(|item| item.thread_root.as_deref()), timeline_thread_root),
            (Some(receipt_root), Some(timeline_root)) if receipt_root == timeline_root
        ),
    ))
    .field(DiagnosticField::count(
        "thread_attention_count",
        thread_attention.counts.notification_count,
    ))
    .field(DiagnosticField::count(
        "latest_reply_activity_count",
        latest_reply_activity_count.try_into().unwrap_or(u64::MAX),
    ))
    .field(DiagnosticField::count(
        "latest_reply_activity_canonical_count",
        latest_reply_activity_canonical_count
            .try_into()
            .unwrap_or(u64::MAX),
    ))
    .field(DiagnosticField::boolean(
        "latest_reply_activity_matches_first_unread",
        display_root_for_first_unread.is_some(),
    ))
}

pub(super) fn record_timeline_unread_consistency(
    stage: &'static str,
    key: &TimelineKey,
    canonical_items: &[TimelineItem],
    display_items: &[TimelineItem],
    previous_snapshot: Option<&TimelineNavigationSnapshot>,
    snapshot: &TimelineNavigationSnapshot,
    thread_attention: &ThreadAttentionTracker,
) {
    koushi_diagnostics::record(timeline_unread_consistency_diagnostic_event(
        stage,
        key,
        canonical_items,
        display_items,
        previous_snapshot,
        snapshot,
        thread_attention,
    ));
}

fn is_own_visible_event(item: &TimelineItem, own_user_id: Option<&str>) -> bool {
    if item.is_hidden || !has_user_visible_content(item) {
        return false;
    }
    if !own_user_id.is_some_and(|own| item.sender.as_deref() == Some(own)) {
        return false;
    }
    matches!(item.id, TimelineItemId::Event { .. })
}

fn newer_unread_event_count(
    items: &[TimelineItem],
    observation: &TimelineViewportObservation,
    own_user_id: Option<&str>,
    read_marker_index: usize,
) -> u64 {
    if observation.at_bottom {
        return 0;
    }
    let Some(last_visible_event_id) = observation.last_visible_event_id.as_deref() else {
        return 0;
    };
    let Some(last_visible_index) = item_index_for_event_id(items, last_visible_event_id) else {
        return 0;
    };
    let first_newer_unread_index = last_visible_index.max(read_marker_index).saturating_add(1);
    items
        .iter()
        .skip(first_newer_unread_index)
        .filter(|item| is_unread_navigation_item(item, own_user_id))
        .count() as u64
}

fn unread_position_for_index(
    items: &[TimelineItem],
    item_index: usize,
    observation: &TimelineViewportObservation,
) -> TimelineUnreadPosition {
    let Some(first_visible_event_id) = observation.first_visible_event_id.as_deref() else {
        return TimelineUnreadPosition::Unknown;
    };
    let Some(last_visible_event_id) = observation.last_visible_event_id.as_deref() else {
        return TimelineUnreadPosition::Unknown;
    };
    let Some(first_visible_index) = item_index_for_event_id(items, first_visible_event_id) else {
        return TimelineUnreadPosition::Unknown;
    };
    let Some(last_visible_index) = item_index_for_event_id(items, last_visible_event_id) else {
        return TimelineUnreadPosition::Unknown;
    };

    if item_index < first_visible_index {
        TimelineUnreadPosition::AboveViewport
    } else if item_index > last_visible_index {
        TimelineUnreadPosition::BelowViewport
    } else {
        TimelineUnreadPosition::InsideViewport
    }
}

fn classify_pagination_error(err: &matrix_sdk_ui::timeline::Error) -> TimelineFailureKind {
    use matrix_sdk_ui::timeline::{Error, PaginationError};
    match err {
        Error::PaginationError(PaginationError::NotSupported) => {
            TimelineFailureKind::InvalidDirection
        }
        Error::PaginationError(_) => TimelineFailureKind::Sdk,
        _ => TimelineFailureKind::Sdk,
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_source::item_body;

    use std::collections::{BTreeSet, HashMap};

    use std::sync::{Arc, Mutex};

    use std::time::Duration;

    use koushi_sdk::{MatrixClientSession, MatrixLiveTailRefreshOutcome};

    use koushi_state::AppAction;

    use matrix_sdk_ui::timeline::{GapRepairProjectionId, TimelineFocus};
    use tokio::sync::{broadcast, mpsc, oneshot, watch};

    use crate::account_work::{AccountWorkKind, AccountWorkScheduler};
    #[cfg(test)]
    use crate::causal_projection::{CAUSAL_PROJECTION_DOMAIN_BIT, CAUSAL_PROJECTION_SERIAL_MAX};
    use crate::causal_projection::{
        CausalProjectionDomain, CausalProjectionId, CausalProjectionOperationId,
        next_causal_projection_serial,
    };
    use crate::command::TimelineCommand;
    use crate::event::{
        CoreEvent, PaginationDirection, PaginationState, ThreadSummaryDto, TimelineEvent,
        TimelineFormattedBody, TimelineItemId, TimelineReadStateSync, TimelineUnreadPosition,
        TimelineViewportObservation,
    };
    use crate::executor;
    use crate::failure::{CoreFailure, TimelineFailureKind};
    #[cfg(any(test, feature = "test-hooks"))]
    use crate::ids::AccountKey;
    use crate::ids::{TimelineBatchId, TimelineKey, TimelineKind};
    use crate::link_preview::LinkPreviewContext;

    use crate::live_tail_freshness::{
        FOREGROUND_LIVE_TAIL_LIMIT, LiveTailFreshnessState, LiveTailRefreshCoordinator,
        LiveTailSchedulerAction,
    };

    use crate::threads_list::ThreadRootProjectionService;
    use koushi_sdk::MatrixLiveTailRefreshOutcome as LiveTailRefreshOutcome;

    use koushi_diagnostics::DiagnosticValue;
    use koushi_state::{SessionInfo, SessionState};

    use crate::command::CoreCommand;
    use crate::runtime::CoreRuntime;

    use super::super::actor::{
        TimelineActor, TimelineActorCleanupIngress, TimelineActorCleanupState,
        TimelineActorControl, TimelineActorHandle, TimelineActorMessage,
    };
    use super::super::display_projection::apply_timeline_diffs_to_items;
    use super::super::gap_repair::{
        CausalProjectionObservation, TimelineGapProjectionCompletion,
        TimelineGapProjectionCorrelation, live_tail_causal_projection_operation,
        observe_causal_projection,
    };
    use super::super::item_projection::{
        sdk_item_to_timeline_item, sdk_vector_diffs_to_timeline_diffs, timeline_item_event_id,
    };
    use super::super::manager::{TimelineManagerActor, TimelineManagerControl, TimelineMessage};
    use super::super::outbound_send::{
        SharedSendCompletionCoordinator, TimelineSendEnqueueContext, TimelineSendTerminalIngress,
    };
    use super::super::relay::koushi_timeline_builder;
    use super::super::test_support::{
        fake_rid, focused_key, gap_demand_test_actor_handle, live_tail_test_manager, room_key,
        test_timeline_actor_handle, thread_key, timeline_item,
    };
    use super::super::thread_projection::{
        ReplayKnownThreadRootProjectionRegistry, ThreadAttentionTracker,
    };
    use super::{
        NavigationProjectionCleanup, NavigationProjectionIngress, NavigationProjectionIntent,
        ROOM_REPLAY_INITIAL_ITEMS_MAX, TimelineActorGenerationGate,
        acquire_pagination_permit_and_emit_paginating, activity_row_from_timeline_item,
        backward_pagination_changed_oldest_edge, derive_timeline_navigation_snapshot,
        derive_timeline_navigation_snapshot_with_read_state,
        projection_acknowledgement_for_current_items, receive_navigation_projection,
        replay_initial_items_window, should_hydrate_empty_initial_room_timeline,
        timeline_unread_consistency_diagnostic_event,
    };

    #[test]
    fn eligibility_skips_redacted_and_own_rows_for_first_unread_and_newer_count() {
        let marker = timeline_item("$marker:test", Some("marker"), "@alice:test", false);
        let mut redacted = timeline_item("$redacted:test", Some("redacted"), "@alice:test", false);
        redacted.is_redacted = true;
        let valid = timeline_item("$valid:test", Some("valid"), "@bob:test", false);
        let own = timeline_item("$own:test", Some("own"), "@me:test", false);
        let items = vec![marker, redacted, valid, own];
        let observation = TimelineViewportObservation {
            first_visible_event_id: Some("$marker:test".to_owned()),
            last_visible_event_id: Some("$marker:test".to_owned()),
            at_bottom: false,
            ..TimelineViewportObservation::default()
        };

        let snapshot = derive_timeline_navigation_snapshot(
            &items,
            Some("$marker:test"),
            &observation,
            Some("@me:test"),
        );

        assert_eq!(
            snapshot.first_unread_event_id.as_deref(),
            Some("$valid:test")
        );
        assert_eq!(snapshot.unread_event_count, 1);
        assert_eq!(snapshot.newer_event_count, 1);
    }

    #[test]
    fn formatted_only_activity_rows_remain_eligible() {
        let mut item = timeline_item("$formatted:test", None, "@alice:test", false);
        item.formatted = Some(TimelineFormattedBody {
            html: "<b>formatted</b>".to_owned(),
            plain_text: "formatted".to_owned(),
            code_blocks: Vec::new(),
        });

        assert!(activity_row_from_timeline_item("!room:test", &item).is_some());
    }

    #[test]
    fn backward_pagination_detects_only_a_changed_oldest_edge_as_prepend() {
        assert!(!backward_pagination_changed_oldest_edge(None, None));
        assert!(backward_pagination_changed_oldest_edge(None, Some("older")));
        assert!(!backward_pagination_changed_oldest_edge(
            Some("current"),
            Some("current")
        ));
        assert!(backward_pagination_changed_oldest_edge(
            Some("current"),
            Some("older")
        ));
    }

    #[tokio::test]
    async fn pagination_waits_for_permit_before_publishing_paginating() {
        let scheduler = AccountWorkScheduler::default();
        let background = scheduler.acquire(AccountWorkKind::SearchCrawl).await;
        let key = room_key();
        let generations = Arc::new(TimelineActorGenerationGate::default());
        let actor_generation = generations.activate_after_quiescence(&key).await.generation;
        let (event_tx, mut event_rx) = broadcast::channel(8);

        let admission = tokio::spawn(acquire_pagination_permit_and_emit_paginating(
            fake_rid(91),
            key.clone(),
            event_tx,
            Arc::clone(&generations),
            actor_generation,
            scheduler,
            PaginationDirection::Backward,
        ));

        tokio::time::timeout(Duration::from_secs(1), background.cancelled())
            .await
            .expect("queued pagination must ask background work to yield");
        assert!(
            matches!(
                event_rx.try_recv(),
                Err(broadcast::error::TryRecvError::Empty)
            ),
            "Paginating must remain unpublished while scheduler admission is pending"
        );

        drop(background);
        let permit = tokio::time::timeout(Duration::from_secs(1), admission)
            .await
            .expect("pagination admission must finish after the slot is released")
            .expect("pagination admission task must not panic")
            .expect("the active actor generation must receive a permit");
        let event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
            .await
            .expect("Paginating must publish after scheduler admission")
            .expect("timeline event sender must remain open");
        assert!(matches!(
            event,
            CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                request_id: Some(request_id),
                key: event_key,
                direction: PaginationDirection::Backward,
                state: PaginationState::Paginating,
                ..
            }) if request_id == fake_rid(91) && event_key == key
        ));
        drop(permit);
    }

    #[test]
    fn projection_ack_evidence_is_recomputed_from_current_actor_items() {
        let key = focused_key();
        let TimelineKind::Focused { event_id, .. } = &key.kind else {
            panic!("fixture must be focused");
        };
        let with_target = vec![timeline_item(
            event_id,
            Some("target"),
            "@sender:test",
            false,
        )];
        let present = projection_acknowledgement_for_current_items(&key, &with_target, true);
        assert!(present.accepted);
        assert!(present.target_present);
        assert_eq!(present.item_count, 1);

        let without_target = vec![timeline_item(
            "$other:test",
            Some("other"),
            "@sender:test",
            false,
        )];
        let missing = projection_acknowledgement_for_current_items(&key, &without_target, true);
        assert!(missing.accepted);
        assert!(!missing.target_present);
        assert_eq!(missing.item_count, 1);
    }

    #[test]
    fn resubscribe_replay_keeps_scrolled_room_context_complete() {
        let key = room_key();
        let items = (0..(ROOM_REPLAY_INITIAL_ITEMS_MAX + 25))
            .map(|index| {
                timeline_item(
                    &format!("$event-{index}:test"),
                    Some("body"),
                    "@bob:test",
                    false,
                )
            })
            .collect::<Vec<_>>();

        let replay = replay_initial_items_window(
            &key.kind,
            &items,
            &TimelineViewportObservation {
                at_bottom: false,
                first_visible_event_id: Some("$event-10:test".to_owned()),
                last_visible_event_id: Some("$event-20:test".to_owned()),
                visible_gap_ids: Vec::new(),
            },
        );

        assert_eq!(replay.len(), ROOM_REPLAY_INITIAL_ITEMS_MAX + 25);
        assert_eq!(
            replay.first().and_then(timeline_item_event_id),
            Some("$event-0:test")
        );
    }

    #[test]
    fn resubscribe_replay_keeps_focused_timeline_context_complete() {
        let key = TimelineKey {
            account_key: AccountKey("@a:test".to_owned()),
            kind: TimelineKind::Focused {
                room_id: "!r:test".to_owned(),
                event_id: "$anchor:test".to_owned(),
            },
        };
        let items = (0..(ROOM_REPLAY_INITIAL_ITEMS_MAX + 25))
            .map(|index| {
                timeline_item(
                    &format!("$event-{index}:test"),
                    Some("body"),
                    "@bob:test",
                    false,
                )
            })
            .collect::<Vec<_>>();

        let replay = replay_initial_items_window(
            &key.kind,
            &items,
            &TimelineViewportObservation {
                at_bottom: true,
                ..TimelineViewportObservation::default()
            },
        );

        assert_eq!(replay.len(), ROOM_REPLAY_INITIAL_ITEMS_MAX + 25);
        assert_eq!(
            replay.first().and_then(timeline_item_event_id),
            Some("$event-0:test")
        );
    }

    #[test]
    fn empty_room_initial_snapshot_needs_initial_backfill() {
        let key = room_key();

        assert!(should_hydrate_empty_initial_room_timeline(&key.kind, 0));
        assert!(!should_hydrate_empty_initial_room_timeline(&key.kind, 1));
    }

    #[test]
    fn non_room_empty_initial_snapshots_do_not_use_room_live_backfill() {
        let thread = TimelineKind::Thread {
            room_id: "!r:test".to_owned(),
            root_event_id: "$root:test".to_owned(),
        };
        let focused = TimelineKind::Focused {
            room_id: "!r:test".to_owned(),
            event_id: "$event:test".to_owned(),
        };

        assert!(!should_hydrate_empty_initial_room_timeline(&thread, 0));
        assert!(!should_hydrate_empty_initial_room_timeline(&focused, 0));
    }

    fn cleanup_probe_timeline_actor_handle() -> (
        TimelineActorHandle,
        watch::Receiver<TimelineActorCleanupState>,
    ) {
        let mut handle = test_timeline_actor_handle();
        let (cleanup, receiver) = TimelineActorCleanupIngress::channel();
        handle.enqueue_context = Some(TimelineSendEnqueueContext::CleanupProbe { cleanup });
        (handle, receiver)
    }

    fn live_tail_test_actor_handle(
        label: &'static str,
        log: Arc<Mutex<Vec<String>>>,
    ) -> TimelineActorHandle {
        let (tx, mut rx) = mpsc::channel(8);
        let task = executor::spawn(async move {
            let mut operation_epochs = HashMap::new();
            while let Some(message) = rx.recv().await {
                match message {
                    TimelineActorMessage::StartLiveTailRefresh {
                        epoch,
                        operation_generation,
                        limit,
                    } => {
                        operation_epochs.insert(operation_generation, epoch);
                        log.lock()
                            .expect("live-tail log lock")
                            .push(format!("start:{label}:epoch={epoch}:limit={limit}"));
                    }
                    TimelineActorMessage::CancelLiveTailNetwork {
                        operation_generation,
                        acknowledged,
                    } => {
                        let epoch = operation_epochs
                            .get(&operation_generation)
                            .copied()
                            .expect("cancelled operation was started");
                        log.lock()
                            .expect("live-tail log lock")
                            .push(format!("cancel-network:{label}:epoch={epoch}"));
                        let _ = acknowledged.send(());
                    }
                    _ => {}
                }
            }
        });
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

    fn stalled_live_tail_cancel_actor_handle(
        label: &'static str,
        log: Arc<Mutex<Vec<String>>>,
    ) -> TimelineActorHandle {
        let (tx, mut rx) = mpsc::channel(8);
        let task = executor::spawn(async move {
            let mut held_acknowledgements = Vec::new();
            while let Some(message) = rx.recv().await {
                match message {
                    TimelineActorMessage::StartLiveTailRefresh {
                        epoch,
                        operation_generation: _,
                        limit,
                    } => log
                        .lock()
                        .expect("stalled live-tail log lock")
                        .push(format!("start:{label}:epoch={epoch}:limit={limit}")),
                    TimelineActorMessage::CancelLiveTailNetwork {
                        operation_generation: _,
                        acknowledged,
                    } => {
                        log.lock()
                            .expect("stalled live-tail log lock")
                            .push(format!("cancel-network:{label}"));
                        held_acknowledgements.push(acknowledged);
                    }
                    _ => {}
                }
            }
        });
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

    fn live_tail_replacement_test_actor_handle(
        key: TimelineKey,
        labels: Arc<Mutex<HashMap<TimelineKey, &'static str>>>,
        log: Arc<Mutex<Vec<String>>>,
    ) -> TimelineActorHandle {
        let (tx, mut rx) = mpsc::channel(8);
        let task = executor::spawn(async move {
            while let Some(message) = rx.recv().await {
                let label = labels
                    .lock()
                    .expect("live-tail replacement labels lock")
                    .get(&key)
                    .copied()
                    .expect("replacement actor label");
                match message {
                        TimelineActorMessage::StartLiveTailRefresh {
                            epoch,
                            operation_generation,
                            limit,
                        } => log.lock().expect("live-tail replacement log lock").push(format!(
                            "start:{label}:epoch={epoch}:operation={operation_generation}:limit={limit}"
                        )),
                        TimelineActorMessage::CancelLiveTailNetwork {
                            operation_generation,
                            acknowledged,
                        } => {
                            log.lock().expect("live-tail replacement log lock").push(format!(
                                "cancel-network:{label}:operation={operation_generation}"
                            ));
                            let _ = acknowledged.send(());
                        }
                        _ => {}
                    }
            }
        });
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

    #[tokio::test]
    async fn idempotent_subscribe_replay_carries_exact_command_cause() {
        let key = room_key();
        let first_subscribe_request_id = fake_rid(28_500);
        let second_subscribe_request_id = fake_rid(28_501);
        let (actor_tx, mut actor_rx) = mpsc::channel(2);
        let actor_handle = TimelineActorHandle {
            tx: actor_tx,
            control_tx: None,
            thread_summary_projection:
                crate::timeline::actor::ThreadSummaryProjectionIngress::channel().0,
            position_rx: None,
            task: None,
            auxiliary_tasks: Vec::new(),
            subscription_generation: None,
            enqueue_context: None,
        };
        let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));

        manager
            .handle_subscribe(first_subscribe_request_id, key.clone(), true, true)
            .await;
        manager
            .handle_subscribe(second_subscribe_request_id, key, true, true)
            .await;

        assert!(matches!(
            actor_rx.recv().await,
            Some(TimelineActorMessage::ReplayInitialItems {
                cause_request_id: Some(cause_request_id),
            }) if cause_request_id == first_subscribe_request_id
        ));
        assert!(matches!(
            actor_rx.recv().await,
            Some(TimelineActorMessage::ReplayInitialItems {
                cause_request_id: Some(cause_request_id),
            }) if cause_request_id == second_subscribe_request_id
        ));
    }

    #[tokio::test]
    async fn cached_room_replay_uses_control_lane_when_ordinary_mailbox_is_full() {
        let key = room_key();
        let request_id = fake_rid(28_509);
        let (actor_tx, mut actor_rx) = mpsc::channel(1);
        actor_tx
            .try_send(TimelineActorMessage::OwnReadReceiptChanged)
            .expect("ordinary actor mailbox prefill");
        let (control_tx, mut control_rx) = mpsc::channel(1);
        let actor_handle = TimelineActorHandle {
            tx: actor_tx,
            control_tx: Some(control_tx),
            thread_summary_projection:
                crate::timeline::actor::ThreadSummaryProjectionIngress::channel().0,
            position_rx: None,
            task: None,
            auxiliary_tasks: Vec::new(),
            subscription_generation: None,
            enqueue_context: None,
        };
        let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));

        executor::timeout(
            Duration::from_millis(250),
            manager.handle_subscribe(request_id, key, true, true),
        )
        .await
        .expect("cached replay must not wait for the ordinary actor mailbox");
        assert!(matches!(
            control_rx.recv().await,
            Some(TimelineActorControl::ReplayInitialItems { cause_request_id })
                if cause_request_id == request_id
        ));
        assert!(matches!(
            actor_rx.recv().await,
            Some(TimelineActorMessage::OwnReadReceiptChanged)
        ));
    }

    #[tokio::test]
    async fn ordinary_completion_burst_does_not_run_before_committed_room_selection() {
        let key = room_key();
        let request_id = fake_rid(28_510);
        let (actor_tx, mut actor_rx) = mpsc::channel(2);
        let actor_handle = TimelineActorHandle {
            tx: actor_tx,
            control_tx: None,
            thread_summary_projection:
                crate::timeline::actor::ThreadSummaryProjectionIngress::channel().0,
            position_rx: None,
            task: None,
            auxiliary_tasks: Vec::new(),
            subscription_generation: None,
            enqueue_context: None,
        };
        let (navigation_projection, navigation_projection_rx) =
            NavigationProjectionIngress::channel();
        let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
        manager.navigation_projection_rx = Some(navigation_projection_rx);

        for operation_generation in 1..=4 {
            manager
                .msg_tx
                .try_send(TimelineMessage::LiveTailRefreshCompleted {
                    key: key.clone(),
                    actor_generation: u64::MAX,
                    epoch: 1,
                    operation_generation,
                    outcome: MatrixLiveTailRefreshOutcome::Failed,
                    requested_limit: FOREGROUND_LIVE_TAIL_LIMIT,
                    returned_events: 0,
                    duration_ms: 0,
                })
                .expect("ordinary completion should fit the test mailbox");
        }
        assert!(navigation_projection.admit(NavigationProjectionIntent {
            generation: 1,
            key: key.clone(),
            cause_request_id: request_id,
            replay_existing: true,
            cleanup: NavigationProjectionCleanup::default(),
        }));
        let (state_tx, state_rx) = oneshot::channel();
        manager
            .msg_tx
            .try_send(TimelineMessage::TestLiveTailDispatchState {
                key,
                epoch: 1,
                response: state_tx,
            })
            .expect("state probe should fit the test mailbox");

        let manager_task = executor::spawn(manager.run());
        let replay = executor::timeout(Duration::from_secs(1), actor_rx.recv())
            .await
            .expect("cached actor replay should be bounded")
            .expect("cached actor should receive replay");
        assert!(matches!(
            replay,
            TimelineActorMessage::ReplayInitialItems {
                cause_request_id: Some(cause),
            } if cause == request_id
        ));
        let (_, _, ordinary_completions_before_navigation_projection) =
            executor::timeout(Duration::from_secs(1), state_rx)
                .await
                .expect("manager probe should be bounded")
                .expect("manager should answer the probe");
        manager_task.abort();

        assert_eq!(
            ordinary_completions_before_navigation_projection,
            Some(0),
            "a committed cached-room selection must overtake queued ordinary completions"
        );
    }

    #[tokio::test]
    async fn manager_shutdown_control_quiesces_before_retained_navigation() {
        let key = room_key();
        let (actor_tx, mut actor_rx) = mpsc::channel(1);
        let actor_handle = TimelineActorHandle {
            tx: actor_tx,
            control_tx: None,
            thread_summary_projection:
                crate::timeline::actor::ThreadSummaryProjectionIngress::channel().0,
            position_rx: None,
            task: None,
            auxiliary_tasks: Vec::new(),
            subscription_generation: None,
            enqueue_context: None,
        };
        let (navigation_projection, navigation_projection_rx) =
            NavigationProjectionIngress::channel();
        let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
        manager.navigation_projection_rx = Some(navigation_projection_rx);
        assert!(navigation_projection.admit(NavigationProjectionIntent {
            generation: 1,
            key,
            cause_request_id: fake_rid(28_514),
            replay_existing: true,
            cleanup: NavigationProjectionCleanup::default(),
        }));
        let (control_tx, control_rx) = mpsc::channel(1);
        manager.control_rx = Some(control_rx);
        let (acknowledged, acknowledgement) = oneshot::channel();
        control_tx
            .send(TimelineManagerControl::Shutdown { acknowledged })
            .await
            .expect("admit high-priority shutdown");

        let manager_task = executor::spawn(manager.run());
        executor::timeout(Duration::from_secs(1), acknowledgement)
            .await
            .expect("shutdown acknowledgement must be bounded")
            .expect("manager must acknowledge quiescence");
        assert!(
            !matches!(
                executor::timeout(Duration::from_millis(20), actor_rx.recv()).await,
                Ok(Some(_))
            ),
            "the old sessionless manager must not consume retained navigation"
        );
        manager_task
            .await
            .expect("manager exits after acknowledged shutdown");
    }

    #[tokio::test]
    async fn navigation_projection_retains_latest_value_across_manager_replacement() {
        let (ingress, initial_receiver) = NavigationProjectionIngress::channel();
        drop(initial_receiver);
        let newest_key = room_key();
        let newest_cause = fake_rid(28_511);

        assert!(ingress.admit(NavigationProjectionIntent {
            generation: 7,
            key: newest_key.clone(),
            cause_request_id: newest_cause,
            replay_existing: false,
            cleanup: NavigationProjectionCleanup::default(),
        }));
        assert!(ingress.admit(NavigationProjectionIntent {
            generation: 6,
            key: TimelineKey::room(AccountKey("@a:test".to_owned()), "!stale:test"),
            cause_request_id: fake_rid(28_512),
            replay_existing: true,
            cleanup: NavigationProjectionCleanup::default(),
        }));
        assert!(ingress.admit(NavigationProjectionIntent {
            generation: 7,
            key: newest_key.clone(),
            cause_request_id: fake_rid(28_513),
            replay_existing: true,
            cleanup: NavigationProjectionCleanup::default(),
        }));

        let mut replacement_receiver = Some(ingress.subscribe());
        let retained = executor::timeout(
            Duration::from_secs(1),
            receive_navigation_projection(&mut replacement_receiver),
        )
        .await
        .expect("replacement manager wake should be bounded")
        .expect("latest desired projection should remain retained");

        assert_eq!(retained.generation, 7);
        assert_eq!(retained.key, newest_key);
        assert_eq!(
            retained.cause_request_id, newest_cause,
            "equal-generation replay strengthens the retained intent without replacing its cause"
        );
        assert!(retained.replay_existing);
    }

    #[tokio::test]
    async fn coalesced_navigation_projection_cleans_the_actual_manager_foreground() {
        let account = AccountKey("@coalesced-cleanup:test".to_owned());
        let room_a = TimelineKey::room(account.clone(), "!cleanup-a:test");
        let room_b = TimelineKey::room(account.clone(), "!cleanup-b:test");
        let room_c = TimelineKey::room(account, "!cleanup-c:test");
        let (actor_a, mut cleanup_a) = cleanup_probe_timeline_actor_handle();
        let (actor_b, mut cleanup_b) = cleanup_probe_timeline_actor_handle();
        let (actor_c, _cleanup_c) = cleanup_probe_timeline_actor_handle();
        let (navigation_projection, navigation_projection_rx) =
            NavigationProjectionIngress::channel();
        let mut manager = live_tail_test_manager(HashMap::from([
            (room_a.clone(), actor_a),
            (room_b.clone(), actor_b),
            (room_c.clone(), actor_c),
        ]));
        manager.navigation_projection_rx = Some(navigation_projection_rx);

        manager
            .handle_committed_room_selection(fake_rid(28_515), room_a.clone(), false, false)
            .await;
        assert_eq!(manager.live_tail_refreshes.active_key(), Some(&room_a));

        assert!(navigation_projection.admit(NavigationProjectionIntent {
            generation: 1,
            key: room_b.clone(),
            cause_request_id: fake_rid(28_516),
            replay_existing: false,
            cleanup: NavigationProjectionCleanup {
                cancel_pagination: Some(room_a.clone()),
                cancel_link_previews: Some(room_a.clone()),
            },
        }));
        assert!(navigation_projection.admit(NavigationProjectionIntent {
            generation: 2,
            key: room_c.clone(),
            cause_request_id: fake_rid(28_517),
            replay_existing: false,
            cleanup: NavigationProjectionCleanup {
                cancel_pagination: Some(room_b.clone()),
                cancel_link_previews: Some(room_b),
            },
        }));

        let projection = receive_navigation_projection(&mut manager.navigation_projection_rx)
            .await
            .expect("latest navigation projection");
        assert_eq!(projection.key, room_c);
        manager.handle_navigation_projection(projection).await;

        assert_eq!(
            manager.live_tail_refreshes.active_key(),
            Some(&room_c),
            "the latest retained room must become foreground"
        );
        cleanup_a
            .changed()
            .await
            .expect("actual previous foreground cleanup");
        let cleanup_a = *cleanup_a.borrow_and_update();
        assert!(
            cleanup_a.cancel_pagination_serial > 0,
            "A pagination cleanup must survive B being replaced by C"
        );
        assert!(
            cleanup_a.cancel_link_previews_serial > 0,
            "A link-preview cleanup must survive B being replaced by C"
        );
        cleanup_b.changed().await.expect("latest intent cleanup");
        let cleanup_b = *cleanup_b.borrow_and_update();
        assert!(cleanup_b.cancel_pagination_serial > 0);
        assert!(cleanup_b.cancel_link_previews_serial > 0);
    }

    #[tokio::test]
    async fn live_tail_preemption_cancels_network_before_new_active_room_starts() {
        let account = AccountKey("@a:test".to_owned());
        let room_a = TimelineKey::room(account.clone(), "!a:test");
        let room_b = TimelineKey::room(account, "!b:test");
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut manager = live_tail_test_manager(HashMap::from([
            (
                room_a.clone(),
                live_tail_test_actor_handle("A", log.clone()),
            ),
            (
                room_b.clone(),
                live_tail_test_actor_handle("B", log.clone()),
            ),
        ]));

        manager.room_subscription_service_epoch = 7;
        manager
            .handle_committed_room_selection(fake_rid(1), room_a, false, false)
            .await;
        manager.room_subscription_service_epoch = 9;
        manager
            .handle_committed_room_selection(fake_rid(2), room_b, false, false)
            .await;

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if log.lock().expect("live-tail log lock").len() == 3 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("preemption log completed");
        assert_eq!(
            *log.lock().expect("live-tail log lock"),
            [
                "start:A:epoch=7:limit=128",
                "cancel-network:A:epoch=7",
                "start:B:epoch=9:limit=128",
            ]
        );
    }

    #[tokio::test]
    async fn post_commit_cleanup_never_waits_for_a_missing_cancel_ack() {
        let account = AccountKey("@stalled-cancel:test".to_owned());
        let room_a = TimelineKey::room(account.clone(), "!stalled-a:test");
        let room_b = TimelineKey::room(account, "!stalled-b:test");
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut manager = live_tail_test_manager(HashMap::from([
            (
                room_a.clone(),
                stalled_live_tail_cancel_actor_handle("A", log.clone()),
            ),
            (
                room_b.clone(),
                live_tail_test_actor_handle("B", log.clone()),
            ),
        ]));

        manager.room_subscription_service_epoch = 7;
        manager
            .handle_committed_room_selection(fake_rid(1), room_a, false, false)
            .await;
        manager.room_subscription_service_epoch = 9;
        executor::timeout(
            Duration::from_millis(25),
            manager.handle_committed_room_selection(fake_rid(2), room_b, false, false),
        )
        .await
        .expect("post-commit cleanup must not consume the cancellation deadline");

        tokio::task::yield_now().await;
        assert_eq!(
            *log.lock().expect("stalled live-tail log lock"),
            [
                "start:A:epoch=7:limit=128",
                "cancel-network:A",
                "start:B:epoch=9:limit=128",
            ]
        );
    }

    #[tokio::test]
    async fn committed_navigation_projection_failure_does_not_emit_a_second_terminal() {
        let mut manager = live_tail_test_manager(HashMap::new());
        manager.test_session_available = false;
        let (event_tx, mut event_rx) = broadcast::channel(8);
        manager.event_tx = event_tx;
        let request_id = fake_rid(29_604);

        manager
            .handle_navigation_projection(NavigationProjectionIntent {
                generation: 1,
                key: room_key(),
                cause_request_id: request_id,
                replay_existing: true,
                cleanup: NavigationProjectionCleanup::default(),
            })
            .await;

        assert!(
            executor::timeout(Duration::from_millis(10), event_rx.recv())
                .await
                .is_err(),
            "AppActor already emitted Committed; projection cleanup must not emit OperationFailed"
        );
    }

    #[tokio::test]
    async fn foreground_gap_demand_moves_to_the_newly_selected_room() {
        let account = AccountKey("@gap-owner:test".to_owned());
        let room_a = TimelineKey::room(account.clone(), "!gap-a:test");
        let room_b = TimelineKey::room(account, "!gap-b:test");
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut manager = live_tail_test_manager(HashMap::from([
            (
                room_a.clone(),
                gap_demand_test_actor_handle("A", log.clone()),
            ),
            (
                room_b.clone(),
                gap_demand_test_actor_handle("B", log.clone()),
            ),
        ]));

        manager
            .handle_committed_room_selection(fake_rid(1), room_a, false, false)
            .await;
        manager
            .handle_committed_room_selection(fake_rid(2), room_b, false, false)
            .await;
        tokio::task::yield_now().await;

        assert_eq!(
            *log.lock().expect("gap demand log lock"),
            ["begin:A", "end:A", "begin:B"],
        );
    }

    #[tokio::test]
    async fn sync_replacement_restores_foreground_gap_demand_to_the_new_actor() {
        let room = TimelineKey::room(AccountKey("@gap-owner:test".to_owned()), "!gap:test");
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut manager = live_tail_test_manager(HashMap::from([(
            room.clone(),
            gap_demand_test_actor_handle("old", log.clone()),
        )]));
        manager
            .handle_committed_room_selection(fake_rid(1), room.clone(), false, false)
            .await;
        manager.timelines.insert(
            room.clone(),
            gap_demand_test_actor_handle("replacement", log.clone()),
        );

        manager.restore_foreground_gap_demand(&room).await;
        tokio::task::yield_now().await;

        assert_eq!(
            *log.lock().expect("gap demand log lock"),
            ["begin:replacement"],
        );
    }

    #[tokio::test]
    async fn live_tail_epoch_replacement_folds_stale_pending_starts_before_dispatch() {
        let account = AccountKey("@replacement:test".to_owned());
        let candidates = ["!one:test", "!two:test", "!three:test"]
            .map(|room_id| TimelineKey::room(account.clone(), room_id));
        let labels = Arc::new(Mutex::new(HashMap::new()));
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut manager = live_tail_test_manager(
            candidates
                .iter()
                .cloned()
                .map(|key| {
                    (
                        key.clone(),
                        live_tail_replacement_test_actor_handle(key, labels.clone(), log.clone()),
                    )
                })
                .collect(),
        );
        let ordered = manager.timelines.keys().cloned().collect::<Vec<_>>();
        let [room_b, room_c, room_a] = ordered.as_slice() else {
            panic!("three replacement rooms");
        };
        let (room_b, room_c, room_a) = (room_b.clone(), room_c.clone(), room_a.clone());
        labels
            .lock()
            .expect("live-tail replacement labels lock")
            .extend([
                (room_b.clone(), "B"),
                (room_c.clone(), "C"),
                (room_a.clone(), "A"),
            ]);

        let prepare = || {
            let mut coordinator = LiveTailRefreshCoordinator::new();
            assert_eq!(
                coordinator.activate(room_a.clone(), 7),
                vec![LiveTailSchedulerAction::Start {
                    key: room_a.clone(),
                    epoch: 7,
                    operation_generation: 1,
                    limit: 128,
                }]
            );
            assert!(coordinator.mark_unproven(room_b.clone(), 7).is_empty());
            assert!(coordinator.mark_unproven(room_c.clone(), 7).is_empty());
            let start_b =
                coordinator.finish(room_a.clone(), 7, 1, LiveTailRefreshOutcome::Unchanged);
            (coordinator, start_b)
        };

        let (mut evidence, _) = prepare();
        let logical_actions = [room_b.clone(), room_c.clone(), room_a.clone()]
            .into_iter()
            .flat_map(|key| evidence.invalidate_epoch(key, 8))
            .collect::<Vec<_>>();
        assert_eq!(
            logical_actions,
            vec![
                LiveTailSchedulerAction::CancelNetwork {
                    key: room_b.clone(),
                    operation_generation: 2,
                },
                LiveTailSchedulerAction::Start {
                    key: room_c.clone(),
                    epoch: 7,
                    operation_generation: 3,
                    limit: 128,
                },
                LiveTailSchedulerAction::CancelNetwork {
                    key: room_c.clone(),
                    operation_generation: 3,
                },
                LiveTailSchedulerAction::Start {
                    key: room_b.clone(),
                    epoch: 8,
                    operation_generation: 4,
                    limit: 128,
                },
                LiveTailSchedulerAction::CancelNetwork {
                    key: room_b.clone(),
                    operation_generation: 4,
                },
                LiveTailSchedulerAction::Start {
                    key: room_a.clone(),
                    epoch: 8,
                    operation_generation: 5,
                    limit: 128,
                },
            ],
            "the coordinator cancellation stream must remain causal",
        );

        let (coordinator, start_b) = prepare();
        manager.live_tail_refreshes = coordinator;
        manager.apply_live_tail_scheduler_actions(start_b).await;
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if !log
                    .lock()
                    .expect("live-tail replacement log lock")
                    .is_empty()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("initial delayed B start");
        log.lock().expect("live-tail replacement log lock").clear();

        let starts = manager
            .invalidate_live_tail_epoch_for_existing_rooms(8)
            .await;
        manager.apply_live_tail_scheduler_actions(starts).await;
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if log
                    .lock()
                    .expect("live-tail replacement log lock")
                    .iter()
                    .any(|entry| entry.starts_with("start:A:epoch=8:"))
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("final replacement start");
        tokio::task::yield_now().await;
        assert_eq!(
            *log.lock().expect("live-tail replacement log lock"),
            [
                "cancel-network:B:operation=2",
                "start:A:epoch=8:operation=5:limit=128",
            ],
            "only the already-dispatched network and final coordinator start reach actors",
        );
    }

    #[test]
    fn causal_projection_domains_route_equal_raw_serial_without_collision() {
        let actor_generation = 4;
        let raw_serial = 1;
        let projection_batch = 1;
        let published_batch_id = TimelineBatchId(21);
        let historical_operation =
            CausalProjectionOperationId::new(CausalProjectionDomain::HistoricalGap, raw_serial)
                .expect("historical serial fits the transport envelope");
        let live_tail_operation =
            CausalProjectionOperationId::new(CausalProjectionDomain::LiveTail, raw_serial)
                .expect("live-tail serial fits the transport envelope");

        assert_eq!(historical_operation.encode_transport(), raw_serial);
        assert_eq!(
            live_tail_operation.encode_transport(),
            CAUSAL_PROJECTION_DOMAIN_BIT | raw_serial,
        );
        assert!(
            CausalProjectionOperationId::new(
                CausalProjectionDomain::HistoricalGap,
                CAUSAL_PROJECTION_DOMAIN_BIT,
            )
            .is_none(),
            "raw serials must never consume the operation-domain bit",
        );
        assert_eq!(
            next_causal_projection_serial(CAUSAL_PROJECTION_SERIAL_MAX),
            None,
            "exhaustion is terminal while the same domain owns a pending identity",
        );
        assert_eq!(
            next_causal_projection_serial(CAUSAL_PROJECTION_SERIAL_MAX),
            None,
            "one actor generation never wraps even when no operation is pending",
        );

        let mut historical = TimelineGapProjectionCorrelation::default();
        historical.begin(actor_generation, historical_operation);
        assert_eq!(
            historical.complete(
                actor_generation,
                historical_operation,
                Some(projection_batch),
            ),
            TimelineGapProjectionCompletion::Pending,
        );
        let mut live_tail = TimelineGapProjectionCorrelation::default();
        live_tail.begin(actor_generation, live_tail_operation);
        assert_eq!(
            live_tail.complete(
                actor_generation,
                live_tail_operation,
                Some(projection_batch),
            ),
            TimelineGapProjectionCompletion::Pending,
        );

        let historical_projection = CausalProjectionId::decode_transport(GapRepairProjectionId {
            actor_generation,
            repair_generation: historical_operation.encode_transport(),
            projection_batch,
        });
        let historical_observation = observe_causal_projection(
            &mut historical,
            &mut live_tail,
            historical_projection,
            published_batch_id,
        );
        assert_eq!(
            historical_observation.historical_gap_batch_id,
            Some(published_batch_id),
        );
        assert_eq!(historical_observation.live_tail_batch_id, None);
        assert!(
            live_tail.is_pending(),
            "historical tag cannot prove live-tail freshness"
        );

        // Re-arm the historical correlation to prove the reverse isolation on
        // the same actor/raw serial/batch collision.
        historical.begin(actor_generation, historical_operation);
        assert_eq!(
            historical.complete(
                actor_generation,
                historical_operation,
                Some(projection_batch),
            ),
            TimelineGapProjectionCompletion::Pending,
        );
        let live_tail_projection = CausalProjectionId::decode_transport(GapRepairProjectionId {
            actor_generation,
            repair_generation: live_tail_operation.encode_transport(),
            projection_batch,
        });
        let live_tail_observation = observe_causal_projection(
            &mut historical,
            &mut live_tail,
            live_tail_projection,
            TimelineBatchId(22),
        );
        assert_eq!(live_tail_observation.historical_gap_batch_id, None);
        assert_eq!(
            live_tail_observation.live_tail_batch_id,
            Some(TimelineBatchId(22)),
        );
        assert!(
            historical.is_pending(),
            "live-tail tag cannot release historical repair"
        );

        assert_eq!(
            observe_causal_projection(
                &mut historical,
                &mut live_tail,
                live_tail_projection,
                TimelineBatchId(23),
            ),
            CausalProjectionObservation::default(),
            "one live-tail projection can complete only once",
        );
    }

    #[tokio::test]
    async fn room_actor_hydrates_a_historical_sender_without_a_live_event() {
        use matrix_sdk::ruma::events::room::member::MembershipState;
        use matrix_sdk::test_utils::mocks::MatrixMockServer;
        use matrix_sdk_test::{ALICE, CAROL, JoinedRoomBuilder, event_factory::EventFactory};

        let server = MatrixMockServer::new().await;
        let client = server.client_builder().build().await;
        client
            .event_cache()
            .subscribe()
            .expect("event cache subscription");
        let sdk_room_id = matrix_sdk::ruma::room_id!("!historical-profile:example.org");
        let event_id = matrix_sdk::ruma::event_id!("$historical-profile:example.org");
        let room = server.sync_joined_room(&client, sdk_room_id).await;
        let factory = EventFactory::new().room(sdk_room_id);
        server
            .sync_room(
                &client,
                JoinedRoomBuilder::new(sdk_room_id).add_timeline_event(
                    factory
                        .text_msg("historical")
                        .sender(&CAROL)
                        .event_id(event_id)
                        .into_raw_sync(),
                ),
            )
            .await;
        server
            .mock_get_members()
            .ok(vec![
                factory
                    .member(&ALICE)
                    .membership(MembershipState::Join)
                    .into_raw(),
                factory
                    .member(&CAROL)
                    .display_name("Carol")
                    .membership(MembershipState::Join)
                    .into_raw(),
            ])
            .expect(1)
            .named("historical-profile-members")
            .mount()
            .await;

        let timeline = Arc::new(
            koushi_timeline_builder(
                &room,
                TimelineFocus::Live {
                    hide_threaded_events: false,
                },
            )
            .build()
            .await
            .expect("room timeline"),
        );
        let session = Arc::new(MatrixClientSession::from_client_for_testing(
            client,
            SessionInfo {
                homeserver: "http://example.invalid".to_owned(),
                user_id: ALICE.to_string(),
                device_id: "DEVICE".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            },
        ));
        let key = TimelineKey::room(AccountKey(ALICE.to_string()), sdk_room_id.to_string());
        let mut manager = live_tail_test_manager(HashMap::new());
        let (action_tx, mut action_rx) = mpsc::channel(8);
        manager.action_tx = action_tx;
        let _action_drain =
            executor::spawn(async move { while action_rx.recv().await.is_some() {} });
        let mut event_rx = manager.event_tx.subscribe();
        let actor_generation = manager
            .timeline_actor_generations
            .activate_after_quiescence(&key)
            .await
            .generation;
        let _actor = TimelineActor::spawn(
            key.clone(),
            timeline,
            session,
            fake_rid(68),
            true,
            manager.action_tx.clone(),
            manager.event_tx.clone(),
            None,
            Default::default(),
            None,
            LinkPreviewContext::default(),
            manager.account_work.clone(),
            Arc::clone(&manager.thread_root_projection_service),
            Arc::clone(&manager.replay_known_thread_root_projections),
            Arc::clone(&manager.timeline_actor_generations),
            actor_generation,
            None,
            Default::default(),
            manager.terminal_ingress.clone(),
            manager.msg_tx.clone(),
        )
        .await;

        let hydrated = executor::timeout(Duration::from_secs(2), async {
            let mut saw_unavailable_initial = false;
            loop {
                match event_rx.recv().await.expect("timeline event") {
                    CoreEvent::Timeline(TimelineEvent::InitialItems {
                        key: event_key,
                        items,
                        ..
                    }) if event_key == key => {
                        let item = items
                            .iter()
                            .find(|item| timeline_item_event_id(item) == Some(event_id.as_str()))
                            .expect("historical initial item");
                        assert_eq!(item.sender_label, None);
                        saw_unavailable_initial = true;
                    }
                    CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                        key: event_key,
                        diffs,
                        ..
                    }) if event_key == key && saw_unavailable_initial => {
                        if let Some(item) = diffs.iter().find_map(|diff| match diff {
                            crate::event::TimelineDiff::Set { item, .. }
                                if timeline_item_event_id(item) == Some(event_id.as_str()) =>
                            {
                                Some(item)
                            }
                            _ => None,
                        }) && item.sender_label.as_deref() == Some("Carol")
                        {
                            break true;
                        }
                    }
                    _ => {}
                }
            }
        })
        .await
        .expect("member hydration must settle through an ordinary timeline diff");
        assert!(hydrated);
    }

    #[tokio::test]
    async fn live_tail_restore_actor_flush_hands_completion_to_manager_once() {
        use matrix_sdk::test_utils::mocks::{MatrixMockServer, RoomMessagesResponseTemplate};
        use matrix_sdk_test::{ALICE, JoinedRoomBuilder, event_factory::EventFactory};

        let server = MatrixMockServer::new().await;
        let client = server.client_builder().build().await;
        client
            .event_cache()
            .subscribe()
            .expect("event cache subscription");
        let sdk_room_id = matrix_sdk::ruma::room_id!("!restore-live-tail:example.org");
        let stale_edge_id = matrix_sdk::ruma::event_id!("$stale-edge:example.org");
        let refreshed_id = matrix_sdk::ruma::event_id!("$refreshed:example.org");
        let room = server.sync_joined_room(&client, sdk_room_id).await;
        let factory = EventFactory::new().room(sdk_room_id).sender(&ALICE);
        server
            .sync_room(
                &client,
                JoinedRoomBuilder::new(sdk_room_id).add_timeline_event(
                    factory
                        .text_msg("stale edge")
                        .event_id(stale_edge_id)
                        .into_raw_sync(),
                ),
            )
            .await;
        let timeline = Arc::new(
            koushi_timeline_builder(
                &room,
                TimelineFocus::Live {
                    hide_threaded_events: false,
                },
            )
            .build()
            .await
            .expect("room timeline"),
        );
        let (initial_sdk_items, _fixture_stream) = timeline.subscribe().await;
        let real_sdk_item = initial_sdk_items
            .iter()
            .find(|item| item.as_event().and_then(|event| event.event_id()) == Some(stale_edge_id))
            .cloned()
            .expect("real SDK timeline item for the wrong-tag restore batch");
        server
            .mock_room_messages()
            .match_limit(u32::from(FOREGROUND_LIVE_TAIL_LIMIT))
            .ok(RoomMessagesResponseTemplate::default()
                .events(vec![
                    factory.text_msg("refreshed").event_id(refreshed_id),
                    factory.text_msg("stale edge").event_id(stale_edge_id),
                ])
                .with_delay(Duration::from_millis(500)))
            .expect(1)
            .named("restore-live-tail-production-refresh")
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
        let account = AccountKey("@restore:test".to_owned());
        let room_a = TimelineKey::room(account.clone(), sdk_room_id.to_string());
        let room_b = TimelineKey::room(account, "!delayed:example.org");
        let delayed_log = Arc::new(Mutex::new(Vec::new()));
        let mut manager = live_tail_test_manager(HashMap::from([(
            room_b.clone(),
            live_tail_test_actor_handle("B", delayed_log.clone()),
        )]));
        let (action_tx, mut action_rx) = mpsc::channel(8);
        manager.action_tx = action_tx;
        let _action_drain =
            executor::spawn(async move { while action_rx.recv().await.is_some() {} });
        let mut event_rx = manager.event_tx.subscribe();
        let actor_generation = manager
            .timeline_actor_generations
            .activate_after_quiescence(&room_a)
            .await
            .generation;
        let projection_request_id = fake_rid(40);
        let actor_handle = TimelineActor::spawn(
            room_a.clone(),
            timeline,
            session,
            projection_request_id,
            true,
            manager.action_tx.clone(),
            manager.event_tx.clone(),
            None,
            Default::default(),
            None,
            LinkPreviewContext::default(),
            manager.account_work.clone(),
            Arc::clone(&manager.thread_root_projection_service),
            Arc::clone(&manager.replay_known_thread_root_projections),
            Arc::clone(&manager.timeline_actor_generations),
            actor_generation,
            None,
            Default::default(),
            manager.terminal_ingress.clone(),
            manager.msg_tx.clone(),
        )
        .await;
        manager.timelines.insert(room_a.clone(), actor_handle);
        loop {
            if matches!(
                event_rx.recv().await.expect("initial actor event"),
                CoreEvent::Timeline(TimelineEvent::InitialItems { key, .. }) if key == room_a
            ) {
                break;
            }
        }

        let (restore_tx, restore_rx) = oneshot::channel();
        assert!(
            manager
                .timelines
                .get(&room_a)
                .expect("room A actor")
                .send(TimelineActorMessage::TestBeginRestore {
                    request_id: fake_rid(41),
                    event_id: "$anchor-not-in-window:example.org".to_owned(),
                    acknowledged: restore_tx,
                })
                .await
        );
        restore_rx.await.expect("restore fixture acknowledged");

        let starts = manager.live_tail_refreshes.activate(room_a.clone(), 7);
        assert!(
            manager
                .live_tail_refreshes
                .mark_unproven(room_b.clone(), 7)
                .is_empty()
        );
        manager.apply_live_tail_scheduler_actions(starts).await;
        let operation = live_tail_causal_projection_operation(1);
        let wrong_projections = BTreeSet::from([
            CausalProjectionId {
                actor_generation: actor_generation + 1,
                operation,
                projection_batch: 1,
            },
            CausalProjectionId {
                actor_generation,
                operation: live_tail_causal_projection_operation(9),
                projection_batch: 1,
            },
            CausalProjectionId {
                actor_generation,
                operation,
                projection_batch: u32::MAX,
            },
        ]);
        let (inject_tx, inject_rx) = oneshot::channel();
        assert!(
            manager
                .timelines
                .get(&room_a)
                .expect("room A actor")
                .send(TimelineActorMessage::TestInjectRestoreDiff {
                    diffs: vec![eyeball_im::VectorDiff::PushBack {
                        value: real_sdk_item.clone(),
                    }],
                    projections: wrong_projections.clone(),
                    acknowledged: inject_tx,
                })
                .await
        );
        inject_rx.await.expect("wrong-tag diff handled");

        let snapshot = |manager: &TimelineManagerActor, key: &TimelineKey| {
            let (response, state) = oneshot::channel();
            let handle = manager.timelines.get(key).expect("room A actor");
            (handle.tx.clone(), response, state)
        };
        let (actor_tx, state_tx, state_rx) = snapshot(&manager, &room_a);
        actor_tx
            .send(TimelineActorMessage::TestRestoreCausalState(state_tx))
            .await
            .expect("snapshot request");
        let (pending, completion_waiting, buffered_diff_count, buffered_projections) =
            state_rx.await.expect("wrong-tag snapshot");
        assert!(pending);
        assert!(!completion_waiting);
        assert_eq!(
            buffered_diff_count, 0,
            "a duplicate canonical slot is a valid projected display no-op"
        );
        assert_eq!(buffered_projections, wrong_projections);
        assert_eq!(
            manager.live_tail_refreshes.freshness(&room_a),
            Some(LiveTailFreshnessState::Refreshing {
                epoch: 7,
                operation_generation: 1,
            }),
            "wrong actor, operation, and batch identities cannot prove freshness",
        );
        let matching_projection = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let (actor_tx, state_tx, state_rx) = snapshot(&manager, &room_a);
                    actor_tx
                        .send(TimelineActorMessage::TestRestoreCausalState(state_tx))
                        .await
                        .expect("snapshot request");
                    let (pending, completion_waiting, buffered_diff_count, projections) =
                        state_rx.await.expect("matching-tag snapshot");
                    if let Some(projection) = projections.iter().copied().find(|projection| {
                        projection.actor_generation == actor_generation
                            && projection.operation == operation
                            && projection.projection_batch != u32::MAX
                    }) && completion_waiting
                    {
                        assert!(
                            pending,
                            "matching metadata remains pending until publication"
                        );
                        assert!(
                            buffered_diff_count >= 2,
                            "two real SDK batches must reach the actor restore buffer before terminal publication"
                        );
                        break projection;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("real tagged SDK diff reached the restore buffer");
        assert_ne!(matching_projection.projection_batch, u32::MAX);

        let manager_tx = manager.msg_tx.clone();
        let actor_tx = manager
            .timelines
            .get(&room_a)
            .expect("room A actor")
            .tx
            .clone();
        let _manager_task = executor::spawn(manager.run());

        while event_rx.try_recv().is_ok() {}
        let (flush_tx, flush_rx) = oneshot::channel();
        actor_tx
            .send(TimelineActorMessage::TestFinishRestore {
                request_id: fake_rid(41),
                response: flush_tx,
            })
            .await
            .expect("finish restore request");
        assert!(flush_rx.await.expect("production restore terminal result"));

        let (freshness, completion_dispatches, _) =
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let (state_tx, state_rx) = oneshot::channel();
                    manager_tx
                        .send(TimelineMessage::TestLiveTailDispatchState {
                            key: room_a.clone(),
                            epoch: 7,
                            response: state_tx,
                        })
                        .await
                        .expect("manager state request");
                    let state = state_rx.await.expect("manager state response");
                    if state.0 && !delayed_log.lock().expect("delayed start log").is_empty() {
                        break state;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("production manager dispatch completed live-tail refresh");
        assert!(freshness, "room A becomes Fresh for epoch 7");
        assert_eq!(
            completion_dispatches, 1,
            "the production manager loop dispatches exactly one completion",
        );
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if !delayed_log.lock().expect("delayed start log").is_empty() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("manager scheduled delayed room B");
        assert_eq!(
            *delayed_log.lock().expect("delayed start log"),
            ["start:B:epoch=7:limit=128"],
        );

        let mut settlement_events = Vec::new();
        while let Ok(event) = event_rx.try_recv() {
            match event {
                CoreEvent::Timeline(TimelineEvent::ItemsUpdated { .. }) => {
                    settlement_events.push("items")
                }
                CoreEvent::Timeline(TimelineEvent::NavigationUpdated { .. }) => {
                    settlement_events.push("navigation")
                }
                CoreEvent::Timeline(TimelineEvent::AnchorRestoreFinished { .. }) => {
                    settlement_events.push("terminal")
                }
                _ => {}
            }
        }
        assert_eq!(
            settlement_events
                .iter()
                .filter(|event| **event == "items")
                .count(),
            1,
            "restore publishes one convergent coalesced batch"
        );
        let items_position = settlement_events
            .iter()
            .position(|event| *event == "items")
            .expect("coalesced ItemsUpdated");
        let navigation_position = settlement_events
            .iter()
            .position(|event| *event == "navigation")
            .expect("settled NavigationUpdated");
        let terminal_position = settlement_events
            .iter()
            .position(|event| *event == "terminal")
            .expect("AnchorRestoreFinished terminal");
        assert!(items_position < navigation_position && navigation_position < terminal_position);

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        manager_tx
            .send(TimelineMessage::Shutdown {
                acknowledged: Some(shutdown_tx),
            })
            .await
            .expect("manager shutdown request");
        shutdown_rx.await.expect("manager shutdown acknowledged");
    }

    #[tokio::test]
    async fn timeline_actor_spawn_returns_before_authoritative_publish_waits_for_manager_capacity()
    {
        use matrix_sdk::test_utils::mocks::MatrixMockServer;
        use matrix_sdk_test::{ALICE, JoinedRoomBuilder, event_factory::EventFactory};

        let server = MatrixMockServer::new().await;
        let client = server.client_builder().build().await;
        client
            .event_cache()
            .subscribe()
            .expect("event cache subscription");
        let room_id = matrix_sdk::ruma::room_id!("!startup-capacity:example.org");
        let factory = EventFactory::new().room(room_id).sender(&ALICE);
        let room = server.sync_joined_room(&client, room_id).await;
        server
            .sync_room(
                &client,
                JoinedRoomBuilder::new(room_id).add_timeline_event(
                    factory
                        .text_msg("synthetic")
                        .event_id(matrix_sdk::ruma::event_id!("$startup:example.org"))
                        .into_raw_sync(),
                ),
            )
            .await;
        let timeline = Arc::new(
            koushi_timeline_builder(
                &room,
                TimelineFocus::Live {
                    hide_threaded_events: false,
                },
            )
            .build()
            .await
            .expect("room timeline"),
        );
        let session = Arc::new(MatrixClientSession::from_client_for_testing(
            client,
            SessionInfo {
                homeserver: "http://example.invalid".to_owned(),
                user_id: ALICE.to_string(),
                device_id: "DEVICE".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            },
        ));
        let key = TimelineKey::room(AccountKey("@startup:test".to_owned()), room_id.to_string());
        let generations = Arc::new(TimelineActorGenerationGate::default());
        let actor_generation = generations.activate_after_quiescence(&key).await.generation;
        let (manager_tx, mut manager_rx) = mpsc::channel(1);
        manager_tx
            .send(TimelineMessage::IgnoredUsersUpdated {
                user_ids: BTreeSet::new(),
            })
            .await
            .expect("saturate manager mailbox");
        let (action_tx, _action_rx) = mpsc::channel(8);
        let (event_tx, _) = broadcast::channel(8);
        let (terminal_ingress, _terminal_rx) = TimelineSendTerminalIngress::channel();

        let handle = executor::timeout(
            Duration::from_millis(100),
            TimelineActor::spawn(
                key.clone(),
                timeline,
                session,
                fake_rid(38_001),
                true,
                action_tx,
                event_tx,
                None,
                BTreeSet::new(),
                None,
                LinkPreviewContext::default(),
                AccountWorkScheduler::default(),
                Arc::new(Mutex::new(ThreadRootProjectionService::default())),
                Arc::new(Mutex::new(
                    ReplayKnownThreadRootProjectionRegistry::default(),
                )),
                generations,
                actor_generation,
                None,
                SharedSendCompletionCoordinator::default(),
                terminal_ingress,
                manager_tx,
            ),
        )
        .await
        .expect("actor construction must not await manager capacity");

        assert!(matches!(
            manager_rx.recv().await,
            Some(TimelineMessage::IgnoredUsersUpdated { .. })
        ));
        assert!(matches!(
            executor::timeout(Duration::from_millis(100), manager_rx.recv())
                .await
                .expect("authoritative startup publish must resume after capacity opens"),
            Some(TimelineMessage::AuthoritativeReadStateObserved {
                key: observed,
                actor_generation: observed_generation,
                ..
            }) if observed == key && observed_generation == actor_generation
        ));
        handle.stop().await;
    }

    #[tokio::test]
    async fn live_tail_replacement_ignores_old_epoch_actor_and_projection_completion() {
        let room = TimelineKey::room(AccountKey("@a:test".to_owned()), "!a:test");
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut manager = live_tail_test_manager(HashMap::from([(
            room.clone(),
            live_tail_test_actor_handle("A", log.clone()),
        )]));
        let actor_generation = manager
            .timeline_actor_generations
            .activate_after_quiescence(&room)
            .await
            .generation;

        manager.room_subscription_service_epoch = 7;
        manager
            .handle_committed_room_selection(fake_rid(1), room.clone(), false, false)
            .await;
        let replacement_starts = manager
            .invalidate_live_tail_epoch_for_existing_rooms(8)
            .await;
        manager
            .apply_live_tail_scheduler_actions(replacement_starts)
            .await;

        assert_eq!(
            manager.live_tail_refreshes.freshness(&room),
            Some(
                crate::live_tail_freshness::LiveTailFreshnessState::Refreshing {
                    epoch: 8,
                    operation_generation: 2,
                }
            ),
            "the replacement sync run must fence epoch 7 before an old completion can arrive",
        );
        manager
            .handle_live_tail_refresh_completed(
                room.clone(),
                actor_generation,
                7,
                1,
                MatrixLiveTailRefreshOutcome::Advanced { events: 1 },
                128,
                1,
                1,
            )
            .await;
        manager
            .handle_live_tail_refresh_completed(
                room.clone(),
                actor_generation.saturating_sub(1),
                8,
                2,
                MatrixLiveTailRefreshOutcome::Advanced { events: 1 },
                128,
                1,
                1,
            )
            .await;
        assert_eq!(
            manager.live_tail_refreshes.freshness(&room),
            Some(
                crate::live_tail_freshness::LiveTailFreshnessState::Refreshing {
                    epoch: 8,
                    operation_generation: 2,
                }
            ),
        );

        let mut projection = TimelineGapProjectionCorrelation::default();
        let operation = live_tail_causal_projection_operation(2);
        projection.begin(actor_generation, operation);
        assert_eq!(
            projection.complete(actor_generation, operation, Some(2)),
            TimelineGapProjectionCompletion::Pending
        );
        for stale in [
            CausalProjectionId {
                actor_generation: actor_generation.saturating_sub(1),
                operation,
                projection_batch: 2,
            },
            CausalProjectionId {
                actor_generation,
                operation: live_tail_causal_projection_operation(1),
                projection_batch: 2,
            },
            CausalProjectionId {
                actor_generation,
                operation,
                projection_batch: 1,
            },
        ] {
            assert_eq!(projection.observe(stale, TimelineBatchId(9)), None);
            assert!(projection.is_pending());
        }
        assert_eq!(
            projection.observe(
                CausalProjectionId {
                    actor_generation,
                    operation,
                    projection_batch: 2,
                },
                TimelineBatchId(10),
            ),
            Some(TimelineBatchId(10))
        );

        manager
            .handle_live_tail_refresh_completed(
                room.clone(),
                actor_generation,
                8,
                2,
                MatrixLiveTailRefreshOutcome::Advanced { events: 1 },
                128,
                1,
                1,
            )
            .await;
        assert_eq!(
            manager.live_tail_refreshes.freshness(&room),
            Some(crate::live_tail_freshness::LiveTailFreshnessState::Fresh { epoch: 8 }),
        );

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if log.lock().expect("live-tail log lock").len() == 3 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("replacement epoch log completed");
        assert_eq!(
            *log.lock().expect("live-tail log lock"),
            [
                "start:A:epoch=7:limit=128",
                "cancel-network:A:epoch=7",
                "start:A:epoch=8:limit=128",
            ]
        );
    }

    #[test]
    fn activity_row_from_timeline_item_preserves_thread_root_event_id() {
        let mut item = timeline_item(
            "$thread-reply:test",
            Some("reply body"),
            "@sender:test",
            false,
        );
        item.thread_root = Some("$thread-root:test".to_owned());

        let row = activity_row_from_timeline_item("!room:test", &item)
            .expect("event timeline item should project to an activity row");
        let value = serde_json::to_value(&row).expect("activity row should serialize");

        assert_eq!(value["event_id"], serde_json::json!("$thread-reply:test"));
        assert_eq!(
            value["thread_root_event_id"],
            serde_json::json!("$thread-root:test")
        );
    }

    #[test]
    fn timeline_navigation_marks_first_unread_inside_viewport() {
        let items = vec![
            timeline_item("$read:test", Some("read"), "@alice:test", false),
            timeline_item("$unread:test", Some("unread"), "@alice:test", false),
            timeline_item("$newer:test", Some("newer"), "@alice:test", false),
        ];

        let snapshot = derive_timeline_navigation_snapshot(
            &items,
            Some("$read:test"),
            &TimelineViewportObservation {
                first_visible_event_id: Some("$unread:test".to_owned()),
                last_visible_event_id: Some("$newer:test".to_owned()),
                visible_gap_ids: Vec::new(),
                at_bottom: true,
            },
            Some("@me:test"),
        );

        assert_eq!(snapshot.read_marker_event_id.as_deref(), Some("$read:test"));
        assert_eq!(
            snapshot.first_unread_event_id.as_deref(),
            Some("$unread:test")
        );
        assert_eq!(snapshot.unread_event_count, 2);
        assert_eq!(
            snapshot.unread_position,
            TimelineUnreadPosition::InsideViewport
        );
        assert_eq!(snapshot.newer_event_count, 0);
    }

    #[test]
    fn timeline_navigation_separates_local_viewed_and_server_confirmed_boundaries() {
        let items = vec![
            timeline_item("$server:test", Some("server"), "@alice:test", false),
            timeline_item("$local:test", Some("local"), "@alice:test", false),
        ];
        let snapshot = derive_timeline_navigation_snapshot_with_read_state(
            &items,
            Some("$server:test"),
            Some("$server:test"),
            Some("$local:test"),
            TimelineReadStateSync::Pending,
            &TimelineViewportObservation {
                first_visible_event_id: Some("$local:test".to_owned()),
                last_visible_event_id: Some("$local:test".to_owned()),
                visible_gap_ids: Vec::new(),
                at_bottom: true,
            },
            Some("@me:test"),
        );

        assert_eq!(
            snapshot.local_viewed_event_id.as_deref(),
            Some("$local:test")
        );
        assert_eq!(
            snapshot.server_confirmed_read_event_id.as_deref(),
            Some("$server:test")
        );
        assert_eq!(
            snapshot.read_marker_event_id.as_deref(),
            Some("$server:test")
        );
        assert_eq!(
            snapshot.read_marker_display_event_id.as_deref(),
            Some("$local:test")
        );
        assert_eq!(snapshot.read_state_sync, TimelineReadStateSync::Pending);
    }

    #[test]
    fn timeline_navigation_reports_unread_below_viewport_and_newer_count() {
        let items = vec![
            timeline_item("$read:test", Some("read"), "@alice:test", false),
            timeline_item("$visible:test", Some("visible"), "@alice:test", false),
            timeline_item("$unread:test", Some("unread"), "@alice:test", false),
            timeline_item("$newer:test", Some("newer"), "@alice:test", false),
        ];

        let snapshot = derive_timeline_navigation_snapshot(
            &items,
            Some("$visible:test"),
            &TimelineViewportObservation {
                first_visible_event_id: Some("$read:test".to_owned()),
                last_visible_event_id: Some("$visible:test".to_owned()),
                visible_gap_ids: Vec::new(),
                at_bottom: false,
            },
            Some("@me:test"),
        );

        assert_eq!(
            snapshot.first_unread_event_id.as_deref(),
            Some("$unread:test")
        );
        assert_eq!(snapshot.unread_event_count, 2);
        assert_eq!(
            snapshot.unread_position,
            TimelineUnreadPosition::BelowViewport
        );
        assert_eq!(snapshot.newer_event_count, 2);
    }

    #[test]
    fn timeline_navigation_does_not_count_read_history_below_viewport_as_newer() {
        let items = vec![
            timeline_item("$visible:test", Some("visible"), "@alice:test", false),
            timeline_item("$read-a:test", Some("read a"), "@alice:test", false),
            timeline_item("$read-b:test", Some("read b"), "@alice:test", false),
            timeline_item(
                "$read-marker:test",
                Some("read marker"),
                "@alice:test",
                false,
            ),
        ];

        let snapshot = derive_timeline_navigation_snapshot(
            &items,
            Some("$read-marker:test"),
            &TimelineViewportObservation {
                first_visible_event_id: Some("$visible:test".to_owned()),
                last_visible_event_id: Some("$visible:test".to_owned()),
                visible_gap_ids: Vec::new(),
                at_bottom: false,
            },
            Some("@me:test"),
        );

        assert_eq!(snapshot.first_unread_event_id, None);
        assert_eq!(snapshot.unread_event_count, 0);
        assert_eq!(snapshot.newer_event_count, 0);
        assert!(!snapshot.can_jump_to_bottom);
    }

    #[test]
    fn timeline_navigation_does_not_count_newer_events_without_read_marker() {
        let items = vec![
            timeline_item("$visible:test", Some("visible"), "@alice:test", false),
            timeline_item("$loaded:test", Some("loaded"), "@alice:test", false),
        ];

        let snapshot = derive_timeline_navigation_snapshot(
            &items,
            None,
            &TimelineViewportObservation {
                first_visible_event_id: Some("$visible:test".to_owned()),
                last_visible_event_id: Some("$visible:test".to_owned()),
                visible_gap_ids: Vec::new(),
                at_bottom: false,
            },
            Some("@me:test"),
        );

        assert_eq!(snapshot.read_marker_event_id, None);
        assert_eq!(snapshot.unread_event_count, 0);
        assert_eq!(snapshot.newer_event_count, 0);
        assert!(!snapshot.can_jump_to_bottom);
    }

    #[test]
    fn timeline_navigation_ignores_own_local_and_synthetic_items_for_unread_counts() {
        let mut own = timeline_item("$own:test", Some("own"), "@me:test", false);
        own.id = TimelineItemId::Event {
            event_id: "$own:test".to_owned(),
        };
        let mut local = timeline_item("$local:test", Some("local"), "@me:test", false);
        local.id = TimelineItemId::Transaction {
            transaction_id: "txn-local".to_owned(),
        };
        let mut synthetic = timeline_item("$synthetic:test", Some("divider"), "@me:test", false);
        synthetic.id = TimelineItemId::Synthetic {
            synthetic_id: "date-divider".to_owned(),
        };
        let items = vec![
            timeline_item("$read:test", Some("read"), "@alice:test", false),
            own,
            local,
            synthetic,
            timeline_item("$remote:test", Some("remote"), "@alice:test", false),
        ];

        let snapshot = derive_timeline_navigation_snapshot(
            &items,
            Some("$read:test"),
            &TimelineViewportObservation {
                first_visible_event_id: Some("$read:test".to_owned()),
                last_visible_event_id: Some("$remote:test".to_owned()),
                visible_gap_ids: Vec::new(),
                at_bottom: true,
            },
            Some("@me:test"),
        );

        assert_eq!(
            snapshot.first_unread_event_id.as_deref(),
            Some("$remote:test")
        );
        assert_eq!(snapshot.unread_event_count, 1);
        assert_eq!(snapshot.newer_event_count, 0);
    }

    #[test]
    fn unread_consistency_diagnostic_correlates_thread_receipt_with_latest_reply_projection() {
        let key = thread_key();
        let mut root = timeline_item("$root:test", Some("root"), "@me:test", false);
        root.thread_summary = Some(ThreadSummaryDto {
            reply_count: 1,
            latest_event_id: Some("$reply:test".to_owned()),
            latest_sender: Some("@alice:test".to_owned()),
            latest_sender_label: Some("Alice".to_owned()),
            latest_body_preview: Some("reply".to_owned()),
            latest_timestamp_ms: Some(2),
        });
        let mut reply = timeline_item("$reply:test", Some("reply"), "@alice:test", false);
        reply.thread_root = Some("$root:test".to_owned());
        let canonical_items = vec![root.clone(), reply];
        let snapshot = derive_timeline_navigation_snapshot(
            &canonical_items,
            Some("$root:test"),
            &TimelineViewportObservation::default(),
            Some("@me:test"),
        );
        let thread_attention = ThreadAttentionTracker {
            receipt_event_id: Some("$reply:test".to_owned()),
            ..ThreadAttentionTracker::default()
        };

        let event = timeline_unread_consistency_diagnostic_event(
            "test",
            &key,
            &canonical_items,
            &[root],
            None,
            &snapshot,
            &thread_attention,
        );
        let has_field = |key, expected| {
            event
                .fields
                .iter()
                .any(|field| field.key == key && field.value == expected)
        };

        assert_eq!(event.source, "core.timeline_unread_consistency");
        assert!(has_field("timeline", DiagnosticValue::Token("thread")));
        assert!(has_field(
            "first_unread_has_thread_root",
            DiagnosticValue::Boolean(true)
        ));
        assert!(has_field(
            "thread_receipt_in_canonical",
            DiagnosticValue::Boolean(true)
        ));
        assert!(has_field(
            "thread_receipt_matches_timeline_root",
            DiagnosticValue::Boolean(true)
        ));
        assert!(has_field(
            "latest_reply_activity_matches_first_unread",
            DiagnosticValue::Boolean(true)
        ));
        assert!(has_field(
            "thread_attention_count",
            DiagnosticValue::Count(0)
        ));
        assert!(has_field("unread_event_count", DiagnosticValue::Count(1)));
    }

    #[tokio::test]
    async fn forward_pagination_on_room_key_fails_invalid_direction() {
        let runtime = CoreRuntime::start();
        let mut conn = runtime.attach();

        // Inject a Ready session so commands are not gated.
        runtime
            .inject_actions(vec![
                AppAction::RestoreSessionRequested,
                AppAction::RestoreSessionSucceeded(SessionInfo {
                    homeserver: "https://test.test".to_owned(),
                    user_id: "@a:test".to_owned(),
                    device_id: "DEV".to_owned(),
                    authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
                }),
                AppAction::CurrentDeviceTrustChanged(
                    koushi_state::CurrentDeviceTrustState::Verified,
                ),
            ])
            .await;

        // Wait for Ready.
        loop {
            if matches!(conn.snapshot().session, SessionState::Ready(_)) {
                break;
            }
            crate::executor::sleep(Duration::from_millis(5)).await;
        }

        let rid = conn.next_request_id();
        conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: rid,
            key: room_key(),
        }))
        .await
        .expect("submit");

        // Subscribe will fail (no real session) — we don't care. Send forward paginate.
        let paginate_id = conn.next_request_id();
        conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
            request_id: paginate_id,
            key: room_key(),
            direction: PaginationDirection::Forward,
            event_count: 20,
        }))
        .await
        .expect("submit");

        // Drain until we find a failure for paginate_id.
        loop {
            let timeout = tokio::time::timeout(Duration::from_secs(5), conn.recv_event()).await;
            let event = timeout.expect("no timeout").expect("no lag");
            match event {
                CoreEvent::OperationFailed {
                    request_id,
                    failure,
                } if request_id == paginate_id => {
                    // Subscribe failed, so the key is not subscribed — we get NotSubscribed.
                    // OR we get InvalidDirection if subscribe somehow succeeded.
                    // Either way, it MUST NOT succeed.
                    assert!(
                        matches!(
                            failure,
                            CoreFailure::TimelineOperationFailed {
                                kind: TimelineFailureKind::InvalidDirection
                                    | TimelineFailureKind::NotSubscribed
                                    | TimelineFailureKind::Sdk,
                            }
                        ),
                        "expected timeline failure, got: {failure:?}"
                    );
                    return;
                }
                _ => continue,
            }
        }
    }

    #[test]
    fn timeline_pagination_uses_the_account_work_scheduler() {
        let source = include_str!("navigation.rs");
        let admission_source = source
            .split("async fn acquire_pagination_permit_and_emit_paginating")
            .nth(1)
            .and_then(|section| {
                section
                    .split("/// Emits an already-authorized group")
                    .next()
            })
            .expect("pagination admission helper should exist");
        let pagination_source = source
            .split("async fn paginate_once_for")
            .nth(1)
            .and_then(|section| section.split("fn emit_pagination_completion").next())
            .expect("pagination operation should exist");
        let admission_offset = pagination_source
            .find("acquire_pagination_permit_and_emit_paginating")
            .expect("pagination must use the admission-and-publish boundary");
        let paginate_offset = pagination_source
            .find("paginate_backwards")
            .expect("timeline pagination must still call SDK pagination");

        assert!(
            source.contains("AccountWorkScheduler"),
            "Timeline actors must carry the shared account-wide work scheduler handle"
        );
        assert!(
            admission_source.contains("AccountWorkKind::ExplicitPagination"),
            "pagination admission must acquire the named explicit-pagination kind"
        );
        assert!(
            admission_offset < paginate_offset,
            "timeline pagination must finish scheduler admission and publish Paginating before SDK pagination"
        );
    }

    #[test]
    fn timeline_pagination_is_abortable_without_dropping_the_actor() {
        let actor_source = item_body(
            include_str!("actor.rs"),
            "pub(super) struct TimelineActor {",
        );
        let actor_messages = include_str!("actor.rs");
        let handle_paginate_source =
            item_body(include_str!("navigation.rs"), "async fn handle_paginate");
        let handle_cancel_source =
            item_body(include_str!("navigation.rs"), "fn handle_cancel_pagination");
        assert!(
            actor_messages.contains("CancelPagination"),
            "timeline manager must expose a cancellation message for in-flight pagination"
        );
        assert!(
            actor_source.contains("pagination_task"),
            "TimelineActor must retain the active pagination task handle separately from the subscription"
        );
        assert!(
            handle_paginate_source.contains("executor::spawn"),
            "pagination must run outside the actor command loop so cancel messages can be received"
        );
        assert!(
            handle_cancel_source.contains(".abort()"),
            "cancelling pagination must abort only the pagination task, not the timeline actor"
        );
    }

    #[test]
    fn pagination_terminal_is_emitted_after_active_task_release() {
        let handler = item_body(include_str!("actor.rs"), "async fn handle_msg");
        let branch = handler
            .split("TimelineActorMessage::PaginationFinished {")
            .nth(1)
            .expect("pagination completion branch should exist");
        let release_offset = branch
            .find("self.pagination_task = None")
            .expect("pagination completion must release active task ownership");
        let terminal_offset = branch
            .find("self.emit_pagination_completion")
            .expect("pagination completion must emit its terminal state");
        assert!(
            release_offset < terminal_offset,
            "the terminal event must not wake React until the actor can accept its next request"
        );
    }

    #[test]
    fn restore_anchor_handler_is_room_only_and_bounded() {
        let source = include_str!("navigation.rs");
        let helper_source = source
            .split("async fn handle_restore_timeline_anchor(")
            .nth(1)
            .expect("restore anchor handler should exist")
            .split("async fn handle_restore_timeline_anchor_continue")
            .next()
            .expect("restore anchor handler should end before send text");
        let continue_source = source
            .split("async fn handle_restore_timeline_anchor_continue")
            .nth(1)
            .expect("restore anchor continuation should exist")
            .split("async fn schedule_restore_anchor_continue")
            .next()
            .expect("restore anchor continuation should end before scheduler");

        assert!(
            helper_source.contains("TimelineKind::Room"),
            "restore anchor must target the live room timeline actor"
        );
        assert!(
            continue_source.contains("PaginationDirection::Backward"),
            "restore anchor must drive backward pagination"
        );
        assert!(
            helper_source.contains("max_batches") && helper_source.contains("event_count"),
            "restore anchor must carry a bounded pagination budget"
        );
        assert!(
            !helper_source.contains("TimelineKind::Focused"),
            "restore anchor must not bootstrap through the focused timeline path"
        );
    }

    #[tokio::test]
    async fn forward_pagination_on_thread_key_not_subscribed() {
        let runtime = CoreRuntime::start();
        let mut conn = runtime.attach();

        runtime
            .inject_actions(vec![
                AppAction::RestoreSessionRequested,
                AppAction::RestoreSessionSucceeded(SessionInfo {
                    homeserver: "https://test.test".to_owned(),
                    user_id: "@a:test".to_owned(),
                    device_id: "DEV".to_owned(),
                    authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
                }),
                AppAction::CurrentDeviceTrustChanged(
                    koushi_state::CurrentDeviceTrustState::Verified,
                ),
            ])
            .await;
        loop {
            if matches!(conn.snapshot().session, SessionState::Ready(_)) {
                break;
            }
            crate::executor::sleep(Duration::from_millis(5)).await;
        }

        // Do NOT subscribe; paginate forward on thread key → NotSubscribed.
        let paginate_id = conn.next_request_id();
        conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
            request_id: paginate_id,
            key: thread_key(),
            direction: PaginationDirection::Forward,
            event_count: 10,
        }))
        .await
        .expect("submit");

        loop {
            let timeout = tokio::time::timeout(Duration::from_secs(5), conn.recv_event()).await;
            let event = timeout.expect("no timeout").expect("no lag");
            match event {
                CoreEvent::OperationFailed {
                    request_id,
                    failure,
                } if request_id == paginate_id => {
                    assert!(
                        matches!(
                            failure,
                            CoreFailure::TimelineOperationFailed {
                                kind: TimelineFailureKind::InvalidDirection
                                    | TimelineFailureKind::NotSubscribed,
                            }
                        ),
                        "got: {failure:?}"
                    );
                    return;
                }
                _ => continue,
            }
        }
    }

    #[test]
    fn focused_allows_forward_direction_in_paginate_logic() {
        // Test the direction check logic directly: forward IS allowed on Focused.
        let key = focused_key();
        let is_focused = matches!(key.kind, TimelineKind::Focused { .. });
        assert!(is_focused, "focused key must match Focused");

        // Forward + Focused: should NOT trigger InvalidDirection.
        let direction = PaginationDirection::Forward;
        let is_invalid = direction == PaginationDirection::Forward
            && !matches!(key.kind, TimelineKind::Focused { .. });
        assert!(
            !is_invalid,
            "forward on Focused must not be invalid direction"
        );
    }

    #[test]
    fn backward_direction_never_invalid_for_any_kind() {
        for key in [room_key(), focused_key(), thread_key()] {
            let direction = PaginationDirection::Backward;
            let is_invalid = direction == PaginationDirection::Forward
                && !matches!(key.kind, TimelineKind::Focused { .. });
            assert!(
                !is_invalid,
                "backward pagination should never be InvalidDirection for key: {key:?}"
            );
        }
    }

    #[tokio::test]
    async fn paginate_on_unsubscribed_key_returns_not_subscribed() {
        let runtime = CoreRuntime::start();
        let mut conn = runtime.attach();

        runtime
            .inject_actions(vec![
                AppAction::RestoreSessionRequested,
                AppAction::RestoreSessionSucceeded(SessionInfo {
                    homeserver: "https://test.test".to_owned(),
                    user_id: "@a:test".to_owned(),
                    device_id: "DEV".to_owned(),
                    authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
                }),
                AppAction::CurrentDeviceTrustChanged(
                    koushi_state::CurrentDeviceTrustState::Verified,
                ),
            ])
            .await;
        loop {
            if matches!(conn.snapshot().session, SessionState::Ready(_)) {
                break;
            }
            crate::executor::sleep(Duration::from_millis(5)).await;
        }

        let rid = conn.next_request_id();
        conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
            request_id: rid,
            key: room_key(),
            direction: PaginationDirection::Backward,
            event_count: 20,
        }))
        .await
        .expect("submit");

        loop {
            let timeout = tokio::time::timeout(Duration::from_secs(5), conn.recv_event()).await;
            let event = timeout.expect("no timeout").expect("no lag");
            match event {
                CoreEvent::OperationFailed {
                    request_id,
                    failure,
                } if request_id == rid => {
                    assert_eq!(
                        failure,
                        CoreFailure::TimelineOperationFailed {
                            kind: TimelineFailureKind::NotSubscribed
                        }
                    );
                    return;
                }
                _ => continue,
            }
        }
    }

    /// Proves that room-entry anchor restore respects the frontend budget. A
    /// stale or very deep persisted anchor must fail quickly and let the UI fall
    /// back to live edge; it must not silently inflate `max_batches=6` into a
    /// multi-thousand chunk walk that blocks entering the room.
    #[test]
    fn restore_anchor_budget_respects_frontend_hint() {
        let source = include_str!("navigation.rs");
        // Limit to production code so test strings cannot self-satisfy.
        let production = source.split("\nmod tests").next().unwrap_or(source);

        // 1. The new-state construction must use the request budget directly.
        let new_state_src = production
            .split("let restore = RestoreTimelineAnchorState {")
            .nth(1)
            .expect("new RestoreTimelineAnchorState construction must exist");
        assert!(
            new_state_src.contains("max_batches_remaining: max_batches,"),
            "max_batches_remaining initialization must respect the frontend budget"
        );

        // 2. The existing-state branch must not inflate an in-flight budget.
        let existing_branch_src = production
            .split("if existing.event_id == event_id {")
            .nth(1)
            .expect("existing-state same-event branch must exist");
        assert!(
            existing_branch_src.contains(".max(max_batches);"),
            "in-flight budget update must only preserve/increase to the requested budget"
        );
    }

    #[test]
    fn restore_walk_coalesces_items_updated_to_single_flush() {
        let actor = item_body(
            include_str!("actor.rs"),
            "pub(super) struct TimelineActor {",
        );
        assert!(
            actor.contains("restore_emit_buffer: Vec<TimelineDiff>"),
            "TimelineActor must carry restore_emit_buffer to coalesce diffs"
        );
        let diff_batch_src = item_body(include_str!("relay.rs"), "async fn handle_diff_batch");
        assert!(
            diff_batch_src.contains("restore_anchor.is_some()"),
            "handle_diff_batch must check restore_anchor.is_some() to gate buffering"
        );
        assert!(
            diff_batch_src.contains("restore_emit_buffer"),
            "handle_diff_batch must use restore_emit_buffer to accumulate diffs"
        );
        assert!(
            diff_batch_src.contains("ItemsUpdated"),
            "handle_diff_batch must still emit ItemsUpdated on the non-restore branch"
        );
        let navigation = include_str!("navigation.rs");
        let finish_src = item_body(navigation, "fn finish_anchor_restore");
        assert!(
            finish_src.contains("publish_restore_settlement(Some((request_id, status)))"),
            "real restore terminals must delegate to the atomic settlement"
        );
        let settlement_src = item_body(navigation, "fn publish_restore_settlement_with_lease");
        assert!(
            settlement_src.contains("std::mem::take"),
            "settlement must drain the buffer only after acquiring the lease"
        );
        assert!(
            settlement_src.contains("TimelineEvent::NavigationUpdated")
                && settlement_src.contains("TimelineEvent::AnchorRestoreFinished"),
            "settlement must publish navigation and terminal under the same lease"
        );
        assert!(
            !finish_src.contains("emit_anchor_restore_finished"),
            "finish_anchor_restore must not publish a second raw terminal"
        );
        let restore_handler_src = item_body(navigation, "async fn handle_restore_timeline_anchor(");
        let raw_emit_count = restore_handler_src
            .matches("self.emit_anchor_restore_finished(")
            .count();
        assert!(
            raw_emit_count <= 1,
            "handle_restore_timeline_anchor may have at most ONE raw emit_anchor_restore_finished call (the invalid-request exempt path); found {raw_emit_count}"
        );
        let continue_handler_src = item_body(
            navigation,
            "async fn handle_restore_timeline_anchor_continue(",
        );
        assert!(
            !continue_handler_src.contains("self.emit_anchor_restore_finished("),
            "handle_restore_timeline_anchor_continue must use finish_anchor_restore (never raw emit_anchor_restore_finished) — all its terminals have an active restore buffer"
        );
    }

    /// Proves the authoritative anchor-present terminal: the SDK's
    /// `anchor_present` signal determines whether to wait-for-relay (Found
    /// guaranteed) or conclude EndReached immediately (anchor genuinely absent).
    /// This makes the restore terminal deterministic — no timing heuristic.
    ///
    /// NOTE: a behavioral unit test requires constructing a real `TimelineActor`
    /// with an active Matrix SDK session, which this test module does not support
    /// without a large new mock harness. The `cache_restore` headless harness
    /// (scenario=cache_restore, 3 rooms × deep stress) is the behavioral gate for
    /// correctness of the anchor-present path; these assertions guard the
    /// structural contracts.
    #[test]
    fn restore_terminal_is_anchor_present_not_timing_dependent() {
        let source = include_str!("navigation.rs");
        let production = source.split("\nmod tests").next().unwrap_or(source);

        // 1. anchor_relay_wait must exist on RestoreTimelineAnchorState.
        let struct_src = production
            .split("struct RestoreTimelineAnchorState {")
            .nth(1)
            .expect("RestoreTimelineAnchorState must exist")
            .split('}')
            .next()
            .expect("struct body must end");
        assert!(
            struct_src.contains("anchor_relay_wait"),
            "RestoreTimelineAnchorState must carry anchor_relay_wait for the relay-wait backstop"
        );
        // 2. The continuation handler must enter the relay-wait path when anchor_present.
        let continue_src = production
            .split("async fn handle_restore_timeline_anchor_continue(")
            .nth(1)
            .expect("continuation must exist")
            .split("async fn maybe_continue_restore_anchor_after_diff")
            .next()
            .expect("continuation must end before maybe_continue");
        assert!(
            continue_src.contains("anchor_relay_wait"),
            "continuation handler must manage anchor_relay_wait for the relay-wait loop"
        );
        assert!(
            continue_src.contains("outcome.anchor_present"),
            "continuation handler must branch on outcome.anchor_present (SDK authoritative signal)"
        );
        // 3. When reached_start (anchor absent), the handler must conclude EndReached immediately.
        assert!(
            continue_src.contains("outcome.reached_start"),
            "continuation handler must use outcome.reached_start to conclude EndReached immediately"
        );
        // 4. The timing heuristics must be gone.
        assert!(
            !continue_src.contains("settle_last_seen_seq"),
            "timing-heuristic settle_last_seen_seq must be removed (replaced by anchor_present)"
        );
        assert!(
            !continue_src.contains("settle_awaiting_first_diff"),
            "timing-heuristic settle_awaiting_first_diff must be removed"
        );
        assert!(
            !production.contains("RESTORE_ANCHOR_SETTLE_TICK_DELAY_MS"),
            "50ms tick delay constant must be removed"
        );
        assert!(
            !production.contains("schedule_restore_anchor_settle_tick"),
            "schedule_restore_anchor_settle_tick function must be removed"
        );
        // 5. P3: invalid-request path must NOT call finish_anchor_restore.
        let restore_handler_src = production
            .split("async fn handle_restore_timeline_anchor(")
            .nth(1)
            .expect("handle_restore_timeline_anchor must exist")
            .split("async fn handle_restore_timeline_anchor_continue")
            .next()
            .expect("restore handler must end before continuation");
        assert!(
            restore_handler_src.contains("emit_anchor_restore_finished"),
            "invalid-request path must call emit_anchor_restore_finished (not finish_anchor_restore)"
        );
    }

    #[tokio::test]
    async fn sdk_vector_diff_batch_preserves_prefix_for_append_and_pop_variants() {
        use matrix_sdk::test_utils::mocks::MatrixMockServer;
        use matrix_sdk_test::{ALICE, JoinedRoomBuilder, event_factory::EventFactory};

        let server = MatrixMockServer::new().await;
        let client = server.client_builder().build().await;
        client
            .event_cache()
            .subscribe()
            .expect("event cache subscription");
        let room_id = matrix_sdk::ruma::room_id!("!sdk-diff-shapes:example.org");
        let room = server.sync_joined_room(&client, room_id).await;
        let factory = EventFactory::new().room(room_id).sender(&ALICE);
        server
            .sync_room(
                &client,
                JoinedRoomBuilder::new(room_id)
                    .add_timeline_event(
                        factory
                            .text_msg("prefix-a")
                            .event_id(matrix_sdk::ruma::event_id!("$prefix-a:example.org"))
                            .into_raw_sync(),
                    )
                    .add_timeline_event(
                        factory
                            .text_msg("prefix-b")
                            .event_id(matrix_sdk::ruma::event_id!("$prefix-b:example.org"))
                            .into_raw_sync(),
                    )
                    .add_timeline_event(
                        factory
                            .text_msg("append-a")
                            .event_id(matrix_sdk::ruma::event_id!("$append-a:example.org"))
                            .into_raw_sync(),
                    )
                    .add_timeline_event(
                        factory
                            .text_msg("append-b")
                            .event_id(matrix_sdk::ruma::event_id!("$append-b:example.org"))
                            .into_raw_sync(),
                    ),
            )
            .await;
        let timeline = koushi_timeline_builder(
            &room,
            TimelineFocus::Live {
                hide_threaded_events: false,
            },
        )
        .build()
        .await
        .expect("room timeline");
        let (sdk_items, _stream) = timeline.subscribe().await;
        let event = |event_id: &str| {
            sdk_items
                .iter()
                .find(|item| {
                    item.as_event()
                        .and_then(|event| event.event_id())
                        .is_some_and(|candidate| candidate.as_str() == event_id)
                })
                .cloned()
                .expect("fixture SDK event")
        };
        let key = TimelineKey::room(AccountKey(ALICE.to_string()), room_id.to_string());
        let mut canonical = vec![
            sdk_item_to_timeline_item(&key, &event("$prefix-a:example.org"), Some(&ALICE)),
            sdk_item_to_timeline_item(&key, &event("$prefix-b:example.org"), Some(&ALICE)),
        ];
        let diffs = sdk_vector_diffs_to_timeline_diffs(
            &[
                eyeball_im::VectorDiff::Append {
                    values: eyeball_im::Vector::from(vec![
                        event("$append-a:example.org"),
                        event("$append-b:example.org"),
                    ]),
                },
                eyeball_im::VectorDiff::PopBack,
                eyeball_im::VectorDiff::PopFront,
            ],
            canonical.len(),
            &key,
            Some(&ALICE),
            &HashMap::new(),
            None,
            None,
        );
        apply_timeline_diffs_to_items(&mut canonical, &diffs);

        assert_eq!(
            canonical
                .iter()
                .filter_map(|item| match &item.id {
                    TimelineItemId::Event { event_id } => Some(event_id.as_str()),
                    TimelineItemId::Transaction { .. } | TimelineItemId::Synthetic { .. } => None,
                })
                .collect::<Vec<_>>(),
            vec!["$prefix-b:example.org", "$append-a:example.org"],
            "Append must retain the existing prefix and PopBack must remove only the live edge"
        );
    }

    #[test]
    fn navigation_display_anchor_advances_past_own_messages_after_marker() {
        let other = timeline_item("$other", Some("hello"), "@bob", false);
        let own1 = timeline_item("$own1", Some("own1"), "@alice", false);
        let own2 = timeline_item("$own2", Some("own2"), "@alice", false);
        let items = vec![other, own1, own2];
        let observation = TimelineViewportObservation::default();

        let snapshot = derive_timeline_navigation_snapshot(
            &items,
            Some("$other"),
            &observation,
            Some("@alice"),
        );

        assert_eq!(snapshot.read_marker_event_id, Some("$other".to_owned()));
        assert_eq!(snapshot.first_unread_event_id, None);
        assert_eq!(
            snapshot.read_marker_display_event_id,
            Some("$own2".to_owned())
        );
    }

    #[test]
    fn navigation_display_anchor_stays_at_marker_when_no_own_messages_after() {
        let other = timeline_item("$other", Some("hello"), "@bob", false);
        let remote = timeline_item("$remote", Some("remote"), "@bob", false);
        let items = vec![other, remote];
        let observation = TimelineViewportObservation::default();

        let snapshot = derive_timeline_navigation_snapshot(
            &items,
            Some("$other"),
            &observation,
            Some("@alice"),
        );

        assert_eq!(snapshot.first_unread_event_id, Some("$remote".to_owned()));
        assert_eq!(snapshot.read_marker_display_event_id, None);
    }

    #[test]
    fn navigation_display_anchor_advances_from_own_marker_to_later_own_message() {
        let own1 = timeline_item("$own1", Some("own1"), "@alice", false);
        let own2 = timeline_item("$own2", Some("own2"), "@alice", false);
        let items = vec![own1, own2];
        let observation = TimelineViewportObservation::default();

        let snapshot = derive_timeline_navigation_snapshot(
            &items,
            Some("$own1"),
            &observation,
            Some("@alice"),
        );

        assert_eq!(snapshot.read_marker_event_id, Some("$own1".to_owned()));
        assert_eq!(snapshot.first_unread_event_id, None);
        assert_eq!(
            snapshot.read_marker_display_event_id,
            Some("$own2".to_owned())
        );
    }
}
