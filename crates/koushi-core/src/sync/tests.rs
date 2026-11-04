use tokio::sync::mpsc;

use super::*;

#[test]
fn any_new_positioned_commit_is_startup_handoff_evidence() {
    assert!(committed_response_is_handoff_evidence(true, 1, 0));
    assert!(committed_response_is_handoff_evidence(true, 2, 1));
    assert!(!committed_response_is_handoff_evidence(false, 1, 0));
    assert!(!committed_response_is_handoff_evidence(true, 1, 1));
}

#[test]
fn controls_are_generation_fenced() {
    assert!(accepts_control(
        SyncLifecycle::Starting,
        7,
        7,
        &[SyncLifecycle::Starting, SyncLifecycle::Reconnecting],
    ));
    assert!(!accepts_control(
        SyncLifecycle::Starting,
        8,
        7,
        &[SyncLifecycle::Starting],
    ));
    assert!(!accepts_control(
        SyncLifecycle::Running,
        7,
        7,
        &[SyncLifecycle::Starting],
    ));
}

#[test]
fn replacement_recovery_requires_matching_encryption_and_room_proofs() {
    use matrix_sdk::encryption::{
        EncryptionSyncReadinessSnapshot as Snapshot, EncryptionSyncReadinessState as State,
    };

    let mut room_first = ReplacementRecoveryProof::new(4, 10);
    assert!(!room_first.observe_room_response(11));
    assert!(!room_first.observe_encryption(Snapshot {
        generation: 4,
        state: State::Received,
    }));
    assert!(room_first.observe_encryption(Snapshot {
        generation: 5,
        state: State::Received,
    }));

    let mut encryption_first = ReplacementRecoveryProof::new(9, 20);
    assert!(!encryption_first.observe_encryption(Snapshot {
        generation: 10,
        state: State::Pending,
    }));
    assert!(!encryption_first.observe_encryption(Snapshot {
        generation: 10,
        state: State::Received,
    }));
    assert!(encryption_first.observe_room_response(21));
}

#[test]
fn newer_replacement_generation_clears_partial_proofs() {
    use matrix_sdk::encryption::{
        EncryptionSyncReadinessSnapshot as Snapshot, EncryptionSyncReadinessState as State,
    };

    let mut proof = ReplacementRecoveryProof::new(1, 30);
    assert!(!proof.observe_room_response(31));
    assert!(proof.observe_encryption(Snapshot {
        generation: 2,
        state: State::Received,
    }));
    assert!(!proof.observe_encryption(Snapshot {
        generation: 3,
        state: State::Pending,
    }));
    assert!(!proof.room_response_committed);
    assert!(!proof.observe_encryption(Snapshot {
        generation: 3,
        state: State::Received,
    }));
    assert!(proof.observe_room_response(32));
}

#[test]
fn queued_pre_termination_room_commit_cannot_satisfy_replacement() {
    use matrix_sdk::encryption::{
        EncryptionSyncReadinessSnapshot as Snapshot, EncryptionSyncReadinessState as State,
    };

    let mut proof = ReplacementRecoveryProof::new(7, 41);
    assert!(!proof.observe_room_response(41));
    assert!(!proof.observe_encryption(Snapshot {
        generation: 8,
        state: State::Received,
    }));
    assert!(!proof.observe_room_response(41));
    assert!(proof.observe_room_response(42));
}

#[tokio::test]
async fn explicit_stop_cancels_replacement_backoff_before_restart() {
    let stop = Arc::new(SyncObserverStop::default());
    let task_stop = stop.clone();
    let task = tokio::spawn(async move {
        replacement_restart_backoff(&task_stop, Duration::from_secs(30)).await
    });
    tokio::task::yield_now().await;
    stop.request();
    assert!(!task.await.expect("backoff task"));
}

#[test]
fn failure_kind_labels_are_private_safe_tokens() {
    assert_eq!(
        sync_failure_kind_label(SyncFailureKind::Http),
        "sync_failed_http"
    );
    assert_eq!(
        sync_failure_kind_label(SyncFailureKind::Auth),
        "sync_failed_auth"
    );
    assert_eq!(
        sync_failure_kind_label(SyncFailureKind::Store),
        "sync_failed_store"
    );
    assert_eq!(
        sync_failure_kind_label(SyncFailureKind::Internal),
        "sync_failed_internal"
    );
}

#[test]
fn bad_request_and_schema_errcodes_are_actionable_diagnostics() {
    use matrix_sdk::ruma::api::error::ErrorKind;

    assert_eq!(
        classify_http_status(Some(400)),
        SlidingSyncHttpStatus::BadRequest
    );
    for (kind, expected) in [
        (ErrorKind::Unknown, SlidingSyncMatrixErrorKind::Unknown),
        (ErrorKind::BadJson, SlidingSyncMatrixErrorKind::BadJson),
        (
            ErrorKind::InvalidParam,
            SlidingSyncMatrixErrorKind::InvalidParam,
        ),
        (
            ErrorKind::MissingParam,
            SlidingSyncMatrixErrorKind::MissingParam,
        ),
        (ErrorKind::NotJson, SlidingSyncMatrixErrorKind::NotJson),
        (ErrorKind::NotFound, SlidingSyncMatrixErrorKind::NotFound),
        (
            ErrorKind::Unauthorized,
            SlidingSyncMatrixErrorKind::Unauthorized,
        ),
    ] {
        assert_eq!(classify_matrix_error_kind(Some(&kind)), expected);
    }
}

#[test]
fn observer_infrastructure_loss_is_not_a_normal_stop() {
    assert!(matches!(
        internal_observer_failure(true),
        SyncTaskOutcome::Failed {
            kind: SyncFailureKind::Internal,
            ever_connected: true,
        }
    ));
}

#[test]
fn newer_room_snapshot_supersedes_without_becoming_an_internal_failure() {
    assert_eq!(
        classify_room_list_reconcile_ack(
            7,
            11,
            RoomListReconcileAck::Superseded {
                backend_generation: 7,
                room_generation: 3,
                response_sequence: 12,
            },
        ),
        RoomListReconcileResult::Superseded {
            response_sequence: 12,
        }
    );
    assert_eq!(
        classify_room_list_reconcile_ack(
            7,
            11,
            RoomListReconcileAck::Superseded {
                backend_generation: 8,
                room_generation: 3,
                response_sequence: 12,
            },
        ),
        RoomListReconcileResult::Failed
    );
}

#[test]
fn projected_room_list_ack_is_connectivity_evidence() {
    assert_eq!(
        classify_room_list_reconcile_ack(
            7,
            11,
            RoomListReconcileAck::Projected {
                backend_generation: 7,
                room_generation: 3,
                response_sequence: 11,
            },
        ),
        RoomListReconcileResult::Projected {
            response_sequence: 11,
        }
    );
    assert_eq!(
        classify_room_list_reconcile_ack(
            7,
            11,
            RoomListReconcileAck::Projected {
                backend_generation: 8,
                room_generation: 3,
                response_sequence: 11,
            },
        ),
        RoomListReconcileResult::Failed
    );
}

#[test]
fn room_list_reconcile_wait_diagnostic_distinguishes_terminal_outcomes() {
    for (outcome, token) in [
        (RoomListReconcileDiagnosticOutcome::Received, "received"),
        (
            RoomListReconcileDiagnosticOutcome::SendClosed,
            "send_closed",
        ),
        (RoomListReconcileDiagnosticOutcome::Timeout, "timeout"),
        (RoomListReconcileDiagnosticOutcome::AckClosed, "ack_closed"),
        (
            RoomListReconcileDiagnosticOutcome::InvalidAck,
            "invalid_ack",
        ),
    ] {
        let event = room_list_reconcile_diagnostic_event(outcome, 42);
        assert_eq!(event.source, "core.sync");
        assert_eq!(event.stage, "room_list_reconcile_wait");
        assert!(event.fields.iter().any(|field| {
            field.key == "outcome"
                && field.value == koushi_diagnostics::DiagnosticValue::Token(token)
        }));
        assert!(event.fields.iter().any(|field| {
            field.key == "elapsed_ms"
                && field.value == koushi_diagnostics::DiagnosticValue::Milliseconds(42)
        }));
    }
}

#[tokio::test]
async fn action_channel_accepts_projected_sync_statuses_with_generations() {
    let (tx, mut rx) = mpsc::channel(4);
    let generation = AtomicU64::new(0);
    send_sync_status(
        &tx,
        &generation,
        crate::SlidingSyncDiagnostics::default().owner(),
        SyncLifecycleStatus::Starting,
    )
    .await;
    send_sync_status(
        &tx,
        &generation,
        crate::SlidingSyncDiagnostics::default().owner(),
        SyncLifecycleStatus::Running,
    )
    .await;
    assert!(matches!(
        rx.recv().await,
        Some(actions) if matches!(
            actions.as_slice(),
            [AppAction::SyncStatusChanged { generation: 1, status: SyncLifecycleStatus::Starting }]
        )
    ));
    assert!(matches!(
        rx.recv().await,
        Some(actions) if matches!(
            actions.as_slice(),
            [AppAction::SyncStatusChanged { generation: 2, status: SyncLifecycleStatus::Running }]
        )
    ));
}

#[test]
fn sync_once_requires_no_continuous_owner() {
    assert!(sync_once_admitted(SyncLifecycle::Stopped, false, false));
    assert!(!sync_once_admitted(SyncLifecycle::Running, true, true));
    assert!(!sync_once_admitted(SyncLifecycle::Failed, false, true));
}
