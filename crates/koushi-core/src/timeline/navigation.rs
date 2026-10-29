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
use crate::executor;
use crate::live_tail_freshness::LiveTailSchedulerAction;
use crate::startup_trace::{self};
use koushi_protocol::event::{
    CoreEvent, PaginationDirection, PaginationState, TimelineDiff, TimelineEvent, TimelineItem,
    TimelineItemId, TimelineNavigationSnapshot, TimelineReadStateSync, TimelineUnreadPosition,
    TimelineViewportObservation,
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
use super::gap_repair::LIVE_TAIL_CANCELLATION_DEADLINE;
use super::item_projection::{
    eligible_activity_preview, has_user_visible_content, is_attention_eligible_event,
    is_unread_navigation_item, item_index_for_event_id, timeline_item_event_id,
};
use super::manager::TimelineManagerActor;
use super::thread_projection::{ThreadAttentionObservation, ThreadAttentionTracker};
// END GENERATED SIBLING IMPORTS

pub(super) const INITIAL_EMPTY_ROOM_BACKFILL_EVENT_COUNT: u16 = 100;

/// #1125: upper bound on the guarded backward passes a fresh subscription takes
/// while its projected window still holds no displayed row. One pass is not
/// always enough, because the window can be entirely hidden technical state
/// events; the round budget keeps a hidden-event burst from paginating without
/// end.
pub(super) const INITIAL_EMPTY_ROOM_HYDRATION_MAX_ROUNDS: u8 = 8;

pub(super) const ROOM_REPLAY_INITIAL_ITEMS_MAX: usize = 120;

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

/// The desired main-pane foreground retained for TimelineManager (#1060).
///
/// `room` is the latest committed room projection; `focused` is the only
/// `TimelineKind::Focused` timeline AppActor still wants. Every other focused
/// actor is retired by the manager. AppActor admits both synchronously, in
/// program order, and always before it sends the mailbox `Subscribe` for a new
/// focused key, so a retained value can never retire a newer focused owner.
/// Cached room actors and session residency are never retired from here.
#[derive(Clone, Default)]
pub(crate) struct NavigationProjectionDemand {
    pub(crate) room: Option<NavigationProjectionIntent>,
    /// Bumped by every accepted room admission (including a same-generation
    /// merge) and never by a focused-only update, so the manager re-runs the
    /// room projection exactly when it did before #1060.
    pub(crate) room_revision: u64,
    pub(crate) focused: Option<TimelineKey>,
    /// #1465: the AppActor navigation request that owns the projection of
    /// `focused`, when a main-pane navigation is in flight for it. Only a
    /// `Subscribe` carrying this request id may take over a pending focused
    /// build; any other subscribe for the key (a renderer recovery/replay) is
    /// an observer. Cleared whenever `focused` changes without a new owner.
    pub(crate) focused_owner: Option<RequestId>,
}

impl NavigationProjectionDemand {
    /// Whether `request_id` is the admitted navigation owner of `key`.
    pub(crate) fn owns_focused(&self, key: &TimelineKey, request_id: RequestId) -> bool {
        self.focused.as_ref() == Some(key) && self.focused_owner == Some(request_id)
    }

    fn replace_focused(&mut self, focused: Option<TimelineKey>) {
        if self.focused != focused {
            self.focused_owner = None;
        }
        self.focused = focused;
    }
}

/// Stable latest-wins ingress shared across session-scoped timeline-manager
/// replacement. A watch channel is a one-slot value plus a coalesced wake:
/// replacing a value cannot fill or block the AppActor/AccountActor mailbox.
#[derive(Clone)]
pub(crate) struct NavigationProjectionIngress {
    tx: watch::Sender<NavigationProjectionDemand>,
}

impl NavigationProjectionIngress {
    pub(crate) fn channel() -> (Self, watch::Receiver<NavigationProjectionDemand>) {
        let (tx, rx) = watch::channel(NavigationProjectionDemand::default());
        (Self { tx }, rx)
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<NavigationProjectionDemand> {
        let receiver = self.tx.subscribe();
        // A replacement manager must observe the retained latest desired
        // projection even when it subscribed after that value was admitted.
        self.tx.send_modify(|_| {});
        receiver
    }

    /// Admit a committed room projection together with the focused
    /// foreground desired at that commit. A stale room part is ignored, but
    /// the focused part is always the caller's current desired value.
    pub(crate) fn admit(
        &self,
        intent: NavigationProjectionIntent,
        focused: Option<TimelineKey>,
    ) -> bool {
        // `send_modify` retains the value even during the brief interval in
        // which a session-scoped manager is being replaced and no receiver
        // exists. A later `subscribe` explicitly wakes on that retained value.
        self.tx.send_modify(|demand| {
            let accepted = match demand.room.take() {
                Some(current) if current.generation > intent.generation => {
                    demand.room = Some(current);
                    false
                }
                Some(mut current)
                    if current.generation == intent.generation && current.key == intent.key =>
                {
                    current.replay_existing |= intent.replay_existing;
                    demand.room = Some(current);
                    true
                }
                _ => {
                    demand.room = Some(intent);
                    true
                }
            };
            if accepted {
                demand.room_revision = demand.room_revision.wrapping_add(1);
            }
            demand.replace_focused(focused);
        });
        true
    }

    /// Replace only the desired focused foreground, carrying the retained
    /// room projection forward so an unpolled room subscription survives.
    /// The focused key has no navigation owner.
    #[cfg(test)]
    pub(crate) fn admit_focused(&self, focused: Option<TimelineKey>) {
        self.admit_focused_with_owner(focused, None);
    }

    /// #1465: replace the desired focused foreground together with the
    /// navigation request that owns its projection. `owner` is ignored when
    /// there is no focused key.
    pub(crate) fn admit_focused_with_owner(
        &self,
        focused: Option<TimelineKey>,
        owner: Option<RequestId>,
    ) {
        self.tx.send_modify(|demand| {
            demand.focused_owner = focused.as_ref().and(owner);
            demand.focused = focused;
        });
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
        cause_request_id: Option<RequestId>,
    ) -> Self {
        Self {
            // This is the actor-owned view-source identity. The command that
            // requested this replay is correlated separately below.
            projection_request_id: Some(projection_request_id),
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
) -> bool {
    let mut published = true;
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
        published &= event_tx.send(CoreEvent::Timeline(event)).is_ok();
        if let Some(commit) = focused_commit {
            lease.gate.publish_focused_projection_commit(commit);
        }
    }
    published
}

/// Generation acceptance advances actor state even without broadcast receivers.
/// Publication diagnostics must use the separate successful-send outcome.
#[derive(Default)]
pub(super) struct TimelineEmissionOutcome {
    pub(super) accepted: bool,
    pub(super) published: bool,
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
) -> TimelineEmissionOutcome {
    let Some(lease) = timeline_actor_generations.try_acquire(key, actor_generation) else {
        return TimelineEmissionOutcome::default();
    };
    let published = emit_timeline_events_with_lease(
        event_tx,
        &lease,
        vec![TimelineEvent::ItemsUpdated {
            key: key.clone(),
            generation,
            batch_id,
            diffs,
        }],
    );
    TimelineEmissionOutcome {
        accepted: true,
        published,
    }
}

/// A fresh actor projection is already display-relative. Canonical navigation
/// state remains actor-owned and is never sent through this event.
#[expect(
    clippy::too_many_arguments,
    reason = "pre-existing internal signature; grouping its inputs is a separate refactor"
)]
pub(super) fn emit_initial_items_for_generation(
    event_tx: &broadcast::Sender<CoreEvent>,
    timeline_actor_generations: &Arc<TimelineActorGenerationGate>,
    key: &TimelineKey,
    actor_generation: u64,
    request_identity: InitialItemsRequestIdentity,
    generation: TimelineGeneration,
    items: Vec<TimelineItem>,
    prefix_events: Vec<TimelineEvent>,
) -> TimelineEmissionOutcome {
    let Some(lease) = timeline_actor_generations.try_acquire(key, actor_generation) else {
        return TimelineEmissionOutcome::default();
    };
    emit_timeline_events_with_lease(event_tx, &lease, prefix_events);
    let published = emit_timeline_events_with_lease(
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
    TimelineEmissionOutcome {
        accepted: true,
        published,
    }
}

pub(super) struct PreparedInitialWindow {
    pub(super) display_projection: DisplayProjectionState,
    pub(super) navigation_items: Option<Vec<TimelineItem>>,
    pub(super) emitted_items: Vec<TimelineItem>,
}

#[expect(
    clippy::too_many_arguments,
    reason = "pre-existing internal signature; grouping its inputs is a separate refactor"
)]
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
    suffix_events: Vec<TimelineEvent>,
) -> TimelineEmissionOutcome {
    let Some(lease) = timeline_actor_generations.try_acquire(key, actor_generation) else {
        return TimelineEmissionOutcome::default();
    };
    if let Some(candidate_navigation_items) = prepared.navigation_items {
        *navigation_items = candidate_navigation_items;
    }
    *display_projection = prepared.display_projection;
    emit_timeline_events_with_lease(event_tx, &lease, prefix_events);
    let published = emit_timeline_events_with_lease(
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
    emit_timeline_events_with_lease(event_tx, &lease, suffix_events);
    TimelineEmissionOutcome {
        accepted: true,
        published,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "pre-existing internal signature; grouping its inputs is a separate refactor"
)]
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
    receiver: &mut Option<watch::Receiver<NavigationProjectionDemand>>,
) -> Option<NavigationProjectionDemand> {
    let Some(active) = receiver.as_mut() else {
        return futures_util::future::pending().await;
    };
    if active.changed().await.is_err() {
        *receiver = None;
        return None;
    }
    Some(active.borrow_and_update().clone())
}

impl TimelineManagerActor {
    pub(super) async fn handle_navigation_projection(
        &mut self,
        demand: super::NavigationProjectionDemand,
    ) {
        // Retire before (re)projecting the room so obsolete focused work can
        // never anchor over the committed foreground.
        self.retire_undesired_focused_timelines(demand.focused.as_ref())
            .await;
        let Some(intent) = demand.room else {
            return;
        };
        if intent.generation < self.last_navigation_projection_generation {
            return;
        }
        // A focused-only update carries the retained room part forward without
        // bumping `room_revision`; skip only that case. Every room admission,
        // including a same-(generation, key) re-admission (SubscribeTimeline
        // for an unchanged `active_room_id` whose `timeline.room_id` differed),
        // still re-runs the committed room selection as before #1060.
        if self.applied_room_projection == Some(demand.room_revision) {
            return;
        }
        self.applied_room_projection = Some(demand.room_revision);
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
        Box::pin(self.handle_committed_room_selection(
            intent.cause_request_id,
            intent.key,
            intent.replay_existing,
            false,
        ))
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
        if let Some(previous) = previous_foreground
            && let Some(handle) = self.timelines.get(&previous)
        {
            // Generation invalidation above makes late old-room work inert;
            // cleanup is best-effort and must never hold the new room.
            handle.end_gap_repair_demand();
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

        Box::pin(self.handle_subscribe(
            request_id,
            key.clone(),
            replay_existing,
            emit_failure_terminal,
            koushi_protocol::command::InitialBackfillPolicy::Disabled,
        ))
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PaginationCompletion {
    state: PaginationState,
    prepend_expected: Option<bool>,
    /// #1676: the SDK's final row after a `Focused` forward page reached the
    /// end and the SDK timeline applied it.
    pub(super) focused_end_tail: Option<String>,
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

        if matches!(self.key.kind, TimelineKind::Room { .. })
            && direction == PaginationDirection::Backward
            && self
                .display_projection
                .first_visible_canonical_index()
                .is_some_and(|i| i > 0)
        {
            let context = self.display_projection_context();
            // Pagination is explicit renderer demand, not a stale viewport event.
            // Rebase surviving endpoint identities onto the current publication.
            let mut demand = self.viewport_observation.clone();
            demand.display_window_revision = Some(self.display_projection.window_revision);
            if demand.first_visible_event_id.is_none() || demand.last_visible_event_id.is_none() {
                let metadata = self
                    .display_projection
                    .window_metadata(self.navigation_items.len());
                demand.first_visible_event_id = metadata.first_event_id.clone();
                demand.last_visible_event_id = metadata.first_event_id;
            }
            let diffs = self.display_projection.reconcile_bounded_window(
                &self.navigation_items,
                &demand,
                &context,
                Some(direction),
            );
            self.emit_navigation_if_changed();
            if !diffs.is_empty() {
                self.emit_items_updated(diffs);
            }
            self.emit(CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                request_id: Some(request_id),
                key: self.key.clone(),
                direction,
                state: PaginationState::Idle,
                prepend_expected: Some(false),
            }));
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
        let focused_cache = self.focused_cache.clone();
        let task = executor::spawn(async move {
            let completion = Box::pin(Self::paginate_once_for(
                request_id,
                key,
                timeline,
                event_tx,
                timeline_actor_generations,
                actor_generation,
                account_work,
                direction,
                event_count,
                focused_cache,
            ))
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
    #[expect(
        clippy::too_many_arguments,
        reason = "pre-existing internal signature; grouping its inputs is a separate refactor"
    )]
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
        focused_cache: Option<matrix_sdk::event_cache::EventFocusedCache>,
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
                focused_end_tail: None,
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
            // #1239: bound the SDK await so a stalled relationship request can
            // never leave the pane in `Paginating` for the life of the timeline.
            let outcome = Box::pin(bounded_paginate_with(TIMELINE_PAGINATION_TIMEOUT, async {
                match direction {
                    PaginationDirection::Backward => timeline.paginate_backwards(event_count).await,
                    PaginationDirection::Forward => timeline.paginate_forwards(event_count).await,
                }
            }))
            .await;
            trace_timeline_paginate(
                "sdk_finish",
                request_id,
                &key,
                direction,
                event_count,
                trace_started.map(|started| started.elapsed().as_millis()),
                gate_ms,
                Some(outcome.trace_token()),
            );
            startup_trace::trace_paginate(
                paginate_started,
                gate_wait,
                matches!(&outcome, PaginationAttempt::ReachedEnd),
            );
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

        let focused_end_tail = match (&result, focused_cache) {
            (PaginationAttempt::ReachedEnd, Some(cache))
                if direction == PaginationDirection::Forward =>
            {
                super::focused_live_tail::final_tail(&timeline, &cache, TIMELINE_PAGINATION_TIMEOUT)
                    .await
            }
            _ => None,
        };
        let next_state = result.into_state();

        PaginationCompletion {
            state: next_state,
            prepend_expected,
            focused_end_tail,
        }
    }
    /// #1676: report the live tail once the projection holds the pending
    /// final row. Only the completion path calls this; SDK batches and
    /// replays carry the signal under their own projection lease.
    pub(super) fn emit_focused_live_tail_if_committed(&mut self) {
        let Some(tail) = self.focused_live_tail.commit(&self.navigation_items) else {
            return;
        };
        emit_timeline_events_for_generation(
            &self.event_tx,
            &self.timeline_actor_generations,
            &self.key,
            self.actor_generation,
            vec![super::focused_live_tail::live_tail_event(
                &self.key,
                self.actor_generation,
                self.generation,
                tail,
            )],
        );
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
    /// Re-emit `navigation_items` as `InitialItems` without touching the SDK
    /// subscription or tearing down the actor. Idempotent Subscribe supplies
    /// an exact cause; internal replay recovery does not. The projection ACK
    /// identity remains owned by the actor in both cases.
    pub(super) fn handle_replay_initial_items(&mut self, cause_request_id: Option<RequestId>) {
        let window = if matches!(self.key.kind, TimelineKind::Room { .. }) {
            self.display_projection
                .canonical_window_range(self.navigation_items.len())
        } else {
            replay_initial_items_window_range(
                &self.key.kind,
                &self.navigation_items,
                &self.viewport_observation,
            )
        };
        let items = if matches!(self.key.kind, TimelineKind::Room { .. }) {
            self.display_projection.display_items().to_vec()
        } else {
            self.navigation_items[window.clone()].to_vec()
        };
        let item_count = items.len();
        trace_timeline_items("replay_initial", &self.key, &items);
        let mut candidate_display_projection = self.display_projection.clone();
        let candidate_context = self.display_projection_context();
        // Replay preserves manager-owned pending rows and suppressed late SDK
        // echoes, including when the previous broadcast had no receivers.
        if matches!(self.key.kind, TimelineKind::Room { .. }) {
            candidate_display_projection.reproject(&candidate_context);
        } else {
            candidate_display_projection.rematerialize_window(
                &self.navigation_items,
                window,
                &candidate_context,
            );
        }
        let emitted = commit_prepared_initial_window_for_generation(
            &mut self.navigation_items,
            &mut self.display_projection,
            &self.event_tx,
            &self.timeline_actor_generations,
            &self.key,
            self.actor_generation,
            InitialItemsRequestIdentity::replay(self.projection_request_id, cause_request_id),
            self.generation,
            Vec::new(),
            PreparedInitialWindow {
                emitted_items: candidate_display_projection.display_items().to_vec(),
                display_projection: candidate_display_projection,
                navigation_items: None,
            },
            // #1676: a replay may reach a renderer that lost the latch.
            self.focused_live_tail
                .latched()
                .map(|tail| {
                    super::focused_live_tail::live_tail_event(
                        &self.key,
                        self.actor_generation,
                        self.generation,
                        tail.to_owned(),
                    )
                })
                .into_iter()
                .collect(),
        );
        if emitted.published {
            super::outbound_send::record_initial_pending_publication(
                &self.pending_send_projections,
                self.display_projection.display_items(),
            );
        }
        if emitted.accepted {
            let _ = self.thread_attention.reconcile(
                &self.key,
                &self.navigation_items,
                self.own_user_id.as_ref().map(|user_id| user_id.as_str()),
                ThreadAttentionObservation::Replay,
            );
            // A replay may serve a new subscriber that never received the last
            // navigation snapshot. Incremental change suppression does not apply.
            self.last_navigation_snapshot = None;
            self.emit_navigation_if_changed();
        }
        record_subscribe_stage(
            if emitted.accepted {
                "replay_initial_emitted"
            } else {
                "replay_initial_rejected_stale_generation"
            },
            Some(item_count),
        );
    }
    fn current_navigation_snapshot(&self) -> TimelineNavigationSnapshot {
        let mut snapshot = derive_timeline_navigation_snapshot_with_read_state(
            &self.key.kind,
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
        align_first_unread_display_to_display_rows(
            &self.key.kind,
            &self.navigation_items,
            self.display_projection.display_items(),
            self.own_user_id.as_ref().map(|user_id| user_id.as_str()),
            &mut snapshot,
        );
        align_read_marker_display_to_display_rows(
            &self.key.kind,
            &self.navigation_items,
            self.display_projection.display_items(),
            &mut snapshot,
        );
        if matches!(self.key.kind, TimelineKind::Room { .. }) {
            snapshot.display_window = Some(
                self.display_projection
                    .window_metadata(self.navigation_items.len()),
            );
        }
        snapshot
    }
    pub(super) fn emit_navigation_if_changed(&mut self) {
        let snapshot = self.current_navigation_snapshot();
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
            snapshot: Box::new(snapshot),
        }));
    }
    /// Emit one display-relative batch through the current actor generation.
    pub(super) fn emit_items_updated(&mut self, diffs: Vec<TimelineDiff>) -> bool {
        let traces = super::outbound_send::pending_publication_traces(
            &self.pending_send_projections,
            &diffs,
        );
        let batch_id = self.next_batch_id;
        let outcome = super::navigation::emit_items_updated_for_generation(
            &self.event_tx,
            &self.timeline_actor_generations,
            &self.key,
            self.actor_generation,
            self.generation,
            batch_id,
            diffs,
        );
        if outcome.published {
            super::outbound_send::record_pending_publication(traces);
        }
        if outcome.accepted {
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
        )
        .accepted
        {
            self.next_batch_id = TimelineBatchId(batch_id.0 + 1);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
pub(super) fn replay_initial_items_window(
    kind: &TimelineKind,
    items: &[TimelineItem],
    observation: &TimelineViewportObservation,
) -> Vec<TimelineItem> {
    items[replay_initial_items_window_range(kind, items, observation)].to_vec()
}

/// The same live-edge rule the display projection applies, so a replay in a test
/// cannot pass while the projection has moved on.
fn replay_initial_items_window_range(
    kind: &TimelineKind,
    items: &[TimelineItem],
    observation: &TimelineViewportObservation,
) -> std::ops::Range<usize> {
    let start = if matches!(kind, TimelineKind::Room { .. }) && observation.at_bottom {
        super::display_projection::live_edge_window_start(items, ROOM_REPLAY_INITIAL_ITEMS_MAX)
    } else {
        0
    };
    start..items.len()
}

/// #1110: a room timeline with no *displayed* row needs the ordinary guarded
/// backfill, not only a literally empty one.
///
/// A snapshot can be nonempty while every item is a suppressed technical state
/// event, so counting raw items left the reader with an empty conversation and
/// no path to the displayable history behind it.
pub(super) fn should_hydrate_empty_initial_room_timeline(
    kind: &TimelineKind,
    items: &[TimelineItem],
) -> bool {
    matches!(kind, TimelineKind::Room { .. })
        && !items
            .iter()
            .any(super::display_projection::item_occupies_display_row)
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
    kind: &TimelineKind,
    items: &[TimelineItem],
    fully_read_event_id: Option<&str>,
    observation: &TimelineViewportObservation,
    own_user_id: Option<&str>,
) -> TimelineNavigationSnapshot {
    derive_timeline_navigation_snapshot_with_read_state(
        kind,
        items,
        fully_read_event_id,
        fully_read_event_id,
        None,
        TimelineReadStateSync::Synced,
        observation,
        own_user_id,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "pre-existing internal signature; grouping its inputs is a separate refactor"
)]
pub(super) fn derive_timeline_navigation_snapshot_with_read_state(
    kind: &TimelineKind,
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
    let local_position = local_viewed_event_id
        .as_deref()
        .and_then(|event_id| item_index_for_event_id(items, event_id));
    let confirmed_position = server_confirmed_read_event_id
        .as_deref()
        .and_then(|event_id| item_index_for_event_id(items, event_id));
    // Resolve all loaded boundaries once, before unread processing can return.
    // Hidden edits and out-of-scope replies are ordering facts, not divider rows.
    let display_marker = local_position.max(confirmed_position).and_then(|position| {
        items[..=position]
            .iter()
            .rev()
            .find(|item| {
                !item.is_hidden
                    && navigation_item_in_scope(kind, item)
                    && timeline_item_event_id(item).is_some()
            })
            .and_then(timeline_item_event_id)
            .map(ToOwned::to_owned)
    });
    let mut snapshot = TimelineNavigationSnapshot {
        display_window: None,
        read_marker_event_id: server_confirmed_read_event_id.clone(),
        read_marker_display_event_id: display_marker,
        first_unread_event_id: None,
        first_unread_display_event_id: None,
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
        newer_unread_event_count(kind, items, observation, own_user_id, read_marker_index);
    snapshot.can_jump_to_bottom = snapshot.newer_event_count > 0;

    let unread_items: Vec<(usize, &TimelineItem)> = items
        .iter()
        .enumerate()
        .skip(read_marker_index.saturating_add(1))
        .filter(|(_, item)| {
            navigation_item_in_scope(kind, item) && is_unread_navigation_item(item, own_user_id)
        })
        .collect();

    snapshot.unread_event_count = unread_items.len() as u64;
    if let Some((first_unread_index, first_unread)) = unread_items.first() {
        snapshot.first_unread_event_id =
            timeline_item_event_id(first_unread).map(ToOwned::to_owned);
        snapshot.first_unread_display_event_id = snapshot.first_unread_event_id.clone();
        snapshot.unread_position =
            unread_position_for_index(items, *first_unread_index, observation);
        return snapshot;
    }

    // No remote unread events after the marker. Advance the display anchor to the
    // current user's latest visible own message at or after the marker so the
    // "Read up to here" separator is rendered after it, not before.
    if local_position.is_none()
        && let Some(own_event_id) = items
            .iter()
            .skip(read_marker_index)
            .rfind(|item| {
                navigation_item_in_scope(kind, item) && is_own_visible_event(item, own_user_id)
            })
            .and_then(timeline_item_event_id)
    {
        snapshot.read_marker_display_event_id = Some(own_event_id.to_owned());
    }
    snapshot
}

/// Key the Room "Read up to here" divider to a displayed row.
///
/// Latest-reply ordering shows a thread root at its newest reply's position, so
/// a root the canonical boundary already covers can render below the boundary
/// row. The divider follows the last displayed row whose content is at or
/// before the boundary, named by that row's activity identity — the identity
/// React matches. Thread replies stay out of Room unread state; while main
/// conversation content is unread, the unread divider owns placement.
pub(super) fn align_read_marker_display_to_display_rows(
    kind: &TimelineKind,
    navigation_items: &[TimelineItem],
    display_items: &[TimelineItem],
    snapshot: &mut TimelineNavigationSnapshot,
) {
    if !matches!(kind, TimelineKind::Room { .. }) || snapshot.first_unread_event_id.is_some() {
        return;
    }
    let Some(boundary) = snapshot
        .read_marker_display_event_id
        .as_deref()
        .or(snapshot.read_marker_event_id.as_deref())
    else {
        return;
    };
    let canonical_positions = navigation_items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| timeline_item_event_id(item).map(|event_id| (event_id, index)))
        .collect::<HashMap<_, _>>();
    let Some(&boundary_index) = canonical_positions.get(boundary) else {
        return;
    };
    let anchor = display_items.iter().rev().find_map(|row| {
        let metadata = row.display_metadata.as_ref()?;
        let content_index = canonical_positions.get(metadata.content_event_id.as_deref()?)?;
        (*content_index <= boundary_index)
            .then_some(metadata.activity_event_id.as_deref())
            .flatten()
    });
    if let Some(anchor) = anchor
        && anchor != boundary
    {
        snapshot.read_marker_display_event_id = Some(anchor.to_owned());
    }
}

/// Key the Room "New messages" divider to a displayed row.
///
/// `first_unread_event_id` stays canonical for counts and diagnostics. Latest-
/// reply ordering shows an unread thread root at its newest reply's position,
/// possibly after other unread rows, so the divider precedes the first
/// displayed row whose content is unread, named by that row's activity
/// identity — the identity React matches.
pub(super) fn align_first_unread_display_to_display_rows(
    kind: &TimelineKind,
    navigation_items: &[TimelineItem],
    display_items: &[TimelineItem],
    own_user_id: Option<&str>,
    snapshot: &mut TimelineNavigationSnapshot,
) {
    if !matches!(kind, TimelineKind::Room { .. }) || snapshot.first_unread_event_id.is_none() {
        return;
    }
    let Some(boundary_index) = snapshot
        .read_marker_event_id
        .as_deref()
        .and_then(|event_id| item_index_for_event_id(navigation_items, event_id))
    else {
        return;
    };
    let unread_content = navigation_items
        .iter()
        .skip(boundary_index.saturating_add(1))
        .filter(|item| {
            navigation_item_in_scope(kind, item) && is_unread_navigation_item(item, own_user_id)
        })
        .filter_map(timeline_item_event_id)
        .collect::<HashSet<_>>();
    let anchor = display_items.iter().find_map(|row| {
        let metadata = row.display_metadata.as_ref()?;
        unread_content
            .contains(metadata.content_event_id.as_deref()?)
            .then_some(metadata.activity_event_id.as_deref())
            .flatten()
    });
    if let Some(anchor) = anchor {
        snapshot.first_unread_display_event_id = Some(anchor.to_owned());
    }
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
    let local_position = snapshot
        .local_viewed_event_id
        .as_deref()
        .and_then(event_position);
    let confirmed_position = snapshot
        .server_confirmed_read_event_id
        .as_deref()
        .and_then(event_position);
    let marker_display_position = snapshot
        .read_marker_display_event_id
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
    .field(DiagnosticField::boolean("local_viewed_in_canonical", local_position.is_some()))
    .field(DiagnosticField::boolean("confirmed_in_canonical", confirmed_position.is_some()))
    .field(DiagnosticField::boolean("display_marker_in_canonical", marker_display_position.is_some()))
    .field(DiagnosticField::boolean("local_before_confirmed",
        matches!((local_position, confirmed_position), (Some(local), Some(confirmed)) if local < confirmed)))
    .field(DiagnosticField::boolean("display_before_confirmed",
        matches!((marker_display_position, confirmed_position), (Some(display), Some(confirmed)) if display < confirmed)))
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
    if own_user_id.is_none_or(|own| item.sender.as_deref() != Some(own)) {
        return false;
    }
    matches!(item.id, TimelineItemId::Event { .. })
}

// Preserve canonical indices for hidden receipt targets, but do not count
// thread replies as main-conversation unread events.
fn navigation_item_in_scope(kind: &TimelineKind, item: &TimelineItem) -> bool {
    !matches!(kind, TimelineKind::Room { .. }) || item.thread_root.is_none()
}

fn newer_unread_event_count(
    kind: &TimelineKind,
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
        .filter(|item| {
            navigation_item_in_scope(kind, item) && is_unread_navigation_item(item, own_user_id)
        })
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

/// #1239: the upper bound for one SDK pagination. A thread backfill must reach a
/// terminal state (Idle / EndReached / Failed) within this window even when the
/// SDK request never returns; the timeout surfaces as a retryable failure.
const TIMELINE_PAGINATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// One bounded SDK pagination outcome (#1239).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PaginationAttempt {
    ReachedEnd,
    Idle,
    Failed(TimelineFailureKind),
}

impl PaginationAttempt {
    /// A terminal-state token for the pagination diagnostics.
    fn trace_token(self) -> &'static str {
        match self {
            PaginationAttempt::ReachedEnd => "end_reached",
            PaginationAttempt::Idle => "idle",
            PaginationAttempt::Failed(TimelineFailureKind::Timeout) => "timeout",
            PaginationAttempt::Failed(_) => "failed",
        }
    }

    fn is_ok(self) -> bool {
        !matches!(self, PaginationAttempt::Failed(_))
    }

    fn into_state(self) -> PaginationState {
        match self {
            PaginationAttempt::ReachedEnd => PaginationState::EndReached,
            PaginationAttempt::Idle => PaginationState::Idle,
            PaginationAttempt::Failed(kind) => PaginationState::Failed { kind },
        }
    }
}

/// The timeout/classification half of one bounded pagination (#1239). The SDK
/// call itself stays at the `paginate_once_for` call site, after scheduler
/// admission; this wraps it in the deadline and classifies the outcome.
async fn bounded_paginate_with<F>(deadline: std::time::Duration, paginate: F) -> PaginationAttempt
where
    F: std::future::Future<Output = Result<bool, matrix_sdk_ui::timeline::Error>>,
{
    match executor::timeout(deadline, paginate).await {
        Ok(Ok(true)) => PaginationAttempt::ReachedEnd,
        Ok(Ok(false)) => PaginationAttempt::Idle,
        Ok(Err(err)) => PaginationAttempt::Failed(classify_pagination_error(&err)),
        Err(_) => PaginationAttempt::Failed(TimelineFailureKind::Timeout),
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

#[cfg(test)]
#[path = "navigation/read_marker_reentry_tests.rs"]
mod read_marker_reentry_tests;

#[cfg(test)]
#[path = "navigation/replay_identity_tests.rs"]
mod replay_identity_tests;
