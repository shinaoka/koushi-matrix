//! Issue #961: a Space's advertised children, with the account's membership.
//!
//! The room list only carries joined rooms, so a Space's own room list could
//! never show what else the Space contains. The server answers that with
//! `/hierarchy`, which the SDK already wraps as the Element X-compatible
//! [`SpaceRoomList`]; this module drives it to completion once and projects the
//! result into Koushi's own vocabulary.
//!
//! A child the Space advertises may be absent from `/hierarchy` when it is an
//! invite-only room. If the sync client already knows that room (for example
//! because an invitation was received), use its local state to project the
//! name and membership. Only completely unknown rooms remain opaque.

use matrix_sdk::{RoomMemberships, RoomState};
use matrix_sdk_ui::spaces::{SpaceRoom, SpaceRoomList, room_list::SpaceRoomListPaginationState};
use ruma::{RoomId, room::JoinRuleSummary};

use crate::{
    client_session::MatrixClientSession,
    room_operations::MatrixRoomOperationError,
    room_operations::space_child_events_permitted,
    room_projection::{
        matrix_room, matrix_room_name_placeholder, matrix_space_child_room_ids,
        matrix_space_child_suggested_flags,
    },
};

/// The account's relationship to a Space child room.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MatrixSpaceChildMembership {
    Joined,
    Invited,
    Knocked,
    Left,
    Banned,
    NotJoined,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixSpaceChildEntry {
    pub room_id: String,
    /// The child's canonical `m.room.name`, when the server reported one.
    pub raw_name: Option<String>,
    /// The name the server computed, or the room ID when it described nothing.
    pub display_name: String,
    /// Set when `display_name` is the SDK's English calculated empty-room
    /// name (#1070), whose product text the GUI catalog owns.
    pub display_name_placeholder: Option<koushi_state::RoomNamePlaceholder>,
    pub avatar_mxc_uri: Option<String>,
    /// The child's description (`m.room.topic`) as the server or the local
    /// cache reports it (#1661). An empty topic is `None`.
    pub topic: Option<String>,
    pub membership: MatrixSpaceChildMembership,
    pub can_join: bool,
    pub is_space: bool,
    pub joined_members: u64,
    /// Issue #1410: the Space's `m.space.child` marks this child suggested.
    pub suggested: bool,
}

/// Every child `space_id` advertises, as the server is willing to describe it.
///
/// Pagination runs to the end before anything is projected: a half-read
/// hierarchy would show the user an arbitrary prefix of their Space and call it
/// the Space.
pub async fn matrix_space_children_projection(
    session: &MatrixClientSession,
    space_id: &str,
) -> Result<Vec<MatrixSpaceChildEntry>, MatrixRoomOperationError> {
    let space_room = matrix_room(session, space_id)?;
    let advertised_child_room_ids = matrix_space_child_room_ids(&space_room).await;
    let local_suggested = matrix_space_child_suggested_flags(&space_room).await;

    let room_list = SpaceRoomList::new(session.client(), space_room.room_id().to_owned()).await;
    loop {
        room_list
            .paginate()
            .await
            .map_err(|_| MatrixRoomOperationError::RoomUnavailable)?;
        match room_list.pagination_state() {
            SpaceRoomListPaginationState::Idle { end_reached: true } => break,
            SpaceRoomListPaginationState::Idle { end_reached: false } => continue,
            SpaceRoomListPaginationState::Loading => continue,
        }
    }

    let mut entries = Vec::new();
    for room in room_list.rooms().await {
        let room_id = room.room_id.to_string();
        let joined_members =
            hierarchy_child_joined_member_count(session, &room_id, room.num_joined_members).await;
        entries.push(MatrixSpaceChildEntry {
            room_id,
            raw_name: room
                .name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(ToOwned::to_owned),
            display_name: if room.display_name.trim().is_empty() {
                room.room_id.to_string()
            } else {
                room.display_name.trim().to_owned()
            },
            display_name_placeholder: hierarchy_name_placeholder(&room),
            avatar_mxc_uri: room.avatar_url.as_ref().map(ToString::to_string),
            topic: room.topic.clone().filter(|topic| !topic.is_empty()),
            membership: membership_from_room_state(room.state),
            can_join: can_join(room.state, room.join_rule.as_ref()),
            is_space: room.room_type == Some(ruma::room::RoomType::Space),
            joined_members,
            suggested: local_suggested
                .get(room.room_id.as_str())
                .copied()
                .unwrap_or(room.suggested),
        });
    }

    // A child the Space advertises that the hierarchy did not describe is one
    // this account may not see. Report it as unknown rather than omitting it:
    // the Space does contain it.
    for room_id in advertised_child_room_ids {
        if entries.iter().any(|entry| entry.room_id == room_id) {
            continue;
        }

        // `/hierarchy` may omit an invite-only child, while the sliding-sync
        // client still has the invited room locally. Prefer that local state
        // so the user sees its name and can accept the invitation.
        if let Some(room) = RoomId::parse(&room_id)
            .ok()
            .and_then(|parsed_room_id| session.client().get_room(&parsed_room_id))
        {
            let raw_name = room
                .name()
                .map(|name| name.trim().to_owned())
                .filter(|name| !name.is_empty());
            let cached_display_name = room
                .cached_display_name()
                .filter(|name| !name.to_string().trim().is_empty());
            let display_name_placeholder = cached_display_name
                .as_ref()
                .and_then(matrix_room_name_placeholder);
            let display_name = cached_display_name
                .map(|name| name.to_string())
                .or_else(|| raw_name.clone())
                .unwrap_or_else(|| room_id.clone());
            let state = room.state();
            let joined_members =
                local_joined_member_count(session, &room_id, room.joined_members_count()).await;
            // The same rule as a hierarchy-described child (#1053): where the
            // description came from must not change what the user is offered.
            let join_rule = room.join_rule().map(JoinRuleSummary::from);
            let can_join = can_join(Some(state), join_rule.as_ref());
            entries.push(MatrixSpaceChildEntry {
                suggested: local_suggested.get(&room_id).copied().unwrap_or(false),
                room_id,
                raw_name,
                display_name,
                display_name_placeholder,
                avatar_mxc_uri: room.avatar_url().map(|url| url.to_string()),
                topic: room.topic().filter(|topic| !topic.is_empty()),
                membership: membership_from_room_state(Some(state)),
                can_join,
                is_space: room.is_space(),
                joined_members,
            });
            continue;
        }

        entries.push(MatrixSpaceChildEntry {
            suggested: local_suggested.get(&room_id).copied().unwrap_or(false),
            room_id: room_id.clone(),
            raw_name: None,
            display_name: room_id,
            display_name_placeholder: None,
            avatar_mxc_uri: None,
            topic: None,
            membership: MatrixSpaceChildMembership::Unknown,
            can_join: false,
            is_space: false,
            joined_members: 0,
        });
    }

    entries.sort_by(|left, right| left.room_id.cmp(&right.room_id));
    entries.dedup_by(|left, right| left.room_id == right.room_id);
    // A child with no current joined members is an abandoned room (for
    // example, a room whose last member left). Pending invitations remain
    // actionable even when the room has no joined members.
    entries.retain(space_child_is_visible);
    Ok(entries)
}

/// Issue #1410: whether this account may change which children `space_id`
/// suggests, i.e. send `m.space.child` there. The homeserver stays the final
/// authority; this only decides whether the control is offered.
pub async fn matrix_space_can_edit_children(
    session: &MatrixClientSession,
    space_id: &str,
) -> Result<bool, MatrixRoomOperationError> {
    let space_room = matrix_room(session, space_id)?;
    let client = session.client();
    let Some(user_id) = client.user_id() else {
        return Ok(false);
    };
    if space_room.state() != RoomState::Joined {
        return Ok(false);
    }
    Ok(space_child_events_permitted(&space_room, user_id).await)
}

/// The structured form of a hierarchy child's calculated empty-room name
/// (#1070). `SpaceRoom` keeps only the SDK's English `Display` text, so the
/// variant is re-derived from the same name, alias, and heroes and accepted
/// only when its text is exactly what the SDK produced. An empty-room name
/// requires at most one effective member, and the SDK's service-member
/// adjustment can only lower the count, so one member is the count that
/// reproduces it; a literal room name "Empty Room" is `Named` and never
/// matches.
fn hierarchy_name_placeholder(room: &SpaceRoom) -> Option<koushi_state::RoomNamePlaceholder> {
    let calculated = matrix_sdk::BaseRoom::compute_display_name_with_fields(
        room.name.clone(),
        room.canonical_alias.as_deref(),
        room.heroes
            .iter()
            .flatten()
            .map(matrix_sdk::RoomHero::from)
            .collect(),
        1,
    );
    if calculated.to_string().trim() != room.display_name.trim() {
        return None;
    }
    matrix_room_name_placeholder(&calculated)
}

fn space_child_is_visible(entry: &MatrixSpaceChildEntry) -> bool {
    entry.joined_members > 0
        || matches!(
            entry.membership,
            MatrixSpaceChildMembership::Joined
                | MatrixSpaceChildMembership::Invited
                | MatrixSpaceChildMembership::Knocked
        )
}

/// The joined-member count for a child `/hierarchy` described (#1062).
///
/// Sync keeps a room's member list current only while the account is in it.
/// Once the account has left, the local list is frozen at the moment of
/// leaving and can never reach zero on its own, so for any room the account is
/// not joined to, the server's fresh count is the authority.
async fn hierarchy_child_joined_member_count(
    session: &MatrixClientSession,
    room_id: &str,
    hierarchy_count: u64,
) -> u64 {
    let joined_locally = RoomId::parse(room_id)
        .ok()
        .and_then(|parsed_room_id| session.client().get_room(&parsed_room_id))
        .is_some_and(|room| room.state() == RoomState::Joined);
    if !joined_locally {
        return hierarchy_count;
    }
    local_joined_member_count(session, room_id, hierarchy_count).await
}

async fn local_joined_member_count(
    session: &MatrixClientSession,
    room_id: &str,
    fallback: u64,
) -> u64 {
    let Some(room) = RoomId::parse(room_id)
        .ok()
        .and_then(|parsed_room_id| session.client().get_room(&parsed_room_id))
    else {
        return fallback;
    };
    room.members_no_sync(RoomMemberships::JOIN)
        .await
        .map(|members| members.len() as u64)
        .unwrap_or(fallback)
}

fn membership_from_room_state(state: Option<RoomState>) -> MatrixSpaceChildMembership {
    match state {
        Some(RoomState::Joined) => MatrixSpaceChildMembership::Joined,
        Some(RoomState::Invited) => MatrixSpaceChildMembership::Invited,
        Some(RoomState::Knocked) => MatrixSpaceChildMembership::Knocked,
        Some(RoomState::Left) => MatrixSpaceChildMembership::Left,
        Some(RoomState::Banned) => MatrixSpaceChildMembership::Banned,
        // The client does not know the room at all, which for a room the
        // hierarchy described means the account is not in it.
        None => MatrixSpaceChildMembership::NotJoined,
    }
}

/// Whether a join attempt is worth offering.
///
/// Only rules that admit a plain join say yes. Knocking is a different request
/// than joining, and an invite-only room cannot be joined by asking, so neither
/// is presented as a join the user can make.
///
/// Both a hierarchy-described child and one known only from local state are
/// judged here (#1053); a local `JoinRule` is converted to its summary form
/// first, so the two sources cannot disagree about the same room.
fn can_join(state: Option<RoomState>, join_rule: Option<&JoinRuleSummary>) -> bool {
    match state {
        Some(RoomState::Joined) | Some(RoomState::Banned) => false,
        // Accepting a pending invitation is a join.
        Some(RoomState::Invited) => true,
        _ => matches!(
            join_rule,
            Some(JoinRuleSummary::Public) | Some(JoinRuleSummary::Restricted(_))
        ),
    }
}

#[cfg(test)]
mod mock_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(
        membership: MatrixSpaceChildMembership,
        can_join: bool,
        joined_members: u64,
    ) -> MatrixSpaceChildEntry {
        MatrixSpaceChildEntry {
            room_id: "!room:example.invalid".to_owned(),
            raw_name: Some("Room".to_owned()),
            display_name: "Room".to_owned(),
            display_name_placeholder: None,
            avatar_mxc_uri: None,
            topic: None,
            membership,
            can_join,
            is_space: false,
            joined_members,
            suggested: false,
        }
    }

    #[test]
    fn only_rules_that_admit_a_plain_join_offer_one() {
        use ruma::{
            events::room::join_rules::{AllowRule, JoinRule, Restricted},
            owned_room_id,
        };

        let restricted = Restricted::new(vec![AllowRule::room_membership(owned_room_id!(
            "!allowed:example.invalid"
        ))]);
        for (rule, expected) in [
            (JoinRule::Public, true),
            (JoinRule::Restricted(restricted.clone()), true),
            (JoinRule::KnockRestricted(restricted), false),
            (JoinRule::Knock, false),
            (JoinRule::Invite, false),
        ] {
            let summary = JoinRuleSummary::from(rule);
            assert_eq!(
                can_join(Some(RoomState::Left), Some(&summary)),
                expected,
                "{summary:?}"
            );
            assert_eq!(can_join(None, Some(&summary)), expected, "{summary:?}");
        }
        assert!(can_join(
            Some(RoomState::Invited),
            Some(&JoinRuleSummary::Invite)
        ));
    }

    #[test]
    fn abandoned_empty_room_is_hidden() {
        assert!(!space_child_is_visible(&entry(
            MatrixSpaceChildMembership::NotJoined,
            false,
            0,
        )));
        assert!(!space_child_is_visible(&entry(
            MatrixSpaceChildMembership::Left,
            false,
            0,
        )));
    }

    #[test]
    fn actionable_or_nonempty_child_remains_visible() {
        assert!(!space_child_is_visible(&entry(
            MatrixSpaceChildMembership::NotJoined,
            true,
            0,
        )));
        assert!(space_child_is_visible(&entry(
            MatrixSpaceChildMembership::Invited,
            false,
            0,
        )));
        assert!(space_child_is_visible(&entry(
            MatrixSpaceChildMembership::Unknown,
            false,
            2,
        )));
        assert!(!space_child_is_visible(&entry(
            MatrixSpaceChildMembership::Unknown,
            true,
            0,
        )));
        assert!(!space_child_is_visible(&entry(
            MatrixSpaceChildMembership::Unknown,
            false,
            0,
        )));
    }
}
