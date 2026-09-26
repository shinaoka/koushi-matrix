//! #1009: one reducer owner for session-status check admission, due time,
//! failure backoff, and in-flight deduplication. Reconnect is a connectivity
//! change that lets pending/due work run, never a reason to check by itself.

use koushi_state::{
    AppAction, AppEffect, AppState, CurrentDeviceTrustState, CurrentSessionBackupState,
    CurrentSessionStatusDetails, CurrentSessionStatusFailureKind, CurrentSessionStatusState,
    CurrentSessionSyncState, OwnIdentityVerification, SESSION_STATUS_FAILURE_BACKOFF_CAP_MS,
    SESSION_STATUS_FRESHNESS_MS, SessionAuthenticationMethod, SessionInfo, SessionState,
    SessionStatusRefreshTrigger, SyncLifecycleStatus, SyncState, reduce,
};

const MINUTE_MS: u64 = 60 * 1_000;
const HOUR_MS: u64 = 60 * MINUTE_MS;

fn details(checked_at_ms: u64) -> CurrentSessionStatusDetails {
    CurrentSessionStatusDetails::new(
        Some("Koushi on Linux".to_owned()),
        "DEVICE".to_owned(),
        SessionAuthenticationMethod::OAuth,
        CurrentSessionSyncState::Running,
        CurrentDeviceTrustState::Verified,
        true,
        OwnIdentityVerification::Verified,
        CurrentSessionBackupState::Ready,
        checked_at_ms,
    )
}

/// A Ready session whose sync connectivity is proven.
fn connected_state(status: CurrentSessionStatusState) -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@user:example.invalid".to_owned(),
            device_id: "DEVICE".to_owned(),
            authentication_method: SessionAuthenticationMethod::Unknown,
        }),
        sync: SyncState::Running,
        sync_generation: 1,
        current_session_status: status,
        ..AppState::default()
    }
}

fn ready_at(checked_at_ms: u64) -> CurrentSessionStatusState {
    CurrentSessionStatusState::Ready {
        request_id: 1,
        details: details(checked_at_ms),
    }
}

fn failed_at(checked_at_ms: u64, consecutive_failures: u32) -> CurrentSessionStatusState {
    CurrentSessionStatusState::Failed {
        request_id: 1,
        kind: CurrentSessionStatusFailureKind::Network,
        checked_at_ms,
        last_known_details: Some(details(0)),
        consecutive_failures,
    }
}

fn refreshes(effects: &[AppEffect]) -> Vec<(u64, SessionStatusRefreshTrigger)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            AppEffect::RefreshCurrentSessionStatus {
                request_id,
                trigger,
            } => Some((*request_id, *trigger)),
            _ => None,
        })
        .collect()
}

fn armed(effects: &[AppEffect]) -> Option<(u64, u64)> {
    effects.iter().rev().find_map(|effect| match effect {
        AppEffect::ArmCurrentSessionStatusCheck { token, due_at_ms } => Some((*token, *due_at_ms)),
        _ => None,
    })
}

fn open(state: &mut AppState, request_id: u64, now_ms: u64) -> Vec<AppEffect> {
    reduce(
        state,
        AppAction::CurrentSessionStatusRefreshRequested {
            request_id,
            trigger: SessionStatusRefreshTrigger::Open,
            now_ms,
        },
    )
}

fn manual(state: &mut AppState, request_id: u64, now_ms: u64) -> Vec<AppEffect> {
    reduce(
        state,
        AppAction::CurrentSessionStatusRefreshRequested {
            request_id,
            trigger: SessionStatusRefreshTrigger::Manual,
            now_ms,
        },
    )
}

fn check_due(state: &mut AppState, token: u64, now_ms: u64) -> Vec<AppEffect> {
    reduce(
        state,
        AppAction::CurrentSessionStatusCheckDue { token, now_ms },
    )
}

/// Drop and restore sync connectivity; returns the effects of the Running edge.
fn reconnect(state: &mut AppState) -> Vec<AppEffect> {
    let generation = state.sync_generation + 1;
    reduce(
        state,
        AppAction::SyncStatusChanged {
            generation,
            status: SyncLifecycleStatus::Reconnecting {
                reason: "transport".to_owned(),
            },
        },
    );
    reduce(
        state,
        AppAction::SyncStatusChanged {
            generation: generation + 1,
            status: SyncLifecycleStatus::Running,
        },
    )
}

/// Let the Core timer fire for the latest arm at `now_ms`.
fn fire(state: &mut AppState, arm: Option<(u64, u64)>, now_ms: u64) -> Vec<AppEffect> {
    let (token, _) = arm.expect("an armed due time");
    check_due(state, token, now_ms)
}

fn active_request(state: &AppState) -> u64 {
    match &state.current_session_status {
        CurrentSessionStatusState::Checking { request_id, .. } => *request_id,
        other => panic!("expected Checking, got {other:?}"),
    }
}

fn fail_active(state: &mut AppState, checked_at_ms: u64) -> Vec<AppEffect> {
    let request_id = active_request(state);
    reduce(
        state,
        AppAction::CurrentSessionStatusRefreshFailed {
            request_id,
            kind: CurrentSessionStatusFailureKind::ConnectivityUnavailable,
            checked_at_ms,
        },
    )
}

fn succeed_active(state: &mut AppState, checked_at_ms: u64) -> Vec<AppEffect> {
    let request_id = active_request(state);
    reduce(
        state,
        AppAction::CurrentSessionStatusRefreshed {
            request_id,
            details: details(checked_at_ms),
        },
    )
}

fn consecutive_failures(state: &AppState) -> u32 {
    match &state.current_session_status {
        CurrentSessionStatusState::Failed {
            consecutive_failures,
            ..
        }
        | CurrentSessionStatusState::Checking {
            consecutive_failures,
            ..
        } => *consecutive_failures,
        _ => 0,
    }
}

/// Row 1: a successful Ready status within the 6 h window is served for any
/// number of opens and reconnects; the boundary admits exactly one check.
#[test]
fn verified_status_is_not_rechecked_by_opens_or_reconnects_until_due() {
    let checked_at = 10 * HOUR_MS;
    let mut state = connected_state(ready_at(checked_at));
    let mut inspections = 0;
    let mut last_arm = None;

    for step in 0..360u64 {
        let now = checked_at + step * MINUTE_MS;
        inspections += refreshes(&open(&mut state, 1_000 + step, now)).len();
        let effects = reconnect(&mut state);
        inspections += refreshes(&effects).len();
        if let Some(arm) = armed(&effects) {
            assert_eq!(
                arm.1,
                checked_at + SESSION_STATUS_FRESHNESS_MS,
                "a reconnect must not move the due time"
            );
            // The Core timer fires immediately only if the due time passed.
            inspections += refreshes(&fire(&mut state, Some(arm), now)).len();
            last_arm = armed(&check_due(&mut state, arm.0, now)).or(Some(arm));
        }
    }
    assert!(last_arm.is_some(), "every Running edge re-arms the timer");
    assert_eq!(inspections, 0, "no recheck before the 6 h due time");

    let boundary = checked_at + SESSION_STATUS_FRESHNESS_MS;
    let arm = armed(&reconnect(&mut state));
    let effects = fire(&mut state, arm, boundary);
    assert_eq!(
        refreshes(&effects).len(),
        1,
        "exactly one check at the boundary"
    );
    assert_eq!(
        refreshes(&open(&mut state, 9_999, boundary)).len(),
        0,
        "an open at the boundary joins the in-flight check"
    );
}

/// Row 2 / reproduction A: a reconnect right after a network failure must
/// respect the same failure backoff as an open.
#[test]
fn reconnect_after_network_failure_respects_the_failure_backoff() {
    let mut state = connected_state(failed_at(2_000, 1));
    state.sync = SyncState::Reconnecting {
        reason: "transport".to_owned(),
    };

    assert!(refreshes(&open(&mut state, 10, 2_001)).is_empty());
    let effects = reduce(
        &mut state,
        AppAction::SyncStatusChanged {
            generation: 2,
            status: SyncLifecycleStatus::Running,
        },
    );
    assert!(
        refreshes(&effects).is_empty(),
        "the Running edge must not issue a check itself"
    );
    let arm = armed(&effects).expect("the Running edge re-arms the timer");
    assert_eq!(arm.1, 2_000 + MINUTE_MS, "the backoff due time is kept");

    // An early timer notification (e.g. clock skew) re-arms instead of checking.
    let early = fire(&mut state, Some(arm), 2_002);
    assert!(refreshes(&early).is_empty());
    let rearmed = armed(&early).expect("an early notification re-arms");
    assert_eq!(rearmed.1, 2_000 + MINUTE_MS);

    let due = fire(&mut state, Some(rearmed), 2_000 + MINUTE_MS);
    assert_eq!(
        refreshes(&due),
        vec![(
            active_request(&state),
            SessionStatusRefreshTrigger::Recovery
        )],
        "one recovery check once the backoff elapsed"
    );
}

/// Row 3 / reproduction B: the cancellation result of an in-flight check can
/// arrive after the next Running edge. The in-flight request must not be
/// replaced, and its failure must count toward the backoff.
#[test]
fn reconnect_never_replaces_an_in_flight_check_and_failures_accumulate() {
    let mut state = connected_state(ready_at(0));
    let mut now = SESSION_STATUS_FRESHNESS_MS;
    let arm = armed(&reconnect(&mut state));
    assert_eq!(refreshes(&fire(&mut state, arm, now)).len(), 1);

    let mut inspections = 1;
    for cycle in 0..100u32 {
        let in_flight = active_request(&state);
        let effects = reconnect(&mut state);
        assert!(
            refreshes(&effects).is_empty(),
            "cycle {cycle}: a Running edge must not re-issue while Checking"
        );
        assert_eq!(
            active_request(&state),
            in_flight,
            "cycle {cycle}: the in-flight request id must be kept"
        );
        now += 1_000;
        let settled = fail_active(&mut state, now);
        assert_eq!(consecutive_failures(&state), cycle + 1);
        // The next automatic check waits for the timer at the backoff due time.
        let arm = armed(&settled).expect("a failure arms the backoff due time");
        assert!(arm.1 > now, "cycle {cycle}: the retry is in the future");
        now = arm.1;
        let effects = fire(&mut state, Some(arm), now);
        inspections += refreshes(&effects).len();
        assert_eq!(refreshes(&effects).len(), 1, "one retry per backoff period");
    }
    assert_eq!(inspections, 101);
    assert!(
        now >= 100 * SESSION_STATUS_FAILURE_BACKOFF_CAP_MS / 2,
        "100 flaps must be spread over the capped backoff, not issued back to back"
    );
}

/// Row 4: after three or more failures, time passing alone retries at the
/// capped backoff, and a success returns to the normal period.
#[test]
fn repeated_failures_keep_retrying_at_the_capped_backoff_and_recover() {
    let failed = 5 * HOUR_MS;
    let mut state = connected_state(failed_at(failed, 7));
    let arm = armed(&reconnect(&mut state)).expect("armed");
    assert_eq!(arm.1, failed + SESSION_STATUS_FAILURE_BACKOFF_CAP_MS);
    let early = fire(&mut state, Some(arm), failed + MINUTE_MS);
    assert!(refreshes(&early).is_empty());

    let due = failed + SESSION_STATUS_FAILURE_BACKOFF_CAP_MS;
    let effects = fire(&mut state, armed(&early), due);
    assert_eq!(
        refreshes(&effects).len(),
        1,
        "no permanent stop after the cap"
    );

    let settled = succeed_active(&mut state, due + 1);
    assert_eq!(consecutive_failures(&state), 0);
    assert_eq!(
        armed(&settled).map(|arm| arm.1),
        Some(due + 1 + SESSION_STATUS_FRESHNESS_MS),
        "a success returns to the 6 h period"
    );
}

/// Row 5: with a stable connection and no panel interaction the timer runs
/// several periodic checks.
#[test]
fn stable_connection_runs_periodic_checks_without_interaction() {
    let mut state = connected_state(CurrentSessionStatusState::Idle);
    state.sync = SyncState::Starting;
    let edge = reduce(
        &mut state,
        AppAction::SyncStatusChanged {
            generation: 2,
            status: SyncLifecycleStatus::Running,
        },
    );
    let mut now = 1_000;
    let mut arm = armed(&edge).expect("the first Running edge arms the initial check");
    assert!(arm.1 <= now, "an unchecked session is due immediately");

    for period in 0..3u64 {
        let effects = fire(&mut state, Some(arm), now);
        assert_eq!(
            refreshes(&effects),
            vec![(
                active_request(&state),
                SessionStatusRefreshTrigger::Scheduled
            )],
            "period {period}: one scheduled check"
        );
        let settled = succeed_active(&mut state, now);
        arm = armed(&settled).expect("a success arms the next period");
        assert_eq!(arm.1, now + SESSION_STATUS_FRESHNESS_MS);
        now = arm.1;
    }
}

/// Row 6: offline past several due times yields exactly one check on return.
#[test]
fn offline_past_due_runs_one_check_on_return_without_catching_up() {
    let mut state = connected_state(ready_at(0));
    let arm = armed(&reconnect(&mut state)).expect("armed");
    state.sync = SyncState::Reconnecting {
        reason: "offline".to_owned(),
    };
    state.sync_generation += 10;

    // The timer fires while offline: nothing starts and nothing re-arms.
    let offline = fire(&mut state, Some(arm), SESSION_STATUS_FRESHNESS_MS);
    assert!(refreshes(&offline).is_empty());
    assert!(armed(&offline).is_none());

    let now = 4 * SESSION_STATUS_FRESHNESS_MS;
    let generation = state.sync_generation + 1;
    let edge = reduce(
        &mut state,
        AppAction::SyncStatusChanged {
            generation,
            status: SyncLifecycleStatus::Running,
        },
    );
    let arm = armed(&edge).expect("the Running edge re-arms");
    let effects = fire(&mut state, Some(arm), now);
    assert_eq!(refreshes(&effects).len(), 1);
    // A duplicate or stale notification cannot start a second check.
    assert!(refreshes(&fire(&mut state, Some(arm), now)).is_empty());
    succeed_active(&mut state, now);
    assert!(refreshes(&fire(&mut state, Some(arm), now + 1)).is_empty());
}

/// Row 7: manual, periodic and reconnect triggers join one in-flight request,
/// on both sides of the cancellation result.
#[test]
fn manual_periodic_and_reconnect_join_one_in_flight_request() {
    let mut state = connected_state(ready_at(0));
    let now = SESSION_STATUS_FRESHNESS_MS;
    let arm = armed(&reconnect(&mut state));
    fire(&mut state, arm, now);
    let scheduled = active_request(&state);

    assert!(refreshes(&manual(&mut state, 50, now)).is_empty());
    assert!(refreshes(&fire(&mut state, arm, now)).is_empty());
    assert!(refreshes(&reconnect(&mut state)).is_empty());
    assert_eq!(active_request(&state), scheduled);

    // Cancellation result lands after the Running edge.
    fail_active(&mut state, now + 1);
    assert_eq!(consecutive_failures(&state), 1);
    // Manual bypasses the backoff; the next reconnect joins it.
    assert_eq!(refreshes(&manual(&mut state, 51, now + 2)).len(), 1);
    assert!(refreshes(&reconnect(&mut state)).is_empty());
    assert_eq!(active_request(&state), 51);
    fail_active(&mut state, now + 3);
    assert_eq!(consecutive_failures(&state), 2);
    // Cancellation result lands before the Running edge: still backed off.
    let edge = reconnect(&mut state);
    assert!(refreshes(&edge).is_empty());
    assert_eq!(armed(&edge).map(|arm| arm.1), Some(now + 3 + 2 * MINUTE_MS));
}

/// Scheduler-issued request ids never collide with command sequence numbers.
#[test]
fn scheduled_request_ids_are_disjoint_from_command_sequences() {
    let mut state = connected_state(CurrentSessionStatusState::Idle);
    let arm = armed(&reconnect(&mut state));
    fire(&mut state, arm, 0);
    assert!(active_request(&state) >= 1 << 62);
}

/// Trust loss and session teardown are applied immediately and fence the
/// previous session's timer notifications.
#[test]
fn trust_loss_and_logout_fence_stale_due_notifications() {
    let mut state = connected_state(ready_at(0));
    let arm = armed(&reconnect(&mut state)).expect("armed");
    reduce(
        &mut state,
        AppAction::AuthoritativeDeviceTrustChanged {
            generation: 1,
            transition_id: 1,
            trust: CurrentDeviceTrustState::Unverified,
        },
    );
    assert_eq!(
        state.current_session_status,
        CurrentSessionStatusState::Idle
    );
    assert!(refreshes(&fire(&mut state, Some(arm), SESSION_STATUS_FRESHNESS_MS)).is_empty());

    let mut state = connected_state(ready_at(0));
    let arm = armed(&reconnect(&mut state)).expect("armed");
    reduce(&mut state, AppAction::LogoutRequested);
    assert_eq!(
        state.current_session_status,
        CurrentSessionStatusState::Idle
    );
    assert!(
        state.current_session_status_schedule.token > arm.0,
        "session teardown must advance the schedule token"
    );
    assert!(refreshes(&fire(&mut state, Some(arm), SESSION_STATUS_FRESHNESS_MS)).is_empty());
}

/// A cached status invalidated by a disagreeing trust observation while
/// connected is re-checked without waiting for the next reconnect.
#[test]
fn invalidated_status_on_a_connected_session_arms_an_immediate_check() {
    let mut stale = details(0);
    stale.verification = CurrentDeviceTrustState::Unverified;
    let mut state = connected_state(CurrentSessionStatusState::Ready {
        request_id: 1,
        details: stale,
    });
    let effects = reduce(
        &mut state,
        AppAction::AuthoritativeDeviceTrustChanged {
            generation: 1,
            transition_id: 1,
            trust: CurrentDeviceTrustState::Verified,
        },
    );
    let arm = armed(&effects).expect("invalidating a cached status arms an immediate check");
    assert_eq!(refreshes(&fire(&mut state, Some(arm), 5)).len(), 1);
}

/// A wall clock that moved backwards must neither suppress checks for the
/// whole skew nor cause a burst.
#[test]
fn clock_regression_runs_one_check_and_restamps() {
    let mut state = connected_state(ready_at(10 * HOUR_MS));
    let arm = armed(&reconnect(&mut state));
    let effects = fire(&mut state, arm, HOUR_MS);
    assert_eq!(refreshes(&effects).len(), 1);
    let settled = succeed_active(&mut state, HOUR_MS);
    assert_eq!(
        armed(&settled).map(|arm| arm.1),
        Some(HOUR_MS + SESSION_STATUS_FRESHNESS_MS)
    );
}

/// The reducer records decisions for the private-safe diagnostic summary.
#[test]
fn decisions_are_counted_for_the_diagnostic_summary() {
    let mut state = connected_state(ready_at(0));
    open(&mut state, 5, MINUTE_MS);
    let arm = armed(&reconnect(&mut state));
    fire(&mut state, arm, SESSION_STATUS_FRESHNESS_MS);
    manual(&mut state, 6, SESSION_STATUS_FRESHNESS_MS);
    fail_active(&mut state, SESSION_STATUS_FRESHNESS_MS + 1);

    let stats = state.current_session_status_schedule.stats;
    assert_eq!(stats.started, 1);
    assert_eq!(stats.joined, 1);
    assert!(stats.deferred >= 1);
    assert_eq!(stats.failed, 1);
    assert_eq!(stats.succeeded, 0);
}
