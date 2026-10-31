use super::support::{avatar, ready_state, rooms, session_info, spaces};
use koushi_state::{
    AppAction, AppEffect, AppState, ConversationActivity, ConversationActivitySource,
    RoomLatestEventSummary, RoomListFilter, RoomListSort, RoomNotificationMode,
    RoomNotificationSettings, RoomSummary, RoomTags, SessionState, SidebarScopeSettings,
    SidebarSectionSettings, SpaceSummary, UiEvent, compose_sidebar, compose_sidebar_for_state,
    compose_sidebar_with_account_facts, compute_room_list_projection, reduce,
};
use serde_json::json;
use std::collections::{BTreeMap, HashMap};

fn active_sort_room(
    room_id: &str,
    is_dm: bool,
    parent_space_ids: &[&str],
    dm_space_ids: &[&str],
    activity_timestamp_ms: Option<u64>,
) -> RoomSummary {
    RoomSummary {
        display_name_placeholder: None,
        display_label_placeholder: None,
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
        thread_unread_count: 0,
        thread_highlight_count: 0,
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

#[test]
fn unread_filter_excludes_server_only_notifications_without_sdk_unread() {
    let mut server_only = active_sort_room("server-only", false, &[], &[], Some(100));
    server_only.notification_count = 2;
    server_only.highlight_count = 1;
    let mut verified_thread = active_sort_room("verified-thread", false, &[], &[], Some(90));
    verified_thread.thread_unread_count = 1;

    let projection = compute_room_list_projection(
        RoomListFilter::Unread,
        RoomListSort::Activity,
        None,
        &[],
        &[server_only, verified_thread],
        &HashMap::new(),
        &[],
    );
    assert_eq!(
        projection
            .items
            .iter()
            .map(|item| item.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["verified-thread"]
    );
}

fn dm_room_for_activity(
    room_id: &str,
    display_label: &str,
    conversation_activity: Option<ConversationActivity>,
) -> RoomSummary {
    RoomSummary {
        display_name_placeholder: None,
        display_label_placeholder: None,
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
        thread_unread_count: 0,
        thread_highlight_count: 0,
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
        thread_root_event_id: None,
        sender_id: Some("@sender:example.invalid".to_owned()),
        sender_label: Some("Sender".to_owned()),
        sender_avatar: None,
        preview: Some("latest message".to_owned()),
        message_text: None,
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

    let sidebar = compose_sidebar_with_account_facts(
        Some("space-a"),
        &spaces(),
        &rooms(),
        &notification_settings,
        0,
    );

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
    // The muted room is this Space's only Rooms row, so its aggregate is empty.
    assert_eq!(sidebar.space_unread_count, 0);
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
        raw_name: None,
        display_name: "Space A".to_owned(),
        avatar: None,
        join_rule: None,
        topic: None,
        child_room_ids: vec!["plain".to_owned(), "notified".to_owned()],
        parent_side_child_room_ids: vec!["plain".to_owned(), "notified".to_owned()],
    }];
    let rooms = vec![
        RoomSummary {
            display_name_placeholder: None,
            display_label_placeholder: None,
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
            thread_unread_count: 0,
            thread_highlight_count: 0,
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

    let sidebar = compose_sidebar(Some("space-a"), &spaces, &rooms);

    assert_eq!(sidebar.account_home.unread_count, 5);
    assert_eq!(sidebar.space_rail[0].unread_count, 5);
    assert_eq!(sidebar.space_unread_count, 5);
    assert_eq!(sidebar.dm_unread_count, 0);
    let plain = sidebar
        .space_rooms
        .iter()
        .find(|room| room.room_id == "plain")
        .expect("plain room should be projected");
    assert_eq!(plain.unread_count, 1);
    assert_eq!(plain.display_count, 1);
    assert!(plain.has_unread_content);
    assert!(!plain.is_attention_highlighted);
    assert!(!plain.has_unread_mention);

    let notified = sidebar
        .space_rooms
        .iter()
        .find(|room| room.room_id == "notified")
        .expect("notified room should be projected");
    assert_eq!(notified.display_count, 4);
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
        display_name_placeholder: None,
        display_label_placeholder: None,
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
        thread_unread_count: 0,
        thread_highlight_count: 0,
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
            raw_name: None,
            display_name: "Space A".to_owned(),
            avatar: None,
            join_rule: None,
            topic: None,
            child_room_ids: vec!["room-a".to_owned(), "dm-multi".to_owned()],
            parent_side_child_room_ids: vec!["room-a".to_owned(), "dm-multi".to_owned()],
        },
        SpaceSummary {
            space_id: "space-b".to_owned(),
            raw_name: None,
            display_name: "Space B".to_owned(),
            avatar: None,
            join_rule: None,
            topic: None,
            child_room_ids: vec!["dm-multi".to_owned()],
            parent_side_child_room_ids: vec!["dm-multi".to_owned()],
        },
    ];
    let multi_rooms = vec![
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
            thread_unread_count: 0,
            thread_highlight_count: 0,
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
    let sidebar = compose_sidebar(Some("space-a"), &spaces(), &rooms());
    let value = serde_json::to_value(sidebar).expect("sidebar serializes");

    assert_eq!(value["account_home"]["unread_count"], json!(10));
    assert_eq!(value["space_rail"][0]["unread_count"], json!(5));
    assert_eq!(value["account_home"]["highlight_count"], json!(1));
    assert_eq!(value["space_rail"][0]["highlight_count"], json!(1));
    let room_a = value["space_rooms"]
        .as_array()
        .unwrap()
        .iter()
        .find(|room| room["room_id"] == "room-a")
        .unwrap();
    assert_eq!(room_a["unread_count"], json!(5));
    assert_eq!(room_a["highlight_count"], json!(1));
    assert_eq!(value["global_dms"][0]["unread_count"], json!(3));
    assert_eq!(value["global_dms"][0]["highlight_count"], json!(0));
    assert_eq!(value["space_unread_count"], json!(5));
    assert_eq!(value["dm_unread_count"], json!(3));
    assert_eq!(value["space_highlight_count"], json!(1));
    assert_eq!(value["dm_highlight_count"], json!(0));
}

#[test]
fn sidebar_items_carry_rust_owned_room_and_space_avatars() {
    let spaces = vec![SpaceSummary {
        space_id: "space-a".to_owned(),
        raw_name: None,
        display_name: "Space A".to_owned(),
        avatar: Some(avatar("mxc://example.invalid/space-a")),
        join_rule: None,
        topic: None,
        child_room_ids: vec!["room-a".to_owned(), "dm-a".to_owned()],
        parent_side_child_room_ids: vec!["room-a".to_owned(), "dm-a".to_owned()],
    }];
    let rooms = vec![
        RoomSummary {
            display_name_placeholder: None,
            display_label_placeholder: None,
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
            avatar: Some(avatar("mxc://example.invalid/dm-a")),
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
                display_name_placeholder: None,
                display_label_placeholder: None,
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
                thread_unread_count: 0,
                thread_highlight_count: 0,
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
                display_name_placeholder: None,
                display_label_placeholder: None,
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
                thread_unread_count: 0,
                thread_highlight_count: 0,
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
    // #542: recent conversation order is the product default.
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
            display_name_placeholder: None,
            display_label_placeholder: None,
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
            thread_unread_count: 0,
            thread_highlight_count: 0,
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
            display_name_placeholder: None,
            display_label_placeholder: None,
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
            thread_unread_count: 0,
            thread_highlight_count: 0,
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
            display_name_placeholder: None,
            display_label_placeholder: None,
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
            thread_unread_count: 0,
            thread_highlight_count: 0,
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
            display_name_placeholder: None,
            display_label_placeholder: None,
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
            thread_unread_count: 0,
            thread_highlight_count: 0,
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
fn active_space_people_projection_matches_sidebar_dm_scope() {
    let rooms = vec![
        active_sort_room("matching", true, &[], &["space-a"], None),
        active_sort_room("outside", true, &[], &[], None),
    ];

    let projection = compute_room_list_projection(
        RoomListFilter::People,
        RoomListSort::Activity,
        Some("space-a"),
        &[],
        &rooms,
        &HashMap::new(),
        &[],
    );
    let sidebar = compose_sidebar(Some("space-a"), &[], &rooms);

    assert_eq!(
        projection
            .items
            .iter()
            .map(|item| item.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["matching"]
    );
    assert_eq!(
        sidebar
            .global_dms
            .iter()
            .map(|item| item.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["matching"]
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
            "ordinary",
            "manual",
            "read-newest",
            "notification",
            "same-a",
            "same-b",
            "mention",
            "missing-a",
            "missing-b",
        ]
    );
}

#[test]
fn active_sort_orders_sidebar_rooms_and_dms_in_home_and_active_space() {
    let spaces = vec![SpaceSummary {
        space_id: "space-a".to_owned(),
        raw_name: None,
        display_name: "Space A".to_owned(),
        avatar: None,
        join_rule: None,
        topic: None,
        child_room_ids: vec![
            "space-read".to_owned(),
            "space-notification".to_owned(),
            "space-mention".to_owned(),
        ],
        parent_side_child_room_ids: vec![
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
    // #1589: the Space's own rooms are listed under the Space only.
    assert_eq!(
        home.space_rooms
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["home-ordinary"]
    );
    assert_eq!(
        home.global_dms
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["dm-read", "dm-outside", "dm-notification", "dm-mention"]
    );

    let active_space = compose_sidebar(Some("space-a"), &spaces, &rooms);
    assert_eq!(
        active_space
            .space_rooms
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["space-read", "space-notification", "space-mention"]
    );
    assert_eq!(
        active_space
            .global_dms
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["dm-read", "dm-notification", "dm-mention"]
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

    // Attention-first ordering is no longer the default (#542), so this
    // test selects it explicitly to keep proving the legacy recompute path.
    state.settings.values.room_list_sort = RoomListSort::Activity;
    let settings = state.settings.values.clone();
    reduce(&mut state, AppAction::SettingsLoaded { values: settings });
    assert_eq!(
        state
            .room_list
            .items
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["ordinary", "selected", "notification", "mention"]
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
        vec!["ordinary", "selected", "notification", "mention"]
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
        vec!["ordinary", "selected", "notification", "mention"]
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
        vec!["selected", "ordinary", "notification", "mention"]
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
        vec!["selected", "ordinary", "notification", "mention"]
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

/// #1166: the room rows the sidebar renders carry their room's own access
/// condition, and a room with no projected condition stays unknown.
#[test]
fn sidebar_room_rows_carry_the_projected_access_condition() {
    let mut state = ready_state();
    state.rooms = {
        let mut room_list = rooms();
        room_list.push(active_sort_room("unprojected", false, &[], &[], None));
        room_list.push(active_sort_room("mixed-unnamed", false, &[], &[], None));
        room_list
    };
    state.spaces = spaces();
    state.room_access = BTreeMap::from([
        (
            "room-a".to_owned(),
            koushi_state::RoomAccessCondition {
                join_rule: Some(koushi_state::RoomJoinRule::Public),
                restricted: None,
                allow_targets: Vec::new(),
            },
        ),
        (
            "dm-a".to_owned(),
            koushi_state::RoomAccessCondition {
                join_rule: Some(koushi_state::RoomJoinRule::KnockRestricted),
                restricted: Some(koushi_state::RestrictedConditions::ConfirmedEmpty),
                // One resolvable route and one this projection cannot name.
                allow_targets: vec![
                    koushi_state::RoomAllowTarget {
                        kind: koushi_state::RoomAllowTargetKind::Room,
                        room_id: "room-a".to_owned(),
                    },
                    koushi_state::RoomAllowTarget {
                        kind: koushi_state::RoomAllowTargetKind::Unknown,
                        room_id: "!invisible:example.invalid".to_owned(),
                    },
                ],
            },
        ),
        (
            "global-room".to_owned(),
            koushi_state::RoomAccessCondition {
                join_rule: Some(koushi_state::RoomJoinRule::Restricted),
                restricted: Some(koushi_state::RestrictedConditions::MembershipOnly),
                // Exactly one distinct target, verified a Space in the local
                // room's create event, with a name this projection carries.
                allow_targets: vec![koushi_state::RoomAllowTarget {
                    kind: koushi_state::RoomAllowTargetKind::Space,
                    room_id: "space-a".to_owned(),
                }],
            },
        ),
        (
            "mixed-unnamed".to_owned(),
            koushi_state::RoomAccessCondition {
                join_rule: Some(koushi_state::RoomJoinRule::Restricted),
                restricted: Some(koushi_state::RestrictedConditions::MembershipOnly),
                // One named Space beside one target this projection cannot name:
                // both count, so a regression that filtered unnamed targets
                // before deciding cardinality would wrongly claim the route.
                allow_targets: vec![
                    koushi_state::RoomAllowTarget {
                        kind: koushi_state::RoomAllowTargetKind::Space,
                        room_id: "space-a".to_owned(),
                    },
                    koushi_state::RoomAllowTarget {
                        kind: koushi_state::RoomAllowTargetKind::Unknown,
                        room_id: "!invisible:example.invalid".to_owned(),
                    },
                ],
            },
        ),
        (
            "space-a".to_owned(),
            koushi_state::RoomAccessCondition {
                join_rule: Some(koushi_state::RoomJoinRule::Restricted),
                restricted: Some(koushi_state::RestrictedConditions::MembershipPlusUnsupported),
                // A membership route beside an unmodelled entry is never the
                // verified single-Space route.
                allow_targets: vec![
                    koushi_state::RoomAllowTarget {
                        kind: koushi_state::RoomAllowTargetKind::Space,
                        room_id: "space-a".to_owned(),
                    },
                    koushi_state::RoomAllowTarget {
                        kind: koushi_state::RoomAllowTargetKind::Unknown,
                        room_id: "!unsupported:example.invalid".to_owned(),
                    },
                ],
            },
        ),
    ]);

    let sidebar = koushi_state::compose_sidebar_for_state(&state);
    // #1589: room-a is Space A's child, so its row is projected under the Space.
    let mut space_scoped = state.clone();
    space_scoped.navigation.active_space_id = Some("space-a".to_owned());
    let space_scoped = koushi_state::compose_sidebar_for_state(&space_scoped);

    let public_room = space_scoped
        .space_rooms
        .iter()
        .find(|room| room.room_id == "room-a")
        .expect("room-a should be in the Space's room list");
    assert_eq!(
        public_room.access_join_rule,
        Some(koushi_state::RoomJoinRule::Public)
    );
    let direct_message = sidebar
        .global_dms
        .iter()
        .find(|room| room.room_id == "dm-a")
        .expect("dm-a should be in the DM list");
    assert_eq!(
        direct_message.access_join_rule,
        Some(koushi_state::RoomJoinRule::KnockRestricted)
    );
    assert_eq!(
        direct_message.access_restricted_conditions,
        Some(koushi_state::RestrictedConditions::ConfirmedEmpty),
        "the restricted-rule facts travel to the row"
    );
    assert_eq!(
        public_room.access_restricted_conditions, None,
        "a non-restricted rule carries no allow-condition claim"
    );
    assert_eq!(
        direct_message.access_allowed_room_names,
        vec!["Room A".to_owned()],
        "a named route becomes a label this projection already carries, and an          entry it cannot name is omitted rather than exposed by id"
    );
    assert!(public_room.access_allowed_room_names.is_empty());
    assert_eq!(
        direct_message.access_space_members_route, None,
        "a rule that is not membership-only is never the verified Space route"
    );
    let space_route_room = sidebar
        .space_rooms
        .iter()
        .find(|room| room.room_id == "global-room")
        .expect("global-room should be in the room list");
    assert_eq!(
        space_route_room.access_space_members_route.as_deref(),
        Some("Space A"),
        "the single verified Space route names the Space for the specific sentence"
    );
    let mixed_unnamed = sidebar
        .space_rooms
        .iter()
        .find(|room| room.room_id == "mixed-unnamed")
        .expect("mixed-unnamed should be in the room list");
    assert_eq!(
        mixed_unnamed.access_space_members_route, None,
        "an unnamed target still counts, so a named-plus-unnamed pair is not the verified single-Space route"
    );
    let space_row = sidebar
        .space_rail
        .iter()
        .find(|space| space.space_id == "space-a")
        .expect("space-a should be on the rail");
    assert_eq!(
        space_row.access_space_members_route, None,
        "a membership-plus-unsupported rule keeps the generic sentence"
    );
    // A room with no projected condition is unknown, never a guessed rule.
    let unknown = sidebar
        .space_rooms
        .iter()
        .find(|room| room.room_id == "unprojected")
        .expect("unprojected should be in the room list");
    assert_eq!(unknown.access_join_rule, None);
}

/// #1364: a room whose newest event is not a conversation event has no
/// conversation timestamp, and the room-list Event Cache window can be one
/// event deep. The server recency stamp must keep that group in true order
/// instead of collapsing it into case-folded label order.
#[test]
fn recent_order_uses_the_server_recency_stamp_when_activity_is_absent() {
    let mut state = ready_state();
    let mut newer = dm_room_for_activity("dm-newer", "Member 2", None);
    newer.recency_stamp = Some(200);
    let mut older = dm_room_for_activity("dm-older", "Member 1", None);
    older.recency_stamp = Some(100);
    state.rooms = vec![newer, older];
    use_recent_first_for_home_dms(&mut state);

    let sidebar = compose_sidebar_for_state(&state);

    assert_eq!(
        sidebar
            .sections
            .people
            .iter()
            .map(|item| item.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["dm-newer", "dm-older"]
    );
}

/// A recency stamp that only moved because of a join or a state event must not
/// lift an activity-less room above a room with a real conversation.
#[test]
fn recent_order_keeps_a_real_conversation_above_a_recency_stamp_bump() {
    let mut state = ready_state();
    let mut joined = dm_room_for_activity("dm-joined", "Member 9", None);
    joined.recency_stamp = Some(900);
    let mut talked = dm_room_for_activity(
        "dm-talked",
        "Member 1",
        Some(ConversationActivity {
            timestamp_ms: 100,
            source: ConversationActivitySource::Message,
        }),
    );
    talked.recency_stamp = Some(100);
    state.rooms = vec![joined, talked];
    use_recent_first_for_home_dms(&mut state);

    let sidebar = compose_sidebar_for_state(&state);

    assert_eq!(
        sidebar
            .sections
            .people
            .iter()
            .map(|item| item.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["dm-talked", "dm-joined"]
    );
}

/// The activity-less fallback is shared by the attention sort, so an
/// attention-first scope keeps a stable order for read rooms too.
#[test]
fn activity_sort_uses_the_server_recency_stamp_between_read_rooms() {
    let mut state = ready_state();
    let mut newer = dm_room_for_activity("dm-newer", "Member 2", None);
    newer.recency_stamp = Some(200);
    let mut older = dm_room_for_activity("dm-older", "Member 1", None);
    older.recency_stamp = Some(100);
    state.rooms = vec![newer, older];

    let sidebar = compose_sidebar_for_state(&state);

    assert_eq!(
        sidebar
            .sections
            .people
            .iter()
            .map(|item| item.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["dm-newer", "dm-older"]
    );
}

fn use_recent_first_for_home_dms(state: &mut AppState) {
    state.settings.values.sidebar.scope_preferences.insert(
        "__home__".to_owned(),
        SidebarScopeSettings {
            dms: SidebarSectionSettings {
                collapsed: false,
                sort: RoomListSort::RecentFirst,
                ..Default::default()
            },
            ..Default::default()
        },
    );
}

// ── #542: recent conversation order is the product default ──────────────────

fn update_room_list(state: &mut AppState, rooms: Vec<RoomSummary>) {
    reduce(
        state,
        AppAction::RoomListUpdated {
            spaces: Vec::new(),
            rooms,
        },
    );
}

fn item_order(state: &AppState) -> Vec<&str> {
    state
        .room_list
        .items
        .iter()
        .map(|item| item.room_id.as_str())
        .collect()
}

fn section_order(items: &[koushi_state::RoomListItem]) -> Vec<&str> {
    items.iter().map(|item| item.room_id.as_str()).collect()
}

/// With no scope preference written, every section already opens in recent
/// conversation order: unread and mention state stays visible but does not move
/// a row.
#[test]
fn default_sidebar_order_is_recent_conversation_order_in_home_and_every_space() {
    let mut state = ready_state();
    let child_room_ids = vec![
        "room-newer".to_owned(),
        "room-unread".to_owned(),
        "room-mention".to_owned(),
        "dm-newer".to_owned(),
        "dm-unread".to_owned(),
    ];
    state.spaces = vec![SpaceSummary {
        space_id: "space-a".to_owned(),
        raw_name: None,
        display_name: "Space A".to_owned(),
        avatar: None,
        join_rule: None,
        topic: None,
        child_room_ids: child_room_ids.clone(),
        parent_side_child_room_ids: child_room_ids,
    }];
    let mut room_unread = active_sort_room("room-unread", false, &["space-a"], &[], Some(500));
    room_unread.unread_count = 4;
    room_unread.notification_count = 4;
    let mut room_mention = active_sort_room("room-mention", false, &["space-a"], &[], Some(100));
    room_mention.highlight_count = 1;
    room_mention.notification_count = 1;
    let mut dm_unread = active_sort_room("dm-unread", true, &[], &["space-a"], Some(400));
    dm_unread.unread_count = 2;
    state.rooms = vec![
        active_sort_room("room-newer", false, &["space-a"], &[], Some(900)),
        room_unread,
        room_mention,
        active_sort_room("dm-newer", true, &[], &["space-a"], Some(950)),
        dm_unread,
    ];

    let home = compose_sidebar_for_state(&state);
    // #1589: every non-DM room here belongs to the Space, so Home lists none.
    assert!(home.sections.rooms.is_empty());
    assert_eq!(
        section_order(&home.sections.people),
        vec!["dm-newer", "dm-unread"]
    );

    state.navigation.active_space_id = Some("space-a".to_owned());
    let space = compose_sidebar_for_state(&state);
    assert_eq!(
        section_order(&space.sections.rooms),
        vec!["room-newer", "room-unread", "room-mention"]
    );
    assert_eq!(
        section_order(&space.sections.people),
        vec!["dm-newer", "dm-unread"]
    );
    assert_eq!(space.sections.rooms[1].unread_count, 4);
    assert_eq!(space.sections.rooms[2].highlight_count, 1);
}

/// Sliding Sync can restore an unread counter long after the message. That must
/// not move an old room upward.
#[test]
fn restoring_stale_unread_counters_does_not_move_an_old_room() {
    let mut state = ready_state();
    let newer = active_sort_room("room-new", false, &[], &[], Some(900));
    let mut older = active_sort_room("room-old", false, &[], &[], Some(100));
    update_room_list(&mut state, vec![newer.clone(), older.clone()]);
    assert_eq!(item_order(&state), vec!["room-new", "room-old"]);

    older.unread_count = 5;
    older.notification_count = 5;
    older.highlight_count = 2;
    older.marked_unread = true;
    update_room_list(&mut state, vec![newer, older]);

    assert_eq!(item_order(&state), vec!["room-new", "room-old"]);
}

/// #438's attention-first order stays reachable for a settings document an
/// older build persisted, even though the selector no longer offers it.
#[test]
fn a_persisted_activity_sort_keeps_attention_order() {
    let sort: RoomListSort =
        serde_json::from_value(json!({ "kind": "activity" })).expect("legacy activity sort");
    assert_eq!(sort, RoomListSort::Activity);

    let mut state = ready_state();
    state.settings.values.room_list_sort = sort;
    let newer = active_sort_room("room-new", false, &[], &[], Some(900));
    let mut older = active_sort_room("room-old", false, &[], &[], Some(100));
    older.unread_count = 5;
    update_room_list(&mut state, vec![newer, older]);

    assert_eq!(item_order(&state), vec!["room-old", "room-new"]);
}

/// A live reorder must not drop the selection, and the filter must keep the
/// projected order instead of re-sorting.
#[test]
fn a_reordered_snapshot_keeps_the_selected_room_and_preserves_order_through_filters() {
    let mut state = ready_state();
    state.navigation.active_room_id = Some("room-old".to_owned());
    let newer = active_sort_room("room-new", false, &[], &[], Some(900));
    let mut older = active_sort_room("room-old", false, &[], &[], Some(100));
    older.unread_count = 1;
    update_room_list(&mut state, vec![newer.clone(), older.clone()]);
    assert_eq!(item_order(&state), vec!["room-new", "room-old"]);

    reduce(
        &mut state,
        AppAction::RoomListFilterSelected {
            filter: RoomListFilter::Unread,
        },
    );
    assert_eq!(item_order(&state), vec!["room-old"]);
    reduce(
        &mut state,
        AppAction::RoomListFilterSelected {
            filter: RoomListFilter::Rooms,
        },
    );

    // A newer message in the other room reorders the list.
    let mut newer = newer;
    newer.conversation_activity = Some(ConversationActivity {
        timestamp_ms: 2_000,
        source: ConversationActivitySource::Message,
    });
    newer.recency_stamp = Some(2_000);
    update_room_list(&mut state, vec![newer, older]);

    assert_eq!(
        state.navigation.active_room_id.as_deref(),
        Some("room-old"),
        "a live reorder must not move the selection"
    );
    assert_eq!(item_order(&state), vec!["room-new", "room-old"]);
}
