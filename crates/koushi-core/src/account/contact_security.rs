//! `contact_security` ownership for AccountActor (#1024).
//!
//! User info asks for one contact's security details. The actor performs a
//! fresh `/keys/query` for that contact, then keeps the details current from
//! the SDK's device/identity store notifications until User info closes,
//! another contact opens, or the session ends. Everything here is read-only:
//! no pinning, verification, or trust change.

use futures_util::{FutureExt, StreamExt};
use koushi_protocol::command::ContactSecurityRequest;
use koushi_protocol::failure::CoreFailure;
use koushi_protocol::ids::RequestId;
use koushi_state::{
    AppAction, ContactSecurityFailureKind, ContactSecuritySummary, TrustOperationFailureKind,
    VerificationCancelReason, VerificationFlowState, VerificationInitiator, VerificationTarget,
};
use tokio::sync::oneshot;

use super::actor::{AccountActor, AccountMessage};
use super::recovery_backup::classify_e2ee_trust_error;
use super::verification::{PendingVerificationRequest, send_observer_output_until_stopped};

/// The open contact and its store-change observer task.
pub(super) struct ContactSecurityObservation {
    user_id: String,
    generation: u64,
    /// Last summary projected to the reducer, to suppress unchanged re-reads.
    last: ContactSecuritySummary,
    stop_tx: oneshot::Sender<()>,
    task: crate::executor::JoinHandle<()>,
}

impl std::fmt::Debug for ContactSecurityObservation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ContactSecurityObservation")
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

pub(super) struct PendingContactVerificationSend {
    request_id: RequestId,
    generation: u64,
    task: crate::executor::JoinHandle<()>,
}

impl AccountActor {
    pub(super) async fn handle_contact_security(
        &mut self,
        request_id: RequestId,
        request: ContactSecurityRequest,
    ) {
        match request {
            ContactSecurityRequest::Load { user_id } => {
                self.load_contact_security(request_id, user_id).await;
            }
            ContactSecurityRequest::RequestVerification { user_id } => {
                self.request_user_verification(request_id, user_id).await;
            }
            ContactSecurityRequest::Close => self.stop_contact_security_observer().await,
        }
    }

    /// **Verify user** (#1024). The runtime already projected
    /// `VerificationRequestSent` (initiator `Us`); send and settle the request
    /// off-actor so a homeserver wait cannot block account commands.
    async fn request_user_verification(&mut self, request_id: RequestId, user_id: String) {
        let target = VerificationTarget {
            user_id: user_id.clone(),
            device_id: String::new(),
        };
        let Some(session) = self.session.clone() else {
            self.send_actions(vec![AppAction::VerificationFailed {
                request_id: request_id.sequence,
                kind: TrustOperationFailureKind::Sdk,
            }])
            .await;
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        self.cancel_verification_handles().await;
        self.contact_verification_send_generation =
            self.contact_verification_send_generation.wrapping_add(1);
        let generation = self.contact_verification_send_generation;
        let tx = self.self_tx.clone();
        let task = crate::executor::spawn(async move {
            let result = koushi_sdk::request_user_verification(&session, &user_id).await;
            let _ = tx
                .send(AccountMessage::ContactUserVerificationRequestFinished {
                    request_id,
                    generation,
                    target,
                    result,
                })
                .await;
        });
        self.pending_contact_verification_send = Some(PendingContactVerificationSend {
            request_id,
            generation,
            task,
        });
    }

    pub(super) async fn handle_contact_user_verification_request_finished(
        &mut self,
        request_id: RequestId,
        generation: u64,
        target: VerificationTarget,
        result: Result<koushi_sdk::MatrixVerificationRequestHandle, koushi_sdk::E2eeTrustError>,
    ) {
        if self.contact_verification_send_generation != generation
            || !self
                .pending_contact_verification_send
                .as_ref()
                .is_some_and(|pending| {
                    pending.generation == generation && pending.request_id == request_id
                })
        {
            return;
        }
        self.pending_contact_verification_send = None;
        match result {
            Ok(handle) => {
                self.verification_request = Some(PendingVerificationRequest {
                    request_id,
                    target: target.clone(),
                    handle: handle.clone(),
                    start_sas_when_ready: true,
                });
                self.observe_verification_request(request_id, target.clone(), handle.clone());
                self.emit_verification_progress(VerificationFlowState::Requested {
                    request_id: request_id.sequence,
                    target: target.clone(),
                    initiator: VerificationInitiator::Us,
                });
                self.project_verification_request_state(request_id, handle.state())
                    .await;
            }
            Err(error) => {
                self.send_actions(vec![AppAction::VerificationFailed {
                    request_id: request_id.sequence,
                    kind: classify_e2ee_trust_error(&error),
                }])
                .await;
            }
        }
        // Sending may have created the direct chat (also when it then
        // failed); re-read so the offer names it. A chat that becomes known
        // only with the next sync arrives through the `m.direct` observer.
        self.refresh_open_contact_security(&target.user_id).await;
    }

    pub(super) async fn cancel_contact_verification_send(
        &mut self,
        flow_id: u64,
        reason: VerificationCancelReason,
    ) -> bool {
        if reason != VerificationCancelReason::User
            || self
                .pending_contact_verification_send
                .as_ref()
                .is_none_or(|pending| pending.request_id.sequence != flow_id)
        {
            return false;
        }
        self.contact_verification_send_generation =
            self.contact_verification_send_generation.wrapping_add(1);
        if let Some(pending) = self.pending_contact_verification_send.take() {
            pending.task.abort();
            let _ = pending.task.await;
        }
        self.send_actions(vec![AppAction::VerificationCancelled {
            request_id: flow_id,
            reason,
        }])
        .await;
        true
    }

    pub(super) async fn cancel_pending_contact_verification_send(&mut self) {
        self.contact_verification_send_generation =
            self.contact_verification_send_generation.wrapping_add(1);
        if let Some(pending) = self.pending_contact_verification_send.take() {
            pending.task.abort();
            let _ = pending.task.await;
        }
    }

    /// Re-read whichever contact is open, for changes outside the contact's
    /// keys, such as this session gaining or losing your cross-signing keys.
    pub(super) async fn refresh_any_open_contact_security(&mut self) {
        let Some(generation) = self
            .contact_security
            .as_ref()
            .map(|observation| observation.generation)
        else {
            return;
        };
        self.handle_contact_security_store_changed(generation).await;
    }

    /// As the requester of a user verification, start SAS once the contact
    /// accepted. If they already started it, the SDK reports `SasStarted`
    /// and the existing adoption path takes over.
    pub(super) async fn start_user_verification_sas_if_ready(&mut self, request_id: RequestId) {
        let Some((target, handle)) = self
            .verification_request
            .as_ref()
            .filter(|pending| {
                pending.request_id.sequence == request_id.sequence && pending.start_sas_when_ready
            })
            .map(|pending| (pending.target.clone(), pending.handle.clone()))
        else {
            return;
        };
        if self.sas_verification.is_some() {
            return;
        }
        match koushi_sdk::start_sas_verification(&handle).await {
            Ok(Some(sas)) => {
                let request_id = self
                    .verification_request
                    .as_ref()
                    .map_or(request_id, |pending| pending.request_id);
                self.store_sas_verification(request_id, target, sas).await;
            }
            Ok(None) => {}
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

    /// Re-read the open contact right after a verification with them
    /// completed, so the row shows the result without waiting for sync.
    pub(super) async fn refresh_open_contact_security(&mut self, user_id: &str) {
        let Some(generation) = self
            .contact_security
            .as_ref()
            .filter(|observation| observation.user_id == user_id)
            .map(|observation| observation.generation)
        else {
            return;
        };
        self.handle_contact_security_store_changed(generation).await;
    }

    async fn load_contact_security(&mut self, request_id: RequestId, user_id: String) {
        self.stop_contact_security_observer().await;
        let Some(session) = self.session.clone() else {
            self.send_actions(vec![AppAction::ContactSecurityLoadFailed {
                request_id: request_id.sequence,
                user_id,
                failure_kind: ContactSecurityFailureKind::SessionRequired,
            }])
            .await;
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        let generation = self.contact_security_generation;
        let tx = self.self_tx.clone();
        self.contact_security_load_task = Some(crate::executor::spawn(async move {
            // Subscribe before retrieval so a concurrent store change is not
            // missed. Network work stays outside the AccountActor loop.
            let changes = koushi_sdk::observe_contact_security_changes(&session).await;
            let result = koushi_sdk::load_contact_security(&session, &user_id).await;
            let _ = tx
                .send(AccountMessage::ContactSecurityLoadFinished {
                    request_id,
                    generation,
                    user_id,
                    result,
                    changes,
                })
                .await;
        }));
    }

    pub(super) async fn handle_contact_security_load_finished(
        &mut self,
        request_id: RequestId,
        generation: u64,
        user_id: String,
        result: Result<ContactSecuritySummary, ContactSecurityFailureKind>,
        changes: Result<koushi_sdk::ContactSecurityChanges, ContactSecurityFailureKind>,
    ) {
        if generation != self.contact_security_generation {
            return;
        }
        self.contact_security_load_task = None;
        match result {
            Ok(summary) => {
                self.send_actions(vec![AppAction::ContactSecurityLoaded {
                    request_id: request_id.sequence,
                    user_id: user_id.clone(),
                    summary: summary.clone(),
                }])
                .await;
                if let Ok(changes) = changes {
                    self.start_contact_security_observer(user_id, generation, summary, changes);
                }
            }
            Err(failure_kind) => {
                self.send_actions(vec![AppAction::ContactSecurityLoadFailed {
                    request_id: request_id.sequence,
                    user_id,
                    failure_kind,
                }])
                .await;
            }
        }
    }

    fn start_contact_security_observer(
        &mut self,
        user_id: String,
        generation: u64,
        summary: ContactSecuritySummary,
        mut changes: koushi_sdk::ContactSecurityChanges,
    ) {
        let (stop_tx, mut stop_rx) = oneshot::channel();
        let tx = self.self_tx.clone();
        let task = crate::executor::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    change = changes.next() => {
                        if change.is_none() {
                            break;
                        }
                        // Coalesce a burst of store writes into one re-read.
                        while let Some(Some(())) = changes.next().now_or_never() {}
                        if !send_observer_output_until_stopped(
                            &tx,
                            AccountMessage::ContactSecurityStoreChanged { generation },
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
        self.contact_security = Some(ContactSecurityObservation {
            user_id,
            generation,
            last: summary,
            stop_tx,
            task,
        });
    }

    pub(super) async fn handle_contact_security_store_changed(&mut self, generation: u64) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let Some(user_id) = self
            .contact_security
            .as_ref()
            .filter(|observation| observation.generation == generation)
            .map(|observation| observation.user_id.clone())
        else {
            return;
        };
        // A failed store read keeps the last projected answer; it is never
        // turned into a confirmation.
        let Ok(summary) = koushi_sdk::read_contact_security(&session, &user_id).await else {
            return;
        };
        let Some(observation) = self
            .contact_security
            .as_mut()
            .filter(|observation| observation.generation == generation)
        else {
            return;
        };
        if observation.last == summary {
            return;
        }
        observation.last = summary.clone();
        self.send_actions(vec![AppAction::ContactSecurityRefreshed {
            user_id,
            summary,
        }])
        .await;
    }

    pub(super) async fn stop_contact_security_observer(&mut self) {
        self.contact_security_generation = self.contact_security_generation.wrapping_add(1);
        if let Some(task) = self.contact_security_load_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(observation) = self.contact_security.take() {
            let ContactSecurityObservation { stop_tx, task, .. } = observation;
            let _ = stop_tx.send(());
            task.abort();
            let _ = task.await;
        }
    }
}
