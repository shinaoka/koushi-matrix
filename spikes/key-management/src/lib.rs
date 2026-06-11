use std::fmt;

use base64::{Engine as _, engine::general_purpose::STANDARD};
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
        format!("{}|{}|{}", self.homeserver, self.user_id, self.device_id)
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
        Self::from_bytes(rand::random())
    }

    pub fn from_bytes(bytes: [u8; LOCAL_SECRET_LEN]) -> Self {
        Self {
            secret: Zeroizing::new(bytes),
        }
    }

    pub fn to_storage_string(&self) -> String {
        STANDARD.encode(&self.secret[..])
    }

    pub fn from_storage_string(value: &str) -> Result<Self, LocalSecretError> {
        let decoded = Zeroizing::new(STANDARD.decode(value)?);
        if decoded.len() != LOCAL_SECRET_LEN {
            return Err(LocalSecretError::InvalidSecretLength {
                expected: LOCAL_SECRET_LEN,
                actual: decoded.len(),
            });
        }

        let mut bytes = [0; LOCAL_SECRET_LEN];
        bytes.copy_from_slice(decoded.as_slice());
        Ok(Self::from_bytes(bytes))
    }

    pub fn derive_sdk_store_key(&self) -> [u8; LOCAL_SECRET_LEN] {
        self.derive_key(SDK_STORE_INFO)
            .expect("32-byte HKDF output length is valid")
    }

    pub fn derive_search_key(&self) -> Zeroizing<String> {
        let key = Zeroizing::new(
            self.derive_key(SEARCH_INDEX_INFO)
                .expect("32-byte HKDF output length is valid"),
        );
        Zeroizing::new(STANDARD.encode(&key[..]))
    }

    fn derive_key(&self, info: &[u8]) -> Result<[u8; LOCAL_SECRET_LEN], LocalSecretError> {
        let hkdf = Hkdf::<Sha256>::new(None, &self.secret[..]);
        let mut output = [0; LOCAL_SECRET_LEN];
        hkdf.expand(info, &mut output)
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
        self.entry(key_id)?
            .set_password(&secret.to_storage_string())
            .map_err(LocalSecretError::CredentialStore)
    }

    pub fn load(&self, key_id: &SessionKeyId) -> Result<LocalUnlockSecret, LocalSecretError> {
        let stored_secret = self
            .entry(key_id)?
            .get_password()
            .map_err(LocalSecretError::CredentialStore)?;
        LocalUnlockSecret::from_storage_string(&stored_secret)
    }

    pub fn delete(&self, key_id: &SessionKeyId) -> Result<(), LocalSecretError> {
        self.entry(key_id)?
            .delete_credential()
            .map_err(LocalSecretError::CredentialStore)
    }

    fn entry(&self, key_id: &SessionKeyId) -> Result<Entry, LocalSecretError> {
        let account_name = key_id.account_name();
        Entry::new(&self.service_name, &account_name).map_err(LocalSecretError::CredentialStore)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;

    #[test]
    fn namespaced_derivations_are_distinct() {
        let secret = LocalUnlockSecret::from_bytes([7; 32]);

        let sdk_store_key = secret.derive_sdk_store_key();
        let search_key = secret.derive_search_key();

        assert_eq!(sdk_store_key.len(), 32);
        assert_ne!(STANDARD.encode(sdk_store_key), *search_key);
    }

    #[test]
    fn account_name_joins_session_key_parts() {
        let id = SessionKeyId {
            homeserver: "https://matrix.example".into(),
            user_id: "@alice:example.com".into(),
            device_id: "DEVICE123".into(),
        };

        assert_eq!(
            id.account_name(),
            "https://matrix.example|@alice:example.com|DEVICE123"
        );
    }

    #[test]
    fn storage_round_trip_preserves_derivations() {
        let original = LocalUnlockSecret::from_bytes([9; 32]);

        let stored = original.to_storage_string();
        let restored = LocalUnlockSecret::from_storage_string(&stored).unwrap();

        assert_eq!(
            original.derive_sdk_store_key(),
            restored.derive_sdk_store_key()
        );
        assert_eq!(*original.derive_search_key(), *restored.derive_search_key());
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
        store.save(&id, &secret).unwrap();
        let loaded = store.load(&id).unwrap();
        store.delete(&id).unwrap();

        assert_eq!(secret.derive_sdk_store_key(), loaded.derive_sdk_store_key());
        assert_eq!(*secret.derive_search_key(), *loaded.derive_search_key());
    }
}
