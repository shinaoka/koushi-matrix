//! Runtime integration tests for scheduled send command handling.

use std::time::Duration;

use matrix_desktop_core::command::{AppCommand, CoreCommand};
use matrix_desktop_core::executor;
use matrix_desktop_core::runtime::CoreRuntime;
use matrix_desktop_state::{
    AppAction, ScheduledSendCapability, ScheduledSendHandle, ScheduledSendItem, SessionState,
};

mod support;
use support::*;

#[tokio::test]
async fn app_command_schedules_cancel_and_reschedules_local_fallback_send() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(vec![
            AppAction::RestoreSessionSucceeded(session_info()),
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![room_summary("!room:example.test")],
            },
            AppAction::SelectRoom {
                room_id: "!room:example.test".to_owned(),
            },
            AppAction::TimelineSubscribed {
                room_id: "!room:example.test".to_owned(),
            },
            AppAction::ScheduledSendCapabilityChanged {
                capability: ScheduledSendCapability::LocalFallback,
            },
            AppAction::ComposerDraftChanged {
                room_id: "!room:example.test".to_owned(),
                draft: "scheduled body".to_owned(),
            },
        ])
        .await;
    wait_for_state(&mut conn, |state| {
        state.timeline.composer.draft == "scheduled body"
    })
    .await;

    conn.command(CoreCommand::App(AppCommand::ScheduleSend {
        request_id: conn.next_request_id(),
        room_id: "!room:example.test".to_owned(),
        body: "scheduled body".to_owned(),
        send_at_ms: future_epoch_ms(Duration::from_secs(60)),
    }))
    .await
    .expect("schedule send");

    let snapshot = wait_for_state(&mut conn, |state| {
        state.timeline.composer.draft.is_empty() && state.timeline.scheduled_sends.len() == 1
    })
    .await;
    assert_eq!(
        snapshot.timeline.scheduled_send_capability,
        ScheduledSendCapability::LocalFallback
    );
    assert_eq!(snapshot.timeline.scheduled_sends[0].body, "scheduled body");
    let scheduled_id = snapshot.timeline.scheduled_sends[0].scheduled_id.clone();

    conn.command(CoreCommand::App(AppCommand::RescheduleScheduledSend {
        request_id: conn.next_request_id(),
        scheduled_id: scheduled_id.clone(),
        send_at_ms: future_epoch_ms(Duration::from_secs(120)),
    }))
    .await
    .expect("reschedule send");

    let rescheduled =
        wait_for_state(&mut conn, |state| {
            state.timeline.scheduled_sends.first().is_some_and(|item| {
                item.send_at_ms > snapshot.timeline.scheduled_sends[0].send_at_ms
            })
        })
        .await;
    assert_eq!(
        rescheduled.timeline.scheduled_sends[0].scheduled_id,
        scheduled_id
    );

    conn.command(CoreCommand::App(AppCommand::CancelScheduledSend {
        request_id: conn.next_request_id(),
        scheduled_id,
    }))
    .await
    .expect("cancel scheduled send");

    wait_for_state(&mut conn, |state| state.timeline.scheduled_sends.is_empty()).await;
}

#[tokio::test]
async fn local_fallback_scheduled_send_fires_at_target_and_leaves_rust_state() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(vec![
            AppAction::RestoreSessionSucceeded(session_info()),
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![room_summary("!room:example.test")],
            },
            AppAction::SelectRoom {
                room_id: "!room:example.test".to_owned(),
            },
            AppAction::TimelineSubscribed {
                room_id: "!room:example.test".to_owned(),
            },
            AppAction::ScheduledSendCapabilityChanged {
                capability: ScheduledSendCapability::LocalFallback,
            },
        ])
        .await;
    wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_))
            && state.timeline.room_id.as_deref() == Some("!room:example.test")
    })
    .await;

    conn.command(CoreCommand::App(AppCommand::ScheduleSend {
        request_id: conn.next_request_id(),
        room_id: "!room:example.test".to_owned(),
        body: "fire later".to_owned(),
        send_at_ms: future_epoch_ms(Duration::from_millis(60)),
    }))
    .await
    .expect("schedule send");

    wait_for_state(&mut conn, |state| state.timeline.scheduled_sends.len() == 1).await;
    let snapshot =
        wait_for_state(&mut conn, |state| state.timeline.scheduled_sends.is_empty()).await;
    assert!(snapshot.scheduled_sends.items.is_empty());
}

#[tokio::test]
async fn server_scheduled_send_items_are_not_dispatched_by_local_fallback_timer() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(vec![
            AppAction::RestoreSessionSucceeded(session_info()),
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![room_summary("!room:example.test")],
            },
            AppAction::SelectRoom {
                room_id: "!room:example.test".to_owned(),
            },
            AppAction::TimelineSubscribed {
                room_id: "!room:example.test".to_owned(),
            },
            AppAction::ScheduledSendCapabilityChanged {
                capability: ScheduledSendCapability::ServerDelayedEvents,
            },
            AppAction::ScheduledSendCreated {
                item: ScheduledSendItem {
                    scheduled_id: "server-scheduled".to_owned(),
                    room_id: "!room:example.test".to_owned(),
                    body: "server delayed body".to_owned(),
                    send_at_ms: future_epoch_ms(Duration::from_millis(20)),
                    handle: ScheduledSendHandle::Server {
                        delay_id: "delay-private".to_owned(),
                    },
                },
            },
        ])
        .await;
    wait_for_state(&mut conn, |state| state.timeline.scheduled_sends.len() == 1).await;

    executor::sleep(Duration::from_millis(80)).await;
    let snapshot = conn.snapshot();
    assert_eq!(snapshot.timeline.scheduled_sends.len(), 1);
    assert_eq!(
        snapshot.timeline.scheduled_sends[0].handle,
        ScheduledSendHandle::Server {
            delay_id: "delay-private".to_owned()
        }
    );
}
