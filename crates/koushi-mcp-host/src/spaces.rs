//! Read-only Space projections for MCP (#1883).
//!
//! An assistant cannot tell two rooms with the same display name apart, and
//! `list_rooms` alone never says which Space a room belongs to. These
//! projections answer "which Space" from the snapshot the app already holds,
//! and never invent what the snapshot does not know: an unloaded child list
//! reports `suggested: null`, an unknown display name is `null`, and the alias
//! field is always `null` because Koushi's state keeps no canonical Space
//! alias (an alias is never promoted into a name).

use std::collections::VecDeque;

use koushi_state::{RoomSummary, SpaceChildrenState, SpaceSummary};
use serde_json::{Value, json};

/// The largest `max_depth` this build expands.
pub(crate) const MAX_DEPTH: usize = 3;

/// Entries one reply carries, so no listing can outgrow the bridge's frame.
/// A Space that hits it reports `truncated`, and narrowing `max_depth` or
/// asking `list_rooms` for one Space reads the rest.
pub(crate) const MAX_ENTRIES: usize = 500;

/// Spaces that hold `child_id`: one that displays it, or one whose own
/// parent-side `m.space.child` list names it. The second is normally a subset
/// of the first (`SpaceSummary::child_room_ids` is their union); it is read
/// anyway because #1883 names both fields, so a Space is found even if the two
/// ever disagree.
fn parent_space_ids(spaces: &[SpaceSummary], child_id: &str) -> Vec<String> {
    spaces
        .iter()
        .filter(|space| {
            space.space_id != child_id
                && (space.child_room_ids.iter().any(|id| id == child_id)
                    || space
                        .parent_side_child_room_ids
                        .iter()
                        .any(|id| id == child_id))
        })
        .map(|space| space.space_id.clone())
        .collect()
}

/// `list_spaces`: every Space in the snapshot, with the Spaces above it.
pub(crate) fn list_spaces(spaces: &[SpaceSummary]) -> Value {
    let truncated = spaces.len() > MAX_ENTRIES;
    let entries: Vec<Value> = spaces
        .iter()
        .take(MAX_ENTRIES)
        .map(|space| {
            let parents = parent_space_ids(spaces, &space.space_id);
            json!({
                "space_id": space.space_id,
                "name": space.display_name,
                "raw_name": space.raw_name,
                "alias": Value::Null,
                "child_count": space.child_room_ids.len(),
                "is_subspace": !parents.is_empty(),
                "parent_space_ids": parents,
                // Every Space in the snapshot is one this account is in.
                "joined": true,
            })
        })
        .collect();
    json!({ "spaces": entries, "truncated": truncated })
}

/// `list_space_rooms`: one Space's children, expanded `max_depth` levels.
///
/// `None` means `space_id` is not a Space in this snapshot, so the caller
/// reports a `room_not_found` error instead of an empty list.
pub(crate) fn space_rooms(
    spaces: &[SpaceSummary],
    rooms: &[RoomSummary],
    children: &SpaceChildrenState,
    space_id: &str,
    max_depth: usize,
) -> Option<Value> {
    let space = spaces.iter().find(|space| space.space_id == space_id)?;
    let max_depth = max_depth.clamp(1, MAX_DEPTH);

    let mut entries: Vec<Value> = Vec::new();
    let mut truncated = false;
    // Breadth-first in room-id order: `child_room_ids` is a sorted set, not the
    // sidebar's order. `seen` starts with the requested Space:
    // a cycle back to it would re-list the Space the reply already names, and
    // seeding it is what terminates a Space that lists itself.
    let mut seen = vec![space.space_id.clone()];
    let mut queue = VecDeque::from([(space.space_id.clone(), 0usize)]);
    while let Some((parent_id, depth)) = queue.pop_front() {
        if depth == max_depth {
            continue;
        }
        let Some(parent) = spaces.iter().find(|space| space.space_id == parent_id) else {
            continue;
        };
        // Koushi loads one Space's child list at a time, and `children_for`
        // returns nothing for any other Space: what it holds for this parent
        // is the one source for a child's `suggested` flag and for the type
        // and name of a child this account has not joined.
        let loaded = children.children_for(&parent_id);
        for child_id in &parent.child_room_ids {
            if seen.contains(child_id) {
                continue;
            }
            if entries.len() == MAX_ENTRIES {
                truncated = true;
                break;
            }
            seen.push(child_id.clone());
            let child_space = spaces.iter().find(|space| &space.space_id == child_id);
            let child_room = rooms.iter().find(|room| &room.room_id == child_id);
            let loaded_child = loaded.iter().find(|child| child.room_id == *child_id);
            let is_space =
                child_space.is_some() || loaded_child.is_some_and(|child| child.is_space);
            let name = match (child_space, child_room) {
                (Some(child_space), _) => Some(child_space.display_name.clone()),
                (None, Some(child_room)) => Some(child_room.display_label.clone()),
                // Only the Space's own child list names this one. Its
                // calculated empty-room placeholder is never shown as a name.
                (None, None) => loaded_child
                    .filter(|child| child.display_name_placeholder.is_none())
                    .map(|child| child.display_name.clone()),
            };
            entries.push(json!({
                "room_id": child_id,
                "name": name,
                "is_space": is_space,
                "joined": child_space.is_some() || child_room.is_some(),
                "suggested": loaded_child.map(|child| child.suggested),
                "parent_space_ids": parent_space_ids(spaces, child_id),
                "depth": depth + 1,
            }));
            if child_space.is_some() {
                queue.push_back((child_id.clone(), depth + 1));
            }
        }
        if truncated {
            break;
        }
    }

    Some(json!({
        "space_id": space.space_id,
        "children": entries,
        "truncated": truncated,
    }))
}

#[cfg(test)]
#[path = "spaces_tests.rs"]
mod tests;
