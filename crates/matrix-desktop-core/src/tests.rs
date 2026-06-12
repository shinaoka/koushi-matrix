//! Phase 1 contract tests: redaction, unauthenticated rejection, request-id
//! correlation, snapshot coalescing, queue overflow.

use std::time::Duration;

use matrix_desktop_state::{
    AppAction, AuthSecret, LoginRequest, RecoveryRequest, SessionInfo, SessionState,
};

use crate::command::{AccountCommand, CoreCommand, RoomCommand, SearchCommand, TimelineCommand};
use crate::event::{CoreEvent, PaginationDirection};
use crate::failure::CoreFailure;
use crate::ids::{AccountKey, RequestId, RuntimeConnectionId, TimelineKey};
use crate::runtime::{CommandSubmitError, CoreRuntime};
use crate::executor;

const PASSWORD: &str = "p4ssw0rd-very-secret";
const RECOVERY: &str = "EsT1 RcVy KeyM ater";
const BODY: &str = "private message body 機密本文";
const QUERY: &str = "secret search terms";

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://example.test".to_owned(),
        user_id: "@alice:example.test".to_owned(),
        device_id: "DEVICE1".to_owned(),
    }
}

fn fake_request_id() -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(999),
        sequence: 1,
    }
}

#[test]
fn secret_bearing_commands_redact_debug() {
    let login = CoreCommand::Account(AccountCommand::LoginPassword {
        request_id: fake_request_id(),
        request: LoginRequest {
            homeserver: "https://example.test".to_owned(),
            username: "alice-login-name".to_owned(),
            password: AuthSecret::new(PASSWORD),
            device_display_name: Some("Alice Laptop".to_owned()),
        },
    });
    let recovery = CoreCommand::Account(AccountCommand::SubmitRecovery {
        request_id: fake_request_id(),
        request: RecoveryRequest {
            secret: AuthSecret::new(RECOVERY),
        },
    });
    let key = TimelineKey::room(AccountKey("acc".to_owned()), "!room:example.test");
    let send = CoreCommand::Timeline(TimelineCommand::SendText {
        request_id: fake_request_id(),
        key: key.clone(),
        transaction_id: "txn-1".to_owned(),
        body: BODY.to_owned(),
    });
    let edit = CoreCommand::Timeline(TimelineCommand::EditText {
        request_id: fake_request_id(),
        key: key.clone(),
        event_id: "$evt".to_owned(),
        body: BODY.to_owned(),
    });
    let search = CoreCommand::Search(SearchCommand::Query {
        request_id: fake_request_id(),
        query: QUERY.to_owned(),
        scope: crate::command::SearchScope::Global,
    });

    for (command, secrets) in [
        (&login, vec![PASSWORD, "alice-login-name", "Alice Laptop"]),
        (&recovery, vec![RECOVERY]),
        (&send, vec![BODY]),
        (&edit, vec![BODY]),
        (&search, vec![QUERY]),
    ] {
        let debug = format!("{command:?}");
        for secret in secrets {
            assert!(
                !debug.contains(secret),
                "Debug output leaked a secret: {debug}"
            );
        }
    }
    // Non-secret correlation data stays visible.
    assert!(format!("{send:?}").contains("txn-1"));
}

#[tokio::test]
async fn unauthenticated_session_commands_are_rejected() {
    let runtime = CoreRuntime::start();
    let mut connection = runtime.attach();

    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Room(RoomCommand::CreateRoom {
            request_id,
            name: "qa room".to_owned(),
        }))
        .await
        .expect("submit");

    match connection.recv_event().await.expect("event") {
        CoreEvent::OperationFailed {
            request_id: failed_id,
            failure,
        } => {
            assert_eq!(failed_id, request_id);
            assert_eq!(failure, CoreFailure::SessionRequired);
        }
        other => panic!("expected OperationFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn ready_session_routes_past_session_gate() {
    let runtime = CoreRuntime::start();
    let mut connection = runtime.attach();
    runtime
        .inject_actions(vec![AppAction::RestoreSessionSucceeded(session_info())])
        .await;
    // Wait for the Ready snapshot before submitting.
    loop {
        if matches!(connection.snapshot().session, SessionState::Ready(_)) {
            break;
        }
        executor::sleep(Duration::from_millis(5)).await;
    }

    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Timeline(TimelineCommand::Paginate {
            request_id,
            key: TimelineKey::room(AccountKey("acc".to_owned()), "!room:example.test"),
            direction: PaginationDirection::Backward,
            event_count: 20,
        }))
        .await
        .expect("submit");

    loop {
        match connection.recv_event().await.expect("event") {
            CoreEvent::OperationFailed {
                request_id: failed_id,
                failure,
            } if failed_id == request_id => {
                // Phase 1 has no TimelineActor yet; the point is that the
                // failure is NOT SessionRequired once the session is Ready.
                assert_ne!(failure, CoreFailure::SessionRequired);
                return;
            }
            _ => continue,
        }
    }
}

#[tokio::test]
async fn mismatched_request_id_fails_locally_without_publishing() {
    let runtime = CoreRuntime::start();
    let intruder = runtime.attach();
    let mut observer = runtime.attach();

    let foreign_id = observer.next_request_id();
    let result = intruder
        .command(CoreCommand::Room(RoomCommand::JoinRoom {
            request_id: foreign_id,
            room_id: "!room:example.test".to_owned(),
        }))
        .await;
    assert_eq!(result, Err(CommandSubmitError::InvalidRequestId));

    // No CoreEvent may be published with the forged RequestId.
    let outcome = executor::timeout(Duration::from_millis(100), observer.recv_event()).await;
    assert!(
        outcome.is_err(),
        "no event should be published for a rejected submission"
    );
}

#[tokio::test]
async fn result_events_correlate_in_submission_order() {
    let runtime = CoreRuntime::start();
    let mut connection = runtime.attach();

    let first = connection.next_request_id();
    let second = connection.next_request_id();
    assert_ne!(first, second);

    for request_id in [first, second] {
        connection
            .command(CoreCommand::Room(RoomCommand::JoinRoom {
                request_id,
                room_id: "!room:example.test".to_owned(),
            }))
            .await
            .expect("submit");
    }

    let mut seen = Vec::new();
    while seen.len() < 2 {
        if let CoreEvent::OperationFailed { request_id, .. } =
            connection.recv_event().await.expect("event")
        {
            seen.push(request_id);
        }
    }
    assert_eq!(seen, vec![first, second], "events must be ordered");
}

#[tokio::test]
async fn reducer_actions_coalesce_into_single_state_changed() {
    let runtime = CoreRuntime::start();
    let mut connection = runtime.attach();

    runtime
        .inject_actions(vec![
            AppAction::AppStarted,
            AppAction::RestoreSessionFailed {
                message: "synthetic".to_owned(),
            },
            AppAction::LoginDiscoveryRequested {
                homeserver: "https://example.test".to_owned(),
            },
        ])
        .await;

    let mut state_changed_count = 0;
    let mut last = None;
    // Drain everything emitted within a quiet period.
    while let Ok(Ok(event)) =
        executor::timeout(Duration::from_millis(200), connection.recv_event()).await
    {
        if let CoreEvent::StateChanged(snapshot) = event {
            state_changed_count += 1;
            last = Some(snapshot);
        }
    }

    assert_eq!(
        state_changed_count, 1,
        "one batch of actions must coalesce into exactly one StateChanged"
    );
    let last = last.expect("snapshot");
    // The final state reflects the LAST action in the batch.
    assert!(matches!(
        last.auth,
        matrix_desktop_state::AuthDiscoveryState::Discovering { .. }
    ));
    assert_eq!(connection.snapshot(), last);
}

#[tokio::test]
async fn slow_consumer_observes_lag_and_recovers_via_snapshot() {
    let runtime = CoreRuntime::start_with_event_capacity(4);
    let pump = runtime.attach();
    let mut slow = runtime.attach();

    // Overflow the slow consumer's bounded queue.
    for _ in 0..32 {
        let request_id = pump.next_request_id();
        pump.command(CoreCommand::Room(RoomCommand::JoinRoom {
            request_id,
            room_id: "!room:example.test".to_owned(),
        }))
        .await
        .expect("submit");
    }
    runtime.inject_actions(vec![AppAction::AppStarted]).await;
    executor::sleep(Duration::from_millis(100)).await;

    let first = slow.recv_event().await;
    assert!(first.is_err(), "slow consumer must observe the lag marker");

    // Recovery path: latest-wins snapshot is intact and current.
    assert!(matches!(
        slow.snapshot().session,
        SessionState::Restoring | SessionState::SignedOut
    ));
}
