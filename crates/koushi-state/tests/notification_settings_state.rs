use koushi_state::{
    AppAction, AppState, NotificationSettings, OperationFailureKind, RoomNotificationMode,
    RoomNotificationModeOperation, RoomSummary, SessionInfo, SessionState, SettingsPatch,
    SettingsValues, UiEvent, reduce,
};

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.org".to_owned(),
        user_id: "@user:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
    }
}

fn ready_state() -> AppState {
    let mut state = AppState::default();
    state.session = SessionState::Ready(session_info());
    state.rooms.push(RoomSummary {
        room_id: "!known:example.invalid".to_owned(),
        display_name: "Known Room".to_owned(),
        display_label: "Known Room".to_owned(),
        original_display_label: "Known Room".to_owned(),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: Default::default(),
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        marked_unread: false,
        last_activity_ms: 0,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 0,
    });
    state
}

#[test]
fn default_room_notification_mode_is_all() {
    assert_eq!(RoomNotificationMode::default(), RoomNotificationMode::All);
}

#[test]
fn set_room_notification_mode_updates_known_room_and_sets_pending() {
    let mut state = ready_state();
    let effects = reduce(
        &mut state,
        AppAction::RoomNotificationModeSet {
            request_id: 1,
            room_id: "!known:example.invalid".to_owned(),
            mode: RoomNotificationMode::Mute,
        },
    );

    assert!(effects.iter().any(|effect| matches!(
        effect,
        koushi_state::AppEffect::EmitUiEvent(UiEvent::RoomNotificationSettingsChanged)
    )));
    let settings = state
        .room_notification_settings
        .get("!known:example.invalid")
        .expect("settings for known room");
    assert_eq!(settings.mode, RoomNotificationMode::Mute);
    assert_eq!(
        settings.operation,
        RoomNotificationModeOperation::Pending { request_id: 1 }
    );
}

#[test]
fn set_room_notification_mode_ignores_unknown_room() {
    let mut state = ready_state();
    let effects = reduce(
        &mut state,
        AppAction::RoomNotificationModeSet {
            request_id: 1,
            room_id: "!unknown:example.invalid".to_owned(),
            mode: RoomNotificationMode::Mute,
        },
    );

    assert!(effects.is_empty());
    assert!(
        !state
            .room_notification_settings
            .contains_key("!unknown:example.invalid")
    );
}

#[test]
fn set_room_notification_mode_requires_ready_session() {
    let mut state = AppState::default();
    let effects = reduce(
        &mut state,
        AppAction::RoomNotificationModeSet {
            request_id: 1,
            room_id: "!known:example.invalid".to_owned(),
            mode: RoomNotificationMode::Mute,
        },
    );

    assert!(effects.is_empty());
    assert!(state.room_notification_settings.is_empty());
}

#[test]
fn completed_mode_set_clears_pending() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomNotificationModeSet {
            request_id: 1,
            room_id: "!known:example.invalid".to_owned(),
            mode: RoomNotificationMode::Mute,
        },
    );
    let effects = reduce(
        &mut state,
        AppAction::RoomNotificationModeCompleted {
            request_id: 1,
            room_id: "!known:example.invalid".to_owned(),
        },
    );

    assert!(effects.iter().any(|effect| matches!(
        effect,
        koushi_state::AppEffect::EmitUiEvent(UiEvent::RoomNotificationSettingsChanged)
    )));
    let settings = state
        .room_notification_settings
        .get("!known:example.invalid")
        .unwrap();
    assert_eq!(settings.operation, RoomNotificationModeOperation::Idle);
}

#[test]
fn completed_mode_set_ignored_for_mismatched_request() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomNotificationModeSet {
            request_id: 1,
            room_id: "!known:example.invalid".to_owned(),
            mode: RoomNotificationMode::Mute,
        },
    );
    reduce(
        &mut state,
        AppAction::RoomNotificationModeCompleted {
            request_id: 2,
            room_id: "!known:example.invalid".to_owned(),
        },
    );

    let settings = state
        .room_notification_settings
        .get("!known:example.invalid")
        .unwrap();
    assert_eq!(
        settings.operation,
        RoomNotificationModeOperation::Pending { request_id: 1 }
    );
}

#[test]
fn failed_mode_set_is_recorded_per_room() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomNotificationModeSet {
            request_id: 1,
            room_id: "!known:example.invalid".to_owned(),
            mode: RoomNotificationMode::Mute,
        },
    );
    let effects = reduce(
        &mut state,
        AppAction::RoomNotificationModeFailed {
            request_id: 1,
            room_id: "!known:example.invalid".to_owned(),
            kind: OperationFailureKind::Network,
        },
    );

    assert!(effects.iter().any(|effect| matches!(
        effect,
        koushi_state::AppEffect::EmitUiEvent(UiEvent::RoomNotificationSettingsChanged)
    )));
    let settings = state
        .room_notification_settings
        .get("!known:example.invalid")
        .unwrap();
    assert_eq!(
        settings.operation,
        RoomNotificationModeOperation::Failed {
            request_id: 1,
            failure_kind: OperationFailureKind::Network,
        }
    );
}

#[test]
fn privacy_settings_persist_defaults() {
    let values = SettingsValues::default();
    assert!(values.notifications.send_read_receipts);
    assert!(values.notifications.send_typing_notifications);
}

#[test]
fn old_persisted_notification_json_defaults_privacy_to_true() {
    let json = r#"{
        "locale": {"language_tag": null, "text_direction": "auto"},
        "appearance": {"theme": "system"},
        "typography": {"font": "system", "emoji": "system"},
        "keyboard": {"composer_send_shortcut": "enter"},
        "notifications": {"desktop_notifications": true, "sound": true, "badges": true},
        "display": {"code_block_wrap": true, "hide_redacted": false},
        "media": {"image_upload_compression": "never", "image_upload_compression_policy": {"threshold_bytes": 1048576, "threshold_long_edge": 2560, "target_long_edge": 2048, "quality_percent": 82}}
    }"#;
    let values: SettingsValues = serde_json::from_str(json).unwrap();
    assert!(values.notifications.send_read_receipts);
    assert!(values.notifications.send_typing_notifications);
}

#[test]
fn settings_patch_can_change_privacy_toggles() {
    let mut state = AppState::default();
    state.settings.values = SettingsValues::default();
    let patch = SettingsPatch {
        notifications: Some(NotificationSettings {
            desktop_notifications: true,
            sound: true,
            badges: true,
            send_read_receipts: false,
            send_typing_notifications: false,
        }),
        ..Default::default()
    };
    let effects = reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 1,
            patch,
        },
    );

    assert!(effects.iter().any(|effect| matches!(
        effect,
        koushi_state::AppEffect::EmitUiEvent(UiEvent::SettingsChanged)
    )));
    assert!(!state.settings.values.notifications.send_read_receipts);
    assert!(
        !state
            .settings
            .values
            .notifications
            .send_typing_notifications
    );
}

#[test]
fn logout_clears_room_notification_settings() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomNotificationModeSet {
            request_id: 1,
            room_id: "!known:example.invalid".to_owned(),
            mode: RoomNotificationMode::Mute,
        },
    );
    assert!(!state.room_notification_settings.is_empty());

    reduce(&mut state, AppAction::LogoutRequested);
    reduce(&mut state, AppAction::LogoutFinished);

    assert!(state.room_notification_settings.is_empty());
}

#[test]
fn settings_loaded_event_populates_privacy_defaults() {
    let mut state = AppState::default();
    let values = SettingsValues {
        notifications: NotificationSettings {
            desktop_notifications: false,
            sound: false,
            badges: false,
            send_read_receipts: false,
            send_typing_notifications: false,
        },
        ..Default::default()
    };
    reduce(&mut state, AppAction::SettingsLoaded { values });

    assert!(!state.settings.values.notifications.send_read_receipts);
    assert!(
        !state
            .settings
            .values
            .notifications
            .send_typing_notifications
    );
}
