use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, Mutex},
};

use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use hkdf::Hkdf;
use keyring::Entry;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use thiserror::Error;
use zeroize::Zeroizing;

pub const LOCAL_UNLOCK_SECRET_LEN: usize = 32;

const SDK_STORE_INFO: &[u8] = b"matrix-desktop:sdk-store";
const SEARCH_INDEX_INFO: &[u8] = b"matrix-desktop:search-index";
const COMPOSER_DRAFTS_INFO: &[u8] = b"matrix-desktop:composer-drafts";
const LAST_SESSION_ACCOUNT_NAME: &str = "matrix-desktop:last-session:v1";
const SAVED_SESSIONS_ACCOUNT_NAME: &str = "matrix-desktop:saved-sessions:v1";

#[derive(Debug, Error)]
pub enum LocalSecretError {
    #[error("credential store error: {0}")]
    CredentialStore(#[from] keyring::Error),
    #[error("credential backend error: {0}")]
    CredentialBackend(CredentialBackendErrorKind),
    #[error("key derivation failed")]
    Derivation,
    #[error("base64 decode error: {0}")]
    Base64Decode(#[from] base64::DecodeError),
    #[error("invalid secret length: expected {expected} bytes, got {actual}")]
    InvalidSecretLength { expected: usize, actual: usize },
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialBackendErrorKind {
    Unavailable,
    LockedOrInaccessible,
    MissingCredential,
    Corrupt,
}

impl CredentialBackendErrorKind {
    fn from_keyring_error(error: keyring::Error) -> Self {
        match error {
            keyring::Error::NoEntry => Self::MissingCredential,
            keyring::Error::NoStorageAccess(_) => Self::LockedOrInaccessible,
            keyring::Error::BadEncoding(_) | keyring::Error::Ambiguous(_) => Self::Corrupt,
            keyring::Error::PlatformFailure(_)
            | keyring::Error::TooLong(_, _)
            | keyring::Error::Invalid(_, _) => Self::Unavailable,
            _ => Self::Unavailable,
        }
    }
}

impl fmt::Display for CredentialBackendErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("unavailable"),
            Self::LockedOrInaccessible => formatter.write_str("locked or inaccessible"),
            Self::MissingCredential => formatter.write_str("missing credential"),
            Self::Corrupt => formatter.write_str("corrupt"),
        }
    }
}

pub trait CredentialBackend: Clone + fmt::Debug + Send + Sync + 'static {
    fn set_password(
        &self,
        service_name: &str,
        account_name: &str,
        value: &str,
    ) -> Result<(), CredentialBackendErrorKind>;

    fn get_password(
        &self,
        service_name: &str,
        account_name: &str,
    ) -> Result<String, CredentialBackendErrorKind>;

    fn delete_password(
        &self,
        service_name: &str,
        account_name: &str,
    ) -> Result<(), CredentialBackendErrorKind>;
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct KeyringCredentialBackend;

impl CredentialBackend for KeyringCredentialBackend {
    fn set_password(
        &self,
        service_name: &str,
        account_name: &str,
        value: &str,
    ) -> Result<(), CredentialBackendErrorKind> {
        Entry::new(service_name, account_name)
            .map_err(CredentialBackendErrorKind::from_keyring_error)?
            .set_password(value)
            .map_err(CredentialBackendErrorKind::from_keyring_error)
    }

    fn get_password(
        &self,
        service_name: &str,
        account_name: &str,
    ) -> Result<String, CredentialBackendErrorKind> {
        Entry::new(service_name, account_name)
            .map_err(CredentialBackendErrorKind::from_keyring_error)?
            .get_password()
            .map_err(CredentialBackendErrorKind::from_keyring_error)
    }

    fn delete_password(
        &self,
        service_name: &str,
        account_name: &str,
    ) -> Result<(), CredentialBackendErrorKind> {
        Entry::new(service_name, account_name)
            .map_err(CredentialBackendErrorKind::from_keyring_error)?
            .delete_credential()
            .map_err(CredentialBackendErrorKind::from_keyring_error)
    }
}

#[derive(Clone, Debug, Default)]
pub struct InMemoryCredentialBackend {
    inner: Arc<Mutex<InMemoryCredentialBackendState>>,
}

#[derive(Debug, Default)]
struct InMemoryCredentialBackendState {
    entries: BTreeMap<(String, String), String>,
    error: Option<CredentialBackendErrorKind>,
}

impl InMemoryCredentialBackend {
    pub fn set_error(&self, error: CredentialBackendErrorKind) {
        self.inner.lock().expect("in-memory backend mutex").error = Some(error);
    }
}

impl CredentialBackend for InMemoryCredentialBackend {
    fn set_password(
        &self,
        service_name: &str,
        account_name: &str,
        value: &str,
    ) -> Result<(), CredentialBackendErrorKind> {
        let mut state = self.inner.lock().expect("in-memory backend mutex");
        if let Some(error) = state.error {
            return Err(error);
        }
        state.entries.insert(
            (service_name.to_owned(), account_name.to_owned()),
            value.to_owned(),
        );
        Ok(())
    }

    fn get_password(
        &self,
        service_name: &str,
        account_name: &str,
    ) -> Result<String, CredentialBackendErrorKind> {
        let state = self.inner.lock().expect("in-memory backend mutex");
        if let Some(error) = state.error {
            return Err(error);
        }
        state
            .entries
            .get(&(service_name.to_owned(), account_name.to_owned()))
            .cloned()
            .ok_or(CredentialBackendErrorKind::MissingCredential)
    }

    fn delete_password(
        &self,
        service_name: &str,
        account_name: &str,
    ) -> Result<(), CredentialBackendErrorKind> {
        let mut state = self.inner.lock().expect("in-memory backend mutex");
        if let Some(error) = state.error {
            return Err(error);
        }
        state
            .entries
            .remove(&(service_name.to_owned(), account_name.to_owned()));
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SessionKeyId {
    pub homeserver: String,
    pub user_id: String,
    pub device_id: String,
}

impl SessionKeyId {
    pub fn account_name(&self) -> String {
        self.local_unlock_account_name()
    }

    pub fn local_unlock_account_name(&self) -> String {
        format!(
            "v1|{}|{}|{}",
            URL_SAFE_NO_PAD.encode(self.homeserver.as_bytes()),
            URL_SAFE_NO_PAD.encode(self.user_id.as_bytes()),
            URL_SAFE_NO_PAD.encode(self.device_id.as_bytes())
        )
    }

    pub fn matrix_session_account_name(&self) -> String {
        format!("matrix-session|{}", self.local_unlock_account_name())
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct LastSessionPointer {
    session_key_id: SessionKeyId,
}

impl LastSessionPointer {
    pub fn new(session_key_id: SessionKeyId) -> Self {
        Self { session_key_id }
    }

    pub fn session_key_id(&self) -> &SessionKeyId {
        &self.session_key_id
    }

    pub fn to_json(&self) -> Result<String, LocalSecretError> {
        serde_json::to_string(&self.session_key_id).map_err(LocalSecretError::Json)
    }

    pub fn from_json(value: &str) -> Result<Self, LocalSecretError> {
        Ok(Self {
            session_key_id: serde_json::from_str(value).map_err(LocalSecretError::Json)?,
        })
    }
}

impl fmt::Debug for LastSessionPointer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LastSessionPointer(..)")
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct SavedSessionIndex {
    sessions: Vec<SessionKeyId>,
}

impl SavedSessionIndex {
    pub fn new() -> Self {
        Self {
            sessions: Vec::new(),
        }
    }

    pub fn sessions(&self) -> &[SessionKeyId] {
        &self.sessions
    }

    pub fn upsert(&mut self, session: SessionKeyId) {
        if self.sessions.iter().any(|existing| existing == &session) {
            return;
        }
        self.sessions.push(session);
    }

    pub fn remove(&mut self, session: &SessionKeyId) {
        self.sessions.retain(|existing| existing != session);
    }

    pub fn to_json(&self) -> Result<String, LocalSecretError> {
        serde_json::to_string(&SavedSessionIndexPayload {
            version: 1,
            sessions: self.sessions.clone(),
        })
        .map_err(LocalSecretError::Json)
    }

    pub fn from_json(value: &str) -> Result<Self, LocalSecretError> {
        let payload: SavedSessionIndexPayload =
            serde_json::from_str(value).map_err(LocalSecretError::Json)?;
        Ok(Self {
            sessions: payload.sessions,
        })
    }
}

impl Default for SavedSessionIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for SavedSessionIndex {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SavedSessionIndex(..)")
    }
}

#[derive(Deserialize, Serialize)]
struct SavedSessionIndexPayload {
    version: u8,
    sessions: Vec<SessionKeyId>,
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

pub struct ComposerDraftsKey {
    key: Zeroizing<[u8; LOCAL_UNLOCK_SECRET_LEN]>,
}

impl ComposerDraftsKey {
    pub fn as_bytes(&self) -> &[u8; LOCAL_UNLOCK_SECRET_LEN] {
        &self.key
    }
}

impl fmt::Debug for ComposerDraftsKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ComposerDraftsKey(..)")
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

pub struct StoredMatrixSession {
    value: Zeroizing<String>,
}

impl StoredMatrixSession {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: Zeroizing::new(value.into()),
        }
    }

    pub fn as_str(&self) -> &str {
        self.value.as_str()
    }

    pub fn into_string(self) -> Zeroizing<String> {
        self.value
    }
}

impl fmt::Debug for StoredMatrixSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StoredMatrixSession(..)")
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

    pub fn derive_composer_drafts_key(&self) -> ComposerDraftsKey {
        ComposerDraftsKey {
            key: self
                .derive_key(COMPOSER_DRAFTS_INFO)
                .expect("32-byte HKDF output length is valid"),
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

#[derive(Clone, Debug)]
pub struct CredentialStore<B = KeyringCredentialBackend> {
    service_name: String,
    backend: B,
}

impl CredentialStore<KeyringCredentialBackend> {
    pub fn new(service_name: impl Into<String>) -> Self {
        Self::with_backend(service_name, KeyringCredentialBackend)
    }

    pub fn last_session_account_name() -> &'static str {
        LAST_SESSION_ACCOUNT_NAME
    }

    pub fn saved_sessions_account_name() -> &'static str {
        SAVED_SESSIONS_ACCOUNT_NAME
    }
}

impl<B: CredentialBackend> CredentialStore<B> {
    pub fn with_backend(service_name: impl Into<String>, backend: B) -> Self {
        Self {
            service_name: service_name.into(),
            backend,
        }
    }

    pub fn save(
        &self,
        key_id: &SessionKeyId,
        secret: &LocalUnlockSecret,
    ) -> Result<(), LocalSecretError> {
        let storage_string = secret.to_storage_string();
        self.backend
            .set_password(
                &self.service_name,
                &key_id.local_unlock_account_name(),
                storage_string.as_str(),
            )
            .map_err(LocalSecretError::CredentialBackend)
    }

    pub fn load(&self, key_id: &SessionKeyId) -> Result<LocalUnlockSecret, LocalSecretError> {
        let stored_secret = Zeroizing::new(self.load_raw(&key_id.local_unlock_account_name())?);
        LocalUnlockSecret::from_storage_string(stored_secret.as_str())
    }

    pub fn delete(&self, key_id: &SessionKeyId) -> Result<(), LocalSecretError> {
        self.delete_raw(&key_id.local_unlock_account_name())
    }

    pub fn save_matrix_session(
        &self,
        key_id: &SessionKeyId,
        session: &StoredMatrixSession,
    ) -> Result<(), LocalSecretError> {
        self.save_raw(&key_id.matrix_session_account_name(), session.as_str())
    }

    pub fn load_matrix_session(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<StoredMatrixSession, LocalSecretError> {
        let stored_session = Zeroizing::new(self.load_raw(&key_id.matrix_session_account_name())?);
        Ok(StoredMatrixSession {
            value: stored_session,
        })
    }

    pub fn delete_matrix_session(&self, key_id: &SessionKeyId) -> Result<(), LocalSecretError> {
        self.delete_raw(&key_id.matrix_session_account_name())
    }

    pub fn save_last_session(&self, key_id: &SessionKeyId) -> Result<(), LocalSecretError> {
        let pointer = LastSessionPointer::new(key_id.clone());
        let pointer_json = Zeroizing::new(pointer.to_json()?);
        self.save_raw(LAST_SESSION_ACCOUNT_NAME, pointer_json.as_str())
    }

    pub fn load_last_session(&self) -> Result<Option<SessionKeyId>, LocalSecretError> {
        match self.load_raw(LAST_SESSION_ACCOUNT_NAME) {
            Ok(pointer_json) => {
                let pointer_json = Zeroizing::new(pointer_json);
                Ok(Some(
                    LastSessionPointer::from_json(pointer_json.as_str())?
                        .session_key_id()
                        .clone(),
                ))
            }
            Err(err) if is_missing_credential_error(&err) => Ok(None),
            Err(err) => Err(err),
        }
    }

    pub fn delete_last_session(&self) -> Result<(), LocalSecretError> {
        self.delete_raw(LAST_SESSION_ACCOUNT_NAME)
    }

    pub fn load_saved_sessions(&self) -> Result<SavedSessionIndex, LocalSecretError> {
        match self.load_raw(SAVED_SESSIONS_ACCOUNT_NAME) {
            Ok(index_json) => {
                let index_json = Zeroizing::new(index_json);
                SavedSessionIndex::from_json(index_json.as_str())
            }
            Err(err) if is_missing_credential_error(&err) => Ok(SavedSessionIndex::new()),
            Err(err) => Err(err),
        }
    }

    pub fn save_saved_sessions(&self, index: &SavedSessionIndex) -> Result<(), LocalSecretError> {
        let index_json = Zeroizing::new(index.to_json()?);
        self.save_raw(SAVED_SESSIONS_ACCOUNT_NAME, index_json.as_str())
    }

    pub fn remember_saved_session(&self, key_id: &SessionKeyId) -> Result<(), LocalSecretError> {
        let mut index = self.load_saved_sessions()?;
        index.upsert(key_id.clone());
        self.save_saved_sessions(&index)
    }

    pub fn forget_saved_session(&self, key_id: &SessionKeyId) -> Result<(), LocalSecretError> {
        let mut index = self.load_saved_sessions()?;
        index.remove(key_id);
        self.save_saved_sessions(&index)
    }

    fn save_raw(&self, account_name: &str, value: &str) -> Result<(), LocalSecretError> {
        self.backend
            .set_password(&self.service_name, account_name, value)
            .map_err(LocalSecretError::CredentialBackend)
    }

    fn load_raw(&self, account_name: &str) -> Result<String, LocalSecretError> {
        self.backend
            .get_password(&self.service_name, account_name)
            .map_err(LocalSecretError::CredentialBackend)
    }

    fn delete_raw(&self, account_name: &str) -> Result<(), LocalSecretError> {
        match self
            .backend
            .delete_password(&self.service_name, account_name)
        {
            Ok(()) | Err(CredentialBackendErrorKind::MissingCredential) => Ok(()),
            Err(error) => Err(LocalSecretError::CredentialBackend(error)),
        }
    }
}

pub fn map_delete_result(result: keyring::Result<()>) -> Result<(), LocalSecretError> {
    match result {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(LocalSecretError::CredentialStore(error)),
    }
}

pub fn is_missing_credential_error(error: &LocalSecretError) -> bool {
    matches!(
        error,
        LocalSecretError::CredentialStore(keyring::Error::NoEntry)
            | LocalSecretError::CredentialBackend(CredentialBackendErrorKind::MissingCredential)
    )
}

pub fn is_locked_or_inaccessible_error(error: &LocalSecretError) -> bool {
    matches!(
        error,
        LocalSecretError::CredentialBackend(CredentialBackendErrorKind::LockedOrInaccessible)
    )
}
