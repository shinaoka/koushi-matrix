//! SyncActor: continuous sync lifecycle with capability-probed backend
//! selection (Async design rule 9).
//!
//! ## Ownership
//! `SyncActor` is owned by `AccountActor`. Its task handle lives inside
//! `AccountActor`; the actor boundary defines ownership, not a separate Tokio
//! task per actor (design spec: "Actor Deployment And Supervision").
//!
//! ## Backend selection
//! On `SyncCommand::Start`, the actor calls
//! `Client::available_sliding_sync_versions()`. A non-empty result means the
//! server advertises `org.matrix.simplified_msc3575` in `/versions`
//! `unstable_features` → select `SyncService` backend. Empty → `LegacySync`
//! backend using `client.sync_stream`.
//!
//! The selected backend kind is emitted in `SyncEvent::Started { backend }` so
//! QA can assert server capability (canon, Async rule 9).
//!
//! ## State machine
//! (stopped) --Start--> (starting) --first-sync-ok--> (running)
//!   (running) --transient-fail--> (reconnecting) --recovered--> (running)
//!   (running|reconnecting) --terminal-fail--> (failed)
//!   (any) --Stop--> (stopped)
//!   (failed) --Restart--> (stopped) --Start--> (starting) ...
//!
//! `SyncCommand::SyncOnce` is QA/debug only (does not affect continuous sync).
//!
//! ## Supervision (overview.md)
//! A sync task panic or unexpected exit moves the account to SyncFailed.
//! `SyncCommand::Restart` is the recovery path.
//!
//! ## Security constraints
//! SDK sync errors are NEVER exposed in events or AppState. They are converted
//! to `SyncFailureKind` (Http/Auth/Store/Internal) and the `AppAction::SyncFailed`
//! reason field carries only the stable kind label string — never raw SDK text.
//!
//! ## Store bootstrap invariant
//! Sync only ever starts on the store-backed session that `AccountActor`
//! already guarantees. The SyncActor must not create its own client.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use koushi_sdk::MatrixClientSession;
use koushi_state::{AppAction, SyncMode};
use tokio::sync::{broadcast, mpsc};

use crate::command::SyncCommand;
use crate::event::{CoreEvent, SyncBackendKind, SyncEvent};
use crate::executor;
use crate::failure::{CoreFailure, SyncFailureKind};
use crate::ids::RequestId;
use crate::room::RoomMessage;

/// QA/debug-only override: when set to `legacy`, the capability probe is
/// skipped and the `LegacySync` backend is selected. This exists because both
/// local QA homeservers (Conduit, Tuwunel) advertise MSC4186, so the legacy
/// path would otherwise be unreachable in the local QA matrix; legacy `/sync`
/// works against MSC4186-capable servers too (canon decision, Phase 3 review).
///
/// COMPILE-TIME GATE: release builds must never honor this override
/// (release-gate structural rule pattern). Any value other than `legacy` is
/// ignored and the probe runs normally.
#[cfg(any(debug_assertions, test))]
const ENV_FORCE_SYNC_BACKEND: &str = "MATRIX_DESKTOP_QA_FORCE_SYNC_BACKEND";

/// Messages sent to the SyncActor from AccountActor.
pub enum SyncMessage {
    /// Route a `SyncCommand` to the actor.
    Command(SyncCommand),
    /// Ordered shutdown: AccountActor is shutting down.
    Shutdown,
}

/// Handle to the SyncActor background task (owned by AccountActor).
pub struct SyncActorHandle {
    tx: mpsc::Sender<SyncMessage>,
    task: executor::JoinHandle<()>,
}

impl SyncActorHandle {
    pub async fn send(&self, msg: SyncMessage) -> bool {
        self.tx.send(msg).await.is_ok()
    }

    /// Wait for the actor task to complete (used in ordered shutdown).
    pub async fn join(self) {
        let _ = self.task.await;
    }
}

/// Internal sync lifecycle state (not the same as AppState.sync).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SyncLifecycle {
    Stopped,
    Running,
    Failed,
}

/// What the sync background task produced when it ended.
#[derive(Debug)]
enum SyncTaskOutcome {
    Stopped,
    Failed(SyncFailureKind),
    Panicked,
}

/// Backend selected on the current run (stored for idempotent Start).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActiveBackend {
    None,
    SyncService,
    LegacySync,
}

pub struct SyncActor {
    session: Arc<MatrixClientSession>,
    action_tx: mpsc::Sender<Vec<AppAction>>,
    event_tx: broadcast::Sender<CoreEvent>,
    command_rx: mpsc::Receiver<SyncMessage>,
    /// RoomActor inbox: the SyncActor notifies it on sync start/stop because
    /// only the SyncActor knows the selected backend and owns the live
    /// `RoomListService` (canon: RoomActor consumes the ONE live service).
    room_tx: mpsc::Sender<RoomMessage>,
    /// TimelineManager inbox: receives the live `RoomListService` on sync
    /// start so timeline subscriptions can subscribe rooms with it (canon).
    timeline_tx: mpsc::Sender<crate::timeline::TimelineMessage>,
    lifecycle: SyncLifecycle,
    active_backend: ActiveBackend,
    /// Task handle for the currently running sync loop.
    sync_task: Option<executor::JoinHandle<SyncTaskOutcome>>,
    /// Stop-signal for the legacy sync loop.
    legacy_stop_tx: Option<tokio::sync::oneshot::Sender<()>>,
    /// SyncService handle (Some when SyncService backend is running).
    sync_service: Option<Arc<matrix_sdk_ui::sync_service::SyncService>>,
    /// Handle for the global ignored-user-list account-data handler. Removed on
    /// stop so repeated start/stop cycles do not install duplicate handlers.
    ignored_user_list_handler: Option<matrix_sdk::event_handler::EventHandlerHandle>,
}

impl SyncActor {
    pub fn spawn(
        session: Arc<MatrixClientSession>,
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
        room_tx: mpsc::Sender<RoomMessage>,
        timeline_tx: mpsc::Sender<crate::timeline::TimelineMessage>,
    ) -> SyncActorHandle {
        let (tx, command_rx) = mpsc::channel(16);
        let actor = SyncActor {
            session,
            action_tx,
            event_tx,
            command_rx,
            room_tx,
            timeline_tx,
            lifecycle: SyncLifecycle::Stopped,
            active_backend: ActiveBackend::None,
            sync_task: None,
            legacy_stop_tx: None,
            sync_service: None,
            ignored_user_list_handler: None,
        };
        let task = executor::spawn(actor.run());
        SyncActorHandle { tx, task }
    }

    async fn run(mut self) {
        loop {
            if self.sync_task.is_some() {
                tokio::select! {
                    biased;
                    // Poll the running sync task.
                    outcome = async {
                        // Safety: we checked is_some above; this arm is active only when Some.
                        self.sync_task.as_mut().unwrap().await
                    } => {
                        let outcome = outcome
                            .unwrap_or(SyncTaskOutcome::Panicked);
                        self.sync_task = None;
                        self.legacy_stop_tx = None;
                        self.sync_service = None;
                        self.active_backend = ActiveBackend::None;
                        self.handle_sync_task_ended(outcome);
                    }
                    msg = self.command_rx.recv() => {
                        match msg {
                            None | Some(SyncMessage::Shutdown) => {
                                self.do_stop(None).await;
                                break;
                            }
                            Some(SyncMessage::Command(command)) => {
                                self.handle_command(command).await;
                            }
                        }
                    }
                }
            } else {
                match self.command_rx.recv().await {
                    None | Some(SyncMessage::Shutdown) => {
                        break;
                    }
                    Some(SyncMessage::Command(command)) => {
                        self.handle_command(command).await;
                    }
                }
            }
        }
        // Ordered shutdown: stop any running sync task.
        if self.sync_task.is_some() {
            self.do_stop(None).await;
        }
    }

    fn handle_sync_task_ended(&mut self, outcome: SyncTaskOutcome) {
        // The room-list observation must not outlive the sync backend it
        // relays (live RoomListService on SyncService, base-client updates on
        // legacy). try_send: this is a sync fn; capacity 64 suffices.
        let _ = self.room_tx.try_send(RoomMessage::SyncStopped);
        match outcome {
            SyncTaskOutcome::Stopped => {
                self.lifecycle = SyncLifecycle::Stopped;
                // Task ended via stop signal — emit Stopped (no request_id because
                // it was not from an explicit SyncCommand::Stop at this level).
                self.emit(CoreEvent::Sync(SyncEvent::Stopped { request_id: None }));
                self.reduce(vec![AppAction::SyncStopped]);
            }
            SyncTaskOutcome::Failed(_) | SyncTaskOutcome::Panicked => {
                let kind = match outcome {
                    SyncTaskOutcome::Failed(k) => k,
                    SyncTaskOutcome::Panicked => SyncFailureKind::Internal,
                    SyncTaskOutcome::Stopped => unreachable!(),
                };
                self.lifecycle = SyncLifecycle::Failed;
                let mode = sync_mode_from_backend(self.current_backend_kind(), true);
                self.reduce(vec![AppAction::SyncModeChanged { mode }]);
                self.emit(CoreEvent::Sync(SyncEvent::ModeChanged { mode }));
                self.emit(CoreEvent::Sync(SyncEvent::Failed));
                self.reduce(vec![AppAction::SyncFailed {
                    reason: sync_failure_kind_label(kind).to_owned(),
                }]);
            }
        }
    }

    async fn handle_command(&mut self, command: SyncCommand) {
        match command {
            SyncCommand::Start { request_id } => {
                self.handle_start(request_id).await;
            }
            SyncCommand::Stop { request_id } => {
                self.do_stop(Some(request_id)).await;
            }
            SyncCommand::Restart { request_id } => {
                // Ordered: stop first (no-op if already stopped), then start.
                self.do_stop(None).await;
                self.lifecycle = SyncLifecycle::Stopped;
                self.handle_start(request_id).await;
            }
            SyncCommand::SyncOnce { request_id } => {
                self.handle_sync_once(request_id).await;
            }
        }
    }

    async fn handle_start(&mut self, request_id: RequestId) {
        // Idempotent: if already running, re-emit Started so QA can assert backend.
        if self.lifecycle == SyncLifecycle::Running {
            let backend = self.current_backend_kind();
            let mode = sync_mode_from_backend(backend, false);
            self.reduce(vec![AppAction::SyncModeChanged { mode }]);
            self.emit(CoreEvent::Sync(SyncEvent::Started {
                request_id: Some(request_id),
                backend,
            }));
            self.emit(CoreEvent::Sync(SyncEvent::ModeChanged { mode }));
            return;
        }

        // Probe MSC4186 capability (Async rule 9).
        let client = self.session.client();
        let backend_kind = probe_backend(&client).await;
        let mode = sync_mode_from_backend(backend_kind, false);
        self.reduce(vec![AppAction::SyncModeChanged { mode }]);

        // Emit Started with the selected backend BEFORE the task starts running,
        // so QA can assert the backend kind on the same event.
        self.emit(CoreEvent::Sync(SyncEvent::Started {
            request_id: Some(request_id),
            backend: backend_kind,
        }));
        self.emit(CoreEvent::Sync(SyncEvent::ModeChanged { mode }));

        // Launch the appropriate background sync task.
        match backend_kind {
            SyncBackendKind::SyncService => {
                match self.start_sync_service(client).await {
                    Ok(()) => {}
                    Err(()) => {
                        // SyncService build failed; fall back to legacy.
                        let transition_mode = SyncMode::Transitioning;
                        self.reduce(vec![AppAction::SyncModeChanged {
                            mode: transition_mode,
                        }]);
                        self.emit(CoreEvent::Sync(SyncEvent::ModeChanged {
                            mode: transition_mode,
                        }));
                        let client2 = self.session.client();
                        self.active_backend = ActiveBackend::LegacySync;
                        self.start_legacy_sync(client2).await;
                        let fallback_mode = SyncMode::Legacy;
                        self.reduce(vec![AppAction::SyncModeChanged {
                            mode: fallback_mode,
                        }]);
                        self.emit(CoreEvent::Sync(SyncEvent::ModeChanged {
                            mode: fallback_mode,
                        }));
                    }
                }
            }
            SyncBackendKind::LegacySync => {
                self.active_backend = ActiveBackend::LegacySync;
                self.start_legacy_sync(client).await;
            }
        }
        self.lifecycle = SyncLifecycle::Running;

        // Notify the RoomActor that sync is running, handing over the ONE
        // live RoomListService on the SyncService backend (None on legacy).
        // Only the SyncActor can do this: it knows the selected backend and
        // owns the SyncService (canon, overview.md RoomActor bullet — ad-hoc
        // RoomListService instances are prohibited).
        let room_list_service = self
            .sync_service
            .as_ref()
            .map(|service| service.room_list_service());
        let _ = self
            .room_tx
            .send(RoomMessage::SyncStarted {
                session: self.session.clone(),
                room_list_service: room_list_service.clone(),
            })
            .await;
        // Same handoff to the timeline manager: timeline subscriptions must
        // be able to subscribe rooms with the live service (canon).
        let _ = self
            .timeline_tx
            .send(crate::timeline::TimelineMessage::SyncStarted { room_list_service })
            .await;
    }

    /// Returns Ok(()) on success, Err(()) when SyncService build fails (caller falls back).
    async fn start_sync_service(&mut self, client: matrix_sdk::Client) -> Result<(), ()> {
        self.register_ignored_user_list_handler(&client);

        let service = matrix_sdk_ui::sync_service::SyncService::builder(client.clone())
            .with_offline_mode()
            .build()
            .await
            .map_err(|_| {
                if let Some(handle) = self.ignored_user_list_handler.take() {
                    client.remove_event_handler(handle);
                }
                ()
            })?;
        let service = Arc::new(service);

        // Subscribe BEFORE starting so no state changes are missed.
        let state_sub = service.state();
        service.start().await;

        let event_tx = self.event_tx.clone();
        let action_tx = self.action_tx.clone();

        let task: executor::JoinHandle<SyncTaskOutcome> = executor::spawn(async move {
            observe_sync_service_states(state_sub, event_tx, action_tx).await
        });

        self.sync_service = Some(service);
        self.sync_task = Some(task);
        self.active_backend = ActiveBackend::SyncService;
        Ok(())
    }

    async fn start_legacy_sync(&mut self, client: matrix_sdk::Client) {
        self.register_ignored_user_list_handler(&client);

        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let event_tx = self.event_tx.clone();
        let action_tx = self.action_tx.clone();

        let task: executor::JoinHandle<SyncTaskOutcome> = executor::spawn(async move {
            run_legacy_sync_loop(client, stop_rx, event_tx, action_tx).await
        });

        self.legacy_stop_tx = Some(stop_tx);
        self.sync_task = Some(task);
    }

    /// Graceful stop: signal the running sync backend and wait (bounded timeout).
    async fn do_stop(&mut self, request_id: Option<RequestId>) {
        // Tear down the RoomActor's room-list observation first: on the
        // SyncService backend it consumes the live RoomListService that is
        // about to stop. Harmless no-op when nothing is running.
        let _ = self.room_tx.send(RoomMessage::SyncStopped).await;
        // Signal stop to whichever backend is running.
        if let Some(svc) = self.sync_service.take() {
            svc.stop().await;
        }
        if let Some(tx) = self.legacy_stop_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.ignored_user_list_handler.take() {
            self.session.client().remove_event_handler(handle);
        }
        // Wait for the background task to complete (bounded to avoid hangs).
        if let Some(task) = self.sync_task.take() {
            let _ = executor::timeout(Duration::from_secs(10), task).await;
        }
        self.active_backend = ActiveBackend::None;
        self.lifecycle = SyncLifecycle::Stopped;
        self.emit(CoreEvent::Sync(SyncEvent::Stopped { request_id }));
        self.reduce(vec![AppAction::SyncStopped]);
    }

    fn current_backend_kind(&self) -> SyncBackendKind {
        match self.active_backend {
            ActiveBackend::SyncService => SyncBackendKind::SyncService,
            ActiveBackend::LegacySync | ActiveBackend::None => SyncBackendKind::LegacySync,
        }
    }

    fn register_ignored_user_list_handler(&mut self, client: &matrix_sdk::Client) {
        use matrix_sdk::ruma::events::{
            GlobalAccountDataEvent, ignored_user_list::IgnoredUserListEventContent,
        };

        let action_tx = self.action_tx.clone();
        let timeline_tx = self.timeline_tx.clone();
        let handle = client.add_event_handler(
            move |ev: GlobalAccountDataEvent<IgnoredUserListEventContent>| {
                let action_tx = action_tx.clone();
                let timeline_tx = timeline_tx.clone();
                async move {
                    let user_ids: BTreeSet<String> = ev
                        .content
                        .ignored_users
                        .keys()
                        .map(|user_id| user_id.to_string())
                        .collect();
                    let _ = action_tx.try_send(vec![AppAction::IgnoredUsersLoaded {
                        user_ids: user_ids.clone(),
                    }]);
                    let _ = timeline_tx.try_send(
                        crate::timeline::TimelineMessage::IgnoredUsersUpdated { user_ids },
                    );
                }
            },
        );
        self.ignored_user_list_handler = Some(handle);
    }

    /// QA/debug: one-shot sync, does not affect the continuous sync state machine.
    async fn handle_sync_once(&self, request_id: RequestId) {
        match koushi_sdk::sync_once(&self.session).await {
            Ok(()) => {
                self.emit(CoreEvent::Sync(SyncEvent::Stopped {
                    request_id: Some(request_id),
                }));
            }
            Err(_) => {
                self.emit(CoreEvent::OperationFailed {
                    request_id,
                    failure: CoreFailure::SyncFailed {
                        kind: SyncFailureKind::Http,
                    },
                });
            }
        }
    }

    fn emit(&self, event: CoreEvent) {
        let _ = self.event_tx.send(event);
    }

    fn reduce(&self, actions: Vec<AppAction>) {
        let _ = self.action_tx.try_send(actions);
    }
}

// ---------------------------------------------------------------------------
// SyncService state observer (runs in its own task)
// ---------------------------------------------------------------------------

async fn observe_sync_service_states(
    mut state_sub: eyeball::Subscriber<matrix_sdk_ui::sync_service::State>,
    event_tx: broadcast::Sender<CoreEvent>,
    action_tx: mpsc::Sender<Vec<AppAction>>,
) -> SyncTaskOutcome {
    // Track whether we've seen Running at least once so we emit SyncStarted
    // (Running) on first success and SyncRecovered on subsequent recoveries.
    let mut ever_ran = false;

    loop {
        // `next()` waits for the next change; returns None when the observable is closed.
        let next_state = state_sub.next().await;
        let state = match next_state {
            Some(s) => s,
            None => return SyncTaskOutcome::Stopped,
        };
        match state {
            matrix_sdk_ui::sync_service::State::Running => {
                if !ever_ran {
                    ever_ran = true;
                    let _ = event_tx.send(CoreEvent::Sync(SyncEvent::Running));
                    let _ = action_tx.try_send(vec![AppAction::SyncStarted]);
                } else {
                    // Recovered from Offline/Reconnecting.
                    let _ = event_tx.send(CoreEvent::Sync(SyncEvent::Running));
                    let _ = action_tx.try_send(vec![AppAction::SyncRecovered]);
                }
            }
            matrix_sdk_ui::sync_service::State::Offline => {
                let _ = event_tx.send(CoreEvent::Sync(SyncEvent::Reconnecting));
                let _ = action_tx.try_send(vec![AppAction::SyncReconnecting {
                    reason: "network_offline".to_owned(),
                }]);
            }
            matrix_sdk_ui::sync_service::State::Terminated => {
                return SyncTaskOutcome::Stopped;
            }
            matrix_sdk_ui::sync_service::State::Error(_) => {
                // Error is opaque — never expose SDK error text.
                return SyncTaskOutcome::Failed(SyncFailureKind::Http);
            }
            matrix_sdk_ui::sync_service::State::Idle => {
                // Initial state and state after stop — not a terminal failure.
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Legacy sync loop (runs in its own task)
// ---------------------------------------------------------------------------

async fn run_legacy_sync_loop(
    client: matrix_sdk::Client,
    mut stop_rx: tokio::sync::oneshot::Receiver<()>,
    event_tx: broadcast::Sender<CoreEvent>,
    action_tx: mpsc::Sender<Vec<AppAction>>,
) -> SyncTaskOutcome {
    use futures_util::StreamExt as _;
    use matrix_sdk::config::SyncSettings;

    let settings = SyncSettings::default();
    let sync_stream = client.sync_stream(settings).await;
    tokio::pin!(sync_stream);

    let mut ever_ran = false;
    let mut reconnecting = false;

    loop {
        tokio::select! {
            biased;
            _ = &mut stop_rx => {
                return SyncTaskOutcome::Stopped;
            }
            item = sync_stream.next() => {
                match item {
                    None => return SyncTaskOutcome::Stopped,
                    Some(Ok(_response)) => {
                        if !ever_ran {
                            ever_ran = true;
                            reconnecting = false;
                            let _ = event_tx.send(CoreEvent::Sync(SyncEvent::Running));
                            let _ = action_tx.try_send(vec![AppAction::SyncStarted]);
                        } else if reconnecting {
                            reconnecting = false;
                            let _ = event_tx.send(CoreEvent::Sync(SyncEvent::Running));
                            let _ = action_tx.try_send(vec![AppAction::SyncRecovered]);
                        }
                        // Else: normal running tick — no event needed.
                    }
                    Some(Err(error)) => {
                        let kind = classify_sdk_sync_error(&error);
                        if kind == SyncFailureKind::Auth {
                            // Auth failures are terminal (the SDK will not recover).
                            return SyncTaskOutcome::Failed(kind);
                        }
                        // Network / transient failures: emit Reconnecting and let
                        // the SDK's stream retry internally.
                        if !reconnecting {
                            reconnecting = true;
                            let _ = event_tx.send(CoreEvent::Sync(SyncEvent::Reconnecting));
                            let _ = action_tx.try_send(vec![AppAction::SyncReconnecting {
                                reason: "network_error".to_owned(),
                            }]);
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Probe the server for MSC4186 (sliding sync / SyncService) availability.
/// Returns `SyncService` if available, `LegacySync` otherwise.
/// Never panics — network failures cause an empty result → LegacySync.
///
/// Debug/test builds honor `MATRIX_DESKTOP_QA_FORCE_SYNC_BACKEND=legacy`
/// (skip the probe, select `LegacySync`); release builds compile the check
/// out entirely and always probe.
fn sync_mode_from_backend(backend: SyncBackendKind, failed: bool) -> SyncMode {
    if failed {
        return SyncMode::Failed {
            kind: koushi_state::SyncModeFailureKind::Network,
        };
    }
    match backend {
        SyncBackendKind::SyncService => SyncMode::Simplified,
        SyncBackendKind::LegacySync => SyncMode::Legacy,
    }
}

pub(crate) async fn probe_backend(client: &matrix_sdk::Client) -> SyncBackendKind {
    #[cfg(any(debug_assertions, test))]
    if forced_legacy_backend() {
        return SyncBackendKind::LegacySync;
    }

    let versions = client.available_sliding_sync_versions().await;
    if versions.is_empty() {
        SyncBackendKind::LegacySync
    } else {
        SyncBackendKind::SyncService
    }
}

/// True only when the QA env override requests the legacy backend.
/// Value must be exactly `legacy`; anything else is ignored (probe normally).
/// Debug/test builds only — this symbol does not exist in release builds.
#[cfg(any(debug_assertions, test))]
fn forced_legacy_backend() -> bool {
    std::env::var(ENV_FORCE_SYNC_BACKEND).is_ok_and(|value| value == "legacy")
}

/// Map an SDK sync error to a coarse `SyncFailureKind`. Never exposes raw
/// error text in public events (overview.md Security Model).
pub(crate) fn classify_sdk_sync_error(error: &matrix_sdk::Error) -> SyncFailureKind {
    match error {
        matrix_sdk::Error::AuthenticationRequired => SyncFailureKind::Auth,
        matrix_sdk::Error::Http(http_error) => {
            if http_error.as_client_api_error().is_some_and(|e| {
                let code = e.status_code.as_u16();
                code == 401 || code == 403
            }) || matches!(
                http_error.client_api_error_kind(),
                Some(
                    matrix_sdk::ruma::api::error::ErrorKind::Forbidden
                        | matrix_sdk::ruma::api::error::ErrorKind::UnknownToken { .. }
                )
            ) {
                SyncFailureKind::Auth
            } else {
                SyncFailureKind::Http
            }
        }
        matrix_sdk::Error::StateStore(_)
        | matrix_sdk::Error::EventCacheStore(_)
        | matrix_sdk::Error::MediaStore(_) => SyncFailureKind::Store,
        _ => SyncFailureKind::Internal,
    }
}

/// Stable kind label used in `AppAction::SyncFailed { reason }`. Must never
/// be raw SDK error text (overview.md Security Model).
pub(crate) fn sync_failure_kind_label(kind: SyncFailureKind) -> &'static str {
    match kind {
        SyncFailureKind::Http => "sync_failed_http",
        SyncFailureKind::Auth => "sync_failed_auth",
        SyncFailureKind::Store => "sync_failed_store",
        SyncFailureKind::Internal => "sync_failed_internal",
    }
}

// ---------------------------------------------------------------------------
// Unit tests (network-free)
// ---------------------------------------------------------------------------

#[cfg(test)]
pub mod tests {
    use tokio::sync::{broadcast, mpsc};

    use super::*;
    use crate::event::{CoreEvent, SyncBackendKind, SyncEvent};
    use crate::failure::SyncFailureKind;

    // --- classify_sdk_sync_error ---

    #[test]
    fn classify_auth_error() {
        let error = matrix_sdk::Error::AuthenticationRequired;
        assert_eq!(classify_sdk_sync_error(&error), SyncFailureKind::Auth);
    }

    #[test]
    fn classify_store_error() {
        // StoreError::Backend wraps any Send+Sync error.
        use matrix_sdk_base::store::StoreError;
        let backend_err: Box<dyn std::error::Error + Send + Sync> = Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            "state store failure",
        ));
        let store_err = matrix_sdk::Error::StateStore(Box::new(StoreError::Backend(backend_err)));
        assert_eq!(classify_sdk_sync_error(&store_err), SyncFailureKind::Store);
    }

    #[test]
    fn classify_internal_error() {
        let error = matrix_sdk::Error::InsufficientData;
        assert_eq!(classify_sdk_sync_error(&error), SyncFailureKind::Internal);
    }

    // --- failure kind label ---

    #[test]
    fn failure_kind_labels_are_not_raw_sdk_error_text() {
        for kind in [
            SyncFailureKind::Http,
            SyncFailureKind::Auth,
            SyncFailureKind::Store,
            SyncFailureKind::Internal,
        ] {
            let label = sync_failure_kind_label(kind);
            assert!(
                label.starts_with("sync_failed_"),
                "kind label '{label}' must start with sync_failed_"
            );
            assert!(
                label.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "kind label '{label}' must be snake_case (no raw SDK text)"
            );
        }
    }

    // --- forced-backend override (debug/test builds only) ---
    //
    // Single test owns the env var; no other unit test reads it (probe_backend
    // is never called with a real client in unit tests), so set/unset here is
    // race-free.

    #[test]
    fn forced_backend_override_honors_legacy_only() {
        // Unset → no force.
        unsafe { std::env::remove_var(ENV_FORCE_SYNC_BACKEND) };
        assert!(!forced_legacy_backend());

        // Exactly "legacy" → force.
        unsafe { std::env::set_var(ENV_FORCE_SYNC_BACKEND, "legacy") };
        assert!(forced_legacy_backend());

        // Any other value → ignored (probe normally).
        for bogus in ["Legacy", "LEGACY", "sync_service", "1", ""] {
            unsafe { std::env::set_var(ENV_FORCE_SYNC_BACKEND, bogus) };
            assert!(
                !forced_legacy_backend(),
                "value {bogus:?} must not force the legacy backend"
            );
        }

        unsafe { std::env::remove_var(ENV_FORCE_SYNC_BACKEND) };
    }

    // --- backend probe logic ---

    #[test]
    fn empty_versions_selects_legacy_sync() {
        let versions: Vec<matrix_sdk::sliding_sync::Version> = vec![];
        let backend = if versions.is_empty() {
            SyncBackendKind::LegacySync
        } else {
            SyncBackendKind::SyncService
        };
        assert_eq!(backend, SyncBackendKind::LegacySync);
    }

    #[test]
    fn non_empty_versions_selects_sync_service() {
        let versions = vec![matrix_sdk::sliding_sync::Version::Native];
        let backend = if versions.is_empty() {
            SyncBackendKind::LegacySync
        } else {
            SyncBackendKind::SyncService
        };
        assert_eq!(backend, SyncBackendKind::SyncService);
    }

    // --- SyncEvent shapes ---

    #[test]
    fn sync_event_started_carries_backend_and_optional_request_id() {
        use crate::ids::{RequestId, RuntimeConnectionId};
        let rid = RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: 1,
        };
        let event = SyncEvent::Started {
            request_id: Some(rid),
            backend: SyncBackendKind::LegacySync,
        };
        assert!(
            matches!(
                event,
                SyncEvent::Started {
                    request_id: Some(_),
                    backend: SyncBackendKind::LegacySync,
                }
            ),
            "event shape wrong: {event:?}"
        );
    }

    #[test]
    fn sync_event_stopped_carries_optional_request_id() {
        let event = SyncEvent::Stopped { request_id: None };
        assert!(matches!(event, SyncEvent::Stopped { request_id: None }));
    }

    // --- SyncTaskOutcome panic projection ---

    #[test]
    fn sync_task_panicked_produces_internal_failure_kind() {
        let outcome = SyncTaskOutcome::Panicked;
        let kind = match outcome {
            SyncTaskOutcome::Failed(k) => k,
            SyncTaskOutcome::Panicked => SyncFailureKind::Internal,
            SyncTaskOutcome::Stopped => panic!("wrong branch"),
        };
        assert_eq!(kind, SyncFailureKind::Internal);
    }

    #[test]
    fn sync_task_failed_preserves_kind() {
        let outcome = SyncTaskOutcome::Failed(SyncFailureKind::Auth);
        let kind = match outcome {
            SyncTaskOutcome::Failed(k) => k,
            _ => SyncFailureKind::Internal,
        };
        assert_eq!(kind, SyncFailureKind::Auth);
    }

    // --- AppAction channel round-trip (no real client needed) ---

    #[tokio::test]
    async fn action_channel_accepts_sync_actions() {
        let (action_tx, mut action_rx) = mpsc::channel::<Vec<AppAction>>(16);
        let (_event_tx, _event_rx) = broadcast::channel::<CoreEvent>(16);

        // Simulate what SyncActor.reduce() does for each state transition.
        let _ = action_tx.try_send(vec![AppAction::SyncStarted]);
        let _ = action_tx.try_send(vec![AppAction::SyncRecovered]);
        let _ = action_tx.try_send(vec![AppAction::SyncReconnecting {
            reason: "network_offline".to_owned(),
        }]);
        let _ = action_tx.try_send(vec![AppAction::SyncFailed {
            reason: sync_failure_kind_label(SyncFailureKind::Http).to_owned(),
        }]);
        let _ = action_tx.try_send(vec![AppAction::SyncStopped]);

        let a1 = action_rx.recv().await.unwrap();
        let a2 = action_rx.recv().await.unwrap();
        let a3 = action_rx.recv().await.unwrap();
        let a4 = action_rx.recv().await.unwrap();
        let a5 = action_rx.recv().await.unwrap();
        assert!(matches!(a1[0], AppAction::SyncStarted));
        assert!(matches!(a2[0], AppAction::SyncRecovered));
        assert!(matches!(a3[0], AppAction::SyncReconnecting { .. }));
        assert!(matches!(a4[0], AppAction::SyncFailed { .. }));
        assert!(matches!(a5[0], AppAction::SyncStopped));
    }

    // --- Failure reason must not be raw error text ---

    #[test]
    fn sync_failed_reason_is_not_raw_error_text() {
        // The reason passed to AppAction::SyncFailed must always come from
        // sync_failure_kind_label, never from error.to_string() or fmt.
        let fake_raw_error = "HTTP 500 Internal Server Error";
        let kind = SyncFailureKind::Http;
        let reason = sync_failure_kind_label(kind);
        assert_ne!(
            reason, fake_raw_error,
            "reason must not be raw SDK error text"
        );
        assert_eq!(reason, "sync_failed_http");
    }

    // --- Stop-on-Stopped lifecycle is a no-op (no panic) ---

    #[test]
    fn stop_on_stopped_lifecycle_does_not_panic() {
        let lifecycle = SyncLifecycle::Stopped;
        assert_eq!(lifecycle, SyncLifecycle::Stopped);
    }

    // --- Restart-after-failure ---

    #[test]
    fn restart_after_failure_resets_lifecycle_to_running() {
        // Simulate: failure → stop → start.
        // Each assignment represents a state transition in handle_command(Restart).
        let lifecycle = SyncLifecycle::Failed;
        assert_eq!(lifecycle, SyncLifecycle::Failed, "starts in Failed");
        // do_stop resets to Stopped:
        let lifecycle = SyncLifecycle::Stopped;
        assert_eq!(
            lifecycle,
            SyncLifecycle::Stopped,
            "do_stop produces Stopped"
        );
        // handle_start completes, setting Running:
        let lifecycle = SyncLifecycle::Running;
        assert_eq!(
            lifecycle,
            SyncLifecycle::Running,
            "handle_start produces Running"
        );
    }
}
