mod commands;
mod dto;

use std::sync::Mutex;

use matrix_desktop_backend::{FakeDesktopBackend, FakeDesktopBackendConfig, LoginDiscoveryMode};

pub struct BackendState {
    backend: Mutex<FakeDesktopBackend>,
}

impl Default for BackendState {
    fn default() -> Self {
        let config = FakeDesktopBackendConfig {
            restore_session: restore_session_enabled_from_env_value(
                std::env::var("MATRIX_DESKTOP_RESTORE_SESSION")
                    .ok()
                    .as_deref(),
            ),
            login_discovery: LoginDiscoveryMode::Http,
            ..FakeDesktopBackendConfig::default()
        };

        Self {
            backend: Mutex::new(FakeDesktopBackend::booted_with_config(config)),
        }
    }
}

fn restore_session_enabled_from_env_value(value: Option<&str>) -> bool {
    !matches!(
        value.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("0" | "false" | "signed-out")
    )
}

pub fn run() {
    tauri::Builder::default()
        .manage(BackendState::default())
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::discover_login_methods,
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

#[cfg(test)]
mod tests {
    use super::restore_session_enabled_from_env_value;

    #[test]
    fn restore_session_env_value_can_start_tauri_signed_out() {
        assert!(!restore_session_enabled_from_env_value(Some("0")));
        assert!(!restore_session_enabled_from_env_value(Some("false")));
        assert!(!restore_session_enabled_from_env_value(Some("signed-out")));
        assert!(restore_session_enabled_from_env_value(None));
        assert!(restore_session_enabled_from_env_value(Some("1")));
    }
}
