use koushi_state::{
    AppAction, AppEffect, AppState, ConversationActivity, ConversationActivitySource,
    NativeAttentionObservationKind, NotificationSettings, OperationFailureKind,
    RoomNotificationMode, RoomNotificationModeOperation, RoomPreference, RoomPreferencesState,
    RoomSummary, SessionInfo, SessionState, SettingsPatch, SettingsValues, UiEvent, reduce,
};

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.org".to_owned(),
        user_id: "@user:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
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
        recency_stamp: None,
        conversation_activity: None,
        latest_event: None,
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
    assert!(state.room_preferences.rooms.is_empty());
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, AppEffect::PersistRoomPreferences { .. }))
    );
}

#[test]
fn room_notification_mode_changes_recompute_native_attention_badge() {
    let mut state = ready_state();
    state.rooms[0].unread_count = 3;
    state.rooms[0].notification_count = 3;
    state.native_attention.summary.unread_count = 3;
    state.native_attention.summary.badge_count = 3;

    let effects = reduce(
        &mut state,
        AppAction::RoomNotificationModeSet {
            request_id: 1,
            room_id: "!known:example.invalid".to_owned(),
            mode: RoomNotificationMode::Mute,
        },
    );

    assert_eq!(state.native_attention.summary.badge_count, 0);
    assert!(effects.iter().any(|effect| matches!(
        effect,
        AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged)
    )));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        AppEffect::RecordNativeAttentionRecomputed {
            observation: NativeAttentionObservationKind::Live,
            ..
        }
    )));

    reduce(
        &mut state,
        AppAction::RoomNotificationModeSet {
            request_id: 2,
            room_id: "!known:example.invalid".to_owned(),
            mode: RoomNotificationMode::All,
        },
    );
    assert_eq!(state.native_attention.summary.badge_count, 3);
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
fn setting_room_notification_mode_to_all_removes_persisted_preference() {
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
            request_id: 1,
            room_id: "!known:example.invalid".to_owned(),
        },
    );
    assert!(
        state
            .room_preferences
            .rooms
            .contains_key("!known:example.invalid")
    );

    reduce(
        &mut state,
        AppAction::RoomNotificationModeSet {
            request_id: 2,
            room_id: "!known:example.invalid".to_owned(),
            mode: RoomNotificationMode::All,
        },
    );
    let effects = reduce(
        &mut state,
        AppAction::RoomNotificationModeCompleted {
            request_id: 2,
            room_id: "!known:example.invalid".to_owned(),
        },
    );

    assert!(
        !state
            .room_preferences
            .rooms
            .contains_key("!known:example.invalid")
    );
    assert!(effects.iter().any(|effect| matches!(
        effect,
        AppEffect::PersistRoomPreferences {
            request_id: 2,
            preferences
        } if preferences.rooms.is_empty()
    )));
}

#[test]
fn room_preferences_loaded_restores_room_notification_modes() {
    let mut state = ready_state();
    let preferences = RoomPreferencesState {
        rooms: std::collections::BTreeMap::from([(
            "!known:example.invalid".to_owned(),
            RoomPreference {
                notification_mode: Some(RoomNotificationMode::Mute),
                ..RoomPreference::default()
            },
        )]),
    };

    let effects = reduce(
        &mut state,
        AppAction::RoomPreferencesLoaded {
            preferences: preferences.clone(),
        },
    );

    assert_eq!(state.room_preferences, preferences);
    assert_eq!(
        state
            .room_notification_settings
            .get("!known:example.invalid")
            .map(|settings| (settings.mode, settings.operation.clone())),
        Some((
            RoomNotificationMode::Mute,
            RoomNotificationModeOperation::Idle
        ))
    );
    assert!(effects.iter().any(|effect| matches!(
        effect,
        AppEffect::EmitUiEvent(UiEvent::RoomNotificationSettingsChanged)
    )));
}

#[test]
fn loaded_notification_preferences_recompute_activity_projection_with_effective_modes() {
    let mut state = ready_state();
    state.navigation.active_room_id = Some("selected".to_owned());
    let room = |room_id: &str, timestamp_ms: u64| RoomSummary {
        room_id: room_id.to_owned(),
        display_name: room_id.to_owned(),
        display_label: room_id.to_owned(),
        original_display_label: room_id.to_owned(),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: Default::default(),
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        marked_unread: false,
        recency_stamp: Some(timestamp_ms),
        conversation_activity: Some(ConversationActivity {
            timestamp_ms,
            source: ConversationActivitySource::Message,
        }),
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 0,
    };
    let mut mention = room("mention", 100);
    mention.highlight_count = 1;
    let mut notification = room("notification", 900);
    notification.notification_count = 1;
    let mut ordinary = room("ordinary", 950);
    ordinary.unread_count = 1;
    state.rooms = vec![room("selected", 1_000), notification, ordinary, mention];

    let settings = state.settings.values.clone();
    reduce(&mut state, AppAction::SettingsLoaded { values: settings });
    assert_eq!(
        state
            .room_list
            .items
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["mention", "notification", "ordinary", "selected"]
    );

    let preferences = RoomPreferencesState {
        rooms: std::collections::BTreeMap::from([
            (
                "mention".to_owned(),
                RoomPreference {
                    notification_mode: Some(RoomNotificationMode::Mute),
                    ..RoomPreference::default()
                },
            ),
            (
                "notification".to_owned(),
                RoomPreference {
                    notification_mode: Some(RoomNotificationMode::Mentions),
                    ..RoomPreference::default()
                },
            ),
        ]),
    };
    let preference_effects = reduce(&mut state, AppAction::RoomPreferencesLoaded { preferences });
    assert!(preference_effects.contains(&AppEffect::EmitUiEvent(UiEvent::RoomListChanged)));
    assert_eq!(
        state
            .room_list
            .items
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["ordinary", "notification", "mention", "selected"]
    );

    let settings = state.settings.values.clone();
    reduce(&mut state, AppAction::SettingsLoaded { values: settings });
    assert_eq!(
        state
            .room_list
            .items
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["ordinary", "notification", "mention", "selected"]
    );
    assert_eq!(state.navigation.active_room_id.as_deref(), Some("selected"));
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
    assert_eq!(
        state.room_preferences.rooms.get("!known:example.invalid"),
        Some(&RoomPreference {
            notification_mode: Some(RoomNotificationMode::Mute),
            ..RoomPreference::default()
        })
    );
    assert!(effects.iter().any(|effect| matches!(
        effect,
        AppEffect::PersistRoomPreferences {
            request_id: 1,
            preferences
        } if preferences == &state.room_preferences
    )));
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

#[test]
fn server_mute_suppresses_existing_highlights_without_marking_read() {
    let mut state = ready_state();
    state.rooms[0].unread_count = 5;
    state.rooms[0].notification_count = 5;
    state.rooms[0].highlight_count = 2;
    let effects = reduce(
        &mut state,
        AppAction::RoomNotificationModesObserved {
            generation: 0,
            source: koushi_state::RoomListSource::Cache,
            modes: [(
                "!known:example.invalid".to_owned(),
                RoomNotificationMode::Mute,
            )]
            .into(),
        },
    );
    assert_eq!(
        state
            .room_notification_settings
            .get("!known:example.invalid")
            .map(|s| s.mode),
        Some(RoomNotificationMode::Mute)
    );
    assert_eq!(state.rooms[0].unread_count, 5);
    assert_eq!(state.rooms[0].highlight_count, 2);
    assert_eq!(state.native_attention.summary.badge_count, 0);
    assert!(state.native_attention.summary.candidate.is_none());
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, AppEffect::EmitUiEvent(UiEvent::RoomListChanged)))
    );
}

#[test]
fn server_modes_ignore_stale_generations_and_pending_writes_then_reconcile_silently() {
    let mut state = ready_state();
    state.rooms[0].unread_count = 5;
    state.rooms[0].notification_count = 5;
    state.room_list.readiness = koushi_state::RoomListReadiness::Ready {
        generation: 3,
        source: koushi_state::RoomListSource::Live,
    };
    let observe = |generation, mode| AppAction::RoomNotificationModesObserved {
        generation,
        source: koushi_state::RoomListSource::Live,
        modes: [("!known:example.invalid".to_owned(), mode)].into(),
    };
    reduce(&mut state, observe(2, RoomNotificationMode::Mute));
    assert!(state.room_notification_settings.is_empty());
    reduce(
        &mut state,
        AppAction::RoomNotificationModeSet {
            request_id: 42,
            room_id: "!known:example.invalid".into(),
            mode: RoomNotificationMode::Mute,
        },
    );
    reduce(&mut state, observe(3, RoomNotificationMode::All));
    assert_eq!(
        state.room_notification_settings["!known:example.invalid"].mode,
        RoomNotificationMode::Mute
    );
    reduce(
        &mut state,
        AppAction::RoomNotificationModeCompleted {
            request_id: 42,
            room_id: "!known:example.invalid".into(),
        },
    );
    reduce(&mut state, observe(3, RoomNotificationMode::All));
    assert_eq!(
        state.room_notification_settings["!known:example.invalid"].mode,
        RoomNotificationMode::Mute
    );
    assert_eq!(state.native_attention.summary.badge_count, 0);
    reduce(&mut state, observe(3, RoomNotificationMode::Mute));
    reduce(&mut state, observe(3, RoomNotificationMode::All));
    assert_eq!(
        state.room_notification_settings["!known:example.invalid"].mode,
        RoomNotificationMode::All
    );
    assert!(state.native_attention.summary.candidate.is_none());
    assert_eq!(state.rooms[0].unread_count, 5);
}

#[test]
fn authoritative_policy_resolves_conflicting_rule_and_fresh_sync_releases_echo_fence() {
    let mut state = ready_state();
    let room_id = "!known:example.invalid".to_owned();
    state.room_list.readiness = koushi_state::RoomListReadiness::Ready {
        generation: 3,
        source: koushi_state::RoomListSource::Live,
    };
    reduce(
        &mut state,
        AppAction::RoomNotificationModeSet {
            request_id: 42,
            room_id: room_id.clone(),
            mode: RoomNotificationMode::All,
        },
    );
    reduce(
        &mut state,
        AppAction::RoomNotificationModeCompleted {
            request_id: 42,
            room_id: room_id.clone(),
        },
    );
    reduce(
        &mut state,
        AppAction::RoomNotificationModeConfirmed {
            request_id: 41,
            room_id: room_id.clone(),
            mode: RoomNotificationMode::Mute,
        },
    );
    assert_eq!(
        state.room_notification_settings[&room_id].mode,
        RoomNotificationMode::All
    );
    reduce(
        &mut state,
        AppAction::RoomNotificationModeConfirmed {
            request_id: 42,
            room_id: room_id.clone(),
            mode: RoomNotificationMode::Mute,
        },
    );
    assert_eq!(
        state.room_notification_settings[&room_id].mode,
        RoomNotificationMode::Mute
    );
    let observe = || AppAction::RoomNotificationModesObserved {
        generation: 3,
        source: koushi_state::RoomListSource::Live,
        modes: [(room_id.clone(), RoomNotificationMode::All)].into(),
    };
    reduce(&mut state, observe());
    assert_eq!(
        state.room_notification_settings[&room_id].mode,
        RoomNotificationMode::Mute
    );
    reduce(
        &mut state,
        AppAction::RoomNotificationPolicySynced { generation: 2 },
    );
    assert!(!state.room_notification_awaiting_echo.is_empty());
    reduce(
        &mut state,
        AppAction::RoomNotificationPolicySynced { generation: 3 },
    );
    reduce(&mut state, observe());
    assert_eq!(
        state.room_notification_settings[&room_id].mode,
        RoomNotificationMode::All
    );
    assert!(state.native_attention.summary.candidate.is_none());
}
