use super::*;

#[tauri::command]
pub async fn query_directory(
    term: Option<String>,
    server_name: Option<String>,
    limit: Option<u32>,
    since: Option<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_query_directory_command(
            request_id,
            term,
            server_name,
            limit,
            since,
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_room_operation(
        &mut event_conn,
        request_id,
        ROOM_OPERATION_EVENT_TIMEOUT,
        |event, expected_request_id| {
            matches!(
                event,
                RoomEvent::DirectoryQueryCompleted { request_id, .. } if *request_id == expected_request_id
            )
        },
        "directory query did not complete",
        "directory query failed",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn join_directory_room(
    alias: String,
    via_server: Option<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    let Some(command) = build_join_directory_room_command(request_id, alias, via_server) else {
        update_qa_window_title_from_state(&app, state.inner()).await;
        return current_snapshot(state.inner()).await;
    };

    event_conn
        .command(command)
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    let joined_room_id =
        wait_for_room_joined(&mut event_conn, request_id, ROOM_OPERATION_EVENT_TIMEOUT).await?;
    wait_for_selected_room(
        &mut event_conn,
        request_id,
        &joined_room_id,
        SELECT_ROOM_EVENT_TIMEOUT,
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}
