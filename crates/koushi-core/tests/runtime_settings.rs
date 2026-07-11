//! Runtime settings integration tests.

use std::time::Duration;

use koushi_core::command::AppCommand;
use koushi_core::settings::{SettingsStore, SettingsStoreErrorKind};
use koushi_core::{CoreCommand, CoreEvent, CoreRuntime, executor};
use koushi_state::{
    DisplaySettings, MediaSettings, NativeAttentionCandidate, NativeAttentionCapabilities,
    NativeAttentionCapability, NativeAttentionDispatchState, NativeAttentionState,
    NativeAttentionSummary, NotificationSettings, RoomAttentionKind, SettingsPatch,
    SettingsPersistenceState, ThemePreference,
};

mod support;
use support::*;

#[tokio::test]
async fn app_update_settings_projects_state_and_persists() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let runtime = CoreRuntime::start_with_data_dir(data_dir.path().to_path_buf());
    let mut connection = runtime.attach();
    let request_id = connection.next_request_id();

    connection
        .command(CoreCommand::App(AppCommand::UpdateSettings {
            request_id,
            patch: dark_theme_settings_patch(),
        }))
        .await
        .expect("submit settings update");

    let snapshot = executor::timeout(Duration::from_secs(1), async {
        loop {
            match connection.recv_event().await.expect("event") {
                CoreEvent::StateChanged(snapshot)
                    if snapshot.settings.values.appearance.theme == ThemePreference::Dark =>
                {
                    return snapshot;
                }
                _ => continue,
            }
        }
    })
    .await
    .expect("settings state should change");

    assert_eq!(
        snapshot.settings.persistence,
        SettingsPersistenceState::Idle
    );
    let persisted = SettingsStore::new(data_dir.path())
        .load()
        .expect("load persisted settings");
    assert_eq!(persisted.appearance.theme, ThemePreference::Dark);
}

#[tokio::test]
async fn persisted_settings_load_when_runtime_restarts() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    {
        let runtime = CoreRuntime::start_with_data_dir(data_dir.path().to_path_buf());
        let mut connection = runtime.attach();
        let request_id = connection.next_request_id();

        connection
            .command(CoreCommand::App(AppCommand::UpdateSettings {
                request_id,
                patch: dark_theme_settings_patch(),
            }))
            .await
            .expect("submit settings update");

        executor::timeout(Duration::from_secs(1), async {
            loop {
                match connection.recv_event().await.expect("event") {
                    CoreEvent::StateChanged(snapshot)
                        if snapshot.settings.values.appearance.theme == ThemePreference::Dark
                            && snapshot.settings.persistence == SettingsPersistenceState::Idle =>
                    {
                        return;
                    }
                    _ => continue,
                }
            }
        })
        .await
        .expect("settings state should persist before restart");
    }

    let restarted = CoreRuntime::start_with_data_dir(data_dir.path().to_path_buf());
    let connection = restarted.attach();

    assert_eq!(
        connection.snapshot().settings.values.appearance.theme,
        ThemePreference::Dark
    );
    assert_eq!(
        connection.snapshot().settings.persistence,
        SettingsPersistenceState::Idle
    );
}

#[tokio::test]
async fn disabled_badges_remain_rust_projected_to_zero_after_runtime_restart() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    {
        let runtime = CoreRuntime::start_with_data_dir(data_dir.path().to_path_buf());
        let mut connection = runtime.attach();
        let request_id = connection.next_request_id();
        let notifications = NotificationSettings {
            badges: false,
            ..NotificationSettings::default()
        };

        connection
            .command(CoreCommand::App(AppCommand::UpdateSettings {
                request_id,
                patch: SettingsPatch {
                    notifications: Some(notifications),
                    ..SettingsPatch::default()
                },
            }))
            .await
            .expect("disable badges");

        executor::timeout(Duration::from_secs(1), async {
            loop {
                match connection.recv_event().await.expect("event") {
                    CoreEvent::StateChanged(snapshot)
                        if !snapshot.settings.values.notifications.badges
                            && snapshot.settings.persistence == SettingsPersistenceState::Idle =>
                    {
                        return;
                    }
                    _ => continue,
                }
            }
        })
        .await
        .expect("disabled badge setting should persist before restart");
    }

    let restarted = CoreRuntime::start_with_data_dir(data_dir.path().to_path_buf());
    let mut connection = restarted.attach();
    assert!(!connection.snapshot().settings.values.notifications.badges);

    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::App(AppCommand::UpdateNativeAttentionState {
            request_id,
            attention: NativeAttentionState {
                summary: NativeAttentionSummary {
                    unread_count: 5,
                    highlight_count: 1,
                    badge_count: 5,
                    candidate: Some(NativeAttentionCandidate {
                        room_display_name: "Room".to_owned(),
                        kind: RoomAttentionKind::Mention,
                        unread_count: 5,
                        highlight_count: 1,
                    }),
                    capabilities: NativeAttentionCapabilities {
                        badge: NativeAttentionCapability::Available,
                        ..NativeAttentionCapabilities::default()
                    },
                },
                dispatch: NativeAttentionDispatchState::Idle,
            },
        }))
        .await
        .expect("project attention after restart");

    let snapshot = executor::timeout(Duration::from_secs(1), async {
        loop {
            match connection.recv_event().await.expect("event") {
                CoreEvent::StateChanged(snapshot)
                    if snapshot.native_attention.summary.unread_count == 5 =>
                {
                    return snapshot;
                }
                _ => continue,
            }
        }
    })
    .await
    .expect("attention should project after restored settings load");

    assert!(!snapshot.settings.values.notifications.badges);
    assert_eq!(snapshot.native_attention.summary.badge_count, 0);
}

#[test]
fn settings_store_rejects_corrupt_json_with_defaults() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let settings_dir = data_dir.path().join("settings");
    std::fs::create_dir_all(&settings_dir).expect("settings dir");
    std::fs::write(settings_dir.join("settings.json"), "{not-json").expect("write corrupt");

    let store = SettingsStore::new(data_dir.path());
    let err = store
        .load()
        .expect_err("corrupt settings should fail safely");

    assert_eq!(err.kind(), SettingsStoreErrorKind::Corrupt);
}

#[test]
fn settings_store_loads_legacy_json_without_notification_settings() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let settings_dir = data_dir.path().join("settings");
    std::fs::create_dir_all(&settings_dir).expect("settings dir");
    std::fs::write(
        settings_dir.join("settings.json"),
        r#"{
  "locale": { "language_tag": null, "text_direction": "auto" },
  "appearance": { "theme": "dark" },
  "typography": { "font": "system", "emoji": "system" },
  "keyboard": { "composer_send_shortcut": "enter" }
}
"#,
    )
    .expect("write legacy settings");

    let values = SettingsStore::new(data_dir.path())
        .load()
        .expect("legacy settings should load with default notification settings");

    assert_eq!(values.appearance.theme, ThemePreference::Dark);
    assert_eq!(values.notifications, NotificationSettings::default());
    assert_eq!(values.display, DisplaySettings::default());
    assert_eq!(values.media, MediaSettings::default());
}
