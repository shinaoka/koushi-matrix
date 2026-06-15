use matrix_desktop_state::{
    AppAction, AppEffect, AppState, OperationFailureKind, RoomHistoryVisibility, RoomJoinRule,
    RoomManagementOperationKind, RoomManagementOperationState, RoomManagementState, RoomMemberRole,
    RoomMemberSummary, RoomModerationAction, RoomPermissionFacts, RoomSettingChange,
    RoomSettingsSnapshot, SessionInfo, SessionState, UiEvent, reduce,
};

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.org".to_owned(),
        user_id: "@user-a:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
    }
}

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(session_info()),
        ..AppState::default()
    }
}

fn editable_settings(room_id: &str) -> RoomSettingsSnapshot {
    RoomSettingsSnapshot {
        room_id: room_id.to_owned(),
        name: Some("Synthetic Room".to_owned()),
        topic: Some("Synthetic topic".to_owned()),
        avatar_url: Some("mxc://example.invalid/avatar".to_owned()),
        join_rule: RoomJoinRule::Invite,
        history_visibility: RoomHistoryVisibility::Shared,
        permissions: RoomPermissionFacts {
            can_edit_settings: true,
            can_edit_roles: true,
            can_kick: true,
            can_ban: true,
            can_unban: false,
        },
        members: vec![
            RoomMemberSummary {
                user_id: "@user-a:example.invalid".to_owned(),
                display_name: Some("User A".to_owned()),
                avatar_url: None,
                power_level: Some(100),
                role: RoomMemberRole::Administrator,
            },
            RoomMemberSummary {
                user_id: "@target:example.invalid".to_owned(),
                display_name: Some("Target".to_owned()),
                avatar_url: Some("mxc://example.invalid/target-avatar".to_owned()),
                power_level: Some(0),
                role: RoomMemberRole::User,
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
            operation: RoomManagementOperationState::Idle,
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
