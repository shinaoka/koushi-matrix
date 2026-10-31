use koushi_state::{
    AppAction, AppEffect, AppState, ProvisionalPhase, SessionAuthenticationMethod, SessionInfo,
    SessionState, VerificationAccountKind, VerificationGateState, VerificationMethod,
    VerificationMethodCapability, reduce,
};

fn info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://example.invalid".to_owned(),
        user_id: "@bootstrap:example.invalid".to_owned(),
        device_id: "SYNTHETIC".to_owned(),
        authentication_method: SessionAuthenticationMethod::Unknown,
    }
}

fn bootstrap() -> VerificationGateState {
    VerificationGateState {
        account_kind: VerificationAccountKind::NewIdentity,
        methods: vec![VerificationMethodCapability::Bootstrap],
        failure: None,
    }
}

#[test]
fn unknown_recheck_rejects_delayed_generic_discovery_until_confirmed_missing_phase_entry() {
    let mut state = AppState {
        session: SessionState::Provisional {
            info: info(),
            phase: ProvisionalPhase::RecheckingTrust { failure: None },
        },
        ..AppState::default()
    };
    for account_kind in [
        VerificationAccountKind::NewIdentity,
        VerificationAccountKind::ExistingIdentity,
        VerificationAccountKind::Unknown,
    ] {
        let before = state.clone();
        let mut gate = bootstrap();
        gate.account_kind = account_kind;
        assert!(reduce(&mut state, AppAction::VerificationMethodsDiscovered(gate)).is_empty());
        assert_eq!(
            state, before,
            "generic discovery cannot settle Unknown rechecking"
        );
    }
    for action in [
        AppAction::VerificationMethodDiscoveryRetryStarted { generation: 2 },
        AppAction::VerificationMethodsDiscovered(bootstrap()),
    ] {
        let effects = reduce(&mut state, action);
        assert!(
            effects
                .iter()
                .all(|effect| matches!(effect, AppEffect::EmitUiEvent(_))),
            "confirmed missing enters bootstrap without another query or promotion"
        );
    }
    assert_eq!(
        state.session,
        SessionState::AwaitingVerification {
            info: info(),
            gate: bootstrap()
        }
    );
}

#[test]
fn duplicate_missing_identity_batches_preserve_bootstrap_and_key_confirmation() {
    for session in [
        SessionState::AwaitingVerification {
            info: info(),
            gate: bootstrap(),
        },
        SessionState::Verifying {
            info: info(),
            gate: bootstrap(),
            method: VerificationMethod::Bootstrap,
            flow_id: 7,
            sas_emojis: vec![],
        },
        SessionState::AwaitingBootstrapConfirmation {
            info: info(),
            gate: bootstrap(),
            flow_id: 7,
        },
        SessionState::Ready(info()),
    ] {
        let mut state = AppState {
            session,
            ..AppState::default()
        };
        let before = state.clone();
        for action in [
            AppAction::VerificationMethodDiscoveryRetryStarted { generation: 2 },
            AppAction::VerificationMethodsDiscovered(bootstrap()),
        ] {
            assert!(reduce(&mut state, action).is_empty());
        }
        assert_eq!(state, before);
    }
}
