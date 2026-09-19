use koushi_state::{
    RoomListSort, SettingsPatch, SettingsValues, SidebarSectionKind, SidebarSectionPatch,
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
                        }),
                        ..Default::default()
                    });
                    let prefs = values.sidebar.scope(Some(scope), global);
                    let (changed, untouched) = match section {
                        SidebarSectionKind::Rooms => (prefs.rooms, prefs.dms),
                        SidebarSectionKind::Dms => (prefs.dms, prefs.rooms),
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
