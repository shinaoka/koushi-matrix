use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimelineScrollAnchorEdge {
    #[default]
    Top,
    Bottom,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineScrollAnchor {
    pub event_id: String,
    #[serde(default)]
    pub edge: TimelineScrollAnchorEdge,
    pub offset_px: i32,
    pub updated_at_ms: u64,
}

/// Rust-owned main-pane timeline mode (issue #161).
///
/// `NavigationState.main_timeline_anchor == None` means the main pane renders
/// the room's live timeline (live edge). `Some` means jump-to-date resolved an
/// event outside the loaded live window and the main pane is anchored to that
/// event — it renders the event-focused timeline until the user returns to live.
/// This is a guarded state machine: `Some` is only ever set for the active,
/// known room and is cleared on any room change or return-to-live.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MainTimelineAnchor {
    pub event_id: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct NavigationState {
    pub active_space_id: Option<String>,
    pub active_room_id: Option<String>,
    #[serde(default)]
    pub space_order: Vec<String>,
    #[serde(default)]
    pub last_room_by_space_id: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub room_scroll_anchors: BTreeMap<String, TimelineScrollAnchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main_timeline_anchor: Option<MainTimelineAnchor>,
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

pub use crate::state::settings::RoomListSort;

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
    active_space_id: Option<&str>,
    spaces: &[super::room::SpaceSummary],
    rooms: &[super::room::RoomSummary],
    invites: &[super::room::InvitePreview],
) -> RoomListProjection {
    let active_space_child_room_ids = active_space_id.and_then(|active_space_id| {
        spaces
            .iter()
            .find(|space| space.space_id == active_space_id)
            .map(|space| space.child_room_ids.as_slice())
    });
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
            .filter(|room| {
                room_visible_in_active_space(room, active_space_id, active_space_child_room_ids)
            })
            .filter(|room| match active_filter {
                RoomListFilter::Unread => {
                    room.unread_count > 0
                        || room.notification_count > 0
                        || room.highlight_count > 0
                        || room.marked_unread
                }
                RoomListFilter::People => room.is_dm,
                RoomListFilter::Rooms => {
                    !room.is_dm && room.tags.favourite.is_none() && room.tags.low_priority.is_none()
                }
                RoomListFilter::Favourites => room.tags.favourite.is_some(),
                RoomListFilter::Invites => unreachable!(),
            })
            .map(|room| RoomListProjectionItem {
                room_id: room.room_id.clone(),
                kind: RoomListEntryKind::Room,
            })
            .collect(),
    };

    match sort {
        RoomListSort::Activity => {
            let room_by_id: std::collections::HashMap<&str, &super::room::RoomSummary> = rooms
                .iter()
                .map(|room| (room.room_id.as_str(), room))
                .collect();
            items.sort_by(|left, right| {
                super::room::compare_conversation_activity(
                    room_by_id.get(left.room_id.as_str()).copied(),
                    room_by_id.get(right.room_id.as_str()).copied(),
                )
            });
        }
        RoomListSort::RecentFirst => {
            let room_by_id: std::collections::HashMap<&str, &super::room::RoomSummary> = rooms
                .iter()
                .map(|room| (room.room_id.as_str(), room))
                .collect();
            items.sort_by(|left, right| {
                super::room::compare_conversation_activity(
                    room_by_id.get(left.room_id.as_str()).copied(),
                    room_by_id.get(right.room_id.as_str()).copied(),
                )
            });
        }
        RoomListSort::NormalLocale => {
            let label_by_id: std::collections::HashMap<&str, &str> = rooms
                .iter()
                .map(|room| (room.room_id.as_str(), room.display_label.as_str()))
                .collect();
            items.sort_by(|left, right| {
                let left_label = label_by_id
                    .get(left.room_id.as_str())
                    .copied()
                    .unwrap_or(left.room_id.as_str())
                    .to_lowercase();
                let right_label = label_by_id
                    .get(right.room_id.as_str())
                    .copied()
                    .unwrap_or(right.room_id.as_str())
                    .to_lowercase();
                left_label
                    .cmp(&right_label)
                    .then_with(|| left.room_id.cmp(&right.room_id))
            });
        }
    }

    RoomListProjection {
        active_filter,
        sort,
        items,
    }
}

fn room_visible_in_active_space(
    room: &super::room::RoomSummary,
    active_space_id: Option<&str>,
    active_space_child_room_ids: Option<&[String]>,
) -> bool {
    let Some(active_space_id) = active_space_id else {
        return true;
    };
    if room.is_dm {
        return true;
    }
    room.parent_space_ids
        .iter()
        .any(|space_id| space_id == active_space_id)
        || active_space_child_room_ids.is_some_and(|child_room_ids| {
            child_room_ids
                .iter()
                .any(|room_id| room_id == &room.room_id)
        })
}
