//! `trust_gate` ownership for AccountActor.

use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_sdk::MatrixClientSession;
#[cfg(any(test, feature = "test-hooks"))]
use koushi_state::VerificationTarget;
use koushi_state::{
    AppAction, E2eeRecoveryState, RecoveryMethod, SessionInfo, TrustOperationFailureKind,
};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::executor;
use crate::store::account_key_from_info;
use koushi_protocol::event::{AccountEvent, CoreEvent};
use koushi_protocol::failure::CoreFailure;
use koushi_protocol::ids::{AccountKey, RequestId};

use super::actor::{AccountActor, AccountMessage};
use super::recovery_backup::{
    classify_e2ee_trust_error, record_recovery_verification_event, recovery_verification_event,
};
use super::verification::send_observer_output_until_stopped;

const VERIFICATION_METHOD_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(30);

const VERIFICATION_METHOD_DISCOVERY_ADMISSION_TIMEOUT: Duration = Duration::from_secs(20);

const CURRENT_SESSION_STATUS_TIMEOUT: Duration = Duration::from_secs(15);

fn current_session_status_connectivity_proven(
    sync_state: koushi_state::CurrentSessionSyncState,
) -> bool {
    sync_state == koushi_state::CurrentSessionSyncState::Running
}

fn current_session_status_failure(
    error: koushi_sdk::MatrixCurrentSessionInspectionError,
) -> koushi_state::CurrentSessionStatusFailureKind {
    match error {
        koushi_sdk::MatrixCurrentSessionInspectionError::Unavailable
        | koushi_sdk::MatrixCurrentSessionInspectionError::CurrentDeviceMissing => {
            koushi_state::CurrentSessionStatusFailureKind::Unavailable
        }
        koushi_sdk::MatrixCurrentSessionInspectionError::DeviceRequest
        | koushi_sdk::MatrixCurrentSessionInspectionError::IdentityRequest => {
            koushi_state::CurrentSessionStatusFailureKind::Sdk
        }
        koushi_sdk::MatrixCurrentSessionInspectionError::Authentication => {
            koushi_state::CurrentSessionStatusFailureKind::Authentication
        }
        koushi_sdk::MatrixCurrentSessionInspectionError::Network => {
            koushi_state::CurrentSessionStatusFailureKind::Network
        }
        koushi_sdk::MatrixCurrentSessionInspectionError::Server => {
            koushi_state::CurrentSessionStatusFailureKind::Server
        }
    }
}

pub(super) fn record_verification_admission_event(event: DiagnosticEvent) {
    koushi_diagnostics::record_and_stderr(event);
}

pub(super) fn verification_admission_event(
    stage: &'static str,
    generation: u64,
    transition_id: u64,
) -> DiagnosticEvent {
    DiagnosticEvent::new(DiagnosticLevel::Info, "core.verification_admission", stage)
        .field(DiagnosticField::count("generation", generation))
        .field(DiagnosticField::count("transition_id", transition_id))
}

pub(super) fn current_device_trust_token(
    trust: koushi_state::CurrentDeviceTrustState,
) -> &'static str {
    match trust {
        koushi_state::CurrentDeviceTrustState::Unknown => "unknown",
        koushi_state::CurrentDeviceTrustState::Unverified => "unverified",
        koushi_state::CurrentDeviceTrustState::Verified => "verified",
    }
}

pub(super) fn current_device_trust_recheck_failure_token(
    error: &koushi_sdk::CurrentDeviceTrustRecheckError,
) -> &'static str {
    match error {
        koushi_sdk::CurrentDeviceTrustRecheckError::Authentication => "authentication",
        koushi_sdk::CurrentDeviceTrustRecheckError::Network => "network",
        koushi_sdk::CurrentDeviceTrustRecheckError::Server => "server",
        koushi_sdk::CurrentDeviceTrustRecheckError::Sdk => "sdk",
    }
}

pub(super) fn record_verification_method_discovery_event(event: DiagnosticEvent) {
    koushi_diagnostics::record_and_stderr(event);
}

pub(super) fn verification_method_discovery_event(
    stage: &'static str,
    generation: u64,
    serial: u64,
) -> DiagnosticEvent {
    DiagnosticEvent::new(
        DiagnosticLevel::Info,
        "core.verification_method_discovery",
        stage,
    )
    .field(DiagnosticField::count("generation", generation))
    .field(DiagnosticField::count("serial", serial))
}

pub(super) struct RecoveryStateObservation {
    stop_tx: oneshot::Sender<()>,
    task: crate::executor::JoinHandle<()>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TrustLifecycleDecision {
    IgnoreStale,
    StayGated,
    Promote,
    Gate,
    AlreadyReady,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct PendingTrustTransition {
    pub(super) generation: u64,
    transition_id: u64,
    pub(super) decision: TrustLifecycleDecision,
}

pub(super) struct OwnedVerificationMethodDiscoveryTask {
    pub(super) generation: u64,
    pub(super) serial: u64,
    pub(super) task: crate::executor::JoinHandle<()>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VerificationMethodDiscoveryResult {
    Discovered(koushi_state::VerificationGateState),
    Failed(koushi_state::VerificationGateFailureKind),
}

fn trust_projection_ack_matches(
    pending: &PendingTrustTransition,
    generation: u64,
    transition_id: u64,
    ready: bool,
    locked: bool,
) -> bool {
    pending.generation == generation
        && pending.transition_id == transition_id
        && match pending.decision {
            TrustLifecycleDecision::Promote => ready && !locked,
            TrustLifecycleDecision::Gate => !ready && !locked,
            _ => false,
        }
}

pub(super) fn verification_gate_failure_token(
    kind: koushi_state::VerificationGateFailureKind,
) -> &'static str {
    match kind {
        koushi_state::VerificationGateFailureKind::Network => "network",
        koushi_state::VerificationGateFailureKind::Cancelled => "cancelled",
        koushi_state::VerificationGateFailureKind::Mismatch => "mismatch",
        koushi_state::VerificationGateFailureKind::Forbidden => "forbidden",
        koushi_state::VerificationGateFailureKind::Timeout => "timeout",
        koushi_state::VerificationGateFailureKind::Sdk => "sdk",
        koushi_state::VerificationGateFailureKind::NoProofMethod => "no_proof_method",
    }
}

fn begin_provisional_encryption_sync_cursor_attempt(
    provisional_encryption_sync_active: bool,
) -> bool {
    !provisional_encryption_sync_active
}

fn recovery_sync_should_resume(
    recovery_generation: u64,
    active_generation: u64,
    session_promoted: bool,
    provisional_encryption_sync_active: bool,
) -> bool {
    recovery_generation == active_generation
        && !session_promoted
        && !provisional_encryption_sync_active
}

#[cfg(any(test, feature = "test-hooks"))]
pub(super) async fn refresh_device_keys_and_assert_known(
    session: &MatrixClientSession,
    target: VerificationTarget,
) -> Result<(), ()> {
    let user_id = matrix_sdk::ruma::UserId::parse(target.user_id).map_err(|_| ())?;
    let device_id = matrix_sdk::ruma::OwnedDeviceId::from(target.device_id);
    let encryption = session.client().encryption();

    let _ = encryption
        .request_user_identity(&user_id)
        .await
        .map_err(|_| ())?;
    encryption
        .get_device(&user_id, &device_id)
        .await
        .map_err(|_| ())?
        .ok_or(())?;
    Ok(())
}

fn trust_lifecycle_decision(
    generation: u64,
    active_generation: u64,
    promoted: bool,
    trust: koushi_state::CurrentDeviceTrustState,
) -> TrustLifecycleDecision {
    if generation != active_generation {
        TrustLifecycleDecision::IgnoreStale
    } else if promoted {
        if matches!(trust, koushi_state::CurrentDeviceTrustState::Verified) {
            TrustLifecycleDecision::AlreadyReady
        } else {
            TrustLifecycleDecision::Gate
        }
    } else if matches!(trust, koushi_state::CurrentDeviceTrustState::Verified) {
        TrustLifecycleDecision::Promote
    } else {
        TrustLifecycleDecision::StayGated
    }
}

#[allow(clippy::too_many_arguments)]
fn current_session_status_completion_action(
    active_request_id: Option<u64>,
    active_generation: u64,
    session_promoted: bool,
    session_info: Option<&SessionInfo>,
    request_id: u64,
    generation: u64,
    sync_state: koushi_state::CurrentSessionSyncState,
    result: Result<
        koushi_sdk::MatrixCurrentSessionInspection,
        koushi_state::CurrentSessionStatusFailureKind,
    >,
    checked_at_ms: u64,
) -> Option<AppAction> {
    if active_request_id != Some(request_id) || active_generation != generation || !session_promoted
    {
        return None;
    }
    Some(match result {
        Ok(inspection) => {
            let info = session_info?;
            AppAction::CurrentSessionStatusRefreshed {
                request_id,
                details: koushi_state::CurrentSessionStatusDetails::new(
                    inspection.device_display_name,
                    info.device_id.clone(),
                    info.authentication_method,
                    sync_state,
                    inspection.verification,
                    inspection.is_cross_signed_by_owner,
                    inspection.own_identity_verification,
                    inspection.key_backup,
                    checked_at_ms,
                ),
            }
        }
        Err(kind) => AppAction::CurrentSessionStatusRefreshFailed {
            request_id,
            kind,
            checked_at_ms,
        },
    })
}

fn current_session_status_observed_non_verified_trust(
    result: &Result<
        koushi_sdk::MatrixCurrentSessionInspection,
        koushi_state::CurrentSessionStatusFailureKind,
    >,
) -> Option<koushi_state::CurrentDeviceTrustState> {
    result.as_ref().ok().and_then(|inspection| {
        (inspection.verification != koushi_state::CurrentDeviceTrustState::Verified)
            .then_some(inspection.verification)
    })
}

fn current_session_status_settled_event(action: &AppAction, elapsed: Duration) -> DiagnosticEvent {
    let event =
        DiagnosticEvent::new(DiagnosticLevel::Debug, "session_status", "refresh_settled").field(
            DiagnosticField::milliseconds("elapsed_ms", elapsed.as_millis()),
        );
    match action {
        AppAction::CurrentSessionStatusRefreshed { details, .. } => event
            .field(DiagnosticField::token("result", "ready"))
            .field(DiagnosticField::token(
                "verdict",
                match details.verification {
                    koushi_state::CurrentDeviceTrustState::Verified => "verified",
                    koushi_state::CurrentDeviceTrustState::Unverified => "unverified",
                    koushi_state::CurrentDeviceTrustState::Unknown => "unknown",
                },
            )),
        AppAction::CurrentSessionStatusRefreshFailed { kind, .. } => {
            event.field(DiagnosticField::token(
                "result",
                match kind {
                    koushi_state::CurrentSessionStatusFailureKind::Sdk => "sdk",
                    koushi_state::CurrentSessionStatusFailureKind::TimedOut => "timed_out",
                    koushi_state::CurrentSessionStatusFailureKind::Unavailable => "unavailable",
                    koushi_state::CurrentSessionStatusFailureKind::ConnectivityUnavailable => {
                        "connectivity_unavailable"
                    }
                    koushi_state::CurrentSessionStatusFailureKind::Authentication => {
                        "authentication"
                    }
                    koushi_state::CurrentSessionStatusFailureKind::Network => "network",
                    koushi_state::CurrentSessionStatusFailureKind::Server => "server",
                },
            ))
        }
        _ => event.field(DiagnosticField::token("result", "invalid")),
    }
}

pub(super) fn method_discovery_is_current(
    generation: u64,
    current_generation: u64,
    serial: u64,
    current_serial: u64,
    has_session: bool,
) -> bool {
    has_session && generation == current_generation && serial == current_serial
}

pub(super) fn retry_should_restart_method_discovery(
    session_promoted: bool,
    trust: Option<koushi_state::CurrentDeviceTrustState>,
) -> bool {
    !session_promoted
        && matches!(
            trust,
            Some(koushi_state::CurrentDeviceTrustState::Unverified)
        )
}

pub(super) fn method_discovery_admission_timeout_is_current(
    generation: u64,
    current_generation: u64,
    serial: u64,
    current_serial: u64,
    has_session: bool,
    session_promoted: bool,
    discovery_task_active: bool,
) -> bool {
    has_session
        && !session_promoted
        && !discovery_task_active
        && generation == current_generation
        && serial == current_serial
}

async fn wait_for_verification_method_discovery<F>(
    timeout: Duration,
    future: F,
) -> VerificationMethodDiscoveryResult
where
    F: Future<Output = koushi_state::VerificationGateState>,
{
    match executor::timeout(timeout, future).await {
        Err(_) => VerificationMethodDiscoveryResult::Failed(
            koushi_state::VerificationGateFailureKind::Timeout,
        ),
        Ok(gate) if gate.account_kind == koushi_state::VerificationAccountKind::Unknown => {
            VerificationMethodDiscoveryResult::Failed(
                koushi_state::VerificationGateFailureKind::Sdk,
            )
        }
        Ok(gate) => VerificationMethodDiscoveryResult::Discovered(gate),
    }
}

pub(super) fn first_provisional_encryption_sync_is_current(
    generation: u64,
    current_generation: u64,
    has_session: bool,
    session_promoted: bool,
) -> bool {
    has_session && !session_promoted && generation == current_generation
}

pub(super) fn own_user_sas_recheck_is_current(
    generation: u64,
    current_generation: u64,
    has_session: bool,
    session_promoted: bool,
    has_own_user_flow: bool,
    has_sas: bool,
) -> bool {
    generation == current_generation
        && has_session
        && !session_promoted
        && has_own_user_flow
        && !has_sas
}

pub(super) fn active_own_user_sas_flow_for_provisional_encryption_sync(
    generation: u64,
    current_generation: u64,
    has_session: bool,
    session_promoted: bool,
    own_flow_id: Option<u64>,
) -> Option<u64> {
    (generation == current_generation && has_session && !session_promoted)
        .then_some(own_flow_id)
        .flatten()
}

#[cfg(test)]
pub(super) fn unknown_verification_gate() -> koushi_state::VerificationGateState {
    koushi_state::VerificationGateState {
        methods: Vec::new(),
        account_kind: koushi_state::VerificationAccountKind::Unknown,
        failure: None,
    }
}

pub(super) fn should_discover_verification_methods(
    trust: koushi_state::CurrentDeviceTrustState,
) -> bool {
    trust == koushi_state::CurrentDeviceTrustState::Unverified
}

pub(super) fn advance_observed_trust(
    last_trust: &mut koushi_state::CurrentDeviceTrustState,
    observed: koushi_state::CurrentDeviceTrustState,
) -> bool {
    if *last_trust == observed {
        return false;
    }
    *last_trust = observed;
    true
}

async fn run_recovery_state_observation<S>(
    state_stream: S,
    account_key: AccountKey,
    action_tx: mpsc::Sender<Vec<AppAction>>,
    event_tx: broadcast::Sender<CoreEvent>,
    mut stop_rx: oneshot::Receiver<()>,
    #[cfg(test)] delivery_barrier: Option<Arc<tokio::sync::Barrier>>,
) where
    S: futures_util::Stream<Item = E2eeRecoveryState> + Send + 'static,
{
    let mut state_stream = Box::pin(state_stream);
    let mut last_state: Option<E2eeRecoveryState> = None;
    let recovery_methods = vec![RecoveryMethod::RecoveryKey];

    loop {
        let mut pinned_stream = state_stream.as_mut();
        let next_state = pinned_stream.next();
        tokio::select! {
            _ = &mut stop_rx => break,
            state = next_state => {
                let Some(state) = state else {
                    break;
                };
                if last_state == Some(state) {
                    continue;
                }
                last_state = Some(state);

                match state {
                    E2eeRecoveryState::Unknown => {}
                    E2eeRecoveryState::Incomplete => {
                        #[cfg(test)]
                        if let Some(barrier) = delivery_barrier.as_ref() {
                            barrier.wait().await;
                        }
                        if !send_observer_output_until_stopped(
                            &action_tx,
                            vec![AppAction::E2eeRecoveryStateChanged {
                                state: E2eeRecoveryState::Incomplete,
                                methods: recovery_methods.clone(),
                            }],
                            &mut stop_rx,
                        )
                        .await {
                            break;
                        }
                        let _ = event_tx.send(CoreEvent::Account(AccountEvent::RecoveryRequired {
                            account_key: account_key.clone(),
                        }));
                    }
                    E2eeRecoveryState::Enabled | E2eeRecoveryState::Disabled => {
                        #[cfg(test)]
                        if let Some(barrier) = delivery_barrier.as_ref() {
                            barrier.wait().await;
                        }
                        if !send_observer_output_until_stopped(
                            &action_tx,
                            vec![AppAction::E2eeRecoveryStateChanged {
                                state,
                                methods: recovery_methods.clone(),
                            }],
                            &mut stop_rx,
                        )
                        .await {
                            break;
                        }
                    }
                }
            }
        }
    }
}

pub(super) fn verification_gate_failure_kind(
    error: &koushi_sdk::E2eeTrustError,
) -> koushi_state::VerificationGateFailureKind {
    match classify_e2ee_trust_error(error) {
        TrustOperationFailureKind::Cancelled => {
            koushi_state::VerificationGateFailureKind::Cancelled
        }
        TrustOperationFailureKind::Mismatch => koushi_state::VerificationGateFailureKind::Mismatch,
        TrustOperationFailureKind::Network => koushi_state::VerificationGateFailureKind::Network,
        TrustOperationFailureKind::Forbidden => {
            koushi_state::VerificationGateFailureKind::Forbidden
        }
        TrustOperationFailureKind::Timeout => koushi_state::VerificationGateFailureKind::Timeout,
        TrustOperationFailureKind::InvalidPassphrase | TrustOperationFailureKind::Sdk => {
            koushi_state::VerificationGateFailureKind::Sdk
        }
    }
}

impl AccountActor {
    pub(super) fn set_secure_backup_send_admitted(&mut self, admitted: bool) {
        self.secure_backup_ready = admitted;
        if let Some(session) = self.session.as_ref() {
            session.set_secure_backup_send_admitted(admitted);
        }
    }

    pub(super) fn start_recovery_observer(&mut self, session: Arc<MatrixClientSession>) {
        let (stop_tx, stop_rx) = oneshot::channel();
        let task = crate::executor::spawn(run_recovery_state_observation(
            session.e2ee_recovery_state_stream(),
            account_key_from_info(&session.info),
            self.action_tx.clone(),
            self.event_tx.clone(),
            stop_rx,
            #[cfg(test)]
            None,
        ));
        self.recovery_observer = Some(RecoveryStateObservation { stop_tx, task });
    }

    pub(super) async fn stop_recovery_observer(&mut self) {
        if let Some(observation) = self.recovery_observer.take() {
            let _ = observation.stop_tx.send(());
            let _ = observation.task.await;
        }
    }

    pub(super) fn resume_provisional_encryption_sync_after_recovery(
        &mut self,
        session: Arc<MatrixClientSession>,
        generation: u64,
        flow_id: u64,
    ) {
        let should_resume = recovery_sync_should_resume(
            generation,
            self.trust_generation,
            self.session_promoted,
            self.provisional_encryption_sync.is_some(),
        );
        record_recovery_verification_event(
            recovery_verification_event("provisional_encryption_sync_resume_decided", flow_id)
                .field(DiagnosticField::boolean("will_resume", should_resume)),
        );
        if should_resume {
            let transition_id = self.next_trust_transition_id();
            self.start_provisional_encryption_sync(session, generation, transition_id);
        }
    }

    pub(super) async fn promote_recovered_session_runtime(
        &mut self,
        generation: u64,
        flow_id: u64,
        request_id: RequestId,
    ) -> bool {
        let (Some(session), Some(key_id)) = (self.session.clone(), self.session_key_id.clone())
        else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return false;
        };
        record_recovery_verification_event(
            recovery_verification_event("promoting", flow_id)
                .field(DiagnosticField::count("generation", generation))
                .field(DiagnosticField::request_id(
                    "request_id",
                    request_id.connection_id.0,
                    request_id.sequence,
                ))
                .field(DiagnosticField::token(
                    "trust",
                    current_device_trust_token(session.current_device_trust()),
                )),
        );
        if self.persist_session(&session, &key_id).await.is_err() {
            self.send_actions(vec![AppAction::SessionPersistenceFailed {
                message: "session persistence failed".to_owned(),
            }])
            .await;
            return false;
        }
        let trust_after_persist = session.current_device_trust();
        record_recovery_verification_event(
            recovery_verification_event("persisted", flow_id)
                .field(DiagnosticField::count("generation", generation))
                .field(DiagnosticField::request_id(
                    "request_id",
                    request_id.connection_id.0,
                    request_id.sequence,
                ))
                .field(DiagnosticField::token(
                    "trust",
                    current_device_trust_token(trust_after_persist),
                )),
        );
        self.provisional_persistable = None;
        self.stop_provisional_runtime().await;
        self.start_incoming_verification_observer(session.clone())
            .await;
        self.spawn_sync_actor(session.clone()).await;
        self.spawn_account_hydration(session.clone());
        self.start_active_session_account_management_discovery(session.clone())
            .await;
        self.start_recovery_observer(session.clone());
        let trust_at_promotion = session.current_device_trust();
        self.start_session_change_observer(session.clone());
        self.session_promoted = true;
        record(
            DiagnosticEvent::new(DiagnosticLevel::Info, "core.session_promotion", "changed")
                .field(DiagnosticField::token("state", "promoted")),
        );
        self.start_secure_backup_observer(session.clone());
        for event in std::mem::take(&mut self.pending_ready_events) {
            self.emit(event);
        }
        if let Some(account_epoch) = self.sliding_sync_revalidation_pending {
            self.start_sliding_sync_revalidation(account_epoch).await;
        }
        record_recovery_verification_event(
            recovery_verification_event("promoted", flow_id)
                .field(DiagnosticField::count("generation", generation))
                .field(DiagnosticField::request_id(
                    "request_id",
                    request_id.connection_id.0,
                    request_id.sequence,
                ))
                .field(DiagnosticField::token(
                    "trust",
                    current_device_trust_token(trust_at_promotion),
                )),
        );
        true
    }

    pub(super) fn request_authoritative_trust_recheck(&mut self) {
        record_verification_admission_event(verification_admission_event(
            "trust_recheck_requested",
            self.trust_generation,
            0,
        ));
        self.trust_recheck_pending = true;
        self.start_authoritative_trust_recheck_if_idle(false);
    }

    pub(super) fn start_authoritative_trust_recheck_if_idle(
        &mut self,
        allow_pending_projection: bool,
    ) {
        // A replay after an in-flight query already represents a later reducer
        // demand, so it may start before the first result's projection settles.
        // Ordinary requests wait: a matching projection can satisfy redundant
        // demand, while a mismatched acknowledgement explicitly releases it.
        if self.trust_recheck_task.is_some()
            || (!allow_pending_projection
                && matches!(
                    self.pending_trust_transition,
                    Some(PendingTrustTransition { generation, .. })
                        if generation == self.trust_generation
                ))
        {
            return;
        }
        let Some(session) = self.session.clone() else {
            self.trust_recheck_pending = false;
            return;
        };
        self.trust_recheck_pending = false;
        let generation = self.trust_generation;
        record_verification_admission_event(verification_admission_event(
            "trust_recheck_started",
            generation,
            0,
        ));
        let tx = self.self_tx.clone();
        self.trust_recheck_task = Some(executor::spawn(async move {
            let result = session.recheck_current_device_trust().await;
            let _ = tx
                .send(AccountMessage::CurrentDeviceTrustRecheckFinished { generation, result })
                .await;
        }));
    }

    pub(super) fn start_current_session_status_refresh(
        &mut self,
        request_id: u64,
        trigger: koushi_state::SessionStatusRefreshTrigger,
        sync_state: koushi_state::CurrentSessionSyncState,
    ) {
        if self.current_session_status_request == Some(request_id) {
            return;
        }
        if let Some(task) = self.current_session_status_task.take() {
            task.abort();
        }
        self.current_session_status_request = Some(request_id);
        let generation = self.trust_generation;
        let started_at = Instant::now();
        record(
            DiagnosticEvent::new(DiagnosticLevel::Debug, "session_status", "refresh_started")
                .field(DiagnosticField::token(
                    "trigger",
                    match trigger {
                        koushi_state::SessionStatusRefreshTrigger::Open => "open",
                        koushi_state::SessionStatusRefreshTrigger::Manual => "manual",
                        koushi_state::SessionStatusRefreshTrigger::Recovery => "recovery",
                    },
                )),
        );
        let Some(session) = self.session.clone().filter(|_| self.session_promoted) else {
            let tx = self.self_tx.clone();
            self.current_session_status_task = Some(executor::spawn(async move {
                let _ = tx
                    .send(AccountMessage::CurrentSessionStatusRefreshFinished {
                        request_id,
                        generation,
                        sync_state,
                        started_at,
                        result: Err(koushi_state::CurrentSessionStatusFailureKind::Unavailable),
                    })
                    .await;
            }));
            return;
        };
        let tx = self.self_tx.clone();
        if !current_session_status_connectivity_proven(sync_state) {
            record(
                DiagnosticEvent::new(DiagnosticLevel::Info, "session_status", "refresh_deferred")
                    .field(DiagnosticField::token("reason", "connectivity_unproven")),
            );
            self.current_session_status_task = Some(executor::spawn(async move {
                let _ = tx
                    .send(AccountMessage::CurrentSessionStatusRefreshFinished {
                        request_id,
                        generation,
                        sync_state,
                        started_at,
                        result: Err(
                            koushi_state::CurrentSessionStatusFailureKind::ConnectivityUnavailable,
                        ),
                    })
                    .await;
            }));
            return;
        }
        self.current_session_status_task = Some(executor::spawn(async move {
            let result = match executor::timeout(
                CURRENT_SESSION_STATUS_TIMEOUT,
                session.inspect_current_session(),
            )
            .await
            {
                Ok(Ok(inspection)) => Ok(inspection),
                Ok(Err(error)) => Err(current_session_status_failure(error)),
                Err(_) => Err(koushi_state::CurrentSessionStatusFailureKind::TimedOut),
            };
            let _ = tx
                .send(AccountMessage::CurrentSessionStatusRefreshFinished {
                    request_id,
                    generation,
                    sync_state,
                    started_at,
                    result,
                })
                .await;
        }));
    }

    pub(super) async fn finish_current_session_status_refresh(
        &mut self,
        request_id: u64,
        generation: u64,
        sync_state: koushi_state::CurrentSessionSyncState,
        started_at: Instant,
        result: Result<
            koushi_sdk::MatrixCurrentSessionInspection,
            koushi_state::CurrentSessionStatusFailureKind,
        >,
    ) {
        if self.current_session_status_request == Some(request_id)
            && self.trust_generation == generation
            && self.session_promoted
            && let Some(trust) = current_session_status_observed_non_verified_trust(&result)
        {
            self.current_session_status_request = None;
            self.current_session_status_task = None;
            self.handle_current_device_trust(generation, trust).await;
            return;
        }
        let checked_at_ms = crate::time::current_epoch_ms();
        let Some(action) = current_session_status_completion_action(
            self.current_session_status_request,
            self.trust_generation,
            self.session_promoted,
            self.session.as_ref().map(|session| &session.info),
            request_id,
            generation,
            sync_state,
            result,
            checked_at_ms,
        ) else {
            return;
        };
        self.current_session_status_request = None;
        self.current_session_status_task = None;
        record(current_session_status_settled_event(
            &action,
            started_at.elapsed(),
        ));
        self.send_actions(vec![action]).await;
    }

    pub(super) async fn cancel_current_session_status_refresh(&mut self) {
        self.current_session_status_request = None;
        if let Some(task) = self.current_session_status_task.take() {
            task.abort();
            let _ = task.await;
        }
    }

    fn start_provisional_encryption_sync(
        &mut self,
        session: Arc<MatrixClientSession>,
        generation: u64,
        transition_id: u64,
    ) {
        if !begin_provisional_encryption_sync_cursor_attempt(
            self.provisional_encryption_sync.is_some(),
        ) {
            return;
        }
        self.provisional_encryption_sync_ready = false;
        record_verification_admission_event(verification_admission_event(
            "provisional_encryption_sync_started",
            generation,
            transition_id,
        ));
        self.sliding_sync_diagnostics
            .provisional_encryption_started();
        #[cfg(any(test, feature = "test-hooks"))]
        if self.trust_observation_is_synthetic {
            let _ = session;
            self.provisional_encryption_sync = Some(executor::spawn(std::future::pending()));
            return;
        }
        let tx = self.self_tx.clone();
        let encryption_sync_permit = self.encryption_sync_permit.clone();
        self.provisional_encryption_sync = Some(executor::spawn(async move {
            let first_response_seen = Arc::new(AtomicBool::new(false));
            let failure_reported = Arc::new(AtomicBool::new(false));
            loop {
                let callback_tx = tx.clone();
                let callback_first_response_seen = first_response_seen.clone();
                let callback_failure_reported = failure_reported.clone();
                let sync = koushi_sdk::provisional_encryption_sync_loop(
                    &session,
                    encryption_sync_permit.clone(),
                    move || {
                        let callback_tx = callback_tx.clone();
                        // Publish the first-response fence only after the actor
                        // message is accepted. If a bounded mailbox send is
                        // cancelled by the outer deadline, the failure path
                        // must still report that admission never completed.
                        let first = !callback_first_response_seen.load(Ordering::Acquire);
                        callback_failure_reported.store(false, Ordering::Release);
                        let callback_first_response_seen = callback_first_response_seen.clone();
                        async move {
                            let message = if first {
                                AccountMessage::FirstProvisionalEncryptionSyncFinished {
                                    generation,
                                    succeeded: true,
                                }
                            } else {
                                AccountMessage::ProvisionalEncryptionSyncSucceeded { generation }
                            };
                            if callback_tx.send(message).await.is_ok() {
                                if first {
                                    callback_first_response_seen.store(true, Ordering::Release);
                                }
                                koushi_sdk::MatrixSyncLoopControl::Continue
                            } else {
                                koushi_sdk::MatrixSyncLoopControl::Stop
                            }
                        }
                    },
                );
                let (result, timed_out) = if first_response_seen.load(Ordering::Acquire) {
                    (sync.await, false)
                } else {
                    match executor::timeout(Duration::from_secs(15), sync).await {
                        Ok(result) => (result, false),
                        Err(_) => (Err(koushi_sdk::ProvisionalEncryptionSyncError::Sdk), true),
                    }
                };
                if timed_out {
                    koushi_sdk::record_encryption_sync_lifecycle(
                        koushi_sdk::EncryptionSyncLifecycleOwner::Provisional,
                        session.client().encryption_sync_readiness_snapshot(),
                        koushi_sdk::EncryptionSyncLifecycleStage::Failed,
                        Duration::from_secs(15),
                    );
                }

                if result.is_ok() {
                    break;
                }
                if !first_response_seen.load(Ordering::Acquire) {
                    if !failure_reported.swap(true, Ordering::AcqRel)
                        && tx
                            .send(AccountMessage::FirstProvisionalEncryptionSyncFinished {
                                generation,
                                succeeded: false,
                            })
                            .await
                            .is_err()
                    {
                        break;
                    }
                    executor::sleep(Duration::from_millis(250)).await;
                    continue;
                }
                if !failure_reported.swap(true, Ordering::AcqRel)
                    && tx
                        .send(AccountMessage::ProvisionalEncryptionSyncFailed { generation })
                        .await
                        .is_err()
                {
                    break;
                }
                executor::sleep(Duration::from_millis(250)).await;
            }
        }));
    }

    pub(super) async fn stop_provisional_encryption_sync(&mut self) {
        self.provisional_encryption_sync_ready = false;
        if let Some(task) = self.provisional_encryption_sync.take() {
            task.abort();
            let _ = task.await;
            self.sliding_sync_diagnostics
                .provisional_encryption_stopped();
            self.record_lifecycle_probe("provisional_encryption_sync_terminated");
        }
    }

    pub(super) async fn handle_current_device_trust(
        &mut self,
        generation: u64,
        trust: koushi_state::CurrentDeviceTrustState,
    ) {
        match trust_lifecycle_decision(
            generation,
            self.trust_generation,
            self.session_promoted,
            trust,
        ) {
            TrustLifecycleDecision::IgnoreStale | TrustLifecycleDecision::AlreadyReady => return,
            TrustLifecycleDecision::StayGated => {
                self.cancel_pending_trust_promotion().await;
                let transition_id = self.next_trust_transition_id();
                if self.provisional_encryption_sync_ready {
                    self.pending_trust_transition = Some(PendingTrustTransition {
                        generation,
                        transition_id,
                        decision: TrustLifecycleDecision::Gate,
                    });
                }
                self.send_actions(vec![AppAction::AuthoritativeDeviceTrustChanged {
                    generation,
                    transition_id,
                    trust,
                }])
                .await;
                if should_discover_verification_methods(trust) {
                    self.arm_verification_method_discovery_admission_timeout(generation)
                        .await;
                }
                if self.provisional_encryption_sync.is_none()
                    && let Some(session) = self.session.clone()
                {
                    self.start_provisional_encryption_sync(session, generation, transition_id);
                }
                return;
            }
            TrustLifecycleDecision::Gate => {
                let transition_id = match self.pending_trust_transition.as_ref() {
                    Some(PendingTrustTransition {
                        generation: pending_generation,
                        transition_id,
                        decision: TrustLifecycleDecision::Gate,
                    }) if *pending_generation == generation => *transition_id,
                    _ => {
                        let transition_id = self.next_trust_transition_id();
                        self.pending_trust_transition = Some(PendingTrustTransition {
                            generation,
                            transition_id,
                            decision: TrustLifecycleDecision::Gate,
                        });
                        transition_id
                    }
                };
                self.send_actions(vec![AppAction::AuthoritativeDeviceTrustChanged {
                    generation,
                    transition_id,
                    trust,
                }])
                .await;
                return;
            }
            TrustLifecycleDecision::Promote => {}
        }
        if !matches!(trust, koushi_state::CurrentDeviceTrustState::Verified) {
            self.send_actions(vec![AppAction::CurrentDeviceTrustChanged(trust)])
                .await;
            return;
        }
        if matches!(
            self.pending_trust_transition,
            Some(PendingTrustTransition {
                generation: pending_generation,
                decision: TrustLifecycleDecision::Promote,
                ..
            }) if pending_generation == generation
        ) {
            return;
        }
        let (Some(session), Some(key_id)) = (self.session.clone(), self.session_key_id.clone())
        else {
            return;
        };
        if self.persist_session(&session, &key_id).await.is_err() {
            self.send_actions(vec![AppAction::SessionPersistenceFailed {
                message: "session persistence failed".to_owned(),
            }])
            .await;
            return;
        }
        record_verification_admission_event(
            verification_admission_event("trust_persisted", generation, 0).field(
                DiagnosticField::token("trust", current_device_trust_token(trust)),
            ),
        );
        self.provisional_persistable = None;
        let transition_id = self.next_trust_transition_id();
        self.pending_trust_transition = Some(PendingTrustTransition {
            generation,
            transition_id,
            decision: TrustLifecycleDecision::Promote,
        });
        let restricted_was_active = self.provisional_encryption_sync.is_some();
        self.stop_provisional_encryption_sync().await;
        record_verification_admission_event(verification_admission_event(
            if restricted_was_active {
                "provisional_encryption_sync_stopped"
            } else {
                "provisional_encryption_sync_skipped"
            },
            generation,
            transition_id,
        ));
        record_verification_admission_event(verification_admission_event(
            "ready_projection_dispatched",
            generation,
            transition_id,
        ));
        self.send_actions(vec![AppAction::AuthoritativeDeviceTrustChanged {
            generation,
            transition_id,
            trust: koushi_state::CurrentDeviceTrustState::Verified,
        }])
        .await;
    }

    pub(super) async fn cancel_pending_trust_promotion(&mut self) {
        if matches!(
            self.pending_trust_transition,
            Some(PendingTrustTransition {
                decision: TrustLifecycleDecision::Promote,
                ..
            })
        ) {
            self.pending_trust_transition = None;
        }
    }

    pub(super) fn next_trust_transition_id(&mut self) -> u64 {
        self.next_trust_transition_id = self.next_trust_transition_id.wrapping_add(1);
        self.next_trust_transition_id
    }

    pub(super) async fn discover_verification_methods(&mut self, generation: u64) {
        self.cancel_verification_method_discovery_admission_timeout()
            .await;
        if let Some(owned) = self.verification_method_discovery_task.take() {
            owned.task.abort();
            let _ = owned.task.await;
            record_verification_method_discovery_event(verification_method_discovery_event(
                "cancelled",
                owned.generation,
                owned.serial,
            ));
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        self.verification_method_discovery_failed = false;
        self.verification_method_discovery_serial =
            self.verification_method_discovery_serial.wrapping_add(1);
        let serial = self.verification_method_discovery_serial;
        let tx = self.self_tx.clone();
        record_verification_method_discovery_event(verification_method_discovery_event(
            "started", generation, serial,
        ));
        let task = executor::spawn(async move {
            let started_at = Instant::now();
            let result = wait_for_verification_method_discovery(
                VERIFICATION_METHOD_DISCOVERY_TIMEOUT,
                koushi_sdk::discover_current_session_verification_methods(&session),
            )
            .await;
            let outcome = match &result {
                VerificationMethodDiscoveryResult::Discovered(_) => "success",
                VerificationMethodDiscoveryResult::Failed(_) => "failed",
            };
            record_verification_method_discovery_event(
                verification_method_discovery_event("finished", generation, serial)
                    .field(DiagnosticField::token("outcome", outcome))
                    .field(DiagnosticField::milliseconds(
                        "elapsed_ms",
                        started_at.elapsed().as_millis(),
                    )),
            );
            let _ = tx
                .send(AccountMessage::VerificationMethodsDiscovered {
                    generation,
                    serial,
                    result,
                })
                .await;
        });
        self.verification_method_discovery_task = Some(OwnedVerificationMethodDiscoveryTask {
            generation,
            serial,
            task,
        });
    }

    pub(super) async fn arm_verification_method_discovery_admission_timeout(
        &mut self,
        generation: u64,
    ) {
        if self.verification_method_discovery_admission_task.is_some() {
            return;
        }
        let serial = self.verification_method_discovery_serial;
        let tx = self.self_tx.clone();
        self.verification_method_discovery_admission_task = Some(executor::spawn(async move {
            executor::sleep(VERIFICATION_METHOD_DISCOVERY_ADMISSION_TIMEOUT).await;
            let _ = tx
                .send(
                    AccountMessage::VerificationMethodDiscoveryAdmissionTimedOut {
                        generation,
                        serial,
                    },
                )
                .await;
        }));
    }

    pub(super) async fn cancel_verification_method_discovery_admission_timeout(&mut self) {
        if let Some(task) = self.verification_method_discovery_admission_task.take() {
            task.abort();
            let _ = task.await;
        }
    }

    pub(super) async fn handle_trust_projection_applied(
        &mut self,
        generation: u64,
        transition_id: u64,
        ready: bool,
        locked: bool,
    ) {
        let Some(pending) = self.pending_trust_transition.as_ref() else {
            return;
        };
        if generation != self.trust_generation {
            return;
        }
        if !trust_projection_ack_matches(pending, generation, transition_id, ready, locked) {
            record_verification_admission_event(verification_admission_event(
                "trust_projection_ack_mismatch",
                generation,
                transition_id,
            ));
            // An acknowledgement for this exact transition proves that its
            // projection did not settle the reducer. It is obsolete whether
            // the next explicit recheck demand arrived just before or just
            // after this message, so never leave it blocking future queries.
            self.pending_trust_transition = None;
            if self.trust_recheck_pending {
                self.start_authoritative_trust_recheck_if_idle(false);
            }
            return;
        }
        let decision = pending.decision;
        self.pending_trust_transition = None;
        self.trust_recheck_pending = false;
        if decision == TrustLifecycleDecision::Gate {
            record(
                DiagnosticEvent::new(DiagnosticLevel::Warn, "core.session_promotion", "changed")
                    .field(DiagnosticField::token("state", "gated")),
            );
            self.record_lifecycle_probe("gate_projection_ack");
            self.stop_normal_runtime_children().await;
            self.session_promoted = false;
            if let Some(session) = self.session.clone() {
                if session.current_device_trust()
                    == koushi_state::CurrentDeviceTrustState::Unverified
                {
                    self.arm_verification_method_discovery_admission_timeout(generation)
                        .await;
                }
                self.start_provisional_encryption_sync(session.clone(), generation, transition_id);
                if self.provisional_encryption_sync_ready
                    && session.current_device_trust()
                        == koushi_state::CurrentDeviceTrustState::Unverified
                {
                    self.discover_verification_methods(generation).await;
                }
            }
            return;
        }
        self.record_lifecycle_probe("ready_projection_ack");
        let Some(session) = self.session.clone() else {
            return;
        };
        debug_assert!(
            self.provisional_encryption_sync.is_none(),
            "normal sync cannot start before restricted sync ownership is released"
        );
        self.start_incoming_verification_observer(session.clone())
            .await;
        self.spawn_sync_actor(session.clone()).await;
        record_verification_admission_event(verification_admission_event(
            "normal_sync_started",
            generation,
            transition_id,
        ));
        self.spawn_account_hydration(session.clone());
        self.start_active_session_account_management_discovery(session.clone())
            .await;
        self.start_recovery_observer(session.clone());
        self.start_session_change_observer(session.clone());
        self.session_promoted = true;
        record(
            DiagnosticEvent::new(DiagnosticLevel::Info, "core.session_promotion", "changed")
                .field(DiagnosticField::token("state", "promoted")),
        );
        self.start_secure_backup_observer(session.clone());
        for event in std::mem::take(&mut self.pending_ready_events) {
            self.emit(event);
        }
        if let Some(account_epoch) = self.sliding_sync_revalidation_pending {
            self.start_sliding_sync_revalidation(account_epoch).await;
        }
        if let Some(pending) = self.pending_recovery_completion.take()
            && pending.generation == generation
        {
            record_recovery_verification_event(
                recovery_verification_event("verified_projection_applied", pending.flow_id)
                    .field(DiagnosticField::count("generation", pending.generation))
                    .field(DiagnosticField::request_id(
                        "request_id",
                        pending.request_id.connection_id.0,
                        pending.request_id.sequence,
                    )),
            );
            self.stop_recovery_trust_settlement_task().await;
            self.complete_recovery_after_verified(pending.request_id, pending.account_key, session)
                .await;
        }
    }
}

#[cfg(test)]
mod tests;
