use matrix_desktop_state::{
    AppAction, AppEffect, AppState, RoomSummary, RoomTagInfo, RoomTagKind, RoomTags, SessionInfo,
    SessionState, UiEvent, reduce,
};

fn session_info() -> SessionInfo {
    SessionInfo {
        homeserver: "https://matrix.example.org".to_owned(),
        user_id: "@user-a:example.invalid".to_owned(),
        device_id: "DEVICE".to_owned(),
    }
}

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(session_info()),
        rooms: vec![room("!room:example.invalid", RoomTags::default())],
        ..AppState::default()
    }
}

fn room(room_id: &str, tags: RoomTags) -> RoomSummary {
    RoomSummary {
        room_id: room_id.to_owned(),
        display_name: "Room".to_owned(),
        display_label: "Room".to_owned(),
        original_display_label: "Room".to_owned(),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags,
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        marked_unread: false,
        last_activity_ms: 0,
        parent_space_ids: Vec::new(),
    }
}

fn tag_info(order: &str) -> RoomTagInfo {
    RoomTagInfo {
        order: Some(order.to_owned()),
    }
}

#[test]
fn room_tag_snapshot_updates_are_rust_owned_and_require_ready_session() {
    let mut state = ready_state();

    let effects = reduce(
        &mut state,
        AppAction::RoomTagsUpdated {
            room_id: "!room:example.invalid".to_owned(),
            tags: RoomTags {
                favourite: Some(tag_info("0.4")),
                low_priority: None,
            },
        },
    );

    assert_eq!(state.rooms[0].tags.favourite, Some(tag_info("0.4")));
    assert_eq!(
        effects,
        vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
    );

    let mut signed_out = AppState {
        rooms: vec![room("!room:example.invalid", RoomTags::default())],
        ..AppState::default()
    };
    let effects = reduce(
        &mut signed_out,
        AppAction::RoomTagsUpdated {
            room_id: "!room:example.invalid".to_owned(),
            tags: RoomTags {
                favourite: Some(tag_info("0.7")),
                low_priority: None,
            },
        },
    );

    assert_eq!(signed_out.rooms[0].tags, RoomTags::default());
    assert!(effects.is_empty());
}

#[test]
fn room_tag_set_and_remove_are_mutually_exclusive_for_favourite_and_low_priority() {
    let mut state = ready_state();

    reduce(
        &mut state,
        AppAction::RoomTagSet {
            room_id: "!room:example.invalid".to_owned(),
            tag: RoomTagKind::LowPriority,
            info: tag_info("0.8"),
        },
    );
    assert_eq!(state.rooms[0].tags.low_priority, Some(tag_info("0.8")));
    assert_eq!(state.rooms[0].tags.favourite, None);

    reduce(
        &mut state,
        AppAction::RoomTagSet {
            room_id: "!room:example.invalid".to_owned(),
            tag: RoomTagKind::Favourite,
            info: tag_info("0.2"),
        },
    );
    assert_eq!(state.rooms[0].tags.favourite, Some(tag_info("0.2")));
    assert_eq!(state.rooms[0].tags.low_priority, None);

    reduce(
        &mut state,
        AppAction::RoomTagRemoved {
            room_id: "!room:example.invalid".to_owned(),
            tag: RoomTagKind::Favourite,
        },
    );
    assert_eq!(state.rooms[0].tags, RoomTags::default());
}

#[test]
fn room_tag_updates_for_unknown_rooms_are_ignored() {
    let mut state = ready_state();

    let effects = reduce(
        &mut state,
        AppAction::RoomTagSet {
            room_id: "!missing:example.invalid".to_owned(),
            tag: RoomTagKind::Favourite,
            info: tag_info("0.1"),
        },
    );

    assert_eq!(state.rooms[0].tags, RoomTags::default());
    assert!(effects.is_empty());
}
