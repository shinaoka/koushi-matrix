//! Low-priority sidebar section, attention exclusion, and scoped collapse
//! preferences (#955).
//!
//! Canon: `docs/architecture/state-machine.md`, "Sidebar Sections And Low
//! Priority" and "Native Attention".

use std::collections::{BTreeSet, HashMap};

use koushi_state::{
    AppState, ConversationActivity, ConversationActivitySource, NativeAttentionCapabilities,
    NativeAttentionObservationKind, NativeAttentionProjectionInput, RoomListSort,
    RoomNotificationMode, RoomNotificationSettings, RoomSummary, RoomTagInfo, RoomTags,
    SidebarModel, SidebarSectionKind, SidebarSectionPatch, SidebarSectionSettings, SidebarSettings,
    SpaceSummary, compose_sidebar_for_state, native_attention_projection_from_rooms,
};

const SPACE_ID: &str = "!space:example.invalid";
const OTHER_SPACE_ID: &str = "!other-space:example.invalid";

fn room(id: &str, label: &str, is_dm: bool, unread: u64, highlight: u64) -> RoomSummary {
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
        unread_count: unread,
        notification_count: unread,
        highlight_count: highlight,
        marked_unread: false,
        recency_stamp: Some(10),
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: vec![SPACE_ID.to_owned()],
        dm_space_ids: is_dm.then(|| SPACE_ID.to_owned()).into_iter().collect(),
        is_encrypted: false,
        joined_members: 2,
    }
}

/// Give a room a conversation timestamp so activity/recency ordering is
/// decided by real activity instead of the display-label tiebreak.
fn with_activity(mut room: RoomSummary, timestamp_ms: u64) -> RoomSummary {
    room.recency_stamp = Some(timestamp_ms);
    room.conversation_activity = Some(ConversationActivity {
        timestamp_ms,
        source: ConversationActivitySource::Message,
    });
    room
}

fn tag_low_priority(mut room: RoomSummary) -> RoomSummary {
    room.tags.low_priority = Some(RoomTagInfo { order: None });
    room
}

fn tag_favourite(mut room: RoomSummary) -> RoomSummary {
    room.tags.favourite = Some(RoomTagInfo { order: None });
    room
}

/// Home scope, one of each conversation shape. Rooms carry unread counts so an
/// aggregate that forgot to exclude low priority is visible as a number.
fn mixed_state() -> AppState {
    AppState {
        spaces: vec![SpaceSummary {
            space_id: SPACE_ID.to_owned(),
            raw_name: None,
            display_name: "Space".to_owned(),
            avatar: None,
            join_rule: None,
            child_room_ids: vec![
                "!plain:example.invalid".to_owned(),
                "!fav:example.invalid".to_owned(),
                "!low-room:example.invalid".to_owned(),
            ],
            parent_side_child_room_ids: vec![
                "!plain:example.invalid".to_owned(),
                "!fav:example.invalid".to_owned(),
                "!low-room:example.invalid".to_owned(),
            ],
        }],
        rooms: vec![
            room("!plain:example.invalid", "Plain", false, 5, 1),
            tag_favourite(room("!fav:example.invalid", "Favourite", false, 0, 0)),
            tag_low_priority(with_activity(
                room("!low-room:example.invalid", "Zed Low", false, 8, 2),
                200,
            )),
            room("!dm:example.invalid", "Person", true, 3, 0),
            tag_low_priority(with_activity(
                room("!low-dm:example.invalid", "Alpha Low", true, 6, 4),
                100,
            )),
        ],
        ..Default::default()
    }
}

fn names(items: &[koushi_state::RoomListItem]) -> Vec<&str> {
    items
        .iter()
        .map(|item| item.display_name.as_str())
        .collect()
}

#[test]
fn sidebar_sections_place_every_conversation_in_exactly_one_visible_section() {
    let sidebar = compose_sidebar_for_state(&mixed_state());

    assert_eq!(names(&sidebar.sections.rooms), ["Plain", "Favourite"]);
    assert_eq!(names(&sidebar.sections.people), ["Person"]);
    assert_eq!(
        names(&sidebar.sections.low_priority),
        ["Zed Low", "Alpha Low"]
    );
    assert_eq!(names(&sidebar.sections.favourites), ["Favourite"]);

    let mut visible: Vec<&str> = sidebar
        .sections
        .rooms
        .iter()
        .chain(sidebar.sections.people.iter())
        .chain(sidebar.sections.low_priority.iter())
        .map(|item| item.room_id.as_str())
        .collect();
    let distinct: BTreeSet<&str> = visible.iter().copied().collect();
    visible.sort_unstable();
    assert_eq!(visible.len(), distinct.len(), "sections must not overlap");
    assert_eq!(distinct.len(), 5, "every conversation must be reachable");
}

#[test]
fn low_priority_rows_keep_their_own_raw_unread_counts() {
    let sidebar = compose_sidebar_for_state(&mixed_state());

    let low_room = sidebar
        .sections
        .low_priority
        .iter()
        .find(|item| item.display_name == "Zed Low")
        .expect("low-priority room is listed");
    assert_eq!(low_room.unread_count, 8);
    assert_eq!(low_room.highlight_count, 2);
    assert!(low_room.has_unread_content);
    assert!(!low_room.is_muted);
}

#[test]
fn low_priority_is_excluded_from_every_sidebar_attention_aggregate() {
    let sidebar = compose_sidebar_for_state(&mixed_state());

    // Only the plain room (5 / 1) and the plain DM (3 / 0) contribute.
    assert_eq!(sidebar.space_unread_count, 5);
    assert_eq!(sidebar.space_highlight_count, 1);
    assert_eq!(sidebar.dm_unread_count, 3);
    assert_eq!(sidebar.dm_highlight_count, 0);
    assert_eq!(sidebar.account_home.unread_count, 8);
    assert_eq!(sidebar.account_home.highlight_count, 1);
    assert_eq!(sidebar.account_home.attention_count, 8);
    assert_eq!(sidebar.space_rail[0].unread_count, 5);
    assert_eq!(sidebar.space_rail[0].highlight_count, 1);
}

#[test]
fn removing_the_low_priority_tag_restores_the_aggregate_contribution() {
    let mut state = mixed_state();
    state.rooms[2].tags.low_priority = None;
    let sidebar = compose_sidebar_for_state(&state);

    assert_eq!(names(&sidebar.sections.low_priority), ["Alpha Low"]);
    assert_eq!(sidebar.space_unread_count, 13);
    assert_eq!(sidebar.space_highlight_count, 3);
    assert_eq!(sidebar.account_home.unread_count, 16);
}

#[test]
fn a_space_scope_lists_only_its_own_low_priority_conversations() {
    let mut state = mixed_state();
    state.navigation.active_space_id = Some(SPACE_ID.to_owned());
    let outside = tag_low_priority(room("!outside:example.invalid", "Outside", true, 9, 9));
    let mut outside = outside;
    outside.parent_space_ids = vec![OTHER_SPACE_ID.to_owned()];
    outside.dm_space_ids = vec![OTHER_SPACE_ID.to_owned()];
    state.rooms.push(outside);

    let sidebar = compose_sidebar_for_state(&state);
    assert_eq!(
        names(&sidebar.sections.low_priority),
        ["Zed Low", "Alpha Low"]
    );
    assert_eq!(sidebar.dm_unread_count, 3);
}

#[test]
fn a_muted_low_priority_room_stays_out_of_both_aggregates_and_sections() {
    let mut state = mixed_state();
    state.room_notification_settings.insert(
        "!low-room:example.invalid".to_owned(),
        RoomNotificationSettings {
            mode: RoomNotificationMode::Mute,
            ..RoomNotificationSettings::default()
        },
    );
    let sidebar = compose_sidebar_for_state(&state);

    assert!(names(&sidebar.sections.rooms).contains(&"Plain"));
    assert!(names(&sidebar.sections.low_priority).contains(&"Zed Low"));
    assert_eq!(sidebar.space_unread_count, 5);
    assert_eq!(sidebar.account_home.unread_count, 8);
}

#[test]
fn low_priority_collapse_falls_back_to_the_legacy_flag_until_the_scope_is_edited() {
    let mut state = mixed_state();
    state.settings.values.sidebar.collapsed.low_priority = true;

    let legacy: SidebarModel = compose_sidebar_for_state(&state);
    assert!(legacy.low_priority_collapsed);
    assert!(!legacy.rooms_collapsed);

    state.settings.values.sidebar.apply_section_patch(
        SidebarSectionPatch {
            scope: "__home__".to_owned(),
            section: SidebarSectionKind::LowPriority,
            collapsed: Some(false),
            sort: None,
        },
        RoomListSort::Activity,
    );
    let scoped = compose_sidebar_for_state(&state);
    assert!(!scoped.low_priority_collapsed);

    // A different scope keeps the legacy fallback until it is edited too.
    state.navigation.active_space_id = Some(SPACE_ID.to_owned());
    assert!(compose_sidebar_for_state(&state).low_priority_collapsed);
}

#[test]
fn the_low_priority_section_follows_the_scope_rooms_sort() {
    let mut settings = SidebarSettings::default();
    settings.apply_section_patch(
        SidebarSectionPatch {
            scope: "__home__".to_owned(),
            section: SidebarSectionKind::Rooms,
            collapsed: None,
            sort: Some(RoomListSort::NormalLocale),
        },
        RoomListSort::Activity,
    );
    let scope = settings.scope(None, RoomListSort::Activity);
    assert_eq!(
        scope.low_priority,
        Some(SidebarSectionSettings {
            collapsed: false,
            sort: RoomListSort::NormalLocale,
        })
    );
}

#[test]
fn a_room_list_sort_change_reorders_the_low_priority_section() {
    let mut state = mixed_state();
    state.settings.values.room_list_sort = RoomListSort::NormalLocale;
    let sidebar = compose_sidebar_for_state(&state);
    assert_eq!(
        names(&sidebar.sections.low_priority),
        ["Alpha Low", "Zed Low"]
    );
}

fn project(rooms: &[RoomSummary]) -> koushi_state::NativeAttentionProjection {
    native_attention_projection_from_rooms(NativeAttentionProjectionInput {
        rooms,
        active_room_id: None,
        muted_room_ids: &[],
        room_notification_modes: &HashMap::new(),
        ignored_user_ids: &BTreeSet::new(),
        window_focused: false,
        observation: NativeAttentionObservationKind::Live,
        previous_candidate: None,
        message_previews: false,
        capabilities: NativeAttentionCapabilities::default(),
    })
}

#[test]
fn low_priority_only_activity_raises_no_native_badge_candidate_or_total() {
    let mut mention = tag_low_priority(room("!low:example.invalid", "Low", false, 7, 3));
    mention.notification_count = 7;
    let projection = project(&[mention]);

    assert_eq!(projection.state.summary.badge_count, 0);
    assert_eq!(projection.state.summary.unread_count, 0);
    assert_eq!(projection.state.summary.highlight_count, 0);
    assert_eq!(projection.notification_count, 0);
    assert_eq!(projection.state.summary.candidate, None);
    assert_eq!(projection.badge_room_count, 0);
    assert_eq!(projection.badge_excluded_room_count, 1);
}

#[test]
fn a_normal_room_keeps_its_native_contribution_beside_a_low_priority_room() {
    let plain = room("!plain:example.invalid", "Plain", false, 4, 1);
    let low = tag_low_priority(room("!low:example.invalid", "Low", false, 9, 9));
    let projection = project(&[plain, low]);

    assert_eq!(projection.state.summary.badge_count, 4);
    assert_eq!(projection.state.summary.unread_count, 4);
    assert_eq!(projection.state.summary.highlight_count, 1);
    assert_eq!(projection.badge_room_count, 1);
    assert_eq!(projection.badge_excluded_room_count, 1);
    assert_eq!(
        projection
            .state
            .summary
            .candidate
            .as_ref()
            .map(|candidate| candidate.room_display_name.as_str()),
        Some("Plain")
    );
}

#[test]
fn an_ignored_user_dm_keeps_its_raw_badge_contribution() {
    let mut dm = room("!ignored:example.invalid", "Ignored", true, 6, 0);
    dm.dm_user_ids = vec!["@ignored:example.invalid".to_owned()];
    let ignored = BTreeSet::from(["@ignored:example.invalid".to_owned()]);

    let projection = native_attention_projection_from_rooms(NativeAttentionProjectionInput {
        rooms: &[dm],
        active_room_id: None,
        muted_room_ids: &[],
        room_notification_modes: &HashMap::new(),
        ignored_user_ids: &ignored,
        window_focused: false,
        observation: NativeAttentionObservationKind::Live,
        previous_candidate: None,
        message_previews: false,
        capabilities: NativeAttentionCapabilities::default(),
    });

    assert_eq!(projection.state.summary.badge_count, 6);
    assert_eq!(projection.state.summary.unread_count, 0);
    assert_eq!(projection.state.summary.candidate, None);
    assert_eq!(projection.badge_room_count, 1);
}
