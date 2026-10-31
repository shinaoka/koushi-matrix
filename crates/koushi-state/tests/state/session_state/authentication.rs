use super::support::{alternate_session_info, recovery_gate, session_info};
use koushi_state::{
    AppAction, AppEffect, AppState, AuthDiscoveryState, AuthFailureDetail, AuthFailureKind,
    AuthFailureStage, AuthMatrixErrorKind, AuthMethod, AuthSecret, DelegatedAuthLinks,
    LoginAttemptId, LoginFlow, LoginFlowKind, LoginRequest, ProvisionalPhase, RecoveryMethod,
    SessionState, SyncState, UiEvent, VerificationGateRejectReason, VerificationMethod, reduce,
};

fn login_attempt_id() -> LoginAttemptId {
    LoginAttemptId::new(0, 7)
}

fn invalid_auth_admission_sessions() -> Vec<SessionState> {
    let info = session_info();
    vec![
        SessionState::Restoring,
        SessionState::SwitchingAccount { info: info.clone() },
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
            flow_id: 9,
            sas_emojis: vec![],
        },
        SessionState::Rejecting {
            info: info.clone(),
            reason: VerificationGateRejectReason::UserRejected,
        },
        SessionState::Ready(info.clone()),
        SessionState::Locked(info),
        SessionState::LoggingOut,
    ]
}

#[test]
fn authenticated_install_is_provisional_for_login_and_restore() {
    for (initial, action) in [
        (
            SessionState::Restoring,
            AppAction::RestoreSessionSucceeded(session_info()),
        ),
        (
            SessionState::Authenticating {
                homeserver: "https://matrix.example.org".to_owned(),
                attempt_id: login_attempt_id(),
            },
            AppAction::LoginSucceeded {
                attempt_id: login_attempt_id(),
                info: session_info(),
            },
        ),
    ] {
        let mut state = AppState {
            session: initial,
            ..AppState::default()
        };
        let effects = reduce(&mut state, action);

        assert_eq!(
            state.session,
            SessionState::Provisional {
                info: session_info(),
                phase: ProvisionalPhase::CheckingTrust,
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
}

#[test]
fn same_homeserver_login_attempts_reject_stale_success_and_failure() {
    let attempt_a = LoginAttemptId::new(1, 41);
    let attempt_b = LoginAttemptId::new(1, 42);
    assert_eq!(format!("{attempt_a:?}"), "LoginAttemptId(..)");
    let login = |attempt_id| AppAction::LoginSubmitted {
        attempt_id,
        request: LoginRequest {
            homeserver: session_info().homeserver,
            username: "user".to_owned(),
            password: AuthSecret::new("synthetic-password"),
            device_display_name: None,
        },
    };
    let mut state = AppState::default();
    reduce(&mut state, login(attempt_a));
    reduce(&mut state, login(attempt_b));
    assert!(matches!(
        state.session,
        SessionState::Authenticating { attempt_id, .. } if attempt_id == attempt_b
    ));

    let before = state.clone();
    assert!(
        reduce(
            &mut state,
            AppAction::LoginSucceeded {
                attempt_id: attempt_a,
                info: session_info(),
            },
        )
        .is_empty()
    );
    assert_eq!(state, before);
    assert!(
        reduce(
            &mut state,
            AppAction::LoginFailed {
                attempt_id: attempt_a,
                message: "stale failure".to_owned(),
                reason: None,
            },
        )
        .is_empty()
    );
    assert_eq!(state, before);

    reduce(
        &mut state,
        AppAction::LoginSucceeded {
            attempt_id: attempt_b,
            info: session_info(),
        },
    );
    assert!(matches!(state.session, SessionState::Provisional { .. }));
}

#[test]
fn same_sequence_from_another_connection_is_a_stale_login_terminal() {
    let stale_attempt = LoginAttemptId::new(1, 7);
    let active_attempt = LoginAttemptId::new(2, 7);
    let mut state = AppState::default();
    reduce(
        &mut state,
        AppAction::AuthenticationStarted {
            attempt_id: active_attempt,
            homeserver: session_info().homeserver,
        },
    );

    let before = state.clone();
    assert!(
        reduce(
            &mut state,
            AppAction::LoginSucceeded {
                attempt_id: stale_attempt,
                info: session_info(),
            },
        )
        .is_empty()
    );
    assert_eq!(state, before);
    assert!(
        reduce(
            &mut state,
            AppAction::LoginFailed {
                attempt_id: stale_attempt,
                message: "stale failure".to_owned(),
                reason: None,
            },
        )
        .is_empty()
    );
    assert_eq!(state, before);
}

#[test]
fn authentication_start_cannot_hide_an_active_or_gated_session() {
    for session in invalid_auth_admission_sessions() {
        let mut state = AppState {
            session,
            ..AppState::default()
        };
        let before = state.clone();
        assert!(
            reduce(
                &mut state,
                AppAction::AuthenticationStarted {
                    attempt_id: LoginAttemptId::new(2, 8),
                    homeserver: "https://replacement.invalid".to_owned(),
                },
            )
            .is_empty()
        );
        assert_eq!(state, before);
    }
}

#[test]
fn login_submitted_emits_no_login_effect_in_active_or_gated_states() {
    for session in invalid_auth_admission_sessions() {
        let mut state = AppState {
            session,
            ..AppState::default()
        };
        let before = state.clone();
        let effects = reduce(
            &mut state,
            AppAction::LoginSubmitted {
                attempt_id: LoginAttemptId::new(2, 8),
                request: LoginRequest {
                    homeserver: "https://replacement.invalid".to_owned(),
                    username: "replacement".to_owned(),
                    password: AuthSecret::new("synthetic-password"),
                    device_display_name: None,
                },
            },
        );
        assert!(effects.is_empty());
        assert_eq!(state, before);
        let effects = reduce(
            &mut state,
            AppAction::VerificationMethodDiscoveryRetryStarted { generation: 7 },
        );
        assert!(effects.is_empty());
        assert_eq!(state, before);
    }
}

#[test]
fn oidc_pending_flow_homeserver_wins_over_mutated_discovery_state() {
    let attempt_id = LoginAttemptId::new(4, 12);
    let mut state = AppState {
        auth: AuthDiscoveryState::Ready {
            homeserver: "https://flow-b.invalid".to_owned(),
            flows: vec![],
            delegated: DelegatedAuthLinks::default(),
        },
        ..Default::default()
    };

    reduce(
        &mut state,
        AppAction::AuthenticationStarted {
            attempt_id,
            homeserver: "https://flow-a.invalid".to_owned(),
        },
    );
    assert!(matches!(
        &state.session,
        SessionState::Authenticating { homeserver, attempt_id: active }
            if homeserver == "https://flow-a.invalid" && *active == attempt_id
    ));

    reduce(
        &mut state,
        AppAction::LoginFailed {
            attempt_id,
            message: "login failed".to_owned(),
            reason: None,
        },
    );
    assert!(matches!(state.session, SessionState::SignedOut));
}

#[test]
fn stale_or_wrong_state_authentication_success_is_ignored() {
    let info = session_info();
    let cases = [
        (
            SessionState::SignedOut,
            AppAction::LoginSucceeded {
                attempt_id: login_attempt_id(),
                info: info.clone(),
            },
        ),
        (
            SessionState::SignedOut,
            AppAction::RestoreSessionSucceeded(info.clone()),
        ),
        (
            SessionState::Ready(info.clone()),
            AppAction::LoginSucceeded {
                attempt_id: login_attempt_id(),
                info: info.clone(),
            },
        ),
        (
            SessionState::Locked(info.clone()),
            AppAction::RestoreSessionSucceeded(info.clone()),
        ),
        (
            SessionState::Rejecting {
                info: info.clone(),
                reason: VerificationGateRejectReason::UserRejected,
            },
            AppAction::LoginSucceeded {
                attempt_id: login_attempt_id(),
                info: info.clone(),
            },
        ),
        (
            SessionState::Restoring,
            AppAction::LoginSucceeded {
                attempt_id: login_attempt_id(),
                info: info.clone(),
            },
        ),
        (
            SessionState::Authenticating {
                homeserver: info.homeserver.clone(),
                attempt_id: login_attempt_id(),
            },
            AppAction::RestoreSessionSucceeded(info.clone()),
        ),
    ];

    for (session, action) in cases {
        let mut state = AppState {
            session,
            ..AppState::default()
        };
        let before = state.clone();
        assert!(reduce(&mut state, action).is_empty());
        assert_eq!(state, before);
    }

    let mut logged_out = AppState {
        session: SessionState::Authenticating {
            homeserver: info.homeserver.clone(),
            attempt_id: login_attempt_id(),
        },
        ..AppState::default()
    };
    reduce(&mut logged_out, AppAction::LogoutRequested);
    let before = logged_out.clone();
    assert!(
        reduce(
            &mut logged_out,
            AppAction::LoginSucceeded {
                attempt_id: login_attempt_id(),
                info,
            },
        )
        .is_empty(),
        "late login success after logout must be stale"
    );
    assert_eq!(logged_out, before);
}

#[test]
fn legacy_recovery_required_only_migrates_matching_provisional_discovery() {
    let info = session_info();
    let action = AppAction::E2eeRecoveryRequired {
        info: info.clone(),
        methods: vec![RecoveryMethod::RecoveryKey],
    };
    for session in [
        SessionState::SignedOut,
        SessionState::Ready(info.clone()),
        SessionState::Provisional {
            info: info.clone(),
            phase: ProvisionalPhase::CheckingTrust,
        },
        SessionState::Provisional {
            info: alternate_session_info(),
            phase: ProvisionalPhase::DiscoveringMethods,
        },
    ] {
        let mut state = AppState {
            session,
            ..AppState::default()
        };
        let before = state.clone();
        assert!(reduce(&mut state, action.clone()).is_empty());
        assert_eq!(state, before);
    }

    let mut matching = AppState {
        session: SessionState::Provisional {
            info: info.clone(),
            phase: ProvisionalPhase::DiscoveringMethods,
        },
        ..AppState::default()
    };
    reduce(&mut matching, action);
    assert!(matches!(
        matching.session,
        SessionState::AwaitingVerification { info: current, .. } if current == info
    ));
}

#[test]
fn explicit_restore_enters_restoring_without_triggering_automatic_restore() {
    let mut state = AppState::default();
    let effects = reduce(&mut state, AppAction::RestoreSessionRequested);
    assert!(matches!(state.session, SessionState::Restoring));
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::RestoreSession))
    );

    reduce(
        &mut state,
        AppAction::RestoreSessionSucceeded(session_info()),
    );
    assert!(matches!(state.session, SessionState::Provisional { .. }));
}

#[test]
fn app_started_requests_session_restore() {
    let mut state = AppState::default();

    let effects = reduce(&mut state, AppAction::AppStarted);

    assert_eq!(state.session, SessionState::Restoring);
    assert_eq!(effects, vec![AppEffect::RestoreSession]);
}

#[test]
fn restore_success_installs_provisional_session_without_persisting_or_syncing() {
    let mut state = AppState {
        session: SessionState::Restoring,
        ..AppState::default()
    };
    let info = session_info();

    let effects = reduce(&mut state, AppAction::RestoreSessionSucceeded(info.clone()));

    assert_eq!(
        state.session,
        SessionState::Provisional {
            info,
            phase: ProvisionalPhase::CheckingTrust,
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
fn restore_not_found_enters_signed_out_without_error() {
    let mut state = AppState {
        session: SessionState::Restoring,
        ..AppState::default()
    };

    let effects = reduce(&mut state, AppAction::RestoreSessionNotFound);

    assert_eq!(state.session, SessionState::SignedOut);
    assert!(state.errors.is_empty());
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
    );
}

#[test]
fn login_discovery_requests_homeserver_flows() {
    let mut state = AppState::default();

    let effects = reduce(
        &mut state,
        AppAction::LoginDiscoveryRequested {
            homeserver: "https://matrix.example.org".to_owned(),
        },
    );

    assert_eq!(
        state.auth,
        AuthDiscoveryState::Discovering {
            homeserver: "https://matrix.example.org".to_owned()
        }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::DiscoverLogin {
                homeserver: "https://matrix.example.org".to_owned(),
            },
            AppEffect::EmitUiEvent(UiEvent::AuthChanged),
        ]
    );
}

#[test]
fn login_discovery_success_records_supported_flows() {
    let mut state = AppState {
        auth: AuthDiscoveryState::Discovering {
            homeserver: "https://matrix.example.org".to_owned(),
        },
        ..AppState::default()
    };
    let flows = vec![
        LoginFlow {
            kind: LoginFlowKind::Password,
            delegated_oidc_compatibility: false,
            display_name: None,
        },
        LoginFlow {
            kind: LoginFlowKind::Sso,
            delegated_oidc_compatibility: true,
            display_name: None,
        },
    ];

    let effects = reduce(
        &mut state,
        AppAction::LoginDiscoverySucceeded {
            homeserver: "https://matrix.example.org".to_owned(),
            flows: flows.clone(),
            delegated: DelegatedAuthLinks::default(),
        },
    );

    assert_eq!(
        state.auth,
        AuthDiscoveryState::Ready {
            homeserver: "https://matrix.example.org".to_owned(),
            flows,
            delegated: DelegatedAuthLinks::default(),
        }
    );
    assert_eq!(effects, vec![AppEffect::EmitUiEvent(UiEvent::AuthChanged)]);
}

#[test]
fn login_discovery_ignores_stale_completions_for_previous_homeserver() {
    let mut state = AppState {
        auth: AuthDiscoveryState::Discovering {
            homeserver: "https://new.example.org".to_owned(),
        },
        ..AppState::default()
    };

    let success_effects = reduce(
        &mut state,
        AppAction::LoginDiscoverySucceeded {
            homeserver: "https://old.example.org".to_owned(),
            flows: vec![LoginFlow {
                kind: LoginFlowKind::Password,
                delegated_oidc_compatibility: false,
                display_name: None,
            }],
            delegated: DelegatedAuthLinks::default(),
        },
    );

    assert!(success_effects.is_empty());
    assert_eq!(
        state.auth,
        AuthDiscoveryState::Discovering {
            homeserver: "https://new.example.org".to_owned(),
        }
    );

    let failure_effects = reduce(
        &mut state,
        AppAction::LoginDiscoveryFailed {
            homeserver: "https://old.example.org".to_owned(),
            kind: AuthFailureKind::Network,
        },
    );

    assert!(failure_effects.is_empty());
    assert_eq!(
        state.auth,
        AuthDiscoveryState::Discovering {
            homeserver: "https://new.example.org".to_owned(),
        }
    );
}

#[test]
fn login_submitted_enters_authenticating_and_emits_session_event() {
    let mut state = AppState::default();

    let effects = reduce(
        &mut state,
        AppAction::LoginSubmitted {
            attempt_id: login_attempt_id(),
            request: LoginRequest {
                homeserver: "https://matrix.example.org".to_owned(),
                username: "user-a".to_owned(),
                password: AuthSecret::new("synthetic-password"),
                device_display_name: Some("Matrix Desktop Test".to_owned()),
            },
        },
    );

    assert_eq!(
        state.session,
        SessionState::Authenticating {
            homeserver: "https://matrix.example.org".to_owned(),
            attempt_id: login_attempt_id(),
        }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::Login {
                attempt_id: login_attempt_id(),
                request: LoginRequest {
                    homeserver: "https://matrix.example.org".to_owned(),
                    username: "user-a".to_owned(),
                    password: AuthSecret::new("synthetic-password"),
                    device_display_name: Some("Matrix Desktop Test".to_owned()),
                },
            },
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
        ]
    );
}

#[test]
fn login_request_debug_redacts_password() {
    let action = AppAction::LoginSubmitted {
        attempt_id: login_attempt_id(),
        request: LoginRequest {
            homeserver: "https://matrix.example.org".to_owned(),
            username: "user-a".to_owned(),
            password: AuthSecret::new("synthetic-password"),
            device_display_name: Some("Matrix Desktop Test".to_owned()),
        },
    };

    let debug = format!("{action:?}");

    assert!(debug.contains("AuthSecret(..)"));
    assert!(!debug.contains("synthetic-password"));
}

#[test]
fn login_failure_returns_to_signed_out_and_records_error() {
    let mut state = AppState {
        session: SessionState::Authenticating {
            homeserver: session_info().homeserver,
            attempt_id: login_attempt_id(),
        },
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::LoginFailed {
            attempt_id: login_attempt_id(),
            message: "invalid password".to_owned(),
            reason: None,
        },
    );

    assert_eq!(state.session, SessionState::SignedOut);
    assert_eq!(state.errors[0].code, "login_failed");
    assert!(state.errors[0].recoverable);
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
        ]
    );
}

#[test]
fn session_persistence_failure_records_error_without_leaving_ready_session() {
    let info = session_info();
    let mut state = AppState {
        session: SessionState::Ready(info.clone()),
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::SessionPersistenceFailed {
            message: "session was not saved".to_owned(),
        },
    );

    assert_eq!(state.session, SessionState::Ready(info));
    assert_eq!(state.errors[0].code, "session_persistence_failed");
    assert!(state.errors[0].recoverable);
    assert_eq!(effects, vec![AppEffect::EmitUiEvent(UiEvent::ErrorChanged)]);
}

/// #1268: the bounded reason from the typed Rust classification is preserved on
/// the visible `login_failed` error so the UI can render classified guidance
/// instead of the generic message.
#[test]
fn login_failed_preserves_the_bounded_reason() {
    let attempt_id = login_attempt_id();
    let mut state = AppState {
        session: SessionState::Authenticating {
            homeserver: "https://flow-a.invalid".to_owned(),
            attempt_id,
        },
        ..Default::default()
    };

    reduce(
        &mut state,
        AppAction::LoginFailed {
            attempt_id,
            message: "login failed".to_owned(),
            reason: Some(AuthFailureKind::RateLimited),
        },
    );

    let error = state
        .errors
        .iter()
        .rev()
        .find(|error| error.code == "login_failed")
        .expect("login failure error");
    assert_eq!(error.reason, Some(AuthFailureKind::RateLimited));
    assert!(error.recoverable);
}

/// #1268: a failure with no classified reason omits the field entirely.
#[test]
fn login_failed_without_a_reason_omits_it() {
    let attempt_id = login_attempt_id();
    let mut state = AppState {
        session: SessionState::Authenticating {
            homeserver: "https://flow-a.invalid".to_owned(),
            attempt_id,
        },
        ..Default::default()
    };

    reduce(
        &mut state,
        AppAction::LoginFailed {
            attempt_id,
            message: "login failed".to_owned(),
            reason: None,
        },
    );

    let error = state
        .errors
        .iter()
        .rev()
        .find(|error| error.code == "login_failed")
        .expect("login failure error");
    assert_eq!(error.reason, None);
}

/// #1268: the bounded detail serializes with the same vocabulary the
/// secure-backup diagnostics work (#1265) established, so there is one bounded
/// failure shape in the product rather than a parallel one.
#[test]
fn auth_failure_detail_mirrors_the_bounded_secure_backup_shape() {
    let value = serde_json::to_value(AuthFailureDetail::http_response(
        AuthMethod::Password,
        AuthFailureStage::PasswordLogin,
        Some(403),
        Some(AuthMatrixErrorKind::Forbidden),
        false,
    ))
    .expect("detail serializes");
    assert_eq!(
        value,
        serde_json::json!({
            "method": "password",
            "stage": "passwordLogin",
            "transport": "httpResponse",
            "httpStatus": 403,
            "matrixErrorKind": "forbidden",
            "retryable": false
        })
    );

    // A failure that produced no response omits the status and kind entirely.
    let transport_only = serde_json::to_value(AuthFailureDetail::no_response(
        AuthMethod::OAuth,
        AuthFailureStage::OidcStart,
    ))
    .expect("detail serializes");
    assert_eq!(
        transport_only,
        serde_json::json!({
            "method": "oauth",
            "stage": "oidcStart",
            "transport": "noResponse",
            "retryable": true
        })
    );
}
