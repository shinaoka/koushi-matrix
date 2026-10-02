use super::timeline::{
    build_send_read_receipt_command, build_set_fully_read_command, build_timeline_key,
    trace_tauri_timeline_command, trace_tauri_timeline_command_elapsed,
};
use super::*;

pub(super) fn build_set_typing_command(
    request_id: koushi_protocol::RequestId,
    account_key: AccountKey,
    room_id: String,
    is_typing: bool,
) -> CoreCommand {
    CoreCommand::Timeline(TimelineCommand::SetTyping {
        request_id,
        key: build_timeline_key(account_key, room_id),
        is_typing,
    })
}

pub(super) fn build_set_presence_command(
    request_id: koushi_protocol::RequestId,
    presence: PresenceKind,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::SetPresence {
        request_id,
        presence,
    })
}

#[tauri::command]
pub async fn send_read_receipt(
    account_tab_id: Option<String>,
    room_id: String,
    event_id: String,
    thread_root_event_id: Option<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<(), String> {
    let account_key = account_key_from_snapshot(state.inner(), account_tab_id.as_deref()).await?;
    let request_id = next_request_id_for(state.inner(), account_tab_id.as_deref()).await?;
    let trace_started = std::time::Instant::now();
    trace_tauri_timeline_command("submit", "send_read_receipt", request_id);
    if let Some(command) = build_send_read_receipt_command(
        request_id,
        account_key,
        room_id,
        event_id,
        thread_root_event_id,
    ) {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    trace_tauri_timeline_command_elapsed(
        "done",
        "send_read_receipt",
        request_id,
        trace_started.elapsed().as_millis(),
    );
    Ok(())
}

#[tauri::command]
pub async fn set_fully_read(
    account_tab_id: Option<String>,
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<(), String> {
    let account_key = account_key_from_snapshot(state.inner(), account_tab_id.as_deref()).await?;
    let request_id = next_request_id_for(state.inner(), account_tab_id.as_deref()).await?;
    let trace_started = std::time::Instant::now();
    trace_tauri_timeline_command("submit", "set_fully_read", request_id);
    if let Some(command) = build_set_fully_read_command(request_id, account_key, room_id, event_id)
    {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    trace_tauri_timeline_command_elapsed(
        "done",
        "set_fully_read",
        request_id,
        trace_started.elapsed().as_millis(),
    );
    Ok(())
}

#[tauri::command]
pub async fn set_typing(
    account_tab_id: Option<String>,
    room_id: String,
    is_typing: bool,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<(), String> {
    let account_key = account_key_from_snapshot(state.inner(), account_tab_id.as_deref()).await?;
    let request_id = next_request_id_for(state.inner(), account_tab_id.as_deref()).await?;
    submit_core_command(
        state.inner(),
        build_set_typing_command(request_id, account_key, room_id, is_typing),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(())
}

#[tauri::command]
pub async fn set_presence(
    account_tab_id: Option<String>,
    presence: PresenceKind,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id_for(state.inner(), account_tab_id.as_deref()).await?;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_set_presence_command(request_id, presence),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}
