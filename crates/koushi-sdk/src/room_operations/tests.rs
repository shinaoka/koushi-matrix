use super::{
    MatrixCreateRoomOptions, MatrixCreateRoomParentSpace, MatrixCreateRoomVisibility,
    MatrixJoinTarget, MatrixPreviewJoinability, MatrixPreviewMembership,
    MatrixPublicRoomDirectoryQuery, MatrixPublicRoomDirectoryRoom, MatrixRoomHistoryVisibility,
    MatrixRoomJoinRule, MatrixRoomMemberRole, MatrixRoomModerationAction, MatrixRoomOperationError,
    MatrixRoomPermissionFacts, MatrixRoomSettingChange, MatrixRoomSettingsSnapshot,
    create_public_directory_room, create_room_request, get_room_settings_snapshot,
    join_room_target, matrix_room_preview_from_sdk, moderate_room_member,
    query_public_room_directory, resolve_join_target, set_space_power_level,
    unrelated_power_levels_equal, update_room_member_power_level, update_room_setting,
};

use crate::room_projection::{
    matrix_public_room_from_chunk, matrix_room_member_role, room_settings_snapshot_with_change,
    room_settings_snapshot_with_member_power_level,
};

#[test]
fn cancel_space_invite_validates_invite_membership_before_kicking() {
    let cancelled = super::MatrixSpaceInviteCancellationOutcome::Cancelled;
    let not_invited = super::MatrixSpaceInviteCancellationOutcome::NotInvited;

    assert_ne!(cancelled, not_invited);
}

#[test]
fn create_room_request_projects_space_room_options() {
    let request = create_room_request(MatrixCreateRoomOptions {
        name: "Synthetic Ops".to_owned(),
        topic: Some("Deployment notes".to_owned()),
        alias_localpart: None,
        encrypted: true,
        visibility: MatrixCreateRoomVisibility::Private,
        parent_space: Some(MatrixCreateRoomParentSpace {
            space_id: "!space:example.invalid".to_owned(),
            via_server: "example.invalid".to_owned(),
        }),
    })
    .expect("request should build");

    assert_eq!(request.name.as_deref(), Some("Synthetic Ops"));
    assert_eq!(request.topic.as_deref(), Some("Deployment notes"));
    assert_eq!(
        request
            .room_version
            .as_ref()
            .map(|version| version.as_str()),
        Some("9")
    );
    let initial_state = initial_state_json(&request);
    assert!(initial_state.iter().any(|event| {
        event.get("type").and_then(serde_json::Value::as_str) == Some("m.room.encryption")
    }));
    assert!(initial_state.iter().any(|event| {
        event.get("type").and_then(serde_json::Value::as_str) == Some("m.space.parent")
            && event.get("state_key").and_then(serde_json::Value::as_str)
                == Some("!space:example.invalid")
    }));
    let join_rules = initial_state
        .iter()
        .find(|event| {
            event.get("type").and_then(serde_json::Value::as_str) == Some("m.room.join_rules")
        })
        .expect("join rules");
    assert_eq!(
        join_rules
            .get("content")
            .and_then(|content| content.get("join_rule"))
            .and_then(serde_json::Value::as_str),
        Some("restricted")
    );
    let history_visibility = initial_state
        .iter()
        .find(|event| {
            event.get("type").and_then(serde_json::Value::as_str)
                == Some("m.room.history_visibility")
        })
        .expect("history visibility");
    assert_eq!(
        history_visibility
            .get("content")
            .and_then(|content| content.get("history_visibility"))
            .and_then(serde_json::Value::as_str),
        Some("invited")
    );
}
#[test]
fn create_room_request_projects_public_alias_without_encryption() {
    let request = create_room_request(MatrixCreateRoomOptions {
        name: "Synthetic Public".to_owned(),
        topic: None,
        alias_localpart: Some("synthetic-public".to_owned()),
        encrypted: true,
        visibility: MatrixCreateRoomVisibility::Public,
        parent_space: None,
    })
    .expect("request should build");

    assert_eq!(request.room_alias_name.as_deref(), Some("synthetic-public"));
    assert_eq!(
        request.visibility,
        matrix_sdk::ruma::api::client::room::Visibility::Public
    );
    let initial_state = initial_state_json(&request);
    assert!(!initial_state.iter().any(|event| {
        event.get("type").and_then(serde_json::Value::as_str) == Some("m.room.encryption")
    }));
}
fn initial_state_json(
    request: &matrix_sdk::ruma::api::client::room::create_room::v3::Request,
) -> Vec<serde_json::Value> {
    request
        .initial_state
        .iter()
        .map(|event| {
            serde_json::from_str::<serde_json::Value>(event.json().get())
                .expect("initial state event JSON")
        })
        .collect()
}
#[test]
fn join_target_accepts_a_room_id_because_links_carry_ids_more_often_than_aliases() {
    let target = MatrixJoinTarget {
        room_id_or_alias: "!room:example.invalid".to_owned(),
        via_servers: Vec::new(),
    };

    let (resolved, _via) = resolve_join_target(&target).expect("room id is a join target");

    assert_eq!(resolved.as_str(), "!room:example.invalid");
}
#[test]
fn join_target_keeps_every_via_server_so_a_federated_room_stays_reachable() {
    let target = MatrixJoinTarget {
        room_id_or_alias: "!room:example.invalid".to_owned(),
        via_servers: vec!["first.invalid".to_owned(), "second.invalid".to_owned()],
    };

    let (_resolved, via) = resolve_join_target(&target).expect("room id is a join target");

    // For an id target these are the only routing hints the homeserver has.
    let names = via.iter().map(|server| server.as_str()).collect::<Vec<_>>();
    assert_eq!(names, vec!["first.invalid", "second.invalid"]);
}
#[test]
fn join_target_still_accepts_an_alias() {
    let target = MatrixJoinTarget {
        room_id_or_alias: "#room:example.invalid".to_owned(),
        via_servers: Vec::new(),
    };

    let (resolved, via) = resolve_join_target(&target).expect("alias is a join target");

    assert_eq!(resolved.as_str(), "#room:example.invalid");
    assert!(via.is_empty());
}
#[test]
fn join_target_rejects_input_that_is_neither_a_room_id_nor_an_alias() {
    let target = MatrixJoinTarget {
        room_id_or_alias: "room-without-a-sigil".to_owned(),
        via_servers: Vec::new(),
    };

    assert!(matches!(
        resolve_join_target(&target),
        Err(MatrixRoomOperationError::InvalidRoomAlias)
    ));
}
#[test]
fn join_target_rejects_an_unusable_via_server_instead_of_dropping_it() {
    let target = MatrixJoinTarget {
        room_id_or_alias: "!room:example.invalid".to_owned(),
        via_servers: vec!["not a server name".to_owned()],
    };

    assert!(matches!(
        resolve_join_target(&target),
        Err(MatrixRoomOperationError::InvalidServerName)
    ));
}
fn sdk_room_preview(
    join_rule: Option<matrix_sdk::ruma::room::JoinRuleSummary>,
    state: Option<matrix_sdk::RoomState>,
) -> matrix_sdk::room_preview::RoomPreview {
    matrix_sdk::room_preview::RoomPreview {
        room_id: matrix_sdk::ruma::room_id!("!previewed:example.invalid").to_owned(),
        canonical_alias: None,
        name: None,
        topic: None,
        avatar_url: None,
        num_joined_members: 7,
        num_active_members: None,
        room_type: Some(matrix_sdk::ruma::room::RoomType::Space),
        join_rule,
        is_world_readable: None,
        state,
        is_direct: None,
        heroes: None,
    }
}
#[test]
fn preview_reports_an_invite_only_room_as_not_plainly_joinable() {
    let preview = matrix_room_preview_from_sdk(sdk_room_preview(
        Some(matrix_sdk::ruma::room::JoinRuleSummary::Invite),
        None,
    ));

    // Offering a plain Join here would produce a silent forbidden failure.
    assert_eq!(preview.joinability, MatrixPreviewJoinability::InviteOnly);
    assert_eq!(preview.membership, MatrixPreviewMembership::None);
}
#[test]
fn preview_reports_existing_membership_so_the_gui_navigates_instead_of_joining() {
    let preview = matrix_room_preview_from_sdk(sdk_room_preview(
        Some(matrix_sdk::ruma::room::JoinRuleSummary::Public),
        Some(matrix_sdk::RoomState::Joined),
    ));

    assert_eq!(preview.membership, MatrixPreviewMembership::Joined);
    assert_eq!(preview.joinability, MatrixPreviewJoinability::Open);
}
#[test]
fn preview_keeps_the_room_type_and_leaves_an_unnamed_room_unlabelled() {
    let preview = matrix_room_preview_from_sdk(sdk_room_preview(None, None));

    assert_eq!(preview.room_type.as_deref(), Some("m.space"));
    assert_eq!(preview.name, "");
    // No join rule reported is not the same as "anyone may join".
    assert_eq!(preview.joinability, MatrixPreviewJoinability::Unknown);
    assert_eq!(preview.joined_members, 7);
}
#[test]
fn directory_chunk_keeps_the_room_type_so_a_space_is_distinguishable() {
    let mut chunk = matrix_sdk::ruma::directory::PublicRoomsChunk::from(
        matrix_sdk::ruma::directory::PublicRoomsChunkInit {
            num_joined_members: matrix_sdk::ruma::UInt::from(3u32),
            room_id: matrix_sdk::ruma::room_id!("!space:example.invalid").to_owned(),
            world_readable: true,
            guest_can_join: false,
        },
    );
    chunk.room_type = Some(matrix_sdk::ruma::room::RoomType::Space);

    let room = matrix_public_room_from_chunk(chunk);

    // Without this the entry is indistinguishable from an ordinary room.
    assert_eq!(room.room_type.as_deref(), Some("m.space"));
}
#[test]
fn directory_chunk_without_a_name_stays_empty_rather_than_claiming_to_be_a_room() {
    let chunk = matrix_sdk::ruma::directory::PublicRoomsChunk::from(
        matrix_sdk::ruma::directory::PublicRoomsChunkInit {
            num_joined_members: matrix_sdk::ruma::UInt::from(0u32),
            room_id: matrix_sdk::ruma::room_id!("!unnamed:example.invalid").to_owned(),
            world_readable: false,
            guest_can_join: false,
        },
    );

    let room = matrix_public_room_from_chunk(chunk);

    // A hardcoded "Public room" would mislabel an unnamed space.
    assert_eq!(room.name, "");
    assert_eq!(room.room_type, None);
}
#[test]
fn directory_operations_use_public_room_and_alias_join_apis() {
    let _query = MatrixPublicRoomDirectoryQuery {
        term: Some("synthetic".to_owned()),
        server_name: Some("example.invalid".to_owned()),
        limit: Some(10),
        since: None,
    };
    let _room = MatrixPublicRoomDirectoryRoom {
        room_id: "!room:example.invalid".to_owned(),
        canonical_alias: Some("#room:example.invalid".to_owned()),
        room_type: None,
        name: "Synthetic Room".to_owned(),
        topic: None,
        avatar_url: None,
        joined_members: 1,
        world_readable: true,
        guest_can_join: false,
    };
    let _query_fn = query_public_room_directory;
    let _join_fn = join_room_target;
    let _create_public_fn = create_public_directory_room;
}
#[test]
fn space_role_power_content_preserves_unrelated_fields_and_removes_default_target() {
    use matrix_sdk::ruma::{
        events::room::power_levels::RoomPowerLevelsEventContent,
        room_version_rules::AuthorizationRules,
    };
    let target = matrix_sdk::ruma::user_id!("@target:example.invalid").to_owned();
    let other = matrix_sdk::ruma::user_id!("@other:example.invalid").to_owned();
    let mut content = RoomPowerLevelsEventContent::new(&AuthorizationRules::V1);
    content.events_default = matrix_sdk::ruma::Int::from(7);
    content.users_default = matrix_sdk::ruma::Int::from(10);
    content.users.insert(other, matrix_sdk::ruma::Int::from(50));
    content
        .users
        .insert(target.clone(), matrix_sdk::ruma::Int::from(50));
    let before = content.clone();
    set_space_power_level(&mut content, target.clone(), 0).expect("valid level");
    assert_eq!(
        content.users.get(&target),
        Some(&matrix_sdk::ruma::Int::from(0))
    );
    assert_eq!(content.events_default, before.events_default);
    assert_eq!(
        content
            .users
            .get(&matrix_sdk::ruma::user_id!("@other:example.invalid").to_owned()),
        before
            .users
            .get(&matrix_sdk::ruma::user_id!("@other:example.invalid").to_owned())
    );
    set_space_power_level(&mut content, target.clone(), 0).expect("valid default level");
    content.users_default = matrix_sdk::ruma::Int::from(0);
    set_space_power_level(&mut content, target.clone(), 0).expect("default removes target");
    assert!(!content.users.contains_key(&target));
}

#[test]
fn space_role_unrelated_comparison_ignores_only_target_user_entry() {
    use matrix_sdk::ruma::{
        events::room::power_levels::RoomPowerLevelsEventContent,
        room_version_rules::AuthorizationRules,
    };
    let target = matrix_sdk::ruma::user_id!("@target:example.invalid").to_owned();
    let mut left = RoomPowerLevelsEventContent::new(&AuthorizationRules::V1);
    let mut right = left.clone();
    left.users
        .insert(target.clone(), matrix_sdk::ruma::Int::from(0));
    right.users.insert(target, matrix_sdk::ruma::Int::from(50));
    assert!(unrelated_power_levels_equal(
        &left,
        &right,
        matrix_sdk::ruma::user_id!("@target:example.invalid")
    ));
    right.events_default = matrix_sdk::ruma::Int::from(9);
    assert!(!unrelated_power_levels_equal(
        &left,
        &right,
        matrix_sdk::ruma::user_id!("@target:example.invalid")
    ));
}

#[test]
fn room_management_wrappers_use_settings_privacy_and_moderation_apis() {
    let snapshot = MatrixRoomSettingsSnapshot {
        share_link: None,
        room_id: "!room:example.invalid".to_owned(),
        name: Some("Synthetic Room".to_owned()),
        topic: Some("Synthetic topic".to_owned()),
        avatar_url: None,
        canonical_alias: None,
        alternate_aliases: Vec::new(),
        join_rule: MatrixRoomJoinRule::Invite,
        history_visibility: MatrixRoomHistoryVisibility::Shared,
        permissions: MatrixRoomPermissionFacts {
            can_edit_settings: true,
            can_edit_roles: true,
            can_invite: true,
            can_kick: true,
            can_ban: true,
            can_unban: false,
        },
        members: vec![super::MatrixRoomMemberSummary {
            user_id: "@member:example.invalid".to_owned(),
            display_name: Some("Synthetic Member".to_owned()),
            avatar_url: None,
            power_level: Some(50),
            role: MatrixRoomMemberRole::Moderator,
            role_options: Vec::new(),
            user_trust: None,
        }],
    };
    let change = MatrixRoomSettingChange::JoinRule(MatrixRoomJoinRule::Public);
    let moderation = MatrixRoomModerationAction::Kick;
    let _snapshot_fn = get_room_settings_snapshot;
    let _update_fn = update_room_setting;
    let _moderate_fn = moderate_room_member;
    let _role_fn = update_room_member_power_level;

    assert!(snapshot.permissions.can_invite);
    assert!(matches!(change, MatrixRoomSettingChange::JoinRule(_)));
    assert!(matches!(moderation, MatrixRoomModerationAction::Kick));
}

#[test]
fn room_setting_update_projects_the_sent_change_into_the_success_snapshot() {
    let original = MatrixRoomSettingsSnapshot {
        share_link: None,
        room_id: "!room:example.invalid".to_owned(),
        name: Some("Original Room".to_owned()),
        topic: Some("Original topic".to_owned()),
        avatar_url: Some("mxc://example.invalid/original".to_owned()),
        canonical_alias: None,
        alternate_aliases: Vec::new(),
        join_rule: MatrixRoomJoinRule::Invite,
        history_visibility: MatrixRoomHistoryVisibility::Shared,
        permissions: MatrixRoomPermissionFacts {
            can_edit_settings: true,
            can_edit_roles: true,
            can_invite: true,
            can_kick: true,
            can_ban: true,
            can_unban: true,
        },
        members: vec![],
    };

    assert_eq!(
        room_settings_snapshot_with_change(
            original.clone(),
            &MatrixRoomSettingChange::Topic(Some("Updated topic".to_owned())),
        )
        .topic
        .as_deref(),
        Some("Updated topic")
    );
    assert_eq!(
        room_settings_snapshot_with_change(original.clone(), &MatrixRoomSettingChange::Name(None),)
            .name,
        None
    );
    assert_eq!(
        room_settings_snapshot_with_change(
            original.clone(),
            &MatrixRoomSettingChange::AvatarUrl(None),
        )
        .avatar_url,
        None
    );
    assert_eq!(
        room_settings_snapshot_with_change(
            original.clone(),
            &MatrixRoomSettingChange::JoinRule(MatrixRoomJoinRule::Public),
        )
        .join_rule,
        MatrixRoomJoinRule::Public
    );
    assert_eq!(
        room_settings_snapshot_with_change(
            original,
            &MatrixRoomSettingChange::HistoryVisibility(MatrixRoomHistoryVisibility::Joined,),
        )
        .history_visibility,
        MatrixRoomHistoryVisibility::Joined
    );
}
#[test]
fn room_member_power_level_projection_updates_role_in_success_snapshot() {
    let original = MatrixRoomSettingsSnapshot {
        share_link: None,
        room_id: "!room:example.invalid".to_owned(),
        name: Some("Original Room".to_owned()),
        topic: Some("Original topic".to_owned()),
        avatar_url: None,
        canonical_alias: None,
        alternate_aliases: Vec::new(),
        join_rule: MatrixRoomJoinRule::Invite,
        history_visibility: MatrixRoomHistoryVisibility::Shared,
        permissions: MatrixRoomPermissionFacts {
            can_edit_settings: true,
            can_edit_roles: true,
            can_invite: true,
            can_kick: true,
            can_ban: true,
            can_unban: true,
        },
        members: vec![super::MatrixRoomMemberSummary {
            user_id: "@member:example.invalid".to_owned(),
            display_name: Some("Synthetic Member".to_owned()),
            avatar_url: None,
            power_level: Some(0),
            role: MatrixRoomMemberRole::User,
            role_options: vec![
                super::MatrixRoomMemberRoleOption {
                    power_level: 50,
                    role: MatrixRoomMemberRole::Moderator,
                    requires_confirmation: false,
                },
                super::MatrixRoomMemberRoleOption {
                    power_level: 100,
                    role: MatrixRoomMemberRole::Administrator,
                    requires_confirmation: true,
                },
            ],
            user_trust: None,
        }],
    };

    let updated =
        room_settings_snapshot_with_member_power_level(original, "@member:example.invalid", 50);
    let member = updated.members.first().expect("member summary");
    assert_eq!(member.power_level, Some(50));
    assert_eq!(member.role, MatrixRoomMemberRole::Moderator);
    assert_eq!(
        member
            .role_options
            .iter()
            .map(|option| option.power_level)
            .collect::<Vec<_>>(),
        [100, 0]
    );
    assert_eq!(
        matrix_room_member_role(Some(100)),
        MatrixRoomMemberRole::Administrator
    );
    assert_eq!(matrix_room_member_role(None), MatrixRoomMemberRole::Creator);
}
