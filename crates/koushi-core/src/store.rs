//! StoreActor: credential store access, per-account store paths, store/search
//! key derivation, and debug/test credential injection policy.
//!
//! Security invariants:
//! - Store and search keys NEVER cross the command/event boundary.
//! - If credential store or encryption cannot be initialized for an account,
//!   `LocalEncryptionUnavailable` is returned (fail-closed).
//! - The file-based credential store override is behind a compile-time gate:
//!   `#[cfg(any(debug_assertions, test, feature = "qa-bin"))]` only.
//!
//! Architecture: overview.md Platform Portability rule 3 — platform
//! capabilities live here behind a port. StoreActor is the only actor allowed
//! platform-conditional code.

use std::path::PathBuf;
use std::sync::Arc;

use chacha20poly1305::{
    ChaCha20Poly1305, Key, KeyInit, Nonce,
    aead::{Aead, OsRng, rand_core::RngCore},
};
use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_key::{CredentialStore, LocalUnlockSecret, SessionKeyId};
use koushi_sdk::{
    MatrixClientStoreConfig, MatrixClientStoreKey, MatrixSearchIndexKey,
    MatrixSearchIndexStoreConfig,
};
#[cfg(test)]
use koushi_state::RoomPreference;
use koushi_state::{
    ComposerDraftStore, LocalEncryptionHealth, NavigationState, RoomPreferencesState,
    ScheduledSendStore,
};

use crate::failure::CoreFailure;

/// Service name used for OS keyring entries. This is user-visible in macOS
/// Keychain Access, so keep it aligned with the shipped product name.
const CREDENTIAL_STORE_SERVICE_NAME: &str = "koushi-desktop";
const COMPOSER_DRAFTS_FILE_MAGIC: &[u8] = b"KOUSHI-DRAFTS-V1\0";
const SCHEDULED_SENDS_FILE_MAGIC: &[u8] = b"KOUSHI-SCHEDULED-SENDS-V1\0";
const NAVIGATION_FILE_MAGIC: &[u8] = b"KOUSHI-NAVIGATION-V1\0";
const ROOM_PREFERENCES_FILE_MAGIC: &[u8] = b"KOUSHI-ROOM-PREFERENCES-V1\0";
const COMPOSER_DRAFTS_NONCE_LEN: usize = 12;

/// Env var for QA/debug file-based credential store override.
/// Only honored in debug/test/qa-bin builds; production release builds ignore it.
#[cfg(any(debug_assertions, test, feature = "qa-bin"))]
const ENV_FILE_CREDENTIAL_STORE_DIR: &str = "KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR";

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
#[derive(Clone)]
pub struct StoreActor {
    pub(crate) credential_store: CredentialStoreBackend,
    data_dir: PathBuf,
}

impl StoreActor {
    /// Create the actor. `data_dir` is the application data directory under
    /// which per-account sub-directories are created.
    ///
    /// Uses the **in-memory** credential store by default (keyring-free).
    /// Production builds must use `with_os_backend` to inject the OS adapter.
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            credential_store: CredentialStoreBackend::resolve(),
            data_dir: data_dir.into(),
        }
    }

    /// Create the actor with an injected OS credential store backend.
    /// Used by production `CoreRuntime::start_with_data_dir_and_os_backend`.
    pub fn with_os_backend(
        data_dir: impl Into<PathBuf>,
        os_backend: Arc<dyn koushi_key::CredentialBackend>,
    ) -> Self {
        Self {
            credential_store: CredentialStoreBackend::resolve_with_os_backend(os_backend),
            data_dir: data_dir.into(),
        }
    }

    /// Access the credential store backend (for session persistence in AccountActor).
    pub fn credential_backend(&self) -> &CredentialStoreBackend {
        &self.credential_store
    }

    /// Test-only constructor with an explicit backend (avoids the env-global
    /// `KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR` race between unit tests).
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

    pub fn load_scheduled_sends(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<ScheduledSendStore, CoreFailure> {
        let path = self.account_scheduled_sends_file(key_id);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ScheduledSendStore::default());
            }
            Err(_) => return Err(CoreFailure::StoreUnavailable),
        };
        decrypt_scheduled_sends_payload(&self.load_unlock_secret(key_id)?, &bytes)
    }

    pub fn save_scheduled_sends(
        &self,
        key_id: &SessionKeyId,
        scheduled_sends: &ScheduledSendStore,
    ) -> Result<(), CoreFailure> {
        let path = self.account_scheduled_sends_file(key_id);
        if scheduled_sends.items.is_empty() {
            match std::fs::remove_file(&path) {
                Ok(()) => return Ok(()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(_) => return Err(CoreFailure::StoreUnavailable),
            }
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| CoreFailure::StoreUnavailable)?;
        }
        let payload = encrypt_scheduled_sends_payload(
            &self.load_or_create_unlock_secret(key_id)?,
            scheduled_sends,
        )?;
        std::fs::write(path, payload).map_err(|_| CoreFailure::StoreUnavailable)
    }

    pub fn load_navigation(&self, key_id: &SessionKeyId) -> Result<NavigationState, CoreFailure> {
        let path = self.account_navigation_file(key_id);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return self.load_legacy_navigation(key_id);
            }
            Err(_) => return Err(CoreFailure::StoreUnavailable),
        };
        decrypt_navigation_payload(&self.load_unlock_secret(key_id)?, &bytes)
    }

    pub fn save_navigation(
        &self,
        key_id: &SessionKeyId,
        navigation: &NavigationState,
    ) -> Result<(), CoreFailure> {
        let path = self.account_navigation_file(key_id);
        let legacy_path = self.account_navigation_legacy_file(key_id);
        if navigation == &NavigationState::default() {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(CoreFailure::StoreUnavailable),
            }
            match std::fs::remove_file(&legacy_path) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(CoreFailure::StoreUnavailable),
            }
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| CoreFailure::StoreUnavailable)?;
        }
        let payload =
            encrypt_navigation_payload(&self.load_or_create_unlock_secret(key_id)?, navigation)?;
        std::fs::write(path, payload).map_err(|_| CoreFailure::StoreUnavailable)?;
        match std::fs::remove_file(&legacy_path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(CoreFailure::StoreUnavailable),
        }
    }

    pub fn load_room_preferences(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<RoomPreferencesState, CoreFailure> {
        let path = self.account_room_preferences_file(key_id);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(RoomPreferencesState::default());
            }
            Err(_) => return Err(CoreFailure::StoreUnavailable),
        };
        decrypt_room_preferences_payload(&self.load_unlock_secret(key_id)?, &bytes)
    }

    pub fn save_room_preferences(
        &self,
        key_id: &SessionKeyId,
        preferences: &RoomPreferencesState,
    ) -> Result<(), CoreFailure> {
        let path = self.account_room_preferences_file(key_id);
        if preferences == &RoomPreferencesState::default() {
            match std::fs::remove_file(&path) {
                Ok(()) => return Ok(()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(_) => return Err(CoreFailure::StoreUnavailable),
            }
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| CoreFailure::StoreUnavailable)?;
        }
        let payload = encrypt_room_preferences_payload(
            &self.load_or_create_unlock_secret(key_id)?,
            preferences,
        )?;
        std::fs::write(path, payload).map_err(|_| CoreFailure::StoreUnavailable)
    }

    fn load_legacy_navigation(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<NavigationState, CoreFailure> {
        let path = self.account_navigation_legacy_file(key_id);
        let json = match std::fs::read_to_string(&path) {
            Ok(json) => json,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(NavigationState::default());
            }
            Err(_) => return Err(CoreFailure::StoreUnavailable),
        };
        serde_json::from_str(&json).map_err(|_| CoreFailure::StoreUnavailable)
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
            Err(err) if koushi_key::is_missing_credential_error(&err) => {
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

    fn account_scheduled_sends_file(&self, key_id: &SessionKeyId) -> PathBuf {
        self.account_root_dir(key_id)
            .join("scheduled-sends")
            .join("scheduled.v1.enc")
    }

    fn account_navigation_file(&self, key_id: &SessionKeyId) -> PathBuf {
        self.account_root_dir(key_id)
            .join("navigation")
            .join("navigation.v1.enc")
    }

    fn account_navigation_legacy_file(&self, key_id: &SessionKeyId) -> PathBuf {
        self.account_root_dir(key_id)
            .join("navigation")
            .join("navigation.v1.json")
    }

    fn account_room_preferences_file(&self, key_id: &SessionKeyId) -> PathBuf {
        self.account_root_dir(key_id)
            .join("room-preferences")
            .join("preferences.v1.enc")
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

fn encrypt_scheduled_sends_payload(
    secret: &LocalUnlockSecret,
    scheduled_sends: &ScheduledSendStore,
) -> Result<Vec<u8>, CoreFailure> {
    let plaintext =
        serde_json::to_vec(scheduled_sends).map_err(|_| CoreFailure::StoreUnavailable)?;
    let key = secret.derive_scheduled_sends_key();
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key.as_bytes()));
    let mut nonce_bytes = [0_u8; COMPOSER_DRAFTS_NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_ref())
        .map_err(|_| CoreFailure::StoreUnavailable)?;
    let mut payload = Vec::with_capacity(
        SCHEDULED_SENDS_FILE_MAGIC.len() + COMPOSER_DRAFTS_NONCE_LEN + ciphertext.len(),
    );
    payload.extend_from_slice(SCHEDULED_SENDS_FILE_MAGIC);
    payload.extend_from_slice(&nonce_bytes);
    payload.extend_from_slice(&ciphertext);
    Ok(payload)
}

fn decrypt_scheduled_sends_payload(
    secret: &LocalUnlockSecret,
    payload: &[u8],
) -> Result<ScheduledSendStore, CoreFailure> {
    let header_len = SCHEDULED_SENDS_FILE_MAGIC.len() + COMPOSER_DRAFTS_NONCE_LEN;
    if payload.len() < header_len || !payload.starts_with(SCHEDULED_SENDS_FILE_MAGIC) {
        return Err(CoreFailure::StoreUnavailable);
    }
    let nonce_start = SCHEDULED_SENDS_FILE_MAGIC.len();
    let nonce_end = nonce_start + COMPOSER_DRAFTS_NONCE_LEN;
    let nonce = Nonce::from_slice(&payload[nonce_start..nonce_end]);
    let key = secret.derive_scheduled_sends_key();
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key.as_bytes()));
    let plaintext = cipher
        .decrypt(nonce, &payload[nonce_end..])
        .map_err(|_| CoreFailure::StoreUnavailable)?;
    serde_json::from_slice(&plaintext).map_err(|_| CoreFailure::StoreUnavailable)
}

fn encrypt_navigation_payload(
    secret: &LocalUnlockSecret,
    navigation: &NavigationState,
) -> Result<Vec<u8>, CoreFailure> {
    let plaintext = serde_json::to_vec(navigation).map_err(|_| CoreFailure::StoreUnavailable)?;
    let key = secret.derive_navigation_key();
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key.as_bytes()));
    let mut nonce_bytes = [0_u8; COMPOSER_DRAFTS_NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_ref())
        .map_err(|_| CoreFailure::StoreUnavailable)?;
    let mut payload = Vec::with_capacity(
        NAVIGATION_FILE_MAGIC.len() + COMPOSER_DRAFTS_NONCE_LEN + ciphertext.len(),
    );
    payload.extend_from_slice(NAVIGATION_FILE_MAGIC);
    payload.extend_from_slice(&nonce_bytes);
    payload.extend_from_slice(&ciphertext);
    Ok(payload)
}

fn decrypt_navigation_payload(
    secret: &LocalUnlockSecret,
    payload: &[u8],
) -> Result<NavigationState, CoreFailure> {
    let header_len = NAVIGATION_FILE_MAGIC.len() + COMPOSER_DRAFTS_NONCE_LEN;
    if payload.len() < header_len || !payload.starts_with(NAVIGATION_FILE_MAGIC) {
        return Err(CoreFailure::StoreUnavailable);
    }
    let nonce_start = NAVIGATION_FILE_MAGIC.len();
    let nonce_end = nonce_start + COMPOSER_DRAFTS_NONCE_LEN;
    let nonce = Nonce::from_slice(&payload[nonce_start..nonce_end]);
    let key = secret.derive_navigation_key();
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key.as_bytes()));
    let plaintext = cipher
        .decrypt(nonce, &payload[nonce_end..])
        .map_err(|_| CoreFailure::StoreUnavailable)?;
    serde_json::from_slice(&plaintext).map_err(|_| CoreFailure::StoreUnavailable)
}

fn encrypt_room_preferences_payload(
    secret: &LocalUnlockSecret,
    preferences: &RoomPreferencesState,
) -> Result<Vec<u8>, CoreFailure> {
    let plaintext = serde_json::to_vec(preferences).map_err(|_| CoreFailure::StoreUnavailable)?;
    let key = secret.derive_room_preferences_key();
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key.as_bytes()));
    let mut nonce_bytes = [0_u8; COMPOSER_DRAFTS_NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_ref())
        .map_err(|_| CoreFailure::StoreUnavailable)?;
    let mut payload = Vec::with_capacity(
        ROOM_PREFERENCES_FILE_MAGIC.len() + COMPOSER_DRAFTS_NONCE_LEN + ciphertext.len(),
    );
    payload.extend_from_slice(ROOM_PREFERENCES_FILE_MAGIC);
    payload.extend_from_slice(&nonce_bytes);
    payload.extend_from_slice(&ciphertext);
    Ok(payload)
}

fn decrypt_room_preferences_payload(
    secret: &LocalUnlockSecret,
    payload: &[u8],
) -> Result<RoomPreferencesState, CoreFailure> {
    let header_len = ROOM_PREFERENCES_FILE_MAGIC.len() + COMPOSER_DRAFTS_NONCE_LEN;
    if payload.len() < header_len || !payload.starts_with(ROOM_PREFERENCES_FILE_MAGIC) {
        return Err(CoreFailure::StoreUnavailable);
    }
    let nonce_start = ROOM_PREFERENCES_FILE_MAGIC.len();
    let nonce_end = nonce_start + COMPOSER_DRAFTS_NONCE_LEN;
    let nonce = Nonce::from_slice(&payload[nonce_start..nonce_end]);
    let key = secret.derive_room_preferences_key();
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

/// Credential store backend. Production = either OS keychain (injected from
/// the platform layer) or in-memory; debug/test/qa-bin may use a file dir
/// override when `KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR` is set.
#[derive(Clone)]
pub enum CredentialStoreBackend {
    OsKeychain(OsCredentialStore),
    #[cfg(any(debug_assertions, test, feature = "qa-bin"))]
    FileDir(FileCredentialStore),
    InMemory(CredentialStore<koushi_key::InMemoryCredentialBackend>),
}

impl CredentialStoreBackend {
    fn resolve() -> Self {
        #[cfg(any(debug_assertions, test, feature = "qa-bin"))]
        if let Ok(dir) = std::env::var(ENV_FILE_CREDENTIAL_STORE_DIR) {
            let dir = PathBuf::from(dir);
            tracing_or_eprintln("file credential store active (debug/test/qa-bin only)");
            return Self::FileDir(FileCredentialStore::new(dir));
        }
        Self::InMemory(CredentialStore::with_backend(
            CREDENTIAL_STORE_SERVICE_NAME,
            koushi_key::InMemoryCredentialBackend::default(),
        ))
    }

    fn resolve_with_os_backend(os_backend: Arc<dyn koushi_key::CredentialBackend>) -> Self {
        #[cfg(any(debug_assertions, test, feature = "qa-bin"))]
        if let Ok(dir) = std::env::var(ENV_FILE_CREDENTIAL_STORE_DIR) {
            let dir = PathBuf::from(dir);
            tracing_or_eprintln("file credential store active (debug/test/qa-bin only)");
            return Self::FileDir(FileCredentialStore::new(dir));
        }
        Self::OsKeychain(OsCredentialStore::with_backend(os_backend))
    }

    fn load(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<LocalUnlockSecret, koushi_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.load(key_id),
            #[cfg(any(debug_assertions, test, feature = "qa-bin"))]
            Self::FileDir(store) => store.load(key_id),
            Self::InMemory(store) => store.load(key_id),
        }
    }

    fn save(
        &self,
        key_id: &SessionKeyId,
        secret: &LocalUnlockSecret,
    ) -> Result<(), koushi_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.save(key_id, secret),
            #[cfg(any(debug_assertions, test, feature = "qa-bin"))]
            Self::FileDir(store) => store.save(key_id, secret),
            Self::InMemory(store) => store.save(key_id, secret),
        }
    }

    fn delete(&self, key_id: &SessionKeyId) -> Result<(), koushi_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.delete(key_id),
            #[cfg(any(debug_assertions, test, feature = "qa-bin"))]
            Self::FileDir(store) => store.delete(key_id),
            Self::InMemory(store) => store.delete(key_id),
        }
    }

    // --- Session persistence operations ---
    // These mirror the CredentialStore API so AccountActor can operate against
    // both backends without knowing which is active.

    pub fn save_matrix_session(
        &self,
        key_id: &SessionKeyId,
        session: &koushi_key::StoredMatrixSession,
    ) -> Result<(), koushi_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.save_matrix_session(key_id, session),
            #[cfg(any(debug_assertions, test, feature = "qa-bin"))]
            Self::FileDir(store) => {
                store.save_named(&key_id.matrix_session_account_name(), session.as_str())
            }
            Self::InMemory(store) => store.save_matrix_session(key_id, session),
        }
    }

    pub fn load_matrix_session(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<koushi_key::StoredMatrixSession, koushi_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.load_matrix_session(key_id),
            #[cfg(any(debug_assertions, test, feature = "qa-bin"))]
            Self::FileDir(store) => {
                let value = store.load_named(&key_id.matrix_session_account_name())?;
                Ok(koushi_key::StoredMatrixSession::new(value))
            }
            Self::InMemory(store) => store.load_matrix_session(key_id),
        }
    }

    pub fn delete_matrix_session(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<(), koushi_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.delete_matrix_session(key_id),
            #[cfg(any(debug_assertions, test, feature = "qa-bin"))]
            Self::FileDir(store) => store.delete_named(&key_id.matrix_session_account_name()),
            Self::InMemory(store) => store.delete_matrix_session(key_id),
        }
    }

    pub fn save_last_session(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<(), koushi_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.save_last_session(key_id),
            #[cfg(any(debug_assertions, test, feature = "qa-bin"))]
            Self::FileDir(store) => {
                let pointer = koushi_key::LastSessionPointer::new(key_id.clone());
                let json = pointer.to_json()?;
                store.save_named(koushi_key::last_session_account_name(), &json)
            }
            Self::InMemory(store) => store.save_last_session(key_id),
        }
    }

    pub fn load_last_session(&self) -> Result<Option<SessionKeyId>, koushi_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.load_last_session(),
            #[cfg(any(debug_assertions, test, feature = "qa-bin"))]
            Self::FileDir(store) => {
                match store.load_named(koushi_key::last_session_account_name()) {
                    Ok(json) => Ok(Some(
                        koushi_key::LastSessionPointer::from_json(&json)?
                            .session_key_id()
                            .clone(),
                    )),
                    Err(err) if koushi_key::is_missing_credential_error(&err) => Ok(None),
                    Err(err) => Err(err),
                }
            }
            Self::InMemory(store) => store.load_last_session(),
        }
    }

    pub fn delete_last_session(&self) -> Result<(), koushi_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.delete_last_session(),
            #[cfg(any(debug_assertions, test, feature = "qa-bin"))]
            Self::FileDir(store) => store.delete_named(koushi_key::last_session_account_name()),
            Self::InMemory(store) => store.delete_last_session(),
        }
    }

    pub fn load_saved_sessions(
        &self,
    ) -> Result<koushi_key::SavedSessionIndex, koushi_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.load_saved_sessions(),
            #[cfg(any(debug_assertions, test, feature = "qa-bin"))]
            Self::FileDir(store) => {
                match store.load_named(koushi_key::saved_sessions_account_name()) {
                    Ok(json) => koushi_key::SavedSessionIndex::from_json(&json),
                    Err(err) if koushi_key::is_missing_credential_error(&err) => {
                        Ok(koushi_key::SavedSessionIndex::new())
                    }
                    Err(err) => Err(err),
                }
            }
            Self::InMemory(store) => store.load_saved_sessions(),
        }
    }

    pub fn remember_saved_session(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<(), koushi_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.remember_saved_session(key_id),
            #[cfg(any(debug_assertions, test, feature = "qa-bin"))]
            Self::FileDir(store) => {
                let mut index = self.load_saved_sessions()?;
                index.upsert(key_id.clone());
                store.save_named(koushi_key::saved_sessions_account_name(), &index.to_json()?)
            }
            Self::InMemory(store) => store.remember_saved_session(key_id),
        }
    }

    pub fn forget_saved_session(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<(), koushi_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.forget_saved_session(key_id),
            #[cfg(any(debug_assertions, test, feature = "qa-bin"))]
            Self::FileDir(store) => {
                let mut index = self.load_saved_sessions()?;
                index.remove(key_id);
                store.save_named(koushi_key::saved_sessions_account_name(), &index.to_json()?)
            }
            Self::InMemory(store) => store.forget_saved_session(key_id),
        }
    }

    /// Expose the underlying `CredentialStore` (for OS keychain backend).
    pub fn as_os_credential_store(
        &self,
    ) -> Option<&CredentialStore<Arc<dyn koushi_key::CredentialBackend>>> {
        match self {
            Self::OsKeychain(store) => Some(store.primary()),
            #[cfg(any(debug_assertions, test, feature = "qa-bin"))]
            Self::FileDir(_) => None,
            Self::InMemory(_) => None,
        }
    }
}

/// OS keychain credential store for the shipped product service.
#[derive(Clone)]
pub struct OsCredentialStore {
    primary: CredentialStore<Arc<dyn koushi_key::CredentialBackend>>,
}

impl OsCredentialStore {
    fn with_backend(backend: Arc<dyn koushi_key::CredentialBackend>) -> Self {
        Self {
            primary: CredentialStore::with_backend(CREDENTIAL_STORE_SERVICE_NAME, backend),
        }
    }

    fn primary(&self) -> &CredentialStore<Arc<dyn koushi_key::CredentialBackend>> {
        &self.primary
    }

    fn load(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<LocalUnlockSecret, koushi_key::LocalSecretError> {
        self.primary.load(key_id)
    }

    fn save(
        &self,
        key_id: &SessionKeyId,
        secret: &LocalUnlockSecret,
    ) -> Result<(), koushi_key::LocalSecretError> {
        self.primary.save(key_id, secret)
    }

    fn delete(&self, key_id: &SessionKeyId) -> Result<(), koushi_key::LocalSecretError> {
        self.primary.delete(key_id)
    }

    fn save_matrix_session(
        &self,
        key_id: &SessionKeyId,
        session: &koushi_key::StoredMatrixSession,
    ) -> Result<(), koushi_key::LocalSecretError> {
        self.primary.save_matrix_session(key_id, session)
    }

    fn load_matrix_session(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<koushi_key::StoredMatrixSession, koushi_key::LocalSecretError> {
        self.primary.load_matrix_session(key_id)
    }

    fn delete_matrix_session(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<(), koushi_key::LocalSecretError> {
        self.primary.delete_matrix_session(key_id)
    }

    fn save_last_session(&self, key_id: &SessionKeyId) -> Result<(), koushi_key::LocalSecretError> {
        self.primary.save_last_session(key_id)
    }

    fn load_last_session(&self) -> Result<Option<SessionKeyId>, koushi_key::LocalSecretError> {
        self.primary.load_last_session()
    }

    fn delete_last_session(&self) -> Result<(), koushi_key::LocalSecretError> {
        self.primary.delete_last_session()
    }

    fn load_saved_sessions(
        &self,
    ) -> Result<koushi_key::SavedSessionIndex, koushi_key::LocalSecretError> {
        self.primary.load_saved_sessions()
    }

    fn remember_saved_session(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<(), koushi_key::LocalSecretError> {
        let mut index = self.load_saved_sessions()?;
        index.upsert(key_id.clone());
        self.primary.save_saved_sessions(&index)
    }

    fn forget_saved_session(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<(), koushi_key::LocalSecretError> {
        let mut index = self.load_saved_sessions()?;
        index.remove(key_id);
        self.primary.save_saved_sessions(&index)
    }
}

fn local_secret_error_health(error: &koushi_key::LocalSecretError) -> LocalEncryptionHealth {
    if koushi_key::is_missing_credential_error(error) {
        return LocalEncryptionHealth::MissingCredential;
    }
    if koushi_key::is_locked_or_inaccessible_error(error) {
        return LocalEncryptionHealth::LockedOrInaccessible;
    }
    // Credential-backend errors arrive pre-abstracted as `CredentialBackendErrorKind`
    // (the platform adapter maps raw OS errors into these kinds), so the domain
    // layer never matches platform error types directly.
    match error {
        koushi_key::LocalSecretError::CredentialBackend(
            koushi_key::CredentialBackendErrorKind::Unavailable,
        ) => LocalEncryptionHealth::Unavailable,
        koushi_key::LocalSecretError::CredentialBackend(
            koushi_key::CredentialBackendErrorKind::Corrupt,
        )
        | koushi_key::LocalSecretError::Base64Decode(_)
        | koushi_key::LocalSecretError::InvalidSecretLength { .. }
        | koushi_key::LocalSecretError::Json(_)
        | koushi_key::LocalSecretError::Derivation => LocalEncryptionHealth::ResetRequired,
        koushi_key::LocalSecretError::CredentialBackend(
            koushi_key::CredentialBackendErrorKind::MissingCredential,
        ) => LocalEncryptionHealth::MissingCredential,
        koushi_key::LocalSecretError::CredentialBackend(
            koushi_key::CredentialBackendErrorKind::LockedOrInaccessible,
        ) => LocalEncryptionHealth::LockedOrInaccessible,
    }
}

// --- File-based credential store (debug/test/qa-bin only) ---

/// A trivial file-based credential store used in unattended QA runs that
/// cannot prompt macOS Keychain. Stored as plain files under `dir`; each
/// entry is a separate file named after the account.
///
/// COMPILE-TIME GATE: only present in debug/test/qa-bin builds.
/// Production release builds must not include this type.
#[cfg(any(debug_assertions, test, feature = "qa-bin"))]
#[derive(Clone)]
pub struct FileCredentialStore {
    dir: PathBuf,
}

#[cfg(any(debug_assertions, test, feature = "qa-bin"))]
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
    ) -> Result<LocalUnlockSecret, koushi_key::LocalSecretError> {
        let path = self.account_file(key_id);
        let value = std::fs::read_to_string(&path).map_err(|_| {
            koushi_key::LocalSecretError::CredentialBackend(
                koushi_key::CredentialBackendErrorKind::MissingCredential,
            )
        })?;
        LocalUnlockSecret::from_storage_string(value.trim())
    }

    fn save(
        &self,
        key_id: &SessionKeyId,
        secret: &LocalUnlockSecret,
    ) -> Result<(), koushi_key::LocalSecretError> {
        self.ensure_dir()?;
        let path = self.account_file(key_id);
        let storage_string = secret.to_storage_string();
        self.write_file(&path, storage_string.as_str())
    }

    fn delete(&self, key_id: &SessionKeyId) -> Result<(), koushi_key::LocalSecretError> {
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
    ) -> Result<(), koushi_key::LocalSecretError> {
        self.ensure_dir()?;
        self.write_file(&self.named_file(name), value)
    }

    /// Load an arbitrary named credential.
    pub(super) fn load_named(&self, name: &str) -> Result<String, koushi_key::LocalSecretError> {
        let path = self.named_file(name);
        std::fs::read_to_string(&path).map_err(|_| {
            koushi_key::LocalSecretError::CredentialBackend(
                koushi_key::CredentialBackendErrorKind::MissingCredential,
            )
        })
    }

    /// Delete an arbitrary named credential (no error if absent).
    pub(super) fn delete_named(&self, name: &str) -> Result<(), koushi_key::LocalSecretError> {
        let _ = std::fs::remove_file(self.named_file(name));
        Ok(())
    }

    fn ensure_dir(&self) -> Result<(), koushi_key::LocalSecretError> {
        std::fs::create_dir_all(&self.dir).map_err(|_| {
            koushi_key::LocalSecretError::CredentialBackend(
                koushi_key::CredentialBackendErrorKind::Unavailable,
            )
        })
    }

    fn write_file(
        &self,
        path: &std::path::Path,
        value: &str,
    ) -> Result<(), koushi_key::LocalSecretError> {
        std::fs::write(path, value).map_err(|_| {
            koushi_key::LocalSecretError::CredentialBackend(
                koushi_key::CredentialBackendErrorKind::Unavailable,
            )
        })
    }
}

/// Make a name filesystem-safe by replacing all non-alphanumeric chars with `_`.
#[cfg(any(debug_assertions, test, feature = "qa-bin"))]
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

/// Debug/test/qa-bin-only diagnostic helper. Compiled out of production release
/// builds along with its only call site (the file credential store branch in
/// `CredentialStoreBackend::resolve`).
#[cfg(any(debug_assertions, test, feature = "qa-bin"))]
fn tracing_or_eprintln(message: &'static str) {
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "core.store", "credential_store")
            .field(DiagnosticField::token("outcome", "file_backend_active")),
    );
    // Use eprintln as a simple diagnostic; in production the tracing crate
    // should be wired instead.
    if std::env::var_os("KOUSHI_DEBUG_SDK_ERROR").is_some() {
        eprintln!("[koushi-core] {message}");
    }
}

/// QA/debug structural guard: true only when the env-resolved credential
/// store backend is the file-dir backend (i.e.
/// `KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR` is set in a debug/test/qa-bin
/// build). Headless QA binaries call this BEFORE any login so unattended runs
/// are structurally unable to reach the OS keychain (engineering-rules
/// Secrets rule: keychain prompts during automation are failures).
///
/// Production release builds have no file backend, so this symbol does not
/// exist there and an app release cannot silently opt into file credentials.
#[cfg(any(debug_assertions, test, feature = "qa-bin"))]
pub fn resolved_credential_backend_is_file_dir() -> bool {
    matches!(
        CredentialStoreBackend::resolve(),
        CredentialStoreBackend::FileDir(_)
    )
}

/// Convert a `SessionInfo` (from koushi-state) into a `SessionKeyId`
/// (from koushi-key). This is the canonical mapping used everywhere
/// in the codebase.
pub fn session_key_id_from_info(info: &koushi_state::SessionInfo) -> SessionKeyId {
    SessionKeyId {
        homeserver: info.homeserver.clone(),
        user_id: info.user_id.clone(),
        device_id: info.device_id.clone(),
    }
}

/// Derive a canonical `AccountKey` string for a session. The account key is
/// the user's Matrix ID — e.g. `@alice:example.com`.
pub fn account_key_from_info(info: &koushi_state::SessionInfo) -> crate::ids::AccountKey {
    crate::ids::AccountKey(info.user_id.clone())
}

// ---- Tests ----

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn store_diagnostic_producer_records_typed_outcome_without_environment_switch() {
        tracing_or_eprintln("synthetic store diagnostic");
        let record = koushi_diagnostics::snapshot()
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
        assert!(
            config
                .store_config
                .cache_path()
                .expect("cache path should be configured")
                .starts_with(data_dir.path())
        );

        // Calling again yields a consistent store path (same key_id).
        let config2 = actor.account_store_config(&key_id).expect("second call");
        assert_eq!(config.store_config.path(), config2.store_config.path());
        assert_eq!(
            config.store_config.cache_path(),
            config2.store_config.cache_path()
        );
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
    fn file_credential_store_is_available_to_release_qa_binary_only() {
        let source = include_str!("store.rs");
        assert!(
            source.contains("cfg(any(debug_assertions, test, feature = \"qa-bin\"))"),
            "release headless QA builds need the file credential backend, while production release builds omit qa-bin"
        );
        assert!(
            source.contains("file credential store active (debug/test/qa-bin only)"),
            "diagnostic should make the qa-bin-only release escape hatch explicit"
        );
    }

    #[test]
    fn store_actor_probe_maps_credential_backend_health_without_raw_errors() {
        let data_dir = tempdir().expect("tempdir");
        let backend = koushi_key::InMemoryCredentialBackend::default();
        let actor = StoreActor::with_backend(
            CredentialStoreBackend::InMemory(koushi_key::CredentialStore::with_backend(
                "koushi-desktop-test",
                backend.clone(),
            )),
            data_dir.path(),
        );
        let key_id = make_key_id();

        assert_eq!(
            actor.probe_local_encryption_health(&key_id),
            koushi_state::LocalEncryptionHealth::MissingCredential
        );

        let secret = LocalUnlockSecret::generate();
        actor
            .credential_backend()
            .save(&key_id, &secret)
            .expect("save synthetic unlock secret");
        assert_eq!(
            actor.probe_local_encryption_health(&key_id),
            koushi_state::LocalEncryptionHealth::Healthy
        );

        backend.set_error(koushi_key::CredentialBackendErrorKind::LockedOrInaccessible);
        assert_eq!(
            actor.probe_local_encryption_health(&key_id),
            koushi_state::LocalEncryptionHealth::LockedOrInaccessible
        );
    }

    #[test]
    fn os_keychain_service_name_is_product_branded() {
        assert_eq!(CREDENTIAL_STORE_SERVICE_NAME, "koushi-desktop");
    }

    #[test]
    fn os_keychain_does_not_read_legacy_matrix_desktop_service() {
        let backend = koushi_key::InMemoryCredentialBackend::default();
        let backend_dyn: Arc<dyn koushi_key::CredentialBackend> = Arc::new(backend);
        let store = OsCredentialStore::with_backend(backend_dyn.clone());
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
        assert!(koushi_key::is_missing_credential_error(&missing));
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
    fn scheduled_sends_are_encrypted_and_reject_corruption() {
        let data_dir = tempdir().expect("tempdir");
        let cred_dir = tempdir().expect("tempdir");
        let key_id = make_key_id();
        let actor = file_store_actor(&data_dir, &cred_dir);
        let plaintext = "secret scheduled body";
        let mut scheduled_sends = ScheduledSendStore {
            capability: koushi_state::ScheduledSendCapability::LocalFallback,
            ..ScheduledSendStore::default()
        };
        scheduled_sends.insert(koushi_state::ScheduledSendItem {
            scheduled_id: "sched-1".to_owned(),
            room_id: "!room:test.example.com".to_owned(),
            body: plaintext.to_owned(),
            send_at_ms: 1_900_000_000_000,
            handle: koushi_state::ScheduledSendHandle::Local,
        });

        actor
            .save_scheduled_sends(&key_id, &scheduled_sends)
            .expect("save encrypted scheduled sends");

        let path = actor.account_scheduled_sends_file(&key_id);
        let bytes = std::fs::read(&path).expect("read encrypted scheduled sends");
        assert!(
            !bytes
                .windows(plaintext.len())
                .any(|window| window == plaintext.as_bytes())
        );

        let loaded = actor
            .load_scheduled_sends(&key_id)
            .expect("load encrypted scheduled sends");
        assert_eq!(
            loaded.items.get("sched-1").map(|item| item.body.as_str()),
            Some(plaintext)
        );
        assert_eq!(
            loaded.capability,
            koushi_state::ScheduledSendCapability::LocalFallback
        );

        let mut corrupted = bytes;
        let last = corrupted
            .last_mut()
            .expect("non-empty encrypted scheduled sends");
        *last ^= 0x01;
        std::fs::write(&path, corrupted).expect("write corrupted scheduled sends");
        assert!(matches!(
            actor.load_scheduled_sends(&key_id),
            Err(CoreFailure::StoreUnavailable)
        ));
    }

    #[test]
    fn loading_scheduled_sends_does_not_create_missing_credentials() {
        let data_dir = tempdir().expect("tempdir");
        let cred_dir = tempdir().expect("tempdir");
        let key_id = make_key_id();
        let actor = file_store_actor(&data_dir, &cred_dir);
        let path = actor.account_scheduled_sends_file(&key_id);
        std::fs::create_dir_all(path.parent().expect("scheduled sends parent"))
            .expect("create parent");
        std::fs::write(&path, SCHEDULED_SENDS_FILE_MAGIC).expect("write scheduled placeholder");

        assert!(matches!(
            actor.load_scheduled_sends(&key_id),
            Err(CoreFailure::LocalEncryptionUnavailable)
        ));
        let missing = actor.credential_backend().load(&key_id).unwrap_err();
        assert!(koushi_key::is_missing_credential_error(&missing));
    }

    #[test]
    fn empty_scheduled_sends_remove_persisted_file() {
        let data_dir = tempdir().expect("tempdir");
        let cred_dir = tempdir().expect("tempdir");
        let key_id = make_key_id();
        let actor = file_store_actor(&data_dir, &cred_dir);
        let mut scheduled_sends = ScheduledSendStore {
            capability: koushi_state::ScheduledSendCapability::LocalFallback,
            ..ScheduledSendStore::default()
        };
        scheduled_sends.insert(koushi_state::ScheduledSendItem {
            scheduled_id: "sched-1".to_owned(),
            room_id: "!room:test.example.com".to_owned(),
            body: "scheduled body".to_owned(),
            send_at_ms: 1_900_000_000_000,
            handle: koushi_state::ScheduledSendHandle::Local,
        });

        actor
            .save_scheduled_sends(&key_id, &scheduled_sends)
            .expect("save non-empty scheduled sends");
        let path = actor.account_scheduled_sends_file(&key_id);
        assert!(path.exists());

        actor
            .save_scheduled_sends(&key_id, &ScheduledSendStore::default())
            .expect("save empty scheduled sends");
        assert!(!path.exists());
        assert!(
            actor
                .load_scheduled_sends(&key_id)
                .expect("load removed scheduled sends")
                .items
                .is_empty()
        );
    }

    #[test]
    fn navigation_state_is_encrypted_and_rejects_corruption() {
        let data_dir = tempdir().expect("tempdir");
        let cred_dir = tempdir().expect("tempdir");
        let key_id = make_key_id();
        let actor = file_store_actor(&data_dir, &cred_dir);
        let navigation = NavigationState {
            active_space_id: Some("!space:test.example.com".to_owned()),
            active_room_id: Some("!room:test.example.com".to_owned()),
            space_order: vec!["!space:test.example.com".to_owned()],
            last_room_by_space_id: std::collections::BTreeMap::from([(
                "!space:test.example.com".to_owned(),
                "!room:test.example.com".to_owned(),
            )]),
            room_scroll_anchors: std::collections::BTreeMap::new(),
            main_timeline_anchor: None,
        };

        actor
            .save_navigation(&key_id, &navigation)
            .expect("save encrypted navigation");

        let path = actor.account_navigation_file(&key_id);
        let bytes = std::fs::read(&path).expect("read encrypted navigation");
        for plaintext in ["!space:test.example.com", "!room:test.example.com"] {
            assert!(
                !bytes
                    .windows(plaintext.len())
                    .any(|window| window == plaintext.as_bytes())
            );
        }

        let loaded = actor
            .load_navigation(&key_id)
            .expect("load encrypted navigation");
        assert_eq!(loaded, navigation);

        let mut corrupted = bytes;
        let last = corrupted
            .last_mut()
            .expect("non-empty encrypted navigation");
        *last ^= 0x01;
        std::fs::write(&path, corrupted).expect("write corrupted navigation");
        assert!(matches!(
            actor.load_navigation(&key_id),
            Err(CoreFailure::StoreUnavailable)
        ));
    }

    #[test]
    fn legacy_navigation_json_loads_and_next_save_migrates_to_encrypted_file() {
        let data_dir = tempdir().expect("tempdir");
        let cred_dir = tempdir().expect("tempdir");
        let key_id = make_key_id();
        let actor = file_store_actor(&data_dir, &cred_dir);
        let navigation = NavigationState {
            active_space_id: Some("!space:test.example.com".to_owned()),
            active_room_id: Some("!room:test.example.com".to_owned()),
            space_order: vec!["!space:test.example.com".to_owned()],
            last_room_by_space_id: std::collections::BTreeMap::from([(
                "!space:test.example.com".to_owned(),
                "!room:test.example.com".to_owned(),
            )]),
            room_scroll_anchors: std::collections::BTreeMap::new(),
            main_timeline_anchor: None,
        };
        let legacy_path = actor.account_navigation_legacy_file(&key_id);
        std::fs::create_dir_all(legacy_path.parent().expect("navigation parent"))
            .expect("create navigation parent");
        std::fs::write(
            &legacy_path,
            serde_json::to_string(&navigation).expect("serialize legacy navigation"),
        )
        .expect("write legacy navigation");

        let loaded = actor
            .load_navigation(&key_id)
            .expect("load legacy navigation");
        assert_eq!(loaded, navigation);

        actor
            .save_navigation(&key_id, &navigation)
            .expect("migrate navigation");
        assert!(!legacy_path.exists());

        let encrypted_path = actor.account_navigation_file(&key_id);
        let bytes = std::fs::read(&encrypted_path).expect("read encrypted navigation");
        for plaintext in ["!space:test.example.com", "!room:test.example.com"] {
            assert!(
                !bytes
                    .windows(plaintext.len())
                    .any(|window| window == plaintext.as_bytes())
            );
        }
    }

    #[test]
    fn default_navigation_removes_encrypted_and_legacy_files() {
        let data_dir = tempdir().expect("tempdir");
        let cred_dir = tempdir().expect("tempdir");
        let key_id = make_key_id();
        let actor = file_store_actor(&data_dir, &cred_dir);
        let navigation = NavigationState {
            active_space_id: None,
            active_room_id: Some("!room:test.example.com".to_owned()),
            space_order: Vec::new(),
            last_room_by_space_id: std::collections::BTreeMap::new(),
            room_scroll_anchors: std::collections::BTreeMap::new(),
            main_timeline_anchor: None,
        };

        actor
            .save_navigation(&key_id, &navigation)
            .expect("save encrypted navigation");
        let encrypted_path = actor.account_navigation_file(&key_id);
        assert!(encrypted_path.exists());

        let legacy_path = actor.account_navigation_legacy_file(&key_id);
        std::fs::create_dir_all(legacy_path.parent().expect("navigation parent"))
            .expect("create navigation parent");
        std::fs::write(&legacy_path, "{}").expect("write legacy navigation");
        assert!(legacy_path.exists());

        actor
            .save_navigation(&key_id, &NavigationState::default())
            .expect("clear navigation");
        assert!(!encrypted_path.exists());
        assert!(!legacy_path.exists());
        assert_eq!(
            actor
                .load_navigation(&key_id)
                .expect("load cleared navigation"),
            NavigationState::default()
        );
    }

    #[test]
    fn encrypted_navigation_state_preserves_room_scroll_anchor() {
        let data_dir = tempdir().expect("tempdir");
        let cred_dir = tempdir().expect("tempdir");
        let key_id = make_key_id();
        let actor = file_store_actor(&data_dir, &cred_dir);
        let navigation = NavigationState {
            active_space_id: Some("!space:test.example.com".to_owned()),
            active_room_id: Some("!room:test.example.com".to_owned()),
            space_order: vec!["!space:test.example.com".to_owned()],
            last_room_by_space_id: std::collections::BTreeMap::from([(
                "!space:test.example.com".to_owned(),
                "!room:test.example.com".to_owned(),
            )]),
            room_scroll_anchors: std::collections::BTreeMap::from([(
                "!room:test.example.com".to_owned(),
                koushi_state::TimelineScrollAnchor {
                    event_id: "$anchor:event".to_owned(),
                    edge: koushi_state::TimelineScrollAnchorEdge::Top,
                    offset_px: -32,
                    updated_at_ms: 1_820_000_000_000,
                },
            )]),
            main_timeline_anchor: None,
        };

        actor
            .save_navigation(&key_id, &navigation)
            .expect("save encrypted navigation");
        let loaded = actor
            .load_navigation(&key_id)
            .expect("load encrypted navigation");

        assert_eq!(loaded, navigation);
    }

    #[test]
    fn room_preferences_are_encrypted_and_reject_corruption() {
        let data_dir = tempdir().expect("tempdir");
        let cred_dir = tempdir().expect("tempdir");
        let key_id = make_key_id();
        let actor = file_store_actor(&data_dir, &cred_dir);
        let room_id = "!room:test.example.com";
        let preferences = RoomPreferencesState {
            rooms: std::collections::BTreeMap::from([(
                room_id.to_owned(),
                RoomPreference {
                    url_previews_enabled_override: Some(false),
                    ..RoomPreference::default()
                },
            )]),
        };

        actor
            .save_room_preferences(&key_id, &preferences)
            .expect("save encrypted room preferences");

        let path = actor.account_room_preferences_file(&key_id);
        let bytes = std::fs::read(&path).expect("read encrypted room preferences");
        assert!(
            !bytes
                .windows(room_id.len())
                .any(|window| window == room_id.as_bytes())
        );

        let loaded = actor
            .load_room_preferences(&key_id)
            .expect("load encrypted room preferences");
        assert_eq!(loaded, preferences);

        let mut corrupted = bytes;
        let last = corrupted
            .last_mut()
            .expect("non-empty encrypted room preferences");
        *last ^= 0x01;
        std::fs::write(&path, corrupted).expect("write corrupted room preferences");
        assert!(matches!(
            actor.load_room_preferences(&key_id),
            Err(CoreFailure::StoreUnavailable)
        ));
    }

    #[test]
    fn legacy_navigation_json_without_scroll_anchors_loads_with_empty_map() {
        let data_dir = tempdir().expect("tempdir");
        let cred_dir = tempdir().expect("tempdir");
        let key_id = make_key_id();
        let actor = file_store_actor(&data_dir, &cred_dir);
        let legacy_path = actor.account_navigation_legacy_file(&key_id);
        std::fs::create_dir_all(legacy_path.parent().expect("navigation parent"))
            .expect("create navigation parent");
        std::fs::write(
            &legacy_path,
            r#"{
                "active_space_id":"!space:test.example.com",
                "active_room_id":"!room:test.example.com",
                "space_order":["!space:test.example.com"],
                "last_room_by_space_id":{"!space:test.example.com":"!room:test.example.com"}
            }"#,
        )
        .expect("write legacy navigation");

        let loaded = actor
            .load_navigation(&key_id)
            .expect("load legacy navigation");

        assert!(loaded.room_scroll_anchors.is_empty());
        assert_eq!(
            loaded.active_room_id.as_deref(),
            Some("!room:test.example.com")
        );
    }

    #[test]
    fn composer_draft_persistence_applies_entry_and_size_bounds() {
        let data_dir = tempdir().expect("tempdir");
        let cred_dir = tempdir().expect("tempdir");
        let key_id = make_key_id();
        let actor = file_store_actor(&data_dir, &cred_dir);
        let mut drafts = ComposerDraftStore::default();
        let oversized = "x".repeat(koushi_state::MAX_PERSISTED_COMPOSER_DRAFT_BYTES + 64);

        for index in 0..(koushi_state::MAX_PERSISTED_COMPOSER_DRAFT_ROOM_COUNT + 8) {
            drafts.set_room_draft(format!("!room-{index}:test.example.com"), oversized.clone());
        }
        for index in 0..(koushi_state::MAX_PERSISTED_COMPOSER_DRAFT_THREAD_COUNT + 8) {
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

        assert!(loaded.rooms.len() <= koushi_state::MAX_PERSISTED_COMPOSER_DRAFT_ROOM_COUNT);
        assert!(
            loaded
                .rooms
                .values()
                .all(|draft| draft.len() <= koushi_state::MAX_PERSISTED_COMPOSER_DRAFT_BYTES)
        );
        let thread_count = loaded
            .threads
            .values()
            .map(std::collections::BTreeMap::len)
            .sum::<usize>();
        assert!(thread_count <= koushi_state::MAX_PERSISTED_COMPOSER_DRAFT_THREAD_COUNT);
        assert!(
            loaded
                .threads
                .values()
                .flat_map(|room_threads| room_threads.values())
                .all(|draft| draft.len() <= koushi_state::MAX_PERSISTED_COMPOSER_DRAFT_BYTES)
        );
    }
}
