use super::account::{
    build_erase_device_cleanup_local_data_anyway_command, build_start_device_cleanup_command,
    build_submit_device_cleanup_uia_command,
};
use super::*;
use crate::dto::FrontendDesktopSnapshot;
use crate::oidc_browser::OidcBrowserLaunchFailure;
#[cfg(not(target_os = "linux"))]
use crate::oidc_browser::launch_oidc_authorization_url;
#[cfg(target_os = "linux")]
use crate::oidc_browser::{
    launch_linux_default_browser, launch_oidc_authorization_url_with_fallback, running_under_wsl,
};
use tauri_plugin_opener::OpenerExt;

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OidcBrowserLaunchOutcome {
    Launched,
    InvalidAuthorizationUrl,
    BrowserLaunchFailed,
}

#[derive(serde::Serialize)]
pub struct OidcBrowserLaunchResponse {
    pub outcome: OidcBrowserLaunchOutcome,
    pub settlement: FrontendCommandSettlement,
}

#[tauri::command]
pub async fn get_snapshot(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
    account_tab_id: Option<String>,
) -> Result<FrontendDesktopSnapshot, String> {
    let (account_tab_id, connection) = match account_tab_id {
        Some(id) => {
            state
                .inner()
                .connection
                .lock_for_tab_id(
                    &koushi_core::account_runtime_manager::AccountTabId::from_string(id),
                )
                .await?
        }
        None => state.inner().connection.lock_with_tab_id().await,
    };
    let snapshot = connection.versioned_snapshot();
    drop(connection);
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(FrontendDesktopSnapshot::from_versioned_for_account(
        snapshot.state,
        snapshot.generation,
        account_tab_id,
    ))
}

/// Reconcile a command receipt whose state delta has not reached the renderer.
#[tauri::command]
pub async fn settlement_snapshot(
    state: State<'_, CoreRuntimeState>,
    account_tab_id: Option<String>,
) -> Result<FrontendDesktopSnapshot, String> {
    let (account_tab_id, connection) = match account_tab_id {
        Some(id) => {
            state
                .inner()
                .connection
                .lock_for_tab_id(
                    &koushi_core::account_runtime_manager::AccountTabId::from_string(id),
                )
                .await?
        }
        None => state.inner().connection.lock_with_tab_id().await,
    };
    let snapshot = connection.versioned_snapshot();
    Ok(FrontendDesktopSnapshot::from_versioned_for_account(
        snapshot.state,
        snapshot.generation,
        account_tab_id,
    ))
}

/// Recover a frontend-detected state/timeline gap from one exact Core snapshot.
///
/// The snapshot is captured before submitting the single replay command so its
/// generation is the generation returned to the caller, not a later watch value.
#[tauri::command]
pub async fn resync_snapshot(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
    account_tab_id: Option<String>,
) -> Result<FrontendDesktopSnapshot, String> {
    let (account_tab_id, versioned_snapshot, request_id) = {
        let (account_tab_id, connection) = match account_tab_id {
            Some(id) => {
                state
                    .inner()
                    .connection
                    .lock_for_tab_id(
                        &koushi_core::account_runtime_manager::AccountTabId::from_string(id),
                    )
                    .await?
            }
            None => state.inner().connection.lock_with_tab_id().await,
        };
        (
            account_tab_id,
            connection.versioned_snapshot(),
            connection.next_request_id(),
        )
    };
    submit_core_command(
        state.inner(),
        CoreCommand::Timeline(TimelineCommand::ReplaySubscribed { request_id }),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(FrontendDesktopSnapshot::from_versioned_for_account(
        versioned_snapshot.state,
        versioned_snapshot.generation,
        account_tab_id,
    ))
}

#[tauri::command]
pub async fn discover_login_methods(
    account_tab_id: Option<String>,
    homeserver: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut wait_conn = account_connection(state.inner(), account_tab_id.as_deref()).await?;
    let baseline_generation = wait_conn.state_generation();
    let request_id = next_request_id_for(state.inner(), account_tab_id.as_deref()).await?;
    submit_core_command(
        state.inner(),
        build_discover_login_command(request_id, homeserver.clone()),
    )
    .await?;
    let outcome = wait_conn
        .wait_for_request_outcome(
            OutcomeCorrelation::Request(request_id),
            RequestOutcomeExpectation::AuthDiscovery {
                request_id,
                homeserver,
            },
            baseline_generation,
            tokio::time::Instant::now() + LOGIN_EVENT_TIMEOUT,
        )
        .await
        .map_err(|error| invoke_error_from_request_outcome("login discovery", error))?;
    let RequestOutcome::AuthDiscovery { generation, .. } = outcome else {
        return Err("login discovery returned an invalid outcome".to_owned());
    };
    Ok(command_settlement(generation))
}

#[tauri::command]
pub async fn start_oidc_login(
    homeserver: String,
    account_tab_id: Option<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<OidcBrowserLaunchResponse, String> {
    let (tab_id, submit_conn) = match account_tab_id {
        Some(id) => {
            state
                .inner()
                .connection
                .lock_for_tab_id(
                    &koushi_core::account_runtime_manager::AccountTabId::from_string(id),
                )
                .await?
        }
        None => state.inner().connection.lock_with_tab_id().await,
    };
    let id = koushi_core::account_runtime_manager::AccountTabId::from_string(tab_id.clone());
    let mut wait_conn = state
        .inner()
        .runtime
        .tab_connection(&id)
        .ok_or_else(|| "account tab does not exist".to_owned())?;
    let baseline_generation = wait_conn.state_generation();
    let request_id = submit_conn.next_request_id();
    tokio::time::timeout(
        CORE_COMMAND_SUBMIT_TIMEOUT,
        submit_conn.command(build_start_oidc_login_command(request_id, homeserver)),
    )
    .await
    .map_err(|_| "command submit timed out".to_owned())?
    .map_err(|error| format!("command submit failed: {error}"))?;
    drop(submit_conn);
    let outcome = wait_conn
        .wait_for_request_outcome(
            OutcomeCorrelation::Request(request_id),
            RequestOutcomeExpectation::OidcAuthorization { request_id },
            baseline_generation,
            tokio::time::Instant::now() + LOGIN_EVENT_TIMEOUT,
        )
        .await
        .map_err(|error| invoke_error_from_request_outcome("OIDC login", error))?;
    let RequestOutcome::OidcAuthorization {
        authorization_url,
        state: oidc_state,
        generation,
        ..
    } = outcome
    else {
        return Err("OIDC login returned an invalid outcome".to_owned());
    };
    state.inner().runtime.register_oidc_attempt(
        &koushi_core::account_runtime_manager::AccountTabId::from_string(tab_id),
        oidc_state,
    );
    record(DiagnosticEvent::new(
        DiagnosticLevel::Info,
        "desktop.oidc_browser",
        "authorization_created",
    ));
    let outcome = match launch_oidc_browser(&app, &authorization_url) {
        Ok(()) => OidcBrowserLaunchOutcome::Launched,
        Err(OidcBrowserLaunchFailure::InvalidAuthorizationUrl) => {
            OidcBrowserLaunchOutcome::InvalidAuthorizationUrl
        }
        Err(OidcBrowserLaunchFailure::BrowserLaunchFailed) => {
            OidcBrowserLaunchOutcome::BrowserLaunchFailed
        }
    };
    let stage = match outcome {
        OidcBrowserLaunchOutcome::Launched => "browser_launch_succeeded",
        OidcBrowserLaunchOutcome::InvalidAuthorizationUrl => "url_rejected",
        OidcBrowserLaunchOutcome::BrowserLaunchFailed => "browser_launch_failed",
    };
    record(DiagnosticEvent::new(
        if outcome == OidcBrowserLaunchOutcome::Launched {
            DiagnosticLevel::Info
        } else {
            DiagnosticLevel::Warn
        },
        "desktop.oidc_browser",
        stage,
    ));
    Ok(OidcBrowserLaunchResponse {
        outcome,
        settlement: FrontendCommandSettlement::from_published_generation(generation),
    })
}

fn launch_oidc_browser(
    app: &AppHandle,
    authorization_url: &str,
) -> Result<(), OidcBrowserLaunchFailure> {
    #[cfg(target_os = "linux")]
    {
        let request_recorded = std::rc::Rc::new(std::cell::Cell::new(false));
        let native_recorded = std::rc::Rc::clone(&request_recorded);
        let native = move |url: &str| {
            if !native_recorded.replace(true) {
                record(DiagnosticEvent::new(
                    DiagnosticLevel::Info,
                    "desktop.oidc_browser",
                    "browser_launch_requested",
                ));
            }
            app.opener().open_url(url, None::<&str>).map_err(|_| ())
        };
        let linux_recorded = std::rc::Rc::clone(&request_recorded);
        let linux = move |url: &str| {
            if !linux_recorded.replace(true) {
                record(DiagnosticEvent::new(
                    DiagnosticLevel::Info,
                    "desktop.oidc_browser",
                    "browser_launch_requested",
                ));
            }
            launch_linux_default_browser(url).map_err(|_| ())
        };
        if running_under_wsl() {
            launch_oidc_authorization_url_with_fallback(authorization_url, linux, native)
        } else {
            launch_oidc_authorization_url_with_fallback(authorization_url, native, linux)
        }
    }

    #[cfg(not(target_os = "linux"))]
    launch_oidc_authorization_url(authorization_url, |url| {
        record(DiagnosticEvent::new(
            DiagnosticLevel::Info,
            "desktop.oidc_browser",
            "browser_launch_requested",
        ));
        app.opener().open_url(url, None::<&str>)
    })
}

#[tauri::command]
pub async fn complete_oidc_login(
    _homeserver: String,
    callback_url: String,
    account_tab_id: Option<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    // The callback shape decides the correlation kind; the account-tab manager
    // decides whether it matches a pending attempt. OAuth callbacks must repeat
    // their CSRF state, while a legacy `m.login.sso` callback carries only
    // `loginToken` and is accepted only for the single pending legacy attempt.
    let correlation = crate::oidc_callback_correlation(&callback_url)
        .ok_or_else(|| "OIDC callback is malformed or unsolicited".to_owned())?;
    let tab_id = state
        .inner()
        .runtime
        .oidc_attempt_tab(&correlation)
        .ok_or_else(|| "OIDC callback does not match a pending login".to_owned())?;
    if account_tab_id
        .as_deref()
        .is_some_and(|expected| expected != tab_id.as_str())
    {
        return Err("OIDC callback belongs to a different account tab".to_owned());
    }
    let tab_id = state
        .inner()
        .runtime
        .take_oidc_attempt(correlation)
        .ok_or_else(|| "OIDC callback was already consumed".to_owned())?;
    let (_, submit_conn) = state.inner().connection.lock_for_tab_id(&tab_id).await?;
    let mut wait_conn = state
        .inner()
        .runtime
        .tab_connection(&tab_id)
        .ok_or_else(|| "account tab does not exist".to_owned())?;
    let baseline_generation = wait_conn.state_generation();
    let account_key = account_key_from_app_state(&wait_conn.snapshot());
    let account_key = (!account_key.0.is_empty()).then_some(account_key);
    let request_id = submit_conn.next_request_id();
    tokio::time::timeout(
        CORE_COMMAND_SUBMIT_TIMEOUT,
        submit_conn.command(build_complete_oidc_login_command(
            request_id,
            callback_url,
            crate::dto::frontend_display_platform(),
        )),
    )
    .await
    .map_err(|_| "command submit timed out".to_owned())?
    .map_err(|error| format!("command submit failed: {error}"))?;
    drop(submit_conn);
    let outcome = wait_conn
        .wait_for_request_outcome(
            OutcomeCorrelation::Request(request_id),
            RequestOutcomeExpectation::Authenticated {
                request_id,
                account_key,
            },
            baseline_generation,
            tokio::time::Instant::now() + LOGIN_EVENT_TIMEOUT,
        )
        .await
        .map_err(|error| invoke_error_from_request_outcome("OIDC login", error))?;
    let RequestOutcome::Authenticated { generation, .. } = outcome else {
        return Err("OIDC login returned an invalid outcome".to_owned());
    };
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(command_settlement(generation))
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn submit_login(
    account_tab_id: Option<String>,
    homeserver: String,
    username: String,
    password: String,
    device_display_name: Option<String>,
    platform: DisplayPlatform,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let login_request = LoginRequest {
        homeserver,
        username,
        password: AuthSecret::new(password),
        device_display_name,
    };
    let generation = submit_login_request(
        app,
        state.inner(),
        account_tab_id.as_deref(),
        login_request,
        platform,
    )
    .await?;
    Ok(command_settlement(generation))
}

#[tauri::command]
pub async fn submit_soft_logout_reauth(
    account_tab_id: Option<String>,
    password: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let generation = submit_soft_logout_reauth_request(
        app,
        state.inner(),
        account_tab_id.as_deref(),
        AuthSecret::new(password),
    )
    .await?;
    Ok(command_settlement(generation))
}

#[tauri::command]
pub async fn list_saved_sessions(
    account_tab_id: Option<String>,
    state: State<'_, CoreRuntimeState>,
) -> Result<Vec<SessionInfo>, String> {
    // GUI-smoke toggle: skip the keychain-backed query entirely.
    if crate::saved_sessions_disabled_from_env() {
        return Ok(Vec::new());
    }

    let mut wait_conn = account_connection(state.inner(), account_tab_id.as_deref()).await?;
    let baseline_generation = wait_conn.state_generation();
    let request_id = next_request_id_for(state.inner(), account_tab_id.as_deref()).await?;
    submit_core_command(
        state.inner(),
        CoreCommand::Account(AccountCommand::QuerySavedSessions { request_id }),
    )
    .await?;
    let outcome = wait_conn
        .wait_for_request_outcome(
            OutcomeCorrelation::Request(request_id),
            RequestOutcomeExpectation::SavedSessions { request_id },
            baseline_generation,
            tokio::time::Instant::now() + SAVED_SESSIONS_EVENT_TIMEOUT,
        )
        .await
        .map_err(|error| invoke_error_from_request_outcome("saved sessions", error))?;
    let RequestOutcome::SavedSessions { sessions, .. } = outcome else {
        return Err("saved sessions returned an invalid outcome".to_owned());
    };
    Ok(sessions)
}

#[tauri::command]
pub async fn switch_account(
    account_tab_id: Option<String>,
    homeserver: String,
    user_id: String,
    device_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut wait_conn = account_connection(state.inner(), account_tab_id.as_deref()).await?;
    let baseline_generation = wait_conn.state_generation();
    let request_id = next_request_id_for(state.inner(), account_tab_id.as_deref()).await?;
    submit_core_command(
        state.inner(),
        build_switch_account_command(request_id, user_id.clone()),
    )
    .await?;
    let outcome = wait_conn
        .wait_for_request_outcome(
            OutcomeCorrelation::Request(request_id),
            RequestOutcomeExpectation::Authenticated {
                request_id,
                account_key: Some(AccountKey(user_id)),
            },
            baseline_generation,
            tokio::time::Instant::now() + LOGIN_EVENT_TIMEOUT,
        )
        .await
        .map_err(|error| invoke_error_from_request_outcome("account switch", error))?;
    let RequestOutcome::Authenticated { generation, .. } = outcome else {
        return Err("account switch returned an invalid outcome".to_owned());
    };
    // AccountKey canonically identifies the account by user_id.
    let _ = (homeserver, device_id);
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(command_settlement(generation))
}

#[tauri::command]
pub async fn submit_recovery(
    account_tab_id: Option<String>,
    secret: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    submit_recovery_request(
        app,
        state.inner(),
        account_tab_id.as_deref(),
        AuthSecret::new(secret),
    )
    .await
}

#[tauri::command]
pub async fn start_device_cleanup(
    account_tab_id: Option<String>,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id_for(state.inner(), account_tab_id.as_deref()).await?;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_start_device_cleanup_command(request_id),
    )
    .await?;
    Ok(admission)
}

#[tauri::command]
pub async fn submit_device_cleanup_uia(
    account_tab_id: Option<String>,
    flow_id: u64,
    password: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id_for(state.inner(), account_tab_id.as_deref()).await?;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_submit_device_cleanup_uia_command(request_id, flow_id, AuthSecret::new(password)),
    )
    .await?;
    Ok(admission)
}

#[tauri::command]
pub async fn erase_local_data_anyway(
    account_tab_id: Option<String>,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id_for(state.inner(), account_tab_id.as_deref()).await?;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_erase_device_cleanup_local_data_anyway_command(request_id),
    )
    .await?;
    Ok(admission)
}

#[tauri::command]
pub async fn logout(
    account_tab_id: Option<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut wait_conn = account_connection(state.inner(), account_tab_id.as_deref()).await?;
    let baseline = wait_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline.state);
    let request_id = next_request_id_for(state.inner(), account_tab_id.as_deref()).await?;
    submit_core_command(state.inner(), build_logout_command(request_id)).await?;
    let outcome = wait_conn
        .wait_for_request_outcome(
            OutcomeCorrelation::Request(request_id),
            RequestOutcomeExpectation::SignedOut {
                request_id,
                account_key,
                allow_projection_only: false,
            },
            baseline.generation,
            tokio::time::Instant::now() + LOGIN_EVENT_TIMEOUT,
        )
        .await
        .map_err(|error| invoke_error_from_request_outcome("logout", error))?;
    let RequestOutcome::SignedOut { generation, .. } = outcome else {
        return Err("logout returned an invalid outcome".to_owned());
    };
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(command_settlement(generation))
}

#[tauri::command]
pub async fn retry_sliding_sync_capability(
    account_tab_id: Option<String>,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id_for(state.inner(), account_tab_id.as_deref()).await?;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_retry_sliding_sync_capability_command(request_id),
    )
    .await?;
    Ok(admission)
}

#[tauri::command]
pub async fn change_homeserver(
    account_tab_id: Option<String>,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id_for(state.inner(), account_tab_id.as_deref()).await?;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_change_homeserver_command(request_id),
    )
    .await?;
    Ok(admission)
}

#[tauri::command]
pub async fn restart_sync(
    account_tab_id: Option<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id_for(state.inner(), account_tab_id.as_deref()).await?;
    let admission =
        submit_core_command_with_admission(state.inner(), build_restart_sync_command(request_id))
            .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

pub(super) async fn submit_login_request(
    app: AppHandle,
    state: &CoreRuntimeState,
    account_tab_id: Option<&str>,
    login_request: LoginRequest,
    platform: DisplayPlatform,
) -> Result<u64, String> {
    submit_login_and_wait_for_authenticated(app, state, account_tab_id, login_request, platform)
        .await
}

pub(super) async fn submit_soft_logout_reauth_request(
    app: AppHandle,
    state: &CoreRuntimeState,
    account_tab_id: Option<&str>,
    password: AuthSecret,
) -> Result<u64, String> {
    let mut wait_conn = account_connection(state, account_tab_id).await?;
    let baseline_generation = wait_conn.state_generation();
    let account_key = account_key_from_app_state(&wait_conn.snapshot());
    let request_id = next_request_id_for(state, account_tab_id).await?;
    submit_core_command(
        state,
        build_submit_soft_logout_reauth_command(request_id, password),
    )
    .await?;

    let outcome = wait_conn
        .wait_for_request_outcome(
            OutcomeCorrelation::Request(request_id),
            RequestOutcomeExpectation::Authenticated {
                request_id,
                account_key: Some(account_key),
            },
            baseline_generation,
            tokio::time::Instant::now() + LOGIN_EVENT_TIMEOUT,
        )
        .await
        .map_err(|error| invoke_error_from_request_outcome("reauthentication", error))?;
    let RequestOutcome::Authenticated { generation, .. } = outcome else {
        return Err("reauthentication returned an invalid outcome".to_owned());
    };
    update_qa_window_title_from_state(&app, state).await;
    Ok(generation)
}

const LOGIN_EVENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

async fn submit_login_and_wait_for_authenticated(
    app: AppHandle,
    state: &CoreRuntimeState,
    account_tab_id: Option<&str>,
    login_request: LoginRequest,
    platform: DisplayPlatform,
) -> Result<u64, String> {
    // Use a dedicated connection so the event cursor is attached before the
    // login command is submitted and the correlated LoggedIn event cannot be
    // missed by this product path.
    let mut wait_conn = account_connection(state, account_tab_id).await?;
    let current_tab = account_tab_id
        .map(|id| koushi_core::account_runtime_manager::AccountTabId::from_string(id.to_owned()))
        .unwrap_or_else(|| state.runtime.selected_tab_id());
    if let Some(existing) = state.runtime.account_tab_for_existing_password_login(
        &current_tab,
        &login_request.homeserver,
        &login_request.username,
    ) {
        let state = app.state::<CoreRuntimeState>();
        super::account_tabs::select_account_tab(app.clone(), state, existing.as_str().to_owned())
            .await?;
        return Err("account is already open in another tab".to_owned());
    }
    let baseline_generation = wait_conn.state_generation();
    let account_key = account_key_from_app_state(&wait_conn.snapshot());
    let account_key = (!account_key.0.is_empty()).then_some(account_key);
    let login_request_id = next_request_id_for(state, account_tab_id).await?;
    submit_core_command(
        state,
        build_submit_login_command(login_request_id, login_request, platform),
    )
    .await?;

    let outcome = wait_conn
        .wait_for_request_outcome(
            OutcomeCorrelation::Request(login_request_id),
            RequestOutcomeExpectation::Authenticated {
                request_id: login_request_id,
                account_key,
            },
            baseline_generation,
            tokio::time::Instant::now() + LOGIN_EVENT_TIMEOUT,
        )
        .await
        .map_err(|error| invoke_error_from_request_outcome("login", error))?;
    let RequestOutcome::Authenticated { generation, .. } = outcome else {
        return Err("login returned an invalid outcome".to_owned());
    };
    update_qa_window_title_from_state(&app, state).await;
    Ok(generation)
}

/// How long the adapter waits for the `SavedSessionsListed` answer before
/// reporting a transport error. The query is a local credential-store read in
/// core, so 5 seconds is generous.
const SAVED_SESSIONS_EVENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

pub(super) async fn submit_recovery_request(
    app: AppHandle,
    state: &CoreRuntimeState,
    account_tab_id: Option<&str>,
    secret: AuthSecret,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id_for(state, account_tab_id).await?;
    let admission = submit_core_command_with_admission(
        state,
        build_submit_recovery_command(request_id, secret),
    )
    .await?;
    update_qa_window_title_from_state(&app, state).await;
    Ok(admission)
}

pub(super) fn build_submit_login_command(
    request_id: koushi_protocol::RequestId,
    login_request: LoginRequest,
    platform: DisplayPlatform,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::LoginPassword {
        request_id,
        request: login_request,
        platform,
    })
}

pub(super) fn build_submit_soft_logout_reauth_command(
    request_id: koushi_protocol::RequestId,
    password: AuthSecret,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::SoftLogoutReauth {
        request_id,
        password,
    })
}

pub(super) fn build_discover_login_command(
    request_id: koushi_protocol::RequestId,
    homeserver: String,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::DiscoverLogin {
        request_id,
        homeserver,
    })
}

pub(super) fn build_start_oidc_login_command(
    request_id: koushi_protocol::RequestId,
    homeserver: String,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::StartOidcLogin {
        request_id,
        homeserver,
    })
}

pub(crate) fn build_complete_oidc_login_command(
    request_id: koushi_protocol::RequestId,
    callback_url: String,
    platform: DisplayPlatform,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::CompleteOidcLogin {
        request_id,
        callback_url,
        platform,
    })
}

pub(super) fn build_switch_account_command(
    request_id: koushi_protocol::RequestId,
    user_id: String,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::SwitchAccount {
        request_id,
        account_key: AccountKey(user_id),
    })
}

pub(super) fn build_submit_recovery_command(
    request_id: koushi_protocol::RequestId,
    secret: AuthSecret,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::SubmitRecovery {
        request_id,
        request: RecoveryRequest { secret },
    })
}

pub(super) fn build_logout_command(request_id: koushi_protocol::RequestId) -> CoreCommand {
    CoreCommand::Account(AccountCommand::Logout { request_id })
}

pub(super) fn build_retry_sliding_sync_capability_command(
    request_id: koushi_protocol::RequestId,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::RetrySlidingSyncCapability { request_id })
}

pub(super) fn build_change_homeserver_command(
    request_id: koushi_protocol::RequestId,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::ChangeHomeserver { request_id })
}

pub(super) fn build_restart_sync_command(request_id: koushi_protocol::RequestId) -> CoreCommand {
    CoreCommand::Sync(SyncCommand::Restart { request_id })
}
