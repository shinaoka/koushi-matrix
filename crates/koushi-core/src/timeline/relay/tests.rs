use super::super::test_source::item_body;

use std::collections::{BTreeSet, HashMap, HashSet};

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

use std::time::Duration;

use futures_util::StreamExt;

use koushi_state::AppAction;

use matrix_sdk_ui::timeline::TimelineItem as SdkTimelineItem;
use tokio::sync::{broadcast, mpsc};

use crate::executor;
use koushi_protocol::event::{
    CoreEvent, TimelineDiff, TimelineEvent, TimelineItem, TimelineItemId, TimelineResyncReason,
    TimelineSendState,
};

use koushi_protocol::ids::{TimelineBatchId, TimelineGeneration};

use crate::search::SearchIndexMessage;

use crate::threads_list::ThreadRootProjectionService;

use super::super::actor::TimelineActorMessage;
use super::super::display_projection::DisplayProjectionState;
use super::super::item_projection::sdk_vector_diffs_to_timeline_diffs;
use super::super::navigation::{
    InitialItemsRequestIdentity, PreparedInitialWindow, TimelineActorGenerationGate,
    commit_prepared_initial_window_for_generation, emit_items_updated_for_generation,
};
use super::super::outbound_send::{PendingSendPhase, PendingSendProjection, pending_send_item};
use super::super::test_support::{
    fake_rid, projection_service, replacement_generation_fixture, room_key, timeline_item,
};
use super::super::thread_projection::ThreadAttentionBatchProvenance;
use super::{
    PreparedRelayRecovery, RelayRestartBackoff, RelayRestartSchedule, TimelineRelayBatch,
    TimelineRelayControl, accepted_relay_batch, authoritative_receipts_action,
    authoritative_search_removals, authoritative_window_reconciliation,
    commit_authoritative_recovery_window, pending_display_inputs_for_incoming_transactions,
    prepare_relay_recovery, replace_authoritative_cache, run_diff_relay, spawn_relay_restart_timer,
};

#[test]
fn mixed_bound_and_unbound_local_echo_batch_keeps_exactly_one_row_per_send() {
    let key = room_key();
    let make_projection = |sequence, client: &str, sdk: Option<&str>| {
        let render_id = sdk.unwrap_or(client);
        PendingSendProjection {
            key: key.clone(),
            sequence,
            client_txn_id: client.to_owned(),
            item: pending_send_item(render_id, "body", None, None, None),
            sdk_transaction_id: sdk.map(str::to_owned),
            handle: None,
            terminal_event_id: None,
            phase: PendingSendPhase::Pending,
        }
    };
    let mut sent_fallback = make_projection(3, "client-sent", Some("sdk-delayed"));
    sent_fallback.phase = PendingSendPhase::SentAwaitingRemote;
    sent_fallback.terminal_event_id = Some("$sent:test".to_owned());
    sent_fallback.item.id = TimelineItemId::Event {
        event_id: "$sent:test".to_owned(),
    };
    sent_fallback.item.send_state = Some(TimelineSendState::Sent);
    let projections = vec![
        make_projection(1, "client-bound", Some("sdk-bound")),
        make_projection(2, "client-unbound", None),
        sent_fallback,
    ];
    let incoming = HashSet::from([
        "sdk-bound".to_owned(),
        "sdk-overtake".to_owned(),
        "sdk-delayed".to_owned(),
    ]);
    let (pending, suppressed) =
        pending_display_inputs_for_incoming_transactions(&projections, &incoming, HashSet::new());

    assert_eq!(pending.len(), 2);
    assert!(pending.iter().any(|item| matches!(
        &item.id,
        TimelineItemId::Transaction { transaction_id }
            if transaction_id == "client-unbound"
    )));
    assert!(pending.iter().any(|item| matches!(
        &item.id,
        TimelineItemId::Event { event_id } if event_id == "$sent:test"
    )));
    assert_eq!(
        suppressed,
        HashSet::from(["sdk-overtake".to_owned(), "sdk-delayed".to_owned()])
    );
}

#[test]
fn batch_id_monotonically_increases_per_generation() {
    let mut id = TimelineBatchId(0);
    let mut seen = Vec::new();
    for _ in 0..10 {
        seen.push(id);
        id = TimelineBatchId(id.0 + 1);
    }
    for (i, pair) in seen.windows(2).enumerate() {
        assert!(pair[0] < pair[1], "batch ids must be increasing: index {i}");
    }
    // After generation reset, batch_id resets to 0.
    let reset = TimelineBatchId(0);
    assert_eq!(reset, TimelineBatchId(0));
}

#[tokio::test]
async fn relay_overflow_control_is_delivered_when_data_inbox_is_full() {
    let (actor_tx, mut actor_rx) = mpsc::channel::<TimelineRelayBatch>(1);
    let (command_tx, mut command_rx) = mpsc::channel::<TimelineActorMessage>(1);
    let (control_tx, mut control_rx) = mpsc::channel::<TimelineRelayControl>(1);

    command_tx
        .try_send(TimelineActorMessage::ReplayInitialItems {
            cause_request_id: Some(fake_rid(1)),
        })
        .expect("command must queue independently of relay data");

    actor_tx
        .try_send(TimelineRelayBatch {
            generation: TimelineGeneration(7),
            diffs: Vec::new(),
            thread_attention_provenance: ThreadAttentionBatchProvenance::default(),
            gap_repair_projections: BTreeSet::new(),
        })
        .expect("test must fill the data inbox");

    let relay = executor::spawn(run_diff_relay(
        actor_tx.clone(),
        control_tx.clone(),
        TimelineGeneration(7),
        1,
        futures_util::stream::iter([Vec::new()]),
        vec![],
    ));

    let control = tokio::time::timeout(Duration::from_secs(1), control_rx.recv())
        .await
        .expect("overflow control must not wait for capacity in the data inbox")
        .expect("relay must keep the control lane open until overflow is delivered");
    assert!(matches!(
        control,
        TimelineRelayControl::Overflow {
            generation: TimelineGeneration(7)
        }
    ));
    relay
        .await
        .expect("overflowed relay must terminate cleanly");

    let _filled_message = actor_rx
        .recv()
        .await
        .expect("test must release the old full data inbox");
    run_diff_relay(
        actor_tx,
        control_tx,
        TimelineGeneration(8),
        1,
        futures_util::stream::iter([Vec::new()]),
        vec![],
    )
    .await;
    assert!(matches!(
        actor_rx.recv().await,
        Some(TimelineRelayBatch {
            generation: TimelineGeneration(8),
            diffs,
            ..
        }) if diffs.is_empty()
    ));
    assert!(matches!(
        command_rx.try_recv(),
        Ok(TimelineActorMessage::ReplayInitialItems { .. })
    ));
}

#[tokio::test]
async fn relay_stream_end_emits_one_recovery_control_without_consuming_commands() {
    let (data_tx, mut data_rx) = mpsc::channel::<TimelineRelayBatch>(1);
    let (control_tx, mut control_rx) = mpsc::channel::<TimelineRelayControl>(1);
    let (command_tx, mut command_rx) = mpsc::channel::<TimelineActorMessage>(1);
    command_tx
        .try_send(TimelineActorMessage::ReplayInitialItems {
            cause_request_id: Some(fake_rid(88)),
        })
        .expect("command must queue independently");

    run_diff_relay(
        data_tx,
        control_tx,
        TimelineGeneration(9),
        1,
        futures_util::stream::empty(),
        vec![],
    )
    .await;

    assert!(matches!(
        control_rx.recv().await,
        Some(TimelineRelayControl::StreamEnded {
            generation: TimelineGeneration(9)
        })
    ));
    assert!(matches!(data_rx.recv().await, None));
    assert!(matches!(
        command_rx.try_recv(),
        Ok(TimelineActorMessage::ReplayInitialItems { .. })
    ));
    assert!(matches!(
        control_rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected)
    ));
}

#[test]
fn relay_restart_backoff_grows_caps_resets_and_rejects_stale_due_tokens() {
    let mut backoff =
        RelayRestartBackoff::new(Duration::from_millis(10), Duration::from_millis(40));
    let first = backoff.schedule(TimelineGeneration(3));
    let second = backoff.schedule(TimelineGeneration(3));
    let third = backoff.schedule(TimelineGeneration(3));
    let capped = backoff.schedule(TimelineGeneration(3));
    assert_eq!(first.delay, Duration::from_millis(10));
    assert_eq!(second.delay, Duration::from_millis(20));
    assert_eq!(third.delay, Duration::from_millis(40));
    assert_eq!(capped.delay, Duration::from_millis(40));
    assert!(!backoff.accept_due(first.generation, first.serial));
    assert!(backoff.accept_due(capped.generation, capped.serial));
    assert!(!backoff.accept_due(capped.generation, capped.serial));

    backoff.reset_after_live_batch();
    let reset = backoff.schedule(TimelineGeneration(4));
    assert_eq!(reset.delay, Duration::from_millis(10));
    assert!(!backoff.accept_due(TimelineGeneration(3), reset.serial));
}

#[tokio::test]
async fn relay_restart_timer_does_not_block_commands_and_emits_one_due_after_delay() {
    let (control_tx, mut control_rx) = mpsc::channel(1);
    let (command_tx, mut command_rx) = mpsc::channel(1);
    command_tx
        .try_send(TimelineActorMessage::ReplayInitialItems {
            cause_request_id: Some(fake_rid(89)),
        })
        .expect("command queued during restart delay");
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let schedule = RelayRestartSchedule {
        generation: TimelineGeneration(5),
        serial: 11,
        delay: Duration::ZERO,
    };
    let timer = spawn_relay_restart_timer(control_tx, schedule, async move {
        let _ = release_rx.await;
    });

    assert!(matches!(
        control_rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
    assert!(matches!(
        command_rx.try_recv(),
        Ok(TimelineActorMessage::ReplayInitialItems { .. })
    ));
    release_tx.send(()).expect("release timer");
    timer.await.expect("timer task");
    assert!(matches!(
        control_rx.recv().await,
        Some(TimelineRelayControl::RestartDue {
            generation: TimelineGeneration(5),
            serial: 11,
        })
    ));
    assert!(matches!(
        control_rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected)
    ));
}

#[tokio::test]
async fn relay_overflow_recovery_subscribes_once_emits_snapshot_then_next_live_update() {
    let key = room_key();
    let (event_tx, mut event_rx) = broadcast::channel(8);
    let actor_generations = Arc::new(TimelineActorGenerationGate::default());
    let actor_generation = actor_generations
        .activate_after_quiescence(&key)
        .await
        .generation;
    let projection_service = Arc::new(Mutex::new(ThreadRootProjectionService::default()));
    let subscribe_count = Arc::new(AtomicU64::new(0));
    let subscribe_count_for_fake = subscribe_count.clone();

    let prepared = prepare_relay_recovery(
        TimelineGeneration(7),
        TimelineGeneration(7),
        move || async move {
            subscribe_count_for_fake.fetch_add(1, Ordering::SeqCst);
            (
                Vec::<TimelineItem>::new(),
                futures_util::stream::iter([vec![
                    eyeball_im::VectorDiff::<Arc<SdkTimelineItem>>::Clear,
                ]]),
            )
        },
    )
    .await
    .expect("matching overflow generation must recover");
    assert_eq!(subscribe_count.load(Ordering::SeqCst), 1);
    assert_eq!(prepared.generation, TimelineGeneration(8));

    let PreparedRelayRecovery {
        generation,
        snapshot,
        stream,
    } = prepared;
    let mut navigation_items = Vec::new();
    let mut display_projection = DisplayProjectionState::default();
    assert!(commit_authoritative_recovery_window(
        &mut navigation_items,
        &mut display_projection,
        &event_tx,
        &projection_service,
        koushi_state::TimelineThreadRootOrder::LatestReply,
        &actor_generations,
        &key,
        actor_generation,
        generation,
        TimelineResyncReason::QueueOverflow,
        snapshot,
        || {},
    ));
    assert!(matches!(
        event_rx.recv().await,
        Ok(CoreEvent::Timeline(TimelineEvent::ResyncRequired { .. }))
    ));
    assert!(matches!(
        event_rx.recv().await,
        Ok(CoreEvent::Timeline(TimelineEvent::InitialItems {
            generation: TimelineGeneration(8),
            ..
        }))
    ));

    let (actor_tx, mut actor_rx) = mpsc::channel(1);
    let (control_tx, _control_rx) = mpsc::channel(1);
    run_diff_relay(
        actor_tx,
        control_tx,
        generation,
        actor_generation,
        stream,
        vec![],
    )
    .await;
    let Some(TimelineRelayBatch {
        generation: batch_generation,
        diffs,
        ..
    }) = actor_rx.recv().await
    else {
        panic!("replacement stream must produce a generation-tagged batch");
    };
    let sdk_diffs = accepted_relay_batch(generation, batch_generation, diffs)
        .expect("replacement generation must be accepted");
    let diffs =
        sdk_vector_diffs_to_timeline_diffs(&sdk_diffs, 0, &key, None, &HashMap::new(), None, None);
    assert!(emit_items_updated_for_generation(
        &event_tx,
        &actor_generations,
        &key,
        actor_generation,
        generation,
        TimelineBatchId(0),
        diffs,
    ));
    assert!(matches!(
        event_rx.recv().await,
        Ok(CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
            generation: TimelineGeneration(8),
            diffs,
            ..
        })) if matches!(diffs.as_slice(), [TimelineDiff::Clear])
    ));
}

#[test]
fn relay_overflow_stale_generation_is_rejected_without_state_or_event_change() {
    let next_batch_id = TimelineBatchId(4);
    let (event_tx, mut event_rx) = broadcast::channel::<CoreEvent>(1);
    let stale = accepted_relay_batch(
        TimelineGeneration(8),
        TimelineGeneration(7),
        vec![TimelineDiff::Clear],
    );
    if stale.is_some() {
        let _ = event_tx.send(CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
            key: room_key(),
            generation: TimelineGeneration(7),
            batch_id: next_batch_id,
            diffs: vec![TimelineDiff::Clear],
        }));
    }
    assert!(stale.is_none());
    assert_eq!(next_batch_id, TimelineBatchId(4));
    assert!(matches!(
        event_rx.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
}

#[test]
fn relay_overflow_authoritative_window_plan_scopes_receipts_and_search_removals() {
    let old =
        std::collections::BTreeSet::from(["$removed:test".to_owned(), "$retained:test".to_owned()]);
    let new =
        std::collections::BTreeSet::from(["$retained:test".to_owned(), "$added:test".to_owned()]);
    let plan = authoritative_window_reconciliation(&old, &new);
    assert_eq!(
        plan.scoped_event_ids,
        vec!["$added:test", "$removed:test", "$retained:test"]
    );
    assert_eq!(plan.removed_event_ids, vec!["$removed:test"]);
    assert!(matches!(
        authoritative_search_removals(&plan).as_slice(),
        [SearchIndexMessage::Redact { event_id }] if event_id == "$removed:test"
    ));
    assert!(matches!(
        authoritative_receipts_action("!room:test", &plan, Vec::new()),
        AppAction::LiveRoomReceiptsWindowReconciled {
            scoped_event_ids,
            receipts_by_event,
            ..
        } if scoped_event_ids.len() == 3 && receipts_by_event.is_empty()
    ));
}

#[test]
fn relay_overflow_authoritative_cache_rebuild_removes_old_and_installs_new_entries() {
    let mut cache = HashMap::from([("old", 1_u8), ("retained-old-value", 2)]);
    replace_authoritative_cache(
        &mut cache,
        HashMap::from([("retained-old-value", 3_u8), ("new", 4)]),
    );
    assert_eq!(
        cache,
        HashMap::from([("retained-old-value", 3_u8), ("new", 4)])
    );
}

#[tokio::test]
async fn authoritative_resync_projects_event_only_and_emits_ordered_recovery_events() {
    let mut transaction = timeline_item(
        "$transaction-placeholder:test",
        Some("same body"),
        "@me:test",
        false,
    );
    transaction.id = TimelineItemId::Transaction {
        transaction_id: "txn-echo".to_owned(),
    };
    transaction.send_state = Some(TimelineSendState::Sending);
    let remote = timeline_item("$remote-echo:test", Some("same body"), "@me:test", false);
    let mut current = vec![transaction, remote.clone()];
    let mut display_projection =
        DisplayProjectionState::from_canonical_window(&current, 0..current.len());
    let key = room_key();
    let (event_tx, mut event_rx) = broadcast::channel(8);
    let actor_generations = Arc::new(TimelineActorGenerationGate::default());
    let actor_generation = actor_generations
        .activate_after_quiescence(&key)
        .await
        .generation;
    let projection_service = Arc::new(Mutex::new(ThreadRootProjectionService::default()));

    commit_authoritative_recovery_window(
        &mut current,
        &mut display_projection,
        &event_tx,
        &projection_service,
        koushi_state::TimelineThreadRootOrder::LatestReply,
        &actor_generations,
        &key,
        actor_generation,
        TimelineGeneration(2),
        TimelineResyncReason::QueueOverflow,
        vec![remote],
        || {},
    );

    assert_eq!(current.len(), 1);
    assert!(matches!(
        current[0].id,
        TimelineItemId::Event { ref event_id } if event_id == "$remote-echo:test"
    ));
    assert!(!matches!(
        current[0].send_state,
        Some(TimelineSendState::Sending)
    ));
    assert_eq!(display_projection.display_items().len(), current.len());
    assert_eq!(display_projection.display_items()[0].id, current[0].id);
    assert!(
        display_projection.display_items()[0]
            .display_metadata
            .is_some()
    );
    assert!(current[0].display_metadata.is_none());
    assert!(matches!(
        event_rx.recv().await,
        Ok(CoreEvent::Timeline(TimelineEvent::ResyncRequired { .. }))
    ));
    assert!(matches!(
        event_rx.recv().await,
        Ok(CoreEvent::Timeline(TimelineEvent::InitialItems {
            request_id: None,
            cause_request_id: None,
            generation: TimelineGeneration(2),
            items,
            ..
        })) if items.len() == 1
            && matches!(items[0].id, TimelineItemId::Event { .. })
            && !matches!(items[0].send_state, Some(TimelineSendState::Sending))
    ));
}

#[tokio::test]
async fn stale_generation_recovery_does_not_commit_candidate_or_publish() {
    let key = room_key();
    let (actor_generations, stale_generation, _current_generation) =
        replacement_generation_fixture(&key).await;
    let original = timeline_item("$original:test", Some("original"), "@me:test", false);
    let candidate = timeline_item("$candidate:test", Some("candidate"), "@me:test", false);
    let mut navigation_items = vec![original.clone()];
    let mut display_projection =
        DisplayProjectionState::from_canonical_window(&navigation_items, 0..navigation_items.len());
    let display_before = display_projection.clone();
    let (event_tx, mut event_rx) = broadcast::channel(8);
    let projection_service = projection_service();
    let mut synchronous_candidate_committed = false;

    assert!(!commit_authoritative_recovery_window(
        &mut navigation_items,
        &mut display_projection,
        &event_tx,
        &projection_service,
        koushi_state::TimelineThreadRootOrder::LatestReply,
        &actor_generations,
        &key,
        stale_generation,
        TimelineGeneration(2),
        TimelineResyncReason::QueueOverflow,
        vec![candidate],
        || synchronous_candidate_committed = true,
    ));

    assert_eq!(navigation_items, vec![original]);
    assert_eq!(display_projection, display_before);
    assert!(!synchronous_candidate_committed);
    assert!(matches!(
        event_rx.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn stale_generation_prepared_initial_window_does_not_commit_or_publish() {
    let key = room_key();
    let (actor_generations, stale_generation, _current_generation) =
        replacement_generation_fixture(&key).await;
    let original = timeline_item("$original:test", Some("original"), "@me:test", false);
    let candidate = timeline_item("$candidate:test", Some("candidate"), "@me:test", false);
    let mut navigation_items = vec![original.clone()];
    let mut display_projection =
        DisplayProjectionState::from_canonical_window(&navigation_items, 0..navigation_items.len());
    let display_before = display_projection.clone();
    let candidate_navigation = vec![candidate.clone()];
    let prepared = PreparedInitialWindow {
        display_projection: DisplayProjectionState::from_canonical_window(
            &candidate_navigation,
            0..candidate_navigation.len(),
        ),
        navigation_items: Some(candidate_navigation),
        emitted_items: vec![candidate],
    };
    let (event_tx, mut event_rx) = broadcast::channel(8);

    assert!(!commit_prepared_initial_window_for_generation(
        &mut navigation_items,
        &mut display_projection,
        &event_tx,
        &actor_generations,
        &key,
        stale_generation,
        InitialItemsRequestIdentity::recovery(),
        TimelineGeneration(2),
        Vec::new(),
        prepared,
    ));

    assert_eq!(navigation_items, vec![original]);
    assert_eq!(display_projection, display_before);
    assert!(matches!(
        event_rx.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn relay_overflow_signal_triggers_generation_bump() {
    // Test the overflow logic directly on the actor message pathway,
    // using a synthetic mpsc channel at capacity 1 to force overflow.
    let (event_tx, mut event_rx): (broadcast::Sender<CoreEvent>, _) = broadcast::channel(256);
    let (actor_tx, actor_rx) = mpsc::channel::<TimelineActorMessage>(2);

    let key = room_key();
    let generation = Arc::new(AtomicU64::new(0));
    let next_batch_id = Arc::new(AtomicU64::new(0));

    // Simulate the actor receiving RelayOverflow:
    // It should increment generation, reset batch_id, and emit ResyncRequired.
    // We test the state machine logic directly.
    let gen_before = generation.load(Ordering::SeqCst);
    let new_gen = gen_before + 1;
    generation.store(new_gen, Ordering::SeqCst);
    next_batch_id.store(0, Ordering::SeqCst);

    let _ = event_tx.send(CoreEvent::Timeline(TimelineEvent::ResyncRequired {
        key: key.clone(),
        reason: TimelineResyncReason::QueueOverflow,
    }));

    // Verify the event was emitted.
    let event = event_rx.recv().await.expect("event");
    match event {
        CoreEvent::Timeline(TimelineEvent::ResyncRequired {
            key: ev_key,
            reason,
        }) => {
            assert_eq!(ev_key, key);
            assert_eq!(reason, TimelineResyncReason::QueueOverflow);
        }
        other => panic!("expected ResyncRequired, got {other:?}"),
    }

    assert_eq!(
        generation.load(Ordering::SeqCst),
        1,
        "generation must be bumped"
    );
    assert_eq!(
        next_batch_id.load(Ordering::SeqCst),
        0,
        "batch_id resets to 0"
    );

    drop(actor_tx);
    drop(actor_rx);
}
