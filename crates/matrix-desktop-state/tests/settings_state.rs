use matrix_desktop_state::{
    AppAction, AppEffect, AppState, AppearanceSettings, ComposerSendShortcut, EmojiPreference,
    FontPreference, KeyboardSettings, LocaleSettings, SettingsPatch, SettingsPersistenceState,
    SettingsValues, TextDirectionPreference, ThemePreference, UiEvent, reduce,
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
