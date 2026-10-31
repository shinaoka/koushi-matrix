use std::collections::HashMap;

use koushi_state::{
    RoomNotificationMode, RoomNotificationSettings, RoomSummary, RoomTags, SpaceSummary,
    compose_sidebar_with_account_facts,
};

fn room(id: &str, unread: u64, notifications: u64, thread_unread: u64) -> RoomSummary {
    RoomSummary {
        room_id: id.into(),
        display_name: id.into(),
        display_name_placeholder: None,
        display_label: id.into(),
        original_display_label: id.into(),
        display_label_placeholder: None,
        avatar: None,
        is_dm: false,
        dm_user_ids: vec![],
        tags: RoomTags::default(),
        unread_count: unread,
        notification_count: notifications,
        highlight_count: 0,
        thread_unread_count: thread_unread,
        thread_highlight_count: 0,
        marked_unread: false,
        recency_stamp: None,
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: vec!["!space:example.invalid".into()],
        dm_space_ids: vec![],
        is_encrypted: false,
        joined_members: 2,
    }
}

#[test]
fn space_badge_equals_the_visible_non_muted_room_counts() {
    let space = SpaceSummary {
        space_id: "!space:example.invalid".into(),
        raw_name: None,
        display_name: "Synthetic Space".into(),
        avatar: None,
        join_rule: None,
        topic: None,
        child_room_ids: vec![
            "!plain:example.invalid".into(),
            "!thread:example.invalid".into(),
        ],
        parent_side_child_room_ids: vec![],
    };
    let rooms = [
        room("!plain:example.invalid", 2, 0, 0),
        room("!thread:example.invalid", 0, 0, 3),
    ];
    let sidebar = compose_sidebar_with_account_facts(
        Some("!space:example.invalid"),
        &[space],
        &rooms,
        &HashMap::new(),
        0,
    );
    let row_sum: u64 = sidebar
        .sections
        .rooms
        .iter()
        .map(|room| room.display_count)
        .sum();
    assert_eq!(sidebar.space_rail[0].unread_count, 5);
    assert_eq!(row_sum, sidebar.space_rail[0].unread_count);
}

#[test]
fn muted_room_keeps_local_unread_but_does_not_inflate_space_badge() {
    let space = SpaceSummary {
        space_id: "!space:example.invalid".into(),
        raw_name: None,
        display_name: "Synthetic Space".into(),
        avatar: None,
        join_rule: None,
        topic: None,
        child_room_ids: vec!["!muted:example.invalid".into()],
        parent_side_child_room_ids: vec![],
    };
    let settings = HashMap::from([(
        "!muted:example.invalid".into(),
        RoomNotificationSettings {
            mode: RoomNotificationMode::Mute,
            ..Default::default()
        },
    )]);
    let sidebar = compose_sidebar_with_account_facts(
        Some("!space:example.invalid"),
        &[space],
        &[room("!muted:example.invalid", 2, 5, 1)],
        &settings,
        0,
    );
    assert_eq!(sidebar.sections.rooms[0].display_count, 3);
    assert_eq!(sidebar.space_rail[0].unread_count, 0);
}
