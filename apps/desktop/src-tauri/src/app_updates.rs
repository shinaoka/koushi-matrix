use std::sync::Mutex;

use koushi_core::CoreConnection;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

#[cfg(target_os = "macos")]
use tauri_plugin_updater::{Update, UpdaterExt};

pub const DESKTOP_UPDATE_EVENT_NAME: &str = "koushi-desktop://update";
const UPDATE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DesktopUpdateState {
    Unsupported,
    Idle,
    Checking,
    Downloading { version: String },
    Ready { version: String },
    Failed { stage: DesktopUpdateFailureStage },
    Installing { version: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopUpdateFailureStage {
    Check,
    DownloadOrVerify,
    Install,
}

#[cfg(target_os = "macos")]
struct PendingUpdate {
    update: Update,
    bytes: Vec<u8>,
}

pub struct DesktopUpdateManager {
    state: Mutex<DesktopUpdateState>,
    #[cfg(target_os = "macos")]
    pending: Mutex<Option<PendingUpdate>>,
}

impl DesktopUpdateManager {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(initial_state()),
            #[cfg(target_os = "macos")]
            pending: Mutex::new(None),
        }
    }

    pub fn state(&self) -> DesktopUpdateState {
        self.state
            .lock()
            .expect("desktop update state mutex")
            .clone()
    }

    fn publish(&self, app: &AppHandle, state: DesktopUpdateState) {
        *self.state.lock().expect("desktop update state mutex") = state.clone();
        let _ = app.emit(DESKTOP_UPDATE_EVENT_NAME, state);
    }

    fn check_is_admissible(&self) -> bool {
        matches!(
            self.state(),
            DesktopUpdateState::Idle | DesktopUpdateState::Failed { .. }
        )
    }
}

fn initial_state() -> DesktopUpdateState {
    if cfg!(target_os = "macos") && configured_updater_public_key().is_some() {
        DesktopUpdateState::Idle
    } else {
        DesktopUpdateState::Unsupported
    }
}

pub(crate) fn configured_updater_public_key() -> Option<&'static str> {
    option_env!("KOUSHI_UPDATER_PUBLIC_KEY").filter(|key| !key.trim().is_empty())
}

pub fn spawn_auto_update_loop(app: AppHandle, mut connection: CoreConnection) {
    #[cfg(target_os = "macos")]
    tauri::async_runtime::spawn(async move {
        if configured_updater_public_key().is_none() {
            return;
        }
        let mut auto_check = connection.snapshot().settings.values.updates.auto_check;
        if auto_check {
            check_and_download(&app).await;
        }

        let mut interval = tokio::time::interval_at(
            tokio::time::Instant::now() + UPDATE_INTERVAL,
            UPDATE_INTERVAL,
        );
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if auto_check {
                        check_and_download(&app).await;
                    }
                }
                snapshot = connection.next_versioned_snapshot() => {
                    let Some(snapshot) = snapshot else { break };
                    let next = snapshot.state.settings.values.updates.auto_check;
                    if next && !auto_check {
                        check_and_download(&app).await;
                    }
                    auto_check = next;
                }
            }
        }
    });

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, &mut connection);
    }
}

#[cfg(target_os = "macos")]
async fn check_and_download(app: &AppHandle) {
    let manager = app.state::<DesktopUpdateManager>();
    if !manager.check_is_admissible() {
        return;
    }
    manager.publish(app, DesktopUpdateState::Checking);

    let updater = match app.updater() {
        Ok(updater) => updater,
        Err(_) => {
            manager.publish(
                app,
                DesktopUpdateState::Failed {
                    stage: DesktopUpdateFailureStage::Check,
                },
            );
            return;
        }
    };
    let update = match updater.check().await {
        Ok(update) => update,
        Err(_) => {
            manager.publish(
                app,
                DesktopUpdateState::Failed {
                    stage: DesktopUpdateFailureStage::Check,
                },
            );
            return;
        }
    };

    let Some(update) = update else {
        manager.publish(app, DesktopUpdateState::Idle);
        return;
    };
    let version = update.version.clone();
    manager.publish(
        app,
        DesktopUpdateState::Downloading {
            version: version.clone(),
        },
    );
    match update.download(|_, _| {}, || {}).await {
        Ok(bytes) => {
            *manager
                .pending
                .lock()
                .expect("desktop pending update mutex") = Some(PendingUpdate { update, bytes });
            manager.publish(app, DesktopUpdateState::Ready { version });
        }
        Err(_) => manager.publish(
            app,
            DesktopUpdateState::Failed {
                stage: DesktopUpdateFailureStage::DownloadOrVerify,
            },
        ),
    }
}

#[cfg(target_os = "macos")]
pub fn install_and_restart(app: &AppHandle) -> Result<(), ()> {
    let manager = app.state::<DesktopUpdateManager>();
    let pending = manager
        .pending
        .lock()
        .expect("desktop pending update mutex")
        .take()
        .ok_or(())?;
    manager.publish(
        app,
        DesktopUpdateState::Installing {
            version: pending.update.version.clone(),
        },
    );
    if pending.update.install(&pending.bytes).is_err() {
        manager.publish(
            app,
            DesktopUpdateState::Failed {
                stage: DesktopUpdateFailureStage::Install,
            },
        );
        return Err(());
    }
    app.restart();
}

#[cfg(not(target_os = "macos"))]
pub fn install_and_restart(_app: &AppHandle) -> Result<(), ()> {
    Err(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_work_and_ready_states_reject_duplicate_checks() {
        let manager = DesktopUpdateManager::new();
        for state in [
            DesktopUpdateState::Checking,
            DesktopUpdateState::Downloading {
                version: "1.2.3".to_owned(),
            },
            DesktopUpdateState::Ready {
                version: "1.2.3".to_owned(),
            },
            DesktopUpdateState::Installing {
                version: "1.2.3".to_owned(),
            },
        ] {
            *manager.state.lock().expect("desktop update state mutex") = state;
            assert!(!manager.check_is_admissible());
        }
    }

    #[test]
    fn idle_and_failure_states_allow_a_later_check() {
        let manager = DesktopUpdateManager::new();
        *manager.state.lock().expect("desktop update state mutex") = DesktopUpdateState::Idle;
        assert!(manager.check_is_admissible());
        *manager.state.lock().expect("desktop update state mutex") = DesktopUpdateState::Failed {
            stage: DesktopUpdateFailureStage::Check,
        };
        assert!(manager.check_is_admissible());
    }

    #[test]
    fn update_state_wire_shape_matches_the_frontend_contract() {
        assert_eq!(
            serde_json::to_value(DesktopUpdateState::Ready {
                version: "1.2.3".to_owned(),
            })
            .expect("serialize update state"),
            serde_json::json!({ "kind": "ready", "version": "1.2.3" })
        );
        assert_eq!(
            serde_json::to_value(DesktopUpdateState::Failed {
                stage: DesktopUpdateFailureStage::DownloadOrVerify,
            })
            .expect("serialize update failure"),
            serde_json::json!({ "kind": "failed", "stage": "download_or_verify" })
        );
    }
}
