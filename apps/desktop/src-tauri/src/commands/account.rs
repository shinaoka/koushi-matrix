use super::*;
#[cfg(test)]
use crate::commands::contracts::fake_request_id;

#[tauri::command]
pub async fn refresh_current_session_status(
    trigger: koushi_state::SessionStatusRefreshTrigger,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::Account(AccountCommand::RefreshCurrentSessionStatus {
            request_id,
            trigger,
        }),
    )
    .await?;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn load_account_management_capabilities(
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::Account(AccountCommand::LoadAccountManagementCapabilities { request_id }),
    )
    .await?;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn change_password(
    new_password: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::Account(AccountCommand::ChangePassword {
            request_id,
            new_password: AuthSecret::new(new_password),
        }),
    )
    .await?;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn deactivate_account(
    erase_data: bool,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::Account(AccountCommand::DeactivateAccount {
            request_id,
            erase_data,
        }),
    )
    .await?;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn submit_account_management_uia(
    flow_id: u64,
    password: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_submit_account_management_uia_command(request_id, flow_id, AuthSecret::new(password)),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

pub(super) fn build_start_device_cleanup_command(
    request_id: koushi_core::RequestId,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::StartDeviceCleanup { request_id })
}

pub(super) fn build_submit_device_cleanup_uia_command(
    request_id: koushi_core::RequestId,
    flow_id: u64,
    password: AuthSecret,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::SubmitDeviceCleanupUia {
        request_id,
        flow_id,
        password,
    })
}

pub(super) fn build_erase_device_cleanup_local_data_anyway_command(
    request_id: koushi_core::RequestId,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::EraseDeviceCleanupLocalDataAnyway { request_id })
}

pub(super) fn build_submit_account_management_uia_command(
    request_id: koushi_core::RequestId,
    flow_id: u64,
    password: AuthSecret,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::SubmitAccountManagementUia {
        request_id,
        flow_id,
        auth: IdentityResetAuthRequest::UiaaPassword { password },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_cleanup_commands_route_to_the_provisional_account_state_machine() {
        let request_id = fake_request_id(49);
        assert!(matches!(
            build_start_device_cleanup_command(request_id),
            CoreCommand::Account(AccountCommand::StartDeviceCleanup {
                request_id: actual
            }) if actual == request_id
        ));
        match build_submit_device_cleanup_uia_command(
            request_id,
            41,
            AuthSecret::new("private-password"),
        ) {
            CoreCommand::Account(AccountCommand::SubmitDeviceCleanupUia {
                request_id: actual,
                flow_id,
                password,
            }) => {
                assert_eq!(actual, request_id);
                assert_eq!(flow_id, 41);
                assert_eq!(password.expose_secret(), "private-password");
            }
            other => panic!("unexpected command: {other:?}"),
        }
        assert!(matches!(
            build_erase_device_cleanup_local_data_anyway_command(request_id),
            CoreCommand::Account(AccountCommand::EraseDeviceCleanupLocalDataAnyway {
                request_id: actual
            }) if actual == request_id
        ));
    }
}
