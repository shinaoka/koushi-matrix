//! Account ownership and conservative migration of notification preferences.
use koushi_core::settings::{SETTINGS_SCHEMA_VERSION, SettingsStore};
use koushi_state::{SettingsPatch, SettingsPatchScope, SettingsValues};

#[test]
fn old_global_routes_are_archived_and_never_become_live_settings() {
    let directory = tempfile::tempdir().expect("isolated settings");
    let path = directory.path().join("settings/settings.json");
    std::fs::create_dir_all(path.parent().expect("settings parent")).expect("create settings");
    let mut old =
        serde_json::to_value(SettingsValues::default().app_settings()).expect("old app values");
    old["schema_version"] = 3.into();
    old["notifications"] = serde_json::json!({"sound": false, "badges": true});
    old["delivery"] = serde_json::json!({"sinks": [{"id": "legacy-route", "destination": "fixture@example.invalid"}]});
    std::fs::write(&path, serde_json::to_vec(&old).expect("old json")).expect("write legacy");
    let store = SettingsStore::new(directory.path());
    assert!(store.load().expect("load old").delivery.sinks.is_empty());
    assert_eq!(
        store.legacy_notification_defaults().expect("old defaults"),
        Some((false, true))
    );
    store
        .save_patch(&SettingsPatch {
            scope: Some(SettingsPatchScope::App),
            ..Default::default()
        })
        .expect("save app");
    store
        .complete_legacy_account_migration()
        .expect("complete prior migration");
    let disk: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).expect("read upgraded"))
            .expect("upgraded json");
    assert_eq!(disk["schema_version"], SETTINGS_SCHEMA_VERSION);
    assert_eq!(disk["legacy_notification_archive"], old);
    assert!(disk.get("delivery").is_none());
    assert!(disk.get("notifications").is_none());
    let restarted = SettingsStore::new(directory.path());
    assert!(
        restarted
            .load()
            .expect("restart load")
            .delivery
            .sinks
            .is_empty()
    );
    assert_eq!(
        restarted
            .legacy_notification_defaults()
            .expect("restart defaults"),
        Some((false, true))
    );
}

#[path = "../support/mod.rs"]
#[expect(
    clippy::duplicate_mod,
    reason = "suite aggregation preserves feature-local fixture scopes and registered helper tests"
)]
mod support;

async fn ready_account(
    runtime: &koushi_core::CoreRuntime,
    user: &str,
) -> koushi_core::runtime::CoreConnection {
    let mut connection = runtime.attach();
    runtime
        .inject_actions(vec![
            koushi_state::AppAction::AppStarted,
            koushi_state::AppAction::RestoreSessionSucceeded(koushi_state::SessionInfo {
                homeserver: "https://example.invalid".into(),
                user_id: user.into(),
                device_id: "DEVICE".into(),
                authentication_method: Default::default(),
            }),
            koushi_state::AppAction::CurrentDeviceTrustChanged(
                koushi_state::CurrentDeviceTrustState::Verified,
            ),
        ])
        .await;
    support::wait_for_state(&mut connection, |state| {
        matches!(state.session, koushi_state::SessionState::Ready(_))
    })
    .await;
    connection
}

async fn save_route(connection: &mut koushi_core::runtime::CoreConnection, recipient: &str) {
    let sink = koushi_state::NotificationSinkSettings {
        id: "mail".into(),
        destination: recipient.into(),
        from_address: "sender@example.invalid".into(),
        config: koushi_state::NotificationSinkConfig::Resend {
            endpoint: "https://relay.example.invalid/emails".into(),
        },
        ..Default::default()
    };
    connection
        .command(koushi_core::CoreCommand::App(
            koushi_protocol::command::AppCommand::SaveNotificationSink {
                request_id: connection.next_request_id(),
                sink,
                secret: Some(koushi_state::AuthSecret::new("fixture-notification-key")),
                accept_content_consent: false,
            },
        ))
        .await
        .expect("save route command");
    support::wait_for_state(connection, |state| {
        state
            .notification_delivery
            .sinks
            .iter()
            .any(|sink| sink.secret_configured)
    })
    .await;
}

#[tokio::test]
async fn simultaneous_accounts_keep_notification_policy_and_same_named_routes_independent() {
    use koushi_core::CoreRuntime;
    let (account_a, stores) = CoreRuntime::start_isolated();
    let mut a = ready_account(&account_a, "@account-a:example.invalid").await;
    let account_b = CoreRuntime::restart_isolated(&stores);
    let mut b = ready_account(&account_b, "@account-b:example.invalid").await;
    save_route(&mut a, "account-a@example.invalid").await;
    assert!(b.snapshot().settings.values.delivery.sinks.is_empty());
    save_route(&mut b, "account-b@example.invalid").await;
    let mut notifications = a.snapshot().settings.values.notifications.clone();
    notifications.sound = false;
    notifications.badges = false;
    a.command(koushi_core::CoreCommand::App(
        koushi_protocol::command::AppCommand::UpdateSettings {
            request_id: a.next_request_id(),
            patch: SettingsPatch {
                scope: Some(SettingsPatchScope::Account),
                notifications: Some(notifications),
                ..Default::default()
            },
        },
    ))
    .await
    .expect("account notification policy");
    support::wait_for_state(&mut a, |state| {
        !state.settings.values.notifications.sound
            && state.settings.persistence == koushi_state::SettingsPersistenceState::Idle
    })
    .await;
    assert!(b.snapshot().settings.values.notifications.sound);
    assert!(b.snapshot().settings.values.notifications.badges);
    assert_eq!(
        a.snapshot().settings.values.delivery.sinks[0].destination,
        "account-a@example.invalid"
    );
    assert_eq!(
        b.snapshot().settings.values.delivery.sinks[0].destination,
        "account-b@example.invalid"
    );
    drop(a);
    account_a
        .shutdown_checked()
        .await
        .expect("ordered shutdown");
    let restarted_a = CoreRuntime::restart_isolated(&stores);
    let mut a = ready_account(&restarted_a, "@account-a:example.invalid").await;
    support::wait_for_state(&mut a, |state| {
        !state.settings.values.notifications.sound
            && state
                .notification_delivery
                .sinks
                .iter()
                .any(|sink| sink.secret_configured)
    })
    .await;
    assert_eq!(
        a.snapshot().settings.values.delivery.sinks[0].destination,
        "account-a@example.invalid"
    );
    assert_eq!(
        b.snapshot().settings.values.delivery.sinks[0].destination,
        "account-b@example.invalid"
    );
}

#[tokio::test]
async fn a_signed_out_runtime_cannot_save_external_destinations_or_credentials() {
    use koushi_core::CoreRuntime;
    let (runtime, _stores) = CoreRuntime::start_isolated();
    let mut connection = runtime.attach();
    let request_id = connection.next_request_id();
    connection
        .command(koushi_core::CoreCommand::App(
            koushi_protocol::command::AppCommand::SetNotificationDeliverySecret {
                request_id,
                sink_id: "mail".into(),
                secret: koushi_state::AuthSecret::new("fixture-notification-key"),
            },
        ))
        .await
        .expect("admit rejected request");
    let failure = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Ok(koushi_core::CoreEvent::OperationFailed {
                request_id: failed,
                failure,
            }) = connection.recv_event().await
                && failed == request_id
            {
                break failure;
            }
        }
    })
    .await
    .expect("rejection settles");
    assert_eq!(failure, koushi_protocol::CoreFailure::StoreUnavailable);
    assert!(
        connection
            .snapshot()
            .settings
            .values
            .delivery
            .sinks
            .is_empty()
    );
}
