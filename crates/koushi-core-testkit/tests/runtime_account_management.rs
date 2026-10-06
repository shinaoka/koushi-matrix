//! Runtime integration tests covering soft-logout reauth and non-device
//! account-management UIA submission projection.

use koushi_core::runtime::CoreRuntime;
use koushi_protocol::command::{AccountCommand, CoreCommand};
use koushi_state::{
    AccountManagementOperation, AccountManagementState, AppAction, AuthSecret,
    IdentityResetAuthRequest, SessionState, SoftLogoutReauthState,
};

mod support;
use support::{restore_ready_actions, wait_for_state, wait_for_state_event};

#[tokio::test]
async fn soft_logout_reauth_command_projects_authenticating_state() {
    let (runtime, _stores) = CoreRuntime::start_isolated();
    let mut connection = runtime.attach();

    runtime.inject_actions(restore_ready_actions()).await;
    wait_for_state(&mut connection, |state| {
        matches!(state.session, SessionState::Ready(_))
    })
    .await;

    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Account(AccountCommand::SoftLogoutReauth {
            request_id,
            password: AuthSecret::new("soft-logout-secret"),
        }))
        .await
        .expect("submit soft-logout reauth");

    let snapshot = wait_for_state_event(&mut connection, |state| {
        matches!(
            state.soft_logout_reauth,
            SoftLogoutReauthState::Authenticating {
                request_id: rid,
            } if rid == request_id.sequence
        )
    })
    .await;

    assert_eq!(
        snapshot.soft_logout_reauth,
        SoftLogoutReauthState::Authenticating {
            request_id: request_id.sequence,
        }
    );
}

#[tokio::test]
async fn submit_account_management_uia_command_projects_auth_submitted_state() {
    let (runtime, _stores) = CoreRuntime::start_isolated();
    let mut connection = runtime.attach();

    runtime.inject_actions(restore_ready_actions()).await;
    wait_for_state(&mut connection, |state| {
        matches!(state.session, SessionState::Ready(_))
    })
    .await;

    runtime
        .inject_actions(vec![
            AppAction::AccountManagementRequested {
                request_id: 10,
                operation: AccountManagementOperation::DeactivateAccount,
            },
            AppAction::AccountManagementUiaRequired {
                request_id: 10,
                flow_id: 10,
                operation: AccountManagementOperation::DeactivateAccount,
            },
        ])
        .await;
    wait_for_state(&mut connection, |state| {
        matches!(
            state.account_management,
            AccountManagementState::AwaitingUia {
                request_id: 10,
                flow_id: 10,
                ..
            }
        )
    })
    .await;

    let request_id = connection.next_request_id();
    connection
        .command(CoreCommand::Account(
            AccountCommand::SubmitAccountManagementUia {
                request_id,
                flow_id: 10,
                auth: IdentityResetAuthRequest::UiaaPassword {
                    password: AuthSecret::new("uia-secret"),
                },
            },
        ))
        .await
        .expect("submit account-management UIA");

    let snapshot = wait_for_state(&mut connection, |state| {
        matches!(
            state.account_management,
            AccountManagementState::Working {
                request_id: 10,
                operation: AccountManagementOperation::DeactivateAccount,
            }
        )
    })
    .await;

    assert_eq!(
        snapshot.account_management,
        AccountManagementState::Working {
            request_id: 10,
            operation: AccountManagementOperation::DeactivateAccount,
        }
    );
}
