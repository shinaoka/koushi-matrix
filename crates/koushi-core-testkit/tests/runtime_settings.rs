//! Runtime settings integration tests.

use koushi_core::settings::{SETTINGS_SCHEMA_VERSION, SettingsStore, SettingsStoreErrorKind};
use koushi_core::{CoreCommand, CoreRuntime};
use koushi_protocol::command::AppCommand;
use koushi_state::{
    AppearanceSettings, DisplayDensity, DisplaySettings, MediaSettings, NativeAttentionCandidate,
    NativeAttentionCapabilities, NativeAttentionCapability, NativeAttentionDispatchState,
    NativeAttentionState, NativeAttentionSummary, NotificationSettings, RoomAttentionKind,
    SettingsPatch, SettingsPersistenceState, ThemePreference,
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

    let snapshot = support::wait_for_state_event(&mut connection, |state| {
        state.settings.values.appearance.theme == ThemePreference::Dark
    })
    .await;

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
async fn legacy_settings_import_persists_once_and_ignores_replay() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let runtime = CoreRuntime::start_with_data_dir(data_dir.path().to_path_buf());
    let connection = runtime.attach();

    let request_id = connection.next_request_id();
    connection
        .command_with_admission(CoreCommand::App(AppCommand::ImportLegacySettings {
            request_id,
            patch: SettingsPatch {
                appearance: Some(AppearanceSettings {
                    density: DisplayDensity::Compact,
                    ..AppearanceSettings::default()
                }),
                ..SettingsPatch::default()
            },
        }))
        .await
        .expect("import legacy settings");

    assert_eq!(
        connection.snapshot().settings.values.appearance.density,
        DisplayDensity::Compact
    );
    assert!(
        connection
            .snapshot()
            .settings
            .values
            .legacy_frontend_preferences_imported
    );

    let replay_id = connection.next_request_id();
    connection
        .command_with_admission(CoreCommand::App(AppCommand::ImportLegacySettings {
            request_id: replay_id,
            patch: SettingsPatch {
                appearance: Some(AppearanceSettings {
                    density: DisplayDensity::Comfortable,
                    ..AppearanceSettings::default()
                }),
                ..SettingsPatch::default()
            },
        }))
        .await
        .expect("admit ignored replay");

    let persisted = SettingsStore::new(data_dir.path())
        .load()
        .expect("load imported settings");
    assert_eq!(persisted.appearance.density, DisplayDensity::Compact);
    assert!(persisted.legacy_frontend_preferences_imported);
}

#[tokio::test]
async fn legacy_settings_import_rejects_a_failed_initial_load() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let settings_dir = data_dir.path().join("settings");
    std::fs::create_dir_all(&settings_dir).expect("settings dir");
    std::fs::write(settings_dir.join("settings.json"), "{not-json").expect("corrupt settings");
    let runtime = CoreRuntime::start_with_data_dir(data_dir.path().to_path_buf());
    let connection = runtime.attach();

    connection
        .command_with_admission(CoreCommand::App(AppCommand::ImportLegacySettings {
            request_id: connection.next_request_id(),
            patch: dark_theme_settings_patch(),
        }))
        .await
        .expect("admit rejected import");

    assert!(
        !connection
            .snapshot()
            .settings
            .values
            .legacy_frontend_preferences_imported
    );
    assert_eq!(
        connection.snapshot().settings.values.appearance.theme,
        ThemePreference::System
    );
    assert_eq!(
        std::fs::read_to_string(settings_dir.join("settings.json")).expect("corrupt file remains"),
        "{not-json"
    );
}

#[tokio::test]
async fn legacy_settings_import_does_not_project_before_persistence() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let runtime = CoreRuntime::start_with_data_dir(data_dir.path().to_path_buf());
    let connection = runtime.attach();
    let settings_path = data_dir.path().join("settings/settings.json");
    std::fs::create_dir_all(&settings_path).expect("block atomic replacement with directory");

    connection
        .command_with_admission(CoreCommand::App(AppCommand::ImportLegacySettings {
            request_id: connection.next_request_id(),
            patch: dark_theme_settings_patch(),
        }))
        .await
        .expect("admit failed persist");

    assert!(
        !connection
            .snapshot()
            .settings
            .values
            .legacy_frontend_preferences_imported
    );
    assert_eq!(
        connection.snapshot().settings.values.appearance.theme,
        ThemePreference::System
    );
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

        support::wait_for_state_event(&mut connection, |state| {
            state.settings.values.appearance.theme == ThemePreference::Dark
                && state.settings.persistence == SettingsPersistenceState::Idle
        })
        .await;
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

        support::wait_for_state_event(&mut connection, |state| {
            !state.settings.values.notifications.badges
                && state.settings.persistence == SettingsPersistenceState::Idle
        })
        .await;
    }

    let restarted = CoreRuntime::start_with_data_dir(data_dir.path().to_path_buf());
    let mut connection = restarted.attach();
    assert!(!connection.snapshot().settings.values.notifications.badges);
    restarted.inject_actions(restore_ready_actions()).await;
    wait_for_state(&mut connection, |state| {
        matches!(state.session, koushi_state::SessionState::Ready(_))
    })
    .await;

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
                notification: None,
            },
        }))
        .await
        .expect("project attention after restart");

    let snapshot = wait_for_state(&mut connection, |state| {
        state.native_attention.summary.unread_count == 5
    })
    .await;

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

fn write_settings_file(data_dir: &std::path::Path, json: &str) -> std::path::PathBuf {
    let settings_dir = data_dir.join("settings");
    std::fs::create_dir_all(&settings_dir).expect("settings dir");
    let path = settings_dir.join("settings.json");
    std::fs::write(&path, json).expect("write settings");
    path
}

fn persisted_settings_json(path: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).expect("read settings"))
        .expect("persisted settings are JSON")
}

const LEGACY_OPTED_IN_ENCRYPTED_PREVIEWS: &str = r#"{
  "locale": { "language_tag": null, "text_direction": "auto" },
  "appearance": { "theme": "dark" },
  "typography": { "font": "system", "emoji": "system" },
  "keyboard": { "composer_send_shortcut": "enter" },
  "display": {
    "code_block_wrap": true,
    "hide_redacted": true,
    "url_previews_enabled": true,
    "encrypted_url_previews_enabled": true
  }
}
"#;

#[test]
fn settings_store_resets_unversioned_encrypted_url_preview_opt_in() {
    // #1034: files written before the settings schema version existed may
    // carry `encrypted_url_previews_enabled: true` from the retired default
    // without any explicit user opt-in.
    let data_dir = tempfile::tempdir().expect("tempdir");
    let path = write_settings_file(data_dir.path(), LEGACY_OPTED_IN_ENCRYPTED_PREVIEWS);

    let values = SettingsStore::new(data_dir.path())
        .load()
        .expect("legacy settings load");

    assert!(!values.display.encrypted_url_previews_enabled);
    assert!(values.display.url_previews_enabled);
    assert_eq!(values.appearance.theme, ThemePreference::Dark);

    let persisted = persisted_settings_json(&path);
    assert_eq!(
        persisted["schema_version"],
        serde_json::json!(SETTINGS_SCHEMA_VERSION)
    );
    assert_eq!(
        persisted["display"]["encrypted_url_previews_enabled"],
        serde_json::json!(false)
    );
}

#[test]
fn settings_store_keeps_versioned_encrypted_url_preview_opt_in() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let store = SettingsStore::new(data_dir.path());
    let mut values = store.load().expect("default settings");
    values.display.encrypted_url_previews_enabled = true;
    store.save(&values).expect("save explicit opt-in");

    let path = data_dir.path().join("settings/settings.json");
    assert_eq!(
        persisted_settings_json(&path)["schema_version"],
        serde_json::json!(SETTINGS_SCHEMA_VERSION)
    );

    let reloaded = store.load().expect("reload versioned settings");
    assert!(reloaded.display.encrypted_url_previews_enabled);
    assert_eq!(reloaded, values);
    // A second load must not re-run the migration.
    assert!(
        store
            .load()
            .expect("load again")
            .display
            .encrypted_url_previews_enabled
    );
}

#[test]
fn settings_store_round_trips_versioned_values() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let store = SettingsStore::new(data_dir.path());
    let mut values = store.load().expect("default settings");
    values.appearance.theme = ThemePreference::Dark;
    values.display.url_previews_enabled = false;
    values.legacy_frontend_preferences_imported = true;
    store.save(&values).expect("save");

    assert_eq!(store.load().expect("reload"), values);
}

#[test]
fn settings_store_rejects_non_integer_schema_version_as_corrupt() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    write_settings_file(
        data_dir.path(),
        r#"{ "schema_version": "one", "locale": { "language_tag": null, "text_direction": "auto" }, "appearance": { "theme": "dark" }, "typography": { "font": "system", "emoji": "system" }, "keyboard": { "composer_send_shortcut": "enter" } }"#,
    );

    let err = SettingsStore::new(data_dir.path())
        .load()
        .expect_err("malformed schema version fails safely");
    assert_eq!(err.kind(), SettingsStoreErrorKind::Corrupt);
}

/// A 0.16.x file: schema version 1 persisted the whole `SettingsValues`,
/// including the retired `message_previews: true` default, alongside an
/// explicit version-1 encrypted-room link-preview opt-in.
const VERSION_1_WITH_RETIRED_MESSAGE_PREVIEW_DEFAULT: &str = r#"{
  "schema_version": 1,
  "locale": { "language_tag": "ja-JP", "text_direction": "auto" },
  "appearance": { "theme": "dark" },
  "typography": { "font": "system", "emoji": "system" },
  "keyboard": { "composer_send_shortcut": "enter" },
  "notifications": {
    "desktop_notifications": true,
    "sound": false,
    "badges": true,
    "message_previews": true,
    "send_read_receipts": true,
    "send_typing_notifications": true
  },
  "display": {
    "code_block_wrap": true,
    "hide_redacted": true,
    "url_previews_enabled": true,
    "encrypted_url_previews_enabled": true
  }
}
"#;

#[test]
fn settings_store_resets_version_1_message_previews_default() {
    // #1054: version-1 files cannot distinguish an explicit opt-in from the
    // retired ON default, so message previews reset to OFF exactly once.
    let data_dir = tempfile::tempdir().expect("tempdir");
    let path = write_settings_file(
        data_dir.path(),
        VERSION_1_WITH_RETIRED_MESSAGE_PREVIEW_DEFAULT,
    );

    let values = SettingsStore::new(data_dir.path())
        .load()
        .expect("version-1 settings load");

    assert!(!values.notifications.message_previews);
    // Unrelated values survive, and the version-0 (#1034) reset does not
    // re-run for a version-1 opt-in.
    assert!(values.display.encrypted_url_previews_enabled);
    assert!(!values.notifications.sound);
    assert_eq!(values.locale.language_tag.as_deref(), Some("ja-JP"));
    assert_eq!(values.appearance.theme, ThemePreference::Dark);

    let persisted = persisted_settings_json(&path);
    assert_eq!(persisted["schema_version"], serde_json::json!(2));
    assert_eq!(
        persisted["notifications"]["message_previews"],
        serde_json::json!(false)
    );
}

#[test]
fn settings_store_resets_unversioned_message_previews_default() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    write_settings_file(
        data_dir.path(),
        &VERSION_1_WITH_RETIRED_MESSAGE_PREVIEW_DEFAULT.replace("\"schema_version\": 1,", ""),
    );

    let values = SettingsStore::new(data_dir.path())
        .load()
        .expect("unversioned settings load");

    assert!(!values.notifications.message_previews);
    assert!(!values.display.encrypted_url_previews_enabled);
}

#[test]
fn settings_store_keeps_versioned_message_previews_opt_in() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let store = SettingsStore::new(data_dir.path());
    let mut values = store.load().expect("default settings");
    assert!(!values.notifications.message_previews);
    values.notifications.message_previews = true;
    store.save(&values).expect("save explicit opt-in");

    let reloaded = store.load().expect("reload versioned settings");
    assert!(reloaded.notifications.message_previews);
    assert_eq!(reloaded, values);
    assert!(
        store
            .load()
            .expect("load again")
            .notifications
            .message_previews
    );
}

#[tokio::test]
async fn runtime_start_migrates_unversioned_encrypted_url_preview_opt_in() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let path = write_settings_file(data_dir.path(), LEGACY_OPTED_IN_ENCRYPTED_PREVIEWS);

    let runtime = CoreRuntime::start_with_data_dir(data_dir.path().to_path_buf());
    let connection = runtime.attach();

    assert!(
        !connection
            .snapshot()
            .settings
            .values
            .display
            .encrypted_url_previews_enabled
    );
    assert_eq!(
        persisted_settings_json(&path)["schema_version"],
        serde_json::json!(SETTINGS_SCHEMA_VERSION)
    );
}
