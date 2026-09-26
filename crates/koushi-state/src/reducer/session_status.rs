//! #1009: the single owner of current-session status check admission, due
//! time, failure backoff, and in-flight deduplication. Every trigger passes
//! through [`admit`]; Core only supplies the clock and runs the timer and the
//! network work the reducer decides on.

use crate::state::{
    ArmedSessionStatusCheck, SESSION_STATUS_FRESHNESS_MS, SessionStatusCheckDecision,
    session_status_failure_backoff_ms,
};
use crate::{
    AppEffect, AppState, CurrentSessionStatusDetails, CurrentSessionStatusFailureKind,
    CurrentSessionStatusState, SessionState, SessionStatusRefreshTrigger, SyncState,
};

fn last_known_details(state: &CurrentSessionStatusState) -> Option<CurrentSessionStatusDetails> {
    match state {
        CurrentSessionStatusState::Ready { details, .. } => Some(details.clone()),
        CurrentSessionStatusState::Checking {
            last_known_details, ..
        }
        | CurrentSessionStatusState::Failed {
            last_known_details, ..
        } => last_known_details.clone(),
        CurrentSessionStatusState::Idle => None,
    }
}

fn consecutive_failures(state: &CurrentSessionStatusState) -> u32 {
    match state {
        CurrentSessionStatusState::Checking {
            consecutive_failures,
            ..
        }
        | CurrentSessionStatusState::Failed {
            consecutive_failures,
            ..
        } => *consecutive_failures,
        CurrentSessionStatusState::Idle | CurrentSessionStatusState::Ready { .. } => 0,
    }
}

/// The settled check time and the interval after which an automatic check is
/// due again. `None` for `Idle` (never checked) and `Checking` (in flight).
fn settled_interval(status: &CurrentSessionStatusState) -> Option<(u64, u64)> {
    match status {
        CurrentSessionStatusState::Idle | CurrentSessionStatusState::Checking { .. } => None,
        CurrentSessionStatusState::Ready { details, .. } => {
            Some((details.checked_at_ms, SESSION_STATUS_FRESHNESS_MS))
        }
        CurrentSessionStatusState::Failed {
            checked_at_ms,
            consecutive_failures,
            ..
        } => Some((
            *checked_at_ms,
            session_status_failure_backoff_ms(*consecutive_failures),
        )),
    }
}

/// Wall-clock due time of the next automatic check. An unchecked session is
/// due immediately. There is no retry ceiling: a failed state is always due
/// after its (capped) backoff.
fn due_at_ms(status: &CurrentSessionStatusState) -> u64 {
    settled_interval(status)
        .map(|(checked_at, interval)| checked_at.saturating_add(interval))
        .unwrap_or(0)
}

/// Whether an automatic check is due at `now_ms`. A clock that moved behind
/// the recorded check time makes that record unreliable, so the state is due:
/// one check re-stamps it instead of suppressing checks for the whole skew.
fn is_due(status: &CurrentSessionStatusState, now_ms: u64) -> bool {
    match settled_interval(status) {
        None => matches!(status, CurrentSessionStatusState::Idle),
        Some((checked_at, interval)) => {
            now_ms < checked_at || now_ms - checked_at >= interval
        }
    }
}

fn session_is_ready(state: &AppState) -> bool {
    matches!(state.session, SessionState::Ready(_))
}

fn connectivity_proven(state: &AppState) -> bool {
    matches!(state.sync, SyncState::Running)
}

/// Arm the Core timer for the next due time. Only a Ready session with proven
/// connectivity and no in-flight check arms; otherwise the next connectivity
/// edge or settlement does.
fn arm(state: &mut AppState, trigger: SessionStatusRefreshTrigger) -> Option<AppEffect> {
    if !session_is_ready(state)
        || !connectivity_proven(state)
        || matches!(
            state.current_session_status,
            CurrentSessionStatusState::Checking { .. }
        )
    {
        return None;
    }
    let due_at_ms = due_at_ms(&state.current_session_status);
    let schedule = &mut state.current_session_status_schedule;
    schedule.token = schedule.token.wrapping_add(1);
    schedule.armed = Some(ArmedSessionStatusCheck {
        token: schedule.token,
        due_at_ms,
        trigger,
    });
    Some(AppEffect::ArmCurrentSessionStatusCheck {
        token: schedule.token,
        due_at_ms,
    })
}

/// The one admission function for every trigger.
fn admit(
    state: &mut AppState,
    request_id: u64,
    trigger: SessionStatusRefreshTrigger,
    now_ms: u64,
) -> Vec<AppEffect> {
    let decision = if !session_is_ready(state) {
        SessionStatusCheckDecision::Deferred
    } else if matches!(
        state.current_session_status,
        CurrentSessionStatusState::Checking { .. }
    ) {
        SessionStatusCheckDecision::Joined
    } else if trigger != SessionStatusRefreshTrigger::Manual
        && !is_due(&state.current_session_status, now_ms)
    {
        SessionStatusCheckDecision::Deferred
    } else {
        SessionStatusCheckDecision::Started
    };
    state
        .current_session_status_schedule
        .record(trigger, decision);
    if decision != SessionStatusCheckDecision::Started {
        return Vec::new();
    }
    let last_known_details = last_known_details(&state.current_session_status);
    let consecutive_failures = consecutive_failures(&state.current_session_status);
    state.current_session_status = CurrentSessionStatusState::Checking {
        request_id,
        trigger,
        last_known_details,
        consecutive_failures,
    };
    // The settlement arms the next due time.
    state.current_session_status_schedule.armed = None;
    vec![AppEffect::RefreshCurrentSessionStatus {
        request_id,
        trigger,
    }]
}

pub(super) fn handle_refresh_requested(
    state: &mut AppState,
    request_id: u64,
    trigger: SessionStatusRefreshTrigger,
    now_ms: u64,
) -> Vec<AppEffect> {
    admit(state, request_id, trigger, now_ms)
}

/// The Core timer for an armed due time fired.
pub(super) fn handle_check_due(state: &mut AppState, token: u64, now_ms: u64) -> Vec<AppEffect> {
    let Some(armed) = state
        .current_session_status_schedule
        .armed
        .filter(|armed| armed.token == token)
    else {
        return Vec::new();
    };
    state.current_session_status_schedule.armed = None;
    if !session_is_ready(state) {
        return Vec::new();
    }
    if !connectivity_proven(state)
        && !matches!(
            state.current_session_status,
            CurrentSessionStatusState::Checking { .. }
        )
    {
        // The next Running edge re-arms; nothing starts while offline.
        state
            .current_session_status_schedule
            .record(armed.trigger, SessionStatusCheckDecision::Dropped);
        return Vec::new();
    }
    if !matches!(
        state.current_session_status,
        CurrentSessionStatusState::Checking { .. }
    ) && !is_due(&state.current_session_status, now_ms)
    {
        state
            .current_session_status_schedule
            .record(armed.trigger, SessionStatusCheckDecision::Deferred);
        return arm(state, armed.trigger).into_iter().collect();
    }
    let request_id = state.current_session_status_schedule.next_request_id();
    admit(state, request_id, armed.trigger, now_ms)
}

/// Sync connectivity became proven. A reconnect is not a reason to check: it
/// only re-arms the timer for the unchanged due time, so pending or overdue
/// work runs once and an in-flight check is joined, never replaced.
pub(super) fn handle_connectivity_proven(state: &mut AppState) -> Option<AppEffect> {
    let trigger = if matches!(
        state.current_session_status,
        CurrentSessionStatusState::Idle
    ) {
        SessionStatusRefreshTrigger::Scheduled
    } else {
        SessionStatusRefreshTrigger::Recovery
    };
    arm(state, trigger)
}

pub(super) fn handle_refreshed(
    state: &mut AppState,
    request_id: u64,
    details: CurrentSessionStatusDetails,
) -> Vec<AppEffect> {
    if !matches!(
        state.current_session_status,
        CurrentSessionStatusState::Checking {
            request_id: active_request_id,
            ..
        } if active_request_id == request_id
    ) {
        return Vec::new();
    }
    state.current_session_status = CurrentSessionStatusState::Ready {
        request_id,
        details,
    };
    let stats = &mut state.current_session_status_schedule.stats;
    stats.succeeded = stats.succeeded.saturating_add(1);
    arm(state, SessionStatusRefreshTrigger::Scheduled)
        .into_iter()
        .collect()
}

pub(super) fn handle_refresh_failed(
    state: &mut AppState,
    request_id: u64,
    kind: CurrentSessionStatusFailureKind,
    checked_at_ms: u64,
) -> Vec<AppEffect> {
    if !matches!(
        state.current_session_status,
        CurrentSessionStatusState::Checking {
            request_id: active_request_id,
            ..
        } if active_request_id == request_id
    ) {
        return Vec::new();
    }
    let last_known_details = last_known_details(&state.current_session_status);
    let consecutive_failures =
        consecutive_failures(&state.current_session_status).saturating_add(1);
    state.current_session_status = CurrentSessionStatusState::Failed {
        request_id,
        kind,
        checked_at_ms,
        last_known_details,
        consecutive_failures,
    };
    let stats = &mut state.current_session_status_schedule.stats;
    stats.failed = stats.failed.saturating_add(1);
    arm(state, SessionStatusRefreshTrigger::Scheduled)
        .into_iter()
        .collect()
}

/// #982: with the freshness gate in place, a cached status must not outlive the
/// device trust it reports. Any observed trust that disagrees with the cached
/// status invalidates it, so the next automatic check re-inspects instead of
/// serving a pre-verification result for the whole freshness window. A repeated
/// signal that agrees with the cache changes nothing. Returns whether the slice
/// was reset.
pub(super) fn invalidate_if_trust_disagrees(
    state: &mut AppState,
    trust: crate::state::CurrentDeviceTrustState,
) -> bool {
    if last_known_details(&state.current_session_status)
        .is_none_or(|details| details.verification != trust)
    {
        reset(state);
        return true;
    }
    false
}

/// After an invalidating reset, a still-Ready connected session does not wait
/// for the next connectivity edge: the `Idle` slice is armed as due now.
pub(super) fn arm_after_reset(state: &mut AppState) -> Option<AppEffect> {
    arm(state, SessionStatusRefreshTrigger::Scheduled)
}

pub(super) fn reset(state: &mut AppState) {
    state.current_session_status = CurrentSessionStatusState::Idle;
    state.current_session_status_schedule.retire();
}
