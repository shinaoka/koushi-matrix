use tauri::{AppHandle, State};

use crate::CoreRuntimeState;
use crate::app_updates::{DesktopUpdateManager, DesktopUpdateState};

#[tauri::command]
pub fn get_desktop_update_state(state: State<'_, DesktopUpdateManager>) -> DesktopUpdateState {
    state.state()
}

#[tauri::command]
pub async fn check_for_desktop_update(
    app: AppHandle,
    core: State<'_, CoreRuntimeState>,
) -> Result<(), ()> {
    let snapshot = core.connection.lock().await.versioned_snapshot();
    crate::app_updates::check_for_update(&app, snapshot).await;
    Ok(())
}

#[tauri::command]
pub async fn download_desktop_update(
    app: AppHandle,
    core: State<'_, CoreRuntimeState>,
    expected_generation: u64,
) -> Result<(), ()> {
    let snapshot = core.connection.lock().await.versioned_snapshot();
    crate::app_updates::download_and_prepare(&app, snapshot, expected_generation).await
}

#[tauri::command]
pub fn restart_to_install_desktop_update(app: AppHandle) {
    let _ = crate::app_updates::install_and_restart(&app);
}
