use super::*;

#[tauri::command]
pub async fn query_devices(
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::Account(AccountCommand::QueryDevices { request_id }),
    )
    .await?;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn rename_device(
    device_ordinal: u64,
    display_name: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::Account(AccountCommand::RenameDevice {
            request_id,
            device_ordinal,
            display_name,
        }),
    )
    .await?;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn delete_devices(
    device_ordinals: Vec<u64>,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::Account(AccountCommand::DeleteDevices {
            request_id,
            device_ordinals,
            auth: None,
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
