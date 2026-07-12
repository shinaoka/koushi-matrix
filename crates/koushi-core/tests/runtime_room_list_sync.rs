//! Runtime room-list projection tests.

use koushi_core::command::{AppCommand, CoreCommand};
use koushi_core::runtime::CoreRuntime;
use koushi_state::{AppAction, RoomListFilter, SessionState};

mod support;
use support::*;

#[tokio::test]
async fn select_room_list_filter_command_updates_projection_through_runtime() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(restore_ready_actions![AppAction::RoomListUpdated {
            spaces: vec![],
            rooms: vec![
                unread_room_summary("!room:example.test", 5),
                unread_room_summary("!dm:example.test", 0),
            ],
        },])
        .await;
    wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_)) && state.rooms.len() == 2
    })
    .await;

    let request_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::SelectRoomListFilter {
        request_id,
        filter: RoomListFilter::Unread,
    }))
    .await
    .expect("select room list filter command");

    let snapshot = wait_for_state(&mut conn, |state| {
        state.room_list.active_filter == RoomListFilter::Unread
            && state.room_list.items.len() == 1
            && state.room_list.items[0].room_id == "!room:example.test"
    })
    .await;
    assert_eq!(snapshot.room_list.active_filter, RoomListFilter::Unread);
    assert_eq!(
        snapshot
            .room_list
            .items
            .iter()
            .map(|item| item.room_id.as_str())
            .collect::<Vec<_>>(),
        vec!["!room:example.test"]
    );
}

#[tokio::test]
async fn mark_as_read_and_unread_success_actions_update_room_list_projection() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(restore_ready_actions![AppAction::RoomListUpdated {
            spaces: vec![],
            rooms: vec![unread_room_summary("!room:example.test", 3)],
        },])
        .await;
    wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_)) && state.rooms.len() == 1
    })
    .await;

    runtime
        .inject_actions(vec![AppAction::RoomMarkedAsUnreadSucceeded {
            request_id: 1,
            room_id: "!room:example.test".to_owned(),
            unread: true,
        }])
        .await;
    let snapshot = wait_for_state(&mut conn, |state| {
        state
            .rooms
            .iter()
            .any(|room| room.room_id == "!room:example.test" && room.marked_unread)
    })
    .await;
    assert!(
        snapshot
            .rooms
            .iter()
            .any(|room| room.room_id == "!room:example.test" && room.marked_unread)
    );

    runtime
        .inject_actions(vec![AppAction::RoomMarkedAsReadSucceeded {
            request_id: 2,
            room_id: "!room:example.test".to_owned(),
        }])
        .await;
    let snapshot = wait_for_state(&mut conn, |state| {
        state
            .rooms
            .iter()
            .any(|room| room.room_id == "!room:example.test" && !room.marked_unread)
    })
    .await;
    assert!(
        snapshot
            .rooms
            .iter()
            .any(|room| room.room_id == "!room:example.test" && !room.marked_unread)
    );
    assert_eq!(
        snapshot
            .rooms
            .iter()
            .find(|room| room.room_id == "!room:example.test")
            .unwrap()
            .unread_count,
        0
    );
}
