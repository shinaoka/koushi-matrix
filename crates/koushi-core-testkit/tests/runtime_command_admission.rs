mod support;

use futures_util::FutureExt;
use koushi_core::composer_draft_lifecycle::ComposerDraftScope;
use koushi_core::{AppCommand, CommandSubmitError, CoreCommand, CoreConnection};
use koushi_state::{ComposerDraftRevision, ComposerMode, ComposerTarget};
use support::{ready_room_conn, session_key};

fn set_reply_target(connection: &CoreConnection, room_id: &str, event_id: &str) -> CoreCommand {
    CoreCommand::App(AppCommand::SetComposerReplyTarget {
        request_id: connection.next_request_id(),
        room_id: room_id.to_owned(),
        event_id: event_id.to_owned(),
    })
}

#[tokio::test]
async fn command_admission_is_not_settled_at_queue_acceptance() {
    let room_id = "!queue-admission:example.invalid";
    let (_runtime, connection, _, _data_dir, _credential_dir) = ready_room_conn(room_id).await;
    let mut admission = Box::pin(connection.command_with_admission(set_reply_target(
        &connection,
        room_id,
        "$queue-admission:example.invalid",
    )));

    assert!(
        admission.as_mut().now_or_never().is_none(),
        "queue acceptance must not settle command admission"
    );
    assert!(
        admission
            .await
            .expect("command admission")
            .admitted_generation
            > 0
    );
}

#[tokio::test]
async fn command_admission_settles_after_state_publication() {
    let room_id = "!admission-room:example.invalid";
    let (_runtime, connection, _, _data_dir, _credential_dir) = ready_room_conn(room_id).await;
    let before = connection.versioned_snapshot().generation;

    let admission = connection
        .command_with_admission(set_reply_target(
            &connection,
            room_id,
            "$admission-event:example.invalid",
        ))
        .await
        .expect("command admission");
    let published = connection.versioned_snapshot();

    assert!(admission.admitted_generation > before);
    assert_eq!(admission.admitted_generation, published.generation);
    assert!(matches!(
        published.state.timeline.composer.mode,
        ComposerMode::Reply { .. }
    ));
}

#[tokio::test]
async fn coalesced_command_admissions_each_settle_once_at_batch_generation() {
    let room_id = "!coalesced-room:example.invalid";
    let (_runtime, connection, _, _data_dir, _credential_dir) = ready_room_conn(room_id).await;

    let first = connection.command_with_admission(set_reply_target(
        &connection,
        room_id,
        "$first:example.invalid",
    ));
    let second = connection.command_with_admission(set_reply_target(
        &connection,
        room_id,
        "$second:example.invalid",
    ));
    let (first, second) = tokio::join!(first, second);
    let first = first.expect("first command admission");
    let second = second.expect("second command admission");

    assert_eq!(first.admitted_generation, second.admitted_generation);
    assert_eq!(
        connection.versioned_snapshot().state.timeline.composer.mode,
        ComposerMode::Reply {
            in_reply_to_event_id: "$second:example.invalid".to_owned()
        }
    );
}

#[tokio::test]
async fn no_delta_routing_returns_the_current_published_generation() {
    let room_id = "!no-delta-admission:example.invalid";
    let event_id = "$same-reply:example.invalid";
    let (_runtime, connection, _, _data_dir, _credential_dir) = ready_room_conn(room_id).await;
    connection
        .command_with_admission(set_reply_target(&connection, room_id, event_id))
        .await
        .expect("first command admission");
    let before = connection.versioned_snapshot().generation;

    let admission = connection
        .command_with_admission(set_reply_target(&connection, room_id, event_id))
        .await
        .expect("idempotent command admission");

    assert_eq!(admission.admitted_generation, before);
    assert_eq!(connection.versioned_snapshot().generation, before);
}

#[tokio::test]
async fn composer_lease_command_admission_settles_after_publication() {
    let room_id = "!composer-admission:example.invalid";
    let (_runtime, connection, _, _data_dir, _credential_dir) = ready_room_conn(room_id).await;
    let account = session_key();
    let generation = connection
        .begin_composer_draft_renderer_generation()
        .expect("renderer generation");
    let lease = connection
        .acquire_composer_draft_lease(
            generation,
            ComposerDraftScope {
                account: account.clone(),
                target: ComposerTarget::Main {
                    room_id: room_id.to_owned(),
                },
            },
        )
        .expect("composer lease");

    let admission = connection
        .command_with_composer_lease_and_admission(
            generation,
            lease,
            CoreCommand::App(AppCommand::SetComposerDraft {
                request_id: connection.next_request_id(),
                expected_account: account,
                room_id: room_id.to_owned(),
                document: "admitted body".into(),
                revision: ComposerDraftRevision::from_u64(0),
            }),
        )
        .await
        .expect("composer command admission");

    assert_eq!(
        admission.admitted_generation,
        connection.versioned_snapshot().generation
    );
}

#[tokio::test]
async fn closed_command_sender_returns_typed_failure() {
    let (connection, control) = CoreConnection::new_for_testing(1);
    drop(control);

    assert_eq!(
        connection
            .command_with_admission(CoreCommand::App(AppCommand::CloseThread {
                request_id: connection.next_request_id(),
            }))
            .await,
        Err(CommandSubmitError::RuntimeClosed)
    );
}
