use koushi_state::{
    AppAction, AppEffect, AppState, AppearanceSettings, ComposerSendShortcut, DisplaySettings,
    EmojiPreference, FontPreference, ImageUploadCompressionMode, KeyboardSettings, LocaleSettings,
    MediaSettings, NotificationSettings, RoomListSort, RoomSummary, SettingsPatch,
    SettingsPersistenceState, SettingsValues, TextDirectionPreference, ThemePreference,
    ThreadListOrder, TimelineSettings, TimelineThreadRootOrder, UiEvent, reduce,
};

fn dark_theme_patch() -> SettingsPatch {
    SettingsPatch {
        appearance: Some(AppearanceSettings {
            theme: ThemePreference::Dark,
        }),
        ..SettingsPatch::default()
    }
}

#[test]
fn app_state_carries_default_non_secret_settings() {
    let state = AppState::default();

    assert_eq!(
        state.settings.values.appearance.theme,
        ThemePreference::System
    );
    assert_eq!(
        state.settings.values.keyboard.composer_send_shortcut,
        ComposerSendShortcut::Enter
    );
    assert_eq!(state.settings.values.locale.language_tag, None);
    assert_eq!(
        state.settings.values.locale.text_direction,
        TextDirectionPreference::Auto
    );
    assert_eq!(
        state.settings.values.typography.font,
        FontPreference::System
    );
    assert_eq!(
        state.settings.values.typography.emoji,
        EmojiPreference::System
    );
    assert_eq!(
        state.settings.values.notifications,
        NotificationSettings::default()
    );
    assert_eq!(
        state.settings.values.display,
        DisplaySettings {
            code_block_wrap: true,
            hide_redacted: true,
            url_previews_enabled: true,
            encrypted_url_previews_enabled: true,
        }
    );
    assert_eq!(
        state.settings.values.media,
        MediaSettings {
            image_upload_compression: ImageUploadCompressionMode::Ask,
            ..MediaSettings::default()
        }
    );
    assert_eq!(
        state.settings.values.timeline,
        TimelineSettings {
            auto_load_older_messages: true,
            thread_root_order: TimelineThreadRootOrder::RootEvent,
        }
    );
    assert_eq!(state.settings.persistence, SettingsPersistenceState::Idle);
}

#[test]
fn settings_loaded_replaces_values_without_requiring_a_session() {
    let mut state = AppState::default();
    let values = SettingsValues {
        locale: LocaleSettings {
            language_tag: Some("ja-JP".to_owned()),
            text_direction: TextDirectionPreference::Auto,
        },
        appearance: AppearanceSettings {
            theme: ThemePreference::Light,
        },
        typography: koushi_state::TypographySettings {
            font: FontPreference::Inter,
            emoji: EmojiPreference::TwemojiColr,
        },
        keyboard: KeyboardSettings {
            composer_send_shortcut: ComposerSendShortcut::ModEnter,
        },
        notifications: NotificationSettings {
            desktop_notifications: false,
            sound: false,
            badges: true,
            send_read_receipts: true,
            send_typing_notifications: true,
        },
        display: DisplaySettings {
            code_block_wrap: false,
            hide_redacted: true,
            url_previews_enabled: true,
            encrypted_url_previews_enabled: true,
        },
        media: MediaSettings {
            image_upload_compression: ImageUploadCompressionMode::Always,
            ..MediaSettings::default()
        },
        timeline: TimelineSettings {
            auto_load_older_messages: true,
            thread_root_order: TimelineThreadRootOrder::RootEvent,
        },
        thread_list_order: ThreadListOrder::LatestReply,
        room_list_sort: RoomListSort::Activity,
        search_crawler: koushi_state::SearchCrawlerSettings::default(),
    };

    let effects = reduce(
        &mut state,
        AppAction::SettingsLoaded {
            values: values.clone(),
        },
    );

    assert_eq!(state.settings.values, values);
    assert_eq!(
        effects,
        vec![
            AppEffect::EmitUiEvent(UiEvent::SettingsChanged),
            AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
        ]
    );
}

#[test]
fn settings_values_deserialize_empty_display_as_default() {
    let values = serde_json::from_str::<SettingsValues>(
        r#"{
  "locale": { "language_tag": null, "text_direction": "auto" },
  "appearance": { "theme": "system" },
  "typography": { "font": "system", "emoji": "system" },
  "keyboard": { "composer_send_shortcut": "enter" },
  "notifications": { "desktop_notifications": true, "sound": true, "badges": true },
  "display": {}
}
"#,
    )
    .expect("empty display object should deserialize");

    assert_eq!(values.display, DisplaySettings::default());
}

#[test]
fn settings_values_deserialize_legacy_display_without_hide_redacted_as_default_on() {
    let values = serde_json::from_str::<SettingsValues>(
        r#"{
  "locale": { "language_tag": null, "text_direction": "auto" },
  "appearance": { "theme": "system" },
  "typography": { "font": "system", "emoji": "system" },
  "keyboard": { "composer_send_shortcut": "enter" },
  "notifications": { "desktop_notifications": true, "sound": true, "badges": true },
  "display": { "code_block_wrap": false }
}
"#,
    )
    .expect("legacy display object should deserialize");

    assert_eq!(
        values.display,
        DisplaySettings {
            code_block_wrap: false,
            hide_redacted: true,
            url_previews_enabled: true,
            encrypted_url_previews_enabled: true,
        }
    );
}

#[test]
fn display_settings_deserialize_legacy_without_url_previews_as_defaults() {
    let display = serde_json::from_str::<DisplaySettings>(
        r#"{ "code_block_wrap": true, "hide_redacted": false }"#,
    )
    .expect("legacy display object should deserialize");

    assert!(display.url_previews_enabled);
    assert!(display.encrypted_url_previews_enabled);
}

#[test]
fn settings_values_deserialize_legacy_without_media_as_default_ask() {
    let values = serde_json::from_str::<SettingsValues>(
        r#"{
  "locale": { "language_tag": null, "text_direction": "auto" },
  "appearance": { "theme": "system" },
  "typography": { "font": "system", "emoji": "system" },
  "keyboard": { "composer_send_shortcut": "enter" },
  "notifications": { "desktop_notifications": true, "sound": true, "badges": true },
  "display": { "code_block_wrap": true, "hide_redacted": false }
}
"#,
    )
    .expect("legacy settings without media should deserialize");

    assert_eq!(
        values.media,
        MediaSettings {
            image_upload_compression: ImageUploadCompressionMode::Ask,
            ..MediaSettings::default()
        }
    );
}

#[test]
fn settings_values_deserialize_legacy_without_timeline_as_default_true() {
    let values = serde_json::from_str::<SettingsValues>(
        r#"{
  "locale": { "language_tag": null, "text_direction": "auto" },
  "appearance": { "theme": "system" },
  "typography": { "font": "system", "emoji": "system" },
  "keyboard": { "composer_send_shortcut": "enter" },
  "notifications": { "desktop_notifications": true, "sound": true, "badges": true },
  "display": { "code_block_wrap": true, "hide_redacted": true },
  "media": { "image_upload_compression": "ask" }
}
"#,
    )
    .expect("legacy settings without timeline should deserialize");

    assert_eq!(
        values.timeline,
        TimelineSettings {
            auto_load_older_messages: true,
            thread_root_order: TimelineThreadRootOrder::RootEvent,
        }
    );
}

#[test]
fn timeline_auto_load_older_messages_defaults_to_true() {
    let values = koushi_state::SettingsValues::default();
    assert!(values.timeline.auto_load_older_messages);
}

#[test]
fn timeline_thread_root_order_defaults_to_root_event() {
    assert_eq!(
        TimelineSettings::default().thread_root_order,
        TimelineThreadRootOrder::RootEvent
    );
}

#[test]
fn timeline_thread_root_order_patch_accepts_latest_reply_and_legacy_settings_default_to_root_event()
{
    let mut state = AppState::default();
    reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 89,
            patch: SettingsPatch {
                timeline: Some(TimelineSettings {
                    thread_root_order: TimelineThreadRootOrder::LatestReply,
                    ..TimelineSettings::default()
                }),
                ..SettingsPatch::default()
            },
        },
    );

    assert_eq!(
        state.settings.values.timeline.thread_root_order,
        TimelineThreadRootOrder::LatestReply
    );

    let legacy_timeline =
        serde_json::from_str::<TimelineSettings>(r#"{ "auto_load_older_messages": true }"#)
            .expect("legacy timeline settings should deserialize");
    assert_eq!(
        legacy_timeline.thread_root_order,
        TimelineThreadRootOrder::RootEvent
    );
}

#[test]
fn missing_timeline_settings_backfill_auto_load_to_true() {
    let json = r#"{
      "locale": {"language_tag": null, "text_direction": "auto"},
      "appearance": {"theme": "system"},
      "typography": {"font": "system", "emoji": "system"},
      "keyboard": {"composer_send_shortcut": "enter"},
      "notifications": {"desktop_notifications": true, "sound": true, "badges": true},
      "display": {"code_block_wrap": true, "hide_redacted": true},
      "media": {"image_upload_compression": "ask"}
    }"#;
    let values: koushi_state::SettingsValues = serde_json::from_str(json).unwrap();
    assert!(values.timeline.auto_load_older_messages);
}

#[test]
fn explicit_false_auto_load_older_messages_is_preserved() {
    let json = r#"{
      "locale": {"language_tag": null, "text_direction": "auto"},
      "appearance": {"theme": "system"},
      "typography": {"font": "system", "emoji": "system"},
      "keyboard": {"composer_send_shortcut": "enter"},
      "notifications": {"desktop_notifications": true, "sound": true, "badges": true},
      "display": {"code_block_wrap": true, "hide_redacted": true},
      "media": {"image_upload_compression": "ask"},
      "timeline": {"auto_load_older_messages": false}
    }"#;
    let values: koushi_state::SettingsValues = serde_json::from_str(json).unwrap();
    assert!(!values.timeline.auto_load_older_messages);
}

#[test]
fn notification_settings_patch_is_rust_owned_and_persisted() {
    let mut state = AppState::default();
    let notification_settings = NotificationSettings {
        desktop_notifications: false,
        sound: false,
        badges: false,
        send_read_receipts: false,
        send_typing_notifications: false,
    };

    let effects = reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 77,
            patch: SettingsPatch {
                notifications: Some(notification_settings.clone()),
                ..SettingsPatch::default()
            },
        },
    );

    assert_eq!(state.settings.values.notifications, notification_settings);
    assert_eq!(
        state.settings.persistence,
        SettingsPersistenceState::Saving { request_id: 77 }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::PersistSettings {
                request_id: 77,
                values: state.settings.values.clone(),
            },
            AppEffect::EmitUiEvent(UiEvent::SettingsChanged),
        ]
    );
}

#[test]
fn code_block_wrap_patch_is_rust_owned_and_persisted() {
    let mut state = AppState::default();
    let display_settings = DisplaySettings {
        code_block_wrap: false,
        hide_redacted: false,
        url_previews_enabled: true,
        encrypted_url_previews_enabled: false,
    };

    let effects = reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 78,
            patch: SettingsPatch {
                display: Some(display_settings.clone()),
                ..SettingsPatch::default()
            },
        },
    );

    assert_eq!(state.settings.values.display, display_settings);
    assert_eq!(
        state.settings.persistence,
        SettingsPersistenceState::Saving { request_id: 78 }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::PersistSettings {
                request_id: 78,
                values: state.settings.values.clone(),
            },
            AppEffect::EmitUiEvent(UiEvent::SettingsChanged),
        ]
    );
}

#[test]
fn hide_redacted_patch_is_rust_owned_and_persisted() {
    let mut state = AppState::default();
    let display_settings = DisplaySettings {
        code_block_wrap: true,
        hide_redacted: true,
        url_previews_enabled: true,
        encrypted_url_previews_enabled: false,
    };

    let effects = reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 79,
            patch: SettingsPatch {
                display: Some(display_settings.clone()),
                ..SettingsPatch::default()
            },
        },
    );

    assert_eq!(state.settings.values.display, display_settings);
    assert_eq!(
        state.settings.persistence,
        SettingsPersistenceState::Saving { request_id: 79 }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::PersistSettings {
                request_id: 79,
                values: state.settings.values.clone(),
            },
            AppEffect::EmitUiEvent(UiEvent::SettingsChanged),
        ]
    );
}

#[test]
fn image_upload_compression_patch_is_rust_owned_and_persisted() {
    let mut state = AppState::default();
    let media_settings = MediaSettings {
        image_upload_compression: ImageUploadCompressionMode::Ask,
        ..MediaSettings::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 80,
            patch: SettingsPatch {
                media: Some(media_settings.clone()),
                ..SettingsPatch::default()
            },
        },
    );

    assert_eq!(state.settings.values.media, media_settings);
    assert_eq!(
        state.settings.persistence,
        SettingsPersistenceState::Saving { request_id: 80 }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::PersistSettings {
                request_id: 80,
                values: state.settings.values.clone(),
            },
            AppEffect::EmitUiEvent(UiEvent::SettingsChanged),
        ]
    );
}

#[test]
fn timeline_auto_load_patch_is_rust_owned_and_persisted() {
    let mut state = AppState::default();
    let timeline_settings = TimelineSettings {
        auto_load_older_messages: true,
        thread_root_order: TimelineThreadRootOrder::RootEvent,
    };

    let effects = reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 88,
            patch: SettingsPatch {
                timeline: Some(timeline_settings.clone()),
                ..SettingsPatch::default()
            },
        },
    );

    assert_eq!(state.settings.values.timeline, timeline_settings);
    assert_eq!(
        state.settings.persistence,
        SettingsPersistenceState::Saving { request_id: 88 }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::PersistSettings {
                request_id: 88,
                values: state.settings.values.clone(),
            },
            AppEffect::EmitUiEvent(UiEvent::SettingsChanged),
        ]
    );
}

#[test]
fn settings_update_is_optimistic_and_emits_a_persist_effect() {
    let mut state = AppState::default();

    let effects = reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 42,
            patch: dark_theme_patch(),
        },
    );

    assert_eq!(
        state.settings.values.appearance.theme,
        ThemePreference::Dark
    );
    assert_eq!(
        state.settings.persistence,
        SettingsPersistenceState::Saving { request_id: 42 }
    );
    assert_eq!(
        effects,
        vec![
            AppEffect::PersistSettings {
                request_id: 42,
                values: state.settings.values.clone(),
            },
            AppEffect::EmitUiEvent(UiEvent::SettingsChanged),
        ]
    );
}

#[test]
fn thread_list_ordering_setting_defaults_to_latest_reply() {
    let values = SettingsValues::default();
    assert_eq!(values.thread_list_order, ThreadListOrder::LatestReply);
}

#[test]
fn room_list_sort_setting_defaults_to_activity_and_supports_recent_and_locale() {
    let values = SettingsValues::default();
    assert_eq!(values.room_list_sort, RoomListSort::Activity);

    let mut state = AppState::default();
    reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 90,
            patch: SettingsPatch {
                room_list_sort: Some(RoomListSort::RecentFirst),
                thread_list_order: Some(ThreadListOrder::RootChronology),
                ..SettingsPatch::default()
            },
        },
    );
    assert_eq!(
        state.settings.values.room_list_sort,
        RoomListSort::RecentFirst
    );
    assert_eq!(
        state.settings.values.thread_list_order,
        ThreadListOrder::RootChronology
    );
}

#[test]
fn settings_persist_settle_requires_matching_request_id() {
    let mut state = AppState::default();

    reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 7,
            patch: dark_theme_patch(),
        },
    );

    let stale = reduce(&mut state, AppAction::SettingsPersisted { request_id: 999 });
    assert_eq!(stale, Vec::new());
    assert_eq!(
        state.settings.persistence,
        SettingsPersistenceState::Saving { request_id: 7 }
    );

    let matched = reduce(&mut state, AppAction::SettingsPersisted { request_id: 7 });
    assert_eq!(state.settings.persistence, SettingsPersistenceState::Idle);
    assert_eq!(
        matched,
        vec![AppEffect::EmitUiEvent(UiEvent::SettingsChanged)]
    );
}

#[test]
fn settings_load_and_persist_failures_are_private_data_free() {
    let mut state = AppState::default();

    reduce(
        &mut state,
        AppAction::SettingsLoadFailed {
            message: "settings file is corrupt".to_owned(),
        },
    );
    assert_eq!(state.settings.values, SettingsValues::default());
    assert_eq!(state.errors[0].code, "settings_load_failed");
    assert!(!state.errors[0].message.contains("@"));

    reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 3,
            patch: dark_theme_patch(),
        },
    );
    reduce(
        &mut state,
        AppAction::SettingsPersistFailed {
            request_id: 3,
            message: "settings file could not be saved".to_owned(),
        },
    );
    assert_eq!(state.settings.persistence, SettingsPersistenceState::Idle);
    assert!(
        state
            .errors
            .iter()
            .any(|error| error.code == "settings_persist_failed")
    );
}

fn test_room(room_id: &str, display_name: &str, last_activity_ms: u64) -> RoomSummary {
    RoomSummary {
        room_id: room_id.to_owned(),
        display_name: display_name.to_owned(),
        display_label: display_name.to_owned(),
        original_display_label: display_name.to_owned(),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: koushi_state::RoomTags::default(),
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        marked_unread: false,
        last_activity_ms,
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 0,
    }
}

#[test]
fn settings_loaded_recomputes_room_list_projection_and_sorts_open_threads() {
    let mut state = AppState::default();
    state.rooms = vec![
        test_room("!alpha:example.invalid", "Alpha", 100),
        test_room("!beta:example.invalid", "Beta", 200),
    ];
    state.threads_list = koushi_state::ThreadsListState::Open {
        room_id: "!alpha:example.invalid".to_owned(),
        request_id: 1,
        items: vec![
            koushi_state::ThreadsListItem {
                root_event_id: "$latest:example.invalid".to_owned(),
                root_sender: "@bob:example.invalid".to_owned(),
                root_sender_label: None,
                root_body_preview: None,
                root_timestamp_ms: Some(200),
                latest_event_id: Some("$latest:example.invalid".to_owned()),
                latest_sender: Some("@bob:example.invalid".to_owned()),
                latest_sender_label: None,
                latest_body_preview: None,
                latest_timestamp_ms: Some(200),
                reply_count: 0,
            },
            koushi_state::ThreadsListItem {
                root_event_id: "$older:example.invalid".to_owned(),
                root_sender: "@bob:example.invalid".to_owned(),
                root_sender_label: None,
                root_body_preview: None,
                root_timestamp_ms: Some(100),
                latest_event_id: Some("$older:example.invalid".to_owned()),
                latest_sender: Some("@bob:example.invalid".to_owned()),
                latest_sender_label: None,
                latest_body_preview: None,
                latest_timestamp_ms: Some(100),
                reply_count: 0,
            },
        ],
        is_paginating: false,
        end_reached: true,
    };

    let values = SettingsValues {
        room_list_sort: RoomListSort::NormalLocale,
        thread_list_order: ThreadListOrder::RootChronology,
        ..SettingsValues::default()
    };
    let effects = reduce(
        &mut state,
        AppAction::SettingsLoaded {
            values: values.clone(),
        },
    );

    assert_eq!(state.settings.values, values);
    assert!(
        effects.contains(&AppEffect::EmitUiEvent(UiEvent::RoomListChanged)),
        "expected RoomListChanged after settings load"
    );
    assert!(
        effects.contains(&AppEffect::EmitUiEvent(UiEvent::ThreadsListChanged)),
        "expected ThreadsListChanged after settings load"
    );
    assert_eq!(
        state
            .room_list
            .items
            .iter()
            .map(|item| item.room_id.clone())
            .collect::<Vec<_>>(),
        vec!["!alpha:example.invalid", "!beta:example.invalid"]
    );
    assert_eq!(
        state
            .threads_list
            .items()
            .iter()
            .map(|item| item.root_event_id.clone())
            .collect::<Vec<_>>(),
        vec!["$older:example.invalid", "$latest:example.invalid"]
    );
}

#[test]
fn settings_update_recomputes_room_list_projection_and_resorts_open_threads() {
    let mut state = AppState::default();
    state.rooms = vec![
        test_room("!alpha:example.invalid", "Alpha", 100),
        test_room("!beta:example.invalid", "Beta", 200),
    ];
    state.threads_list = koushi_state::ThreadsListState::Open {
        room_id: "!alpha:example.invalid".to_owned(),
        request_id: 1,
        items: vec![
            koushi_state::ThreadsListItem {
                root_event_id: "$latest:example.invalid".to_owned(),
                root_sender: "@bob:example.invalid".to_owned(),
                root_sender_label: None,
                root_body_preview: None,
                root_timestamp_ms: Some(200),
                latest_event_id: Some("$latest:example.invalid".to_owned()),
                latest_sender: Some("@bob:example.invalid".to_owned()),
                latest_sender_label: None,
                latest_body_preview: None,
                latest_timestamp_ms: Some(200),
                reply_count: 0,
            },
            koushi_state::ThreadsListItem {
                root_event_id: "$older:example.invalid".to_owned(),
                root_sender: "@bob:example.invalid".to_owned(),
                root_sender_label: None,
                root_body_preview: None,
                root_timestamp_ms: Some(100),
                latest_event_id: Some("$older:example.invalid".to_owned()),
                latest_sender: Some("@bob:example.invalid".to_owned()),
                latest_sender_label: None,
                latest_body_preview: None,
                latest_timestamp_ms: Some(100),
                reply_count: 0,
            },
        ],
        is_paginating: false,
        end_reached: true,
    };

    reduce(
        &mut state,
        AppAction::SettingsLoaded {
            values: SettingsValues::default(),
        },
    );
    assert_eq!(
        state
            .room_list
            .items
            .iter()
            .map(|item| item.room_id.clone())
            .collect::<Vec<_>>(),
        vec!["!beta:example.invalid", "!alpha:example.invalid"]
    );

    let effects = reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 42,
            patch: SettingsPatch {
                room_list_sort: Some(RoomListSort::NormalLocale),
                thread_list_order: Some(ThreadListOrder::RootChronology),
                ..SettingsPatch::default()
            },
        },
    );

    assert!(
        effects.contains(&AppEffect::EmitUiEvent(UiEvent::RoomListChanged)),
        "expected RoomListChanged after sort settings update"
    );
    assert!(
        effects.contains(&AppEffect::EmitUiEvent(UiEvent::ThreadsListChanged)),
        "expected ThreadsListChanged after thread order settings update"
    );
    assert_eq!(
        state
            .room_list
            .items
            .iter()
            .map(|item| item.room_id.clone())
            .collect::<Vec<_>>(),
        vec!["!alpha:example.invalid", "!beta:example.invalid"]
    );
    assert_eq!(
        state
            .threads_list
            .items()
            .iter()
            .map(|item| item.root_event_id.clone())
            .collect::<Vec<_>>(),
        vec!["$older:example.invalid", "$latest:example.invalid"]
    );
}

#[test]
fn settings_update_without_sort_changes_does_not_emit_room_or_threads_list_events() {
    let mut state = AppState::default();
    state.rooms = vec![
        test_room("!alpha:example.invalid", "Alpha", 100),
        test_room("!beta:example.invalid", "Beta", 200),
    ];

    reduce(
        &mut state,
        AppAction::SettingsLoaded {
            values: SettingsValues::default(),
        },
    );

    let effects = reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 43,
            patch: SettingsPatch {
                appearance: Some(AppearanceSettings {
                    theme: ThemePreference::Dark,
                }),
                ..SettingsPatch::default()
            },
        },
    );

    assert!(
        !effects.contains(&AppEffect::EmitUiEvent(UiEvent::RoomListChanged)),
        "expected no RoomListChanged when sort is unchanged"
    );
    assert!(
        !effects.contains(&AppEffect::EmitUiEvent(UiEvent::ThreadsListChanged)),
        "expected no ThreadsListChanged when order is unchanged"
    );
}
