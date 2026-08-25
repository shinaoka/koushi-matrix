use super::support::{recovery_gate, session_info};
use koushi_state::{
    AppAction, AppState, CurrentDeviceTrustState, DeviceCleanupAuthMode, DeviceCleanupFailureKind,
    DeviceCleanupLocalMode, DeviceCleanupOfferReason, DeviceCleanupRemoteOutcome,
    DeviceCleanupState, ProvisionalPhase, SessionState, VerificationAccountKind,
    VerificationGateFailureKind, VerificationGateState, VerificationMethod, reduce,
};

#[test]
fn device_cleanup_is_offered_without_automatically_discarding_failed_verification() {
    let mut state = AppState {
        session: SessionState::Verifying {
            info: session_info(),
            gate: recovery_gate(),
            method: VerificationMethod::RecoveryKey,
            flow_id: 41,
            sas_emojis: vec![],
        },
        ..AppState::default()
    };

    reduce(
        &mut state,
        AppAction::VerificationGateAttemptFailed {
            flow_id: 41,
            kind: VerificationGateFailureKind::Forbidden,
        },
    );

    assert!(matches!(
        state.session,
        SessionState::AwaitingVerification { .. }
    ));
    assert_eq!(
        state.device_cleanup,
        DeviceCleanupState::Offered {
            reason: DeviceCleanupOfferReason::RecoveryFailed,
        }
    );
}

#[test]
fn device_cleanup_is_offered_when_recovery_task_fails() {
    let mut state = AppState {
        session: SessionState::Verifying {
            info: session_info(),
            gate: recovery_gate(),
            method: VerificationMethod::RecoveryKey,
            flow_id: 42,
            sas_emojis: vec![],
        },
        ..AppState::default()
    };

    reduce(
        &mut state,
        AppAction::E2eeRecoveryFailed {
            message: "recovery failed".to_owned(),
        },
    );

    assert!(matches!(
        state.session,
        SessionState::AwaitingVerification { .. }
    ));
    assert_eq!(
        state.device_cleanup,
        DeviceCleanupState::Offered {
            reason: DeviceCleanupOfferReason::RecoveryFailed,
        }
    );
}

#[test]
fn unknown_trust_remains_retryable_without_offering_device_cleanup() {
    let mut state = AppState {
        session: SessionState::Provisional {
            info: session_info(),
            phase: ProvisionalPhase::CheckingTrust,
        },
        ..AppState::default()
    };

    reduce(
        &mut state,
        AppAction::CurrentDeviceTrustChanged(CurrentDeviceTrustState::Unknown),
    );

    assert!(matches!(
        state.session,
        SessionState::Provisional {
            phase: ProvisionalPhase::RecheckingTrust { failure: None },
            ..
        }
    ));
    assert_eq!(state.device_cleanup, DeviceCleanupState::Idle);
}

#[test]
fn no_proof_method_offers_explicit_cleanup_instead_of_auto_rejection() {
    let mut state = AppState {
        session: SessionState::Provisional {
            info: session_info(),
            phase: ProvisionalPhase::DiscoveringMethods,
        },
        ..AppState::default()
    };

    reduce(
        &mut state,
        AppAction::VerificationMethodsDiscovered(VerificationGateState {
            methods: vec![],
            account_kind: VerificationAccountKind::ExistingIdentity,
            failure: None,
        }),
    );

    assert!(matches!(
        &state.session,
        SessionState::AwaitingVerification { gate, .. }
            if gate.failure == Some(VerificationGateFailureKind::NoProofMethod)
    ));
    assert_eq!(
        state.device_cleanup,
        DeviceCleanupState::Offered {
            reason: DeviceCleanupOfferReason::NoProofMethod,
        }
    );
}

#[test]
fn device_cleanup_remote_failure_is_retryable_and_oauth_never_enters_uia() {
    let mut state = AppState {
        session: SessionState::AwaitingVerification {
            info: session_info(),
            gate: VerificationGateState {
                failure: Some(VerificationGateFailureKind::Sdk),
                ..recovery_gate()
            },
        },
        device_cleanup: DeviceCleanupState::Offered {
            reason: DeviceCleanupOfferReason::RecoveryFailed,
        },
        ..AppState::default()
    };

    reduce(
        &mut state,
        AppAction::DeviceCleanupStartRequested { request_id: 51 },
    );
    assert_eq!(
        state.device_cleanup,
        DeviceCleanupState::ResolvingRemote { request_id: 51 }
    );
    reduce(
        &mut state,
        AppAction::DeviceCleanupRemoteStarted {
            request_id: 51,
            auth_mode: DeviceCleanupAuthMode::OAuth,
        },
    );
    reduce(
        &mut state,
        AppAction::DeviceCleanupUiaRequired {
            request_id: 51,
            flow_id: 51,
        },
    );
    assert_eq!(
        state.device_cleanup,
        DeviceCleanupState::RemovingRemote {
            request_id: 51,
            auth_mode: DeviceCleanupAuthMode::OAuth,
        }
    );

    reduce(
        &mut state,
        AppAction::DeviceCleanupRemoteFailed {
            request_id: 51,
            auth_mode: DeviceCleanupAuthMode::OAuth,
            kind: DeviceCleanupFailureKind::Network,
        },
    );
    assert_eq!(
        state.device_cleanup,
        DeviceCleanupState::RemoteFailed {
            request_id: 51,
            auth_mode: DeviceCleanupAuthMode::OAuth,
            failure: DeviceCleanupFailureKind::Network,
        }
    );
    reduce(
        &mut state,
        AppAction::DeviceCleanupStartRequested { request_id: 52 },
    );
    assert_eq!(
        state.device_cleanup,
        DeviceCleanupState::ResolvingRemote { request_id: 52 }
    );
}

#[test]
fn device_cleanup_legacy_uia_requires_matching_request_and_flow() {
    let mut state = AppState {
        session: SessionState::AwaitingVerification {
            info: session_info(),
            gate: recovery_gate(),
        },
        device_cleanup: DeviceCleanupState::ResolvingRemote { request_id: 61 },
        ..AppState::default()
    };
    reduce(
        &mut state,
        AppAction::DeviceCleanupRemoteStarted {
            request_id: 61,
            auth_mode: DeviceCleanupAuthMode::Legacy,
        },
    );
    reduce(
        &mut state,
        AppAction::DeviceCleanupUiaRequired {
            request_id: 61,
            flow_id: 900,
        },
    );
    assert_eq!(
        state.device_cleanup,
        DeviceCleanupState::AwaitingUia {
            request_id: 61,
            flow_id: 900,
        }
    );

    let awaiting = state.clone();
    reduce(
        &mut state,
        AppAction::DeviceCleanupUiaSubmitted {
            request_id: 62,
            flow_id: 900,
        },
    );
    assert_eq!(state, awaiting);
    reduce(
        &mut state,
        AppAction::DeviceCleanupUiaSubmitted {
            request_id: 61,
            flow_id: 901,
        },
    );
    assert_eq!(state, awaiting);
    reduce(
        &mut state,
        AppAction::DeviceCleanupUiaSubmitted {
            request_id: 61,
            flow_id: 900,
        },
    );
    assert_eq!(
        state.device_cleanup,
        DeviceCleanupState::RemovingRemote {
            request_id: 61,
            auth_mode: DeviceCleanupAuthMode::Legacy,
        }
    );
}

#[test]
fn device_cleanup_success_and_already_absent_both_enter_local_reset() {
    for outcome in [
        DeviceCleanupRemoteOutcome::Success,
        DeviceCleanupRemoteOutcome::AlreadyAbsent,
    ] {
        let mut state = AppState {
            session: SessionState::AwaitingVerification {
                info: session_info(),
                gate: recovery_gate(),
            },
            device_cleanup: DeviceCleanupState::RemovingRemote {
                request_id: 71,
                auth_mode: DeviceCleanupAuthMode::Legacy,
            },
            ..AppState::default()
        };
        reduce(
            &mut state,
            AppAction::DeviceCleanupRemoteSettled {
                request_id: 71,
                outcome,
            },
        );
        assert_eq!(
            state.device_cleanup,
            DeviceCleanupState::ResettingLocal {
                request_id: 71,
                mode: DeviceCleanupLocalMode::RemoteRemoved { outcome },
            }
        );
    }
}

#[test]
fn device_cleanup_local_failure_retries_local_only_and_escape_is_separate() {
    let mut state = AppState {
        session: SessionState::AwaitingVerification {
            info: session_info(),
            gate: recovery_gate(),
        },
        device_cleanup: DeviceCleanupState::RemoteFailed {
            request_id: 81,
            auth_mode: DeviceCleanupAuthMode::Legacy,
            failure: DeviceCleanupFailureKind::Sdk,
        },
        ..AppState::default()
    };

    reduce(
        &mut state,
        AppAction::DeviceCleanupEraseLocalAnywayRequested { request_id: 82 },
    );
    assert_eq!(
        state.device_cleanup,
        DeviceCleanupState::ErasingLocalAnyway { request_id: 82 }
    );
    reduce(
        &mut state,
        AppAction::DeviceCleanupLocalResetFailed {
            request_id: 82,
            kind: DeviceCleanupFailureKind::LocalData,
        },
    );
    assert_eq!(
        state.device_cleanup,
        DeviceCleanupState::LocalResetFailed {
            request_id: 82,
            mode: DeviceCleanupLocalMode::RemoteMayRemain,
            failure: DeviceCleanupFailureKind::LocalData,
        }
    );
    reduce(
        &mut state,
        AppAction::DeviceCleanupStartRequested { request_id: 83 },
    );
    assert_eq!(
        state.device_cleanup,
        DeviceCleanupState::ErasingLocalAnyway { request_id: 83 }
    );
}

#[test]
fn device_cleanup_terminal_and_session_replacement_clear_the_slice() {
    let active = AppState {
        session: SessionState::AwaitingVerification {
            info: session_info(),
            gate: recovery_gate(),
        },
        device_cleanup: DeviceCleanupState::ResettingLocal {
            request_id: 91,
            mode: DeviceCleanupLocalMode::RemoteRemoved {
                outcome: DeviceCleanupRemoteOutcome::Success,
            },
        },
        ..AppState::default()
    };
    let mut completed = active.clone();
    reduce(
        &mut completed,
        AppAction::DeviceCleanupCompleted { request_id: 91 },
    );
    assert_eq!(completed.session, SessionState::SignedOut);
    assert_eq!(completed.device_cleanup, DeviceCleanupState::Idle);

    let mut replaced = active;
    reduce(&mut replaced, AppAction::LogoutRequested);
    assert_eq!(replaced.device_cleanup, DeviceCleanupState::Idle);
}
