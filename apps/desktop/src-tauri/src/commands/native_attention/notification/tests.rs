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

fn account_tab_id() -> AccountTabId {
    AccountTabId::from_string("account:@attention:example.invalid".to_owned())
}

fn payload() -> NativeNotificationPayload {
    NativeNotificationPayload {
        title: "Mention in Room".to_owned(),
        body: "Alice: private body".to_owned(),
        sender_avatar: Some("avatar/0123456789abcdef".to_owned()),
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
        pending_notification(&state, account_tab_id()).err(),
        Some("session_unavailable")
    );
}

#[test]
fn pending_notification_skips_without_a_payload_or_with_messages_off() {
    let mut state = ready_state();

    assert_eq!(
        pending_notification(&state, account_tab_id()).err(),
        Some("no_candidate")
    );

    state.native_attention.notification = Some(payload());
    state.settings.values = SettingsValues {
        notifications: NotificationSettings {
            desktop_notifications: false,
            ..NotificationSettings::default()
        },
        ..SettingsValues::default()
    };
    assert_eq!(
        pending_notification(&state, account_tab_id()).err(),
        Some("desktop_notifications_off")
    );
}

#[test]
fn pending_notification_refuses_an_agent_account() {
    let mut state = ready_state();
    state.native_attention.notification = Some(payload());
    let user_id = match &state.session {
        SessionState::Ready(info) => info.user_id.clone(),
        _ => unreachable!(),
    };
    state
        .settings
        .values
        .agent_accounts
        .apply(&koushi_state::AgentAccountPatch {
            user_id,
            role: Some(koushi_state::AgentAccountRole::default()),
        });
    assert_eq!(
        pending_notification(&state, account_tab_id()).err(),
        Some("agent_account")
    );
}

#[test]
fn pending_notification_fences_the_ready_account() {
    let mut state = ready_state();
    state.native_attention.notification = Some(payload());

    let pending = pending_notification(&state, account_tab_id()).expect("eligible banner");
    assert_eq!(pending.fence.account_key.0, "@attention:example.invalid");
    assert_eq!(pending.fence.account_tab_id, account_tab_id());
    assert_eq!(pending.payload.body, "Alice: private body");
}

#[test]
fn pending_notification_captures_preview_policy_with_the_source_payload() {
    let mut state = ready_state();
    state.settings.values.notifications.message_previews = true;
    state.native_attention.notification = Some(payload());
    let pending = pending_notification(&state, account_tab_id()).expect("source notification");
    state.settings.values.notifications.message_previews = false;
    assert!(pending.message_previews);
    assert!(!state.settings.values.notifications.message_previews);
}

#[test]
fn dispatch_diagnostics_name_the_click_waiter() {
    let _guard = koushi_diagnostics::test_support::lock();
    record_notification_outcome(NativeNotificationOutcome::Delivered, "candidate");
    record_notification_outcome(NativeNotificationOutcome::DisplayOnly, "candidate");
    record_notification_outcome(NativeNotificationOutcome::Skipped, "no_candidate");
    let snapshot = koushi_diagnostics::test_support::detail_snapshot();
    let click_waiters: Vec<&str> = snapshot
        .records
        .iter()
        .filter(|record| {
            record.event.source == "desktop.native_notification"
                && record.event.stage == "dispatch_settled"
        })
        .filter_map(|record| {
            record
                .event
                .fields
                .iter()
                .find_map(|field| match (&field.key, &field.value) {
                    (&"click_waiter", koushi_diagnostics::DiagnosticValue::Token(token)) => {
                        Some(*token)
                    }
                    _ => None,
                })
        })
        .collect();
    assert!(
        click_waiters.contains(&"configured"),
        "a delivered banner must report a configured click waiter: {click_waiters:?}"
    );
    assert!(
        click_waiters.contains(&"unattached"),
        "a display-only or skipped dispatch must report no click waiter: {click_waiters:?}"
    );
    assert_eq!(
        click_waiters
            .iter()
            .filter(|token| **token == "configured")
            .count(),
        1,
        "only a delivered banner configures a click waiter: {click_waiters:?}"
    );
}

#[cfg(not(target_os = "macos"))]
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

#[tokio::test]
async fn notification_activation_selects_owning_account_tab() {
    let data_dir = tempfile::tempdir().expect("data directory");
    let factory: std::sync::Arc<
        dyn Fn() -> std::sync::Arc<dyn koushi_core::NativeArtifactPort> + Send + Sync,
    > = std::sync::Arc::new(|| std::sync::Arc::new(koushi_core::NativeArtifactRegistry::new()));
    let runtime = AccountRuntimeManager::new(
        koushi_core::store::StoreActor::with_backend(
            koushi_core::store::TestCredentialStoreBackend::in_memory(),
            data_dir.path(),
        ),
        koushi_core::settings::SettingsStore::new(data_dir.path()),
        factory,
    );
    let session = |user_id: &str, device_id: &str| SessionInfo {
        homeserver: "https://example.invalid".to_owned(),
        user_id: user_id.to_owned(),
        device_id: device_id.to_owned(),
        authentication_method: SessionAuthenticationMethod::Unknown,
    };
    let first = runtime.selected_tab_id();
    runtime
        .bind_authenticated_session(&first, &session("@alice:example.invalid", "ALICE"))
        .await
        .expect("bind first account");
    let second = runtime.add_account_tab().await.expect("add account tab");
    runtime
        .bind_authenticated_session(&second, &session("@bob:example.invalid", "BOB"))
        .await
        .expect("bind second account");

    assert_eq!(runtime.selected_tab_id(), second);
    let focus_generation = std::sync::atomic::AtomicU64::new(0);
    assert!(
        select_activation_tab(&runtime, &focus_generation, &first, true).await,
        "notification should select its owning tab"
    );
    assert_eq!(runtime.selected_tab_id(), first);
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let alice_focused = runtime
                .tab_connection(&first)
                .expect("Alice connection")
                .snapshot()
                .native_attention_context
                .window_focused;
            let bob_focused = runtime
                .tab_connection(&second)
                .expect("Bob connection")
                .snapshot()
                .native_attention_context
                .window_focused;
            if alice_focused && !bob_focused {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("notification activation transfers focus to the owning tab");
    runtime.shutdown_all().await;
}

#[test]
fn activation_event_carries_its_account_and_target_without_preview_text() {
    let account_tab_id = AccountTabId::from_string("account:@alice:example.invalid".to_owned());
    let activation = NativeNotificationActivation::new(&account_tab_id, 7, payload().target);
    assert_eq!(
        serde_json::to_value(&activation).expect("activation serializes"),
        serde_json::json!({
            "account_tab_id": "account:@alice:example.invalid",
            "activation_id": 7,
            "room_id": "!room:example.invalid",
            "event_id": "$event:example.invalid",
            "thread_root_event_id": "$root:example.invalid",
        })
    );
}

#[test]
fn notification_authorization_serializes_the_wire_state() {
    let wire = [
        (NativeNotificationAuthorization::Authorized, "authorized"),
        (NativeNotificationAuthorization::Provisional, "provisional"),
        (NativeNotificationAuthorization::Ephemeral, "ephemeral"),
        (NativeNotificationAuthorization::Denied, "denied"),
        (
            NativeNotificationAuthorization::NotDetermined,
            "not_determined",
        ),
        (NativeNotificationAuthorization::Unsupported, "unsupported"),
        (NativeNotificationAuthorization::Unavailable, "unavailable"),
    ];
    for (state, token) in wire {
        assert_eq!(
            serde_json::to_value(state).expect("authorization serializes"),
            serde_json::json!(token)
        );
        // Diagnostics readers see the same words as the settings UI.
        assert_eq!(state.token(), token);
    }
}

#[test]
fn only_granted_authorization_allows_delivery() {
    for granted in [
        NativeNotificationAuthorization::Authorized,
        NativeNotificationAuthorization::Provisional,
        NativeNotificationAuthorization::Ephemeral,
    ] {
        assert!(granted.allows_delivery(), "{granted:?} delivers");
    }
    for blocked in [
        NativeNotificationAuthorization::Denied,
        NativeNotificationAuthorization::NotDetermined,
        NativeNotificationAuthorization::Unsupported,
        NativeNotificationAuthorization::Unavailable,
    ] {
        assert!(!blocked.allows_delivery(), "{blocked:?} must not deliver");
    }
}

#[cfg(target_os = "macos")]
#[test]
fn macos_authorization_status_maps_to_the_wire_state() {
    use mac_usernotifications::AuthorizationStatus as Status;

    for (status, expected) in [
        (
            Status::Authorized,
            NativeNotificationAuthorization::Authorized,
        ),
        (
            Status::Provisional,
            NativeNotificationAuthorization::Provisional,
        ),
        (
            Status::Ephemeral,
            NativeNotificationAuthorization::Ephemeral,
        ),
        (Status::Denied, NativeNotificationAuthorization::Denied),
        (
            Status::NotDetermined,
            NativeNotificationAuthorization::NotDetermined,
        ),
        // An OS status this build does not know cannot be treated as a grant.
        (
            Status::Unknown,
            NativeNotificationAuthorization::Unavailable,
        ),
    ] {
        assert_eq!(authorization_from_status(status), expected);
    }
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn an_unbundled_process_reports_unavailable() {
    // `cargo test` has no application bundle, so UNUserNotificationCenter has
    // nothing to authorize.
    assert_eq!(
        current_authorization().await,
        NativeNotificationAuthorization::Unavailable
    );
}

#[cfg(not(target_os = "macos"))]
#[tokio::test]
async fn a_platform_without_an_os_grant_reports_unsupported() {
    assert_eq!(
        current_authorization().await,
        NativeNotificationAuthorization::Unsupported
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn opening_the_os_notification_settings_is_unsupported_off_macos() {
    assert!(open_notification_settings().is_err());
}

#[test]
fn outcome_tokens_stay_stable_for_diagnostics() {
    assert_eq!(NativeNotificationOutcome::Delivered.token(), "delivered");
    assert_eq!(
        serde_json::to_value(NativeNotificationOutcome::DisplayOnly).expect("outcome serializes"),
        serde_json::json!("displayOnly")
    );
}

#[test]
fn the_authorization_request_asks_for_the_dock_badge() {
    // #2095: macOS shows the *Badge application icon* switch — and honours the
    // Dock badge Koushi sets — only for an app that requested the badge option.
    assert_ne!(
        REQUESTED_NOTIFICATION_OPTIONS & UNAUTHORIZATION_OPTION_BADGE,
        0
    );
    assert_ne!(
        REQUESTED_NOTIFICATION_OPTIONS & UNAUTHORIZATION_OPTION_SOUND,
        0
    );
    assert_ne!(
        REQUESTED_NOTIFICATION_OPTIONS & UNAUTHORIZATION_OPTION_ALERT,
        0
    );
}

#[cfg(target_os = "macos")]
#[test]
fn the_requested_options_map_onto_the_user_notifications_bits() {
    use objc2_user_notifications::UNAuthorizationOptions;

    // The plain bits above stand in for Apple's constants on every target; a
    // wrong value here would silently request the wrong permission.
    assert_eq!(
        UNAUTHORIZATION_OPTION_BADGE,
        UNAuthorizationOptions::Badge.bits() as u64
    );
    assert_eq!(
        UNAUTHORIZATION_OPTION_SOUND,
        UNAuthorizationOptions::Sound.bits() as u64
    );
    assert_eq!(
        UNAUTHORIZATION_OPTION_ALERT,
        UNAuthorizationOptions::Alert.bits() as u64
    );

    let requested = macos_notification_options(REQUESTED_NOTIFICATION_OPTIONS);
    assert!(requested.contains(UNAuthorizationOptions::Badge));
    assert!(requested.contains(UNAuthorizationOptions::Alert));
    assert!(requested.contains(UNAuthorizationOptions::Sound));
    assert_eq!(
        requested,
        UNAuthorizationOptions::Alert
            | UNAuthorizationOptions::Sound
            | UNAuthorizationOptions::Badge
    );
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn reading_the_grant_reports_what_was_requested_and_the_badge_setting() {
    use koushi_diagnostics::DiagnosticValue;

    let _guard = koushi_diagnostics::test_support::lock_async().await;
    let cursor = koushi_diagnostics::test_support::detail_cursor();
    let _ = current_authorization().await;
    let records = koushi_diagnostics::test_support::detail_records_since(cursor);
    let read = records
        .iter()
        .find(|record| {
            record.event.source == "desktop.native_notification"
                && record.event.stage == "grant_read"
        })
        .expect("a grant read records what the OS reports");
    let field = |key: &str| {
        read.event
            .fields
            .iter()
            .find(|field| field.key == key)
            .map(|field| match &field.value {
                DiagnosticValue::Boolean(value) => value.to_string(),
                DiagnosticValue::Token(token) => (*token).to_owned(),
                other => format!("{other:?}"),
            })
    };
    assert_eq!(field("alert_requested").as_deref(), Some("true"));
    assert_eq!(field("sound_requested").as_deref(), Some("true"));
    assert_eq!(field("badge_requested").as_deref(), Some("true"));
    // `cargo test` has no bundle, so the OS cannot answer.
    assert_eq!(field("authorization").as_deref(), Some("unavailable"));
    assert_eq!(field("badge_setting").as_deref(), Some("unknown"));
    // A read is not a request: it must not claim a grant answer.
    assert_eq!(field("granted"), None);
}

#[cfg(target_os = "macos")]
#[test]
fn every_badge_setting_maps_to_its_own_diagnostics_token() {
    use mac_usernotifications::NotificationSettingStatus as Setting;

    for (setting, token) in [
        (Setting::Enabled, "enabled"),
        (Setting::Disabled, "disabled"),
        (Setting::NotSupported, "not_supported"),
        (Setting::Unknown, "unknown"),
    ] {
        assert_eq!(badge_setting_token(setting), token);
    }
}
