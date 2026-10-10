//! Leaving a Space together with its joined child rooms.
//!
//! The rooms a Space-leave may take with it are a Rust projection: the joined,
//! non-DM rooms the Space shows. The same projection admits the child room IDs
//! a `LeaveSpace` command carries, so a stale or forged ID never leaves a room
//! outside the Space.

use koushi_state::{
    AppState, RoomSummary, RoomTags, SpaceSummary, admit_space_leave_room_ids,
    compose_sidebar_for_state, space_leave_candidates_for_state,
};

const SPACE: &str = "!space:example.invalid";
const OTHER_SPACE: &str = "!other-space:example.invalid";

fn room(room_id: &str, name: &str, is_dm: bool) -> RoomSummary {
    RoomSummary {
        display_name_placeholder: None,
        display_label_placeholder: None,
        room_id: room_id.to_owned(),
        display_name: name.to_owned(),
        display_label: name.to_owned(),
        original_display_label: name.to_owned(),
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
        recency_stamp: None,
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 1,
    }
}

fn space(space_id: &str, name: &str, children: &[&str]) -> SpaceSummary {
    SpaceSummary {
        space_id: space_id.to_owned(),
        raw_name: Some(name.to_owned()),
        display_name: name.to_owned(),
        avatar: None,
        join_rule: None,
        child_room_ids: children.iter().map(|id| (*id).to_owned()).collect(),
        parent_side_child_room_ids: children.iter().map(|id| (*id).to_owned()).collect(),
    }
}

fn state() -> AppState {
    AppState {
        spaces: vec![
            space(
                SPACE,
                "Synthetic Workspace",
                &[
                    "!beta:example.invalid",
                    "!alpha:example.invalid",
                    "!shared:example.invalid",
                    "!dm:example.invalid",
                    // Advertised by the Space but not joined by the account.
                    "!not-joined:example.invalid",
                    // A subspace: spaces are not rooms in the joined room list.
                    OTHER_SPACE,
                ],
            ),
            space(OTHER_SPACE, "Other Workspace", &["!shared:example.invalid"]),
        ],
        rooms: vec![
            room("!alpha:example.invalid", "alpha", false),
            room("!beta:example.invalid", "Beta", false),
            room("!shared:example.invalid", "Shared", false),
            room("!dm:example.invalid", "Member 1", true),
            room("!outside:example.invalid", "Outside", false),
        ],
        ..AppState::default()
    }
}

#[test]
fn candidates_are_the_joined_non_dm_children_sorted_by_label() {
    let candidates = space_leave_candidates_for_state(&state(), SPACE);
    let ids: Vec<&str> = candidates.iter().map(|c| c.room_id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "!alpha:example.invalid",
            "!beta:example.invalid",
            "!shared:example.invalid",
        ]
    );
    assert_eq!(candidates[0].display_name, "alpha");
}

#[test]
fn candidates_mark_rooms_another_joined_space_also_contains() {
    let candidates = space_leave_candidates_for_state(&state(), SPACE);
    let shared: Vec<(&str, bool)> = candidates
        .iter()
        .map(|c| (c.room_id.as_str(), c.in_other_space))
        .collect();
    assert_eq!(
        shared,
        [
            ("!alpha:example.invalid", false),
            ("!beta:example.invalid", false),
            ("!shared:example.invalid", true),
        ]
    );
}

#[test]
fn unknown_space_has_no_candidates() {
    assert!(space_leave_candidates_for_state(&state(), "!missing:example.invalid").is_empty());
}

#[test]
fn space_rail_items_carry_their_leave_candidates() {
    let sidebar = compose_sidebar_for_state(&state());
    let rail = sidebar
        .space_rail
        .iter()
        .find(|item| item.space_id == SPACE)
        .expect("space rail item");
    assert_eq!(rail.leave_candidates.len(), 3);
    let other = sidebar
        .space_rail
        .iter()
        .find(|item| item.space_id == OTHER_SPACE)
        .expect("other space rail item");
    assert_eq!(other.leave_candidates.len(), 1);
    assert!(other.leave_candidates[0].in_other_space);
}

#[test]
fn admission_keeps_only_candidates_once_in_request_order() {
    let requested = [
        "!shared:example.invalid",
        "!outside:example.invalid",
        "!dm:example.invalid",
        "!not-joined:example.invalid",
        "!alpha:example.invalid",
        "!shared:example.invalid",
    ]
    .map(str::to_owned);
    assert_eq!(
        admit_space_leave_room_ids(&state(), SPACE, &requested),
        ["!shared:example.invalid", "!alpha:example.invalid"]
    );
}

#[test]
fn admission_for_an_unknown_space_keeps_nothing() {
    let requested = ["!alpha:example.invalid".to_owned()];
    assert!(
        admit_space_leave_room_ids(&state(), "!missing:example.invalid", &requested).is_empty()
    );
}
