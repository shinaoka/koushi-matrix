//! Runtime room preference persistence integration tests.

use std::time::Duration;

use koushi_core::command::{AppCommand, CoreCommand};
use koushi_core::{CoreRuntime, executor};
use koushi_state::{AppAction, RoomNotificationMode, RoomNotificationModeOperation, SessionState};

mod support;
use support::*;

#[tokio::test]
async fn room_url_preview_override_persists_when_runtime_restarts() {
    let data_dir = tempfile::tempdir().expect("data dir");
    let credential_dir = tempfile::tempdir().expect("credential dir");
    {
        let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
            data_dir.path().to_path_buf(),
            credential_dir.path().to_path_buf(),
        );
        let mut connection = runtime.attach();
        runtime
            .inject_actions(restore_ready_actions![AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![room_summary("!room:example.test")],
            },])
            .await;
        wait_for_state(&mut connection, |state| {
            matches!(state.session, SessionState::Ready(_))
                && state
                    .rooms
                    .iter()
                    .any(|room| room.room_id == "!room:example.test")
        })
        .await;

        let request_id = connection.next_request_id();
        connection
            .command(CoreCommand::App(AppCommand::SetRoomUrlPreviewOverride {
                request_id,
                room_id: "!room:example.test".to_owned(),
                enabled: false,
            }))
            .await
            .expect("submit room URL preview override");

        wait_for_state(&mut connection, |state| {
            state
                .room_preferences
                .rooms
                .get("!room:example.test")
                .is_some_and(|preference| preference.url_previews_enabled_override == Some(false))
        })
        .await;
    }

    let restarted = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    let mut connection = restarted.attach();
    restarted
        .inject_actions(restore_ready_actions![AppAction::RoomListUpdated {
            spaces: vec![],
            rooms: vec![room_summary("!room:example.test")],
        },])
        .await;

    let snapshot = executor::timeout(Duration::from_secs(1), async {
        wait_for_state(&mut connection, |state| {
            state
                .link_preview_settings
                .room_overrides
                .get("!room:example.test")
                == Some(&false)
        })
        .await
    })
    .await
    .expect("persisted room preference should be restored after room list reload");

    assert_eq!(
        snapshot
            .room_preferences
            .rooms
            .get("!room:example.test")
            .and_then(|preference| preference.url_previews_enabled_override),
        Some(false)
    );
}

#[tokio::test]
async fn room_notification_mode_persists_when_runtime_restarts() {
    let data_dir = tempfile::tempdir().expect("data dir");
    let credential_dir = tempfile::tempdir().expect("credential dir");
    {
        let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
            data_dir.path().to_path_buf(),
            credential_dir.path().to_path_buf(),
        );
        let mut connection = runtime.attach();
        runtime
            .inject_actions(restore_ready_actions![AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![room_summary("!room:example.test")],
            },])
            .await;
        wait_for_state(&mut connection, |state| {
            matches!(state.session, SessionState::Ready(_))
                && state
                    .rooms
                    .iter()
                    .any(|room| room.room_id == "!room:example.test")
        })
        .await;

        runtime
            .inject_actions(vec![
                AppAction::RoomNotificationModeSet {
                    request_id: 11,
                    room_id: "!room:example.test".to_owned(),
                    mode: RoomNotificationMode::Mute,
                },
                AppAction::RoomNotificationModeCompleted {
                    request_id: 11,
                    room_id: "!room:example.test".to_owned(),
                },
            ])
            .await;

        wait_for_state(&mut connection, |state| {
            state
                .room_preferences
                .rooms
                .get("!room:example.test")
                .is_some_and(|preference| {
                    preference.notification_mode == Some(RoomNotificationMode::Mute)
                })
        })
        .await;
    }

    let restarted = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    let mut connection = restarted.attach();
    restarted
        .inject_actions(restore_ready_actions![AppAction::RoomListUpdated {
            spaces: vec![],
            rooms: vec![room_summary("!room:example.test")],
        },])
        .await;

    let snapshot = executor::timeout(Duration::from_secs(1), async {
        wait_for_state(&mut connection, |state| {
            state
                .room_notification_settings
                .get("!room:example.test")
                .is_some_and(|settings| {
                    settings.mode == RoomNotificationMode::Mute
                        && settings.operation == RoomNotificationModeOperation::Idle
                })
        })
        .await
    })
    .await
    .expect("persisted room notification preference should be restored after restart");

    assert_eq!(
        snapshot
            .room_preferences
            .rooms
            .get("!room:example.test")
            .and_then(|preference| preference.notification_mode),
        Some(RoomNotificationMode::Mute)
    );
}
