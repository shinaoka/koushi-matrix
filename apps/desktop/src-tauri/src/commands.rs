use matrix_desktop_state::{AppAction, AuthSecret, LoginRequest};
use tauri::State;

use crate::{
    BackendState,
    dto::{FrontendDesktopSnapshot, SearchScopeKind},
};

#[tauri::command]
pub fn get_snapshot(state: State<'_, BackendState>) -> Result<FrontendDesktopSnapshot, String> {
    let backend = state.backend.lock().map_err(lock_error)?;
    Ok(FrontendDesktopSnapshot::from(backend.snapshot()))
}

#[tauri::command]
pub fn submit_login(
    homeserver: String,
    username: String,
    password: String,
    device_display_name: Option<String>,
    state: State<'_, BackendState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut backend = state.backend.lock().map_err(lock_error)?;
    backend.dispatch(AppAction::LoginSubmitted(LoginRequest {
        homeserver,
        username,
        password: AuthSecret::new(password),
        device_display_name,
    }));
    Ok(FrontendDesktopSnapshot::from(backend.snapshot()))
}

#[tauri::command]
pub fn select_space(
    space_id: Option<String>,
    state: State<'_, BackendState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut backend = state.backend.lock().map_err(lock_error)?;
    backend.dispatch(AppAction::SelectSpace {
        space_id: space_id.clone(),
    });

    if let Some(room_id) = first_room_id_for_space(&backend.snapshot(), space_id.as_deref()) {
        backend.dispatch(AppAction::SelectRoom { room_id });
    }

    Ok(FrontendDesktopSnapshot::from(backend.snapshot()))
}

#[tauri::command]
pub fn select_room(
    room_id: String,
    state: State<'_, BackendState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut backend = state.backend.lock().map_err(lock_error)?;
    backend.dispatch(AppAction::SelectRoom { room_id });
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
pub fn submit_search(
    query: String,
    scope: SearchScopeKind,
    state: State<'_, BackendState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut backend = state.backend.lock().map_err(lock_error)?;
    let resolved_scope = scope.resolve(backend.state());
    backend.submit_search(query, resolved_scope);
    Ok(FrontendDesktopSnapshot::from(backend.snapshot()))
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

fn lock_error<T>(_: std::sync::PoisonError<T>) -> String {
    "backend state lock is poisoned".to_owned()
}
