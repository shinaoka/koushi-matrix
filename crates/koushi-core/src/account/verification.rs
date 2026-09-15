//! `verification` ownership for AccountActor.

use std::{future::Future, sync::Arc, time::Duration};

use futures_util::StreamExt;
use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_sdk::MatrixClientSession;
use koushi_state::{
    AppAction, SasEmoji, TrustOperationFailureKind, VerificationCancelReason,
    VerificationFlowState, VerificationTarget,
};
use tokio::sync::{mpsc, oneshot};

use crate::executor;
use koushi_protocol::event::{CoreEvent, E2eeTrustEvent};
use koushi_protocol::failure::{CoreFailure, RecoveryFailureKind};
use koushi_protocol::ids::{AccountKey, RequestId, RuntimeConnectionId};

use super::actor::{AccountActor, AccountMessage};
use super::local_data_cleanup::record_device_cleanup_offer;
use super::recovery_backup::{
    classify_e2ee_trust_error, project_identity_reset_failed_event,
    project_reset_identity_completed, project_reset_identity_error,
};

const IDENTITY_RESET_AUTH_TIMEOUT: Duration = Duration::from_secs(300);

/// Diagnostic trigger for the activation summary recorded on session restore.
pub(super) const VERIFICATION_PROTECTION_SUMMARY_TRIGGER_RESTORE: &str = "restore";

const INCOMING_VERIFICATION_OBSERVER_JOIN_TIMEOUT: Duration = Duration::from_millis(100);

pub(super) const INCOMING_VERIFICATION_FLOW_ID_BASE: u64 = 1 << 63;

/// Record the private-data-free activation counters for the incoming
/// verification-request protections.
///
/// The counters show whether the protections against rare conditions (unknown
/// sender devices, repeated SAS start events, released deliveries) are still
/// exercised in practice; only counts are recorded, never identifiers or
/// content.
pub(super) fn record_incoming_verification_protection_summary(
    counters: &koushi_sdk::IncomingVerificationRequestProtectionCounters,
    trigger: &'static str,
) {
    record(
        DiagnosticEvent::new(
            DiagnosticLevel::Info,
            "core.verification_protection_summary",
            "summary",
        )
        .field(DiagnosticField::token("trigger", trigger))
        .field(DiagnosticField::count(
            "unknown_sender_deferred",
            counters.unknown_sender_deferred,
        ))
        .field(DiagnosticField::count("key_query_replays", counters.key_query_replays))
        .field(DiagnosticField::count(
            "released_deliveries",
            counters.released_deliveries,
        ))
        .field(DiagnosticField::count(
            "suppressed_sas_start_replays",
            counters.suppressed_sas_start_replays,
        )),
    );
}

pub(super) struct VerificationObservation {
    pub(super) stop_tx: oneshot::Sender<()>,
    pub(super) task: crate::executor::JoinHandle<()>,
}

pub(super) struct IncomingVerificationObservation {
    stop_tx: oneshot::Sender<()>,
    task: crate::executor::JoinHandle<()>,
    observer: koushi_sdk::MatrixIncomingVerificationRequestObserver,
}

/// Session-owned observers must race every blocking outbound delivery against
/// their stop signal. Normal operation still awaits reliable mailbox delivery;
/// shutdown drops only the not-yet-delivered observer output so its owner can
/// join the task without mailbox-backpressure deadlock.
pub(super) async fn send_observer_output_until_stopped<T>(
    sender: &mpsc::Sender<T>,
    message: T,
    stop_rx: &mut oneshot::Receiver<()>,
) -> bool {
    tokio::select! {
        biased;
        _ = stop_rx => false,
        result = sender.send(message) => result.is_ok(),
    }
}

async fn stop_incoming_verification_observation(observation: IncomingVerificationObservation) {
    stop_incoming_verification_observation_with_timeout(
        observation,
        INCOMING_VERIFICATION_OBSERVER_JOIN_TIMEOUT,
    )
    .await;
}

async fn stop_incoming_verification_observation_with_timeout(
    observation: IncomingVerificationObservation,
    timeout: Duration,
) {
    let IncomingVerificationObservation {
        stop_tx,
        mut task,
        mut observer,
    } = observation;
    let _ = stop_tx.send(());
    if executor::timeout(timeout, &mut task).await.is_err() {
        task.abort();
        let _ = task.await;
    }
    observer.shutdown().await;
}

pub(super) struct PendingVerificationRequest {
    request_id: RequestId,
    target: VerificationTarget,
    handle: koushi_sdk::MatrixVerificationRequestHandle,
}

pub(super) struct PendingSasVerification {
    request_id: RequestId,
    target: VerificationTarget,
    handle: koushi_sdk::MatrixSasVerificationHandle,
}

#[derive(Clone, Copy)]
pub(super) enum VerificationTerminal {
    Success,
    Cancelled(VerificationCancelReason),
    Failed(TrustOperationFailureKind),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SasVerificationWaitState {
    RecipientDevices,
    ToDeviceDelivery,
    RemoteAccept,
    SasStart,
    Mac,
    CrossSigningSettlement,
    NormalSyncResume,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SasAdoptionDecision {
    Adopt,
    Replay,
    Conflict,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IncomingVerificationRequestDecision {
    Adopt,
    Replay,
    Conflict,
}

#[derive(Clone, Copy)]
struct IncomingVerificationActivity<'a> {
    active_request: Option<(&'a VerificationTarget, &'a str)>,
    sas_active: bool,
    own_user_active: bool,
}

fn classify_incoming_verification_request(
    activity: IncomingVerificationActivity<'_>,
    incoming_target: &VerificationTarget,
    incoming_flow_id: &str,
) -> IncomingVerificationRequestDecision {
    if activity.own_user_active {
        return IncomingVerificationRequestDecision::Conflict;
    }

    match activity.active_request {
        Some((active_target, active_flow_id))
            if active_target == incoming_target && active_flow_id == incoming_flow_id =>
        {
            IncomingVerificationRequestDecision::Replay
        }
        Some(_) => IncomingVerificationRequestDecision::Conflict,
        None if activity.sas_active => IncomingVerificationRequestDecision::Conflict,
        None => IncomingVerificationRequestDecision::Adopt,
    }
}

pub(super) fn incoming_verification_request_is_current(
    message_generation: u64,
    current_generation: u64,
    has_session: bool,
) -> bool {
    has_session && message_generation == current_generation
}

fn classify_sas_adoption(
    active_flow_id: Option<u64>,
    incoming_flow_id: u64,
) -> SasAdoptionDecision {
    match active_flow_id {
        None => SasAdoptionDecision::Adopt,
        Some(active_flow_id) if active_flow_id == incoming_flow_id => SasAdoptionDecision::Replay,
        Some(_) => SasAdoptionDecision::Conflict,
    }
}

async fn resolve_sas_adoption<F, Fut>(
    active_flow_id: Option<u64>,
    incoming_flow_id: u64,
    reject_conflict: F,
) -> (SasAdoptionDecision, Option<bool>)
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = bool>,
{
    let decision = classify_sas_adoption(active_flow_id, incoming_flow_id);
    let rejection_succeeded = match decision {
        SasAdoptionDecision::Conflict => Some(reject_conflict().await),
        SasAdoptionDecision::Adopt | SasAdoptionDecision::Replay => None,
    };
    (decision, rejection_succeeded)
}

pub(super) fn sas_verification_event(stage: &'static str, flow_id: u64) -> DiagnosticEvent {
    DiagnosticEvent::new(DiagnosticLevel::Info, "core.sas_verification", stage)
        .field(DiagnosticField::count("flow_id", flow_id))
}

pub(super) fn record_sas_verification_event(event: DiagnosticEvent) {
    koushi_diagnostics::record(event);
}

fn verification_request_state_token(
    state: &koushi_sdk::MatrixVerificationRequestState,
) -> &'static str {
    match state {
        koushi_sdk::MatrixVerificationRequestState::Created => "created",
        koushi_sdk::MatrixVerificationRequestState::Requested => "requested",
        koushi_sdk::MatrixVerificationRequestState::Ready => "ready",
        koushi_sdk::MatrixVerificationRequestState::SasStarted(_) => "sas_started",
        koushi_sdk::MatrixVerificationRequestState::Done => "done",
        koushi_sdk::MatrixVerificationRequestState::Cancelled { .. } => "cancelled",
        koushi_sdk::MatrixVerificationRequestState::UnsupportedMethod => "unsupported_method",
    }
}

fn verification_cancel_kind_token(kind: koushi_sdk::MatrixVerificationCancelKind) -> &'static str {
    match kind {
        koushi_sdk::MatrixVerificationCancelKind::UnknownMethod => "unknown_method",
        koushi_sdk::MatrixVerificationCancelKind::KeyMismatch => "key_mismatch",
        koushi_sdk::MatrixVerificationCancelKind::User => "user",
        koushi_sdk::MatrixVerificationCancelKind::Timeout => "timeout",
        koushi_sdk::MatrixVerificationCancelKind::AcceptedElsewhere => "accepted_elsewhere",
        koushi_sdk::MatrixVerificationCancelKind::Other => "other",
    }
}

fn sas_state_token(state: &koushi_sdk::MatrixSasState) -> &'static str {
    match state {
        koushi_sdk::MatrixSasState::Created => "created",
        koushi_sdk::MatrixSasState::Started => "started",
        koushi_sdk::MatrixSasState::Accepted => "accepted",
        koushi_sdk::MatrixSasState::SasPresented { .. } => "sas_presented",
        koushi_sdk::MatrixSasState::Confirmed => "confirmed",
        koushi_sdk::MatrixSasState::Done => "done",
        koushi_sdk::MatrixSasState::Cancelled { .. } => "cancelled",
        koushi_sdk::MatrixSasState::UnsupportedShortAuth => "unsupported_short_auth",
    }
}

fn sas_waiting_for_token(waiting_for: SasVerificationWaitState) -> &'static str {
    match waiting_for {
        SasVerificationWaitState::RecipientDevices => "recipient_devices",
        SasVerificationWaitState::ToDeviceDelivery => "to_device_delivery",
        SasVerificationWaitState::RemoteAccept => "remote_accept",
        SasVerificationWaitState::SasStart => "sas_start",
        SasVerificationWaitState::Mac => "mac",
        SasVerificationWaitState::CrossSigningSettlement => "cross_signing_settlement",
        SasVerificationWaitState::NormalSyncResume => "normal_sync_resume",
    }
}

fn add_sas_waiting_for_field(
    event: DiagnosticEvent,
    waiting_for: SasVerificationWaitState,
) -> DiagnosticEvent {
    event.field(DiagnosticField::token(
        "waiting_for",
        sas_waiting_for_token(waiting_for),
    ))
}

fn sas_waiting_event(flow_id: u64, waiting_for: SasVerificationWaitState) -> DiagnosticEvent {
    add_sas_waiting_for_field(sas_verification_event("waiting", flow_id), waiting_for)
}

fn verification_request_waiting_for(
    state: &koushi_sdk::MatrixVerificationRequestState,
) -> Option<SasVerificationWaitState> {
    match state {
        koushi_sdk::MatrixVerificationRequestState::Created
        | koushi_sdk::MatrixVerificationRequestState::Requested => {
            Some(SasVerificationWaitState::RemoteAccept)
        }
        koushi_sdk::MatrixVerificationRequestState::Ready
        | koushi_sdk::MatrixVerificationRequestState::SasStarted(_) => {
            Some(SasVerificationWaitState::SasStart)
        }
        koushi_sdk::MatrixVerificationRequestState::Done
        | koushi_sdk::MatrixVerificationRequestState::Cancelled { .. }
        | koushi_sdk::MatrixVerificationRequestState::UnsupportedMethod => None,
    }
}

fn sas_state_waiting_for(state: &koushi_sdk::MatrixSasState) -> Option<SasVerificationWaitState> {
    match state {
        koushi_sdk::MatrixSasState::Created | koushi_sdk::MatrixSasState::Started => {
            Some(SasVerificationWaitState::RemoteAccept)
        }
        koushi_sdk::MatrixSasState::Accepted => Some(SasVerificationWaitState::SasStart),
        koushi_sdk::MatrixSasState::Confirmed => Some(SasVerificationWaitState::Mac),
        koushi_sdk::MatrixSasState::Done
        | koushi_sdk::MatrixSasState::Cancelled { .. }
        | koushi_sdk::MatrixSasState::UnsupportedShortAuth
        | koushi_sdk::MatrixSasState::SasPresented { .. } => None,
    }
}

fn sas_state_changed_event(flow_id: u64, state: &koushi_sdk::MatrixSasState) -> DiagnosticEvent {
    let mut event = sas_verification_event("sas_state_changed", flow_id)
        .field(DiagnosticField::token("state", sas_state_token(state)));
    if let Some(waiting_for) = sas_state_waiting_for(state) {
        event = add_sas_waiting_for_field(event, waiting_for);
    }
    if let koushi_sdk::MatrixSasState::Cancelled {
        kind,
        cancelled_by_us,
    } = state
    {
        event = event
            .field(DiagnosticField::token(
                "cancel_kind",
                verification_cancel_kind_token(*kind),
            ))
            .field(DiagnosticField::boolean(
                "cancelled_by_us",
                *cancelled_by_us,
            ));
    }
    event
}

fn trust_failure_token(kind: TrustOperationFailureKind) -> &'static str {
    match kind {
        TrustOperationFailureKind::Cancelled => "cancelled",
        TrustOperationFailureKind::Mismatch => "mismatch",
        TrustOperationFailureKind::InvalidPassphrase => "invalid_passphrase",
        TrustOperationFailureKind::Network => "network",
        TrustOperationFailureKind::Forbidden => "forbidden",
        TrustOperationFailureKind::Timeout => "timeout",
        TrustOperationFailureKind::Sdk => "sdk",
    }
}

pub(super) fn recovery_failure_token(kind: RecoveryFailureKind) -> &'static str {
    match kind {
        RecoveryFailureKind::InvalidRecoveryKey => "invalid_recovery_key",
        RecoveryFailureKind::Network => "network",
        RecoveryFailureKind::Server => "server",
        RecoveryFailureKind::Timeout => "timeout",
    }
}

fn verification_terminal_token(terminal: VerificationTerminal) -> &'static str {
    match terminal {
        VerificationTerminal::Success => "success",
        VerificationTerminal::Cancelled(_) => "cancelled",
        VerificationTerminal::Failed(_) => "failed",
    }
}

fn verification_cancel_reason_token(reason: VerificationCancelReason) -> &'static str {
    match reason {
        VerificationCancelReason::User => "user",
        VerificationCancelReason::Mismatch => "mismatch",
    }
}

fn sas_settled_event(
    flow_id: u64,
    terminal: VerificationTerminal,
    waiting_for: Option<SasVerificationWaitState>,
) -> DiagnosticEvent {
    let mut event = sas_verification_event("settled", flow_id).field(DiagnosticField::token(
        "terminal",
        verification_terminal_token(terminal),
    ));
    if let Some(waiting_for) = waiting_for {
        event = add_sas_waiting_for_field(event, waiting_for);
    }
    match terminal {
        VerificationTerminal::Success => {}
        VerificationTerminal::Cancelled(reason) => {
            event = event.field(DiagnosticField::token(
                "reason",
                verification_cancel_reason_token(reason),
            ));
        }
        VerificationTerminal::Failed(kind) => {
            event = event.field(DiagnosticField::token(
                "failure_kind",
                trust_failure_token(kind),
            ));
        }
    }
    event
}

fn sas_timeout_fired_event(
    flow_id: u64,
    waiting_for: Option<SasVerificationWaitState>,
) -> DiagnosticEvent {
    match waiting_for {
        Some(waiting_for) => add_sas_waiting_for_field(
            sas_verification_event("timeout_fired", flow_id),
            waiting_for,
        ),
        None => sas_verification_event("timeout_fired", flow_id),
    }
}

async fn run_own_user_sas_start<T, F>(
    flow_id: u64,
    source: &'static str,
    start: F,
) -> Result<Option<T>, koushi_sdk::E2eeTrustError>
where
    F: Future<Output = Result<Option<T>, koushi_sdk::E2eeTrustError>>,
{
    record_sas_verification_event(
        sas_verification_event("sas_start_attempted", flow_id)
            .field(DiagnosticField::token("source", source)),
    );
    let result = start.await;
    let mut event = sas_verification_event("sas_start_finished", flow_id)
        .field(DiagnosticField::token("source", source));
    event = match &result {
        Ok(Some(_)) => event.field(DiagnosticField::token("outcome", "started")),
        Ok(None) => event.field(DiagnosticField::token("outcome", "pending")),
        Err(error) => {
            let kind = classify_e2ee_trust_error(error);
            event
                .field(DiagnosticField::token("outcome", "failed"))
                .field(DiagnosticField::token(
                    "failure_kind",
                    trust_failure_token(kind),
                ))
        }
    };
    record_sas_verification_event(event);
    result
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub enum SyntheticVerificationTerminal {
    Success,
    Cancelled(VerificationCancelReason),
    Failed(TrustOperationFailureKind),
}

fn sas_projection_action(own_user_flow: bool, flow_id: u64, emojis: Vec<SasEmoji>) -> AppAction {
    if own_user_flow {
        AppAction::GateSasPresented { flow_id, emojis }
    } else {
        AppAction::VerificationSasPresented {
            request_id: flow_id,
            emojis,
        }
    }
}

pub(super) fn incoming_verification_request_id(sequence: u64) -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(0),
        sequence,
    }
}

impl AccountActor {
    pub(super) async fn start_incoming_verification_observer(
        &mut self,
        session: Arc<MatrixClientSession>,
    ) {
        self.incoming_verification_session_generation = self
            .incoming_verification_session_generation
            .wrapping_add(1);
        let generation = self.incoming_verification_session_generation;
        let (stop_tx, mut stop_rx) = oneshot::channel();
        let mut observer = koushi_sdk::observe_incoming_verification_requests(&session).await;
        let mut receiver = observer
            .take_receiver()
            .expect("incoming verification observer receiver is available once");
        let tx = self.self_tx.clone();
        let task = crate::executor::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    request = receiver.recv() => {
                        let Some(request) = request else { break };
                        let (target, handle) = request.into_parts();
                        if !send_observer_output_until_stopped(
                            &tx,
                            AccountMessage::IncomingVerificationRequest {
                                generation,
                                target,
                                handle,
                            },
                            &mut stop_rx,
                        )
                        .await
                        {
                            break;
                        }
                    }
                }
            }
        });
        self.incoming_verification_observer = Some(IncomingVerificationObservation {
            stop_tx,
            task,
            observer,
        });
    }

    pub(super) fn next_incoming_verification_request_id(&mut self) -> RequestId {
        let sequence = self.next_incoming_verification_sequence;
        self.next_incoming_verification_sequence = self
            .next_incoming_verification_sequence
            .checked_add(1)
            .unwrap_or(INCOMING_VERIFICATION_FLOW_ID_BASE);
        incoming_verification_request_id(sequence)
    }

    pub(super) async fn stop_incoming_verification_observer(&mut self) {
        self.incoming_verification_session_generation = self
            .incoming_verification_session_generation
            .wrapping_add(1);
        if let Some(observation) = self.incoming_verification_observer.take() {
            stop_incoming_verification_observation(observation).await;
        }
    }

    pub(super) async fn cancel_identity_reset_handle(&mut self) {
        self.identity_reset_flow_id = None;
        if let Some(task) = self.identity_reset_timeout_task.take() {
            task.abort();
        }
        if let Some(handle) = self.identity_reset_handle.take() {
            handle.cancel().await;
        }
    }

    pub(super) fn spawn_identity_reset_auth_timeout(&mut self, flow_id: u64) {
        if let Some(task) = self.identity_reset_timeout_task.take() {
            task.abort();
        }
        let tx = self.self_tx.clone();
        self.identity_reset_timeout_task = Some(executor::spawn(async move {
            executor::sleep(IDENTITY_RESET_AUTH_TIMEOUT).await;
            let _ = tx
                .send(AccountMessage::IdentityResetAuthTimedOut { flow_id })
                .await;
        }));
    }

    fn clear_identity_reset_handle_after_completion(&mut self) {
        self.identity_reset_flow_id = None;
        if let Some(task) = self.identity_reset_timeout_task.take() {
            task.abort();
        }
        self.identity_reset_handle = None;
    }

    async fn stop_verification_request_observer(&mut self) {
        if let Some(observation) = self.verification_request_observer.take() {
            let _ = observation.stop_tx.send(());
            let _ = observation.task.await;
        }
    }

    async fn stop_sas_verification_observer(&mut self) {
        if let Some(observation) = self.sas_verification_observer.take() {
            let _ = observation.stop_tx.send(());
            let _ = observation.task.await;
        }
    }

    fn record_sas_waiting_for(&mut self, flow_id: u64, waiting_for: SasVerificationWaitState) {
        if self.sas_waiting_for == Some((flow_id, waiting_for)) {
            return;
        }
        self.sas_waiting_for = Some((flow_id, waiting_for));
        record_sas_verification_event(sas_waiting_event(flow_id, waiting_for));
    }

    fn active_sas_waiting_for(&self, flow_id: u64) -> Option<SasVerificationWaitState> {
        self.sas_waiting_for
            .filter(|(active_flow_id, _)| *active_flow_id == flow_id)
            .map(|(_, waiting_for)| waiting_for)
    }

    fn clear_sas_waiting_for(&mut self, flow_id: u64) {
        if self
            .sas_waiting_for
            .is_some_and(|(active_flow_id, _)| active_flow_id == flow_id)
        {
            self.sas_waiting_for = None;
        }
    }

    pub(super) async fn cancel_verification_handles(&mut self) {
        self.stop_sas_timeout().await;
        self.stop_verification_request_observer().await;
        self.stop_sas_verification_observer().await;
        self.sas_waiting_for = None;
        if let Some(pending) = self.sas_verification.take() {
            let _ = koushi_sdk::cancel_sas_verification(&pending.handle).await;
        }
        if let Some(pending) = self.verification_request.take() {
            let _ = koushi_sdk::cancel_verification_request(&pending.handle).await;
        }
        if let Some((_, handle)) = self.own_user_verification.take() {
            let _ = koushi_sdk::cancel_own_user_sas_verification(&handle).await;
        }
    }

    pub(super) async fn handle_request_verification(
        &mut self,
        request_id: RequestId,
        target: VerificationTarget,
    ) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.send_actions(vec![AppAction::VerificationFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };

        self.cancel_verification_handles().await;
        match koushi_sdk::request_device_verification(&session, &target).await {
            Ok(handle) => {
                self.verification_request = Some(PendingVerificationRequest {
                    request_id,
                    target: target.clone(),
                    handle: handle.clone(),
                });
                self.observe_verification_request(request_id, target.clone(), handle.clone());
                self.send_actions(vec![AppAction::VerificationRequested {
                    request_id: request_id.sequence,
                    target: target.clone(),
                }])
                .await;
                self.emit_verification_progress(VerificationFlowState::Requested {
                    request_id: request_id.sequence,
                    target,
                });
                self.project_verification_request_state(request_id, handle.state())
                    .await;
            }
            Err(error) => {
                self.project_verification_failure(
                    request_id.sequence,
                    target,
                    classify_e2ee_trust_error(&error),
                )
                .await;
            }
        }
    }

    pub(super) async fn handle_start_own_user_sas(&mut self, request_id: RequestId, flow_id: u64) {
        let Some(session) = self.session.clone() else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        self.cancel_verification_handles().await;
        self.record_sas_waiting_for(flow_id, SasVerificationWaitState::RecipientDevices);
        let own_handle =
            match koushi_sdk::request_own_user_sas_verification(&session, flow_id).await {
                Ok(handle) => handle,
                Err(error) => {
                    let kind = classify_e2ee_trust_error(&error);
                    record_sas_verification_event(sas_settled_event(
                        flow_id,
                        VerificationTerminal::Failed(kind),
                        self.active_sas_waiting_for(flow_id),
                    ));
                    self.clear_sas_waiting_for(flow_id);
                    self.send_actions(vec![AppAction::VerificationFailed {
                        request_id: flow_id,
                        kind,
                    }])
                    .await;
                    return;
                }
            };
        if let Some(waiting_for) = verification_request_waiting_for(&own_handle.state()) {
            self.record_sas_waiting_for(flow_id, waiting_for);
        } else {
            self.record_sas_waiting_for(flow_id, SasVerificationWaitState::ToDeviceDelivery);
        }
        let sas = match run_own_user_sas_start(
            flow_id,
            "initial",
            koushi_sdk::start_own_user_sas_verification(&own_handle),
        )
        .await
        {
            Ok(Some(sas)) => sas,
            Ok(None) => {
                self.own_user_verification = Some((flow_id, own_handle));
                self.start_sas_timeout(flow_id);
                self.observe_own_user_verification(request_id, flow_id);
                return;
            }
            Err(error) => {
                let kind = classify_e2ee_trust_error(&error);
                record_sas_verification_event(sas_settled_event(
                    flow_id,
                    VerificationTerminal::Failed(kind),
                    self.active_sas_waiting_for(flow_id),
                ));
                self.clear_sas_waiting_for(flow_id);
                self.send_actions(vec![AppAction::VerificationFailed {
                    request_id: flow_id,
                    kind,
                }])
                .await;
                return;
            }
        };
        self.own_user_verification = Some((flow_id, own_handle));
        self.store_sas_verification(
            RequestId {
                connection_id: request_id.connection_id,
                sequence: flow_id,
            },
            VerificationTarget {
                user_id: "current-user".to_owned(),
                device_id: "eligible-device".to_owned(),
            },
            sas,
        )
        .await;
    }

    pub(super) async fn recheck_own_user_sas_after_sync(&mut self) {
        if self.sas_verification.is_some() {
            return;
        }
        let Some((flow_id, handle)) = self.own_user_verification.as_ref() else {
            return;
        };
        let state = handle.state();
        if !matches!(state, koushi_sdk::MatrixVerificationRequestState::Ready) {
            return;
        }
        let flow_id = *flow_id;
        let handle = handle.clone();
        self.record_sas_waiting_for(flow_id, SasVerificationWaitState::SasStart);
        match run_own_user_sas_start(
            flow_id,
            "provisional_encryption_sync",
            koushi_sdk::start_own_user_sas_verification(&handle),
        )
        .await
        {
            Ok(Some(sas)) => {
                self.store_sas_verification(
                    RequestId {
                        connection_id: RuntimeConnectionId(0),
                        sequence: flow_id,
                    },
                    VerificationTarget {
                        user_id: "current-user".to_owned(),
                        device_id: "eligible-device".to_owned(),
                    },
                    sas,
                )
                .await;
            }
            Ok(None) => {}
            Err(error) => {
                let kind = classify_e2ee_trust_error(&error);
                record_sas_verification_event(sas_settled_event(
                    flow_id,
                    VerificationTerminal::Failed(kind),
                    self.active_sas_waiting_for(flow_id),
                ));
                self.clear_sas_waiting_for(flow_id);
                self.send_actions(vec![AppAction::VerificationFailed {
                    request_id: flow_id,
                    kind,
                }])
                .await;
            }
        }
    }

    fn start_sas_timeout(&mut self, flow_id: u64) {
        if let Some(task) = self.sas_timeout_task.take() {
            task.abort();
        }
        let tx = self.self_tx.clone();
        self.sas_timeout_task = Some(executor::spawn(async move {
            executor::sleep(Duration::from_secs(120)).await;
            let _ = tx
                .send(AccountMessage::SasVerificationTimedOut { flow_id })
                .await;
        }));
    }

    async fn stop_sas_timeout(&mut self) {
        if let Some(task) = self.sas_timeout_task.take() {
            task.abort();
            let _ = task.await;
        }
    }

    pub(super) async fn handle_sas_verification_timeout(&mut self, flow_id: u64) {
        let active = self
            .sas_verification
            .as_ref()
            .is_some_and(|pending| pending.request_id.sequence == flow_id)
            || self
                .own_user_verification
                .as_ref()
                .is_some_and(|(active_flow_id, _)| *active_flow_id == flow_id);
        if !active {
            return;
        }
        record_sas_verification_event(sas_timeout_fired_event(
            flow_id,
            self.active_sas_waiting_for(flow_id),
        ));
        self.settle_verification(
            flow_id,
            VerificationTerminal::Failed(TrustOperationFailureKind::Timeout),
        )
        .await;
    }

    fn observe_own_user_verification(&mut self, request_id: RequestId, flow_id: u64) {
        let Some((_, handle)) = self.own_user_verification.as_ref() else {
            return;
        };
        let (stop_tx, mut stop_rx) = oneshot::channel();
        let mut states = handle.changes();
        let tx = self.self_tx.clone();
        let request_id = RequestId {
            connection_id: request_id.connection_id,
            sequence: flow_id,
        };
        let target = VerificationTarget {
            user_id: "current-user".to_owned(),
            device_id: "eligible-device".to_owned(),
        };
        let task = executor::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    state = states.next() => {
                        let Some(state) = state else {
                            let _ = tx.send(AccountMessage::VerificationRequestObserverEnded {
                                flow_id: request_id.sequence,
                            }).await;
                            break;
                        };
                        let terminal = matches!(state,
                            koushi_sdk::MatrixVerificationRequestState::Done
                            | koushi_sdk::MatrixVerificationRequestState::Cancelled { .. }
                            | koushi_sdk::MatrixVerificationRequestState::UnsupportedMethod);
                        if tx.send(AccountMessage::VerificationRequestProgress {
                            request_id,
                            target: target.clone(),
                            state,
                        }).await.is_err() { break; }
                        if terminal { break; }
                    }
                }
            }
        });
        self.verification_request_observer = Some(VerificationObservation { stop_tx, task });
    }

    pub(super) async fn handle_incoming_verification_request(
        &mut self,
        request_id: RequestId,
        target: VerificationTarget,
        handle: koushi_sdk::MatrixVerificationRequestHandle,
    ) {
        let active_request = self
            .verification_request
            .as_ref()
            .map(|pending| (&pending.target, pending.handle.flow_id()));
        let decision = classify_incoming_verification_request(
            IncomingVerificationActivity {
                active_request,
                sas_active: self.sas_verification.is_some(),
                own_user_active: self.own_user_verification.is_some(),
            },
            &target,
            handle.flow_id(),
        );
        match decision {
            IncomingVerificationRequestDecision::Adopt => {}
            IncomingVerificationRequestDecision::Replay => return,
            IncomingVerificationRequestDecision::Conflict => {
                let _ = koushi_sdk::cancel_verification_request(&handle).await;
                return;
            }
        }

        self.verification_request = Some(PendingVerificationRequest {
            request_id,
            target: target.clone(),
            handle: handle.clone(),
        });
        self.observe_verification_request(request_id, target.clone(), handle.clone());
        self.send_actions(vec![AppAction::VerificationRequested {
            request_id: request_id.sequence,
            target: target.clone(),
        }])
        .await;
        self.emit_verification_progress(VerificationFlowState::Requested {
            request_id: request_id.sequence,
            target,
        });
        self.project_verification_request_state(request_id, handle.state())
            .await;
    }

    pub(super) async fn handle_accept_verification(&mut self, request_id: RequestId, flow_id: u64) {
        let Some(pending) = self
            .verification_request
            .as_ref()
            .filter(|pending| pending.request_id.sequence == flow_id)
        else {
            self.project_active_or_missing_verification_failure(request_id, flow_id)
                .await;
            return;
        };
        let pending_request_id = pending.request_id;
        let target = pending.target.clone();
        let handle = pending.handle.clone();

        match handle.state() {
            koushi_sdk::MatrixVerificationRequestState::Requested => {
                if let Err(error) = koushi_sdk::accept_verification_request(&handle).await {
                    self.project_verification_failure(
                        flow_id,
                        target,
                        classify_e2ee_trust_error(&error),
                    )
                    .await;
                    return;
                }
                self.project_verification_request_state(pending_request_id, handle.state())
                    .await;
            }
            koushi_sdk::MatrixVerificationRequestState::Ready => {
                match koushi_sdk::start_sas_verification(&handle).await {
                    Ok(Some(sas)) => {
                        self.store_sas_verification(pending_request_id, target, sas)
                            .await;
                    }
                    Ok(None) => {
                        self.project_verification_failure(
                            flow_id,
                            target,
                            TrustOperationFailureKind::Sdk,
                        )
                        .await;
                    }
                    Err(error) => {
                        self.project_verification_failure(
                            flow_id,
                            target,
                            classify_e2ee_trust_error(&error),
                        )
                        .await;
                    }
                }
            }
            koushi_sdk::MatrixVerificationRequestState::SasStarted(sas) => {
                self.store_sas_verification(pending_request_id, target, sas)
                    .await;
            }
            koushi_sdk::MatrixVerificationRequestState::Done => {
                self.project_verification_completed(pending_request_id)
                    .await;
            }
            koushi_sdk::MatrixVerificationRequestState::Created
            | koushi_sdk::MatrixVerificationRequestState::Cancelled { .. }
            | koushi_sdk::MatrixVerificationRequestState::UnsupportedMethod => {
                self.project_verification_failure(flow_id, target, TrustOperationFailureKind::Sdk)
                    .await;
            }
        }
    }

    pub(super) async fn handle_confirm_sas_verification(
        &mut self,
        request_id: RequestId,
        flow_id: u64,
    ) {
        let Some(pending) = self
            .sas_verification
            .as_ref()
            .filter(|pending| pending.request_id.sequence == flow_id)
        else {
            self.project_active_or_missing_verification_failure(request_id, flow_id)
                .await;
            return;
        };
        let pending_request_id = pending.request_id;
        let target = pending.target.clone();
        let handle = pending.handle.clone();

        match koushi_sdk::confirm_sas_verification(&handle).await {
            Ok(()) => {
                self.project_sas_state(pending_request_id, target, handle.state())
                    .await;
            }
            Err(error) => {
                self.project_verification_failure(
                    flow_id,
                    target,
                    classify_e2ee_trust_error(&error),
                )
                .await;
            }
        }
    }

    pub(super) async fn handle_cancel_verification(
        &mut self,
        request_id: RequestId,
        flow_id: u64,
        reason: VerificationCancelReason,
    ) {
        if self
            .recovery_task
            .as_ref()
            .is_some_and(|pending| pending.flow_id == flow_id)
        {
            self.stop_recovery_task().await;
            record_device_cleanup_offer("recovery_failed");
            self.send_actions(vec![AppAction::VerificationGateAttemptFailed {
                flow_id,
                kind: koushi_state::VerificationGateFailureKind::Cancelled,
            }])
            .await;
            return;
        }
        if self.active_verification_target(flow_id).is_some() {
            self.settle_verification(flow_id, VerificationTerminal::Cancelled(reason))
                .await;
            return;
        }
        enum CancelTarget {
            Sas {
                target: VerificationTarget,
                handle: koushi_sdk::MatrixSasVerificationHandle,
            },
            Request {
                target: VerificationTarget,
                handle: koushi_sdk::MatrixVerificationRequestHandle,
            },
            Own {
                handle: koushi_sdk::MatrixOwnUserVerificationHandle,
            },
        }

        let sas_target = self
            .sas_verification
            .as_ref()
            .filter(|pending| pending.request_id.sequence == flow_id)
            .map(|pending| CancelTarget::Sas {
                target: pending.target.clone(),
                handle: pending.handle.clone(),
            });
        let cancel_target = match reason {
            VerificationCancelReason::Mismatch => sas_target,
            VerificationCancelReason::User => sas_target
                .or_else(|| {
                    self.verification_request
                        .as_ref()
                        .filter(|pending| pending.request_id.sequence == flow_id)
                        .map(|pending| CancelTarget::Request {
                            target: pending.target.clone(),
                            handle: pending.handle.clone(),
                        })
                })
                .or_else(|| {
                    self.own_user_verification
                        .as_ref()
                        .filter(|(active_flow_id, _)| *active_flow_id == flow_id)
                        .map(|(_, handle)| CancelTarget::Own {
                            handle: handle.clone(),
                        })
                }),
        };

        let Some(cancel_target) = cancel_target else {
            if reason == VerificationCancelReason::Mismatch {
                self.emit_failure(request_id, CoreFailure::LocalEncryptionUnavailable);
            } else {
                self.project_active_or_missing_verification_failure(request_id, flow_id)
                    .await;
            }
            return;
        };

        let target = match &cancel_target {
            CancelTarget::Sas { target, .. } | CancelTarget::Request { target, .. } => {
                target.clone()
            }
            CancelTarget::Own { .. } => VerificationTarget {
                user_id: "current-user".to_owned(),
                device_id: "eligible-device".to_owned(),
            },
        };
        let result = match cancel_target {
            CancelTarget::Sas { handle, .. } => match reason {
                VerificationCancelReason::User => {
                    koushi_sdk::cancel_sas_verification(&handle).await
                }
                VerificationCancelReason::Mismatch => {
                    koushi_sdk::mismatch_sas_verification(&handle).await
                }
            },
            CancelTarget::Request { handle, .. } => {
                koushi_sdk::cancel_verification_request(&handle).await
            }
            CancelTarget::Own { handle } => {
                koushi_sdk::cancel_own_user_sas_verification(&handle).await
            }
        };

        self.stop_verification_request_observer().await;
        self.stop_sas_verification_observer().await;
        self.verification_request = None;
        self.sas_verification = None;
        self.own_user_verification = None;

        if let Err(error) = result {
            self.project_verification_failure(flow_id, target, classify_e2ee_trust_error(&error))
                .await;
        }
    }

    fn observe_verification_request(
        &mut self,
        request_id: RequestId,
        target: VerificationTarget,
        handle: koushi_sdk::MatrixVerificationRequestHandle,
    ) {
        let (stop_tx, mut stop_rx) = oneshot::channel();
        let mut states = handle.changes();
        let tx = self.self_tx.clone();
        let task = crate::executor::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    state = states.next() => {
                        let Some(state) = state else {
                            let _ = tx.send(AccountMessage::VerificationRequestObserverEnded {
                                flow_id: request_id.sequence,
                            }).await;
                            break;
                        };
                        let terminal = matches!(
                            state,
                            koushi_sdk::MatrixVerificationRequestState::Done
                                | koushi_sdk::MatrixVerificationRequestState::Cancelled { .. }
                                | koushi_sdk::MatrixVerificationRequestState::UnsupportedMethod
                        );
                        if tx
                            .send(AccountMessage::VerificationRequestProgress {
                                request_id,
                                target: target.clone(),
                                state,
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }
                        if terminal {
                            break;
                        }
                    }
                }
            }
        });
        self.verification_request_observer = Some(VerificationObservation { stop_tx, task });
    }

    fn observe_sas_verification(
        &mut self,
        request_id: RequestId,
        target: VerificationTarget,
        handle: koushi_sdk::MatrixSasVerificationHandle,
    ) {
        let (stop_tx, mut stop_rx) = oneshot::channel();
        let mut states = handle.changes();
        let tx = self.self_tx.clone();
        let task = crate::executor::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    state = states.next() => {
                        let Some(state) = state else {
                            let _ = tx.send(AccountMessage::SasVerificationObserverEnded {
                                flow_id: request_id.sequence,
                            }).await;
                            break;
                        };
                        let terminal = matches!(
                            state,
                            koushi_sdk::MatrixSasState::Done
                                | koushi_sdk::MatrixSasState::Cancelled { .. }
                                | koushi_sdk::MatrixSasState::UnsupportedShortAuth
                        );
                        if tx
                            .send(AccountMessage::SasVerificationProgress {
                                request_id,
                                target: target.clone(),
                                state,
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }
                        if terminal {
                            break;
                        }
                    }
                }
            }
        });
        self.sas_verification_observer = Some(VerificationObservation { stop_tx, task });
    }

    pub(super) async fn handle_verification_request_progress(
        &mut self,
        request_id: RequestId,
        _target: VerificationTarget,
        state: koushi_sdk::MatrixVerificationRequestState,
    ) {
        if !self
            .verification_request
            .as_ref()
            .is_some_and(|pending| pending.request_id.sequence == request_id.sequence)
            && !self
                .own_user_verification
                .as_ref()
                .is_some_and(|(flow_id, _)| *flow_id == request_id.sequence)
        {
            return;
        }
        let waiting_for = verification_request_waiting_for(&state);
        if let Some(waiting_for) = waiting_for {
            self.record_sas_waiting_for(request_id.sequence, waiting_for);
        }
        let mut event = sas_verification_event("request_state_changed", request_id.sequence).field(
            DiagnosticField::token("state", verification_request_state_token(&state)),
        );
        if let Some(waiting_for) = waiting_for {
            event = add_sas_waiting_for_field(event, waiting_for);
        }
        if let koushi_sdk::MatrixVerificationRequestState::Cancelled {
            kind,
            cancelled_by_us,
        } = &state
        {
            event = event
                .field(DiagnosticField::token(
                    "cancel_kind",
                    verification_cancel_kind_token(*kind),
                ))
                .field(DiagnosticField::boolean(
                    "cancelled_by_us",
                    *cancelled_by_us,
                ));
        }
        record_sas_verification_event(event);
        self.project_verification_request_state(request_id, state)
            .await;
    }

    pub(super) async fn handle_sas_verification_progress(
        &mut self,
        request_id: RequestId,
        target: VerificationTarget,
        state: koushi_sdk::MatrixSasState,
    ) {
        if !self
            .sas_verification
            .as_ref()
            .is_some_and(|pending| pending.request_id.sequence == request_id.sequence)
        {
            return;
        }
        record_sas_verification_event(sas_state_changed_event(request_id.sequence, &state));
        self.project_sas_state(request_id, target, state).await;
    }

    async fn project_verification_request_state(
        &mut self,
        request_id: RequestId,
        state: koushi_sdk::MatrixVerificationRequestState,
    ) {
        match state {
            koushi_sdk::MatrixVerificationRequestState::Created
            | koushi_sdk::MatrixVerificationRequestState::Requested => {}
            koushi_sdk::MatrixVerificationRequestState::Ready => {
                self.send_actions(vec![AppAction::VerificationAccepted {
                    request_id: request_id.sequence,
                }])
                .await;
                if let Some((flow_id, handle)) = self.own_user_verification.as_ref()
                    && *flow_id == request_id.sequence
                    && self.sas_verification.is_none()
                {
                    let handle = handle.clone();
                    self.record_sas_waiting_for(
                        request_id.sequence,
                        SasVerificationWaitState::SasStart,
                    );
                    match run_own_user_sas_start(
                        request_id.sequence,
                        "request_ready",
                        koushi_sdk::start_own_user_sas_verification(&handle),
                    )
                    .await
                    {
                        Ok(Some(sas)) => {
                            self.store_sas_verification(
                                request_id,
                                VerificationTarget {
                                    user_id: "current-user".to_owned(),
                                    device_id: "eligible-device".to_owned(),
                                },
                                sas,
                            )
                            .await;
                        }
                        Ok(None) => {}
                        Err(error) => {
                            let kind = classify_e2ee_trust_error(&error);
                            record_sas_verification_event(sas_settled_event(
                                request_id.sequence,
                                VerificationTerminal::Failed(kind),
                                self.active_sas_waiting_for(request_id.sequence),
                            ));
                            self.clear_sas_waiting_for(request_id.sequence);
                            self.send_actions(vec![AppAction::VerificationFailed {
                                request_id: request_id.sequence,
                                kind,
                            }])
                            .await;
                        }
                    }
                }
            }
            koushi_sdk::MatrixVerificationRequestState::SasStarted(sas) => {
                let target = self
                    .verification_request
                    .as_ref()
                    .filter(|pending| pending.request_id.sequence == request_id.sequence)
                    .map(|pending| pending.target.clone())
                    .or_else(|| {
                        self.own_user_verification
                            .as_ref()
                            .and_then(|(flow_id, _)| {
                                (*flow_id == request_id.sequence).then(|| VerificationTarget {
                                    user_id: "current-user".to_owned(),
                                    device_id: "eligible-device".to_owned(),
                                })
                            })
                    });
                let Some(target) = target else {
                    return;
                };
                self.store_sas_verification(request_id, target, sas).await;
            }
            koushi_sdk::MatrixVerificationRequestState::Done => {
                self.project_verification_completed(request_id).await;
            }
            koushi_sdk::MatrixVerificationRequestState::Cancelled {
                kind,
                cancelled_by_us,
            } => {
                let _ = (kind, cancelled_by_us);
                self.project_active_or_missing_verification_failure_with_kind(
                    request_id,
                    request_id.sequence,
                    TrustOperationFailureKind::Cancelled,
                )
                .await;
            }
            koushi_sdk::MatrixVerificationRequestState::UnsupportedMethod => {
                self.project_active_or_missing_verification_failure(
                    request_id,
                    request_id.sequence,
                )
                .await;
            }
        }
    }

    async fn store_sas_verification(
        &mut self,
        request_id: RequestId,
        target: VerificationTarget,
        handle: koushi_sdk::MatrixSasVerificationHandle,
    ) {
        let active_flow_id = self
            .sas_verification
            .as_ref()
            .map(|pending| pending.request_id.sequence);
        let (decision, conflict_rejection_succeeded) =
            resolve_sas_adoption(active_flow_id, request_id.sequence, || async {
                koushi_sdk::cancel_sas_verification(&handle).await.is_ok()
            })
            .await;
        match decision {
            SasAdoptionDecision::Adopt => {}
            SasAdoptionDecision::Replay => return,
            SasAdoptionDecision::Conflict => {
                record_sas_verification_event(
                    sas_verification_event("conflicting_sas_rejected", request_id.sequence).field(
                        DiagnosticField::token(
                            "outcome",
                            if conflict_rejection_succeeded == Some(true) {
                                "success"
                            } else {
                                "failed"
                            },
                        ),
                    ),
                );
                return;
            }
        }

        self.stop_sas_verification_observer().await;
        self.sas_verification = Some(PendingSasVerification {
            request_id,
            target: target.clone(),
            handle: handle.clone(),
        });
        self.start_sas_timeout(request_id.sequence);
        self.observe_sas_verification(request_id, target.clone(), handle.clone());
        let initial_state = handle.state();
        if let Some(waiting_for) = sas_state_waiting_for(&initial_state) {
            self.record_sas_waiting_for(request_id.sequence, waiting_for);
        }
        record_sas_verification_event(sas_state_changed_event(request_id.sequence, &initial_state));
        if matches!(initial_state, koushi_sdk::MatrixSasState::Started)
            && let Err(error) = koushi_sdk::accept_sas_verification(&handle).await
        {
            self.project_verification_failure(
                request_id.sequence,
                target,
                classify_e2ee_trust_error(&error),
            )
            .await;
            return;
        }
        self.project_sas_state(request_id, target, handle.state())
            .await;
    }

    async fn project_sas_state(
        &mut self,
        request_id: RequestId,
        target: VerificationTarget,
        state: koushi_sdk::MatrixSasState,
    ) {
        if let Some(waiting_for) = sas_state_waiting_for(&state) {
            self.record_sas_waiting_for(request_id.sequence, waiting_for);
        }
        match state {
            koushi_sdk::MatrixSasState::Created
            | koushi_sdk::MatrixSasState::Started
            | koushi_sdk::MatrixSasState::Accepted => {}
            koushi_sdk::MatrixSasState::SasPresented { emojis } => {
                if emojis.len() != 7 {
                    self.project_verification_failure(
                        request_id.sequence,
                        target,
                        TrustOperationFailureKind::Sdk,
                    )
                    .await;
                    return;
                }
                let own_user_flow = self
                    .own_user_verification
                    .as_ref()
                    .is_some_and(|(flow_id, _)| *flow_id == request_id.sequence);
                let action =
                    sas_projection_action(own_user_flow, request_id.sequence, emojis.clone());
                self.send_actions(vec![action]).await;
                self.emit_verification_progress(VerificationFlowState::SasPresented {
                    request_id: request_id.sequence,
                    target,
                    emojis,
                });
            }
            koushi_sdk::MatrixSasState::Confirmed => {}
            koushi_sdk::MatrixSasState::Done => {
                self.project_verification_completed(request_id).await;
            }
            koushi_sdk::MatrixSasState::Cancelled { .. } => {
                self.project_verification_failure(
                    request_id.sequence,
                    target,
                    TrustOperationFailureKind::Cancelled,
                )
                .await;
            }
            koushi_sdk::MatrixSasState::UnsupportedShortAuth => {
                self.project_verification_failure(
                    request_id.sequence,
                    target,
                    TrustOperationFailureKind::Sdk,
                )
                .await;
            }
        }
    }

    pub(super) fn active_verification_target(&self, flow_id: u64) -> Option<VerificationTarget> {
        self.sas_verification
            .as_ref()
            .filter(|pending| pending.request_id.sequence == flow_id)
            .map(|pending| pending.target.clone())
            .or_else(|| {
                self.verification_request
                    .as_ref()
                    .filter(|pending| pending.request_id.sequence == flow_id)
                    .map(|pending| pending.target.clone())
            })
            .or_else(|| {
                self.own_user_verification
                    .as_ref()
                    .and_then(|(active_flow_id, _)| {
                        (*active_flow_id == flow_id).then(|| VerificationTarget {
                            user_id: "current-user".to_owned(),
                            device_id: "eligible-device".to_owned(),
                        })
                    })
            })
            .or_else(|| {
                #[cfg(test)]
                {
                    return self.synthetic_verification.as_ref().and_then(
                        |(active_flow_id, target)| {
                            (*active_flow_id == flow_id).then(|| target.clone())
                        },
                    );
                }
                #[cfg(not(test))]
                None
            })
    }

    pub(super) async fn settle_verification(
        &mut self,
        flow_id: u64,
        terminal: VerificationTerminal,
    ) {
        let Some(target) = self.active_verification_target(flow_id) else {
            return;
        };
        let waiting_for = if matches!(terminal, VerificationTerminal::Success) {
            Some(SasVerificationWaitState::CrossSigningSettlement)
        } else {
            self.active_sas_waiting_for(flow_id)
        };
        record_sas_verification_event(sas_settled_event(flow_id, terminal, waiting_for));
        self.stop_sas_timeout().await;
        self.stop_verification_request_observer().await;
        self.stop_sas_verification_observer().await;
        self.clear_sas_waiting_for(flow_id);
        let sas = self.sas_verification.take();
        let request = self.verification_request.take();
        let own = self.own_user_verification.take();
        #[cfg(test)]
        {
            self.synthetic_verification = None;
        }

        if !matches!(terminal, VerificationTerminal::Success) {
            if let Some(pending) = sas.as_ref() {
                let _ = match terminal {
                    VerificationTerminal::Cancelled(VerificationCancelReason::Mismatch) => {
                        koushi_sdk::mismatch_sas_verification(&pending.handle).await
                    }
                    _ => koushi_sdk::cancel_sas_verification(&pending.handle).await,
                };
            } else if let Some(pending) = request.as_ref() {
                let _ = koushi_sdk::cancel_verification_request(&pending.handle).await;
            } else if let Some((_, handle)) = own.as_ref() {
                let _ = koushi_sdk::cancel_own_user_sas_verification(handle).await;
            }
        }
        drop(sas);
        drop(request);
        drop(own);

        match terminal {
            VerificationTerminal::Success => {
                self.send_actions(vec![AppAction::VerificationCompleted {
                    request_id: flow_id,
                }])
                .await;
                self.request_authoritative_trust_recheck();
                record_sas_verification_event(sas_waiting_event(
                    flow_id,
                    SasVerificationWaitState::NormalSyncResume,
                ));
                self.emit_verification_progress(VerificationFlowState::Done {
                    request_id: flow_id,
                    target,
                });
            }
            VerificationTerminal::Cancelled(reason) => {
                self.send_actions(vec![AppAction::VerificationCancelled {
                    request_id: flow_id,
                    reason,
                }])
                .await;
            }
            VerificationTerminal::Failed(kind) => {
                self.send_actions(vec![AppAction::VerificationFailed {
                    request_id: flow_id,
                    kind,
                }])
                .await;
                self.emit_verification_progress(VerificationFlowState::Failed {
                    request_id: flow_id,
                    target,
                    kind,
                });
            }
        }
    }

    async fn project_verification_completed(&mut self, request_id: RequestId) {
        if self
            .active_verification_target(request_id.sequence)
            .is_none()
        {
            return;
        }
        self.settle_verification(request_id.sequence, VerificationTerminal::Success)
            .await;
    }

    async fn project_active_or_missing_verification_failure(
        &mut self,
        request_id: RequestId,
        flow_id: u64,
    ) {
        self.project_active_or_missing_verification_failure_with_kind(
            request_id,
            flow_id,
            TrustOperationFailureKind::Sdk,
        )
        .await;
    }

    async fn project_active_or_missing_verification_failure_with_kind(
        &mut self,
        request_id: RequestId,
        flow_id: u64,
        kind: TrustOperationFailureKind,
    ) {
        if self.active_verification_target(flow_id).is_some() {
            self.settle_verification(flow_id, VerificationTerminal::Failed(kind))
                .await;
        } else {
            self.send_actions(vec![AppAction::VerificationFailed {
                request_id: flow_id,
                kind,
            }])
            .await;
            let failure = if self.session.is_some() {
                CoreFailure::LocalEncryptionUnavailable
            } else {
                CoreFailure::SessionRequired
            };
            self.emit_failure(request_id, failure);
        }
    }

    async fn project_verification_failure(
        &mut self,
        flow_id: u64,
        _target: VerificationTarget,
        kind: TrustOperationFailureKind,
    ) {
        self.settle_verification(flow_id, VerificationTerminal::Failed(kind))
            .await;
    }

    fn emit_verification_progress(&self, state: VerificationFlowState) {
        if let Some(account_key) = self.active_account_key() {
            self.emit(CoreEvent::E2eeTrust(E2eeTrustEvent::VerificationProgress {
                account_key,
                state,
            }));
        }
    }

    pub(super) async fn handle_cancel_identity_reset(
        &mut self,
        _request_id: RequestId,
        flow_id: u64,
    ) {
        if self.identity_reset_flow_id != Some(flow_id) {
            return;
        }
        let account_key = self.active_account_key();
        self.cancel_identity_reset_handle().await;
        self.send_actions(vec![AppAction::ResetIdentityCancelled {
            request_id: flow_id,
        }])
        .await;
        if let Some(account_key) = account_key {
            for event in project_identity_reset_failed_event(
                flow_id,
                account_key,
                TrustOperationFailureKind::Cancelled,
            ) {
                self.emit(event);
            }
        }
    }

    pub(super) async fn handle_identity_reset_auth_timeout(&mut self, flow_id: u64) {
        if self.identity_reset_flow_id != Some(flow_id) {
            return;
        }
        let account_key = self.active_account_key();
        self.cancel_identity_reset_handle().await;
        self.send_actions(vec![AppAction::ResetIdentityTimedOut {
            request_id: flow_id,
        }])
        .await;
        if let Some(account_key) = account_key {
            for event in project_identity_reset_failed_event(
                flow_id,
                account_key,
                TrustOperationFailureKind::Timeout,
            ) {
                self.emit(event);
            }
        }
    }

    pub(super) async fn handle_submit_identity_reset_auth(
        &mut self,
        request_id: RequestId,
        flow_id: u64,
        request: koushi_state::IdentityResetAuthRequest,
    ) {
        let flow_request_id = RequestId {
            connection_id: request_id.connection_id,
            sequence: flow_id,
        };
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.cancel_identity_reset_handle().await;
                self.send_actions(vec![AppAction::ResetIdentityFailed {
                    request_id: flow_id,
                    kind: TrustOperationFailureKind::Sdk,
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };
        let account_key = AccountKey(session.info.user_id.clone());
        if self.identity_reset_flow_id != Some(flow_id) {
            return;
        }
        let result = match self.identity_reset_handle.as_ref() {
            Some(handle) => koushi_sdk::complete_identity_reset(&session, handle, &request).await,
            None => Err(koushi_sdk::E2eeTrustError::Sdk(
                "identity reset auth continuation missing".to_owned(),
            )),
        };

        drop(request);

        match result {
            Ok(()) => {
                self.clear_identity_reset_handle_after_completion();
                let (actions, events) =
                    project_reset_identity_completed(flow_request_id, account_key);
                self.send_actions(actions).await;
                for event in events {
                    self.emit(event);
                }
            }
            Err(error) => {
                self.cancel_identity_reset_handle().await;
                let (actions, events) =
                    project_reset_identity_error(flow_request_id, account_key, error);
                self.send_actions(actions).await;
                for event in events {
                    self.emit(event);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
