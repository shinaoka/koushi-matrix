use matrix_desktop_state::{
    AppAction, AppEffect, AppState, AvatarImage, AvatarThumbnailState,
    NativeAttentionObservationKind, NativeAttentionProjectionInput, RoomSummary, RoomTags,
    SessionInfo, SessionState, SpaceSummary, ThreadPaneState, TimelinePaneState, UiEvent,
    compose_sidebar, native_attention_state_from_rooms, reduce,
};
use serde_json::json;
use std::collections::BTreeMap;

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.org".to_owned(),
        user_id: "@user-a:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
    }
}

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(session_info()),
        ..AppState::default()
    }
}

fn avatar(mxc_uri: &str) -> AvatarImage {
    AvatarImage {
        mxc_uri: mxc_uri.to_owned(),
        thumbnail: AvatarThumbnailState::NotRequested,
    }
}

fn spaces() -> Vec<SpaceSummary> {
    vec![SpaceSummary {
        space_id: "space-a".to_owned(),
        display_name: "Space A".to_owned(),
        avatar: None,
        child_room_ids: vec!["room-a".to_owned(), "dm-a".to_owned()],
    }]
}

fn rooms() -> Vec<RoomSummary> {
    vec![
        RoomSummary {
            room_id: "room-a".to_owned(),
            display_name: "Room A".to_owned(),
            display_label: "Room A".to_owned(),
            original_display_label: "Room A".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 5,
            notification_count: 5,
            highlight_count: 1,
            marked_unread: false,
            last_activity_ms: 0,
            parent_space_ids: vec!["space-a".to_owned()],
        },
        RoomSummary {
            room_id: "dm-a".to_owned(),
            display_name: "Alice".to_owned(),
            display_label: "Alice".to_owned(),
            original_display_label: "Alice".to_owned(),
            avatar: None,
            is_dm: true,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 3,
            notification_count: 3,
            highlight_count: 0,
            marked_unread: false,
            last_activity_ms: 0,
            parent_space_ids: vec!["space-a".to_owned()],
        },
        RoomSummary {
            room_id: "global-room".to_owned(),
            display_name: "Global Room".to_owned(),
            display_label: "Global Room".to_owned(),
            original_display_label: "Global Room".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 2,
            notification_count: 2,
            highlight_count: 0,
            marked_unread: false,
            last_activity_ms: 0,
            parent_space_ids: vec![],
        },
    ]
}

#[test]
fn room_summary_serializes_projected_label_and_dm_identity_contract() {
    let room = RoomSummary {
        room_id: "dm-a".to_owned(),
        display_name: "Alice Upstream".to_owned(),
        display_label: "Alice Upstream".to_owned(),
        original_display_label: "Alice Upstream".to_owned(),
        avatar: None,
        is_dm: true,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: 3,
        notification_count: 3,
        highlight_count: 0,
        marked_unread: false,
        last_activity_ms: 0,
        parent_space_ids: Vec::new(),
    };

    let value = serde_json::to_value(&room).expect("serialize room summary");

    assert_eq!(value["display_label"], json!("Alice Upstream"));
    assert_eq!(value["original_display_label"], json!("Alice Upstream"));
    assert_eq!(value["dm_user_ids"], json!([]));
}

#[test]
fn room_list_update_projects_dm_room_display_labels_from_aliases() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::LocalUserAliasesLoaded {
            aliases: BTreeMap::from([(
                "@alice:example.invalid".to_owned(),
                "Alice Local".to_owned(),
            )]),
        },
    );

    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![RoomSummary {
                room_id: "dm-a".to_owned(),
                display_name: "Alice Upstream".to_owned(),
                display_label: "Alice Upstream".to_owned(),
                original_display_label: "Alice Upstream".to_owned(),
                avatar: None,
                is_dm: true,
                dm_user_ids: vec!["@alice:example.invalid".to_owned()],
                tags: RoomTags::default(),
                unread_count: 3,
                notification_count: 3,
                highlight_count: 0,
                marked_unread: false,
                last_activity_ms: 0,
                parent_space_ids: Vec::new(),
            }],
        },
    );

    let room = state.rooms.first().expect("projected room");
    let value = serde_json::to_value(room).expect("serialize room summary");

    assert_eq!(room.display_name, "Alice Upstream");
    assert_eq!(room.display_label, "Alice Local");
    assert_eq!(room.original_display_label, "Alice Upstream");
    assert_eq!(value["display_name"], json!("Alice Upstream"));
    assert_eq!(value["display_label"], json!("Alice Local"));
    assert_eq!(value["original_display_label"], json!("Alice Upstream"));
}

#[test]
fn local_alias_update_refreshes_open_dm_room_labels_and_notification_candidate() {
    let mut state = ready_state();
    state.rooms = vec![RoomSummary {
        room_id: "dm-a".to_owned(),
        display_name: "Alice Upstream".to_owned(),
        display_label: "Alice Upstream".to_owned(),
        original_display_label: "Alice Upstream".to_owned(),
        avatar: None,
        is_dm: true,
        dm_user_ids: vec!["@alice:example.invalid".to_owned()],
        tags: RoomTags::default(),
        unread_count: 3,
        notification_count: 3,
        highlight_count: 0,
        marked_unread: false,
        last_activity_ms: 0,
        parent_space_ids: Vec::new(),
    }];
    state.native_attention = native_attention_state_from_rooms(NativeAttentionProjectionInput {
        rooms: &state.rooms,
        active_room_id: None,
        muted_room_ids: &[],
        room_notification_modes: &std::collections::HashMap::new(),
        ignored_user_ids: &std::collections::BTreeSet::new(),
        window_focused: false,
        observation: NativeAttentionObservationKind::Live,
        previous_candidate: None,
        capabilities: Default::default(),
    });

    let effects = reduce(
        &mut state,
        AppAction::LocalUserAliasUpdateRequested {
            request_id: 64,
            user_id: "@alice:example.invalid".to_owned(),
            alias: Some("Alice Local".to_owned()),
        },
    );

    assert_eq!(state.rooms[0].display_name, "Alice Upstream");
    assert_eq!(state.rooms[0].display_label, "Alice Local");
    assert_eq!(
        state
            .native_attention
            .summary
            .candidate
            .as_ref()
            .map(|candidate| candidate.room_display_name.as_str()),
        Some("Alice Local")
    );
    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::ProfileChanged)));
    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::RoomListChanged)));
    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged)));
}

#[test]
fn room_list_update_replaces_state_and_emits_room_list_event() {
    let mut state = ready_state();

    let effects = reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: spaces(),
            rooms: rooms(),
        },
    );

    assert_eq!(state.spaces.len(), 1);
    assert_eq!(state.rooms.len(), 3);
    assert_eq!(state.rooms[0].notification_count, 5);
    assert_eq!(state.rooms[0].highlight_count, 1);
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
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
fn room_list_update_selects_first_room_when_no_room_is_active() {
    let mut state = ready_state();

    let effects = reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: spaces(),
            rooms: rooms(),
        },
    );

    assert_eq!(state.navigation.active_room_id.as_deref(), Some("room-a"));
    assert_eq!(state.timeline.room_id.as_deref(), Some("room-a"));
    assert!(!state.timeline.is_subscribed);
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
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
fn room_list_update_clears_missing_active_space_and_room() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        spaces: spaces(),
        rooms: rooms(),
        navigation: matrix_desktop_state::NavigationState {
            active_space_id: Some("space-a".to_owned()),
            active_room_id: Some("room-a".to_owned()),
        },
        timeline: TimelinePaneState {
            room_id: Some("room-a".to_owned()),
            is_subscribed: true,
            is_paginating_backwards: false,
            composer: Default::default(),
            scheduled_send_capability: Default::default(),
            scheduled_sends: Vec::new(),
            staged_uploads: Vec::new(),
            media_gallery: Vec::new(),
        },
        thread: ThreadPaneState::Open {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            is_subscribed: true,
            composer: Default::default(),
        },
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms: vec![RoomSummary {
                room_id: "global-room".to_owned(),
                display_name: "Global Room".to_owned(),
                display_label: "Global Room".to_owned(),
                original_display_label: "Global Room".to_owned(),
                avatar: None,
                is_dm: false,
                dm_user_ids: Vec::new(),
                tags: RoomTags::default(),
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                marked_unread: false,
                last_activity_ms: 0,
                parent_space_ids: vec![],
            }],
        },
    );

    assert_eq!(state.navigation.active_space_id, None);
    assert_eq!(state.navigation.active_room_id, None);
    assert_eq!(state.timeline, TimelinePaneState::default());
    assert_eq!(state.thread, ThreadPaneState::Closed);
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
            AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: "room-a".to_owned(),
            }),
            AppEffect::EmitUiEvent(UiEvent::ThreadChanged),
        ]
    );
}

#[test]
fn room_list_update_moves_active_room_when_it_leaves_selected_space() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        spaces: vec![SpaceSummary {
            space_id: "space-a".to_owned(),
            display_name: "Space A".to_owned(),
            avatar: None,
            child_room_ids: vec!["room-a".to_owned()],
        }],
        rooms: vec![
            RoomSummary {
                room_id: "room-a".to_owned(),
                display_name: "Room A".to_owned(),
                display_label: "Room A".to_owned(),
                original_display_label: "Room A".to_owned(),
                avatar: None,
                is_dm: false,
                dm_user_ids: Vec::new(),
                tags: RoomTags::default(),
                unread_count: 5,
                notification_count: 5,
                highlight_count: 0,
                marked_unread: false,
                last_activity_ms: 0,
                parent_space_ids: vec!["space-a".to_owned()],
            },
            RoomSummary {
                room_id: "room-b".to_owned(),
                display_name: "Room B".to_owned(),
                display_label: "Room B".to_owned(),
                original_display_label: "Room B".to_owned(),
                avatar: None,
                is_dm: false,
                dm_user_ids: Vec::new(),
                tags: RoomTags::default(),
                unread_count: 2,
                notification_count: 2,
                highlight_count: 0,
                marked_unread: false,
                last_activity_ms: 0,
                parent_space_ids: Vec::new(),
            },
        ],
        navigation: matrix_desktop_state::NavigationState {
            active_space_id: Some("space-a".to_owned()),
            active_room_id: Some("room-a".to_owned()),
        },
        timeline: TimelinePaneState {
            room_id: Some("room-a".to_owned()),
            is_subscribed: true,
            is_paginating_backwards: false,
            composer: Default::default(),
            scheduled_send_capability: Default::default(),
            scheduled_sends: Vec::new(),
            staged_uploads: Vec::new(),
            media_gallery: Vec::new(),
        },
        thread: ThreadPaneState::Open {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            is_subscribed: true,
            composer: Default::default(),
        },
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: vec![SpaceSummary {
                space_id: "space-a".to_owned(),
                display_name: "Space A".to_owned(),
                avatar: None,
                child_room_ids: vec!["room-b".to_owned()],
            }],
            rooms: vec![
                RoomSummary {
                    room_id: "room-a".to_owned(),
                    display_name: "Room A".to_owned(),
                    display_label: "Room A".to_owned(),
                    original_display_label: "Room A".to_owned(),
                    avatar: None,
                    is_dm: false,
                    dm_user_ids: Vec::new(),
                    tags: RoomTags::default(),
                    unread_count: 5,
                    notification_count: 5,
                    highlight_count: 0,
                    marked_unread: false,
                    last_activity_ms: 0,
                    parent_space_ids: Vec::new(),
                },
                RoomSummary {
                    room_id: "room-b".to_owned(),
                    display_name: "Room B".to_owned(),
                    display_label: "Room B".to_owned(),
                    original_display_label: "Room B".to_owned(),
                    avatar: None,
                    is_dm: false,
                    dm_user_ids: Vec::new(),
                    tags: RoomTags::default(),
                    unread_count: 2,
                    notification_count: 2,
                    highlight_count: 0,
                    marked_unread: false,
                    last_activity_ms: 0,
                    parent_space_ids: vec!["space-a".to_owned()],
                },
            ],
        },
    );

    assert_eq!(state.navigation.active_space_id.as_deref(), Some("space-a"));
    assert_eq!(state.navigation.active_room_id.as_deref(), Some("room-b"));
    assert_eq!(state.timeline.room_id.as_deref(), Some("room-b"));
    assert!(!state.timeline.is_subscribed);
    assert_eq!(state.thread, ThreadPaneState::Closed);
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
            AppEffect::SubscribeTimeline {
                room_id: "room-b".to_owned(),
            },
            AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: "room-b".to_owned(),
            }),
            AppEffect::EmitUiEvent(UiEvent::ThreadChanged),
        ]
    );
}

#[test]
fn room_list_update_moves_active_room_when_it_disappears_from_selected_space() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        spaces: vec![SpaceSummary {
            space_id: "space-a".to_owned(),
            display_name: "Space A".to_owned(),
            avatar: None,
            child_room_ids: vec!["room-a".to_owned()],
        }],
        rooms: vec![RoomSummary {
            room_id: "room-a".to_owned(),
            display_name: "Room A".to_owned(),
            display_label: "Room A".to_owned(),
            original_display_label: "Room A".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 5,
            notification_count: 5,
            highlight_count: 0,
            marked_unread: false,
            last_activity_ms: 0,
            parent_space_ids: vec!["space-a".to_owned()],
        }],
        navigation: matrix_desktop_state::NavigationState {
            active_space_id: Some("space-a".to_owned()),
            active_room_id: Some("room-a".to_owned()),
        },
        timeline: TimelinePaneState {
            room_id: Some("room-a".to_owned()),
            is_subscribed: true,
            is_paginating_backwards: false,
            composer: Default::default(),
            scheduled_send_capability: Default::default(),
            scheduled_sends: Vec::new(),
            staged_uploads: Vec::new(),
            media_gallery: Vec::new(),
        },
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: vec![SpaceSummary {
                space_id: "space-a".to_owned(),
                display_name: "Space A".to_owned(),
                avatar: None,
                child_room_ids: vec!["room-b".to_owned()],
            }],
            rooms: vec![RoomSummary {
                room_id: "room-b".to_owned(),
                display_name: "Room B".to_owned(),
                display_label: "Room B".to_owned(),
                original_display_label: "Room B".to_owned(),
                avatar: None,
                is_dm: false,
                dm_user_ids: Vec::new(),
                tags: RoomTags::default(),
                unread_count: 2,
                notification_count: 2,
                highlight_count: 0,
                marked_unread: false,
                last_activity_ms: 0,
                parent_space_ids: vec!["space-a".to_owned()],
            }],
        },
    );

    assert_eq!(state.navigation.active_space_id.as_deref(), Some("space-a"));
    assert_eq!(state.navigation.active_room_id.as_deref(), Some("room-b"));
    assert_eq!(state.timeline.room_id.as_deref(), Some("room-b"));
    assert!(!state.timeline.is_subscribed);
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
            AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: "room-a".to_owned(),
            }),
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
fn room_list_update_keeps_active_dm_global_with_selected_space() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        spaces: spaces(),
        rooms: rooms(),
        navigation: matrix_desktop_state::NavigationState {
            active_space_id: Some("space-a".to_owned()),
            active_room_id: Some("dm-a".to_owned()),
        },
        timeline: TimelinePaneState {
            room_id: Some("dm-a".to_owned()),
            is_subscribed: true,
            is_paginating_backwards: false,
            composer: Default::default(),
            scheduled_send_capability: Default::default(),
            scheduled_sends: Vec::new(),
            staged_uploads: Vec::new(),
            media_gallery: Vec::new(),
        },
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: vec![SpaceSummary {
                space_id: "space-a".to_owned(),
                display_name: "Space A".to_owned(),
                avatar: None,
                child_room_ids: vec!["room-a".to_owned()],
            }],
            rooms: rooms(),
        },
    );

    assert_eq!(state.navigation.active_space_id.as_deref(), Some("space-a"));
    assert_eq!(state.navigation.active_room_id.as_deref(), Some("dm-a"));
    assert_eq!(state.timeline.room_id.as_deref(), Some("dm-a"));
    assert!(state.timeline.is_subscribed);
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
    );
}

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
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
    );
}

#[test]
fn account_home_lists_all_non_dm_rooms_and_keeps_dms_global() {
    let sidebar = compose_sidebar(None, &spaces(), &rooms());

    assert!(sidebar.account_home.is_active);
    assert_eq!(sidebar.account_home.unread_count, 7);
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
fn sidebar_projection_carries_rust_owned_highlight_counts_for_mention_affordances() {
    let sidebar = compose_sidebar(None, &spaces(), &rooms());
    let value = serde_json::to_value(sidebar).expect("sidebar serializes");

    assert_eq!(value["account_home"]["unread_count"], json!(7));
    assert_eq!(value["space_rail"][0]["unread_count"], json!(5));
    assert_eq!(value["account_home"]["highlight_count"], json!(1));
    assert_eq!(value["space_rail"][0]["highlight_count"], json!(1));
    assert_eq!(value["space_rooms"][0]["unread_count"], json!(5));
    assert_eq!(value["space_rooms"][0]["highlight_count"], json!(1));
    assert_eq!(value["global_dms"][0]["unread_count"], json!(3));
    assert_eq!(value["global_dms"][0]["highlight_count"], json!(0));
    assert_eq!(value["space_unread_count"], json!(7));
    assert_eq!(value["dm_unread_count"], json!(3));
    assert_eq!(value["space_highlight_count"], json!(1));
    assert_eq!(value["dm_highlight_count"], json!(0));
}

#[test]
fn sidebar_items_carry_rust_owned_room_and_space_avatars() {
    let spaces = vec![SpaceSummary {
        space_id: "space-a".to_owned(),
        display_name: "Space A".to_owned(),
        avatar: Some(avatar("mxc://example.invalid/space-a")),
        child_room_ids: vec!["room-a".to_owned(), "dm-a".to_owned()],
    }];
    let rooms = vec![
        RoomSummary {
            room_id: "room-a".to_owned(),
            display_name: "Room A".to_owned(),
            display_label: "Room A".to_owned(),
            original_display_label: "Room A".to_owned(),
            avatar: Some(avatar("mxc://example.invalid/room-a")),
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 5,
            notification_count: 5,
            highlight_count: 1,
            marked_unread: false,
            last_activity_ms: 0,
            parent_space_ids: vec!["space-a".to_owned()],
        },
        RoomSummary {
            room_id: "dm-a".to_owned(),
            display_name: "Alice".to_owned(),
            display_label: "Alice".to_owned(),
            original_display_label: "Alice".to_owned(),
            avatar: Some(avatar("mxc://example.invalid/dm-a")),
            is_dm: true,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 3,
            notification_count: 3,
            highlight_count: 0,
            marked_unread: false,
            last_activity_ms: 0,
            parent_space_ids: vec!["space-a".to_owned()],
        },
    ];

    let sidebar = compose_sidebar(Some("space-a"), &spaces, &rooms);

    assert_eq!(
        sidebar.space_rail[0]
            .avatar
            .as_ref()
            .map(|avatar| avatar.mxc_uri.as_str()),
        Some("mxc://example.invalid/space-a")
    );
    assert_eq!(
        sidebar.space_rooms[0]
            .avatar
            .as_ref()
            .map(|avatar| avatar.mxc_uri.as_str()),
        Some("mxc://example.invalid/room-a")
    );
    assert_eq!(
        sidebar.global_dms[0]
            .avatar
            .as_ref()
            .map(|avatar| avatar.mxc_uri.as_str()),
        Some("mxc://example.invalid/dm-a")
    );
}

#[test]
fn selecting_room_subscribes_timeline_and_clears_thread() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        rooms: rooms(),
        thread: ThreadPaneState::Open {
            room_id: "room-a".to_owned(),
            root_event_id: "$root".to_owned(),
            is_subscribed: true,
            composer: Default::default(),
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
