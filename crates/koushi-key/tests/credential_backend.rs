use koushi_key::{
    CredentialBackendErrorKind, CredentialStore, InMemoryCredentialBackend, LocalSecretError,
    LocalUnlockSecret, SessionKeyId, is_locked_or_inaccessible_error, is_missing_credential_error,
};

fn key_id() -> SessionKeyId {
    SessionKeyId {
        homeserver: "https://matrix.example".to_owned(),
        user_id: "@user-a:example.invalid".to_owned(),
        device_id: "DEVICE123".to_owned(),
    }
}

#[test]
fn credential_backend_fake_round_trips_one_local_unlock_secret() {
    let backend = InMemoryCredentialBackend::default();
    let store = CredentialStore::with_backend("koushi-desktop-test", backend);
    let secret = LocalUnlockSecret::generate();

    store.save(&key_id(), &secret).expect("save");
    let loaded = store.load(&key_id()).expect("load");

    assert_eq!(
        secret.derive_sdk_store_key().as_bytes(),
        loaded.derive_sdk_store_key().as_bytes()
    );
}

#[test]
fn credential_backend_fake_reports_missing_credential_without_raw_error_text() {
    let backend = InMemoryCredentialBackend::default();
    let store = CredentialStore::with_backend("koushi-desktop-test", backend);

    let error = store.load(&key_id()).expect_err("missing credential");

    assert!(is_missing_credential_error(&error));
    assert_eq!(
        format!("{error}"),
        "credential backend error: missing credential"
    );
    assert!(!format!("{error:?}").contains("@user-a:example.invalid"));
}

#[test]
fn credential_backend_fake_locked_state_maps_to_locked_or_inaccessible() {
    let backend = InMemoryCredentialBackend::default();
    backend.set_error(CredentialBackendErrorKind::LockedOrInaccessible);
    let store = CredentialStore::with_backend("koushi-desktop-test", backend);

    let error = store.load(&key_id()).expect_err("locked credential store");

    assert!(is_locked_or_inaccessible_error(&error));
    assert!(matches!(
        error,
        LocalSecretError::CredentialBackend(CredentialBackendErrorKind::LockedOrInaccessible)
    ));
    assert_eq!(
        format!("{error}"),
        "credential backend error: locked or inaccessible"
    );
}
