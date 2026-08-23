use super::support::{avatar, ready_state, rooms, session_info, spaces};
use koushi_state::{
    AppAction, AppEffect, AppState, ConversationActivity, ConversationActivitySource,
    RoomLatestEventSummary, RoomListFilter, RoomListSort, RoomNotificationMode,
    RoomNotificationSettings, RoomSummary, RoomTags, SessionState, SpaceSummary, UiEvent,
    compose_sidebar, compose_sidebar_with_account_facts, compute_room_list_projection, reduce,
};
use serde_json::json;
use std::collections::HashMap;

fn active_sort_room(
    room_id: &str,
    is_dm: bool,
    parent_space_ids: &[&str],
    dm_space_ids: &[&str],
    activity_timestamp_ms: Option<u64>,
) -> RoomSummary {
    RoomSummary {
        room_id: room_id.to_owned(),
        display_name: room_id.to_owned(),
        display_label: room_id.to_owned(),
        original_display_label: room_id.to_owned(),
        avatar: None,
        is_dm,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        marked_unread: false,
        recency_stamp: activity_timestamp_ms,
        conversation_activity: activity_timestamp_ms.map(|timestamp_ms| ConversationActivity {
            timestamp_ms,
            source: ConversationActivitySource::Message,
        }),
        latest_event: None,
        parent_space_ids: parent_space_ids
            .iter()
            .map(|space_id| (*space_id).to_owned())
            .collect(),
        dm_space_ids: dm_space_ids
            .iter()
            .map(|space_id| (*space_id).to_owned())
            .collect(),
        is_encrypted: false,
        joined_members: 0,
    }
}

fn dm_room_for_activity(
    room_id: &str,
    display_label: &str,
    conversation_activity: Option<ConversationActivity>,
) -> RoomSummary {
    RoomSummary {
        room_id: room_id.to_owned(),
        display_name: display_label.to_owned(),
        display_label: display_label.to_owned(),
        original_display_label: display_label.to_owned(),
        avatar: None,
        is_dm: true,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        marked_unread: false,
        recency_stamp: Some(999),
        conversation_activity,
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 0,
    }
}

fn latest_message(event_id: &str, timestamp_ms: u64) -> RoomLatestEventSummary {
    RoomLatestEventSummary {
        event_id: event_id.to_owned(),
        relation_type: None,
        relation_event_id: None,
        sender_id: Some("@sender:example.invalid".to_owned()),
        sender_label: Some("Sender".to_owned()),
        sender_avatar: None,
        preview: Some("latest message".to_owned()),
        timestamp_ms,
        is_redacted: false,
    }
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

    let sidebar =
        compose_sidebar_with_account_facts(None, &spaces(), &rooms(), &notification_settings, 0);

    let muted = sidebar
        .space_rooms
        .iter()
        .find(|room| room.room_id == "room-a")
        .expect("muted room should remain in the room list");
    assert_eq!(
        muted.unread_count, 5,
        "muted rooms keep their activity count"
    );
    assert_eq!(
        muted.display_count, 5,
        "muted rows display raw unread messages"
    );
    assert!(muted.is_muted);
    assert!(!muted.is_attention_highlighted);
    assert_eq!(sidebar.account_home.unread_count, 5);
    assert_eq!(sidebar.account_home.highlight_count, 0);
    assert_eq!(sidebar.space_rail[0].unread_count, 0);
    assert_eq!(sidebar.space_rail[0].highlight_count, 0);
    assert_eq!(sidebar.space_unread_count, 2);
    assert_eq!(sidebar.dm_unread_count, 3);
}

#[test]
fn home_attention_counts_invites_separately_from_unread_messages() {
    // #330: the Home rail badge is unread messages plus pending invites, but the
    // two must stay separately readable — the accessible label names them
    // individually, and `unread_count` keeps meaning only messages.
    let without_invites =
        compose_sidebar_with_account_facts(None, &spaces(), &rooms(), &HashMap::new(), 0);
    assert_eq!(without_invites.account_home.unread_count, 10);
    assert_eq!(without_invites.account_home.invite_count, 0);
    assert_eq!(
        without_invites.account_home.attention_count, 10,
        "with no invites the total is the unread message count"
    );

    let with_invites =
        compose_sidebar_with_account_facts(None, &spaces(), &rooms(), &HashMap::new(), 2);
    assert_eq!(
        with_invites.account_home.unread_count, 10,
        "invites must not be folded into the unread message count"
    );
    assert_eq!(with_invites.account_home.invite_count, 2);
    assert_eq!(with_invites.account_home.attention_count, 12);

    assert_eq!(
        with_invites.space_rail[0].unread_count, without_invites.space_rail[0].unread_count,
        "space rail badges stay space unread only"
    );
}

#[test]
fn home_attention_counts_invites_even_when_every_room_is_muted() {
    // Invites are account-level, so a muted room silencing the message count
    // must not silence the invite count with it.
    let muted = HashMap::from([
        (
            "room-a".to_owned(),
            RoomNotificationSettings {
                mode: RoomNotificationMode::Mute,
                ..RoomNotificationSettings::default()
            },
        ),
        (
            "global-room".to_owned(),
            RoomNotificationSettings {
                mode: RoomNotificationMode::Mute,
                ..RoomNotificationSettings::default()
            },
        ),
        (
            "dm-a".to_owned(),
            RoomNotificationSettings {
                mode: RoomNotificationMode::Mute,
                ..RoomNotificationSettings::default()
            },
        ),
    ]);

    let sidebar = compose_sidebar_with_account_facts(None, &spaces(), &rooms(), &muted, 3);

    assert_eq!(sidebar.account_home.unread_count, 0);
    assert_eq!(sidebar.account_home.invite_count, 3);
    assert_eq!(sidebar.account_home.attention_count, 3);
}

#[test]
fn compose_sidebar_without_invites_reports_no_pending_invites() {
    // The three-argument wrapper has no invite input, so it must report zero
    // rather than guess — its callers are projections that do not own invites.
    let sidebar = compose_sidebar(None, &spaces(), &rooms());

    assert_eq!(sidebar.account_home.invite_count, 0);
    assert_eq!(
        sidebar.account_home.attention_count, sidebar.account_home.unread_count,
        "with no invite input the total is the unread message count"
    );
}

#[test]
fn sidebar_badges_include_plain_unread_counts_and_keep_display_semantics() {
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
            recency_stamp: None,
            conversation_activity: None,
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
            recency_stamp: None,
            conversation_activity: None,
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
            recency_stamp: None,
            conversation_activity: None,
            latest_event: None,
            parent_space_ids: vec!["space-a".to_owned()],
            dm_space_ids: vec!["space-a".to_owned()],
            is_encrypted: false,
            joined_members: 0,
        },
    ];

    let sidebar = compose_sidebar(None, &spaces, &rooms);

    assert_eq!(sidebar.account_home.unread_count, 6);
    assert_eq!(sidebar.space_rail[0].unread_count, 5);
    assert_eq!(sidebar.space_unread_count, 5);
    assert_eq!(sidebar.dm_unread_count, 1);
    let plain = sidebar
        .space_rooms
        .iter()
        .find(|room| room.room_id == "plain")
        .expect("plain room should be projected");
    assert_eq!(plain.unread_count, 1);
    assert_eq!(plain.display_count, 0);
    assert!(plain.has_unread_content);
    assert!(!plain.is_attention_highlighted);
    assert!(!plain.has_unread_mention);

    let notified = sidebar
        .space_rooms
        .iter()
        .find(|room| room.room_id == "notified")
        .expect("notified room should be projected");
    assert_eq!(notified.display_count, 2);
    assert!(notified.is_attention_highlighted);

    let marked = sidebar
        .global_dms
        .iter()
        .find(|room| room.room_id == "marked-dm")
        .expect("marked DM should be projected");
    assert_eq!(marked.display_count, 0);
    assert!(marked.has_unread_content);
}

#[test]
fn marking_a_room_unread_does_not_fabricate_an_unread_message_count() {
    let mut state = ready_state();
    let mut room = rooms().remove(0);
    room.unread_count = 0;
    room.notification_count = 0;
    room.highlight_count = 0;
    state.rooms = vec![room];

    reduce(
        &mut state,
        AppAction::RoomMarkedAsUnreadSucceeded {
            request_id: 1,
            room_id: "room-a".to_owned(),
            unread: true,
        },
    );

    let room = state.rooms.first().expect("room should remain projected");
    assert!(room.marked_unread);
    assert_eq!(room.unread_count, 0);
    assert_eq!(state.native_attention.summary.badge_count, 0);
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
        recency_stamp: None,
        conversation_activity: None,
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
            recency_stamp: None,
            conversation_activity: None,
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
            recency_stamp: None,
            conversation_activity: None,
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
            recency_stamp: None,
            conversation_activity: None,
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
            recency_stamp: None,
            conversation_activity: None,
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
                recency_stamp: Some(2000),
                conversation_activity: Some(ConversationActivity {
                    timestamp_ms: 2000,
                    source: ConversationActivitySource::Message,
                }),
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
                recency_stamp: Some(1000),
                conversation_activity: Some(ConversationActivity {
                    timestamp_ms: 1000,
                    source: ConversationActivitySource::Message,
                }),
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

#[test]
fn room_list_activity_sort_uses_latest_message_timestamp_before_status_activity() {
    let rooms = vec![
        RoomSummary {
            room_id: "status-newer".to_owned(),
            display_name: "Status Newer".to_owned(),
            display_label: "Status Newer".to_owned(),
            original_display_label: "Status Newer".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            marked_unread: false,
            recency_stamp: Some(300),
            conversation_activity: Some(ConversationActivity {
                timestamp_ms: 100,
                source: ConversationActivitySource::Message,
            }),
            latest_event: Some(latest_message("$status-newer", 100)),
            parent_space_ids: Vec::new(),
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 0,
        },
        RoomSummary {
            room_id: "message-newer".to_owned(),
            display_name: "Message Newer".to_owned(),
            display_label: "Message Newer".to_owned(),
            original_display_label: "Message Newer".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            marked_unread: false,
            recency_stamp: Some(200),
            conversation_activity: Some(ConversationActivity {
                timestamp_ms: 250,
                source: ConversationActivitySource::Message,
            }),
            latest_event: Some(latest_message("$message-newer", 250)),
            parent_space_ids: Vec::new(),
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 0,
        },
    ];
    let projection = compute_room_list_projection(
        RoomListFilter::Rooms,
        RoomListSort::Activity,
        None,
        &[],
        &rooms,
        &HashMap::new(),
        &[],
    );

    assert_eq!(
        projection
            .items
            .iter()
            .map(|item| item.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["message-newer", "status-newer"]
    );
}

#[test]
fn room_list_activity_sort_keeps_a_messaged_dm_ahead_of_a_newer_join_only_dm() {
    let rooms = vec![
        RoomSummary {
            room_id: "join-only".to_owned(),
            display_name: "New Contact".to_owned(),
            display_label: "New Contact".to_owned(),
            original_display_label: "New Contact".to_owned(),
            avatar: None,
            is_dm: true,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            marked_unread: false,
            recency_stamp: Some(300),
            conversation_activity: None,
            latest_event: None,
            parent_space_ids: Vec::new(),
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 0,
        },
        RoomSummary {
            room_id: "messaged".to_owned(),
            display_name: "Existing Contact".to_owned(),
            display_label: "Existing Contact".to_owned(),
            original_display_label: "Existing Contact".to_owned(),
            avatar: None,
            is_dm: true,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 0,
            notification_count: 0,
            highlight_count: 0,
            marked_unread: false,
            recency_stamp: Some(200),
            conversation_activity: Some(ConversationActivity {
                timestamp_ms: 200,
                source: ConversationActivitySource::Message,
            }),
            latest_event: Some(latest_message("$message", 200)),
            parent_space_ids: Vec::new(),
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 0,
        },
    ];

    let projection = compute_room_list_projection(
        RoomListFilter::People,
        RoomListSort::Activity,
        None,
        &[],
        &rooms,
        &HashMap::new(),
        &[],
    );

    assert_eq!(
        projection
            .items
            .iter()
            .map(|item| item.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["messaged", "join-only"]
    );
}

#[test]
fn room_list_activity_sort_uses_labels_then_room_ids_as_a_stable_fallback() {
    let rooms = vec![
        dm_room_for_activity("room-z", "alpha", None),
        dm_room_for_activity("room-b", "Beta", None),
        dm_room_for_activity("room-a", "beta", None),
        dm_room_for_activity(
            "room-c",
            "Later Label",
            Some(ConversationActivity {
                timestamp_ms: 42,
                source: ConversationActivitySource::EncryptedMessage,
            }),
        ),
        dm_room_for_activity(
            "room-d",
            "Earlier Label",
            Some(ConversationActivity {
                timestamp_ms: 42,
                source: ConversationActivitySource::ThreadReply,
            }),
        ),
    ];

    let projection = compute_room_list_projection(
        RoomListFilter::People,
        RoomListSort::Activity,
        None,
        &[],
        &rooms,
        &HashMap::new(),
        &[],
    );

    assert_eq!(
        projection
            .items
            .iter()
            .map(|item| item.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["room-d", "room-c", "room-z", "room-a", "room-b"]
    );
}

#[test]
fn active_sort_prioritizes_attention_before_newer_activity_and_keeps_stable_fallbacks() {
    let mut mention = active_sort_room("mention", false, &[], &[], Some(10));
    mention.highlight_count = 1;
    mention.notification_count = 1;

    let mut notification = active_sort_room("notification", false, &[], &[], Some(900));
    notification.notification_count = 1;

    let mut ordinary = active_sort_room("ordinary", false, &[], &[], Some(800));
    ordinary.unread_count = 1;

    let mut manual = active_sort_room("manual", false, &[], &[], Some(700));
    manual.marked_unread = true;

    let rooms = vec![
        active_sort_room("read-newest", false, &[], &[], Some(1_000)),
        notification,
        ordinary,
        manual,
        mention,
        active_sort_room("same-b", false, &[], &[], Some(42)),
        active_sort_room("same-a", false, &[], &[], Some(42)),
        active_sort_room("missing-b", false, &[], &[], None),
        active_sort_room("missing-a", false, &[], &[], None),
    ];

    let projection = compute_room_list_projection(
        RoomListFilter::Rooms,
        RoomListSort::Activity,
        None,
        &[],
        &rooms,
        &HashMap::new(),
        &[],
    );

    assert_eq!(
        projection
            .items
            .iter()
            .map(|item| item.room_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "mention",
            "notification",
            "ordinary",
            "manual",
            "read-newest",
            "same-a",
            "same-b",
            "missing-a",
            "missing-b",
        ]
    );
}

#[test]
fn active_sort_orders_sidebar_rooms_and_dms_in_home_and_active_space() {
    let spaces = vec![SpaceSummary {
        space_id: "space-a".to_owned(),
        display_name: "Space A".to_owned(),
        avatar: None,
        child_room_ids: vec![
            "space-read".to_owned(),
            "space-notification".to_owned(),
            "space-mention".to_owned(),
        ],
    }];
    let mut space_mention = active_sort_room("space-mention", false, &["space-a"], &[], Some(10));
    space_mention.highlight_count = 1;
    let mut space_notification =
        active_sort_room("space-notification", false, &["space-a"], &[], Some(900));
    space_notification.notification_count = 1;
    let mut home_ordinary = active_sort_room("home-ordinary", false, &[], &[], Some(800));
    home_ordinary.unread_count = 1;
    let mut dm_mention = active_sort_room("dm-mention", true, &[], &["space-a"], Some(10));
    dm_mention.highlight_count = 1;
    let mut dm_notification =
        active_sort_room("dm-notification", true, &[], &["space-a"], Some(900));
    dm_notification.notification_count = 1;

    let rooms = vec![
        active_sort_room("space-read", false, &["space-a"], &[], Some(1_000)),
        space_notification,
        space_mention,
        home_ordinary,
        active_sort_room("dm-read", true, &[], &["space-a"], Some(1_000)),
        dm_notification,
        dm_mention,
        active_sort_room("dm-outside", true, &[], &[], Some(950)),
    ];

    let home = compose_sidebar(None, &spaces, &rooms);
    assert_eq!(
        home.space_rooms
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "space-mention",
            "space-notification",
            "home-ordinary",
            "space-read",
        ]
    );
    assert_eq!(
        home.global_dms
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["dm-mention", "dm-notification", "dm-read", "dm-outside"]
    );

    let active_space = compose_sidebar(Some("space-a"), &spaces, &rooms);
    assert_eq!(
        active_space
            .space_rooms
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["space-mention", "space-notification", "space-read"]
    );
    assert_eq!(
        active_space
            .global_dms
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["dm-mention", "dm-notification", "dm-read"]
    );
}

#[test]
fn active_sort_recomputes_from_mute_and_mentions_actions_without_changing_selection() {
    let mut state = ready_state();
    state.navigation.active_room_id = Some("selected".to_owned());

    let mut mention = active_sort_room("mention", false, &[], &[], Some(100));
    mention.highlight_count = 1;
    let mut notification = active_sort_room("notification", false, &[], &[], Some(900));
    notification.notification_count = 1;
    let mut ordinary = active_sort_room("ordinary", false, &[], &[], Some(950));
    ordinary.unread_count = 1;
    state.rooms = vec![
        active_sort_room("selected", false, &[], &[], Some(1_000)),
        notification,
        ordinary,
        mention,
    ];

    let settings = state.settings.values.clone();
    reduce(&mut state, AppAction::SettingsLoaded { values: settings });
    assert_eq!(
        state
            .room_list
            .items
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["mention", "notification", "ordinary", "selected"]
    );

    let mute_effects = reduce(
        &mut state,
        AppAction::RoomNotificationModeSet {
            request_id: 1,
            room_id: "mention".to_owned(),
            mode: RoomNotificationMode::Mute,
        },
    );
    assert!(mute_effects.contains(&AppEffect::EmitUiEvent(UiEvent::RoomListChanged)));
    assert_eq!(
        state
            .room_list
            .items
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["notification", "ordinary", "mention", "selected"]
    );

    let mentions_effects = reduce(
        &mut state,
        AppAction::RoomNotificationModeSet {
            request_id: 2,
            room_id: "notification".to_owned(),
            mode: RoomNotificationMode::Mentions,
        },
    );
    assert!(mentions_effects.contains(&AppEffect::EmitUiEvent(UiEvent::RoomListChanged)));
    assert_eq!(
        state
            .room_list
            .items
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["ordinary", "notification", "mention", "selected"]
    );

    let read_effects = reduce(
        &mut state,
        AppAction::RoomMarkedAsReadSucceeded {
            request_id: 3,
            room_id: "ordinary".to_owned(),
        },
    );
    assert!(read_effects.contains(&AppEffect::EmitUiEvent(UiEvent::RoomListChanged)));
    assert_eq!(
        state
            .room_list
            .items
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["notification", "mention", "selected", "ordinary"]
    );

    let sidebar = compose_sidebar_with_account_facts(
        None,
        &state.spaces,
        &state.rooms,
        &state.room_notification_settings,
        0,
    );
    assert_eq!(
        sidebar
            .space_rooms
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["notification", "mention", "selected", "ordinary"]
    );
    assert_eq!(state.navigation.active_room_id.as_deref(), Some("selected"));
}

#[test]
fn sidebar_global_dms_uses_the_authoritative_conversation_activity_order() {
    let rooms = vec![
        dm_room_for_activity("join-only", "New Contact", None),
        dm_room_for_activity(
            "messaged",
            "Existing Contact",
            Some(ConversationActivity {
                timestamp_ms: 42,
                source: ConversationActivitySource::Message,
            }),
        ),
    ];

    let sidebar = compose_sidebar(None, &[], &rooms);

    assert_eq!(
        sidebar
            .global_dms
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["messaged", "join-only"]
    );
}
