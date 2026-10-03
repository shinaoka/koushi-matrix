#![recursion_limit = "256"]

mod app_updates;
mod commands;
mod core_event_forwarder;
mod desktop_menu;
mod dto;
pub mod keyring_backend;
mod media_save;
mod oidc_browser;
mod spell_checking;
mod tray;
mod viewport_sync;
mod window_state;

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering},
    },
};
use tokio::sync::Mutex as TokioMutex;

use tauri::{Emitter, Manager};

#[cfg(any(debug_assertions, test))]
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

// koushi-core owns each account runtime. All session, credential, and Matrix
// operations go through CoreCommand/CoreEvent; the adapter never touches the
// credential store or SDK directly.
use koushi_core::account_runtime_manager::{AccountRuntimeManager, AccountTabId};
use koushi_core::renderable_thumbnail::{
    cleanup_legacy_media_downloads, cleanup_legacy_plaintext_thumbnail_dirs,
    lookup_renderable_thumbnail,
};
use koushi_core::{
    CoreConnection, NativeArtifactPort, NativeArtifactRegistry, ReaderSubscription,
    ReaderSubscriptionCloser, settings::SettingsStore, store::StoreActor,
};
use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel};
use koushi_protocol::{CoreCommand, command::AccountCommand, view::ViewScopeId};

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
/// Holds the account runtime manager plus selected per-tab connections for
/// command dispatch and snapshot reads. Each account-tab watcher and the
/// selected event forwarder owns a separate connection so event reads do not
/// block command dispatch.
///
/// Startup restore and saved-session listing go through the canon command
/// boundary; the adapter never reads the credential store.
///
/// Remaining design note:
/// `timeline_items_count`: `AppState` snapshots never embed timeline lists
/// (Async rule 4). The count needed for `qa_window_title` is tracked here
/// via a Tauri-side counter updated by the event forwarding loop.
#[derive(Clone)]
pub(crate) struct ReaderSubscriptionEntry {
    pub(crate) subscription: Arc<TokioMutex<ReaderSubscription>>,
    pub(crate) close: ReaderSubscriptionCloser,
    pub(crate) control: koushi_core::ReaderSubscriptionControl,
}

type AccountConnections = Arc<TokioMutex<HashMap<AccountTabId, Arc<TokioMutex<CoreConnection>>>>>;

#[derive(Clone)]
pub(crate) struct SelectedCoreConnection {
    runtime: Arc<AccountRuntimeManager>,
    connections: AccountConnections,
    restore_ready: tokio::sync::watch::Receiver<bool>,
}

impl SelectedCoreConnection {
    pub(crate) async fn lock(&self) -> tokio::sync::OwnedMutexGuard<CoreConnection> {
        self.lock_with_tab_id().await.1
    }

    pub(crate) async fn lock_with_tab_id(
        &self,
    ) -> (String, tokio::sync::OwnedMutexGuard<CoreConnection>) {
        loop {
            let id = self.runtime.selected_tab_id();
            if let Ok(binding) = self.lock_for_tab_id(&id).await
                && self.runtime.selected_tab_id() == id
            {
                return binding;
            }
        }
    }

    pub(crate) async fn lock_for_tab_id(
        &self,
        id: &AccountTabId,
    ) -> Result<(String, tokio::sync::OwnedMutexGuard<CoreConnection>), String> {
        let mut restore_ready = self.restore_ready.clone();
        loop {
            if *restore_ready.borrow_and_update() {
                break;
            }
            if restore_ready.changed().await.is_err() {
                break;
            }
        }
        let connection = {
            let mut connections = self.connections.lock().await;
            if let Some(connection) = connections.get(id) {
                Arc::clone(connection)
            } else {
                let connection = self
                    .runtime
                    .tab_connection(id)
                    .ok_or_else(|| "account tab does not exist".to_owned())?;
                let connection = Arc::new(TokioMutex::new(connection));
                connections.insert(id.clone(), Arc::clone(&connection));
                connection
            }
        };
        Ok((id.as_str().to_owned(), connection.lock_owned().await))
    }

    pub(crate) async fn command_handle_for_request(
        &self,
        request_id: koushi_protocol::RequestId,
    ) -> Option<koushi_core::CoreCommandHandle> {
        let connections: Vec<_> = self.connections.lock().await.values().cloned().collect();
        for connection in connections {
            let connection = connection.lock().await;
            if connection.connection_id() == request_id.connection_id {
                return Some(connection.command_handle());
            }
        }
        None
    }

    pub(crate) async fn clear_cached_connections(&self) {
        self.connections.lock().await.clear();
    }

    pub(crate) async fn remove_cached_connection(&self, id: &AccountTabId) {
        self.connections.lock().await.remove(id);
    }

    #[cfg(not(target_os = "macos"))]
    fn window_settings(&self) -> koushi_state::WindowSettings {
        self.runtime
            .subscribe_app_settings_updates()
            .borrow()
            .window
    }
}

pub const ACCOUNT_TABS_EVENT_NAME: &str = "koushi-desktop://account-tabs-update";
const ACCOUNT_TAB_IDENTITY_MISMATCH_EVENT: &str = "koushi-desktop://account-identity-mismatch";

async fn retire_rejected_login(connection: &mut CoreConnection) -> bool {
    let request_id = connection.next_request_id();
    if connection
        .command(CoreCommand::Account(AccountCommand::ChangeHomeserver {
            request_id,
        }))
        .await
        .is_err()
    {
        return false;
    }
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if matches!(
                connection.snapshot().session,
                koushi_state::SessionState::SignedOut
            ) {
                return true;
            }
            if connection.next_versioned_snapshot().await.is_none() {
                return false;
            }
        }
    })
    .await
    .unwrap_or(false)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AccountTabStatus {
    AddAccount,
    Restoring,
    Authenticating,
    NeedsVerification,
    Ready,
    SignedOut,
    LoggingOut,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountTabSummary {
    pub id: String,
    pub account_key: Option<String>,
    pub homeserver: Option<String>,
    pub display_name: Option<String>,
    pub avatar_source_ref: Option<String>,
    pub status: AccountTabStatus,
    pub unread_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountTabsSnapshot {
    pub selected_tab_id: String,
    pub tabs: Vec<AccountTabSummary>,
    pub badge_count: u64,
}

pub struct CoreRuntimeState {
    pub(crate) runtime: Arc<AccountRuntimeManager>,
    /// Resolves to the selected account's independent CoreConnection at each
    /// command boundary.
    pub(crate) connection: SelectedCoreConnection,
    /// Synchronous latest-wins access to Rust-owned app window settings.
    /// `CloseRequested` cannot await command locks, and a retiring/closed
    /// manager may have no selected child. Read the shared settings owner,
    /// independent of the account actor trees. macOS hides on close.
    #[cfg(not(target_os = "macos"))]
    pub(crate) window_lifecycle_connection: SelectedCoreConnection,
    /// Tauri-side timeline item count (updated by event loop; QA title only).
    pub(crate) timeline_items_count: Arc<AtomicUsize>,
    _forwarder_task: Mutex<Option<CoreEventForwarderTask>>,
    startup_restore_task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    account_tab_watchers: TokioMutex<Vec<tauri::async_runtime::JoinHandle<()>>>,
    pub(crate) native_window_focused: AtomicBool,
    pub(crate) native_window_focus_generation: AtomicU64,
    pub(crate) viewport_sync_generation: viewport_sync::ViewportSyncGeneration,
    /// Graceful-quit barrier; see [`quit_request_action`].
    pub(crate) quit_stage: AtomicU8,
    /// Written by the updater before it leaves the owner joined by shutdown.
    restart_after_shutdown: AtomicBool,
    pub(crate) reader_subscriptions:
        TokioMutex<HashMap<(Option<AccountTabId>, ViewScopeId), ReaderSubscriptionEntry>>,
}

impl CoreRuntimeState {
    pub(crate) fn restart_selected_forwarder(&self, app: tauri::AppHandle) {
        let (tab_id, connection) = self.runtime.selected_binding();
        let task = spawn_core_event_forwarder(
            app,
            tab_id,
            connection,
            Arc::clone(&self.timeline_items_count),
        );
        *self
            ._forwarder_task
            .lock()
            .expect("core event forwarder mutex") = Some(task);
    }

    pub(crate) async fn stop_selected_forwarder(&self) {
        let task = self
            ._forwarder_task
            .lock()
            .expect("core event forwarder mutex")
            .take();
        if let Some(task) = task {
            task.stop().await;
        }
    }

    async fn wait_for_startup_restore(&self) -> Result<(), ()> {
        let task = self
            .startup_restore_task
            .lock()
            .expect("startup restore task mutex")
            .take();
        if let Some(task) = task {
            task.await.map_err(|_| ())?;
        }
        Ok(())
    }

    pub(crate) async fn stop_account_tab_watchers(&self) {
        let watchers = {
            let mut watchers = self.account_tab_watchers.lock().await;
            std::mem::take(&mut *watchers)
        };
        for watcher in watchers {
            watcher.abort();
            let _ = watcher.await;
        }
    }

    pub(crate) async fn close_reader_subscriptions(&self) {
        let mut subscriptions = self.reader_subscriptions.lock().await;
        for entry in subscriptions.values() {
            entry.close.close();
        }
        subscriptions.clear();
    }

    pub(crate) async fn restart_account_tab_watchers(&self, app: tauri::AppHandle) {
        self.stop_account_tab_watchers().await;
        let tabs = self.runtime.tab_connections();
        let mut watchers = self.account_tab_watchers.lock().await;
        for (tab, mut connection) in tabs {
            let tab_id = tab.id.clone();
            let runtime = Arc::clone(&self.runtime);
            let app = app.clone();
            watchers.push(tauri::async_runtime::spawn(async move {
                let mut previous_attention = connection.snapshot().native_attention;
                while let Some(snapshot) = connection.next_versioned_snapshot().await {
                    if let Some(info) = session_info_for_account(&snapshot.state.session)
                        && !runtime.is_session_binding_current(&tab_id, &info)
                    {
                            match runtime.bind_authenticated_session(&tab_id, &info).await {
                                Ok(
                                    koushi_core::account_runtime_manager::BindAccountOutcome::Bound(
                                        _,
                                    ),
                                ) => {}
                                Ok(
                                    koushi_core::account_runtime_manager::BindAccountOutcome::Duplicate(
                                        existing,
                                    ),
                                ) => {
                                    // This provisional login may have reused the existing
                                    // account's stored device; retire it without revoking or
                                    // deleting shared account persistence.
                                    if retire_rejected_login(&mut connection).await {
                                        let previous_tab = runtime.selected_tab_id();
                                        if runtime.select_tab(&existing).await.unwrap_or(false) {
                                            let state = app.state::<CoreRuntimeState>();
                                            state.close_reader_subscriptions().await;
                                            commands::native_attention::transfer_native_window_focus(
                                                &state.runtime,
                                                &state.native_window_focus_generation,
                                                &previous_tab,
                                                &existing,
                                                state.native_window_focused.load(Ordering::Relaxed),
                                            )
                                            .await;
                                            state.stop_selected_forwarder().await;
                                            state.restart_selected_forwarder(app.clone());
                                            emit_account_tabs_changed(&app, &runtime);
                                        }
                                    }
                                }
                                Ok(
                                    koushi_core::account_runtime_manager::BindAccountOutcome::IdentityMismatch {
                                        ..
                                    },
                                ) => {
                                    if retire_rejected_login(&mut connection).await {
                                        let _ = app.emit(
                                            ACCOUNT_TAB_IDENTITY_MISMATCH_EVENT,
                                            tab_id.as_str(),
                                        );
                                    }
                                }
                                Err(_) => {}
                            }
                    }
                    let descriptor = runtime
                        .tab_descriptors()
                        .into_iter()
                        .find(|tab| tab.id == tab_id);
                    let attention = &snapshot.state.native_attention;
                    let background = runtime.selected_tab_id() != tab_id;
                    if background
                        && descriptor
                            .as_ref()
                            .is_some_and(|tab| tab.account_key.is_some())
                    {
                        if attention.notification != previous_attention.notification {
                            commands::native_attention::notification::dispatch_notification_for_tab(
                                &app,
                                &tab_id,
                                &snapshot.state,
                            );
                        }
                        if attention.summary.candidate != previous_attention.summary.candidate
                            && snapshot.state.settings.values.notifications.sound
                            && cfg!(any(target_os = "macos", target_os = "windows"))
                            && matches!(snapshot.state.session, koushi_state::SessionState::Ready(_))
                            && let Some(sound_connection) = runtime.tab_connection(&tab_id)
                        {
                            let _ = commands::native_attention::dispatch_native_attention_sound_for_connection(
                                app.clone(),
                                sound_connection,
                            )
                            .await;
                        }
                    }
                    previous_attention = attention.clone();
                    emit_account_tabs_changed(&app, &runtime);
                    allow_account_media_cache_dirs(&app, &runtime);
                }
            }));
        }
        emit_account_tabs_changed(&app, &self.runtime);
        allow_account_media_cache_dirs(&app, &self.runtime);
    }
}

fn session_info_for_account(
    session: &koushi_state::SessionState,
) -> Option<koushi_state::SessionInfo> {
    match session {
        koushi_state::SessionState::Ready(info)
        | koushi_state::SessionState::Provisional { info, .. }
        | koushi_state::SessionState::AwaitingVerification { info, .. }
        | koushi_state::SessionState::Verifying { info, .. }
        | koushi_state::SessionState::AwaitingBootstrapConfirmation { info, .. }
        | koushi_state::SessionState::Rejecting { info, .. }
        | koushi_state::SessionState::Locked(info)
        | koushi_state::SessionState::CapabilityBlocked { info, .. }
        | koushi_state::SessionState::SwitchingAccount { info } => Some(info.clone()),
        _ => None,
    }
}

pub(crate) fn account_tabs_snapshot(runtime: &AccountRuntimeManager) -> AccountTabsSnapshot {
    let selected_tab_id = runtime.selected_tab_id().as_str().to_owned();
    let tab_states = runtime
        .tab_connections()
        .into_iter()
        .map(|(tab, connection)| (tab, connection.snapshot()))
        .collect::<Vec<_>>();
    account_tabs_snapshot_from_states(selected_tab_id, tab_states)
}

fn account_tabs_snapshot_from_states(
    selected_tab_id: String,
    tab_states: impl IntoIterator<
        Item = (
            koushi_core::account_runtime_manager::AccountTabDescriptor,
            koushi_state::AppState,
        ),
    >,
) -> AccountTabsSnapshot {
    let tab_states = tab_states.into_iter().collect::<Vec<_>>();
    let badge_count = aggregate_account_badge_count(
        tab_states
            .iter()
            .map(|(_, state)| state.native_attention.summary.badge_count),
    );
    let tabs = tab_states
        .into_iter()
        .map(|(tab, state)| {
            let status = match &state.session {
                koushi_state::SessionState::SignedOut => {
                    if tab.account_key.is_none() {
                        AccountTabStatus::AddAccount
                    } else {
                        AccountTabStatus::SignedOut
                    }
                }
                koushi_state::SessionState::Restoring
                | koushi_state::SessionState::SwitchingAccount { .. } => {
                    AccountTabStatus::Restoring
                }
                koushi_state::SessionState::Authenticating { .. }
                | koushi_state::SessionState::Provisional { .. } => {
                    AccountTabStatus::Authenticating
                }
                koushi_state::SessionState::AwaitingVerification { .. }
                | koushi_state::SessionState::Verifying { .. }
                | koushi_state::SessionState::AwaitingBootstrapConfirmation { .. } => {
                    AccountTabStatus::NeedsVerification
                }
                koushi_state::SessionState::Ready(_) => AccountTabStatus::Ready,
                koushi_state::SessionState::LoggingOut => AccountTabStatus::LoggingOut,
                koushi_state::SessionState::Rejecting { .. }
                | koushi_state::SessionState::Locked(_)
                | koushi_state::SessionState::CapabilityBlocked { .. } => AccountTabStatus::Error,
            };
            let avatar_source_ref =
                state
                    .profile
                    .own
                    .avatar
                    .as_ref()
                    .and_then(|avatar| match &avatar.thumbnail {
                        koushi_state::AvatarThumbnailState::Ready { source_ref, .. } => {
                            Some(source_ref.clone())
                        }
                        _ => None,
                    });
            AccountTabSummary {
                id: tab.id.as_str().to_owned(),
                account_key: tab.account_key.map(|key| key.0),
                homeserver: tab.homeserver,
                display_name: state.profile.own.display_name,
                avatar_source_ref,
                status,
                unread_count: state.native_attention.summary.unread_count,
            }
        })
        .collect();
    AccountTabsSnapshot {
        selected_tab_id,
        tabs,
        badge_count,
    }
}

fn aggregate_account_badge_count(counts: impl IntoIterator<Item = u64>) -> u64 {
    counts.into_iter().fold(0, u64::saturating_add)
}

fn emit_account_tabs_changed(app: &tauri::AppHandle, runtime: &AccountRuntimeManager) {
    let _ = app.emit(ACCOUNT_TABS_EVENT_NAME, account_tabs_snapshot(runtime));
}

fn allow_account_media_cache_dirs(app: &tauri::AppHandle, runtime: &AccountRuntimeManager) {
    let asset_scope = app.asset_protocol_scope();
    for cache_dir in runtime.media_cache_dirs() {
        let _ = asset_scope.allow_directory(cache_dir, true);
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

fn start_account_runtime_manager_for_tauri(data_dir: PathBuf) -> Arc<AccountRuntimeManager> {
    let store = {
        #[cfg(any(debug_assertions, test))]
        if keychain_persistence_disabled_from_env() {
            StoreActor::new(data_dir.clone())
        } else {
            StoreActor::with_os_backend(
                data_dir.clone(),
                Arc::new(crate::keyring_backend::KeyringCredentialBackend),
            )
        }
        #[cfg(not(any(debug_assertions, test)))]
        {
            StoreActor::with_os_backend(
                data_dir.clone(),
                Arc::new(crate::keyring_backend::KeyringCredentialBackend),
            )
        }
    };
    let native_artifact_factory: Arc<dyn Fn() -> Arc<dyn NativeArtifactPort> + Send + Sync> =
        Arc::new(|| Arc::new(NativeArtifactRegistry::new()));
    Arc::new(AccountRuntimeManager::new(
        store,
        SettingsStore::new(data_dir),
        native_artifact_factory,
    ))
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

// Pure decision for the macOS window-close path; compiled for its macOS caller
// and for the cross-platform window-close tests.
#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MacosCloseRequestedAction {
    Hide,
    ExitFullscreenAndHide,
}

#[cfg(any(target_os = "macos", test))]
fn macos_close_requested_action(is_fullscreen: Option<bool>) -> MacosCloseRequestedAction {
    if is_fullscreen == Some(true) {
        MacosCloseRequestedAction::ExitFullscreenAndHide
    } else {
        MacosCloseRequestedAction::Hide
    }
}

#[cfg(target_os = "macos")]
impl MacosCloseRequestedAction {
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
/// Process exit shuts down the account runtimes exactly once, no matter which
/// path started it — explicit Quit (app-menu or tray), or a real window close
/// that destroys the product window — and even though the exit request is
/// re-delivered after shutdown completes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QuitStage {
    Idle,
    ShuttingDown,
    ShutdownComplete,
    ForcedExit,
}

const QUIT_STAGE_IDLE: u8 = 0;
const QUIT_STAGE_SHUTTING_DOWN: u8 = 1;
const QUIT_STAGE_SHUTDOWN_COMPLETE: u8 = 2;
const QUIT_STAGE_FORCED_EXIT: u8 = 3;

impl QuitStage {
    fn from_repr(value: u8) -> Self {
        match value {
            QUIT_STAGE_SHUTTING_DOWN => Self::ShuttingDown,
            QUIT_STAGE_SHUTDOWN_COMPLETE => Self::ShutdownComplete,
            QUIT_STAGE_FORCED_EXIT => Self::ForcedExit,
            _ => Self::Idle,
        }
    }

    fn repr(self) -> u8 {
        match self {
            Self::Idle => QUIT_STAGE_IDLE,
            Self::ShuttingDown => QUIT_STAGE_SHUTTING_DOWN,
            Self::ShutdownComplete => QUIT_STAGE_SHUTDOWN_COMPLETE,
            Self::ForcedExit => QUIT_STAGE_FORCED_EXIT,
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
        QuitStage::ShutdownComplete | QuitStage::ForcedExit => QuitRequestAction::Exit,
    }
}

/// Request application exit. Menu Quit, tray Quit, and this helper all end up
/// in the same `RunEvent::ExitRequested` barrier, so shutdown ordering has one
/// owner.
fn request_application_exit<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    app.exit(0);
}

trait ApplicationExit {
    fn ordinary_exit(&self);
    fn final_restart(&self);
}

impl<R: tauri::Runtime> ApplicationExit for tauri::AppHandle<R> {
    fn ordinary_exit(&self) {
        self.exit(0);
    }
    fn final_restart(&self) {
        self.request_restart();
    }
}

// Restart-after-shutdown barrier shared by every updater install backend. It
// is compiled where a backend can request a relaunch and in the
// platform-neutral restart-barrier tests (see app_updates.rs).
#[cfg(any(koushi_updater_backend, test))]
fn request_application_restart_with(
    quit_stage: &AtomicU8,
    restart_after_shutdown: &AtomicBool,
    exit: &impl ApplicationExit,
) {
    if matches!(
        QuitStage::from_repr(quit_stage.load(Ordering::Acquire)),
        QuitStage::ShutdownComplete | QuitStage::ForcedExit
    ) {
        return;
    }
    if !restart_after_shutdown.swap(true, Ordering::AcqRel) {
        // Tauri's special restart exit cannot be prevented. First take the
        // ordinary path so the single shutdown coordinator can await cleanup.
        exit.ordinary_exit();
    }
}

#[cfg(koushi_updater_backend)]
pub(crate) fn request_application_restart(app: &tauri::AppHandle) {
    let core_state = app.state::<CoreRuntimeState>();
    request_application_restart_with(
        &core_state.quit_stage,
        &core_state.restart_after_shutdown,
        app,
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CoreExitOutcome {
    Completed,
    Failed,
    TimedOut,
}

const CORE_EXIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[cfg(test)]
async fn stop_core_for_exit(runtime: &AccountRuntimeManager) -> CoreExitOutcome {
    stop_core_for_exit_with_timeout(runtime, CORE_EXIT_TIMEOUT).await
}

async fn stop_core_state_for_exit(state: &CoreRuntimeState) -> CoreExitOutcome {
    await_core_exit(CORE_EXIT_TIMEOUT, async {
        let restore_result = state.wait_for_startup_restore().await;
        state.close_reader_subscriptions().await;
        state.stop_account_tab_watchers().await;
        state.stop_selected_forwarder().await;
        state.connection.clear_cached_connections().await;
        let shutdown_result = state.runtime.shutdown_all_checked().await;
        if restore_result.is_err() || shutdown_result.is_err() {
            Err(())
        } else {
            Ok(())
        }
    })
    .await
}

#[cfg(test)]
async fn stop_core_for_exit_with_timeout(
    runtime: &AccountRuntimeManager,
    timeout: std::time::Duration,
) -> CoreExitOutcome {
    await_core_exit(timeout, async {
        runtime.shutdown_all_checked().await.map_err(|_| ())
    })
    .await
}

async fn await_core_exit(
    timeout: std::time::Duration,
    shutdown: impl std::future::Future<Output = Result<(), ()>>,
) -> CoreExitOutcome {
    match tokio::time::timeout(timeout, shutdown).await {
        Ok(Ok(())) => CoreExitOutcome::Completed,
        Ok(Err(())) => CoreExitOutcome::Failed,
        Err(_) => CoreExitOutcome::TimedOut,
    }
}

async fn finish_application_shutdown(
    quit_stage: &AtomicU8,
    restart_after_shutdown: &AtomicBool,
    updater: impl std::future::Future<Output = ()>,
    core: impl std::future::Future<Output = CoreExitOutcome>,
    exit: &impl ApplicationExit,
) {
    // Never apply the Core deadline to blocking native application replacement.
    updater.await;
    let outcome = core.await;
    if outcome == CoreExitOutcome::Completed {
        quit_stage.store(QuitStage::ShutdownComplete.repr(), Ordering::Release);
        if restart_after_shutdown.load(Ordering::Acquire) {
            exit.final_restart();
        } else {
            exit.ordinary_exit();
        }
    } else {
        restart_after_shutdown.store(false, Ordering::Release);
        quit_stage.store(QuitStage::ForcedExit.repr(), Ordering::Release);
        koushi_diagnostics::record(
            DiagnosticEvent::new(DiagnosticLevel::Warn, "desktop.lifecycle", "forced_exit").field(
                DiagnosticField::token(
                    "reason",
                    match outcome {
                        CoreExitOutcome::TimedOut => "core_shutdown_timeout",
                        _ => "core_shutdown_failed",
                    },
                ),
            ),
        );
        exit.ordinary_exit();
    }
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
/// After the updater/installer settles, Core submission and acknowledged cleanup
/// share a deadline. Failure authorizes a diagnosed ordinary exit, never restart.
fn begin_graceful_shutdown(app: tauri::AppHandle) {
    koushi_diagnostics::record(
        DiagnosticEvent::new(DiagnosticLevel::Info, "desktop.lifecycle", "quit_requested")
            .field(DiagnosticField::token("action", "graceful_shutdown")),
    );
    tauri::async_runtime::spawn(async move {
        let core_state = app.state::<CoreRuntimeState>();
        finish_application_shutdown(
            &core_state.quit_stage,
            &core_state.restart_after_shutdown,
            app_updates::shutdown(&app),
            stop_core_state_for_exit(&core_state),
            &app,
        )
        .await;
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

#[cfg(target_os = "linux")]
fn repair_linux_deep_link_desktop_entry_contents(contents: &str) -> String {
    let repaired = contents
        .lines()
        .map(|line| {
            let Some(executable) = line
                .strip_prefix("Exec=\"")
                .and_then(|value| value.strip_suffix("\" %u"))
            else {
                return line.to_owned();
            };
            // xdg-open's shell parser understands backslash escapes but not
            // quote marks. Only remove the quotes when the executable path is
            // safe as an unquoted desktop-entry argument; paths containing
            // whitespace retain the standards-compliant form for launchers
            // that implement the full Desktop Entry specification.
            if executable
                .chars()
                .any(|character| character.is_whitespace() || character == '"')
            {
                line.to_owned()
            } else {
                format!("Exec={executable} %u")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    if contents.ends_with('\n') {
        format!("{repaired}\n")
    } else {
        repaired
    }
}

#[cfg(target_os = "linux")]
fn repair_linux_deep_link_desktop_entry(app: &tauri::App) -> tauri::Result<()> {
    let executable = tauri::utils::platform::current_exe()?;
    let Some(file_name) = executable.file_name() else {
        return Ok(());
    };
    let desktop_entry = app
        .path()
        .data_dir()?
        .join("applications")
        .join(format!("{}-handler.desktop", file_name.to_string_lossy()));
    let Ok(contents) = std::fs::read_to_string(&desktop_entry) else {
        return Ok(());
    };
    let repaired = repair_linux_deep_link_desktop_entry_contents(&contents);
    if repaired != contents {
        std::fs::write(desktop_entry, repaired)?;
    }
    Ok(())
}

pub(crate) fn oidc_callback_state(callback_url: &str) -> Option<String> {
    let url = url::Url::parse(callback_url).ok()?;
    let states: Vec<_> = url
        .query_pairs()
        .filter(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .collect();
    match states.as_slice() {
        [state] if !state.is_empty() => Some(state.clone()),
        _ => None,
    }
}

fn submit_oidc_callback_url(app: tauri::AppHandle, callback_url: String) {
    if !is_oidc_callback_url(&callback_url) {
        return;
    }

    let Some(oidc_state) = oidc_callback_state(&callback_url) else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let core_state = app.state::<CoreRuntimeState>();
        let Some(tab_id) = core_state.runtime.take_oidc_attempt(&oidc_state) else {
            return;
        };
        let Ok((_, connection)) = core_state.connection.lock_for_tab_id(&tab_id).await else {
            return;
        };
        let request_id = connection.next_request_id();
        let _ = connection
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
    if app.deep_link().register_all().is_ok() {
        #[cfg(target_os = "linux")]
        let _ = repair_linux_deep_link_desktop_entry(app);
    }

    Ok(())
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
fn install_oidc_deep_link_handler(_app: &tauri::App) -> tauri::Result<()> {
    Ok(())
}

pub fn run() {
    #[cfg(target_os = "macos")]
    desktop_menu::configure_fullscreen_menu();

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
            ensure_main_window_visible_for_handle(app);
        }));
    }

    #[cfg(target_os = "macos")]
    {
        builder = builder.plugin(
            tauri_plugin_updater::Builder::new()
                .pubkey(app_updates::configured_updater_public_key().unwrap_or_default())
                .build(),
        );
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
            let _ = cleanup_legacy_media_downloads(&data_dir);
            // Enter Tauri's tokio runtime so `executor::spawn` (tokio::task::spawn)
            // can find a runtime handle from this non-tokio-worker thread.
            let async_handle = tauri::async_runtime::handle();
            let _guard = async_handle.inner().enter();
            let runtime = start_account_runtime_manager_for_tauri(data_dir);
            let account_connections = Arc::new(TokioMutex::new(HashMap::new()));
            let update_settings = runtime.subscribe_app_settings_updates();
            let timeline_items_count = Arc::new(AtomicUsize::new(0));
            let (restore_ready_sender, restore_ready) = tokio::sync::watch::channel(false);
            let core_state = CoreRuntimeState {
                runtime: Arc::clone(&runtime),
                connection: SelectedCoreConnection {
                    runtime: Arc::clone(&runtime),
                    connections: Arc::clone(&account_connections),
                    restore_ready: restore_ready.clone(),
                },
                #[cfg(not(target_os = "macos"))]
                window_lifecycle_connection: SelectedCoreConnection {
                    runtime: Arc::clone(&runtime),
                    connections: account_connections,
                    restore_ready,
                },
                timeline_items_count,
                _forwarder_task: Mutex::new(None),
                startup_restore_task: Mutex::new(None),
                account_tab_watchers: TokioMutex::new(Vec::new()),
                native_window_focused: AtomicBool::new(false),
                native_window_focus_generation: AtomicU64::new(0),
                viewport_sync_generation: viewport_sync::ViewportSyncGeneration::default(),
                quit_stage: AtomicU8::new(QuitStage::Idle.repr()),
                restart_after_shutdown: AtomicBool::new(false),
                reader_subscriptions: TokioMutex::new(HashMap::new()),
            };
            app.manage(core_state);
            app.manage(app_updates::DesktopUpdateManager::new());
            app.manage(commands::dropped_files::DroppedFileLedger::default());
            app_updates::spawn_auto_update_loop(app.handle().clone(), update_settings);
            install_oidc_deep_link_handler(app)?;

            let app_handle = app.handle().clone();
            let restore_task = tauri::async_runtime::spawn(async move {
                let state = app_handle.state::<CoreRuntimeState>();
                // No event receiver may retain a connection while restore replaces the
                // manager's initial add-tab runtime; always restart forwarding afterward.
                state.stop_selected_forwarder().await;
                if restore_session {
                    let _ = state.runtime.restore_saved_accounts().await;
                    state.connection.clear_cached_connections().await;
                }
                state.restart_selected_forwarder(app_handle.clone());
                state.restart_account_tab_watchers(app_handle.clone()).await;
                emit_account_tabs_changed(&app_handle, &state.runtime);
                let _ = restore_ready_sender.send(true);
            });
            *app.state::<CoreRuntimeState>()
                .startup_restore_task
                .lock()
                .expect("startup restore task mutex") = Some(restore_task);

            // Built before the webview resolves the catalog locale; the
            // localized labels arrive through set_native_menu_labels.
            let menu = build_desktop_menu(app, &Default::default())?;
            app.set_menu(menu)?;
            // Best-effort by contract: a session with no status-notifier host
            // simply has no tray, and close-to-hide stays off there.
            tray::install_tray_icon(app);
            let _ = restore_main_window_state(app);
            ensure_main_window_visible(app);
            spell_checking::enable_for_main_window(app.handle());
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
                // Only Linux enables native drag/drop (tauri.linux.conf.json).
                if let tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) =
                    event
                    && let Some(ledger) =
                        window.try_state::<commands::dropped_files::DroppedFileLedger>()
                {
                    ledger.record_drop(paths);
                }
                if let Some(focused) = observed_native_window_focus(event)
                    && let Some(core_state) = window.try_state::<CoreRuntimeState>()
                {
                    core_state
                        .native_window_focused
                        .store(focused, Ordering::Relaxed);
                    if let Some(observation_generation) = next_native_window_focus_generation(
                        &core_state.native_window_focus_generation,
                    ) {
                        let app_handle = window.app_handle().clone();
                        tauri::async_runtime::spawn(async move {
                            let core_state = app_handle.state::<CoreRuntimeState>();
                            let request_id = core_state.connection.lock().await.next_request_id();
                            let command =
                                commands::native_attention::build_observe_native_window_focus_command(
                                    request_id,
                                    focused,
                                    observation_generation,
                                );
                            let _ = commands::submit_core_command(&core_state, command).await;
                        });
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
                                .window_settings()
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
                        .field(DiagnosticField::boolean("close_to_tray", close_to_tray))
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
                    // instead of shutting runtimes down ahead of the following
                    // `ExitRequested`.
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
            commands::app_updates::check_for_desktop_update,
            commands::app_updates::get_desktop_update_state,
            commands::app_updates::download_desktop_update,
            commands::app_updates::restart_to_install_desktop_update,
            commands::diagnostics::get_diagnostic_snapshot,
            commands::diagnostics::observe_viewport_sync,
            commands::account_tabs::list_account_tabs,
            commands::account_tabs::select_account_tab,
            commands::account_tabs::add_account_tab,
            commands::account_tabs::remove_signed_out_account_tab,
            commands::account_tabs::cancel_add_account_tab,
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
            desktop_menu::native_menu_label_keys,
            desktop_menu::set_native_menu_labels,
            commands::native_attention::play_native_attention_sound,
            commands::native_attention::set_native_attention_badge,
            commands::native_attention::notification::show_native_attention_notification,
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
            commands::account::load_account_notifications,
            commands::account::load_contact_security,
            commands::account::close_contact_security,
            commands::account::request_contact_verification,
            commands::account::set_notification_category,
            commands::account::set_account_push_enabled,
            commands::account::request_notification_email_token,
            commands::account::resend_notification_email_token,
            commands::account::confirm_notification_email,
            commands::account::submit_notification_email_uia,
            commands::account::cancel_notification_email,
            commands::account::enable_email_notifications,
            commands::account::disable_email_notifications,
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
            commands::e2ee::save_secure_backup_recovery_key,
            commands::e2ee::confirm_secure_backup_recovery_key_saved,
            commands::e2ee::export_room_keys,
            commands::e2ee::import_room_keys,
            commands::history_export::history_export_time_zone,
            commands::history_export::export_history,
            commands::history_export::stop_history_export,
            commands::history_export::retry_history_export,
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
            commands::navigation::dismiss_event_navigation_failure,
            commands::navigation::open_pinned_event,
            commands::navigation::open_notification_event,
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
            commands::clipboard_image::read_clipboard_image_png,
            commands::dropped_files::claim_dropped_files,
            commands::dropped_files::read_dropped_file,
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
            commands::profile::cancel_avatar_thumbnail,
            commands::room::leave_room,
            commands::room::forget_room,
            commands::room::set_room_tag,
            commands::room::remove_room_tag,
            commands::room::pin_event,
            commands::room::unpin_event,
            commands::room::refresh_pinned_events,
            commands::room::load_room_settings,
            commands::room::load_space_members,
            commands::room::load_space_children,
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
            commands::views::subscribe_receipt_reader,
            commands::views::receive_receipt_reader,
            commands::views::read_receipt_reader_resource,
            commands::views::update_receipt_reader_window,
            commands::views::observe_receipt_reader_avatars,
            commands::views::ack_receipt_reader,
            commands::views::close_receipt_reader,
            commands::search::submit_search,
            commands::search::close_search,
            commands::search::start_room_crawl,
            commands::search::stop_room_crawl,
            commands::directory::query_directory,
            commands::room::preview_room_address,
            commands::room::create_room,
            commands::room::create_space,
            commands::directory::join_directory_room,
            commands::directory::preview_join_target,
            commands::directory::dismiss_directory_preview,
            commands::room::set_space_child,
            commands::room::check_room_address_availability,
            commands::room::clear_room_address_availability,
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
            // window-destroy path, so Core is shut down exactly once whether
            // the product window was hidden or destroyed,
            // and `ExitRequested` is treated the same for any exit code.
            if let tauri::RunEvent::ExitRequested { api, .. } = &event
                && let Some(core_state) = app.try_state::<CoreRuntimeState>()
            {
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

#[cfg(test)]
#[path = "app_updates/restart_barrier_tests.rs"]
mod restart_barrier_tests;
