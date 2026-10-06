use crate::config::{EVENT_TIMEOUT, SYNC_TIMEOUT};
use crate::event_source::{QaEventDeadline, QaEventFuture, QaEventSource, QaSnapshotEventSource};
use crate::waiters::{
    ensure_session_restored_account_key, wait_for_logged_out, wait_for_operation_failed,
    wait_for_operation_failed_and_signed_out, wait_for_post_login_ready_snapshot,
    wait_for_ready_or_recovery_required, wait_for_recovery_outcome_until,
    wait_for_recovery_required_after_sync,
};
use crate::{
    AccountEvent, AccountKey, AppState, CoreEvent, CoreFailure, CoreRuntime, RequestId,
    SessionState, SyncEvent,
};
use std::time::Duration;
mod search_plan_tests {
    use crate::config::build_real_homeserver_qa_message_plan;

    #[test]
    fn real_homeserver_qa_search_plan_uses_a_dedicated_unedited_probe_message() {
        let plan = build_real_homeserver_qa_message_plan(1234567890);

        assert!(plan.search_probe_body.contains(&plan.search_token));
        assert_ne!(plan.search_probe_body, plan.msg1_body);
        assert!(!plan.msg1_body.contains(&plan.search_token));
        assert_eq!(plan.msg2_body, "Real homeserver QA message 2");
        assert_eq!(plan.edited_body, "Real homeserver QA message 1 EDITED");
        assert_eq!(plan.reply_body, "Real homeserver QA reply to message 1");
        // The reply body must not carry the search token, so replying does not
        // perturb the later search-probe assertion.
        assert!(!plan.reply_body.contains(&plan.search_token));
    }
}

#[cfg(test)]
mod scenario_tests {
    use crate::config::RealQaScenario;

    #[test]
    fn real_homeserver_qa_scenario_parses_known_names() {
        assert_eq!(
            RealQaScenario::from_env_value(Some("compat".to_owned())).unwrap(),
            RealQaScenario::Compat
        );
        assert_eq!(
            RealQaScenario::from_env_value(Some("space_compat".to_owned())).unwrap(),
            RealQaScenario::SpaceCompat
        );
        assert_eq!(
            RealQaScenario::from_env_value(Some("all".to_owned())).unwrap(),
            RealQaScenario::All
        );
    }

    #[test]
    fn real_homeserver_qa_scenario_defaults_to_space_compat_when_missing() {
        // The default real lane proves space create/link/cleanup, matching the
        // qa:headless-basic:real package script and docs/qa contract.
        assert_eq!(
            RealQaScenario::from_env_value(None).unwrap(),
            RealQaScenario::SpaceCompat
        );
    }

    #[test]
    fn startup_latency_scenario_parses_from_env() {
        assert_eq!(
            RealQaScenario::from_env_value(Some("startup_latency".to_owned())),
            Ok(RealQaScenario::StartupLatency)
        );
    }
}

use std::sync::Arc;

use tempfile::tempdir;
use tokio::time::sleep;

struct ScriptedQaSnapshotEventSource {
    events: std::collections::VecDeque<(CoreEvent, SessionState)>,
    snapshot: AppState,
    received: usize,
}

impl QaEventSource for ScriptedQaSnapshotEventSource {
    fn recv_event(&mut self) -> QaEventFuture<'_> {
        Box::pin(async move {
            match self.events.pop_front() {
                Some((event, session)) => {
                    self.snapshot.session = session;
                    self.received += 1;
                    Ok(event)
                }
                None => std::future::pending().await,
            }
        })
    }
}

impl QaSnapshotEventSource for ScriptedQaSnapshotEventSource {
    fn snapshot(&self) -> AppState {
        self.snapshot.clone()
    }
}

struct IntervalQaEventSource {
    interval: tokio::time::Interval,
}

impl QaEventSource for IntervalQaEventSource {
    fn recv_event(&mut self) -> QaEventFuture<'_> {
        Box::pin(async move {
            self.interval.tick().await;
            Ok(CoreEvent::Sync(SyncEvent::Running))
        })
    }
}

struct IntervalQaSnapshotEventSource {
    interval: tokio::time::Interval,
    snapshot: AppState,
    first_event: Option<CoreEvent>,
}

impl QaEventSource for IntervalQaSnapshotEventSource {
    fn recv_event(&mut self) -> QaEventFuture<'_> {
        Box::pin(async move {
            if let Some(event) = self.first_event.take() {
                return Ok(event);
            }
            self.interval.tick().await;
            Ok(CoreEvent::Sync(SyncEvent::Running))
        })
    }
}

impl QaSnapshotEventSource for IntervalQaSnapshotEventSource {
    fn snapshot(&self) -> AppState {
        self.snapshot.clone()
    }
}

fn qa_state_with_session(session: SessionState) -> AppState {
    AppState {
        session,
        ..AppState::default()
    }
}

fn qa_logged_out_event(request_id: RequestId, account_key: AccountKey) -> CoreEvent {
    CoreEvent::Account(AccountEvent::LoggedOut {
        request_id,
        account_key,
    })
}

fn qa_operation_failed_event(request_id: RequestId) -> CoreEvent {
    CoreEvent::OperationFailed {
        request_id,
        failure: CoreFailure::SessionNotFound,
    }
}

fn qa_state_delta_event() -> CoreEvent {
    CoreEvent::StateDelta(koushi_core::StateDelta {
        generation: 1,
        changed: Box::default(),
    })
}

#[test]
fn session_restored_account_mismatch_is_private_safe() {
    let error = ensure_session_restored_account_key(
        &AccountKey("@unexpected:example.invalid".to_owned()),
        &AccountKey("@expected:example.invalid".to_owned()),
        "restore mismatch",
    )
    .expect_err("wrong restored account must fail immediately");
    assert!(error.contains("account_key mismatch"));
    assert!(!error.contains('@'));
}

#[tokio::test]
async fn logged_out_waiter_requires_event_and_signed_out_snapshot_in_either_order() {
    let request_id = RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
        sequence: 7,
    };
    let account_key = AccountKey("@logout-barrier:example.invalid".to_owned());
    let cases = [
        [
            (
                qa_logged_out_event(request_id, account_key.clone()),
                SessionState::LoggingOut,
            ),
            (qa_state_delta_event(), SessionState::SignedOut),
        ],
        [
            (qa_state_delta_event(), SessionState::SignedOut),
            (
                qa_logged_out_event(request_id, account_key.clone()),
                SessionState::SignedOut,
            ),
        ],
    ];

    for events in cases {
        let mut source = ScriptedQaSnapshotEventSource {
            events: events.into(),
            snapshot: qa_state_with_session(SessionState::LoggingOut),
            received: 0,
        };
        wait_for_logged_out(&mut source, request_id, &account_key, "logout barrier")
            .await
            .expect("both authoritative logout signals should satisfy the barrier");
        assert_eq!(
            source.received, 2,
            "neither event nor snapshot may complete the barrier alone"
        );
    }
}

#[tokio::test]
async fn logged_out_waiter_keeps_wrong_account_and_failure_terminal_and_private_safe() {
    let request_id = RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
        sequence: 8,
    };
    let account_key = AccountKey("@expected:example.invalid".to_owned());
    let mut wrong_account = ScriptedQaSnapshotEventSource {
        events: [(
            qa_logged_out_event(
                request_id,
                AccountKey("@unexpected:example.invalid".to_owned()),
            ),
            SessionState::SignedOut,
        )]
        .into(),
        snapshot: qa_state_with_session(SessionState::LoggingOut),
        received: 0,
    };
    let error = wait_for_logged_out(
        &mut wrong_account,
        request_id,
        &account_key,
        "logout barrier",
    )
    .await
    .expect_err("wrong account must fail immediately");
    assert!(error.contains("account_key mismatch"));
    assert!(!error.contains('@'));
    assert_eq!(wrong_account.received, 1);

    let mut failed = ScriptedQaSnapshotEventSource {
        events: [(
            CoreEvent::OperationFailed {
                request_id,
                failure: CoreFailure::SessionRequired,
            },
            SessionState::SignedOut,
        )]
        .into(),
        snapshot: qa_state_with_session(SessionState::LoggingOut),
        received: 0,
    };
    let error = wait_for_logged_out(&mut failed, request_id, &account_key, "logout barrier")
        .await
        .expect_err("correlated failure must fail immediately");
    assert!(error.contains("SessionRequired"));
    assert_eq!(failed.received, 1);
}

#[tokio::test]
async fn operation_failed_signed_out_waiter_requires_both_signals_in_either_order() {
    let request_id = RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
        sequence: 10,
    };
    let cases = [
        [
            (
                qa_operation_failed_event(request_id),
                SessionState::Restoring,
            ),
            (qa_state_delta_event(), SessionState::SignedOut),
        ],
        [
            (qa_state_delta_event(), SessionState::SignedOut),
            (
                qa_operation_failed_event(request_id),
                SessionState::SignedOut,
            ),
        ],
    ];

    for events in cases {
        let mut source = ScriptedQaSnapshotEventSource {
            events: events.into(),
            snapshot: qa_state_with_session(SessionState::Restoring),
            received: 0,
        };
        let failure = wait_for_operation_failed_and_signed_out(
            &mut source,
            request_id,
            "restore cleanup barrier",
        )
        .await
        .expect("both authoritative cleanup signals should satisfy the barrier");
        assert_eq!(failure, CoreFailure::SessionNotFound);
        assert_eq!(
            source.received, 2,
            "neither failure nor SignedOut may complete the barrier alone"
        );
    }

    let mut succeeded = ScriptedQaSnapshotEventSource {
        events: [(
            CoreEvent::Account(AccountEvent::SessionRestored {
                request_id,
                account_key: AccountKey("@private:example.invalid".to_owned()),
            }),
            SessionState::SignedOut,
        )]
        .into(),
        snapshot: qa_state_with_session(SessionState::SignedOut),
        received: 0,
    };
    let error = wait_for_operation_failed_and_signed_out(
        &mut succeeded,
        request_id,
        "restore cleanup barrier",
    )
    .await
    .expect_err("a same-request success terminal must fail immediately");
    assert!(error.contains("operation succeeded"));
    assert!(!error.contains('@'));
    assert_eq!(succeeded.received, 1);
}

#[tokio::test(start_paused = true)]
async fn operation_failed_signed_out_deadline_survives_unrelated_event_starvation() {
    let request_id = RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
        sequence: 11,
    };
    let mut source = IntervalQaSnapshotEventSource {
        interval: tokio::time::interval(Duration::from_secs(1)),
        snapshot: qa_state_with_session(SessionState::Restoring),
        first_event: Some(qa_operation_failed_event(request_id)),
    };
    let started_at = tokio::time::Instant::now();
    wait_for_operation_failed_and_signed_out(&mut source, request_id, "restore cleanup deadline")
        .await
        .expect_err("unrelated events must not restart the cleanup deadline");
    assert_eq!(
        tokio::time::Instant::now().duration_since(started_at),
        EVENT_TIMEOUT
    );
}

#[tokio::test(start_paused = true)]
async fn ready_recovery_deadline_survives_unrelated_event_starvation() {
    let mut source = IntervalQaSnapshotEventSource {
        interval: tokio::time::interval(Duration::from_secs(1)),
        snapshot: qa_state_with_session(SessionState::SignedOut),
        first_event: None,
    };
    let deadline = QaEventDeadline::after(SYNC_TIMEOUT);
    let started_at = tokio::time::Instant::now();
    tokio::time::timeout(
        SYNC_TIMEOUT + Duration::from_secs(1),
        wait_for_ready_or_recovery_required(&mut source, deadline, "ready recovery deadline"),
    )
    .await
    .expect("the ready/recovery waiter must enforce its own absolute deadline")
    .expect_err("unrelated events must not restart the ready/recovery deadline");
    assert_eq!(
        tokio::time::Instant::now().duration_since(started_at),
        SYNC_TIMEOUT
    );

    let request_id = RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
        sequence: 12,
    };
    let mut recovery_source = IntervalQaEventSource {
        interval: tokio::time::interval(Duration::from_secs(1)),
    };
    let shared_deadline = QaEventDeadline::after(SYNC_TIMEOUT);
    let nested_started_at = tokio::time::Instant::now();
    tokio::time::advance(Duration::from_secs(30)).await;
    let recovery_result = wait_for_recovery_outcome_until(
        &mut recovery_source,
        request_id,
        "nested recovery deadline",
        shared_deadline,
    )
    .await;
    assert!(
        recovery_result.is_err(),
        "nested recovery waiting must consume the shared remaining budget"
    );
    assert_eq!(
        tokio::time::Instant::now().duration_since(nested_started_at),
        SYNC_TIMEOUT
    );
}

#[tokio::test(start_paused = true)]
async fn logout_and_operation_failed_deadlines_survive_unrelated_event_starvation() {
    let request_id = RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
        sequence: 9,
    };
    let account_key = AccountKey("@deadline:example.invalid".to_owned());
    let mut logout_source = IntervalQaSnapshotEventSource {
        interval: tokio::time::interval(Duration::from_secs(1)),
        snapshot: qa_state_with_session(SessionState::LoggingOut),
        first_event: Some(qa_logged_out_event(request_id, account_key.clone())),
    };
    let logout_started_at = tokio::time::Instant::now();
    tokio::time::timeout(
        EVENT_TIMEOUT + Duration::from_secs(1),
        wait_for_logged_out(
            &mut logout_source,
            request_id,
            &account_key,
            "logout deadline",
        ),
    )
    .await
    .expect("the logout waiter must enforce its own absolute deadline")
    .expect_err("a LoggedOut event without SignedOut state must time out");
    assert_eq!(
        tokio::time::Instant::now().duration_since(logout_started_at),
        EVENT_TIMEOUT
    );

    let mut failure_source = IntervalQaEventSource {
        interval: tokio::time::interval(Duration::from_secs(1)),
    };
    let failure_started_at = tokio::time::Instant::now();
    tokio::time::timeout(
        EVENT_TIMEOUT + Duration::from_secs(1),
        wait_for_operation_failed(&mut failure_source, request_id, "failure deadline"),
    )
    .await
    .expect("the failure waiter must enforce its own absolute deadline")
    .expect_err("unrelated events must not restart the failure deadline");
    assert_eq!(
        tokio::time::Instant::now().duration_since(failure_started_at),
        EVENT_TIMEOUT
    );
}

#[tokio::test]
async fn recovery_gate_waits_for_late_authoritative_recovery_required_after_ready_event() {
    let info = koushi_state::SessionInfo {
        homeserver: "https://example.test".to_owned(),
        user_id: "@alice:example.test".to_owned(),
        device_id: "DEVICE1".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    };
    let ready = qa_state_with_session(SessionState::Ready(info.clone()));
    let mut source = ScriptedQaSnapshotEventSource {
        events: [
            (qa_state_delta_event(), ready.session.clone()),
            (
                CoreEvent::Account(AccountEvent::RecoveryRequired {
                    account_key: AccountKey(info.user_id),
                }),
                ready.session.clone(),
            ),
        ]
        .into(),
        snapshot: ready,
        received: 0,
    };

    wait_for_recovery_required_after_sync(&mut source, "gate")
        .await
        .expect("the gate should accept the late authoritative recovery event");
    assert_eq!(
        source.received, 2,
        "the Ready event alone must not satisfy the recovery gate"
    );
}

#[tokio::test]
async fn post_login_ready_gate_waits_for_late_ready_snapshot_before_sync() {
    let data_dir = tempdir().unwrap();
    let runtime = Arc::new(CoreRuntime::start_with_data_dir(
        data_dir.path().to_path_buf(),
    ));
    let mut conn = runtime.attach();

    let runtime2 = Arc::clone(&runtime);
    let delayed = tokio::spawn(async move {
        sleep(Duration::from_millis(50)).await;
        let attempt_id = koushi_state::LoginAttemptId::new(1, 2);
        runtime2
            .inject_actions(vec![
                koushi_state::AppAction::AuthenticationStarted {
                    attempt_id,
                    homeserver: "https://example.test".to_owned(),
                },
                koushi_state::AppAction::LoginSucceeded {
                    attempt_id,
                    info: koushi_state::SessionInfo {
                        homeserver: "https://example.test".to_owned(),
                        user_id: "@alice:example.test".to_owned(),
                        device_id: "DEVICE1".to_owned(),
                        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
                    },
                },
                koushi_state::AppAction::CurrentDeviceTrustChanged(
                    koushi_state::CurrentDeviceTrustState::Verified,
                ),
            ])
            .await;
    });

    let result = wait_for_post_login_ready_snapshot(&mut conn, "post-login gate").await;
    delayed.await.expect("delayed injector");

    assert!(result.is_ok());
    assert!(matches!(conn.snapshot().session, SessionState::Ready(_)));
}
