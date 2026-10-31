//! Home's room scope (#1589): a non-DM room a joined Space lists as its child
//! belongs to that Space's section, so Home must not list it again.
//!
//! Canon: `docs/architecture/state-machine.md`, "Navigation" and "Sidebar
//! Sections And Low Priority".

use super::support::ready_state;
use koushi_state::{AppAction, RoomSummary, RoomTags, SpaceSummary, compose_sidebar, reduce};

fn space(space_id: &str, children: &[&str]) -> SpaceSummary {
    SpaceSummary {
        space_id: space_id.to_owned(),
        raw_name: None,
        display_name: "Space".to_owned(),
        avatar: None,
        join_rule: None,
        topic: None,
        child_room_ids: children.iter().map(|id| (*id).to_owned()).collect(),
        parent_side_child_room_ids: children.iter().map(|id| (*id).to_owned()).collect(),
    }
}

fn room(room_id: &str, is_dm: bool, parent_space_ids: &[&str], unread_count: u64) -> RoomSummary {
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
        unread_count,
        notification_count: unread_count,
        highlight_count: 0,
        thread_unread_count: 0,
        thread_highlight_count: 0,
        marked_unread: false,
        recency_stamp: None,
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: parent_space_ids.iter().map(|id| (*id).to_owned()).collect(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 0,
    }
}

fn room_ids(items: &[koushi_state::RoomListItem]) -> Vec<&str> {
    items.iter().map(|item| item.room_id.as_str()).collect()
}

#[test]
fn home_lists_a_space_child_only_under_its_space() {
    let spaces = vec![space("space-a", &["!child:example.invalid"])];
    let rooms = vec![
        room("!child:example.invalid", false, &["space-a"], 4),
        room("!loose:example.invalid", false, &[], 0),
        room(
            "!orphan:example.invalid",
            false,
            &["!gone:example.invalid"],
            0,
        ),
        room("!dm:example.invalid", true, &["space-a"], 0),
    ];

    let home = compose_sidebar(None, &spaces, &rooms);
    assert_eq!(
        room_ids(&home.sections.rooms),
        vec!["!loose:example.invalid", "!orphan:example.invalid"]
    );
    assert_eq!(room_ids(&home.sections.people), vec!["!dm:example.invalid"]);
    // The account-wide Home badge still covers every joined conversation; only
    // the Rooms section and its heading aggregate follow the section scope.
    assert_eq!(home.account_home.unread_count, 4);
    assert_eq!(home.space_unread_count, 0);

    let space_scope = compose_sidebar(Some("space-a"), &spaces, &rooms);
    assert_eq!(
        room_ids(&space_scope.sections.rooms),
        vec!["!child:example.invalid"]
    );
}

/// Selecting a room opens a scope that can show it: a parent Space this account
/// is not in is not a sidebar scope, so the joined one wins even when the room
/// names it later.
#[test]
fn selecting_a_room_opens_a_joined_space_that_lists_it() {
    let mut state = ready_state();
    state.spaces = vec![space("space-a", &["!child:example.invalid"])];
    state.rooms = vec![room(
        "!child:example.invalid",
        false,
        &["!unjoined:example.invalid", "space-a"],
        0,
    )];

    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "!child:example.invalid".to_owned(),
        },
    );

    assert_eq!(
        state.navigation.active_space_id.as_deref(),
        Some("space-a"),
        "a room opens under a Space whose section can list it"
    );
}

#[test]
fn selecting_a_room_in_no_joined_space_opens_home() {
    let mut state = ready_state();
    state.rooms = vec![room(
        "!loose:example.invalid",
        false,
        &["!unjoined:example.invalid"],
        0,
    )];

    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "!loose:example.invalid".to_owned(),
        },
    );

    assert_eq!(state.navigation.active_space_id, None);
}
