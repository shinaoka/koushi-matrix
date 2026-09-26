//! Reducer for account notification settings (#981).
//!
//! State-machine contract: docs/architecture/state-machine.md,
//! "Account Notification Settings". Loads are read-only; only
//! `AccountNotificationsOperationRequested` reflects a user-initiated write,
//! and every completion replaces the snapshot with the actor's post-write
//! server re-read instead of applying the requested value optimistically.

use crate::{
    effect::{AppEffect, UiEvent},
    state::{
        AccountNotificationsFailureKind, AccountNotificationsLoadState,
        AccountNotificationsOperation, AccountNotificationsOperationState,
        AccountNotificationsSnapshot, AppState, PendingNotificationEmail,
    },
};

use super::is_session_ready;

fn changed() -> Vec<AppEffect> {
    vec![AppEffect::EmitUiEvent(UiEvent::AccountNotificationsChanged)]
}

pub(crate) fn handle_load_requested(state: &mut AppState, request_id: u64) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    state.account_notifications.load = AccountNotificationsLoadState::Loading { request_id };
    changed()
}

pub(crate) fn handle_loaded(
    state: &mut AppState,
    request_id: u64,
    snapshot: AccountNotificationsSnapshot,
) -> Vec<AppEffect> {
    if state.account_notifications.load != (AccountNotificationsLoadState::Loading { request_id }) {
        return Vec::new();
    }
    apply_snapshot(state, Some(snapshot));
    changed()
}

pub(crate) fn handle_load_failed(
    state: &mut AppState,
    request_id: u64,
    failure_kind: AccountNotificationsFailureKind,
) -> Vec<AppEffect> {
    if state.account_notifications.load != (AccountNotificationsLoadState::Loading { request_id }) {
        return Vec::new();
    }
    // The previous authoritative snapshot (if any) is kept: it is still the
    // last confirmed server state, and the failure is shown next to it.
    state.account_notifications.load = AccountNotificationsLoadState::Failed {
        request_id,
        failure_kind,
    };
    changed()
}

pub(crate) fn handle_operation_requested(
    state: &mut AppState,
    request_id: u64,
    operation: AccountNotificationsOperation,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    // Resend/confirm need a pending address; without one the request is stale.
    if matches!(
        operation,
        AccountNotificationsOperation::ResendEmailToken
            | AccountNotificationsOperation::ConfirmEmail
    ) && state.account_notifications.pending_email.is_none()
    {
        return Vec::new();
    }
    // Latest wins: the account actor serializes writes, and each completion
    // carries a fresh server re-read, so the newest request's completion is
    // the authoritative projection.
    state.account_notifications.operation = AccountNotificationsOperationState::Working {
        request_id,
        operation,
    };
    changed()
}

pub(crate) fn handle_email_token_sent(
    state: &mut AppState,
    request_id: u64,
    operation: AccountNotificationsOperation,
    address: String,
    resend_count: u32,
) -> Vec<AppEffect> {
    if !matches!(
        operation,
        AccountNotificationsOperation::RequestEmailToken
            | AccountNotificationsOperation::ResendEmailToken
    ) || !working_matches(
        &state.account_notifications.operation,
        request_id,
        operation,
    ) {
        return Vec::new();
    }
    state.account_notifications.pending_email = Some(PendingNotificationEmail {
        address,
        resend_count,
    });
    state.account_notifications.operation = AccountNotificationsOperationState::Succeeded {
        request_id,
        operation,
    };
    changed()
}

pub(crate) fn handle_uia_required(
    state: &mut AppState,
    request_id: u64,
    flow_id: u64,
    operation: AccountNotificationsOperation,
) -> Vec<AppEffect> {
    if !matches!(
        state.account_notifications.operation,
        AccountNotificationsOperationState::Working {
            request_id: active,
            operation: active_operation,
        } if active == request_id && active_operation == operation
    ) {
        return Vec::new();
    }
    state.account_notifications.operation = AccountNotificationsOperationState::AwaitingUia {
        request_id,
        flow_id,
        operation,
    };
    changed()
}

pub(crate) fn handle_uia_submitted(
    state: &mut AppState,
    request_id: u64,
    flow_id: u64,
) -> Vec<AppEffect> {
    let operation = match &state.account_notifications.operation {
        AccountNotificationsOperationState::AwaitingUia {
            request_id: active_request_id,
            flow_id: active_flow_id,
            operation,
        } if *active_request_id == request_id && *active_flow_id == flow_id => *operation,
        _ => return Vec::new(),
    };
    state.account_notifications.operation = AccountNotificationsOperationState::Working {
        request_id,
        operation,
    };
    changed()
}

pub(crate) fn handle_operation_succeeded(
    state: &mut AppState,
    request_id: u64,
    operation: AccountNotificationsOperation,
    snapshot: Option<AccountNotificationsSnapshot>,
) -> Vec<AppEffect> {
    if !working_matches(
        &state.account_notifications.operation,
        request_id,
        operation,
    ) {
        return Vec::new();
    }
    if operation == AccountNotificationsOperation::ConfirmEmail {
        state.account_notifications.pending_email = None;
    }
    apply_snapshot(state, snapshot);
    state.account_notifications.operation = AccountNotificationsOperationState::Succeeded {
        request_id,
        operation,
    };
    changed()
}

pub(crate) fn handle_operation_failed(
    state: &mut AppState,
    request_id: u64,
    operation: AccountNotificationsOperation,
    failure_kind: AccountNotificationsFailureKind,
    snapshot: Option<AccountNotificationsSnapshot>,
) -> Vec<AppEffect> {
    if !working_matches(
        &state.account_notifications.operation,
        request_id,
        operation,
    ) {
        return Vec::new();
    }
    // The pending address is left untouched: the account actor replaces its
    // pending verification only after a successful token request and clears
    // it only after a successful confirmation, so a not-yet-verified link,
    // rejected re-authentication, or failed resend keeps the same pending
    // address the user can retry, resend, or cancel.
    apply_snapshot(state, snapshot);
    state.account_notifications.operation = AccountNotificationsOperationState::Failed {
        request_id,
        operation,
        failure_kind,
    };
    changed()
}

pub(crate) fn handle_pending_email_cancelled(state: &mut AppState) -> Vec<AppEffect> {
    if state.account_notifications.pending_email.is_none() {
        return Vec::new();
    }
    state.account_notifications.pending_email = None;
    if matches!(
        state.account_notifications.operation,
        AccountNotificationsOperationState::AwaitingUia { .. }
            | AccountNotificationsOperationState::Succeeded {
                operation: AccountNotificationsOperation::RequestEmailToken
                    | AccountNotificationsOperation::ResendEmailToken,
                ..
            }
            | AccountNotificationsOperationState::Failed {
                operation: AccountNotificationsOperation::RequestEmailToken
                    | AccountNotificationsOperation::ResendEmailToken
                    | AccountNotificationsOperation::ConfirmEmail,
                ..
            }
    ) {
        state.account_notifications.operation = AccountNotificationsOperationState::Idle;
    }
    changed()
}

pub(crate) fn handle_pending_email_verified(state: &mut AppState) -> Vec<AppEffect> {
    if state.account_notifications.pending_email.take().is_none() {
        return Vec::new();
    }
    changed()
}

fn apply_snapshot(state: &mut AppState, snapshot: Option<AccountNotificationsSnapshot>) {
    if let Some(snapshot) = snapshot {
        clear_pending_if_verified(state, &snapshot);
        state.account_notifications.snapshot = Some(snapshot);
        state.account_notifications.load = AccountNotificationsLoadState::Loaded;
    }
}

/// A pending address that the server now lists as a validated 3PID is no
/// longer pending (confirmed here, or in another client).
fn clear_pending_if_verified(state: &mut AppState, snapshot: &AccountNotificationsSnapshot) {
    let verified = state
        .account_notifications
        .pending_email
        .as_ref()
        .is_some_and(|pending| {
            snapshot
                .emails
                .iter()
                .any(|email| email.address.eq_ignore_ascii_case(&pending.address))
        });
    if verified {
        state.account_notifications.pending_email = None;
    }
}

fn working_matches(
    operation_state: &AccountNotificationsOperationState,
    request_id: u64,
    operation: AccountNotificationsOperation,
) -> bool {
    matches!(
        operation_state,
        AccountNotificationsOperationState::Working {
            request_id: active,
            operation: active_operation,
        }
        | AccountNotificationsOperationState::AwaitingUia {
            request_id: active,
            operation: active_operation,
            ..
        } if *active == request_id && *active_operation == operation
    )
}
