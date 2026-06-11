mod commands;
mod dto;

use std::sync::Mutex;

use matrix_desktop_backend::FakeDesktopBackend;

pub struct BackendState {
    backend: Mutex<FakeDesktopBackend>,
}

impl Default for BackendState {
    fn default() -> Self {
        Self {
            backend: Mutex::new(FakeDesktopBackend::booted()),
        }
    }
}

pub fn run() {
    tauri::Builder::default()
        .manage(BackendState::default())
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::submit_login,
            commands::select_space,
            commands::select_room,
            commands::open_thread,
            commands::close_thread,
            commands::submit_search,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run matrix desktop app");
}
