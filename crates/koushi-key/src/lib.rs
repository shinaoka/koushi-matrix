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
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use thiserror::Error;
use zeroize::Zeroizing;

pub const LOCAL_UNLOCK_SECRET_LEN: usize = 32;
pub const VAULT_MASTER_KEY_LEN: usize = 32;
const LOCAL_STORE_ID_LEN: usize = 16;

const SDK_STORE_INFO: &[u8] = b"koushi-desktop:sdk-store";
const SEARCH_INDEX_INFO: &[u8] = b"koushi-desktop:search-index";
const COMPOSER_DRAFTS_INFO: &[u8] = b"koushi-desktop:composer-drafts";
const SCHEDULED_SENDS_INFO: &[u8] = b"koushi-desktop:scheduled-sends";
const NAVIGATION_INFO: &[u8] = b"koushi-desktop:navigation";
const ROOM_PREFERENCES_INFO: &[u8] = b"koushi-desktop:room-preferences";
const READ_STATE_OUTBOX_INFO: &[u8] = b"koushi-desktop:read-state-outbox";
const LAST_SESSION_ACCOUNT_NAME: &str = "koushi-desktop:last-session:v1";
const SAVED_SESSIONS_ACCOUNT_NAME: &str = "koushi-desktop:saved-sessions:v1";
const CREDENTIAL_VAULT_KEY_ACCOUNT_NAME: &str = "koushi-desktop:credential-vault-key:v1";
const PENDING_LOGIN_JOURNAL_ACCOUNT_NAME: &str = "koushi-desktop:pending-login-journal:v1";
const LOCAL_STORE_MIGRATION_ACCOUNT_NAME: &str = "koushi-desktop:local-store-migration:v1";

pub fn last_session_account_name() -> &'static str {
    LAST_SESSION_ACCOUNT_NAME
}

pub fn saved_sessions_account_name() -> &'static str {
    SAVED_SESSIONS_ACCOUNT_NAME
}

pub fn credential_vault_key_account_name() -> &'static str {
    CREDENTIAL_VAULT_KEY_ACCOUNT_NAME
}

pub fn pending_login_journal_account_name() -> &'static str {
    PENDING_LOGIN_JOURNAL_ACCOUNT_NAME
}

pub fn local_store_migration_account_name() -> &'static str {
    LOCAL_STORE_MIGRATION_ACCOUNT_NAME
}

#[derive(Debug, Error)]
pub enum LocalSecretError {
    // Credential-backend failures are carried as the platform-free
    // `CredentialBackendErrorKind`; the platform adapter maps raw OS errors
    // into a kind so consumers (e.g. koushi-core) never see platform
    // error types.
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

/// Platform-independent credential store port.
///
/// Implementations provide OS-specific or test credential storage. The trait
/// is **object-safe** so callers can use `Arc<dyn CredentialBackend>`.
pub trait CredentialBackend: fmt::Debug + Send + Sync + 'static {
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

/// Forwarding blanket impl so `Arc<dyn CredentialBackend>` is itself a
/// `CredentialBackend`. This lets callers hold an `Arc<dyn>` as the backend
/// type and inject the real OS adapter from the platform layer.
impl<T: CredentialBackend + ?Sized> CredentialBackend for Arc<T> {
    fn set_password(
        &self,
        service_name: &str,
        account_name: &str,
        value: &str,
    ) -> Result<(), CredentialBackendErrorKind> {
        (**self).set_password(service_name, account_name, value)
    }

    fn get_password(
        &self,
        service_name: &str,
        account_name: &str,
    ) -> Result<String, CredentialBackendErrorKind> {
        (**self).get_password(service_name, account_name)
    }

    fn delete_password(
        &self,
        service_name: &str,
        account_name: &str,
    ) -> Result<(), CredentialBackendErrorKind> {
        (**self).delete_password(service_name, account_name)
    }
}

#[derive(Clone, Default)]
pub struct InMemoryCredentialBackend {
    inner: Arc<Mutex<InMemoryCredentialBackendState>>,
}

#[derive(Default)]
struct InMemoryCredentialBackendState {
    entries: BTreeMap<(String, String), String>,
    error: Option<CredentialBackendErrorKind>,
    delete_error: Option<CredentialBackendErrorKind>,
    get_password_count: usize,
}

impl InMemoryCredentialBackend {
    pub fn set_error(&self, error: CredentialBackendErrorKind) {
        self.inner.lock().expect("in-memory backend mutex").error = Some(error);
    }

    pub fn clear_error(&self) {
        self.inner.lock().expect("in-memory backend mutex").error = None;
    }

    pub fn set_delete_error(&self, error: CredentialBackendErrorKind) {
        self.inner
            .lock()
            .expect("in-memory backend mutex")
            .delete_error = Some(error);
    }

    pub fn clear_delete_error(&self) {
        self.inner
            .lock()
            .expect("in-memory backend mutex")
            .delete_error = None;
    }

    pub fn get_password_count(&self) -> usize {
        self.inner
            .lock()
            .expect("in-memory backend mutex")
            .get_password_count
    }

    pub fn contains_entry(&self, service_name: &str, account_name: &str) -> bool {
        self.inner
            .lock()
            .expect("in-memory backend mutex")
            .entries
            .contains_key(&(service_name.to_owned(), account_name.to_owned()))
    }
}

impl fmt::Debug for InMemoryCredentialBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.inner.lock().map_err(|_| fmt::Error)?;
        formatter
            .debug_struct("InMemoryCredentialBackend")
            .field("entry_count", &state.entries.len())
            .field("has_error", &state.error.is_some())
            .field("has_delete_error", &state.delete_error.is_some())
            .field("get_password_count", &state.get_password_count)
            .finish()
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
        let mut state = self.inner.lock().expect("in-memory backend mutex");
        state.get_password_count += 1;
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
        if let Some(error) = state.delete_error {
            return Err(error);
        }
        state
            .entries
            .remove(&(service_name.to_owned(), account_name.to_owned()));
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
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

/// Opaque, validated identity for an account-local encrypted store.
///
/// The value is intentionally not printable: it is a filesystem/credential
/// lookup key, not an application or diagnostic identifier.
#[derive(Clone, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct LocalStoreId(String);

impl LocalStoreId {
    pub fn generate() -> Self {
        Self(URL_SAFE_NO_PAD.encode(rand::random::<[u8; LOCAL_STORE_ID_LEN]>()))
    }

    pub fn parse(value: &str) -> Result<Self, LocalSecretError> {
        let decoded = URL_SAFE_NO_PAD
            .decode(value)
            .map_err(LocalSecretError::Base64Decode)?;
        if decoded.len() != LOCAL_STORE_ID_LEN || value != URL_SAFE_NO_PAD.encode(&decoded) {
            return Err(LocalSecretError::InvalidSecretLength {
                expected: LOCAL_STORE_ID_LEN,
                actual: decoded.len(),
            });
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for LocalStoreId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LocalStoreId(..)")
    }
}

/// The account-local identity and secret used to derive every encrypted store
/// key.  The identity is opaque and the secret never has a printable/debug
/// representation.
pub struct LocalStoreBinding {
    local_store_id: LocalStoreId,
    unlock_secret: LocalUnlockSecret,
}

impl LocalStoreBinding {
    pub fn new(local_store_id: LocalStoreId, unlock_secret: LocalUnlockSecret) -> Self {
        Self {
            local_store_id,
            unlock_secret,
        }
    }

    pub fn generate() -> Self {
        Self::new(LocalStoreId::generate(), LocalUnlockSecret::generate())
    }

    pub fn local_store_id(&self) -> &LocalStoreId {
        &self.local_store_id
    }

    pub fn unlock_secret(&self) -> &LocalUnlockSecret {
        &self.unlock_secret
    }
}

impl fmt::Debug for LocalStoreBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LocalStoreBinding(..)")
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

pub struct ScheduledSendsKey {
    key: Zeroizing<[u8; LOCAL_UNLOCK_SECRET_LEN]>,
}

impl ScheduledSendsKey {
    pub fn as_bytes(&self) -> &[u8; LOCAL_UNLOCK_SECRET_LEN] {
        &self.key
    }
}

impl fmt::Debug for ScheduledSendsKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ScheduledSendsKey(..)")
    }
}

pub struct NavigationKey {
    key: Zeroizing<[u8; LOCAL_UNLOCK_SECRET_LEN]>,
}

impl NavigationKey {
    pub fn as_bytes(&self) -> &[u8; LOCAL_UNLOCK_SECRET_LEN] {
        &self.key
    }
}

impl fmt::Debug for NavigationKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NavigationKey(..)")
    }
}

pub struct RoomPreferencesKey {
    key: Zeroizing<[u8; LOCAL_UNLOCK_SECRET_LEN]>,
}

pub struct ReadStateOutboxKey {
    key: Zeroizing<[u8; LOCAL_UNLOCK_SECRET_LEN]>,
}

impl ReadStateOutboxKey {
    pub fn as_bytes(&self) -> &[u8; LOCAL_UNLOCK_SECRET_LEN] {
        &self.key
    }
}

impl fmt::Debug for ReadStateOutboxKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ReadStateOutboxKey(..)")
    }
}

impl RoomPreferencesKey {
    pub fn as_bytes(&self) -> &[u8; LOCAL_UNLOCK_SECRET_LEN] {
        &self.key
    }
}

impl fmt::Debug for RoomPreferencesKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RoomPreferencesKey(..)")
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

pub struct CredentialVaultMasterKey {
    key: Zeroizing<[u8; VAULT_MASTER_KEY_LEN]>,
}

impl CredentialVaultMasterKey {
    pub fn generate() -> Self {
        Self {
            key: Zeroizing::new(rand::random()),
        }
    }

    pub fn to_storage_string(&self) -> Zeroizing<String> {
        Zeroizing::new(STANDARD.encode(&self.key[..]))
    }

    pub fn from_storage_string(value: &str) -> Result<Self, LocalSecretError> {
        let decoded = Zeroizing::new(STANDARD.decode(value)?);
        if decoded.len() != VAULT_MASTER_KEY_LEN {
            return Err(LocalSecretError::InvalidSecretLength {
                expected: VAULT_MASTER_KEY_LEN,
                actual: decoded.len(),
            });
        }
        let mut key = Zeroizing::new([0; VAULT_MASTER_KEY_LEN]);
        key.copy_from_slice(decoded.as_slice());
        Ok(Self { key })
    }

    pub fn as_bytes(&self) -> &[u8; VAULT_MASTER_KEY_LEN] {
        &self.key
    }
}

impl fmt::Debug for CredentialVaultMasterKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CredentialVaultMasterKey(..)")
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

    pub fn derive_scheduled_sends_key(&self) -> ScheduledSendsKey {
        ScheduledSendsKey {
            key: self
                .derive_key(SCHEDULED_SENDS_INFO)
                .expect("32-byte HKDF output length is valid"),
        }
    }

    pub fn derive_navigation_key(&self) -> NavigationKey {
        NavigationKey {
            key: self
                .derive_key(NAVIGATION_INFO)
                .expect("32-byte HKDF output length is valid"),
        }
    }

    pub fn derive_room_preferences_key(&self) -> RoomPreferencesKey {
        RoomPreferencesKey {
            key: self
                .derive_key(ROOM_PREFERENCES_INFO)
                .expect("32-byte HKDF output length is valid"),
        }
    }

    pub fn derive_read_state_outbox_key(&self) -> ReadStateOutboxKey {
        ReadStateOutboxKey {
            key: self
                .derive_key(READ_STATE_OUTBOX_INFO)
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

#[derive(Clone)]
pub struct CredentialStore<B> {
    service_name: String,
    backend: B,
}

impl<B> fmt::Debug for CredentialStore<B> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialStore")
            .field("service_name", &self.service_name)
            .field("backend", &"<redacted>")
            .finish()
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

    pub fn save_vault_master_key(
        &self,
        key: &CredentialVaultMasterKey,
    ) -> Result<(), LocalSecretError> {
        let storage_string = key.to_storage_string();
        self.save_raw(CREDENTIAL_VAULT_KEY_ACCOUNT_NAME, storage_string.as_str())
    }

    pub fn load_vault_master_key(&self) -> Result<CredentialVaultMasterKey, LocalSecretError> {
        let stored_key = Zeroizing::new(self.load_raw(CREDENTIAL_VAULT_KEY_ACCOUNT_NAME)?);
        CredentialVaultMasterKey::from_storage_string(stored_key.as_str())
    }

    pub fn delete_vault_master_key(&self) -> Result<(), LocalSecretError> {
        self.delete_raw(CREDENTIAL_VAULT_KEY_ACCOUNT_NAME)
    }

    /// Persist the serialized pending-login journal under its own credential
    /// name. Callers own the schema; this API deliberately does not pretend a
    /// pending allocation is a SessionKeyId.
    pub fn save_pending_login_journal(&self, value: &str) -> Result<(), LocalSecretError> {
        self.save_raw(PENDING_LOGIN_JOURNAL_ACCOUNT_NAME, value)
    }

    pub fn load_pending_login_journal(&self) -> Result<String, LocalSecretError> {
        self.load_raw(PENDING_LOGIN_JOURNAL_ACCOUNT_NAME)
    }

    pub fn delete_pending_login_journal(&self) -> Result<(), LocalSecretError> {
        self.delete_raw(PENDING_LOGIN_JOURNAL_ACCOUNT_NAME)
    }

    pub fn save_local_store_migration(&self, value: &str) -> Result<(), LocalSecretError> {
        self.save_raw(LOCAL_STORE_MIGRATION_ACCOUNT_NAME, value)
    }

    pub fn load_local_store_migration(&self) -> Result<String, LocalSecretError> {
        self.load_raw(LOCAL_STORE_MIGRATION_ACCOUNT_NAME)
    }

    pub fn delete_local_store_migration(&self) -> Result<(), LocalSecretError> {
        self.delete_raw(LOCAL_STORE_MIGRATION_ACCOUNT_NAME)
    }

    pub fn load(&self, key_id: &SessionKeyId) -> Result<LocalUnlockSecret, LocalSecretError> {
        let stored_secret = Zeroizing::new(self.load_raw(&key_id.local_unlock_account_name())?);
        LocalUnlockSecret::from_storage_string(stored_secret.as_str())
    }

    pub fn delete(&self, key_id: &SessionKeyId) -> Result<(), LocalSecretError> {
        self.delete_raw(&key_id.local_unlock_account_name())
    }

    pub fn save_local_store_id(
        &self,
        key_id: &SessionKeyId,
        store_id: &LocalStoreId,
    ) -> Result<(), LocalSecretError> {
        self.save_raw(
            &format!("local-store|{}", key_id.local_unlock_account_name()),
            store_id.as_str(),
        )
    }

    pub fn load_local_store_id(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<LocalStoreId, LocalSecretError> {
        let stored = Zeroizing::new(self.load_raw(&format!(
            "local-store|{}",
            key_id.local_unlock_account_name()
        ))?);
        LocalStoreId::parse(stored.as_str())
    }

    pub fn delete_local_store_id(&self, key_id: &SessionKeyId) -> Result<(), LocalSecretError> {
        self.delete_raw(&format!(
            "local-store|{}",
            key_id.local_unlock_account_name()
        ))
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

    pub fn delete_saved_sessions(&self) -> Result<(), LocalSecretError> {
        self.delete_raw(SAVED_SESSIONS_ACCOUNT_NAME)
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

pub fn is_missing_credential_error(error: &LocalSecretError) -> bool {
    matches!(
        error,
        LocalSecretError::CredentialBackend(CredentialBackendErrorKind::MissingCredential)
    )
}

pub fn is_locked_or_inaccessible_error(error: &LocalSecretError) -> bool {
    matches!(
        error,
        LocalSecretError::CredentialBackend(CredentialBackendErrorKind::LockedOrInaccessible)
    )
}

#[cfg(test)]
mod tests {
    use super::{
        CredentialStore, CredentialVaultMasterKey, InMemoryCredentialBackend, LocalUnlockSecret,
        credential_vault_key_account_name,
    };

    #[test]
    fn credential_vault_master_key_round_trips_without_debug_exposure() {
        let key = CredentialVaultMasterKey::generate();
        let encoded = key.to_storage_string();
        let restored =
            CredentialVaultMasterKey::from_storage_string(encoded.as_str()).expect("decode key");

        assert_eq!(restored.as_bytes(), key.as_bytes());
        assert!(!format!("{key:?}").contains(encoded.as_str()));
        assert_eq!(
            credential_vault_key_account_name(),
            "koushi-desktop:credential-vault-key:v1"
        );
    }

    #[test]
    fn credential_vault_master_key_store_counts_one_read() {
        let backend = InMemoryCredentialBackend::default();
        let store = CredentialStore::with_backend("service", backend.clone());
        let key = CredentialVaultMasterKey::generate();

        store.save_vault_master_key(&key).expect("save key");
        let restored = store.load_vault_master_key().expect("load key");

        assert_eq!(restored.as_bytes(), key.as_bytes());
        assert_eq!(backend.get_password_count(), 1);
        assert!(backend.contains_entry("service", credential_vault_key_account_name()));
    }

    #[test]
    fn credential_store_debug_redacts_backend_entries() {
        let backend = InMemoryCredentialBackend::default();
        let store = CredentialStore::with_backend("service", backend.clone());
        let key = CredentialVaultMasterKey::generate();
        let encoded = key.to_storage_string();
        store.save_vault_master_key(&key).expect("save key");

        let backend_debug = format!("{backend:?}");
        let store_debug = format!("{store:?}");
        assert!(!backend_debug.contains(encoded.as_str()));
        assert!(!store_debug.contains(encoded.as_str()));
        assert!(!backend_debug.contains(credential_vault_key_account_name()));
        assert!(!store_debug.contains(credential_vault_key_account_name()));
    }

    #[test]
    fn read_state_outbox_uses_a_dedicated_hkdf_domain() {
        let secret = LocalUnlockSecret::generate();
        let read_state = secret.derive_read_state_outbox_key();

        assert_ne!(
            read_state.as_bytes(),
            secret.derive_composer_drafts_key().as_bytes()
        );
        assert_ne!(
            read_state.as_bytes(),
            secret.derive_scheduled_sends_key().as_bytes()
        );
        assert_ne!(
            read_state.as_bytes(),
            secret.derive_navigation_key().as_bytes()
        );
    }
}
