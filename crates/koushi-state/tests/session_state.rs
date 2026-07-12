use koushi_state::{
    AppAction, AppEffect, AppError, AppState, AuthDiscoveryState, AuthFailureKind, AuthSecret,
    BasicOperationState, ComposerSubmissionTarget, ComposerSubmissionTerminalOutcome,
    CurrentDeviceTrustState, DelegatedAuthLinks, E2eeRecoveryState, InviteOperationState,
    InviteScopeSelection, InviteTargetQueryState, InviteWorkflowState, LoginAttemptId, LoginFlow,
    LoginFlowKind, LoginRequest, NativeAttentionCandidate, NativeAttentionCapabilities,
    NativeAttentionCapability, NativeAttentionState, NativeAttentionSummary, NavigationState,
    ProvisionalPhase, RecoveryMethod, RecoveryRequest, RoomAttentionKind, RoomSummary, RoomTags,
    SearchCrawlerLastActive, SearchCrawlerLastActiveStatus, SearchCrawlerRoomState,
    SearchCrawlerState, SearchScope, SearchState, SessionInfo, SessionState, SpaceSummary,
    SubmissionId, SyncState, ThreadAttentionState, ThreadPaneState, TimelinePaneState, UiEvent,
    VerificationAccountKind, VerificationGateFailureKind, VerificationGateRejectReason,
    VerificationGateState, VerificationMethod, VerificationMethodCapability, reduce,
};

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.org".to_owned(),
        user_id: "@user-a:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
    }
}

fn alternate_session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.org".to_owned(),
        user_id: "@user-b:example.invalid".to_owned(),
        device_id: "DEVICE-B".to_owned(),
    }
}

fn login_attempt_id() -> LoginAttemptId {
    LoginAttemptId::new(0, 7)
}

fn state_with_session_scoped_workflows() -> AppState {
    AppState {
        session: SessionState::Ready(session_info()),
        sync: SyncState::Running,
        basic_operation: BasicOperationState::CreatingRoom {
            request_id: 77,
            name: "Stale room".to_owned(),
        },
        invite_workflow: InviteWorkflowState {
            query: InviteTargetQueryState {
                room_id: Some("room-a".to_owned()),
                query: "alice".to_owned(),
                candidates: Vec::new(),
                explicit_user_id: None,
            },
            operation: InviteOperationState::Pending {
                request_id: 88,
                room_id: "room-a".to_owned(),
                user_ids: vec!["@alice:example.invalid".to_owned()],
                scope: InviteScopeSelection::RoomOnly,
            },
            ..Default::default()
        },
        search_crawler: SearchCrawlerState {
            rooms: std::collections::BTreeMap::from([(
                "room-a".to_owned(),
                SearchCrawlerRoomState::Running {
                    processed: 4,
                    indexed: 3,
                },
            )]),
            last_active: Some(SearchCrawlerLastActive {
                room_id: "room-a".to_owned(),
                updated_at_ms: 1_000,
                status: SearchCrawlerLastActiveStatus::Running,
                processed: 4,
                indexed: 3,
            }),
        },
        ..AppState::default()
    }
}

fn assert_session_scoped_workflows_cleared(state: &AppState) {
    assert_eq!(state.basic_operation, BasicOperationState::Idle);
    assert_eq!(state.invite_workflow, InviteWorkflowState::default());
    assert_eq!(state.search_crawler, SearchCrawlerState::default());
}

fn recovery_gate() -> VerificationGateState {
    VerificationGateState {
        methods: vec![VerificationMethodCapability::RecoveryKey],
        account_kind: VerificationAccountKind::ExistingIdentity,
        failure: None,
    }
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
            },
        )
        .is_empty()
    );
    assert_eq!(state, before);
}

#[test]
fn authentication_start_cannot_hide_an_active_or_gated_session() {
    let info = session_info();
    let invalid_sessions = vec![
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
        },
        SessionState::Rejecting {
            info: info.clone(),
            reason: VerificationGateRejectReason::UserRejected,
        },
        SessionState::Ready(info.clone()),
        SessionState::Locked(info.clone()),
        SessionState::LoggingOut,
    ];

    for session in invalid_sessions {
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
fn oidc_pending_flow_homeserver_wins_over_mutated_discovery_state() {
    let attempt_id = LoginAttemptId::new(4, 12);
    let mut state = AppState::default();
    state.auth = AuthDiscoveryState::Ready {
        homeserver: "https://flow-b.invalid".to_owned(),
        flows: vec![],
        delegated: DelegatedAuthLinks::default(),
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
                homeserver: "https://other.example.org".to_owned(),
                attempt_id: login_attempt_id(),
            },
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
fn only_authoritative_verified_promotes_and_trust_loss_locks_and_clears() {
    let mut gated = AppState {
        session: SessionState::Verifying {
            info: session_info(),
            gate: recovery_gate(),
            method: VerificationMethod::RecoveryKey,
            flow_id: 17,
        },
        ..AppState::default()
    };
    let effects = reduce(
        &mut gated,
        AppAction::CurrentDeviceTrustChanged(CurrentDeviceTrustState::Verified),
    );
    assert_eq!(gated.session, SessionState::Ready(session_info()));
    assert!(effects.contains(&AppEffect::PersistSession(session_info())));
    assert!(effects.contains(&AppEffect::StartSync));

    let mut ready = state_with_session_scoped_workflows();
    let effects = reduce(
        &mut ready,
        AppAction::CurrentDeviceTrustChanged(CurrentDeviceTrustState::Unverified),
    );
    assert_eq!(ready.session, SessionState::Locked(session_info()));
    assert_eq!(ready.sync, SyncState::Stopped);
    assert_session_scoped_workflows_cleared(&ready);
    assert!(effects.contains(&AppEffect::StopSync));
}

#[test]
fn existing_identity_without_proof_rejects_then_discards() {
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
        AppAction::VerificationMethodsDiscovered(no_proof),
    );
    assert_eq!(
        state.session,
        SessionState::Rejecting {
            info: session_info(),
            reason: VerificationGateRejectReason::ExistingIdentityWithoutProof,
        }
    );
    assert_eq!(effects, vec![AppEffect::RejectProvisionalSession]);

    reduce(&mut state, AppAction::ProvisionalSessionDiscarded);
    assert_eq!(state.session, SessionState::SignedOut);
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

#[test]
fn account_switch_request_enters_switching_state_and_clears_views() {
    let current = session_info();
    let target = alternate_session_info();
    let mut state = AppState {
        session: SessionState::Ready(current),
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
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            marked_unread: false,
            last_activity_ms: 0,
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
        },
        thread: ThreadPaneState::Open {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            is_subscribed: true,
            composer: Default::default(),
        },
        thread_attention: ThreadAttentionState::Tracking {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            notification_count: 2,
            highlight_count: 1,
            live_event_marker_count: 2,
        },
        search: SearchState::Editing {
            query: "hello".to_owned(),
            scope: SearchScope::AllRooms,
        },
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::SwitchAccountRequested {
            info: target.clone(),
        },
    );

    assert_eq!(
        state.session,
        SessionState::SwitchingAccount {
            info: target.clone()
        }
    );
    assert_eq!(state.sync, SyncState::Stopped);
    assert_eq!(state.navigation, NavigationState::default());
    assert!(state.spaces.is_empty());
    assert!(state.rooms.is_empty());
    assert_eq!(state.timeline, TimelinePaneState::default());
    assert_eq!(state.thread, ThreadPaneState::Closed);
    assert_eq!(state.thread_attention, ThreadAttentionState::Closed);
    assert_eq!(state.search, SearchState::Closed);
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
            AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: "room-a".to_owned(),
            }),
            AppEffect::EmitUiEvent(UiEvent::ThreadChanged),
            AppEffect::EmitUiEvent(UiEvent::SearchChanged),
        ]
    );
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
        AppAction::E2eeRecoverySubmitted(RecoveryRequest {
            secret: AuthSecret::new("synthetic-recovery-secret"),
        }),
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
            flow_id: 0,
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
fn e2ee_recovery_success_requests_authoritative_trust_recheck() {
    let info = session_info();
    let mut state = AppState {
        session: SessionState::Verifying {
            info: info.clone(),
            gate: recovery_gate(),
            method: VerificationMethod::RecoveryKey,
            flow_id: 0,
        },
        ..AppState::default()
    };

    let effects = reduce(&mut state, AppAction::E2eeRecoverySucceeded);

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
            last_activity_ms: 0,
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
fn logout_stops_sync_and_clears_session() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        sync: SyncState::Running,
        ..AppState::default()
    };

    let effects = reduce(&mut state, AppAction::LogoutRequested);

    assert_eq!(state.session, SessionState::LoggingOut);
    assert_eq!(state.sync, SyncState::Stopped);
    assert_eq!(
        effects,
        vec![
            AppEffect::StopSync,
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
        ]
    );
}

#[test]
fn logout_clears_session_views_and_notifies_ui() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
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
            last_activity_ms: 0,
            latest_event: None,
            parent_space_ids: vec!["space-a".to_owned()],
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 0,
        }],
        timeline: TimelinePaneState {
            room_id: Some("room-a".to_owned()),
            is_subscribed: true,
            is_paginating_backwards: true,
            composer: Default::default(),
            submission_registry: Default::default(),
            scheduled_send_capability: Default::default(),
            scheduled_sends: Vec::new(),
            staged_uploads: Vec::new(),
            media_gallery: Vec::new(),
            media_downloads: Default::default(),
        },
        thread: ThreadPaneState::Open {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            is_subscribed: true,
            composer: Default::default(),
        },
        thread_attention: ThreadAttentionState::Tracking {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            notification_count: 2,
            highlight_count: 1,
            live_event_marker_count: 2,
        },
        search: SearchState::Editing {
            query: "アンケート".to_owned(),
            scope: SearchScope::AllRooms,
        },
        ..AppState::default()
    };

    let effects = reduce(&mut state, AppAction::LogoutRequested);

    assert_eq!(state.navigation, NavigationState::default());
    assert!(state.spaces.is_empty());
    assert!(state.rooms.is_empty());
    assert_eq!(state.timeline, TimelinePaneState::default());
    assert_eq!(state.thread, ThreadPaneState::Closed);
    assert_eq!(state.thread_attention, ThreadAttentionState::Closed);
    assert_eq!(state.search, SearchState::Closed);
    assert_eq!(
        effects,
        vec![
            AppEffect::StopSync,
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
            AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: "room-a".to_owned(),
            }),
            AppEffect::EmitUiEvent(UiEvent::ThreadChanged),
            AppEffect::EmitUiEvent(UiEvent::SearchChanged),
        ]
    );
}

#[test]
fn logout_clears_session_scoped_workflows_and_crawler_state() {
    let mut state = state_with_session_scoped_workflows();

    let effects = reduce(&mut state, AppAction::LogoutRequested);

    assert_session_scoped_workflows_cleared(&state);
    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged)));
}

#[test]
fn logout_clears_native_attention_state_and_notifies_ui() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        sync: SyncState::Running,
        native_attention: NativeAttentionState {
            summary: NativeAttentionSummary {
                unread_count: 4,
                highlight_count: 1,
                badge_count: 4,
                candidate: Some(NativeAttentionCandidate {
                    room_display_name: "Announcements".to_owned(),
                    kind: RoomAttentionKind::Mention,
                    unread_count: 4,
                    highlight_count: 1,
                }),
                capabilities: NativeAttentionCapabilities {
                    notifications: NativeAttentionCapability::Available,
                    badge: NativeAttentionCapability::Available,
                    overlay_icon: NativeAttentionCapability::Available,
                    sound: NativeAttentionCapability::Available,
                    tray: NativeAttentionCapability::Available,
                    activation: NativeAttentionCapability::Available,
                },
            },
            dispatch: Default::default(),
        },
        ..AppState::default()
    };

    let effects = reduce(&mut state, AppAction::LogoutRequested);

    assert_eq!(state.native_attention, NativeAttentionState::default());
    assert_eq!(
        effects,
        vec![
            AppEffect::StopSync,
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
            AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged),
        ]
    );
}

#[test]
fn session_locked_stops_sync_and_clears_session_views() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        sync: SyncState::Running,
        spaces: vec![SpaceSummary {
            space_id: "space-a".to_owned(),
            display_name: "Space A".to_owned(),
            avatar: None,
            child_room_ids: vec![],
        }],
        ..AppState::default()
    };

    let effects = reduce(&mut state, AppAction::SessionLocked);

    assert_eq!(state.session, SessionState::Locked(session_info()));
    assert_eq!(state.sync, SyncState::Stopped);
    assert!(state.spaces.is_empty());
    assert_eq!(
        effects,
        vec![
            AppEffect::StopSync,
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
        ]
    );
}

#[test]
fn lock_preserves_global_submission_registry_and_records_terminal() {
    let id = SubmissionId::new("locked-submission");
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        ..AppState::default()
    };
    reduce(
        &mut state,
        AppAction::ComposerSubmissionAccepted {
            submission_id: id.clone(),
            room_id: "room-a".to_owned(),
            transaction_id: "txn".to_owned(),
            body: "body".to_owned(),
        },
    );
    reduce(&mut state, AppAction::SessionLocked);
    assert!(
        state
            .timeline
            .submission_registry
            .accepted_submission_ids
            .contains(&id)
    );
    reduce(
        &mut state,
        AppAction::ComposerSubmissionSettled {
            submission_id: id.clone(),
            transaction_id: "wrong-txn".to_owned(),
            target: ComposerSubmissionTarget::Main {
                room_id: "room-a".to_owned(),
            },
            outcome: ComposerSubmissionTerminalOutcome::Succeeded,
        },
    );
    assert!(
        state
            .timeline
            .submission_registry
            .accepted_submission_ids
            .contains(&id)
    );
    reduce(
        &mut state,
        AppAction::ComposerSubmissionSettled {
            submission_id: id.clone(),
            transaction_id: "txn".to_owned(),
            target: ComposerSubmissionTarget::Main {
                room_id: "room-a".to_owned(),
            },
            outcome: ComposerSubmissionTerminalOutcome::Succeeded,
        },
    );
    assert!(
        state
            .timeline
            .submission_registry
            .settled_submission_ids
            .contains(&id)
    );
}

#[test]
fn account_replacement_clears_registry_and_ignores_unaccepted_late_terminal() {
    let old = SubmissionId::new("old-account-submission");
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        ..AppState::default()
    };
    state
        .timeline
        .submission_registry
        .accepted_submission_ids
        .push_back(old.clone());
    reduce(
        &mut state,
        AppAction::SwitchAccountRequested {
            info: alternate_session_info(),
        },
    );
    assert!(
        state
            .timeline
            .submission_registry
            .accepted_submission_ids
            .is_empty()
    );
    reduce(
        &mut state,
        AppAction::ComposerSubmissionSettled {
            submission_id: old,
            transaction_id: "txn".to_owned(),
            target: ComposerSubmissionTarget::Main {
                room_id: "old-room".to_owned(),
            },
            outcome: ComposerSubmissionTerminalOutcome::Succeeded,
        },
    );
    assert!(
        state
            .timeline
            .submission_registry
            .settled_submission_ids
            .is_empty()
    );
}

#[test]
fn session_locked_clears_session_scoped_workflows_and_crawler_state() {
    let mut state = state_with_session_scoped_workflows();

    let effects = reduce(&mut state, AppAction::SessionLocked);

    assert_session_scoped_workflows_cleared(&state);
    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged)));
}

#[test]
fn switch_account_clears_session_scoped_workflows_and_crawler_state() {
    let mut state = state_with_session_scoped_workflows();

    let effects = reduce(
        &mut state,
        AppAction::SwitchAccountRequested {
            info: alternate_session_info(),
        },
    );

    assert_session_scoped_workflows_cleared(&state);
    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged)));
}

#[test]
fn sync_failure_enters_failed_state_before_retry() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        sync: SyncState::Running,
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::SyncFailed {
            reason: "limited network".to_owned(),
        },
    );

    assert_eq!(
        state.sync,
        SyncState::Failed {
            reason: "limited network".to_owned(),
        }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
            AppEffect::StartSync,
        ]
    );
}

#[test]
fn sync_auth_failure_locks_session_and_does_not_retry() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        sync: SyncState::Running,
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::SyncFailed {
            reason: "sync_failed_auth".to_owned(),
        },
    );

    assert_eq!(
        state.sync,
        SyncState::Failed {
            reason: "sync_failed_auth".to_owned(),
        }
    );
    assert_eq!(state.session, SessionState::Locked(session_info()));
    assert!(
        state
            .errors
            .iter()
            .any(|error| error.code == "sync_auth_required" && error.recoverable)
    );
    // Auth failures must NOT emit StartSync: the refresh token is invalid and
    // retrying creates an infinite loop with HTTP 401 on every attempt.
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
        ]
    );
}

#[test]
fn sync_retry_enters_reconnecting_state() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        sync: SyncState::Failed {
            reason: "limited network".to_owned(),
        },
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::SyncReconnecting {
            reason: "limited network".to_owned(),
        },
    );

    assert_eq!(
        state.sync,
        SyncState::Reconnecting {
            reason: "limited network".to_owned(),
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
    );
}

#[test]
fn late_sync_signals_after_logout_are_ignored() {
    let mut state = AppState {
        session: SessionState::LoggingOut,
        sync: SyncState::Stopped,
        ..AppState::default()
    };

    assert_eq!(reduce(&mut state, AppAction::SyncStarted), Vec::new());
    assert_eq!(
        reduce(
            &mut state,
            AppAction::SyncFailed {
                reason: "late failure".to_owned(),
            },
        ),
        Vec::new()
    );
    assert_eq!(
        reduce(
            &mut state,
            AppAction::SyncReconnecting {
                reason: "late reconnect".to_owned(),
            },
        ),
        Vec::new()
    );
    assert_eq!(reduce(&mut state, AppAction::SyncRecovered), Vec::new());
    assert_eq!(state.sync, SyncState::Stopped);
}

#[test]
fn sync_stopped_is_a_completion_signal() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        sync: SyncState::Running,
        ..AppState::default()
    };

    let effects = reduce(&mut state, AppAction::SyncStopped);

    assert_eq!(state.sync, SyncState::Stopped);
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
    );
}
