use std::{collections::BTreeSet, sync::Arc};

use koushi_core::command::{AppCommand, CoreCommand, TimelineCommand};
use koushi_core::composer_draft_lifecycle::{
    ComposerDraftLeaseFailure, ComposerDraftLeaseRegistry, ComposerDraftScope,
};
use koushi_core::runtime::CommandSubmitError;
use koushi_core::{AccountKey, RequestId, RuntimeConnectionId, TimelineKey, TimelineKind};
use koushi_key::SessionKeyId;
use koushi_state::{
    AppAction, ComposerDraftProtection, ComposerDraftRevision, ComposerDraftStore, ComposerTarget,
    MentionIntent, SubmissionId,
};
use tokio::sync::oneshot;

mod support;
use support::{ready_room_conn, room_summary, session_key, wait_for_state_event};

fn account(name: &str) -> SessionKeyId {
    SessionKeyId {
        homeserver: format!("https://{name}.invalid"),
        user_id: format!("@{name}:invalid"),
        device_id: format!("{name}-device"),
    }
}

fn main_scope(account: SessionKeyId, room_id: &str) -> ComposerDraftScope {
    ComposerDraftScope {
        account,
        target: ComposerTarget::Main {
            room_id: room_id.to_owned(),
        },
    }
}

#[test]
fn revision_bearing_commands_declare_exact_account_main_and_thread_scopes() {
    let account = account("scope");
    let request_id = RequestId {
        connection_id: RuntimeConnectionId(7),
        sequence: 11,
    };
    let room_key = TimelineKey::room(AccountKey(account.user_id.clone()), "scope-room");
    let thread_key = TimelineKey {
        account_key: AccountKey(account.user_id.clone()),
        kind: TimelineKind::Thread {
            room_id: "scope-room".to_owned(),
            root_event_id: "scope-root".to_owned(),
        },
    };
    let main = ComposerTarget::Main {
        room_id: "scope-room".to_owned(),
    };
    let thread = ComposerTarget::Thread {
        room_id: "scope-room".to_owned(),
        root_event_id: "scope-root".to_owned(),
    };
    let cases = vec![
        (
            CoreCommand::App(AppCommand::SetComposerDraft {
                request_id,
                expected_account: account.clone(),
                room_id: "scope-room".to_owned(),
                draft: String::new(),
                revision: 1.into(),
            }),
            main.clone(),
        ),
        (
            CoreCommand::App(AppCommand::SetThreadComposerDraft {
                request_id,
                expected_account: account.clone(),
                room_id: "scope-room".to_owned(),
                root_event_id: "scope-root".to_owned(),
                draft: String::new(),
                revision: 1.into(),
            }),
            thread.clone(),
        ),
        (
            CoreCommand::App(AppCommand::AcceptComposerDraft {
                request_id,
                expected_account: account.clone(),
                target: main.clone(),
                submitted_revision: 1.into(),
            }),
            main.clone(),
        ),
        (
            CoreCommand::App(AppCommand::AcceptComposerDraft {
                request_id,
                expected_account: account.clone(),
                target: thread.clone(),
                submitted_revision: 1.into(),
            }),
            thread.clone(),
        ),
        (
            CoreCommand::App(AppCommand::ScheduleSend {
                request_id,
                expected_account: account.clone(),
                room_id: "scope-room".to_owned(),
                thread_root_event_id: None,
                body: String::new(),
                send_at_ms: 1,
                draft_revision: 1.into(),
            }),
            main.clone(),
        ),
        (
            CoreCommand::App(AppCommand::ScheduleSend {
                request_id,
                expected_account: account.clone(),
                room_id: "scope-room".to_owned(),
                thread_root_event_id: Some("scope-root".to_owned()),
                body: String::new(),
                send_at_ms: 1,
                draft_revision: 1.into(),
            }),
            thread.clone(),
        ),
        (
            CoreCommand::Timeline(TimelineCommand::SubmitText {
                request_id,
                expected_account: account.clone(),
                submission_id: SubmissionId::new("scope-plain"),
                key: room_key.clone(),
                transaction_id: "scope-plain-transaction".to_owned(),
                body: String::new(),
                draft_revision: 1.into(),
                mentions: MentionIntent::default(),
            }),
            main.clone(),
        ),
        (
            CoreCommand::Timeline(TimelineCommand::SubmitReply {
                request_id,
                expected_account: account.clone(),
                submission_id: SubmissionId::new("scope-reply"),
                key: room_key,
                transaction_id: "scope-reply-transaction".to_owned(),
                in_reply_to_event_id: "scope-reply-root".to_owned(),
                body: String::new(),
                draft_revision: 1.into(),
                mentions: MentionIntent::default(),
            }),
            main.clone(),
        ),
        (
            CoreCommand::Timeline(TimelineCommand::SubmitReply {
                request_id,
                expected_account: account.clone(),
                submission_id: SubmissionId::new("scope-thread"),
                key: thread_key,
                transaction_id: "scope-thread-transaction".to_owned(),
                in_reply_to_event_id: "scope-root".to_owned(),
                body: String::new(),
                draft_revision: 1.into(),
                mentions: MentionIntent::default(),
            }),
            thread,
        ),
    ];

    for (command, target) in cases {
        assert_eq!(
            command.composer_draft_scope(),
            Some(ComposerDraftScope {
                account: account.clone(),
                target,
            })
        );
    }
}

#[tokio::test]
async fn retired_renderer_generation_cannot_submit_or_recreate_target() {
    let registry = Arc::new(ComposerDraftLeaseRegistry::new());
    let mut changes = registry.subscribe();
    let generation = registry.begin_renderer_generation().expect("generation");
    let scope = main_scope(account("retired"), "room-retired");
    let lease = registry
        .acquire(generation, scope.clone())
        .expect("active lease");
    let admitted = registry
        .try_command_permit(generation, lease, &scope)
        .expect("admitted command");
    let (permit_held, permit_is_held) = oneshot::channel();
    let (release_permit, permit_released) = oneshot::channel();
    let admitted_task = tokio::spawn(async move {
        let _admitted = admitted;
        permit_held.send(()).expect("report admitted permit");
        permit_released.await.expect("release admitted permit");
    });
    permit_is_held.await.expect("admitted permit held");

    registry.revoke_generation(generation);

    assert_eq!(
        registry.protected_targets(&scope.account),
        BTreeSet::from([scope.target.clone()]),
        "generation retirement removes activation but preserves the admitted command guard"
    );
    assert!(matches!(
        registry.try_command_permit(generation, lease, &scope),
        Err(ComposerDraftLeaseFailure::RendererGenerationRetired)
    ));
    release_permit.send(()).expect("release admitted permit");
    admitted_task.await.expect("admitted permit task");
    changes
        .changed()
        .await
        .expect("permit release notification");

    let mut drafts = ComposerDraftStore::default();
    drafts
        .apply_room_draft(
            "room-retired".to_owned(),
            String::new(),
            ComposerDraftRevision::from_u64(7),
        )
        .expect("seed retired target");
    drafts.reconcile_lifecycle(&ComposerDraftProtection {
        active: BTreeSet::new(),
        leased: registry.protected_targets(&scope.account),
        ..ComposerDraftProtection::default()
    });
    for index in 0..=128 {
        let room_id = format!("retired-churn-{index:03}");
        drafts
            .apply_room_draft(room_id, String::new(), ComposerDraftRevision::from_u64(1))
            .expect("churn retired target quota");
    }
    drafts.reconcile_lifecycle(&ComposerDraftProtection::default());

    assert_eq!(
        drafts.room_revision("room-retired"),
        ComposerDraftRevision::ZERO,
        "a retired producer must not recreate its collected target"
    );
}

#[tokio::test]
async fn account_runtime_teardown_revokes_live_renderer_generation() {
    let runtime = koushi_core::CoreRuntime::start();
    let registry = runtime.composer_draft_lease_registry_for_testing();
    let scope = main_scope(session_key(), "teardown-room");
    let generation = registry
        .begin_renderer_generation()
        .expect("begin renderer generation");
    let lease_id = registry
        .acquire(generation, scope.clone())
        .expect("acquire composer lease");

    runtime.shutdown().await;

    assert!(matches!(
        registry.try_command_permit(generation, lease_id, &scope),
        Err(ComposerDraftLeaseFailure::RendererGenerationRetired)
    ));
}

#[tokio::test]
async fn account_main_and_thread_leases_are_isolated() {
    let registry = Arc::new(ComposerDraftLeaseRegistry::new());
    let generation = registry.begin_renderer_generation().expect("generation");
    let account_a = account("a");
    let account_b = account("b");
    let scopes = [
        main_scope(account_a.clone(), "same-room"),
        ComposerDraftScope {
            account: account_a.clone(),
            target: ComposerTarget::Thread {
                room_id: "same-room".to_owned(),
                root_event_id: "same-root".to_owned(),
            },
        },
        main_scope(account_b.clone(), "same-room"),
        ComposerDraftScope {
            account: account_b.clone(),
            target: ComposerTarget::Thread {
                room_id: "same-room".to_owned(),
                root_event_id: "same-root".to_owned(),
            },
        },
    ];
    let leases = scopes
        .iter()
        .cloned()
        .map(|scope| registry.acquire(generation, scope).expect("lease"))
        .collect::<Vec<_>>();

    registry.release(generation, leases[0]).expect("release");

    let protected_a = registry.protected_targets(&account_a);
    let protected_b = registry.protected_targets(&account_b);
    assert_eq!(
        protected_a,
        BTreeSet::from([scopes[1].target.clone()]),
        "the released main lease must not disturb the account's thread lease"
    );
    assert_eq!(
        protected_b,
        BTreeSet::from([scopes[2].target.clone(), scopes[3].target.clone()]),
        "the other account must retain its exact main and thread targets"
    );
}

#[tokio::test]
async fn persistence_guard_outlives_activation_release() {
    let registry = Arc::new(ComposerDraftLeaseRegistry::new());
    let mut changes = registry.subscribe();
    let generation = registry.begin_renderer_generation().expect("generation");
    let scope = main_scope(account("persist"), "room-persist");
    let lease = registry
        .acquire(generation, scope.clone())
        .expect("active lease");
    let mut persistence = registry
        .persistence_permits(&scope.account, [scope.target.clone()])
        .expect("persistence permit");
    let (save_started, save_is_in_progress) = oneshot::channel();
    let (finish_save, save_finished) = oneshot::channel();
    let save_task = tokio::spawn(async move {
        let _persistence = std::mem::take(&mut persistence);
        save_started.send(()).expect("report save in progress");
        save_finished.await.expect("finish persistence operation");
    });
    save_is_in_progress
        .await
        .expect("persistence operation started");

    registry.release(generation, lease).expect("release");
    assert!(
        registry
            .protected_targets(&scope.account)
            .contains(&scope.target)
    );
    changes.borrow_and_update();
    finish_save
        .send(())
        .expect("complete persistence operation");
    save_task.await.expect("persistence operation task");
    changes
        .changed()
        .await
        .expect("permit release notification");
    assert!(
        !registry
            .protected_targets(&scope.account)
            .contains(&scope.target)
    );
}

#[tokio::test]
async fn queued_stale_write_keeps_exact_target_protected() {
    let (runtime, mut connection, _, _data_dir, _credential_dir) =
        ready_room_conn("room-stale").await;
    let account = session_key();
    let scope = main_scope(account.clone(), "room-stale");
    let generation = connection
        .begin_composer_draft_renderer_generation()
        .expect("renderer generation");
    let lease_id = connection
        .acquire_composer_draft_lease(generation, scope.clone())
        .expect("active room lease");

    connection
        .command_with_composer_lease(
            generation,
            lease_id,
            CoreCommand::App(AppCommand::SetComposerDraft {
                request_id: connection.next_request_id(),
                expected_account: account.clone(),
                room_id: "room-stale".to_owned(),
                draft: "current body".to_owned(),
                revision: ComposerDraftRevision::from_u64(7),
            }),
        )
        .await
        .expect("seed revision seven");
    wait_for_state_event(&mut connection, |state| {
        state.composer_drafts.room_revision("room-stale") == ComposerDraftRevision::from_u64(7)
    })
    .await;

    let stale_request_id = connection.next_request_id();
    let stale_handle = connection.command_handle();
    let (admitted, admission_observed) = oneshot::channel();
    let (release_stale, stale_released) = oneshot::channel();
    let stale_task = tokio::spawn(async move {
        stale_handle
            .command_with_composer_lease_after_admission(
                generation,
                lease_id,
                CoreCommand::App(AppCommand::SetComposerDraft {
                    request_id: stale_request_id,
                    expected_account: account,
                    room_id: "room-stale".to_owned(),
                    draft: "admitted stale body".to_owned(),
                    revision: ComposerDraftRevision::from_u64(7),
                }),
                admitted,
                stale_released,
            )
            .await
    });
    admission_observed
        .await
        .expect("stale command admitted before actor delivery");

    connection
        .command_with_composer_lease(
            generation,
            lease_id,
            CoreCommand::App(AppCommand::AcceptComposerDraft {
                request_id: connection.next_request_id(),
                expected_account: session_key(),
                target: scope.target.clone(),
                submitted_revision: ComposerDraftRevision::from_u64(7),
            }),
        )
        .await
        .expect("accept current draft");
    connection
        .release_composer_draft_lease(generation, lease_id)
        .expect("release activation after both commands were admitted");
    wait_for_state_event(&mut connection, |state| {
        state.composer_drafts.room_revision("room-stale") == ComposerDraftRevision::from_u64(8)
            && !state.composer_drafts.rooms.contains_key("room-stale")
    })
    .await;

    let mut rooms = vec![room_summary("room-stale"), room_summary("room-current")];
    rooms.extend((0..=128).map(|index| room_summary(&format!("stale-churn-{index:03}"))));
    runtime
        .inject_actions(vec![
            AppAction::RoomListUpdated {
                spaces: vec![],
                rooms,
            },
            AppAction::SelectRoom {
                room_id: "room-current".to_owned(),
            },
            AppAction::TimelineSubscribed {
                room_id: "room-current".to_owned(),
            },
        ])
        .await;
    wait_for_state_event(&mut connection, |state| {
        state.timeline.room_id.as_deref() == Some("room-current")
    })
    .await;

    let mut churn_actions = (0..=128)
        .map(|index| AppAction::ComposerDraftChangedAtRevision {
            room_id: format!("stale-churn-{index:03}"),
            draft: String::new(),
            revision: ComposerDraftRevision::from_u64(1),
        })
        .collect::<Vec<_>>();
    churn_actions.push(AppAction::ComposerDraftChangedAtRevision {
        room_id: "room-current".to_owned(),
        draft: "observation-fence".to_owned(),
        revision: ComposerDraftRevision::from_u64(1),
    });
    runtime.inject_actions(churn_actions).await;
    let churned = wait_for_state_event(&mut connection, |state| {
        state.timeline.composer.draft == "observation-fence"
    })
    .await;
    assert_eq!(
        churned.composer_drafts.room_revision("room-stale"),
        ComposerDraftRevision::from_u64(8),
        "the admitted command permit must protect the exact inactive tombstone"
    );

    release_stale
        .send(())
        .expect("release stale command to actor");
    stale_task
        .await
        .expect("stale command task")
        .expect("stale command delivery");

    connection
        .command(CoreCommand::App(AppCommand::SetComposerReplyTarget {
            request_id: connection.next_request_id(),
            room_id: "room-current".to_owned(),
            event_id: "actor-fence".to_owned(),
        }))
        .await
        .expect("submit actor FIFO fence");
    let snapshot = wait_for_state_event(&mut connection, |state| {
        matches!(
            &state.timeline.composer.mode,
            koushi_state::ComposerMode::Reply {
                in_reply_to_event_id
            } if in_reply_to_event_id == "actor-fence"
        )
    })
    .await;

    assert_eq!(
        snapshot.composer_drafts.room_revision("room-stale"),
        ComposerDraftRevision::from_u64(8)
    );
    assert_eq!(
        snapshot.composer_drafts.rooms.get("room-stale"),
        None,
        "the admitted stale body must not cross the revision fence"
    );
}

#[tokio::test]
async fn revision_bearing_commands_cannot_bypass_lease_admission() {
    let (_runtime, connection, _, _data_dir, _credential_dir) =
        ready_room_conn("room-admission").await;
    let error = connection
        .command(CoreCommand::App(AppCommand::SetComposerDraft {
            request_id: connection.next_request_id(),
            expected_account: session_key(),
            room_id: "room-admission".to_owned(),
            draft: "must not enter the inbox".to_owned(),
            revision: ComposerDraftRevision::from_u64(1),
        }))
        .await
        .expect_err("revision-bearing command requires a lease");

    assert_eq!(error, CommandSubmitError::ComposerLeaseRequired);
    assert_eq!(
        connection
            .snapshot()
            .composer_drafts
            .room_revision("room-admission"),
        ComposerDraftRevision::ZERO
    );
}
