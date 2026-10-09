//! #935: a Space's access mode — the join-rule permission guard and how a
//! join rule changed elsewhere reaches the open settings snapshot.

use koushi_state::{
    AppAction, AppEffect, AppState, OperationFailureKind, RoomHistoryVisibility, RoomJoinRule,
    RoomManagementOperationKind, RoomManagementOperationState, RoomPermissionFacts,
    RoomSettingChange, RoomSettingsSnapshot, SessionInfo, SessionState, SpaceSummary, UiEvent,
    reduce,
};

const SPACE_ID: &str = "!space:example.invalid";

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://matrix.example.org".to_owned(),
            user_id: "@user-a:example.invalid".to_owned(),
            device_id: "DEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        ..AppState::default()
    }
}

fn space(join_rule: Option<RoomJoinRule>) -> SpaceSummary {
    SpaceSummary {
        space_id: SPACE_ID.to_owned(),
        raw_name: Some("Space".to_owned()),
        display_name: "Space".to_owned(),
        avatar: None,
        join_rule,
        child_room_ids: Vec::new(),
        parent_side_child_room_ids: Vec::new(),
    }
}

fn settings(join_rule: RoomJoinRule, permissions: RoomPermissionFacts) -> RoomSettingsSnapshot {
    RoomSettingsSnapshot {
        room_id: SPACE_ID.to_owned(),
        name: Some("Space".to_owned()),
        topic: None,
        avatar_url: None,
        canonical_alias: None,
        alternate_aliases: Vec::new(),
        share_link: None,
        join_rule,
        history_visibility: RoomHistoryVisibility::Shared,
        access: koushi_state::RoomAccessCondition::default(),
        permissions,
        members: Vec::new(),
    }
}

/// May change who can join, but not rename or re-topic the Space.
fn join_rule_only() -> RoomPermissionFacts {
    RoomPermissionFacts {
        can_change_join_rule: true,
        ..RoomPermissionFacts::default()
    }
}

fn open_space_settings(state: &mut AppState, join_rule: RoomJoinRule) {
    reduce(
        state,
        AppAction::RoomListUpdated {
            spaces: vec![space(Some(join_rule))],
            rooms: Vec::new(),
        },
    );
    reduce(
        state,
        AppAction::RoomSettingsSnapshotLoaded {
            room_id: SPACE_ID.to_owned(),
            settings: settings(join_rule, join_rule_only()),
        },
    );
}

fn sync_join_rule(state: &mut AppState, join_rule: RoomJoinRule) -> Vec<AppEffect> {
    reduce(
        state,
        AppAction::RoomListUpdated {
            spaces: vec![space(Some(join_rule))],
            rooms: Vec::new(),
        },
    )
}

fn shown_join_rule(state: &AppState) -> Option<RoomJoinRule> {
    state
        .room_management
        .settings
        .as_ref()
        .map(|settings| settings.join_rule)
}

#[test]
fn join_rule_permission_is_separate_from_the_other_settings() {
    let permissions = join_rule_only();
    assert!(permissions.allows_setting_change(&RoomSettingChange::JoinRule(RoomJoinRule::Public)));
    for change in [
        RoomSettingChange::Name(Some("Renamed".to_owned())),
        RoomSettingChange::Topic(None),
        RoomSettingChange::AvatarUrl(None),
        RoomSettingChange::HistoryVisibility(RoomHistoryVisibility::Joined),
    ] {
        assert!(!permissions.allows_setting_change(&change));
    }

    let settings_only = RoomPermissionFacts {
        can_edit_settings: true,
        ..RoomPermissionFacts::default()
    };
    assert!(
        !settings_only.allows_setting_change(&RoomSettingChange::JoinRule(RoomJoinRule::Invite))
    );
}

#[test]
fn only_rules_the_command_can_carry_are_settable() {
    assert!(RoomJoinRule::Public.is_settable());
    assert!(RoomJoinRule::Invite.is_settable());
    assert!(!RoomJoinRule::Restricted.is_settable());
    assert!(!RoomJoinRule::KnockRestricted.is_settable());
    assert!(!RoomJoinRule::Unknown.is_settable());
}

#[test]
fn join_rule_change_is_admitted_on_the_join_rule_permission_alone() {
    let mut state = ready_state();
    open_space_settings(&mut state, RoomJoinRule::Invite);

    reduce(
        &mut state,
        AppAction::RoomSettingUpdateRequested {
            request_id: 3,
            room_id: SPACE_ID.to_owned(),
            change: RoomSettingChange::JoinRule(RoomJoinRule::Public),
        },
    );

    assert_eq!(
        state.room_management.operation,
        RoomManagementOperationState::Pending {
            request_id: 3,
            room_id: SPACE_ID.to_owned(),
            operation: RoomManagementOperationKind::Settings,
        }
    );
}

#[test]
fn join_rule_change_without_the_join_rule_permission_is_forbidden() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomSettingsSnapshotLoaded {
            room_id: SPACE_ID.to_owned(),
            settings: settings(
                RoomJoinRule::Invite,
                RoomPermissionFacts {
                    can_edit_settings: true,
                    ..RoomPermissionFacts::default()
                },
            ),
        },
    );

    reduce(
        &mut state,
        AppAction::RoomSettingUpdateRequested {
            request_id: 4,
            room_id: SPACE_ID.to_owned(),
            change: RoomSettingChange::JoinRule(RoomJoinRule::Public),
        },
    );

    assert_eq!(
        state.room_management.operation,
        RoomManagementOperationState::Failed {
            request_id: 4,
            room_id: SPACE_ID.to_owned(),
            operation: RoomManagementOperationKind::Settings,
            kind: OperationFailureKind::Forbidden,
        }
    );
}

#[test]
fn a_join_rule_changed_by_another_client_reaches_the_open_settings() {
    let mut state = ready_state();
    open_space_settings(&mut state, RoomJoinRule::Invite);

    let effects = sync_join_rule(&mut state, RoomJoinRule::Public);

    assert_eq!(shown_join_rule(&state), Some(RoomJoinRule::Public));
    assert!(effects.contains(&AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)));
    // The permission facts are Core's to re-read; sync leaves them alone.
    assert_eq!(
        state
            .room_management
            .settings
            .as_ref()
            .map(|s| s.permissions),
        Some(join_rule_only())
    );
}

#[test]
fn a_room_list_still_carrying_the_old_rule_does_not_revert_a_saved_change() {
    let mut state = ready_state();
    open_space_settings(&mut state, RoomJoinRule::Invite);
    reduce(
        &mut state,
        AppAction::RoomSettingUpdateRequested {
            request_id: 5,
            room_id: SPACE_ID.to_owned(),
            change: RoomSettingChange::JoinRule(RoomJoinRule::Public),
        },
    );
    reduce(
        &mut state,
        AppAction::RoomSettingUpdateSucceeded {
            request_id: 5,
            room_id: SPACE_ID.to_owned(),
            settings: settings(RoomJoinRule::Public, join_rule_only()),
        },
    );

    // The server has not echoed the event back yet.
    let effects = sync_join_rule(&mut state, RoomJoinRule::Invite);
    assert_eq!(shown_join_rule(&state), Some(RoomJoinRule::Public));
    assert!(!effects.contains(&AppEffect::EmitUiEvent(UiEvent::RoomManagementChanged)));

    // Then it has.
    sync_join_rule(&mut state, RoomJoinRule::Public);
    assert_eq!(shown_join_rule(&state), Some(RoomJoinRule::Public));
}

#[test]
fn a_rule_first_learned_from_sync_does_not_override_loaded_settings() {
    let mut state = ready_state();
    // A restored snapshot predates the field, so the Space's rule is unknown.
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: vec![space(None)],
            rooms: Vec::new(),
        },
    );
    reduce(
        &mut state,
        AppAction::RoomSettingsSnapshotLoaded {
            room_id: SPACE_ID.to_owned(),
            settings: settings(RoomJoinRule::Public, join_rule_only()),
        },
    );

    sync_join_rule(&mut state, RoomJoinRule::Invite);

    assert_eq!(shown_join_rule(&state), Some(RoomJoinRule::Public));
}

#[test]
fn another_spaces_join_rule_change_leaves_the_open_settings_alone() {
    let mut state = ready_state();
    open_space_settings(&mut state, RoomJoinRule::Invite);
    let other = |join_rule| SpaceSummary {
        space_id: "!other:example.invalid".to_owned(),
        ..space(Some(join_rule))
    };
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: vec![
                space(Some(RoomJoinRule::Invite)),
                other(RoomJoinRule::Invite),
            ],
            rooms: Vec::new(),
        },
    );

    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: vec![
                space(Some(RoomJoinRule::Invite)),
                other(RoomJoinRule::Public),
            ],
            rooms: Vec::new(),
        },
    );

    assert_eq!(shown_join_rule(&state), Some(RoomJoinRule::Invite));
}

#[test]
fn space_join_rule_serializes_with_the_wire_names() {
    let value = serde_json::to_value(space(Some(RoomJoinRule::KnockRestricted))).expect("space");
    assert_eq!(value["join_rule"], serde_json::json!("knockRestricted"));
    let legacy: SpaceSummary = serde_json::from_value(serde_json::json!({
        "space_id": SPACE_ID,
        "display_name": "Space",
        "child_room_ids": [],
    }))
    .expect("legacy space summary");
    assert_eq!(legacy.join_rule, None);
}
