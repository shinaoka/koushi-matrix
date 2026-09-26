//! #1009: private-safe, cumulative summary of current-session verification
//! checks. The reducer owns the full-inspection decisions; the AccountActor
//! owns the trust-only recheck path. Both are summarized here as counters,
//! fixed tokens, and coarse time buckets so a frequent check path stays
//! traceable after the bounded diagnostic ring has discarded its records.
//! No identifiers, keys, or tokens enter this lane.

use std::sync::Mutex;

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_state::{
    AppState, CurrentSessionStatusState, SessionStatusCheckDecision, SessionStatusCheckStats,
    SessionStatusRefreshTrigger,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TrustRecheckStats {
    pub started: u64,
    /// Joined an in-flight full inspection instead of querying.
    pub joined: u64,
    /// Held behind the failure backoff.
    pub deferred: u64,
    pub succeeded: u64,
    pub failed: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SessionCheckSummary {
    pub inspection: SessionStatusCheckStats,
    pub last_trigger: Option<SessionStatusRefreshTrigger>,
    pub last_decision: Option<SessionStatusCheckDecision>,
    pub last_success_at_ms: Option<u64>,
    pub next_due_at_ms: Option<u64>,
    pub trust: TrustRecheckStats,
}

static SUMMARY: Mutex<SessionCheckSummary> = Mutex::new(SessionCheckSummary {
    inspection: SessionStatusCheckStats {
        started: 0,
        joined: 0,
        deferred: 0,
        dropped: 0,
        succeeded: 0,
        failed: 0,
    },
    last_trigger: None,
    last_decision: None,
    last_success_at_ms: None,
    next_due_at_ms: None,
    trust: TrustRecheckStats {
        started: 0,
        joined: 0,
        deferred: 0,
        succeeded: 0,
        failed: 0,
    },
});

fn with_summary<R>(update: impl FnOnce(&mut SessionCheckSummary) -> R) -> R {
    let mut summary = SUMMARY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    update(&mut summary)
}

/// Mirror the reducer-owned schedule after a reduction.
pub(crate) fn observe_schedule(state: &AppState) {
    let schedule = &state.current_session_status_schedule;
    let last_success_at_ms = match &state.current_session_status {
        CurrentSessionStatusState::Ready { details, .. } => Some(details.checked_at_ms),
        CurrentSessionStatusState::Checking {
            last_known_details: Some(details),
            ..
        }
        | CurrentSessionStatusState::Failed {
            last_known_details: Some(details),
            ..
        } => Some(details.checked_at_ms),
        _ => None,
    };
    with_summary(|summary| {
        summary.inspection = schedule.stats;
        summary.last_trigger = schedule.last_trigger;
        summary.last_decision = schedule.last_decision;
        summary.next_due_at_ms = schedule.armed.map(|armed| armed.due_at_ms);
        if last_success_at_ms.is_some() {
            summary.last_success_at_ms = last_success_at_ms;
        }
    });
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TrustRecheckOutcome {
    Started,
    Joined,
    Deferred,
    Succeeded,
    Failed,
}

pub(crate) fn record_trust_recheck(outcome: TrustRecheckOutcome) {
    with_summary(|summary| {
        let counter = match outcome {
            TrustRecheckOutcome::Started => &mut summary.trust.started,
            TrustRecheckOutcome::Joined => &mut summary.trust.joined,
            TrustRecheckOutcome::Deferred => &mut summary.trust.deferred,
            TrustRecheckOutcome::Succeeded => &mut summary.trust.succeeded,
            TrustRecheckOutcome::Failed => &mut summary.trust.failed,
        };
        *counter = counter.saturating_add(1);
    });
}

pub fn session_check_summary() -> SessionCheckSummary {
    with_summary(|summary| *summary)
}

fn elapsed_bucket(since_ms: Option<u64>, now_ms: u64) -> &'static str {
    let Some(since_ms) = since_ms else {
        return "never";
    };
    let Some(elapsed) = now_ms.checked_sub(since_ms) else {
        return "clock_behind";
    };
    duration_bucket(elapsed)
}

fn remaining_bucket(due_ms: Option<u64>, now_ms: u64) -> &'static str {
    match due_ms {
        None => "unarmed",
        Some(due_ms) if due_ms <= now_ms => "due",
        Some(due_ms) => duration_bucket(due_ms - now_ms),
    }
}

fn duration_bucket(ms: u64) -> &'static str {
    const MINUTE: u64 = 60 * 1_000;
    const HOUR: u64 = 60 * MINUTE;
    if ms < MINUTE {
        "<1m"
    } else if ms < 5 * MINUTE {
        "1-5m"
    } else if ms < 30 * MINUTE {
        "5-30m"
    } else if ms < 2 * HOUR {
        "30m-2h"
    } else if ms < 6 * HOUR {
        "2-6h"
    } else {
        ">=6h"
    }
}

pub fn session_check_summary_event(summary: SessionCheckSummary, now_ms: u64) -> DiagnosticEvent {
    DiagnosticEvent::new(DiagnosticLevel::Info, "session_status", "check_summary")
        .field(DiagnosticField::token(
            "last_trigger",
            summary
                .last_trigger
                .map_or("none", |trigger| trigger.as_str()),
        ))
        .field(DiagnosticField::token(
            "last_decision",
            summary
                .last_decision
                .map_or("none", |decision| decision.as_str()),
        ))
        .field(DiagnosticField::token(
            "since_last_success",
            elapsed_bucket(summary.last_success_at_ms, now_ms),
        ))
        .field(DiagnosticField::token(
            "until_next_due",
            remaining_bucket(summary.next_due_at_ms, now_ms),
        ))
        .field(DiagnosticField::count(
            "inspection_started",
            summary.inspection.started,
        ))
        .field(DiagnosticField::count(
            "inspection_joined",
            summary.inspection.joined,
        ))
        .field(DiagnosticField::count(
            "inspection_deferred",
            summary.inspection.deferred,
        ))
        .field(DiagnosticField::count(
            "inspection_dropped",
            summary.inspection.dropped,
        ))
        .field(DiagnosticField::count(
            "inspection_succeeded",
            summary.inspection.succeeded,
        ))
        .field(DiagnosticField::count(
            "inspection_failed",
            summary.inspection.failed,
        ))
        .field(DiagnosticField::count(
            "trust_recheck_started",
            summary.trust.started,
        ))
        .field(DiagnosticField::count(
            "trust_recheck_joined",
            summary.trust.joined,
        ))
        .field(DiagnosticField::count(
            "trust_recheck_deferred",
            summary.trust.deferred,
        ))
        .field(DiagnosticField::count(
            "trust_recheck_succeeded",
            summary.trust.succeeded,
        ))
        .field(DiagnosticField::count(
            "trust_recheck_failed",
            summary.trust.failed,
        ))
}

/// Record the summary into the diagnostic ring; called when a diagnostic
/// snapshot is taken, like the media memory summaries.
pub fn record_session_check_summary() {
    record(session_check_summary_event(
        session_check_summary(),
        crate::time::current_epoch_ms(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_event_carries_only_tokens_counts_and_buckets() {
        let summary = SessionCheckSummary {
            inspection: SessionStatusCheckStats {
                started: 3,
                joined: 2,
                deferred: 40,
                dropped: 1,
                succeeded: 2,
                failed: 1,
            },
            last_trigger: Some(SessionStatusRefreshTrigger::Scheduled),
            last_decision: Some(SessionStatusCheckDecision::Deferred),
            last_success_at_ms: Some(1_000),
            next_due_at_ms: Some(1_000 + 6 * 60 * 60 * 1_000),
            trust: TrustRecheckStats {
                started: 1,
                joined: 4,
                deferred: 2,
                succeeded: 1,
                failed: 0,
            },
        };
        let formatted = koushi_diagnostics::format_event(&session_check_summary_event(
            summary,
            1_000 + 10 * 60 * 1_000,
        ));
        for expected in [
            "last_trigger=scheduled",
            "last_decision=deferred",
            "since_last_success=5-30m",
            "until_next_due=2-6h",
            "inspection_started=3",
            "inspection_joined=2",
            "inspection_deferred=40",
            "inspection_dropped=1",
            "inspection_failed=1",
            "trust_recheck_joined=4",
            "trust_recheck_deferred=2",
        ] {
            assert!(
                formatted.contains(expected),
                "{expected} missing: {formatted}"
            );
        }
    }

    #[test]
    fn buckets_handle_missing_times_and_clock_regression() {
        assert_eq!(elapsed_bucket(None, 5), "never");
        assert_eq!(elapsed_bucket(Some(10), 5), "clock_behind");
        assert_eq!(remaining_bucket(None, 5), "unarmed");
        assert_eq!(remaining_bucket(Some(5), 5), "due");
    }
}
