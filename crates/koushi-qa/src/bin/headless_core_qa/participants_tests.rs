use super::{
    SecondarySasObservation, after_receiver_device_known, incoming_request_timeout_context,
    observe_secondary_sas, requested_verification_flow_id,
};
use crate::registry::{QaScenario, should_run_normal_secondary_participant};
use crate::{
    Arc, AtomicUsize, Ordering, SessionInfo, SessionState, VerificationFlowState,
    VerificationTarget,
};

#[test]
fn stale_gate_failure_is_not_attributed_to_a_fresh_sas_flow() {
    let session = SessionState::AwaitingVerification {
        info: SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@alice:example.invalid".to_owned(),
            device_id: "ALICEDEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
        gate: koushi_state::VerificationGateState {
            methods: vec![koushi_state::VerificationMethodCapability::ExistingDeviceSas],
            account_kind: koushi_state::VerificationAccountKind::ExistingIdentity,
            failure: Some(koushi_state::VerificationGateFailureKind::Timeout),
        },
    };

    assert_eq!(
        observe_secondary_sas(&session, 42, false),
        SecondarySasObservation::Pending
    );
    assert_eq!(
        observe_secondary_sas(&session, 42, true),
        SecondarySasObservation::Failed
    );
}

#[test]
fn incoming_waiter_ignores_the_previous_terminal_flow() {
    let target = VerificationTarget {
        user_id: "@alice:example.invalid".to_owned(),
        device_id: "ALICEDEVICE".to_owned(),
    };
    let stale = VerificationFlowState::Failed {
        request_id: 41,
        target: target.clone(),
        kind: koushi_state::TrustOperationFailureKind::Cancelled,
    };
    let fresh = VerificationFlowState::Requested {
        request_id: 42,
        target: target.clone(),
        initiator: koushi_state::VerificationInitiator::Them,
    };

    assert_eq!(
        requested_verification_flow_id(&stale, Some(&target), Some(41)).unwrap(),
        None
    );
    assert_eq!(
        requested_verification_flow_id(&fresh, Some(&target), Some(41)).unwrap(),
        Some(42)
    );
}

#[test]
fn normal_secondary_participant_policy_covers_only_shared_b_stages() {
    for scenario in [
        QaScenario::All,
        QaScenario::InvitesDm,
        QaScenario::Directory,
        QaScenario::RoomSpace,
        QaScenario::Timeline,
    ] {
        assert!(
            should_run_normal_secondary_participant(scenario),
            "{scenario:?} needs the shared normal B session"
        );
    }

    for scenario in [
        QaScenario::Safety,
        QaScenario::LoginSync,
        QaScenario::SessionStatus,
        QaScenario::CredentialHealth,
        QaScenario::E2eeTrust,
        QaScenario::GateRestore,
        QaScenario::GateNegative,
        QaScenario::SendQueue,
    ] {
        assert!(
            !should_run_normal_secondary_participant(scenario),
            "{scenario:?} must not create the shared normal B session"
        );
    }
}

#[tokio::test]
async fn receiver_device_checkpoint_holds_start_once_until_ack_and_skips_it_on_failure() {
    let starts = Arc::new(AtomicUsize::new(0));
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let task_starts = starts.clone();
    let task = tokio::spawn(after_receiver_device_known(
        async move {
            entered_tx.send(()).map_err(|_| "checkpoint entry closed")?;
            release_rx.await.map_err(|_| "checkpoint release closed")?;
            Ok(())
        },
        move || async move {
            task_starts.fetch_add(1, Ordering::SeqCst);
            Ok::<_, &'static str>(())
        },
    ));

    entered_rx
        .await
        .expect("refresh checkpoint should be polled");
    assert_eq!(starts.load(Ordering::SeqCst), 0);
    release_tx.send(()).expect("release checkpoint");
    task.await
        .expect("checkpoint task should join")
        .expect("checkpoint should succeed");
    assert_eq!(starts.load(Ordering::SeqCst), 1);

    let failed_starts = Arc::new(AtomicUsize::new(0));
    let closure_starts = failed_starts.clone();
    let failed = after_receiver_device_known(
        async { Err::<(), _>("device unknown") },
        move || async move {
            closure_starts.fetch_add(1, Ordering::SeqCst);
            Ok::<_, &'static str>(())
        },
    )
    .await;
    assert_eq!(failed, Err("device unknown"));
    assert_eq!(failed_starts.load(Ordering::SeqCst), 0);
}

/// #1279: the receiver-side timeout context must name state kinds and one
/// boolean, and must never echo the free-form reasons the sync variants carry.
#[test]
fn incoming_request_timeout_context_is_token_only() {
    let idle = incoming_request_timeout_context(
        &VerificationFlowState::Idle,
        None,
        &koushi_state::SyncState::Running,
    );
    assert_eq!(idle, "b_flow=idle b_target_matched=true b_sync=running");

    let reconnecting = incoming_request_timeout_context(
        &VerificationFlowState::Idle,
        None,
        &koushi_state::SyncState::Reconnecting {
            reason: "@alice:example.invalid leaked-secret".to_owned(),
        },
    );
    assert!(reconnecting.contains("b_sync=reconnecting"));
    assert!(!reconnecting.contains("leaked-secret"));
    assert!(!reconnecting.contains("@alice"));
}
