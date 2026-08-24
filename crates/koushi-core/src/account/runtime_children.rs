//! `runtime_children` ownership for AccountActor.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_key::SessionKeyId;
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
    pub(super) async fn shutdown_owned_runtime(&mut self) {
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
        self.stop_current_session_runtime().await;
        if let Some(session) = self.session.take() {
            let _ = koushi_sdk::close_session_stores(&session).await;
            drop(session);
            self.record_lifecycle_probe("current_session_released");
        }
        if let Some(pending) = self.pending_session_teardown.take() {
            let _ = koushi_sdk::close_session_stores(&pending.session).await;
            drop(pending.session);
            if let SessionTeardownContinuation::InstallReplacement { session, .. } =
                pending.continuation
            {
                let _ = koushi_sdk::close_session_stores(&session).await;
                drop(session);
            }
            self.record_lifecycle_probe("pending_teardown_sessions_released");
        }
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
        #[cfg(feature = "test-hooks")]
        let clear_room_session = !self.residency_preserve_room_session;
        #[cfg(not(feature = "test-hooks"))]
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
        self.device_session_ordinals.clear();
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
        #[cfg(feature = "test-hooks")]
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
        self.timeline_manager = crate::timeline::TimelineManagerActor::spawn_with_session(
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
        );
        self.room_actor
            .bind_timeline_residency(session.clone(), self.timeline_manager.residency_handle());
        #[cfg(feature = "test-hooks")]
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
            self.room_actor.tx.clone(),
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
        #[cfg(feature = "qa-bin")]
        record(DiagnosticEvent::new(
            DiagnosticLevel::Debug,
            "core.account",
            "sync_actor_stop",
        ));
        let _ = handle.shutdown().await;
    }

    /// Ordered shutdown of the RoomActor after the session runtime has stopped.
    /// The acknowledgement is the actor task join, including its observation.
    pub(super) async fn stop_room_actor(&mut self) {
        self.room_actor.shutdown().await;
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
        self.record_lifecycle_probe("stop_timeline_manager");
        self.stop_timeline_actor().await;
        self.stop_read_persistence_worker().await;
        self.timeline_manager = crate::timeline::TimelineManagerActor::spawn(
            self.action_tx.clone(),
            self.event_tx.clone(),
            Some(self.data_dir.clone()),
            self.account_work.clone(),
            Some(self.navigation_projection.subscribe()),
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
mod tests {
    use std::time::Duration;

    use koushi_key::SessionKeyId;

    use koushi_state::{AppAction, LoginRequest};

    use tokio::sync::mpsc;

    use super::{next_read_persistence_session_generation, run_read_persistence_worker};
    use crate::account::actor::AccountMessage;
    use crate::account::test_support::{
        acknowledge_next_verified_projection, assert_no_logout_finished, configure_verified_trust,
        recv_account_action_with_sliding_sync_effects, recv_probe_with_sliding_sync_effects,
        shutdown_and_ack, spawn_actor_with_dirs, spawn_quarantine_password_server, test_request_id,
    };
    use crate::command::AccountCommand;

    use crate::event::{AccountEvent, CoreEvent};
    use crate::executor;

    use crate::ids::RequestId;

    use crate::store::CredentialStoreBackend;
    use crate::store::StoreActor;

    use crate::timeline::{ReadPersistenceIngress, ReadPersistenceRequest};

    use tempfile::tempdir;

    #[tokio::test]
    async fn shutdown_quiesces_provisional_tasks_and_releases_session_without_logout_terminal() {
        let homeserver = spawn_quarantine_password_server();
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, mut event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
        handle
            .send(AccountMessage::AttachLifecycleProbe { probe_tx })
            .await;
        handle
            .send(AccountMessage::Command(AccountCommand::LoginPassword {
                request_id: test_request_id(),
                request: LoginRequest {
                    homeserver,
                    username: "fixture-user".to_owned(),
                    password: koushi_state::AuthSecret::new("synthetic-password"),
                    device_display_name: None,
                },
                platform: koushi_state::DisplayPlatform::Linux,
            }))
            .await;
        while !matches!(
            recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx)
                .await
                .as_slice(),
            [AppAction::LoginSucceeded { .. }]
        ) {}
        shutdown_and_ack(&handle).await;
        let tokens: Vec<_> = std::iter::from_fn(|| probe_rx.try_recv().ok()).collect();
        assert!(tokens.contains(&"trust_observer_terminated"));
        assert!(tokens.contains(&"provisional_encryption_sync_terminated"));
        assert!(tokens.contains(&"current_session_released"));
        assert_no_logout_finished(&mut action_rx);
        while let Ok(event) = event_rx.try_recv() {
            assert!(!matches!(
                event,
                CoreEvent::Account(AccountEvent::LoggedOut { .. })
            ));
        }
    }

    #[tokio::test]
    async fn shutdown_quiesces_promoted_children_before_releasing_session() {
        let homeserver = spawn_quarantine_password_server();
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, _event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
        handle
            .send(AccountMessage::AttachLifecycleProbe { probe_tx })
            .await;
        configure_verified_trust(&handle).await;
        handle
            .send(AccountMessage::Command(AccountCommand::LoginPassword {
                request_id: test_request_id(),
                request: LoginRequest {
                    homeserver,
                    username: "fixture-user".to_owned(),
                    password: koushi_state::AuthSecret::new("synthetic-password"),
                    device_display_name: None,
                },
                platform: koushi_state::DisplayPlatform::Linux,
            }))
            .await;
        acknowledge_next_verified_projection(&handle, &mut action_rx).await;
        while probe_rx.try_recv().is_ok() {}
        shutdown_and_ack(&handle).await;
        let tokens: Vec<_> = std::iter::from_fn(|| probe_rx.try_recv().ok()).collect();
        assert!(tokens.contains(&"trust_observer_terminated"));
        assert!(tokens.contains(&"shutdown_stop_sync_actor"));
        assert!(tokens.contains(&"shutdown_clear_room_session"));
        assert_eq!(tokens.last(), Some(&"current_session_released"));
    }

    #[tokio::test]
    async fn shutdown_aborts_pending_teardown_retry_and_releases_held_sessions_without_terminal() {
        let first_homeserver = spawn_quarantine_password_server();
        let second_homeserver = spawn_quarantine_password_server();
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, _event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
        handle
            .send(AccountMessage::AttachLifecycleProbe { probe_tx })
            .await;
        let request_id = test_request_id();
        handle
            .send(AccountMessage::Command(AccountCommand::LoginPassword {
                request_id,
                request: LoginRequest {
                    homeserver: first_homeserver,
                    username: "fixture-user".to_owned(),
                    password: koushi_state::AuthSecret::new("synthetic-password"),
                    device_display_name: None,
                },
                platform: koushi_state::DisplayPlatform::Linux,
            }))
            .await;
        while !matches!(
            recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx)
                .await
                .as_slice(),
            [AppAction::LoginSucceeded { .. }]
        ) {}
        handle
            .send(AccountMessage::ConfigureCloseStoreResults {
                results: vec![false; 8],
            })
            .await;
        handle
            .send(AccountMessage::Command(AccountCommand::LoginPassword {
                request_id: RequestId {
                    connection_id: crate::ids::RuntimeConnectionId(4),
                    sequence: 2,
                },
                request: LoginRequest {
                    homeserver: second_homeserver,
                    username: "replacement".to_owned(),
                    password: koushi_state::AuthSecret::new("synthetic-password"),
                    device_display_name: None,
                },
                platform: koushi_state::DisplayPlatform::Linux,
            }))
            .await;
        recv_probe_with_sliding_sync_effects(
            &handle,
            &mut action_rx,
            &mut probe_rx,
            "session_store_close_retrying",
        )
        .await;
        shutdown_and_ack(&handle).await;
        let tokens: Vec<_> = std::iter::from_fn(|| probe_rx.try_recv().ok()).collect();
        assert!(tokens.contains(&"teardown_retry_terminated"));
        assert!(tokens.contains(&"pending_teardown_sessions_released"));
        assert_no_logout_finished(&mut action_rx);
    }

    #[test]
    fn session_established_handoff_to_room_actor_is_reliable() {
        let spawn_body = crate::account::test_source::item_body(
            include_str!("runtime_children.rs"),
            "async fn spawn_sync_actor",
        );
        let session_handoff = spawn_body
            .find(".send(RoomMessage::SessionEstablished")
            .expect("RoomActor session handoff should use reliable send");

        assert!(
            !spawn_body.contains("room_actor.try_send(RoomMessage::SessionEstablished"),
            "SessionEstablished must not be delivered through drop-on-full try_send"
        );
        assert!(
            spawn_body[session_handoff..].contains(".await"),
            "SessionEstablished handoff must await reliable delivery before dependent actors start"
        );
    }

    #[tokio::test]
    async fn read_persistence_worker_saves_latest_snapshot_and_joins_after_channel_close() {
        use crate::read_state::{ReadStateEngine, ReadStateKey, ReadTarget, ReadWaiterId};

        fn snapshot(event_id: &str) -> crate::read_state::ReadPersistenceSnapshot {
            let mut engine = ReadStateEngine::new(1);
            engine.admit(
                1,
                ReadStateKey::PublicUnthreaded {
                    room_id: "!worker-room:example.test".to_owned(),
                },
                ReadTarget::new(event_id.to_owned()),
                ReadWaiterId::new(1),
            );
            engine.persistence_snapshot()
        }

        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let key_id = SessionKeyId {
            homeserver: "https://example.test".to_owned(),
            user_id: "@worker:example.test".to_owned(),
            device_id: "WORKER".to_owned(),
        };
        let store = StoreActor::with_backend(
            CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(
                cred_dir.path(),
            )),
            data_dir.path(),
        );
        store
            .account_store_config(&key_id)
            .expect("seed unlock secret");
        let (ingress, requests) = ReadPersistenceIngress::channel();
        let worker_store = store.clone();
        let worker_key_id = key_id.clone();
        let mut worker = executor::spawn(run_read_persistence_worker(
            worker_store,
            worker_key_id,
            7,
            requests,
        ));
        ingress.publish(ReadPersistenceRequest::new(7, 1, snapshot("$first")));
        let latest = snapshot("$latest");
        ingress.publish(ReadPersistenceRequest::new(7, 2, latest.clone()));
        drop(ingress);

        executor::timeout(Duration::from_secs(1), &mut worker)
            .await
            .expect("closed persistence channel must join within the shutdown bound")
            .expect("persistence worker task");
        assert_eq!(
            store
                .load_read_state_outbox(&key_id)
                .expect("load saved latest snapshot"),
            latest
        );
    }

    #[test]
    fn read_persistence_session_generation_survives_actor_recreation() {
        let first_actor_generation = next_read_persistence_session_generation();
        let recreated_actor_generation = next_read_persistence_session_generation();

        assert!(
            recreated_actor_generation > first_actor_generation,
            "a recreated AccountActor must not reuse a process-local outbox generation"
        );
    }
}
