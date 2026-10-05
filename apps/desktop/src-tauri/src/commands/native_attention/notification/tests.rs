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
fn pending_notification_fences_the_ready_account() {
    let mut state = ready_state();
    state.native_attention.notification = Some(payload());

    let pending = pending_notification(&state, account_tab_id()).expect("eligible banner");
    assert_eq!(pending.fence.account_key.0, "@attention:example.invalid");
    assert_eq!(pending.fence.account_tab_id, account_tab_id());
    assert_eq!(pending.payload.body, "Alice: private body");
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

/// Structural guard for the #1128 defect: the pinned macOS `notify-rust`
/// backend never sets this flag, so the macOS sender must ask for the click
/// response itself. This proves the request, not the OS behaviour; the live
/// smoke below is the behavioural evidence.
#[cfg(target_os = "macos")]
#[test]
fn macos_banner_requests_a_click_waiter() {
    assert!(macos_click_wait_requested());
}

#[cfg(target_os = "macos")]
#[test]
fn macos_banner_activates_only_on_a_direct_click() {
    use mac_notification_sys::NotificationResponse;

    assert!(macos_banner_activates(&NotificationResponse::Click));
    assert!(!macos_banner_activates(&NotificationResponse::None));
    assert!(!macos_banner_activates(&NotificationResponse::CloseButton(
        "Close".to_owned()
    )));
    assert!(!macos_banner_activates(
        &NotificationResponse::ActionButton("Show".to_owned())
    ));
    assert!(!macos_banner_activates(&NotificationResponse::Reply(
        "reply".to_owned()
    )));
}

/// #1128's reproduction against the production macOS send path.
///
/// A banner with a click waiter does **not** settle on its own: the pinned
/// fire-and-forget path returned after `mac-notification-sys`'s ~2 s delivery
/// confirmation, which is what let the adapter treat a click as never arriving.
/// This is the issue's headless oracle — no click and no visual assertion — and
/// it leaves the banner to dismiss.
///
/// Run on demand: `cargo test -p koushi-desktop --lib -- --ignored macos_click_waiter`
///
/// The dismissal half of this path (a dismissed banner must settle the waiter
/// with a non-activating response) needs the application's main run loop: the
/// pinned backend schedules its dismissal poll on `NSRunLoop mainRunLoop`, which
/// no `cargo test` harness runs. It is therefore covered by the packaged-build
/// check in #1128 rather than by a test here.
///
/// A dev/test binary has no bundle identifier, so the banner is attributed to
/// the development terminal identity; the packaged build is covered by the
/// manual check in #1128. Passing this proves the waiter did not settle by
/// itself; it does not prove the process registered a click handler for a
/// banner it never displayed.
#[cfg(target_os = "macos")]
#[test]
#[ignore = "shows a real macOS banner"]
fn macos_click_waiter_does_not_settle_without_interaction() {
    assert!(
        mac_notification_sys::set_application("com.apple.Terminal").is_ok(),
        "the smoke needs a bundle identity for the banner"
    );
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("koushi-notification-smoke".to_owned())
        .spawn(move || {
            let _ = sender.send(send_macos_banner_awaiting_click(
                "Koushi notification smoke",
                "Dismiss this banner when the run finishes",
            ));
        })
        .expect("smoke sender thread");

    // Long enough that the fire-and-forget path has already returned.
    std::thread::sleep(std::time::Duration::from_secs(3));
    match receiver.try_recv() {
        Err(std::sync::mpsc::TryRecvError::Empty) => {}
        Ok(response) => panic!("the banner settled with no click and no dismissal: {response:?}"),
        Err(error) => panic!("the smoke sender stopped early: {error}"),
    }
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

#[tokio::test]
async fn notification_activation_selects_owning_account_tab() {
    let data_dir = tempfile::tempdir().expect("data directory");
    let factory: std::sync::Arc<
        dyn Fn() -> std::sync::Arc<dyn koushi_core::NativeArtifactPort> + Send + Sync,
    > = std::sync::Arc::new(|| std::sync::Arc::new(koushi_core::NativeArtifactRegistry::new()));
    let runtime = AccountRuntimeManager::new(
        koushi_core::store::StoreActor::new(data_dir.path()),
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
    let activation = NativeNotificationActivation::new(&account_tab_id, payload().target);
    assert_eq!(
        serde_json::to_value(&activation).expect("activation serializes"),
        serde_json::json!({
            "account_tab_id": "account:@alice:example.invalid",
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
