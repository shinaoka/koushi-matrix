use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::state::{
    AppState, AvatarImage, InvitePreview, RoomListSort, RoomNotificationMode,
    RoomNotificationSettings, RoomSummary, RoomTags, SidebarScopeSettings, SpaceChildMembership,
    SpaceChildSummary, SpaceChildrenState, SpaceLocalPresentations, SpaceSummary,
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
    #[serde(default)]
    pub low_priority_collapsed: bool,
    pub sections: SidebarSections,
    /// The active Space's add-existing-room rows (#1007); `None` at Home.
    #[serde(default)]
    pub space_add_rooms: Option<crate::space_add_rooms::SpaceAddRoomsModel>,
}

/// The Rust-owned visible sidebar sections.
///
/// `rooms`, `people`, and `low_priority` are mutually exclusive over the
/// current Home/Space scope, so one conversation renders exactly once
/// (state-machine.md, "Sidebar Sections And Low Priority"). `favourites` is a
/// derived convenience list of favourite rooms that also appear in `rooms`; it
/// is not a separate visible section.
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
    /// Issue #961: the viewer's relationship to this room. Every item the
    /// joined room list produces is `Joined`; the not-joined lane is the only
    /// producer of anything else, so an older snapshot loads as joined.
    #[serde(default = "joined_membership")]
    pub membership: SpaceChildMembership,
    /// Whether this row offers a join. Always false for a joined room; the
    /// not-joined lane is the only producer of a true.
    #[serde(default)]
    pub can_join: bool,
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
        &SpaceChildrenState::default(),
        &[],
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
        &state.space_children,
        &state.invites,
    );
    let preferred_positions: HashMap<&str, usize> = state
        .navigation
        .space_order
        .iter()
        .enumerate()
        .map(|(position, space_id)| (space_id.as_str(), position))
        .collect();
    sidebar.space_add_rooms = crate::space_add_rooms::space_add_rooms_for_state(state);
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
    space_children: &SpaceChildrenState,
    invites: &[InvitePreview],
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
        .filter(|room| contributes_attention(room, room_notification_settings))
        .map(room_activity_unread_count)
        .sum();
    let account_home = AccountHomeItem {
        display_name: "Home".to_owned(),
        unread_count: home_unread_count,
        highlight_count: rooms
            .iter()
            .filter(|room| contributes_attention(room, room_notification_settings))
            .map(|room| room.highlight_count)
            .sum(),
        invite_count: pending_invite_count,
        attention_count: home_unread_count + pending_invite_count,
        is_active: active_space_id.is_none(),
    };

    let mut space_room_summaries: Vec<&RoomSummary> = active_space_id
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
    sort_room_summaries(
        &mut space_room_summaries,
        rooms_sort,
        room_notification_settings,
    );
    let space_rooms: Vec<_> = space_room_summaries
        .iter()
        .map(|room| room_list_item(room, room_notification_settings))
        .collect();

    // Issue #961: the Space's advertised children the account is not in. The
    // joined room list stays authoritative: a child that is already a joined
    // room belongs to the lanes above, whatever a `/hierarchy` response that
    // crossed a join says, and a pending invitation is reported as invited
    // from the account's own invite list rather than from the server summary.
    let invited_room_ids: HashSet<&str> = invites
        .iter()
        .map(|invite| invite.room_id.as_str())
        .collect();
    let mut not_joined_children: Vec<&SpaceChildSummary> = active_space_id
        .map(|space_id| space_children.children_for(space_id))
        .unwrap_or_default()
        .iter()
        // Absence from the joined room list is what decides, not the cached
        // `/hierarchy` membership: a room the user just left is gone from
        // `rooms` long before the next hierarchy fetch would say so, and it
        // must reappear here rather than vanish from the Space entirely.
        .filter(|child| !rooms_by_id.contains_key(child.room_id.as_str()))
        // The hierarchy can contain stale children after every member has
        // left. Keep actionable invitations/knocks and opaque private rooms,
        // but do not expose a described room with no joined members.
        .filter(|child| space_child_is_visible(child, &invited_room_ids))
        // Keep advertised children visible even when the hierarchy endpoint
        // could not describe them. Encrypted/private rooms can legitimately
        // arrive as `Unknown`; hiding them makes a real Space child disappear.
        .collect();
    not_joined_children.sort_by(|left, right| {
        left.display_name
            .to_lowercase()
            .cmp(&right.display_name.to_lowercase())
            .then_with(|| left.room_id.cmp(&right.room_id))
    });
    let not_joined_space_rooms: Vec<RoomListItem> = not_joined_children
        .into_iter()
        .map(|child| not_joined_room_list_item(child, &invited_room_ids))
        .collect();

    let mut global_dm_summaries: Vec<&RoomSummary> = rooms
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
    sort_room_summaries(
        &mut global_dm_summaries,
        dms_sort,
        room_notification_settings,
    );
    let global_dms: Vec<_> = global_dm_summaries
        .iter()
        .map(|room| room_list_item(room, room_notification_settings))
        .collect();
    // Low priority spans both scope lists and is shown once, in the Rooms
    // order, so a low-priority DM and room interleave by the same comparator.
    let mut low_priority_summaries: Vec<&RoomSummary> = space_room_summaries
        .iter()
        .chain(global_dm_summaries.iter())
        .copied()
        .filter(|room| room.tags.low_priority.is_some())
        .collect();
    sort_room_summaries(
        &mut low_priority_summaries,
        rooms_sort,
        room_notification_settings,
    );

    let sections = SidebarSections {
        favourites: space_rooms
            .iter()
            .filter(|room| room.tags.favourite.is_some() && room.tags.low_priority.is_none())
            .cloned()
            .collect(),
        rooms: space_rooms
            .iter()
            .filter(|room| room.tags.low_priority.is_none())
            .cloned()
            .collect(),
        people: global_dms
            .iter()
            .filter(|room| room.tags.low_priority.is_none())
            .cloned()
            .collect(),
        low_priority: low_priority_summaries
            .into_iter()
            .map(|room| room_list_item(room, room_notification_settings))
            .collect(),
        not_joined: not_joined_space_rooms.clone(),
    };

    SidebarModel {
        active_space_id: active_space_id.map(str::to_owned),
        account_home,
        space_unread_count: unread_count(&sections.rooms, room_notification_settings),
        dm_unread_count: unread_count(&sections.people, room_notification_settings),
        space_highlight_count: highlight_count(&sections.rooms, room_notification_settings),
        dm_highlight_count: highlight_count(&sections.people, room_notification_settings),
        rooms_sort,
        dms_sort,
        rooms_collapsed: section_settings.rooms.collapsed,
        dms_collapsed: section_settings.dms.collapsed,
        low_priority_collapsed: section_settings.low_priority.unwrap_or_default().collapsed,
        sections,
        space_rail,
        space_rooms,
        not_joined_space_rooms,
        global_dms,
        space_add_rooms: None,
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
        .filter(|room| contributes_attention(room, room_notification_settings))
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
        .filter(|room| contributes_attention(room, room_notification_settings))
        .map(|room| room.highlight_count)
        .sum()
}

/// Issue #961: a room the account has not joined has no read state of its own,
/// so every attention field is zero and no tag can apply to it.
fn not_joined_room_list_item(
    child: &SpaceChildSummary,
    invited_room_ids: &HashSet<&str>,
) -> RoomListItem {
    // The account's own lists outrank the cached server summary in both
    // directions: an invitation it no longer holds is not an invitation, and a
    // room it is no longer in is not joined.
    let invited = invited_room_ids.contains(child.room_id.as_str());
    let membership = match (invited, child.membership) {
        (true, _) => SpaceChildMembership::Invited,
        (false, SpaceChildMembership::Joined | SpaceChildMembership::Invited) => {
            SpaceChildMembership::NotJoined
        }
        (false, membership) => membership,
    };
    RoomListItem {
        room_id: child.room_id.clone(),
        membership,
        // Accepting an invitation this account holds is always available; for
        // everything else the server's join rule decides.
        can_join: invited || child.can_join,
        display_name: child.display_name.clone(),
        avatar: child.avatar.clone(),
        tags: RoomTags::default(),
        unread_count: 0,
        highlight_count: 0,
        notification_count: 0,
        display_count: 0,
        has_unread_content: false,
        is_attention_highlighted: false,
        has_unread_mention: false,
        is_muted: false,
    }
}

fn joined_membership() -> SpaceChildMembership {
    SpaceChildMembership::Joined
}

fn space_child_is_visible(child: &SpaceChildSummary, invited_room_ids: &HashSet<&str>) -> bool {
    // The current invite list is authoritative even when the hierarchy only
    // reports the room as a generic non-joined child.
    if invited_room_ids.contains(child.room_id.as_str()) {
        return true;
    }
    match child.membership {
        SpaceChildMembership::Joined => true,
        SpaceChildMembership::Unknown => child.joined_members > 0,
        // The hierarchy can retain an old invited state after the invite was
        // withdrawn or accepted. Only the account's current invite list makes
        // that state actionable.
        SpaceChildMembership::Invited => invited_room_ids.contains(child.room_id.as_str()),
        SpaceChildMembership::Knocked => true,
        SpaceChildMembership::NotJoined
        | SpaceChildMembership::Left
        | SpaceChildMembership::Banned => child.joined_members > 0,
    }
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
        membership: SpaceChildMembership::Joined,
        can_join: false,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn child(membership: SpaceChildMembership, joined_members: u64) -> SpaceChildSummary {
        SpaceChildSummary {
            room_id: "!room:example.invalid".to_owned(),
            display_name: "Room".to_owned(),
            avatar: None,
            membership,
            can_join: false,
            is_space: false,
            joined_members,
        }
    }

    #[test]
    fn sidebar_hides_empty_described_children_but_keeps_actionable_or_opaque_rooms() {
        let no_invites = HashSet::new();
        assert!(!space_child_is_visible(
            &child(SpaceChildMembership::NotJoined, 0),
            &no_invites
        ));
        assert!(!space_child_is_visible(
            &child(SpaceChildMembership::Left, 0),
            &no_invites
        ));
        assert!(space_child_is_visible(
            &child(SpaceChildMembership::NotJoined, 1),
            &no_invites
        ));
        assert!(!space_child_is_visible(
            &child(SpaceChildMembership::Invited, 0),
            &no_invites
        ));
        let mut current_invites = HashSet::new();
        current_invites.insert("!room:example.invalid");
        assert!(space_child_is_visible(
            &child(SpaceChildMembership::Invited, 0),
            &current_invites
        ));
        assert!(!space_child_is_visible(
            &child(SpaceChildMembership::Unknown, 0),
            &no_invites
        ));
        assert!(space_child_is_visible(
            &child(SpaceChildMembership::Unknown, 2),
            &no_invites
        ));
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

/// Whether a room feeds the Home/Space/Rooms/DMs attention aggregates.
///
/// Muted and low-priority conversations keep their own raw counts but never
/// contribute to an aggregate badge (state-machine.md, "Sidebar Sections And
/// Low Priority").
fn contributes_attention(
    room: &RoomSummary,
    room_notification_settings: &HashMap<String, RoomNotificationSettings>,
) -> bool {
    room.tags.low_priority.is_none() && !room_is_muted(&room.room_id, room_notification_settings)
}

fn room_is_muted(
    room_id: &str,
    room_notification_settings: &HashMap<String, RoomNotificationSettings>,
) -> bool {
    room_notification_settings
        .get(room_id)
        .is_some_and(|settings| settings.mode == RoomNotificationMode::Mute)
}
