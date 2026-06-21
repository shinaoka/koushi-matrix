//! Runtime navigation persistence integration tests.

use std::time::Duration;

use koushi_core::{CoreRuntime, executor};
use koushi_state::{AppAction, RoomSummary, SessionState, SpaceSummary};

mod support;
use support::*;

#[tokio::test]
async fn navigation_selection_persists_when_runtime_restarts() {
    let data_dir = tempfile::tempdir().expect("data dir");
    {
        let runtime = CoreRuntime::start_with_data_dir(data_dir.path().to_path_buf());
        let mut connection = runtime.attach();
        runtime
            .inject_actions(vec![
                AppAction::RestoreSessionSucceeded(session_info()),
                AppAction::RoomListUpdated {
                    spaces: vec![space_summary(
                        "!space-a:example.test",
                        &["!room-a:example.test"],
                    )],
                    rooms: vec![
                        room_in_space("!room-a:example.test", "!space-a:example.test"),
                        room_summary("!room-home:example.test"),
                    ],
                },
                AppAction::SelectSpace {
                    space_id: Some("!space-a:example.test".to_owned()),
                },
                AppAction::SelectRoom {
                    room_id: "!room-a:example.test".to_owned(),
                },
            ])
            .await;

        wait_for_state(&mut connection, |state| {
            state.navigation.active_space_id.as_deref() == Some("!space-a:example.test")
                && state.navigation.active_room_id.as_deref() == Some("!room-a:example.test")
        })
        .await;
    }

    let restarted = CoreRuntime::start_with_data_dir(data_dir.path().to_path_buf());
    let mut connection = restarted.attach();
    restarted
        .inject_actions(vec![
            AppAction::RestoreSessionSucceeded(session_info()),
            AppAction::RoomListUpdated {
                spaces: vec![space_summary(
                    "!space-a:example.test",
                    &["!room-a:example.test"],
                )],
                rooms: vec![
                    room_in_space("!room-a:example.test", "!space-a:example.test"),
                    room_summary("!room-home:example.test"),
                ],
            },
        ])
        .await;

    let snapshot = executor::timeout(Duration::from_secs(1), async {
        wait_for_state(&mut connection, |state| {
            matches!(state.session, SessionState::Ready(_))
                && state.navigation.active_space_id.as_deref() == Some("!space-a:example.test")
                && state.navigation.active_room_id.as_deref() == Some("!room-a:example.test")
        })
        .await
    })
    .await
    .expect("persisted navigation should be restored after room list reload");

    assert_eq!(
        snapshot
            .navigation
            .last_room_by_space_id
            .get("!space-a:example.test"),
        Some(&"!room-a:example.test".to_owned())
    );
}

fn space_summary(space_id: &str, child_room_ids: &[&str]) -> SpaceSummary {
    SpaceSummary {
        space_id: space_id.to_owned(),
        display_name: "QA Space".to_owned(),
        avatar: None,
        child_room_ids: child_room_ids
            .iter()
            .map(|room_id| (*room_id).to_owned())
            .collect(),
    }
}

fn room_in_space(room_id: &str, space_id: &str) -> RoomSummary {
    RoomSummary {
        parent_space_ids: vec![space_id.to_owned()],
        ..room_summary(room_id)
    }
}
