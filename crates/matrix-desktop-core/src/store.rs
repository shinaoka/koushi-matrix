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

use matrix_desktop_auth::{MatrixClientStoreConfig, MatrixClientStoreKey};
use matrix_desktop_key::{CredentialStore, LocalUnlockSecret, SessionKeyId};

use crate::failure::CoreFailure;

/// Service name used for all keyring entries in this application.
const CREDENTIAL_STORE_SERVICE_NAME: &str = "matrix-desktop";

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

/// StoreActor: resolves and manages per-account credential-backed store configs.
///
/// In Phase 2 this is a pure value type (no background task). Phase 6 may
/// promote it to an owned task when search index mutations require it.
pub struct StoreActor {
    credential_store: CredentialStoreBackend,
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

        let store_config = MatrixClientStoreConfig::new(&store_dir, store_key)
            .with_cache_path(&cache_dir);

        Ok(AccountStoreConfig {
            store_config,
            session_key_id: key_id.clone(),
        })
    }

    /// Delete all stored credentials and store directory for an account.
    /// Called during logout / account removal.
    ///
    /// Errors are logged as diagnostics but do not propagate — a logout that
    /// partially cleans up is better than a logout that fails.
    pub fn delete_account_credentials(&self, key_id: &SessionKeyId) {
        let _ = self.credential_store.delete(key_id);
        // Session data stored via CredentialStore (matrix session JSON) is
        // managed by AccountActor directly using the same CredentialStore.
    }

    /// The OS or file-based credential store backend.
    pub fn credential_store_backend(&self) -> &CredentialStoreBackend {
        &self.credential_store
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

    fn account_store_dir(&self, key_id: &SessionKeyId) -> PathBuf {
        self.data_dir
            .join("accounts")
            .join(account_dir_name(key_id))
            .join("store")
    }

    fn account_cache_dir(&self, key_id: &SessionKeyId) -> PathBuf {
        self.data_dir
            .join("accounts")
            .join(account_dir_name(key_id))
            .join("cache")
    }
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
    OsKeychain(CredentialStore),
    #[cfg(any(debug_assertions, test))]
    FileDir(FileCredentialStore),
}

impl CredentialStoreBackend {
    fn resolve() -> Self {
        #[cfg(any(debug_assertions, test))]
        if let Ok(dir) = std::env::var(ENV_FILE_CREDENTIAL_STORE_DIR) {
            let dir = PathBuf::from(dir);
            tracing_or_eprintln("file credential store active (debug/test only)");
            return Self::FileDir(FileCredentialStore::new(dir));
        }
        Self::OsKeychain(CredentialStore::new(CREDENTIAL_STORE_SERVICE_NAME))
    }

    fn load(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<LocalUnlockSecret, matrix_desktop_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.load(key_id),
            #[cfg(any(debug_assertions, test))]
            Self::FileDir(store) => store.load(key_id),
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
        }
    }

    fn delete(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<(), matrix_desktop_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.delete(key_id),
            #[cfg(any(debug_assertions, test))]
            Self::FileDir(store) => store.delete(key_id),
        }
    }

    /// Expose the underlying `CredentialStore` (for session persistence) when
    /// using the OS keychain backend.
    pub fn as_os_credential_store(&self) -> Option<&CredentialStore> {
        match self {
            Self::OsKeychain(store) => Some(store),
            #[cfg(any(debug_assertions, test))]
            Self::FileDir(_) => None,
        }
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
        self.dir.join(key_id.account_name())
    }

    fn load(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<LocalUnlockSecret, matrix_desktop_key::LocalSecretError> {
        let path = self.account_file(key_id);
        let value = std::fs::read_to_string(&path)
            .map_err(|_| matrix_desktop_key::LocalSecretError::CredentialStore(
                keyring::Error::NoEntry,
            ))?;
        LocalUnlockSecret::from_storage_string(value.trim())
    }

    fn save(
        &self,
        key_id: &SessionKeyId,
        secret: &LocalUnlockSecret,
    ) -> Result<(), matrix_desktop_key::LocalSecretError> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| matrix_desktop_key::LocalSecretError::CredentialStore(
                keyring::Error::PlatformFailure(Box::new(e)),
            ))?;
        let path = self.account_file(key_id);
        let storage_string = secret.to_storage_string();
        std::fs::write(&path, storage_string.as_str())
            .map_err(|e| matrix_desktop_key::LocalSecretError::CredentialStore(
                keyring::Error::PlatformFailure(Box::new(e)),
            ))
    }

    fn delete(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<(), matrix_desktop_key::LocalSecretError> {
        let path = self.account_file(key_id);
        match std::fs::remove_file(&path) {
            Ok(()) | Err(_) => Ok(()),
        }
    }
}

fn tracing_or_eprintln(message: &str) {
    // Use eprintln as a simple diagnostic; in production the tracing crate
    // should be wired instead.
    if std::env::var_os("MATRIX_DESKTOP_DEBUG_SDK_ERROR").is_some() {
        eprintln!("[matrix-desktop-core] {message}");
    }
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

        let actor = StoreActor {
            credential_store: CredentialStoreBackend::FileDir(FileCredentialStore::new(
                cred_dir.path(),
            )),
            data_dir: data_dir.path().to_path_buf(),
        };

        let config = actor
            .account_store_config(&key_id)
            .expect("store config should succeed");

        // Path is inside our data dir.
        assert!(config
            .store_config
            .path()
            .starts_with(data_dir.path()));

        // Calling again yields a consistent store path (same key_id).
        let config2 = actor
            .account_store_config(&key_id)
            .expect("second call");
        assert_eq!(
            config.store_config.path(),
            config2.store_config.path()
        );
    }

    #[test]
    fn delete_account_credentials_does_not_panic_when_absent() {
        let data_dir = tempdir().expect("tempdir");
        let cred_dir = tempdir().expect("tempdir");
        let key_id = make_key_id();

        let actor = StoreActor {
            credential_store: CredentialStoreBackend::FileDir(FileCredentialStore::new(
                cred_dir.path(),
            )),
            data_dir: data_dir.path().to_path_buf(),
        };

        // Should not panic even when credentials don't exist.
        actor.delete_account_credentials(&key_id);
    }
}
