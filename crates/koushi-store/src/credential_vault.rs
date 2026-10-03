use std::{fmt, fs, path::PathBuf};

use koushi_key::{CredentialVaultMasterKey, LocalStoreId, SavedSessionIndex};
use koushi_protocol::SessionKeyId;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

const CREDENTIAL_VAULT_FILE_MAGIC: &[u8] = b"KOUSHI-CREDENTIAL-VAULT-V1\0";
const CREDENTIAL_VAULT_VERSION: u8 = 2;
const CREDENTIAL_VAULT_MAX_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone)]
pub struct CredentialVaultData {
    last_session: Option<SessionKeyId>,
    saved_sessions: Vec<SessionKeyId>,
    entries: Vec<CredentialVaultEntry>,
    legacy_cleanup_pending: Vec<SessionKeyId>,
    pending_logins: Vec<PendingLoginRecord>,
    local_store_migration: Option<LocalStoreMigrationRecord>,
    payload_version: u8,
}

impl Default for CredentialVaultData {
    fn default() -> Self {
        Self {
            last_session: None,
            saved_sessions: Vec::new(),
            entries: Vec::new(),
            legacy_cleanup_pending: Vec::new(),
            pending_logins: Vec::new(),
            local_store_migration: None,
            payload_version: CREDENTIAL_VAULT_VERSION,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct CredentialVaultEntry {
    key_id: SessionKeyId,
    matrix_session: Option<String>,
    local_unlock_secret: Option<String>,
    #[serde(default)]
    local_store_id: Option<LocalStoreId>,
}

impl Drop for CredentialVaultEntry {
    fn drop(&mut self) {
        if let Some(session) = self.matrix_session.as_mut() {
            session.zeroize();
        }
        if let Some(secret) = self.local_unlock_secret.as_mut() {
            secret.zeroize();
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PendingLoginRecord {
    pub allocation_id: LocalStoreId,
    pub slot: u8,
    pub attempt_generation: u64,
    pub normalized_homeserver: String,
    pub auth_method: String,
    pub device_id: String,
    pub local_store_id: LocalStoreId,
    /// Storage form of the local unlock secret. It is encrypted in the OS
    /// vault and is only exposed to the journal owner while materializing a
    /// binding.
    pub binding_secret: String,
    pub state: PendingLoginState,
    pub final_session_key_id: Option<SessionKeyId>,
}

impl Drop for PendingLoginRecord {
    fn drop(&mut self) {
        self.binding_secret.zeroize();
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct LocalStoreMigrationRecord {
    pub key_id: SessionKeyId,
    pub local_store_id: LocalStoreId,
    #[serde(default)]
    pub state: LocalStoreMigrationState,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, Default, Eq, PartialEq)]
pub enum LocalStoreMigrationState {
    #[default]
    Marked,
    Renamed,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, Eq, PartialEq)]
pub enum PendingLoginState {
    PreAuth,
    BoundTokenless,
    Abandoning,
    Persisted,
}

#[derive(Serialize, Deserialize)]
struct CredentialVaultPayload {
    version: u8,
    last_session: Option<SessionKeyId>,
    saved_sessions: Vec<SessionKeyId>,
    entries: Vec<CredentialVaultEntry>,
    #[serde(default)]
    legacy_cleanup_pending: Vec<SessionKeyId>,
    #[serde(default)]
    pending_logins: Vec<PendingLoginRecord>,
    #[serde(default)]
    local_store_migration: Option<LocalStoreMigrationRecord>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialVaultError {
    Unavailable,
    Corrupt,
}

impl CredentialVaultData {
    pub fn last_session(&self) -> Option<&SessionKeyId> {
        self.last_session.as_ref()
    }

    pub fn set_last_session(&mut self, key_id: Option<SessionKeyId>) {
        self.last_session = key_id;
    }

    pub fn saved_sessions(&self) -> SavedSessionIndex {
        let mut index = SavedSessionIndex::new();
        for key_id in &self.saved_sessions {
            index.upsert(key_id.clone());
        }
        index
    }

    pub fn remember_session(&mut self, key_id: SessionKeyId) {
        if !self.saved_sessions.contains(&key_id) {
            self.saved_sessions.push(key_id);
        }
    }

    pub fn forget_session(&mut self, key_id: &SessionKeyId) {
        self.saved_sessions.retain(|saved| saved != key_id);
    }

    pub fn matrix_session(&self, key_id: &SessionKeyId) -> Option<&str> {
        self.entry(key_id)
            .and_then(|entry| entry.matrix_session.as_deref())
    }

    pub fn upsert_matrix_session(&mut self, key_id: SessionKeyId, session: impl Into<String>) {
        let entry = self.entry_mut(key_id);
        zeroize_replaced_string(&mut entry.matrix_session);
        entry.matrix_session = Some(session.into());
    }

    pub fn delete_matrix_session(&mut self, key_id: &SessionKeyId) {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| &entry.key_id == key_id)
            && let Some(mut session) = entry.matrix_session.take()
        {
            session.zeroize();
        }
    }

    pub fn local_unlock_secret(&self, key_id: &SessionKeyId) -> Option<&str> {
        self.entry(key_id)
            .and_then(|entry| entry.local_unlock_secret.as_deref())
    }

    pub fn upsert_local_unlock_secret(&mut self, key_id: SessionKeyId, secret: impl Into<String>) {
        let entry = self.entry_mut(key_id);
        zeroize_replaced_string(&mut entry.local_unlock_secret);
        entry.local_unlock_secret = Some(secret.into());
    }

    pub fn delete_local_unlock_secret(&mut self, key_id: &SessionKeyId) {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| &entry.key_id == key_id)
            && let Some(mut secret) = entry.local_unlock_secret.take()
        {
            secret.zeroize();
        }
    }

    pub fn local_store_id(&self, key_id: &SessionKeyId) -> Option<&LocalStoreId> {
        self.entry(key_id)
            .and_then(|entry| entry.local_store_id.as_ref())
    }

    pub fn upsert_local_store_id(&mut self, key_id: SessionKeyId, store_id: LocalStoreId) {
        self.entry_mut(key_id).local_store_id = Some(store_id);
    }

    pub fn delete_local_store_id(&mut self, key_id: &SessionKeyId) {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| &entry.key_id == key_id)
        {
            entry.local_store_id = None;
        }
    }

    pub fn payload_version(&self) -> u8 {
        self.payload_version
    }

    pub fn mark_current_version(&mut self) {
        self.payload_version = CREDENTIAL_VAULT_VERSION;
    }

    pub fn pending_logins(&self) -> &[PendingLoginRecord] {
        &self.pending_logins
    }

    pub fn pending_logins_mut(&mut self) -> &mut Vec<PendingLoginRecord> {
        &mut self.pending_logins
    }

    pub fn local_store_migration(&self) -> Option<&LocalStoreMigrationRecord> {
        self.local_store_migration.as_ref()
    }

    pub fn set_local_store_migration(&mut self, migration: LocalStoreMigrationRecord) {
        self.local_store_migration = Some(migration);
    }

    pub fn clear_local_store_migration(&mut self) -> bool {
        self.local_store_migration.take().is_some()
    }

    pub fn legacy_cleanup_pending(&self) -> &[SessionKeyId] {
        &self.legacy_cleanup_pending
    }

    pub fn set_legacy_cleanup_pending(&mut self, key_ids: Vec<SessionKeyId>) {
        self.legacy_cleanup_pending = key_ids;
    }

    pub fn clear_legacy_cleanup_pending(&mut self) {
        self.legacy_cleanup_pending.clear();
    }

    fn entry(&self, key_id: &SessionKeyId) -> Option<&CredentialVaultEntry> {
        self.entries.iter().find(|entry| &entry.key_id == key_id)
    }

    fn entry_mut(&mut self, key_id: SessionKeyId) -> &mut CredentialVaultEntry {
        if let Some(index) = self.entries.iter().position(|entry| entry.key_id == key_id) {
            return &mut self.entries[index];
        }
        self.entries.push(CredentialVaultEntry {
            key_id,
            matrix_session: None,
            local_unlock_secret: None,
            local_store_id: None,
        });
        self.entries.last_mut().expect("entry was inserted")
    }
}

impl fmt::Debug for CredentialVaultData {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialVaultData")
            .field("entry_count", &self.entries.len())
            .field("saved_session_count", &self.saved_sessions.len())
            .field("has_last_session", &self.last_session.is_some())
            .field(
                "legacy_cleanup_pending_count",
                &self.legacy_cleanup_pending.len(),
            )
            .field("pending_login_count", &self.pending_logins.len())
            .field(
                "has_local_store_migration",
                &self.local_store_migration.is_some(),
            )
            .field("payload_version", &self.payload_version)
            .finish()
    }
}

#[derive(Clone)]
pub struct CredentialVaultFile {
    path: PathBuf,
}

impl fmt::Debug for CredentialVaultFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CredentialVaultFile(..)")
    }
}

impl CredentialVaultFile {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn exists(&self) -> bool {
        self.path.is_file()
    }

    pub fn load(
        &self,
        master_key: &CredentialVaultMasterKey,
    ) -> Result<CredentialVaultData, CredentialVaultError> {
        let payload = fs::read(&self.path).map_err(|_| CredentialVaultError::Unavailable)?;
        decrypt_payload(master_key, &payload)
    }

    pub fn store(
        &self,
        master_key: &CredentialVaultMasterKey,
        data: &CredentialVaultData,
    ) -> Result<(), CredentialVaultError> {
        self.store_with_fault(master_key, data, false)
    }

    fn store_with_fault(
        &self,
        master_key: &CredentialVaultMasterKey,
        data: &CredentialVaultData,
        fail_before_persist: bool,
    ) -> Result<(), CredentialVaultError> {
        let payload = encrypt_payload(master_key, data, CREDENTIAL_VAULT_VERSION)?;
        crate::atomic_replace_file(&self.path, &payload, fail_before_persist)
            .map_err(|_| CredentialVaultError::Unavailable)
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn store_version_for_test(
        &self,
        master_key: &CredentialVaultMasterKey,
        data: &CredentialVaultData,
        version: u8,
    ) -> Result<(), CredentialVaultError> {
        let payload = encrypt_payload(master_key, data, version)?;
        crate::atomic_replace_file(&self.path, &payload, false)
            .map_err(|_| CredentialVaultError::Unavailable)
    }
}

fn encrypt_payload(
    master_key: &CredentialVaultMasterKey,
    data: &CredentialVaultData,
    version: u8,
) -> Result<Vec<u8>, CredentialVaultError> {
    let payload = CredentialVaultPayload {
        version,
        last_session: data.last_session.clone(),
        saved_sessions: data.saved_sessions.clone(),
        entries: data.entries.clone(),
        legacy_cleanup_pending: data.legacy_cleanup_pending.clone(),
        pending_logins: data.pending_logins.clone(),
        local_store_migration: data.local_store_migration.clone(),
    };
    let plaintext =
        Zeroizing::new(serde_json::to_vec(&payload).map_err(|_| CredentialVaultError::Corrupt)?);
    if plaintext.len() > CREDENTIAL_VAULT_MAX_BYTES {
        return Err(CredentialVaultError::Unavailable);
    }
    crate::encrypt_envelope(
        CREDENTIAL_VAULT_FILE_MAGIC,
        master_key.as_bytes(),
        plaintext.as_ref(),
        CREDENTIAL_VAULT_MAX_BYTES,
    )
    .map_err(|_| CredentialVaultError::Unavailable)
}

fn decrypt_payload(
    master_key: &CredentialVaultMasterKey,
    encrypted: &[u8],
) -> Result<CredentialVaultData, CredentialVaultError> {
    let plaintext = Zeroizing::new(
        crate::decrypt_envelope(
            CREDENTIAL_VAULT_FILE_MAGIC,
            master_key.as_bytes(),
            encrypted,
            CREDENTIAL_VAULT_MAX_BYTES,
        )
        .map_err(|_| CredentialVaultError::Corrupt)?,
    );
    let payload: CredentialVaultPayload =
        serde_json::from_slice(&plaintext).map_err(|_| CredentialVaultError::Corrupt)?;
    if !matches!(payload.version, 1 | CREDENTIAL_VAULT_VERSION) {
        return Err(CredentialVaultError::Corrupt);
    }
    Ok(CredentialVaultData {
        last_session: payload.last_session,
        saved_sessions: payload.saved_sessions,
        entries: payload.entries,
        legacy_cleanup_pending: payload.legacy_cleanup_pending,
        pending_logins: payload.pending_logins,
        local_store_migration: payload.local_store_migration,
        payload_version: payload.version,
    })
}

fn zeroize_replaced_string(value: &mut Option<String>) {
    if let Some(mut previous) = value.take() {
        previous.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use koushi_key::{CredentialVaultMasterKey, LocalUnlockSecret};
    use koushi_protocol::SessionKeyId;
    use tempfile::tempdir;

    use super::{CredentialVaultData, CredentialVaultFile};

    fn session(name: &str) -> SessionKeyId {
        SessionKeyId {
            homeserver: format!("https://{name}.invalid"),
            user_id: format!("@{name}:invalid"),
            device_id: format!("{name}-device"),
        }
    }

    #[test]
    fn credential_vault_file_round_trips_without_plaintext() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("credentials").join("credentials.v1.enc");
        let file = CredentialVaultFile::new(path.clone());
        let key = CredentialVaultMasterKey::generate();
        let alice = session("alice");
        let bob = session("bob");
        let alice_unlock = LocalUnlockSecret::generate().to_storage_string();
        let bob_unlock = LocalUnlockSecret::generate().to_storage_string();
        let mut data = CredentialVaultData::default();
        data.set_last_session(Some(alice.clone()));
        data.upsert_matrix_session(alice.clone(), "alice-session");
        data.upsert_local_unlock_secret(alice.clone(), alice_unlock.as_str());
        data.upsert_matrix_session(bob.clone(), "bob-session");
        data.upsert_local_unlock_secret(bob.clone(), bob_unlock.as_str());

        file.store(&key, &data).expect("store vault");
        let encrypted = fs::read(&path).expect("read encrypted vault");
        assert!(
            !encrypted
                .windows(b"alice-session".len())
                .any(|window| window == b"alice-session")
        );
        assert!(
            !encrypted
                .windows(alice.user_id.len())
                .any(|window| window == alice.user_id.as_bytes())
        );

        let restored = file.load(&key).expect("load vault");
        assert_eq!(restored.last_session(), Some(&alice));
        assert_eq!(restored.matrix_session(&alice), Some("alice-session"));
        assert_eq!(
            restored.local_unlock_secret(&alice),
            Some(alice_unlock.as_str())
        );
        assert_eq!(restored.matrix_session(&bob), Some("bob-session"));
        assert_eq!(
            restored.local_unlock_secret(&bob),
            Some(bob_unlock.as_str())
        );
    }

    #[test]
    fn credential_vault_file_creates_missing_parent() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("nested").join("credentials.v1.enc");
        let file = CredentialVaultFile::new(path.clone());
        let key = CredentialVaultMasterKey::generate();

        file.store(&key, &CredentialVaultData::default())
            .expect("store vault");

        assert!(path.is_file());
    }

    #[test]
    fn credential_vault_file_rejects_modified_ciphertext() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("credentials.v1.enc");
        let file = CredentialVaultFile::new(path.clone());
        let key = CredentialVaultMasterKey::generate();
        file.store(&key, &CredentialVaultData::default())
            .expect("store vault");
        let mut encrypted = fs::read(&path).expect("read vault");
        let last = encrypted.last_mut().expect("ciphertext byte");
        *last ^= 0x80;
        fs::write(&path, encrypted).expect("corrupt vault");

        assert!(file.load(&key).is_err());
    }

    #[test]
    fn credential_vault_file_failed_replace_preserves_previous_payload() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("credentials.v1.enc");
        let file = CredentialVaultFile::new(path.clone());
        let key = CredentialVaultMasterKey::generate();
        let mut original = CredentialVaultData::default();
        original.set_last_session(Some(session("original")));
        file.store(&key, &original).expect("store original");
        let original_bytes = fs::read(&path).expect("read original");

        let mut replacement = CredentialVaultData::default();
        replacement.set_last_session(Some(session("replacement")));
        file.store_with_fault(&key, &replacement, true)
            .expect_err("injected failure");

        assert_eq!(fs::read(path).expect("read preserved"), original_bytes);
    }

    #[test]
    fn credential_vault_file_does_not_infer_saved_session_from_partial_credentials() {
        let account = session("partial");
        let mut data = CredentialVaultData::default();
        data.upsert_local_unlock_secret(
            account.clone(),
            LocalUnlockSecret::generate().to_storage_string().as_str(),
        );
        data.upsert_matrix_session(account.clone(), "session");

        assert!(data.saved_sessions().sessions().is_empty());
        data.remember_session(account.clone());
        assert_eq!(data.saved_sessions().sessions(), &[account]);
    }

    #[test]
    fn credential_vault_file_forget_preserves_credentials_and_last_pointer() {
        let account = session("rollback");
        let unlock = LocalUnlockSecret::generate().to_storage_string();
        let mut data = CredentialVaultData::default();
        data.set_last_session(Some(account.clone()));
        data.upsert_local_unlock_secret(account.clone(), unlock.as_str());
        data.upsert_matrix_session(account.clone(), "session");
        data.remember_session(account.clone());

        data.forget_session(&account);

        assert!(data.saved_sessions().sessions().is_empty());
        assert_eq!(data.last_session(), Some(&account));
        assert_eq!(data.matrix_session(&account), Some("session"));
        assert_eq!(data.local_unlock_secret(&account), Some(unlock.as_str()));
    }

    #[test]
    fn credential_vault_debug_redacts_credentials() {
        let account = session("debug-secret-user");
        let unlock = LocalUnlockSecret::generate().to_storage_string();
        let mut data = CredentialVaultData::default();
        data.upsert_matrix_session(account.clone(), "debug-secret-session");
        data.upsert_local_unlock_secret(account, unlock.as_str());

        let debug = format!("{data:?}");
        assert!(!debug.contains("debug-secret-user"));
        assert!(!debug.contains("debug-secret-session"));
        assert!(!debug.contains(unlock.as_str()));

        let file = CredentialVaultFile::new(std::path::PathBuf::from(
            "/synthetic/private/credentials.v1.enc",
        ));
        assert_eq!(format!("{file:?}"), "CredentialVaultFile(..)");
    }
}
