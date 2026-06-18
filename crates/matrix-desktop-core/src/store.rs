//! StoreActor: credential store access, per-account store paths, store/search
//! key derivation, and debug/test credential injection policy.
//!
//! Security invariants:
//! - Store and search keys NEVER cross the command/event boundary.
//! - If credential store or encryption cannot be initialized for an account,
//!   `LocalEncryptionUnavailable` is returned (fail-closed).
//! - The file-based credential store override is behind a compile-time gate:
//!   `#[cfg(any(debug_assertions, test))]` only.
//!
//! Architecture: overview.md Platform Portability rule 3 — platform
//! capabilities live here behind a port. StoreActor is the only actor allowed
//! platform-conditional code.

use std::path::PathBuf;

use chacha20poly1305::{
    ChaCha20Poly1305, Key, KeyInit, Nonce,
    aead::{Aead, OsRng, rand_core::RngCore},
};
use matrix_desktop_key::{CredentialStore, LocalUnlockSecret, SessionKeyId};
use matrix_desktop_sdk::{
    MatrixClientStoreConfig, MatrixClientStoreKey, MatrixSearchIndexKey,
    MatrixSearchIndexStoreConfig,
};
use matrix_desktop_state::{ComposerDraftStore, LocalEncryptionHealth};

use crate::failure::CoreFailure;

/// Service name used for OS keyring entries. This is user-visible in macOS
/// Keychain Access, so keep it aligned with the shipped product name.
const CREDENTIAL_STORE_SERVICE_NAME: &str = "koushi-desktop";
const LEGACY_CREDENTIAL_STORE_SERVICE_NAME: &str = "matrix-desktop";
const COMPOSER_DRAFTS_FILE_MAGIC: &[u8] = b"RURI-DRAFTS-V1\0";
const COMPOSER_DRAFTS_NONCE_LEN: usize = 12;

/// Env var for QA/debug file-based credential store override.
/// Only honored in debug/test builds; release builds ignore it entirely.
#[cfg(any(debug_assertions, test))]
const ENV_FILE_CREDENTIAL_STORE_DIR: &str = "MATRIX_DESKTOP_QA_FILE_CREDENTIAL_STORE_DIR";

/// Resolved store configuration for one account.
///
/// Keys never leave this module's calling chain — they are consumed by
/// `login_with_password_with_store` / `restore_session_with_store` and then
/// dropped. They never appear in events, snapshots, or logs.
pub struct AccountStoreConfig {
    pub store_config: MatrixClientStoreConfig,
    /// The session key id that identifies this account in the credential store.
    /// Retained so the account actor can persist / delete credentials.
    pub session_key_id: SessionKeyId,
}

/// Resolved search index configuration for one account.
///
/// Key never crosses the command/event boundary. Consumed by the client
/// builder and then dropped.
pub struct AccountSearchIndexConfig {
    pub search_index_config: MatrixSearchIndexStoreConfig,
}

/// StoreActor: resolves and manages per-account credential-backed store configs.
///
/// Owns the single `CredentialStoreBackend` — used for both unlock secrets
/// and session persistence. AccountActor delegates all credential operations
/// through `StoreActor`.
///
/// In Phase 2 this is a pure value type (no background task). Phase 6 may
/// promote it to an owned task when search index mutations require it.
pub struct StoreActor {
    pub(crate) credential_store: CredentialStoreBackend,
    data_dir: PathBuf,
}

impl StoreActor {
    /// Create the actor. `data_dir` is the application data directory under
    /// which per-account sub-directories are created.
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            credential_store: CredentialStoreBackend::resolve(),
            data_dir: data_dir.into(),
        }
    }

    /// Access the credential store backend (for session persistence in AccountActor).
    pub fn credential_backend(&self) -> &CredentialStoreBackend {
        &self.credential_store
    }

    /// Test-only constructor with an explicit backend (avoids the env-global
    /// `MATRIX_DESKTOP_QA_FILE_CREDENTIAL_STORE_DIR` race between unit tests).
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn with_backend(
        credential_store: CredentialStoreBackend,
        data_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            credential_store,
            data_dir: data_dir.into(),
        }
    }

    /// Resolve (and if necessary create) a store configuration for the given
    /// account identity. On first use a fresh `LocalUnlockSecret` is generated
    /// and persisted; on subsequent uses the existing secret is loaded.
    ///
    /// Returns `LocalEncryptionUnavailable` if the credential store or key
    /// derivation fails — login/restore must not proceed in that case.
    pub fn account_store_config(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<AccountStoreConfig, CoreFailure> {
        let secret = self.load_or_create_unlock_secret(key_id)?;
        let sdk_store_key = secret.derive_sdk_store_key();
        let store_key = MatrixClientStoreKey::new(*sdk_store_key.as_bytes());

        let store_dir = self.account_store_dir(key_id);
        let cache_dir = self.account_cache_dir(key_id);

        let store_config =
            MatrixClientStoreConfig::new(&store_dir, store_key).with_cache_path(&cache_dir);

        Ok(AccountStoreConfig {
            store_config,
            session_key_id: key_id.clone(),
        })
    }

    /// Derive the encrypted ngram search index configuration for the given
    /// account. Called by `AccountActor` when building the store-backed client
    /// so the SDK search index is initialized with the correct key.
    ///
    /// Returns `LocalEncryptionUnavailable` if the credential store is
    /// unreachable — the same fail-closed behavior as `account_store_config`.
    pub fn account_search_index_config(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<AccountSearchIndexConfig, CoreFailure> {
        let secret = self.load_or_create_unlock_secret(key_id)?;
        let search_key = secret.derive_search_index_key();
        let search_dir = self.account_search_index_dir(key_id);
        let config = MatrixSearchIndexStoreConfig::new(
            &search_dir,
            MatrixSearchIndexKey::new(search_key.as_str()),
        );
        Ok(AccountSearchIndexConfig {
            search_index_config: config,
        })
    }

    pub fn load_composer_drafts(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<ComposerDraftStore, CoreFailure> {
        let path = self.account_composer_drafts_file(key_id);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ComposerDraftStore::default());
            }
            Err(_) => return Err(CoreFailure::StoreUnavailable),
        };
        decrypt_composer_drafts_payload(&self.load_unlock_secret(key_id)?, &bytes)
    }

    pub fn save_composer_drafts(
        &self,
        key_id: &SessionKeyId,
        drafts: &ComposerDraftStore,
    ) -> Result<(), CoreFailure> {
        let path = self.account_composer_drafts_file(key_id);
        let drafts = drafts.bounded_for_persistence();
        if drafts.is_empty() {
            match std::fs::remove_file(&path) {
                Ok(()) => return Ok(()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(_) => return Err(CoreFailure::StoreUnavailable),
            }
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| CoreFailure::StoreUnavailable)?;
        }
        let payload =
            encrypt_composer_drafts_payload(&self.load_or_create_unlock_secret(key_id)?, &drafts)?;
        std::fs::write(path, payload).map_err(|_| CoreFailure::StoreUnavailable)
    }

    /// Delete the stored unlock secret and the per-account store/cache
    /// directories for an account (shutdown step 7: "clear credentials and
    /// stores"). Called during logout / account removal.
    ///
    /// Errors do not propagate — a logout that partially cleans up is better
    /// than a logout that fails. Matrix session JSON / pointers stored via the
    /// credential backend are cleaned up by AccountActor through the same
    /// backend.
    pub fn delete_account_credentials(&self, key_id: &SessionKeyId) {
        let _ = self.credential_store.delete(key_id);
        let _ = std::fs::remove_dir_all(self.account_root_dir(key_id));
    }

    /// Probe the stored local unlock secret without creating a new one.
    ///
    /// This is the Rust-owned source for Settings/Security credential-store
    /// health. It is intentionally kind-only; raw backend errors never leave
    /// the store layer.
    pub fn probe_local_encryption_health(&self, key_id: &SessionKeyId) -> LocalEncryptionHealth {
        match self.credential_store.load(key_id) {
            Ok(_) => LocalEncryptionHealth::Healthy,
            Err(error) => local_secret_error_health(&error),
        }
    }

    /// The OS or file-based credential store backend.
    pub fn credential_store_backend(&self) -> &CredentialStoreBackend {
        &self.credential_store
    }

    /// Application data directory under which per-account sub-directories are
    /// created.
    pub fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }

    // --- private helpers ---

    fn load_or_create_unlock_secret(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<LocalUnlockSecret, CoreFailure> {
        match self.credential_store.load(key_id) {
            Ok(secret) => Ok(secret),
            Err(err) if matrix_desktop_key::is_missing_credential_error(&err) => {
                // First use: generate and persist a new unlock secret.
                let secret = LocalUnlockSecret::generate();
                self.credential_store
                    .save(key_id, &secret)
                    .map_err(|_| CoreFailure::LocalEncryptionUnavailable)?;
                Ok(secret)
            }
            Err(_) => Err(CoreFailure::LocalEncryptionUnavailable),
        }
    }

    fn load_unlock_secret(&self, key_id: &SessionKeyId) -> Result<LocalUnlockSecret, CoreFailure> {
        self.credential_store
            .load(key_id)
            .map_err(|_| CoreFailure::LocalEncryptionUnavailable)
    }

    fn account_root_dir(&self, key_id: &SessionKeyId) -> PathBuf {
        self.data_dir
            .join("accounts")
            .join(account_dir_name(key_id))
    }

    fn account_store_dir(&self, key_id: &SessionKeyId) -> PathBuf {
        self.account_root_dir(key_id).join("store")
    }

    fn account_cache_dir(&self, key_id: &SessionKeyId) -> PathBuf {
        self.account_root_dir(key_id).join("cache")
    }

    fn account_search_index_dir(&self, key_id: &SessionKeyId) -> PathBuf {
        self.account_root_dir(key_id).join("search-index")
    }

    fn account_composer_drafts_file(&self, key_id: &SessionKeyId) -> PathBuf {
        self.account_root_dir(key_id)
            .join("composer-drafts")
            .join("drafts.v1.enc")
    }
}

fn encrypt_composer_drafts_payload(
    secret: &LocalUnlockSecret,
    drafts: &ComposerDraftStore,
) -> Result<Vec<u8>, CoreFailure> {
    let plaintext = serde_json::to_vec(drafts).map_err(|_| CoreFailure::StoreUnavailable)?;
    let key = secret.derive_composer_drafts_key();
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key.as_bytes()));
    let mut nonce_bytes = [0_u8; COMPOSER_DRAFTS_NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_ref())
        .map_err(|_| CoreFailure::StoreUnavailable)?;
    let mut payload = Vec::with_capacity(
        COMPOSER_DRAFTS_FILE_MAGIC.len() + COMPOSER_DRAFTS_NONCE_LEN + ciphertext.len(),
    );
    payload.extend_from_slice(COMPOSER_DRAFTS_FILE_MAGIC);
    payload.extend_from_slice(&nonce_bytes);
    payload.extend_from_slice(&ciphertext);
    Ok(payload)
}

fn decrypt_composer_drafts_payload(
    secret: &LocalUnlockSecret,
    payload: &[u8],
) -> Result<ComposerDraftStore, CoreFailure> {
    let header_len = COMPOSER_DRAFTS_FILE_MAGIC.len() + COMPOSER_DRAFTS_NONCE_LEN;
    if payload.len() < header_len || !payload.starts_with(COMPOSER_DRAFTS_FILE_MAGIC) {
        return Err(CoreFailure::StoreUnavailable);
    }
    let nonce_start = COMPOSER_DRAFTS_FILE_MAGIC.len();
    let nonce_end = nonce_start + COMPOSER_DRAFTS_NONCE_LEN;
    let nonce = Nonce::from_slice(&payload[nonce_start..nonce_end]);
    let key = secret.derive_composer_drafts_key();
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key.as_bytes()));
    let plaintext = cipher
        .decrypt(nonce, &payload[nonce_end..])
        .map_err(|_| CoreFailure::StoreUnavailable)?;
    serde_json::from_slice(&plaintext).map_err(|_| CoreFailure::StoreUnavailable)
}

/// Derive a filesystem-safe directory name from a `SessionKeyId`.
/// Uses the same base64url encoding the key crate uses for credential store
/// account names, so both namespaces are consistent.
fn account_dir_name(key_id: &SessionKeyId) -> String {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    // Build a deterministic slug: encode homeserver + user_id + device_id
    // separated by underscores so the path stays readable in debug tooling.
    format!(
        "{}_{}_{}",
        URL_SAFE_NO_PAD.encode(key_id.homeserver.as_bytes()),
        URL_SAFE_NO_PAD.encode(key_id.user_id.as_bytes()),
        URL_SAFE_NO_PAD.encode(key_id.device_id.as_bytes()),
    )
}

/// Credential store backend. Production = OS keychain; debug/test = file dir
/// override when `MATRIX_DESKTOP_QA_FILE_CREDENTIAL_STORE_DIR` is set.
pub enum CredentialStoreBackend {
    OsKeychain(OsCredentialStore),
    #[cfg(any(debug_assertions, test))]
    FileDir(FileCredentialStore),
    #[cfg(test)]
    InMemory(CredentialStore<matrix_desktop_key::InMemoryCredentialBackend>),
}

impl CredentialStoreBackend {
    fn resolve() -> Self {
        #[cfg(any(debug_assertions, test))]
        if let Ok(dir) = std::env::var(ENV_FILE_CREDENTIAL_STORE_DIR) {
            let dir = PathBuf::from(dir);
            tracing_or_eprintln("file credential store active (debug/test only)");
            return Self::FileDir(FileCredentialStore::new(dir));
        }
        Self::OsKeychain(OsCredentialStore::new())
    }

    fn load(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<LocalUnlockSecret, matrix_desktop_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.load(key_id),
            #[cfg(any(debug_assertions, test))]
            Self::FileDir(store) => store.load(key_id),
            #[cfg(test)]
            Self::InMemory(store) => store.load(key_id),
        }
    }

    fn save(
        &self,
        key_id: &SessionKeyId,
        secret: &LocalUnlockSecret,
    ) -> Result<(), matrix_desktop_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.save(key_id, secret),
            #[cfg(any(debug_assertions, test))]
            Self::FileDir(store) => store.save(key_id, secret),
            #[cfg(test)]
            Self::InMemory(store) => store.save(key_id, secret),
        }
    }

    fn delete(&self, key_id: &SessionKeyId) -> Result<(), matrix_desktop_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.delete(key_id),
            #[cfg(any(debug_assertions, test))]
            Self::FileDir(store) => store.delete(key_id),
            #[cfg(test)]
            Self::InMemory(store) => store.delete(key_id),
        }
    }

    // --- Session persistence operations ---
    // These mirror the CredentialStore API so AccountActor can operate against
    // both backends without knowing which is active.

    pub fn save_matrix_session(
        &self,
        key_id: &SessionKeyId,
        session: &matrix_desktop_key::StoredMatrixSession,
    ) -> Result<(), matrix_desktop_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.save_matrix_session(key_id, session),
            #[cfg(any(debug_assertions, test))]
            Self::FileDir(store) => {
                store.save_named(&key_id.matrix_session_account_name(), session.as_str())
            }
            #[cfg(test)]
            Self::InMemory(store) => store.save_matrix_session(key_id, session),
        }
    }

    pub fn load_matrix_session(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<matrix_desktop_key::StoredMatrixSession, matrix_desktop_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.load_matrix_session(key_id),
            #[cfg(any(debug_assertions, test))]
            Self::FileDir(store) => {
                let value = store.load_named(&key_id.matrix_session_account_name())?;
                Ok(matrix_desktop_key::StoredMatrixSession::new(value))
            }
            #[cfg(test)]
            Self::InMemory(store) => store.load_matrix_session(key_id),
        }
    }

    pub fn delete_matrix_session(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<(), matrix_desktop_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.delete_matrix_session(key_id),
            #[cfg(any(debug_assertions, test))]
            Self::FileDir(store) => store.delete_named(&key_id.matrix_session_account_name()),
            #[cfg(test)]
            Self::InMemory(store) => store.delete_matrix_session(key_id),
        }
    }

    pub fn save_last_session(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<(), matrix_desktop_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.save_last_session(key_id),
            #[cfg(any(debug_assertions, test))]
            Self::FileDir(store) => {
                let pointer = matrix_desktop_key::LastSessionPointer::new(key_id.clone());
                let json = pointer.to_json()?;
                store.save_named(
                    matrix_desktop_key::CredentialStore::last_session_account_name(),
                    &json,
                )
            }
            #[cfg(test)]
            Self::InMemory(store) => store.save_last_session(key_id),
        }
    }

    pub fn load_last_session(
        &self,
    ) -> Result<Option<SessionKeyId>, matrix_desktop_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.load_last_session(),
            #[cfg(any(debug_assertions, test))]
            Self::FileDir(store) => {
                match store
                    .load_named(matrix_desktop_key::CredentialStore::last_session_account_name())
                {
                    Ok(json) => Ok(Some(
                        matrix_desktop_key::LastSessionPointer::from_json(&json)?
                            .session_key_id()
                            .clone(),
                    )),
                    Err(err) if matrix_desktop_key::is_missing_credential_error(&err) => Ok(None),
                    Err(err) => Err(err),
                }
            }
            #[cfg(test)]
            Self::InMemory(store) => store.load_last_session(),
        }
    }

    pub fn delete_last_session(&self) -> Result<(), matrix_desktop_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.delete_last_session(),
            #[cfg(any(debug_assertions, test))]
            Self::FileDir(store) => {
                store.delete_named(matrix_desktop_key::CredentialStore::last_session_account_name())
            }
            #[cfg(test)]
            Self::InMemory(store) => store.delete_last_session(),
        }
    }

    pub fn load_saved_sessions(
        &self,
    ) -> Result<matrix_desktop_key::SavedSessionIndex, matrix_desktop_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.load_saved_sessions(),
            #[cfg(any(debug_assertions, test))]
            Self::FileDir(store) => {
                match store
                    .load_named(matrix_desktop_key::CredentialStore::saved_sessions_account_name())
                {
                    Ok(json) => matrix_desktop_key::SavedSessionIndex::from_json(&json),
                    Err(err) if matrix_desktop_key::is_missing_credential_error(&err) => {
                        Ok(matrix_desktop_key::SavedSessionIndex::new())
                    }
                    Err(err) => Err(err),
                }
            }
            #[cfg(test)]
            Self::InMemory(store) => store.load_saved_sessions(),
        }
    }

    pub fn remember_saved_session(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<(), matrix_desktop_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.remember_saved_session(key_id),
            #[cfg(any(debug_assertions, test))]
            Self::FileDir(store) => {
                let mut index = self.load_saved_sessions()?;
                index.upsert(key_id.clone());
                store.save_named(
                    matrix_desktop_key::CredentialStore::saved_sessions_account_name(),
                    &index.to_json()?,
                )
            }
            #[cfg(test)]
            Self::InMemory(store) => store.remember_saved_session(key_id),
        }
    }

    pub fn forget_saved_session(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<(), matrix_desktop_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.forget_saved_session(key_id),
            #[cfg(any(debug_assertions, test))]
            Self::FileDir(store) => {
                let mut index = self.load_saved_sessions()?;
                index.remove(key_id);
                store.save_named(
                    matrix_desktop_key::CredentialStore::saved_sessions_account_name(),
                    &index.to_json()?,
                )
            }
            #[cfg(test)]
            Self::InMemory(store) => store.forget_saved_session(key_id),
        }
    }

    /// Expose the underlying `CredentialStore` (for OS keychain backend).
    pub fn as_os_credential_store(&self) -> Option<&CredentialStore> {
        match self {
            Self::OsKeychain(store) => Some(store.primary()),
            #[cfg(any(debug_assertions, test))]
            Self::FileDir(_) => None,
            #[cfg(test)]
            Self::InMemory(_) => None,
        }
    }
}

pub struct OsCredentialStore<B = matrix_desktop_key::KeyringCredentialBackend> {
    primary: CredentialStore<B>,
    legacy: Option<CredentialStore<B>>,
}

impl OsCredentialStore {
    fn new() -> Self {
        let legacy = (LEGACY_CREDENTIAL_STORE_SERVICE_NAME != CREDENTIAL_STORE_SERVICE_NAME)
            .then(|| CredentialStore::new(LEGACY_CREDENTIAL_STORE_SERVICE_NAME));
        Self {
            primary: CredentialStore::new(CREDENTIAL_STORE_SERVICE_NAME),
            legacy,
        }
    }
}

impl<B: matrix_desktop_key::CredentialBackend> OsCredentialStore<B> {
    fn primary(&self) -> &CredentialStore<B> {
        &self.primary
    }

    fn load(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<LocalUnlockSecret, matrix_desktop_key::LocalSecretError> {
        match self.primary.load(key_id) {
            Ok(secret) => Ok(secret),
            Err(error) if matrix_desktop_key::is_missing_credential_error(&error) => {
                let Some(legacy) = &self.legacy else {
                    return Err(error);
                };
                match legacy.load(key_id) {
                    Ok(secret) => {
                        self.primary.save(key_id, &secret)?;
                        let _ = legacy.delete(key_id);
                        Ok(secret)
                    }
                    Err(legacy_error)
                        if matrix_desktop_key::is_missing_credential_error(&legacy_error) =>
                    {
                        Err(error)
                    }
                    Err(legacy_error) => Err(legacy_error),
                }
            }
            Err(error) => Err(error),
        }
    }

    fn save(
        &self,
        key_id: &SessionKeyId,
        secret: &LocalUnlockSecret,
    ) -> Result<(), matrix_desktop_key::LocalSecretError> {
        self.primary.save(key_id, secret)?;
        if let Some(legacy) = &self.legacy {
            let _ = legacy.delete(key_id);
        }
        Ok(())
    }

    fn delete(&self, key_id: &SessionKeyId) -> Result<(), matrix_desktop_key::LocalSecretError> {
        let primary = self.primary.delete(key_id);
        let legacy = self.legacy.as_ref().map(|store| store.delete(key_id));
        primary?;
        if let Some(result) = legacy {
            result?;
        }
        Ok(())
    }

    fn save_matrix_session(
        &self,
        key_id: &SessionKeyId,
        session: &matrix_desktop_key::StoredMatrixSession,
    ) -> Result<(), matrix_desktop_key::LocalSecretError> {
        self.primary.save_matrix_session(key_id, session)?;
        if let Some(legacy) = &self.legacy {
            let _ = legacy.delete_matrix_session(key_id);
        }
        Ok(())
    }

    fn load_matrix_session(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<matrix_desktop_key::StoredMatrixSession, matrix_desktop_key::LocalSecretError> {
        match self.primary.load_matrix_session(key_id) {
            Ok(session) => Ok(session),
            Err(error) if matrix_desktop_key::is_missing_credential_error(&error) => {
                let Some(legacy) = &self.legacy else {
                    return Err(error);
                };
                match legacy.load_matrix_session(key_id) {
                    Ok(session) => {
                        self.primary.save_matrix_session(key_id, &session)?;
                        let _ = legacy.delete_matrix_session(key_id);
                        Ok(session)
                    }
                    Err(legacy_error)
                        if matrix_desktop_key::is_missing_credential_error(&legacy_error) =>
                    {
                        Err(error)
                    }
                    Err(legacy_error) => Err(legacy_error),
                }
            }
            Err(error) => Err(error),
        }
    }

    fn delete_matrix_session(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<(), matrix_desktop_key::LocalSecretError> {
        let primary = self.primary.delete_matrix_session(key_id);
        let legacy = self
            .legacy
            .as_ref()
            .map(|store| store.delete_matrix_session(key_id));
        primary?;
        if let Some(result) = legacy {
            result?;
        }
        Ok(())
    }

    fn save_last_session(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<(), matrix_desktop_key::LocalSecretError> {
        self.primary.save_last_session(key_id)?;
        if let Some(legacy) = &self.legacy {
            let _ = legacy.delete_last_session();
        }
        Ok(())
    }

    fn load_last_session(
        &self,
    ) -> Result<Option<SessionKeyId>, matrix_desktop_key::LocalSecretError> {
        if let Some(key_id) = self.primary.load_last_session()? {
            return Ok(Some(key_id));
        }
        let Some(legacy) = &self.legacy else {
            return Ok(None);
        };
        let Some(key_id) = legacy.load_last_session()? else {
            return Ok(None);
        };
        self.primary.save_last_session(&key_id)?;
        let _ = legacy.delete_last_session();
        Ok(Some(key_id))
    }

    fn delete_last_session(&self) -> Result<(), matrix_desktop_key::LocalSecretError> {
        let primary = self.primary.delete_last_session();
        let legacy = self
            .legacy
            .as_ref()
            .map(|store| store.delete_last_session());
        primary?;
        if let Some(result) = legacy {
            result?;
        }
        Ok(())
    }

    fn load_saved_sessions(
        &self,
    ) -> Result<matrix_desktop_key::SavedSessionIndex, matrix_desktop_key::LocalSecretError> {
        let mut index = self.primary.load_saved_sessions()?;
        let Some(legacy) = &self.legacy else {
            return Ok(index);
        };
        let legacy_index = legacy.load_saved_sessions()?;
        if legacy_index.sessions().is_empty() {
            return Ok(index);
        }
        for key_id in legacy_index.sessions() {
            index.upsert(key_id.clone());
        }
        self.primary.save_saved_sessions(&index)?;
        let _ = legacy.delete_saved_sessions();
        Ok(index)
    }

    fn remember_saved_session(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<(), matrix_desktop_key::LocalSecretError> {
        let mut index = self.load_saved_sessions()?;
        index.upsert(key_id.clone());
        self.primary.save_saved_sessions(&index)?;
        if let Some(legacy) = &self.legacy {
            let _ = legacy.delete_saved_sessions();
        }
        Ok(())
    }

    fn forget_saved_session(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<(), matrix_desktop_key::LocalSecretError> {
        let mut index = self.load_saved_sessions()?;
        index.remove(key_id);
        self.primary.save_saved_sessions(&index)?;
        if let Some(legacy) = &self.legacy {
            let _ = legacy.delete_saved_sessions();
        }
        Ok(())
    }
}

#[cfg(test)]
impl<B: matrix_desktop_key::CredentialBackend> OsCredentialStore<B> {
    fn with_stores(primary: CredentialStore<B>, legacy: Option<CredentialStore<B>>) -> Self {
        Self { primary, legacy }
    }
}

fn local_secret_error_health(
    error: &matrix_desktop_key::LocalSecretError,
) -> LocalEncryptionHealth {
    if matrix_desktop_key::is_missing_credential_error(error) {
        return LocalEncryptionHealth::MissingCredential;
    }
    if matrix_desktop_key::is_locked_or_inaccessible_error(error) {
        return LocalEncryptionHealth::LockedOrInaccessible;
    }
    match error {
        matrix_desktop_key::LocalSecretError::CredentialBackend(
            matrix_desktop_key::CredentialBackendErrorKind::Unavailable,
        )
        | matrix_desktop_key::LocalSecretError::CredentialStore(keyring::Error::PlatformFailure(
            _,
        ))
        | matrix_desktop_key::LocalSecretError::CredentialStore(keyring::Error::TooLong(_, _))
        | matrix_desktop_key::LocalSecretError::CredentialStore(keyring::Error::Invalid(_, _)) => {
            LocalEncryptionHealth::Unavailable
        }
        matrix_desktop_key::LocalSecretError::CredentialStore(keyring::Error::NoStorageAccess(
            _,
        )) => LocalEncryptionHealth::LockedOrInaccessible,
        matrix_desktop_key::LocalSecretError::CredentialBackend(
            matrix_desktop_key::CredentialBackendErrorKind::Corrupt,
        )
        | matrix_desktop_key::LocalSecretError::Base64Decode(_)
        | matrix_desktop_key::LocalSecretError::InvalidSecretLength { .. }
        | matrix_desktop_key::LocalSecretError::Json(_)
        | matrix_desktop_key::LocalSecretError::Derivation
        | matrix_desktop_key::LocalSecretError::CredentialStore(keyring::Error::BadEncoding(_))
        | matrix_desktop_key::LocalSecretError::CredentialStore(keyring::Error::Ambiguous(_)) => {
            LocalEncryptionHealth::ResetRequired
        }
        matrix_desktop_key::LocalSecretError::CredentialStore(keyring::Error::NoEntry)
        | matrix_desktop_key::LocalSecretError::CredentialBackend(
            matrix_desktop_key::CredentialBackendErrorKind::MissingCredential,
        ) => LocalEncryptionHealth::MissingCredential,
        matrix_desktop_key::LocalSecretError::CredentialBackend(
            matrix_desktop_key::CredentialBackendErrorKind::LockedOrInaccessible,
        ) => LocalEncryptionHealth::LockedOrInaccessible,
        _ => LocalEncryptionHealth::Unavailable,
    }
}

// --- File-based credential store (debug/test only) ---

/// A trivial file-based credential store used in unattended QA runs that
/// cannot prompt macOS Keychain. Stored as plain files under `dir`; each
/// entry is a separate file named after the account.
///
/// COMPILE-TIME GATE: only present in debug/test builds.
/// Release builds must not include this type.
#[cfg(any(debug_assertions, test))]
pub struct FileCredentialStore {
    dir: PathBuf,
}

#[cfg(any(debug_assertions, test))]
impl FileCredentialStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn account_file(&self, key_id: &SessionKeyId) -> PathBuf {
        // Use base64url-encoded account name as filename to stay FS-safe.
        self.dir.join(safe_filename(key_id.account_name()))
    }

    fn named_file(&self, name: &str) -> PathBuf {
        self.dir.join(safe_filename(name.to_owned()))
    }

    fn load(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<LocalUnlockSecret, matrix_desktop_key::LocalSecretError> {
        let path = self.account_file(key_id);
        let value = std::fs::read_to_string(&path).map_err(|_| {
            matrix_desktop_key::LocalSecretError::CredentialStore(keyring::Error::NoEntry)
        })?;
        LocalUnlockSecret::from_storage_string(value.trim())
    }

    fn save(
        &self,
        key_id: &SessionKeyId,
        secret: &LocalUnlockSecret,
    ) -> Result<(), matrix_desktop_key::LocalSecretError> {
        self.ensure_dir()?;
        let path = self.account_file(key_id);
        let storage_string = secret.to_storage_string();
        self.write_file(&path, storage_string.as_str())
    }

    fn delete(&self, key_id: &SessionKeyId) -> Result<(), matrix_desktop_key::LocalSecretError> {
        let path = self.account_file(key_id);
        let _ = std::fs::remove_file(&path);
        Ok(())
    }

    /// Save an arbitrary named credential (used for session JSON, last-session
    /// pointer, etc.).
    pub(super) fn save_named(
        &self,
        name: &str,
        value: &str,
    ) -> Result<(), matrix_desktop_key::LocalSecretError> {
        self.ensure_dir()?;
        self.write_file(&self.named_file(name), value)
    }

    /// Load an arbitrary named credential.
    pub(super) fn load_named(
        &self,
        name: &str,
    ) -> Result<String, matrix_desktop_key::LocalSecretError> {
        let path = self.named_file(name);
        std::fs::read_to_string(&path).map_err(|_| {
            matrix_desktop_key::LocalSecretError::CredentialStore(keyring::Error::NoEntry)
        })
    }

    /// Delete an arbitrary named credential (no error if absent).
    pub(super) fn delete_named(
        &self,
        name: &str,
    ) -> Result<(), matrix_desktop_key::LocalSecretError> {
        let _ = std::fs::remove_file(self.named_file(name));
        Ok(())
    }

    fn ensure_dir(&self) -> Result<(), matrix_desktop_key::LocalSecretError> {
        std::fs::create_dir_all(&self.dir).map_err(|e| {
            matrix_desktop_key::LocalSecretError::CredentialStore(keyring::Error::PlatformFailure(
                Box::new(e),
            ))
        })
    }

    fn write_file(
        &self,
        path: &std::path::Path,
        value: &str,
    ) -> Result<(), matrix_desktop_key::LocalSecretError> {
        std::fs::write(path, value).map_err(|e| {
            matrix_desktop_key::LocalSecretError::CredentialStore(keyring::Error::PlatformFailure(
                Box::new(e),
            ))
        })
    }
}

/// Make a name filesystem-safe by replacing all non-alphanumeric chars with `_`.
#[cfg(any(debug_assertions, test))]
fn safe_filename(name: String) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Debug/test-only diagnostic helper. Compiled out of release builds along
/// with its only call site (the file credential store branch in
/// `CredentialStoreBackend::resolve`).
#[cfg(any(debug_assertions, test))]
fn tracing_or_eprintln(message: &str) {
    // Use eprintln as a simple diagnostic; in production the tracing crate
    // should be wired instead.
    if std::env::var_os("MATRIX_DESKTOP_DEBUG_SDK_ERROR").is_some() {
        eprintln!("[matrix-desktop-core] {message}");
    }
}

/// QA/debug structural guard: true only when the env-resolved credential
/// store backend is the file-dir backend (i.e.
/// `MATRIX_DESKTOP_QA_FILE_CREDENTIAL_STORE_DIR` is set in a debug/test
/// build). Headless QA binaries call this BEFORE any login so unattended runs
/// are structurally unable to reach the OS keychain (engineering-rules
/// Secrets rule: keychain prompts during automation are failures).
///
/// Debug/test only: release builds have no file backend, so this symbol does
/// not exist there and a release-built QA guard cannot silently pass.
#[cfg(any(debug_assertions, test))]
pub fn resolved_credential_backend_is_file_dir() -> bool {
    matches!(
        CredentialStoreBackend::resolve(),
        CredentialStoreBackend::FileDir(_)
    )
}

/// Convert a `SessionInfo` (from matrix-desktop-state) into a `SessionKeyId`
/// (from matrix-desktop-key). This is the canonical mapping used everywhere
/// in the codebase.
pub fn session_key_id_from_info(info: &matrix_desktop_state::SessionInfo) -> SessionKeyId {
    SessionKeyId {
        homeserver: info.homeserver.clone(),
        user_id: info.user_id.clone(),
        device_id: info.device_id.clone(),
    }
}

/// Derive a canonical `AccountKey` string for a session. The account key is
/// the user's Matrix ID — e.g. `@alice:example.com`.
pub fn account_key_from_info(info: &matrix_desktop_state::SessionInfo) -> crate::ids::AccountKey {
    crate::ids::AccountKey(info.user_id.clone())
}

// ---- Tests ----

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_key_id() -> SessionKeyId {
        SessionKeyId {
            homeserver: "https://test.example.com".to_owned(),
            user_id: "@alice:test.example.com".to_owned(),
            device_id: "DEVICE1".to_owned(),
        }
    }

    fn file_store_actor(data_dir: &tempfile::TempDir, cred_dir: &tempfile::TempDir) -> StoreActor {
        StoreActor {
            credential_store: CredentialStoreBackend::FileDir(FileCredentialStore::new(
                cred_dir.path(),
            )),
            data_dir: data_dir.path().to_path_buf(),
        }
    }

    #[test]
    fn file_credential_store_round_trip() {
        let dir = tempdir().expect("tempdir");
        let store = FileCredentialStore::new(dir.path());
        let key_id = make_key_id();

        // Not found initially.
        let result = store.load(&key_id);
        assert!(matrix_desktop_key::is_missing_credential_error(
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

        // Calling again yields a consistent store path (same key_id).
        let config2 = actor.account_store_config(&key_id).expect("second call");
        assert_eq!(config.store_config.path(), config2.store_config.path());
    }

    #[test]
    fn delete_account_credentials_does_not_panic_when_absent() {
        let data_dir = tempdir().expect("tempdir");
        let cred_dir = tempdir().expect("tempdir");
        let key_id = make_key_id();

        let actor = file_store_actor(&data_dir, &cred_dir);

        // Should not panic even when credentials don't exist.
        actor.delete_account_credentials(&key_id);
    }

    #[test]
    fn store_actor_probe_maps_credential_backend_health_without_raw_errors() {
        let data_dir = tempdir().expect("tempdir");
        let backend = matrix_desktop_key::InMemoryCredentialBackend::default();
        let actor = StoreActor::with_backend(
            CredentialStoreBackend::InMemory(matrix_desktop_key::CredentialStore::with_backend(
                "matrix-desktop-test",
                backend.clone(),
            )),
            data_dir.path(),
        );
        let key_id = make_key_id();

        assert_eq!(
            actor.probe_local_encryption_health(&key_id),
            matrix_desktop_state::LocalEncryptionHealth::MissingCredential
        );

        let secret = LocalUnlockSecret::generate();
        actor
            .credential_backend()
            .save(&key_id, &secret)
            .expect("save synthetic unlock secret");
        assert_eq!(
            actor.probe_local_encryption_health(&key_id),
            matrix_desktop_state::LocalEncryptionHealth::Healthy
        );

        backend.set_error(matrix_desktop_key::CredentialBackendErrorKind::LockedOrInaccessible);
        assert_eq!(
            actor.probe_local_encryption_health(&key_id),
            matrix_desktop_state::LocalEncryptionHealth::LockedOrInaccessible
        );
    }

    #[test]
    fn os_keychain_service_name_is_product_branded() {
        assert_eq!(CREDENTIAL_STORE_SERVICE_NAME, "Kagome");
    }

    #[test]
    fn os_keychain_load_migrates_legacy_unlock_secret_to_product_service() {
        let backend = matrix_desktop_key::InMemoryCredentialBackend::default();
        let primary = matrix_desktop_key::CredentialStore::with_backend(
            CREDENTIAL_STORE_SERVICE_NAME,
            backend.clone(),
        );
        let legacy = matrix_desktop_key::CredentialStore::with_backend(
            LEGACY_CREDENTIAL_STORE_SERVICE_NAME,
            backend.clone(),
        );
        let store = OsCredentialStore::with_stores(primary, Some(legacy));
        let key_id = make_key_id();
        let secret = LocalUnlockSecret::generate();

        let legacy_probe = matrix_desktop_key::CredentialStore::with_backend(
            LEGACY_CREDENTIAL_STORE_SERVICE_NAME,
            backend.clone(),
        );
        legacy_probe
            .save(&key_id, &secret)
            .expect("seed legacy unlock secret");

        let loaded = store.load(&key_id).expect("migrate legacy secret");
        assert_eq!(
            secret.derive_sdk_store_key().as_bytes(),
            loaded.derive_sdk_store_key().as_bytes()
        );

        let primary_probe = matrix_desktop_key::CredentialStore::with_backend(
            CREDENTIAL_STORE_SERVICE_NAME,
            backend,
        );
        primary_probe
            .load(&key_id)
            .expect("primary product service should receive migrated secret");
        let legacy_missing = legacy_probe
            .load(&key_id)
            .expect_err("legacy service secret should be removed");
        assert!(matrix_desktop_key::is_missing_credential_error(
            &legacy_missing
        ));
    }

    #[test]
    fn composer_drafts_are_encrypted_and_reject_corruption() {
        let data_dir = tempdir().expect("tempdir");
        let cred_dir = tempdir().expect("tempdir");
        let key_id = make_key_id();
        let actor = file_store_actor(&data_dir, &cred_dir);
        let plaintext = "secret draft body";
        let mut drafts = ComposerDraftStore::default();
        drafts.set_room_draft("!room:test.example.com".to_owned(), plaintext.to_owned());

        actor
            .save_composer_drafts(&key_id, &drafts)
            .expect("save encrypted drafts");

        let path = actor.account_composer_drafts_file(&key_id);
        let bytes = std::fs::read(&path).expect("read encrypted drafts");
        assert!(
            !bytes
                .windows(plaintext.len())
                .any(|window| window == plaintext.as_bytes())
        );

        let loaded = actor
            .load_composer_drafts(&key_id)
            .expect("load encrypted drafts");
        assert_eq!(
            loaded
                .rooms
                .get("!room:test.example.com")
                .map(String::as_str),
            Some(plaintext)
        );

        let mut corrupted = bytes;
        let last = corrupted.last_mut().expect("non-empty encrypted payload");
        *last ^= 0x01;
        std::fs::write(&path, corrupted).expect("write corrupted drafts");
        assert!(matches!(
            actor.load_composer_drafts(&key_id),
            Err(CoreFailure::StoreUnavailable)
        ));
    }

    #[test]
    fn loading_composer_drafts_does_not_create_missing_credentials() {
        let data_dir = tempdir().expect("tempdir");
        let cred_dir = tempdir().expect("tempdir");
        let key_id = make_key_id();
        let actor = file_store_actor(&data_dir, &cred_dir);
        let path = actor.account_composer_drafts_file(&key_id);
        std::fs::create_dir_all(path.parent().expect("draft parent")).expect("create parent");
        std::fs::write(&path, COMPOSER_DRAFTS_FILE_MAGIC).expect("write draft placeholder");

        assert!(matches!(
            actor.load_composer_drafts(&key_id),
            Err(CoreFailure::LocalEncryptionUnavailable)
        ));
        let missing = actor.credential_backend().load(&key_id).unwrap_err();
        assert!(matrix_desktop_key::is_missing_credential_error(&missing));
    }

    #[test]
    fn empty_composer_drafts_remove_persisted_file() {
        let data_dir = tempdir().expect("tempdir");
        let cred_dir = tempdir().expect("tempdir");
        let key_id = make_key_id();
        let actor = file_store_actor(&data_dir, &cred_dir);
        let mut drafts = ComposerDraftStore::default();
        drafts.set_room_draft("!room:test.example.com".to_owned(), "draft".to_owned());

        actor
            .save_composer_drafts(&key_id, &drafts)
            .expect("save non-empty drafts");
        let path = actor.account_composer_drafts_file(&key_id);
        assert!(path.exists());

        actor
            .save_composer_drafts(&key_id, &ComposerDraftStore::default())
            .expect("save empty drafts");
        assert!(!path.exists());
        assert!(
            actor
                .load_composer_drafts(&key_id)
                .expect("load removed drafts")
                .is_empty()
        );
    }

    #[test]
    fn composer_draft_persistence_applies_entry_and_size_bounds() {
        let data_dir = tempdir().expect("tempdir");
        let cred_dir = tempdir().expect("tempdir");
        let key_id = make_key_id();
        let actor = file_store_actor(&data_dir, &cred_dir);
        let mut drafts = ComposerDraftStore::default();
        let oversized =
            "x".repeat(matrix_desktop_state::state::MAX_PERSISTED_COMPOSER_DRAFT_BYTES + 64);

        for index in 0..(matrix_desktop_state::state::MAX_PERSISTED_COMPOSER_DRAFT_ROOM_COUNT + 8) {
            drafts.set_room_draft(format!("!room-{index}:test.example.com"), oversized.clone());
        }
        for index in 0..(matrix_desktop_state::state::MAX_PERSISTED_COMPOSER_DRAFT_THREAD_COUNT + 8)
        {
            drafts.set_thread_draft(
                "!thread-room:test.example.com".to_owned(),
                format!("$root-{index}"),
                oversized.clone(),
            );
        }

        actor
            .save_composer_drafts(&key_id, &drafts)
            .expect("save bounded drafts");
        let loaded = actor
            .load_composer_drafts(&key_id)
            .expect("load bounded drafts");

        assert!(
            loaded.rooms.len()
                <= matrix_desktop_state::state::MAX_PERSISTED_COMPOSER_DRAFT_ROOM_COUNT
        );
        assert!(
            loaded.rooms.values().all(|draft| draft.len()
                <= matrix_desktop_state::state::MAX_PERSISTED_COMPOSER_DRAFT_BYTES)
        );
        let thread_count = loaded
            .threads
            .values()
            .map(std::collections::BTreeMap::len)
            .sum::<usize>();
        assert!(
            thread_count <= matrix_desktop_state::state::MAX_PERSISTED_COMPOSER_DRAFT_THREAD_COUNT
        );
        assert!(
            loaded
                .threads
                .values()
                .flat_map(|room_threads| room_threads.values())
                .all(|draft| draft.len()
                    <= matrix_desktop_state::state::MAX_PERSISTED_COMPOSER_DRAFT_BYTES)
        );
    }
}
