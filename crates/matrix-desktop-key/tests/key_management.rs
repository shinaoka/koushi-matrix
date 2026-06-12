use base64::{Engine as _, engine::general_purpose::STANDARD};
use matrix_desktop_key::{
    CredentialStore, LocalSecretError, LocalUnlockSecret, SavedSessionIndex, SessionKeyId,
    StoredMatrixSession, is_missing_credential_error, map_delete_result,
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
        user_id: "@user-a:example.invalid".into(),
        device_id: "DEVICE123".into(),
    };
    let alternate_split = SessionKeyId {
        homeserver: "https://matrix.example".into(),
        user_id: "with-pipe|@user-a:example.invalid".into(),
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
fn matrix_session_account_name_is_separate_from_local_unlock_secret() {
    let id = SessionKeyId {
        homeserver: "https://matrix.example".into(),
        user_id: "@user-a:example.invalid".into(),
        device_id: "DEVICE123".into(),
    };

    assert!(
        id.matrix_session_account_name()
            .starts_with("matrix-session|v1|")
    );
    assert_ne!(id.account_name(), id.matrix_session_account_name());
}

#[test]
fn last_session_pointer_account_name_is_global_and_versioned() {
    assert_eq!(
        CredentialStore::last_session_account_name(),
        "matrix-desktop:last-session:v1"
    );
}

#[test]
fn saved_session_index_tracks_unique_sessions_and_redacts_debug() {
    let alpha = SessionKeyId {
        homeserver: "https://matrix.example".into(),
        user_id: "@user-a:example.invalid".into(),
        device_id: "DEVICE-A".into(),
    };
    let beta = SessionKeyId {
        homeserver: "https://matrix.example".into(),
        user_id: "@user-b:example.invalid".into(),
        device_id: "DEVICE-B".into(),
    };

    let mut index = SavedSessionIndex::new();
    index.upsert(alpha.clone());
    index.upsert(beta.clone());
    index.upsert(alpha.clone());

    assert_eq!(index.sessions(), &[alpha.clone(), beta.clone()]);

    let json = index.to_json().unwrap();
    let restored = SavedSessionIndex::from_json(&json).unwrap();
    assert_eq!(restored.sessions(), &[alpha.clone(), beta.clone()]);
    assert_eq!(format!("{restored:?}"), "SavedSessionIndex(..)");
    assert!(!format!("{restored:?}").contains("@user-a:example.invalid"));

    index.remove(&alpha);
    assert_eq!(index.sessions(), &[beta]);
}

#[test]
fn saved_session_index_account_name_is_global_and_versioned() {
    assert_eq!(
        CredentialStore::saved_sessions_account_name(),
        "matrix-desktop:saved-sessions:v1"
    );
}

#[test]
fn last_session_pointer_round_trip_redacts_debug_output() {
    let id = SessionKeyId {
        homeserver: "https://matrix.example".into(),
        user_id: "@user-a:example.invalid".into(),
        device_id: "DEVICE123".into(),
    };
    let pointer = matrix_desktop_key::LastSessionPointer::new(id.clone());
    let json = pointer.to_json().unwrap();
    let restored = matrix_desktop_key::LastSessionPointer::from_json(&json).unwrap();

    assert_eq!(restored.session_key_id(), &id);
    assert_eq!(format!("{pointer:?}"), "LastSessionPointer(..)");
    assert!(!json.contains("access_token"));
}

#[test]
fn stored_matrix_session_redacts_debug_output() {
    let payload = r#"{"access_token":"fixture-access-token","device_id":"DEVICE123"}"#;
    let stored = StoredMatrixSession::new(payload);

    assert_eq!(stored.as_str(), payload);
    assert_eq!(format!("{stored:?}"), "StoredMatrixSession(..)");
    assert!(!format!("{stored:?}").contains("fixture-access-token"));
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
fn missing_credential_error_is_classified_for_fail_closed_store_restore() {
    assert!(is_missing_credential_error(
        &LocalSecretError::CredentialStore(keyring::Error::NoEntry)
    ));
}

#[test]
#[ignore = "uses the live operating-system credential store"]
fn credential_store_last_session_pointer_round_trip() {
    let store = CredentialStore::new("matrix-desktop-key-test");
    let id = SessionKeyId {
        homeserver: "https://matrix.example".into(),
        user_id: "@user-a:example.invalid".into(),
        device_id: format!("TEST-POINTER-{}", std::process::id()),
    };

    let _ = store.delete_last_session();
    let result = (|| -> Result<_, LocalSecretError> {
        store.save_last_session(&id)?;
        Ok(store.load_last_session()?)
    })();
    store.delete_last_session().unwrap();

    assert_eq!(result.unwrap(), Some(id));
}

#[test]
#[ignore = "uses the live operating-system credential store"]
fn credential_store_matrix_session_round_trip() {
    let store = CredentialStore::new("matrix-desktop-key-test");
    let id = SessionKeyId {
        homeserver: "https://matrix.example".into(),
        user_id: "@user-a:example.invalid".into(),
        device_id: format!("TEST-SESSION-{}", std::process::id()),
    };
    let session = StoredMatrixSession::new(
        r#"{"access_token":"fixture-access-token","device_id":"DEVICE123"}"#,
    );

    let _ = store.delete_matrix_session(&id);
    let result = (|| -> Result<_, LocalSecretError> {
        store.save_matrix_session(&id, &session)?;
        Ok(store.load_matrix_session(&id)?)
    })();
    store.delete_matrix_session(&id).unwrap();

    let loaded = result.unwrap();
    assert_eq!(loaded.as_str(), session.as_str());
    assert!(!format!("{loaded:?}").contains("fixture-access-token"));
}

#[test]
#[ignore = "uses the live operating-system credential store"]
fn credential_store_round_trip() {
    let store = CredentialStore::new("matrix-desktop-key-test");
    let id = SessionKeyId {
        homeserver: "https://matrix.example".into(),
        user_id: "@user-a:example.invalid".into(),
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
