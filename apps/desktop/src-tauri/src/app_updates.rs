use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

use koushi_core::CoreConnection;
use koushi_state::UpdatesSettings;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{Notify, watch};
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
    Available { version: String, generation: u64 },
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

struct PendingUpdate<C> {
    version: String,
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
                DesktopUpdateState::Unsupported
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

#[cfg(target_os = "macos")]
type Candidate = Update;
#[cfg(not(target_os = "macos"))]
type Candidate = ();

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

enum Work<C> {
    Check(bool),
    Download(PendingUpdate<C>),
    Install(PendingUpdate<C>),
}
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

impl SettingsSource for CoreConnection {
    fn next(&mut self) -> UpdateFuture<'_, Option<PolicySnapshot>> {
        Box::pin(async move {
            self.next_versioned_snapshot()
                .await
                .map(|snapshot| PolicySnapshot {
                    generation: snapshot.generation,
                    settings: snapshot.state.settings.values.updates,
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
    if cfg!(target_os = "macos") && configured_updater_public_key().is_some() {
        DesktopUpdateState::Idle
    } else {
        DesktopUpdateState::Unsupported
    }
}

pub(crate) fn configured_updater_public_key() -> Option<&'static str> {
    option_env!("KOUSHI_UPDATER_PUBLIC_KEY").filter(|key| !key.trim().is_empty())
}

pub fn spawn_auto_update_loop(app: AppHandle, connection: CoreConnection) {
    #[cfg(target_os = "macos")]
    {
        if configured_updater_public_key().is_none() {
            return;
        }
        let snapshot = connection.versioned_snapshot();
        let shared = app.state::<DesktopUpdateManager>().shared.clone();
        shared.transition(
            |state| {
                let _ = app.emit(DESKTOP_UPDATE_EVENT_NAME, state);
            },
            |lifecycle| {
                lifecycle.observe(PolicySnapshot {
                    generation: snapshot.generation,
                    settings: snapshot.state.settings.values.updates,
                });
            },
        );
        let current_version = app.package_info().version.to_string();
        start_owner(
            shared,
            NativeBackend {
                app,
                current_version,
            },
            connection,
        );
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, connection);
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

#[cfg(target_os = "macos")]
struct NativeBackend {
    app: AppHandle,
    current_version: String,
}

#[cfg(target_os = "macos")]
impl Backend<Update> for NativeBackend {
    fn start(&self, work: Work<Update>) -> UpdateFuture<'static, Completion<Update>> {
        let app = self.app.clone();
        match work {
            Work::Check(include_prereleases) => Box::pin(async move {
                let mut update = None;
                let endpoints = if include_prereleases {
                    vec![STABLE_UPDATE_ENDPOINT, BETA_UPDATE_ENDPOINT]
                } else {
                    vec![STABLE_UPDATE_ENDPOINT]
                };
                for endpoint in endpoints {
                    match check_update_endpoint(&app, endpoint).await {
                        Ok(Some(candidate)) => {
                            update = Some(select_newer_update(update, candidate))
                        }
                        Ok(None) => {}
                        Err(()) => return Completion::Check(Err(())),
                    }
                }
                Completion::Check(Ok(update.map(|update| PendingUpdate {
                    version: update.version.clone(),
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

pub async fn download_and_prepare(
    app: &AppHandle,
    snapshot: koushi_protocol::state_update::VersionedAppStateSnapshot,
    expected_generation: u64,
) -> Result<(), ()> {
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
mod regression_tests;
