use base64::{Engine as _, engine::general_purpose::STANDARD};
use matrix_desktop_key::{
    CredentialStore, LocalSecretError, LocalUnlockSecret, SessionKeyId, map_delete_result,
};

fn secret_from_test_byte(byte: u8) -> LocalUnlockSecret {
    LocalUnlockSecret::from_storage_string(&STANDARD.encode([byte; 32])).unwrap()
}

#[test]
fn namespaced_derivations_are_distinct() {
    let secret = secret_from_test_byte(7);

    let sdk_store_key = secret.derive_sdk_store_key();
    let search_key = secret.derive_search_index_key();

    assert_eq!(sdk_store_key.as_bytes().len(), 32);
    assert_ne!(
        STANDARD.encode(sdk_store_key.as_bytes()),
        search_key.as_str()
    );
}

#[test]
fn derived_and_stored_secrets_have_redacted_debug() {
    let secret = secret_from_test_byte(7);
    let sdk_store_key = secret.derive_sdk_store_key();
    let search_key = secret.derive_search_index_key();
    let stored_secret = secret.to_storage_string();

    assert_eq!(format!("{sdk_store_key:?}"), "SdkStoreKey(..)");
    assert_eq!(format!("{search_key:?}"), "SearchIndexKey(..)");
    assert_eq!(format!("{stored_secret:?}"), "StoredLocalUnlockSecret(..)");
    assert!(!format!("{secret:?}").contains(stored_secret.as_str()));
}

#[test]
fn account_name_is_versioned_and_collision_safe() {
    let id = SessionKeyId {
        homeserver: "https://matrix.example|with-pipe".into(),
        user_id: "@alice:example.com".into(),
        device_id: "DEVICE123".into(),
    };
    let alternate_split = SessionKeyId {
        homeserver: "https://matrix.example".into(),
        user_id: "with-pipe|@alice:example.com".into(),
        device_id: "DEVICE123".into(),
    };

    assert_eq!(
        format!("{}|{}|{}", id.homeserver, id.user_id, id.device_id),
        format!(
            "{}|{}|{}",
            alternate_split.homeserver, alternate_split.user_id, alternate_split.device_id
        )
    );
    assert_eq!(id.account_name().split('|').count(), 4);
    assert!(id.account_name().starts_with("v1|"));
    assert_ne!(id.account_name(), alternate_split.account_name());
}

#[test]
fn storage_round_trip_preserves_derivations() {
    let original = secret_from_test_byte(9);

    let stored = original.to_storage_string();
    let restored = LocalUnlockSecret::from_storage_string(stored.as_str()).unwrap();

    assert_eq!(
        original.derive_sdk_store_key().as_bytes(),
        restored.derive_sdk_store_key().as_bytes()
    );
    assert_eq!(
        original.derive_search_index_key().as_str(),
        restored.derive_search_index_key().as_str()
    );
}

#[test]
fn stored_secret_rejects_invalid_length() {
    let stored = STANDARD.encode([1; 31]);

    let error = LocalUnlockSecret::from_storage_string(&stored).unwrap_err();

    assert!(matches!(
        error,
        LocalSecretError::InvalidSecretLength {
            expected: 32,
            actual: 31,
        }
    ));
}

#[test]
fn delete_missing_credential_is_success() {
    assert!(map_delete_result(Err(keyring::Error::NoEntry)).is_ok());
}

#[test]
#[ignore = "uses the live operating-system credential store"]
fn credential_store_round_trip() {
    let store = CredentialStore::new("matrix-desktop-key-test");
    let id = SessionKeyId {
        homeserver: "https://matrix.example".into(),
        user_id: "@alice:example.com".into(),
        device_id: format!("TEST-{}", std::process::id()),
    };
    let secret = LocalUnlockSecret::generate();

    let _ = store.delete(&id);
    let result = (|| -> Result<_, LocalSecretError> {
        store.save(&id, &secret)?;
        let loaded = store.load(&id)?;
        Ok((
            secret.derive_sdk_store_key(),
            loaded.derive_sdk_store_key(),
            secret.derive_search_index_key(),
            loaded.derive_search_index_key(),
        ))
    })();
    store.delete(&id).unwrap();

    let (expected_sdk, loaded_sdk, expected_search, loaded_search) = result.unwrap();
    assert_eq!(expected_sdk.as_bytes(), loaded_sdk.as_bytes());
    assert_eq!(expected_search.as_str(), loaded_search.as_str());
}
