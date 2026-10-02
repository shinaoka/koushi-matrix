use koushi_state::{
    AccountSettingsValues, SearchCrawlerSpeed, SettingsPatch, SettingsPatchScope, SettingsValues,
    ThemePreference,
};

#[test]
fn scoped_patches_preserve_fields_owned_by_the_other_settings_store() {
    let mut values = SettingsValues::default();
    values.notifications.desktop_notifications = false;
    values.notifications.send_read_receipts = false;
    values.display.url_previews_enabled = false;
    values.search_crawler.include_filenames = false;

    let mut notifications = values.notifications.clone();
    notifications.sound = false;
    notifications.badges = false;
    notifications.desktop_notifications = true;
    notifications.send_read_receipts = true;
    let app_patch = SettingsPatch {
        scope: Some(SettingsPatchScope::App),
        notifications: Some(notifications),
        ..SettingsPatch::default()
    };
    assert!(!app_patch.affects_account_settings());
    values.apply_patch(app_patch);
    assert!(!values.notifications.sound);
    assert!(!values.notifications.badges);
    assert!(!values.notifications.desktop_notifications);
    assert!(!values.notifications.send_read_receipts);

    let mut notifications = values.notifications.clone();
    notifications.sound = true;
    notifications.badges = true;
    notifications.desktop_notifications = true;
    notifications.send_read_receipts = true;
    let account_patch = SettingsPatch {
        scope: Some(SettingsPatchScope::Account),
        notifications: Some(notifications),
        ..SettingsPatch::default()
    };
    assert!(account_patch.affects_account_settings());
    values.apply_patch(account_patch);
    assert!(!values.notifications.sound);
    assert!(!values.notifications.badges);
    assert!(values.notifications.desktop_notifications);
    assert!(values.notifications.send_read_receipts);
}

#[test]
fn account_settings_load_fallback_disables_privacy_sensitive_features() {
    let settings = AccountSettingsValues::privacy_safe_fallback();
    assert!(!settings.notifications.desktop_notifications);
    assert!(!settings.notifications.message_previews);
    assert!(!settings.notifications.send_read_receipts);
    assert!(!settings.notifications.send_typing_notifications);
    assert!(!settings.display.url_previews_enabled);
    assert!(!settings.display.encrypted_url_previews_enabled);
    assert!(!settings.search_crawler.include_media_captions);
    assert!(!settings.search_crawler.include_filenames);
}

#[test]
fn app_and_account_settings_round_trip_without_overwriting_each_other() {
    let mut values = SettingsValues::default();
    let mut account = AccountSettingsValues::default();
    account.notifications.desktop_notifications = false;
    account.notifications.send_read_receipts = false;
    account.display.url_previews_enabled = false;
    account
        .sidebar
        .scope_preferences
        .insert("!private-space:example.org".to_owned(), Default::default());
    account.recent_emojis = vec!["🙂".to_owned()];
    account.search_crawler.include_filenames = false;
    values.apply_account_settings(&account);

    let mut app = values.app_settings();
    app.appearance.theme = ThemePreference::Dark;
    app.search_crawler_speed = SearchCrawlerSpeed::Fast;
    app.notifications.sound = false;
    values.apply_app_settings(&app);

    assert_eq!(values.app_settings(), app);
    assert!(values.account_settings() == account);
    assert!(
        !serde_json::to_string(&account)
            .expect("serialize account settings")
            .contains("appearance")
    );
}
