use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use koushi_key::{CredentialStore, LocalUnlockSecret, SessionKeyId};
use koushi_state::LocalEncryptionHealth;

use super::CREDENTIAL_STORE_SERVICE_NAME;

/// Env var for QA/debug file-based credential store override.
/// Only honored in debug/test/qa-bin builds; production release builds ignore it.
#[cfg(any(debug_assertions, test, feature = "qa-bin"))]
const ENV_FILE_CREDENTIAL_STORE_DIR: &str = "KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR";

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
    pub(super) fn resolve() -> Self {
        #[cfg(any(debug_assertions, test, feature = "qa-bin"))]
        if let Ok(dir) = std::env::var(ENV_FILE_CREDENTIAL_STORE_DIR) {
            let dir = PathBuf::from(dir);
            record_file_credential_store_active();
            return Self::FileDir(FileCredentialStore::new(dir));
        }
        Self::InMemory(CredentialStore::with_backend(
            CREDENTIAL_STORE_SERVICE_NAME,
            koushi_key::InMemoryCredentialBackend::default(),
        ))
    }

    pub(super) fn resolve_with_os_backend(
        data_dir: PathBuf,
        os_backend: Arc<dyn koushi_key::CredentialBackend>,
    ) -> Self {
        #[cfg(any(debug_assertions, test, feature = "qa-bin"))]
        if let Ok(dir) = std::env::var(ENV_FILE_CREDENTIAL_STORE_DIR) {
            let dir = PathBuf::from(dir);
            record_file_credential_store_active();
            return Self::FileDir(FileCredentialStore::new(dir));
        }
        Self::OsKeychain(OsCredentialStore::with_backend(data_dir, os_backend))
    }

    pub(super) fn load(
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

    pub(super) fn save(
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

    pub(super) fn delete(&self, key_id: &SessionKeyId) -> Result<(), koushi_key::LocalSecretError> {
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

    pub fn save_local_store_id(
        &self,
        key_id: &SessionKeyId,
        store_id: &koushi_key::LocalStoreId,
    ) -> Result<(), koushi_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.save_local_store_id(key_id, store_id),
            #[cfg(any(debug_assertions, test, feature = "qa-bin"))]
            Self::FileDir(store) => store.save_named(
                &format!("local-store|{}", key_id.local_unlock_account_name()),
                store_id.as_str(),
            ),
            Self::InMemory(store) => store.save_local_store_id(key_id, store_id),
        }
    }

    pub fn load_local_store_id(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<koushi_key::LocalStoreId, koushi_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.load_local_store_id(key_id),
            #[cfg(any(debug_assertions, test, feature = "qa-bin"))]
            Self::FileDir(store) => koushi_key::LocalStoreId::parse(&store.load_named(
                &format!("local-store|{}", key_id.local_unlock_account_name()),
            )?),
            Self::InMemory(store) => store.load_local_store_id(key_id),
        }
    }

    pub fn delete_local_store_id(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<(), koushi_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.delete_local_store_id(key_id),
            #[cfg(any(debug_assertions, test, feature = "qa-bin"))]
            Self::FileDir(store) => store.delete_named(&format!(
                "local-store|{}",
                key_id.local_unlock_account_name()
            )),
            Self::InMemory(store) => store.delete_local_store_id(key_id),
        }
    }

    /// Persist the journal as one named credential for non-vault backends.
    /// The OS backend stores the same value in the encrypted vault instead.
    pub fn save_pending_login_journal(
        &self,
        value: &str,
    ) -> Result<(), koushi_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.save_pending_login_journal(value),
            #[cfg(any(debug_assertions, test, feature = "qa-bin"))]
            Self::FileDir(store) => {
                store.save_named(koushi_key::pending_login_journal_account_name(), value)
            }
            Self::InMemory(store) => store.save_pending_login_journal(value),
        }
    }

    pub fn load_pending_login_journal(
        &self,
    ) -> Result<Option<String>, koushi_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.load_pending_login_journal(),
            #[cfg(any(debug_assertions, test, feature = "qa-bin"))]
            Self::FileDir(store) => {
                match store.load_named(koushi_key::pending_login_journal_account_name()) {
                    Ok(value) => Ok(Some(value)),
                    Err(error) if koushi_key::is_missing_credential_error(&error) => Ok(None),
                    Err(error) => Err(error),
                }
            }
            Self::InMemory(store) => match store.load_pending_login_journal() {
                Ok(value) => Ok(Some(value)),
                Err(error) if koushi_key::is_missing_credential_error(&error) => Ok(None),
                Err(error) => Err(error),
            },
        }
    }

    pub fn delete_pending_login_journal(&self) -> Result<(), koushi_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.delete_pending_login_journal(),
            #[cfg(any(debug_assertions, test, feature = "qa-bin"))]
            Self::FileDir(store) => {
                store.delete_named(koushi_key::pending_login_journal_account_name())
            }
            Self::InMemory(store) => store.delete_pending_login_journal(),
        }
    }

    pub fn save_local_store_migration(
        &self,
        value: &str,
    ) -> Result<(), koushi_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.save_local_store_migration(value),
            #[cfg(any(debug_assertions, test, feature = "qa-bin"))]
            Self::FileDir(store) => {
                store.save_named(koushi_key::local_store_migration_account_name(), value)
            }
            Self::InMemory(store) => store.save_local_store_migration(value),
        }
    }

    pub fn load_local_store_migration(
        &self,
    ) -> Result<Option<String>, koushi_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.load_local_store_migration(),
            #[cfg(any(debug_assertions, test, feature = "qa-bin"))]
            Self::FileDir(store) => {
                match store.load_named(koushi_key::local_store_migration_account_name()) {
                    Ok(value) => Ok(Some(value)),
                    Err(error) if koushi_key::is_missing_credential_error(&error) => Ok(None),
                    Err(error) => Err(error),
                }
            }
            Self::InMemory(store) => match store.load_local_store_migration() {
                Ok(value) => Ok(Some(value)),
                Err(error) if koushi_key::is_missing_credential_error(&error) => Ok(None),
                Err(error) => Err(error),
            },
        }
    }

    pub fn delete_local_store_migration(&self) -> Result<(), koushi_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.delete_local_store_migration(),
            #[cfg(any(debug_assertions, test, feature = "qa-bin"))]
            Self::FileDir(store) => {
                store.delete_named(koushi_key::local_store_migration_account_name())
            }
            Self::InMemory(store) => store.delete_local_store_migration(),
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
}

/// OS keychain credential store for the shipped product service.
#[derive(Clone)]
pub struct OsCredentialStore {
    primary: CredentialStore<Arc<dyn koushi_key::CredentialBackend>>,
    vault_file: crate::credential_vault::CredentialVaultFile,
    vault_state: Arc<Mutex<Option<OsCredentialVaultState>>>,
    cache_reuse_recorded: Arc<AtomicBool>,
}

struct OsCredentialVaultState {
    master_key: Option<koushi_key::CredentialVaultMasterKey>,
    data: crate::credential_vault::CredentialVaultData,
}

impl OsCredentialStore {
    fn with_backend(
        data_dir: impl AsRef<std::path::Path>,
        backend: Arc<dyn koushi_key::CredentialBackend>,
    ) -> Self {
        Self {
            primary: CredentialStore::with_backend(CREDENTIAL_STORE_SERVICE_NAME, backend),
            vault_file: crate::credential_vault::CredentialVaultFile::new(
                data_dir
                    .as_ref()
                    .join("credentials")
                    .join("credentials.v1.enc"),
            ),
            vault_state: Arc::new(Mutex::new(None)),
            cache_reuse_recorded: Arc::new(AtomicBool::new(false)),
        }
    }

    fn load(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<LocalUnlockSecret, koushi_key::LocalSecretError> {
        self.read_vault(|data| {
            let stored = data
                .local_unlock_secret(key_id)
                .ok_or_else(missing_credential_error)?;
            LocalUnlockSecret::from_storage_string(stored)
        })
    }

    fn save(
        &self,
        key_id: &SessionKeyId,
        secret: &LocalUnlockSecret,
    ) -> Result<(), koushi_key::LocalSecretError> {
        let stored = secret.to_storage_string();
        self.mutate_vault(|data| {
            data.upsert_local_unlock_secret(key_id.clone(), stored.as_str());
        })
    }

    fn delete(&self, key_id: &SessionKeyId) -> Result<(), koushi_key::LocalSecretError> {
        self.mutate_vault(|data| data.delete_local_unlock_secret(key_id))
    }

    fn save_local_store_id(
        &self,
        key_id: &SessionKeyId,
        store_id: &koushi_key::LocalStoreId,
    ) -> Result<(), koushi_key::LocalSecretError> {
        self.mutate_vault(|data| data.upsert_local_store_id(key_id.clone(), store_id.clone()))
    }

    fn load_local_store_id(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<koushi_key::LocalStoreId, koushi_key::LocalSecretError> {
        self.read_vault(|data| {
            data.local_store_id(key_id)
                .cloned()
                .ok_or_else(missing_credential_error)
        })
    }

    fn delete_local_store_id(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<(), koushi_key::LocalSecretError> {
        self.mutate_vault(|data| data.delete_local_store_id(key_id))
    }

    fn save_pending_login_journal(&self, value: &str) -> Result<(), koushi_key::LocalSecretError> {
        let records: Vec<crate::credential_vault::PendingLoginRecord> =
            serde_json::from_str(value).map_err(koushi_key::LocalSecretError::Json)?;
        self.mutate_vault(|data| *data.pending_logins_mut() = records)
    }

    fn load_pending_login_journal(&self) -> Result<Option<String>, koushi_key::LocalSecretError> {
        self.read_vault(|data| {
            if data.pending_logins().is_empty() {
                Ok(None)
            } else {
                serde_json::to_string(data.pending_logins())
                    .map(Some)
                    .map_err(koushi_key::LocalSecretError::Json)
            }
        })
    }

    fn delete_pending_login_journal(&self) -> Result<(), koushi_key::LocalSecretError> {
        self.mutate_vault(|data| data.pending_logins_mut().clear())
    }

    fn save_local_store_migration(&self, value: &str) -> Result<(), koushi_key::LocalSecretError> {
        let migration: crate::credential_vault::LocalStoreMigrationRecord =
            serde_json::from_str(value).map_err(koushi_key::LocalSecretError::Json)?;
        self.mutate_vault(|data| data.set_local_store_migration(migration))
    }

    fn load_local_store_migration(&self) -> Result<Option<String>, koushi_key::LocalSecretError> {
        self.read_vault(|data| {
            data.local_store_migration()
                .map(serde_json::to_string)
                .transpose()
                .map_err(koushi_key::LocalSecretError::Json)
        })
    }

    fn delete_local_store_migration(&self) -> Result<(), koushi_key::LocalSecretError> {
        self.mutate_vault(|data| {
            data.clear_local_store_migration();
        })
    }

    fn save_matrix_session(
        &self,
        key_id: &SessionKeyId,
        session: &koushi_key::StoredMatrixSession,
    ) -> Result<(), koushi_key::LocalSecretError> {
        self.mutate_vault(|data| {
            data.upsert_matrix_session(key_id.clone(), session.as_str());
        })
    }

    fn load_matrix_session(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<koushi_key::StoredMatrixSession, koushi_key::LocalSecretError> {
        self.read_vault(|data| {
            data.matrix_session(key_id)
                .map(koushi_key::StoredMatrixSession::new)
                .ok_or_else(missing_credential_error)
        })
    }

    fn delete_matrix_session(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<(), koushi_key::LocalSecretError> {
        self.mutate_vault(|data| data.delete_matrix_session(key_id))
    }

    fn save_last_session(&self, key_id: &SessionKeyId) -> Result<(), koushi_key::LocalSecretError> {
        self.mutate_vault(|data| data.set_last_session(Some(key_id.clone())))
    }

    fn load_last_session(&self) -> Result<Option<SessionKeyId>, koushi_key::LocalSecretError> {
        self.read_vault(|data| Ok(data.last_session().cloned()))
    }

    fn delete_last_session(&self) -> Result<(), koushi_key::LocalSecretError> {
        self.mutate_vault(|data| data.set_last_session(None))
    }

    fn load_saved_sessions(
        &self,
    ) -> Result<koushi_key::SavedSessionIndex, koushi_key::LocalSecretError> {
        self.read_vault(|data| Ok(data.saved_sessions()))
    }

    fn remember_saved_session(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<(), koushi_key::LocalSecretError> {
        self.mutate_vault(|data| data.remember_session(key_id.clone()))
    }

    fn forget_saved_session(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<(), koushi_key::LocalSecretError> {
        self.mutate_vault(|data| data.forget_session(key_id))
    }

    fn read_vault<T>(
        &self,
        read: impl FnOnce(
            &crate::credential_vault::CredentialVaultData,
        ) -> Result<T, koushi_key::LocalSecretError>,
    ) -> Result<T, koushi_key::LocalSecretError> {
        let mut state = self
            .vault_state
            .lock()
            .map_err(|_| unavailable_credential_error())?;
        self.initialize_vault(&mut state)?;
        read(
            &state
                .as_ref()
                .expect("vault is initialized before reads")
                .data,
        )
    }

    fn mutate_vault(
        &self,
        mutate: impl FnOnce(&mut crate::credential_vault::CredentialVaultData),
    ) -> Result<(), koushi_key::LocalSecretError> {
        let mut state = self
            .vault_state
            .lock()
            .map_err(|_| unavailable_credential_error())?;
        self.initialize_vault(&mut state)?;
        let current = state.as_mut().expect("vault is initialized before writes");
        if current.master_key.is_none() {
            let master_key = koushi_key::CredentialVaultMasterKey::generate();
            self.primary.save_vault_master_key(&master_key)?;
            current.master_key = Some(master_key);
        }
        let mut next = current.data.clone();
        mutate(&mut next);
        self.vault_file
            .store(
                current
                    .master_key
                    .as_ref()
                    .expect("master key was installed before vault write"),
                &next,
            )
            .map_err(vault_error_to_local_secret_error)?;
        current.data = next;
        self.retry_legacy_cleanup(current);
        Ok(())
    }

    fn initialize_vault(
        &self,
        state: &mut Option<OsCredentialVaultState>,
    ) -> Result<(), koushi_key::LocalSecretError> {
        if state.is_some() {
            if !self.cache_reuse_recorded.swap(true, Ordering::Relaxed) {
                record_credential_vault_access("memory_cache_reused");
            }
            return Ok(());
        }
        record_credential_vault_access("keychain_read_started");
        let master_key = match self.primary.load_vault_master_key() {
            Ok(master_key) => {
                record_credential_vault_access("keychain_read_succeeded");
                Some(master_key)
            }
            Err(error) if koushi_key::is_missing_credential_error(&error) => {
                record_credential_vault_access("keychain_entry_missing");
                None
            }
            Err(error) => {
                record_credential_vault_access(credential_vault_failure_outcome(&error));
                return Err(error);
            }
        };
        if self.vault_file.exists() {
            let master_key = master_key.ok_or_else(missing_credential_error)?;
            let mut data = self
                .vault_file
                .load(&master_key)
                .map_err(vault_error_to_local_secret_error)?;
            if data.payload_version() == 1 {
                self.vault_file
                    .store(&master_key, &data)
                    .map_err(vault_error_to_local_secret_error)?;
                data.mark_current_version();
            }
            let pending = data.legacy_cleanup_pending().to_vec();
            if !pending.is_empty() && self.cleanup_legacy_credentials(&pending) {
                let mut cleaned = data.clone();
                cleaned.clear_legacy_cleanup_pending();
                if self.vault_file.store(&master_key, &cleaned).is_ok() {
                    data = cleaned;
                }
            }
            *state = Some(OsCredentialVaultState {
                master_key: Some(master_key),
                data,
            });
            return Ok(());
        }

        let saved_sessions = self.primary.load_saved_sessions()?;
        let last_session = self.primary.load_last_session()?;
        let mut legacy_keys = saved_sessions.sessions().to_vec();
        if let Some(last_session) = last_session.as_ref()
            && !legacy_keys.contains(last_session)
        {
            legacy_keys.push(last_session.clone());
        }
        if legacy_keys.is_empty() {
            *state = Some(OsCredentialVaultState {
                master_key,
                data: crate::credential_vault::CredentialVaultData::default(),
            });
            return Ok(());
        }

        let mut data = crate::credential_vault::CredentialVaultData::default();
        data.set_last_session(last_session);
        for key_id in &legacy_keys {
            let session = self.primary.load_matrix_session(key_id)?;
            let secret = self.primary.load(key_id)?;
            data.remember_session(key_id.clone());
            data.upsert_matrix_session(key_id.clone(), session.as_str());
            let stored_secret = secret.to_storage_string();
            data.upsert_local_unlock_secret(key_id.clone(), stored_secret.as_str());
        }
        data.set_legacy_cleanup_pending(legacy_keys.clone());
        let master_key = match master_key {
            Some(master_key) => master_key,
            None => {
                let master_key = koushi_key::CredentialVaultMasterKey::generate();
                self.primary.save_vault_master_key(&master_key)?;
                master_key
            }
        };
        self.vault_file
            .store(&master_key, &data)
            .map_err(vault_error_to_local_secret_error)?;
        let mut verified = self
            .vault_file
            .load(&master_key)
            .map_err(vault_error_to_local_secret_error)?;
        if self.cleanup_legacy_credentials(&legacy_keys) {
            let mut cleaned = verified.clone();
            cleaned.clear_legacy_cleanup_pending();
            if self.vault_file.store(&master_key, &cleaned).is_ok() {
                verified = cleaned;
            }
        }
        *state = Some(OsCredentialVaultState {
            master_key: Some(master_key),
            data: verified,
        });
        Ok(())
    }

    fn retry_legacy_cleanup(&self, current: &mut OsCredentialVaultState) {
        let pending = current.data.legacy_cleanup_pending().to_vec();
        if pending.is_empty() || !self.cleanup_legacy_credentials(&pending) {
            return;
        }
        let Some(master_key) = current.master_key.as_ref() else {
            return;
        };
        let mut cleaned = current.data.clone();
        cleaned.clear_legacy_cleanup_pending();
        if self.vault_file.store(master_key, &cleaned).is_ok() {
            current.data = cleaned;
        }
    }

    fn cleanup_legacy_credentials(&self, key_ids: &[SessionKeyId]) -> bool {
        let mut succeeded = true;
        for key_id in key_ids {
            succeeded &= self.primary.delete_matrix_session(key_id).is_ok();
            succeeded &= self.primary.delete(key_id).is_ok();
        }
        succeeded &= self.primary.delete_last_session().is_ok();
        succeeded &= self.primary.delete_saved_sessions().is_ok();
        succeeded
    }
}

fn missing_credential_error() -> koushi_key::LocalSecretError {
    koushi_key::LocalSecretError::CredentialBackend(
        koushi_key::CredentialBackendErrorKind::MissingCredential,
    )
}

fn unavailable_credential_error() -> koushi_key::LocalSecretError {
    koushi_key::LocalSecretError::CredentialBackend(
        koushi_key::CredentialBackendErrorKind::Unavailable,
    )
}

fn vault_error_to_local_secret_error(
    error: crate::credential_vault::CredentialVaultError,
) -> koushi_key::LocalSecretError {
    let kind = match error {
        crate::credential_vault::CredentialVaultError::Unavailable => {
            koushi_key::CredentialBackendErrorKind::Unavailable
        }
        crate::credential_vault::CredentialVaultError::Corrupt => {
            koushi_key::CredentialBackendErrorKind::Corrupt
        }
    };
    koushi_key::LocalSecretError::CredentialBackend(kind)
}

pub(super) fn local_secret_error_health(
    error: &koushi_key::LocalSecretError,
) -> LocalEncryptionHealth {
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
fn record_file_credential_store_active() {
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "core.store", "credential_store")
            .field(DiagnosticField::token("outcome", "file_backend_active")),
    );
}

pub(super) fn record_local_unlock_secret(purpose: Option<&'static str>, outcome: &'static str) {
    let Some(purpose) = purpose else {
        return;
    };
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "core.store", "local_unlock_secret")
            .field(DiagnosticField::token("purpose", purpose))
            .field(DiagnosticField::token("outcome", outcome)),
    );
}

fn record_credential_vault_access(outcome: &'static str) {
    record(
        DiagnosticEvent::new(
            DiagnosticLevel::Debug,
            "core.store",
            "credential_vault_access",
        )
        .field(DiagnosticField::token("outcome", outcome)),
    );
}

fn credential_vault_failure_outcome(error: &koushi_key::LocalSecretError) -> &'static str {
    match error {
        koushi_key::LocalSecretError::CredentialBackend(
            koushi_key::CredentialBackendErrorKind::LockedOrInaccessible,
        ) => "keychain_read_locked_or_denied",
        koushi_key::LocalSecretError::CredentialBackend(
            koushi_key::CredentialBackendErrorKind::Corrupt,
        )
        | koushi_key::LocalSecretError::Base64Decode(_)
        | koushi_key::LocalSecretError::InvalidSecretLength { .. } => "keychain_read_corrupt",
        _ => "keychain_read_unavailable",
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

#[cfg(test)]
mod tests {
    use super::super::StoreActor;
    use super::super::test_support::{file_store_actor, make_key_id};
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn store_diagnostic_producer_records_typed_outcome_without_environment_switch() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        record_file_credential_store_active();
        let record = koushi_diagnostics::test_support::detail_snapshot()
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
    fn account_store_and_search_config_trace_unlock_secret_source() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        let data_dir = tempdir().expect("tempdir");
        let cred_dir = tempdir().expect("tempdir");
        let key_id = make_key_id();
        let actor = file_store_actor(&data_dir, &cred_dir);
        let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
            .records
            .len();

        actor
            .account_store_config(&key_id)
            .expect("first store config creates the unlock secret");
        actor
            .account_search_index_config(&key_id)
            .expect("search config reuses the unlock secret");
        actor
            .account_store_config(&key_id)
            .expect("second store config reuses the unlock secret");

        let records = koushi_diagnostics::test_support::detail_snapshot().records;
        let unlock_events = records
            .iter()
            .skip(diagnostic_start)
            .filter(|record| {
                record.event.source == "core.store" && record.event.stage == "local_unlock_secret"
            })
            .map(|record| koushi_diagnostics::format_event(&record.event))
            .collect::<Vec<_>>();
        assert!(
            unlock_events
                .iter()
                .any(|line| line.contains("purpose=account_store")
                    && line.contains("outcome=created")),
            "first account store config must say it created the account-local unlock secret"
        );
        assert!(
            unlock_events.iter().any(
                |line| line.contains("purpose=search_index") && line.contains("outcome=loaded")
            ),
            "search index config must say it loaded the existing account-local unlock secret"
        );
        assert!(
            !unlock_events
                .iter()
                .any(|line| line.contains("@alice") || line.contains("DEVICE1")),
            "unlock diagnostics must not leak account identifiers"
        );
    }
    #[test]
    fn delete_account_credentials_does_not_panic_when_absent() {
        let data_dir = tempdir().expect("tempdir");
        let cred_dir = tempdir().expect("tempdir");
        let key_id = make_key_id();

        let actor = file_store_actor(&data_dir, &cred_dir);

        // Should not panic even when credentials don't exist.
        actor
            .delete_account_credentials(&key_id)
            .expect("account credentials delete");
    }
    #[test]
    fn file_credential_store_is_available_to_release_qa_binary_only() {
        let source = include_str!("credential_backend.rs");
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
    fn migrated_credential_vault_reads_keychain_once() {
        let data_dir = tempdir().expect("tempdir");
        let backend = koushi_key::InMemoryCredentialBackend::default();
        let master_key_store = koushi_key::CredentialStore::with_backend(
            CREDENTIAL_STORE_SERVICE_NAME,
            backend.clone(),
        );
        let master_key = koushi_key::CredentialVaultMasterKey::generate();
        master_key_store
            .save_vault_master_key(&master_key)
            .expect("seed master key");
        let alice = make_key_id();
        let bob = SessionKeyId {
            homeserver: "https://test.example.com".to_owned(),
            user_id: "@bob:test.example.com".to_owned(),
            device_id: "DEVICE2".to_owned(),
        };
        let mut vault = crate::credential_vault::CredentialVaultData::default();
        vault.set_last_session(Some(alice.clone()));
        vault.upsert_matrix_session(alice.clone(), "alice-session");
        vault.remember_session(alice.clone());
        vault.upsert_local_unlock_secret(
            alice.clone(),
            LocalUnlockSecret::generate().to_storage_string().as_str(),
        );
        vault.upsert_matrix_session(bob.clone(), "bob-session");
        vault.remember_session(bob.clone());
        vault.upsert_local_unlock_secret(
            bob.clone(),
            LocalUnlockSecret::generate().to_storage_string().as_str(),
        );
        crate::credential_vault::CredentialVaultFile::new(
            data_dir
                .path()
                .join("credentials")
                .join("credentials.v1.enc"),
        )
        .store(&master_key, &vault)
        .expect("seed credential vault");

        let actor = StoreActor::with_os_backend(data_dir.path(), Arc::new(backend.clone()));
        let credentials = actor.credential_backend();
        assert_eq!(
            credentials.load_last_session().expect("last session"),
            Some(alice.clone())
        );
        assert_eq!(
            credentials
                .load_saved_sessions()
                .expect("saved sessions")
                .sessions(),
            &[alice.clone(), bob.clone()]
        );
        assert_eq!(
            credentials
                .load_matrix_session(&alice)
                .expect("alice session")
                .as_str(),
            "alice-session"
        );
        actor.account_store_config(&alice).expect("alice store");
        actor
            .account_search_index_config(&alice)
            .expect("alice search");
        actor
            .load_composer_drafts(&alice)
            .expect("alice composer drafts");
        actor
            .load_scheduled_sends(&alice)
            .expect("alice scheduled sends");
        actor.load_navigation(&alice).expect("alice navigation");
        actor
            .load_room_preferences(&alice)
            .expect("alice room preferences");
        actor
            .load_read_state_outbox(&alice)
            .expect("alice read state outbox");
        assert_eq!(
            credentials
                .load_matrix_session(&bob)
                .expect("bob session")
                .as_str(),
            "bob-session"
        );
        actor.account_store_config(&bob).expect("bob store");

        assert_eq!(backend.get_password_count(), 1);
    }
    #[test]
    fn legacy_credentials_migrate_without_losing_session() {
        let data_dir = tempdir().expect("tempdir");
        let backend = koushi_key::InMemoryCredentialBackend::default();
        let key_id = make_key_id();
        let first_secret = seed_legacy_credentials(&backend, &key_id);
        let second_key_id = SessionKeyId {
            homeserver: "https://test.example.com".to_owned(),
            user_id: "@bob:test.example.com".to_owned(),
            device_id: "DEVICE2".to_owned(),
        };
        let second_secret = seed_legacy_credentials(&backend, &second_key_id);
        koushi_key::CredentialStore::with_backend(CREDENTIAL_STORE_SERVICE_NAME, backend.clone())
            .save_last_session(&key_id)
            .expect("restore first account as last session");

        let actor = StoreActor::with_os_backend(data_dir.path(), Arc::new(backend.clone()));
        let credentials = actor.credential_backend();
        assert_eq!(
            credentials.load_last_session().expect("migrated pointer"),
            Some(key_id.clone())
        );
        assert_eq!(
            credentials
                .load_matrix_session(&key_id)
                .expect("migrated session")
                .as_str(),
            "legacy-session"
        );
        let migrated_first_secret = credentials
            .load(&key_id)
            .expect("migrated unlock secret")
            .to_storage_string();
        let expected_first_secret = first_secret.to_storage_string();
        assert_eq!(
            migrated_first_secret.as_str(),
            expected_first_secret.as_str()
        );
        assert_eq!(
            credentials
                .load_matrix_session(&second_key_id)
                .expect("second migrated session")
                .as_str(),
            "legacy-session"
        );
        let migrated_second_secret = credentials
            .load(&second_key_id)
            .expect("second migrated unlock secret")
            .to_storage_string();
        let expected_second_secret = second_secret.to_storage_string();
        assert_eq!(
            migrated_second_secret.as_str(),
            expected_second_secret.as_str()
        );
        assert!(
            data_dir
                .path()
                .join("credentials")
                .join("credentials.v1.enc")
                .is_file()
        );
        assert!(backend.contains_entry(
            CREDENTIAL_STORE_SERVICE_NAME,
            koushi_key::credential_vault_key_account_name()
        ));
        assert!(!backend.contains_entry(
            CREDENTIAL_STORE_SERVICE_NAME,
            koushi_key::last_session_account_name()
        ));
        assert!(!backend.contains_entry(
            CREDENTIAL_STORE_SERVICE_NAME,
            &key_id.matrix_session_account_name()
        ));
        assert!(!backend.contains_entry(
            CREDENTIAL_STORE_SERVICE_NAME,
            &key_id.local_unlock_account_name()
        ));
        assert!(!backend.contains_entry(
            CREDENTIAL_STORE_SERVICE_NAME,
            &second_key_id.matrix_session_account_name()
        ));
        assert!(!backend.contains_entry(
            CREDENTIAL_STORE_SERVICE_NAME,
            &second_key_id.local_unlock_account_name()
        ));
    }
    fn seed_legacy_credentials(
        backend: &koushi_key::InMemoryCredentialBackend,
        key_id: &SessionKeyId,
    ) -> LocalUnlockSecret {
        let store = koushi_key::CredentialStore::with_backend(
            CREDENTIAL_STORE_SERVICE_NAME,
            backend.clone(),
        );
        let secret = LocalUnlockSecret::generate();
        store.save(key_id, &secret).expect("seed legacy unlock");
        store
            .save_matrix_session(
                key_id,
                &koushi_key::StoredMatrixSession::new("legacy-session"),
            )
            .expect("seed legacy session");
        store
            .remember_saved_session(key_id)
            .expect("seed legacy index");
        store
            .save_last_session(key_id)
            .expect("seed legacy pointer");
        secret
    }
    #[test]
    fn legacy_credentials_missing_entry_preserves_legacy_index() {
        let data_dir = tempdir().expect("tempdir");
        let backend = koushi_key::InMemoryCredentialBackend::default();
        let complete_key_id = make_key_id();
        let _complete_secret = seed_legacy_credentials(&backend, &complete_key_id);
        let key_id = SessionKeyId {
            homeserver: "https://test.example.com".to_owned(),
            user_id: "@incomplete:test.example.com".to_owned(),
            device_id: "INCOMPLETE".to_owned(),
        };
        let legacy = koushi_key::CredentialStore::with_backend(
            CREDENTIAL_STORE_SERVICE_NAME,
            backend.clone(),
        );
        legacy
            .remember_saved_session(&key_id)
            .expect("seed incomplete legacy index");

        let actor = StoreActor::with_os_backend(data_dir.path(), Arc::new(backend.clone()));
        assert!(actor.credential_backend().load_saved_sessions().is_err());
        assert!(backend.contains_entry(
            CREDENTIAL_STORE_SERVICE_NAME,
            koushi_key::saved_sessions_account_name()
        ));
        assert!(backend.contains_entry(
            CREDENTIAL_STORE_SERVICE_NAME,
            &complete_key_id.matrix_session_account_name()
        ));
        assert!(backend.contains_entry(
            CREDENTIAL_STORE_SERVICE_NAME,
            &complete_key_id.local_unlock_account_name()
        ));
        assert!(!backend.contains_entry(
            CREDENTIAL_STORE_SERVICE_NAME,
            koushi_key::credential_vault_key_account_name()
        ));
        assert!(
            !data_dir
                .path()
                .join("credentials")
                .join("credentials.v1.enc")
                .exists()
        );
    }
    #[test]
    fn legacy_credentials_resume_with_existing_master_key() {
        let data_dir = tempdir().expect("tempdir");
        let backend = koushi_key::InMemoryCredentialBackend::default();
        let key_id = make_key_id();
        let _ = seed_legacy_credentials(&backend, &key_id);
        let key_store = koushi_key::CredentialStore::with_backend(
            CREDENTIAL_STORE_SERVICE_NAME,
            backend.clone(),
        );
        key_store
            .save_vault_master_key(&koushi_key::CredentialVaultMasterKey::generate())
            .expect("seed orphan master key");

        let actor = StoreActor::with_os_backend(data_dir.path(), Arc::new(backend));
        assert_eq!(
            actor
                .credential_backend()
                .load_last_session()
                .expect("resumed migration"),
            Some(key_id)
        );
    }
    #[test]
    fn legacy_credentials_delete_failure_keeps_vault_authoritative() {
        let data_dir = tempdir().expect("tempdir");
        let backend = koushi_key::InMemoryCredentialBackend::default();
        let key_id = make_key_id();
        let _ = seed_legacy_credentials(&backend, &key_id);
        backend.set_delete_error(koushi_key::CredentialBackendErrorKind::Unavailable);

        let actor = StoreActor::with_os_backend(data_dir.path(), Arc::new(backend.clone()));
        assert_eq!(
            actor
                .credential_backend()
                .load_matrix_session(&key_id)
                .expect("new vault remains authoritative")
                .as_str(),
            "legacy-session"
        );
        assert!(backend.contains_entry(
            CREDENTIAL_STORE_SERVICE_NAME,
            &key_id.matrix_session_account_name()
        ));
        assert!(
            data_dir
                .path()
                .join("credentials")
                .join("credentials.v1.enc")
                .is_file()
        );

        drop(actor);
        backend.clear_delete_error();
        let restarted = StoreActor::with_os_backend(data_dir.path(), Arc::new(backend.clone()));
        assert_eq!(
            restarted
                .credential_backend()
                .load_matrix_session(&key_id)
                .expect("vault restores while retrying cleanup")
                .as_str(),
            "legacy-session"
        );
        assert!(!backend.contains_entry(
            CREDENTIAL_STORE_SERVICE_NAME,
            &key_id.matrix_session_account_name()
        ));
        assert!(!backend.contains_entry(
            CREDENTIAL_STORE_SERVICE_NAME,
            &key_id.local_unlock_account_name()
        ));
        assert!(!backend.contains_entry(
            CREDENTIAL_STORE_SERVICE_NAME,
            koushi_key::saved_sessions_account_name()
        ));
    }
    #[test]
    fn credential_vault_concurrent_initialization_reads_keychain_once() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
            .records
            .len();
        let data_dir = tempdir().expect("tempdir");
        let backend = koushi_key::InMemoryCredentialBackend::default();
        let key_store = koushi_key::CredentialStore::with_backend(
            CREDENTIAL_STORE_SERVICE_NAME,
            backend.clone(),
        );
        let master_key = koushi_key::CredentialVaultMasterKey::generate();
        key_store
            .save_vault_master_key(&master_key)
            .expect("seed master key");
        crate::credential_vault::CredentialVaultFile::new(
            data_dir
                .path()
                .join("credentials")
                .join("credentials.v1.enc"),
        )
        .store(
            &master_key,
            &crate::credential_vault::CredentialVaultData::default(),
        )
        .expect("seed vault");
        let actor = StoreActor::with_os_backend(data_dir.path(), Arc::new(backend.clone()));
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let threads = (0..8)
            .map(|_| {
                let actor = actor.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    actor
                        .credential_backend()
                        .load_saved_sessions()
                        .expect("concurrent vault read");
                })
            })
            .collect::<Vec<_>>();
        for thread in threads {
            thread.join().expect("join reader");
        }

        assert_eq!(backend.get_password_count(), 1);
        let outcomes = koushi_diagnostics::test_support::detail_snapshot()
            .records
            .into_iter()
            .skip(diagnostic_start)
            .filter(|record| {
                record.event.source == "core.store"
                    && record.event.stage == "credential_vault_access"
            })
            .flat_map(|record| record.event.fields)
            .filter_map(|field| match field.value {
                koushi_diagnostics::DiagnosticValue::Token(outcome) if field.key == "outcome" => {
                    Some(outcome)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(outcomes.contains(&"keychain_read_started"));
        assert!(outcomes.contains(&"keychain_read_succeeded"));
        assert!(outcomes.contains(&"memory_cache_reused"));
    }
    #[test]
    fn credential_vault_initialization_retries_after_transient_keychain_failure() {
        let data_dir = tempdir().expect("tempdir");
        let backend = koushi_key::InMemoryCredentialBackend::default();
        backend.set_error(koushi_key::CredentialBackendErrorKind::LockedOrInaccessible);
        let actor = StoreActor::with_os_backend(data_dir.path(), Arc::new(backend.clone()));

        actor
            .credential_backend()
            .load_saved_sessions()
            .expect_err("locked keychain");
        backend.clear_error();

        assert!(
            actor
                .credential_backend()
                .load_saved_sessions()
                .expect("retry after unlocking keychain")
                .sessions()
                .is_empty()
        );
    }
    #[test]
    fn fresh_saved_session_list_does_not_create_key_or_vault() {
        let data_dir = tempdir().expect("tempdir");
        let backend = koushi_key::InMemoryCredentialBackend::default();
        let actor = StoreActor::with_os_backend(data_dir.path(), Arc::new(backend.clone()));

        assert!(
            actor
                .credential_backend()
                .load_saved_sessions()
                .expect("empty saved sessions")
                .sessions()
                .is_empty()
        );
        assert!(!backend.contains_entry(
            CREDENTIAL_STORE_SERVICE_NAME,
            koushi_key::credential_vault_key_account_name()
        ));
        assert!(
            !data_dir
                .path()
                .join("credentials")
                .join("credentials.v1.enc")
                .exists()
        );
    }
    #[test]
    fn credential_vault_corrupt_file_is_not_overwritten() {
        let data_dir = tempdir().expect("tempdir");
        let backend = koushi_key::InMemoryCredentialBackend::default();
        let key_store = koushi_key::CredentialStore::with_backend(
            CREDENTIAL_STORE_SERVICE_NAME,
            backend.clone(),
        );
        key_store
            .save_vault_master_key(&koushi_key::CredentialVaultMasterKey::generate())
            .expect("seed master key");
        let path = data_dir
            .path()
            .join("credentials")
            .join("credentials.v1.enc");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        let corrupt = b"not-a-credential-vault".to_vec();
        std::fs::write(&path, &corrupt).expect("seed corrupt vault");

        let actor = StoreActor::with_os_backend(data_dir.path(), Arc::new(backend));
        assert!(actor.credential_backend().load_saved_sessions().is_err());
        assert_eq!(std::fs::read(path).expect("read corrupt vault"), corrupt);
    }
    #[test]
    fn os_keychain_does_not_read_legacy_matrix_desktop_service() {
        let data_dir = tempdir().expect("tempdir");
        let backend = koushi_key::InMemoryCredentialBackend::default();
        let backend_dyn: Arc<dyn koushi_key::CredentialBackend> = Arc::new(backend);
        let store = OsCredentialStore::with_backend(data_dir.path(), backend_dyn.clone());
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
}
