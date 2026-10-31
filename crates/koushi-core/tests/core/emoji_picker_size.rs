use koushi_core::settings::SettingsStore;
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
