use koushi_state::{
    AppAction, AppEffect, AppState, AvatarImage, AvatarThumbnailState, MainTimelineAnchor,
    NativeAttentionObservationKind, NativeAttentionProjectionInput, RoomListFilter, RoomListSort,
    RoomNotificationMode, RoomNotificationSettings, RoomSummary, RoomTags, SearchCrawlerSettings,
    SessionInfo, SessionState, SpaceSummary, ThreadPaneState, TimelinePaneState,
    TimelineScrollAnchorEdge, UiEvent, UserProfile, compose_sidebar,
    compose_sidebar_with_room_notification_settings, native_attention_state_from_rooms, reduce,
};
use serde_json::json;
use std::collections::{BTreeMap, HashMap};

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

fn ready_avatar_thumbnail(label: &str) -> AvatarThumbnailState {
    AvatarThumbnailState::Ready {
        source_url: format!("file:///tmp/koushi-test-{label}.png"),
        width: Some(64),
        height: Some(64),
        mime_type: Some("image/png".to_owned()),
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
            latest_event: None,
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
            latest_event: None,
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
            latest_event: None,
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
        latest_event: None,
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
                latest_event: None,
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
                latest_event: None,
                parent_space_ids: Vec::new(),
                dm_space_ids: Vec::new(),
                is_encrypted: false,
                joined_members: 0,
            }],
        },
    );

    assert_eq!(
        state.rooms[0]
            .avatar
            .as_ref()
            .map(|avatar| avatar.mxc_uri.as_str()),
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
fn avatar_thumbnail_update_refreshes_people_filter_room_avatar_surface() {
    let mut state = ready_state();
    let mxc_uri = "mxc://example.invalid/dm-avatar";
    let thumbnail = ready_avatar_thumbnail("people-filter");
    state.rooms = vec![RoomSummary {
        room_id: "dm-a".to_owned(),
        display_name: "Alice".to_owned(),
        display_label: "Alice".to_owned(),
        original_display_label: "Alice".to_owned(),
        avatar: Some(AvatarImage {
            mxc_uri: mxc_uri.to_owned(),
            thumbnail: AvatarThumbnailState::NotRequested,
        }),
        is_dm: true,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: 3,
        notification_count: 3,
        highlight_count: 0,
        marked_unread: false,
        last_activity_ms: 0,
        latest_event: None,
        parent_space_ids: vec!["space-a".to_owned()],
        dm_space_ids: vec!["space-a".to_owned()],
        is_encrypted: false,
        joined_members: 0,
    }];

    reduce(
        &mut state,
        AppAction::RoomListFilterSelected {
            filter: RoomListFilter::People,
        },
    );
    assert_eq!(state.room_list.active_filter, RoomListFilter::People);
    assert_eq!(state.room_list.items.len(), 1);

    reduce(
        &mut state,
        AppAction::AvatarThumbnailUpdated {
            mxc_uri: mxc_uri.to_owned(),
            thumbnail: thumbnail.clone(),
        },
    );

    assert_eq!(
        state.rooms[0]
            .avatar
            .as_ref()
            .map(|avatar| &avatar.thumbnail),
        Some(&thumbnail)
    );
    assert_eq!(state.room_list.active_filter, RoomListFilter::People);
    assert_eq!(state.room_list.items.len(), 1);
    let sidebar = compose_sidebar(None, &state.spaces, &state.rooms);
    assert_eq!(
        sidebar.global_dms[0]
            .avatar
            .as_ref()
            .map(|avatar| &avatar.thumbnail),
        Some(&thumbnail)
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
        latest_event: None,
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
                latest_event: None,
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
                latest_event: None,
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
                latest_event: None,
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
                    latest_event: None,
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
                    latest_event: None,
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
            latest_event: None,
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
                latest_event: None,
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
        latest_event: None,
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
fn sidebar_aggregate_badges_ignore_muted_rooms_but_room_items_keep_counts() {
    let notification_settings = HashMap::from([(
        "room-a".to_owned(),
        RoomNotificationSettings {
            mode: RoomNotificationMode::Mute,
            ..RoomNotificationSettings::default()
        },
    )]);

    let sidebar = compose_sidebar_with_room_notification_settings(
        None,
        &spaces(),
        &rooms(),
        &notification_settings,
    );

    assert_eq!(
        sidebar
            .space_rooms
            .iter()
            .find(|room| room.room_id == "room-a")
            .map(|room| room.unread_count),
        Some(5),
        "muted rooms still show their own unread count"
    );
    assert_eq!(sidebar.account_home.unread_count, 5);
    assert_eq!(sidebar.account_home.highlight_count, 0);
    assert_eq!(sidebar.space_rail[0].unread_count, 0);
    assert_eq!(sidebar.space_rail[0].highlight_count, 0);
    assert_eq!(sidebar.space_unread_count, 2);
    assert_eq!(sidebar.dm_unread_count, 3);
}

#[test]
fn sidebar_badges_ignore_plain_unread_counts_absent_from_activity_unread() {
    let spaces = vec![SpaceSummary {
        space_id: "space-a".to_owned(),
        display_name: "Space A".to_owned(),
        avatar: None,
        child_room_ids: vec!["plain".to_owned(), "notified".to_owned()],
    }];
    let rooms = vec![
        RoomSummary {
            room_id: "plain".to_owned(),
            display_name: "Plain".to_owned(),
            display_label: "Plain".to_owned(),
            original_display_label: "Plain".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 1,
            notification_count: 0,
            highlight_count: 0,
            marked_unread: false,
            last_activity_ms: 0,
            latest_event: None,
            parent_space_ids: vec!["space-a".to_owned()],
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 0,
        },
        RoomSummary {
            room_id: "notified".to_owned(),
            display_name: "Notified".to_owned(),
            display_label: "Notified".to_owned(),
            original_display_label: "Notified".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 4,
            notification_count: 2,
            highlight_count: 0,
            marked_unread: false,
            last_activity_ms: 0,
            latest_event: None,
            parent_space_ids: vec!["space-a".to_owned()],
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 0,
        },
        RoomSummary {
            room_id: "marked-dm".to_owned(),
            display_name: "Marked DM".to_owned(),
            display_label: "Marked DM".to_owned(),
            original_display_label: "Marked DM".to_owned(),
            avatar: None,
            is_dm: true,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            marked_unread: true,
            last_activity_ms: 0,
            latest_event: None,
            parent_space_ids: vec!["space-a".to_owned()],
            dm_space_ids: vec!["space-a".to_owned()],
            is_encrypted: false,
            joined_members: 0,
        },
    ];

    let sidebar = compose_sidebar(None, &spaces, &rooms);

    assert_eq!(sidebar.account_home.unread_count, 3);
    assert_eq!(sidebar.space_rail[0].unread_count, 2);
    assert_eq!(sidebar.space_unread_count, 2);
    assert_eq!(sidebar.dm_unread_count, 1);
    assert_eq!(
        sidebar
            .space_rooms
            .iter()
            .find(|room| room.room_id == "plain")
            .map(|room| room.unread_count),
        Some(0)
    );
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
        latest_event: None,
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
            latest_event: None,
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
            latest_event: None,
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

    assert_eq!(value["account_home"]["unread_count"], json!(10));
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
            latest_event: None,
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
            latest_event: None,
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

#[test]
fn legacy_navigation_json_without_scroll_anchors_loads_with_empty_map() {
    let json = r#"{
        "active_space_id": "!space:test.example.com",
        "active_room_id": "!room:test.example.com",
        "space_order": ["!space:test.example.com"],
        "last_room_by_space_id": {"!space:test.example.com": "!room:test.example.com"}
    }"#;

    let navigation: koushi_state::NavigationState =
        serde_json::from_str(json).expect("deserialize legacy navigation");

    assert!(navigation.room_scroll_anchors.is_empty());
    assert_eq!(
        navigation.active_space_id.as_deref(),
        Some("!space:test.example.com")
    );
    assert_eq!(
        navigation.active_room_id.as_deref(),
        Some("!room:test.example.com")
    );
}

#[test]
fn legacy_navigation_scroll_anchor_without_edge_defaults_to_top() {
    let json = r#"{
        "active_space_id": "!space:test.example.com",
        "active_room_id": "!room:test.example.com",
        "space_order": ["!space:test.example.com"],
        "last_room_by_space_id": {"!space:test.example.com": "!room:test.example.com"},
        "room_scroll_anchors": {
            "!room:test.example.com": {
                "event_id": "$anchor:event",
                "offset_px": 24,
                "updated_at_ms": 1820000000000
            }
        }
    }"#;

    let navigation: koushi_state::NavigationState =
        serde_json::from_str(json).expect("deserialize legacy navigation scroll anchor");

    let anchor = navigation
        .room_scroll_anchors
        .get("!room:test.example.com")
        .expect("legacy anchor should survive");
    assert_eq!(anchor.edge, TimelineScrollAnchorEdge::Top);
    assert_eq!(anchor.event_id, "$anchor:event");
    assert_eq!(anchor.offset_px, 24);
    assert_eq!(anchor.updated_at_ms, 1_820_000_000_000);
}

#[test]
fn navigation_state_round_trips_scroll_anchors_through_serde() {
    let navigation = koushi_state::NavigationState {
        active_space_id: Some("!space:test.example.com".to_owned()),
        active_room_id: Some("!room:test.example.com".to_owned()),
        space_order: vec!["!space:test.example.com".to_owned()],
        last_room_by_space_id: BTreeMap::from([(
            "!space:test.example.com".to_owned(),
            "!room:test.example.com".to_owned(),
        )]),
        room_scroll_anchors: BTreeMap::from([(
            "!room:test.example.com".to_owned(),
            koushi_state::TimelineScrollAnchor {
                event_id: "$anchor:event".to_owned(),
                edge: TimelineScrollAnchorEdge::Top,
                offset_px: 24,
                updated_at_ms: 1_820_000_000_000,
            },
        )]),
        main_timeline_anchor: None,
    };

    let encoded = serde_json::to_string(&navigation).expect("serialize navigation");
    let decoded: koushi_state::NavigationState =
        serde_json::from_str(&encoded).expect("deserialize navigation");

    assert_eq!(decoded, navigation);
}

#[test]
fn room_list_sort_supports_recent_and_locale_modes() {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        rooms: vec![
            RoomSummary {
                room_id: "room-b".to_owned(),
                display_name: "Beta".to_owned(),
                display_label: "Beta".to_owned(),
                original_display_label: "Beta".to_owned(),
                avatar: None,
                is_dm: false,
                dm_user_ids: Vec::new(),
                tags: RoomTags::default(),
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                marked_unread: false,
                last_activity_ms: 2000,
                latest_event: None,
                parent_space_ids: Vec::new(),
                dm_space_ids: Vec::new(),
                is_encrypted: false,
                joined_members: 0,
            },
            RoomSummary {
                room_id: "room-a".to_owned(),
                display_name: "Alpha".to_owned(),
                display_label: "Alpha".to_owned(),
                original_display_label: "Alpha".to_owned(),
                avatar: None,
                is_dm: false,
                dm_user_ids: Vec::new(),
                tags: RoomTags::default(),
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                marked_unread: false,
                last_activity_ms: 1000,
                latest_event: None,
                parent_space_ids: Vec::new(),
                dm_space_ids: Vec::new(),
                is_encrypted: false,
                joined_members: 0,
            },
        ],
        ..AppState::default()
    };

    reduce(
        &mut state,
        AppAction::RoomListFilterSelected {
            filter: RoomListFilter::Unread,
        },
    );
    reduce(
        &mut state,
        AppAction::RoomListFilterSelected {
            filter: RoomListFilter::Rooms,
        },
    );
    assert_eq!(state.room_list.sort, RoomListSort::Activity);
    assert_eq!(
        state
            .room_list
            .items
            .iter()
            .map(|i| i.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["room-b", "room-a"]
    );

    reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 1,
            patch: koushi_state::SettingsPatch {
                room_list_sort: Some(RoomListSort::NormalLocale),
                ..koushi_state::SettingsPatch::default()
            },
        },
    );
    reduce(
        &mut state,
        AppAction::RoomListFilterSelected {
            filter: RoomListFilter::Unread,
        },
    );
    reduce(
        &mut state,
        AppAction::RoomListFilterSelected {
            filter: RoomListFilter::Rooms,
        },
    );
    assert_eq!(state.room_list.sort, RoomListSort::NormalLocale);
    assert_eq!(
        state
            .room_list
            .items
            .iter()
            .map(|i| i.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["room-a", "room-b"]
    );

    reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 2,
            patch: koushi_state::SettingsPatch {
                room_list_sort: Some(RoomListSort::RecentFirst),
                ..koushi_state::SettingsPatch::default()
            },
        },
    );
    reduce(
        &mut state,
        AppAction::RoomListFilterSelected {
            filter: RoomListFilter::Unread,
        },
    );
    reduce(
        &mut state,
        AppAction::RoomListFilterSelected {
            filter: RoomListFilter::Rooms,
        },
    );
    assert_eq!(state.room_list.sort, RoomListSort::RecentFirst);
    assert_eq!(
        state
            .room_list
            .items
            .iter()
            .map(|i| i.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["room-b", "room-a"]
    );
}

// #161: main-pane timeline mode is a guarded Live <-> Anchored state machine.
#[test]
fn main_timeline_anchor_enters_returns_and_resets_on_room_change() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: spaces(),
            rooms: rooms(),
        },
    );
    assert_eq!(state.navigation.active_room_id.as_deref(), Some("room-a"));
    assert_eq!(state.navigation.main_timeline_anchor, None);

    // Enter anchored mode for the active room.
    let effects = reduce(
        &mut state,
        AppAction::EnterAnchoredTimeline {
            room_id: "room-a".to_owned(),
            event_id: "$deep-event".to_owned(),
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.navigation.main_timeline_anchor,
        Some(MainTimelineAnchor {
            event_id: "$deep-event".to_owned(),
        })
    );

    // Returning to live clears the anchor.
    reduce(
        &mut state,
        AppAction::ReturnMainTimelineToLive {
            room_id: "room-a".to_owned(),
        },
    );
    assert_eq!(state.navigation.main_timeline_anchor, None);

    // Re-enter, then switch rooms -> the anchor resets to live.
    reduce(
        &mut state,
        AppAction::EnterAnchoredTimeline {
            room_id: "room-a".to_owned(),
            event_id: "$deep-event".to_owned(),
        },
    );
    assert!(state.navigation.main_timeline_anchor.is_some());
    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "global-room".to_owned(),
        },
    );
    assert_eq!(
        state.navigation.active_room_id.as_deref(),
        Some("global-room")
    );
    assert_eq!(state.navigation.main_timeline_anchor, None);
}

#[test]
fn main_timeline_anchor_is_guarded_by_session_and_active_room() {
    // Not session-ready -> no-op.
    let mut signed_out = AppState::default();
    reduce(
        &mut signed_out,
        AppAction::EnterAnchoredTimeline {
            room_id: "room-a".to_owned(),
            event_id: "$e".to_owned(),
        },
    );
    assert_eq!(signed_out.navigation.main_timeline_anchor, None);

    // Ready, but the target room is not the active room -> no-op.
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: spaces(),
            rooms: rooms(),
        },
    );
    assert_eq!(state.navigation.active_room_id.as_deref(), Some("room-a"));
    reduce(
        &mut state,
        AppAction::EnterAnchoredTimeline {
            room_id: "global-room".to_owned(),
            event_id: "$e".to_owned(),
        },
    );
    assert_eq!(state.navigation.main_timeline_anchor, None);
}

// #161: jump-to-date renders the focused timeline in the main pane; closing the
// focused context (live-edge return) clears the main-pane anchor.
#[test]
fn closing_focused_context_returns_main_pane_to_live() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: spaces(),
            rooms: rooms(),
        },
    );
    // Seed a persisted live-timeline scroll anchor for the room.
    reduce(
        &mut state,
        AppAction::TimelineScrollAnchorUpdated {
            room_id: "room-a".to_owned(),
            anchor: koushi_state::TimelineScrollAnchor {
                event_id: "$old-live-pos".to_owned(),
                edge: TimelineScrollAnchorEdge::Bottom,
                offset_px: 0,
                updated_at_ms: 1_700_000_000_000,
            },
        },
    );
    assert!(state.navigation.room_scroll_anchors.contains_key("room-a"));

    reduce(
        &mut state,
        AppAction::OpenFocusedContext {
            room_id: "room-a".to_owned(),
            event_id: "$deep-event".to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::EnterAnchoredTimeline {
            room_id: "room-a".to_owned(),
            event_id: "$deep-event".to_owned(),
        },
    );
    assert!(state.navigation.main_timeline_anchor.is_some());

    reduce(&mut state, AppAction::CloseFocusedContext);
    assert_eq!(state.navigation.main_timeline_anchor, None);
    // #161: returning to live from the anchored view drops the stale room scroll
    // anchor so the live timeline pins to the live edge, not a pre-jump position.
    assert!(!state.navigation.room_scroll_anchors.contains_key("room-a"));
}
