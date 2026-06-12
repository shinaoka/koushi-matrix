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

use matrix_desktop_auth::{MatrixClientSession, PersistableMatrixSession};
use matrix_desktop_key::{SessionKeyId, StoredMatrixSession};
use matrix_desktop_state::{AppAction, LoginRequest, SessionInfo};
use tokio::sync::{broadcast, mpsc};

use crate::command::{AccountCommand, RoomCommand, SyncCommand};
use crate::event::{AccountEvent, CoreEvent};
use crate::failure::{CoreFailure, LoginFailureKind};
use crate::ids::{AccountKey, RequestId};
use crate::room::{RoomActorHandle, RoomMessage};
use crate::store::{StoreActor, account_key_from_info, session_key_id_from_info};
use crate::sync::{SyncActorHandle, SyncMessage};

/// "Credential store healthy, but no stored session for that account"
/// during restore/switch (canon: `CoreFailure::SessionNotFound`).
const SESSION_NOT_FOUND_FAILURE: CoreFailure = CoreFailure::SessionNotFound;

/// Redacted message used in reducer error projections (never raw SDK text).
const RESTORE_FAILED_MESSAGE: &str = "session restore failed";

/// Messages routed to the AccountActor task.
pub enum AccountMessage {
    Command(AccountCommand),
    SyncCommand(SyncCommand),
    RoomCommand(RoomCommand),
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
    /// SyncActor child handle (Phase 3). Present only when a store-backed
    /// session exists. Created on first login/restore; destroyed on logout /
    /// account switch.
    sync_actor: Option<SyncActorHandle>,
    /// RoomActor child handle (Phase 4). Spawned once at actor creation and
    /// kept alive for the lifetime of the AccountActor. Session is provided
    /// via `RoomMessage::SyncStarted` when sync begins.
    room_actor: RoomActorHandle,
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
        let actor = AccountActor {
            session: None,
            session_key_id: None,
            store: store_actor,
            action_tx,
            event_tx,
            command_rx,
            sync_actor: None,
            room_actor,
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
            }
        }
        // Ordered shutdown (overview.md Async rule 12):
        // timelines (phase 5 no-op) → search (phase 6 no-op) → room → sync → SDK handles.
        self.stop_room_actor().await;
        self.stop_sync_actor().await;
        // Drop the session handle inside the runtime context
        // (overview.md Async rule 11 — deadpool-runtime panic prevention).
        drop(self.session.take());
    }

    /// Route a RoomCommand to the RoomActor. The RoomActor handles the
    /// SessionRequired check internally (it holds the session ref after
    /// SyncStarted).
    async fn route_room_command(&self, command: RoomCommand) {
        let _ = self
            .room_actor
            .send(RoomMessage::Command(command))
            .await;
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
    /// notify the RoomActor so it can set up the room list.
    fn spawn_sync_actor(&mut self, session: Arc<MatrixClientSession>) {
        // Notify the RoomActor that we have a session. It will do a one-shot
        // room list snapshot (auth::room_list_snapshot covers both backends).
        // We use try_send since spawn_sync_actor is a sync fn; the channel
        // capacity of 64 is more than enough for this one message.
        let session_for_room = session.clone();
        self.room_actor.try_send(RoomMessage::SyncStarted {
            session: session_for_room,
        });

        let handle = crate::sync::SyncActor::spawn(
            session,
            self.action_tx.clone(),
            self.event_tx.clone(),
        );
        self.sync_actor = Some(handle);
    }

    /// Ordered shutdown of the SyncActor (step 4 of the shutdown sequence).
    async fn stop_sync_actor(&mut self) {
        if let Some(handle) = self.sync_actor.take() {
            let _ = handle.send(SyncMessage::Shutdown).await;
            handle.join().await;
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
                request: _,
            } => {
                // Phase 2 stub: recovery is a Phase 8 feature.
                self.emit_failure(
                    request_id,
                    CoreFailure::RecoveryFailed {
                        kind: crate::failure::RecoveryFailureKind::Server,
                    },
                );
            }
        }
    }

    async fn handle_login_password(
        &mut self,
        request_id: RequestId,
        request: LoginRequest,
    ) {
        // Store bootstrap step 1: the password exchange runs on a storeless
        // client. The device id (and therefore the store path) is unknown
        // before this completes. The storeless client must never sync or
        // initialize encryption.
        let login_result =
            matrix_desktop_auth::login_with_password_with_store(&request, None).await;

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
        self.spawn_sync_actor(session_arc);

        // Project login success through the reducer.
        self.reduce(vec![AppAction::LoginSucceeded(info)]);

        // Emit domain event.
        self.emit(CoreEvent::Account(AccountEvent::LoggedIn {
            request_id,
            account_key,
        }));
    }

    async fn handle_restore_session(
        &mut self,
        request_id: RequestId,
        account_key: AccountKey,
    ) {
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

    async fn handle_switch_account(
        &mut self,
        request_id: RequestId,
        account_key: AccountKey,
    ) {
        // Ordered shutdown of the current account runtime WITHOUT clearing
        // credentials or stores.
        // Phase 3: stop sync. Phases 4-6 add their children here.
        self.stop_sync_actor().await;
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
                self.spawn_sync_actor(session_arc);

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

        // Ordered shutdown step 4: stop sync before dropping the session.
        self.stop_sync_actor().await;

        // Attempt server-side logout (best-effort; local cleanup always happens).
        let _ = matrix_desktop_auth::logout(&session).await;

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
    async fn restore_into_store(
        &self,
        persistable: &PersistableMatrixSession,
        key_id: &SessionKeyId,
    ) -> Result<MatrixClientSession, CoreFailure> {
        let store_config = self.store.account_store_config(key_id)?;
        matrix_desktop_auth::restore_session_with_store(
            persistable,
            Some(&store_config.store_config),
        )
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
        let _ = matrix_desktop_auth::logout(&login_session).await;
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
    fn lookup_session_key_id(
        &self,
        account_key: &AccountKey,
    ) -> Result<Option<SessionKeyId>, ()> {
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

/// Map a `PasswordLoginError` to a coarse `LoginFailureKind` without exposing
/// raw SDK error text in public events.
fn classify_login_error(error: &matrix_desktop_auth::PasswordLoginError) -> LoginFailureKind {
    use matrix_desktop_auth::{LoginDiscoveryError, PasswordLoginError};
    match error {
        PasswordLoginError::InvalidHomeserver(discovery_err) => match discovery_err {
            LoginDiscoveryError::RequestFailed(_)
            | LoginDiscoveryError::HttpStatus { .. } => LoginFailureKind::Network,
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
    use tempfile::tempdir;
    use tokio::sync::{broadcast, mpsc};

    use super::*;
    use crate::store::CredentialStoreBackend;

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
}
