#![recursion_limit = "256"]

mod commands;
mod dto;

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex as TokioMutex;

use tauri::{
    Emitter, Manager,
    menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder},
};

use serde::{Deserialize, Serialize};

// matrix-desktop-core: the production runtime host.
use matrix_desktop_core::{
    AccountCommand, CoreCommand, CoreConnection, CoreEvent, CoreRuntime, TimelineEvent,
    event::AppStateSnapshot,
};

// matrix-desktop-auth / key: still used for credential store access during startup
// restore and saved-session listing (design gap: ideally these would be
// AccountCommand::RestoreLastSession and AccountCommand::QuerySavedSessions).
use matrix_desktop_auth::{
    MatrixClientSession, MatrixClientStoreConfig, MatrixClientStoreKey, MatrixSearchIndexKey,
    MatrixSearchIndexStoreConfig, PersistableMatrixSession,
};
use matrix_desktop_key::{
    CredentialStore, LastSessionPointer, LocalUnlockSecret, SavedSessionIndex, SessionKeyId,
    StoredMatrixSession, is_missing_credential_error,
};
use matrix_desktop_state::SessionInfo;

// matrix-desktop-backend: fixture/demo preview only; never on a production
// Matrix path (overview.md: "fixture/demo data only").
use matrix_desktop_backend::{
    E2eeRecoveryMode, FakeDesktopBackend, FakeDesktopBackendConfig, LoginDiscoveryMode, LoginMode,
    SyncMode,
};

const CREDENTIAL_SERVICE_NAME: &str = "matrix-desktop";
const MENU_EVENT_NAME: &str = "matrix-desktop://menu";
/// Tauri event for serialized CoreEvent payloads (discrete events + diff batches).
pub(crate) const CORE_EVENT_NAME: &str = "matrix-desktop://event";
/// Tauri event for serialized AppStateSnapshot payloads (latest-wins).
const STATE_EVENT_NAME: &str = "matrix-desktop://state";
const MENU_ID_OPEN_USER_SETTINGS: &str = "open_user_settings";
const MENU_ID_SHOW_KEYBOARD_SETTINGS: &str = "show_keyboard_settings";
const MENU_ID_TOGGLE_RIGHT_PANEL: &str = "toggle_right_panel";
const MIN_RESTORABLE_WINDOW_WIDTH: u32 = 760;
const MIN_RESTORABLE_WINDOW_HEIGHT: u32 = 620;
#[cfg(any(debug_assertions, test))]
const QA_LOGIN_PIPE_ENV: &str = "MATRIX_DESKTOP_QA_LOGIN_PIPE";
#[cfg(any(debug_assertions, test))]
const QA_FILE_CREDENTIAL_STORE_DIR_ENV: &str = "MATRIX_DESKTOP_QA_FILE_CREDENTIAL_STORE_DIR";
const SKIP_KEYCHAIN_PERSISTENCE_ENV: &str = "MATRIX_DESKTOP_SKIP_KEYCHAIN_PERSISTENCE";
const SEARCH_INDEX_METADATA_FILE: &str = ".matrix-desktop-search-index.json";
const SEARCH_INDEX_SCHEMA_VERSION: u32 = 1;
const SEARCH_INDEX_TOKENIZER: &str = "ngram";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DesktopMenuItem {
    pub id: &'static str,
    pub label: &'static str,
    pub menu: &'static str,
    pub accelerator: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg(test)]
pub(crate) struct DesktopStandardMenuItem {
    pub id: &'static str,
    pub label: &'static str,
    pub menu: &'static str,
    pub accelerator: &'static str,
}

pub(crate) fn desktop_menu_items() -> Vec<DesktopMenuItem> {
    vec![
        DesktopMenuItem {
            id: MENU_ID_OPEN_USER_SETTINGS,
            label: "User Settings",
            menu: "app",
            accelerator: "CmdOrCtrl+,",
        },
        DesktopMenuItem {
            id: MENU_ID_TOGGLE_RIGHT_PANEL,
            label: "Toggle Right Panel",
            menu: "view",
            accelerator: "CmdOrCtrl+.",
        },
        DesktopMenuItem {
            id: MENU_ID_SHOW_KEYBOARD_SETTINGS,
            label: "Keyboard Shortcuts",
            menu: "help",
            accelerator: "CmdOrCtrl+/",
        },
    ]
}

#[cfg(test)]
pub(crate) fn desktop_standard_menu_items() -> Vec<DesktopStandardMenuItem> {
    vec![
        DesktopStandardMenuItem {
            id: "close_window",
            label: "Close Window",
            menu: "file",
            accelerator: "CmdOrCtrl+W",
        },
        DesktopStandardMenuItem {
            id: "quit",
            label: "Quit",
            menu: "app",
            accelerator: "CmdOrCtrl+Q",
        },
    ]
}

fn desktop_menu_action_id(menu_id: &str) -> Option<&'static str> {
    match menu_id {
        MENU_ID_OPEN_USER_SETTINGS => Some("openUserSettings"),
        MENU_ID_TOGGLE_RIGHT_PANEL => Some("toggleRightPanel"),
        MENU_ID_SHOW_KEYBOARD_SETTINGS => Some("showKeyboardSettings"),
        _ => None,
    }
}

/// Transport-adapter state.
///
/// Holds the `CoreRuntime` (the only production runtime owner) plus one
/// `CoreConnection` for command dispatch and snapshot reads.
///
/// The event-forwarding task owns a SECOND connection (obtained by calling
/// `runtime.attach()` in `run()`) so it can loop on `recv_event` without
/// blocking command dispatch.
///
/// DESIGN GAPS (canon-first escalation required before resolving):
/// 1. Startup `RestoreLastSession`: no `AccountCommand::RestoreLastSession`
///    exists in the canon yet. The adapter temporarily reads the credential
///    store directly to find the last-session `AccountKey`, then sends
///    `AccountCommand::RestoreSession`. See `STARTUP_RESTORE_DESIGN_GAP`.
/// 2. `list_saved_sessions`: no `AccountCommand::QuerySavedSessions` and no
///    `AppState.saved_sessions` in the canon yet. The adapter temporarily
///    reads the credential store directly. See `SAVED_SESSIONS_DESIGN_GAP`.
/// 3. `timeline_items_count`: `AppState` snapshots never embed timeline lists
///    (Async rule 4). The count needed for `qa_window_title` is tracked here
///    via a Tauri-side counter updated by the event forwarding loop.
pub struct CoreRuntimeState {
    pub(crate) runtime: CoreRuntime,
    /// Command-dispatch connection. Uses `tokio::sync::Mutex` so the guard can
    /// be held across `.await` points in async Tauri command handlers.
    pub(crate) connection: TokioMutex<CoreConnection>,
    /// Tauri-side timeline item count (updated by event loop; QA title only).
    pub(crate) timeline_items_count: AtomicUsize,
}

/// Fixture backend for browser-only dev/demo preview.
///
/// This is the non-Tauri path. It is NEVER constructed on a production Matrix
/// path; it exists only so the React components can be previewed in a browser
/// without a running Tauri process.
#[allow(dead_code)]
pub struct BackendState {
    backend: Mutex<FakeDesktopBackend>,
    sync_task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    timeline_task: Mutex<Option<TimelineTaskHandle>>,
}

#[allow(dead_code)]
pub(crate) struct TimelineTaskHandle {
    room_id: String,
    task: tauri::async_runtime::JoinHandle<()>,
    pagination_sender: tokio::sync::mpsc::Sender<TimelinePaginationRequest>,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TimelinePaginationRequest {
    pub room_id: String,
    pub event_count: u16,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct PersistedWindowState {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct SearchIndexMetadata {
    schema_version: u32,
    tokenizer: String,
}

impl Default for BackendState {
    fn default() -> Self {
        let config = FakeDesktopBackendConfig {
            restore_session: false,
            login_discovery: LoginDiscoveryMode::Http,
            login: LoginMode::Deferred,
            e2ee_recovery: E2eeRecoveryMode::SdkState,
            sync: SyncMode::Deferred,
            ..FakeDesktopBackendConfig::default()
        };
        let mut backend = FakeDesktopBackend::new(config);
        backend.boot();

        Self {
            backend: Mutex::new(backend),
            sync_task: Mutex::new(None),
            timeline_task: Mutex::new(None),
        }
    }
}

fn restore_session_enabled_from_env_value(value: Option<&str>) -> bool {
    !matches!(
        value.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("0" | "false" | "signed-out")
    )
}

fn saved_sessions_disabled_from_env_value(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("1" | "true" | "yes")
    )
}

#[cfg(any(debug_assertions, test))]
fn qa_login_pipe_path_from_env_value(value: Option<&str>) -> Option<PathBuf> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

// Release builds must not honor credential injection through the QA login
// pipe (engineering-rules: Secrets rule 2).
#[cfg(any(debug_assertions, test))]
fn qa_login_pipe_path_from_env() -> Option<PathBuf> {
    qa_login_pipe_path_from_env_value(std::env::var(QA_LOGIN_PIPE_ENV).ok().as_deref())
}

#[cfg(any(debug_assertions, test))]
fn qa_file_credential_store_dir_from_env_value(value: Option<&str>) -> Option<PathBuf> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(any(debug_assertions, test))]
fn qa_file_credential_store_dir_from_env() -> Option<PathBuf> {
    qa_file_credential_store_dir_from_env_value(
        std::env::var(QA_FILE_CREDENTIAL_STORE_DIR_ENV)
            .ok()
            .as_deref(),
    )
}

#[cfg(not(any(debug_assertions, test)))]
fn qa_file_credential_store_dir_from_env() -> Option<PathBuf> {
    None
}

fn qa_skips_keychain_persistence_from_env_value(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("1" | "true" | "yes")
    )
}

pub(crate) fn qa_skips_keychain_persistence_from_env() -> bool {
    qa_skips_keychain_persistence_from_env_value(
        std::env::var(SKIP_KEYCHAIN_PERSISTENCE_ENV).ok().as_deref(),
    )
}

#[derive(Debug, Eq, PartialEq)]
enum DesktopCredentialError {
    Missing,
    Store,
}

#[derive(Clone, Debug)]
enum DesktopCredentialStore {
    Os(CredentialStore),
    QaFile(QaFileCredentialStore),
}

impl DesktopCredentialStore {
    fn save(
        &self,
        key_id: &SessionKeyId,
        secret: &LocalUnlockSecret,
    ) -> Result<(), DesktopCredentialError> {
        match self {
            Self::Os(store) => store
                .save(key_id, secret)
                .map_err(desktop_credential_error_from_keyring),
            Self::QaFile(store) => store.save(key_id, secret),
        }
    }

    fn load(&self, key_id: &SessionKeyId) -> Result<LocalUnlockSecret, DesktopCredentialError> {
        match self {
            Self::Os(store) => store
                .load(key_id)
                .map_err(desktop_credential_error_from_keyring),
            Self::QaFile(store) => store.load(key_id),
        }
    }

    fn delete(&self, key_id: &SessionKeyId) -> Result<(), DesktopCredentialError> {
        match self {
            Self::Os(store) => store
                .delete(key_id)
                .map_err(desktop_credential_error_from_keyring),
            Self::QaFile(store) => store.delete(key_id),
        }
    }

    fn save_matrix_session(
        &self,
        key_id: &SessionKeyId,
        session: &StoredMatrixSession,
    ) -> Result<(), DesktopCredentialError> {
        match self {
            Self::Os(store) => store
                .save_matrix_session(key_id, session)
                .map_err(desktop_credential_error_from_keyring),
            Self::QaFile(store) => store.save_matrix_session(key_id, session),
        }
    }

    fn load_matrix_session(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<StoredMatrixSession, DesktopCredentialError> {
        match self {
            Self::Os(store) => store
                .load_matrix_session(key_id)
                .map_err(desktop_credential_error_from_keyring),
            Self::QaFile(store) => store.load_matrix_session(key_id),
        }
    }

    fn delete_matrix_session(&self, key_id: &SessionKeyId) -> Result<(), DesktopCredentialError> {
        match self {
            Self::Os(store) => store
                .delete_matrix_session(key_id)
                .map_err(desktop_credential_error_from_keyring),
            Self::QaFile(store) => store.delete_matrix_session(key_id),
        }
    }

    fn save_last_session(&self, key_id: &SessionKeyId) -> Result<(), DesktopCredentialError> {
        match self {
            Self::Os(store) => store
                .save_last_session(key_id)
                .map_err(desktop_credential_error_from_keyring),
            Self::QaFile(store) => {
                let pointer = LastSessionPointer::new(key_id.clone());
                let pointer_json = pointer
                    .to_json()
                    .map_err(|_| DesktopCredentialError::Store)?;
                store
                    .save_account_value(CredentialStore::last_session_account_name(), &pointer_json)
            }
        }
    }

    fn load_last_session(&self) -> Result<Option<SessionKeyId>, DesktopCredentialError> {
        match self {
            Self::Os(store) => store
                .load_last_session()
                .map_err(desktop_credential_error_from_keyring),
            Self::QaFile(store) => {
                let pointer_json =
                    match store.load_account_value(CredentialStore::last_session_account_name()) {
                        Ok(pointer_json) => pointer_json,
                        Err(DesktopCredentialError::Missing) => return Ok(None),
                        Err(error) => return Err(error),
                    };
                LastSessionPointer::from_json(&pointer_json)
                    .map(|pointer| Some(pointer.session_key_id().clone()))
                    .map_err(|_| DesktopCredentialError::Store)
            }
        }
    }

    fn delete_last_session(&self) -> Result<(), DesktopCredentialError> {
        match self {
            Self::Os(store) => store
                .delete_last_session()
                .map_err(desktop_credential_error_from_keyring),
            Self::QaFile(store) => {
                store.delete_account_value(CredentialStore::last_session_account_name())
            }
        }
    }

    fn load_saved_sessions(&self) -> Result<SavedSessionIndex, DesktopCredentialError> {
        match self {
            Self::Os(store) => store
                .load_saved_sessions()
                .map_err(desktop_credential_error_from_keyring),
            Self::QaFile(store) => {
                let index_json = match store
                    .load_account_value(CredentialStore::saved_sessions_account_name())
                {
                    Ok(index_json) => index_json,
                    Err(DesktopCredentialError::Missing) => return Ok(SavedSessionIndex::new()),
                    Err(error) => return Err(error),
                };
                SavedSessionIndex::from_json(&index_json).map_err(|_| DesktopCredentialError::Store)
            }
        }
    }

    fn save_saved_sessions(&self, index: &SavedSessionIndex) -> Result<(), DesktopCredentialError> {
        match self {
            Self::Os(store) => store
                .save_saved_sessions(index)
                .map_err(desktop_credential_error_from_keyring),
            Self::QaFile(store) => {
                let index_json = index.to_json().map_err(|_| DesktopCredentialError::Store)?;
                store
                    .save_account_value(CredentialStore::saved_sessions_account_name(), &index_json)
            }
        }
    }

    fn remember_saved_session(&self, key_id: &SessionKeyId) -> Result<(), DesktopCredentialError> {
        let mut index = self.load_saved_sessions()?;
        index.upsert(key_id.clone());
        self.save_saved_sessions(&index)
    }

    fn forget_saved_session(&self, key_id: &SessionKeyId) -> Result<(), DesktopCredentialError> {
        let mut index = self.load_saved_sessions()?;
        index.remove(key_id);
        self.save_saved_sessions(&index)
    }
}

#[derive(Clone, Debug)]
struct QaFileCredentialStore {
    base_dir: PathBuf,
}

impl QaFileCredentialStore {
    fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    fn save(
        &self,
        key_id: &SessionKeyId,
        secret: &LocalUnlockSecret,
    ) -> Result<(), DesktopCredentialError> {
        let storage_string = secret.to_storage_string();
        self.save_account_value(
            key_id.local_unlock_account_name().as_str(),
            storage_string.as_str(),
        )
    }

    fn load(&self, key_id: &SessionKeyId) -> Result<LocalUnlockSecret, DesktopCredentialError> {
        let stored_secret = self.load_account_value(key_id.local_unlock_account_name().as_str())?;
        LocalUnlockSecret::from_storage_string(&stored_secret)
            .map_err(|_| DesktopCredentialError::Store)
    }

    fn delete(&self, key_id: &SessionKeyId) -> Result<(), DesktopCredentialError> {
        self.delete_account_value(key_id.local_unlock_account_name().as_str())
    }

    fn save_matrix_session(
        &self,
        key_id: &SessionKeyId,
        session: &StoredMatrixSession,
    ) -> Result<(), DesktopCredentialError> {
        self.save_account_value(
            key_id.matrix_session_account_name().as_str(),
            session.as_str(),
        )
    }

    fn load_matrix_session(
        &self,
        key_id: &SessionKeyId,
    ) -> Result<StoredMatrixSession, DesktopCredentialError> {
        self.load_account_value(key_id.matrix_session_account_name().as_str())
            .map(StoredMatrixSession::new)
    }

    fn delete_matrix_session(&self, key_id: &SessionKeyId) -> Result<(), DesktopCredentialError> {
        self.delete_account_value(key_id.matrix_session_account_name().as_str())
    }

    fn save_account_value(
        &self,
        account_name: &str,
        value: &str,
    ) -> Result<(), DesktopCredentialError> {
        let path = self.account_path(account_name);
        let parent = path.parent().ok_or(DesktopCredentialError::Store)?;
        fs::create_dir_all(parent).map_err(|_| DesktopCredentialError::Store)?;
        let tmp_path = secret_tmp_path(&path);
        write_secret_file(&tmp_path, value.as_bytes())
            .map_err(|_| DesktopCredentialError::Store)?;
        fs::rename(&tmp_path, &path).map_err(|_| DesktopCredentialError::Store)?;
        set_secret_file_permissions(&path).map_err(|_| DesktopCredentialError::Store)
    }

    fn load_account_value(&self, account_name: &str) -> Result<String, DesktopCredentialError> {
        match fs::read_to_string(self.account_path(account_name)) {
            Ok(value) => Ok(value),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(DesktopCredentialError::Missing)
            }
            Err(_) => Err(DesktopCredentialError::Store),
        }
    }

    fn delete_account_value(&self, account_name: &str) -> Result<(), DesktopCredentialError> {
        match fs::remove_file(self.account_path(account_name)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(DesktopCredentialError::Store),
        }
    }

    fn account_path(&self, account_name: &str) -> PathBuf {
        self.base_dir
            .join("v1")
            .join(format!("{}.secret", hex_file_name(account_name)))
    }
}

fn desktop_credential_store() -> DesktopCredentialStore {
    if let Some(base_dir) = qa_file_credential_store_dir_from_env() {
        DesktopCredentialStore::QaFile(QaFileCredentialStore::new(base_dir))
    } else {
        DesktopCredentialStore::Os(CredentialStore::new(CREDENTIAL_SERVICE_NAME))
    }
}

fn desktop_credential_error_from_keyring(
    error: matrix_desktop_key::LocalSecretError,
) -> DesktopCredentialError {
    if is_missing_credential_error(&error) {
        DesktopCredentialError::Missing
    } else {
        DesktopCredentialError::Store
    }
}

fn hex_file_name(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value.as_bytes() {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn secret_tmp_path(path: &Path) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    path.with_extension(format!("tmp-{}-{nanos}", std::process::id()))
}

#[cfg(unix)]
fn write_secret_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)
}

#[cfg(not(unix))]
fn write_secret_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    fs::write(path, bytes)
}

#[cfg(unix)]
fn set_secret_file_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(path, permissions)
}

#[cfg(not(unix))]
fn set_secret_file_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

// ---- Session persistence helpers (DESIGN GAP: ideally StoreActor owns all of
// this; these remain in the adapter until AccountCommand::RestoreLastSession
// and AccountCommand::QuerySavedSessions land in the canon). ----

pub(crate) fn persist_matrix_session(session: &MatrixClientSession) -> Result<(), String> {
    let persistable = session
        .persistable_session()
        .map_err(|_| "session persistence failed".to_owned())?;
    let key_id = session_key_id_from_info(&persistable.info);
    let session_json = persistable
        .to_json()
        .map_err(|_| "session persistence failed".to_owned())?;
    let stored_session = StoredMatrixSession::new(session_json);
    let store = desktop_credential_store();

    store
        .save_matrix_session(&key_id, &stored_session)
        .map_err(|_| "session persistence failed".to_owned())?;
    if store.remember_saved_session(&key_id).is_err() {
        let _ = store.delete_matrix_session(&key_id);
        return Err("session persistence failed".to_owned());
    }
    if store.save_last_session(&key_id).is_err() {
        let _ = store.delete_matrix_session(&key_id);
        let _ = store.forget_saved_session(&key_id);
        return Err("session persistence failed".to_owned());
    }

    Ok(())
}

pub(crate) async fn restore_matrix_session_with_local_store(
    session: &MatrixClientSession,
) -> Result<MatrixClientSession, String> {
    let persistable = session
        .persistable_session()
        .map_err(|_| "session persistence failed".to_owned())?;
    let key_id = session_key_id_from_info(&persistable.info);
    let store = desktop_credential_store();
    restore_persistable_matrix_session_with_store_retry(
        &persistable,
        &key_id,
        &matrix_desktop_data_dir()?,
        &store,
        "encrypted session store initialization failed",
    )
    .await
}

pub(crate) fn clear_persisted_matrix_session(info: &SessionInfo) -> Result<(), String> {
    let store = desktop_credential_store();
    clear_persisted_matrix_session_with_base(info, &matrix_desktop_data_dir()?, &store)
}

fn clear_persisted_matrix_session_with_base(
    info: &SessionInfo,
    base_dir: &Path,
    store: &DesktopCredentialStore,
) -> Result<(), String> {
    let key_id = session_key_id_from_info(info);
    store
        .delete(&key_id)
        .map_err(|_| "local store credential could not be deleted".to_owned())?;
    store
        .delete_matrix_session(&key_id)
        .map_err(|_| "session credential could not be deleted".to_owned())?;
    store
        .forget_saved_session(&key_id)
        .map_err(|_| "saved session index could not be updated".to_owned())?;
    match store.load_last_session() {
        Ok(Some(last_session)) if last_session == key_id => store
            .delete_last_session()
            .map_err(|_| "last session pointer could not be deleted".to_owned())?,
        Ok(_) => {}
        Err(_) => store
            .delete_last_session()
            .map_err(|_| "last session pointer could not be deleted".to_owned())?,
    }
    remove_local_store_paths(base_dir, &key_id)
}

/// STARTUP_RESTORE_DESIGN_GAP: reads the credential store to find the last
/// session `AccountKey`. Temporary until `AccountCommand::RestoreLastSession`
/// is added to the canon (Phase 9 cleanup). Returns `None` if no last session
/// exists. Returns `Err` only if the credential store is unreachable.
///
/// After finding the key, the caller sends `AccountCommand::RestoreSession`
/// with the derived `AccountKey`; the `AccountActor` completes the restore
/// using its own `StoreActor` (which also has the credential store).
pub(crate) fn last_session_account_key_from_store(
) -> Result<Option<matrix_desktop_core::AccountKey>, String> {
    let store = desktop_credential_store();
    let key_id = match store
        .load_last_session()
        .map_err(|_| "session restore failed".to_owned())?
    {
        Some(key_id) => key_id,
        None => return Ok(None),
    };
    // Verify the session data exists before asking core to restore it.
    if store.load_matrix_session(&key_id).is_err() {
        return Ok(None);
    }
    Ok(Some(matrix_desktop_core::AccountKey(key_id.user_id)))
}

/// SAVED_SESSIONS_DESIGN_GAP: reads the credential store to list all saved
/// sessions. Temporary until `AppState.saved_sessions` or
/// `AccountCommand::QuerySavedSessions` lands in the canon.
pub(crate) fn saved_matrix_session_infos() -> Result<Vec<SessionInfo>, String> {
    if saved_sessions_disabled_from_env_value(
        std::env::var("MATRIX_DESKTOP_SKIP_SAVED_SESSIONS")
            .ok()
            .as_deref(),
    ) {
        return Ok(Vec::new());
    }

    let store = desktop_credential_store();
    let index = store
        .load_saved_sessions()
        .map_err(|_| "saved sessions could not be loaded".to_owned())?;
    Ok(saved_session_infos_from_index(&index))
}

pub(crate) fn mark_last_matrix_session(info: &SessionInfo) -> Result<(), String> {
    let store = desktop_credential_store();
    store
        .save_last_session(&session_key_id_from_info(info))
        .map_err(|_| "last session pointer could not be saved".to_owned())
}

async fn restore_persisted_matrix_session_for_info(
    info: &SessionInfo,
) -> Result<MatrixClientSession, String> {
    let key_id = session_key_id_from_info(info);
    restore_persisted_matrix_session_for_key_id(&key_id).await
}

async fn restore_persisted_matrix_session_for_key_id(
    key_id: &SessionKeyId,
) -> Result<MatrixClientSession, String> {
    let store = desktop_credential_store();
    let stored_session = store
        .load_matrix_session(key_id)
        .map_err(|_| "session restore failed".to_owned())?;
    let persistable = PersistableMatrixSession::from_json(stored_session.as_str())
        .map_err(|_| "session restore failed".to_owned())?;
    restore_persistable_matrix_session_with_store_retry(
        &persistable,
        key_id,
        &matrix_desktop_data_dir()?,
        &store,
        "session restore failed",
    )
    .await
}

async fn restore_persistable_matrix_session_with_store_retry(
    persistable: &PersistableMatrixSession,
    key_id: &SessionKeyId,
    base_dir: &Path,
    store: &DesktopCredentialStore,
    error_message: &'static str,
) -> Result<MatrixClientSession, String> {
    let store_config =
        matrix_client_store_config_for_session_with_base(&persistable.info, base_dir, store)?;
    match matrix_desktop_auth::restore_session_with_store(persistable, Some(&store_config)).await {
        Ok(session) => Ok(session),
        Err(_) => {
            quarantine_local_store_paths(base_dir, key_id)?;
            let retry_store_config = matrix_client_store_config_for_session_with_base(
                &persistable.info,
                base_dir,
                store,
            )?;
            matrix_desktop_auth::restore_session_with_store(persistable, Some(&retry_store_config))
                .await
                .map_err(|_| error_message.to_owned())
        }
    }
}

fn matrix_client_store_config_for_session_with_base(
    info: &SessionInfo,
    base_dir: &Path,
    store: &DesktopCredentialStore,
) -> Result<MatrixClientStoreConfig, String> {
    let key_id = session_key_id_from_info(info);
    let paths = local_store_paths(base_dir, &key_id);
    let local_secret = load_or_create_local_unlock_secret(store, &key_id, &paths.store_path)?;

    std::fs::create_dir_all(&paths.store_path)
        .map_err(|_| "encrypted session store directory could not be created".to_owned())?;
    std::fs::create_dir_all(&paths.cache_path)
        .map_err(|_| "encrypted session cache directory could not be created".to_owned())?;
    prepare_search_index_path(&paths.search_index_path)?;

    let sdk_store_key = local_secret.derive_sdk_store_key();
    let search_index_key = local_secret.derive_search_index_key();
    Ok(MatrixClientStoreConfig::new(
        paths.store_path,
        MatrixClientStoreKey::new(*sdk_store_key.as_bytes()),
    )
    .with_cache_path(paths.cache_path)
    .with_search_index_store(MatrixSearchIndexStoreConfig::new(
        paths.search_index_path,
        MatrixSearchIndexKey::new(search_index_key.as_str().to_owned()),
    )))
}

pub(crate) fn matrix_desktop_data_dir() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("MATRIX_DESKTOP_DATA_DIR") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    dirs::data_local_dir()
        .map(|path| path.join("matrix-desktop"))
        .ok_or_else(|| "local application data directory is unavailable".to_owned())
}

fn window_state_path(base_dir: &Path) -> PathBuf {
    base_dir.join("app-shell").join("window-state.json")
}

fn persisted_window_state_is_restorable(state: &PersistedWindowState) -> bool {
    state.width >= MIN_RESTORABLE_WINDOW_WIDTH && state.height >= MIN_RESTORABLE_WINDOW_HEIGHT
}

fn persisted_window_state_from_geometry(
    position: tauri::PhysicalPosition<i32>,
    size: tauri::PhysicalSize<u32>,
    maximized: bool,
) -> PersistedWindowState {
    PersistedWindowState {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
        maximized,
    }
}

fn window_event_should_persist(event: &tauri::WindowEvent) -> bool {
    matches!(
        event,
        tauri::WindowEvent::Resized(_)
            | tauri::WindowEvent::Moved(_)
            | tauri::WindowEvent::ScaleFactorChanged { .. }
            | tauri::WindowEvent::CloseRequested { .. }
            | tauri::WindowEvent::Destroyed
    )
}

fn window_event_should_stop_background_tasks(event: &tauri::WindowEvent) -> bool {
    matches!(
        event,
        tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed
    )
}

fn load_window_state_with_base(base_dir: &Path) -> Result<Option<PersistedWindowState>, String> {
    let path = window_state_path(base_dir);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("window state could not be read".to_owned()),
    };

    let state = match serde_json::from_slice::<PersistedWindowState>(&bytes) {
        Ok(state) => state,
        Err(_) => return Ok(None),
    };

    Ok(persisted_window_state_is_restorable(&state).then_some(state))
}

fn load_window_state() -> Result<Option<PersistedWindowState>, String> {
    load_window_state_with_base(&matrix_desktop_data_dir()?)
}

fn persist_window_state_with_base(
    base_dir: &Path,
    state: &PersistedWindowState,
) -> Result<(), String> {
    if !persisted_window_state_is_restorable(state) {
        return Ok(());
    }

    let path = window_state_path(base_dir);
    let parent = path
        .parent()
        .ok_or_else(|| "window state path is invalid".to_owned())?;
    std::fs::create_dir_all(parent)
        .map_err(|_| "window state directory could not be created".to_owned())?;

    let tmp_path = parent.join("window-state.json.tmp");
    let json =
        serde_json::to_vec(state).map_err(|_| "window state could not be serialized".to_owned())?;
    std::fs::write(&tmp_path, json).map_err(|_| "window state could not be written".to_owned())?;
    std::fs::rename(&tmp_path, &path)
        .map_err(|_| "window state could not be committed".to_owned())?;
    Ok(())
}

fn persist_window_state(state: &PersistedWindowState) -> Result<(), String> {
    persist_window_state_with_base(&matrix_desktop_data_dir()?, state)
}

fn apply_persisted_window_state<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
    state: PersistedWindowState,
) -> Result<(), String> {
    if !persisted_window_state_is_restorable(&state) {
        return Ok(());
    }

    window
        .set_size(tauri::Size::Physical(tauri::PhysicalSize::new(
            state.width,
            state.height,
        )))
        .map_err(|_| "window size could not be restored".to_owned())?;
    window
        .set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(
            state.x, state.y,
        )))
        .map_err(|_| "window position could not be restored".to_owned())?;
    if state.maximized {
        window
            .maximize()
            .map_err(|_| "window maximized state could not be restored".to_owned())?;
    }
    Ok(())
}

fn restore_main_window_state<R: tauri::Runtime, M: Manager<R>>(manager: &M) -> Result<(), String> {
    let Some(window) = manager.get_webview_window("main") else {
        return Ok(());
    };
    let Some(state) = load_window_state()? else {
        return Ok(());
    };
    apply_persisted_window_state(&window, state)
}

fn persisted_window_state_from_window<R: tauri::Runtime>(
    window: &tauri::Window<R>,
) -> Result<PersistedWindowState, String> {
    let position = window
        .outer_position()
        .map_err(|_| "window position could not be captured".to_owned())?;
    let size = window
        .outer_size()
        .map_err(|_| "window size could not be captured".to_owned())?;
    let maximized = window
        .is_maximized()
        .map_err(|_| "window maximized state could not be captured".to_owned())?;
    Ok(persisted_window_state_from_geometry(
        position, size, maximized,
    ))
}

fn persist_current_window_state<R: tauri::Runtime>(
    window: &tauri::Window<R>,
) -> Result<(), String> {
    let state = persisted_window_state_from_window(window)?;
    persist_window_state(&state)
}

fn load_or_create_local_unlock_secret(
    store: &DesktopCredentialStore,
    key_id: &SessionKeyId,
    store_path: &Path,
) -> Result<LocalUnlockSecret, String> {
    match store.load(key_id) {
        Ok(secret) => Ok(secret),
        Err(DesktopCredentialError::Missing) => {
            if directory_has_entries(store_path)? {
                return Err("local store credential is missing".to_owned());
            }
            let secret = LocalUnlockSecret::generate();
            store
                .save(key_id, &secret)
                .map_err(|_| "local store credential could not be saved".to_owned())?;
            Ok(secret)
        }
        Err(_) => Err("local store credential could not be loaded".to_owned()),
    }
}

fn directory_has_entries(path: &Path) -> Result<bool, String> {
    match std::fs::read_dir(path) {
        Ok(mut entries) => Ok(entries.next().is_some()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err("local store directory could not be inspected".to_owned()),
    }
}

fn search_index_metadata_path(search_index_path: &Path) -> PathBuf {
    search_index_path.join(SEARCH_INDEX_METADATA_FILE)
}

fn current_search_index_metadata() -> SearchIndexMetadata {
    SearchIndexMetadata {
        schema_version: SEARCH_INDEX_SCHEMA_VERSION,
        tokenizer: SEARCH_INDEX_TOKENIZER.to_owned(),
    }
}

fn prepare_search_index_path(search_index_path: &Path) -> Result<(), String> {
    prepare_search_index_path_with_rebuild_suffix(
        search_index_path,
        &recovery_suffix("search-index-rebuild"),
    )
}

fn prepare_search_index_path_with_rebuild_suffix(
    search_index_path: &Path,
    rebuild_suffix: &str,
) -> Result<(), String> {
    let metadata_path = search_index_metadata_path(search_index_path);
    let current = current_search_index_metadata();
    let needs_rebuild = match std::fs::read(&metadata_path) {
        Ok(bytes) => serde_json::from_slice::<SearchIndexMetadata>(&bytes)
            .map(|metadata| metadata != current)
            .unwrap_or(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            directory_has_entries(search_index_path)?
        }
        Err(_) => true,
    };

    if needs_rebuild {
        let _ = quarantine_path_with_suffix(search_index_path, rebuild_suffix)?;
    }

    std::fs::create_dir_all(search_index_path)
        .map_err(|_| "encrypted search index directory could not be created".to_owned())?;
    write_search_index_metadata(search_index_path, &current)
}

fn write_search_index_metadata(
    search_index_path: &Path,
    metadata: &SearchIndexMetadata,
) -> Result<(), String> {
    let metadata_path = search_index_metadata_path(search_index_path);
    let tmp_path = metadata_path.with_extension("json.tmp");
    let json = serde_json::to_vec(metadata)
        .map_err(|_| "search index metadata could not be serialized".to_owned())?;
    std::fs::write(&tmp_path, json)
        .map_err(|_| "search index metadata could not be written".to_owned())?;
    std::fs::rename(&tmp_path, &metadata_path)
        .map_err(|_| "search index metadata could not be committed".to_owned())
}

fn quarantine_path_with_suffix(path: &Path, suffix: &str) -> Result<Option<PathBuf>, String> {
    match std::fs::metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("local store directory could not be inspected".to_owned()),
    }

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "local store path is invalid".to_owned())?;
    let quarantine_path = path.with_file_name(format!("{file_name}.{suffix}"));
    std::fs::rename(path, &quarantine_path)
        .map_err(|_| "local store directory could not be quarantined".to_owned())?;
    Ok(Some(quarantine_path))
}

fn quarantine_local_store_paths(
    base_dir: &Path,
    key_id: &SessionKeyId,
) -> Result<Vec<PathBuf>, String> {
    quarantine_local_store_paths_with_suffix(base_dir, key_id, &recovery_suffix("restore-retry"))
}

fn quarantine_local_store_paths_with_suffix(
    base_dir: &Path,
    key_id: &SessionKeyId,
    suffix: &str,
) -> Result<Vec<PathBuf>, String> {
    let paths = local_store_paths(base_dir, key_id);
    let mut quarantined = Vec::new();
    for path in [paths.store_path, paths.cache_path, paths.search_index_path] {
        if let Some(quarantine_path) = quarantine_path_with_suffix(&path, suffix)? {
            quarantined.push(quarantine_path);
        }
    }
    Ok(quarantined)
}

fn recovery_suffix(reason: &str) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!("{reason}-{millis}")
}

fn remove_local_store_paths(base_dir: &Path, key_id: &SessionKeyId) -> Result<(), String> {
    let paths = local_store_paths(base_dir, key_id);
    for path in [paths.store_path, paths.cache_path, paths.search_index_path] {
        match std::fs::remove_dir_all(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("local store directory could not be deleted".to_owned()),
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LocalStorePaths {
    store_path: PathBuf,
    cache_path: PathBuf,
    search_index_path: PathBuf,
}

fn local_store_paths(base_dir: &Path, key_id: &SessionKeyId) -> LocalStorePaths {
    let namespace = path_namespace_for_key_id(key_id);
    LocalStorePaths {
        store_path: base_dir.join("sdk-store").join(&namespace),
        cache_path: base_dir.join("sdk-cache").join(&namespace),
        search_index_path: base_dir.join("search-index").join(namespace),
    }
}

fn path_namespace_for_key_id(key_id: &SessionKeyId) -> String {
    key_id
        .local_unlock_account_name()
        .chars()
        .map(|character| match character {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' => character,
            _ => '_',
        })
        .collect()
}

fn session_key_id_from_info(info: &SessionInfo) -> SessionKeyId {
    SessionKeyId {
        homeserver: info.homeserver.clone(),
        user_id: info.user_id.clone(),
        device_id: info.device_id.clone(),
    }
}

fn session_info_from_key_id(key_id: &SessionKeyId) -> SessionInfo {
    SessionInfo {
        homeserver: key_id.homeserver.clone(),
        user_id: key_id.user_id.clone(),
        device_id: key_id.device_id.clone(),
    }
}

fn saved_session_infos_from_index(index: &SavedSessionIndex) -> Vec<SessionInfo> {
    index
        .sessions()
        .iter()
        .map(session_info_from_key_id)
        .collect()
}

fn build_desktop_menu<R: tauri::Runtime, M: Manager<R>>(
    manager: &M,
) -> tauri::Result<tauri::menu::Menu<R>> {
    let open_user_settings = menu_item(manager, MENU_ID_OPEN_USER_SETTINGS)?;
    let toggle_right_panel = menu_item(manager, MENU_ID_TOGGLE_RIGHT_PANEL)?;
    let show_keyboard_settings = menu_item(manager, MENU_ID_SHOW_KEYBOARD_SETTINGS)?;

    let app_menu = SubmenuBuilder::new(manager, "matrix-desktop")
        .item(&open_user_settings)
        .separator()
        .quit()
        .build()?;
    let file_menu = SubmenuBuilder::new(manager, "File")
        .close_window()
        .build()?;
    let edit_menu = SubmenuBuilder::new(manager, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;
    let view_menu = SubmenuBuilder::new(manager, "View")
        .item(&toggle_right_panel)
        .build()?;
    let help_menu = SubmenuBuilder::new(manager, "Help")
        .item(&show_keyboard_settings)
        .build()?;

    MenuBuilder::new(manager)
        .item(&app_menu)
        .item(&file_menu)
        .item(&edit_menu)
        .item(&view_menu)
        .item(&help_menu)
        .build()
}

fn menu_item<R: tauri::Runtime, M: Manager<R>>(
    manager: &M,
    id: &str,
) -> tauri::Result<tauri::menu::MenuItem<R>> {
    let item = desktop_menu_items()
        .into_iter()
        .find(|item| item.id == id)
        .expect("desktop menu item id should be registered");
    MenuItemBuilder::with_id(item.id, item.label)
        .accelerator(item.accelerator)
        .build(manager)
}

/// Spawn the CoreEvent forwarding task. This task owns a dedicated connection
/// (second `attach()`) so it can loop on `recv_event` without blocking command
/// dispatch.
///
/// On `CoreEvent::StateChanged`: emit `matrix-desktop://state` with the
/// serialized snapshot + update QA window title.
/// On any `CoreEvent`: emit `matrix-desktop://event` with a serialized DTO.
/// On `EventStreamLag`: emit the latest snapshot (resync) + a
/// `ResyncMarker` event so the frontend resets its timeline stores.
fn spawn_core_event_forwarder(
    app: tauri::AppHandle,
    mut event_conn: CoreConnection,
    timeline_items_count: &'static AtomicUsize,
) {
    tauri::async_runtime::spawn(async move {
        loop {
            match event_conn.recv_event().await {
                Ok(event) => {
                    forward_core_event(&app, &event, timeline_items_count);
                }
                Err(_lag) => {
                    // Consumer fell behind. Emit the latest snapshot so the
                    // frontend can resync, then a ResyncMarker so it resets
                    // its timeline stores.
                    let snapshot = event_conn.snapshot();
                    emit_state_snapshot(&app, &snapshot, timeline_items_count);
                    let _ = app.emit(CORE_EVENT_NAME, serde_json::json!({ "kind": "ResyncMarker" }));
                }
            }
        }
    });
}

fn forward_core_event(
    app: &tauri::AppHandle,
    event: &CoreEvent,
    timeline_items_count: &'static AtomicUsize,
) {
    // Track timeline item count for QA window title.
    match event {
        CoreEvent::Timeline(TimelineEvent::InitialItems { items, .. }) => {
            timeline_items_count.store(items.len(), Ordering::Relaxed);
        }
        CoreEvent::Timeline(TimelineEvent::ItemsUpdated { diffs, .. }) => {
            // Apply diff count delta (approximate; exact count tracked by React store)
            let current = timeline_items_count.load(Ordering::Relaxed);
            let delta = diffs_net_count_change(diffs);
            let new_count = (current as i64 + delta).max(0) as usize;
            timeline_items_count.store(new_count, Ordering::Relaxed);
        }
        _ => {}
    }

    // Serialize and forward as `matrix-desktop://event`.
    if let Some(payload) = serialize_core_event(event) {
        let _ = app.emit(CORE_EVENT_NAME, payload);
    }

    // On StateChanged: also emit `matrix-desktop://state` for backward compat
    // (the React app currently listens on this event to trigger a `get_snapshot`
    // poll; future work is to eliminate the poll entirely).
    if let CoreEvent::StateChanged(snapshot) = event {
        emit_state_snapshot(app, snapshot, timeline_items_count);
    }
}

fn diffs_net_count_change(diffs: &[matrix_desktop_core::TimelineDiff]) -> i64 {
    diffs.iter().map(|diff| match diff {
        matrix_desktop_core::TimelineDiff::PushFront { .. }
        | matrix_desktop_core::TimelineDiff::PushBack { .. }
        | matrix_desktop_core::TimelineDiff::Insert { .. } => 1_i64,
        matrix_desktop_core::TimelineDiff::Remove { .. } => -1_i64,
        matrix_desktop_core::TimelineDiff::Truncate { .. }
        | matrix_desktop_core::TimelineDiff::Clear
        | matrix_desktop_core::TimelineDiff::Reset { .. }
        | matrix_desktop_core::TimelineDiff::Set { .. } => 0_i64,
    }).sum()
}

fn emit_state_snapshot(
    app: &tauri::AppHandle,
    _snapshot: &AppStateSnapshot,
    _timeline_items_count: &AtomicUsize,
) {
    // Emit the snapshot event (triggers React to call get_snapshot).
    let _ = app.emit(STATE_EVENT_NAME, "stateChanged");
}

/// Serialize a `CoreEvent` to a JSON value for IPC.
///
/// Security: message bodies flow in `Timeline` events. These are visible
/// content (not secret), but we never trace IPC payloads in release.
/// The serialization produces structured JSON only — no raw SDK errors.
fn serialize_core_event(event: &CoreEvent) -> Option<serde_json::Value> {
    Some(match event {
        CoreEvent::StateChanged(_) => {
            // StateChanged snapshots are sent via `matrix-desktop://state`;
            // don't duplicate as a generic event.
            return None;
        }
        CoreEvent::Account(e) => serde_json::json!({ "kind": "Account", "event": e }),
        CoreEvent::Sync(e) => serde_json::json!({ "kind": "Sync", "event": e }),
        CoreEvent::Room(e) => serde_json::json!({ "kind": "Room", "event": e }),
        CoreEvent::Timeline(e) => serde_json::json!({ "kind": "Timeline", "event": e }),
        CoreEvent::Search(e) => serde_json::json!({ "kind": "Search", "event": e }),
        CoreEvent::OperationFailed { request_id, failure } => {
            serde_json::json!({
                "kind": "OperationFailed",
                "request_id": request_id,
                "failure": failure,
            })
        }
    })
}

pub fn run() {
    let restore_session = restore_session_enabled_from_env_value(
        std::env::var("MATRIX_DESKTOP_RESTORE_SESSION")
            .ok()
            .as_deref(),
    );

    tauri::Builder::default()
        .setup(move |app| {
            // Build the CoreRuntime inside setup() so Tauri's async runtime is
            // already active. `CoreRuntime::start_with_data_dir` calls
            // `executor::spawn` which requires a Tokio runtime context. Tauri
            // starts its tokio runtime before invoking setup; we enter the
            // handle so `tokio::task::spawn` can find it from the main thread.
            let data_dir = matrix_desktop_data_dir()
                .unwrap_or_else(|_| PathBuf::from("matrix-desktop-data"));
            // Enter Tauri's tokio runtime so `executor::spawn` (tokio::task::spawn)
            // can find a runtime handle from this non-tokio-worker thread.
            let async_handle = tauri::async_runtime::handle();
            let _guard = async_handle.inner().enter();
            let runtime = CoreRuntime::start_with_data_dir(data_dir);

            // command-dispatch connection (held in state)
            let command_conn = runtime.attach();
            // event-forwarding connection (owned by the spawned task below)
            let event_conn = runtime.attach();

            // Static storage for timeline_items_count so the forwarder task
            // can hold a 'static reference. We use Box::leak because the
            // runtime lives for the entire process lifetime.
            let timeline_items_count: &'static AtomicUsize =
                Box::leak(Box::new(AtomicUsize::new(0)));

            let core_state = CoreRuntimeState {
                runtime,
                connection: TokioMutex::new(command_conn),
                timeline_items_count: AtomicUsize::new(0),
            };
            app.manage(core_state);

            let menu = build_desktop_menu(app)?;
            app.set_menu(menu)?;
            let _ = restore_main_window_state(app);
            app.on_menu_event(|app, event| {
                if let Some(action_id) = desktop_menu_action_id(event.id().as_ref()) {
                    let _ = app.emit(MENU_EVENT_NAME, action_id);
                }
            });

            // Start the CoreEvent forwarding task.
            spawn_core_event_forwarder(app.handle().clone(), event_conn, timeline_items_count);

            #[cfg(any(debug_assertions, test))]
            if let Some(pipe_path) = qa_login_pipe_path_from_env() {
                commands::spawn_qa_login_pipe_reader(app.handle().clone(), pipe_path);
            }

            if restore_session {
                // STARTUP_RESTORE_DESIGN_GAP: read credential store to find
                // the last session AccountKey, then send RestoreSession.
                // After AccountCommand::RestoreLastSession lands in the canon,
                // this credential-store read moves into StoreActor.
                if let Ok(Some(account_key)) = last_session_account_key_from_store() {
                    // setup() is not async; spawn the restore command.
                    let app_handle = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        let core_state = app_handle.state::<CoreRuntimeState>();
                        let request_id =
                            core_state.connection.lock().await.next_request_id();
                        let _ = commands::submit_core_command(
                            &core_state,
                            CoreCommand::Account(AccountCommand::RestoreSession {
                                request_id,
                                account_key,
                            }),
                        )
                        .await;
                    });
                }
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if window_event_should_persist(event) {
                    let _ = persist_current_window_state(window);
                }
                if window_event_should_stop_background_tasks(event) {
                    // Core runtime cleanup: send Shutdown command.
                    // (The runtime actor will stop when command_tx is dropped
                    // at process exit; explicit Shutdown is belt-and-suspenders.)
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::discover_login_methods,
            commands::submit_login,
            commands::list_saved_sessions,
            commands::switch_account,
            commands::submit_recovery,
            commands::logout,
            commands::select_space,
            commands::select_room,
            commands::paginate_timeline_backwards,
            commands::send_text,
            commands::edit_message,
            commands::redact_message,
            commands::open_thread,
            commands::close_thread,
            commands::submit_search,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run matrix desktop app");
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use matrix_desktop_key::{LocalUnlockSecret, StoredMatrixSession};
    use matrix_desktop_state::SessionInfo;

    use super::{
        DesktopCredentialStore, PersistedWindowState, QaFileCredentialStore, desktop_menu_items,
        desktop_standard_menu_items, hex_file_name, load_window_state_with_base, local_store_paths,
        path_namespace_for_key_id, persist_window_state_with_base,
        persisted_window_state_from_geometry, persisted_window_state_is_restorable,
        prepare_search_index_path_with_rebuild_suffix, qa_file_credential_store_dir_from_env_value,
        qa_login_pipe_path_from_env_value, qa_skips_keychain_persistence_from_env_value,
        quarantine_local_store_paths_with_suffix, remove_local_store_paths,
        restore_session_enabled_from_env_value, saved_session_infos_from_index,
        saved_sessions_disabled_from_env_value, search_index_metadata_path,
        session_key_id_from_info, window_event_should_persist,
        window_event_should_stop_background_tasks, window_state_path,
    };
    use crate::commands::parse_qa_login_pipe_payload;

    #[test]
    fn restore_session_env_value_can_start_tauri_signed_out() {
        assert!(!restore_session_enabled_from_env_value(Some("0")));
        assert!(!restore_session_enabled_from_env_value(Some("false")));
        assert!(!restore_session_enabled_from_env_value(Some("signed-out")));
        assert!(restore_session_enabled_from_env_value(None));
        assert!(restore_session_enabled_from_env_value(Some("1")));
    }

    #[test]
    fn saved_sessions_env_value_can_disable_keychain_reads_for_gui_smoke() {
        assert!(saved_sessions_disabled_from_env_value(Some("1")));
        assert!(saved_sessions_disabled_from_env_value(Some("true")));
        assert!(saved_sessions_disabled_from_env_value(Some("yes")));
        assert!(!saved_sessions_disabled_from_env_value(None));
        assert!(!saved_sessions_disabled_from_env_value(Some("0")));
    }

    #[test]
    fn qa_login_pipe_env_uses_path_only() {
        assert_eq!(
            qa_login_pipe_path_from_env_value(Some(" /tmp/matrix-desktop-login.pipe ")),
            Some(Path::new("/tmp/matrix-desktop-login.pipe").to_path_buf())
        );
        assert_eq!(qa_login_pipe_path_from_env_value(Some("   ")), None);
        assert_eq!(qa_login_pipe_path_from_env_value(None), None);
    }

    #[test]
    fn qa_file_credential_store_env_uses_path_only() {
        assert_eq!(
            qa_file_credential_store_dir_from_env_value(Some(" /tmp/matrix-desktop-qa-creds ")),
            Some(Path::new("/tmp/matrix-desktop-qa-creds").to_path_buf())
        );
        assert_eq!(
            qa_file_credential_store_dir_from_env_value(Some("   ")),
            None
        );
        assert_eq!(qa_file_credential_store_dir_from_env_value(None), None);
    }

    #[test]
    fn qa_keychain_skip_env_is_explicitly_opted_in() {
        assert!(qa_skips_keychain_persistence_from_env_value(Some("1")));
        assert!(qa_skips_keychain_persistence_from_env_value(Some("true")));
        assert!(qa_skips_keychain_persistence_from_env_value(Some("yes")));
        assert!(!qa_skips_keychain_persistence_from_env_value(None));
        assert!(!qa_skips_keychain_persistence_from_env_value(Some("0")));
        assert!(!qa_skips_keychain_persistence_from_env_value(Some("false")));
    }

    #[test]
    fn qa_login_pipe_payload_maps_to_login_request_without_debugging_secret() {
        let request = parse_qa_login_pipe_payload(
            r#"{"homeserver":"https://matrix.example.org","username":"fixture-user","password":"synthetic-password","device_display_name":"Matrix Desktop GUI Smoke","recovery_secret":"synthetic-recovery-secret"}"#,
        )
        .expect("payload should parse");

        assert_eq!(request.login.homeserver, "https://matrix.example.org");
        assert_eq!(request.login.username, "fixture-user");
        assert_eq!(request.login.password.expose_secret(), "synthetic-password");
        assert_eq!(
            request.login.device_display_name.as_deref(),
            Some("Matrix Desktop GUI Smoke")
        );
        assert_eq!(
            request
                .recovery_secret
                .as_ref()
                .map(|secret| secret.expose_secret()),
            Some("synthetic-recovery-secret")
        );
        assert!(!format!("{request:?}").contains("synthetic-password"));
        assert!(!format!("{request:?}").contains("synthetic-recovery-secret"));
    }

    #[test]
    fn qa_file_credential_store_round_trips_session_state_without_keychain() {
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        let store = DesktopCredentialStore::QaFile(QaFileCredentialStore::new(
            tempdir.path().join("qa-credential-store"),
        ));
        let info = SessionInfo {
            homeserver: "https://matrix.example.org".to_owned(),
            user_id: "@user-a:example.invalid".to_owned(),
            device_id: "DEVICE123".to_owned(),
        };
        let key_id = session_key_id_from_info(&info);
        let secret = LocalUnlockSecret::generate();

        store
            .save(&key_id, &secret)
            .expect("local unlock secret should be saved");
        let loaded_secret = store
            .load(&key_id)
            .expect("local unlock secret should be loaded");
        assert_eq!(
            loaded_secret.to_storage_string().as_str(),
            secret.to_storage_string().as_str()
        );

        store
            .save_matrix_session(
                &key_id,
                &StoredMatrixSession::new(r#"{"access_token":"synthetic-token"}"#),
            )
            .expect("session should be saved");
        assert_eq!(
            store
                .load_matrix_session(&key_id)
                .expect("session should be loaded")
                .as_str(),
            r#"{"access_token":"synthetic-token"}"#
        );

        store
            .remember_saved_session(&key_id)
            .expect("saved session index should be updated");
        assert_eq!(
            store.load_saved_sessions().unwrap().sessions(),
            &[key_id.clone()]
        );

        store
            .save_last_session(&key_id)
            .expect("last session should be saved");
        assert_eq!(store.load_last_session().unwrap(), Some(key_id.clone()));

        let secret_path = tempdir
            .path()
            .join("qa-credential-store")
            .join("v1")
            .join(format!(
                "{}.secret",
                hex_file_name(key_id.local_unlock_account_name().as_str())
            ));
        assert!(secret_path.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mode = std::fs::metadata(secret_path)
                .expect("secret metadata should be readable")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn session_key_id_preserves_session_info_for_secret_store_lookup() {
        let info = SessionInfo {
            homeserver: "https://matrix.example.org".to_owned(),
            user_id: "@user-a:example.invalid".to_owned(),
            device_id: "DEVICE123".to_owned(),
        };

        let key_id = session_key_id_from_info(&info);

        assert_eq!(key_id.homeserver, info.homeserver);
        assert_eq!(key_id.user_id, info.user_id);
        assert_eq!(key_id.device_id, info.device_id);
    }

    #[test]
    fn local_store_paths_are_namespaced_without_windows_invalid_separators() {
        let info = SessionInfo {
            homeserver: "https://matrix.example.org:8448".to_owned(),
            user_id: "@user-a:example.invalid".to_owned(),
            device_id: "DEVICE123".to_owned(),
        };
        let key_id = session_key_id_from_info(&info);

        let namespace = path_namespace_for_key_id(&key_id);
        let paths = local_store_paths(Path::new("/tmp/matrix-desktop"), &key_id);

        assert!(!namespace.contains('|'));
        assert!(!namespace.contains(':'));
        assert!(paths.store_path.ends_with(&namespace));
        assert!(paths.cache_path.ends_with(&namespace));
        assert!(paths.search_index_path.ends_with(&namespace));
        assert_ne!(paths.store_path, paths.cache_path);
        assert_ne!(paths.store_path, paths.search_index_path);
        assert_ne!(paths.cache_path, paths.search_index_path);
    }

    #[test]
    fn local_store_cleanup_removes_all_namespaced_store_directories() {
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        let info = SessionInfo {
            homeserver: "https://matrix.example.org".to_owned(),
            user_id: "@user-a:example.invalid".to_owned(),
            device_id: "DEVICE123".to_owned(),
        };
        let key_id = session_key_id_from_info(&info);
        let paths = local_store_paths(tempdir.path(), &key_id);
        for path in [
            &paths.store_path,
            &paths.cache_path,
            &paths.search_index_path,
        ] {
            std::fs::create_dir_all(path).expect("store path should be created");
            std::fs::write(path.join("sentinel"), b"synthetic")
                .expect("sentinel should be written");
        }

        remove_local_store_paths(tempdir.path(), &key_id).expect("cleanup should succeed");

        assert!(!paths.store_path.exists());
        assert!(!paths.cache_path.exists());
        assert!(!paths.search_index_path.exists());
    }

    #[test]
    fn corrupted_local_store_recovery_quarantines_store_cache_and_search_index() {
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        let info = SessionInfo {
            homeserver: "https://matrix.example.org".to_owned(),
            user_id: "@user-a:example.invalid".to_owned(),
            device_id: "DEVICE123".to_owned(),
        };
        let key_id = session_key_id_from_info(&info);
        let paths = local_store_paths(tempdir.path(), &key_id);
        for path in [
            &paths.store_path,
            &paths.cache_path,
            &paths.search_index_path,
        ] {
            std::fs::create_dir_all(path).expect("store path should be created");
            std::fs::write(path.join("sentinel"), b"synthetic")
                .expect("sentinel should be written");
        }

        let quarantine =
            quarantine_local_store_paths_with_suffix(tempdir.path(), &key_id, "restore-retry")
                .expect("local store paths should be quarantined");

        assert!(!paths.store_path.exists());
        assert!(!paths.cache_path.exists());
        assert!(!paths.search_index_path.exists());
        assert_eq!(quarantine.len(), 3);
        for quarantined in quarantine {
            let file_name = quarantined
                .file_name()
                .and_then(|name| name.to_str())
                .expect("quarantine path should have a file name");
            assert!(file_name.ends_with(".restore-retry"));
            assert!(quarantined.join("sentinel").exists());
        }
    }

    #[test]
    fn search_index_metadata_mismatch_rebuilds_index_directory() {
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        let index_path = tempdir.path().join("search-index");
        std::fs::create_dir_all(&index_path).expect("index path should be created");
        std::fs::write(
            search_index_metadata_path(&index_path),
            r#"{"schema_version":0,"tokenizer":"word"}"#,
        )
        .expect("old metadata should be written");
        std::fs::write(index_path.join("old-index-segment"), b"synthetic")
            .expect("old segment should be written");

        prepare_search_index_path_with_rebuild_suffix(&index_path, "schema-v1")
            .expect("index should be rebuilt");

        assert!(index_path.exists());
        assert!(search_index_metadata_path(&index_path).exists());
        assert!(!index_path.join("old-index-segment").exists());
        assert!(
            tempdir
                .path()
                .join("search-index.schema-v1")
                .join("old-index-segment")
                .exists()
        );
    }

    #[test]
    fn search_index_file_at_directory_path_is_quarantined_and_recreated() {
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        let index_path = tempdir.path().join("search-index");
        std::fs::write(&index_path, b"not-a-directory").expect("blocking file should be written");

        prepare_search_index_path_with_rebuild_suffix(&index_path, "path-rebuild")
            .expect("blocking file should be quarantined");

        assert!(index_path.is_dir());
        assert!(search_index_metadata_path(&index_path).exists());
        assert_eq!(
            std::fs::read(tempdir.path().join("search-index.path-rebuild"))
                .expect("quarantined file should be readable"),
            b"not-a-directory"
        );
    }

    #[test]
    fn saved_session_infos_from_index_preserves_account_device_identity() {
        let mut index = matrix_desktop_key::SavedSessionIndex::new();
        let alpha = matrix_desktop_key::SessionKeyId {
            homeserver: "https://matrix.example.org".to_owned(),
            user_id: "@user-a:example.invalid".to_owned(),
            device_id: "DEVICE-A".to_owned(),
        };
        let beta = matrix_desktop_key::SessionKeyId {
            homeserver: "https://matrix.example.org".to_owned(),
            user_id: "@user-b:example.invalid".to_owned(),
            device_id: "DEVICE-B".to_owned(),
        };
        index.upsert(alpha.clone());
        index.upsert(beta.clone());

        let infos = saved_session_infos_from_index(&index);

        assert_eq!(infos.len(), 2);
        assert_eq!(infos[0].homeserver, alpha.homeserver);
        assert_eq!(infos[0].user_id, alpha.user_id);
        assert_eq!(infos[0].device_id, alpha.device_id);
        assert_eq!(infos[1].homeserver, beta.homeserver);
        assert_eq!(infos[1].user_id, beta.user_id);
        assert_eq!(infos[1].device_id, beta.device_id);
    }

    #[test]
    fn window_state_path_is_separate_from_encrypted_session_stores() {
        let path = window_state_path(Path::new("/tmp/matrix-desktop"));

        assert_eq!(
            path,
            Path::new("/tmp/matrix-desktop")
                .join("app-shell")
                .join("window-state.json")
        );
    }

    #[test]
    fn persisted_window_state_rejects_tiny_or_empty_geometry() {
        assert!(persisted_window_state_is_restorable(
            &PersistedWindowState {
                x: 20,
                y: 40,
                width: 1280,
                height: 820,
                maximized: false,
            }
        ));
        assert!(!persisted_window_state_is_restorable(
            &PersistedWindowState {
                x: 20,
                y: 40,
                width: 120,
                height: 80,
                maximized: false,
            }
        ));
        assert!(!persisted_window_state_is_restorable(
            &PersistedWindowState {
                x: 20,
                y: 40,
                width: 0,
                height: 820,
                maximized: false,
            }
        ));
    }

    #[test]
    fn window_state_persistence_writes_json_atomically_to_app_shell_path() {
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        let state = PersistedWindowState {
            x: 24,
            y: 48,
            width: 1440,
            height: 900,
            maximized: true,
        };

        persist_window_state_with_base(tempdir.path(), &state)
            .expect("window state should be written");

        let saved = std::fs::read_to_string(window_state_path(tempdir.path()))
            .expect("window state json should be readable");
        assert!(saved.contains("\"width\":1440"));
        assert!(saved.contains("\"maximized\":true"));
        assert!(!saved.contains("access_token"));
    }

    #[test]
    fn window_state_load_ignores_corrupted_or_unrestorable_json() {
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        let path = window_state_path(tempdir.path());
        std::fs::create_dir_all(path.parent().expect("state path should have parent"))
            .expect("state dir should be created");

        std::fs::write(&path, b"{not-json").expect("corrupted state should be written");
        assert_eq!(
            load_window_state_with_base(tempdir.path()).expect("corruption should be ignored"),
            None
        );

        std::fs::write(
            &path,
            r#"{"x":1,"y":2,"width":300,"height":200,"maximized":false}"#,
        )
        .expect("tiny state should be written");
        assert_eq!(
            load_window_state_with_base(tempdir.path()).expect("tiny state should be ignored"),
            None
        );
    }

    #[test]
    fn persisted_window_state_from_geometry_preserves_position_size_and_maximized_flag() {
        let state = persisted_window_state_from_geometry(
            tauri::PhysicalPosition::new(50, 70),
            tauri::PhysicalSize::new(1366, 768),
            true,
        );

        assert_eq!(
            state,
            PersistedWindowState {
                x: 50,
                y: 70,
                width: 1366,
                height: 768,
                maximized: true,
            }
        );
    }

    #[test]
    fn window_event_should_persist_for_geometry_changes_but_not_focus() {
        assert!(window_event_should_persist(&tauri::WindowEvent::Resized(
            tauri::PhysicalSize::new(1280, 820)
        )));
        assert!(window_event_should_persist(&tauri::WindowEvent::Moved(
            tauri::PhysicalPosition::new(30, 50)
        )));
        assert!(!window_event_should_persist(&tauri::WindowEvent::Focused(
            true
        )));
    }

    #[test]
    fn window_event_should_stop_background_tasks_on_shutdown() {
        assert!(window_event_should_stop_background_tasks(
            &tauri::WindowEvent::Destroyed
        ));
        assert!(!window_event_should_stop_background_tasks(
            &tauri::WindowEvent::Focused(false)
        ));
        assert!(!window_event_should_stop_background_tasks(
            &tauri::WindowEvent::Resized(tauri::PhysicalSize::new(1280, 820))
        ));
    }

    #[test]
    fn desktop_menu_items_include_element_compatible_shortcuts() {
        let items = desktop_menu_items();

        assert!(items.iter().any(|item| {
            item.id == "open_user_settings"
                && item.accelerator == "CmdOrCtrl+,"
                && item.menu == "app"
        }));
        assert!(items.iter().any(|item| {
            item.id == "show_keyboard_settings"
                && item.accelerator == "CmdOrCtrl+/"
                && item.menu == "help"
        }));
        assert!(items.iter().any(|item| {
            item.id == "toggle_right_panel"
                && item.accelerator == "CmdOrCtrl+."
                && item.menu == "view"
        }));
    }

    #[test]
    fn desktop_menu_items_include_platform_standard_close_and_quit() {
        let items = desktop_standard_menu_items();

        assert!(items.iter().any(|item| {
            item.id == "close_window" && item.accelerator == "CmdOrCtrl+W" && item.menu == "file"
        }));
        assert!(items.iter().any(|item| {
            item.id == "quit" && item.accelerator == "CmdOrCtrl+Q" && item.menu == "app"
        }));
    }
}
