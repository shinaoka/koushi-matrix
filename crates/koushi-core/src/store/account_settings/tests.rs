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

#[test]
fn notification_migration_seeds_only_sound_and_badges_once() {
    use super::{ACCOUNT_SETTINGS_FILE_MAGIC, AccountSettingsFile};
    let data = tempdir().expect("data");
    let credentials = tempdir().expect("credentials");
    let key_id = make_key_id();
    let store = file_store_actor(&data, &credentials);
    let mut values = AccountSettingsValues::default();
    values.notifications.message_previews = true;
    values.notifications.desktop_notifications = false;
    let old_file = AccountSettingsFile {
        version: 1,
        legacy_migration_complete: true,
        values,
    };
    let secret = store
        .load_or_create_unlock_secret(&key_id)
        .expect("unlock secret");
    let payload = koushi_store::encrypt_envelope(
        ACCOUNT_SETTINGS_FILE_MAGIC,
        secret.derive_account_settings_key().as_bytes(),
        &serde_json::to_vec(&old_file).expect("old account json"),
        usize::MAX,
    )
    .expect("old encrypted account");
    let path = store.account_settings_file(&key_id);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("create settings");
    std::fs::write(&path, payload).expect("write old account");
    store
        .migrate_account_notifications(&key_id, Some((false, false)))
        .expect("migrate");
    let mut migrated = store.load_account_settings(&key_id).expect("load migrated");
    assert!(!migrated.notifications.sound && !migrated.notifications.badges);
    assert!(migrated.notifications.message_previews);
    assert!(!migrated.notifications.desktop_notifications);
    assert!(migrated.delivery.sinks.is_empty());
    migrated.notifications.sound = true;
    store
        .save_account_settings(&key_id, &migrated)
        .expect("account edit");
    store
        .migrate_account_notifications(&key_id, Some((false, false)))
        .expect("retry");
    assert!(
        store
            .load_account_settings(&key_id)
            .expect("retry load")
            .notifications
            .sound
    );
}

/// #2110: the agent role is the owner's device-level setting for a user id. A
/// sign-out deletes the account's own store, so a role kept there would be gone
/// when the same user signs in again; kept with the device's settings it
/// survives both a restart (a fresh `SettingsStore` on the same directory) and
/// the account's removal.
#[test]
fn the_agent_role_survives_a_restart_and_a_sign_out_of_the_same_user() {
    use crate::settings::SettingsStore;
    use koushi_state::{
        AgentAccountPatch, AgentAccountRole, SettingsPatch, SettingsPatchScope, SettingsValues,
    };

    let data_dir = tempdir().expect("data dir");
    let credential_dir = tempdir().expect("credential dir");
    let key_id = make_key_id();
    let store = file_store_actor(&data_dir, &credential_dir);
    store
        .save_account_settings(&key_id, &AccountSettingsValues::default())
        .expect("the account has its own store");

    let settings = SettingsStore::new(data_dir.path());
    settings
        .save_patch(&SettingsPatch {
            scope: Some(SettingsPatchScope::App),
            agent_account: Some(AgentAccountPatch {
                user_id: "@agent:example.invalid".to_owned(),
                role: Some(AgentAccountRole::default()),
            }),
            ..SettingsPatch::default()
        })
        .expect("save the role");

    store
        .delete_account_credentials(&key_id)
        .expect("sign the account out");
    assert!(store.load_account_settings(&key_id).unwrap() == AccountSettingsValues::default());

    let restarted: SettingsValues = SettingsStore::new(data_dir.path())
        .load()
        .expect("load after a restart");
    assert_eq!(
        restarted.agent_accounts.role_for("@agent:example.invalid"),
        Some(AgentAccountRole::default())
    );
    assert_eq!(
        restarted
            .agent_accounts
            .role_for("@someone-else:example.invalid"),
        None
    );

    // Unmarking removes only that account.
    settings
        .save_patch(&SettingsPatch {
            scope: Some(SettingsPatchScope::App),
            agent_account: Some(AgentAccountPatch {
                user_id: "@agent:example.invalid".to_owned(),
                role: None,
            }),
            ..SettingsPatch::default()
        })
        .expect("clear the role");
    assert!(
        SettingsStore::new(data_dir.path())
            .load()
            .unwrap()
            .agent_accounts
            .accounts
            .is_empty()
    );
}

/// #2130: `hide_tab` was an option of the agent role in #2121. A stored one is
/// ignored on load (the tab is visible) and gone from the file after a save.
#[test]
fn a_stored_hide_tab_is_ignored_and_dropped_on_the_next_save() {
    use crate::settings::SettingsStore;
    use koushi_state::{AgentAccountRole, SettingsPatch, SettingsPatchScope};

    let data_dir = tempdir().expect("data dir");
    let path = data_dir.path().join("settings").join("settings.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        r#"{"schema_version":4,"agent_accounts":{"accounts":{"@agent:example.invalid":{"hide_tab":true},"@shown:example.invalid":{"hide_tab":false}}}}"#,
    )
    .unwrap();

    let settings = SettingsStore::new(data_dir.path());
    let loaded = settings.load().expect("load");
    assert_eq!(
        loaded.agent_accounts.role_for("@agent:example.invalid"),
        Some(AgentAccountRole::default())
    );
    assert_eq!(
        loaded.agent_accounts.role_for("@shown:example.invalid"),
        Some(AgentAccountRole::default())
    );
    settings
        .save_patch(&SettingsPatch {
            scope: Some(SettingsPatchScope::App),
            agent_account: Some(koushi_state::AgentAccountPatch {
                user_id: "@other:example.invalid".to_owned(),
                role: Some(AgentAccountRole::default()),
            }),
            ..SettingsPatch::default()
        })
        .expect("save");
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(!saved.contains("hide_tab"), "{saved}");
    assert!(saved.contains("@agent:example.invalid"), "{saved}");
}
