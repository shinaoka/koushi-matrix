//! The wake budget: a loop guard Core owns and an agent cannot change (#2155).
//!
//! The allow-list and explicit mentions stop loops between agents that behave.
//! An agent that is hijacked or runs away can mention another agent on every
//! turn and wake it without end, each wake spending LLM budget. So the push
//! subscription counts the rows it pushes per profile and, past a budget, holds
//! the rest. A held row is not dropped: it is already in the durable wake feed
//! and readable there, and the subscription's position stays in front of it, so
//! it is pushed in order once the window allows or the owner resets the budget.
//!
//! Only the owner path writes the budget (`ConsentBroker::set_wake_budget`). No
//! IPC method reads or writes it, and the pump reads it from the profile row on
//! every page.
//!
//! The ledger is in memory. A restart of Koushi clears the window, so a restart
//! can release up to one budget of held rows early. An agent cannot restart
//! Koushi, and the held rows themselves are never lost: they are unacknowledged
//! feed rows, which the next subscription resumes at.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::sync::Notify;

use crate::subject::ProfileId;

/// Default rows pushed per (account, room) per window. Derived by judgement,
/// not measured: nobody has profiled a real agent harness. Chosen so that a
/// busy human conversation (a few messages a minute) passes, while a pair of
/// agents waking each other every turn is held within minutes.
pub(crate) const ROOM_LIMIT: u32 = 30;
/// The window of [`ROOM_LIMIT`].
pub(crate) const ROOM_WINDOW: Duration = Duration::from_secs(10 * 60);
/// Default rows pushed per account across all rooms per window. Judgement, not
/// measurement: four busy rooms at the room limit, with the hour window making
/// a steady runaway visible to the owner before it costs much.
pub(crate) const ACCOUNT_LIMIT: u32 = 120;
/// The window of [`ACCOUNT_LIMIT`].
pub(crate) const ACCOUNT_WINDOW: Duration = Duration::from_secs(60 * 60);

/// A profile's budget, set by the owner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WakeBudget {
    pub room_limit: u32,
    pub room_window: Duration,
    pub account_limit: u32,
    pub account_window: Duration,
    /// Matrix user ids whose DMs are exempt, besides the non-agent accounts
    /// signed in on this host. A host with no owner account (a headless agent
    /// host) names the owner here.
    pub exempt_senders: Vec<String>,
}

impl Default for WakeBudget {
    fn default() -> Self {
        Self {
            room_limit: ROOM_LIMIT,
            room_window: ROOM_WINDOW,
            account_limit: ACCOUNT_LIMIT,
            account_window: ACCOUNT_WINDOW,
            exempt_senders: Vec::new(),
        }
    }
}

impl WakeBudget {
    /// Whether the owner may store this: a zero limit or window would hold or
    /// release everything.
    pub fn is_valid(&self) -> bool {
        self.room_limit > 0
            && self.account_limit > 0
            && !self.room_window.is_zero()
            && !self.account_window.is_zero()
    }

    pub(crate) fn to_json(&self) -> Value {
        json!({
            "room_limit": self.room_limit,
            "room_window_ms": millis(self.room_window),
            "account_limit": self.account_limit,
            "account_window_ms": millis(self.account_window),
            "exempt_senders": self.exempt_senders,
        })
    }

    /// A row that cannot be understood is the default budget, which is the
    /// stricter reading than none.
    pub(crate) fn from_json(value: &Value) -> Self {
        let number = |key: &str| value.get(key).and_then(Value::as_u64);
        let parsed = (|| {
            Some(Self {
                room_limit: u32::try_from(number("room_limit")?).ok()?,
                room_window: Duration::from_millis(number("room_window_ms")?),
                account_limit: u32::try_from(number("account_limit")?).ok()?,
                account_window: Duration::from_millis(number("account_window_ms")?),
                exempt_senders: value
                    .get("exempt_senders")
                    .and_then(Value::as_array)
                    .map(|rows| {
                        rows.iter()
                            .filter_map(|row| row.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default(),
            })
        })();
        parsed.filter(Self::is_valid).unwrap_or_default()
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Which window a held row exceeded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BudgetWindow {
    Room,
    Account,
}

impl BudgetWindow {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Room => "room",
            Self::Account => "account",
        }
    }
}

/// What the owner sees of a profile that is over budget.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct HeldView {
    pub room_id: String,
    /// `room` or `account`.
    pub window: String,
    pub since_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Held {
    pub(crate) room_id: String,
    pub(crate) window: BudgetWindow,
    pub(crate) limit: u32,
    pub(crate) window_len: Duration,
    pub(crate) retry_after: Duration,
}

/// One row offered to the budget, in feed order.
pub(crate) struct Offer<'a> {
    pub(crate) room_id: &'a str,
    /// An owner DM: never held, and not counted.
    pub(crate) exempt: bool,
}

/// What the budget decided about a page.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct Admission {
    /// The leading rows to push now; the rest are held.
    pub(crate) admitted: usize,
    /// Set when a row is held.
    pub(crate) held: Option<Held>,
    /// The caller must tell the peer: the first hold of an episode.
    pub(crate) notify: bool,
}

#[derive(Default)]
struct Ledger {
    /// Rows pushed, oldest first.
    pushed: VecDeque<(Instant, String)>,
    /// A notice was sent for the current hold.
    notified: bool,
}

impl Ledger {
    fn prune(&mut self, now: Instant, budget: &WakeBudget) {
        let longest = budget.room_window.max(budget.account_window);
        while self
            .pushed
            .front()
            .is_some_and(|(at, _)| now.duration_since(*at) >= longest)
        {
            self.pushed.pop_front();
        }
    }

    /// `Some` when one more row for `room` would exceed a window.
    fn exceeded(&self, now: Instant, budget: &WakeBudget, room: &str) -> Option<Held> {
        let room_hit = self.check(
            now,
            budget.room_window,
            budget.room_limit,
            |row| row == room,
            BudgetWindow::Room,
            room,
        );
        let account_hit = self.check(
            now,
            budget.account_window,
            budget.account_limit,
            |_| true,
            BudgetWindow::Account,
            room,
        );
        match (room_hit, account_hit) {
            (Some(a), Some(b)) => Some(if b.retry_after > a.retry_after { b } else { a }),
            (a, b) => a.or(b),
        }
    }

    fn check(
        &self,
        now: Instant,
        window: Duration,
        limit: u32,
        counts: impl Fn(&str) -> bool,
        which: BudgetWindow,
        room: &str,
    ) -> Option<Held> {
        let inside: Vec<Instant> = self
            .pushed
            .iter()
            .filter(|(at, row)| now.duration_since(*at) < window && counts(row))
            .map(|(at, _)| *at)
            .collect();
        let limit_len = limit as usize;
        if inside.len() < limit_len {
            return None;
        }
        // Admissible once enough of the oldest rows have left the window.
        let frees_at = inside[inside.len() - limit_len] + window;
        Some(Held {
            room_id: room.to_owned(),
            window: which,
            limit,
            window_len: window,
            retry_after: frees_at.saturating_duration_since(now),
        })
    }
}

/// The ledgers of every profile, and the owner's reset signal.
#[derive(Default)]
pub(crate) struct Budgets {
    ledgers: std::sync::Mutex<HashMap<ProfileId, Ledger>>,
    reset: Notify,
}

impl Budgets {
    fn ledgers(&self) -> std::sync::MutexGuard<'_, HashMap<ProfileId, Ledger>> {
        self.ledgers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Decides a page. Rows up to the last exempt one are admitted whatever the
    /// budget says: the subscription's cursor is cumulative, so a row cannot be
    /// pushed past an earlier held one, and an owner DM that reaches the agent
    /// brings the held rows before it. Every other row is admitted until the
    /// first that exceeds a window; the rest wait.
    pub(crate) fn admit(
        &self,
        profile: &ProfileId,
        budget: &WakeBudget,
        offers: &[Offer<'_>],
        now: Instant,
    ) -> Admission {
        let mut ledgers = self.ledgers();
        let ledger = ledgers.entry(profile.clone()).or_default();
        ledger.prune(now, budget);
        let forced = offers
            .iter()
            .rposition(|offer| offer.exempt)
            .map_or(0, |last| last + 1);
        let mut admitted = 0;
        let mut held = None;
        for (index, offer) in offers.iter().enumerate() {
            if !offer.exempt {
                if index >= forced
                    && let Some(hit) = ledger.exceeded(now, budget, offer.room_id)
                {
                    held = Some(hit);
                    break;
                }
                ledger.pushed.push_back((now, offer.room_id.to_owned()));
            }
            admitted += 1;
        }
        let notify = held.is_some() && !ledger.notified;
        if held.is_some() {
            ledger.notified = true;
        } else if admitted > 0 {
            ledger.notified = false;
        }
        Admission {
            admitted,
            held,
            notify,
        }
    }

    /// Whether `admit` would hold any of these rows if none were exempt.
    pub(crate) fn would_hold(
        &self,
        profile: &ProfileId,
        budget: &WakeBudget,
        offers: &[Offer<'_>],
        now: Instant,
    ) -> bool {
        let mut ledgers = self.ledgers();
        let ledger = ledgers.entry(profile.clone()).or_default();
        ledger.prune(now, budget);
        // Rows are counted as they are admitted, so a page that fits an empty
        // ledger can still overflow: replay it on a copy.
        let mut probe = Ledger {
            pushed: ledger.pushed.clone(),
            notified: true,
        };
        offers.iter().any(|offer| {
            if probe.exceeded(now, budget, offer.room_id).is_some() {
                return true;
            }
            probe.pushed.push_back((now, offer.room_id.to_owned()));
            false
        })
    }

    /// Forgets the profile's window (owner reset) and wakes a waiting pump.
    pub(crate) fn reset(&self, profile: &ProfileId) {
        self.ledgers().remove(profile);
        self.reset.notify_one();
    }

    /// The hold ended with the subscription: the next subscriber is told again.
    pub(crate) fn episode_over(&self, profile: &ProfileId) {
        if let Some(ledger) = self.ledgers().get_mut(profile) {
            ledger.notified = false;
        }
    }

    pub(crate) fn forget(&self, profile: &ProfileId) {
        self.ledgers().remove(profile);
    }

    /// Resolves when the owner resets a budget.
    pub(crate) async fn reset_requested(&self) {
        self.reset.notified().await;
    }
}

#[cfg(test)]
#[path = "wake_budget_tests.rs"]
mod tests;
