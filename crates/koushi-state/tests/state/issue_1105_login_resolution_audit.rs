//! Same-attempt successful authentication must accept core-resolved servers.
use koushi_state::{
    AppAction, AppState, LoginAttemptId, SessionAuthenticationMethod, SessionInfo, SessionState,
    reduce,
};

fn accepts_resolved_homeserver(input: &str, resolved: &str) {
    let mut state = AppState::default();
    let attempt_id = LoginAttemptId::new(23, 1);
    reduce(
        &mut state,
        AppAction::AuthenticationStarted {
            attempt_id,
            homeserver: input.into(),
        },
    );
    reduce(
        &mut state,
        AppAction::LoginSucceeded {
            attempt_id,
            info: SessionInfo {
                homeserver: resolved.into(),
                user_id: "@member:example.invalid".into(),
                device_id: "SYNTHETIC".into(),
                authentication_method: SessionAuthenticationMethod::Password,
            },
        },
    );
    assert!(
        matches!(state.session, SessionState::Provisional { .. }),
        "same-attempt SDK success rejected after server resolution"
    );
}

#[test]
fn successful_login_accepts_well_known_delegation() {
    accepts_resolved_homeserver("https://example.invalid", "https://matrix.example.invalid");
}

#[test]
fn successful_login_accepts_normalized_bare_server_name() {
    accepts_resolved_homeserver("example.invalid", "https://example.invalid");
}

#[test]
fn successful_login_accepts_unchanged_explicit_url() {
    accepts_resolved_homeserver("https://example.invalid/", "https://example.invalid");
}
