use super::*;
use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};

fn search_scope_kind_trace_label(scope: SearchScopeKind) -> &'static str {
    match scope {
        SearchScopeKind::CurrentRoom => "current_room",
        SearchScopeKind::CurrentSpace => "current_space",
        SearchScopeKind::AllRooms => "all_rooms",
    }
}

fn resolved_search_scope_trace_label(scope: &SearchScope) -> &'static str {
    match scope {
        SearchScope::CurrentRoom { .. } => "current_room",
        SearchScope::CurrentSpace { .. } => "current_space",
        SearchScope::AllRooms => "all_rooms",
    }
}

pub(crate) fn record_search_trace(
    scope: SearchScopeKind,
    search_scope: &SearchScope,
    query: &str,
    request_id: koushi_protocol::RequestId,
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
) -> Result<FrontendCommandSettlement, String> {
    let search_scope = resolve_search_scope(scope, state.inner()).await;
    let settlement =
        submit_search_production_path(query, scope, search_scope, state.inner()).await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(settlement)
}

/// Command-body boundary used by `submit_search` so the IPC handler remains a
/// thin adapter over Core command submission and request settlement.
pub(crate) async fn submit_search_production_path(
    query: String,
    scope: SearchScopeKind,
    search_scope: SearchScope,
    state: &CoreRuntimeState,
) -> Result<FrontendCommandSettlement, String> {
    let mut wait_conn = state.runtime.attach();
    let baseline_snapshot = wait_conn.versioned_snapshot();
    let baseline_generation = baseline_snapshot.generation;
    let account_key = account_key_from_app_state(&baseline_snapshot.state);
    let account_key = (!account_key.0.is_empty()).then_some(account_key);
    let request_id = next_request_id(state).await;
    record_search_trace(scope, &search_scope, &query, request_id);
    submit_core_command(
        state,
        build_submit_search_command(request_id, query.clone(), search_scope.clone()),
    )
    .await?;
    let outcome = wait_conn
        .wait_for_request_outcome(
            OutcomeCorrelation::Request(request_id),
            RequestOutcomeExpectation::SearchStarted {
                request_id,
                account_key,
                query,
                scope: match &search_scope {
                    SearchScope::AllRooms => koushi_state::SearchScope::AllRooms,
                    SearchScope::CurrentRoom { room_id } => {
                        koushi_state::SearchScope::CurrentRoom {
                            room_id: room_id.clone(),
                        }
                    }
                    SearchScope::CurrentSpace { space_id } => {
                        koushi_state::SearchScope::CurrentSpace {
                            space_id: space_id.clone(),
                        }
                    }
                },
            },
            baseline_generation,
            tokio::time::Instant::now() + SEARCH_EVENT_TIMEOUT,
        )
        .await
        .map_err(|error| invoke_error_from_request_outcome("search", error))?;
    match outcome {
        RequestOutcome::Search { generation, .. } => Ok(
            FrontendCommandSettlement::from_published_generation(generation),
        ),
        _ => Err("search returned an invalid outcome".to_owned()),
    }
}

#[tauri::command]
pub async fn close_search(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut wait_conn = state.inner().runtime.attach();
    let baseline_snapshot = wait_conn.versioned_snapshot();
    let baseline_generation = baseline_snapshot.generation;
    let account_key = account_key_from_app_state(&baseline_snapshot.state);
    let account_key = (!account_key.0.is_empty()).then_some(account_key);
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(state.inner(), build_close_search_command(request_id)).await?;
    let outcome = wait_conn
        .wait_for_request_outcome(
            OutcomeCorrelation::Request(request_id),
            RequestOutcomeExpectation::SearchClosed {
                request_id,
                account_key,
                allow_initial: false,
                allow_projection_only: true,
            },
            baseline_generation,
            tokio::time::Instant::now() + SEARCH_EVENT_TIMEOUT,
        )
        .await
        .map_err(|error| invoke_error_from_request_outcome("search close", error))?;
    let RequestOutcome::Search { generation, .. } = outcome else {
        return Err("search close returned an invalid outcome".to_owned());
    };
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(FrontendCommandSettlement::from_published_generation(
        generation,
    ))
}

#[tauri::command]
pub async fn start_room_crawl(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
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
    let admission = submit_core_command_with_admission(
        state.inner(),
        CoreCommand::Search(SearchCommand::StartHistoryCrawl {
            request_id,
            room_id,
            settings,
        }),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn stop_room_crawl(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        CoreCommand::Search(SearchCommand::StopHistoryCrawl {
            request_id,
            room_id,
        }),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

const SEARCH_EVENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

pub(super) fn build_submit_search_command(
    request_id: koushi_protocol::RequestId,
    query: String,
    scope: SearchScope,
) -> CoreCommand {
    CoreCommand::Search(SearchCommand::Query {
        request_id,
        query,
        scope,
        room_filter: koushi_state::SearchRoomFilter::AllRooms,
    })
}

pub(super) fn build_close_search_command(request_id: koushi_protocol::RequestId) -> CoreCommand {
    CoreCommand::App(AppCommand::CloseSearch { request_id })
}

pub(super) fn resolve_search_scope_from_active_room(
    scope: SearchScopeKind,
    active_room_id: Option<String>,
    active_space_id: Option<String>,
) -> SearchScope {
    match scope {
        SearchScopeKind::CurrentRoom => active_room_id
            .map(|room_id| SearchScope::CurrentRoom { room_id })
            .unwrap_or_else(|| SearchScope::CurrentRoom {
                room_id: String::new(),
            }),
        SearchScopeKind::CurrentSpace => active_space_id
            .map(|space_id| SearchScope::CurrentSpace { space_id })
            .unwrap_or_else(|| SearchScope::CurrentSpace {
                space_id: String::new(),
            }),
        SearchScopeKind::AllRooms => SearchScope::AllRooms,
    }
}

async fn resolve_search_scope(
    scope: SearchScopeKind,
    state: &CoreRuntimeState,
) -> koushi_protocol::SearchScope {
    let snapshot = state.connection.lock().await.snapshot();
    resolve_search_scope_from_active_room(
        scope,
        snapshot.navigation.active_room_id,
        snapshot.navigation.active_space_id,
    )
}
