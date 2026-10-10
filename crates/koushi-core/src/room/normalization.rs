use super::mentions::user_profile_mention_search_terms;
use koushi_sdk::MatrixRoomTags;
use koushi_state::{
    AvatarImage, AvatarThumbnailState, InvitePreview, RoomSummary, RoomTagInfo, RoomTags,
    SpaceSummary, UserProfile,
};
use std::{
    collections::BTreeSet,
    sync::{Arc, RwLock},
};

/// Convert `MatrixRoomListSnapshot` spaces into `SpaceSummary` values with
/// child room id lists. Homeservers may sync one side of the Matrix space
/// relationship before the other, so the projection uses both the space's
/// `m.space.child` state and rooms' `m.space.parent` state.
pub(super) fn normalize_spaces(snapshot: &koushi_sdk::MatrixRoomListSnapshot) -> Vec<SpaceSummary> {
    snapshot
        .spaces
        .iter()
        .map(|space| {
            let child_room_ids = normalize_space_child_room_ids(snapshot, space);
            SpaceSummary {
                space_id: space.space_id.clone(),
                raw_name: space.raw_name.clone(),
                display_name: space.display_name.clone(),
                avatar: avatar_from_mxc_uri(space.avatar_mxc_uri.as_deref()),
                join_rule: Some(super::management::room_join_rule_from_sdk(space.join_rule)),
                child_room_ids,
                // The Space's own `m.space.child` list, kept separate from the
                // display union so add-existing eligibility stays parent-side
                // (#1007); it is a subset of `child_room_ids`.
                parent_side_child_room_ids: space.child_room_ids.clone(),
            }
        })
        .collect()
}

fn normalize_space_child_room_ids(
    snapshot: &koushi_sdk::MatrixRoomListSnapshot,
    space: &koushi_sdk::MatrixRoomListSpace,
) -> Vec<String> {
    let mut child_room_ids = BTreeSet::new();
    child_room_ids.extend(space.child_room_ids.iter().cloned());
    child_room_ids.extend(
        snapshot
            .rooms
            .iter()
            .filter(|room| room.parent_space_ids.iter().any(|id| id == &space.space_id))
            .map(|room| room.room_id.clone()),
    );
    child_room_ids.into_iter().collect()
}

/// Convert `MatrixRoomListSnapshot` rooms into `RoomSummary` values.
#[cfg(test)]
pub(super) fn normalize_rooms(snapshot: &koushi_sdk::MatrixRoomListSnapshot) -> Vec<RoomSummary> {
    normalize_rooms_with_previous(snapshot, &[])
}

pub(super) fn normalize_rooms_with_previous(
    snapshot: &koushi_sdk::MatrixRoomListSnapshot,
    previous_rooms: &[RoomSummary],
) -> Vec<RoomSummary> {
    let mut rooms: Vec<RoomSummary> = snapshot
        .rooms
        .iter()
        .map(|room| {
            let display_label = if room.display_name.trim().is_empty() {
                room.room_id.clone()
            } else {
                room.display_name.trim().to_owned()
            };
            RoomSummary {
                display_name_placeholder: room.display_name_placeholder.clone(),
                display_label_placeholder: room.display_name_placeholder.clone(),
                room_id: room.room_id.clone(),
                display_name: room.display_name.clone(),
                display_label: display_label.clone(),
                original_display_label: display_label,
                avatar: avatar_from_mxc_uri(room.avatar_mxc_uri.as_deref()),
                is_dm: room.is_dm,
                dm_user_ids: room.dm_user_ids.clone(),
                tags: normalize_room_tags(&room.tags),
                unread_count: room.unread_count,
                notification_count: room.notification_count,
                highlight_count: room.highlight_count,
                // #1238: a room-list snapshot has no thread-cache facts. The reducer
                // re-derives both fields from the per-root values it already holds
                // when this projection is installed, so they start empty here.
                thread_unread_count: 0,
                thread_highlight_count: 0,
                marked_unread: room.marked_unread,
                recency_stamp: room.recency_stamp,
                conversation_activity: room.conversation_activity.map(|activity| {
                    koushi_state::ConversationActivity {
                        timestamp_ms: activity.timestamp_ms,
                        source: match activity.source {
                            koushi_sdk::MatrixConversationActivitySource::Message => {
                                koushi_state::ConversationActivitySource::Message
                            }
                            koushi_sdk::MatrixConversationActivitySource::EncryptedMessage => {
                                koushi_state::ConversationActivitySource::EncryptedMessage
                            }
                            koushi_sdk::MatrixConversationActivitySource::ThreadReply => {
                                koushi_state::ConversationActivitySource::ThreadReply
                            }
                        },
                    }
                }),
                latest_event: room.latest_event.as_ref().map(|event| {
                    koushi_state::RoomLatestEventSummary {
                        event_id: event.event_id.clone(),
                        relation_type: event.relation_type.clone(),
                        relation_event_id: event.relation_event_id.clone(),
                        thread_root_event_id: event.thread_root_event_id.clone(),
                        sender_id: event.sender_id.clone(),
                        sender_label: event.sender_label.clone(),
                        sender_avatar: avatar_from_mxc_uri(event.sender_avatar_mxc_uri.as_deref()),
                        preview: event.preview.clone(),
                        timestamp_ms: event.timestamp_ms,
                        is_redacted: event.is_redacted,
                    }
                }),
                parent_space_ids: normalize_room_parent_space_ids(snapshot, room),
                dm_space_ids: Vec::new(),
                is_encrypted: room.is_encrypted,
                joined_members: room.joined_members,
            }
        })
        .collect();
    let space_members: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        snapshot
            .spaces
            .iter()
            .map(|s| {
                (
                    s.space_id.clone(),
                    s.member_user_ids.iter().cloned().collect(),
                )
            })
            .collect();
    assign_dm_space_ids(&mut rooms, &space_members);

    let previous_dm_spaces: std::collections::BTreeMap<&str, BTreeSet<&str>> = previous_rooms
        .iter()
        .filter(|room| room.is_dm)
        .map(|room| {
            (
                room.room_id.as_str(),
                room.dm_space_ids.iter().map(String::as_str).collect(),
            )
        })
        .collect();
    for room in rooms.iter_mut().filter(|room| room.is_dm) {
        let Some(previous) = previous_dm_spaces.get(room.room_id.as_str()) else {
            continue;
        };
        room.dm_space_ids.extend(
            snapshot
                .spaces
                .iter()
                .filter(|space| {
                    !snapshot.complete_space_member_ids.contains(&space.space_id)
                        && previous.contains(space.space_id.as_str())
                })
                .map(|space| space.space_id.clone()),
        );
        room.dm_space_ids.sort();
        room.dm_space_ids.dedup();
    }
    rooms
}

fn normalize_room_parent_space_ids(
    snapshot: &koushi_sdk::MatrixRoomListSnapshot,
    room: &koushi_sdk::MatrixRoomListRoom,
) -> Vec<String> {
    let mut parent_space_ids: BTreeSet<String> = room.parent_space_ids.iter().cloned().collect();
    parent_space_ids.extend(
        snapshot
            .spaces
            .iter()
            .filter(|space| space.child_room_ids.iter().any(|id| id == &room.room_id))
            .map(|space| space.space_id.clone()),
    );
    parent_space_ids.into_iter().collect()
}

/// Populate `dm_space_ids` on each `RoomSummary` in `rooms`.
///
/// For each DM room, `dm_space_ids` is set to the sorted list of space IDs
/// (keys of `space_members`) whose member set contains at least one of
/// `room.dm_user_ids`. Non-DM rooms always get an empty `dm_space_ids`.
///
/// The result is deterministically ordered because `space_members` is a
/// `BTreeMap` and iteration yields keys in ascending order.
pub fn assign_dm_space_ids(
    rooms: &mut [koushi_state::RoomSummary],
    space_members: &std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
) {
    for room in rooms.iter_mut() {
        if !room.is_dm {
            room.dm_space_ids = Vec::new();
            continue;
        }
        room.dm_space_ids = space_members
            .iter()
            .filter(|(_space_id, members)| room.dm_user_ids.iter().any(|uid| members.contains(uid)))
            .map(|(space_id, _)| space_id.clone())
            .collect();
    }
}

fn normalize_room_tags(tags: &MatrixRoomTags) -> RoomTags {
    RoomTags {
        favourite: tags.favourite.as_ref().map(|info| RoomTagInfo {
            order: info.order.clone(),
        }),
        low_priority: tags.low_priority.as_ref().map(|info| RoomTagInfo {
            order: info.order.clone(),
        }),
    }
}

pub(super) fn normalize_user_profiles(
    snapshot: &koushi_sdk::MatrixRoomListSnapshot,
) -> Vec<UserProfile> {
    snapshot
        .user_profiles
        .iter()
        .map(|profile| {
            let display_label = profile
                .display_name
                .as_deref()
                .map(str::trim)
                .filter(|display_name| !display_name.is_empty())
                .unwrap_or(profile.user_id.as_str())
                .to_owned();
            UserProfile {
                user_id: profile.user_id.clone(),
                display_name: profile.display_name.clone(),
                display_label: display_label.clone(),
                original_display_label: display_label,
                mention_search_terms: user_profile_mention_search_terms(
                    &profile.user_id,
                    profile.display_name.as_deref(),
                ),
                avatar: avatar_from_mxc_uri(profile.avatar_mxc_uri.as_deref()),
            }
        })
        .collect()
}

pub(super) fn replace_known_room_ids(
    known_room_ids: &Arc<RwLock<BTreeSet<String>>>,
    rooms: &[RoomSummary],
) {
    if let Ok(mut known_room_ids) = known_room_ids.write() {
        *known_room_ids = rooms.iter().map(|room| room.room_id.clone()).collect();
    }
}

/// Convert `MatrixRoomListSnapshot` invites into Rust-owned invite previews.
pub(super) fn normalize_invites(
    snapshot: &koushi_sdk::MatrixRoomListSnapshot,
) -> Vec<InvitePreview> {
    snapshot
        .invites
        .iter()
        .map(|invite| InvitePreview {
            room_id: invite.room_id.clone(),
            display_name: invite.display_name.clone(),
            display_name_placeholder: invite.display_name_placeholder.clone(),
            avatar: avatar_from_mxc_uri(invite.avatar_mxc_uri.as_deref()),
            topic: invite.topic.clone(),
            inviter_display_name: invite.inviter_display_name.clone(),
            inviter_user_id: invite.inviter_user_id.clone(),
            is_dm: invite.is_dm,
            is_space: invite.is_space,
        })
        .collect()
}

pub(super) fn avatar_from_mxc_uri(mxc_uri: Option<&str>) -> Option<AvatarImage> {
    mxc_uri.map(|mxc_uri| AvatarImage {
        mxc_uri: mxc_uri.to_owned(),
        thumbnail: AvatarThumbnailState::NotRequested,
    })
}

#[cfg(test)]
mod invite_tests;
#[cfg(test)]
mod tests;
