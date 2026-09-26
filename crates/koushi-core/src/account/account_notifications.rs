//! `account_notifications` ownership for AccountActor (#981).
//!
//! Every write is followed by an authoritative server re-read that becomes the
//! next reducer snapshot, so the UI never shows a requested value the server
//! did not confirm. Loads are read-only.

use koushi_protocol::command::AccountNotificationsRequest;
use koushi_protocol::failure::CoreFailure;
use koushi_protocol::ids::RequestId;
use koushi_state::{
    AccountNotificationsFailureKind, AccountNotificationsOperation, AppAction, AuthFailureKind,
    normalize_notification_email,
};

use super::actor::AccountActor;

/// Actor-private continuation for a notification email awaiting ownership
/// confirmation. The client secret, validation session id, and UIA session
/// never leave this actor.
pub(super) struct PendingNotificationEmailVerification {
    address: String,
    lang: String,
    client_secret: matrix_sdk::ruma::OwnedClientSecret,
    sid: String,
    send_attempt: u32,
    resend_count: u32,
    uia: Option<PendingEmailUia>,
}

struct PendingEmailUia {
    flow_id: u64,
    session: Option<String>,
}

impl std::fmt::Debug for PendingNotificationEmailVerification {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingNotificationEmailVerification")
            .field("address", &"<redacted>")
            .field("send_attempt", &self.send_attempt)
            .field("awaiting_uia", &self.uia.is_some())
            .finish_non_exhaustive()
    }
}

type OperationResult = Result<(), AccountNotificationsFailureKind>;

impl AccountActor {
    pub(super) async fn handle_account_notifications(
        &mut self,
        request_id: RequestId,
        request: AccountNotificationsRequest,
    ) {
        match request {
            AccountNotificationsRequest::Load => self.load_account_notifications(request_id).await,
            AccountNotificationsRequest::SetCategory { category, enabled } => {
                let operation = AccountNotificationsOperation::SetCategory { category, enabled };
                let Some(session) = self.notifications_session(request_id, operation).await else {
                    return;
                };
                let result =
                    koushi_sdk::set_notification_category(&session, category, enabled).await;
                self.finish_notifications_operation(request_id, operation, result)
                    .await;
            }
            AccountNotificationsRequest::SetAccountPush { enabled } => {
                let operation = AccountNotificationsOperation::SetAccountPush { enabled };
                let Some(session) = self.notifications_session(request_id, operation).await else {
                    return;
                };
                let result = koushi_sdk::set_account_push_enabled(&session, enabled).await;
                self.finish_notifications_operation(request_id, operation, result)
                    .await;
            }
            AccountNotificationsRequest::RequestEmailToken { address, lang } => {
                self.request_notification_email_token(request_id, address, lang)
                    .await;
            }
            AccountNotificationsRequest::ResendEmailToken => {
                self.resend_notification_email_token(request_id).await;
            }
            AccountNotificationsRequest::ConfirmEmail => {
                self.confirm_notification_email(request_id, None).await;
            }
            AccountNotificationsRequest::SubmitUia { flow_id, auth } => {
                let matches = self
                    .pending_notification_email
                    .as_ref()
                    .and_then(|pending| pending.uia.as_ref())
                    .is_some_and(|uia| uia.flow_id == flow_id);
                if !matches {
                    drop(auth);
                    self.emit_failure(
                        request_id,
                        CoreFailure::AccountOperationFailed {
                            kind: AuthFailureKind::Sdk,
                        },
                    );
                    return;
                }
                let flow_request_id = RequestId {
                    connection_id: request_id.connection_id,
                    sequence: flow_id,
                };
                self.confirm_notification_email(flow_request_id, Some(auth))
                    .await;
            }
            AccountNotificationsRequest::CancelPendingEmail => {
                // The reducer clears its display fact through the runtime's
                // command projection; the secret continuation dies here.
                self.pending_notification_email = None;
            }
            AccountNotificationsRequest::EnableEmailNotifications { address, lang } => {
                let operation = AccountNotificationsOperation::EnableEmailNotifications;
                let Some(session) = self.notifications_session(request_id, operation).await else {
                    return;
                };
                let result = match normalize_notification_email(&address) {
                    Some(address) => {
                        koushi_sdk::set_email_notification_target(&session, &address, &lang).await
                    }
                    None => Err(AccountNotificationsFailureKind::InvalidEmail),
                };
                self.finish_notifications_operation(request_id, operation, result)
                    .await;
            }
            AccountNotificationsRequest::DisableEmailNotifications => {
                let operation = AccountNotificationsOperation::DisableEmailNotifications;
                let Some(session) = self.notifications_session(request_id, operation).await else {
                    return;
                };
                let result = koushi_sdk::disable_email_notifications(&session).await;
                self.finish_notifications_operation(request_id, operation, result)
                    .await;
            }
        }
    }

    async fn load_account_notifications(&mut self, request_id: RequestId) {
        let Some(session) = self.session.clone() else {
            self.send_actions(vec![AppAction::AccountNotificationsLoadFailed {
                request_id: request_id.sequence,
                failure_kind: AccountNotificationsFailureKind::SessionRequired,
            }])
            .await;
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };
        let action = match koushi_sdk::load_account_notifications(&session).await {
            Ok(snapshot) => AppAction::AccountNotificationsLoaded {
                request_id: request_id.sequence,
                snapshot,
            },
            Err(failure_kind) => AppAction::AccountNotificationsLoadFailed {
                request_id: request_id.sequence,
                failure_kind,
            },
        };
        self.send_actions(vec![action]).await;
    }

    /// Resolve the active session, or settle the operation as failed.
    async fn notifications_session(
        &mut self,
        request_id: RequestId,
        operation: AccountNotificationsOperation,
    ) -> Option<std::sync::Arc<koushi_sdk::MatrixClientSession>> {
        if let Some(session) = self.session.clone() {
            return Some(session);
        }
        self.send_actions(vec![AppAction::AccountNotificationsOperationFailed {
            request_id: request_id.sequence,
            operation,
            failure_kind: AccountNotificationsFailureKind::SessionRequired,
            snapshot: None,
        }])
        .await;
        self.emit_failure(request_id, CoreFailure::SessionRequired);
        None
    }

    /// Settle an operation with a fresh authoritative snapshot. A failed
    /// write may still have changed part of the server state (for example
    /// the new email pusher was added but removing the old one failed), so
    /// the re-read is taken on both paths.
    async fn finish_notifications_operation(
        &mut self,
        request_id: RequestId,
        operation: AccountNotificationsOperation,
        result: OperationResult,
    ) {
        let snapshot = match &self.session {
            Some(session) => koushi_sdk::load_account_notifications(session).await.ok(),
            None => None,
        };
        let action = match result {
            Ok(()) => AppAction::AccountNotificationsOperationSucceeded {
                request_id: request_id.sequence,
                operation,
                snapshot,
            },
            Err(failure_kind) => AppAction::AccountNotificationsOperationFailed {
                request_id: request_id.sequence,
                operation,
                failure_kind,
                snapshot,
            },
        };
        self.send_actions(vec![action]).await;
    }

    async fn request_notification_email_token(
        &mut self,
        request_id: RequestId,
        address: String,
        lang: String,
    ) {
        let operation = AccountNotificationsOperation::RequestEmailToken;
        let Some(session) = self.notifications_session(request_id, operation).await else {
            return;
        };
        let Some(address) = normalize_notification_email(&address) else {
            self.send_notifications_failure(
                request_id,
                operation,
                AccountNotificationsFailureKind::InvalidEmail,
            )
            .await;
            return;
        };
        let client_secret = matrix_sdk::ruma::ClientSecret::new();
        let send_attempt = 1;
        match koushi_sdk::request_notification_email_token(
            &session,
            &client_secret,
            &address,
            send_attempt,
        )
        .await
        {
            Ok(sid) => {
                // A new request replaces any earlier pending address.
                self.pending_notification_email = Some(PendingNotificationEmailVerification {
                    address: address.clone(),
                    lang,
                    client_secret,
                    sid,
                    send_attempt,
                    resend_count: 0,
                    uia: None,
                });
                self.send_actions(vec![AppAction::AccountNotificationsEmailTokenSent {
                    request_id: request_id.sequence,
                    operation,
                    address,
                    resend_count: 0,
                }])
                .await;
            }
            Err(failure_kind) => {
                self.send_notifications_failure(request_id, operation, failure_kind)
                    .await;
            }
        }
    }

    async fn resend_notification_email_token(&mut self, request_id: RequestId) {
        let operation = AccountNotificationsOperation::ResendEmailToken;
        let Some(session) = self.notifications_session(request_id, operation).await else {
            return;
        };
        let Some(pending) = self.pending_notification_email.as_ref() else {
            self.send_notifications_failure(
                request_id,
                operation,
                AccountNotificationsFailureKind::EmailNotVerified,
            )
            .await;
            return;
        };
        let send_attempt = pending.send_attempt.saturating_add(1);
        let result = koushi_sdk::request_notification_email_token(
            &session,
            &pending.client_secret,
            &pending.address,
            send_attempt,
        )
        .await;
        match result {
            Ok(sid) => {
                let Some(pending) = self.pending_notification_email.as_mut() else {
                    return;
                };
                pending.sid = sid;
                pending.send_attempt = send_attempt;
                pending.resend_count = pending.resend_count.saturating_add(1);
                pending.uia = None;
                let action = AppAction::AccountNotificationsEmailTokenSent {
                    request_id: request_id.sequence,
                    operation,
                    address: pending.address.clone(),
                    resend_count: pending.resend_count,
                };
                self.send_actions(vec![action]).await;
            }
            Err(failure_kind) => {
                self.send_notifications_failure(request_id, operation, failure_kind)
                    .await;
            }
        }
    }

    /// `POST /account/3pid/add`, with UIA continuation. `request_id` is the
    /// original confirm request (also the UIA flow id on resubmission).
    async fn confirm_notification_email(
        &mut self,
        request_id: RequestId,
        auth: Option<koushi_state::IdentityResetAuthRequest>,
    ) {
        let operation = AccountNotificationsOperation::ConfirmEmail;
        let Some(session) = self.notifications_session(request_id, operation).await else {
            return;
        };
        let Some(pending) = self.pending_notification_email.as_ref() else {
            self.send_notifications_failure(
                request_id,
                operation,
                AccountNotificationsFailureKind::EmailNotVerified,
            )
            .await;
            return;
        };
        let uiaa_session = if auth.is_some() {
            pending.uia.as_ref().and_then(|uia| uia.session.clone())
        } else {
            None
        };
        let result = koushi_sdk::add_notification_email(
            &session,
            &pending.client_secret,
            &pending.sid,
            auth.as_ref(),
            uiaa_session.as_deref(),
        )
        .await;
        let submitted_auth = auth.is_some();
        drop(auth);
        match result {
            Ok(()) => {
                let Some(pending) = self.pending_notification_email.take() else {
                    return;
                };
                // "Change": if email notifications were active, move the
                // single target to the newly verified address (add, then
                // remove the old pusher). Otherwise the user turns email
                // notifications on explicitly.
                let carry_over = match koushi_sdk::email_notifications_active(&session).await {
                    Ok(true) => {
                        koushi_sdk::set_email_notification_target(
                            &session,
                            &pending.address,
                            &pending.lang,
                        )
                        .await
                    }
                    Ok(false) => Ok(()),
                    Err(kind) => Err(kind),
                };
                self.finish_notifications_operation(request_id, operation, carry_over)
                    .await;
            }
            Err(koushi_sdk::AddNotificationEmailError::UiaaChallenge { session: uia }) => {
                if submitted_auth {
                    // Credentials were supplied but the server asked again:
                    // report rejection and restart UIA on the next confirm.
                    if let Some(pending) = self.pending_notification_email.as_mut() {
                        pending.uia = None;
                    }
                    self.send_notifications_failure(
                        request_id,
                        operation,
                        AccountNotificationsFailureKind::AuthRejected,
                    )
                    .await;
                    return;
                }
                let flow_id = request_id.sequence;
                if let Some(pending) = self.pending_notification_email.as_mut() {
                    pending.uia = Some(PendingEmailUia {
                        flow_id,
                        session: uia,
                    });
                }
                self.send_actions(vec![AppAction::AccountNotificationsUiaRequired {
                    request_id: request_id.sequence,
                    flow_id,
                    operation,
                }])
                .await;
            }
            Err(koushi_sdk::AddNotificationEmailError::Failed(failure_kind)) => {
                if let Some(pending) = self.pending_notification_email.as_mut() {
                    pending.uia = None;
                }
                self.send_notifications_failure(request_id, operation, failure_kind)
                    .await;
            }
        }
    }

    async fn send_notifications_failure(
        &self,
        request_id: RequestId,
        operation: AccountNotificationsOperation,
        failure_kind: AccountNotificationsFailureKind,
    ) {
        self.send_actions(vec![AppAction::AccountNotificationsOperationFailed {
            request_id: request_id.sequence,
            operation,
            failure_kind,
            snapshot: None,
        }])
        .await;
    }
}
