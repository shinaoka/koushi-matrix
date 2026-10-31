use koushi_state::{
    AppAction, AppEffect, AppState, CurrentDeviceTrustState, CurrentSessionBackupState,
    CurrentSessionStatusDetails, CurrentSessionStatusFailureKind, CurrentSessionStatusState,
    CurrentSessionSyncState, OwnIdentityVerification, SessionAuthenticationMethod, SessionInfo,
    SessionState, SessionStatusRefreshTrigger, SyncLifecycleStatus, SyncState, reduce,
};

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@user:example.invalid".to_owned(),
            device_id: "DEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        ..AppState::default()
    }
}

fn details(
    is_cross_signed_by_owner: bool,
    own_identity: OwnIdentityVerification,
) -> CurrentSessionStatusDetails {
    CurrentSessionStatusDetails::new(
        Some("Koushi on Linux".to_owned()),
        "DEVICE".to_owned(),
        SessionAuthenticationMethod::OAuth,
        CurrentSessionSyncState::Running,
        CurrentDeviceTrustState::Verified,
        is_cross_signed_by_owner,
        own_identity,
        CurrentSessionBackupState::Ready,
        1_234,
    )
}

#[test]
fn refresh_enters_checking_and_emits_one_correlated_effect() {
    let mut state = ready_state();

    let effects = reduce(
        &mut state,
        AppAction::CurrentSessionStatusRefreshRequested {
            request_id: 7,
            trigger: SessionStatusRefreshTrigger::Open,
            now_ms: 0,
        },
    );

    assert_eq!(
        state.current_session_status,
        CurrentSessionStatusState::Checking {
            request_id: 7,
            trigger: SessionStatusRefreshTrigger::Open,
            last_known_details: None,
            consecutive_failures: 0,
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::RefreshCurrentSessionStatus {
            request_id: 7,
            trigger: SessionStatusRefreshTrigger::Open,
        }]
    );
}

#[test]
fn duplicate_refresh_is_rejected_while_checking() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::CurrentSessionStatusRefreshRequested {
            request_id: 7,
            trigger: SessionStatusRefreshTrigger::Open,
            now_ms: 0,
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::CurrentSessionStatusRefreshRequested {
            request_id: 8,
            trigger: SessionStatusRefreshTrigger::Manual,
            now_ms: 0,
        },
    );

    assert!(effects.is_empty());
    assert!(matches!(
        state.current_session_status,
        CurrentSessionStatusState::Checking { request_id: 7, .. }
    ));
}

#[test]
fn correlated_completion_settles_ready_and_derives_verified_once_in_rust() {
    let mut state = ready_state();
    state.current_session_status = CurrentSessionStatusState::Checking {
        request_id: 7,
        trigger: SessionStatusRefreshTrigger::Manual,
        last_known_details: None,
        consecutive_failures: 0,
    };

    reduce(
        &mut state,
        AppAction::CurrentSessionStatusRefreshed {
            request_id: 7,
            details: details(true, OwnIdentityVerification::Verified),
        },
    );

    let CurrentSessionStatusState::Ready {
        request_id,
        details,
    } = &state.current_session_status
    else {
        panic!("expected ready status");
    };
    assert_eq!(*request_id, 7);
    assert_eq!(details.verification, CurrentDeviceTrustState::Verified);
}

#[test]
fn supplemental_identity_facts_do_not_override_authoritative_device_verification() {
    assert_eq!(
        details(true, OwnIdentityVerification::Unverified).verification,
        CurrentDeviceTrustState::Verified
    );
    assert_eq!(
        CurrentSessionStatusDetails::new(
            None,
            "DEVICE".to_owned(),
            SessionAuthenticationMethod::Unknown,
            CurrentSessionSyncState::Running,
            CurrentDeviceTrustState::Unknown,
            true,
            OwnIdentityVerification::Verified,
            CurrentSessionBackupState::Ready,
            1_235,
        )
        .verification,
        CurrentDeviceTrustState::Unknown
    );
}

#[test]
fn failed_refresh_preserves_prior_ready_facts() {
    let mut state = ready_state();
    state.current_session_status = CurrentSessionStatusState::Ready {
        request_id: 6,
        details: details(true, OwnIdentityVerification::Verified),
    };
    reduce(
        &mut state,
        AppAction::CurrentSessionStatusRefreshRequested {
            request_id: 7,
            trigger: SessionStatusRefreshTrigger::Manual,
            now_ms: 0,
        },
    );

    reduce(
        &mut state,
        AppAction::CurrentSessionStatusRefreshFailed {
            request_id: 7,
            kind: CurrentSessionStatusFailureKind::Sdk,
            checked_at_ms: 1_235,
        },
    );

    assert_eq!(
        state.current_session_status,
        CurrentSessionStatusState::Failed {
            request_id: 7,
            kind: CurrentSessionStatusFailureKind::Sdk,
            checked_at_ms: 1_235,
            last_known_details: Some(details(true, OwnIdentityVerification::Verified)),
            consecutive_failures: 1,
        }
    );
}

#[test]
fn stale_completion_cannot_replace_the_current_request() {
    let mut state = ready_state();
    state.current_session_status = CurrentSessionStatusState::Checking {
        request_id: 8,
        trigger: SessionStatusRefreshTrigger::Manual,
        last_known_details: None,
        consecutive_failures: 0,
    };

    let effects = reduce(
        &mut state,
        AppAction::CurrentSessionStatusRefreshed {
            request_id: 7,
            details: details(true, OwnIdentityVerification::Verified),
        },
    );

    assert!(effects.is_empty());
    assert!(matches!(
        state.current_session_status,
        CurrentSessionStatusState::Checking { request_id: 8, .. }
    ));
}

#[test]
fn trust_loss_clears_status_and_late_completions_stay_idle() {
    let mut state = ready_state();
    state.current_session_status = CurrentSessionStatusState::Checking {
        request_id: 41,
        trigger: SessionStatusRefreshTrigger::Manual,
        last_known_details: None,
        consecutive_failures: 0,
    };

    reduce(
        &mut state,
        AppAction::CurrentDeviceTrustChanged(CurrentDeviceTrustState::Unverified),
    );
    assert_eq!(
        state.current_session_status,
        CurrentSessionStatusState::Idle
    );

    let details = CurrentSessionStatusDetails::new(
        None,
        "DEVICE".to_owned(),
        SessionAuthenticationMethod::Unknown,
        CurrentSessionSyncState::Running,
        CurrentDeviceTrustState::Verified,
        true,
        OwnIdentityVerification::Verified,
        CurrentSessionBackupState::Ready,
        2_000,
    );
    reduce(
        &mut state,
        AppAction::CurrentSessionStatusRefreshed {
            request_id: 41,
            details,
        },
    );
    reduce(
        &mut state,
        AppAction::CurrentSessionStatusRefreshFailed {
            request_id: 41,
            kind: CurrentSessionStatusFailureKind::Sdk,
            checked_at_ms: 2_001,
        },
    );
    assert_eq!(
        state.current_session_status,
        CurrentSessionStatusState::Idle
    );
}

#[test]
fn logout_resets_current_session_status() {
    let mut state = ready_state();
    state.current_session_status = CurrentSessionStatusState::Ready {
        request_id: 7,
        details: details(true, OwnIdentityVerification::Verified),
    };

    reduce(&mut state, AppAction::LogoutRequested);

    assert_eq!(
        state.current_session_status,
        CurrentSessionStatusState::Idle
    );
}

#[test]
fn connectivity_recovery_rearms_and_a_due_recovery_check_coalesces_a_later_manual_retry() {
    let mut state = ready_state();
    state.sync = SyncState::Reconnecting {
        reason: "transport".to_owned(),
    };
    state.current_session_status = CurrentSessionStatusState::Failed {
        request_id: 40,
        kind: CurrentSessionStatusFailureKind::Network,
        checked_at_ms: 2_000,
        last_known_details: Some(details(true, OwnIdentityVerification::Verified)),
        consecutive_failures: 0,
    };

    let effects = reduce(
        &mut state,
        AppAction::SyncStatusChanged {
            generation: 41,
            status: SyncLifecycleStatus::Running,
        },
    );

    // #1009: the Running edge only re-arms the Core timer; it never issues a
    // check or mints a request id itself.
    let token = state.current_session_status_schedule.token;
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(koushi_state::UiEvent::RoomListChanged),
            AppEffect::SyncConnectivityChanged { proven: true },
            AppEffect::ArmCurrentSessionStatusCheck {
                token,
                due_at_ms: 62_000,
            },
        ]
    );
    assert!(matches!(
        state.current_session_status,
        CurrentSessionStatusState::Failed { request_id: 40, .. }
    ));

    let effects = reduce(
        &mut state,
        AppAction::CurrentSessionStatusCheckDue {
            token,
            now_ms: 62_000,
        },
    );
    let CurrentSessionStatusState::Checking {
        request_id: recovery_request,
        trigger: SessionStatusRefreshTrigger::Recovery,
        last_known_details: Some(_),
        consecutive_failures: 0,
    } = state.current_session_status
    else {
        panic!("a due recovery check must enter Checking");
    };
    assert_eq!(
        effects,
        vec![AppEffect::RefreshCurrentSessionStatus {
            request_id: recovery_request,
            trigger: SessionStatusRefreshTrigger::Recovery,
        }]
    );

    let duplicate_effects = reduce(
        &mut state,
        AppAction::CurrentSessionStatusRefreshRequested {
            request_id: 42,
            trigger: SessionStatusRefreshTrigger::Manual,
            now_ms: 0,
        },
    );
    assert!(duplicate_effects.is_empty());
    assert!(matches!(
        state.current_session_status,
        CurrentSessionStatusState::Checking { request_id, .. } if request_id == recovery_request
    ));

    reduce(
        &mut state,
        AppAction::CurrentSessionStatusRefreshed {
            request_id: recovery_request,
            details: details(true, OwnIdentityVerification::Verified),
        },
    );
    assert!(matches!(
        state.current_session_status,
        CurrentSessionStatusState::Ready { request_id, .. } if request_id == recovery_request
    ));
}

#[test]
fn connectivity_recovery_keeps_checking_request_and_counts_its_late_failure() {
    let mut state = ready_state();
    state.sync = SyncState::Running;
    state.current_session_status = CurrentSessionStatusState::Checking {
        request_id: 42,
        trigger: SessionStatusRefreshTrigger::Manual,
        last_known_details: Some(details(true, OwnIdentityVerification::Verified)),
        consecutive_failures: 0,
    };

    reduce(
        &mut state,
        AppAction::SyncStatusChanged {
            generation: 41,
            status: SyncLifecycleStatus::Reconnecting {
                reason: "transport".to_owned(),
            },
        },
    );
    let effects = reduce(
        &mut state,
        AppAction::SyncStatusChanged {
            generation: 42,
            status: SyncLifecycleStatus::Running,
        },
    );

    // #1009: the in-flight request is joined, never replaced.
    assert!(effects.contains(&AppEffect::SyncConnectivityChanged { proven: true }));
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::RefreshCurrentSessionStatus { .. }))
    );
    assert!(matches!(
        state.current_session_status,
        CurrentSessionStatusState::Checking { request_id: 42, .. }
    ));

    // The actor settles the request it cancelled on the outage after the
    // Running edge; the failure is current and counts toward the backoff.
    let effects = reduce(
        &mut state,
        AppAction::CurrentSessionStatusRefreshFailed {
            request_id: 42,
            kind: CurrentSessionStatusFailureKind::ConnectivityUnavailable,
            checked_at_ms: 2_001,
        },
    );
    assert!(matches!(
        state.current_session_status,
        CurrentSessionStatusState::Failed {
            request_id: 42,
            consecutive_failures: 1,
            ..
        }
    ));
    let token = state.current_session_status_schedule.token;
    assert_eq!(
        effects,
        vec![AppEffect::ArmCurrentSessionStatusCheck {
            token,
            due_at_ms: 2_001 + 60_000,
        }]
    );
}

#[test]
fn connectivity_recovery_after_cancelled_request_fails_first_waits_for_backoff() {
    let mut state = ready_state();
    state.sync = SyncState::Running;
    state.current_session_status = CurrentSessionStatusState::Checking {
        request_id: 40,
        trigger: SessionStatusRefreshTrigger::Manual,
        last_known_details: None,
        consecutive_failures: 0,
    };

    reduce(
        &mut state,
        AppAction::SyncStatusChanged {
            generation: 41,
            status: SyncLifecycleStatus::Reconnecting {
                reason: "transport".to_owned(),
            },
        },
    );
    reduce(
        &mut state,
        AppAction::CurrentSessionStatusRefreshFailed {
            request_id: 40,
            kind: CurrentSessionStatusFailureKind::ConnectivityUnavailable,
            checked_at_ms: 2_001,
        },
    );
    assert!(matches!(
        state.current_session_status,
        CurrentSessionStatusState::Failed {
            request_id: 40,
            kind: CurrentSessionStatusFailureKind::ConnectivityUnavailable,
            ..
        }
    ));

    let effects = reduce(
        &mut state,
        AppAction::SyncStatusChanged {
            generation: 42,
            status: SyncLifecycleStatus::Running,
        },
    );
    let token = state.current_session_status_schedule.token;
    assert!(effects.contains(&AppEffect::ArmCurrentSessionStatusCheck {
        token,
        due_at_ms: 2_001 + 60_000,
    }));
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::RefreshCurrentSessionStatus { .. }))
    );

    let effects = reduce(
        &mut state,
        AppAction::CurrentSessionStatusCheckDue {
            token,
            now_ms: 2_001 + 60_000,
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [AppEffect::RefreshCurrentSessionStatus {
            trigger: SessionStatusRefreshTrigger::Recovery,
            ..
        }]
    ));
}

#[test]
fn timeout_preserves_last_known_session_facts() {
    let mut state = ready_state();
    state.sync = SyncState::Running;
    let known = details(true, OwnIdentityVerification::Verified);
    state.current_session_status = CurrentSessionStatusState::Ready {
        request_id: 6,
        details: known.clone(),
    };

    reduce(
        &mut state,
        AppAction::CurrentSessionStatusRefreshRequested {
            request_id: 7,
            trigger: SessionStatusRefreshTrigger::Manual,
            now_ms: 0,
        },
    );
    reduce(
        &mut state,
        AppAction::CurrentSessionStatusRefreshFailed {
            request_id: 7,
            kind: CurrentSessionStatusFailureKind::TimedOut,
            checked_at_ms: 2_001,
        },
    );

    assert_eq!(
        state.current_session_status,
        CurrentSessionStatusState::Failed {
            request_id: 7,
            kind: CurrentSessionStatusFailureKind::TimedOut,
            checked_at_ms: 2_001,
            last_known_details: Some(known),
            consecutive_failures: 1,
        }
    );
}

#[test]
fn legacy_session_info_defaults_authentication_method_to_unknown() {
    let info: SessionInfo = serde_json::from_value(serde_json::json!({
        "homeserver": "https://example.invalid",
        "user_id": "@user:example.invalid",
        "device_id": "DEVICE"
    }))
    .expect("legacy session info");

    assert_eq!(
        info.authentication_method,
        SessionAuthenticationMethod::Unknown
    );
}

#[test]
fn session_info_serializes_only_the_coarse_authentication_method() {
    let info: SessionInfo = serde_json::from_value(serde_json::json!({
        "homeserver": "https://example.invalid",
        "user_id": "@user:example.invalid",
        "device_id": "DEVICE",
        "authentication_method": "oauth"
    }))
    .expect("session info");

    let serialized = serde_json::to_string(&info).expect("serialize session info");
    assert!(serialized.contains(r#""authentication_method":"oauth""#));
    assert!(!serialized.contains("access_token"));
    assert!(!serialized.contains("refresh_token"));
}

const HOUR_MS: u64 = 60 * 60 * 1_000;
const MINUTE_MS: u64 = 60 * 1_000;

fn ready_at(checked_at_ms: u64) -> CurrentSessionStatusState {
    let mut details = details(true, OwnIdentityVerification::Verified);
    details.checked_at_ms = checked_at_ms;
    CurrentSessionStatusState::Ready {
        request_id: 1,
        details,
    }
}

fn request(
    state: &mut AppState,
    request_id: u64,
    trigger: SessionStatusRefreshTrigger,
    now_ms: u64,
) -> Vec<AppEffect> {
    reduce(
        state,
        AppAction::CurrentSessionStatusRefreshRequested {
            request_id,
            trigger,
            now_ms,
        },
    )
}

/// #982: opening the session-status panel issued a full remote inspection every
/// time — devices, own identity, crypto device, backup probe. Ordinary opens
/// must serve the last known status instead.
#[test]
fn repeated_panel_opens_serve_the_last_known_status_until_it_goes_stale() {
    let mut state = ready_state();
    let fresh = ready_at(10 * HOUR_MS);
    state.current_session_status = fresh.clone();

    for (attempt, elapsed) in [MINUTE_MS, 30 * MINUTE_MS, 2 * HOUR_MS].iter().enumerate() {
        let effects = request(
            &mut state,
            100 + attempt as u64,
            SessionStatusRefreshTrigger::Open,
            10 * HOUR_MS + elapsed,
        );
        assert!(
            effects.is_empty(),
            "panel open {elapsed}ms after a successful check must not re-inspect"
        );
        assert_eq!(
            state.current_session_status, fresh,
            "the last known status must stay readable"
        );
    }

    // Once the cached status is stale, an open refreshes again.
    let effects = request(
        &mut state,
        200,
        SessionStatusRefreshTrigger::Open,
        10 * HOUR_MS + 24 * HOUR_MS,
    );
    assert_eq!(
        effects,
        vec![AppEffect::RefreshCurrentSessionStatus {
            request_id: 200,
            trigger: SessionStatusRefreshTrigger::Open,
        }]
    );
}

#[test]
fn manual_refresh_bypasses_the_freshness_gate() {
    let mut state = ready_state();
    state.current_session_status = ready_at(10 * HOUR_MS);

    let effects = request(
        &mut state,
        300,
        SessionStatusRefreshTrigger::Manual,
        10 * HOUR_MS + 1,
    );

    assert_eq!(
        effects,
        vec![AppEffect::RefreshCurrentSessionStatus {
            request_id: 300,
            trigger: SessionStatusRefreshTrigger::Manual,
        }],
        "an explicit manual refresh is always an immediate check"
    );
}

#[test]
fn an_unchecked_session_always_refreshes() {
    // App/session start and account switch both leave the status Idle.
    let mut state = ready_state();
    assert_eq!(
        state.current_session_status,
        CurrentSessionStatusState::Idle
    );

    let effects = request(&mut state, 400, SessionStatusRefreshTrigger::Open, HOUR_MS);

    assert_eq!(
        effects,
        vec![AppEffect::RefreshCurrentSessionStatus {
            request_id: 400,
            trigger: SessionStatusRefreshTrigger::Open,
        }]
    );
}

#[test]
fn repeated_failures_back_off_before_the_next_automatic_check() {
    let mut state = ready_state();
    let mut now = 10 * HOUR_MS;

    // First failure: a short cooldown.
    state.current_session_status = CurrentSessionStatusState::Failed {
        request_id: 1,
        kind: CurrentSessionStatusFailureKind::Network,
        checked_at_ms: now,
        last_known_details: None,
        consecutive_failures: 1,
    };
    assert!(
        request(&mut state, 500, SessionStatusRefreshTrigger::Open, now + 1).is_empty(),
        "an open immediately after a failure must not retry"
    );
    now += 2 * MINUTE_MS;
    assert!(
        !request(&mut state, 501, SessionStatusRefreshTrigger::Open, now).is_empty(),
        "the first cooldown must expire within a couple of minutes"
    );

    // Fourth consecutive failure: a longer cooldown than the first.
    let failed_at = now;
    state.current_session_status = CurrentSessionStatusState::Failed {
        request_id: 2,
        kind: CurrentSessionStatusFailureKind::Network,
        checked_at_ms: failed_at,
        last_known_details: None,
        consecutive_failures: 4,
    };
    assert!(
        request(
            &mut state,
            502,
            SessionStatusRefreshTrigger::Open,
            failed_at + 2 * MINUTE_MS,
        )
        .is_empty(),
        "repeated failures must back off further than the first"
    );
    assert!(
        !request(
            &mut state,
            503,
            SessionStatusRefreshTrigger::Manual,
            failed_at + 2 * MINUTE_MS,
        )
        .is_empty(),
        "manual refresh still bypasses the backoff"
    );
}

#[test]
fn consecutive_failures_accumulate_and_reset_on_success() {
    let mut state = ready_state();
    for expected in 1..=3u32 {
        state.current_session_status = CurrentSessionStatusState::Checking {
            request_id: expected as u64,
            trigger: SessionStatusRefreshTrigger::Manual,
            last_known_details: None,
            consecutive_failures: expected - 1,
        };
        reduce(
            &mut state,
            AppAction::CurrentSessionStatusRefreshFailed {
                request_id: expected as u64,
                kind: CurrentSessionStatusFailureKind::Network,
                checked_at_ms: 1_000 * u64::from(expected),
            },
        );
        let CurrentSessionStatusState::Failed {
            consecutive_failures,
            ..
        } = state.current_session_status
        else {
            panic!("expected a failed status");
        };
        assert_eq!(consecutive_failures, expected);
    }

    state.current_session_status = CurrentSessionStatusState::Checking {
        request_id: 9,
        trigger: SessionStatusRefreshTrigger::Manual,
        last_known_details: None,
        consecutive_failures: 0,
    };
    reduce(
        &mut state,
        AppAction::CurrentSessionStatusRefreshed {
            request_id: 9,
            details: details(true, OwnIdentityVerification::Verified),
        },
    );
    state.current_session_status = CurrentSessionStatusState::Checking {
        request_id: 10,
        trigger: SessionStatusRefreshTrigger::Manual,
        last_known_details: None,
        consecutive_failures: 0,
    };
    reduce(
        &mut state,
        AppAction::CurrentSessionStatusRefreshFailed {
            request_id: 10,
            kind: CurrentSessionStatusFailureKind::Network,
            checked_at_ms: 9_000,
        },
    );
    let CurrentSessionStatusState::Failed {
        consecutive_failures,
        ..
    } = state.current_session_status
    else {
        panic!("expected a failed status");
    };
    assert_eq!(
        consecutive_failures, 1,
        "a successful check must reset the backoff"
    );
}

/// #982/#1009: a flapping connection must not re-issue a full inspection per
/// reconnect, and repeated failures must not stop automatic checks forever:
/// reconnects issue nothing by themselves, and the timer retries at the
/// (capped) backoff.
#[test]
fn repeated_reconnects_do_not_re_inspect_and_the_backoff_keeps_retrying() {
    let mut state = ready_state();
    state.current_session_status = CurrentSessionStatusState::Failed {
        request_id: 1,
        kind: CurrentSessionStatusFailureKind::Network,
        checked_at_ms: 0,
        last_known_details: Some(details(true, OwnIdentityVerification::Verified)),
        consecutive_failures: 3,
    };

    for generation in 1..=8u64 {
        state.sync = SyncState::Reconnecting {
            reason: "transport".to_owned(),
        };
        let effects = reduce(
            &mut state,
            AppAction::SyncStatusChanged {
                generation,
                status: SyncLifecycleStatus::Running,
            },
        );
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, AppEffect::RefreshCurrentSessionStatus { .. })),
            "reconnect {generation} must not issue an inspection"
        );
        let token = state.current_session_status_schedule.token;
        assert!(
            reduce(
                &mut state,
                AppAction::CurrentSessionStatusCheckDue {
                    token,
                    now_ms: 1_000 * generation,
                },
            )
            .iter()
            .all(|effect| matches!(effect, AppEffect::ArmCurrentSessionStatusCheck { .. })),
            "a due notification inside the backoff only re-arms"
        );
    }

    let token = state.current_session_status_schedule.token;
    let effects = reduce(
        &mut state,
        AppAction::CurrentSessionStatusCheckDue {
            token,
            now_ms: 4 * MINUTE_MS,
        },
    );
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            AppEffect::RefreshCurrentSessionStatus {
                trigger: SessionStatusRefreshTrigger::Recovery,
                ..
            }
        )),
        "time passing alone must retry after three failures"
    );
}

/// #982: with the freshness gate in place, a verification completion must still
/// be observable without a restart — the cached "unverified" status cannot
/// survive the transition to Verified for the whole freshness window.
#[test]
fn completing_verification_invalidates_a_cached_unverified_status() {
    let mut state = ready_state();
    let mut stale = details(false, OwnIdentityVerification::Unverified);
    stale.verification = CurrentDeviceTrustState::Unverified;
    stale.checked_at_ms = 10 * HOUR_MS;
    state.current_session_status = CurrentSessionStatusState::Ready {
        request_id: 1,
        details: stale,
    };

    reduce(
        &mut state,
        AppAction::AuthoritativeDeviceTrustChanged {
            generation: 1,
            transition_id: 1,
            trust: CurrentDeviceTrustState::Verified,
        },
    );

    let effects = request(
        &mut state,
        600,
        SessionStatusRefreshTrigger::Open,
        10 * HOUR_MS + MINUTE_MS,
    );
    assert_eq!(
        effects,
        vec![AppEffect::RefreshCurrentSessionStatus {
            request_id: 600,
            trigger: SessionStatusRefreshTrigger::Open,
        }],
        "a verification completion must invalidate the cached status"
    );
}

#[test]
fn a_repeated_verified_trust_signal_does_not_invalidate_a_matching_cached_status() {
    let mut state = ready_state();
    let fresh = ready_at(10 * HOUR_MS);
    state.current_session_status = fresh.clone();

    reduce(
        &mut state,
        AppAction::AuthoritativeDeviceTrustChanged {
            generation: 1,
            transition_id: 1,
            trust: CurrentDeviceTrustState::Verified,
        },
    );

    assert_eq!(
        state.current_session_status, fresh,
        "a redundant Verified signal must not discard a matching fresh status"
    );
    assert!(
        request(
            &mut state,
            601,
            SessionStatusRefreshTrigger::Open,
            10 * HOUR_MS + MINUTE_MS,
        )
        .is_empty(),
        "and must not re-open the inspection path"
    );
}
