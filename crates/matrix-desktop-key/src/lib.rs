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

pub const LOCAL_UNLOCK_SECRET_LEN: usize = 32;

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
    key: Zeroizing<[u8; LOCAL_UNLOCK_SECRET_LEN]>,
}

impl SdkStoreKey {
    pub fn as_bytes(&self) -> &[u8; LOCAL_UNLOCK_SECRET_LEN] {
        &self.key
    }

    pub fn into_bytes(self) -> Zeroizing<[u8; LOCAL_UNLOCK_SECRET_LEN]> {
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
    secret: Zeroizing<[u8; LOCAL_UNLOCK_SECRET_LEN]>,
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

    fn from_zeroizing_bytes(secret: Zeroizing<[u8; LOCAL_UNLOCK_SECRET_LEN]>) -> Self {
        Self { secret }
    }

    pub fn to_storage_string(&self) -> StoredLocalUnlockSecret {
        StoredLocalUnlockSecret {
            value: Zeroizing::new(STANDARD.encode(&self.secret[..])),
        }
    }

    pub fn from_storage_string(value: &str) -> Result<Self, LocalSecretError> {
        let decoded = Zeroizing::new(STANDARD.decode(value)?);
        if decoded.len() != LOCAL_UNLOCK_SECRET_LEN {
            return Err(LocalSecretError::InvalidSecretLength {
                expected: LOCAL_UNLOCK_SECRET_LEN,
                actual: decoded.len(),
            });
        }

        let mut bytes = Zeroizing::new([0; LOCAL_UNLOCK_SECRET_LEN]);
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

    pub fn derive_search_index_key(&self) -> SearchIndexKey {
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
    ) -> Result<Zeroizing<[u8; LOCAL_UNLOCK_SECRET_LEN]>, LocalSecretError> {
        let hkdf = Hkdf::<Sha256>::new(None, &self.secret[..]);
        let mut output = Zeroizing::new([0; LOCAL_UNLOCK_SECRET_LEN]);
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

pub fn map_delete_result(result: keyring::Result<()>) -> Result<(), LocalSecretError> {
    match result {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(LocalSecretError::CredentialStore(error)),
    }
}
