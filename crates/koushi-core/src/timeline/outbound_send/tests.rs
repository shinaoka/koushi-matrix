use super::super::test_source::item_body;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use std::sync::{Arc, Mutex, atomic::Ordering};
use std::task::Poll;
use std::time::Duration;

use futures_util::{FutureExt, StreamExt};

use koushi_state::{AppAction, ComposerDocument, ComposerFormattingOptions};

use crate::send_diagnostics::SendFailureDiagnostic;

use matrix_sdk::{
    send_queue::{RoomSendQueueUpdate, SendQueueUpdate},
    test_utils::mocks::MatrixMockServer,
};

use tokio::sync::{broadcast, mpsc, oneshot};

use crate::account_work::AccountWorkScheduler;

use crate::executor;
use crate::link_preview::LinkPreviewContext;
use koushi_protocol::command::TimelineCommand;
use koushi_protocol::event::{CoreEvent, TimelineEvent};
use koushi_protocol::failure::{CoreFailure, TimelineFailureKind};
#[cfg(any(test, feature = "test-hooks"))]
use koushi_protocol::ids::AccountKey;
use koushi_protocol::ids::{TimelineKey, TimelineKind};

use crate::live_tail_freshness::LiveTailRefreshCoordinator;

use crate::threads_list::ThreadRootProjectionService;

use koushi_diagnostics::DiagnosticValue;
use koushi_state::{SessionInfo, SessionState, SubmissionId};
use std::sync::atomic::AtomicBool;

use crate::runtime::CoreRuntime;
use koushi_protocol::command::CoreCommand;

use super::super::actor::{TimelineActorHandle, TimelineActorMessage};

use super::super::manager::{TimelineManagerActor, TimelineMessage};
use super::super::navigation::{TimelineActorGenerationGate, send_generation_fenced};
use super::super::read_state::ReadWorkerSupervisor;
use super::super::test_support::{
    fake_rid, gap_demand_test_actor_handle, live_tail_test_manager, room_key,
    test_timeline_actor_handle,
};
use super::super::thread_projection::ThreadRootProjectionFetchRegistry;
use super::{
    EncryptedSendDiagnosticSnapshot, MAX_PENDING_SEND_PROJECTIONS, MAX_SETTLED_SEND_TOMBSTONES,
    MAX_SUBMISSION_TOMBSTONES, MediaSendQueuedDelivery, OwnUserTrackingDiagnosticState,
    PendingSendPhase, PendingSendProjection, RoomEncryptionDiagnosticState,
    SEND_ENQUEUE_WORKER_SHUTDOWN_DEADLINE, SendCompletionObservation, SendCompletionRegistration,
    SendCorrelationKey, SendEnqueueSuccess, SendEnqueueWorkerCompletion,
    SendEnqueueWorkerSupervisor, SendLifecycleTrace, SharedSendCompletionCoordinator,
    SubmissionAdmissionLedger, SyntheticSendEnqueueRequest, TimelineSendCompletionDelivery,
    TimelineSendEnqueueContext, TimelineSendEnqueuePayload, TimelineSendFailureDelivery,
    TimelineSendTerminalAdmission, TimelineSendTerminalHandoff, TimelineSendTerminalIngress,
    apply_send_completion_observation_and_handoff,
    apply_send_completion_observation_loss_and_handoff, await_submission_admission,
    classify_timeline_send_error, media_upload_progress_identity, pending_send_item,
    run_global_send_completion_observer,
};

fn test_session_key() -> koushi_protocol::SessionKeyId {
    koushi_protocol::SessionKeyId {
        homeserver: "https://example.test".to_owned(),
        user_id: "@a:test".to_owned(),
        device_id: "DEVICE".to_owned(),
    }
}

#[tokio::test]
async fn generation_fenced_send_discards_a_continuation_replaced_during_capacity_await() {
    let key = room_key();
    let generations = Arc::new(TimelineActorGenerationGate::default());
    let old_generation = generations.activate_after_quiescence(&key).await.generation;
    let (tx, mut rx) = mpsc::channel(1);
    tx.send("occupied").await.expect("fill bounded channel");

    let send_task = tokio::spawn({
        let tx = tx.clone();
        let generations = Arc::clone(&generations);
        let key = key.clone();
        async move { send_generation_fenced(&tx, &generations, &key, old_generation, "stale").await }
    });
    tokio::task::yield_now().await;
    let replacement_generation = generations.activate_after_quiescence(&key).await.generation;
    assert_ne!(replacement_generation, old_generation);

    assert_eq!(rx.recv().await, Some("occupied"));
    assert!(!send_task.await.expect("fenced send task"));
    assert!(
        rx.try_recv().is_err(),
        "stale value must never be published"
    );
}

#[tokio::test]
async fn send_terminal_handoff_survives_origin_abort_and_delivers_exactly_once() {
    let key = room_key();
    let sdk_transaction_id = "sdk-terminal-handoff".to_owned();
    let client_transaction_id = "client-terminal-handoff".to_owned();
    let event_id = "$event-terminal-handoff:test".to_owned();
    let request_id = fake_rid(775);
    let coordinator = SharedSendCompletionCoordinator::default();

    let (action_tx, mut action_rx) = mpsc::channel(1);
    action_tx
        .send(vec![AppAction::ThreadRootProjectionsCleared {
            room_id: "!occupied:test".to_owned(),
        }])
        .await
        .expect("fill reducer channel");
    let (event_tx, mut event_rx) = broadcast::channel(8);
    let manager = TimelineManagerActor::spawn(
        action_tx,
        event_tx,
        None,
        AccountWorkScheduler::default(),
        None,
        None,
    );
    let mut registration = SendCompletionRegistration::begin(
        Arc::clone(&coordinator),
        manager.terminal_sender(),
        key.clone(),
        client_transaction_id.clone(),
        None,
        request_id,
        true,
    );
    registration.activate();
    registration.bind(sdk_transaction_id.clone());

    let (settled_tx, settled_rx) = oneshot::channel();
    let origin = executor::spawn({
        let coordinator = Arc::clone(&coordinator);
        let terminal_tx = manager.terminal_sender();
        let key = key.clone();
        let sdk_transaction_id = sdk_transaction_id.clone();
        let event_id = event_id.clone();
        async move {
            apply_send_completion_observation_and_handoff(
                &coordinator,
                &terminal_tx,
                key.room_id(),
                SendCompletionObservation::Sent {
                    sdk_transaction_id,
                    event_id,
                },
            );
            let _ = settled_tx.send(());
            std::future::pending::<()>().await;
        }
    });
    settled_rx.await.expect("origin settled SDK terminal");
    origin.abort();

    // Model a duplicate global/direct terminal observation. The manager
    // coordinator must suppress it before another handoff is scheduled.
    apply_send_completion_observation_and_handoff(
        &coordinator,
        &manager.terminal_sender(),
        key.room_id(),
        SendCompletionObservation::Sent {
            sdk_transaction_id,
            event_id: event_id.clone(),
        },
    );

    let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
    assert!(
        manager
            .send(TimelineMessage::Shutdown {
                acknowledged: Some(shutdown_tx),
            })
            .await
    );
    assert!(matches!(
        shutdown_rx.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    assert!(matches!(
        event_rx.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));

    let _occupied = action_rx.recv().await.expect("occupied reducer action");
    let delivered = tokio::time::timeout(Duration::from_secs(1), action_rx.recv())
        .await
        .expect("manager terminal action timeout")
        .expect("manager terminal action");
    assert!(matches!(
        delivered.as_slice(),
        [AppAction::SendTextFinished { transaction_id, .. }]
            if transaction_id == &client_transaction_id
    ));
    let completed = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .expect("manager SendCompleted timeout")
        .expect("manager SendCompleted");
    assert!(matches!(
        completed,
        CoreEvent::Timeline(TimelineEvent::SendCompleted {
            request_id: delivered_request_id,
            key: delivered_key,
            transaction_id,
            event_id: delivered_event_id,
        }) if delivered_request_id == request_id
            && delivered_key == key
            && transaction_id == client_transaction_id
            && delivered_event_id == event_id
    ));

    shutdown_rx.await.expect("manager shutdown barrier");
    assert!(
        matches!(
            action_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty) | Err(mpsc::error::TryRecvError::Disconnected)
        ),
        "duplicate terminal must not enqueue a second reducer action"
    );
    assert!(
        matches!(
            event_rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
                | Err(broadcast::error::TryRecvError::Closed)
        ),
        "duplicate terminal must not survive the manager shutdown barrier"
    );
}

#[tokio::test]
async fn media_enqueue_publishes_queued_before_a_prebind_terminal() {
    let key = room_key();
    let request_id = fake_rid(7751);
    let client_transaction_id = "client-media-order".to_owned();
    let sdk_transaction_id = "sdk-media-order".to_owned();
    let event_id = "$event-media-order:test".to_owned();
    let mut manager = live_tail_test_manager(HashMap::new());
    let mut event_rx = manager.event_tx.subscribe();
    let mut registration = SendCompletionRegistration::begin(
        Arc::clone(&manager.send_completion),
        manager.terminal_ingress.clone(),
        key.clone(),
        client_transaction_id.clone(),
        None,
        request_id,
        false,
    );
    registration.activate();

    apply_send_completion_observation_and_handoff(
        &manager.send_completion,
        &manager.terminal_ingress,
        key.room_id(),
        SendCompletionObservation::Sent {
            sdk_transaction_id: sdk_transaction_id.clone(),
            event_id: event_id.clone(),
        },
    );
    assert!(
        manager.terminal_rx.try_recv().is_err(),
        "the SDK terminal must remain held until the enqueue worker binds its transaction"
    );

    manager.spawn_send_enqueue_future(registration, {
        let key = key.clone();
        let client_transaction_id = client_transaction_id.clone();
        async move {
            Ok(SendEnqueueSuccess {
                sdk_transaction_id,
                handle: None,
                media_queued: Some(MediaSendQueuedDelivery {
                    request_id,
                    key,
                    transaction_id: client_transaction_id,
                }),
            })
        }
    });
    let manager_tx = manager.msg_tx.clone();
    let manager_task = executor::spawn(manager.run());

    let first = event_rx.recv().await.expect("first media lifecycle event");
    let second = event_rx.recv().await.expect("second media lifecycle event");
    assert!(matches!(
        first,
        CoreEvent::Timeline(TimelineEvent::MediaSendQueued {
            request_id: queued_request_id,
            key: queued_key,
            transaction_id: queued_transaction_id,
        }) if queued_request_id == request_id
            && queued_key == key
            && queued_transaction_id == client_transaction_id
    ));
    assert!(matches!(
        second,
        CoreEvent::Timeline(TimelineEvent::SendCompleted {
            request_id: completed_request_id,
            key: completed_key,
            transaction_id: completed_transaction_id,
            event_id: completed_event_id,
        }) if completed_request_id == request_id
            && completed_key == key
            && completed_transaction_id == client_transaction_id
            && completed_event_id == event_id
    ));

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    manager_tx
        .send(TimelineMessage::Shutdown {
            acknowledged: Some(shutdown_tx),
        })
        .await
        .expect("manager shutdown command");
    shutdown_rx.await.expect("manager shutdown acknowledgement");
    manager_task.await.expect("manager shutdown task");
}

#[tokio::test]
async fn send_terminal_required_action_failure_suppresses_completion_and_shutdowns() {
    let key = room_key();
    let request_id = fake_rid(776);
    let submission_id = SubmissionId::new("closed-reducer-terminal");
    let transaction_id = "client-closed-reducer".to_owned();
    let mut manager = live_tail_test_manager(HashMap::new());
    manager
        .accepted_submissions
        .accept(submission_id.clone(), key.clone(), transaction_id.clone());
    let mut event_rx = manager.event_tx.subscribe();
    assert!(matches!(
        manager.terminal_ingress.admit(TimelineSendTerminalHandoff {
            key: None,
            hydration: None,
            submission_id: Some(submission_id.clone()),
            action: Some(AppAction::SendTextFinished {
                room_id: key.room_id().to_owned(),
                transaction_id: transaction_id.clone(),
            }),
            completion: Some(TimelineSendCompletionDelivery {
                request_id,
                key,
                transaction_id,
                event_id: "$event-closed-reducer:test".to_owned(),
            }),
            failure: None,
        }),
        TimelineSendTerminalAdmission::Accepted
    ));
    let handoff = manager.terminal_rx.recv().await;
    manager
        .handle_send_terminal_handoff(handoff.expect("accepted terminal handoff"))
        .await;
    assert!(
        matches!(
            event_rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ),
        "SendCompleted must fail closed when its required reducer action cannot be enqueued"
    );
    assert!(
        manager
            .accepted_submissions
            .active
            .contains_key(&submission_id)
    );
    assert!(
        !manager
            .accepted_submissions
            .tombstones
            .iter()
            .any(|(settled, _, _)| settled == &submission_id),
        "the admission ledger must not claim a terminal whose reducer action was rejected"
    );

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let manager_tx = manager.msg_tx.clone();
    let manager_task = executor::spawn(manager.run());
    manager_tx
        .send(TimelineMessage::Shutdown {
            acknowledged: Some(shutdown_tx),
        })
        .await
        .expect("manager shutdown command");
    tokio::time::timeout(Duration::from_secs(1), shutdown_rx)
        .await
        .expect("manager shutdown timeout")
        .expect("manager shutdown acknowledgement");
    manager_task.await.expect("manager shutdown task");
}

#[tokio::test]
async fn observation_loss_failure_survives_required_action_channel_shutdown() {
    let key = room_key();
    let request_id = fake_rid(777);
    let mut manager = live_tail_test_manager(HashMap::new());
    let mut event_rx = manager.event_tx.subscribe();

    manager
        .handle_send_terminal_handoff(TimelineSendTerminalHandoff {
            key: None,
            hydration: None,
            submission_id: None,
            action: Some(AppAction::SendTextFailed {
                room_id: key.room_id().to_owned(),
                transaction_id: "client-observation-loss".to_owned(),
                message: "send failed".to_owned(),
            }),
            completion: None,
            failure: Some(TimelineSendFailureDelivery {
                request_id,
                failure: CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::QueueOverflow,
                },
            }),
        })
        .await;

    assert!(matches!(
        event_rx.try_recv(),
        Ok(CoreEvent::OperationFailed {
            request_id: delivered_request_id,
            failure: CoreFailure::TimelineOperationFailed {
                kind: TimelineFailureKind::QueueOverflow,
            },
        }) if delivered_request_id == request_id
    ));
    assert!(matches!(
        event_rx.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
}

fn synthetic_send_timeline_actor_handle(
    requests: mpsc::UnboundedSender<SyntheticSendEnqueueRequest>,
) -> TimelineActorHandle {
    synthetic_send_timeline_actor_handle_with_projection_probe(requests, None, true)
}

fn synthetic_send_timeline_actor_handle_with_projection_probe(
    requests: mpsc::UnboundedSender<SyntheticSendEnqueueRequest>,
    mut projection_probe: Option<oneshot::Sender<Vec<PendingSendProjection>>>,
    acknowledge: bool,
) -> TimelineActorHandle {
    let mut handle = test_timeline_actor_handle();
    let (tx, mut rx) = mpsc::channel(1);
    handle.tx = tx;
    handle.task = Some(executor::spawn(async move {
        while let Some(message) = rx.recv().await {
            if let TimelineActorMessage::RefreshPendingSendProjection {
                projections,
                acknowledged,
                ..
            } = message
            {
                if let Some(probe) = projection_probe.take() {
                    let _ = probe.send(projections);
                }
                let _ = acknowledged.send(acknowledge);
            }
        }
    }));
    handle.enqueue_context = Some(TimelineSendEnqueueContext::Synthetic { requests });
    handle
}

#[tokio::test]
async fn full_actor_mailbox_defers_pending_refresh_without_blocking_manager() {
    let key = room_key();
    let (tx, mut rx) = mpsc::channel(1);
    tx.try_send(TimelineActorMessage::DisplayPolicyChanged {
        thread_root_order: koushi_state::TimelineThreadRootOrder::RootEvent,
    })
    .expect("fill actor mailbox");
    let mut handle = test_timeline_actor_handle();
    handle.tx = tx;
    let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), handle)]));

    manager.refresh_pending_send_projection(&key).await;
    manager.refresh_pending_send_projection(&key).await;
    assert_eq!(
        manager.send_enqueue_workers.tasks.len(),
        1,
        "repeated refreshes for one actor generation are coalesced"
    );
    assert!(matches!(
        rx.recv().await,
        Some(TimelineActorMessage::DisplayPolicyChanged { .. })
    ));
    let completion = manager
        .send_enqueue_workers
        .tasks
        .next()
        .await
        .expect("deferred refresh settles after mailbox capacity returns");
    manager
        .handle_send_enqueue_worker_completion(completion)
        .await;
    assert!(matches!(
        rx.recv().await,
        Some(TimelineActorMessage::RefreshPendingSendProjection { .. })
    ));
}

async fn poll_manager_enqueue_workers_once(manager: &mut TimelineManagerActor) {
    let completion = std::future::poll_fn(|context| {
        Poll::Ready(
            match manager.send_enqueue_workers.tasks.poll_next_unpin(context) {
                Poll::Ready(completion) => completion,
                Poll::Pending => None,
            },
        )
    })
    .await;
    if let Some(completion) = completion {
        manager
            .handle_send_enqueue_worker_completion(completion)
            .await;
    }
}

#[tokio::test]
async fn duplicate_submission_routes_one_manager_enqueue_worker() {
    let key = room_key();
    let (enqueue_tx, mut enqueue_rx) = mpsc::unbounded_channel();
    let (action_tx, mut action_rx) = mpsc::channel(4);
    let (event_tx, mut event_rx) = broadcast::channel(4);
    let (msg_tx, msg_rx) = mpsc::channel(1);
    let (terminal_ingress, terminal_rx) = TimelineSendTerminalIngress::channel();
    let mut manager = TimelineManagerActor {
        session: None,
        room_list_service: None,
        room_subscription_checkpoint_task: None,
        room_subscription_service_epoch: 0,
        current_core_generation: None,
        room_leave_states: BTreeMap::new(),
        #[cfg(any(test, feature = "test-hooks"))]
        restored_room_subscription_probe: None,
        session_subscribed_rooms: BTreeSet::new(),
        subscribed_room_leases: BTreeMap::new(),
        subscription_room_seen: BTreeSet::new(),
        subscription_room_ordinals: BTreeMap::new(),
        next_subscription_room_ordinal: 0,
        global_response_commit: None,
        timelines: HashMap::from([(
            key.clone(),
            synthetic_send_timeline_actor_handle(enqueue_tx),
        )]),
        accepted_submissions: SubmissionAdmissionLedger::default(),
        send_completion: SharedSendCompletionCoordinator::default(),
        global_send_completion_observer_future: None,
        send_enqueue_workers: SendEnqueueWorkerSupervisor::new(terminal_ingress.clone()),
        read_workers: ReadWorkerSupervisor::unavailable(),
        action_tx,
        event_tx,
        msg_tx,
        msg_rx,
        control_rx: None,
        navigation_projection_rx: None,
        last_navigation_projection_generation: 0,
        terminal_ingress,
        terminal_rx,
        search_index_tx: None,
        ignored_user_ids: Default::default(),
        data_dir: None,
        link_preview_policy: LinkPreviewContext::default(),
        composer_formatting_options: ComposerFormattingOptions::default(),
        account_work: AccountWorkScheduler::default(),
        thread_root_projection_service: Arc::new(
            Mutex::new(ThreadRootProjectionService::default()),
        ),
        thread_root_projection_fetches: ThreadRootProjectionFetchRegistry::default(),
        thread_root_order: koushi_state::TimelineThreadRootOrder::LatestReply,
        timeline_actor_generations: Arc::new(TimelineActorGenerationGate::default()),
        live_tail_refreshes: LiveTailRefreshCoordinator::new(),
        test_session_available: true,
    };
    manager.send_enqueue_workers.tasks.push(Box::pin(async {
        SendEnqueueWorkerCompletion {
            changed_key: None,
            hydration: None,
            deferred_refresh: None,
        }
    }));
    let submission_id = SubmissionId::new("opaque-submission");
    for request_id in [fake_rid(7300), fake_rid(7301)] {
        manager
            .handle_command(TimelineCommand::SubmitText {
                request_id,
                expected_account: test_session_key(),
                submission_id: submission_id.clone(),
                key: key.clone(),
                transaction_id: "txn-once".to_owned(),
                document: ComposerDocument::from_plain_text("body"),
                draft_revision: 1.into(),
            })
            .await;
    }
    let request = tokio::time::timeout(Duration::from_secs(1), enqueue_rx.recv())
        .await
        .expect("manager enqueue worker must be driven")
        .expect("one manager enqueue worker");
    assert!(matches!(
        request.payload,
        TimelineSendEnqueuePayload::Text { ref document, .. } if document.plain_body() == "body"
    ));
    assert!(enqueue_rx.try_recv().is_err());
    assert!(matches!(
        action_rx.try_recv(),
        Ok(actions) if matches!(actions.as_slice(), [AppAction::ComposerSubmissionAcceptedAtRevision { submission_id: accepted, .. }] if accepted == &submission_id)
    ));
    assert!(action_rx.try_recv().is_err());
    assert!(
        request
            .response
            .send(Ok(SendEnqueueSuccess::terminal_only(
                "sdk-transaction".to_owned(),
            )))
            .is_ok(),
        "complete synthetic enqueue"
    );
    manager.join_send_enqueue_workers().await;

    while event_rx.try_recv().is_ok() {}
    let mut cap_registrations = Vec::new();
    for index in 1..MAX_PENDING_SEND_PROJECTIONS {
        let client_txn_id = format!("client-route-cap-{index}");
        cap_registrations.push(SendCompletionRegistration::begin_with_projection(
            Arc::clone(&manager.send_completion),
            manager.terminal_ingress.clone(),
            key.clone(),
            client_txn_id.clone(),
            None,
            fake_rid(7_400 + index as u64),
            true,
            Some(PendingSendProjection {
                key: key.clone(),
                sequence: 0,
                item: pending_send_item(&client_txn_id, "body", None, None, None),
                client_txn_id,
                sdk_transaction_id: None,
                handle: None,
                terminal_event_id: None,
                phase: PendingSendPhase::Pending,
            }),
        ));
    }
    assert_eq!(
        manager
            .send_completion
            .lock()
            .expect("coordinator")
            .pending_projection_count(),
        MAX_PENDING_SEND_PROJECTIONS
    );
    let cap_rejected_id = SubmissionId::new("pending-cap-rejected");
    manager
        .handle_command(TimelineCommand::SubmitText {
            request_id: fake_rid(7_599),
            expected_account: test_session_key(),
            submission_id: cap_rejected_id.clone(),
            key: key.clone(),
            transaction_id: "txn-cap-rejected".to_owned(),
            document: ComposerDocument::from_plain_text("body"),
            draft_revision: 2.into(),
        })
        .await;
    assert!(action_rx.try_recv().is_err());
    assert!(enqueue_rx.try_recv().is_err());
    assert!(matches!(
        event_rx.try_recv(),
        Ok(CoreEvent::Timeline(TimelineEvent::SubmissionRejected {
            submission_id,
            kind: TimelineFailureKind::QueueOverflow,
            ..
        })) if submission_id == cap_rejected_id
    ));
    drop(cap_registrations);

    let (rejected_projection_enqueue_tx, mut rejected_projection_enqueue_rx) =
        mpsc::unbounded_channel();
    manager.timelines.insert(
        key.clone(),
        synthetic_send_timeline_actor_handle_with_projection_probe(
            rejected_projection_enqueue_tx,
            None,
            false,
        ),
    );
    let publication_rejected_id = SubmissionId::new("publication-rejected");
    manager
        .handle_command(TimelineCommand::SubmitText {
            request_id: fake_rid(7_600),
            expected_account: test_session_key(),
            submission_id: publication_rejected_id.clone(),
            key: key.clone(),
            transaction_id: "txn-publication-rejected".to_owned(),
            document: ComposerDocument::from_plain_text("body"),
            draft_revision: 3.into(),
        })
        .await;
    manager.join_send_enqueue_workers().await;
    assert!(action_rx.try_recv().is_err());
    assert!(rejected_projection_enqueue_rx.try_recv().is_err());
    assert!(matches!(
        event_rx.try_recv(),
        Ok(CoreEvent::Timeline(TimelineEvent::SubmissionRejected {
            submission_id,
            kind: TimelineFailureKind::QueueOverflow,
            ..
        })) if submission_id == publication_rejected_id
    ));
    assert!(
        manager
            .send_completion
            .lock()
            .expect("coordinator")
            .projections_for_key(&key)
            .iter()
            .all(|projection| projection.client_txn_id != "txn-publication-rejected")
    );

    manager.timelines.remove(&key);
    let rejected_id = SubmissionId::new("unsubscribed-submission");
    manager
        .handle_command(TimelineCommand::SubmitText {
            request_id: fake_rid(7302),
            expected_account: test_session_key(),
            submission_id: rejected_id.clone(),
            key: key.clone(),
            transaction_id: "txn-rejected".to_owned(),
            document: ComposerDocument::from_plain_text("body"),
            draft_revision: 2.into(),
        })
        .await;
    assert!(action_rx.try_recv().is_err());
    assert!(matches!(
        event_rx.try_recv(),
        Ok(CoreEvent::Timeline(TimelineEvent::SubmissionRejected {
            submission_id,
            ..
        })) if submission_id == rejected_id
    ));

    let failed_id = SubmissionId::new("reducer-closed-submission");
    let (enqueue_tx, mut enqueue_rx) = mpsc::unbounded_channel();
    manager.timelines.insert(
        key.clone(),
        synthetic_send_timeline_actor_handle(enqueue_tx),
    );
    let (closed_action_tx, closed_action_rx) = mpsc::channel(1);
    drop(closed_action_rx);
    manager.action_tx = closed_action_tx;
    manager
        .handle_command(TimelineCommand::SubmitText {
            request_id: fake_rid(7303),
            expected_account: test_session_key(),
            submission_id: failed_id.clone(),
            key: key.clone(),
            transaction_id: "txn-reducer-closed".to_owned(),
            document: ComposerDocument::from_plain_text("body"),
            draft_revision: 3.into(),
        })
        .await;
    manager.join_send_enqueue_workers().await;
    assert!(
        enqueue_rx.try_recv().is_err(),
        "a rejected reducer action never releases the SDK enqueue permit"
    );
    assert!(
        manager
            .send_completion
            .lock()
            .expect("coordinator")
            .projections_for_key(&key)
            .iter()
            .all(|projection| projection.client_txn_id != "txn-reducer-closed"),
        "a rejected acceptance must retract the actor-visible pending projection"
    );
    manager
        .handle_command(TimelineCommand::SubmitText {
            request_id: fake_rid(7304),
            expected_account: test_session_key(),
            submission_id: failed_id.clone(),
            key,
            transaction_id: "txn-replayed".to_owned(),
            document: ComposerDocument::from_plain_text("changed"),
            draft_revision: 3.into(),
        })
        .await;
    assert!(
        enqueue_rx.try_recv().is_err(),
        "rejected replay never reaches SDK actor"
    );
    assert!(matches!(
        event_rx.try_recv(),
        Ok(CoreEvent::Timeline(TimelineEvent::SubmissionRejected { submission_id, .. }))
            if submission_id == failed_id
    ));
}

#[tokio::test]
async fn drive_send_enqueue_until_preflight_started_returns_when_sender_closes() {
    let mut manager = live_tail_test_manager(HashMap::new());
    let (preflight_started_tx, preflight_started_rx) = oneshot::channel();
    drop(preflight_started_tx);

    tokio::time::timeout(
        Duration::from_secs(1),
        manager.drive_send_enqueue_until_preflight_started(preflight_started_rx),
    )
    .await
    .expect("a dropped preflight-start sender must not stall the manager command loop");
}

#[tokio::test]
async fn submission_admission_permit_blocks_until_reducer_acceptance_and_aborts_on_drop() {
    let (permit_tx, mut permit_rx) = tokio::sync::oneshot::channel();
    assert!(matches!(
        permit_rx.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    permit_tx.send(()).expect("open admission permit");
    assert!(await_submission_admission(Some(permit_rx)).await);

    let (permit_tx, permit_rx) = tokio::sync::oneshot::channel::<()>();
    drop(permit_tx);
    assert!(
        !await_submission_admission(Some(permit_rx)).await,
        "dropped permit aborts actor SDK work"
    );
    assert!(
        await_submission_admission(None).await,
        "legacy sends need no permit"
    );
}

#[tokio::test]
async fn shutdown_acknowledges_after_timeline_children_are_dropped() {
    let (action_tx, _action_rx) = mpsc::channel(1);
    let (event_tx, _) = broadcast::channel(1);
    let handle = TimelineManagerActor::spawn(
        action_tx,
        event_tx,
        None,
        AccountWorkScheduler::default(),
        None,
        None,
    );
    let (acknowledged, acknowledgement) = tokio::sync::oneshot::channel();
    assert!(
        handle
            .send(TimelineMessage::Shutdown {
                acknowledged: Some(acknowledged),
            })
            .await
    );
    tokio::time::timeout(Duration::from_secs(1), acknowledgement)
        .await
        .expect("shutdown acknowledgement must not hang")
        .expect("timeline manager acknowledges shutdown");
}

#[tokio::test(start_paused = true)]
async fn shutdown_deadline_aborts_stalled_enqueue_worker_before_stopping_terminal_observer() {
    struct ObserverDrop(Arc<AtomicBool>);
    impl Drop for ObserverDrop {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }
    struct WorkerDrop {
        alive: Arc<AtomicBool>,
        observer_alive: Arc<AtomicBool>,
        settled_before_observer_stop: Arc<AtomicBool>,
    }
    impl Drop for WorkerDrop {
        fn drop(&mut self) {
            self.settled_before_observer_stop
                .store(self.observer_alive.load(Ordering::SeqCst), Ordering::SeqCst);
            self.alive.store(false, Ordering::SeqCst);
        }
    }

    let mut manager = live_tail_test_manager(HashMap::new());
    let observer_alive = Arc::new(AtomicBool::new(true));
    let worker_alive = Arc::new(AtomicBool::new(true));
    let settled_before_observer_stop = Arc::new(AtomicBool::new(false));
    let (observer_started, observer_ready) = oneshot::channel();
    manager.global_send_completion_observer_future = Some(Box::pin({
        let observer_alive = Arc::clone(&observer_alive);
        async move {
            let _drop = ObserverDrop(observer_alive);
            let _ = observer_started.send(());
            futures_util::future::pending::<()>().await;
        }
    }));
    let mut registration = SendCompletionRegistration::begin(
        Arc::clone(&manager.send_completion),
        manager.terminal_ingress.clone(),
        room_key(),
        "client-stalled-shutdown".to_owned(),
        None,
        fake_rid(7470),
        true,
    );
    registration.activate();
    let (worker_started, worker_ready) = oneshot::channel();
    manager.spawn_send_enqueue_future(registration, {
        let alive = Arc::clone(&worker_alive);
        let observer_alive = Arc::clone(&observer_alive);
        let settled_before_observer_stop = Arc::clone(&settled_before_observer_stop);
        async move {
            let _drop = WorkerDrop {
                alive,
                observer_alive,
                settled_before_observer_stop,
            };
            let _ = worker_started.send(());
            futures_util::future::pending::<Result<SendEnqueueSuccess, TimelineFailureKind>>().await
        }
    });
    let msg_tx = manager.msg_tx.clone();
    let run = executor::spawn(manager.run());
    observer_ready.await.expect("terminal observer started");
    worker_ready.await.expect("enqueue worker started");
    let (ack_tx, mut ack_rx) = oneshot::channel();
    msg_tx
        .send(TimelineMessage::Shutdown {
            acknowledged: Some(ack_tx),
        })
        .await
        .expect("shutdown command");

    tokio::task::yield_now().await;
    tokio::time::advance(SEND_ENQUEUE_WORKER_SHUTDOWN_DEADLINE).await;
    tokio::task::yield_now().await;

    assert!(
        matches!(ack_rx.try_recv(), Ok(())),
        "a stalled SDK enqueue must not hold shutdown acknowledgement forever"
    );
    assert!(!worker_alive.load(Ordering::SeqCst));
    assert!(settled_before_observer_stop.load(Ordering::SeqCst));
    assert!(!observer_alive.load(Ordering::SeqCst));
    run.await.expect("bounded manager shutdown");
}

#[tokio::test]
async fn shutdown_grace_polls_exact_terminal_observer_before_worker_quiescence() {
    let mut manager = live_tail_test_manager(HashMap::new());
    let key = room_key();
    let sdk_transaction_id = "sdk-shutdown-grace";
    let mut registration = SendCompletionRegistration::begin(
        Arc::clone(&manager.send_completion),
        manager.terminal_ingress.clone(),
        key.clone(),
        "client-shutdown-grace".to_owned(),
        None,
        fake_rid(7472),
        true,
    );
    registration.activate();

    let (updates_tx, updates_rx) = broadcast::channel(4);
    manager.global_send_completion_observer_future =
        Some(Box::pin(run_global_send_completion_observer(
            updates_rx,
            Arc::clone(&manager.send_completion),
            manager.terminal_ingress.clone(),
        )));
    let (release_tx, release_rx) = oneshot::channel();
    manager.spawn_send_enqueue_future(registration, async move {
        let _ = release_rx.await;
        Ok(SendEnqueueSuccess::terminal_only(
            sdk_transaction_id.to_owned(),
        ))
    });

    let queue_terminal = async move {
        tokio::task::yield_now().await;
        updates_tx
            .send(SendQueueUpdate {
                room_id: matrix_sdk::ruma::OwnedRoomId::try_from(key.room_id()).expect("room id"),
                update: RoomSendQueueUpdate::SentEvent {
                    transaction_id: matrix_sdk::ruma::OwnedTransactionId::from(sdk_transaction_id),
                    event_id: matrix_sdk::ruma::OwnedEventId::try_from("$shutdown-grace:test")
                        .expect("event id"),
                },
            })
            .expect("queue exact SDK terminal");
        let _ = release_tx.send(());
    };
    let ((), ()) = tokio::join!(manager.join_send_enqueue_workers(), queue_terminal);

    let terminal = manager
        .terminal_rx
        .try_recv()
        .expect("graceful drain observes the exact terminal before observer teardown");
    assert!(terminal.failure.is_none());
    assert!(matches!(
        terminal.completion,
        Some(TimelineSendCompletionDelivery { event_id, .. })
            if event_id == "$shutdown-grace:test"
    ));
}

#[tokio::test]
async fn shutdown_cleans_captured_room_keys_before_acknowledging() {
    struct DropSignal(Option<oneshot::Sender<()>>);
    impl Drop for DropSignal {
        fn drop(&mut self) {
            if let Some(signal) = self.0.take() {
                let _ = signal.send(());
            }
        }
    }

    let key = room_key();
    let generations = Arc::new(TimelineActorGenerationGate::default());
    generations.activate_after_quiescence(&key).await;
    let (dropped_tx, dropped_rx) = oneshot::channel();
    let child = executor::spawn(async move {
        let _signal = DropSignal(Some(dropped_tx));
        std::future::pending::<()>().await;
    });
    let (actor_tx, _actor_rx) = mpsc::channel(1);
    let (action_tx, mut action_rx) = mpsc::channel(4);
    let (event_tx, _) = broadcast::channel(4);
    let (msg_tx, msg_rx) = mpsc::channel(4);
    let (terminal_ingress, terminal_rx) = TimelineSendTerminalIngress::channel();
    let manager = TimelineManagerActor {
        session: None,
        room_list_service: None,
        room_subscription_checkpoint_task: None,
        room_subscription_service_epoch: 0,
        current_core_generation: None,
        room_leave_states: BTreeMap::new(),
        #[cfg(any(test, feature = "test-hooks"))]
        restored_room_subscription_probe: None,
        session_subscribed_rooms: BTreeSet::new(),
        subscribed_room_leases: BTreeMap::new(),
        subscription_room_seen: BTreeSet::new(),
        subscription_room_ordinals: BTreeMap::new(),
        next_subscription_room_ordinal: 0,
        global_response_commit: None,
        timelines: HashMap::from([(
            key.clone(),
            TimelineActorHandle {
                tx: actor_tx,
                control_tx: None,
                thread_summary_projection:
                    crate::timeline::actor::ThreadSummaryProjectionIngress::channel().0,
                position_rx: None,
                task: Some(child),
                auxiliary_tasks: Vec::new(),
                subscription_generation: None,
                enqueue_context: None,
            },
        )]),
        accepted_submissions: SubmissionAdmissionLedger::default(),
        send_completion: SharedSendCompletionCoordinator::default(),
        global_send_completion_observer_future: None,
        send_enqueue_workers: SendEnqueueWorkerSupervisor::new(terminal_ingress.clone()),
        read_workers: ReadWorkerSupervisor::unavailable(),
        action_tx,
        event_tx,
        msg_tx: msg_tx.clone(),
        msg_rx,
        control_rx: None,
        navigation_projection_rx: None,
        last_navigation_projection_generation: 0,
        terminal_ingress,
        terminal_rx,
        search_index_tx: None,
        ignored_user_ids: Default::default(),
        data_dir: None,
        link_preview_policy: LinkPreviewContext::default(),
        composer_formatting_options: ComposerFormattingOptions::default(),
        account_work: AccountWorkScheduler::default(),
        thread_root_projection_service: Arc::new(
            Mutex::new(ThreadRootProjectionService::default()),
        ),
        thread_root_projection_fetches: ThreadRootProjectionFetchRegistry::default(),
        thread_root_order: koushi_state::TimelineThreadRootOrder::LatestReply,
        timeline_actor_generations: generations.clone(),
        live_tail_refreshes: LiveTailRefreshCoordinator::new(),
        test_session_available: true,
    };
    let run = executor::spawn(async move { manager.run().await });
    let (ack_tx, ack_rx) = oneshot::channel();
    msg_tx
        .send(TimelineMessage::Shutdown {
            acknowledged: Some(ack_tx),
        })
        .await
        .expect("shutdown command");
    ack_rx.await.expect("shutdown acknowledgement");
    dropped_rx
        .await
        .expect("child dropped before acknowledgement");
    assert!(matches!(
        action_rx.recv().await,
        Some(actions) if matches!(actions.as_slice(), [AppAction::ThreadRootProjectionsCleared { room_id }] if room_id == key.room_id())
    ));
    assert!(
        !generations
            .state
            .lock()
            .expect("generation gate")
            .entries
            .contains_key(&key)
    );
    run.await.expect("manager shutdown");
}

#[tokio::test]
async fn manager_enqueue_worker_waits_for_reducer_acceptance_delivery() {
    let key = room_key();
    let (enqueue_tx, mut enqueue_rx) = mpsc::unbounded_channel();
    let (projection_probe_tx, projection_probe_rx) = oneshot::channel();
    let (action_tx, mut action_rx) = mpsc::channel(1);
    action_tx
        .try_send(Vec::new())
        .expect("pause reducer delivery");
    let (event_tx, mut event_rx) = broadcast::channel(4);
    let (msg_tx, msg_rx) = mpsc::channel(1);
    let (terminal_ingress, terminal_rx) = TimelineSendTerminalIngress::channel();
    let mut manager = TimelineManagerActor {
        session: None,
        room_list_service: None,
        room_subscription_checkpoint_task: None,
        room_subscription_service_epoch: 0,
        current_core_generation: None,
        room_leave_states: BTreeMap::new(),
        #[cfg(any(test, feature = "test-hooks"))]
        restored_room_subscription_probe: None,
        session_subscribed_rooms: BTreeSet::new(),
        subscribed_room_leases: BTreeMap::new(),
        subscription_room_seen: BTreeSet::new(),
        subscription_room_ordinals: BTreeMap::new(),
        next_subscription_room_ordinal: 0,
        global_response_commit: None,
        timelines: HashMap::from([(
            key.clone(),
            synthetic_send_timeline_actor_handle_with_projection_probe(
                enqueue_tx,
                Some(projection_probe_tx),
                true,
            ),
        )]),
        accepted_submissions: SubmissionAdmissionLedger::default(),
        send_completion: SharedSendCompletionCoordinator::default(),
        global_send_completion_observer_future: None,
        send_enqueue_workers: SendEnqueueWorkerSupervisor::new(terminal_ingress.clone()),
        read_workers: ReadWorkerSupervisor::unavailable(),
        action_tx,
        event_tx,
        msg_tx,
        msg_rx,
        control_rx: None,
        navigation_projection_rx: None,
        last_navigation_projection_generation: 0,
        terminal_ingress,
        terminal_rx,
        search_index_tx: None,
        ignored_user_ids: Default::default(),
        data_dir: None,
        link_preview_policy: LinkPreviewContext::default(),
        composer_formatting_options: ComposerFormattingOptions::default(),
        account_work: AccountWorkScheduler::default(),
        thread_root_projection_service: Arc::new(
            Mutex::new(ThreadRootProjectionService::default()),
        ),
        thread_root_projection_fetches: ThreadRootProjectionFetchRegistry::default(),
        thread_root_order: koushi_state::TimelineThreadRootOrder::LatestReply,
        timeline_actor_generations: Arc::new(TimelineActorGenerationGate::default()),
        live_tail_refreshes: LiveTailRefreshCoordinator::new(),
        test_session_available: true,
    };
    let submission_id = SubmissionId::new("paused-admission");
    let command_id = submission_id.clone();
    let registry = Arc::new(crate::composer_draft_lifecycle::ComposerDraftLeaseRegistry::new());
    let scope = crate::composer_draft_lifecycle::ComposerDraftScope {
        account: test_session_key(),
        target: koushi_state::ComposerTarget::Main {
            room_id: key.room_id().to_owned(),
        },
    };
    let renderer_generation = registry
        .begin_renderer_generation()
        .expect("begin renderer generation");
    let lease_id = registry
        .acquire(renderer_generation, scope.clone())
        .expect("acquire exact composer lease");
    let command_permit = registry
        .try_command_permit(renderer_generation, lease_id, &scope)
        .expect("admit exact composer command");
    let app_pending_permit = command_permit.clone();
    registry
        .release(renderer_generation, lease_id)
        .expect("release activation after command admission");
    let (rejected_tx, mut rejected_rx) = mpsc::unbounded_channel();
    let (acceptance_probe_tx, acceptance_probe_rx) = oneshot::channel();
    let forwarded_permit =
        crate::composer_draft_lifecycle::ForwardedComposerDraftPermit::new_with_acceptance_probe(
            fake_rid(7310),
            command_permit,
            rejected_tx,
            acceptance_probe_tx,
        );
    let route = tokio::spawn(async move {
        manager
            .handle_command_with_permit(
                TimelineCommand::SubmitText {
                    request_id: fake_rid(7310),
                    expected_account: test_session_key(),
                    submission_id: command_id,
                    key,
                    transaction_id: "txn-paused".to_owned(),
                    document: ComposerDocument::from_plain_text("body"),
                    draft_revision: 4.into(),
                },
                Some(forwarded_permit),
            )
            .await;
        manager
    });
    let projections = projection_probe_rx
        .await
        .expect("actor receives pending projection before acknowledging acceptance");
    assert!(matches!(projections.as_slice(), [projection] if matches!(
        &projection.item.id,
        koushi_protocol::event::TimelineItemId::Transaction { transaction_id }
            if transaction_id == "txn-paused"
    ) && projection.item.send_state == Some(koushi_protocol::event::TimelineSendState::Sending)));
    acceptance_probe_rx
        .await
        .expect("timeline reached acceptance projection");
    assert_eq!(
        registry.protected_targets(&scope.account),
        BTreeSet::from([scope.target.clone()]),
        "the forwarded permit must protect the exact target while reducer delivery is blocked"
    );
    assert!(
        enqueue_rx.try_recv().is_err(),
        "the manager worker must stay permit-blocked before reducer acceptance"
    );
    assert!(event_rx.try_recv().is_err());
    assert!(action_rx.recv().await.expect("pause marker").is_empty());
    assert!(
        matches!(action_rx.recv().await, Some(actions) if matches!(actions.as_slice(), [AppAction::ComposerSubmissionAcceptedAtRevision { submission_id: accepted, .. }] if accepted == &submission_id))
    );
    let mut manager = route.await.expect("manager route");
    assert!(
        matches!(event_rx.try_recv(), Ok(CoreEvent::Timeline(TimelineEvent::SubmissionAccepted { submission_id: accepted, .. })) if accepted == submission_id)
    );
    assert_eq!(
        registry.protected_targets(&scope.account),
        BTreeSet::from([scope.target.clone()]),
        "the AppActor pending clone must outlive timeline acceptance enqueue"
    );
    let mut registry_changes = registry.subscribe();
    registry_changes.borrow_and_update();
    drop(app_pending_permit);
    registry_changes
        .changed()
        .await
        .expect("pending acceptance permit release notification");
    assert!(
        registry.protected_targets(&scope.account).is_empty(),
        "the exact target becomes eligible only after the matching reducer acceptance"
    );
    assert!(
        rejected_rx.try_recv().is_err(),
        "successful acceptance enqueue must disarm rejection cleanup"
    );
    let request = tokio::time::timeout(Duration::from_secs(1), enqueue_rx.recv())
        .await
        .expect("accepted enqueue worker must be driven")
        .expect("accepted submission releases manager enqueue worker");
    assert!(matches!(
        request.payload,
        TimelineSendEnqueuePayload::Text { ref document, .. } if document.plain_body() == "body"
    ));
    assert!(
        request
            .response
            .send(Ok(SendEnqueueSuccess::terminal_only(
                "sdk-transaction".to_owned(),
            )))
            .is_ok(),
        "complete synthetic enqueue"
    );
    manager.join_send_enqueue_workers().await;
}

#[test]
fn submission_admission_tombstones_are_bounded_and_active_is_retained() {
    let mut ledger = SubmissionAdmissionLedger::default();
    let key = room_key();
    let active = SubmissionId::new("active");
    ledger.accept(active.clone(), key.clone(), "active-txn".to_owned());
    for index in 0..=MAX_SUBMISSION_TOMBSTONES {
        let id = SubmissionId::new(format!("terminal-{index}"));
        ledger.accept(id.clone(), key.clone(), format!("txn-{index}"));
        ledger.terminal(&id);
    }
    assert_eq!(ledger.tombstones.len(), MAX_SUBMISSION_TOMBSTONES);
    assert!(ledger.active.contains_key(&active));
    assert!(ledger.get(&SubmissionId::new("terminal-0")).is_none());
}

#[tokio::test]
async fn send_without_authoritative_account_session_fails_closed() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();

    runtime
        .inject_actions(vec![
            AppAction::RestoreSessionRequested,
            AppAction::RestoreSessionSucceeded(SessionInfo {
                homeserver: "https://test.test".to_owned(),
                user_id: "@a:test".to_owned(),
                device_id: "DEV".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            }),
            AppAction::CurrentDeviceTrustChanged(koushi_state::CurrentDeviceTrustState::Verified),
        ])
        .await;
    loop {
        if matches!(conn.snapshot().session, SessionState::Ready(_)) {
            break;
        }
        crate::executor::sleep(Duration::from_millis(5)).await;
    }

    let rid = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
        request_id: rid,
        key: room_key(),
        transaction_id: "txn-unsubscribed".to_owned(),
        document: koushi_state::ComposerDocument::from_plain_text("hello".to_owned()),
    }))
    .await
    .expect("submit");

    loop {
        let timeout = tokio::time::timeout(Duration::from_secs(5), conn.recv_event()).await;
        let event = timeout.expect("no timeout").expect("no lag");
        match event {
            CoreEvent::OperationFailed {
                request_id,
                failure,
            } if request_id == rid => {
                assert_eq!(failure, CoreFailure::SessionRequired);
                return;
            }
            _ => continue,
        }
    }
}

#[test]
fn send_completion_trace_orders_terminal_before_and_after_binding() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let coordinator = SharedSendCompletionCoordinator::default();
    let (ingress, _terminal_rx) = TimelineSendTerminalIngress::channel();
    let key = room_key();
    let mut owned_correlations = Vec::new();

    for (index, terminal_before_bind) in [true, false].into_iter().enumerate() {
        let mut registration = SendCompletionRegistration::begin(
            Arc::clone(&coordinator),
            ingress.clone(),
            key.clone(),
            format!("client-trace-{index}"),
            None,
            fake_rid(7400 + index as u64),
            true,
        );
        owned_correlations.push(
            registration
                .lifecycle_trace
                .as_ref()
                .expect("registration owns a lifecycle trace")
                .correlation(),
        );
        registration.activate();
        if terminal_before_bind {
            apply_send_completion_observation_and_handoff(
                &coordinator,
                &ingress,
                key.room_id(),
                SendCompletionObservation::Sent {
                    sdk_transaction_id: format!("sdk-trace-{index}"),
                    event_id: format!("$event-trace-{index}:test"),
                },
            );
        }
        registration.bind(format!("sdk-trace-{index}"));
        if !terminal_before_bind {
            apply_send_completion_observation_and_handoff(
                &coordinator,
                &ingress,
                key.room_id(),
                SendCompletionObservation::Sent {
                    sdk_transaction_id: format!("sdk-trace-{index}"),
                    event_id: format!("$event-trace-{index}:test"),
                },
            );
        }
    }

    let diagnostics = koushi_diagnostics::test_support::detail_snapshot();
    let records = diagnostics
        .records
        .iter()
        .filter(|record| {
            record.event.source == "core.send"
                && record.event.fields.iter().any(|field| {
                    matches!(
                        field.value,
                        DiagnosticValue::Correlation(value)
                            if owned_correlations.contains(&value)
                    )
                })
        })
        .collect::<Vec<_>>();
    let stages = records
        .iter()
        .map(|record| record.event.stage)
        .collect::<Vec<_>>();
    assert_eq!(
        stages,
        vec![
            "accepted",
            "sdk_enqueue_finished",
            "terminal_bound",
            "sdk_terminal_observed",
            "terminal_applied",
            "guard_released",
            "accepted",
            "sdk_enqueue_finished",
            "terminal_bound",
            "sdk_terminal_observed",
            "terminal_applied",
            "guard_released",
        ]
    );
    let correlations = records
        .chunks(6)
        .map(|trace| {
            trace
                .iter()
                .flat_map(|record| record.event.fields.iter())
                .find(|field| field.key == "correlation")
                .map(|field| field.value.clone())
        })
        .collect::<Vec<_>>();
    assert_eq!(correlations.len(), 2);
    assert!(correlations[0].is_some());
    assert!(correlations[1].is_some());
    assert_ne!(correlations[0], correlations[1]);
    for trace in records.chunks(6) {
        let trace_correlation = trace
            .iter()
            .flat_map(|record| record.event.fields.iter())
            .find(|field| field.key == "correlation")
            .map(|field| field.value.clone());
        assert!(trace.iter().all(|record| {
            record
                .event
                .fields
                .iter()
                .find(|field| field.key == "correlation")
                .map(|field| field.value.clone())
                == trace_correlation
        }));
        assert!(trace.iter().all(|record| {
            record.event.fields.iter().all(|field| {
                !matches!(
                    field.key,
                    "room_id" | "event_id" | "user_id" | "transaction_id" | "request_id"
                )
            })
        }));
    }
}

#[test]
fn pending_projection_uses_exact_ids_and_converges_with_or_without_local_echo() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let coordinator = SharedSendCompletionCoordinator::default();
    let (ingress, _terminal_rx) = TimelineSendTerminalIngress::channel();
    let key = room_key();
    let projection = |client_txn_id: &str| PendingSendProjection {
        key: key.clone(),
        sequence: 0,
        client_txn_id: client_txn_id.to_owned(),
        item: pending_send_item(client_txn_id, "private body", None, None, None),
        sdk_transaction_id: None,
        handle: None,
        terminal_event_id: None,
        phase: PendingSendPhase::Pending,
    };

    let mut normal = SendCompletionRegistration::begin_with_projection(
        Arc::clone(&coordinator),
        ingress.clone(),
        key.clone(),
        "client-normal".to_owned(),
        None,
        fake_rid(7402),
        true,
        Some(projection("client-normal")),
    );
    let normal_correlation = normal
        .lifecycle_trace
        .as_ref()
        .expect("normal trace")
        .correlation();
    normal.activate();
    normal.bind("sdk-normal".to_owned());
    {
        let mut owner = coordinator.lock().expect("coordinator");
        let visible = owner.projections_for_key(&key);
        assert!(matches!(visible.as_slice(), [projection] if matches!(
            &projection.item.id,
            koushi_protocol::event::TimelineItemId::Transaction { transaction_id }
                if transaction_id == "sdk-normal"
        )));
        assert_eq!(
            owner.reconcile_local_echo(key.room_id(), "sdk-normal"),
            Some(key.clone())
        );
        assert!(owner.projections_for_key(&key).is_empty());
    }

    let mut omitted = SendCompletionRegistration::begin_with_projection(
        Arc::clone(&coordinator),
        ingress.clone(),
        key.clone(),
        "client-omitted".to_owned(),
        None,
        fake_rid(7403),
        true,
        Some(projection("client-omitted")),
    );
    let omitted_correlation = omitted
        .lifecycle_trace
        .as_ref()
        .expect("omitted trace")
        .correlation();
    omitted.activate();
    omitted.bind("sdk-omitted".to_owned());
    assert!(
        !coordinator
            .lock()
            .expect("coordinator")
            .mark_retry_sending(&key, "sdk-omitted"),
        "a still-pending send cannot be resurrected through retry"
    );
    apply_send_completion_observation_and_handoff(
        &coordinator,
        &ingress,
        key.room_id(),
        SendCompletionObservation::Sent {
            sdk_transaction_id: "sdk-omitted".to_owned(),
            event_id: "$event-omitted:test".to_owned(),
        },
    );
    {
        let mut owner = coordinator.lock().expect("coordinator");
        let visible = owner.projections_for_key(&key);
        assert!(matches!(visible.as_slice(), [projection] if matches!(
            &projection.item.id,
            koushi_protocol::event::TimelineItemId::Event { event_id }
                if event_id == "$event-omitted:test"
        ) && projection.item.send_state == Some(koushi_protocol::event::TimelineSendState::Sent)));
        assert_eq!(owner.pending_projection_count(), 1);
        assert_eq!(
            owner.reconcile_local_echo(key.room_id(), "sdk-omitted"),
            None
        );
        assert_eq!(owner.projections_for_key(&key).len(), 1);
        let mut hydrated_item = visible[0].item.clone();
        hydrated_item.body = Some("authoritative fetched body".to_owned());
        hydrated_item.send_state = None;
        let correlation = SendCorrelationKey {
            room_id: key.room_id().to_owned(),
            sdk_transaction_id: "sdk-omitted".to_owned(),
        };
        assert_eq!(owner.pending_hydrations_for_key(&key).len(), 1);
        assert!(owner.begin_hydration(&correlation));
        assert!(owner.pending_hydrations_for_key(&key).is_empty());
        owner.finish_hydration_failure(&correlation);
        assert_eq!(owner.pending_hydrations_for_key(&key).len(), 1);
        assert!(owner.begin_hydration(&correlation));
        assert_eq!(
            owner.mark_hydrated(&correlation, hydrated_item),
            Some(key.clone())
        );
        assert_eq!(owner.pending_projection_count(), 0);
        let hydrated = owner.projections_for_key(&key);
        assert_eq!(hydrated.len(), 1);
        assert_eq!(
            hydrated[0].item.body.as_deref(),
            Some("authoritative fetched body")
        );
        assert_eq!(
            hydrated[0].item.send_state,
            Some(koushi_protocol::event::TimelineSendState::Sent)
        );
        assert_eq!(
            owner.reconcile_remote_event(key.room_id(), "$event-omitted:test"),
            Some(key.clone())
        );
        assert!(owner.projections_for_key(&key).is_empty());
        assert_eq!(owner.pending_projection_count(), 0);
        assert!(
            !owner.mark_retry_sending(&key, "sdk-omitted"),
            "a converged send cannot be resurrected through retry"
        );
    }

    let mut recoverable = SendCompletionRegistration::begin_with_projection(
        Arc::clone(&coordinator),
        ingress.clone(),
        key.clone(),
        "client-recoverable".to_owned(),
        None,
        fake_rid(7404),
        true,
        Some(projection("client-recoverable")),
    );
    recoverable.activate();
    recoverable.bind("sdk-recoverable".to_owned());
    apply_send_completion_observation_and_handoff(
        &coordinator,
        &ingress,
        key.room_id(),
        SendCompletionObservation::SendError {
            sdk_transaction_id: "sdk-recoverable".to_owned(),
            diagnostic: SendFailureDiagnostic {
                reason: "http",
                recoverable: true,
            },
        },
    );
    {
        let mut owner = coordinator.lock().expect("coordinator");
        assert!(owner.mark_retry_sending(&key, "sdk-recoverable"));
        let retried = owner.projections_for_key(&key);
        assert!(matches!(retried.as_slice(), [projection] if
            projection.phase == PendingSendPhase::Pending
                && projection.item.send_state
                    == Some(koushi_protocol::event::TimelineSendState::Sending)));
    }

    let diagnostics = koushi_diagnostics::test_support::detail_snapshot();
    let stages_for = |correlation| {
        diagnostics
            .records
            .iter()
            .filter(|record| {
                record.event.source == "core.send"
                    && record
                        .event
                        .fields
                        .iter()
                        .any(|field| field.value == DiagnosticValue::Correlation(correlation))
            })
            .map(|record| record.event.stage)
            .collect::<Vec<_>>()
    };
    assert!(stages_for(normal_correlation).contains(&"pending_projection_inserted"));
    assert!(stages_for(normal_correlation).contains(&"sdk_local_echo_merged"));
    assert!(stages_for(omitted_correlation).contains(&"sdk_local_echo_missing"));
    assert!(stages_for(omitted_correlation).contains(&"remote_echo_converged"));
}

#[test]
fn pending_projection_capacity_is_hard_bounded() {
    let coordinator = SharedSendCompletionCoordinator::default();
    let (ingress, _terminal_rx) = TimelineSendTerminalIngress::channel();
    let key = room_key();
    let mut registrations = Vec::new();
    for index in 0..MAX_PENDING_SEND_PROJECTIONS {
        let client_txn_id = format!("client-cap-{index}");
        registrations.push(SendCompletionRegistration::begin_with_projection(
            Arc::clone(&coordinator),
            ingress.clone(),
            key.clone(),
            client_txn_id.clone(),
            None,
            fake_rid(8_000 + index as u64),
            true,
            Some(PendingSendProjection {
                key: key.clone(),
                sequence: 0,
                item: pending_send_item(&client_txn_id, "body", None, None, None),
                client_txn_id,
                sdk_transaction_id: None,
                handle: None,
                terminal_event_id: None,
                phase: PendingSendPhase::Pending,
            }),
        ));
    }
    {
        let owner = coordinator.lock().expect("coordinator");
        assert_eq!(
            owner.pending_projection_count(),
            MAX_PENDING_SEND_PROJECTIONS
        );
        let visible = owner.projections_for_key(&key);
        assert_eq!(
            visible.first().map(|item| item.client_txn_id.as_str()),
            Some("client-cap-0")
        );
        assert_eq!(
            visible.last().map(|item| item.client_txn_id.as_str()),
            Some("client-cap-127")
        );
    }
    drop(registrations);
    assert_eq!(
        coordinator
            .lock()
            .expect("coordinator")
            .pending_projection_count(),
        0
    );
}

#[tokio::test]
async fn causal_hydration_retry_rearms_after_a_failed_exact_event_fetch() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let session = Arc::new(koushi_sdk::MatrixClientSession::from_client_for_testing(
        client.clone(),
        koushi_state::SessionInfo {
            homeserver: server.server().uri(),
            user_id: client.user_id().expect("synthetic user id").to_string(),
            device_id: client.device_id().expect("synthetic device id").to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    ));
    let key = room_key();
    let mut manager = live_tail_test_manager(HashMap::new());
    manager.session = Some(session);
    let (ingress, _terminal_rx) = TimelineSendTerminalIngress::channel();
    let mut registration = SendCompletionRegistration::begin_with_projection(
        Arc::clone(&manager.send_completion),
        ingress.clone(),
        key.clone(),
        "client-hydration-retry".to_owned(),
        None,
        fake_rid(8_999),
        true,
        Some(PendingSendProjection {
            key: key.clone(),
            sequence: 0,
            client_txn_id: "client-hydration-retry".to_owned(),
            item: pending_send_item("client-hydration-retry", "body", None, None, None),
            sdk_transaction_id: None,
            handle: None,
            terminal_event_id: None,
            phase: PendingSendPhase::Pending,
        }),
    );
    registration.activate();
    registration.bind("sdk-hydration-retry".to_owned());
    apply_send_completion_observation_and_handoff(
        &manager.send_completion,
        &ingress,
        key.room_id(),
        SendCompletionObservation::Sent {
            sdk_transaction_id: "sdk-hydration-retry".to_owned(),
            event_id: "$missing-hydration:test".to_owned(),
        },
    );

    manager.retry_pending_send_hydrations(&key);
    assert_eq!(manager.send_enqueue_workers.tasks.len(), 1);
    assert!(
        manager
            .send_completion
            .lock()
            .expect("coordinator")
            .pending_hydrations_for_key(&key)
            .is_empty(),
        "the exact hydration is marked in flight"
    );
    let completion = manager
        .send_enqueue_workers
        .tasks
        .next()
        .await
        .expect("missing-room hydration settles");
    manager
        .handle_send_enqueue_worker_completion(completion)
        .await;
    assert_eq!(
        manager
            .send_completion
            .lock()
            .expect("coordinator")
            .pending_hydrations_for_key(&key)
            .len(),
        1,
        "a causal subscribe/live-tail wake can retry after failure"
    );
}

#[test]
fn hydrated_sent_projection_cache_evicts_oldest_without_consuming_active_capacity() {
    let coordinator = SharedSendCompletionCoordinator::default();
    let (ingress, _terminal_rx) = TimelineSendTerminalIngress::channel();
    let key = room_key();
    for index in 0..=MAX_PENDING_SEND_PROJECTIONS {
        let client_txn_id = format!("client-hydrated-{index}");
        let sdk_txn_id = format!("sdk-hydrated-{index}");
        let event_id = format!("$event-hydrated-{index}:test");
        let mut registration = SendCompletionRegistration::begin_with_projection(
            Arc::clone(&coordinator),
            ingress.clone(),
            key.clone(),
            client_txn_id.clone(),
            None,
            fake_rid(9_000 + index as u64),
            true,
            Some(PendingSendProjection {
                key: key.clone(),
                sequence: 0,
                item: pending_send_item(&client_txn_id, "body", None, None, None),
                client_txn_id,
                sdk_transaction_id: None,
                handle: None,
                terminal_event_id: None,
                phase: PendingSendPhase::Pending,
            }),
        );
        registration.activate();
        registration.bind(sdk_txn_id.clone());
        apply_send_completion_observation_and_handoff(
            &coordinator,
            &ingress,
            key.room_id(),
            SendCompletionObservation::Sent {
                sdk_transaction_id: sdk_txn_id.clone(),
                event_id,
            },
        );
        let correlation = SendCorrelationKey {
            room_id: key.room_id().to_owned(),
            sdk_transaction_id: sdk_txn_id,
        };
        let mut owner = coordinator.lock().expect("coordinator");
        let hydrated_item = owner
            .projections_for_key(&key)
            .into_iter()
            .find(|projection| {
                projection.sdk_transaction_id.as_deref()
                    == Some(correlation.sdk_transaction_id.as_str())
            })
            .expect("current retained projection")
            .item;
        assert!(owner.begin_hydration(&correlation));
        assert_eq!(
            owner.mark_hydrated(&correlation, hydrated_item),
            Some(key.clone())
        );
        assert_eq!(owner.pending_projection_count(), 0);
    }
    assert_eq!(
        coordinator
            .lock()
            .expect("coordinator")
            .projections_for_key(&key)
            .len(),
        MAX_PENDING_SEND_PROJECTIONS
    );
}

#[test]
fn send_failure_trace_records_only_closed_failure_fields() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
        .records
        .len();
    let key = room_key();
    let mut trace = SendLifecycleTrace::new(&key, true);
    let correlation = trace.correlation();

    trace.stage_with_failure(
        "sdk_terminal_observed",
        Some("failed"),
        Some("immediate"),
        SendFailureDiagnostic {
            reason: "http",
            recoverable: true,
        },
    );

    let diagnostics = koushi_diagnostics::test_support::detail_snapshot();
    let event = &diagnostics.records[diagnostic_start..]
        .iter()
        .find(|record| {
            record.event.source == "core.send"
                && record.event.stage == "sdk_terminal_observed"
                && record.event.fields.iter().any(|field| {
                    field.key == "correlation"
                        && field.value == DiagnosticValue::Correlation(correlation)
                })
        })
        .expect("send failure terminal diagnostic")
        .event;

    assert!(
        event.fields.iter().any(|field| {
            field.key == "reason" && field.value == DiagnosticValue::Token("http")
        })
    );
    assert!(event.fields.iter().any(|field| {
        field.key == "recoverable" && field.value == DiagnosticValue::Boolean(true)
    }));
    assert!(event.fields.iter().all(|field| {
        !matches!(
            field.key,
            "room_id"
                | "event_id"
                | "user_id"
                | "device_id"
                | "transaction_id"
                | "endpoint"
                | "error"
        )
    }));
}

#[test]
fn encrypted_send_local_store_diagnostics_are_correlated_and_privacy_safe() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
        .records
        .len();
    let key = room_key();
    let trace = SendLifecycleTrace::new(&key, true);
    let correlation = trace.correlation();

    trace.record_encryption_local_store_snapshot(&EncryptedSendDiagnosticSnapshot {
        room_encryption: RoomEncryptionDiagnosticState::Encrypted,
        own_user_tracking: OwnUserTrackingDiagnosticState::Tracked,
        own_device_present: Some(true),
        known_own_device_count: Some(4),
        known_own_other_device_count: Some(3),
        key_capable_own_other_device_count: Some(2),
        cross_signed_own_other_device_count: Some(2),
        dehydrated_own_other_device_count: Some(1),
        blacklisted_own_other_device_count: Some(1),
    });
    let diagnostics = koushi_diagnostics::test_support::detail_snapshot();
    let record = diagnostics.records[diagnostic_start..]
        .iter()
        .find(|record| {
            record.event.source == "core.send"
                && record.event.stage == "encryption_local_store_snapshot"
                && record.event.fields.iter().any(|field| {
                    field.key == "correlation"
                        && field.value == DiagnosticValue::Correlation(correlation)
                })
        })
        .expect("encrypted-send snapshot diagnostic");

    for (key, value) in [
        ("room_encryption", DiagnosticValue::Token("encrypted")),
        ("recipient_strategy", DiagnosticValue::Token("all_devices")),
        (
            "snapshot_consistency",
            DiagnosticValue::Token("best_effort_concurrent_local_store"),
        ),
        ("own_user_tracking", DiagnosticValue::Token("tracked")),
        ("own_device_present", DiagnosticValue::Boolean(true)),
        ("known_own_device_count", DiagnosticValue::Count(4)),
        ("known_own_other_device_count", DiagnosticValue::Count(3)),
        (
            "key_capable_own_other_device_count",
            DiagnosticValue::Count(2),
        ),
        (
            "cross_signed_own_other_device_count",
            DiagnosticValue::Count(2),
        ),
        (
            "dehydrated_own_other_device_count",
            DiagnosticValue::Count(1),
        ),
        (
            "blacklisted_own_other_device_count",
            DiagnosticValue::Count(1),
        ),
    ] {
        assert!(
            record
                .event
                .fields
                .iter()
                .any(|field| { field.key == key && field.value == value }),
            "missing {key}"
        );
    }
    assert!(record.event.fields.iter().all(|field| {
        !matches!(
            field.key,
            "room_id"
                | "event_id"
                | "user_id"
                | "device_id"
                | "session_id"
                | "transaction_id"
                | "request_id"
                | "message"
                | "key"
                | "key_material"
        )
    }));
}

#[tokio::test]
async fn manager_coordinator_survives_unsubscribe_until_sdk_terminal() {
    let key = room_key();
    let mut manager = live_tail_test_manager(HashMap::from([(
        key.clone(),
        gap_demand_test_actor_handle("send-owner", Arc::new(Mutex::new(Vec::new()))),
    )]));
    let request_id = fake_rid(7410);
    let mut registration = SendCompletionRegistration::begin(
        Arc::clone(&manager.send_completion),
        manager.terminal_ingress.clone(),
        key.clone(),
        "client-unsubscribe-unit".to_owned(),
        None,
        request_id,
        true,
    );
    registration.activate();
    registration.bind("sdk-unsubscribe-unit".to_owned());

    manager
        .handle_command(TimelineCommand::Unsubscribe {
            request_id: fake_rid(7411),
            key: key.clone(),
        })
        .await;
    assert!(!manager.timelines.contains_key(&key));
    apply_send_completion_observation_and_handoff(
        &manager.send_completion,
        &manager.terminal_ingress,
        key.room_id(),
        SendCompletionObservation::Sent {
            sdk_transaction_id: "sdk-unsubscribe-unit".to_owned(),
            event_id: "$event-unsubscribe-unit:test".to_owned(),
        },
    );

    let handoff = manager
        .terminal_rx
        .recv()
        .await
        .expect("manager-owned completion after unsubscribe");
    assert!(matches!(
        handoff.completion,
        Some(TimelineSendCompletionDelivery {
            request_id: delivered_request_id,
            key: delivered_key,
            transaction_id,
            event_id,
            ..
        }) if delivered_request_id == request_id
            && delivered_key == key
            && transaction_id == "client-unsubscribe-unit"
            && event_id == "$event-unsubscribe-unit:test"
    ));
}

#[tokio::test]
async fn manager_owned_prebind_enqueue_survives_room_and_thread_unsubscribe() {
    let account = AccountKey("@prebind-owner:test".to_owned());
    let keys = [
        TimelineKey::room(account.clone(), "!prebind-room:test"),
        TimelineKey {
            account_key: account,
            kind: TimelineKind::Thread {
                room_id: "!prebind-room:test".to_owned(),
                root_event_id: "$prebind-root:test".to_owned(),
            },
        },
    ];

    for (serial, key) in keys.into_iter().enumerate() {
        let mut manager = live_tail_test_manager(HashMap::from([(
            key.clone(),
            gap_demand_test_actor_handle("prebind", Arc::new(Mutex::new(Vec::new()))),
        )]));
        let request_id = fake_rid(7430 + serial as u64);
        let sdk_transaction_id = format!("sdk-prebind-{serial}");
        let mut registration = SendCompletionRegistration::begin(
            Arc::clone(&manager.send_completion),
            manager.terminal_ingress.clone(),
            key.clone(),
            format!("client-prebind-{serial}"),
            None,
            request_id,
            true,
        );
        registration.activate();
        let (durably_saved, saved) = oneshot::channel();
        let (release, released) = oneshot::channel();
        manager.spawn_send_enqueue_future(registration, async move {
            let _ = durably_saved.send(());
            let _ = released.await;
            Ok(SendEnqueueSuccess::terminal_only(sdk_transaction_id))
        });
        poll_manager_enqueue_workers_once(&mut manager).await;
        tokio::time::timeout(Duration::from_secs(1), saved)
            .await
            .expect("pre-bind enqueue worker must be driven")
            .expect("synthetic QueueStorage save committed");

        manager
            .handle_command(TimelineCommand::Unsubscribe {
                request_id: fake_rid(7440 + serial as u64),
                key: key.clone(),
            })
            .await;
        assert!(
            manager.terminal_rx.try_recv().is_err(),
            "actor removal must not abandon the manager-owned pre-bind registration"
        );

        let _ = release.send(());
        manager.join_send_enqueue_workers().await;
        apply_send_completion_observation_and_handoff(
            &manager.send_completion,
            &manager.terminal_ingress,
            key.room_id(),
            SendCompletionObservation::Sent {
                sdk_transaction_id: format!("sdk-prebind-{serial}"),
                event_id: format!("$event-prebind-{serial}:test"),
            },
        );
        let terminal = manager
            .terminal_rx
            .try_recv()
            .expect("correlated terminal after pre-bind unsubscribe");
        assert!(terminal.failure.is_none());
        assert!(matches!(
            terminal.completion,
            Some(TimelineSendCompletionDelivery {
                request_id: completed_request_id,
                key: completed_key,
                ..
            }) if completed_request_id == request_id && completed_key == key
        ));
        assert!(manager.terminal_rx.try_recv().is_err());
    }
}

#[tokio::test]
async fn manager_drop_aborts_owned_observer_and_send_enqueue_workers() {
    struct OwnedTaskDropFlag(Arc<AtomicBool>);

    impl Drop for OwnedTaskDropFlag {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }

    let mut manager = live_tail_test_manager(HashMap::new());
    let observer_alive = Arc::new(AtomicBool::new(true));
    manager.global_send_completion_observer_future = Some(Box::pin({
        let observer_drop = OwnedTaskDropFlag(Arc::clone(&observer_alive));
        async move {
            let _drop = observer_drop;
            futures_util::future::pending::<()>().await;
        }
    }));
    let mut registration = SendCompletionRegistration::begin(
        Arc::clone(&manager.send_completion),
        manager.terminal_ingress.clone(),
        room_key(),
        "client-manager-drop".to_owned(),
        None,
        fake_rid(7465),
        true,
    );
    registration.activate();
    let worker_alive = Arc::new(AtomicBool::new(true));
    manager.spawn_send_enqueue_future(registration, {
        let worker_drop = OwnedTaskDropFlag(Arc::clone(&worker_alive));
        async move {
            let _drop = worker_drop;
            futures_util::future::pending::<Result<SendEnqueueSuccess, TimelineFailureKind>>().await
        }
    });
    drop(manager);

    assert!(
        !observer_alive.load(Ordering::SeqCst),
        "dropping the manager must synchronously drop the owned observer future"
    );
    assert!(
        !worker_alive.load(Ordering::SeqCst),
        "dropping the manager must synchronously drop every owned enqueue future"
    );

    let quiesced = tokio::time::timeout(Duration::from_secs(1), async {
        while observer_alive.load(Ordering::SeqCst) || worker_alive.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(
        quiesced.is_ok(),
        "unexpected manager drop must quiesce observer={} worker={}",
        !observer_alive.load(Ordering::SeqCst),
        !worker_alive.load(Ordering::SeqCst),
    );
}

#[tokio::test]
async fn panicked_enqueue_future_is_fail_closed_without_stopping_manager_workers() {
    let mut manager = live_tail_test_manager(HashMap::new());
    let mut panicked_registration = SendCompletionRegistration::begin(
        Arc::clone(&manager.send_completion),
        manager.terminal_ingress.clone(),
        room_key(),
        "client-panicked-enqueue".to_owned(),
        None,
        fake_rid(7466),
        true,
    );
    panicked_registration.activate();
    manager.spawn_send_enqueue_future(panicked_registration, async move {
        panic!("synthetic enqueue panic");
        #[allow(unreachable_code)]
        Err(TimelineFailureKind::Sdk)
    });

    let completion = manager
        .send_enqueue_workers
        .tasks
        .next()
        .await
        .expect("caught panic still settles the supervised future");
    manager
        .handle_send_enqueue_worker_completion(completion)
        .await;
    let panic_terminal = manager
        .terminal_rx
        .try_recv()
        .expect("registration drop emits one private-safe terminal");
    assert!(matches!(
        panic_terminal.failure,
        Some(TimelineSendFailureDelivery {
            failure: CoreFailure::TimelineOperationFailed {
                kind: TimelineFailureKind::QueueOverflow,
            },
            ..
        })
    ));

    let mut next_registration = SendCompletionRegistration::begin(
        Arc::clone(&manager.send_completion),
        manager.terminal_ingress.clone(),
        room_key(),
        "client-after-panic".to_owned(),
        None,
        fake_rid(7467),
        true,
    );
    next_registration.activate();
    manager.spawn_send_enqueue_future(
        next_registration,
        async move { Err(TimelineFailureKind::Sdk) },
    );
    let completion = manager
        .send_enqueue_workers
        .tasks
        .next()
        .await
        .expect("manager continues polling workers after an isolated panic");
    manager
        .handle_send_enqueue_worker_completion(completion)
        .await;
    let next_terminal = manager
        .terminal_rx
        .try_recv()
        .expect("later worker terminal is still delivered");
    assert!(matches!(
        next_terminal.failure,
        Some(TimelineSendFailureDelivery {
            failure: CoreFailure::TimelineOperationFailed {
                kind: TimelineFailureKind::Sdk,
            },
            ..
        })
    ));
    assert!(manager.terminal_rx.try_recv().is_err());
}

#[test]
fn manager_coordinator_keeps_same_room_room_and_thread_keys_collision_safe() {
    let account = AccountKey("@send-owner:test".to_owned());
    let room_key = TimelineKey::room(account.clone(), "!shared-room:test");
    let thread_key = TimelineKey {
        account_key: account,
        kind: TimelineKind::Thread {
            room_id: "!shared-room:test".to_owned(),
            root_event_id: "$thread-root:test".to_owned(),
        },
    };
    let coordinator = SharedSendCompletionCoordinator::default();
    let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
    let mut room_registration = SendCompletionRegistration::begin(
        Arc::clone(&coordinator),
        ingress.clone(),
        room_key.clone(),
        "client-room".to_owned(),
        None,
        fake_rid(7420),
        true,
    );
    room_registration.activate();
    room_registration.bind("sdk-room".to_owned());
    let mut thread_registration = SendCompletionRegistration::begin(
        Arc::clone(&coordinator),
        ingress.clone(),
        thread_key.clone(),
        "client-thread".to_owned(),
        None,
        fake_rid(7421),
        true,
    );
    thread_registration.activate();
    thread_registration.bind("sdk-thread".to_owned());

    for (sdk_transaction_id, event_id) in [
        ("sdk-thread", "$event-thread:test"),
        ("sdk-room", "$event-room:test"),
    ] {
        apply_send_completion_observation_and_handoff(
            &coordinator,
            &ingress,
            "!shared-room:test",
            SendCompletionObservation::Sent {
                sdk_transaction_id: sdk_transaction_id.to_owned(),
                event_id: event_id.to_owned(),
            },
        );
    }

    let thread_handoff = terminal_rx.try_recv().expect("thread terminal first");
    let room_handoff = terminal_rx.try_recv().expect("room terminal second");
    assert!(matches!(
        thread_handoff.completion,
        Some(TimelineSendCompletionDelivery { key, .. }) if key == thread_key
    ));
    assert!(matches!(
        room_handoff.completion,
        Some(TimelineSendCompletionDelivery { key, .. }) if key == room_key
    ));
}

#[test]
fn unmatched_terminal_cohort_overflow_fails_safe_once_without_unbounded_growth() {
    let key = room_key();
    let coordinator = SharedSendCompletionCoordinator::default();
    let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
    let mut registration = SendCompletionRegistration::begin(
        Arc::clone(&coordinator),
        ingress.clone(),
        key.clone(),
        "client-cohort".to_owned(),
        None,
        fake_rid(7430),
        true,
    );
    registration.activate();

    apply_send_completion_observation_and_handoff(
        &coordinator,
        &ingress,
        key.room_id(),
        SendCompletionObservation::Sent {
            sdk_transaction_id: "sdk-cohort-candidate".to_owned(),
            event_id: "$event-cohort-candidate:test".to_owned(),
        },
    );
    apply_send_completion_observation_and_handoff(
        &coordinator,
        &ingress,
        key.room_id(),
        SendCompletionObservation::Sent {
            sdk_transaction_id: "sdk-cohort-overflow".to_owned(),
            event_id: "$event-cohort-overflow:test".to_owned(),
        },
    );
    let overflow = terminal_rx.try_recv().expect("cohort overflow terminal");
    assert!(matches!(
        overflow.failure,
        Some(TimelineSendFailureDelivery {
            request_id,
            failure: CoreFailure::TimelineOperationFailed {
                kind: TimelineFailureKind::QueueOverflow,
            },
        }) if request_id == fake_rid(7430)
    ));
    assert_eq!(
        coordinator
            .lock()
            .expect("send completion coordinator")
            .unmatched_terminals
            .len(),
        1,
        "one active unbound registration admits only one unmatched transaction cohort"
    );

    apply_send_completion_observation_and_handoff(
        &coordinator,
        &ingress,
        key.room_id(),
        SendCompletionObservation::Sent {
            sdk_transaction_id: "sdk-cohort-overflow-again".to_owned(),
            event_id: "$event-cohort-overflow-again:test".to_owned(),
        },
    );
    assert!(
        terminal_rx.try_recv().is_err(),
        "cohort overflow failure must be reported once per active request"
    );

    registration.bind("sdk-cohort-candidate".to_owned());
    let completion = terminal_rx.try_recv().expect("retained exact terminal");
    assert!(completion.failure.is_none());
    assert!(matches!(
        completion.completion,
        Some(TimelineSendCompletionDelivery { key: delivered_key, .. })
            if delivered_key == key
    ));
}

#[test]
fn known_enqueue_failure_and_active_registration_abort_have_distinct_terminals() {
    let key = room_key();
    let coordinator = SharedSendCompletionCoordinator::default();
    let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
    let mut known_failure = SendCompletionRegistration::begin(
        Arc::clone(&coordinator),
        ingress.clone(),
        key.clone(),
        "client-known-failure".to_owned(),
        None,
        fake_rid(7435),
        true,
    );
    known_failure.activate();
    known_failure.fail_known(TimelineFailureKind::Forbidden);
    drop(known_failure);
    assert!(matches!(
        terminal_rx.try_recv().expect("known failure").failure,
        Some(TimelineSendFailureDelivery {
            failure: CoreFailure::TimelineOperationFailed {
                kind: TimelineFailureKind::Forbidden,
            },
            ..
        })
    ));
    assert!(terminal_rx.try_recv().is_err());

    let mut abandoned = SendCompletionRegistration::begin(
        Arc::clone(&coordinator),
        ingress,
        key,
        "client-abandoned".to_owned(),
        None,
        fake_rid(7436),
        true,
    );
    abandoned.activate();
    drop(abandoned);
    assert!(matches!(
        terminal_rx.try_recv().expect("abandoned failure").failure,
        Some(TimelineSendFailureDelivery {
            failure: CoreFailure::TimelineOperationFailed {
                kind: TimelineFailureKind::QueueOverflow,
            },
            ..
        })
    ));
    assert!(terminal_rx.try_recv().is_err());
}

#[tokio::test]
async fn global_send_observer_lag_fails_bound_and_unbound_in_registration_order() {
    let account = AccountKey("@lag-owner:test".to_owned());
    let first_key = TimelineKey::room(account.clone(), "!lag-room:test");
    let second_key = TimelineKey {
        account_key: account,
        kind: TimelineKind::Focused {
            room_id: "!lag-room:test".to_owned(),
            event_id: "$focus:test".to_owned(),
        },
    };
    let coordinator = SharedSendCompletionCoordinator::default();
    let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
    let mut first = SendCompletionRegistration::begin(
        Arc::clone(&coordinator),
        ingress.clone(),
        first_key,
        "client-lag-first".to_owned(),
        None,
        fake_rid(7440),
        true,
    );
    first.activate();
    first.bind("sdk-lag-first".to_owned());
    let mut second = SendCompletionRegistration::begin(
        Arc::clone(&coordinator),
        ingress.clone(),
        second_key,
        "client-lag-second".to_owned(),
        None,
        fake_rid(7441),
        false,
    );
    second.activate();

    let (updates_tx, updates_rx) = broadcast::channel(1);
    let room_id = matrix_sdk::ruma::OwnedRoomId::try_from("!lag-room:test").expect("lag room id");
    for transaction_id in ["sdk-overflow-one", "sdk-overflow-two"] {
        updates_tx
            .send(matrix_sdk::send_queue::SendQueueUpdate {
                room_id: room_id.clone(),
                update: RoomSendQueueUpdate::RetryEvent {
                    transaction_id: matrix_sdk::ruma::OwnedTransactionId::from(transaction_id),
                },
            })
            .expect("queue lag update");
    }
    drop(updates_tx);
    run_global_send_completion_observer(updates_rx, Arc::clone(&coordinator), ingress.clone())
        .await;

    let first_failure = terminal_rx.try_recv().expect("first lag failure");
    let second_failure = terminal_rx.try_recv().expect("second lag failure");
    assert!(matches!(
        first_failure.failure,
        Some(TimelineSendFailureDelivery { request_id, .. }) if request_id == fake_rid(7440)
    ));
    assert!(first_failure.action.is_some());
    assert!(matches!(
        second_failure.failure,
        Some(TimelineSendFailureDelivery { request_id, .. }) if request_id == fake_rid(7441)
    ));
    assert!(second_failure.action.is_none());
    assert!(terminal_rx.try_recv().is_err());

    apply_send_completion_observation_loss_and_handoff(&coordinator, &ingress, None);
    assert!(
        terminal_rx.try_recv().is_err(),
        "a repeated lag notification must not report either request twice"
    );
    second.bind("sdk-lag-second".to_owned());
    apply_send_completion_observation_and_handoff(
        &coordinator,
        &ingress,
        "!lag-room:test",
        SendCompletionObservation::Sent {
            sdk_transaction_id: "sdk-lag-second".to_owned(),
            event_id: "$event-after-lag:test".to_owned(),
        },
    );
    let recovered = terminal_rx.try_recv().expect("exact terminal after lag");
    assert!(recovered.action.is_none());
    assert!(recovered.failure.is_none());
    assert!(recovered.completion.is_some());
}

#[tokio::test]
async fn shutdown_joins_observer_then_actor_and_drains_registration_failure_before_ack() {
    struct OrderedDrop {
        label: &'static str,
        log: Arc<Mutex<Vec<&'static str>>>,
    }

    impl Drop for OrderedDrop {
        fn drop(&mut self) {
            self.log
                .lock()
                .expect("shutdown ordering log")
                .push(self.label);
        }
    }

    let key = room_key();
    let mut manager = live_tail_test_manager(HashMap::new());
    let (action_tx, mut action_rx) = mpsc::channel(8);
    manager.action_tx = action_tx;
    let order = Arc::new(Mutex::new(Vec::new()));
    let (observer_started_tx, observer_started_rx) = oneshot::channel();
    manager.global_send_completion_observer_future = Some(Box::pin({
        let order = Arc::clone(&order);
        async move {
            let _drop = OrderedDrop {
                label: "observer",
                log: order,
            };
            let _ = observer_started_tx.send(());
            std::future::pending::<()>().await;
        }
    }));

    let mut registration = SendCompletionRegistration::begin(
        Arc::clone(&manager.send_completion),
        manager.terminal_ingress.clone(),
        key.clone(),
        "client-shutdown-order".to_owned(),
        None,
        fake_rid(7450),
        true,
    );
    registration.activate();
    let (actor_started_tx, actor_started_rx) = oneshot::channel();
    let (actor_tx, _actor_rx) = mpsc::channel(1);
    let actor_task = executor::spawn({
        let order = Arc::clone(&order);
        async move {
            let _drop = OrderedDrop {
                label: "actor",
                log: order,
            };
            let _registration = registration;
            let _ = actor_started_tx.send(());
            std::future::pending::<()>().await;
        }
    });
    manager.timelines.insert(
        key,
        TimelineActorHandle {
            tx: actor_tx,
            control_tx: None,
            thread_summary_projection:
                crate::timeline::actor::ThreadSummaryProjectionIngress::channel().0,
            position_rx: None,
            task: Some(actor_task),
            auxiliary_tasks: Vec::new(),
            subscription_generation: None,
            enqueue_context: None,
        },
    );
    let manager_tx = manager.msg_tx.clone();
    let manager_task = executor::spawn(manager.run());
    observer_started_rx.await.expect("observer started");
    actor_started_rx.await.expect("actor started");

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    manager_tx
        .send(TimelineMessage::Shutdown {
            acknowledged: Some(shutdown_tx),
        })
        .await
        .expect("shutdown command");
    shutdown_rx.await.expect("shutdown acknowledgement");
    manager_task.await.expect("manager shutdown task");

    let observed_order = order.lock().expect("shutdown ordering log").clone();
    assert_eq!(
        observed_order,
        ["observer", "actor"],
        "the sole observer must stop before actor registration producers"
    );
    let mut send_failure_count = 0;
    while let Ok(actions) = action_rx.try_recv() {
        send_failure_count += actions
            .iter()
            .filter(|action| {
                matches!(
                    action,
                    AppAction::SendTextFailed { transaction_id, .. }
                        if transaction_id == "client-shutdown-order"
                )
            })
            .count();
    }
    assert_eq!(
        send_failure_count, 1,
        "shutdown must drain one fail-safe terminal"
    );
}

#[test]
fn coordinator_maps_sdk_transaction_to_client_request_and_completion() {
    let key = room_key();
    let coordinator = SharedSendCompletionCoordinator::default();
    let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
    let mut registration = SendCompletionRegistration::begin(
        Arc::clone(&coordinator),
        ingress.clone(),
        key.clone(),
        "client-txn-42".to_owned(),
        None,
        fake_rid(42),
        true,
    );
    registration.activate();
    registration.bind("sdk-auto-generated-txn".to_owned());

    assert_eq!(
        coordinator
            .lock()
            .expect("send completion coordinator")
            .pending_send(key.room_id(), "sdk-auto-generated-txn"),
        Some((&key, "client-txn-42", fake_rid(42)))
    );
    apply_send_completion_observation_and_handoff(
        &coordinator,
        &ingress,
        key.room_id(),
        SendCompletionObservation::Sent {
            sdk_transaction_id: "sdk-auto-generated-txn".to_owned(),
            event_id: "$event-42:test".to_owned(),
        },
    );
    assert!(matches!(
        terminal_rx.try_recv().expect("mapped completion").completion,
        Some(TimelineSendCompletionDelivery {
            request_id,
            transaction_id,
            event_id,
            ..
        }) if request_id == fake_rid(42)
            && transaction_id == "client-txn-42"
            && event_id == "$event-42:test"
    ));
}

#[test]
fn send_completion_race_delivers_completion_when_sent_event_arrives_first() {
    let key = room_key();
    let coordinator = SharedSendCompletionCoordinator::default();
    let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
    let mut registration = SendCompletionRegistration::begin(
        Arc::clone(&coordinator),
        ingress.clone(),
        key.clone(),
        "client-race-txn".to_owned(),
        None,
        fake_rid(77),
        true,
    );
    registration.activate();
    apply_send_completion_observation_and_handoff(
        &coordinator,
        &ingress,
        key.room_id(),
        SendCompletionObservation::Sent {
            sdk_transaction_id: "sdk-race-txn".to_owned(),
            event_id: "$event-race:test".to_owned(),
        },
    );
    assert!(terminal_rx.try_recv().is_err());

    registration.bind("sdk-race-txn".to_owned());
    assert!(matches!(
        terminal_rx.try_recv().expect("early completion correlated").completion,
        Some(TimelineSendCompletionDelivery {
            request_id,
            transaction_id,
            event_id,
            ..
        }) if request_id == fake_rid(77)
            && transaction_id == "client-race-txn"
            && event_id == "$event-race:test"
    ));
}

#[test]
fn replacement_owner_preserves_pending_send_completion_correlation() {
    let key = room_key();
    let current_owner = SharedSendCompletionCoordinator::default();
    let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
    let mut registration = SendCompletionRegistration::begin(
        Arc::clone(&current_owner),
        ingress.clone(),
        key.clone(),
        "client-owner-handoff-txn".to_owned(),
        None,
        fake_rid(773),
        true,
    );
    registration.activate();
    registration.bind("sdk-owner-handoff-txn".to_owned());

    let replacement_owner = Arc::clone(&current_owner);
    drop(registration);
    drop(current_owner);
    apply_send_completion_observation_and_handoff(
        &replacement_owner,
        &ingress,
        key.room_id(),
        SendCompletionObservation::Sent {
            sdk_transaction_id: "sdk-owner-handoff-txn".to_owned(),
            event_id: "$event-owner-handoff:test".to_owned(),
        },
    );
    assert!(matches!(
        terminal_rx.try_recv().expect("replacement completion").completion,
        Some(TimelineSendCompletionDelivery {
            request_id,
            transaction_id,
            event_id,
            ..
        }) if request_id == fake_rid(773)
            && transaction_id == "client-owner-handoff-txn"
            && event_id == "$event-owner-handoff:test"
    ));
}

#[test]
fn duplicate_sent_event_after_completion_is_idempotent() {
    let key = room_key();
    let coordinator = SharedSendCompletionCoordinator::default();
    let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
    let mut registration = SendCompletionRegistration::begin(
        Arc::clone(&coordinator),
        ingress.clone(),
        key.clone(),
        "client-duplicate-txn".to_owned(),
        None,
        fake_rid(770),
        true,
    );
    registration.activate();
    registration.bind("sdk-duplicate-txn".to_owned());
    for _ in 0..2 {
        apply_send_completion_observation_and_handoff(
            &coordinator,
            &ingress,
            key.room_id(),
            SendCompletionObservation::Sent {
                sdk_transaction_id: "sdk-duplicate-txn".to_owned(),
                event_id: "$event-duplicate:test".to_owned(),
            },
        );
    }

    assert!(
        terminal_rx
            .try_recv()
            .expect("first completion")
            .completion
            .is_some()
    );
    assert!(
        terminal_rx.try_recv().is_err(),
        "an overlapping observer must not emit twice"
    );
}

#[test]
fn sent_event_before_pending_race_remains_idempotent_after_settlement() {
    let key = room_key();
    let coordinator = SharedSendCompletionCoordinator::default();
    let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
    let mut registration = SendCompletionRegistration::begin(
        Arc::clone(&coordinator),
        ingress.clone(),
        key.clone(),
        "client-early-duplicate-txn".to_owned(),
        None,
        fake_rid(771),
        true,
    );
    registration.activate();
    apply_send_completion_observation_and_handoff(
        &coordinator,
        &ingress,
        key.room_id(),
        SendCompletionObservation::Sent {
            sdk_transaction_id: "sdk-early-duplicate-txn".to_owned(),
            event_id: "$event-early-duplicate:test".to_owned(),
        },
    );
    registration.bind("sdk-early-duplicate-txn".to_owned());
    assert!(
        terminal_rx
            .try_recv()
            .expect("early completion")
            .completion
            .is_some()
    );

    apply_send_completion_observation_and_handoff(
        &coordinator,
        &ingress,
        key.room_id(),
        SendCompletionObservation::Sent {
            sdk_transaction_id: "sdk-early-duplicate-txn".to_owned(),
            event_id: "$event-early-duplicate:test".to_owned(),
        },
    );
    assert!(terminal_rx.try_recv().is_err());
}

#[test]
fn cancelled_completion_is_tombstoned_against_late_sent_event() {
    let key = room_key();
    let coordinator = SharedSendCompletionCoordinator::default();
    let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
    let mut registration = SendCompletionRegistration::begin(
        Arc::clone(&coordinator),
        ingress.clone(),
        key.clone(),
        "client-cancelled-txn".to_owned(),
        None,
        fake_rid(772),
        true,
    );
    registration.activate();
    registration.bind("sdk-cancelled-txn".to_owned());
    apply_send_completion_observation_and_handoff(
        &coordinator,
        &ingress,
        key.room_id(),
        SendCompletionObservation::Cancelled {
            sdk_transaction_id: "sdk-cancelled-txn".to_owned(),
        },
    );
    assert!(
        terminal_rx
            .try_recv()
            .expect("cancel terminal")
            .action
            .is_some()
    );

    apply_send_completion_observation_and_handoff(
        &coordinator,
        &ingress,
        key.room_id(),
        SendCompletionObservation::Sent {
            sdk_transaction_id: "sdk-cancelled-txn".to_owned(),
            event_id: "$late-event:test".to_owned(),
        },
    );
    assert!(terminal_rx.try_recv().is_err());
    assert!(
        coordinator
            .lock()
            .expect("send completion coordinator")
            .settled_send_tombstones
            .contains(&SendCorrelationKey {
                room_id: key.room_id().to_owned(),
                sdk_transaction_id: "sdk-cancelled-txn".to_owned(),
            })
    );
}

#[test]
fn unmatched_early_send_completions_survive_beyond_tombstone_history_bound() {
    let key = room_key();
    let coordinator = SharedSendCompletionCoordinator::default();
    let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
    let observed = MAX_SETTLED_SEND_TOMBSTONES + 64;
    let mut registrations = Vec::with_capacity(observed);
    for index in 0..observed {
        let mut registration = SendCompletionRegistration::begin(
            Arc::clone(&coordinator),
            ingress.clone(),
            key.clone(),
            format!("client-early-{index}"),
            None,
            fake_rid(900 + index as u64),
            true,
        );
        registration.activate();
        registrations.push(registration);
    }
    for index in 0..observed {
        apply_send_completion_observation_and_handoff(
            &coordinator,
            &ingress,
            key.room_id(),
            SendCompletionObservation::Sent {
                sdk_transaction_id: format!("sdk-early-{index}"),
                event_id: format!("$event-early-{index}:test"),
            },
        );
    }
    assert_eq!(
        coordinator
            .lock()
            .expect("send completion coordinator")
            .unmatched_terminals
            .len(),
        observed,
        "active unmatched correlations are not tombstone history and must not be evicted"
    );

    registrations[0].bind("sdk-early-0".to_owned());
    assert!(matches!(
        terminal_rx.try_recv().expect("oldest early completion").completion,
        Some(TimelineSendCompletionDelivery {
            request_id,
            event_id,
            ..
        }) if request_id == fake_rid(900) && event_id == "$event-early-0:test"
    ));
}

#[test]
fn settled_send_tombstones_are_bounded() {
    let key = room_key();
    let coordinator = SharedSendCompletionCoordinator::default();
    let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
    for index in 0..=MAX_SETTLED_SEND_TOMBSTONES {
        let sdk_transaction_id = format!("sdk-bounded-{index}");
        let mut registration = SendCompletionRegistration::begin(
            Arc::clone(&coordinator),
            ingress.clone(),
            key.clone(),
            format!("client-bounded-{index}"),
            None,
            fake_rid(1200 + index as u64),
            true,
        );
        registration.activate();
        registration.bind(sdk_transaction_id.clone());
        apply_send_completion_observation_and_handoff(
            &coordinator,
            &ingress,
            key.room_id(),
            SendCompletionObservation::Sent {
                sdk_transaction_id,
                event_id: format!("$event-bounded-{index}:test"),
            },
        );
        assert!(
            terminal_rx
                .try_recv()
                .expect("bounded completion")
                .completion
                .is_some()
        );
    }

    let first = SendCorrelationKey {
        room_id: key.room_id().to_owned(),
        sdk_transaction_id: "sdk-bounded-0".to_owned(),
    };
    let newest = SendCorrelationKey {
        room_id: key.room_id().to_owned(),
        sdk_transaction_id: format!("sdk-bounded-{MAX_SETTLED_SEND_TOMBSTONES}"),
    };
    let coordinator_guard = coordinator.lock().expect("send completion coordinator");
    assert_eq!(
        coordinator_guard.settled_send_tombstones.len(),
        MAX_SETTLED_SEND_TOMBSTONES
    );
    assert!(!coordinator_guard.settled_send_tombstones.contains(&first));
    assert!(coordinator_guard.settled_send_tombstones.contains(&newest));
    drop(coordinator_guard);

    apply_send_completion_observation_and_handoff(
        &coordinator,
        &ingress,
        key.room_id(),
        SendCompletionObservation::Sent {
            sdk_transaction_id: newest.sdk_transaction_id,
            event_id: "$duplicate:test".to_owned(),
        },
    );
    assert!(terminal_rx.try_recv().is_err());
}

#[test]
fn send_completion_coordinator_preserves_submission_id_for_terminal_paths() {
    let key = room_key();
    let coordinator = SharedSendCompletionCoordinator::default();
    let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
    let submission_id = SubmissionId::new("submission-terminal");
    let mut registration = SendCompletionRegistration::begin(
        Arc::clone(&coordinator),
        ingress.clone(),
        key.clone(),
        "client-submission-terminal".to_owned(),
        Some(submission_id.clone()),
        fake_rid(7400),
        true,
    );
    registration.activate();
    registration.bind("sdk-submission-terminal".to_owned());

    apply_send_completion_observation_and_handoff(
        &coordinator,
        &ingress,
        key.room_id(),
        SendCompletionObservation::SendError {
            sdk_transaction_id: "sdk-submission-terminal".to_owned(),
            diagnostic: SendFailureDiagnostic {
                reason: "http",
                recoverable: true,
            },
        },
    );
    let failure = terminal_rx.try_recv().expect("submission send error");
    assert!(matches!(
        failure.action,
        Some(AppAction::ComposerSubmissionSettled {
            submission_id: found,
            ..
        }) if found == submission_id
    ));
    apply_send_completion_observation_and_handoff(
        &coordinator,
        &ingress,
        key.room_id(),
        SendCompletionObservation::Cancelled {
            sdk_transaction_id: "sdk-submission-terminal".to_owned(),
        },
    );
    let cancelled = terminal_rx.try_recv().expect("submission cancellation");
    assert!(cancelled.action.is_none());
    assert!(cancelled.completion.is_none());
}

#[test]
fn media_pending_send_does_not_settle_text_composer() {
    let key = room_key();
    let coordinator = SharedSendCompletionCoordinator::default();
    let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
    let mut registration = SendCompletionRegistration::begin(
        Arc::clone(&coordinator),
        ingress.clone(),
        key.clone(),
        "client-media-txn".to_owned(),
        None,
        fake_rid(78),
        false,
    );
    registration.activate();
    registration.bind("sdk-media-txn".to_owned());
    apply_send_completion_observation_and_handoff(
        &coordinator,
        &ingress,
        key.room_id(),
        SendCompletionObservation::Sent {
            sdk_transaction_id: "sdk-media-txn".to_owned(),
            event_id: "$event-media:test".to_owned(),
        },
    );
    let terminal = terminal_rx.try_recv().expect("media completion");
    assert!(terminal.action.is_none());
    assert!(terminal.completion.is_some());
}

#[test]
fn timeline_send_error_classifies_not_joined_as_forbidden() {
    let error = matrix_sdk_ui::timeline::Error::SendQueueError(
        matrix_sdk::send_queue::RoomSendQueueError::RoomNotJoined,
    );

    assert_eq!(
        classify_timeline_send_error(&error),
        TimelineFailureKind::Forbidden
    );
}

#[test]
fn same_room_thread_media_progress_does_not_borrow_room_request_correlation() {
    let account = AccountKey("@media-progress:test".to_owned());
    let room_key = TimelineKey::room(account.clone(), "!media-progress:test");
    let thread_key = TimelineKey {
        account_key: account,
        kind: TimelineKind::Thread {
            room_id: "!media-progress:test".to_owned(),
            root_event_id: "$media-root:test".to_owned(),
        },
    };
    let coordinator = SharedSendCompletionCoordinator::default();
    let (ingress, _terminal_rx) = TimelineSendTerminalIngress::channel();
    let mut registration = SendCompletionRegistration::begin(
        Arc::clone(&coordinator),
        ingress,
        room_key.clone(),
        "client-media-progress".to_owned(),
        None,
        fake_rid(7424),
        false,
    );
    registration.activate();
    registration.bind("sdk-media-progress".to_owned());

    assert_eq!(
        media_upload_progress_identity(&coordinator, &room_key, "sdk-media-progress"),
        ("client-media-progress".to_owned(), Some(fake_rid(7424)))
    );
    assert_eq!(
        media_upload_progress_identity(&coordinator, &thread_key, "sdk-media-progress"),
        ("sdk-media-progress".to_owned(), None),
        "same-room thread presentation must not borrow room request correlation"
    );
}
