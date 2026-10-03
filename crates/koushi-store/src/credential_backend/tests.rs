use super::*;
use std::sync::Arc;
use tempfile::tempdir;

fn make_key_id() -> SessionKeyId {
    SessionKeyId {
        homeserver: "https://test.example.com".to_owned(),
        user_id: "@alice:test.example.com".to_owned(),
        device_id: "DEVICE1".to_owned(),
    }
}

#[test]
fn store_diagnostic_producer_records_typed_outcome_without_environment_switch() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    record_file_credential_store_active();
    let record = koushi_diagnostics::test_support::detail_snapshot()
        .records
        .into_iter()
        .rev()
        .find(|record| {
            record.event.source == "core.store" && record.event.stage == "credential_store"
        })
        .expect("store producer should record");
    assert!(
        record
            .event
            .fields
            .iter()
            .any(|field| field.key == "outcome")
    );
}

#[test]
fn file_credential_store_round_trip() {
    let dir = tempdir().expect("tempdir");
    let store = FileCredentialStore::new(dir.path());
    let key_id = make_key_id();

    // Not found initially.
    let result = store.load(&key_id);
    assert!(koushi_key::is_missing_credential_error(
        &result.unwrap_err()
    ));

    // Save and reload.
    let secret = LocalUnlockSecret::generate();
    store.save(&key_id, &secret).expect("save");
    let loaded = store.load(&key_id).expect("load");

    // Keys derived from both secrets must match.
    let key1 = secret.derive_sdk_store_key();
    let key2 = loaded.derive_sdk_store_key();
    assert_eq!(key1.as_bytes(), key2.as_bytes());
}

#[test]
fn os_keychain_does_not_read_legacy_matrix_desktop_service() {
    let data_dir = tempdir().expect("tempdir");
    let backend = koushi_key::InMemoryCredentialBackend::default();
    let backend_dyn: Arc<dyn koushi_key::CredentialBackend> = Arc::new(backend);
    let store = OsCredentialStore::with_backend(data_dir.path(), backend_dyn.clone());
    let key_id = make_key_id();
    let secret = LocalUnlockSecret::generate();

    let legacy_probe =
        koushi_key::CredentialStore::with_backend("matrix-desktop", backend_dyn.clone());
    legacy_probe
        .save(&key_id, &secret)
        .expect("seed legacy unlock secret");

    let error = store.load(&key_id).expect_err("legacy service is not read");
    assert!(
        koushi_key::is_missing_credential_error(&error),
        "legacy matrix-desktop credentials must not be migrated"
    );
}
