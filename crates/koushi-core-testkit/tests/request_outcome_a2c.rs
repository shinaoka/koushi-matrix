use futures_util::FutureExt;
use koushi_core::runtime::request_outcome::{
    OutcomeCorrelation, RequestOutcome, RequestOutcomeError, RequestOutcomeExpectation,
};
use koushi_core::{AccountKey, CoreConnection, RequestId, RuntimeConnectionId, TimelineKey};
use koushi_protocol::event::{CoreEvent, TimelineEvent};
use koushi_state::{AppState, ComposerDocument, ComposerTarget, SessionInfo, SessionState};
use std::time::Duration;

fn request(sequence: u64) -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(41),
        sequence,
    }
}

fn ready_state(user_id: &str) -> AppState {
    let mut state = AppState::default();
    state.session = SessionState::Ready(SessionInfo {
        homeserver: "https://example.invalid".to_owned(),
        user_id: user_id.to_owned(),
        device_id: "DEVICE".to_owned(),
        authentication_method: Default::default(),
    });
    state
}

fn versioned(
    state: AppState,
    generation: u64,
) -> koushi_protocol::state_update::VersionedAppStateSnapshot {
    koushi_protocol::state_update::VersionedAppStateSnapshot { generation, state }
}

fn staged(staged_id: &str, room_id: &str) -> koushi_state::StagedUploadItem {
    koushi_state::StagedUploadItem {
        staged_id: staged_id.to_owned(),
        room_id: room_id.to_owned(),
        position: 0,
        filename: "fixture.bin".to_owned(),
        mime_type: "application/octet-stream".to_owned(),
        byte_count: 1,
        kind: koushi_state::StagedUploadKind::File,
        caption: None,
        compression_choice: koushi_state::StagedUploadCompressionChoice::NotApplicable,
        preparation: Default::default(),
    }
}

fn main_target(room_id: &str) -> ComposerTarget {
    ComposerTarget::Main {
        room_id: room_id.to_owned(),
    }
}

fn state_with_revision(user_id: &str, room_id: &str, revision: u64) -> AppState {
    let mut state = ready_state(user_id);
    state.timeline.room_id = Some(room_id.to_owned());
    state
        .composer_drafts
        .apply_room_draft(
            room_id.to_owned(),
            ComposerDocument::from_plain_text("fixture draft"),
            revision.into(),
        )
        .expect("synthetic revision");
    state
}

#[tokio::test]
async fn upload_staging_requires_exact_ids_target_account_and_newer_snapshot() {
    let (mut connection, control) = CoreConnection::new_for_testing(8);
    let request_id = request(1);
    let target = main_target("!room-a:example.invalid");
    let account_key = AccountKey("@alice:example.invalid".to_owned());
    let waiter = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::UploadStaging {
            request_id,
            account_key: account_key.clone(),
            target: target.clone(),
            staged_ids: vec!["one".to_owned(), "two".to_owned()],
            allow_initial: false,
        },
        1,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    let mut waiter = Box::pin(waiter);

    let mut wrong_account = ready_state("@bob:example.invalid");
    wrong_account.timeline.room_id = Some("!room-a:example.invalid".to_owned());
    wrong_account.timeline.staged_uploads = vec![
        staged("one", "!room-a:example.invalid"),
        staged("two", "!room-a:example.invalid"),
    ];
    control.send_snapshot(versioned(wrong_account, 2));
    assert!(waiter.as_mut().now_or_never().is_none());

    let mut wrong_target = ready_state("@alice:example.invalid");
    wrong_target.timeline.room_id = Some("!room-b:example.invalid".to_owned());
    wrong_target.timeline.staged_uploads = vec![
        staged("one", "!room-b:example.invalid"),
        staged("two", "!room-b:example.invalid"),
    ];
    control.send_snapshot(versioned(wrong_target, 3));
    assert!(waiter.as_mut().now_or_never().is_none());

    let mut wrong_ids = ready_state("@alice:example.invalid");
    wrong_ids.timeline.room_id = Some("!room-a:example.invalid".to_owned());
    wrong_ids.timeline.staged_uploads = vec![staged("one", "!room-a:example.invalid")];
    control.send_snapshot(versioned(wrong_ids, 4));
    assert!(waiter.as_mut().now_or_never().is_none());

    let mut settled = ready_state("@alice:example.invalid");
    settled.timeline.room_id = Some("!room-a:example.invalid".to_owned());
    settled.timeline.staged_uploads = vec![
        staged("one", "!room-a:example.invalid"),
        staged("two", "!room-a:example.invalid"),
    ];
    let published = versioned(settled, 5);
    control.send_snapshot(published.clone());
    assert_eq!(
        waiter.await,
        Ok(RequestOutcome::UploadStaging {
            request_id,
            generation: published.generation,
        })
    );
    assert_eq!(connection.versioned_snapshot(), published);
}

#[tokio::test]
async fn composer_acceptance_requires_exact_account_target_revision_and_returns_snapshot() {
    let (mut connection, control) = CoreConnection::new_for_testing(8);
    let request_id = request(2);
    let target = main_target("!room-a:example.invalid");
    let account_key = AccountKey("@alice:example.invalid".to_owned());
    let waiter = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::ComposerAccepted {
            request_id,
            account_key,
            target,
            expected_revision: 3.into(),
        },
        1,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    let mut waiter = Box::pin(waiter);

    control.send_snapshot(versioned(
        state_with_revision("@bob:example.invalid", "!room-a:example.invalid", 3),
        2,
    ));
    assert!(waiter.as_mut().now_or_never().is_none());
    control.send_snapshot(versioned(
        state_with_revision("@alice:example.invalid", "!room-b:example.invalid", 3),
        3,
    ));
    assert!(waiter.as_mut().now_or_never().is_none());
    control.send_snapshot(versioned(
        state_with_revision("@alice:example.invalid", "!room-a:example.invalid", 2),
        4,
    ));
    assert!(waiter.as_mut().now_or_never().is_none());

    let published = versioned(
        state_with_revision("@alice:example.invalid", "!room-a:example.invalid", 3),
        5,
    );
    control.send_snapshot(published.clone());
    assert_eq!(
        waiter.await,
        Ok(RequestOutcome::ComposerAccepted {
            request_id,
            revision: 3.into(),
            generation: published.generation,
        })
    );
    assert_eq!(connection.versioned_snapshot(), published);
}

#[tokio::test]
async fn composer_terminal_lag_still_checks_the_final_authoritative_snapshot() {
    let (mut connection, control) = CoreConnection::new_for_testing(1);
    let request_id = request(3);
    let target = main_target("!room-a:example.invalid");
    let waiter = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::ComposerAccepted {
            request_id,
            account_key: AccountKey("@alice:example.invalid".to_owned()),
            target,
            expected_revision: 3.into(),
        },
        1,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    control.send_event(CoreEvent::OperationFailed {
        request_id: request(30),
        failure: koushi_core::CoreFailure::SessionRequired,
    });
    control.send_event(CoreEvent::OperationFailed {
        request_id: request(31),
        failure: koushi_core::CoreFailure::SessionRequired,
    });
    let published = versioned(
        state_with_revision("@alice:example.invalid", "!room-a:example.invalid", 3),
        2,
    );
    control.send_snapshot(published.clone());
    assert!(matches!(
        waiter.await,
        Ok(RequestOutcome::ComposerAccepted { generation, .. }) if generation == published.generation
    ));
    assert_eq!(connection.versioned_snapshot(), published);
}

#[tokio::test]
async fn submission_acceptance_and_rejection_require_exact_key_account_target() {
    let (mut connection, control) = CoreConnection::new_for_testing(8);
    let request_id = request(4);
    let submission_id = koushi_state::SubmissionId::new("submission-a");
    let target = main_target("!room-a:example.invalid");
    let expected_key = TimelineKey::room(
        AccountKey("@alice:example.invalid".to_owned()),
        "!room-a:example.invalid",
    );
    let mut state = ready_state("@alice:example.invalid");
    state
        .timeline
        .submission_registry
        .active_submissions
        .push_back(koushi_state::ComposerSubmissionRecord {
            submission_id: submission_id.clone(),
            transaction_id: "txn-a".to_owned(),
            target: target.clone(),
        });
    control.send_snapshot(versioned(state, 2));
    let waiter = connection.wait_for_request_outcome(
        OutcomeCorrelation::Submission {
            request_id,
            submission_id: submission_id.clone(),
        },
        RequestOutcomeExpectation::Submission {
            request_id,
            account_key: AccountKey("@alice:example.invalid".to_owned()),
            target: target.clone(),
            submission_id: submission_id.clone(),
        },
        1,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    let mut waiter = Box::pin(waiter);

    control.send_event(CoreEvent::Timeline(TimelineEvent::SubmissionAccepted {
        request_id,
        key: TimelineKey::room(
            AccountKey("@alice:example.invalid".to_owned()),
            "!room-b:example.invalid",
        ),
        submission_id: submission_id.clone(),
        transaction_id: "txn-a".to_owned(),
    }));
    assert!(waiter.as_mut().now_or_never().is_none());
    control.send_event(CoreEvent::Timeline(TimelineEvent::SubmissionAccepted {
        request_id,
        key: expected_key.clone(),
        submission_id: submission_id.clone(),
        transaction_id: "txn-a".to_owned(),
    }));
    assert!(matches!(
        waiter.await,
        Ok(RequestOutcome::SubmissionAccepted { generation, .. }) if generation == 2
    ));

    let (mut connection, control) = CoreConnection::new_for_testing(8);
    let rejection_id = koushi_state::SubmissionId::new("submission-reject");
    let rejection_request = request(5);
    let rejection = connection.wait_for_request_outcome(
        OutcomeCorrelation::Submission {
            request_id: rejection_request,
            submission_id: rejection_id.clone(),
        },
        RequestOutcomeExpectation::Submission {
            request_id: rejection_request,
            account_key: AccountKey("@alice:example.invalid".to_owned()),
            target,
            submission_id: rejection_id.clone(),
        },
        0,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    tokio::pin!(rejection);
    control.send_event(CoreEvent::Timeline(TimelineEvent::SubmissionRejected {
        request_id: rejection_request,
        key: TimelineKey::room(
            AccountKey("@alice:example.invalid".to_owned()),
            "!room-b:example.invalid",
        ),
        submission_id: rejection_id.clone(),
        kind: koushi_core::TimelineFailureKind::NotSubscribed,
    }));
    assert!(rejection.as_mut().now_or_never().is_none());
    control.send_event(CoreEvent::Timeline(TimelineEvent::SubmissionRejected {
        request_id: rejection_request,
        key: expected_key,
        submission_id: rejection_id.clone(),
        kind: koushi_core::TimelineFailureKind::NotSubscribed,
    }));
    assert!(matches!(
        rejection.await,
        Ok(RequestOutcome::SubmissionRejected { submission_id, .. }) if submission_id == rejection_id
    ));
}

#[tokio::test]
async fn prepared_media_queue_requires_exact_request_transaction_key_and_returns_payload() {
    let (mut connection, control) = CoreConnection::new_for_testing(8);
    let request_id = request(6);
    let expected_key = TimelineKey::room(
        AccountKey("@alice:example.invalid".to_owned()),
        "!room-a:example.invalid",
    );
    let waiter = connection.wait_for_request_outcome(
        OutcomeCorrelation::Request(request_id),
        RequestOutcomeExpectation::PreparedMediaQueued {
            request_id,
            key: expected_key.clone(),
            transaction_id: "txn-a".to_owned(),
        },
        0,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    let mut waiter = Box::pin(waiter);
    let published = versioned(ready_state("@alice:example.invalid"), 4);
    control.send_snapshot(published.clone());
    control.send_event(CoreEvent::Timeline(TimelineEvent::MediaSendQueued {
        request_id,
        key: TimelineKey::room(
            AccountKey("@alice:example.invalid".to_owned()),
            "!room-b:example.invalid",
        ),
        transaction_id: "txn-a".to_owned(),
    }));
    assert!(waiter.as_mut().now_or_never().is_none());
    control.send_event(CoreEvent::Timeline(TimelineEvent::MediaSendQueued {
        request_id,
        key: expected_key.clone(),
        transaction_id: "txn-a".to_owned(),
    }));
    assert!(matches!(
        waiter.await,
        Ok(RequestOutcome::PreparedMediaQueued { key, generation, .. })
            if key == expected_key && generation == published.generation
    ));
    assert_eq!(connection.versioned_snapshot(), published);
}

#[tokio::test]
async fn submission_acceptance_survives_already_settled_snapshot_coalescing() {
    let (mut connection, control) = CoreConnection::new_for_testing(8);
    let request_id = request(20);
    let submission_id = koushi_state::SubmissionId::new("submission-fast");
    let target = main_target("!room-a:example.invalid");
    let key = TimelineKey::room(
        AccountKey("@alice:example.invalid".to_owned()),
        "!room-a:example.invalid",
    );
    let settled_snapshot = versioned(ready_state("@alice:example.invalid"), 2);
    control.send_snapshot(settled_snapshot.clone());
    let waiter = connection.wait_for_request_outcome(
        OutcomeCorrelation::Submission {
            request_id,
            submission_id: submission_id.clone(),
        },
        RequestOutcomeExpectation::Submission {
            request_id,
            account_key: AccountKey("@alice:example.invalid".to_owned()),
            target,
            submission_id: submission_id.clone(),
        },
        1,
        tokio::time::Instant::now() + Duration::from_secs(1),
    );
    control.send_event(CoreEvent::Timeline(TimelineEvent::SubmissionAccepted {
        request_id,
        key,
        submission_id,
        transaction_id: "txn-fast".to_owned(),
    }));
    assert!(matches!(
        waiter.await,
        Ok(RequestOutcome::SubmissionAccepted { generation, .. }) if generation == settled_snapshot.generation
    ));
    assert_eq!(connection.versioned_snapshot(), settled_snapshot);
}

#[tokio::test]
async fn upload_staging_allows_an_already_satisfied_idempotent_projection() {
    let (mut connection, control) = CoreConnection::new_for_testing(8);
    let request_id = request(21);
    let target = main_target("!room-a:example.invalid");
    let mut state = ready_state("@alice:example.invalid");
    state.timeline.room_id = Some("!room-a:example.invalid".to_owned());
    state.timeline.staged_uploads = vec![staged("one", "!room-a:example.invalid")];
    control.send_snapshot(versioned(state, 1));
    let outcome = connection
        .wait_for_request_outcome(
            OutcomeCorrelation::Request(request_id),
            RequestOutcomeExpectation::UploadStaging {
                request_id,
                account_key: AccountKey("@alice:example.invalid".to_owned()),
                target,
                staged_ids: vec!["one".to_owned()],
                allow_initial: true,
            },
            1,
            tokio::time::Instant::now() + Duration::from_secs(1),
        )
        .await;
    assert!(matches!(outcome, Ok(RequestOutcome::UploadStaging { .. })));
}

#[tokio::test]
async fn submission_correlation_rejects_mismatched_request_or_submission() {
    let (mut connection, _control) = CoreConnection::new_for_testing(4);
    let request_id = request(7);
    let submission_id = koushi_state::SubmissionId::new("submission-a");
    let result = connection
        .wait_for_request_outcome(
            OutcomeCorrelation::Submission {
                request_id,
                submission_id: submission_id.clone(),
            },
            RequestOutcomeExpectation::Submission {
                request_id: request(8),
                account_key: AccountKey("@alice:example.invalid".to_owned()),
                target: main_target("!room-a:example.invalid"),
                submission_id,
            },
            0,
            tokio::time::Instant::now() + Duration::from_secs(1),
        )
        .await;
    assert_eq!(result, Err(RequestOutcomeError::InvalidOutcome));
}
