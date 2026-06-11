use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::state::{RoomSummary, SpaceSummary};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SidebarModel {
    pub active_space_id: Option<String>,
    pub space_rail: Vec<SpaceRailItem>,
    pub space_rooms: Vec<RoomListItem>,
    pub global_dms: Vec<RoomListItem>,
    pub space_unread_count: u64,
    pub dm_unread_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SpaceRailItem {
    pub space_id: String,
    pub display_name: String,
    pub unread_count: u64,
    pub is_active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomListItem {
    pub room_id: String,
    pub display_name: String,
    pub unread_count: u64,
}

pub fn compose_sidebar(
    active_space_id: Option<&str>,
    spaces: &[SpaceSummary],
    rooms: &[RoomSummary],
) -> SidebarModel {
    let rooms_by_id: HashMap<&str, &RoomSummary> = rooms
        .iter()
        .map(|room| (room.room_id.as_str(), room))
        .collect();

    let space_rail = spaces
        .iter()
        .map(|space| SpaceRailItem {
            space_id: space.space_id.clone(),
            display_name: space.display_name.clone(),
            unread_count: space_unread_count(space, &rooms_by_id),
            is_active: active_space_id == Some(space.space_id.as_str()),
        })
        .collect();

    let space_rooms: Vec<_> = active_space_id
        .and_then(|space_id| spaces.iter().find(|space| space.space_id == space_id))
        .map(|space| {
            space
                .child_room_ids
                .iter()
                .filter_map(|room_id| rooms_by_id.get(room_id.as_str()).copied())
                .filter(|room| !room.is_dm)
                .map(room_list_item)
                .collect()
        })
        .unwrap_or_else(|| {
            rooms
                .iter()
                .filter(|room| !room.is_dm && room.parent_space_ids.is_empty())
                .map(room_list_item)
                .collect()
        });

    let global_dms: Vec<_> = rooms
        .iter()
        .filter(|room| room.is_dm)
        .map(room_list_item)
        .collect();

    SidebarModel {
        active_space_id: active_space_id.map(str::to_owned),
        space_unread_count: unread_count(&space_rooms),
        dm_unread_count: unread_count(&global_dms),
        space_rail,
        space_rooms,
        global_dms,
    }
}

fn space_unread_count(space: &SpaceSummary, rooms_by_id: &HashMap<&str, &RoomSummary>) -> u64 {
    space
        .child_room_ids
        .iter()
        .filter_map(|room_id| rooms_by_id.get(room_id.as_str()).copied())
        .filter(|room| !room.is_dm)
        .map(|room| room.unread_count)
        .sum()
}

fn room_list_item(room: &RoomSummary) -> RoomListItem {
    RoomListItem {
        room_id: room.room_id.clone(),
        display_name: room.display_name.clone(),
        unread_count: room.unread_count,
    }
}

fn unread_count(rooms: &[RoomListItem]) -> u64 {
    rooms.iter().map(|room| room.unread_count).sum()
}
