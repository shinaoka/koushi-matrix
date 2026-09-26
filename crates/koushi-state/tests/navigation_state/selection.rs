use super::support::{
    initial_attention_diagnostic, ready_state, rooms, search_crawler_settings_standard,
    session_info, spaces,
};
use koushi_state::{
    AppAction, AppEffect, AppState, RoomListFilter, RoomSummary, RoomTags, SessionState,
    SpaceSummary, ThreadPaneState, TimelinePaneState, UiEvent, compose_sidebar, reduce,
};

#[test]
fn navigation_actions_are_ignored_without_ready_session() {
    let mut state = AppState::default();

    assert_eq!(
        reduce(
            &mut state,
            AppAction::RoomListUpdated {
                spaces: spaces(),
                rooms: rooms(),
            },
        ),
        Vec::new()
    );
    assert_eq!(
        reduce(
            &mut state,
            AppAction::SelectSpace {
                space_id: Some("space-a".to_owned()),
            },
        ),
        Vec::new()
    );
    assert_eq!(
        reduce(
            &mut state,
            AppAction::SelectRoom {
                room_id: "room-a".to_owned(),
            },
        ),
        Vec::new()
    );
    assert!(state.spaces.is_empty());
    assert_eq!(state.navigation.active_space_id, None);
    assert_eq!(state.timeline, TimelinePaneState::default());
}

#[test]
fn selecting_space_filters_rooms_and_keeps_dms_global() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        spaces: spaces(),
        rooms: rooms(),
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::SelectSpace {
            space_id: Some("space-a".to_owned()),
        },
    );

    assert_eq!(state.navigation.active_space_id.as_deref(), Some("space-a"));
    assert_eq!(state.navigation.active_room_id.as_deref(), Some("room-a"));
    assert_eq!(state.timeline.room_id.as_deref(), Some("room-a"));
    let sidebar = compose_sidebar(
        state.navigation.active_space_id.as_deref(),
        &state.spaces,
        &state.rooms,
    );
    assert_eq!(sidebar.space_rooms.len(), 1);
    assert_eq!(sidebar.global_dms.len(), 1);
    assert_eq!(sidebar.space_rooms[0].room_id, "room-a");
    assert_eq!(sidebar.global_dms[0].room_id, "dm-a");
    assert_eq!(sidebar.space_unread_count, 5);
    assert_eq!(sidebar.dm_unread_count, 3);
    assert_eq!(
        state
            .room_list
            .items
            .iter()
            .map(|item| item.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["room-a"]
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
            AppEffect::EmitUiEvent(UiEvent::SpaceMembersChanged),
            // Issue #961: selecting a Space also discards the previous Space's
            // advertised children.
            AppEffect::EmitUiEvent(UiEvent::SpaceChildrenChanged),
            AppEffect::SubscribeTimeline {
                room_id: "room-a".to_owned(),
            },
            AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: "room-a".to_owned(),
            }),
        ]
    );
}

#[test]
fn selecting_account_home_clears_active_room_without_subscribing_default_room() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        spaces: spaces(),
        rooms: rooms(),
        navigation: koushi_state::NavigationState {
            active_space_id: Some("space-a".to_owned()),
            active_room_id: Some("room-a".to_owned()),
            ..Default::default()
        },
        timeline: TimelinePaneState {
            room_id: Some("room-a".to_owned()),
            is_subscribed: true,
            ..Default::default()
        },
        ..AppState::default()
    };

    let effects = reduce(&mut state, AppAction::SelectSpace { space_id: None });

    assert_eq!(state.navigation.active_space_id, None);
    assert_eq!(state.navigation.active_room_id, None);
    assert_eq!(state.timeline, TimelinePaneState::default());
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
            AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: "room-a".to_owned(),
            }),
        ]
    );
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, AppEffect::SubscribeTimeline { .. })),
        "account Home must not subscribe an arbitrary default room: {effects:?}"
    );
}

#[test]
fn active_space_hides_child_room_ids_without_joined_room_summaries() {
    let mut spaces = spaces();
    spaces[0].child_room_ids.push("room-not-joined".to_owned());
    let sidebar = compose_sidebar(Some("space-a"), &spaces, &rooms());
    let home_sidebar = compose_sidebar(None, &spaces, &rooms());

    assert_eq!(
        sidebar
            .space_rooms
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["room-a"]
    );
    assert!(sidebar.not_joined_space_rooms.is_empty());
    assert!(home_sidebar.not_joined_space_rooms.is_empty());
}

#[test]
fn room_list_update_keeps_empty_selected_space_empty() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        spaces: vec![
            SpaceSummary {
                space_id: "space-empty".to_owned(),
                raw_name: None,
                display_name: "Empty Space".to_owned(),
                avatar: None,
                join_rule: None,
                child_room_ids: Vec::new(),
            },
            SpaceSummary {
                space_id: "space-a".to_owned(),
                raw_name: None,
                display_name: "Space A".to_owned(),
                avatar: None,
                join_rule: None,
                child_room_ids: vec!["room-a".to_owned()],
            },
        ],
        rooms: rooms(),
        ..AppState::default()
    };

    reduce(
        &mut state,
        AppAction::SelectSpace {
            space_id: Some("space-empty".to_owned()),
        },
    );
    let updated_spaces = state.spaces.clone();
    let updated_rooms = state.rooms.clone();
    let effects = reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: updated_spaces,
            rooms: updated_rooms,
        },
    );

    assert_eq!(
        state.navigation.active_space_id.as_deref(),
        Some("space-empty")
    );
    assert_eq!(state.navigation.active_room_id, None);
    assert_eq!(state.timeline, TimelinePaneState::default());
    assert!(state.room_list.items.is_empty());
    let sidebar = compose_sidebar(
        state.navigation.active_space_id.as_deref(),
        &state.spaces,
        &state.rooms,
    );
    assert!(sidebar.space_rooms.is_empty());
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
            initial_attention_diagnostic(10, 3, false),
            AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged),
            AppEffect::NotifySearchCrawlerRoomsAvailable {
                room_ids: vec![
                    "room-a".to_owned(),
                    "dm-a".to_owned(),
                    "global-room".to_owned(),
                ],
                latest_event_ids: Default::default(),
                settings: search_crawler_settings_standard(),
            },
        ]
    );
}

#[test]
fn selecting_space_restores_last_non_dm_room_for_that_space() {
    let mut all_rooms = rooms();
    all_rooms.push(RoomSummary {
        room_id: "room-b".to_owned(),
        display_name: "Room B".to_owned(),
        display_label: "Room B".to_owned(),
        original_display_label: "Room B".to_owned(),
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
        parent_space_ids: vec!["space-a".to_owned()],
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 0,
    });
    let all_spaces = vec![SpaceSummary {
        space_id: "space-a".to_owned(),
        raw_name: None,
        display_name: "Space A".to_owned(),
        avatar: None,
        join_rule: None,
        child_room_ids: vec!["room-a".to_owned(), "room-b".to_owned(), "dm-a".to_owned()],
    }];
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        spaces: all_spaces,
        rooms: all_rooms,
        navigation: koushi_state::NavigationState {
            active_space_id: Some("space-a".to_owned()),
            active_room_id: Some("room-a".to_owned()),
            ..Default::default()
        },
        ..AppState::default()
    };

    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "room-b".to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "global-room".to_owned(),
        },
    );
    let effects = reduce(
        &mut state,
        AppAction::SelectSpace {
            space_id: Some("space-a".to_owned()),
        },
    );

    assert_eq!(state.navigation.active_space_id.as_deref(), Some("space-a"));
    assert_eq!(state.navigation.active_room_id.as_deref(), Some("room-b"));
    assert_eq!(state.timeline.room_id.as_deref(), Some("room-b"));
    assert_eq!(
        state.navigation.last_room_by_space_id.get("space-a"),
        Some(&"room-b".to_owned())
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
            AppEffect::EmitUiEvent(UiEvent::SpaceMembersChanged),
            // Issue #961: selecting a Space also discards the previous Space's
            // advertised children.
            AppEffect::EmitUiEvent(UiEvent::SpaceChildrenChanged),
            AppEffect::SubscribeTimeline {
                room_id: "room-b".to_owned(),
            },
            AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: "room-b".to_owned(),
            }),
        ]
    );
}

#[test]
fn selecting_space_restores_last_dm_and_dms_surface_for_that_space() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        spaces: spaces(),
        rooms: rooms(),
        navigation: koushi_state::NavigationState {
            active_space_id: Some("space-a".to_owned()),
            active_room_id: Some("room-a".to_owned()),
            ..Default::default()
        },
        ..AppState::default()
    };

    // Settle on a DM inside Space A. `room_belongs_to_space` used to return
    // `false` for every DM, so this was never recorded and never restored (#445).
    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "dm-a".to_owned(),
        },
    );
    assert_eq!(
        state.navigation.last_selection_by_space_id.get("space-a"),
        Some(&koushi_state::SpaceNavigationSelection {
            surface: koushi_state::SpaceConversationSurface::Dms,
            room_id: Some("dm-a".to_owned()),
        }),
        "a DM selection must be remembered against the Space whose DM list shows it"
    );
    assert_eq!(
        state.navigation.last_room_by_space_id.get("space-a"),
        Some(&"room-a".to_owned()),
        "the legacy map stays non-DM-only, so it still holds the last non-DM room \
         and an older build reading the same payload behaves unchanged"
    );

    // Leave for Home, then come back.
    reduce(&mut state, AppAction::SelectSpace { space_id: None });
    reduce(
        &mut state,
        AppAction::SelectSpace {
            space_id: Some("space-a".to_owned()),
        },
    );

    assert_eq!(state.navigation.active_space_id.as_deref(), Some("space-a"));
    assert_eq!(state.navigation.active_room_id.as_deref(), Some("dm-a"));
    assert_eq!(state.timeline.room_id.as_deref(), Some("dm-a"));
    assert_eq!(
        state.room_list.active_filter,
        RoomListFilter::People,
        "the DMs surface must be restored with the conversation"
    );
}

#[test]
fn provisional_room_list_projection_preserves_navigation_memory() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        spaces: spaces(),
        rooms: rooms(),
        navigation: koushi_state::NavigationState {
            active_space_id: Some("space-a".to_owned()),
            active_room_id: Some("room-a".to_owned()),
            ..Default::default()
        },
        ..AppState::default()
    };
    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "dm-a".to_owned(),
        },
    );
    let remembered = state
        .navigation
        .last_selection_by_space_id
        .get("space-a")
        .cloned()
        .expect("selection remembered");

    // An incomplete Sliding Sync projection is not evidence that the DM is gone.
    reduce(
        &mut state,
        AppAction::RoomListSnapshotProvisional {
            generation: 1,
            source: koushi_state::RoomListSource::Live,
            spaces: Vec::new(),
            rooms: Vec::new(),
            invites: Vec::new(),
        },
    );

    assert_eq!(
        state.navigation.last_selection_by_space_id.get("space-a"),
        Some(&remembered),
        "a provisional projection must not erase per-Space navigation memory"
    );
}

#[test]
fn authoritative_room_list_projection_invalidates_removed_selection() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        spaces: spaces(),
        rooms: rooms(),
        navigation: koushi_state::NavigationState {
            active_space_id: Some("space-a".to_owned()),
            active_room_id: Some("room-a".to_owned()),
            ..Default::default()
        },
        ..AppState::default()
    };
    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "dm-a".to_owned(),
        },
    );

    // Authoritative removal of the DM: the Space survives, so its surface memory
    // survives, but the conversation itself must be invalidated rather than kept
    // pointing at something the Space can no longer show.
    let remaining_rooms = rooms()
        .into_iter()
        .filter(|room| room.room_id != "dm-a")
        .collect::<Vec<_>>();
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: spaces(),
            rooms: remaining_rooms,
        },
    );

    assert_eq!(
        state.navigation.last_selection_by_space_id.get("space-a"),
        Some(&koushi_state::SpaceNavigationSelection {
            surface: koushi_state::SpaceConversationSurface::Dms,
            room_id: None,
        }),
        "authoritative removal clears the conversation but keeps the surface"
    );
}

#[test]
fn room_list_update_reopens_restored_active_room_timeline() {
    let mut state = ready_state();
    state.navigation.active_room_id = Some("room-a".to_owned());
    state.timeline = Default::default();

    let effects = reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: vec![],
            rooms: rooms()
                .into_iter()
                .filter(|room| room.room_id == "room-a")
                .collect(),
        },
    );

    assert_eq!(state.navigation.active_room_id.as_deref(), Some("room-a"));
    assert_eq!(state.timeline.room_id.as_deref(), Some("room-a"));
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::SubscribeTimeline { room_id } if room_id == "room-a")),
        "restored active room should subscribe its timeline after room list reload: {effects:?}"
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }) if room_id == "room-a")),
        "restored active room should emit timeline changed after room list reload: {effects:?}"
    );
}

#[test]
fn account_home_lists_all_non_dm_rooms_and_keeps_dms_global() {
    let sidebar = compose_sidebar(None, &spaces(), &rooms());

    assert!(sidebar.account_home.is_active);
    assert_eq!(sidebar.account_home.unread_count, 10);
    assert_eq!(
        sidebar
            .space_rooms
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["room-a", "global-room"]
    );
    assert_eq!(
        sidebar
            .global_dms
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["dm-a"]
    );
    assert_eq!(sidebar.space_unread_count, 7);
    assert_eq!(sidebar.dm_unread_count, 3);
}

#[test]
fn selecting_room_subscribes_timeline_and_clears_thread() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        spaces: spaces(),
        rooms: rooms(),
        navigation: koushi_state::NavigationState {
            active_space_id: Some("space-a".to_owned()),
            active_room_id: None,
            ..Default::default()
        },
        thread: ThreadPaneState::Open {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            intent: koushi_state::ThreadOpenIntent::ExistingThread,
            is_subscribed: true,
            composer: Default::default(),
            staged_uploads: Vec::new(),
        },
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "room-a".to_owned(),
        },
    );

    assert_eq!(state.navigation.active_room_id.as_deref(), Some("room-a"));
    assert_eq!(state.timeline.room_id.as_deref(), Some("room-a"));
    assert!(!state.timeline.is_subscribed);
    assert_eq!(state.thread, ThreadPaneState::Closed);
    assert_eq!(
        effects,
        vec![
            AppEffect::SubscribeTimeline {
                room_id: "room-a".to_owned(),
            },
            AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: "room-a".to_owned(),
            }),
            AppEffect::EmitUiEvent(UiEvent::ThreadChanged),
        ]
    );
}

#[test]
fn selecting_current_room_does_not_resubscribe_timeline() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        spaces: spaces(),
        rooms: rooms(),
        navigation: koushi_state::NavigationState {
            active_space_id: Some("space-a".to_owned()),
            active_room_id: Some("room-a".to_owned()),
            ..Default::default()
        },
        timeline: TimelinePaneState {
            room_id: Some("room-a".to_owned()),
            is_subscribed: true,
            ..Default::default()
        },
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "room-a".to_owned(),
        },
    );

    assert_eq!(state.navigation.active_room_id.as_deref(), Some("room-a"));
    assert_eq!(state.timeline.room_id.as_deref(), Some("room-a"));
    assert!(state.timeline.is_subscribed);
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, AppEffect::SubscribeTimeline { .. })),
        "selecting the current room must not replay the existing room timeline"
    );
    assert_eq!(effects, Vec::<AppEffect>::new());
}

#[test]
fn selecting_non_dm_room_moves_scope_to_containing_space_or_home() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        spaces: spaces(),
        rooms: rooms(),
        navigation: koushi_state::NavigationState {
            active_space_id: Some("space-a".to_owned()),
            active_room_id: Some("room-a".to_owned()),
            ..Default::default()
        },
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "global-room".to_owned(),
        },
    );

    assert_eq!(state.navigation.active_space_id, None);
    assert_eq!(
        state.navigation.active_room_id.as_deref(),
        Some("global-room")
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
            AppEffect::SubscribeTimeline {
                room_id: "global-room".to_owned(),
            },
            AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: "global-room".to_owned(),
            }),
        ]
    );

    let effects = reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "room-a".to_owned(),
        },
    );

    assert_eq!(state.navigation.active_space_id.as_deref(), Some("space-a"));
    assert_eq!(state.navigation.active_room_id.as_deref(), Some("room-a"));
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
            AppEffect::EmitUiEvent(UiEvent::SpaceMembersChanged),
            // Issue #961: selecting a Space also discards the previous Space's
            // advertised children.
            AppEffect::EmitUiEvent(UiEvent::SpaceChildrenChanged),
            AppEffect::SubscribeTimeline {
                room_id: "room-a".to_owned(),
            },
            AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: "room-a".to_owned(),
            }),
        ]
    );
}

#[test]
fn selecting_dm_room_preserves_current_space_scope() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        spaces: spaces(),
        rooms: rooms(),
        navigation: koushi_state::NavigationState {
            active_space_id: Some("space-a".to_owned()),
            active_room_id: Some("room-a".to_owned()),
            ..Default::default()
        },
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "dm-a".to_owned(),
        },
    );

    assert_eq!(state.navigation.active_space_id.as_deref(), Some("space-a"));
    assert_eq!(state.navigation.active_room_id.as_deref(), Some("dm-a"));
    assert_eq!(
        effects,
        vec![
            AppEffect::SubscribeTimeline {
                room_id: "dm-a".to_owned(),
            },
            AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: "dm-a".to_owned(),
            }),
        ]
    );
}
