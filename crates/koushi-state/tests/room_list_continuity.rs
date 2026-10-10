use koushi_state::{
    AppAction, AppState, RoomListSource, RoomSummary, RoomTags, SessionAuthenticationMethod,
    SessionInfo, SessionState, SpaceSummary, reduce,
};

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://matrix.example.org".to_owned(),
            user_id: "@user-a:example.invalid".to_owned(),
            device_id: "DEVICE".to_owned(),
            authentication_method: SessionAuthenticationMethod::Unknown,
        }),
        ..AppState::default()
    }
}

fn test_space(space_id: &str) -> SpaceSummary {
    SpaceSummary {
        space_id: space_id.to_owned(),
        raw_name: None,
        display_name: space_id.to_owned(),
        avatar: None,
        join_rule: None,
        child_room_ids: Vec::new(),
        parent_side_child_room_ids: Vec::new(),
    }
}

fn test_room(room_id: &str) -> RoomSummary {
    RoomSummary {
        display_name_placeholder: None,
        display_label_placeholder: None,
        room_id: room_id.to_owned(),
        display_name: room_id.to_owned(),
        display_label: room_id.to_owned(),
        original_display_label: room_id.to_owned(),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: 0,
        notification_count: 0,
        highlight_count: 0,
        thread_unread_count: 0,
        thread_highlight_count: 0,
        marked_unread: false,
        recency_stamp: None,
        conversation_activity: None,
        latest_event: None,
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 0,
    }
}

fn test_room_in_space(room_id: &str, space_id: &str) -> RoomSummary {
    let mut room = test_room(room_id);
    room.parent_space_ids = vec![space_id.to_owned()];
    room
}

fn loading_room_list_with_active_space() -> AppState {
    let space_id = "!space:example.invalid";
    let room_one_id = "!room-one:example.invalid";
    let room_two_id = "!room-two:example.invalid";
    let mut space = test_space(space_id);
    space.child_room_ids = vec![room_one_id.to_owned(), room_two_id.to_owned()];
    let mut state = ready_state();

    reduce(
        &mut state,
        AppAction::RoomListBootstrapStarted {
            generation: 1,
            source: RoomListSource::Live,
        },
    );
    reduce(
        &mut state,
        AppAction::RoomListSnapshotAuthoritative {
            generation: 1,
            source: RoomListSource::Live,
            spaces: vec![space],
            rooms: vec![
                test_room_in_space(room_one_id, space_id),
                test_room_in_space(room_two_id, space_id),
            ],
            invites: Vec::new(),
        },
    );
    reduce(
        &mut state,
        AppAction::SelectSpace {
            space_id: Some(space_id.to_owned()),
        },
    );
    reduce(
        &mut state,
        AppAction::RoomListBootstrapStarted {
            generation: 2,
            source: RoomListSource::Live,
        },
    );

    state
}

#[test]
fn room_list_bootstrap_provisional_snapshot_preserves_space_and_active_selection() {
    let space_id = "!space:example.invalid";
    let room_one_id = "!room-one:example.invalid";
    let room_two_id = "!room-two:example.invalid";
    let mut room_one = test_room_in_space(room_one_id, space_id);
    room_one.display_name = "Refreshed room one".to_owned();
    let mut state = loading_room_list_with_active_space();

    reduce(
        &mut state,
        AppAction::RoomListSnapshotProvisional {
            generation: 2,
            source: RoomListSource::Live,
            spaces: Vec::new(),
            rooms: vec![room_one, test_room_in_space(room_two_id, space_id)],
            invites: Vec::new(),
        },
    );

    assert!(state.spaces.iter().any(|space| space.space_id == space_id));
    assert_eq!(state.navigation.active_space_id.as_deref(), Some(space_id));
    assert_eq!(
        state
            .rooms
            .iter()
            .find(|room| room.room_id == room_one_id)
            .map(|room| room.display_name.as_str()),
        Some("Refreshed room one")
    );
}

#[test]
fn room_list_bootstrap_provisional_snapshot_upserts_new_room_without_removing_prior_entries() {
    let space_id = "!space:example.invalid";
    let room_one_id = "!room-one:example.invalid";
    let room_two_id = "!room-two:example.invalid";
    let room_three_id = "!room-three:example.invalid";
    let mut state = loading_room_list_with_active_space();

    reduce(
        &mut state,
        AppAction::RoomListSnapshotProvisional {
            generation: 2,
            source: RoomListSource::Live,
            spaces: Vec::new(),
            rooms: vec![test_room_in_space(room_three_id, space_id)],
            invites: Vec::new(),
        },
    );

    assert!(state.spaces.iter().any(|space| space.space_id == space_id));
    for room_id in [room_one_id, room_two_id, room_three_id] {
        assert!(state.rooms.iter().any(|room| room.room_id == room_id));
    }
}

#[test]
fn room_list_bootstrap_provisional_room_with_known_space_id_is_not_duplicated() {
    let space_id = "!space:example.invalid";
    let mut state = loading_room_list_with_active_space();

    reduce(
        &mut state,
        AppAction::RoomListSnapshotProvisional {
            generation: 2,
            source: RoomListSource::Live,
            spaces: Vec::new(),
            rooms: vec![test_room(space_id)],
            invites: Vec::new(),
        },
    );

    assert_eq!(
        state
            .spaces
            .iter()
            .filter(|space| space.space_id == space_id)
            .count(),
        1
    );
    assert!(!state.rooms.iter().any(|room| room.room_id == space_id));
}

#[test]
fn room_list_bootstrap_provisional_space_reclassifies_retained_room() {
    let space_id = "!space:example.invalid";
    let mut state = ready_state();

    reduce(
        &mut state,
        AppAction::RoomListBootstrapStarted {
            generation: 1,
            source: RoomListSource::Live,
        },
    );
    reduce(
        &mut state,
        AppAction::RoomListSnapshotAuthoritative {
            generation: 1,
            source: RoomListSource::Live,
            spaces: Vec::new(),
            rooms: vec![test_room(space_id)],
            invites: Vec::new(),
        },
    );
    reduce(
        &mut state,
        AppAction::RoomListBootstrapStarted {
            generation: 2,
            source: RoomListSource::Live,
        },
    );

    reduce(
        &mut state,
        AppAction::RoomListSnapshotProvisional {
            generation: 2,
            source: RoomListSource::Live,
            spaces: vec![test_space(space_id)],
            rooms: Vec::new(),
            invites: Vec::new(),
        },
    );

    assert!(state.spaces.iter().any(|space| space.space_id == space_id));
    assert!(!state.rooms.iter().any(|room| room.room_id == space_id));
}

#[test]
fn room_list_bootstrap_authoritative_omission_removes_space_after_provisional_merge() {
    let space_id = "!space:example.invalid";
    let room_one_id = "!room-one:example.invalid";
    let room_two_id = "!room-two:example.invalid";
    let mut state = loading_room_list_with_active_space();

    reduce(
        &mut state,
        AppAction::RoomListSnapshotProvisional {
            generation: 2,
            source: RoomListSource::Live,
            spaces: Vec::new(),
            rooms: vec![
                test_room_in_space(room_one_id, space_id),
                test_room_in_space(room_two_id, space_id),
            ],
            invites: Vec::new(),
        },
    );
    assert!(state.spaces.iter().any(|space| space.space_id == space_id));

    reduce(
        &mut state,
        AppAction::RoomListSnapshotAuthoritative {
            generation: 2,
            source: RoomListSource::Live,
            spaces: Vec::new(),
            rooms: vec![
                test_room_in_space(room_one_id, space_id),
                test_room_in_space(room_two_id, space_id),
            ],
            invites: Vec::new(),
        },
    );

    assert!(state.spaces.is_empty());
    assert_eq!(state.navigation.active_space_id, None);
    assert_eq!(
        state.navigation.active_room_id.as_deref(),
        Some(room_one_id)
    );
    assert_eq!(state.timeline.room_id.as_deref(), Some(room_one_id));
}
