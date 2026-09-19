use super::*;
use koushi_state::{
    AppAction, RoomListSort, RoomSummary, SettingsPatch, SettingsValues, SidebarSectionKind,
    SidebarSectionPatch, SpaceSummary, reduce,
};

const SPACE: &str = "!space:example.invalid";
const OTHER: &str = "!other:example.invalid";

fn fixture(active: Option<&str>) -> AppState {
    let mut state = AppState::default();
    state.navigation.active_space_id = active.map(str::to_owned);
    for is_dm in [false, true] {
        for (name, timestamp, unread) in [("A", 1, 0), ("B", 3, 0), ("C", 2, 1)] {
            state.rooms.push(
                serde_json::from_value::<RoomSummary>(serde_json::json!({
                    "room_id": format!("!{name}-{is_dm}:example.invalid"),
                    "display_name": name, "display_label": name, "is_dm": is_dm,
                    "unread_count": unread, "notification_count": unread, "highlight_count": 0,
                    "parent_space_ids": [SPACE], "dm_space_ids": [SPACE],
                    "conversation_activity": {"timestamp_ms": timestamp, "source": "message"}
                }))
                .unwrap(),
            );
        }
    }
    state.spaces = [SPACE, OTHER]
        .into_iter()
        .map(|space_id| SpaceSummary {
            space_id: space_id.into(),
            display_name: "Synthetic Space".into(),
            avatar: None,
            child_room_ids: state
                .rooms
                .iter()
                .filter(|room| !room.is_dm)
                .map(|room| room.room_id.clone())
                .collect(),
        })
        .collect();
    state
}

fn patch(
    scope: Option<&str>,
    section: SidebarSectionKind,
    collapsed: Option<bool>,
    sort: Option<RoomListSort>,
) -> SettingsPatch {
    SettingsPatch {
        sidebar_section: Some(SidebarSectionPatch {
            scope: scope.unwrap_or("__home__").into(),
            section,
            collapsed,
            sort,
        }),
        ..Default::default()
    }
}

fn update(state: &mut AppState, patch: SettingsPatch) {
    reduce(
        state,
        AppAction::SettingsUpdateRequested {
            request_id: 1,
            patch,
        },
    );
}

#[test]
fn home_and_active_space_collapse_and_expand_publish_immediately() {
    for active in [None, Some(SPACE)] {
        for section in [SidebarSectionKind::Rooms, SidebarSectionKind::Dms] {
            let mut previous = fixture(active);
            for collapsed in [true, false] {
                let mut next = previous.clone();
                update(&mut next, patch(active, section, Some(collapsed), None));
                let sidebar = build_state_delta(1, &previous, &next)
                    .unwrap()
                    .changed
                    .sidebar
                    .expect("section collapse must publish without switching spaces");
                assert_eq!(sidebar, compose_sidebar_for_state(&next));
                assert_eq!(
                    sidebar.rooms_collapsed,
                    section == SidebarSectionKind::Rooms && collapsed
                );
                assert_eq!(
                    sidebar.dms_collapsed,
                    section == SidebarSectionKind::Dms && collapsed
                );
                assert_eq!(next.navigation, previous.navigation);
                previous = next;
            }
        }
    }
}

#[test]
fn home_and_active_space_sorts_publish_order_and_preserve_other_section() {
    for active in [None, Some(SPACE)] {
        for section in [SidebarSectionKind::Rooms, SidebarSectionKind::Dms] {
            let mut previous = fixture(active);
            update(&mut previous, patch(active, section, Some(true), None));
            for (sort, names) in [
                (RoomListSort::NormalLocale, vec!["A", "B", "C"]),
                (RoomListSort::RecentFirst, vec!["B", "C", "A"]),
                (RoomListSort::Activity, vec!["C", "B", "A"]),
            ] {
                let before = compose_sidebar_for_state(&previous);
                let mut next = previous.clone();
                update(&mut next, patch(active, section, None, Some(sort)));
                let sidebar = build_state_delta(1, &previous, &next)
                    .unwrap()
                    .changed
                    .sidebar
                    .expect("sort must publish without switching spaces");
                assert_eq!(sidebar, compose_sidebar_for_state(&next));
                let rows = match section {
                    SidebarSectionKind::Rooms => {
                        assert!(sidebar.rooms_collapsed);
                        assert_eq!(sidebar.rooms_sort, sort);
                        assert_eq!(sidebar.global_dms, before.global_dms);
                        assert_eq!(sidebar.dms_sort, before.dms_sort);
                        &sidebar.space_rooms
                    }
                    SidebarSectionKind::Dms => {
                        assert!(sidebar.dms_collapsed);
                        assert_eq!(sidebar.dms_sort, sort);
                        assert_eq!(sidebar.space_rooms, before.space_rooms);
                        assert_eq!(sidebar.rooms_sort, before.rooms_sort);
                        &sidebar.global_dms
                    }
                };
                assert_eq!(
                    rows.iter()
                        .map(|room| room.display_name.as_str())
                        .collect::<Vec<_>>(),
                    names
                );
                assert_eq!(next.navigation, previous.navigation);
                previous = next;
            }
        }
    }
}

#[test]
fn inactive_scope_updates_wait_until_selected() {
    for (active, target) in [
        (None, Some(SPACE)),
        (Some(SPACE), None),
        (Some(SPACE), Some(OTHER)),
    ] {
        for section in [SidebarSectionKind::Rooms, SidebarSectionKind::Dms] {
            let previous = fixture(active);
            let mut next = previous.clone();
            update(
                &mut next,
                patch(
                    target,
                    section,
                    Some(true),
                    Some(RoomListSort::NormalLocale),
                ),
            );
            let delta = build_state_delta(1, &previous, &next).unwrap();
            assert!(delta.changed.settings.is_some());
            assert!(delta.changed.sidebar.is_none());
            assert_eq!(
                compose_sidebar_for_state(&previous),
                compose_sidebar_for_state(&next)
            );
            let mut selected = next.clone();
            selected.navigation.active_space_id = target.map(str::to_owned);
            assert_eq!(
                build_state_delta(2, &next, &selected)
                    .unwrap()
                    .changed
                    .sidebar,
                Some(compose_sidebar_for_state(&selected))
            );
        }
    }
}

#[test]
fn repeated_and_effectively_unchanged_preferences_do_not_publish_sidebar() {
    for active in [None, Some(SPACE)] {
        for section in [SidebarSectionKind::Rooms, SidebarSectionKind::Dms] {
            let previous = fixture(active);
            let mut next = previous.clone();
            let same = patch(active, section, Some(false), Some(RoomListSort::Activity));
            update(&mut next, same.clone());
            assert!(
                build_state_delta(1, &previous, &next)
                    .unwrap()
                    .changed
                    .sidebar
                    .is_none()
            );
            let mut repeated = next.clone();
            update(&mut repeated, same);
            assert!(build_state_delta(2, &next, &repeated).is_none());
        }
    }
}

#[test]
fn serialized_settings_reload_publishes_current_scope_preferences() {
    let mut saved = fixture(None);
    for scope in [None, Some(SPACE), Some(OTHER)] {
        update(
            &mut saved,
            patch(
                scope,
                SidebarSectionKind::Rooms,
                Some(true),
                Some(RoomListSort::NormalLocale),
            ),
        );
        update(
            &mut saved,
            patch(
                scope,
                SidebarSectionKind::Dms,
                Some(true),
                Some(RoomListSort::RecentFirst),
            ),
        );
    }
    let json = serde_json::to_string(&saved.settings.values).unwrap();
    let values: SettingsValues = serde_json::from_str(&json).unwrap();
    assert_eq!(values, saved.settings.values);
    for active in [None, Some(SPACE), Some(OTHER)] {
        let previous = fixture(active);
        let mut next = previous.clone();
        reduce(
            &mut next,
            AppAction::SettingsLoaded {
                values: values.clone(),
            },
        );
        let sidebar = build_state_delta(1, &previous, &next)
            .unwrap()
            .changed
            .sidebar
            .expect("reload must publish current scope");
        assert_eq!(sidebar, compose_sidebar_for_state(&next));
        assert!(sidebar.rooms_collapsed && sidebar.dms_collapsed);
        assert_eq!(sidebar.rooms_sort, RoomListSort::NormalLocale);
        assert_eq!(sidebar.dms_sort, RoomListSort::RecentFirst);
    }
}
