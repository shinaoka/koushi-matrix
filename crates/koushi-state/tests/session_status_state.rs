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
        },
    );

    assert_eq!(
        state.current_session_status,
        CurrentSessionStatusState::Checking {
            request_id: 7,
            trigger: SessionStatusRefreshTrigger::Open,
            last_known_details: None,
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
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::CurrentSessionStatusRefreshRequested {
            request_id: 8,
            trigger: SessionStatusRefreshTrigger::Manual,
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
fn connectivity_recovery_refreshes_once_and_coalesces_a_later_manual_retry() {
    let mut state = ready_state();
    state.sync = SyncState::Reconnecting {
        reason: "transport".to_owned(),
    };
    state.current_session_status = CurrentSessionStatusState::Failed {
        request_id: 40,
        kind: CurrentSessionStatusFailureKind::Network,
        checked_at_ms: 2_000,
        last_known_details: Some(details(true, OwnIdentityVerification::Verified)),
    };

    let effects = reduce(
        &mut state,
        AppAction::SyncStatusChanged {
            generation: 41,
            status: SyncLifecycleStatus::Running,
        },
    );

    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(koushi_state::UiEvent::RoomListChanged),
            AppEffect::SyncConnectivityChanged { proven: true },
            AppEffect::RefreshCurrentSessionStatus {
                request_id: 41,
                trigger: SessionStatusRefreshTrigger::Recovery,
            },
        ]
    );
    assert!(matches!(
        state.current_session_status,
        CurrentSessionStatusState::Checking {
            request_id: 41,
            trigger: SessionStatusRefreshTrigger::Recovery,
            last_known_details: Some(_),
        }
    ));

    let duplicate_effects = reduce(
        &mut state,
        AppAction::CurrentSessionStatusRefreshRequested {
            request_id: 42,
            trigger: SessionStatusRefreshTrigger::Manual,
        },
    );
    assert!(duplicate_effects.is_empty());
    assert!(matches!(
        state.current_session_status,
        CurrentSessionStatusState::Checking { request_id: 41, .. }
    ));

    reduce(
        &mut state,
        AppAction::CurrentSessionStatusRefreshed {
            request_id: 41,
            details: details(true, OwnIdentityVerification::Verified),
        },
    );
    assert!(matches!(
        state.current_session_status,
        CurrentSessionStatusState::Ready { request_id: 41, .. }
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
