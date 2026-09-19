//! `runtime_children` ownership for AccountActor.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_protocol::SessionKeyId;
use koushi_sdk::MatrixClientSession;
use tokio::sync::{oneshot, watch};

use crate::executor;
use crate::room::RoomMessage;
use crate::store::StoreActor;
use crate::timeline::{ReadPersistenceIngress, ReadPersistenceRequest};

use super::actor::AccountActor;
use super::session_lifecycle::{SessionTeardownContinuation, trace_restore_simple};

const READ_PERSISTENCE_DEBOUNCE: Duration = Duration::from_millis(100);

const READ_PERSISTENCE_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

static READ_PERSISTENCE_SESSION_SERIAL: AtomicU64 = AtomicU64::new(0);

fn record_read_persistence(
    stage: &'static str,
    outcome: &'static str,
    session_generation: u64,
    save_generation: u64,
    entry_count: usize,
    candidate_count: usize,
) {
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "core.read_state_persistence", stage)
            .field(DiagnosticField::token("outcome", outcome))
            .field(DiagnosticField::count(
                "session_generation",
                session_generation,
            ))
            .field(DiagnosticField::count("save_generation", save_generation))
            .field(DiagnosticField::count(
                "entry_count",
                entry_count.try_into().unwrap_or(u64::MAX),
            ))
            .field(DiagnosticField::count(
                "candidate_count",
                candidate_count.try_into().unwrap_or(u64::MAX),
            )),
    );
}

pub(super) fn next_read_persistence_session_generation() -> u64 {
    READ_PERSISTENCE_SESSION_SERIAL
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1)
        .max(1)
}

async fn run_read_persistence_worker(
    store: StoreActor,
    key_id: SessionKeyId,
    session_generation: u64,
    mut requests: watch::Receiver<Option<ReadPersistenceRequest>>,
) {
    let mut accepted_save_generation = 0;
    while requests.changed().await.is_ok() {
        executor::sleep(READ_PERSISTENCE_DEBOUNCE).await;
        let request = requests.borrow_and_update().clone();
        let Some(request) = request else {
            continue;
        };
        if request.session_generation() != session_generation
            || request.save_generation() <= accepted_save_generation
        {
            record_read_persistence(
                "save",
                "stale_rejected",
                request.session_generation(),
                request.save_generation(),
                request.snapshot().entry_count(),
                request.snapshot().candidate_count(),
            );
            continue;
        }
        let save_generation = request.save_generation();
        let snapshot = request.snapshot().clone();
        let entry_count = snapshot.entry_count();
        let candidate_count = snapshot.candidate_count();
        let save_store = store.clone();
        let save_key_id = key_id.clone();
        let outcome = executor::spawn_blocking(move || {
            save_store.save_read_state_outbox_if_current(
                &save_key_id,
                session_generation,
                save_generation,
                &snapshot,
            )
        })
        .await;
        match outcome {
            Ok(Ok(true)) => {
                accepted_save_generation = save_generation;
                record_read_persistence(
                    "save",
                    "saved",
                    session_generation,
                    save_generation,
                    entry_count,
                    candidate_count,
                );
            }
            Ok(Ok(false)) => record_read_persistence(
                "save",
                "stale_rejected",
                session_generation,
                save_generation,
                entry_count,
                candidate_count,
            ),
            Ok(Err(_)) | Err(_) => record_read_persistence(
                "save",
                "failed",
                session_generation,
                save_generation,
                entry_count,
                candidate_count,
            ),
        }
    }
}

impl AccountActor {
    pub(super) async fn shutdown_owned_runtime(&mut self) -> bool {
        self.cancel_sliding_sync_discovery_task().await;
        self.discard_pending_sliding_sync_admission().await;
        self.pending_sliding_sync_retry = None;
        self.stored_sliding_sync_admission = None;
        self.sliding_sync_revalidation_pending = None;
        self.sliding_sync_revalidation_request = None;
        if let Some(task) = self.teardown_retry_task.take() {
            task.abort();
            let _ = task.await;
            self.record_lifecycle_probe("teardown_retry_terminated");
        }
        let mut cleanup_ok = self.stop_current_session_runtime().await;
        if let Some(session) = self.session.take() {
            cleanup_ok &= koushi_sdk::close_session_stores(&session).await.is_ok();
            drop(session);
            self.record_lifecycle_probe("current_session_released");
        }
        if let Some(pending) = self.pending_session_teardown.take() {
            cleanup_ok &= koushi_sdk::close_session_stores(&pending.session)
                .await
                .is_ok();
            drop(pending.session);
            if let SessionTeardownContinuation::InstallReplacement { session, .. } =
                pending.continuation
            {
                cleanup_ok &= koushi_sdk::close_session_stores(&session).await.is_ok();
                drop(session);
            }
            self.record_lifecycle_probe("pending_teardown_sessions_released");
        }
        cleanup_ok
    }

    /// Ordered shutdown of the SearchActor (step 3 of the shutdown sequence,
    /// after timelines and before sync — canon Async rule 12 step 3).
    async fn stop_search_actor(&mut self) {
        // Clear any buffered notification so it is not replayed for the next
        // session after logout or account switch.
        self.pending_crawler_notification = None;
        if let Some(handle) = self.search_actor.take() {
            handle.shutdown().await;
        }
    }

    pub(super) async fn stop_current_session_runtime(&mut self) -> bool {
        self.set_secure_backup_send_admitted(false);
        self.recovery_key_delivery_pending = false;
        // Retire the renderer before any account-owned child can be replaced.
        // Already-admitted command permits remain live until their exact
        // reducer settlement, but no producer from the retired generation can
        // enter a new command.
        self.composer_draft_leases.revoke_live_generation();
        self.stop_recovery_task().await;
        self.stop_recovery_trust_settlement_task().await;
        self.stop_provisional_runtime().await;
        self.cancel_current_session_status_refresh().await;
        self.stop_active_session_account_management_discovery()
            .await;
        self.cancel_secure_backup_inspection().await;
        self.stop_secure_backup_observer().await;
        self.stop_recovery_observer().await;
        self.stop_incoming_verification_observer().await;
        self.stop_session_change_observer().await;
        self.record_lifecycle_probe("shutdown_stop_timeline_actor");
        self.stop_timeline_actor().await;
        self.stop_read_persistence_worker().await;
        self.stop_threads_list_actor().await;
        self.record_lifecycle_probe("shutdown_stop_search_actor");
        self.stop_search_actor().await;
        self.record_lifecycle_probe("shutdown_stop_sync_actor");
        self.stop_sync_actor().await;
        #[cfg(any(test, feature = "test-hooks"))]
        let clear_room_session = !self.residency_preserve_room_session;
        #[cfg(not(any(test, feature = "test-hooks")))]
        let clear_room_session = true;
        let mut teardown_ok = true;
        if clear_room_session {
            self.record_lifecycle_probe("shutdown_clear_room_session");
            teardown_ok = self.clear_room_actor_session().await;
        }
        self.cancel_verification_handles().await;
        self.cancel_identity_reset_handle().await;
        self.invalidate_account_hydration();
        self.abort_avatar_fetch_tasks();
        self.pending_uia_operations.clear();
        self.provisional_persistable = None;
        self.session_promoted = false;
        self.pending_ready_events.clear();
        self.pending_trust_transition = None;
        self.pending_recovery_completion = None;
        teardown_ok
    }

    /// Ordered shutdown of the ThreadsListActor. Dropping the handle cancels
    /// the actor and its SDK subscriptions.
    async fn stop_threads_list_actor(&mut self) {
        if let Some(handle) = self.threads_list_actor.take() {
            let _ = handle.shutdown().await;
        }
    }

    /// Ordered shutdown of the TimelineManagerActor (step 2 of the shutdown
    /// sequence per Async rule 12 — timelines before search/room/sync).
    async fn stop_timeline_actor(&mut self) {
        self.room_actor.clear_timeline_residency();
        #[cfg(any(test, feature = "test-hooks"))]
        if let Some((reached, release)) = self.residency_teardown_gap.take() {
            let _ = reached.send(self.room_actor.timeline_residency_snapshot().is_none());
            let _ = release.await;
        }
        let _ = self.timeline_manager.shutdown().await;
    }

    async fn stop_read_persistence_worker(&mut self) {
        let Some(mut task) = self.read_persistence_task.take() else {
            return;
        };
        if executor::timeout(READ_PERSISTENCE_SHUTDOWN_TIMEOUT, &mut task)
            .await
            .is_err()
        {
            self.read_persistence_session_generation = next_read_persistence_session_generation();
            if let Some(key_id) = self.session_key_id.as_ref() {
                self.store.invalidate_read_state_outbox_saves(
                    key_id,
                    self.read_persistence_session_generation,
                );
            }
            task.abort();
            let _ = task.await;
            record_read_persistence(
                "shutdown",
                "timed_out",
                self.read_persistence_session_generation,
                0,
                0,
                0,
            );
        } else {
            record_read_persistence(
                "shutdown",
                "saved",
                self.read_persistence_session_generation,
                0,
                0,
                0,
            );
        }
    }

    /// Spawn the SyncActor for the just-established store-backed session and
    /// notify the RoomActor so room operations become available.
    /// Also replace the TimelineManagerActor with one that holds the session.
    /// Also spawn the SearchActor (Phase 6).
    pub(super) async fn spawn_sync_actor(&mut self, session: Arc<MatrixClientSession>) {
        trace_restore_simple("spawn_sync_actor", "begin");
        // A trust promotion can race the reducer's StartSync effect and reach
        // this constructor after the normal actor is already owned. Keep the
        // existing owner; replacing its handle would drop the old sender,
        // make that actor stop its SyncService implicitly, and publish a stale
        // stopped status into the still-valid runtime. Session replacement
        // paths retire the old actor before installing the new session.
        if self.sync_actor.is_some() {
            trace_restore_simple("spawn_sync_actor", "already_owned");
            return;
        }
        // The exact session/manager binding is installed immediately before
        // SessionEstablished below. Room operations therefore cannot observe
        // the replacement gap with a mismatched manager.
        // Spawn SearchActor (Phase 6). The session already holds the search
        // index (configured in restore_into_store / the client builder). The
        // search actor gets an mpsc::Sender<SearchIndexMessage> which will be
        // forwarded to the TimelineManagerActor below.
        let search_handle = crate::search::SearchActor::spawn(
            session.clone(),
            self.action_tx.clone(),
            self.event_tx.clone(),
            self.account_work.clone(),
        );
        let search_index_tx = search_handle.index_sender();

        self.search_actor = Some(search_handle);
        // Replay any notification that arrived before the actor was ready so
        // rooms already known to the reducer at session-restore time are not
        // missed by the auto-start logic. Flush is non-blocking; if the search
        // actor is already saturated, the latest payload remains pending for
        // the next AccountActor tick.
        self.flush_pending_crawler_notification();

        // Load the account-scoped encrypted read outbox before constructing a
        // retry-capable manager. Replacement sessions first quiesce the old
        // manager and its serialized saver so late blocking writes cannot race
        // the new account/session generation.
        self.stop_timeline_actor().await;
        if self.read_persistence_task.is_some() {
            self.stop_read_persistence_worker().await;
        }
        self.read_persistence_session_generation = next_read_persistence_session_generation();
        let read_session_generation = self.read_persistence_session_generation;
        let restored_read_state = if let Some(key_id) = self.session_key_id.clone() {
            let store = self.store.clone();
            let load_key_id = key_id.clone();
            match executor::spawn_blocking(move || store.load_read_state_outbox(&load_key_id)).await
            {
                Ok(Ok(snapshot)) => {
                    record_read_persistence(
                        "load",
                        "loaded",
                        read_session_generation,
                        0,
                        snapshot.entry_count(),
                        snapshot.candidate_count(),
                    );
                    snapshot
                }
                Ok(Err(_)) | Err(_) => {
                    record_read_persistence(
                        "load",
                        "failed_closed",
                        read_session_generation,
                        0,
                        0,
                        0,
                    );
                    crate::read_state::ReadPersistenceSnapshot::default()
                }
            }
        } else {
            record_read_persistence("load", "session_missing", read_session_generation, 0, 0, 0);
            crate::read_state::ReadPersistenceSnapshot::default()
        };
        let (read_persistence, read_persistence_rx) = ReadPersistenceIngress::channel();
        if let Some(key_id) = self.session_key_id.clone() {
            self.read_persistence_task = Some(executor::spawn(run_read_persistence_worker(
                self.store.clone(),
                key_id,
                read_session_generation,
                read_persistence_rx,
            )));
        }
        self.timeline_manager = crate::timeline::TimelineManagerHandle::spawn_with_session(
            session.clone(),
            read_session_generation,
            restored_read_state,
            read_persistence,
            self.send_read_receipts,
            self.action_tx.clone(),
            self.event_tx.clone(),
            search_index_tx,
            Some(self.data_dir.clone()),
            self.link_preview_policy.clone(),
            self.account_work.clone(),
            Some(self.navigation_projection.subscribe()),
            Some(self.focused_projection_tx.clone()),
        );
        self.room_actor
            .bind_timeline_residency(session.clone(), self.timeline_manager.residency_handle());
        #[cfg(any(test, feature = "test-hooks"))]
        if let Some((reached, release)) = self.residency_install_gap.take() {
            let _ = reached.send((
                self.room_actor.session_snapshot(),
                self.room_actor
                    .timeline_residency_snapshot()
                    .map(|(session, _)| session),
            ));
            let _ = release.await;
        }
        let _ = self
            .room_actor
            .send(RoomMessage::SessionEstablished {
                session: session.clone(),
            })
            .await;

        let handle = crate::sync::SyncActor::spawn(
            session.clone(),
            self.action_tx.clone(),
            self.event_tx.clone(),
            self.room_actor.sender(),
            self.timeline_manager.sender(),
            self.sync_generation.clone(),
            self.encryption_sync_permit.clone(),
            self.sliding_sync_diagnostics.clone(),
        );
        self.sync_actor = Some(handle);
        trace_restore_simple("spawn_sync_actor", "done");
        self.start_scheduled_send_capability_probe(session);
    }

    /// Ordered shutdown of the SyncActor (step 4 of the shutdown sequence).
    pub(super) async fn stop_sync_actor(&mut self) {
        let Some(handle) = self.sync_actor.take() else {
            return;
        };
        #[cfg(any(test, feature = "test-hooks"))]
        record(DiagnosticEvent::new(
            DiagnosticLevel::Debug,
            "core.account",
            "sync_actor_stop",
        ));
        let _ = handle.shutdown().await;
    }

    /// Ordered shutdown of the RoomActor after the session runtime has stopped.
    /// The acknowledgement is the actor task join, including its observation.
    pub(super) async fn stop_room_actor(&mut self) -> bool {
        self.room_actor.shutdown().await
    }

    pub(super) async fn clear_room_actor_session(&mut self) -> bool {
        // Acknowledged teardown: wait for the RoomActor to cancel and settle
        // any in-flight encryption-debug operation before clearing the
        // session (issue #538). Failures are surfaced AND reported to the
        // caller so account switch/session replacement can abort unless the
        // dangerous operation is confirmed settled.
        let (ack_tx, ack_rx) = oneshot::channel();
        if self
            .room_actor
            .send(RoomMessage::SessionCleared { ack: ack_tx })
            .await
        {
            match ack_rx.await {
                Ok(()) => true,
                Err(_) => {
                    record(DiagnosticEvent::new(
                        DiagnosticLevel::Warn,
                        "core.room_key_debug",
                        "teardown_ack_failed",
                    ));
                    false
                }
            }
        } else {
            record(DiagnosticEvent::new(
                DiagnosticLevel::Warn,
                "core.room_key_debug",
                "teardown_send_failed",
            ));
            false
        }
    }

    pub(super) async fn stop_normal_runtime_children(&mut self) {
        self.set_secure_backup_send_admitted(false);
        self.cancel_secure_backup_inspection().await;
        self.stop_secure_backup_observer().await;
        self.record_lifecycle_probe("stop_recovery_observer");
        self.stop_recovery_observer().await;
        self.record_lifecycle_probe("stop_incoming_verification_observer");
        self.stop_incoming_verification_observer().await;
        self.record_lifecycle_probe("stop_session_change_observer");
        self.stop_session_change_observer().await;
        self.stop_active_session_account_management_discovery()
            .await;
        self.record_lifecycle_probe("stop_timeline_manager");
        self.stop_timeline_actor().await;
        self.stop_read_persistence_worker().await;
        self.timeline_manager = crate::timeline::TimelineManagerHandle::spawn(
            self.action_tx.clone(),
            self.event_tx.clone(),
            Some(self.data_dir.clone()),
            self.account_work.clone(),
            Some(self.navigation_projection.subscribe()),
            Some(self.focused_projection_tx.clone()),
        );
        self.record_lifecycle_probe("stop_threads_manager");
        self.stop_threads_list_actor().await;
        self.record_lifecycle_probe("stop_search_actor");
        self.stop_search_actor().await;
        self.record_lifecycle_probe("stop_sync_actor");
        self.stop_sync_actor().await;
        self.record_lifecycle_probe("clear_room_session");
        self.clear_room_actor_session().await;
        self.record_lifecycle_probe("abort_hydration");
        self.invalidate_account_hydration();
        self.record_lifecycle_probe("abort_attention_media_tasks");
        self.abort_avatar_fetch_tasks();
    }
}

#[cfg(test)]
mod tests;
