use super::super::test_source::item_body;
use futures_util::{FutureExt, StreamExt};

use std::collections::{HashMap, HashSet};

use std::sync::Arc;

use std::time::Duration;

use koushi_sdk::MatrixClientSession;
use koushi_sdk::MatrixUserProfile;

use koushi_state::UserProfile;
use koushi_state::{AppAction, LiveEventReceipts, LiveReadReceipt};

use matrix_sdk_ui::timeline::TimelineFocus;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::executor;
use koushi_protocol::command::TimelineCommand;
use koushi_protocol::event::{CoreEvent, LiveSignalsEvent, TimelineReadStateSync};
use koushi_protocol::failure::{CoreFailure, ReadStateFailureKind, TimelineFailureKind};
#[cfg(any(test, feature = "test-hooks"))]
use koushi_protocol::ids::AccountKey;
use koushi_protocol::ids::{TimelineKey, TimelineKind};

use crate::read_state::{
    ReadPersistenceSnapshot, ReadStateEngine, ReadStateKey, ReadTarget, ReadWaiterId,
};

use koushi_diagnostics::DiagnosticValue;
use koushi_state::{SessionInfo, SessionState};

use super::super::actor::{TimelineActorControl, TimelineActorHandle, TimelinePositionIndex};
use super::super::diagnostics::{
    FullyReadReceiptContext, private_read_receipt_event_id_for_fully_read,
};
use super::super::item_projection::{
    build_live_receipt_observation_actions, emit_live_receipt_observation_actions,
    live_event_receipts_from_endpoint_changes, live_receipt_observation_actions_from_sdk_receipts,
};
use super::super::manager::TimelineMessage;
use super::super::navigation::TimelineActorGenerationGate;

use super::super::relay::koushi_timeline_builder;
use super::{
    MAX_CONCURRENT_READ_WRITES, ReadActorApplyKind, ReadCommandKind, ReadNetworkFailure,
    ReadNetworkOutcome, ReadPersistenceIngress, ReadRetrySource, ReadWorkerCompletion,
    ReadWorkerSupervisor, read_retry_delay_for_attempt,
};

use super::super::test_support::{
    fake_rid, live_tail_test_manager, room_key, test_timeline_actor_handle,
};

#[test]
fn private_read_receipt_target_advances_to_hidden_edit_notification() {
    let target = private_read_receipt_event_id_for_fully_read(FullyReadReceiptContext {
        visible_event_id: "$visible:test",
        latest_event_id: Some("$latest-edit:test"),
        latest_event_relation_type: Some("m.replace"),
        unread_messages: 0,
        notification_count: 1,
    });

    assert_eq!(target, "$latest-edit:test");

    for context in [
        FullyReadReceiptContext {
            visible_event_id: "$visible:test",
            latest_event_id: Some("$latest-message:test"),
            latest_event_relation_type: None,
            unread_messages: 0,
            notification_count: 1,
        },
        FullyReadReceiptContext {
            visible_event_id: "$visible:test",
            latest_event_id: Some("$latest-edit:test"),
            latest_event_relation_type: Some("m.replace"),
            unread_messages: 1,
            notification_count: 1,
        },
        FullyReadReceiptContext {
            visible_event_id: "$visible:test",
            latest_event_id: Some("$latest-edit:test"),
            latest_event_relation_type: Some("m.replace"),
            unread_messages: 0,
            notification_count: 0,
        },
        FullyReadReceiptContext {
            visible_event_id: "$visible:test",
            latest_event_id: None,
            latest_event_relation_type: Some("m.replace"),
            unread_messages: 0,
            notification_count: 1,
        },
    ] {
        assert_eq!(
            private_read_receipt_event_id_for_fully_read(context),
            "$visible:test"
        );
    }
}

#[test]
fn private_read_receipt_target_advances_to_hidden_thread_notification() {
    let target = private_read_receipt_event_id_for_fully_read(FullyReadReceiptContext {
        visible_event_id: "$visible:test",
        latest_event_id: Some("$latest-thread:test"),
        latest_event_relation_type: Some("m.thread"),
        unread_messages: 0,
        notification_count: 1,
    });

    assert_eq!(target, "$latest-thread:test");
}

fn restored_read_snapshot(key: ReadStateKey, event_id: &str) -> ReadPersistenceSnapshot {
    let mut engine = ReadStateEngine::new(7);
    engine.admit(
        7,
        key,
        ReadTarget::new(event_id.to_owned()),
        ReadWaiterId::new(1),
    );
    engine.persistence_snapshot()
}

fn restored_public_read_snapshot(room_id: &str, event_id: &str) -> ReadPersistenceSnapshot {
    restored_read_snapshot(
        ReadStateKey::PublicUnthreaded {
            room_id: room_id.to_owned(),
        },
        event_id,
    )
}

#[tokio::test]
async fn twenty_read_keys_never_exceed_four_concurrent_writes() {
    let (network_tx, mut network_rx) = mpsc::unbounded_channel();
    let mut supervisor = ReadWorkerSupervisor::synthetic(network_tx, Duration::from_secs(30));
    let keys = (0..20)
        .map(|index| ReadStateKey::PublicUnthreaded {
            room_id: format!("!dispatcher-{index}:example.invalid"),
        })
        .collect::<Vec<_>>();
    for (index, key) in keys.iter().enumerate() {
        supervisor.state.admit_background(
            1,
            key.clone(),
            ReadTarget::new(format!("$dispatcher-{index}:example.invalid")),
        );
        supervisor.enqueue_key(key.clone());
    }
    supervisor.dispatch_ready_reads();

    let mut started = Vec::new();
    for expected in 0..keys.len() {
        assert!(supervisor.state.active_operation_count() <= MAX_CONCURRENT_READ_WRITES);
        let request = next_synthetic_request(&mut supervisor, &mut network_rx).await;
        started.push(request.operation.target().event_id().to_owned());
        let operation = request.operation.clone();
        request
            .response
            .send(Ok(()))
            .expect("release dispatcher slot");
        let _completion = supervisor.tasks.next().await.expect("write completion");
        supervisor.state.complete(
            operation.key(),
            operation.fence(),
            ReadNetworkOutcome::Succeeded,
        );
        supervisor.dispatch_ready_reads();
        if expected < keys.len() - 1 {
            assert!(supervisor.state.active_operation_count() <= MAX_CONCURRENT_READ_WRITES);
        }
    }

    assert_eq!(started.len(), 20);
    assert_eq!(supervisor.state.active_operation_count(), 0);
    assert_eq!(started[0], "$dispatcher-0:example.invalid");
    assert_eq!(started[19], "$dispatcher-19:example.invalid");
}

#[test]
fn synchronous_dispatch_failures_are_all_retained_for_settlement() {
    let (network_tx, network_rx) = mpsc::unbounded_channel();
    drop(network_rx);
    let mut supervisor = ReadWorkerSupervisor::synthetic(network_tx, Duration::from_secs(30));
    supervisor.network = None;
    for index in 0..20 {
        let key = ReadStateKey::PublicUnthreaded {
            room_id: format!("!dispatch-failure-{index}:example.invalid"),
        };
        supervisor.state.admit_background(
            1,
            key.clone(),
            ReadTarget::new(format!("$dispatch-failure-{index}:example.invalid")),
        );
        supervisor.enqueue_key(key);
    }

    supervisor.dispatch_ready_reads();

    assert_eq!(supervisor.take_dispatch_failures().len(), 20);
    assert_eq!(
        supervisor.state.persistence_snapshot().candidate_count(),
        20
    );
}

#[tokio::test(start_paused = true)]
async fn fifo_peers_start_before_a_failed_key_retries() {
    let (network_tx, mut network_rx) = mpsc::unbounded_channel();
    let mut supervisor = ReadWorkerSupervisor::synthetic_with_retry(
        network_tx,
        Duration::from_secs(30),
        Duration::from_secs(1),
        Duration::from_secs(60),
    );
    let keys = (0..6)
        .map(|index| ReadStateKey::PublicUnthreaded {
            room_id: format!("!fair-{index}:example.invalid"),
        })
        .collect::<Vec<_>>();
    for (index, key) in keys.iter().enumerate() {
        supervisor.state.admit_background(
            1,
            key.clone(),
            ReadTarget::new(format!("$fair-{index}:example.invalid")),
        );
        supervisor.enqueue_key(key.clone());
    }
    supervisor.dispatch_ready_reads();

    let mut initial = Vec::new();
    for _ in 0..4 {
        initial.push(next_synthetic_request(&mut supervisor, &mut network_rx).await);
    }
    let failed_index = initial
        .iter()
        .position(|request| request.operation.target().event_id() == "$fair-0:example.invalid")
        .expect("first FIFO key is active");
    let failed_request = initial.remove(failed_index);
    let failed = failed_request.operation.clone();
    failed_request
        .response
        .send(Err(()))
        .expect("fail first FIFO request");
    let _completion = supervisor.tasks.next().await.expect("failed completion");
    supervisor.state.complete(
        failed.key(),
        failed.fence(),
        ReadNetworkOutcome::Failed(ReadNetworkFailure::new(ReadStateFailureKind::Sdk)),
    );
    supervisor.schedule_retry(&keys[0]);
    supervisor.dispatch_ready_reads();

    let peer = next_synthetic_request(&mut supervisor, &mut network_rx).await;
    assert_eq!(
        peer.operation.target().event_id(),
        "$fair-4:example.invalid"
    );
    let peer_operation = peer.operation.clone();
    peer.response.send(Ok(())).expect("complete queued peer");
    let _completion = supervisor.tasks.next().await.expect("peer completion");
    supervisor.state.complete(
        peer_operation.key(),
        peer_operation.fence(),
        ReadNetworkOutcome::Succeeded,
    );
    supervisor.dispatch_ready_reads();

    let peer = next_synthetic_request(&mut supervisor, &mut network_rx).await;
    assert_eq!(
        peer.operation.target().event_id(),
        "$fair-5:example.invalid"
    );
    let peer_operation = peer.operation.clone();
    peer.response
        .send(Ok(()))
        .expect("complete second queued peer");
    let _completion = supervisor
        .tasks
        .next()
        .await
        .expect("second peer completion");
    supervisor.state.complete(
        peer_operation.key(),
        peer_operation.fence(),
        ReadNetworkOutcome::Succeeded,
    );
    supervisor.dispatch_ready_reads();

    for peer in initial {
        let peer_operation = peer.operation.clone();
        peer.response.send(Ok(())).expect("complete initial peer");
        let _completion = supervisor
            .tasks
            .next()
            .await
            .expect("initial peer completion");
        supervisor.state.complete(
            peer_operation.key(),
            peer_operation.fence(),
            ReadNetworkOutcome::Succeeded,
        );
        supervisor.dispatch_ready_reads();
    }

    assert!(supervisor.retry_tasks.next().now_or_never().is_none());
    tokio::time::advance(Duration::from_secs(1)).await;
    let retry = supervisor.retry_tasks.next().await.expect("due FIFO retry");
    let ReadWorkerCompletion::RetryWake {
        key,
        generation,
        cancelled: false,
    } = retry
    else {
        panic!("expected due retry wake");
    };
    assert!(supervisor.accept_retry_wake(&key, generation));
    supervisor.enqueue_key(key);
    supervisor.dispatch_ready_reads();
    let retried = next_synthetic_request(&mut supervisor, &mut network_rx).await;
    assert_eq!(
        retried.operation.target().event_id(),
        "$fair-0:example.invalid"
    );
}

#[tokio::test(start_paused = true)]
async fn rate_limit_retry_after_is_the_exact_dispatch_delay() {
    let key = ReadStateKey::PublicUnthreaded {
        room_id: "!retry-after:example.invalid".to_owned(),
    };
    let (network_tx, mut network_rx) = mpsc::unbounded_channel();
    let mut supervisor = ReadWorkerSupervisor::synthetic_with_retry(
        network_tx,
        Duration::from_secs(30),
        Duration::from_secs(1),
        Duration::from_secs(60),
    );
    supervisor.state.admit_background(
        1,
        key.clone(),
        ReadTarget::new("$retry-after:example.invalid".to_owned()),
    );
    supervisor.enqueue_key(key.clone());
    supervisor.dispatch_ready_reads();
    let request = next_synthetic_request(&mut supervisor, &mut network_rx).await;
    let operation = request.operation.clone();
    request
        .response
        .send(Err(()))
        .expect("fail retry-after request");
    let _completion = supervisor
        .tasks
        .next()
        .await
        .expect("retry-after completion");
    supervisor.state.complete(
        operation.key(),
        operation.fence(),
        ReadNetworkOutcome::Failed(ReadNetworkFailure::with_retry_after(
            ReadStateFailureKind::RateLimited,
            Duration::from_secs(7),
        )),
    );
    supervisor.schedule_retry(&key);

    tokio::time::advance(Duration::from_secs(6) + Duration::from_millis(999)).await;
    assert!(supervisor.retry_tasks.next().now_or_never().is_none());
    tokio::time::advance(Duration::from_millis(1)).await;
    let retry = supervisor
        .retry_tasks
        .next()
        .await
        .expect("exact retry-after wake");
    let ReadWorkerCompletion::RetryWake {
        key: retry_key,
        generation,
        cancelled: false,
    } = retry
    else {
        panic!("expected retry-after wake");
    };
    assert_eq!(retry_key, key);
    assert!(supervisor.accept_retry_wake(&key, generation));
    supervisor.enqueue_key(key);
    supervisor.dispatch_ready_reads();
    let retry = next_synthetic_request(&mut supervisor, &mut network_rx).await;
    assert_eq!(
        retry.operation.target().event_id(),
        "$retry-after:example.invalid"
    );
}

#[tokio::test]
async fn cancellation_keeps_a_dispatch_slot_until_cancelled_completion() {
    let (network_tx, mut network_rx) = mpsc::unbounded_channel();
    let mut supervisor = ReadWorkerSupervisor::synthetic(network_tx, Duration::from_secs(30));
    let keys = (0..5)
        .map(|index| ReadStateKey::PublicUnthreaded {
            room_id: format!("!cancel-slot-{index}:example.invalid"),
        })
        .collect::<Vec<_>>();
    for (index, key) in keys.iter().enumerate() {
        supervisor.state.admit_background(
            1,
            key.clone(),
            ReadTarget::new(format!("$cancel-slot-{index}:example.invalid")),
        );
        supervisor.enqueue_key(key.clone());
    }
    supervisor.dispatch_ready_reads();
    let mut first_four = Vec::new();
    for _ in 0..4 {
        first_four.push(next_synthetic_request(&mut supervisor, &mut network_rx).await);
    }
    let cancelled = first_four[0].operation.clone();
    supervisor.cancel(cancelled.fence());
    supervisor.dispatch_ready_reads();
    assert_eq!(supervisor.state.active_operation_count(), 4);
    assert!(network_rx.try_recv().is_err());

    let cancellation = supervisor.tasks.next().await.expect("cancelled completion");
    assert!(matches!(
        cancellation,
        ReadWorkerCompletion::Cancelled { ref operation }
            if operation.fence() == cancelled.fence()
    ));
    supervisor
        .state
        .complete_cancelled(&keys[0], cancelled.fence());
    supervisor.dispatch_ready_reads();
    let next = next_synthetic_request(&mut supervisor, &mut network_rx).await;
    assert_eq!(
        next.operation.target().event_id(),
        "$cancel-slot-4:example.invalid"
    );
}

#[tokio::test]
async fn local_read_correlation_projects_lifecycle_and_fences_stale_b_before_new_c() {
    let key = room_key();
    let (actor_handle, mut control_rx) =
        actor_handle_with_positions(7, [("$local-b:test", 2), ("$local-c:test", 3)]);
    let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
    let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
    manager.read_workers =
        ReadWorkerSupervisor::synthetic(read_network_tx, Duration::from_secs(30));

    manager
        .handle_local_read_boundary_observed(
            key.clone(),
            7,
            ReadTarget::with_position(
                "$local-b:test".to_owned(),
                crate::read_state::ReadPositionEvidence {
                    generation: 7_u128 << 64,
                    rank: 2,
                },
            ),
        )
        .await;
    assert_eq!(manager.read_workers.local_read_correlation_count(), 1);
    assert_eq!(
        manager.read_workers.local_read_sync(
            manager
                .read_workers
                .local_read_correlations
                .get(&key)
                .expect("local B correlation")
        ),
        TimelineReadStateSync::Pending
    );
    let _public_b = next_synthetic_request(&mut manager.read_workers, &mut read_network_rx).await;
    let fully_b = next_synthetic_request(&mut manager.read_workers, &mut read_network_rx).await;

    manager
        .handle_local_read_boundary_observed(
            key.clone(),
            7,
            ReadTarget::with_position(
                "$local-c:test".to_owned(),
                crate::read_state::ReadPositionEvidence {
                    generation: 7_u128 << 64,
                    rank: 3,
                },
            ),
        )
        .await;
    let stale_operation = fully_b.operation.clone();
    manager
        .handle_read_worker_completion(ReadWorkerCompletion::Network {
            operation: stale_operation,
            outcome: ReadNetworkOutcome::Succeeded,
        })
        .await;
    let correlation = manager
        .read_workers
        .local_read_correlations
        .get(&key)
        .expect("new C correlation");
    assert_eq!(correlation.local_target.event_id(), "$local-c:test");
    assert_eq!(correlation.server_confirmed_read_event_id, None);
    assert!(
        manager
            .read_workers
            .state
            .has_candidate(fully_b.operation.key(), "$local-c:test")
    );
    assert!(
        manager
            .read_workers
            .state
            .active_operation(fully_b.operation.key())
            .is_some(),
        "stale completion must refill its dispatcher slot with desired C"
    );
    let replacement = loop {
        tokio::select! {
            request = read_network_rx.recv() => {
                break request.expect("replacement C synthetic request");
            }
            completion = manager.read_workers.tasks.next() => {
                manager
                    .handle_read_worker_completion(
                        completion.expect("cancelled B completion before replacement C"),
                    )
                    .await;
            }
        }
    };
    assert_eq!(replacement.operation.target().event_id(), "$local-c:test");

    while let Ok(control) = control_rx.try_recv() {
        assert!(
            !matches!(control, TimelineActorControl::ApplyReadSuccess { .. }),
            "stale B success must not reach the actor after desired C replaces it"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn local_read_correlation_reports_failed_then_synced_and_capacity_truthfully() {
    let key = room_key();
    let (actor_handle, mut control_rx) = actor_handle_with_positions(7, [("$local-b:test", 2)]);
    let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
    let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
    manager.read_workers = ReadWorkerSupervisor::synthetic_with_retry(
        read_network_tx,
        Duration::from_secs(30),
        Duration::from_secs(1),
        Duration::from_secs(60),
    );
    manager
        .handle_local_read_boundary_observed(
            key.clone(),
            7,
            ReadTarget::with_position(
                "$local-b:test".to_owned(),
                crate::read_state::ReadPositionEvidence {
                    generation: 7_u128 << 64,
                    rank: 2,
                },
            ),
        )
        .await;
    let mut failed_requests = Vec::new();
    for _ in 0..2 {
        failed_requests
            .push(next_synthetic_request(&mut manager.read_workers, &mut read_network_rx).await);
    }
    for request in failed_requests {
        let operation = request.operation.clone();
        request.response.send(Err(())).expect("fail local read");
        let _completion = manager
            .read_workers
            .tasks
            .next()
            .await
            .expect("failed read");
        manager
            .handle_read_worker_completion(ReadWorkerCompletion::Network {
                operation,
                outcome: ReadNetworkOutcome::Failed(ReadNetworkFailure::new(
                    ReadStateFailureKind::Transport,
                )),
            })
            .await;
    }
    let correlation = manager
        .read_workers
        .local_read_correlations
        .get(&key)
        .expect("failed local correlation");
    assert_eq!(
        manager.read_workers.local_read_sync(correlation),
        TimelineReadStateSync::Failed {
            kind: ReadStateFailureKind::Transport
        }
    );

    assert!(
        manager
            .read_workers
            .retry_tasks
            .next()
            .now_or_never()
            .is_none()
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    for _ in 0..2 {
        let wake = manager
            .read_workers
            .retry_tasks
            .next()
            .await
            .expect("local retry wake");
        manager.handle_read_worker_completion(wake).await;
    }
    let mut successful_requests = Vec::new();
    for _ in 0..2 {
        successful_requests
            .push(next_synthetic_request(&mut manager.read_workers, &mut read_network_rx).await);
    }
    for request in successful_requests {
        let operation = request.operation.clone();
        request
            .response
            .send(Ok(()))
            .expect("successful local read");
        let completion = manager
            .read_workers
            .tasks
            .next()
            .await
            .expect("retry completion");
        assert_eq!(completion.fence(), Some(operation.fence()));
        assert_eq!(
            manager.read_workers.state.active_operation(operation.key()),
            Some(operation.fence())
        );
        if matches!(
            operation.key(),
            ReadStateKey::FullyReadAndPrivateUnthreaded { .. }
        ) {
            assert_eq!(
                manager.read_timeline_key_for_operation(&operation),
                Some(key.clone())
            );
        }
        manager.handle_read_worker_completion(completion).await;
        if matches!(
            operation.key(),
            ReadStateKey::FullyReadAndPrivateUnthreaded { .. }
        ) {
            let acknowledge = async {
                loop {
                    match control_rx.recv().await.expect("fully-read apply control") {
                        TimelineActorControl::ApplyReadSuccess { acknowledged, .. } => {
                            acknowledged
                                .send(true)
                                .expect("acknowledge fully-read apply");
                            break;
                        }
                        TimelineActorControl::ReadStateProjection { .. } => {}
                        TimelineActorControl::ReadStatePolicyChanged { .. } => {}
                        TimelineActorControl::DisplayPolicyChanged { .. } => {}
                        TimelineActorControl::ReplayInitialItems { .. }
                        | TimelineActorControl::StartLiveTailRefresh { .. }
                        | TimelineActorControl::CancelLiveTailNetwork { .. }
                        | TimelineActorControl::BeginGapRepairDemand
                        | TimelineActorControl::EndGapRepairDemand => {}
                    }
                }
            };
            let (apply_completion, ()) =
                tokio::join!(manager.read_workers.tasks.next(), acknowledge);
            manager
                .handle_read_worker_completion(
                    apply_completion.expect("fully-read apply completion"),
                )
                .await;
        }
    }
    let correlation = manager
        .read_workers
        .local_read_correlations
        .get(&key)
        .expect("synced local correlation");
    assert_eq!(
        manager.read_workers.local_read_sync(correlation),
        TimelineReadStateSync::Synced
    );
    assert_eq!(
        correlation.server_confirmed_read_event_id.as_deref(),
        Some("$local-b:test")
    );

    let capacity_key = TimelineKey::room(
        AccountKey("@capacity:example.invalid".to_owned()),
        "!capacity-room:example.invalid",
    );
    let (capacity_actor, _capacity_controls) =
        actor_handle_with_positions(8, [("$capacity:test", 1)]);
    let mut capacity_manager =
        live_tail_test_manager(HashMap::from([(capacity_key.clone(), capacity_actor)]));
    let (capacity_tx, _capacity_rx) = mpsc::unbounded_channel();
    capacity_manager.read_workers =
        ReadWorkerSupervisor::synthetic(capacity_tx, Duration::from_secs(30));
    for index in 0..crate::read_state::READ_STATE_OUTBOX_ENTRY_LIMIT {
        capacity_manager.read_workers.state.admit_background(
            1,
            ReadStateKey::PublicUnthreaded {
                room_id: format!("!capacity-fill-{index}:example.invalid"),
            },
            ReadTarget::new(format!("$capacity-fill-{index}:example.invalid")),
        );
    }
    capacity_manager
        .handle_local_read_boundary_observed(
            capacity_key.clone(),
            8,
            ReadTarget::with_position(
                "$capacity:test".to_owned(),
                crate::read_state::ReadPositionEvidence {
                    generation: 8_u128 << 64,
                    rank: 1,
                },
            ),
        )
        .await;
    let capacity_correlation = capacity_manager
        .read_workers
        .local_read_correlations
        .get(&capacity_key)
        .expect("capacity admission keeps correlation");
    assert_eq!(
        capacity_manager.read_workers.local_read_correlation_count(),
        1
    );
    assert_eq!(
        capacity_manager
            .read_workers
            .local_read_sync(capacity_correlation),
        TimelineReadStateSync::Failed {
            kind: ReadStateFailureKind::Capacity
        }
    );
}

#[tokio::test]
async fn thread_read_policy_toggle_preserves_local_correlation_and_not_requested_state() {
    let room_id = "!policy-room:example.invalid";
    let key = TimelineKey {
        account_key: AccountKey("@policy:example.invalid".to_owned()),
        kind: TimelineKind::Thread {
            room_id: room_id.to_owned(),
            root_event_id: "$policy-root:example.invalid".to_owned(),
        },
    };
    let (actor_handle, _control_rx) = actor_handle_with_positions(9, [("$policy:test", 4)]);
    let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
    let (read_network_tx, _read_network_rx) = mpsc::unbounded_channel();
    manager.read_workers =
        ReadWorkerSupervisor::synthetic(read_network_tx, Duration::from_secs(30));
    let (persistence, mut persistence_rx) = ReadPersistenceIngress::channel();
    manager.read_workers.persistence = Some(persistence);
    manager
        .handle_local_read_boundary_observed(
            key.clone(),
            9,
            ReadTarget::with_position(
                "$policy:test".to_owned(),
                crate::read_state::ReadPositionEvidence {
                    generation: 9_u128 << 64,
                    rank: 4,
                },
            ),
        )
        .await;
    assert_eq!(manager.read_workers.local_read_correlation_count(), 1);
    assert_eq!(
        manager.read_workers.local_read_sync(
            manager
                .read_workers
                .local_read_correlations
                .get(&key)
                .expect("thread policy correlation")
        ),
        TimelineReadStateSync::Pending
    );

    let _ = persistence_rx.borrow_and_update();
    manager.handle_read_state_policy_changed(1, false).await;
    persistence_rx
        .changed()
        .await
        .expect("privacy disable publishes the reduced outbox");
    let disabled_snapshot = persistence_rx
        .borrow_and_update()
        .as_ref()
        .expect("privacy disable persistence request")
        .snapshot()
        .clone();
    assert!(disabled_snapshot.is_empty());
    let (restored_network_tx, mut restored_network_rx) = mpsc::unbounded_channel();
    let (restored_persistence, _restored_persistence_rx) = ReadPersistenceIngress::channel();
    let mut restored = ReadWorkerSupervisor::synthetic_restored(
        restored_network_tx,
        disabled_snapshot,
        restored_persistence,
    );
    restored.send_read_receipts = false;
    restored.dispatch_ready_reads();
    assert!(restored_network_rx.try_recv().is_err());

    let stale_snapshot = restored_public_read_snapshot(room_id, "$stale-policy:test");
    let (stale_network_tx, mut stale_network_rx) = mpsc::unbounded_channel();
    let mut stale_supervisor =
        ReadWorkerSupervisor::synthetic(stale_network_tx, Duration::from_secs(30));
    stale_supervisor.state = ReadStateEngine::restore(1, stale_snapshot)
        .expect("stale privacy snapshot restores for defense-in-depth check");
    stale_supervisor.send_read_receipts = false;
    for read_key in stale_supervisor.desired_keys() {
        stale_supervisor.enqueue_key(read_key);
    }
    stale_supervisor.dispatch_ready_reads();
    assert!(stale_network_rx.try_recv().is_err());

    assert_eq!(
        manager.read_workers.local_read_sync(
            manager
                .read_workers
                .local_read_correlations
                .get(&key)
                .expect("disabled thread policy correlation")
        ),
        TimelineReadStateSync::NotRequested
    );
    assert_eq!(manager.read_workers.local_read_correlation_count(), 1);

    manager.handle_read_state_policy_changed(1, true).await;
    assert_eq!(
        manager.read_workers.local_read_sync(
            manager
                .read_workers
                .local_read_correlations
                .get(&key)
                .expect("re-enabled thread policy correlation")
        ),
        TimelineReadStateSync::Pending
    );
}

#[tokio::test]
async fn actor_retirement_retires_its_read_keys_and_persistence() {
    let key = room_key();
    let (actor_handle, _control_rx) = actor_handle_with_positions(10, [("$retired:test", 1)]);
    let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
    let (read_network_tx, _read_network_rx) = mpsc::unbounded_channel();
    manager.read_workers =
        ReadWorkerSupervisor::synthetic(read_network_tx, Duration::from_secs(30));
    manager
        .handle_local_read_boundary_observed(
            key.clone(),
            10,
            ReadTarget::with_position(
                "$retired:test".to_owned(),
                crate::read_state::ReadPositionEvidence {
                    generation: 10_u128 << 64,
                    rank: 1,
                },
            ),
        )
        .await;
    assert!(!manager.read_workers.state.persistence_snapshot().is_empty());

    manager.read_workers.remove_local_read_correlation(&key);

    assert_eq!(manager.read_workers.local_read_correlation_count(), 0);
    assert!(manager.read_workers.state.persistence_snapshot().is_empty());
    assert_eq!(manager.read_workers.state.active_operation_count(), 0);
}

async fn next_synthetic_request(
    supervisor: &mut ReadWorkerSupervisor,
    receiver: &mut mpsc::UnboundedReceiver<super::SyntheticReadNetworkRequest>,
) -> super::SyntheticReadNetworkRequest {
    let mut completion = Box::pin(supervisor.tasks.next());
    tokio::select! {
        request = receiver.recv() => request.expect("synthetic read request"),
        _ = &mut completion => panic!("synthetic worker completed before request was observed"),
    }
}

fn actor_handle_with_positions(
    actor_generation: u64,
    positions: impl IntoIterator<Item = (&'static str, u64)>,
) -> (TimelineActorHandle, mpsc::Receiver<TimelineActorControl>) {
    let (tx, _rx) = mpsc::channel(1);
    let (control_tx, control_rx) = mpsc::channel(32);
    let (_position_tx, position_rx) = watch::channel(Arc::new(TimelinePositionIndex {
        generation: u128::from(actor_generation) << 64,
        ranks: positions
            .into_iter()
            .map(|(event_id, rank)| (event_id.to_owned(), rank))
            .collect(),
    }));
    (
        TimelineActorHandle {
            tx,
            control_tx: Some(control_tx),
            thread_summary_projection:
                crate::timeline::actor::ThreadSummaryProjectionIngress::channel().0,
            position_rx: Some(position_rx),
            task: None,
            auxiliary_tasks: Vec::new(),
            subscription_generation: None,
            enqueue_context: None,
        },
        control_rx,
    )
}

#[tokio::test]
async fn restored_read_waits_for_authoritative_reconciliation_before_retrying() {
    let key = room_key();
    let read_key = ReadStateKey::PublicUnthreaded {
        room_id: key.room_id().to_owned(),
    };
    let (ordinary_tx, _ordinary_rx) = mpsc::channel(1);
    let (control_tx, mut control_rx) = mpsc::channel(8);
    let (_position_tx, position_rx) = watch::channel(Arc::new(TimelinePositionIndex {
        generation: u128::from(7_u64) << 64,
        ranks: HashMap::from([("$desired:test".to_owned(), 5)]),
    }));
    let actor_handle = TimelineActorHandle {
        tx: ordinary_tx,
        control_tx: Some(control_tx),
        thread_summary_projection: crate::timeline::actor::ThreadSummaryProjectionIngress::channel(
        )
        .0,
        position_rx: Some(position_rx),
        task: None,
        auxiliary_tasks: Vec::new(),
        subscription_generation: None,
        enqueue_context: None,
    };
    let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
    let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
    let (persistence, mut persistence_rx) = ReadPersistenceIngress::channel();
    manager.read_workers = ReadWorkerSupervisor::synthetic_restored(
        read_network_tx,
        restored_public_read_snapshot(key.room_id(), "$desired:test"),
        persistence,
    );

    manager
        .wake_all_desired_reads(ReadRetrySource::Reconnect)
        .await;
    assert!(manager.read_workers.tasks.is_empty());
    assert!(read_network_rx.try_recv().is_err());

    manager
        .handle_authoritative_read_state_observed(&key, 7, read_key, None)
        .await;
    assert!(matches!(
        control_rx.recv().await,
        Some(TimelineActorControl::ReadStateProjection {
            local_viewed_event_id: Some(event_id),
            server_confirmed_read_event_id: None,
            sync: TimelineReadStateSync::Pending,
        }) if event_id == "$desired:test"
    ));
    let responder = async {
        let retry = read_network_rx
            .recv()
            .await
            .expect("server-behind reconciliation starts retry");
        assert_eq!(retry.operation.target().event_id(), "$desired:test");
        retry.response.send(Ok(())).expect("retry succeeds");
    };
    let (completion, ()) = tokio::join!(manager.read_workers.tasks.next(), responder);
    manager
        .handle_read_worker_completion(completion.expect("retry completion"))
        .await;
    persistence_rx
        .changed()
        .await
        .expect("successful retry publishes outbox removal");
    assert!(
        persistence_rx
            .borrow_and_update()
            .as_ref()
            .expect("persistence request")
            .snapshot()
            .is_empty()
    );
    assert!(matches!(
        control_rx.recv().await,
        Some(TimelineActorControl::ReadStateProjection {
            local_viewed_event_id: Some(local),
            server_confirmed_read_event_id: None,
            sync: TimelineReadStateSync::Synced,
        }) if local == "$desired:test"
    ));
}

#[tokio::test]
async fn restored_fully_read_projects_pending_then_server_confirmed_after_apply() {
    let key = room_key();
    let read_key = ReadStateKey::FullyReadAndPrivateUnthreaded {
        room_id: key.room_id().to_owned(),
    };
    let (actor_handle, mut control_rx) =
        actor_handle_with_positions(7, [("$restored-fully:test", 5)]);
    let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
    let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
    let (persistence, mut persistence_rx) = ReadPersistenceIngress::channel();
    manager.read_workers = ReadWorkerSupervisor::synthetic_restored(
        read_network_tx,
        restored_read_snapshot(read_key.clone(), "$restored-fully:test"),
        persistence,
    );

    manager
        .handle_authoritative_read_state_observed(&key, 7, read_key, None)
        .await;
    assert!(matches!(
        control_rx.recv().await,
        Some(TimelineActorControl::ReadStateProjection {
            local_viewed_event_id: Some(event_id),
            server_confirmed_read_event_id: None,
            sync: TimelineReadStateSync::Pending,
        }) if event_id == "$restored-fully:test"
    ));

    let responder = async {
        let request = read_network_rx.recv().await.expect("restored retry starts");
        request
            .response
            .send(Ok(()))
            .expect("restored retry succeeds");
    };
    let (network_completion, ()) = tokio::join!(manager.read_workers.tasks.next(), responder);
    manager
        .handle_read_worker_completion(network_completion.expect("network completion"))
        .await;

    let acknowledge = async {
        loop {
            match control_rx.recv().await.expect("actor apply control") {
                TimelineActorControl::ApplyReadSuccess {
                    kind: ReadActorApplyKind::FullyRead,
                    event_id,
                    acknowledged,
                } => {
                    assert_eq!(event_id, "$restored-fully:test");
                    acknowledged.send(true).expect("acknowledge actor apply");
                    break;
                }
                TimelineActorControl::ReadStateProjection { .. } => {}
                TimelineActorControl::ReadStatePolicyChanged { .. } => {}
                TimelineActorControl::DisplayPolicyChanged { .. } => {}
                TimelineActorControl::ReplayInitialItems { .. }
                | TimelineActorControl::StartLiveTailRefresh { .. }
                | TimelineActorControl::CancelLiveTailNetwork { .. }
                | TimelineActorControl::BeginGapRepairDemand
                | TimelineActorControl::EndGapRepairDemand => {}
                TimelineActorControl::ApplyReadSuccess { .. } => {
                    panic!("unexpected actor apply kind")
                }
            }
        }
    };
    let (apply_completion, ()) = tokio::join!(manager.read_workers.tasks.next(), acknowledge);
    manager
        .handle_read_worker_completion(apply_completion.expect("actor apply completion"))
        .await;

    persistence_rx
        .changed()
        .await
        .expect("successful restore publishes empty outbox");
    assert!(
        persistence_rx
            .borrow_and_update()
            .as_ref()
            .expect("persistence request")
            .snapshot()
            .is_empty()
    );
    assert!(matches!(
        control_rx.recv().await,
        Some(TimelineActorControl::ReadStateProjection {
            local_viewed_event_id: Some(local),
            server_confirmed_read_event_id: Some(server),
            sync: TimelineReadStateSync::Synced,
        }) if local == "$restored-fully:test" && server == "$restored-fully:test"
    ));
}

#[tokio::test(start_paused = true)]
async fn reconnect_preserves_a_bounded_reconciliation_wake_for_new_read_waiters() {
    let key = room_key();
    let read_key = ReadStateKey::PublicUnthreaded {
        room_id: key.room_id().to_owned(),
    };
    let (ordinary_tx, _ordinary_rx) = mpsc::channel(1);
    let (control_tx, _control_rx) = mpsc::channel(1);
    let (_position_tx, position_rx) = watch::channel(Arc::new(TimelinePositionIndex {
        generation: u128::from(7_u64) << 64,
        ranks: HashMap::new(),
    }));
    let actor_handle = TimelineActorHandle {
        tx: ordinary_tx,
        control_tx: Some(control_tx),
        thread_summary_projection: crate::timeline::actor::ThreadSummaryProjectionIngress::channel(
        )
        .0,
        position_rx: Some(position_rx),
        task: None,
        auxiliary_tasks: Vec::new(),
        subscription_generation: None,
        enqueue_context: None,
    };
    let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
    let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
    let (persistence, _persistence_rx) = ReadPersistenceIngress::channel();
    manager.read_workers = ReadWorkerSupervisor::synthetic_restored(
        read_network_tx,
        restored_public_read_snapshot(key.room_id(), "$restored:test"),
        persistence,
    );

    manager
        .wake_all_desired_reads(ReadRetrySource::Reconnect)
        .await;
    manager
        .route_read_command(
            fake_rid(29_601),
            key,
            "$new-waiter:test".to_owned(),
            ReadCommandKind::Receipt,
        )
        .await;

    assert!(
        manager
            .read_workers
            .scheduled_retries
            .contains_key(&read_key),
        "reconnect must not cancel the only bounded reconciliation wake"
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    let completion = manager
        .read_workers
        .retry_tasks
        .next()
        .await
        .expect("bounded reconciliation wake");
    manager.handle_read_worker_completion(completion).await;
    let responder = async {
        let request = read_network_rx
            .recv()
            .await
            .expect("new waiter receives a network attempt after the bound");
        assert_eq!(request.operation.target().event_id(), "$new-waiter:test");
        request.response.send(Err(())).expect("settle retry");
    };
    let (completion, ()) = tokio::join!(manager.read_workers.tasks.next(), responder);
    manager
        .handle_read_worker_completion(completion.expect("network completion"))
        .await;
}

#[tokio::test]
async fn invalidating_retry_actively_finishes_the_long_lived_sleeper() {
    let (network_tx, _network_rx) = mpsc::unbounded_channel();
    let mut supervisor = ReadWorkerSupervisor::synthetic_with_retry(
        network_tx,
        Duration::from_secs(30),
        Duration::from_secs(60),
        Duration::from_secs(60),
    );
    let key = ReadStateKey::PublicUnthreaded {
        room_id: "!retry-cancel:example.invalid".to_owned(),
    };
    supervisor.schedule_retry(&key);
    assert_eq!(supervisor.retry_tasks.len(), 1);
    assert_eq!(supervisor.scheduled_retries.len(), 1);

    supervisor.invalidate_retry(&key);

    assert!(supervisor.scheduled_retries.is_empty());
    let completion = executor::timeout(Duration::from_millis(25), supervisor.retry_tasks.next())
        .await
        .expect("retry invalidation must wake the sleeper promptly")
        .expect("cancelled retry completion");
    assert!(matches!(
        completion,
        ReadWorkerCompletion::RetryWake {
            key: observed,
            cancelled: true,
            ..
        } if observed == key
    ));
    assert!(
        supervisor.retry_tasks.is_empty(),
        "an invalidated retry must not leave a sixty-second task behind"
    );
}

#[tokio::test]
async fn retry_serial_exhaustion_never_reuses_a_live_stale_token() {
    let (network_tx, _network_rx) = mpsc::unbounded_channel();
    let mut supervisor = ReadWorkerSupervisor::synthetic_with_retry(
        network_tx,
        Duration::from_secs(30),
        Duration::from_secs(60),
        Duration::from_secs(60),
    );
    let key = ReadStateKey::PublicUnthreaded {
        room_id: "!retry-token-exhaustion:example.invalid".to_owned(),
    };

    supervisor.retry_serial = u64::MAX;
    supervisor.schedule_retry(&key);
    let stale_generation = supervisor
        .scheduled_retries
        .get(&key)
        .map(|(generation, _)| generation.clone())
        .expect("stale retry token");
    supervisor.invalidate_retry(&key);

    // Model the manager-wide serial reaching exhaustion again while the
    // cancelled wake remains queued in `retry_tasks`.
    supervisor.retry_serial = u64::MAX;
    supervisor.schedule_retry(&key);
    let current_generation = supervisor
        .scheduled_retries
        .get(&key)
        .map(|(generation, _)| generation.clone())
        .expect("current retry token");

    let stale = executor::timeout(Duration::from_millis(25), supervisor.retry_tasks.next())
        .await
        .expect("cancelled stale wake must be ready")
        .expect("cancelled stale retry completion");
    assert!(matches!(
        stale,
        ReadWorkerCompletion::RetryWake {
            key: observed,
            generation: observed_generation,
            cancelled: true,
        } if observed == key && observed_generation == stale_generation
    ));
    assert!(
        !supervisor.accept_retry_wake(&key, stale_generation),
        "an exhausted stale token must not settle the current retry"
    );
    assert!(
        supervisor
            .scheduled_retries
            .get(&key)
            .is_some_and(|(generation, _)| generation == &current_generation),
        "the current retry must remain scheduled after the stale wake"
    );
}

#[tokio::test]
async fn completed_retry_keys_do_not_accumulate_generation_bookkeeping() {
    let (network_tx, _network_rx) = mpsc::unbounded_channel();
    let mut supervisor = ReadWorkerSupervisor::synthetic_with_retry(
        network_tx,
        Duration::from_secs(30),
        Duration::from_secs(60),
        Duration::from_secs(60),
    );

    for index in 0..256 {
        let key = ReadStateKey::PublicUnthreaded {
            room_id: format!("!completed-retry-{index}:example.invalid"),
        };
        supervisor.schedule_retry(&key);
        let generation = supervisor
            .scheduled_retries
            .get(&key)
            .map(|(generation, _)| generation.clone())
            .expect("retry generation");

        supervisor.reset_retry(&key);
        let cancelled = executor::timeout(Duration::from_millis(25), supervisor.retry_tasks.next())
            .await
            .expect("retry cancellation must be bounded")
            .expect("cancelled retry completion");
        assert!(matches!(
            cancelled,
            ReadWorkerCompletion::RetryWake {
                key: observed,
                generation: observed_generation,
                cancelled: true,
            } if observed == key && observed_generation == generation
        ));
        assert!(
            !supervisor.accept_retry_wake(&key, generation),
            "a cancelled sleeper must remain stale after its key retires"
        );
    }

    assert_eq!(
        supervisor.retry_bookkeeping_key_count(),
        0,
        "completed historical keys must not remain in retry bookkeeping"
    );
}

#[tokio::test]
async fn authoritative_server_ahead_clears_restored_read_without_network_retry() {
    let key = room_key();
    let read_key = ReadStateKey::PublicUnthreaded {
        room_id: key.room_id().to_owned(),
    };
    let (ordinary_tx, _ordinary_rx) = mpsc::channel(1);
    let (control_tx, _control_rx) = mpsc::channel(1);
    let (_position_tx, position_rx) = watch::channel(Arc::new(TimelinePositionIndex {
        generation: u128::from(7_u64) << 64,
        ranks: HashMap::from([
            ("$desired:test".to_owned(), 5),
            ("$server-ahead:test".to_owned(), 6),
        ]),
    }));
    let actor_handle = TimelineActorHandle {
        tx: ordinary_tx,
        control_tx: Some(control_tx),
        thread_summary_projection: crate::timeline::actor::ThreadSummaryProjectionIngress::channel(
        )
        .0,
        position_rx: Some(position_rx),
        task: None,
        auxiliary_tasks: Vec::new(),
        subscription_generation: None,
        enqueue_context: None,
    };
    let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
    let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
    let (persistence, mut persistence_rx) = ReadPersistenceIngress::channel();
    manager.read_workers = ReadWorkerSupervisor::synthetic_restored(
        read_network_tx,
        restored_public_read_snapshot(key.room_id(), "$desired:test"),
        persistence,
    );

    manager
        .handle_authoritative_read_state_observed(
            &key,
            7,
            read_key,
            Some("$server-ahead:test".to_owned()),
        )
        .await;

    assert!(read_network_rx.try_recv().is_err());
    assert!(manager.read_workers.tasks.is_empty());
    persistence_rx
        .changed()
        .await
        .expect("server-ahead reconciliation publishes removal");
    assert!(
        persistence_rx
            .borrow_and_update()
            .as_ref()
            .expect("persistence request")
            .snapshot()
            .is_empty()
    );
}

#[tokio::test]
async fn authoritative_reconciliation_keeps_unordered_remaining_candidate_pending() {
    let key = room_key();
    let read_key = ReadStateKey::PublicUnthreaded {
        room_id: key.room_id().to_owned(),
    };
    let mut restored = ReadStateEngine::new(7);
    restored.admit(
        7,
        read_key.clone(),
        ReadTarget::new("$positioned:test".to_owned()),
        ReadWaiterId::new(1),
    );
    restored.admit(
        7,
        read_key.clone(),
        ReadTarget::new("$outside-window:test".to_owned()),
        ReadWaiterId::new(2),
    );
    let (ordinary_tx, _ordinary_rx) = mpsc::channel(1);
    let (control_tx, mut control_rx) = mpsc::channel(1);
    let (_position_tx, position_rx) = watch::channel(Arc::new(TimelinePositionIndex {
        generation: u128::from(7_u64) << 64,
        ranks: HashMap::from([
            ("$positioned:test".to_owned(), 5),
            ("$server-ahead:test".to_owned(), 6),
        ]),
    }));
    let actor_handle = TimelineActorHandle {
        tx: ordinary_tx,
        control_tx: Some(control_tx),
        thread_summary_projection: crate::timeline::actor::ThreadSummaryProjectionIngress::channel(
        )
        .0,
        position_rx: Some(position_rx),
        task: None,
        auxiliary_tasks: Vec::new(),
        subscription_generation: None,
        enqueue_context: None,
    };
    let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
    let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
    let (persistence, _persistence_rx) = ReadPersistenceIngress::channel();
    manager.read_workers = ReadWorkerSupervisor::synthetic_restored(
        read_network_tx,
        restored.persistence_snapshot(),
        persistence,
    );

    manager
        .handle_authoritative_read_state_observed(
            &key,
            7,
            read_key.clone(),
            Some("$server-ahead:test".to_owned()),
        )
        .await;

    assert_eq!(manager.read_workers.state.candidate_count(&read_key), 1);
    assert!(manager.read_workers.reconciliation_pending(&read_key));
    assert!(manager.read_workers.tasks.is_empty());
    assert!(read_network_rx.try_recv().is_err());
    assert!(matches!(
        control_rx.recv().await,
        Some(TimelineActorControl::ReadStateProjection {
            local_viewed_event_id: None,
            sync: TimelineReadStateSync::Pending,
            ..
        })
    ));
}

#[tokio::test]
async fn stalled_read_receipt_worker_does_not_block_cached_subscription_replay() {
    let key = room_key();
    let read_request_id = fake_rid(28_480);
    let subscribe_request_id = fake_rid(28_481);
    let (ordinary_tx, _ordinary_rx) = mpsc::channel(1);
    let (control_tx, mut control_rx) = mpsc::channel(2);
    let (_position_tx, position_rx) = watch::channel(Arc::new(TimelinePositionIndex {
        generation: 11,
        ranks: HashMap::from([("$read-target:test".to_owned(), 7)]),
    }));
    let actor_handle = TimelineActorHandle {
        tx: ordinary_tx,
        control_tx: Some(control_tx),
        thread_summary_projection: crate::timeline::actor::ThreadSummaryProjectionIngress::channel(
        )
        .0,
        position_rx: Some(position_rx),
        task: None,
        auxiliary_tasks: Vec::new(),
        subscription_generation: None,
        enqueue_context: None,
    };
    let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
    let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
    manager.read_workers =
        ReadWorkerSupervisor::synthetic(read_network_tx, Duration::from_secs(30));
    let (manager_tx, manager_rx) = mpsc::channel(4);
    manager.msg_tx = manager_tx.clone();
    manager.msg_rx = manager_rx;
    let run = executor::spawn(manager.run());

    manager_tx
        .send(TimelineMessage::Command(TimelineCommand::SendReadReceipt {
            request_id: read_request_id,
            key: key.clone(),
            event_id: "$read-target:test".to_owned(),
        }))
        .await
        .expect("admit read command");
    let stalled = executor::timeout(Duration::from_millis(100), read_network_rx.recv())
        .await
        .expect("read worker must start")
        .expect("synthetic read request");

    manager_tx
        .send(TimelineMessage::Command(TimelineCommand::Subscribe {
            request_id: subscribe_request_id,
            key,
            initial_backfill: koushi_protocol::command::InitialBackfillPolicy::Disabled,
        }))
        .await
        .expect("queue cached subscribe");

    assert!(matches!(
        executor::timeout(Duration::from_millis(100), control_rx.recv())
            .await
            .expect("cached replay must not wait for read network"),
        Some(TimelineActorControl::ReplayInitialItems { cause_request_id })
            if cause_request_id == subscribe_request_id
    ));

    drop(stalled);
    let (acknowledged, acknowledgement) = oneshot::channel();
    manager_tx
        .send(TimelineMessage::Shutdown {
            acknowledged: Some(acknowledged),
        })
        .await
        .expect("shutdown manager");
    acknowledgement.await.expect("shutdown acknowledgement");
    run.await.expect("manager task");
}

#[tokio::test]
async fn newer_positioned_read_target_cancels_stale_worker_and_settles_both_waiters_once() {
    let key = room_key();
    let older_request_id = fake_rid(28_482);
    let newer_request_id = fake_rid(28_483);
    let (ordinary_tx, _ordinary_rx) = mpsc::channel(1);
    let (control_tx, _control_rx) = mpsc::channel(2);
    let (_position_tx, position_rx) = watch::channel(Arc::new(TimelinePositionIndex {
        generation: 12,
        ranks: HashMap::from([
            ("$read-old:test".to_owned(), 7),
            ("$read-new:test".to_owned(), 8),
        ]),
    }));
    let actor_handle = TimelineActorHandle {
        tx: ordinary_tx,
        control_tx: Some(control_tx),
        thread_summary_projection: crate::timeline::actor::ThreadSummaryProjectionIngress::channel(
        )
        .0,
        position_rx: Some(position_rx),
        task: None,
        auxiliary_tasks: Vec::new(),
        subscription_generation: None,
        enqueue_context: None,
    };
    let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
    let (event_tx, mut event_rx) = broadcast::channel(8);
    manager.event_tx = event_tx;
    let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
    manager.read_workers =
        ReadWorkerSupervisor::synthetic(read_network_tx, Duration::from_secs(30));
    let (manager_tx, manager_rx) = mpsc::channel(4);
    manager.msg_tx = manager_tx.clone();
    manager.msg_rx = manager_rx;
    let run = executor::spawn(manager.run());

    for (request_id, event_id) in [
        (older_request_id, "$read-old:test"),
        (newer_request_id, "$read-new:test"),
    ] {
        manager_tx
            .send(TimelineMessage::Command(TimelineCommand::SendReadReceipt {
                request_id,
                key: key.clone(),
                event_id: event_id.to_owned(),
            }))
            .await
            .expect("admit read command");
        if request_id == older_request_id {
            break;
        }
    }
    let older = executor::timeout(Duration::from_millis(100), read_network_rx.recv())
        .await
        .expect("older read worker must start")
        .expect("older synthetic read request");
    assert_eq!(older.operation.target().event_id(), "$read-old:test");

    manager_tx
        .send(TimelineMessage::Command(TimelineCommand::SendReadReceipt {
            request_id: newer_request_id,
            key: key.clone(),
            event_id: "$read-new:test".to_owned(),
        }))
        .await
        .expect("admit newer read command");
    let newer = executor::timeout(Duration::from_millis(100), read_network_rx.recv())
        .await
        .expect("newer read worker must start")
        .expect("newer synthetic read request");
    assert_eq!(newer.operation.target().event_id(), "$read-new:test");
    assert!(
        older.response.send(Ok(())).is_err(),
        "dominated worker must be cancelled before its late success"
    );
    newer.response.send(Ok(())).expect("complete newer target");

    let mut settled = HashSet::new();
    while settled.len() < 2 {
        let event = executor::timeout(Duration::from_millis(100), event_rx.recv())
            .await
            .expect("both waiters must settle")
            .expect("event stream");
        if let CoreEvent::LiveSignals(LiveSignalsEvent::ReadReceiptSent { request_id, .. }) = event
        {
            assert!(settled.insert(request_id), "duplicate waiter success");
        }
    }
    assert_eq!(settled, HashSet::from([older_request_id, newer_request_id]));
    assert!(
        executor::timeout(Duration::from_millis(25), event_rx.recv())
            .await
            .is_err(),
        "stale completion must not emit a second terminal"
    );

    let (acknowledged, acknowledgement) = oneshot::channel();
    manager_tx
        .send(TimelineMessage::Shutdown {
            acknowledged: Some(acknowledged),
        })
        .await
        .expect("shutdown manager");
    acknowledgement.await.expect("shutdown acknowledgement");
    run.await.expect("manager task");
}

#[tokio::test]
async fn coalesced_read_timeout_fails_each_waiter_once_without_retry_storm() {
    let key = room_key();
    let request_ids = [fake_rid(28_484), fake_rid(28_485)];
    let (ordinary_tx, _ordinary_rx) = mpsc::channel(1);
    let (control_tx, _control_rx) = mpsc::channel(1);
    let actor_handle = TimelineActorHandle {
        tx: ordinary_tx,
        control_tx: Some(control_tx),
        thread_summary_projection: crate::timeline::actor::ThreadSummaryProjectionIngress::channel(
        )
        .0,
        position_rx: None,
        task: None,
        auxiliary_tasks: Vec::new(),
        subscription_generation: None,
        enqueue_context: None,
    };
    let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
    let (event_tx, mut event_rx) = broadcast::channel(8);
    manager.event_tx = event_tx;
    let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
    manager.read_workers =
        ReadWorkerSupervisor::synthetic(read_network_tx, Duration::from_millis(20));
    let (manager_tx, manager_rx) = mpsc::channel(4);
    manager.msg_tx = manager_tx.clone();
    manager.msg_rx = manager_rx;
    let run = executor::spawn(manager.run());

    for request_id in request_ids {
        manager_tx
            .send(TimelineMessage::Command(TimelineCommand::SendReadReceipt {
                request_id,
                key: key.clone(),
                event_id: "$same-target:test".to_owned(),
            }))
            .await
            .expect("admit coalesced read");
    }
    let stalled = executor::timeout(Duration::from_millis(100), read_network_rx.recv())
        .await
        .expect("one network worker must start")
        .expect("synthetic read request");

    let mut failed = HashSet::new();
    while failed.len() < 2 {
        let event = executor::timeout(Duration::from_millis(100), event_rx.recv())
            .await
            .expect("timeout must settle both waiters")
            .expect("event stream");
        if let CoreEvent::OperationFailed {
            request_id,
            failure:
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::Timeout,
                },
        } = event
        {
            assert!(failed.insert(request_id), "duplicate waiter timeout");
        }
    }
    assert_eq!(failed, HashSet::from(request_ids));
    assert!(
        executor::timeout(Duration::from_millis(40), read_network_rx.recv())
            .await
            .is_err(),
        "timeout retains desired state but must not spin an immediate retry"
    );
    assert!(
        executor::timeout(Duration::from_millis(20), event_rx.recv())
            .await
            .is_err(),
        "each waiter receives exactly one timeout"
    );

    drop(stalled);
    let (acknowledged, acknowledgement) = oneshot::channel();
    manager_tx
        .send(TimelineMessage::Shutdown {
            acknowledged: Some(acknowledged),
        })
        .await
        .expect("shutdown manager");
    acknowledgement.await.expect("shutdown acknowledgement");
    run.await.expect("manager task");
}

#[tokio::test]
async fn fully_read_success_waits_for_actor_control_ack_before_terminal_event() {
    let key = room_key();
    let request_id = fake_rid(28_486);
    let (ordinary_tx, _ordinary_rx) = mpsc::channel(1);
    let (control_tx, mut control_rx) = mpsc::channel(1);
    let actor_handle = TimelineActorHandle {
        tx: ordinary_tx,
        control_tx: Some(control_tx),
        thread_summary_projection: crate::timeline::actor::ThreadSummaryProjectionIngress::channel(
        )
        .0,
        position_rx: None,
        task: None,
        auxiliary_tasks: Vec::new(),
        subscription_generation: None,
        enqueue_context: None,
    };
    let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
    let (action_tx, mut action_rx) = mpsc::channel(4);
    let (event_tx, mut event_rx) = broadcast::channel(4);
    manager.action_tx = action_tx;
    manager.event_tx = event_tx;
    let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
    manager.read_workers =
        ReadWorkerSupervisor::synthetic(read_network_tx, Duration::from_secs(30));
    let (manager_tx, manager_rx) = mpsc::channel(4);
    manager.msg_tx = manager_tx.clone();
    manager.msg_rx = manager_rx;
    let run = executor::spawn(manager.run());

    manager_tx
        .send(TimelineMessage::Command(TimelineCommand::SetFullyRead {
            request_id,
            key: key.clone(),
            event_id: "$fully-read:test".to_owned(),
        }))
        .await
        .expect("admit fully-read command");
    let network = executor::timeout(Duration::from_millis(100), read_network_rx.recv())
        .await
        .expect("fully-read worker must start")
        .expect("synthetic read request");
    network.response.send(Ok(())).expect("SDK success");
    let control = executor::timeout(Duration::from_millis(100), control_rx.recv())
        .await
        .expect("success must enter actor control lane")
        .expect("actor apply control");
    assert!(
        event_rx.try_recv().is_err(),
        "success must wait for actor ACK"
    );
    let TimelineActorControl::ApplyReadSuccess {
        kind: ReadActorApplyKind::FullyRead,
        event_id,
        acknowledged,
    } = control
    else {
        panic!("expected fully-read actor control");
    };
    assert_eq!(event_id, "$fully-read:test");
    acknowledged.send(true).expect("ack actor state update");

    assert!(matches!(
        executor::timeout(Duration::from_millis(100), action_rx.recv())
            .await
            .expect("reducer action after ACK"),
        Some(actions)
            if matches!(actions.as_slice(), [AppAction::RoomMarkedAsReadSucceeded { request_id: sequence, .. }] if *sequence == request_id.sequence)
    ));
    assert!(matches!(
        executor::timeout(Duration::from_millis(100), event_rx.recv())
            .await
            .expect("success after ACK")
            .expect("event stream"),
        CoreEvent::LiveSignals(LiveSignalsEvent::FullyReadSet {
            request_id: settled,
            ..
        }) if settled == request_id
    ));

    let (acknowledged, acknowledgement) = oneshot::channel();
    manager_tx
        .send(TimelineMessage::Shutdown {
            acknowledged: Some(acknowledged),
        })
        .await
        .expect("shutdown manager");
    acknowledgement.await.expect("shutdown acknowledgement");
    run.await.expect("manager task");
}

#[tokio::test]
async fn fully_read_success_after_actor_removal_fails_without_success_terminal() {
    let key = room_key();
    let request_id = fake_rid(28_487);
    let (ordinary_tx, _ordinary_rx) = mpsc::channel(1);
    let (control_tx, mut control_rx) = mpsc::channel(1);
    let actor_handle = TimelineActorHandle {
        tx: ordinary_tx,
        control_tx: Some(control_tx),
        thread_summary_projection: crate::timeline::actor::ThreadSummaryProjectionIngress::channel(
        )
        .0,
        position_rx: None,
        task: None,
        auxiliary_tasks: Vec::new(),
        subscription_generation: None,
        enqueue_context: None,
    };
    let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
    let (action_tx, _action_rx) = mpsc::channel(4);
    let (event_tx, mut event_rx) = broadcast::channel(4);
    manager.action_tx = action_tx;
    manager.event_tx = event_tx;
    let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
    manager.read_workers =
        ReadWorkerSupervisor::synthetic(read_network_tx, Duration::from_secs(30));
    let (manager_tx, manager_rx) = mpsc::channel(4);
    manager.msg_tx = manager_tx.clone();
    manager.msg_rx = manager_rx;
    let run = executor::spawn(manager.run());

    manager_tx
        .send(TimelineMessage::Command(TimelineCommand::SetFullyRead {
            request_id,
            key: key.clone(),
            event_id: "$fully-read:test".to_owned(),
        }))
        .await
        .expect("admit fully-read command");
    let network = executor::timeout(Duration::from_millis(100), read_network_rx.recv())
        .await
        .expect("fully-read worker must start")
        .expect("synthetic read request");
    manager_tx
        .send(TimelineMessage::Command(TimelineCommand::Unsubscribe {
            request_id: fake_rid(28_488),
            key: key.clone(),
        }))
        .await
        .expect("remove actor");
    assert!(
        executor::timeout(Duration::from_millis(100), control_rx.recv())
            .await
            .expect("actor control sender must close")
            .is_none()
    );
    network
        .response
        .send(Ok(()))
        .expect("late SDK success after actor removal");

    assert!(matches!(
        executor::timeout(Duration::from_millis(100), event_rx.recv())
            .await
            .expect("missing actor must fail waiter")
            .expect("event stream"),
        CoreEvent::OperationFailed {
            request_id: failed,
            failure: CoreFailure::TimelineOperationFailed {
                kind: TimelineFailureKind::Sdk,
            },
        } if failed == request_id
    ));
    assert!(
        executor::timeout(Duration::from_millis(20), event_rx.recv())
            .await
            .is_err(),
        "late network success must not emit a success terminal"
    );

    let (acknowledged, acknowledgement) = oneshot::channel();
    manager_tx
        .send(TimelineMessage::Shutdown {
            acknowledged: Some(acknowledged),
        })
        .await
        .expect("shutdown manager");
    acknowledgement.await.expect("shutdown acknowledgement");
    run.await.expect("manager task");
}

#[tokio::test]
async fn read_admission_rejects_missing_session_actor_and_invalid_ids_immediately() {
    let key = room_key();
    let (event_tx, mut event_rx) = broadcast::channel(8);
    let mut manager =
        live_tail_test_manager(HashMap::from([(key.clone(), test_timeline_actor_handle())]));
    manager.event_tx = event_tx;

    manager
        .handle_command(TimelineCommand::SendReadReceipt {
            request_id: fake_rid(28_489),
            key: key.clone(),
            event_id: "$event:test".to_owned(),
        })
        .await;
    assert!(matches!(
        event_rx.try_recv(),
        Ok(CoreEvent::OperationFailed {
            failure: CoreFailure::SessionRequired,
            ..
        })
    ));

    let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
    manager.read_workers =
        ReadWorkerSupervisor::synthetic(read_network_tx, Duration::from_secs(30));
    manager.timelines.clear();
    manager
        .handle_command(TimelineCommand::SendReadReceipt {
            request_id: fake_rid(28_490),
            key: key.clone(),
            event_id: "$event:test".to_owned(),
        })
        .await;
    assert!(matches!(
        event_rx.try_recv(),
        Ok(CoreEvent::OperationFailed {
            failure: CoreFailure::TimelineOperationFailed {
                kind: TimelineFailureKind::NotSubscribed,
            },
            ..
        })
    ));

    manager
        .timelines
        .insert(key.clone(), test_timeline_actor_handle());
    manager.read_workers.send_read_receipts = false;
    manager
        .handle_command(TimelineCommand::SendReadReceipt {
            request_id: fake_rid(28_491),
            key: key.clone(),
            event_id: "$event:test".to_owned(),
        })
        .await;
    assert!(matches!(
        event_rx.try_recv(),
        Ok(CoreEvent::OperationFailed {
            failure: CoreFailure::TimelineOperationFailed {
                kind: TimelineFailureKind::Forbidden,
            },
            ..
        })
    ));
    assert!(manager.read_workers.waiters.is_empty());
    assert!(manager.read_workers.tasks.is_empty());

    manager.read_workers.send_read_receipts = true;
    let flip_request_id = fake_rid(28_492);
    manager
        .handle_command(TimelineCommand::SendReadReceipt {
            request_id: flip_request_id,
            key: key.clone(),
            event_id: "$event:test".to_owned(),
        })
        .await;
    assert_eq!(manager.read_workers.waiters.len(), 1);
    manager.handle_read_state_policy_changed(1, false).await;
    assert!(matches!(
        event_rx.try_recv(),
        Ok(CoreEvent::OperationFailed {
            request_id,
            failure: CoreFailure::TimelineOperationFailed {
                kind: TimelineFailureKind::Forbidden,
            },
        }) if request_id == flip_request_id
    ));
    assert!(manager.read_workers.waiters.is_empty());
    assert!(manager.read_workers.state.persistence_snapshot().is_empty());
    let cancelled = manager
        .read_workers
        .tasks
        .next()
        .await
        .expect("policy flip cancels the admitted worker");
    manager.handle_read_worker_completion(cancelled).await;
    assert!(event_rx.try_recv().is_err());
    assert!(read_network_rx.try_recv().is_err());

    manager
        .handle_command(TimelineCommand::SetFullyRead {
            request_id: fake_rid(28_493),
            key,
            event_id: "not-an-event-id".to_owned(),
        })
        .await;
    assert!(matches!(
        event_rx.try_recv(),
        Ok(CoreEvent::OperationFailed {
            failure: CoreFailure::TimelineOperationFailed {
                kind: TimelineFailureKind::Sdk,
            },
            ..
        })
    ));
    assert!(manager.read_workers.tasks.is_empty());
    assert!(read_network_rx.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn failed_read_network_settles_waiter_once_then_retries_after_capped_backoff() {
    let key = room_key();
    let request_id = fake_rid(28_492);
    let (event_tx, mut event_rx) = broadcast::channel(4);
    let mut manager =
        live_tail_test_manager(HashMap::from([(key.clone(), test_timeline_actor_handle())]));
    manager.event_tx = event_tx;
    let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
    manager.read_workers = ReadWorkerSupervisor::synthetic_with_retry(
        read_network_tx,
        Duration::from_secs(30),
        Duration::from_secs(1),
        Duration::from_secs(4),
    );

    manager
        .handle_command(TimelineCommand::SendReadReceipt {
            request_id,
            key: key.clone(),
            event_id: "$event:test".to_owned(),
        })
        .await;
    let responder = async {
        let request = read_network_rx.recv().await.expect("read request");
        request
            .response
            .send(Err(()))
            .expect("fail network request");
    };
    let (completion, ()) = tokio::join!(manager.read_workers.tasks.next(), responder);
    manager
        .handle_read_worker_completion(completion.expect("worker completion"))
        .await;

    assert!(matches!(
        event_rx.try_recv(),
        Ok(CoreEvent::OperationFailed {
            request_id: failed,
            failure: CoreFailure::TimelineOperationFailed {
                kind: TimelineFailureKind::Sdk,
            },
        }) if failed == request_id
    ));
    assert!(event_rx.try_recv().is_err());
    assert!(read_network_rx.try_recv().is_err());

    assert!(
        manager
            .read_workers
            .retry_tasks
            .next()
            .now_or_never()
            .is_none(),
        "scheduled retry must begin pending"
    );
    tokio::time::advance(Duration::from_millis(999)).await;
    assert!(
        manager
            .read_workers
            .retry_tasks
            .next()
            .now_or_never()
            .is_none(),
        "retry must not run before the backoff deadline"
    );
    tokio::time::advance(Duration::from_millis(1)).await;
    let retry_wake = manager
        .read_workers
        .retry_tasks
        .next()
        .await
        .expect("backoff wake");
    manager.handle_read_worker_completion(retry_wake).await;
    let responder = async {
        let retried = read_network_rx.recv().await.expect("retry network request");
        assert_eq!(retried.operation.target().event_id(), "$event:test");
        retried.response.send(Ok(())).expect("retry succeeds");
    };
    let (completion, ()) = tokio::join!(manager.read_workers.tasks.next(), responder);
    manager
        .handle_read_worker_completion(completion.expect("retry completion"))
        .await;
    assert!(
        event_rx.try_recv().is_err(),
        "background retry must not emit a second user terminal"
    );
    assert!(!manager.read_workers.state.has_candidate(
        &ReadStateKey::PublicUnthreaded {
            room_id: key.room_id().to_owned(),
        },
        "$event:test",
    ));
}

#[test]
fn read_retry_delay_is_exponential_and_capped() {
    assert_eq!(
        read_retry_delay_for_attempt(Duration::from_secs(1), Duration::from_secs(4), 0,),
        Duration::from_secs(1)
    );
    assert_eq!(
        read_retry_delay_for_attempt(Duration::from_secs(1), Duration::from_secs(4), 1,),
        Duration::from_secs(2)
    );
    assert_eq!(
        read_retry_delay_for_attempt(Duration::from_secs(1), Duration::from_secs(4), 64,),
        Duration::from_secs(4)
    );
}

#[tokio::test(start_paused = true)]
async fn sync_restart_preserves_failed_read_backoff_until_its_due_token() {
    let key = room_key();
    let request_id = fake_rid(28_493);
    let (event_tx, mut event_rx) = broadcast::channel(4);
    let mut manager =
        live_tail_test_manager(HashMap::from([(key.clone(), test_timeline_actor_handle())]));
    manager.event_tx = event_tx;
    let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
    manager.read_workers = ReadWorkerSupervisor::synthetic_with_retry(
        read_network_tx,
        Duration::from_secs(30),
        Duration::from_secs(30),
        Duration::from_secs(60),
    );

    manager
        .handle_command(TimelineCommand::SendReadReceipt {
            request_id,
            key,
            event_id: "$event:test".to_owned(),
        })
        .await;
    let responder = async {
        let first = read_network_rx.recv().await.expect("initial request");
        first.response.send(Err(())).expect("fail initial request");
    };
    let (completion, ()) = tokio::join!(manager.read_workers.tasks.next(), responder);
    manager
        .handle_read_worker_completion(completion.expect("initial completion"))
        .await;
    assert!(matches!(
        event_rx.try_recv(),
        Ok(CoreEvent::OperationFailed {
            request_id: failed,
            ..
        }) if failed == request_id
    ));

    manager
        .wake_all_desired_reads(ReadRetrySource::Reconnect)
        .await;
    assert!(
        read_network_rx.try_recv().is_err(),
        "reconnect must not bypass the scheduled backoff"
    );
    tokio::time::advance(Duration::from_secs(30)).await;
    let retry_wake = manager
        .read_workers
        .retry_tasks
        .next()
        .await
        .expect("exact due token");
    manager.handle_read_worker_completion(retry_wake).await;
    let responder = async {
        let retry = read_network_rx.recv().await.expect("due retry");
        retry.response.send(Ok(())).expect("retry succeeds");
    };
    let (completion, ()) = tokio::join!(manager.read_workers.tasks.next(), responder);
    manager
        .handle_read_worker_completion(completion.expect("retry completion"))
        .await;
    tokio::time::advance(Duration::from_secs(60)).await;
    assert!(
        event_rx.try_recv().is_err(),
        "restart retry must not emit a second user terminal"
    );
}

#[tokio::test(start_paused = true)]
async fn room_subscription_checkpoint_preserves_failed_read_backoff() {
    let key = room_key();
    let request_id = fake_rid(28_494);
    let (event_tx, mut event_rx) = broadcast::channel(4);
    let mut manager =
        live_tail_test_manager(HashMap::from([(key.clone(), test_timeline_actor_handle())]));
    manager.event_tx = event_tx;
    manager.room_subscription_service_epoch = 9;
    let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
    manager.read_workers = ReadWorkerSupervisor::synthetic_with_retry(
        read_network_tx,
        Duration::from_secs(30),
        Duration::from_secs(30),
        Duration::from_secs(60),
    );

    manager
        .handle_command(TimelineCommand::SendReadReceipt {
            request_id,
            key: key.clone(),
            event_id: "$event:test".to_owned(),
        })
        .await;
    let responder = async {
        let first = read_network_rx.recv().await.expect("initial request");
        first.response.send(Err(())).expect("fail initial request");
    };
    let (completion, ()) = tokio::join!(manager.read_workers.tasks.next(), responder);
    manager
        .handle_read_worker_completion(completion.expect("initial completion"))
        .await;
    assert!(event_rx.try_recv().is_ok());

    manager
        .wake_desired_reads_for_room(key.room_id(), ReadRetrySource::Checkpoint)
        .await;
    assert!(
        read_network_rx.try_recv().is_err(),
        "checkpoint must not bypass the scheduled backoff"
    );
    tokio::time::advance(Duration::from_secs(30)).await;
    let retry_wake = manager
        .read_workers
        .retry_tasks
        .next()
        .await
        .expect("exact checkpoint retry token");
    manager.handle_read_worker_completion(retry_wake).await;
    let responder = async {
        let retry = read_network_rx.recv().await.expect("due checkpoint retry");
        retry
            .response
            .send(Ok(()))
            .expect("checkpoint retry succeeds");
    };
    let (completion, ()) = tokio::join!(manager.read_workers.tasks.next(), responder);
    manager
        .handle_read_worker_completion(completion.expect("checkpoint retry completion"))
        .await;
    assert!(
        event_rx.try_recv().is_err(),
        "checkpoint retry must not emit a second user terminal"
    );
}

#[tokio::test]
async fn koushi_timeline_builder_projects_sdk_read_receipts() {
    use matrix_sdk::assert_next_with_timeout;
    use matrix_sdk::ruma::{event_id, room_id, user_id};
    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!receipts:example.test");
    let room = server.sync_joined_room(&client, room_id).await;
    let timeline = koushi_timeline_builder(
        &room,
        TimelineFocus::Live {
            hide_threaded_events: false,
        },
    )
    .build()
    .await
    .expect("timeline");
    let (initial_items, mut stream) = timeline.subscribe().await;
    let mut endpoints =
        super::super::receipt_endpoints::ReceiptEndpointMirror::new(initial_items.iter());

    let factory = EventFactory::new().room(room_id);
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(room_id)
                .add_timeline_event(
                    factory
                        .text_msg("first")
                        .event_id(event_id!("$first:example.test"))
                        .sender(user_id!("@alice:example.test"))
                        .into_raw_sync(),
                )
                .add_timeline_event(
                    factory
                        .text_msg("second")
                        .event_id(event_id!("$second:example.test"))
                        .sender(user_id!("@bob:example.test"))
                        .into_raw_sync(),
                ),
        )
        .await;

    let diffs = assert_next_with_timeout!(stream);
    let receipts_by_event =
        live_event_receipts_from_endpoint_changes(endpoints.apply_batch(&diffs), &endpoints);

    let second = receipts_by_event
        .iter()
        .find(|entry| entry.event_id == "$second:example.test")
        .expect("Koushi timeline builder must opt in to SDK read receipt tracking");
    assert!(
        second
            .receipts
            .iter()
            .any(|receipt| receipt.user_id == "@bob:example.test")
    );
}

#[test]
fn live_receipt_observation_action_builder_is_pure_and_orders_profiles_first() {
    let actions = build_live_receipt_observation_actions(
        "!room:example.test",
        vec![LiveEventReceipts {
            event_id: "$event:example.test".to_owned(),
            receipts: vec![LiveReadReceipt {
                user_id: "@bob:example.test".to_owned(),
                display_name: None,
                original_display_label: String::new(),
                avatar: None,
                timestamp_ms: Some(1),
            }],
        }],
        vec![MatrixUserProfile {
            user_id: "@bob:example.test".to_owned(),
            display_name: Some("Bob".to_owned()),
            avatar_mxc_uri: None,
        }],
    );

    assert!(matches!(
        actions.as_slice(),
        [
            AppAction::LiveRoomProfilesObserved { profiles, .. },
            AppAction::UserProfilesUpdated { profiles: cached },
            AppAction::LiveRoomReceiptSummariesUpdated { .. },
        ] if profiles[0].display_label == "Bob"
            && cached[0].display_label == "Bob"
    ));
}

#[tokio::test]
async fn local_receipt_observation_helper_builds_profile_then_receipt_actions() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    use koushi_state::{AppState, SessionInfo, SessionState, reduce};
    use matrix_sdk::assert_next_with_timeout;
    use matrix_sdk::ruma::{event_id, room_id, user_id};
    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    use matrix_sdk_test::{ALICE, JoinedRoomBuilder, event_factory::EventFactory};

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!receipt-profiles:example.test");
    let bob = user_id!("@bob:example.test");
    let room = server.sync_joined_room(&client, room_id).await;
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(room_id).add_state_event(
                EventFactory::new()
                    .room(room_id)
                    .member(bob)
                    .display_name("Relevant room member")
                    .into_raw_sync_state(),
            ),
        )
        .await;

    let timeline = koushi_timeline_builder(
        &room,
        TimelineFocus::Live {
            hide_threaded_events: false,
        },
    )
    .build()
    .await
    .expect("timeline");
    let (initial_items, mut stream) = timeline.subscribe().await;
    let mut endpoints =
        super::super::receipt_endpoints::ReceiptEndpointMirror::new(initial_items.iter());
    let factory = EventFactory::new().room(room_id);
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(room_id)
                .add_timeline_event(
                    factory
                        .text_msg("receipt source")
                        .event_id(event_id!("$receipt-source:example.test"))
                        .sender(bob)
                        .into_raw_sync(),
                )
                .add_timeline_event(
                    factory
                        .text_msg("second receipt source")
                        .event_id(event_id!("$receipt-source-two:example.test"))
                        .sender(bob)
                        .into_raw_sync(),
                ),
        )
        .await;

    let diffs = assert_next_with_timeout!(stream);
    let receipts_by_event =
        live_event_receipts_from_endpoint_changes(endpoints.apply_batch(&diffs), &endpoints);
    let observed_receipts = receipts_by_event
        .iter()
        .find(|entry| {
            entry
                .receipts
                .iter()
                .any(|receipt| receipt.user_id == bob.as_str())
        })
        .cloned()
        .expect("timeline diff should contain a real receipt for the member");

    let session = MatrixClientSession::from_client_for_testing(
        client,
        SessionInfo {
            homeserver: "http://example.invalid".to_owned(),
            user_id: ALICE.to_string(),
            device_id: "DEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    );
    let mut state = AppState {
        session: SessionState::Ready(session.info.clone()),
        ..AppState::default()
    };
    reduce(
        &mut state,
        AppAction::LiveRoomReceiptsWindowReconciled {
            room_id: room_id.to_string(),
            scoped_event_ids: Vec::new(),
            receipts_by_event: vec![observed_receipts.clone()],
        },
    );
    assert_eq!(
        state.live_signals.rooms[room_id.as_str()].receipts_by_event[&observed_receipts.event_id]
            .readers[0]
            .display_name
            .as_deref(),
        Some("Unknown user")
    );

    let action_batch = live_receipt_observation_actions_from_sdk_receipts(
        &session,
        room_id.as_str(),
        vec![observed_receipts.clone()],
    )
    .await;
    assert!(matches!(
        action_batch.first(),
        Some(AppAction::LiveRoomProfilesObserved {
            room_id: observed_room_id,
            profiles,
        }) if observed_room_id == room_id.as_str()
            && profiles.iter().any(|profile| {
                profile.user_id == bob.as_str()
                    && profile.display_name.as_deref() == Some("Relevant room member")
            })
    ));
    assert!(matches!(
        action_batch.last(),
        Some(AppAction::LiveRoomReceiptSummariesUpdated { room_id: observed_room_id, .. })
            if observed_room_id == room_id.as_str()
    ));

    for action in action_batch {
        reduce(&mut state, action);
    }

    assert_eq!(
        state.profile.room_users[room_id.as_str()][bob.as_str()]
            .display_name
            .as_deref(),
        Some("Relevant room member")
    );
    assert_eq!(
        state.profile.users[bob.as_str()].display_name.as_deref(),
        Some("Relevant room member")
    );
    assert_eq!(
        state.live_signals.rooms[room_id.as_str()].receipts_by_event[&observed_receipts.event_id]
            .readers[0]
            .display_name
            .as_deref(),
        Some("Relevant room member")
    );
}

#[tokio::test]
async fn production_receipt_diff_delivery_refreshes_unknown_with_room_profile() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    use koushi_state::{AppState, reduce};
    use matrix_sdk::ruma::{event_id, room_id, user_id};
    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    use matrix_sdk_test::{ALICE, JoinedRoomBuilder, event_factory::EventFactory};

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!receipt-production:example.test");
    let bob = user_id!("@bob:example.test");
    server.sync_joined_room(&client, room_id).await;
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(room_id).add_state_event(
                EventFactory::new()
                    .room(room_id)
                    .member(bob)
                    .display_name("Relevant room member")
                    .into_raw_sync_state(),
            ),
        )
        .await;

    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client,
        SessionInfo {
            homeserver: "http://example.invalid".to_owned(),
            user_id: ALICE.to_string(),
            device_id: "DEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    ));
    let receipts = vec![LiveEventReceipts {
        event_id: event_id!("$receipt-production:example.test").to_string(),
        receipts: vec![LiveReadReceipt {
            user_id: bob.to_string(),
            display_name: None,
            original_display_label: String::new(),
            avatar: None,
            timestamp_ms: Some(1),
        }],
    }];
    let mut state = AppState {
        session: SessionState::Ready(session.info.clone()),
        ..AppState::default()
    };
    reduce(
        &mut state,
        AppAction::LiveRoomReceiptsWindowReconciled {
            room_id: room_id.to_string(),
            scoped_event_ids: Vec::new(),
            receipts_by_event: receipts.clone(),
        },
    );
    state.profile.users.insert(
        bob.to_string(),
        UserProfile {
            user_id: bob.to_string(),
            display_name: Some("Global cache".to_owned()),
            display_label: "Global cache".to_owned(),
            original_display_label: "Global cache".to_owned(),
            mention_search_terms: Vec::new(),
            avatar: None,
        },
    );
    assert_eq!(
        state.live_signals.rooms[room_id.as_str()].receipts_by_event[&receipts[0].event_id].readers
            [0]
        .display_name
        .as_deref(),
        Some("Unknown user"),
        "the production batch must refresh an already-projected Unknown receipt"
    );

    let key = TimelineKey::room(AccountKey(ALICE.to_string()), room_id.to_string());
    let generations = Arc::new(TimelineActorGenerationGate::default());
    let actor_generation = generations.activate_after_quiescence(&key).await.generation;
    let (action_tx, mut action_rx) = mpsc::channel(1);
    assert!(
        emit_live_receipt_observation_actions(
            session.as_ref(),
            &action_tx,
            &generations,
            &key,
            actor_generation,
            room_id.as_str(),
            receipts.clone(),
        )
        .await
    );
    let action_batch = action_rx.recv().await.expect("receipt action batch");
    assert!(matches!(
        action_batch.as_slice(),
        [
            AppAction::LiveRoomProfilesObserved { profiles, .. },
            AppAction::UserProfilesUpdated { profiles: cached },
            AppAction::LiveRoomReceiptSummariesUpdated { .. },
        ] if profiles.iter().any(|profile| {
            profile.user_id == bob.as_str()
                && profile.display_name.as_deref() == Some("Relevant room member")
        }) && cached.iter().any(|profile| {
            profile.user_id == bob.as_str()
                && profile.display_name.as_deref() == Some("Relevant room member")
        })
    ));

    for action in action_batch {
        reduce(&mut state, action);
    }
    assert_eq!(
        state.live_signals.rooms[room_id.as_str()].receipts_by_event[&receipts[0].event_id].readers
            [0]
        .display_name
        .as_deref(),
        Some("Relevant room member"),
        "the relevant room profile must beat the global cache"
    );
}

#[tokio::test]
async fn production_receipt_diff_delivery_uses_global_cache_when_local_lookup_misses() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    use koushi_state::{AppState, reduce};
    use matrix_sdk::ruma::{event_id, room_id};
    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    use matrix_sdk_test::ALICE;

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!receipt-cache-fallback:example.test");
    server.sync_joined_room(&client, room_id).await;
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client,
        SessionInfo {
            homeserver: "http://example.invalid".to_owned(),
            user_id: ALICE.to_string(),
            device_id: "DEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    ));
    let bob = "@bob:example.test";
    let receipts = vec![LiveEventReceipts {
        event_id: event_id!("$receipt-cache-fallback:example.test").to_string(),
        receipts: vec![LiveReadReceipt {
            user_id: bob.to_owned(),
            display_name: None,
            original_display_label: String::new(),
            avatar: None,
            timestamp_ms: Some(2),
        }],
    }];
    let mut state = AppState {
        session: SessionState::Ready(session.info.clone()),
        ..AppState::default()
    };
    state.profile.users.insert(
        bob.to_owned(),
        UserProfile {
            user_id: bob.to_owned(),
            display_name: Some("Global cache".to_owned()),
            display_label: "Global cache".to_owned(),
            original_display_label: "Global cache".to_owned(),
            mention_search_terms: Vec::new(),
            avatar: None,
        },
    );

    let key = TimelineKey::room(AccountKey(ALICE.to_string()), room_id.to_string());
    let generations = Arc::new(TimelineActorGenerationGate::default());
    let actor_generation = generations.activate_after_quiescence(&key).await.generation;
    let (action_tx, mut action_rx) = mpsc::channel(1);
    assert!(
        emit_live_receipt_observation_actions(
            session.as_ref(),
            &action_tx,
            &generations,
            &key,
            actor_generation,
            room_id.as_str(),
            receipts.clone(),
        )
        .await
    );
    let action_batch = action_rx.recv().await.expect("receipt fallback batch");
    assert!(matches!(
        action_batch.as_slice(),
        [AppAction::LiveRoomReceiptSummariesUpdated { .. }]
    ));
    for action in action_batch {
        reduce(&mut state, action);
    }
    assert_eq!(
        state.live_signals.rooms[room_id.as_str()].receipts_by_event[&receipts[0].event_id].readers
            [0]
        .display_name
        .as_deref(),
        Some("Global cache")
    );
}

#[tokio::test]
async fn scoped_receipt_window_prepares_only_its_selected_profiles() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = matrix_sdk::ruma::room_id!("!bounded-profile:example.test");
    server.sync_joined_room(&client, room_id).await;
    let session = MatrixClientSession::from_client_for_testing(
        client,
        SessionInfo {
            homeserver: "http://example.invalid".into(),
            user_id: matrix_sdk_test::ALICE.to_string(),
            device_id: "DEVICE".into(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    );
    let mut window = crate::timeline::RawReceiptWindow {
        total_count: 1500,
        start: 0,
        profiles: Vec::new(),
        owner: None,
        epoch: std::sync::Weak::new(),
        receipts: (0..3)
            .map(|i| LiveReadReceipt {
                user_id: format!("@reader-{i}:example.test"),
                display_name: None,
                original_display_label: String::new(),
                avatar: None,
                timestamp_ms: Some(i),
            })
            .collect(),
    };
    let before = koushi_diagnostics::test_support::detail_snapshot()
        .records
        .len();
    let (mut reply, _receiver) = tokio::sync::oneshot::channel();
    assert!(
        crate::timeline::item_projection::prepare_receipt_window_profiles(
            &session,
            room_id.as_str(),
            &mut window,
            &mut reply,
        )
        .await
    );
    assert_eq!(window.total_count, 1500);
    assert_eq!(window.receipts.len(), 3);
    let mut profiles = koushi_state::ProfileState::default();
    profiles
        .local_aliases
        .insert(window.receipts[0].user_id.clone(), "Current alias".into());
    window.resolve_profiles(&profiles, room_id.as_str(), None);
    assert_eq!(
        window.receipts[0].display_name.as_deref(),
        Some("Current alias")
    );
    assert_eq!(window.total_count, 1500);
    let snapshot = koushi_diagnostics::test_support::detail_snapshot();
    let requested = snapshot
        .records
        .iter()
        .skip(before)
        .filter(|record| record.event.source == "core.read_receipt_profile")
        .flat_map(|record| &record.event.fields)
        .find_map(|field| match field.value {
            DiagnosticValue::Count(count) if field.key == "requested_user_count" => Some(count),
            _ => None,
        })
        .expect("production profile lookup count");
    assert_eq!(requested, 3);
    let before_cancel = snapshot.records.len();
    let (mut cancelled, receiver) = tokio::sync::oneshot::channel();
    drop(receiver);
    assert!(
        !crate::timeline::item_projection::prepare_receipt_window_profiles(
            &session,
            room_id.as_str(),
            &mut window,
            &mut cancelled,
        )
        .await
    );
    let after = koushi_diagnostics::test_support::detail_snapshot();
    assert!(
        !after
            .records
            .iter()
            .skip(before_cancel)
            .any(|record| record.event.source == "core.read_receipt_profile")
    );
}

#[tokio::test]
async fn compact_receipt_profile_lookup_is_bounded_for_1500_readers() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    use koushi_state::{AppState, SessionAuthenticationMethod, SessionState, reduce};
    use matrix_sdk::ruma::room_id;
    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    use matrix_sdk_test::ALICE;

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!receipt-scale:example.test");
    server.sync_joined_room(&client, room_id).await;
    let session = MatrixClientSession::from_client_for_testing(
        client,
        SessionInfo {
            homeserver: "http://example.invalid".to_owned(),
            user_id: ALICE.to_string(),
            device_id: "DEVICE".to_owned(),
            authentication_method: SessionAuthenticationMethod::Unknown,
        },
    );
    let event_id = "$receipt-scale:example.test";
    let receipts = vec![LiveEventReceipts {
        event_id: event_id.to_owned(),
        receipts: (0..1500)
            .map(|index| LiveReadReceipt {
                user_id: format!("@reader-{index}:example.test"),
                display_name: None,
                original_display_label: String::new(),
                avatar: None,
                timestamp_ms: Some(index),
            })
            .collect(),
    }];
    let key = TimelineKey::room(AccountKey(ALICE.to_string()), room_id.to_string());
    let generations = Arc::new(TimelineActorGenerationGate::default());
    let actor_generation = generations.activate_after_quiescence(&key).await.generation;
    let (action_tx, mut action_rx) = mpsc::channel(1);
    let records_before = koushi_diagnostics::test_support::detail_snapshot()
        .records
        .len();
    assert!(
        emit_live_receipt_observation_actions(
            &session,
            &action_tx,
            &generations,
            &key,
            actor_generation,
            room_id.as_str(),
            receipts,
        )
        .await
    );
    let mut state = AppState {
        session: SessionState::Ready(session.info.clone()),
        ..AppState::default()
    };
    for action in action_rx.recv().await.expect("receipt actions") {
        reduce(&mut state, action);
    }
    assert_eq!(
        state.live_signals.rooms[room_id.as_str()].receipts_by_event[event_id].total_count,
        1500
    );

    let snapshot = koushi_diagnostics::test_support::detail_snapshot();
    let requested = snapshot
        .records
        .iter()
        .skip(records_before)
        .filter(|record| record.event.source == "core.read_receipt_profile")
        .flat_map(|record| &record.event.fields)
        .find_map(|field| match field.value {
            DiagnosticValue::Count(count) if field.key == "requested_user_count" => Some(count),
            _ => None,
        })
        .expect("production profile-lookup count");
    // No full-reader surface is open: five or more readers show three plus a count.
    assert!(
        requested <= 3,
        "compact receipt profile lookup must be bounded, requested {requested}"
    );
}

#[tokio::test]
async fn production_receipt_diff_delivery_sends_receipts_when_local_lookup_fails() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    use koushi_state::SessionAuthenticationMethod;
    use matrix_sdk::ruma::event_id;
    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    use matrix_sdk_test::ALICE;

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client,
        SessionInfo {
            homeserver: "http://example.invalid".to_owned(),
            user_id: ALICE.to_string(),
            device_id: "DEVICE".to_owned(),
            authentication_method: SessionAuthenticationMethod::Unknown,
        },
    ));
    let receipts = vec![LiveEventReceipts {
        event_id: event_id!("$receipt-lookup-failure:example.test").to_string(),
        receipts: vec![LiveReadReceipt {
            user_id: "@bob:example.test".to_owned(),
            display_name: None,
            original_display_label: String::new(),
            avatar: None,
            timestamp_ms: Some(3),
        }],
    }];
    let key = TimelineKey::room(
        AccountKey(ALICE.to_string()),
        "!receipt-failure:example.test",
    );
    let generations = Arc::new(TimelineActorGenerationGate::default());
    let actor_generation = generations.activate_after_quiescence(&key).await.generation;
    let (action_tx, mut action_rx) = mpsc::channel(1);
    let records_before = koushi_diagnostics::test_support::detail_snapshot()
        .records
        .len();
    assert!(
        emit_live_receipt_observation_actions(
            session.as_ref(),
            &action_tx,
            &generations,
            &key,
            actor_generation,
            "not-a-room-id",
            receipts,
        )
        .await
    );
    let action_batch = action_rx.recv().await.expect("failed lookup receipt batch");
    assert!(matches!(
        action_batch.as_slice(),
        [AppAction::LiveRoomReceiptSummariesUpdated { .. }]
    ));
    assert!(
        koushi_diagnostics::test_support::detail_snapshot()
            .records
            .iter()
            .skip(records_before)
            .any(|record| {
                record.event.source == "core.read_receipt_profile"
                    && record.event.stage == "local_lookup"
                    && record.event.fields.iter().any(|field| {
                        field.key == "lookup_outcome"
                            && field.value == DiagnosticValue::Token("failed")
                    })
            }),
        "lookup failures must record a sanitized outcome"
    );
}

#[tokio::test]
async fn stale_production_receipt_diff_result_is_discarded_after_generation_replacement() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    use koushi_state::SessionAuthenticationMethod;
    use matrix_sdk::ruma::event_id;
    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    use matrix_sdk_test::ALICE;

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client,
        SessionInfo {
            homeserver: "http://example.invalid".to_owned(),
            user_id: ALICE.to_string(),
            device_id: "DEVICE".to_owned(),
            authentication_method: SessionAuthenticationMethod::Unknown,
        },
    ));
    let receipts = vec![LiveEventReceipts {
        event_id: event_id!("$receipt-stale:example.test").to_string(),
        receipts: vec![LiveReadReceipt {
            user_id: "@bob:example.test".to_owned(),
            display_name: None,
            original_display_label: String::new(),
            avatar: None,
            timestamp_ms: Some(4),
        }],
    }];
    let key = TimelineKey::room(AccountKey(ALICE.to_string()), "!receipt-stale:example.test");
    let generations = Arc::new(TimelineActorGenerationGate::default());
    let stale_generation = generations.activate_after_quiescence(&key).await.generation;
    let (action_tx, mut action_rx) = mpsc::channel(1);
    action_tx
        .send(vec![AppAction::TypingUsersUpdated {
            room_id: "!occupied:example.test".to_owned(),
            user_ids: Vec::new(),
        }])
        .await
        .expect("fill action channel");

    let delivery = tokio::spawn({
        let session = Arc::clone(&session);
        let action_tx = action_tx.clone();
        let generations = Arc::clone(&generations);
        let key = key.clone();
        async move {
            emit_live_receipt_observation_actions(
                session.as_ref(),
                &action_tx,
                &generations,
                &key,
                stale_generation,
                "not-a-room-id",
                receipts,
            )
            .await
        }
    });
    tokio::task::yield_now().await;
    let replacement_generation = generations.activate_after_quiescence(&key).await.generation;
    assert_ne!(replacement_generation, stale_generation);
    assert!(matches!(
        action_rx.recv().await,
        Some(actions) if matches!(
            actions.as_slice(),
            [AppAction::TypingUsersUpdated { room_id, .. }] if room_id == "!occupied:example.test"
        )
    ));
    assert!(!delivery.await.expect("stale delivery task"));
    assert!(
        action_rx.try_recv().is_err(),
        "a stale actor generation must not publish the receipt batch"
    );
}
