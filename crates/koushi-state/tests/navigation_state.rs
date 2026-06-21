use koushi_state::{
    AppAction, AppEffect, AppState, AvatarImage, AvatarThumbnailState,
    NativeAttentionObservationKind, NativeAttentionProjectionInput, RoomSummary, RoomTags,
    SearchCrawlerSettings, SessionInfo, SessionState, SpaceSummary, ThreadPaneState,
    TimelinePaneState, UiEvent, UserProfile, compose_sidebar, native_attention_state_from_rooms,
    reduce,
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

fn search_crawler_settings_standard() -> SearchCrawlerSettings {
    SearchCrawlerSettings::default()
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
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 0,
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
            dm_space_ids: vec!["space-a".to_owned()],
            is_encrypted: false,
            joined_members: 0,
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
            dm_space_ids: vec![],
            is_encrypted: false,
            joined_members: 0,
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
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 0,
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
                dm_space_ids: Vec::new(),
                is_encrypted: false,
                joined_members: 0,
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
fn room_list_update_projects_dm_room_avatar_from_counterpart_profile() {
    let mut state = ready_state();
    state.profile.users.insert(
        "@alice:example.invalid".to_owned(),
        UserProfile {
            user_id: "@alice:example.invalid".to_owned(),
            display_name: Some("Alice Upstream".to_owned()),
            display_label: "Alice Upstream".to_owned(),
            original_display_label: "Alice Upstream".to_owned(),
            mention_search_terms: vec!["@alice:example.invalid".to_owned()],
            avatar: Some(avatar("mxc://example.invalid/alice-avatar")),
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
                dm_space_ids: Vec::new(),
                is_encrypted: false,
                joined_members: 0,
            }],
        },
    );

    assert_eq!(
        state.rooms[0].avatar.as_ref().map(|avatar| avatar.mxc_uri.as_str()),
        Some("mxc://example.invalid/alice-avatar")
    );
    let sidebar = compose_sidebar(None, &state.spaces, &state.rooms);
    assert_eq!(
        sidebar.global_dms[0]
            .avatar
            .as_ref()
            .map(|avatar| avatar.mxc_uri.as_str()),
        Some("mxc://example.invalid/alice-avatar")
    );
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
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 0,
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
            AppEffect::NotifySearchCrawlerRoomsAvailable {
                room_ids: vec![
                    "room-a".to_owned(),
                    "dm-a".to_owned(),
                    "global-room".to_owned(),
                ],
                settings: search_crawler_settings_standard(),
            },
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
            AppEffect::NotifySearchCrawlerRoomsAvailable {
                room_ids: vec![
                    "room-a".to_owned(),
                    "dm-a".to_owned(),
                    "global-room".to_owned(),
                ],
                settings: search_crawler_settings_standard(),
            },
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
        navigation: koushi_state::NavigationState {
            active_space_id: Some("space-a".to_owned()),
            active_room_id: Some("room-a".to_owned()),
            ..Default::default()
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
            media_downloads: Default::default(),
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
                dm_space_ids: vec![],
                is_encrypted: false,
                joined_members: 0,
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
            AppEffect::NotifySearchCrawlerRoomsAvailable {
                room_ids: vec!["global-room".to_owned()],
                settings: search_crawler_settings_standard(),
            },
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
                dm_space_ids: Vec::new(),
                is_encrypted: false,
                joined_members: 0,
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
                dm_space_ids: Vec::new(),
                is_encrypted: false,
                joined_members: 0,
            },
        ],
        navigation: koushi_state::NavigationState {
            active_space_id: Some("space-a".to_owned()),
            active_room_id: Some("room-a".to_owned()),
            ..Default::default()
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
            media_downloads: Default::default(),
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
                    dm_space_ids: Vec::new(),
                    is_encrypted: false,
                    joined_members: 0,
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
                    dm_space_ids: Vec::new(),
                    is_encrypted: false,
                    joined_members: 0,
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
            AppEffect::NotifySearchCrawlerRoomsAvailable {
                room_ids: vec!["room-a".to_owned(), "room-b".to_owned()],
                settings: search_crawler_settings_standard(),
            },
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
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 0,
        }],
        navigation: koushi_state::NavigationState {
            active_space_id: Some("space-a".to_owned()),
            active_room_id: Some("room-a".to_owned()),
            ..Default::default()
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
            media_downloads: Default::default(),
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
                dm_space_ids: Vec::new(),
                is_encrypted: false,
                joined_members: 0,
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
            AppEffect::NotifySearchCrawlerRoomsAvailable {
                room_ids: vec!["room-b".to_owned()],
                settings: search_crawler_settings_standard(),
            },
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
        navigation: koushi_state::NavigationState {
            active_space_id: Some("space-a".to_owned()),
            active_room_id: Some("dm-a".to_owned()),
            ..Default::default()
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
            media_downloads: Default::default(),
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
        vec![
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
            AppEffect::NotifySearchCrawlerRoomsAvailable {
                room_ids: vec![
                    "room-a".to_owned(),
                    "dm-a".to_owned(),
                    "global-room".to_owned(),
                ],
                settings: search_crawler_settings_standard(),
            },
        ]
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
fn room_list_update_keeps_empty_selected_space_empty() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        spaces: vec![
            SpaceSummary {
                space_id: "space-empty".to_owned(),
                display_name: "Empty Space".to_owned(),
                avatar: None,
                child_room_ids: Vec::new(),
            },
            SpaceSummary {
                space_id: "space-a".to_owned(),
                display_name: "Space A".to_owned(),
                avatar: None,
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
            AppEffect::NotifySearchCrawlerRoomsAvailable {
                room_ids: vec![
                    "room-a".to_owned(),
                    "dm-a".to_owned(),
                    "global-room".to_owned(),
                ],
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
        last_activity_ms: 0,
        parent_space_ids: vec!["space-a".to_owned()],
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 0,
    });
    let all_spaces = vec![SpaceSummary {
        space_id: "space-a".to_owned(),
        display_name: "Space A".to_owned(),
        avatar: None,
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
fn home_lists_all_dms() {
    let sidebar = compose_sidebar(None, &spaces(), &rooms());

    assert_eq!(
        sidebar
            .global_dms
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["dm-a"]
    );
    assert_eq!(sidebar.dm_unread_count, 3);
}

#[test]
fn active_space_lists_only_dms_belonging_to_that_space() {
    let mut rooms_with_outside = rooms();
    rooms_with_outside.push(RoomSummary {
        room_id: "dm-outside".to_owned(),
        display_name: "Outside DM".to_owned(),
        display_label: "Outside DM".to_owned(),
        original_display_label: "Outside DM".to_owned(),
        avatar: None,
        is_dm: true,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: 9,
        notification_count: 9,
        highlight_count: 0,
        marked_unread: false,
        last_activity_ms: 0,
        parent_space_ids: Vec::new(),
        dm_space_ids: vec![],
        is_encrypted: false,
        joined_members: 0,
    });

    let sidebar = compose_sidebar(Some("space-a"), &spaces(), &rooms_with_outside);

    assert_eq!(
        sidebar
            .global_dms
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["dm-a"]
    );
    assert_eq!(sidebar.dm_unread_count, 3);
}

#[test]
fn dm_in_multiple_spaces_appears_under_each() {
    let multi_spaces = vec![
        SpaceSummary {
            space_id: "space-a".to_owned(),
            display_name: "Space A".to_owned(),
            avatar: None,
            child_room_ids: vec!["room-a".to_owned(), "dm-multi".to_owned()],
        },
        SpaceSummary {
            space_id: "space-b".to_owned(),
            display_name: "Space B".to_owned(),
            avatar: None,
            child_room_ids: vec!["dm-multi".to_owned()],
        },
    ];
    let multi_rooms = vec![
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
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 0,
        },
        RoomSummary {
            room_id: "dm-multi".to_owned(),
            display_name: "Multi DM".to_owned(),
            display_label: "Multi DM".to_owned(),
            original_display_label: "Multi DM".to_owned(),
            avatar: None,
            is_dm: true,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 2,
            notification_count: 2,
            highlight_count: 0,
            marked_unread: false,
            last_activity_ms: 0,
            parent_space_ids: Vec::new(),
            dm_space_ids: vec!["space-a".to_owned(), "space-b".to_owned()],
            is_encrypted: false,
            joined_members: 0,
        },
    ];

    let sidebar_a = compose_sidebar(Some("space-a"), &multi_spaces, &multi_rooms);
    let sidebar_b = compose_sidebar(Some("space-b"), &multi_spaces, &multi_rooms);

    assert_eq!(
        sidebar_a
            .global_dms
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["dm-multi"]
    );
    assert_eq!(
        sidebar_b
            .global_dms
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["dm-multi"]
    );
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
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 0,
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
            dm_space_ids: vec!["space-a".to_owned()],
            is_encrypted: false,
            joined_members: 0,
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
