use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use futures_util::StreamExt;
use matrix_desktop_state::{
    AppAction, AppEffect, AppState, AuthSecret, LoginRequest, RecoveryRequest, SessionInfo,
    SessionState,
};
use serde::Deserialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::{
    BackendState, TimelinePaginationRequest, TimelineTaskHandle,
    dto::{FrontendDesktopSnapshot, SearchScopeKind},
};

const MATRIX_SYNC_RETRY_DELAY: Duration = Duration::from_secs(2);
const MATRIX_TIMELINE_BACK_PAGINATION_EVENT_COUNT: u16 = 30;
const QA_TIMELINE_BACKFILL_EVENT_COUNT: u16 = 30;
const QA_TIMELINE_ROOM_SAMPLE_LIMIT: usize = 20;
const QA_TIMELINE_ROOM_SAMPLE_TIMEOUT: Duration = Duration::from_secs(8);
const QA_RECOVERY_PROMPT_TIMEOUT: Duration = Duration::from_secs(60);
const QA_TITLE_ENV: &str = "MATRIX_DESKTOP_QA_TITLE";
const STATE_EVENT_NAME: &str = "matrix-desktop://state";
const MATRIX_SEND_TEXT_FAILED_MESSAGE: &str = "Matrix send failed";
static NEXT_TRANSACTION_ID: AtomicU64 = AtomicU64::new(1);

#[tauri::command]
pub fn get_snapshot(state: State<'_, BackendState>) -> Result<FrontendDesktopSnapshot, String> {
    let backend = state.backend.lock().map_err(lock_error)?;
    Ok(FrontendDesktopSnapshot::from(backend.snapshot()))
}

#[tauri::command]
pub fn discover_login_methods(
    homeserver: String,
    state: State<'_, BackendState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut backend = state.backend.lock().map_err(lock_error)?;
    backend.dispatch(AppAction::LoginDiscoveryRequested { homeserver });
    Ok(FrontendDesktopSnapshot::from(backend.snapshot()))
}

#[tauri::command]
pub async fn submit_login(
    homeserver: String,
    username: String,
    password: String,
    device_display_name: Option<String>,
    app: AppHandle,
    state: State<'_, BackendState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let login_request = LoginRequest {
        homeserver,
        username,
        password: AuthSecret::new(password),
        device_display_name,
    };
    submit_login_request(app, state.inner(), login_request).await?;

    let backend = state.backend.lock().map_err(lock_error)?;
    Ok(FrontendDesktopSnapshot::from(backend.snapshot()))
}

pub(crate) async fn submit_login_request(
    app: AppHandle,
    state: &BackendState,
    login_request: LoginRequest,
) -> Result<(), String> {
    let deferred_login = {
        let mut backend = state.backend.lock().map_err(lock_error)?;
        let effects = backend.dispatch(AppAction::LoginSubmitted(login_request));
        emit_ui_events(&app, &effects);
        let deferred_login = deferred_login_request(effects, backend.state());
        let snapshot = backend.snapshot();
        drop(backend);
        update_qa_window_title(&app, &snapshot);
        deferred_login
    };

    if let Some(login_request) = deferred_login {
        let login_result = matrix_desktop_auth::login_with_password(&login_request).await;

        match login_result {
            Ok(session) => {
                if crate::qa_skips_keychain_persistence_from_env() {
                    complete_login_with_session(app.clone(), state, session, None)?;
                } else {
                    let persistence_error = crate::persist_matrix_session(&session).err();
                    match crate::restore_matrix_session_with_local_store(&session).await {
                        Ok(stored_session) => {
                            complete_login_with_session(
                                app.clone(),
                                state,
                                stored_session,
                                persistence_error,
                            )?;
                        }
                        Err(error) => {
                            let (effects, snapshot) = {
                                let mut backend = state.backend.lock().map_err(lock_error)?;
                                let effects = backend.fail_login(error);
                                let snapshot = backend.snapshot();
                                (effects, snapshot)
                            };
                            emit_ui_events(&app, &effects);
                            update_qa_window_title(&app, &snapshot);
                        }
                    }
                }
            }
            Err(error) => {
                let (effects, snapshot) = {
                    let mut backend = state.backend.lock().map_err(lock_error)?;
                    let effects = backend.fail_login(error.to_string());
                    let snapshot = backend.snapshot();
                    (effects, snapshot)
                };
                emit_ui_events(&app, &effects);
                update_qa_window_title(&app, &snapshot);
            }
        }
    }

    Ok(())
}

fn complete_login_with_session(
    app: AppHandle,
    state: &BackendState,
    session: matrix_desktop_auth::MatrixClientSession,
    persistence_error: Option<String>,
) -> Result<(), String> {
    let recovery_observer_session = session.clone();
    let matrix_sync_session = session.clone();
    let (should_start_sync, snapshot) = {
        let mut backend = state.backend.lock().map_err(lock_error)?;
        let effects = backend.complete_matrix_login(session);
        if let Some(message) = persistence_error {
            let persistence_effects = backend.record_session_persistence_failure(message);
            emit_ui_events(&app, &persistence_effects);
        }
        emit_ui_events(&app, &effects);
        (effects_include_start_sync(&effects), backend.snapshot())
    };
    update_qa_window_title(&app, &snapshot);
    if should_start_sync {
        start_matrix_sync_task(app.clone(), matrix_sync_session)?;
    }
    spawn_e2ee_recovery_state_observer(app, recovery_observer_session);
    Ok(())
}

#[derive(Deserialize)]
struct QaLoginPipePayload {
    homeserver: String,
    username: String,
    password: String,
    device_display_name: Option<String>,
    recovery_secret: Option<String>,
}

#[derive(Debug)]
pub(crate) struct QaLoginPipeRequest {
    pub login: LoginRequest,
    pub recovery_secret: Option<AuthSecret>,
}

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

pub(crate) fn spawn_qa_login_pipe_reader(app: AppHandle, pipe_path: PathBuf) {
    tauri::async_runtime::spawn(async move {
        let payload = match read_qa_login_pipe(pipe_path).await {
            Ok(payload) => payload,
            Err(message) => {
                record_qa_login_failure(&app, message);
                return;
            }
        };
        let request = match parse_qa_login_pipe_payload(&payload) {
            Ok(request) => request,
            Err(message) => {
                record_qa_login_failure(&app, message);
                return;
            }
        };
        let state = app.state::<BackendState>();
        if let Err(message) = submit_login_request(app.clone(), state.inner(), request.login).await
        {
            record_qa_login_failure(&app, message);
            return;
        }
        if let Some(recovery_secret) = request.recovery_secret {
            if let Err(message) =
                wait_for_qa_recovery_prompt(&app, state.inner(), QA_RECOVERY_PROMPT_TIMEOUT).await
            {
                record_qa_login_failure(&app, message);
                return;
            }
            if let Err(message) =
                submit_recovery_request(app.clone(), state.inner(), recovery_secret).await
            {
                record_qa_login_failure(&app, message);
            }
        }
    });
}

async fn read_qa_login_pipe(pipe_path: PathBuf) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::read_to_string(pipe_path).map_err(|_| "QA login pipe could not be read".to_owned())
    })
    .await
    .map_err(|_| "QA login pipe reader failed".to_owned())?
}

fn record_qa_login_failure(app: &AppHandle, message: String) {
    let state = app.state::<BackendState>();
    if let Ok(mut backend) = state.backend.lock() {
        let effects = backend.fail_login(message);
        let snapshot = backend.snapshot();
        drop(backend);
        emit_ui_events(app, &effects);
        update_qa_window_title(app, &snapshot);
    }
}

async fn wait_for_qa_recovery_prompt(
    app: &AppHandle,
    state: &BackendState,
    timeout: Duration,
) -> Result<(), String> {
    let started_at = std::time::Instant::now();
    while started_at.elapsed() < timeout {
        let snapshot = {
            let backend = state.backend.lock().map_err(lock_error)?;
            let snapshot = backend.snapshot();
            if qa_recovery_prompt_is_available(&snapshot.state) {
                update_qa_window_title(app, &snapshot);
                return Ok(());
            }
            snapshot
        };
        update_qa_window_title(app, &snapshot);
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err("QA recovery prompt did not become available".to_owned())
}

pub(crate) fn qa_recovery_prompt_is_available(state: &AppState) -> bool {
    matches!(state.session, SessionState::NeedsRecovery { .. })
}

pub(crate) fn spawn_e2ee_recovery_state_observer(
    app: AppHandle,
    session: matrix_desktop_auth::MatrixClientSession,
) {
    tauri::async_runtime::spawn(async move {
        let mut stream = session.e2ee_recovery_state_stream();
        while let Some(recovery_state) = stream.next().await {
            let backend_state = app.state::<BackendState>();
            let Ok(mut backend) = backend_state.backend.lock() else {
                break;
            };
            backend.observe_e2ee_recovery_state(recovery_state);
        }
    });
}

pub(crate) fn start_matrix_sync_task(
    app: AppHandle,
    session: matrix_desktop_auth::MatrixClientSession,
) -> Result<(), String> {
    let app_for_task = app.clone();
    let task = tauri::async_runtime::spawn(async move {
        loop {
            let failure_reason = match matrix_desktop_auth::sync_once(&session).await {
                Ok(()) => {
                    if let Err(error) = dispatch_matrix_room_list_snapshot(
                        &app_for_task,
                        &session,
                    )
                    .await
                    {
                        error
                    } else {
                        dispatch_matrix_sync_action(&app_for_task, AppAction::SyncStarted);
                        continue;
                    }
                }
                Err(error) => error.to_string(),
            };
            let effects = dispatch_matrix_sync_action(
                &app_for_task,
                AppAction::SyncFailed {
                    reason: failure_reason.clone(),
                },
            );
            if !effects_include_start_sync(&effects) {
                break;
            }
            dispatch_matrix_sync_action(
                &app_for_task,
                AppAction::SyncReconnecting {
                    reason: failure_reason,
                },
            );
            tokio::time::sleep(MATRIX_SYNC_RETRY_DELAY).await;
        }
    });

    let backend_state = app.state::<BackendState>();
    let mut sync_task = backend_state.sync_task.lock().map_err(lock_error)?;
    if let Some(previous_task) = sync_task.take() {
        previous_task.abort();
    }
    *sync_task = Some(task);
    Ok(())
}

async fn dispatch_matrix_room_list_snapshot(
    app: &AppHandle,
    session: &matrix_desktop_auth::MatrixClientSession,
) -> Result<(), String> {
    let snapshot = matrix_desktop_auth::room_list_snapshot(session)
        .await
        .map_err(|error| error.to_string())?;
    let snapshot = qa_room_list_snapshot_with_visible_timeline_first(session, snapshot).await;
    let update = matrix_room_list_snapshot_to_backend_update(snapshot);
    let backend_state = app.state::<BackendState>();
    let (effects, backend_snapshot) = {
        let mut backend = backend_state.backend.lock().map_err(lock_error)?;
        let effects = backend.dispatch(matrix_desktop_backend::compose_room_list_update(update));
        let snapshot = backend.snapshot();
        (effects, snapshot)
    };
    emit_ui_events(app, &effects);
    update_qa_window_title(app, &backend_snapshot);
    if let Some(room_id) = room_list_sync_follow_up(&effects) {
        start_matrix_timeline_task(app.clone(), session.clone(), room_id)?;
    }
    Ok(())
}

async fn qa_room_list_snapshot_with_visible_timeline_first(
    session: &matrix_desktop_auth::MatrixClientSession,
    mut snapshot: matrix_desktop_auth::MatrixRoomListSnapshot,
) -> matrix_desktop_auth::MatrixRoomListSnapshot {
    if !qa_window_title_enabled() {
        return snapshot;
    }
    if let Some(room_id) = qa_visible_timeline_room_id(session, &snapshot).await {
        promote_room_to_front(&mut snapshot, &room_id);
    }
    snapshot
}

async fn qa_visible_timeline_room_id(
    session: &matrix_desktop_auth::MatrixClientSession,
    snapshot: &matrix_desktop_auth::MatrixRoomListSnapshot,
) -> Option<String> {
    for room in snapshot.rooms.iter().take(QA_TIMELINE_ROOM_SAMPLE_LIMIT) {
        let items = tokio::time::timeout(
            QA_TIMELINE_ROOM_SAMPLE_TIMEOUT,
            matrix_desktop_auth::room_timeline_visible_items(
                session,
                &room.room_id,
                QA_TIMELINE_BACKFILL_EVENT_COUNT,
            ),
        )
        .await;
        if let Ok(Ok(items)) = items
            && !items.is_empty()
        {
            return Some(room.room_id.clone());
        }
    }
    None
}

pub(crate) fn promote_room_to_front(
    snapshot: &mut matrix_desktop_auth::MatrixRoomListSnapshot,
    room_id: &str,
) {
    let Some(index) = snapshot
        .rooms
        .iter()
        .position(|room| room.room_id == room_id)
    else {
        return;
    };
    let room = snapshot.rooms.remove(index);
    snapshot.rooms.insert(0, room);
}

fn matrix_room_list_snapshot_to_backend_update(
    snapshot: matrix_desktop_auth::MatrixRoomListSnapshot,
) -> matrix_desktop_backend::DesktopRoomListUpdate {
    matrix_desktop_backend::DesktopRoomListUpdate {
        spaces: snapshot
            .spaces
            .into_iter()
            .map(|space| matrix_desktop_backend::DesktopRoomListSpace {
                space_id: space.space_id,
                display_name: space.display_name,
            })
            .collect(),
        rooms: snapshot
            .rooms
            .into_iter()
            .map(|room| matrix_desktop_backend::DesktopRoomListRoom {
                room_id: room.room_id,
                display_name: room.display_name,
                is_dm: room.is_dm,
                unread_count: room.unread_count,
                parent_space_ids: room.parent_space_ids,
            })
            .collect(),
    }
}

fn dispatch_matrix_sync_action(app: &AppHandle, action: AppAction) -> Vec<AppEffect> {
    let backend_state = app.state::<BackendState>();
    let Ok(mut backend) = backend_state.backend.lock() else {
        return Vec::new();
    };
    let effects = backend.dispatch(action);
    let snapshot = backend.snapshot();
    drop(backend);
    emit_ui_events(app, &effects);
    update_qa_window_title(app, &snapshot);
    effects
}

pub(crate) fn abort_matrix_sync_task(state: &BackendState) -> Result<(), String> {
    let mut sync_task = state.sync_task.lock().map_err(lock_error)?;
    if let Some(task) = sync_task.take() {
        task.abort();
    }
    Ok(())
}

pub(crate) fn start_matrix_timeline_task(
    app: AppHandle,
    session: matrix_desktop_auth::MatrixClientSession,
    room_id: String,
) -> Result<(), String> {
    let app_for_task = app.clone();
    let task_room_id = room_id.clone();
    let (pagination_sender, mut pagination_receiver) =
        tokio::sync::mpsc::channel::<TimelinePaginationRequest>(8);
    let task = tauri::async_runtime::spawn(async move {
        match matrix_desktop_auth::subscribe_room_timeline(&session, &task_room_id).await {
            Ok(mut subscription) => {
                let pagination_handle = subscription.pagination_handle();
                let pagination_app = app_for_task.clone();
                let pagination_room_id = task_room_id.clone();
                tauri::async_runtime::spawn(async move {
                    while let Some(request) = pagination_receiver.recv().await {
                        if request.room_id != pagination_room_id {
                            continue;
                        }
                        let _ = pagination_handle
                            .paginate_backwards(request.event_count)
                            .await;
                        dispatch_timeline_action(
                            &pagination_app,
                            AppAction::TimelineBackPaginationFinished {
                                room_id: request.room_id,
                            },
                        );
                    }
                });

                dispatch_timeline_messages(
                    &app_for_task,
                    matrix_timeline_items_to_backend_messages(
                        subscription.initial_items().to_vec(),
                    ),
                );
                dispatch_timeline_action(
                    &app_for_task,
                    AppAction::TimelineSubscribed {
                        room_id: task_room_id.clone(),
                    },
                );
                while let Some(updates) = subscription.next_update().await {
                    dispatch_timeline_updates(
                        &app_for_task,
                        matrix_timeline_updates_to_backend_updates(updates),
                    );
                }
            }
            Err(error) => {
                dispatch_timeline_action(
                    &app_for_task,
                    AppAction::TimelineSubscriptionFailed {
                        room_id: task_room_id,
                        message: error.to_string(),
                    },
                );
            }
        }
    });

    let backend_state = app.state::<BackendState>();
    let mut timeline_task = backend_state.timeline_task.lock().map_err(lock_error)?;
    if let Some(previous_task) = timeline_task.take() {
        previous_task.task.abort();
    }
    *timeline_task = Some(TimelineTaskHandle {
        room_id,
        task,
        pagination_sender,
    });
    Ok(())
}

fn dispatch_timeline_action(app: &AppHandle, action: AppAction) -> Vec<AppEffect> {
    let backend_state = app.state::<BackendState>();
    let Ok(mut backend) = backend_state.backend.lock() else {
        return Vec::new();
    };
    let effects = backend.dispatch(action);
    let snapshot = backend.snapshot();
    drop(backend);
    emit_ui_events(app, &effects);
    update_qa_window_title(app, &snapshot);
    effects
}

fn dispatch_timeline_messages(
    app: &AppHandle,
    messages: Vec<matrix_desktop_backend::TimelineMessage>,
) {
    if messages.is_empty() {
        return;
    }

    let backend_state = app.state::<BackendState>();
    let Ok(mut backend) = backend_state.backend.lock() else {
        return;
    };
    if !timeline_messages_target_active_room(backend.state(), &messages) {
        return;
    }
    backend.upsert_timeline_messages(messages);
    let snapshot = backend.snapshot();
    drop(backend);
    emit_state_event(app, "timelineChanged");
    update_qa_window_title(app, &snapshot);
}

fn dispatch_timeline_updates(
    app: &AppHandle,
    updates: Vec<matrix_desktop_backend::TimelineUpdate>,
) {
    if updates.is_empty() {
        return;
    }

    let backend_state = app.state::<BackendState>();
    let Ok(mut backend) = backend_state.backend.lock() else {
        return;
    };
    if !timeline_updates_target_active_room(backend.state(), &updates) {
        return;
    }
    backend.apply_timeline_updates(updates);
    let snapshot = backend.snapshot();
    drop(backend);
    emit_state_event(app, "timelineChanged");
    update_qa_window_title(app, &snapshot);
}

pub(crate) fn abort_matrix_timeline_task(state: &BackendState) -> Result<(), String> {
    let mut timeline_task = state.timeline_task.lock().map_err(lock_error)?;
    if let Some(handle) = timeline_task.take() {
        handle.task.abort();
    }
    Ok(())
}

#[tauri::command]
pub fn list_saved_sessions() -> Result<Vec<SessionInfo>, String> {
    crate::saved_matrix_session_infos()
}

#[tauri::command]
pub async fn switch_account(
    homeserver: String,
    user_id: String,
    device_id: String,
    app: AppHandle,
    state: State<'_, BackendState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let target = SessionInfo {
        homeserver,
        user_id,
        device_id,
    };
    let restore_target = {
        let mut backend = state.backend.lock().map_err(lock_error)?;
        let effects = backend.dispatch(AppAction::SwitchAccountRequested { info: target });
        effects_restore_session_info(&effects)
    };

    let Some(restore_target) = restore_target else {
        let backend = state.backend.lock().map_err(lock_error)?;
        return Ok(FrontendDesktopSnapshot::from(backend.snapshot()));
    };

    abort_matrix_sync_task(state.inner())?;
    abort_matrix_timeline_task(state.inner())?;

    match crate::restore_persisted_matrix_session_for_info(&restore_target).await {
        Ok(restored_session) => {
            let recovery_observer_session = restored_session.clone();
            let matrix_sync_session = restored_session.clone();
            let persistence_error = crate::mark_last_matrix_session(&restore_target).err();
            let should_start_sync = {
                let mut backend = state.backend.lock().map_err(lock_error)?;
                let effects = backend.complete_matrix_restore(restored_session);
                if let Some(message) = persistence_error {
                    backend.record_session_persistence_failure(message);
                }
                effects_include_start_sync(&effects)
            };
            if should_start_sync {
                start_matrix_sync_task(app.clone(), matrix_sync_session)?;
            }
            spawn_e2ee_recovery_state_observer(app, recovery_observer_session);
        }
        Err(error) => {
            let mut backend = state.backend.lock().map_err(lock_error)?;
            backend.dispatch(AppAction::RestoreSessionFailed { message: error });
        }
    }

    let backend = state.backend.lock().map_err(lock_error)?;
    Ok(FrontendDesktopSnapshot::from(backend.snapshot()))
}

#[tauri::command]
pub async fn submit_recovery(
    secret: String,
    app: AppHandle,
    state: State<'_, BackendState>,
) -> Result<FrontendDesktopSnapshot, String> {
    submit_recovery_request(app, state.inner(), AuthSecret::new(secret)).await?;

    let backend = state.backend.lock().map_err(lock_error)?;
    Ok(FrontendDesktopSnapshot::from(backend.snapshot()))
}

pub(crate) async fn submit_recovery_request(
    app: AppHandle,
    state: &BackendState,
    secret: AuthSecret,
) -> Result<(), String> {
    let recovery_request = RecoveryRequest { secret };
    let deferred_recovery = {
        let mut backend = state.backend.lock().map_err(lock_error)?;
        let effects = backend.dispatch(AppAction::E2eeRecoverySubmitted(recovery_request));
        let deferred_recovery = deferred_recovery_request(effects.clone(), backend.state())
            .map(|request| (backend.matrix_session(), request));
        let snapshot = backend.snapshot();
        drop(backend);
        emit_ui_events(&app, &effects);
        update_qa_window_title(&app, &snapshot);
        deferred_recovery
    };

    if let Some((matrix_session, recovery_request)) = deferred_recovery {
        let recovery_result = match matrix_session {
            Some(matrix_session) => tauri::async_runtime::spawn_blocking(move || {
                matrix_desktop_auth::recover_e2ee_blocking(&matrix_session, &recovery_request)
            })
            .await
            .map_err(|error| format!("E2EE recovery task failed: {error}"))?,
            None => Err(matrix_desktop_auth::E2eeRecoveryError::Sdk(
                "Matrix session is not available for E2EE recovery".to_owned(),
            )),
        };

        let (effects, snapshot, should_start_sync, sync_session) = match recovery_result {
            Ok(()) => {
                let mut backend = state.backend.lock().map_err(lock_error)?;
                let effects = backend.dispatch(AppAction::E2eeRecoverySucceeded);
                let should_start_sync = effects_include_start_sync(&effects);
                let sync_session = backend.matrix_session();
                let snapshot = backend.snapshot();
                (effects, snapshot, should_start_sync, sync_session)
            }
            Err(error) => {
                let mut backend = state.backend.lock().map_err(lock_error)?;
                let effects = backend.dispatch(AppAction::E2eeRecoveryFailed {
                    message: error.to_string(),
                });
                let snapshot = backend.snapshot();
                (effects, snapshot, false, None)
            }
        };
        emit_ui_events(&app, &effects);
        update_qa_window_title(&app, &snapshot);
        if should_start_sync
            && let Some(sync_session) = sync_session
        {
            start_matrix_sync_task(app, sync_session)?;
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn logout(state: State<'_, BackendState>) -> Result<FrontendDesktopSnapshot, String> {
    abort_matrix_sync_task(state.inner())?;
    abort_matrix_timeline_task(state.inner())?;

    let (matrix_session, session_info) = {
        let backend = state.backend.lock().map_err(lock_error)?;
        let matrix_session = backend.matrix_session();
        let session_info = matrix_session
            .as_ref()
            .map(|session| session.info.clone())
            .or_else(|| session_info_from_state(backend.state()));
        (matrix_session, session_info)
    };

    let remote_logout_error = match matrix_session.as_ref() {
        Some(matrix_session) => matrix_desktop_auth::logout(matrix_session)
            .await
            .err()
            .map(|error| error.to_string()),
        None => None,
    };

    {
        let mut backend = state.backend.lock().map_err(lock_error)?;
        backend.dispatch(AppAction::LogoutRequested);
    }
    drop(matrix_session);

    if let Some(info) = &session_info {
        crate::clear_persisted_matrix_session(info)?;
    }

    let mut backend = state.backend.lock().map_err(lock_error)?;
    backend.dispatch(AppAction::LogoutFinished);
    if let Some(message) = remote_logout_error {
        backend.record_session_persistence_failure(message);
    }
    Ok(FrontendDesktopSnapshot::from(backend.snapshot()))
}

#[tauri::command]
pub async fn select_space(
    space_id: Option<String>,
    app: AppHandle,
    state: State<'_, BackendState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let (effects, matrix_session) = {
        let mut backend = state.backend.lock().map_err(lock_error)?;
        let mut effects = backend.dispatch(AppAction::SelectSpace {
            space_id: space_id.clone(),
        });

        if let Some(room_id) = first_room_id_for_space(&backend.snapshot(), space_id.as_deref()) {
            effects.extend(backend.dispatch(AppAction::SelectRoom { room_id }));
        }
        (effects, backend.matrix_session())
    };

    if let Some(room_id) = effects_subscribe_timeline_room_id(&effects)
        && let Some(matrix_session) = matrix_session
    {
        start_matrix_timeline_task(app, matrix_session, room_id)?;
    }

    let backend = state.backend.lock().map_err(lock_error)?;
    Ok(FrontendDesktopSnapshot::from(backend.snapshot()))
}

#[tauri::command]
pub async fn select_room(
    room_id: String,
    app: AppHandle,
    state: State<'_, BackendState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let (effects, matrix_session) = {
        let mut backend = state.backend.lock().map_err(lock_error)?;
        let effects = backend.dispatch(AppAction::SelectRoom { room_id });
        (effects, backend.matrix_session())
    };

    if let Some(room_id) = effects_subscribe_timeline_room_id(&effects)
        && let Some(matrix_session) = matrix_session
    {
        start_matrix_timeline_task(app, matrix_session, room_id)?;
    }

    let backend = state.backend.lock().map_err(lock_error)?;
    Ok(FrontendDesktopSnapshot::from(backend.snapshot()))
}

#[tauri::command]
pub fn paginate_timeline_backwards(
    room_id: String,
    state: State<'_, BackendState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let effects = {
        let mut backend = state.backend.lock().map_err(lock_error)?;
        backend.dispatch(AppAction::TimelineBackPaginationRequested { room_id })
    };

    if let Some(room_id) = effects_paginate_timeline_room_id(&effects) {
        let request = TimelinePaginationRequest {
            room_id: room_id.clone(),
            event_count: MATRIX_TIMELINE_BACK_PAGINATION_EVENT_COUNT,
        };
        let sent = state
            .timeline_task
            .lock()
            .map_err(lock_error)?
            .as_ref()
            .is_some_and(|handle| {
                timeline_task_can_paginate_room(&handle.room_id, &request.room_id)
                    && handle.pagination_sender.try_send(request).is_ok()
            });
        if !sent {
            let mut backend = state.backend.lock().map_err(lock_error)?;
            backend.dispatch(AppAction::TimelineBackPaginationFinished { room_id });
        }
    }

    let backend = state.backend.lock().map_err(lock_error)?;
    Ok(FrontendDesktopSnapshot::from(backend.snapshot()))
}

#[tauri::command]
pub async fn send_text(
    room_id: String,
    body: String,
    app: AppHandle,
    state: State<'_, BackendState>,
) -> Result<FrontendDesktopSnapshot, String> {
    if body.trim().is_empty() {
        let backend = state.backend.lock().map_err(lock_error)?;
        return Ok(FrontendDesktopSnapshot::from(backend.snapshot()));
    }

    let transaction_id = format!(
        "desktop-{}",
        NEXT_TRANSACTION_ID.fetch_add(1, Ordering::Relaxed)
    );
    let (effects, matrix_session, snapshot) = {
        let mut backend = state.backend.lock().map_err(lock_error)?;
        let matrix_session = backend.matrix_session();
        let effects = backend.dispatch(AppAction::SendTextSubmitted {
            room_id,
            transaction_id,
            body,
        });
        let snapshot = backend.snapshot();
        (effects, matrix_session, snapshot)
    };
    emit_ui_events(&app, &effects);
    update_qa_window_title(&app, &snapshot);

    if let Some((room_id, transaction_id, body)) = effects_send_text_request(&effects)
        && let Some(matrix_session) = matrix_session
    {
        spawn_matrix_send_text_task(app, matrix_session, room_id, transaction_id, body);
    }

    Ok(FrontendDesktopSnapshot::from(snapshot))
}

fn spawn_matrix_send_text_task(
    app: AppHandle,
    matrix_session: matrix_desktop_auth::MatrixClientSession,
    room_id: String,
    transaction_id: String,
    body: String,
) {
    tauri::async_runtime::spawn(async move {
        let action =
            match matrix_desktop_auth::send_text_message(&matrix_session, &room_id, &body, &transaction_id)
                .await
            {
                Ok(()) => AppAction::SendTextFinished {
                    room_id,
                    transaction_id,
                },
                Err(_) => AppAction::SendTextFailed {
                    room_id,
                    transaction_id,
                    message: MATRIX_SEND_TEXT_FAILED_MESSAGE.to_owned(),
                },
            };
        dispatch_timeline_action(&app, action);
    });
}

#[tauri::command]
pub async fn edit_message(
    room_id: String,
    event_id: String,
    body: String,
    state: State<'_, BackendState>,
) -> Result<FrontendDesktopSnapshot, String> {
    if body.trim().is_empty() {
        let backend = state.backend.lock().map_err(lock_error)?;
        return Ok(FrontendDesktopSnapshot::from(backend.snapshot()));
    }
    let matrix_session = {
        let backend = state.backend.lock().map_err(lock_error)?;
        backend.matrix_session()
    };

    if let Some(matrix_session) = matrix_session.as_ref() {
        matrix_desktop_auth::edit_text_message(matrix_session, &room_id, &event_id, &body)
            .await
            .map_err(|error| error.to_string())?;
    }

    let mut backend = state.backend.lock().map_err(lock_error)?;
    backend.edit_message(&room_id, &event_id, &body);
    Ok(FrontendDesktopSnapshot::from(backend.snapshot()))
}

#[tauri::command]
pub async fn redact_message(
    room_id: String,
    event_id: String,
    state: State<'_, BackendState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let matrix_session = {
        let backend = state.backend.lock().map_err(lock_error)?;
        backend.matrix_session()
    };

    if let Some(matrix_session) = matrix_session.as_ref() {
        matrix_desktop_auth::redact_message(matrix_session, &room_id, &event_id)
            .await
            .map_err(|error| error.to_string())?;
    }

    let mut backend = state.backend.lock().map_err(lock_error)?;
    backend.redact_message(&room_id, &event_id);
    Ok(FrontendDesktopSnapshot::from(backend.snapshot()))
}

#[tauri::command]
pub fn open_thread(
    room_id: String,
    root_event_id: String,
    state: State<'_, BackendState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut backend = state.backend.lock().map_err(lock_error)?;
    backend.dispatch(AppAction::OpenThread {
        room_id,
        root_event_id,
    });
    Ok(FrontendDesktopSnapshot::from(backend.snapshot()))
}

#[tauri::command]
pub fn close_thread(state: State<'_, BackendState>) -> Result<FrontendDesktopSnapshot, String> {
    let mut backend = state.backend.lock().map_err(lock_error)?;
    backend.dispatch(AppAction::CloseThread);
    Ok(FrontendDesktopSnapshot::from(backend.snapshot()))
}

#[tauri::command]
pub async fn submit_search(
    query: String,
    scope: SearchScopeKind,
    state: State<'_, BackendState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let (matrix_session, resolved_scope) = {
        let backend = state.backend.lock().map_err(lock_error)?;
        (backend.matrix_session(), scope.resolve(backend.state()))
    };

    if let Some(matrix_session) = matrix_session.as_ref() {
        let candidates = matrix_desktop_auth::search_message_candidates(matrix_session, &query, 50)
            .await
            .map_err(|error| error.to_string())?;
        let candidates = sdk_search_candidates_to_backend(candidates);
        let mut backend = state.backend.lock().map_err(lock_error)?;
        backend.submit_search_candidates(query, resolved_scope, candidates);
        return Ok(FrontendDesktopSnapshot::from(backend.snapshot()));
    }

    let mut backend = state.backend.lock().map_err(lock_error)?;
    backend.submit_search(query, resolved_scope);
    Ok(FrontendDesktopSnapshot::from(backend.snapshot()))
}

fn sdk_search_candidates_to_backend(
    candidates: Vec<matrix_desktop_auth::MatrixSearchCandidate>,
) -> Vec<matrix_desktop_backend::SearchCandidate> {
    candidates
        .into_iter()
        .map(|candidate| matrix_desktop_backend::SearchCandidate {
            room_id: candidate.room_id,
            event_id: candidate.event_id,
            score_millis: candidate.score_millis,
        })
        .collect()
}

fn matrix_timeline_items_to_backend_messages(
    items: Vec<matrix_desktop_auth::MatrixTimelineItem>,
) -> Vec<matrix_desktop_backend::TimelineMessage> {
    items
        .into_iter()
        .map(|item| matrix_desktop_backend::TimelineMessage {
            room_id: item.room_id,
            event_id: item.event_id,
            sender: item.sender,
            timestamp_ms: item.timestamp_ms,
            body: item.body,
            attachment_filename: None,
            reply_count: 0,
        })
        .collect()
}

fn matrix_timeline_updates_to_backend_updates(
    updates: Vec<matrix_desktop_auth::MatrixTimelineUpdate>,
) -> Vec<matrix_desktop_backend::TimelineUpdate> {
    updates
        .into_iter()
        .map(|update| match update {
            matrix_desktop_auth::MatrixTimelineUpdate::Upsert(item) => {
                matrix_desktop_backend::TimelineUpdate::Upsert(
                    matrix_desktop_backend::TimelineMessage {
                        room_id: item.room_id,
                        event_id: item.event_id,
                        sender: item.sender,
                        timestamp_ms: item.timestamp_ms,
                        body: item.body,
                        attachment_filename: None,
                        reply_count: 0,
                    },
                )
            }
            matrix_desktop_auth::MatrixTimelineUpdate::Remove { room_id, event_id } => {
                matrix_desktop_backend::TimelineUpdate::Remove { room_id, event_id }
            }
        })
        .collect()
}

fn timeline_messages_target_active_room(
    state: &AppState,
    messages: &[matrix_desktop_backend::TimelineMessage],
) -> bool {
    let Some(active_room_id) = state.timeline.room_id.as_deref() else {
        return false;
    };
    !messages.is_empty()
        && messages
            .iter()
            .all(|message| message.room_id == active_room_id)
}

fn timeline_updates_target_active_room(
    state: &AppState,
    updates: &[matrix_desktop_backend::TimelineUpdate],
) -> bool {
    let Some(active_room_id) = state.timeline.room_id.as_deref() else {
        return false;
    };
    !updates.is_empty()
        && updates
            .iter()
            .all(|update| timeline_update_room_id(update) == active_room_id)
}

fn timeline_update_room_id(update: &matrix_desktop_backend::TimelineUpdate) -> &str {
    match update {
        matrix_desktop_backend::TimelineUpdate::Upsert(message) => &message.room_id,
        matrix_desktop_backend::TimelineUpdate::Remove { room_id, .. } => room_id,
    }
}

fn first_room_id_for_space(
    snapshot: &matrix_desktop_backend::DesktopSnapshot,
    space_id: Option<&str>,
) -> Option<String> {
    let space_id = space_id?;
    let space = snapshot
        .state
        .spaces
        .iter()
        .find(|candidate| candidate.space_id == space_id)?;
    space
        .child_room_ids
        .iter()
        .find(|room_id| {
            snapshot
                .state
                .rooms
                .iter()
                .any(|room| room.room_id == **room_id && !room.is_dm)
        })
        .cloned()
}

fn deferred_login_request(
    effects: Vec<AppEffect>,
    state: &matrix_desktop_state::AppState,
) -> Option<LoginRequest> {
    if !matches!(state.session, SessionState::Authenticating { .. }) {
        return None;
    }

    effects.into_iter().find_map(|effect| match effect {
        AppEffect::Login(request) => Some(request),
        _ => None,
    })
}

fn deferred_recovery_request(
    effects: Vec<AppEffect>,
    state: &matrix_desktop_state::AppState,
) -> Option<RecoveryRequest> {
    if !matches!(state.session, SessionState::Recovering { .. }) {
        return None;
    }

    effects.into_iter().find_map(|effect| match effect {
        AppEffect::RecoverE2ee(request) => Some(request),
        _ => None,
    })
}

pub(crate) fn effects_include_start_sync(effects: &[AppEffect]) -> bool {
    effects
        .iter()
        .any(|effect| matches!(effect, AppEffect::StartSync))
}

pub(crate) fn effects_subscribe_timeline_room_id(effects: &[AppEffect]) -> Option<String> {
    effects.iter().find_map(|effect| match effect {
        AppEffect::SubscribeTimeline { room_id } => Some(room_id.clone()),
        _ => None,
    })
}

pub(crate) fn room_list_sync_follow_up(effects: &[AppEffect]) -> Option<String> {
    effects_subscribe_timeline_room_id(effects)
}

pub(crate) fn ui_event_payloads(effects: &[AppEffect]) -> Vec<&'static str> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            AppEffect::EmitUiEvent(event) => Some(match event {
                matrix_desktop_state::UiEvent::SessionChanged => "sessionChanged",
                matrix_desktop_state::UiEvent::AuthChanged => "authChanged",
                matrix_desktop_state::UiEvent::RoomListChanged => "roomListChanged",
                matrix_desktop_state::UiEvent::TimelineChanged { .. } => "timelineChanged",
                matrix_desktop_state::UiEvent::ThreadChanged => "threadChanged",
                matrix_desktop_state::UiEvent::SearchChanged => "searchChanged",
                matrix_desktop_state::UiEvent::ErrorChanged => "errorChanged",
            }),
            _ => None,
        })
        .collect()
}

fn emit_ui_events(app: &AppHandle, effects: &[AppEffect]) {
    for payload in ui_event_payloads(effects) {
        emit_state_event(app, payload);
    }
}

fn emit_state_event(app: &AppHandle, payload: &'static str) {
    let _ = app.emit(STATE_EVENT_NAME, payload);
}

fn update_qa_window_title(app: &AppHandle, snapshot: &matrix_desktop_backend::DesktopSnapshot) {
    if !qa_window_title_enabled() {
        return;
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_title(&qa_window_title(snapshot));
    }
}

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

pub(crate) fn qa_window_title(snapshot: &matrix_desktop_backend::DesktopSnapshot) -> String {
    [
        "matrix-desktop qa".to_owned(),
        format!("session={}", qa_session_label(&snapshot.state.session)),
        format!("sync={}", qa_sync_label(&snapshot.state.sync)),
        format!("rooms={}", snapshot.state.rooms.len()),
        format!("spaces={}", snapshot.state.spaces.len()),
        format!(
            "active_room={}",
            snapshot.state.navigation.active_room_id.is_some()
        ),
        format!(
            "timeline_subscribed={}",
            snapshot.state.timeline.is_subscribed
        ),
        format!("timeline_items={}", snapshot.timeline.len()),
        format!("errors={}", snapshot.state.errors.len()),
    ]
    .join(" ")
}

fn qa_session_label(session: &SessionState) -> &'static str {
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

pub(crate) fn effects_paginate_timeline_room_id(effects: &[AppEffect]) -> Option<String> {
    effects.iter().find_map(|effect| match effect {
        AppEffect::PaginateTimelineBackwards { room_id } => Some(room_id.clone()),
        _ => None,
    })
}

pub(crate) fn timeline_task_can_paginate_room(
    task_room_id: &str,
    request_room_id: &str,
) -> bool {
    task_room_id == request_room_id
}

pub(crate) fn effects_send_text_request(effects: &[AppEffect]) -> Option<(String, String, String)> {
    effects.iter().find_map(|effect| match effect {
        AppEffect::SendText {
            room_id,
            transaction_id,
            body,
        } => Some((room_id.clone(), transaction_id.clone(), body.clone())),
        _ => None,
    })
}

pub(crate) fn effects_restore_session_info(effects: &[AppEffect]) -> Option<SessionInfo> {
    effects.iter().find_map(|effect| match effect {
        AppEffect::RestoreSessionFor(info) => Some(info.clone()),
        _ => None,
    })
}

fn session_info_from_state(state: &AppState) -> Option<SessionInfo> {
    match &state.session {
        SessionState::NeedsRecovery { info, .. }
        | SessionState::Recovering { info, .. }
        | SessionState::Ready(info)
        | SessionState::Locked(info)
        | SessionState::SwitchingAccount { info } => Some(info.clone()),
        SessionState::SignedOut
        | SessionState::Restoring
        | SessionState::Authenticating { .. }
        | SessionState::LoggingOut => None,
    }
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> String {
    "backend state lock is poisoned".to_owned()
}

#[cfg(test)]
mod tests {
    use matrix_desktop_state::{AppState, RecoveryMethod, SessionInfo, SessionState};

    use super::{
        deferred_login_request, deferred_recovery_request, effects_include_start_sync,
        effects_paginate_timeline_room_id, effects_restore_session_info, effects_send_text_request,
        effects_subscribe_timeline_room_id, matrix_room_list_snapshot_to_backend_update,
        matrix_timeline_items_to_backend_messages, matrix_timeline_updates_to_backend_updates,
        promote_room_to_front, qa_recovery_prompt_is_available, qa_window_title,
        room_list_sync_follow_up, sdk_search_candidates_to_backend, session_info_from_state,
        timeline_messages_target_active_room, timeline_updates_target_active_room,
        timeline_task_can_paginate_room, ui_event_payloads,
    };
    use matrix_desktop_state::{
        AppEffect, AuthSecret, LoginRequest, RecoveryRequest, RoomSummary, SyncState,
        TimelinePaneState,
    };

    #[test]
    fn deferred_login_request_only_returns_request_while_authenticating() {
        let request = LoginRequest {
            homeserver: "https://matrix.example.org".to_owned(),
            username: "fixture-user".to_owned(),
            password: AuthSecret::new("synthetic-password"),
            device_display_name: Some("Matrix Desktop Test".to_owned()),
        };
        let mut state = AppState {
            session: SessionState::Authenticating {
                homeserver: "https://matrix.example.org".to_owned(),
            },
            ..AppState::default()
        };

        let deferred = deferred_login_request(vec![AppEffect::Login(request.clone())], &state);
        assert_eq!(deferred, Some(request.clone()));

        state.session = SessionState::SignedOut;
        let skipped = deferred_login_request(vec![AppEffect::Login(request)], &state);
        assert_eq!(skipped, None);
    }

    #[test]
    fn deferred_recovery_request_only_returns_request_while_recovering() {
        let request = RecoveryRequest {
            secret: AuthSecret::new("synthetic-recovery-secret"),
        };
        let mut state = AppState {
            session: SessionState::Recovering {
                info: SessionInfo {
                    homeserver: "https://matrix.example.org".to_owned(),
                    user_id: "@user-a:example.invalid".to_owned(),
                    device_id: "DEVICE123".to_owned(),
                },
                methods: vec![RecoveryMethod::RecoveryKey],
            },
            ..AppState::default()
        };

        let deferred =
            deferred_recovery_request(vec![AppEffect::RecoverE2ee(request.clone())], &state);
        assert_eq!(deferred, Some(request.clone()));

        state.session = SessionState::SignedOut;
        let skipped = deferred_recovery_request(vec![AppEffect::RecoverE2ee(request)], &state);
        assert_eq!(skipped, None);
    }

    #[test]
    fn effects_include_start_sync_only_detects_sync_start_effect() {
        assert!(effects_include_start_sync(&[
            AppEffect::EmitUiEvent(matrix_desktop_state::UiEvent::SessionChanged),
            AppEffect::StartSync,
        ]));
        assert!(!effects_include_start_sync(&[
            AppEffect::EmitUiEvent(matrix_desktop_state::UiEvent::SessionChanged),
            AppEffect::StopSync,
        ]));
    }

    #[test]
    fn effects_subscribe_timeline_room_id_returns_first_timeline_subscription() {
        let effects = vec![
            AppEffect::EmitUiEvent(matrix_desktop_state::UiEvent::TimelineChanged {
                room_id: "!ignored:example.invalid".to_owned(),
            }),
            AppEffect::SubscribeTimeline {
                room_id: "!room-alpha:example.invalid".to_owned(),
            },
        ];

        assert_eq!(
            effects_subscribe_timeline_room_id(&effects).as_deref(),
            Some("!room-alpha:example.invalid")
        );
    }

    #[test]
    fn room_list_sync_follow_up_starts_timeline_for_subscription_effect() {
        let effects = vec![
            AppEffect::EmitUiEvent(matrix_desktop_state::UiEvent::RoomListChanged),
            AppEffect::SubscribeTimeline {
                room_id: "!room-after-sync:example.invalid".to_owned(),
            },
        ];

        assert_eq!(
            room_list_sync_follow_up(&effects).as_deref(),
            Some("!room-after-sync:example.invalid")
        );
    }

    #[test]
    fn ui_event_payloads_do_not_include_room_ids() {
        let effects = vec![
            AppEffect::EmitUiEvent(matrix_desktop_state::UiEvent::RoomListChanged),
            AppEffect::EmitUiEvent(matrix_desktop_state::UiEvent::TimelineChanged {
                room_id: "!private-room:example.invalid".to_owned(),
            }),
        ];

        let payloads = ui_event_payloads(&effects);

        assert_eq!(payloads, vec!["roomListChanged", "timelineChanged"]);
        assert!(!format!("{payloads:?}").contains("!private-room"));
    }

    #[test]
    fn qa_window_title_summarizes_backend_state_without_private_identifiers() {
        let snapshot = matrix_desktop_backend::DesktopSnapshot {
            state: AppState {
                session: SessionState::Ready(SessionInfo {
                    homeserver: "https://matrix.example.org".to_owned(),
                    user_id: "@private-user:example.invalid".to_owned(),
                    device_id: "DEVICE123".to_owned(),
                }),
                sync: SyncState::Running,
                navigation: matrix_desktop_state::NavigationState {
                    active_space_id: None,
                    active_room_id: Some("!private-room:example.invalid".to_owned()),
                },
                rooms: vec![RoomSummary {
                    room_id: "!private-room:example.invalid".to_owned(),
                    display_name: "Private Room Name".to_owned(),
                    is_dm: false,
                    unread_count: 0,
                    parent_space_ids: Vec::new(),
                }],
                timeline: TimelinePaneState {
                    room_id: Some("!private-room:example.invalid".to_owned()),
                    is_subscribed: true,
                    is_paginating_backwards: false,
                    composer: matrix_desktop_state::ComposerState::default(),
                },
                ..AppState::default()
            },
            sidebar: matrix_desktop_state::SidebarModel {
                active_space_id: None,
                account_home: matrix_desktop_state::AccountHomeItem {
                    display_name: "Home".to_owned(),
                    unread_count: 0,
                    is_active: true,
                },
                space_rail: Vec::new(),
                space_rooms: Vec::new(),
                global_dms: Vec::new(),
                space_unread_count: 0,
                dm_unread_count: 0,
            },
            timeline: vec![matrix_desktop_backend::TimelineMessage {
                room_id: "!private-room:example.invalid".to_owned(),
                event_id: "$private-event".to_owned(),
                sender: "@private-user:example.invalid".to_owned(),
                timestamp_ms: 1_830_000_000_000,
                body: "private message body".to_owned(),
                attachment_filename: None,
                reply_count: 0,
            }],
            thread: None,
        };

        let title = qa_window_title(&snapshot);

        assert!(title.contains("matrix-desktop qa"));
        assert!(title.contains("session=ready"));
        assert!(title.contains("sync=running"));
        assert!(title.contains("rooms=1"));
        assert!(title.contains("active_room=true"));
        assert!(title.contains("timeline_subscribed=true"));
        assert!(title.contains("timeline_items=1"));
        assert!(!title.contains("Private"));
        assert!(!title.contains("@private"));
        assert!(!title.contains("!private"));
        assert!(!title.contains("$private"));
    }

    #[test]
    fn promote_room_to_front_keeps_room_list_complete_for_qa_timeline_sampling() {
        let mut snapshot = matrix_desktop_auth::MatrixRoomListSnapshot {
            spaces: Vec::new(),
            rooms: vec![
                matrix_desktop_auth::MatrixRoomListRoom {
                    room_id: "!empty:example.invalid".to_owned(),
                    display_name: "Empty Room".to_owned(),
                    is_dm: false,
                    unread_count: 0,
                    parent_space_ids: Vec::new(),
                },
                matrix_desktop_auth::MatrixRoomListRoom {
                    room_id: "!visible:example.invalid".to_owned(),
                    display_name: "Visible Room".to_owned(),
                    is_dm: false,
                    unread_count: 3,
                    parent_space_ids: Vec::new(),
                },
            ],
        };

        promote_room_to_front(&mut snapshot, "!visible:example.invalid");

        assert_eq!(snapshot.rooms[0].room_id, "!visible:example.invalid");
        assert_eq!(snapshot.rooms[1].room_id, "!empty:example.invalid");
        assert_eq!(snapshot.rooms.len(), 2);
    }

    #[test]
    fn qa_recovery_prompt_waits_for_needs_recovery_session() {
        let info = SessionInfo {
            homeserver: "https://matrix.example.org".to_owned(),
            user_id: "@user-a:example.invalid".to_owned(),
            device_id: "DEVICE123".to_owned(),
        };
        let ready = AppState {
            session: SessionState::Ready(info.clone()),
            ..AppState::default()
        };
        let needs_recovery = AppState {
            session: SessionState::NeedsRecovery {
                info,
                methods: vec![RecoveryMethod::RecoveryKey],
            },
            ..AppState::default()
        };

        assert!(!qa_recovery_prompt_is_available(&ready));
        assert!(qa_recovery_prompt_is_available(&needs_recovery));
    }

    #[test]
    fn effects_paginate_timeline_room_id_returns_first_pagination_request() {
        let effects = vec![
            AppEffect::EmitUiEvent(matrix_desktop_state::UiEvent::TimelineChanged {
                room_id: "!ignored:example.invalid".to_owned(),
            }),
            AppEffect::PaginateTimelineBackwards {
                room_id: "!room-alpha:example.invalid".to_owned(),
            },
        ];

        assert_eq!(
            effects_paginate_timeline_room_id(&effects).as_deref(),
            Some("!room-alpha:example.invalid")
        );
    }

    #[test]
    fn timeline_task_room_must_match_pagination_request() {
        assert!(timeline_task_can_paginate_room(
            "!room-alpha:example.invalid",
            "!room-alpha:example.invalid"
        ));
        assert!(!timeline_task_can_paginate_room(
            "!room-alpha:example.invalid",
            "!room-beta:example.invalid"
        ));
    }

    #[test]
    fn effects_send_text_request_returns_first_send_request() {
        let effects = vec![
            AppEffect::EmitUiEvent(matrix_desktop_state::UiEvent::TimelineChanged {
                room_id: "!ignored:example.invalid".to_owned(),
            }),
            AppEffect::SendText {
                room_id: "!room-alpha:example.invalid".to_owned(),
                transaction_id: "txn1".to_owned(),
                body: "hello".to_owned(),
            },
        ];

        assert_eq!(
            effects_send_text_request(&effects),
            Some((
                "!room-alpha:example.invalid".to_owned(),
                "txn1".to_owned(),
                "hello".to_owned(),
            ))
        );
    }

    #[test]
    fn effects_restore_session_info_returns_first_account_switch_target() {
        let target = SessionInfo {
            homeserver: "https://matrix.example.org".to_owned(),
            user_id: "@user-b:example.invalid".to_owned(),
            device_id: "DEVICE-B".to_owned(),
        };
        let effects = vec![
            AppEffect::StopSync,
            AppEffect::RestoreSessionFor(target.clone()),
            AppEffect::EmitUiEvent(matrix_desktop_state::UiEvent::SessionChanged),
        ];

        assert_eq!(effects_restore_session_info(&effects), Some(target));
    }

    #[test]
    fn sdk_search_candidates_convert_to_backend_candidates_without_content() {
        let candidates =
            sdk_search_candidates_to_backend(vec![matrix_desktop_auth::MatrixSearchCandidate {
                room_id: "!room-alpha:example.invalid".to_owned(),
                event_id: "$alpha-update".to_owned(),
                score_millis: 900,
            }]);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].room_id, "!room-alpha:example.invalid");
        assert_eq!(candidates[0].event_id, "$alpha-update");
        assert_eq!(candidates[0].score_millis, 900);
        assert!(!format!("{candidates:?}").contains("synthetic body"));
    }

    #[test]
    fn matrix_timeline_items_convert_to_backend_messages() {
        let messages = matrix_timeline_items_to_backend_messages(vec![
            matrix_desktop_auth::MatrixTimelineItem {
                room_id: "!room-alpha:example.invalid".to_owned(),
                event_id: "$sdk-visible".to_owned(),
                sender: "@sdk-user:example.invalid".to_owned(),
                timestamp_ms: 1_830_000_000_000,
                body: "SDK visible timeline body".to_owned(),
            },
        ]);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].room_id, "!room-alpha:example.invalid");
        assert_eq!(messages[0].event_id, "$sdk-visible");
        assert_eq!(messages[0].body, "SDK visible timeline body");
        assert_eq!(messages[0].attachment_filename, None);
        assert_eq!(messages[0].reply_count, 0);
    }

    #[test]
    fn matrix_room_list_snapshot_converts_to_backend_update() {
        let update = matrix_room_list_snapshot_to_backend_update(
            matrix_desktop_auth::MatrixRoomListSnapshot {
                spaces: vec![matrix_desktop_auth::MatrixRoomListSpace {
                    space_id: "!space-alpha:example.invalid".to_owned(),
                    display_name: "Example Space".to_owned(),
                }],
                rooms: vec![matrix_desktop_auth::MatrixRoomListRoom {
                    room_id: "!room-alpha:example.invalid".to_owned(),
                    display_name: "Example Room".to_owned(),
                    is_dm: false,
                    unread_count: 7,
                    parent_space_ids: vec!["!space-alpha:example.invalid".to_owned()],
                }],
            },
        );

        assert_eq!(update.spaces.len(), 1);
        assert_eq!(update.spaces[0].space_id, "!space-alpha:example.invalid");
        assert_eq!(update.spaces[0].display_name, "Example Space");
        assert_eq!(update.rooms.len(), 1);
        assert_eq!(update.rooms[0].room_id, "!room-alpha:example.invalid");
        assert_eq!(update.rooms[0].display_name, "Example Room");
        assert!(!update.rooms[0].is_dm);
        assert_eq!(update.rooms[0].unread_count, 7);
        assert_eq!(
            update.rooms[0].parent_space_ids,
            vec!["!space-alpha:example.invalid".to_owned()]
        );
    }

    #[test]
    fn matrix_timeline_updates_convert_to_backend_updates() {
        let updates = matrix_timeline_updates_to_backend_updates(vec![
            matrix_desktop_auth::MatrixTimelineUpdate::Upsert(
                matrix_desktop_auth::MatrixTimelineItem {
                    room_id: "!room-alpha:example.invalid".to_owned(),
                    event_id: "$sdk-visible".to_owned(),
                    sender: "@sdk-user:example.invalid".to_owned(),
                    timestamp_ms: 1_830_000_000_000,
                    body: "SDK visible timeline body".to_owned(),
                },
            ),
            matrix_desktop_auth::MatrixTimelineUpdate::Remove {
                room_id: "!room-alpha:example.invalid".to_owned(),
                event_id: "$sdk-visible".to_owned(),
            },
        ]);

        assert_eq!(updates.len(), 2);
        match &updates[0] {
            matrix_desktop_backend::TimelineUpdate::Upsert(message) => {
                assert_eq!(message.event_id, "$sdk-visible");
                assert_eq!(message.body, "SDK visible timeline body");
            }
            matrix_desktop_backend::TimelineUpdate::Remove { .. } => {
                panic!("first update should upsert")
            }
        }
        assert_eq!(
            updates[1],
            matrix_desktop_backend::TimelineUpdate::Remove {
                room_id: "!room-alpha:example.invalid".to_owned(),
                event_id: "$sdk-visible".to_owned(),
            }
        );
    }

    #[test]
    fn timeline_messages_must_target_active_timeline_room() {
        let state = AppState {
            timeline: TimelinePaneState {
                room_id: Some("!room-alpha:example.invalid".to_owned()),
                is_subscribed: true,
                is_paginating_backwards: false,
                composer: Default::default(),
            },
            ..AppState::default()
        };

        assert!(timeline_messages_target_active_room(
            &state,
            &[matrix_desktop_backend::TimelineMessage {
                room_id: "!room-alpha:example.invalid".to_owned(),
                event_id: "$alpha".to_owned(),
                sender: "@sender:example.invalid".to_owned(),
                timestamp_ms: 1_830_000_000_000,
                body: "alpha body".to_owned(),
                attachment_filename: None,
                reply_count: 0,
            }]
        ));
        assert!(!timeline_messages_target_active_room(
            &state,
            &[matrix_desktop_backend::TimelineMessage {
                room_id: "!room-beta:example.invalid".to_owned(),
                event_id: "$beta".to_owned(),
                sender: "@sender:example.invalid".to_owned(),
                timestamp_ms: 1_830_000_000_001,
                body: "beta body".to_owned(),
                attachment_filename: None,
                reply_count: 0,
            }]
        ));
        assert!(!timeline_messages_target_active_room(&state, &[]));
        assert!(!timeline_messages_target_active_room(
            &AppState::default(),
            &[matrix_desktop_backend::TimelineMessage {
                room_id: "!room-alpha:example.invalid".to_owned(),
                event_id: "$alpha".to_owned(),
                sender: "@sender:example.invalid".to_owned(),
                timestamp_ms: 1_830_000_000_000,
                body: "alpha body".to_owned(),
                attachment_filename: None,
                reply_count: 0,
            }]
        ));
    }

    #[test]
    fn timeline_updates_must_target_active_timeline_room() {
        let state = AppState {
            timeline: TimelinePaneState {
                room_id: Some("!room-alpha:example.invalid".to_owned()),
                is_subscribed: true,
                is_paginating_backwards: false,
                composer: Default::default(),
            },
            ..AppState::default()
        };

        assert!(timeline_updates_target_active_room(
            &state,
            &[
                matrix_desktop_backend::TimelineUpdate::Upsert(
                    matrix_desktop_backend::TimelineMessage {
                        room_id: "!room-alpha:example.invalid".to_owned(),
                        event_id: "$alpha".to_owned(),
                        sender: "@sender:example.invalid".to_owned(),
                        timestamp_ms: 1_830_000_000_000,
                        body: "alpha body".to_owned(),
                        attachment_filename: None,
                        reply_count: 0,
                    },
                ),
                matrix_desktop_backend::TimelineUpdate::Remove {
                    room_id: "!room-alpha:example.invalid".to_owned(),
                    event_id: "$alpha".to_owned(),
                },
            ]
        ));
        assert!(!timeline_updates_target_active_room(
            &state,
            &[matrix_desktop_backend::TimelineUpdate::Remove {
                room_id: "!room-beta:example.invalid".to_owned(),
                event_id: "$beta".to_owned(),
            }]
        ));
        assert!(!timeline_updates_target_active_room(&state, &[]));
    }

    #[test]
    fn session_info_from_state_reads_all_authenticated_session_variants() {
        let info = SessionInfo {
            homeserver: "https://matrix.example.org".to_owned(),
            user_id: "@user-a:example.invalid".to_owned(),
            device_id: "DEVICE123".to_owned(),
        };

        for session in [
            SessionState::NeedsRecovery {
                info: info.clone(),
                methods: vec![RecoveryMethod::RecoveryKey],
            },
            SessionState::Recovering {
                info: info.clone(),
                methods: vec![RecoveryMethod::RecoveryKey],
            },
            SessionState::Ready(info.clone()),
            SessionState::Locked(info.clone()),
        ] {
            let state = AppState {
                session,
                ..AppState::default()
            };
            assert_eq!(session_info_from_state(&state), Some(info.clone()));
        }

        assert_eq!(session_info_from_state(&AppState::default()), None);
    }
}
