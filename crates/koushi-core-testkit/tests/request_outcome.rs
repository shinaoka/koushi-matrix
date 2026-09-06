use futures_util::FutureExt;
use koushi_core::runtime::request_outcome::{
    OutcomeCorrelation, RequestOutcome, RequestOutcomeError, RequestOutcomeExpectation,
};
use koushi_core::{
    AccountKey, CoreConnection, IntentNoOpReason, RequestId, RuntimeConnectionId, TimelineKey,
};
use koushi_protocol::event::{AccountEvent, CoreEvent, RoomEvent, TimelineEvent};
use koushi_state::{AppState, ComposerTarget, SessionInfo, SessionState};
use std::time::Duration;

fn request(connection_id: u64, sequence: u64) -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(connection_id),
        sequence,
    }
}

fn account_state(user_id: &str) -> AppState {
    let mut state = AppState::default();
    state.session = SessionState::Ready(SessionInfo {
        homeserver: "https://example.invalid".to_owned(),
        user_id: user_id.to_owned(),
        device_id: "device".to_owned(),
        authentication_method: Default::default(),
    });
    state
}

fn room_summary(room_id: &str) -> koushi_state::RoomSummary {
    koushi_state::RoomSummary {
        room_id: room_id.to_owned(),
        display_name: "Room".to_owned(),
        display_label: "Room".to_owned(),
        original_display_label: "Room".to_owned(),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: Default::default(),
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        marked_unread: false,
        recency_stamp: None,
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 0,
    }
}

fn selected_snapshot(
    room_id: &str,
    generation: u64,
) -> koushi_protocol::state_update::VersionedAppStateSnapshot {
    let mut state = AppState::default();
    state.navigation.active_room_id = Some(room_id.to_owned());
    koushi_protocol::state_update::VersionedAppStateSnapshot { generation, state }
}

#[tokio::test]
async fn event_before_projection_waits_for_authoritative_snapshot_and_returns_generation() {
    let (mut connection, control) = CoreConnection::new_for_testing(8);
    let request_id = request(700, 1);
    let room_id = "!created:example.invalid";
    let mut state = account_state("@alice:example.invalid");
    state.rooms.push(room_summary(room_id));
    let published = koushi_protocol::state_update::VersionedAppStateSnapshot {
        generation: 9,
        state,
    };
    let waiter = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::RoomCreated {
            request_id,
            account_key: AccountKey("@alice:example.invalid".to_owned()),
        },
        2,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    let mut waiter = Box::pin(waiter);
    control.send_event(CoreEvent::Room(RoomEvent::RoomCreated {
        request_id,
        room_id: room_id.to_owned(),
    }));
    assert!(matches!(waiter.as_mut().now_or_never(), None));
    control.send_snapshot(published.clone());
    assert_eq!(
        waiter.await.expect("room creation outcome"),
        RequestOutcome::RoomCreated {
            request_id,
            room_id: room_id.to_owned(),
            generation: published.generation,
        }
    );
    assert_eq!(connection.versioned_snapshot(), published);
}

#[tokio::test]
async fn initial_snapshot_can_satisfy_idempotent_room_selection() {
    let (mut connection, control) = CoreConnection::new_for_testing(4);
    let room_id = "!already-active:example.invalid";
    control.send_snapshot(selected_snapshot(room_id, 0));
    let outcome = connection
        .wait_for_request_outcome(
            OutcomeCorrelation::Request(request(1, 1)),
            RequestOutcomeExpectation::RoomSelected {
                request_id: request(1, 1),
                room_id: room_id.to_owned(),
                account_key: None,
                allow_initial: true,
            },
            0,
            tokio::time::Instant::now() + Duration::from_secs(1),
        )
        .await;
    assert!(
        matches!(&outcome, Ok(RequestOutcome::RoomSelected { generation }) if *generation == 0_u64),
        "unexpected outcome: {outcome:?}"
    );
}

#[tokio::test]
async fn baseline_generation_fences_projection_until_newer_snapshot() {
    let (mut connection, control) = CoreConnection::new_for_testing(4);
    let request_id = request(2, 1);
    let room_id = "!baseline:example.invalid";
    let mut state = account_state("@alice:example.invalid");
    state.rooms.push(room_summary(room_id));
    control.send_snapshot(koushi_protocol::state_update::VersionedAppStateSnapshot {
        generation: 3,
        state: state.clone(),
    });
    let waiter = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::RoomCreated {
            request_id,
            account_key: AccountKey("@alice:example.invalid".to_owned()),
        },
        3,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    let mut waiter = Box::pin(waiter);
    assert!(waiter.as_mut().now_or_never().is_none());
    control.send_event(CoreEvent::Room(RoomEvent::RoomCreated {
        request_id,
        room_id: room_id.to_owned(),
    }));
    let published = koushi_protocol::state_update::VersionedAppStateSnapshot {
        generation: 4,
        state,
    };
    control.send_snapshot(published.clone());
    assert!(matches!(
        waiter.await,
        Ok(RequestOutcome::RoomCreated { generation, .. }) if generation == 4
    ));
}

#[tokio::test]
async fn unrelated_and_foreign_request_ids_are_ignored_on_a_separate_wait_connection() {
    let (submitter, _submitter_control) = CoreConnection::new_for_testing(4);
    let (mut waiter, waiter_control) = CoreConnection::new_for_testing(4);
    let request_id = submitter.next_request_id();
    let foreign = request(999, request_id.sequence);
    let operation = waiter.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::OidcAuthorization { request_id },
        0,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    tokio::pin!(operation);
    waiter_control.send_event(CoreEvent::Account(AccountEvent::OidcAuthorizationCreated {
        request_id: foreign,
        authorization_url: "https://example.invalid/authorize".to_owned(),
        state: "state".to_owned(),
    }));
    assert!(operation.as_mut().now_or_never().is_none());
    waiter_control.send_event(CoreEvent::Account(AccountEvent::OidcAuthorizationCreated {
        request_id,
        authorization_url: "https://example.invalid/authorize".to_owned(),
        state: "state".to_owned(),
    }));
    assert!(matches!(
        operation.await,
        Ok(RequestOutcome::OidcAuthorization { .. })
    ));
}

#[tokio::test]
async fn wrong_target_and_operation_failure_do_not_settle_success() {
    let (mut connection, control) = CoreConnection::new_for_testing(4);
    let request_id = request(3, 1);
    let expected = "!expected:example.invalid";
    let operation = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::RoomJoined {
            request_id,
            account_key: AccountKey("@alice:example.invalid".to_owned()),
            room_id: expected.to_owned(),
        },
        0,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    tokio::pin!(operation);
    control.send_event(CoreEvent::Room(RoomEvent::RoomJoined {
        request_id,
        room_id: "!wrong:example.invalid".to_owned(),
    }));
    assert!(operation.as_mut().now_or_never().is_none());
    control.send_event(CoreEvent::OperationFailed {
        request_id,
        failure: koushi_core::CoreFailure::SessionRequired,
    });
    assert_eq!(
        operation.await,
        Err(RequestOutcomeError::OperationFailed {
            failure: koushi_core::CoreFailure::SessionRequired,
        })
    );
}

#[tokio::test]
async fn failed_noop_is_typed() {
    let (mut connection, control) = CoreConnection::new_for_testing(4);
    let request_id = request(4, 1);
    let operation = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::RoomSelected {
            request_id,
            room_id: "!room:example.invalid".to_owned(),
            account_key: None,
            allow_initial: false,
        },
        0,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    tokio::pin!(operation);
    control.send_event(CoreEvent::IntentLifecycle {
        request_id,
        outcome: koushi_core::IntentOutcome::FailedNoOp(IntentNoOpReason::RoomNotInState),
        published_generation: 0,
    });
    assert_eq!(
        operation.await,
        Err(RequestOutcomeError::FailedNoOp {
            reason: IntentNoOpReason::RoomNotInState,
        })
    );
}

#[tokio::test]
async fn lag_policy_can_continue_or_finish_as_terminal_lag() {
    let (mut connection, control) = CoreConnection::new_for_testing(1);
    let request_id = request(5, 1);
    let operation = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::RoomJoined {
            request_id,
            account_key: AccountKey("@alice:example.invalid".to_owned()),
            room_id: "!room:example.invalid".to_owned(),
        },
        0,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    tokio::pin!(operation);
    for sequence in 2..=4 {
        control.send_event(CoreEvent::OperationFailed {
            request_id: request(5, sequence),
            failure: koushi_core::CoreFailure::SessionRequired,
        });
    }
    assert!(operation.as_mut().now_or_never().is_none());
    control.send_event(CoreEvent::Room(RoomEvent::RoomJoined {
        request_id,
        room_id: "!room:example.invalid".to_owned(),
    }));
    let mut joined_state = account_state("@alice:example.invalid");
    joined_state
        .rooms
        .push(room_summary("!room:example.invalid"));
    control.send_snapshot(koushi_protocol::state_update::VersionedAppStateSnapshot {
        generation: 1,
        state: joined_state,
    });
    assert!(matches!(
        operation.await,
        Ok(RequestOutcome::RoomJoined { .. })
    ));

    let (mut connection, control) = CoreConnection::new_for_testing(1);
    let request_id = request(6, 1);
    let operation = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::OidcAuthorization { request_id },
        0,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    tokio::pin!(operation);
    for sequence in 2..=4 {
        control.send_event(CoreEvent::OperationFailed {
            request_id: request(6, sequence),
            failure: koushi_core::CoreFailure::SessionRequired,
        });
    }
    assert_eq!(operation.await, Err(RequestOutcomeError::Lagged));
}

#[tokio::test]
async fn disconnect_and_timeout_perform_final_snapshot_check() {
    let (mut connection, control) = CoreConnection::new_for_testing(4);
    let request_id = request(7, 1);
    let room_id = "!final:example.invalid";
    let operation = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::RoomSelected {
            request_id,
            room_id: room_id.to_owned(),
            account_key: None,
            allow_initial: false,
        },
        0,
        tokio::time::Instant::now() + Duration::from_millis(20),
    );
    tokio::pin!(operation);
    control.send_snapshot(selected_snapshot(room_id, 1));
    drop(control);
    assert!(
        matches!(operation.await, Ok(RequestOutcome::RoomSelected { generation }) if generation == 1)
    );

    let (mut connection, control) = CoreConnection::new_for_testing(4);
    let operation = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request(8, 1)),
        RequestOutcomeExpectation::RoomSelected {
            request_id: request(8, 1),
            room_id: "!timeout:example.invalid".to_owned(),
            account_key: None,
            allow_initial: false,
        },
        0,
        tokio::time::Instant::now() + Duration::from_millis(1),
    );
    tokio::pin!(operation);
    tokio::time::sleep(Duration::from_millis(5)).await;
    control.send_snapshot(selected_snapshot("!timeout:example.invalid", 2));
    assert!(
        matches!(operation.await, Ok(RequestOutcome::RoomSelected { generation }) if generation == 2)
    );
}

#[tokio::test]
async fn submission_requires_both_request_and_submission_and_transaction_correlations() {
    let (mut connection, control) = CoreConnection::new_for_testing(4);
    let request_id = request(9, 1);
    let submission_id = koushi_state::SubmissionId::new("submission-test");
    let mismatch = connection.wait_for_request_outcome(
        OutcomeCorrelation::Submission {
            request_id,
            submission_id: submission_id.clone(),
        },
        RequestOutcomeExpectation::Submission {
            request_id: request(9, 2),
            account_key: AccountKey("@alice:example.invalid".to_owned()),
            target: ComposerTarget::Main {
                room_id: "!room:example.invalid".to_owned(),
            },
            submission_id: submission_id.clone(),
        },
        0,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    assert_eq!(mismatch.await, Err(RequestOutcomeError::InvalidOutcome));

    let request_id = request(9, 3);
    let key = TimelineKey::room(
        AccountKey("@alice:example.invalid".to_owned()),
        "!room:example.invalid",
    );
    let operation = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::PreparedMediaQueued {
            request_id,
            key: key.clone(),
            transaction_id: "txn-test".to_owned(),
        },
        0,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    tokio::pin!(operation);
    control.send_event(CoreEvent::Timeline(TimelineEvent::MediaSendQueued {
        request_id,
        key,
        transaction_id: "txn-other".to_owned(),
    }));
    assert!(operation.as_mut().now_or_never().is_none());
    control.send_event(CoreEvent::Timeline(TimelineEvent::MediaSendQueued {
        request_id,
        key: TimelineKey::room(
            AccountKey("@alice:example.invalid".to_owned()),
            "!room:example.invalid",
        ),
        transaction_id: "txn-test".to_owned(),
    }));
    assert!(matches!(
        operation.await,
        Ok(RequestOutcome::PreparedMediaQueued { .. })
    ));
}

#[test]
fn outcome_debug_is_private_safe_and_types_are_not_serde_contracts() {
    let expectation = RequestOutcomeExpectation::PreparedMediaQueued {
        request_id: request(10, 1),
        key: TimelineKey::room(
            AccountKey("@private:example.invalid".to_owned()),
            "!private:example.invalid",
        ),
        transaction_id: "private-transaction".to_owned(),
    };
    let debug = format!("{expectation:?}");
    assert!(!debug.contains("@private"));
    assert!(!debug.contains("private-transaction"));
}
