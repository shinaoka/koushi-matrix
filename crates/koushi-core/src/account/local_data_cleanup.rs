//! `local_data_cleanup` ownership for AccountActor.

use std::time::{Duration, Instant};

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_key::SessionKeyId;
use koushi_state::{
    AppAction, DeviceCleanupAuthMode, DeviceCleanupFailureKind, DeviceCleanupRemoteOutcome,
};

use crate::event::{CoreEvent, LocalEncryptionEvent};
use crate::executor;
use crate::failure::CoreFailure;
use crate::ids::RequestId;

use super::actor::AccountActor;
use super::runtime_children::next_read_persistence_session_generation;

const DEVICE_CLEANUP_REMOTE_TIMEOUT: Duration = Duration::from_secs(20);

pub(super) struct PendingDeviceCleanup {
    original_request_id: RequestId,
    trust_generation: u64,
    key_id: SessionKeyId,
    stage: PendingDeviceCleanupStage,
}

#[derive(Clone)]
enum PendingDeviceCleanupStage {
    AwaitingUia {
        session: Option<String>,
    },
    RemoteFailed,
    Local {
        remote_outcome: Option<DeviceCleanupRemoteOutcome>,
    },
}

fn local_data_reset_event(stage: &'static str, request_id: RequestId) -> DiagnosticEvent {
    DiagnosticEvent::new(DiagnosticLevel::Info, "core.local_data_reset", stage).field(
        DiagnosticField::request_id(
            "request_id",
            request_id.connection_id.0,
            request_id.sequence,
        ),
    )
}

fn record_local_data_reset_event(event: DiagnosticEvent) {
    koushi_diagnostics::record_and_stderr(event);
}

fn device_cleanup_event(stage: &'static str, request_id: RequestId) -> DiagnosticEvent {
    DiagnosticEvent::new(DiagnosticLevel::Info, "device_cleanup", stage).field(
        DiagnosticField::request_id(
            "request_id",
            request_id.connection_id.0,
            request_id.sequence,
        ),
    )
}

fn record_device_cleanup_event(event: DiagnosticEvent) {
    koushi_diagnostics::record_and_stderr(event);
}

pub(super) fn record_device_cleanup_offer(reason: &'static str) {
    record_device_cleanup_event(
        DiagnosticEvent::new(DiagnosticLevel::Info, "device_cleanup", "offered")
            .field(DiagnosticField::token("reason", reason)),
    );
}

fn device_cleanup_failure_token(kind: DeviceCleanupFailureKind) -> &'static str {
    match kind {
        DeviceCleanupFailureKind::Network => "network",
        DeviceCleanupFailureKind::Forbidden => "forbidden",
        DeviceCleanupFailureKind::Timeout => "timeout",
        DeviceCleanupFailureKind::Sdk => "sdk",
        DeviceCleanupFailureKind::LocalData => "local_data",
    }
}

impl AccountActor {
    pub(super) async fn handle_probe_local_encryption_health(&self, request_id: RequestId) {
        let health = if let Some(key_id) = self.session_key_id.clone() {
            let store = self.store.clone();
            executor::spawn_blocking(move || store.probe_local_encryption_health(&key_id))
                .await
                .unwrap_or(koushi_state::LocalEncryptionHealth::Unavailable)
        } else {
            koushi_state::LocalEncryptionHealth::Unknown
        };
        self.send_actions(vec![AppAction::LocalEncryptionHealthChanged {
            request_id: request_id.sequence,
            health,
        }])
        .await;
        self.emit(CoreEvent::LocalEncryption(
            LocalEncryptionEvent::HealthChanged { health },
        ));
    }

    pub(super) async fn handle_start_device_cleanup(&mut self, request_id: RequestId) {
        if let Some(mut pending) = self.pending_device_cleanup.take() {
            if matches!(&pending.stage, PendingDeviceCleanupStage::Local { .. }) {
                pending.original_request_id = request_id;
                self.finish_device_cleanup_local(pending).await;
                return;
            }
        }
        self.run_device_cleanup_remote(request_id, None, None).await;
    }

    pub(super) async fn handle_submit_device_cleanup_uia(
        &mut self,
        request_id: RequestId,
        flow_id: u64,
        password: koushi_state::AuthSecret,
    ) {
        let Some(pending) = self.pending_device_cleanup.take() else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        let uiaa_session = match &pending.stage {
            PendingDeviceCleanupStage::AwaitingUia { session } => session.clone(),
            PendingDeviceCleanupStage::RemoteFailed | PendingDeviceCleanupStage::Local { .. } => {
                self.pending_device_cleanup = Some(pending);
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };
        if pending.original_request_id.sequence != flow_id
            || pending.trust_generation != self.trust_generation
            || self.session_key_id.as_ref() != Some(&pending.key_id)
        {
            self.pending_device_cleanup = Some(pending);
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        }
        self.run_device_cleanup_remote(
            pending.original_request_id,
            Some(&password),
            uiaa_session.as_deref(),
        )
        .await;
    }

    pub(super) async fn handle_erase_device_cleanup_local_data_anyway(
        &mut self,
        request_id: RequestId,
    ) {
        let Some(failed) = self.pending_device_cleanup.take() else {
            self.send_actions(vec![AppAction::DeviceCleanupLocalResetFailed {
                request_id: request_id.sequence,
                kind: DeviceCleanupFailureKind::LocalData,
            }])
            .await;
            return;
        };
        if !matches!(&failed.stage, PendingDeviceCleanupStage::RemoteFailed)
            || failed.trust_generation != self.trust_generation
            || self.session_key_id.as_ref() != Some(&failed.key_id)
        {
            self.pending_device_cleanup = Some(failed);
            self.send_actions(vec![AppAction::DeviceCleanupLocalResetFailed {
                request_id: request_id.sequence,
                kind: DeviceCleanupFailureKind::LocalData,
            }])
            .await;
            return;
        }
        self.finish_device_cleanup_local(PendingDeviceCleanup {
            original_request_id: request_id,
            trust_generation: self.trust_generation,
            key_id: failed.key_id,
            stage: PendingDeviceCleanupStage::Local {
                remote_outcome: None,
            },
        })
        .await;
    }

    async fn run_device_cleanup_remote(
        &mut self,
        request_id: RequestId,
        password: Option<&koushi_state::AuthSecret>,
        uiaa_session: Option<&str>,
    ) {
        let missing_context_auth_mode = if password.is_some() {
            DeviceCleanupAuthMode::Legacy
        } else {
            DeviceCleanupAuthMode::Unknown
        };
        let Some(session) = self.session.clone() else {
            let mut actions = Vec::new();
            if password.is_none() {
                actions.push(AppAction::DeviceCleanupRemoteStarted {
                    request_id: request_id.sequence,
                    auth_mode: missing_context_auth_mode,
                });
            }
            actions.push(AppAction::DeviceCleanupRemoteFailed {
                request_id: request_id.sequence,
                auth_mode: missing_context_auth_mode,
                kind: DeviceCleanupFailureKind::Sdk,
            });
            self.send_actions(actions).await;
            return;
        };
        let Some(key_id) = self.session_key_id.clone() else {
            let mut actions = Vec::new();
            if password.is_none() {
                actions.push(AppAction::DeviceCleanupRemoteStarted {
                    request_id: request_id.sequence,
                    auth_mode: missing_context_auth_mode,
                });
            }
            actions.push(AppAction::DeviceCleanupRemoteFailed {
                request_id: request_id.sequence,
                auth_mode: missing_context_auth_mode,
                kind: DeviceCleanupFailureKind::Sdk,
            });
            self.send_actions(actions).await;
            return;
        };
        let trust_generation = self.trust_generation;
        let auth_mode = session.device_cleanup_auth_mode();
        let is_uia_continuation = password.is_some();
        if !is_uia_continuation {
            self.send_actions(vec![AppAction::DeviceCleanupRemoteStarted {
                request_id: request_id.sequence,
                auth_mode,
            }])
            .await;
        }
        record_device_cleanup_event(
            device_cleanup_event(
                if is_uia_continuation {
                    "uia_submitted"
                } else {
                    "remote_started"
                },
                request_id,
            )
            .field(DiagnosticField::token(
                "auth_mode",
                match auth_mode {
                    DeviceCleanupAuthMode::Legacy => "legacy",
                    DeviceCleanupAuthMode::OAuth => "oauth",
                    DeviceCleanupAuthMode::Unknown => "unknown",
                },
            )),
        );

        #[cfg(test)]
        let configured_result = self.device_cleanup_results.pop_front();
        let result = executor::timeout(DEVICE_CLEANUP_REMOTE_TIMEOUT, async {
            #[cfg(test)]
            if let Some(result) = configured_result {
                return result;
            }
            koushi_sdk::cleanup_current_device(&session, password, uiaa_session).await
        })
        .await
        .unwrap_or(Err(DeviceCleanupFailureKind::Timeout));
        if trust_generation != self.trust_generation
            || self.session_key_id.as_ref() != Some(&key_id)
        {
            record_device_cleanup_event(
                device_cleanup_event("stale_ignored", request_id)
                    .field(DiagnosticField::token("stage", "remote_settlement")),
            );
            return;
        }
        match result {
            Ok(koushi_sdk::MatrixDeviceCleanupOutcome::UiaaRequired { session }) => {
                debug_assert_eq!(auth_mode, DeviceCleanupAuthMode::Legacy);
                self.pending_device_cleanup = Some(PendingDeviceCleanup {
                    original_request_id: request_id,
                    trust_generation,
                    key_id,
                    stage: PendingDeviceCleanupStage::AwaitingUia { session },
                });
                self.send_actions(vec![AppAction::DeviceCleanupUiaRequired {
                    request_id: request_id.sequence,
                    flow_id: request_id.sequence,
                }])
                .await;
                record_device_cleanup_event(device_cleanup_event("uia_required", request_id));
            }
            Ok(koushi_sdk::MatrixDeviceCleanupOutcome::Settled(outcome)) => {
                self.send_actions(vec![AppAction::DeviceCleanupRemoteSettled {
                    request_id: request_id.sequence,
                    outcome,
                }])
                .await;
                record_device_cleanup_event(
                    device_cleanup_event("remote_settled", request_id).field(
                        DiagnosticField::token(
                            "outcome",
                            match outcome {
                                DeviceCleanupRemoteOutcome::Success => "success",
                                DeviceCleanupRemoteOutcome::AlreadyAbsent => "already_absent",
                            },
                        ),
                    ),
                );
                self.finish_device_cleanup_local(PendingDeviceCleanup {
                    original_request_id: request_id,
                    trust_generation,
                    key_id,
                    stage: PendingDeviceCleanupStage::Local {
                        remote_outcome: Some(outcome),
                    },
                })
                .await;
            }
            Err(kind) => {
                self.pending_device_cleanup = Some(PendingDeviceCleanup {
                    original_request_id: request_id,
                    trust_generation,
                    key_id,
                    stage: PendingDeviceCleanupStage::RemoteFailed,
                });
                self.send_actions(vec![AppAction::DeviceCleanupRemoteFailed {
                    request_id: request_id.sequence,
                    auth_mode,
                    kind,
                }])
                .await;
                record_device_cleanup_event(
                    device_cleanup_event("remote_failed", request_id).field(
                        DiagnosticField::token("failure_kind", device_cleanup_failure_token(kind)),
                    ),
                );
            }
        }
    }

    async fn finish_device_cleanup_local(&mut self, mut pending: PendingDeviceCleanup) {
        let request_id = pending.original_request_id;
        if pending.trust_generation != self.trust_generation
            || self.session_key_id.as_ref() != Some(&pending.key_id)
        {
            record_device_cleanup_event(
                device_cleanup_event("stale_ignored", request_id)
                    .field(DiagnosticField::token("stage", "local_reset")),
            );
            return;
        }
        let PendingDeviceCleanupStage::Local { remote_outcome } = &pending.stage else {
            return;
        };
        let mut local_started = device_cleanup_event("local_reset_started", request_id).field(
            DiagnosticField::boolean("remote_may_remain", remote_outcome.is_none()),
        );
        if let Some(outcome) = *remote_outcome {
            local_started = local_started.field(DiagnosticField::token(
                "outcome",
                match outcome {
                    DeviceCleanupRemoteOutcome::Success => "success",
                    DeviceCleanupRemoteOutcome::AlreadyAbsent => "already_absent",
                },
            ));
        }
        record_device_cleanup_event(local_started);
        if !self.stop_current_session_runtime().await {
            // Fail closed: retain the pending cleanup state and report the
            // failure instead of continuing to close stores or drop the
            // session (issue #538).
            record(DiagnosticEvent::new(
                DiagnosticLevel::Warn,
                "core.room_key_debug",
                "device_cleanup_teardown_unconfirmed",
            ));
            // Refresh the retained generation: teardown bumped
            // trust_generation, and a stale value would make the retry
            // reject the pending entry as outdated (issue #538).
            pending.trust_generation = self.trust_generation;
            self.pending_device_cleanup = Some(pending);
            self.send_device_cleanup_local_failure(request_id).await;
            return;
        }
        pending.trust_generation = self.trust_generation;
        let active_session = self.session.clone();
        let stores_closed = match active_session.as_deref() {
            Some(session) => self.close_pending_session_stores(session).await.is_ok(),
            None => false,
        };
        if !stores_closed {
            self.pending_device_cleanup = Some(pending);
            self.send_device_cleanup_local_failure(request_id).await;
            return;
        }
        self.read_persistence_session_generation = next_read_persistence_session_generation();
        self.store.invalidate_read_state_outbox_saves(
            &pending.key_id,
            self.read_persistence_session_generation,
        );
        if !self.clear_account_persistence(&pending.key_id).await {
            self.pending_device_cleanup = Some(pending);
            self.send_device_cleanup_local_failure(request_id).await;
            return;
        }

        self.pending_device_cleanup = None;
        self.session_key_id.take();
        self.provisional_persistable.take();
        self.session_promoted = false;
        drop(self.session.take());
        self.send_actions(vec![AppAction::DeviceCleanupCompleted {
            request_id: request_id.sequence,
        }])
        .await;
        record_device_cleanup_event(device_cleanup_event("completed", request_id));
    }

    async fn send_device_cleanup_local_failure(&self, request_id: RequestId) {
        self.send_actions(vec![AppAction::DeviceCleanupLocalResetFailed {
            request_id: request_id.sequence,
            kind: DeviceCleanupFailureKind::LocalData,
        }])
        .await;
        record_device_cleanup_event(
            device_cleanup_event("local_reset_failed", request_id).field(DiagnosticField::token(
                "failure_kind",
                device_cleanup_failure_token(DeviceCleanupFailureKind::LocalData),
            )),
        );
    }

    pub(super) async fn handle_reset_local_data(&mut self, request_id: RequestId) {
        let started_at = Instant::now();
        let key_id = self.session_key_id.clone().or_else(|| {
            self.pending_sliding_sync_admission
                .as_ref()
                .map(|pending| pending.key_id().clone())
        });
        record_local_data_reset_event(local_data_reset_event("started", request_id).field(
            DiagnosticField::boolean("session_key_available", key_id.is_some()),
        ));
        let Some(key_id) = key_id else {
            record_local_data_reset_event(
                local_data_reset_event("rejected", request_id)
                    .field(DiagnosticField::token("reason", "session_key_unavailable"))
                    .field(DiagnosticField::milliseconds(
                        "elapsed_ms",
                        started_at.elapsed().as_millis(),
                    )),
            );
            self.send_actions(vec![AppAction::ResetLocalDataFailed {
                request_id: request_id.sequence,
            }])
            .await;
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        record_local_data_reset_event(local_data_reset_event(
            "session_runtime_stop_started",
            request_id,
        ));
        self.cancel_sliding_sync_discovery_task().await;
        self.discard_pending_sliding_sync_admission().await;
        self.pending_sliding_sync_retry = None;
        self.stored_sliding_sync_admission = None;
        self.sliding_sync_revalidation_pending = None;
        if !self.stop_current_session_runtime().await {
            // Fail closed: do not close stores, take keys, drop the session,
            // or delete persistence unless the encryption-debug operation is
            // confirmed settled (issue #538).
            record(DiagnosticEvent::new(
                DiagnosticLevel::Warn,
                "core.room_key_debug",
                "local_data_reset_teardown_unconfirmed",
            ));
            self.send_actions(vec![AppAction::ResetLocalDataFailed {
                request_id: request_id.sequence,
            }])
            .await;
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        }
        record_local_data_reset_event(
            local_data_reset_event("session_runtime_stop_finished", request_id).field(
                DiagnosticField::milliseconds("elapsed_ms", started_at.elapsed().as_millis()),
            ),
        );
        self.session_key_id.take();
        self.read_persistence_session_generation = next_read_persistence_session_generation();
        self.store
            .invalidate_read_state_outbox_saves(&key_id, self.read_persistence_session_generation);

        drop(self.session.take());
        record_local_data_reset_event(local_data_reset_event(
            "persistence_clear_started",
            request_id,
        ));
        self.clear_account_persistence(&key_id).await;
        record_local_data_reset_event(
            local_data_reset_event("persistence_clear_finished", request_id).field(
                DiagnosticField::milliseconds("elapsed_ms", started_at.elapsed().as_millis()),
            ),
        );
        self.send_actions(vec![
            AppAction::ResetLocalDataCompleted {
                request_id: request_id.sequence,
            },
            AppAction::LogoutFinished,
        ])
        .await;
        record_local_data_reset_event(local_data_reset_event("completed", request_id).field(
            DiagnosticField::milliseconds("elapsed_ms", started_at.elapsed().as_millis()),
        ));
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, HashMap},
        sync::{Arc, atomic::AtomicU64},
        time::Duration,
    };

    use koushi_key::{SessionKeyId, StoredMatrixSession};

    use koushi_state::{
        AppAction, DeviceCleanupAuthMode, DeviceCleanupFailureKind, DeviceCleanupRemoteOutcome,
    };

    use tokio::sync::{Semaphore, broadcast, mpsc, oneshot};

    use crate::account::actor::{AccountActor, AccountActorHandle, AccountMessage};
    use crate::account::profile::AVATAR_DOWNLOAD_CONCURRENCY;
    use crate::account::test_support::{
        inspect_session_runtime, login_gated_actor, test_request_id,
    };
    use crate::account::verification::INCOMING_VERIFICATION_FLOW_ID_BASE;
    use crate::command::AccountCommand;
    use crate::composer_draft_lifecycle::ComposerDraftLeaseRegistry;

    use crate::executor;

    use crate::ids::{RequestId, RuntimeConnectionId};
    use crate::link_preview::LinkPreviewContext;

    use crate::store::CredentialStoreBackend;
    use crate::store::StoreActor;

    use crate::timeline::NavigationProjectionIngress;

    use tempfile::tempdir;

    async fn next_device_cleanup_actions(
        action_rx: &mut mpsc::Receiver<Vec<AppAction>>,
    ) -> Vec<AppAction> {
        executor::timeout(Duration::from_secs(2), async {
            loop {
                let actions = action_rx
                    .recv()
                    .await
                    .expect("device cleanup action channel");
                if actions.iter().any(|action| {
                    matches!(
                        action,
                        AppAction::DeviceCleanupRemoteStarted { .. }
                            | AppAction::DeviceCleanupUiaRequired { .. }
                            | AppAction::DeviceCleanupRemoteSettled { .. }
                            | AppAction::DeviceCleanupRemoteFailed { .. }
                            | AppAction::DeviceCleanupLocalResetFailed { .. }
                            | AppAction::DeviceCleanupCompleted { .. }
                    )
                }) {
                    return actions;
                }
            }
        })
        .await
        .expect("device cleanup action timeout")
    }

    #[tokio::test]
    async fn device_cleanup_remote_failure_preserves_the_provisional_session() {
        let (handle, mut action_rx) = login_gated_actor().await;
        handle
            .send(AccountMessage::ConfigureDeviceCleanupResults {
                results: vec![Err(DeviceCleanupFailureKind::Network)],
            })
            .await;
        let request_id = RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: 301,
        };

        handle
            .send(AccountMessage::Command(
                AccountCommand::StartDeviceCleanup { request_id },
            ))
            .await;

        assert!(matches!(
            next_device_cleanup_actions(&mut action_rx).await.as_slice(),
            [AppAction::DeviceCleanupRemoteStarted {
                request_id: 301,
                auth_mode: DeviceCleanupAuthMode::Legacy,
            }]
        ));
        assert!(matches!(
            next_device_cleanup_actions(&mut action_rx).await.as_slice(),
            [AppAction::DeviceCleanupRemoteFailed {
                request_id: 301,
                auth_mode: DeviceCleanupAuthMode::Legacy,
                kind: DeviceCleanupFailureKind::Network,
            }]
        ));
        assert!(
            inspect_session_runtime(&handle).await.0,
            "remote failure must retain the provisional SDK session"
        );
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn device_cleanup_uia_and_local_retry_do_not_repeat_remote_cleanup() {
        let (handle, mut action_rx) = login_gated_actor().await;
        handle
            .send(AccountMessage::ConfigureDeviceCleanupResults {
                results: vec![
                    Ok(koushi_sdk::MatrixDeviceCleanupOutcome::UiaaRequired {
                        session: Some("opaque-test-session".to_owned()),
                    }),
                    Ok(koushi_sdk::MatrixDeviceCleanupOutcome::Settled(
                        DeviceCleanupRemoteOutcome::Success,
                    )),
                ],
            })
            .await;
        handle
            .send(AccountMessage::ConfigureCloseStoreResults {
                results: vec![false, true],
            })
            .await;
        let start_request_id = RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: 401,
        };
        handle
            .send(AccountMessage::Command(
                AccountCommand::StartDeviceCleanup {
                    request_id: start_request_id,
                },
            ))
            .await;
        assert!(matches!(
            next_device_cleanup_actions(&mut action_rx).await.as_slice(),
            [AppAction::DeviceCleanupRemoteStarted {
                request_id: 401,
                ..
            }]
        ));
        assert!(matches!(
            next_device_cleanup_actions(&mut action_rx).await.as_slice(),
            [AppAction::DeviceCleanupUiaRequired {
                request_id: 401,
                flow_id: 401,
            }]
        ));

        handle
            .send(AccountMessage::Command(
                AccountCommand::SubmitDeviceCleanupUia {
                    request_id: RequestId {
                        connection_id: RuntimeConnectionId(1),
                        sequence: 402,
                    },
                    flow_id: 401,
                    password: koushi_state::AuthSecret::new("test-password"),
                },
            ))
            .await;
        assert!(matches!(
            next_device_cleanup_actions(&mut action_rx).await.as_slice(),
            [AppAction::DeviceCleanupRemoteSettled {
                request_id: 401,
                outcome: DeviceCleanupRemoteOutcome::Success,
            }]
        ));
        assert!(matches!(
            next_device_cleanup_actions(&mut action_rx).await.as_slice(),
            [AppAction::DeviceCleanupLocalResetFailed {
                request_id: 401,
                kind: DeviceCleanupFailureKind::LocalData,
            }]
        ));

        let retry_request_id = RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: 403,
        };
        handle
            .send(AccountMessage::Command(
                AccountCommand::StartDeviceCleanup {
                    request_id: retry_request_id,
                },
            ))
            .await;
        assert!(matches!(
            next_device_cleanup_actions(&mut action_rx).await.as_slice(),
            [AppAction::DeviceCleanupCompleted { request_id: 403 }]
        ));
        assert!(
            !inspect_session_runtime(&handle).await.0,
            "successful local retry must drop the provisional SDK session"
        );
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn device_cleanup_local_only_escape_runs_only_after_remote_failure() {
        let (handle, mut action_rx) = login_gated_actor().await;
        handle
            .send(AccountMessage::ConfigureDeviceCleanupResults {
                results: vec![Err(DeviceCleanupFailureKind::Forbidden)],
            })
            .await;
        let start_request_id = RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: 501,
        };
        handle
            .send(AccountMessage::Command(
                AccountCommand::StartDeviceCleanup {
                    request_id: start_request_id,
                },
            ))
            .await;
        let _ = next_device_cleanup_actions(&mut action_rx).await;
        assert!(matches!(
            next_device_cleanup_actions(&mut action_rx).await.as_slice(),
            [AppAction::DeviceCleanupRemoteFailed {
                request_id: 501,
                kind: DeviceCleanupFailureKind::Forbidden,
                ..
            }]
        ));

        handle
            .send(AccountMessage::Command(
                AccountCommand::EraseDeviceCleanupLocalDataAnyway {
                    request_id: RequestId {
                        connection_id: RuntimeConnectionId(1),
                        sequence: 502,
                    },
                },
            ))
            .await;
        assert!(matches!(
            next_device_cleanup_actions(&mut action_rx).await.as_slice(),
            [AppAction::DeviceCleanupCompleted { request_id: 502 }]
        ));
        assert!(!inspect_session_runtime(&handle).await.0);
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn provisional_teardown_drops_actor_private_device_cleanup_continuation() {
        let (handle, mut action_rx) = login_gated_actor().await;
        handle
            .send(AccountMessage::ConfigureDeviceCleanupResults {
                results: vec![Ok(koushi_sdk::MatrixDeviceCleanupOutcome::UiaaRequired {
                    session: Some("opaque-test-session".to_owned()),
                })],
            })
            .await;
        handle
            .send(AccountMessage::Command(
                AccountCommand::StartDeviceCleanup {
                    request_id: RequestId {
                        connection_id: RuntimeConnectionId(1),
                        sequence: 601,
                    },
                },
            ))
            .await;
        let _ = next_device_cleanup_actions(&mut action_rx).await;
        assert!(matches!(
            next_device_cleanup_actions(&mut action_rx).await.as_slice(),
            [AppAction::DeviceCleanupUiaRequired {
                request_id: 601,
                flow_id: 601,
            }]
        ));
        assert!(inspect_pending_device_cleanup(&handle).await);

        handle
            .send(AccountMessage::RejectProvisionalSession {
                request_id: RequestId {
                    connection_id: RuntimeConnectionId(1),
                    sequence: 602,
                },
            })
            .await;
        executor::timeout(Duration::from_secs(2), async {
            loop {
                if matches!(
                    action_rx.recv().await.as_deref(),
                    Some([AppAction::LogoutFinished])
                ) {
                    break;
                }
            }
        })
        .await
        .expect("provisional rejection settles");

        assert!(
            !inspect_pending_device_cleanup(&handle).await,
            "teardown must discard actor-private UIAA continuation state"
        );
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    async fn inspect_pending_device_cleanup(handle: &AccountActorHandle) -> bool {
        let (response, result) = oneshot::channel();
        assert!(
            handle
                .send(AccountMessage::InspectPendingDeviceCleanup { response })
                .await
        );
        result.await.expect("pending device cleanup inspection")
    }

    #[tokio::test]
    async fn reset_local_data_clears_current_account_persistence_and_signs_out_locally() {
        use crate::read_state::{ReadStateEngine, ReadStateKey, ReadTarget, ReadWaiterId};

        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let key_id = SessionKeyId {
            homeserver: "https://example.test".to_owned(),
            user_id: "@reset-user:example.test".to_owned(),
            device_id: "RESETDEVICE".to_owned(),
        };
        let store = StoreActor::with_backend(
            CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(
                cred_dir.path(),
            )),
            data_dir.path(),
        );
        let store_config = store
            .account_store_config(&key_id)
            .expect("seed local unlock secret");
        let account_root = store_config
            .store_config
            .path()
            .parent()
            .expect("store path should have account root")
            .to_path_buf();
        std::fs::create_dir_all(store_config.store_config.path()).expect("create store dir");
        std::fs::write(
            store_config.store_config.path().join("sentinel"),
            b"local data",
        )
        .expect("write local store sentinel");
        let mut read_state = ReadStateEngine::new(1);
        read_state.admit(
            1,
            ReadStateKey::PublicUnthreaded {
                room_id: "!reset-room:example.test".to_owned(),
            },
            ReadTarget::new("$reset-event".to_owned()),
            ReadWaiterId::new(1),
        );
        store
            .save_read_state_outbox(&key_id, &read_state.persistence_snapshot())
            .expect("seed read-state outbox");
        store
            .credential_backend()
            .save_matrix_session(&key_id, &StoredMatrixSession::new("{\"redacted\":true}"))
            .expect("seed session");
        store
            .credential_backend()
            .remember_saved_session(&key_id)
            .expect("seed saved-session index");
        store
            .credential_backend()
            .save_last_session(&key_id)
            .expect("seed last-session pointer");
        assert_eq!(
            store.probe_local_encryption_health(&key_id),
            koushi_state::LocalEncryptionHealth::Healthy
        );

        let (action_tx, mut action_rx) = mpsc::channel(16);
        let (event_tx, _) = broadcast::channel(16);
        let (self_tx, command_rx) = mpsc::channel(16);
        let data_dir_path = store.data_dir().to_path_buf();
        let account_work = crate::account_work::AccountWorkScheduler::default();
        let room_actor = crate::room::RoomActor::spawn_with_account_work(
            action_tx.clone(),
            event_tx.clone(),
            crate::SlidingSyncDiagnostics::default(),
            account_work.clone(),
        );
        let (navigation_projection, navigation_projection_rx) =
            NavigationProjectionIngress::channel();
        let timeline_manager = crate::timeline::TimelineManagerActor::spawn(
            action_tx.clone(),
            event_tx.clone(),
            Some(data_dir_path.clone()),
            account_work.clone(),
            Some(navigation_projection_rx),
        );
        let mut actor = AccountActor {
            session: None,
            session_key_id: Some(key_id.clone()),
            provisional_persistable: None,
            sliding_sync_positive_evidence: None,
            sliding_sync_account_epoch: 0,
            sliding_sync_request_id: 0,
            pending_sliding_sync_admission: None,
            pending_sliding_sync_retry: None,
            stored_sliding_sync_admission: None,
            sliding_sync_discovery_task: None,
            sliding_sync_revalidation_pending: None,
            sliding_sync_revalidation_request: None,
            sliding_sync_diagnostics: crate::SlidingSyncDiagnostics::default(),
            session_promoted: false,
            trust_generation: 0,
            trust_observer: None,
            trust_recheck_task: None,
            trust_recheck_pending: false,
            current_session_status_task: None,
            current_session_status_request: None,
            secure_backup_ready: false,
            recovery_key_delivery_pending: false,
            secure_backup_inspection_task: None,
            secure_backup_monitor_task: None,
            secure_backup_monitor_serial: 0,
            secure_backup_inspection_pending: false,
            secure_backup_observer: None,
            verification_method_discovery_task: None,
            verification_method_discovery_serial: 0,
            verification_method_discovery_failed: false,
            recovery_task: None,
            pending_recovery_completion: None,
            recovery_trust_settlement_task: None,
            provisional_encryption_sync: None,
            provisional_encryption_sync_ready: false,
            encryption_sync_permit: koushi_sdk::new_encryption_sync_permit_owner(),
            pending_ready_events: Vec::new(),
            pending_trust_transition: None,
            next_trust_transition_id: 0,
            pending_session_teardown: None,
            next_teardown_generation: 0,
            teardown_retry_task: None,
            lifecycle_probe: None,
            residency_install_gap: None,
            #[cfg(feature = "test-hooks")]
            residency_teardown_gap: None,
            #[cfg(feature = "test-hooks")]
            residency_preserve_room_session: false,
            trust_observation_override: std::sync::Mutex::new(None),
            trust_observation_is_synthetic: false,
            recovery_download_override: std::sync::Mutex::new(None),
            recovery_result_override: std::sync::Mutex::new(None),
            close_store_results: std::collections::VecDeque::new(),
            device_cleanup_results: std::collections::VecDeque::new(),
            store: store.clone(),
            action_tx,
            event_tx,
            command_rx,
            self_tx,
            sync_actor: None,
            sync_generation: Arc::new(AtomicU64::new(0)),
            room_actor,
            timeline_manager,
            read_persistence_task: None,
            read_persistence_session_generation: 0,
            navigation_projection,
            account_work,
            activity_resolution_task: None,
            data_dir: data_dir_path,
            link_preview_policy: LinkPreviewContext::default(),
            send_read_receipts: true,
            pending_oidc_login: None,
            oidc_completion_override: None,
            search_actor: None,
            threads_list_actor: None,
            recovery_observer: None,
            identity_reset_handle: None,
            identity_reset_flow_id: None,
            identity_reset_timeout_task: None,
            device_session_ordinals: BTreeMap::new(),
            pending_uia_operations: BTreeMap::new(),
            pending_device_cleanup: None,
            verification_request: None,
            sas_verification: None,
            own_user_verification: None,
            sas_waiting_for: None,
            verification_request_observer: None,
            sas_verification_observer: None,
            sas_timeout_task: None,
            synthetic_verification: None,
            incoming_verification_observer: None,
            incoming_verification_session_generation: 0,
            session_change_observer: None,
            account_hydration_task: None,
            account_hydration_generation: 0,
            composer_draft_leases: Arc::new(ComposerDraftLeaseRegistry::new()),
            next_incoming_verification_sequence: INCOMING_VERIFICATION_FLOW_ID_BASE,
            pending_crawler_notification: None,
            avatar_cache: HashMap::new(),
            avatar_inflight: HashMap::new(),
            avatar_download_semaphore: Arc::new(Semaphore::new(AVATAR_DOWNLOAD_CONCURRENCY)),
            avatar_fetch_tasks: tokio::task::JoinSet::new(),
            avatar_session_generation: 0,
        };
        let request_id = test_request_id();

        actor.handle_reset_local_data(request_id).await;

        let actions = action_rx.recv().await.expect("reset actions");
        assert!(
            matches!(
                actions.as_slice(),
                [
                    AppAction::ResetLocalDataCompleted { request_id: 1 },
                    AppAction::LogoutFinished,
                ]
            ),
            "reset must complete and locally sign out, got {actions:?}"
        );
        assert!(!account_root.exists(), "account root should be removed");
        assert!(
            store
                .load_read_state_outbox(&key_id)
                .expect("removed read-state outbox reads as empty")
                .is_empty()
        );

        let check_backend = CredentialStoreBackend::FileDir(
            crate::store::FileCredentialStore::new(cred_dir.path()),
        );
        assert!(koushi_key::is_missing_credential_error(
            &check_backend
                .load_matrix_session(&key_id)
                .expect_err("matrix session should be deleted")
        ));
        assert!(
            check_backend
                .load_saved_sessions()
                .expect("saved-session index")
                .sessions()
                .is_empty()
        );
        assert_eq!(
            check_backend
                .load_last_session()
                .expect("last-session pointer"),
            None
        );
        let check_store = StoreActor::with_backend(
            CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(
                cred_dir.path(),
            )),
            data_dir.path(),
        );
        assert_eq!(
            check_store.probe_local_encryption_health(&key_id),
            koushi_state::LocalEncryptionHealth::MissingCredential
        );
    }
}
