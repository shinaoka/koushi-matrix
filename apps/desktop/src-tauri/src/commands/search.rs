use super::*;

#[tauri::command]
pub async fn submit_search(
    query: String,
    scope: SearchScopeKind,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let search_scope = resolve_search_scope(scope, state.inner()).await;
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_submit_search_command(request_id, query, search_scope))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_search_started(&mut event_conn, request_id, SEARCH_EVENT_TIMEOUT).await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn close_search(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_close_search_command(request_id))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_search_closed(&mut event_conn, request_id, SEARCH_EVENT_TIMEOUT).await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn start_room_crawl(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    // Read current crawler settings from the Rust-owned snapshot so this
    // command doesn't duplicate settings state in the TypeScript layer.
    let settings = state
        .connection
        .lock()
        .await
        .snapshot()
        .settings
        .values
        .search_crawler
        .clone();
    submit_core_command(
        state.inner(),
        CoreCommand::Search(SearchCommand::StartHistoryCrawl {
            request_id,
            room_id,
            settings,
        }),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn stop_room_crawl(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::Search(SearchCommand::StopHistoryCrawl {
            request_id,
            room_id,
        }),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}
