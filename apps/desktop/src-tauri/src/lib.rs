#![recursion_limit = "256"]

mod commands;
mod core_event_forwarder;
mod desktop_menu;
mod dto;
pub mod keyring_backend;
mod media_save;
mod oidc_browser;
mod tray;
mod viewport_sync;
mod window_state;

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering},
    },
};
use tokio::sync::Mutex as TokioMutex;

use tauri::{Emitter, Manager};

pub(crate) use crate::core_event_forwarder::CORE_EVENT_NAME;
use crate::core_event_forwarder::{CoreEventForwarderTask, spawn_core_event_forwarder};
use crate::desktop_menu::{MENU_EVENT_NAME, build_desktop_menu, desktop_menu_action_id};
#[cfg(target_os = "macos")]
use crate::desktop_menu::{MENU_ID_TOGGLE_FULLSCREEN, toggle_main_window_fullscreen};
#[cfg(test)]
use crate::desktop_menu::{desktop_menu_items, desktop_standard_menu_items};
use crate::window_state::{
    WindowCloseEvent, WindowStatePersistenceGate, persist_close_window_state_if_ready,
    persist_observed_window_geometry, restore_main_window_state, window_event_is_geometry,
    window_event_should_persist,
};

// koushi-core: the production runtime host. All session, credential,
// and Matrix operations go through CoreCommand/CoreEvent — the adapter never
// touches the credential store or the SDK directly.
use koushi_core::renderable_thumbnail::{
    cleanup_legacy_plaintext_thumbnail_dirs, lookup_renderable_thumbnail,
};
use koushi_core::{CoreConnection, CoreRuntime, NativeArtifactRegistry};
use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel};
use koushi_protocol::{AccountCommand, AppCommand, CoreCommand};

// Must stay in sync with `OIDC_REDIRECT_URI` in koushi-core. The scheme is
// reverse-DNS per RFC 8252 §7.1 because MAS deployments reject bare schemes.
const OIDC_CALLBACK_SCHEME_PREFIX: &str = "com.github.shinaoka.koushi-matrix:";
const OIDC_CALLBACK_PATH: &str = "auth/callback";
#[cfg(any(debug_assertions, test))]
const QA_LOGIN_PIPE_ENV: &str = "KOUSHI_QA_LOGIN_PIPE";
#[cfg(any(debug_assertions, test))]
const QA_CONTROL_PIPE_ENV: &str = "KOUSHI_QA_CONTROL_PIPE";
#[cfg(any(debug_assertions, test))]
const SKIP_KEYCHAIN_PERSISTENCE_ENV: &str = "KOUSHI_SKIP_KEYCHAIN_PERSISTENCE";

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
    /// Window-lifecycle connection, used only for synchronous latest-wins
    /// snapshot reads. `WindowEvent::CloseRequested` must decide whether to
    /// hide before it returns, because `api.prevent_close()` cannot be deferred
    /// across an await, so the close-to-hide gate cannot go through the
    /// `tokio::sync::Mutex`-guarded command connection.
    ///
    /// This connection is never polled for events, and that is safe: the event
    /// side of a `CoreConnection` is a `tokio::sync::broadcast::Receiver`
    /// (`crates/koushi-core/src/runtime/connection.rs`), whose ring is
    /// pre-allocated at `EVENT_QUEUE_CAPACITY` when the runtime is built and
    /// overwritten oldest-first by every send. A receiver that never drains is
    /// therefore lossy, not buffering: it retains no memory beyond the shared
    /// ring that already exists for the drained connections, and it never
    /// applies backpressure to senders. The snapshot side is a `watch`
    /// receiver, which is latest-wins by construction.
    pub(crate) window_lifecycle_connection: CoreConnection,
    /// Tauri-side timeline item count (updated by event loop; QA title only).
    pub(crate) timeline_items_count: Arc<AtomicUsize>,
    _forwarder_task: Option<CoreEventForwarderTask>,
    pub(crate) native_window_focus_generation: AtomicU64,
    pub(crate) viewport_sync_generation: viewport_sync::ViewportSyncGeneration,
    /// Graceful-quit barrier; see [`quit_request_action`].
    pub(crate) quit_stage: AtomicU8,
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
    let source_ref = request.uri().path().strip_prefix('/').unwrap_or_default();
    let Some(content) = lookup_renderable_thumbnail(source_ref) else {
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
    let native_artifacts = Arc::new(NativeArtifactRegistry::new());
    #[cfg(any(debug_assertions, test))]
    {
        if keychain_persistence_disabled_from_env() {
            return CoreRuntime::start_with_data_dir_and_native_artifact_port(
                data_dir,
                native_artifacts,
            );
        }
    }

    CoreRuntime::start_with_data_dir_and_os_backend_and_native_artifact_port(
        data_dir,
        std::sync::Arc::new(crate::keyring_backend::KeyringCredentialBackend),
        native_artifacts,
    )
}

fn observed_native_window_focus(event: &tauri::WindowEvent) -> Option<bool> {
    match event {
        tauri::WindowEvent::Focused(focused) => Some(*focused),
        _ => None,
    }
}

fn next_native_window_focus_generation(counter: &AtomicU64) -> Option<u64> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .ok()
        .and_then(|previous| previous.checked_add(1))
}

fn window_event_should_stop_background_tasks(event: &tauri::WindowEvent) -> bool {
    matches!(event, tauri::WindowEvent::Destroyed)
}

fn ensure_main_window_visible<R: tauri::Runtime>(app: &mut tauri::App<R>) {
    ensure_main_window_visible_for_handle(app.handle());
}

fn ensure_main_window_visible_for_handle<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    #[cfg(target_os = "macos")]
    activate_macos_application(app);

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

fn schedule_native_viewport_sync<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    trigger: viewport_sync::ViewportSyncTrigger,
) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let state = app.state::<CoreRuntimeState>();
        let _ =
            viewport_sync::synchronize_and_record(window, &state.viewport_sync_generation, trigger)
                .await;
    });
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MacosCloseRequestedAction {
    Hide,
    ExitFullscreenAndHide,
}

fn macos_close_requested_action(is_fullscreen: Option<bool>) -> MacosCloseRequestedAction {
    if is_fullscreen == Some(true) {
        MacosCloseRequestedAction::ExitFullscreenAndHide
    } else {
        MacosCloseRequestedAction::Hide
    }
}

impl MacosCloseRequestedAction {
    #[cfg(target_os = "macos")]
    fn diagnostic_token(self) -> &'static str {
        match self {
            Self::Hide => "hide",
            Self::ExitFullscreenAndHide => "exit_fullscreen_and_hide",
        }
    }
}

/// What a non-macOS `CloseRequested` should do.
///
/// macOS hides unconditionally per platform convention and does not use this
/// decision (overview.md, "Desktop Window Lifecycle And Tray").
#[cfg_attr(all(target_os = "macos", not(test)), allow(dead_code))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CloseRequestedAction {
    HideToTray,
    DestroyWindow,
}

/// Close-to-hide gate for Linux and Windows.
///
/// Both inputs must hold: the user must not have opted out, and a tray icon
/// must actually exist. Hiding the only window with no tray and no dock
/// presence would leave the process unreachable, so a missing or unknown tray
/// always lets the close proceed.
#[cfg_attr(all(target_os = "macos", not(test)), allow(dead_code))]
fn close_requested_action(tray_available: bool, close_to_tray: bool) -> CloseRequestedAction {
    if tray_available && close_to_tray {
        CloseRequestedAction::HideToTray
    } else {
        CloseRequestedAction::DestroyWindow
    }
}

impl CloseRequestedAction {
    #[cfg(not(target_os = "macos"))]
    fn diagnostic_token(self) -> &'static str {
        match self {
            Self::HideToTray => "hide_to_tray",
            Self::DestroyWindow => "destroy_window",
        }
    }
}

/// Graceful-quit barrier stage.
///
/// Process exit triggers `AppCommand::Shutdown` exactly once, no matter which
/// path started it — explicit Quit (app-menu or tray), or a real window close
/// that destroys the product window — and even though the exit request is
/// re-delivered after shutdown completes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QuitStage {
    Idle,
    ShuttingDown,
    ShutdownComplete,
}

const QUIT_STAGE_IDLE: u8 = 0;
const QUIT_STAGE_SHUTTING_DOWN: u8 = 1;
const QUIT_STAGE_SHUTDOWN_COMPLETE: u8 = 2;

impl QuitStage {
    fn from_repr(value: u8) -> Self {
        match value {
            QUIT_STAGE_SHUTTING_DOWN => Self::ShuttingDown,
            QUIT_STAGE_SHUTDOWN_COMPLETE => Self::ShutdownComplete,
            _ => Self::Idle,
        }
    }

    fn repr(self) -> u8 {
        match self {
            Self::Idle => QUIT_STAGE_IDLE,
            Self::ShuttingDown => QUIT_STAGE_SHUTTING_DOWN,
            Self::ShutdownComplete => QUIT_STAGE_SHUTDOWN_COMPLETE,
        }
    }
}

/// What an `ExitRequested` should do for a given barrier stage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QuitRequestAction {
    /// First request: hold the exit and submit core shutdown.
    BeginShutdown,
    /// Shutdown already in flight: hold the exit, submit nothing.
    AwaitShutdown,
    /// Shutdown finished: let the process exit.
    Exit,
}

fn quit_request_action(stage: QuitStage) -> QuitRequestAction {
    match stage {
        QuitStage::Idle => QuitRequestAction::BeginShutdown,
        QuitStage::ShuttingDown => QuitRequestAction::AwaitShutdown,
        QuitStage::ShutdownComplete => QuitRequestAction::Exit,
    }
}

/// Request application exit. Menu Quit, tray Quit, and this helper all end up
/// in the same `RunEvent::ExitRequested` barrier, so shutdown ordering has one
/// owner.
fn request_application_exit<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    app.exit(0);
}

/// Move the barrier out of `Idle` and report whether this caller won the race.
///
/// Only the winner may call [`begin_graceful_shutdown`]; every later caller
/// observes a non-`Idle` stage and must submit nothing. This is what makes
/// shutdown exactly-once when the window-destroy path and the subsequent
/// `RunEvent::ExitRequested` both want to start it.
fn claim_core_shutdown(quit_stage: &AtomicU8) -> bool {
    quit_stage
        .compare_exchange(
            QuitStage::Idle.repr(),
            QuitStage::ShuttingDown.repr(),
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_ok()
}

/// Hold the exit, shut the core runtime down, then exit for real.
///
/// The awaited submit is bounded by `commands::CORE_COMMAND_SUBMIT_TIMEOUT`
/// and its error is intentionally ignored, so `app.exit(0)` always runs and the
/// held exit can never deadlock.
fn begin_graceful_shutdown(app: tauri::AppHandle) {
    koushi_diagnostics::record(
        DiagnosticEvent::new(DiagnosticLevel::Info, "desktop.lifecycle", "quit_requested")
            .field(DiagnosticField::token("action", "graceful_shutdown")),
    );
    tauri::async_runtime::spawn(async move {
        {
            let core_state = app.state::<CoreRuntimeState>();
            let request_id = core_state.connection.lock().await.next_request_id();
            let _ = commands::submit_core_command(
                &core_state,
                CoreCommand::App(AppCommand::Shutdown { request_id }),
            )
            .await;
            core_state
                .quit_stage
                .store(QuitStage::ShutdownComplete.repr(), Ordering::Release);
        }
        app.exit(0);
    });
}

fn is_oidc_callback_url(url: &str) -> bool {
    // The registered redirect URI is hostless (`scheme:/auth/callback`), but
    // URL normalization between the browser, the OS opener, and the deep-link
    // plugin may add authority slashes; accept any number of leading slashes.
    let Some(rest) = url.strip_prefix(OIDC_CALLBACK_SCHEME_PREFIX) else {
        return false;
    };
    match rest
        .trim_start_matches('/')
        .strip_prefix(OIDC_CALLBACK_PATH)
    {
        Some("") => true,
        Some(tail) => tail.starts_with('?') || tail.starts_with('#'),
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
            .command(commands::session::build_complete_oidc_login_command(
                request_id,
                callback_url,
                dto::frontend_display_platform(),
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

pub fn run() {
    let restore_session = restore_session_enabled_from_env_value(
        std::env::var("KOUSHI_RESTORE_SESSION").ok().as_deref(),
    );

    let mut builder = tauri::Builder::default();

    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // The deep-link plugin consumes configured callback URLs and emits
            // them through `on_open_url`; keep this callback side-effect-free so
            // it never logs authorization callback query strings.
            koushi_diagnostics::record(
                DiagnosticEvent::new(
                    DiagnosticLevel::Info,
                    "desktop.lifecycle",
                    "reopen_requested",
                )
                .field(DiagnosticField::token("action", "show_main_window")),
            );
            ensure_main_window_visible_for_handle(&app);
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
            // Window creation can emit geometry events before setup finishes.
            // Keep persistence fail-closed until restore has armed this gate.
            app.manage(Mutex::new(WindowStatePersistenceGate::PreArm));

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

            let timeline_items_count = Arc::new(AtomicUsize::new(0));
            let forwarder_task = spawn_core_event_forwarder(
                app.handle().clone(),
                event_conn,
                Arc::clone(&timeline_items_count),
            );
            // synchronous snapshot connection for the window-close gate
            let window_lifecycle_connection = runtime.attach();
            let core_state = CoreRuntimeState {
                runtime,
                connection: TokioMutex::new(command_conn),
                window_lifecycle_connection,
                timeline_items_count,
                _forwarder_task: Some(forwarder_task),
                native_window_focus_generation: AtomicU64::new(0),
                viewport_sync_generation: viewport_sync::ViewportSyncGeneration::default(),
                quit_stage: AtomicU8::new(QuitStage::Idle.repr()),
            };
            app.manage(core_state);
            install_oidc_deep_link_handler(app)?;

            let menu = build_desktop_menu(app)?;
            app.set_menu(menu)?;
            // Best-effort by contract: a session with no status-notifier host
            // simply has no tray, and close-to-hide stays off there.
            tray::install_tray_icon(app);
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

            #[cfg(any(debug_assertions, test))]
            if let Some(pipe_path) = qa_login_pipe_path_from_env() {
                commands::diagnostics::spawn_qa_login_pipe_reader(app.handle().clone(), pipe_path);
            }

            #[cfg(any(debug_assertions, test))]
            if let Some(pipe_path) = qa_control_pipe_path_from_env() {
                commands::diagnostics::spawn_qa_control_pipe_reader(
                    app.handle().clone(),
                    pipe_path,
                );
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
                schedule_native_viewport_sync(
                    window.app_handle().clone(),
                    viewport_sync::ViewportSyncTrigger::PageLoad,
                );
            }
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let Some(focused) = observed_native_window_focus(event) {
                    if let Some(core_state) = window.try_state::<CoreRuntimeState>() {
                        if let Some(observation_generation) = next_native_window_focus_generation(
                            &core_state.native_window_focus_generation,
                        ) {
                            let app_handle = window.app_handle().clone();
                            tauri::async_runtime::spawn(async move {
                                let core_state = app_handle.state::<CoreRuntimeState>();
                                let request_id =
                                    core_state.connection.lock().await.next_request_id();
                                let command = commands::native_attention::
                                    build_observe_native_window_focus_command(
                                        request_id,
                                        focused,
                                        observation_generation,
                                    );
                                let _ = commands::submit_core_command(&core_state, command).await;
                            });
                        }
                    }
                }
                let viewport_trigger = match event {
                    tauri::WindowEvent::Resized(_) => {
                        Some(viewport_sync::ViewportSyncTrigger::Resized)
                    }
                    tauri::WindowEvent::ScaleFactorChanged { .. } => {
                        Some(viewport_sync::ViewportSyncTrigger::ScaleFactorChanged)
                    }
                    _ => None,
                };
                if let Some(trigger) = viewport_trigger {
                    schedule_native_viewport_sync(window.app_handle().clone(), trigger);
                }
                #[cfg(target_os = "macos")]
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    let _ = persist_close_window_state_if_ready(
                        window,
                        WindowCloseEvent::CloseRequested,
                    );
                    let action = macos_close_requested_action(window.is_fullscreen().ok());
                    api.prevent_close();
                    if matches!(action, MacosCloseRequestedAction::ExitFullscreenAndHide) {
                        let _ = window.set_fullscreen(false);
                    }
                    let _ = window.hide();
                    koushi_diagnostics::record(
                        DiagnosticEvent::new(
                            DiagnosticLevel::Info,
                            "desktop.lifecycle",
                            "close_requested",
                        )
                        .field(DiagnosticField::token("action", action.diagnostic_token()))
                        .field(DiagnosticField::boolean(
                            "was_fullscreen",
                            matches!(action, MacosCloseRequestedAction::ExitFullscreenAndHide),
                        )),
                    );
                    return;
                }
                #[cfg(not(target_os = "macos"))]
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    // The gate is a synchronous latest-wins snapshot read:
                    // `prevent_close` cannot be deferred across an await.
                    let close_to_tray = window
                        .try_state::<CoreRuntimeState>()
                        .map(|core_state| {
                            core_state
                                .window_lifecycle_connection
                                .snapshot()
                                .settings
                                .values
                                .window
                                .close_to_tray
                        })
                        .unwrap_or(false);
                    let action = close_requested_action(tray::tray_is_available(), close_to_tray);
                    koushi_diagnostics::record(
                        DiagnosticEvent::new(
                            DiagnosticLevel::Info,
                            "desktop.lifecycle",
                            "close_requested",
                        )
                        .field(DiagnosticField::token("action", action.diagnostic_token()))
                        .field(DiagnosticField::boolean(
                            "close_to_tray",
                            close_to_tray,
                        ))
                        .field(DiagnosticField::boolean(
                            "tray_available",
                            tray::tray_is_available(),
                        )),
                    );
                    if matches!(action, CloseRequestedAction::HideToTray) {
                        // Persist geometry exactly as a real close would, then
                        // keep the window alive. `DestroyWindow` falls through
                        // to the shared persistence path below instead.
                        let _ = persist_close_window_state_if_ready(
                            window,
                            WindowCloseEvent::CloseRequested,
                        );
                        api.prevent_close();
                        let _ = window.hide();
                        return;
                    }
                }
                if window_event_should_persist(event) {
                    if window_event_is_geometry(event) {
                        let _ = persist_observed_window_geometry(window);
                    } else if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                        let _ = persist_close_window_state_if_ready(
                            window,
                            WindowCloseEvent::CloseRequested,
                        );
                    } else if matches!(event, tauri::WindowEvent::Destroyed) {
                        let _ = persist_close_window_state_if_ready(
                            window,
                            WindowCloseEvent::Destroyed,
                        );
                    }
                }
                if window_event_should_stop_background_tasks(event) {
                    // The product window was really destroyed, so the process
                    // is going away. Enter the same barrier the Quit paths use
                    // instead of submitting a second `AppCommand::Shutdown`
                    // ahead of the `ExitRequested` that follows.
                    let app = window.app_handle().clone();
                    let claimed = app
                        .try_state::<CoreRuntimeState>()
                        .is_some_and(|core_state| claim_core_shutdown(&core_state.quit_stage));
                    if claimed {
                        begin_graceful_shutdown(app);
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::diagnostics::get_diagnostic_snapshot,
            commands::diagnostics::observe_viewport_sync,
            commands::session::get_snapshot,
            commands::session::settlement_snapshot,
            commands::session::resync_snapshot,
            commands::session::discover_login_methods,
            commands::session::start_oidc_login,
            commands::session::complete_oidc_login,
            commands::session::submit_login,
            commands::session::submit_soft_logout_reauth,
            commands::session::list_saved_sessions,
            commands::session::switch_account,
            commands::session::submit_recovery,
            commands::session::start_device_cleanup,
            commands::session::submit_device_cleanup_uia,
            commands::session::erase_local_data_anyway,
            commands::session::logout,
            commands::session::retry_sliding_sync_capability,
            commands::session::change_homeserver,
            commands::session::restart_sync,
            commands::settings::update_settings,
            commands::settings::import_legacy_settings,
            commands::settings::rebuild_search_index,
            commands::settings::set_room_url_preview_override,
            commands::native_attention::play_native_attention_sound,
            commands::native_attention::set_native_attention_badge,
            commands::room::select_room_list_filter,
            commands::room::mark_room_as_read,
            commands::room::mark_room_as_unread,
            commands::room::force_rotate_outbound_session,
            commands::room::set_room_notification_mode,
            commands::account::refresh_current_session_status,
            commands::account::load_account_management_capabilities,
            commands::account::change_password,
            commands::account::deactivate_account,
            commands::account::submit_account_management_uia,
            commands::local_encryption::probe_local_encryption_health,
            commands::local_encryption::reset_local_data,
            commands::e2ee::bootstrap_cross_signing,
            commands::e2ee::start_own_user_sas,
            commands::e2ee::retry_current_device_trust_discovery,
            commands::e2ee::mismatch_sas_verification,
            commands::e2ee::start_session_bootstrap,
            commands::e2ee::confirm_session_bootstrap_saved,
            commands::e2ee::enable_key_backup,
            commands::e2ee::bootstrap_secure_backup,
            commands::e2ee::recover_secure_backup,
            commands::e2ee::retry_secure_backup_inspection,
            commands::e2ee::change_secure_backup_passphrase,
            commands::e2ee::export_room_keys,
            commands::e2ee::import_room_keys,
            commands::e2ee::accept_verification,
            commands::e2ee::confirm_sas_verification,
            commands::e2ee::cancel_verification,
            commands::e2ee::reset_identity,
            commands::e2ee::cancel_identity_reset,
            commands::e2ee::submit_identity_reset_password,
            commands::e2ee::submit_identity_reset_oauth,
            commands::timeline::resolve_composer_key_action,
            commands::timeline::begin_composer_draft_renderer_generation,
            commands::timeline::acquire_composer_draft_lease,
            commands::timeline::release_composer_draft_lease,
            commands::navigation::update_navigation_preference,
            commands::navigation::select_space,
            commands::navigation::reorder_spaces,
            commands::navigation::select_room,
            commands::navigation::open_activity_event,
            commands::navigation::open_pinned_event,
            commands::navigation::select_search_result,
            commands::navigation::close_focused_context,
            commands::navigation::open_timeline_at_timestamp,
            commands::navigation::update_navigation_scroll_anchor,
            commands::navigation::observe_timeline_viewport,
            commands::timeline::ensure_timeline_subscribed,
            commands::timeline::paginate_timeline_backwards,
            commands::timeline::restore_timeline_anchor,
            commands::timeline::paginate_thread_timeline_backwards,
            commands::timeline::send_text,
            commands::timeline::schedule_send,
            commands::timeline::stage_upload_bytes,
            commands::timeline::select_staged_upload_output,
            commands::timeline::retry_staged_upload_preparation,
            commands::timeline::use_original_staged_upload,
            commands::timeline::prepared_upload_preview,
            commands::timeline::send_prepared_uploads,
            commands::timeline::update_staged_upload_caption,
            commands::timeline::update_staged_upload_compression,
            commands::timeline::clear_upload_staging,
            commands::timeline::cancel_scheduled_send,
            commands::timeline::reschedule_scheduled_send,
            commands::timeline::retry_send,
            commands::timeline::cancel_send,
            commands::timeline::download_media,
            commands::timeline::default_media_save_path,
            commands::timeline::save_downloaded_media,
            commands::timeline::load_message_source,
            commands::timeline::request_room_key,
            commands::timeline::request_late_decryption,
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
            commands::room::refresh_pinned_events,
            commands::room::load_room_settings,
            commands::room::load_space_members,
            commands::room::query_mention_candidates,
            commands::room::repair_room_timeline,
            commands::room::update_room_setting,
            commands::room::moderate_room_member,
            commands::room::update_room_member_role,
            commands::room::update_space_member_role,
            commands::activity::open_activity,
            commands::activity::close_activity,
            commands::activity::set_activity_tab,
            commands::activity::paginate_activity,
            commands::activity::retry_activity_resolution,
            commands::activity::mark_activity_read,
            commands::views::open_files_view,
            commands::views::close_files_view,
            commands::views::open_threads_list,
            commands::views::close_threads_list,
            commands::views::paginate_threads_list,
            commands::views::open_thread,
            commands::views::close_thread,
            commands::search::submit_search,
            commands::search::close_search,
            commands::search::start_room_crawl,
            commands::search::stop_room_crawl,
            commands::directory::query_directory,
            commands::room::create_room,
            commands::room::create_space,
            commands::directory::join_directory_room,
            commands::directory::preview_join_target,
            commands::directory::dismiss_directory_preview,
            commands::room::set_space_child,
            commands::room::join_room,
            commands::room::accept_invite,
            commands::room::decline_invite,
            commands::room::start_direct_message,
            commands::room::invite_user,
            commands::room::invite_user_to_space,
            commands::room::cancel_space_invite,
            commands::room::open_invite_workflow,
            commands::room::close_invite_workflow,
            commands::room::search_invite_targets,
            commands::room::set_invite_scope,
            commands::room::select_invite_target,
            commands::room::remove_invite_target,
            commands::room::invite_targets,
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
        .build(tauri::generate_context!())
        .expect("failed to build matrix desktop app")
        .run(|app, event| {
            // Hold the exit until core shutdown has completed; the re-delivered
            // request after completion proceeds. The barrier is shared with the
            // window-destroy path, so `AppCommand::Shutdown` is submitted
            // exactly once whether the product window was hidden or destroyed,
            // and `ExitRequested` is treated the same for any exit code.
            if let tauri::RunEvent::ExitRequested { api, .. } = &event {
                if let Some(core_state) = app.try_state::<CoreRuntimeState>() {
                    let stage = QuitStage::from_repr(core_state.quit_stage.load(Ordering::Acquire));
                    match quit_request_action(stage) {
                        QuitRequestAction::BeginShutdown => {
                            api.prevent_exit();
                            if claim_core_shutdown(&core_state.quit_stage) {
                                begin_graceful_shutdown(app.clone());
                            }
                        }
                        QuitRequestAction::AwaitShutdown => api.prevent_exit(),
                        QuitRequestAction::Exit => {}
                    }
                }
            }
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                koushi_diagnostics::record(
                    DiagnosticEvent::new(
                        DiagnosticLevel::Info,
                        "desktop.lifecycle",
                        "reopen_requested",
                    )
                    .field(DiagnosticField::token("action", "show_main_window")),
                );
                ensure_main_window_visible_for_handle(app);
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = (app, event);
            }
        });
}

#[cfg(test)]
mod tests;
