use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct NavigationState {
    pub active_space_id: Option<String>,
    pub active_room_id: Option<String>,
    #[serde(default)]
    pub space_order: Vec<String>,
    #[serde(default)]
    pub last_room_by_space_id: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RoomListFilter {
    #[default]
    Rooms,
    Unread,
    People,
    Favourites,
    Invites,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RoomListSort {
    #[default]
    Activity,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomListProjection {
    pub active_filter: RoomListFilter,
    pub sort: RoomListSort,
    pub items: Vec<RoomListProjectionItem>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomListProjectionItem {
    pub room_id: String,
    pub kind: RoomListEntryKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomListEntryKind {
    Room,
    Invite,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FocusedContextState {
    Closed,
    Opening {
        room_id: String,
        event_id: String,
    },
    Open {
        room_id: String,
        event_id: String,
        is_subscribed: bool,
    },
}

/// Rust-owned room-list filter + activity-sort projection. React renders this
/// snapshot and must not recompute filter membership or sort order.
pub fn compute_room_list_projection(
    active_filter: RoomListFilter,
    sort: RoomListSort,
    rooms: &[super::room::RoomSummary],
    invites: &[super::room::InvitePreview],
) -> RoomListProjection {
    let mut items: Vec<RoomListProjectionItem> = match active_filter {
        RoomListFilter::Invites => invites
            .iter()
            .map(|invite| RoomListProjectionItem {
                room_id: invite.room_id.clone(),
                kind: RoomListEntryKind::Invite,
            })
            .collect(),
        _ => rooms
            .iter()
            .filter(|room| match active_filter {
                RoomListFilter::Unread => {
                    room.unread_count > 0
                        || room.notification_count > 0
                        || room.highlight_count > 0
                        || room.marked_unread
                }
                RoomListFilter::People => room.is_dm,
                RoomListFilter::Rooms => !room.is_dm,
                RoomListFilter::Favourites => room.tags.favourite.is_some(),
                RoomListFilter::Invites => unreachable!(),
            })
            .map(|room| RoomListProjectionItem {
                room_id: room.room_id.clone(),
                kind: RoomListEntryKind::Room,
            })
            .collect(),
    };

    if sort == RoomListSort::Activity {
        let activity_by_id: std::collections::HashMap<&str, u64> = rooms
            .iter()
            .map(|room| (room.room_id.as_str(), room.last_activity_ms))
            .collect();
        items.sort_by(|left, right| {
            let left_ts = activity_by_id
                .get(left.room_id.as_str())
                .copied()
                .unwrap_or_default();
            let right_ts = activity_by_id
                .get(right.room_id.as_str())
                .copied()
                .unwrap_or_default();
            right_ts
                .cmp(&left_ts)
                .then_with(|| left.room_id.cmp(&right.room_id))
        });
    }

    RoomListProjection {
        active_filter,
        sort,
        items,
    }
}
