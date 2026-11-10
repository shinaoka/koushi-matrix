use super::*;
use futures_util::FutureExt;
use koushi_protocol::command::EventNavigationMissingTargetPolicy;
use koushi_protocol::event::{CoreEvent, IntentNoOpReason, IntentOutcome};
use koushi_protocol::ids::{RequestId, RuntimeConnectionId};
use koushi_protocol::state_update::VersionedAppStateSnapshot;
use koushi_state::{AppState, EventNavigationSource, EventNavigationState};
use std::time::Duration;

#[tokio::test]
async fn matching_operation_failure_rejects_event_navigation_waiter_promptly() {
    let (mut connection, mut control) = CoreConnection::new_for_testing(4);
    let mut waiter = Box::pin(connection.navigate_to_event_and_wait(
        "!room:example.invalid".to_owned(),
        "$event:example.invalid".to_owned(),
        EventNavigationSource::Activity,
        EventNavigationMissingTargetPolicy::LiveFallback,
        Duration::from_secs(1),
    ));
    assert!(waiter.as_mut().now_or_never().is_none());
    let request_id = control
        .recv_command()
        .await
        .expect("navigation command")
        .request_id();

    control.send_event(CoreEvent::OperationFailed {
        request_id,
        failure: koushi_protocol::failure::CoreFailure::SessionRequired,
    });

    assert_eq!(waiter.await, Err(EventNavigationError::Rejected));
}

#[tokio::test]
async fn displaced_event_navigation_waiter_accepts_matching_superseded_lifecycle() {
    let (mut connection, mut control) = CoreConnection::new_for_testing(4);
    let expected = connection.versioned_snapshot();
    let mut waiter = Box::pin(connection.navigate_to_event_and_wait(
        "!room:example.invalid".to_owned(),
        "$event:example.invalid".to_owned(),
        EventNavigationSource::Activity,
        EventNavigationMissingTargetPolicy::LiveFallback,
        Duration::from_secs(1),
    ));
    assert!(waiter.as_mut().now_or_never().is_none());
    let request_id = control
        .recv_command()
        .await
        .expect("navigation command")
        .request_id();

    control.send_event(CoreEvent::IntentLifecycle {
        request_id,
        outcome: IntentOutcome::BenignNoOp(IntentNoOpReason::Superseded),
        published_generation: expected.generation,
    });

    assert_eq!(waiter.await, Ok(expected));
}

/// #1854: a target that is a reply inside a thread is carried by the thread
/// pane, so its navigation commits without `Anchored`/`LiveFallback`. The waiter
/// must accept that terminal instead of waiting for its deadline.
#[tokio::test]
async fn thread_routed_event_navigation_waiter_accepts_matching_committed_lifecycle() {
    let (mut connection, mut control) = CoreConnection::new_for_testing(4);
    let expected = connection.versioned_snapshot();
    let mut waiter = Box::pin(connection.navigate_to_event_and_wait(
        "!room:example.invalid".to_owned(),
        "$event:example.invalid".to_owned(),
        EventNavigationSource::Search,
        EventNavigationMissingTargetPolicy::LiveFallback,
        Duration::from_secs(1),
    ));
    assert!(waiter.as_mut().now_or_never().is_none());
    let request_id = control
        .recv_command()
        .await
        .expect("navigation command")
        .request_id();

    control.send_event(CoreEvent::IntentLifecycle {
        request_id,
        outcome: IntentOutcome::Committed,
        published_generation: expected.generation,
    });

    assert_eq!(waiter.await, Ok(expected));
}

#[tokio::test]
async fn unrelated_event_navigation_lifecycle_does_not_settle_waiter() {
    let (mut connection, mut control) = CoreConnection::new_for_testing(4);
    let mut waiter = Box::pin(connection.navigate_to_event_and_wait(
        "!room:example.invalid".to_owned(),
        "$event:example.invalid".to_owned(),
        EventNavigationSource::Activity,
        EventNavigationMissingTargetPolicy::LiveFallback,
        Duration::from_millis(20),
    ));
    assert!(waiter.as_mut().now_or_never().is_none());
    let request_id = control
        .recv_command()
        .await
        .expect("navigation command")
        .request_id();

    let unrelated_request_id = RequestId {
        sequence: request_id.sequence + 1,
        ..request_id
    };
    control.send_event(CoreEvent::OperationFailed {
        request_id: unrelated_request_id,
        failure: koushi_protocol::failure::CoreFailure::SessionRequired,
    });
    control.send_event(CoreEvent::IntentLifecycle {
        request_id: unrelated_request_id,
        outcome: IntentOutcome::BenignNoOp(IntentNoOpReason::Superseded),
        published_generation: 0,
    });

    assert!(waiter.as_mut().now_or_never().is_none());
    assert_eq!(waiter.await, Err(EventNavigationError::Timeout));
}

#[tokio::test]
async fn lagged_event_navigation_rechecks_the_authoritative_snapshot() {
    let (mut connection, mut control) = CoreConnection::new_for_testing(1);
    let mut waiter = Box::pin(connection.navigate_to_event_and_wait(
        "!room:example.invalid".to_owned(),
        "$event:example.invalid".to_owned(),
        EventNavigationSource::Activity,
        EventNavigationMissingTargetPolicy::LiveFallback,
        Duration::from_secs(1),
    ));
    assert!(waiter.as_mut().now_or_never().is_none());
    let _request = control
        .recv_command()
        .await
        .expect("navigation command")
        .request_id();

    for sequence in 1..=2 {
        control.send_event(CoreEvent::IntentLifecycle {
            request_id: RequestId {
                connection_id: RuntimeConnectionId(99),
                sequence,
            },
            outcome: IntentOutcome::BenignNoOp(IntentNoOpReason::Superseded),
            published_generation: 0,
        });
    }
    let mut state = AppState::default();
    state.navigation.event_navigation = EventNavigationState::Opening {
        generation: 2,
        source: EventNavigationSource::Search,
    };
    let expected = VersionedAppStateSnapshot {
        generation: 7,
        state,
    };
    control.send_snapshot(expected.clone());

    assert_eq!(waiter.await, Ok(expected));
}

/// #1854: the thread route settles the navigation as `Idle` and never publishes
/// `Anchored`/`LiveFallback`, so the pane is the only snapshot fact that carries
/// the outcome. A waiter that misses the `Committed` broadcast must recover that
/// outcome from the snapshot instead of timing out.
#[tokio::test]
async fn a_lagged_thread_navigation_waiter_recovers_its_outcome_from_the_snapshot() {
    let (mut connection, mut control) = CoreConnection::new_for_testing(1);
    let mut waiter = Box::pin(connection.navigate_to_event_and_wait(
        "!thread-room:example.invalid".to_owned(),
        "$thread-reply:example.invalid".to_owned(),
        EventNavigationSource::ReplyQuote,
        EventNavigationMissingTargetPolicy::Fail,
        Duration::from_secs(1),
    ));
    assert!(waiter.as_mut().now_or_never().is_none());
    let request_id = control
        .recv_command()
        .await
        .expect("navigation command")
        .request_id();

    // The navigation's own terminal is published first and then dropped by the
    // lag: no `Committed` lifecycle is ever read.
    control.send_event(CoreEvent::IntentLifecycle {
        request_id,
        outcome: IntentOutcome::Committed,
        published_generation: 6,
    });
    control.send_event(CoreEvent::IntentLifecycle {
        request_id: RequestId {
            connection_id: RuntimeConnectionId(99),
            sequence: 1,
        },
        outcome: IntentOutcome::BenignNoOp(IntentNoOpReason::Superseded),
        published_generation: 6,
    });

    let state = AppState {
        thread: koushi_state::ThreadPaneState::Opening {
            room_id: "!thread-room:example.invalid".to_owned(),
            root_event_id: "$thread-root:example.invalid".to_owned(),
            intent: koushi_state::ThreadOpenIntent::PinnedReply {
                event_id: "$thread-reply:example.invalid".to_owned(),
            },
        },
        ..AppState::default()
    };
    let expected = VersionedAppStateSnapshot {
        generation: 7,
        state,
    };
    control.send_snapshot(expected.clone());

    assert_eq!(waiter.await, Ok(expected));
}

/// #1854: a failed thread open is a failure, not a silent success or a timeout.
#[tokio::test]
async fn a_failed_thread_pane_fails_its_navigation_waiter() {
    let (mut connection, mut control) = CoreConnection::new_for_testing(1);
    let mut waiter = Box::pin(connection.navigate_to_event_and_wait(
        "!thread-room:example.invalid".to_owned(),
        "$thread-reply:example.invalid".to_owned(),
        EventNavigationSource::ReplyQuote,
        EventNavigationMissingTargetPolicy::Fail,
        Duration::from_secs(1),
    ));
    assert!(waiter.as_mut().now_or_never().is_none());
    let _request = control
        .recv_command()
        .await
        .expect("navigation command")
        .request_id();

    let state = AppState {
        thread: koushi_state::ThreadPaneState::Failed {
            room_id: "!thread-room:example.invalid".to_owned(),
            root_event_id: "$thread-root:example.invalid".to_owned(),
            intent: koushi_state::ThreadOpenIntent::PinnedReply {
                event_id: "$thread-reply:example.invalid".to_owned(),
            },
            failure_kind: koushi_state::OperationFailureKind::Network,
        },
        ..AppState::default()
    };
    control.send_snapshot(VersionedAppStateSnapshot {
        generation: 7,
        state,
    });

    assert_eq!(
        waiter.await,
        Err(EventNavigationError::Failed(
            koushi_state::EventNavigationFailureKind::Timeline
        ))
    );
}
