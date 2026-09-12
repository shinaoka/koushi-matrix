use tauri::{AppHandle, State};

use crate::app_updates::{DesktopUpdateManager, DesktopUpdateState};

#[tauri::command]
pub fn get_desktop_update_state(state: State<'_, DesktopUpdateManager>) -> DesktopUpdateState {
    state.state()
}

#[tauri::command]
pub fn restart_to_install_desktop_update(app: AppHandle) {
    let _ = crate::app_updates::install_and_restart(&app);
}
