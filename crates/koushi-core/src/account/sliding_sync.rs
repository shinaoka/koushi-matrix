//! `sliding_sync` ownership for AccountActor.

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel};
use koushi_key::{SessionKeyId, StoredMatrixSession};
use koushi_sdk::{MatrixClientSession, PersistableMatrixSession};
use koushi_state::{
    AppAction, AuthFailureKind, LoginAttemptId, SlidingSyncAdmission, SlidingSyncAdmissionSource,
    SlidingSyncCapabilityResult, SlidingSyncPositiveEvidence,
};

use crate::event::{AccountEvent, CoreEvent};
use crate::executor;
use crate::failure::CoreFailure;
use crate::ids::RequestId;
use crate::store::account_key_from_info;

use super::actor::{AccountActor, AccountMessage, current_epoch_ms, trace_account_request};
use super::session_lifecycle::{RESTORE_FAILED_MESSAGE, RestoreOutcome};

fn sliding_sync_capability_result(
    result: koushi_sdk::SlidingSyncDiscoveryResult,
) -> SlidingSyncCapabilityResult {
    match result {
        koushi_sdk::SlidingSyncDiscoveryResult::Supported { .. } => {
            SlidingSyncCapabilityResult::Supported {
                evidence: SlidingSyncPositiveEvidence {
                    observed_at_ms: current_epoch_ms(),
                },
            }
        }
        koushi_sdk::SlidingSyncDiscoveryResult::Unsupported { .. } => {
            SlidingSyncCapabilityResult::Unsupported
        }
        koushi_sdk::SlidingSyncDiscoveryResult::Unreachable { .. } => {
            SlidingSyncCapabilityResult::Unreachable
        }
        koushi_sdk::SlidingSyncDiscoveryResult::InvalidResponse { .. } => {
            SlidingSyncCapabilityResult::InvalidResponse
        }
    }
}

fn sliding_sync_auth_failure_kind(result: &SlidingSyncCapabilityResult) -> AuthFailureKind {
    match result {
        SlidingSyncCapabilityResult::Unsupported => AuthFailureKind::Unsupported,
        SlidingSyncCapabilityResult::Unreachable => AuthFailureKind::Network,
        SlidingSyncCapabilityResult::InvalidResponse
        | SlidingSyncCapabilityResult::Supported { .. } => AuthFailureKind::Sdk,
    }
}

fn sliding_sync_revalidation_completion_is_current(
    active: Option<(u64, u64)>,
    account_epoch: u64,
    request_id: u64,
) -> bool {
    active == Some((account_epoch, request_id))
}

fn record_sliding_sync_capability_persistence(outcome: &'static str) {
    koushi_diagnostics::record(
        DiagnosticEvent::new(
            if outcome == "saved" {
                DiagnosticLevel::Info
            } else {
                DiagnosticLevel::Warn
            },
            "core.sliding_sync_capability",
            "positive_evidence_persistence",
        )
        .field(DiagnosticField::token("outcome", outcome)),
    );
}

pub(super) enum PendingSlidingSyncAdmission {
    NewLogin {
        account_epoch: u64,
        request_id: u64,
        core_request_id: RequestId,
        login_session: MatrixClientSession,
        persistable: PersistableMatrixSession,
        key_id: SessionKeyId,
        action: AppAction,
        ready_events: Vec<CoreEvent>,
    },
    StoredSessionRestore {
        account_epoch: u64,
        request_id: u64,
        core_request_id: RequestId,
        persistable: PersistableMatrixSession,
        key_id: SessionKeyId,
        outcome: RestoreOutcome,
    },
}

#[derive(Clone, Copy)]
pub(super) struct PendingSlidingSyncRetry {
    account_epoch: u64,
    blocked_request_id: u64,
    request_id: u64,
    core_request_id: RequestId,
}

#[derive(Clone)]
pub(super) struct StoredSlidingSyncAdmissionContext {
    core_request_id: RequestId,
    persistable: PersistableMatrixSession,
    key_id: SessionKeyId,
    outcome: RestoreOutcome,
}

impl PendingSlidingSyncAdmission {
    fn correlation(&self) -> (u64, u64) {
        match self {
            Self::NewLogin {
                account_epoch,
                request_id,
                ..
            }
            | Self::StoredSessionRestore {
                account_epoch,
                request_id,
                ..
            } => (*account_epoch, *request_id),
        }
    }

    fn positive_evidence(&self) -> Option<SlidingSyncPositiveEvidence> {
        match self {
            Self::NewLogin { persistable, .. } | Self::StoredSessionRestore { persistable, .. } => {
                persistable.sliding_sync_positive_evidence()
            }
        }
    }

    pub(super) fn key_id(&self) -> &SessionKeyId {
        match self {
            Self::NewLogin { key_id, .. } | Self::StoredSessionRestore { key_id, .. } => key_id,
        }
    }

    fn set_positive_evidence(&mut self, evidence: SlidingSyncPositiveEvidence) {
        match self {
            Self::NewLogin { persistable, .. } | Self::StoredSessionRestore { persistable, .. } => {
                *persistable = persistable
                    .clone()
                    .with_sliding_sync_positive_evidence(evidence);
            }
        }
    }

    fn prepare_retry(
        &mut self,
        request_id: u64,
        core_request_id: RequestId,
    ) -> Option<(
        u64,
        SlidingSyncAdmission,
        String,
        Option<SlidingSyncPositiveEvidence>,
    )> {
        let Self::StoredSessionRestore {
            account_epoch,
            request_id: active_request_id,
            core_request_id: active_core_request_id,
            persistable,
            ..
        } = self
        else {
            return None;
        };
        *active_request_id = request_id;
        *active_core_request_id = core_request_id;
        let info = persistable.info.clone();
        Some((
            *account_epoch,
            SlidingSyncAdmission::StoredSessionRestore { info: info.clone() },
            info.homeserver,
            persistable.sliding_sync_positive_evidence(),
        ))
    }
}

impl AccountActor {
    pub(super) fn next_sliding_sync_correlation(&mut self) -> Option<(u64, u64)> {
        self.sliding_sync_account_epoch = self.sliding_sync_account_epoch.checked_add(1)?;
        self.sliding_sync_request_id = self.sliding_sync_request_id.checked_add(1)?;
        Some((
            self.sliding_sync_account_epoch,
            self.sliding_sync_request_id,
        ))
    }

    fn next_sliding_sync_request_id(&mut self) -> Option<u64> {
        self.sliding_sync_request_id = self.sliding_sync_request_id.checked_add(1)?;
        Some(self.sliding_sync_request_id)
    }

    pub(super) async fn handle_retry_sliding_sync_capability(
        &mut self,
        core_request_id: RequestId,
    ) {
        let Some((account_epoch, blocked_request_id)) = self
            .pending_sliding_sync_admission
            .as_ref()
            .map(PendingSlidingSyncAdmission::correlation)
        else {
            self.emit_failure(core_request_id, CoreFailure::SessionRequired);
            return;
        };
        let Some(request_id) = self.next_sliding_sync_request_id() else {
            self.emit_failure(
                core_request_id,
                CoreFailure::AccountOperationFailed {
                    kind: AuthFailureKind::Sdk,
                },
            );
            return;
        };
        self.pending_sliding_sync_retry = Some(PendingSlidingSyncRetry {
            account_epoch,
            blocked_request_id,
            request_id,
            core_request_id,
        });
        self.send_actions(vec![AppAction::SlidingSyncCapabilityRetryAccepted {
            account_epoch,
            blocked_request_id,
            request_id,
        }])
        .await;
    }

    pub(super) async fn start_sliding_sync_capability_retry(
        &mut self,
        account_epoch: u64,
        blocked_request_id: u64,
        request_id: u64,
    ) {
        let Some(pending_retry) = self.pending_sliding_sync_retry else {
            return;
        };
        if pending_retry.account_epoch != account_epoch
            || pending_retry.blocked_request_id != blocked_request_id
            || pending_retry.request_id != request_id
            || self
                .pending_sliding_sync_admission
                .as_ref()
                .map(PendingSlidingSyncAdmission::correlation)
                != Some((account_epoch, blocked_request_id))
        {
            return;
        }
        let Some((active_epoch, admission, homeserver, positive_evidence)) = self
            .pending_sliding_sync_admission
            .as_mut()
            .and_then(|pending| pending.prepare_retry(request_id, pending_retry.core_request_id))
        else {
            return;
        };
        if active_epoch != account_epoch {
            return;
        }
        self.pending_sliding_sync_retry = None;
        self.cancel_sliding_sync_discovery_task().await;
        self.send_actions(vec![AppAction::SlidingSyncCapabilityCheckStarted {
            account_epoch,
            request_id,
            admission,
            positive_evidence,
        }])
        .await;
        let tx = self.self_tx.clone();
        self.sliding_sync_discovery_task = Some(executor::spawn(async move {
            let result = koushi_sdk::discover_sliding_sync_support(&homeserver).await;
            let _ = tx
                .send(AccountMessage::SlidingSyncCapabilityDiscovered {
                    account_epoch,
                    request_id,
                    result,
                })
                .await;
        }));
    }

    pub(super) async fn begin_sliding_sync_capability_discovery(
        &mut self,
        pending: PendingSlidingSyncAdmission,
        admission: SlidingSyncAdmission,
        homeserver: String,
    ) {
        self.sliding_sync_diagnostics.admission_discovery_started();
        self.cancel_sliding_sync_discovery_task().await;
        self.discard_pending_sliding_sync_admission().await;
        self.pending_sliding_sync_retry = None;
        self.stored_sliding_sync_admission = None;
        self.sliding_sync_revalidation_pending = None;
        let (account_epoch, request_id) = pending.correlation();
        let positive_evidence = pending.positive_evidence();
        self.pending_sliding_sync_admission = Some(pending);
        self.send_actions(vec![AppAction::SlidingSyncCapabilityCheckStarted {
            account_epoch,
            request_id,
            admission,
            positive_evidence,
        }])
        .await;
        let tx = self.self_tx.clone();
        self.sliding_sync_discovery_task = Some(executor::spawn(async move {
            let result = koushi_sdk::discover_sliding_sync_support(&homeserver).await;
            let _ = tx
                .send(AccountMessage::SlidingSyncCapabilityDiscovered {
                    account_epoch,
                    request_id,
                    result,
                })
                .await;
        }));
    }

    pub(super) async fn finish_sliding_sync_capability_discovery(
        &mut self,
        account_epoch: u64,
        request_id: u64,
        result: koushi_sdk::SlidingSyncDiscoveryResult,
    ) {
        if self
            .pending_sliding_sync_admission
            .as_ref()
            .map(PendingSlidingSyncAdmission::correlation)
            != Some((account_epoch, request_id))
        {
            return;
        }
        self.sliding_sync_discovery_task = None;
        self.sliding_sync_diagnostics
            .record_discovery(crate::SlidingSyncDiscoveryDiagnostic::from_result(&result));
        let state_result = sliding_sync_capability_result(result);
        if matches!(state_result, SlidingSyncCapabilityResult::Supported { .. })
            && let SlidingSyncCapabilityResult::Supported { evidence } = &state_result
            && let Some(pending) = self.pending_sliding_sync_admission.as_mut()
        {
            pending.set_positive_evidence(evidence.clone());
        }
        let has_positive_evidence = self
            .pending_sliding_sync_admission
            .as_ref()
            .and_then(PendingSlidingSyncAdmission::positive_evidence)
            .is_some();
        let is_restore = matches!(
            self.pending_sliding_sync_admission,
            Some(PendingSlidingSyncAdmission::StoredSessionRestore { .. })
        );
        let will_continue = matches!(&state_result, SlidingSyncCapabilityResult::Supported { .. })
            || (is_restore
                && has_positive_evidence
                && matches!(
                    state_result,
                    SlidingSyncCapabilityResult::Unreachable
                        | SlidingSyncCapabilityResult::InvalidResponse
                ));
        self.send_actions(vec![AppAction::SlidingSyncCapabilityCheckCompleted {
            account_epoch,
            request_id,
            result: state_result.clone(),
        }])
        .await;
        if !will_continue {
            let core_request_id = match self.pending_sliding_sync_admission.as_ref() {
                Some(PendingSlidingSyncAdmission::NewLogin {
                    core_request_id, ..
                })
                | Some(PendingSlidingSyncAdmission::StoredSessionRestore {
                    core_request_id, ..
                }) => *core_request_id,
                None => return,
            };
            if matches!(
                self.pending_sliding_sync_admission,
                Some(PendingSlidingSyncAdmission::NewLogin { .. })
            ) {
                self.discard_pending_sliding_sync_admission().await;
                self.send_actions(vec![AppAction::LoginFailed {
                    attempt_id: LoginAttemptId::new(
                        core_request_id.connection_id.0,
                        core_request_id.sequence,
                    ),
                    message: "login failed".to_owned(),
                }])
                .await;
            }
            self.emit_failure(
                core_request_id,
                CoreFailure::AccountOperationFailed {
                    kind: sliding_sync_auth_failure_kind(&state_result),
                },
            );
        }
    }

    pub(super) async fn continue_sliding_sync_admission(
        &mut self,
        account_epoch: u64,
        request_id: u64,
        source: SlidingSyncAdmissionSource,
    ) {
        if self
            .pending_sliding_sync_admission
            .as_ref()
            .map(PendingSlidingSyncAdmission::correlation)
            != Some((account_epoch, request_id))
        {
            return;
        }
        let pending = self
            .pending_sliding_sync_admission
            .take()
            .expect("matching Sliding Sync admission remains pending");
        match pending {
            PendingSlidingSyncAdmission::NewLogin {
                core_request_id,
                login_session,
                persistable,
                key_id,
                action,
                ready_events,
                ..
            } => {
                self.stored_sliding_sync_admission = None;
                self.prepare_store_backed_session(&login_session, true)
                    .await;
                self.install_provisional_session(login_session, persistable, key_id, action)
                    .await;
                self.pending_ready_events.extend(ready_events);
            }
            PendingSlidingSyncAdmission::StoredSessionRestore {
                core_request_id,
                persistable,
                key_id,
                outcome,
                ..
            } => {
                if source == SlidingSyncAdmissionSource::Network
                    && persistable.sliding_sync_positive_evidence().is_some()
                {
                    let persistence_outcome = if self
                        .persist_stored_sliding_sync_session(&key_id, &persistable)
                        .await
                        .is_ok()
                    {
                        "saved"
                    } else {
                        "failed"
                    };
                    record_sliding_sync_capability_persistence(persistence_outcome);
                }
                trace_account_request("restore_account", core_request_id, "store_restore_begin");
                match self.restore_into_store(&persistable, &key_id).await {
                    Ok(session) => {
                        trace_account_request(
                            "restore_account",
                            core_request_id,
                            "store_restore_ok",
                        );
                        let info = session.info.clone();
                        let account_key = account_key_from_info(&info);
                        self.stored_sliding_sync_admission =
                            Some(StoredSlidingSyncAdmissionContext {
                                core_request_id,
                                persistable: persistable.clone(),
                                key_id: key_id.clone(),
                                outcome,
                            });
                        self.install_provisional_session(
                            session,
                            persistable,
                            key_id,
                            AppAction::RestoreSessionSucceeded(info),
                        )
                        .await;
                        self.pending_ready_events
                            .push(CoreEvent::Account(match outcome {
                                RestoreOutcome::Restored => AccountEvent::SessionRestored {
                                    request_id: core_request_id,
                                    account_key,
                                },
                                RestoreOutcome::Switched => AccountEvent::AccountSwitched {
                                    request_id: core_request_id,
                                    account_key,
                                },
                            }));
                    }
                    Err(failure) => {
                        trace_account_request(
                            "restore_account",
                            core_request_id,
                            "store_restore_failed",
                        );
                        self.send_actions(vec![AppAction::RestoreSessionFailed {
                            message: RESTORE_FAILED_MESSAGE.to_owned(),
                        }])
                        .await;
                        self.emit_failure(core_request_id, failure);
                    }
                }
            }
        }
    }

    pub(super) async fn start_sliding_sync_revalidation(&mut self, account_epoch: u64) {
        if self.sliding_sync_revalidation_pending != Some(account_epoch)
            || self.sliding_sync_discovery_task.is_some()
        {
            return;
        }
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let homeserver = session.info.homeserver.clone();
        let Some(request_id) = self.next_sliding_sync_request_id() else {
            return;
        };
        self.sliding_sync_diagnostics.discovery_started();
        self.send_actions(vec![AppAction::SlidingSyncCapabilityRevalidationStarted {
            account_epoch,
            request_id,
        }])
        .await;
        self.sliding_sync_revalidation_pending = None;
        self.sliding_sync_revalidation_request = Some((account_epoch, request_id));
        let tx = self.self_tx.clone();
        self.sliding_sync_discovery_task = Some(executor::spawn(async move {
            let result = koushi_sdk::discover_sliding_sync_support(&homeserver).await;
            let _ = tx
                .send(
                    AccountMessage::SlidingSyncCapabilityRevalidationDiscovered {
                        account_epoch,
                        request_id,
                        result,
                    },
                )
                .await;
        }));
    }

    pub(super) async fn finish_sliding_sync_revalidation(
        &mut self,
        account_epoch: u64,
        request_id: u64,
        result: koushi_sdk::SlidingSyncDiscoveryResult,
    ) {
        if account_epoch != self.sliding_sync_account_epoch
            || !sliding_sync_revalidation_completion_is_current(
                self.sliding_sync_revalidation_request,
                account_epoch,
                request_id,
            )
        {
            return;
        }
        self.sliding_sync_diagnostics
            .record_discovery(crate::SlidingSyncDiscoveryDiagnostic::from_result(&result));
        let state_result = sliding_sync_capability_result(result);
        self.send_actions(vec![
            AppAction::SlidingSyncCapabilityRevalidationCompleted {
                account_epoch,
                request_id,
                result: state_result,
            },
        ])
        .await;
    }

    pub(super) async fn settle_sliding_sync_revalidation(
        &mut self,
        account_epoch: u64,
        request_id: u64,
        state_result: SlidingSyncCapabilityResult,
    ) {
        if account_epoch != self.sliding_sync_account_epoch
            || !sliding_sync_revalidation_completion_is_current(
                self.sliding_sync_revalidation_request,
                account_epoch,
                request_id,
            )
        {
            return;
        }
        self.sliding_sync_revalidation_request = None;
        self.sliding_sync_discovery_task = None;
        if let SlidingSyncCapabilityResult::Supported { evidence } = &state_result {
            self.sliding_sync_positive_evidence = Some(evidence.clone());
            if let Some(context) = self.stored_sliding_sync_admission.as_mut() {
                context.persistable = context
                    .persistable
                    .clone()
                    .with_sliding_sync_positive_evidence(evidence.clone());
            }
            let outcome = if self
                .persist_sliding_sync_positive_evidence(evidence.clone())
                .await
                .is_ok()
            {
                "saved"
            } else {
                "failed"
            };
            record_sliding_sync_capability_persistence(outcome);
        } else if matches!(state_result, SlidingSyncCapabilityResult::Unsupported) {
            if let Some(context) = self.stored_sliding_sync_admission.clone() {
                self.pending_sliding_sync_admission =
                    Some(PendingSlidingSyncAdmission::StoredSessionRestore {
                        account_epoch,
                        request_id,
                        core_request_id: context.core_request_id,
                        persistable: context.persistable,
                        key_id: context.key_id,
                        outcome: context.outcome,
                    });
            }
            self.stop_current_session_runtime().await;
            self.session_promoted = false;
        } else {
            self.sliding_sync_revalidation_pending = Some(account_epoch);
        }
    }

    async fn persist_sliding_sync_positive_evidence(
        &self,
        evidence: SlidingSyncPositiveEvidence,
    ) -> Result<(), ()> {
        let (Some(session), Some(key_id)) = (self.session.clone(), self.session_key_id.clone())
        else {
            return Err(());
        };
        let store = self.store.clone();
        executor::spawn_blocking(move || {
            let persistable = session
                .persistable_session()
                .map_err(|_| ())?
                .with_sliding_sync_positive_evidence(evidence);
            let json = persistable.to_json().map_err(|_| ())?;
            store
                .credential_backend()
                .save_matrix_session(&key_id, &StoredMatrixSession::new(json))
                .map_err(|_| ())
        })
        .await
        .map_err(|_| ())?
    }

    async fn persist_stored_sliding_sync_session(
        &self,
        key_id: &SessionKeyId,
        persistable: &PersistableMatrixSession,
    ) -> Result<(), ()> {
        let store = self.store.clone();
        let key_id = key_id.clone();
        let persistable = persistable.clone();
        executor::spawn_blocking(move || {
            let json = persistable.to_json().map_err(|_| ())?;
            store
                .credential_backend()
                .save_matrix_session(&key_id, &StoredMatrixSession::new(json))
                .map_err(|_| ())
        })
        .await
        .map_err(|_| ())?
    }

    pub(super) async fn cancel_sliding_sync_discovery_task(&mut self) {
        self.sliding_sync_revalidation_request = None;
        if let Some(task) = self.sliding_sync_discovery_task.take() {
            task.abort();
            let _ = task.await;
        }
    }

    pub(super) async fn discard_pending_sliding_sync_admission(&mut self) {
        if let Some(PendingSlidingSyncAdmission::NewLogin {
            login_session,
            key_id,
            ..
        }) = self.pending_sliding_sync_admission.take()
        {
            self.abort_login(login_session, &key_id, false, true).await;
        }
    }
}

#[cfg(test)]
mod tests {

    use koushi_key::StoredMatrixSession;
    use koushi_sdk::PersistableMatrixSession;
    use koushi_state::{
        AppAction, AuthFailureKind, LoginRequest, SlidingSyncAdmission,
        SlidingSyncCapabilityResult, SlidingSyncPositiveEvidence,
    };

    use super::{
        record_sliding_sync_capability_persistence, sliding_sync_revalidation_completion_is_current,
    };
    use crate::account::actor::AccountMessage;
    use crate::account::test_support::{
        inspect_session_runtime, recv_account_action_with_sliding_sync_effects,
        spawn_actor_with_dirs, spawn_named_quarantine_password_server_with_controls,
        test_request_id,
    };
    use crate::command::AccountCommand;

    use crate::event::CoreEvent;

    use crate::failure::CoreFailure;
    use crate::ids::{RequestId, RuntimeConnectionId};

    use crate::store::CredentialStoreBackend;
    use crate::store::session_key_id_from_info;

    use tempfile::tempdir;

    #[test]
    fn sliding_sync_evidence_persistence_diagnostic_is_private_and_closed() {
        let output = std::process::Command::new(
            std::env::current_exe().expect("current test executable should be available"),
        )
        .args([
            "--exact",
            "account::sliding_sync::tests::sliding_sync_evidence_persistence_diagnostic_child",
            "--ignored",
            "--nocapture",
        ])
        .output()
        .expect("sliding sync persistence diagnostic child should run");
        assert!(output.status.success(), "child failed: {output:?}");
        assert!(output.stderr.is_empty(), "diagnostics must stay buffered");

        let stdout = String::from_utf8(output.stdout).expect("child stdout should be utf8");
        let snapshot: serde_json::Value = serde_json::from_str(
            stdout
                .lines()
                .find(|line| line.starts_with('{'))
                .expect("child should print one JSON snapshot"),
        )
        .expect("child output should be a JSON snapshot");
        let matching = snapshot["records"]
            .as_array()
            .expect("diagnostic records")
            .iter()
            .filter(|record| {
                record["event"]["source"] == "core.sliding_sync_capability"
                    && record["event"]["stage"] == "positive_evidence_persistence"
            })
            .collect::<Vec<_>>();
        assert_eq!(matching.len(), 2);
        let outcomes = matching
            .iter()
            .map(|record| record["event"]["fields"][0]["value"]["value"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(outcomes, vec![Some("saved"), Some("failed")]);
        assert!(matching.iter().all(|record| {
            record["event"]["fields"]
                .as_array()
                .is_some_and(|fields| fields.len() == 1 && fields[0]["key"] == "outcome")
        }));
    }

    #[test]
    #[ignore]
    fn sliding_sync_evidence_persistence_diagnostic_child() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        record_sliding_sync_capability_persistence("saved");
        record_sliding_sync_capability_persistence("failed");
        println!(
            "{}",
            serde_json::to_string(&koushi_diagnostics::test_support::detail_snapshot())
                .expect("diagnostic snapshot should serialize")
        );
    }

    #[test]
    fn sliding_sync_revalidation_completion_requires_the_exact_active_request() {
        let active = Some((7, 12));

        assert!(sliding_sync_revalidation_completion_is_current(
            active, 7, 12
        ));
        assert!(!sliding_sync_revalidation_completion_is_current(
            active, 7, 11
        ));
        assert!(!sliding_sync_revalidation_completion_is_current(
            active, 6, 12
        ));
        assert!(!sliding_sync_revalidation_completion_is_current(
            None, 7, 12
        ));
    }

    #[tokio::test]
    async fn unsupported_password_login_never_installs_or_persists_the_session() {
        let homeserver = spawn_unsupported_quarantine_password_server();
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, mut event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let request_id = test_request_id();
        assert!(
            handle
                .send(AccountMessage::Command(AccountCommand::LoginPassword {
                    request_id,
                    request: LoginRequest {
                        homeserver,
                        username: "fixture-user".to_owned(),
                        password: koushi_state::AuthSecret::new("synthetic-password"),
                        device_display_name: None,
                    },
                    platform: koushi_state::DisplayPlatform::Linux,
                }))
                .await
        );
        assert!(matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::SlidingSyncCapabilityCheckStarted {
                admission: SlidingSyncAdmission::NewLogin { .. },
                ..
            }])
        ));
        assert!(matches!(
            recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx)
                .await
                .as_slice(),
            [AppAction::SlidingSyncCapabilityCheckCompleted {
                result: SlidingSyncCapabilityResult::Unsupported,
                ..
            }]
        ));
        assert!(matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::LoginFailed { .. }])
        ));
        assert_eq!(
            inspect_session_runtime(&handle).await,
            (false, false, false, false)
        );
        let backend = CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(
            cred_dir.path(),
        ));
        assert!(backend.load_last_session().expect("last pointer").is_none());
        assert!(
            backend
                .load_saved_sessions()
                .expect("saved sessions")
                .sessions()
                .is_empty()
        );
        assert!(matches!(
            event_rx.recv().await,
            Ok(CoreEvent::OperationFailed {
                request_id: failed_request_id,
                failure: CoreFailure::AccountOperationFailed {
                    kind: AuthFailureKind::Unsupported,
                },
            }) if failed_request_id == request_id
        ));
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn unsupported_restore_preserves_persisted_session_and_positive_evidence() {
        let homeserver = spawn_unsupported_quarantine_password_server();
        let login = koushi_sdk::login_with_password_with_store(
            &LoginRequest {
                homeserver,
                username: "fixture-user".to_owned(),
                password: koushi_state::AuthSecret::new("synthetic-password"),
                device_display_name: None,
            },
            None,
        )
        .await
        .expect("fixture login");
        let expected_info = login.info.clone();
        let key_id = session_key_id_from_info(&login.info);
        let evidence = SlidingSyncPositiveEvidence { observed_at_ms: 7 };
        let stored = StoredMatrixSession::new(
            login
                .persistable_session()
                .expect("persistable")
                .with_sliding_sync_positive_evidence(evidence.clone())
                .to_json()
                .expect("json"),
        );
        drop(login);

        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let backend = CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(
            cred_dir.path(),
        ));
        backend
            .save_matrix_session(&key_id, &stored)
            .expect("session seed");
        backend.remember_saved_session(&key_id).expect("index seed");
        backend.save_last_session(&key_id).expect("pointer seed");

        let (handle, mut action_rx, _event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        assert!(
            handle
                .send(AccountMessage::Command(
                    AccountCommand::RestoreLastSession {
                        request_id: test_request_id(),
                    },
                ))
                .await
        );
        assert!(matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::SlidingSyncCapabilityCheckStarted {
                admission: SlidingSyncAdmission::StoredSessionRestore { .. },
                positive_evidence: Some(saved),
                ..
            }]) if saved == &evidence
        ));
        assert!(matches!(
            recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx)
                .await
                .as_slice(),
            [AppAction::SlidingSyncCapabilityCheckCompleted {
                result: SlidingSyncCapabilityResult::Unsupported,
                ..
            }]
        ));
        assert_eq!(
            inspect_session_runtime(&handle).await,
            (false, false, false, false)
        );
        let persisted = backend
            .load_matrix_session(&key_id)
            .expect("preserved session");
        let reopened = PersistableMatrixSession::from_json(persisted.as_str())
            .expect("preserved session JSON");
        assert_eq!(reopened.info, expected_info);
        assert_eq!(reopened.sliding_sync_positive_evidence(), Some(evidence));
        assert!(backend.load_last_session().expect("last pointer").is_some());

        assert!(
            handle
                .send(AccountMessage::Command(
                    AccountCommand::RetrySlidingSyncCapability {
                        request_id: RequestId {
                            connection_id: RuntimeConnectionId(1),
                            sequence: 2,
                        },
                    },
                ))
                .await
        );
        assert!(matches!(
            recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx)
                .await
                .as_slice(),
            [AppAction::SlidingSyncCapabilityRetryAccepted { .. }]
        ));
        assert!(matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::SlidingSyncCapabilityCheckStarted {
                admission: SlidingSyncAdmission::StoredSessionRestore { .. },
                ..
            }])
        ));
        assert!(matches!(
            recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx)
                .await
                .as_slice(),
            [AppAction::SlidingSyncCapabilityCheckCompleted {
                result: SlidingSyncCapabilityResult::Unsupported,
                ..
            }]
        ));

        handle
            .send(AccountMessage::Command(AccountCommand::ResetLocalData {
                request_id: RequestId {
                    connection_id: RuntimeConnectionId(1),
                    sequence: 3,
                },
            }))
            .await;
        assert!(matches!(
            action_rx.recv().await.as_deref(),
            Some([
                AppAction::ResetLocalDataCompleted { request_id: 3 },
                AppAction::LogoutFinished,
            ])
        ));
        assert!(koushi_key::is_missing_credential_error(
            &backend
                .load_matrix_session(&key_id)
                .expect_err("blocked session persistence should be deleted")
        ));
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    fn spawn_unsupported_quarantine_password_server() -> String {
        spawn_named_quarantine_password_server_with_controls(
            "@fixture-user:example.invalid",
            "FIXTUREDEVICE",
            None,
            None,
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
    }
}
