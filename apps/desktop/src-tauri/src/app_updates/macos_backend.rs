//! macOS install backend for the shared updater engine.
//!
//! Only the package-specific integration lives here: feed access and signature
//! verification through the Tauri updater plugin, artifact installation, and
//! the native relaunch request. Lifecycle, policy, and shutdown coordination
//! stay in the platform-neutral engine (`app_updates.rs`).

use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::{Update, UpdaterExt};
use url::Url;

use super::channel_policy::{select_newer_candidate, update_endpoints};
use super::{
    Backend, Completion, DESKTOP_UPDATE_EVENT_NAME, DesktopUpdateState, PendingUpdate,
    UpdateFuture, Work,
};

pub(super) type Candidate = Update;

pub(super) struct MacosBackend {
    app: AppHandle,
    current_version: String,
}

impl MacosBackend {
    pub(super) fn for_app(app: &AppHandle) -> Option<Self> {
        Some(Self {
            app: app.clone(),
            current_version: app.package_info().version.to_string(),
        })
    }
}

impl Backend<Update> for MacosBackend {
    fn start(&self, work: Work<Update>) -> UpdateFuture<'static, Completion<Update>> {
        let app = self.app.clone();
        match work {
            Work::Check(include_prereleases) => Box::pin(async move {
                let mut update = None;
                for endpoint in update_endpoints(include_prereleases) {
                    match check_update_endpoint(&app, endpoint).await {
                        Ok(Some(candidate)) => {
                            update = Some(select_newer_candidate(update, candidate, |update| {
                                update.version.as_str()
                            }))
                        }
                        Ok(None) => {}
                        Err(()) => return Completion::Check(Err(())),
                    }
                }
                Completion::Check(Ok(update.map(|update| PendingUpdate {
                    version: update.version.clone(),
                    notification_only: false,
                    update,
                    bytes: None,
                })))
            }),
            Work::Download(mut pending) => Box::pin(async move {
                match pending.update.download(|_, _| {}, || {}).await {
                    Ok(bytes) => {
                        pending.bytes = Some(bytes);
                        Completion::Download(Ok(pending))
                    }
                    Err(_) => Completion::Download(Err(())),
                }
            }),
            Work::Install(pending) => {
                // Retain the blocking handle inside an owner-polled future. The
                // owner joins this future, even on shutdown, instead of aborting it.
                let install = tauri::async_runtime::spawn_blocking(move || {
                    let bytes = pending.bytes.ok_or(())?;
                    pending.update.install(&bytes).map_err(|_| ())
                });
                Box::pin(async move { Completion::Install(install.await.unwrap_or(Err(()))) })
            }
        }
    }
    fn emit(&self, state: DesktopUpdateState) {
        let _ = self.app.emit(DESKTOP_UPDATE_EVENT_NAME, state);
    }
    fn current_version(&self) -> &str {
        &self.current_version
    }
    fn restart(&self) {
        // restart() blocks its calling thread, which would deadlock the graceful
        // shutdown barrier waiting to join this owner. Request exit and return.
        crate::request_application_restart(&self.app);
    }
}

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
