use matrix_desktop_state::{
    AppAction, AppEffect, AppState, AppearanceSettings, ComposerSendShortcut, DisplaySettings,
    EmojiPreference, FontPreference, ImageUploadCompressionMode, KeyboardSettings, LocaleSettings,
    MediaSettings, NotificationSettings, SettingsPatch, SettingsPersistenceState, SettingsValues,
    TextDirectionPreference, ThemePreference, UiEvent, reduce,
};

fn dark_theme_patch() -> SettingsPatch {
    SettingsPatch {
        appearance: Some(AppearanceSettings {
            theme: ThemePreference::Dark,
        }),
        ..SettingsPatch::default()
    }
}

#[test]
fn app_state_carries_default_non_secret_settings() {
    let state = AppState::default();

    assert_eq!(
        state.settings.values.appearance.theme,
        ThemePreference::System
    );
    assert_eq!(
        state.settings.values.keyboard.composer_send_shortcut,
        ComposerSendShortcut::Enter
    );
    assert_eq!(state.settings.values.locale.language_tag, None);
    assert_eq!(
        state.settings.values.locale.text_direction,
        TextDirectionPreference::Auto
    );
    assert_eq!(
        state.settings.values.typography.font,
        FontPreference::System
    );
    assert_eq!(
        state.settings.values.typography.emoji,
        EmojiPreference::System
    );
    assert_eq!(
        state.settings.values.notifications,
        NotificationSettings::default()
    );
    assert_eq!(
        state.settings.values.display,
        DisplaySettings {
            code_block_wrap: true,
            hide_redacted: false,
            url_previews_enabled: true,
        }
    );
    assert_eq!(
        state.settings.values.media,
        MediaSettings {
            image_upload_compression: ImageUploadCompressionMode::Never,
            ..MediaSettings::default()
        }
    );
    assert_eq!(state.settings.persistence, SettingsPersistenceState::Idle);
}

#[test]
fn settings_loaded_replaces_values_without_requiring_a_session() {
    let mut state = AppState::default();
    let values = SettingsValues {
        locale: LocaleSettings {
            language_tag: Some("ja-JP".to_owned()),
            text_direction: TextDirectionPreference::Auto,
        },
        appearance: AppearanceSettings {
            theme: ThemePreference::Light,
        },
        typography: matrix_desktop_state::TypographySettings {
            font: FontPreference::Inter,
            emoji: EmojiPreference::TwemojiColr,
        },
        keyboard: KeyboardSettings {
            composer_send_shortcut: ComposerSendShortcut::ModEnter,
        },
        notifications: NotificationSettings {
            desktop_notifications: false,
            sound: false,
            badges: true,
            send_read_receipts: true,
            send_typing_notifications: true,
        },
        display: DisplaySettings {
            code_block_wrap: false,
            hide_redacted: true,
            url_previews_enabled: true,
        },
        media: MediaSettings {
            image_upload_compression: ImageUploadCompressionMode::Always,
            ..MediaSettings::default()
        },
        room_url_previews: Default::default(),
    };

    let effects = reduce(
        &mut state,
        AppAction::SettingsLoaded {
            values: values.clone(),
        },
    );

    assert_eq!(state.settings.values, values);
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::SettingsChanged)]
    );
}

#[test]
fn settings_values_deserialize_empty_display_as_default() {
    let values = serde_json::from_str::<SettingsValues>(
        r#"{
  "locale": { "language_tag": null, "text_direction": "auto" },
  "appearance": { "theme": "system" },
  "typography": { "font": "system", "emoji": "system" },
  "keyboard": { "composer_send_shortcut": "enter" },
  "notifications": { "desktop_notifications": true, "sound": true, "badges": true },
  "display": {}
}
"#,
    )
    .expect("empty display object should deserialize");

    assert_eq!(values.display, DisplaySettings::default());
}

#[test]
fn settings_values_deserialize_legacy_display_without_hide_redacted_as_default_off() {
    let values = serde_json::from_str::<SettingsValues>(
        r#"{
  "locale": { "language_tag": null, "text_direction": "auto" },
  "appearance": { "theme": "system" },
  "typography": { "font": "system", "emoji": "system" },
  "keyboard": { "composer_send_shortcut": "enter" },
  "notifications": { "desktop_notifications": true, "sound": true, "badges": true },
  "display": { "code_block_wrap": false }
}
"#,
    )
    .expect("legacy display object should deserialize");

    assert_eq!(
        values.display,
        DisplaySettings {
            code_block_wrap: false,
            hide_redacted: false,
            url_previews_enabled: true,
        }
    );
}

#[test]
fn settings_values_deserialize_legacy_without_media_as_default_never() {
    let values = serde_json::from_str::<SettingsValues>(
        r#"{
  "locale": { "language_tag": null, "text_direction": "auto" },
  "appearance": { "theme": "system" },
  "typography": { "font": "system", "emoji": "system" },
  "keyboard": { "composer_send_shortcut": "enter" },
  "notifications": { "desktop_notifications": true, "sound": true, "badges": true },
  "display": { "code_block_wrap": true, "hide_redacted": false }
}
"#,
    )
    .expect("legacy settings without media should deserialize");

    assert_eq!(
        values.media,
        MediaSettings {
            image_upload_compression: ImageUploadCompressionMode::Never,
            ..MediaSettings::default()
        }
    );
}

#[test]
fn notification_settings_patch_is_rust_owned_and_persisted() {
    let mut state = AppState::default();
    let notification_settings = NotificationSettings {
        desktop_notifications: false,
        sound: false,
        badges: false,
        send_read_receipts: false,
        send_typing_notifications: false,
    };

    let effects = reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 77,
            patch: SettingsPatch {
                notifications: Some(notification_settings.clone()),
                ..SettingsPatch::default()
            },
        },
    );

    assert_eq!(state.settings.values.notifications, notification_settings);
    assert_eq!(
        state.settings.persistence,
        SettingsPersistenceState::Saving { request_id: 77 }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::PersistSettings {
                request_id: 77,
                values: state.settings.values.clone(),
            },
            AppEffect::EmitUiEvent(UiEvent::SettingsChanged),
        ]
    );
}

#[test]
fn code_block_wrap_patch_is_rust_owned_and_persisted() {
    let mut state = AppState::default();
    let display_settings = DisplaySettings {
        code_block_wrap: false,
        hide_redacted: false,
        url_previews_enabled: true,
    };

    let effects = reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 78,
            patch: SettingsPatch {
                display: Some(display_settings.clone()),
                ..SettingsPatch::default()
            },
        },
    );

    assert_eq!(state.settings.values.display, display_settings);
    assert_eq!(
        state.settings.persistence,
        SettingsPersistenceState::Saving { request_id: 78 }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::PersistSettings {
                request_id: 78,
                values: state.settings.values.clone(),
            },
            AppEffect::EmitUiEvent(UiEvent::SettingsChanged),
        ]
    );
}

#[test]
fn hide_redacted_patch_is_rust_owned_and_persisted() {
    let mut state = AppState::default();
    let display_settings = DisplaySettings {
        code_block_wrap: true,
        hide_redacted: true,
        url_previews_enabled: true,
    };

    let effects = reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 79,
            patch: SettingsPatch {
                display: Some(display_settings.clone()),
                ..SettingsPatch::default()
            },
        },
    );

    assert_eq!(state.settings.values.display, display_settings);
    assert_eq!(
        state.settings.persistence,
        SettingsPersistenceState::Saving { request_id: 79 }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::PersistSettings {
                request_id: 79,
                values: state.settings.values.clone(),
            },
            AppEffect::EmitUiEvent(UiEvent::SettingsChanged),
        ]
    );
}

#[test]
fn image_upload_compression_patch_is_rust_owned_and_persisted() {
    let mut state = AppState::default();
    let media_settings = MediaSettings {
        image_upload_compression: ImageUploadCompressionMode::Ask,
        ..MediaSettings::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 80,
            patch: SettingsPatch {
                media: Some(media_settings.clone()),
                ..SettingsPatch::default()
            },
        },
    );

    assert_eq!(state.settings.values.media, media_settings);
    assert_eq!(
        state.settings.persistence,
        SettingsPersistenceState::Saving { request_id: 80 }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::PersistSettings {
                request_id: 80,
                values: state.settings.values.clone(),
            },
            AppEffect::EmitUiEvent(UiEvent::SettingsChanged),
        ]
    );
}

#[test]
fn settings_update_is_optimistic_and_emits_a_persist_effect() {
    let mut state = AppState::default();

    let effects = reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 42,
            patch: dark_theme_patch(),
        },
    );

    assert_eq!(
        state.settings.values.appearance.theme,
        ThemePreference::Dark
    );
    assert_eq!(
        state.settings.persistence,
        SettingsPersistenceState::Saving { request_id: 42 }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::PersistSettings {
                request_id: 42,
                values: state.settings.values.clone(),
            },
            AppEffect::EmitUiEvent(UiEvent::SettingsChanged),
        ]
    );
}

#[test]
fn settings_persist_settle_requires_matching_request_id() {
    let mut state = AppState::default();

    reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 7,
            patch: dark_theme_patch(),
        },
    );

    let stale = reduce(&mut state, AppAction::SettingsPersisted { request_id: 999 });
    assert_eq!(stale, Vec::new());
    assert_eq!(
        state.settings.persistence,
        SettingsPersistenceState::Saving { request_id: 7 }
    );

    let matched = reduce(&mut state, AppAction::SettingsPersisted { request_id: 7 });
    assert_eq!(state.settings.persistence, SettingsPersistenceState::Idle);
    assert_eq!(
        matched,
        vec![AppEffect::EmitUiEvent(UiEvent::SettingsChanged)]
    );
}

#[test]
fn settings_load_and_persist_failures_are_private_data_free() {
    let mut state = AppState::default();

    reduce(
        &mut state,
        AppAction::SettingsLoadFailed {
            message: "settings file is corrupt".to_owned(),
        },
    );
    assert_eq!(state.settings.values, SettingsValues::default());
    assert_eq!(state.errors[0].code, "settings_load_failed");
    assert!(!state.errors[0].message.contains("@"));

    reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 3,
            patch: dark_theme_patch(),
        },
    );
    reduce(
        &mut state,
        AppAction::SettingsPersistFailed {
            request_id: 3,
            message: "settings file could not be saved".to_owned(),
        },
    );
    assert_eq!(state.settings.persistence, SettingsPersistenceState::Idle);
    assert!(
        state
            .errors
            .iter()
            .any(|error| error.code == "settings_persist_failed")
    );
}
