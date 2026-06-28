#![recursion_limit = "256"]

mod commands;
mod dto;
pub mod keyring_backend;

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

use crate::dto::FrontendDesktopSnapshotDelta;

// koushi-core: the production runtime host. All session, credential,
// and Matrix operations go through CoreCommand/CoreEvent — the adapter never
// touches the credential store or the SDK directly.
use koushi_core::renderable_thumbnail::{
    cleanup_legacy_plaintext_thumbnail_dirs, lookup_renderable_thumbnail,
};
use koushi_core::{
    AccountCommand, CoreCommand, CoreConnection, CoreEvent, CoreRuntime, SearchEvent,
    TimelineEvent, event::AppStateSnapshot,
};

// koushi-backend: fixture/demo preview only; never on a production
// Matrix path (overview.md: "fixture/demo data only").
use koushi_backend::{
    E2eeRecoveryMode, FakeDesktopBackend, FakeDesktopBackendConfig, LoginDiscoveryMode, LoginMode,
    SyncMode,
};

const MENU_EVENT_NAME: &str = "koushi-desktop://menu";
/// Tauri event for serialized CoreEvent payloads (discrete events + diff batches).
pub(crate) const CORE_EVENT_NAME: &str = "koushi-desktop://event";
/// Tauri event for serialized AppStateSnapshot payloads (latest-wins).
const STATE_EVENT_NAME: &str = "koushi-desktop://state";
const OIDC_CALLBACK_URL_PREFIX: &str = "koushi-desktop://auth/callback";
const MENU_ID_OPEN_USER_SETTINGS: &str = "open_user_settings";
const MENU_ID_SIGN_OUT: &str = "sign_out";
const MENU_ID_SHOW_KEYBOARD_SETTINGS: &str = "show_keyboard_settings";
const MENU_ID_TOGGLE_RIGHT_PANEL: &str = "toggle_right_panel";
const MENU_ID_TOGGLE_FULLSCREEN: &str = "toggle_fullscreen";
const MIN_RESTORABLE_WINDOW_WIDTH: u32 = 760;
const MIN_RESTORABLE_WINDOW_HEIGHT: u32 = 620;
#[cfg(any(debug_assertions, test))]
const QA_LOGIN_PIPE_ENV: &str = "KOUSHI_QA_LOGIN_PIPE";
#[cfg(any(debug_assertions, test))]
const QA_CONTROL_PIPE_ENV: &str = "KOUSHI_QA_CONTROL_PIPE";
#[cfg(any(debug_assertions, test))]
const SKIP_KEYCHAIN_PERSISTENCE_ENV: &str = "KOUSHI_SKIP_KEYCHAIN_PERSISTENCE";

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
    let mut items = vec![
        DesktopMenuItem {
            id: MENU_ID_OPEN_USER_SETTINGS,
            label: "User Settings",
            menu: "app",
            accelerator: "CmdOrCtrl+,",
        },
        DesktopMenuItem {
            id: MENU_ID_SIGN_OUT,
            label: "Sign Out",
            menu: "app",
            accelerator: "",
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
    ];

    #[cfg(target_os = "macos")]
    items.push(DesktopMenuItem {
        id: MENU_ID_TOGGLE_FULLSCREEN,
        label: "Toggle Fullscreen",
        menu: "view",
        accelerator: "Ctrl+Command+F",
    });

    items
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
        MENU_ID_SIGN_OUT => Some("logout"),
        MENU_ID_TOGGLE_RIGHT_PANEL => Some("toggleRightPanel"),
        MENU_ID_SHOW_KEYBOARD_SETTINGS => Some("showKeyboardSettings"),
        MENU_ID_TOGGLE_FULLSCREEN => Some("toggleFullscreen"),
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
fn keychain_persistence_disabled_from_env_value(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("1" | "true" | "yes")
    )
}

#[cfg(any(debug_assertions, test))]
fn keychain_persistence_disabled_from_env() -> bool {
    keychain_persistence_disabled_from_env_value(
        std::env::var(SKIP_KEYCHAIN_PERSISTENCE_ENV).ok().as_deref(),
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

/// GUI-smoke toggle: when `KOUSHI_SKIP_SAVED_SESSIONS` is set, the
/// adapter answers `list_saved_sessions` with an empty list WITHOUT routing
/// the command to core. This prevents the OS keychain read that would
/// otherwise prompt during unattended automation. Adapter-level concern: the
/// command boundary stays untouched.
pub(crate) fn saved_sessions_disabled_from_env() -> bool {
    saved_sessions_disabled_from_env_value(
        std::env::var("KOUSHI_SKIP_SAVED_SESSIONS").ok().as_deref(),
    )
}

const DATA_DIR_NAME: &str = "koushi-desktop";

pub(crate) fn app_data_dir() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("KOUSHI_DATA_DIR") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    dirs::data_local_dir()
        .map(|path| path.join(DATA_DIR_NAME))
        .ok_or_else(|| "local application data directory is unavailable".to_owned())
}

fn renderable_asset_cache_dirs(data_dir: &Path) -> [PathBuf; 1] {
    [data_dir.join("media_downloads")]
}

fn allow_runtime_asset_cache_dirs(app: &tauri::App, data_dir: &Path) {
    let asset_scope = app.asset_protocol_scope();
    for cache_dir in renderable_asset_cache_dirs(data_dir) {
        let _ = asset_scope.allow_directory(cache_dir, true);
    }
}

fn renderable_thumbnail_protocol_response(
    request: tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>> {
    let Some(content) = lookup_renderable_thumbnail(request.uri().path()) else {
        return tauri::http::Response::builder()
            .status(tauri::http::StatusCode::NOT_FOUND)
            .header(tauri::http::header::CACHE_CONTROL, "no-store")
            .header("X-Content-Type-Options", "nosniff")
            .body(Vec::new())
            .expect("thumbnail 404 response");
    };

    tauri::http::Response::builder()
        .status(tauri::http::StatusCode::OK)
        .header(
            tauri::http::header::CONTENT_TYPE,
            content
                .mime_type
                .as_deref()
                .unwrap_or("application/octet-stream"),
        )
        .header(tauri::http::header::CACHE_CONTROL, "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .body(content.bytes)
        .expect("thumbnail response")
}

fn start_core_runtime_for_tauri(data_dir: PathBuf) -> CoreRuntime {
    #[cfg(any(debug_assertions, test))]
    {
        if keychain_persistence_disabled_from_env() {
            return CoreRuntime::start_with_data_dir(data_dir.clone());
        }
    }

    CoreRuntime::start_with_data_dir_and_os_backend(
        data_dir,
        std::sync::Arc::new(crate::keyring_backend::KeyringCredentialBackend),
    )
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

fn ensure_main_window_visible<R: tauri::Runtime>(app: &mut tauri::App<R>) {
    #[cfg(target_os = "macos")]
    {
        app.set_activation_policy(tauri::ActivationPolicy::Regular);
        activate_macos_application(app.handle());
    }

    if let Some(window) = app.get_webview_window("main") {
        ensure_webview_window_visible(&window);
    }
}

#[cfg(target_os = "macos")]
fn activate_macos_application<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    let _ = app.show();
    let _ = app.run_on_main_thread(|| {
        activate_macos_application_now();
    });
}

#[cfg(target_os = "macos")]
fn activate_macos_application_now() {
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        let ns_app = objc2_app_kit::NSApplication::sharedApplication(mtm);
        #[allow(deprecated)]
        ns_app.activateIgnoringOtherApps(true);
    }
}

fn ensure_webview_window_visible<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
    #[cfg(target_os = "macos")]
    {
        if qa_window_visibility_mode_enabled() {
            let _ = window.set_visible_on_all_workspaces(true);
        }
        if let Ok(ns_window) = window.ns_window() {
            let ns_window_addr = ns_window as usize;
            let _ = window.run_on_main_thread(move || {
                order_macos_ns_window_front(ns_window_addr);
            });
        }
    }

    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

fn ensure_main_window_visible_after_page_load<R: tauri::Runtime>(window: &tauri::Window<R>) {
    #[cfg(target_os = "macos")]
    {
        if qa_window_visibility_mode_enabled() {
            let _ = window.set_visible_on_all_workspaces(true);
        }
        if let Ok(ns_window) = window.ns_window() {
            let ns_window_addr = ns_window as usize;
            let _ = window.run_on_main_thread(move || {
                order_macos_ns_window_front(ns_window_addr);
            });
        }
        let _ = window.run_on_main_thread(|| {
            activate_macos_application_now();
        });
    }

    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

#[cfg(target_os = "macos")]
fn order_macos_ns_window_front(ns_window_addr: usize) {
    let ns_window = ns_window_addr as *mut objc2_app_kit::NSWindow;
    // The pointer comes from Tauri's `ns_window()` for the live main window.
    // Ordering must run on the main thread; callers enforce that with
    // `run_on_main_thread`.
    if let Some(ns_window) = unsafe { ns_window.as_ref() } {
        ns_window.makeKeyAndOrderFront(None);
        ns_window.orderFrontRegardless();
    }
}

#[cfg(target_os = "macos")]
fn qa_window_visibility_mode_enabled() -> bool {
    matches!(std::env::var("KOUSHI_QA_TITLE").ok().as_deref(), Some("1"))
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
    let sign_out = menu_item(manager, MENU_ID_SIGN_OUT)?;
    let toggle_right_panel = menu_item(manager, MENU_ID_TOGGLE_RIGHT_PANEL)?;
    let show_keyboard_settings = menu_item(manager, MENU_ID_SHOW_KEYBOARD_SETTINGS)?;

    #[cfg(target_os = "macos")]
    let toggle_fullscreen = menu_item(manager, MENU_ID_TOGGLE_FULLSCREEN)?;

    let app_menu = SubmenuBuilder::new(manager, "Koushi")
        .item(&open_user_settings)
        .item(&sign_out)
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
    let view_menu = {
        let builder = SubmenuBuilder::new(manager, "View").item(&toggle_right_panel);
        #[cfg(target_os = "macos")]
        let builder = builder.item(&toggle_fullscreen);
        builder.build()?
    };
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

#[cfg(target_os = "macos")]
fn toggle_main_window_fullscreen(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if let Ok(fullscreen) = window.is_fullscreen() {
            let _ = window.set_fullscreen(!fullscreen);
        }
    }
}

fn menu_item<R: tauri::Runtime, M: Manager<R>>(
    manager: &M,
    id: &str,
) -> tauri::Result<tauri::menu::MenuItem<R>> {
    let item = desktop_menu_items()
        .into_iter()
        .find(|item| item.id == id)
        .expect("desktop menu item id should be registered");
    let builder = MenuItemBuilder::with_id(item.id, item.label);
    if item.accelerator.is_empty() {
        builder.build(manager)
    } else {
        builder.accelerator(item.accelerator).build(manager)
    }
}

/// Spawn the CoreEvent forwarding task. This task owns a dedicated connection
/// (second `attach()`) so it can loop on `recv_event` without blocking command
/// dispatch.
///
/// On `CoreEvent::StateChanged`: emit `koushi-desktop://state` with the
/// serialized snapshot + update QA window title.
/// On any `CoreEvent`: emit `koushi-desktop://event` with a serialized DTO.
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

    if let CoreEvent::StateDelta(delta) = event {
        forwarded.push(ForwardedWebviewEvent {
            event_name: CORE_EVENT_NAME,
            payload: serde_json::json!({
                "kind": "StateDelta",
                "generation": delta.generation,
                "changed": FrontendDesktopSnapshotDelta::from(delta.clone()).changed,
            }),
        });
    }

    if let Some(payload) = serialize_core_event(event) {
        forwarded.push(ForwardedWebviewEvent {
            event_name: CORE_EVENT_NAME,
            payload,
        });
    }

    forwarded
}

fn diffs_net_count_change(diffs: &[koushi_core::TimelineDiff]) -> i64 {
    diffs
        .iter()
        .map(|diff| match diff {
            koushi_core::TimelineDiff::PushFront { .. }
            | koushi_core::TimelineDiff::PushBack { .. }
            | koushi_core::TimelineDiff::Insert { .. } => 1_i64,
            koushi_core::TimelineDiff::Remove { .. } => -1_i64,
            koushi_core::TimelineDiff::Truncate { .. }
            | koushi_core::TimelineDiff::Clear
            | koushi_core::TimelineDiff::Reset { .. }
            | koushi_core::TimelineDiff::Set { .. } => 0_i64,
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

fn is_oidc_callback_url(url: &str) -> bool {
    match url.strip_prefix(OIDC_CALLBACK_URL_PREFIX) {
        Some("") => true,
        Some(rest) => rest.starts_with('?') || rest.starts_with('#'),
        None => false,
    }
}

fn submit_oidc_callback_url(app: tauri::AppHandle, callback_url: String) {
    if !is_oidc_callback_url(&callback_url) {
        return;
    }

    tauri::async_runtime::spawn(async move {
        let core_state = app.state::<CoreRuntimeState>();
        let event_conn = core_state.runtime.attach();
        let request_id = event_conn.next_request_id();
        let _ = event_conn
            .command(commands::build_complete_oidc_login_command(
                request_id,
                callback_url,
            ))
            .await;
    });
}

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
fn install_oidc_deep_link_handler(app: &tauri::App) -> tauri::Result<()> {
    use tauri_plugin_deep_link::DeepLinkExt;

    if let Ok(Some(urls)) = app.deep_link().get_current() {
        let app_handle = app.handle().clone();
        for url in urls {
            submit_oidc_callback_url(app_handle.clone(), url.to_string());
        }
    }

    let app_handle = app.handle().clone();
    app.deep_link().on_open_url(move |event| {
        for url in event.urls() {
            submit_oidc_callback_url(app_handle.clone(), url.to_string());
        }
    });

    #[cfg(any(target_os = "linux", all(debug_assertions, windows)))]
    let _ = app.deep_link().register_all();

    Ok(())
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
fn install_oidc_deep_link_handler(_app: &tauri::App) -> tauri::Result<()> {
    Ok(())
}

/// Serialize a `CoreEvent` to a JSON value for IPC.
///
/// Security: message bodies flow in `Timeline` events. These are visible
/// content (not secret), but we never trace IPC payloads in release.
/// The serialization produces structured JSON only — no raw SDK errors.
fn serialize_core_event(event: &CoreEvent) -> Option<serde_json::Value> {
    Some(match event {
        CoreEvent::StateDelta(_) => {
            return None;
        }
        CoreEvent::StateChanged(_) => {
            // StateChanged snapshots are sent via `koushi-desktop://state`;
            // don't duplicate as a generic event.
            return None;
        }
        CoreEvent::Account(e) => serde_json::json!({ "kind": "Account", "event": e }),
        CoreEvent::Sync(e) => serde_json::json!({ "kind": "Sync", "event": e }),
        CoreEvent::Room(e) => serde_json::json!({ "kind": "Room", "event": e }),
        CoreEvent::Timeline(e) => serde_json::json!({ "kind": "Timeline", "event": e }),
        CoreEvent::LiveSignals(e) => serde_json::json!({ "kind": "LiveSignals", "event": e }),
        CoreEvent::Search(SearchEvent::IndexUpdated { .. }) => {
            // Internal indexer wake-up signal. Forwarding one WebView IPC event
            // per indexed message competes with input and scroll rendering.
            return None;
        }
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
        // Telemetry-lane event: emitted after reduce, never mixed with
        // StateDelta/StateChanged, never drives product state in React.
        CoreEvent::IntentLifecycle {
            request_id,
            outcome,
        } => {
            serde_json::json!({
                "kind": "IntentLifecycle",
                "request_id": request_id,
                "outcome": outcome,
            })
        }
    })
}

pub fn run() {
    let restore_session = restore_session_enabled_from_env_value(
        std::env::var("KOUSHI_RESTORE_SESSION").ok().as_deref(),
    );

    let mut builder = tauri::Builder::default();

    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|_app, _argv, _cwd| {
            // The deep-link plugin consumes configured callback URLs and emits
            // them through `on_open_url`; keep this callback side-effect-free so
            // it never logs authorization callback query strings.
        }));
    }

    builder
        .plugin(tauri_plugin_deep_link::init())
        .register_uri_scheme_protocol("koushi-thumbnail", move |_, request| {
            renderable_thumbnail_protocol_response(request)
        })
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(move |app| {
            // Build the CoreRuntime inside setup() so Tauri's async runtime is
            // already active. `CoreRuntime::start_with_data_dir` calls
            // `executor::spawn` which requires a Tokio runtime context. Tauri
            // starts its tokio runtime before invoking setup; we enter the
            // handle so `tokio::task::spawn` can find it from the main thread.
            let data_dir = app_data_dir().unwrap_or_else(|_| PathBuf::from("koushi-desktop-data"));
            let _ = cleanup_legacy_plaintext_thumbnail_dirs(&data_dir);
            allow_runtime_asset_cache_dirs(app, &data_dir);
            // Enter Tauri's tokio runtime so `executor::spawn` (tokio::task::spawn)
            // can find a runtime handle from this non-tokio-worker thread.
            let async_handle = tauri::async_runtime::handle();
            let _guard = async_handle.inner().enter();
            let runtime = start_core_runtime_for_tauri(data_dir);

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
            install_oidc_deep_link_handler(app)?;

            let menu = build_desktop_menu(app)?;
            app.set_menu(menu)?;
            let _ = restore_main_window_state(app);
            ensure_main_window_visible(app);
            app.on_menu_event(|app, event| {
                #[cfg(target_os = "macos")]
                if event.id().as_ref() == MENU_ID_TOGGLE_FULLSCREEN {
                    toggle_main_window_fullscreen(app);
                    return;
                }
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
        .on_page_load(|webview, _payload| {
            if webview.label() == "main" {
                let window = webview.window();
                ensure_main_window_visible_after_page_load(&window);
            }
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
            commands::session::get_snapshot,
            commands::session::discover_login_methods,
            commands::session::start_oidc_login,
            commands::session::complete_oidc_login,
            commands::session::submit_login,
            commands::session::list_saved_sessions,
            commands::session::switch_account,
            commands::session::submit_recovery,
            commands::session::logout,
            commands::session::restart_sync,
            commands::settings::update_settings,
            commands::settings::rebuild_search_index,
            commands::settings::set_room_url_preview_override,
            commands::room::select_room_list_filter,
            commands::room::mark_room_as_read,
            commands::room::mark_room_as_unread,
            commands::room::set_room_notification_mode,
            commands::account::query_devices,
            commands::account::load_account_management_capabilities,
            commands::account::rename_device,
            commands::account::delete_devices,
            commands::account::change_password,
            commands::account::deactivate_account,
            commands::account::submit_account_management_uia,
            commands::local_encryption::probe_local_encryption_health,
            commands::local_encryption::reset_local_data,
            commands::e2ee::bootstrap_cross_signing,
            commands::e2ee::enable_key_backup,
            commands::e2ee::bootstrap_secure_backup,
            commands::e2ee::change_secure_backup_passphrase,
            commands::e2ee::export_room_keys,
            commands::e2ee::import_room_keys,
            commands::e2ee::accept_verification,
            commands::e2ee::confirm_sas_verification,
            commands::e2ee::cancel_verification,
            commands::e2ee::reset_identity,
            commands::e2ee::submit_identity_reset_password,
            commands::e2ee::submit_identity_reset_oauth,
            commands::timeline::resolve_composer_key_action,
            commands::navigation::select_space,
            commands::navigation::reorder_spaces,
            commands::navigation::select_room,
            commands::navigation::open_activity_event,
            commands::navigation::select_search_result,
            commands::navigation::close_focused_context,
            commands::navigation::open_timeline_at_timestamp,
            commands::navigation::update_navigation_scroll_anchor,
            commands::navigation::observe_timeline_viewport,
            commands::timeline::ensure_timeline_subscribed,
            commands::timeline::paginate_timeline_backwards,
            commands::timeline::materialize_timeline_anchor,
            commands::timeline::paginate_thread_timeline_backwards,
            commands::timeline::send_text,
            commands::timeline::schedule_send,
            commands::timeline::stage_uploads,
            commands::timeline::update_staged_upload_caption,
            commands::timeline::update_staged_upload_compression,
            commands::timeline::clear_upload_staging,
            commands::timeline::cancel_scheduled_send,
            commands::timeline::reschedule_scheduled_send,
            commands::timeline::retry_send,
            commands::timeline::cancel_send,
            commands::timeline::upload_media,
            commands::timeline::download_media,
            commands::timeline::load_message_source,
            commands::timeline::request_room_key,
            commands::timeline::load_link_previews,
            commands::timeline::hide_link_preview,
            commands::timeline::forward_message,
            commands::timeline::edit_message,
            commands::timeline::redact_message,
            commands::live_signals::send_read_receipt,
            commands::live_signals::set_fully_read,
            commands::live_signals::set_typing,
            commands::live_signals::set_presence,
            commands::profile::set_display_name,
            commands::profile::set_local_user_alias,
            commands::profile::ignore_user,
            commands::profile::unignore_user,
            commands::profile::report_user,
            commands::profile::report_content,
            commands::profile::report_room,
            commands::profile::set_avatar,
            commands::profile::download_avatar_thumbnail,
            commands::room::leave_room,
            commands::room::forget_room,
            commands::room::set_room_tag,
            commands::room::remove_room_tag,
            commands::room::pin_event,
            commands::room::unpin_event,
            commands::room::load_room_settings,
            commands::room::reshare_room_key,
            commands::room::update_room_setting,
            commands::room::moderate_room_member,
            commands::room::update_room_member_role,
            commands::activity::open_activity,
            commands::activity::close_activity,
            commands::activity::set_activity_tab,
            commands::activity::paginate_activity,
            commands::activity::mark_activity_read,
            commands::views::open_files_view,
            commands::views::close_files_view,
            commands::views::open_threads_list,
            commands::views::close_threads_list,
            commands::views::paginate_threads_list,
            commands::views::open_thread,
            commands::views::close_thread,
            commands::search::submit_search,
            commands::search::start_room_crawl,
            commands::search::stop_room_crawl,
            commands::directory::query_directory,
            commands::room::create_room,
            commands::room::create_space,
            commands::directory::join_directory_room,
            commands::room::set_space_child,
            commands::room::accept_invite,
            commands::room::decline_invite,
            commands::room::start_direct_message,
            commands::room::invite_user,
            commands::timeline::set_composer_reply_target,
            commands::timeline::cancel_composer_reply,
            commands::timeline::set_composer_draft,
            commands::timeline::set_thread_composer_draft,
            commands::timeline::toggle_reaction,
            commands::timeline::send_reaction,
            commands::timeline::redact_reaction,
            commands::timeline::send_reply,
            commands::timeline::send_thread_reply,
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
    fn keychain_persistence_env_value_can_disable_os_keychain_for_gui_smoke() {
        assert!(super::keychain_persistence_disabled_from_env_value(Some(
            "1"
        )));
        assert!(super::keychain_persistence_disabled_from_env_value(Some(
            "true"
        )));
        assert!(super::keychain_persistence_disabled_from_env_value(Some(
            "yes"
        )));
        assert!(!super::keychain_persistence_disabled_from_env_value(None));
        assert!(!super::keychain_persistence_disabled_from_env_value(Some(
            "0"
        )));
    }

    #[test]
    fn renderable_asset_cache_scope_is_limited_to_media_cache_dirs() {
        let base = Path::new("/tmp/koushi-data");
        assert_eq!(
            super::renderable_asset_cache_dirs(base),
            [base.join("media_downloads")]
        );
    }

    #[test]
    fn renderable_thumbnail_protocol_serves_known_cached_bytes() {
        let ready = koushi_core::renderable_thumbnail::store_renderable_thumbnail(
            koushi_core::renderable_thumbnail::RenderableThumbnailKind::Avatar,
            "mxc://example.test/avatar",
            b"protocol-bytes".to_vec(),
        );
        let source_url = match ready {
            koushi_state::AvatarThumbnailState::Ready { source_url, .. } => source_url,
            other => panic!("unexpected thumbnail state: {other:?}"),
        };
        let response = super::renderable_thumbnail_protocol_response(
            tauri::http::Request::builder()
                .uri(source_url)
                .body(Vec::new())
                .expect("request"),
        );
        assert_eq!(response.status(), tauri::http::StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(tauri::http::header::CACHE_CONTROL)
                .and_then(|value| value.to_str().ok()),
            Some("no-store")
        );
        assert_eq!(
            response
                .headers()
                .get(tauri::http::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("application/octet-stream")
        );
        assert_eq!(
            response
                .headers()
                .get("X-Content-Type-Options")
                .and_then(|value| value.to_str().ok()),
            Some("nosniff")
        );
        assert_eq!(response.body(), &b"protocol-bytes".to_vec());
    }

    #[test]
    fn renderable_thumbnail_protocol_rejects_unknown_refs() {
        let response = super::renderable_thumbnail_protocol_response(
            tauri::http::Request::builder()
                .uri("koushi-thumbnail://localhost/avatar/unknown")
                .body(Vec::new())
                .expect("request"),
        );
        assert_eq!(response.status(), tauri::http::StatusCode::NOT_FOUND);
        assert_eq!(
            response
                .headers()
                .get("X-Content-Type-Options")
                .and_then(|value| value.to_str().ok()),
            Some("nosniff")
        );
    }

    #[test]
    fn qa_login_pipe_env_uses_path_only() {
        assert_eq!(
            qa_login_pipe_path_from_env_value(Some(" /tmp/koushi-desktop-login.pipe ")),
            Some(Path::new("/tmp/koushi-desktop-login.pipe").to_path_buf())
        );
        assert_eq!(qa_login_pipe_path_from_env_value(Some("   ")), None);
        assert_eq!(qa_login_pipe_path_from_env_value(None), None);
    }

    #[test]
    fn qa_control_pipe_env_uses_path_only() {
        assert_eq!(
            qa_control_pipe_path_from_env_value(Some(" /tmp/koushi-desktop-control.pipe ")),
            Some(Path::new("/tmp/koushi-desktop-control.pipe").to_path_buf())
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
            r#"{"homeserver":"https://matrix.example.org","username":"fixture-user","password":"synthetic-password","device_display_name":"Koushi GUI Smoke","recovery_secret":"synthetic-recovery-secret"}"#,
        )
        .expect("payload should parse");

        assert_eq!(request.login.homeserver, "https://matrix.example.org");
        assert_eq!(request.login.username, "fixture-user");
        assert_eq!(request.login.password.expose_secret(), "synthetic-password");
        assert_eq!(
            request.login.device_display_name.as_deref(),
            Some("Koushi GUI Smoke")
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
        use koushi_core::{
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
    fn legacy_state_changed_forwarding_is_not_the_webview_state_path() {
        use koushi_core::CoreEvent;
        use koushi_state::AppState;

        let timeline_items_count = AtomicUsize::new(17);
        let event = CoreEvent::StateChanged(AppState::default());

        let forwarded = forwarded_webview_events_for_core_event(&event, &timeline_items_count);

        assert_eq!(timeline_items_count.load(Ordering::Relaxed), 17);
        assert!(
            forwarded.is_empty(),
            "legacy full StateChanged events must not drive the normal webview state path"
        );
    }

    #[test]
    fn state_delta_forwarding_emits_core_event_changed_slices() {
        use koushi_core::{CoreEvent, build_state_delta};
        use koushi_state::{AppState, SearchCrawlerRoomState};
        use serde_json::json;

        let timeline_items_count = AtomicUsize::new(17);
        let previous = AppState::default();
        let mut next = previous.clone();
        next.navigation.active_room_id = Some("!selected:example.invalid".to_owned());
        next.navigation.active_space_id = Some("!space:example.invalid".to_owned());
        next.search_crawler.rooms.insert(
            "!crawler:example.invalid".to_owned(),
            SearchCrawlerRoomState::Queued,
        );
        let delta = build_state_delta(1, &previous, &next).expect("delta");
        let forwarded = forwarded_webview_events_for_core_event(
            &CoreEvent::StateDelta(delta),
            &timeline_items_count,
        );

        assert_eq!(timeline_items_count.load(Ordering::Relaxed), 17);
        assert_eq!(forwarded.len(), 1);
        assert_eq!(forwarded[0].event_name, CORE_EVENT_NAME);
        assert_eq!(forwarded[0].payload["kind"], json!("StateDelta"));
        assert_eq!(forwarded[0].payload["generation"], json!(1));
        assert_eq!(
            forwarded[0].payload["changed"]["state"]["domain"]["search_crawler"]["rooms"]["!crawler:example.invalid"]
                ["kind"],
            json!("queued")
        );
        assert_eq!(
            forwarded[0].payload["changed"]["state"]["ui"]["navigation"]["active_room_id"],
            json!("!selected:example.invalid")
        );
        assert_eq!(
            forwarded[0].payload["changed"]["state"]["ui"]["navigation"]["active_space_id"],
            json!("!space:example.invalid")
        );
    }

    #[test]
    fn lag_resync_forwarding_emits_state_then_resync_marker() {
        use koushi_state::AppState;
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
        let path = window_state_path(Path::new("/tmp/koushi-desktop"));

        assert_eq!(
            path,
            Path::new("/tmp/koushi-desktop")
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
    fn oidc_callback_url_accepts_only_expected_auth_callback_shape() {
        assert!(super::is_oidc_callback_url("koushi-desktop://auth/callback"));
        assert!(super::is_oidc_callback_url(
            "koushi-desktop://auth/callback?code=synthetic&state=synthetic"
        ));
        assert!(!super::is_oidc_callback_url("koushi-desktop://event"));
        assert!(!super::is_oidc_callback_url(
            "koushi-desktop://auth/callback-extra?code=synthetic"
        ));
        assert!(!super::is_oidc_callback_url(
            "https://auth.example.test/callback?code=synthetic"
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
        assert!(
            items
                .iter()
                .any(|item| item.id == "sign_out" && item.accelerator == "" && item.menu == "app")
        );
        let user_settings_index = items
            .iter()
            .position(|item| item.id == "open_user_settings")
            .expect("user settings menu item should exist");
        let sign_out_index = items
            .iter()
            .position(|item| item.id == "sign_out")
            .expect("sign out menu item should exist");
        assert_eq!(sign_out_index, user_settings_index + 1);
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

        #[cfg(target_os = "macos")]
        assert!(items.iter().any(|item| {
            item.id == "toggle_fullscreen"
                && item.accelerator == "Ctrl+Command+F"
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
        use koushi_core::{
            AccountKey, CoreEvent, TimelineDiff, TimelineKey, build_state_delta,
            event::{
                AccountEvent, ActivityEvent, CjkTextPolicyEvent, E2eeTrustEvent,
                EventCacheFailureReasonClass, EventCacheSubscribeStatus, IntentNoOpReason,
                IntentOutcome, LinkPreview, LinkPreviewImage, LinkPreviewState, LiveSignalsEvent,
                LocalEncryptionEvent, NativeAttentionEvent, PaginationDirection, PaginationState,
                ReactionGroup, RoomEvent, SearchEvent, SyncEvent, ThreadsListEvent,
                TimelineAnchorMaterializeStatus, TimelineCodeBlock, TimelineDisplayLabelUpdate,
                TimelineEvent, TimelineFormattedBody, TimelineItem, TimelineItemId, TimelineMedia,
                TimelineMediaKind, TimelineMediaSource, TimelineMediaThumbnail,
                TimelineMessageActions, TimelineMessageKind, TimelineMessageSource,
                TimelineNavigationSnapshot, TimelineResyncReason, TimelineSendFailureReason,
                TimelineSendState, TimelineSpoilerSpan, TimelineUnreadPosition,
            },
            failure::{CoreFailure, TimelineFailureKind},
            ids::{RequestId, RuntimeConnectionId, TimelineBatchId, TimelineGeneration},
        };
        use koushi_state::{
            ActivityRow, ActivityStream, ActivityTab, AppState, AttachmentKind, AttachmentResult,
            AvatarThumbnailState, DirectoryQuery, DirectoryRoomSummary, IdentityResetAuthType,
            IdentityResetState, JapaneseCatalogProfile, LocalEncryptionHealth,
            MediaTransferProgress,
            NativeAttentionCapabilities, NativeAttentionCapability, NativeAttentionSummary,
            PresenceKind, ReplyQuote, ReplyQuoteState, RoomHistoryVisibility, RoomJoinRule,
            RoomMemberRole, RoomModerationAction, RoomPermissionFacts, RoomSettingsSnapshot,
            RoomTagKind, SasEmoji, SearchCrawlerFailureKind, SearchCrawlerRoomState, SyncMode,
            UserTrustState, VerificationFlowState, VerificationTarget,
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
            sender_avatar: None,
            body: Some("hello".to_owned()),
            notice_i18n_key: None,
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
            link_ranges: Vec::new(),
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
            unable_to_decrypt: None,
        };
        let media_item = TimelineItem {
            id: TimelineItemId::Event {
                event_id: "$media1".to_owned(),
            },
            sender: Some("@u:example.test".to_owned()),
            sender_label: None,
            sender_avatar: None,
            body: Some("caption".to_owned()),
            notice_i18n_key: None,
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
            link_ranges: Vec::new(),
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
            unable_to_decrypt: None,
        };
        let send_state_item = TimelineItem {
            id: TimelineItemId::Transaction {
                transaction_id: "txn-not-sent".to_owned(),
            },
            sender: Some("@u:example.test".to_owned()),
            sender_label: None,
            sender_avatar: None,
            body: Some("queued".to_owned()),
            notice_i18n_key: None,
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
            link_ranges: Vec::new(),
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
            unable_to_decrypt: None,
        };
        let reply_quote_item = TimelineItem {
            id: TimelineItemId::Event {
                event_id: "$reply1".to_owned(),
            },
            sender: Some("@u:example.test".to_owned()),
            sender_label: None,
            sender_avatar: None,
            body: Some("reply body".to_owned()),
            notice_i18n_key: None,
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
            link_ranges: Vec::new(),
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
            unable_to_decrypt: None,
        };
        let link_preview_item = TimelineItem {
            id: TimelineItemId::Event {
                event_id: "$linkpreview1".to_owned(),
            },
            sender: Some("@u:example.test".to_owned()),
            sender_label: None,
            sender_avatar: None,
            body: Some("Check out https://example.invalid/page".to_owned()),
            notice_i18n_key: None,
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
                        source_url: "koushi-thumbnail://localhost/link-preview/fixture.bin"
                            .to_owned(),
                        width: Some(600),
                        height: Some(315),
                        mime_type: Some("image/png".to_owned()),
                    },
                }),
                state: LinkPreviewState::Ready,
            }]),
            link_ranges: Vec::new(),
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
            unable_to_decrypt: None,
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
                            "source_url": "koushi-thumbnail://localhost/link-preview/fixture.bin",
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

        let media_download_progress =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::MediaDownloadProgress {
                request_id,
                key: key.clone(),
                event_id: "$media1".to_owned(),
                progress: MediaTransferProgress {
                    current: 0,
                    total: 68,
                },
            }))
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

        let media_download_failed =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::MediaDownloadFailed {
                request_id,
                key: key.clone(),
                event_id: "$media1".to_owned(),
                kind: TimelineFailureKind::Sdk,
            }))
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
                    original_json: None,
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

        let anchor_materialize_finished =
            serialize_core_event(&CoreEvent::Timeline(TimelineEvent::AnchorMaterializeFinished {
                request_id,
                key: key.clone(),
                status: TimelineAnchorMaterializeStatus::BudgetExhausted,
            }))
            .expect("serialize anchor materialize finished");
        assert_eq!(
            anchor_materialize_finished["event"]["AnchorMaterializeFinished"]["status"],
            json!("BudgetExhausted")
        );

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
                    read_marker_display_event_id: Some("$read:example.test".to_owned()),
                    latest_readable_event_id: Some("$latest:example.test".to_owned()),
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
            sessions: vec![koushi_state::SessionInfo {
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
                kind: koushi_core::event::ReportKind::User,
            }))
            .expect("serialize account report completed event");
        let account_oidc_authorization_created =
            serialize_core_event(&CoreEvent::Account(AccountEvent::OidcAuthorizationCreated {
                request_id,
                authorization_url: "https://auth.example.test/authorize".to_owned(),
                state: "synthetic-state".to_owned(),
            }))
            .expect("serialize OIDC authorization event");

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
                kind: koushi_core::event::ReportKind::Event,
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
            members: vec![koushi_state::RoomMemberSummary {
                user_id: "@member:example.test".to_owned(),
                display_name: Some("Synthetic Member".to_owned()),
                display_label: "Synthetic Member".to_owned(),
                original_display_label: "Synthetic Member".to_owned(),
                avatar_url: Some("mxc://example.test/member-avatar".to_owned()),
                power_level: Some(50),
                role: RoomMemberRole::Moderator,
                user_trust: Some(UserTrustState::Verified),
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
                        kind: koushi_state::ActivityRowKind::Event,
                        room_id: "!activity-recent:example.test".to_owned(),
                        event_id: Some("$activity-recent:example.test".to_owned()),
                        room_label: "Recent room".to_owned(),
                        sender_label: Some("Recent sender".to_owned()),
                        preview: Some("Recent preview".to_owned()),
                        timestamp_ms: 20,
                        unread: false,
                        highlight: false,
                        ..Default::default()
                    }],
                    next_batch: Some("recent-next".to_owned()),
                },
                unread: ActivityStream {
                    rows: vec![
                        ActivityRow {
                            kind: koushi_state::ActivityRowKind::Event,
                            room_id: "!activity-unread:example.test".to_owned(),
                            event_id: Some("$activity-unread:example.test".to_owned()),
                            room_label: "Unread room".to_owned(),
                            sender_label: Some("Unread sender".to_owned()),
                            preview: Some("Unread preview".to_owned()),
                            timestamp_ms: 10,
                            unread: true,
                            highlight: true,
                            ..Default::default()
                        },
                        ActivityRow::room_unread_placeholder(
                            "!activity-placeholder:example.test".to_owned(),
                            "Placeholder room".to_owned(),
                            9,
                            false,
                        ),
                    ],
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
        assert_eq!(
            activity_snapshot_loaded["event"]["SnapshotLoaded"]["unread"]["rows"][1]["kind"],
            json!("roomUnread")
        );
        assert_eq!(
            activity_snapshot_loaded["event"]["SnapshotLoaded"]["unread"]["rows"][1]["event_id"],
            serde_json::Value::Null
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

        let local_encryption_event_cache_enabled = serialize_core_event(
            &CoreEvent::LocalEncryption(LocalEncryptionEvent::EventCacheStatus {
                encrypted_store: true,
                subscribed: true,
                subscribe_status: EventCacheSubscribeStatus::AlreadyEnabled,
                reason_class: None,
            }),
        )
        .expect("serialize enabled local encryption event cache status");
        assert_eq!(
            local_encryption_event_cache_enabled["event"]["kind"],
            json!("eventCacheStatus")
        );
        assert_eq!(
            local_encryption_event_cache_enabled["event"]["encrypted_store"],
            json!(true)
        );
        assert_eq!(
            local_encryption_event_cache_enabled["event"]["subscribed"],
            json!(true)
        );
        assert_eq!(
            local_encryption_event_cache_enabled["event"]["subscribe_status"],
            json!("already_enabled")
        );
        assert!(
            local_encryption_event_cache_enabled["event"]
                .get("reason_class")
                .is_none(),
            "success diagnostics should omit the optional failure reason"
        );

        let local_encryption_event_cache_failed = serialize_core_event(
            &CoreEvent::LocalEncryption(LocalEncryptionEvent::EventCacheStatus {
                encrypted_store: true,
                subscribed: false,
                subscribe_status: EventCacheSubscribeStatus::SubscribeFailed,
                reason_class: Some(EventCacheFailureReasonClass::SubscribeFailed),
            }),
        )
        .expect("serialize failed local encryption event cache status");
        assert_eq!(
            local_encryption_event_cache_failed["event"]["subscribe_status"],
            json!("subscribe_failed")
        );
        assert_eq!(
            local_encryption_event_cache_failed["event"]["reason_class"],
            json!("subscribe_failed")
        );

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

        assert!(
            serialize_core_event(&CoreEvent::Search(SearchEvent::IndexUpdated {
                room_id: "!r:example.test".to_owned(),
                event_id: "$indexed:example.test".to_owned(),
            }))
            .is_none(),
            "per-message index updates are internal and must not cross WebView IPC"
        );

        // Search history crawler contract events (#77).
        let search_crawl_progress =
            serialize_core_event(&CoreEvent::Search(SearchEvent::HistoryCrawlProgress {
                room_id: "!r:example.test".to_owned(),
                processed: 100,
                indexed: 42,
            }))
            .expect("serialize history crawl progress event");
        assert_eq!(
            search_crawl_progress["event"]["HistoryCrawlProgress"]["processed"],
            json!(100u64)
        );

        let search_crawl_completed =
            serialize_core_event(&CoreEvent::Search(SearchEvent::HistoryCrawlCompleted {
                room_id: "!r:example.test".to_owned(),
                indexed: 42,
            }))
            .expect("serialize history crawl completed event");
        assert_eq!(
            search_crawl_completed["event"]["HistoryCrawlCompleted"]["indexed"],
            json!(42u64)
        );

        let search_crawl_failed =
            serialize_core_event(&CoreEvent::Search(SearchEvent::HistoryCrawlFailed {
                room_id: "!r:example.test".to_owned(),
                kind: SearchCrawlerFailureKind::Sdk,
            }))
            .expect("serialize history crawl failed event");
        assert_eq!(
            search_crawl_failed["event"]["HistoryCrawlFailed"]["failureKind"],
            json!("sdk")
        );
        // Privacy assertion: no raw error text in the failed event.
        assert!(
            !serde_json::to_string(&search_crawl_failed)
                .unwrap()
                .contains("message"),
            "crawl failure must not carry a raw message field"
        );

        let state_delta_previous = AppState::default();
        let mut state_delta_next = state_delta_previous.clone();
        state_delta_next.search_crawler.rooms.insert(
            "!crawler:example.test".to_owned(),
            SearchCrawlerRoomState::Queued,
        );
        let state_delta_event = CoreEvent::StateDelta(
            build_state_delta(1, &state_delta_previous, &state_delta_next).expect("fixture delta"),
        );
        let state_delta =
            forwarded_webview_events_for_core_event(&state_delta_event, &AtomicUsize::new(0))
                .into_iter()
                .next()
                .expect("state delta should be forwarded")
                .payload;

        let actual_contract = json!({
            "activityOpened": activity_opened,
            "activityMarkedRead": activity_marked_read,
            "activitySnapshotLoaded": activity_snapshot_loaded,
            "cjkTextPolicyJapaneseCatalogProfileChanged": cjk_text_policy,
            "e2eeTrustIdentityResetChanged": e2ee_identity_reset,
            "accountProfileUpdated": profile_updated,
            "accountOidcAuthorizationCreated": account_oidc_authorization_created,
            "accountReportCompleted": account_report_completed,
            "accountSavedSessionsListed": listed,
            "e2eeTrustVerificationProgress": e2ee_trust,
            "localEncryptionHealthChanged": local_encryption,
            "localEncryptionEventCacheStatus": local_encryption_event_cache_failed,
            "liveSignalsPresenceSet": live_presence,
            "nativeAttentionSummaryUpdated": native_attention,
            "operationFailedSessionNotFound": failed,
            "searchAttachmentsFailed": search_attachments_failed,
            "searchAttachmentsResults": search_attachments_results,
            "searchCrawlProgress": search_crawl_progress,
            "searchCrawlCompleted": search_crawl_completed,
            "searchCrawlFailed": search_crawl_failed,
            "stateDeltaSearchCrawlerQueued": state_delta,
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
            "timelineAnchorMaterializeFinished": anchor_materialize_finished,
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
            "intentLifecycleCommitted": serialize_core_event(&CoreEvent::IntentLifecycle {
                request_id,
                outcome: IntentOutcome::Committed,
            })
            .expect("serialize intent lifecycle committed"),
            "intentLifecycleFailedNoOpRoomNotInState": serialize_core_event(
                &CoreEvent::IntentLifecycle {
                    request_id,
                    outcome: IntentOutcome::FailedNoOp(IntentNoOpReason::RoomNotInState),
                },
            )
            .expect("serialize intent lifecycle failed noop room not in state"),
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
            "accountOidcAuthorizationCreated",
            "accountReportCompleted",
            "accountSavedSessionsListed",
            "activityMarkedRead",
            "activityOpened",
            "activitySnapshotLoaded",
            "cjkTextPolicyJapaneseCatalogProfileChanged",
            "e2eeTrustIdentityResetChanged",
            "e2eeTrustVerificationProgress",
            "intentLifecycleCommitted",
            "intentLifecycleFailedNoOpRoomNotInState",
            "liveSignalsPresenceSet",
            "localEncryptionHealthChanged",
            "localEncryptionEventCacheStatus",
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
            "stateDeltaSearchCrawlerQueued",
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
            "timelineAnchorMaterializeFinished",
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
