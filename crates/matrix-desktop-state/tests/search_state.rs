use matrix_desktop_state::{
    AppAction, AppEffect, AppState, SearchMatchKind, SearchResult, SearchScope, SearchState,
    SessionInfo, SessionState, TextRange, UiEvent, reduce,
};

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.org".to_owned(),
        user_id: "@alice:example.org".to_owned(),
        device_id: "DEVICE".to_owned(),
    }
}

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(session_info()),
        ..AppState::default()
    }
}

fn scope() -> SearchScope {
    SearchScope::AllRooms
}

fn result(event_id: &str) -> SearchResult {
    SearchResult {
        room_id: "room-a".to_owned(),
        event_id: event_id.to_owned(),
        sender: "@alice:example.org".to_owned(),
        timestamp_ms: 1_700_000_000_000,
        score_millis: 900,
        snippet: "再アンケートです".to_owned(),
        highlights: vec![TextRange {
            start_utf16: 1,
            end_utf16: 6,
        }],
        match_kind: SearchMatchKind::Exact,
    }
}

#[test]
fn editing_search_updates_local_state_and_emits_event() {
    let mut state = ready_state();

    let effects = reduce(
        &mut state,
        AppAction::SearchEdited {
            query: "アンケート".to_owned(),
            scope: scope(),
        },
    );

    assert_eq!(
        state.search,
        SearchState::Editing {
            query: "アンケート".to_owned(),
            scope: scope(),
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::SearchChanged)]
    );
}

#[test]
fn search_result_carries_verified_exact_highlights() {
    let result = result("$event");

    assert_eq!(result.match_kind, SearchMatchKind::Exact);
    assert_eq!(
        result.highlights,
        vec![TextRange {
            start_utf16: 1,
            end_utf16: 6,
        }]
    );
}

#[test]
fn submitting_search_emits_search_effect() {
    let mut state = ready_state();

    let effects = reduce(
        &mut state,
        AppAction::SearchSubmitted {
            request_id: 7,
            query: "アンケート".to_owned(),
            scope: scope(),
        },
    );

    assert_eq!(
        state.search,
        SearchState::Searching {
            request_id: 7,
            query: "アンケート".to_owned(),
            scope: scope(),
        }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::SearchMessages {
                request_id: 7,
                query: "アンケート".to_owned(),
                scope: scope(),
            },
            AppEffect::EmitUiEvent(UiEvent::SearchChanged),
        ]
    );
}

#[test]
fn search_actions_are_ignored_without_ready_session() {
    let mut state = AppState::default();

    assert_eq!(
        reduce(
            &mut state,
            AppAction::SearchEdited {
                query: "アンケート".to_owned(),
                scope: scope(),
            },
        ),
        Vec::new()
    );
    assert_eq!(
        reduce(
            &mut state,
            AppAction::SearchSubmitted {
                request_id: 7,
                query: "アンケート".to_owned(),
                scope: scope(),
            },
        ),
        Vec::new()
    );
    assert_eq!(
        reduce(
            &mut state,
            AppAction::SearchSucceeded {
                request_id: 7,
                results: vec![result("$event")],
            },
        ),
        Vec::new()
    );
    assert_eq!(state.search, SearchState::Closed);
}

#[test]
fn editing_search_after_submit_suppresses_previous_response() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SearchSubmitted {
            request_id: 8,
            query: "old".to_owned(),
            scope: scope(),
        },
    );
    reduce(
        &mut state,
        AppAction::SearchEdited {
            query: "new".to_owned(),
            scope: scope(),
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::SearchSucceeded {
            request_id: 8,
            results: vec![result("$old")],
        },
    );

    assert_eq!(
        state.search,
        SearchState::Editing {
            query: "new".to_owned(),
            scope: scope(),
        }
    );
    assert_eq!(effects, Vec::<AppEffect>::new());
}

#[test]
fn stale_search_result_is_ignored() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SearchSubmitted {
            request_id: 8,
            query: "new".to_owned(),
            scope: scope(),
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::SearchSucceeded {
            request_id: 7,
            results: vec![result("$old")],
        },
    );

    assert_eq!(
        state.search,
        SearchState::Searching {
            request_id: 8,
            query: "new".to_owned(),
            scope: scope(),
        }
    );
    assert_eq!(effects, Vec::<AppEffect>::new());
}

#[test]
fn matching_search_result_updates_results() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SearchSubmitted {
            request_id: 9,
            query: "アンケート".to_owned(),
            scope: scope(),
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::SearchSucceeded {
            request_id: 9,
            results: vec![result("$event")],
        },
    );

    assert_eq!(
        state.search,
        SearchState::Results {
            request_id: 9,
            query: "アンケート".to_owned(),
            scope: scope(),
            results: vec![result("$event")],
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::SearchChanged)]
    );
}

#[test]
fn duplicate_search_response_after_results_is_ignored() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SearchSubmitted {
            request_id: 13,
            query: "アンケート".to_owned(),
            scope: scope(),
        },
    );
    reduce(
        &mut state,
        AppAction::SearchSucceeded {
            request_id: 13,
            results: vec![result("$event")],
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::SearchFailed {
            request_id: 13,
            message: "late failure".to_owned(),
        },
    );

    assert_eq!(
        state.search,
        SearchState::Results {
            request_id: 13,
            query: "アンケート".to_owned(),
            scope: scope(),
            results: vec![result("$event")],
        }
    );
    assert_eq!(effects, Vec::<AppEffect>::new());
}

#[test]
fn matching_search_failure_updates_failed_state() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SearchSubmitted {
            request_id: 10,
            query: "アンケート".to_owned(),
            scope: scope(),
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::SearchFailed {
            request_id: 10,
            message: "search unavailable".to_owned(),
        },
    );

    assert_eq!(
        state.search,
        SearchState::Failed {
            request_id: 10,
            query: "アンケート".to_owned(),
            scope: scope(),
            message: "search unavailable".to_owned(),
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::SearchChanged)]
    );
}

#[test]
fn stale_search_failure_is_ignored() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SearchSubmitted {
            request_id: 12,
            query: "new".to_owned(),
            scope: scope(),
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::SearchFailed {
            request_id: 11,
            message: "late failure".to_owned(),
        },
    );

    assert_eq!(
        state.search,
        SearchState::Searching {
            request_id: 12,
            query: "new".to_owned(),
            scope: scope(),
        }
    );
    assert_eq!(effects, Vec::<AppEffect>::new());
}
