#![recursion_limit = "256"]

mod commands;
mod dto;

use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use tauri::{
    Emitter, Manager,
    menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder},
};

use serde::{Deserialize, Serialize};

use matrix_desktop_auth::{
    MatrixClientSession, MatrixClientStoreConfig, MatrixClientStoreKey, MatrixSearchIndexKey,
    MatrixSearchIndexStoreConfig, PersistableMatrixSession,
};
use matrix_desktop_backend::{
    E2eeRecoveryMode, FakeDesktopBackend, FakeDesktopBackendConfig, LoginDiscoveryMode, LoginMode,
    SyncMode,
};
use matrix_desktop_key::{
    CredentialStore, LocalUnlockSecret, SavedSessionIndex, SessionKeyId, StoredMatrixSession,
    is_missing_credential_error,
};
use matrix_desktop_state::SessionInfo;

const CREDENTIAL_SERVICE_NAME: &str = "matrix-desktop";
const MENU_EVENT_NAME: &str = "matrix-desktop://menu";
const MENU_ID_OPEN_USER_SETTINGS: &str = "open_user_settings";
const MENU_ID_SHOW_KEYBOARD_SETTINGS: &str = "show_keyboard_settings";
const MENU_ID_TOGGLE_RIGHT_PANEL: &str = "toggle_right_panel";
const MIN_RESTORABLE_WINDOW_WIDTH: u32 = 760;
const MIN_RESTORABLE_WINDOW_HEIGHT: u32 = 620;
const QA_LOGIN_PIPE_ENV: &str = "MATRIX_DESKTOP_QA_LOGIN_PIPE";
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
pub struct BackendState {
    backend: Mutex<FakeDesktopBackend>,
    sync_task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    timeline_task: Mutex<Option<TimelineTaskHandle>>,
}

pub(crate) struct TimelineTaskHandle {
    room_id: String,
    task: tauri::async_runtime::JoinHandle<()>,
    pagination_sender: tokio::sync::mpsc::Sender<TimelinePaginationRequest>,
}

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

fn qa_login_pipe_path_from_env_value(value: Option<&str>) -> Option<PathBuf> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn qa_login_pipe_path_from_env() -> Option<PathBuf> {
    qa_login_pipe_path_from_env_value(std::env::var(QA_LOGIN_PIPE_ENV).ok().as_deref())
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

pub(crate) fn persist_matrix_session(session: &MatrixClientSession) -> Result<(), String> {
    let persistable = session
        .persistable_session()
        .map_err(|_| "session persistence failed".to_owned())?;
    let key_id = session_key_id_from_info(&persistable.info);
    let session_json = persistable
        .to_json()
        .map_err(|_| "session persistence failed".to_owned())?;
    let stored_session = StoredMatrixSession::new(session_json);
    let store = CredentialStore::new(CREDENTIAL_SERVICE_NAME);

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
    restore_persistable_matrix_session_with_store_retry(
        &persistable,
        &key_id,
        &matrix_desktop_data_dir()?,
        CREDENTIAL_SERVICE_NAME,
        "encrypted session store initialization failed",
    )
    .await
}

pub(crate) fn clear_persisted_matrix_session(info: &SessionInfo) -> Result<(), String> {
    clear_persisted_matrix_session_with_base(
        info,
        &matrix_desktop_data_dir()?,
        CREDENTIAL_SERVICE_NAME,
    )
}

fn clear_persisted_matrix_session_with_base(
    info: &SessionInfo,
    base_dir: &Path,
    credential_service_name: &str,
) -> Result<(), String> {
    let key_id = session_key_id_from_info(info);
    let store = CredentialStore::new(credential_service_name);
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

async fn restore_persisted_matrix_session() -> Result<Option<MatrixClientSession>, String> {
    let store = CredentialStore::new(CREDENTIAL_SERVICE_NAME);
    let Some(key_id) = store
        .load_last_session()
        .map_err(|_| "session restore failed".to_owned())?
    else {
        return Ok(None);
    };
    let stored_session = match store.load_matrix_session(&key_id) {
        Ok(session) => session,
        Err(_) => return Ok(None),
    };
    let persistable = PersistableMatrixSession::from_json(stored_session.as_str())
        .map_err(|_| "session restore failed".to_owned())?;
    restore_persistable_matrix_session_with_store_retry(
        &persistable,
        &key_id,
        &matrix_desktop_data_dir()?,
        CREDENTIAL_SERVICE_NAME,
        "session restore failed",
    )
    .await
    .map(Some)
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
    let store = CredentialStore::new(CREDENTIAL_SERVICE_NAME);
    let stored_session = store
        .load_matrix_session(key_id)
        .map_err(|_| "session restore failed".to_owned())?;
    let persistable = PersistableMatrixSession::from_json(stored_session.as_str())
        .map_err(|_| "session restore failed".to_owned())?;
    restore_persistable_matrix_session_with_store_retry(
        &persistable,
        key_id,
        &matrix_desktop_data_dir()?,
        CREDENTIAL_SERVICE_NAME,
        "session restore failed",
    )
    .await
}

async fn restore_persistable_matrix_session_with_store_retry(
    persistable: &PersistableMatrixSession,
    key_id: &SessionKeyId,
    base_dir: &Path,
    credential_service_name: &str,
    error_message: &'static str,
) -> Result<MatrixClientSession, String> {
    let store_config = matrix_client_store_config_for_session_with_base(
        &persistable.info,
        base_dir,
        credential_service_name,
    )?;
    match matrix_desktop_auth::restore_session_with_store(persistable, Some(&store_config)).await {
        Ok(session) => Ok(session),
        Err(_) => {
            quarantine_local_store_paths(base_dir, key_id)?;
            let retry_store_config = matrix_client_store_config_for_session_with_base(
                &persistable.info,
                base_dir,
                credential_service_name,
            )?;
            matrix_desktop_auth::restore_session_with_store(persistable, Some(&retry_store_config))
                .await
                .map_err(|_| error_message.to_owned())
        }
    }
}

fn saved_matrix_session_infos() -> Result<Vec<SessionInfo>, String> {
    if saved_sessions_disabled_from_env_value(
        std::env::var("MATRIX_DESKTOP_SKIP_SAVED_SESSIONS")
            .ok()
            .as_deref(),
    ) {
        return Ok(Vec::new());
    }

    let store = CredentialStore::new(CREDENTIAL_SERVICE_NAME);
    let index = store
        .load_saved_sessions()
        .map_err(|_| "saved sessions could not be loaded".to_owned())?;
    Ok(saved_session_infos_from_index(&index))
}

fn mark_last_matrix_session(info: &SessionInfo) -> Result<(), String> {
    let store = CredentialStore::new(CREDENTIAL_SERVICE_NAME);
    store
        .save_last_session(&session_key_id_from_info(info))
        .map_err(|_| "last session pointer could not be saved".to_owned())
}

fn matrix_client_store_config_for_session_with_base(
    info: &SessionInfo,
    base_dir: &Path,
    credential_service_name: &str,
) -> Result<MatrixClientStoreConfig, String> {
    let key_id = session_key_id_from_info(info);
    let paths = local_store_paths(base_dir, &key_id);
    let store = CredentialStore::new(credential_service_name);
    let local_secret = load_or_create_local_unlock_secret(&store, &key_id, &paths.store_path)?;

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

fn matrix_desktop_data_dir() -> Result<PathBuf, String> {
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
    store: &CredentialStore,
    key_id: &SessionKeyId,
    store_path: &Path,
) -> Result<LocalUnlockSecret, String> {
    match store.load(key_id) {
        Ok(secret) => Ok(secret),
        Err(error) if is_missing_credential_error(&error) => {
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

pub fn run() {
    let restore_session = restore_session_enabled_from_env_value(
        std::env::var("MATRIX_DESKTOP_RESTORE_SESSION")
            .ok()
            .as_deref(),
    );

    tauri::Builder::default()
        .manage(BackendState::default())
        .setup(move |app| {
            let menu = build_desktop_menu(app)?;
            app.set_menu(menu)?;
            let _ = restore_main_window_state(app);
            app.on_menu_event(|app, event| {
                if let Some(action_id) = desktop_menu_action_id(event.id().as_ref()) {
                    let _ = app.emit(MENU_EVENT_NAME, action_id);
                }
            });

            if let Some(pipe_path) = qa_login_pipe_path_from_env() {
                commands::spawn_qa_login_pipe_reader(app.handle().clone(), pipe_path);
            }

            if restore_session {
                let app_handle = app.handle().clone();
                if let Ok(Some(session)) =
                    tauri::async_runtime::block_on(restore_persisted_matrix_session())
                {
                    let recovery_observer_session = session.clone();
                    let matrix_sync_session = session.clone();
                    let mut should_start_sync = false;
                    if let Ok(mut backend) = app_handle.state::<BackendState>().backend.lock() {
                        let effects = backend.complete_matrix_restore(session);
                        should_start_sync = commands::effects_include_start_sync(&effects);
                        backend.open_default_thread();
                    }
                    if should_start_sync {
                        let _ = commands::start_matrix_sync_task(
                            app_handle.clone(),
                            matrix_sync_session,
                        );
                    }
                    commands::spawn_e2ee_recovery_state_observer(
                        app_handle,
                        recovery_observer_session,
                    );
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
                    let backend_state = window.state::<BackendState>();
                    let _ = commands::abort_matrix_timeline_task(backend_state.inner());
                    let _ = commands::abort_matrix_sync_task(backend_state.inner());
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

    use matrix_desktop_state::SessionInfo;

    use super::{
        PersistedWindowState, desktop_menu_items, desktop_standard_menu_items,
        load_window_state_with_base, local_store_paths, path_namespace_for_key_id,
        persist_window_state_with_base, persisted_window_state_from_geometry,
        persisted_window_state_is_restorable, prepare_search_index_path_with_rebuild_suffix,
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
