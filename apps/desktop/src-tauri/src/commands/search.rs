use super::*;
use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};

fn search_trace_enabled() -> bool {
    std::env::var_os("KOUSHI_SEARCH_TRACE").is_some()
}

fn search_scope_kind_trace_label(scope: SearchScopeKind) -> &'static str {
    match scope {
        SearchScopeKind::CurrentRoom => "current_room",
        SearchScopeKind::CurrentSpace => "current_space",
        SearchScopeKind::Dms => "dms",
        SearchScopeKind::AllRooms => "all_rooms",
    }
}

fn resolved_search_scope_trace_label(scope: &SearchScope) -> &'static str {
    match scope {
        SearchScope::CurrentRoom { .. } => "current_room",
        SearchScope::CurrentSpace { .. } => "current_space",
        SearchScope::Dms => "dms",
        SearchScope::AllRooms => "all_rooms",
    }
}

#[tauri::command]
pub async fn submit_search(
    query: String,
    scope: SearchScopeKind,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let search_scope = resolve_search_scope(scope, state.inner()).await;
    let mut event_conn = state.runtime.attach();
    let request_id = next_request_id(state.inner()).await;
    let trimmed_query = query.trim();
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "desktop.search", "submit")
            .field(DiagnosticField::token(
                "ui_scope",
                search_scope_kind_trace_label(scope),
            ))
            .field(DiagnosticField::token(
                "resolved_scope",
                resolved_search_scope_trace_label(&search_scope),
            ))
            .field(DiagnosticField::count(
                "query_bytes",
                trimmed_query.len() as u64,
            ))
            .field(DiagnosticField::count(
                "query_chars",
                trimmed_query.chars().count() as u64,
            ))
            .field(DiagnosticField::request_id(
                "request_id",
                request_id.connection_id.0,
                request_id.sequence,
            )),
    );
    if search_trace_enabled() {
        eprintln!(
            "koushi.search_cmd stage=submit request={} ui_scope={} resolved_scope={} query_bytes={} query_chars={}",
            request_id.sequence,
            search_scope_kind_trace_label(scope),
            resolved_search_scope_trace_label(&search_scope),
            query.trim().len(),
            query.trim().chars().count()
        );
    }
    submit_core_command(
        state.inner(),
        build_submit_search_command(request_id, query, search_scope),
    )
    .await?;
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
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(state.inner(), build_close_search_command(request_id)).await?;
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
