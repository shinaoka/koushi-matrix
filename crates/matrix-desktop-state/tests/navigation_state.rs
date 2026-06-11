use matrix_desktop_state::{
    AppAction, AppEffect, AppState, RoomSummary, SpaceSummary, UiEvent, compose_sidebar, reduce,
};

fn spaces() -> Vec<SpaceSummary> {
    vec![SpaceSummary {
        space_id: "space-a".to_owned(),
        display_name: "Space A".to_owned(),
        child_room_ids: vec!["room-a".to_owned(), "dm-a".to_owned()],
    }]
}

fn rooms() -> Vec<RoomSummary> {
    vec![
        RoomSummary {
            room_id: "room-a".to_owned(),
            display_name: "Room A".to_owned(),
            is_dm: false,
            unread_count: 5,
            parent_space_ids: vec!["space-a".to_owned()],
        },
        RoomSummary {
            room_id: "dm-a".to_owned(),
            display_name: "Alice".to_owned(),
            is_dm: true,
            unread_count: 3,
            parent_space_ids: vec!["space-a".to_owned()],
        },
        RoomSummary {
            room_id: "global-room".to_owned(),
            display_name: "Global Room".to_owned(),
            is_dm: false,
            unread_count: 2,
            parent_space_ids: vec![],
        },
    ]
}

#[test]
fn room_list_update_replaces_state_and_emits_room_list_event() {
    let mut state = AppState::default();

    let effects = reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: spaces(),
            rooms: rooms(),
        },
    );

    assert_eq!(state.spaces.len(), 1);
    assert_eq!(state.rooms.len(), 3);
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
    );
}

#[test]
fn selecting_space_filters_rooms_and_keeps_dms_global() {
    let mut state = AppState {
        spaces: spaces(),
        rooms: rooms(),
        ..AppState::default()
    };

    let effects = reduce(
        &mut state,
        AppAction::SelectSpace {
            space_id: Some("space-a".to_owned()),
        },
    );

    assert_eq!(state.navigation.active_space_id.as_deref(), Some("space-a"));
    let sidebar = compose_sidebar(
        state.navigation.active_space_id.as_deref(),
        &state.spaces,
        &state.rooms,
    );
    assert_eq!(sidebar.space_rooms[0].room_id, "room-a");
    assert_eq!(sidebar.global_dms[0].room_id, "dm-a");
    assert_eq!(sidebar.space_unread_count, 5);
    assert_eq!(sidebar.dm_unread_count, 3);
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
    );
}

#[test]
fn selecting_room_subscribes_timeline_and_clears_thread() {
    let mut state = AppState::default();

    let effects = reduce(
        &mut state,
        AppAction::SelectRoom {
            room_id: "room-a".to_owned(),
        },
    );

    assert_eq!(state.navigation.active_room_id.as_deref(), Some("room-a"));
    assert_eq!(state.timeline.room_id.as_deref(), Some("room-a"));
    assert!(!state.timeline.is_subscribed);
    assert_eq!(
        effects,
        vec![
            AppEffect::SubscribeTimeline {
                room_id: "room-a".to_owned(),
            },
            AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: "room-a".to_owned(),
            }),
        ]
    );
}
