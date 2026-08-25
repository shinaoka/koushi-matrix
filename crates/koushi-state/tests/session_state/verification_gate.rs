use super::support::{
    assert_session_scoped_workflows_cleared, recovery_gate, session_info,
    state_with_session_scoped_workflows,
};
use koushi_state::{
    AppAction, AppEffect, AppError, AppState, AuthSecret, CurrentDeviceTrustState,
    DeviceCleanupOfferReason, DeviceCleanupState, E2eeRecoveryState, NativeAttentionState,
    NavigationState, ProvisionalPhase, RecoveryMethod, RecoveryRequest, RoomSummary, RoomTags,
    SearchScope, SecureBackupGateState, SessionState, SpaceSummary, SubmissionId, SyncState,
    TimelinePaneState, TrustOperationFailureKind, UiEvent, VerificationAccountKind,
    VerificationGateFailureKind, VerificationGateRejectReason, VerificationGateState,
    VerificationMethod, VerificationMethodCapability, reduce,
};

#[test]
fn verification_gate_transition_table_is_fail_closed() {
    let info = session_info();
    let cases = [
        (
            SessionState::Provisional {
                info: info.clone(),
                phase: ProvisionalPhase::CheckingTrust,
            },
            AppAction::CurrentDeviceTrustChanged(CurrentDeviceTrustState::Unverified),
            SessionState::Provisional {
                info: info.clone(),
                phase: ProvisionalPhase::DiscoveringMethods,
            },
        ),
        (
            SessionState::Provisional {
                info: info.clone(),
                phase: ProvisionalPhase::DiscoveringMethods,
            },
            AppAction::VerificationMethodsDiscovered(recovery_gate()),
            SessionState::AwaitingVerification {
                info: info.clone(),
                gate: recovery_gate(),
            },
        ),
        (
            SessionState::AwaitingVerification {
                info: info.clone(),
                gate: recovery_gate(),
            },
            AppAction::VerificationMethodSubmitted {
                method: VerificationMethod::RecoveryKey,
                flow_id: 17,
            },
            SessionState::Verifying {
                info: info.clone(),
                gate: recovery_gate(),
                method: VerificationMethod::RecoveryKey,
                flow_id: 17,
                sas_emojis: vec![],
            },
        ),
    ];

    for (initial, action, expected) in cases {
        let mut state = AppState {
            session: initial,
            ..AppState::default()
        };
        reduce(&mut state, action);
        assert_eq!(state.session, expected);
        assert!(!matches!(state.session, SessionState::Ready(_)));
    }
}

#[test]
fn verification_method_discovery_failure_is_retryable_and_phase_scoped() {
    let info = session_info();
    let mut discovering = AppState {
        session: SessionState::Provisional {
            info: info.clone(),
            phase: ProvisionalPhase::DiscoveringMethods,
        },
        ..AppState::default()
    };

    let effects = reduce(
        &mut discovering,
        AppAction::VerificationMethodDiscoveryFailed {
            generation: 7,
            kind: VerificationGateFailureKind::Timeout,
        },
    );

    assert_eq!(
        discovering.session,
        SessionState::Provisional {
            info: info.clone(),
            phase: ProvisionalPhase::RecheckingTrust {
                failure: Some(VerificationGateFailureKind::Timeout),
            },
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
    );
    assert_eq!(discovering.device_cleanup, DeviceCleanupState::Idle);

    let effects = reduce(
        &mut discovering,
        AppAction::VerificationMethodDiscoveryRetryStarted { generation: 7 },
    );
    assert_eq!(
        discovering.session,
        SessionState::Provisional {
            info: info.clone(),
            phase: ProvisionalPhase::DiscoveringMethods,
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
    );
    assert_eq!(discovering.device_cleanup, DeviceCleanupState::Idle);
    reduce(
        &mut discovering,
        AppAction::VerificationMethodsDiscovered(recovery_gate()),
    );
    assert!(matches!(
        discovering.session,
        SessionState::AwaitingVerification { .. }
    ));

    for session in [
        SessionState::Provisional {
            info: info.clone(),
            phase: ProvisionalPhase::CheckingTrust,
        },
        SessionState::AwaitingVerification {
            info: info.clone(),
            gate: recovery_gate(),
        },
        SessionState::Ready(info.clone()),
    ] {
        let mut state = AppState {
            session,
            ..AppState::default()
        };
        let before = state.clone();
        let effects = reduce(
            &mut state,
            AppAction::VerificationMethodDiscoveryFailed {
                generation: 7,
                kind: VerificationGateFailureKind::Timeout,
            },
        );
        assert!(effects.is_empty());
        assert_eq!(state, before);
    }
}

#[test]
fn only_authoritative_verified_promotes_and_unverified_reenters_the_gate() {
    let mut gated = AppState {
        session: SessionState::Verifying {
            info: session_info(),
            gate: recovery_gate(),
            method: VerificationMethod::RecoveryKey,
            flow_id: 17,
            sas_emojis: vec![],
        },
        ..AppState::default()
    };
    let effects = reduce(
        &mut gated,
        AppAction::CurrentDeviceTrustChanged(CurrentDeviceTrustState::Verified),
    );
    assert_eq!(gated.session, SessionState::Ready(session_info()));
    assert_eq!(gated.secure_backup_gate, SecureBackupGateState::Checking);
    assert!(effects.contains(&AppEffect::PersistSession(session_info())));
    assert!(effects.contains(&AppEffect::StartSync));
    assert!(effects.contains(&AppEffect::InspectSecureBackup));

    let mut ready = state_with_session_scoped_workflows();
    let effects = reduce(
        &mut ready,
        AppAction::CurrentDeviceTrustChanged(CurrentDeviceTrustState::Unverified),
    );
    assert_eq!(
        ready.session,
        SessionState::Provisional {
            info: session_info(),
            phase: ProvisionalPhase::DiscoveringMethods,
        }
    );
    assert_eq!(ready.sync, SyncState::Stopped);
    assert_session_scoped_workflows_cleared(&ready);
    assert!(effects.contains(&AppEffect::StopSync));
}

#[test]
fn awaiting_verification_ignores_non_verified_trust_updates() {
    let info = session_info();
    for trust in [
        CurrentDeviceTrustState::Unknown,
        CurrentDeviceTrustState::Unverified,
    ] {
        let mut state = AppState {
            session: SessionState::AwaitingVerification {
                info: info.clone(),
                gate: recovery_gate(),
            },
            ..AppState::default()
        };
        let before = state.clone();

        let effects = reduce(
            &mut state,
            AppAction::AuthoritativeDeviceTrustChanged {
                generation: 2,
                transition_id: 7,
                trust,
            },
        );

        assert!(effects.is_empty());
        assert_eq!(state, before);
    }
}

#[test]
fn authoritative_verified_promotion_requests_sync_after_actor_ack() {
    let mut gated = AppState {
        session: SessionState::Verifying {
            info: session_info(),
            gate: recovery_gate(),
            method: VerificationMethod::RecoveryKey,
            flow_id: 17,
            sas_emojis: vec![],
        },
        ..AppState::default()
    };

    let effects = reduce(
        &mut gated,
        AppAction::AuthoritativeDeviceTrustChanged {
            generation: 2,
            transition_id: 7,
            trust: CurrentDeviceTrustState::Verified,
        },
    );

    assert_eq!(gated.session, SessionState::Ready(session_info()));
    assert_eq!(gated.sync, SyncState::Starting);
    assert!(effects.contains(&AppEffect::StartSync));
    assert!(!effects.contains(&AppEffect::PersistSession(session_info())));
}

#[test]
fn authoritative_verified_does_not_unlock_an_authentication_locked_session() {
    let mut state = AppState {
        session: SessionState::Locked(session_info()),
        secure_backup_gate: SecureBackupGateState::Ready,
        sync: SyncState::Stopped,
        ..AppState::default()
    };
    let before = state.clone();

    let effects = reduce(
        &mut state,
        AppAction::AuthoritativeDeviceTrustChanged {
            generation: 17,
            transition_id: 1,
            trust: CurrentDeviceTrustState::Verified,
        },
    );

    assert!(effects.is_empty());
    assert_eq!(state, before);
}

#[test]
fn existing_identity_without_proof_waits_for_explicit_rejection_then_discards() {
    let mut state = AppState {
        session: SessionState::Provisional {
            info: session_info(),
            phase: ProvisionalPhase::DiscoveringMethods,
        },
        ..AppState::default()
    };
    let no_proof = VerificationGateState {
        methods: Vec::new(),
        account_kind: VerificationAccountKind::ExistingIdentity,
        failure: Some(VerificationGateFailureKind::NoProofMethod),
    };
    let effects = reduce(
        &mut state,
        AppAction::VerificationMethodsDiscovered(no_proof.clone()),
    );
    assert_eq!(
        state.session,
        SessionState::AwaitingVerification {
            info: session_info(),
            gate: no_proof,
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
    );

    let effects = reduce(
        &mut state,
        AppAction::VerificationSessionRejected {
            reason: VerificationGateRejectReason::ExistingIdentityWithoutProof,
        },
    );
    assert!(matches!(state.session, SessionState::Rejecting { .. }));
    assert!(effects.contains(&AppEffect::RejectProvisionalSession));

    reduce(&mut state, AppAction::ProvisionalSessionDiscarded);
    assert_eq!(state.session, SessionState::SignedOut);
}

#[test]
fn new_identity_bootstrap_requires_written_destination_and_matching_confirmation() {
    let info = session_info();
    let gate = VerificationGateState {
        methods: vec![VerificationMethodCapability::Bootstrap],
        account_kind: VerificationAccountKind::NewIdentity,
        failure: None,
    };
    let mut state = AppState {
        session: SessionState::Verifying {
            info: info.clone(),
            gate: gate.clone(),
            method: VerificationMethod::Bootstrap,
            flow_id: 41,
            sas_emojis: vec![],
        },
        ..AppState::default()
    };
    let effects = reduce(
        &mut state,
        AppAction::BootstrapRecoveryKeyDelivered { flow_id: 41 },
    );
    assert_eq!(
        state.session,
        SessionState::AwaitingBootstrapConfirmation {
            info: info.clone(),
            gate: gate.clone(),
            flow_id: 41,
            destination_written: true,
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
    );

    let before = state.clone();
    assert!(
        reduce(
            &mut state,
            AppAction::BootstrapRecoverySavedConfirmed { flow_id: 40 }
        )
        .is_empty()
    );
    assert_eq!(state, before);
    let effects = reduce(
        &mut state,
        AppAction::BootstrapRecoverySavedConfirmed { flow_id: 41 },
    );
    assert_eq!(
        state.session,
        SessionState::Provisional {
            info,
            phase: ProvisionalPhase::RecheckingTrust { failure: None },
        }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::CheckCurrentDeviceTrust,
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
        ]
    );
}

#[test]
fn bootstrap_delivery_failure_is_retryable_and_unknown_is_never_new_identity() {
    let info = session_info();
    let gate = VerificationGateState {
        methods: vec![VerificationMethodCapability::Bootstrap],
        account_kind: VerificationAccountKind::NewIdentity,
        failure: None,
    };
    let mut state = AppState {
        session: SessionState::Verifying {
            info: info.clone(),
            gate: gate.clone(),
            method: VerificationMethod::Bootstrap,
            flow_id: 9,
            sas_emojis: vec![],
        },
        ..AppState::default()
    };
    reduce(
        &mut state,
        AppAction::BootstrapRecoveryKeyDeliveryFailed {
            flow_id: 9,
            kind: VerificationGateFailureKind::Sdk,
        },
    );
    assert!(matches!(
        state.session,
        SessionState::AwaitingVerification {
            gate: VerificationGateState {
                account_kind: VerificationAccountKind::NewIdentity,
                failure: Some(VerificationGateFailureKind::Sdk),
                ..
            },
            ..
        }
    ));

    let mut unknown = AppState {
        session: SessionState::Provisional {
            info,
            phase: ProvisionalPhase::DiscoveringMethods,
        },
        ..AppState::default()
    };
    reduce(
        &mut unknown,
        AppAction::VerificationMethodsDiscovered(VerificationGateState {
            methods: Vec::new(),
            account_kind: VerificationAccountKind::Unknown,
            failure: Some(VerificationGateFailureKind::Network),
        }),
    );
    assert!(matches!(
        unknown.session,
        SessionState::AwaitingVerification {
            gate: VerificationGateState {
                account_kind: VerificationAccountKind::Unknown,
                ..
            },
            ..
        }
    ));
}

#[test]
fn gate_sas_terminals_are_retryable_and_done_only_requests_trust_recheck() {
    let info = session_info();
    let gate = VerificationGateState {
        methods: vec![VerificationMethodCapability::ExistingDeviceSas],
        account_kind: VerificationAccountKind::ExistingIdentity,
        failure: None,
    };
    for (kind, expected) in [
        (
            TrustOperationFailureKind::Cancelled,
            VerificationGateFailureKind::Cancelled,
        ),
        (
            TrustOperationFailureKind::Mismatch,
            VerificationGateFailureKind::Mismatch,
        ),
        (
            TrustOperationFailureKind::Timeout,
            VerificationGateFailureKind::Timeout,
        ),
        (
            TrustOperationFailureKind::Forbidden,
            VerificationGateFailureKind::Forbidden,
        ),
        (
            TrustOperationFailureKind::Network,
            VerificationGateFailureKind::Network,
        ),
        (
            TrustOperationFailureKind::Sdk,
            VerificationGateFailureKind::Sdk,
        ),
    ] {
        let mut state = AppState {
            session: SessionState::Verifying {
                info: info.clone(),
                gate: gate.clone(),
                method: VerificationMethod::ExistingDeviceSas,
                flow_id: 77,
                sas_emojis: vec![],
            },
            ..AppState::default()
        };
        reduce(
            &mut state,
            AppAction::VerificationFailed {
                request_id: 77,
                kind,
            },
        );
        assert!(matches!(
            state.session,
            SessionState::AwaitingVerification {
                gate: VerificationGateState { failure: Some(value), .. },
                ..
            } if value == expected
        ));
    }

    let mut done = AppState {
        session: SessionState::Verifying {
            info: info.clone(),
            gate,
            method: VerificationMethod::ExistingDeviceSas,
            flow_id: 77,
            sas_emojis: vec![],
        },
        ..AppState::default()
    };
    let effects = reduce(
        &mut done,
        AppAction::VerificationCompleted { request_id: 77 },
    );
    assert_eq!(
        done.session,
        SessionState::Provisional {
            info,
            phase: ProvisionalPhase::RecheckingTrust { failure: None },
        }
    );
    assert_eq!(effects[0], AppEffect::CheckCurrentDeviceTrust);
    assert!(!matches!(done.session, SessionState::Ready(_)));
}

#[test]
fn active_verification_survives_unknown_and_unverified_trust_observations() {
    let info = session_info();
    let gate = VerificationGateState {
        methods: vec![VerificationMethodCapability::ExistingDeviceSas],
        account_kind: VerificationAccountKind::ExistingIdentity,
        failure: None,
    };
    for trust in [
        CurrentDeviceTrustState::Unknown,
        CurrentDeviceTrustState::Unverified,
    ] {
        let expected = SessionState::Verifying {
            info: info.clone(),
            gate: gate.clone(),
            method: VerificationMethod::ExistingDeviceSas,
            flow_id: 41,
            sas_emojis: Vec::new(),
        };
        let mut state = AppState {
            session: expected.clone(),
            ..AppState::default()
        };
        let effects = reduce(
            &mut state,
            AppAction::AuthoritativeDeviceTrustChanged {
                generation: 7,
                transition_id: 1,
                trust,
            },
        );
        assert_eq!(state.session, expected);
        assert!(effects.is_empty());
    }
}

#[test]
fn gate_sas_start_mismatch_cancel_and_retry_remain_correlated() {
    let info = session_info();
    let gate = VerificationGateState {
        methods: vec![VerificationMethodCapability::ExistingDeviceSas],
        account_kind: VerificationAccountKind::ExistingIdentity,
        failure: None,
    };
    let mut state = AppState {
        session: SessionState::AwaitingVerification {
            info: info.clone(),
            gate: gate.clone(),
        },
        ..AppState::default()
    };
    reduce(
        &mut state,
        AppAction::VerificationMethodSubmitted {
            method: VerificationMethod::ExistingDeviceSas,
            flow_id: 10,
        },
    );
    assert!(matches!(
        state.session,
        SessionState::Verifying { flow_id: 10, .. }
    ));
    let before = state.clone();
    assert!(
        reduce(
            &mut state,
            AppAction::VerificationCancelled {
                request_id: 9,
                reason: koushi_state::VerificationCancelReason::Mismatch,
            },
        )
        .is_empty()
    );
    assert_eq!(state, before);
    reduce(
        &mut state,
        AppAction::VerificationCancelled {
            request_id: 10,
            reason: koushi_state::VerificationCancelReason::Mismatch,
        },
    );
    assert!(matches!(
        state.session,
        SessionState::AwaitingVerification {
            gate: VerificationGateState {
                failure: Some(VerificationGateFailureKind::Mismatch),
                ..
            },
            ..
        }
    ));
    reduce(
        &mut state,
        AppAction::VerificationMethodSubmitted {
            method: VerificationMethod::ExistingDeviceSas,
            flow_id: 11,
        },
    );
    assert!(matches!(
        state.session,
        SessionState::Verifying { flow_id: 11, .. }
    ));
}

#[test]
fn verification_retry_clears_the_completed_attempt_failure() {
    let info = session_info();
    let gate = VerificationGateState {
        methods: vec![VerificationMethodCapability::ExistingDeviceSas],
        account_kind: VerificationAccountKind::ExistingIdentity,
        failure: None,
    };
    let mut state = AppState {
        session: SessionState::AwaitingVerification {
            info: info.clone(),
            gate,
        },
        ..AppState::default()
    };

    reduce(
        &mut state,
        AppAction::VerificationMethodSubmitted {
            method: VerificationMethod::ExistingDeviceSas,
            flow_id: 77,
        },
    );
    reduce(
        &mut state,
        AppAction::VerificationFailed {
            request_id: 77,
            kind: TrustOperationFailureKind::Timeout,
        },
    );
    assert!(matches!(
        state.session,
        SessionState::AwaitingVerification {
            gate: VerificationGateState {
                failure: Some(VerificationGateFailureKind::Timeout),
                ..
            },
            ..
        }
    ));

    reduce(
        &mut state,
        AppAction::VerificationMethodSubmitted {
            method: VerificationMethod::ExistingDeviceSas,
            flow_id: 78,
        },
    );

    assert_eq!(
        state.session,
        SessionState::Verifying {
            info,
            gate: VerificationGateState {
                methods: vec![VerificationMethodCapability::ExistingDeviceSas],
                account_kind: VerificationAccountKind::ExistingIdentity,
                failure: None,
            },
            method: VerificationMethod::ExistingDeviceSas,
            flow_id: 78,
            sas_emojis: Vec::new(),
        }
    );
}

#[test]
fn recovery_cancel_and_retry_never_escape_the_gate() {
    let info = session_info();
    let gate = recovery_gate();
    let mut state = AppState {
        session: SessionState::AwaitingVerification { info, gate },
        ..AppState::default()
    };
    reduce(
        &mut state,
        AppAction::VerificationMethodSubmitted {
            method: VerificationMethod::RecoveryKey,
            flow_id: 21,
        },
    );
    reduce(
        &mut state,
        AppAction::VerificationCancelled {
            request_id: 21,
            reason: koushi_state::VerificationCancelReason::User,
        },
    );
    assert!(matches!(
        state.session,
        SessionState::AwaitingVerification {
            gate: VerificationGateState {
                failure: Some(VerificationGateFailureKind::Cancelled),
                ..
            },
            ..
        }
    ));
    reduce(
        &mut state,
        AppAction::VerificationMethodSubmitted {
            method: VerificationMethod::RecoveryKey,
            flow_id: 22,
        },
    );
    assert!(matches!(
        state.session,
        SessionState::Verifying { flow_id: 22, .. }
    ));
    assert!(!matches!(state.session, SessionState::Ready(_)));
}

#[test]
fn stale_gate_failure_from_previous_sas_does_not_interrupt_recovery_key_flow() {
    let info = session_info();
    let gate = VerificationGateState {
        methods: vec![
            VerificationMethodCapability::ExistingDeviceSas,
            VerificationMethodCapability::RecoveryKey,
        ],
        account_kind: VerificationAccountKind::ExistingIdentity,
        failure: None,
    };
    let mut state = AppState {
        session: SessionState::AwaitingVerification {
            info: info.clone(),
            gate,
        },
        ..AppState::default()
    };

    reduce(
        &mut state,
        AppAction::VerificationMethodSubmitted {
            method: VerificationMethod::ExistingDeviceSas,
            flow_id: 30,
        },
    );
    reduce(
        &mut state,
        AppAction::VerificationFailed {
            request_id: 30,
            kind: TrustOperationFailureKind::Timeout,
        },
    );
    reduce(
        &mut state,
        AppAction::VerificationMethodSubmitted {
            method: VerificationMethod::RecoveryKey,
            flow_id: 31,
        },
    );
    let recovery_verifying = state.clone();

    assert!(
        reduce(
            &mut state,
            AppAction::VerificationGateAttemptFailed {
                flow_id: 30,
                kind: VerificationGateFailureKind::Cancelled,
            },
        )
        .is_empty()
    );
    assert_eq!(state, recovery_verifying);
}

#[test]
fn normal_room_commands_are_rejected_in_every_verification_gate_state() {
    let info = session_info();
    let states = [
        SessionState::Provisional {
            info: info.clone(),
            phase: ProvisionalPhase::CheckingTrust,
        },
        SessionState::AwaitingVerification {
            info: info.clone(),
            gate: recovery_gate(),
        },
        SessionState::Verifying {
            info: info.clone(),
            gate: recovery_gate(),
            method: VerificationMethod::RecoveryKey,
            flow_id: 17,
            sas_emojis: vec![],
        },
        SessionState::AwaitingBootstrapConfirmation {
            info: info.clone(),
            gate: VerificationGateState {
                methods: vec![VerificationMethodCapability::Bootstrap],
                account_kind: VerificationAccountKind::NewIdentity,
                failure: None,
            },
            flow_id: 18,
            destination_written: true,
        },
        SessionState::Rejecting {
            info,
            reason: VerificationGateRejectReason::ExistingIdentityWithoutProof,
        },
    ];

    let mut attention = NativeAttentionState::default();
    attention.summary.unread_count = 1;
    let actions = vec![
        AppAction::RoomListFilterSelected {
            filter: koushi_state::RoomListFilter::Unread,
        },
        AppAction::TimelineBackPaginationRequested {
            room_id: "room-a".to_owned(),
        },
        AppAction::OpenThread {
            room_id: "room-a".to_owned(),
            root_event_id: "event-a".to_owned(),
            intent: koushi_state::ThreadOpenIntent::ExistingThread,
        },
        AppAction::SearchSubmitted {
            request_id: 1,
            query: "query".to_owned(),
            scope: SearchScope::AllRooms,
        },
        AppAction::SendTextSubmitted {
            room_id: "room-a".to_owned(),
            transaction_id: "txn-a".to_owned(),
            body: "body".to_owned(),
        },
        AppAction::ComposerSubmissionAccepted {
            submission_id: SubmissionId::new("submission-a"),
            room_id: "room-a".to_owned(),
            transaction_id: "txn-a".to_owned(),
            body: "body".to_owned(),
        },
        AppAction::ThreadSubmissionAccepted {
            submission_id: SubmissionId::new("thread-submission-a"),
            room_id: "room-a".to_owned(),
            root_event_id: "event-a".to_owned(),
            transaction_id: "thread-txn-a".to_owned(),
            body: "body".to_owned(),
        },
        AppAction::DirectoryQueryRequested {
            request_id: 1,
            query: koushi_state::DirectoryQuery {
                term: Some("query".to_owned()),
                server_name: None,
                limit: Some(10),
                since: None,
            },
        },
        AppAction::NativeAttentionUpdated { attention },
    ];

    for session in states {
        for action in &actions {
            let mut state = AppState {
                session: session.clone(),
                ..AppState::default()
            };
            let before = state.clone();
            let effects = reduce(&mut state, action.clone());
            assert_eq!(state, before, "gate accepted normal action: {action:?}");
            assert!(effects.is_empty(), "gate emitted effect for: {action:?}");
        }
    }
}

#[test]
fn verification_gate_capabilities_serialize_without_secrets_or_sdk_identifiers() {
    let gate = VerificationGateState {
        methods: vec![
            VerificationMethodCapability::ExistingDeviceSas,
            VerificationMethodCapability::RecoveryKey,
        ],
        account_kind: VerificationAccountKind::ExistingIdentity,
        failure: Some(VerificationGateFailureKind::Network),
    };
    let serialized = serde_json::to_string(&gate).expect("gate serializes");
    let debug = format!("{gate:?}");
    for forbidden in [
        "synthetic-recovery-secret",
        "synthetic-access-token",
        "RAWDEVICEID",
        "raw sdk error",
    ] {
        assert!(!serialized.contains(forbidden));
        assert!(!debug.contains(forbidden));
    }
}

#[test]
fn e2ee_recovery_required_after_login_enters_gate_without_normal_sync() {
    let mut state = AppState {
        session: SessionState::Provisional {
            info: session_info(),
            phase: ProvisionalPhase::DiscoveringMethods,
        },
        ..AppState::default()
    };
    let info = session_info();
    let methods = vec![RecoveryMethod::RecoveryKey, RecoveryMethod::SecurityPhrase];

    let effects = reduce(
        &mut state,
        AppAction::E2eeRecoveryRequired {
            info: info.clone(),
            methods: methods.clone(),
        },
    );

    assert_eq!(
        state.session,
        SessionState::AwaitingVerification {
            info: info.clone(),
            gate: VerificationGateState {
                methods: vec![
                    VerificationMethodCapability::RecoveryKey,
                    VerificationMethodCapability::SecurityPhrase,
                ],
                account_kind: VerificationAccountKind::ExistingIdentity,
                failure: None,
            },
        }
    );
    assert_eq!(state.sync, SyncState::Stopped);
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
        ]
    );
}

#[test]
fn e2ee_recovery_required_after_failed_login_clears_login_error() {
    let mut state = AppState {
        session: SessionState::Provisional {
            info: session_info(),
            phase: ProvisionalPhase::DiscoveringMethods,
        },
        errors: vec![AppError {
            code: "login_failed".to_owned(),
            message: "Invalid username or password".to_owned(),
            recoverable: true,
        }],
        ..AppState::default()
    };
    let info = session_info();

    let effects = reduce(
        &mut state,
        AppAction::E2eeRecoveryRequired {
            info: info.clone(),
            methods: vec![RecoveryMethod::RecoveryKey],
        },
    );

    assert!(state.errors.is_empty());
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
            AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
        ]
    );
}

#[test]
fn e2ee_recovery_submission_emits_recover_effect_without_exposing_secret() {
    let info = session_info();
    let mut state = AppState {
        session: SessionState::AwaitingVerification {
            info: info.clone(),
            gate: VerificationGateState {
                methods: vec![
                    VerificationMethodCapability::RecoveryKey,
                    VerificationMethodCapability::SecurityPhrase,
                ],
                account_kind: VerificationAccountKind::ExistingIdentity,
                failure: None,
            },
        },
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::E2eeRecoverySubmitted {
            flow_id: 77,
            request: RecoveryRequest {
                secret: AuthSecret::new("synthetic-recovery-secret"),
            },
        },
    );

    assert_eq!(
        state.session,
        SessionState::Verifying {
            info: info.clone(),
            gate: VerificationGateState {
                methods: vec![
                    VerificationMethodCapability::RecoveryKey,
                    VerificationMethodCapability::SecurityPhrase,
                ],
                account_kind: VerificationAccountKind::ExistingIdentity,
                failure: None,
            },
            method: VerificationMethod::RecoveryKey,
            flow_id: 77,
            sas_emojis: vec![],
        }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::RecoverE2ee(RecoveryRequest {
                secret: AuthSecret::new("synthetic-recovery-secret"),
            }),
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
        ]
    );
    assert!(!format!("{effects:?}").contains("synthetic-recovery-secret"));
}

#[test]
fn e2ee_recovery_retry_retires_the_cleanup_offer() {
    let mut state = AppState {
        session: SessionState::AwaitingVerification {
            info: session_info(),
            gate: recovery_gate(),
        },
        device_cleanup: DeviceCleanupState::Offered {
            reason: DeviceCleanupOfferReason::RecoveryFailed,
        },
        ..AppState::default()
    };

    reduce(
        &mut state,
        AppAction::E2eeRecoverySubmitted {
            flow_id: 78,
            request: RecoveryRequest {
                secret: AuthSecret::new("synthetic-recovery-secret"),
            },
        },
    );

    assert!(matches!(state.session, SessionState::Verifying { .. }));
    assert_eq!(state.device_cleanup, DeviceCleanupState::Idle);
}

#[test]
fn e2ee_recovery_success_promotes_session_and_starts_sync() {
    let info = session_info();
    let mut state = AppState {
        session: SessionState::Verifying {
            info: info.clone(),
            gate: recovery_gate(),
            method: VerificationMethod::RecoveryKey,
            flow_id: 0,
            sas_emojis: vec![],
        },
        ..AppState::default()
    };

    let effects = reduce(&mut state, AppAction::E2eeRecoverySucceeded);

    assert_eq!(state.session, SessionState::Ready(info.clone()));
    assert_eq!(state.sync, SyncState::Starting);
    assert_eq!(
        effects,
        vec![
            AppEffect::PersistSession(info),
            AppEffect::StartSync,
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
        ]
    );
}

#[test]
fn gate_sas_projection_requires_matching_flow_and_exactly_seven_emojis() {
    let emojis = (0..7)
        .map(|index| koushi_state::SasEmoji {
            symbol: format!("e{index}"),
            description: format!("d{index}"),
        })
        .collect::<Vec<_>>();
    let mut state = AppState {
        session: SessionState::Verifying {
            info: session_info(),
            gate: recovery_gate(),
            method: VerificationMethod::ExistingDeviceSas,
            flow_id: 44,
            sas_emojis: vec![],
        },
        ..AppState::default()
    };
    let before = state.clone();
    assert!(
        reduce(
            &mut state,
            AppAction::GateSasPresented {
                flow_id: 43,
                emojis: emojis.clone()
            }
        )
        .is_empty()
    );
    assert_eq!(state, before);
    assert!(
        reduce(
            &mut state,
            AppAction::GateSasPresented {
                flow_id: 44,
                emojis: emojis[..6].to_vec()
            }
        )
        .is_empty()
    );
    assert_eq!(state, before);
    assert!(
        !reduce(
            &mut state,
            AppAction::GateSasPresented {
                flow_id: 44,
                emojis: emojis.clone()
            }
        )
        .is_empty()
    );
    assert!(
        matches!(state.session, SessionState::Verifying { sas_emojis: ref projected, .. } if projected == &emojis)
    );
    reduce(
        &mut state,
        AppAction::VerificationCompleted { request_id: 44 },
    );
    assert!(matches!(
        state.session,
        SessionState::Provisional {
            phase: ProvisionalPhase::RecheckingTrust { .. },
            ..
        }
    ));
}

#[test]
fn unknown_e2ee_recovery_state_does_not_prompt_or_stop_sync() {
    let info = session_info();
    let mut state = AppState {
        session: SessionState::Ready(info.clone()),
        sync: SyncState::Running,
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::E2eeRecoveryStateChanged {
            state: E2eeRecoveryState::Unknown,
            methods: vec![RecoveryMethod::RecoveryKey],
        },
    );

    assert_eq!(state.session, SessionState::Ready(info));
    assert_eq!(state.sync, SyncState::Running);
    assert!(effects.is_empty());
}

#[test]
fn ready_session_ignores_recovery_availability_as_an_admission_signal() {
    let info = session_info();
    let mut state = AppState {
        session: SessionState::Ready(info.clone()),
        sync: SyncState::Running,
        navigation: NavigationState {
            active_space_id: Some("space-a".to_owned()),
            active_room_id: Some("room-a".to_owned()),
            ..Default::default()
        },
        spaces: vec![SpaceSummary {
            space_id: "space-a".to_owned(),
            display_name: "Space A".to_owned(),
            avatar: None,
            child_room_ids: vec!["room-a".to_owned()],
        }],
        rooms: vec![RoomSummary {
            room_id: "room-a".to_owned(),
            display_name: "Room A".to_owned(),
            display_label: "Room A".to_owned(),
            original_display_label: "Room A".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 3,
            notification_count: 3,
            highlight_count: 0,
            marked_unread: false,
            recency_stamp: None,
            conversation_activity: None,
            latest_event: None,
            parent_space_ids: vec!["space-a".to_owned()],
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 0,
        }],
        timeline: TimelinePaneState {
            room_id: Some("room-a".to_owned()),
            is_subscribed: true,
            is_paginating_backwards: false,
            composer: Default::default(),
            submission_registry: Default::default(),
            scheduled_send_capability: Default::default(),
            scheduled_sends: Vec::new(),
            staged_uploads: Vec::new(),
            media_gallery: Vec::new(),
            media_downloads: Default::default(),
            continuity: Default::default(),
        },
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::E2eeRecoveryStateChanged {
            state: E2eeRecoveryState::Incomplete,
            methods: vec![RecoveryMethod::RecoveryKey, RecoveryMethod::SecurityPhrase],
        },
    );

    assert_eq!(state.session, SessionState::Ready(info.clone()));
    assert_eq!(state.sync, SyncState::Running);
    assert_eq!(
        state.navigation,
        NavigationState {
            active_space_id: Some("space-a".to_owned()),
            active_room_id: Some("room-a".to_owned()),
            ..Default::default()
        }
    );
    assert_eq!(state.spaces.len(), 1);
    assert_eq!(state.rooms.len(), 1);
    assert!(state.timeline.is_subscribed);
    assert!(effects.is_empty());
}

#[test]
fn enabled_e2ee_recovery_state_requests_authoritative_trust_recheck() {
    let info = session_info();
    let mut state = AppState {
        session: SessionState::AwaitingVerification {
            info: info.clone(),
            gate: recovery_gate(),
        },
        sync: SyncState::Stopped,
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::E2eeRecoveryStateChanged {
            state: E2eeRecoveryState::Enabled,
            methods: vec![RecoveryMethod::RecoveryKey],
        },
    );

    assert_eq!(
        state.session,
        SessionState::Provisional {
            info,
            phase: ProvisionalPhase::RecheckingTrust { failure: None },
        }
    );
    assert_eq!(state.sync, SyncState::Stopped);
    assert_eq!(
        effects,
        vec![
            AppEffect::CheckCurrentDeviceTrust,
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
        ]
    );
}

#[test]
fn active_recovery_ignores_recovery_state_observer_updates() {
    let info = session_info();
    for recovery_state in [E2eeRecoveryState::Enabled, E2eeRecoveryState::Disabled] {
        let mut state = AppState {
            session: SessionState::Verifying {
                info: info.clone(),
                gate: recovery_gate(),
                method: VerificationMethod::RecoveryKey,
                flow_id: 3,
                sas_emojis: Vec::new(),
            },
            sync: SyncState::Stopped,
            ..AppState::default()
        };
        let before = state.clone();

        let effects = reduce(
            &mut state,
            AppAction::E2eeRecoveryStateChanged {
                state: recovery_state,
                methods: vec![RecoveryMethod::RecoveryKey],
            },
        );

        assert!(effects.is_empty());
        assert_eq!(state, before);
    }
}
