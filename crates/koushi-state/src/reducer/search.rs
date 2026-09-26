use crate::{
    effect::{AppEffect, UiEvent},
    state::{
        AppState, AttachmentScope, FilesViewScope, FilesViewState, SearchCrawlerFailureKind,
        SearchCrawlerLastActive, SearchCrawlerLastActiveStatus, SearchCrawlerRoomState,
        SearchCrawlerSpeed, SearchResult, SearchRoomFilter, SearchScope, SearchState,
        search_query_too_short,
    },
};

use super::{is_session_ready, session_user_id};

/// Joined room ids plus each room's latest event id, the payload of
/// `AppEffect::NotifySearchCrawlerRoomsAvailable` (#996 catch-up).
pub(crate) fn search_crawler_rooms(
    state: &AppState,
) -> (Vec<String>, std::collections::BTreeMap<String, String>) {
    let room_ids = state
        .rooms
        .iter()
        .map(|room| room.room_id.clone())
        .collect();
    let latest_event_ids = state
        .rooms
        .iter()
        .filter_map(|room| {
            room.latest_event
                .as_ref()
                .map(|latest| (room.room_id.clone(), latest.event_id.clone()))
        })
        .collect();
    (room_ids, latest_event_ids)
}

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
    scope: SearchScope,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    if let Some(min_chars) = search_query_too_short(&query) {
        state.search = SearchState::TooShort {
            request_id,
            query,
            scope,
            min_chars,
        };
        return vec![AppEffect::EmitUiEvent(UiEvent::SearchChanged)];
    }

    let room_filter = search_room_filter(state, &scope);
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
            room_filter,
        },
        AppEffect::EmitUiEvent(UiEvent::SearchChanged),
    ]
}

pub(crate) fn handle_search_succeeded(
    state: &mut AppState,
    request_id: u64,
    response_query: String,
    response_scope: crate::state::SearchScope,
    results: Vec<crate::state::SearchResult>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let (current_request_id, current_query, current_scope) = match &state.search {
        SearchState::Searching {
            request_id,
            query,
            scope,
        }
        | SearchState::Results {
            request_id,
            query,
            scope,
            ..
        } => (*request_id, query.clone(), scope.clone()),
        _ => return Vec::new(),
    };

    if current_request_id != request_id
        || response_query != current_query
        || response_scope != current_scope
    {
        return Vec::new();
    }

    let results = attach_search_context_labels(state, &current_scope, results);
    state.search = SearchState::Results {
        request_id,
        query: current_query,
        scope: current_scope,
        results,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::SearchChanged)]
}

pub(crate) fn handle_search_failed(
    state: &mut AppState,
    request_id: u64,
    response_query: String,
    response_scope: crate::state::SearchScope,
    message: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let (current_request_id, current_query, current_scope) = match &state.search {
        SearchState::Searching {
            request_id,
            query,
            scope,
        } => (*request_id, query.clone(), scope.clone()),
        _ => return Vec::new(),
    };

    if current_request_id != request_id
        || response_query != current_query
        || response_scope != current_scope
    {
        return Vec::new();
    }

    state.search = SearchState::Failed {
        request_id,
        query: current_query,
        scope: current_scope,
        message,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::SearchChanged)]
}

pub(crate) fn handle_search_closed(state: &mut AppState) -> Vec<AppEffect> {
    if !is_session_ready(state) || state.search == SearchState::Closed {
        return Vec::new();
    }

    state.search = SearchState::Closed;
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
    state.search_crawler.last_active = None;

    let mut effects = vec![
        AppEffect::RebuildSearchIndex,
        AppEffect::EmitUiEvent(UiEvent::SearchChanged),
        AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged),
    ];

    let settings = state.settings.values.search_crawler.clone();
    if settings.speed != SearchCrawlerSpeed::Paused {
        let (room_ids, latest_event_ids) = search_crawler_rooms(state);
        if !room_ids.is_empty() {
            effects.push(AppEffect::NotifySearchCrawlerRoomsAvailable {
                room_ids,
                latest_event_ids,
                settings,
            });
        }
    }

    effects
}

pub(crate) fn handle_history_crawl_started(
    state: &mut AppState,
    room_id: String,
    timestamp_ms: u64,
) -> Vec<AppEffect> {
    state.search_crawler.rooms.insert(
        room_id.clone(),
        crate::state::SearchCrawlerRoomState::Queued,
    );
    remember_search_crawler_activity(
        state,
        room_id,
        timestamp_ms,
        SearchCrawlerLastActiveStatus::Queued,
        0,
        0,
    );
    vec![AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged)]
}

pub(crate) fn handle_history_crawl_progress(
    state: &mut AppState,
    room_id: String,
    processed: u64,
    indexed: u64,
    timestamp_ms: u64,
) -> Vec<AppEffect> {
    if state.settings.values.search_crawler.speed == crate::state::SearchCrawlerSpeed::Paused {
        return Vec::new();
    }

    state.search_crawler.rooms.insert(
        room_id.clone(),
        crate::state::SearchCrawlerRoomState::Running { processed, indexed },
    );
    remember_search_crawler_activity(
        state,
        room_id,
        timestamp_ms,
        SearchCrawlerLastActiveStatus::Running,
        processed,
        indexed,
    );
    vec![AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged)]
}

pub(crate) fn handle_history_crawl_stopped(
    state: &mut AppState,
    room_id: String,
) -> Vec<AppEffect> {
    state
        .search_crawler
        .rooms
        .insert(room_id, crate::state::SearchCrawlerRoomState::Idle);
    vec![AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged)]
}

pub(crate) fn handle_history_crawl_completed(
    state: &mut AppState,
    room_id: String,
    indexed: u64,
    timestamp_ms: u64,
) -> Vec<AppEffect> {
    state.search_crawler.rooms.insert(
        room_id.clone(),
        crate::state::SearchCrawlerRoomState::Completed { indexed },
    );
    remember_search_crawler_activity(
        state,
        room_id,
        timestamp_ms,
        SearchCrawlerLastActiveStatus::Completed,
        indexed,
        indexed,
    );
    vec![AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged)]
}

pub(crate) fn handle_history_crawl_failed(
    state: &mut AppState,
    room_id: String,
    kind: SearchCrawlerFailureKind,
    timestamp_ms: u64,
) -> Vec<AppEffect> {
    state.search_crawler.rooms.insert(
        room_id.clone(),
        crate::state::SearchCrawlerRoomState::Failed { kind },
    );
    remember_search_crawler_activity(
        state,
        room_id,
        timestamp_ms,
        SearchCrawlerLastActiveStatus::Failed,
        0,
        0,
    );
    vec![AppEffect::EmitUiEvent(UiEvent::SearchCrawlerChanged)]
}

fn remember_search_crawler_activity(
    state: &mut AppState,
    room_id: String,
    timestamp_ms: u64,
    status: SearchCrawlerLastActiveStatus,
    processed: u64,
    indexed: u64,
) {
    state.search_crawler.last_active = Some(SearchCrawlerLastActive {
        room_id,
        updated_at_ms: timestamp_ms,
        status,
        processed,
        indexed,
    });
}

fn search_room_filter(state: &AppState, scope: &SearchScope) -> SearchRoomFilter {
    match scope {
        SearchScope::AllRooms => SearchRoomFilter::AllRooms,
        SearchScope::CurrentRoom { room_id } => SearchRoomFilter::OnlyRooms(vec![room_id.clone()]),
        SearchScope::CurrentSpace { space_id } => SearchRoomFilter::OnlyRooms(
            state
                .rooms
                .iter()
                .filter(|room| {
                    room.parent_space_ids
                        .iter()
                        .any(|candidate| candidate == space_id)
                        || room
                            .dm_space_ids
                            .iter()
                            .any(|candidate| candidate == space_id)
                })
                .map(|room| room.room_id.clone())
                .collect(),
        ),
    }
}

fn attach_search_context_labels(
    state: &AppState,
    scope: &SearchScope,
    results: Vec<SearchResult>,
) -> Vec<SearchResult> {
    results
        .into_iter()
        .map(|mut result| {
            result.context_label = search_result_context_label(state, scope, &result.room_id);
            result
        })
        .collect()
}

fn search_result_context_label(
    state: &AppState,
    scope: &SearchScope,
    room_id: &str,
) -> Option<String> {
    let room = state.rooms.iter().find(|room| room.room_id == room_id)?;
    let room_label = room_result_label(room);
    match search_result_space_label(state, scope, room) {
        Some(space_label) => Some(format!("{space_label} · {room_label}")),
        None => Some(room_label),
    }
}

fn room_result_label(room: &crate::state::RoomSummary) -> String {
    for label in [&room.display_label, &room.display_name, &room.room_id] {
        let label = label.trim();
        if !label.is_empty() {
            return label.to_owned();
        }
    }
    "Room".to_owned()
}

fn search_result_space_label(
    state: &AppState,
    scope: &SearchScope,
    room: &crate::state::RoomSummary,
) -> Option<String> {
    // A DM is reachable through every Space its counterpart belongs to
    // (`dm_space_ids`), so a containing-Space name would both be arbitrary and
    // imply the DM is a room inside that Space. Identify the participant only.
    // The Activity projection makes the same call in `activity_row_context_label`.
    if room.is_dm {
        return None;
    }

    if let SearchScope::CurrentSpace { space_id } = scope
        && room_belongs_to_space(room, space_id)
        && let Some(label) = space_label_by_id(state, space_id)
    {
        return Some(label);
    }

    if let Some(active_space_id) = state.navigation.active_space_id.as_deref()
        && room_belongs_to_space(room, active_space_id)
        && let Some(label) = space_label_by_id(state, active_space_id)
    {
        return Some(label);
    }

    state
        .spaces
        .iter()
        .filter(|space| room_belongs_to_space(room, &space.space_id))
        .find_map(|space| space_result_label(&space.display_name))
}

fn room_belongs_to_space(room: &crate::state::RoomSummary, space_id: &str) -> bool {
    room.parent_space_ids
        .iter()
        .any(|candidate| candidate == space_id)
        || room
            .dm_space_ids
            .iter()
            .any(|candidate| candidate == space_id)
}

fn space_label_by_id(state: &AppState, space_id: &str) -> Option<String> {
    state
        .spaces
        .iter()
        .find(|space| space.space_id == space_id)
        .and_then(|space| space_result_label(&space.display_name))
}

fn space_result_label(display_name: &str) -> Option<String> {
    let label = display_name.trim();
    if label.is_empty() {
        None
    } else {
        Some(label.to_owned())
    }
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
    mut items: Vec<crate::state::AttachmentResult>,
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

    let own_user_id = session_user_id(state);
    for item in &mut items {
        item.sender_label = crate::state::resolve_optional_user_display_name(
            &state.profile,
            &item.sender,
            item.sender_label.as_deref(),
            own_user_id,
        );
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
