use koushi_state::{
    AppAction, AppState, OperationFailureKind, RoomAccessDraftScope, RoomHistoryVisibility,
    RoomJoinRule, RoomManagementOperationKind, RoomManagementOperationState, RoomPermissionFacts,
    RoomSettingChange, RoomSettingsSnapshot, SessionInfo, SessionState, reduce,
};

const ROOM: &str = "!space:example.invalid";
const EPOCH: u64 = 41;

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://example.invalid".into(),
        user_id: "@topic-test:example.invalid".into(),
        device_id: "TOPICDEVICE".into(),
        authentication_method: Default::default(),
    }
}

fn ready_settings() -> AppState {
    let mut state = AppState {
        session: SessionState::Ready(session_info()),
        sliding_sync_account_epoch: EPOCH,
        ..Default::default()
    };
    reduce(
        &mut state,
        AppAction::RoomSettingsSnapshotLoaded {
            room_id: ROOM.into(),
            settings: RoomSettingsSnapshot {
                room_id: ROOM.into(),
                name: Some("Synthetic Space".into()),
                topic: None,
                avatar_url: None,
                canonical_alias: None,
                alternate_aliases: Vec::new(),
                share_link: None,
                join_rule: RoomJoinRule::Invite,
                history_visibility: RoomHistoryVisibility::Shared,
                access: Default::default(),
                permissions: RoomPermissionFacts {
                    can_change_topic: true,
                    ..Default::default()
                },
                members: Vec::new(),
            },
        },
    );
    state
}

fn observation(
    room_id: &str,
    account_epoch: u64,
    topic: Option<&str>,
    can_change_topic: bool,
) -> AppAction {
    AppAction::RoomTopicObserved {
        room_id: room_id.into(),
        account_epoch,
        topic: topic.map(str::to_owned),
        can_change_topic,
    }
}

#[test]
fn topic_permission_does_not_require_unrelated_settings_powers() {
    let facts = RoomPermissionFacts {
        can_change_topic: true,
        ..Default::default()
    };
    assert!(facts.allows_setting_change(&RoomSettingChange::Topic(Some("Synthetic".into()))));
    assert!(facts.allows_setting_change(&RoomSettingChange::Topic(None)));
    assert!(!facts.allows_setting_change(&RoomSettingChange::Name(Some("Synthetic".into()))));
}

#[test]
fn aggregate_settings_permission_cannot_override_denied_topic_permission() {
    let facts = RoomPermissionFacts {
        can_edit_settings: true,
        ..Default::default()
    };
    assert!(!facts.allows_setting_change(&RoomSettingChange::Topic(Some("Synthetic".into()))));
    assert!(!facts.allows_setting_change(&RoomSettingChange::Topic(None)));
    let mut state = ready_settings();
    state.room_management.settings.as_mut().unwrap().permissions = facts;
    reduce(
        &mut state,
        AppAction::RoomSettingUpdateRequested {
            request_id: 9,
            room_id: ROOM.into(),
            change: RoomSettingChange::Topic(None),
        },
    );
    assert!(matches!(
        state.room_management.operation,
        RoomManagementOperationState::Failed {
            request_id: 9,
            kind: OperationFailureKind::Forbidden,
            ..
        }
    ));
}

#[test]
fn selected_topic_update_clear_and_permission_revocation_are_observed() {
    let mut state = ready_settings();
    for (topic, permission) in [
        (Some("Synthetic shared\nSecond line"), true),
        (None, false),
        (None, true),
    ] {
        let mut expected = state.clone();
        let settings = expected.room_management.settings.as_mut().unwrap();
        settings.topic = topic.map(str::to_owned);
        settings.permissions.can_change_topic = permission;
        reduce(&mut state, observation(ROOM, EPOCH, topic, permission));
        assert_eq!(
            state, expected,
            "observation changed an unrelated state field"
        );
        let before_duplicate = state.clone();
        let effects = reduce(&mut state, observation(ROOM, EPOCH, topic, permission));
        assert_eq!(
            state, before_duplicate,
            "duplicate observation changed state"
        );
        assert!(
            effects.is_empty(),
            "duplicate observation emitted an effect"
        );
    }
}

#[test]
fn observations_preserve_pending_and_failed_operations_draft_and_target() {
    for operation in [
        RoomManagementOperationState::Pending {
            request_id: 17,
            room_id: ROOM.into(),
            operation: RoomManagementOperationKind::Settings,
        },
        RoomManagementOperationState::Failed {
            request_id: 17,
            room_id: ROOM.into(),
            operation: RoomManagementOperationKind::Settings,
            kind: OperationFailureKind::Network,
        },
    ] {
        let mut state = ready_settings();
        let scope = RoomAccessDraftScope::Room {
            room_id: ROOM.into(),
        };
        reduce(
            &mut state,
            AppAction::RoomAccessDraftOpened {
                scope: scope.clone(),
                create: None,
            },
        );
        reduce(
            &mut state,
            AppAction::RoomAccessDraftRuleSet {
                scope,
                rule: Some(RoomJoinRule::Public),
            },
        );
        assert!(
            state.room_management.draft.is_some(),
            "fixture must carry an unsaved draft"
        );
        state.room_management.operation = operation;
        let mut expected = state.clone();
        let settings = expected.room_management.settings.as_mut().unwrap();
        settings.topic = Some("Synthetic remote update".into());
        settings.permissions.can_change_topic = false;
        reduce(
            &mut state,
            observation(ROOM, EPOCH, Some("Synthetic remote update"), false),
        );
        assert_eq!(
            state, expected,
            "observation reset operation, draft or selected target"
        );
    }
}

#[test]
fn topic_observation_never_selects_or_resets_a_settings_target() {
    let mut state = AppState::default();
    let before = state.clone();
    reduce(
        &mut state,
        observation("!other:example.invalid", EPOCH, Some("Synthetic"), true),
    );
    assert_eq!(state, before);
    let mut state = ready_settings();
    let before = state.clone();
    for action in [
        observation("!other:example.invalid", EPOCH, Some("Synthetic"), false),
        observation(ROOM, EPOCH - 1, Some("Synthetic"), false),
        observation(ROOM, EPOCH + 1, Some("Synthetic"), false),
    ] {
        assert!(reduce(&mut state, action).is_empty());
        assert_eq!(state, before);
    }
    state.room_management.settings.as_mut().unwrap().room_id = "!other:example.invalid".into();
    let before = state.clone();
    reduce(
        &mut state,
        observation(ROOM, EPOCH, Some("Synthetic"), false),
    );
    assert_eq!(
        state, before,
        "observation must match selected AND loaded settings room"
    );
}

#[test]
fn topic_observation_is_ignored_outside_a_ready_session() {
    for session in [
        SessionState::SignedOut,
        SessionState::Restoring,
        SessionState::LoggingOut,
        SessionState::Locked(session_info()),
        SessionState::SwitchingAccount {
            info: session_info(),
        },
    ] {
        let mut state = ready_settings();
        state.session = session;
        let before = state.clone();
        assert!(
            reduce(
                &mut state,
                observation(ROOM, EPOCH, Some("Synthetic"), false)
            )
            .is_empty()
        );
        assert_eq!(state, before);
    }
}
