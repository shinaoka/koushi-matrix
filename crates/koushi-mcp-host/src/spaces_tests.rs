//! Fixture-level tests for the Space projections (#1883): the two rooms named
//! `all`, the sub-Spaces, the unknown child, the unloaded `suggested` flag and
//! the entry cap.

use koushi_state::{RoomSummary, SpaceChildMembership, SpaceChildSummary, SpaceChildrenState};

use super::*;

fn space(space_id: &str, display_name: &str, children: &[&str]) -> SpaceSummary {
    SpaceSummary {
        space_id: space_id.to_owned(),
        raw_name: Some(display_name.to_owned()),
        display_name: display_name.to_owned(),
        avatar: None,
        join_rule: None,
        topic: None,
        child_room_ids: children.iter().map(|id| (*id).to_owned()).collect(),
        parent_side_child_room_ids: Vec::new(),
    }
}

fn room(room_id: &str, display_label: &str, parent_space_ids: &[&str]) -> RoomSummary {
    serde_json::from_value(json!({
        "room_id": room_id,
        "display_name": display_label,
        "display_label": display_label,
        "is_dm": false,
        "unread_count": 0,
        "notification_count": 0,
        "highlight_count": 0,
        "parent_space_ids": parent_space_ids,
    }))
    .expect("room summary")
}

fn loaded_child(room_id: &str, suggested: bool) -> SpaceChildSummary {
    SpaceChildSummary {
        room_id: room_id.to_owned(),
        display_name: format!("loaded {room_id}"),
        display_name_placeholder: None,
        avatar: None,
        topic: None,
        membership: SpaceChildMembership::NotJoined,
        can_join: true,
        is_space: false,
        joined_members: 0,
        suggested,
    }
}

fn loaded_children(space_id: &str, children: Vec<SpaceChildSummary>) -> SpaceChildrenState {
    SpaceChildrenState {
        selected_space_id: Some(space_id.to_owned()),
        children,
        ..SpaceChildrenState::default()
    }
}

/// Two Spaces holding a room displayed as `all`, a chain of sub-Spaces four
/// levels deep, and a child `!far` advertised by `!a` that this account has not
/// joined.
fn fixture() -> (Vec<SpaceSummary>, Vec<RoomSummary>) {
    let spaces = vec![
        space("!a", "Space A", &["!all-a", "!sub", "!far"]),
        space("!b", "Space B", &["!all-b"]),
        space("!sub", "Space A / sub", &["!deep", "!sub2"]),
        space("!sub2", "Space A / sub2", &["!deeper", "!sub3"]),
        space("!sub3", "Space A / sub3", &["!deepest"]),
    ];
    let rooms = vec![
        room("!all-a", "all", &["!a"]),
        room("!all-b", "all", &["!b"]),
        room("!deep", "deep", &["!sub"]),
        room("!deeper", "deeper", &["!sub2"]),
        room("!deepest", "deepest", &["!sub3"]),
    ];
    (spaces, rooms)
}

fn entries(value: &Value) -> &Vec<Value> {
    value["children"].as_array().expect("children")
}

fn names(value: &Value) -> Vec<Option<&str>> {
    entries(value)
        .iter()
        .map(|entry| entry["name"].as_str())
        .collect()
}

fn entry<'a>(value: &'a Value, room_id: &str) -> &'a Value {
    entries(value)
        .iter()
        .find(|entry| entry["room_id"] == room_id)
        .unwrap_or_else(|| panic!("{room_id} in {value}"))
}

#[test]
fn a_repeated_room_name_resolves_to_the_one_in_that_space() {
    let (spaces, rooms) = fixture();
    let listing = list_spaces(&spaces);
    let listed = listing["spaces"].as_array().expect("spaces");
    assert_eq!(listed.len(), 5);
    assert_eq!(listing["truncated"], false);
    let a = listed
        .iter()
        .find(|space| space["space_id"] == "!a")
        .expect("Space A");
    assert_eq!(a["name"], "Space A");
    assert_eq!(a["raw_name"], "Space A");
    assert_eq!(a["child_count"], 3);
    assert_eq!(a["joined"], true);
    assert_eq!(a["is_subspace"], false);
    assert_eq!(a["parent_space_ids"], json!([]));
    assert_eq!(a["alias"], Value::Null, "no Space alias is invented");

    let sub = listed
        .iter()
        .find(|space| space["space_id"] == "!sub2")
        .expect("sub-Space");
    assert_eq!(sub["is_subspace"], true);
    assert_eq!(sub["parent_space_ids"], json!(["!sub"]));

    let a_children =
        space_rooms(&spaces, &rooms, &SpaceChildrenState::default(), "!a", 1).expect("Space A");
    let shared: Vec<&Value> = entries(&a_children)
        .iter()
        .filter(|entry| entry["name"] == "all")
        .collect();
    assert_eq!(shared.len(), 1, "{a_children}");
    assert_eq!(shared[0]["room_id"], "!all-a");

    let b_children =
        space_rooms(&spaces, &rooms, &SpaceChildrenState::default(), "!b", 1).expect("Space B");
    assert_eq!(
        entries(&b_children)
            .iter()
            .filter(|entry| entry["name"] == "all")
            .map(|entry| entry["room_id"].clone())
            .collect::<Vec<Value>>(),
        vec![json!("!all-b")]
    );
}

#[test]
fn an_unjoined_child_has_no_name_and_an_unknown_space_is_none() {
    let (spaces, rooms) = fixture();
    let a_children =
        space_rooms(&spaces, &rooms, &SpaceChildrenState::default(), "!a", 1).expect("Space A");
    let far = entry(&a_children, "!far");
    assert_eq!(far["name"], Value::Null, "no name is guessed: {far}");
    assert_eq!(far["joined"], false);
    assert_eq!(far["is_space"], false);
    assert_eq!(far["parent_space_ids"], json!(["!a"]));

    assert!(
        space_rooms(&spaces, &rooms, &SpaceChildrenState::default(), "!nope", 1).is_none(),
        "an id that is not a Space must be an error at the handler, not an empty list"
    );
}

#[test]
fn a_sub_space_expands_one_level_at_a_time_up_to_the_cap() {
    let (spaces, rooms) = fixture();
    let level = |max_depth: usize| {
        space_rooms(
            &spaces,
            &rooms,
            &SpaceChildrenState::default(),
            "!a",
            max_depth,
        )
        .expect("Space A")
    };

    let shallow = level(1);
    assert_eq!(
        names(&shallow),
        vec![Some("all"), Some("Space A / sub"), None]
    );
    assert!(
        !entries(&shallow)
            .iter()
            .any(|entry| entry["room_id"] == "!deep")
    );

    let second = level(2);
    assert_eq!(entry(&second, "!deep")["depth"], 2);
    assert_eq!(entry(&second, "!deep")["name"], "deep");
    assert_eq!(entry(&second, "!deep")["parent_space_ids"], json!(["!sub"]));
    assert_eq!(entry(&second, "!sub2")["depth"], 2);
    assert!(
        !entries(&second)
            .iter()
            .any(|entry| entry["room_id"] == "!deeper")
    );

    let third = level(MAX_DEPTH);
    assert_eq!(entry(&third, "!deeper")["depth"], 3);
    assert_eq!(entry(&third, "!deeper")["name"], "deeper");
    assert_eq!(
        entry(&third, "!deeper")["parent_space_ids"],
        json!(["!sub2"])
    );
    assert!(
        !entries(&third)
            .iter()
            .any(|entry| entry["room_id"] == "!deepest"),
        "depth 4 is past the cap: {third}"
    );

    assert_eq!(
        level(99),
        third,
        "max_depth above the cap reads the same as the cap"
    );
}

#[test]
fn a_cycle_between_spaces_lists_each_child_once() {
    let mut spaces = vec![
        space("!a", "Space A", &["!b"]),
        space("!b", "Space B", &["!a", "!r"]),
    ];
    // A Space that lists itself must not appear as its own child either.
    spaces[0].child_room_ids.push("!a".to_owned());
    let rooms = vec![room("!r", "r", &["!b"])];

    let listing = space_rooms(
        &spaces,
        &rooms,
        &SpaceChildrenState::default(),
        "!a",
        MAX_DEPTH,
    )
    .expect("Space A");
    assert_eq!(names(&listing), vec![Some("Space B"), Some("r")]);
    assert_eq!(
        entries(&listing)
            .iter()
            .map(|entry| entry["depth"].clone())
            .collect::<Vec<Value>>(),
        vec![json!(1), json!(2)]
    );
}

#[test]
fn suggested_is_reported_only_for_the_space_whose_children_are_loaded() {
    let (spaces, rooms) = fixture();
    let loaded = loaded_children(
        "!a",
        vec![loaded_child("!all-a", true), loaded_child("!sub", false)],
    );
    let a_children = space_rooms(&spaces, &rooms, &loaded, "!a", 1).expect("Space A");
    assert_eq!(entry(&a_children, "!all-a")["suggested"], json!(true));
    assert_eq!(entry(&a_children, "!sub")["suggested"], json!(false));
    assert_eq!(
        entry(&a_children, "!far")["suggested"],
        Value::Null,
        "a child the loaded list omits is unknown, not unsuggested"
    );

    // The loaded list belongs to another Space, so this Space's flags are
    // unknown, and the sub-Space's own children are never flagged from it.
    let b_children = space_rooms(&spaces, &rooms, &loaded, "!b", 1).expect("Space B");
    assert_eq!(entry(&b_children, "!all-b")["suggested"], Value::Null);
    let deep = space_rooms(&spaces, &rooms, &loaded, "!a", 2).expect("Space A");
    assert_eq!(entry(&deep, "!deep")["suggested"], Value::Null);
}

#[test]
fn the_loaded_child_list_names_and_types_an_unjoined_child() {
    let (spaces, rooms) = fixture();
    let mut sub = loaded_child("!far", false);
    sub.is_space = true;
    let loaded = loaded_children("!a", vec![sub]);
    let a_children = space_rooms(&spaces, &rooms, &loaded, "!a", 1).expect("Space A");
    let far = entry(&a_children, "!far");
    assert_eq!(
        far["is_space"], true,
        "only the loaded list knows this child is a Space"
    );
    assert_eq!(far["name"], "loaded !far");
    assert_eq!(far["joined"], false, "joined still comes from the snapshot");

    // A calculated empty-room name is never promoted into a name.
    let mut placeholder = loaded_child("!far", false);
    placeholder.display_name_placeholder = Some(koushi_state::RoomNamePlaceholder::Empty);
    let loaded = loaded_children("!a", vec![placeholder]);
    let a_children = space_rooms(&spaces, &rooms, &loaded, "!a", 1).expect("Space A");
    assert_eq!(entry(&a_children, "!far")["name"], Value::Null);
}

#[test]
fn a_listing_that_hits_the_entry_cap_says_so() {
    let children: Vec<String> = (0..MAX_ENTRIES + 1)
        .map(|index| format!("!r{index}"))
        .collect();
    let child_refs: Vec<&str> = children.iter().map(String::as_str).collect();
    let spaces = vec![space("!a", "Space A", &child_refs)];
    let rooms: Vec<RoomSummary> = children
        .iter()
        .map(|room_id| room(room_id, "r", &["!a"]))
        .collect();

    let listing =
        space_rooms(&spaces, &rooms, &SpaceChildrenState::default(), "!a", 1).expect("Space A");
    assert_eq!(entries(&listing).len(), MAX_ENTRIES);
    assert_eq!(listing["truncated"], true);

    let many_spaces: Vec<SpaceSummary> = (0..MAX_ENTRIES + 1)
        .map(|index| space(&format!("!s{index}"), "S", &[]))
        .collect();
    let spaces_listing = list_spaces(&many_spaces);
    assert_eq!(
        spaces_listing["spaces"].as_array().map(Vec::len),
        Some(MAX_ENTRIES)
    );
    assert_eq!(spaces_listing["truncated"], true);
}

#[test]
fn a_parent_side_only_child_still_names_its_parent() {
    let mut parent = space("!a", "Space A", &[]);
    // `parent_side_child_room_ids` is the Space's own `m.space.child` list, a
    // subset of the display union. It normally cannot hold a child the union
    // does not, but #1883 names both fields, so both are read.
    parent.parent_side_child_room_ids = vec!["!child".to_owned()];
    let spaces = vec![parent, space("!b", "Space B", &["!child"])];
    let rooms = vec![room("!child", "child", &["!a", "!b"])];

    assert_eq!(parent_space_ids(&spaces, "!child"), vec!["!a", "!b"]);
    let a = space_rooms(&spaces, &rooms, &SpaceChildrenState::default(), "!a", 1).expect("A");
    assert!(
        entries(&a).is_empty(),
        "list_space_rooms follows the display union: {a}"
    );
    let b = space_rooms(&spaces, &rooms, &SpaceChildrenState::default(), "!b", 1).expect("B");
    assert_eq!(entry(&b, "!child")["parent_space_ids"], json!(["!a", "!b"]));
}
