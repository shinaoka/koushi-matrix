//! Runtime timeline / composer integration tests.

use koushi_core::command::{AppCommand, CoreCommand, TimelineCommand};
use koushi_core::event::CoreEvent;
use koushi_core::executor;
use koushi_core::failure::{CoreFailure, TimelineFailureKind};
use koushi_core::ids::{AccountKey, RequestId, TimelineKey, TimelineKind};
use koushi_core::runtime::{COMPOSER_DRAFT_PERSIST_DEBOUNCE, CoreRuntime};
use koushi_key::SessionKeyId;
use koushi_state::{
    AppAction, ComposerDraftRevision, ComposerDraftStore, ComposerMode, ComposerTarget,
    CurrentDeviceTrustState, MentionIntent, PreparedUploadFormat, PreparedUploadVariant,
    SessionInfo, SessionState, StagedUploadCompressionChoice, StagedUploadItem, StagedUploadKind,
    StagedUploadPreparation, SubmissionId, ThreadPaneState,
};

mod support;
use support::*;

fn draft_account() -> SessionKeyId {
    let info = session_info();
    SessionKeyId {
        homeserver: info.homeserver,
        user_id: info.user_id,
        device_id: info.device_id,
    }
}

fn runtime_with_file_credentials() -> (CoreRuntime, tempfile::TempDir, tempfile::TempDir) {
    let data_dir = tempfile::tempdir().expect("runtime data dir");
    let credential_dir = tempfile::tempdir().expect("runtime credential dir");
    let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_owned(),
        credential_dir.path().to_owned(),
    );
    (runtime, data_dir, credential_dir)
}

fn single_composer_draft_file(root: &std::path::Path) -> std::path::PathBuf {
    let mut pending = vec![root.to_path_buf()];
    let mut matches = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).expect("read synthetic data directory") {
            let entry = entry.expect("read synthetic data entry");
            let file_type = entry.file_type().expect("read synthetic data entry type");
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if entry.file_name() == "drafts.v1.enc" {
                matches.push(entry.path());
            }
        }
    }
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one synthetic composer payload"
    );
    matches.pop().expect("single composer payload")
}

async fn wait_for_operation_failure(
    connection: &mut koushi_core::runtime::CoreConnection,
    request_id: RequestId,
) -> CoreFailure {
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            match connection.recv_event().await.expect("runtime event stream") {
                CoreEvent::OperationFailed {
                    request_id: failed_request_id,
                    failure,
                } if failed_request_id == request_id => break failure,
                _ => continue,
            }
        }
    })
    .await
    .expect("operation failure should be correlated")
}

fn restore_ready_room_actions(
    active_room_id: &str,
    room_ids: impl IntoIterator<Item = String>,
) -> Vec<AppAction> {
    restore_ready_actions![
        AppAction::RoomListUpdated {
            spaces: vec![],
            rooms: room_ids
                .into_iter()
                .map(|room_id| room_summary(&room_id))
                .collect(),
        },
        AppAction::SelectRoom {
            room_id: active_room_id.to_owned(),
        },
        AppAction::TimelineSubscribed {
            room_id: active_room_id.to_owned(),
        },
    ]
}

async fn wait_for_ready_room(connection: &mut koushi_core::runtime::CoreConnection, room_id: &str) {
    wait_for_state_event(connection, |state| {
        matches!(state.session, SessionState::Ready(_))
            && state.timeline.room_id.as_deref() == Some(room_id)
    })
    .await;
}

async fn seed_composer_payload(
    data_dir: &std::path::Path,
    credential_dir: &std::path::Path,
    room_id: &str,
    body: &str,
) {
    let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.to_path_buf(),
        credential_dir.to_path_buf(),
    );
    let mut connection = runtime.attach();
    runtime
        .inject_actions(restore_ready_room_actions(room_id, [room_id.to_owned()]))
        .await;
    wait_for_ready_room(&mut connection, room_id).await;
    let mut drafts = ComposerDraftStore::default();
    drafts
        .apply_room_draft(room_id.to_owned(), body.to_owned(), 1.into())
        .expect("seed persisted composer draft");
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        runtime.inject_composer_drafts_and_wait_for_testing(drafts),
    )
    .await
    .expect("seed composer payload must settle");
}

async fn wait_for_composer_load_io(
    barrier: &mut koushi_core::runtime::ComposerDraftIoBarrierForTesting,
    stage: &str,
) {
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        barrier.wait_for_load_started(),
    )
    .await
    .unwrap_or_else(|_| panic!("{stage} composer load must start"));
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        barrier.wait_for_load_completed(),
    )
    .await
    .unwrap_or_else(|_| panic!("{stage} composer load must complete"));
}

fn ready_staged_file(id: &str, room_id: &str, position: u64) -> StagedUploadItem {
    let variant_id = format!("{id}-prepared");
    StagedUploadItem {
        staged_id: id.to_owned(),
        room_id: room_id.to_owned(),
        position,
        filename: format!("{id}.txt"),
        mime_type: "text/plain".to_owned(),
        byte_count: 32,
        kind: StagedUploadKind::File,
        caption: None,
        compression_choice: StagedUploadCompressionChoice::NotApplicable,
        preparation: StagedUploadPreparation::Ready {
            variants: vec![PreparedUploadVariant {
                variant_id: variant_id.clone(),
                filename: format!("{id}.txt"),
                mime_type: "text/plain".to_owned(),
                byte_count: 32,
                width: None,
                height: None,
                format: PreparedUploadFormat::Original,
                savings_percent: 0,
                metadata_stripped: false,
                thumbnail_refreshed: false,
            }],
            selected_variant_id: variant_id,
        },
    }
}

#[tokio::test]
async fn composer_revision_exhaustion_blocks_prepared_plain_reply_and_thread_acceptance() {
    let (runtime, mut conn, _snapshot, _data_dir, _credential_dir) =
        ready_room_conn("!room:example.test").await;
    let room_id = "!room:example.test".to_owned();
    let root_event_id = "$root:example.test".to_owned();
    runtime
        .inject_actions(vec![
            AppAction::OpenThread {
                room_id: room_id.clone(),
                root_event_id: root_event_id.clone(),
            },
            AppAction::ThreadSubscribed {
                room_id: room_id.clone(),
                root_event_id: root_event_id.clone(),
            },
        ])
        .await;
    wait_for_state(&mut conn, |state| {
        matches!(state.thread, ThreadPaneState::Open { .. })
    })
    .await;

    submit_composer_command(
        &conn,
        CoreCommand::App(AppCommand::SetComposerDraft {
            request_id: conn.next_request_id(),
            expected_account: draft_account(),
            room_id: room_id.clone(),
            draft: "keep room draft".to_owned(),
            revision: ComposerDraftRevision::MAX,
        }),
    )
    .await
    .expect("seed maximum room revision");
    submit_composer_command(
        &conn,
        CoreCommand::App(AppCommand::SetThreadComposerDraft {
            request_id: conn.next_request_id(),
            expected_account: draft_account(),
            room_id: room_id.clone(),
            root_event_id: root_event_id.clone(),
            draft: "keep thread draft".to_owned(),
            revision: ComposerDraftRevision::MAX,
        }),
    )
    .await
    .expect("seed maximum thread revision");
    wait_for_state(&mut conn, |state| {
        state.composer_drafts.room_revision("!room:example.test") == ComposerDraftRevision::MAX
            && state
                .composer_drafts
                .thread_revision("!room:example.test", "$root:example.test")
                == ComposerDraftRevision::MAX
    })
    .await;

    let main_target = ComposerTarget::Main {
        room_id: room_id.clone(),
    };
    let thread_target = ComposerTarget::Thread {
        room_id: room_id.clone(),
        root_event_id: root_event_id.clone(),
    };
    let main_staged = vec![
        ready_staged_file("main-stage-1", &room_id, 1),
        ready_staged_file("main-stage-2", &room_id, 2),
    ];
    let thread_staged = vec![
        ready_staged_file("thread-stage-1", &room_id, 1),
        ready_staged_file("thread-stage-2", &room_id, 2),
    ];
    conn.command(CoreCommand::App(AppCommand::SetUploadStaging {
        request_id: conn.next_request_id(),
        target: main_target.clone(),
        items: main_staged.clone(),
    }))
    .await
    .expect("stage main prepared uploads");
    conn.command(CoreCommand::App(AppCommand::SetUploadStaging {
        request_id: conn.next_request_id(),
        target: thread_target.clone(),
        items: thread_staged.clone(),
    }))
    .await
    .expect("stage thread prepared uploads");
    wait_for_state(&mut conn, |state| {
        state.upload_staging.items_for_target(&main_target) == main_staged
            && state.upload_staging.items_for_target(&thread_target) == thread_staged
    })
    .await;

    for target in [main_target.clone(), thread_target.clone()] {
        let request_id = conn.next_request_id();
        submit_composer_command(
            &conn,
            CoreCommand::App(AppCommand::AcceptComposerDraft {
                request_id,
                expected_account: draft_account(),
                target,
                submitted_revision: ComposerDraftRevision::MAX,
            }),
        )
        .await
        .expect("submit maximum prepared-upload acceptance");
        let event = tokio::time::timeout(std::time::Duration::from_secs(1), async {
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
        .expect("maximum prepared-upload acceptance should be correlated");
        assert!(matches!(
            event,
            CoreEvent::OperationFailed {
                failure: CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::ComposerRevisionExhausted
                },
                ..
            }
        ));
    }
    let after_prepared_acceptance = conn.snapshot();
    assert_eq!(
        after_prepared_acceptance
            .upload_staging
            .items_for_target(&main_target),
        main_staged
    );
    assert_eq!(
        after_prepared_acceptance
            .upload_staging
            .items_for_target(&thread_target),
        thread_staged
    );

    let account_key = AccountKey("@alice:example.test".to_owned());
    let room_key = TimelineKey::room(account_key.clone(), room_id.clone());
    let thread_key = TimelineKey {
        account_key,
        kind: TimelineKind::Thread {
            room_id: room_id.clone(),
            root_event_id: root_event_id.clone(),
        },
    };
    let plain_request_id = conn.next_request_id();
    let reply_request_id = conn.next_request_id();
    let thread_request_id = conn.next_request_id();
    let submissions = [
        (
            plain_request_id,
            TimelineCommand::SubmitText {
                request_id: plain_request_id,
                expected_account: draft_account(),
                submission_id: SubmissionId::new("maximum-plain"),
                key: room_key.clone(),
                transaction_id: "maximum-plain-transaction".to_owned(),
                body: "plain".to_owned(),
                mentions: MentionIntent::default(),
                draft_revision: ComposerDraftRevision::MAX,
            },
        ),
        (
            reply_request_id,
            TimelineCommand::SubmitReply {
                request_id: reply_request_id,
                expected_account: draft_account(),
                submission_id: SubmissionId::new("maximum-reply"),
                key: room_key,
                transaction_id: "maximum-reply-transaction".to_owned(),
                in_reply_to_event_id: root_event_id.clone(),
                body: "reply".to_owned(),
                mentions: MentionIntent::default(),
                draft_revision: ComposerDraftRevision::MAX,
            },
        ),
        (
            thread_request_id,
            TimelineCommand::SubmitReply {
                request_id: thread_request_id,
                expected_account: draft_account(),
                submission_id: SubmissionId::new("maximum-thread"),
                key: thread_key,
                transaction_id: "maximum-thread-transaction".to_owned(),
                in_reply_to_event_id: root_event_id,
                body: "thread".to_owned(),
                mentions: MentionIntent::default(),
                draft_revision: ComposerDraftRevision::MAX,
            },
        ),
    ];

    for (request_id, command) in submissions {
        submit_composer_command(&conn, CoreCommand::Timeline(command))
            .await
            .expect("submit maximum revision command");
        let event = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                match conn.recv_event().await.expect("runtime event stream") {
                    event @ CoreEvent::Timeline(
                        koushi_core::event::TimelineEvent::SubmissionRejected {
                            request_id: rejected_request_id,
                            ..
                        },
                    ) if rejected_request_id == request_id => break event,
                    _ => continue,
                }
            }
        })
        .await
        .expect("maximum revision rejection should be correlated");
        assert!(matches!(
            event,
            CoreEvent::Timeline(koushi_core::event::TimelineEvent::SubmissionRejected {
                kind: TimelineFailureKind::ComposerRevisionExhausted,
                ..
            })
        ));
    }

    let snapshot = conn.snapshot();
    assert_eq!(
        snapshot
            .composer_drafts
            .rooms
            .get("!room:example.test")
            .map(String::as_str),
        Some("keep room draft")
    );
    assert_eq!(
        snapshot
            .composer_drafts
            .threads
            .get("!room:example.test")
            .and_then(|threads| threads.get("$root:example.test"))
            .map(String::as_str),
        Some("keep thread draft")
    );
    assert!(
        snapshot
            .timeline
            .composer
            .accepted_submission_ids
            .is_empty()
    );
    assert!(snapshot.timeline.composer.pending_submission_id.is_none());
    assert!(snapshot.timeline.composer.pending_transaction_id.is_none());
    assert!(
        snapshot
            .timeline
            .composer
            .last_accepted_clear_revision
            .is_zero()
    );
    let ThreadPaneState::Open {
        composer,
        staged_uploads,
        ..
    } = &snapshot.thread
    else {
        panic!("thread must remain open");
    };
    assert!(composer.accepted_submission_ids.is_empty());
    assert!(composer.pending_submission_id.is_none());
    assert!(composer.pending_transaction_id.is_none());
    assert!(composer.last_accepted_clear_revision.is_zero());
    assert_eq!(staged_uploads, &thread_staged);
    assert_eq!(snapshot.timeline.staged_uploads, main_staged);
    drop(runtime);
}

#[tokio::test]
async fn submitted_text_rejects_a_stale_full_session_owner_before_timeline_routing() {
    let (_runtime, mut conn, _snapshot, _data_dir, _credential_dir) =
        ready_room_conn("!room:example.test").await;
    let request_id = conn.next_request_id();
    submit_composer_command(
        &conn,
        CoreCommand::Timeline(TimelineCommand::SubmitText {
            request_id,
            expected_account: SessionKeyId {
                homeserver: "https://stale.example.test".to_owned(),
                user_id: draft_account().user_id,
                device_id: "STALE".to_owned(),
            },
            submission_id: SubmissionId::new("stale-owner-submission"),
            key: TimelineKey::room(
                AccountKey("@alice:example.test".to_owned()),
                "!room:example.test",
            ),
            transaction_id: "stale-owner-transaction".to_owned(),
            body: "must not reach another account".to_owned(),
            mentions: MentionIntent::default(),
            draft_revision: 1.into(),
        }),
    )
    .await
    .expect("submit stale-owner text command");

    let event = tokio::time::timeout(std::time::Duration::from_secs(1), async {
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
    .expect("stale-owner rejection should be correlated");

    assert!(matches!(
        event,
        CoreEvent::OperationFailed {
            failure: CoreFailure::SessionRequired,
            ..
        }
    ));
}

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
    let (runtime, _data_dir, _credential_dir) = runtime_with_file_credentials();
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
    submit_composer_command(
        &conn,
        CoreCommand::App(AppCommand::SetThreadComposerDraft {
            request_id,
            expected_account: draft_account(),
            room_id: "!room:example.test".to_owned(),
            root_event_id: "$root:example.test".to_owned(),
            draft: "thread draft".to_owned(),
            revision: 1.into(),
        }),
    )
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
    let (runtime, _data_dir, _credential_dir) = runtime_with_file_credentials();
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
    submit_composer_command(
        &conn,
        CoreCommand::App(AppCommand::SetComposerDraft {
            request_id,
            expected_account: draft_account(),
            room_id: "!room:example.test".to_owned(),
            draft: "room draft".to_owned(),
            revision: 1.into(),
        }),
    )
    .await
    .expect("set room composer draft command");

    let snapshot = wait_for_state(&mut conn, |state| {
        state.timeline.composer.draft == "room draft"
    })
    .await;
    assert_eq!(snapshot.timeline.composer.draft, "room draft");
}

#[tokio::test]
async fn composer_draft_command_rejects_a_stale_account_owner() {
    let (runtime, _data_dir, _credential_dir) = runtime_with_file_credentials();
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
        matches!(state.session, SessionState::Ready(_))
            && state.timeline.room_id.as_deref() == Some("!room:example.test")
    })
    .await;

    submit_composer_command(
        &conn,
        CoreCommand::App(AppCommand::SetComposerDraft {
            request_id: conn.next_request_id(),
            expected_account: SessionKeyId {
                homeserver: "https://stale.example.test".to_owned(),
                user_id: "@stale-account:example.test".to_owned(),
                device_id: "STALE".to_owned(),
            },
            room_id: "!room:example.test".to_owned(),
            draft: "must not cross accounts".to_owned(),
            revision: 10.into(),
        }),
    )
    .await
    .expect("submit stale-owner draft command");
    submit_composer_command(
        &conn,
        CoreCommand::App(AppCommand::SetComposerDraft {
            request_id: conn.next_request_id(),
            expected_account: draft_account(),
            room_id: "!room:example.test".to_owned(),
            draft: "current account draft".to_owned(),
            revision: 1.into(),
        }),
    )
    .await
    .expect("submit current-owner draft command");
    submit_composer_command(
        &conn,
        CoreCommand::App(AppCommand::AcceptComposerDraft {
            request_id: conn.next_request_id(),
            expected_account: SessionKeyId {
                homeserver: "https://stale.example.test".to_owned(),
                user_id: "@stale-account:example.test".to_owned(),
                device_id: "STALE".to_owned(),
            },
            target: koushi_state::ComposerTarget::Main {
                room_id: "!room:example.test".to_owned(),
            },
            submitted_revision: 10.into(),
        }),
    )
    .await
    .expect("submit stale-owner main acceptance");
    submit_composer_command(
        &conn,
        CoreCommand::App(AppCommand::SetComposerDraft {
            request_id: conn.next_request_id(),
            expected_account: draft_account(),
            room_id: "!room:example.test".to_owned(),
            draft: "current account draft after stale acceptance".to_owned(),
            revision: 2.into(),
        }),
    )
    .await
    .expect("submit current-owner draft after stale acceptance");
    submit_composer_command(
        &conn,
        CoreCommand::App(AppCommand::SetThreadComposerDraft {
            request_id: conn.next_request_id(),
            expected_account: SessionKeyId {
                homeserver: "https://stale.example.test".to_owned(),
                user_id: "@stale-account:example.test".to_owned(),
                device_id: "STALE".to_owned(),
            },
            room_id: "!room:example.test".to_owned(),
            root_event_id: "$root:example.test".to_owned(),
            draft: "must not cross thread accounts".to_owned(),
            revision: 10.into(),
        }),
    )
    .await
    .expect("submit stale-owner thread draft command");
    submit_composer_command(
        &conn,
        CoreCommand::App(AppCommand::SetThreadComposerDraft {
            request_id: conn.next_request_id(),
            expected_account: draft_account(),
            room_id: "!room:example.test".to_owned(),
            root_event_id: "$root:example.test".to_owned(),
            draft: "current account thread draft".to_owned(),
            revision: 1.into(),
        }),
    )
    .await
    .expect("submit current-owner thread draft command");
    submit_composer_command(
        &conn,
        CoreCommand::App(AppCommand::AcceptComposerDraft {
            request_id: conn.next_request_id(),
            expected_account: SessionKeyId {
                homeserver: "https://stale.example.test".to_owned(),
                user_id: "@stale-account:example.test".to_owned(),
                device_id: "STALE".to_owned(),
            },
            target: koushi_state::ComposerTarget::Thread {
                room_id: "!room:example.test".to_owned(),
                root_event_id: "$root:example.test".to_owned(),
            },
            submitted_revision: 10.into(),
        }),
    )
    .await
    .expect("submit stale-owner thread acceptance");
    submit_composer_command(
        &conn,
        CoreCommand::App(AppCommand::SetThreadComposerDraft {
            request_id: conn.next_request_id(),
            expected_account: draft_account(),
            room_id: "!room:example.test".to_owned(),
            root_event_id: "$root:example.test".to_owned(),
            draft: "current account thread draft after stale acceptance".to_owned(),
            revision: 2.into(),
        }),
    )
    .await
    .expect("submit current-owner thread draft after stale acceptance");

    let snapshot = wait_for_state(&mut conn, |state| {
        state.timeline.composer.draft == "current account draft after stale acceptance"
            && matches!(
                &state.thread,
                ThreadPaneState::Open { composer, .. }
                    if composer.draft == "current account thread draft after stale acceptance"
            )
    })
    .await;
    assert_eq!(
        snapshot.composer_drafts.room_revision("!room:example.test"),
        2.into()
    );
    assert_eq!(
        snapshot
            .composer_drafts
            .thread_revision("!room:example.test", "$root:example.test"),
        2.into()
    );
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

        submit_composer_command(
            &conn,
            CoreCommand::App(AppCommand::SetComposerDraft {
                request_id: conn.next_request_id(),
                expected_account: draft_account(),
                room_id: "!room:example.test".to_owned(),
                draft: "survives restart".to_owned(),
                revision: 1.into(),
            }),
        )
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

static CORRUPT_COMPOSER_LOAD_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn composer_load_diagnostic_count(stage: &str) -> usize {
    koushi_diagnostics::snapshot()
        .records
        .into_iter()
        .filter(|record| {
            record.event.source == "core.composer_draft" && record.event.stage == stage
        })
        .count()
}

struct CorruptComposerLoadFixture {
    _data_dir: tempfile::TempDir,
    _credential_dir: tempfile::TempDir,
    runtime: CoreRuntime,
    connection: koushi_core::runtime::CoreConnection,
    room_id: &'static str,
    payload_path: std::path::PathBuf,
    corrupt_payload: Vec<u8>,
    valid_payload: Vec<u8>,
    failed_before: usize,
}

impl CorruptComposerLoadFixture {
    async fn start() -> Self {
        let data_dir = tempfile::tempdir().expect("data dir");
        let credential_dir = tempfile::tempdir().expect("credential dir");
        let room_id = "!room:example.test";
        seed_composer_payload(
            data_dir.path(),
            credential_dir.path(),
            room_id,
            "persisted before corruption",
        )
        .await;
        let payload_path = single_composer_draft_file(data_dir.path());
        let valid_payload =
            std::fs::read(&payload_path).expect("read valid encrypted composer payload");

        let mut corrupt_payload = valid_payload.clone();
        *corrupt_payload
            .last_mut()
            .expect("encrypted composer payload is nonempty") ^= 0x01;
        std::fs::write(&payload_path, &corrupt_payload)
            .expect("corrupt encrypted composer payload");
        let failed_before = composer_load_diagnostic_count("load_failed");

        let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
            data_dir.path().to_path_buf(),
            credential_dir.path().to_path_buf(),
        );
        let mut connection = runtime.attach();
        let mut failed_load = runtime.install_composer_draft_io_barrier_for_testing();
        runtime
            .inject_actions(restore_ready_room_actions(room_id, [room_id.to_owned()]))
            .await;
        wait_for_composer_load_io(&mut failed_load, "first corrupt").await;
        wait_for_ready_room(&mut connection, room_id).await;
        let failed_state = connection.snapshot();
        assert_eq!(
            failed_state
                .composer_drafts
                .rooms
                .get(room_id)
                .map(String::as_str),
            None
        );
        assert_eq!(
            composer_load_diagnostic_count("load_failed"),
            failed_before + 1
        );
        assert!(koushi_diagnostics::snapshot().records.iter().all(|record| {
            record.event.source != "core.composer_draft"
                || record.event.stage != "load_failed"
                || record.event.fields.is_empty()
        }));

        Self {
            _data_dir: data_dir,
            _credential_dir: credential_dir,
            runtime,
            connection,
            room_id,
            payload_path,
            corrupt_payload,
            valid_payload,
            failed_before,
        }
    }
}

#[tokio::test]
async fn corrupt_load_attempts_once_per_session() {
    let _serial = CORRUPT_COMPOSER_LOAD_TEST_LOCK.lock().await;
    let mut fixture = CorruptComposerLoadFixture::start().await;
    let benign_room = "!benign:example.test";
    let mut unexpected_reload = fixture
        .runtime
        .install_composer_draft_io_barrier_for_testing();
    fixture
        .runtime
        .inject_actions(vec![AppAction::RoomListUpdated {
            spaces: vec![],
            rooms: vec![room_summary(fixture.room_id), room_summary(benign_room)],
        }])
        .await;
    wait_for_state_event(&mut fixture.connection, |state| {
        state.rooms.iter().any(|room| room.room_id == benign_room)
    })
    .await;
    assert!(
        !unexpected_reload.load_started_before_release(),
        "same-session benign state changes must not retry a failed composer load"
    );
    assert_eq!(
        composer_load_diagnostic_count("load_failed"),
        fixture.failed_before + 1
    );
}

#[tokio::test]
async fn revision_commands_fail_while_composer_load_failed() {
    let _serial = CORRUPT_COMPOSER_LOAD_TEST_LOCK.lock().await;
    let mut fixture = CorruptComposerLoadFixture::start().await;
    let before = fixture.connection.snapshot();
    let set_request_id = fixture.connection.next_request_id();
    submit_composer_command(
        &fixture.connection,
        CoreCommand::App(AppCommand::SetComposerDraft {
            request_id: set_request_id,
            expected_account: draft_account(),
            room_id: fixture.room_id.to_owned(),
            draft: "must remain frontend-owned".to_owned(),
            revision: 2.into(),
        }),
    )
    .await
    .expect("submit draft command to fail-closed actor gate");
    assert_eq!(
        wait_for_operation_failure(&mut fixture.connection, set_request_id).await,
        CoreFailure::StoreUnavailable
    );

    let accept_request_id = fixture.connection.next_request_id();
    submit_composer_command(
        &fixture.connection,
        CoreCommand::App(AppCommand::AcceptComposerDraft {
            request_id: accept_request_id,
            expected_account: draft_account(),
            target: ComposerTarget::Main {
                room_id: fixture.room_id.to_owned(),
            },
            submitted_revision: 1.into(),
        }),
    )
    .await
    .expect("submit acceptance command to fail-closed actor gate");
    assert_eq!(
        wait_for_operation_failure(&mut fixture.connection, accept_request_id).await,
        CoreFailure::StoreUnavailable
    );
    let after = fixture.connection.snapshot();
    assert_eq!(after.composer_drafts, before.composer_drafts);
    assert_eq!(after.timeline.composer, before.timeline.composer);
    assert_eq!(
        std::fs::read(&fixture.payload_path).expect("read corrupt payload"),
        fixture.corrupt_payload
    );
}

#[tokio::test]
async fn lock_unlock_retries_repaired_composer_payload() {
    let _serial = CORRUPT_COMPOSER_LOAD_TEST_LOCK.lock().await;
    let mut fixture = CorruptComposerLoadFixture::start().await;
    std::fs::write(&fixture.payload_path, &fixture.valid_payload)
        .expect("install repaired valid encrypted payload");
    fixture
        .runtime
        .inject_actions(vec![AppAction::SessionLocked])
        .await;
    wait_for_state_event(&mut fixture.connection, |state| {
        matches!(state.session, SessionState::Locked(_))
    })
    .await;
    let mut repaired_load = fixture
        .runtime
        .install_composer_draft_io_barrier_for_testing();
    fixture
        .runtime
        .inject_actions(vec![AppAction::AuthoritativeDeviceTrustChanged {
            generation: 1,
            transition_id: 1,
            trust: CurrentDeviceTrustState::Verified,
        }])
        .await;
    wait_for_composer_load_io(&mut repaired_load, "repaired lifecycle retry").await;
    wait_for_state_event(&mut fixture.connection, |state| {
        matches!(state.session, SessionState::Ready(_))
            && state
                .composer_drafts
                .rooms
                .get(fixture.room_id)
                .is_some_and(|body| body == "persisted before corruption")
    })
    .await;
}

#[tokio::test]
async fn persisted_lru_evicts_same_oldest_target_after_restart() {
    let data_dir = tempfile::tempdir().expect("data dir");
    let credential_dir = tempfile::tempdir().expect("credential dir");
    let room_id = "!room:example.test";
    let mut ordered_root_event_ids = vec!["z-oldest".to_owned(), "a-newer".to_owned()];
    ordered_root_event_ids.extend(
        (0..(koushi_state::MAX_LIVE_COMPOSER_THREAD_TOMBSTONES - 2))
            .map(|index| format!("middle-{index:03}")),
    );
    let newest_after_restart = "b-newest-after-restart".to_owned();

    {
        let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
            data_dir.path().to_path_buf(),
            credential_dir.path().to_path_buf(),
        );
        let mut conn = runtime.attach();
        runtime
            .inject_actions(restore_ready_actions![AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![room_summary(room_id)],
            }])
            .await;
        wait_for_state(&mut conn, |state| {
            matches!(state.session, SessionState::Ready(_))
        })
        .await;

        let mut drafts = koushi_state::ComposerDraftStore::default();
        for root_event_id in &ordered_root_event_ids {
            assert!(
                drafts
                    .apply_thread_draft(
                        room_id.to_owned(),
                        root_event_id.clone(),
                        String::new(),
                        1.into(),
                    )
                    .expect("ordered tombstone")
            );
        }
        let snapshot = runtime
            .inject_composer_drafts_and_wait_for_testing(drafts)
            .await;
        assert_eq!(
            snapshot.composer_drafts.quiescent_thread_tombstone_count(),
            koushi_state::MAX_LIVE_COMPOSER_THREAD_TOMBSTONES
        );
        assert_eq!(
            snapshot
                .composer_drafts
                .thread_revision(room_id, "z-oldest"),
            1.into()
        );
        assert_eq!(
            snapshot.composer_drafts.thread_revision(room_id, "a-newer"),
            1.into()
        );
    }

    let restarted = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    let mut conn = restarted.attach();
    restarted
        .inject_actions(restore_ready_actions![AppAction::RoomListUpdated {
            spaces: vec![],
            rooms: vec![room_summary(room_id)],
        }])
        .await;
    let loaded = wait_for_state_event(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_))
            && state.composer_drafts.quiescent_thread_tombstone_count() > 0
    })
    .await;
    assert_eq!(
        loaded.composer_drafts.quiescent_thread_tombstone_count(),
        koushi_state::MAX_LIVE_COMPOSER_THREAD_TOMBSTONES
    );
    assert_eq!(
        loaded.composer_drafts.thread_revision(room_id, "z-oldest"),
        1.into()
    );
    assert_eq!(
        loaded.composer_drafts.thread_revision(room_id, "a-newer"),
        1.into()
    );

    let mut churned = loaded.composer_drafts.clone();
    assert!(
        churned
            .apply_thread_draft(
                room_id.to_owned(),
                newest_after_restart.clone(),
                String::new(),
                1.into(),
            )
            .expect("post-restart tombstone")
    );
    let snapshot = restarted
        .inject_composer_drafts_and_wait_for_testing(churned)
        .await;
    assert!(
        snapshot
            .composer_drafts
            .thread_revision(room_id, &newest_after_restart)
            > koushi_state::ComposerDraftRevision::ZERO
    );
    assert_eq!(
        snapshot
            .composer_drafts
            .thread_revision(room_id, "z-oldest"),
        koushi_state::ComposerDraftRevision::ZERO
    );
    assert_eq!(
        snapshot.composer_drafts.thread_revision(room_id, "a-newer"),
        1.into()
    );
}

#[tokio::test]
async fn same_key_debounce_preserves_nonlexical_lru_victim_across_restart() {
    let data_dir = tempfile::tempdir().expect("data dir");
    let credential_dir = tempfile::tempdir().expect("credential dir");
    let active_room_id = "!active:example.test";
    let tombstone_room_ids = ["z-oldest".to_owned(), "a-newer".to_owned()]
        .into_iter()
        .chain(
            (0..(koushi_state::MAX_LIVE_COMPOSER_ROOM_TOMBSTONES - 2))
                .map(|index| format!("middle-{index:03}")),
        )
        .collect::<Vec<_>>();

    let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    let mut conn = runtime.attach();
    runtime
        .inject_actions(restore_ready_room_actions(
            active_room_id,
            std::iter::once(active_room_id.to_owned()).chain(tombstone_room_ids.iter().cloned()),
        ))
        .await;
    wait_for_ready_room(&mut conn, active_room_id).await;

    let mut ordered = ComposerDraftStore::default();
    for room_id in tombstone_room_ids.iter().cloned() {
        ordered
            .apply_room_draft(room_id, String::new(), 1.into())
            .expect("seed ordered tombstone");
    }
    let seeded = runtime
        .inject_composer_drafts_and_wait_for_testing(ordered)
        .await;
    assert_eq!(
        seeded.composer_drafts.quiescent_room_tombstone_count(),
        koushi_state::MAX_LIVE_COMPOSER_ROOM_TOMBSTONES
    );

    runtime
        .inject_actions(vec![AppAction::ComposerDraftChangedAtRevision {
            room_id: active_room_id.to_owned(),
            draft: "first debounced content".to_owned(),
            revision: 1.into(),
        }])
        .await;
    wait_for_state_event(&mut conn, |state| {
        state.timeline.composer.draft == "first debounced content"
    })
    .await;
    runtime
        .inject_actions(vec![AppAction::ComposerDraftChangedAtRevision {
            room_id: active_room_id.to_owned(),
            draft: "second debounced content".to_owned(),
            revision: 2.into(),
        }])
        .await;
    wait_for_state_event(&mut conn, |state| {
        state.timeline.composer.draft == "second debounced content"
    })
    .await;
    drop(conn);
    runtime.shutdown().await;

    let restarted = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    let mut restart_conn = restarted.attach();
    restarted
        .inject_actions(restore_ready_room_actions(
            active_room_id,
            std::iter::once(active_room_id.to_owned()).chain(tombstone_room_ids.iter().cloned()),
        ))
        .await;
    let loaded = wait_for_state_event(&mut restart_conn, |state| {
        state
            .composer_drafts
            .rooms
            .get(active_room_id)
            .is_some_and(|body| body == "second debounced content")
    })
    .await
    .composer_drafts;
    assert_eq!(
        loaded.quiescent_room_tombstone_count(),
        koushi_state::MAX_LIVE_COMPOSER_ROOM_TOMBSTONES
    );

    let mut churned = loaded;
    churned
        .apply_room_draft("newest-after-restart".to_owned(), String::new(), 1.into())
        .expect("apply one post-restart churn");
    let after_churn = restarted
        .inject_composer_drafts_and_wait_for_testing(churned)
        .await;
    assert_eq!(
        after_churn.composer_drafts.room_revision("z-oldest"),
        ComposerDraftRevision::ZERO,
        "the exact original oldest tombstone must be evicted after restart"
    );
    assert_eq!(
        after_churn.composer_drafts.room_revision("a-newer"),
        1.into(),
        "nonlexical order must not be canonicalized by same-key coalescing"
    );
}

#[tokio::test]
async fn failed_same_key_permit_replacement_keeps_previous_pending_save() {
    let data_dir = tempfile::tempdir().expect("data dir");
    let credential_dir = tempfile::tempdir().expect("credential dir");
    let room_id = "!room:example.test";
    let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    let mut conn = runtime.attach();
    runtime
        .inject_actions(restore_ready_room_actions(room_id, [room_id.to_owned()]))
        .await;
    wait_for_ready_room(&mut conn, room_id).await;

    runtime
        .inject_actions(vec![AppAction::ComposerDraftChangedAtRevision {
            room_id: room_id.to_owned(),
            draft: "first pending save".to_owned(),
            revision: 1.into(),
        }])
        .await;
    wait_for_state_event(&mut conn, |state| {
        state.timeline.composer.draft == "first pending save"
    })
    .await;
    runtime.fail_next_composer_draft_persistence_permit_for_testing();
    runtime
        .inject_actions(vec![AppAction::ComposerDraftChangedAtRevision {
            room_id: room_id.to_owned(),
            draft: "replacement without permits".to_owned(),
            revision: 2.into(),
        }])
        .await;
    wait_for_state_event(&mut conn, |state| {
        state.timeline.composer.draft == "replacement without permits"
    })
    .await;
    drop(conn);
    runtime.shutdown().await;

    let restarted = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    let mut restarted_conn = restarted.attach();
    restarted
        .inject_actions(restore_ready_room_actions(room_id, [room_id.to_owned()]))
        .await;
    wait_for_state_event(&mut restarted_conn, |state| {
        state
            .composer_drafts
            .rooms
            .get(room_id)
            .is_some_and(|body| body == "first pending save")
    })
    .await;
}

#[tokio::test]
async fn permit_drop_notification_reconciles_collector_without_followup_action() {
    let data_dir = tempfile::tempdir().expect("data dir");
    let credential_dir = tempfile::tempdir().expect("credential dir");
    let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    let mut conn = runtime.attach();
    let held_room = "!held-oldest:example.test";
    let active_room = "!active:example.test";
    let room_ids = [held_room.to_owned(), active_room.to_owned()]
        .into_iter()
        .chain((0..128).map(|index| format!("!churn-{index:03}:example.test")));
    runtime
        .inject_actions(restore_ready_room_actions(active_room, room_ids))
        .await;
    wait_for_ready_room(&mut conn, active_room).await;

    let registry = runtime.composer_draft_lease_registry_for_testing();
    let held_target = ComposerTarget::Main {
        room_id: held_room.to_owned(),
    };
    let held_permits = registry
        .persistence_permits(&draft_account(), [held_target])
        .expect("hold the oldest tombstone through the collector pass");

    let mut drafts = ComposerDraftStore::default();
    drafts
        .apply_room_draft(held_room.to_owned(), String::new(), 1.into())
        .expect("seed oldest tombstone");
    for index in 0..128 {
        drafts
            .apply_room_draft(
                format!("!churn-{index:03}:example.test"),
                String::new(),
                1.into(),
            )
            .expect("seed churn tombstone");
    }
    let held_snapshot = runtime
        .inject_composer_drafts_and_wait_for_testing(drafts)
        .await;
    assert_eq!(
        held_snapshot.composer_drafts.room_revision(held_room),
        ComposerDraftRevision::from_u64(1),
        "the non-touching store hold must protect the oldest tombstone"
    );

    drop(held_permits);
    let collected = wait_for_state(&mut conn, |state| {
        state.composer_drafts.room_revision(held_room).is_zero()
    })
    .await;
    assert!(
        collected.composer_drafts.rooms.get(held_room).is_none(),
        "permit-drop notification alone must reconcile and collect the oldest eligible tombstone"
    );
}

#[tokio::test]
async fn account_switch_flushes_old_composer_save_before_new_load() {
    let data_dir = tempfile::tempdir().expect("data dir");
    let credential_dir = tempfile::tempdir().expect("credential dir");
    let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    let mut conn = runtime.attach();
    runtime.inject_actions(restore_ready_actions![]).await;
    wait_for_state_event(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_))
    })
    .await;

    let mut drafts = koushi_state::ComposerDraftStore::default();
    assert!(
        drafts
            .apply_room_draft("old-room".to_owned(), "old body".to_owned(), 1.into())
            .expect("seed old-account draft")
    );
    runtime
        .inject_composer_drafts_and_wait_for_testing(drafts)
        .await;

    let mut barrier = runtime.install_composer_draft_io_barrier_for_testing();
    let new_session = SessionInfo {
        homeserver: "https://new.example.test".to_owned(),
        user_id: "@new:new.example.test".to_owned(),
        device_id: "NEWDEVICE".to_owned(),
    };
    runtime
        .inject_actions(vec![
            AppAction::ComposerDraftChangedAtRevision {
                room_id: "old-room".to_owned(),
                draft: "latest old body".to_owned(),
                revision: 2.into(),
            },
            AppAction::SwitchAccountRequested {
                info: new_session.clone(),
            },
            AppAction::RestoreSessionNotFound,
            AppAction::RestoreSessionRequested,
            AppAction::RestoreSessionSucceeded(new_session.clone()),
            AppAction::CurrentDeviceTrustChanged(CurrentDeviceTrustState::Verified),
        ])
        .await;

    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        barrier.wait_for_save_started(),
    )
    .await
    .expect("old-account save must reach the blocking store port");
    assert!(
        !barrier.load_started_before_release(),
        "new-account load must not overtake the blocked old-account save"
    );
    barrier.release_save();
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        barrier.wait_for_save_completed(),
    )
    .await
    .expect("old-account save must complete after release");
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        barrier.wait_for_load_started(),
    )
    .await
    .expect("new-account load must follow old-account save completion");
    wait_for_state_event(
        &mut conn,
        |state| matches!(&state.session, SessionState::Ready(info) if info == &new_session),
    )
    .await;
}

#[tokio::test]
async fn same_account_unlock_flushes_preserved_composer_save_before_reload() {
    let data_dir = tempfile::tempdir().expect("data dir");
    let credential_dir = tempfile::tempdir().expect("credential dir");
    let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    let mut conn = runtime.attach();
    runtime
        .inject_actions(restore_ready_actions![
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![room_summary("same-account-room")],
            },
            AppAction::SelectRoom {
                room_id: "same-account-room".to_owned(),
            },
            AppAction::TimelineSubscribed {
                room_id: "same-account-room".to_owned(),
            },
        ])
        .await;
    wait_for_state_event(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_))
            && state.timeline.room_id.as_deref() == Some("same-account-room")
    })
    .await;

    let mut drafts = koushi_state::ComposerDraftStore::default();
    assert!(
        drafts
            .apply_room_draft(
                "same-account-room".to_owned(),
                "saved body".to_owned(),
                1.into(),
            )
            .expect("seed same-account draft")
    );
    runtime
        .inject_composer_drafts_and_wait_for_testing(drafts)
        .await;

    let mut barrier = runtime.install_composer_draft_io_barrier_for_testing();
    runtime
        .inject_actions(vec![
            AppAction::ComposerDraftChangedAtRevision {
                room_id: "same-account-room".to_owned(),
                draft: "latest body".to_owned(),
                revision: 2.into(),
            },
            AppAction::SessionLocked,
            AppAction::AuthoritativeDeviceTrustChanged {
                generation: 1,
                transition_id: 1,
                trust: CurrentDeviceTrustState::Verified,
            },
        ])
        .await;

    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        barrier.wait_for_save_started(),
    )
    .await
    .expect("same-account preservation save must reach the blocking store port");
    assert!(
        !barrier.load_started_before_release(),
        "same-account reload must not overtake the blocked preservation save"
    );
    barrier.release_save();
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        barrier.wait_for_save_completed(),
    )
    .await
    .expect("same-account preservation save must complete after release");
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        barrier.wait_for_load_started(),
    )
    .await
    .expect("same-account reload must follow preservation save completion");
    wait_for_state_event(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_))
            && state
                .composer_drafts
                .rooms
                .get("same-account-room")
                .is_some_and(|body| body == "latest body")
    })
    .await;
}

#[tokio::test]
async fn ignored_stale_reset_completion_does_not_cancel_pending_composer_save() {
    let data_dir = tempfile::tempdir().expect("data dir");
    let credential_dir = tempfile::tempdir().expect("credential dir");
    let runtime = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    let mut conn = runtime.attach();
    runtime
        .inject_actions(restore_ready_actions![
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![room_summary("reset-stale-room")],
            },
            AppAction::SelectRoom {
                room_id: "reset-stale-room".to_owned(),
            },
            AppAction::TimelineSubscribed {
                room_id: "reset-stale-room".to_owned(),
            },
        ])
        .await;
    wait_for_state_event(&mut conn, |state| {
        matches!(state.session, SessionState::Ready(_))
            && state.timeline.room_id.as_deref() == Some("reset-stale-room")
    })
    .await;

    let mut drafts = koushi_state::ComposerDraftStore::default();
    assert!(
        drafts
            .apply_room_draft(
                "reset-stale-room".to_owned(),
                "saved body".to_owned(),
                1.into(),
            )
            .expect("seed stale-reset draft")
    );
    runtime
        .inject_composer_drafts_and_wait_for_testing(drafts)
        .await;

    let mut barrier = runtime.install_composer_draft_io_barrier_for_testing();
    runtime
        .inject_actions(vec![
            AppAction::ComposerDraftChangedAtRevision {
                room_id: "reset-stale-room".to_owned(),
                draft: "latest body".to_owned(),
                revision: 2.into(),
            },
            AppAction::ResetLocalDataCompleted { request_id: 999 },
        ])
        .await;

    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        barrier.wait_for_save_started(),
    )
    .await
    .expect("ignored stale reset completion must preserve the pending save");
    barrier.release_save();
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        barrier.wait_for_save_completed(),
    )
    .await
    .expect("preserved pending save must complete after release");
    drop(conn);
    runtime.shutdown().await;

    let restarted = CoreRuntime::start_with_data_dir_and_file_credentials(
        data_dir.path().to_path_buf(),
        credential_dir.path().to_path_buf(),
    );
    let mut restarted_conn = restarted.attach();
    restarted
        .inject_actions(restore_ready_actions![
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms: vec![room_summary("reset-stale-room")],
            },
            AppAction::SelectRoom {
                room_id: "reset-stale-room".to_owned(),
            },
            AppAction::TimelineSubscribed {
                room_id: "reset-stale-room".to_owned(),
            },
        ])
        .await;
    wait_for_state_event(&mut restarted_conn, |state| {
        state.timeline.composer.draft == "latest body"
            && state.timeline.composer.draft_revision == 2.into()
    })
    .await;
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

        submit_composer_command(
            &conn,
            CoreCommand::App(AppCommand::SetComposerDraft {
                request_id: conn.next_request_id(),
                expected_account: draft_account(),
                room_id: "!room:example.test".to_owned(),
                draft: "deleted before restart".to_owned(),
                revision: 1.into(),
            }),
        )
        .await
        .expect("set room composer draft");
        wait_for_state(&mut conn, |state| {
            state.timeline.composer.draft == "deleted before restart"
        })
        .await;
        executor::sleep(COMPOSER_DRAFT_PERSIST_DEBOUNCE * 2).await;

        submit_composer_command(
            &conn,
            CoreCommand::App(AppCommand::SetComposerDraft {
                request_id: conn.next_request_id(),
                expected_account: draft_account(),
                room_id: "!room:example.test".to_owned(),
                draft: String::new(),
                revision: 2.into(),
            }),
        )
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
