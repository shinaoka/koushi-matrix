use std::time::Duration;

use koushi_core::runtime::CoreRuntime;
use koushi_core::settings::SettingsStore;
use koushi_protocol::{AppCommand, CoreCommand};
use koushi_state::{SettingsPatch, SettingsPatchScope, WindowSettings};

#[test]
fn emoji_picker_size_survives_a_fresh_store_and_preserves_window_preferences() {
    let directory = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(directory.path());
    store
        .save_patch(&SettingsPatch {
            scope: Some(SettingsPatchScope::App),
            window: Some(WindowSettings {
                close_to_tray: false,
                emoji_picker_size: Some([640, 420]),
            }),
            ..Default::default()
        })
        .unwrap();
    drop(store);
    let restored = SettingsStore::new(directory.path()).load().unwrap();
    assert_eq!(restored.window.emoji_picker_size, Some([640, 420]));
    assert!(!restored.window.close_to_tray);
}

#[test]
fn old_window_preferences_default_to_the_original_picker_size() {
    let window: WindowSettings = serde_json::from_str(r#"{"close_to_tray":false}"#).unwrap();
    assert_eq!(window.emoji_picker_size, None);
    assert!(!window.close_to_tray);
}

/// #1583 follow-up: the renderer resizes the picker with `UpdateSettings`, then
/// re-reads the size from the snapshot it gets back, and a restarted app loads
/// it from disk. Both hops are measured here so a loss cannot hide in either.
#[test]
fn update_settings_carries_the_emoji_picker_size_into_the_snapshot_and_a_restart() {
    let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_stack_size(2 * 1024 * 1024)
        .enable_all()
        .build()
        .expect("test runtime");

    tokio_runtime.block_on(async {
        let data_dir = tempfile::tempdir().expect("runtime data directory");
        let core = CoreRuntime::start_with_data_dir(data_dir.path().to_owned());
        let mut connection = core.attach();
        let request_id = connection.next_request_id();
        connection
            .command(CoreCommand::App(AppCommand::UpdateSettings {
                request_id,
                patch: SettingsPatch {
                    scope: Some(SettingsPatchScope::App),
                    window: Some(WindowSettings {
                        close_to_tray: true,
                        emoji_picker_size: Some([640, 420]),
                    }),
                    ..Default::default()
                },
            }))
            .await
            .expect("resize command");

        let settled = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let snapshot = connection.snapshot();
                if snapshot.settings.persistence == koushi_state::SettingsPersistenceState::Idle
                    && snapshot.settings.values.window.emoji_picker_size.is_some()
                {
                    return snapshot;
                }
                connection
                    .next_versioned_snapshot()
                    .await
                    .expect("runtime snapshot stream should remain open");
            }
        })
        .await
        .expect("picker size should reach the published snapshot");
        assert_eq!(
            settled.settings.values.window.emoji_picker_size,
            Some([640, 420]),
            "the snapshot the renderer re-reads after the command"
        );

        let on_disk =
            std::fs::read_to_string(data_dir.path().join("settings").join("settings.json"))
                .expect("settings file");
        assert!(
            on_disk.contains("\"emoji_picker_size\""),
            "settings file should carry the picker size: {on_disk}"
        );

        drop(connection);
        core.shutdown().await;

        let restarted = CoreRuntime::start_with_data_dir(data_dir.path().to_owned());
        let mut connection = restarted.attach();
        let loaded = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let snapshot = connection.snapshot();
                if snapshot.settings.persistence == koushi_state::SettingsPersistenceState::Idle
                    && snapshot.settings.values.window.emoji_picker_size.is_some()
                {
                    return snapshot;
                }
                connection
                    .next_versioned_snapshot()
                    .await
                    .expect("runtime snapshot stream should remain open");
            }
        })
        .await
        .expect("a restarted app should load the saved picker size");
        assert_eq!(
            loaded.settings.values.window.emoji_picker_size,
            Some([640, 420]),
            "the snapshot a restarted app opens with"
        );
        drop(connection);
        restarted.shutdown().await;
    });
}
