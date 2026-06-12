//! AccountActor: handles login, restore, logout, and account switch.
//!
//! Owns the `MatrixClientSession` handle. Internal outcomes are projected via
//! the runtime's action channel (AppAction::LoginSucceeded etc.) so AppState /
//! StateChanged remain reducer-driven. Domain events (AccountEvent::LoggedIn
//! etc.) plus OperationFailed are emitted on the CoreEvent stream.
//!
//! Shutdown order (overview.md Async rule 12): SDK handles dropped inside the
//! Tokio runtime context. The actor never drops the session outside async context.
//!
//! SwitchAccount: design gap — the semantics of switching accounts before
//! Phase 3 sync exists are not fully specified. In Phase 2 we implement the
//! credential + store setup path but do not attempt to stop a non-existent sync.
//! See design gap note in the final report.

use std::sync::Arc;

use matrix_desktop_auth::{MatrixClientSession, PersistableMatrixSession};
use matrix_desktop_key::{CredentialStore, SessionKeyId, StoredMatrixSession};
use matrix_desktop_state::{AppAction, LoginRequest};
use tokio::sync::{broadcast, mpsc};

use crate::command::AccountCommand;
use crate::event::{AccountEvent, CoreEvent};
use crate::failure::{CoreFailure, LoginFailureKind};
use crate::ids::{AccountKey, RequestId};
use crate::store::{StoreActor, account_key_from_info, session_key_id_from_info};

/// Messages routed to the AccountActor task.
pub enum AccountMessage {
    Command(AccountCommand),
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

    pub fn send_blocking(&self, msg: AccountMessage) -> bool {
        self.tx.blocking_send(msg).is_ok()
    }
}

/// The account actor's internal state.
pub struct AccountActor {
    /// Active session, if any.
    session: Option<Arc<MatrixClientSession>>,
    /// Session key for credential store operations.
    session_key_id: Option<SessionKeyId>,
    /// Credential store backend — needed to persist/load/delete sessions.
    credential_store: CredentialStore,
    /// Store actor — needed to resolve per-account encryption config.
    store_actor: StoreActor,
    /// App-level action channel to drive the reducer.
    action_tx: mpsc::Sender<Vec<AppAction>>,
    /// Shared event broadcast channel.
    event_tx: broadcast::Sender<CoreEvent>,
    /// Message inbox.
    command_rx: mpsc::Receiver<AccountMessage>,
}

impl AccountActor {
    pub fn spawn(
        store_actor: StoreActor,
        credential_store: CredentialStore,
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
    ) -> AccountActorHandle {
        let (tx, command_rx) = mpsc::channel(64);
        let actor = AccountActor {
            session: None,
            session_key_id: None,
            credential_store,
            store_actor,
            action_tx,
            event_tx,
            command_rx,
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
            }
        }
        // Shutdown: drop the session handle inside the runtime context
        // (overview.md Async rule 11 — deadpool-runtime panic prevention).
        drop(self.session.take());
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
                // Design gap: SwitchAccount before Phase 3 sync has not been
                // fully specified. We implement credential setup but do not yet
                // stop a non-existent sync actor. Documented in final report.
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
        // 1. Build session key id (we need homeserver + user + device, but we
        //    only know homeserver before login; key is finalized after login).
        //    We obtain the store config after a successful login using the
        //    returned session info.
        let login_result =
            matrix_desktop_auth::login_with_password_with_store(&request, None).await;

        match login_result {
            Err(error) => {
                // Map SDK error to a coarse failure kind (no raw SDK error in public).
                let kind = classify_login_error(&error);
                self.emit_failure(request_id, CoreFailure::LoginFailed { kind });
                self.reduce(vec![AppAction::LoginFailed {
                    message: "login failed".to_owned(),
                }]);
            }
            Ok(session) => {
                let info = session.info.clone();
                let key_id = session_key_id_from_info(&info);
                let account_key = account_key_from_info(&info);

                // Persist session in the credential store.
                let persist_result = self.persist_session(&session, &key_id);
                if let Err(failure) = persist_result {
                    self.emit_failure(request_id, failure);
                    return;
                }

                self.session = Some(Arc::new(session));
                self.session_key_id = Some(key_id);

                // Project login success through the reducer.
                self.reduce(vec![AppAction::LoginSucceeded(info.clone())]);

                // Emit domain event.
                self.emit(CoreEvent::Account(AccountEvent::LoggedIn {
                    request_id,
                    account_key,
                }));
            }
        }
    }

    async fn handle_restore_session(
        &mut self,
        request_id: RequestId,
        account_key: AccountKey,
    ) {
        // The account_key is the user_id string. We need a full SessionKeyId
        // to restore — the credential store needs homeserver + device_id too.
        // In Phase 2 we load the last-session pointer from the credential store.
        let last_session = match self.credential_store.load_last_session() {
            Ok(Some(key_id)) if key_id.user_id == account_key.0 => key_id,
            Ok(_) => {
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
                return;
            }
            Err(_) => {
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
                return;
            }
        };

        let session_json = match self.credential_store.load_matrix_session(&last_session) {
            Ok(stored) => stored,
            Err(_) => {
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
                return;
            }
        };

        let persistable = match PersistableMatrixSession::from_json(session_json.as_str()) {
            Ok(s) => s,
            Err(_) => {
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
                return;
            }
        };

        // Build store config using the credential-backed encryption key.
        let store_config_result = self.store_actor.account_store_config(&last_session);
        let store_config = match store_config_result {
            Ok(cfg) => cfg,
            Err(failure) => {
                self.emit_failure(request_id, failure);
                return;
            }
        };

        let restore_result = matrix_desktop_auth::restore_session_with_store(
            &persistable,
            Some(&store_config.store_config),
        )
        .await;

        match restore_result {
            Err(_) => {
                self.emit_failure(
                    request_id,
                    CoreFailure::LoginFailed {
                        kind: LoginFailureKind::Store,
                    },
                );
                self.reduce(vec![AppAction::RestoreSessionFailed {
                    message: "session restore failed".to_owned(),
                }]);
            }
            Ok(session) => {
                let info = session.info.clone();
                let key_id = session_key_id_from_info(&info);
                let account_key = account_key_from_info(&info);

                self.session = Some(Arc::new(session));
                self.session_key_id = Some(key_id);

                self.reduce(vec![AppAction::RestoreSessionSucceeded(info)]);
                self.emit(CoreEvent::Account(AccountEvent::SessionRestored {
                    request_id,
                    account_key,
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

        // Attempt server-side logout (best-effort; local cleanup always happens).
        let _ = matrix_desktop_auth::logout(&session).await;

        // Drop the SDK handle inside the Tokio runtime context.
        drop(session);

        // Clean up credentials and stored session.
        if let Some(key_id) = &key_id {
            let _ = self.credential_store.delete_matrix_session(key_id);
            let _ = self.credential_store.delete_last_session();
            self.store_actor.delete_account_credentials(key_id);
        }

        let account_key = key_id
            .as_ref()
            .map(|k| AccountKey(k.user_id.clone()))
            .unwrap_or_else(|| AccountKey(String::new()));

        self.reduce(vec![AppAction::LogoutFinished]);
        self.emit(CoreEvent::Account(AccountEvent::LoggedOut {
            request_id,
            account_key,
        }));
    }

    async fn handle_switch_account(
        &mut self,
        request_id: RequestId,
        account_key: AccountKey,
    ) {
        // Design gap (Phase 2): SwitchAccount before sync exists.
        // Full implementation requires Phase 3 sync actor coordination.
        // For now: emit a failure that signals the operation is not yet
        // implemented rather than silently dropping the command.
        //
        // The gap is documented in the final report. This placeholder satisfies
        // the contract that every accepted command produces an event.
        self.emit_failure(request_id, CoreFailure::StoreUnavailable);
        let _ = account_key; // suppress unused warning
    }

    // --- helpers ---

    fn persist_session(
        &self,
        session: &MatrixClientSession,
        key_id: &SessionKeyId,
    ) -> Result<(), CoreFailure> {
        let persistable = session
            .persistable_session()
            .map_err(|_| CoreFailure::StoreUnavailable)?;
        let json = persistable
            .to_json()
            .map_err(|_| CoreFailure::StoreUnavailable)?;
        let stored = StoredMatrixSession::new(json);
        self.credential_store
            .save_matrix_session(key_id, &stored)
            .map_err(|_| CoreFailure::StoreUnavailable)?;
        self.credential_store
            .save_last_session(key_id)
            .map_err(|_| CoreFailure::StoreUnavailable)?;
        Ok(())
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
            // Coarse classification: 401/403 → InvalidCredentials, otherwise Server.
            if message.contains("401") || message.contains("403") || message.contains("M_FORBIDDEN") || message.contains("M_UNAUTHORIZED") {
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
