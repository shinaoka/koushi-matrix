use super::*;
use crate::event_source::{QaEventFuture, QaEventSource};
use crate::{AppState, CoreFailure};
use koushi_state::{
    SessionAuthenticationMethod, SessionInfo, VerificationAccountKind, VerificationGateState,
    VerificationMethodCapability,
};
use std::collections::VecDeque;

struct Source {
    state: SessionState,
    events: VecDeque<(CoreEvent, SessionState)>,
    commands: usize,
}
impl QaEventSource for Source {
    fn recv_event(&mut self) -> QaEventFuture<'_> {
        Box::pin(async move {
            if self.commands == 0 {
                return std::future::pending().await;
            }
            match self.events.pop_front() {
                Some((event, state)) => {
                    self.state = state;
                    Ok(event)
                }
                None => std::future::pending().await,
            }
        })
    }
}
impl QaSnapshotEventSource for Source {
    fn snapshot(&self) -> AppState {
        AppState {
            session: self.state.clone(),
            ..AppState::default()
        }
    }
}
impl AdmissionSource for Source {
    fn next_request_id(&mut self) -> RequestId {
        id(2)
    }
    fn command(
        &mut self,
        command: CoreCommand,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        assert!(matches!(
            command,
            CoreCommand::Account(AccountCommand::SubmitRecovery { .. })
        ));
        self.commands += 1;
        Box::pin(async { Ok(()) })
    }
}
fn fixture(restore: bool, reverse: bool) -> (Source, AccountKey) {
    let info = SessionInfo {
        homeserver: "https://example.test".into(),
        user_id: "@qa:example.test".into(),
        device_id: "QA".into(),
        authentication_method: SessionAuthenticationMethod::Unknown,
    };
    let key = AccountKey(info.user_id.clone());
    let gate = SessionState::AwaitingVerification {
        info: info.clone(),
        gate: VerificationGateState {
            methods: vec![VerificationMethodCapability::RecoveryKey],
            account_kind: VerificationAccountKind::ExistingIdentity,
            failure: None,
        },
    };
    let terminal = if restore {
        AccountEvent::SessionRestored {
            request_id: id(1),
            account_key: key.clone(),
        }
    } else {
        AccountEvent::LoggedIn {
            request_id: id(1),
            account_key: key.clone(),
        }
    };
    let recovery = AccountEvent::RecoveryCompleted {
        request_id: id(2),
        account_key: key.clone(),
    };
    let mut events = vec![terminal, recovery];
    if reverse {
        events.reverse();
    }
    let mut source = Source {
        state: gate.clone(),
        events: events
            .into_iter()
            .map(|e| (CoreEvent::Account(e), gate.clone()))
            .collect(),
        commands: 0,
    };
    source.events.push_back((
        CoreEvent::Sync(crate::SyncEvent::Running),
        SessionState::Ready(info),
    ));
    (source, key)
}
#[tokio::test(start_paused = true)]
async fn verification_precedes_login_and_both_completions_are_retained() {
    let (mut source, key) = fixture(false, false);
    let result = wait_for_admission(
        &mut source,
        id(1),
        None,
        Some(&AuthSecret::new("synthetic")),
        "test",
    )
    .await;
    assert_eq!(
        result,
        Ok(AdmissionOutcome {
            account_key: key,
            recovered: true
        })
    );
    assert_eq!(source.commands, 1);
    assert!(source.events.is_empty());
}

fn id(sequence: u64) -> RequestId {
    RequestId {
        connection_id: koushi_protocol::ids::RuntimeConnectionId(1),
        sequence,
    }
}

#[tokio::test(start_paused = true)]
async fn login_and_restore_accept_both_completion_orders() {
    for restore in [false, true] {
        for reverse in [false, true] {
            let (mut source, key) = fixture(restore, reverse);
            let result = wait_for_admission(
                &mut source,
                id(1),
                restore.then_some(&key),
                Some(&AuthSecret::new("synthetic")),
                "test",
            )
            .await;
            assert_eq!(
                result,
                Ok(AdmissionOutcome {
                    account_key: key,
                    recovered: true
                })
            );
            assert_eq!(source.commands, 1);
            assert!(source.events.is_empty());
        }
    }
}

#[tokio::test(start_paused = true)]
async fn admission_rejects_mismatched_account_and_failure() {
    let (mut source, _) = fixture(true, false);
    let wrong = AccountKey("@other:example.test".into());
    assert!(
        wait_for_admission(
            &mut source,
            id(1),
            Some(&wrong),
            Some(&AuthSecret::new("synthetic")),
            "test"
        )
        .await
        .unwrap_err()
        .contains("account mismatch")
    );
    for failure_id in [id(1), id(2)] {
        let (mut source, _) = fixture(false, false);
        source.events.front_mut().unwrap().0 = CoreEvent::OperationFailed {
            request_id: failure_id,
            failure: CoreFailure::SessionRequired,
        };
        assert!(
            wait_for_admission(
                &mut source,
                id(1),
                None,
                Some(&AuthSecret::new("synthetic")),
                "test"
            )
            .await
            .unwrap_err()
            .contains("admission failed")
        );
    }
}

#[tokio::test(start_paused = true)]
async fn unsupported_gate_does_not_submit_recovery() {
    let (mut source, _) = fixture(false, false);
    if let SessionState::AwaitingVerification { gate, .. } = &mut source.state {
        gate.methods = vec![VerificationMethodCapability::ExistingDeviceSas];
    }
    assert!(
        wait_for_admission(
            &mut source,
            id(1),
            None,
            Some(&AuthSecret::new("synthetic")),
            "test"
        )
        .await
        .is_err()
    );
    assert_eq!(source.commands, 0);
}

#[tokio::test(start_paused = true)]
async fn unrelated_completion_cannot_satisfy_admission() {
    let (mut source, _) = fixture(false, false);
    source.events.front_mut().unwrap().0 = CoreEvent::Account(AccountEvent::LoggedIn {
        request_id: id(99),
        account_key: AccountKey("@qa:example.test".into()),
    });
    let start = tokio::time::Instant::now();
    assert!(
        wait_for_admission(
            &mut source,
            id(1),
            None,
            Some(&AuthSecret::new("synthetic")),
            "test"
        )
        .await
        .unwrap_err()
        .contains("timed out")
    );
    assert_eq!(start.elapsed(), SYNC_TIMEOUT);
    assert_eq!(source.commands, 1);
}

#[tokio::test(start_paused = true)]
async fn ready_without_recovery_is_not_reported_as_recovered() {
    let (mut source, key) = fixture(false, false);
    source.state = source.events.back().unwrap().1.clone();
    source.events = source
        .events
        .into_iter()
        .filter(|(event, _)| matches!(event, CoreEvent::Account(AccountEvent::LoggedIn { .. })))
        .map(|(event, _)| (event, source.state.clone()))
        .collect();
    source.commands = 1; // This source permits event delivery without a recovery command.
    let outcome = wait_for_admission(&mut source, id(1), None, None, "test")
        .await
        .unwrap();
    assert_eq!(
        outcome,
        AdmissionOutcome {
            account_key: key,
            recovered: false
        }
    );
    assert_eq!(source.commands, 1);
}
