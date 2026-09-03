//! `session_lifecycle` ownership for AccountActor.

use std::{
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_key::{LocalStoreId, StoredMatrixSession};
use koushi_protocol::SessionKeyId;
use koushi_sdk::{MatrixClientSession, PendingOidcLogin, PersistableMatrixSession};
use koushi_state::{
    AppAction, AuthFailureKind, LoginAttemptId, LoginRequest, SessionInfo, SlidingSyncAdmission,
};
use tokio::sync::{mpsc, oneshot};

use crate::executor;
use crate::startup_trace::{self, StartupPhase};
use crate::store::{
    AccountStoreConfig, PendingLoginCleanupEvidence, account_key_from_info,
    session_key_id_from_info,
};
use koushi_protocol::event::{AccountEvent, CoreEvent};
use koushi_protocol::failure::{CoreFailure, LoginFailureKind};
use koushi_protocol::ids::{AccountKey, RequestId};

use super::actor::{AccountActor, AccountMessage, trace_account_request, trace_restore};
use super::sliding_sync::PendingSlidingSyncAdmission;
use super::trust_gate::{
    advance_observed_trust, current_device_trust_token, record_verification_admission_event,
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

// Hostless private-use URI in reverse-DNS notation (RFC 8252 §7.1). MAS
// deployments (matrix.org) reject both a bare scheme like `koushi-desktop`
// and the `scheme://host/path` form with `invalid_redirect_uri`; the scheme
// must be derived from the client_uri host (github.com) and the URI must
// carry no authority component.
const OIDC_REDIRECT_URI: &str = "com.github.shinaoka.koushi-matrix:/auth/callback";

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

pub(super) struct LockedSessionRecord {
    pub(super) info: SessionInfo,
    pub(super) key_id: SessionKeyId,
    pub(super) persistable: PersistableMatrixSession,
    pub(super) binding: Option<AccountStoreConfig>,
}

pub(super) enum PendingOidcFlow {
    Sdk {
        pending: PendingOidcLogin,
        allocation: Option<(LocalStoreId, u64)>,
    },
    #[cfg(test)]
    Synthetic { homeserver: String },
}

impl PendingOidcFlow {
    fn homeserver(&self) -> &str {
        match self {
            Self::Sdk { pending, .. } => pending.homeserver(),
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
                    Ok(matrix_sdk::SessionChange::TokensRefreshed) => {
                        record(
                            DiagnosticEvent::new(
                                DiagnosticLevel::Info,
                                "core.account",
                                "session_change_received",
                            )
                            .field(DiagnosticField::token("source", "matrix_sdk"))
                            .field(DiagnosticField::token("reason", "tokens_refreshed")),
                        );
                        // Unlike an invalidation, a rotation leaves the session
                        // usable: keep observing so a later invalidation still
                        // reaches the actor.
                        if !send_observer_output_until_stopped(
                            &tx,
                            AccountMessage::SessionTokensRefreshed,
                            &mut stop_rx,
                        )
                        .await
                        {
                            break;
                        }
                    }
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
        PasswordLoginError::Serialization(_) | PasswordLoginError::SavedCryptoStore(_) => {
            LoginFailureKind::Store
        }
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

fn fresh_login_cleanup_evidence(
    error: &koushi_sdk::PasswordLoginError,
) -> Option<PendingLoginCleanupEvidence> {
    match error {
        koushi_sdk::PasswordLoginError::InvalidHomeserver(_) => {
            Some(PendingLoginCleanupEvidence::NoRequestSent)
        }
        koushi_sdk::PasswordLoginError::Sdk(message)
            if message.contains("401")
                || message.contains("403")
                || message.contains("M_UNAUTHORIZED")
                || message.contains("M_FORBIDDEN") =>
        {
            Some(PendingLoginCleanupEvidence::ServerRejectedBeforeSession)
        }
        _ => None,
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
        | koushi_sdk::PasswordLoginError::Serialization(_)
        | koushi_sdk::PasswordLoginError::SavedCryptoStore(_) => AuthFailureKind::Sdk,
    }
}

impl AccountActor {
    #[cfg(any(test, feature = "test-hooks"))]
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

    /// Replace the stored credentials after the SDK rotated the session
    /// tokens. MAS invalidates a refresh token as soon as it is used, so a
    /// vault that still holds the pre-rotation copy cannot restore the session
    /// on the next launch — the homeserver answers the first authenticated
    /// request with `M_UNKNOWN_TOKEN`.
    pub(super) async fn handle_session_tokens_refreshed(&mut self) {
        let (Some(session), Some(key_id)) = (self.session.clone(), self.session_key_id.clone())
        else {
            return;
        };
        let outcome = if self.persist_session(&session, &key_id).await.is_ok() {
            "persisted"
        } else {
            "failed"
        };
        trace_restore!(
            "session_tokens_refreshed",
            [DiagnosticField::token("outcome", outcome)],
            "outcome={}",
            outcome
        );
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

        let locked_record = self.session.as_ref().and_then(|session| {
            let key_id = self.session_key_id.clone()?;
            let persistable = session.persistable_session().ok()?;
            let binding = self.store.existing_account_store_config(&key_id).ok();
            Some(LockedSessionRecord {
                info: session.info.clone(),
                key_id,
                persistable,
                binding,
            })
        });
        if !self.stop_current_session_runtime().await {
            return;
        }
        drop(self.session.take());
        self.session_key_id = None;
        self.locked_session_record = locked_record;
        self.record_lifecycle_probe("locked_client_released");
    }

    pub(super) async fn handle_discover_login(
        &mut self,
        request_id: RequestId,
        homeserver: String,
    ) {
        let requested_homeserver = homeserver.clone();
        let discovery_result =
            tokio::task::spawn_blocking(move || koushi_sdk::discover_login_flows(&homeserver))
                .await;

        match discovery_result {
            Ok(Ok(discovery)) => {
                self.send_actions(vec![AppAction::LoginDiscoverySucceeded {
                    homeserver: requested_homeserver.clone(),
                    flows: discovery.flows,
                    delegated: discovery.delegated,
                }])
                .await;
                self.emit(CoreEvent::Account(AccountEvent::AuthDiscoveryChanged {
                    request_id,
                    homeserver: requested_homeserver,
                }));
            }
            Ok(Err(error)) => {
                self.send_actions(vec![AppAction::LoginDiscoveryFailed {
                    homeserver: requested_homeserver.clone(),
                    kind: login_discovery_failure_kind(&error),
                }])
                .await;
                self.emit(CoreEvent::Account(AccountEvent::AuthDiscoveryChanged {
                    request_id,
                    homeserver: requested_homeserver,
                }));
            }
            Err(_) => {
                self.send_actions(vec![AppAction::LoginDiscoveryFailed {
                    homeserver: requested_homeserver.clone(),
                    kind: AuthFailureKind::Sdk,
                }])
                .await;
                self.emit(CoreEvent::Account(AccountEvent::AuthDiscoveryChanged {
                    request_id,
                    homeserver: requested_homeserver,
                }));
            }
        }
    }

    pub(super) async fn handle_start_oidc_login(
        &mut self,
        request_id: RequestId,
        homeserver: String,
    ) {
        let homeserver = match koushi_sdk::Homeserver::parse(&homeserver) {
            Ok(homeserver) => homeserver,
            Err(error) => {
                let kind = login_discovery_failure_kind(&error);
                self.emit_failure(request_id, CoreFailure::AccountOperationFailed { kind });
                return;
            }
        };
        let normalized_homeserver = homeserver.normalized();

        let (store_config, requested_device_id, allocation) =
            if let Some(locked) = self.locked_session_record.as_ref() {
                let Some(binding) = locked.binding.as_ref() else {
                    self.emit_failure(request_id, CoreFailure::LocalEncryptionUnavailable);
                    return;
                };
                (
                    Some(binding.store_config.clone()),
                    Some(locked.info.device_id.clone()),
                    None,
                )
            } else {
                let device_id = LocalStoreId::generate().as_str().to_owned();
                let pending = match self.store.pending_login_owner().resume_or_create(
                    normalized_homeserver.clone(),
                    "oidc",
                    device_id,
                ) {
                    Ok(pending) => pending,
                    Err(failure) => {
                        self.emit_failure(request_id, failure);
                        return;
                    }
                };
                let store_config = match self.store.pending_login_owner().store_config(&pending) {
                    Ok(config) => config,
                    Err(failure) => {
                        self.emit_failure(request_id, failure);
                        return;
                    }
                };
                (
                    Some(store_config),
                    Some(pending.device_id.clone()),
                    Some((pending.allocation_id.clone(), pending.attempt_generation)),
                )
            };

        match koushi_sdk::start_oidc_login_with_store(
            &normalized_homeserver,
            OIDC_REDIRECT_URI,
            store_config.as_ref(),
            requested_device_id.as_deref(),
            self.locked_session_record.is_some(),
        )
        .await
        {
            Ok((pending, authorization)) => {
                self.pending_oidc_login = Some((
                    request_id,
                    PendingOidcFlow::Sdk {
                        pending,
                        allocation,
                    },
                ));
                self.emit(CoreEvent::Account(AccountEvent::OidcAuthorizationCreated {
                    request_id,
                    authorization_url: authorization.authorization_url,
                    state: authorization.state,
                }));
            }
            Err(error) => {
                let kind = classify_auth_error(&error);
                self.send_actions(vec![AppAction::LoginDiscoveryFailed {
                    homeserver: normalized_homeserver,
                    kind,
                }])
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
        let allocation = match &pending {
            PendingOidcFlow::Sdk { allocation, .. } => allocation.clone(),
            #[cfg(test)]
            PendingOidcFlow::Synthetic { .. } => None,
        };
        if let Some((allocation_id, attempt_generation)) = allocation.as_ref()
            && !self
                .store
                .pending_login_owner()
                .is_current(allocation_id, *attempt_generation)
                .unwrap_or(false)
        {
            self.emit_failure(
                request_id,
                CoreFailure::AccountOperationFailed {
                    kind: AuthFailureKind::Cancelled,
                },
            );
            return;
        }
        self.send_actions(vec![AppAction::AuthenticationStarted {
            attempt_id: LoginAttemptId::new(request_id.connection_id.0, request_id.sequence),
            homeserver: homeserver.clone(),
        }])
        .await;

        #[cfg(test)]
        let login_result = match self.oidc_completion_override.take() {
            Some(session) => Ok(session),
            None => match pending {
                PendingOidcFlow::Sdk { pending, .. } => {
                    koushi_sdk::finish_oidc_login(pending, &callback_url).await
                }
                PendingOidcFlow::Synthetic { .. } => {
                    unreachable!("synthetic OIDC completion requires a session override")
                }
            },
        };
        #[cfg(not(test))]
        let login_result = match pending {
            PendingOidcFlow::Sdk { pending, .. } => {
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
        let reauth_key_id = self
            .locked_session_record
            .as_ref()
            .map(|record| record.key_id.clone());
        if let Some(expected_key_id) = reauth_key_id.as_ref()
            && key_id != *expected_key_id
        {
            self.abort_login(login_session, expected_key_id, false, true)
                .await;
            self.emit_failure(request_id, CoreFailure::LocalEncryptionUnavailable);
            return;
        }
        if let Some((allocation_id, attempt_generation)) = allocation
            && self
                .store
                .pending_login_owner()
                .bind(&allocation_id, attempt_generation, key_id.clone())
                .is_err()
        {
            self.abort_login(login_session, &key_id, false, true).await;
            self.emit_failure(request_id, CoreFailure::StoreUnavailable);
            return;
        }
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
        let admission_action = if reauth_key_id.is_some() {
            self.locked_session_record = None;
            AppAction::SoftLogoutReauthSessionInstalled {
                request_id: request_id.sequence,
                info: info.clone(),
            }
        } else {
            AppAction::LoginSucceeded {
                attempt_id: LoginAttemptId::new(request_id.connection_id.0, request_id.sequence),
                info: info.clone(),
            }
        };
        self.begin_sliding_sync_capability_discovery(
            PendingSlidingSyncAdmission::NewLogin {
                account_epoch,
                request_id: capability_request_id,
                core_request_id: request_id,
                login_session,
                persistable,
                key_id,
                action: admission_action,
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

        let normalized_homeserver = match koushi_sdk::Homeserver::parse(&request.homeserver) {
            Ok(homeserver) => homeserver.normalized(),
            Err(error) => {
                let error = koushi_sdk::PasswordLoginError::InvalidHomeserver(error);
                self.emit_failure(
                    request_id,
                    CoreFailure::LoginFailed {
                        kind: classify_login_error(&error),
                    },
                );
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

        let saved_key_id = match self
            .lookup_saved_device_for_password_login(&request.username, &normalized_homeserver)
            .await
        {
            Ok(key_id) => key_id,
            Err(failure) => {
                self.emit_failure(request_id, failure);
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

        let (login_session, key_id) = if let Some(saved_key_id) = saved_key_id {
            let store_config = match self.store.existing_account_store_config(&saved_key_id) {
                Ok(config) => config,
                Err(failure) => {
                    self.emit_failure(request_id, failure);
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
            let preflight = koushi_sdk::preflight_saved_crypto_store(
                &store_config.store_config,
                Some(&saved_key_id.user_id),
                Some(&saved_key_id.device_id),
            )
            .await;
            if preflight != koushi_sdk::SavedCryptoStorePreflight::PresentMatching {
                self.emit_failure(request_id, CoreFailure::LocalEncryptionUnavailable);
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
            let mut login_request = request.clone();
            if !login_request.username.starts_with('@') {
                // Use the selected saved identity as the expected Matrix user
                // without changing the UI's login identifier.
                login_request.username = saved_key_id.user_id.clone();
            }
            let login_session = match koushi_sdk::login_with_password_with_store_and_device(
                &login_request,
                Some(&store_config.store_config),
                Some(&saved_key_id.device_id),
            )
            .await
            {
                Ok(session) => session,
                Err(error) => {
                    self.emit_failure(
                        request_id,
                        CoreFailure::LoginFailed {
                            kind: classify_login_error(&error),
                        },
                    );
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
            if session_key_id_from_info(&login_session.info) != saved_key_id {
                self.abort_login(login_session, &saved_key_id, false, true)
                    .await;
                let _ = koushi_sdk::preflight_saved_crypto_store(
                    &store_config.store_config,
                    Some(&saved_key_id.user_id),
                    Some(&saved_key_id.device_id),
                )
                .await;
                self.emit_failure(request_id, CoreFailure::LocalEncryptionUnavailable);
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
            (login_session, saved_key_id)
        } else {
            let device_id = LocalStoreId::generate().as_str().to_owned();
            let pending = match self.store.pending_login_owner().resume_or_create(
                normalized_homeserver.clone(),
                "password",
                device_id.clone(),
            ) {
                Ok(record) => record,
                Err(failure) => {
                    self.emit_failure(request_id, failure);
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
            let store_config = match self.store.pending_login_owner().store_config(&pending) {
                Ok(config) => config,
                Err(failure) => {
                    self.emit_failure(request_id, failure);
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
            let login_session = match koushi_sdk::login_with_password_with_new_device(
                &request,
                &store_config,
                &pending.device_id,
            )
            .await
            {
                Ok(session) => session,
                Err(error) => {
                    if let Some(evidence) = fresh_login_cleanup_evidence(&error) {
                        let _ = self.store.pending_login_owner().cancel(
                            &pending.allocation_id,
                            pending.attempt_generation,
                            evidence,
                        );
                    }
                    self.emit_failure(
                        request_id,
                        CoreFailure::LoginFailed {
                            kind: classify_login_error(&error),
                        },
                    );
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
            let key_id = session_key_id_from_info(&login_session.info);
            if self
                .store
                .pending_login_owner()
                .bind(
                    &pending.allocation_id,
                    pending.attempt_generation,
                    key_id.clone(),
                )
                .is_err()
            {
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
            (login_session, key_id)
        };

        let info = login_session.info.clone();
        let account_key = account_key_from_info(&info);
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
            normalized_homeserver,
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
        record(DiagnosticEvent::new(
            DiagnosticLevel::Info,
            "core.login_store_reauth",
            "started",
        ));
        let locked_record = if let Some(record) = self.locked_session_record.take() {
            record
        } else {
            let Some(session) = self.session.as_ref() else {
                self.emit_failure(request_id, CoreFailure::SessionRequired);
                return;
            };
            let key_id = session_key_id_from_info(&session.info);
            LockedSessionRecord {
                info: session.info.clone(),
                key_id: key_id.clone(),
                persistable: match session.persistable_session() {
                    Ok(persistable) => persistable,
                    Err(_) => {
                        self.emit_failure(request_id, CoreFailure::LocalEncryptionUnavailable);
                        return;
                    }
                },
                binding: self.store.existing_account_store_config(&key_id).ok(),
            }
        };
        let LockedSessionRecord {
            info,
            key_id,
            persistable: old_persistable,
            binding,
        } = locked_record;
        let Some(binding) = binding else {
            self.locked_session_record = Some(LockedSessionRecord {
                info,
                key_id,
                persistable: old_persistable,
                binding: None,
            });
            self.emit_failure(request_id, CoreFailure::LocalEncryptionUnavailable);
            return;
        };

        // Reauthentication is store-backed from the first SDK call. Retire every
        // owner and drop the invalid client before creating its replacement.
        if self.session.is_some() && !self.stop_current_session_runtime().await {
            self.locked_session_record = Some(LockedSessionRecord {
                info,
                key_id,
                persistable: old_persistable,
                binding: Some(binding),
            });
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        }
        drop(self.session.take());
        self.session_key_id = None;

        if koushi_sdk::preflight_saved_crypto_store(
            &binding.store_config,
            Some(&key_id.user_id),
            Some(&key_id.device_id),
        )
        .await
            != koushi_sdk::SavedCryptoStorePreflight::PresentMatching
        {
            self.locked_session_record = Some(LockedSessionRecord {
                info,
                key_id,
                persistable: old_persistable,
                binding: Some(binding),
            });
            self.emit_failure(request_id, CoreFailure::LocalEncryptionUnavailable);
            return;
        }
        record(DiagnosticEvent::new(
            DiagnosticLevel::Info,
            "core.login_store_reauth",
            "preflight_ok",
        ));

        let request = LoginRequest {
            homeserver: info.homeserver.clone(),
            username: info.user_id.clone(),
            password,
            device_display_name: None,
        };
        let login_session = match koushi_sdk::login_with_password_with_store_and_device(
            &request,
            Some(&binding.store_config),
            Some(&key_id.device_id),
        )
        .await
        {
            Ok(session) => session,
            Err(error) => {
                self.locked_session_record = Some(LockedSessionRecord {
                    info,
                    key_id,
                    persistable: old_persistable,
                    binding: Some(binding),
                });
                self.send_actions(vec![AppAction::SoftLogoutReauthFailed {
                    request_id: request_id.sequence,
                    kind: classify_auth_error(&error),
                }])
                .await;
                self.emit_failure(
                    request_id,
                    CoreFailure::LoginFailed {
                        kind: classify_login_error(&error),
                    },
                );
                return;
            }
        };
        if session_key_id_from_info(&login_session.info) != key_id {
            self.abort_login(login_session, &key_id, false, true).await;
            self.locked_session_record = Some(LockedSessionRecord {
                info,
                key_id,
                persistable: old_persistable,
                binding: Some(binding),
            });
            self.emit_failure(request_id, CoreFailure::LocalEncryptionUnavailable);
            return;
        }
        record(DiagnosticEvent::new(
            DiagnosticLevel::Info,
            "core.login_store_reauth",
            "identity_ok",
        ));
        let persistable = match login_session.persistable_session() {
            Ok(persistable) => persistable,
            Err(_) => {
                self.locked_session_record = Some(LockedSessionRecord {
                    info,
                    key_id: key_id.clone(),
                    persistable: old_persistable,
                    binding: Some(binding),
                });
                self.abort_login(login_session, &key_id, false, true).await;
                self.emit_failure(request_id, CoreFailure::StoreUnavailable);
                return;
            }
        };

        self.pending_ready_events
            .push(CoreEvent::Account(AccountEvent::LoggedIn {
                request_id,
                account_key: account_key_from_info(&info),
            }));
        record(DiagnosticEvent::new(
            DiagnosticLevel::Info,
            "core.login_store_reauth",
            "installing",
        ));
        self.prepare_store_backed_session(&login_session, true)
            .await;
        self.install_provisional_session(
            login_session,
            persistable,
            key_id,
            AppAction::SoftLogoutReauthSessionInstalled {
                request_id: request_id.sequence,
                info,
            },
        )
        .await;
        self.send_actions(vec![AppAction::SoftLogoutReauthSucceeded {
            request_id: request_id.sequence,
        }])
        .await;
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
        if self.session.is_none()
            && let Some(locked) = self.locked_session_record.take()
        {
            if preserve_persistence {
                self.forget_last_session_pointer_if_matches(&locked.key_id)
                    .await;
            } else {
                self.clear_account_persistence(&locked.key_id).await;
            }
            self.send_actions(vec![AppAction::LogoutFinished]).await;
            self.emit(CoreEvent::Account(AccountEvent::LoggedOut {
                request_id,
                account_key: AccountKey(locked.key_id.user_id),
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
        // Match Element's explicit sign-out semantics: revoke the server
        // session and remove this device's credentials and keyed local store.
        // Persistence is retained only by non-destructive flows such as soft
        // logout reauthentication and account switching.
        self.perform_logout(request_id, true, false).await;
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
            // The SDK verification-state subscriber may replay its current
            // value and may emit the same coarse trust state for consecutive
            // crypto updates. Re-projecting an unchanged Unverified value
            // creates a fresh gate transition whose acknowledgement restarts
            // (and aborts) verification-method discovery indefinitely.
            let mut last_trust = current_trust;
            while let Some(trust) = updates.next().await {
                if !advance_observed_trust(&mut last_trust, trust) {
                    continue;
                }
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
        self.cancel_verification_method_discovery_admission_timeout()
            .await;
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
            if store.pending_login_owner().complete_bound(&key_id).is_err() {
                let _ = backend.delete_matrix_session(&key_id);
                let _ = backend.forget_saved_session(&key_id);
                let _ = backend.delete_last_session();
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
        let store_config = self
            .store
            .existing_account_store_config(key_id)
            .map_err(|failure| {
                record(
                    DiagnosticEvent::new(
                        DiagnosticLevel::Warn,
                        "core.login_store_restore",
                        "refused",
                    )
                    .field(DiagnosticField::token("stage", "store_config")),
                );
                failure
            })?;
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
        let session = match koushi_sdk::restore_session_with_verified_store(
            persistable,
            &store_config_with_search,
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
        self.prepare_store_backed_session(&session, encrypted_store)
            .await;
        Ok(session)
    }

    /// Install the event-cache and room-key diagnostic hooks on the exact
    /// authenticated persistent client before it can enter admission.
    pub(super) async fn prepare_store_backed_session(
        &self,
        session: &MatrixClientSession,
        encrypted_store: bool,
    ) {
        let event_cache_result = koushi_sdk::enable_event_cache(session).await;
        self.emit_event_cache_status(encrypted_store, &event_cache_result);
        crate::room_key_receive::reset_late_decryption_counters();
        let diagnostics = koushi_sdk::room_key_receive_diagnostics(session).await;
        crate::room_key_receive::record_room_key_receive_summary(
            &diagnostics,
            crate::room_key_receive::RECEIVE_SUMMARY_TRIGGER_RESTORE,
        );
    }

    /// Roll back a failed login bootstrap: best-effort server logout of the
    /// provisional store-backed client, drop it inside the runtime context,
    /// and — if credentials were already persisted —
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

    async fn lookup_saved_device_for_password_login(
        &self,
        identifier: &str,
        normalized_homeserver: &str,
    ) -> Result<Option<SessionKeyId>, CoreFailure> {
        let identifier = identifier.trim().to_owned();
        let homeserver = normalized_homeserver.to_owned();
        let store = self.store.clone();
        executor::spawn_blocking(move || {
            let sessions = store
                .credential_backend()
                .load_saved_sessions()
                .map_err(|_| CoreFailure::StoreUnavailable)?;
            let exact = identifier.starts_with('@') && identifier.contains(':');
            let mut selected = None;
            for key_id in sessions.sessions().iter().filter(|key_id| {
                if key_id.homeserver != homeserver {
                    return false;
                }
                if exact {
                    key_id.user_id == identifier
                } else {
                    key_id
                        .user_id
                        .strip_prefix('@')
                        .and_then(|user| user.split_once(':'))
                        .is_some_and(|(localpart, _)| localpart == identifier)
                }
            }) {
                if selected.is_some() {
                    return Ok(None);
                }
                selected = Some(key_id.clone());
            }
            Ok(selected)
        })
        .await
        .unwrap_or(Err(CoreFailure::StoreUnavailable))
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
mod tests;
