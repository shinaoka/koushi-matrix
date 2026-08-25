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
#[cfg(feature = "qa-bin")]
use koushi_state::VerificationTarget;
use koushi_state::{
    AppAction, E2eeRecoveryState, RecoveryMethod, SessionInfo, TrustOperationFailureKind,
};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::event::{AccountEvent, CoreEvent};
use crate::executor;
use crate::failure::CoreFailure;
use crate::ids::{AccountKey, RequestId};
use crate::store::account_key_from_info;

use super::actor::{AccountActor, AccountMessage, current_epoch_ms};
use super::recovery_backup::{
    classify_e2ee_trust_error, record_recovery_verification_event, recovery_verification_event,
};
use super::verification::send_observer_output_until_stopped;

const VERIFICATION_METHOD_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(30);

const CURRENT_SESSION_STATUS_TIMEOUT: Duration = Duration::from_secs(15);

pub(super) fn record_verification_admission_event(event: DiagnosticEvent) {
    koushi_diagnostics::record(event);
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
    koushi_diagnostics::record(event);
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

#[cfg(feature = "qa-bin")]
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
    discovery_task_active: bool,
    discovery_failed: bool,
) -> bool {
    !session_promoted
        && (discovery_task_active || discovery_failed)
        && matches!(
            trust,
            Some(koushi_state::CurrentDeviceTrustState::Unverified)
        )
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
        self.start_recovery_observer(session.clone());
        let trust_at_promotion = session.current_device_trust();
        self.start_session_change_observer(session.clone());
        self.session_promoted = true;
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
        self.current_session_status_task = Some(executor::spawn(async move {
            let result = match executor::timeout(
                CURRENT_SESSION_STATUS_TIMEOUT,
                session.inspect_current_session(),
            )
            .await
            {
                Ok(Ok(inspection)) => Ok(inspection),
                Ok(Err(
                    koushi_sdk::MatrixCurrentSessionInspectionError::Unavailable
                    | koushi_sdk::MatrixCurrentSessionInspectionError::CurrentDeviceMissing,
                )) => Err(koushi_state::CurrentSessionStatusFailureKind::Unavailable),
                Ok(Err(
                    koushi_sdk::MatrixCurrentSessionInspectionError::DeviceRequest
                    | koushi_sdk::MatrixCurrentSessionInspectionError::IdentityRequest,
                )) => Err(koushi_state::CurrentSessionStatusFailureKind::Sdk),
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
        let checked_at_ms = current_epoch_ms();
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
                        let first = !callback_first_response_seen.swap(true, Ordering::AcqRel);
                        callback_failure_reported.store(false, Ordering::Release);
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
            self.record_lifecycle_probe("gate_projection_ack");
            self.stop_normal_runtime_children().await;
            self.session_promoted = false;
            if let Some(session) = self.session.clone() {
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
        self.start_recovery_observer(session.clone());
        self.start_session_change_observer(session.clone());
        self.session_promoted = true;
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
mod tests {
    use std::{sync::Arc, time::Duration};

    use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel};
    #[cfg(feature = "qa-bin")]
    use koushi_sdk::MatrixClientSession;
    #[cfg(feature = "qa-bin")]
    use koushi_state::VerificationTarget;
    use koushi_state::{AppAction, SessionInfo};
    #[cfg(feature = "qa-bin")]
    use matrix_sdk::test_utils::mocks::MatrixMockServer;

    use tokio::sync::{broadcast, mpsc, oneshot};

    #[cfg(feature = "qa-bin")]
    use super::refresh_device_keys_and_assert_known;
    use super::{
        PendingTrustTransition, TrustLifecycleDecision, VerificationMethodDiscoveryResult,
        active_own_user_sas_flow_for_provisional_encryption_sync,
        begin_provisional_encryption_sync_cursor_attempt, current_session_status_completion_action,
        current_session_status_observed_non_verified_trust, current_session_status_settled_event,
        first_provisional_encryption_sync_is_current, method_discovery_is_current,
        own_user_sas_recheck_is_current, record_verification_admission_event,
        record_verification_method_discovery_event, recovery_sync_should_resume,
        retry_should_restart_method_discovery, run_recovery_state_observation,
        should_discover_verification_methods, trust_lifecycle_decision,
        trust_projection_ack_matches, unknown_verification_gate,
        verification_method_discovery_event, wait_for_verification_method_discovery,
    };
    use crate::account::actor::AccountMessage;
    use crate::account::test_support::{
        KeyQueryControl, acknowledge_next_verified_projection,
        consume_initial_unknown_trust_projection, inspect_session_runtime, inspect_sync_owners,
        login_gated_actor, login_gated_actor_at, recv_account_action_with_sliding_sync_effects,
        spawn_named_quarantine_password_server_with_controls,
    };

    use crate::event::{AccountEvent, CoreEvent};
    use crate::executor;

    use crate::ids::AccountKey;

    use futures_util::stream;

    fn session_status_info() -> SessionInfo {
        SessionInfo {
            homeserver: "https://private.example.test".to_owned(),
            user_id: "@private:example.test".to_owned(),
            device_id: "PRIVATE-DEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::OAuth,
        }
    }

    fn verified_session_inspection() -> koushi_sdk::MatrixCurrentSessionInspection {
        koushi_sdk::MatrixCurrentSessionInspection {
            device_display_name: Some("Private Device Name".to_owned()),
            verification: koushi_state::CurrentDeviceTrustState::Verified,
            is_cross_signed_by_owner: true,
            own_identity_verification: koushi_state::OwnIdentityVerification::Verified,
            key_backup: koushi_state::CurrentSessionBackupState::Ready,
        }
    }

    #[test]
    fn secure_backup_queue_latch_follows_authoritative_gate_lifecycle() {
        let inspection_start = crate::account::test_source::item_body(
            include_str!("recovery_backup.rs"),
            "fn start_secure_backup_inspection",
        );
        assert!(
            !inspection_start.contains("set_secure_backup_send_admitted(false)"),
            "a periodic health inspection must preserve established admission"
        );

        let state_change = crate::account::test_source::item_body(
            include_str!("recovery_backup.rs"),
            "async fn handle_secure_backup_state_changed",
        );
        assert!(
            state_change.contains("set_secure_backup_send_admitted(false)"),
            "loss of local backup or recovery readiness must close admission"
        );

        let teardown = crate::account::test_source::item_body(
            include_str!("runtime_children.rs"),
            "async fn stop_current_session_runtime",
        );
        assert!(
            teardown.contains("set_secure_backup_send_admitted(false)"),
            "runtime teardown must close admission"
        );

        let completion = crate::account::test_source::item_body(
            include_str!("recovery_backup.rs"),
            "async fn finish_secure_backup_inspection",
        );
        assert!(
            completion.contains("set_secure_backup_send_admitted(admitted)"),
            "the generation-fenced completion must project operational backup authority"
        );
    }

    #[test]
    fn session_status_completion_requires_request_and_session_generation_fences() {
        let info = session_status_info();
        for (active_request, active_generation, promoted) in
            [(Some(8), 4, true), (Some(7), 5, true), (Some(7), 4, false)]
        {
            assert!(
                current_session_status_completion_action(
                    active_request,
                    active_generation,
                    promoted,
                    Some(&info),
                    7,
                    4,
                    koushi_state::CurrentSessionSyncState::Running,
                    Ok(verified_session_inspection()),
                    123,
                )
                .is_none(),
                "stale request, stale generation, and cleared sessions must all be rejected"
            );
        }
    }

    #[test]
    fn session_status_non_verified_observation_routes_to_the_trust_gate() {
        for trust in [
            koushi_state::CurrentDeviceTrustState::Unknown,
            koushi_state::CurrentDeviceTrustState::Unverified,
        ] {
            let mut inspection = verified_session_inspection();
            inspection.verification = trust;
            assert_eq!(
                current_session_status_observed_non_verified_trust(&Ok(inspection)),
                Some(trust)
            );
        }
        assert_eq!(
            current_session_status_observed_non_verified_trust(&Ok(verified_session_inspection())),
            None
        );
    }

    #[test]
    fn session_status_sdk_failure_projects_coarse_failed_action() {
        let info = session_status_info();
        assert_eq!(
            current_session_status_completion_action(
                Some(7),
                4,
                true,
                Some(&info),
                7,
                4,
                koushi_state::CurrentSessionSyncState::Error,
                Err(koushi_state::CurrentSessionStatusFailureKind::Sdk),
                123,
            ),
            Some(AppAction::CurrentSessionStatusRefreshFailed {
                request_id: 7,
                kind: koushi_state::CurrentSessionStatusFailureKind::Sdk,
                checked_at_ms: 123,
            })
        );
    }

    #[test]
    fn session_status_success_uses_durable_auth_sync_and_sdk_trust_facts() {
        let info = session_status_info();
        let Some(AppAction::CurrentSessionStatusRefreshed {
            request_id,
            details,
        }) = current_session_status_completion_action(
            Some(7),
            4,
            true,
            Some(&info),
            7,
            4,
            koushi_state::CurrentSessionSyncState::Running,
            Ok(verified_session_inspection()),
            123,
        )
        else {
            panic!("current completion should project ready details");
        };
        assert_eq!(request_id, 7);
        assert_eq!(details.device_id, "PRIVATE-DEVICE");
        assert_eq!(
            details.authentication_method,
            koushi_state::SessionAuthenticationMethod::OAuth
        );
        assert_eq!(
            details.sync_state,
            koushi_state::CurrentSessionSyncState::Running
        );
        assert_eq!(
            details.verification,
            koushi_state::CurrentDeviceTrustState::Verified
        );
    }

    #[test]
    fn session_status_diagnostics_expose_only_coarse_result_and_elapsed_time() {
        let info = session_status_info();
        let action = current_session_status_completion_action(
            Some(7),
            4,
            true,
            Some(&info),
            7,
            4,
            koushi_state::CurrentSessionSyncState::Running,
            Ok(verified_session_inspection()),
            123,
        )
        .expect("current completion");
        let formatted = koushi_diagnostics::format_event(&current_session_status_settled_event(
            &action,
            Duration::from_millis(9),
        ));
        assert_eq!(
            formatted,
            "stage=refresh_settled elapsed_ms=9 result=ready verdict=verified"
        );
        for private in [
            "private.example.test",
            "@private:example.test",
            "PRIVATE-DEVICE",
            "Private Device Name",
        ] {
            assert!(!formatted.contains(private));
        }
    }

    #[test]
    fn session_status_refresh_task_is_cancelled_with_the_session_runtime() {
        let shutdown = crate::account::test_source::item_body(
            include_str!("runtime_children.rs"),
            "async fn stop_current_session_runtime",
        );
        assert!(
            shutdown.contains("cancel_current_session_status_refresh().await"),
            "logout, switch, and shutdown must abort the actor-owned refresh task"
        );
    }

    #[test]
    fn method_discovery_rejects_stale_generation_serial_and_missing_session() {
        assert!(method_discovery_is_current(4, 4, 9, 9, true));
        assert!(!method_discovery_is_current(3, 4, 9, 9, true));
        assert!(!method_discovery_is_current(4, 4, 8, 9, true));
        assert!(!method_discovery_is_current(4, 4, 9, 9, false));
    }

    #[tokio::test]
    async fn verification_method_discovery_times_out_pending_sdk_work() {
        let result = wait_for_verification_method_discovery(
            Duration::from_millis(1),
            std::future::pending(),
        )
        .await;

        assert_eq!(
            result,
            VerificationMethodDiscoveryResult::Failed(
                koushi_state::VerificationGateFailureKind::Timeout
            )
        );
    }

    #[tokio::test]
    async fn verification_method_discovery_maps_known_and_unknown_gate_results() {
        let known = koushi_state::VerificationGateState {
            methods: vec![koushi_state::VerificationMethodCapability::RecoveryKey],
            account_kind: koushi_state::VerificationAccountKind::ExistingIdentity,
            failure: None,
        };
        assert_eq!(
            wait_for_verification_method_discovery(
                Duration::from_secs(1),
                std::future::ready(known.clone()),
            )
            .await,
            VerificationMethodDiscoveryResult::Discovered(known)
        );
        assert_eq!(
            wait_for_verification_method_discovery(
                Duration::from_secs(1),
                std::future::ready(unknown_verification_gate()),
            )
            .await,
            VerificationMethodDiscoveryResult::Failed(
                koushi_state::VerificationGateFailureKind::Sdk
            )
        );
    }

    #[test]
    fn verification_method_discovery_retry_restarts_only_for_unverified_provisional_session() {
        assert!(retry_should_restart_method_discovery(
            false,
            Some(koushi_state::CurrentDeviceTrustState::Unverified),
            true,
            false,
        ));
        assert!(retry_should_restart_method_discovery(
            false,
            Some(koushi_state::CurrentDeviceTrustState::Unverified),
            false,
            true,
        ));
        assert!(!retry_should_restart_method_discovery(
            true,
            Some(koushi_state::CurrentDeviceTrustState::Unverified),
            true,
            false,
        ));
        assert!(!retry_should_restart_method_discovery(
            false,
            Some(koushi_state::CurrentDeviceTrustState::Unknown),
            true,
            false,
        ));
        assert!(!retry_should_restart_method_discovery(
            false,
            Some(koushi_state::CurrentDeviceTrustState::Verified),
            true,
            false,
        ));
        assert!(!retry_should_restart_method_discovery(
            false,
            Some(koushi_state::CurrentDeviceTrustState::Unverified),
            false,
            false,
        ));
        assert!(!retry_should_restart_method_discovery(
            false, None, true, true,
        ));
    }

    #[test]
    fn provisional_encryption_sync_attempt_starts_only_without_an_active_owner() {
        assert!(begin_provisional_encryption_sync_cursor_attempt(false));
        assert!(!begin_provisional_encryption_sync_cursor_attempt(true));
    }

    #[test]
    fn provisional_pre_first_response_failure_retries_under_the_same_owner() {
        let source = include_str!("trust_gate.rs");
        let owner = source
            .split("fn start_provisional_encryption_sync")
            .nth(1)
            .expect("provisional owner")
            .split("pub(super) async fn stop_provisional_encryption_sync")
            .next()
            .expect("provisional owner body");
        let branch_start = owner
            .find("if !first_response_seen.load(Ordering::Acquire)")
            .expect("pre-first-response failure branch");
        let retry_sleep = owner[branch_start..]
            .find("executor::sleep(Duration::from_millis(250)).await;")
            .map(|offset| branch_start + offset)
            .expect("bounded retry backoff");
        let retry_continue = owner[retry_sleep..]
            .find("continue;")
            .map(|offset| retry_sleep + offset)
            .expect("retry continues under the same owner");
        let post_first_failure = owner[retry_continue..]
            .find("AccountMessage::ProvisionalEncryptionSyncFailed")
            .map(|offset| retry_continue + offset)
            .expect("post-first-response failure branch");
        assert!(branch_start < retry_sleep);
        assert!(retry_sleep < retry_continue);
        assert!(retry_continue < post_first_failure);
    }

    #[test]
    fn recovery_resumes_provisional_encryption_sync_only_for_the_current_gated_session() {
        assert!(recovery_sync_should_resume(4, 4, false, false));
        assert!(!recovery_sync_should_resume(3, 4, false, false));
        assert!(!recovery_sync_should_resume(4, 4, true, false));
        assert!(!recovery_sync_should_resume(4, 4, false, true));
    }

    #[test]
    fn first_provisional_encryption_sync_ack_rejects_stale_torn_down_and_promoted_sessions() {
        assert!(first_provisional_encryption_sync_is_current(
            4, 4, true, false
        ));
        assert!(!first_provisional_encryption_sync_is_current(
            3, 4, true, false
        ));
        assert!(!first_provisional_encryption_sync_is_current(
            4, 4, false, false
        ));
        assert!(!first_provisional_encryption_sync_is_current(
            4, 4, true, true
        ));
        assert_eq!(unknown_verification_gate().methods, Vec::new());
        assert_eq!(
            unknown_verification_gate().account_kind,
            koushi_state::VerificationAccountKind::Unknown
        );
    }

    #[test]
    fn provisional_encryption_sync_rechecks_only_current_unstarted_own_user_flow() {
        assert!(own_user_sas_recheck_is_current(
            4, 4, true, false, true, false
        ));
        assert!(!own_user_sas_recheck_is_current(
            3, 4, true, false, true, false
        ));
        assert!(!own_user_sas_recheck_is_current(
            4, 4, false, false, true, false
        ));
        assert!(!own_user_sas_recheck_is_current(
            4, 4, true, true, true, false
        ));
        assert!(!own_user_sas_recheck_is_current(
            4, 4, true, false, false, false
        ));
        assert!(!own_user_sas_recheck_is_current(
            4, 4, true, false, true, true
        ));
    }

    #[test]
    fn provisional_encryption_sync_diagnostics_require_current_own_user_flow() {
        assert_eq!(
            active_own_user_sas_flow_for_provisional_encryption_sync(4, 4, true, false, Some(73)),
            Some(73)
        );
        assert_eq!(
            active_own_user_sas_flow_for_provisional_encryption_sync(3, 4, true, false, Some(73)),
            None
        );
        assert_eq!(
            active_own_user_sas_flow_for_provisional_encryption_sync(4, 4, false, false, Some(73)),
            None
        );
        assert_eq!(
            active_own_user_sas_flow_for_provisional_encryption_sync(4, 4, true, true, Some(73)),
            None
        );
        assert_eq!(
            active_own_user_sas_flow_for_provisional_encryption_sync(4, 4, true, false, None),
            None
        );
    }

    #[test]
    fn unknown_trust_does_not_discover_verification_methods() {
        assert!(!should_discover_verification_methods(
            koushi_state::CurrentDeviceTrustState::Unknown
        ));
        assert!(should_discover_verification_methods(
            koushi_state::CurrentDeviceTrustState::Unverified
        ));
        assert!(!should_discover_verification_methods(
            koushi_state::CurrentDeviceTrustState::Verified
        ));
    }

    #[test]
    fn verification_admission_diagnostic_records_without_stderr() {
        let output = std::process::Command::new(
            std::env::current_exe().expect("current test executable should be available"),
        )
        .args([
            "--exact",
            "account::trust_gate::tests::verification_admission_diagnostic_child",
            "--ignored",
            "--nocapture",
        ])
        .output()
        .expect("verification admission diagnostic child should run");
        assert!(output.status.success(), "child failed: {output:?}");

        let stderr = String::from_utf8(output.stderr).expect("child stderr should be utf8");
        assert!(
            stderr.is_empty(),
            "private diagnostics stay in the buffer only"
        );
        assert!(!stderr.contains('@'));
        assert!(!stderr.contains("access_token"));

        let stdout = String::from_utf8(output.stdout).expect("child stdout should be utf8");
        let snapshot: serde_json::Value = serde_json::from_str(
            stdout
                .lines()
                .find(|line| line.starts_with('{'))
                .expect("child should print one JSON snapshot"),
        )
        .expect("child output should be a JSON snapshot");
        assert!(snapshot["records"].as_array().is_some_and(|records| {
            records.iter().any(|record| {
                record["event"]["source"] == "core.verification_admission"
                    && record["event"]["stage"] == "trust_read_finished"
            })
        }));
    }

    #[test]
    #[ignore]
    fn verification_admission_diagnostic_child() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        record_verification_admission_event(
            DiagnosticEvent::new(
                DiagnosticLevel::Info,
                "core.verification_admission",
                "trust_read_finished",
            )
            .field(DiagnosticField::count("generation", 7))
            .field(DiagnosticField::count("transition_id", 0))
            .field(DiagnosticField::token("trust", "verified"))
            .field(DiagnosticField::milliseconds("elapsed_ms", 42)),
        );
        println!(
            "{}",
            serde_json::to_string(&koushi_diagnostics::test_support::detail_snapshot())
                .expect("diagnostic snapshot should serialize")
        );
    }

    #[test]
    fn verification_method_discovery_diagnostic_records_without_stderr() {
        let output = std::process::Command::new(
            std::env::current_exe().expect("current test executable should be available"),
        )
        .args([
            "--exact",
            "account::trust_gate::tests::verification_method_discovery_diagnostic_child",
            "--ignored",
            "--nocapture",
        ])
        .output()
        .expect("verification method discovery diagnostic child should run");
        assert!(output.status.success(), "child failed: {output:?}");

        let stderr = String::from_utf8(output.stderr).expect("child stderr should be utf8");
        assert!(
            stderr.is_empty(),
            "private diagnostics stay in the buffer only"
        );

        let stdout = String::from_utf8(output.stdout).expect("child stdout should be utf8");
        let snapshot: serde_json::Value = serde_json::from_str(
            stdout
                .lines()
                .find(|line| line.starts_with('{'))
                .expect("child should print one JSON snapshot"),
        )
        .expect("child output should be a JSON snapshot");
        assert!(snapshot["records"].as_array().is_some_and(|records| {
            records.iter().any(|record| {
                record["event"]["source"] == "core.verification_method_discovery"
                    && record["event"]["stage"] == "finished"
            })
        }));
    }

    #[test]
    #[ignore]
    fn verification_method_discovery_diagnostic_child() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        record_verification_method_discovery_event(
            verification_method_discovery_event("finished", 7, 11)
                .field(DiagnosticField::token("outcome", "failed"))
                .field(DiagnosticField::milliseconds("elapsed_ms", 42)),
        );
        println!(
            "{}",
            serde_json::to_string(&koushi_diagnostics::test_support::detail_snapshot())
                .expect("diagnostic snapshot should serialize")
        );
    }

    #[test]
    fn trust_lifecycle_is_generation_safe_and_fail_closed() {
        use koushi_state::CurrentDeviceTrustState::{Unknown, Unverified, Verified};

        assert_eq!(
            trust_lifecycle_decision(4, 5, false, Verified),
            TrustLifecycleDecision::IgnoreStale
        );
        assert_eq!(
            trust_lifecycle_decision(5, 5, false, Unknown),
            TrustLifecycleDecision::StayGated
        );
        assert_eq!(
            trust_lifecycle_decision(5, 5, false, Unverified),
            TrustLifecycleDecision::StayGated
        );
        assert_eq!(
            trust_lifecycle_decision(5, 5, false, Verified),
            TrustLifecycleDecision::Promote
        );
        assert_eq!(
            trust_lifecycle_decision(5, 5, true, Unverified),
            TrustLifecycleDecision::Gate
        );
        assert_eq!(
            trust_lifecycle_decision(5, 5, true, Unknown),
            TrustLifecycleDecision::Gate
        );
    }

    #[tokio::test]
    async fn duplicate_gate_observations_reuse_one_projection_transition() {
        let (handle, mut action_rx) = login_gated_actor().await;
        consume_initial_unknown_trust_projection(&mut action_rx).await;
        handle
            .send(AccountMessage::CurrentDeviceTrustChanged {
                generation: 2,
                trust: koushi_state::CurrentDeviceTrustState::Verified,
            })
            .await;
        acknowledge_next_verified_projection(&handle, &mut action_rx).await;

        for _ in 0..2 {
            handle
                .send(AccountMessage::CurrentDeviceTrustChanged {
                    generation: 2,
                    trust: koushi_state::CurrentDeviceTrustState::Unverified,
                })
                .await;
        }
        let mut transitions = Vec::new();
        while transitions.len() < 2 {
            let actions = action_rx.recv().await.expect("gate projection action");
            if let [
                AppAction::AuthoritativeDeviceTrustChanged {
                    generation,
                    transition_id,
                    trust: koushi_state::CurrentDeviceTrustState::Unverified,
                },
            ] = actions.as_slice()
            {
                transitions.push((*generation, *transition_id));
            }
        }
        assert_eq!(transitions[0], transitions[1]);
        assert!(
            handle
                .send(AccountMessage::TrustProjectionApplied {
                    generation: transitions[0].0,
                    transition_id: transitions[0].1,
                    ready: false,
                    locked: false,
                })
                .await
        );
        executor::timeout(Duration::from_secs(1), async {
            loop {
                if inspect_session_runtime(&handle).await == (true, false, false, true) {
                    break;
                }
                executor::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("one gated acknowledgement must stop normal children");
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn authoritative_trust_recheck_completion_promotes_through_generation_gated_path() {
        let (handle, mut action_rx) = login_gated_actor().await;
        consume_initial_unknown_trust_projection(&mut action_rx).await;

        handle
            .send(AccountMessage::CurrentDeviceTrustRecheckFinished {
                generation: 2,
                result: Ok(koushi_state::CurrentDeviceTrustState::Verified),
            })
            .await;
        acknowledge_next_verified_projection(&handle, &mut action_rx).await;

        assert_eq!(
            inspect_session_runtime(&handle).await,
            (true, true, true, true)
        );
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn authoritative_trust_recheck_failure_settles_as_retryable_unknown_trust() {
        let (handle, mut action_rx) = login_gated_actor().await;
        consume_initial_unknown_trust_projection(&mut action_rx).await;

        handle
            .send(AccountMessage::CurrentDeviceTrustRecheckFinished {
                generation: 2,
                result: Err(koushi_sdk::CurrentDeviceTrustRecheckError::Sdk),
            })
            .await;

        while !matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::AuthoritativeDeviceTrustChanged {
                trust: koushi_state::CurrentDeviceTrustState::Unknown,
                ..
            }])
        ) {}
        assert_eq!(
            inspect_session_runtime(&handle).await,
            (true, false, false, true)
        );
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn network_trust_recheck_settlement_records_generation_without_authentication_lock() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        let (handle, mut action_rx) = login_gated_actor().await;
        consume_initial_unknown_trust_projection(&mut action_rx).await;
        handle
            .send(AccountMessage::CurrentDeviceTrustChanged {
                generation: 2,
                trust: koushi_state::CurrentDeviceTrustState::Verified,
            })
            .await;
        acknowledge_next_verified_projection(&handle, &mut action_rx).await;
        let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
            .records
            .len();

        handle
            .send(AccountMessage::CurrentDeviceTrustRecheckFinished {
                generation: 2,
                result: Err(koushi_sdk::CurrentDeviceTrustRecheckError::Network),
            })
            .await;
        assert_eq!(
            inspect_session_runtime(&handle).await,
            (true, true, true, true)
        );

        let expected =
            "stage=trust_recheck_finished_failed generation=2 transition_id=0 failure_kind=network";
        assert!(
            koushi_diagnostics::test_support::detail_snapshot().records[diagnostic_start..]
                .iter()
                .any(|record| koushi_diagnostics::format_event(&record.event) == expected),
            "missing exact trust-recheck settlement diagnostic: {expected}"
        );
        while let Ok(actions) = action_rx.try_recv() {
            assert!(
                !matches!(
                    actions.as_slice(),
                    [AppAction::SessionAuthenticationInvalidated { .. }]
                ),
                "network trust failure must not emit authentication invalidation"
            );
        }
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn authoritative_trust_recheck_stale_generation_cannot_promote_session() {
        let (handle, mut action_rx) = login_gated_actor().await;
        consume_initial_unknown_trust_projection(&mut action_rx).await;

        handle
            .send(AccountMessage::CurrentDeviceTrustRecheckFinished {
                generation: 1,
                result: Ok(koushi_state::CurrentDeviceTrustState::Verified),
            })
            .await;
        let runtime = inspect_session_runtime(&handle).await;

        while let Ok(actions) = action_rx.try_recv() {
            assert!(
                !matches!(
                    actions.as_slice(),
                    [AppAction::AuthoritativeDeviceTrustChanged {
                        trust: koushi_state::CurrentDeviceTrustState::Verified,
                        ..
                    }]
                ),
                "a stale recheck completion must not emit a verified projection"
            );
        }
        assert_eq!(runtime, (true, false, false, true));
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn explicit_trust_recheck_is_not_dropped_behind_pending_projection() {
        let (homeserver, query_control) = spawn_counting_quarantine_password_server();
        let (handle, mut action_rx) = login_gated_actor_at(homeserver).await;
        consume_initial_unknown_trust_projection(&mut action_rx).await;

        handle
            .send(AccountMessage::CurrentDeviceTrustChanged {
                generation: 2,
                trust: koushi_state::CurrentDeviceTrustState::Verified,
            })
            .await;
        let (generation, transition_id) = loop {
            let actions =
                recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx).await;
            if let [
                AppAction::AuthoritativeDeviceTrustChanged {
                    generation,
                    transition_id,
                    trust: koushi_state::CurrentDeviceTrustState::Verified,
                },
            ] = actions.as_slice()
            {
                break (*generation, *transition_id);
            }
        };

        let baseline = query_control
            .count
            .load(std::sync::atomic::Ordering::SeqCst);
        handle.send(AccountMessage::CheckCurrentDeviceTrust).await;
        handle
            .send(AccountMessage::TrustProjectionApplied {
                generation,
                transition_id,
                ready: false,
                locked: false,
            })
            .await;

        executor::timeout(Duration::from_secs(1), async {
            while query_control
                .count
                .load(std::sync::atomic::Ordering::SeqCst)
                == baseline
            {
                executor::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("a projection mismatch must replay the explicit reducer recheck");
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn projection_mismatch_before_explicit_recheck_does_not_block_later_query() {
        let (homeserver, query_control) = spawn_counting_quarantine_password_server();
        let (handle, mut action_rx) = login_gated_actor_at(homeserver).await;
        consume_initial_unknown_trust_projection(&mut action_rx).await;

        handle
            .send(AccountMessage::CurrentDeviceTrustChanged {
                generation: 2,
                trust: koushi_state::CurrentDeviceTrustState::Verified,
            })
            .await;
        let (generation, transition_id) = loop {
            let actions = action_rx.recv().await.expect("account actions");
            if let [
                AppAction::AuthoritativeDeviceTrustChanged {
                    generation,
                    transition_id,
                    trust: koushi_state::CurrentDeviceTrustState::Verified,
                },
            ] = actions.as_slice()
            {
                break (*generation, *transition_id);
            }
        };

        handle
            .send(AccountMessage::TrustProjectionApplied {
                generation,
                transition_id,
                ready: false,
                locked: false,
            })
            .await;
        let baseline = query_control
            .count
            .load(std::sync::atomic::Ordering::SeqCst);
        handle.send(AccountMessage::CheckCurrentDeviceTrust).await;

        executor::timeout(Duration::from_secs(1), async {
            while query_control
                .count
                .load(std::sync::atomic::Ordering::SeqCst)
                == baseline
            {
                executor::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("a prior projection mismatch must not block a later explicit recheck");
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn explicit_trust_recheck_arriving_in_flight_is_replayed_after_settlement() {
        let (homeserver, query_control) = spawn_counting_quarantine_password_server();
        let (handle, mut action_rx) = login_gated_actor_at(homeserver).await;
        consume_initial_unknown_trust_projection(&mut action_rx).await;
        query_control
            .hold
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let baseline = query_control
            .count
            .load(std::sync::atomic::Ordering::SeqCst);

        handle.send(AccountMessage::CheckCurrentDeviceTrust).await;
        executor::timeout(Duration::from_secs(1), async {
            while query_control
                .count
                .load(std::sync::atomic::Ordering::SeqCst)
                == baseline
            {
                executor::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("first authoritative query must start");

        handle.send(AccountMessage::CheckCurrentDeviceTrust).await;
        query_control
            .hold
            .store(false, std::sync::atomic::Ordering::SeqCst);

        executor::timeout(Duration::from_secs(1), async {
            while query_control
                .count
                .load(std::sync::atomic::Ordering::SeqCst)
                < baseline + 2
            {
                executor::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("an in-flight reducer recheck demand must replay after the first query settles");
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn verification_to_normal_sync_handoff_has_one_owner() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
            .records
            .len();
        let (handle, mut action_rx) = login_gated_actor().await;
        assert!(
            koushi_diagnostics::test_support::detail_snapshot().records[diagnostic_start..]
                .iter()
                .any(|record| {
                    record.event.source == "core.verification_admission"
                        && record.event.stage == "provisional_encryption_sync_started"
                }),
            "gated admission must diagnose restricted sync ownership start"
        );
        let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
        handle
            .send(AccountMessage::AttachLifecycleProbe { probe_tx })
            .await;
        assert_eq!(inspect_sync_owners(&handle).await, (true, false, false));
        handle
            .send(AccountMessage::CurrentDeviceTrustChanged {
                generation: 2,
                trust: koushi_state::CurrentDeviceTrustState::Verified,
            })
            .await;
        assert_eq!(
            inspect_sync_owners(&handle).await,
            (false, false, false),
            "restricted owner must stop before Ready projection acknowledgement"
        );
        assert_eq!(
            probe_rx.try_recv(),
            Ok("provisional_encryption_sync_terminated")
        );
        acknowledge_next_verified_projection(&handle, &mut action_rx).await;
        assert_eq!(
            inspect_sync_owners(&handle).await,
            (false, false, true),
            "normal sync must be the only owner after Ready acknowledgement"
        );
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    fn spawn_counting_quarantine_password_server() -> (String, std::sync::Arc<KeyQueryControl>) {
        let control = std::sync::Arc::new(KeyQueryControl::default());
        let homeserver = spawn_named_quarantine_password_server_with_controls(
            "@fixture-user:example.invalid",
            "FIXTUREDEVICE",
            None,
            Some(std::sync::Arc::clone(&control)),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
        );
        (homeserver, control)
    }

    #[test]
    fn stale_projection_ack_does_not_consume_pending_promotion() {
        let pending = PendingTrustTransition {
            generation: 7,
            transition_id: 42,
            decision: TrustLifecycleDecision::Promote,
        };
        assert!(!trust_projection_ack_matches(&pending, 7, 41, false, false));
        assert!(trust_projection_ack_matches(&pending, 7, 42, true, false));
    }

    #[test]
    fn provisional_verification_uses_encryption_sync_service() {
        let provisional_owner = crate::account::test_source::item_body(
            include_str!("trust_gate.rs"),
            "fn start_provisional_encryption_sync",
        );

        assert!(
            provisional_owner.contains("provisional_encryption_sync_loop"),
            "provisional verification must use EncryptionSyncService"
        );
        assert!(
            !provisional_owner.contains("restricted_verification_sync_once_with_token"),
            "provisional verification must never construct classic /sync"
        );
    }

    #[cfg(feature = "qa-bin")]
    #[test]
    fn qa_device_key_refresh_queries_before_asserting_the_exact_device() {
        let helper = crate::account::test_source::item_body(
            include_str!("trust_gate.rs"),
            "async fn refresh_device_keys_and_assert_known",
        );
        let query = helper
            .find("request_user_identity(&user_id)")
            .expect("QA checkpoint must perform an explicit /keys/query");
        let exact_device = helper
            .find("get_device(&user_id, &device_id)")
            .expect("QA checkpoint must require the exact device after refresh");

        assert!(query < exact_device);
        assert!(helper[exact_device..].contains(".ok_or(())?"));
    }

    #[cfg(feature = "qa-bin")]
    #[tokio::test]
    async fn qa_device_key_refresh_accepts_identityless_exact_device_and_rejects_missing_device() {
        let server = MatrixMockServer::new().await;
        server.mock_crypto_endpoints_preset().await;
        let (alice, bob) = server.set_up_alice_and_bob_for_encryption().await;
        let bob_target = VerificationTarget {
            user_id: bob.user_id().expect("synthetic Bob user").to_string(),
            device_id: bob.device_id().expect("synthetic Bob device").to_string(),
        };
        let session = MatrixClientSession::from_client_for_testing(
            alice,
            koushi_state::SessionInfo {
                homeserver: server.uri(),
                user_id: "@alice:example.org".to_owned(),
                device_id: "4L1C3".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            },
        );

        assert_eq!(
            refresh_device_keys_and_assert_known(&session, bob_target.clone()).await,
            Ok(())
        );
        assert_eq!(
            refresh_device_keys_and_assert_known(
                &session,
                VerificationTarget {
                    device_id: "MISSINGDEVICE".to_owned(),
                    ..bob_target
                },
            )
            .await,
            Err(())
        );
    }

    #[test]
    fn verification_method_discovery_completion_projects_without_awaiting_sender_task() {
        let actor_source = include_str!("actor.rs");
        let completion_arm = actor_source
            .split("AccountMessage::VerificationMethodsDiscovered")
            .nth(1)
            .expect("verification method discovery completion arm")
            .split("AccountMessage::RecoveryFinished")
            .next()
            .expect("recovery completion arm follows method discovery");

        assert!(
            !completion_arm.contains("owned.task.await"),
            "the completion arm is handling a message sent by the discovery task; awaiting that task before projection can leave the gate stuck in DiscoveringMethods"
        );
        assert!(
            completion_arm.contains("success_projected"),
            "successful discovery projection must be diagnosable after completion_received"
        );
    }

    /// Recovery-state observation must emit the reducer-legal state change
    /// once per Incomplete transition, even if the stream repeats that state
    /// before later becoming Enabled.
    #[tokio::test]
    async fn recovery_state_observer_deduplicates_repeated_incomplete() {
        let info = SessionInfo {
            homeserver: "https://example.test".to_owned(),
            user_id: "@alice:example.test".to_owned(),
            device_id: "DEVICE1".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        };
        let account_key = AccountKey(info.user_id.clone());
        let states = stream::iter([
            koushi_state::E2eeRecoveryState::Unknown,
            koushi_state::E2eeRecoveryState::Incomplete,
            koushi_state::E2eeRecoveryState::Incomplete,
            koushi_state::E2eeRecoveryState::Enabled,
            koushi_state::E2eeRecoveryState::Enabled,
        ]);
        let (action_tx, mut action_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = broadcast::channel(8);
        let (_stop_tx, stop_rx) = tokio::sync::oneshot::channel();

        run_recovery_state_observation(
            states,
            account_key.clone(),
            action_tx,
            event_tx,
            stop_rx,
            None,
        )
        .await;

        let first_actions = action_rx.recv().await.expect("first action batch");
        assert_eq!(
            first_actions,
            vec![AppAction::E2eeRecoveryStateChanged {
                state: koushi_state::E2eeRecoveryState::Incomplete,
                methods: vec![koushi_state::RecoveryMethod::RecoveryKey],
            }]
        );

        match event_rx.recv().await.expect("recovery event") {
            CoreEvent::Account(AccountEvent::RecoveryRequired {
                account_key: emitted_key,
            }) => {
                assert_eq!(emitted_key, account_key);
            }
            other => panic!("expected RecoveryRequired event, got {other:?}"),
        }

        let second_actions = action_rx.recv().await.expect("follow-up action batch");
        assert_eq!(
            second_actions,
            vec![AppAction::E2eeRecoveryStateChanged {
                state: koushi_state::E2eeRecoveryState::Enabled,
                methods: vec![koushi_state::RecoveryMethod::RecoveryKey],
            }]
        );

        assert!(
            matches!(
                action_rx.try_recv(),
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
            ),
            "repeated recovery states must not emit duplicate actions"
        );
        assert!(
            matches!(
                event_rx.recv().await,
                Err(tokio::sync::broadcast::error::RecvError::Closed)
            ),
            "repeated recovery states must not emit duplicate RecoveryRequired events"
        );
    }

    #[tokio::test]
    async fn recovery_state_observer_stop_interrupts_blocked_action_delivery() {
        let states = stream::iter([koushi_state::E2eeRecoveryState::Incomplete]);
        let (action_tx, mut action_rx) = mpsc::channel(1);
        action_tx
            .send(vec![AppAction::SessionLocked])
            .await
            .expect("fill the reducer action mailbox");
        let (event_tx, mut event_rx) = broadcast::channel(1);
        let (stop_tx, stop_rx) = oneshot::channel();
        let delivery_barrier = Arc::new(tokio::sync::Barrier::new(2));
        let mut task = executor::spawn(run_recovery_state_observation(
            states,
            AccountKey("@observer-stop:example.invalid".to_owned()),
            action_tx,
            event_tx,
            stop_rx,
            Some(delivery_barrier.clone()),
        ));

        delivery_barrier.wait().await;
        stop_tx.send(()).expect("request observer stop");
        match executor::timeout(Duration::from_millis(250), &mut task).await {
            Ok(joined) => joined.expect("recovery-state observer task"),
            Err(_) => {
                task.abort();
                let _ = task.await;
                panic!("stop must interrupt a blocked recovery action delivery");
            }
        }

        assert!(matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::SessionLocked])
        ));
        assert!(
            action_rx.try_recv().is_err(),
            "stop must discard only the blocked observer action"
        );
        assert!(
            event_rx.try_recv().is_err(),
            "a stopped Incomplete delivery must not emit RecoveryRequired"
        );
    }
}
