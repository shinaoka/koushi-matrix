use std::sync::Mutex;

use koushi_core::CoreConnection;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
#[cfg(target_os = "macos")]
use url::Url;

#[cfg(target_os = "macos")]
use tauri_plugin_updater::{Update, UpdaterExt};

pub const DESKTOP_UPDATE_EVENT_NAME: &str = "koushi-desktop://update";
pub const STABLE_UPDATE_ENDPOINT: &str =
    "https://github.com/shinaoka/koushi-matrix/releases/latest/download/latest.json";
pub const BETA_UPDATE_ENDPOINT: &str =
    "https://github.com/shinaoka/koushi-matrix/releases/download/latest-beta/latest-beta.json";
const UPDATE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DesktopUpdateState {
    Unsupported,
    Idle,
    UpToDate { version: String },
    Checking,
    Available { version: String },
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
    bytes: Option<Vec<u8>>,
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
            DesktopUpdateState::Idle
                | DesktopUpdateState::UpToDate { .. }
                | DesktopUpdateState::Failed { .. }
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
        let settings = connection.snapshot().settings.values.updates;
        let mut auto_check = settings.auto_check;
        let mut include_prereleases = settings.include_prereleases;
        if auto_check {
            check_for_update(&app, include_prereleases, false).await;
        }

        let mut interval = tokio::time::interval_at(
            tokio::time::Instant::now() + UPDATE_INTERVAL,
            UPDATE_INTERVAL,
        );
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if auto_check {
                        check_for_update(&app, include_prereleases, false).await;
                    }
                }
                snapshot = connection.next_versioned_snapshot() => {
                    let Some(snapshot) = snapshot else { break };
                    let next = snapshot.state.settings.values.updates.auto_check;
                    let next_include_prereleases =
                        snapshot.state.settings.values.updates.include_prereleases;
                    if next && (!auto_check || next_include_prereleases != include_prereleases) {
                        check_for_update(&app, next_include_prereleases, false).await;
                    }
                    auto_check = next;
                    include_prereleases = next_include_prereleases;
                }
            }
        }
    });

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, &mut connection);
    }
}

pub async fn check_for_update(app: &AppHandle, include_prereleases: bool, manual: bool) {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, include_prereleases, manual);
        return;
    }

    #[cfg(target_os = "macos")]
    {
        let manager = app.state::<DesktopUpdateManager>();
        if !manager.check_is_admissible() {
            return;
        }
        manager.publish(app, DesktopUpdateState::Checking);

        let mut endpoints = vec![STABLE_UPDATE_ENDPOINT];
        if include_prereleases {
            endpoints.push(BETA_UPDATE_ENDPOINT);
        }
        let mut update = None;
        for endpoint in endpoints {
            match check_update_endpoint(app, endpoint).await {
                Ok(Some(candidate)) => {
                    update = Some(select_newer_update(update, candidate));
                }
                Ok(None) => {}
                Err(()) => {
                    manager.publish(
                        app,
                        DesktopUpdateState::Failed {
                            stage: DesktopUpdateFailureStage::Check,
                        },
                    );
                    return;
                }
            }
        }

        let Some(update) = update else {
            let state = if manual {
                DesktopUpdateState::UpToDate {
                    version: app.package_info().version.to_string(),
                }
            } else {
                DesktopUpdateState::Idle
            };
            manager.publish(app, state);
            return;
        };
        let version = update.version.clone();
        *manager
            .pending
            .lock()
            .expect("desktop pending update mutex") = Some(PendingUpdate {
            update,
            bytes: None,
        });
        manager.publish(app, DesktopUpdateState::Available { version });
    }
}

#[cfg(target_os = "macos")]
async fn check_update_endpoint(app: &AppHandle, endpoint: &str) -> Result<Option<Update>, ()> {
    let endpoint = Url::parse(endpoint).map_err(|_| ())?;
    let updater = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|_| ())?
        .build()
        .map_err(|_| ())?;
    updater.check().await.map_err(|_| ())
}

#[cfg(target_os = "macos")]
fn select_newer_update(current: Option<Update>, candidate: Update) -> Update {
    match current {
        None => candidate,
        Some(current) => {
            if candidate_version_is_newer(&current.version, &candidate.version) {
                candidate
            } else {
                current
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn candidate_version_is_newer(current: &str, candidate: &str) -> bool {
    compare_semver(candidate, current) == std::cmp::Ordering::Greater
}

#[cfg(target_os = "macos")]
fn compare_semver(left: &str, right: &str) -> std::cmp::Ordering {
    let left = parse_semver(left);
    let right = parse_semver(right);
    for (left_part, right_part) in left.core.iter().zip(right.core.iter()) {
        match left_part.cmp(right_part) {
            std::cmp::Ordering::Equal => {}
            ordering => return ordering,
        }
    }
    match (left.prerelease.as_slice(), right.prerelease.as_slice()) {
        ([], []) => std::cmp::Ordering::Equal,
        ([], _) => std::cmp::Ordering::Greater,
        (_, []) => std::cmp::Ordering::Less,
        (left, right) => {
            for (left_part, right_part) in left.iter().zip(right.iter()) {
                match compare_prerelease_identifier(left_part, right_part) {
                    std::cmp::Ordering::Equal => {}
                    ordering => return ordering,
                }
            }
            left.len().cmp(&right.len())
        }
    }
}

#[cfg(target_os = "macos")]
struct ParsedSemVer<'a> {
    core: [u64; 3],
    prerelease: Vec<&'a str>,
}

#[cfg(target_os = "macos")]
fn parse_semver(version: &str) -> ParsedSemVer<'_> {
    let version = version
        .split_once('+')
        .map_or(version, |(version, _)| version);
    let (core, prerelease) = match version.split_once('-') {
        Some((core, prerelease)) => (core, prerelease.split('.').collect::<Vec<_>>()),
        None => (version, Vec::new()),
    };
    let mut core_parts = core.split('.');
    let core = [
        core_parts
            .next()
            .and_then(|part| part.parse().ok())
            .expect("tauri updater returns valid SemVer"),
        core_parts
            .next()
            .and_then(|part| part.parse().ok())
            .expect("tauri updater returns valid SemVer"),
        core_parts
            .next()
            .and_then(|part| part.parse().ok())
            .expect("tauri updater returns valid SemVer"),
    ];
    ParsedSemVer { core, prerelease }
}

#[cfg(target_os = "macos")]
fn compare_prerelease_identifier(left: &str, right: &str) -> std::cmp::Ordering {
    let left_numeric = left.parse::<u64>();
    let right_numeric = right.parse::<u64>();
    match (left_numeric, right_numeric) {
        (Ok(left), Ok(right)) => left.cmp(&right),
        (Ok(_), Err(_)) => std::cmp::Ordering::Less,
        (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
        (Err(_), Err(_)) => left.cmp(right),
    }
}

#[cfg(target_os = "macos")]
pub async fn download_and_prepare(app: &AppHandle) -> Result<(), ()> {
    let manager = app.state::<DesktopUpdateManager>();
    if !matches!(manager.state(), DesktopUpdateState::Available { .. }) {
        return Err(());
    }
    let pending = manager
        .pending
        .lock()
        .expect("desktop pending update mutex")
        .take()
        .ok_or(())?;
    let version = pending.update.version.clone();
    manager.publish(
        app,
        DesktopUpdateState::Downloading {
            version: version.clone(),
        },
    );
    match pending.update.download(|_, _| {}, || {}).await {
        Ok(bytes) => {
            *manager
                .pending
                .lock()
                .expect("desktop pending update mutex") = Some(PendingUpdate {
                update: pending.update,
                bytes: Some(bytes),
            });
            manager.publish(app, DesktopUpdateState::Ready { version });
            Ok(())
        }
        Err(_) => {
            manager.publish(
                app,
                DesktopUpdateState::Failed {
                    stage: DesktopUpdateFailureStage::DownloadOrVerify,
                },
            );
            Err(())
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub async fn download_and_prepare(_app: &AppHandle) -> Result<(), ()> {
    Err(())
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
    let bytes = pending.bytes.ok_or(())?;
    manager.publish(
        app,
        DesktopUpdateState::Installing {
            version: pending.update.version.clone(),
        },
    );
    if pending.update.install(&bytes).is_err() {
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
            DesktopUpdateState::Available {
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
        *manager.state.lock().expect("desktop update state mutex") = DesktopUpdateState::UpToDate {
            version: "1.2.3".to_owned(),
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
            serde_json::to_value(DesktopUpdateState::Available {
                version: "1.2.3".to_owned(),
            })
            .expect("serialize available update"),
            serde_json::json!({ "kind": "available", "version": "1.2.3" })
        );
        assert_eq!(
            serde_json::to_value(DesktopUpdateState::UpToDate {
                version: "1.2.3".to_owned(),
            })
            .expect("serialize up-to-date state"),
            serde_json::json!({ "kind": "up_to_date", "version": "1.2.3" })
        );
        assert_eq!(
            serde_json::to_value(DesktopUpdateState::Failed {
                stage: DesktopUpdateFailureStage::DownloadOrVerify,
            })
            .expect("serialize update failure"),
            serde_json::json!({ "kind": "failed", "stage": "download_or_verify" })
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn semver_candidate_selection_handles_prerelease_ordering() {
        assert!(candidate_version_is_newer("1.2.0-beta.2", "1.2.0-beta.10"));
        assert!(candidate_version_is_newer("1.2.0-beta.10", "1.2.0"));
        assert!(!candidate_version_is_newer("1.2.0", "1.2.0+build.1"));
        assert!(!candidate_version_is_newer("1.3.0", "1.2.0-rc.1"));
    }
}
