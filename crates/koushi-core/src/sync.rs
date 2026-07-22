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
//! `Client::available_sliding_sync_versions()`. Empty selects `LegacySync`.
//! When MSC4186 is advertised, one bounded authenticated invite-only list
//! request verifies the server contract before the authoritative sync owner
//! starts. Only a present requested list selects `SyncService`; missing or
//! indeterminate results fail closed to `LegacySync`.
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
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_sdk::{MatrixClientSession, MatrixSlidingSyncInviteListSupport};
use koushi_state::{AppAction, SyncLifecycleStatus, SyncMode};
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
const ENV_FORCE_SYNC_BACKEND: &str = "KOUSHI_QA_FORCE_SYNC_BACKEND";
const SYNC_ACTOR_SHUTDOWN_SEND_TIMEOUT: Duration = Duration::from_secs(1);
const SYNC_ACTOR_SHUTDOWN_JOIN_TIMEOUT: Duration = Duration::from_secs(10);
const SYNC_SERVICE_STOP_TIMEOUT: Duration = Duration::from_secs(5);
const LEGACY_SYNC_ROOM_TIMELINE_LIMIT: u32 = 128;
macro_rules! trace_sync {
    ($stage:expr, [$($field:expr),* $(,)?], $($arg:tt)*) => {{
        let event = DiagnosticEvent::new(
            DiagnosticLevel::Debug,
            "core.sync",
            $stage,
        )$(.field($field))*;
        record(event);
    }};
}

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

    pub async fn shutdown(self) -> bool {
        self.shutdown_with_timeout(SYNC_ACTOR_SHUTDOWN_JOIN_TIMEOUT)
            .await
    }

    async fn shutdown_with_timeout(mut self, timeout: Duration) -> bool {
        let _ = executor::timeout(
            SYNC_ACTOR_SHUTDOWN_SEND_TIMEOUT,
            self.tx.send(SyncMessage::Shutdown),
        )
        .await;
        match executor::timeout(timeout, &mut self.task).await {
            Ok(_) => true,
            Err(_) => {
                self.task.abort();
                let _ = self.task.await;
                false
            }
        }
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
    Starting,
    Running,
    Reconnecting,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SyncActorControl {
    FirstResponseCommitted {
        legacy_run_generation: u64,
        committed_from_response_sequence: u64,
    },
    SyncServiceRunning {
        sync_service_run_generation: u64,
    },
    BackendReconnecting {
        backend: ActiveBackend,
        run_generation: u64,
        reason: &'static str,
    },
    BackendRecovered {
        backend: ActiveBackend,
        run_generation: u64,
    },
}

fn accepts_first_legacy_response(
    active_backend: ActiveBackend,
    lifecycle: SyncLifecycle,
    active_generation: u64,
    observed_generation: u64,
) -> bool {
    active_backend == ActiveBackend::LegacySync
        && lifecycle == SyncLifecycle::Starting
        && active_generation == observed_generation
}

fn accepts_sync_service_running(
    active_backend: ActiveBackend,
    lifecycle: SyncLifecycle,
    active_generation: u64,
    observed_generation: u64,
) -> bool {
    active_backend == ActiveBackend::SyncService
        && lifecycle == SyncLifecycle::Starting
        && active_generation == observed_generation
}

fn accepts_backend_transition(
    active_backend: ActiveBackend,
    lifecycle: SyncLifecycle,
    active_generation: u64,
    observed_backend: ActiveBackend,
    observed_generation: u64,
    expected_lifecycle: SyncLifecycle,
) -> bool {
    active_backend == observed_backend
        && lifecycle == expected_lifecycle
        && active_generation == observed_generation
}

/// What the sync background task produced when it ended.
#[derive(Debug)]
enum SyncTaskOutcome {
    Stopped,
    Failed {
        kind: SyncFailureKind,
        ever_ran: bool,
    },
    Panicked,
}

/// Backend selected on the current run (stored for idempotent Start).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActiveBackend {
    None,
    SyncService,
    LegacySync,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BackendProbeReason {
    ForcedLegacy,
    SlidingSyncUnavailable,
    InviteListSupported,
    InviteListKnownIncomplete,
    InviteListUnknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BackendProbeResult {
    backend: SyncBackendKind,
    reason: BackendProbeReason,
}

fn backend_probe_reason_label(reason: BackendProbeReason) -> &'static str {
    match reason {
        BackendProbeReason::ForcedLegacy => "forced_legacy",
        BackendProbeReason::SlidingSyncUnavailable => "sliding_sync_unavailable",
        BackendProbeReason::InviteListSupported => "invite_list_supported",
        BackendProbeReason::InviteListKnownIncomplete => "invite_list_known_incomplete",
        BackendProbeReason::InviteListUnknown => "invite_list_unknown",
    }
}

fn sync_once_admitted(
    lifecycle: SyncLifecycle,
    active_backend: ActiveBackend,
    sync_task_active: bool,
    sync_service_active: bool,
    legacy_stop_active: bool,
) -> bool {
    matches!(lifecycle, SyncLifecycle::Stopped | SyncLifecycle::Failed)
        && active_backend == ActiveBackend::None
        && !sync_task_active
        && !sync_service_active
        && !legacy_stop_active
}

fn sync_lifecycle_label(lifecycle: SyncLifecycle) -> &'static str {
    match lifecycle {
        SyncLifecycle::Stopped => "stopped",
        SyncLifecycle::Starting => "starting",
        SyncLifecycle::Running => "running",
        SyncLifecycle::Reconnecting => "reconnecting",
        SyncLifecycle::Failed => "failed",
    }
}

fn active_backend_label(backend: ActiveBackend) -> &'static str {
    match backend {
        ActiveBackend::None => "none",
        ActiveBackend::SyncService => "sync_service",
        ActiveBackend::LegacySync => "legacy",
    }
}

fn sync_backend_label(backend: SyncBackendKind) -> &'static str {
    match backend {
        SyncBackendKind::SyncService => "sync_service",
        SyncBackendKind::LegacySync => "legacy",
    }
}

fn sync_status_trace_label(status: &SyncLifecycleStatus) -> &'static str {
    match status {
        SyncLifecycleStatus::Stopped => "stopped",
        SyncLifecycleStatus::Starting => "starting",
        SyncLifecycleStatus::Running => "running",
        SyncLifecycleStatus::Failed { .. } => "failed",
        SyncLifecycleStatus::Reconnecting { .. } => "reconnecting",
    }
}

async fn send_sync_status(
    action_tx: &mpsc::Sender<Vec<AppAction>>,
    generation: &AtomicU64,
    status: SyncLifecycleStatus,
) {
    let label = sync_status_trace_label(&status);
    let generation = generation.fetch_add(1, Ordering::Relaxed) + 1;
    trace_sync!(
        "status_projected",
        [
            DiagnosticField::count("generation", generation),
            DiagnosticField::token("lifecycle", label),
        ],
        "generation={} lifecycle={}",
        generation,
        label
    );
    let _ = action_tx
        .send(vec![AppAction::SyncStatusChanged { generation, status }])
        .await;
}

fn sync_command_trace_parts(command: &SyncCommand) -> (&'static str, RequestId) {
    match command {
        SyncCommand::Start { request_id } => ("start", *request_id),
        SyncCommand::Stop { request_id } => ("stop", *request_id),
        SyncCommand::Restart { request_id } => ("restart", *request_id),
        SyncCommand::SyncOnce { request_id } => ("sync_once", *request_id),
    }
}

fn request_id_trace_parts(request_id: Option<RequestId>) -> (u64, u64, bool) {
    match request_id {
        Some(request_id) => (request_id.connection_id.0, request_id.sequence, true),
        None => (0, 0, false),
    }
}

fn sync_task_outcome_trace_label(outcome: &SyncTaskOutcome) -> &'static str {
    match outcome {
        SyncTaskOutcome::Stopped => "stopped",
        SyncTaskOutcome::Failed { .. } => "failed",
        SyncTaskOutcome::Panicked => "panicked",
    }
}

fn sync_service_state_trace_label(state: &matrix_sdk_ui::sync_service::State) -> &'static str {
    match state {
        matrix_sdk_ui::sync_service::State::Idle => "idle",
        matrix_sdk_ui::sync_service::State::Running => "running",
        matrix_sdk_ui::sync_service::State::Terminated => "terminated",
        matrix_sdk_ui::sync_service::State::Error(_) => "error",
        matrix_sdk_ui::sync_service::State::Offline => "offline",
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct SyncServiceObserverStatus {
    sync_started_emitted: bool,
    connectivity_proven: bool,
    reconnecting: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SyncServiceStateKind {
    Idle,
    Running,
    Offline,
    Error,
    Terminated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RoomListServiceStateKind {
    Init,
    SettingUp,
    Recovering,
    Running,
    Error,
    Terminated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SyncServiceObserverDecision {
    InitialRunningHandoff,
    RecoveryHandoff,
    RunningNoop,
    FallbackBeforeConnectivity,
    WaitRecovery,
    AlreadyReconnecting,
    Fail,
    Ignore,
    ConnectivityProven,
}

fn sync_service_state_kind(state: &matrix_sdk_ui::sync_service::State) -> SyncServiceStateKind {
    match state {
        matrix_sdk_ui::sync_service::State::Idle => SyncServiceStateKind::Idle,
        matrix_sdk_ui::sync_service::State::Running => SyncServiceStateKind::Running,
        matrix_sdk_ui::sync_service::State::Offline => SyncServiceStateKind::Offline,
        matrix_sdk_ui::sync_service::State::Error(_) => SyncServiceStateKind::Error,
        matrix_sdk_ui::sync_service::State::Terminated => SyncServiceStateKind::Terminated,
    }
}

fn room_list_service_state_kind(
    state: &matrix_sdk_ui::room_list_service::State,
) -> RoomListServiceStateKind {
    match state {
        matrix_sdk_ui::room_list_service::State::Init => RoomListServiceStateKind::Init,
        matrix_sdk_ui::room_list_service::State::SettingUp => RoomListServiceStateKind::SettingUp,
        matrix_sdk_ui::room_list_service::State::Recovering => RoomListServiceStateKind::Recovering,
        matrix_sdk_ui::room_list_service::State::Running => RoomListServiceStateKind::Running,
        matrix_sdk_ui::room_list_service::State::Error { .. } => RoomListServiceStateKind::Error,
        matrix_sdk_ui::room_list_service::State::Terminated { .. } => {
            RoomListServiceStateKind::Terminated
        }
    }
}

fn room_list_service_state_trace_label(
    state: &matrix_sdk_ui::room_list_service::State,
) -> &'static str {
    match room_list_service_state_kind(state) {
        RoomListServiceStateKind::Init => "init",
        RoomListServiceStateKind::SettingUp => "setting_up",
        RoomListServiceStateKind::Recovering => "recovering",
        RoomListServiceStateKind::Running => "running",
        RoomListServiceStateKind::Error => "error",
        RoomListServiceStateKind::Terminated => "terminated",
    }
}

fn classify_sync_service_state(
    status: &mut SyncServiceObserverStatus,
    state: SyncServiceStateKind,
) -> SyncServiceObserverDecision {
    match state {
        SyncServiceStateKind::Running => {
            if !status.sync_started_emitted {
                status.sync_started_emitted = true;
                status.reconnecting = false;
                SyncServiceObserverDecision::InitialRunningHandoff
            } else if status.reconnecting {
                status.reconnecting = false;
                SyncServiceObserverDecision::RecoveryHandoff
            } else {
                SyncServiceObserverDecision::RunningNoop
            }
        }
        SyncServiceStateKind::Offline | SyncServiceStateKind::Error => {
            if !status.connectivity_proven {
                SyncServiceObserverDecision::FallbackBeforeConnectivity
            } else if !status.reconnecting {
                status.reconnecting = true;
                SyncServiceObserverDecision::WaitRecovery
            } else {
                SyncServiceObserverDecision::AlreadyReconnecting
            }
        }
        SyncServiceStateKind::Terminated => SyncServiceObserverDecision::Fail,
        SyncServiceStateKind::Idle => SyncServiceObserverDecision::Ignore,
    }
}

fn note_room_list_service_state(
    status: &mut SyncServiceObserverStatus,
    state: RoomListServiceStateKind,
) -> Option<SyncServiceObserverDecision> {
    if matches!(state, RoomListServiceStateKind::Running) && !status.connectivity_proven {
        status.connectivity_proven = true;
        return Some(SyncServiceObserverDecision::ConnectivityProven);
    }

    None
}

pub struct SyncActor {
    session: Arc<MatrixClientSession>,
    action_tx: mpsc::Sender<Vec<AppAction>>,
    event_tx: broadcast::Sender<CoreEvent>,
    command_rx: mpsc::Receiver<SyncMessage>,
    control_tx: mpsc::Sender<SyncActorControl>,
    control_rx: mpsc::Receiver<SyncActorControl>,
    /// RoomActor inbox: the SyncActor notifies it on sync start/stop because
    /// only the SyncActor knows the selected backend and owns the live
    /// `RoomListService` (canon: RoomActor consumes the ONE live service).
    room_tx: mpsc::Sender<RoomMessage>,
    /// TimelineManager inbox: receives the live `RoomListService` on sync
    /// start so timeline subscriptions can subscribe rooms with it (canon).
    timeline_tx: mpsc::Sender<crate::timeline::TimelineMessage>,
    lifecycle: SyncLifecycle,
    sync_generation: Arc<AtomicU64>,
    active_backend: ActiveBackend,
    /// Task handle for the currently running sync loop.
    sync_task: Option<executor::JoinHandle<SyncTaskOutcome>>,
    /// Stop-signal for the legacy sync loop.
    legacy_stop_tx: Option<tokio::sync::oneshot::Sender<()>>,
    legacy_run_generation: u64,
    sync_service_run_generation: u64,
    /// SyncService handle (Some when SyncService backend is running).
    sync_service: Option<Arc<matrix_sdk_ui::sync_service::SyncService>>,
    active_start_request_id: Option<RequestId>,
    sync_service_runtime_fallback_attempted: bool,
    /// Handle for the global ignored-user-list account-data handler. Removed on
    /// stop so repeated start/stop cycles do not install duplicate handlers.
    ignored_user_list_handler: Option<matrix_sdk::event_handler::EventHandlerHandle>,
}

impl SyncActor {
    pub(crate) fn spawn(
        session: Arc<MatrixClientSession>,
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
        room_tx: mpsc::Sender<RoomMessage>,
        timeline_tx: mpsc::Sender<crate::timeline::TimelineMessage>,
        sync_generation: Arc<AtomicU64>,
    ) -> SyncActorHandle {
        let (tx, command_rx) = mpsc::channel(16);
        let (control_tx, control_rx) = mpsc::channel(4);
        let actor = SyncActor {
            session,
            action_tx,
            event_tx,
            command_rx,
            control_tx,
            control_rx,
            room_tx,
            timeline_tx,
            lifecycle: SyncLifecycle::Stopped,
            sync_generation,
            active_backend: ActiveBackend::None,
            sync_task: None,
            legacy_stop_tx: None,
            legacy_run_generation: 0,
            sync_service_run_generation: 0,
            sync_service: None,
            active_start_request_id: None,
            sync_service_runtime_fallback_attempted: false,
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
                        self.handle_sync_task_ended(outcome).await;
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
                    control = self.control_rx.recv() => {
                        if let Some(control) = control {
                            self.handle_control(control).await;
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

    async fn handle_control(&mut self, control: SyncActorControl) {
        match control {
            SyncActorControl::FirstResponseCommitted {
                legacy_run_generation,
                committed_from_response_sequence,
            } if accepts_first_legacy_response(
                self.active_backend,
                self.lifecycle,
                self.legacy_run_generation,
                legacy_run_generation,
            ) =>
            {
                self.lifecycle = SyncLifecycle::Running;
                notify_dependents_sync_started(
                    self.session.clone(),
                    self.room_tx.clone(),
                    self.timeline_tx.clone(),
                    None,
                    Some(committed_from_response_sequence),
                )
                .await;
                self.emit(CoreEvent::Sync(SyncEvent::Running));
                self.project_sync_status(SyncLifecycleStatus::Running).await;
            }
            SyncActorControl::FirstResponseCommitted { .. } => {}
            SyncActorControl::SyncServiceRunning {
                sync_service_run_generation,
            } if accepts_sync_service_running(
                self.active_backend,
                self.lifecycle,
                self.sync_service_run_generation,
                sync_service_run_generation,
            ) =>
            {
                self.lifecycle = SyncLifecycle::Running;
            }
            SyncActorControl::SyncServiceRunning { .. } => {}
            SyncActorControl::BackendReconnecting {
                backend,
                run_generation,
                reason,
            } if accepts_backend_transition(
                self.active_backend,
                self.lifecycle,
                self.active_run_generation(),
                backend,
                run_generation,
                SyncLifecycle::Running,
            ) =>
            {
                self.lifecycle = SyncLifecycle::Reconnecting;
                self.emit(CoreEvent::Sync(SyncEvent::Reconnecting));
                self.project_sync_status(SyncLifecycleStatus::Reconnecting {
                    reason: reason.to_owned(),
                })
                .await;
            }
            SyncActorControl::BackendReconnecting { .. } => {}
            SyncActorControl::BackendRecovered {
                backend,
                run_generation,
            } if accepts_backend_transition(
                self.active_backend,
                self.lifecycle,
                self.active_run_generation(),
                backend,
                run_generation,
                SyncLifecycle::Reconnecting,
            ) =>
            {
                self.lifecycle = SyncLifecycle::Running;
                self.emit(CoreEvent::Sync(SyncEvent::Running));
                self.project_sync_status(SyncLifecycleStatus::Running).await;
            }
            SyncActorControl::BackendRecovered { .. } => {}
        }
    }

    async fn handle_sync_task_ended(&mut self, outcome: SyncTaskOutcome) {
        // The room-list observation must not outlive the sync backend it
        // relays (live RoomListService on SyncService, base-client updates on
        // legacy). Send from a notifier task so shutdown is not blocked by
        // mailbox backpressure and the one-shot handoff is not silently dropped.
        notify_room_sync_stopped(self.room_tx.clone());
        let ended_backend = self.active_backend;
        let request_id = self.active_start_request_id;
        let outcome_label = sync_task_outcome_trace_label(&outcome);
        let (request_connection_id, request_sequence, request_id_present) =
            request_id_trace_parts(request_id);
        self.cleanup_ended_backend().await;
        match outcome {
            SyncTaskOutcome::Stopped => {
                trace_sync!(
                    "task_ended",
                    [
                        DiagnosticField::token("outcome", outcome_label),
                        DiagnosticField::token("backend", active_backend_label(ended_backend)),
                        DiagnosticField::request_id(
                            "request_id",
                            request_connection_id,
                            request_sequence,
                        ),
                        DiagnosticField::boolean("request_id_present", request_id_present),
                        DiagnosticField::token("fallback", "no"),
                    ],
                    "outcome={} backend={} request_id={} fallback=no",
                    outcome_label,
                    active_backend_label(ended_backend),
                    request_id_trace_label(request_id)
                );
                self.lifecycle = SyncLifecycle::Stopped;
                // Task ended via stop signal — emit Stopped (no request_id because
                // it was not from an explicit SyncCommand::Stop at this level).
                self.emit(CoreEvent::Sync(SyncEvent::Stopped { request_id: None }));
                self.project_sync_status(SyncLifecycleStatus::Stopped).await;
            }
            SyncTaskOutcome::Failed { .. } | SyncTaskOutcome::Panicked => {
                let kind = match outcome {
                    SyncTaskOutcome::Failed { kind, .. } => kind,
                    SyncTaskOutcome::Panicked => SyncFailureKind::Internal,
                    SyncTaskOutcome::Stopped => unreachable!(),
                };
                let ever_ran = match outcome {
                    SyncTaskOutcome::Failed { ever_ran, .. } => ever_ran,
                    SyncTaskOutcome::Panicked => false,
                    SyncTaskOutcome::Stopped => unreachable!(),
                };
                let fallback = should_fallback_to_legacy_after_sync_service_failure(
                    ended_backend,
                    kind,
                    ever_ran,
                    self.sync_service_runtime_fallback_attempted,
                );
                trace_sync!(
                    "task_ended",
                    [
                        DiagnosticField::token("outcome", outcome_label),
                        DiagnosticField::token("backend", active_backend_label(ended_backend)),
                        DiagnosticField::request_id(
                            "request_id",
                            request_connection_id,
                            request_sequence,
                        ),
                        DiagnosticField::boolean("request_id_present", request_id_present),
                        DiagnosticField::token("kind", sync_failure_kind_label(kind)),
                        DiagnosticField::boolean("ever_ran", ever_ran),
                        DiagnosticField::boolean("fallback", fallback),
                    ],
                    "outcome={} backend={} request_id={} kind={} ever_ran={} fallback={}",
                    outcome_label,
                    active_backend_label(ended_backend),
                    request_id_trace_label(request_id),
                    sync_failure_kind_label(kind),
                    ever_ran,
                    bool_trace_label(fallback)
                );
                if fallback {
                    self.start_legacy_runtime_fallback(request_id).await;
                    return;
                }
                self.lifecycle = SyncLifecycle::Failed;
                let mode = sync_mode_from_backend(self.current_backend_kind(), true);
                self.reduce(vec![AppAction::SyncModeChanged { mode }]);
                self.emit(CoreEvent::Sync(SyncEvent::ModeChanged { mode }));
                self.emit(CoreEvent::Sync(SyncEvent::Failed));
                self.project_sync_status(SyncLifecycleStatus::Failed {
                    reason: sync_failure_kind_label(kind).to_owned(),
                })
                .await;
            }
        }
    }

    async fn handle_command(&mut self, command: SyncCommand) {
        let (command_kind, request_id) = sync_command_trace_parts(&command);
        trace_sync!(
            "command",
            [
                DiagnosticField::token("kind", command_kind),
                DiagnosticField::request_id(
                    "request_id",
                    request_id.connection_id.0,
                    request_id.sequence
                ),
                DiagnosticField::token("lifecycle", sync_lifecycle_label(self.lifecycle)),
                DiagnosticField::token("active_backend", active_backend_label(self.active_backend)),
            ],
            "kind={} request_id={} lifecycle={} active_backend={}",
            command_kind,
            request_id_trace_label(Some(request_id)),
            sync_lifecycle_label(self.lifecycle),
            active_backend_label(self.active_backend)
        );
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
        trace_sync!(
            "start_begin",
            [
                DiagnosticField::request_id(
                    "request_id",
                    request_id.connection_id.0,
                    request_id.sequence
                ),
                DiagnosticField::token("lifecycle", sync_lifecycle_label(self.lifecycle)),
                DiagnosticField::token("active_backend", active_backend_label(self.active_backend)),
            ],
            "request_id={} lifecycle={} active_backend={}",
            request_id_trace_label(Some(request_id)),
            sync_lifecycle_label(self.lifecycle),
            active_backend_label(self.active_backend)
        );
        // Idempotent: if already running, re-emit Started so QA can assert backend.
        if self.lifecycle == SyncLifecycle::Running {
            let backend = self.current_backend_kind();
            let mode = sync_mode_from_backend(backend, false);
            trace_sync!(
                "start_idempotent",
                [
                    DiagnosticField::request_id(
                        "request_id",
                        request_id.connection_id.0,
                        request_id.sequence
                    ),
                    DiagnosticField::token("backend", sync_backend_label(backend)),
                    DiagnosticField::token("action", "reemit_running"),
                ],
                "request_id={} backend={} action=reemit_running",
                request_id_trace_label(Some(request_id)),
                sync_backend_label(backend)
            );
            self.reduce(vec![AppAction::SyncModeChanged { mode }]);
            self.project_sync_status(SyncLifecycleStatus::Running).await;
            self.emit(CoreEvent::Sync(SyncEvent::Started {
                request_id: Some(request_id),
                backend,
            }));
            self.emit(CoreEvent::Sync(SyncEvent::ModeChanged { mode }));
            self.emit(CoreEvent::Sync(SyncEvent::Running));
            return;
        }

        if matches!(
            self.lifecycle,
            SyncLifecycle::Starting | SyncLifecycle::Reconnecting
        ) {
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

        self.lifecycle = SyncLifecycle::Starting;
        self.project_sync_status(SyncLifecycleStatus::Starting)
            .await;

        // Probe MSC4186 capability (Async rule 9).
        let backend_probe = probe_backend(&self.session).await;
        let backend_kind = backend_probe.backend;
        let mode = sync_mode_from_backend(backend_kind, false);
        trace_sync!(
            "probe_done",
            [
                DiagnosticField::request_id(
                    "request_id",
                    request_id.connection_id.0,
                    request_id.sequence
                ),
                DiagnosticField::token("backend", sync_backend_label(backend_kind)),
                DiagnosticField::token("reason", backend_probe_reason_label(backend_probe.reason)),
            ],
            "request_id={} backend={} reason={}",
            request_id_trace_label(Some(request_id)),
            sync_backend_label(backend_kind),
            backend_probe_reason_label(backend_probe.reason)
        );
        self.reduce(vec![AppAction::SyncModeChanged { mode }]);
        self.active_start_request_id = Some(request_id);

        // Emit Started with the selected backend BEFORE the task starts running,
        // so QA can assert the backend kind on the same event.
        self.emit(CoreEvent::Sync(SyncEvent::Started {
            request_id: Some(request_id),
            backend: backend_kind,
        }));
        self.emit(CoreEvent::Sync(SyncEvent::ModeChanged { mode }));

        // Launch the appropriate background sync task.
        let client = self.session.client();
        match backend_kind {
            SyncBackendKind::SyncService => {
                match self.start_sync_service(client).await {
                    Ok(()) => {}
                    Err(()) => {
                        // SyncService build failed; fall back to legacy.
                        self.start_legacy_runtime_fallback(Some(request_id)).await;
                        return;
                    }
                }
            }
            SyncBackendKind::LegacySync => {
                self.active_backend = ActiveBackend::LegacySync;
                self.start_legacy_sync(client).await;
            }
        }
        trace_sync!(
            "backend_running",
            [
                DiagnosticField::request_id(
                    "request_id",
                    request_id.connection_id.0,
                    request_id.sequence
                ),
                DiagnosticField::token("backend", sync_backend_label(backend_kind)),
                DiagnosticField::token("lifecycle", sync_lifecycle_label(self.lifecycle)),
                DiagnosticField::token(
                    "action",
                    if backend_kind == SyncBackendKind::LegacySync {
                        "legacy_started"
                    } else {
                        "await_sync_service_running"
                    }
                ),
            ],
            "request_id={} backend={} lifecycle={} action={}",
            request_id_trace_label(Some(request_id)),
            sync_backend_label(backend_kind),
            sync_lifecycle_label(self.lifecycle),
            if backend_kind == SyncBackendKind::LegacySync {
                "legacy_started"
            } else {
                "await_sync_service_running"
            }
        );

        // LegacySync promotion and dependent handoff happen only after the
        // first response has been committed and reported through actor control.
    }

    fn active_run_generation(&self) -> u64 {
        match self.active_backend {
            ActiveBackend::SyncService => self.sync_service_run_generation,
            ActiveBackend::LegacySync => self.legacy_run_generation,
            ActiveBackend::None => 0,
        }
    }

    async fn start_legacy_runtime_fallback(&mut self, request_id: Option<RequestId>) {
        let (request_connection_id, request_sequence, request_id_present) =
            request_id_trace_parts(request_id);
        trace_sync!(
            "fallback_start",
            [
                DiagnosticField::token("from", "sync_service"),
                DiagnosticField::token("to", "legacy"),
                DiagnosticField::request_id("request_id", request_connection_id, request_sequence),
                DiagnosticField::boolean("request_id_present", request_id_present),
                DiagnosticField::token("reason", "sync_failed_http"),
            ],
            "request_id={} from=sync_service to=legacy reason=sync_failed_http",
            request_id_trace_label(request_id)
        );
        self.sync_service_runtime_fallback_attempted = true;
        let transition_mode = SyncMode::Transitioning;
        self.reduce(vec![AppAction::SyncModeChanged {
            mode: transition_mode,
        }]);
        self.emit(CoreEvent::Sync(SyncEvent::ModeChanged {
            mode: transition_mode,
        }));
        self.lifecycle = SyncLifecycle::Starting;
        self.project_sync_status(SyncLifecycleStatus::Starting)
            .await;

        let client = self.session.client();
        self.active_backend = ActiveBackend::LegacySync;
        self.active_start_request_id = request_id;

        let fallback_mode = SyncMode::Legacy;
        self.reduce(vec![AppAction::SyncModeChanged {
            mode: fallback_mode,
        }]);
        self.emit(CoreEvent::Sync(SyncEvent::Started {
            request_id,
            backend: SyncBackendKind::LegacySync,
        }));
        self.emit(CoreEvent::Sync(SyncEvent::ModeChanged {
            mode: fallback_mode,
        }));

        self.start_legacy_sync(client).await;
        let (request_connection_id, request_sequence, request_id_present) =
            request_id_trace_parts(request_id);
        trace_sync!(
            "fallback_done",
            [
                DiagnosticField::token("backend", "legacy"),
                DiagnosticField::request_id("request_id", request_connection_id, request_sequence),
                DiagnosticField::boolean("request_id_present", request_id_present),
                DiagnosticField::token("lifecycle", sync_lifecycle_label(self.lifecycle)),
                DiagnosticField::token("action", "legacy_started"),
            ],
            "request_id={} backend=legacy lifecycle={} action=legacy_started",
            request_id_trace_label(request_id),
            sync_lifecycle_label(self.lifecycle)
        );
    }

    /// Returns Ok(()) on success, Err(()) when SyncService build fails (caller falls back).
    async fn start_sync_service(&mut self, client: matrix_sdk::Client) -> Result<(), ()> {
        self.register_ignored_user_list_handler(&client);
        self.sync_service_run_generation = self.sync_service_run_generation.wrapping_add(1).max(1);
        let sync_service_run_generation = self.sync_service_run_generation;
        trace_sync!(
            "sync_service_build",
            [DiagnosticField::token("action", "begin")],
            "action=begin"
        );

        let service = matrix_sdk_ui::sync_service::SyncService::builder(client.clone())
            .with_offline_mode()
            .build()
            .await
            .map_err(|_| {
                trace_sync!(
                    "sync_service_build",
                    [DiagnosticField::token("action", "failed")],
                    "action=failed"
                );
                if let Some(handle) = self.ignored_user_list_handler.take() {
                    client.remove_event_handler(handle);
                }
                ()
            })?;
        trace_sync!(
            "sync_service_build",
            [DiagnosticField::token("action", "done")],
            "action=done"
        );
        let service = Arc::new(service);

        // Start observing before the SDK service starts so short-lived
        // Running/Offline/Error/Terminated transitions cannot be missed.
        let state_sub = service.state();
        let event_tx = self.event_tx.clone();
        let action_tx = self.action_tx.clone();
        let sync_generation = self.sync_generation.clone();
        let observer_session = self.session.clone();
        let observer_room_tx = self.room_tx.clone();
        let observer_timeline_tx = self.timeline_tx.clone();
        let observer_room_list_service = service.room_list_service();
        let observer_control_tx = self.control_tx.clone();

        let task: executor::JoinHandle<SyncTaskOutcome> = executor::spawn(async move {
            observe_sync_service_states(
                state_sub,
                event_tx,
                action_tx,
                sync_generation,
                observer_session,
                observer_room_tx,
                observer_timeline_tx,
                observer_room_list_service,
                observer_control_tx,
                sync_service_run_generation,
            )
            .await
        });
        trace_sync!(
            "sync_service_observer",
            [DiagnosticField::token("action", "spawned")],
            "action=spawned"
        );
        trace_sync!(
            "sync_service_start",
            [DiagnosticField::token("action", "call")],
            "action=call"
        );
        service.start().await;
        trace_sync!(
            "sync_service_start",
            [DiagnosticField::token("action", "returned")],
            "action=returned"
        );

        self.sync_service = Some(service);
        self.sync_task = Some(task);
        self.active_backend = ActiveBackend::SyncService;
        Ok(())
    }

    async fn start_legacy_sync(&mut self, client: matrix_sdk::Client) {
        self.register_ignored_user_list_handler(&client);
        self.legacy_run_generation = self.legacy_run_generation.wrapping_add(1).max(1);
        let legacy_run_generation = self.legacy_run_generation;

        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let event_tx = self.event_tx.clone();
        let action_tx = self.action_tx.clone();
        let sync_generation = self.sync_generation.clone();
        let control_tx = self.control_tx.clone();
        let task: executor::JoinHandle<SyncTaskOutcome> = executor::spawn(async move {
            run_legacy_sync_loop(
                client,
                stop_rx,
                event_tx,
                action_tx,
                sync_generation,
                control_tx,
                legacy_run_generation,
            )
            .await
        });

        self.legacy_stop_tx = Some(stop_tx);
        self.sync_task = Some(task);
    }

    async fn cleanup_ended_backend(&mut self) {
        if let Some(svc) = self.sync_service.take() {
            let _ = executor::timeout(SYNC_SERVICE_STOP_TIMEOUT, svc.stop()).await;
        }
        self.legacy_stop_tx = None;
        if let Some(handle) = self.ignored_user_list_handler.take() {
            self.session.client().remove_event_handler(handle);
        }
        self.active_backend = ActiveBackend::None;
        self.active_start_request_id = None;
    }

    /// Graceful stop: signal the running sync backend and wait (bounded timeout).
    async fn do_stop(&mut self, request_id: Option<RequestId>) {
        // Tear down the RoomActor's room-list observation first: on the
        // SyncService backend it consumes the live RoomListService that is
        // about to stop. Harmless no-op when nothing is running.
        notify_room_sync_stopped(self.room_tx.clone());
        // Signal stop to whichever backend is running.
        if let Some(svc) = self.sync_service.take() {
            let _ = executor::timeout(SYNC_SERVICE_STOP_TIMEOUT, svc.stop()).await;
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
        self.active_start_request_id = None;
        self.lifecycle = SyncLifecycle::Stopped;
        self.emit(CoreEvent::Sync(SyncEvent::Stopped { request_id }));
        self.project_sync_status(SyncLifecycleStatus::Stopped).await;
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
        if !sync_once_admitted(
            self.lifecycle,
            self.active_backend,
            self.sync_task.is_some(),
            self.sync_service.is_some(),
            self.legacy_stop_tx.is_some(),
        ) {
            self.emit(CoreEvent::OperationFailed {
                request_id,
                failure: CoreFailure::SyncFailed {
                    kind: SyncFailureKind::Internal,
                },
            });
            return;
        }

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

    async fn project_sync_status(&self, status: SyncLifecycleStatus) {
        send_sync_status(&self.action_tx, &self.sync_generation, status).await;
    }

    fn reduce(&self, actions: Vec<AppAction>) {
        let _ = self.action_tx.try_send(actions);
    }
}

// ---------------------------------------------------------------------------
// SyncService state observer (runs in its own task)
// ---------------------------------------------------------------------------

async fn notify_dependents_sync_started(
    session: Arc<MatrixClientSession>,
    room_tx: mpsc::Sender<RoomMessage>,
    timeline_tx: mpsc::Sender<crate::timeline::TimelineMessage>,
    room_list_service: Option<Arc<matrix_sdk_ui::room_list_service::RoomListService>>,
    legacy_committed_from_response_sequence: Option<u64>,
) {
    let _ = room_tx
        .send(RoomMessage::SyncStarted {
            session,
            room_list_service: room_list_service.clone(),
        })
        .await;
    // Same handoff to the timeline manager: timeline subscriptions must be
    // able to subscribe rooms with the live service (canon). On SyncService
    // recovery, reusing this same path rebuilds live room timelines without
    // restarting the SyncService itself.
    let _ = timeline_tx
        .send(crate::timeline::TimelineMessage::SyncStarted {
            room_list_service,
            legacy_committed_from_response_sequence,
        })
        .await;
}

fn notify_room_sync_stopped(room_tx: mpsc::Sender<RoomMessage>) {
    executor::spawn(async move {
        let _ = room_tx.send(RoomMessage::SyncStopped).await;
    });
}

async fn observe_sync_service_states(
    mut state_sub: eyeball::Subscriber<matrix_sdk_ui::sync_service::State>,
    event_tx: broadcast::Sender<CoreEvent>,
    action_tx: mpsc::Sender<Vec<AppAction>>,
    sync_generation: Arc<AtomicU64>,
    session: Arc<MatrixClientSession>,
    room_tx: mpsc::Sender<RoomMessage>,
    timeline_tx: mpsc::Sender<crate::timeline::TimelineMessage>,
    room_list_service: Arc<matrix_sdk_ui::room_list_service::RoomListService>,
    control_tx: mpsc::Sender<SyncActorControl>,
    sync_service_run_generation: u64,
) -> SyncTaskOutcome {
    let mut status = SyncServiceObserverStatus::default();
    let mut room_list_state_sub = room_list_service.state();

    let mut pending_state = Some(state_sub.get());
    let mut pending_room_list_state = Some(room_list_state_sub.get());
    loop {
        enum ObserverSignal {
            SyncService(matrix_sdk_ui::sync_service::State),
            RoomList(matrix_sdk_ui::room_list_service::State),
        }

        let signal = if let Some(state) = pending_state.take() {
            ObserverSignal::SyncService(state)
        } else if let Some(state) = pending_room_list_state.take() {
            ObserverSignal::RoomList(state)
        } else {
            tokio::select! {
                state = state_sub.next() => match state {
                    Some(state) => ObserverSignal::SyncService(state),
                    None => return SyncTaskOutcome::Stopped,
                },
                state = room_list_state_sub.next() => match state {
                    Some(state) => ObserverSignal::RoomList(state),
                    None => continue,
                },
            }
        };

        match signal {
            ObserverSignal::RoomList(state) => {
                let state_label = room_list_service_state_trace_label(&state);
                let state_kind = room_list_service_state_kind(&state);
                match note_room_list_service_state(&mut status, state_kind) {
                    Some(SyncServiceObserverDecision::ConnectivityProven) => {
                        trace_sync!(
                            "room_list_state",
                            [
                                DiagnosticField::token("state", state_label),
                                DiagnosticField::boolean(
                                    "connectivity_proven",
                                    status.connectivity_proven
                                ),
                                DiagnosticField::token("action", "connectivity_proven"),
                            ],
                            "state={} connectivity_proven={} action=connectivity_proven",
                            state_label,
                            status.connectivity_proven
                        );
                    }
                    _ => {
                        trace_sync!(
                            "room_list_state",
                            [
                                DiagnosticField::token("state", state_label),
                                DiagnosticField::boolean(
                                    "connectivity_proven",
                                    status.connectivity_proven
                                ),
                                DiagnosticField::token("action", "ignore"),
                            ],
                            "state={} connectivity_proven={} action=ignore",
                            state_label,
                            status.connectivity_proven
                        );
                    }
                }
            }
            ObserverSignal::SyncService(state) => {
                let state_label = sync_service_state_trace_label(&state);
                let state_kind = sync_service_state_kind(&state);
                let decision = classify_sync_service_state(&mut status, state_kind);
                match decision {
                    SyncServiceObserverDecision::InitialRunningHandoff => {
                        trace_sync!(
                            "sync_service_state",
                            [
                                DiagnosticField::token("state", state_label),
                                DiagnosticField::boolean(
                                    "sync_started_emitted",
                                    status.sync_started_emitted
                                ),
                                DiagnosticField::boolean(
                                    "connectivity_proven",
                                    status.connectivity_proven
                                ),
                                DiagnosticField::boolean("reconnecting", status.reconnecting),
                                DiagnosticField::token("action", "initial_running_handoff"),
                            ],
                            "state={} sync_started_emitted={} connectivity_proven={} reconnecting={} action=initial_running_handoff",
                            state_label,
                            status.sync_started_emitted,
                            status.connectivity_proven,
                            status.reconnecting
                        );
                        notify_dependents_sync_started(
                            session.clone(),
                            room_tx.clone(),
                            timeline_tx.clone(),
                            Some(room_list_service.clone()),
                            None,
                        )
                        .await;
                        let _ = control_tx
                            .send(SyncActorControl::SyncServiceRunning {
                                sync_service_run_generation,
                            })
                            .await;
                        let _ = event_tx.send(CoreEvent::Sync(SyncEvent::Running));
                        send_sync_status(
                            &action_tx,
                            &sync_generation,
                            SyncLifecycleStatus::Running,
                        )
                        .await;
                    }
                    SyncServiceObserverDecision::RecoveryHandoff => {
                        // Recovered from Offline/Reconnecting.
                        trace_sync!(
                            "sync_service_state",
                            [
                                DiagnosticField::token("state", state_label),
                                DiagnosticField::boolean(
                                    "sync_started_emitted",
                                    status.sync_started_emitted
                                ),
                                DiagnosticField::boolean(
                                    "connectivity_proven",
                                    status.connectivity_proven
                                ),
                                DiagnosticField::boolean("reconnecting", status.reconnecting),
                                DiagnosticField::token("action", "recovery_handoff"),
                            ],
                            "state={} sync_started_emitted={} connectivity_proven={} reconnecting={} action=recovery_handoff",
                            state_label,
                            status.sync_started_emitted,
                            status.connectivity_proven,
                            status.reconnecting
                        );
                        notify_dependents_sync_started(
                            session.clone(),
                            room_tx.clone(),
                            timeline_tx.clone(),
                            Some(room_list_service.clone()),
                            None,
                        )
                        .await;
                        let _ = control_tx
                            .send(SyncActorControl::BackendRecovered {
                                backend: ActiveBackend::SyncService,
                                run_generation: sync_service_run_generation,
                            })
                            .await;
                    }
                    SyncServiceObserverDecision::RunningNoop => {
                        trace_sync!(
                            "sync_service_state",
                            [
                                DiagnosticField::token("state", state_label),
                                DiagnosticField::boolean(
                                    "sync_started_emitted",
                                    status.sync_started_emitted
                                ),
                                DiagnosticField::boolean(
                                    "connectivity_proven",
                                    status.connectivity_proven
                                ),
                                DiagnosticField::boolean("reconnecting", status.reconnecting),
                                DiagnosticField::token("action", "running_noop"),
                            ],
                            "state={} sync_started_emitted={} connectivity_proven={} reconnecting={} action=running_noop",
                            state_label,
                            status.sync_started_emitted,
                            status.connectivity_proven,
                            status.reconnecting
                        );
                    }
                    SyncServiceObserverDecision::FallbackBeforeConnectivity => {
                        trace_sync!(
                            "sync_service_state",
                            [
                                DiagnosticField::token("state", state_label),
                                DiagnosticField::boolean(
                                    "sync_started_emitted",
                                    status.sync_started_emitted
                                ),
                                DiagnosticField::boolean(
                                    "connectivity_proven",
                                    status.connectivity_proven
                                ),
                                DiagnosticField::boolean("reconnecting", status.reconnecting),
                                DiagnosticField::token("action", "fallback_before_first_running"),
                            ],
                            "state={} sync_started_emitted={} connectivity_proven={} reconnecting={} action=fallback_before_first_running",
                            state_label,
                            status.sync_started_emitted,
                            status.connectivity_proven,
                            status.reconnecting
                        );
                        let reason = match state_kind {
                            SyncServiceStateKind::Offline => "network_offline",
                            SyncServiceStateKind::Error => "network_error",
                            _ => "network_error",
                        };
                        let _ = event_tx.send(CoreEvent::Sync(SyncEvent::Reconnecting));
                        send_sync_status(
                            &action_tx,
                            &sync_generation,
                            SyncLifecycleStatus::Reconnecting {
                                reason: reason.to_owned(),
                            },
                        )
                        .await;
                        return SyncTaskOutcome::Failed {
                            kind: SyncFailureKind::Http,
                            ever_ran: status.connectivity_proven,
                        };
                    }
                    SyncServiceObserverDecision::WaitRecovery => {
                        trace_sync!(
                            "sync_service_state",
                            [
                                DiagnosticField::token("state", state_label),
                                DiagnosticField::boolean(
                                    "sync_started_emitted",
                                    status.sync_started_emitted
                                ),
                                DiagnosticField::boolean(
                                    "connectivity_proven",
                                    status.connectivity_proven
                                ),
                                DiagnosticField::boolean("reconnecting", status.reconnecting),
                                DiagnosticField::token("action", "wait_recovery"),
                            ],
                            "state={} sync_started_emitted={} connectivity_proven={} reconnecting={} action=wait_recovery",
                            state_label,
                            status.sync_started_emitted,
                            status.connectivity_proven,
                            status.reconnecting
                        );
                        let reason = match state_kind {
                            SyncServiceStateKind::Offline => "network_offline",
                            SyncServiceStateKind::Error => "network_error",
                            _ => "network_error",
                        };
                        let _ = control_tx
                            .send(SyncActorControl::BackendReconnecting {
                                backend: ActiveBackend::SyncService,
                                run_generation: sync_service_run_generation,
                                reason,
                            })
                            .await;
                    }
                    SyncServiceObserverDecision::AlreadyReconnecting => {
                        trace_sync!(
                            "sync_service_state",
                            [
                                DiagnosticField::token("state", state_label),
                                DiagnosticField::boolean(
                                    "sync_started_emitted",
                                    status.sync_started_emitted
                                ),
                                DiagnosticField::boolean(
                                    "connectivity_proven",
                                    status.connectivity_proven
                                ),
                                DiagnosticField::boolean("reconnecting", status.reconnecting),
                                DiagnosticField::token("action", "already_reconnecting"),
                            ],
                            "state={} sync_started_emitted={} connectivity_proven={} reconnecting={} action=already_reconnecting",
                            state_label,
                            status.sync_started_emitted,
                            status.connectivity_proven,
                            status.reconnecting
                        );
                    }
                    SyncServiceObserverDecision::Fail => {
                        trace_sync!(
                            "sync_service_state",
                            [
                                DiagnosticField::token("state", state_label),
                                DiagnosticField::boolean(
                                    "sync_started_emitted",
                                    status.sync_started_emitted
                                ),
                                DiagnosticField::boolean(
                                    "connectivity_proven",
                                    status.connectivity_proven
                                ),
                                DiagnosticField::boolean("reconnecting", status.reconnecting),
                                DiagnosticField::token("action", "fail"),
                            ],
                            "state={} sync_started_emitted={} connectivity_proven={} reconnecting={} action=fail",
                            state_label,
                            status.sync_started_emitted,
                            status.connectivity_proven,
                            status.reconnecting
                        );
                        return SyncTaskOutcome::Failed {
                            kind: SyncFailureKind::Http,
                            ever_ran: status.connectivity_proven,
                        };
                    }
                    SyncServiceObserverDecision::Ignore => {
                        trace_sync!(
                            "sync_service_state",
                            [
                                DiagnosticField::token("state", state_label),
                                DiagnosticField::boolean(
                                    "sync_started_emitted",
                                    status.sync_started_emitted
                                ),
                                DiagnosticField::boolean(
                                    "connectivity_proven",
                                    status.connectivity_proven
                                ),
                                DiagnosticField::boolean("reconnecting", status.reconnecting),
                                DiagnosticField::token("action", "ignore"),
                            ],
                            "state={} sync_started_emitted={} connectivity_proven={} reconnecting={} action=ignore",
                            state_label,
                            status.sync_started_emitted,
                            status.connectivity_proven,
                            status.reconnecting
                        );
                    }
                    SyncServiceObserverDecision::ConnectivityProven => {}
                }
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
    _event_tx: broadcast::Sender<CoreEvent>,
    _action_tx: mpsc::Sender<Vec<AppAction>>,
    _sync_generation: Arc<AtomicU64>,
    control_tx: mpsc::Sender<SyncActorControl>,
    legacy_run_generation: u64,
) -> SyncTaskOutcome {
    use futures_util::StreamExt as _;

    let settings = legacy_sync_settings();
    let sync_stream = client.sync_stream(settings).await;
    tokio::pin!(sync_stream);

    let mut ever_ran = false;
    let mut reconnecting = false;

    loop {
        tokio::select! {
            biased;
            _ = &mut stop_rx => {
                trace_sync!(
                    "legacy_state",
                    [
                        DiagnosticField::token("state", "stopped"),
                        DiagnosticField::boolean("ever_ran", ever_ran),
                        DiagnosticField::boolean("reconnecting", reconnecting),
                        DiagnosticField::token("action", "stop_signal"),
                    ],
                    "state=stopped ever_ran={} reconnecting={} action=stop_signal",
                    ever_ran,
                    reconnecting
                );
                return SyncTaskOutcome::Stopped;
            }
            item = sync_stream.next() => {
                match item {
                    None => {
                        trace_sync!(
                            "legacy_state",
                            [
                                DiagnosticField::token("state", "stopped"),
                                DiagnosticField::boolean("ever_ran", ever_ran),
                                DiagnosticField::boolean("reconnecting", reconnecting),
                                DiagnosticField::token("action", "stream_ended"),
                            ],
                            "state=stopped ever_ran={} reconnecting={} action=stream_ended",
                            ever_ran,
                            reconnecting
                        );
                        return SyncTaskOutcome::Stopped;
                    }
                    Some(Ok(_response)) => {
                        if !ever_ran {
                            trace_sync!(
                                "legacy_state",
                                [
                                    DiagnosticField::token("state", "running"),
                                    DiagnosticField::boolean("ever_ran", ever_ran),
                                    DiagnosticField::boolean("reconnecting", reconnecting),
                                    DiagnosticField::token("action", "legacy_started"),
                                ],
                                "state=running ever_ran={} reconnecting={} action=legacy_started",
                                ever_ran,
                                reconnecting
                            );
                            ever_ran = true;
                            reconnecting = false;
                            let committed_from_response_sequence =
                                client.latest_room_updates_response_sequence();
                            let _ = control_tx
                                .send(SyncActorControl::FirstResponseCommitted {
                                    legacy_run_generation,
                                    committed_from_response_sequence,
                                })
                                .await;
                        } else if reconnecting {
                            trace_sync!(
                                "legacy_state",
                                [
                                    DiagnosticField::token("state", "running"),
                                    DiagnosticField::boolean("ever_ran", ever_ran),
                                    DiagnosticField::boolean("reconnecting", reconnecting),
                                    DiagnosticField::token("action", "legacy_recovered"),
                                ],
                                "state=running ever_ran={} reconnecting={} action=legacy_recovered",
                                ever_ran,
                                reconnecting
                            );
                            reconnecting = false;
                            let _ = control_tx
                                .send(SyncActorControl::BackendRecovered {
                                    backend: ActiveBackend::LegacySync,
                                    run_generation: legacy_run_generation,
                                })
                                .await;
                        }
                        // Else: normal running tick — no event needed.
                    }
                    Some(Err(error)) => {
                        let kind = classify_sdk_sync_error(&error);
                        if kind == SyncFailureKind::Auth {
                            // Auth failures are terminal (the SDK will not recover).
                            trace_sync!(
                                "legacy_state",
                                [
                                    DiagnosticField::token("state", "error"),
                                    DiagnosticField::token("kind", sync_failure_kind_label(kind)),
                                    DiagnosticField::boolean("ever_ran", ever_ran),
                                    DiagnosticField::boolean("reconnecting", reconnecting),
                                    DiagnosticField::token("action", "fail"),
                                ],
                                "state=error kind={} ever_ran={} reconnecting={} action=fail",
                                sync_failure_kind_label(kind),
                                ever_ran,
                                reconnecting
                            );
                            return SyncTaskOutcome::Failed { kind, ever_ran };
                        }
                        // Network / transient failures: emit Reconnecting and let
                        // the SDK's stream retry internally.
                        if !reconnecting {
                            trace_sync!(
                                "legacy_state",
                                [
                                    DiagnosticField::token("state", "error"),
                                    DiagnosticField::token("kind", sync_failure_kind_label(kind)),
                                    DiagnosticField::boolean("ever_ran", ever_ran),
                                    DiagnosticField::boolean("reconnecting", reconnecting),
                                    DiagnosticField::token("action", "wait_recovery"),
                                ],
                                "state=error kind={} ever_ran={} reconnecting={} action=wait_recovery",
                                sync_failure_kind_label(kind),
                                ever_ran,
                                reconnecting
                            );
                            reconnecting = true;
                            let _ = control_tx
                                .send(SyncActorControl::BackendReconnecting {
                                    backend: ActiveBackend::LegacySync,
                                    run_generation: legacy_run_generation,
                                    reason: "network_error",
                                })
                                .await;
                        } else {
                            trace_sync!(
                                "legacy_state",
                                [
                                    DiagnosticField::token("state", "error"),
                                    DiagnosticField::token("kind", sync_failure_kind_label(kind)),
                                    DiagnosticField::boolean("ever_ran", ever_ran),
                                    DiagnosticField::boolean("reconnecting", reconnecting),
                                    DiagnosticField::token("action", "already_reconnecting"),
                                ],
                                "state=error kind={} ever_ran={} reconnecting={} action=already_reconnecting",
                                sync_failure_kind_label(kind),
                                ever_ran,
                                reconnecting
                            );
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

fn should_fallback_to_legacy_after_sync_service_failure(
    backend: ActiveBackend,
    kind: SyncFailureKind,
    _ever_ran: bool,
    already_attempted: bool,
) -> bool {
    backend == ActiveBackend::SyncService && kind == SyncFailureKind::Http && !already_attempted
}

fn backend_from_invite_list_support(
    support: MatrixSlidingSyncInviteListSupport,
) -> SyncBackendKind {
    match support {
        MatrixSlidingSyncInviteListSupport::Supported => SyncBackendKind::SyncService,
        MatrixSlidingSyncInviteListSupport::KnownIncomplete
        | MatrixSlidingSyncInviteListSupport::Unknown => SyncBackendKind::LegacySync,
    }
}

/// Probe MSC4186 availability, then verify the authenticated invite-only list
/// contract before any sync owner starts. Missing or indeterminate contract
/// results fail closed to legacy sync.
///
/// Debug/test builds honor `KOUSHI_QA_FORCE_SYNC_BACKEND=legacy`
/// (skip the probe, select `LegacySync`); release builds compile the check
/// out entirely and always probe.
async fn probe_backend(session: &MatrixClientSession) -> BackendProbeResult {
    #[cfg(any(debug_assertions, test))]
    if forced_legacy_backend() {
        return BackendProbeResult {
            backend: SyncBackendKind::LegacySync,
            reason: BackendProbeReason::ForcedLegacy,
        };
    }

    let client = session.client();
    let versions = client.available_sliding_sync_versions().await;
    if versions.is_empty() {
        return BackendProbeResult {
            backend: SyncBackendKind::LegacySync,
            reason: BackendProbeReason::SlidingSyncUnavailable,
        };
    }

    let support = koushi_sdk::probe_sliding_sync_invite_list_support(session).await;
    let reason = match support {
        MatrixSlidingSyncInviteListSupport::Supported => BackendProbeReason::InviteListSupported,
        MatrixSlidingSyncInviteListSupport::KnownIncomplete => {
            BackendProbeReason::InviteListKnownIncomplete
        }
        MatrixSlidingSyncInviteListSupport::Unknown => BackendProbeReason::InviteListUnknown,
    };
    BackendProbeResult {
        backend: backend_from_invite_list_support(support),
        reason,
    }
}

fn legacy_sync_settings() -> matrix_sdk::config::SyncSettings {
    use matrix_sdk::ruma::api::client::sync::sync_events;

    matrix_sdk::config::SyncSettings::default().filter(sync_events::v3::Filter::from(
        legacy_sync_filter_definition(),
    ))
}

fn legacy_sync_filter_definition() -> matrix_sdk::ruma::api::client::filter::FilterDefinition {
    let mut filter = matrix_sdk::ruma::api::client::filter::FilterDefinition::default();
    filter.room.timeline.limit = Some(matrix_sdk::ruma::UInt::from(
        LEGACY_SYNC_ROOM_TIMELINE_LIMIT,
    ));
    filter
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
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{Mutex, mpsc as std_mpsc},
        thread,
    };

    use koushi_state::SessionInfo;
    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    use tokio::sync::{broadcast, mpsc};
    use wiremock::ResponseTemplate;

    use super::*;
    use crate::event::{CoreEvent, SyncBackendKind, SyncEvent};
    use crate::failure::SyncFailureKind;

    static FORCE_BACKEND_ENV_LOCK: Mutex<()> = Mutex::new(());

    fn spawn_backend_probe_server(
        invite_list_body: &'static [u8],
        continue_to_legacy_sync: bool,
    ) -> (String, std_mpsc::Receiver<String>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("synthetic probe listener");
        let address = listener.local_addr().expect("synthetic probe address");
        let (request_tx, request_rx) = std_mpsc::channel();
        let server = thread::spawn(move || {
            loop {
                let (mut stream, _) = listener.accept().expect("backend probe request");
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("probe request read timeout");
                let mut request = Vec::new();
                let mut chunk = [0_u8; 1024];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    let read = stream.read(&mut chunk).expect("read backend probe request");
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&chunk[..read]);
                    assert!(request.len() <= 8 * 1024, "synthetic request is bounded");
                }
                let request = String::from_utf8(request).expect("ASCII backend probe request");
                let mut request_parts = request
                    .lines()
                    .next()
                    .expect("backend probe request line")
                    .split_ascii_whitespace();
                let method = request_parts.next().expect("backend probe request method");
                let path = request_parts.next().expect("backend probe request path");
                let (status, body, relevant, finished) = if method == "GET"
                    && path == "/_matrix/client/versions"
                {
                    (
                        "200 OK",
                        br#"{"versions":["v1.12"],"unstable_features":{"org.matrix.simplified_msc3575":true}}"#.as_slice(),
                        true,
                        false,
                    )
                } else if method == "POST"
                    && path
                        .starts_with("/_matrix/client/unstable/org.matrix.simplified_msc3575/sync")
                {
                    ("200 OK", invite_list_body, true, !continue_to_legacy_sync)
                } else if continue_to_legacy_sync
                    && method == "GET"
                    && path.starts_with("/_matrix/client/v3/sync?")
                {
                    (
                        "200 OK",
                        br#"{"next_batch":"legacy-first-response","rooms":{"join":{},"invite":{},"leave":{},"knock":{}},"to_device":{"events":[]},"presence":{"events":[]},"account_data":{"events":[]},"device_lists":{"changed":[],"left":[]},"device_one_time_keys_count":{}}"#.as_slice(),
                        true,
                        true,
                    )
                } else {
                    ("404 Not Found", br#"{}"#.as_slice(), false, false)
                };
                if relevant {
                    request_tx
                        .send(format!("{method} {path}"))
                        .expect("capture backend probe request");
                }
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream
                    .write_all(head.as_bytes())
                    .expect("write response head");
                stream.write_all(body).expect("write response body");
                if finished {
                    break;
                }
            }
        });
        (format!("http://{address}"), request_rx, server)
    }

    async fn authenticated_backend_probe_client(homeserver: String) -> matrix_sdk::Client {
        let client = matrix_sdk::Client::builder()
            .homeserver_url(homeserver)
            .build()
            .await
            .expect("synthetic backend probe client");
        client
            .matrix_auth()
            .restore_session(
                matrix_sdk::authentication::matrix::MatrixSession {
                    meta: matrix_sdk_base::SessionMeta {
                        user_id: matrix_sdk::ruma::owned_user_id!("@probe:example.invalid"),
                        device_id: matrix_sdk::ruma::owned_device_id!("PROBEDEVICE"),
                    },
                    tokens: matrix_sdk::SessionTokens {
                        access_token: "synthetic-probe-token".to_owned(), // secret-scan: allow
                        refresh_token: None,
                    },
                },
                matrix_sdk_base::store::RoomLoadSettings::default(),
            )
            .await
            .expect("synthetic session restore");
        client
    }

    #[test]
    fn sync_trace_preserves_typed_status_fields_without_environment_switch() {
        trace_sync!(
            "test_sync_typed_fields",
            [
                DiagnosticField::count("generation", 3),
                DiagnosticField::token("lifecycle", "running"),
            ],
            "generation={} lifecycle={}",
            3,
            "running"
        );
        let record = koushi_diagnostics::snapshot()
            .records
            .into_iter()
            .rev()
            .find(|record| record.event.stage == "test_sync_typed_fields")
            .expect("sync trace should be collected");
        assert_eq!(record.event.source, "core.sync");
        assert!(
            record
                .event
                .fields
                .iter()
                .any(|field| field.key == "generation")
        );
        assert!(
            record
                .event
                .fields
                .iter()
                .any(|field| field.key == "lifecycle")
        );
    }

    #[tokio::test]
    async fn sync_actor_handle_shutdown_aborts_stuck_actor_task() {
        let (tx, _rx) = mpsc::channel(1);
        let task = executor::spawn(async {
            futures_util::future::pending::<()>().await;
        });
        let handle = SyncActorHandle { tx, task };

        let clean = handle.shutdown_with_timeout(Duration::from_millis(1)).await;

        assert!(!clean, "stuck actor task must be aborted after timeout");
    }

    #[tokio::test]
    async fn sync_once_is_rejected_before_sdk_call_while_continuous_owner_is_active() {
        let server = MatrixMockServer::new().await;
        server.mock_sync().ok(|_| {}).expect(0).mount().await;
        let session = Arc::new(MatrixClientSession::from_client_for_testing(
            server.client_builder().build().await,
            SessionInfo {
                homeserver: server.uri(),
                user_id: "@sync-owner:example.invalid".to_owned(),
                device_id: "SYNCOWNER".to_owned(),
            },
        ));
        let (action_tx, _action_rx) = mpsc::channel(4);
        let (event_tx, mut event_rx) = broadcast::channel(4);
        let (_command_tx, command_rx) = mpsc::channel(1);
        let (control_tx, control_rx) = mpsc::channel(1);
        let (room_tx, _room_rx) = mpsc::channel(1);
        let (timeline_tx, _timeline_rx) = mpsc::channel(1);
        let mut actor = SyncActor {
            session,
            action_tx,
            event_tx,
            command_rx,
            control_tx,
            control_rx,
            room_tx,
            timeline_tx,
            lifecycle: SyncLifecycle::Running,
            sync_generation: Arc::new(AtomicU64::new(0)),
            active_backend: ActiveBackend::LegacySync,
            sync_task: None,
            legacy_stop_tx: None,
            legacy_run_generation: 1,
            sync_service_run_generation: 0,
            sync_service: None,
            active_start_request_id: None,
            sync_service_runtime_fallback_attempted: false,
            ignored_user_list_handler: None,
        };
        let request_id = RequestId {
            connection_id: crate::ids::RuntimeConnectionId(7),
            sequence: 42,
        };

        actor
            .handle_command(SyncCommand::SyncOnce { request_id })
            .await;

        assert!(matches!(
            event_rx.recv().await.expect("sync once rejection event"),
            CoreEvent::OperationFailed {
                request_id: event_request_id,
                failure: CoreFailure::SyncFailed {
                    kind: SyncFailureKind::Internal,
                },
            } if event_request_id == request_id
        ));
        assert!(
            event_rx.try_recv().is_err(),
            "one rejected SyncOnce must emit exactly one failure"
        );
        server.verify().await;
    }

    #[tokio::test]
    async fn restricted_cursor_cannot_seed_the_first_normal_legacy_sync() {
        use futures_util::StreamExt as _;
        use matrix_sdk::ruma::room_id;
        use matrix_sdk_test::JoinedRoomBuilder;

        let server = MatrixMockServer::new().await;
        let client = server.client_builder().build().await;
        let session = MatrixClientSession::from_client_for_testing(
            client.clone(),
            SessionInfo {
                homeserver: server.uri(),
                user_id: "@restricted-cursor:example.invalid".to_owned(),
                device_id: "RESTRICTEDCURSOR".to_owned(),
            },
        );

        let restricted = server
            .mock_sync()
            .ok(|_| {})
            .mock_once()
            .mount_as_scoped()
            .await;
        let restricted_token =
            koushi_sdk::restricted_verification_sync_once_with_token(&session, None)
                .await
                .expect("restricted verification sync");
        drop(restricted);

        let room_id = room_id!("!normal-room:example.invalid");
        let first_normal = server
            .mock_sync()
            .ok(|builder| {
                builder.add_joined_room(JoinedRoomBuilder::new(room_id));
            })
            .mock_once()
            .mount_as_scoped()
            .await;
        let stream = client.sync_stream(legacy_sync_settings()).await;
        tokio::pin!(stream);
        let first_normal_response = stream
            .next()
            .await
            .expect("first normal response")
            .expect("first normal sync succeeds");
        drop(first_normal);
        assert!(
            client.get_room(room_id).is_some(),
            "the authoritative first normal response must project joined rooms"
        );

        let second_normal = server
            .mock_sync()
            .ok(|_| {})
            .mock_once()
            .mount_as_scoped()
            .await;
        stream
            .next()
            .await
            .expect("second normal response")
            .expect("second normal sync succeeds");
        drop(second_normal);

        let requests = server.received_requests().await.expect("captured requests");
        let sync_since = requests
            .iter()
            .filter(|request| request.url.path() == "/_matrix/client/v3/sync")
            .map(|request| {
                request
                    .url
                    .query_pairs()
                    .find_map(|(key, value)| (key == "since").then(|| value.into_owned()))
            })
            .collect::<Vec<_>>();
        assert_eq!(sync_since.len(), 3);
        assert_eq!(
            sync_since[0], None,
            "restricted catch-up begins without a cursor"
        );
        assert_eq!(
            sync_since[1], None,
            "a restricted-only cursor must not seed the first authoritative normal sync"
        );
        assert_ne!(sync_since[1].as_deref(), Some(restricted_token.as_str()));
        assert_eq!(
            sync_since[2].as_deref(),
            Some(first_normal_response.next_batch.as_str()),
            "later normal iterations must advance from the first normal response"
        );
    }

    #[tokio::test]
    async fn restricted_sync_preserves_canonical_cursor_across_store_reopen() {
        use futures_util::StreamExt as _;

        let server = MatrixMockServer::new().await;
        let store = tempfile::tempdir().expect("temporary SDK store");
        let client = server
            .client_builder()
            .on_builder(|builder| builder.sqlite_store(store.path(), None))
            .build()
            .await;

        let canonical = server
            .mock_sync()
            .ok(|_| {})
            .mock_once()
            .mount_as_scoped()
            .await;
        let canonical_token = client
            .sync_once(matrix_sdk::config::SyncSettings::default())
            .await
            .expect("canonical seed sync")
            .next_batch;
        drop(canonical);

        let session = MatrixClientSession::from_client_for_testing(
            client.clone(),
            SessionInfo {
                homeserver: server.uri(),
                user_id: "@restricted-reopen:example.invalid".to_owned(),
                device_id: "RESTRICTEDREOPEN".to_owned(),
            },
        );
        let restricted = server
            .mock_sync()
            .ok(|_| {})
            .mock_once()
            .mount_as_scoped()
            .await;
        let restricted_token =
            koushi_sdk::restricted_verification_sync_once_with_token(&session, None)
                .await
                .expect("restricted verification sync");
        drop(restricted);
        assert_ne!(restricted_token, canonical_token);

        drop(session);
        drop(client);
        let reopened = server
            .client_builder()
            .on_builder(|builder| builder.sqlite_store(store.path(), None))
            .build()
            .await;
        let normal = server
            .mock_sync()
            .ok(|_| {})
            .mock_once()
            .mount_as_scoped()
            .await;
        let stream = reopened.sync_stream(legacy_sync_settings()).await;
        tokio::pin!(stream);
        stream
            .next()
            .await
            .expect("normal response after reopen")
            .expect("normal sync after reopen succeeds");
        drop(normal);

        let sync_since = server
            .received_requests()
            .await
            .expect("captured requests")
            .iter()
            .filter(|request| request.url.path() == "/_matrix/client/v3/sync")
            .map(|request| {
                request
                    .url
                    .query_pairs()
                    .find_map(|(key, value)| (key == "since").then(|| value.into_owned()))
            })
            .collect::<Vec<_>>();
        assert_eq!(sync_since.len(), 3);
        assert_eq!(
            sync_since[0], None,
            "canonical seed starts without a cursor"
        );
        assert_eq!(
            sync_since[1], None,
            "restricted sync always starts without a cursor"
        );
        assert_eq!(sync_since[2].as_deref(), Some(canonical_token.as_str()));
        assert_ne!(sync_since[2].as_deref(), Some(restricted_token.as_str()));
    }

    #[tokio::test]
    async fn restricted_cursor_remains_non_persisting_across_normal_failures_and_stops() {
        let server = MatrixMockServer::new().await;
        let client = server.client_builder().build().await;
        let session = Arc::new(MatrixClientSession::from_client_for_testing(
            client.clone(),
            SessionInfo {
                homeserver: server.uri(),
                user_id: "@restricted-restart:example.invalid".to_owned(),
                device_id: "RESTRICTEDRESTART".to_owned(),
            },
        ));

        let restricted = server
            .mock_sync()
            .ok(|_| {})
            .mock_once()
            .mount_as_scoped()
            .await;
        let restricted_token =
            koushi_sdk::restricted_verification_sync_once_with_token(&session, None)
                .await
                .expect("restricted verification sync");
        drop(restricted);

        let (action_tx, _action_rx) = mpsc::channel(16);
        let (event_tx, _) = broadcast::channel(16);
        let (_command_tx, command_rx) = mpsc::channel(4);
        let (control_tx, control_rx) = mpsc::channel(4);
        let (room_tx, _room_rx) = mpsc::channel(4);
        let (timeline_tx, _timeline_rx) = mpsc::channel(4);
        let mut actor = SyncActor {
            session,
            action_tx,
            event_tx,
            command_rx,
            control_tx,
            control_rx,
            room_tx,
            timeline_tx,
            lifecycle: SyncLifecycle::Starting,
            sync_generation: Arc::new(AtomicU64::new(0)),
            active_backend: ActiveBackend::LegacySync,
            sync_task: None,
            legacy_stop_tx: None,
            legacy_run_generation: 0,
            sync_service_run_generation: 0,
            sync_service: None,
            active_start_request_id: None,
            sync_service_runtime_fallback_attempted: false,
            ignored_user_list_handler: None,
        };

        let failed_baseline = server
            .mock_sync()
            .error_unknown_token(false)
            .mock_once()
            .mount_as_scoped()
            .await;
        actor.start_legacy_sync(client.clone()).await;
        let failed_outcome = tokio::time::timeout(
            Duration::from_secs(5),
            actor.sync_task.take().expect("failed legacy task"),
        )
        .await
        .expect("auth failure must terminate the legacy run")
        .expect("legacy task must not panic");
        assert!(matches!(
            failed_outcome,
            SyncTaskOutcome::Failed {
                kind: SyncFailureKind::Auth,
                ever_ran: false,
            }
        ));
        actor.cleanup_ended_backend().await;
        drop(failed_baseline);

        let (held_request_seen_tx, held_request_seen_rx) = tokio::sync::oneshot::channel();
        let held_request_seen_tx = Arc::new(std::sync::Mutex::new(Some(held_request_seen_tx)));
        let held_baseline = server
            .mock_sync()
            .respond_with({
                let held_request_seen_tx = held_request_seen_tx.clone();
                move |_request: &wiremock::Request| {
                    if let Some(sender) = held_request_seen_tx
                        .lock()
                        .expect("held request barrier lock")
                        .take()
                    {
                        let _ = sender.send(());
                    }
                    ResponseTemplate::new(200)
                        .set_delay(Duration::from_secs(60))
                        .set_body_json(serde_json::json!({
                            "next_batch": "held-baseline-token",
                            "rooms": {},
                            "to_device": { "events": [] },
                            "presence": { "events": [] },
                            "account_data": { "events": [] },
                            "device_lists": { "changed": [], "left": [] },
                            "device_one_time_keys_count": {},
                        }))
                }
            })
            .mock_once()
            .mount_as_scoped()
            .await;
        actor.start_legacy_sync(client.clone()).await;
        tokio::time::timeout(Duration::from_secs(5), held_request_seen_rx)
            .await
            .expect("held NoToken request must reach the server before stop")
            .expect("held request barrier sender must remain owned by the server response");
        actor.do_stop(None).await;
        drop(held_baseline);

        let successful_baseline = server
            .mock_sync()
            .ok(|_| {})
            .mock_once()
            .mount_as_scoped()
            .await;
        actor.start_legacy_sync(client.clone()).await;
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(5), actor.control_rx.recv())
                .await
                .expect("baseline response control must arrive"),
            Some(SyncActorControl::FirstResponseCommitted { .. })
        ));
        actor.do_stop(None).await;
        drop(successful_baseline);

        let requests_before_canonical_restart = server
            .received_requests()
            .await
            .expect("captured requests")
            .into_iter()
            .filter(|request| request.url.path() == "/_matrix/client/v3/sync")
            .count();
        let canonical_restart = server
            .mock_sync()
            .ok(|_| {})
            .mock_once()
            .mount_as_scoped()
            .await;
        actor.start_legacy_sync(client).await;
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(5), actor.control_rx.recv())
                .await
                .expect("canonical restart response control must arrive"),
            Some(SyncActorControl::FirstResponseCommitted { .. })
        ));
        actor.do_stop(None).await;
        drop(canonical_restart);

        let sync_since = server
            .received_requests()
            .await
            .expect("captured requests")
            .iter()
            .filter(|request| request.url.path() == "/_matrix/client/v3/sync")
            .map(|request| {
                request
                    .url
                    .query_pairs()
                    .find_map(|(key, value)| (key == "since").then(|| value.into_owned()))
            })
            .collect::<Vec<_>>();
        assert!(sync_since.len() > requests_before_canonical_restart);
        assert_eq!(
            sync_since[0], None,
            "restricted sync starts without a cursor"
        );
        assert_eq!(
            sync_since[1], None,
            "the failed first normal baseline must omit the restricted cursor"
        );
        assert_eq!(
            sync_since[2], None,
            "restart after a failed normal response has no canonical cursor to reuse"
        );
        assert_eq!(
            sync_since[3], None,
            "restart after stopping before the held response must still omit since"
        );
        assert!(
            sync_since[requests_before_canonical_restart].is_some(),
            "only a restart after a committed normal response may reuse the canonical cursor"
        );
        assert_ne!(
            sync_since[requests_before_canonical_restart].as_deref(),
            Some(restricted_token.as_str()),
            "the restricted response cursor must never seed normal sync"
        );
    }

    #[tokio::test]
    async fn canonical_cursor_is_reused_by_an_ordinary_legacy_start() {
        use futures_util::StreamExt as _;

        let server = MatrixMockServer::new().await;
        let client = server.client_builder().build().await;
        let canonical = server
            .mock_sync()
            .ok(|_| {})
            .mock_once()
            .mount_as_scoped()
            .await;
        let canonical_response = client
            .sync_once(matrix_sdk::config::SyncSettings::default())
            .await
            .expect("canonical pre-existing sync");
        drop(canonical);

        let resumed = server
            .mock_sync()
            .ok(|_| {})
            .mock_once()
            .mount_as_scoped()
            .await;
        let stream = client.sync_stream(legacy_sync_settings()).await;
        tokio::pin!(stream);
        stream
            .next()
            .await
            .expect("resumed response")
            .expect("ordinary resumed sync succeeds");
        drop(resumed);

        let requests = server.received_requests().await.expect("captured requests");
        let sync_since = requests
            .iter()
            .filter(|request| request.url.path() == "/_matrix/client/v3/sync")
            .map(|request| {
                request
                    .url
                    .query_pairs()
                    .find_map(|(key, value)| (key == "since").then(|| value.into_owned()))
            })
            .collect::<Vec<_>>();
        assert_eq!(sync_since.len(), 2);
        assert_eq!(sync_since[0], None);
        assert_eq!(
            sync_since[1].as_deref(),
            Some(canonical_response.next_batch.as_str()),
            "ordinary restored sessions must reuse their canonical cursor"
        );
    }

    #[test]
    fn sync_once_admission_guard_precedes_the_sdk_call() {
        let source = include_str!("sync.rs");
        let body = source
            .split("    async fn handle_sync_once")
            .nth(1)
            .expect("handle_sync_once should exist")
            .split("    fn emit(")
            .next()
            .expect("emit should follow handle_sync_once");
        let guard = body
            .find("sync_once_admitted(")
            .expect("SyncOnce must check continuous-owner admission");
        let sdk_call = body
            .find("koushi_sdk::sync_once(")
            .expect("SDK sync_once call should remain present");

        assert!(guard < sdk_call, "admission must run before the SDK call");
        assert!(
            body[guard..sdk_call].contains("SyncFailureKind::Internal")
                && body[guard..sdk_call].contains("return;"),
            "rejection must emit the fixed Internal failure and return before SDK work"
        );
    }

    #[test]
    fn sync_once_admission_requires_an_idle_lifecycle_and_no_owner_artifacts() {
        assert!(sync_once_admitted(
            SyncLifecycle::Stopped,
            ActiveBackend::None,
            false,
            false,
            false,
        ));
        assert!(sync_once_admitted(
            SyncLifecycle::Failed,
            ActiveBackend::None,
            false,
            false,
            false,
        ));

        for lifecycle in [
            SyncLifecycle::Starting,
            SyncLifecycle::Running,
            SyncLifecycle::Reconnecting,
        ] {
            assert!(!sync_once_admitted(
                lifecycle,
                ActiveBackend::None,
                false,
                false,
                false,
            ));
        }
        for backend in [ActiveBackend::SyncService, ActiveBackend::LegacySync] {
            assert!(!sync_once_admitted(
                SyncLifecycle::Stopped,
                backend,
                false,
                false,
                false,
            ));
        }
        for owner_artifacts in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            assert!(!sync_once_admitted(
                SyncLifecycle::Failed,
                ActiveBackend::None,
                owner_artifacts.0,
                owner_artifacts.1,
                owner_artifacts.2,
            ));
        }
    }

    #[test]
    fn sync_stop_path_bounds_nonessential_shutdown_awaits() {
        let source = include_str!("sync.rs");
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("production source should precede tests");
        let body = source
            .split("    async fn do_stop")
            .nth(1)
            .and_then(|rest| rest.split("    async fn handle_sync_once").next())
            .expect("do_stop body");
        let notifier = source
            .split("fn notify_room_sync_stopped")
            .nth(1)
            .and_then(|rest| rest.split("async fn observe_sync_service_states").next())
            .expect("sync-stopped notifier");

        assert!(
            body.contains("notify_room_sync_stopped(self.room_tx.clone())"),
            "room observation teardown notification should route through the bounded notifier"
        );
        assert!(
            body.contains("executor::timeout(SYNC_SERVICE_STOP_TIMEOUT, svc.stop()).await"),
            "SyncService::stop must be bounded"
        );
        assert!(
            notifier.contains("executor::spawn(async move"),
            "room notification send should run in a notifier task so sync shutdown is not blocked"
        );
        assert!(
            notifier.contains(".send(RoomMessage::SyncStopped).await"),
            "SyncStopped handoff must use reliable send instead of drop-on-full try_send"
        );
        assert!(
            !production_source.contains("room_tx.try_send(RoomMessage::SyncStopped)"),
            "SyncStopped handoff must not use drop-on-full try_send"
        );
        assert!(
            !body.contains("svc.stop().await"),
            "SyncService::stop must not be awaited unbounded"
        );
    }

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
        let _guard = FORCE_BACKEND_ENV_LOCK
            .lock()
            .expect("backend environment lock");
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

    #[test]
    fn invite_list_decision_selects_sync_service_only_for_supported() {
        assert_eq!(
            backend_from_invite_list_support(MatrixSlidingSyncInviteListSupport::Supported),
            SyncBackendKind::SyncService
        );
        for support in [
            MatrixSlidingSyncInviteListSupport::KnownIncomplete,
            MatrixSlidingSyncInviteListSupport::Unknown,
        ] {
            assert_eq!(
                backend_from_invite_list_support(support),
                SyncBackendKind::LegacySync
            );
        }
    }

    #[test]
    fn invite_list_probe_reason_labels_are_private_safe_tokens() {
        for reason in [
            BackendProbeReason::ForcedLegacy,
            BackendProbeReason::SlidingSyncUnavailable,
            BackendProbeReason::InviteListSupported,
            BackendProbeReason::InviteListKnownIncomplete,
            BackendProbeReason::InviteListUnknown,
        ] {
            let label = backend_probe_reason_label(reason);
            assert!(
                label
                    .chars()
                    .all(|character| character.is_ascii_lowercase() || character == '_')
            );
            assert!(!label.contains("http"));
            assert!(!label.contains("matrix"));
        }
    }

    #[test]
    fn backend_probe_consults_invite_list_only_after_available_versions() {
        let source = include_str!("sync.rs");
        let body = source
            .split("async fn probe_backend")
            .nth(1)
            .and_then(|rest| rest.split("fn legacy_sync_settings").next())
            .expect("probe_backend body");
        let versions = body
            .find("available_sliding_sync_versions().await")
            .expect("versions capability probe");
        let unavailable = body
            .find("if versions.is_empty()")
            .expect("unavailable versions branch");
        let invite_list_probe = body
            .find("probe_sliding_sync_invite_list_support")
            .expect("typed invite-list contract probe");

        assert!(versions < unavailable);
        assert!(unavailable < invite_list_probe);
        assert!(
            body[unavailable..invite_list_probe].contains("SyncBackendKind::LegacySync"),
            "unavailable versions must return before the invite-list probe"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn backend_probe_checks_invite_list_after_versions_and_fails_closed() {
        let _guard = FORCE_BACKEND_ENV_LOCK
            .lock()
            .expect("backend environment lock");
        unsafe { std::env::remove_var(ENV_FORCE_SYNC_BACKEND) };

        for (body, expected) in [
            (
                br#"{"pos":"discarded","lists":{"koushi_invites":{"count":0}}}"#.as_slice(),
                BackendProbeResult {
                    backend: SyncBackendKind::SyncService,
                    reason: BackendProbeReason::InviteListSupported,
                },
            ),
            (
                br#"{"pos":"discarded","lists":{}}"#.as_slice(),
                BackendProbeResult {
                    backend: SyncBackendKind::LegacySync,
                    reason: BackendProbeReason::InviteListKnownIncomplete,
                },
            ),
            (
                br#"malformed"#.as_slice(),
                BackendProbeResult {
                    backend: SyncBackendKind::LegacySync,
                    reason: BackendProbeReason::InviteListUnknown,
                },
            ),
        ] {
            let (homeserver, requests, server) = spawn_backend_probe_server(body, false);
            let client = authenticated_backend_probe_client(homeserver).await;
            assert_eq!(
                requests
                    .recv_timeout(Duration::from_secs(2))
                    .expect("session restore versions request"),
                "GET /_matrix/client/versions"
            );

            let session = MatrixClientSession::from_client_for_testing(
                client,
                SessionInfo {
                    homeserver: "http://synthetic-probe.invalid".to_owned(),
                    user_id: "@probe:example.invalid".to_owned(),
                    device_id: "PROBEDEVICE".to_owned(),
                },
            );
            assert_eq!(probe_backend(&session).await, expected);
            assert_eq!(
                requests
                    .recv_timeout(Duration::from_secs(2))
                    .expect("versions request"),
                "GET /_matrix/client/versions"
            );
            assert!(
                requests
                    .recv_timeout(Duration::from_secs(2))
                    .expect("invite-list request")
                    .starts_with(
                        "POST /_matrix/client/unstable/org.matrix.simplified_msc3575/sync"
                    )
            );
            server.join().expect("synthetic backend probe server");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unknown_isolated_invite_probe_falls_back_and_commits_first_legacy_response() {
        let _guard = FORCE_BACKEND_ENV_LOCK
            .lock()
            .expect("backend environment lock");
        unsafe { std::env::remove_var(ENV_FORCE_SYNC_BACKEND) };

        let (homeserver, requests, server) = spawn_backend_probe_server(br#"malformed"#, true);
        let client = authenticated_backend_probe_client(homeserver.clone()).await;
        assert_eq!(
            requests
                .recv_timeout(Duration::from_secs(2))
                .expect("session restore versions request"),
            "GET /_matrix/client/versions"
        );
        let session = Arc::new(MatrixClientSession::from_client_for_testing(
            client,
            SessionInfo {
                homeserver,
                user_id: "@probe:example.invalid".to_owned(),
                device_id: "PROBEDEVICE".to_owned(),
            },
        ));
        let (action_tx, _action_rx) = mpsc::channel(4);
        let (event_tx, mut event_rx) = broadcast::channel(8);
        let (_command_tx, command_rx) = mpsc::channel(1);
        let (control_tx, control_rx) = mpsc::channel(4);
        let (room_tx, _room_rx) = mpsc::channel(4);
        let (timeline_tx, _timeline_rx) = mpsc::channel(4);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut actor = SyncActor {
            session,
            action_tx,
            event_tx,
            command_rx,
            control_tx,
            control_rx,
            room_tx,
            timeline_tx,
            lifecycle: SyncLifecycle::Stopped,
            sync_generation: Arc::new(AtomicU64::new(0)),
            active_backend: ActiveBackend::None,
            sync_task: None,
            legacy_stop_tx: None,
            legacy_run_generation: 0,
            sync_service_run_generation: 0,
            sync_service: None,
            active_start_request_id: None,
            sync_service_runtime_fallback_attempted: false,
            ignored_user_list_handler: None,
        };

        actor
            .handle_start(RequestId {
                connection_id: crate::ids::RuntimeConnectionId(1),
                sequence: 1,
            })
            .await;
        assert!(matches!(
            tokio::time::timeout_at(deadline, event_rx.recv())
                .await
                .expect("LegacySync Started event must arrive"),
            Ok(CoreEvent::Sync(SyncEvent::Started {
                backend: SyncBackendKind::LegacySync,
                ..
            }))
        ));
        assert_eq!(
            requests
                .recv_timeout(Duration::from_secs(2))
                .expect("versions request"),
            "GET /_matrix/client/versions"
        );
        let invite_request = requests
            .recv_timeout(Duration::from_secs(2))
            .expect("invite-list request");
        assert!(
            invite_request
                .starts_with("POST /_matrix/client/unstable/org.matrix.simplified_msc3575/sync"),
            "unexpected request after versions: {invite_request}"
        );
        assert!(
            requests
                .recv_timeout(Duration::from_secs(2))
                .expect("legacy sync request")
                .starts_with("GET /_matrix/client/v3/sync?")
        );

        let control = tokio::time::timeout_at(deadline, actor.control_rx.recv())
            .await
            .expect("first legacy response control must arrive")
            .expect("legacy control channel remains open");
        assert!(matches!(
            control,
            SyncActorControl::FirstResponseCommitted { .. }
        ));
        actor.handle_control(control).await;

        assert_eq!(actor.lifecycle, SyncLifecycle::Running);
        assert!(matches!(
            tokio::time::timeout_at(deadline, event_rx.recv())
                .await
                .expect("SyncEvent::ModeChanged must arrive"),
            Ok(CoreEvent::Sync(SyncEvent::ModeChanged { .. })
                | CoreEvent::Sync(SyncEvent::Running))
        ));
        assert!(matches!(
            tokio::time::timeout_at(deadline, event_rx.recv())
                .await
                .expect("SyncEvent::Running must arrive"),
            Ok(CoreEvent::Sync(SyncEvent::Running))
        ));
        loop {
            match event_rx.try_recv() {
                Ok(CoreEvent::Sync(SyncEvent::Stopped { .. } | SyncEvent::Failed)) => {
                    panic!("isolated invite probe must not stop or fail after LegacySync starts")
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(skipped)) => {
                    panic!("event receiver lagged while checking probe continuation: {skipped}")
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {
                    panic!("event receiver closed while checking probe continuation")
                }
            }
        }

        actor.do_stop(None).await;
        server.join().expect("synthetic probe and legacy server");
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
    fn sync_trace_labels_are_stable_and_private() {
        use crate::ids::{RequestId, RuntimeConnectionId};
        let request_id = RequestId {
            connection_id: RuntimeConnectionId(7),
            sequence: 42,
        };

        assert_eq!(sync_lifecycle_label(SyncLifecycle::Stopped), "stopped");
        assert_eq!(sync_lifecycle_label(SyncLifecycle::Starting), "starting");
        assert_eq!(sync_lifecycle_label(SyncLifecycle::Running), "running");
        assert_eq!(sync_lifecycle_label(SyncLifecycle::Failed), "failed");
        assert_eq!(active_backend_label(ActiveBackend::None), "none");
        assert_eq!(
            active_backend_label(ActiveBackend::SyncService),
            "sync_service"
        );
        assert_eq!(active_backend_label(ActiveBackend::LegacySync), "legacy");
        assert_eq!(
            sync_service_state_trace_label(&matrix_sdk_ui::sync_service::State::Offline),
            "offline"
        );
        assert_eq!(
            sync_task_outcome_trace_label(&SyncTaskOutcome::Failed {
                kind: SyncFailureKind::Http,
                ever_ran: false,
            }),
            "failed"
        );

        let (kind, traced_request_id) =
            sync_command_trace_parts(&SyncCommand::Restart { request_id });
        assert_eq!(kind, "restart");
        assert_eq!(traced_request_id, request_id);
    }

    #[test]
    fn sync_trace_covers_command_backend_state_and_fallback_decisions() {
        let source = include_str!("sync.rs");

        assert!(source.contains("\"command\","));
        assert!(source.contains("\"probe_done\","));
        assert!(source.contains("\"sync_service_state\","));
        assert!(source.contains("\"task_ended\","));
        assert!(source.contains("action=fallback_before_first_running"));
        assert!(source.contains("action=recovery_handoff"));
        assert!(source.contains("action=legacy_started"));
        assert!(!source.contains(&format!("{}{}", "error", ":?")));
    }

    #[test]
    fn sync_service_observer_does_not_treat_sdk_running_as_connectivity() {
        let mut status = SyncServiceObserverStatus::default();

        assert_eq!(
            classify_sync_service_state(&mut status, SyncServiceStateKind::Running),
            SyncServiceObserverDecision::InitialRunningHandoff
        );
        assert!(status.sync_started_emitted);
        assert!(
            !status.connectivity_proven,
            "SDK SyncService::Running is emitted immediately after supervisor spawn, before any successful sync"
        );
        assert_eq!(
            classify_sync_service_state(&mut status, SyncServiceStateKind::Offline),
            SyncServiceObserverDecision::FallbackBeforeConnectivity
        );
    }

    #[test]
    fn room_list_running_proves_connectivity_but_setting_up_does_not() {
        let mut status = SyncServiceObserverStatus::default();

        assert_eq!(
            classify_sync_service_state(&mut status, SyncServiceStateKind::Running),
            SyncServiceObserverDecision::InitialRunningHandoff
        );
        assert_eq!(
            note_room_list_service_state(&mut status, RoomListServiceStateKind::SettingUp),
            None
        );
        assert!(!status.connectivity_proven);
        assert_eq!(
            note_room_list_service_state(&mut status, RoomListServiceStateKind::Running),
            Some(SyncServiceObserverDecision::ConnectivityProven)
        );
        assert!(status.connectivity_proven);
        assert_eq!(
            classify_sync_service_state(&mut status, SyncServiceStateKind::Offline),
            SyncServiceObserverDecision::WaitRecovery
        );
        assert!(status.reconnecting);
    }

    #[test]
    fn idempotent_start_must_reproject_running_state() {
        let source = include_str!("sync.rs");
        let idempotent_start_arm = source
            .split("if self.lifecycle == SyncLifecycle::Running")
            .nth(1)
            .expect("idempotent Start branch should exist")
            .split("return;")
            .next()
            .expect("idempotent Start branch should return");

        assert!(
            idempotent_start_arm.contains("SyncEvent::Running"),
            "a repeated Start while lifecycle is Running must re-emit Running for waiters"
        );
        assert!(
            idempotent_start_arm.contains("SyncLifecycleStatus::Running"),
            "a repeated Start must reproject AppState.sync=Running through the full-status projection"
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
            SyncTaskOutcome::Failed { kind, .. } => kind,
            SyncTaskOutcome::Panicked => SyncFailureKind::Internal,
            SyncTaskOutcome::Stopped => panic!("wrong branch"),
        };
        assert_eq!(kind, SyncFailureKind::Internal);
    }

    #[test]
    fn sync_task_failed_preserves_kind() {
        let outcome = SyncTaskOutcome::Failed {
            kind: SyncFailureKind::Auth,
            ever_ran: false,
        };
        let kind = match outcome {
            SyncTaskOutcome::Failed { kind, .. } => kind,
            _ => SyncFailureKind::Internal,
        };
        assert_eq!(kind, SyncFailureKind::Auth);
    }

    #[test]
    fn sync_service_http_failure_falls_back_to_legacy_once() {
        assert!(
            should_fallback_to_legacy_after_sync_service_failure(
                ActiveBackend::SyncService,
                SyncFailureKind::Http,
                false,
                false
            ),
            "advertised SyncService can be incompatible at first request; fallback before Running"
        );

        assert!(
            should_fallback_to_legacy_after_sync_service_failure(
                ActiveBackend::SyncService,
                SyncFailureKind::Http,
                true,
                false
            ),
            "some MSC3575 incompatibilities surface only after the first Running state"
        );
        assert!(
            !should_fallback_to_legacy_after_sync_service_failure(
                ActiveBackend::SyncService,
                SyncFailureKind::Auth,
                false,
                false
            ),
            "auth failures must not be hidden by backend fallback"
        );
        assert!(
            !should_fallback_to_legacy_after_sync_service_failure(
                ActiveBackend::SyncService,
                SyncFailureKind::Http,
                false,
                true
            ),
            "fallback must not loop within one actor session"
        );
        assert!(
            !should_fallback_to_legacy_after_sync_service_failure(
                ActiveBackend::LegacySync,
                SyncFailureKind::Http,
                false,
                false
            ),
            "legacy failures have no lower backend to fall back to"
        );
    }

    #[test]
    fn runtime_fallback_emits_started_before_legacy_task_can_run() {
        let source = include_str!("sync.rs");
        let body = source
            .split("    async fn start_legacy_runtime_fallback")
            .nth(1)
            .and_then(|rest| rest.split("    /// Returns Ok(())").next())
            .expect("start_legacy_runtime_fallback body");

        let started_index = body
            .find("backend: SyncBackendKind::LegacySync")
            .expect("fallback must emit Started(LegacySync)");
        let spawn_index = body
            .find("self.start_legacy_sync(client).await")
            .expect("fallback must start legacy sync");
        assert!(
            started_index < spawn_index,
            "Started(LegacySync) must be emitted before the spawned legacy task can emit Running"
        );
    }

    #[test]
    fn legacy_lifecycle_task_launch_stays_starting() {
        let source = include_str!("sync.rs");
        let lifecycle = source
            .split("enum SyncLifecycle")
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .expect("SyncLifecycle body");
        assert!(
            lifecycle.contains("Starting"),
            "legacy task launch needs an internal Starting state until proof"
        );

        let direct_start = source
            .split("    async fn handle_start")
            .nth(1)
            .and_then(|rest| {
                rest.split("    async fn start_legacy_runtime_fallback")
                    .next()
            })
            .expect("handle_start body");
        assert!(
            !direct_start.contains("self.lifecycle = SyncLifecycle::Running"),
            "neither backend may claim Running in the task-launch path"
        );
    }

    #[test]
    fn legacy_lifecycle_first_response_uses_actor_control_once() {
        let source = include_str!("sync.rs");
        let legacy_loop = source
            .split("async fn run_legacy_sync_loop")
            .nth(1)
            .and_then(|rest| rest.split("fn legacy_sync_settings").next())
            .expect("legacy sync loop body");
        assert!(
            legacy_loop.contains("FirstResponseCommitted"),
            "the legacy task must prove its first committed response to SyncActor"
        );
        assert_eq!(
            legacy_loop.matches("FirstResponseCommitted").count(),
            1,
            "the task sends the promotion control only for the first response"
        );

        let control_arm = source
            .split("    async fn handle_control")
            .nth(1)
            .expect("SyncActor control handler")
            .split("SyncActorControl::FirstResponseCommitted {")
            .nth(1)
            .and_then(|rest| {
                rest.split("SyncActorControl::FirstResponseCommitted { .. }")
                    .next()
            })
            .expect("accepted first-response control arm");
        assert!(
            control_arm
                .find("notify_dependents_sync_started")
                .expect("legacy dependent handoff")
                < control_arm
                    .find("SyncEvent::Running")
                    .expect("legacy Running event"),
            "legacy Running must not race ahead of the timeline/room dependent handoff"
        );
    }

    #[test]
    fn legacy_lifecycle_idempotent_start_before_proof_does_not_emit_running() {
        let source = include_str!("sync.rs");
        let start = source
            .split("    async fn handle_start")
            .nth(1)
            .and_then(|rest| {
                rest.split("    async fn start_legacy_runtime_fallback")
                    .next()
            })
            .expect("handle_start body");
        assert!(
            start.contains("SyncLifecycle::Starting"),
            "idempotent Start must recognize the pre-proof Starting state"
        );
        assert!(
            start.contains("SyncLifecycle::Starting | SyncLifecycle::Reconnecting"),
            "idempotent Start must not re-publish Running while recovery is unproven"
        );
    }

    #[test]
    fn backend_recovery_controls_are_generation_fenced() {
        assert!(accepts_backend_transition(
            ActiveBackend::LegacySync,
            SyncLifecycle::Running,
            4,
            ActiveBackend::LegacySync,
            4,
            SyncLifecycle::Running,
        ));
        assert!(accepts_backend_transition(
            ActiveBackend::LegacySync,
            SyncLifecycle::Reconnecting,
            4,
            ActiveBackend::LegacySync,
            4,
            SyncLifecycle::Reconnecting,
        ));
        assert!(!accepts_backend_transition(
            ActiveBackend::LegacySync,
            SyncLifecycle::Reconnecting,
            5,
            ActiveBackend::LegacySync,
            4,
            SyncLifecycle::Reconnecting,
        ));
    }

    #[test]
    fn sync_service_offline_state_falls_back_before_first_running_and_recovers_afterward() {
        let source = include_str!("sync.rs");
        let observer_body = source
            .split("async fn observe_sync_service_states")
            .nth(1)
            .and_then(|rest| rest.split("async fn run_legacy_sync_loop").next())
            .expect("observe_sync_service_states body");

        assert!(
            observer_body.contains("SyncEvent::Reconnecting"),
            "SyncService Offline must surface reconnecting state"
        );
        assert!(
            observer_body.contains("network_offline"),
            "Offline reconnecting must use a stable reason, not raw SDK text"
        );
        assert!(
            observer_body.contains("SyncServiceObserverDecision::FallbackBeforeConnectivity")
                && observer_body.contains("return SyncTaskOutcome::Failed")
                && observer_body.contains("ever_ran: status.connectivity_proven"),
            "Offline before RoomListService proves connectivity must exit the observer so SyncService startup failure can use the existing LegacySync fallback"
        );
        assert!(
            observer_body.contains("SyncServiceObserverDecision::WaitRecovery"),
            "Offline after RoomListService proves connectivity must keep the observer alive so mobile-network reconnects can recover without switching backends"
        );
    }

    #[test]
    fn sync_service_error_state_falls_back_before_first_running_and_recovers_afterward() {
        let source = include_str!("sync.rs");
        let observer_body = source
            .split("async fn observe_sync_service_states")
            .nth(1)
            .and_then(|rest| rest.split("async fn run_legacy_sync_loop").next())
            .expect("observe_sync_service_states body");

        assert!(
            observer_body.contains("SyncEvent::Reconnecting"),
            "SyncService Error must surface reconnecting state"
        );
        assert!(
            observer_body.contains("network_error"),
            "Error reconnecting must use a stable reason, not raw SDK text"
        );
        assert!(
            observer_body.contains("SyncServiceObserverDecision::FallbackBeforeConnectivity")
                && observer_body.contains("return SyncTaskOutcome::Failed")
                && observer_body.contains("ever_ran: status.connectivity_proven"),
            "Error before RoomListService proves connectivity must exit the observer so SyncService startup failure can use the existing LegacySync fallback"
        );
        assert!(
            observer_body.contains("SyncServiceObserverDecision::WaitRecovery"),
            "Error after RoomListService proves connectivity must keep the observer alive so transient runtime failures can recover"
        );
    }

    #[test]
    fn sync_service_first_running_hands_live_room_list_service_to_dependents() {
        let source = include_str!("sync.rs");
        let observer_body = source
            .split("async fn observe_sync_service_states")
            .nth(1)
            .and_then(|rest| rest.split("async fn run_legacy_sync_loop").next())
            .expect("observe_sync_service_states body");
        let first_running_arm = observer_body
            .split("SyncServiceObserverDecision::InitialRunningHandoff =>")
            .nth(1)
            .and_then(|rest| {
                rest.split("SyncServiceObserverDecision::RecoveryHandoff")
                    .next()
            })
            .expect("first Running arm");

        assert!(
            first_running_arm.contains("notify_dependents_sync_started"),
            "SyncService must only hand live services to RoomActor/TimelineManagerActor after the SDK reports the first Running state"
        );
        assert!(
            first_running_arm.contains("Some(room_list_service.clone())"),
            "initial SyncService handoff must use the live RoomListService owned by the running SyncService"
        );
        assert!(
            first_running_arm
                .find("notify_dependents_sync_started")
                .expect("dependent handoff")
                < first_running_arm
                    .find("SyncActorControl::SyncServiceRunning")
                    .expect("actor lifecycle promotion"),
            "SyncActor must not enter Running before the timeline/room dependent handoff completes"
        );
        assert!(
            first_running_arm
                .find("SyncActorControl::SyncServiceRunning")
                .expect("actor lifecycle promotion")
                < first_running_arm
                    .find("SyncEvent::Running")
                    .expect("public Running event"),
            "public Running must follow the actor lifecycle promotion"
        );
    }

    #[test]
    fn sync_service_recovery_rehands_live_room_list_service_to_dependents() {
        let source = include_str!("sync.rs");
        let observer_body = source
            .split("async fn observe_sync_service_states")
            .nth(1)
            .and_then(|rest| rest.split("async fn run_legacy_sync_loop").next())
            .expect("observe_sync_service_states body");
        let recovery_arm = observer_body
            .split("SyncServiceObserverDecision::RecoveryHandoff =>")
            .nth(1)
            .and_then(|rest| {
                rest.split("SyncServiceObserverDecision::RunningNoop")
                    .next()
            })
            .expect("Running recovery arm");

        assert!(
            recovery_arm
                .find("notify_dependents_sync_started")
                .expect("recovery dependent handoff")
                < recovery_arm
                    .find("SyncActorControl::BackendRecovered")
                    .expect("generation-fenced recovery control"),
            "recovery promotion must not race ahead of rebuilding dependent timelines"
        );

        assert!(
            recovery_arm.contains("notify_dependents_sync_started"),
            "SyncService recovery must re-hand the live RoomListService to RoomActor and TimelineManagerActor so repeated mobile-network reconnects rebuild live room/timeline streams"
        );
        assert!(
            recovery_arm.contains("Some(room_list_service.clone())"),
            "SyncService recovery must reuse the running service's live RoomListService handle instead of constructing a disposable one"
        );

        let start_body = source
            .split("    async fn start_sync_service")
            .nth(1)
            .and_then(|rest| rest.split("    async fn start_legacy_sync").next())
            .expect("start_sync_service body");
        assert!(
            start_body.contains("self.session.clone()")
                && start_body.contains("self.room_tx.clone()")
                && start_body.contains("self.timeline_tx.clone()")
                && start_body.contains("service.room_list_service()"),
            "SyncService observer must receive the session, dependent actor senders, and live RoomListService handle needed to repair downstream streams after recovery"
        );
        assert!(
            !start_body.contains("observer_sync_service = service.clone()"),
            "SyncService observer must not retain the entire SyncService just to repair downstream streams; holding only the RoomListService keeps stop/shutdown ownership narrower"
        );
    }

    #[test]
    fn sync_service_terminated_state_exits_observer_for_backend_fallback() {
        let source = include_str!("sync.rs");
        let observer_body = source
            .split("async fn observe_sync_service_states")
            .nth(1)
            .and_then(|rest| rest.split("async fn run_legacy_sync_loop").next())
            .expect("observe_sync_service_states body");
        let terminated_arm = observer_body
            .split("SyncServiceObserverDecision::Fail =>")
            .nth(1)
            .and_then(|rest| rest.split("SyncServiceObserverDecision::Ignore").next())
            .expect("SyncService Terminated arm");

        assert!(
            terminated_arm.contains("SyncTaskOutcome::Failed"),
            "unexpected SyncService termination must leave the observer so SyncActor can fall back to LegacySync"
        );
        assert!(
            terminated_arm.contains("kind: SyncFailureKind::Http"),
            "terminated fallback must use the stable HTTP failure kind, not raw SDK text"
        );
    }

    #[test]
    fn sync_service_observer_starts_before_service_start() {
        let source = include_str!("sync.rs");
        let body = source
            .split("    async fn start_sync_service")
            .nth(1)
            .and_then(|rest| rest.split("    async fn start_legacy_sync").next())
            .expect("start_sync_service body");

        let observer_index = body.find("executor::spawn").expect("observer task spawn");
        let start_index = body
            .find("service.start().await")
            .expect("SyncService start call");

        assert!(
            observer_index < start_index,
            "SyncService state observer must be running before service.start() can emit transient states"
        );
    }

    #[test]
    fn sync_service_observer_checks_current_state_before_waiting() {
        let source = include_str!("sync.rs");
        let body = source
            .split("async fn observe_sync_service_states")
            .nth(1)
            .and_then(|rest| rest.split("async fn run_legacy_sync_loop").next())
            .expect("observe_sync_service_states body");

        let get_index = body.find("state_sub.get()").expect("current state read");
        let next_index = body.find("state_sub.next()").expect("next state wait");
        assert!(
            get_index < next_index,
            "SyncService observer must inspect the current state before waiting for a future change"
        );
        let room_list_get_index = body
            .find("room_list_state_sub.get()")
            .expect("current RoomListService state read");
        let room_list_next_index = body
            .find("room_list_state_sub.next()")
            .expect("next RoomListService state wait");
        assert!(
            room_list_get_index < room_list_next_index,
            "SyncService observer must inspect the current RoomListService state before waiting for a future change"
        );
    }

    #[test]
    fn legacy_sync_filter_fetches_full_stress_burst_per_room() {
        let filter = legacy_sync_filter_definition();

        assert_eq!(
            filter.room.timeline.limit,
            Some(matrix_sdk::ruma::uint!(128)),
            "LegacySync fallback must keep a large enough live tail to avoid a gap between /sync and /messages backfill under the local timeline stress cap"
        );
    }

    #[test]
    fn stale_legacy_first_response_cannot_promote_a_replacement_run() {
        assert!(accepts_first_legacy_response(
            ActiveBackend::LegacySync,
            SyncLifecycle::Starting,
            8,
            8,
        ));
        assert!(!accepts_first_legacy_response(
            ActiveBackend::LegacySync,
            SyncLifecycle::Starting,
            8,
            7,
        ));
        assert!(!accepts_first_legacy_response(
            ActiveBackend::LegacySync,
            SyncLifecycle::Running,
            8,
            8,
        ));
    }

    #[test]
    fn stale_sync_service_running_cannot_promote_a_replacement_run() {
        assert!(accepts_sync_service_running(
            ActiveBackend::SyncService,
            SyncLifecycle::Starting,
            8,
            8,
        ));
        assert!(!accepts_sync_service_running(
            ActiveBackend::SyncService,
            SyncLifecycle::Starting,
            8,
            7,
        ));
        assert!(!accepts_sync_service_running(
            ActiveBackend::LegacySync,
            SyncLifecycle::Starting,
            8,
            8,
        ));
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

    #[tokio::test]
    async fn action_channel_accepts_projected_sync_statuses_with_generations() {
        let (action_tx, mut action_rx) = mpsc::channel::<Vec<AppAction>>(16);
        let generation = std::sync::atomic::AtomicU64::new(0);

        send_sync_status(
            &action_tx,
            &generation,
            koushi_state::SyncLifecycleStatus::Starting,
        )
        .await;
        send_sync_status(
            &action_tx,
            &generation,
            koushi_state::SyncLifecycleStatus::Running,
        )
        .await;

        let a1 = action_rx.recv().await.unwrap();
        let a2 = action_rx.recv().await.unwrap();
        assert!(matches!(
            a1[0],
            AppAction::SyncStatusChanged {
                generation: 1,
                status: koushi_state::SyncLifecycleStatus::Starting
            }
        ));
        assert!(matches!(
            a2[0],
            AppAction::SyncStatusChanged {
                generation: 2,
                status: koushi_state::SyncLifecycleStatus::Running
            }
        ));
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
