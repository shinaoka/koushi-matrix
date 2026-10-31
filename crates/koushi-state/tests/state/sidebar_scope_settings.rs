use koushi_state::{
    RoomListSort, SettingsPatch, SettingsValues, SidebarSectionKind, SidebarSectionPatch,
    SidebarSectionSettings,
};

#[test]
fn first_section_patch_inherits_global_sort_and_preserves_existing_preferences() {
    for scope in ["__home__", "!space:example.invalid"] {
        for global in [RoomListSort::RecentFirst, RoomListSort::NormalLocale] {
            for section in [SidebarSectionKind::Rooms, SidebarSectionKind::Dms] {
                for sort in [None, Some(RoomListSort::Activity)] {
                    let mut values = SettingsValues::default();
                    // The global sort in the same patch must take effect before inheritance.
                    values.apply_patch(SettingsPatch {
                        room_list_sort: Some(global),
                        sidebar_section: Some(SidebarSectionPatch {
                            scope: scope.into(),
                            section,
                            collapsed: Some(true),
                            sort,
                            expanded: None,
                        }),
                        ..Default::default()
                    });
                    let prefs = values.sidebar.scope(Some(scope), global);
                    let (changed, untouched) = match section {
                        SidebarSectionKind::Rooms => (prefs.rooms, prefs.dms),
                        SidebarSectionKind::Dms => (prefs.dms, prefs.rooms),
                        SidebarSectionKind::LowPriority => {
                            unreachable!("Low priority has no independent sort; see its own test")
                        }
                    };
                    assert!(changed.collapsed);
                    assert_eq!(changed.sort, sort.unwrap_or(global));
                    assert!(!untouched.collapsed);
                    assert_eq!(untouched.sort, global);
                    values.apply_patch(SettingsPatch {
                        room_list_sort: Some(RoomListSort::Activity),
                        sidebar_section: Some(SidebarSectionPatch {
                            scope: scope.into(),
                            section,
                            collapsed: Some(true),
                            sort: None,
                            expanded: None,
                        }),
                        ..Default::default()
                    });
                    assert_eq!(
                        values.sidebar.scope(Some(scope), RoomListSort::Activity),
                        prefs
                    );
                    let loaded: SettingsValues =
                        serde_json::from_str(&serde_json::to_string(&values).unwrap()).unwrap();
                    assert_eq!(loaded, values);
                    assert_eq!(
                        loaded.sidebar.scope(Some(scope), RoomListSort::Activity),
                        prefs
                    );
                }
            }
        }
    }
}

/// #955: the Low priority section stores only a per-scope collapse choice. It
/// has no independent sort and inherits the legacy device-global flag until the
/// scope is edited.
#[test]
fn low_priority_section_patch_stores_collapse_and_follows_the_rooms_sort() {
    for scope in ["__home__", "!space:example.invalid"] {
        let mut values = SettingsValues::default();
        values.sidebar.collapsed.low_priority = true;
        assert_eq!(
            values.sidebar.scope(Some(scope), RoomListSort::RecentFirst),
            koushi_state::SidebarScopeSettings {
                rooms: SidebarSectionSettings {
                    collapsed: false,
                    sort: RoomListSort::RecentFirst,
                    expanded: false,
                },
                dms: SidebarSectionSettings {
                    collapsed: false,
                    sort: RoomListSort::RecentFirst,
                    expanded: false,
                },
                low_priority: Some(SidebarSectionSettings {
                    collapsed: true,
                    sort: RoomListSort::RecentFirst,
                    expanded: false,
                }),
            },
            "an unedited scope inherits the legacy collapse flag"
        );

        values.apply_patch(SettingsPatch {
            room_list_sort: Some(RoomListSort::NormalLocale),
            sidebar_section: Some(SidebarSectionPatch {
                scope: scope.into(),
                section: SidebarSectionKind::LowPriority,
                collapsed: Some(false),
                sort: Some(RoomListSort::Activity),
                expanded: None,
            }),
            ..Default::default()
        });
        let prefs = values
            .sidebar
            .scope(Some(scope), RoomListSort::NormalLocale);
        assert_eq!(
            prefs.low_priority,
            Some(SidebarSectionSettings {
                collapsed: false,
                sort: RoomListSort::NormalLocale,
                expanded: false,
            }),
            "the scoped collapse wins and the sort follows Rooms"
        );
        assert!(!prefs.rooms.collapsed);
        assert!(!prefs.dms.collapsed);

        values.apply_patch(SettingsPatch {
            sidebar_section: Some(SidebarSectionPatch {
                scope: scope.into(),
                section: SidebarSectionKind::Rooms,
                collapsed: None,
                sort: Some(RoomListSort::Activity),
                expanded: None,
            }),
            ..Default::default()
        });
        assert_eq!(
            values
                .sidebar
                .scope(Some(scope), RoomListSort::NormalLocale)
                .low_priority
                .map(|section| section.sort),
            Some(RoomListSort::Activity),
            "a Rooms sort change reorders Low priority too"
        );

        let loaded: SettingsValues =
            serde_json::from_str(&serde_json::to_string(&values).unwrap()).unwrap();
        assert_eq!(loaded, values);
    }
}

/// A settings file written before #955 has no `low_priority` scope entry and
/// must keep its legacy collapse choice after load.
#[test]
fn a_legacy_scope_preference_without_low_priority_deserializes_and_falls_back() {
    let sidebar: koushi_state::SidebarSettings = serde_json::from_value(serde_json::json!({
        "collapsed": { "favourites": false, "low_priority": true, "not_joined": false },
        "scope_preferences": {
            "__home__": {
                "rooms": { "collapsed": true, "sort": { "kind": "normalLocale" } },
                "dms": { "collapsed": false, "sort": { "kind": "recentFirst" } }
            }
        }
    }))
    .expect("a pre-#955 settings file must still load");

    let prefs = sidebar.scope(None, RoomListSort::Activity);
    assert!(prefs.rooms.collapsed);
    assert_eq!(prefs.rooms.sort, RoomListSort::NormalLocale);
    assert_eq!(
        prefs.low_priority,
        Some(SidebarSectionSettings {
            collapsed: true,
            sort: RoomListSort::NormalLocale,
            expanded: false,
        })
    );
}
