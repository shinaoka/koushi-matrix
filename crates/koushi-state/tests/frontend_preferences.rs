use std::collections::BTreeMap;

use koushi_state::{
    AppState, AppearanceSettings, ConversationActivity, ConversationActivitySource, DisplayDensity,
    HomeSelection, NavigationState, RoomListSort, RoomSummary, RoomTagInfo, RoomTags,
    SettingsPatch, SettingsValues, SidebarCategory, SidebarCollapsedSections, SidebarScopeSettings,
    SidebarSectionSettings, SidebarSettings, SpaceLocalPresentation, SpaceLocalPresentations,
    SpaceSummary, compose_sidebar_for_state,
};

fn room(id: &str, label: &str, is_dm: bool, tags: RoomTags, timestamp_ms: u64) -> RoomSummary {
    RoomSummary {
        room_id: id.to_owned(),
        display_name: label.to_owned(),
        display_label: label.to_owned(),
        original_display_label: label.to_owned(),
        avatar: None,
        is_dm,
        dm_user_ids: Vec::new(),
        tags,
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
        parent_space_ids: vec!["!space:example.invalid".to_owned()],
        dm_space_ids: is_dm
            .then(|| "!space:example.invalid".to_owned())
            .into_iter()
            .collect(),
        is_encrypted: true,
        joined_members: 2,
    }
}

fn favourite() -> RoomTags {
    RoomTags {
        favourite: Some(RoomTagInfo { order: None }),
        low_priority: None,
    }
}

fn low_priority() -> RoomTags {
    RoomTags {
        favourite: None,
        low_priority: Some(RoomTagInfo { order: None }),
    }
}

#[test]
fn frontend_preference_defaults_and_legacy_json_backfill_match_existing_behavior() {
    let defaults = SettingsValues::default();
    assert_eq!(defaults.appearance.density, DisplayDensity::Comfortable);
    assert_eq!(defaults.sidebar.category, SidebarCategory::Rooms);
    assert_eq!(
        defaults.sidebar.collapsed,
        SidebarCollapsedSections::default()
    );
    assert!(defaults.composer.recent_emojis.is_empty());

    let legacy: SettingsValues = serde_json::from_value(serde_json::json!({
        "locale": { "language_tag": null, "text_direction": "auto" },
        "appearance": { "theme": "system" },
        "typography": { "font": "system", "emoji": "system" },
        "keyboard": { "composer_send_shortcut": "enter" }
    }))
    .expect("legacy settings should backfill new preferences");
    assert_eq!(legacy.appearance.density, DisplayDensity::Comfortable);
    assert_eq!(legacy.sidebar.category, SidebarCategory::Rooms);
    assert!(legacy.composer.recent_emojis.is_empty());
}

#[test]
fn recent_emoji_patch_is_canonical_distinct_and_bounded() {
    let mut values = SettingsValues::default();
    let mut recent = vec!["😀".to_owned(), "😀".to_owned(), "🚀".to_owned()];
    recent.extend((0..30).map(|index| format!("emoji-{index}")));
    values.apply_patch(SettingsPatch {
        composer: Some(koushi_state::ComposerSettings {
            math_mode: true,
            recent_emojis: recent,
        }),
        ..SettingsPatch::default()
    });

    assert_eq!(&values.composer.recent_emojis[..2], ["😀", "🚀"]);
    assert_eq!(values.composer.recent_emojis.len(), 24);
}

#[test]
fn settings_debug_redacts_recent_emoji_values() {
    let mut values = SettingsValues::default();
    values.composer.recent_emojis = vec!["private-emoji-value".to_owned()];
    let debug = format!("{values:?}");
    assert!(!debug.contains("private-emoji-value"), "{debug}");
    assert!(debug.contains("recent_emoji_count"), "{debug}");
}

#[test]
fn navigation_debug_redacts_every_identifier_and_local_presentation_value() {
    let navigation = NavigationState {
        active_space_id: Some("!active-space:example.invalid".to_owned()),
        active_room_id: Some("!active-room:example.invalid".to_owned()),
        home_selection: HomeSelection::DirectMessage {
            room_id: "!remembered-dm:example.invalid".to_owned(),
        },
        space_local_presentations: SpaceLocalPresentations(BTreeMap::from([(
            "!private-space:example.invalid".to_owned(),
            SpaceLocalPresentation {
                name: Some("Private project name".to_owned()),
                icon: Some("🔒".to_owned()),
            },
        )])),
        ..NavigationState::default()
    };

    let debug = format!("{navigation:?}");
    for private in [
        "!active-space:example.invalid",
        "!active-room:example.invalid",
        "!remembered-dm:example.invalid",
        "!private-space:example.invalid",
        "Private project name",
        "🔒",
    ] {
        assert!(
            !debug.contains(private),
            "navigation Debug leaked {private}: {debug}"
        );
    }
    assert!(debug.contains("space_local_presentation_count"));
}

#[test]
fn rust_sidebar_projects_complete_sections_order_and_local_space_presentation() {
    let mut state = AppState::default();
    state.navigation.active_space_id = Some("!space:example.invalid".to_owned());
    state.navigation.space_local_presentations = SpaceLocalPresentations(BTreeMap::from([(
        "!space:example.invalid".to_owned(),
        SpaceLocalPresentation {
            name: Some("Local Space".to_owned()),
            icon: Some("🧪".to_owned()),
        },
    )]));
    state.spaces = vec![SpaceSummary {
        space_id: "!space:example.invalid".to_owned(),
        display_name: "Server Space".to_owned(),
        avatar: None,
        child_room_ids: vec![
            "!normal-b:example.invalid".to_owned(),
            "!normal-a:example.invalid".to_owned(),
            "!fav:example.invalid".to_owned(),
            "!low:example.invalid".to_owned(),
        ],
    }];
    state.rooms = vec![
        room(
            "!normal-b:example.invalid",
            "Beta",
            false,
            RoomTags::default(),
            20,
        ),
        room(
            "!normal-a:example.invalid",
            "Alpha",
            false,
            RoomTags::default(),
            10,
        ),
        room("!fav:example.invalid", "Favourite", false, favourite(), 30),
        room("!low:example.invalid", "Low", false, low_priority(), 40),
        room(
            "!dm:example.invalid",
            "Person",
            true,
            RoomTags::default(),
            50,
        ),
        room(
            "!dm-old:example.invalid",
            "Zed",
            true,
            RoomTags::default(),
            5,
        ),
    ];

    state.settings.values.sidebar = SidebarSettings {
        category: SidebarCategory::Rooms,
        collapsed: SidebarCollapsedSections::default(),
        scope_preferences: Default::default(),
    };
    state.settings.values.room_list_sort = RoomListSort::RecentFirst;
    let recent = compose_sidebar_for_state(&state);
    assert_eq!(recent.space_rail[0].display_name, "Local Space");
    assert_eq!(recent.space_rail[0].local_icon.as_deref(), Some("🧪"));
    assert_eq!(
        recent
            .sections
            .rooms
            .iter()
            .map(|item| item.display_name.as_str())
            .collect::<Vec<_>>(),
        ["Beta", "Alpha"]
    );
    assert_eq!(recent.sections.favourites.len(), 1);
    assert_eq!(recent.sections.low_priority.len(), 1);
    assert_eq!(recent.sections.people.len(), 2);

    state.settings.values.room_list_sort = RoomListSort::NormalLocale;
    let by_name = compose_sidebar_for_state(&state);
    assert_eq!(
        by_name
            .sections
            .rooms
            .iter()
            .map(|item| item.display_name.as_str())
            .collect::<Vec<_>>(),
        ["Alpha", "Beta"]
    );

    state.settings.values.room_list_sort = RoomListSort::Activity;
    state.settings.values.sidebar.scope_preferences.insert(
        "!space:example.invalid".to_owned(),
        SidebarScopeSettings {
            rooms: SidebarSectionSettings {
                sort: RoomListSort::NormalLocale,
                ..SidebarSectionSettings::default()
            },
            dms: SidebarSectionSettings {
                sort: RoomListSort::RecentFirst,
                ..SidebarSectionSettings::default()
            },
        },
    );
    let independently_sorted = compose_sidebar_for_state(&state);
    assert_eq!(independently_sorted.rooms_sort, RoomListSort::NormalLocale);
    assert_eq!(independently_sorted.dms_sort, RoomListSort::RecentFirst);
    assert_eq!(
        independently_sorted
            .space_rooms
            .iter()
            .map(|item| item.display_name.as_str())
            .collect::<Vec<_>>(),
        ["Alpha", "Beta", "Favourite", "Low"]
    );
    assert_eq!(
        independently_sorted
            .global_dms
            .iter()
            .map(|item| item.display_name.as_str())
            .collect::<Vec<_>>(),
        ["Person", "Zed"]
    );

    state.spaces.push(SpaceSummary {
        space_id: "!other:example.invalid".to_owned(),
        display_name: "Other".to_owned(),
        avatar: None,
        child_room_ids: Vec::new(),
    });
    state.navigation.space_order = vec![
        "!other:example.invalid".to_owned(),
        "!space:example.invalid".to_owned(),
    ];
    let reordered = compose_sidebar_for_state(&state);
    assert_eq!(
        reordered
            .space_rail
            .iter()
            .map(|space| space.space_id.as_str())
            .collect::<Vec<_>>(),
        ["!other:example.invalid", "!space:example.invalid"]
    );
}

#[test]
fn appearance_patch_accepts_density_without_replacing_theme() {
    let mut values = SettingsValues::default();
    let theme = values.appearance.theme.clone();
    values.apply_patch(SettingsPatch {
        appearance: Some(AppearanceSettings {
            theme: theme.clone(),
            density: DisplayDensity::Compact,
        }),
        ..SettingsPatch::default()
    });
    assert_eq!(values.appearance.theme, theme);
    assert_eq!(values.appearance.density, DisplayDensity::Compact);
}
