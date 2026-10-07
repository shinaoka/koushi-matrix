use super::*;
use koushi_state::{
    AppAction, RoomListSort, RoomSummary, SettingsPatch, SettingsValues, SidebarSectionKind,
    SidebarSectionPatch, SpaceChildMembership, SpaceChildSummary, SpaceSummary, reduce,
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
            raw_name: None,
            display_name: "Synthetic Space".into(),
            avatar: None,
            join_rule: None,
            child_room_ids: state
                .rooms
                .iter()
                .filter(|room| !room.is_dm)
                .map(|room| room.room_id.clone())
                .collect(),
            parent_side_child_room_ids: state
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
                    // Low priority carries no independent sort (#955); it is
                    // covered by `low_priority_collapse_publishes_per_scope`.
                    SidebarSectionKind::LowPriority => unreachable!(),
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
fn loading_space_children_publishes_the_not_joined_sidebar_lane() {
    let previous = fixture(Some(SPACE));
    let mut next = previous.clone();
    next.space_children.selected_space_id = Some(SPACE.to_owned());
    next.space_children.children = vec![SpaceChildSummary {
        room_id: "!private:example.invalid".to_owned(),
        display_name: "Private Room".to_owned(),
        display_name_placeholder: None,
        avatar: None,
        membership: SpaceChildMembership::NotJoined,
        can_join: false,
        is_space: false,
        joined_members: 1,
    }];

    let sidebar = build_state_delta(1, &previous, &next)
        .unwrap()
        .changed
        .sidebar
        .expect("space child loading must republish the sidebar");
    assert_eq!(sidebar.sections.not_joined[0].display_name, "Private Room");
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

/// #955: the Low priority section stores only a per-scope collapse choice and
/// publishes it on the same immediate delta lane as Rooms and DMs.
#[test]
fn low_priority_collapse_publishes_per_scope() {
    for active in [None, Some(SPACE)] {
        let mut previous = fixture(active);
        previous
            .rooms
            .iter_mut()
            .for_each(|room| room.tags.low_priority = None);
        for collapsed in [true, false] {
            let mut next = previous.clone();
            update(
                &mut next,
                patch(
                    active,
                    SidebarSectionKind::LowPriority,
                    Some(collapsed),
                    None,
                ),
            );
            let sidebar = build_state_delta(1, &previous, &next)
                .unwrap()
                .changed
                .sidebar
                .expect("Low priority collapse must publish without switching spaces");
            assert_eq!(sidebar, compose_sidebar_for_state(&next));
            assert_eq!(sidebar.low_priority_collapsed, collapsed);
            assert!(!sidebar.rooms_collapsed);
            assert!(!sidebar.dms_collapsed);
            previous = next;
        }

        // An inactive scope's Low priority choice must not change this view.
        let inactive = if active.is_none() { Some(OTHER) } else { None };
        let mut next = previous.clone();
        update(
            &mut next,
            patch(inactive, SidebarSectionKind::LowPriority, Some(true), None),
        );
        let delta = build_state_delta(1, &previous, &next).unwrap();
        assert!(delta.changed.settings.is_some());
        assert!(delta.changed.sidebar.is_none());
    }
}

/// #955: a tag-only account-data change re-projects the sidebar sections and
/// the native attention totals, without replaying a notification candidate.
#[test]
fn a_low_priority_tag_change_republishes_sections_and_native_attention() {
    let mut previous = fixture(None);
    // Tag actions are accepted only for a Ready session.
    previous.session = koushi_state::SessionState::Ready(koushi_state::SessionInfo {
        homeserver: "https://matrix.example.invalid".to_owned(),
        user_id: "@fixture:example.invalid".to_owned(),
        device_id: "FIXTURE_DEVICE".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    });
    let tagged_room_id = "!C-false:example.invalid";
    let before = compose_sidebar_for_state(&previous);
    assert_eq!(before.space_unread_count, 1);
    assert!(
        before
            .sections
            .rooms
            .iter()
            .any(|room| room.room_id == tagged_room_id)
    );
    assert!(before.sections.low_priority.is_empty());

    let mut next = previous.clone();
    reduce(
        &mut next,
        AppAction::RoomTagsUpdated {
            room_id: tagged_room_id.to_owned(),
            tags: koushi_state::RoomTags {
                favourite: None,
                low_priority: Some(koushi_state::RoomTagInfo { order: None }),
            },
        },
    );

    let delta = build_state_delta(1, &previous, &next).unwrap();
    let sidebar = delta
        .changed
        .sidebar
        .expect("a tag-only change must republish the sidebar");
    assert_eq!(sidebar, compose_sidebar_for_state(&next));
    assert_eq!(
        sidebar
            .sections
            .low_priority
            .iter()
            .map(|room| room.room_id.as_str())
            .collect::<Vec<_>>(),
        [tagged_room_id]
    );
    assert!(
        !sidebar
            .sections
            .rooms
            .iter()
            .any(|room| room.room_id == tagged_room_id)
    );
    assert_eq!(sidebar.space_unread_count, 0);
    assert_eq!(sidebar.account_home.unread_count, 1, "the DM still counts");
    assert!(next.native_attention.summary.candidate.is_none());
}

/// #1166: a room's projected access condition is visible on its sidebar row, so
/// changing only that slice must publish a sidebar delta.
#[test]
fn access_condition_change_alone_publishes_the_sidebar() {
    let previous = fixture(None);
    let mut next = previous.clone();
    next.room_access.insert(
        "!A-false:example.invalid".to_owned(),
        koushi_state::RoomAccessCondition {
            join_rule: koushi_state::RoomJoinRule::Public,
            restricted: None,
            allowed_room_ids: Vec::new(),
        },
    );

    let sidebar = build_state_delta(1, &previous, &next)
        .expect("an access-condition change is a delta")
        .changed
        .sidebar
        .expect("the sidebar carries the row's access condition");

    assert_eq!(sidebar, compose_sidebar_for_state(&next));
    let row = sidebar
        .space_rooms
        .iter()
        .find(|room| room.room_id == "!A-false:example.invalid")
        .expect("the room is in the sidebar");
    assert_eq!(
        row.access_join_rule,
        Some(koushi_state::RoomJoinRule::Public)
    );
}
