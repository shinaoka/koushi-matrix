use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::state::{
    AppState, AvatarImage, RoomListSort, RoomNotificationMode, RoomNotificationSettings,
    RoomSummary, RoomTags, SidebarScopeSettings, SpaceLocalPresentations, SpaceSummary,
    compare_conversation_activity, room_activity_unread_count, room_attention_projection,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SidebarModel {
    pub active_space_id: Option<String>,
    pub account_home: AccountHomeItem,
    pub space_rail: Vec<SpaceRailItem>,
    pub space_rooms: Vec<RoomListItem>,
    #[serde(default)]
    pub not_joined_space_rooms: Vec<RoomListItem>,
    pub global_dms: Vec<RoomListItem>,
    pub space_unread_count: u64,
    pub dm_unread_count: u64,
    pub space_highlight_count: u64,
    pub dm_highlight_count: u64,
    #[serde(default)]
    pub rooms_sort: RoomListSort,
    #[serde(default)]
    pub dms_sort: RoomListSort,
    #[serde(default)]
    pub rooms_collapsed: bool,
    #[serde(default)]
    pub dms_collapsed: bool,
    pub sections: SidebarSections,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SidebarSections {
    pub favourites: Vec<RoomListItem>,
    pub rooms: Vec<RoomListItem>,
    pub people: Vec<RoomListItem>,
    pub low_priority: Vec<RoomListItem>,
    pub not_joined: Vec<RoomListItem>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AccountHomeItem {
    pub display_name: String,
    /// Unread messages only. Invites are counted separately so the accessible
    /// rail label can name them individually (#330).
    pub unread_count: u64,
    pub highlight_count: u64,
    /// Invites pending for the account. Invites are not room-scoped attention,
    /// so room notification settings do not silence them.
    #[serde(default)]
    pub invite_count: u64,
    /// What the Home rail badge shows: `unread_count + invite_count`. Owned here
    /// rather than summed in the webview, because rail badges render a value
    /// this projection produced.
    #[serde(default)]
    pub attention_count: u64,
    pub is_active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SpaceRailItem {
    pub space_id: String,
    pub display_name: String,
    #[serde(default)]
    pub local_icon: Option<String>,
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
    #[serde(default)]
    pub notification_count: u64,
    #[serde(default)]
    pub display_count: u64,
    #[serde(default)]
    pub has_unread_content: bool,
    #[serde(default)]
    pub is_attention_highlighted: bool,
    #[serde(default)]
    pub has_unread_mention: bool,
    #[serde(default)]
    pub is_muted: bool,
}

/// Compose the sidebar from room/space facts alone.
///
/// Reports no pending invites, because a caller with only rooms and spaces does
/// not know about them. Callers that own `AppState` use
/// [`compose_sidebar_with_account_facts`].
pub fn compose_sidebar(
    active_space_id: Option<&str>,
    spaces: &[SpaceSummary],
    rooms: &[RoomSummary],
) -> SidebarModel {
    compose_sidebar_with_account_facts(active_space_id, spaces, rooms, &HashMap::new(), 0)
}

pub fn compose_sidebar_with_account_facts(
    active_space_id: Option<&str>,
    spaces: &[SpaceSummary],
    rooms: &[RoomSummary],
    room_notification_settings: &HashMap<String, RoomNotificationSettings>,
    pending_invite_count: u64,
) -> SidebarModel {
    compose_sidebar_with_preferences(
        active_space_id,
        spaces,
        rooms,
        room_notification_settings,
        pending_invite_count,
        RoomListSort::Activity,
        RoomListSort::Activity,
        SidebarScopeSettings::default(),
        &SpaceLocalPresentations::default(),
    )
}

pub fn compose_sidebar_for_state(state: &AppState) -> SidebarModel {
    let scope = state.settings.values.sidebar.scope(
        state.navigation.active_space_id.as_deref(),
        state.settings.values.room_list_sort,
    );
    let mut sidebar = compose_sidebar_with_preferences(
        state.navigation.active_space_id.as_deref(),
        &state.spaces,
        &state.rooms,
        &state.room_notification_settings,
        state.invites.len() as u64,
        scope.rooms.sort,
        scope.dms.sort,
        scope,
        &state.navigation.space_local_presentations,
    );
    let preferred_positions: HashMap<&str, usize> = state
        .navigation
        .space_order
        .iter()
        .enumerate()
        .map(|(position, space_id)| (space_id.as_str(), position))
        .collect();
    sidebar.space_rail.sort_by_key(|space| {
        preferred_positions
            .get(space.space_id.as_str())
            .copied()
            .unwrap_or(usize::MAX)
    });
    sidebar
}

fn compose_sidebar_with_preferences(
    active_space_id: Option<&str>,
    spaces: &[SpaceSummary],
    rooms: &[RoomSummary],
    room_notification_settings: &HashMap<String, RoomNotificationSettings>,
    pending_invite_count: u64,
    rooms_sort: RoomListSort,
    dms_sort: RoomListSort,
    section_settings: SidebarScopeSettings,
    local_presentations: &SpaceLocalPresentations,
) -> SidebarModel {
    let rooms_by_id: HashMap<&str, &RoomSummary> = rooms
        .iter()
        .map(|room| (room.room_id.as_str(), room))
        .collect();

    let space_rail = spaces
        .iter()
        .map(|space| {
            let local = local_presentations.0.get(&space.space_id);
            SpaceRailItem {
                space_id: space.space_id.clone(),
                display_name: local
                    .and_then(|presentation| presentation.name.as_ref())
                    .cloned()
                    .unwrap_or_else(|| space.display_name.clone()),
                local_icon: local.and_then(|presentation| presentation.icon.clone()),
                avatar: space.avatar.clone(),
                unread_count: space_unread_count(space, &rooms_by_id, room_notification_settings),
                highlight_count: space_highlight_count(
                    space,
                    &rooms_by_id,
                    room_notification_settings,
                ),
                is_active: active_space_id == Some(space.space_id.as_str()),
            }
        })
        .collect();

    let home_unread_count: u64 = rooms
        .iter()
        .filter(|room| !room_is_muted(&room.room_id, room_notification_settings))
        .map(room_activity_unread_count)
        .sum();
    let account_home = AccountHomeItem {
        display_name: "Home".to_owned(),
        unread_count: home_unread_count,
        highlight_count: rooms
            .iter()
            .filter(|room| !room_is_muted(&room.room_id, room_notification_settings))
            .map(|room| room.highlight_count)
            .sum(),
        invite_count: pending_invite_count,
        attention_count: home_unread_count + pending_invite_count,
        is_active: active_space_id.is_none(),
    };

    let mut space_rooms: Vec<_> = active_space_id
        .and_then(|space_id| spaces.iter().find(|space| space.space_id == space_id))
        .map(|space| {
            space
                .child_room_ids
                .iter()
                .filter_map(|room_id| rooms_by_id.get(room_id.as_str()).copied())
                .filter(|room| !room.is_dm)
                .collect()
        })
        .unwrap_or_else(|| rooms.iter().filter(|room| !room.is_dm).collect());
    sort_room_summaries(&mut space_rooms, rooms_sort, room_notification_settings);
    let space_rooms: Vec<_> = space_rooms
        .into_iter()
        .map(|room| room_list_item(room, room_notification_settings))
        .collect();

    let not_joined_space_rooms = Vec::new();

    let mut global_dm_rooms: Vec<_> = rooms
        .iter()
        .filter(|room| {
            room.is_dm
                && (active_space_id.is_none()
                    || room
                        .dm_space_ids
                        .iter()
                        .any(|space_id| Some(space_id.as_str()) == active_space_id))
        })
        .collect();
    sort_room_summaries(&mut global_dm_rooms, dms_sort, room_notification_settings);
    let global_dms: Vec<_> = global_dm_rooms
        .into_iter()
        .map(|room| room_list_item(room, room_notification_settings))
        .collect();
    let sections = SidebarSections {
        favourites: space_rooms
            .iter()
            .filter(|room| room.tags.favourite.is_some())
            .cloned()
            .collect(),
        rooms: space_rooms
            .iter()
            .filter(|room| room.tags.favourite.is_none() && room.tags.low_priority.is_none())
            .cloned()
            .collect(),
        people: global_dms.clone(),
        low_priority: space_rooms
            .iter()
            .filter(|room| room.tags.low_priority.is_some())
            .cloned()
            .collect(),
        not_joined: not_joined_space_rooms.clone(),
    };

    SidebarModel {
        active_space_id: active_space_id.map(str::to_owned),
        account_home,
        space_unread_count: unread_count(&space_rooms, room_notification_settings),
        dm_unread_count: unread_count(&global_dms, room_notification_settings),
        space_highlight_count: highlight_count(&space_rooms, room_notification_settings),
        dm_highlight_count: highlight_count(&global_dms, room_notification_settings),
        rooms_sort,
        dms_sort,
        rooms_collapsed: section_settings.rooms.collapsed,
        dms_collapsed: section_settings.dms.collapsed,
        sections,
        space_rail,
        space_rooms,
        not_joined_space_rooms,
        global_dms,
    }
}

fn sort_room_summaries(
    rooms: &mut Vec<&RoomSummary>,
    sort: RoomListSort,
    room_notification_settings: &HashMap<String, RoomNotificationSettings>,
) {
    rooms.sort_by(|left, right| match sort {
        RoomListSort::Activity => RoomSummary::compare_attention_activity(
            Some(*left),
            room_notification_settings
                .get(left.room_id.as_str())
                .map(|settings| settings.mode),
            Some(*right),
            room_notification_settings
                .get(right.room_id.as_str())
                .map(|settings| settings.mode),
        ),
        RoomListSort::RecentFirst => compare_conversation_activity(Some(*left), Some(*right)),
        RoomListSort::NormalLocale => left
            .display_label
            .to_lowercase()
            .cmp(&right.display_label.to_lowercase())
            .then_with(|| left.room_id.cmp(&right.room_id)),
    });
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
        .map(room_activity_unread_count)
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

fn room_list_item(
    room: &RoomSummary,
    room_notification_settings: &HashMap<String, RoomNotificationSettings>,
) -> RoomListItem {
    let mode = room_notification_settings
        .get(&room.room_id)
        .map(|settings| settings.mode);
    let projection = room_attention_projection(room, mode);
    RoomListItem {
        room_id: room.room_id.clone(),
        display_name: room.display_label.clone(),
        avatar: room.avatar.clone(),
        tags: room.tags.clone(),
        unread_count: projection.unread_count,
        highlight_count: projection.highlight_count,
        notification_count: projection.notification_count,
        display_count: projection.display_count,
        has_unread_content: projection.has_unread_content,
        is_attention_highlighted: projection.is_attention_highlighted,
        has_unread_mention: projection.has_unread_mention,
        is_muted: projection.is_muted,
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
