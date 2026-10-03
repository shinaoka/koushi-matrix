use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[cfg(any(debug_assertions, test, feature = "test-hooks"))]
use koushi_key::SessionKeyIdCredentialNames;
use koushi_key::{CredentialStore, LocalUnlockSecret};
use koushi_protocol::SessionKeyId;
use koushi_state::LocalEncryptionHealth;

use crate::CREDENTIAL_STORE_SERVICE_NAME;

/// Env var for QA/debug file-based credential store override.
/// Only honored in debug/test/test-hooks builds; production release builds ignore it.
#[cfg(any(debug_assertions, test, feature = "test-hooks"))]
const ENV_FILE_CREDENTIAL_STORE_DIR: &str = "KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR";

/// Credential store backend. Production = either OS keychain (injected from
/// the platform layer) or in-memory; debug/test/test-hooks may use a file dir
/// override when `KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR` is set.
#[derive(Clone)]
pub enum CredentialStoreBackend {
    OsKeychain(OsCredentialStore),
    #[cfg(any(debug_assertions, test, feature = "test-hooks"))]
    FileDir(FileCredentialStore),
    InMemory(CredentialStore<koushi_key::InMemoryCredentialBackend>),
}

impl CredentialStoreBackend {
    pub fn resolve() -> Self {
        #[cfg(any(debug_assertions, test, feature = "test-hooks"))]
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

    pub fn resolve_with_os_backend(
        data_dir: PathBuf,
        os_backend: Arc<dyn koushi_key::CredentialBackend>,
    ) -> Self {
        #[cfg(any(debug_assertions, test, feature = "test-hooks"))]
        if let Ok(dir) = std::env::var(ENV_FILE_CREDENTIAL_STORE_DIR) {
            let dir = PathBuf::from(dir);
            record_file_credential_store_active();
            return Self::FileDir(FileCredentialStore::new(dir));
        }
        Self::OsKeychain(OsCredentialStore::with_backend(data_dir, os_backend))
    }

    pub fn load(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<LocalUnlockSecret, koushi_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.load(key_id),
            #[cfg(any(debug_assertions, test, feature = "test-hooks"))]
            Self::FileDir(store) => store.load(key_id),
            Self::InMemory(store) => store.load(key_id),
        }
    }

    pub fn save(
        &self,
        key_id: &SessionKeyId,
        secret: &LocalUnlockSecret,
    ) -> Result<(), koushi_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.save(key_id, secret),
            #[cfg(any(debug_assertions, test, feature = "test-hooks"))]
            Self::FileDir(store) => store.save(key_id, secret),
            Self::InMemory(store) => store.save(key_id, secret),
        }
    }

    pub fn delete(&self, key_id: &SessionKeyId) -> Result<(), koushi_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.delete(key_id),
            #[cfg(any(debug_assertions, test, feature = "test-hooks"))]
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
            #[cfg(any(debug_assertions, test, feature = "test-hooks"))]
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
            #[cfg(any(debug_assertions, test, feature = "test-hooks"))]
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
            #[cfg(any(debug_assertions, test, feature = "test-hooks"))]
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
            #[cfg(any(debug_assertions, test, feature = "test-hooks"))]
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
            #[cfg(any(debug_assertions, test, feature = "test-hooks"))]
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
            #[cfg(any(debug_assertions, test, feature = "test-hooks"))]
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
            #[cfg(any(debug_assertions, test, feature = "test-hooks"))]
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
            #[cfg(any(debug_assertions, test, feature = "test-hooks"))]
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
            #[cfg(any(debug_assertions, test, feature = "test-hooks"))]
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
            #[cfg(any(debug_assertions, test, feature = "test-hooks"))]
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
            #[cfg(any(debug_assertions, test, feature = "test-hooks"))]
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
            #[cfg(any(debug_assertions, test, feature = "test-hooks"))]
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
            #[cfg(any(debug_assertions, test, feature = "test-hooks"))]
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
            #[cfg(any(debug_assertions, test, feature = "test-hooks"))]
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
            #[cfg(any(debug_assertions, test, feature = "test-hooks"))]
            Self::FileDir(store) => store.delete_named(koushi_key::last_session_account_name()),
            Self::InMemory(store) => store.delete_last_session(),
        }
    }

    pub fn load_saved_sessions(
        &self,
    ) -> Result<koushi_key::SavedSessionIndex, koushi_key::LocalSecretError> {
        match self {
            Self::OsKeychain(store) => store.load_saved_sessions(),
            #[cfg(any(debug_assertions, test, feature = "test-hooks"))]
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
            #[cfg(any(debug_assertions, test, feature = "test-hooks"))]
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
            #[cfg(any(debug_assertions, test, feature = "test-hooks"))]
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

pub fn local_secret_error_health(error: &koushi_key::LocalSecretError) -> LocalEncryptionHealth {
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

// --- File-based credential store (debug/test/test-hooks only) ---

/// A trivial file-based credential store used in unattended QA runs that
/// cannot prompt macOS Keychain. Stored as plain files under `dir`; each
/// entry is a separate file named after the account.
///
/// COMPILE-TIME GATE: only present in debug/test/test-hooks builds.
/// Production release builds must not include this type.
#[cfg(any(debug_assertions, test, feature = "test-hooks"))]
#[derive(Clone)]
pub struct FileCredentialStore {
    dir: PathBuf,
}

#[cfg(any(debug_assertions, test, feature = "test-hooks"))]
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
    fn save_named(&self, name: &str, value: &str) -> Result<(), koushi_key::LocalSecretError> {
        self.ensure_dir()?;
        self.write_file(&self.named_file(name), value)
    }

    /// Load an arbitrary named credential.
    fn load_named(&self, name: &str) -> Result<String, koushi_key::LocalSecretError> {
        let path = self.named_file(name);
        std::fs::read_to_string(&path).map_err(|_| {
            koushi_key::LocalSecretError::CredentialBackend(
                koushi_key::CredentialBackendErrorKind::MissingCredential,
            )
        })
    }

    /// Delete an arbitrary named credential (no error if absent).
    fn delete_named(&self, name: &str) -> Result<(), koushi_key::LocalSecretError> {
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
#[cfg(any(debug_assertions, test, feature = "test-hooks"))]
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

/// Debug/test/test-hooks-only diagnostic helper. Compiled out of production release
/// builds along with its only call site (the file credential store branch in
/// `CredentialStoreBackend::resolve`).
#[cfg(any(debug_assertions, test, feature = "test-hooks"))]
fn record_file_credential_store_active() {
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "core.store", "credential_store")
            .field(DiagnosticField::token("outcome", "file_backend_active")),
    );
}

pub fn record_local_unlock_secret(purpose: Option<&'static str>, outcome: &'static str) {
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
/// `KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR` is set in a debug/test/test-hooks
/// build). Headless QA binaries call this BEFORE any login so unattended runs
/// are structurally unable to reach the OS keychain (engineering-rules
/// Secrets rule: keychain prompts during automation are failures).
///
/// Production release builds have no file backend, so this symbol does not
/// exist there and an app release cannot silently opt into file credentials.
#[cfg(any(debug_assertions, test, feature = "test-hooks"))]
pub fn resolved_credential_backend_is_file_dir() -> bool {
    matches!(
        CredentialStoreBackend::resolve(),
        CredentialStoreBackend::FileDir(_)
    )
}

#[cfg(test)]
mod tests;
