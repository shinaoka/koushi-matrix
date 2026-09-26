//! Rust-owned projection for adding existing joined rooms to the selected
//! Space (#1007).
//!
//! Eligibility is decided from the Space's parent-side `m.space.child`
//! relationships (`SpaceSummary::child_room_ids`), never from a child's
//! `m.space.parent`: a room that only claims the Space as its parent is shown
//! inside the Space by the room list, but other clients do not list it, so it
//! remains addable. React renders these rows and may text-filter them; it
//! must not classify rooms or derive their status.

use std::{collections::HashSet, fmt};

use serde::{Deserialize, Serialize};

use crate::state::{
    AppState, AvatarImage, BasicOperationState, OperationFailureKind, SpaceChildLinkOutcome,
};

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct SpaceAddRoomsModel {
    pub space_id: String,
    pub candidates: Vec<SpaceAddRoomCandidate>,
}

impl fmt::Debug for SpaceAddRoomsModel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SpaceAddRoomsModel")
            .field("space_id", &"[redacted]")
            .field("candidates", &self.candidates)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct SpaceAddRoomCandidate {
    pub room_id: String,
    pub display_name: String,
    pub avatar: Option<AvatarImage>,
    pub status: SpaceAddRoomStatus,
}

impl fmt::Debug for SpaceAddRoomCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SpaceAddRoomCandidate")
            .field("room_id", &"[redacted]")
            .field("display_name", &"[redacted]")
            .field("has_avatar", &self.avatar.is_some())
            .field("status", &self.status)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SpaceAddRoomStatus {
    /// Not a parent-side child of the Space; the add action is offered.
    Available,
    /// This pair is the in-flight basic operation.
    Adding,
    /// A parent-side child relationship exists or the homeserver accepted it.
    Added,
    /// The latest attempt failed; the add action is offered again as retry.
    Failed { reason: OperationFailureKind },
}

/// Joined, non-DM rooms of the account with their add status for the active
/// Space, ordered by display name. `None` outside a Space.
pub fn space_add_rooms_for_state(state: &AppState) -> Option<SpaceAddRoomsModel> {
    let space_id = state.navigation.active_space_id.as_deref()?;
    let space = state
        .spaces
        .iter()
        .find(|space| space.space_id == space_id)?;
    let in_flight = match &state.basic_operation {
        BasicOperationState::LinkingSpaceChild {
            space_id: linking_space_id,
            child_room_id,
            ..
        } if linking_space_id == space_id => Some(child_room_id.as_str()),
        _ => None,
    };
    let space_ids: HashSet<&str> = state
        .spaces
        .iter()
        .map(|space| space.space_id.as_str())
        .collect();
    let child_room_ids: HashSet<&str> = space.child_room_ids.iter().map(String::as_str).collect();
    let mut candidates: Vec<SpaceAddRoomCandidate> = state
        .rooms
        .iter()
        .filter(|room| !room.is_dm && room.room_id != space_id)
        .filter(|room| !space_ids.contains(room.room_id.as_str()))
        .map(|room| {
            let is_child = child_room_ids.contains(room.room_id.as_str());
            let status = if in_flight == Some(room.room_id.as_str()) {
                SpaceAddRoomStatus::Adding
            } else {
                match state
                    .space_child_links
                    .latest(space_id, &room.room_id)
                    .map(|result| result.outcome)
                {
                    _ if is_child => SpaceAddRoomStatus::Added,
                    Some(SpaceChildLinkOutcome::Linked) => SpaceAddRoomStatus::Added,
                    Some(SpaceChildLinkOutcome::Failed { reason }) => {
                        SpaceAddRoomStatus::Failed { reason }
                    }
                    None => SpaceAddRoomStatus::Available,
                }
            };
            SpaceAddRoomCandidate {
                room_id: room.room_id.clone(),
                display_name: room.display_label.clone(),
                avatar: room.avatar.clone(),
                status,
            }
        })
        .collect();
    candidates.sort_by_cached_key(|candidate| {
        (
            candidate.display_name.to_lowercase(),
            candidate.room_id.clone(),
        )
    });
    Some(SpaceAddRoomsModel {
        space_id: space_id.to_owned(),
        candidates,
    })
}
