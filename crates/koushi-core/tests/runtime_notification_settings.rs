//! Runtime notification settings integration tests.

use std::time::Duration;

use koushi_core::command::{AppCommand, CoreCommand, TimelineCommand};
use koushi_core::event::CoreEvent;
use koushi_core::executor;
use koushi_core::ids::{AccountKey, TimelineKey, TimelineKind};
use koushi_core::runtime::CoreRuntime;
use koushi_state::{
    AppAction, NotificationSettings, RoomNotificationMode, RoomNotificationModeOperation,
    SessionState, SettingsPatch,
};

mod support;
use support::*;

#[tokio::test]
async fn set_room_notification_mode_for_known_room_projects_pending_then_completed() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(restore_ready_actions![AppAction::RoomListUpdated {
            spaces: vec![],
            rooms: vec![room_summary("!room:example.test")],
        },])
        .await;

    wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_))
            && state
                .rooms
                .iter()
                .any(|room| room.room_id == "!room:example.test")
    })
    .await;

    // The SDK is not available in unit tests, so synthesise the expected
    // action sequence directly through the reducer.
    let request_id = 42;
    runtime
        .inject_actions(vec![
            AppAction::RoomNotificationModeSet {
                request_id,
                room_id: "!room:example.test".to_owned(),
                mode: RoomNotificationMode::Mentions,
            },
            AppAction::RoomNotificationModeCompleted {
                request_id,
                room_id: "!room:example.test".to_owned(),
            },
        ])
        .await;

    let snapshot = wait_for_state(&mut conn, |state| {
        state
            .room_notification_settings
            .get("!room:example.test")
            .is_some_and(|settings| {
                settings.mode == RoomNotificationMode::Mentions
                    && settings.operation == RoomNotificationModeOperation::Idle
            })
    })
    .await;

    assert_eq!(
        snapshot
            .room_notification_settings
            .get("!room:example.test")
            .unwrap()
            .mode,
        RoomNotificationMode::Mentions
    );
}

#[tokio::test]
async fn set_room_notification_mode_for_unknown_room_is_ignored_by_reducer() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(restore_ready_actions![AppAction::RoomListUpdated {
            spaces: vec![],
            rooms: vec![room_summary("!room:example.test")],
        },])
        .await;

    wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_))
    })
    .await;

    runtime
        .inject_actions(vec![AppAction::RoomNotificationModeSet {
            request_id: 1,
            room_id: "!unknown:example.test".to_owned(),
            mode: RoomNotificationMode::Mute,
        }])
        .await;

    // Give the reducer a chance to process.
    executor::sleep(Duration::from_millis(50)).await;

    let snapshot = conn.snapshot();
    assert!(
        !snapshot
            .room_notification_settings
            .contains_key("!unknown:example.test")
    );
}

#[tokio::test]
async fn privacy_settings_patch_gates_read_receipt_dispatch() {
    let (_runtime, mut conn, _snapshot, _data_dir, _credential_dir) =
        ready_room_conn("!room:example.test").await;

    // Confirm the timeline command path is alive by submitting a subscribe.
    // It fails because there is no real SDK session, but it proves commands are
    // dispatched when not suppressed.
    let subscribe_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
        request_id: subscribe_id,
        key: TimelineKey {
            account_key: AccountKey("@alice:example.test".to_owned()),
            kind: TimelineKind::Room {
                room_id: "!room:example.test".to_owned(),
            },
        },
    }))
    .await
    .expect("submit subscribe");

    let saw_subscribe_failure = executor::timeout(Duration::from_millis(500), async {
        loop {
            if let CoreEvent::OperationFailed { request_id, .. } =
                conn.recv_event().await.expect("event")
            {
                if request_id == subscribe_id {
                    return true;
                }
            }
        }
    })
    .await
    .is_ok();
    assert!(
        saw_subscribe_failure,
        "subscribe command should be dispatched when not suppressed"
    );

    // Disable read receipts.
    let patch = SettingsPatch {
        notifications: Some(NotificationSettings {
            desktop_notifications: true,
            sound: true,
            badges: true,
            send_read_receipts: false,
            send_typing_notifications: true,
        }),
        ..Default::default()
    };
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::UpdateSettings {
        request_id,
        patch,
    }))
    .await
    .expect("submit settings");
    wait_for_state(&mut conn, |state| {
        !state.settings.values.notifications.send_read_receipts
    })
    .await;

    // Now the read receipt command must be silently suppressed by the runtime gate.
    let second = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::SendReadReceipt {
        request_id: second,
        key: TimelineKey {
            account_key: AccountKey("@alice:example.test".to_owned()),
            kind: TimelineKind::Room {
                room_id: "!room:example.test".to_owned(),
            },
        },
        event_id: "$event:example.test".to_owned(),
    }))
    .await
    .expect("submit");

    let suppressed = executor::timeout(Duration::from_millis(200), async {
        loop {
            if let CoreEvent::OperationFailed { request_id, .. } =
                conn.recv_event().await.expect("event")
            {
                if request_id == second {
                    return false;
                }
            }
        }
    })
    .await
    .is_err();
    assert!(
        suppressed,
        "read receipt should be suppressed when disabled"
    );
}

#[tokio::test]
async fn privacy_settings_patch_gates_typing_dispatch() {
    let (_runtime, mut conn, _snapshot, _data_dir, _credential_dir) =
        ready_room_conn("!room:example.test").await;

    // Confirm the timeline command path is alive.
    let subscribe_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
        request_id: subscribe_id,
        key: TimelineKey {
            account_key: AccountKey("@alice:example.test".to_owned()),
            kind: TimelineKind::Room {
                room_id: "!room:example.test".to_owned(),
            },
        },
    }))
    .await
    .expect("submit subscribe");

    let saw_subscribe_failure = executor::timeout(Duration::from_millis(500), async {
        loop {
            if let CoreEvent::OperationFailed { request_id, .. } =
                conn.recv_event().await.expect("event")
            {
                if request_id == subscribe_id {
                    return true;
                }
            }
        }
    })
    .await
    .is_ok();
    assert!(
        saw_subscribe_failure,
        "subscribe command should be dispatched when not suppressed"
    );

    // Disable typing notifications.
    let patch = SettingsPatch {
        notifications: Some(NotificationSettings {
            desktop_notifications: true,
            sound: true,
            badges: true,
            send_read_receipts: true,
            send_typing_notifications: false,
        }),
        ..Default::default()
    };
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::UpdateSettings {
        request_id,
        patch,
    }))
    .await
    .expect("submit settings");
    wait_for_state(&mut conn, |state| {
        !state
            .settings
            .values
            .notifications
            .send_typing_notifications
    })
    .await;

    let typed = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::SetTyping {
        request_id: typed,
        key: TimelineKey {
            account_key: AccountKey("@alice:example.test".to_owned()),
            kind: TimelineKind::Room {
                room_id: "!room:example.test".to_owned(),
            },
        },
        is_typing: true,
    }))
    .await
    .expect("submit");

    let suppressed = executor::timeout(Duration::from_millis(200), async {
        loop {
            if let CoreEvent::OperationFailed { request_id, .. } =
                conn.recv_event().await.expect("event")
            {
                if request_id == typed {
                    return false;
                }
            }
        }
    })
    .await
    .is_err();
    assert!(
        suppressed,
        "typing notice should be suppressed when disabled"
    );
}
