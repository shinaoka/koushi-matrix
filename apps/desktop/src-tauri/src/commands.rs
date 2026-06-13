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
    AccountCommand, AccountKey, CoreCommand, PaginationDirection, RoomCommand, SearchCommand,
    SearchScope, TimelineCommand, TimelineKey, TimelineKind,
};
use matrix_desktop_state::{AuthSecret, LoginRequest, RecoveryRequest, SessionInfo};
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
    // Compatibility shim: discovery is implicit in LoginPassword (core
    // resolves it), and the frontend keeps homeserver form text locally.
    // Keeping this command avoids frontend API churn without adding a core
    // command that would duplicate login behavior.
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
    let request_id = next_request_id(state).await;
    submit_core_command(
        state,
        CoreCommand::Account(AccountCommand::LoginPassword {
            request_id,
            request: login_request,
        }),
    )
    .await?;
    update_qa_window_title_from_state(&app, state).await;
    Ok(())
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
    let account_key = AccountKey(user_id);
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::Account(AccountCommand::SwitchAccount {
            request_id,
            account_key,
        }),
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
    submit_core_command(
        state,
        CoreCommand::Account(AccountCommand::SubmitRecovery {
            request_id,
            request: RecoveryRequest { secret },
        }),
    )
    .await?;
    update_qa_window_title_from_state(&app, state).await;
    Ok(())
}

#[tauri::command]
pub async fn logout(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::Account(AccountCommand::Logout { request_id }),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
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
        CoreCommand::Room(RoomCommand::SelectSpace {
            request_id,
            space_id,
        }),
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
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::Room(RoomCommand::SelectRoom {
            request_id,
            room_id,
        }),
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
    let key = TimelineKey {
        account_key,
        kind: TimelineKind::Room {
            room_id: room_id.clone(),
        },
    };
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::Timeline(TimelineCommand::Paginate {
            request_id,
            key,
            direction: PaginationDirection::Backward,
            event_count: 30,
        }),
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
    let key = TimelineKey {
        account_key,
        kind: TimelineKind::Room {
            room_id: room_id.clone(),
        },
    };
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::Timeline(TimelineCommand::SendText {
            request_id,
            key,
            transaction_id,
            body,
        }),
    )
    .await?;
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
    let key = TimelineKey {
        account_key,
        kind: TimelineKind::Room {
            room_id: room_id.clone(),
        },
    };
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::Timeline(TimelineCommand::EditText {
            request_id,
            key,
            event_id,
            body,
        }),
    )
    .await?;
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
    let key = TimelineKey {
        account_key,
        kind: TimelineKind::Room {
            room_id: room_id.clone(),
        },
    };
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::Timeline(TimelineCommand::Redact {
            request_id,
            key,
            event_id,
        }),
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
    submit_core_command(
        state.inner(),
        CoreCommand::Room(RoomCommand::LeaveRoom {
            request_id,
            room_id,
        }),
    )
    .await?;
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
        CoreCommand::Room(RoomCommand::ForgetRoom {
            request_id,
            room_id,
        }),
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
    // Frontend navigation shim: opening/closing the thread pane is local UI
    // state until product design requires native cross-window thread commands.
    let _ = (room_id, root_event_id);
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn close_thread(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    // Same frontend navigation shim as `open_thread`.
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
        CoreCommand::Search(SearchCommand::Query {
            request_id,
            query,
            scope: search_scope,
        }),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

// ---- Helpers ----

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

async fn resolve_search_scope(
    scope: SearchScopeKind,
    state: &CoreRuntimeState,
) -> matrix_desktop_core::SearchScope {
    let snapshot = state.connection.lock().await.snapshot();
    match scope {
        SearchScopeKind::CurrentRoom => snapshot
            .navigation
            .active_room_id
            .as_ref()
            .map(|room_id| SearchScope::Room {
                room_id: room_id.clone(),
            })
            .unwrap_or(SearchScope::Global),
        SearchScopeKind::CurrentSpace | SearchScopeKind::AllRooms => SearchScope::Global,
        SearchScopeKind::Dms => SearchScope::Global,
    }
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

#[cfg(test)]
mod tests {
    use matrix_desktop_state::{SessionInfo, SessionState};

    use super::{
        parse_qa_login_pipe_payload, qa_recovery_prompt_is_available, qa_window_title_string,
    };
    use matrix_desktop_state::{AppState, RoomSummary};

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
                is_dm: false,
                unread_count: 0,
                parent_space_ids: vec![],
            },
            RoomSummary {
                room_id: "!room2:example.org".to_owned(),
                display_name: "Room 2".to_owned(),
                is_dm: false,
                unread_count: 0,
                parent_space_ids: vec![],
            },
        ];

        let title = qa_window_title_string(&snapshot, 42);

        assert!(title.contains("session=signedOut"));
        assert!(title.contains("sync=stopped"));
        assert!(title.contains("rooms=2"));
        assert!(title.contains("timeline_items=42"));
    }
}
