use tauri::{AppHandle, State};

use crate::app_updates::{DesktopUpdateManager, DesktopUpdateState};
use crate::CoreRuntimeState;

#[tauri::command]
pub fn get_desktop_update_state(state: State<'_, DesktopUpdateManager>) -> DesktopUpdateState {
    state.state()
}

#[tauri::command]
pub async fn check_for_desktop_update(
    app: AppHandle,
    core: State<'_, CoreRuntimeState>,
) -> Result<(), ()> {
    let include_prereleases = core
        .connection
        .lock()
        .await
        .snapshot()
        .settings
        .values
        .updates
        .include_prereleases;
    crate::app_updates::check_for_update(&app, include_prereleases, true).await;
    Ok(())
}

#[tauri::command]
pub async fn download_desktop_update(app: AppHandle) -> Result<(), ()> {
    crate::app_updates::download_and_prepare(&app).await
}

#[tauri::command]
pub fn restart_to_install_desktop_update(app: AppHandle) {
    let _ = crate::app_updates::install_and_restart(&app);
}
