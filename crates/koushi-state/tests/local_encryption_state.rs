use koushi_state::{
    AppAction, AppEffect, AppState, LocalEncryptionHealth, LocalEncryptionState, SessionInfo,
    SessionState, UiEvent, reduce,
};

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.org".to_owned(),
        user_id: "@user-a:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
    }
}

#[test]
fn local_encryption_defaults_to_unknown_private_state() {
    let state = AppState::default();

    assert_eq!(state.local_encryption, LocalEncryptionState::Unknown);
    assert_eq!(state.local_encryption.kind(), "unknown");
}

#[test]
fn local_encryption_probe_is_request_correlated_and_kind_only() {
    let mut state = AppState::default();

    let effects = reduce(
        &mut state,
        AppAction::LocalEncryptionProbeRequested { request_id: 10 },
    );

    assert_eq!(
        state.local_encryption,
        LocalEncryptionState::Probing { request_id: 10 }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::LocalEncryptionChanged)]
    );

    assert_eq!(
        reduce(
            &mut state,
            AppAction::LocalEncryptionHealthChanged {
                request_id: 9,
                health: LocalEncryptionHealth::Healthy,
            },
        ),
        Vec::new()
    );
    assert_eq!(
        state.local_encryption,
        LocalEncryptionState::Probing { request_id: 10 }
    );

    let effects = reduce(
        &mut state,
        AppAction::LocalEncryptionHealthChanged {
            request_id: 10,
            health: LocalEncryptionHealth::LockedOrInaccessible,
        },
    );

    assert_eq!(
        state.local_encryption,
        LocalEncryptionState::LockedOrInaccessible
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::LocalEncryptionChanged)]
    );
}

#[test]
fn local_encryption_fail_closed_states_are_explicit() {
    for (health, expected) in [
        (
            LocalEncryptionHealth::Unavailable,
            LocalEncryptionState::Unavailable,
        ),
        (
            LocalEncryptionHealth::MissingCredential,
            LocalEncryptionState::MissingCredential,
        ),
        (
            LocalEncryptionHealth::ResetRequired,
            LocalEncryptionState::ResetRequired,
        ),
    ] {
        let mut state = AppState {
            local_encryption: LocalEncryptionState::Probing { request_id: 42 },
            ..AppState::default()
        };

        let effects = reduce(
            &mut state,
            AppAction::LocalEncryptionHealthChanged {
                request_id: 42,
                health,
            },
        );

        assert_eq!(state.local_encryption, expected);
        assert_eq!(
            effects,
            vec![AppEffect::EmitUiEvent(UiEvent::LocalEncryptionChanged)]
        );
    }
}

#[test]
fn local_data_reset_is_guarded_and_request_correlated() {
    let mut state = AppState {
        local_encryption: LocalEncryptionState::MissingCredential,
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::ResetLocalDataRequested { request_id: 77 },
    );

    assert_eq!(
        state.local_encryption,
        LocalEncryptionState::Resetting { request_id: 77 }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::LocalEncryptionChanged)]
    );

    assert_eq!(
        reduce(
            &mut state,
            AppAction::ResetLocalDataFailed { request_id: 76 },
        ),
        Vec::new()
    );
    assert_eq!(
        state.local_encryption,
        LocalEncryptionState::Resetting { request_id: 77 }
    );

    reduce(
        &mut state,
        AppAction::ResetLocalDataFailed { request_id: 77 },
    );
    assert_eq!(state.local_encryption, LocalEncryptionState::ResetRequired);

    reduce(
        &mut state,
        AppAction::ResetLocalDataRequested { request_id: 78 },
    );
    reduce(
        &mut state,
        AppAction::ResetLocalDataCompleted { request_id: 78 },
    );
    assert_eq!(state.local_encryption, LocalEncryptionState::Unknown);
}

#[test]
fn logout_lock_and_account_switch_clear_local_encryption_health() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        local_encryption: LocalEncryptionState::Healthy,
        ..AppState::default()
    };

    reduce(&mut state, AppAction::SessionLocked);
    assert_eq!(state.local_encryption, LocalEncryptionState::Unknown);

    state.session = SessionState::Ready(session_info());
    state.local_encryption = LocalEncryptionState::LockedOrInaccessible;
    reduce(&mut state, AppAction::LogoutRequested);
    assert_eq!(state.local_encryption, LocalEncryptionState::Unknown);
}
