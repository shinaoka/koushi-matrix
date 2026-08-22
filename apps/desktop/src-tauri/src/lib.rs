#![recursion_limit = "256"]

mod commands;
mod core_event_forwarder;
mod desktop_menu;
mod dto;
pub mod keyring_backend;
mod window_state;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
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
use koushi_core::composer_draft_lifecycle::{ComposerDraftLeaseId, ComposerRendererGeneration};
use koushi_core::renderable_thumbnail::{
    cleanup_legacy_plaintext_thumbnail_dirs, lookup_renderable_thumbnail,
};
use koushi_core::{AccountCommand, AppCommand, CoreCommand, CoreConnection, CoreRuntime};
use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel};

const OIDC_CALLBACK_URL_PREFIX: &str = "koushi-desktop://auth/callback";
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
    pub(crate) composer_draft_transport: Mutex<ComposerDraftTransportIdentities>,
    /// Tauri-side timeline item count (updated by event loop; QA title only).
    pub(crate) timeline_items_count: Arc<AtomicUsize>,
    _forwarder_task: Option<CoreEventForwarderTask>,
    pub(crate) native_window_focus_generation: AtomicU64,
}

#[derive(Default)]
pub(crate) struct ComposerDraftTransportIdentities {
    next_generation: u64,
    next_lease_id: u64,
    generations: HashMap<String, ComposerRendererGeneration>,
    leases: HashMap<(String, String), ComposerDraftLeaseId>,
}

impl ComposerDraftTransportIdentities {
    pub(crate) fn install_generation(
        &mut self,
        generation: ComposerRendererGeneration,
    ) -> Result<String, String> {
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .ok_or_else(|| "composer renderer generation exhausted".to_owned())?;
        let wire = self.next_generation.to_string();
        self.generations.clear();
        self.leases.clear();
        self.generations.insert(wire.clone(), generation);
        Ok(wire)
    }

    pub(crate) fn generation(&self, wire: &str) -> Result<ComposerRendererGeneration, String> {
        self.generations
            .get(wire)
            .copied()
            .ok_or_else(|| "composer renderer generation retired".to_owned())
    }

    pub(crate) fn install_lease(
        &mut self,
        renderer_generation: &str,
        lease_id: ComposerDraftLeaseId,
    ) -> Result<String, String> {
        self.next_lease_id = self
            .next_lease_id
            .checked_add(1)
            .ok_or_else(|| "composer draft lease exhausted".to_owned())?;
        let wire = self.next_lease_id.to_string();
        self.leases
            .insert((renderer_generation.to_owned(), wire.clone()), lease_id);
        Ok(wire)
    }

    pub(crate) fn lease(
        &self,
        renderer_generation: &str,
        lease_id: &str,
    ) -> Result<ComposerDraftLeaseId, String> {
        self.leases
            .get(&(renderer_generation.to_owned(), lease_id.to_owned()))
            .copied()
            .ok_or_else(|| "composer draft lease mismatch".to_owned())
    }

    pub(crate) fn remove_lease(&mut self, renderer_generation: &str, lease_id: &str) {
        self.leases
            .remove(&(renderer_generation.to_owned(), lease_id.to_owned()));
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

fn is_oidc_callback_url(url: &str) -> bool {
    match url.strip_prefix(OIDC_CALLBACK_URL_PREFIX) {
        Some("") => true,
        Some(rest) => rest.starts_with('?') || rest.starts_with('#'),
        None => false,
    }
}

fn submit_core_shutdown(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let core_state = app.state::<CoreRuntimeState>();
        let request_id = core_state.connection.lock().await.next_request_id();
        let _ = commands::submit_core_command(
            &core_state,
            CoreCommand::App(AppCommand::Shutdown { request_id }),
        )
        .await;
    });
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
            let core_state = CoreRuntimeState {
                runtime,
                connection: TokioMutex::new(command_conn),
                composer_draft_transport: Mutex::new(ComposerDraftTransportIdentities::default()),
                timeline_items_count,
                _forwarder_task: Some(forwarder_task),
                native_window_focus_generation: AtomicU64::new(0),
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
                    submit_core_shutdown(window.app_handle().clone());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::diagnostics::get_diagnostic_snapshot,
            commands::session::get_snapshot,
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
            commands::settings::rebuild_search_index,
            commands::settings::set_room_url_preview_override,
            commands::native_attention::play_native_attention_sound,
            commands::native_attention::set_native_attention_badge,
            commands::room::force_new_outbound_session,
            commands::room::share_index0_room_key,
            commands::room::resend_index0_room_key,
            commands::room::select_room_list_filter,
            commands::room::mark_room_as_read,
            commands::room::mark_room_as_unread,
            commands::room::set_room_notification_mode,
            commands::account::query_devices,
            commands::account::refresh_current_session_status,
            commands::account::load_account_management_capabilities,
            commands::account::rename_device,
            commands::account::delete_devices,
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
            commands::e2ee::reenable_secure_backup,
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
            commands::navigation::select_space,
            commands::navigation::reorder_spaces,
            commands::navigation::select_room,
            commands::navigation::open_activity_event,
            commands::navigation::open_pinned_event,
            commands::navigation::select_search_result,
            commands::navigation::acknowledge_timeline_projection,
            commands::navigation::acknowledge_timeline_batch_rendered,
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
            commands::timeline::stage_uploads,
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
            commands::timeline::upload_media,
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
            commands::room::reshare_room_key,
            commands::room::update_room_setting,
            commands::room::moderate_room_member,
            commands::room::update_room_member_role,
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
mod tests {
    use super::{
        MacosCloseRequestedAction, desktop_menu_items, desktop_standard_menu_items,
        macos_close_requested_action, next_native_window_focus_generation,
        observed_native_window_focus, qa_control_pipe_path_from_env_value,
        qa_login_pipe_path_from_env_value, restore_session_enabled_from_env_value,
        saved_sessions_disabled_from_env_value, window_event_should_stop_background_tasks,
    };
    use crate::commands::diagnostics::parse_qa_login_pipe_payload;
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn main_window_overlay_permission_contract() {
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/windows-overlay.json"))
                .expect("Windows overlay capability must be valid JSON");
        assert_eq!(capability["identifier"], "windows-overlay");
        assert_eq!(capability["platforms"], serde_json::json!(["windows"]));
        assert_eq!(capability["windows"], serde_json::json!(["main"]));
        let permissions = capability["permissions"]
            .as_array()
            .expect("main capability permissions must be an array");
        assert!(
            permissions
                .iter()
                .any(|permission| permission == "core:window:allow-set-overlay-icon"),
            "main Windows window must explicitly admit the overlay command"
        );
    }

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
        )
        .expect("protocol fixture is within the thumbnail cache bound");
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
    fn observed_native_window_focus_extracts_only_focus_events() {
        assert_eq!(
            observed_native_window_focus(&tauri::WindowEvent::Focused(true)),
            Some(true)
        );
        assert_eq!(
            observed_native_window_focus(&tauri::WindowEvent::Focused(false)),
            Some(false)
        );
        assert_eq!(
            observed_native_window_focus(&tauri::WindowEvent::Resized(tauri::PhysicalSize::new(
                1280, 820
            ))),
            None
        );
        assert_eq!(
            observed_native_window_focus(&tauri::WindowEvent::Moved(tauri::PhysicalPosition::new(
                30, 50
            ))),
            None
        );
        assert_eq!(
            observed_native_window_focus(&tauri::WindowEvent::Destroyed),
            None
        );
    }

    #[test]
    fn native_window_focus_generation_is_monotonic_and_exhaustion_safe() {
        let counter = AtomicU64::new(0);
        assert_eq!(next_native_window_focus_generation(&counter), Some(1));
        assert_eq!(next_native_window_focus_generation(&counter), Some(2));

        let exhausted = AtomicU64::new(u64::MAX);
        assert_eq!(next_native_window_focus_generation(&exhausted), None);
        assert_eq!(exhausted.load(Ordering::Relaxed), u64::MAX);
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

        let source = include_str!("lib.rs");
        let destroyed_handler = source
            .split("if window_event_should_stop_background_tasks(event)")
            .nth(1)
            .and_then(|rest| rest.split(".invoke_handler").next())
            .expect("window destruction handler should exist");
        assert!(destroyed_handler.contains("submit_core_shutdown"));
        assert!(source.contains("AppCommand::Shutdown { request_id }"));
    }

    #[test]
    fn macos_close_requested_hides_without_stopping_background_tasks() {
        let source = include_str!("lib.rs");
        let stop_helper = source
            .split("fn window_event_should_stop_background_tasks")
            .nth(1)
            .and_then(|rest| rest.split("fn ensure_main_window_visible").next())
            .expect("window event stop helper should exist");
        assert!(
            !stop_helper.contains("CloseRequested"),
            "red close on macOS must hide the window without stopping account/runtime background tasks"
        );

        let close_handler = source
            .split(".on_window_event")
            .nth(1)
            .and_then(|rest| rest.split("tauri::WindowEvent::CloseRequested").nth(1))
            .and_then(|rest| rest.split("if window_event_should_persist").next())
            .expect("CloseRequested handler should be explicit before persistence handling");
        assert!(close_handler.contains("prevent_close()"));
        assert!(close_handler.contains(".hide()"));
    }

    #[test]
    fn macos_close_requested_exits_fullscreen_before_hiding() {
        assert_eq!(
            macos_close_requested_action(Some(true)),
            MacosCloseRequestedAction::ExitFullscreenAndHide
        );
        assert_eq!(
            macos_close_requested_action(Some(false)),
            MacosCloseRequestedAction::Hide
        );
        assert_eq!(
            macos_close_requested_action(None),
            MacosCloseRequestedAction::Hide
        );

        let source = include_str!("lib.rs");
        let close_handler = source
            .split(".on_window_event")
            .nth(1)
            .and_then(|rest| rest.split("tauri::WindowEvent::CloseRequested").nth(1))
            .and_then(|rest| rest.split("if window_event_should_persist").next())
            .expect("CloseRequested handler should be explicit before persistence handling");
        assert!(close_handler.contains("window.is_fullscreen()"));
        assert!(close_handler.contains("window.set_fullscreen(false)"));
        assert!(
            close_handler
                .find("window.set_fullscreen(false)")
                .expect("fullscreen close should exit fullscreen")
                < close_handler
                    .find("window.hide()")
                    .expect("close should hide the window")
        );
    }

    #[test]
    fn single_instance_reopen_shows_existing_main_window() {
        let source = include_str!("lib.rs");
        let callback = source
            .split("tauri_plugin_single_instance::init(")
            .nth(1)
            .and_then(|rest| rest.split(".plugin(tauri_plugin_deep_link::init())").next())
            .expect("single instance plugin callback should be wired before other plugins");

        assert!(
            callback.contains("ensure_main_window_visible_for_handle"),
            "reopening Koushi or launching a second instance should show and focus the resident main window"
        );
        assert!(callback.contains("desktop.lifecycle"));
        assert!(callback.contains("reopen_requested"));
    }

    #[test]
    fn macos_run_event_reopen_shows_existing_main_window() {
        let source = include_str!("lib.rs");
        let run_block = source
            .split("pub fn run()")
            .nth(1)
            .and_then(|rest| rest.split("#[cfg(test)]").next())
            .expect("run function body should exist");

        assert!(run_block.contains(".build(tauri::generate_context!())"));
        assert!(run_block.contains("tauri::RunEvent::Reopen"));
        assert!(run_block.contains("ensure_main_window_visible_for_handle"));
        assert!(run_block.contains("desktop.lifecycle"));
        assert!(run_block.contains("reopen_requested"));
    }

    #[test]
    fn oidc_callback_url_accepts_only_expected_auth_callback_shape() {
        assert!(super::is_oidc_callback_url(
            "koushi-desktop://auth/callback"
        ));
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
}
