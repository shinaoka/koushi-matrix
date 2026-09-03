//! `local_data_cleanup` ownership for AccountActor.

use std::time::{Duration, Instant};

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_protocol::SessionKeyId;
use koushi_state::{
    AppAction, DeviceCleanupAuthMode, DeviceCleanupFailureKind, DeviceCleanupRemoteOutcome,
};

use crate::executor;
use koushi_protocol::event::{CoreEvent, LocalEncryptionEvent};
use koushi_protocol::failure::CoreFailure;
use koushi_protocol::ids::RequestId;

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
        self.retire_pending_oidc_login();
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
mod tests;
