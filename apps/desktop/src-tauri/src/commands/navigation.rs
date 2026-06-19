use super::*;

#[tauri::command]
pub async fn select_space(
    space_id: Option<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_select_space_command(request_id, space_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn reorder_spaces(
    space_ids: Vec<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_reorder_spaces_command(request_id, space_ids),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn select_room(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let selected_room_id = room_id.clone();
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_select_room_command(request_id, room_id))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_selected_room(
        &mut event_conn,
        request_id,
        &selected_room_id,
        SELECT_ROOM_EVENT_TIMEOUT,
    )
    .await?;
    let account_key = account_key_from_snapshot(state.inner()).await;
    let subscribe_request_id = event_conn.next_request_id();
    event_conn
        .command(build_subscribe_timeline_command(
            subscribe_request_id,
            account_key,
            selected_room_id,
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn select_search_result(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let selected_room_id = room_id.clone();
    let mut event_conn = state.runtime.attach();

    let close_request_id = event_conn.next_request_id();
    event_conn
        .command(CoreCommand::App(AppCommand::CloseFocusedContext {
            request_id: close_request_id,
        }))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;

    let select_request_id = event_conn.next_request_id();
    event_conn
        .command(build_select_room_command(
            select_request_id,
            room_id.clone(),
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_selected_room(
        &mut event_conn,
        select_request_id,
        &selected_room_id,
        SELECT_ROOM_EVENT_TIMEOUT,
    )
    .await?;

    let account_key = account_key_from_snapshot(state.inner()).await;
    let subscribe_request_id = event_conn.next_request_id();
    event_conn
        .command(build_subscribe_timeline_command(
            subscribe_request_id,
            account_key,
            selected_room_id,
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;

    let open_request_id = event_conn.next_request_id();
    event_conn
        .command(CoreCommand::App(AppCommand::OpenFocusedContext {
            request_id: open_request_id,
            room_id,
            event_id,
        }))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;

    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn close_focused_context(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::App(AppCommand::CloseFocusedContext { request_id }),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn open_timeline_at_timestamp(
    room_id: String,
    timestamp_ms: u64,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let focused_room_id = room_id.clone();
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_open_timeline_at_timestamp_command(
            request_id,
            room_id,
            timestamp_ms,
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_focused_context(
        &mut event_conn,
        request_id,
        &focused_room_id,
        FOCUSED_CONTEXT_EVENT_TIMEOUT,
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn observe_timeline_viewport(
    room_id: String,
    first_visible_event_id: Option<String>,
    last_visible_event_id: Option<String>,
    at_bottom: bool,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_observe_timeline_viewport_command(
            request_id,
            account_key,
            room_id,
            first_visible_event_id,
            last_visible_event_id,
            at_bottom,
        ),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}
