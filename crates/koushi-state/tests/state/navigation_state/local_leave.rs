use super::support::{ready_state, rooms, spaces};
use koushi_state::{AppAction, RoomListSource, reduce};

#[test]
fn space_local_leave_removes_space_immediately() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomListUpdated {
            spaces: spaces(),
            rooms: rooms(),
        },
    );

    // RoomActor delivers this action after LeaveSpace succeeds and residency acknowledges it.
    reduce(
        &mut state,
        AppAction::RoomLeftLocally {
            room_id: "space-a".to_owned(),
        },
    );

    assert!(
        state.spaces.is_empty(),
        "a successfully left Space must disappear immediately"
    );
    assert!(state.room_list.locally_left_room_ids.contains("space-a"));
    assert_eq!(
        state.rooms,
        rooms(),
        "leaving a Space alone does not leave its children"
    );
}

#[test]
fn space_local_leave_survives_stale_authoritative_room_list() {
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::RoomListBootstrapStarted {
            generation: 1,
            source: RoomListSource::Live,
        },
    );
    // Isolate list handling from the local-leave handler: both leaves already settled.
    state
        .room_list
        .locally_left_room_ids
        .extend(["space-a".to_owned(), "room-a".to_owned()]);
    state.rooms = rooms()
        .into_iter()
        .filter(|room| room.room_id != "room-a")
        .collect();

    reduce(
        &mut state,
        AppAction::RoomListSnapshotAuthoritative {
            generation: 1,
            source: RoomListSource::Live,
            spaces: spaces(),
            rooms: rooms(),
            invites: Vec::new(),
        },
    );

    assert!(
        state.spaces.is_empty(),
        "a queued pre-leave Space must not reappear"
    );
    assert!(!state.rooms.iter().any(|room| room.room_id == "room-a"));
    assert!(state.room_list.locally_left_room_ids.contains("space-a"));
    assert!(state.room_list.locally_left_room_ids.contains("room-a"));
    assert!(state.rooms.iter().any(|room| room.room_id == "dm-a"));
}
