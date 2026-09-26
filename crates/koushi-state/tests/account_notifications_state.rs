//! Reducer contract for account notification settings (#981).

use koushi_state::{
    AccountNotificationsFailureKind, AccountNotificationsLoadState, AccountNotificationsOperation,
    AccountNotificationsOperationState, AccountNotificationsSnapshot, AppAction, AppState,
    NotificationCategory, NotificationCategoryState, NotificationCategoryStates,
    NotificationEmailAddress, NotificationEmailManagement, SessionInfo, SessionState, reduce,
};

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@user:example.invalid".to_owned(),
            device_id: "DEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        ..AppState::default()
    }
}

fn snapshot(email_active: bool, group: NotificationCategoryState) -> AccountNotificationsSnapshot {
    AccountNotificationsSnapshot {
        account_push_enabled: true,
        categories: NotificationCategoryStates {
            direct_messages: NotificationCategoryState::On,
            group_messages: group,
            mentions_and_replies: NotificationCategoryState::On,
            invites: NotificationCategoryState::On,
        },
        email_management: NotificationEmailManagement::Available,
        emails: vec![NotificationEmailAddress {
            address: "user@example.invalid".to_owned(),
            notifications_active: email_active,
        }],
        unverified_email_pusher_count: 0,
    }
}

const SET_GROUP_OFF: AccountNotificationsOperation = AccountNotificationsOperation::SetCategory {
    category: NotificationCategory::GroupMessages,
    enabled: false,
};

#[test]
fn load_requires_ready_session_and_matching_request() {
    let mut state = AppState::default();
    assert!(
        reduce(
            &mut state,
            AppAction::AccountNotificationsLoadRequested { request_id: 1 }
        )
        .is_empty()
    );
    assert_eq!(
        state.account_notifications.load,
        AccountNotificationsLoadState::NotLoaded
    );

    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::AccountNotificationsLoadRequested { request_id: 1 },
    );
    // A stale completion for another request is ignored.
    reduce(
        &mut state,
        AppAction::AccountNotificationsLoaded {
            request_id: 9,
            snapshot: snapshot(true, NotificationCategoryState::On),
        },
    );
    assert!(state.account_notifications.snapshot.is_none());

    reduce(
        &mut state,
        AppAction::AccountNotificationsLoaded {
            request_id: 1,
            snapshot: snapshot(true, NotificationCategoryState::Mixed),
        },
    );
    assert_eq!(
        state.account_notifications.load,
        AccountNotificationsLoadState::Loaded
    );
    let loaded = state.account_notifications.snapshot.as_ref().unwrap();
    assert_eq!(
        loaded.categories.group_messages,
        NotificationCategoryState::Mixed,
        "a mixed server state is projected as-is"
    );
    // Duplicate completion is ignored.
    assert!(
        reduce(
            &mut state,
            AppAction::AccountNotificationsLoaded {
                request_id: 1,
                snapshot: snapshot(false, NotificationCategoryState::On),
            },
        )
        .is_empty()
    );
}

#[test]
fn load_failure_keeps_last_confirmed_snapshot() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::AccountNotificationsLoadRequested { request_id: 1 },
    );
    reduce(
        &mut state,
        AppAction::AccountNotificationsLoaded {
            request_id: 1,
            snapshot: snapshot(true, NotificationCategoryState::On),
        },
    );
    reduce(
        &mut state,
        AppAction::AccountNotificationsLoadRequested { request_id: 2 },
    );
    reduce(
        &mut state,
        AppAction::AccountNotificationsLoadFailed {
            request_id: 2,
            failure_kind: AccountNotificationsFailureKind::Network,
        },
    );
    assert!(matches!(
        state.account_notifications.load,
        AccountNotificationsLoadState::Failed {
            request_id: 2,
            failure_kind: AccountNotificationsFailureKind::Network
        }
    ));
    assert!(state.account_notifications.snapshot.is_some());
}

#[test]
fn a_requested_write_is_not_applied_until_the_server_reread_arrives() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::AccountNotificationsLoadRequested { request_id: 1 },
    );
    reduce(
        &mut state,
        AppAction::AccountNotificationsLoaded {
            request_id: 1,
            snapshot: snapshot(false, NotificationCategoryState::On),
        },
    );
    reduce(
        &mut state,
        AppAction::AccountNotificationsOperationRequested {
            request_id: 2,
            operation: SET_GROUP_OFF,
        },
    );
    assert_eq!(
        state
            .account_notifications
            .snapshot
            .as_ref()
            .unwrap()
            .categories
            .group_messages,
        NotificationCategoryState::On,
        "no optimistic OFF while the write is in flight"
    );

    reduce(
        &mut state,
        AppAction::AccountNotificationsOperationSucceeded {
            request_id: 2,
            operation: SET_GROUP_OFF,
            snapshot: Some(snapshot(false, NotificationCategoryState::Off)),
        },
    );
    assert_eq!(
        state
            .account_notifications
            .snapshot
            .as_ref()
            .unwrap()
            .categories
            .group_messages,
        NotificationCategoryState::Off
    );
    assert!(matches!(
        state.account_notifications.operation,
        AccountNotificationsOperationState::Succeeded { request_id: 2, .. }
    ));
}

#[test]
fn failed_email_enable_never_projects_on() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::AccountNotificationsLoadRequested { request_id: 1 },
    );
    reduce(
        &mut state,
        AppAction::AccountNotificationsLoaded {
            request_id: 1,
            snapshot: snapshot(false, NotificationCategoryState::On),
        },
    );
    let op = AccountNotificationsOperation::EnableEmailNotifications;
    reduce(
        &mut state,
        AppAction::AccountNotificationsOperationRequested {
            request_id: 2,
            operation: op,
        },
    );
    reduce(
        &mut state,
        AppAction::AccountNotificationsOperationFailed {
            request_id: 2,
            operation: op,
            failure_kind: AccountNotificationsFailureKind::Unsupported,
            snapshot: Some(snapshot(false, NotificationCategoryState::On)),
        },
    );
    let snapshot = state.account_notifications.snapshot.as_ref().unwrap();
    assert!(!snapshot.email_notifications_active());
    assert!(matches!(
        state.account_notifications.operation,
        AccountNotificationsOperationState::Failed {
            failure_kind: AccountNotificationsFailureKind::Unsupported,
            ..
        }
    ));
}

#[test]
fn stale_and_mismatched_operation_completions_are_ignored() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::AccountNotificationsOperationRequested {
            request_id: 5,
            operation: SET_GROUP_OFF,
        },
    );
    // Wrong request id.
    assert!(
        reduce(
            &mut state,
            AppAction::AccountNotificationsOperationSucceeded {
                request_id: 4,
                operation: SET_GROUP_OFF,
                snapshot: Some(snapshot(true, NotificationCategoryState::Off)),
            },
        )
        .is_empty()
    );
    // Wrong operation.
    assert!(
        reduce(
            &mut state,
            AppAction::AccountNotificationsOperationSucceeded {
                request_id: 5,
                operation: AccountNotificationsOperation::DisableEmailNotifications,
                snapshot: None,
            },
        )
        .is_empty()
    );
    assert!(state.account_notifications.snapshot.is_none());
    // A newer request replaces the older one (latest wins).
    reduce(
        &mut state,
        AppAction::AccountNotificationsOperationRequested {
            request_id: 6,
            operation: AccountNotificationsOperation::DisableEmailNotifications,
        },
    );
    assert!(
        reduce(
            &mut state,
            AppAction::AccountNotificationsOperationSucceeded {
                request_id: 5,
                operation: SET_GROUP_OFF,
                snapshot: None,
            },
        )
        .is_empty()
    );
}

#[test]
fn email_verification_flow_with_uia_and_resend() {
    let mut state = ready_state();
    // Resend/confirm without a pending address is stale.
    assert!(
        reduce(
            &mut state,
            AppAction::AccountNotificationsOperationRequested {
                request_id: 1,
                operation: AccountNotificationsOperation::ConfirmEmail,
            },
        )
        .is_empty()
    );

    let request = AccountNotificationsOperation::RequestEmailToken;
    reduce(
        &mut state,
        AppAction::AccountNotificationsOperationRequested {
            request_id: 2,
            operation: request,
        },
    );
    reduce(
        &mut state,
        AppAction::AccountNotificationsEmailTokenSent {
            request_id: 2,
            operation: request,
            address: "new@example.invalid".to_owned(),
            resend_count: 0,
        },
    );
    let pending = state.account_notifications.pending_email.clone().unwrap();
    assert_eq!(pending.address, "new@example.invalid");
    assert_eq!(pending.resend_count, 0);

    let resend = AccountNotificationsOperation::ResendEmailToken;
    reduce(
        &mut state,
        AppAction::AccountNotificationsOperationRequested {
            request_id: 3,
            operation: resend,
        },
    );
    reduce(
        &mut state,
        AppAction::AccountNotificationsEmailTokenSent {
            request_id: 3,
            operation: resend,
            address: "new@example.invalid".to_owned(),
            resend_count: 1,
        },
    );
    assert_eq!(
        state
            .account_notifications
            .pending_email
            .as_ref()
            .unwrap()
            .resend_count,
        1
    );

    let confirm = AccountNotificationsOperation::ConfirmEmail;
    reduce(
        &mut state,
        AppAction::AccountNotificationsOperationRequested {
            request_id: 4,
            operation: confirm,
        },
    );
    // Link not opened yet: pending stays, failure shown.
    reduce(
        &mut state,
        AppAction::AccountNotificationsOperationFailed {
            request_id: 4,
            operation: confirm,
            failure_kind: AccountNotificationsFailureKind::EmailNotVerified,
            snapshot: None,
        },
    );
    assert!(state.account_notifications.pending_email.is_some());

    reduce(
        &mut state,
        AppAction::AccountNotificationsOperationRequested {
            request_id: 5,
            operation: confirm,
        },
    );
    reduce(
        &mut state,
        AppAction::AccountNotificationsUiaRequired {
            request_id: 5,
            flow_id: 5,
            operation: confirm,
        },
    );
    assert!(matches!(
        state.account_notifications.operation,
        AccountNotificationsOperationState::AwaitingUia { flow_id: 5, .. }
    ));
    // Mismatched flow id is rejected.
    assert!(
        reduce(
            &mut state,
            AppAction::AccountNotificationsUiaSubmitted {
                request_id: 5,
                flow_id: 6,
            },
        )
        .is_empty()
    );
    reduce(
        &mut state,
        AppAction::AccountNotificationsUiaSubmitted {
            request_id: 5,
            flow_id: 5,
        },
    );
    reduce(
        &mut state,
        AppAction::AccountNotificationsOperationSucceeded {
            request_id: 5,
            operation: confirm,
            snapshot: Some(snapshot(false, NotificationCategoryState::On)),
        },
    );
    assert!(state.account_notifications.pending_email.is_none());
    assert_eq!(
        state
            .account_notifications
            .snapshot
            .as_ref()
            .unwrap()
            .emails[0]
            .address,
        "user@example.invalid"
    );
}

#[test]
fn cancel_pending_email_and_logout_reset_the_slice() {
    let mut state = ready_state();
    let request = AccountNotificationsOperation::RequestEmailToken;
    reduce(
        &mut state,
        AppAction::AccountNotificationsOperationRequested {
            request_id: 1,
            operation: request,
        },
    );
    reduce(
        &mut state,
        AppAction::AccountNotificationsEmailTokenSent {
            request_id: 1,
            operation: request,
            address: "new@example.invalid".to_owned(),
            resend_count: 0,
        },
    );
    reduce(
        &mut state,
        AppAction::AccountNotificationsPendingEmailCancelled,
    );
    assert!(state.account_notifications.pending_email.is_none());
    assert_eq!(
        state.account_notifications.operation,
        AccountNotificationsOperationState::Idle
    );

    reduce(
        &mut state,
        AppAction::AccountNotificationsLoadRequested { request_id: 2 },
    );
    reduce(
        &mut state,
        AppAction::AccountNotificationsLoaded {
            request_id: 2,
            snapshot: snapshot(true, NotificationCategoryState::On),
        },
    );
    reduce(&mut state, AppAction::LogoutRequested);
    assert_eq!(state.account_notifications, Default::default());
}
