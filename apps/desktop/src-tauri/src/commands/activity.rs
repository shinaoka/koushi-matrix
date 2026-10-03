use super::*;
#[tauri::command]
pub async fn open_activity(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission =
        submit_core_command_with_admission(state.inner(), build_open_activity_command(request_id))
            .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn close_activity(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission =
        submit_core_command_with_admission(state.inner(), build_close_activity_command(request_id))
            .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn set_activity_tab(
    tab: ActivityTab,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_set_activity_tab_command(request_id, tab),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn paginate_activity(
    tab: ActivityTab,
    cursor: Option<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_paginate_activity_command(request_id, tab, cursor),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn retry_activity_resolution(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_retry_activity_resolution_command(request_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn mark_activity_read(
    target: ActivityMarkReadTarget,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_mark_activity_read_command(request_id, target),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

pub(super) fn build_open_activity_command(request_id: koushi_protocol::RequestId) -> CoreCommand {
    CoreCommand::App(AppCommand::OpenActivity { request_id })
}

pub(super) fn build_close_activity_command(request_id: koushi_protocol::RequestId) -> CoreCommand {
    CoreCommand::App(AppCommand::CloseActivity { request_id })
}

pub(super) fn build_set_activity_tab_command(
    request_id: koushi_protocol::RequestId,
    tab: ActivityTab,
) -> CoreCommand {
    CoreCommand::App(AppCommand::SetActivityTab { request_id, tab })
}

pub(super) fn build_paginate_activity_command(
    request_id: koushi_protocol::RequestId,
    tab: ActivityTab,
    cursor: Option<String>,
) -> CoreCommand {
    CoreCommand::App(AppCommand::PaginateActivity {
        request_id,
        tab,
        cursor: optional_non_blank(cursor),
    })
}

pub(super) fn build_mark_activity_read_command(
    request_id: koushi_protocol::RequestId,
    target: ActivityMarkReadTarget,
) -> CoreCommand {
    CoreCommand::App(AppCommand::MarkActivityRead { request_id, target })
}

pub(super) fn build_retry_activity_resolution_command(
    request_id: koushi_protocol::RequestId,
) -> CoreCommand {
    CoreCommand::App(AppCommand::RetryActivityResolution { request_id })
}
