use koushi_state::{
    AppAction, AppState, SessionAuthenticationMethod, SessionInfo, SessionState, reduce,
};

fn destination(value: &str) -> Option<koushi_state::AccountManagementUrl> {
    Some(koushi_state::AccountManagementUrl::from_validated(
        value.to_owned(),
    ))
}

fn session(homeserver: &str, device_id: &str) -> SessionInfo {
    SessionInfo {
        homeserver: homeserver.to_owned(),
        user_id: format!("@alice:{}", homeserver.trim_start_matches("https://")),
        device_id: device_id.to_owned(),
        authentication_method: SessionAuthenticationMethod::Password,
    }
}

#[test]
fn destination_serializes_as_a_string_and_redacts_debug() {
    let destination = koushi_state::AccountManagementUrl::from_validated(
        "https://account.example/devices?action=sessions".to_owned(),
    );

    assert_eq!(
        serde_json::to_value(&destination).expect("serialize destination"),
        serde_json::json!("https://account.example/devices?action=sessions")
    );
    let debug = format!("{destination:?}");
    assert!(!debug.contains("account.example"));
    assert!(!debug.contains("action=sessions"));
}

#[test]
fn destination_is_owned_by_the_exact_active_session_and_cleared_with_it() {
    let first = session("https://first.example", "FIRST");
    let second = session("https://second.example", "SECOND");
    let mut state = AppState {
        session: SessionState::Ready(first.clone()),
        ..AppState::default()
    };

    reduce(
        &mut state,
        AppAction::ActiveSessionAccountManagementUrlResolved {
            info: first.clone(),
            url: destination("https://account.first.example/devices"),
        },
    );
    assert_eq!(
        state.account_management_url.as_deref(),
        Some("https://account.first.example/devices")
    );
    reduce(
        &mut state,
        AppAction::ActiveSessionAccountManagementUrlResolved {
            info: first.clone(),
            url: None,
        },
    );
    assert_eq!(state.account_management_url, None);

    state.session = SessionState::Ready(second.clone());
    state.account_management_url = None;
    reduce(
        &mut state,
        AppAction::ActiveSessionAccountManagementUrlResolved {
            info: first,
            url: destination("https://stale.example/devices"),
        },
    );
    assert_eq!(state.account_management_url, None);

    reduce(
        &mut state,
        AppAction::ActiveSessionAccountManagementUrlResolved {
            info: second,
            url: destination("https://account.second.example/devices"),
        },
    );
    assert!(state.account_management_url.is_some());

    reduce(&mut state, AppAction::LogoutFinished);
    assert_eq!(state.account_management_url, None);
}

#[test]
fn authentication_lock_and_trust_quarantine_clear_the_destination() {
    let info = session("https://matrix.example", "DEVICE");
    let mut locked = AppState {
        session: SessionState::Ready(info.clone()),
        account_management_url: destination("https://account.example/devices"),
        ..AppState::default()
    };
    reduce(
        &mut locked,
        AppAction::SessionAuthenticationInvalidated { soft_logout: true },
    );
    assert_eq!(locked.account_management_url, None);

    let mut quarantined = AppState {
        session: SessionState::Ready(info),
        account_management_url: destination("https://account.example/devices"),
        ..AppState::default()
    };
    reduce(
        &mut quarantined,
        AppAction::AuthoritativeDeviceTrustChanged {
            generation: 1,
            transition_id: 1,
            trust: koushi_state::CurrentDeviceTrustState::Unknown,
        },
    );
    assert_eq!(quarantined.account_management_url, None);
}
