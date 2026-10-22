//! Sidebar section row bound (#1363, #1366).
//!
//! A visible section shows at most `SIDEBAR_SECTION_DEFAULT_VISIBLE_COUNT` rows
//! unless the user expanded it, the section keeps reporting its whole contents,
//! and the open conversation is never hidden by the bound.
//!
//! Canon: `docs/architecture/state-machine.md`, "Sidebar Sections And Low
//! Priority".

use koushi_state::{
    AppAction, AppState, ConversationActivity, ConversationActivitySource, NavigationState,
    RoomListSort, RoomSummary, RoomTags, SIDEBAR_HOME_SCOPE_KEY,
    SIDEBAR_SECTION_DEFAULT_VISIBLE_COUNT, SessionInfo, SessionState, SettingsPatch,
    SettingsValues, SidebarSectionKind, SidebarSectionPatch, SidebarSectionSettings, SpaceSummary,
    compose_sidebar_for_state, reduce, sidebar_scope_key,
};

const SPACE_ID: &str = "!space:example.invalid";

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.org".to_owned(),
        user_id: "@user:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    }
}

fn room(id: &str, label: &str, is_dm: bool, activity_ms: u64) -> RoomSummary {
    RoomSummary {
        display_name_placeholder: None,
        display_label_placeholder: None,
        room_id: id.to_owned(),
        display_name: label.to_owned(),
        display_label: label.to_owned(),
        original_display_label: label.to_owned(),
        avatar: None,
        is_dm,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: 1,
        notification_count: 1,
        highlight_count: 0,
        thread_unread_count: 0,
        thread_highlight_count: 0,
        marked_unread: false,
        recency_stamp: Some(activity_ms),
        conversation_activity: Some(ConversationActivity {
            timestamp_ms: activity_ms,
            source: ConversationActivitySource::Message,
        }),
        latest_event: None,
        // Home scope: these rooms belong to no Space, so Home lists them (#1589).
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 2,
    }
}

fn tagged(mut room: RoomSummary, low_priority: bool) -> RoomSummary {
    if low_priority {
        room.tags.low_priority = Some(koushi_state::RoomTagInfo { order: None });
    }
    room
}

/// Home: nine rooms, seven DMs, and seven low-priority conversations, each with
/// a synthetic, distinct activity timestamp so the Recent-first order is known.
/// Rooms and DMs are ordered newest first: `room-9`, `room-8`, ... `room-1`.
///
/// The joined Space owns a separate nine rooms (#1589), so the Space scope has
/// the same bound to exercise while Home keeps its own rows.
fn bounded_state() -> AppState {
    let mut rooms: Vec<RoomSummary> = (1..=9)
        .map(|index| {
            room(
                &format!("!room-{index}:example.invalid"),
                &format!("Room {index}"),
                false,
                index,
            )
        })
        .collect();
    rooms.extend((1..=9).map(|index| {
        let mut in_space = room(
            &format!("!space-room-{index}:example.invalid"),
            &format!("Space room {index}"),
            false,
            200 + index,
        );
        in_space.unread_count = 0;
        in_space.notification_count = 0;
        in_space.parent_space_ids = vec![SPACE_ID.to_owned()];
        in_space
    }));
    rooms.extend((1..=7).map(|index| {
        room(
            &format!("!dm-{index}:example.invalid"),
            &format!("Person {index}"),
            true,
            index,
        )
    }));
    rooms.extend((1..=7).map(|index| {
        tagged(
            room(
                &format!("!low-{index}:example.invalid"),
                &format!("Low {index}"),
                false,
                index,
            ),
            true,
        )
    }));
    AppState {
        session: SessionState::Ready(session_info()),
        spaces: vec![SpaceSummary {
            space_id: SPACE_ID.to_owned(),
            raw_name: None,
            display_name: "Space".to_owned(),
            avatar: None,
            join_rule: None,
            child_room_ids: (1..=9)
                .map(|index| format!("!space-room-{index}:example.invalid"))
                .collect(),
            parent_side_child_room_ids: Vec::new(),
        }],
        rooms,
        ..Default::default()
    }
}

fn names(items: &[koushi_state::RoomListItem]) -> Vec<&str> {
    items
        .iter()
        .map(|item| item.display_name.as_str())
        .collect()
}

fn expand(state: &mut AppState, scope: &str, section: SidebarSectionKind, expanded: bool) {
    state.settings.values.sidebar.apply_section_patch(
        SidebarSectionPatch {
            scope: scope.to_owned(),
            section,
            collapsed: None,
            sort: None,
            expanded: Some(expanded),
        },
        state.settings.values.room_list_sort,
    );
}

#[test]
fn a_section_with_more_rows_than_the_bound_shows_the_first_rows_and_the_hidden_count() {
    let state = bounded_state();
    let sidebar = compose_sidebar_for_state(&state);
    let bound = sidebar
        .section_bounds
        .rooms
        .expect("a projected Rooms section always carries its bound");

    assert_eq!(bound.visible_limit, SIDEBAR_SECTION_DEFAULT_VISIBLE_COUNT);
    assert_eq!(bound.visible_limit, 5);
    assert_eq!(bound.total_count, 9);
    assert_eq!(bound.visible_count, 5);
    assert_eq!(bound.hidden_count, 4);
    assert!(!bound.expanded);
    assert!(!bound.expanded_by_active_room);
    assert!(bound.shows_expand_control);

    // The bound is applied after the Rust sort, so the visible rows are the
    // newest five, not an arbitrary prefix. Synthetic timestamps make this a
    // real ordering assertion instead of a fixture that hides a regression.
    assert_eq!(
        names(&sidebar.sections.rooms[..bound.visible_count]),
        vec!["Room 9", "Room 8", "Room 7", "Room 6", "Room 5"]
    );
    assert_eq!(sidebar.sections.rooms.len(), 9);
}

#[test]
fn a_section_at_or_below_the_bound_has_no_expand_control() {
    for count in [0_usize, 1, 5] {
        let mut state = bounded_state();
        state.rooms.retain(|room| {
            room.is_dm || room.tags.low_priority.is_some() || room_index(room) <= count
        });
        let sidebar = compose_sidebar_for_state(&state);
        let bound = sidebar.section_bounds.rooms.expect("projected bound");
        assert_eq!(bound.total_count, count);
        assert_eq!(bound.visible_count, count);
        assert_eq!(bound.hidden_count, 0);
        assert!(
            !bound.shows_expand_control,
            "a section of {count} rows has nothing to reveal"
        );
    }
}

fn room_index(room: &RoomSummary) -> usize {
    room.display_name
        .rsplit(' ')
        .next()
        .and_then(|index| index.parse().ok())
        .unwrap_or(0)
}

#[test]
fn expanding_a_section_reveals_every_row_without_reordering_the_section() {
    let mut state = bounded_state();
    let collapsed = compose_sidebar_for_state(&state);
    expand(&mut state, "__home__", SidebarSectionKind::Rooms, true);
    let expanded = compose_sidebar_for_state(&state);
    let bound = expanded.section_bounds.rooms.expect("projected bound");

    assert!(bound.expanded);
    assert_eq!(bound.total_count, 9);
    assert_eq!(bound.visible_count, 9);
    assert_eq!(bound.hidden_count, 0);
    assert!(bound.shows_expand_control);
    // The section itself is not reordered by the bound: the projection always
    // emits the whole sorted list and the renderer slices it.
    assert_eq!(expanded.sections.rooms, collapsed.sections.rooms);
}

#[test]
fn the_bound_is_independent_per_scope_and_per_section() {
    let mut state = bounded_state();
    expand(&mut state, "__home__", SidebarSectionKind::Rooms, true);
    expand(&mut state, "__home__", SidebarSectionKind::Dms, true);

    let home = compose_sidebar_for_state(&state);
    assert!(home.section_bounds.rooms.expect("rooms").expanded);
    assert!(home.section_bounds.dms.expect("dms").expanded);
    assert!(
        !home.section_bounds.low_priority.expect("low").expanded,
        "expanding Rooms and DMs must not expand Low priority"
    );

    state.navigation.active_space_id = Some(SPACE_ID.to_owned());
    let space = compose_sidebar_for_state(&state);
    assert!(
        !space.section_bounds.rooms.expect("rooms").expanded,
        "Home's expanded Rooms must not expand the Space's Rooms section"
    );
    assert_eq!(space.section_bounds.rooms.expect("rooms").hidden_count, 4);
}

#[test]
fn the_low_priority_section_follows_the_same_bound() {
    let state = bounded_state();
    let sidebar = compose_sidebar_for_state(&state);
    let bound = sidebar
        .section_bounds
        .low_priority
        .expect("projected bound");
    assert_eq!(bound.total_count, 7);
    assert_eq!(bound.visible_count, 5);
    assert_eq!(bound.hidden_count, 2);
    assert!(bound.shows_expand_control);
    // Not joined keeps its own behavior and is not bounded (#1363 item 8): the
    // bound struct has no entry for it, so its rows always render in full.
    let bounds = serde_json::to_value(sidebar.section_bounds).expect("bounds JSON");
    let mut bounded: Vec<&str> = bounds
        .as_object()
        .expect("the bound is an object")
        .keys()
        .map(String::as_str)
        .collect();
    bounded.sort_unstable();
    assert_eq!(bounded, vec!["dms", "low_priority", "rooms"]);
    assert_eq!(sidebar.section_bounds.rooms.expect("rooms").total_count, 9);
}

#[test]
fn the_open_conversation_is_never_hidden_by_the_bound() {
    let mut state = bounded_state();
    // Room 1 is the oldest room, so it sits past the bound.
    state.navigation.active_room_id = Some("!room-1:example.invalid".to_owned());
    let sidebar = compose_sidebar_for_state(&state);
    let bound = sidebar.section_bounds.rooms.expect("projected bound");

    assert!(bound.expanded);
    assert!(bound.expanded_by_active_room);
    assert_eq!(bound.visible_count, bound.total_count);
    assert_eq!(bound.hidden_count, 0);
    assert!(
        !bound.shows_expand_control,
        "toggling the bound cannot change what is visible while the open room stays past it"
    );

    // A conversation inside the bound leaves the bound in place.
    state.navigation.active_room_id = Some("!room-8:example.invalid".to_owned());
    let inside = compose_sidebar_for_state(&state);
    let bound = inside.section_bounds.rooms.expect("projected bound");
    assert!(!bound.expanded);
    assert_eq!(bound.visible_count, 5);
    assert!(bound.shows_expand_control);

    // An open conversation in another section leaves this section bounded.
    state.navigation.active_room_id = Some("!dm-1:example.invalid".to_owned());
    let other_section = compose_sidebar_for_state(&state);
    assert!(
        !other_section
            .section_bounds
            .rooms
            .expect("projected bound")
            .expanded
    );
}

#[test]
fn section_totals_report_the_whole_section_not_the_bounded_rows() {
    let state = bounded_state();
    let sidebar = compose_sidebar_for_state(&state);
    let bound = sidebar.section_bounds.rooms.expect("projected bound");
    assert_eq!(sidebar.sections.rooms.len(), bound.total_count);
    // Nine rooms with one unread message each: the header aggregate is the whole
    // section, never the five rows the bound shows.
    assert_eq!(sidebar.space_unread_count, 9);
}

#[test]
fn the_bound_preference_persists_per_scope_across_a_settings_round_trip() {
    let mut values = SettingsValues::default();
    values.apply_patch(SettingsPatch {
        sidebar_section: Some(SidebarSectionPatch {
            scope: "__home__".to_owned(),
            section: SidebarSectionKind::Rooms,
            collapsed: None,
            sort: None,
            expanded: Some(true),
        }),
        ..Default::default()
    });
    let loaded: SettingsValues =
        serde_json::from_str(&serde_json::to_string(&values).unwrap()).unwrap();
    assert_eq!(loaded, values);
    assert!(
        loaded
            .sidebar
            .scope(None, RoomListSort::RecentFirst)
            .rooms
            .expanded
    );
    assert!(
        !loaded
            .sidebar
            .scope(Some(SPACE_ID), RoomListSort::RecentFirst)
            .rooms
            .expanded
    );
    // A settings file written before this preference existed stays bounded.
    let legacy: koushi_state::SidebarSettings = serde_json::from_value(serde_json::json!({
        "scope_preferences": {
            "__home__": {
                "rooms": { "collapsed": false, "sort": { "kind": "recentFirst" } }
            }
        }
    }))
    .expect("a pre-#1363 settings file must still load");
    assert!(!legacy.scope(None, RoomListSort::RecentFirst).rooms.expanded);
}

/// #1366: revealing a collapsed section and then changing the *bound* must not
/// re-collapse it. The two dimensions are independent, so "Show more" on a
/// revealed section shows more rows; only the user's collapse choice ends the
/// reveal (which would otherwise hide the section the bound control sits in).
#[test]
fn the_bound_toggle_does_not_end_a_reveal() {
    let mut state = bounded_state();
    collapse(&mut state, "__home__", SidebarSectionKind::Rooms);
    // A room inside the bound, so the revealed section also offers a control.
    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "!room-1:example.invalid".to_owned(),
        },
    );
    assert!(!compose_sidebar_for_state(&state).rooms_collapsed);

    reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 1,
            patch: SettingsPatch {
                sidebar_section: Some(SidebarSectionPatch {
                    scope: "__home__".to_owned(),
                    section: SidebarSectionKind::Rooms,
                    collapsed: None,
                    sort: None,
                    expanded: Some(true),
                }),
                ..Default::default()
            },
        },
    );

    let sidebar = compose_sidebar_for_state(&state);
    assert!(state.navigation.revealed_sidebar_section.is_some());
    assert!(
        !sidebar.rooms_collapsed,
        "the bound toggle must not collapse"
    );
    let bound = sidebar.section_bounds.rooms.expect("projected bound");
    // The user's own bound choice shows every row and keeps the control (now
    // "Show fewer"); the open conversation is not what holds it open.
    assert!(bound.expanded);
    assert!(!bound.expanded_by_active_room);
    assert!(bound.shows_expand_control);
    assert!(bound.visible_count > SIDEBAR_SECTION_DEFAULT_VISIBLE_COUNT);
}

#[test]
fn selecting_a_conversation_reveals_a_collapsed_section() {
    let mut state = bounded_state();
    collapse(&mut state, "__home__", SidebarSectionKind::Rooms);
    assert!(
        compose_sidebar_for_state(&state)
            .section_bounds
            .rooms
            .is_some()
    );
    assert!(compose_sidebar_for_state(&state).rooms_collapsed);

    // A notification or permalink selects a room whose section the persisted
    // collapse would hide. The section is revealed for this session, and the
    // stored preference is untouched.
    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "!room-1:example.invalid".to_owned(),
        },
    );
    let revealed = compose_sidebar_for_state(&state);
    assert!(!revealed.rooms_collapsed);
    assert_eq!(
        state
            .settings
            .values
            .sidebar
            .scope(None, RoomListSort::RecentFirst)
            .rooms,
        SidebarSectionSettings {
            collapsed: true,
            sort: RoomListSort::RecentFirst,
            expanded: false,
        }
    );
    // The reveal is session state and never enters the persisted payload, with
    // the reveal present or absent.
    let persisted = serde_json::to_value(NavigationState::default()).expect("navigation JSON");
    assert!(persisted.get("revealed_sidebar_section").is_none());
    let persisted = serde_json::to_value(&state.navigation).expect("navigation JSON");
    assert!(state.navigation.revealed_sidebar_section.is_some());
    assert!(persisted.get("revealed_sidebar_section").is_none());

    // The user's own collapse toggle ends the reveal.
    reduce(
        &mut state,
        AppAction::SettingsUpdateRequested {
            request_id: 1,
            patch: SettingsPatch {
                sidebar_section: Some(SidebarSectionPatch {
                    scope: "__home__".to_owned(),
                    section: SidebarSectionKind::Rooms,
                    collapsed: Some(true),
                    sort: None,
                    expanded: None,
                }),
                ..Default::default()
            },
        },
    );
    assert!(compose_sidebar_for_state(&state).rooms_collapsed);
}

/// #1363/#1366: the Home scope key is a wire token the renderer spells too
/// (`HOME_SCOPE_KEY` in `apps/desktop/src/components/Shell.tsx`), so pin it.
#[test]
fn the_home_scope_key_is_the_wire_token_the_renderer_sends() {
    assert_eq!(SIDEBAR_HOME_SCOPE_KEY, "__home__");
    assert_eq!(sidebar_scope_key(None), SIDEBAR_HOME_SCOPE_KEY);
    assert_eq!(
        sidebar_scope_key(Some("!space:example.invalid")),
        "!space:example.invalid"
    );
}

fn collapse(state: &mut AppState, scope: &str, section: SidebarSectionKind) {
    state.settings.values.sidebar.apply_section_patch(
        SidebarSectionPatch {
            scope: scope.to_owned(),
            section,
            collapsed: Some(true),
            sort: None,
            expanded: None,
        },
        state.settings.values.room_list_sort,
    );
}

#[test]
fn a_reveal_never_uncollapses_another_scope() {
    let mut state = bounded_state();
    collapse(&mut state, "__home__", SidebarSectionKind::Rooms);
    collapse(&mut state, SPACE_ID, SidebarSectionKind::Rooms);

    state.navigation.active_space_id = Some(SPACE_ID.to_owned());
    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "!space-room-1:example.invalid".to_owned(),
        },
    );
    assert!(
        !compose_sidebar_for_state(&state).rooms_collapsed,
        "the selected room's own scope is revealed"
    );

    // The reveal names the Space, so Home's collapsed section stays collapsed.
    state.navigation.active_space_id = None;
    state.navigation.active_room_id = None;
    assert!(compose_sidebar_for_state(&state).rooms_collapsed);
}

/// The browser harness renders from its own fake snapshot, so the bound it
/// applies must come from Rust rather than from a second implementation of the
/// projection rule. This writes the exact projections the harness looks up, and
/// fails when the checked-in fixture drifts.
///
/// Regenerate with `UPDATE_SIDEBAR_SECTION_BOUNDS_GOLDEN=1`.
#[test]
fn the_frontend_bound_fixture_matches_the_rust_projection() {
    let active_room_indices = [0_usize, 2, 4, 5, 6, 8];
    let mut cases = Vec::new();
    for total_count in 0_usize..=8 {
        for expanded in [false, true] {
            let mut indices: Vec<Option<usize>> = vec![None];
            indices.extend(
                active_room_indices
                    .iter()
                    .copied()
                    .filter(|index| *index < total_count)
                    .map(Some),
            );
            for active_room_index in indices {
                let bound = koushi_state::SidebarSectionBound::project(
                    total_count,
                    expanded,
                    active_room_index,
                );
                cases.push(serde_json::json!({
                    // `persistedExpanded` is the stored preference the frontend
                    // fakes hold; `expanded` is the effective bound the packaged
                    // Rust projects, which the open conversation can force.
                    "totalCount": total_count,
                    "persistedExpanded": expanded,
                    "activeRoomIndex": active_room_index,
                    "expanded": bound.expanded,
                    "visibleCount": bound.visible_count,
                    "hiddenCount": bound.hidden_count,
                    "expandedByActiveRoom": bound.expanded_by_active_room,
                    "showsExpandControl": bound.shows_expand_control,
                }));
            }
        }
    }
    let actual = serde_json::json!({
        "visibleLimit": SIDEBAR_SECTION_DEFAULT_VISIBLE_COUNT,
        "cases": cases,
    });

    let artifact_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../apps/desktop/src/test/sidebarSectionBounds.generated.json"
    );
    if std::env::var("UPDATE_SIDEBAR_SECTION_BOUNDS_GOLDEN").as_deref() == Ok("1") {
        std::fs::write(
            artifact_path,
            serde_json::to_string_pretty(&actual).expect("format sidebar section bounds") + "\n",
        )
        .expect("write sidebar section bounds fixture");
        return;
    }
    let checked_in: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(artifact_path).expect("read sidebar section bounds fixture"),
    )
    .expect("checked-in sidebar section bounds fixture must be valid JSON");
    assert_eq!(actual, checked_in);
}
