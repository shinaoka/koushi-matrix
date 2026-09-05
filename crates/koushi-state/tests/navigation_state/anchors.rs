use super::support::{ready_state, rooms, spaces};
use koushi_state::{AppAction, AppState, MainTimelineAnchor, TimelineScrollAnchorEdge, reduce};
use std::collections::BTreeMap;

#[test]
fn legacy_navigation_json_without_scroll_anchors_loads_with_empty_map() {
    let json = r#"{
        "active_space_id": "!space:test.example.com",
        "active_room_id": "!room:test.example.com",
        "space_order": ["!space:test.example.com"],
        "last_room_by_space_id": {"!space:test.example.com": "!room:test.example.com"}
    }"#;

    let navigation: koushi_state::NavigationState =
        serde_json::from_str(json).expect("deserialize legacy navigation");

    assert!(navigation.room_scroll_anchors.is_empty());
    assert_eq!(
        navigation.active_space_id.as_deref(),
        Some("!space:test.example.com")
    );
    assert_eq!(
        navigation.active_room_id.as_deref(),
        Some("!room:test.example.com")
    );
}

#[test]
fn legacy_navigation_scroll_anchor_without_edge_defaults_to_top() {
    let json = r#"{
        "active_space_id": "!space:test.example.com",
        "active_room_id": "!room:test.example.com",
        "space_order": ["!space:test.example.com"],
        "last_room_by_space_id": {"!space:test.example.com": "!room:test.example.com"},
        "room_scroll_anchors": {
            "!room:test.example.com": {
                "event_id": "$anchor:event",
                "offset_px": 24,
                "updated_at_ms": 1820000000000
            }
        }
    }"#;

    let navigation: koushi_state::NavigationState =
        serde_json::from_str(json).expect("deserialize legacy navigation scroll anchor");

    let anchor = navigation
        .room_scroll_anchors
        .get("!room:test.example.com")
        .expect("legacy anchor should survive");
    assert_eq!(anchor.edge, TimelineScrollAnchorEdge::Top);
    assert_eq!(anchor.event_id, "$anchor:event");
    assert_eq!(anchor.offset_px, 24);
    assert_eq!(anchor.updated_at_ms, 1_820_000_000_000);
}

#[test]
fn navigation_state_round_trips_scroll_anchors_through_serde() {
    let navigation = koushi_state::NavigationState {
        active_space_id: Some("!space:test.example.com".to_owned()),
        active_room_id: Some("!room:test.example.com".to_owned()),
        home_selection: koushi_state::HomeSelection::default(),
        space_local_presentations: koushi_state::SpaceLocalPresentations::default(),
        legacy_frontend_preferences_imported: false,
        space_order: vec!["!space:test.example.com".to_owned()],
        last_room_by_space_id: BTreeMap::from([(
            "!space:test.example.com".to_owned(),
            "!room:test.example.com".to_owned(),
        )]),
        last_selection_by_space_id: BTreeMap::from([(
            "!space:test.example.com".to_owned(),
            koushi_state::SpaceNavigationSelection {
                surface: koushi_state::SpaceConversationSurface::Dms,
                room_id: Some("!dm:test.example.com".to_owned()),
            },
        )]),
        room_scroll_anchors: BTreeMap::from([(
            "!room:test.example.com".to_owned(),
            koushi_state::TimelineScrollAnchor {
                event_id: "$anchor:event".to_owned(),
                edge: TimelineScrollAnchorEdge::Top,
                offset_px: 24,
                updated_at_ms: 1_820_000_000_000,
            },
        )]),
        main_timeline_anchor: None,
        event_navigation: koushi_state::EventNavigationState::Idle,
    };

    let encoded = serde_json::to_string(&navigation).expect("serialize navigation");
    let decoded: koushi_state::NavigationState =
        serde_json::from_str(&encoded).expect("deserialize navigation");

    assert_eq!(decoded, navigation);
}

#[test]
fn main_timeline_anchor_enters_returns_and_resets_on_room_change() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: spaces(),
            rooms: rooms(),
        },
    );
    assert_eq!(state.navigation.active_room_id.as_deref(), Some("room-a"));
    assert_eq!(state.navigation.main_timeline_anchor, None);

    // Enter anchored mode for the active room.
    let effects = reduce(
        &mut state,
        AppAction::EnterAnchoredTimeline {
            room_id: "room-a".to_owned(),
            event_id: "$deep-event".to_owned(),
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.navigation.main_timeline_anchor,
        Some(MainTimelineAnchor {
            event_id: "$deep-event".to_owned(),
        })
    );

    // Returning to live clears the anchor.
    reduce(
        &mut state,
        AppAction::ReturnMainTimelineToLive {
            room_id: "room-a".to_owned(),
        },
    );
    assert_eq!(state.navigation.main_timeline_anchor, None);

    // Re-enter, then switch rooms -> the anchor resets to live.
    reduce(
        &mut state,
        AppAction::EnterAnchoredTimeline {
            room_id: "room-a".to_owned(),
            event_id: "$deep-event".to_owned(),
        },
    );
    assert!(state.navigation.main_timeline_anchor.is_some());
    reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "global-room".to_owned(),
        },
    );
    assert_eq!(
        state.navigation.active_room_id.as_deref(),
        Some("global-room")
    );
    assert_eq!(state.navigation.main_timeline_anchor, None);
}

#[test]
fn main_timeline_anchor_is_guarded_by_session_and_active_room() {
    // Not session-ready -> no-op.
    let mut signed_out = AppState::default();
    reduce(
        &mut signed_out,
        AppAction::EnterAnchoredTimeline {
            room_id: "room-a".to_owned(),
            event_id: "$e".to_owned(),
        },
    );
    assert_eq!(signed_out.navigation.main_timeline_anchor, None);

    // Ready, but the target room is not the active room -> no-op.
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: spaces(),
            rooms: rooms(),
        },
    );
    assert_eq!(state.navigation.active_room_id.as_deref(), Some("room-a"));
    reduce(
        &mut state,
        AppAction::EnterAnchoredTimeline {
            room_id: "global-room".to_owned(),
            event_id: "$e".to_owned(),
        },
    );
    assert_eq!(state.navigation.main_timeline_anchor, None);
}

#[test]
fn closing_focused_context_returns_main_pane_to_live() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: spaces(),
            rooms: rooms(),
        },
    );
    // Seed a persisted live-timeline scroll anchor for the room.
    reduce(
        &mut state,
        AppAction::TimelineScrollAnchorUpdated {
            room_id: "room-a".to_owned(),
            anchor: koushi_state::TimelineScrollAnchor {
                event_id: "$old-live-pos".to_owned(),
                edge: TimelineScrollAnchorEdge::Bottom,
                offset_px: 0,
                updated_at_ms: 1_700_000_000_000,
            },
        },
    );
    assert!(state.navigation.room_scroll_anchors.contains_key("room-a"));

    reduce(
        &mut state,
        AppAction::OpenFocusedContext {
            room_id: "room-a".to_owned(),
            event_id: "$deep-event".to_owned(),
        },
    );
    reduce(
        &mut state,
        AppAction::EnterAnchoredTimeline {
            room_id: "room-a".to_owned(),
            event_id: "$deep-event".to_owned(),
        },
    );
    assert!(state.navigation.main_timeline_anchor.is_some());

    reduce(&mut state, AppAction::CloseFocusedContext);
    assert_eq!(state.navigation.main_timeline_anchor, None);
    // #161: returning to live from the anchored view drops the stale room scroll
    // anchor so the live timeline pins to the live edge, not a pre-jump position.
    assert!(!state.navigation.room_scroll_anchors.contains_key("room-a"));
}
