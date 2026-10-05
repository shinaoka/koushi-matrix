//! Rust-owned projection for leaving a Space together with its child rooms.
//!
//! The rooms a Space-leave may take with it are the joined, non-DM rooms the
//! Space shows (`SpaceSummary::child_room_ids`, the union the room list
//! renders). Subspaces and advertised-but-not-joined children are not in the
//! joined room list, so they are never candidates. React renders these rows and
//! the user's selection; the same projection admits the IDs a `LeaveSpace`
//! command carries.

use std::{
    collections::{HashMap, HashSet},
    fmt,
};

use serde::{Deserialize, Serialize};

use crate::state::{AppState, AvatarImage, RoomNamePlaceholder, RoomSummary, SpaceSummary};

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct SpaceLeaveCandidate {
    pub room_id: String,
    pub display_name: String,
    /// Mirrors `RoomSummary.display_label_placeholder` for `display_name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name_placeholder: Option<RoomNamePlaceholder>,
    pub avatar: Option<AvatarImage>,
    /// Another joined Space also shows this room, so leaving it removes it
    /// from that Space too.
    pub in_other_space: bool,
}

impl fmt::Debug for SpaceLeaveCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SpaceLeaveCandidate")
            .field("room_id", &"[redacted]")
            .field("display_name", &"[redacted]")
            .field("has_avatar", &self.avatar.is_some())
            .field("in_other_space", &self.in_other_space)
            .finish()
    }
}

/// The joined, non-DM child rooms of `space_id`, ordered by display label.
pub fn space_leave_candidates_for_state(
    state: &AppState,
    space_id: &str,
) -> Vec<SpaceLeaveCandidate> {
    let Some(space) = state.spaces.iter().find(|space| space.space_id == space_id) else {
        return Vec::new();
    };
    let rooms_by_id: HashMap<&str, &RoomSummary> = state
        .rooms
        .iter()
        .map(|room| (room.room_id.as_str(), room))
        .collect();
    space_leave_candidates(space, &state.spaces, &rooms_by_id)
}

pub(crate) fn space_leave_candidates(
    space: &SpaceSummary,
    spaces: &[SpaceSummary],
    rooms_by_id: &HashMap<&str, &RoomSummary>,
) -> Vec<SpaceLeaveCandidate> {
    let mut seen = HashSet::new();
    let mut rooms: Vec<&RoomSummary> = space
        .child_room_ids
        .iter()
        .filter(|room_id| seen.insert(room_id.as_str()))
        .filter_map(|room_id| rooms_by_id.get(room_id.as_str()).copied())
        .filter(|room| !room.is_dm)
        .collect();
    rooms.sort_by(|left, right| {
        left.display_label
            .to_lowercase()
            .cmp(&right.display_label.to_lowercase())
            .then_with(|| left.room_id.cmp(&right.room_id))
    });
    rooms
        .into_iter()
        .map(|room| SpaceLeaveCandidate {
            room_id: room.room_id.clone(),
            display_name: room.display_label.clone(),
            display_name_placeholder: room.display_label_placeholder.clone(),
            avatar: room.avatar.clone(),
            in_other_space: spaces.iter().any(|other| {
                other.space_id != space.space_id && other.child_room_ids.contains(&room.room_id)
            }),
        })
        .collect()
}

/// The requested child room IDs that are current leave candidates of
/// `space_id`, deduplicated, in request order. Anything else (a room outside
/// the Space, a DM, a room already left) is dropped rather than left.
pub fn admit_space_leave_room_ids(
    state: &AppState,
    space_id: &str,
    requested: &[String],
) -> Vec<String> {
    let candidates: HashSet<String> = space_leave_candidates_for_state(state, space_id)
        .into_iter()
        .map(|candidate| candidate.room_id)
        .collect();
    let mut admitted = HashSet::new();
    requested
        .iter()
        .filter(|room_id| candidates.contains(*room_id) && admitted.insert(room_id.as_str()))
        .cloned()
        .collect()
}
