use super::StoreActor;
use super::test_support::{file_store_actor, make_key_id};
use koushi_key::{LocalUnlockSecret, SessionKeyIdCredentialNames};
use koushi_protocol::SessionKeyId;
use koushi_store::{CREDENTIAL_STORE_SERVICE_NAME, CredentialStoreBackend};
use std::sync::Arc;
use tempfile::tempdir;

#[test]
fn store_actor_generates_config_with_file_backend() {
    let data_dir = tempdir().expect("tempdir");
    let cred_dir = tempdir().expect("tempdir");
    let key_id = make_key_id();

    let actor = file_store_actor(&data_dir, &cred_dir);

    let config = actor
        .account_store_config(&key_id)
        .expect("store config should succeed");

    // Path is inside our data dir.
    assert!(config.store_config.path().starts_with(data_dir.path()));
    assert!(
        config
            .store_config
            .cache_path()
            .expect("cache path should be configured")
            .starts_with(data_dir.path())
    );

    // Calling again yields a consistent store path (same key_id).
    let config2 = actor.account_store_config(&key_id).expect("second call");
    assert_eq!(config.store_config.path(), config2.store_config.path());
    assert_eq!(
        config.store_config.cache_path(),
        config2.store_config.cache_path()
    );
}
#[test]
fn account_store_and_search_config_trace_unlock_secret_source() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let data_dir = tempdir().expect("tempdir");
    let cred_dir = tempdir().expect("tempdir");
    let key_id = make_key_id();
    let actor = file_store_actor(&data_dir, &cred_dir);
    let diagnostic_start = koushi_diagnostics::test_support::detail_cursor();

    actor
        .account_store_config(&key_id)
        .expect("first store config creates the unlock secret");
    actor
        .account_search_index_config(&key_id)
        .expect("search config reuses the unlock secret");
    actor
        .account_store_config(&key_id)
        .expect("second store config reuses the unlock secret");

    let records = koushi_diagnostics::test_support::detail_records_since(diagnostic_start);
    let unlock_events = records
        .iter()
        .filter(|record| {
            record.event.source == "core.store" && record.event.stage == "local_unlock_secret"
        })
        .map(|record| koushi_diagnostics::format_event(&record.event))
        .collect::<Vec<_>>();
    assert!(
        unlock_events
            .iter()
            .any(|line| line.contains("purpose=account_store") && line.contains("outcome=created")),
        "first account store config must say it created the account-local unlock secret"
    );
    assert!(
        unlock_events
            .iter()
            .any(|line| line.contains("purpose=search_index") && line.contains("outcome=loaded")),
        "search index config must say it loaded the existing account-local unlock secret"
    );
    assert!(
        !unlock_events
            .iter()
            .any(|line| line.contains("@alice") || line.contains("DEVICE1")),
        "unlock diagnostics must not leak account identifiers"
    );
}
#[test]
fn delete_account_credentials_does_not_panic_when_absent() {
    let data_dir = tempdir().expect("tempdir");
    let cred_dir = tempdir().expect("tempdir");
    let key_id = make_key_id();

    let actor = file_store_actor(&data_dir, &cred_dir);

    // Should not panic even when credentials don't exist.
    actor
        .delete_account_credentials(&key_id)
        .expect("account credentials delete");
}
#[test]
fn store_actor_probe_maps_credential_backend_health_without_raw_errors() {
    let data_dir = tempdir().expect("tempdir");
    let backend = koushi_key::InMemoryCredentialBackend::default();
    let actor = StoreActor::with_backend(
        CredentialStoreBackend::InMemory(koushi_key::CredentialStore::with_backend(
            "koushi-desktop-test",
            backend.clone(),
        )),
        data_dir.path(),
    );
    let key_id = make_key_id();

    assert_eq!(
        actor.probe_local_encryption_health(&key_id),
        koushi_state::LocalEncryptionHealth::MissingCredential
    );

    let secret = LocalUnlockSecret::generate();
    actor
        .credential_backend()
        .save(&key_id, &secret)
        .expect("save synthetic unlock secret");
    assert_eq!(
        actor.probe_local_encryption_health(&key_id),
        koushi_state::LocalEncryptionHealth::Healthy
    );

    backend.set_error(koushi_key::CredentialBackendErrorKind::LockedOrInaccessible);
    assert_eq!(
        actor.probe_local_encryption_health(&key_id),
        koushi_state::LocalEncryptionHealth::LockedOrInaccessible
    );
}
#[test]
fn os_keychain_service_name_is_product_branded() {
    assert_eq!(CREDENTIAL_STORE_SERVICE_NAME, "koushi-desktop");
}
#[test]
fn migrated_credential_vault_reads_keychain_once() {
    let data_dir = tempdir().expect("tempdir");
    let backend = koushi_key::InMemoryCredentialBackend::default();
    let master_key_store =
        koushi_key::CredentialStore::with_backend(CREDENTIAL_STORE_SERVICE_NAME, backend.clone());
    let master_key = koushi_key::CredentialVaultMasterKey::generate();
    master_key_store
        .save_vault_master_key(&master_key)
        .expect("seed master key");
    let alice = make_key_id();
    let bob = SessionKeyId {
        homeserver: "https://test.example.com".to_owned(),
        user_id: "@bob:test.example.com".to_owned(),
        device_id: "DEVICE2".to_owned(),
    };
    let mut vault = koushi_store::CredentialVaultData::default();
    vault.set_last_session(Some(alice.clone()));
    vault.upsert_matrix_session(alice.clone(), "alice-session");
    vault.remember_session(alice.clone());
    vault.upsert_local_unlock_secret(
        alice.clone(),
        LocalUnlockSecret::generate().to_storage_string().as_str(),
    );
    vault.upsert_matrix_session(bob.clone(), "bob-session");
    vault.remember_session(bob.clone());
    vault.upsert_local_unlock_secret(
        bob.clone(),
        LocalUnlockSecret::generate().to_storage_string().as_str(),
    );
    koushi_store::CredentialVaultFile::new(
        data_dir
            .path()
            .join("credentials")
            .join("credentials.v1.enc"),
    )
    .store(&master_key, &vault)
    .expect("seed credential vault");

    let actor = StoreActor::with_os_backend(data_dir.path(), Arc::new(backend.clone()));
    let credentials = actor.credential_backend();
    assert_eq!(
        credentials.load_last_session().expect("last session"),
        Some(alice.clone())
    );
    assert_eq!(
        credentials
            .load_saved_sessions()
            .expect("saved sessions")
            .sessions(),
        &[alice.clone(), bob.clone()]
    );
    assert_eq!(
        credentials
            .load_matrix_session(&alice)
            .expect("alice session")
            .as_str(),
        "alice-session"
    );
    actor.account_store_config(&alice).expect("alice store");
    actor
        .account_search_index_config(&alice)
        .expect("alice search");
    actor
        .load_composer_drafts(&alice)
        .expect("alice composer drafts");
    actor
        .load_scheduled_sends(&alice)
        .expect("alice scheduled sends");
    actor.load_navigation(&alice).expect("alice navigation");
    actor
        .load_room_preferences(&alice)
        .expect("alice room preferences");
    actor
        .load_read_state_outbox(&alice)
        .expect("alice read state outbox");
    assert_eq!(
        credentials
            .load_matrix_session(&bob)
            .expect("bob session")
            .as_str(),
        "bob-session"
    );
    actor.account_store_config(&bob).expect("bob store");

    assert_eq!(backend.get_password_count(), 1);
}
#[test]
fn legacy_credentials_migrate_without_losing_session() {
    let data_dir = tempdir().expect("tempdir");
    let backend = koushi_key::InMemoryCredentialBackend::default();
    let key_id = make_key_id();
    let first_secret = seed_legacy_credentials(&backend, &key_id);
    let second_key_id = SessionKeyId {
        homeserver: "https://test.example.com".to_owned(),
        user_id: "@bob:test.example.com".to_owned(),
        device_id: "DEVICE2".to_owned(),
    };
    let second_secret = seed_legacy_credentials(&backend, &second_key_id);
    koushi_key::CredentialStore::with_backend(CREDENTIAL_STORE_SERVICE_NAME, backend.clone())
        .save_last_session(&key_id)
        .expect("restore first account as last session");

    let actor = StoreActor::with_os_backend(data_dir.path(), Arc::new(backend.clone()));
    let credentials = actor.credential_backend();
    assert_eq!(
        credentials.load_last_session().expect("migrated pointer"),
        Some(key_id.clone())
    );
    assert_eq!(
        credentials
            .load_matrix_session(&key_id)
            .expect("migrated session")
            .as_str(),
        "legacy-session"
    );
    let migrated_first_secret = credentials
        .load(&key_id)
        .expect("migrated unlock secret")
        .to_storage_string();
    let expected_first_secret = first_secret.to_storage_string();
    assert_eq!(
        migrated_first_secret.as_str(),
        expected_first_secret.as_str()
    );
    assert_eq!(
        credentials
            .load_matrix_session(&second_key_id)
            .expect("second migrated session")
            .as_str(),
        "legacy-session"
    );
    let migrated_second_secret = credentials
        .load(&second_key_id)
        .expect("second migrated unlock secret")
        .to_storage_string();
    let expected_second_secret = second_secret.to_storage_string();
    assert_eq!(
        migrated_second_secret.as_str(),
        expected_second_secret.as_str()
    );
    assert!(
        data_dir
            .path()
            .join("credentials")
            .join("credentials.v1.enc")
            .is_file()
    );
    assert!(backend.contains_entry(
        CREDENTIAL_STORE_SERVICE_NAME,
        koushi_key::credential_vault_key_account_name()
    ));
    assert!(!backend.contains_entry(
        CREDENTIAL_STORE_SERVICE_NAME,
        koushi_key::last_session_account_name()
    ));
    assert!(!backend.contains_entry(
        CREDENTIAL_STORE_SERVICE_NAME,
        &key_id.matrix_session_account_name()
    ));
    assert!(!backend.contains_entry(
        CREDENTIAL_STORE_SERVICE_NAME,
        &key_id.local_unlock_account_name()
    ));
    assert!(!backend.contains_entry(
        CREDENTIAL_STORE_SERVICE_NAME,
        &second_key_id.matrix_session_account_name()
    ));
    assert!(!backend.contains_entry(
        CREDENTIAL_STORE_SERVICE_NAME,
        &second_key_id.local_unlock_account_name()
    ));
}
fn seed_legacy_credentials(
    backend: &koushi_key::InMemoryCredentialBackend,
    key_id: &SessionKeyId,
) -> LocalUnlockSecret {
    let store =
        koushi_key::CredentialStore::with_backend(CREDENTIAL_STORE_SERVICE_NAME, backend.clone());
    let secret = LocalUnlockSecret::generate();
    store.save(key_id, &secret).expect("seed legacy unlock");
    store
        .save_matrix_session(
            key_id,
            &koushi_key::StoredMatrixSession::new("legacy-session"),
        )
        .expect("seed legacy session");
    store
        .remember_saved_session(key_id)
        .expect("seed legacy index");
    store
        .save_last_session(key_id)
        .expect("seed legacy pointer");
    secret
}
#[test]
fn legacy_credentials_missing_entry_preserves_legacy_index() {
    let data_dir = tempdir().expect("tempdir");
    let backend = koushi_key::InMemoryCredentialBackend::default();
    let complete_key_id = make_key_id();
    let _complete_secret = seed_legacy_credentials(&backend, &complete_key_id);
    let key_id = SessionKeyId {
        homeserver: "https://test.example.com".to_owned(),
        user_id: "@incomplete:test.example.com".to_owned(),
        device_id: "INCOMPLETE".to_owned(),
    };
    let legacy =
        koushi_key::CredentialStore::with_backend(CREDENTIAL_STORE_SERVICE_NAME, backend.clone());
    legacy
        .remember_saved_session(&key_id)
        .expect("seed incomplete legacy index");

    let actor = StoreActor::with_os_backend(data_dir.path(), Arc::new(backend.clone()));
    assert!(actor.credential_backend().load_saved_sessions().is_err());
    assert!(backend.contains_entry(
        CREDENTIAL_STORE_SERVICE_NAME,
        koushi_key::saved_sessions_account_name()
    ));
    assert!(backend.contains_entry(
        CREDENTIAL_STORE_SERVICE_NAME,
        &complete_key_id.matrix_session_account_name()
    ));
    assert!(backend.contains_entry(
        CREDENTIAL_STORE_SERVICE_NAME,
        &complete_key_id.local_unlock_account_name()
    ));
    assert!(!backend.contains_entry(
        CREDENTIAL_STORE_SERVICE_NAME,
        koushi_key::credential_vault_key_account_name()
    ));
    assert!(
        !data_dir
            .path()
            .join("credentials")
            .join("credentials.v1.enc")
            .exists()
    );
}
#[test]
fn legacy_credentials_resume_with_existing_master_key() {
    let data_dir = tempdir().expect("tempdir");
    let backend = koushi_key::InMemoryCredentialBackend::default();
    let key_id = make_key_id();
    let _ = seed_legacy_credentials(&backend, &key_id);
    let key_store =
        koushi_key::CredentialStore::with_backend(CREDENTIAL_STORE_SERVICE_NAME, backend.clone());
    key_store
        .save_vault_master_key(&koushi_key::CredentialVaultMasterKey::generate())
        .expect("seed orphan master key");

    let actor = StoreActor::with_os_backend(data_dir.path(), Arc::new(backend));
    assert_eq!(
        actor
            .credential_backend()
            .load_last_session()
            .expect("resumed migration"),
        Some(key_id)
    );
}
#[test]
fn legacy_credentials_delete_failure_keeps_vault_authoritative() {
    let data_dir = tempdir().expect("tempdir");
    let backend = koushi_key::InMemoryCredentialBackend::default();
    let key_id = make_key_id();
    let _ = seed_legacy_credentials(&backend, &key_id);
    backend.set_delete_error(koushi_key::CredentialBackendErrorKind::Unavailable);

    let actor = StoreActor::with_os_backend(data_dir.path(), Arc::new(backend.clone()));
    assert_eq!(
        actor
            .credential_backend()
            .load_matrix_session(&key_id)
            .expect("new vault remains authoritative")
            .as_str(),
        "legacy-session"
    );
    assert!(backend.contains_entry(
        CREDENTIAL_STORE_SERVICE_NAME,
        &key_id.matrix_session_account_name()
    ));
    assert!(
        data_dir
            .path()
            .join("credentials")
            .join("credentials.v1.enc")
            .is_file()
    );

    drop(actor);
    backend.clear_delete_error();
    let restarted = StoreActor::with_os_backend(data_dir.path(), Arc::new(backend.clone()));
    assert_eq!(
        restarted
            .credential_backend()
            .load_matrix_session(&key_id)
            .expect("vault restores while retrying cleanup")
            .as_str(),
        "legacy-session"
    );
    assert!(!backend.contains_entry(
        CREDENTIAL_STORE_SERVICE_NAME,
        &key_id.matrix_session_account_name()
    ));
    assert!(!backend.contains_entry(
        CREDENTIAL_STORE_SERVICE_NAME,
        &key_id.local_unlock_account_name()
    ));
    assert!(!backend.contains_entry(
        CREDENTIAL_STORE_SERVICE_NAME,
        koushi_key::saved_sessions_account_name()
    ));
}
#[test]
fn credential_vault_concurrent_initialization_reads_keychain_once() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let diagnostic_start = koushi_diagnostics::test_support::detail_cursor();
    let data_dir = tempdir().expect("tempdir");
    let backend = koushi_key::InMemoryCredentialBackend::default();
    let key_store =
        koushi_key::CredentialStore::with_backend(CREDENTIAL_STORE_SERVICE_NAME, backend.clone());
    let master_key = koushi_key::CredentialVaultMasterKey::generate();
    key_store
        .save_vault_master_key(&master_key)
        .expect("seed master key");
    koushi_store::CredentialVaultFile::new(
        data_dir
            .path()
            .join("credentials")
            .join("credentials.v1.enc"),
    )
    .store(&master_key, &koushi_store::CredentialVaultData::default())
    .expect("seed vault");
    let actor = StoreActor::with_os_backend(data_dir.path(), Arc::new(backend.clone()));
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let threads = (0..8)
        .map(|_| {
            let actor = actor.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                actor
                    .credential_backend()
                    .load_saved_sessions()
                    .expect("concurrent vault read");
            })
        })
        .collect::<Vec<_>>();
    for thread in threads {
        thread.join().expect("join reader");
    }

    assert_eq!(backend.get_password_count(), 1);
    let outcomes = koushi_diagnostics::test_support::detail_records_since(diagnostic_start)
        .into_iter()
        .filter(|record| {
            record.event.source == "core.store" && record.event.stage == "credential_vault_access"
        })
        .flat_map(|record| record.event.fields)
        .filter_map(|field| match field.value {
            koushi_diagnostics::DiagnosticValue::Token(outcome) if field.key == "outcome" => {
                Some(outcome)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(outcomes.contains(&"keychain_read_started"));
    assert!(outcomes.contains(&"keychain_read_succeeded"));
    assert!(outcomes.contains(&"memory_cache_reused"));
}
#[test]
fn credential_vault_initialization_retries_after_transient_keychain_failure() {
    let data_dir = tempdir().expect("tempdir");
    let backend = koushi_key::InMemoryCredentialBackend::default();
    backend.set_error(koushi_key::CredentialBackendErrorKind::LockedOrInaccessible);
    let actor = StoreActor::with_os_backend(data_dir.path(), Arc::new(backend.clone()));

    actor
        .credential_backend()
        .load_saved_sessions()
        .expect_err("locked keychain");
    backend.clear_error();

    assert!(
        actor
            .credential_backend()
            .load_saved_sessions()
            .expect("retry after unlocking keychain")
            .sessions()
            .is_empty()
    );
}
#[test]
fn fresh_saved_session_list_does_not_create_key_or_vault() {
    let data_dir = tempdir().expect("tempdir");
    let backend = koushi_key::InMemoryCredentialBackend::default();
    let actor = StoreActor::with_os_backend(data_dir.path(), Arc::new(backend.clone()));

    assert!(
        actor
            .credential_backend()
            .load_saved_sessions()
            .expect("empty saved sessions")
            .sessions()
            .is_empty()
    );
    assert!(!backend.contains_entry(
        CREDENTIAL_STORE_SERVICE_NAME,
        koushi_key::credential_vault_key_account_name()
    ));
    assert!(
        !data_dir
            .path()
            .join("credentials")
            .join("credentials.v1.enc")
            .exists()
    );
}
#[test]
fn credential_vault_corrupt_file_is_not_overwritten() {
    let data_dir = tempdir().expect("tempdir");
    let backend = koushi_key::InMemoryCredentialBackend::default();
    let key_store =
        koushi_key::CredentialStore::with_backend(CREDENTIAL_STORE_SERVICE_NAME, backend.clone());
    key_store
        .save_vault_master_key(&koushi_key::CredentialVaultMasterKey::generate())
        .expect("seed master key");
    let path = data_dir
        .path()
        .join("credentials")
        .join("credentials.v1.enc");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
    let corrupt = b"not-a-credential-vault".to_vec();
    std::fs::write(&path, &corrupt).expect("seed corrupt vault");

    let actor = StoreActor::with_os_backend(data_dir.path(), Arc::new(backend));
    assert!(actor.credential_backend().load_saved_sessions().is_err());
    assert_eq!(std::fs::read(path).expect("read corrupt vault"), corrupt);
}
