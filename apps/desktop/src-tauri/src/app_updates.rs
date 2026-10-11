use std::{
    future::Future,
    path::Path,
    pin::Pin,
    sync::{Arc, Mutex},
};

use koushi_state::{AppSettingsValues, UpdatesSettings};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{Notify, watch};

// Compilation boundary (#1035, overview.md "Desktop Application Updates"):
// everything in this file is the platform-neutral facade and lifecycle engine
// and is compiled and tested on every desktop target. A platform contributes
// only a `Backend` implementation plus its candidate type. Targets without an
// install backend use the uninhabited `PlatformBackend` below, so the engine
// stays type-checked there while no update work can ever start.
#[cfg(any(koushi_updater_backend, test))]
mod channel_policy;
#[cfg(target_os = "macos")]
mod macos_backend;
#[cfg(target_os = "macos")]
use macos_backend::{Candidate, MacosBackend as PlatformBackend};
#[cfg(windows)]
mod windows_backend;
#[cfg(windows)]
use windows_backend::{Candidate, WindowsBackend as PlatformBackend};

pub const DESKTOP_UPDATE_EVENT_NAME: &str = "koushi-desktop://update";
const UPDATE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DesktopUpdateState {
    Unsupported {
        reason: DesktopUpdateUnsupportedReason,
    },
    Idle,
    UpToDate {
        version: String,
    },
    Checking,
    Available {
        version: String,
        generation: u64,
        notification_only: bool,
    },
    Downloading {
        version: String,
    },
    Ready {
        version: String,
    },
    Failed {
        stage: DesktopUpdateFailureStage,
    },
    Installing {
        version: String,
    },
}

/// Why this installation cannot update itself. `Build` covers targets without
/// an install backend and builds without updater trust material;
/// `PackageManaged` is the packager opt-out (#1063).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopUpdateUnsupportedReason {
    Build,
    PackageManaged,
}

/// Packager opt-out marker (#1063). A distribution package that owns the
/// installed files (for example a repackaged `.deb` under `/usr`) installs this
/// file so the app never selects an install backend or issues update requests.
/// Its contents are ignored; only its presence matters.
pub const LINUX_PACKAGE_MANAGED_MARKER: &str = "/usr/share/koushi-desktop/package-managed";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopUpdateFailureStage {
    Check,
    DownloadOrVerify,
    Install,
}

struct PendingUpdate<C> {
    version: String,
    notification_only: bool,
    // Read only by an install backend; unused where no backend is compiled.
    #[cfg_attr(not(koushi_updater_backend), allow(dead_code))]
    update: C,
    bytes: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Check,
    Download,
    Install,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Operation {
    generation: u64,
    phase: Phase,
}

#[derive(Clone)]
struct PolicySnapshot {
    generation: u64,
    settings: UpdatesSettings,
}

struct Lifecycle<C> {
    state: DesktopUpdateState,
    pending: Option<PendingUpdate<C>>,
    generation: u64,
    settings_generation: Option<u64>,
    settings: UpdatesSettings,
    claimed: bool,
    stopping: bool,
    owner: Option<tauri::async_runtime::JoinHandle<()>>,
    owner_started: bool,
}

impl<C> Lifecycle<C> {
    fn new(state: DesktopUpdateState) -> Self {
        Self {
            state,
            pending: None,
            generation: 0,
            settings_generation: None,
            settings: UpdatesSettings {
                auto_check: false,
                include_prereleases: false,
            },
            claimed: false,
            stopping: false,
            owner: None,
            owner_started: false,
        }
    }

    fn advance(&mut self) {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("updater generation exhausted");
        self.claimed = false;
    }

    fn operation(&self) -> Option<Operation> {
        let phase = match self.state {
            DesktopUpdateState::Checking => Phase::Check,
            DesktopUpdateState::Downloading { .. } => Phase::Download,
            DesktopUpdateState::Installing { .. } => Phase::Install,
            _ => return None,
        };
        (!self.stopping).then_some(Operation {
            generation: self.generation,
            phase,
        })
    }

    fn begin_check(&mut self) -> bool {
        if self.stopping
            || !matches!(
                self.state,
                DesktopUpdateState::Idle
                    | DesktopUpdateState::UpToDate { .. }
                    | DesktopUpdateState::Failed { .. }
            )
        {
            return false;
        }
        self.advance();
        self.pending = None;
        self.state = DesktopUpdateState::Checking;
        true
    }

    fn observe(&mut self, snapshot: PolicySnapshot) -> bool {
        if self.stopping {
            return false;
        }
        if let Some(watermark) = self.settings_generation {
            if snapshot.generation < watermark {
                return false;
            }
            if snapshot.generation == watermark {
                return snapshot.settings == self.settings;
            }
        }
        let channel_changed =
            snapshot.settings.include_prereleases != self.settings.include_prereleases;
        let enable_check = snapshot.settings.auto_check
            && (self.settings_generation.is_none() || !self.settings.auto_check);
        self.settings_generation = Some(snapshot.generation);
        self.settings = snapshot.settings;
        if channel_changed
            && !matches!(
                self.state,
                DesktopUpdateState::Unsupported { .. }
                    | DesktopUpdateState::Downloading { .. }
                    | DesktopUpdateState::Ready { .. }
                    | DesktopUpdateState::Installing { .. }
            )
        {
            self.advance();
            self.pending = None;
            self.state = DesktopUpdateState::Idle;
            if self.settings.auto_check {
                self.begin_check();
            }
        } else if enable_check {
            self.begin_check();
        }
        true
    }

    fn request_check(&mut self, snapshot: PolicySnapshot) -> bool {
        // An older command snapshot must not roll policy back, but it does not
        // invalidate the user's intent. Admit against the latest owned state.
        self.observe(snapshot);
        self.begin_check()
    }

    fn request_download(
        &mut self,
        snapshot: PolicySnapshot,
        expected_generation: u64,
    ) -> Result<(), ()> {
        self.observe(snapshot);
        self.begin_download(expected_generation)
    }

    fn begin_download(&mut self, expected_generation: u64) -> Result<(), ()> {
        if self.stopping {
            return Err(());
        }
        let DesktopUpdateState::Available {
            version,
            generation,
            ..
        } = &self.state
        else {
            return Err(());
        };
        if *generation != expected_generation || self.pending.is_none() {
            return Err(());
        }
        let version = version.clone();
        self.advance();
        self.state = DesktopUpdateState::Downloading { version };
        Ok(())
    }

    fn begin_install(&mut self) -> Result<(), ()> {
        if self.stopping {
            return Err(());
        }
        let DesktopUpdateState::Ready { version } = &self.state else {
            return Err(());
        };
        if self
            .pending
            .as_ref()
            .and_then(|pending| pending.bytes.as_ref())
            .is_none()
        {
            return Err(());
        }
        let version = version.clone();
        self.advance();
        self.state = DesktopUpdateState::Installing { version };
        Ok(())
    }

    fn claim_work(&mut self) -> Option<(Operation, Work<C>)> {
        let operation = self.operation()?;
        if self.claimed {
            return None;
        }
        let work = match operation.phase {
            Phase::Check => Work::Check(self.settings.include_prereleases),
            Phase::Download => Work::Download(self.pending.take()?),
            Phase::Install => Work::Install(self.pending.take()?),
        };
        self.claimed = true;
        Some((operation, work))
    }

    fn complete(
        &mut self,
        operation: Operation,
        completion: Completion<C>,
        current_version: &str,
    ) -> bool {
        if self.operation() != Some(operation) || !self.claimed {
            return false;
        }
        if !matches!(
            (&completion, operation.phase),
            (Completion::Check(_), Phase::Check)
                | (Completion::Download(_), Phase::Download)
                | (Completion::Install(_), Phase::Install)
        ) {
            return false;
        }
        self.claimed = false;
        match (operation.phase, completion) {
            (Phase::Check, Completion::Check(Ok(Some(pending)))) => {
                self.state = DesktopUpdateState::Available {
                    version: pending.version.clone(),
                    generation: operation.generation,
                    notification_only: pending.notification_only,
                };
                self.pending = Some(pending);
            }
            (Phase::Check, Completion::Check(Ok(None))) => {
                self.state = no_update_state(current_version.to_owned());
            }
            (Phase::Check, Completion::Check(Err(()))) => {
                self.state = DesktopUpdateState::Failed {
                    stage: DesktopUpdateFailureStage::Check,
                };
            }
            (Phase::Download, Completion::Download(Ok(pending))) => {
                self.state = DesktopUpdateState::Ready {
                    version: pending.version.clone(),
                };
                self.pending = Some(pending);
            }
            (Phase::Download, Completion::Download(Err(()))) => {
                self.state = DesktopUpdateState::Failed {
                    stage: DesktopUpdateFailureStage::DownloadOrVerify,
                };
            }
            (Phase::Install, Completion::Install(Ok(()))) => return true,
            (Phase::Install, Completion::Install(Err(()))) => {
                self.state = DesktopUpdateState::Failed {
                    stage: DesktopUpdateFailureStage::Install,
                };
            }
            _ => {}
        }
        false
    }

    fn stop(&mut self) {
        if !self.stopping {
            self.stopping = true;
            self.advance();
            self.pending = None;
        }
    }
}

struct Shared<C> {
    lifecycle: Mutex<Lifecycle<C>>,
    wake: Notify,
    joined: watch::Sender<bool>,
}

impl<C> Shared<C> {
    fn new(state: DesktopUpdateState) -> Self {
        Self {
            lifecycle: Mutex::new(Lifecycle::new(state)),
            wake: Notify::new(),
            joined: watch::channel(true).0,
        }
    }

    fn state(&self) -> DesktopUpdateState {
        self.lifecycle
            .lock()
            .expect("desktop update lifecycle mutex")
            .state
            .clone()
    }

    // Emission is synchronous and ordered with the mutation. The emitter must not
    // re-enter the lifecycle; no network, installation or await runs in this lock.
    fn transition<R>(
        &self,
        emit: impl FnOnce(DesktopUpdateState),
        action: impl FnOnce(&mut Lifecycle<C>) -> R,
    ) -> R {
        let mut lifecycle = self
            .lifecycle
            .lock()
            .expect("desktop update lifecycle mutex");
        let previous = lifecycle.state.clone();
        let generation = lifecycle.generation;
        let result = action(&mut lifecycle);
        if previous != lifecycle.state {
            emit(lifecycle.state.clone());
        }
        if previous != lifecycle.state || generation != lifecycle.generation {
            self.wake.notify_one();
        }
        result
    }

    async fn shutdown(&self) {
        let mut joined = self.joined.subscribe();
        self.transition(
            |_| {},
            |lifecycle| {
                lifecycle.stop();
            },
        );
        while !*joined.borrow_and_update() {
            tokio::select! {
                _ = joined.changed() => {}
                _ = std::future::poll_fn(|cx| {
                    // Poll only the join handle, never its work, while locked.
                    // Keep it retained if this shutdown waiter is cancelled.
                    let mut lifecycle = self.lifecycle.lock().expect("desktop update lifecycle mutex");
                    let ready = lifecycle.owner.as_mut().is_none_or(|owner| Pin::new(owner).poll(cx).is_ready());
                    if ready {
                        lifecycle.owner = None;
                        self.joined.send_replace(true);
                        std::task::Poll::Ready(())
                    } else {
                        std::task::Poll::Pending
                    }
                }) => {}
            }
        }
    }
}

/// Placeholder backend for installations without a supported install path.
/// It is uninhabited: `for_app` can only return `None`, so the owner is never
/// started, while the shared engine remains compiled against the same
/// `Backend` contract a future Linux backend implements.
#[cfg(not(koushi_updater_backend))]
enum PlatformBackend {}
#[cfg(not(koushi_updater_backend))]
type Candidate = ();

#[cfg(not(koushi_updater_backend))]
impl PlatformBackend {
    fn for_app(_app: &AppHandle) -> Option<Self> {
        None
    }
}

#[cfg(not(koushi_updater_backend))]
impl Backend<Candidate> for PlatformBackend {
    fn start(&self, _work: Work<Candidate>) -> UpdateFuture<'static, Completion<Candidate>> {
        match *self {}
    }
    fn emit(&self, _state: DesktopUpdateState) {
        match *self {}
    }
    fn current_version(&self) -> &str {
        match *self {}
    }
    fn restart(&self) {
        match *self {}
    }
}

pub struct DesktopUpdateManager {
    shared: Arc<Shared<Candidate>>,
}

impl DesktopUpdateManager {
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Shared::new(initial_state())),
        }
    }

    pub fn state(&self) -> DesktopUpdateState {
        self.shared.state()
    }
}

// Backend-facing seam: only an install backend reads `Work` payloads and
// constructs `Completion`s, so builds without a backend never touch them.
#[cfg_attr(not(koushi_updater_backend), allow(dead_code))]
enum Work<C> {
    Check(bool),
    Download(PendingUpdate<C>),
    Install(PendingUpdate<C>),
}
#[cfg_attr(not(koushi_updater_backend), allow(dead_code))]
enum Completion<C> {
    Check(Result<Option<PendingUpdate<C>>, ()>),
    Download(Result<PendingUpdate<C>, ()>),
    Install(Result<(), ()>),
}
type UpdateFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

trait Backend<C>: Send + Sync + 'static {
    fn start(&self, work: Work<C>) -> UpdateFuture<'static, Completion<C>>;
    fn emit(&self, state: DesktopUpdateState);
    fn current_version(&self) -> &str;
    fn restart(&self);
}

trait SettingsSource: Send + 'static {
    fn next(&mut self) -> UpdateFuture<'_, Option<PolicySnapshot>>;
}

struct AppSettingsSource {
    updates: watch::Receiver<AppSettingsValues>,
    generation: u64,
}

impl SettingsSource for AppSettingsSource {
    fn next(&mut self) -> UpdateFuture<'_, Option<PolicySnapshot>> {
        Box::pin(async move {
            self.updates.changed().await.ok()?;
            self.generation = self.generation.saturating_add(1);
            Some(PolicySnapshot {
                generation: self.generation,
                settings: self.updates.borrow_and_update().updates,
            })
        })
    }
}

fn start_owner<C: Send + 'static>(
    shared: Arc<Shared<C>>,
    backend: impl Backend<C>,
    source: impl SettingsSource,
) {
    let mut lifecycle = shared
        .lifecycle
        .lock()
        .expect("desktop update lifecycle mutex");
    if lifecycle.owner_started || lifecycle.stopping {
        return;
    }
    lifecycle.owner_started = true;
    shared.joined.send_replace(false);
    lifecycle.owner = Some(tauri::async_runtime::spawn(run_owner(
        shared.clone(),
        backend,
        source,
    )));
}

async fn run_owner<C: Send + 'static>(
    shared: Arc<Shared<C>>,
    backend: impl Backend<C>,
    mut source: impl SettingsSource,
) {
    let mut active: Option<(Operation, UpdateFuture<'static, Completion<C>>)> = None;
    let mut interval = tokio::time::interval_at(
        tokio::time::Instant::now() + UPDATE_INTERVAL,
        UPDATE_INTERVAL,
    );
    loop {
        let (stopping, operation) = {
            let lifecycle = shared
                .lifecycle
                .lock()
                .expect("desktop update lifecycle mutex");
            (lifecycle.stopping, lifecycle.operation())
        };
        if active
            .as_ref()
            .is_some_and(|(token, _)| Some(*token) != operation)
        {
            let (token, future) = active.take().expect("active operation");
            if token.phase == Phase::Install {
                let _ = future.await;
            }
            // Dropping a check/download future cancels its network work.
        }
        if stopping {
            break;
        }
        if active.is_none() {
            let claimed = shared
                .lifecycle
                .lock()
                .expect("desktop update lifecycle mutex")
                .claim_work();
            if let Some((operation, work)) = claimed {
                active = Some((operation, backend.start(work)));
            }
        }
        tokio::select! {
            biased;
            _ = shared.wake.notified() => {}
            snapshot = source.next() => {
                shared.transition(|state| backend.emit(state), |lifecycle| {
                    match snapshot {
                        Some(snapshot) => { lifecycle.observe(snapshot); }
                        None => lifecycle.stop(),
                    }
                });
            }
            _ = interval.tick() => {
                shared.transition(|state| backend.emit(state), |lifecycle| {
                    if lifecycle.settings.auto_check { lifecycle.begin_check(); }
                });
            }
            completion = async { active.as_mut().expect("guarded active operation").1.as_mut().await }, if active.is_some() => {
                let (operation, _) = active.take().expect("completed active operation");
                let restart = shared.transition(|state| backend.emit(state), |lifecycle| {
                    lifecycle.complete(operation, completion, backend.current_version())
                });
                if restart {
                    backend.restart();
                    break;
                }
            }
        }
    }
}

fn initial_state() -> DesktopUpdateState {
    initial_state_for(package_managed_marker())
}

/// The marker probed on this target. Only Linux distribution packages can opt
/// out today; other targets have no marker and never read the filesystem here.
fn package_managed_marker() -> Option<&'static Path> {
    cfg!(target_os = "linux").then(|| Path::new(LINUX_PACKAGE_MANAGED_MARKER))
}

fn initial_state_for(marker: Option<&Path>) -> DesktopUpdateState {
    // `Unsupported` is the capability of this installation, not a permanent
    // platform policy: it changes once the target has an install backend.
    // The packager opt-out is checked at runtime because a repackaged binary is
    // byte-identical to the upstream one (#1063); it wins over every backend.
    if marker.is_some_and(Path::exists) {
        DesktopUpdateState::Unsupported {
            reason: DesktopUpdateUnsupportedReason::PackageManaged,
        }
    } else if cfg!(target_os = "windows")
        || (cfg!(koushi_updater_backend) && configured_updater_public_key().is_some())
    {
        DesktopUpdateState::Idle
    } else {
        DesktopUpdateState::Unsupported {
            reason: DesktopUpdateUnsupportedReason::Build,
        }
    }
}

/// Selects the install backend only for an installation whose owned state can
/// update. An `Unsupported` lifecycle (no backend, no trust material, or the
/// packager opt-out) never constructs a backend, so no owner, feed request, or
/// installer process (`pkexec`, `sudo`, `dpkg`, `rpm`) can start.
fn select_backend<B>(state: &DesktopUpdateState, for_app: impl FnOnce() -> Option<B>) -> Option<B> {
    if matches!(state, DesktopUpdateState::Unsupported { .. }) {
        None
    } else {
        for_app()
    }
}

pub(crate) fn configured_updater_public_key() -> Option<&'static str> {
    option_env!("KOUSHI_UPDATER_PUBLIC_KEY").filter(|key| !key.trim().is_empty())
}

pub fn spawn_auto_update_loop(
    app: AppHandle,
    mut settings_updates: watch::Receiver<AppSettingsValues>,
) {
    if configured_updater_public_key().is_none() && !cfg!(target_os = "windows") {
        return;
    }
    let shared = app.state::<DesktopUpdateManager>().shared.clone();
    let Some(backend) = select_backend(&shared.state(), || PlatformBackend::for_app(&app)) else {
        return;
    };
    let initial_settings = settings_updates.borrow_and_update().clone();
    shared.transition(
        |state| {
            let _ = app.emit(DESKTOP_UPDATE_EVENT_NAME, state);
        },
        |lifecycle| {
            lifecycle.observe(PolicySnapshot {
                generation: 0,
                settings: initial_settings.updates,
            });
        },
    );
    start_owner(
        shared,
        backend,
        AppSettingsSource {
            updates: settings_updates,
            generation: 0,
        },
    );
}

pub async fn shutdown(app: &AppHandle) {
    let shared = app.state::<DesktopUpdateManager>().shared.clone();
    shared.shutdown().await;
}

pub async fn check_for_update(
    app: &AppHandle,
    snapshot: koushi_protocol::state_update::VersionedAppStateSnapshot,
) {
    app.state::<DesktopUpdateManager>().shared.transition(
        |state| {
            let _ = app.emit(DESKTOP_UPDATE_EVENT_NAME, state);
        },
        |lifecycle| {
            lifecycle.request_check(PolicySnapshot {
                generation: snapshot.generation,
                settings: snapshot.state.settings.values.updates,
            });
        },
    );
}

fn no_update_state(version: String) -> DesktopUpdateState {
    DesktopUpdateState::UpToDate { version }
}

pub async fn download_and_prepare(
    app: &AppHandle,
    snapshot: koushi_protocol::state_update::VersionedAppStateSnapshot,
    expected_generation: u64,
) -> Result<(), ()> {
    // Windows is notification-only until signed NSIS updater artifacts and
    // their manifest entry are published. Never turn a release-page notice
    // into an installer request through an old or malicious renderer.
    if cfg!(target_os = "windows") {
        return Err(());
    }
    app.state::<DesktopUpdateManager>().shared.transition(
        |state| {
            let _ = app.emit(DESKTOP_UPDATE_EVENT_NAME, state);
        },
        |lifecycle| {
            lifecycle.request_download(
                PolicySnapshot {
                    generation: snapshot.generation,
                    settings: snapshot.state.settings.values.updates,
                },
                expected_generation,
            )
        },
    )
}

pub fn install_and_restart(app: &AppHandle) -> Result<(), ()> {
    app.state::<DesktopUpdateManager>().shared.transition(
        |state| {
            let _ = app.emit(DESKTOP_UPDATE_EVENT_NAME, state);
        },
        |lifecycle| lifecycle.begin_install(),
    )
}

#[cfg(test)]
mod package_managed_tests;
#[cfg(test)]
mod regression_tests;
