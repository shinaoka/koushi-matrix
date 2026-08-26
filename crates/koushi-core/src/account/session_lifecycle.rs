//! `session_lifecycle` ownership for AccountActor.

use std::{
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_key::{SessionKeyId, StoredMatrixSession};
use koushi_sdk::{MatrixClientSession, PendingOidcLogin, PersistableMatrixSession};
use koushi_state::{
    AppAction, AuthFailureKind, LoginAttemptId, LoginRequest, SessionInfo, SlidingSyncAdmission,
};
use tokio::sync::{mpsc, oneshot};

use crate::event::{AccountEvent, CoreEvent};
use crate::executor;
use crate::failure::{CoreFailure, LoginFailureKind};
use crate::ids::{AccountKey, RequestId};
use crate::startup_trace::{self, StartupPhase};
use crate::store::{account_key_from_info, session_key_id_from_info};

use super::actor::{AccountActor, AccountMessage, trace_account_request, trace_restore};
use super::sliding_sync::PendingSlidingSyncAdmission;
use super::trust_gate::{
    current_device_trust_token, record_verification_admission_event,
    record_verification_method_discovery_event, verification_admission_event,
    verification_method_discovery_event,
};
use super::verification::send_observer_output_until_stopped;

/// "Credential store healthy, but no stored session for that account"
/// during restore/switch (canon: `CoreFailure::SessionNotFound`).
const SESSION_NOT_FOUND_FAILURE: CoreFailure = CoreFailure::SessionNotFound;

const SERVER_LOGOUT_TIMEOUT: Duration = Duration::from_secs(10);

/// The device display-name repair is cosmetic: never let a hanging devices
/// endpoint hold up login on the critical path (#474 review).
const DEVICE_NAME_TIMEOUT: Duration = Duration::from_secs(5);

const OIDC_REDIRECT_URI: &str = "koushi-desktop://auth/callback";

/// Redacted message used in reducer error projections (never raw SDK text).
pub(super) const RESTORE_FAILED_MESSAGE: &str = "session restore failed";

pub(super) fn trace_restore_simple(stage: &'static str, action: &'static str) {
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "core.account", stage)
            .field(DiagnosticField::token("action", action)),
    );
}

fn record_device_name_outcome(outcome: koushi_sdk::MatrixDeviceNameOutcome) {
    use koushi_sdk::MatrixDeviceNameOutcome;

    let (inspection, rename) = match outcome {
        MatrixDeviceNameOutcome::Present => ("present", None),
        MatrixDeviceNameOutcome::Renamed => ("empty", Some("success")),
        MatrixDeviceNameOutcome::RenameFailed => ("empty", Some("failed")),
        MatrixDeviceNameOutcome::CurrentDeviceMissing
        | MatrixDeviceNameOutcome::InspectionFailed => ("failed", None),
    };
    record(
        DiagnosticEvent::new(DiagnosticLevel::Info, "device_name", "inspected")
            .field(DiagnosticField::token("outcome", inspection)),
    );
    if let Some(rename) = rename {
        record(
            DiagnosticEvent::new(
                if rename == "success" {
                    DiagnosticLevel::Info
                } else {
                    DiagnosticLevel::Warn
                },
                "device_name",
                "rename_settled",
            )
            .field(DiagnosticField::token("outcome", rename)),
        );
    }
}

fn restore_store_event(action: &'static str, request_id: Option<RequestId>) -> DiagnosticEvent {
    let event = DiagnosticEvent::new(DiagnosticLevel::Debug, "core.account", "restore_store")
        .field(DiagnosticField::token("action", action));
    match request_id {
        Some(request_id) => event.field(DiagnosticField::request_id(
            "request_id",
            request_id.connection_id.0,
            request_id.sequence,
        )),
        None => event,
    }
}

fn record_restore_store_event(event: DiagnosticEvent) {
    koushi_diagnostics::record(event);
}

/// How a successful store-backed restore is reported.
#[derive(Clone, Copy)]
pub(super) enum RestoreOutcome {
    /// `RestoreSession` command → `AccountEvent::SessionRestored`.
    Restored,
    /// `SwitchAccount` command → `AccountEvent::AccountSwitched`.
    Switched,
}

pub(super) struct SessionChangeObservation {
    stop_tx: oneshot::Sender<()>,
    task: crate::executor::JoinHandle<()>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionInvalidationReason {
    UnknownToken { soft_logout: bool },
}

pub(super) struct PendingSessionTeardown {
    generation: u64,
    attempt: u32,
    pub(super) session: Arc<MatrixClientSession>,
    key_id: Option<SessionKeyId>,
    pub(super) continuation: SessionTeardownContinuation,
}

pub(super) enum SessionTeardownContinuation {
    Logout {
        request_id: RequestId,
        server_logout: bool,
        preserve_persistence: bool,
    },
    InstallReplacement {
        session: MatrixClientSession,
        persistable: PersistableMatrixSession,
        key_id: SessionKeyId,
        action: AppAction,
    },
}

pub(super) enum PendingOidcFlow {
    Sdk(PendingOidcLogin),
    #[cfg(test)]
    Synthetic {
        homeserver: String,
    },
}

impl PendingOidcFlow {
    fn homeserver(&self) -> &str {
        match self {
            Self::Sdk(pending) => pending.homeserver(),
            #[cfg(test)]
            Self::Synthetic { homeserver } => homeserver,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ServerLogoutOutcome {
    Completed,
    Failed,
    TimedOut,
}

async fn logout_server_best_effort(session: &MatrixClientSession) -> ServerLogoutOutcome {
    wait_for_server_logout_best_effort(SERVER_LOGOUT_TIMEOUT, koushi_sdk::logout(session)).await
}

async fn wait_for_server_logout_best_effort<F>(timeout: Duration, request: F) -> ServerLogoutOutcome
where
    F: Future<Output = Result<(), koushi_sdk::PasswordLoginError>>,
{
    match tokio::time::timeout(timeout, request).await {
        Ok(Ok(())) => ServerLogoutOutcome::Completed,
        Ok(Err(_)) => ServerLogoutOutcome::Failed,
        Err(_) => ServerLogoutOutcome::TimedOut,
    }
}

fn session_info_from_key_id(key_id: &SessionKeyId) -> SessionInfo {
    SessionInfo {
        homeserver: key_id.homeserver.clone(),
        user_id: key_id.user_id.clone(),
        device_id: key_id.device_id.clone(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    }
}

async fn run_session_change_observation(
    mut changes: tokio::sync::broadcast::Receiver<matrix_sdk::SessionChange>,
    tx: mpsc::Sender<AccountMessage>,
    mut stop_rx: oneshot::Receiver<()>,
    #[cfg(test)] delivery_barrier: Option<Arc<tokio::sync::Barrier>>,
) {
    loop {
        tokio::select! {
            _ = &mut stop_rx => break,
            change = changes.recv() => {
                match change {
                    Ok(matrix_sdk::SessionChange::UnknownToken(data)) => {
                        record(
                            DiagnosticEvent::new(
                                DiagnosticLevel::Info,
                                "core.account",
                                "session_change_received",
                            )
                            .field(DiagnosticField::token("source", "matrix_sdk"))
                            .field(DiagnosticField::token("reason", "unknown_token"))
                            .field(DiagnosticField::boolean("soft_logout", data.soft_logout)),
                        );
                        #[cfg(test)]
                        if let Some(barrier) = delivery_barrier.as_ref() {
                            barrier.wait().await;
                        }
                        if !send_observer_output_until_stopped(
                            &tx,
                            AccountMessage::SessionInvalidated {
                                reason: SessionInvalidationReason::UnknownToken {
                                    soft_logout: data.soft_logout,
                                },
                            },
                            &mut stop_rx,
                        )
                        .await {
                            break;
                        }
                        break;
                    }
                    Ok(matrix_sdk::SessionChange::TokensRefreshed) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

/// Map a `PasswordLoginError` to a coarse `LoginFailureKind` without exposing
/// raw SDK error text in public events.
fn classify_login_error(error: &koushi_sdk::PasswordLoginError) -> LoginFailureKind {
    use koushi_sdk::{LoginDiscoveryError, PasswordLoginError};
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

fn login_discovery_failure_kind(error: &koushi_sdk::LoginDiscoveryError) -> AuthFailureKind {
    match error {
        koushi_sdk::LoginDiscoveryError::RequestFailed(_) => AuthFailureKind::Network,
        koushi_sdk::LoginDiscoveryError::HttpStatus { status: 403, .. } => {
            AuthFailureKind::Forbidden
        }
        koushi_sdk::LoginDiscoveryError::HttpStatus { .. }
        | koushi_sdk::LoginDiscoveryError::MissingFlows
        | koushi_sdk::LoginDiscoveryError::InvalidResponse(_) => AuthFailureKind::Sdk,
        koushi_sdk::LoginDiscoveryError::InvalidHomeserver(_)
        | koushi_sdk::LoginDiscoveryError::UnsupportedHomeserverScheme
        | koushi_sdk::LoginDiscoveryError::InsecureHomeserverScheme => AuthFailureKind::Unsupported,
    }
}

fn classify_auth_error(error: &koushi_sdk::PasswordLoginError) -> AuthFailureKind {
    match error {
        koushi_sdk::PasswordLoginError::InvalidHomeserver(discovery_err) => {
            login_discovery_failure_kind(discovery_err)
        }
        koushi_sdk::PasswordLoginError::Sdk(message) => {
            if message.contains("401")
                || message.contains("403")
                || message.contains("M_FORBIDDEN")
                || message.contains("M_UNAUTHORIZED")
            {
                AuthFailureKind::Forbidden
            } else {
                AuthFailureKind::Sdk
            }
        }
        koushi_sdk::PasswordLoginError::Runtime(_)
        | koushi_sdk::PasswordLoginError::MissingSession
        | koushi_sdk::PasswordLoginError::Serialization(_) => AuthFailureKind::Sdk,
    }
}

impl AccountActor {
    #[cfg(feature = "test-hooks")]
    pub(super) async fn install_residency_test_session(
        &mut self,
        session: Arc<MatrixClientSession>,
    ) {
        if self.session.is_some() {
            self.residency_preserve_room_session = true;
            self.stop_current_session_runtime().await;
            self.residency_preserve_room_session = false;
        }
        self.session = Some(session.clone());
        self.session_key_id = None;
        self.spawn_sync_actor(session.clone()).await;
        let _ = self.room_actor.wait_for_session(&session).await;
    }

    pub(super) fn start_scheduled_send_capability_probe(&self, session: Arc<MatrixClientSession>) {
        let action_tx = self.action_tx.clone();
        crate::executor::spawn(async move {
            let capability = crate::scheduled_send::detect_capability(&session.client()).await;
            let _ = action_tx
                .send(vec![AppAction::ScheduledSendCapabilityChanged {
                    capability,
                }])
                .await;
        });
    }

    pub(super) fn start_session_change_observer(&mut self, session: Arc<MatrixClientSession>) {
        let (stop_tx, stop_rx) = oneshot::channel();
        let changes = session.client().subscribe_to_session_changes();
        let tx = self.self_tx.clone();
        let task = crate::executor::spawn(run_session_change_observation(
            changes,
            tx,
            stop_rx,
            #[cfg(test)]
            None,
        ));
        self.session_change_observer = Some(SessionChangeObservation { stop_tx, task });
    }

    pub(super) async fn stop_session_change_observer(&mut self) {
        if let Some(observation) = self.session_change_observer.take() {
            let _ = observation.stop_tx.send(());
            let _ = observation.task.await;
        }
    }

    pub(super) async fn handle_session_invalidated(&mut self, reason: SessionInvalidationReason) {
        let SessionInvalidationReason::UnknownToken { soft_logout } = reason;
        if self.session.is_none() || !self.session_promoted {
            return;
        }
        trace_restore!(
            "session_invalidated",
            [
                DiagnosticField::token("reason", "unknown_token"),
                DiagnosticField::boolean("soft_logout", soft_logout),
                DiagnosticField::token("action", "lock"),
            ],
            "reason=unknown_token soft_logout={} action=lock",
            bool_trace_label(soft_logout)
        );

        self.send_actions(vec![AppAction::SessionAuthenticationInvalidated {
            soft_logout,
        }])
        .await;
        self.session_promoted = false;
        self.stop_provisional_runtime().await;
        self.stop_active_session_account_management_discovery()
            .await;
        self.invalidate_account_hydration();
        self.stop_sync_actor().await;
    }

    pub(super) async fn handle_discover_login(
        &mut self,
        _request_id: RequestId,
        homeserver: String,
    ) {
        let requested_homeserver = homeserver.clone();
        let discovery_result =
            tokio::task::spawn_blocking(move || koushi_sdk::discover_login_flows(&homeserver))
                .await;

        match discovery_result {
            Ok(Ok(discovery)) => {
                self.send_actions(vec![AppAction::LoginDiscoverySucceeded {
                    homeserver: requested_homeserver,
                    flows: discovery.flows,
                    delegated: discovery.delegated,
                }])
                .await;
            }
            Ok(Err(error)) => {
                self.send_actions(vec![AppAction::LoginDiscoveryFailed {
                    homeserver: requested_homeserver,
                    kind: login_discovery_failure_kind(&error),
                }])
                .await;
            }
            Err(_) => {
                self.send_actions(vec![AppAction::LoginDiscoveryFailed {
                    homeserver: requested_homeserver,
                    kind: AuthFailureKind::Sdk,
                }])
                .await;
            }
        }
    }

    pub(super) async fn handle_start_oidc_login(
        &mut self,
        request_id: RequestId,
        homeserver: String,
    ) {
        match koushi_sdk::start_oidc_login(&homeserver, OIDC_REDIRECT_URI).await {
            Ok((pending, authorization)) => {
                self.pending_oidc_login = Some((request_id, PendingOidcFlow::Sdk(pending)));
                self.emit(CoreEvent::Account(AccountEvent::OidcAuthorizationCreated {
                    request_id,
                    authorization_url: authorization.authorization_url,
                    state: authorization.state,
                }));
            }
            Err(error) => {
                let kind = classify_auth_error(&error);
                self.send_actions(vec![AppAction::LoginDiscoveryFailed { homeserver, kind }])
                    .await;
                self.emit_failure(request_id, CoreFailure::AccountOperationFailed { kind });
            }
        }
    }

    pub(super) async fn handle_complete_oidc_login(
        &mut self,
        request_id: RequestId,
        callback_url: String,
        platform: koushi_state::DisplayPlatform,
    ) {
        if self.pending_session_teardown.is_some() {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        }
        let Some((start_request_id, pending)) = self.pending_oidc_login.take() else {
            self.send_actions(vec![AppAction::LoginDiscoveryFailed {
                homeserver: String::new(),
                kind: AuthFailureKind::Cancelled,
            }])
            .await;
            self.emit_failure(
                request_id,
                CoreFailure::AccountOperationFailed {
                    kind: AuthFailureKind::Cancelled,
                },
            );
            self.send_actions(vec![AppAction::LoginFailed {
                attempt_id: LoginAttemptId::new(request_id.connection_id.0, request_id.sequence),
                message: "login failed".to_owned(),
            }])
            .await;
            return;
        };
        let homeserver = pending.homeserver().to_owned();
        self.send_actions(vec![AppAction::AuthenticationStarted {
            attempt_id: LoginAttemptId::new(request_id.connection_id.0, request_id.sequence),
            homeserver: homeserver.clone(),
        }])
        .await;

        #[cfg(test)]
        let login_result = match self.oidc_completion_override.take() {
            Some(session) => Ok(session),
            None => match pending {
                PendingOidcFlow::Sdk(pending) => {
                    koushi_sdk::finish_oidc_login(pending, &callback_url).await
                }
                PendingOidcFlow::Synthetic { .. } => {
                    unreachable!("synthetic OIDC completion requires a session override")
                }
            },
        };
        #[cfg(not(test))]
        let login_result = match pending {
            PendingOidcFlow::Sdk(pending) => {
                koushi_sdk::finish_oidc_login(pending, &callback_url).await
            }
        };

        let login_session = match login_result {
            Ok(session) => session,
            Err(error) => {
                let kind = classify_auth_error(&error);
                self.send_actions(vec![AppAction::LoginDiscoveryFailed { homeserver, kind }])
                    .await;
                self.emit_failure(request_id, CoreFailure::AccountOperationFailed { kind });
                self.send_actions(vec![AppAction::LoginFailed {
                    attempt_id: LoginAttemptId::new(
                        request_id.connection_id.0,
                        request_id.sequence,
                    ),
                    message: "login failed".to_owned(),
                }])
                .await;
                return;
            }
        };

        let device_name_outcome = match tokio::time::timeout(
            DEVICE_NAME_TIMEOUT,
            koushi_sdk::ensure_device_display_name(
                &login_session,
                platform.oauth_device_display_name(),
            ),
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(_) => koushi_sdk::MatrixDeviceNameOutcome::InspectionFailed,
        };
        record_device_name_outcome(device_name_outcome);

        let info = login_session.info.clone();
        let key_id = session_key_id_from_info(&info);
        let account_key = account_key_from_info(&info);

        let persistable = match login_session.persistable_session() {
            Ok(persistable) => persistable,
            Err(_) => {
                self.abort_login(login_session, &key_id, false, true).await;
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
                self.send_actions(vec![AppAction::LoginFailed {
                    attempt_id: LoginAttemptId::new(
                        request_id.connection_id.0,
                        request_id.sequence,
                    ),
                    message: "login failed".to_owned(),
                }])
                .await;
                return;
            }
        };

        let Some((account_epoch, capability_request_id)) = self.next_sliding_sync_correlation()
        else {
            self.abort_login(login_session, &key_id, false, true).await;
            self.emit_failure(request_id, CoreFailure::StoreUnavailable);
            self.send_actions(vec![AppAction::LoginFailed {
                attempt_id: LoginAttemptId::new(request_id.connection_id.0, request_id.sequence),
                message: "login failed".to_owned(),
            }])
            .await;
            return;
        };
        let mut ready_events = vec![CoreEvent::Account(AccountEvent::LoggedIn {
            request_id: start_request_id,
            account_key: account_key.clone(),
        })];
        if request_id != start_request_id {
            ready_events.push(CoreEvent::Account(AccountEvent::LoggedIn {
                request_id,
                account_key,
            }));
        }
        self.begin_sliding_sync_capability_discovery(
            PendingSlidingSyncAdmission::NewLogin {
                account_epoch,
                request_id: capability_request_id,
                core_request_id: request_id,
                login_session,
                persistable,
                key_id,
                action: AppAction::LoginSucceeded {
                    attempt_id: LoginAttemptId::new(
                        request_id.connection_id.0,
                        request_id.sequence,
                    ),
                    info,
                },
                ready_events,
            },
            SlidingSyncAdmission::NewLogin {
                attempt_id: LoginAttemptId::new(request_id.connection_id.0, request_id.sequence),
            },
            homeserver,
        )
        .await;
    }

    pub(super) async fn handle_login_password(
        &mut self,
        request_id: RequestId,
        request: LoginRequest,
        platform: koushi_state::DisplayPlatform,
    ) {
        if self.pending_session_teardown.is_some() {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        }
        // Store bootstrap step 1: the password exchange runs on a storeless
        // client. The device id (and therefore the store path) is unknown
        // before this completes. The storeless client must never sync or
        // initialize encryption.
        let login_result = koushi_sdk::login_with_password_with_store(&request, None).await;

        let login_session = match login_result {
            Err(error) => {
                let kind = classify_login_error(&error);
                self.emit_failure(request_id, CoreFailure::LoginFailed { kind });
                self.send_actions(vec![AppAction::LoginFailed {
                    attempt_id: LoginAttemptId::new(
                        request_id.connection_id.0,
                        request_id.sequence,
                    ),
                    message: "login failed".to_owned(),
                }])
                .await;
                return;
            }
            Ok(session) => session,
        };

        let info = login_session.info.clone();
        let key_id = session_key_id_from_info(&info);
        let account_key = account_key_from_info(&info);
        let (login_session, info, key_id) = self
            .prefer_saved_device_for_password_login(
                login_session,
                info,
                key_id,
                &account_key,
                &request.password,
            )
            .await;

        // #474: a fresh or re-login device gets a descriptive display name
        // ("Koushi on macOS/Windows/Linux") when its authoritative name is
        // empty; a user-customized name is never overwritten. Cosmetic only:
        // a failure is recorded and login continues untouched.
        let device_name_outcome = match tokio::time::timeout(
            DEVICE_NAME_TIMEOUT,
            koushi_sdk::ensure_device_display_name(
                &login_session,
                platform.oauth_device_display_name(),
            ),
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(_) => koushi_sdk::MatrixDeviceNameOutcome::InspectionFailed,
        };
        record_device_name_outcome(device_name_outcome);

        // Build a restorable in-memory session shape without writing the
        // active credential index or last-session pointer before verification.
        let persistable = match login_session.persistable_session() {
            Ok(persistable) => persistable,
            Err(_) => {
                self.abort_login(login_session, &key_id, false, true).await;
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
                self.send_actions(vec![AppAction::LoginFailed {
                    attempt_id: LoginAttemptId::new(
                        request_id.connection_id.0,
                        request_id.sequence,
                    ),
                    message: "login failed".to_owned(),
                }])
                .await;
                return;
            }
        };

        let Some((account_epoch, capability_request_id)) = self.next_sliding_sync_correlation()
        else {
            self.abort_login(login_session, &key_id, false, true).await;
            self.emit_failure(request_id, CoreFailure::StoreUnavailable);
            self.send_actions(vec![AppAction::LoginFailed {
                attempt_id: LoginAttemptId::new(request_id.connection_id.0, request_id.sequence),
                message: "login failed".to_owned(),
            }])
            .await;
            return;
        };
        let homeserver = info.homeserver.clone();
        let attempt_id = LoginAttemptId::new(request_id.connection_id.0, request_id.sequence);
        self.begin_sliding_sync_capability_discovery(
            PendingSlidingSyncAdmission::NewLogin {
                account_epoch,
                request_id: capability_request_id,
                core_request_id: request_id,
                login_session,
                persistable,
                key_id,
                action: AppAction::LoginSucceeded { attempt_id, info },
                ready_events: vec![CoreEvent::Account(AccountEvent::LoggedIn {
                    request_id,
                    account_key,
                })],
            },
            SlidingSyncAdmission::NewLogin { attempt_id },
            homeserver,
        )
        .await;
    }

    pub(super) async fn handle_restore_session(
        &mut self,
        request_id: RequestId,
        account_key: AccountKey,
    ) {
        if self.pending_session_teardown.is_some() {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        }
        trace_account_request("restore_session", request_id, "lookup_key");
        let key_id = match self.lookup_session_key_id(&account_key).await {
            Ok(Some(key_id)) => {
                trace_account_request("restore_session", request_id, "key_found");
                key_id
            }
            Ok(None) => {
                trace_account_request("restore_session", request_id, "key_missing");
                // No stored session for this account: project
                // RestoreSessionNotFound so AppState returns to SignedOut, and
                // keep the redacted failure event for command correlation.
                self.send_actions(vec![AppAction::RestoreSessionNotFound])
                    .await;
                self.emit_failure(request_id, SESSION_NOT_FOUND_FAILURE);
                return;
            }
            Err(()) => {
                trace_account_request("restore_session", request_id, "key_lookup_failed");
                // Credential store unreachable.
                self.send_actions(vec![AppAction::RestoreSessionFailed {
                    message: RESTORE_FAILED_MESSAGE.to_owned(),
                }])
                .await;
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
    pub(super) async fn handle_restore_last_session(&mut self, request_id: RequestId) {
        if self.pending_session_teardown.is_some() {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        }
        trace_account_request("restore_last_session", request_id, "load_pointer");
        let store = self.store.clone();
        let key_id =
            match executor::spawn_blocking(move || store.credential_backend().load_last_session())
                .await
            {
                Ok(Ok(Some(key_id))) => {
                    trace_account_request("restore_last_session", request_id, "pointer_found");
                    key_id
                }
                Ok(Ok(None)) => {
                    trace_account_request("restore_last_session", request_id, "pointer_missing");
                    self.send_actions(vec![AppAction::RestoreSessionNotFound])
                        .await;
                    self.emit_failure(request_id, SESSION_NOT_FOUND_FAILURE);
                    return;
                }
                Ok(Err(_)) | Err(_) => {
                    trace_account_request(
                        "restore_last_session",
                        request_id,
                        "pointer_load_failed",
                    );
                    self.send_actions(vec![AppAction::RestoreSessionFailed {
                        message: RESTORE_FAILED_MESSAGE.to_owned(),
                    }])
                    .await;
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
    pub(super) async fn handle_query_saved_sessions(&self, request_id: RequestId) {
        let store = self.store.clone();
        match executor::spawn_blocking(move || store.credential_backend().load_saved_sessions())
            .await
        {
            Ok(Ok(index)) => {
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
            Ok(Err(_)) | Err(_) => {
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
            }
        }
    }

    pub(super) async fn handle_soft_logout_reauth(
        &mut self,
        request_id: RequestId,
        password: koushi_state::AuthSecret,
    ) {
        let Some(session) = self.session.as_ref() else {
            self.send_actions(vec![AppAction::SoftLogoutReauthFailed {
                request_id: request_id.sequence,
                kind: AuthFailureKind::Sdk,
            }])
            .await;
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        let info = session.info.clone();
        let key_id = session_key_id_from_info(&info);

        // Stop live sync immediately, but keep the locked session until the
        // password login succeeds so a bad password does not make retry impossible.
        self.stop_sync_actor().await;

        let login_session = match koushi_sdk::login_with_existing_device(
            &info.homeserver,
            &info.user_id,
            &info.device_id,
            &password,
        )
        .await
        {
            Ok(session) => session,
            Err(error) => {
                self.send_actions(vec![AppAction::SoftLogoutReauthFailed {
                    request_id: request_id.sequence,
                    kind: AuthFailureKind::Sdk,
                }])
                .await;
                let failure = CoreFailure::LoginFailed {
                    kind: classify_login_error(&koushi_sdk::PasswordLoginError::Sdk(
                        error.to_string(),
                    )),
                };
                self.emit_failure(request_id, failure);
                return;
            }
        };
        drop(password);

        let persistable = match self.persist_session(&login_session, &key_id).await {
            Ok(persistable) => persistable,
            Err(failure) => {
                self.abort_login(login_session, &key_id, false, true).await;
                self.send_actions(vec![AppAction::SoftLogoutReauthFailed {
                    request_id: request_id.sequence,
                    kind: AuthFailureKind::Sdk,
                }])
                .await;
                self.emit_failure(request_id, failure);
                return;
            }
        };

        // The locked session's observers own SDK streams and therefore keep the old client
        // alive. Stop and join them before replacing the session or subscribing successors.
        self.record_lifecycle_probe("recovery_observer_stop_requested");
        self.stop_recovery_observer().await;
        self.record_lifecycle_probe("recovery_observer_terminated");
        self.record_lifecycle_probe("incoming_verification_observer_stop_requested");
        self.stop_incoming_verification_observer().await;
        self.record_lifecycle_probe("incoming_verification_observer_terminated");
        self.stop_session_change_observer().await;
        self.invalidate_account_hydration();
        self.set_secure_backup_send_admitted(false);
        drop(self.session.take());
        self.session_key_id = None;

        let store_backed = match self.restore_into_store(&persistable, &key_id).await {
            Ok(session) => session,
            Err(failure) => {
                self.abort_login(login_session, &key_id, true, true).await;
                self.send_actions(vec![AppAction::SoftLogoutReauthFailed {
                    request_id: request_id.sequence,
                    kind: AuthFailureKind::Sdk,
                }])
                .await;
                self.emit_failure(request_id, failure);
                return;
            }
        };
        drop(login_session);

        let session_arc = Arc::new(store_backed);
        self.pending_uia_operations.clear();
        self.session = Some(session_arc.clone());
        self.session_key_id = Some(key_id);
        self.record_lifecycle_probe("incoming_verification_observer_subscribing");
        self.start_incoming_verification_observer(session_arc.clone())
            .await;
        self.spawn_sync_actor(session_arc.clone()).await;

        let account_key = account_key_from_info(&info);
        self.send_actions(vec![
            AppAction::SoftLogoutReauthSucceeded {
                request_id: request_id.sequence,
            },
            AppAction::LoginSucceeded {
                attempt_id: LoginAttemptId::new(request_id.connection_id.0, request_id.sequence),
                info,
            },
        ])
        .await;
        self.emit(CoreEvent::Account(AccountEvent::LoggedIn {
            request_id,
            account_key,
        }));
        self.spawn_account_hydration(session_arc.clone());

        self.start_recovery_observer(session_arc.clone());
        self.record_lifecycle_probe("recovery_observer_started");
        self.start_session_change_observer(session_arc);
    }

    pub(super) async fn handle_switch_account(
        &mut self,
        request_id: RequestId,
        account_key: AccountKey,
    ) {
        if self.pending_session_teardown.is_some() {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        }
        let key_id = match self.lookup_session_key_id(&account_key).await {
            Ok(Some(key_id)) => key_id,
            Ok(None) => {
                // Same not-found contract as RestoreSession.
                self.send_actions(vec![AppAction::RestoreSessionNotFound])
                    .await;
                self.emit_failure(request_id, SESSION_NOT_FOUND_FAILURE);
                return;
            }
            Err(()) => {
                self.send_actions(vec![AppAction::RestoreSessionFailed {
                    message: RESTORE_FAILED_MESSAGE.to_owned(),
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
                return;
            }
        };

        // Acknowledge the debug-operation teardown (cancel + join + inline
        // CancelledStale settlement) BEFORE the switch reset clears room
        // interactions, so no pending operation is stranded by the reset
        // (issue #538). Fail closed: unless the dangerous operation is
        // confirmed settled, do not proceed with the account switch.
        if !self.clear_room_actor_session().await {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        }

        // Project the switch intent so the reducer drives state
        // (SwitchingAccount → cleared views), then run the store-backed
        // restore of the target account.
        self.send_actions(vec![AppAction::SwitchAccountRequested {
            info: session_info_from_key_id(&key_id),
        }])
        .await;

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
        let restore_started = Some(startup_trace::now());
        trace_account_request("restore_account", request_id, "load_session");
        let session_json = match self.store.credential_backend().load_matrix_session(&key_id) {
            Ok(stored) => stored,
            Err(err) if koushi_key::is_missing_credential_error(&err) => {
                trace_account_request("restore_account", request_id, "session_missing");
                self.send_actions(vec![AppAction::RestoreSessionNotFound])
                    .await;
                self.emit_failure(request_id, SESSION_NOT_FOUND_FAILURE);
                return;
            }
            Err(_) => {
                trace_account_request("restore_account", request_id, "session_load_failed");
                self.send_actions(vec![AppAction::RestoreSessionFailed {
                    message: RESTORE_FAILED_MESSAGE.to_owned(),
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
                return;
            }
        };

        let persistable = match PersistableMatrixSession::from_json(session_json.as_str()) {
            Ok(s) => s,
            Err(_) => {
                trace_account_request("restore_account", request_id, "session_parse_failed");
                self.send_actions(vec![AppAction::RestoreSessionFailed {
                    message: RESTORE_FAILED_MESSAGE.to_owned(),
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
                return;
            }
        };

        startup_trace::trace_phase(StartupPhase::Restore, restore_started);
        let Some((account_epoch, capability_request_id)) = self.next_sliding_sync_correlation()
        else {
            self.send_actions(vec![AppAction::RestoreSessionFailed {
                message: RESTORE_FAILED_MESSAGE.to_owned(),
            }])
            .await;
            self.emit_failure(request_id, CoreFailure::StoreUnavailable);
            return;
        };
        let info = persistable.info.clone();
        let homeserver = info.homeserver.clone();
        self.begin_sliding_sync_capability_discovery(
            PendingSlidingSyncAdmission::StoredSessionRestore {
                account_epoch,
                request_id: capability_request_id,
                core_request_id: request_id,
                persistable,
                key_id,
                outcome,
            },
            SlidingSyncAdmission::StoredSessionRestore { info },
            homeserver,
        )
        .await;
    }

    pub(super) async fn perform_logout(
        &mut self,
        request_id: RequestId,
        server_logout: bool,
        preserve_persistence: bool,
    ) {
        if self.session.is_none()
            && let Some(pending) = self.pending_sliding_sync_admission.take()
        {
            self.cancel_sliding_sync_discovery_task().await;
            self.pending_sliding_sync_retry = None;
            self.stored_sliding_sync_admission = None;
            self.sliding_sync_revalidation_pending = None;
            let key_id = match pending {
                PendingSlidingSyncAdmission::NewLogin {
                    login_session,
                    key_id,
                    ..
                } => {
                    self.abort_login(login_session, &key_id, false, server_logout)
                        .await;
                    key_id
                }
                PendingSlidingSyncAdmission::StoredSessionRestore { key_id, .. } => {
                    if preserve_persistence {
                        self.forget_last_session_pointer_if_matches(&key_id).await;
                    } else {
                        self.clear_account_persistence(&key_id).await;
                    }
                    key_id
                }
            };
            self.send_actions(vec![AppAction::LogoutFinished]).await;
            self.emit(CoreEvent::Account(AccountEvent::LoggedOut {
                request_id,
                account_key: AccountKey(key_id.user_id),
            }));
            return;
        }
        if self.pending_session_teardown.is_some() {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        }
        self.cancel_sliding_sync_discovery_task().await;
        self.discard_pending_sliding_sync_admission().await;
        self.pending_sliding_sync_retry = None;
        self.stored_sliding_sync_admission = None;
        self.sliding_sync_revalidation_pending = None;
        self.set_secure_backup_send_admitted(false);

        // Fail closed: run the acknowledged RoomActor teardown BEFORE taking
        // the session, so a failure leaves the complete previous runtime
        // intact (issue #538).
        if !self.stop_current_session_runtime().await {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        }
        let session = match self.session.take() {
            Some(s) => s,
            None => {
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            }
        };
        let key_id = self.session_key_id.take();

        if server_logout {
            let _ = logout_server_best_effort(&session).await;
        }

        self.next_teardown_generation = self.next_teardown_generation.wrapping_add(1);
        let generation = self.next_teardown_generation;
        self.pending_session_teardown = Some(PendingSessionTeardown {
            generation,
            attempt: 0,
            session,
            key_id,
            continuation: SessionTeardownContinuation::Logout {
                request_id,
                server_logout,
                preserve_persistence,
            },
        });
        self.retry_session_teardown(generation).await;
    }

    pub(super) async fn close_pending_session_stores(
        &mut self,
        session: &MatrixClientSession,
    ) -> Result<(), ()> {
        #[cfg(test)]
        if let Some(success) = self.close_store_results.pop_front()
            && !success
        {
            return Err(());
        }
        koushi_sdk::close_session_stores(session)
            .await
            .map_err(|_| ())
    }

    pub(super) async fn retry_session_teardown(&mut self, generation: u64) {
        let Some(pending) = self.pending_session_teardown.as_ref() else {
            return;
        };
        if pending.generation != generation {
            return;
        }
        let session = pending.session.clone();
        if self.close_pending_session_stores(&session).await.is_err() {
            let pending = self
                .pending_session_teardown
                .as_mut()
                .expect("teardown remains pending after close failure");
            pending.attempt = pending.attempt.saturating_add(1);
            let shift = pending.attempt.min(5);
            let delay_ms = 25_u64.saturating_mul(1_u64 << shift).min(1_000);
            record(
                DiagnosticEvent::new(
                    DiagnosticLevel::Warn,
                    "core.account",
                    "session_store_close_retrying",
                )
                .field(DiagnosticField::count("attempt", pending.attempt as u64)),
            );
            self.record_lifecycle_probe("session_store_close_retrying");
            let tx = self.self_tx.clone();
            self.teardown_retry_task = Some(executor::spawn(async move {
                executor::sleep(Duration::from_millis(delay_ms)).await;
                let _ = tx
                    .send(AccountMessage::RetrySessionTeardown { generation })
                    .await;
            }));
            return;
        }
        if let Some(task) = self.teardown_retry_task.take() {
            task.abort();
        }
        self.record_lifecycle_probe("session_store_closed");
        record(DiagnosticEvent::new(
            DiagnosticLevel::Debug,
            "core.account",
            "session_store_closed",
        ));
        let pending = self
            .pending_session_teardown
            .take()
            .expect("successful teardown remains pending");
        drop(pending.session);
        match pending.continuation {
            SessionTeardownContinuation::Logout {
                request_id,
                server_logout,
                preserve_persistence,
            } => {
                let _ = server_logout;
                let account_key = if let Some(key_id) = &pending.key_id {
                    if preserve_persistence {
                        self.forget_last_session_pointer_if_matches(key_id).await;
                    } else {
                        self.clear_account_persistence(key_id).await;
                    }
                    AccountKey(key_id.user_id.clone())
                } else {
                    AccountKey(String::new())
                };
                self.record_lifecycle_probe(if preserve_persistence {
                    "session_persistence_preserved"
                } else {
                    "session_persistence_deleted"
                });
                self.send_actions(vec![AppAction::LogoutFinished]).await;
                self.emit(CoreEvent::Account(AccountEvent::LoggedOut {
                    request_id,
                    account_key,
                }));
            }
            SessionTeardownContinuation::InstallReplacement {
                session,
                persistable,
                key_id,
                action,
            } => {
                // Account replacement must preserve the source account's
                // credentials, saved-session index entry, last pointer, and
                // keyed store. Only its live SDK handles are drained/dropped.
                self.record_lifecycle_probe("replacement_teardown_complete");
                Box::pin(self.install_provisional_session(session, persistable, key_id, action))
                    .await;
            }
        }
    }

    pub(super) async fn handle_logout(&mut self, request_id: RequestId) {
        self.perform_logout(request_id, true, true).await;
    }

    pub(super) async fn handle_change_homeserver(&mut self, request_id: RequestId) {
        self.perform_logout(request_id, false, true).await;
    }

    pub(super) async fn install_provisional_session(
        &mut self,
        session: MatrixClientSession,
        persistable: PersistableMatrixSession,
        key_id: SessionKeyId,
        action: AppAction,
    ) {
        debug_assert!(self.pending_session_teardown.is_none());
        if self.session.is_some() {
            self.set_secure_backup_send_admitted(false);
        }
        if let Some(previous_session) = self.session.take() {
            let previous_key_id = self.session_key_id.take();
            if !self.stop_current_session_runtime().await {
                // Fail closed: do not replace the session unless the
                // encryption-debug operation is confirmed settled. Restore
                // the previous session and surface the failure.
                self.session = Some(previous_session);
                self.session_key_id = previous_key_id;
                record(DiagnosticEvent::new(
                    DiagnosticLevel::Warn,
                    "core.room_key_debug",
                    "session_replacement_aborted_teardown_unconfirmed",
                ));
                return;
            }
            self.next_teardown_generation = self.next_teardown_generation.wrapping_add(1);
            let generation = self.next_teardown_generation;
            self.pending_session_teardown = Some(PendingSessionTeardown {
                generation,
                attempt: 0,
                session: previous_session,
                key_id: previous_key_id,
                continuation: SessionTeardownContinuation::InstallReplacement {
                    session,
                    persistable,
                    key_id,
                    action,
                },
            });
            self.retry_session_teardown(generation).await;
            return;
        }
        self.stop_provisional_runtime().await;
        let session = Arc::new(session);
        self.pending_uia_operations.clear();
        self.pending_device_cleanup = None;
        self.session = Some(session.clone());
        self.set_secure_backup_send_admitted(false);
        self.session_key_id = Some(key_id);
        self.sliding_sync_positive_evidence = persistable.sliding_sync_positive_evidence();
        self.provisional_persistable = Some(persistable);
        self.session_promoted = false;
        self.send_actions(vec![action]).await;
        self.start_provisional_runtime(session).await;
    }

    async fn start_provisional_runtime(&mut self, session: Arc<MatrixClientSession>) {
        self.trust_generation = self.trust_generation.wrapping_add(1);
        let generation = self.trust_generation;
        let trust_read_started_at = Instant::now();
        #[cfg(any(test, feature = "test-hooks"))]
        let (observation, synthetic_trust_observation) = {
            let override_observation = self
                .trust_observation_override
                .lock()
                .expect("trust observation override lock")
                .take();
            let synthetic = override_observation.is_some();
            (
                override_observation.unwrap_or_else(|| session.observe_current_device_trust()),
                synthetic,
            )
        };
        #[cfg(not(any(test, feature = "test-hooks")))]
        let observation = session.observe_current_device_trust();
        let current_trust = observation.current;
        #[cfg(any(test, feature = "test-hooks"))]
        {
            self.trust_observation_is_synthetic = synthetic_trust_observation;
        }
        record_verification_admission_event(
            verification_admission_event("trust_read_finished", generation, 0)
                .field(DiagnosticField::token(
                    "trust",
                    current_device_trust_token(current_trust),
                ))
                .field(DiagnosticField::milliseconds(
                    "elapsed_ms",
                    trust_read_started_at.elapsed().as_millis(),
                )),
        );
        let tx = self.self_tx.clone();
        let mut updates = observation.updates;
        self.trust_observer = Some(executor::spawn(async move {
            while let Some(trust) = updates.next().await {
                if tx
                    .send(AccountMessage::CurrentDeviceTrustChanged { generation, trust })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }));
        self.handle_current_device_trust(generation, current_trust)
            .await;
    }

    pub(super) async fn stop_provisional_runtime(&mut self) {
        self.trust_generation = self.trust_generation.wrapping_add(1);
        self.trust_recheck_pending = false;
        self.pending_device_cleanup = None;
        self.cancel_pending_trust_promotion().await;
        if let Some(task) = self.trust_observer.take() {
            task.abort();
            let _ = task.await;
            self.record_lifecycle_probe("trust_observer_terminated");
        }
        if let Some(task) = self.trust_recheck_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(owned) = self.verification_method_discovery_task.take() {
            owned.task.abort();
            let _ = owned.task.await;
            record_verification_method_discovery_event(verification_method_discovery_event(
                "cancelled",
                owned.generation,
                owned.serial,
            ));
        }
        self.verification_method_discovery_failed = false;
        self.stop_provisional_encryption_sync().await;
        #[cfg(any(test, feature = "test-hooks"))]
        {
            self.trust_observation_is_synthetic = false;
        }
    }

    pub(super) fn record_lifecycle_probe(&self, token: &'static str) {
        #[cfg(test)]
        if let Some(probe) = &self.lifecycle_probe {
            let _ = probe.send(token);
        }
        #[cfg(not(test))]
        let _ = token;
    }

    /// Persist session credentials, mirroring the src-tauri flow: session
    /// JSON, saved-session index entry, last-session pointer — with rollback
    /// on partial failure.
    pub(super) async fn persist_session(
        &self,
        session: &MatrixClientSession,
        key_id: &SessionKeyId,
    ) -> Result<PersistableMatrixSession, CoreFailure> {
        let store = self.store.clone();
        let session = session.clone();
        let key_id = key_id.clone();
        let sliding_sync_positive_evidence = self.sliding_sync_positive_evidence.clone();
        executor::spawn_blocking(move || {
            let backend = store.credential_backend();
            let mut persistable = session
                .persistable_session()
                .map_err(|_| CoreFailure::StoreUnavailable)?;
            if let Some(evidence) = sliding_sync_positive_evidence {
                persistable = persistable.with_sliding_sync_positive_evidence(evidence);
            }
            let json = persistable
                .to_json()
                .map_err(|_| CoreFailure::StoreUnavailable)?;
            let stored = StoredMatrixSession::new(json);
            backend
                .save_matrix_session(&key_id, &stored)
                .map_err(|_| CoreFailure::StoreUnavailable)?;
            if backend.remember_saved_session(&key_id).is_err() {
                let _ = backend.delete_matrix_session(&key_id);
                return Err(CoreFailure::StoreUnavailable);
            }
            if backend.save_last_session(&key_id).is_err() {
                let _ = backend.delete_matrix_session(&key_id);
                let _ = backend.forget_saved_session(&key_id);
                return Err(CoreFailure::StoreUnavailable);
            }
            Ok(persistable)
        })
        .await
        .unwrap_or(Err(CoreFailure::StoreUnavailable))
    }

    /// Restore a persisted session into the per-account encrypted store
    /// (fail-closed: any store init failure is `LocalEncryptionUnavailable`).
    /// The store config includes the search index so the SDK initializes it
    /// alongside the SQLite store, and event-cache subscription is attempted
    /// before the restored session is returned to any sync/timeline caller.
    /// The encrypted-store diagnostic flag is derived from the keyed store
    /// invariant exposed by `MatrixClientStoreConfig`.
    pub(super) async fn restore_into_store(
        &self,
        persistable: &PersistableMatrixSession,
        key_id: &SessionKeyId,
    ) -> Result<MatrixClientSession, CoreFailure> {
        let store_config = self.store.account_store_config(key_id)?;
        record_restore_store_event(
            restore_store_event("store_config_ready", None)
                .field(DiagnosticField::boolean(
                    "store_dir_exists",
                    store_config.store_config.path().exists(),
                ))
                .field(DiagnosticField::boolean(
                    "cache_dir_exists",
                    store_config
                        .store_config
                        .cache_path()
                        .is_some_and(|path| path.exists()),
                ))
                .field(DiagnosticField::boolean(
                    "encrypted_store",
                    store_config.store_config.encrypted_at_rest_configured(),
                )),
        );
        // Derive the search index configuration. Fail-closed: if the
        // credential store is unreachable, deny the restore (LocalEncryptionUnavailable).
        let search_config = self.store.account_search_index_config(key_id)?;
        record_restore_store_event(restore_store_event("search_config_ready", None).field(
            DiagnosticField::boolean(
                "search_dir_exists",
                search_config.search_index_config.path().exists(),
            ),
        ));
        let encrypted_store = store_config.store_config.encrypted_at_rest_configured();
        let store_config_with_search = store_config
            .store_config
            .with_search_index_store(search_config.search_index_config);
        let restore_started = Instant::now();
        record_restore_store_event(restore_store_event("sdk_restore_begin", None));
        let session = match koushi_sdk::restore_session_with_store(
            persistable,
            Some(&store_config_with_search),
        )
        .await
        {
            Ok(session) => {
                record_restore_store_event(restore_store_event("sdk_restore_ok", None).field(
                    DiagnosticField::milliseconds(
                        "elapsed_ms",
                        restore_started.elapsed().as_millis(),
                    ),
                ));
                session
            }
            Err(_) => {
                record_restore_store_event(restore_store_event("sdk_restore_failed", None).field(
                    DiagnosticField::milliseconds(
                        "elapsed_ms",
                        restore_started.elapsed().as_millis(),
                    ),
                ));
                return Err(CoreFailure::LocalEncryptionUnavailable);
            }
        };
        let event_cache_result = koushi_sdk::enable_event_cache(&session).await;
        self.emit_event_cache_status(encrypted_store, &event_cache_result);
        // Baseline receive-side room-key diagnostics for this account runtime
        // (#476). The observer is installed by `restore_session_with_store`
        // before sync can deliver to-device events; reset the per-runtime
        // late-decryption counters and record the initial summary.
        crate::room_key_receive::reset_late_decryption_counters();
        let diagnostics = koushi_sdk::room_key_receive_diagnostics(&session).await;
        crate::room_key_receive::record_room_key_receive_summary(
            &diagnostics,
            crate::room_key_receive::RECEIVE_SUMMARY_TRIGGER_RESTORE,
        );
        Ok(session)
    }

    /// Roll back a failed login bootstrap: best-effort server logout of the
    /// storeless client (so no orphan device stays registered), drop it inside
    /// the runtime context, and — if credentials were already persisted —
    /// remove them again so a later restore does not pick up a session whose
    /// token was just invalidated.
    pub(super) async fn abort_login(
        &self,
        login_session: MatrixClientSession,
        key_id: &SessionKeyId,
        credentials_persisted: bool,
        server_logout: bool,
    ) {
        if server_logout {
            let _ = koushi_sdk::logout(&login_session).await;
        }
        drop(login_session);
        if credentials_persisted {
            self.clear_account_persistence(key_id).await;
        }
    }

    /// When a normal sign-out preserved a keyed store for this Matrix user,
    /// prefer logging back into that same device id. This preserves the local
    /// SDK store and avoids turning every sign-out/sign-in into a cold device.
    ///
    /// The optimization is deliberately fail-open: if the homeserver rejects an
    /// existing-device login, continue with the already-successful fresh login
    /// instead of converting a successful password login into a user-visible
    /// failure.
    async fn prefer_saved_device_for_password_login(
        &self,
        fresh_login_session: MatrixClientSession,
        fresh_info: SessionInfo,
        fresh_key_id: SessionKeyId,
        account_key: &AccountKey,
        password: &koushi_state::AuthSecret,
    ) -> (MatrixClientSession, SessionInfo, SessionKeyId) {
        let saved_key_id = match self.lookup_session_key_id(account_key).await {
            Ok(Some(key_id)) => key_id,
            Ok(None) | Err(()) => {
                return (fresh_login_session, fresh_info, fresh_key_id);
            }
        };

        if saved_key_id == fresh_key_id {
            return (fresh_login_session, fresh_info, fresh_key_id);
        }

        if saved_key_id.homeserver != fresh_key_id.homeserver
            || saved_key_id.user_id != fresh_key_id.user_id
        {
            return (fresh_login_session, fresh_info, fresh_key_id);
        }

        record(
            DiagnosticEvent::new(
                DiagnosticLevel::Debug,
                "core.account",
                "password_login_saved_device_attempted",
            )
            .field(DiagnosticField::boolean("has_saved_device", true)),
        );

        let saved_login_session = match koushi_sdk::login_with_existing_device(
            &fresh_info.homeserver,
            &fresh_info.user_id,
            &saved_key_id.device_id,
            password,
        )
        .await
        {
            Ok(session) => session,
            Err(_) => {
                record(
                    DiagnosticEvent::new(
                        DiagnosticLevel::Warn,
                        "core.account",
                        "password_login_saved_device_failed",
                    )
                    .field(DiagnosticField::boolean("fallback_to_fresh_device", true)),
                );
                return (fresh_login_session, fresh_info, fresh_key_id);
            }
        };

        let saved_info = saved_login_session.info.clone();
        let actual_saved_key_id = session_key_id_from_info(&saved_info);
        if actual_saved_key_id != saved_key_id {
            let _ = koushi_sdk::logout(&saved_login_session).await;
            drop(saved_login_session);
            record(
                DiagnosticEvent::new(
                    DiagnosticLevel::Warn,
                    "core.account",
                    "password_login_saved_device_mismatch",
                )
                .field(DiagnosticField::boolean("fallback_to_fresh_device", true)),
            );
            return (fresh_login_session, fresh_info, fresh_key_id);
        }

        let _ = koushi_sdk::logout(&fresh_login_session).await;
        drop(fresh_login_session);
        record(
            DiagnosticEvent::new(
                DiagnosticLevel::Debug,
                "core.account",
                "password_login_saved_device_reused",
            )
            .field(DiagnosticField::boolean("fallback_to_fresh_device", false)),
        );
        (saved_login_session, saved_info, actual_saved_key_id)
    }

    /// Remove all persisted material for one account: session JSON, saved
    /// session index entry, last-session pointer (only if it points at this
    /// account), unlock secret, and store/cache directories.
    pub(super) async fn clear_account_persistence(&self, key_id: &SessionKeyId) -> bool {
        let store = self.store.clone();
        let key_id = key_id.clone();
        executor::spawn_blocking(move || {
            let backend = store.credential_backend();
            let mut cleared = backend.delete_matrix_session(&key_id).is_ok();
            cleared &= backend.forget_saved_session(&key_id).is_ok();
            match backend.load_last_session() {
                Ok(Some(last)) if last == key_id => {
                    cleared &= backend.delete_last_session().is_ok();
                }
                Ok(_) => {}
                Err(_) => {
                    cleared = false;
                    let _ = backend.delete_last_session();
                }
            }
            cleared &= store.delete_account_credentials(&key_id).is_ok();
            cleared
        })
        .await
        .unwrap_or(false)
    }

    /// Leave the saved session, unlock secret, and keyed store intact, but
    /// remove the automatic startup pointer when it targets the just-signed-out
    /// device. Normal sign-out should not make the next login a cold start, but
    /// it also must not auto-restore a token that server logout just invalidated.
    async fn forget_last_session_pointer_if_matches(&self, key_id: &SessionKeyId) {
        let store = self.store.clone();
        let key_id = key_id.clone();
        let _ = executor::spawn_blocking(move || {
            let backend = store.credential_backend();
            match backend.load_last_session() {
                Ok(Some(last)) if last == key_id => {
                    let _ = backend.delete_last_session();
                }
                Ok(_) => {}
                Err(_) => {
                    let _ = backend.delete_last_session();
                }
            }
        })
        .await;
    }

    /// Find the stored `SessionKeyId` for an account key (the user's Matrix
    /// ID). Checks the last-session pointer first, then the saved-session
    /// index. `Ok(None)` = no stored session; `Err(())` = store unreachable.
    async fn lookup_session_key_id(
        &self,
        account_key: &AccountKey,
    ) -> Result<Option<SessionKeyId>, ()> {
        let store = self.store.clone();
        let account_key = account_key.clone();
        executor::spawn_blocking(move || {
            let backend = store.credential_backend();
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
        })
        .await
        .unwrap_or(Err(()))
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use koushi_key::{SessionKeyId, StoredMatrixSession};
    use koushi_sdk::PersistableMatrixSession;
    use koushi_state::{
        AppAction, LoginAttemptId, LoginRequest, SlidingSyncAdmission, SlidingSyncAdmissionSource,
        SlidingSyncCapabilityResult, SlidingSyncPositiveEvidence,
    };

    use tokio::sync::{broadcast, mpsc, oneshot};

    use super::{
        SESSION_NOT_FOUND_FAILURE, ServerLogoutOutcome, SessionInvalidationReason,
        run_session_change_observation, wait_for_server_logout_best_effort,
    };
    use crate::account::actor::{AccountActor, AccountActorHandle, AccountMessage};
    use crate::account::test_support::{
        acknowledge_next_verified_projection, assert_no_logout_finished, configure_verified_trust,
        consume_initial_unknown_trust_projection, inspect_session_runtime, inspect_sync_owners,
        recv_account_action_with_sliding_sync_effects, recv_probe_with_sliding_sync_effects,
        shutdown_and_ack, spawn_actor_with_dirs, spawn_named_quarantine_password_server,
        spawn_named_quarantine_password_server_with_controls, spawn_quarantine_password_server,
        test_request_id,
    };
    use crate::command::AccountCommand;
    use crate::composer_draft_lifecycle::ComposerDraftLeaseRegistry;
    use crate::event::{AccountEvent, CoreEvent};
    use crate::executor;

    use crate::failure::CoreFailure;
    use crate::ids::{AccountKey, RequestId, RuntimeConnectionId};
    use crate::link_preview::LinkPreviewContext;

    use crate::store::CredentialStoreBackend;
    use crate::store::{StoreActor, session_key_id_from_info};

    use tempfile::tempdir;

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
        let handle = AccountActor::spawn(
            store,
            action_tx,
            event_tx,
            LinkPreviewContext::default(),
            Arc::new(ComposerDraftLeaseRegistry::new()),
        );

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

    #[tokio::test]
    async fn logout_cleanup_is_bounded_and_preserves_account_persistence() {
        let homeserver = spawn_quarantine_password_server();
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let baseline_files = recursive_file_count(data_dir.path());
        let (handle, mut action_rx, mut event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
        configure_verified_trust(&handle).await;
        handle
            .send(AccountMessage::AttachLifecycleProbe { probe_tx })
            .await;
        handle
            .send(AccountMessage::ConfigureCloseStoreResults {
                results: vec![false, true],
            })
            .await;
        let login_request_id = test_request_id();
        handle
            .send(AccountMessage::Command(AccountCommand::LoginPassword {
                request_id: login_request_id,
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
        let files_before_logout = recursive_file_count(data_dir.path());
        assert!(files_before_logout > baseline_files);

        let request_id = RequestId {
            connection_id: crate::ids::RuntimeConnectionId(1),
            sequence: 2,
        };
        handle
            .send(AccountMessage::Command(AccountCommand::Logout {
                request_id,
            }))
            .await;
        recv_probe_with_sliding_sync_effects(
            &handle,
            &mut action_rx,
            &mut probe_rx,
            "session_store_close_retrying",
        )
        .await;
        assert_eq!(recursive_file_count(data_dir.path()), files_before_logout);
        assert_no_logout_finished(&mut action_rx);

        handle
            .send(AccountMessage::RetrySessionTeardown { generation: 1 })
            .await;
        assert_eq!(probe_rx.recv().await, Some("session_store_closed"));
        assert_eq!(probe_rx.recv().await, Some("session_persistence_preserved"));
        while !matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::LogoutFinished])
        ) {}
        let backend = CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(
            cred_dir.path(),
        ));
        assert!(
            backend
                .load_last_session()
                .expect("last pointer after logout")
                .is_none()
        );
        loop {
            if let CoreEvent::Account(AccountEvent::LoggedOut {
                request_id: terminal,
                ..
            }) = event_rx.recv().await.expect("logout event")
            {
                assert_eq!(terminal, request_id);
                break;
            }
        }
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[test]
    fn logout_teardown_preserves_persistence_and_only_forgets_startup_pointer() {
        let logout_continuation = crate::account::test_source::item_body(
            include_str!("session_lifecycle.rs"),
            "match pending.continuation",
        );

        assert!(
            logout_continuation.contains("preserve_persistence")
                && logout_continuation.contains("forget_last_session_pointer_if_matches(key_id)"),
            "normal sign-out should preserve the keyed store and saved-session index"
        );
        assert!(
            logout_continuation.contains("clear_account_persistence(key_id)"),
            "non-preserving teardown paths such as provisional rejection must still delete the local database"
        );
        assert!(
            logout_continuation.contains("session_persistence_preserved"),
            "logout diagnostics should make the preservation explicit"
        );
        assert!(
            logout_continuation.contains("session_persistence_deleted"),
            "non-preserving teardown diagnostics should remain explicit"
        );
    }

    #[tokio::test]
    async fn password_login_names_an_unnamed_device_with_the_platform_default() {
        let (homeserver, rename_bodies) = spawn_device_naming_password_server();
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, mut event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
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

        // The cosmetic device rename ran exactly once with the platform
        // default. Exact JSON equality proves the body is only the display
        // name — no username, device id, token, or other private identifier.
        let bodies = rename_bodies.lock().expect("rename record");
        assert_eq!(bodies.len(), 1, "device rename should run once");
        let parsed: serde_json::Value =
            serde_json::from_str(&bodies[0]).expect("rename body should be JSON");
        assert_eq!(
            parsed,
            serde_json::json!({ "display_name": "Koushi on Linux" })
        );
        drop(bodies);
        shutdown_and_ack(&handle).await;
        while let Ok(event) = event_rx.try_recv() {
            assert!(!matches!(
                event,
                CoreEvent::Account(AccountEvent::LoggedOut { .. })
            ));
        }
    }

    #[tokio::test]
    async fn password_login_preserves_a_customized_device_name() {
        let (homeserver, rename_bodies) = spawn_device_naming_password_server();
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, mut event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        handle
            .send(AccountMessage::Command(AccountCommand::LoginPassword {
                request_id: test_request_id(),
                request: LoginRequest {
                    homeserver,
                    username: "fixture-user".to_owned(),
                    password: koushi_state::AuthSecret::new("synthetic-password"),
                    device_display_name: Some("My Laptop".to_owned()),
                },
                platform: koushi_state::DisplayPlatform::Macos,
            }))
            .await;
        while !matches!(
            recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx)
                .await
                .as_slice(),
            [AppAction::LoginSucceeded { .. }]
        ) {}

        let bodies = rename_bodies.lock().expect("rename record");
        assert_eq!(
            bodies.len(),
            0,
            "a customized device name must not be rewritten"
        );
        drop(bodies);
        shutdown_and_ack(&handle).await;
        while let Ok(event) = event_rx.try_recv() {
            assert!(!matches!(
                event,
                CoreEvent::Account(AccountEvent::LoggedOut { .. })
            ));
        }
    }

    #[test]
    fn restore_into_store_emits_event_cache_status_without_failing_restore() {
        let body = crate::account::test_source::item_body(
            include_str!("session_lifecycle.rs"),
            "async fn restore_into_store",
        );
        let compact: String = body.chars().filter(|c| !c.is_whitespace()).collect();
        let helper = crate::account::test_source::item_body(
            include_str!("actor.rs"),
            "fn emit_event_cache_status(",
        );
        let helper_compact: String = helper.chars().filter(|c| !c.is_whitespace()).collect();
        let restore = compact
            .find("koushi_sdk::restore_session_with_store")
            .expect("restore_session_with_store call");
        let store_config = compact
            .find("letstore_config=self.store.account_store_config(key_id)?;")
            .expect("keyed store configuration");
        let encrypted_store = compact
            .find("letencrypted_store=store_config.store_config.encrypted_at_rest_configured();")
            .expect("derived encrypted-store flag");
        let enable = compact
            .find("koushi_sdk::enable_event_cache(&session).await")
            .expect("enable_event_cache call");
        let emit = compact
            .find("self.emit_event_cache_status(encrypted_store,&event_cache_result);")
            .expect("event cache diagnostic emission");
        // rfind: the restore body also contains an `Ok(session) =>` match arm
        // (session-preservation restore path), so the tail return is the LAST
        // occurrence, not the first.
        let return_ok = compact.rfind("Ok(session)").expect("return statement");

        assert!(store_config < encrypted_store);
        assert!(restore < enable);
        assert!(encrypted_store < emit);
        assert!(enable < return_ok);
        assert!(
            helper_compact.contains("EventCacheSubscribeStatus::Enabled,None"),
            "enabled diagnostics should carry an explicit subscribe status and no failure reason"
        );
        assert!(
            helper_compact.contains("EventCacheSubscribeStatus::AlreadyEnabled,None"),
            "already-enabled diagnostics should carry an explicit subscribe status and no failure reason"
        );
        assert!(
            helper_compact.contains(
                "EventCacheSubscribeStatus::SubscribeFailed,Some(EventCacheFailureReasonClass::SubscribeFailed),",
            ),
            "failure diagnostics should carry an explicit subscribe status and a private-data-free reason"
        );
        assert!(
            compact.contains(
                "letencrypted_store=store_config.store_config.encrypted_at_rest_configured();"
            ),
            "restore_into_store must derive the encrypted-store diagnostic from the keyed store invariant"
        );
        assert!(
            compact.contains("self.emit_event_cache_status(encrypted_store,&event_cache_result);"),
            "restore_into_store must pass the derived encrypted-store flag into the diagnostic"
        );
        assert_eq!(
            compact
                .matches("self.emit_event_cache_status(encrypted_store,&event_cache_result);")
                .count(),
            1,
            "restore_into_store should call the diagnostic helper exactly once"
        );
        assert!(
            !compact.contains("enable_event_cache(&session).await.map_err"),
            "event-cache subscription failure must not be mapped into restore failure"
        );
        assert!(
            !compact.contains("enable_event_cache(&session).await?"),
            "event-cache subscription failure must not use ? to fail the restore path"
        );
        assert!(
            !helper_compact.contains("encrypted_store:true"),
            "the event-cache diagnostic helper must not hardcode the encrypted-store flag"
        );
        assert!(
            !compact.contains("cache_path().is_some()"),
            "restore_into_store must not use cache_path presence as an encryption invariant"
        );
    }

    #[test]
    fn changing_homeserver_does_not_logout_pending_login_on_the_old_server() {
        let logout = crate::account::test_source::item_body(
            include_str!("session_lifecycle.rs"),
            "async fn perform_logout",
        );
        let abort = crate::account::test_source::item_body(
            include_str!("session_lifecycle.rs"),
            "async fn abort_login",
        );

        assert!(logout.contains("self.abort_login(login_session, &key_id, false, server_logout)"));
        assert!(abort.contains("if server_logout") && abort.contains("koushi_sdk::logout"));
    }

    #[test]
    fn authentication_completion_installs_quarantine_before_ready_side_effects() {
        let password = crate::account::test_source::item_body(
            include_str!("session_lifecycle.rs"),
            "async fn handle_login_password",
        );
        let before_success = password
            .split("AppAction::LoginSucceeded")
            .next()
            .expect("password pre-success body");
        assert!(!before_success.contains("persist_session("));
        assert!(!before_success.contains("spawn_sync_actor("));
        assert!(before_success.contains("begin_sliding_sync_capability_discovery"));
        assert!(!before_success.contains("install_provisional_session"));

        let restore = crate::account::test_source::item_body(
            include_str!("session_lifecycle.rs"),
            "async fn restore_account",
        );
        let before_restore_success = restore
            .split("AppAction::RestoreSessionSucceeded")
            .next()
            .expect("restore pre-success body");
        assert!(!before_restore_success.contains("spawn_sync_actor("));
        assert!(before_restore_success.contains("begin_sliding_sync_capability_discovery"));
        assert!(!before_restore_success.contains("install_provisional_session"));
        let continuation = crate::account::test_source::item_body(
            include_str!("sliding_sync.rs"),
            "async fn continue_sliding_sync_admission",
        );
        assert!(continuation.contains("install_provisional_session"));
        let discovery_completion = crate::account::test_source::item_body(
            include_str!("sliding_sync.rs"),
            "async fn finish_sliding_sync_capability_discovery",
        );
        assert!(
            !discovery_completion.contains("self.continue_sliding_sync_admission("),
            "session installation must wait for the reducer-produced continuation effect"
        );
    }

    #[tokio::test]
    async fn password_quarantine_persists_no_credentials_and_restart_is_signed_out() {
        let homeserver = spawn_quarantine_password_server();
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, _event_rx) =
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
                        device_display_name: Some("Quarantine Test".to_owned()),
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
                result: SlidingSyncCapabilityResult::Supported { .. },
                ..
            }]
        ));
        let actions = action_rx.recv().await.expect("provisional login action");
        assert!(matches!(
            actions.as_slice(),
            [AppAction::LoginSucceeded { .. }]
        ));

        let backend = CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(
            cred_dir.path(),
        ));
        assert!(
            backend
                .load_last_session()
                .expect("last pointer read")
                .is_none()
        );
        assert!(
            backend
                .load_saved_sessions()
                .expect("saved index read")
                .sessions()
                .is_empty()
        );

        let _ = handle.send(AccountMessage::Shutdown).await;
        let (restarted, mut restarted_actions, _events) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        assert!(
            restarted
                .send(AccountMessage::Command(
                    AccountCommand::RestoreLastSession { request_id }
                ))
                .await
        );
        assert!(matches!(
            restarted_actions.recv().await.as_deref(),
            Some([AppAction::RestoreSessionNotFound])
        ));
        let _ = restarted.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn oidc_completion_installs_only_a_provisional_quarantined_session() {
        let homeserver = spawn_quarantine_password_server();
        let login_session = koushi_sdk::login_with_password_with_store(
            &LoginRequest {
                homeserver: homeserver.clone(),
                username: "fixture-user".to_owned(),
                password: koushi_state::AuthSecret::new("synthetic-password"),
                device_display_name: Some("OIDC Quarantine Test".to_owned()),
            },
            None,
        )
        .await
        .expect("fixture login");

        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, mut event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let (_trust_tx, trust_rx) = mpsc::unbounded_channel();
        let updates = futures_util::stream::unfold(trust_rx, |mut rx| async move {
            rx.recv().await.map(|trust| (trust, rx))
        });
        assert!(
            handle
                .send(AccountMessage::ConfigureTrustObservation {
                    observation: koushi_sdk::CurrentDeviceTrustObservation {
                        current: koushi_state::CurrentDeviceTrustState::Unknown,
                        updates: Box::pin(updates),
                    },
                })
                .await
        );
        let start_request_id = test_request_id();
        assert!(
            handle
                .send(AccountMessage::ConfigureOidcCompletion {
                    start_request_id,
                    homeserver: homeserver.clone(),
                    session: login_session,
                })
                .await
        );
        let completion_request_id = RequestId {
            connection_id: crate::ids::RuntimeConnectionId(41),
            sequence: 7,
        };
        assert!(
            handle
                .send(AccountMessage::Command(AccountCommand::CompleteOidcLogin {
                    request_id: completion_request_id,
                    callback_url: "http://127.0.0.1/callback?code=fixture&state=fixture".to_owned(),
                    platform: koushi_state::DisplayPlatform::Linux,
                },))
                .await
        );
        assert!(matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::AuthenticationStarted {
                attempt_id,
                homeserver: projected_homeserver,
            }]) if *attempt_id == LoginAttemptId::new(41, 7)
                && projected_homeserver == &homeserver
        ));
        assert!(matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::SlidingSyncCapabilityCheckStarted {
                admission: SlidingSyncAdmission::NewLogin { attempt_id },
                ..
            }]) if *attempt_id == LoginAttemptId::new(41, 7)
        ));
        assert!(matches!(
            recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx)
                .await
                .as_slice(),
            [AppAction::SlidingSyncCapabilityCheckCompleted {
                result: SlidingSyncCapabilityResult::Supported { .. },
                ..
            }]
        ));
        assert!(matches!(
            recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx)
                .await
                .as_slice(),
            [AppAction::LoginSucceeded { attempt_id, .. }]
                if *attempt_id == LoginAttemptId::new(41, 7)
        ));
        assert_eq!(
            inspect_session_runtime(&handle).await,
            (true, false, false, true)
        );

        let backend = CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(
            cred_dir.path(),
        ));
        assert!(backend.load_last_session().expect("pointer read").is_none());
        assert!(
            backend
                .load_saved_sessions()
                .expect("index read")
                .sessions()
                .is_empty()
        );
        assert!(
            executor::timeout(Duration::from_millis(100), async {
                loop {
                    match event_rx.recv().await.expect("event stream") {
                        CoreEvent::Account(AccountEvent::LoggedIn { .. }) | CoreEvent::Sync(_) => {
                            return;
                        }
                        _ => {}
                    }
                }
            })
            .await
            .is_err(),
            "OIDC completion escaped quarantine before Verified"
        );
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn verified_warm_restore_skips_restricted_and_full_state_preparation() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
            .records
            .len();
        let homeserver = spawn_quarantine_password_server();
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, _event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
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
        while !matches!(
            recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx)
                .await
                .as_slice(),
            [AppAction::LoginSucceeded { .. }]
        ) {}

        assert_eq!(
            inspect_sync_owners(&handle).await,
            (false, false, false),
            "authoritative Verified restore must not start restricted or promotion sync"
        );

        acknowledge_next_verified_projection(&handle, &mut action_rx).await;
        assert_eq!(
            inspect_sync_owners(&handle).await,
            (false, false, true),
            "normal sync must be the sole owner after Ready projection acknowledgement"
        );
        let snapshot = koushi_diagnostics::test_support::detail_snapshot();
        let stages = snapshot.records[diagnostic_start..]
            .iter()
            .filter(|record| record.event.source == "core.verification_admission")
            .map(|record| record.event.stage)
            .collect::<Vec<_>>();
        let mut remaining = stages.as_slice();
        for expected in [
            "provisional_encryption_sync_skipped",
            "ready_projection_dispatched",
            "normal_sync_started",
        ] {
            let index = remaining
                .iter()
                .position(|stage| *stage == expected)
                .unwrap_or_else(|| {
                    panic!("missing ordered admission stage {expected}: {stages:?}")
                });
            remaining = &remaining[index + 1..];
        }
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn verified_offline_warm_restore_reaches_ready_without_network_catch_up() {
        let (homeserver, offline, sliding_sync_supported) =
            spawn_controllable_quarantine_password_server();
        let login = koushi_sdk::login_with_password_with_store(
            &LoginRequest {
                homeserver,
                username: "fixture-user".to_owned(),
                password: koushi_state::AuthSecret::new("synthetic-password"),
                device_display_name: Some("Offline Restore Test".to_owned()),
            },
            None,
        )
        .await
        .expect("fixture login");
        let key_id = session_key_id_from_info(&login.info);
        let stored = StoredMatrixSession::new(
            login
                .persistable_session()
                .expect("persistable")
                .with_sliding_sync_positive_evidence(SlidingSyncPositiveEvidence {
                    observed_at_ms: 11,
                })
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
        configure_verified_trust(&handle).await;
        offline.store(true, std::sync::atomic::Ordering::SeqCst);
        handle
            .send(AccountMessage::Command(
                AccountCommand::RestoreLastSession {
                    request_id: test_request_id(),
                },
            ))
            .await;
        assert!(matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::SlidingSyncCapabilityCheckStarted {
                admission: SlidingSyncAdmission::StoredSessionRestore { .. },
                positive_evidence: Some(_),
                ..
            }])
        ));
        let (offline_epoch, offline_request_id) = match action_rx.recv().await.as_deref() {
            Some(
                [
                    AppAction::SlidingSyncCapabilityCheckCompleted {
                        account_epoch,
                        request_id,
                        result: SlidingSyncCapabilityResult::Unreachable,
                    },
                ],
            ) => (*account_epoch, *request_id),
            other => panic!("expected unreachable capability result, got {other:?}"),
        };
        handle
            .send(AccountMessage::ContinueSlidingSyncAdmission {
                account_epoch: offline_epoch,
                request_id: offline_request_id,
                source: SlidingSyncAdmissionSource::PositiveCache,
            })
            .await;
        handle
            .send(AccountMessage::ScheduleSlidingSyncCapabilityRevalidation {
                account_epoch: offline_epoch,
            })
            .await;
        assert!(matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::RestoreSessionSucceeded(_)])
        ));

        offline.store(false, std::sync::atomic::Ordering::SeqCst);
        sliding_sync_supported.store(false, std::sync::atomic::Ordering::SeqCst);
        executor::timeout(
            Duration::from_secs(1),
            acknowledge_next_verified_projection(&handle, &mut action_rx),
        )
        .await
        .expect("offline verified restore must reach Ready without network catch-up");
        let (account_epoch, blocked_request_id) = match action_rx.recv().await.as_deref() {
            Some(
                [
                    AppAction::SlidingSyncCapabilityRevalidationStarted {
                        account_epoch,
                        request_id,
                    },
                ],
            ) => (*account_epoch, *request_id),
            other => panic!("expected revalidation start, got {other:?}"),
        };
        let revalidation_result = loop {
            let actions = action_rx.recv().await.expect("revalidation action");
            if let [AppAction::SlidingSyncCapabilityRevalidationCompleted { result, .. }] =
                actions.as_slice()
            {
                break result.clone();
            }
        };
        assert_eq!(
            revalidation_result,
            SlidingSyncCapabilityResult::Unsupported
        );
        assert_eq!(
            inspect_sync_owners(&handle).await,
            (false, false, true),
            "actor must await the reducer-accepted settlement effect"
        );
        handle
            .send(AccountMessage::SettleSlidingSyncCapabilityRevalidation {
                account_epoch,
                request_id: blocked_request_id,
                result: SlidingSyncCapabilityResult::Unsupported,
            })
            .await;
        assert_eq!(inspect_sync_owners(&handle).await, (false, false, false));
        handle
            .send(AccountMessage::Command(
                AccountCommand::RetrySlidingSyncCapability {
                    request_id: RequestId {
                        connection_id: RuntimeConnectionId(1),
                        sequence: 2,
                    },
                },
            ))
            .await;
        loop {
            let actions =
                recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx).await;
            if matches!(
                actions.as_slice(),
                [AppAction::SlidingSyncCapabilityRetryAccepted {
                    account_epoch: accepted_epoch,
                    blocked_request_id: accepted_request_id,
                    ..
                }] if *accepted_epoch == account_epoch && *accepted_request_id == blocked_request_id
            ) {
                break;
            }
        }
        loop {
            let actions = action_rx.recv().await.expect("retry start action");
            if matches!(
                actions.as_slice(),
                [AppAction::SlidingSyncCapabilityCheckStarted {
                    admission: SlidingSyncAdmission::StoredSessionRestore { .. },
                    ..
                }]
            ) {
                break;
            }
        }
        assert_eq!(inspect_sync_owners(&handle).await, (false, false, false));
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn provisional_rejection_deletes_keyed_store_before_signed_out_ack() {
        let homeserver = spawn_quarantine_password_server();
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, _event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
        assert!(
            handle
                .send(AccountMessage::AttachLifecycleProbe { probe_tx })
                .await
        );
        let baseline_files = recursive_file_count(data_dir.path());
        let request_id = test_request_id();
        assert!(
            handle
                .send(AccountMessage::Command(AccountCommand::LoginPassword {
                    request_id,
                    request: LoginRequest {
                        homeserver,
                        username: "fixture-user".to_owned(),
                        password: koushi_state::AuthSecret::new("synthetic-password"),
                        device_display_name: Some("Quarantine Test".to_owned()),
                    },
                    platform: koushi_state::DisplayPlatform::Linux,
                }))
                .await
        );
        loop {
            let actions =
                recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx).await;
            if matches!(actions.as_slice(), [AppAction::LoginSucceeded { .. }]) {
                break;
            }
        }
        assert!(
            recursive_file_count(data_dir.path()) > baseline_files,
            "keyed store was not created"
        );

        assert!(
            handle
                .send(AccountMessage::RejectProvisionalSession { request_id })
                .await
        );
        loop {
            let actions = action_rx.recv().await.expect("rejection action");
            if matches!(actions.as_slice(), [AppAction::LogoutFinished]) {
                assert_eq!(
                    probe_rx.try_recv(),
                    Ok("trust_observer_terminated"),
                    "LogoutFinished preceded trust-observer termination"
                );
                assert_eq!(
                    probe_rx.try_recv(),
                    Ok("provisional_encryption_sync_terminated"),
                    "LogoutFinished preceded restricted-sync termination"
                );
                assert_eq!(
                    recursive_file_count(data_dir.path()),
                    baseline_files,
                    "SignedOut ack preceded keyed-store deletion"
                );
                break;
            }
        }
        let backend = CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(
            cred_dir.path(),
        ));
        assert!(backend.load_last_session().expect("pointer read").is_none());
        assert!(
            backend
                .load_saved_sessions()
                .expect("index read")
                .sessions()
                .is_empty()
        );
        shutdown_and_ack(&handle).await;
        let (restarted, mut restarted_actions, _restarted_events) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let restore_id = RequestId {
            connection_id: RuntimeConnectionId(19),
            sequence: 1,
        };
        assert!(
            restarted
                .send(AccountMessage::Command(
                    AccountCommand::RestoreLastSession {
                        request_id: restore_id,
                    },
                ))
                .await
        );
        assert!(matches!(
            restarted_actions.recv().await.as_deref(),
            Some([AppAction::RestoreSessionNotFound])
        ));
        shutdown_and_ack(&restarted).await;
    }

    #[tokio::test]
    async fn teardown_close_failure_retries_without_early_ack_and_preserves_request_correlation() {
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
            .send(AccountMessage::ConfigureCloseStoreResults {
                results: vec![false, true],
            })
            .await;
        let original = test_request_id();
        handle
            .send(AccountMessage::Command(AccountCommand::LoginPassword {
                request_id: original,
                request: LoginRequest {
                    homeserver,
                    username: "fixture-user".to_owned(),
                    password: koushi_state::AuthSecret::new("synthetic-password"),
                    device_display_name: Some("Teardown Retry Test".to_owned()),
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
            .send(AccountMessage::RejectProvisionalSession {
                request_id: original,
            })
            .await;
        while probe_rx.recv().await != Some("session_store_close_retrying") {}
        assert_no_logout_finished(&mut action_rx);

        let later = RequestId {
            connection_id: crate::ids::RuntimeConnectionId(77),
            sequence: 2,
        };
        handle
            .send(AccountMessage::RejectProvisionalSession { request_id: later })
            .await;
        loop {
            if let CoreEvent::OperationFailed {
                request_id,
                failure,
            } = event_rx.recv().await.expect("failure event")
                && request_id == later
            {
                assert_eq!(failure, CoreFailure::SessionRequired);
                break;
            }
        }
        handle
            .send(AccountMessage::RetrySessionTeardown { generation: 999 })
            .await;
        assert_no_logout_finished(&mut action_rx);
        handle
            .send(AccountMessage::RetrySessionTeardown { generation: 1 })
            .await;
        assert_eq!(probe_rx.recv().await, Some("session_store_closed"));
        assert_eq!(probe_rx.recv().await, Some("session_persistence_deleted"));
        while !matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::LogoutFinished])
        ) {}
        loop {
            if let CoreEvent::Account(AccountEvent::LoggedOut { request_id, .. }) =
                event_rx.recv().await.expect("logout event")
            {
                assert_eq!(request_id, original);
                break;
            }
        }
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn permanent_close_failures_never_ack_before_a_success_barrier() {
        let homeserver = spawn_quarantine_password_server();
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, _event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
        handle
            .send(AccountMessage::AttachLifecycleProbe { probe_tx })
            .await;
        handle
            .send(AccountMessage::ConfigureCloseStoreResults {
                results: vec![false; 16],
            })
            .await;
        let request_id = test_request_id();
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
            .await;
        while !matches!(
            recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx)
                .await
                .as_slice(),
            [AppAction::LoginSucceeded { .. }]
        ) {}
        handle
            .send(AccountMessage::RejectProvisionalSession { request_id })
            .await;
        for _ in 0..4 {
            while probe_rx.recv().await != Some("session_store_close_retrying") {}
            assert_no_logout_finished(&mut action_rx);
            handle
                .send(AccountMessage::RetrySessionTeardown { generation: 1 })
                .await;
        }
        assert_no_logout_finished(&mut action_rx);
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn replacement_install_waits_for_provisional_tasks_to_terminate() {
        let first_homeserver = spawn_quarantine_password_server();
        let second_homeserver = spawn_quarantine_password_server();
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, _event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
        assert!(
            handle
                .send(AccountMessage::AttachLifecycleProbe { probe_tx })
                .await
        );
        for homeserver in [first_homeserver, second_homeserver] {
            let request_id = test_request_id();
            assert!(
                handle
                    .send(AccountMessage::Command(AccountCommand::LoginPassword {
                        request_id,
                        request: LoginRequest {
                            homeserver,
                            username: "fixture-user".to_owned(),
                            password: koushi_state::AuthSecret::new("synthetic-password"),
                            device_display_name: Some("Replacement Barrier Test".to_owned()),
                        },
                        platform: koushi_state::DisplayPlatform::Linux,
                    }))
                    .await
            );
            loop {
                if matches!(
                    recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx)
                        .await
                        .as_slice(),
                    [AppAction::LoginSucceeded { .. }]
                ) {
                    break;
                }
            }
        }
        assert_eq!(probe_rx.try_recv(), Ok("trust_observer_terminated"));
        assert_eq!(
            probe_rx.try_recv(),
            Ok("provisional_encryption_sync_terminated")
        );
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn replacement_close_failure_holds_incoming_until_generation_retry_succeeds() {
        let first_homeserver = spawn_quarantine_password_server();
        let second_homeserver = spawn_quarantine_password_server();
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, mut event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
        handle
            .send(AccountMessage::AttachLifecycleProbe { probe_tx })
            .await;
        let first_request = test_request_id();
        handle
            .send(AccountMessage::Command(AccountCommand::LoginPassword {
                request_id: first_request,
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
                results: vec![false, true],
            })
            .await;
        let replacement_request = RequestId {
            connection_id: crate::ids::RuntimeConnectionId(2),
            sequence: 2,
        };
        handle
            .send(AccountMessage::Command(AccountCommand::LoginPassword {
                request_id: replacement_request,
                request: LoginRequest {
                    homeserver: second_homeserver.clone(),
                    username: "fixture-user".to_owned(),
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
        assert_no_login_succeeded_for(&mut action_rx, &second_homeserver);
        assert_eq!(
            inspect_session_runtime(&handle).await,
            (false, false, false, false)
        );

        let later = RequestId {
            connection_id: crate::ids::RuntimeConnectionId(3),
            sequence: 3,
        };
        handle
            .send(AccountMessage::Command(AccountCommand::LoginPassword {
                request_id: later,
                request: LoginRequest {
                    homeserver: "http://127.0.0.1:9".to_owned(),
                    username: "later".to_owned(),
                    password: koushi_state::AuthSecret::new("not-used"),
                    device_display_name: None,
                },
                platform: koushi_state::DisplayPlatform::Linux,
            }))
            .await;
        loop {
            if let CoreEvent::OperationFailed {
                request_id,
                failure,
            } = event_rx.recv().await.expect("later rejection")
                && request_id == later
            {
                assert_eq!(failure, CoreFailure::SessionRequired);
                break;
            }
        }
        handle
            .send(AccountMessage::RetrySessionTeardown { generation: 999 })
            .await;
        assert_no_login_succeeded_for(&mut action_rx, &second_homeserver);
        handle
            .send(AccountMessage::RetrySessionTeardown { generation: 1 })
            .await;
        while probe_rx.recv().await != Some("replacement_teardown_complete") {}
        loop {
            let actions =
                recv_account_action_with_sliding_sync_effects(&handle, &mut action_rx).await;
            if matches!(
                actions.as_slice(),
                [AppAction::LoginSucceeded { info, .. }] if info.homeserver == second_homeserver
            ) {
                break;
            }
        }
        assert_eq!(
            inspect_session_runtime(&handle).await,
            (true, false, false, true)
        );
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn real_store_switch_a_to_b_preserves_both_accounts_and_switches_back() {
        let server_a = spawn_named_quarantine_password_server("@alpha:example.invalid", "DEVICEA");
        let server_b = spawn_named_quarantine_password_server("@beta:example.invalid", "DEVICEB");
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, _event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());

        for (sequence, homeserver) in [(1, server_a.clone()), (2, server_b.clone())] {
            configure_verified_trust(&handle).await;
            let request_id = RequestId {
                connection_id: crate::ids::RuntimeConnectionId(9),
                sequence,
            };
            handle
                .send(AccountMessage::Command(AccountCommand::LoginPassword {
                    request_id,
                    request: LoginRequest {
                        homeserver,
                        username: "fixture".to_owned(),
                        password: koushi_state::AuthSecret::new("synthetic-password"),
                        device_display_name: None,
                    },
                    platform: koushi_state::DisplayPlatform::Linux,
                }))
                .await;
            acknowledge_next_verified_projection(&handle, &mut action_rx).await;
        }

        let backend = CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(
            cred_dir.path(),
        ));
        let saved = backend.load_saved_sessions().expect("saved index");
        assert_eq!(saved.sessions().len(), 2);
        let alpha_key = saved
            .sessions()
            .iter()
            .find(|key| key.user_id == "@alpha:example.invalid")
            .expect("alpha saved")
            .clone();
        let beta_key = saved
            .sessions()
            .iter()
            .find(|key| key.user_id == "@beta:example.invalid")
            .expect("beta saved")
            .clone();
        assert!(backend.load_matrix_session(&alpha_key).is_ok());
        assert!(backend.load_matrix_session(&beta_key).is_ok());

        for (sequence, user_id) in [(3, "@alpha:example.invalid"), (4, "@beta:example.invalid")] {
            configure_verified_trust(&handle).await;
            handle
                .send(AccountMessage::Command(AccountCommand::SwitchAccount {
                    request_id: RequestId {
                        connection_id: crate::ids::RuntimeConnectionId(9),
                        sequence,
                    },
                    account_key: AccountKey(user_id.to_owned()),
                }))
                .await;
            acknowledge_next_verified_projection(&handle, &mut action_rx).await;
            let saved = backend
                .load_saved_sessions()
                .expect("saved index after switch");
            assert_eq!(saved.sessions().len(), 2);
            assert!(backend.load_matrix_session(&alpha_key).is_ok());
            assert!(backend.load_matrix_session(&beta_key).is_ok());
            assert_eq!(
                backend
                    .load_last_session()
                    .expect("last pointer after switch")
                    .expect("last pointer present")
                    .user_id,
                user_id
            );
        }
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn same_key_replacement_preserves_open_store_and_restores_again_once() {
        let homeserver =
            spawn_named_quarantine_password_server("@same-key:example.invalid", "SAMEDEVICE");
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, _event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        for sequence in [1, 2] {
            configure_verified_trust(&handle).await;
            handle
                .send(AccountMessage::Command(AccountCommand::LoginPassword {
                    request_id: RequestId {
                        connection_id: crate::ids::RuntimeConnectionId(11),
                        sequence,
                    },
                    request: LoginRequest {
                        homeserver: homeserver.clone(),
                        username: "same-key".to_owned(),
                        password: koushi_state::AuthSecret::new("synthetic-password"),
                        device_display_name: None,
                    },
                    platform: koushi_state::DisplayPlatform::Linux,
                }))
                .await;
            acknowledge_next_verified_projection(&handle, &mut action_rx).await;
        }
        let backend = CredentialStoreBackend::FileDir(crate::store::FileCredentialStore::new(
            cred_dir.path(),
        ));
        let saved = backend.load_saved_sessions().expect("saved same-key index");
        assert_eq!(saved.sessions().len(), 1);
        let key_id = saved.sessions()[0].clone();
        assert!(backend.load_matrix_session(&key_id).is_ok());
        assert!(recursive_file_count(data_dir.path()) > 0);

        configure_verified_trust(&handle).await;
        handle
            .send(AccountMessage::Command(AccountCommand::SwitchAccount {
                request_id: RequestId {
                    connection_id: crate::ids::RuntimeConnectionId(11),
                    sequence: 3,
                },
                account_key: AccountKey("@same-key:example.invalid".to_owned()),
            }))
            .await;
        acknowledge_next_verified_projection(&handle, &mut action_rx).await;
        assert!(backend.load_matrix_session(&key_id).is_ok());
        assert!(recursive_file_count(data_dir.path()) > 0);
        assert_eq!(
            inspect_session_runtime(&handle).await,
            (true, true, true, true)
        );
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    fn assert_no_login_succeeded_for(
        action_rx: &mut mpsc::Receiver<Vec<AppAction>>,
        homeserver: &str,
    ) {
        while let Ok(actions) = action_rx.try_recv() {
            assert!(!matches!(
                actions.as_slice(),
                [AppAction::LoginSucceeded { info, .. }] if info.homeserver == homeserver
            ));
        }
    }

    async fn recv_until_session_install(
        handle: &AccountActorHandle,
        action_rx: &mut mpsc::Receiver<Vec<AppAction>>,
    ) -> Vec<AppAction> {
        loop {
            let actions = recv_account_action_with_sliding_sync_effects(handle, action_rx).await;
            if actions.iter().any(|action| {
                matches!(
                    action,
                    AppAction::LoginSucceeded { .. } | AppAction::RestoreSessionSucceeded(_)
                )
            }) {
                return actions;
            }
            assert!(actions.iter().all(|action| matches!(
                action,
                AppAction::SlidingSyncCapabilityCheckStarted { .. }
                    | AppAction::SlidingSyncCapabilityCheckCompleted { .. }
            )));
        }
    }

    #[tokio::test]
    async fn restore_installs_provisional_without_normal_sync_or_public_ready_event() {
        let homeserver = spawn_quarantine_password_server();
        let login = koushi_sdk::login_with_password_with_store(
            &LoginRequest {
                homeserver,
                username: "fixture-user".to_owned(),
                password: koushi_state::AuthSecret::new("synthetic-password"),
                device_display_name: Some("Quarantine Test".to_owned()),
            },
            None,
        )
        .await
        .expect("fixture login");
        let key_id = session_key_id_from_info(&login.info);
        let stored = StoredMatrixSession::new(
            login
                .persistable_session()
                .expect("persistable")
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
        assert!(matches!(
            recv_until_session_install(&handle, &mut action_rx)
                .await
                .as_slice(),
            [AppAction::RestoreSessionSucceeded(_)]
        ));
        let persisted = backend
            .load_matrix_session(&key_id)
            .expect("restored credential should remain readable");
        assert!(
            PersistableMatrixSession::from_json(persisted.as_str())
                .expect("persisted restored session")
                .sliding_sync_positive_evidence()
                .is_some(),
            "network support evidence must be durable before trust promotion"
        );
        let public_ready = executor::timeout(Duration::from_millis(100), async {
            loop {
                match event_rx.recv().await.expect("event stream") {
                    CoreEvent::Account(AccountEvent::SessionRestored { .. })
                    | CoreEvent::Sync(_) => return true,
                    _ => {}
                }
            }
        })
        .await;
        assert!(
            public_ready.is_err(),
            "restore escaped quarantine before Verified"
        );
        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    fn recursive_file_count(path: &std::path::Path) -> usize {
        let Ok(entries) = std::fs::read_dir(path) else {
            return 0;
        };
        entries
            .flatten()
            .map(|entry| {
                let path = entry.path();
                if path.is_dir() {
                    recursive_file_count(&path)
                } else {
                    1
                }
            })
            .sum()
    }

    /// Password-login fixture server that also serves the devices list (with
    /// the current device unnamed) and records `PUT /devices/…` rename bodies,
    /// so the #474 password-login device-naming path is provable end-to-end.
    fn spawn_device_naming_password_server()
    -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        let rename_bodies = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let recorder = std::sync::Arc::clone(&rename_bodies);
        let requested_name = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
        let name_writer = std::sync::Arc::clone(&requested_name);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("address");
        std::thread::spawn(move || {
            'accept: while let Ok((mut stream, _)) = listener.accept() {
                let mut request = Vec::new();
                let mut buffer = [0_u8; 4096];
                loop {
                    let count = match stream.read(&mut buffer) {
                        Ok(0) => continue 'accept,
                        Ok(count) => count,
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::ConnectionReset
                                    | std::io::ErrorKind::BrokenPipe
                                    | std::io::ErrorKind::UnexpectedEof
                            ) =>
                        {
                            continue 'accept;
                        }
                        Err(error) => panic!("read: {error}"),
                    };
                    request.extend_from_slice(&buffer[..count]);
                    let text = String::from_utf8_lossy(&request);
                    let Some(end) = text.find("\r\n\r\n") else {
                        continue;
                    };
                    // Header names are case-insensitive; parse the declared
                    // Content-Length so a split segment is never mistaken for
                    // the full request.
                    let length = text
                        .split("\r\n")
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&request);
                let body = if text.starts_with("GET /_matrix/client/versions ") {
                    r#"{"versions":["v1.7"],"unstable_features":{"org.matrix.simplified_msc3575":true}}"#
                        .to_owned()
                } else if text.contains("/_matrix/client/") && text.contains("login") {
                    // Remember an explicit initial device name so the devices
                    // list can report it back (a customized name must read as
                    // present and never be rewritten).
                    let requested_name = text
                        .split("\r\n\r\n")
                        .nth(1)
                        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
                        .and_then(|value| {
                            value
                                .get("initial_device_display_name")
                                .and_then(|name| name.as_str())
                                .map(|name| name.to_owned())
                        })
                        .filter(|name| !name.trim().is_empty());
                    if let Some(name) = requested_name {
                        *name_writer.lock().unwrap() = Some(name);
                    }
                    r#"{"access_token":"fixture-token","device_id":"FIXTUREDEVICE","user_id":"@fixture-user:example.invalid"}"#
                        .to_owned()
                } else if text.contains("GET /_matrix/client/v3/devices ") {
                    // The current device is authoritative; it is unnamed unless
                    // the login request explicitly named it.
                    match name_writer.lock().unwrap().clone() {
                        Some(name) => format!(
                            r#"{{"devices":[{{"device_id":"FIXTUREDEVICE","display_name":"{name}"}}]}}"#
                        ),
                        None => {
                            r#"{"devices":[{"device_id":"FIXTUREDEVICE","display_name":null}]}"#
                                .to_owned()
                        }
                    }
                } else if text.contains("PUT /_matrix/client/v3/devices/") {
                    let json_start = text.find("\r\n\r\n").map(|index| index + 4).unwrap_or(0);
                    recorder
                        .lock()
                        .unwrap()
                        .push(text[json_start..].trim_end().to_owned());
                    r#"{}"#.to_owned()
                } else if text.contains("/_matrix/client/") && text.contains("/keys/query") {
                    r#"{"device_keys":{},"failures":{}}"#.to_owned()
                } else if text.contains("/_matrix/client/") && text.contains("/sync") {
                    r#"{"next_batch":"batch","device_lists":{"changed":[],"left":[]},"rooms":{"invite":{},"join":{},"leave":{},"knock":{}},"to_device":{"events":[]},"presence":{"events":[]},"account_data":{"events":[]},"device_one_time_keys_count":{}}"#
                        .to_owned()
                } else {
                    r#"{"errcode":"M_NOT_FOUND","error":"not found"}"#.to_owned()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).expect("write");
            }
        });
        (format!("http://{addr}"), rename_bodies)
    }

    fn spawn_controllable_quarantine_password_server() -> (
        String,
        std::sync::Arc<std::sync::atomic::AtomicBool>,
        std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        let offline = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let sliding_sync_supported = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let homeserver = spawn_named_quarantine_password_server_with_controls(
            "@fixture-user:example.invalid",
            "FIXTUREDEVICE",
            Some(std::sync::Arc::clone(&offline)),
            None,
            std::sync::Arc::clone(&sliding_sync_supported),
        );
        (homeserver, offline, sliding_sync_supported)
    }

    #[tokio::test]
    async fn quarantine_password_server_outlives_the_legacy_request_budget() {
        let homeserver = spawn_quarantine_password_server();
        let address = homeserver
            .strip_prefix("http://")
            .expect("fixture homeserver scheme")
            .parse::<std::net::SocketAddr>()
            .expect("fixture homeserver address");

        for request_number in 0..300 {
            use std::io::{Read, Write};

            let mut stream = std::net::TcpStream::connect_timeout(&address, Duration::from_secs(1))
                .unwrap_or_else(|error| {
                    panic!("fixture stopped at request {request_number}: {error}")
                });
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .expect("fixture read timeout");
            stream
                .write_all(
                    b"GET /_matrix/client/versions HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                )
                .expect("fixture request");
            let mut response = String::new();
            stream
                .read_to_string(&mut response)
                .unwrap_or_else(|error| {
                    panic!("fixture response {request_number} failed: {error}")
                });
            assert!(
                response.contains(r#""org.matrix.simplified_msc3575":true"#),
                "fixture response {request_number}: {response}"
            );
        }
    }

    #[test]
    fn restore_trace_covers_startup_restore_boundaries_without_private_ids() {
        let restore_last = crate::account::test_source::item_body(
            include_str!("session_lifecycle.rs"),
            "async fn handle_restore_last_session",
        );
        let restore_account = crate::account::test_source::item_body(
            include_str!("session_lifecycle.rs"),
            "async fn restore_account",
        );
        let restore_continuation = crate::account::test_source::item_body(
            include_str!("sliding_sync.rs"),
            "async fn continue_sliding_sync_admission",
        );

        assert!(
            restore_last.contains(
                "trace_account_request(\"restore_last_session\", request_id, \"load_pointer\")"
            ),
            "startup restore must log before reading the last-session pointer"
        );
        assert!(
            restore_last.contains("executor::spawn_blocking"),
            "startup restore must not block the account actor on keychain/filesystem pointer reads"
        );
        assert!(
            restore_last.contains(
                "trace_account_request(\"restore_last_session\", request_id, \"pointer_found\")"
            ),
            "startup restore must log that a pointer exists without printing the account id"
        );
        assert!(
            restore_account.contains(
                "trace_account_request(\"restore_account\", request_id, \"load_session\")"
            ),
            "restore must log before loading the persisted Matrix session blob"
        );
        assert!(
            restore_continuation.contains("trace_account_request(")
                && restore_continuation.contains("\"restore_account\"")
                && restore_continuation.contains("core_request_id")
                && restore_continuation.contains("\"store_restore_ok\""),
            "restore must log successful SDK store restore before sync starts"
        );
        assert!(restore_continuation.contains("install_provisional_session"));
        assert!(!restore_account.contains("sync_actor_spawned"));
        assert!(
            include_str!("actor.rs").contains("DiagnosticField::request_id"),
            "restore diagnostics must include request ids for correlation"
        );
        assert!(
            !restore_last.contains("account_name()")
                && !restore_account.contains("account_name()")
                && !restore_continuation.contains("account_name()"),
            "startup restore diagnostics must not print account identifiers"
        );
    }

    #[test]
    fn verification_restore_diagnostics_separate_trust_timing_from_persistence() {
        let restore_into_store = crate::account::test_source::item_body(
            include_str!("session_lifecycle.rs"),
            "async fn restore_into_store",
        );
        let recovery_success = crate::account::test_source::item_body(
            include_str!("recovery_backup.rs"),
            "async fn handle_recovery_finished",
        );
        let recovery_promote = crate::account::test_source::item_body(
            include_str!("trust_gate.rs"),
            "async fn promote_recovered_session_runtime",
        );

        assert!(
            restore_into_store.contains("\"store_config_ready\"")
                && restore_into_store.contains("\"sdk_restore_begin\"")
                && restore_into_store.contains("\"sdk_restore_ok\""),
            "restore diagnostics must show store config readiness separately from SDK restore"
        );
        assert!(
            recovery_success.contains("\"post_recovery_trust_read\""),
            "recovery success must log the immediate SDK trust read before promotion"
        );
        assert!(
            recovery_promote.contains("\"persisted\"")
                && recovery_promote.contains("\"promoted\"")
                && recovery_promote.contains("current_device_trust_token"),
            "recovery promotion must log trust around persistence and promotion"
        );
    }

    #[test]
    fn password_login_prefers_saved_device_without_making_login_fail_closed() {
        let login_handler = crate::account::test_source::item_body(
            include_str!("session_lifecycle.rs"),
            "async fn handle_login_password",
        );
        let reuse_helper = crate::account::test_source::item_body(
            include_str!("session_lifecycle.rs"),
            "async fn prefer_saved_device_for_password_login",
        );

        assert!(
            login_handler.contains("prefer_saved_device_for_password_login"),
            "password login should try to reuse a preserved signed-out device before restoring into a store"
        );
        assert!(
            reuse_helper.contains("self.lookup_session_key_id(account_key).await"),
            "saved-device reuse must be driven by the saved-session index"
        );
        assert!(
            reuse_helper.contains("koushi_sdk::login_with_existing_device"),
            "saved-device reuse must explicitly login with the preserved device id"
        );
        assert!(
            reuse_helper.contains("fallback_to_fresh_device"),
            "saved-device reuse must be fail-open so password login availability is not reduced"
        );
        assert!(
            reuse_helper.contains("actual_saved_key_id != saved_key_id"),
            "homeservers that ignore the requested device id must not poison the preserved store"
        );
    }

    #[test]
    fn session_change_observer_routes_unknown_token_to_session_lock() {
        let observer_start_body = crate::account::test_source::item_body(
            include_str!("session_lifecycle.rs"),
            "fn start_session_change_observer",
        );
        let observer_run_body = crate::account::test_source::item_body(
            include_str!("session_lifecycle.rs"),
            "async fn run_session_change_observation",
        );
        let handler_body = crate::account::test_source::item_body(
            include_str!("session_lifecycle.rs"),
            "async fn handle_session_invalidated",
        );

        assert!(
            observer_start_body.contains("subscribe_to_session_changes()"),
            "AccountActor must subscribe to the SDK session-change channel; sync errors are not a reliable auth-invalidated source"
        );
        assert!(
            observer_run_body.contains("matrix_sdk::SessionChange::UnknownToken(data)"),
            "UnknownToken must be handled explicitly instead of inferred from SyncService Offline/Error"
        );
        assert!(
            observer_run_body.contains("soft_logout: data.soft_logout"),
            "only the private-data-free soft_logout bool may cross into AccountActor"
        );
        assert!(
            handler_body.contains("AppAction::SessionAuthenticationInvalidated"),
            "auth invalidation must preserve its distinct reason when locking the active session"
        );
        assert!(
            handler_body.contains("self.stop_sync_actor().await"),
            "auth invalidation must stop the old sync loop instead of leaving it reconnecting forever"
        );
    }

    #[tokio::test]
    async fn session_change_observer_records_exact_unknown_token_diagnostics_for_both_soft_logout_values()
     {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();

        for soft_logout in [true, false] {
            let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
                .records
                .len();
            let (tx, mut receiver) = mpsc::channel(1);
            let (change_tx, change_rx) = broadcast::channel(1);
            let (_stop_tx, stop_rx) = oneshot::channel();
            let task =
                executor::spawn(run_session_change_observation(change_rx, tx, stop_rx, None));
            let mut unknown_token = matrix_sdk::ruma::api::error::UnknownTokenErrorData::new();
            unknown_token.soft_logout = soft_logout;
            change_tx
                .send(matrix_sdk::SessionChange::UnknownToken(unknown_token))
                .expect("publish synthetic session invalidation");

            match receiver.recv().await.expect("observer message") {
                AccountMessage::SessionInvalidated {
                    reason:
                        SessionInvalidationReason::UnknownToken {
                            soft_logout: observed,
                        },
                } => assert_eq!(observed, soft_logout),
                _ => panic!("expected UnknownToken invalidation"),
            }
            task.await.expect("session-change observer task");

            let expected = format!(
                "stage=session_change_received source=matrix_sdk reason=unknown_token soft_logout={soft_logout}"
            );
            assert!(
                koushi_diagnostics::test_support::detail_snapshot().records[diagnostic_start..]
                    .iter()
                    .any(|record| koushi_diagnostics::format_event(&record.event) == expected),
                "missing exact observer diagnostic: {expected}"
            );
        }
    }

    #[tokio::test]
    async fn admitted_unknown_token_records_exact_lock_diagnostics_for_both_soft_logout_values() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();

        for soft_logout in [true, false] {
            let (handle, mut action_rx) = crate::account::test_support::login_gated_actor().await;
            consume_initial_unknown_trust_projection(&mut action_rx).await;
            handle
                .send(AccountMessage::CurrentDeviceTrustChanged {
                    generation: 2,
                    trust: koushi_state::CurrentDeviceTrustState::Verified,
                })
                .await;
            acknowledge_next_verified_projection(&handle, &mut action_rx).await;
            let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
                .records
                .len();

            assert!(
                handle
                    .send(AccountMessage::SessionInvalidated {
                        reason: SessionInvalidationReason::UnknownToken { soft_logout },
                    })
                    .await
            );
            loop {
                let actions = action_rx.recv().await.expect("account action");
                if let [
                    AppAction::SessionAuthenticationInvalidated {
                        soft_logout: observed,
                    },
                ] = actions.as_slice()
                {
                    assert_eq!(*observed, soft_logout);
                    break;
                }
            }

            let expected = format!(
                "stage=session_invalidated reason=unknown_token soft_logout={soft_logout} action=lock"
            );
            assert!(
                koushi_diagnostics::test_support::detail_snapshot().records[diagnostic_start..]
                    .iter()
                    .any(|record| koushi_diagnostics::format_event(&record.event) == expected),
                "missing exact admission diagnostic: {expected}"
            );
            shutdown_and_ack(&handle).await;
        }
    }

    #[tokio::test]
    async fn unknown_token_before_session_promotion_is_inert_and_not_diagnosed() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        let (handle, mut action_rx) = crate::account::test_support::login_gated_actor().await;
        consume_initial_unknown_trust_projection(&mut action_rx).await;

        let before = inspect_session_runtime(&handle).await;
        assert!(before.0, "the provisional actor must still own a session");
        assert!(!before.1, "the session must not be promoted yet");
        let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
            .records
            .len();

        assert!(
            handle
                .send(AccountMessage::SessionInvalidated {
                    reason: SessionInvalidationReason::UnknownToken { soft_logout: true },
                })
                .await
        );
        let after = inspect_session_runtime(&handle).await;
        assert!(
            after.0,
            "an unpromoted UnknownToken must retain the session"
        );
        assert!(
            !after.1,
            "an unpromoted UnknownToken must remain unpromoted"
        );
        while let Ok(actions) = action_rx.try_recv() {
            assert!(
                !matches!(
                    actions.as_slice(),
                    [AppAction::SessionAuthenticationInvalidated { .. }]
                ),
                "an unpromoted UnknownToken must not dispatch an authentication lock"
            );
        }
        assert!(
            !koushi_diagnostics::test_support::detail_snapshot().records[diagnostic_start..]
                .iter()
                .any(|record| {
                    record.event.source == "core.account"
                        && record.event.stage == "session_invalidated"
                }),
            "an unpromoted UnknownToken must not emit an admitted lock diagnostic"
        );
        shutdown_and_ack(&handle).await;
    }

    #[tokio::test]
    async fn unknown_token_fences_an_in_flight_verified_trust_completion() {
        let (handle, mut action_rx) = crate::account::test_support::login_gated_actor().await;
        consume_initial_unknown_trust_projection(&mut action_rx).await;
        handle
            .send(AccountMessage::CurrentDeviceTrustChanged {
                generation: 2,
                trust: koushi_state::CurrentDeviceTrustState::Verified,
            })
            .await;
        acknowledge_next_verified_projection(&handle, &mut action_rx).await;

        assert!(
            handle
                .send(AccountMessage::SessionInvalidated {
                    reason: SessionInvalidationReason::UnknownToken { soft_logout: false },
                })
                .await
        );
        while !matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::SessionAuthenticationInvalidated { .. }])
        ) {}
        handle
            .send(AccountMessage::CurrentDeviceTrustRecheckFinished {
                generation: 2,
                result: Ok(koushi_state::CurrentDeviceTrustState::Verified),
            })
            .await;
        let _ = inspect_session_runtime(&handle).await;
        while let Ok(actions) = action_rx.try_recv() {
            assert!(
                !matches!(
                    actions.as_slice(),
                    [AppAction::AuthoritativeDeviceTrustChanged {
                        trust: koushi_state::CurrentDeviceTrustState::Verified,
                        ..
                    }]
                ),
                "stale trust completion must not unlock an invalid authentication session"
            );
        }
        shutdown_and_ack(&handle).await;
    }

    #[tokio::test]
    async fn post_teardown_unknown_token_message_is_inert_and_not_diagnosed() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        let (handle, mut action_rx) = crate::account::test_support::login_gated_actor().await;
        consume_initial_unknown_trust_projection(&mut action_rx).await;
        handle
            .send(AccountMessage::CurrentDeviceTrustChanged {
                generation: 2,
                trust: koushi_state::CurrentDeviceTrustState::Verified,
            })
            .await;
        acknowledge_next_verified_projection(&handle, &mut action_rx).await;

        assert!(
            handle
                .send(AccountMessage::Command(AccountCommand::Logout {
                    request_id: RequestId {
                        connection_id: RuntimeConnectionId(1),
                        sequence: 2,
                    },
                }))
                .await
        );
        while !matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::LogoutFinished])
        ) {}
        let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
            .records
            .len();

        assert!(
            handle
                .send(AccountMessage::SessionInvalidated {
                    reason: SessionInvalidationReason::UnknownToken { soft_logout: true },
                })
                .await
        );
        assert_eq!(
            inspect_session_runtime(&handle).await,
            (false, false, false, false)
        );
        while let Ok(actions) = action_rx.try_recv() {
            assert!(
                !matches!(
                    actions.as_slice(),
                    [AppAction::SessionAuthenticationInvalidated { .. }]
                ),
                "post-teardown invalidation must not dispatch a state action"
            );
        }
        assert!(
            !koushi_diagnostics::test_support::detail_snapshot().records[diagnostic_start..]
                .iter()
                .any(|record| {
                    record.event.source == "core.account"
                        && record.event.stage == "session_invalidated"
                }),
            "post-teardown invalidation must not emit an admission diagnostic"
        );
        shutdown_and_ack(&handle).await;
    }

    #[tokio::test]
    async fn session_change_observer_stop_interrupts_blocked_mailbox_delivery() {
        let (tx, mut receiver) = mpsc::channel(1);
        tx.send(AccountMessage::Shutdown)
            .await
            .expect("fill the account mailbox");
        let (change_tx, change_rx) = broadcast::channel(1);
        let (stop_tx, stop_rx) = oneshot::channel();
        let delivery_barrier = Arc::new(tokio::sync::Barrier::new(2));
        let mut task = executor::spawn(run_session_change_observation(
            change_rx,
            tx,
            stop_rx,
            Some(delivery_barrier.clone()),
        ));
        let mut unknown_token = matrix_sdk::ruma::api::error::UnknownTokenErrorData::new();
        unknown_token.soft_logout = true;
        change_tx
            .send(matrix_sdk::SessionChange::UnknownToken(unknown_token))
            .expect("publish synthetic session invalidation");

        delivery_barrier.wait().await;
        stop_tx.send(()).expect("request observer stop");
        match executor::timeout(Duration::from_millis(250), &mut task).await {
            Ok(joined) => joined.expect("session-change observer task"),
            Err(_) => {
                task.abort();
                let _ = task.await;
                panic!("stop must interrupt a blocked session-change mailbox delivery");
            }
        }

        assert!(matches!(
            receiver.recv().await,
            Some(AccountMessage::Shutdown)
        ));
        assert!(
            receiver.try_recv().is_err(),
            "stop must discard only the blocked observer delivery"
        );
    }

    #[test]
    fn soft_logout_reauth_keeps_locked_session_until_password_login_succeeds() {
        let handler_body = crate::account::test_source::item_body(
            include_str!("session_lifecycle.rs"),
            "async fn handle_soft_logout_reauth",
        );
        let login_call = handler_body
            .find("koushi_sdk::login_with_existing_device")
            .expect("reauth must use device-preserving password login");
        let drop_old_session = handler_body
            .find("drop(self.session.take())")
            .expect("reauth must drop the old client before restoring into the account store");

        assert!(
            login_call < drop_old_session,
            "wrong passwords must not discard the locked session before the user can retry"
        );
    }

    #[tokio::test]
    async fn soft_logout_reauth_joins_old_observers_before_subscribing_replacements() {
        let homeserver = spawn_quarantine_password_server();
        let cred_dir = tempdir().expect("tempdir");
        let data_dir = tempdir().expect("tempdir");
        let (handle, mut action_rx, _event_rx) =
            spawn_actor_with_dirs(cred_dir.path(), data_dir.path());
        let (probe_tx, mut probe_rx) = mpsc::unbounded_channel();
        assert!(
            handle
                .send(AccountMessage::AttachLifecycleProbe { probe_tx })
                .await
        );
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

        assert!(
            handle
                .send(AccountMessage::SessionInvalidated {
                    reason: SessionInvalidationReason::UnknownToken { soft_logout: true },
                })
                .await
        );
        while !matches!(
            action_rx.recv().await.as_deref(),
            Some([AppAction::SessionAuthenticationInvalidated { soft_logout: true }])
        ) {}

        let request_id = RequestId {
            connection_id: crate::ids::RuntimeConnectionId(1),
            sequence: 2,
        };
        assert!(
            handle
                .send(AccountMessage::Command(AccountCommand::SoftLogoutReauth {
                    request_id,
                    password: koushi_state::AuthSecret::new("synthetic-password"),
                }))
                .await
        );
        while !matches!(
            action_rx.recv().await.as_deref(),
            Some([
                AppAction::SoftLogoutReauthSucceeded { request_id: 2 },
                AppAction::LoginSucceeded { .. }
            ])
        ) {}
        let _ = inspect_session_runtime(&handle).await;

        let tokens: Vec<_> = std::iter::from_fn(|| probe_rx.try_recv().ok()).collect();
        let recovery_stop = tokens
            .iter()
            .position(|token| *token == "recovery_observer_stop_requested")
            .expect("the old recovery observer must be stopped");
        let recovery_join = tokens
            .iter()
            .position(|token| *token == "recovery_observer_terminated")
            .expect("the old recovery observer must be joined");
        let recovery_start = tokens
            .iter()
            .position(|token| *token == "recovery_observer_started")
            .expect("the replacement recovery observer must start");
        let verification_stop = tokens
            .iter()
            .position(|token| *token == "incoming_verification_observer_stop_requested")
            .expect("the old verification observer must be stopped");
        let verification_join = tokens
            .iter()
            .position(|token| *token == "incoming_verification_observer_terminated")
            .expect("the old verification observer must be joined");
        let verification_subscribe = tokens
            .iter()
            .position(|token| *token == "incoming_verification_observer_subscribing")
            .expect("the replacement verification observer must subscribe");
        assert!(
            recovery_stop < recovery_join && recovery_join < recovery_start,
            "{tokens:?}"
        );
        assert!(
            verification_stop < verification_join && verification_join < verification_subscribe,
            "{tokens:?}"
        );

        let _ = handle.send(AccountMessage::Shutdown).await;
    }

    #[tokio::test]
    async fn server_logout_best_effort_returns_on_timeout() {
        let outcome = wait_for_server_logout_best_effort(
            std::time::Duration::from_millis(1),
            futures_util::future::pending(),
        )
        .await;

        assert_eq!(outcome, ServerLogoutOutcome::TimedOut);
    }

    #[tokio::test]
    async fn server_logout_best_effort_treats_network_failure_as_settled() {
        let outcome =
            wait_for_server_logout_best_effort(std::time::Duration::from_secs(1), async {
                Err(koushi_sdk::PasswordLoginError::Sdk(
                    "synthetic network failure".to_owned(),
                ))
            })
            .await;

        assert_eq!(outcome, ServerLogoutOutcome::Failed);
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

    #[test]
    fn account_actor_credential_store_hot_paths_use_blocking_port() {
        let persist_session = crate::account::test_source::item_body(
            include_str!("session_lifecycle.rs"),
            "async fn persist_session",
        );
        let clear_persistence = crate::account::test_source::item_body(
            include_str!("session_lifecycle.rs"),
            "async fn clear_account_persistence",
        );
        let lookup_session = crate::account::test_source::item_body(
            include_str!("session_lifecycle.rs"),
            "async fn lookup_session_key_id",
        );
        let query_saved = crate::account::test_source::item_body(
            include_str!("session_lifecycle.rs"),
            "async fn handle_query_saved_sessions",
        );
        let probe_health = crate::account::test_source::item_body(
            include_str!("local_data_cleanup.rs"),
            "async fn handle_probe_local_encryption_health",
        );

        for section in [
            persist_session,
            clear_persistence,
            lookup_session,
            query_saved,
            probe_health,
        ] {
            assert!(
                section.contains("executor::spawn_blocking"),
                "AccountActor credential-store and filesystem hot paths must be offloaded"
            );
        }
    }
}
