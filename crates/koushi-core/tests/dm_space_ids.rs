mod support;

use std::collections::{BTreeMap, BTreeSet};

use koushi_state::{RoomSummary, RoomTags};

fn dm_summary(room_id: &str, dm_user_ids: Vec<&str>) -> RoomSummary {
    RoomSummary {
        room_id: room_id.to_owned(),
        display_name: room_id.to_owned(),
        display_label: room_id.to_owned(),
        original_display_label: room_id.to_owned(),
        avatar: None,
        is_dm: true,
        dm_user_ids: dm_user_ids.iter().map(|s| s.to_string()).collect(),
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
    }
}

fn non_dm_summary(room_id: &str) -> RoomSummary {
    RoomSummary {
        room_id: room_id.to_owned(),
        display_name: room_id.to_owned(),
        display_label: room_id.to_owned(),
        original_display_label: room_id.to_owned(),
        avatar: None,
        is_dm: false,
        dm_user_ids: vec![],
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
    }
}

#[test]
fn assign_dm_space_ids_scopes_dms_to_spaces_by_counterpart_membership() {
    let mut space_members: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    space_members.insert("space-a".to_owned(), BTreeSet::from(["@alice".to_owned()]));
    space_members.insert("space-b".to_owned(), BTreeSet::from(["@bob".to_owned()]));

    let mut rooms = vec![
        dm_summary("dm-alice", vec!["@alice"]),
        dm_summary("dm-bob", vec!["@bob"]),
        dm_summary("dm-carol", vec!["@carol"]),
        dm_summary("dm-group", vec!["@alice", "@bob"]),
        non_dm_summary("room-x"),
    ];

    koushi_core::room::assign_dm_space_ids(&mut rooms, &space_members);

    let find = |id: &str| rooms.iter().find(|r| r.room_id == id).unwrap();

    assert_eq!(find("dm-alice").dm_space_ids, vec!["space-a"]);
    assert_eq!(find("dm-bob").dm_space_ids, vec!["space-b"]);
    assert_eq!(find("dm-carol").dm_space_ids, Vec::<String>::new());
    assert_eq!(find("dm-group").dm_space_ids, vec!["space-a", "space-b"]);
    assert_eq!(find("room-x").dm_space_ids, Vec::<String>::new());
}
