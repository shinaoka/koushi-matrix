use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel};
use koushi_state::ActivityRow;

use matrix_sdk_ui::timeline::Timeline;
use tokio::sync::{broadcast, mpsc, watch};

use crate::account_work::{AccountWorkKind, AccountWorkPermit, AccountWorkScheduler};
use crate::executor;
use crate::live_tail_freshness::LiveTailSchedulerAction;
use crate::startup_trace::{self};
use koushi_protocol::event::{
    CoreEvent, PaginationDirection, PaginationState, TimelineAnchorRestoreStatus, TimelineDiff,
    TimelineEvent, TimelineItem, TimelineItemId, TimelineNavigationSnapshot, TimelineReadStateSync,
    TimelineUnreadPosition, TimelineViewportObservation,
};
use koushi_protocol::failure::{CoreFailure, TimelineFailureKind};
use koushi_protocol::ids::{
    RequestId, TimelineBatchId, TimelineGeneration, TimelineKey, TimelineKind,
};
use koushi_sdk::MatrixLiveTailRefreshOutcome as LiveTailRefreshOutcome;

// BEGIN GENERATED SIBLING IMPORTS
use super::actor::{TimelineActor, TimelineActorControl, TimelineActorMessage};
use super::diagnostics::{
    record_live_tail_queue, record_live_tail_state, record_subscribe_stage,
    timeline_key_trace_kind, trace_timeline_items, trace_timeline_paginate,
};
use super::display_projection::DisplayProjectionState;
use super::gap_repair::{LIVE_TAIL_CANCELLATION_DEADLINE, RestoreCausalProjectionBuffer};
use super::item_projection::{
    eligible_activity_preview, has_user_visible_content, is_attention_eligible_event,
    is_unread_navigation_item, item_index_for_event_id, timeline_item_event_id,
};
use super::manager::TimelineManagerActor;
use super::thread_projection::{ThreadAttentionObservation, ThreadAttentionTracker};
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FocusedProjectionCommitted {
    pub(crate) projection_request_id: RequestId,
    pub(crate) key: TimelineKey,
    pub(crate) actor_generation: u64,
    pub(crate) timeline_generation: TimelineGeneration,
    pub(crate) item_count: u64,
    pub(crate) target_present: bool,
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
#[cfg(any(test, feature = "test-hooks"))]
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
    focused_projection_tx: Option<mpsc::UnboundedSender<FocusedProjectionCommitted>>,
}

pub(super) struct TimelineActorGenerationActivation {
    pub(super) generation: u64,
    previous_generation: Option<u64>,
}

pub(crate) struct TimelineActorGenerationLease {
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
            focused_projection_tx: None,
        }
    }
}

impl TimelineActorGenerationGate {
    pub(super) fn with_focused_projection_commits(
        focused_projection_tx: Option<mpsc::UnboundedSender<FocusedProjectionCommitted>>,
    ) -> Self {
        Self {
            focused_projection_tx,
            ..Self::default()
        }
    }

    fn publish_focused_projection_commit(&self, commit: FocusedProjectionCommitted) {
        if let Some(tx) = self.focused_projection_tx.as_ref() {
            // This internal lane is scoped to one AppActor tree. It is unbounded so
            // publishing under a synchronous generation lease can neither block nor
            // drop a commit; the sole AppActor receiver drains it for that lifetime.
            let _ = tx.send(commit);
        }
    }

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

pub(super) fn replay_projection_request_id(
    projection_request_id: RequestId,
    initial_projection_committed: bool,
) -> Option<RequestId> {
    (!initial_projection_committed).then_some(projection_request_id)
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
        initial_projection_committed: bool,
        cause_request_id: Option<RequestId>,
    ) -> Self {
        Self {
            projection_request_id: replay_projection_request_id(
                projection_request_id,
                initial_projection_committed,
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
    lease: &TimelineActorGenerationLease,
    events: Vec<TimelineEvent>,
) {
    for event in events {
        let focused_commit = match &event {
            TimelineEvent::InitialItems {
                request_id: Some(projection_request_id),
                key,
                actor_generation,
                generation,
                items,
                ..
            } if matches!(key.kind, TimelineKind::Focused { .. }) => {
                let target_present = match &key.kind {
                    TimelineKind::Focused { event_id, .. } => items
                        .iter()
                        .any(|item| timeline_item_event_id(item) == Some(event_id.as_str())),
                    _ => false,
                };
                Some(FocusedProjectionCommitted {
                    projection_request_id: *projection_request_id,
                    key: key.clone(),
                    actor_generation: *actor_generation,
                    timeline_generation: *generation,
                    item_count: items.len() as u64,
                    target_present,
                })
            }
            _ => None,
        };
        let _ = event_tx.send(CoreEvent::Timeline(event));
        if let Some(commit) = focused_commit {
            lease.gate.publish_focused_projection_commit(commit);
        }
    }
}

/// Publish one validated display-relative `ItemsUpdated` batch.
pub(super) fn emit_items_updated_for_generation(
    event_tx: &broadcast::Sender<CoreEvent>,
    timeline_actor_generations: &Arc<TimelineActorGenerationGate>,
    key: &TimelineKey,
    actor_generation: u64,
    generation: TimelineGeneration,
    batch_id: TimelineBatchId,
    diffs: Vec<TimelineDiff>,
) -> bool {
    let Some(lease) = timeline_actor_generations.try_acquire(key, actor_generation) else {
        return false;
    };
    emit_timeline_events_with_lease(
        event_tx,
        &lease,
        vec![TimelineEvent::ItemsUpdated {
            key: key.clone(),
            generation,
            batch_id,
            diffs,
        }],
    );
    true
}

/// A fresh actor projection is already display-relative. Canonical navigation
/// state remains actor-owned and is never sent through this event.
pub(super) fn emit_initial_items_for_generation(
    event_tx: &broadcast::Sender<CoreEvent>,
    timeline_actor_generations: &Arc<TimelineActorGenerationGate>,
    key: &TimelineKey,
    actor_generation: u64,
    request_identity: InitialItemsRequestIdentity,
    generation: TimelineGeneration,
    items: Vec<TimelineItem>,
    prefix_events: Vec<TimelineEvent>,
) -> bool {
    let Some(lease) = timeline_actor_generations.try_acquire(key, actor_generation) else {
        return false;
    };
    emit_timeline_events_with_lease(event_tx, &lease, prefix_events);
    emit_timeline_events_with_lease(
        event_tx,
        &lease,
        vec![TimelineEvent::InitialItems {
            request_id: request_identity.projection_request_id,
            cause_request_id: request_identity.cause_request_id,
            key: key.clone(),
            actor_generation,
            generation,
            items,
        }],
    );
    true
}

pub(super) struct RestoreSettlement {
    pub(super) navigation_snapshot: Option<TimelineNavigationSnapshot>,
    pub(super) terminal: Option<(RequestId, TimelineAnchorRestoreStatus)>,
}

pub(super) fn publish_restore_settlement_for_generation(
    restore_emit_buffer: &mut Vec<TimelineDiff>,
    force_items_updated: bool,
    next_batch_id: &mut TimelineBatchId,
    event_tx: &broadcast::Sender<CoreEvent>,
    timeline_actor_generations: &Arc<TimelineActorGenerationGate>,
    key: &TimelineKey,
    actor_generation: u64,
    generation: TimelineGeneration,
    navigation_items: &[TimelineItem],
    display_items: &[TimelineItem],
    settlement: RestoreSettlement,
) -> Option<bool> {
    let lease = timeline_actor_generations.try_acquire(key, actor_generation)?;
    let published_items = force_items_updated || !restore_emit_buffer.is_empty();
    if published_items {
        let batch_id = *next_batch_id;
        let diffs = std::mem::take(restore_emit_buffer);
        emit_timeline_events_with_lease(
            event_tx,
            &lease,
            vec![TimelineEvent::ItemsUpdated {
                key: key.clone(),
                generation,
                batch_id,
                diffs,
            }],
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
    emit_timeline_events_with_lease(event_tx, &lease, terminal_events);
    let _ = (navigation_items, display_items);
    Some(published_items)
}

pub(super) struct PreparedInitialWindow {
    pub(super) display_projection: DisplayProjectionState,
    pub(super) navigation_items: Option<Vec<TimelineItem>>,
    pub(super) emitted_items: Vec<TimelineItem>,
}

pub(super) fn commit_prepared_initial_window_for_generation(
    navigation_items: &mut Vec<TimelineItem>,
    display_projection: &mut DisplayProjectionState,
    event_tx: &broadcast::Sender<CoreEvent>,
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
    if let Some(candidate_navigation_items) = prepared.navigation_items {
        *navigation_items = candidate_navigation_items;
    }
    *display_projection = prepared.display_projection;
    emit_timeline_events_with_lease(event_tx, &lease, prefix_events);
    emit_timeline_events_with_lease(
        event_tx,
        &lease,
        vec![TimelineEvent::InitialItems {
            request_id: request_identity.projection_request_id,
            cause_request_id: request_identity.cause_request_id,
            key: key.clone(),
            actor_generation,
            generation,
            items: prepared.emitted_items,
        }],
    );
    true
}

pub(super) fn commit_prepared_initial_window_with_lease<F>(
    navigation_items: &mut Vec<TimelineItem>,
    display_projection: &mut DisplayProjectionState,
    event_tx: &broadcast::Sender<CoreEvent>,
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
    if let Some(candidate_navigation_items) = prepared.navigation_items {
        *navigation_items = candidate_navigation_items;
    }
    *display_projection = prepared.display_projection;
    commit_synchronous_candidates();
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
            items: prepared.emitted_items,
        }],
    );
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
            koushi_protocol::command::InitialBackfillPolicy::Disabled,
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
    pub(super) async fn maybe_refill_reset_room_cache(&mut self) {
        if !self.cache_reset_refill_pending
            || self.pagination_task.is_some()
            || self.gap_repair.active_serial.is_some()
            || self.gap_projection_correlation.is_pending()
            || self.pending_gap_projection.is_some()
        {
            return;
        }
        self.cache_reset_refill_pending = false;
        self.handle_paginate(
            self.projection_request_id,
            PaginationDirection::Backward,
            INITIAL_EMPTY_ROOM_BACKFILL_EVENT_COUNT,
        )
        .await;
    }

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
        self.cache_reset_refill_pending = false;
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
        let mut candidate_display_projection =
            DisplayProjectionState::from_canonical_window(&self.navigation_items, window);
        let candidate_context = self.display_projection_context();
        candidate_display_projection.reproject(&candidate_context);
        let emitted = commit_prepared_initial_window_for_generation(
            &mut self.navigation_items,
            &mut self.display_projection,
            &self.event_tx,
            &self.timeline_actor_generations,
            &self.key,
            self.actor_generation,
            InitialItemsRequestIdentity::replay(
                self.projection_request_id,
                self.initial_projection_committed,
                cause_request_id,
            ),
            self.generation,
            Vec::new(),
            PreparedInitialWindow {
                emitted_items: candidate_display_projection.display_items().to_vec(),
                display_projection: candidate_display_projection,
                navigation_items: None,
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
    /// Emit one display-relative batch through the current actor generation.
    pub(super) fn emit_items_updated(&mut self, diffs: Vec<TimelineDiff>) -> bool {
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
            true
        } else {
            false
        }
    }

    pub(super) fn emit_non_sdk_item_sets(&mut self, diffs: Vec<TimelineDiff>) -> bool {
        let batch_id = self.next_batch_id;
        let context = self.display_projection_context();
        let display_diffs =
            super::display_projection::apply_non_sdk_item_set_diffs_to_display_items(
                &mut self.display_projection,
                &diffs,
                &context,
            );
        if display_diffs.is_empty() {
            return false;
        }
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
mod tests;
