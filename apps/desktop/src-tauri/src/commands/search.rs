use super::*;
use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use std::{future::Future, pin::Pin};

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

pub(crate) fn record_search_trace(
    scope: SearchScopeKind,
    search_scope: &SearchScope,
    query: &str,
    request_id: koushi_core::RequestId,
) {
    let trimmed_query = query.trim();
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "desktop.search", "submit")
            .field(DiagnosticField::token(
                "ui_scope",
                search_scope_kind_trace_label(scope),
            ))
            .field(DiagnosticField::token(
                "resolved_scope",
                resolved_search_scope_trace_label(search_scope),
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
}

#[tauri::command]
pub async fn submit_search(
    query: String,
    scope: SearchScopeKind,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let search_scope = resolve_search_scope(scope, state.inner()).await;
    submit_search_production_path(
        query,
        scope,
        search_scope,
        state.inner(),
        &ProductionSearchPathIo,
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

/// Command-body boundary used by `submit_search` and its Tauri-adapter test.
/// Keeping the runtime submission and correlated wait here exercises the same
/// production path without requiring a platform-specific `AppHandle` in the
/// mock-runtime child.
pub(crate) async fn submit_search_production_path(
    query: String,
    scope: SearchScopeKind,
    search_scope: SearchScope,
    state: &CoreRuntimeState,
    io: &impl SearchPathIo,
) -> Result<(), String> {
    let mut event_conn = state.runtime.attach();
    let request_id = next_request_id(state).await;
    record_search_trace(scope, &search_scope, &query, request_id);
    io.submit(
        state,
        build_submit_search_command(request_id, query, search_scope),
    )
    .await?;
    io.wait(&mut event_conn, request_id).await?;
    Ok(())
}

pub(crate) type SearchPathFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

pub(crate) trait SearchPathIo {
    fn submit<'a>(
        &'a self,
        state: &'a CoreRuntimeState,
        command: CoreCommand,
    ) -> SearchPathFuture<'a>;
    fn wait<'a>(
        &'a self,
        connection: &'a mut CoreConnection,
        request_id: RequestId,
    ) -> SearchPathFuture<'a>;
}

struct ProductionSearchPathIo;

impl SearchPathIo for ProductionSearchPathIo {
    fn submit<'a>(
        &'a self,
        state: &'a CoreRuntimeState,
        command: CoreCommand,
    ) -> SearchPathFuture<'a> {
        Box::pin(async move { submit_core_command(state, command).await })
    }

    fn wait<'a>(
        &'a self,
        connection: &'a mut CoreConnection,
        request_id: RequestId,
    ) -> SearchPathFuture<'a> {
        Box::pin(async move {
            wait_for_search_started(connection, request_id, SEARCH_EVENT_TIMEOUT).await
        })
    }
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
