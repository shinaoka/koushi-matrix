use super::super::test_support::{file_store_actor, make_key_id};
use koushi_state::AccountSettingsValues;
use tempfile::tempdir;

#[test]
fn account_settings_are_encrypted_and_legacy_seed_is_retry_safe() {
    let data_dir = tempdir().expect("data dir");
    let credential_dir = tempdir().expect("credential dir");
    let key_id = make_key_id();
    let store = file_store_actor(&data_dir, &credential_dir);

    let mut legacy = AccountSettingsValues::default();
    legacy.notifications.desktop_notifications = false;
    legacy.notifications.message_previews = true;
    legacy.display.encrypted_url_previews_enabled = true;
    legacy.sidebar.scope_preferences.insert(
        "!private-space:test.example.com".to_owned(),
        Default::default(),
    );
    store
        .seed_account_settings_from_legacy(&key_id, &legacy)
        .expect("seed legacy settings");

    let path = store.account_settings_file(&key_id);
    assert!(path.ends_with("settings/account-settings.v1.enc"));
    let encrypted = std::fs::read(&path).expect("read encrypted settings");
    assert!(
        !encrypted
            .windows("!private-space:test.example.com".len())
            .any(|window| window == b"!private-space:test.example.com")
    );
    assert!(
        store
            .load_account_settings(&key_id)
            .expect("load account settings")
            == legacy
    );

    let mut changed = AccountSettingsValues::default();
    changed.notifications.desktop_notifications = true;
    store
        .save_account_settings(&key_id, &changed)
        .expect("save updated settings");
    store
        .seed_account_settings_from_legacy(&key_id, &legacy)
        .expect("retry migration");
    assert!(
        store
            .load_account_settings(&key_id)
            .expect("load migrated settings")
            == changed
    );

    let mut corrupted = encrypted;
    let last = corrupted.last_mut().expect("encrypted file bytes");
    *last ^= 1;
    std::fs::write(path, corrupted).expect("corrupt account settings");
    assert!(store.load_account_settings(&key_id).is_err());
}
