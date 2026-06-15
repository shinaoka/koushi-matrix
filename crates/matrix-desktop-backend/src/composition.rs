use matrix_desktop_state::{AppAction, RoomSummary, RoomTags, SpaceSummary};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DesktopRoomListUpdate {
    pub spaces: Vec<DesktopRoomListSpace>,
    pub rooms: Vec<DesktopRoomListRoom>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DesktopRoomListSpace {
    pub space_id: String,
    pub display_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DesktopRoomListRoom {
    pub room_id: String,
    pub display_name: String,
    pub is_dm: bool,
    pub unread_count: u64,
    pub notification_count: u64,
    pub highlight_count: u64,
    pub parent_space_ids: Vec<String>,
}

pub fn compose_room_list_update(update: DesktopRoomListUpdate) -> AppAction {
    let known_space_ids = update
        .spaces
        .iter()
        .map(|space| space.space_id.clone())
        .collect::<Vec<_>>();

    let mut spaces = update
        .spaces
        .into_iter()
        .map(|space| SpaceSummary {
            space_id: space.space_id,
            display_name: space.display_name,
            avatar: None,
            child_room_ids: Vec::new(),
        })
        .collect::<Vec<_>>();

    let rooms = update
        .rooms
        .into_iter()
        .map(|room| {
            let parent_space_ids = if room.is_dm {
                Vec::new()
            } else {
                normalized_parent_space_ids(room.parent_space_ids, &known_space_ids)
            };

            for parent_space_id in &parent_space_ids {
                if let Some(space) = spaces
                    .iter_mut()
                    .find(|space| space.space_id == *parent_space_id)
                {
                    space.child_room_ids.push(room.room_id.clone());
                }
            }

            RoomSummary {
                room_id: room.room_id,
                display_name: room.display_name,
                avatar: None,
                is_dm: room.is_dm,
                tags: RoomTags::default(),
                unread_count: room.unread_count,
                notification_count: room.notification_count,
                highlight_count: room.highlight_count,
                parent_space_ids,
            }
        })
        .collect();

    AppAction::RoomListUpdated { spaces, rooms }
}

fn normalized_parent_space_ids(
    parent_space_ids: Vec<String>,
    known_space_ids: &[String],
) -> Vec<String> {
    let mut normalized = Vec::new();
    for parent_space_id in parent_space_ids {
        if known_space_ids.contains(&parent_space_id) && !normalized.contains(&parent_space_id) {
            normalized.push(parent_space_id);
        }
    }
    normalized
}
