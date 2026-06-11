use std::fmt;

use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use hkdf::Hkdf;
use keyring::Entry;
use sha2::Sha256;
use thiserror::Error;
use zeroize::Zeroizing;

const LOCAL_SECRET_LEN: usize = 32;
const SDK_STORE_INFO: &[u8] = b"matrix-desktop:sdk-store";
const SEARCH_INDEX_INFO: &[u8] = b"matrix-desktop:search-index";

#[derive(Debug, Error)]
pub enum LocalSecretError {
    #[error("credential store error: {0}")]
    CredentialStore(#[from] keyring::Error),
    #[error("key derivation failed")]
    Derivation,
    #[error("base64 decode error: {0}")]
    Base64Decode(#[from] base64::DecodeError),
    #[error("invalid secret length: expected {expected} bytes, got {actual}")]
    InvalidSecretLength { expected: usize, actual: usize },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionKeyId {
    pub homeserver: String,
    pub user_id: String,
    pub device_id: String,
}

impl SessionKeyId {
    pub fn account_name(&self) -> String {
        format!(
            "v1|{}|{}|{}",
            URL_SAFE_NO_PAD.encode(self.homeserver.as_bytes()),
            URL_SAFE_NO_PAD.encode(self.user_id.as_bytes()),
            URL_SAFE_NO_PAD.encode(self.device_id.as_bytes())
        )
    }
}

pub struct SdkStoreKey {
    key: Zeroizing<[u8; LOCAL_SECRET_LEN]>,
}

impl SdkStoreKey {
    pub fn as_bytes(&self) -> &[u8; LOCAL_SECRET_LEN] {
        &self.key
    }

    pub fn into_bytes(self) -> Zeroizing<[u8; LOCAL_SECRET_LEN]> {
        self.key
    }
}

impl fmt::Debug for SdkStoreKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SdkStoreKey(..)")
    }
}

pub struct SearchIndexKey {
    key: Zeroizing<String>,
}

impl SearchIndexKey {
    pub fn as_str(&self) -> &str {
        self.key.as_str()
    }

    pub fn into_string(self) -> Zeroizing<String> {
        self.key
    }
}

impl fmt::Debug for SearchIndexKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SearchIndexKey(..)")
    }
}

pub struct StoredLocalUnlockSecret {
    value: Zeroizing<String>,
}

impl StoredLocalUnlockSecret {
    pub fn as_str(&self) -> &str {
        self.value.as_str()
    }

    pub fn into_string(self) -> Zeroizing<String> {
        self.value
    }
}

impl fmt::Debug for StoredLocalUnlockSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StoredLocalUnlockSecret(..)")
    }
}

pub struct LocalUnlockSecret {
    secret: Zeroizing<[u8; LOCAL_SECRET_LEN]>,
}

impl fmt::Debug for LocalUnlockSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalUnlockSecret")
            .finish_non_exhaustive()
    }
}

impl LocalUnlockSecret {
    pub fn generate() -> Self {
        Self::from_zeroizing_bytes(Zeroizing::new(rand::random()))
    }

    fn from_zeroizing_bytes(secret: Zeroizing<[u8; LOCAL_SECRET_LEN]>) -> Self {
        Self { secret }
    }

    #[cfg(test)]
    fn from_test_bytes(bytes: [u8; LOCAL_SECRET_LEN]) -> Self {
        Self::from_zeroizing_bytes(Zeroizing::new(bytes))
    }

    pub fn to_storage_string(&self) -> StoredLocalUnlockSecret {
        StoredLocalUnlockSecret {
            value: Zeroizing::new(STANDARD.encode(&self.secret[..])),
        }
    }

    pub fn from_storage_string(value: &str) -> Result<Self, LocalSecretError> {
        let decoded = Zeroizing::new(STANDARD.decode(value)?);
        if decoded.len() != LOCAL_SECRET_LEN {
            return Err(LocalSecretError::InvalidSecretLength {
                expected: LOCAL_SECRET_LEN,
                actual: decoded.len(),
            });
        }

        let mut bytes = Zeroizing::new([0; LOCAL_SECRET_LEN]);
        bytes.copy_from_slice(decoded.as_slice());
        Ok(Self::from_zeroizing_bytes(bytes))
    }

    pub fn derive_sdk_store_key(&self) -> SdkStoreKey {
        SdkStoreKey {
            key: self
                .derive_key(SDK_STORE_INFO)
                .expect("32-byte HKDF output length is valid"),
        }
    }

    pub fn derive_search_key(&self) -> SearchIndexKey {
        let key = Zeroizing::new(
            self.derive_key(SEARCH_INDEX_INFO)
                .expect("32-byte HKDF output length is valid"),
        );
        SearchIndexKey {
            key: Zeroizing::new(STANDARD.encode(&key[..])),
        }
    }

    fn derive_key(
        &self,
        info: &[u8],
    ) -> Result<Zeroizing<[u8; LOCAL_SECRET_LEN]>, LocalSecretError> {
        let hkdf = Hkdf::<Sha256>::new(None, &self.secret[..]);
        let mut output = Zeroizing::new([0; LOCAL_SECRET_LEN]);
        hkdf.expand(info, &mut output[..])
            .map_err(|_| LocalSecretError::Derivation)?;
        Ok(output)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialStore {
    service_name: String,
}

impl CredentialStore {
    pub fn new(service_name: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
        }
    }

    pub fn save(
        &self,
        key_id: &SessionKeyId,
        secret: &LocalUnlockSecret,
    ) -> Result<(), LocalSecretError> {
        let storage_string = secret.to_storage_string();
        self.entry(key_id)?
            .set_password(storage_string.as_str())
            .map_err(LocalSecretError::CredentialStore)
    }

    pub fn load(&self, key_id: &SessionKeyId) -> Result<LocalUnlockSecret, LocalSecretError> {
        let stored_secret = Zeroizing::new(
            self.entry(key_id)?
                .get_password()
                .map_err(LocalSecretError::CredentialStore)?,
        );
        LocalUnlockSecret::from_storage_string(stored_secret.as_str())
    }

    pub fn delete(&self, key_id: &SessionKeyId) -> Result<(), LocalSecretError> {
        map_delete_result(self.entry(key_id)?.delete_credential())
    }

    fn entry(&self, key_id: &SessionKeyId) -> Result<Entry, LocalSecretError> {
        let account_name = key_id.account_name();
        Entry::new(&self.service_name, &account_name).map_err(LocalSecretError::CredentialStore)
    }
}

fn map_delete_result(result: keyring::Result<()>) -> Result<(), LocalSecretError> {
    match result {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(LocalSecretError::CredentialStore(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;

    #[test]
    fn namespaced_derivations_are_distinct() {
        let secret = LocalUnlockSecret::from_test_bytes([7; 32]);

        let sdk_store_key = secret.derive_sdk_store_key();
        let search_key = secret.derive_search_key();

        assert_eq!(sdk_store_key.as_bytes().len(), 32);
        assert_ne!(
            STANDARD.encode(sdk_store_key.as_bytes()),
            search_key.as_str()
        );
    }

    #[test]
    fn derived_and_stored_secrets_have_redacted_debug() {
        let secret = LocalUnlockSecret::from_test_bytes([7; 32]);
        let sdk_store_key = secret.derive_sdk_store_key();
        let search_key = secret.derive_search_key();
        let stored_secret = secret.to_storage_string();

        assert_eq!(format!("{sdk_store_key:?}"), "SdkStoreKey(..)");
        assert_eq!(format!("{search_key:?}"), "SearchIndexKey(..)");
        assert_eq!(format!("{stored_secret:?}"), "StoredLocalUnlockSecret(..)");
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
        let original = LocalUnlockSecret::from_test_bytes([9; 32]);

        let stored = original.to_storage_string();
        let restored = LocalUnlockSecret::from_storage_string(stored.as_str()).unwrap();

        assert_eq!(
            original.derive_sdk_store_key().as_bytes(),
            restored.derive_sdk_store_key().as_bytes()
        );
        assert_eq!(
            original.derive_search_key().as_str(),
            restored.derive_search_key().as_str()
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
        let store = CredentialStore::new("matrix-desktop-key-management-spike-test");
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
                secret.derive_search_key(),
                loaded.derive_search_key(),
            ))
        })();
        let cleanup_result = store.delete(&id);
        cleanup_result.unwrap();

        let (expected_sdk, loaded_sdk, expected_search, loaded_search) = result.unwrap();
        assert_eq!(expected_sdk.as_bytes(), loaded_sdk.as_bytes());
        assert_eq!(expected_search.as_str(), loaded_search.as_str());
    }
}
