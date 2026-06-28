use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::state::{
    AvatarImage, RoomNotificationMode, RoomNotificationSettings, RoomSummary, RoomTags,
    SpaceSummary,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SidebarModel {
    pub active_space_id: Option<String>,
    pub account_home: AccountHomeItem,
    pub space_rail: Vec<SpaceRailItem>,
    pub space_rooms: Vec<RoomListItem>,
    pub global_dms: Vec<RoomListItem>,
    pub space_unread_count: u64,
    pub dm_unread_count: u64,
    pub space_highlight_count: u64,
    pub dm_highlight_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AccountHomeItem {
    pub display_name: String,
    pub unread_count: u64,
    pub highlight_count: u64,
    pub is_active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SpaceRailItem {
    pub space_id: String,
    pub display_name: String,
    pub avatar: Option<AvatarImage>,
    pub unread_count: u64,
    pub highlight_count: u64,
    pub is_active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomListItem {
    pub room_id: String,
    pub display_name: String,
    pub avatar: Option<AvatarImage>,
    pub tags: RoomTags,
    pub unread_count: u64,
    pub highlight_count: u64,
}

pub fn compose_sidebar(
    active_space_id: Option<&str>,
    spaces: &[SpaceSummary],
    rooms: &[RoomSummary],
) -> SidebarModel {
    compose_sidebar_with_room_notification_settings(active_space_id, spaces, rooms, &HashMap::new())
}

pub fn compose_sidebar_with_room_notification_settings(
    active_space_id: Option<&str>,
    spaces: &[SpaceSummary],
    rooms: &[RoomSummary],
    room_notification_settings: &HashMap<String, RoomNotificationSettings>,
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
            avatar: space.avatar.clone(),
            unread_count: space_unread_count(space, &rooms_by_id, room_notification_settings),
            highlight_count: space_highlight_count(space, &rooms_by_id, room_notification_settings),
            is_active: active_space_id == Some(space.space_id.as_str()),
        })
        .collect();

    let account_home = AccountHomeItem {
        display_name: "Home".to_owned(),
        unread_count: rooms
            .iter()
            .filter(|room| !room_is_muted(&room.room_id, room_notification_settings))
            .map(|room| room.unread_count)
            .sum(),
        highlight_count: rooms
            .iter()
            .filter(|room| !room_is_muted(&room.room_id, room_notification_settings))
            .map(|room| room.highlight_count)
            .sum(),
        is_active: active_space_id.is_none(),
    };

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
                .filter(|room| !room.is_dm)
                .map(room_list_item)
                .collect()
        });

    let global_dms: Vec<_> = rooms
        .iter()
        .filter(|room| {
            room.is_dm
                && (active_space_id.is_none()
                    || room
                        .dm_space_ids
                        .iter()
                        .any(|space_id| Some(space_id.as_str()) == active_space_id))
        })
        .map(room_list_item)
        .collect();

    SidebarModel {
        active_space_id: active_space_id.map(str::to_owned),
        account_home,
        space_unread_count: unread_count(&space_rooms, room_notification_settings),
        dm_unread_count: unread_count(&global_dms, room_notification_settings),
        space_highlight_count: highlight_count(&space_rooms, room_notification_settings),
        dm_highlight_count: highlight_count(&global_dms, room_notification_settings),
        space_rail,
        space_rooms,
        global_dms,
    }
}

fn space_unread_count(
    space: &SpaceSummary,
    rooms_by_id: &HashMap<&str, &RoomSummary>,
    room_notification_settings: &HashMap<String, RoomNotificationSettings>,
) -> u64 {
    space
        .child_room_ids
        .iter()
        .filter_map(|room_id| rooms_by_id.get(room_id.as_str()).copied())
        .filter(|room| !room.is_dm)
        .filter(|room| !room_is_muted(&room.room_id, room_notification_settings))
        .map(|room| room.unread_count)
        .sum()
}

fn space_highlight_count(
    space: &SpaceSummary,
    rooms_by_id: &HashMap<&str, &RoomSummary>,
    room_notification_settings: &HashMap<String, RoomNotificationSettings>,
) -> u64 {
    space
        .child_room_ids
        .iter()
        .filter_map(|room_id| rooms_by_id.get(room_id.as_str()).copied())
        .filter(|room| !room.is_dm)
        .filter(|room| !room_is_muted(&room.room_id, room_notification_settings))
        .map(|room| room.highlight_count)
        .sum()
}

fn room_list_item(room: &RoomSummary) -> RoomListItem {
    RoomListItem {
        room_id: room.room_id.clone(),
        display_name: room.display_label.clone(),
        avatar: room.avatar.clone(),
        tags: room.tags.clone(),
        unread_count: room.unread_count,
        highlight_count: room.highlight_count,
    }
}

fn unread_count(
    rooms: &[RoomListItem],
    room_notification_settings: &HashMap<String, RoomNotificationSettings>,
) -> u64 {
    rooms
        .iter()
        .filter(|room| !room_is_muted(&room.room_id, room_notification_settings))
        .map(|room| room.unread_count)
        .sum()
}

fn highlight_count(
    rooms: &[RoomListItem],
    room_notification_settings: &HashMap<String, RoomNotificationSettings>,
) -> u64 {
    rooms
        .iter()
        .filter(|room| !room_is_muted(&room.room_id, room_notification_settings))
        .map(|room| room.highlight_count)
        .sum()
}

fn room_is_muted(
    room_id: &str,
    room_notification_settings: &HashMap<String, RoomNotificationSettings>,
) -> bool {
    room_notification_settings
        .get(room_id)
        .is_some_and(|settings| settings.mode == RoomNotificationMode::Mute)
}
