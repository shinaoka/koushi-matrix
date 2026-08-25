use koushi_state::{
    AppAction, AppEffect, AppError, AppState, LoginAttemptId, SessionAuthenticationMethod,
    SessionInfo, SessionLockReason, SessionState, SlidingSyncAdmission, SlidingSyncAdmissionKind,
    SlidingSyncAdmissionSource, SlidingSyncCapabilityFailureKind, SlidingSyncCapabilityResult,
    SlidingSyncCapabilityState, SlidingSyncPositiveEvidence, SlidingSyncRevalidationState, reduce,
};

const ACCOUNT_EPOCH: u64 = 7;
const REQUEST_ID: u64 = 41;

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.invalid".to_owned(),
        user_id: "@alice:example.invalid".to_owned(),
        device_id: "ALICEDEVICE".to_owned(),
        authentication_method: SessionAuthenticationMethod::Password,
    }
}

fn positive_evidence(observed_at_ms: u64) -> SlidingSyncPositiveEvidence {
    SlidingSyncPositiveEvidence { observed_at_ms }
}

fn restore_admission() -> SlidingSyncAdmission {
    SlidingSyncAdmission::StoredSessionRestore {
        info: session_info(),
    }
}

fn start_restore_attempt(
    state: &mut AppState,
    request_id: u64,
    evidence: Option<SlidingSyncPositiveEvidence>,
) {
    state.session = SessionState::Restoring;
    let effects = reduce(
        state,
        AppAction::SlidingSyncCapabilityCheckStarted {
            account_epoch: ACCOUNT_EPOCH,
            request_id,
            admission: restore_admission(),
            positive_evidence: evidence,
        },
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(
            koushi_state::UiEvent::SessionChanged
        )]
    );
}

fn complete(
    state: &mut AppState,
    account_epoch: u64,
    request_id: u64,
    result: SlidingSyncCapabilityResult,
) -> Vec<AppEffect> {
    reduce(
        state,
        AppAction::SlidingSyncCapabilityCheckCompleted {
            account_epoch,
            request_id,
            result,
        },
    )
}

#[test]
fn supported_advances_login_and_restore_admission() {
    let evidence = positive_evidence(1_000);
    let login_attempt = LoginAttemptId::new(3, 9);

    for (session, admission, kind) in [
        (
            SessionState::Authenticating {
                homeserver: "https://matrix.example.invalid".to_owned(),
                attempt_id: login_attempt,
            },
            SlidingSyncAdmission::NewLogin {
                attempt_id: login_attempt,
            },
            SlidingSyncAdmissionKind::NewLogin,
        ),
        (
            SessionState::Restoring,
            restore_admission(),
            SlidingSyncAdmissionKind::StoredSessionRestore,
        ),
    ] {
        let mut state = AppState {
            session,
            ..AppState::default()
        };
        reduce(
            &mut state,
            AppAction::SlidingSyncCapabilityCheckStarted {
                account_epoch: ACCOUNT_EPOCH,
                request_id: REQUEST_ID,
                admission,
                positive_evidence: None,
            },
        );

        let effects = complete(
            &mut state,
            ACCOUNT_EPOCH,
            REQUEST_ID,
            SlidingSyncCapabilityResult::Supported {
                evidence: evidence.clone(),
            },
        );

        assert_eq!(
            state.sliding_sync_capability,
            SlidingSyncCapabilityState::Supported {
                account_epoch: ACCOUNT_EPOCH,
                request_id: REQUEST_ID,
                admission: if kind == SlidingSyncAdmissionKind::NewLogin {
                    SlidingSyncAdmission::NewLogin {
                        attempt_id: login_attempt,
                    }
                } else {
                    restore_admission()
                },
                evidence: evidence.clone(),
                revalidation: SlidingSyncRevalidationState::NotRequired,
            }
        );
        assert_eq!(
            effects,
            vec![
                AppEffect::ContinueSlidingSyncAdmission {
                    account_epoch: ACCOUNT_EPOCH,
                    request_id: REQUEST_ID,
                    admission: kind,
                    source: SlidingSyncAdmissionSource::Network,
                },
                AppEffect::EmitUiEvent(koushi_state::UiEvent::SessionChanged),
            ]
        );
    }
}

#[test]
fn stored_restore_capability_gate_accepts_only_the_active_switch_target() {
    let target = session_info();
    let mut state = AppState {
        session: SessionState::SwitchingAccount {
            info: target.clone(),
        },
        ..AppState::default()
    };
    let effects = reduce(
        &mut state,
        AppAction::SlidingSyncCapabilityCheckStarted {
            account_epoch: ACCOUNT_EPOCH,
            request_id: REQUEST_ID,
            admission: SlidingSyncAdmission::StoredSessionRestore {
                info: target.clone(),
            },
            positive_evidence: None,
        },
    );
    assert!(!effects.is_empty());
    assert!(
        complete(
            &mut state,
            ACCOUNT_EPOCH,
            REQUEST_ID,
            SlidingSyncCapabilityResult::Supported {
                evidence: positive_evidence(4_000),
            },
        )
        .iter()
        .any(|effect| matches!(effect, AppEffect::ContinueSlidingSyncAdmission { .. }))
    );

    let mut stale = AppState {
        session: SessionState::SwitchingAccount { info: target },
        ..AppState::default()
    };
    let mut other = session_info();
    other.user_id = "@other:example.invalid".to_owned();
    assert!(
        reduce(
            &mut stale,
            AppAction::SlidingSyncCapabilityCheckStarted {
                account_epoch: ACCOUNT_EPOCH,
                request_id: REQUEST_ID,
                admission: SlidingSyncAdmission::StoredSessionRestore { info: other },
                positive_evidence: None,
            },
        )
        .is_empty()
    );
}

#[test]
fn unsupported_unreachable_and_invalid_response_are_distinct_recoverable_blocks() {
    for (result, failure) in [
        (
            SlidingSyncCapabilityResult::Unsupported,
            SlidingSyncCapabilityFailureKind::Unsupported,
        ),
        (
            SlidingSyncCapabilityResult::Unreachable,
            SlidingSyncCapabilityFailureKind::Unreachable,
        ),
        (
            SlidingSyncCapabilityResult::InvalidResponse,
            SlidingSyncCapabilityFailureKind::InvalidResponse,
        ),
    ] {
        let mut state = AppState::default();
        start_restore_attempt(&mut state, REQUEST_ID, None);

        let effects = complete(&mut state, ACCOUNT_EPOCH, REQUEST_ID, result);

        assert_eq!(
            state.session,
            SessionState::CapabilityBlocked {
                info: session_info(),
                failure,
            }
        );
        assert_eq!(
            state.sliding_sync_capability,
            SlidingSyncCapabilityState::Blocked {
                account_epoch: ACCOUNT_EPOCH,
                request_id: REQUEST_ID,
                admission: restore_admission(),
                failure,
                positive_evidence: None,
            }
        );
        assert_eq!(
            effects,
            vec![AppEffect::EmitUiEvent(
                koushi_state::UiEvent::SessionChanged
            )]
        );
    }
}

#[test]
fn stale_request_or_account_epoch_completion_is_ignored() {
    let mut state = AppState::default();
    start_restore_attempt(&mut state, REQUEST_ID, None);

    for (account_epoch, request_id) in [
        (ACCOUNT_EPOCH, REQUEST_ID - 1),
        (ACCOUNT_EPOCH - 1, REQUEST_ID),
    ] {
        let before = state.clone();
        let effects = complete(
            &mut state,
            account_epoch,
            request_id,
            SlidingSyncCapabilityResult::Supported {
                evidence: positive_evidence(2_000),
            },
        );

        assert!(effects.is_empty());
        assert_eq!(state, before);
    }
}

#[test]
fn retry_clears_only_the_current_capability_attempt() {
    let mut state = AppState {
        errors: vec![AppError {
            code: "unrelated".to_owned(),
            message: "preserve this local failure".to_owned(),
            recoverable: true,
        }],
        ..AppState::default()
    };
    start_restore_attempt(&mut state, REQUEST_ID, None);
    complete(
        &mut state,
        ACCOUNT_EPOCH,
        REQUEST_ID,
        SlidingSyncCapabilityResult::InvalidResponse,
    );

    let blocked = state.clone();
    assert!(
        reduce(
            &mut state,
            AppAction::SlidingSyncCapabilityRetryAccepted {
                account_epoch: ACCOUNT_EPOCH,
                blocked_request_id: REQUEST_ID - 1,
                request_id: REQUEST_ID + 1,
            },
        )
        .is_empty()
    );
    assert_eq!(state, blocked);

    let effects = reduce(
        &mut state,
        AppAction::SlidingSyncCapabilityRetryAccepted {
            account_epoch: ACCOUNT_EPOCH,
            blocked_request_id: REQUEST_ID,
            request_id: REQUEST_ID + 1,
        },
    );

    assert_eq!(state.session, SessionState::Restoring);
    assert_eq!(state.errors, blocked.errors);
    assert_eq!(
        state.sliding_sync_capability,
        SlidingSyncCapabilityState::Checking {
            account_epoch: ACCOUNT_EPOCH,
            request_id: REQUEST_ID + 1,
            admission: restore_admission(),
            positive_evidence: None,
        }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::RetrySlidingSyncCapabilityDiscovery {
                account_epoch: ACCOUNT_EPOCH,
                blocked_request_id: REQUEST_ID,
                request_id: REQUEST_ID + 1,
            },
            AppEffect::EmitUiEvent(koushi_state::UiEvent::SessionChanged),
        ]
    );
}

#[test]
fn blocking_preserves_local_identity_and_positive_support_evidence() {
    let evidence = positive_evidence(3_000);
    let mut state = AppState {
        errors: vec![AppError {
            code: "local-store-marker".to_owned(),
            message: "local state must survive capability blocking".to_owned(),
            recoverable: true,
        }],
        ..AppState::default()
    };
    start_restore_attempt(&mut state, REQUEST_ID, Some(evidence.clone()));
    state.session_lock_reason = Some(SessionLockReason::UnknownToken { soft_logout: false });

    complete(
        &mut state,
        ACCOUNT_EPOCH,
        REQUEST_ID,
        SlidingSyncCapabilityResult::Unsupported,
    );

    assert_eq!(state.session_lock_reason, None);
    assert_eq!(
        state.session,
        SessionState::CapabilityBlocked {
            info: session_info(),
            failure: SlidingSyncCapabilityFailureKind::Unsupported,
        }
    );
    assert_eq!(state.errors[0].code, "local-store-marker");
    assert!(matches!(
        &state.sliding_sync_capability,
        SlidingSyncCapabilityState::Blocked {
            admission: SlidingSyncAdmission::StoredSessionRestore { info, .. },
            positive_evidence: Some(saved),
            ..
        } if info == &session_info() && saved == &evidence
    ));
}

#[test]
fn positive_cache_admits_offline_restore_and_schedules_revalidation() {
    let evidence = positive_evidence(4_000);
    let mut state = AppState::default();
    start_restore_attempt(&mut state, REQUEST_ID, Some(evidence.clone()));

    let effects = complete(
        &mut state,
        ACCOUNT_EPOCH,
        REQUEST_ID,
        SlidingSyncCapabilityResult::Unreachable,
    );

    assert_eq!(state.session, SessionState::Restoring);
    assert_eq!(
        state.sliding_sync_capability,
        SlidingSyncCapabilityState::Supported {
            account_epoch: ACCOUNT_EPOCH,
            request_id: REQUEST_ID,
            admission: restore_admission(),
            evidence,
            revalidation: SlidingSyncRevalidationState::Pending {
                failure: SlidingSyncCapabilityFailureKind::Unreachable,
            },
        }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::ContinueSlidingSyncAdmission {
                account_epoch: ACCOUNT_EPOCH,
                request_id: REQUEST_ID,
                admission: SlidingSyncAdmissionKind::StoredSessionRestore,
                source: SlidingSyncAdmissionSource::PositiveCache,
            },
            AppEffect::ScheduleSlidingSyncCapabilityRevalidation {
                account_epoch: ACCOUNT_EPOCH,
            },
            AppEffect::EmitUiEvent(koushi_state::UiEvent::SessionChanged),
        ]
    );
}

#[test]
fn absent_positive_cache_cannot_manufacture_support() {
    let mut state = AppState::default();
    start_restore_attempt(&mut state, REQUEST_ID, None);

    let effects = complete(
        &mut state,
        ACCOUNT_EPOCH,
        REQUEST_ID,
        SlidingSyncCapabilityResult::Unreachable,
    );

    assert!(matches!(
        state.sliding_sync_capability,
        SlidingSyncCapabilityState::Blocked {
            failure: SlidingSyncCapabilityFailureKind::Unreachable,
            positive_evidence: None,
            ..
        }
    ));
    assert!(!effects.iter().any(|effect| matches!(
        effect,
        AppEffect::ContinueSlidingSyncAdmission { .. }
            | AppEffect::ScheduleSlidingSyncCapabilityRevalidation { .. }
    )));
}

#[test]
fn unsupported_is_never_mislabeled_as_offline_even_with_positive_cache() {
    let evidence = positive_evidence(5_000);
    let mut state = AppState::default();
    start_restore_attempt(&mut state, REQUEST_ID, Some(evidence.clone()));

    let effects = complete(
        &mut state,
        ACCOUNT_EPOCH,
        REQUEST_ID,
        SlidingSyncCapabilityResult::Unsupported,
    );

    assert_eq!(
        state.session,
        SessionState::CapabilityBlocked {
            info: session_info(),
            failure: SlidingSyncCapabilityFailureKind::Unsupported,
        }
    );
    assert!(matches!(
        state.sliding_sync_capability,
        SlidingSyncCapabilityState::Blocked {
            failure: SlidingSyncCapabilityFailureKind::Unsupported,
            positive_evidence: Some(saved),
            ..
        } if saved == evidence
    ));
    assert!(!effects.iter().any(|effect| matches!(
        effect,
        AppEffect::ContinueSlidingSyncAdmission { .. }
            | AppEffect::ScheduleSlidingSyncCapabilityRevalidation { .. }
    )));
}

#[test]
fn logout_and_account_replacement_retire_attempts_and_late_completions() {
    let mut state = AppState::default();
    start_restore_attempt(&mut state, REQUEST_ID, None);

    reduce(&mut state, AppAction::LogoutRequested);
    assert_eq!(
        state.sliding_sync_capability,
        SlidingSyncCapabilityState::Unknown
    );
    let logged_out = state.clone();
    assert!(
        complete(
            &mut state,
            ACCOUNT_EPOCH,
            REQUEST_ID,
            SlidingSyncCapabilityResult::Supported {
                evidence: positive_evidence(6_000),
            },
        )
        .is_empty()
    );
    assert_eq!(state, logged_out);

    reduce(&mut state, AppAction::LogoutFinished);
    assert_eq!(state.sliding_sync_account_epoch, ACCOUNT_EPOCH);
    assert_eq!(
        state.sliding_sync_capability,
        SlidingSyncCapabilityState::Unknown
    );

    let mut state = AppState::default();
    start_restore_attempt(&mut state, REQUEST_ID, None);
    complete(
        &mut state,
        ACCOUNT_EPOCH,
        REQUEST_ID,
        SlidingSyncCapabilityResult::Unsupported,
    );
    reduce(
        &mut state,
        AppAction::SwitchAccountRequested {
            info: SessionInfo {
                homeserver: "https://other.example.invalid".to_owned(),
                user_id: "@bob:example.invalid".to_owned(),
                device_id: "BOBDEVICE".to_owned(),
                authentication_method: SessionAuthenticationMethod::Password,
            },
        },
    );
    assert_eq!(
        state.sliding_sync_capability,
        SlidingSyncCapabilityState::Unknown
    );
    let replaced = state.clone();
    assert!(
        reduce(
            &mut state,
            AppAction::SlidingSyncCapabilityRetryAccepted {
                account_epoch: ACCOUNT_EPOCH,
                blocked_request_id: REQUEST_ID,
                request_id: REQUEST_ID + 1,
            },
        )
        .is_empty()
    );
    assert_eq!(state, replaced);
}

#[test]
fn process_local_capability_correlation_is_not_serialized() {
    let mut state = AppState::default();
    start_restore_attempt(&mut state, REQUEST_ID, None);

    let serialized = serde_json::to_value(&state).expect("serialize app state");
    assert!(serialized.get("sliding_sync_account_epoch").is_none());
    assert!(serialized.get("sliding_sync_capability").is_none());

    let restored: AppState = serde_json::from_value(serialized).expect("deserialize app state");
    assert_eq!(restored.sliding_sync_account_epoch, 0);
    assert_eq!(
        restored.sliding_sync_capability,
        SlidingSyncCapabilityState::Unknown
    );
}

#[test]
fn duplicate_completion_and_reused_retry_request_are_ignored() {
    let mut state = AppState::default();
    start_restore_attempt(&mut state, REQUEST_ID, None);
    complete(
        &mut state,
        ACCOUNT_EPOCH,
        REQUEST_ID,
        SlidingSyncCapabilityResult::Supported {
            evidence: positive_evidence(7_000),
        },
    );
    let completed = state.clone();
    assert!(
        complete(
            &mut state,
            ACCOUNT_EPOCH,
            REQUEST_ID,
            SlidingSyncCapabilityResult::Unsupported,
        )
        .is_empty()
    );
    assert_eq!(state, completed);

    let mut state = AppState::default();
    start_restore_attempt(&mut state, REQUEST_ID, None);
    complete(
        &mut state,
        ACCOUNT_EPOCH,
        REQUEST_ID,
        SlidingSyncCapabilityResult::Unsupported,
    );
    let blocked = state.clone();
    assert!(
        reduce(
            &mut state,
            AppAction::SlidingSyncCapabilityRetryAccepted {
                account_epoch: ACCOUNT_EPOCH,
                blocked_request_id: REQUEST_ID,
                request_id: REQUEST_ID,
            },
        )
        .is_empty()
    );
    assert_eq!(state, blocked);
}

#[test]
fn delayed_starts_cannot_replace_retry_revalidation_or_a_new_account_epoch() {
    let mut state = AppState::default();
    start_restore_attempt(&mut state, REQUEST_ID, None);
    complete(
        &mut state,
        ACCOUNT_EPOCH,
        REQUEST_ID,
        SlidingSyncCapabilityResult::Unsupported,
    );
    reduce(
        &mut state,
        AppAction::SlidingSyncCapabilityRetryAccepted {
            account_epoch: ACCOUNT_EPOCH,
            blocked_request_id: REQUEST_ID,
            request_id: REQUEST_ID + 1,
        },
    );
    let retry = state.clone();
    assert!(
        reduce(
            &mut state,
            AppAction::SlidingSyncCapabilityCheckStarted {
                account_epoch: ACCOUNT_EPOCH,
                request_id: REQUEST_ID + 2,
                admission: restore_admission(),
                positive_evidence: None,
            },
        )
        .is_empty()
    );
    assert_eq!(state, retry);

    reduce(&mut state, AppAction::LogoutRequested);
    state.session = SessionState::Restoring;
    let replaced = state.clone();
    assert!(
        reduce(
            &mut state,
            AppAction::SlidingSyncCapabilityCheckStarted {
                account_epoch: ACCOUNT_EPOCH,
                request_id: REQUEST_ID + 3,
                admission: restore_admission(),
                positive_evidence: None,
            },
        )
        .is_empty()
    );
    assert_eq!(state, replaced);

    assert!(
        !reduce(
            &mut state,
            AppAction::SlidingSyncCapabilityCheckStarted {
                account_epoch: ACCOUNT_EPOCH + 1,
                request_id: 1,
                admission: restore_admission(),
                positive_evidence: None,
            },
        )
        .is_empty()
    );
}

#[test]
fn new_login_failures_are_retryable_but_a_newer_attempt_retires_them() {
    let attempt = LoginAttemptId::new(3, 9);
    for result in [
        SlidingSyncCapabilityResult::Unsupported,
        SlidingSyncCapabilityResult::Unreachable,
        SlidingSyncCapabilityResult::InvalidResponse,
    ] {
        let mut state = AppState {
            session: SessionState::Authenticating {
                homeserver: "https://matrix.example.invalid".to_owned(),
                attempt_id: attempt,
            },
            ..AppState::default()
        };
        reduce(
            &mut state,
            AppAction::SlidingSyncCapabilityCheckStarted {
                account_epoch: ACCOUNT_EPOCH,
                request_id: REQUEST_ID,
                admission: SlidingSyncAdmission::NewLogin {
                    attempt_id: attempt,
                },
                positive_evidence: None,
            },
        );
        complete(&mut state, ACCOUNT_EPOCH, REQUEST_ID, result);
        assert!(matches!(
            state.sliding_sync_capability,
            SlidingSyncCapabilityState::Blocked {
                admission: SlidingSyncAdmission::NewLogin { attempt_id },
                ..
            } if attempt_id == attempt
        ));
        assert!(matches!(
            state.session,
            SessionState::Authenticating { attempt_id, .. } if attempt_id == attempt
        ));

        let retry = reduce(
            &mut state,
            AppAction::SlidingSyncCapabilityRetryAccepted {
                account_epoch: ACCOUNT_EPOCH,
                blocked_request_id: REQUEST_ID,
                request_id: REQUEST_ID + 1,
            },
        );
        assert!(!retry.is_empty());

        let newer = LoginAttemptId::new(3, 10);
        reduce(
            &mut state,
            AppAction::AuthenticationStarted {
                attempt_id: newer,
                homeserver: "https://other.example.invalid".to_owned(),
            },
        );
        assert_eq!(
            state.sliding_sync_capability,
            SlidingSyncCapabilityState::Unknown
        );
        assert!(
            complete(
                &mut state,
                ACCOUNT_EPOCH,
                REQUEST_ID + 1,
                SlidingSyncCapabilityResult::Supported {
                    evidence: positive_evidence(8_000),
                },
            )
            .is_empty()
        );
    }
}

#[test]
fn cached_restore_revalidation_blocks_only_explicit_unsupported() {
    let evidence = positive_evidence(9_000);
    let mut state = AppState::default();
    start_restore_attempt(&mut state, REQUEST_ID, Some(evidence.clone()));
    complete(
        &mut state,
        ACCOUNT_EPOCH,
        REQUEST_ID,
        SlidingSyncCapabilityResult::Unreachable,
    );

    let restoring = state.clone();
    assert!(
        reduce(
            &mut state,
            AppAction::SlidingSyncCapabilityRevalidationStarted {
                account_epoch: ACCOUNT_EPOCH,
                request_id: REQUEST_ID + 1,
            },
        )
        .is_empty()
    );
    assert_eq!(state, restoring);

    state.session = SessionState::Ready(session_info());

    let pending = state.clone();
    assert!(
        reduce(
            &mut state,
            AppAction::SlidingSyncCapabilityRevalidationStarted {
                account_epoch: ACCOUNT_EPOCH,
                request_id: REQUEST_ID,
            },
        )
        .is_empty()
    );
    assert_eq!(state, pending);

    assert_eq!(
        reduce(
            &mut state,
            AppAction::SlidingSyncCapabilityRevalidationStarted {
                account_epoch: ACCOUNT_EPOCH,
                request_id: REQUEST_ID + 1,
            },
        ),
        vec![AppEffect::EmitUiEvent(
            koushi_state::UiEvent::SessionChanged
        )]
    );
    assert!(matches!(
        state.sliding_sync_capability,
        SlidingSyncCapabilityState::Supported {
            revalidation: SlidingSyncRevalidationState::Checking { request_id },
            ..
        } if request_id == REQUEST_ID + 1
    ));

    state.session = SessionState::Locked(session_info());

    let retryable = reduce(
        &mut state,
        AppAction::SlidingSyncCapabilityRevalidationCompleted {
            account_epoch: ACCOUNT_EPOCH,
            request_id: REQUEST_ID + 1,
            result: SlidingSyncCapabilityResult::InvalidResponse,
        },
    );
    assert_eq!(state.session, SessionState::Locked(session_info()));
    assert!(matches!(
        state.sliding_sync_capability,
        SlidingSyncCapabilityState::Supported {
            revalidation: SlidingSyncRevalidationState::Pending {
                failure: SlidingSyncCapabilityFailureKind::InvalidResponse,
            },
            ..
        }
    ));
    assert_eq!(
        retryable,
        vec![
            AppEffect::SettleSlidingSyncCapabilityRevalidation {
                account_epoch: ACCOUNT_EPOCH,
                request_id: REQUEST_ID + 1,
                result: SlidingSyncCapabilityResult::InvalidResponse,
            },
            AppEffect::EmitUiEvent(koushi_state::UiEvent::SessionChanged),
        ]
    );

    state.session = SessionState::Ready(session_info());
    reduce(
        &mut state,
        AppAction::SlidingSyncCapabilityRevalidationStarted {
            account_epoch: ACCOUNT_EPOCH,
            request_id: REQUEST_ID + 2,
        },
    );
    state.session = SessionState::Locked(session_info());
    state.session_lock_reason = Some(SessionLockReason::UnknownToken { soft_logout: false });
    let blocked = reduce(
        &mut state,
        AppAction::SlidingSyncCapabilityRevalidationCompleted {
            account_epoch: ACCOUNT_EPOCH,
            request_id: REQUEST_ID + 2,
            result: SlidingSyncCapabilityResult::Unsupported,
        },
    );
    assert!(matches!(
        state.session,
        SessionState::CapabilityBlocked {
            failure: SlidingSyncCapabilityFailureKind::Unsupported,
            ..
        }
    ));
    assert_eq!(state.session_lock_reason, None);
    assert!(
        blocked.contains(&AppEffect::SettleSlidingSyncCapabilityRevalidation {
            account_epoch: ACCOUNT_EPOCH,
            request_id: REQUEST_ID + 2,
            result: SlidingSyncCapabilityResult::Unsupported,
        })
    );
}
