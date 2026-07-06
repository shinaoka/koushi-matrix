use super::*;

#[derive(serde::Serialize)]
pub struct OidcAuthorizationResponse {
    pub authorization_url: String,
    pub state: String,
}

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
    let mut event_conn = state.inner().runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_discover_login_command(request_id, homeserver))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_auth_changed(&mut event_conn, LOGIN_EVENT_TIMEOUT).await?;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn start_oidc_login(
    homeserver: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<OidcAuthorizationResponse, String> {
    let mut event_conn = state.inner().runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_start_oidc_login_command(request_id, homeserver))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_oidc_authorization(&mut event_conn, request_id, LOGIN_EVENT_TIMEOUT).await
}

#[tauri::command]
pub async fn complete_oidc_login(
    _homeserver: String,
    callback_url: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut event_conn = state.inner().runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_complete_oidc_login_command(request_id, callback_url))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_logged_in_authenticated(&mut event_conn, request_id, LOGIN_EVENT_TIMEOUT).await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

async fn wait_for_oidc_authorization(
    event_conn: &mut koushi_core::CoreConnection,
    request_id: koushi_core::RequestId,
    timeout: std::time::Duration,
) -> Result<OidcAuthorizationResponse, String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let event = tokio::time::timeout_at(deadline, event_conn.recv_event())
            .await
            .map_err(|_| "OIDC login did not start".to_owned())?;
        match event {
            Ok(koushi_core::CoreEvent::Account(
                koushi_core::AccountEvent::OidcAuthorizationCreated {
                    request_id: ev_id,
                    authorization_url,
                    state,
                },
            )) if ev_id == request_id => {
                return Ok(OidcAuthorizationResponse {
                    authorization_url,
                    state,
                });
            }
            Ok(koushi_core::CoreEvent::OperationFailed {
                request_id: ev_id, ..
            }) if ev_id == request_id => return Err("OIDC login failed".to_owned()),
            Err(_) => return Err("OIDC login event stream lagged".to_owned()),
            _ => {}
        }
    }
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

#[tauri::command]
pub async fn submit_soft_logout_reauth(
    password: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    submit_soft_logout_reauth_request(app, state.inner(), AuthSecret::new(password)).await?;
    current_snapshot(state.inner()).await
}

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
            Ok(koushi_core::CoreEvent::Account(
                koushi_core::AccountEvent::SavedSessionsListed {
                    request_id: ev_id,
                    sessions,
                },
            )) if ev_id == request_id => return Ok(sessions),
            Ok(koushi_core::CoreEvent::OperationFailed {
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
