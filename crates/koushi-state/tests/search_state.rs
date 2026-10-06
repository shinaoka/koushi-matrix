use koushi_state::{
    AppAction, AppEffect, AppState, RoomSummary, RoomTags, SearchMatchField, SearchMatchKind,
    SearchResult, SearchRoomFilter, SearchScope, SearchState, SessionInfo, SessionState,
    SpaceSummary, TextRange, UiEvent, reduce,
};

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.org".to_owned(),
        user_id: "@user-a:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
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
        context_label: None,
        sender: "@user-a:example.invalid".to_owned(),
        timestamp_ms: 1_700_000_000_000,
        score_millis: 900,
        snippet: "再アンケートです".to_owned(),
        match_field: SearchMatchField::MessageBody,
        highlights: vec![TextRange {
            start_utf16: 1,
            end_utf16: 6,
        }],
        match_kind: SearchMatchKind::Exact,
    }
}

fn room_summary(room_id: &str) -> RoomSummary {
    RoomSummary {
        display_name_placeholder: None,
        display_label_placeholder: None,
        room_id: room_id.to_owned(),
        display_name: room_id.to_owned(),
        display_label: room_id.to_owned(),
        original_display_label: room_id.to_owned(),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        marked_unread: false,
        recency_stamp: None,
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 0,
    }
}

fn attachment_filename_result(event_id: &str) -> SearchResult {
    SearchResult {
        room_id: "room-a".to_owned(),
        event_id: event_id.to_owned(),
        context_label: None,
        sender: "@user-a:example.invalid".to_owned(),
        timestamp_ms: 1_700_000_000_000,
        score_millis: 875,
        snippet: "seminar_schedule.pdf".to_owned(),
        match_field: SearchMatchField::AttachmentFileName,
        highlights: vec![TextRange {
            start_utf16: 8,
            end_utf16: 16,
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
fn search_result_can_identify_attachment_filename_match() {
    let result = attachment_filename_result("$file");

    assert_eq!(result.match_field, SearchMatchField::AttachmentFileName);
    assert_eq!(result.snippet, "seminar_schedule.pdf");
    assert_eq!(
        result.highlights,
        vec![TextRange {
            start_utf16: 8,
            end_utf16: 16,
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
                room_filter: SearchRoomFilter::AllRooms,
                content_policy: koushi_state::SearchCrawlerSettings::default(),
            },
            AppEffect::EmitUiEvent(UiEvent::SearchChanged),
        ]
    );
}

#[test]
fn submitting_short_ascii_search_stays_too_short_without_sdk_effect() {
    let mut state = ready_state();

    let effects = reduce(
        &mut state,
        AppAction::SearchSubmitted {
            request_id: 7,
            query: "GP".to_owned(),
            scope: scope(),
        },
    );

    assert_eq!(
        state.search,
        SearchState::TooShort {
            request_id: 7,
            query: "GP".to_owned(),
            scope: scope(),
            min_chars: 3,
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::SearchChanged)]
    );
}

#[test]
fn submitting_short_cjk_search_uses_cjk_threshold_without_sdk_effect() {
    let mut state = ready_state();

    let short_effects = reduce(
        &mut state,
        AppAction::SearchSubmitted {
            request_id: 8,
            query: "通".to_owned(),
            scope: scope(),
        },
    );

    assert_eq!(
        state.search,
        SearchState::TooShort {
            request_id: 8,
            query: "通".to_owned(),
            scope: scope(),
            min_chars: 2,
        }
    );
    assert_eq!(
        short_effects,
        vec![AppEffect::EmitUiEvent(UiEvent::SearchChanged)]
    );

    let searchable_effects = reduce(
        &mut state,
        AppAction::SearchSubmitted {
            request_id: 9,
            query: "通院".to_owned(),
            scope: scope(),
        },
    );

    assert_eq!(
        state.search,
        SearchState::Searching {
            request_id: 9,
            query: "通院".to_owned(),
            scope: scope(),
        }
    );
    assert_eq!(
        searchable_effects,
        vec![
            AppEffect::SearchMessages {
                request_id: 9,
                query: "通院".to_owned(),
                scope: scope(),
                room_filter: SearchRoomFilter::AllRooms,
                content_policy: koushi_state::SearchCrawlerSettings::default(),
            },
            AppEffect::EmitUiEvent(UiEvent::SearchChanged),
        ]
    );
}

#[test]
fn submitting_scoped_search_carries_rust_resolved_room_filter() {
    let mut state = ready_state();
    let mut space_child = room_summary("space-child");
    space_child.parent_space_ids = vec!["space-a".to_owned()];
    let mut dm_child = room_summary("dm-child");
    dm_child.is_dm = true;
    dm_child.dm_space_ids = vec!["space-a".to_owned()];
    let outside = room_summary("outside");
    state.rooms = vec![space_child, dm_child, outside];

    let space_scope = SearchScope::CurrentSpace {
        space_id: "space-a".to_owned(),
    };
    let effects = reduce(
        &mut state,
        AppAction::SearchSubmitted {
            request_id: 11,
            query: "GPT".to_owned(),
            scope: space_scope.clone(),
        },
    );

    assert_eq!(
        effects,
        vec![
            AppEffect::SearchMessages {
                request_id: 11,
                query: "GPT".to_owned(),
                scope: space_scope,
                room_filter: SearchRoomFilter::OnlyRooms(vec![
                    "space-child".to_owned(),
                    "dm-child".to_owned(),
                ]),
                content_policy: koushi_state::SearchCrawlerSettings::default(),
            },
            AppEffect::EmitUiEvent(UiEvent::SearchChanged),
        ]
    );

    let effects = reduce(
        &mut state,
        AppAction::SearchSubmitted {
            request_id: 12,
            query: "GPT".to_owned(),
            scope: SearchScope::CurrentRoom {
                room_id: "dm-child".to_owned(),
            },
        },
    );

    assert_eq!(
        effects,
        vec![
            AppEffect::SearchMessages {
                request_id: 12,
                query: "GPT".to_owned(),
                scope: SearchScope::CurrentRoom {
                    room_id: "dm-child".to_owned(),
                },
                room_filter: SearchRoomFilter::OnlyRooms(vec!["dm-child".to_owned()]),
                content_policy: koushi_state::SearchCrawlerSettings::default(),
            },
            AppEffect::EmitUiEvent(UiEvent::SearchChanged),
        ]
    );
}

#[test]
fn search_results_carry_rust_owned_space_context_label() {
    let mut state = ready_state();
    let mut room = room_summary("room-a");
    room.display_name = "Ops".to_owned();
    room.display_label = "Ops".to_owned();
    room.parent_space_ids = vec!["space-fallback".to_owned(), "space-active".to_owned()];
    state.rooms = vec![room];
    state.spaces = vec![
        SpaceSummary {
            space_id: "space-fallback".to_owned(),
            raw_name: None,
            display_name: "Fallback Space".to_owned(),
            avatar: None,
            join_rule: None,
            child_room_ids: vec!["room-a".to_owned()],
            parent_side_child_room_ids: vec!["room-a".to_owned()],
        },
        SpaceSummary {
            space_id: "space-active".to_owned(),
            raw_name: None,
            display_name: "Active Space".to_owned(),
            avatar: None,
            join_rule: None,
            child_room_ids: vec!["room-a".to_owned()],
            parent_side_child_room_ids: vec!["room-a".to_owned()],
        },
    ];
    state.navigation.active_space_id = Some("space-active".to_owned());
    let search_scope = SearchScope::CurrentSpace {
        space_id: "space-fallback".to_owned(),
    };

    reduce(
        &mut state,
        AppAction::SearchSubmitted {
            request_id: 13,
            query: "GPT".to_owned(),
            scope: search_scope.clone(),
        },
    );
    reduce(
        &mut state,
        AppAction::SearchSucceeded {
            request_id: 13,
            query: "GPT".to_owned(),
            scope: search_scope,
            results: vec![result("$event")],
        },
    );

    let SearchState::Results { results, .. } = &state.search else {
        panic!("expected search results");
    };
    assert_eq!(
        results[0].context_label,
        Some("Fallback Space · Ops".to_owned())
    );
}

#[test]
fn dm_search_results_never_carry_a_space_context_label() {
    let mut state = ready_state();
    let mut dm = room_summary("dm-child");
    dm.display_name = "Satoshi Terasaki".to_owned();
    dm.display_label = "Satoshi Terasaki".to_owned();
    dm.is_dm = true;
    dm.dm_user_ids = vec!["@satoshi:example.invalid".to_owned()];
    // A DM is reachable through every space its counterpart belongs to, and may
    // additionally be an explicit space child. Neither makes it a room inside
    // that space.
    dm.dm_space_ids = vec!["space-active".to_owned()];
    dm.parent_space_ids = vec!["space-fallback".to_owned()];
    state.rooms = vec![dm];
    state.spaces = vec![
        SpaceSummary {
            space_id: "space-fallback".to_owned(),
            raw_name: None,
            display_name: "Fallback Space".to_owned(),
            avatar: None,
            join_rule: None,
            child_room_ids: vec!["dm-child".to_owned()],
            parent_side_child_room_ids: vec!["dm-child".to_owned()],
        },
        SpaceSummary {
            space_id: "space-active".to_owned(),
            raw_name: None,
            display_name: "CompPhysHack2026".to_owned(),
            avatar: None,
            join_rule: None,
            child_room_ids: vec!["dm-child".to_owned()],
            parent_side_child_room_ids: vec!["dm-child".to_owned()],
        },
    ];
    state.navigation.active_space_id = Some("space-active".to_owned());

    for search_scope in [
        SearchScope::CurrentRoom {
            room_id: "dm-child".to_owned(),
        },
        SearchScope::CurrentSpace {
            space_id: "space-active".to_owned(),
        },
        SearchScope::AllRooms,
    ] {
        let mut dm_result = result("$event");
        dm_result.room_id = "dm-child".to_owned();
        reduce(
            &mut state,
            AppAction::SearchSubmitted {
                request_id: 21,
                query: "GPT".to_owned(),
                scope: search_scope.clone(),
            },
        );
        reduce(
            &mut state,
            AppAction::SearchSucceeded {
                request_id: 21,
                query: "GPT".to_owned(),
                scope: search_scope.clone(),
                results: vec![dm_result],
            },
        );

        let SearchState::Results { results, .. } = &state.search else {
            panic!("expected search results for {search_scope:?}");
        };
        assert_eq!(
            results[0].context_label,
            Some("Satoshi Terasaki".to_owned()),
            "DM result must identify the participant without a Space name for {search_scope:?}"
        );
    }
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
                query: "アンケート".to_owned(),
                scope: scope(),
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
            query: "old".to_owned(),
            scope: scope(),
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
            query: "old".to_owned(),
            scope: scope(),
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
fn same_sequence_search_result_for_different_query_is_ignored() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SearchSubmitted {
            request_id: 1,
            query: "gpt".to_owned(),
            scope: scope(),
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::SearchSucceeded {
            request_id: 1,
            query: "pt".to_owned(),
            scope: scope(),
            results: vec![result("$pt")],
        },
    );

    assert_eq!(
        state.search,
        SearchState::Searching {
            request_id: 1,
            query: "gpt".to_owned(),
            scope: scope(),
        }
    );
    assert_eq!(effects, Vec::<AppEffect>::new());
}

#[test]
fn same_sequence_search_failure_for_different_scope_is_ignored() {
    let mut state = ready_state();
    let current_scope = SearchScope::CurrentRoom {
        room_id: "room-a".to_owned(),
    };
    reduce(
        &mut state,
        AppAction::SearchSubmitted {
            request_id: 1,
            query: "gpt".to_owned(),
            scope: current_scope.clone(),
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::SearchFailed {
            request_id: 1,
            query: "gpt".to_owned(),
            scope: SearchScope::AllRooms,
            message: "late failure".to_owned(),
        },
    );

    assert_eq!(
        state.search,
        SearchState::Searching {
            request_id: 1,
            query: "gpt".to_owned(),
            scope: current_scope,
        }
    );
    assert_eq!(effects, Vec::<AppEffect>::new());
}

#[test]
fn a_submitted_query_carries_the_account_content_policy() {
    let mut state = ready_state();
    state.settings.values.search_crawler.include_filenames = false;

    let effects = reduce(
        &mut state,
        AppAction::SearchSubmitted {
            request_id: 9,
            query: "アンケート".to_owned(),
            scope: scope(),
        },
    );

    // The actor verifies with the policy the state accepted the query under, so
    // it does not depend on a crawler notification that can be delivered late.
    let policy = effects
        .iter()
        .find_map(|effect| match effect {
            AppEffect::SearchMessages { content_policy, .. } => Some(content_policy.clone()),
            _ => None,
        })
        .expect("a submitted query is dispatched");
    assert!(!policy.include_filenames);
    assert!(policy.include_media_captions);
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
            query: "アンケート".to_owned(),
            scope: scope(),
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
        vec![
            AppEffect::EmitUiEvent(UiEvent::SearchChanged),
            AppEffect::PublishSearchResults {
                request_id: 9,
                results: vec![result("$event")],
            },
        ]
    );
}

#[test]
fn a_settled_search_result_is_not_replaced_by_a_later_one() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::SearchSubmitted {
            request_id: 9,
            query: "GPT".to_owned(),
            scope: scope(),
        },
    );
    reduce(
        &mut state,
        AppAction::SearchSucceeded {
            request_id: 9,
            query: "GPT".to_owned(),
            scope: scope(),
            results: vec![result("$local")],
        },
    );

    // Search is index-first: a query settles once, so a second result for the
    // same request is ignored rather than replacing what the user sees.
    let effects = reduce(
        &mut state,
        AppAction::SearchSucceeded {
            request_id: 9,
            query: "GPT".to_owned(),
            scope: scope(),
            results: vec![result("$sdk"), result("$local")],
        },
    );

    assert_eq!(
        state.search,
        SearchState::Results {
            request_id: 9,
            query: "GPT".to_owned(),
            scope: scope(),
            results: vec![result("$local")],
        }
    );
    assert!(effects.is_empty());
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
            query: "アンケート".to_owned(),
            scope: scope(),
            results: vec![result("$event")],
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::SearchFailed {
            request_id: 13,
            query: "アンケート".to_owned(),
            scope: scope(),
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
            query: "アンケート".to_owned(),
            scope: scope(),
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
fn closing_search_clears_state_and_emits_event() {
    let mut state = ready_state();
    state.search = SearchState::Results {
        request_id: 21,
        query: "若手".to_owned(),
        scope: scope(),
        results: vec![result("$event")],
    };

    let effects = reduce(&mut state, AppAction::SearchClosed);

    assert_eq!(state.search, SearchState::Closed);
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::SearchChanged)]
    );
}

#[test]
fn selecting_another_room_closes_current_room_search() {
    let mut state = ready_state();
    state.rooms = vec![room_summary("room-a"), room_summary("room-b")];
    state.navigation.active_room_id = Some("room-a".to_owned());
    state.timeline.room_id = Some("room-a".to_owned());
    state.search = SearchState::Results {
        request_id: 22,
        query: "若手".to_owned(),
        scope: SearchScope::CurrentRoom {
            room_id: "room-a".to_owned(),
        },
        results: vec![result("$event")],
    };

    let effects = reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "room-b".to_owned(),
        },
    );

    assert_eq!(state.search, SearchState::Closed);
    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::SearchChanged)));
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
            query: "new".to_owned(),
            scope: scope(),
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
