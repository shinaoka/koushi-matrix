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
                display_name: space.display_name.clone(),
                avatar: avatar_from_mxc_uri(space.avatar_mxc_uri.as_deref()),
                child_room_ids,
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
pub(super) fn normalize_rooms(snapshot: &koushi_sdk::MatrixRoomListSnapshot) -> Vec<RoomSummary> {
    let mut rooms: Vec<RoomSummary> = snapshot
        .rooms
        .iter()
        .map(|room| {
            let display_label = room
                .display_name
                .trim()
                .is_empty()
                .then(|| room.room_id.clone())
                .unwrap_or_else(|| room.display_name.trim().to_owned());
            RoomSummary {
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
            avatar: avatar_from_mxc_uri(invite.avatar_mxc_uri.as_deref()),
            topic: invite.topic.clone(),
            inviter_display_name: invite.inviter_display_name.clone(),
            inviter_user_id: invite.inviter_user_id.clone(),
            is_dm: invite.is_dm,
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
mod tests {
    use super::{normalize_invites, normalize_rooms, normalize_spaces, normalize_user_profiles};

    use koushi_sdk::{
        MatrixConversationActivity, MatrixConversationActivitySource, MatrixInvitePreview,
        MatrixRoomLatestEventSummary, MatrixRoomTagInfo,
    };
    use koushi_sdk::{
        MatrixRoomListRoom, MatrixRoomListSnapshot, MatrixRoomListSpace, MatrixRoomTags,
    };

    use koushi_state::{AvatarImage, AvatarThumbnailState, RoomTagInfo, UserProfile};

    #[test]
    fn normalize_rooms_preserves_typed_conversation_activity_and_opaque_recency() {
        let snapshot = MatrixRoomListSnapshot {
            rooms: vec![MatrixRoomListRoom {
                room_id: "!dm:example.test".to_owned(),
                display_name: "Synthetic DM".to_owned(),
                avatar_mxc_uri: None,
                is_dm: true,
                dm_user_ids: vec!["@member:example.test".to_owned()],
                tags: MatrixRoomTags::default(),
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                marked_unread: false,
                recency_stamp: Some(9),
                conversation_activity: Some(MatrixConversationActivity {
                    timestamp_ms: 42,
                    source: MatrixConversationActivitySource::EncryptedMessage,
                }),
                latest_event: None,
                parent_space_ids: Vec::new(),
                is_encrypted: true,
                joined_members: 2,
            }],
            ..MatrixRoomListSnapshot::default()
        };

        let rooms = normalize_rooms(&snapshot);
        let room = rooms.first().expect("normalized room");

        assert_eq!(room.recency_stamp, Some(9));
        assert_eq!(
            room.conversation_activity,
            Some(koushi_state::ConversationActivity {
                timestamp_ms: 42,
                source: koushi_state::ConversationActivitySource::EncryptedMessage,
            })
        );
    }

    #[test]
    fn normalize_rooms_preserves_latest_redaction_fact() {
        let snapshot = MatrixRoomListSnapshot {
            rooms: vec![MatrixRoomListRoom {
                room_id: "!room:example.test".to_owned(),
                display_name: "Room".to_owned(),
                avatar_mxc_uri: None,
                is_dm: false,
                dm_user_ids: Vec::new(),
                tags: MatrixRoomTags::default(),
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                marked_unread: false,
                recency_stamp: None,
                conversation_activity: None,
                latest_event: Some(MatrixRoomLatestEventSummary {
                    event_id: "$redacted:example.test".to_owned(),
                    sender_id: None,
                    sender_label: None,
                    sender_avatar_mxc_uri: None,
                    preview: Some("deleted".to_owned()),
                    timestamp_ms: 42,
                    event_type: Some("m.room.message".to_owned()),
                    relation_type: None,
                    relation_event_id: None,
                    content_converted: true,
                    is_threaded: false,
                    is_reply: false,
                    has_thread_summary: false,
                    has_reactions: false,
                    is_redacted: true,
                }),
                parent_space_ids: Vec::new(),
                is_encrypted: false,
                joined_members: 1,
            }],
            ..MatrixRoomListSnapshot::default()
        };

        let rooms = normalize_rooms(&snapshot);

        assert!(
            rooms[0]
                .latest_event
                .as_ref()
                .is_some_and(|event| event.is_redacted)
        );
    }

    #[test]
    fn normalize_spaces_with_child_rooms() {
        let snapshot = MatrixRoomListSnapshot {
            spaces: vec![MatrixRoomListSpace {
                space_id: "!space1:example.test".to_owned(),
                display_name: "My Space".to_owned(),
                avatar_mxc_uri: None,
                child_room_ids: Vec::new(),
                member_user_ids: Vec::new(),
            }],
            rooms: vec![
                MatrixRoomListRoom {
                    room_id: "!room1:example.test".to_owned(),
                    display_name: "Room 1".to_owned(),
                    avatar_mxc_uri: None,
                    is_dm: false,
                    dm_user_ids: Vec::new(),
                    tags: MatrixRoomTags::default(),
                    unread_count: 0,
                    notification_count: 0,
                    highlight_count: 0,
                    marked_unread: false,
                    recency_stamp: None,
                    conversation_activity: None,
                    latest_event: None,
                    parent_space_ids: vec!["!space1:example.test".to_owned()],
                    is_encrypted: false,
                    joined_members: 0,
                },
                MatrixRoomListRoom {
                    room_id: "!room2:example.test".to_owned(),
                    display_name: "Room 2".to_owned(),
                    avatar_mxc_uri: None,
                    is_dm: false,
                    dm_user_ids: Vec::new(),
                    tags: MatrixRoomTags::default(),
                    unread_count: 0,
                    notification_count: 0,
                    highlight_count: 0,
                    marked_unread: false,
                    recency_stamp: None,
                    conversation_activity: None,
                    latest_event: None,
                    parent_space_ids: vec![],
                    is_encrypted: false,
                    joined_members: 0,
                },
            ],
            ..MatrixRoomListSnapshot::default()
        };
        let spaces = normalize_spaces(&snapshot);
        assert_eq!(spaces.len(), 1);
        assert_eq!(spaces[0].space_id, "!space1:example.test");
        assert_eq!(spaces[0].child_room_ids, vec!["!room1:example.test"]);
    }

    #[test]
    fn normalize_spaces_uses_direct_space_child_state() {
        let snapshot = MatrixRoomListSnapshot {
            spaces: vec![MatrixRoomListSpace {
                space_id: "!space1:example.test".to_owned(),
                display_name: "My Space".to_owned(),
                avatar_mxc_uri: None,
                child_room_ids: vec!["!room1:example.test".to_owned()],
                member_user_ids: Vec::new(),
            }],
            rooms: vec![MatrixRoomListRoom {
                room_id: "!room1:example.test".to_owned(),
                display_name: "Room 1".to_owned(),
                avatar_mxc_uri: None,
                is_dm: false,
                dm_user_ids: Vec::new(),
                tags: MatrixRoomTags::default(),
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                marked_unread: false,
                recency_stamp: None,
                conversation_activity: None,
                latest_event: None,
                parent_space_ids: Vec::new(),
                is_encrypted: false,
                joined_members: 0,
            }],
            ..MatrixRoomListSnapshot::default()
        };

        let spaces = normalize_spaces(&snapshot);

        assert_eq!(spaces.len(), 1);
        assert_eq!(spaces[0].child_room_ids, vec!["!room1:example.test"]);
    }

    #[test]
    fn normalize_spaces_no_children() {
        let snapshot = MatrixRoomListSnapshot {
            spaces: vec![MatrixRoomListSpace {
                space_id: "!space:example.test".to_owned(),
                display_name: "Empty Space".to_owned(),
                avatar_mxc_uri: None,
                child_room_ids: Vec::new(),
                member_user_ids: Vec::new(),
            }],
            rooms: vec![],
            ..MatrixRoomListSnapshot::default()
        };
        let spaces = normalize_spaces(&snapshot);
        assert_eq!(spaces.len(), 1);
        assert_eq!(spaces[0].child_room_ids, Vec::<String>::new());
    }

    #[test]
    fn normalize_spaces_preserves_avatar_mxc_as_unrequested_thumbnail() {
        let snapshot = MatrixRoomListSnapshot {
            spaces: vec![MatrixRoomListSpace {
                space_id: "!space:example.test".to_owned(),
                display_name: "Space".to_owned(),
                avatar_mxc_uri: Some("mxc://example.test/space-avatar".to_owned()),
                child_room_ids: Vec::new(),
                member_user_ids: Vec::new(),
            }],
            ..MatrixRoomListSnapshot::default()
        };
        let spaces = normalize_spaces(&snapshot);

        let avatar = spaces[0].avatar.as_ref().expect("space avatar");
        assert_eq!(avatar.mxc_uri, "mxc://example.test/space-avatar");
        assert_eq!(avatar.thumbnail, AvatarThumbnailState::NotRequested);
    }

    #[test]
    fn normalize_rooms_preserves_dm_and_unread() {
        let snapshot = MatrixRoomListSnapshot {
            spaces: vec![],
            rooms: vec![MatrixRoomListRoom {
                room_id: "!dm:example.test".to_owned(),
                display_name: "Alice".to_owned(),
                avatar_mxc_uri: None,
                is_dm: true,
                dm_user_ids: vec!["@alice:example.test".to_owned()],
                tags: MatrixRoomTags::default(),
                unread_count: 3,
                notification_count: 3,
                highlight_count: 1,
                marked_unread: false,
                recency_stamp: None,
                conversation_activity: None,
                latest_event: None,
                parent_space_ids: vec![],
                is_encrypted: false,
                joined_members: 0,
            }],
            ..MatrixRoomListSnapshot::default()
        };
        let rooms = normalize_rooms(&snapshot);
        assert_eq!(rooms.len(), 1);
        assert_eq!(rooms[0].room_id, "!dm:example.test");
        assert!(rooms[0].is_dm);
        assert_eq!(rooms[0].unread_count, 3);
        assert_eq!(rooms[0].notification_count, 3);
        assert_eq!(rooms[0].highlight_count, 1);
    }

    #[test]
    fn normalize_rooms_non_dm() {
        let snapshot = MatrixRoomListSnapshot {
            spaces: vec![],
            rooms: vec![MatrixRoomListRoom {
                room_id: "!room:example.test".to_owned(),
                display_name: "General".to_owned(),
                avatar_mxc_uri: None,
                is_dm: false,
                dm_user_ids: Vec::new(),
                tags: MatrixRoomTags::default(),
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                marked_unread: false,
                recency_stamp: None,
                conversation_activity: None,
                latest_event: None,
                parent_space_ids: vec!["!space:example.test".to_owned()],
                is_encrypted: false,
                joined_members: 0,
            }],
            ..MatrixRoomListSnapshot::default()
        };
        let rooms = normalize_rooms(&snapshot);
        assert_eq!(rooms.len(), 1);
        assert!(!rooms[0].is_dm);
        assert_eq!(rooms[0].parent_space_ids, vec!["!space:example.test"]);
        assert_eq!(rooms[0].notification_count, 0);
        assert_eq!(rooms[0].highlight_count, 0);
    }

    #[test]
    fn normalize_rooms_uses_direct_space_child_state_as_parent() {
        let snapshot = MatrixRoomListSnapshot {
            spaces: vec![MatrixRoomListSpace {
                space_id: "!space:example.test".to_owned(),
                display_name: "Space".to_owned(),
                avatar_mxc_uri: None,
                child_room_ids: vec!["!room:example.test".to_owned()],
                member_user_ids: Vec::new(),
            }],
            rooms: vec![MatrixRoomListRoom {
                room_id: "!room:example.test".to_owned(),
                display_name: "General".to_owned(),
                avatar_mxc_uri: None,
                is_dm: false,
                dm_user_ids: Vec::new(),
                tags: MatrixRoomTags::default(),
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                marked_unread: false,
                recency_stamp: None,
                conversation_activity: None,
                latest_event: None,
                parent_space_ids: Vec::new(),
                is_encrypted: false,
                joined_members: 0,
            }],
            ..MatrixRoomListSnapshot::default()
        };

        let rooms = normalize_rooms(&snapshot);

        assert_eq!(rooms.len(), 1);
        assert_eq!(rooms[0].parent_space_ids, vec!["!space:example.test"]);
    }

    #[test]
    fn normalize_rooms_assigns_dm_space_ids_by_counterpart_membership() {
        let snapshot = MatrixRoomListSnapshot {
            spaces: vec![MatrixRoomListSpace {
                space_id: "space-a".to_owned(),
                display_name: "Space A".to_owned(),
                avatar_mxc_uri: None,
                child_room_ids: Vec::new(),
                member_user_ids: vec!["@alice".to_owned()],
            }],
            rooms: vec![
                MatrixRoomListRoom {
                    room_id: "dm-alice".to_owned(),
                    display_name: "Alice".to_owned(),
                    avatar_mxc_uri: None,
                    is_dm: true,
                    dm_user_ids: vec!["@alice".to_owned()],
                    tags: MatrixRoomTags::default(),
                    unread_count: 0,
                    notification_count: 0,
                    highlight_count: 0,
                    marked_unread: false,
                    recency_stamp: None,
                    conversation_activity: None,
                    latest_event: None,
                    parent_space_ids: Vec::new(),
                    is_encrypted: false,
                    joined_members: 0,
                },
                MatrixRoomListRoom {
                    room_id: "dm-bob".to_owned(),
                    display_name: "Bob".to_owned(),
                    avatar_mxc_uri: None,
                    is_dm: true,
                    dm_user_ids: vec!["@bob".to_owned()],
                    tags: MatrixRoomTags::default(),
                    unread_count: 0,
                    notification_count: 0,
                    highlight_count: 0,
                    marked_unread: false,
                    recency_stamp: None,
                    conversation_activity: None,
                    latest_event: None,
                    parent_space_ids: Vec::new(),
                    is_encrypted: false,
                    joined_members: 0,
                },
            ],
            ..MatrixRoomListSnapshot::default()
        };
        let rooms = normalize_rooms(&snapshot);
        let alice_room = rooms.iter().find(|r| r.room_id == "dm-alice").unwrap();
        let bob_room = rooms.iter().find(|r| r.room_id == "dm-bob").unwrap();
        assert_eq!(alice_room.dm_space_ids, vec!["space-a"]);
        assert_eq!(bob_room.dm_space_ids, Vec::<String>::new());
    }

    #[test]
    fn normalize_rooms_preserves_avatar_mxc_as_unrequested_thumbnail() {
        let snapshot = MatrixRoomListSnapshot {
            rooms: vec![MatrixRoomListRoom {
                room_id: "!room:example.test".to_owned(),
                display_name: "General".to_owned(),
                avatar_mxc_uri: Some("mxc://example.test/room-avatar".to_owned()),
                is_dm: false,
                dm_user_ids: Vec::new(),
                tags: MatrixRoomTags::default(),
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                marked_unread: false,
                recency_stamp: None,
                conversation_activity: None,
                latest_event: None,
                parent_space_ids: vec![],
                is_encrypted: false,
                joined_members: 0,
            }],
            ..MatrixRoomListSnapshot::default()
        };
        let rooms = normalize_rooms(&snapshot);

        let avatar = rooms[0].avatar.as_ref().expect("room avatar");
        assert_eq!(avatar.mxc_uri, "mxc://example.test/room-avatar");
        assert_eq!(avatar.thumbnail, AvatarThumbnailState::NotRequested);
    }

    #[test]
    fn normalize_invites_preserves_preview_fields() {
        let snapshot = MatrixRoomListSnapshot {
            invites: vec![MatrixInvitePreview {
                room_id: "!invite:example.test".to_owned(),
                display_name: "Project invite".to_owned(),
                avatar_mxc_uri: None,
                topic: Some("Project topic".to_owned()),
                inviter_display_name: Some("Inviter".to_owned()),
                inviter_user_id: Some("@inviter:example.test".to_owned()),
                is_dm: true,
            }],
            ..MatrixRoomListSnapshot::default()
        };
        let invites = normalize_invites(&snapshot);

        assert_eq!(invites.len(), 1);
        assert_eq!(invites[0].room_id, "!invite:example.test");
        assert_eq!(invites[0].display_name, "Project invite");
        assert_eq!(invites[0].topic.as_deref(), Some("Project topic"));
        assert_eq!(invites[0].inviter_display_name.as_deref(), Some("Inviter"));
        assert!(invites[0].is_dm);
    }

    #[test]
    fn normalize_invites_preserves_avatar_mxc_as_unrequested_thumbnail() {
        let snapshot = MatrixRoomListSnapshot {
            invites: vec![MatrixInvitePreview {
                room_id: "!invite:example.test".to_owned(),
                display_name: "Invite".to_owned(),
                avatar_mxc_uri: Some("mxc://example.test/invite-avatar".to_owned()),
                topic: None,
                inviter_display_name: None,
                inviter_user_id: None,
                is_dm: false,
            }],
            ..MatrixRoomListSnapshot::default()
        };
        let invites = normalize_invites(&snapshot);

        let avatar = invites[0].avatar.as_ref().expect("invite avatar");
        assert_eq!(avatar.mxc_uri, "mxc://example.test/invite-avatar");
        assert_eq!(avatar.thumbnail, AvatarThumbnailState::NotRequested);
    }

    #[test]
    fn normalize_user_profiles_preserves_member_profile_fields() {
        let snapshot = MatrixRoomListSnapshot {
            user_profiles: vec![koushi_sdk::MatrixUserProfile {
                user_id: "@alice:example.test".to_owned(),
                display_name: Some("Alice".to_owned()),
                avatar_mxc_uri: Some("mxc://example.test/alice".to_owned()),
            }],
            ..MatrixRoomListSnapshot::default()
        };

        let profiles = normalize_user_profiles(&snapshot);

        assert_eq!(
            profiles,
            vec![UserProfile {
                user_id: "@alice:example.test".to_owned(),
                display_name: Some("Alice".to_owned()),
                display_label: "Alice".to_owned(),
                original_display_label: "Alice".to_owned(),
                mention_search_terms: vec!["Alice".to_owned(), "@alice:example.test".to_owned(),],
                avatar: Some(AvatarImage {
                    mxc_uri: "mxc://example.test/alice".to_owned(),
                    thumbnail: AvatarThumbnailState::NotRequested,
                }),
            }]
        );
    }

    #[test]
    fn normalize_rooms_carries_sdk_room_tags() {
        let snapshot = MatrixRoomListSnapshot {
            spaces: vec![],
            rooms: vec![MatrixRoomListRoom {
                room_id: "!room1:example.test".to_owned(),
                display_name: "Room 1".to_owned(),
                avatar_mxc_uri: None,
                is_dm: false,
                dm_user_ids: Vec::new(),
                tags: MatrixRoomTags {
                    favourite: Some(MatrixRoomTagInfo {
                        order: Some("0.25".to_owned()),
                    }),
                    low_priority: None,
                },
                unread_count: 0,
                notification_count: 0,
                highlight_count: 0,
                marked_unread: false,
                recency_stamp: None,
                conversation_activity: None,
                latest_event: None,
                parent_space_ids: vec![],
                is_encrypted: false,
                joined_members: 0,
            }],
            invites: vec![],
            user_profiles: vec![],
        };

        let rooms = normalize_rooms(&snapshot);

        assert_eq!(
            rooms[0].tags.favourite,
            Some(RoomTagInfo {
                order: Some("0.25".to_owned())
            })
        );
        assert_eq!(rooms[0].tags.low_priority, None);
    }

    #[test]
    fn normalize_empty_snapshot() {
        let snapshot = MatrixRoomListSnapshot::default();
        assert!(normalize_spaces(&snapshot).is_empty());
        assert!(normalize_rooms(&snapshot).is_empty());
    }
}
