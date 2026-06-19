#![recursion_limit = "256"]

mod commands;
mod dto;

use std::{
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::sync::Mutex as TokioMutex;

use tauri::{
    Emitter, Manager,
    menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder},
};

use serde::{Deserialize, Serialize};

// matrix-desktop-core: the production runtime host. All session, credential,
// and Matrix operations go through CoreCommand/CoreEvent — the adapter never
// touches the credential store or the SDK directly.
use matrix_desktop_core::{
    AccountCommand, CoreCommand, CoreConnection, CoreEvent, CoreRuntime, TimelineEvent,
    event::AppStateSnapshot,
};

// matrix-desktop-backend: fixture/demo preview only; never on a production
// Matrix path (overview.md: "fixture/demo data only").
use matrix_desktop_backend::{
    E2eeRecoveryMode, FakeDesktopBackend, FakeDesktopBackendConfig, LoginDiscoveryMode, LoginMode,
    SyncMode,
};

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
const QA_CONTROL_PIPE_ENV: &str = "MATRIX_DESKTOP_QA_CONTROL_PIPE";

#[derive(Clone, Debug, Eq, PartialEq)]
struct ForwardedWebviewEvent {
    event_name: &'static str,
    payload: serde_json::Value,
}

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
/// Startup restore and saved-session listing go through the canon command
/// boundary (`AccountCommand::RestoreLastSession` /
/// `AccountCommand::QuerySavedSessions`, resolved 2026-06-13); the adapter
/// never reads the credential store.
///
/// Remaining design note:
/// `timeline_items_count`: `AppState` snapshots never embed timeline lists
/// (Async rule 4). The count needed for `qa_window_title` is tracked here
/// via a Tauri-side counter updated by the event forwarding loop.
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
fn qa_control_pipe_path_from_env_value(value: Option<&str>) -> Option<PathBuf> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

// The QA control pipe lets unattended GUI smoke drive a clean logout after a
// real login. Release builds must NOT honor it — the compile-time gate keeps a
// release binary from ever reading this env var (engineering-rules: Secrets
// rule 2; debug/test-only QA control surface).
#[cfg(any(debug_assertions, test))]
fn qa_control_pipe_path_from_env() -> Option<PathBuf> {
    qa_control_pipe_path_from_env_value(std::env::var(QA_CONTROL_PIPE_ENV).ok().as_deref())
}

/// GUI-smoke toggle: when `MATRIX_DESKTOP_SKIP_SAVED_SESSIONS` is set, the
/// adapter answers `list_saved_sessions` with an empty list WITHOUT routing
/// the command to core. This prevents the OS keychain read that would
/// otherwise prompt during unattended automation. Adapter-level concern: the
/// command boundary stays untouched.
pub(crate) fn saved_sessions_disabled_from_env() -> bool {
    saved_sessions_disabled_from_env_value(
        std::env::var("MATRIX_DESKTOP_SKIP_SAVED_SESSIONS")
            .ok()
            .as_deref(),
    )
}

const DATA_DIR_NAME: &str = "koushi-desktop";
const LEGACY_DATA_DIR_NAME: &str = "matrix-desktop";

pub(crate) fn app_data_dir() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("KOUSHI_DATA_DIR") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    // Retain the legacy override name so existing QA lanes and portable runs
    // continue to work across the rebrand.
    if let Ok(path) = std::env::var("MATRIX_DESKTOP_DATA_DIR") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    dirs::data_local_dir()
        .map(|path| path.join(DATA_DIR_NAME))
        .ok_or_else(|| "local application data directory is unavailable".to_owned())
}

/// Migrate on-disk data from the legacy `matrix-desktop` directory to the
/// current `koushi-desktop` directory. This is a one-time copy performed before
/// the runtime starts so existing installs are not locked out after the rebrand.
///
/// The migration is atomic from the app's point of view: entries are copied
/// into a sibling temporary directory first, and only renamed to the final name
/// after every entry succeeds. A failure therefore leaves the final directory
/// absent, so the next launch will retry the full migration.
pub(crate) fn migrate_app_data_dir_if_needed() -> Result<(), String> {
    // When an explicit data-dir override is in use we cannot know where the
    // legacy default profile lives, and the caller is responsible for isolation.
    // Empty or whitespace-only values are ignored, matching `app_data_dir()`.
    let has_override = |name: &str| {
        std::env::var(name)
            .ok()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
    };
    if has_override("KOUSHI_DATA_DIR") || has_override("MATRIX_DESKTOP_DATA_DIR") {
        return Ok(());
    }

    let Some(base) = dirs::data_local_dir() else {
        // No local app-data directory is available (e.g. minimal/headless
        // environments). There is no default legacy profile to migrate, so let
        // the caller fall back to its own data-dir path.
        return Ok(());
    };
    let new_dir = base.join(DATA_DIR_NAME);
    if new_dir.exists() {
        return Ok(());
    }
    let legacy_dir = base.join(LEGACY_DATA_DIR_NAME);
    if !legacy_dir.exists() {
        return Ok(());
    }

    let temp_dir = base.join(format!("{}.{}", DATA_DIR_NAME, "migrating"));
    // Remove any stale temporary directory from a previous failed attempt.
    if temp_dir.exists() {
        std::fs::remove_dir_all(&temp_dir)
            .map_err(|e| format!("failed to remove stale migration temp dir: {e}"))?;
    }
    std::fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("failed to create migration temp dir: {e}"))?;

    for entry in std::fs::read_dir(&legacy_dir).map_err(|e| format!("failed to read legacy data dir: {e}"))? {
        let entry = entry.map_err(|e| format!("failed to read legacy data dir entry: {e}"))?;
        let src = entry.path();
        let dst = temp_dir.join(entry.file_name());
        copy_dir_or_file(&src, &dst).map_err(|e| format!("failed to migrate {}: {e}", src.display()))?;
    }

    std::fs::rename(&temp_dir, &new_dir)
        .map_err(|e| format!("failed to finalize migration: {e}"))?;
    Ok(())
}

fn copy_dir_or_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    let metadata = std::fs::metadata(src)?;
    if metadata.is_dir() {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_dir_or_file(&entry.path(), &dst.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(src, dst).map(|_| ())
    }
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
    load_window_state_with_base(&app_data_dir()?)
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
    persist_window_state_with_base(&app_data_dir()?, state)
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

fn build_desktop_menu<R: tauri::Runtime, M: Manager<R>>(
    manager: &M,
) -> tauri::Result<tauri::menu::Menu<R>> {
    let open_user_settings = menu_item(manager, MENU_ID_OPEN_USER_SETTINGS)?;
    let toggle_right_panel = menu_item(manager, MENU_ID_TOGGLE_RIGHT_PANEL)?;
    let show_keyboard_settings = menu_item(manager, MENU_ID_SHOW_KEYBOARD_SETTINGS)?;

    let app_menu = SubmenuBuilder::new(manager, "Koushi")
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
                    emit_forwarded_webview_events(
                        &app,
                        forwarded_webview_events_for_core_event(&event, timeline_items_count),
                    );
                }
                Err(_lag) => {
                    // Consumer fell behind. Emit the latest snapshot so the
                    // frontend can resync, then a ResyncMarker so it resets
                    // its timeline stores.
                    let snapshot = event_conn.snapshot();
                    emit_forwarded_webview_events(
                        &app,
                        forwarded_webview_events_for_lag_resync(&snapshot),
                    );
                }
            }
        }
    });
}

fn forwarded_webview_events_for_core_event(
    event: &CoreEvent,
    timeline_items_count: &AtomicUsize,
) -> Vec<ForwardedWebviewEvent> {
    let mut forwarded = Vec::new();

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

    if let Some(payload) = serialize_core_event(event) {
        forwarded.push(ForwardedWebviewEvent {
            event_name: CORE_EVENT_NAME,
            payload,
        });
    }

    if let CoreEvent::StateChanged(snapshot) = event {
        forwarded.extend(forwarded_webview_events_for_state_changed(snapshot));
    }

    forwarded
}

fn diffs_net_count_change(diffs: &[matrix_desktop_core::TimelineDiff]) -> i64 {
    diffs
        .iter()
        .map(|diff| match diff {
            matrix_desktop_core::TimelineDiff::PushFront { .. }
            | matrix_desktop_core::TimelineDiff::PushBack { .. }
            | matrix_desktop_core::TimelineDiff::Insert { .. } => 1_i64,
            matrix_desktop_core::TimelineDiff::Remove { .. } => -1_i64,
            matrix_desktop_core::TimelineDiff::Truncate { .. }
            | matrix_desktop_core::TimelineDiff::Clear
            | matrix_desktop_core::TimelineDiff::Reset { .. }
            | matrix_desktop_core::TimelineDiff::Set { .. } => 0_i64,
        })
        .sum()
}

fn forwarded_webview_events_for_state_changed(
    _snapshot: &AppStateSnapshot,
) -> Vec<ForwardedWebviewEvent> {
    vec![ForwardedWebviewEvent {
        event_name: STATE_EVENT_NAME,
        payload: serde_json::Value::String("stateChanged".to_owned()),
    }]
}

fn forwarded_webview_events_for_lag_resync(
    snapshot: &AppStateSnapshot,
) -> Vec<ForwardedWebviewEvent> {
    let mut forwarded = forwarded_webview_events_for_state_changed(snapshot);
    forwarded.push(ForwardedWebviewEvent {
        event_name: CORE_EVENT_NAME,
        payload: serde_json::json!({ "kind": "ResyncMarker" }),
    });
    forwarded
}

fn emit_forwarded_webview_events(
    app: &tauri::AppHandle,
    forwarded_events: Vec<ForwardedWebviewEvent>,
) {
    for forwarded_event in forwarded_events {
        let _ = app.emit(forwarded_event.event_name, forwarded_event.payload);
    }
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
        CoreEvent::LiveSignals(e) => serde_json::json!({ "kind": "LiveSignals", "event": e }),
        CoreEvent::Search(e) => serde_json::json!({ "kind": "Search", "event": e }),
        CoreEvent::E2eeTrust(e) => serde_json::json!({ "kind": "E2eeTrust", "event": e }),
        CoreEvent::Activity(e) => serde_json::json!({ "kind": "Activity", "event": e }),
        CoreEvent::LocalEncryption(e) => {
            serde_json::json!({ "kind": "LocalEncryption", "event": e })
        }
        CoreEvent::NativeAttention(e) => {
            serde_json::json!({ "kind": "NativeAttention", "event": e })
        }
        CoreEvent::CjkTextPolicy(e) => serde_json::json!({ "kind": "CjkTextPolicy", "event": e }),
        CoreEvent::ThreadsList(e) => serde_json::json!({ "kind": "ThreadsList", "event": e }),
        CoreEvent::OperationFailed {
            request_id,
            failure,
        } => {
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
        .plugin(tauri_plugin_notification::init())
        .setup(move |app| {
            // One-time migration of on-disk data from the legacy
            // `matrix-desktop` directory to `koushi-desktop`. A failure here
            // is blocking: continuing with an empty new directory would strand
            // existing installs, and the legacy directory is left untouched so
            // the user can retry after resolving the error.
            migrate_app_data_dir_if_needed()
                .map_err(|e| format!("data directory migration failed: {e}"))?;

            // Build the CoreRuntime inside setup() so Tauri's async runtime is
            // already active. `CoreRuntime::start_with_data_dir` calls
            // `executor::spawn` which requires a Tokio runtime context. Tauri
            // starts its tokio runtime before invoking setup; we enter the
            // handle so `tokio::task::spawn` can find it from the main thread.
            let data_dir =
                app_data_dir().unwrap_or_else(|_| PathBuf::from("koushi-desktop-data"));
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

            #[cfg(any(debug_assertions, test))]
            if let Some(pipe_path) = qa_control_pipe_path_from_env() {
                commands::spawn_qa_control_pipe_reader(app.handle().clone(), pipe_path);
            }

            if restore_session {
                // Startup restore goes through the canon command boundary:
                // `AccountCommand::RestoreLastSession` resolves the
                // last-session pointer inside StoreActor/AccountActor. A
                // missing pointer is a NORMAL outcome
                // (`CoreFailure::SessionNotFound`) — AppState stays SignedOut
                // and the login screen shows. The adapter never reads the
                // credential store.
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let core_state = app_handle.state::<CoreRuntimeState>();
                    let request_id = core_state.connection.lock().await.next_request_id();
                    let _ = commands::submit_core_command(
                        &core_state,
                        CoreCommand::Account(AccountCommand::RestoreLastSession { request_id }),
                    )
                    .await;
                });
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
            commands::restart_sync,
            commands::update_settings,
            commands::set_room_url_preview_override,
            commands::select_room_list_filter,
            commands::mark_room_as_read,
            commands::mark_room_as_unread,
            commands::set_room_notification_mode,
            commands::query_devices,
            commands::load_account_management_capabilities,
            commands::rename_device,
            commands::delete_devices,
            commands::change_password,
            commands::deactivate_account,
            commands::submit_account_management_uia,
            commands::probe_local_encryption_health,
            commands::reset_local_data,
            commands::bootstrap_cross_signing,
            commands::enable_key_backup,
            commands::bootstrap_secure_backup,
            commands::change_secure_backup_passphrase,
            commands::export_room_keys,
            commands::import_room_keys,
            commands::accept_verification,
            commands::confirm_sas_verification,
            commands::cancel_verification,
            commands::reset_identity,
            commands::submit_identity_reset_password,
            commands::submit_identity_reset_oauth,
            commands::resolve_composer_key_action,
            commands::select_space,
            commands::reorder_spaces,
            commands::select_room,
            commands::select_search_result,
            commands::close_focused_context,
            commands::open_timeline_at_timestamp,
            commands::observe_timeline_viewport,
            commands::paginate_timeline_backwards,
            commands::paginate_thread_timeline_backwards,
            commands::send_text,
            commands::schedule_send,
            commands::stage_uploads,
            commands::update_staged_upload_caption,
            commands::update_staged_upload_compression,
            commands::clear_upload_staging,
            commands::cancel_scheduled_send,
            commands::reschedule_scheduled_send,
            commands::retry_send,
            commands::cancel_send,
            commands::upload_media,
            commands::download_media,
            commands::load_message_source,
            commands::load_link_previews,
            commands::hide_link_preview,
            commands::forward_message,
            commands::edit_message,
            commands::redact_message,
            commands::send_read_receipt,
            commands::set_fully_read,
            commands::set_typing,
            commands::set_presence,
            commands::set_display_name,
            commands::set_local_user_alias,
            commands::ignore_user,
            commands::unignore_user,
            commands::report_user,
            commands::report_content,
            commands::report_room,
            commands::set_avatar,
            commands::leave_room,
            commands::forget_room,
            commands::set_room_tag,
            commands::remove_room_tag,
            commands::pin_event,
            commands::unpin_event,
            commands::load_room_settings,
            commands::update_room_setting,
            commands::moderate_room_member,
            commands::update_room_member_role,
            commands::open_activity,
            commands::close_activity,
            commands::set_activity_tab,
            commands::paginate_activity,
            commands::mark_activity_read,
            commands::open_files_view,
            commands::close_files_view,
            commands::open_threads_list,
            commands::close_threads_list,
            commands::paginate_threads_list,
            commands::open_thread,
            commands::close_thread,
            commands::submit_search,
            commands::start_room_crawl,
            commands::stop_room_crawl,
            commands::query_directory,
            commands::create_room,
            commands::create_space,
            commands::join_directory_room,
            commands::set_space_child,
            commands::accept_invite,
            commands::decline_invite,
            commands::start_direct_message,
            commands::invite_user,
            commands::set_composer_reply_target,
            commands::cancel_composer_reply,
            commands::set_composer_draft,
            commands::set_thread_composer_draft,
            commands::toggle_reaction,
            commands::send_reaction,
            commands::redact_reaction,
            commands::send_reply,
            commands::send_thread_reply,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run matrix desktop app");
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{
        CORE_EVENT_NAME, STATE_EVENT_NAME, forwarded_webview_events_for_core_event,
        forwarded_webview_events_for_lag_resync, serialize_core_event,
    };
    use super::{
        PersistedWindowState, desktop_menu_items, desktop_standard_menu_items,
        load_window_state_with_base, persist_window_state_with_base,
        persisted_window_state_from_geometry, persisted_window_state_is_restorable,
        qa_control_pipe_path_from_env_value, qa_login_pipe_path_from_env_value,
        restore_session_enabled_from_env_value, saved_sessions_disabled_from_env_value,
        window_event_should_persist, window_event_should_stop_background_tasks, window_state_path,
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
    fn qa_control_pipe_env_uses_path_only() {
        assert_eq!(
            qa_control_pipe_path_from_env_value(Some(" /tmp/matrix-desktop-control.pipe ")),
            Some(Path::new("/tmp/matrix-desktop-control.pipe").to_path_buf())
        );
        assert_eq!(qa_control_pipe_path_from_env_value(Some("   ")), None);
        assert_eq!(qa_control_pipe_path_from_env_value(None), None);
    }

    /// Release builds must NEVER read the QA control pipe env var. The pipe is a
    /// debug/test-only logout-cleanup surface, so its const, helpers, and reader
    /// spawn must all sit behind the same `#[cfg(any(debug_assertions, test))]`
    /// compile-time gate as the QA login pipe (engineering-rules: Secrets rule
    /// 2). This source-level assertion is the release gate: a release binary
    /// cannot compile the env read at all.
    #[test]
    fn qa_control_pipe_env_is_debug_or_test_only() {
        let source = include_str!("lib.rs");
        let const_decl = concat!("const QA_CONTROL", "_PIPE_ENV");
        let from_env = concat!("fn qa_control_pipe", "_path_from_env()");
        let spawn_reader = concat!("spawn_qa_control", "_pipe_reader");

        // Every place that names, reads, or wires the control pipe must sit
        // directly under the debug/test cfg gate, so a release binary cannot
        // even compile the env read (engineering-rules: Secrets rule 2).
        for token in [const_decl, from_env, spawn_reader] {
            let offset = source
                .find(token)
                .unwrap_or_else(|| panic!("expected `{token}` to exist in lib.rs"));
            let preceding = &source[..offset];
            let gate_offset = preceding
                .rfind("#[cfg(any(debug_assertions, test))]")
                .unwrap_or_else(|| panic!("`{token}` should be preceded by a debug/test cfg gate"));
            // The cfg gate must be the immediately-preceding attribute (nothing
            // but whitespace / single-line attributes between it and the item).
            let between = &preceding[gate_offset..];
            assert!(
                !between.contains("\n\n"),
                "`{token}` must sit directly under the debug/test cfg gate"
            );
        }

        // The env var is read exactly once, inside the gated `from_env` helper.
        let read_token = concat!("std::env::var(QA_CONTROL", "_PIPE_ENV)");
        assert_eq!(
            source.matches(read_token).count(),
            1,
            "control pipe env should be read once, only inside the gated from_env helper"
        );
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
    fn timeline_items_updated_forwarding_emits_core_event_name_and_all_diffs() {
        use matrix_desktop_core::{
            AccountKey, CoreEvent, TimelineDiff, TimelineEvent, TimelineKey,
            ids::{TimelineBatchId, TimelineGeneration},
        };
        use serde_json::json;

        let timeline_items_count = AtomicUsize::new(500);
        let diffs = (0..1000)
            .map(|index| TimelineDiff::Remove { index })
            .collect::<Vec<_>>();
        let event = CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
            key: TimelineKey::room(
                AccountKey("@u:example.test".to_owned()),
                "!room:example.test",
            ),
            generation: TimelineGeneration(7),
            batch_id: TimelineBatchId(13),
            diffs,
        });

        let forwarded = forwarded_webview_events_for_core_event(&event, &timeline_items_count);

        assert_eq!(timeline_items_count.load(Ordering::Relaxed), 0);
        assert_eq!(forwarded.len(), 1);
        assert_eq!(forwarded[0].event_name, CORE_EVENT_NAME);
        assert_eq!(forwarded[0].payload["kind"], json!("Timeline"));
        let diffs = forwarded[0].payload["event"]["ItemsUpdated"]["diffs"]
            .as_array()
            .expect("timeline diffs should serialize as an array");
        assert_eq!(diffs.len(), 1000);
        assert_eq!(diffs[0], json!({ "Remove": { "index": 0 } }));
        assert_eq!(diffs[999], json!({ "Remove": { "index": 999 } }));
    }

    #[test]
    fn state_changed_forwarding_emits_state_event_only() {
        use matrix_desktop_core::CoreEvent;
        use matrix_desktop_state::AppState;
        use serde_json::Value;

        let timeline_items_count = AtomicUsize::new(17);
        let event = CoreEvent::StateChanged(AppState::default());

        let forwarded = forwarded_webview_events_for_core_event(&event, &timeline_items_count);

        assert_eq!(timeline_items_count.load(Ordering::Relaxed), 17);
        assert_eq!(forwarded.len(), 1);
        assert_eq!(forwarded[0].event_name, STATE_EVENT_NAME);
        assert_eq!(
            forwarded[0].payload,
            Value::String("stateChanged".to_owned())
        );
    }

    #[test]
    fn lag_resync_forwarding_emits_state_then_resync_marker() {
        use matrix_desktop_state::AppState;
        use serde_json::json;

        let forwarded = forwarded_webview_events_for_lag_resync(&AppState::default());

        assert_eq!(forwarded.len(), 2);
        assert_eq!(forwarded[0].event_name, STATE_EVENT_NAME);
        assert_eq!(forwarded[0].payload, json!("stateChanged"));
        assert_eq!(forwarded[1].event_name, CORE_EVENT_NAME);
        assert_eq!(forwarded[1].payload, json!({ "kind": "ResyncMarker" }));
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

    /// Wire-format contract test: pins the serialized JSON shapes the React
    /// layer types against (apps/desktop/src/domain/coreEvents.ts). Serde
    /// enums are externally tagged: struct variants serialize as
    /// {"Variant":{..}}, unit variants as "Variant". If this test changes,
    /// coreEvents.ts and coreEvents.generated.json must change with it.
    #[test]
    fn core_event_wire_format_matches_checked_in_contract_artifact() {
        use matrix_desktop_core::{
            AccountKey, CoreEvent, TimelineDiff, TimelineKey,
            event::{
                AccountEvent, ActivityEvent, CjkTextPolicyEvent, E2eeTrustEvent, LinkPreview,
                LinkPreviewImage, LinkPreviewState, LiveSignalsEvent, LocalEncryptionEvent,
                NativeAttentionEvent, PaginationDirection, PaginationState,
                ReactionGroup, RoomEvent, SearchEvent, SyncEvent, ThreadsListEvent,
                TimelineCodeBlock, TimelineDisplayLabelUpdate, TimelineEvent, TimelineFormattedBody,
                TimelineItem, TimelineItemId, TimelineMedia, TimelineMediaKind, TimelineMediaSource,
                TimelineMediaThumbnail, TimelineMessageActions, TimelineMessageKind,
                TimelineMessageSource, TimelineNavigationSnapshot, TimelineResyncReason,
                TimelineSendFailureReason, TimelineSendState, TimelineSpoilerSpan,
                TimelineUnreadPosition,
            },
            failure::{CoreFailure, TimelineFailureKind},
            ids::{RequestId, RuntimeConnectionId, TimelineBatchId, TimelineGeneration},
        };
        use matrix_desktop_state::{
            ActivityRow, ActivityStream, ActivityTab, AttachmentKind, AttachmentResult,
            AvatarThumbnailState, DirectoryQuery, DirectoryRoomSummary, IdentityResetAuthType,
            IdentityResetState, JapaneseCatalogProfile, LiveEventReceipts, LiveReadReceipt,
            LiveRoomSignalUpdate, LocalEncryptionHealth, MediaTransferProgress,
            NativeAttentionCapabilities, NativeAttentionCapability, NativeAttentionSummary,
            PresenceKind, ReplyQuote, ReplyQuoteState, RoomHistoryVisibility, RoomJoinRule,
            RoomMemberRole, RoomModerationAction, RoomPermissionFacts, RoomSettingsSnapshot,
            RoomTagKind, SasEmoji, SearchCrawlerFailureKind, SyncMode, VerificationFlowState,
            VerificationTarget,
        };
        use serde_json::json;

        let request_id = RequestId {
            connection_id: RuntimeConnectionId(3),
            sequence: 7,
        };
        let key = TimelineKey::room(AccountKey("@u:example.test".to_owned()), "!r:example.test");
        let item = TimelineItem {
            id: TimelineItemId::Event {
                event_id: "$e1".to_owned(),
            },
            sender: Some("@u:example.test".to_owned()),
            sender_label: None,
            body: Some("hello".to_owned()),
            message_kind: TimelineMessageKind::Emote,
            spoiler_spans: vec![TimelineSpoilerSpan {
                start_utf16: 0,
                end_utf16: 5,
                reason: Some("fixture".to_owned()),
            }],
            timestamp_ms: Some(123),
            in_reply_to_event_id: None,
            formatted: Some(TimelineFormattedBody {
                html: "<strong>hello</strong><pre><code class=\"language-rust\">fn main() {}</code></pre>".to_owned(),
                plain_text: "hellofn main() {}".to_owned(),
                code_blocks: vec![TimelineCodeBlock {
                    language: Some("rust".to_owned()),
                    body: "fn main() {}".to_owned(),
                }],
            }),
            reply_quote: None,
            thread_root: None,
            thread_summary: None,
            media: None,
            link_previews: None,
            reactions: vec![ReactionGroup {
                key: "👍".to_owned(),
                count: 2,
                reacted_by_me: true,
                my_reaction_event_id: Some("$reaction:test".to_owned()),
                sender_preview: vec!["@u:example.test".to_owned()],
            }],
            can_react: true,
            is_redacted: false,
            is_hidden: false,
            can_redact: true,
            is_edited: true,
            can_edit: true,
            actions: TimelineMessageActions {
                can_copy: true,
                can_forward: true,
                can_permalink: true,
                can_view_source: true,
                permalink: Some("https://matrix.to/#/!r%3Aexample.test/%24e1".to_owned()),
            },
            send_state: None,
        };
        let media_item = TimelineItem {
            id: TimelineItemId::Event {
                event_id: "$media1".to_owned(),
            },
            sender: Some("@u:example.test".to_owned()),
            sender_label: None,
            body: Some("caption".to_owned()),
            message_kind: Default::default(),
            spoiler_spans: Vec::new(),
            timestamp_ms: Some(456),
            in_reply_to_event_id: None,
            formatted: None,
            reply_quote: None,
            thread_root: None,
            thread_summary: None,
            media: Some(TimelineMedia {
                kind: TimelineMediaKind::Image,
                filename: "fixture.png".to_owned(),
                source: TimelineMediaSource {
                    mxc_uri: "mxc://example.test/media".to_owned(),
                    encrypted: true,
                    encryption_version: Some("v2".to_owned()),
                },
                mimetype: Some("image/png".to_owned()),
                size: Some(68),
                width: Some(2),
                height: Some(2),
                thumbnail: Some(TimelineMediaThumbnail {
                    source: TimelineMediaSource {
                        mxc_uri: "mxc://example.test/thumb".to_owned(),
                        encrypted: false,
                        encryption_version: None,
                    },
                    mimetype: Some("image/png".to_owned()),
                    size: Some(32),
                    width: Some(1),
                    height: Some(1),
                }),
            }),
            link_previews: None,
            reactions: Vec::new(),
            can_react: true,
            is_redacted: false,
            is_hidden: false,
            can_redact: true,
            is_edited: false,
            can_edit: false,
            actions: TimelineMessageActions {
                can_copy: true,
                can_forward: true,
                can_permalink: true,
                can_view_source: true,
                permalink: Some("https://matrix.to/#/!r%3Aexample.test/%24media1".to_owned()),
            },
            send_state: None,
        };
        let send_state_item = TimelineItem {
            id: TimelineItemId::Transaction {
                transaction_id: "txn-not-sent".to_owned(),
            },
            sender: Some("@u:example.test".to_owned()),
            sender_label: None,
            body: Some("queued".to_owned()),
            message_kind: Default::default(),
            spoiler_spans: Vec::new(),
            timestamp_ms: Some(789),
            in_reply_to_event_id: None,
            formatted: None,
            reply_quote: None,
            thread_root: None,
            thread_summary: None,
            media: None,
            link_previews: None,
            reactions: Vec::new(),
            can_react: false,
            is_redacted: false,
            is_hidden: false,
            can_redact: false,
            is_edited: false,
            can_edit: false,
            actions: TimelineMessageActions::default(),
            send_state: Some(TimelineSendState::NotSent {
                reason: TimelineSendFailureReason::Recoverable,
            }),
        };
        let reply_quote_item = TimelineItem {
            id: TimelineItemId::Event {
                event_id: "$reply1".to_owned(),
            },
            sender: Some("@u:example.test".to_owned()),
            sender_label: None,
            body: Some("reply body".to_owned()),
            message_kind: Default::default(),
            spoiler_spans: Vec::new(),
            timestamp_ms: Some(987),
            in_reply_to_event_id: Some("$root1".to_owned()),
            formatted: None,
            reply_quote: Some(ReplyQuote {
                event_id: "$root1".to_owned(),
                sender: Some("@other:example.test".to_owned()),
                sender_label: None,
                body_preview: Some("quoted preview".to_owned()),
                state: ReplyQuoteState::Ready,
            }),
            thread_root: None,
            thread_summary: None,
            media: None,
            link_previews: None,
            reactions: Vec::new(),
            can_react: true,
            is_redacted: false,
            is_hidden: false,
            can_redact: true,
            is_edited: false,
            can_edit: false,
            actions: TimelineMessageActions {
                can_copy: true,
                can_forward: true,
                can_permalink: true,
                can_view_source: true,
                permalink: Some("https://matrix.to/#/!r%3Aexample.test/%24reply1".to_owned()),
            },
            send_state: None,
        };
        let link_preview_item = TimelineItem {
            id: TimelineItemId::Event {
                event_id: "$linkpreview1".to_owned(),
            },
            sender: Some("@u:example.test".to_owned()),
            sender_label: None,
            body: Some("Check out https://example.invalid/page".to_owned()),
            message_kind: Default::default(),
            spoiler_spans: Vec::new(),
            timestamp_ms: Some(1111),
            in_reply_to_event_id: None,
            formatted: None,
            reply_quote: None,
            thread_root: None,
            thread_summary: None,
            media: None,
            link_previews: Some(vec![LinkPreview {
                url: "https://example.invalid/page".to_owned(),
                title: Some("Example Page".to_owned()),
                description: Some("A synthetic fixture page.".to_owned()),
                image: Some(LinkPreviewImage {
                    source: TimelineMediaSource {
                        mxc_uri: "mxc://example.invalid/preview-image".to_owned(),
                        encrypted: false,
                        encryption_version: None,
                    },
                    width: Some(1200),
                    height: Some(630),
                    thumbnail: AvatarThumbnailState::Ready {
                        source_url: "file:///tmp/link-preview-thumbnails/fixture.bin".to_owned(),
                        width: Some(600),
                        height: Some(315),
                        mime_type: Some("image/png".to_owned()),
                    },
                }),
                state: LinkPreviewState::Ready,
            }]),
            reactions: Vec::new(),
            can_react: true,
            is_redacted: false,
            is_hidden: false,
            can_redact: true,
            is_edited: false,
            can_edit: false,
            actions: TimelineMessageActions {
                can_copy: true,
                can_forward: true,
                can_permalink: true,
                can_view_source: true,
                permalink: Some("https://matrix.to/#/!r%3Aexample.test/%24linkpreview1".to_owned()),
            },
            send_state: None,
        };

        // InitialItems envelope + payload
        let initial = serialize_core_event(&CoreEvent::Timeline(TimelineEvent::InitialItems {
            request_id: Some(request_id),
            key: key.clone(),
            generation: TimelineGeneration(1),
            items: vec![item.clone()],
        }))
        .expect("timeline events serialize");
        assert_eq!(initial["kind"], json!("Timeline"));
        let payload = &initial["event"]["InitialItems"];
        assert_eq!(
            payload["request_id"],
            json!({ "connection_id": 3, "sequence": 7 })
        );
        assert_eq!(
            payload["key"],
            json!({
                "account_key": "@u:example.test",
                "kind": { "Room": { "room_id": "!r:example.test" } }
            })
        );
        assert_eq!(payload["generation"], json!(1));
        assert_eq!(
            payload["items"][0],
            json!({
                "id": { "Event": { "event_id": "$e1" } },
                "sender": "@u:example.test",
                "sender_label": null,
                "body": "hello",
                "message_kind": "emote",
                "spoiler_spans": [
                    {
                        "start_utf16": 0,
                        "end_utf16": 5,
                        "reason": "fixture"
                    }
                ],
                "timestamp_ms": 123,
                "in_reply_to_event_id": null,
                "formatted": {
                    "html": "<strong>hello</strong><pre><code class=\"language-rust\">fn main() {}</code></pre>",
                    "plain_text": "hellofn main() {}",
                    "code_blocks": [
                        {
                            "language": "rust",
                            "body": "fn main() {}"
                        }
                    ]
                },
                "thread_root": null,
                "thread_summary": null,
                "can_react": true,
                "is_redacted": false,
                "is_hidden": false,
                "can_redact": true,
                "is_edited": true,
                "can_edit": true,
                "actions": {
                    "can_copy": true,
                    "can_forward": true,
                    "can_permalink": true,
                    "can_view_source": true,
                    "permalink": "https://matrix.to/#/!r%3Aexample.test/%24e1"
                },
                "reactions": [
                    {
                        "key": "👍",
                        "count": 2,
                        "reacted_by_me": true,
                        "my_reaction_event_id": "$reaction:test",
                        "sender_preview": ["@u:example.test"]
                    }
                ]
            })
        );

        // ItemsUpdated: diffs are externally tagged; unit variants are strings
        let updated = serialize_core_event(&CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
            key: key.clone(),
            generation: TimelineGeneration(1),
            batch_id: TimelineBatchId(9),
            diffs: vec![
                TimelineDiff::PushFront { item: item.clone() },
                TimelineDiff::Remove { index: 2 },
                TimelineDiff::Clear,
            ],
        }))
        .expect("serialize");
        let diffs = &updated["event"]["ItemsUpdated"]["diffs"];
        assert!(diffs[0]["PushFront"]["item"]["id"]["Event"]["event_id"] == json!("$e1"));
        assert_eq!(diffs[1], json!({ "Remove": { "index": 2 } }));
        assert_eq!(diffs[2], json!("Clear"));
        assert_eq!(updated["event"]["ItemsUpdated"]["batch_id"], json!(9));

        let media_initial =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::InitialItems {
                request_id: Some(request_id),
                key: key.clone(),
                generation: TimelineGeneration(2),
                items: vec![media_item],
            }))
            .expect("serialize media initial items");
        assert_eq!(
            media_initial["event"]["InitialItems"]["items"][0]["media"],
            json!({
                "kind": "Image",
                "filename": "fixture.png",
                "source": {
                    "mxc_uri": "mxc://example.test/media",
                    "encrypted": true,
                    "encryption_version": "v2"
                },
                "mimetype": "image/png",
                "size": 68,
                "width": 2,
                "height": 2,
                "thumbnail": {
                    "source": {
                        "mxc_uri": "mxc://example.test/thumb",
                        "encrypted": false,
                        "encryption_version": null
                    },
                    "mimetype": "image/png",
                    "size": 32,
                    "width": 1,
                    "height": 1
                }
            })
        );

        let send_state_initial =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::InitialItems {
                request_id: Some(request_id),
                key: key.clone(),
                generation: TimelineGeneration(3),
                items: vec![send_state_item],
            }))
            .expect("serialize send-state initial items");
        assert_eq!(
            send_state_initial["event"]["InitialItems"]["items"][0]["send_state"],
            json!({
                "kind": "notSent",
                "reason": "recoverable"
            })
        );

        let reply_quote_initial =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::InitialItems {
                request_id: Some(request_id),
                key: key.clone(),
                generation: TimelineGeneration(4),
                items: vec![reply_quote_item],
            }))
            .expect("serialize reply quote initial items");
        assert_eq!(
            reply_quote_initial["event"]["InitialItems"]["items"][0]["reply_quote"],
            json!({
                "event_id": "$root1",
                "sender": "@other:example.test",
                "sender_label": null,
                "body_preview": "quoted preview",
                "state": "ready"
            })
        );

        let link_preview_initial =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::InitialItems {
                request_id: Some(request_id),
                key: key.clone(),
                generation: TimelineGeneration(5),
                items: vec![link_preview_item],
            }))
            .expect("serialize link preview initial items");
        assert_eq!(
            link_preview_initial["event"]["InitialItems"]["items"][0]["link_previews"],
            json!([
                {
                    "url": "https://example.invalid/page",
                    "title": "Example Page",
                    "description": "A synthetic fixture page.",
                    "image": {
                        "source": {
                            "mxc_uri": "mxc://example.invalid/preview-image",
                            "encrypted": false,
                            "encryption_version": null
                        },
                        "width": 1200,
                        "height": 630,
                        "thumbnail": {
                            "kind": "ready",
                            "source_url": "file:///tmp/link-preview-thumbnails/fixture.bin",
                            "width": 600,
                            "height": 315,
                            "mime_type": "image/png"
                        }
                    },
                    "state": "ready"
                }
            ])
        );

        let media_upload_progress =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::MediaUploadProgress {
                request_id: Some(request_id),
                key: key.clone(),
                transaction_id: "txn-media".to_owned(),
                index: 0,
                progress: MediaTransferProgress {
                    current: 1,
                    total: 2,
                },
                source: Some(TimelineMediaSource {
                    mxc_uri: "mxc://example.test/media".to_owned(),
                    encrypted: false,
                    encryption_version: None,
                }),
            }))
            .expect("serialize media upload progress");

        let media_download_progress = serialize_core_event(&CoreEvent::Timeline(
            TimelineEvent::MediaDownloadProgress {
                request_id,
                key: key.clone(),
                event_id: "$media1".to_owned(),
                progress: MediaTransferProgress {
                    current: 0,
                    total: 68,
                },
            },
        ))
        .expect("serialize media download progress");

        let media_download_completed = serialize_core_event(&CoreEvent::Timeline(
            TimelineEvent::MediaDownloadCompleted {
                request_id,
                key: key.clone(),
                event_id: "$media1".to_owned(),
                source_url: "/data/media_downloads/!r:example.test/$media1.bin".to_owned(),
                byte_count: 68,
                mimetype: Some("image/png".to_owned()),
                width: Some(2),
                height: Some(2),
            },
        ))
        .expect("serialize media download completion");

        let media_download_failed = serialize_core_event(&CoreEvent::Timeline(
            TimelineEvent::MediaDownloadFailed {
                request_id,
                key: key.clone(),
                event_id: "$media1".to_owned(),
                kind: TimelineFailureKind::Sdk,
            },
        ))
        .expect("serialize media download failure");

        let message_source_loaded =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::MessageSourceLoaded {
                request_id,
                key: key.clone(),
                source: TimelineMessageSource {
                    event_id: "$e1".to_owned(),
                    sender: Some("@u:example.test".to_owned()),
                    timestamp_ms: Some(123),
                    body: Some("hello".to_owned()),
                    in_reply_to_event_id: None,
                    thread_root: None,
                    is_redacted: false,
                    is_edited: true,
                    has_media: false,
                },
            }))
            .expect("serialize message source loaded");
        let message_forwarded =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::MessageForwarded {
                request_id,
                key: key.clone(),
                destination_room_id: "!destination:example.test".to_owned(),
                transaction_id: "txn-forward".to_owned(),
                event_id: "$forwarded1".to_owned(),
            }))
            .expect("serialize message forwarded");

        // PaginationStateChanged: unit states are strings, Failed is tagged
        let pagination = serialize_core_event(&CoreEvent::Timeline(
            TimelineEvent::PaginationStateChanged {
                request_id: None,
                key: key.clone(),
                direction: PaginationDirection::Backward,
                state: PaginationState::EndReached,
            },
        ))
        .expect("serialize");
        let pagination = &pagination["event"]["PaginationStateChanged"];
        assert_eq!(pagination["request_id"], json!(null));
        assert_eq!(pagination["direction"], json!("Backward"));
        assert_eq!(pagination["state"], json!("EndReached"));

        // ResyncRequired reason is a string
        let resync = serialize_core_event(&CoreEvent::Timeline(TimelineEvent::ResyncRequired {
            key: key.clone(),
            reason: TimelineResyncReason::QueueOverflow,
        }))
        .expect("serialize");
        assert_eq!(
            resync["event"]["ResyncRequired"]["reason"],
            json!("QueueOverflow")
        );

        let navigation_updated =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::NavigationUpdated {
                key: key.clone(),
                snapshot: TimelineNavigationSnapshot {
                    read_marker_event_id: Some("$read:example.test".to_owned()),
                    first_unread_event_id: Some("$unread:example.test".to_owned()),
                    unread_event_count: 2,
                    unread_position: TimelineUnreadPosition::BelowViewport,
                    newer_event_count: 3,
                    can_jump_to_bottom: true,
                },
            }))
            .expect("serialize navigation update event");
        assert_eq!(
            navigation_updated["event"]["NavigationUpdated"]["snapshot"]["unread_position"],
            json!("belowViewport")
        );

        let display_labels_updated =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::DisplayLabelsUpdated {
                labels: vec![TimelineDisplayLabelUpdate {
                    user_id: "@u:example.test".to_owned(),
                    display_label: "User Alias".to_owned(),
                }],
            }))
            .expect("serialize display label update event");
        assert_eq!(
            display_labels_updated["event"]["DisplayLabelsUpdated"]["labels"][0],
            json!({
                "user_id": "@u:example.test",
                "display_label": "User Alias"
            })
        );
        let display_policy_updated =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::DisplayPolicyUpdated {
                hide_redacted: true,
            }))
            .expect("serialize display policy update event");
        assert_eq!(
            display_policy_updated["event"]["DisplayPolicyUpdated"]["hide_redacted"],
            json!(true)
        );

        // Account events are externally tagged under the Account envelope
        let listed = serialize_core_event(&CoreEvent::Account(AccountEvent::SavedSessionsListed {
            request_id,
            sessions: vec![matrix_desktop_state::SessionInfo {
                homeserver: "https://example.test".to_owned(),
                user_id: "@u:example.test".to_owned(),
                device_id: "DEV".to_owned(),
            }],
        }))
        .expect("serialize");
        assert_eq!(listed["kind"], json!("Account"));
        assert_eq!(
            listed["event"]["SavedSessionsListed"]["sessions"][0]["device_id"],
            json!("DEV")
        );
        let profile_updated =
            serialize_core_event(&CoreEvent::Account(AccountEvent::ProfileUpdated {
                request_id,
                account_key: AccountKey("@u:example.test".to_owned()),
            }))
            .expect("serialize profile update event");

        let account_report_completed =
            serialize_core_event(&CoreEvent::Account(AccountEvent::ReportCompleted {
                request_id,
                kind: matrix_desktop_core::event::ReportKind::User,
            }))
            .expect("serialize account report completed event");

        // OperationFailed: unit failures are strings
        let failed = serialize_core_event(&CoreEvent::OperationFailed {
            request_id,
            failure: CoreFailure::SessionNotFound,
        })
        .expect("serialize");
        assert_eq!(failed["kind"], json!("OperationFailed"));
        assert_eq!(failed["failure"], json!("SessionNotFound"));

        let room_left = serialize_core_event(&CoreEvent::Room(RoomEvent::RoomLeft {
            request_id,
            room_id: "!r:example.test".to_owned(),
        }))
        .expect("serialize");
        assert_eq!(room_left["kind"], json!("Room"));
        assert_eq!(
            room_left["event"]["RoomLeft"]["room_id"],
            json!("!r:example.test")
        );

        let room_invite_accepted =
            serialize_core_event(&CoreEvent::Room(RoomEvent::InviteAccepted {
                request_id,
                room_id: "!r:example.test".to_owned(),
            }))
            .expect("serialize");
        let room_invite_declined =
            serialize_core_event(&CoreEvent::Room(RoomEvent::InviteDeclined {
                request_id,
                room_id: "!r:example.test".to_owned(),
            }))
            .expect("serialize");
        let room_direct_message_started =
            serialize_core_event(&CoreEvent::Room(RoomEvent::DirectMessageStarted {
                request_id,
                room_id: "!dm:example.test".to_owned(),
            }))
            .expect("serialize");
        let room_tag_set = serialize_core_event(&CoreEvent::Room(RoomEvent::RoomTagSet {
            request_id,
            room_id: "!r:example.test".to_owned(),
            tag: RoomTagKind::Favourite,
        }))
        .expect("serialize room tag set");
        let room_tag_removed = serialize_core_event(&CoreEvent::Room(RoomEvent::RoomTagRemoved {
            request_id,
            room_id: "!r:example.test".to_owned(),
            tag: RoomTagKind::LowPriority,
        }))
        .expect("serialize room tag removed");
        let room_marked_as_read = serialize_core_event(&CoreEvent::Room(RoomEvent::MarkedAsRead {
            request_id,
            room_id: "!r:example.test".to_owned(),
        }))
        .expect("serialize room marked as read");
        let room_marked_as_unread =
            serialize_core_event(&CoreEvent::Room(RoomEvent::MarkedAsUnread {
                request_id,
                room_id: "!r:example.test".to_owned(),
                unread: true,
            }))
            .expect("serialize room marked as unread");
        let room_report_completed =
            serialize_core_event(&CoreEvent::Room(RoomEvent::ReportCompleted {
                request_id,
                kind: matrix_desktop_core::event::ReportKind::Event,
            }))
            .expect("serialize room report completed event");
        let sync_mode_changed = serialize_core_event(&CoreEvent::Sync(SyncEvent::ModeChanged {
            mode: SyncMode::Simplified,
        }))
        .expect("serialize sync mode changed");
        let directory_query_completed =
            serialize_core_event(&CoreEvent::Room(RoomEvent::DirectoryQueryCompleted {
                request_id,
                query: DirectoryQuery {
                    term: Some("public".to_owned()),
                    server_name: Some("example.test".to_owned()),
                    limit: Some(20),
                    since: Some("page-2".to_owned()),
                },
                rooms: vec![DirectoryRoomSummary {
                    room_id: "!public:example.test".to_owned(),
                    canonical_alias: Some("#public:example.test".to_owned()),
                    name: "Public Room".to_owned(),
                    topic: Some("Directory sample".to_owned()),
                    avatar_url: None,
                    joined_members: 5,
                    world_readable: true,
                    guest_can_join: false,
                }],
                next_batch: Some("page-3".to_owned()),
            }))
            .expect("serialize directory query completion");
        let room_settings_snapshot = RoomSettingsSnapshot {
            room_id: "!r:example.test".to_owned(),
            name: Some("Room Settings Sample".to_owned()),
            topic: Some("Private topic sample".to_owned()),
            avatar_url: Some("mxc://example.test/avatar".to_owned()),
            join_rule: RoomJoinRule::Invite,
            history_visibility: RoomHistoryVisibility::Shared,
            permissions: RoomPermissionFacts {
                can_edit_settings: true,
                can_edit_roles: true,
                can_kick: true,
                can_ban: true,
                can_unban: true,
            },
            members: vec![matrix_desktop_state::RoomMemberSummary {
                user_id: "@member:example.test".to_owned(),
                display_name: Some("Synthetic Member".to_owned()),
                display_label: "Synthetic Member".to_owned(),
                original_display_label: "Synthetic Member".to_owned(),
                avatar_url: Some("mxc://example.test/member-avatar".to_owned()),
                power_level: Some(50),
                role: RoomMemberRole::Moderator,
            }],
        };
        let room_settings_loaded =
            serialize_core_event(&CoreEvent::Room(RoomEvent::RoomSettingsLoaded {
                request_id,
                settings: room_settings_snapshot.clone(),
            }))
            .expect("serialize room settings loaded");
        let room_setting_updated =
            serialize_core_event(&CoreEvent::Room(RoomEvent::RoomSettingUpdated {
                request_id,
                settings: room_settings_snapshot,
            }))
            .expect("serialize room setting updated");
        let room_member_moderated =
            serialize_core_event(&CoreEvent::Room(RoomEvent::RoomMemberModerated {
                request_id,
                room_id: "!r:example.test".to_owned(),
                target_user_id: "@target:example.test".to_owned(),
                action: RoomModerationAction::Kick,
            }))
            .expect("serialize room member moderated");
        let room_member_role_updated =
            serialize_core_event(&CoreEvent::Room(RoomEvent::RoomMemberRoleUpdated {
                request_id,
                room_id: "!r:example.test".to_owned(),
                target_user_id: "@target:example.test".to_owned(),
                power_level: 50,
            }))
            .expect("serialize room member role updated");
        assert_eq!(
            room_settings_loaded["event"]["RoomSettingsLoaded"]["settings"]["permissions"]["can_edit_settings"],
            json!(true)
        );
        assert_eq!(
            room_settings_loaded["event"]["RoomSettingsLoaded"]["settings"]["permissions"]["can_edit_roles"],
            json!(true)
        );
        assert_eq!(
            room_settings_loaded["event"]["RoomSettingsLoaded"]["settings"]["members"][0]["role"],
            json!("moderator")
        );
        assert_eq!(
            room_member_moderated["event"]["RoomMemberModerated"]["action"],
            json!("kick")
        );
        assert_eq!(
            room_member_role_updated["event"]["RoomMemberRoleUpdated"]["power_level"],
            json!(50)
        );

        let e2ee_trust = serialize_core_event(&CoreEvent::E2eeTrust(
            E2eeTrustEvent::VerificationProgress {
                account_key: AccountKey("@u:example.test".to_owned()),
                state: VerificationFlowState::SasPresented {
                    request_id: request_id.sequence,
                    target: VerificationTarget {
                        user_id: "@other:example.test".to_owned(),
                        device_id: "OTHERDEVICE".to_owned(),
                    },
                    emojis: vec![SasEmoji {
                        symbol: "🐶".to_owned(),
                        description: "Dog".to_owned(),
                    }],
                },
            },
        ))
        .expect("serialize");
        assert_eq!(e2ee_trust["kind"], json!("E2eeTrust"));
        assert_eq!(e2ee_trust["event"]["kind"], json!("verificationProgress"));
        assert_eq!(e2ee_trust["event"]["state"]["kind"], json!("sasPresented"));

        let e2ee_identity_reset = serialize_core_event(&CoreEvent::E2eeTrust(
            E2eeTrustEvent::IdentityResetChanged {
                account_key: AccountKey("@u:example.test".to_owned()),
                state: IdentityResetState::AwaitingAuth {
                    request_id: request_id.sequence,
                    auth_type: IdentityResetAuthType::Uiaa,
                },
            },
        ))
        .expect("serialize identity reset event");
        assert_eq!(
            e2ee_identity_reset["event"]["kind"],
            json!("identityResetChanged")
        );
        assert_eq!(
            e2ee_identity_reset["event"]["state"]["kind"],
            json!("awaitingAuth")
        );

        let live_signals = serialize_core_event(&CoreEvent::LiveSignals(
            LiveSignalsEvent::RoomSignalsUpdated {
                room_id: "!r:example.test".to_owned(),
                update: LiveRoomSignalUpdate {
                    receipts_by_event: vec![LiveEventReceipts {
                        event_id: "$e1".to_owned(),
                        receipts: vec![LiveReadReceipt {
                            user_id: "@other:example.test".to_owned(),
                            display_name: Some("Other".to_owned()),
                            original_display_label: String::new(),
                            avatar: None,
                            timestamp_ms: Some(123),
                        }],
                    }],
                    fully_read_event_id: Some("$e1".to_owned()),
                    typing_user_ids: vec!["@other:example.test".to_owned()],
                },
            },
        ))
        .expect("serialize live signals event");
        assert_eq!(live_signals["kind"], json!("LiveSignals"));
        assert_eq!(live_signals["event"]["kind"], json!("roomSignalsUpdated"));

        let live_presence =
            serialize_core_event(&CoreEvent::LiveSignals(LiveSignalsEvent::PresenceSet {
                request_id,
                presence: PresenceKind::Away,
            }))
            .expect("serialize live presence event");
        assert_eq!(live_presence["event"]["kind"], json!("presenceSet"));

        let activity_opened =
            serialize_core_event(&CoreEvent::Activity(ActivityEvent::Opened { request_id }))
                .expect("serialize activity event");
        assert_eq!(activity_opened["kind"], json!("Activity"));
        assert_eq!(
            activity_opened["event"]["Opened"]["request_id"],
            json!({ "connection_id": 3, "sequence": 7 })
        );
        let activity_snapshot_loaded =
            serialize_core_event(&CoreEvent::Activity(ActivityEvent::SnapshotLoaded {
                request_id,
                active_tab: ActivityTab::Unread,
                recent: ActivityStream {
                    rows: vec![ActivityRow {
                        room_id: "!activity-recent:example.test".to_owned(),
                        event_id: "$activity-recent:example.test".to_owned(),
                        room_label: "Recent room".to_owned(),
                        sender_label: Some("Recent sender".to_owned()),
                        preview: Some("Recent preview".to_owned()),
                        timestamp_ms: 20,
                        unread: false,
                        highlight: false,
                    }],
                    next_batch: Some("recent-next".to_owned()),
                },
                unread: ActivityStream {
                    rows: vec![ActivityRow {
                        room_id: "!activity-unread:example.test".to_owned(),
                        event_id: "$activity-unread:example.test".to_owned(),
                        room_label: "Unread room".to_owned(),
                        sender_label: Some("Unread sender".to_owned()),
                        preview: Some("Unread preview".to_owned()),
                        timestamp_ms: 10,
                        unread: true,
                        highlight: true,
                    }],
                    next_batch: Some("unread-next".to_owned()),
                },
            }))
            .expect("serialize activity snapshot event");
        assert_eq!(
            activity_snapshot_loaded["event"]["SnapshotLoaded"]["active_tab"],
            json!("unread")
        );
        assert_eq!(
            activity_snapshot_loaded["event"]["SnapshotLoaded"]["unread"]["rows"][0]["highlight"],
            json!(true)
        );
        let activity_marked_read =
            serialize_core_event(&CoreEvent::Activity(ActivityEvent::MarkedRead {
                request_id,
                cleared_event_ids: vec!["$activity-unread:example.test".to_owned()],
            }))
            .expect("serialize activity marked-read event");
        assert_eq!(
            activity_marked_read["event"]["MarkedRead"]["cleared_event_ids"],
            json!(["$activity-unread:example.test"])
        );

        let local_encryption = serialize_core_event(&CoreEvent::LocalEncryption(
            LocalEncryptionEvent::HealthChanged {
                health: LocalEncryptionHealth::Healthy,
            },
        ))
        .expect("serialize local encryption event");
        assert_eq!(local_encryption["event"]["kind"], json!("healthChanged"));
        assert_eq!(local_encryption["event"]["health"], json!("healthy"));

        let native_attention = serialize_core_event(&CoreEvent::NativeAttention(
            NativeAttentionEvent::SummaryUpdated {
                summary: NativeAttentionSummary {
                    unread_count: 3,
                    highlight_count: 1,
                    badge_count: 3,
                    candidate: None,
                    capabilities: NativeAttentionCapabilities {
                        notifications: NativeAttentionCapability::Available,
                        badge: NativeAttentionCapability::Available,
                        overlay_icon: NativeAttentionCapability::Unknown,
                        sound: NativeAttentionCapability::Unavailable,
                        tray: NativeAttentionCapability::Unknown,
                        activation: NativeAttentionCapability::Available,
                    },
                },
            },
        ))
        .expect("serialize native attention event");
        assert_eq!(native_attention["event"]["kind"], json!("summaryUpdated"));
        assert_eq!(
            native_attention["event"]["summary"]["badge_count"],
            json!(3)
        );

        let cjk_text_policy = serialize_core_event(&CoreEvent::CjkTextPolicy(
            CjkTextPolicyEvent::JapaneseCatalogProfileChanged {
                profile: JapaneseCatalogProfile {
                    catalog_locale: "ja".to_owned(),
                    complete: false,
                    missing_message_ids: vec!["settings.title".to_owned()],
                },
            },
        ))
        .expect("serialize cjk text policy event");
        assert_eq!(
            cjk_text_policy["event"]["kind"],
            json!("japaneseCatalogProfileChanged")
        );

        let search_attachments_results =
            serialize_core_event(&CoreEvent::Search(SearchEvent::AttachmentsResults {
                request_id,
                results: vec![AttachmentResult {
                    room_id: "!r:example.test".to_owned(),
                    event_id: "$f1".to_owned(),
                    sender: "@u:example.test".to_owned(),
                    timestamp_ms: 1,
                    kind: AttachmentKind::Image,
                    filename: "photo.png".to_owned(),
                    mimetype: Some("image/png".to_owned()),
                    size: Some(1234),
                    source_mxc: "mxc://example.invalid/abc".to_owned(),
                    thumbnail_mxc: Some("mxc://example.invalid/abc-thumb".to_owned()),
                    thread_root: None,
                    encrypted: false,
                    encryption_version: None,
                    width: None,
                    height: None,
                    is_edited: false,
                }],
            }))
            .expect("serialize search attachments results event");
        assert_eq!(
            search_attachments_results["event"]["AttachmentsResults"]["results"][0]["kind"],
            json!("image")
        );

        let search_attachments_failed =
            serialize_core_event(&CoreEvent::Search(SearchEvent::AttachmentsFailed {
                request_id,
                message: "index unavailable".to_owned(),
            }))
            .expect("serialize search attachments failed event");
        assert_eq!(
            search_attachments_failed["event"]["AttachmentsFailed"]["message"],
            json!("index unavailable")
        );

        // Search history crawler contract events (#77).
        let search_crawl_progress = serialize_core_event(&CoreEvent::Search(
            SearchEvent::HistoryCrawlProgress {
                room_id: "!r:example.test".to_owned(),
                processed: 100,
                indexed: 42,
            },
        ))
        .expect("serialize history crawl progress event");
        assert_eq!(
            search_crawl_progress["event"]["HistoryCrawlProgress"]["processed"],
            json!(100u64)
        );

        let search_crawl_completed = serialize_core_event(&CoreEvent::Search(
            SearchEvent::HistoryCrawlCompleted {
                room_id: "!r:example.test".to_owned(),
                indexed: 42,
            },
        ))
        .expect("serialize history crawl completed event");
        assert_eq!(
            search_crawl_completed["event"]["HistoryCrawlCompleted"]["indexed"],
            json!(42u64)
        );

        let search_crawl_failed = serialize_core_event(&CoreEvent::Search(
            SearchEvent::HistoryCrawlFailed {
                room_id: "!r:example.test".to_owned(),
                kind: SearchCrawlerFailureKind::Sdk,
            },
        ))
        .expect("serialize history crawl failed event");
        assert_eq!(
            search_crawl_failed["event"]["HistoryCrawlFailed"]["failureKind"],
            json!("sdk")
        );
        // Privacy assertion: no raw error text in the failed event.
        assert!(
            !serde_json::to_string(&search_crawl_failed).unwrap().contains("message"),
            "crawl failure must not carry a raw message field"
        );

        let actual_contract = json!({
            "activityOpened": activity_opened,
            "activityMarkedRead": activity_marked_read,
            "activitySnapshotLoaded": activity_snapshot_loaded,
            "cjkTextPolicyJapaneseCatalogProfileChanged": cjk_text_policy,
            "e2eeTrustIdentityResetChanged": e2ee_identity_reset,
            "accountProfileUpdated": profile_updated,
            "accountReportCompleted": account_report_completed,
            "accountSavedSessionsListed": listed,
            "e2eeTrustVerificationProgress": e2ee_trust,
            "localEncryptionHealthChanged": local_encryption,
            "liveSignalsPresenceSet": live_presence,
            "liveSignalsRoomSignalsUpdated": live_signals,
            "nativeAttentionSummaryUpdated": native_attention,
            "operationFailedSessionNotFound": failed,
            "searchAttachmentsFailed": search_attachments_failed,
            "searchAttachmentsResults": search_attachments_results,
            "searchCrawlProgress": search_crawl_progress,
            "searchCrawlCompleted": search_crawl_completed,
            "searchCrawlFailed": search_crawl_failed,
            "roomDirectoryQueryCompleted": directory_query_completed,
            "roomDirectMessageStarted": room_direct_message_started,
            "roomInviteAccepted": room_invite_accepted,
            "roomInviteDeclined": room_invite_declined,
            "roomLeft": room_left,
            "roomMarkedAsRead": room_marked_as_read,
            "roomMarkedAsUnread": room_marked_as_unread,
            "roomReportCompleted": room_report_completed,
            "roomMemberModerated": room_member_moderated,
            "roomMemberRoleUpdated": room_member_role_updated,
            "roomSettingUpdated": room_setting_updated,
            "roomSettingsLoaded": room_settings_loaded,
            "roomTagRemoved": room_tag_removed,
            "roomTagSet": room_tag_set,
            "syncModeChanged": sync_mode_changed,
            "timelineDisplayLabelsUpdated": display_labels_updated,
            "timelineDisplayPolicyUpdated": display_policy_updated,
            "timelineInitialItems": initial,
            "timelineItemsUpdated": updated,
            "timelineLinkPreviewInitialItems": link_preview_initial,
            "timelineMediaDownloadCompleted": media_download_completed,
            "timelineMediaDownloadFailed": media_download_failed,
            "timelineMediaDownloadProgress": media_download_progress,
            "timelineMediaInitialItems": media_initial,
            "timelineMediaUploadProgress": media_upload_progress,
            "timelineMessageForwarded": message_forwarded,
            "timelineMessageSourceLoaded": message_source_loaded,
            "timelineNavigationUpdated": navigation_updated,
            "timelinePaginationEndReached": serialize_core_event(&CoreEvent::Timeline(
                TimelineEvent::PaginationStateChanged {
                    request_id: None,
                    key: key.clone(),
                    direction: PaginationDirection::Backward,
                    state: PaginationState::EndReached,
                },
            ))
            .expect("serialize"),
            "timelineReplyQuoteInitialItems": reply_quote_initial,
            "timelineResyncRequired": resync,
            "timelineSendStateInitialItems": send_state_initial,
            "threadsListOpened": serialize_core_event(&CoreEvent::ThreadsList(
                ThreadsListEvent::Opened {
                    request_id,
                    room_id: "!room:example.test".to_owned(),
                    items: vec![],
                    end_reached: false,
                },
            ))
            .expect("serialize threads list opened"),
        });
        let checked_in_contract: serde_json::Value =
            serde_json::from_str(include_str!("../../src/domain/coreEvents.generated.json"))
                .expect("checked-in core event contract artifact must be valid JSON");
        assert_eq!(actual_contract, checked_in_contract);
    }

    /// CoreEvent IPC-contract key-completeness guard.
    ///
    /// The `core_event_wire_format_matches_checked_in_contract_artifact` test
    /// proves the Rust-serialized shapes equal the checked-in JSON. This
    /// companion test locks in the EXACT SET of keys so a later refactor
    /// cannot accidentally remove a variant from the artifact without being
    /// caught — even if the remaining keys still match.
    ///
    /// If a new `CoreEvent` variant is added, extend `core_event_wire_format_...`
    /// first (to produce the serialized form), update the artifact, then this
    /// expected set gains the new key automatically (it reads the artifact). The
    /// test therefore functions as a "no-shrink" guard: the key count must not
    /// decrease, and every key must remain in the known-valid set derived from
    /// the Rust contract test above.
    #[test]
    fn core_event_contract_artifact_key_set_does_not_shrink() {
        // This set is the canonical key list produced by the Rust contract
        // test. It is spelled out here so that deleting a key from the artifact
        // (or from the contract test's `actual_contract` object) fails this test
        // immediately, requiring a deliberate update in both places.
        let expected_keys: std::collections::BTreeSet<&str> = [
            "accountProfileUpdated",
            "accountReportCompleted",
            "accountSavedSessionsListed",
            "activityMarkedRead",
            "activityOpened",
            "activitySnapshotLoaded",
            "cjkTextPolicyJapaneseCatalogProfileChanged",
            "e2eeTrustIdentityResetChanged",
            "e2eeTrustVerificationProgress",
            "liveSignalsPresenceSet",
            "liveSignalsRoomSignalsUpdated",
            "localEncryptionHealthChanged",
            "nativeAttentionSummaryUpdated",
            "operationFailedSessionNotFound",
            "roomDirectMessageStarted",
            "roomDirectoryQueryCompleted",
            "roomInviteAccepted",
            "roomInviteDeclined",
            "roomLeft",
            "roomMarkedAsRead",
            "roomMarkedAsUnread",
            "roomMemberModerated",
            "roomMemberRoleUpdated",
            "roomReportCompleted",
            "roomSettingUpdated",
            "roomSettingsLoaded",
            "roomTagRemoved",
            "roomTagSet",
            "searchAttachmentsFailed",
            "searchAttachmentsResults",
            "searchCrawlCompleted",
            "searchCrawlFailed",
            "searchCrawlProgress",
            "syncModeChanged",
            "threadsListOpened",
            "timelineDisplayLabelsUpdated",
            "timelineDisplayPolicyUpdated",
            "timelineInitialItems",
            "timelineItemsUpdated",
            "timelineLinkPreviewInitialItems",
            "timelineMediaDownloadCompleted",
            "timelineMediaDownloadFailed",
            "timelineMediaDownloadProgress",
            "timelineMediaInitialItems",
            "timelineMediaUploadProgress",
            "timelineMessageForwarded",
            "timelineMessageSourceLoaded",
            "timelineNavigationUpdated",
            "timelinePaginationEndReached",
            "timelineReplyQuoteInitialItems",
            "timelineResyncRequired",
            "timelineSendStateInitialItems",
        ]
        .iter()
        .copied()
        .collect();

        let artifact: serde_json::Value =
            serde_json::from_str(include_str!("../../src/domain/coreEvents.generated.json"))
                .expect("contract artifact must be valid JSON");

        let artifact_keys: std::collections::BTreeSet<&str> = artifact
            .as_object()
            .expect("contract artifact must be a JSON object")
            .keys()
            .map(String::as_str)
            .collect();

        let missing_from_artifact: Vec<&&str> = expected_keys
            .iter()
            .filter(|k| !artifact_keys.contains(*k))
            .collect();
        assert!(
            missing_from_artifact.is_empty(),
            "CoreEvent contract artifact is missing keys that were previously present: {missing_from_artifact:?}. \
            If a variant was intentionally removed, update the expected set in this test \
            AND the coreEvents.ts TypeScript types in the same PR."
        );

        let unexpected_in_artifact: Vec<&&str> = artifact_keys
            .iter()
            .filter(|k| !expected_keys.contains(*k))
            .collect();
        assert!(
            unexpected_in_artifact.is_empty(),
            "CoreEvent contract artifact contains keys not present in the expected set: {unexpected_in_artifact:?}. \
            Add the new key to the expected set in this test after adding the corresponding \
            Rust serialization entry in core_event_wire_format_matches_checked_in_contract_artifact."
        );
    }
}
