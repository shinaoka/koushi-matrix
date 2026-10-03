//! Account-wide priority scheduler for app-owned work that competes for the
//! homeserver.
//!
//! Interactive operations (sends, user-triggered room operations) must never
//! queue behind background history traffic, and background traffic must yield
//! when something more important needs the account. One policy boundary owns
//! that decision so a new caller cannot invent its own gate: callers name a
//! semantic [`AccountWorkKind`], and [`AccountWorkKind::policy`] is the only
//! place priority numbers, concurrency, and batch bounds live.
//!
//! Sync and other SDK-owned essential traffic stay outside this scheduler.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel};
use tokio::sync::Notify;

/// Diagnostics source for every scheduler stage.
const DIAGNOSTIC_SOURCE: &str = "core.account_work";

/// Account-wide ceiling on scheduled history requests in flight.
///
/// History pagination, gap repair, and crawling all page the same
/// `/rooms/{roomId}/messages` endpoint, so the account keeps one page in flight
/// regardless of per-kind policy. `max_concurrency` bounds a single kind below
/// this ceiling; it can never raise work above it.
const ACCOUNT_HISTORY_CONCURRENCY: usize = 1;

/// Semantic classification every scheduled or interactive caller submits.
///
/// Call sites name the work, never a number.
///
/// `UserRoomOperation` and `Maintenance` are reserved bands from the #306
/// policy table: they are covered by the policy tests but have no production
/// caller yet, and classifying the remaining heavy app-owned work onto them is
/// tracked as follow-up in the Phase A plan.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AccountWorkKind {
    /// Outgoing message/media send, edit, reaction, or redaction.
    MessageSend,
    /// Other user-triggered room operation.
    UserRoomOperation,
    /// Gap repair for a gap intersecting the reported viewport.
    VisibleGapRepair,
    /// Explicit or visible timeline pagination.
    ExplicitPagination,
    /// Selected-room offscreen or live-edge repair.
    OffscreenGapRepair,
    /// Search history crawling and non-visible history hydration.
    SearchCrawl,
    /// Housekeeping that may wait for an idle account.
    Maintenance,
}

/// Scheduling class. The three classes match the issue's three scheduling
/// rule sets and decide admission shape, not just ordering.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AccountWorkClass {
    /// Never queues; signals worse-priority work to yield.
    Interactive,
    /// User-visible work that may preempt background work and is itself
    /// preemptible, but is never deferred behind an interactive enqueue.
    Foreground,
    /// Bounded account-wide, yields to everything better, and waits for an
    /// active interactive enqueue instead of re-contending immediately.
    Background,
}

/// Policy resolved from a work kind. Priority is only one property: the rest
/// stays explicit so nothing is inferred from the number.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AccountWorkPolicy {
    /// Lower is more important.
    pub(crate) priority: u8,
    /// Admission shape.
    pub(crate) class: AccountWorkClass,
    /// Whether better-priority work may ask this work to yield.
    pub(crate) preemptible: bool,
    /// Ceiling on concurrent work of this kind, bounded by
    /// [`ACCOUNT_HISTORY_CONCURRENCY`].
    pub(crate) max_concurrency: u8,
    /// Events one permit may fetch before yielding and re-entering scheduling.
    pub(crate) batch_limit: u16,
}

impl AccountWorkKind {
    /// The single source of priority bands. Gaps are intentional so later work
    /// kinds can be inserted without renumbering existing ones.
    pub(crate) const fn policy(self) -> AccountWorkPolicy {
        match self {
            Self::MessageSend => AccountWorkPolicy {
                priority: 0,
                class: AccountWorkClass::Interactive,
                preemptible: false,
                max_concurrency: 1,
                batch_limit: 0,
            },
            Self::UserRoomOperation => AccountWorkPolicy {
                priority: 16,
                class: AccountWorkClass::Interactive,
                preemptible: false,
                max_concurrency: 1,
                batch_limit: 0,
            },
            Self::VisibleGapRepair => AccountWorkPolicy {
                priority: 32,
                class: AccountWorkClass::Foreground,
                preemptible: true,
                max_concurrency: 1,
                batch_limit: 64,
            },
            Self::ExplicitPagination => AccountWorkPolicy {
                priority: 40,
                class: AccountWorkClass::Foreground,
                preemptible: true,
                max_concurrency: 1,
                batch_limit: 64,
            },
            Self::OffscreenGapRepair => AccountWorkPolicy {
                priority: 96,
                class: AccountWorkClass::Background,
                preemptible: true,
                max_concurrency: 1,
                batch_limit: 64,
            },
            Self::SearchCrawl => AccountWorkPolicy {
                priority: 128,
                class: AccountWorkClass::Background,
                preemptible: true,
                max_concurrency: 1,
                batch_limit: 64,
            },
            Self::Maintenance => AccountWorkPolicy {
                priority: 192,
                class: AccountWorkClass::Background,
                preemptible: true,
                max_concurrency: 1,
                batch_limit: 32,
            },
        }
    }

    /// Private-data-free diagnostics token.
    pub(crate) const fn token(self) -> &'static str {
        match self {
            Self::MessageSend => "message_send",
            Self::UserRoomOperation => "user_room_operation",
            Self::VisibleGapRepair => "visible_gap_repair",
            Self::ExplicitPagination => "explicit_pagination",
            Self::OffscreenGapRepair => "offscreen_gap_repair",
            Self::SearchCrawl => "search_crawl",
            Self::Maintenance => "maintenance",
        }
    }

    /// Interactive work is admitted immediately and never queues.
    pub(crate) const fn is_interactive(self) -> bool {
        matches!(self.policy().class, AccountWorkClass::Interactive)
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct AccountWorkScheduler {
    inner: Arc<SchedulerInner>,
}

#[derive(Debug, Default)]
struct SchedulerInner {
    state: Mutex<SchedulerState>,
    notify: Notify,
    next_id: AtomicU64,
    next_seq: AtomicU64,
}

#[derive(Debug, Default)]
struct SchedulerState {
    active: Vec<ActiveWork>,
    waiting: Vec<WaitingWork>,
    interactive: Vec<InteractiveWork>,
}

#[derive(Debug)]
struct ActiveWork {
    id: u64,
    kind: AccountWorkKind,
    priority: u8,
    preemptible: bool,
    cancel: Arc<Notify>,
    /// Set once so a repeatedly re-evaluated queue cannot re-report preemption.
    preempted: bool,
}

#[derive(Debug)]
struct WaitingWork {
    id: u64,
    priority: u8,
    seq: u64,
}

#[derive(Debug)]
struct InteractiveWork {
    id: u64,
    priority: u8,
}

/// Held while scheduled work runs. Dropping it releases the slot and wakes
/// waiters, including on panic or timeout.
#[must_use]
pub(crate) struct AccountWorkPermit {
    inner: Arc<SchedulerInner>,
    id: u64,
    kind: AccountWorkKind,
    cancel: Arc<Notify>,
    started: Instant,
}

impl AccountWorkPermit {
    /// Resolves when better-priority work needs this slot.
    ///
    /// Cancellation is cooperative and is not a failure: finish the current
    /// bounded batch, keep the checkpoint, release the permit, and re-enter
    /// scheduling later.
    pub(crate) async fn cancelled(&self) {
        self.cancel.notified().await;
    }

    /// Report a cooperative yield before the permit is released.
    pub(crate) fn record_yield(&self, batches: u64, items: u64) {
        record_stage(
            "yielded",
            self.id,
            self.kind,
            None,
            Some(self.started.elapsed().as_millis() as u64),
            Some((batches, items)),
            None,
        );
    }
}

impl Drop for AccountWorkPermit {
    fn drop(&mut self) {
        {
            let mut state = lock_state(&self.inner);
            state.active.retain(|active| active.id != self.id);
            state.preempt_locked();
        }
        self.inner.notify.notify_waiters();
        record_stage(
            "completed",
            self.id,
            self.kind,
            None,
            Some(self.started.elapsed().as_millis() as u64),
            None,
            None,
        );
    }
}

/// Held across an interactive operation's SDK enqueue.
///
/// Entering asks worse-priority preemptible work to yield; while held, that
/// work is not re-admitted, so a yielding job cannot immediately re-contend.
/// It is intentionally scoped to enqueue, not remote settlement.
#[must_use]
pub(crate) struct InteractiveWorkGuard {
    inner: Arc<SchedulerInner>,
    id: u64,
    kind: AccountWorkKind,
    started: Instant,
}

impl Drop for InteractiveWorkGuard {
    fn drop(&mut self) {
        {
            let mut state = lock_state(&self.inner);
            state.interactive.retain(|entry| entry.id != self.id);
        }
        self.inner.notify.notify_waiters();
        record_stage(
            "completed",
            self.id,
            self.kind,
            None,
            Some(self.started.elapsed().as_millis() as u64),
            None,
            None,
        );
    }
}

impl AccountWorkScheduler {
    /// Admit interactive work immediately and ask worse-priority preemptible
    /// work to yield.
    pub(crate) fn begin_interactive(&self, kind: AccountWorkKind) -> InteractiveWorkGuard {
        debug_assert!(
            kind.is_interactive(),
            "begin_interactive requires an interactive work kind"
        );
        let policy = kind.policy();
        let id = self.next_id();
        let (better, worse) = {
            let mut state = lock_state(&self.inner);
            state.interactive.push(InteractiveWork {
                id,
                priority: policy.priority,
            });
            state.preempt_locked();
            state.active_priority_counts(policy.priority)
        };
        record_stage(
            "started",
            id,
            kind,
            Some(0),
            None,
            None,
            Some((better, worse)),
        );
        InteractiveWorkGuard {
            inner: self.inner.clone(),
            id,
            kind,
            started: Instant::now(),
        }
    }

    /// Wait for a slot for scheduled work.
    ///
    /// Admission requires a free account-wide history slot, room under the
    /// kind's `max_concurrency`, no strictly-better-priority waiter ahead, FIFO
    /// order inside one priority, and no active interactive guard outranking
    /// preemptible work.
    pub(crate) async fn acquire(&self, kind: AccountWorkKind) -> AccountWorkPermit {
        debug_assert!(
            !kind.is_interactive(),
            "interactive work must use begin_interactive so it never queues"
        );
        let policy = kind.policy();
        let id = self.next_id();
        let queued = Instant::now();
        let mut slot = Some(WaitingSlot::enter(
            self.inner.clone(),
            WaitingWork {
                id,
                priority: policy.priority,
                seq: self.next_seq(),
            },
        ));
        record_stage("queued", id, kind, None, None, None, None);

        loop {
            let notified = self.inner.notify.notified();
            {
                let mut state = lock_state(&self.inner);
                let waiting = slot
                    .as_ref()
                    .map(|slot| &slot.work)
                    .expect("waiting slot is retained until admission");
                if state.can_admit_locked(kind, policy, waiting) {
                    let cancel = Arc::new(Notify::new());
                    state.active.push(ActiveWork {
                        id,
                        kind,
                        priority: policy.priority,
                        preemptible: policy.preemptible,
                        cancel: cancel.clone(),
                        preempted: false,
                    });
                    let (better, worse) = state.active_priority_counts(policy.priority);
                    slot.take()
                        .expect("waiting slot is retained until admission")
                        .admit(&mut state);
                    drop(state);
                    record_stage(
                        "started",
                        id,
                        kind,
                        Some(queued.elapsed().as_millis() as u64),
                        None,
                        None,
                        Some((better, worse)),
                    );
                    return AccountWorkPermit {
                        inner: self.inner.clone(),
                        id,
                        kind,
                        cancel,
                        started: Instant::now(),
                    };
                }
                state.preempt_locked();
            }
            notified.await;
        }
    }

    fn next_id(&self) -> u64 {
        self.inner.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn next_seq(&self) -> u64 {
        self.inner.next_seq.fetch_add(1, Ordering::Relaxed)
    }

    #[cfg(test)]
    fn active_kinds(&self) -> Vec<AccountWorkKind> {
        lock_state(&self.inner)
            .active
            .iter()
            .map(|active| active.kind)
            .collect()
    }
}

impl SchedulerState {
    fn can_admit_locked(
        &self,
        kind: AccountWorkKind,
        policy: AccountWorkPolicy,
        waiting: &WaitingWork,
    ) -> bool {
        if self.active.len() >= ACCOUNT_HISTORY_CONCURRENCY {
            return false;
        }
        let same_kind = self
            .active
            .iter()
            .filter(|active| active.kind == kind)
            .count();
        if same_kind >= usize::from(policy.max_concurrency) {
            return false;
        }
        // A strictly better-priority waiter goes first; equal priority is FIFO.
        let outranked = self.waiting.iter().any(|other| {
            other.id != waiting.id
                && (other.priority < waiting.priority
                    || (other.priority == waiting.priority && other.seq < waiting.seq))
        });
        if outranked {
            return false;
        }
        // An interactive enqueue is short-lived. Background work waits for it
        // instead of re-contending right after yielding; foreground work is
        // user-visible too and is never deferred behind it.
        if matches!(policy.class, AccountWorkClass::Background)
            && self
                .interactive
                .iter()
                .any(|entry| entry.priority < waiting.priority)
        {
            return false;
        }
        true
    }

    /// Ask active preemptible work to yield for the best pending priority.
    fn preempt_locked(&mut self) {
        let Some(best_pending) = self.best_pending_priority() else {
            return;
        };
        for active in &mut self.active {
            if active.preemptible && active.priority > best_pending && !active.preempted {
                active.preempted = true;
                active.cancel.notify_waiters();
                active.cancel.notify_one();
                record_stage(
                    "preempted",
                    active.id,
                    active.kind,
                    None,
                    None,
                    None,
                    Some((1, 0)),
                );
            }
        }
    }

    fn best_pending_priority(&self) -> Option<u8> {
        self.waiting
            .iter()
            .map(|waiting| waiting.priority)
            .chain(self.interactive.iter().map(|entry| entry.priority))
            .min()
    }

    /// Active counts strictly better and strictly worse than `priority`.
    fn active_priority_counts(&self, priority: u8) -> (u64, u64) {
        let better = self
            .active
            .iter()
            .filter(|active| active.priority < priority)
            .count() as u64;
        let worse = self
            .active
            .iter()
            .filter(|active| active.priority > priority)
            .count() as u64;
        (better, worse)
    }
}

/// Keeps a waiter visible to the scheduler until it is admitted, and removes it
/// again if the waiting future is dropped.
struct WaitingSlot {
    inner: Option<Arc<SchedulerInner>>,
    work: WaitingWork,
}

impl WaitingSlot {
    fn enter(inner: Arc<SchedulerInner>, work: WaitingWork) -> Self {
        {
            let mut state = lock_state(&inner);
            state.waiting.push(WaitingWork {
                id: work.id,
                priority: work.priority,
                seq: work.seq,
            });
            state.preempt_locked();
        }
        inner.notify.notify_waiters();
        Self {
            inner: Some(inner),
            work,
        }
    }

    fn admit(mut self, state: &mut SchedulerState) {
        state.waiting.retain(|waiting| waiting.id != self.work.id);
        self.inner = None;
    }
}

impl Drop for WaitingSlot {
    fn drop(&mut self) {
        let Some(inner) = self.inner.take() else {
            return;
        };
        {
            let mut state = lock_state(&inner);
            state.waiting.retain(|waiting| waiting.id != self.work.id);
        }
        inner.notify.notify_waiters();
    }
}

fn lock_state(inner: &SchedulerInner) -> MutexGuard<'_, SchedulerState> {
    inner
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Emit one private-data-free scheduler diagnostic.
fn record_stage(
    stage: &'static str,
    id: u64,
    kind: AccountWorkKind,
    queue_wait_ms: Option<u64>,
    run_ms: Option<u64>,
    batches_and_items: Option<(u64, u64)>,
    better_and_worse: Option<(u64, u64)>,
) {
    let policy = kind.policy();
    let mut event = DiagnosticEvent::new(DiagnosticLevel::Info, DIAGNOSTIC_SOURCE, stage)
        .field(DiagnosticField::count("work_id", id))
        .field(DiagnosticField::token("kind", kind.token()))
        .field(DiagnosticField::count(
            "priority",
            u64::from(policy.priority),
        ))
        .field(DiagnosticField::boolean("preemptible", policy.preemptible));
    if let Some(queue_wait_ms) = queue_wait_ms {
        event = event.field(DiagnosticField::count("queue_wait_ms", queue_wait_ms));
    }
    if let Some(run_ms) = run_ms {
        event = event.field(DiagnosticField::count("run_ms", run_ms));
    }
    if let Some((batches, items)) = batches_and_items {
        event = event
            .field(DiagnosticField::count("batches", batches))
            .field(DiagnosticField::count("items", items));
    }
    if let Some((better, worse)) = better_and_worse {
        event = event
            .field(DiagnosticField::count("active_better", better))
            .field(DiagnosticField::count("active_worse", worse));
    }
    koushi_diagnostics::record(event);
}

#[cfg(test)]
mod tests;
