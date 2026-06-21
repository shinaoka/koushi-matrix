use crate::{
    effect::{AppEffect, UiEvent},
    state::{
        AppState, AttachmentScope, FilesViewScope, FilesViewState, SearchCrawlerFailureKind,
        SearchCrawlerRoomState, SearchCrawlerSpeed, SearchState,
    },
};

use super::is_session_ready;

pub(crate) fn handle_search_edited(
    state: &mut AppState,
    query: String,
    scope: crate::state::SearchScope,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    state.search = SearchState::Editing { query, scope };
    vec![AppEffect::EmitUiEvent(UiEvent::SearchChanged)]
}

pub(crate) fn handle_search_submitted(
    state: &mut AppState,
    request_id: u64,
    query: String,
    scope: crate::state::SearchScope,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    state.search = SearchState::Searching {
        request_id,
        query: query.clone(),
        scope: scope.clone(),
    };
    vec![
        AppEffect::SearchMessages {
            request_id,
            query,
            scope,
        },
        AppEffect::EmitUiEvent(UiEvent::SearchChanged),
    ]
}

pub(crate) fn handle_search_succeeded(
    state: &mut AppState,
    request_id: u64,
    results: Vec<crate::state::SearchResult>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let (current_request_id, query, scope) = match &state.search {
        SearchState::Searching {
            request_id,
            query,
            scope,
        } => (*request_id, query.clone(), scope.clone()),
        _ => return Vec::new(),
    };

    if current_request_id != request_id {
        return Vec::new();
    }

    state.search = SearchState::Results {
        request_id,
        query,
        scope,
        results,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::SearchChanged)]
}

pub(crate) fn handle_search_failed(
    state: &mut AppState,
    request_id: u64,
    message: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let (current_request_id, query, scope) = match &state.search {
        SearchState::Searching {
            request_id,
            query,
            scope,
        } => (*request_id, query.clone(), scope.clone()),
        _ => return Vec::new(),
    };

    if current_request_id != request_id {
        return Vec::new();
    }

    state.search = SearchState::Failed {
        request_id,
        query,
        scope,
        message,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::SearchChanged)]
}

pub(crate) fn handle_search_index_rebuild_requested(state: &mut AppState) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    state.search = SearchState::Closed;
    state.search_crawler.rooms = state
        .rooms
        .iter()
        .map(|room| (room.room_id.clone(), SearchCrawlerRoomState::Idle))
        .collect();

    let mut effects = vec![
        AppEffect::RebuildSearchIndex,
        AppEffect::EmitUiEvent(UiEvent::SearchChanged),
        AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged),
    ];

    let settings = state.settings.values.search_crawler.clone();
    if settings.speed != SearchCrawlerSpeed::Paused {
        let room_ids: Vec<String> = state
            .rooms
            .iter()
            .map(|room| room.room_id.clone())
            .collect();
        if !room_ids.is_empty() {
            effects.push(AppEffect::NotifySearchCrawlerRoomsAvailable { room_ids, settings });
        }
    }

    effects
}

pub(crate) fn handle_history_crawl_started(
    state: &mut AppState,
    room_id: String,
) -> Vec<AppEffect> {
    state.search_crawler.rooms.insert(
        room_id,
        crate::state::SearchCrawlerRoomState::Queued,
    );
    vec![AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged)]
}

pub(crate) fn handle_history_crawl_progress(
    state: &mut AppState,
    room_id: String,
    processed: u64,
    indexed: u64,
) -> Vec<AppEffect> {
    state.search_crawler.rooms.insert(
        room_id,
        crate::state::SearchCrawlerRoomState::Running { processed, indexed },
    );
    vec![AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged)]
}

pub(crate) fn handle_history_crawl_completed(
    state: &mut AppState,
    room_id: String,
    indexed: u64,
) -> Vec<AppEffect> {
    state.search_crawler.rooms.insert(
        room_id,
        crate::state::SearchCrawlerRoomState::Completed { indexed },
    );
    vec![AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged)]
}

pub(crate) fn handle_history_crawl_failed(
    state: &mut AppState,
    room_id: String,
    kind: SearchCrawlerFailureKind,
) -> Vec<AppEffect> {
    state.search_crawler.rooms.insert(
        room_id,
        crate::state::SearchCrawlerRoomState::Failed { kind },
    );
    vec![AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged)]
}

pub(crate) fn handle_files_view_opened(
    state: &mut AppState,
    request_id: u64,
    scope: FilesViewScope,
    filter: crate::state::AttachmentFilter,
    sort: crate::state::AttachmentSort,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let scope = resolve_files_view_scope(state, scope);
    state.files_view = FilesViewState::Loading {
        request_id,
        scope: scope.clone(),
        filter: filter.clone(),
        sort,
    };
    vec![
        AppEffect::SearchAttachments {
            request_id,
            scope,
            filter,
            sort,
        },
        AppEffect::EmitUiEvent(UiEvent::FilesViewChanged),
    ]
}

pub(crate) fn handle_files_view_closed(state: &mut AppState) -> Vec<AppEffect> {
    if state.files_view == FilesViewState::Closed {
        return Vec::new();
    }

    state.files_view = FilesViewState::Closed;
    vec![AppEffect::EmitUiEvent(UiEvent::FilesViewChanged)]
}

pub(crate) fn handle_files_view_query_requested(
    state: &mut AppState,
    request_id: u64,
    scope: AttachmentScope,
    filter: crate::state::AttachmentFilter,
    sort: crate::state::AttachmentSort,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    state.files_view = FilesViewState::Loading {
        request_id,
        scope: scope.clone(),
        filter: filter.clone(),
        sort,
    };
    vec![
        AppEffect::SearchAttachments {
            request_id,
            scope,
            filter,
            sort,
        },
        AppEffect::EmitUiEvent(UiEvent::FilesViewChanged),
    ]
}

pub(crate) fn handle_files_view_query_succeeded(
    state: &mut AppState,
    request_id: u64,
    items: Vec<crate::state::AttachmentResult>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let (current_request_id, scope, filter, sort) = match &state.files_view {
        FilesViewState::Loading {
            request_id,
            scope,
            filter,
            sort,
        } => (*request_id, scope.clone(), filter.clone(), *sort),
        _ => return Vec::new(),
    };

    if current_request_id != request_id {
        return Vec::new();
    }

    state.files_view = FilesViewState::Open {
        request_id,
        scope,
        filter,
        sort,
        items,
        selected_event_id: None,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::FilesViewChanged)]
}

pub(crate) fn handle_files_view_query_failed(
    state: &mut AppState,
    request_id: u64,
    message: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let (current_request_id, scope, filter, sort) = match &state.files_view {
        FilesViewState::Loading {
            request_id,
            scope,
            filter,
            sort,
        } => (*request_id, scope.clone(), filter.clone(), *sort),
        _ => return Vec::new(),
    };

    if current_request_id != request_id {
        return Vec::new();
    }

    state.files_view = FilesViewState::Failed {
        request_id,
        scope,
        filter,
        sort,
        message,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::FilesViewChanged)]
}

pub(crate) fn handle_files_view_selection_changed(
    state: &mut AppState,
    event_id: Option<String>,
) -> Vec<AppEffect> {
    if let FilesViewState::Open {
        selected_event_id, ..
    } = &mut state.files_view
    {
        if *selected_event_id == event_id {
            return Vec::new();
        }
        *selected_event_id = event_id;
        vec![AppEffect::EmitUiEvent(UiEvent::FilesViewChanged)]
    } else {
        Vec::new()
    }
}

// --- Private helpers ---

fn resolve_files_view_scope(state: &AppState, scope: FilesViewScope) -> AttachmentScope {
    match scope {
        FilesViewScope::Room { room_id } => AttachmentScope::Room { room_id },
        FilesViewScope::Space { space_id } => {
            let child_room_ids = state
                .spaces
                .iter()
                .find(|space| space.space_id == space_id)
                .map(|space| space.child_room_ids.clone())
                .unwrap_or_default();
            AttachmentScope::Space {
                space_id,
                child_room_ids,
            }
        }
        FilesViewScope::Account => AttachmentScope::Account,
    }
}
