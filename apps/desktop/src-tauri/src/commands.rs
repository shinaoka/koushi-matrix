//! Tauri command handlers: transport adapter only.
//!
//! Each handler allocates a `RequestId`, submits a `CoreCommand`, and returns
//! the current `FrontendDesktopSnapshot`. Side-effects (state changes, timeline
//! diffs) flow back to the webview as Tauri events — not as command return
//! values.
//!
//! No Matrix semantics live here. No SDK types. No `matrix_desktop_sdk` calls.
//! (Secret-bearing QA helpers remain behind `#[cfg(any(debug_assertions, test))]`.)

use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(any(debug_assertions, test))]
use std::path::PathBuf;

use matrix_desktop_core::{
    AccountCommand, AccountEvent, AccountKey, AppCommand, CoreCommand, CoreConnection, CoreEvent,
    MediaDownloadSelection, PaginationDirection, RequestId, RoomCommand, RoomEvent, SearchCommand,
    SearchScope, SetAvatarRequest, SyncCommand, TimelineCommand, TimelineKey, TimelineKind,
    UploadMediaKind, UploadMediaRequest,
};
use matrix_desktop_state::{
    AuthSecret, ComposerKeyEvent, ComposerResolvedAction, ComposerResolverContext, ComposerSurface,
    IdentityResetAuthRequest, LoginRequest, PresenceKind, RecoveryRequest, RoomTagKind,
    SessionInfo, SettingsPatch, VerificationCancelReason,
};
#[cfg(any(debug_assertions, test))]
use serde::Deserialize;
#[cfg(any(debug_assertions, test))]
use tauri::Emitter;
use tauri::{AppHandle, Manager, State};

use crate::{
    CoreRuntimeState,
    dto::{FrontendDesktopSnapshot, SearchScopeKind},
};

static NEXT_TRANSACTION_ID: AtomicU64 = AtomicU64::new(1);

#[cfg(any(debug_assertions, test))]
const QA_RECOVERY_PROMPT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const QA_TITLE_ENV: &str = "MATRIX_DESKTOP_QA_TITLE";
const TIMELINE_BACKWARDS_PAGE_EVENT_COUNT: u16 = 30;

// ---- Core command dispatch helpers ----

/// Submit a `CoreCommand` over the command-dispatch connection.
///
/// This is the ONLY way commands leave the Tauri adapter.
/// Uses `tokio::sync::Mutex` so the guard may be held across `.await`.
pub(crate) async fn submit_core_command(
    state: &CoreRuntimeState,
    command: CoreCommand,
) -> Result<(), String> {
    state
        .connection
        .lock()
        .await
        .command(command)
        .await
        .map_err(|e| format!("command submit failed: {e}"))
}

/// Allocate a `RequestId` from the command-dispatch connection.
async fn next_request_id(state: &CoreRuntimeState) -> matrix_desktop_core::RequestId {
    state.connection.lock().await.next_request_id()
}

/// Read the latest `AppStateSnapshot` and convert to `FrontendDesktopSnapshot`.
async fn current_snapshot(state: &CoreRuntimeState) -> Result<FrontendDesktopSnapshot, String> {
    let snapshot = state.connection.lock().await.snapshot();
    Ok(FrontendDesktopSnapshot::from(snapshot))
}

// ---- QA window title ----

fn qa_window_title_enabled() -> bool {
    matches!(
        std::env::var(QA_TITLE_ENV)
            .ok()
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("1" | "true" | "yes")
    )
}

async fn update_qa_window_title_from_state(app: &AppHandle, state: &CoreRuntimeState) {
    if !qa_window_title_enabled() {
        return;
    }
    let snapshot = state.connection.lock().await.snapshot();
    let timeline_items = state.timeline_items_count.load(Ordering::Relaxed);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_title(&qa_window_title_string(&snapshot, timeline_items));
    }
}

pub(crate) fn qa_window_title_string(
    snapshot: &matrix_desktop_state::AppState,
    timeline_items: usize,
) -> String {
    [
        "matrix-desktop qa".to_owned(),
        format!("session={}", qa_session_label(&snapshot.session)),
        format!("sync={}", qa_sync_label(&snapshot.sync)),
        format!("rooms={}", snapshot.rooms.len()),
        format!("spaces={}", snapshot.spaces.len()),
        format!(
            "active_room={}",
            snapshot.navigation.active_room_id.is_some()
        ),
        format!("timeline_subscribed={}", snapshot.timeline.is_subscribed),
        format!("timeline_items={timeline_items}"),
        format!("errors={}", snapshot.errors.len()),
    ]
    .join(" ")
}

fn qa_session_label(session: &matrix_desktop_state::SessionState) -> &'static str {
    use matrix_desktop_state::SessionState;
    match session {
        SessionState::SignedOut => "signedOut",
        SessionState::Restoring => "restoring",
        SessionState::SwitchingAccount { .. } => "switchingAccount",
        SessionState::Authenticating { .. } => "authenticating",
        SessionState::NeedsRecovery { .. } => "needsRecovery",
        SessionState::Recovering { .. } => "recovering",
        SessionState::Ready(_) => "ready",
        SessionState::Locked(_) => "locked",
        SessionState::LoggingOut => "loggingOut",
    }
}

fn qa_sync_label(sync: &matrix_desktop_state::SyncState) -> &'static str {
    match sync {
        matrix_desktop_state::SyncState::Stopped => "stopped",
        matrix_desktop_state::SyncState::Starting => "starting",
        matrix_desktop_state::SyncState::Running => "running",
        matrix_desktop_state::SyncState::Failed { .. } => "failed",
        matrix_desktop_state::SyncState::Reconnecting { .. } => "reconnecting",
    }
}

// ---- Tauri commands ----

#[tauri::command]
pub async fn get_snapshot(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn discover_login_methods(
    homeserver: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    // KNOWN DEFERRED SHIM (tracked, not endorsed): login-flow discovery is
    // currently implicit in LoginPassword, so this command returns the snapshot
    // without driving the `auth` LoginDiscovery state machine. Per
    // REPOSITORY_RULES "State-Machine Discipline" this should become a real core
    // command that transitions auth `unknown -> discovering -> ready/failed` from
    // an SDK homeserver/login-flow query (the `LoginDiscovery` reducer state
    // already exists). It is intentionally left for a focused auth-discovery
    // change rather than deleted, because removing it would drop the homeserver
    // login-method UX (e.g. SSO discovery). Do not extend this shim.
    let _ = homeserver;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn submit_login(
    homeserver: String,
    username: String,
    password: String,
    device_display_name: Option<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let login_request = LoginRequest {
        homeserver,
        username,
        password: AuthSecret::new(password),
        device_display_name,
    };
    submit_login_request(app, state.inner(), login_request).await?;
    current_snapshot(state.inner()).await
}

pub(crate) async fn submit_login_request(
    app: AppHandle,
    state: &CoreRuntimeState,
    login_request: LoginRequest,
) -> Result<(), String> {
    submit_login_and_start_sync(app, state, login_request).await?;
    Ok(())
}

const LOGIN_EVENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const SELECT_ROOM_EVENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const ROOM_OPERATION_EVENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

async fn submit_login_and_start_sync(
    app: AppHandle,
    state: &CoreRuntimeState,
    login_request: LoginRequest,
) -> Result<(), String> {
    // Use a dedicated connection so the event cursor is attached before the
    // login command is submitted and the correlated LoggedIn event cannot be
    // missed by this product path.
    let mut event_conn = state.runtime.attach();
    let login_request_id = event_conn.next_request_id();
    event_conn
        .command(build_submit_login_command(login_request_id, login_request))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;

    wait_for_logged_in_ready(&mut event_conn, login_request_id, LOGIN_EVENT_TIMEOUT).await?;

    let sync_request_id = event_conn.next_request_id();
    event_conn
        .command(build_start_sync_command(sync_request_id))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    update_qa_window_title_from_state(&app, state).await;
    Ok(())
}

async fn wait_for_logged_in_ready(
    event_conn: &mut CoreConnection,
    login_request_id: RequestId,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut logged_in = false;

    loop {
        if logged_in && snapshot_has_ready_session(&event_conn.snapshot()) {
            return Ok(());
        }

        let event = tokio::time::timeout_at(deadline, event_conn.recv_event())
            .await
            .map_err(|_| "login did not complete".to_owned())?;
        match event {
            Ok(CoreEvent::Account(AccountEvent::LoggedIn { request_id, .. }))
                if request_id == login_request_id =>
            {
                logged_in = true;
            }
            Ok(CoreEvent::OperationFailed { request_id, .. }) if request_id == login_request_id => {
                return Err("login failed".to_owned());
            }
            Ok(_) => {}
            Err(_) => return Err("login event stream lagged".to_owned()),
        }
    }
}

fn snapshot_has_ready_session(snapshot: &matrix_desktop_state::AppState) -> bool {
    matches!(
        snapshot.session,
        matrix_desktop_state::SessionState::Ready(_)
    )
}

async fn wait_for_selected_room(
    event_conn: &mut CoreConnection,
    select_request_id: RequestId,
    selected_room_id: &str,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        if snapshot_has_active_room(&event_conn.snapshot(), selected_room_id) {
            return Ok(());
        }

        let event = tokio::time::timeout_at(deadline, event_conn.recv_event())
            .await
            .map_err(|_| "room selection did not complete".to_owned())?;
        match event {
            Ok(CoreEvent::StateChanged(snapshot))
                if snapshot_has_active_room(&snapshot, selected_room_id) =>
            {
                return Ok(());
            }
            Ok(CoreEvent::OperationFailed { request_id, .. })
                if request_id == select_request_id =>
            {
                return Err("room selection failed".to_owned());
            }
            Ok(_) => {}
            Err(_) if snapshot_has_active_room(&event_conn.snapshot(), selected_room_id) => {
                return Ok(());
            }
            Err(_) => return Err("room selection event stream lagged".to_owned()),
        }
    }
}

fn snapshot_has_active_room(snapshot: &matrix_desktop_state::AppState, room_id: &str) -> bool {
    snapshot.navigation.active_room_id.as_deref() == Some(room_id)
}

/// How long the adapter waits for the `SavedSessionsListed` answer before
/// reporting a transport error. The query is a local credential-store read in
/// core, so 5 seconds is generous.
const SAVED_SESSIONS_EVENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[tauri::command]
pub async fn list_saved_sessions(
    state: State<'_, CoreRuntimeState>,
) -> Result<Vec<SessionInfo>, String> {
    // GUI-smoke toggle: skip the keychain-backed query entirely.
    if crate::saved_sessions_disabled_from_env() {
        return Ok(Vec::new());
    }

    // Attach a dedicated connection so (a) the request id belongs to this
    // connection and (b) the broadcast cursor starts BEFORE the command is
    // submitted — the correlated answer cannot be missed.
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(CoreCommand::Account(AccountCommand::QuerySavedSessions {
            request_id,
        }))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;

    let deadline = tokio::time::Instant::now() + SAVED_SESSIONS_EVENT_TIMEOUT;
    loop {
        let event = tokio::time::timeout_at(deadline, event_conn.recv_event())
            .await
            .map_err(|_| "saved sessions could not be loaded".to_owned())?;
        match event {
            Ok(matrix_desktop_core::CoreEvent::Account(
                matrix_desktop_core::AccountEvent::SavedSessionsListed {
                    request_id: ev_id,
                    sessions,
                },
            )) if ev_id == request_id => return Ok(sessions),
            Ok(matrix_desktop_core::CoreEvent::OperationFailed {
                request_id: ev_id, ..
            }) if ev_id == request_id => {
                return Err("saved sessions could not be loaded".to_owned());
            }
            // Unrelated events / lag: keep waiting until the deadline.
            _ => {}
        }
    }
}

#[tauri::command]
pub async fn switch_account(
    homeserver: String,
    user_id: String,
    device_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_switch_account_command(request_id, user_id),
    )
    .await?;
    // AccountKey canonically identifies the account by user_id.
    let _ = (homeserver, device_id);
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn submit_recovery(
    secret: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    submit_recovery_request(app, state.inner(), AuthSecret::new(secret)).await?;
    current_snapshot(state.inner()).await
}

pub(crate) async fn submit_recovery_request(
    app: AppHandle,
    state: &CoreRuntimeState,
    secret: AuthSecret,
) -> Result<(), String> {
    let request_id = next_request_id(state).await;
    submit_core_command(state, build_submit_recovery_command(request_id, secret)).await?;
    update_qa_window_title_from_state(&app, state).await;
    Ok(())
}

#[tauri::command]
pub async fn logout(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(state.inner(), build_logout_command(request_id)).await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn restart_sync(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(state.inner(), build_restart_sync_command(request_id)).await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn update_settings(
    patch: SettingsPatch,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_update_settings_command(request_id, patch),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn bootstrap_cross_signing(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_bootstrap_cross_signing_command(request_id, None),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn enable_key_backup(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(state.inner(), build_enable_key_backup_command(request_id)).await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn accept_verification(
    flow_id: u64,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_accept_verification_command(request_id, flow_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn confirm_sas_verification(
    flow_id: u64,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_confirm_sas_verification_command(request_id, flow_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn cancel_verification(
    flow_id: u64,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_cancel_verification_command(request_id, flow_id, VerificationCancelReason::User),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn reset_identity(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(state.inner(), build_reset_identity_command(request_id)).await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn submit_identity_reset_password(
    flow_id: u64,
    password: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_submit_identity_reset_password_command(
            request_id,
            flow_id,
            AuthSecret::new(password),
        ),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn submit_identity_reset_oauth(
    flow_id: u64,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_submit_identity_reset_oauth_command(request_id, flow_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn resolve_composer_key_action(
    surface: ComposerSurface,
    key_event: ComposerKeyEvent,
    autocomplete_open: bool,
    send_enabled: bool,
    state: State<'_, CoreRuntimeState>,
) -> Result<ComposerResolvedAction, String> {
    let snapshot = state.connection.lock().await.snapshot();
    Ok(matrix_desktop_state::resolve_composer_key_action(
        key_event,
        ComposerResolverContext {
            surface,
            send_shortcut: snapshot.settings.values.keyboard.composer_send_shortcut,
            autocomplete_open,
            send_enabled,
        },
    ))
}

#[tauri::command]
pub async fn select_space(
    space_id: Option<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_select_space_command(request_id, space_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn select_room(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let selected_room_id = room_id.clone();
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_select_room_command(request_id, room_id))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_selected_room(
        &mut event_conn,
        request_id,
        &selected_room_id,
        SELECT_ROOM_EVENT_TIMEOUT,
    )
    .await?;
    let account_key = account_key_from_snapshot(state.inner()).await;
    let subscribe_request_id = event_conn.next_request_id();
    event_conn
        .command(build_subscribe_timeline_command(
            subscribe_request_id,
            account_key,
            selected_room_id,
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn select_search_result(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let selected_room_id = room_id.clone();
    let mut event_conn = state.runtime.attach();

    let close_request_id = event_conn.next_request_id();
    event_conn
        .command(CoreCommand::App(AppCommand::CloseFocusedContext {
            request_id: close_request_id,
        }))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;

    let select_request_id = event_conn.next_request_id();
    event_conn
        .command(build_select_room_command(
            select_request_id,
            room_id.clone(),
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_selected_room(
        &mut event_conn,
        select_request_id,
        &selected_room_id,
        SELECT_ROOM_EVENT_TIMEOUT,
    )
    .await?;

    let account_key = account_key_from_snapshot(state.inner()).await;
    let subscribe_request_id = event_conn.next_request_id();
    event_conn
        .command(build_subscribe_timeline_command(
            subscribe_request_id,
            account_key,
            selected_room_id,
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;

    let open_request_id = event_conn.next_request_id();
    event_conn
        .command(CoreCommand::App(AppCommand::OpenFocusedContext {
            request_id: open_request_id,
            room_id,
            event_id,
        }))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;

    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn close_focused_context(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::App(AppCommand::CloseFocusedContext { request_id }),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn paginate_timeline_backwards(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_paginate_timeline_backwards_command(request_id, account_key, room_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn paginate_thread_timeline_backwards(
    room_id: String,
    root_event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_paginate_thread_timeline_backwards_command(
            request_id,
            account_key,
            room_id,
            root_event_id,
        ),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn send_text(
    room_id: String,
    body: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    if body.trim().is_empty() {
        return current_snapshot(state.inner()).await;
    }

    let transaction_id = format!(
        "desktop-{}",
        NEXT_TRANSACTION_ID.fetch_add(1, Ordering::Relaxed)
    );
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) =
        build_send_text_command(request_id, account_key, room_id, transaction_id, body)
    {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn upload_media(
    room_id: String,
    filename: String,
    mime_type: String,
    bytes: Vec<u8>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    if bytes.is_empty() {
        return current_snapshot(state.inner()).await;
    }

    let transaction_id = format!(
        "desktop-media-{}",
        NEXT_TRANSACTION_ID.fetch_add(1, Ordering::Relaxed)
    );
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) = build_upload_media_command(
        request_id,
        account_key,
        room_id,
        transaction_id,
        filename,
        mime_type,
        bytes,
    ) {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn download_media(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    if event_id.trim().is_empty() {
        return current_snapshot(state.inner()).await;
    }

    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) = build_download_media_command(request_id, account_key, room_id, event_id)
    {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn edit_message(
    room_id: String,
    event_id: String,
    body: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    if body.trim().is_empty() {
        return current_snapshot(state.inner()).await;
    }
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) =
        build_edit_message_command(request_id, account_key, room_id, event_id, body)
    {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn redact_message(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_redact_message_command(request_id, account_key, room_id, event_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn toggle_reaction(
    room_id: String,
    event_id: String,
    reaction_key: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    if reaction_key.is_empty() {
        return current_snapshot(state.inner()).await;
    }

    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) =
        build_toggle_reaction_command(request_id, account_key, room_id, event_id, reaction_key)
    {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn send_read_receipt(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) =
        build_send_read_receipt_command(request_id, account_key, room_id, event_id)
    {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn set_fully_read(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) = build_set_fully_read_command(request_id, account_key, room_id, event_id)
    {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn set_typing(
    room_id: String,
    is_typing: bool,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_set_typing_command(request_id, account_key, room_id, is_typing),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn set_presence(
    presence: PresenceKind,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_set_presence_command(request_id, presence),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn set_display_name(
    display_name: Option<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_set_display_name_command(request_id, display_name),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn set_avatar(
    mime_type: String,
    bytes: Vec<u8>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    if bytes.is_empty() {
        return current_snapshot(state.inner()).await;
    }

    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_set_avatar_command(request_id, mime_type, bytes),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn leave_room(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(state.inner(), build_leave_room_command(request_id, room_id)).await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn forget_room(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_forget_room_command(request_id, room_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn set_room_tag(
    room_id: String,
    tag: RoomTagKind,
    order: Option<f64>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_set_room_tag_command(request_id, room_id, tag, order),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn remove_room_tag(
    room_id: String,
    tag: RoomTagKind,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_remove_room_tag_command(request_id, room_id, tag),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn open_thread(
    room_id: String,
    root_event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    // Thread open/close is Rust-owned product state: drive the reducer's
    // ThreadPaneState through a first-class core command instead of discarding
    // the inputs in a snapshot-only shim.
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::App(AppCommand::OpenThread {
            request_id,
            room_id,
            root_event_id,
        }),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn close_thread(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::App(AppCommand::CloseThread { request_id }),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn submit_search(
    query: String,
    scope: SearchScopeKind,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let search_scope = resolve_search_scope(scope, state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_submit_search_command(request_id, query, search_scope),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

const CREATE_EVENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

async fn wait_for_room_created(
    event_conn: &mut CoreConnection,
    create_request_id: RequestId,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let event = tokio::time::timeout_at(deadline, event_conn.recv_event())
            .await
            .map_err(|_| "room creation did not complete".to_owned())?;
        match event {
            Ok(CoreEvent::Room(RoomEvent::RoomCreated { request_id, .. }))
                if request_id == create_request_id =>
            {
                return Ok(());
            }
            Ok(CoreEvent::OperationFailed { request_id, .. })
                if request_id == create_request_id =>
            {
                return Err("room creation failed".to_owned());
            }
            Ok(_) => {}
            Err(_) => return Err("room creation event stream lagged".to_owned()),
        }
    }
}

async fn wait_for_space_created(
    event_conn: &mut CoreConnection,
    create_request_id: RequestId,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let event = tokio::time::timeout_at(deadline, event_conn.recv_event())
            .await
            .map_err(|_| "space creation did not complete".to_owned())?;
        match event {
            Ok(CoreEvent::Room(RoomEvent::SpaceCreated { request_id, .. }))
                if request_id == create_request_id =>
            {
                return Ok(());
            }
            Ok(CoreEvent::OperationFailed { request_id, .. })
                if request_id == create_request_id =>
            {
                return Err("space creation failed".to_owned());
            }
            Ok(_) => {}
            Err(_) => return Err("space creation event stream lagged".to_owned()),
        }
    }
}

async fn wait_for_room_operation<F>(
    event_conn: &mut CoreConnection,
    operation_request_id: RequestId,
    timeout: std::time::Duration,
    is_success: F,
    timeout_message: &'static str,
    failure_message: &'static str,
) -> Result<(), String>
where
    F: Fn(&RoomEvent, RequestId) -> bool,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let event = tokio::time::timeout_at(deadline, event_conn.recv_event())
            .await
            .map_err(|_| timeout_message.to_owned())?;
        match event {
            Ok(CoreEvent::Room(room_event)) if is_success(&room_event, operation_request_id) => {
                return Ok(());
            }
            Ok(CoreEvent::OperationFailed { request_id, .. })
                if request_id == operation_request_id =>
            {
                return Err(failure_message.to_owned());
            }
            Ok(_) => {}
            Err(_) => return Err("room operation event stream lagged".to_owned()),
        }
    }
}

#[tauri::command]
pub async fn create_room(
    name: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_create_room_command(request_id, name))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_room_created(&mut event_conn, request_id, CREATE_EVENT_TIMEOUT).await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn create_space(
    name: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_create_space_command(request_id, name))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_space_created(&mut event_conn, request_id, CREATE_EVENT_TIMEOUT).await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn set_space_child(
    space_id: String,
    child_room_id: String,
    via_server: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_set_space_child_command(request_id, space_id, child_room_id, via_server),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn accept_invite(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_accept_invite_command(request_id, room_id))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_room_operation(
        &mut event_conn,
        request_id,
        ROOM_OPERATION_EVENT_TIMEOUT,
        |event, expected_request_id| {
            matches!(
                event,
                RoomEvent::InviteAccepted { request_id, .. } if *request_id == expected_request_id
            )
        },
        "invite acceptance did not complete",
        "invite acceptance failed",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn decline_invite(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_decline_invite_command(request_id, room_id))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_room_operation(
        &mut event_conn,
        request_id,
        ROOM_OPERATION_EVENT_TIMEOUT,
        |event, expected_request_id| {
            matches!(
                event,
                RoomEvent::InviteDeclined { request_id, .. } if *request_id == expected_request_id
            )
        },
        "invite decline did not complete",
        "invite decline failed",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn start_direct_message(
    user_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_start_direct_message_command(request_id, user_id))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_room_operation(
        &mut event_conn,
        request_id,
        ROOM_OPERATION_EVENT_TIMEOUT,
        |event, expected_request_id| {
            matches!(
                event,
                RoomEvent::DirectMessageStarted { request_id, .. } if *request_id == expected_request_id
            )
        },
        "direct message start did not complete",
        "direct message start failed",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn invite_user(
    room_id: String,
    user_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_invite_user_command(request_id, room_id, user_id))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_room_operation(
        &mut event_conn,
        request_id,
        ROOM_OPERATION_EVENT_TIMEOUT,
        |event, expected_request_id| {
            matches!(
                event,
                RoomEvent::UserInvited { request_id, .. } if *request_id == expected_request_id
            )
        },
        "user invite did not complete",
        "user invite failed",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn set_composer_reply_target(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::App(AppCommand::SetComposerReplyTarget {
            request_id,
            room_id,
            event_id,
        }),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn cancel_composer_reply(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::App(AppCommand::CancelComposerReply { request_id }),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn set_thread_composer_draft(
    room_id: String,
    root_event_id: String,
    draft: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_set_thread_composer_draft_command(request_id, room_id, root_event_id, draft),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn send_reply(
    room_id: String,
    in_reply_to_event_id: String,
    body: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    if body.trim().is_empty() {
        return current_snapshot(state.inner()).await;
    }

    let transaction_id = format!(
        "desktop-{}",
        NEXT_TRANSACTION_ID.fetch_add(1, Ordering::Relaxed)
    );
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) = build_send_reply_command(
        request_id,
        account_key,
        room_id,
        transaction_id,
        in_reply_to_event_id,
        body,
    ) {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn send_thread_reply(
    room_id: String,
    root_event_id: String,
    body: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    if body.trim().is_empty() {
        return current_snapshot(state.inner()).await;
    }

    let transaction_id = format!(
        "desktop-{}",
        NEXT_TRANSACTION_ID.fetch_add(1, Ordering::Relaxed)
    );
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) = build_send_thread_reply_command(
        request_id,
        account_key,
        room_id,
        root_event_id,
        transaction_id,
        body,
    ) {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

// ---- Helpers ----

pub(crate) fn build_submit_login_command(
    request_id: matrix_desktop_core::RequestId,
    login_request: LoginRequest,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::LoginPassword {
        request_id,
        request: login_request,
    })
}

pub(crate) fn build_switch_account_command(
    request_id: matrix_desktop_core::RequestId,
    user_id: String,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::SwitchAccount {
        request_id,
        account_key: AccountKey(user_id),
    })
}

pub(crate) fn build_submit_recovery_command(
    request_id: matrix_desktop_core::RequestId,
    secret: AuthSecret,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::SubmitRecovery {
        request_id,
        request: RecoveryRequest { secret },
    })
}

pub(crate) fn build_logout_command(request_id: matrix_desktop_core::RequestId) -> CoreCommand {
    CoreCommand::Account(AccountCommand::Logout { request_id })
}

pub(crate) fn build_restart_sync_command(
    request_id: matrix_desktop_core::RequestId,
) -> CoreCommand {
    CoreCommand::Sync(SyncCommand::Restart { request_id })
}

pub(crate) fn build_update_settings_command(
    request_id: matrix_desktop_core::RequestId,
    patch: SettingsPatch,
) -> CoreCommand {
    CoreCommand::App(AppCommand::UpdateSettings { request_id, patch })
}

pub(crate) fn build_bootstrap_cross_signing_command(
    request_id: matrix_desktop_core::RequestId,
    auth: Option<AuthSecret>,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::BootstrapCrossSigning { request_id, auth })
}

pub(crate) fn build_enable_key_backup_command(
    request_id: matrix_desktop_core::RequestId,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::EnableKeyBackup {
        request_id,
        passphrase: None,
    })
}

pub(crate) fn build_accept_verification_command(
    request_id: matrix_desktop_core::RequestId,
    flow_id: u64,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::AcceptVerification {
        request_id,
        flow_id,
    })
}

pub(crate) fn build_confirm_sas_verification_command(
    request_id: matrix_desktop_core::RequestId,
    flow_id: u64,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::ConfirmSasVerification {
        request_id,
        flow_id,
    })
}

pub(crate) fn build_cancel_verification_command(
    request_id: matrix_desktop_core::RequestId,
    flow_id: u64,
    reason: VerificationCancelReason,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::CancelVerification {
        request_id,
        flow_id,
        reason,
    })
}

pub(crate) fn build_reset_identity_command(
    request_id: matrix_desktop_core::RequestId,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::ResetIdentity { request_id })
}

pub(crate) fn build_submit_identity_reset_password_command(
    request_id: matrix_desktop_core::RequestId,
    flow_id: u64,
    password: AuthSecret,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::SubmitIdentityResetAuth {
        request_id,
        flow_id,
        request: IdentityResetAuthRequest::UiaaPassword { password },
    })
}

pub(crate) fn build_submit_identity_reset_oauth_command(
    request_id: matrix_desktop_core::RequestId,
    flow_id: u64,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::SubmitIdentityResetAuth {
        request_id,
        flow_id,
        request: IdentityResetAuthRequest::OAuthApproved,
    })
}

pub(crate) fn build_start_sync_command(request_id: matrix_desktop_core::RequestId) -> CoreCommand {
    CoreCommand::Sync(SyncCommand::Start { request_id })
}

pub(crate) fn build_select_space_command(
    request_id: matrix_desktop_core::RequestId,
    space_id: Option<String>,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::SelectSpace {
        request_id,
        space_id,
    })
}

pub(crate) fn build_select_room_command(
    request_id: matrix_desktop_core::RequestId,
    room_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::SelectRoom {
        request_id,
        room_id,
    })
}

fn build_timeline_key(account_key: AccountKey, room_id: String) -> TimelineKey {
    TimelineKey {
        account_key,
        kind: TimelineKind::Room { room_id },
    }
}

pub(crate) fn build_subscribe_timeline_command(
    request_id: matrix_desktop_core::RequestId,
    account_key: AccountKey,
    room_id: String,
) -> CoreCommand {
    CoreCommand::Timeline(TimelineCommand::Subscribe {
        request_id,
        key: build_timeline_key(account_key, room_id),
    })
}

#[cfg(test)]
pub(crate) fn build_subscribe_focused_timeline_command(
    request_id: matrix_desktop_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
) -> CoreCommand {
    CoreCommand::Timeline(TimelineCommand::Subscribe {
        request_id,
        key: TimelineKey {
            account_key,
            kind: TimelineKind::Focused { room_id, event_id },
        },
    })
}

pub(crate) fn build_paginate_timeline_backwards_command(
    request_id: matrix_desktop_core::RequestId,
    account_key: AccountKey,
    room_id: String,
) -> CoreCommand {
    CoreCommand::Timeline(TimelineCommand::Paginate {
        request_id,
        key: build_timeline_key(account_key, room_id),
        direction: PaginationDirection::Backward,
        event_count: TIMELINE_BACKWARDS_PAGE_EVENT_COUNT,
    })
}

pub(crate) fn build_paginate_thread_timeline_backwards_command(
    request_id: matrix_desktop_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    root_event_id: String,
) -> CoreCommand {
    CoreCommand::Timeline(TimelineCommand::Paginate {
        request_id,
        key: TimelineKey {
            account_key,
            kind: TimelineKind::Thread {
                room_id,
                root_event_id,
            },
        },
        direction: PaginationDirection::Backward,
        event_count: TIMELINE_BACKWARDS_PAGE_EVENT_COUNT,
    })
}

pub(crate) fn build_send_text_command(
    request_id: matrix_desktop_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    transaction_id: String,
    body: String,
) -> Option<CoreCommand> {
    if body.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::SendText {
        request_id,
        key: build_timeline_key(account_key, room_id),
        transaction_id,
        body,
    }))
}

pub(crate) fn build_upload_media_command(
    request_id: matrix_desktop_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    transaction_id: String,
    filename: String,
    mime_type: String,
    bytes: Vec<u8>,
) -> Option<CoreCommand> {
    if bytes.is_empty() {
        return None;
    }
    let filename = match filename.trim() {
        "" => "attachment".to_owned(),
        value => value.to_owned(),
    };
    let mime_type = match mime_type.trim() {
        "" => "application/octet-stream".to_owned(),
        value => value.to_owned(),
    };
    let kind = if mime_type.to_ascii_lowercase().starts_with("image/") {
        UploadMediaKind::Image {
            width: None,
            height: None,
        }
    } else {
        UploadMediaKind::File
    };

    Some(CoreCommand::Timeline(TimelineCommand::UploadAndSendMedia {
        request_id,
        key: build_timeline_key(account_key, room_id),
        transaction_id,
        request: UploadMediaRequest {
            filename,
            mime_type,
            bytes,
            kind,
            caption: None,
        },
    }))
}

pub(crate) fn build_download_media_command(
    request_id: matrix_desktop_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
) -> Option<CoreCommand> {
    if event_id.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::DownloadMedia {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
        selection: MediaDownloadSelection::File,
    }))
}

pub(crate) fn build_edit_message_command(
    request_id: matrix_desktop_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
    body: String,
) -> Option<CoreCommand> {
    if body.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::EditText {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
        body,
    }))
}

pub(crate) fn build_redact_message_command(
    request_id: matrix_desktop_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
) -> CoreCommand {
    CoreCommand::Timeline(TimelineCommand::Redact {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
    })
}

pub(crate) fn build_toggle_reaction_command(
    request_id: matrix_desktop_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
    reaction_key: String,
) -> Option<CoreCommand> {
    if reaction_key.is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::ToggleReaction {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
        reaction_key,
    }))
}

pub(crate) fn build_send_read_receipt_command(
    request_id: matrix_desktop_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
) -> Option<CoreCommand> {
    if event_id.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::SendReadReceipt {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
    }))
}

pub(crate) fn build_set_fully_read_command(
    request_id: matrix_desktop_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
) -> Option<CoreCommand> {
    if event_id.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::SetFullyRead {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
    }))
}

pub(crate) fn build_set_typing_command(
    request_id: matrix_desktop_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    is_typing: bool,
) -> CoreCommand {
    CoreCommand::Timeline(TimelineCommand::SetTyping {
        request_id,
        key: build_timeline_key(account_key, room_id),
        is_typing,
    })
}

pub(crate) fn build_set_presence_command(
    request_id: matrix_desktop_core::RequestId,
    presence: PresenceKind,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::SetPresence {
        request_id,
        presence,
    })
}

pub(crate) fn build_set_display_name_command(
    request_id: matrix_desktop_core::RequestId,
    display_name: Option<String>,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::SetDisplayName {
        request_id,
        display_name,
    })
}

pub(crate) fn build_set_avatar_command(
    request_id: matrix_desktop_core::RequestId,
    mime_type: String,
    bytes: Vec<u8>,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::SetAvatar {
        request_id,
        request: SetAvatarRequest { mime_type, bytes },
    })
}

pub(crate) fn build_leave_room_command(
    request_id: matrix_desktop_core::RequestId,
    room_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::LeaveRoom {
        request_id,
        room_id,
    })
}

pub(crate) fn build_forget_room_command(
    request_id: matrix_desktop_core::RequestId,
    room_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::ForgetRoom {
        request_id,
        room_id,
    })
}

pub(crate) fn build_set_room_tag_command(
    request_id: matrix_desktop_core::RequestId,
    room_id: String,
    tag: RoomTagKind,
    order: Option<f64>,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::SetTag {
        request_id,
        room_id,
        tag,
        order,
    })
}

pub(crate) fn build_remove_room_tag_command(
    request_id: matrix_desktop_core::RequestId,
    room_id: String,
    tag: RoomTagKind,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::RemoveTag {
        request_id,
        room_id,
        tag,
    })
}

pub(crate) fn build_submit_search_command(
    request_id: matrix_desktop_core::RequestId,
    query: String,
    scope: SearchScope,
) -> CoreCommand {
    CoreCommand::Search(SearchCommand::Query {
        request_id,
        query,
        scope,
    })
}

pub(crate) fn build_create_room_command(
    request_id: matrix_desktop_core::RequestId,
    name: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::CreateRoom {
        request_id,
        name,
        encrypted: false,
    })
}

pub(crate) fn build_create_space_command(
    request_id: matrix_desktop_core::RequestId,
    name: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::CreateSpace { request_id, name })
}

pub(crate) fn build_set_space_child_command(
    request_id: matrix_desktop_core::RequestId,
    space_id: String,
    child_room_id: String,
    via_server: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::SetSpaceChild {
        request_id,
        space_id,
        child_room_id,
        via_server,
    })
}

pub(crate) fn build_accept_invite_command(
    request_id: matrix_desktop_core::RequestId,
    room_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::AcceptInvite {
        request_id,
        room_id,
    })
}

pub(crate) fn build_decline_invite_command(
    request_id: matrix_desktop_core::RequestId,
    room_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::DeclineInvite {
        request_id,
        room_id,
    })
}

pub(crate) fn build_start_direct_message_command(
    request_id: matrix_desktop_core::RequestId,
    user_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::StartDirectMessage {
        request_id,
        user_id,
    })
}

pub(crate) fn build_invite_user_command(
    request_id: matrix_desktop_core::RequestId,
    room_id: String,
    user_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::InviteUser {
        request_id,
        room_id,
        user_id,
    })
}

pub(crate) fn build_send_reply_command(
    request_id: matrix_desktop_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    transaction_id: String,
    in_reply_to_event_id: String,
    body: String,
) -> Option<CoreCommand> {
    if body.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::SendReply {
        request_id,
        key: build_timeline_key(account_key, room_id),
        transaction_id,
        in_reply_to_event_id,
        body,
    }))
}

pub(crate) fn build_set_thread_composer_draft_command(
    request_id: matrix_desktop_core::RequestId,
    room_id: String,
    root_event_id: String,
    draft: String,
) -> CoreCommand {
    CoreCommand::App(AppCommand::SetThreadComposerDraft {
        request_id,
        room_id,
        root_event_id,
        draft,
    })
}

pub(crate) fn build_send_thread_reply_command(
    request_id: matrix_desktop_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    root_event_id: String,
    transaction_id: String,
    body: String,
) -> Option<CoreCommand> {
    if body.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::SendReply {
        request_id,
        key: TimelineKey {
            account_key,
            kind: TimelineKind::Thread {
                room_id,
                root_event_id: root_event_id.clone(),
            },
        },
        transaction_id,
        in_reply_to_event_id: root_event_id,
        body,
    }))
}

/// Derive the `AccountKey` for the currently active session from the snapshot.
///
/// Returns an empty key if no session is active (commands that require a Ready
/// session will be rejected by `AppActor::requires_ready_session`).
async fn account_key_from_snapshot(state: &CoreRuntimeState) -> AccountKey {
    let snapshot = state.connection.lock().await.snapshot();
    match &snapshot.session {
        matrix_desktop_state::SessionState::Ready(info)
        | matrix_desktop_state::SessionState::NeedsRecovery { info, .. }
        | matrix_desktop_state::SessionState::Recovering { info, .. }
        | matrix_desktop_state::SessionState::Locked(info)
        | matrix_desktop_state::SessionState::SwitchingAccount { info } => {
            AccountKey(info.user_id.clone())
        }
        _ => AccountKey(String::new()),
    }
}

fn resolve_search_scope_from_active_room(
    scope: SearchScopeKind,
    active_room_id: Option<String>,
) -> SearchScope {
    match scope {
        SearchScopeKind::CurrentRoom => active_room_id
            .map(|room_id| SearchScope::Room { room_id })
            .unwrap_or(SearchScope::Global),
        SearchScopeKind::CurrentSpace | SearchScopeKind::AllRooms => SearchScope::Global,
        SearchScopeKind::Dms => SearchScope::Global,
    }
}

async fn resolve_search_scope(
    scope: SearchScopeKind,
    state: &CoreRuntimeState,
) -> matrix_desktop_core::SearchScope {
    let snapshot = state.connection.lock().await.snapshot();
    resolve_search_scope_from_active_room(scope, snapshot.navigation.active_room_id)
}

// ---- QA login pipe (debug/test only) ----

#[cfg(any(debug_assertions, test))]
#[derive(Deserialize)]
struct QaLoginPipePayload {
    homeserver: String,
    username: String,
    password: String,
    device_display_name: Option<String>,
    recovery_secret: Option<String>,
}

#[cfg(any(debug_assertions, test))]
#[derive(Debug)]
pub(crate) struct QaLoginPipeRequest {
    pub login: LoginRequest,
    pub recovery_secret: Option<AuthSecret>,
}

#[cfg(any(debug_assertions, test))]
pub(crate) fn parse_qa_login_pipe_payload(payload: &str) -> Result<QaLoginPipeRequest, String> {
    let payload: QaLoginPipePayload =
        serde_json::from_str(payload).map_err(|_| "QA login payload was invalid".to_owned())?;
    if payload.homeserver.trim().is_empty()
        || payload.username.trim().is_empty()
        || payload.password.is_empty()
    {
        return Err("QA login payload was incomplete".to_owned());
    }

    Ok(QaLoginPipeRequest {
        login: LoginRequest {
            homeserver: payload.homeserver,
            username: payload.username,
            password: AuthSecret::new(payload.password),
            device_display_name: payload.device_display_name,
        },
        recovery_secret: payload
            .recovery_secret
            .filter(|secret| !secret.trim().is_empty())
            .map(AuthSecret::new),
    })
}

#[cfg(any(debug_assertions, test))]
pub(crate) fn spawn_qa_login_pipe_reader(app: AppHandle, pipe_path: PathBuf) {
    tauri::async_runtime::spawn(async move {
        let payload = match read_qa_login_pipe(pipe_path).await {
            Ok(payload) => payload,
            Err(message) => {
                record_qa_login_failure(&app, &message).await;
                return;
            }
        };
        let request = match parse_qa_login_pipe_payload(&payload) {
            Ok(request) => request,
            Err(message) => {
                record_qa_login_failure(&app, &message).await;
                return;
            }
        };
        let state = app.state::<CoreRuntimeState>();
        if let Err(message) = submit_login_request(app.clone(), state.inner(), request.login).await
        {
            record_qa_login_failure(&app, &message).await;
            return;
        }
        if let Some(recovery_secret) = request.recovery_secret {
            let state = app.state::<CoreRuntimeState>();
            if let Err(message) =
                wait_for_qa_recovery_prompt(&app, state.inner(), QA_RECOVERY_PROMPT_TIMEOUT).await
            {
                record_qa_login_failure(&app, &message).await;
                return;
            }
            let state = app.state::<CoreRuntimeState>();
            if let Err(message) =
                submit_recovery_request(app.clone(), state.inner(), recovery_secret).await
            {
                record_qa_login_failure(&app, &message).await;
            }
        }
    });
}

#[cfg(any(debug_assertions, test))]
async fn read_qa_login_pipe(pipe_path: PathBuf) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::read_to_string(pipe_path).map_err(|_| "QA login pipe could not be read".to_owned())
    })
    .await
    .map_err(|_| "QA login pipe reader failed".to_owned())?
}

#[cfg(any(debug_assertions, test))]
async fn record_qa_login_failure(app: &AppHandle, message: &str) {
    // Emit a QA title update so the harness sees `session=signedOut`.
    let state = app.state::<CoreRuntimeState>();
    update_qa_window_title_from_state(app, state.inner()).await;
    // Also emit a discrete error event.
    let _ = app.emit(
        crate::CORE_EVENT_NAME,
        serde_json::json!({
            "kind": "OperationFailed",
            "request_id": null,
            "failure": { "kind": "LoginFailed", "message": message },
        }),
    );
}

#[cfg(any(debug_assertions, test))]
async fn wait_for_qa_recovery_prompt(
    app: &AppHandle,
    state: &CoreRuntimeState,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let started_at = std::time::Instant::now();
    while started_at.elapsed() < timeout {
        let snapshot = state.connection.lock().await.snapshot();
        if qa_recovery_prompt_is_available(&snapshot) {
            update_qa_window_title_from_state(app, state).await;
            return Ok(());
        }
        update_qa_window_title_from_state(app, state).await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Err("QA recovery prompt did not become available".to_owned())
}

#[cfg(any(debug_assertions, test))]
pub(crate) fn qa_recovery_prompt_is_available(state: &matrix_desktop_state::AppState) -> bool {
    matches!(
        state.session,
        matrix_desktop_state::SessionState::NeedsRecovery { .. }
    )
}

// ---- QA control pipe (debug/test only) ----
//
// A newline-delimited JSON control channel that lets unattended GUI smoke drive
// a clean logout after a real login, so no stale device survives the run. This
// mirrors the QA login pipe: it carries no secrets, only control commands, and
// is gated to debug/test builds (release builds never read the env var).

#[cfg(any(debug_assertions, test))]
#[derive(Deserialize)]
struct QaControlPipeCommand {
    command: String,
}

/// Parsed QA control command. Only logout is supported today; unknown commands
/// are ignored by the reader rather than treated as failures.
#[cfg(any(debug_assertions, test))]
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum QaControlCommand {
    Logout,
    Unknown(String),
}

#[cfg(any(debug_assertions, test))]
pub(crate) fn parse_qa_control_pipe_line(line: &str) -> Result<QaControlCommand, String> {
    let parsed: QaControlPipeCommand =
        serde_json::from_str(line).map_err(|_| "QA control command was invalid".to_owned())?;
    Ok(match parsed.command.as_str() {
        "logout" => QaControlCommand::Logout,
        other => QaControlCommand::Unknown(other.to_owned()),
    })
}

#[cfg(any(debug_assertions, test))]
pub(crate) fn spawn_qa_control_pipe_reader(app: AppHandle, pipe_path: PathBuf) {
    tauri::async_runtime::spawn(async move {
        let contents = match read_qa_control_pipe(pipe_path).await {
            Ok(contents) => contents,
            Err(message) => {
                record_qa_login_failure(&app, &message).await;
                return;
            }
        };
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match parse_qa_control_pipe_line(line) {
                Ok(QaControlCommand::Logout) => {
                    let state = app.state::<CoreRuntimeState>();
                    let request_id = next_request_id(state.inner()).await;
                    if let Err(message) =
                        submit_core_command(state.inner(), build_logout_command(request_id)).await
                    {
                        record_qa_login_failure(&app, &message).await;
                        continue;
                    }
                    // Surface the post-logout state in the QA window title so the
                    // smoke harness can wait for `session=signedOut`.
                    update_qa_window_title_from_state(&app, state.inner()).await;
                }
                Ok(QaControlCommand::Unknown(_)) => {
                    // Forward-compatible: ignore commands we do not recognise.
                }
                Err(message) => {
                    record_qa_login_failure(&app, &message).await;
                }
            }
        }
    });
}

#[cfg(any(debug_assertions, test))]
async fn read_qa_control_pipe(pipe_path: PathBuf) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::read_to_string(pipe_path)
            .map_err(|_| "QA control pipe could not be read".to_owned())
    })
    .await
    .map_err(|_| "QA control pipe reader failed".to_owned())?
}

#[cfg(test)]
mod tests {
    use matrix_desktop_core::AccountKey;
    use matrix_desktop_core::{
        AccountCommand, AppCommand, CoreCommand, MediaDownloadSelection, PaginationDirection,
        RoomCommand, SearchCommand, SearchScope, SyncCommand, TimelineCommand, UploadMediaKind,
    };
    use matrix_desktop_state::{
        AppState, AuthSecret, IdentityResetAuthRequest, LoginRequest, SessionInfo, SessionState,
        VerificationCancelReason,
    };
    use matrix_desktop_state::{
        AppearanceSettings, LocaleSettings, SettingsPatch, TextDirectionPreference, ThemePreference,
    };

    use super::QaControlCommand;
    use super::SearchScopeKind;
    use super::{
        build_accept_invite_command, build_accept_verification_command,
        build_bootstrap_cross_signing_command, build_cancel_verification_command,
        build_confirm_sas_verification_command, build_create_room_command,
        build_create_space_command, build_decline_invite_command, build_download_media_command,
        build_edit_message_command, build_enable_key_backup_command, build_forget_room_command,
        build_invite_user_command, build_leave_room_command, build_logout_command,
        build_paginate_thread_timeline_backwards_command,
        build_paginate_timeline_backwards_command, build_redact_message_command,
        build_remove_room_tag_command, build_reset_identity_command, build_restart_sync_command,
        build_select_room_command, build_select_space_command, build_send_read_receipt_command,
        build_send_reply_command, build_send_text_command, build_send_thread_reply_command,
        build_set_avatar_command, build_set_display_name_command, build_set_fully_read_command,
        build_set_presence_command, build_set_room_tag_command, build_set_space_child_command,
        build_set_thread_composer_draft_command, build_set_typing_command,
        build_start_direct_message_command,
        build_submit_identity_reset_oauth_command, build_submit_identity_reset_password_command,
        build_submit_login_command, build_submit_recovery_command, build_submit_search_command,
        build_subscribe_focused_timeline_command, build_subscribe_timeline_command,
        build_switch_account_command, build_toggle_reaction_command, build_update_settings_command,
        build_upload_media_command, parse_qa_control_pipe_line, parse_qa_login_pipe_payload,
        qa_recovery_prompt_is_available, qa_window_title_string,
        resolve_search_scope_from_active_room,
    };
    use matrix_desktop_state::{PresenceKind, RoomSummary, RoomTagKind, RoomTags};

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
    fn qa_control_pipe_line_parses_logout_and_ignores_unknown_commands() {
        assert_eq!(
            parse_qa_control_pipe_line(r#"{"command":"logout"}"#).expect("logout should parse"),
            QaControlCommand::Logout
        );
        assert_eq!(
            parse_qa_control_pipe_line(r#"{"command":"focus"}"#).expect("unknown should parse"),
            QaControlCommand::Unknown("focus".to_owned())
        );
        assert!(parse_qa_control_pipe_line("not json").is_err());
    }

    #[test]
    fn qa_control_logout_builds_account_logout_command() {
        // The control pipe must reuse the same logout core command the manual
        // logout button submits — no bespoke logout path.
        match build_logout_command(fake_request_id(99)) {
            CoreCommand::Account(AccountCommand::Logout { request_id }) => {
                assert_eq!(request_id, fake_request_id(99));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn qa_recovery_prompt_available_iff_needs_recovery() {
        let mut state = AppState::default();
        assert!(!qa_recovery_prompt_is_available(&state));

        state.session = SessionState::NeedsRecovery {
            info: SessionInfo {
                homeserver: "https://matrix.example.org".to_owned(),
                user_id: "@user:example.org".to_owned(),
                device_id: "DEVICE".to_owned(),
            },
            methods: vec![],
        };
        assert!(qa_recovery_prompt_is_available(&state));
    }

    #[test]
    fn qa_window_title_reflects_session_sync_room_and_timeline_counts() {
        let mut snapshot = AppState::default();
        snapshot.rooms = vec![
            RoomSummary {
                room_id: "!room1:example.org".to_owned(),
                display_name: "Room 1".to_owned(),
                avatar: None,
                is_dm: false,
                tags: RoomTags::default(),
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                parent_space_ids: vec![],
            },
            RoomSummary {
                room_id: "!room2:example.org".to_owned(),
                display_name: "Room 2".to_owned(),
                avatar: None,
                is_dm: false,
                tags: RoomTags::default(),
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                parent_space_ids: vec![],
            },
        ];

        let title = qa_window_title_string(&snapshot, 42);

        assert!(title.contains("session=signedOut"));
        assert!(title.contains("sync=stopped"));
        assert!(title.contains("rooms=2"));
        assert!(title.contains("timeline_items=42"));
    }

    fn fake_request_id(sequence: u64) -> matrix_desktop_core::RequestId {
        matrix_desktop_core::RequestId {
            connection_id: matrix_desktop_core::RuntimeConnectionId(7),
            sequence,
        }
    }

    #[test]
    fn tauri_command_routes_build_expected_core_commands() {
        let active_account_key = AccountKey("@alice:example.org".to_owned());
        let room_id = "!room:example.org".to_owned();
        let transaction_id = "desktop-1".to_owned();
        let body = "body with visible content".to_owned();
        let edit_body = "updated body".to_owned();
        let query = "search terms".to_owned();

        match build_submit_login_command(
            fake_request_id(1),
            LoginRequest {
                homeserver: "https://matrix.example.org".to_owned(),
                username: "alice".to_owned(),
                password: AuthSecret::new("password-123"),
                device_display_name: Some("Laptop".to_owned()),
            },
        ) {
            CoreCommand::Account(AccountCommand::LoginPassword {
                request_id,
                request,
            }) => {
                assert_eq!(request_id, fake_request_id(1));
                assert_eq!(request.homeserver, "https://matrix.example.org");
                assert_eq!(request.username, "alice");
                assert_eq!(request.password.expose_secret(), "password-123");
                assert_eq!(request.device_display_name.as_deref(), Some("Laptop"));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_switch_account_command(fake_request_id(2), "@bob:example.org".to_owned()) {
            CoreCommand::Account(AccountCommand::SwitchAccount {
                request_id,
                account_key,
            }) => {
                assert_eq!(request_id, fake_request_id(2));
                assert_eq!(
                    account_key,
                    matrix_desktop_core::AccountKey("@bob:example.org".to_owned())
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_submit_recovery_command(fake_request_id(3), AuthSecret::new("recovery-123")) {
            CoreCommand::Account(AccountCommand::SubmitRecovery {
                request_id,
                request,
            }) => {
                assert_eq!(request_id, fake_request_id(3));
                assert_eq!(request.secret.expose_secret(), "recovery-123");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_logout_command(fake_request_id(4)) {
            CoreCommand::Account(AccountCommand::Logout { request_id }) => {
                assert_eq!(request_id, fake_request_id(4));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_restart_sync_command(fake_request_id(5)) {
            CoreCommand::Sync(SyncCommand::Restart { request_id }) => {
                assert_eq!(request_id, fake_request_id(5));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_select_space_command(fake_request_id(6), Some("!space:example.org".to_owned()))
        {
            CoreCommand::Room(RoomCommand::SelectSpace {
                request_id,
                space_id,
            }) => {
                assert_eq!(request_id, fake_request_id(6));
                assert_eq!(space_id.as_deref(), Some("!space:example.org"));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_select_room_command(fake_request_id(7), room_id.clone()) {
            CoreCommand::Room(RoomCommand::SelectRoom {
                request_id,
                room_id: route_room_id,
            }) => {
                assert_eq!(request_id, fake_request_id(7));
                assert_eq!(route_room_id, room_id);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_subscribe_timeline_command(
            fake_request_id(8),
            active_account_key.clone(),
            room_id.clone(),
        ) {
            CoreCommand::Timeline(TimelineCommand::Subscribe { request_id, key }) => {
                assert_eq!(request_id, fake_request_id(8));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    matrix_desktop_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_paginate_timeline_backwards_command(
            fake_request_id(9),
            active_account_key.clone(),
            room_id.clone(),
        ) {
            CoreCommand::Timeline(TimelineCommand::Paginate {
                request_id,
                key,
                direction,
                event_count,
            }) => {
                assert_eq!(request_id, fake_request_id(9));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    matrix_desktop_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(direction, PaginationDirection::Backward);
                assert_eq!(event_count, 30);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_send_text_command(
            fake_request_id(10),
            active_account_key.clone(),
            room_id.clone(),
            transaction_id.clone(),
            body.clone(),
        )
        .expect("send_text should build a command")
        {
            CoreCommand::Timeline(TimelineCommand::SendText {
                request_id,
                key,
                transaction_id: route_transaction_id,
                body: route_body,
            }) => {
                assert_eq!(request_id, fake_request_id(10));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    matrix_desktop_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(route_transaction_id, transaction_id);
                assert_eq!(route_body, body);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_upload_media_command(
            fake_request_id(25),
            active_account_key.clone(),
            room_id.clone(),
            "desktop-media-1".to_owned(),
            "report.pdf".to_owned(),
            "application/pdf".to_owned(),
            vec![1, 2, 3, 4],
        )
        .expect("upload_media should build a command")
        {
            CoreCommand::Timeline(TimelineCommand::UploadAndSendMedia {
                request_id,
                key,
                transaction_id,
                request,
            }) => {
                assert_eq!(request_id, fake_request_id(25));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    matrix_desktop_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(transaction_id, "desktop-media-1");
                assert_eq!(request.filename, "report.pdf");
                assert_eq!(request.mime_type, "application/pdf");
                assert_eq!(request.bytes, vec![1, 2, 3, 4]);
                assert_eq!(request.kind, UploadMediaKind::File);
                assert_eq!(request.caption, None);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_upload_media_command(
            fake_request_id(26),
            active_account_key.clone(),
            room_id.clone(),
            "desktop-media-2".to_owned(),
            "photo.png".to_owned(),
            "image/png".to_owned(),
            vec![9],
        )
        .expect("image upload_media should build a command")
        {
            CoreCommand::Timeline(TimelineCommand::UploadAndSendMedia { request, .. }) => {
                assert_eq!(
                    request.kind,
                    UploadMediaKind::Image {
                        width: None,
                        height: None
                    }
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_download_media_command(
            fake_request_id(27),
            active_account_key.clone(),
            room_id.clone(),
            "$media-event".to_owned(),
        )
        .expect("download_media should build a command")
        {
            CoreCommand::Timeline(TimelineCommand::DownloadMedia {
                request_id,
                key,
                event_id,
                selection,
            }) => {
                assert_eq!(request_id, fake_request_id(27));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    matrix_desktop_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(event_id, "$media-event");
                assert_eq!(selection, MediaDownloadSelection::File);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_edit_message_command(
            fake_request_id(11),
            active_account_key.clone(),
            room_id.clone(),
            "$event".to_owned(),
            edit_body.clone(),
        )
        .expect("edit_message should build a command")
        {
            CoreCommand::Timeline(TimelineCommand::EditText {
                request_id,
                key,
                event_id,
                body: route_body,
            }) => {
                assert_eq!(request_id, fake_request_id(11));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    matrix_desktop_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(event_id, "$event");
                assert_eq!(route_body, edit_body);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_redact_message_command(
            fake_request_id(12),
            active_account_key.clone(),
            room_id.clone(),
            "$event".to_owned(),
        ) {
            CoreCommand::Timeline(TimelineCommand::Redact {
                request_id,
                key,
                event_id,
            }) => {
                assert_eq!(request_id, fake_request_id(12));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    matrix_desktop_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(event_id, "$event");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_toggle_reaction_command(
            fake_request_id(13),
            active_account_key.clone(),
            room_id.clone(),
            "$event".to_owned(),
            "👍".to_owned(),
        )
        .expect("toggle_reaction should build a command")
        {
            CoreCommand::Timeline(TimelineCommand::ToggleReaction {
                request_id,
                key,
                event_id,
                reaction_key,
            }) => {
                assert_eq!(request_id, fake_request_id(13));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    matrix_desktop_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(event_id, "$event");
                assert_eq!(reaction_key, "👍");
                let debug = format!(
                    "{:?}",
                    CoreCommand::Timeline(TimelineCommand::ToggleReaction {
                        request_id: fake_request_id(13),
                        key,
                        event_id,
                        reaction_key,
                    })
                );
                assert!(!debug.contains("👍"), "{debug}");
                assert!(!debug.contains("$event"), "{debug}");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_send_read_receipt_command(
            fake_request_id(28),
            active_account_key.clone(),
            room_id.clone(),
            "$receipt-event".to_owned(),
        )
        .expect("send_read_receipt should build a command")
        {
            CoreCommand::Timeline(TimelineCommand::SendReadReceipt {
                request_id,
                key,
                event_id,
            }) => {
                assert_eq!(request_id, fake_request_id(28));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    matrix_desktop_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(event_id, "$receipt-event");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_set_fully_read_command(
            fake_request_id(29),
            active_account_key.clone(),
            room_id.clone(),
            "$fully-read-event".to_owned(),
        )
        .expect("set_fully_read should build a command")
        {
            CoreCommand::Timeline(TimelineCommand::SetFullyRead {
                request_id,
                key,
                event_id,
            }) => {
                assert_eq!(request_id, fake_request_id(29));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    matrix_desktop_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(event_id, "$fully-read-event");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_set_typing_command(
            fake_request_id(30),
            active_account_key.clone(),
            room_id.clone(),
            true,
        ) {
            CoreCommand::Timeline(TimelineCommand::SetTyping {
                request_id,
                key,
                is_typing,
            }) => {
                assert_eq!(request_id, fake_request_id(30));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    matrix_desktop_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert!(is_typing);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_set_presence_command(fake_request_id(31), PresenceKind::Away) {
            CoreCommand::Account(AccountCommand::SetPresence {
                request_id,
                presence,
            }) => {
                assert_eq!(request_id, fake_request_id(31));
                assert_eq!(presence, PresenceKind::Away);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_set_display_name_command(
            fake_request_id(32),
            Some("Private Display".to_owned()),
        ) {
            CoreCommand::Account(AccountCommand::SetDisplayName {
                request_id,
                display_name,
            }) => {
                assert_eq!(request_id, fake_request_id(32));
                assert_eq!(display_name.as_deref(), Some("Private Display"));
                let debug = format!(
                    "{:?}",
                    CoreCommand::Account(AccountCommand::SetDisplayName {
                        request_id,
                        display_name,
                    })
                );
                assert!(!debug.contains("Private Display"), "{debug}");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_set_avatar_command(
            fake_request_id(33),
            "image/png".to_owned(),
            vec![9, 8, 7, 6],
        ) {
            CoreCommand::Account(AccountCommand::SetAvatar {
                request_id,
                request,
            }) => {
                assert_eq!(request_id, fake_request_id(33));
                assert_eq!(request.mime_type, "image/png");
                assert_eq!(request.bytes, vec![9, 8, 7, 6]);
                let debug = format!(
                    "{:?}",
                    CoreCommand::Account(AccountCommand::SetAvatar {
                        request_id,
                        request,
                    })
                );
                assert!(debug.contains("image/png"), "{debug}");
                assert!(!debug.contains("9, 8, 7, 6"), "{debug}");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_leave_room_command(fake_request_id(13), room_id.clone()) {
            CoreCommand::Room(RoomCommand::LeaveRoom {
                request_id,
                room_id: route_room_id,
            }) => {
                assert_eq!(request_id, fake_request_id(13));
                assert_eq!(route_room_id, room_id);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_forget_room_command(fake_request_id(14), room_id.clone()) {
            CoreCommand::Room(RoomCommand::ForgetRoom {
                request_id,
                room_id: route_room_id,
            }) => {
                assert_eq!(request_id, fake_request_id(14));
                assert_eq!(route_room_id, room_id);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_submit_search_command(
            fake_request_id(15),
            query.clone(),
            resolve_search_scope_from_active_room(
                SearchScopeKind::CurrentRoom,
                Some(room_id.clone()),
            ),
        ) {
            CoreCommand::Search(SearchCommand::Query {
                request_id,
                query: route_query,
                scope,
            }) => {
                assert_eq!(request_id, fake_request_id(15));
                assert_eq!(route_query, query);
                assert_eq!(
                    scope,
                    SearchScope::Room {
                        room_id: room_id.clone()
                    }
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }

        assert_eq!(
            resolve_search_scope_from_active_room(SearchScopeKind::CurrentRoom, None,),
            SearchScope::Global
        );

        match build_create_room_command(fake_request_id(16), "Local QA Room".to_owned()) {
            CoreCommand::Room(RoomCommand::CreateRoom {
                request_id,
                name,
                encrypted,
            }) => {
                assert_eq!(request_id, fake_request_id(16));
                assert_eq!(name, "Local QA Room");
                assert!(!encrypted);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_create_space_command(fake_request_id(17), "Local QA Space".to_owned()) {
            CoreCommand::Room(RoomCommand::CreateSpace { request_id, name }) => {
                assert_eq!(request_id, fake_request_id(17));
                assert_eq!(name, "Local QA Space");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_set_space_child_command(
            fake_request_id(18),
            "!space:example.org".to_owned(),
            "!room:example.org".to_owned(),
            "example.org".to_owned(),
        ) {
            CoreCommand::Room(RoomCommand::SetSpaceChild {
                request_id,
                space_id,
                child_room_id,
                via_server,
            }) => {
                assert_eq!(request_id, fake_request_id(18));
                assert_eq!(space_id, "!space:example.org");
                assert_eq!(child_room_id, "!room:example.org");
                assert_eq!(via_server, "example.org");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_accept_invite_command(fake_request_id(19), "!invite:example.org".to_owned()) {
            CoreCommand::Room(RoomCommand::AcceptInvite {
                request_id,
                room_id,
            }) => {
                assert_eq!(request_id, fake_request_id(19));
                assert_eq!(room_id, "!invite:example.org");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_decline_invite_command(fake_request_id(20), "!decline:example.org".to_owned()) {
            CoreCommand::Room(RoomCommand::DeclineInvite {
                request_id,
                room_id,
            }) => {
                assert_eq!(request_id, fake_request_id(20));
                assert_eq!(room_id, "!decline:example.org");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_start_direct_message_command(
            fake_request_id(21),
            "@target:example.org".to_owned(),
        ) {
            CoreCommand::Room(RoomCommand::StartDirectMessage {
                request_id,
                user_id,
            }) => {
                assert_eq!(request_id, fake_request_id(21));
                assert_eq!(user_id, "@target:example.org");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_invite_user_command(
            fake_request_id(22),
            "!room:example.org".to_owned(),
            "@target:example.org".to_owned(),
        ) {
            CoreCommand::Room(RoomCommand::InviteUser {
                request_id,
                room_id,
                user_id,
            }) => {
                assert_eq!(request_id, fake_request_id(22));
                assert_eq!(room_id, "!room:example.org");
                assert_eq!(user_id, "@target:example.org");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_set_room_tag_command(
            fake_request_id(23),
            "!room:example.org".to_owned(),
            RoomTagKind::Favourite,
            Some(0.25),
        ) {
            CoreCommand::Room(RoomCommand::SetTag {
                request_id,
                room_id,
                tag,
                order,
            }) => {
                assert_eq!(request_id, fake_request_id(23));
                assert_eq!(room_id, "!room:example.org");
                assert_eq!(tag, RoomTagKind::Favourite);
                assert_eq!(order, Some(0.25));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_remove_room_tag_command(
            fake_request_id(24),
            "!room:example.org".to_owned(),
            RoomTagKind::LowPriority,
        ) {
            CoreCommand::Room(RoomCommand::RemoveTag {
                request_id,
                room_id,
                tag,
            }) => {
                assert_eq!(request_id, fake_request_id(24));
                assert_eq!(room_id, "!room:example.org");
                assert_eq!(tag, RoomTagKind::LowPriority);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_send_reply_command(
            fake_request_id(23),
            active_account_key.clone(),
            room_id.clone(),
            "desktop-reply-1".to_owned(),
            "$root".to_owned(),
            "reply body".to_owned(),
        )
        .expect("send_reply should build a command")
        {
            CoreCommand::Timeline(TimelineCommand::SendReply {
                request_id,
                key,
                transaction_id,
                in_reply_to_event_id,
                body,
            }) => {
                assert_eq!(request_id, fake_request_id(23));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    matrix_desktop_core::TimelineKind::Room {
                        room_id: room_id.clone()
                    }
                );
                assert_eq!(transaction_id, "desktop-reply-1");
                assert_eq!(in_reply_to_event_id, "$root");
                assert_eq!(body, "reply body");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_send_thread_reply_command(
            fake_request_id(24),
            active_account_key.clone(),
            room_id.clone(),
            "$root".to_owned(),
            "desktop-thread-reply-1".to_owned(),
            "thread reply body".to_owned(),
        )
        .expect("send_thread_reply should build a command")
        {
            CoreCommand::Timeline(TimelineCommand::SendReply {
                request_id,
                key,
                transaction_id,
                in_reply_to_event_id,
                body,
            }) => {
                assert_eq!(request_id, fake_request_id(24));
                assert_eq!(key.account_key, active_account_key);
                assert_eq!(
                    key.kind,
                    matrix_desktop_core::TimelineKind::Thread {
                        room_id: room_id.clone(),
                        root_event_id: "$root".to_owned(),
                    }
                );
                assert_eq!(transaction_id, "desktop-thread-reply-1");
                assert_eq!(in_reply_to_event_id, "$root");
                assert_eq!(body, "thread reply body");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_set_thread_composer_draft_command(
            fake_request_id(21),
            room_id.clone(),
            "$root".to_owned(),
            "thread draft".to_owned(),
        ) {
            CoreCommand::App(AppCommand::SetThreadComposerDraft {
                request_id,
                room_id: command_room_id,
                root_event_id,
                draft,
            }) => {
                assert_eq!(request_id, fake_request_id(21));
                assert_eq!(command_room_id, room_id);
                assert_eq!(root_event_id, "$root");
                assert_eq!(draft, "thread draft");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn tauri_command_routes_blank_message_bodies_return_no_command() {
        let account_key = AccountKey("@alice:example.org".to_owned());
        let room_id = "!room:example.org".to_owned();

        assert!(
            build_send_text_command(
                fake_request_id(14),
                account_key.clone(),
                room_id.clone(),
                "desktop-14".to_owned(),
                "   ".to_owned(),
            )
            .is_none()
        );
        assert!(
            build_edit_message_command(
                fake_request_id(15),
                account_key,
                room_id,
                "$event".to_owned(),
                "\n\t ".to_owned(),
            )
            .is_none()
        );
        assert!(
            build_upload_media_command(
                fake_request_id(17),
                AccountKey("@alice:example.org".to_owned()),
                "!room:example.org".to_owned(),
                "desktop-media-empty".to_owned(),
                "empty.bin".to_owned(),
                "application/octet-stream".to_owned(),
                vec![],
            )
            .is_none()
        );
        assert!(
            build_download_media_command(
                fake_request_id(18),
                AccountKey("@alice:example.org".to_owned()),
                "!room:example.org".to_owned(),
                "\n\t ".to_owned(),
            )
            .is_none()
        );
        assert!(
            build_send_thread_reply_command(
                fake_request_id(16),
                AccountKey("@alice:example.org".to_owned()),
                "!room:example.org".to_owned(),
                "$root".to_owned(),
                "desktop-16".to_owned(),
                "\n\t ".to_owned(),
            )
            .is_none()
        );
    }

    #[test]
    fn thread_timeline_backwards_pagination_builder_targets_thread_key() {
        let account_key = AccountKey("@alice:example.org".to_owned());
        let room_id = "!room:example.org".to_owned();
        let root_event_id = "$thread-root".to_owned();

        match build_paginate_thread_timeline_backwards_command(
            fake_request_id(22),
            account_key.clone(),
            room_id.clone(),
            root_event_id.clone(),
        ) {
            CoreCommand::Timeline(TimelineCommand::Paginate {
                request_id,
                key,
                direction,
                event_count,
            }) => {
                assert_eq!(request_id, fake_request_id(22));
                assert_eq!(key.account_key, account_key);
                assert_eq!(
                    key.kind,
                    matrix_desktop_core::TimelineKind::Thread {
                        room_id,
                        root_event_id,
                    }
                );
                assert_eq!(direction, PaginationDirection::Backward);
                assert_eq!(event_count, 30);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn thread_timeline_backwards_pagination_contract_is_present() {
        let commands_source = include_str!("commands.rs");
        let lib_source = include_str!("lib.rs");
        let helper_name = "build_paginate_thread_timeline_backwards_command";
        let command_name = "pub async fn paginate_thread_timeline_backwards";
        let registration_name = "commands::paginate_thread_timeline_backwards";

        let helper_offset = commands_source
            .find(helper_name)
            .expect("thread pagination builder helper should exist");
        let helper_source = &commands_source[helper_offset..];
        let helper_end = helper_source
            .find("pub(crate) fn build_send_text_command")
            .expect("thread pagination builder should live before send_text builder");
        let helper_source = &helper_source[..helper_end];

        assert!(
            commands_source.contains(command_name),
            "Tauri command should expose thread pagination"
        );
        assert!(
            lib_source.contains(registration_name),
            "Tauri command should be registered in generate_handler"
        );
        assert!(
            helper_source.contains("TimelineKind::Thread"),
            "thread pagination builder should use a thread timeline key"
        );
        assert!(
            helper_source.contains("PaginationDirection::Backward"),
            "thread pagination builder should request backward pagination"
        );
        assert!(
            helper_source.contains("event_count: TIMELINE_BACKWARDS_PAGE_EVENT_COUNT"),
            "thread pagination should keep the shared room pagination event count"
        );
    }

    #[test]
    fn update_settings_command_routes_patch_to_app_update_settings() {
        let command = build_update_settings_command(
            fake_request_id(23),
            SettingsPatch {
                appearance: Some(AppearanceSettings {
                    theme: ThemePreference::Dark,
                }),
                ..SettingsPatch::default()
            },
        );

        match command {
            CoreCommand::App(AppCommand::UpdateSettings { request_id, patch }) => {
                assert_eq!(request_id, fake_request_id(23));
                assert_eq!(
                    patch.appearance.expect("appearance patch").theme,
                    ThemePreference::Dark
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }

        let debug = format!(
            "{:?}",
            build_update_settings_command(
                fake_request_id(24),
                SettingsPatch {
                    locale: Some(LocaleSettings {
                        language_tag: Some("ja-JP-private".to_owned()),
                        text_direction: TextDirectionPreference::Auto,
                    }),
                    ..SettingsPatch::default()
                },
            )
        );
        assert!(debug.contains("locale"), "{debug}");
        assert!(!debug.contains("ja-JP-private"), "{debug}");
    }

    #[test]
    fn e2ee_trust_commands_route_to_account_state_machine() {
        match build_bootstrap_cross_signing_command(
            fake_request_id(25),
            Some(AuthSecret::new("cross-signing-password")),
        ) {
            CoreCommand::Account(AccountCommand::BootstrapCrossSigning { request_id, auth }) => {
                assert_eq!(request_id, fake_request_id(25));
                assert_eq!(
                    auth.expect("auth secret").expose_secret(),
                    "cross-signing-password"
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_enable_key_backup_command(fake_request_id(26)) {
            CoreCommand::Account(AccountCommand::EnableKeyBackup {
                request_id,
                passphrase,
            }) => {
                assert_eq!(request_id, fake_request_id(26));
                assert!(passphrase.is_none());
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_accept_verification_command(fake_request_id(27), 72) {
            CoreCommand::Account(AccountCommand::AcceptVerification {
                request_id,
                flow_id,
            }) => {
                assert_eq!(request_id, fake_request_id(27));
                assert_eq!(flow_id, 72);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_confirm_sas_verification_command(fake_request_id(28), 73) {
            CoreCommand::Account(AccountCommand::ConfirmSasVerification {
                request_id,
                flow_id,
            }) => {
                assert_eq!(request_id, fake_request_id(28));
                assert_eq!(flow_id, 73);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_cancel_verification_command(
            fake_request_id(29),
            74,
            VerificationCancelReason::User,
        ) {
            CoreCommand::Account(AccountCommand::CancelVerification {
                request_id,
                flow_id,
                reason,
            }) => {
                assert_eq!(request_id, fake_request_id(29));
                assert_eq!(flow_id, 74);
                assert_eq!(reason, VerificationCancelReason::User);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        match build_reset_identity_command(fake_request_id(30)) {
            CoreCommand::Account(AccountCommand::ResetIdentity { request_id }) => {
                assert_eq!(request_id, fake_request_id(30));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        let password_command = build_submit_identity_reset_password_command(
            fake_request_id(31),
            75,
            AuthSecret::new("identity-reset-password"),
        );
        match &password_command {
            CoreCommand::Account(AccountCommand::SubmitIdentityResetAuth {
                request_id,
                flow_id,
                request: IdentityResetAuthRequest::UiaaPassword { password },
            }) => {
                assert_eq!(*request_id, fake_request_id(31));
                assert_eq!(*flow_id, 75);
                assert_eq!(password.expose_secret(), "identity-reset-password");
            }
            other => panic!("unexpected command: {other:?}"),
        }
        let debug = format!("{password_command:?}");
        assert!(!debug.contains("identity-reset-password"), "{debug}");

        match build_submit_identity_reset_oauth_command(fake_request_id(32), 76) {
            CoreCommand::Account(AccountCommand::SubmitIdentityResetAuth {
                request_id,
                flow_id,
                request: IdentityResetAuthRequest::OAuthApproved,
            }) => {
                assert_eq!(request_id, fake_request_id(32));
                assert_eq!(flow_id, 76);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn update_settings_tauri_command_contract_is_present() {
        let commands_source = include_str!("commands.rs");
        let lib_source = include_str!("lib.rs");
        let command_name = "pub async fn update_settings";
        let builder_name = "build_update_settings_command";
        let route_name = "AppCommand::UpdateSettings";
        let registration_name = "commands::update_settings";

        assert!(
            commands_source.contains(command_name),
            "Tauri command should expose update_settings"
        );
        assert!(
            commands_source.contains(builder_name),
            "Tauri command should keep a testable UpdateSettings builder"
        );
        assert!(
            commands_source.contains(route_name),
            "Tauri command should route through the Rust settings state machine"
        );
        assert!(
            lib_source.contains(registration_name),
            "Tauri command should be registered in generate_handler"
        );
    }

    #[test]
    fn e2ee_trust_tauri_command_contracts_are_present() {
        let commands_source = include_str!("commands.rs");
        let lib_source = include_str!("lib.rs");
        for (command_name, route_name, registration_name) in [
            (
                "pub async fn bootstrap_cross_signing",
                "build_bootstrap_cross_signing_command",
                "commands::bootstrap_cross_signing",
            ),
            (
                "pub async fn enable_key_backup",
                "build_enable_key_backup_command",
                "commands::enable_key_backup",
            ),
            (
                "pub async fn accept_verification",
                "build_accept_verification_command",
                "commands::accept_verification",
            ),
            (
                "pub async fn confirm_sas_verification",
                "build_confirm_sas_verification_command",
                "commands::confirm_sas_verification",
            ),
            (
                "pub async fn cancel_verification",
                "build_cancel_verification_command",
                "commands::cancel_verification",
            ),
            (
                "pub async fn reset_identity",
                "build_reset_identity_command",
                "commands::reset_identity",
            ),
            (
                "pub async fn submit_identity_reset_password",
                "build_submit_identity_reset_password_command",
                "commands::submit_identity_reset_password",
            ),
            (
                "pub async fn submit_identity_reset_oauth",
                "build_submit_identity_reset_oauth_command",
                "commands::submit_identity_reset_oauth",
            ),
        ] {
            assert!(
                commands_source.contains(command_name),
                "Tauri command should expose {command_name}"
            );
            assert!(
                commands_source.contains(route_name),
                "Tauri command should route through {route_name}"
            );
            assert!(
                lib_source.contains(registration_name),
                "Tauri command should register {registration_name}"
            );
        }
    }

    #[test]
    fn profile_tauri_command_contracts_are_present() {
        let commands_source = include_str!("commands.rs");
        let lib_source = include_str!("lib.rs");
        for (command_name, route_name, registration_name) in [
            (
                "pub async fn set_display_name",
                "build_set_display_name_command",
                "commands::set_display_name",
            ),
            (
                "pub async fn set_avatar",
                "build_set_avatar_command",
                "commands::set_avatar",
            ),
        ] {
            assert!(
                commands_source.contains(command_name),
                "Tauri command should expose {command_name}"
            );
            assert!(
                commands_source.contains(route_name),
                "Tauri command should route through {route_name}"
            );
            assert!(
                lib_source.contains(registration_name),
                "Tauri command should register {registration_name}"
            );
        }
    }

    #[test]
    fn composer_key_resolver_command_contract_is_present() {
        let commands_source = include_str!("commands.rs");
        let lib_source = include_str!("lib.rs");
        let command_name = "pub async fn resolve_composer_key_action";
        let route_name = "matrix_desktop_state::resolve_composer_key_action";
        let settings_token = "settings.values.keyboard.composer_send_shortcut";
        let registration_name = "commands::resolve_composer_key_action";

        assert!(
            commands_source.contains(command_name),
            "Tauri command should expose resolve_composer_key_action"
        );
        assert!(
            commands_source.contains(route_name),
            "Tauri command should route through the Rust-owned resolver"
        );
        assert!(
            commands_source.contains(settings_token),
            "resolver should derive the send shortcut from Rust-owned settings"
        );
        assert!(
            lib_source.contains(registration_name),
            "Tauri command should be registered in generate_handler"
        );
    }

    #[test]
    fn select_room_submits_timeline_subscribe_after_room_selection() {
        let source = include_str!("commands.rs");
        let fn_name = concat!("pub async fn select", "_room");
        let select_token = concat!("build_select", "_room_command");
        let attach_token = concat!("state.runtime.", "attach");
        let wait_token = concat!("wait_for_selected", "_room");
        let subscribe_token = concat!("build_subscribe", "_timeline_command");
        let account_key_token = concat!("account_key", "_from_snapshot");
        let timeout_token = concat!("SELECT_ROOM", "_EVENT_TIMEOUT");
        let fn_offset = source
            .find(fn_name)
            .expect("select_room command should exist");
        let rest = &source[fn_offset..];
        let end = rest
            .find("pub async fn paginate_timeline_backwards")
            .expect("next command should exist");
        let select_room_source = &rest[..end];
        let attach_offset = select_room_source
            .find(attach_token)
            .expect("select_room should attach an event connection before selecting");
        let select_offset = select_room_source
            .find(select_token)
            .expect("select_room should submit room selection");
        let wait_offset = select_room_source
            .find(wait_token)
            .expect("select_room should wait for selected-room state");
        let subscribe_offset = select_room_source
            .find(subscribe_token)
            .expect("select_room should subscribe the selected timeline");

        assert!(
            attach_offset < select_offset,
            "event connection should be attached before room selection"
        );
        assert!(
            select_offset < wait_offset && wait_offset < subscribe_offset,
            "room selection state should be observed before timeline subscription"
        );
        assert!(
            select_room_source.contains(account_key_token),
            "select_room should derive the active account key for timeline subscription"
        );
        assert!(
            select_room_source.contains(timeout_token),
            "selected-room wait should be bounded"
        );
    }

    #[test]
    fn select_search_result_submits_room_selection_then_focused_timeline_subscription() {
        let source = include_str!("commands.rs");
        let fn_name = "pub async fn select_search_result";
        let select_token = "select_search_result";
        let close_token = "CloseFocusedContext";
        let open_token = "OpenFocusedContext";
        let select_room_token = concat!("build_select", "_room_command");
        let subscribe_room_token = "build_subscribe_timeline_command";

        let fn_offset = source
            .find(fn_name)
            .expect("select_search_result command should exist");
        let rest = &source[fn_offset..];
        let end = rest
            .find("pub async fn paginate_timeline_backwards")
            .expect("next command should exist");
        let select_source = &rest[..end];

        assert!(
            select_source.contains(select_token),
            "select_search_result should name the command path"
        );
        assert!(
            select_source.contains(close_token),
            "select_search_result should close the previous focused context"
        );
        assert!(
            select_source.contains(open_token),
            "select_search_result should open the new focused context through AppCommand"
        );
        assert!(
            select_source.contains(select_room_token),
            "select_search_result should select the room before opening the focused context"
        );
        assert!(
            select_source.contains(subscribe_room_token),
            "select_search_result should subscribe the selected room timeline before opening focused context"
        );
        assert!(
            select_source.contains("wait_for_selected_room"),
            "select_search_result should wait for the selected room state"
        );
        assert!(
            select_source.contains("state.runtime.attach"),
            "select_search_result should attach a fresh core connection"
        );

        let subscribe_offset = select_source
            .find(subscribe_room_token)
            .expect("search result command should subscribe the normal room timeline");
        let open_offset = select_source
            .find(open_token)
            .expect("search result command should open focused context");
        assert!(
            subscribe_offset < open_offset,
            "normal room timeline subscription should happen before focused context open"
        );
    }

    #[test]
    fn close_focused_context_command_routes_to_app_close_focused_context() {
        let source = include_str!("commands.rs");
        let fn_name = concat!("pub async fn close", "_focused_context");
        let command_token = concat!("Close", "FocusedContext");
        let submit_token = "submit_core_command";
        let title_token = "update_qa_window_title_from_state";
        let snapshot_token = "current_snapshot";

        let fn_offset = source
            .find(fn_name)
            .expect("close_focused_context command should exist");
        let rest = &source[fn_offset..];
        let end = rest
            .find("pub async fn paginate_timeline_backwards")
            .expect("next command should exist");
        let close_source = &rest[..end];

        assert!(
            close_source.contains(command_token),
            "close_focused_context should route through AppCommand::CloseFocusedContext"
        );
        assert!(
            close_source.contains(submit_token),
            "close_focused_context should submit the core command"
        );
        assert!(
            close_source.contains(title_token),
            "close_focused_context should refresh the QA title after state changes"
        );
        assert!(
            close_source.contains(snapshot_token),
            "close_focused_context should return the current snapshot"
        );
    }

    #[test]
    fn build_subscribe_focused_timeline_command_routes_to_focused_timeline_kind() {
        let account_key = AccountKey("@alice:example.org".to_owned());
        let command = build_subscribe_focused_timeline_command(
            fake_request_id(21),
            account_key.clone(),
            "!room:example.org".to_owned(),
            "$event".to_owned(),
        );

        match command {
            CoreCommand::Timeline(TimelineCommand::Subscribe { request_id, key }) => {
                assert_eq!(request_id, fake_request_id(21));
                assert_eq!(key.account_key, account_key);
                assert_eq!(
                    key.kind,
                    matrix_desktop_core::TimelineKind::Focused {
                        room_id: "!room:example.org".to_owned(),
                        event_id: "$event".to_owned(),
                    }
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn wait_for_selected_room_observes_state_changed_failures_and_timeout() {
        let source = include_str!("commands.rs");
        let helper_name = concat!("async fn wait_for_selected", "_room");
        let helper_offset = source
            .find(helper_name)
            .expect("selected-room wait helper should exist");
        let rest = &source[helper_offset..];
        let end = rest
            .find("fn snapshot_has_active_room")
            .expect("active-room snapshot helper should follow selected-room wait");
        let helper_source = &rest[..end];

        assert!(helper_source.contains("timeout_at"));
        assert!(helper_source.contains("CoreEvent::StateChanged"));
        assert!(helper_source.contains(concat!("Operation", "Failed")));
        assert!(helper_source.contains(concat!("snapshot_has_active", "_room")));
    }

    #[test]
    fn tauri_command_routes_redact_secret_bearing_values_from_debug() {
        let account_key = AccountKey("@alice:example.org".to_owned());
        let room_id = "!room:example.org".to_owned();
        let login = build_submit_login_command(
            fake_request_id(16),
            LoginRequest {
                homeserver: "https://matrix.example.org".to_owned(),
                username: "alice".to_owned(),
                password: AuthSecret::new("password-123"),
                device_display_name: Some("Laptop".to_owned()),
            },
        );
        let recovery =
            build_submit_recovery_command(fake_request_id(17), AuthSecret::new("recovery-123"));
        let send = build_send_text_command(
            fake_request_id(18),
            account_key.clone(),
            room_id.clone(),
            "desktop-18".to_owned(),
            "sensitive body".to_owned(),
        )
        .expect("send_text should build a command");
        let edit = build_edit_message_command(
            fake_request_id(19),
            account_key,
            room_id,
            "$event".to_owned(),
            "sensitive edit body".to_owned(),
        )
        .expect("edit_message should build a command");
        let upload = build_upload_media_command(
            fake_request_id(21),
            AccountKey("@alice:example.org".to_owned()),
            "!room:example.org".to_owned(),
            "desktop-media-sensitive".to_owned(),
            "secret-filename.pdf".to_owned(),
            "application/pdf".to_owned(),
            b"secret media bytes".to_vec(),
        )
        .expect("upload_media should build a command");
        let download = build_download_media_command(
            fake_request_id(22),
            AccountKey("@alice:example.org".to_owned()),
            "!room:example.org".to_owned(),
            "$secret-media-event".to_owned(),
        )
        .expect("download_media should build a command");
        let search = build_submit_search_command(
            fake_request_id(20),
            "secret search terms".to_owned(),
            resolve_search_scope_from_active_room(
                SearchScopeKind::CurrentRoom,
                Some("!room:example.org".to_owned()),
            ),
        );

        for (command, secret) in [
            (&login, "password-123"),
            (&recovery, "recovery-123"),
            (&send, "sensitive body"),
            (&edit, "sensitive edit body"),
            (&upload, "secret-filename.pdf"),
            (&upload, "secret media bytes"),
            (&download, "$secret-media-event"),
            (&search, "secret search terms"),
        ] {
            let debug = format!("{command:?}");
            assert!(
                !debug.contains(secret),
                "Debug output leaked a secret: {debug}"
            );
        }
    }

    #[test]
    fn submit_login_request_waits_for_logged_in_then_starts_sync() {
        let source = include_str!("commands.rs");
        let helper_name = concat!("async fn submit_login", "_and_start_sync");
        let wait_call_token = concat!("wait_for_logged", "_in_ready");
        let logged_in_token = concat!("AccountEvent::", "LoggedIn");
        let start_sync_token = concat!("build_start", "_sync_command");
        let failed_token = concat!("Operation", "Failed");
        let timeout_token = concat!("LOGIN_EVENT", "_TIMEOUT");
        let helper_offset = source
            .find(helper_name)
            .expect("shared login helper should exist");
        let helper_source = &source[helper_offset..];
        let wait_call_offset = helper_source
            .find(wait_call_token)
            .expect("helper should wait for login before sync");
        let start_sync_offset = helper_source
            .find(start_sync_token)
            .expect("helper should submit SyncCommand::Start");

        assert!(
            wait_call_offset < start_sync_offset,
            "sync start must be submitted only after login success"
        );
        assert!(helper_source.contains(timeout_token));
        let wait_helper_offset = source
            .find(concat!("async fn wait_for_logged", "_in_ready"))
            .expect("login wait helper should exist");
        let wait_helper_source = &source[wait_helper_offset..];
        assert!(wait_helper_source.contains(logged_in_token));
        assert!(wait_helper_source.contains(failed_token));
        assert!(wait_helper_source.contains("timeout_at"));
    }
}
