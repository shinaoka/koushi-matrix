use koushi_state::{
    AppEffect, AppState, AvatarImage, AvatarThumbnailState, NativeAttentionObservationKind,
    NativeAttentionSuppressionReason, RoomSummary, RoomTags, SearchCrawlerSettings, SessionInfo,
    SessionState, SpaceSummary,
};

pub(super) fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.org".to_owned(),
        user_id: "@user-a:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    }
}

pub(super) fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(session_info()),
        ..AppState::default()
    }
}

pub(super) fn avatar(mxc_uri: &str) -> AvatarImage {
    AvatarImage {
        mxc_uri: mxc_uri.to_owned(),
        thumbnail: AvatarThumbnailState::NotRequested,
    }
}

pub(super) fn spaces() -> Vec<SpaceSummary> {
    vec![SpaceSummary {
        space_id: "space-a".to_owned(),
        raw_name: None,
        display_name: "Space A".to_owned(),
        avatar: None,
        join_rule: None,
        child_room_ids: vec!["room-a".to_owned(), "dm-a".to_owned()],
        parent_side_child_room_ids: vec!["room-a".to_owned(), "dm-a".to_owned()],
    }]
}

pub(super) fn search_crawler_settings_standard() -> SearchCrawlerSettings {
    SearchCrawlerSettings::default()
}

pub(super) fn initial_attention_diagnostic(
    unread_count: u64,
    badge_room_count: u64,
    active_room_match: bool,
) -> AppEffect {
    AppEffect::RecordNativeAttentionRecomputed {
        observation: NativeAttentionObservationKind::InitialSync,
        unread_count,
        notification_count: unread_count,
        badge_count: unread_count,
        badge_room_count,
        badge_excluded_room_count: 0,
        candidate: None,
        suppression: (unread_count > 0).then_some(NativeAttentionSuppressionReason::InitialSync),
        window_focused: true,
        active_room_match,
    }
}

pub(super) fn rooms() -> Vec<RoomSummary> {
    vec![
        RoomSummary {
            display_name_placeholder: None,
            display_label_placeholder: None,
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
            thread_unread_count: 0,
            thread_highlight_count: 0,
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
            display_name_placeholder: None,
            display_label_placeholder: None,
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
            thread_unread_count: 0,
            thread_highlight_count: 0,
            marked_unread: false,
            recency_stamp: None,
            conversation_activity: None,
            latest_event: None,
            parent_space_ids: vec!["space-a".to_owned()],
            dm_space_ids: vec!["space-a".to_owned()],
            is_encrypted: false,
            joined_members: 0,
        },
        RoomSummary {
            display_name_placeholder: None,
            display_label_placeholder: None,
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
            thread_unread_count: 0,
            thread_highlight_count: 0,
            marked_unread: false,
            recency_stamp: None,
            conversation_activity: None,
            latest_event: None,
            parent_space_ids: vec![],
            dm_space_ids: vec![],
            is_encrypted: false,
            joined_members: 0,
        },
    ]
}
