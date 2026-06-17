use matrix_desktop_state::{
    AccountManagementCapabilities, AccountManagementOperation, AccountManagementState, AppAction,
    AppState, AuthFailureKind, CapabilityState, SessionInfo, SessionState, reduce,
};

fn ready_session() -> SessionState {
    SessionState::Ready(SessionInfo {
        homeserver: "https://example.invalid".to_owned(),
        user_id: "@user:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
    })
}

#[test]
fn default_capabilities_are_unknown() {
    let state = AppState::default();

    assert_eq!(
        state.account_management_capabilities,
        AccountManagementCapabilities {
            change_password: CapabilityState::Unknown,
        }
    );
}

#[test]
fn capabilities_load_marks_change_password_enabled() {
    let mut state = AppState::default();

    let effects = reduce(
        &mut state,
        AppAction::AccountManagementCapabilitiesLoaded {
            change_password: true,
        },
    );

    assert_eq!(
        state.account_management_capabilities.change_password,
        CapabilityState::Enabled
    );
    assert!(effects.iter().any(|e| matches!(
        e,
        matrix_desktop_state::AppEffect::EmitUiEvent(
            matrix_desktop_state::UiEvent::AccountManagementCapabilitiesChanged
        )
    )));
}

#[test]
fn capabilities_load_marks_change_password_disabled() {
    let mut state = AppState::default();

    reduce(
        &mut state,
        AppAction::AccountManagementCapabilitiesLoaded {
            change_password: false,
        },
    );

    assert_eq!(
        state.account_management_capabilities.change_password,
        CapabilityState::Disabled
    );
}

#[test]
fn capabilities_load_failure_resets_to_unknown() {
    let mut state = AppState::default();
    state.account_management_capabilities.change_password = CapabilityState::Enabled;

    reduce(
        &mut state,
        AppAction::AccountManagementCapabilitiesLoadFailed,
    );

    assert_eq!(
        state.account_management_capabilities.change_password,
        CapabilityState::Unknown
    );
}

#[test]
fn password_change_request_enters_working_state() {
    let mut state = AppState::default();
    state.session = ready_session();

    let effects = reduce(
        &mut state,
        AppAction::AccountManagementRequested {
            request_id: 1,
            operation: AccountManagementOperation::ChangePassword,
        },
    );

    assert_eq!(
        state.account_management,
        AccountManagementState::Working {
            request_id: 1,
            operation: AccountManagementOperation::ChangePassword,
        }
    );
    assert!(effects.iter().any(|e| matches!(
        e,
        matrix_desktop_state::AppEffect::EmitUiEvent(
            matrix_desktop_state::UiEvent::AccountManagementChanged
        )
    )));
}

#[test]
fn deactivation_request_enters_working_state() {
    let mut state = AppState::default();
    state.session = ready_session();

    reduce(
        &mut state,
        AppAction::AccountManagementRequested {
            request_id: 2,
            operation: AccountManagementOperation::DeactivateAccount,
        },
    );

    assert_eq!(
        state.account_management,
        AccountManagementState::Working {
            request_id: 2,
            operation: AccountManagementOperation::DeactivateAccount,
        }
    );
}

#[test]
fn uia_challenge_transitions_to_awaiting_uia() {
    let mut state = AppState::default();
    state.session = ready_session();
    state.account_management = AccountManagementState::Working {
        request_id: 3,
        operation: AccountManagementOperation::ChangePassword,
    };

    reduce(
        &mut state,
        AppAction::AccountManagementUiaRequired {
            request_id: 3,
            flow_id: 30,
            operation: AccountManagementOperation::ChangePassword,
        },
    );

    assert_eq!(
        state.account_management,
        AccountManagementState::AwaitingUia {
            request_id: 3,
            flow_id: 30,
            operation: AccountManagementOperation::ChangePassword,
        }
    );
}

#[test]
fn password_change_success_settles_state() {
    let mut state = AppState::default();
    state.session = ready_session();
    state.account_management = AccountManagementState::Working {
        request_id: 4,
        operation: AccountManagementOperation::ChangePassword,
    };

    reduce(
        &mut state,
        AppAction::AccountManagementSucceeded {
            request_id: 4,
            operation: AccountManagementOperation::ChangePassword,
        },
    );

    assert_eq!(
        state.account_management,
        AccountManagementState::Succeeded {
            request_id: 4,
            operation: AccountManagementOperation::ChangePassword,
        }
    );
}

#[test]
fn password_change_failure_settles_state() {
    let mut state = AppState::default();
    state.session = ready_session();
    state.account_management = AccountManagementState::Working {
        request_id: 5,
        operation: AccountManagementOperation::ChangePassword,
    };

    reduce(
        &mut state,
        AppAction::AccountManagementFailed {
            request_id: 5,
            operation: AccountManagementOperation::ChangePassword,
            kind: AuthFailureKind::Sdk,
        },
    );

    assert!(matches!(
        state.account_management,
        AccountManagementState::Failed {
            request_id: 5,
            operation: AccountManagementOperation::ChangePassword,
            kind: AuthFailureKind::Sdk,
        }
    ));
}

#[test]
fn logout_clears_capabilities() {
    let mut state = AppState::default();
    state.session = ready_session();
    state.account_management_capabilities.change_password = CapabilityState::Enabled;

    reduce(&mut state, AppAction::LogoutRequested);

    assert_eq!(
        state.account_management_capabilities.change_password,
        CapabilityState::Unknown
    );
}
