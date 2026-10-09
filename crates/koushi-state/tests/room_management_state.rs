use koushi_state::{
    AppAction, AppEffect, AppState, OperationFailureKind, OwnProfile, ProfileUpdateRequest,
    RoomHistoryVisibility, RoomJoinRule, RoomManagementOperationKind, RoomManagementOperationState,
    RoomManagementState, RoomMemberRole, RoomMemberSummary, RoomModerationAction,
    RoomPermissionFacts, RoomSettingChange, RoomSettingsSnapshot, SessionInfo, SessionState,
    UiEvent, UserProfile, reduce,
};
use std::collections::BTreeMap;

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.org".to_owned(),
        user_id: "@user-a:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    }
}

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(session_info()),
        ..AppState::default()
    }
}

#[test]
fn room_permission_facts_round_trip_invite_permission_with_snake_case() {
    let facts: RoomPermissionFacts = serde_json::from_value(serde_json::json!({
        "can_edit_settings": false,
        "can_edit_roles": false,
        "can_kick": false,
        "can_ban": false,
        "can_unban": false,
        "can_invite": true,
    }))
    .expect("room permission facts");

    assert_eq!(
        serde_json::to_value(facts).expect("serialize room permission facts")["can_invite"],
        serde_json::json!(true)
    );

    let legacy_facts: RoomPermissionFacts = serde_json::from_value(serde_json::json!({
        "can_edit_settings": false,
        "can_edit_roles": false,
        "can_kick": false,
        "can_ban": false,
        "can_unban": false,
    }))
    .expect("legacy room permission facts");
    assert_eq!(
        serde_json::to_value(legacy_facts).expect("serialize legacy room permission facts")["can_invite"],
        serde_json::json!(false)
    );
}

fn editable_settings(room_id: &str) -> RoomSettingsSnapshot {
    RoomSettingsSnapshot {
        room_id: room_id.to_owned(),
        name: Some("Synthetic Room".to_owned()),
        topic: Some("Synthetic topic".to_owned()),
        avatar_url: Some("mxc://example.invalid/avatar".to_owned()),
        canonical_alias: None,
        alternate_aliases: Vec::new(),
        share_link: None,
        join_rule: RoomJoinRule::Invite,
        history_visibility: RoomHistoryVisibility::Shared,
        access: koushi_state::RoomAccessCondition::default(),
        permissions: RoomPermissionFacts {
            can_edit_settings: true,
            can_change_join_rule: true,
            can_edit_roles: true,
            can_invite: true,
            can_kick: true,
            can_ban: true,
            can_unban: false,
        },
        members: vec![
            RoomMemberSummary {
                membership: koushi_state::RoomMemberMembership::Joined,
                user_id: "@user-a:example.invalid".to_owned(),
                display_name: Some("User A".to_owned()),
                display_label: "User A".to_owned(),
                original_display_label: "User A".to_owned(),
                avatar_url: None,
                power_level: Some(100),
                role: RoomMemberRole::Administrator,
                role_options: Vec::new(),
                user_trust: None,
            },
            RoomMemberSummary {
                membership: koushi_state::RoomMemberMembership::Joined,
                user_id: "@target:example.invalid".to_owned(),
                display_name: Some("Target".to_owned()),
                display_label: "Target".to_owned(),
                original_display_label: "Target".to_owned(),
                avatar_url: Some("mxc://example.invalid/target-avatar".to_owned()),
                power_level: Some(0),
                role: RoomMemberRole::User,
                role_options: Vec::new(),
                user_trust: None,
            },
        ],
    }
}

fn locked_settings(room_id: &str) -> RoomSettingsSnapshot {
    RoomSettingsSnapshot {
        permissions: RoomPermissionFacts::default(),
        ..editable_settings(room_id)
    }
}

fn serialized_member_display_label(state: &AppState, user_id: &str) -> Option<String> {
    let settings = state.room_management.settings.as_ref()?;
    let value = serde_json::to_value(settings).expect("serialize room settings");
    value["members"]
        .as_array()
        .expect("members array")
        .iter()
        .find(|member| member["user_id"] == user_id)
        .and_then(|member| member["display_label"].as_str())
        .map(str::to_owned)
}

fn serialized_member_original_display_label(state: &AppState, user_id: &str) -> Option<String> {
    let settings = state.room_management.settings.as_ref()?;
    let value = serde_json::to_value(settings).expect("serialize room settings");
    value["members"]
        .as_array()
        .expect("members array")
        .iter()
        .find(|member| member["user_id"] == user_id)
        .and_then(|member| member["original_display_label"].as_str())
        .map(str::to_owned)
}

#[test]
fn room_settings_snapshot_load_projects_member_display_labels_from_aliases() {
    let mut state = ready_state();
    let room_id = "!room:example.invalid";

    reduce(
        &mut state,
        AppAction::LocalUserAliasesLoaded {
            aliases: BTreeMap::from([(
                "@target:example.invalid".to_owned(),
                "Target Local".to_owned(),
            )]),
        },
    );
    reduce(
        &mut state,
        AppAction::RoomSettingsSnapshotLoaded {
            room_id: room_id.to_owned(),
            settings: editable_settings(room_id),
        },
    );

    assert_eq!(
        serialized_member_display_label(&state, "@target:example.invalid"),
        Some("Target Local".to_owned())
    );
    assert_eq!(
        serialized_member_original_display_label(&state, "@target:example.invalid"),
        Some("Target".to_owned())
    );
    assert_eq!(
        state
            .room_management
            .settings
            .as_ref()
            .and_then(|settings| settings
                .members
                .iter()
                .find(|member| member.user_id == "@target:example.invalid"))
            .and_then(|member| member.display_name.as_deref()),
        Some("Target")
    );
}

#[test]
fn room_settings_member_display_label_uses_profile_cache_when_room_name_is_blank() {
    let mut state = ready_state();
    let room_id = "!room:example.invalid";
    let mut settings = editable_settings(room_id);
    let target = settings
        .members
        .iter_mut()
        .find(|member| member.user_id == "@target:example.invalid")
        .expect("target member");
    target.display_name = Some("   ".to_owned());
    target.display_label = "@target:example.invalid".to_owned();

    reduce(
        &mut state,
        AppAction::UserProfilesUpdated {
            profiles: vec![UserProfile {
                user_id: "@target:example.invalid".to_owned(),
                display_name: Some("Profile Cache Name".to_owned()),
                display_label: String::new(),
                original_display_label: String::new(),
                mention_search_terms: Vec::new(),
                avatar: None,
            }],
        },
    );
    reduce(
        &mut state,
        AppAction::RoomSettingsSnapshotLoaded {
            room_id: room_id.to_owned(),
            settings,
        },
    );

    assert_eq!(
        serialized_member_display_label(&state, "@target:example.invalid"),
        Some("Profile Cache Name".to_owned())
    );
    assert_eq!(
        serialized_member_original_display_label(&state, "@target:example.invalid"),
        Some("Profile Cache Name".to_owned())
    );
    assert_eq!(
        state
            .room_management
            .settings
            .as_ref()
            .and_then(|settings| settings
                .members
                .iter()
                .find(|member| member.user_id == "@target:example.invalid"))
            .and_then(|member| member.display_name.as_deref()),
        Some("   ")
    );
}

#[test]
fn local_alias_update_refreshes_open_room_member_display_labels() {
    let mut state = ready_state();
    let room_id = "!room:example.invalid";
    reduce(
        &mut state,
        AppAction::RoomSettingsSnapshotLoaded {
            room_id: room_id.to_owned(),
            settings: editable_settings(room_id),
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::LocalUserAliasUpdateRequested {
            request_id: 52,
            user_id: "@target:example.invalid".to_owned(),
            alias: Some("Target Local".to_owned()),
        },
    );

    assert_eq!(
        serialized_member_display_label(&state, "@target:example.invalid"),
        Some("Target Local".to_owned())
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::ProfileChanged(
                koushi_state::ProfileDisplayChange {
                    user_ids: vec!["@target:example.invalid".to_owned()]
                }
            )),
            AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged),
        ]
    );
}

#[test]
fn own_profile_update_success_refreshes_open_room_member_display_labels() {
    let mut state = ready_state();
    let room_id = "!room:example.invalid";
    let mut settings = editable_settings(room_id);
    let own_member = settings
        .members
        .iter_mut()
        .find(|member| member.user_id == "@user-a:example.invalid")
        .expect("own member");
    own_member.display_name = None;
    own_member.display_label = "@user-a:example.invalid".to_owned();

    reduce(
        &mut state,
        AppAction::RoomSettingsSnapshotLoaded {
            room_id: room_id.to_owned(),
            settings,
        },
    );
    reduce(
        &mut state,
        AppAction::ProfileUpdateRequested {
            request_id: 53,
            request: ProfileUpdateRequest::SetDisplayName {
                display_name: Some("Own Local Name".to_owned()),
            },
        },
    );
    let effects = reduce(
        &mut state,
        AppAction::ProfileUpdateSucceeded {
            request_id: 53,
            profile: OwnProfile {
                display_name: Some("Own Local Name".to_owned()),
                avatar: None,
            },
        },
    );

    assert_eq!(
        serialized_member_display_label(&state, "@user-a:example.invalid"),
        Some("Own Local Name".to_owned())
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::ProfileChanged(
                koushi_state::ProfileDisplayChange {
                    user_ids: vec!["@user-a:example.invalid".to_owned()]
                }
            )),
            AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged),
        ]
    );
}

#[test]
fn room_management_debug_output_redacts_private_values() {
    let settings = RoomSettingsSnapshot {
        room_id: "!private-room:example.invalid".to_owned(),
        name: Some("Private Room Name".to_owned()),
        topic: Some("Private room topic".to_owned()),
        avatar_url: Some("mxc://example.invalid/private-avatar".to_owned()),
        ..editable_settings("!private-room:example.invalid")
    };

    let debug_values = [
        format!("{settings:?}"),
        format!(
            "{:?}",
            RoomManagementState {
                selected_room_id: Some("!private-room:example.invalid".to_owned()),
                settings: Some(settings.clone()),
                draft: None,
                directory: koushi_state::RoomDirectoryVisibility::Loading,
                active_room_editor: None,
                active_create_session: None,
                operation: RoomManagementOperationState::Pending {
                    request_id: 30,
                    room_id: "!private-room:example.invalid".to_owned(),
                    operation: RoomManagementOperationKind::Settings,
                },
            }
        ),
        format!(
            "{:?}",
            RoomSettingChange::Topic(Some("Private updated topic".to_owned()))
        ),
        format!(
            "{:?}",
            AppAction::RoomSettingsSnapshotLoaded {
                room_id: "!private-room:example.invalid".to_owned(),
                settings: settings.clone(),
            }
        ),
        format!(
            "{:?}",
            AppAction::RoomSettingUpdateRequested {
                request_id: 31,
                room_id: "!private-room:example.invalid".to_owned(),
                change: RoomSettingChange::Name(Some("Private updated name".to_owned())),
            }
        ),
        format!(
            "{:?}",
            AppAction::RoomModerationRequested {
                request_id: 32,
                room_id: "!private-room:example.invalid".to_owned(),
                target_user_id: "@private-target:example.invalid".to_owned(),
                action: RoomModerationAction::Ban,
                reason: Some("Private moderation reason".to_owned()),
            }
        ),
        format!(
            "{:?}",
            AppAction::RoomMemberRoleUpdateRequested {
                request_id: 33,
                room_id: "!private-room:example.invalid".to_owned(),
                target_user_id: "@private-target:example.invalid".to_owned(),
                power_level: 50,
            }
        ),
    ];

    for debug in debug_values {
        for private_value in [
            "!private-room:example.invalid",
            "Private Room Name",
            "Private room topic",
            "mxc://example.invalid/private-avatar",
            "Private updated topic",
            "Private updated name",
            "Target",
            "mxc://example.invalid/target-avatar",
            "@private-target:example.invalid",
            "Private moderation reason",
        ] {
            assert!(
                !debug.contains(private_value),
                "debug leaked {private_value}: {debug}"
            );
        }
    }
}

#[test]
fn room_member_role_update_records_pending_and_matching_completion_updates_member_snapshot() {
    let mut state = ready_state();
    let room_id = "!room:example.invalid";
    reduce(
        &mut state,
        AppAction::RoomSettingsSnapshotLoaded {
            room_id: room_id.to_owned(),
            settings: editable_settings(room_id),
        },
    );

    reduce(
        &mut state,
        AppAction::RoomMemberRoleUpdateRequested {
            request_id: 41,
            room_id: room_id.to_owned(),
            target_user_id: "@target:example.invalid".to_owned(),
            power_level: 50,
        },
    );

    assert_eq!(
        state.room_management.operation,
        RoomManagementOperationState::Pending {
            request_id: 41,
            room_id: room_id.to_owned(),
            operation: RoomManagementOperationKind::Roles,
        }
    );

    reduce(
        &mut state,
        AppAction::RoomMemberRoleUpdateSucceeded {
            request_id: 41,
            room_id: room_id.to_owned(),
            target_user_id: "@target:example.invalid".to_owned(),
            power_level: 50,
        },
    );

    let settings = state
        .room_management
        .settings
        .expect("room management settings");
    let target = settings
        .members
        .iter()
        .find(|member| member.user_id == "@target:example.invalid")
        .expect("target member");
    assert_eq!(target.power_level, Some(50));
    assert_eq!(target.role, RoomMemberRole::Moderator);
    assert_eq!(
        state.room_management.operation,
        RoomManagementOperationState::Idle
    );
}

#[test]
fn room_member_role_update_without_permission_is_rejected_in_rust_state() {
    let mut state = ready_state();
    let room_id = "!room:example.invalid";
    reduce(
        &mut state,
        AppAction::RoomSettingsSnapshotLoaded {
            room_id: room_id.to_owned(),
            settings: locked_settings(room_id),
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::RoomMemberRoleUpdateRequested {
            request_id: 42,
            room_id: room_id.to_owned(),
            target_user_id: "@target:example.invalid".to_owned(),
            power_level: 50,
        },
    );

    assert_eq!(
        state.room_management.operation,
        RoomManagementOperationState::Failed {
            request_id: 42,
            room_id: room_id.to_owned(),
            operation: RoomManagementOperationKind::Roles,
            kind: OperationFailureKind::Forbidden,
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
    );
}

#[test]
fn room_settings_snapshot_replaces_existing_room_management_state() {
    let mut state = ready_state();
    state.room_management = RoomManagementState {
        selected_room_id: Some("!old:example.invalid".to_owned()),
        settings: Some(editable_settings("!old:example.invalid")),
        draft: None,
        directory: koushi_state::RoomDirectoryVisibility::Loading,
        active_room_editor: None,
        active_create_session: None,
        operation: RoomManagementOperationState::Pending {
            request_id: 1,
            room_id: "!old:example.invalid".to_owned(),
            operation: RoomManagementOperationKind::Settings,
        },
    };

    let effects = reduce(
        &mut state,
        AppAction::RoomSettingsSnapshotLoaded {
            room_id: "!new:example.invalid".to_owned(),
            settings: editable_settings("!new:example.invalid"),
        },
    );

    assert_eq!(
        state.room_management,
        RoomManagementState {
            selected_room_id: Some("!new:example.invalid".to_owned()),
            settings: Some(editable_settings("!new:example.invalid")),
            draft: None,
            directory: koushi_state::RoomDirectoryVisibility::Loading,
            active_room_editor: Some("!new:example.invalid".to_owned()),
            active_create_session: None,
            operation: RoomManagementOperationState::Idle,
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
    );
}

#[test]
fn room_settings_snapshot_preserves_same_room_pending_operation() {
    let mut state = ready_state();
    let room_id = "!room:example.invalid";
    state.room_management = RoomManagementState {
        selected_room_id: Some(room_id.to_owned()),
        settings: Some(editable_settings(room_id)),
        draft: None,
        directory: koushi_state::RoomDirectoryVisibility::Loading,
        active_room_editor: None,
        active_create_session: None,
        operation: RoomManagementOperationState::Pending {
            request_id: 7,
            room_id: room_id.to_owned(),
            operation: RoomManagementOperationKind::Settings,
        },
    };

    let effects = reduce(
        &mut state,
        AppAction::RoomSettingsSnapshotLoaded {
            room_id: room_id.to_owned(),
            settings: RoomSettingsSnapshot {
                topic: Some("Fresh synthetic topic".to_owned()),
                ..editable_settings(room_id)
            },
        },
    );

    assert_eq!(
        state.room_management.settings,
        Some(RoomSettingsSnapshot {
            topic: Some("Fresh synthetic topic".to_owned()),
            ..editable_settings(room_id)
        })
    );
    assert_eq!(
        state.room_management.operation,
        RoomManagementOperationState::Pending {
            request_id: 7,
            room_id: room_id.to_owned(),
            operation: RoomManagementOperationKind::Settings,
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
    );
}

#[test]
fn room_setting_update_records_pending_and_matching_completion_clears_it() {
    let mut state = ready_state();
    let room_id = "!room:example.invalid";
    reduce(
        &mut state,
        AppAction::RoomSettingsSnapshotLoaded {
            room_id: room_id.to_owned(),
            settings: editable_settings(room_id),
        },
    );

    reduce(
        &mut state,
        AppAction::RoomSettingUpdateRequested {
            request_id: 7,
            room_id: room_id.to_owned(),
            change: RoomSettingChange::Topic(Some("New synthetic topic".to_owned())),
        },
    );

    assert_eq!(
        state.room_management.operation,
        RoomManagementOperationState::Pending {
            request_id: 7,
            room_id: room_id.to_owned(),
            operation: RoomManagementOperationKind::Settings,
        }
    );

    reduce(
        &mut state,
        AppAction::RoomSettingUpdateSucceeded {
            request_id: 7,
            room_id: room_id.to_owned(),
            change: RoomSettingChange::Topic(Some("New synthetic topic".to_owned())),
            settings: RoomSettingsSnapshot {
                topic: Some("New synthetic topic".to_owned()),
                ..editable_settings(room_id)
            },
        },
    );

    assert_eq!(
        state.room_management.settings,
        Some(RoomSettingsSnapshot {
            topic: Some("New synthetic topic".to_owned()),
            ..editable_settings(room_id)
        })
    );
    assert_eq!(
        state.room_management.operation,
        RoomManagementOperationState::Idle
    );
}

/// #1220: the event-specific fact admits a join-rule change on its own, and the
/// one `allows_setting_change` guard stays the authority for every change.
#[test]
fn join_rule_change_needs_only_the_join_rule_permission() {
    let mut state = ready_state();
    let room_id = "!room:example.invalid";
    let mut settings = editable_settings(room_id);
    settings.permissions.can_edit_settings = false;
    settings.permissions.can_change_join_rule = true;
    reduce(
        &mut state,
        AppAction::RoomSettingsSnapshotLoaded {
            room_id: room_id.to_owned(),
            settings,
        },
    );

    reduce(
        &mut state,
        AppAction::RoomSettingUpdateRequested {
            request_id: 11,
            room_id: room_id.to_owned(),
            change: RoomSettingChange::JoinRule(RoomJoinRule::Public),
        },
    );
    assert_eq!(
        state.room_management.operation,
        RoomManagementOperationState::Pending {
            request_id: 11,
            room_id: room_id.to_owned(),
            operation: RoomManagementOperationKind::Settings,
        },
        "an account that may change join rules but not rename the room can change the join rule"
    );

    // A name change still needs the aggregate and is rejected before mutation.
    reduce(
        &mut state,
        AppAction::RoomSettingUpdateRequested {
            request_id: 12,
            room_id: room_id.to_owned(),
            change: RoomSettingChange::Name(Some("New synthetic name".to_owned())),
        },
    );
    assert_eq!(
        state.room_management.operation,
        RoomManagementOperationState::Failed {
            request_id: 12,
            room_id: room_id.to_owned(),
            operation: RoomManagementOperationKind::Settings,
            kind: OperationFailureKind::Forbidden,
        }
    );
}

#[test]
fn stale_room_management_completion_is_ignored() {
    let mut state = ready_state();
    let room_id = "!room:example.invalid";
    reduce(
        &mut state,
        AppAction::RoomSettingsSnapshotLoaded {
            room_id: room_id.to_owned(),
            settings: editable_settings(room_id),
        },
    );
    reduce(
        &mut state,
        AppAction::RoomSettingUpdateRequested {
            request_id: 11,
            room_id: room_id.to_owned(),
            change: RoomSettingChange::Name(Some("Fresh name".to_owned())),
        },
    );

    assert_eq!(
        reduce(
            &mut state,
            AppAction::RoomSettingUpdateSucceeded {
                request_id: 12,
                room_id: room_id.to_owned(),
                change: RoomSettingChange::Name(Some("Fresh name".to_owned())),
                settings: editable_settings(room_id),
            },
        ),
        Vec::new()
    );

    assert_eq!(
        state.room_management.operation,
        RoomManagementOperationState::Pending {
            request_id: 11,
            room_id: room_id.to_owned(),
            operation: RoomManagementOperationKind::Settings,
        }
    );
}

#[test]
fn moderation_command_without_permission_is_rejected_in_rust_state() {
    let mut state = ready_state();
    let room_id = "!room:example.invalid";
    reduce(
        &mut state,
        AppAction::RoomSettingsSnapshotLoaded {
            room_id: room_id.to_owned(),
            settings: locked_settings(room_id),
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::RoomModerationRequested {
            request_id: 13,
            room_id: room_id.to_owned(),
            target_user_id: "@target:example.invalid".to_owned(),
            action: RoomModerationAction::Kick,
            reason: Some("Synthetic reason".to_owned()),
        },
    );

    assert_eq!(
        state.room_management.operation,
        RoomManagementOperationState::Failed {
            request_id: 13,
            room_id: room_id.to_owned(),
            operation: RoomManagementOperationKind::Moderation,
            kind: OperationFailureKind::Forbidden,
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
    );
}

#[test]
fn successful_kick_removes_target_from_room_scoped_member_snapshot() {
    let mut state = ready_state();
    let room_id = "!room:example.invalid";
    reduce(
        &mut state,
        AppAction::RoomSettingsSnapshotLoaded {
            room_id: room_id.to_owned(),
            settings: editable_settings(room_id),
        },
    );

    reduce(
        &mut state,
        AppAction::RoomModerationRequested {
            request_id: 21,
            room_id: room_id.to_owned(),
            target_user_id: "@target:example.invalid".to_owned(),
            action: RoomModerationAction::Kick,
            reason: None,
        },
    );

    reduce(
        &mut state,
        AppAction::RoomModerationSucceeded {
            request_id: 21,
            room_id: room_id.to_owned(),
            target_user_id: "@target:example.invalid".to_owned(),
            action: RoomModerationAction::Kick,
        },
    );

    let settings = state
        .room_management
        .settings
        .expect("room management settings");
    assert_eq!(
        settings
            .members
            .iter()
            .map(|member| member.user_id.as_str())
            .collect::<Vec<_>>(),
        vec!["@user-a:example.invalid"]
    );
    assert_eq!(
        state.room_management.operation,
        RoomManagementOperationState::Idle
    );
}

#[test]
fn room_management_logout_clears_state() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomSettingsSnapshotLoaded {
            room_id: "!room:example.invalid".to_owned(),
            settings: editable_settings("!room:example.invalid"),
        },
    );

    reduce(&mut state, AppAction::LogoutFinished);

    assert_eq!(state.room_management, RoomManagementState::default());
}

#[test]
fn avatar_metadata_debug_redacts_mxc_and_user_room_associations() {
    let member = RoomMemberSummary {
        membership: koushi_state::RoomMemberMembership::Joined,
        user_id: "@member:example.invalid".to_owned(),
        display_name: Some("Member".to_owned()),
        display_label: "Member".to_owned(),
        original_display_label: "Member".to_owned(),
        avatar_url: Some("mxc://example.invalid/member-avatar".to_owned()),
        power_level: Some(0),
        role: RoomMemberRole::User,
        role_options: Vec::new(),
        user_trust: None,
    };
    let member_debug = format!("{:?}", member);
    assert!(
        !member_debug.contains("mxc://example.invalid/member-avatar"),
        "{member_debug}"
    );
    assert!(member_debug.contains("MxcUri(..)"), "{member_debug}");

    let settings = RoomSettingsSnapshot {
        room_id: "!room:example.invalid".to_owned(),
        name: Some("Synthetic Room".to_owned()),
        topic: Some("Synthetic topic".to_owned()),
        avatar_url: Some("mxc://example.invalid/room-avatar".to_owned()),
        canonical_alias: None,
        alternate_aliases: Vec::new(),
        share_link: None,
        join_rule: RoomJoinRule::Invite,
        history_visibility: RoomHistoryVisibility::Shared,
        access: koushi_state::RoomAccessCondition::default(),
        permissions: RoomPermissionFacts::default(),
        members: vec![member],
    };
    let settings_debug = format!("{:?}", settings);
    assert!(
        !settings_debug.contains("mxc://example.invalid/room-avatar"),
        "{settings_debug}"
    );
    assert!(settings_debug.contains("MxcUri(..)"), "{settings_debug}");
}

fn settings_with_access(
    room_id: &str,
    access: koushi_state::RoomAccessCondition,
) -> RoomSettingsSnapshot {
    RoomSettingsSnapshot {
        access,
        ..editable_settings(room_id)
    }
}

fn restricted_access(
    completeness: koushi_state::RestrictedConditions,
) -> koushi_state::RoomAccessCondition {
    koushi_state::RoomAccessCondition {
        join_rule: Some(RoomJoinRule::Restricted),
        restricted: Some(completeness),
        allow_targets: vec![koushi_state::RoomAllowTarget {
            kind: koushi_state::RoomAllowTargetKind::Space,
            room_id: "!space:example.invalid".to_owned(),
        }],
    }
}

fn access_policy() -> RoomSettingChange {
    RoomSettingChange::AccessPolicy(koushi_state::RoomAccessPolicy::new(
        RoomJoinRule::Restricted,
        vec!["!space:example.invalid".to_owned()],
    ))
}

#[test]
fn access_policy_edit_is_rejected_with_the_unsupported_condition_kind() {
    let mut state = ready_state();
    state.room_management = RoomManagementState {
        selected_room_id: Some("!room:example.invalid".to_owned()),
        settings: Some(settings_with_access(
            "!room:example.invalid",
            restricted_access(koushi_state::RestrictedConditions::MembershipPlusUnsupported),
        )),
        draft: None,
        directory: koushi_state::RoomDirectoryVisibility::Loading,
        active_room_editor: None,
        active_create_session: None,
        operation: RoomManagementOperationState::Idle,
    };

    let effects = reduce(
        &mut state,
        AppAction::RoomSettingUpdateRequested {
            request_id: 7,
            room_id: "!room:example.invalid".to_owned(),
            change: access_policy(),
        },
    );

    assert_eq!(
        state.room_management.operation,
        RoomManagementOperationState::Failed {
            request_id: 7,
            room_id: "!room:example.invalid".to_owned(),
            operation: RoomManagementOperationKind::Settings,
            kind: OperationFailureKind::UnsupportedPolicyCondition,
        }
    );
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)]
    );
}

#[test]
fn access_policy_edit_is_rejected_when_the_current_policy_is_unverified() {
    let mut state = ready_state();
    state.room_management = RoomManagementState {
        selected_room_id: Some("!room:example.invalid".to_owned()),
        settings: Some(settings_with_access(
            "!room:example.invalid",
            koushi_state::RoomAccessCondition {
                join_rule: None,
                restricted: Some(koushi_state::RestrictedConditions::NotInspected),
                allow_targets: Vec::new(),
            },
        )),
        draft: None,
        directory: koushi_state::RoomDirectoryVisibility::Loading,
        active_room_editor: None,
        active_create_session: None,
        operation: RoomManagementOperationState::Idle,
    };

    reduce(
        &mut state,
        AppAction::RoomSettingUpdateRequested {
            request_id: 8,
            room_id: "!room:example.invalid".to_owned(),
            change: access_policy(),
        },
    );

    assert_eq!(
        state.room_management.operation,
        RoomManagementOperationState::Failed {
            request_id: 8,
            room_id: "!room:example.invalid".to_owned(),
            operation: RoomManagementOperationKind::Settings,
            kind: OperationFailureKind::PolicyNotVerified,
        }
    );
}

#[test]
fn access_policy_edit_is_admitted_for_verified_membership_only_content() {
    let mut state = ready_state();
    state.room_management = RoomManagementState {
        selected_room_id: Some("!room:example.invalid".to_owned()),
        settings: Some(settings_with_access(
            "!room:example.invalid",
            restricted_access(koushi_state::RestrictedConditions::MembershipOnly),
        )),
        draft: None,
        directory: koushi_state::RoomDirectoryVisibility::Loading,
        active_room_editor: None,
        active_create_session: None,
        operation: RoomManagementOperationState::Idle,
    };

    reduce(
        &mut state,
        AppAction::RoomSettingUpdateRequested {
            request_id: 9,
            room_id: "!room:example.invalid".to_owned(),
            change: access_policy(),
        },
    );

    assert_eq!(
        state.room_management.operation,
        RoomManagementOperationState::Pending {
            request_id: 9,
            room_id: "!room:example.invalid".to_owned(),
            operation: RoomManagementOperationKind::Settings,
        }
    );
}

#[test]
fn a_pre_send_rejection_is_admitted_against_the_raw_read_not_the_preserved_value() {
    let room = "!room:example.invalid";
    let mut state = ready_state();
    // A local access save was accepted and its echo has not landed.
    let accepted = restricted_access(koushi_state::RestrictedConditions::MembershipOnly);
    state.room_access.insert(room.to_owned(), accepted.clone());
    state.room_access_observed.insert(
        room.to_owned(),
        koushi_state::RoomAccessObservation {
            access: koushi_state::RoomAccessCondition {
                join_rule: Some(RoomJoinRule::Invite),
                restricted: None,
                allow_targets: Vec::new(),
            },
            history_visibility: RoomHistoryVisibility::Shared,
        },
    );
    state.room_management = RoomManagementState {
        selected_room_id: Some(room.to_owned()),
        settings: Some(settings_with_access(room, accepted.clone())),
        ..RoomManagementState::default()
    };
    state.spaces = vec![joined_space("!space:example.invalid")];

    // The pre-send read still carries the RAW current policy, whose condition is
    // mixed-unsupported. The overlay keeps the accepted value for display.
    reduce(
        &mut state,
        AppAction::RoomSettingsSnapshotLoaded {
            room_id: room.to_owned(),
            settings: settings_with_access(
                room,
                restricted_access(koushi_state::RestrictedConditions::MembershipPlusUnsupported),
            ),
        },
    );
    assert_eq!(
        state
            .room_management
            .settings
            .as_ref()
            .expect("settings")
            .access,
        accepted,
        "the accepted local value stays displayed"
    );

    // Admission uses the raw read's facts, so the edit is rejected rather than
    // left pending against the presentation-preserved value.
    reduce(
        &mut state,
        AppAction::RoomSettingUpdateRequested {
            request_id: 5,
            room_id: room.to_owned(),
            change: access_policy(),
        },
    );
    assert_eq!(
        state.room_management.operation,
        RoomManagementOperationState::Failed {
            request_id: 5,
            room_id: room.to_owned(),
            operation: RoomManagementOperationKind::Settings,
            kind: OperationFailureKind::UnsupportedPolicyCondition,
        }
    );
}

fn scope(room_id: &str) -> koushi_state::RoomAccessDraftScope {
    koushi_state::RoomAccessDraftScope::Room {
        room_id: room_id.to_owned(),
    }
}

fn load_settings(state: &mut AppState, room_id: &str) {
    reduce(
        state,
        AppAction::RoomSettingsSnapshotLoaded {
            room_id: room_id.to_owned(),
            settings: editable_settings(room_id),
        },
    );
}

#[test]
fn reopening_the_room_editor_restores_access_but_keeps_the_history_draft() {
    let room = "!room:example.invalid";
    let mut state = ready_state();
    load_settings(&mut state, room);
    state.room_management.settings = Some(settings_with_access(
        room,
        koushi_state::RoomAccessCondition {
            join_rule: Some(RoomJoinRule::Invite),
            restricted: None,
            allow_targets: Vec::new(),
        },
    ));
    reduce(
        &mut state,
        AppAction::RoomAccessDraftOpened {
            scope: scope(room),
            create: None,
        },
    );
    reduce(
        &mut state,
        AppAction::RoomAccessDraftRuleSet {
            scope: scope(room),
            rule: Some(RoomJoinRule::Public),
        },
    );
    reduce(
        &mut state,
        AppAction::RoomAccessDraftHistorySet {
            scope: scope(room),
            history: Some(RoomHistoryVisibility::Joined),
        },
    );

    // The access Cancel re-seeds only the access selection; the history draft
    // must survive it (#1177).
    reduce(
        &mut state,
        AppAction::RoomAccessDraftOpened {
            scope: scope(room),
            create: None,
        },
    );
    let draft = state.room_management.draft.as_ref().expect("draft");
    assert_eq!(draft.rule, Some(RoomJoinRule::Invite));
    assert_eq!(
        draft.history,
        Some(RoomHistoryVisibility::Joined),
        "Cancel restores access without discarding the history selection"
    );
}

#[test]
fn room_access_draft_commands_are_canonical_and_revisioned() {
    let mut state = ready_state();
    load_settings(&mut state, "!room:example.invalid");

    reduce(
        &mut state,
        AppAction::RoomAccessDraftRuleSet {
            scope: scope("!room:example.invalid"),
            rule: Some(RoomJoinRule::Restricted),
        },
    );
    reduce(
        &mut state,
        AppAction::RoomAccessDraftAllowTargetsSet {
            scope: scope("!room:example.invalid"),
            allow_targets: vec![
                "!b:example.invalid".to_owned(),
                "!a:example.invalid".to_owned(),
                "!a:example.invalid".to_owned(),
            ],
        },
    );
    reduce(
        &mut state,
        AppAction::RoomAccessDraftHistorySet {
            scope: scope("!room:example.invalid"),
            history: Some(RoomHistoryVisibility::Invited),
        },
    );

    let draft = state.room_management.draft.as_ref().expect("draft");
    assert_eq!(
        draft.allow_targets,
        vec![
            "!a:example.invalid".to_owned(),
            "!b:example.invalid".to_owned()
        ]
    );
    assert_eq!(draft.history, Some(RoomHistoryVisibility::Invited));
    assert_eq!(draft.revision, 3, "three real mutations");

    reduce(
        &mut state,
        AppAction::RoomAccessDraftReset {
            scope: scope("!room:example.invalid"),
        },
    );
    assert!(state.room_management.draft.is_none());
}

#[test]
fn room_access_draft_ignores_a_stale_room_scope() {
    let mut state = ready_state();
    load_settings(&mut state, "!room:example.invalid");

    let effects = reduce(
        &mut state,
        AppAction::RoomAccessDraftRuleSet {
            scope: scope("!other:example.invalid"),
            rule: Some(RoomJoinRule::Public),
        },
    );

    assert!(
        effects.is_empty(),
        "an unloaded room's draft command is ignored"
    );
    assert!(state.room_management.draft.is_none());
}

#[test]
fn access_policy_change_wire_shape_is_camel_case_and_canonical() {
    let change = RoomSettingChange::AccessPolicy(koushi_state::RoomAccessPolicy::new(
        RoomJoinRule::Restricted,
        vec![
            "!b:example.invalid".to_owned(),
            "!a:example.invalid".to_owned(),
        ],
    ));
    let value = serde_json::to_value(&change).expect("serialize access policy change");
    assert_eq!(
        value,
        serde_json::json!({
            "accessPolicy": {
                "rule": "restricted",
                "allowTargets": ["!a:example.invalid", "!b:example.invalid"],
            }
        })
    );
    let round_tripped: RoomSettingChange =
        serde_json::from_value(value).expect("deserialize access policy change");
    assert_eq!(round_tripped, change);
}

fn joined_space(space_id: &str) -> koushi_state::SpaceSummary {
    koushi_state::SpaceSummary {
        space_id: space_id.to_owned(),
        raw_name: Some("Design Team".to_owned()),
        display_name: "Design Team".to_owned(),
        avatar: None,
        join_rule: None,
        child_room_ids: Vec::new(),
        parent_side_child_room_ids: Vec::new(),
    }
}

fn open_create(state: &mut AppState, session_id: u64, parent_space: Option<&str>) {
    reduce(
        state,
        AppAction::RoomAccessDraftOpened {
            scope: koushi_state::RoomAccessDraftScope::Create { session_id },
            create: Some(koushi_state::CreateRoomAccessSeed {
                visibility: koushi_state::CreateRoomVisibility::Private,
                invited_only: false,
                parent_space_id: parent_space.map(str::to_owned),
            }),
        },
    );
}

#[test]
fn opening_a_create_session_seeds_the_legacy_preset_and_its_target_set() {
    let mut state = ready_state();
    open_create(&mut state, 1, Some("!space:example.invalid"));

    let draft = state.room_management.draft.as_ref().expect("create draft");
    assert_eq!(
        draft.scope,
        koushi_state::RoomAccessDraftScope::Create { session_id: 1 }
    );
    assert_eq!(draft.rule, Some(RoomJoinRule::Restricted));
    assert_eq!(
        draft.allow_targets,
        vec!["!space:example.invalid".to_owned()]
    );
    assert!(
        !draft.touched,
        "the seeded legacy selection is not a user choice"
    );

    // A target edit carries the seeded rule and marks the selection touched.
    reduce(
        &mut state,
        AppAction::RoomAccessDraftAllowTargetsSet {
            scope: koushi_state::RoomAccessDraftScope::Create { session_id: 1 },
            allow_targets: vec![
                "!space:example.invalid".to_owned(),
                "!space-b:example.invalid".to_owned(),
            ],
        },
    );
    let draft = state.room_management.draft.as_ref().expect("create draft");
    assert_eq!(draft.rule, Some(RoomJoinRule::Restricted));
    assert_eq!(
        draft.allow_targets,
        vec![
            "!space-b:example.invalid".to_owned(),
            "!space:example.invalid".to_owned()
        ]
    );
    assert!(draft.touched);
}

#[test]
fn a_retired_create_session_cannot_recreate_its_draft() {
    let mut state = ready_state();
    open_create(&mut state, 1, None);
    reduce(
        &mut state,
        AppAction::RoomAccessDraftRuleSet {
            scope: koushi_state::RoomAccessDraftScope::Create { session_id: 1 },
            rule: Some(RoomJoinRule::Public),
        },
    );
    assert!(
        state
            .room_management
            .draft
            .as_ref()
            .is_some_and(|draft| draft.rule == Some(RoomJoinRule::Public))
    );

    reduce(
        &mut state,
        AppAction::RoomAccessDraftReset {
            scope: koushi_state::RoomAccessDraftScope::Create { session_id: 1 },
        },
    );
    assert!(state.room_management.draft.is_none());
    assert_eq!(state.room_management.active_create_session, None);

    // An old mutation from the retired session is ignored, not recreated.
    let effects = reduce(
        &mut state,
        AppAction::RoomAccessDraftRuleSet {
            scope: koushi_state::RoomAccessDraftScope::Create { session_id: 1 },
            rule: Some(RoomJoinRule::Knock),
        },
    );
    assert!(effects.is_empty());
    assert!(state.room_management.draft.is_none());

    // A stale re-open of the retired session must not recreate the draft either.
    open_create(&mut state, 1, None);
    assert!(
        state.room_management.draft.is_none(),
        "a stale Open cannot recreate a retired session's draft"
    );

    // Session zero is never admitted.
    open_create(&mut state, 0, None);
    assert!(state.room_management.draft.is_none());

    open_create(&mut state, 2, None);
    assert_eq!(
        state
            .room_management
            .draft
            .as_ref()
            .map(|draft| draft.scope.clone()),
        Some(koushi_state::RoomAccessDraftScope::Create { session_id: 2 })
    );
}

#[test]
fn a_retired_create_session_mutation_never_replaces_the_current_draft() {
    let mut state = ready_state();
    open_create(&mut state, 1, Some("!space:example.invalid"));
    reduce(
        &mut state,
        AppAction::RoomAccessDraftRuleSet {
            scope: koushi_state::RoomAccessDraftScope::Create { session_id: 1 },
            rule: Some(RoomJoinRule::Public),
        },
    );
    // A second dialog opens before the first one's close lands.
    open_create(&mut state, 2, Some("!space:example.invalid"));
    let current = state
        .room_management
        .draft
        .as_ref()
        .expect("session 2 draft");
    assert_eq!(current.rule, Some(RoomJoinRule::Restricted));

    // The retired session 1 mutation must not replace session 2's draft.
    let effects = reduce(
        &mut state,
        AppAction::RoomAccessDraftRuleSet {
            scope: koushi_state::RoomAccessDraftScope::Create { session_id: 1 },
            rule: Some(RoomJoinRule::Knock),
        },
    );
    assert!(effects.is_empty());
    let draft = state
        .room_management
        .draft
        .as_ref()
        .expect("session 2 draft survives");
    assert_eq!(
        draft.scope,
        koushi_state::RoomAccessDraftScope::Create { session_id: 2 }
    );
    assert_eq!(draft.rule, Some(RoomJoinRule::Restricted));
    assert_eq!(
        draft.allow_targets,
        vec!["!space:example.invalid".to_owned()]
    );
}

#[test]
fn loading_another_room_invalidates_the_previous_rooms_draft() {
    let mut state = ready_state();
    load_settings(&mut state, "!room:example.invalid");
    reduce(
        &mut state,
        AppAction::RoomAccessDraftRuleSet {
            scope: scope("!room:example.invalid"),
            rule: Some(RoomJoinRule::Public),
        },
    );
    assert!(state.room_management.draft.is_some());

    load_settings(&mut state, "!other:example.invalid");
    assert!(
        state.room_management.draft.is_none(),
        "the previous room's draft is invalidated synchronously"
    );
    assert_eq!(
        state.room_management.active_room_editor.as_deref(),
        Some("!other:example.invalid")
    );
}

#[test]
fn a_newly_selected_target_must_be_a_joined_verified_space() {
    let mut state = ready_state();
    load_settings(&mut state, "!room:example.invalid");
    // A valid, editable confirmed condition, so the target check is what decides.
    // Dispatched as a read so the raw pre-send facts the admission consults move
    // with it.
    reduce(
        &mut state,
        AppAction::RoomSettingsSnapshotLoaded {
            room_id: "!room:example.invalid".to_owned(),
            settings: settings_with_access(
                "!room:example.invalid",
                koushi_state::RoomAccessCondition {
                    join_rule: Some(RoomJoinRule::Invite),
                    restricted: None,
                    allow_targets: Vec::new(),
                },
            ),
        },
    );

    // An ordinary room id that is not a joined Space is rejected.
    let effects = reduce(
        &mut state,
        AppAction::RoomSettingUpdateRequested {
            request_id: 1,
            room_id: "!room:example.invalid".to_owned(),
            change: access_policy_with_target("!ordinary:example.invalid"),
        },
    );
    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)));
    assert_eq!(
        state.room_management.operation,
        RoomManagementOperationState::Failed {
            request_id: 1,
            room_id: "!room:example.invalid".to_owned(),
            operation: RoomManagementOperationKind::Settings,
            kind: OperationFailureKind::PolicyNotVerified,
        }
    );

    // A joined verified Space is admitted.
    state.spaces = vec![joined_space("!space-b:example.invalid")];
    reduce(
        &mut state,
        AppAction::RoomSettingUpdateRequested {
            request_id: 2,
            room_id: "!room:example.invalid".to_owned(),
            change: access_policy_with_target("!space-b:example.invalid"),
        },
    );
    assert_eq!(
        state.room_management.operation,
        RoomManagementOperationState::Pending {
            request_id: 2,
            room_id: "!room:example.invalid".to_owned(),
            operation: RoomManagementOperationKind::Settings,
        }
    );
}

#[test]
fn an_existing_non_space_condition_is_rejected_when_its_edit_would_drop_it() {
    let mut state = ready_state();
    state.room_management = RoomManagementState {
        selected_room_id: Some("!room:example.invalid".to_owned()),
        settings: Some(settings_with_access(
            "!room:example.invalid",
            koushi_state::RoomAccessCondition {
                join_rule: Some(RoomJoinRule::Restricted),
                restricted: Some(koushi_state::RestrictedConditions::MembershipOnly),
                allow_targets: vec![koushi_state::RoomAllowTarget {
                    kind: koushi_state::RoomAllowTargetKind::Room,
                    room_id: "!ordinary:example.invalid".to_owned(),
                }],
            },
        )),
        draft: None,
        directory: koushi_state::RoomDirectoryVisibility::Loading,
        active_room_editor: None,
        active_create_session: None,
        operation: RoomManagementOperationState::Idle,
    };
    state.spaces = vec![joined_space("!space-b:example.invalid")];

    reduce(
        &mut state,
        AppAction::RoomSettingUpdateRequested {
            request_id: 1,
            room_id: "!room:example.invalid".to_owned(),
            change: access_policy_with_target("!space-b:example.invalid"),
        },
    );
    assert_eq!(
        state.room_management.operation,
        RoomManagementOperationState::Failed {
            request_id: 1,
            room_id: "!room:example.invalid".to_owned(),
            operation: RoomManagementOperationKind::Settings,
            kind: OperationFailureKind::UnsupportedPolicyCondition,
        },
        "dropping an unmodelled non-Space condition is rejected explicitly"
    );
}

fn access_policy_with_target(target: &str) -> RoomSettingChange {
    RoomSettingChange::AccessPolicy(koushi_state::RoomAccessPolicy::new(
        RoomJoinRule::Restricted,
        vec![target.to_owned()],
    ))
}

#[test]
fn directory_visibility_observation_only_applies_to_the_open_room() {
    let mut state = ready_state();
    load_settings(&mut state, "!room:example.invalid");
    assert_eq!(
        state.room_management.directory,
        koushi_state::RoomDirectoryVisibility::Loading
    );

    reduce(
        &mut state,
        AppAction::RoomDirectoryVisibilityObserved {
            room_id: "!other:example.invalid".to_owned(),
            visibility: koushi_state::RoomDirectoryVisibility::Public,
        },
    );
    assert_eq!(
        state.room_management.directory,
        koushi_state::RoomDirectoryVisibility::Loading,
        "another room's publication does not describe the open room"
    );

    reduce(
        &mut state,
        AppAction::RoomDirectoryVisibilityObserved {
            room_id: "!room:example.invalid".to_owned(),
            visibility: koushi_state::RoomDirectoryVisibility::Public,
        },
    );
    assert_eq!(
        state.room_management.directory,
        koushi_state::RoomDirectoryVisibility::Public
    );

    // A same-room pre-send read keeps the confirmed publication it already read.
    load_settings(&mut state, "!room:example.invalid");
    assert_eq!(
        state.room_management.directory,
        koushi_state::RoomDirectoryVisibility::Public
    );
}

#[test]
fn opening_a_room_editor_restores_the_confirmed_allow_list() {
    let mut state = ready_state();
    state.room_management = RoomManagementState {
        selected_room_id: Some("!room:example.invalid".to_owned()),
        settings: Some(settings_with_access(
            "!room:example.invalid",
            restricted_access(koushi_state::RestrictedConditions::MembershipOnly),
        )),
        draft: None,
        directory: koushi_state::RoomDirectoryVisibility::Loading,
        active_room_editor: None,
        active_create_session: None,
        operation: RoomManagementOperationState::Idle,
    };

    reduce(
        &mut state,
        AppAction::RoomAccessDraftOpened {
            scope: scope("!room:example.invalid"),
            create: None,
        },
    );

    let draft = state.room_management.draft.as_ref().expect("room draft");
    assert_eq!(draft.rule, Some(RoomJoinRule::Restricted));
    assert_eq!(
        draft.allow_targets,
        vec!["!space:example.invalid".to_owned()],
        "the confirmed allow list is restored into the draft, not left empty"
    );
    assert!(
        !draft.touched,
        "the seeded confirmed selection is not a user choice"
    );
    assert!(
        draft.context_is_confirmed(
            koushi_state::RoomAccessPreviewContext::Access,
            state.room_management.settings.as_ref().unwrap(),
        ),
        "the restored selection reads as confirmed until it is edited"
    );
}

#[test]
fn two_rapid_room_target_edits_apply_against_the_current_draft() {
    let mut state = ready_state();
    state.room_management = RoomManagementState {
        selected_room_id: Some("!room:example.invalid".to_owned()),
        settings: Some(settings_with_access(
            "!room:example.invalid",
            restricted_access(koushi_state::RestrictedConditions::MembershipOnly),
        )),
        draft: None,
        directory: koushi_state::RoomDirectoryVisibility::Loading,
        active_room_editor: None,
        active_create_session: None,
        operation: RoomManagementOperationState::Idle,
    };
    reduce(
        &mut state,
        AppAction::RoomAccessDraftOpened {
            scope: scope("!room:example.invalid"),
            create: None,
        },
    );

    // Two edits arrive back-to-back: the second must be applied to the draft the
    // first produced, so neither selection is dropped.
    reduce(
        &mut state,
        AppAction::RoomAccessDraftAllowTargetToggled {
            scope: scope("!room:example.invalid"),
            target: "!space-b:example.invalid".to_owned(),
            selected: true,
        },
    );
    reduce(
        &mut state,
        AppAction::RoomAccessDraftAllowTargetToggled {
            scope: scope("!room:example.invalid"),
            target: "!space-c:example.invalid".to_owned(),
            selected: true,
        },
    );

    let draft = state.room_management.draft.as_ref().expect("room draft");
    assert_eq!(
        draft.allow_targets,
        vec![
            "!space-b:example.invalid".to_owned(),
            "!space-c:example.invalid".to_owned(),
            "!space:example.invalid".to_owned(),
        ]
    );
    assert!(draft.touched);

    // An edit against the confirmed rule clears the rest of the list as a user
    // choice, preserving the unmodified confirmed target only.
    reduce(
        &mut state,
        AppAction::RoomAccessDraftAllowTargetToggled {
            scope: scope("!room:example.invalid"),
            target: "!space:example.invalid".to_owned(),
            selected: false,
        },
    );
    let draft = state.room_management.draft.as_ref().expect("room draft");
    assert_eq!(
        draft.allow_targets,
        vec![
            "!space-b:example.invalid".to_owned(),
            "!space-c:example.invalid".to_owned()
        ]
    );
}

#[test]
fn the_draft_serializes_only_renderable_allow_targets() {
    let room = "!room:example.invalid";
    let mut state = ready_state();
    state.room_management = RoomManagementState {
        selected_room_id: Some(room.to_owned()),
        settings: Some(settings_with_access(
            room,
            koushi_state::RoomAccessCondition {
                join_rule: Some(RoomJoinRule::Restricted),
                restricted: Some(koushi_state::RestrictedConditions::MembershipOnly),
                allow_targets: vec![
                    koushi_state::RoomAllowTarget {
                        kind: koushi_state::RoomAllowTargetKind::Space,
                        room_id: "!space:example.invalid".to_owned(),
                    },
                    koushi_state::RoomAllowTarget {
                        kind: koushi_state::RoomAllowTargetKind::Room,
                        room_id: "!ordinary:example.invalid".to_owned(),
                    },
                ],
            },
        )),
        ..RoomManagementState::default()
    };
    state.spaces = vec![joined_space("!space:example.invalid")];
    reduce(
        &mut state,
        AppAction::RoomAccessDraftOpened {
            scope: scope(room),
            create: None,
        },
    );

    let draft = state.room_management.draft.as_ref().expect("draft");
    assert_eq!(
        draft.allow_targets.len(),
        2,
        "the complete selected set stays inside Rust"
    );
    let wire = serde_json::to_value(draft).expect("draft wire");
    assert_eq!(
        wire["allowTargets"],
        serde_json::json!(["!space:example.invalid"]),
        "only a joined Space the picker can render is serialized"
    );
    assert!(
        !wire.to_string().contains("!ordinary:example.invalid"),
        "an ordinary-room identity never reaches the renderer: {wire}"
    );
}

#[test]
fn the_access_preview_never_serializes_a_preserved_target_identity() {
    let room = "!room:example.invalid";
    let mut state = ready_state();
    state.room_management = RoomManagementState {
        selected_room_id: Some(room.to_owned()),
        settings: Some(settings_with_access(
            room,
            koushi_state::RoomAccessCondition {
                join_rule: Some(RoomJoinRule::Restricted),
                restricted: Some(koushi_state::RestrictedConditions::MembershipPlusUnsupported),
                allow_targets: vec![
                    koushi_state::RoomAllowTarget {
                        kind: koushi_state::RoomAllowTargetKind::Space,
                        room_id: "!space:example.invalid".to_owned(),
                    },
                    koushi_state::RoomAllowTarget {
                        kind: koushi_state::RoomAllowTargetKind::Room,
                        room_id: "!ordinary:example.invalid".to_owned(),
                    },
                ],
            },
        )),
        ..RoomManagementState::default()
    };
    state.spaces = vec![joined_space("!space:example.invalid")];
    let editor_scope = scope(room);
    reduce(
        &mut state,
        AppAction::RoomAccessDraftOpened {
            scope: editor_scope.clone(),
            create: None,
        },
    );

    for context in [
        koushi_state::RoomAccessPreviewContext::Access,
        koushi_state::RoomAccessPreviewContext::History,
    ] {
        let preview = koushi_state::preview_room_access_draft(&state, &editor_scope, context);
        let wire = serde_json::to_value(&preview).expect("preview wire");
        assert!(
            !wire.to_string().contains("!ordinary:example.invalid"),
            "an unrenderable preserved identity never reaches the renderer through a {context:?} \
             preview: {wire}"
        );
    }
}
