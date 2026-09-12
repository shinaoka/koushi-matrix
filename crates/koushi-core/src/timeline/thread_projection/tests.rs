use super::super::test_source::item_body;

use std::collections::{HashMap, HashSet};

use std::sync::{Arc, Mutex};
use std::time::Duration;

use koushi_state::{AppAction, OperationFailureKind};

use matrix_sdk::test_utils::mocks::MatrixMockServer;
use matrix_sdk_ui::timeline::{
    EventItemOrigin, TimelineDetails, TimelineEventItemId, TimelineItemContent,
};
use tokio::sync::{broadcast, mpsc};

use crate::executor;
use koushi_protocol::event::{
    CoreEvent, ThreadSummaryDto, TimelineDiff, TimelineEvent, TimelineItem, TimelineItemId,
    TimelineMediaKind, TimelineMessageActions,
};

use crate::threads_list::{
    AggregateRefreshCause, ThreadRootProjectionActivity, ThreadRootProjectionDecision,
    ThreadRootProjectionService,
};
use koushi_protocol::ids::{TimelineBatchId, TimelineGeneration, TimelineKey, TimelineKind};

use std::future::poll_fn;

use matrix_sdk::ruma::{OwnedUserId, uint};

use super::super::actor::{
    ThreadSummaryProjectionIngress, TimelineActorHandle, TimelineActorMessage,
    emit_app_action_reliable,
};
use super::super::display_projection::apply_timeline_diffs_to_items;
use super::super::item_projection::{
    megolm_session_fingerprint, thread_root_from_original_json, thread_summary_from_sdk,
    timeline_item_event_id, timeline_item_should_be_hidden_for_key,
};
use super::super::navigation::{TimelineActorGenerationGate, emit_timeline_events_for_generation};
use super::super::outbound_send::{
    newest_provable_receipt_event_id, thread_activity_observed_action,
    thread_activity_observed_action_for_batch,
};
use super::super::test_support::{
    focused_key, live_tail_test_manager, room_key, test_timeline_actor_handle, thread_key,
    timeline_item,
};
use crate::threads_list::AuthoritativeThreadAggregate;

use super::{
    ThreadAttentionBatchProvenance, ThreadAttentionCounters, ThreadAttentionObservation,
    ThreadAttentionTracker, ThreadRootProjectionFetchRegistry, overlay_thread_summary_diff,
    reaction_groups_from_cached_relation_events, thread_attention_observation_from_event_origin,
    thread_root_item_with_authoritative_aggregate, thread_root_projection_activity_from_item,
    thread_root_projection_item_from_raw, thread_summary_affected_root_event_ids,
};

#[test]
fn thread_activity_promotion_requires_a_matching_event_backed_reply() {
    let key = thread_key();
    let matching = thread_reply_item("$reply:test", "@b:test", "$root:test");
    assert_eq!(
        thread_activity_observed_action(&key, std::slice::from_ref(&matching)),
        Some(AppAction::ThreadActivityObserved {
            room_id: "!r:test".to_owned(),
            root_event_id: "$root:test".to_owned(),
        })
    );
    let live_batch = ThreadAttentionBatchProvenance::from_timeline_items(
        std::slice::from_ref(&matching),
        ThreadAttentionObservation::Live,
    );
    assert_eq!(
        thread_activity_observed_action_for_batch(
            &key,
            std::slice::from_ref(&matching),
            &live_batch,
        ),
        Some(AppAction::ThreadActivityObserved {
            room_id: "!r:test".to_owned(),
            root_event_id: "$root:test".to_owned(),
        })
    );
    assert_eq!(
        thread_activity_observed_action_for_batch(
            &key,
            std::slice::from_ref(&matching),
            &ThreadAttentionBatchProvenance::default(),
        ),
        None
    );

    let mut local_echo = matching;
    local_echo.id = TimelineItemId::Transaction {
        transaction_id: "txn".to_owned(),
    };
    assert_eq!(thread_activity_observed_action(&key, &[local_echo]), None);
    assert_eq!(
        thread_activity_observed_action(
            &key,
            &[thread_reply_item(
                "$other:test",
                "@b:test",
                "$other-root:test",
            )],
        ),
        None
    );
    assert_eq!(
        thread_activity_observed_action(
            &room_key(),
            &[thread_reply_item("$reply:test", "@b:test", "$root:test",)]
        ),
        None
    );
}

#[test]
fn thread_and_focused_items_do_not_claim_room_canonical_summary_ownership() {
    let service = Arc::new(Mutex::new(ThreadRootProjectionService::default()));
    let mut root = timeline_item("$root:test", Some("root"), "@root:test", false);
    root.thread_summary = Some(ThreadSummaryDto {
        reply_count: 1,
        latest_event_id: Some("$reply:test".to_owned()),
        latest_sender: None,
        latest_sender_label: None,
        latest_body_preview: Some("reply".to_owned()),
        latest_timestamp_ms: Some(100),
    });
    super::seed_thread_summary_item(&service, &thread_key(), &root);
    assert!(
        service
            .lock()
            .expect("service lock")
            .current_aggregate("!r:test", "$root:test")
            .is_none()
    );
}

#[test]
fn newer_sdk_summary_is_detected_before_overlay_and_repaired_by_exact_aggregate() {
    let key = room_key();
    let service = Arc::new(Mutex::new(ThreadRootProjectionService::default()));
    let mut root_a = timeline_item("$root:test", Some("root"), "@root:test", false);
    root_a.thread_summary = Some(ThreadSummaryDto {
        reply_count: 1,
        latest_event_id: Some("$reply-a:test".to_owned()),
        latest_sender: Some("@a:test".to_owned()),
        latest_sender_label: Some("A".to_owned()),
        latest_body_preview: Some("A".to_owned()),
        latest_timestamp_ms: Some(100),
    });
    super::seed_thread_summary_item(&service, &key, &root_a);

    let mut root_b = root_a.clone();
    root_b.thread_summary = Some(ThreadSummaryDto {
        reply_count: 2,
        latest_event_id: Some("$reply-b:test".to_owned()),
        latest_sender: Some("@b:test".to_owned()),
        latest_sender_label: Some("B".to_owned()),
        latest_body_preview: Some("B".to_owned()),
        latest_timestamp_ms: Some(200),
    });
    let raw_diff = TimelineDiff::Set {
        index: 0,
        item: root_b.clone(),
    };
    let mut raw_after = vec![root_a.clone()];
    apply_timeline_diffs_to_items(&mut raw_after, std::slice::from_ref(&raw_diff));
    assert_eq!(
        thread_summary_affected_root_event_ids(&key, &[root_a.clone()], &raw_after),
        HashSet::from(["$root:test".to_owned()])
    );

    // The bundled identity is provisional (it may be an edit event), so
    // overlay retains A until the exact event-cache aggregate validates B.
    super::seed_thread_summary_diff(&service, &key, &raw_diff);
    let mut overlaid_diff = raw_diff;
    overlay_thread_summary_diff(&service, &key, &mut overlaid_diff);
    let TimelineDiff::Set { item, .. } = &overlaid_diff else {
        panic!("expected root Set")
    };
    assert_eq!(
        item.thread_summary
            .as_ref()
            .and_then(|summary| summary.latest_event_id.as_deref()),
        Some("$reply-a:test")
    );

    let activity = service
        .lock()
        .expect("service lock")
        .activity_for_root(key.room_id(), "$root:test")
        .expect("tracked root");
    let refresh = service
        .lock()
        .expect("service lock")
        .schedule_aggregate_refresh_with_canonical_root(
            &activity,
            AggregateRefreshCause::CanonicalBatch,
            true,
            true,
            false,
        )
        .expect("aggregate refresh");
    assert!(matches!(
        service.lock().expect("service lock").complete_refresh(
            &refresh,
            Ok(
                crate::threads_list::ThreadRootProjectionRefreshResult::Aggregate(
                    AuthoritativeThreadAggregate {
                        reply_count: 2,
                        latest_event_id: Some("$reply-b:test".to_owned()),
                        latest_sender: Some("@b:test".to_owned()),
                        latest_sender_label: Some("B".to_owned()),
                        latest_body_preview: Some("B".to_owned()),
                        latest_timestamp_ms: Some(200),
                    },
                )
            ),
        ),
        crate::threads_list::ThreadRootProjectionCompletion::Updated(_)
    ));
    let mut validated_diff = TimelineDiff::Set {
        index: 0,
        item: root_b,
    };
    overlay_thread_summary_diff(&service, &key, &mut validated_diff);
    let TimelineDiff::Set { item, .. } = validated_diff else {
        panic!("expected validated root Set")
    };
    assert_eq!(
        item.thread_summary
            .as_ref()
            .and_then(|summary| summary.latest_event_id.as_deref()),
        Some("$reply-b:test")
    );
    assert_eq!(
        item.thread_summary
            .as_ref()
            .map(|summary| summary.reply_count),
        Some(2)
    );
}

#[tokio::test]
async fn canonical_completion_bypasses_a_full_room_mailbox_via_projection_watch() {
    let key = room_key();
    let (actor_tx, _actor_rx) = mpsc::channel(1);
    actor_tx
        .try_send(TimelineActorMessage::OwnReadReceiptChanged)
        .expect("fill ordinary Room actor mailbox");
    let (projection, projection_rx) = ThreadSummaryProjectionIngress::channel();
    let mut manager = live_tail_test_manager(HashMap::from([(
        key.clone(),
        TimelineActorHandle {
            tx: actor_tx,
            control_tx: None,
            thread_summary_projection: projection,
            position_rx: None,
            task: None,
            auxiliary_tasks: Vec::new(),
            subscription_generation: None,
            enqueue_context: None,
        },
    )]));
    let actor_generation = manager
        .timeline_actor_generations
        .activate_after_quiescence(&key)
        .await
        .generation;
    let activity = ThreadRootProjectionActivity {
        room_id: key.room_id().to_owned(),
        root_event_id: "$root:test".to_owned(),
        activity_event_id: "$reply-b:test".to_owned(),
        activity_timestamp_ms: Some(200),
        activity_sender: Some("@b:test".to_owned()),
        activity_sender_label: Some("B".to_owned()),
        activity_body_preview: Some("B".to_owned()),
    };
    let refresh = {
        let mut service = manager
            .thread_root_projection_service
            .lock()
            .expect("service lock");
        assert!(matches!(
            service.observe(activity.clone()),
            ThreadRootProjectionDecision::StartFetch(_)
        ));
        service.set_canonical_root_event_ids(
            key.room_id(),
            &HashSet::from([activity.root_event_id.clone()]),
        );
        service
            .schedule_aggregate_refresh_with_canonical_root(
                &activity,
                AggregateRefreshCause::SelectedActivity,
                true,
                true,
                false,
            )
            .expect("canonical refresh")
    };
    manager.thread_root_projection_fetches.insert(
        activity.room_id.clone(),
        activity.root_event_id.clone(),
        actor_generation,
        Some(refresh.summary_revision),
        executor::spawn(async { std::future::pending::<()>().await }),
    );

    executor::timeout(
        Duration::from_millis(100),
        manager.handle_aggregate_refresh_finished(
            key,
            actor_generation,
            refresh,
            Ok(
                crate::threads_list::ThreadRootProjectionRefreshResult::Aggregate(
                    AuthoritativeThreadAggregate {
                        reply_count: 2,
                        latest_event_id: Some(activity.activity_event_id.clone()),
                        latest_sender: activity.activity_sender.clone(),
                        latest_sender_label: activity.activity_sender_label.clone(),
                        latest_body_preview: activity.activity_body_preview.clone(),
                        latest_timestamp_ms: activity.activity_timestamp_ms,
                    },
                ),
            ),
        ),
    )
    .await
    .expect("manager must not wait for ordinary Room actor capacity");
    let pending = projection_rx.borrow();
    let wake = pending
        .get(&activity.root_event_id)
        .expect("accepted canonical completion wake");
    assert!(matches!(
        wake,
        super::ThreadSummaryProjectionWake::Updated {
            activity_revision: 1,
            summary_revision: 1,
            ..
        }
    ));
}

#[tokio::test]
async fn actor_owner_generation_remains_monotonic_across_manager_gate_recreation() {
    let key = focused_key();
    let first_gate = TimelineActorGenerationGate::default();
    let first = first_gate.activate_after_quiescence(&key).await.generation;
    drop(first_gate);

    let replacement_gate = TimelineActorGenerationGate::default();
    let replacement = replacement_gate
        .activate_after_quiescence(&key)
        .await
        .generation;
    assert!(replacement > first);
}

#[tokio::test]
async fn stale_actor_generation_cannot_emit_any_timeline_event_after_replacement() {
    let key = room_key();
    let actor_generations = Arc::new(TimelineActorGenerationGate::default());
    let old_generation = actor_generations
        .activate_after_quiescence(&key)
        .await
        .generation;
    let old_lease = actor_generations
        .try_acquire(&key, old_generation)
        .expect("old actor lease");
    let replacement_gate = actor_generations.clone();
    let replacement_key = key.clone();
    let replacement = tokio::spawn(async move {
        replacement_gate
            .activate_after_quiescence(&replacement_key)
            .await
    });
    for _ in 0..10 {
        if actor_generations
            .try_acquire(&key, old_generation)
            .is_none()
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        actor_generations
            .try_acquire(&key, old_generation)
            .is_none()
    );
    drop(old_lease);
    let new_generation = replacement.await.expect("replacement task").generation;

    let (event_tx, mut event_rx) = broadcast::channel(8);
    assert!(!emit_timeline_events_for_generation(
        &event_tx,
        &actor_generations,
        &key,
        old_generation,
        vec![TimelineEvent::ItemsUpdated {
            key: key.clone(),
            generation: TimelineGeneration(0),
            batch_id: TimelineBatchId(1),
            diffs: vec![TimelineDiff::PushBack {
                item: timeline_item("$old-diff:test", Some("old"), "@a:test", false),
            }],
        }],
    ));
    assert!(!emit_timeline_events_for_generation(
        &event_tx,
        &actor_generations,
        &key,
        old_generation,
        vec![TimelineEvent::InitialItems {
            request_id: None,
            cause_request_id: None,
            key: key.clone(),
            actor_generation: old_generation,
            generation: TimelineGeneration(0),
            items: vec![timeline_item(
                "$old-initial:test",
                Some("old"),
                "@a:test",
                false
            )],
        }],
    ));
    assert!(matches!(
        event_rx.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));

    assert!(emit_timeline_events_for_generation(
        &event_tx,
        &actor_generations,
        &key,
        new_generation,
        vec![TimelineEvent::InitialItems {
            request_id: None,
            cause_request_id: None,
            key: key.clone(),
            actor_generation: new_generation,
            generation: TimelineGeneration(0),
            items: vec![timeline_item(
                "$new-initial:test",
                Some("new"),
                "@a:test",
                false
            )],
        }],
    ));
    assert!(matches!(
        event_rx.try_recv(),
        Ok(CoreEvent::Timeline(TimelineEvent::InitialItems { items, .. }))
            if items.iter().any(|item| timeline_item_event_id(item) == Some("$new-initial:test"))
    ));
    assert!(matches!(
        event_rx.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
}

fn timeline_message_item(event_id: &str, sender: &str) -> TimelineItem {
    TimelineItem {
        request_state: None,
        id: TimelineItemId::Event {
            event_id: event_id.to_owned(),
        },
        sender: Some(sender.to_owned()),
        sender_label: None,
        sender_avatar: None,
        body: Some("body".to_owned()),
        notice_i18n: None,
        message_kind: Default::default(),
        spoiler_spans: Vec::new(),
        timestamp_ms: Some(1),
        in_reply_to_event_id: None,
        formatted: None,
        reply_quote: None,
        thread_root: None,
        thread_summary: None,
        media: None,
        link_previews: None,
        link_ranges: Vec::new(),
        mentioned_user_ids: Vec::new(),
        reactions: Vec::new(),
        can_react: true,
        is_redacted: false,
        is_hidden: false,
        can_redact: false,
        is_edited: false,
        can_edit: false,
        actions: TimelineMessageActions::default(),
        send_state: None,
        unable_to_decrypt: None,
        display_metadata: None,
    }
}

fn thread_reply_item(event_id: &str, sender: &str, root_event_id: &str) -> TimelineItem {
    TimelineItem {
        thread_root: Some(root_event_id.to_owned()),
        ..timeline_message_item(event_id, sender)
    }
}

#[test]
fn old_root_reply_reaches_bounded_room_projection_hydration_without_pagination() {
    let mut reply = timeline_item(
        "$latest-reply:test",
        Some("new reply"),
        "@alice:test",
        false,
    );
    reply.timestamp_ms = Some(1_700_000_100_000);
    reply.thread_root = Some("$old-root:test".to_owned());

    let activity = thread_root_projection_activity_from_item("!room:test", &reply)
        .expect("a canonical Room reply must be observable for root hydration");
    assert_eq!(activity.root_event_id, "$old-root:test");
    assert_eq!(activity.activity_event_id, "$latest-reply:test");
    assert_eq!(activity.activity_timestamp_ms, Some(1_700_000_100_000));
}

#[tokio::test]
async fn root_projection_actions_wait_for_reducer_capacity_instead_of_dropping() {
    let (action_tx, mut action_rx) = mpsc::channel(1);
    action_tx
        .try_send(vec![AppAction::ThreadRootProjectionsCleared {
            room_id: "!already-buffered:test".to_owned(),
        }])
        .expect("fill the reducer channel");

    let reliable_tx = action_tx.clone();
    let delivery = tokio::spawn(async move {
        emit_app_action_reliable(
            &reliable_tx,
            AppAction::ThreadRootProjectionsCleared {
                room_id: "!must-arrive:test".to_owned(),
            },
        )
        .await
    });
    tokio::task::yield_now().await;
    assert!(
        !delivery.is_finished(),
        "the reliable sender must wait behind a full channel, not discard the projection transition"
    );
    let _ = action_rx.recv().await.expect("drain buffered action");
    assert!(delivery.await.expect("delivery task"));
    assert!(matches!(
        action_rx.recv().await,
        Some(actions) if matches!(
            actions.as_slice(),
            [AppAction::ThreadRootProjectionsCleared { room_id }]
                if room_id == "!must-arrive:test"
        )
    ));
}

#[tokio::test]
async fn root_projection_fetch_registry_aborts_room_workers_and_rejects_late_completion() {
    struct CancellationProbe(Option<tokio::sync::oneshot::Sender<()>>);

    impl Drop for CancellationProbe {
        fn drop(&mut self) {
            if let Some(tx) = self.0.take() {
                let _ = tx.send(());
            }
        }
    }

    let (cancelled_tx, cancelled_rx) = tokio::sync::oneshot::channel();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let task = executor::spawn(async move {
        let _probe = CancellationProbe(Some(cancelled_tx));
        let _ = started_tx.send(());
        std::future::pending::<()>().await;
    });
    let mut registry = ThreadRootProjectionFetchRegistry::default();
    registry.insert(
        "!room:test".to_owned(),
        "$root:test".to_owned(),
        7,
        None,
        task,
    );
    started_rx
        .await
        .expect("worker must be in flight before cancellation");

    assert_eq!(registry.abort_room("!room:test").await, 1);
    tokio::time::timeout(Duration::from_secs(1), cancelled_rx)
        .await
        .expect("abort must end the in-flight hydration worker")
        .expect("worker cancellation probe should be delivered");
    assert!(
        !registry.take_completion("!room:test", "$root:test", 7, None),
        "a completion queued before unsubscribe must not publish a stale terminal state"
    );
}

#[tokio::test]
async fn aggregate_start_preserves_fetch_finished_worker_and_failed_hydration_terminal() {
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
    let mut manager =
        live_tail_test_manager(HashMap::from([(key.clone(), test_timeline_actor_handle())]));
    manager.session = Some(session);
    let actor_generation = manager
        .timeline_actor_generations
        .activate_after_quiescence(&key)
        .await
        .generation;
    let activity = ThreadRootProjectionActivity {
        room_id: key.room_id().to_owned(),
        root_event_id: "$failed-root:test".to_owned(),
        activity_event_id: "$reply:test".to_owned(),
        activity_timestamp_ms: Some(100),
        activity_sender: None,
        activity_sender_label: None,
        activity_body_preview: None,
    };
    let refresh = {
        let mut service = manager
            .thread_root_projection_service
            .lock()
            .expect("service lock");
        assert!(matches!(
            service.observe(activity.clone()),
            ThreadRootProjectionDecision::StartFetch(_)
        ));
        let refresh = service
            .schedule_aggregate_refresh(
                &activity,
                AggregateRefreshCause::InitialHydration,
                true,
                false,
            )
            .expect("initial aggregate refresh");
        service.mark_failed(&activity, OperationFailureKind::NotFound);
        refresh
    };

    // FetchFinished has removed hydration and started this exact aggregate
    // worker before the original StartAggregateRefresh reaches the FIFO.
    manager.thread_root_projection_fetches.insert(
        activity.room_id.clone(),
        activity.root_event_id.clone(),
        actor_generation,
        None,
        executor::spawn(async { std::future::pending::<()>().await }),
    );
    assert!(manager.thread_root_projection_fetches.take_completion(
        &activity.room_id,
        &activity.root_event_id,
        actor_generation,
        None,
    ));
    manager.thread_root_projection_fetches.insert(
        activity.room_id.clone(),
        activity.root_event_id.clone(),
        actor_generation,
        Some(refresh.summary_revision),
        executor::spawn(async { std::future::pending::<()>().await }),
    );
    assert!(manager.thread_root_projection_fetches.contains_aggregate(
        &activity.room_id,
        &activity.root_event_id,
        actor_generation,
        refresh.summary_revision,
    ));

    manager
        .handle_aggregate_refresh_start(key.clone(), actor_generation, None, vec![refresh.clone()])
        .await;
    assert!(manager.thread_root_projection_fetches.contains_aggregate(
        &activity.room_id,
        &activity.root_event_id,
        actor_generation,
        refresh.summary_revision,
    ));
    assert!(!manager.thread_root_projection_fetches.contains_hydration(
        &activity.room_id,
        &activity.root_event_id,
        actor_generation,
    ));

    assert!(manager.thread_root_projection_fetches.take_completion(
        &activity.room_id,
        &activity.root_event_id,
        actor_generation,
        Some(refresh.summary_revision),
    ));
    assert!(matches!(
        manager
            .thread_root_projection_service
            .lock()
            .expect("service lock")
            .complete_refresh(&refresh, Err(OperationFailureKind::Network)),
        crate::threads_list::ThreadRootProjectionCompletion::Updated(record)
            if record.failure_kind() == Some(OperationFailureKind::Network)
    ));
    let service = manager
        .thread_root_projection_service
        .lock()
        .expect("service lock");
    assert!(!service.has_pending_attempt(&activity));
    drop(service);
    manager
        .handle_aggregate_refresh_start(key, actor_generation, None, vec![refresh])
        .await;
    assert!(!manager.thread_root_projection_fetches.contains_hydration(
        &activity.room_id,
        &activity.root_event_id,
        actor_generation,
    ));
}

#[test]
fn loaded_old_root_raw_event_projects_renderable_snapshot_with_latest_activity_identity() {
    let activity = ThreadRootProjectionActivity {
        room_id: "!room:test".to_owned(),
        root_event_id: "$old-root:test".to_owned(),
        activity_event_id: "$latest-reply:test".to_owned(),
        activity_timestamp_ms: Some(1_700_000_100_000),
        activity_sender: Some("@latest:test".to_owned()),
        activity_sender_label: Some("Latest".to_owned()),
        activity_body_preview: Some("live reply preview".to_owned()),
    };
    let raw = serde_json::json!({
            "type": "m.room.message",
            "event_id": "$old-root:test",
            "sender": "@alice:test",
            "origin_server_ts": 1_700_000_000_000_u64,
            "content": { "msgtype": "m.text", "body": "old root body" },
            "unsigned": {
                "m.relations": {
                    "m.thread": {
                        "count": 3,
                        "latest_event": {
                            "event_id": "$stale-latest:test",
                            "sender": "@bob:test",
                            "origin_server_ts": 1_700_000_050_000_u64,
                            "content": { "body": "stale preview" }
                    }
                }
            }
        }
    });

    let item = thread_root_projection_item_from_raw(&room_key(), None, &activity, raw)
        .expect("valid loaded root must yield a renderable snapshot");
    assert_eq!(timeline_item_event_id(&item), Some("$old-root:test"));
    assert_eq!(item.body.as_deref(), Some("old root body"));
    assert_eq!(item.timestamp_ms, Some(1_700_000_000_000));
    assert_eq!(item.thread_root, None);
    assert_eq!(
        item.thread_summary
            .as_ref()
            .and_then(|summary| summary.latest_event_id.as_deref()),
        Some("$stale-latest:test"),
        "raw bundled relation data is only provisional before Task A resolution"
    );
    assert_eq!(
        item.thread_summary
            .as_ref()
            .map(|summary| summary.reply_count),
        Some(3)
    );

    let authoritative = thread_root_item_with_authoritative_aggregate(
        &item,
        &AuthoritativeThreadAggregate {
            reply_count: 4,
            latest_event_id: Some(activity.activity_event_id.clone()),
            latest_sender: activity.activity_sender.clone(),
            latest_sender_label: activity.activity_sender_label.clone(),
            latest_body_preview: activity.activity_body_preview.clone(),
            latest_timestamp_ms: activity.activity_timestamp_ms,
        },
    );
    assert_eq!(
        authoritative
            .thread_summary
            .as_ref()
            .and_then(|summary| summary.latest_event_id.as_deref()),
        Some("$latest-reply:test")
    );
    assert_eq!(
        authoritative
            .thread_summary
            .as_ref()
            .map(|summary| summary.reply_count),
        Some(4)
    );
}

#[test]
fn loaded_old_root_reuses_message_projection_for_formatted_spoiler_and_media_content() {
    let activity = ThreadRootProjectionActivity {
        room_id: "!room:test".to_owned(),
        root_event_id: "$old-root:test".to_owned(),
        activity_event_id: "$latest-reply:test".to_owned(),
        activity_timestamp_ms: Some(1_700_000_100_000),
        activity_sender: Some("@latest:test".to_owned()),
        activity_sender_label: Some("Latest".to_owned()),
        activity_body_preview: Some("live reply preview".to_owned()),
    };
    let raw = serde_json::json!({
            "event_id": "$old-root:test",
            "sender": "@alice:test",
            "origin_server_ts": 1_700_000_000_000u64,
            "type": "m.room.message",
            "content": {
                "msgtype": "m.image",
                "body": "caption ||secret||",
                "filename": "image.png",
                "format": "org.matrix.custom.html",
                "formatted_body": "<strong>caption</strong> <span data-mx-spoiler=\"reason\">secret</span>",
                "url": "mxc://test/media",
                "info": {
                    "mimetype": "image/png",
                    "size": 42,
                    "w": 640,
                    "h": 480
            }
        }
    });

    let item = thread_root_projection_item_from_raw(&room_key(), None, &activity, raw)
        .expect("loaded image root must keep normal render fields");

    assert_eq!(
        item.formatted
            .as_ref()
            .map(|formatted| formatted.plain_text.as_str()),
        Some("caption secret")
    );
    assert!(
        item.spoiler_spans
            .iter()
            .any(|span| span.reason.as_deref() == Some("reason"))
    );
    let media = item
        .media
        .expect("image root must retain media renderer data");
    assert_eq!(media.kind, TimelineMediaKind::Image);
    assert_eq!(media.source.mxc_uri, "mxc://test/media");
    assert_eq!(media.width, Some(640));
    assert_eq!(media.height, Some(480));
}

#[test]
fn loaded_old_root_reuses_message_projection_for_file_audio_and_sticker_content() {
    let activity = ThreadRootProjectionActivity {
        room_id: "!room:test".to_owned(),
        root_event_id: "$old-root:test".to_owned(),
        activity_event_id: "$latest-reply:test".to_owned(),
        activity_timestamp_ms: Some(1_700_000_100_000),
        activity_sender: Some("@latest:test".to_owned()),
        activity_sender_label: Some("Latest".to_owned()),
        activity_body_preview: Some("live reply preview".to_owned()),
    };

    let file = thread_root_projection_item_from_raw(
        &room_key(),
        None,
        &activity,
        serde_json::json!({
                "event_id": "$old-root:test",
                "sender": "@alice:test",
                "origin_server_ts": 1_700_000_000_000u64,
                "type": "m.room.message",
                "content": {
                    "msgtype": "m.file", "body": "report.pdf", "url": "mxc://test/file",
                    "filename": "report.pdf", "info": { "mimetype": "application/pdf", "size": 4 }
            }
        }),
    )
    .expect("loaded file root should use the standard file projection");
    assert_eq!(
        file.media.as_ref().map(|media| media.kind),
        Some(TimelineMediaKind::File)
    );
    assert_eq!(
        file.media.as_ref().map(|media| media.filename.as_str()),
        Some("report.pdf")
    );

    let audio = thread_root_projection_item_from_raw(
        &room_key(),
        None,
        &activity,
        serde_json::json!({
                "event_id": "$old-root:test",
                "sender": "@alice:test",
                "origin_server_ts": 1_700_000_000_000u64,
                "type": "m.room.message",
                "content": {
                    "msgtype": "m.audio", "body": "voice.ogg", "url": "mxc://test/audio",
                    "info": { "mimetype": "audio/ogg", "size": 4 }
            }
        }),
    )
    .expect("loaded audio root should use the standard audio projection");
    assert_eq!(
        audio.media.as_ref().map(|media| media.kind),
        Some(TimelineMediaKind::Audio)
    );

    let sticker = thread_root_projection_item_from_raw(
        &room_key(),
        None,
        &activity,
        serde_json::json!({
                "event_id": "$old-root:test",
                "sender": "@alice:test",
                "origin_server_ts": 1_700_000_000_000u64,
                "type": "m.sticker",
                "content": {
                    "body": "party", "url": "mxc://test/sticker",
                    "info": { "mimetype": "image/png" }
            }
        }),
    )
    .expect("loaded sticker root should use the standard sticker projection");
    assert_eq!(sticker.body.as_deref(), Some("party"));
}

#[test]
fn cached_root_relations_project_reactions_without_network_or_unrelated_targets() {
    let relations = vec![
        serde_json::json!({
                "event_id": "$reaction-a:test", "sender": "@alice:test", "type": "m.reaction",
                "content": { "m.relates_to": { "rel_type": "m.annotation", "event_id": "$old-root:test", "key": "👍" } }
        }),
        serde_json::json!({
                "event_id": "$reaction-b:test", "sender": "@me:test", "type": "m.reaction",
                "content": { "m.relates_to": { "rel_type": "m.annotation", "event_id": "$old-root:test", "key": "👍" } }
        }),
        serde_json::json!({
                "event_id": "$different-target:test", "sender": "@eve:test", "type": "m.reaction",
                "content": { "m.relates_to": { "rel_type": "m.annotation", "event_id": "$other-root:test", "key": "👍" } }
        }),
    ];
    let own_user_id = matrix_sdk::ruma::UserId::parse("@me:test").expect("valid own user");

    let reactions = reaction_groups_from_cached_relation_events(
        relations,
        "$old-root:test",
        Some(own_user_id.as_ref()),
    );

    assert_eq!(reactions.len(), 1);
    assert_eq!(reactions[0].key, "👍");
    assert_eq!(reactions[0].count, 2);
    assert!(reactions[0].reacted_by_me);
    assert_eq!(
        reactions[0].my_reaction_event_id.as_deref(),
        Some("$reaction-b:test")
    );
}

#[test]
fn thread_summary_projection_preserves_ready_latest_event_id() {
    use matrix_sdk::ruma::{MilliSecondsSinceUnixEpoch, OwnedEventId};
    use matrix_sdk_ui::timeline::{EmbeddedEvent, MsgLikeContent, ThreadSummary};

    let latest_event_id = OwnedEventId::try_from("$latest-thread-reply:test").expect("event id");
    let summary = ThreadSummary {
        latest_event: TimelineDetails::Ready(Box::new(EmbeddedEvent {
            content: TimelineItemContent::MsgLike(MsgLikeContent::redacted()),
            sender: OwnedUserId::try_from("@latest:test").expect("user id"),
            sender_profile: TimelineDetails::Unavailable,
            timestamp: MilliSecondsSinceUnixEpoch(uint!(42)),
            identifier: TimelineEventItemId::EventId(latest_event_id.clone()),
        })),
        num_replies: 1,
        public_read_receipt_event_id: None,
        private_read_receipt_event_id: None,
    };

    let dto = thread_summary_from_sdk(summary);

    assert_eq!(
        dto.latest_event_id.as_deref(),
        Some(latest_event_id.as_str())
    );
}

#[test]
fn encrypted_thread_reply_relation_is_recovered_from_original_json() {
    let original_json = serde_json::json!({
            "content": {
                "algorithm": "m.megolm.v1.aes-sha2",
                "ciphertext": "ciphertext",
                "m.relates_to": {
                    "rel_type": "m.thread",
                    "event_id": "$thread-root:test",
                    "m.in_reply_to": {
                        "event_id": "$reply-target:test"
                },
                    "is_falling_back": true
            },
                "session_id": "session"
        },
            "event_id": "$thread-reply:test",
            "type": "m.room.encrypted"
    });

    assert_eq!(
        thread_root_from_original_json(&original_json).as_deref(),
        Some("$thread-root:test")
    );
}

#[test]
fn megolm_session_fingerprint_is_stable_compact_and_distinguishes_rotation() {
    let first = megolm_session_fingerprint("AbCdEfGhIjKlMnOpQrStUvWxYz0123456789");
    let same = megolm_session_fingerprint("AbCdEfGhIjKlMnOpQrStUvWxYz0123456789");
    let rotated = megolm_session_fingerprint("ZyXwVuTsRqPoNmLkJiHgFeDcBa9876543210");

    assert_eq!(first, "AbCdEfGhIjKl");
    assert_eq!(first, same);
    assert_ne!(first, rotated);
}

#[test]
fn room_timeline_keeps_renderable_thread_messages_visible() {
    let key = room_key();

    assert!(!timeline_item_should_be_hidden_for_key(
        &key,
        true,
        false,
        Some("$thread-root:test")
    ));
}

#[test]
fn thread_root_activity_requires_shared_attention_eligibility() {
    let mut item = timeline_item("$reply:test", Some("reply"), "@alice:test", false);
    item.thread_root = Some("$root:test".to_owned());
    item.is_redacted = true;
    assert!(thread_root_projection_activity_from_item("!r:test", &item).is_none());

    item.is_redacted = false;
    item.is_hidden = true;
    assert!(thread_root_projection_activity_from_item("!r:test", &item).is_none());
}

#[test]
fn thread_attention_does_not_count_root_or_hydrated_history_pushed_back() {
    let key = thread_key();
    let own_user_id = "@me:test";
    let items = vec![
        timeline_message_item("$root:test", "@alice:test"),
        thread_reply_item("$historical:test", "@bob:test", "$root:test"),
    ];
    let tracker = ThreadAttentionTracker::hydrate(
        &key,
        &items,
        Some(own_user_id),
        Some("$historical:test".to_owned()),
    );

    assert_eq!(tracker.counts, ThreadAttentionCounters::default());
}

#[test]
fn thread_attention_hydration_uses_visible_authoritative_receipt_baseline() {
    let key = thread_key();
    let items = vec![
        thread_reply_item("$read:test", "@alice:test", "$root:test"),
        thread_reply_item("$unread:test", "@bob:test", "$root:test"),
    ];

    let tracker = ThreadAttentionTracker::hydrate(
        &key,
        &items,
        Some("@me:test"),
        Some("$read:test".to_owned()),
    );

    assert_eq!(tracker.counts.notification_count, 1);
    assert_eq!(tracker.counts.live_event_marker_count, 1);
}

#[test]
fn thread_attention_prunes_redacted_reply_before_replay() {
    let key = thread_key();
    let mut tracker = ThreadAttentionTracker::hydrate(&key, &[], Some("@me:test"), None);
    let live = thread_reply_item("$live-redaction:test", "@bob:test", "$root:test");
    assert!(
        tracker
            .reconcile(
                &key,
                std::slice::from_ref(&live),
                Some("@me:test"),
                ThreadAttentionObservation::Live,
            )
            .is_some()
    );
    assert_eq!(tracker.counts.notification_count, 1);

    let mut redacted = live.clone();
    redacted.is_redacted = true;
    let provenance = ThreadAttentionBatchProvenance::from_timeline_items(
        std::slice::from_ref(&redacted),
        ThreadAttentionObservation::Replay,
    );
    assert_eq!(
        tracker.reconcile_batch(
            &key,
            std::slice::from_ref(&redacted),
            Some("@me:test"),
            &provenance,
        ),
        Some(AppAction::ThreadAttentionUpdated {
            room_id: "!r:test".to_owned(),
            root_event_id: "$root:test".to_owned(),
            notification_count: 0,
            highlight_count: 0,
            live_event_marker_count: 0,
        })
    );
    assert_eq!(tracker.counts.notification_count, 0);
    assert_eq!(
        tracker.reconcile(
            &key,
            std::slice::from_ref(&redacted),
            Some("@me:test"),
            ThreadAttentionObservation::Replay,
        ),
        None
    );
}

#[test]
fn thread_attention_acknowledge_prunes_hidden_reply_without_reconcile() {
    let key = thread_key();
    let mut tracker = ThreadAttentionTracker::hydrate(&key, &[], Some("@me:test"), None);
    let live = thread_reply_item("$live-hidden:test", "@bob:test", "$root:test");
    assert!(
        tracker
            .reconcile(
                &key,
                std::slice::from_ref(&live),
                Some("@me:test"),
                ThreadAttentionObservation::Live,
            )
            .is_some()
    );
    let mut hidden = live;
    hidden.is_hidden = true;

    assert_eq!(
        tracker.acknowledge(
            &key,
            std::slice::from_ref(&hidden),
            "$outside:test".to_owned()
        ),
        Some(AppAction::ThreadAttentionUpdated {
            room_id: "!r:test".to_owned(),
            root_event_id: "$root:test".to_owned(),
            notification_count: 0,
            highlight_count: 0,
            live_event_marker_count: 0,
        })
    );
}

#[test]
fn thread_attention_counts_one_live_remote_reply_and_deduplicates_replay() {
    let key = thread_key();
    let own_user_id = "@me:test";
    let mut items = vec![thread_reply_item(
        "$baseline:test",
        "@alice:test",
        "$root:test",
    )];
    let mut tracker = ThreadAttentionTracker::hydrate(
        &key,
        &items,
        Some(own_user_id),
        Some("$baseline:test".to_owned()),
    );

    let mut local_echo = thread_reply_item("$unused:test", own_user_id, "$root:test");
    local_echo.id = TimelineItemId::Transaction {
        transaction_id: "txn-own".to_owned(),
    };
    items.extend([
        local_echo,
        thread_reply_item("$own-remote:test", own_user_id, "$root:test"),
        thread_reply_item("$live:test", "@bob:test", "$root:test"),
    ]);

    assert_eq!(
        tracker.reconcile(
            &key,
            &items,
            Some(own_user_id),
            ThreadAttentionObservation::Live,
        ),
        Some(AppAction::ThreadAttentionUpdated {
            room_id: "!r:test".to_owned(),
            root_event_id: "$root:test".to_owned(),
            notification_count: 1,
            highlight_count: 0,
            live_event_marker_count: 1,
        })
    );
    assert_eq!(
        tracker.reconcile(
            &key,
            &items,
            Some(own_user_id),
            ThreadAttentionObservation::Replay,
        ),
        None,
        "the same stable event must not increment after reconnect/replay"
    );
    assert_eq!(tracker.counts.notification_count, 1);
}

#[test]
fn live_encrypted_reply_counts_when_a_later_set_becomes_renderable() {
    let key = thread_key();
    let own_user_id = "@me:test";
    let mut unavailable = thread_reply_item("$encrypted-live:test", "@bob:test", "$root:test");
    unavailable.body = None;
    unavailable.media = None;
    let mut tracker = ThreadAttentionTracker::hydrate(&key, &[], Some(own_user_id), None);

    let unavailable_provenance = ThreadAttentionBatchProvenance::from_timeline_items(
        std::slice::from_ref(&unavailable),
        ThreadAttentionObservation::Live,
    );
    assert_eq!(
        tracker.reconcile_batch(
            &key,
            std::slice::from_ref(&unavailable),
            Some(own_user_id),
            &unavailable_provenance,
        ),
        None
    );

    let unrelated = thread_reply_item("$unrelated:test", "@alice:test", "$other-root:test");
    let unrelated_provenance = ThreadAttentionBatchProvenance::from_timeline_items(
        std::slice::from_ref(&unrelated),
        ThreadAttentionObservation::Live,
    );
    assert_eq!(
        tracker.reconcile_batch(
            &key,
            &[unavailable, unrelated],
            Some(own_user_id),
            &unrelated_provenance,
        ),
        None,
        "an unrelated batch must not absorb the pending live encrypted event"
    );

    let renderable = thread_reply_item("$encrypted-live:test", "@bob:test", "$root:test");
    let renderable_provenance = ThreadAttentionBatchProvenance::from_timeline_items(
        std::slice::from_ref(&renderable),
        ThreadAttentionObservation::Live,
    );
    assert_eq!(
        tracker.reconcile_batch(
            &key,
            &[renderable],
            Some(own_user_id),
            &renderable_provenance,
        ),
        Some(AppAction::ThreadAttentionUpdated {
            room_id: "!r:test".to_owned(),
            root_event_id: "$root:test".to_owned(),
            notification_count: 1,
            highlight_count: 0,
            live_event_marker_count: 1,
        })
    );
}

#[test]
fn thread_attention_backfill_reset_and_other_roots_do_not_increment() {
    let key = thread_key();
    let own_user_id = "@me:test";
    let mut tracker = ThreadAttentionTracker::hydrate(&key, &[], Some(own_user_id), None);
    let other_root = thread_reply_item("$other:test", "@alice:test", "$other-root:test");
    let historical = thread_reply_item("$old:test", "@bob:test", "$root:test");

    assert_eq!(
        tracker.reconcile(
            &key,
            std::slice::from_ref(&historical),
            Some(own_user_id),
            ThreadAttentionObservation::Backfill,
        ),
        None
    );
    assert_eq!(
        tracker.reconcile(
            &key,
            &[historical, other_root],
            Some(own_user_id),
            ThreadAttentionObservation::Replay,
        ),
        None
    );
    assert_eq!(tracker.counts, ThreadAttentionCounters::default());

    let receipt = thread_reply_item("$visible-read:test", own_user_id, "$root:test");
    let after_receipt = thread_reply_item("$historical-after:test", "@bob:test", "$root:test");
    let mut tracker = ThreadAttentionTracker::hydrate(
        &key,
        std::slice::from_ref(&receipt),
        Some(own_user_id),
        Some("$visible-read:test".to_owned()),
    );
    assert_eq!(
        tracker.reconcile(
            &key,
            &[receipt, after_receipt],
            Some(own_user_id),
            ThreadAttentionObservation::Backfill,
        ),
        None,
        "ordinary pagination never manufactures attention"
    );
    assert_eq!(tracker.counts, ThreadAttentionCounters::default());
}

#[test]
fn delayed_pagination_batch_does_not_become_live_after_task_completion() {
    let key = thread_key();
    let own_user_id = "@me:test";
    let historical = thread_reply_item("$old-delayed:test", "@bob:test", "$root:test");
    let mut tracker = ThreadAttentionTracker::hydrate(&key, &[], Some(own_user_id), None);

    // Reproduce the actor race reported by independent review: the SDK
    // pagination call has completed and cleared ambient task state before
    // its separately relayed PushBack batch reaches the actor.
    let delayed_pagination_provenance = ThreadAttentionBatchProvenance::from_timeline_items(
        std::slice::from_ref(&historical),
        ThreadAttentionObservation::Backfill,
    );

    assert_eq!(
        tracker.reconcile_batch(
            &key,
            std::slice::from_ref(&historical),
            Some(own_user_id),
            &delayed_pagination_provenance,
        ),
        None,
        "pagination provenance must travel with the delayed batch"
    );
    assert_eq!(tracker.counts, ThreadAttentionCounters::default());
}

#[test]
fn sdk_event_origin_is_the_relay_batch_attention_provenance() {
    assert_eq!(
        thread_attention_observation_from_event_origin(Some(EventItemOrigin::Sync)),
        ThreadAttentionObservation::Live
    );
    assert_eq!(
        thread_attention_observation_from_event_origin(Some(EventItemOrigin::Pagination)),
        ThreadAttentionObservation::Backfill
    );
    assert_eq!(
        thread_attention_observation_from_event_origin(Some(EventItemOrigin::Cache)),
        ThreadAttentionObservation::Replay
    );
    assert_eq!(
        thread_attention_observation_from_event_origin(None),
        ThreadAttentionObservation::Replay,
        "unknown and delayed hydration must be conservative"
    );
}

#[test]
fn thread_attention_trackers_do_not_contaminate_different_threads() {
    let first_key = thread_key();
    let second_key = TimelineKey {
        account_key: first_key.account_key.clone(),
        kind: TimelineKind::Thread {
            room_id: "!r:test".to_owned(),
            root_event_id: "$second-root:test".to_owned(),
        },
    };
    let first_live = thread_reply_item("$first-live:test", "@alice:test", "$root:test");
    let mut first = ThreadAttentionTracker::hydrate(&first_key, &[], Some("@me:test"), None);
    let mut second = ThreadAttentionTracker::hydrate(&second_key, &[], Some("@me:test"), None);

    assert!(
        first
            .reconcile(
                &first_key,
                std::slice::from_ref(&first_live),
                Some("@me:test"),
                ThreadAttentionObservation::Live,
            )
            .is_some()
    );
    assert_eq!(
        second.reconcile(
            &second_key,
            &[first_live],
            Some("@me:test"),
            ThreadAttentionObservation::Live,
        ),
        None
    );
    assert_eq!(first.counts.notification_count, 1);
    assert_eq!(second.counts.notification_count, 0);
}

#[test]
fn thread_attention_acknowledgement_clears_without_changing_total_reply_count() {
    let key = thread_key();
    let own_user_id = "@me:test";
    let mut root = timeline_message_item("$root:test", "@alice:test");
    root.thread_summary = Some(ThreadSummaryDto {
        reply_count: 2,
        latest_event_id: Some("$live:test".to_owned()),
        latest_sender: Some("@bob:test".to_owned()),
        latest_sender_label: Some("Bob".to_owned()),
        latest_body_preview: Some("preview".to_owned()),
        latest_timestamp_ms: Some(2),
    });
    let items = vec![
        root,
        thread_reply_item("$baseline:test", "@alice:test", "$root:test"),
        thread_reply_item("$live:test", "@bob:test", "$root:test"),
    ];
    let mut tracker = ThreadAttentionTracker::hydrate(
        &key,
        &items[..2],
        Some(own_user_id),
        Some("$baseline:test".to_owned()),
    );
    let _ = tracker.reconcile(
        &key,
        &items,
        Some(own_user_id),
        ThreadAttentionObservation::Live,
    );

    assert_eq!(tracker.counts.notification_count, 1);
    assert_eq!(items[0].thread_summary.as_ref().unwrap().reply_count, 2);
    assert_eq!(
        tracker.acknowledge(&key, &items, "$outside-window:test".to_owned()),
        Some(AppAction::ThreadAttentionUpdated {
            room_id: "!r:test".to_owned(),
            root_event_id: "$root:test".to_owned(),
            notification_count: 1,
            highlight_count: 0,
            live_event_marker_count: 1,
        }),
        "an out-of-window receipt must not guess the relative ordering"
    );
    assert_eq!(
        tracker.acknowledge(&key, &items, "$live:test".to_owned()),
        Some(AppAction::ThreadAttentionUpdated {
            room_id: "!r:test".to_owned(),
            root_event_id: "$root:test".to_owned(),
            notification_count: 0,
            highlight_count: 0,
            live_event_marker_count: 0,
        })
    );
    assert_eq!(items[0].thread_summary.as_ref().unwrap().reply_count, 2);
}

#[test]
fn visible_receipt_prunes_attention_preserved_while_it_was_outside_the_window() {
    let key = thread_key();
    let own_user_id = "@me:test";
    let live = thread_reply_item("$live-before-receipt:test", "@bob:test", "$root:test");
    let mut tracker = ThreadAttentionTracker::hydrate(&key, &[], Some(own_user_id), None);
    let _ = tracker.reconcile(
        &key,
        std::slice::from_ref(&live),
        Some(own_user_id),
        ThreadAttentionObservation::Live,
    );
    assert_eq!(tracker.counts.notification_count, 1);
    let _ = tracker.acknowledge(
        &key,
        std::slice::from_ref(&live),
        "$later-receipt:test".to_owned(),
    );
    assert_eq!(tracker.counts.notification_count, 1);

    let receipt = thread_reply_item("$later-receipt:test", own_user_id, "$root:test");
    let expanded = vec![live, receipt];
    assert_eq!(
        tracker.reconcile(
            &key,
            &expanded,
            Some(own_user_id),
            ThreadAttentionObservation::Backfill,
        ),
        Some(AppAction::ThreadAttentionUpdated {
            room_id: "!r:test".to_owned(),
            root_event_id: "$root:test".to_owned(),
            notification_count: 0,
            highlight_count: 0,
            live_event_marker_count: 0,
        })
    );
}

#[test]
fn recovery_counts_first_seen_unread_reply_after_visible_receipt() {
    let key = thread_key();
    let own_user_id = "@me:test";
    let receipt = thread_reply_item("$read-before-overflow:test", own_user_id, "$root:test");
    let unread = thread_reply_item("$missed-during-overflow:test", "@bob:test", "$root:test");
    let mut tracker = ThreadAttentionTracker::hydrate(
        &key,
        std::slice::from_ref(&receipt),
        Some(own_user_id),
        Some("$read-before-overflow:test".to_owned()),
    );

    assert_eq!(
        tracker.reconcile(
            &key,
            &[receipt, unread],
            Some(own_user_id),
            ThreadAttentionObservation::Replay,
        ),
        Some(AppAction::ThreadAttentionUpdated {
            room_id: "!r:test".to_owned(),
            root_event_id: "$root:test".to_owned(),
            notification_count: 1,
            highlight_count: 0,
            live_event_marker_count: 1,
        })
    );
}

#[test]
fn successful_receipt_uses_newest_provable_canonical_boundary() {
    let items = vec![
        thread_reply_item("$old-read:test", "@me:test", "$root:test"),
        thread_reply_item("$requested-read:test", "@me:test", "$root:test"),
        thread_reply_item("$newer-device-read:test", "@me:test", "$root:test"),
    ];

    let requested = "$requested-read:test";
    let selected = newest_provable_receipt_event_id(
        &items,
        requested,
        Some("$old-read:test".to_owned()),
        Some("$old-read:test"),
    );
    assert_eq!(
        selected, requested,
        "a stale SDK query must not delay the successful newer request"
    );

    assert_eq!(
        newest_provable_receipt_event_id(
            &items,
            "$requested-read:test",
            Some("$old-read:test".to_owned()),
            Some("$newer-device-read:test"),
        ),
        "$newer-device-read:test",
        "a stale request must not regress a newer multi-device boundary"
    );

    assert_eq!(
        newest_provable_receipt_event_id(
            &items[1..2],
            "$requested-read:test",
            Some("$queried-outside-window:test".to_owned()),
            Some("$current-outside-window:test"),
        ),
        "$requested-read:test",
        "unknown out-of-window IDs cannot override a visible successful request"
    );
}
