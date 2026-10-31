//! #1505: own-session check policy and its reducer projection.

use koushi_state::{
    AppAction, AppState, DeviceTrustLevel, DeviceTrustSummary, OWN_SESSION_CHECK_INTERVAL_MS,
    SessionInfo, SessionState, UnverifiedOwnSessionsAlert, own_session_check_due_at_ms, reduce,
    unverified_own_sessions_alert,
};

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@user:example.invalid".to_owned(),
            device_id: "CURRENT".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        ..AppState::default()
    }
}

fn device(device_id: &str, trust_level: DeviceTrustLevel, is_current: bool) -> DeviceTrustSummary {
    DeviceTrustSummary {
        user_id: "@user:example.invalid".to_owned(),
        device_id: device_id.to_owned(),
        trust_level,
        display_name: Some(format!("Session {device_id}")),
        last_seen_ts: Some(1_000),
        is_current,
    }
}

fn mixed_devices() -> Vec<DeviceTrustSummary> {
    vec![
        device("CURRENT", DeviceTrustLevel::Verified, true),
        device("PHONE", DeviceTrustLevel::Verified, false),
        device("OLD", DeviceTrustLevel::Unverified, false),
        device("UNKNOWN", DeviceTrustLevel::Unknown, false),
    ]
}

#[test]
fn a_never_checked_account_is_due_at_once() {
    assert_eq!(own_session_check_due_at_ms(None), 0);
}

#[test]
fn a_checked_account_is_due_only_after_the_interval() {
    let last = 1_700_000_000_000;
    let due = own_session_check_due_at_ms(Some(last));
    assert_eq!(due, last + OWN_SESSION_CHECK_INTERVAL_MS);
    assert_eq!(OWN_SESSION_CHECK_INTERVAL_MS, 24 * 60 * 60 * 1_000);
    assert!(last + OWN_SESSION_CHECK_INTERVAL_MS - 1 < due);
}

#[test]
fn counts_include_every_session_but_never_count_the_current_one_as_unverified() {
    let mut devices = mixed_devices();
    // Even an unverified-looking current session is not an "other" session.
    devices[0].trust_level = DeviceTrustLevel::Unverified;
    assert_eq!(
        unverified_own_sessions_alert(&devices, true),
        Some(UnverifiedOwnSessionsAlert {
            total_sessions: 4,
            unverified_sessions: 2,
        })
    );
}

#[test]
fn no_alert_without_a_verified_current_session_or_without_unverified_others() {
    assert_eq!(unverified_own_sessions_alert(&mixed_devices(), false), None);
    let all_verified = vec![
        device("CURRENT", DeviceTrustLevel::Verified, true),
        device("PHONE", DeviceTrustLevel::Verified, false),
    ];
    assert_eq!(unverified_own_sessions_alert(&all_verified, true), None);
}

#[test]
fn a_check_installs_the_inventory_and_alert_and_dismissal_clears_only_the_alert() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::OwnSessionsChecked {
            devices: mixed_devices(),
            current_session_verified: true,
        },
    );
    assert_eq!(state.e2ee_trust.devices, mixed_devices());
    assert_eq!(
        state.e2ee_trust.unverified_own_sessions_alert,
        Some(UnverifiedOwnSessionsAlert {
            total_sessions: 4,
            unverified_sessions: 2,
        })
    );

    reduce(&mut state, AppAction::UnverifiedOwnSessionsAlertDismissed);
    assert_eq!(state.e2ee_trust.unverified_own_sessions_alert, None);
    assert_eq!(state.e2ee_trust.devices, mixed_devices());
}

#[test]
fn a_later_check_without_unverified_sessions_clears_a_pending_alert() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::OwnSessionsChecked {
            devices: mixed_devices(),
            current_session_verified: true,
        },
    );
    let verified = vec![device("CURRENT", DeviceTrustLevel::Verified, true)];
    reduce(
        &mut state,
        AppAction::OwnSessionsChecked {
            devices: verified.clone(),
            current_session_verified: true,
        },
    );
    assert_eq!(state.e2ee_trust.devices, verified);
    assert_eq!(state.e2ee_trust.unverified_own_sessions_alert, None);
}

#[test]
fn a_check_settling_after_the_session_left_ready_is_inert() {
    let mut state = AppState::default();
    let effects = reduce(
        &mut state,
        AppAction::OwnSessionsChecked {
            devices: mixed_devices(),
            current_session_verified: true,
        },
    );
    assert!(effects.is_empty());
    assert!(state.e2ee_trust.devices.is_empty());
    assert_eq!(state.e2ee_trust.unverified_own_sessions_alert, None);
}
