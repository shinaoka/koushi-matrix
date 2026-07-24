//! Runtime integration tests for scheduled send command handling.

use std::time::Duration;

use koushi_core::command::{AppCommand, CoreCommand};
use koushi_core::event::CoreEvent;
use koushi_core::executor;
use koushi_core::failure::{CoreFailure, TimelineFailureKind};
use koushi_core::runtime::CoreRuntime;
use koushi_state::{
    AppAction, ComposerDraftRevision, ScheduledSendCapability, ScheduledSendHandle,
    ScheduledSendItem, SessionState,
};

mod support;
use support::*;

#[tokio::test]
async fn composer_revision_exhaustion_blocks_scheduled_send_before_store_or_matrix_side_effect() {
    let (runtime, mut conn, _) = ready_room_conn("!room:example.test").await;
    runtime
        .inject_actions(vec![AppAction::ScheduledSendCapabilityChanged {
            capability: ScheduledSendCapability::LocalFallback,
        }])
        .await;
    submit_composer_command(
        &conn,
        CoreCommand::App(AppCommand::SetComposerDraft {
            request_id: conn.next_request_id(),
            expected_account: session_key(),
            room_id: "!room:example.test".to_owned(),
            draft: "keep scheduled draft".to_owned(),
            revision: ComposerDraftRevision::MAX,
        }),
    )
    .await
    .expect("seed maximum scheduled draft revision");
    wait_for_state(&mut conn, |state| {
        state.composer_drafts.room_revision("!room:example.test") == ComposerDraftRevision::MAX
            && state.timeline.scheduled_send_capability == ScheduledSendCapability::LocalFallback
    })
    .await;

    let request_id = conn.next_request_id();
    submit_composer_command(
        &conn,
        CoreCommand::App(AppCommand::ScheduleSend {
            request_id,
            expected_account: session_key(),
            room_id: "!room:example.test".to_owned(),
            thread_root_event_id: None,
            body: "must not schedule".to_owned(),
            send_at_ms: future_epoch_ms(Duration::from_secs(60)),
            draft_revision: ComposerDraftRevision::MAX,
        }),
    )
    .await
    .expect("submit exhausted scheduled send");

    let event = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            match conn.recv_event().await.expect("runtime event stream") {
                event @ CoreEvent::OperationFailed {
                    request_id: failed_request_id,
                    ..
                } if failed_request_id == request_id => break event,
                _ => continue,
            }
        }
    })
    .await
    .expect("exhausted schedule rejection should be correlated");
    assert!(matches!(
        event,
        CoreEvent::OperationFailed {
            failure: CoreFailure::TimelineOperationFailed {
                kind: TimelineFailureKind::ComposerRevisionExhausted,
            },
            ..
        }
    ));
    let snapshot = conn.snapshot();
    assert!(snapshot.timeline.scheduled_sends.is_empty());
    assert_eq!(snapshot.timeline.composer.draft, "keep scheduled draft");
    assert_eq!(
        snapshot.timeline.composer.draft_revision,
        ComposerDraftRevision::MAX
    );
    assert!(
        snapshot
            .timeline
            .composer
            .last_accepted_clear_revision
            .is_zero()
    );
    drop(runtime);
}

#[tokio::test]
async fn schedule_send_rejects_a_command_captured_for_another_account() {
    let (runtime, mut conn, _) = ready_room_conn("!room:example.test").await;
    runtime
        .inject_actions(vec![AppAction::ScheduledSendCapabilityChanged {
            capability: ScheduledSendCapability::LocalFallback,
        }])
        .await;
    wait_for_state(&mut conn, |state| {
        state.timeline.scheduled_send_capability == ScheduledSendCapability::LocalFallback
    })
    .await;
    let wrong_account = koushi_key::SessionKeyId {
        homeserver: "https://other.example.test".to_owned(),
        user_id: "@alice:example.test".to_owned(),
        device_id: "OTHER-DEVICE".to_owned(),
    };

    submit_composer_command(
        &conn,
        CoreCommand::App(AppCommand::ScheduleSend {
            request_id: conn.next_request_id(),
            expected_account: wrong_account,
            room_id: "!room:example.test".to_owned(),
            thread_root_event_id: None,
            body: "must not cross accounts".to_owned(),
            send_at_ms: future_epoch_ms(Duration::from_secs(60)),
            draft_revision: 0.into(),
        }),
    )
    .await
    .expect("submit stale-account schedule");
    submit_composer_command(
        &conn,
        CoreCommand::App(AppCommand::ScheduleSend {
            request_id: conn.next_request_id(),
            expected_account: session_key(),
            room_id: "!room:example.test".to_owned(),
            thread_root_event_id: None,
            body: "current account schedule".to_owned(),
            send_at_ms: future_epoch_ms(Duration::from_secs(60)),
            draft_revision: 0.into(),
        }),
    )
    .await
    .expect("submit current-account schedule");

    let snapshot =
        wait_for_state(&mut conn, |state| state.timeline.scheduled_sends.len() == 1).await;
    assert_eq!(
        snapshot.timeline.scheduled_sends[0].body,
        "current account schedule"
    );
}

#[tokio::test]
async fn app_command_schedules_cancel_and_reschedules_local_fallback_send() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(restore_ready_actions![
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

    submit_composer_command(
        &conn,
        CoreCommand::App(AppCommand::ScheduleSend {
            request_id: conn.next_request_id(),
            expected_account: session_key(),
            room_id: "!room:example.test".to_owned(),
            thread_root_event_id: None,
            body: "scheduled body".to_owned(),
            send_at_ms: future_epoch_ms(Duration::from_secs(60)),
            draft_revision: 1.into(),
        }),
    )
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
async fn local_fallback_scheduled_send_ids_do_not_reuse_fresh_runtime_request_ids() {
    async fn schedule_from_fresh_runtime() -> String {
        let runtime = CoreRuntime::start();
        let mut conn = runtime.attach();
        inject_ready_local_fallback_room(&runtime, "!room:example.test").await;
        wait_for_state(&mut conn, |state| {
            matches!(state.session, SessionState::Ready(_))
                && state.timeline.room_id.as_deref() == Some("!room:example.test")
        })
        .await;
        submit_composer_command(
            &conn,
            CoreCommand::App(AppCommand::ScheduleSend {
                request_id: conn.next_request_id(),
                expected_account: session_key(),
                room_id: "!room:example.test".to_owned(),
                thread_root_event_id: None,
                body: "unique scheduled message".to_owned(),
                send_at_ms: future_epoch_ms(Duration::from_secs(60)),
                draft_revision: 0.into(),
            }),
        )
        .await
        .expect("schedule send");
        let snapshot =
            wait_for_state(&mut conn, |state| state.timeline.scheduled_sends.len() == 1).await;
        snapshot.timeline.scheduled_sends[0].scheduled_id.clone()
    }

    assert_ne!(
        schedule_from_fresh_runtime().await,
        schedule_from_fresh_runtime().await
    );
}

#[tokio::test]
async fn local_fallback_scheduled_send_is_retained_when_delivery_cannot_start() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(restore_ready_actions![
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

    let send_at_ms = future_epoch_ms(Duration::from_millis(60));
    submit_composer_command(
        &conn,
        CoreCommand::App(AppCommand::ScheduleSend {
            request_id: conn.next_request_id(),
            expected_account: session_key(),
            room_id: "!room:example.test".to_owned(),
            thread_root_event_id: None,
            body: "retry instead of drop".to_owned(),
            send_at_ms,
            draft_revision: 0.into(),
        }),
    )
    .await
    .expect("schedule send");

    wait_for_state(&mut conn, |state| state.timeline.scheduled_sends.len() == 1).await;
    let snapshot = wait_for_state_for(&mut conn, Duration::from_secs(3), |state| {
        state
            .scheduled_sends
            .items
            .values()
            .any(|item| item.body == "retry instead of drop" && item.send_at_ms > send_at_ms)
    })
    .await;
    assert_eq!(snapshot.scheduled_sends.items.len(), 1);
}

#[tokio::test]
async fn server_scheduled_send_items_are_not_dispatched_by_local_fallback_timer() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();
    runtime
        .inject_actions(restore_ready_actions![
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
                    thread_root_event_id: None,
                    body: "server delayed body".to_owned(),
                    send_at_ms: future_epoch_ms(Duration::from_millis(20)),
                    handle: ScheduledSendHandle::Server {
                        delay_id: "delay-private".to_owned(),
                    },
                    is_dispatching: false,
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

#[tokio::test]
async fn local_fallback_scheduled_sends_persist_and_load_on_restart() {
    let data_dir = tempfile::tempdir().expect("data dir");
    let credential_dir = tempfile::tempdir().expect("credential dir");
    let send_at_ms = future_epoch_ms(Duration::from_secs(120));

    {
        let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
            data_dir.path().to_path_buf(),
            credential_dir.path().to_path_buf(),
        );
        let mut conn = runtime.attach();
        inject_ready_local_fallback_room(&runtime, "!room:example.test").await;
        wait_for_state(&mut conn, |state| {
            matches!(state.session, SessionState::Ready(_))
                && state.timeline.room_id.as_deref() == Some("!room:example.test")
                && state.timeline.scheduled_send_capability
                    == ScheduledSendCapability::LocalFallback
        })
        .await;

        submit_composer_command(
            &conn,
            CoreCommand::App(AppCommand::ScheduleSend {
                request_id: conn.next_request_id(),
                expected_account: session_key(),
                room_id: "!room:example.test".to_owned(),
                thread_root_event_id: None,
                body: "survives restart".to_owned(),
                send_at_ms,
                draft_revision: 0.into(),
            }),
        )
        .await
        .expect("schedule send");

        wait_for_state(&mut conn, |state| state.timeline.scheduled_sends.len() == 1).await;
    }

    let restarted = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    let mut conn = restarted.attach();
    inject_ready_local_fallback_room(&restarted, "!room:example.test").await;

    let snapshot = wait_for_state(&mut conn, |state| {
        state
            .timeline
            .scheduled_sends
            .first()
            .is_some_and(|item| item.body == "survives restart")
    })
    .await;
    assert_eq!(snapshot.timeline.scheduled_sends[0].send_at_ms, send_at_ms);
}

#[tokio::test]
async fn local_fallback_scheduled_sends_survive_a_session_lock() {
    let data_dir = tempfile::tempdir().expect("data dir");
    let credential_dir = tempfile::tempdir().expect("credential dir");
    let send_at_ms = future_epoch_ms(Duration::from_secs(120));

    {
        let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
            data_dir.path().to_path_buf(),
            credential_dir.path().to_path_buf(),
        );
        let mut conn = runtime.attach();
        inject_ready_local_fallback_room(&runtime, "!room:example.test").await;
        wait_for_state(&mut conn, |state| {
            matches!(state.session, SessionState::Ready(_))
                && state.timeline.room_id.as_deref() == Some("!room:example.test")
        })
        .await;

        submit_composer_command(
            &conn,
            CoreCommand::App(AppCommand::ScheduleSend {
                request_id: conn.next_request_id(),
                expected_account: session_key(),
                room_id: "!room:example.test".to_owned(),
                thread_root_event_id: None,
                body: "survives lock".to_owned(),
                send_at_ms,
                draft_revision: 0.into(),
            }),
        )
        .await
        .expect("schedule send");
        wait_for_state(&mut conn, |state| state.timeline.scheduled_sends.len() == 1).await;

        runtime.inject_actions(vec![AppAction::SessionLocked]).await;
        wait_for_state(&mut conn, |state| {
            matches!(state.session, SessionState::Locked(_))
                && state.scheduled_sends.items.is_empty()
        })
        .await;
    }

    let restarted = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    let mut conn = restarted.attach();
    inject_ready_local_fallback_room(&restarted, "!room:example.test").await;

    let snapshot = wait_for_state(&mut conn, |state| {
        state
            .timeline
            .scheduled_sends
            .first()
            .is_some_and(|item| item.body == "survives lock")
    })
    .await;
    assert_eq!(snapshot.timeline.scheduled_sends[0].send_at_ms, send_at_ms);
}

#[tokio::test]
async fn due_local_fallback_scheduled_send_is_retained_for_retry_after_restart() {
    let data_dir = tempfile::tempdir().expect("data dir");
    let credential_dir = tempfile::tempdir().expect("credential dir");
    let send_at_ms = future_epoch_ms(Duration::from_millis(1_500));

    {
        let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
            data_dir.path().to_path_buf(),
            credential_dir.path().to_path_buf(),
        );
        let mut conn = runtime.attach();
        inject_ready_local_fallback_room(&runtime, "!room:example.test").await;
        wait_for_state(&mut conn, |state| {
            matches!(state.session, SessionState::Ready(_))
                && state.timeline.room_id.as_deref() == Some("!room:example.test")
                && state.timeline.scheduled_send_capability
                    == ScheduledSendCapability::LocalFallback
        })
        .await;

        submit_composer_command(
            &conn,
            CoreCommand::App(AppCommand::ScheduleSend {
                request_id: conn.next_request_id(),
                expected_account: session_key(),
                room_id: "!room:example.test".to_owned(),
                thread_root_event_id: None,
                body: "retry after restart".to_owned(),
                send_at_ms,
                draft_revision: 0.into(),
            }),
        )
        .await
        .expect("schedule send");

        wait_for_state(&mut conn, |state| state.timeline.scheduled_sends.len() == 1).await;
    }

    let restarted = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    let mut conn = restarted.attach();
    inject_ready_local_fallback_room(&restarted, "!room:example.test").await;

    wait_for_state(&mut conn, |state| {
        state
            .timeline
            .scheduled_sends
            .first()
            .is_some_and(|item| item.body == "retry after restart")
    })
    .await;

    let snapshot = wait_for_state_for(&mut conn, Duration::from_secs(3), |state| {
        state
            .scheduled_sends
            .items
            .values()
            .any(|item| item.body == "retry after restart" && item.send_at_ms > send_at_ms)
    })
    .await;
    assert_eq!(snapshot.scheduled_sends.items.len(), 1);
}

#[tokio::test]
async fn cancelled_local_fallback_scheduled_send_does_not_resurrect_on_restart() {
    let data_dir = tempfile::tempdir().expect("data dir");
    let credential_dir = tempfile::tempdir().expect("credential dir");

    {
        let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
            data_dir.path().to_path_buf(),
            credential_dir.path().to_path_buf(),
        );
        let mut conn = runtime.attach();
        inject_ready_local_fallback_room(&runtime, "!room:example.test").await;
        wait_for_state(&mut conn, |state| {
            matches!(state.session, SessionState::Ready(_))
                && state.timeline.room_id.as_deref() == Some("!room:example.test")
                && state.timeline.scheduled_send_capability
                    == ScheduledSendCapability::LocalFallback
        })
        .await;

        submit_composer_command(
            &conn,
            CoreCommand::App(AppCommand::ScheduleSend {
                request_id: conn.next_request_id(),
                expected_account: session_key(),
                room_id: "!room:example.test".to_owned(),
                thread_root_event_id: None,
                body: "cancel before restart".to_owned(),
                send_at_ms: future_epoch_ms(Duration::from_secs(120)),
                draft_revision: 0.into(),
            }),
        )
        .await
        .expect("schedule send");

        let scheduled =
            wait_for_state(&mut conn, |state| state.timeline.scheduled_sends.len() == 1).await;
        conn.command(CoreCommand::App(AppCommand::CancelScheduledSend {
            request_id: conn.next_request_id(),
            scheduled_id: scheduled.timeline.scheduled_sends[0].scheduled_id.clone(),
        }))
        .await
        .expect("cancel scheduled send");
        wait_for_state(&mut conn, |state| state.timeline.scheduled_sends.is_empty()).await;
    }

    let restarted = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    let mut conn = restarted.attach();
    inject_ready_local_fallback_room(&restarted, "!room:example.test").await;

    let snapshot = wait_for_state_for(&mut conn, Duration::from_millis(200), |state| {
        matches!(state.session, SessionState::Ready(_))
            && state.timeline.room_id.as_deref() == Some("!room:example.test")
            && state.timeline.scheduled_sends.is_empty()
            && state.scheduled_sends.items.is_empty()
    })
    .await;
    assert!(snapshot.scheduled_sends.items.is_empty());
}

async fn inject_ready_local_fallback_room(runtime: &CoreRuntime, room_id: &str) {
    runtime
        .inject_actions(restore_ready_actions![
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![room_summary(room_id)],
            },
            AppAction::SelectRoom {
                room_id: room_id.to_owned(),
            },
            AppAction::TimelineSubscribed {
                room_id: room_id.to_owned(),
            },
            AppAction::ScheduledSendCapabilityChanged {
                capability: ScheduledSendCapability::LocalFallback,
            },
        ])
        .await;
}

async fn wait_for_state_for<F>(
    connection: &mut koushi_core::runtime::CoreConnection,
    timeout: Duration,
    predicate: F,
) -> koushi_state::AppState
where
    F: Fn(&koushi_state::AppState) -> bool,
{
    let attempts = (timeout.as_millis() / 5).max(1) as usize;
    for _ in 0..attempts {
        let snapshot = connection.snapshot();
        if predicate(&snapshot) {
            return snapshot;
        }
        executor::sleep(Duration::from_millis(5)).await;
    }
    panic!("state predicate was not satisfied within {timeout:?}");
}
