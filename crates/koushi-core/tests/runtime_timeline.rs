//! Runtime timeline / composer integration tests.

use koushi_core::command::{AppCommand, CoreCommand};
use koushi_core::executor;
use koushi_core::runtime::{COMPOSER_DRAFT_PERSIST_DEBOUNCE, CoreRuntime};
use koushi_state::{AppAction, ComposerMode, SessionState, ThreadPaneState};

mod support;
use support::*;

#[tokio::test]
async fn app_command_sets_and_clears_reply_target() {
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
        ])
        .await;
    wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_))
            && state.timeline.room_id.as_deref() == Some("!room:example.test")
    })
    .await;

    let set_request = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::SetComposerReplyTarget {
        request_id: set_request,
        room_id: "!room:example.test".to_owned(),
        event_id: "$root:example.test".to_owned(),
    }))
    .await
    .expect("set reply target command");

    let snapshot = wait_for_state(&mut conn, |state| {
        matches!(
            state.timeline.composer.mode,
            ComposerMode::Reply { ref in_reply_to_event_id }
                if in_reply_to_event_id == "$root:example.test"
        )
    })
    .await;
    assert!(matches!(
        snapshot.timeline.composer.mode,
        ComposerMode::Reply { .. }
    ));

    let cancel_request = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::CancelComposerReply {
        request_id: cancel_request,
    }))
    .await
    .expect("cancel reply target command");

    let snapshot = wait_for_state(&mut conn, |state| {
        state.timeline.composer.mode == ComposerMode::Plain
    })
    .await;
    assert_eq!(snapshot.timeline.composer.mode, ComposerMode::Plain);
}

#[tokio::test]
async fn app_command_sets_open_thread_composer_draft() {
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
            AppAction::OpenThread {
                room_id: "!room:example.test".to_owned(),
                root_event_id: "$root:example.test".to_owned(),
            },
            AppAction::ThreadSubscribed {
                room_id: "!room:example.test".to_owned(),
                root_event_id: "$root:example.test".to_owned(),
            },
        ])
        .await;
    wait_for_state(&mut conn, |state| {
        matches!(
            &state.thread,
            ThreadPaneState::Open {
                room_id,
                root_event_id,
                ..
            } if room_id == "!room:example.test" && root_event_id == "$root:example.test"
        )
    })
    .await;

    let request_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::SetThreadComposerDraft {
        request_id,
        room_id: "!room:example.test".to_owned(),
        root_event_id: "$root:example.test".to_owned(),
        draft: "thread draft".to_owned(),
    }))
    .await
    .expect("set thread composer draft command");

    let snapshot = wait_for_state(&mut conn, |state| {
        matches!(
            &state.thread,
            ThreadPaneState::Open { composer, .. }
                if composer.draft == "thread draft"
        )
    })
    .await;

    match snapshot.thread {
        ThreadPaneState::Open { composer, .. } => {
            assert_eq!(composer.draft, "thread draft");
        }
        other => panic!("expected open thread, got {other:?}"),
    }
}

#[tokio::test]
async fn app_command_sets_selected_room_composer_draft() {
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
        ])
        .await;
    wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_))
            && state.timeline.room_id.as_deref() == Some("!room:example.test")
    })
    .await;

    let request_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::SetComposerDraft {
        request_id,
        room_id: "!room:example.test".to_owned(),
        draft: "room draft".to_owned(),
    }))
    .await
    .expect("set room composer draft command");

    let snapshot = wait_for_state(&mut conn, |state| {
        state.timeline.composer.draft == "room draft"
    })
    .await;
    assert_eq!(snapshot.timeline.composer.draft, "room draft");
}

#[tokio::test]
async fn composer_drafts_persist_after_debounce_and_load_on_restart() {
    let data_dir = tempfile::tempdir().expect("data dir");
    let credential_dir = tempfile::tempdir().expect("credential dir");

    {
        let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
            data_dir.path().to_path_buf(),
            credential_dir.path().to_path_buf(),
        );
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
            ])
            .await;
        wait_for_state(&mut conn, |state| {
            matches!(state.session, SessionState::Ready(_))
                && state.timeline.room_id.as_deref() == Some("!room:example.test")
        })
        .await;

        conn.command(CoreCommand::App(AppCommand::SetComposerDraft {
            request_id: conn.next_request_id(),
            room_id: "!room:example.test".to_owned(),
            draft: "survives restart".to_owned(),
        }))
        .await
        .expect("set room composer draft");

        wait_for_state(&mut conn, |state| {
            state.timeline.composer.draft == "survives restart"
        })
        .await;
        executor::sleep(COMPOSER_DRAFT_PERSIST_DEBOUNCE * 2).await;
    }

    let restarted = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    let mut conn = restarted.attach();
    restarted
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
        ])
        .await;

    let snapshot = wait_for_state(&mut conn, |state| {
        state.timeline.composer.draft == "survives restart"
    })
    .await;
    assert_eq!(snapshot.timeline.composer.draft, "survives restart");
}

#[tokio::test]
async fn cleared_composer_drafts_do_not_resurrect_on_restart() {
    let data_dir = tempfile::tempdir().expect("data dir");
    let credential_dir = tempfile::tempdir().expect("credential dir");

    {
        let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
            data_dir.path().to_path_buf(),
            credential_dir.path().to_path_buf(),
        );
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
            ])
            .await;
        wait_for_state(&mut conn, |state| {
            matches!(state.session, SessionState::Ready(_))
                && state.timeline.room_id.as_deref() == Some("!room:example.test")
        })
        .await;

        conn.command(CoreCommand::App(AppCommand::SetComposerDraft {
            request_id: conn.next_request_id(),
            room_id: "!room:example.test".to_owned(),
            draft: "deleted before restart".to_owned(),
        }))
        .await
        .expect("set room composer draft");
        wait_for_state(&mut conn, |state| {
            state.timeline.composer.draft == "deleted before restart"
        })
        .await;
        executor::sleep(COMPOSER_DRAFT_PERSIST_DEBOUNCE * 2).await;

        conn.command(CoreCommand::App(AppCommand::SetComposerDraft {
            request_id: conn.next_request_id(),
            room_id: "!room:example.test".to_owned(),
            draft: String::new(),
        }))
        .await
        .expect("clear room composer draft");
        wait_for_state(&mut conn, |state| state.timeline.composer.draft.is_empty()).await;
        executor::sleep(COMPOSER_DRAFT_PERSIST_DEBOUNCE * 2).await;
    }

    let restarted = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    let mut conn = restarted.attach();
    restarted
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
        ])
        .await;

    let snapshot = wait_for_state(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_))
            && state.timeline.room_id.as_deref() == Some("!room:example.test")
            && state.timeline.is_subscribed
    })
    .await;
    assert!(snapshot.timeline.composer.draft.is_empty());
}

#[tokio::test]
async fn send_completion_clears_reply_mode_through_runtime() {
    // Regression: production send/reply completion must be Rust-owned. The core
    // drives SendTextSubmitted -> SendTextFinished into AppState so the composer
    // returns to Plain without React repairing product state after the fact.
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
            AppAction::ComposerReplyTargetSelected {
                room_id: "!room:example.test".to_owned(),
                event_id: "$root:example.test".to_owned(),
            },
        ])
        .await;
    wait_for_state(&mut conn, |state| {
        matches!(state.timeline.composer.mode, ComposerMode::Reply { .. })
    })
    .await;

    runtime
        .inject_actions(vec![
            AppAction::SendTextSubmitted {
                room_id: "!room:example.test".to_owned(),
                transaction_id: "txn-reply".to_owned(),
                body: "reply body".to_owned(),
            },
            AppAction::SendTextFinished {
                room_id: "!room:example.test".to_owned(),
                transaction_id: "txn-reply".to_owned(),
            },
        ])
        .await;

    let snapshot = wait_for_state(&mut conn, |state| {
        state.timeline.composer.pending_transaction_id.is_none()
            && state.timeline.composer.mode == ComposerMode::Plain
    })
    .await;
    assert_eq!(snapshot.timeline.composer.mode, ComposerMode::Plain);
    assert_eq!(snapshot.timeline.composer.pending_transaction_id, None);
}
