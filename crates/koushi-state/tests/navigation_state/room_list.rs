use super::support::{
    avatar, initial_attention_diagnostic, ready_state, rooms, search_crawler_settings_standard,
    session_info, spaces,
};
use koushi_state::{
    AppAction, AppEffect, AppState, AvatarImage, AvatarThumbnailState, InvitePreview,
    NativeAttentionObservationKind, NativeAttentionProjectionInput, RoomListFilter, RoomSummary,
    RoomTags, SessionState, SpaceSummary, ThreadPaneState, TimelinePaneState, UiEvent, UserProfile,
    compose_sidebar, native_attention_state_from_rooms, reduce,
};
use serde_json::json;
use std::collections::BTreeMap;

fn ready_avatar_thumbnail(label: &str) -> AvatarThumbnailState {
    AvatarThumbnailState::Ready {
        source_ref: format!("avatar/{:016x}", label.len()),
        width: Some(64),
        height: Some(64),
        mime_type: Some("image/png".to_owned()),
    }
}

#[test]
fn room_list_source_wire_is_exactly_cache_or_live() {
    for (wire, expected) in [
        (json!("cache"), koushi_state::RoomListSource::Cache),
        (json!("live"), koushi_state::RoomListSource::Live),
    ] {
        assert_eq!(
            serde_json::from_value::<koushi_state::RoomListSource>(wire)
                .expect("supported room-list source"),
            expected
        );
    }

    for unsupported in [json!("legacy"), json!("syncService")] {
        assert!(
            serde_json::from_value::<koushi_state::RoomListSource>(unsupported).is_err(),
            "obsolete room-list source must be rejected"
        );
    }
}

#[test]
fn room_list_readiness_round_trips_every_engine_neutral_wire_state() {
    let cases = [
        (
            koushi_state::RoomListReadiness::Uninitialized,
            json!({"kind": "uninitialized"}),
        ),
        (
            koushi_state::RoomListReadiness::Loading {
                source: koushi_state::RoomListSource::Live,
                generation: 7,
            },
            json!({"kind": "loading", "source": "live", "generation": 7}),
        ),
        (
            koushi_state::RoomListReadiness::Ready {
                source: koushi_state::RoomListSource::Live,
                generation: 8,
            },
            json!({"kind": "ready", "source": "live", "generation": 8}),
        ),
        (
            koushi_state::RoomListReadiness::Failed {
                source: koushi_state::RoomListSource::Cache,
                generation: 9,
                kind: koushi_state::RoomListFailureKind::Connectivity,
            },
            json!({
                "kind": "failed",
                "source": "cache",
                "generation": 9,
                "failureKind": "connectivity"
            }),
        ),
    ];

    for (readiness, expected) in cases {
        let encoded = serde_json::to_value(readiness).expect("serialize room-list readiness");
        assert_eq!(encoded, expected);
        let decoded: koushi_state::RoomListReadiness =
            serde_json::from_value(encoded).expect("deserialize room-list readiness");
        assert_eq!(decoded, readiness);
    }
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
        recency_stamp: None,
        conversation_activity: None,
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
fn room_summary_legacy_json_defaults_activity_fields_to_none() {
    let value = json!({
        "room_id": "dm-a",
        "display_name": "Alice",
        "display_label": "Alice",
        "is_dm": true,
        "tags": { "favourite": null, "low_priority": null },
        "unread_count": 0,
        "notification_count": 0,
        "highlight_count": 0,
        "parent_space_ids": [],
        "is_encrypted": false
    });

    let room: RoomSummary = serde_json::from_value(value).expect("deserialize legacy room");

    assert_eq!(room.recency_stamp, None);
    assert_eq!(room.conversation_activity, None);
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
                recency_stamp: None,
                conversation_activity: None,
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
                recency_stamp: None,
                conversation_activity: None,
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
        recency_stamp: None,
        conversation_activity: None,
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
        recency_stamp: None,
        conversation_activity: None,
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
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::EmitUiEvent(UiEvent::ProfileChanged(_))))
    );
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
            initial_attention_diagnostic(10, 3, false),
            AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged),
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
fn room_list_update_is_ignored_while_session_is_locked() {
    let mut state = ready_state();
    state.session = SessionState::Locked(session_info());

    let effects = reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: spaces(),
            rooms: rooms(),
        },
    );

    assert!(state.spaces.is_empty());
    assert!(state.rooms.is_empty());
    assert!(effects.is_empty());
}

#[test]
fn transient_room_list_update_is_whole_state_inert_before_readiness_bump() {
    for session in [
        SessionState::Locked(session_info()),
        SessionState::SignedOut,
        SessionState::SwitchingAccount {
            info: session_info(),
        },
    ] {
        let mut state = ready_state();
        state.session = session;
        state.room_list.readiness = koushi_state::RoomListReadiness::Uninitialized;
        state.spaces = spaces();
        state.rooms = rooms();
        let before = state.clone();

        assert!(
            reduce(
                &mut state,
                AppAction::RoomListUpdated {
                    spaces: spaces(),
                    rooms: rooms(),
                },
            )
            .is_empty()
        );
        assert_eq!(state, before);
    }
}

#[test]
fn transient_room_list_snapshots_are_whole_state_inert_before_invites_write() {
    let existing_invite = InvitePreview {
        room_id: "!existing:example.invalid".to_owned(),
        display_name: "Existing invite".to_owned(),
        avatar: None,
        topic: None,
        inviter_display_name: None,
        inviter_user_id: None,
        is_dm: false,
    };

    for session in [
        SessionState::Locked(session_info()),
        SessionState::SignedOut,
        SessionState::SwitchingAccount {
            info: session_info(),
        },
    ] {
        let mut provisional = ready_state();
        provisional.session = session.clone();
        provisional.room_list.readiness = koushi_state::RoomListReadiness::Uninitialized;
        provisional.invites = vec![existing_invite.clone()];
        provisional.spaces = spaces();
        provisional.rooms = rooms();
        let before = provisional.clone();
        assert!(
            reduce(
                &mut provisional,
                AppAction::RoomListSnapshotProvisional {
                    generation: 0,
                    source: koushi_state::RoomListSource::Cache,
                    spaces: spaces(),
                    rooms: rooms(),
                    invites: vec![],
                },
            )
            .is_empty()
        );
        assert_eq!(provisional, before);

        let mut authoritative = ready_state();
        authoritative.session = session;
        authoritative.room_list.readiness = koushi_state::RoomListReadiness::Ready {
            source: koushi_state::RoomListSource::Cache,
            generation: 0,
        };
        authoritative.invites = vec![existing_invite.clone()];
        authoritative.spaces = spaces();
        authoritative.rooms = rooms();
        let before = authoritative.clone();
        assert!(
            reduce(
                &mut authoritative,
                AppAction::RoomListSnapshotAuthoritative {
                    generation: 0,
                    source: koushi_state::RoomListSource::Cache,
                    spaces: vec![],
                    rooms: vec![],
                    invites: vec![],
                },
            )
            .is_empty()
        );
        assert_eq!(authoritative, before);
    }
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
            initial_attention_diagnostic(10, 3, false),
            AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged),
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
            submission_registry: Default::default(),
            scheduled_send_capability: Default::default(),
            scheduled_sends: Vec::new(),
            staged_uploads: Vec::new(),
            media_gallery: Vec::new(),
            media_downloads: Default::default(),
            continuity: Default::default(),
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
                recency_stamp: None,
                conversation_activity: None,
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
            initial_attention_diagnostic(0, 1, false),
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
                recency_stamp: None,
                conversation_activity: None,
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
                recency_stamp: None,
                conversation_activity: None,
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
            submission_registry: Default::default(),
            scheduled_send_capability: Default::default(),
            scheduled_sends: Vec::new(),
            staged_uploads: Vec::new(),
            media_gallery: Vec::new(),
            media_downloads: Default::default(),
            continuity: Default::default(),
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
                    recency_stamp: None,
                    conversation_activity: None,
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
                    recency_stamp: None,
                    conversation_activity: None,
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
            initial_attention_diagnostic(7, 2, true),
            AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged),
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
            recency_stamp: None,
            conversation_activity: None,
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
            submission_registry: Default::default(),
            scheduled_send_capability: Default::default(),
            scheduled_sends: Vec::new(),
            staged_uploads: Vec::new(),
            media_gallery: Vec::new(),
            media_downloads: Default::default(),
            continuity: Default::default(),
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
                recency_stamp: None,
                conversation_activity: None,
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
            initial_attention_diagnostic(2, 1, false),
            AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged),
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
            submission_registry: Default::default(),
            scheduled_send_capability: Default::default(),
            scheduled_sends: Vec::new(),
            staged_uploads: Vec::new(),
            media_gallery: Vec::new(),
            media_downloads: Default::default(),
            continuity: Default::default(),
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
            initial_attention_diagnostic(10, 3, false),
            AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged),
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
