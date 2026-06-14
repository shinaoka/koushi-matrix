//! AccountActor: handles login, restore, logout, account switch, and owns the
//! SyncActor child.
//!
//! Owns the `MatrixClientSession` handle and the `StoreActor` (which owns
//! the single `CredentialStoreBackend` used for both unlock secrets and
//! session persistence). Internal outcomes are projected via the runtime's
//! action channel (AppAction::LoginSucceeded etc.) so AppState / StateChanged
//! remain reducer-driven. Domain events (AccountEvent::LoggedIn etc.) plus
//! OperationFailed are emitted on the CoreEvent stream.
//!
//! Account store bootstrap invariant (overview.md, Runtime Model): per-account
//! store paths derive from homeserver|user|device, so the device id is unknown
//! until the password exchange completes. First login runs on a storeless
//! client that never syncs or initializes encryption; immediately after login
//! the session is persisted and restored into the per-account encrypted store,
//! and only the store-backed session may start sync or E2EE traffic. The
//! fail-closed local-encryption rule applies to the store creation step: if it
//! fails, the storeless session is NOT kept as a fallback.
//!
//! SwitchAccount (overview.md): ordered shutdown of the current account
//! runtime WITHOUT clearing credentials or stores, followed by a store-backed
//! restore of the target account. Shutdown order: timelines → search → sync
//! (phases 4, 5, 6 add their children; Phase 3 adds sync).
//!
//! Shutdown order (overview.md Async rules 11/12, rule 12 step 4):
//!   stop timeline subscriptions → stop search → stop sync → drop SDK handles.
//! SDK handles dropped inside the Tokio runtime context.

use std::sync::Arc;

use futures_util::StreamExt;
use matrix_desktop_key::{SessionKeyId, StoredMatrixSession};
use matrix_desktop_sdk::{MatrixClientSession, PersistableMatrixSession};
use matrix_desktop_state::{
    AppAction, CrossSigningStatus, E2eeRecoveryState, IdentityResetAuthType, IdentityResetState,
    LoginRequest, RecoveryMethod, RecoveryRequest, SessionInfo, TrustOperationFailureKind,
    VerificationCancelReason, VerificationFlowState, VerificationTarget,
};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::command::{AccountCommand, RoomCommand, SearchCommand, SyncCommand, TimelineCommand};
use crate::event::{AccountEvent, CoreEvent, E2eeTrustEvent};
use crate::failure::{CoreFailure, LoginFailureKind};
use crate::ids::{AccountKey, RequestId, RuntimeConnectionId};
use crate::room::{RoomActorHandle, RoomMessage};
use crate::search::SearchActorHandle;
use crate::store::{StoreActor, account_key_from_info, session_key_id_from_info};
use crate::sync::{SyncActorHandle, SyncMessage};
use crate::timeline::{TimelineManagerHandle, TimelineMessage};

/// "Credential store healthy, but no stored session for that account"
/// during restore/switch (canon: `CoreFailure::SessionNotFound`).
const SESSION_NOT_FOUND_FAILURE: CoreFailure = CoreFailure::SessionNotFound;
const SEARCH_UNAVAILABLE_MESSAGE: &str = "search unavailable";

/// Redacted message used in reducer error projections (never raw SDK text).
const RESTORE_FAILED_MESSAGE: &str = "session restore failed";
const INCOMING_VERIFICATION_FLOW_ID_BASE: u64 = 1 << 63;

/// Messages routed to the AccountActor task.
pub enum AccountMessage {
    Command(AccountCommand),
    SyncCommand(SyncCommand),
    RoomCommand(RoomCommand),
    TimelineCommand(TimelineCommand),
    SearchCommand(SearchCommand),
    VerificationRequestProgress {
        request_id: RequestId,
        target: VerificationTarget,
        state: matrix_desktop_sdk::MatrixVerificationRequestState,
    },
    SasVerificationProgress {
        request_id: RequestId,
        target: VerificationTarget,
        state: matrix_desktop_sdk::MatrixSasState,
    },
    IncomingVerificationRequest {
        target: VerificationTarget,
        handle: matrix_desktop_sdk::MatrixVerificationRequestHandle,
    },
    Shutdown,
}

/// Handle to the AccountActor background task.
pub struct AccountActorHandle {
    tx: mpsc::Sender<AccountMessage>,
}

impl AccountActorHandle {
    pub async fn send(&self, msg: AccountMessage) -> bool {
        self.tx.send(msg).await.is_ok()
    }
}

/// How a successful store-backed restore is reported.
enum RestoreOutcome {
    /// `RestoreSession` command → `AccountEvent::SessionRestored`.
    Restored,
    /// `SwitchAccount` command → `AccountEvent::AccountSwitched`.
    Switched,
}

struct RecoveryStateObservation {
    stop_tx: oneshot::Sender<()>,
    task: crate::executor::JoinHandle<()>,
}

struct VerificationObservation {
    stop_tx: oneshot::Sender<()>,
    task: crate::executor::JoinHandle<()>,
}

struct IncomingVerificationObservation {
    stop_tx: oneshot::Sender<()>,
    task: crate::executor::JoinHandle<()>,
}

struct PendingVerificationRequest {
    request_id: RequestId,
    target: VerificationTarget,
    handle: matrix_desktop_sdk::MatrixVerificationRequestHandle,
}

struct PendingSasVerification {
    request_id: RequestId,
    target: VerificationTarget,
    handle: matrix_desktop_sdk::MatrixSasVerificationHandle,
}

/// The account actor's internal state.
pub struct AccountActor {
    /// Active store-backed session, if any.
    session: Option<Arc<MatrixClientSession>>,
    /// Session key for credential store operations.
    session_key_id: Option<SessionKeyId>,
    /// Store actor — owns the credential store backend and per-account paths.
    store: StoreActor,
    /// App-level action channel to drive the reducer.
    action_tx: mpsc::Sender<Vec<AppAction>>,
    /// Shared event broadcast channel.
    event_tx: broadcast::Sender<CoreEvent>,
    /// Message inbox.
    command_rx: mpsc::Receiver<AccountMessage>,
    /// Sender clone used by SDK observation tasks to report actor-owned
    /// verification progress back into this actor's mailbox.
    self_tx: mpsc::Sender<AccountMessage>,
    /// SyncActor child handle (Phase 3). Present only when a store-backed
    /// session exists. Created on first login/restore; destroyed on logout /
    /// account switch.
    sync_actor: Option<SyncActorHandle>,
    /// RoomActor child handle (Phase 4). Spawned once at actor creation and
    /// kept alive for the lifetime of the AccountActor. Session is provided
    /// via `RoomMessage::SyncStarted` when sync begins.
    room_actor: RoomActorHandle,
    /// TimelineManagerActor handle (Phase 5). Spawned once at actor creation;
    /// session reference is updated when a store-backed session is established.
    timeline_manager: TimelineManagerHandle,
    /// SearchActor handle (Phase 6). Present only when a store-backed session
    /// exists. Created at the same time as SyncActor; stopped in the ordered
    /// shutdown between timelines and sync (canon Async rule 12 step 3).
    search_actor: Option<SearchActorHandle>,
    /// Recovery-state observer task for the active store-backed session.
    recovery_observer: Option<RecoveryStateObservation>,
    /// Pending SDK identity reset continuation, held only inside AccountActor.
    identity_reset_handle: Option<matrix_desktop_sdk::MatrixIdentityResetHandle>,
    /// Pending SDK verification request continuation, held only inside
    /// AccountActor and never projected into AppState.
    verification_request: Option<PendingVerificationRequest>,
    /// Pending SDK SAS continuation, held only inside AccountActor and never
    /// projected into AppState.
    sas_verification: Option<PendingSasVerification>,
    /// SDK verification request observer task for the active flow.
    verification_request_observer: Option<VerificationObservation>,
    /// SDK SAS observer task for the active flow.
    sas_verification_observer: Option<VerificationObservation>,
    /// SDK incoming verification request observer for the active session.
    incoming_verification_observer: Option<IncomingVerificationObservation>,
    /// Synthetic flow id sequence for SDK-originated verification requests.
    next_incoming_verification_sequence: u64,
}

impl AccountActor {
    pub fn spawn(
        store_actor: StoreActor,
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
    ) -> AccountActorHandle {
        let (tx, command_rx) = mpsc::channel(64);
        // Spawn RoomActor once at AccountActor creation. It starts with no
        // session and waits for RoomMessage::SyncStarted.
        let room_actor = crate::room::RoomActor::spawn(action_tx.clone(), event_tx.clone());
        // Spawn TimelineManagerActor. It starts with no session; the session
        // is injected when a store-backed session is established.
        let timeline_manager =
            crate::timeline::TimelineManagerActor::spawn(action_tx.clone(), event_tx.clone());
        let actor = AccountActor {
            session: None,
            session_key_id: None,
            store: store_actor,
            action_tx,
            event_tx,
            command_rx,
            self_tx: tx.clone(),
            sync_actor: None,
            room_actor,
            timeline_manager,
            search_actor: None,
            recovery_observer: None,
            identity_reset_handle: None,
            verification_request: None,
            sas_verification: None,
            verification_request_observer: None,
            sas_verification_observer: None,
            incoming_verification_observer: None,
            next_incoming_verification_sequence: INCOMING_VERIFICATION_FLOW_ID_BASE,
        };
        crate::executor::spawn(actor.run());
        AccountActorHandle { tx }
    }

    async fn run(mut self) {
        while let Some(msg) = self.command_rx.recv().await {
            match msg {
                AccountMessage::Shutdown => break,
                AccountMessage::Command(command) => {
                    self.handle_command(command).await;
                }
                AccountMessage::SyncCommand(sync_command) => {
                    self.route_sync_command(sync_command).await;
                }
                AccountMessage::RoomCommand(room_command) => {
                    self.route_room_command(room_command).await;
                }
                AccountMessage::TimelineCommand(timeline_command) => {
                    self.route_timeline_command(timeline_command).await;
                }
                AccountMessage::SearchCommand(search_command) => {
                    self.route_search_command(search_command).await;
                }
                AccountMessage::VerificationRequestProgress {
                    request_id,
                    target,
                    state,
                } => {
                    self.handle_verification_request_progress(request_id, target, state)
                        .await;
                }
                AccountMessage::SasVerificationProgress {
                    request_id,
                    target,
                    state,
                } => {
                    self.handle_sas_verification_progress(request_id, target, state)
                        .await;
                }
                AccountMessage::IncomingVerificationRequest { target, handle } => {
                    let request_id = self.next_incoming_verification_request_id();
                    self.handle_incoming_verification_request(request_id, target, handle)
                        .await;
                }
            }
        }
        // Ordered shutdown (overview.md Async rule 12):
        // recovery/incoming observers → timelines → search → room → sync → SDK handles.
        self.stop_recovery_observer().await;
        self.stop_incoming_verification_observer().await;
        self.stop_timeline_actor().await;
        self.stop_search_actor().await;
        self.stop_room_actor().await;
        self.stop_sync_actor().await;
        self.cancel_verification_handles().await;
        self.cancel_identity_reset_handle().await;
        // Drop the session handle inside the runtime context
        // (overview.md Async rule 11 — deadpool-runtime panic prevention).
        drop(self.session.take());
    }

    /// Route a RoomCommand to the RoomActor. The RoomActor handles the
    /// SessionRequired check internally (it holds the session ref after
    /// SyncStarted).
    async fn route_room_command(&self, command: RoomCommand) {
        let _ = self.room_actor.send(RoomMessage::Command(command)).await;
    }

    /// Route a TimelineCommand to the TimelineManagerActor.
    /// Session guard is enforced by AppActor before routing; AccountActor
    /// passes through directly to avoid double-gating.
    async fn route_timeline_command(&self, command: TimelineCommand) {
        let _ = self
            .timeline_manager
            .send(TimelineMessage::Command(command))
            .await;
    }

    /// Route a SearchCommand to the SearchActor. Emit SessionRequired if no
    /// search actor is active.
    async fn route_search_command(&self, command: SearchCommand) {
        let request_id = match &command {
            SearchCommand::Query { request_id, .. } => *request_id,
        };
        match &self.search_actor {
            Some(handle) => {
                if !handle.send_command(command).await {
                    self.emit_search_failed(request_id, SEARCH_UNAVAILABLE_MESSAGE)
                        .await;
                    self.emit_failure(request_id, CoreFailure::SessionRequired);
                }
            }
            None => {
                self.emit_search_failed(request_id, SEARCH_UNAVAILABLE_MESSAGE)
                    .await;
                self.emit_failure(request_id, CoreFailure::SessionRequired);
            }
        }
    }

    async fn emit_search_failed(&self, request_id: RequestId, message: &str) {
        let _ = self
            .action_tx
            .send(vec![AppAction::SearchFailed {
                request_id: request_id.sequence,
                message: message.to_owned(),
            }])
            .await;
    }

    /// Ordered shutdown of the SearchActor (step 3 of the shutdown sequence,
    /// after timelines and before sync — canon Async rule 12 step 3).
    async fn stop_search_actor(&mut self) {
        if let Some(handle) = self.search_actor.take() {
            handle.shutdown().await;
        }
    }

    /// Ordered shutdown of the TimelineManagerActor (step 2 of the shutdown
    /// sequence per Async rule 12 — timelines before search/room/sync).
    async fn stop_timeline_actor(&mut self) {
        let _ = self.timeline_manager.send(TimelineMessage::Shutdown).await;
    }

    /// Route a SyncCommand to the SyncActor, or emit SessionRequired if no
    /// store-backed session is active yet.
    async fn route_sync_command(&self, command: SyncCommand) {
        let request_id = match &command {
            SyncCommand::Start { request_id }
            | SyncCommand::Stop { request_id }
            | SyncCommand::Restart { request_id }
            | SyncCommand::SyncOnce { request_id } => *request_id,
        };

        match &self.sync_actor {
            Some(handle) => {
                // The SyncActor notifies the RoomActor itself on start/stop/
                // restart: only it knows the selected backend and owns the
                // live RoomListService (canon, overview.md RoomActor bullet).
                let _ = handle.send(SyncMessage::Command(command)).await;
            }
            None => {
                // Session not yet ready — gate is enforced in AppActor but be
                // defensive here too.
                self.emit_failure(request_id, CoreFailure::SessionRequired);
            }
        }
    }

    /// Spawn the SyncActor for the just-established store-backed session and
    /// notify the RoomActor so room operations become available.
    /// Also replace the TimelineManagerActor with one that holds the session.
    /// Also spawn the SearchActor (Phase 6).
    fn spawn_sync_actor(&mut self, session: Arc<MatrixClientSession>) {
        // Give the RoomActor the session so room ops work even before sync
        // starts. The room-list observation starts later, on the SyncActor's
        // RoomMessage::SyncStarted (which carries the live RoomListService on
        // the SyncService backend). try_send: this is a sync fn; capacity 64
        // is more than enough for this one message.
        self.room_actor.try_send(RoomMessage::SessionEstablished {
            session: session.clone(),
        });

        // Spawn SearchActor (Phase 6). The session already holds the search
        // index (configured in restore_into_store / the client builder). The
        // search actor gets an mpsc::Sender<SearchIndexMessage> which will be
        // forwarded to the TimelineManagerActor below.
        let search_handle = crate::search::SearchActor::spawn(
            session.clone(),
            self.action_tx.clone(),
            self.event_tx.clone(),
        );
        let search_index_tx = search_handle.index_sender();
        self.search_actor = Some(search_handle);

        // Replace the TimelineManagerActor with one holding the current session
        // AND the search index sender. The old manager (with no session) is
        // stopped by dropping its handle. We use try_send to shut down the old.
        self.timeline_manager.try_send(TimelineMessage::Shutdown);
        self.timeline_manager = crate::timeline::TimelineManagerActor::spawn_with_session(
            session.clone(),
            self.action_tx.clone(),
            self.event_tx.clone(),
            search_index_tx,
        );

        let handle = crate::sync::SyncActor::spawn(
            session,
            self.action_tx.clone(),
            self.event_tx.clone(),
            self.room_actor.tx.clone(),
            self.timeline_manager.sender(),
        );
        self.sync_actor = Some(handle);
    }

    fn start_recovery_observer(&mut self, session: Arc<MatrixClientSession>) {
        let (stop_tx, stop_rx) = oneshot::channel();
        let task = crate::executor::spawn(run_recovery_state_observation(
            session.e2ee_recovery_state_stream(),
            account_key_from_info(&session.info),
            self.action_tx.clone(),
            self.event_tx.clone(),
            stop_rx,
        ));
        self.recovery_observer = Some(RecoveryStateObservation { stop_tx, task });
    }

    fn start_incoming_verification_observer(&mut self, session: Arc<MatrixClientSession>) {
        let (stop_tx, mut stop_rx) = oneshot::channel();
        let mut observer = matrix_desktop_sdk::observe_incoming_verification_requests(&session);
        let tx = self.self_tx.clone();
        let task = crate::executor::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    request = observer.recv() => {
                        let Some(request) = request else { break };
                        let (target, handle) = request.into_parts();
                        if tx
                            .send(AccountMessage::IncomingVerificationRequest {
                                target,
                                handle,
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        });
        self.incoming_verification_observer =
            Some(IncomingVerificationObservation { stop_tx, task });
    }

    fn next_incoming_verification_request_id(&mut self) -> RequestId {
        let sequence = self.next_incoming_verification_sequence;
        self.next_incoming_verification_sequence = self
            .next_incoming_verification_sequence
            .checked_add(1)
            .unwrap_or(INCOMING_VERIFICATION_FLOW_ID_BASE);
        incoming_verification_request_id(sequence)
    }

    /// Ordered shutdown of the SyncActor (step 4 of the shutdown sequence).
    async fn stop_sync_actor(&mut self) {
        if let Some(handle) = self.sync_actor.take() {
            let _ = handle.send(SyncMessage::Shutdown).await;
            handle.join().await;
        }
    }

    async fn stop_recovery_observer(&mut self) {
        if let Some(observation) = self.recovery_observer.take() {
            let _ = observation.stop_tx.send(());
            let _ = observation.task.await;
        }
    }

    async fn stop_incoming_verification_observer(&mut self) {
        if let Some(observation) = self.incoming_verification_observer.take() {
            let _ = observation.stop_tx.send(());
            let _ = observation.task.await;
        }
    }

    async fn cancel_identity_reset_handle(&mut self) {
        if let Some(handle) = self.identity_reset_handle.take() {
            handle.cancel().await;
        }
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

    async fn cancel_verification_handles(&mut self) {
        self.stop_verification_request_observer().await;
        self.stop_sas_verification_observer().await;
        if let Some(pending) = self.sas_verification.take() {
            let _ = matrix_desktop_sdk::cancel_sas_verification(&pending.handle).await;
        }
        if let Some(pending) = self.verification_request.take() {
            let _ = matrix_desktop_sdk::cancel_verification_request(&pending.handle).await;
        }
    }

    /// Ordered shutdown of the RoomActor (before sync stop in the shutdown
    /// sequence). The RoomActor is not Option<> since it is always present;
    /// we send Shutdown and the task finishes on its own after processing it.
    async fn stop_room_actor(&mut self) {
        let _ = self.room_actor.send(RoomMessage::Shutdown).await;
    }

    async fn handle_command(&mut self, command: AccountCommand) {
        match command {
            AccountCommand::LoginPassword {
                request_id,
                request,
            } => {
                self.handle_login_password(request_id, request).await;
            }
            AccountCommand::RestoreSession {
                request_id,
                account_key,
            } => {
                self.handle_restore_session(request_id, account_key).await;
            }
            AccountCommand::RestoreLastSession { request_id } => {
                self.handle_restore_last_session(request_id).await;
            }
            AccountCommand::QuerySavedSessions { request_id } => {
                self.handle_query_saved_sessions(request_id);
            }
            AccountCommand::Logout { request_id } => {
                self.handle_logout(request_id).await;
            }
            AccountCommand::SwitchAccount {
                request_id,
                account_key,
            } => {
                self.handle_switch_account(request_id, account_key).await;
            }
            AccountCommand::SubmitRecovery {
                request_id,
                request,
            } => {
                self.handle_submit_recovery(request_id, request).await;
            }
            AccountCommand::BootstrapCrossSigning { request_id, auth } => {
                self.handle_bootstrap_cross_signing(request_id, auth).await;
            }
            AccountCommand::EnableKeyBackup { request_id } => {
                self.handle_enable_key_backup(request_id).await;
            }
            AccountCommand::RestoreKeyBackup {
                request_id,
                version,
                request,
            } => {
                self.handle_restore_key_backup(request_id, version, request)
                    .await;
            }
            AccountCommand::ResetIdentity { request_id } => {
                self.handle_reset_identity(request_id).await;
            }
            AccountCommand::SubmitIdentityResetAuth {
                request_id,
                flow_id,
                request,
            } => {
                self.handle_submit_identity_reset_auth(request_id, flow_id, request)
                    .await;
            }
            AccountCommand::RequestVerification { request_id, target } => {
                self.handle_request_verification(request_id, target).await;
            }
            AccountCommand::AcceptVerification {
                request_id,
                flow_id,
            } => {
                self.handle_accept_verification(request_id, flow_id).await;
            }
            AccountCommand::ConfirmSasVerification {
                request_id,
                flow_id,
            } => {
                self.handle_confirm_sas_verification(request_id, flow_id)
                    .await;
            }
            AccountCommand::CancelVerification {
                request_id,
                flow_id,
                reason,
            } => {
                self.handle_cancel_verification(request_id, flow_id, reason)
                    .await;
            }
        }
    }

    async fn handle_request_verification(
        &mut self,
        request_id: RequestId,
        target: VerificationTarget,
    ) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.reduce(vec![AppAction::VerificationFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }]);
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };

        self.cancel_verification_handles().await;
        match matrix_desktop_sdk::request_device_verification(&session, &target).await {
            Ok(handle) => {
                self.verification_request = Some(PendingVerificationRequest {
                    request_id,
                    target: target.clone(),
                    handle: handle.clone(),
                });
                self.observe_verification_request(request_id, target.clone(), handle.clone());
                self.reduce(vec![AppAction::VerificationRequested {
                    request_id: request_id.sequence,
                    target: target.clone(),
                }]);
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

    async fn handle_incoming_verification_request(
        &mut self,
        request_id: RequestId,
        target: VerificationTarget,
        handle: matrix_desktop_sdk::MatrixVerificationRequestHandle,
    ) {
        if self
            .verification_request
            .as_ref()
            .is_some_and(|pending| pending.handle.flow_id() == handle.flow_id())
        {
            return;
        }

        if self.verification_request.is_some() || self.sas_verification.is_some() {
            let _ = matrix_desktop_sdk::cancel_verification_request(&handle).await;
            return;
        }

        self.verification_request = Some(PendingVerificationRequest {
            request_id,
            target: target.clone(),
            handle: handle.clone(),
        });
        self.observe_verification_request(request_id, target.clone(), handle.clone());
        self.reduce(vec![AppAction::VerificationRequested {
            request_id: request_id.sequence,
            target: target.clone(),
        }]);
        self.emit_verification_progress(VerificationFlowState::Requested {
            request_id: request_id.sequence,
            target,
        });
        self.project_verification_request_state(request_id, handle.state())
            .await;
    }

    async fn handle_accept_verification(&mut self, request_id: RequestId, flow_id: u64) {
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
            matrix_desktop_sdk::MatrixVerificationRequestState::Requested => {
                if let Err(error) = matrix_desktop_sdk::accept_verification_request(&handle).await {
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
            matrix_desktop_sdk::MatrixVerificationRequestState::Ready => {
                match matrix_desktop_sdk::start_sas_verification(&handle).await {
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
            matrix_desktop_sdk::MatrixVerificationRequestState::SasStarted(sas) => {
                self.store_sas_verification(pending_request_id, target, sas)
                    .await;
            }
            matrix_desktop_sdk::MatrixVerificationRequestState::Done => {
                self.project_verification_completed(pending_request_id)
                    .await;
            }
            matrix_desktop_sdk::MatrixVerificationRequestState::Created
            | matrix_desktop_sdk::MatrixVerificationRequestState::Cancelled
            | matrix_desktop_sdk::MatrixVerificationRequestState::UnsupportedMethod => {
                self.project_verification_failure(flow_id, target, TrustOperationFailureKind::Sdk)
                    .await;
            }
        }
    }

    async fn handle_confirm_sas_verification(&mut self, request_id: RequestId, flow_id: u64) {
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

        match matrix_desktop_sdk::confirm_sas_verification(&handle).await {
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

    async fn handle_cancel_verification(
        &mut self,
        request_id: RequestId,
        flow_id: u64,
        reason: VerificationCancelReason,
    ) {
        enum CancelTarget {
            Sas {
                target: VerificationTarget,
                handle: matrix_desktop_sdk::MatrixSasVerificationHandle,
            },
            Request {
                target: VerificationTarget,
                handle: matrix_desktop_sdk::MatrixVerificationRequestHandle,
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
            VerificationCancelReason::User => sas_target.or_else(|| {
                self.verification_request
                    .as_ref()
                    .filter(|pending| pending.request_id.sequence == flow_id)
                    .map(|pending| CancelTarget::Request {
                        target: pending.target.clone(),
                        handle: pending.handle.clone(),
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
        };
        let result = match cancel_target {
            CancelTarget::Sas { handle, .. } => match reason {
                VerificationCancelReason::User => {
                    matrix_desktop_sdk::cancel_sas_verification(&handle).await
                }
                VerificationCancelReason::Mismatch => {
                    matrix_desktop_sdk::mismatch_sas_verification(&handle).await
                }
            },
            CancelTarget::Request { handle, .. } => {
                matrix_desktop_sdk::cancel_verification_request(&handle).await
            }
        };

        self.stop_verification_request_observer().await;
        self.stop_sas_verification_observer().await;
        self.verification_request = None;
        self.sas_verification = None;

        if let Err(error) = result {
            self.project_verification_failure(flow_id, target, classify_e2ee_trust_error(&error))
                .await;
        }
    }

    fn observe_verification_request(
        &mut self,
        request_id: RequestId,
        target: VerificationTarget,
        handle: matrix_desktop_sdk::MatrixVerificationRequestHandle,
    ) {
        let (stop_tx, mut stop_rx) = oneshot::channel();
        let mut states = handle.changes();
        let tx = self.self_tx.clone();
        let task = crate::executor::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    state = states.next() => {
                        let Some(state) = state else { break };
                        let terminal = matches!(
                            state,
                            matrix_desktop_sdk::MatrixVerificationRequestState::Done
                                | matrix_desktop_sdk::MatrixVerificationRequestState::Cancelled
                                | matrix_desktop_sdk::MatrixVerificationRequestState::UnsupportedMethod
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
        handle: matrix_desktop_sdk::MatrixSasVerificationHandle,
    ) {
        let (stop_tx, mut stop_rx) = oneshot::channel();
        let mut states = handle.changes();
        let tx = self.self_tx.clone();
        let task = crate::executor::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    state = states.next() => {
                        let Some(state) = state else { break };
                        let terminal = matches!(
                            state,
                            matrix_desktop_sdk::MatrixSasState::Done
                                | matrix_desktop_sdk::MatrixSasState::Cancelled
                                | matrix_desktop_sdk::MatrixSasState::UnsupportedShortAuth
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

    async fn handle_verification_request_progress(
        &mut self,
        request_id: RequestId,
        _target: VerificationTarget,
        state: matrix_desktop_sdk::MatrixVerificationRequestState,
    ) {
        if !self
            .verification_request
            .as_ref()
            .is_some_and(|pending| pending.request_id.sequence == request_id.sequence)
        {
            return;
        }
        self.project_verification_request_state(request_id, state)
            .await;
    }

    async fn handle_sas_verification_progress(
        &mut self,
        request_id: RequestId,
        target: VerificationTarget,
        state: matrix_desktop_sdk::MatrixSasState,
    ) {
        if !self
            .sas_verification
            .as_ref()
            .is_some_and(|pending| pending.request_id.sequence == request_id.sequence)
        {
            return;
        }
        self.project_sas_state(request_id, target, state).await;
    }

    async fn project_verification_request_state(
        &mut self,
        request_id: RequestId,
        state: matrix_desktop_sdk::MatrixVerificationRequestState,
    ) {
        match state {
            matrix_desktop_sdk::MatrixVerificationRequestState::Created
            | matrix_desktop_sdk::MatrixVerificationRequestState::Requested => {}
            matrix_desktop_sdk::MatrixVerificationRequestState::Ready => {
                self.reduce(vec![AppAction::VerificationAccepted {
                    request_id: request_id.sequence,
                }]);
            }
            matrix_desktop_sdk::MatrixVerificationRequestState::SasStarted(sas) => {
                let Some(target) = self
                    .verification_request
                    .as_ref()
                    .filter(|pending| pending.request_id.sequence == request_id.sequence)
                    .map(|pending| pending.target.clone())
                else {
                    return;
                };
                self.store_sas_verification(request_id, target, sas).await;
            }
            matrix_desktop_sdk::MatrixVerificationRequestState::Done => {
                self.project_verification_completed(request_id).await;
            }
            matrix_desktop_sdk::MatrixVerificationRequestState::Cancelled => {
                self.project_active_or_missing_verification_failure_with_kind(
                    request_id,
                    request_id.sequence,
                    TrustOperationFailureKind::Cancelled,
                )
                .await;
            }
            matrix_desktop_sdk::MatrixVerificationRequestState::UnsupportedMethod => {
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
        handle: matrix_desktop_sdk::MatrixSasVerificationHandle,
    ) {
        self.stop_sas_verification_observer().await;
        self.sas_verification = Some(PendingSasVerification {
            request_id,
            target: target.clone(),
            handle: handle.clone(),
        });
        self.observe_sas_verification(request_id, target.clone(), handle.clone());
        if matches!(handle.state(), matrix_desktop_sdk::MatrixSasState::Started)
            && let Err(error) = matrix_desktop_sdk::accept_sas_verification(&handle).await
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
        state: matrix_desktop_sdk::MatrixSasState,
    ) {
        match state {
            matrix_desktop_sdk::MatrixSasState::Created
            | matrix_desktop_sdk::MatrixSasState::Started
            | matrix_desktop_sdk::MatrixSasState::Accepted => {}
            matrix_desktop_sdk::MatrixSasState::SasPresented { emojis } => {
                self.reduce(vec![AppAction::VerificationSasPresented {
                    request_id: request_id.sequence,
                    emojis: emojis.clone(),
                }]);
                self.emit_verification_progress(VerificationFlowState::SasPresented {
                    request_id: request_id.sequence,
                    target,
                    emojis,
                });
            }
            matrix_desktop_sdk::MatrixSasState::Confirmed => {}
            matrix_desktop_sdk::MatrixSasState::Done => {
                self.project_verification_completed(request_id).await;
            }
            matrix_desktop_sdk::MatrixSasState::Cancelled => {
                self.project_verification_failure(
                    request_id.sequence,
                    target,
                    TrustOperationFailureKind::Cancelled,
                )
                .await;
            }
            matrix_desktop_sdk::MatrixSasState::UnsupportedShortAuth => {
                self.project_verification_failure(
                    request_id.sequence,
                    target,
                    TrustOperationFailureKind::Sdk,
                )
                .await;
            }
        }
    }

    async fn project_verification_completed(&mut self, request_id: RequestId) {
        let target = self
            .sas_verification
            .as_ref()
            .filter(|pending| pending.request_id.sequence == request_id.sequence)
            .map(|pending| pending.target.clone())
            .or_else(|| {
                self.verification_request
                    .as_ref()
                    .filter(|pending| pending.request_id.sequence == request_id.sequence)
                    .map(|pending| pending.target.clone())
            });
        let Some(target) = target else {
            return;
        };

        self.stop_verification_request_observer().await;
        self.stop_sas_verification_observer().await;
        self.verification_request = None;
        self.sas_verification = None;
        self.reduce(vec![AppAction::VerificationCompleted {
            request_id: request_id.sequence,
        }]);
        self.emit_verification_progress(VerificationFlowState::Done {
            request_id: request_id.sequence,
            target,
        });
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
        let target = self
            .sas_verification
            .as_ref()
            .filter(|pending| pending.request_id.sequence == flow_id)
            .map(|pending| pending.target.clone())
            .or_else(|| {
                self.verification_request
                    .as_ref()
                    .filter(|pending| pending.request_id.sequence == flow_id)
                    .map(|pending| pending.target.clone())
            });
        match target {
            Some(target) => {
                self.project_verification_failure(flow_id, target, kind)
                    .await
            }
            None => {
                self.reduce(vec![AppAction::VerificationFailed {
                    request_id: flow_id,
                    kind,
                }]);
                let failure = if self.session.is_some() {
                    CoreFailure::LocalEncryptionUnavailable
                } else {
                    CoreFailure::SessionRequired
                };
                self.emit_failure(request_id, failure);
            }
        }
    }

    async fn project_verification_failure(
        &mut self,
        flow_id: u64,
        target: VerificationTarget,
        kind: TrustOperationFailureKind,
    ) {
        self.stop_verification_request_observer().await;
        self.stop_sas_verification_observer().await;
        self.verification_request = None;
        self.sas_verification = None;
        self.reduce(vec![AppAction::VerificationFailed {
            request_id: flow_id,
            kind,
        }]);
        self.emit_verification_progress(VerificationFlowState::Failed {
            request_id: flow_id,
            target,
            kind,
        });
    }

    fn emit_verification_progress(&self, state: VerificationFlowState) {
        if let Some(account_key) = self.active_account_key() {
            self.emit(CoreEvent::E2eeTrust(E2eeTrustEvent::VerificationProgress {
                account_key,
                state,
            }));
        }
    }

    async fn handle_submit_identity_reset_auth(
        &mut self,
        request_id: RequestId,
        flow_id: u64,
        request: matrix_desktop_state::IdentityResetAuthRequest,
    ) {
        let flow_request_id = RequestId {
            connection_id: request_id.connection_id,
            sequence: flow_id,
        };
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.cancel_identity_reset_handle().await;
                self.reduce(vec![AppAction::ResetIdentityFailed {
                    request_id: flow_id,
                    kind: TrustOperationFailureKind::Sdk,
                }]);
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };
        let account_key = AccountKey(session.info.user_id.clone());
        let result = match self.identity_reset_handle.as_ref() {
            Some(handle) => {
                matrix_desktop_sdk::complete_identity_reset(&session, handle, &request).await
            }
            None => Err(matrix_desktop_sdk::E2eeTrustError::Sdk(
                "identity reset auth continuation missing".to_owned(),
            )),
        };

        drop(request);

        match result {
            Ok(()) => {
                self.identity_reset_handle = None;
                let (actions, events) =
                    project_reset_identity_completed(flow_request_id, account_key);
                self.reduce(actions);
                for event in events {
                    self.emit(event);
                }
            }
            Err(error) => {
                self.cancel_identity_reset_handle().await;
                let (actions, events) =
                    project_reset_identity_error(flow_request_id, account_key, error);
                self.reduce(actions);
                for event in events {
                    self.emit(event);
                }
            }
        }
    }

    async fn handle_bootstrap_cross_signing(
        &self,
        request_id: RequestId,
        auth: Option<matrix_desktop_state::AuthSecret>,
    ) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.reduce(vec![AppAction::BootstrapCrossSigningFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }]);
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };
        let account_key = AccountKey(session.info.user_id.clone());
        let result = matrix_desktop_sdk::bootstrap_cross_signing(&session, auth.as_ref()).await;
        let (actions, events) =
            project_bootstrap_cross_signing_result(request_id, account_key, result);
        self.reduce(actions);
        for event in events {
            self.emit(event);
        }
    }

    async fn handle_enable_key_backup(&self, request_id: RequestId) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.reduce(vec![AppAction::KeyBackupFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }]);
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };
        let account_key = AccountKey(session.info.user_id.clone());
        let result = matrix_desktop_sdk::enable_key_backup(&session).await;
        let (actions, events) = project_enable_key_backup_result(request_id, account_key, result);
        self.reduce(actions);
        for event in events {
            self.emit(event);
        }
    }

    async fn handle_restore_key_backup(
        &self,
        request_id: RequestId,
        version: Option<String>,
        request: RecoveryRequest,
    ) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.reduce(vec![AppAction::KeyBackupFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }]);
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };
        let account_key = AccountKey(session.info.user_id.clone());
        let result =
            matrix_desktop_sdk::restore_key_backup(&session, &request, version.as_deref()).await;
        drop(request);

        let (actions, events) = project_restore_key_backup_result(request_id, account_key, result);
        self.reduce(actions);
        for event in events {
            self.emit(event);
        }
    }

    async fn handle_reset_identity(&mut self, request_id: RequestId) {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                self.cancel_identity_reset_handle().await;
                self.reduce(vec![AppAction::ResetIdentityFailed {
                    request_id: request_id.sequence,
                    kind: TrustOperationFailureKind::Sdk,
                }]);
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };
        let account_key = AccountKey(session.info.user_id.clone());
        match matrix_desktop_sdk::reset_identity(&session).await {
            Ok(matrix_desktop_sdk::IdentityResetOutcome::Completed) => {
                self.cancel_identity_reset_handle().await;
                let (actions, events) = project_reset_identity_completed(request_id, account_key);
                self.reduce(actions);
                for event in events {
                    self.emit(event);
                }
            }
            Ok(matrix_desktop_sdk::IdentityResetOutcome::AuthRequired(handle)) => {
                let auth_type = handle.desktop_auth_type();
                self.cancel_identity_reset_handle().await;
                self.identity_reset_handle = Some(handle);
                let (actions, events) =
                    project_reset_identity_auth_required(request_id, account_key, auth_type);
                self.reduce(actions);
                for event in events {
                    self.emit(event);
                }
            }
            Err(error) => {
                self.cancel_identity_reset_handle().await;
                let (actions, events) =
                    project_reset_identity_error(request_id, account_key, error);
                self.reduce(actions);
                for event in events {
                    self.emit(event);
                }
            }
        }
    }

    async fn handle_login_password(&mut self, request_id: RequestId, request: LoginRequest) {
        // Store bootstrap step 1: the password exchange runs on a storeless
        // client. The device id (and therefore the store path) is unknown
        // before this completes. The storeless client must never sync or
        // initialize encryption.
        let login_result = matrix_desktop_sdk::login_with_password_with_store(&request, None).await;

        let login_session = match login_result {
            Err(error) => {
                let kind = classify_login_error(&error);
                self.emit_failure(request_id, CoreFailure::LoginFailed { kind });
                self.reduce(vec![AppAction::LoginFailed {
                    message: "login failed".to_owned(),
                }]);
                return;
            }
            Ok(session) => session,
        };

        let info = login_session.info.clone();
        let key_id = session_key_id_from_info(&info);
        let account_key = account_key_from_info(&info);

        // Store bootstrap step 2a: persist the session credentials.
        let persistable = match self.persist_session(&login_session, &key_id) {
            Ok(persistable) => persistable,
            Err(failure) => {
                self.abort_login(login_session, &key_id, false).await;
                self.emit_failure(request_id, failure);
                self.reduce(vec![AppAction::LoginFailed {
                    message: "login failed".to_owned(),
                }]);
                return;
            }
        };

        // Store bootstrap step 2b: restore the session into the per-account
        // encrypted store. The store-backed session replaces the login client
        // BEFORE any sync or E2EE traffic. Fail-closed: if store creation or
        // the store-backed restore fails, the storeless session is dropped,
        // never kept as a fallback.
        let store_backed = match self.restore_into_store(&persistable, &key_id).await {
            Ok(session) => session,
            Err(failure) => {
                self.abort_login(login_session, &key_id, true).await;
                self.emit_failure(request_id, failure);
                self.reduce(vec![AppAction::LoginFailed {
                    message: "login failed".to_owned(),
                }]);
                return;
            }
        };

        // The storeless client never synced; drop it inside the runtime
        // context (Async rule 11).
        drop(login_session);

        let session_arc = Arc::new(store_backed);
        self.session = Some(session_arc.clone());
        self.session_key_id = Some(key_id);

        // Spawn the SyncActor now that we have a store-backed session
        // (store bootstrap invariant: sync only on the store-backed session).
        self.spawn_sync_actor(session_arc.clone());

        // Project login success through the reducer (session → Ready).
        self.reduce(vec![AppAction::LoginSucceeded(info)]);

        // Emit domain event carrying the request_id for command correlation.
        self.emit(CoreEvent::Account(AccountEvent::LoggedIn {
            request_id,
            account_key,
        }));

        // Observe the SDK recovery stream asynchronously. New-device recovery
        // can arrive after login completes, once account data syncs in.
        self.start_recovery_observer(session_arc.clone());
        self.start_incoming_verification_observer(session_arc);
    }

    async fn handle_restore_session(&mut self, request_id: RequestId, account_key: AccountKey) {
        let key_id = match self.lookup_session_key_id(&account_key) {
            Ok(Some(key_id)) => key_id,
            Ok(None) => {
                // No stored session for this account: project
                // RestoreSessionNotFound so AppState returns to SignedOut, and
                // keep the redacted failure event for command correlation.
                self.reduce(vec![AppAction::RestoreSessionNotFound]);
                self.emit_failure(request_id, SESSION_NOT_FOUND_FAILURE);
                return;
            }
            Err(()) => {
                // Credential store unreachable.
                self.reduce(vec![AppAction::RestoreSessionFailed {
                    message: RESTORE_FAILED_MESSAGE.to_owned(),
                }]);
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
                return;
            }
        };

        self.restore_account(request_id, key_id, RestoreOutcome::Restored)
            .await;
    }

    /// Resolve the last-session pointer inside the actor and run a
    /// store-backed restore. A missing pointer is a NORMAL outcome
    /// (`CoreFailure::SessionNotFound`): the UI goes to login quietly.
    /// A pointer whose session data is missing follows the same not-found
    /// contract (handled inside `restore_account`).
    async fn handle_restore_last_session(&mut self, request_id: RequestId) {
        let key_id = match self.store.credential_backend().load_last_session() {
            Ok(Some(key_id)) => key_id,
            Ok(None) => {
                self.reduce(vec![AppAction::RestoreSessionNotFound]);
                self.emit_failure(request_id, SESSION_NOT_FOUND_FAILURE);
                return;
            }
            Err(_) => {
                self.reduce(vec![AppAction::RestoreSessionFailed {
                    message: RESTORE_FAILED_MESSAGE.to_owned(),
                }]);
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
                return;
            }
        };

        self.restore_account(request_id, key_id, RestoreOutcome::Restored)
            .await;
    }

    /// List saved sessions from the credential store. Emits
    /// `AccountEvent::SavedSessionsListed` with identity data only
    /// (homeserver / user_id / device_id) — never tokens or secrets.
    /// An empty list is a normal answer, not a failure.
    fn handle_query_saved_sessions(&self, request_id: RequestId) {
        match self.store.credential_backend().load_saved_sessions() {
            Ok(index) => {
                let sessions = index
                    .sessions()
                    .iter()
                    .map(session_info_from_key_id)
                    .collect();
                self.emit(CoreEvent::Account(AccountEvent::SavedSessionsListed {
                    request_id,
                    sessions,
                }));
            }
            Err(_) => {
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
            }
        }
    }

    async fn handle_switch_account(&mut self, request_id: RequestId, account_key: AccountKey) {
        // Ordered shutdown of the current account runtime WITHOUT clearing
        // credentials or stores.
        // Phase 3: stop recovery/incoming observers and sync. Phases 4-6 add
        // their children here.
        self.stop_recovery_observer().await;
        self.stop_incoming_verification_observer().await;
        self.stop_sync_actor().await;
        self.cancel_verification_handles().await;
        self.cancel_identity_reset_handle().await;
        // Drop the SDK handle inside the runtime context (Async rule 11).
        drop(self.session.take());
        self.session_key_id = None;

        let key_id = match self.lookup_session_key_id(&account_key) {
            Ok(Some(key_id)) => key_id,
            Ok(None) => {
                // Same not-found contract as RestoreSession.
                self.reduce(vec![AppAction::RestoreSessionNotFound]);
                self.emit_failure(request_id, SESSION_NOT_FOUND_FAILURE);
                return;
            }
            Err(()) => {
                self.reduce(vec![AppAction::RestoreSessionFailed {
                    message: RESTORE_FAILED_MESSAGE.to_owned(),
                }]);
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
                return;
            }
        };

        // Project the switch intent so the reducer drives state
        // (SwitchingAccount → cleared views), then run the store-backed
        // restore of the target account.
        self.reduce(vec![AppAction::SwitchAccountRequested {
            info: session_info_from_key_id(&key_id),
        }]);

        self.restore_account(request_id, key_id, RestoreOutcome::Switched)
            .await;
    }

    /// Store-backed restore of a known stored account. Shared by
    /// `RestoreSession` and `SwitchAccount`.
    async fn restore_account(
        &mut self,
        request_id: RequestId,
        key_id: SessionKeyId,
        outcome: RestoreOutcome,
    ) {
        let session_json = match self.store.credential_backend().load_matrix_session(&key_id) {
            Ok(stored) => stored,
            Err(err) if matrix_desktop_key::is_missing_credential_error(&err) => {
                self.reduce(vec![AppAction::RestoreSessionNotFound]);
                self.emit_failure(request_id, SESSION_NOT_FOUND_FAILURE);
                return;
            }
            Err(_) => {
                self.reduce(vec![AppAction::RestoreSessionFailed {
                    message: RESTORE_FAILED_MESSAGE.to_owned(),
                }]);
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
                return;
            }
        };

        let persistable = match PersistableMatrixSession::from_json(session_json.as_str()) {
            Ok(s) => s,
            Err(_) => {
                self.reduce(vec![AppAction::RestoreSessionFailed {
                    message: RESTORE_FAILED_MESSAGE.to_owned(),
                }]);
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
                return;
            }
        };

        match self.restore_into_store(&persistable, &key_id).await {
            Err(failure) => {
                self.reduce(vec![AppAction::RestoreSessionFailed {
                    message: RESTORE_FAILED_MESSAGE.to_owned(),
                }]);
                self.emit_failure(request_id, failure);
            }
            Ok(session) => {
                let info = session.info.clone();
                let account_key = account_key_from_info(&info);

                let session_arc = Arc::new(session);
                self.session = Some(session_arc.clone());
                self.session_key_id = Some(key_id);

                // Spawn the SyncActor for the newly restored store-backed session.
                self.spawn_sync_actor(session_arc.clone());
                self.reduce(vec![AppAction::RestoreSessionSucceeded(info)]);

                self.emit(CoreEvent::Account(match outcome {
                    RestoreOutcome::Restored => AccountEvent::SessionRestored {
                        request_id,
                        account_key,
                    },
                    RestoreOutcome::Switched => AccountEvent::AccountSwitched {
                        request_id,
                        account_key,
                    },
                }));

                self.start_recovery_observer(session_arc.clone());
                self.start_incoming_verification_observer(session_arc);
            }
        }
    }

    /// Submit a recovery secret. Calls the auth crate's `recover_e2ee`
    /// primitive. On success: project E2eeRecoverySucceeded (→ Ready) and emit
    /// RecoveryCompleted. On failure: classify conservatively to
    /// InvalidRecoveryKey/Network/Server (never raw error text) and emit
    /// OperationFailed with RecoveryFailed.
    ///
    /// The recovery secret is NEVER logged, included in error messages, or
    /// stored in any event/snapshot.
    async fn handle_submit_recovery(&mut self, request_id: RequestId, request: RecoveryRequest) {
        let session = match &self.session {
            Some(s) => s.clone(),
            None => {
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };

        let account_key = AccountKey(session.info.user_id.clone());

        // Project E2eeRecoverySubmitted so the reducer transitions
        // NeedsRecovery → Recovering while the async call runs.
        self.reduce(vec![AppAction::E2eeRecoverySubmitted(request.clone())]);

        let result = matrix_desktop_sdk::recover_e2ee(&session, &request).await;

        // Zero the request secret now — it has been consumed.
        drop(request);

        match result {
            Ok(()) => {
                // Project success: Recovering → Ready.
                self.reduce(vec![AppAction::E2eeRecoverySucceeded]);
                self.emit(CoreEvent::Account(AccountEvent::RecoveryCompleted {
                    request_id,
                    account_key,
                }));
            }
            Err(error) => {
                let kind = classify_recovery_error(&error);
                // Project failure: Recovering → NeedsRecovery.
                self.reduce(vec![AppAction::E2eeRecoveryFailed {
                    message: "recovery failed".to_owned(),
                }]);
                self.emit_failure(request_id, CoreFailure::RecoveryFailed { kind });
            }
        }
    }

    async fn handle_logout(&mut self, request_id: RequestId) {
        let session = match self.session.take() {
            Some(s) => s,
            None => {
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };
        let key_id = self.session_key_id.take();

        self.stop_recovery_observer().await;
        self.stop_incoming_verification_observer().await;
        // Ordered shutdown step 4: stop sync before dropping the session.
        self.stop_sync_actor().await;
        self.cancel_verification_handles().await;
        self.cancel_identity_reset_handle().await;

        // Attempt server-side logout (best-effort; local cleanup always happens).
        let _ = matrix_desktop_sdk::logout(&session).await;

        // Drop the SDK handle inside the Tokio runtime context (Async rule 11).
        drop(session);

        // Clean up credentials and stores for this account only.
        let account_key = if let Some(key_id) = &key_id {
            self.clear_account_persistence(key_id);
            AccountKey(key_id.user_id.clone())
        } else {
            AccountKey(String::new())
        };

        self.reduce(vec![AppAction::LogoutFinished]);
        self.emit(CoreEvent::Account(AccountEvent::LoggedOut {
            request_id,
            account_key,
        }));
    }

    // --- helpers ---

    /// Persist session credentials, mirroring the src-tauri flow: session
    /// JSON, saved-session index entry, last-session pointer — with rollback
    /// on partial failure.
    fn persist_session(
        &self,
        session: &MatrixClientSession,
        key_id: &SessionKeyId,
    ) -> Result<PersistableMatrixSession, CoreFailure> {
        let backend = self.store.credential_backend();
        let persistable = session
            .persistable_session()
            .map_err(|_| CoreFailure::StoreUnavailable)?;
        let json = persistable
            .to_json()
            .map_err(|_| CoreFailure::StoreUnavailable)?;
        let stored = StoredMatrixSession::new(json);
        backend
            .save_matrix_session(key_id, &stored)
            .map_err(|_| CoreFailure::StoreUnavailable)?;
        if backend.remember_saved_session(key_id).is_err() {
            let _ = backend.delete_matrix_session(key_id);
            return Err(CoreFailure::StoreUnavailable);
        }
        if backend.save_last_session(key_id).is_err() {
            let _ = backend.delete_matrix_session(key_id);
            let _ = backend.forget_saved_session(key_id);
            return Err(CoreFailure::StoreUnavailable);
        }
        Ok(persistable)
    }

    /// Restore a persisted session into the per-account encrypted store
    /// (fail-closed: any store init failure is `LocalEncryptionUnavailable`).
    /// The store config includes the search index so the SDK initializes it
    /// alongside the SQLite store.
    async fn restore_into_store(
        &self,
        persistable: &PersistableMatrixSession,
        key_id: &SessionKeyId,
    ) -> Result<MatrixClientSession, CoreFailure> {
        let store_config = self.store.account_store_config(key_id)?;
        // Derive the search index configuration. Fail-closed: if the
        // credential store is unreachable, deny the restore (LocalEncryptionUnavailable).
        let search_config = self.store.account_search_index_config(key_id)?;
        let store_config_with_search = store_config
            .store_config
            .with_search_index_store(search_config.search_index_config);
        matrix_desktop_sdk::restore_session_with_store(persistable, Some(&store_config_with_search))
            .await
            .map_err(|_| CoreFailure::LocalEncryptionUnavailable)
    }

    /// Roll back a failed login bootstrap: best-effort server logout of the
    /// storeless client (so no orphan device stays registered), drop it inside
    /// the runtime context, and — if credentials were already persisted —
    /// remove them again so a later restore does not pick up a session whose
    /// token was just invalidated.
    async fn abort_login(
        &self,
        login_session: MatrixClientSession,
        key_id: &SessionKeyId,
        credentials_persisted: bool,
    ) {
        let _ = matrix_desktop_sdk::logout(&login_session).await;
        drop(login_session);
        if credentials_persisted {
            self.clear_account_persistence(key_id);
        }
    }

    /// Remove all persisted material for one account: session JSON, saved
    /// session index entry, last-session pointer (only if it points at this
    /// account), unlock secret, and store/cache directories.
    fn clear_account_persistence(&self, key_id: &SessionKeyId) {
        let backend = self.store.credential_backend();
        let _ = backend.delete_matrix_session(key_id);
        let _ = backend.forget_saved_session(key_id);
        match backend.load_last_session() {
            Ok(Some(last)) if last == *key_id => {
                let _ = backend.delete_last_session();
            }
            Ok(_) => {}
            Err(_) => {
                let _ = backend.delete_last_session();
            }
        }
        self.store.delete_account_credentials(key_id);
    }

    /// Find the stored `SessionKeyId` for an account key (the user's Matrix
    /// ID). Checks the last-session pointer first, then the saved-session
    /// index. `Ok(None)` = no stored session; `Err(())` = store unreachable.
    fn lookup_session_key_id(&self, account_key: &AccountKey) -> Result<Option<SessionKeyId>, ()> {
        let backend = self.store.credential_backend();
        match backend.load_last_session() {
            Ok(Some(key_id)) if key_id.user_id == account_key.0 => {
                return Ok(Some(key_id));
            }
            Ok(_) => {}
            Err(_) => return Err(()),
        }
        let index = backend.load_saved_sessions().map_err(|_| ())?;
        Ok(index
            .sessions()
            .iter()
            .find(|session| session.user_id == account_key.0)
            .cloned())
    }

    fn emit(&self, event: CoreEvent) {
        let _ = self.event_tx.send(event);
    }

    fn emit_failure(&self, request_id: RequestId, failure: CoreFailure) {
        self.emit(CoreEvent::OperationFailed {
            request_id,
            failure,
        });
    }

    fn active_account_key(&self) -> Option<AccountKey> {
        self.session
            .as_ref()
            .map(|session| AccountKey(session.info.user_id.clone()))
    }

    fn reduce(&self, actions: Vec<AppAction>) {
        let _ = self.action_tx.try_send(actions);
    }
}

fn session_info_from_key_id(key_id: &SessionKeyId) -> SessionInfo {
    SessionInfo {
        homeserver: key_id.homeserver.clone(),
        user_id: key_id.user_id.clone(),
        device_id: key_id.device_id.clone(),
    }
}

async fn run_recovery_state_observation<S>(
    state_stream: S,
    account_key: AccountKey,
    action_tx: mpsc::Sender<Vec<AppAction>>,
    event_tx: broadcast::Sender<CoreEvent>,
    mut stop_rx: oneshot::Receiver<()>,
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
                        let _ = action_tx
                            .send(vec![AppAction::E2eeRecoveryStateChanged {
                                state: E2eeRecoveryState::Incomplete,
                                methods: recovery_methods.clone(),
                            }])
                            .await;
                        let _ = event_tx.send(CoreEvent::Account(AccountEvent::RecoveryRequired {
                            account_key: account_key.clone(),
                        }));
                    }
                    E2eeRecoveryState::Enabled | E2eeRecoveryState::Disabled => {
                        let _ = action_tx
                            .send(vec![AppAction::E2eeRecoveryStateChanged {
                                state,
                                methods: recovery_methods.clone(),
                            }])
                            .await;
                    }
                }
            }
        }
    }
}

/// Map an `E2eeRecoveryError` to a coarse `RecoveryFailureKind` without
/// exposing raw SDK error text in public events or error messages.
/// Conservative classification: prefer InvalidRecoveryKey for auth-type SDK
/// errors, Network for network errors, Server for anything else.
fn classify_recovery_error(
    error: &matrix_desktop_sdk::E2eeRecoveryError,
) -> crate::failure::RecoveryFailureKind {
    use crate::failure::RecoveryFailureKind;
    use matrix_desktop_sdk::E2eeRecoveryError;
    match error {
        E2eeRecoveryError::Runtime(_) => RecoveryFailureKind::Network,
        E2eeRecoveryError::Sdk(message) => {
            // Classify by error text fragments — these fragments come from the
            // SDK/server and are used only for kind selection, never emitted.
            if message.contains("invalid")
                || message.contains("Invalid")
                || message.contains("M_FORBIDDEN")
                || message.contains("401")
                || message.contains("403")
            {
                RecoveryFailureKind::InvalidRecoveryKey
            } else if message.contains("network")
                || message.contains("timeout")
                || message.contains("connection")
                || message.contains("connect")
            {
                RecoveryFailureKind::Network
            } else {
                RecoveryFailureKind::Server
            }
        }
    }
}

fn classify_e2ee_trust_error(
    error: &matrix_desktop_sdk::E2eeTrustError,
) -> TrustOperationFailureKind {
    match error {
        matrix_desktop_sdk::E2eeTrustError::NoOlmMachine => TrustOperationFailureKind::Sdk,
        matrix_desktop_sdk::E2eeTrustError::Sdk(message) => {
            let lower = message.to_ascii_lowercase();
            if lower.contains("timeout") {
                TrustOperationFailureKind::Timeout
            } else if lower.contains("forbidden")
                || lower.contains("m_forbidden")
                || lower.contains("401")
                || lower.contains("403")
            {
                TrustOperationFailureKind::Forbidden
            } else if lower.contains("network")
                || lower.contains("connection")
                || lower.contains("connect")
            {
                TrustOperationFailureKind::Network
            } else {
                TrustOperationFailureKind::Sdk
            }
        }
    }
}

fn project_bootstrap_cross_signing_result(
    request_id: RequestId,
    account_key: AccountKey,
    result: Result<matrix_desktop_state::CrossSigningStatus, matrix_desktop_sdk::E2eeTrustError>,
) -> (Vec<AppAction>, Vec<CoreEvent>) {
    match result {
        Ok(status) => (
            vec![AppAction::CrossSigningStatusChanged {
                status: status.clone(),
            }],
            vec![CoreEvent::E2eeTrust(E2eeTrustEvent::CrossSigningChanged {
                account_key,
                status,
            })],
        ),
        Err(error) => {
            let kind = classify_e2ee_trust_error(&error);
            let status = matrix_desktop_state::CrossSigningStatus::Failed {
                request_id: request_id.sequence,
                kind,
            };
            (
                vec![AppAction::BootstrapCrossSigningFailed {
                    request_id: request_id.sequence,
                    kind,
                }],
                vec![CoreEvent::E2eeTrust(E2eeTrustEvent::CrossSigningChanged {
                    account_key,
                    status,
                })],
            )
        }
    }
}

fn project_enable_key_backup_result(
    request_id: RequestId,
    account_key: AccountKey,
    result: Result<matrix_desktop_state::KeyBackupStatus, matrix_desktop_sdk::E2eeTrustError>,
) -> (Vec<AppAction>, Vec<CoreEvent>) {
    match result {
        Ok(matrix_desktop_state::KeyBackupStatus::Enabled { version }) => {
            let status = matrix_desktop_state::KeyBackupStatus::Enabled {
                version: version.clone(),
            };
            (
                vec![AppAction::KeyBackupEnabled {
                    request_id: request_id.sequence,
                    version,
                }],
                vec![CoreEvent::E2eeTrust(E2eeTrustEvent::KeyBackupChanged {
                    account_key,
                    status,
                })],
            )
        }
        Ok(status) => (
            vec![AppAction::KeyBackupFailed {
                request_id: request_id.sequence,
                kind: TrustOperationFailureKind::Sdk,
            }],
            vec![CoreEvent::E2eeTrust(E2eeTrustEvent::KeyBackupChanged {
                account_key,
                status,
            })],
        ),
        Err(error) => {
            let kind = classify_e2ee_trust_error(&error);
            let status = matrix_desktop_state::KeyBackupStatus::Failed {
                request_id: request_id.sequence,
                kind,
            };
            (
                vec![AppAction::KeyBackupFailed {
                    request_id: request_id.sequence,
                    kind,
                }],
                vec![CoreEvent::E2eeTrust(E2eeTrustEvent::KeyBackupChanged {
                    account_key,
                    status,
                })],
            )
        }
    }
}

fn project_restore_key_backup_result(
    request_id: RequestId,
    account_key: AccountKey,
    result: Result<matrix_desktop_sdk::KeyBackupRestoreSummary, matrix_desktop_sdk::E2eeTrustError>,
) -> (Vec<AppAction>, Vec<CoreEvent>) {
    match result {
        Ok(summary) => {
            let progress_status = matrix_desktop_state::KeyBackupStatus::Restoring {
                request_id: request_id.sequence,
                version: summary.version.clone(),
                restored_rooms: summary.restored_rooms,
                total_rooms: summary.total_rooms,
            };
            let restored_status = match summary.version.clone() {
                Some(version) => matrix_desktop_state::KeyBackupStatus::Enabled { version },
                None => matrix_desktop_state::KeyBackupStatus::Unknown,
            };
            (
                vec![
                    AppAction::KeyBackupRestoreProgress {
                        request_id: request_id.sequence,
                        restored_rooms: summary.restored_rooms,
                        total_rooms: summary.total_rooms,
                    },
                    AppAction::KeyBackupRestored {
                        request_id: request_id.sequence,
                        version: summary.version,
                    },
                ],
                vec![
                    CoreEvent::E2eeTrust(E2eeTrustEvent::KeyBackupChanged {
                        account_key: account_key.clone(),
                        status: progress_status,
                    }),
                    CoreEvent::E2eeTrust(E2eeTrustEvent::KeyBackupChanged {
                        account_key,
                        status: restored_status,
                    }),
                ],
            )
        }
        Err(error) => {
            let kind = classify_e2ee_trust_error(&error);
            let status = matrix_desktop_state::KeyBackupStatus::Failed {
                request_id: request_id.sequence,
                kind,
            };
            (
                vec![AppAction::KeyBackupFailed {
                    request_id: request_id.sequence,
                    kind,
                }],
                vec![CoreEvent::E2eeTrust(E2eeTrustEvent::KeyBackupChanged {
                    account_key,
                    status,
                })],
            )
        }
    }
}

fn project_reset_identity_completed(
    request_id: RequestId,
    account_key: AccountKey,
) -> (Vec<AppAction>, Vec<CoreEvent>) {
    (
        vec![AppAction::ResetIdentityCompleted {
            request_id: request_id.sequence,
        }],
        vec![CoreEvent::E2eeTrust(E2eeTrustEvent::IdentityResetChanged {
            account_key,
            state: IdentityResetState::Idle,
        })],
    )
}

fn project_reset_identity_auth_required(
    request_id: RequestId,
    account_key: AccountKey,
    auth_type: IdentityResetAuthType,
) -> (Vec<AppAction>, Vec<CoreEvent>) {
    let state = IdentityResetState::AwaitingAuth {
        request_id: request_id.sequence,
        auth_type,
    };
    (
        vec![AppAction::ResetIdentityAuthRequired {
            request_id: request_id.sequence,
            auth_type,
        }],
        vec![CoreEvent::E2eeTrust(E2eeTrustEvent::IdentityResetChanged {
            account_key,
            state,
        })],
    )
}

fn project_reset_identity_error(
    request_id: RequestId,
    account_key: AccountKey,
    error: matrix_desktop_sdk::E2eeTrustError,
) -> (Vec<AppAction>, Vec<CoreEvent>) {
    let kind = classify_e2ee_trust_error(&error);
    let state = IdentityResetState::Failed {
        request_id: request_id.sequence,
        kind,
    };
    (
        vec![AppAction::ResetIdentityFailed {
            request_id: request_id.sequence,
            kind,
        }],
        vec![
            CoreEvent::E2eeTrust(E2eeTrustEvent::CrossSigningChanged {
                account_key: account_key.clone(),
                status: CrossSigningStatus::Failed {
                    request_id: request_id.sequence,
                    kind,
                },
            }),
            CoreEvent::E2eeTrust(E2eeTrustEvent::IdentityResetChanged { account_key, state }),
        ],
    )
}

fn incoming_verification_request_id(sequence: u64) -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(0),
        sequence,
    }
}

/// Map a `PasswordLoginError` to a coarse `LoginFailureKind` without exposing
/// raw SDK error text in public events.
fn classify_login_error(error: &matrix_desktop_sdk::PasswordLoginError) -> LoginFailureKind {
    use matrix_desktop_sdk::{LoginDiscoveryError, PasswordLoginError};
    match error {
        PasswordLoginError::InvalidHomeserver(discovery_err) => match discovery_err {
            LoginDiscoveryError::RequestFailed(_) | LoginDiscoveryError::HttpStatus { .. } => {
                LoginFailureKind::Network
            }
            _ => LoginFailureKind::Server,
        },
        PasswordLoginError::Sdk(message) => {
            if message.contains("401")
                || message.contains("403")
                || message.contains("M_FORBIDDEN")
                || message.contains("M_UNAUTHORIZED")
            {
                LoginFailureKind::InvalidCredentials
            } else if message.contains("429") || message.contains("M_LIMIT_EXCEEDED") {
                LoginFailureKind::RateLimited
            } else {
                LoginFailureKind::Server
            }
        }
        PasswordLoginError::Runtime(_) => LoginFailureKind::Server,
        PasswordLoginError::MissingSession => LoginFailureKind::Server,
        PasswordLoginError::Serialization(_) => LoginFailureKind::Store,
    }
}

#[cfg(test)]
mod tests {
    use futures_util::stream;
    use tempfile::tempdir;
    use tokio::sync::{broadcast, mpsc};

    use super::*;
    use crate::store::CredentialStoreBackend;

    #[test]
    fn incoming_verification_flow_ids_use_reserved_internal_namespace() {
        let request_id = incoming_verification_request_id(INCOMING_VERIFICATION_FLOW_ID_BASE);

        assert_eq!(request_id.connection_id, RuntimeConnectionId(0));
        assert_eq!(request_id.sequence, INCOMING_VERIFICATION_FLOW_ID_BASE);
    }

    /// Network-free: restoring an account with no stored session must emit the
    /// redacted not-found failure AND project `RestoreSessionNotFound` so the
    /// reducer returns AppState to SignedOut. Same contract for SwitchAccount.
    #[tokio::test]
    async fn restore_and_switch_of_unknown_account_emit_not_found() {
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let store = StoreActor::with_backend(
            CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(
                cred_dir.path(),
            )),
            data_dir.path(),
        );

        let (action_tx, mut action_rx) = mpsc::channel(16);
        let (event_tx, mut event_rx) = broadcast::channel(16);
        let handle = AccountActor::spawn(store, action_tx, event_tx);

        let request_id = RequestId {
            connection_id: crate::ids::RuntimeConnectionId(1),
            sequence: 1,
        };
        let account_key = AccountKey("@nobody:example.test".to_owned());

        for command in [
            AccountCommand::RestoreSession {
                request_id,
                account_key: account_key.clone(),
            },
            AccountCommand::SwitchAccount {
                request_id,
                account_key: account_key.clone(),
            },
        ] {
            assert!(handle.send(AccountMessage::Command(command)).await);

            let actions = action_rx.recv().await.expect("reducer actions");
            assert!(
                matches!(actions.as_slice(), [AppAction::RestoreSessionNotFound]),
                "not-found must project RestoreSessionNotFound, got {actions:?}"
            );

            match event_rx.recv().await.expect("event") {
                CoreEvent::OperationFailed {
                    request_id: ev_id,
                    failure,
                } => {
                    assert_eq!(ev_id, request_id);
                    assert_eq!(failure, SESSION_NOT_FOUND_FAILURE);
                }
                other => panic!("expected OperationFailed, got {other:?}"),
            }
        }
    }

    fn test_request_id() -> RequestId {
        RequestId {
            connection_id: crate::ids::RuntimeConnectionId(1),
            sequence: 1,
        }
    }

    fn spawn_actor_with_dirs(
        cred_dir: &std::path::Path,
        data_dir: &std::path::Path,
    ) -> (
        AccountActorHandle,
        mpsc::Receiver<Vec<AppAction>>,
        broadcast::Receiver<CoreEvent>,
    ) {
        let store = StoreActor::with_backend(
            CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(cred_dir)),
            data_dir,
        );
        let (action_tx, action_rx) = mpsc::channel(16);
        let (event_tx, event_rx) = broadcast::channel(16);
        let handle = AccountActor::spawn(store, action_tx, event_tx);
        (handle, action_rx, event_rx)
    }

    /// Network-free: `RestoreLastSession` with no last-session pointer is the
    /// NORMAL first-launch outcome — `SessionNotFound` failure event plus the
    /// `RestoreSessionNotFound` projection so AppState shows SignedOut/login.
    #[tokio::test]
    async fn restore_last_session_without_pointer_emits_not_found() {
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, mut event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());

        let request_id = test_request_id();
        assert!(
            handle
                .send(AccountMessage::Command(
                    AccountCommand::RestoreLastSession { request_id }
                ))
                .await
        );

        let actions = action_rx.recv().await.expect("reducer actions");
        assert!(
            matches!(actions.as_slice(), [AppAction::RestoreSessionNotFound]),
            "not-found must project RestoreSessionNotFound, got {actions:?}"
        );

        match event_rx.recv().await.expect("event") {
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } => {
                assert_eq!(ev_id, request_id);
                assert_eq!(failure, SESSION_NOT_FOUND_FAILURE);
            }
            other => panic!("expected OperationFailed, got {other:?}"),
        }
    }

    /// Network-free: a last-session pointer whose session data is gone (e.g.
    /// cleared by logout) must follow the same not-found contract.
    #[tokio::test]
    async fn restore_last_session_with_dangling_pointer_emits_not_found() {
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");

        // Seed only the pointer — no session JSON behind it.
        let seeding_backend = CredentialStoreBackend::FileDir(
            crate::store::FileCredentialStore::new(cred_dir.path()),
        );
        let key_id = SessionKeyId {
            homeserver: "https://example.test".to_owned(),
            user_id: "@dangling:example.test".to_owned(),
            device_id: "DEVICE1".to_owned(),
        };
        seeding_backend
            .save_last_session(&key_id)
            .expect("seed last-session pointer");

        let (handle, mut action_rx, mut event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());

        let request_id = test_request_id();
        assert!(
            handle
                .send(AccountMessage::Command(
                    AccountCommand::RestoreLastSession { request_id }
                ))
                .await
        );

        let actions = action_rx.recv().await.expect("reducer actions");
        assert!(
            matches!(actions.as_slice(), [AppAction::RestoreSessionNotFound]),
            "dangling pointer must project RestoreSessionNotFound, got {actions:?}"
        );

        match event_rx.recv().await.expect("event") {
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } => {
                assert_eq!(ev_id, request_id);
                assert_eq!(failure, SESSION_NOT_FOUND_FAILURE);
            }
            other => panic!("expected OperationFailed, got {other:?}"),
        }
    }

    /// Network-free: `QuerySavedSessions` on an empty store answers with an
    /// empty list — a normal outcome, not a failure.
    #[tokio::test]
    async fn query_saved_sessions_empty_store_lists_nothing() {
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, _action_rx, mut event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());

        let request_id = test_request_id();
        assert!(
            handle
                .send(AccountMessage::Command(
                    AccountCommand::QuerySavedSessions { request_id }
                ))
                .await
        );

        match event_rx.recv().await.expect("event") {
            CoreEvent::Account(AccountEvent::SavedSessionsListed {
                request_id: ev_id,
                sessions,
            }) => {
                assert_eq!(ev_id, request_id);
                assert!(sessions.is_empty(), "expected empty list, got {sessions:?}");
            }
            other => panic!("expected SavedSessionsListed, got {other:?}"),
        }
    }

    /// Network-free: `QuerySavedSessions` lists seeded sessions with identity
    /// data only (homeserver / user_id / device_id).
    #[tokio::test]
    async fn query_saved_sessions_lists_seeded_identities() {
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");

        let seeding_backend = CredentialStoreBackend::FileDir(
            crate::store::FileCredentialStore::new(cred_dir.path()),
        );
        let alpha = SessionKeyId {
            homeserver: "https://example.test".to_owned(),
            user_id: "@alpha:example.test".to_owned(),
            device_id: "DEVICE-A".to_owned(),
        };
        let beta = SessionKeyId {
            homeserver: "https://example.test".to_owned(),
            user_id: "@beta:example.test".to_owned(),
            device_id: "DEVICE-B".to_owned(),
        };
        seeding_backend
            .remember_saved_session(&alpha)
            .expect("seed alpha");
        seeding_backend
            .remember_saved_session(&beta)
            .expect("seed beta");

        let (handle, _action_rx, mut event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());

        let request_id = test_request_id();
        assert!(
            handle
                .send(AccountMessage::Command(
                    AccountCommand::QuerySavedSessions { request_id }
                ))
                .await
        );

        match event_rx.recv().await.expect("event") {
            CoreEvent::Account(AccountEvent::SavedSessionsListed {
                request_id: ev_id,
                sessions,
            }) => {
                assert_eq!(ev_id, request_id);
                assert_eq!(sessions.len(), 2);
                assert!(
                    sessions.iter().any(|s| {
                        s.user_id == "@alpha:example.test" && s.device_id == "DEVICE-A"
                    })
                );
                assert!(
                    sessions.iter().any(|s| {
                        s.user_id == "@beta:example.test" && s.device_id == "DEVICE-B"
                    })
                );
                // Identity data only: SessionInfo has exactly homeserver /
                // user_id / device_id (enforced by type); the Debug output of
                // the event must not contain anything token-shaped.
                let debug = format!("{sessions:?}");
                assert!(!debug.contains("access_token"));
                assert!(!debug.contains("secret"));
            }
            other => panic!("expected SavedSessionsListed, got {other:?}"),
        }
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
        };
        let account_key = AccountKey(info.user_id.clone());
        let states = stream::iter([
            matrix_desktop_state::E2eeRecoveryState::Unknown,
            matrix_desktop_state::E2eeRecoveryState::Incomplete,
            matrix_desktop_state::E2eeRecoveryState::Incomplete,
            matrix_desktop_state::E2eeRecoveryState::Enabled,
            matrix_desktop_state::E2eeRecoveryState::Enabled,
        ]);
        let (action_tx, mut action_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = broadcast::channel(8);
        let (_stop_tx, stop_rx) = tokio::sync::oneshot::channel();

        run_recovery_state_observation(states, account_key.clone(), action_tx, event_tx, stop_rx)
            .await;

        let first_actions = action_rx.recv().await.expect("first action batch");
        assert_eq!(
            first_actions,
            vec![AppAction::E2eeRecoveryStateChanged {
                state: matrix_desktop_state::E2eeRecoveryState::Incomplete,
                methods: vec![matrix_desktop_state::RecoveryMethod::RecoveryKey],
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
                state: matrix_desktop_state::E2eeRecoveryState::Enabled,
                methods: vec![matrix_desktop_state::RecoveryMethod::RecoveryKey],
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

    // -----------------------------------------------------------------------
    // Recovery unit tests (network-free: use fake recovery port)
    // -----------------------------------------------------------------------

    /// Verify classify_recovery_error maps SDK error text to coarse kinds
    /// without leaking the raw message in any public type.
    #[test]
    fn recovery_error_classification_invalid_key() {
        let err = matrix_desktop_sdk::E2eeRecoveryError::Sdk("invalid recovery key".to_owned());
        assert_eq!(
            classify_recovery_error(&err),
            crate::failure::RecoveryFailureKind::InvalidRecoveryKey,
            "SDK 'invalid' text must map to InvalidRecoveryKey"
        );
    }

    #[test]
    fn recovery_error_classification_network() {
        let err = matrix_desktop_sdk::E2eeRecoveryError::Runtime("runtime error".to_owned());
        assert_eq!(
            classify_recovery_error(&err),
            crate::failure::RecoveryFailureKind::Network,
            "Runtime error must map to Network"
        );
    }

    #[test]
    fn recovery_error_classification_server_fallback() {
        let err = matrix_desktop_sdk::E2eeRecoveryError::Sdk("unexpected server error".to_owned());
        assert_eq!(
            classify_recovery_error(&err),
            crate::failure::RecoveryFailureKind::Server,
            "Unknown SDK error must map to Server (conservative)"
        );
    }

    /// Verify that RecoveryRequest's Debug output does not leak the secret.
    #[test]
    fn recovery_request_debug_redacts_secret() {
        use matrix_desktop_state::AuthSecret;
        let req = matrix_desktop_state::RecoveryRequest {
            secret: AuthSecret::new("super-secret-recovery-key"),
        };
        let debug = format!("{req:?}");
        assert!(
            !debug.contains("super-secret-recovery-key"),
            "RecoveryRequest Debug must redact the secret: {debug}"
        );
    }

    /// Network-free: SubmitRecovery without an active session must emit
    /// SessionRequired, not panic or crash.
    #[tokio::test]
    async fn submit_recovery_without_session_emits_session_required() {
        use matrix_desktop_state::AuthSecret;
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, _action_rx, mut event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());

        let request_id = test_request_id();
        assert!(
            handle
                .send(AccountMessage::Command(AccountCommand::SubmitRecovery {
                    request_id,
                    request: matrix_desktop_state::RecoveryRequest {
                        secret: AuthSecret::new("some-key"),
                    },
                }))
                .await
        );

        match event_rx.recv().await.expect("event") {
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } => {
                assert_eq!(ev_id, request_id);
                assert_eq!(failure, CoreFailure::SessionRequired);
            }
            other => panic!("expected OperationFailed(SessionRequired), got {other:?}"),
        }
    }

    /// Network-free: E2EE trust commands are ready-session operations. Without
    /// an active store-backed session they must fail as SessionRequired, not as
    /// local-encryption unavailable. LocalEncryptionUnavailable is reserved for
    /// store/key initialization failure, not for command gating.
    #[tokio::test]
    async fn e2ee_trust_commands_without_session_emit_session_required() {
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, mut event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());

        let request_id = test_request_id();
        assert!(
            handle
                .send(AccountMessage::Command(
                    AccountCommand::BootstrapCrossSigning {
                        request_id,
                        auth: None,
                    }
                ))
                .await
        );

        let actions = action_rx.recv().await.expect("trust failure action batch");
        assert_eq!(
            actions,
            vec![AppAction::BootstrapCrossSigningFailed {
                request_id: request_id.sequence,
                kind: matrix_desktop_state::TrustOperationFailureKind::Sdk,
            }]
        );

        match event_rx.recv().await.expect("event") {
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } => {
                assert_eq!(ev_id, request_id);
                assert_eq!(failure, CoreFailure::SessionRequired);
            }
            other => panic!("expected OperationFailed(SessionRequired), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn identity_reset_auth_without_session_settles_pending_state() {
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, mut event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());

        let request_id = test_request_id();
        let flow_id = 99;
        assert!(
            handle
                .send(AccountMessage::Command(
                    AccountCommand::SubmitIdentityResetAuth {
                        request_id,
                        flow_id,
                        request: matrix_desktop_state::IdentityResetAuthRequest::OAuthApproved,
                    }
                ))
                .await
        );

        let actions = action_rx.recv().await.expect("trust failure action batch");
        assert_eq!(
            actions,
            vec![AppAction::ResetIdentityFailed {
                request_id: flow_id,
                kind: matrix_desktop_state::TrustOperationFailureKind::Sdk,
            }]
        );

        match event_rx.recv().await.expect("event") {
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } => {
                assert_eq!(ev_id, request_id);
                assert_eq!(failure, CoreFailure::SessionRequired);
            }
            other => panic!("expected OperationFailed(SessionRequired), got {other:?}"),
        }
    }

    #[test]
    fn e2ee_trust_error_classification_is_kind_only() {
        assert_eq!(
            classify_e2ee_trust_error(&matrix_desktop_sdk::E2eeTrustError::NoOlmMachine),
            matrix_desktop_state::TrustOperationFailureKind::Sdk
        );
        assert_eq!(
            classify_e2ee_trust_error(&matrix_desktop_sdk::E2eeTrustError::Sdk(
                "timeout while talking to @alice:example.test".to_owned()
            )),
            matrix_desktop_state::TrustOperationFailureKind::Timeout
        );
        assert_eq!(
            classify_e2ee_trust_error(&matrix_desktop_sdk::E2eeTrustError::Sdk(
                "M_FORBIDDEN".to_owned()
            )),
            matrix_desktop_state::TrustOperationFailureKind::Forbidden
        );
    }

    #[test]
    fn e2ee_trust_sdk_results_project_actions_and_typed_events() {
        let request_id = test_request_id();
        let account_key = AccountKey("@alice:example.test".to_owned());

        let (actions, events) = project_bootstrap_cross_signing_result(
            request_id,
            account_key.clone(),
            Ok(matrix_desktop_state::CrossSigningStatus::Trusted),
        );
        assert_eq!(
            actions,
            vec![AppAction::CrossSigningStatusChanged {
                status: matrix_desktop_state::CrossSigningStatus::Trusted,
            }]
        );
        assert!(matches!(
            events.as_slice(),
            [CoreEvent::E2eeTrust(
                crate::event::E2eeTrustEvent::CrossSigningChanged {
                    status: matrix_desktop_state::CrossSigningStatus::Trusted,
                    ..
                }
            )]
        ));

        let (actions, events) = project_bootstrap_cross_signing_result(
            request_id,
            account_key,
            Err(matrix_desktop_sdk::E2eeTrustError::Sdk(
                "timeout from @alice:example.test".to_owned(),
            )),
        );
        assert_eq!(
            actions,
            vec![AppAction::BootstrapCrossSigningFailed {
                request_id: request_id.sequence,
                kind: matrix_desktop_state::TrustOperationFailureKind::Timeout,
            }]
        );
        assert!(matches!(
            events.as_slice(),
            [CoreEvent::E2eeTrust(
                crate::event::E2eeTrustEvent::CrossSigningChanged {
                    status: matrix_desktop_state::CrossSigningStatus::Failed {
                        kind: matrix_desktop_state::TrustOperationFailureKind::Timeout,
                        ..
                    },
                    ..
                }
            )]
        ));
        let debug = format!("{events:?}");
        assert!(!debug.contains("@alice:example.test"));
        assert!(!debug.contains("timeout from"));

        let (actions, events) = project_enable_key_backup_result(
            request_id,
            AccountKey("@alice:example.test".to_owned()),
            Ok(matrix_desktop_state::KeyBackupStatus::Enabled {
                version: "available".to_owned(),
            }),
        );
        assert_eq!(
            actions,
            vec![AppAction::KeyBackupEnabled {
                request_id: request_id.sequence,
                version: "available".to_owned(),
            }]
        );
        assert!(matches!(
            events.as_slice(),
            [CoreEvent::E2eeTrust(
                crate::event::E2eeTrustEvent::KeyBackupChanged {
                    status: matrix_desktop_state::KeyBackupStatus::Enabled { .. },
                    ..
                }
            )]
        ));

        let (actions, events) = project_restore_key_backup_result(
            request_id,
            AccountKey("@alice:example.test".to_owned()),
            Ok(matrix_desktop_sdk::KeyBackupRestoreSummary {
                version: Some("available".to_owned()),
                restored_rooms: 2,
                total_rooms: Some(3),
            }),
        );
        assert_eq!(
            actions,
            vec![
                AppAction::KeyBackupRestoreProgress {
                    request_id: request_id.sequence,
                    restored_rooms: 2,
                    total_rooms: Some(3),
                },
                AppAction::KeyBackupRestored {
                    request_id: request_id.sequence,
                    version: Some("available".to_owned()),
                },
            ]
        );
        assert!(matches!(
            events.as_slice(),
            [
                CoreEvent::E2eeTrust(crate::event::E2eeTrustEvent::KeyBackupChanged {
                    status: matrix_desktop_state::KeyBackupStatus::Restoring {
                        restored_rooms: 2,
                        total_rooms: Some(3),
                        ..
                    },
                    ..
                }),
                CoreEvent::E2eeTrust(crate::event::E2eeTrustEvent::KeyBackupChanged {
                    status: matrix_desktop_state::KeyBackupStatus::Enabled { .. },
                    ..
                })
            ]
        ));
    }

    #[test]
    fn identity_reset_sdk_results_project_actions_and_typed_events() {
        let request_id = test_request_id();
        let account_key = AccountKey("@alice:example.test".to_owned());

        let (actions, events) = project_reset_identity_completed(request_id, account_key.clone());
        assert_eq!(
            actions,
            vec![AppAction::ResetIdentityCompleted {
                request_id: request_id.sequence,
            }]
        );
        assert!(matches!(
            events.as_slice(),
            [CoreEvent::E2eeTrust(
                crate::event::E2eeTrustEvent::IdentityResetChanged {
                    state: matrix_desktop_state::IdentityResetState::Idle,
                    ..
                }
            )]
        ));

        let (actions, events) = project_reset_identity_auth_required(
            request_id,
            account_key,
            matrix_desktop_state::IdentityResetAuthType::Uiaa,
        );
        assert_eq!(
            actions,
            vec![AppAction::ResetIdentityAuthRequired {
                request_id: request_id.sequence,
                auth_type: matrix_desktop_state::IdentityResetAuthType::Uiaa,
            }]
        );
        assert!(matches!(
            events.as_slice(),
            [CoreEvent::E2eeTrust(
                crate::event::E2eeTrustEvent::IdentityResetChanged {
                    state: matrix_desktop_state::IdentityResetState::AwaitingAuth {
                        auth_type: matrix_desktop_state::IdentityResetAuthType::Uiaa,
                        ..
                    },
                    ..
                }
            )]
        ));

        let debug = format!("{events:?}");
        assert!(!debug.contains("@alice:example.test"));
    }
}
