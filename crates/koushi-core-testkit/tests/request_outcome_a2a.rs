use futures_util::FutureExt;
use koushi_core::runtime::request_outcome::{
    OutcomeCorrelation, RequestOutcome, RequestOutcomeError, RequestOutcomeExpectation,
};
use koushi_core::{
    AccountKey, CoreConnection, IntentOutcome, RequestId, RuntimeConnectionId, TimelineKey,
};
use koushi_protocol::event::{AccountEvent, CoreEvent};
use koushi_state::{
    AppState, FocusedContextState, MainTimelineAnchor, SearchScope, SessionInfo, SessionState,
};
use std::time::Duration;

fn request(sequence: u64) -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(41),
        sequence,
    }
}

fn ready_state(user_id: &str) -> AppState {
    let mut state = AppState::default();
    state.session = SessionState::Ready(SessionInfo {
        homeserver: "https://example.invalid".to_owned(),
        user_id: user_id.to_owned(),
        device_id: "DEVICE".to_owned(),
        authentication_method: Default::default(),
    });
    state
}

fn versioned(
    state: AppState,
    generation: u64,
) -> koushi_protocol::state_update::VersionedAppStateSnapshot {
    koushi_protocol::state_update::VersionedAppStateSnapshot { generation, state }
}

fn commit(request_id: RequestId) -> CoreEvent {
    CoreEvent::IntentLifecycle {
        request_id,
        outcome: IntentOutcome::Committed,
        published_generation: 1,
    }
}

// Rejection is pending until the controlled deadline, regardless of CI scheduling.
#[tokio::test(start_paused = true)]
async fn authenticated_rejects_a_non_terminal_locked_session() {
    let (mut connection, control) = CoreConnection::new_for_testing(4);
    let request_id = request(1);
    let account_key = AccountKey("@alice:example.invalid".to_owned());
    let mut state = ready_state(&account_key.0);
    state.session = SessionState::Locked(match state.session {
        SessionState::Ready(info) => info,
        _ => unreachable!(),
    });

    let waiter = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::Authenticated {
            request_id,
            account_key: Some(account_key.clone()),
        },
        0,
        tokio::time::Instant::now() + Duration::from_millis(20),
    );
    tokio::pin!(waiter);
    control.send_event(CoreEvent::Account(AccountEvent::LoggedIn {
        request_id,
        account_key,
    }));
    control.send_snapshot(versioned(state, 1));
    assert!(waiter.as_mut().now_or_never().is_none());
    tokio::time::advance(Duration::from_millis(20)).await;
    assert_eq!(waiter.await, Err(RequestOutcomeError::TimedOut));
}

#[tokio::test(start_paused = true)]
async fn signed_out_rejects_a_foreign_account_event() {
    let (mut connection, control) = CoreConnection::new_for_testing(4);
    let request_id = request(2);
    let expected = AccountKey("@alice:example.invalid".to_owned());
    let mut state = AppState::default();
    state.session = SessionState::SignedOut;

    let waiter = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::SignedOut {
            request_id,
            account_key: expected,
            allow_projection_only: false,
        },
        0,
        tokio::time::Instant::now() + Duration::from_millis(20),
    );
    tokio::pin!(waiter);
    control.send_event(CoreEvent::Account(AccountEvent::LoggedOut {
        request_id,
        account_key: AccountKey("@bob:example.invalid".to_owned()),
    }));
    control.send_snapshot(versioned(state, 1));
    assert!(waiter.as_mut().now_or_never().is_none());
    tokio::time::advance(Duration::from_millis(20)).await;
    assert_eq!(waiter.await, Err(RequestOutcomeError::TimedOut));
}

#[tokio::test]
async fn focused_context_close_and_open_require_correlated_snapshot_targets() {
    let account_key = AccountKey("@alice:example.invalid".to_owned());
    let room_id = "!room:example.invalid".to_owned();
    let event_id = "$event:example.invalid".to_owned();

    let (mut connection, control) = CoreConnection::new_for_testing(4);
    let close_request = request(3);
    let close = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(close_request),
        RequestOutcomeExpectation::FocusedContextClosed {
            request_id: close_request,
            account_key: account_key.clone(),
            room_id: Some(room_id.clone()),
            allow_projection_only: false,
        },
        0,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    tokio::pin!(close);
    control.send_event(commit(close_request));
    let mut wrong_closed = ready_state("@bob:example.invalid");
    wrong_closed.navigation.active_room_id = Some("!other:example.invalid".to_owned());
    wrong_closed.focused_context = FocusedContextState::Closed;
    control.send_snapshot(versioned(wrong_closed, 1));
    assert!(close.as_mut().now_or_never().is_none());

    let mut closed = ready_state(&account_key.0);
    closed.navigation.active_room_id = Some(room_id.clone());
    closed.focused_context = FocusedContextState::Closed;
    control.send_snapshot(versioned(closed, 2));
    assert!(matches!(
        close.await,
        Ok(RequestOutcome::FocusedContext { .. })
    ));

    let (mut connection, control) = CoreConnection::new_for_testing(4);
    let open_request = request(4);
    let open = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(open_request),
        RequestOutcomeExpectation::FocusedContextOpened {
            request_id: open_request,
            account_key,
            room_id: room_id.clone(),
            event_id: Some(event_id.clone()),
        },
        0,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    tokio::pin!(open);
    control.send_event(commit(open_request));
    let mut wrong_opened = ready_state("@alice:example.invalid");
    wrong_opened.focused_context = FocusedContextState::Open {
        room_id: room_id.clone(),
        event_id: "$other:example.invalid".to_owned(),
        is_subscribed: true,
    };
    control.send_snapshot(versioned(wrong_opened, 1));
    assert!(open.as_mut().now_or_never().is_none());

    let mut opened = ready_state("@alice:example.invalid");
    opened.focused_context = FocusedContextState::Open {
        room_id,
        event_id,
        is_subscribed: true,
    };
    control.send_snapshot(versioned(opened, 2));
    assert!(matches!(
        open.await,
        Ok(RequestOutcome::FocusedContext { .. })
    ));
}

#[tokio::test]
async fn main_timeline_anchor_requires_the_exact_focused_timeline_key() {
    let (mut connection, control) = CoreConnection::new_for_testing(4);
    let request_id = request(5);
    let key = TimelineKey {
        account_key: AccountKey("@alice:example.invalid".to_owned()),
        kind: koushi_core::TimelineKind::Focused {
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$event:example.invalid".to_owned(),
        },
    };
    let waiter = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::MainTimelineAnchor {
            request_id,
            key,
            event_id: "$event:example.invalid".to_owned(),
            allow_live_fallback: false,
        },
        1,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    tokio::pin!(waiter);
    control.send_event(commit(request_id));
    let mut wrong_state = ready_state("@alice:example.invalid");
    wrong_state.navigation.active_room_id = Some("!room:example.invalid".to_owned());
    wrong_state.navigation.main_timeline_anchor = Some(MainTimelineAnchor {
        event_id: "$other:example.invalid".to_owned(),
    });
    control.send_snapshot(versioned(wrong_state, 1));
    assert!(waiter.as_mut().now_or_never().is_none());

    let mut state = ready_state("@alice:example.invalid");
    state.navigation.active_room_id = Some("!room:example.invalid".to_owned());
    state.navigation.main_timeline_anchor = Some(MainTimelineAnchor {
        event_id: "$event:example.invalid".to_owned(),
    });
    control.send_snapshot(versioned(state, 2));
    assert!(matches!(
        waiter.await,
        Ok(RequestOutcome::MainTimelineAnchor { .. })
    ));
}

#[tokio::test]
async fn main_timeline_anchor_allows_live_fallback_only_when_requested() {
    let (mut connection, control) = CoreConnection::new_for_testing(4);
    let request_id = request(8);
    let key = TimelineKey {
        account_key: AccountKey("@alice:example.invalid".to_owned()),
        kind: koushi_core::TimelineKind::Focused {
            room_id: "!room:example.invalid".to_owned(),
            event_id: "$missing:example.invalid".to_owned(),
        },
    };
    let waiter = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::MainTimelineAnchor {
            request_id,
            key,
            event_id: "$missing:example.invalid".to_owned(),
            allow_live_fallback: true,
        },
        0,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    tokio::pin!(waiter);
    control.send_event(CoreEvent::IntentLifecycle {
        request_id,
        outcome: IntentOutcome::BenignNoOp(koushi_core::IntentNoOpReason::TimelineTargetMissing),
        published_generation: 1,
    });
    let mut state = ready_state("@alice:example.invalid");
    state.navigation.active_room_id = Some("!room:example.invalid".to_owned());
    control.send_snapshot(versioned(state, 1));
    assert!(matches!(
        waiter.await,
        Ok(RequestOutcome::MainTimelineAnchor { .. })
    ));
}

#[tokio::test]
async fn search_close_requires_the_correlated_commit_and_exact_account_snapshot() {
    let (mut connection, control) = CoreConnection::new_for_testing(4);
    let request_id = request(6);
    let waiter = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::SearchClosed {
            request_id,
            account_key: Some(AccountKey("@alice:example.invalid".to_owned())),
            allow_initial: false,
            allow_projection_only: false,
        },
        0,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    tokio::pin!(waiter);
    control.send_event(commit(request_id));
    let mut wrong_account = ready_state("@bob:example.invalid");
    wrong_account.search = koushi_state::SearchState::Closed;
    control.send_snapshot(versioned(wrong_account, 1));
    assert!(waiter.as_mut().now_or_never().is_none());

    let mut state = ready_state("@alice:example.invalid");
    state.search = koushi_state::SearchState::Closed;
    control.send_snapshot(versioned(state, 2));
    assert!(matches!(waiter.await, Ok(RequestOutcome::Search { .. })));
}

#[tokio::test]
async fn search_start_keeps_exact_account_scope_and_query() {
    let (mut connection, control) = CoreConnection::new_for_testing(4);
    let request_id = request(7);
    let scope = SearchScope::CurrentRoom {
        room_id: "!room:example.invalid".to_owned(),
    };
    let waiter = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::SearchStarted {
            request_id,
            account_key: Some(AccountKey("@alice:example.invalid".to_owned())),
            query: "synthetic query".to_owned(),
            scope: scope.clone(),
        },
        0,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    tokio::pin!(waiter);
    control.send_event(CoreEvent::Search(koushi_core::SearchEvent::Results {
        request_id,
        results: Vec::new(),
    }));

    let mut wrong_account = ready_state("@bob:example.invalid");
    wrong_account.search = koushi_state::SearchState::Searching {
        request_id: request_id.sequence,
        query: "synthetic query".to_owned(),
        scope: scope.clone(),
    };
    control.send_snapshot(versioned(wrong_account, 1));
    assert!(waiter.as_mut().now_or_never().is_none());

    let mut wrong_query = ready_state("@alice:example.invalid");
    wrong_query.search = koushi_state::SearchState::Searching {
        request_id: request_id.sequence,
        query: "other synthetic query".to_owned(),
        scope: scope.clone(),
    };
    control.send_snapshot(versioned(wrong_query, 2));
    assert!(waiter.as_mut().now_or_never().is_none());

    let mut wrong_scope = ready_state("@alice:example.invalid");
    wrong_scope.search = koushi_state::SearchState::Searching {
        request_id: request_id.sequence,
        query: "synthetic query".to_owned(),
        scope: SearchScope::CurrentSpace {
            space_id: "!space:example.invalid".to_owned(),
        },
    };
    control.send_snapshot(versioned(wrong_scope, 3));
    assert!(waiter.as_mut().now_or_never().is_none());

    let mut state = ready_state("@alice:example.invalid");
    state.search = koushi_state::SearchState::Searching {
        request_id: request_id.sequence,
        query: "synthetic query".to_owned(),
        scope,
    };
    control.send_snapshot(versioned(state, 4));
    assert!(matches!(waiter.await, Ok(RequestOutcome::Search { .. })));
}

#[tokio::test]
async fn search_outcomes_treat_lag_as_terminal() {
    let (mut connection, control) = CoreConnection::new_for_testing(1);
    let request_id = request(9);
    let waiter = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::SearchStarted {
            request_id,
            account_key: None,
            query: "synthetic query".to_owned(),
            scope: SearchScope::AllRooms,
        },
        0,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    tokio::pin!(waiter);
    control.send_event(CoreEvent::OperationFailed {
        request_id: request(90),
        failure: koushi_core::CoreFailure::SessionRequired,
    });
    control.send_event(CoreEvent::OperationFailed {
        request_id: request(91),
        failure: koushi_core::CoreFailure::SessionRequired,
    });
    assert_eq!(waiter.await, Err(RequestOutcomeError::Lagged));

    let (mut connection, control) = CoreConnection::new_for_testing(1);
    let request_id = request(10);
    let waiter = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::SearchClosed {
            request_id,
            account_key: None,
            allow_initial: false,
            allow_projection_only: false,
        },
        0,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    tokio::pin!(waiter);
    control.send_event(CoreEvent::OperationFailed {
        request_id: request(92),
        failure: koushi_core::CoreFailure::SessionRequired,
    });
    control.send_event(CoreEvent::OperationFailed {
        request_id: request(93),
        failure: koushi_core::CoreFailure::SessionRequired,
    });
    assert_eq!(waiter.await, Err(RequestOutcomeError::Lagged));
}

#[test]
fn request_outcome_error_remains_typed() {
    assert_eq!(format!("{:?}", RequestOutcomeError::TimedOut), "TimedOut");
}

#[tokio::test]
async fn projection_only_idempotent_outcomes_settle_without_unavailable_terminal_events() {
    let (mut connection, control) = CoreConnection::new_for_testing(4);
    let request_id = request(11);
    let wait = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::SignedOut {
            request_id,
            account_key: AccountKey("@alice:example.invalid".to_owned()),
            allow_projection_only: true,
        },
        0,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    tokio::pin!(wait);
    assert!(wait.as_mut().now_or_never().is_none());
    control.send_snapshot(versioned(AppState::default(), 1));
    assert!(matches!(
        wait.as_mut().now_or_never(),
        Some(Ok(RequestOutcome::SignedOut { .. }))
    ));

    let (mut connection, control) = CoreConnection::new_for_testing(4);
    let request_id = request(12);
    let mut closed = ready_state("@alice:example.invalid");
    closed.navigation.active_room_id = Some("!room:example.invalid".to_owned());
    let wait = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::FocusedContextClosed {
            request_id,
            account_key: AccountKey("@alice:example.invalid".to_owned()),
            room_id: Some("!room:example.invalid".to_owned()),
            allow_projection_only: true,
        },
        0,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    tokio::pin!(wait);
    assert!(wait.as_mut().now_or_never().is_none());
    control.send_snapshot(versioned(closed, 1));
    assert!(matches!(
        wait.as_mut().now_or_never(),
        Some(Ok(RequestOutcome::FocusedContext { .. }))
    ));

    let (mut connection, control) = CoreConnection::new_for_testing(4);
    let request_id = request(13);
    let wait = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::SearchClosed {
            request_id,
            account_key: Some(AccountKey("@alice:example.invalid".to_owned())),
            allow_initial: false,
            allow_projection_only: true,
        },
        0,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    tokio::pin!(wait);
    assert!(wait.as_mut().now_or_never().is_none());
    control.send_snapshot(versioned(ready_state("@alice:example.invalid"), 1));
    assert!(matches!(
        wait.as_mut().now_or_never(),
        Some(Ok(RequestOutcome::Search { .. }))
    ));
}
