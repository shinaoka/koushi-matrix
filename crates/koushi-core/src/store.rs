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

pub(crate) mod composer_drafts;
mod credential_backend;
mod navigation;
mod read_state;
mod room_preferences;
mod scheduled_sends;
#[cfg(test)]
mod test_support;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(any(test, feature = "test-hooks"))]
use std::sync::{Mutex, atomic::AtomicUsize};

use koushi_key::{LocalStoreBinding, LocalStoreId, LocalUnlockSecret, SessionKeyId};

use crate::credential_vault::{
    LocalStoreMigrationRecord, LocalStoreMigrationState, PendingLoginRecord, PendingLoginState,
};
use koushi_sdk::{
    MatrixClientStoreConfig, MatrixClientStoreKey, MatrixSearchIndexKey,
    MatrixSearchIndexStoreConfig,
};
use koushi_state::LocalEncryptionHealth;

use crate::failure::CoreFailure;
pub use credential_backend::{CredentialStoreBackend, OsCredentialStore};
#[cfg(any(debug_assertions, test, feature = "qa-bin"))]
pub use credential_backend::{FileCredentialStore, resolved_credential_backend_is_file_dir};

use composer_drafts::{
    decode_payload_json as decode_composer_draft_payload_json,
    encode_payload_json as encode_composer_draft_payload_json,
};
use credential_backend::{local_secret_error_health, record_local_unlock_secret};

/// Service name used for OS keyring entries. This is user-visible in macOS
/// Keychain Access, so keep it aligned with the shipped product name.
const CREDENTIAL_STORE_SERVICE_NAME: &str = "koushi-desktop";
const COMPOSER_DRAFTS_FILE_MAGIC: &[u8] = b"KOUSHI-DRAFTS-V1\0";
const COMPOSER_DRAFTS_NONCE_LEN: usize = 12;
const PENDING_LOGIN_CAP: u8 = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PendingLoginCleanupEvidence {
    NoRequestSent,
    ServerRejectedBeforeSession,
    Timeout,
    TransportFailure,
    BrowserCancellation,
    CallbackLoss,
    TokenExchangeAmbiguous,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PendingLoginFault {
    None,
    BeforeRootDelete,
    AfterRootDelete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MigrationFault {
    None,
    AfterMarker,
    AfterRename,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct JournalReport {
    pub(crate) deleted_roots: usize,
    pub(crate) parent_syncs: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct MigrationReport {
    pub(crate) parent_syncs: usize,
}

/// Derive a filesystem-safe directory name from a `SessionKeyId`.
/// Uses the same base64url encoding the key crate uses for credential store
/// account names, so both namespaces are consistent.
pub(crate) fn account_dir_name(key_id: &SessionKeyId) -> String {
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
    #[cfg(any(test, feature = "test-hooks"))]
    composer_draft_io_probe: Arc<Mutex<Option<ComposerDraftIoProbe>>>,
    #[cfg(test)]
    composer_draft_replace_fault: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Clone)]
pub(crate) struct PendingLoginJournalOwner<'a> {
    store: &'a StoreActor,
}

impl<'a> PendingLoginJournalOwner<'a> {
    fn new(store: &'a StoreActor) -> Self {
        Self { store }
    }

    pub(crate) fn create(
        &self,
        normalized_homeserver: impl Into<String>,
        auth_method: impl Into<String>,
        device_id: impl Into<String>,
    ) -> Result<PendingLoginRecord, CoreFailure> {
        let mut records = self.load()?;
        let normalized_homeserver = normalized_homeserver.into();
        let auth_method = auth_method.into();
        if records.iter().any(|record| {
            record.normalized_homeserver == normalized_homeserver
                && record.auth_method == auth_method
                && matches!(
                    record.state,
                    PendingLoginState::PreAuth | PendingLoginState::BoundTokenless
                )
        }) {
            return Err(CoreFailure::StoreUnavailable);
        }

        if records.len() >= PENDING_LOGIN_CAP as usize {
            return Err(CoreFailure::StoreUnavailable);
        }
        let slot = (0..PENDING_LOGIN_CAP)
            .find(|slot| records.iter().all(|record| record.slot != *slot))
            .ok_or(CoreFailure::StoreUnavailable)?;
        let binding = LocalStoreBinding::generate();
        let allocation_id = binding.local_store_id().clone();
        let root = self.root(&allocation_id);
        std::fs::create_dir_all(&root).map_err(|_| CoreFailure::StoreUnavailable)?;
        let record = PendingLoginRecord {
            allocation_id: allocation_id.clone(),
            slot,
            attempt_generation: 1,
            normalized_homeserver,
            auth_method,
            device_id: device_id.into(),
            local_store_id: allocation_id,
            binding_secret: binding
                .unlock_secret()
                .to_storage_string()
                .as_str()
                .to_owned(),
            state: PendingLoginState::PreAuth,
            final_session_key_id: None,
        };
        records.push(record.clone());
        if let Err(error) = self.persist(&records) {
            let _ = std::fs::remove_dir_all(root);
            return Err(error);
        }
        Ok(record)
    }

    /// Resume one interrupted authorization on its original store/device. A
    /// new generation makes callbacks from the retired authorization inert.
    pub(crate) fn resume_or_create(
        &self,
        normalized_homeserver: impl Into<String>,
        auth_method: impl Into<String>,
        device_id: impl Into<String>,
    ) -> Result<PendingLoginRecord, CoreFailure> {
        self.reconcile()?;
        let normalized_homeserver = normalized_homeserver.into();
        let auth_method = auth_method.into();
        let mut records = self.load()?;
        if let Some(index) = records.iter().position(|record| {
            record.normalized_homeserver == normalized_homeserver
                && record.auth_method == auth_method
                && matches!(
                    record.state,
                    PendingLoginState::PreAuth | PendingLoginState::BoundTokenless
                )
        }) {
            records[index].attempt_generation = records[index]
                .attempt_generation
                .checked_add(1)
                .ok_or(CoreFailure::StoreUnavailable)?;
            records[index].state = PendingLoginState::PreAuth;
            records[index].final_session_key_id = None;
            let record = records[index].clone();
            self.persist(&records)?;
            return Ok(record);
        }
        drop(records);
        self.create(normalized_homeserver, auth_method, device_id)
    }

    pub(crate) fn is_current(
        &self,
        allocation_id: &LocalStoreId,
        attempt_generation: u64,
    ) -> Result<bool, CoreFailure> {
        Ok(self.load()?.into_iter().any(|record| {
            &record.allocation_id == allocation_id
                && record.attempt_generation == attempt_generation
                && record.state == PendingLoginState::PreAuth
        }))
    }

    pub(crate) fn bind(
        &self,
        allocation_id: &LocalStoreId,
        attempt_generation: u64,
        final_session_key_id: SessionKeyId,
    ) -> Result<(), CoreFailure> {
        let mut records = self.load()?;
        let record = records
            .iter_mut()
            .find(|record| &record.allocation_id == allocation_id)
            .ok_or(CoreFailure::StoreUnavailable)?;
        if record.attempt_generation != attempt_generation
            || !matches!(record.state, PendingLoginState::PreAuth)
        {
            return Err(CoreFailure::StoreUnavailable);
        }
        let secret = LocalUnlockSecret::from_storage_string(&record.binding_secret)
            .map_err(|_| CoreFailure::StoreUnavailable)?;
        self.store
            .credential_store
            .save_local_store_id(&final_session_key_id, &record.local_store_id)
            .map_err(|_| CoreFailure::StoreUnavailable)?;
        self.store
            .credential_store
            .save(&final_session_key_id, &secret)
            .map_err(|_| CoreFailure::StoreUnavailable)?;
        record.final_session_key_id = Some(final_session_key_id);
        record.state = PendingLoginState::BoundTokenless;
        self.persist(&records)
    }

    pub(crate) fn complete_bound(&self, key_id: &SessionKeyId) -> Result<(), CoreFailure> {
        let mut records = self.load()?;
        let Some(index) = records.iter().position(|record| {
            record.state == PendingLoginState::BoundTokenless
                && record.final_session_key_id.as_ref() == Some(key_id)
        }) else {
            return Ok(());
        };
        records.remove(index);
        self.persist(&records)
    }

    pub(crate) fn store_config(
        &self,
        record: &PendingLoginRecord,
    ) -> Result<MatrixClientStoreConfig, CoreFailure> {
        let secret = LocalUnlockSecret::from_storage_string(&record.binding_secret)
            .map_err(|_| CoreFailure::StoreUnavailable)?;
        Ok(self
            .store
            .store_config_for_binding(&record.local_store_id, &secret))
    }

    pub(crate) fn cancel(
        &self,
        allocation_id: &LocalStoreId,
        attempt_generation: u64,
        evidence: PendingLoginCleanupEvidence,
    ) -> Result<(), CoreFailure> {
        let mut records = self.load()?;
        let index = records
            .iter()
            .position(|record| &record.allocation_id == allocation_id)
            .ok_or(CoreFailure::StoreUnavailable)?;
        if records[index].attempt_generation != attempt_generation {
            return Err(CoreFailure::StoreUnavailable);
        }
        if matches!(
            evidence,
            PendingLoginCleanupEvidence::NoRequestSent
                | PendingLoginCleanupEvidence::ServerRejectedBeforeSession
        ) {
            drop(records);
            self.abandon(allocation_id, attempt_generation, PendingLoginFault::None)?;
            Ok(())
        } else {
            records[index].attempt_generation = records[index]
                .attempt_generation
                .checked_add(1)
                .ok_or(CoreFailure::StoreUnavailable)?;
            records[index].state = PendingLoginState::PreAuth;
            records[index].final_session_key_id = None;
            self.persist(&records)
        }
    }

    pub(crate) fn abandon(
        &self,
        allocation_id: &LocalStoreId,
        attempt_generation: u64,
        fault: PendingLoginFault,
    ) -> Result<JournalReport, CoreFailure> {
        let mut records = self.load()?;
        let index = records
            .iter()
            .position(|record| &record.allocation_id == allocation_id)
            .ok_or(CoreFailure::StoreUnavailable)?;
        if records[index].attempt_generation != attempt_generation {
            return Err(CoreFailure::StoreUnavailable);
        }
        records[index].state = PendingLoginState::Abandoning;
        self.persist(&records)?;
        let mut report = JournalReport::default();
        if fault == PendingLoginFault::BeforeRootDelete {
            return Ok(report);
        }
        if self.root_exists(&records[index].local_store_id) {
            self.delete_exact_root(&records[index].local_store_id)?;
            report.deleted_roots = 1;
            report.parent_syncs = 1;
        }
        if fault == PendingLoginFault::AfterRootDelete {
            return Ok(report);
        }
        records.remove(index);
        self.persist(&records)?;
        Ok(report)
    }

    pub(crate) fn reconcile(&self) -> Result<JournalReport, CoreFailure> {
        let mut records = self.load()?;
        self.validate(&records)?;
        let mut report = JournalReport::default();
        let mut remaining = Vec::with_capacity(records.len());
        for record in records.drain(..) {
            if record.state != PendingLoginState::Abandoning {
                remaining.push(record);
                continue;
            }
            let root = self.root(&record.local_store_id);
            if root.exists() && !root.is_dir() {
                return Err(CoreFailure::StoreUnavailable);
            }
            if root.is_dir() {
                self.delete_exact_root(&record.local_store_id)?;
                report.deleted_roots += 1;
                report.parent_syncs += 1;
            }
        }
        if remaining.len() != self.load()?.len() {
            self.persist(&remaining)?;
        }
        Ok(report)
    }

    pub(crate) fn records(&self) -> Result<Vec<PendingLoginRecord>, CoreFailure> {
        let records = self.load()?;
        self.validate(&records)?;
        Ok(records)
    }

    fn load(&self) -> Result<Vec<PendingLoginRecord>, CoreFailure> {
        let value = self
            .store
            .credential_store
            .load_pending_login_journal()
            .map_err(|_| CoreFailure::StoreUnavailable)?;
        value
            .map(|value| serde_json::from_str(&value).map_err(|_| CoreFailure::StoreUnavailable))
            .transpose()
            .map(|records| records.unwrap_or_default())
    }

    fn persist(&self, records: &[PendingLoginRecord]) -> Result<(), CoreFailure> {
        let value = serde_json::to_string(records).map_err(|_| CoreFailure::StoreUnavailable)?;
        self.store
            .credential_store
            .save_pending_login_journal(&value)
            .map_err(|_| CoreFailure::StoreUnavailable)
    }

    fn validate(&self, records: &[PendingLoginRecord]) -> Result<(), CoreFailure> {
        let mut ids = HashSet::new();
        let mut slots = HashSet::new();
        for record in records {
            if record.slot >= PENDING_LOGIN_CAP
                || record.allocation_id != record.local_store_id
                || !ids.insert(record.allocation_id.as_str().to_owned())
                || !slots.insert(record.slot)
                || LocalStoreId::parse(record.local_store_id.as_str()).is_err()
            {
                return Err(CoreFailure::StoreUnavailable);
            }
            let root = self.root(&record.local_store_id);
            if root.exists() && !root.is_dir() {
                return Err(CoreFailure::StoreUnavailable);
            }
            if record.state != PendingLoginState::Abandoning && !root.is_dir() {
                return Err(CoreFailure::StoreUnavailable);
            }
        }
        Ok(())
    }

    fn root(&self, store_id: &LocalStoreId) -> PathBuf {
        self.store
            .data_dir
            .join("accounts")
            .join("v2")
            .join(store_id.as_str())
    }

    fn root_exists(&self, store_id: &LocalStoreId) -> bool {
        self.root(store_id).is_dir()
    }

    fn delete_exact_root(&self, store_id: &LocalStoreId) -> Result<(), CoreFailure> {
        LocalStoreId::parse(store_id.as_str()).map_err(|_| CoreFailure::StoreUnavailable)?;
        let root = self.root(store_id);
        match std::fs::remove_dir_all(root) {
            Ok(()) => self.sync_accounts_parent(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.sync_accounts_parent()
            }
            Err(_) => Err(CoreFailure::StoreUnavailable),
        }
    }

    fn sync_accounts_parent(&self) -> Result<(), CoreFailure> {
        let parent = self.store.data_dir.join("accounts").join("v2");
        std::fs::create_dir_all(&parent).map_err(|_| CoreFailure::StoreUnavailable)?;
        std::fs::File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(|_| CoreFailure::StoreUnavailable)
    }
}

pub(crate) struct LocalStoreMigrationOwner<'a> {
    store: &'a StoreActor,
}

impl<'a> LocalStoreMigrationOwner<'a> {
    fn new(store: &'a StoreActor) -> Self {
        Self { store }
    }

    pub(crate) fn migrate(
        &self,
        key_id: &SessionKeyId,
        store_id: &LocalStoreId,
        fault: MigrationFault,
    ) -> Result<MigrationReport, CoreFailure> {
        let source = self.legacy_root(key_id);
        let destination = self.v2_root(store_id);
        let marker = self
            .store
            .credential_store
            .load_local_store_migration()
            .map_err(|_| CoreFailure::StoreUnavailable)?;
        if let Some(marker) = marker {
            let record: LocalStoreMigrationRecord =
                serde_json::from_str(&marker).map_err(|_| CoreFailure::StoreUnavailable)?;
            if &record.key_id != key_id || &record.local_store_id != store_id {
                return Err(CoreFailure::StoreUnavailable);
            }
            return self.resume(record, source, destination);
        }
        if !source.exists() {
            if destination.is_dir() && self.has_crypto_db(&destination) {
                return Ok(MigrationReport::default());
            }
            if destination.exists() {
                return Err(CoreFailure::StoreUnavailable);
            }
            return Ok(MigrationReport::default());
        }
        if destination.exists() || !self.has_crypto_db(&source) {
            return Err(CoreFailure::StoreUnavailable);
        }
        self.ensure_v2_parent()?;
        let mut report = MigrationReport { parent_syncs: 1 };
        self.save_marker(LocalStoreMigrationRecord {
            key_id: key_id.clone(),
            local_store_id: store_id.clone(),
            state: LocalStoreMigrationState::Marked,
        })?;
        if fault == MigrationFault::AfterMarker {
            return Ok(report);
        }
        std::fs::rename(&source, &destination).map_err(|_| CoreFailure::StoreUnavailable)?;
        self.sync_v2_parent()?;
        report.parent_syncs += 1;
        self.save_marker(LocalStoreMigrationRecord {
            key_id: key_id.clone(),
            local_store_id: store_id.clone(),
            state: LocalStoreMigrationState::Renamed,
        })?;
        if fault == MigrationFault::AfterRename {
            return Ok(report);
        }
        self.finish(&destination)?;
        Ok(report)
    }

    fn resume(
        &self,
        record: LocalStoreMigrationRecord,
        source: PathBuf,
        destination: PathBuf,
    ) -> Result<MigrationReport, CoreFailure> {
        match record.state {
            LocalStoreMigrationState::Marked if source.exists() && !destination.exists() => {
                std::fs::rename(&source, &destination)
                    .map_err(|_| CoreFailure::StoreUnavailable)?;
                self.sync_v2_parent()?;
                self.save_marker(LocalStoreMigrationRecord {
                    state: LocalStoreMigrationState::Renamed,
                    ..record
                })?;
                self.finish(&destination)?;
                Ok(MigrationReport { parent_syncs: 1 })
            }
            LocalStoreMigrationState::Renamed if !source.exists() && destination.is_dir() => {
                self.finish(&destination)?;
                Ok(MigrationReport::default())
            }
            _ => Err(CoreFailure::StoreUnavailable),
        }
    }

    fn finish(&self, destination: &Path) -> Result<(), CoreFailure> {
        if !self.has_crypto_db(destination) {
            return Err(CoreFailure::StoreUnavailable);
        }
        self.store
            .credential_store
            .delete_local_store_migration()
            .map_err(|_| CoreFailure::StoreUnavailable)
    }

    fn save_marker(&self, record: LocalStoreMigrationRecord) -> Result<(), CoreFailure> {
        let value = serde_json::to_string(&record).map_err(|_| CoreFailure::StoreUnavailable)?;
        self.store
            .credential_store
            .save_local_store_migration(&value)
            .map_err(|_| CoreFailure::StoreUnavailable)
    }

    fn ensure_v2_parent(&self) -> Result<(), CoreFailure> {
        let parent = self.store.data_dir.join("accounts").join("v2");
        std::fs::create_dir_all(&parent).map_err(|_| CoreFailure::StoreUnavailable)?;
        self.sync_v2_parent()
    }

    fn sync_v2_parent(&self) -> Result<(), CoreFailure> {
        let parent = self.store.data_dir.join("accounts").join("v2");
        std::fs::File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(|_| CoreFailure::StoreUnavailable)
    }

    fn has_crypto_db(&self, root: &Path) -> bool {
        root.join("store")
            .join("matrix-sdk-crypto.sqlite3")
            .is_file()
    }

    fn legacy_root(&self, key_id: &SessionKeyId) -> PathBuf {
        self.store
            .data_dir
            .join("accounts")
            .join(account_dir_name(key_id))
    }

    fn v2_root(&self, store_id: &LocalStoreId) -> PathBuf {
        self.store
            .data_dir
            .join("accounts")
            .join("v2")
            .join(store_id.as_str())
    }
}

#[cfg(any(test, feature = "test-hooks"))]
struct ComposerDraftIoProbe {
    save_started: Option<tokio::sync::oneshot::Sender<()>>,
    save_release: Option<std::sync::mpsc::Receiver<()>>,
    save_completed: Option<tokio::sync::oneshot::Sender<()>>,
    load_started: Option<tokio::sync::oneshot::Sender<()>>,
    load_completed: Option<tokio::sync::oneshot::Sender<()>>,
    load_attempt_count: Arc<AtomicUsize>,
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
            #[cfg(any(test, feature = "test-hooks"))]
            composer_draft_io_probe: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            composer_draft_replace_fault: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Create the actor with an injected OS credential store backend.
    /// Used by production `CoreRuntime::start_with_data_dir_and_os_backend`.
    pub fn with_os_backend(
        data_dir: impl Into<PathBuf>,
        os_backend: Arc<dyn koushi_key::CredentialBackend>,
    ) -> Self {
        let data_dir = data_dir.into();
        Self {
            credential_store: CredentialStoreBackend::resolve_with_os_backend(
                data_dir.clone(),
                os_backend,
            ),
            data_dir,
            #[cfg(any(test, feature = "test-hooks"))]
            composer_draft_io_probe: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            composer_draft_replace_fault: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Access the credential store backend (for session persistence in AccountActor).
    pub fn credential_backend(&self) -> &CredentialStoreBackend {
        &self.credential_store
    }

    pub(crate) fn pending_login_owner(&self) -> PendingLoginJournalOwner<'_> {
        PendingLoginJournalOwner::new(self)
    }

    pub(crate) fn local_store_migration_owner(&self) -> LocalStoreMigrationOwner<'_> {
        LocalStoreMigrationOwner::new(self)
    }

    /// QA/test constructor with an explicit credential backend. This avoids the
    /// env-global `KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR` race between unit tests
    /// and lets the headless QA binary isolate same-user device fixtures.
    #[cfg(any(test, feature = "test-hooks", feature = "qa-bin"))]
    pub(crate) fn with_backend(
        credential_store: CredentialStoreBackend,
        data_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            credential_store,
            data_dir: data_dir.into(),
            #[cfg(any(test, feature = "test-hooks"))]
            composer_draft_io_probe: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            composer_draft_replace_fault: Arc::new(std::sync::atomic::AtomicBool::new(false)),
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
        let binding = self.load_or_create_binding(key_id, "account_store")?;
        self.migrate_legacy_root(key_id, binding.local_store_id())?;
        Ok(AccountStoreConfig {
            store_config: self
                .store_config_for_binding(binding.local_store_id(), binding.unlock_secret()),
            session_key_id: key_id.clone(),
        })
    }

    /// Resolve a saved account without creating credentials, a store id, or a
    /// crypto database. Legacy roots are migrated only when the saved secret
    /// and the existing crypto root are both present.
    pub(crate) fn existing_account_store_config(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<AccountStoreConfig, CoreFailure> {
        let secret = self.load_unlock_secret(key_id)?;
        let store_id = match self.credential_store.load_local_store_id(key_id) {
            Ok(store_id) => store_id,
            Err(error) if koushi_key::is_missing_credential_error(&error) => {
                let legacy = self
                    .data_dir
                    .join("accounts")
                    .join(account_dir_name(key_id));
                if !legacy
                    .join("store")
                    .join("matrix-sdk-crypto.sqlite3")
                    .is_file()
                {
                    return Err(CoreFailure::LocalEncryptionUnavailable);
                }
                let store_id = LocalStoreId::generate();
                self.migrate_legacy_root(key_id, &store_id)?;
                self.credential_store
                    .save_local_store_id(key_id, &store_id)
                    .map_err(|_| CoreFailure::LocalEncryptionUnavailable)?;
                store_id
            }
            Err(_) => return Err(CoreFailure::LocalEncryptionUnavailable),
        };
        self.migrate_legacy_root(key_id, &store_id)?;
        Ok(AccountStoreConfig {
            store_config: self.store_config_for_binding(&store_id, &secret),
            session_key_id: key_id.clone(),
        })
    }

    fn store_config_for_binding(
        &self,
        store_id: &LocalStoreId,
        secret: &LocalUnlockSecret,
    ) -> MatrixClientStoreConfig {
        let sdk_store_key = secret.derive_sdk_store_key();
        let store_key = MatrixClientStoreKey::new(*sdk_store_key.as_bytes());
        let search_key = secret.derive_search_index_key();
        let root = self
            .data_dir
            .join("accounts")
            .join("v2")
            .join(store_id.as_str());
        MatrixClientStoreConfig::new(root.join("store"), store_key)
            .with_cache_path(root.join("cache"))
            .with_search_index_store(MatrixSearchIndexStoreConfig::new(
                root.join("search-index"),
                MatrixSearchIndexKey::new(search_key.as_str()),
            ))
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
        let binding = self.load_or_create_binding(key_id, "search_index")?;
        self.migrate_legacy_root(key_id, binding.local_store_id())?;
        let search_key = binding.unlock_secret().derive_search_index_key();
        let search_dir = self.account_search_index_dir(key_id);
        let config = MatrixSearchIndexStoreConfig::new(
            &search_dir,
            MatrixSearchIndexKey::new(search_key.as_str()),
        );
        Ok(AccountSearchIndexConfig {
            search_index_config: config,
        })
    }

    /// Delete the stored unlock secret and the per-account store/cache
    /// directories for an account (shutdown step 7: "clear credentials and
    /// stores"). Called during logout / account removal.
    ///
    /// Errors do not propagate — a logout that partially cleans up is better
    /// than a logout that fails. Matrix session JSON / pointers stored via the
    /// credential backend are cleaned up by AccountActor through the same
    /// backend.
    pub fn delete_account_credentials(&self, key_id: &SessionKeyId) -> Result<(), ()> {
        let root = self.account_root_dir(key_id);
        let credential_deleted = self.credential_store.delete(key_id).is_ok()
            && self.credential_store.delete_local_store_id(key_id).is_ok();
        let directory_deleted = match std::fs::remove_dir_all(root) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Err(_) => false,
        };
        if credential_deleted && directory_deleted {
            Ok(())
        } else {
            Err(())
        }
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
        self.load_or_create_unlock_secret_with_diagnostic(key_id, None)
    }

    fn load_or_create_unlock_secret_for(
        &self,
        key_id: &SessionKeyId,
        purpose: &'static str,
    ) -> Result<LocalUnlockSecret, CoreFailure> {
        self.load_or_create_unlock_secret_with_diagnostic(key_id, Some(purpose))
    }

    fn load_or_create_binding(
        &self,
        key_id: &SessionKeyId,
        purpose: &'static str,
    ) -> Result<LocalStoreBinding, CoreFailure> {
        let secret = self.load_or_create_unlock_secret_for(key_id, purpose)?;
        let store_id = match self.credential_store.load_local_store_id(key_id) {
            Ok(store_id) => store_id,
            Err(error) if koushi_key::is_missing_credential_error(&error) => {
                let store_id = LocalStoreId::generate();
                self.credential_store
                    .save_local_store_id(key_id, &store_id)
                    .map_err(|_| CoreFailure::LocalEncryptionUnavailable)?;
                store_id
            }
            Err(_) => return Err(CoreFailure::LocalEncryptionUnavailable),
        };
        Ok(LocalStoreBinding::new(store_id, secret))
    }

    fn load_or_create_unlock_secret_with_diagnostic(
        &self,
        key_id: &SessionKeyId,
        purpose: Option<&'static str>,
    ) -> Result<LocalUnlockSecret, CoreFailure> {
        match self.credential_store.load(key_id) {
            Ok(secret) => {
                record_local_unlock_secret(purpose, "loaded");
                Ok(secret)
            }
            Err(err) if koushi_key::is_missing_credential_error(&err) => {
                // First use: generate and persist a new unlock secret.
                let secret = LocalUnlockSecret::generate();
                if self.credential_store.save(key_id, &secret).is_err() {
                    record_local_unlock_secret(purpose, "save_failed");
                    return Err(CoreFailure::LocalEncryptionUnavailable);
                }
                record_local_unlock_secret(purpose, "created");
                Ok(secret)
            }
            Err(_) => {
                record_local_unlock_secret(purpose, "load_failed");
                Err(CoreFailure::LocalEncryptionUnavailable)
            }
        }
    }

    fn load_unlock_secret(&self, key_id: &SessionKeyId) -> Result<LocalUnlockSecret, CoreFailure> {
        self.credential_store
            .load(key_id)
            .map_err(|_| CoreFailure::LocalEncryptionUnavailable)
    }

    fn account_root_dir(&self, key_id: &SessionKeyId) -> PathBuf {
        let accounts = self.data_dir.join("accounts");
        match self.credential_store.load_local_store_id(key_id) {
            Ok(store_id) => accounts.join("v2").join(store_id.as_str()),
            Err(_) => accounts.join(account_dir_name(key_id)),
        }
    }

    fn migrate_legacy_root(
        &self,
        key_id: &SessionKeyId,
        store_id: &LocalStoreId,
    ) -> Result<(), CoreFailure> {
        self.local_store_migration_owner()
            .migrate(key_id, store_id, MigrationFault::None)
            .map(|_| ())
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
}

fn atomic_replace_file(
    path: &std::path::Path,
    payload: &[u8],
    fail_before_persist: bool,
) -> Result<(), CoreFailure> {
    use std::io::Write as _;

    let parent = path.parent().ok_or(CoreFailure::StoreUnavailable)?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|_| CoreFailure::StoreUnavailable)?;
    temporary
        .write_all(payload)
        .map_err(|_| CoreFailure::StoreUnavailable)?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| CoreFailure::StoreUnavailable)?;
    if fail_before_persist {
        return Err(CoreFailure::StoreUnavailable);
    }
    temporary
        .persist(path)
        .map(|_| ())
        .map_err(|_| CoreFailure::StoreUnavailable)
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
