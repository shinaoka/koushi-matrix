//! Issue #961: every child room a Space advertises, with the viewer's
//! membership beside it.
//!
//! The room list only ever carried rooms the account has joined, so a Space's
//! own room list could not show what else is in the Space. This slice holds the
//! Space's advertised children — the `/hierarchy` projection — so the sidebar
//! and the Space info panel can show a joined room, an invitation and a room
//! the account has not joined in one list, without the frontend inferring any
//! of it.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::{AvatarImage, errors::OperationFailureKind};

/// The viewer's relationship to a Space child room.
///
/// `Unknown` is the honest answer for a child the server did not describe and
/// that is not present in the local room cache. Invited children that the sync
/// client knows are projected with their local membership instead.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpaceChildMembership {
    Joined,
    Invited,
    Knocked,
    Left,
    Banned,
    NotJoined,
    #[default]
    Unknown,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct SpaceChildSummary {
    pub room_id: String,
    /// Display name as the server computed it, or the room ID when the server
    /// described nothing.
    pub display_name: String,
    /// Set when `display_name` is the SDK's English calculated empty-room
    /// name (#1070); the GUI renders catalog text for it instead.
    #[serde(default)]
    pub display_name_placeholder: Option<super::room::RoomNamePlaceholder>,
    #[serde(default)]
    pub avatar: Option<AvatarImage>,
    /// Issue #1661: the child's description (`m.room.topic`) as `/hierarchy`
    /// or the local cache reports it; `None` when it has none.
    #[serde(default)]
    pub topic: Option<String>,
    pub membership: SpaceChildMembership,
    /// Whether the join rule the server reported permits an attempt to join.
    /// A room the account is invited to counts: accepting is a join.
    #[serde(default)]
    pub can_join: bool,
    #[serde(default)]
    pub is_space: bool,
    #[serde(default)]
    pub joined_members: u64,
    /// Issue #1410: the Space's `m.space.child` marks this child suggested.
    #[serde(default)]
    pub suggested: bool,
}

impl fmt::Debug for SpaceChildSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SpaceChildSummary")
            .field("room_id", &"RoomId(..)")
            .field("display_name", &"RoomName(..)")
            .field("display_name_placeholder", &self.display_name_placeholder)
            .field("avatar", &self.avatar.as_ref().map(|_| "AvatarImage(..)"))
            .field("topic", &self.topic.as_ref().map(|_| "RoomTopic(..)"))
            .field("membership", &self.membership)
            .field("can_join", &self.can_join)
            .field("is_space", &self.is_space)
            .field("joined_members", &self.joined_members)
            .field("suggested", &self.suggested)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SpaceChildrenLoadState {
    #[default]
    Idle,
    Loading,
    Failed {
        failure: OperationFailureKind,
    },
}

/// The advertised children of the currently selected Space.
///
/// Scoped to one Space at a time, like the Space member projection: selecting
/// another Space bumps the generation and clears what the previous one loaded,
/// so a late response can never paint the wrong Space's rooms.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SpaceChildrenState {
    #[serde(default)]
    pub selected_space_id: Option<String>,
    #[serde(default)]
    pub generation: u64,
    #[serde(default)]
    pub children: Vec<SpaceChildSummary>,
    #[serde(default)]
    pub load: SpaceChildrenLoadState,
    /// Issue #1410: whether this account may send `m.space.child` in the
    /// selected Space, loaded with `children` under the same generation.
    #[serde(default)]
    pub can_edit_suggested: bool,
    /// Issue #1410: the in-flight or failed suggested toggle for the selected
    /// Space. Reset with the selection.
    #[serde(default)]
    pub suggested_edit: SpaceChildSuggestedEditState,
    /// Issue #1410: the one-time offer to join a newly joined Space's
    /// suggested rooms. Survives selection changes: an offer being answered
    /// or joined must not vanish when the user looks elsewhere.
    #[serde(default)]
    pub suggested_offer: SuggestedRoomsOffer,
    /// Issue #1410: rooms this client joined recently, newest last, bounded
    /// to [`RECENTLY_JOINED_ROOM_LIMIT`]. The first children load of a Space
    /// found here may make an offer and consumes the entry. Core-internal.
    #[serde(skip)]
    pub recently_joined_room_ids: RecentlyJoinedRoomIds,
}

impl SpaceChildrenState {
    /// The children of `space_id`, or nothing when another Space is selected.
    pub fn children_for(&self, space_id: &str) -> &[SpaceChildSummary] {
        match self.selected_space_id.as_deref() {
            Some(selected) if selected == space_id => &self.children,
            _ => &[],
        }
    }
}

/// Issue #1410: how many recently joined room IDs are kept for the
/// suggested-rooms offer.
pub const RECENTLY_JOINED_ROOM_LIMIT: usize = 16;

/// Recently joined room IDs (#1410). Debug output never carries the IDs.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct RecentlyJoinedRoomIds(pub Vec<String>);

impl fmt::Debug for RecentlyJoinedRoomIds {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecentlyJoinedRoomIds")
            .field("len", &self.0.len())
            .finish()
    }
}

impl RecentlyJoinedRoomIds {
    pub fn record(&mut self, room_id: &str) {
        self.0.retain(|recorded| recorded != room_id);
        self.0.push(room_id.to_owned());
        let excess = self.0.len().saturating_sub(RECENTLY_JOINED_ROOM_LIMIT);
        self.0.drain(..excess);
    }

    /// Remove `room_id`, reporting whether it was recorded.
    pub fn take(&mut self, room_id: &str) -> bool {
        let before = self.0.len();
        self.0.retain(|recorded| recorded != room_id);
        self.0.len() != before
    }
}

/// Issue #1410: the selected Space's suggested toggle.
#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SpaceChildSuggestedEditState {
    #[default]
    Idle,
    Saving {
        request_id: u64,
        child_room_id: String,
        suggested: bool,
    },
    Failed {
        child_room_id: String,
        suggested: bool,
        failure: OperationFailureKind,
    },
}

impl fmt::Debug for SpaceChildSuggestedEditState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => formatter.write_str("Idle"),
            Self::Saving {
                request_id,
                suggested,
                ..
            } => formatter
                .debug_struct("Saving")
                .field("request_id", request_id)
                .field("child_room_id", &"RoomId(..)")
                .field("suggested", suggested)
                .finish(),
            Self::Failed {
                suggested, failure, ..
            } => formatter
                .debug_struct("Failed")
                .field("child_room_id", &"RoomId(..)")
                .field("suggested", suggested)
                .field("failure", failure)
                .finish(),
        }
    }
}

/// Issue #1410: a suggested room offered for joining, named as the Space's
/// children projection named it when the offer was made.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct SuggestedRoomCandidate {
    pub room_id: String,
    pub display_name: String,
    #[serde(default)]
    pub display_name_placeholder: Option<super::room::RoomNamePlaceholder>,
}

impl fmt::Debug for SuggestedRoomCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SuggestedRoomCandidate")
            .field("room_id", &"RoomId(..)")
            .field("display_name", &"RoomName(..)")
            .finish()
    }
}

/// Issue #1410: one room's outcome in a suggested-rooms join, as Core reports
/// it. `failure` is `None` when the room was joined.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct SuggestedRoomJoinOutcome {
    pub room_id: String,
    #[serde(default)]
    pub failure: Option<OperationFailureKind>,
}

impl fmt::Debug for SuggestedRoomJoinOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SuggestedRoomJoinOutcome")
            .field("room_id", &"RoomId(..)")
            .field("failure", &self.failure)
            .finish()
    }
}

/// Issue #1410: a settled room in a suggested-rooms join, with its name.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct SuggestedRoomJoinResult {
    pub room_id: String,
    pub display_name: String,
    #[serde(default)]
    pub display_name_placeholder: Option<super::room::RoomNamePlaceholder>,
    #[serde(default)]
    pub failure: Option<OperationFailureKind>,
}

impl fmt::Debug for SuggestedRoomJoinResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SuggestedRoomJoinResult")
            .field("room_id", &"RoomId(..)")
            .field("display_name", &"RoomName(..)")
            .field("failure", &self.failure)
            .finish()
    }
}

/// Issue #1410: the opt-in offer to join a newly joined Space's suggested
/// rooms ([canon](../../../../docs/architecture/state-machine.md)).
#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SuggestedRoomsOffer {
    #[default]
    None,
    Offered {
        space_id: String,
        rooms: Vec<SuggestedRoomCandidate>,
    },
    Joining {
        request_id: u64,
        space_id: String,
        rooms: Vec<SuggestedRoomCandidate>,
    },
    /// At least one room failed; every room's result is listed.
    Completed {
        space_id: String,
        results: Vec<SuggestedRoomJoinResult>,
    },
}

impl SuggestedRoomsOffer {
    pub fn space_id(&self) -> Option<&str> {
        match self {
            Self::None => None,
            Self::Offered { space_id, .. }
            | Self::Joining { space_id, .. }
            | Self::Completed { space_id, .. } => Some(space_id),
        }
    }
}

impl fmt::Debug for SuggestedRoomsOffer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => formatter.write_str("None"),
            Self::Offered { rooms, .. } => formatter
                .debug_struct("Offered")
                .field("space_id", &"RoomId(..)")
                .field("rooms", rooms)
                .finish(),
            Self::Joining {
                request_id, rooms, ..
            } => formatter
                .debug_struct("Joining")
                .field("request_id", request_id)
                .field("space_id", &"RoomId(..)")
                .field("rooms", rooms)
                .finish(),
            Self::Completed { results, .. } => formatter
                .debug_struct("Completed")
                .field("space_id", &"RoomId(..)")
                .field("results", results)
                .finish(),
        }
    }
}
