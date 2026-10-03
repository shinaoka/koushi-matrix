use super::*;
use koushi_state::{
    NotificationSettings, SessionAuthenticationMethod, SessionInfo, SettingsValues,
};

fn ready_state() -> AppState {
    let mut state = AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://matrix.example.invalid".to_owned(),
            user_id: "@attention:example.invalid".to_owned(),
            device_id: "ATTENTION".to_owned(),
            authentication_method: SessionAuthenticationMethod::Unknown,
        }),
        ..Default::default()
    };
    state.settings.values = SettingsValues::default();
    state
}

fn payload() -> NativeNotificationPayload {
    NativeNotificationPayload {
        title: "Mention in Room".to_owned(),
        body: "Alice: private body".to_owned(),
        target: NativeNotificationTarget {
            room_id: "!room:example.invalid".to_owned(),
            event_id: Some("$event:example.invalid".to_owned()),
            thread_root_event_id: Some("$root:example.invalid".to_owned()),
        },
    }
}

#[test]
fn pending_notification_requires_a_ready_session() {
    let mut state = ready_state();
    state.native_attention.notification = Some(payload());
    state.session = SessionState::SignedOut;

    assert_eq!(
        pending_notification(&state).err(),
        Some("session_unavailable")
    );
}

#[test]
fn pending_notification_skips_without_a_payload_or_with_messages_off() {
    let mut state = ready_state();

    assert_eq!(pending_notification(&state).err(), Some("no_candidate"));

    state.native_attention.notification = Some(payload());
    state.settings.values = SettingsValues {
        notifications: NotificationSettings {
            desktop_notifications: false,
            ..NotificationSettings::default()
        },
        ..SettingsValues::default()
    };
    assert_eq!(
        pending_notification(&state).err(),
        Some("desktop_notifications_off")
    );
}

#[test]
fn pending_notification_fences_the_ready_account() {
    let mut state = ready_state();
    state.native_attention.notification = Some(payload());

    let pending = pending_notification(&state).expect("eligible banner");
    assert_eq!(pending.fence.0.0, "@attention:example.invalid");
    assert_eq!(pending.payload.body, "Alice: private body");
}

#[test]
fn activation_waiter_budget_is_bounded_and_released() {
    let mut held = Vec::new();
    for _ in 0..MAX_PENDING_ACTIVATION_WAITERS {
        held.push(ActivationWaiterReservation::acquire().expect("waiter slot"));
    }
    assert!(ActivationWaiterReservation::acquire().is_none());
    assert_eq!(
        PENDING_ACTIVATION_WAITERS.load(Ordering::Relaxed),
        MAX_PENDING_ACTIVATION_WAITERS
    );

    held.pop();
    let reacquired = ActivationWaiterReservation::acquire().expect("released slot");
    drop(reacquired);
    drop(held);
    assert_eq!(PENDING_ACTIVATION_WAITERS.load(Ordering::Relaxed), 0);
}

#[test]
fn activation_event_carries_the_target_without_preview_text() {
    let activation = NativeNotificationActivation::from(payload().target);
    assert_eq!(
        serde_json::to_value(&activation).expect("activation serializes"),
        serde_json::json!({
            "room_id": "!room:example.invalid",
            "event_id": "$event:example.invalid",
            "thread_root_event_id": "$root:example.invalid",
        })
    );
}

#[test]
fn outcome_tokens_stay_stable_for_diagnostics() {
    assert_eq!(NativeNotificationOutcome::Delivered.token(), "delivered");
    assert_eq!(
        serde_json::to_value(NativeNotificationOutcome::DisplayOnly).expect("outcome serializes"),
        serde_json::json!("displayOnly")
    );
}
