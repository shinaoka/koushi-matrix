use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomInput {
    pub room_id: String,
    pub display_name: String,
    pub is_dm: bool,
    pub unread_count: u64,
    pub parent_space_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SpaceInput {
    pub space_id: String,
    pub display_name: String,
    pub child_room_ids: Vec<String>,
}

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
    spaces: &[SpaceInput],
    rooms: &[RoomInput],
) -> SidebarModel {
    let rooms_by_id: HashMap<&str, &RoomInput> = rooms
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

    let space_rooms: Vec<RoomListItem> = active_space_id
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
        space_rail,
        space_unread_count: room_list_unread_count(&space_rooms),
        dm_unread_count: room_list_unread_count(&global_dms),
        space_rooms,
        global_dms,
    }
}

fn space_unread_count(space: &SpaceInput, rooms_by_id: &HashMap<&str, &RoomInput>) -> u64 {
    space
        .child_room_ids
        .iter()
        .filter_map(|room_id| rooms_by_id.get(room_id.as_str()).copied())
        .filter(|room| !room.is_dm)
        .map(|room| room.unread_count)
        .sum()
}

fn room_list_item(room: &RoomInput) -> RoomListItem {
    RoomListItem {
        room_id: room.room_id.clone(),
        display_name: room.display_name.clone(),
        unread_count: room.unread_count,
    }
}

fn room_list_unread_count(rooms: &[RoomListItem]) -> u64 {
    rooms.iter().map(|room| room.unread_count).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_space_filters_rooms_and_keeps_dms_global() {
        let spaces = vec![
            SpaceInput {
                space_id: "space-a".into(),
                display_name: "Space A".into(),
                child_room_ids: vec!["room-a".into(), "dm-a".into()],
            },
            SpaceInput {
                space_id: "space-b".into(),
                display_name: "Space B".into(),
                child_room_ids: vec!["room-b".into()],
            },
        ];
        let rooms = vec![
            RoomInput {
                room_id: "room-a".into(),
                display_name: "Room A".into(),
                is_dm: false,
                unread_count: 5,
                parent_space_ids: vec!["space-a".into()],
            },
            RoomInput {
                room_id: "room-b".into(),
                display_name: "Room B".into(),
                is_dm: false,
                unread_count: 7,
                parent_space_ids: vec!["space-b".into()],
            },
            RoomInput {
                room_id: "dm-a".into(),
                display_name: "Alice".into(),
                is_dm: true,
                unread_count: 3,
                parent_space_ids: vec!["space-a".into()],
            },
        ];

        let model = compose_sidebar(Some("space-a"), &spaces, &rooms);

        assert_eq!(
            model,
            SidebarModel {
                active_space_id: Some("space-a".into()),
                space_rail: vec![
                    SpaceRailItem {
                        space_id: "space-a".into(),
                        display_name: "Space A".into(),
                        unread_count: 5,
                        is_active: true,
                    },
                    SpaceRailItem {
                        space_id: "space-b".into(),
                        display_name: "Space B".into(),
                        unread_count: 7,
                        is_active: false,
                    },
                ],
                space_rooms: vec![RoomListItem {
                    room_id: "room-a".into(),
                    display_name: "Room A".into(),
                    unread_count: 5,
                }],
                global_dms: vec![RoomListItem {
                    room_id: "dm-a".into(),
                    display_name: "Alice".into(),
                    unread_count: 3,
                }],
                space_unread_count: 5,
                dm_unread_count: 3,
            }
        );
    }

    #[test]
    fn fallback_shows_unparented_rooms_and_keeps_dms_global() {
        let spaces = vec![SpaceInput {
            space_id: "space-a".into(),
            display_name: "Space A".into(),
            child_room_ids: vec!["room-in-space".into()],
        }];
        let rooms = vec![
            RoomInput {
                room_id: "room-global".into(),
                display_name: "Global Room".into(),
                is_dm: false,
                unread_count: 2,
                parent_space_ids: vec![],
            },
            RoomInput {
                room_id: "room-in-space".into(),
                display_name: "Room In Space".into(),
                is_dm: false,
                unread_count: 5,
                parent_space_ids: vec!["space-a".into()],
            },
            RoomInput {
                room_id: "dm-global".into(),
                display_name: "Alice".into(),
                is_dm: true,
                unread_count: 3,
                parent_space_ids: vec![],
            },
        ];

        let model = compose_sidebar(None, &spaces, &rooms);

        assert_eq!(
            model.space_rooms,
            vec![RoomListItem {
                room_id: "room-global".into(),
                display_name: "Global Room".into(),
                unread_count: 2,
            }]
        );
        assert_eq!(
            model.global_dms,
            vec![RoomListItem {
                room_id: "dm-global".into(),
                display_name: "Alice".into(),
                unread_count: 3,
            }]
        );
        assert_eq!(model.space_unread_count, 2);
        assert_eq!(model.dm_unread_count, 3);

        let missing_space_model = compose_sidebar(Some("missing-space"), &spaces, &rooms);
        assert_eq!(missing_space_model.space_rooms, model.space_rooms);
    }
}
