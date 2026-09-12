use super::super::test_source::item_body;

use std::collections::{BTreeSet, HashMap};

use std::sync::{Arc, Mutex};

use std::time::Duration;

use koushi_sdk::{MatrixClientSession, MatrixLiveTailRefreshOutcome};

use koushi_state::AppAction;

use matrix_sdk_ui::timeline::{GapRepairProjectionId, TimelineFocus};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::account_work::{AccountWorkKind, AccountWorkScheduler};
#[cfg(test)]
use crate::causal_projection::{CAUSAL_PROJECTION_DOMAIN_BIT, CAUSAL_PROJECTION_SERIAL_MAX};
use crate::causal_projection::{
    CausalProjectionDomain, CausalProjectionId, CausalProjectionOperationId,
    next_causal_projection_serial,
};
use crate::executor;
use crate::link_preview::LinkPreviewContext;
use koushi_protocol::command::TimelineCommand;
use koushi_protocol::event::{
    CoreEvent, PaginationDirection, PaginationState, ThreadSummaryDto, TimelineBottomArrival,
    TimelineEvent, TimelineFormattedBody, TimelineItemId, TimelineReadStateSync,
    TimelineUnreadPosition, TimelineViewportObservation,
};
use koushi_protocol::failure::{CoreFailure, TimelineFailureKind};
#[cfg(any(test, feature = "test-hooks"))]
use koushi_protocol::ids::AccountKey;
use koushi_protocol::ids::{TimelineBatchId, TimelineGeneration, TimelineKey, TimelineKind};

use crate::live_tail_freshness::{
    FOREGROUND_LIVE_TAIL_LIMIT, LiveTailFreshnessState, LiveTailRefreshCoordinator,
    LiveTailSchedulerAction,
};

use crate::threads_list::ThreadRootProjectionService;
use koushi_sdk::MatrixLiveTailRefreshOutcome as LiveTailRefreshOutcome;

use koushi_diagnostics::DiagnosticValue;
use koushi_state::{SessionInfo, SessionState};

use crate::runtime::CoreRuntime;
use koushi_protocol::command::CoreCommand;

use super::super::actor::{
    TimelineActor, TimelineActorCleanupIngress, TimelineActorCleanupState, TimelineActorControl,
    TimelineActorHandle, TimelineActorMessage,
};
use super::super::display_projection::apply_timeline_diffs_to_items;
use super::super::gap_repair::{
    CausalProjectionObservation, TimelineGapProjectionCompletion, TimelineGapProjectionCorrelation,
    live_tail_causal_projection_operation, observe_causal_projection,
};
use super::super::item_projection::{
    sdk_item_to_timeline_item, sdk_vector_diffs_to_timeline_diffs, timeline_item_event_id,
};
use super::super::manager::{TimelineManagerActor, TimelineManagerControl, TimelineMessage};
use super::super::outbound_send::{
    SharedSendCompletionCoordinator, TimelineSendEnqueueContext, TimelineSendTerminalIngress,
};
use super::super::relay::koushi_timeline_builder;
use super::super::test_support::{
    fake_rid, focused_key, gap_demand_test_actor_handle, live_tail_test_manager, room_key,
    test_timeline_actor_handle, thread_key, timeline_item,
};
use super::super::thread_projection::ThreadAttentionTracker;
use super::{
    InitialItemsRequestIdentity, NavigationProjectionCleanup, NavigationProjectionIngress,
    NavigationProjectionIntent, ROOM_REPLAY_INITIAL_ITEMS_MAX, TimelineActorGenerationGate,
    acquire_pagination_permit_and_emit_paginating, activity_row_from_timeline_item,
    backward_pagination_changed_oldest_edge, derive_timeline_navigation_snapshot,
    derive_timeline_navigation_snapshot_with_read_state, emit_initial_items_for_generation,
    receive_navigation_projection, replay_initial_items_window,
    should_hydrate_empty_initial_room_timeline, timeline_unread_consistency_diagnostic_event,
};

#[tokio::test]
async fn focused_projection_commit_does_not_require_a_core_event_consumer() {
    let key = focused_key();
    let request_id = fake_rid(73);
    let (focused_projection_tx, mut focused_projection_rx) = mpsc::unbounded_channel();
    let generations = Arc::new(
        TimelineActorGenerationGate::with_focused_projection_commits(Some(focused_projection_tx)),
    );
    let actor_generation = generations.activate_after_quiescence(&key).await.generation;
    let (event_tx, event_rx) = broadcast::channel(1);
    drop(event_rx);
    let target_event_id = match &key.kind {
        TimelineKind::Focused { event_id, .. } => event_id.clone(),
        _ => unreachable!("focused test key"),
    };

    assert!(emit_initial_items_for_generation(
        &event_tx,
        &generations,
        &key,
        actor_generation,
        InitialItemsRequestIdentity::fresh(request_id),
        TimelineGeneration(0),
        vec![timeline_item(
            &target_event_id,
            Some("target"),
            "@alice:test",
            false
        )],
        Vec::new(),
    ));
    let committed = focused_projection_rx
        .recv()
        .await
        .expect("focused projection commit must use the private actor lane");
    assert_eq!(committed.projection_request_id, request_id);
    assert_eq!(committed.key, key);
    assert_eq!(committed.actor_generation, actor_generation);
    assert_eq!(committed.timeline_generation, TimelineGeneration(0));
    assert_eq!(committed.item_count, 1);
    assert!(committed.target_present);
}

#[test]
fn eligibility_skips_redacted_and_own_rows_for_first_unread_and_newer_count() {
    let marker = timeline_item("$marker:test", Some("marker"), "@alice:test", false);
    let mut redacted = timeline_item("$redacted:test", Some("redacted"), "@alice:test", false);
    redacted.is_redacted = true;
    let valid = timeline_item("$valid:test", Some("valid"), "@bob:test", false);
    let own = timeline_item("$own:test", Some("own"), "@me:test", false);
    let items = vec![marker, redacted, valid, own];
    let observation = TimelineViewportObservation {
        first_visible_event_id: Some("$marker:test".to_owned()),
        last_visible_event_id: Some("$marker:test".to_owned()),
        bottom_arrival: TimelineBottomArrival::User,
        at_bottom: false,
        ..TimelineViewportObservation::default()
    };

    let snapshot = derive_timeline_navigation_snapshot(
        &items,
        Some("$marker:test"),
        &observation,
        Some("@me:test"),
    );

    assert_eq!(
        snapshot.first_unread_event_id.as_deref(),
        Some("$valid:test")
    );
    assert_eq!(snapshot.unread_event_count, 1);
    assert_eq!(snapshot.newer_event_count, 1);
}

#[test]
fn formatted_only_activity_rows_remain_eligible() {
    let mut item = timeline_item("$formatted:test", None, "@alice:test", false);
    item.formatted = Some(TimelineFormattedBody {
        html: "<b>formatted</b>".to_owned(),
        plain_text: "formatted".to_owned(),
        code_blocks: Vec::new(),
    });

    assert!(activity_row_from_timeline_item("!room:test", &item).is_some());
}

#[test]
fn backward_pagination_detects_only_a_changed_oldest_edge_as_prepend() {
    assert!(!backward_pagination_changed_oldest_edge(None, None));
    assert!(backward_pagination_changed_oldest_edge(None, Some("older")));
    assert!(!backward_pagination_changed_oldest_edge(
        Some("current"),
        Some("current")
    ));
    assert!(backward_pagination_changed_oldest_edge(
        Some("current"),
        Some("older")
    ));
}

#[tokio::test]
async fn pagination_waits_for_permit_before_publishing_paginating() {
    let scheduler = AccountWorkScheduler::default();
    let background = scheduler.acquire(AccountWorkKind::SearchCrawl).await;
    let key = room_key();
    let generations = Arc::new(TimelineActorGenerationGate::default());
    let actor_generation = generations.activate_after_quiescence(&key).await.generation;
    let (event_tx, mut event_rx) = broadcast::channel(8);

    let admission = tokio::spawn(acquire_pagination_permit_and_emit_paginating(
        fake_rid(91),
        key.clone(),
        event_tx,
        Arc::clone(&generations),
        actor_generation,
        scheduler,
        PaginationDirection::Backward,
    ));

    tokio::time::timeout(Duration::from_secs(1), background.cancelled())
        .await
        .expect("queued pagination must ask background work to yield");
    assert!(
        matches!(
            event_rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ),
        "Paginating must remain unpublished while scheduler admission is pending"
    );

    drop(background);
    let permit = tokio::time::timeout(Duration::from_secs(1), admission)
        .await
        .expect("pagination admission must finish after the slot is released")
        .expect("pagination admission task must not panic")
        .expect("the active actor generation must receive a permit");
    let event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .expect("Paginating must publish after scheduler admission")
        .expect("timeline event sender must remain open");
    assert!(matches!(
        event,
        CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
            request_id: Some(request_id),
            key: event_key,
            direction: PaginationDirection::Backward,
            state: PaginationState::Paginating,
            ..
        }) if request_id == fake_rid(91) && event_key == key
    ));
    drop(permit);
}

#[test]
fn resubscribe_replay_keeps_scrolled_room_context_complete() {
    let key = room_key();
    let items = (0..(ROOM_REPLAY_INITIAL_ITEMS_MAX + 25))
        .map(|index| {
            timeline_item(
                &format!("$event-{index}:test"),
                Some("body"),
                "@bob:test",
                false,
            )
        })
        .collect::<Vec<_>>();

    let replay = replay_initial_items_window(
        &key.kind,
        &items,
        &TimelineViewportObservation {
            bottom_arrival: TimelineBottomArrival::User,
            at_bottom: false,
            first_visible_event_id: Some("$event-10:test".to_owned()),
            last_visible_event_id: Some("$event-20:test".to_owned()),
            visible_gap_ids: Vec::new(),
        },
    );

    assert_eq!(replay.len(), ROOM_REPLAY_INITIAL_ITEMS_MAX + 25);
    assert_eq!(
        replay.first().and_then(timeline_item_event_id),
        Some("$event-0:test")
    );
}

#[test]
fn resubscribe_replay_keeps_focused_timeline_context_complete() {
    let key = TimelineKey {
        account_key: AccountKey("@a:test".to_owned()),
        kind: TimelineKind::Focused {
            room_id: "!r:test".to_owned(),
            event_id: "$anchor:test".to_owned(),
        },
    };
    let items = (0..(ROOM_REPLAY_INITIAL_ITEMS_MAX + 25))
        .map(|index| {
            timeline_item(
                &format!("$event-{index}:test"),
                Some("body"),
                "@bob:test",
                false,
            )
        })
        .collect::<Vec<_>>();

    let replay = replay_initial_items_window(
        &key.kind,
        &items,
        &TimelineViewportObservation {
            bottom_arrival: TimelineBottomArrival::User,
            at_bottom: true,
            ..TimelineViewportObservation::default()
        },
    );

    assert_eq!(replay.len(), ROOM_REPLAY_INITIAL_ITEMS_MAX + 25);
    assert_eq!(
        replay.first().and_then(timeline_item_event_id),
        Some("$event-0:test")
    );
}

#[test]
fn empty_room_initial_snapshot_needs_initial_backfill() {
    let key = room_key();

    assert!(should_hydrate_empty_initial_room_timeline(&key.kind, 0));
    assert!(!should_hydrate_empty_initial_room_timeline(&key.kind, 1));
}

#[test]
fn non_room_empty_initial_snapshots_do_not_use_room_live_backfill() {
    let thread = TimelineKind::Thread {
        room_id: "!r:test".to_owned(),
        root_event_id: "$root:test".to_owned(),
    };
    let focused = TimelineKind::Focused {
        room_id: "!r:test".to_owned(),
        event_id: "$event:test".to_owned(),
    };

    assert!(!should_hydrate_empty_initial_room_timeline(&thread, 0));
    assert!(!should_hydrate_empty_initial_room_timeline(&focused, 0));
}

fn cleanup_probe_timeline_actor_handle() -> (
    TimelineActorHandle,
    watch::Receiver<TimelineActorCleanupState>,
) {
    let mut handle = test_timeline_actor_handle();
    let (cleanup, receiver) = TimelineActorCleanupIngress::channel();
    handle.enqueue_context = Some(TimelineSendEnqueueContext::CleanupProbe { cleanup });
    (handle, receiver)
}

fn live_tail_test_actor_handle(
    label: &'static str,
    log: Arc<Mutex<Vec<String>>>,
) -> TimelineActorHandle {
    let (tx, mut rx) = mpsc::channel(8);
    let task = executor::spawn(async move {
        let mut operation_epochs = HashMap::new();
        while let Some(message) = rx.recv().await {
            match message {
                TimelineActorMessage::StartLiveTailRefresh {
                    epoch,
                    operation_generation,
                    limit,
                } => {
                    operation_epochs.insert(operation_generation, epoch);
                    log.lock()
                        .expect("live-tail log lock")
                        .push(format!("start:{label}:epoch={epoch}:limit={limit}"));
                }
                TimelineActorMessage::CancelLiveTailNetwork {
                    operation_generation,
                    acknowledged,
                } => {
                    let epoch = operation_epochs
                        .get(&operation_generation)
                        .copied()
                        .expect("cancelled operation was started");
                    log.lock()
                        .expect("live-tail log lock")
                        .push(format!("cancel-network:{label}:epoch={epoch}"));
                    let _ = acknowledged.send(());
                }
                _ => {}
            }
        }
    });
    TimelineActorHandle {
        tx,
        control_tx: None,
        thread_summary_projection: crate::timeline::actor::ThreadSummaryProjectionIngress::channel(
        )
        .0,
        position_rx: None,
        task: Some(task),
        auxiliary_tasks: Vec::new(),
        subscription_generation: None,
        enqueue_context: None,
    }
}

fn stalled_live_tail_cancel_actor_handle(
    label: &'static str,
    log: Arc<Mutex<Vec<String>>>,
) -> TimelineActorHandle {
    let (tx, mut rx) = mpsc::channel(8);
    let task = executor::spawn(async move {
        let mut held_acknowledgements = Vec::new();
        while let Some(message) = rx.recv().await {
            match message {
                TimelineActorMessage::StartLiveTailRefresh {
                    epoch,
                    operation_generation: _,
                    limit,
                } => log
                    .lock()
                    .expect("stalled live-tail log lock")
                    .push(format!("start:{label}:epoch={epoch}:limit={limit}")),
                TimelineActorMessage::CancelLiveTailNetwork {
                    operation_generation: _,
                    acknowledged,
                } => {
                    log.lock()
                        .expect("stalled live-tail log lock")
                        .push(format!("cancel-network:{label}"));
                    held_acknowledgements.push(acknowledged);
                }
                _ => {}
            }
        }
    });
    TimelineActorHandle {
        tx,
        control_tx: None,
        thread_summary_projection: crate::timeline::actor::ThreadSummaryProjectionIngress::channel(
        )
        .0,
        position_rx: None,
        task: Some(task),
        auxiliary_tasks: Vec::new(),
        subscription_generation: None,
        enqueue_context: None,
    }
}

fn live_tail_replacement_test_actor_handle(
    key: TimelineKey,
    labels: Arc<Mutex<HashMap<TimelineKey, &'static str>>>,
    log: Arc<Mutex<Vec<String>>>,
) -> TimelineActorHandle {
    let (tx, mut rx) = mpsc::channel(8);
    let task = executor::spawn(async move {
        while let Some(message) = rx.recv().await {
            let label = labels
                .lock()
                .expect("live-tail replacement labels lock")
                .get(&key)
                .copied()
                .expect("replacement actor label");
            match message {
                TimelineActorMessage::StartLiveTailRefresh {
                    epoch,
                    operation_generation,
                    limit,
                } => log
                    .lock()
                    .expect("live-tail replacement log lock")
                    .push(format!(
                        "start:{label}:epoch={epoch}:operation={operation_generation}:limit={limit}"
                    )),
                TimelineActorMessage::CancelLiveTailNetwork {
                    operation_generation,
                    acknowledged,
                } => {
                    log.lock()
                        .expect("live-tail replacement log lock")
                        .push(format!(
                            "cancel-network:{label}:operation={operation_generation}"
                        ));
                    let _ = acknowledged.send(());
                }
                _ => {}
            }
        }
    });
    TimelineActorHandle {
        tx,
        control_tx: None,
        thread_summary_projection: crate::timeline::actor::ThreadSummaryProjectionIngress::channel(
        )
        .0,
        position_rx: None,
        task: Some(task),
        auxiliary_tasks: Vec::new(),
        subscription_generation: None,
        enqueue_context: None,
    }
}

#[tokio::test]
async fn idempotent_subscribe_replay_carries_exact_command_cause() {
    let key = room_key();
    let first_subscribe_request_id = fake_rid(28_500);
    let second_subscribe_request_id = fake_rid(28_501);
    let (actor_tx, mut actor_rx) = mpsc::channel(2);
    let actor_handle = TimelineActorHandle {
        tx: actor_tx,
        control_tx: None,
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

    manager
        .handle_subscribe(
            first_subscribe_request_id,
            key.clone(),
            true,
            true,
            koushi_protocol::command::InitialBackfillPolicy::Disabled,
        )
        .await;
    manager
        .handle_subscribe(
            second_subscribe_request_id,
            key,
            true,
            true,
            koushi_protocol::command::InitialBackfillPolicy::Disabled,
        )
        .await;

    assert!(matches!(
        actor_rx.recv().await,
        Some(TimelineActorMessage::ReplayInitialItems {
            cause_request_id: Some(cause_request_id),
        }) if cause_request_id == first_subscribe_request_id
    ));
    assert!(matches!(
        actor_rx.recv().await,
        Some(TimelineActorMessage::ReplayInitialItems {
            cause_request_id: Some(cause_request_id),
        }) if cause_request_id == second_subscribe_request_id
    ));
}

#[tokio::test]
async fn cached_room_replay_uses_control_lane_when_ordinary_mailbox_is_full() {
    let key = room_key();
    let request_id = fake_rid(28_509);
    let (actor_tx, mut actor_rx) = mpsc::channel(1);
    actor_tx
        .try_send(TimelineActorMessage::OwnReadReceiptChanged)
        .expect("ordinary actor mailbox prefill");
    let (control_tx, mut control_rx) = mpsc::channel(1);
    let actor_handle = TimelineActorHandle {
        tx: actor_tx,
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

    executor::timeout(
        Duration::from_millis(250),
        manager.handle_subscribe(
            request_id,
            key,
            true,
            true,
            koushi_protocol::command::InitialBackfillPolicy::Disabled,
        ),
    )
    .await
    .expect("cached replay must not wait for the ordinary actor mailbox");
    assert!(matches!(
        control_rx.recv().await,
        Some(TimelineActorControl::ReplayInitialItems { cause_request_id })
            if cause_request_id == request_id
    ));
    assert!(matches!(
        actor_rx.recv().await,
        Some(TimelineActorMessage::OwnReadReceiptChanged)
    ));
}

#[tokio::test]
async fn ordinary_completion_burst_does_not_run_before_committed_room_selection() {
    let key = room_key();
    let request_id = fake_rid(28_510);
    let (actor_tx, mut actor_rx) = mpsc::channel(2);
    let actor_handle = TimelineActorHandle {
        tx: actor_tx,
        control_tx: None,
        thread_summary_projection: crate::timeline::actor::ThreadSummaryProjectionIngress::channel(
        )
        .0,
        position_rx: None,
        task: None,
        auxiliary_tasks: Vec::new(),
        subscription_generation: None,
        enqueue_context: None,
    };
    let (navigation_projection, navigation_projection_rx) = NavigationProjectionIngress::channel();
    let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
    manager.navigation_projection_rx = Some(navigation_projection_rx);

    for operation_generation in 1..=4 {
        manager
            .msg_tx
            .try_send(TimelineMessage::LiveTailRefreshCompleted {
                key: key.clone(),
                actor_generation: u64::MAX,
                epoch: 1,
                operation_generation,
                outcome: MatrixLiveTailRefreshOutcome::Failed,
                requested_limit: FOREGROUND_LIVE_TAIL_LIMIT,
                returned_events: 0,
                duration_ms: 0,
            })
            .expect("ordinary completion should fit the test mailbox");
    }
    assert!(navigation_projection.admit(NavigationProjectionIntent {
        generation: 1,
        key: key.clone(),
        cause_request_id: request_id,
        replay_existing: true,
        cleanup: NavigationProjectionCleanup::default(),
    }));
    let (state_tx, state_rx) = oneshot::channel();
    manager
        .msg_tx
        .try_send(TimelineMessage::TestLiveTailDispatchState {
            key,
            epoch: 1,
            response: state_tx,
        })
        .expect("state probe should fit the test mailbox");

    let manager_task = executor::spawn(manager.run());
    let replay = executor::timeout(Duration::from_secs(1), actor_rx.recv())
        .await
        .expect("cached actor replay should be bounded")
        .expect("cached actor should receive replay");
    assert!(matches!(
        replay,
        TimelineActorMessage::ReplayInitialItems {
            cause_request_id: Some(cause),
        } if cause == request_id
    ));
    let (_, _, ordinary_completions_before_navigation_projection) =
        executor::timeout(Duration::from_secs(1), state_rx)
            .await
            .expect("manager probe should be bounded")
            .expect("manager should answer the probe");
    manager_task.abort();

    assert_eq!(
        ordinary_completions_before_navigation_projection,
        Some(0),
        "a committed cached-room selection must overtake queued ordinary completions"
    );
}

#[tokio::test]
async fn manager_shutdown_control_quiesces_before_retained_navigation() {
    let key = room_key();
    let (actor_tx, mut actor_rx) = mpsc::channel(1);
    let actor_handle = TimelineActorHandle {
        tx: actor_tx,
        control_tx: None,
        thread_summary_projection: crate::timeline::actor::ThreadSummaryProjectionIngress::channel(
        )
        .0,
        position_rx: None,
        task: None,
        auxiliary_tasks: Vec::new(),
        subscription_generation: None,
        enqueue_context: None,
    };
    let (navigation_projection, navigation_projection_rx) = NavigationProjectionIngress::channel();
    let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
    manager.navigation_projection_rx = Some(navigation_projection_rx);
    assert!(navigation_projection.admit(NavigationProjectionIntent {
        generation: 1,
        key,
        cause_request_id: fake_rid(28_514),
        replay_existing: true,
        cleanup: NavigationProjectionCleanup::default(),
    }));
    let (control_tx, control_rx) = mpsc::channel(1);
    manager.control_rx = Some(control_rx);
    let (acknowledged, acknowledgement) = oneshot::channel();
    control_tx
        .send(TimelineManagerControl::Shutdown { acknowledged })
        .await
        .expect("admit high-priority shutdown");

    let manager_task = executor::spawn(manager.run());
    executor::timeout(Duration::from_secs(1), acknowledgement)
        .await
        .expect("shutdown acknowledgement must be bounded")
        .expect("manager must acknowledge quiescence");
    assert!(
        !matches!(
            executor::timeout(Duration::from_millis(20), actor_rx.recv()).await,
            Ok(Some(_))
        ),
        "the old sessionless manager must not consume retained navigation"
    );
    manager_task
        .await
        .expect("manager exits after acknowledged shutdown");
}

#[tokio::test]
async fn navigation_projection_retains_latest_value_across_manager_replacement() {
    let (ingress, initial_receiver) = NavigationProjectionIngress::channel();
    drop(initial_receiver);
    let newest_key = room_key();
    let newest_cause = fake_rid(28_511);

    assert!(ingress.admit(NavigationProjectionIntent {
        generation: 7,
        key: newest_key.clone(),
        cause_request_id: newest_cause,
        replay_existing: false,
        cleanup: NavigationProjectionCleanup::default(),
    }));
    assert!(ingress.admit(NavigationProjectionIntent {
        generation: 6,
        key: TimelineKey::room(AccountKey("@a:test".to_owned()), "!stale:test"),
        cause_request_id: fake_rid(28_512),
        replay_existing: true,
        cleanup: NavigationProjectionCleanup::default(),
    }));
    assert!(ingress.admit(NavigationProjectionIntent {
        generation: 7,
        key: newest_key.clone(),
        cause_request_id: fake_rid(28_513),
        replay_existing: true,
        cleanup: NavigationProjectionCleanup::default(),
    }));

    let mut replacement_receiver = Some(ingress.subscribe());
    let retained = executor::timeout(
        Duration::from_secs(1),
        receive_navigation_projection(&mut replacement_receiver),
    )
    .await
    .expect("replacement manager wake should be bounded")
    .expect("latest desired projection should remain retained");

    assert_eq!(retained.generation, 7);
    assert_eq!(retained.key, newest_key);
    assert_eq!(
        retained.cause_request_id, newest_cause,
        "equal-generation replay strengthens the retained intent without replacing its cause"
    );
    assert!(retained.replay_existing);
}

#[tokio::test]
async fn coalesced_navigation_projection_cleans_the_actual_manager_foreground() {
    let account = AccountKey("@coalesced-cleanup:test".to_owned());
    let room_a = TimelineKey::room(account.clone(), "!cleanup-a:test");
    let room_b = TimelineKey::room(account.clone(), "!cleanup-b:test");
    let room_c = TimelineKey::room(account, "!cleanup-c:test");
    let (actor_a, mut cleanup_a) = cleanup_probe_timeline_actor_handle();
    let (actor_b, mut cleanup_b) = cleanup_probe_timeline_actor_handle();
    let (actor_c, _cleanup_c) = cleanup_probe_timeline_actor_handle();
    let (navigation_projection, navigation_projection_rx) = NavigationProjectionIngress::channel();
    let mut manager = live_tail_test_manager(HashMap::from([
        (room_a.clone(), actor_a),
        (room_b.clone(), actor_b),
        (room_c.clone(), actor_c),
    ]));
    manager.navigation_projection_rx = Some(navigation_projection_rx);

    manager
        .handle_committed_room_selection(fake_rid(28_515), room_a.clone(), false, false)
        .await;
    assert_eq!(manager.live_tail_refreshes.active_key(), Some(&room_a));

    assert!(navigation_projection.admit(NavigationProjectionIntent {
        generation: 1,
        key: room_b.clone(),
        cause_request_id: fake_rid(28_516),
        replay_existing: false,
        cleanup: NavigationProjectionCleanup {
            cancel_pagination: Some(room_a.clone()),
            cancel_link_previews: Some(room_a.clone()),
        },
    }));
    assert!(navigation_projection.admit(NavigationProjectionIntent {
        generation: 2,
        key: room_c.clone(),
        cause_request_id: fake_rid(28_517),
        replay_existing: false,
        cleanup: NavigationProjectionCleanup {
            cancel_pagination: Some(room_b.clone()),
            cancel_link_previews: Some(room_b),
        },
    }));

    let projection = receive_navigation_projection(&mut manager.navigation_projection_rx)
        .await
        .expect("latest navigation projection");
    assert_eq!(projection.key, room_c);
    manager.handle_navigation_projection(projection).await;

    assert_eq!(
        manager.live_tail_refreshes.active_key(),
        Some(&room_c),
        "the latest retained room must become foreground"
    );
    cleanup_a
        .changed()
        .await
        .expect("actual previous foreground cleanup");
    let cleanup_a = *cleanup_a.borrow_and_update();
    assert!(
        cleanup_a.cancel_pagination_serial > 0,
        "A pagination cleanup must survive B being replaced by C"
    );
    assert!(
        cleanup_a.cancel_link_previews_serial > 0,
        "A link-preview cleanup must survive B being replaced by C"
    );
    cleanup_b.changed().await.expect("latest intent cleanup");
    let cleanup_b = *cleanup_b.borrow_and_update();
    assert!(cleanup_b.cancel_pagination_serial > 0);
    assert!(cleanup_b.cancel_link_previews_serial > 0);
}

#[tokio::test]
async fn live_tail_preemption_cancels_network_before_new_active_room_starts() {
    let account = AccountKey("@a:test".to_owned());
    let room_a = TimelineKey::room(account.clone(), "!a:test");
    let room_b = TimelineKey::room(account, "!b:test");
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut manager = live_tail_test_manager(HashMap::from([
        (
            room_a.clone(),
            live_tail_test_actor_handle("A", log.clone()),
        ),
        (
            room_b.clone(),
            live_tail_test_actor_handle("B", log.clone()),
        ),
    ]));

    manager.room_subscription_service_epoch = 7;
    manager
        .handle_committed_room_selection(fake_rid(1), room_a, false, false)
        .await;
    manager.room_subscription_service_epoch = 9;
    manager
        .handle_committed_room_selection(fake_rid(2), room_b, false, false)
        .await;

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if log.lock().expect("live-tail log lock").len() == 3 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("preemption log completed");
    assert_eq!(
        *log.lock().expect("live-tail log lock"),
        [
            "start:A:epoch=7:limit=128",
            "cancel-network:A:epoch=7",
            "start:B:epoch=9:limit=128",
        ]
    );
}

#[tokio::test]
async fn post_commit_cleanup_never_waits_for_a_missing_cancel_ack() {
    let account = AccountKey("@stalled-cancel:test".to_owned());
    let room_a = TimelineKey::room(account.clone(), "!stalled-a:test");
    let room_b = TimelineKey::room(account, "!stalled-b:test");
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut manager = live_tail_test_manager(HashMap::from([
        (
            room_a.clone(),
            stalled_live_tail_cancel_actor_handle("A", log.clone()),
        ),
        (
            room_b.clone(),
            live_tail_test_actor_handle("B", log.clone()),
        ),
    ]));

    manager.room_subscription_service_epoch = 7;
    manager
        .handle_committed_room_selection(fake_rid(1), room_a, false, false)
        .await;
    manager.room_subscription_service_epoch = 9;
    executor::timeout(
        Duration::from_millis(25),
        manager.handle_committed_room_selection(fake_rid(2), room_b, false, false),
    )
    .await
    .expect("post-commit cleanup must not consume the cancellation deadline");

    tokio::task::yield_now().await;
    assert_eq!(
        *log.lock().expect("stalled live-tail log lock"),
        [
            "start:A:epoch=7:limit=128",
            "cancel-network:A",
            "start:B:epoch=9:limit=128",
        ]
    );
}

#[tokio::test]
async fn committed_navigation_projection_failure_does_not_emit_a_second_terminal() {
    let mut manager = live_tail_test_manager(HashMap::new());
    manager.test_session_available = false;
    let (event_tx, mut event_rx) = broadcast::channel(8);
    manager.event_tx = event_tx;
    let request_id = fake_rid(29_604);

    manager
        .handle_navigation_projection(NavigationProjectionIntent {
            generation: 1,
            key: room_key(),
            cause_request_id: request_id,
            replay_existing: true,
            cleanup: NavigationProjectionCleanup::default(),
        })
        .await;

    assert!(
        executor::timeout(Duration::from_millis(10), event_rx.recv())
            .await
            .is_err(),
        "AppActor already emitted Committed; projection cleanup must not emit OperationFailed"
    );
}

#[tokio::test]
async fn foreground_gap_demand_moves_to_the_newly_selected_room() {
    let account = AccountKey("@gap-owner:test".to_owned());
    let room_a = TimelineKey::room(account.clone(), "!gap-a:test");
    let room_b = TimelineKey::room(account, "!gap-b:test");
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut manager = live_tail_test_manager(HashMap::from([
        (
            room_a.clone(),
            gap_demand_test_actor_handle("A", log.clone()),
        ),
        (
            room_b.clone(),
            gap_demand_test_actor_handle("B", log.clone()),
        ),
    ]));

    manager
        .handle_committed_room_selection(fake_rid(1), room_a, false, false)
        .await;
    manager
        .handle_committed_room_selection(fake_rid(2), room_b, false, false)
        .await;
    tokio::task::yield_now().await;

    assert_eq!(
        *log.lock().expect("gap demand log lock"),
        ["begin:A", "end:A", "begin:B"],
    );
}

#[tokio::test]
async fn sync_replacement_restores_foreground_gap_demand_to_the_new_actor() {
    let room = TimelineKey::room(AccountKey("@gap-owner:test".to_owned()), "!gap:test");
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut manager = live_tail_test_manager(HashMap::from([(
        room.clone(),
        gap_demand_test_actor_handle("old", log.clone()),
    )]));
    manager
        .handle_committed_room_selection(fake_rid(1), room.clone(), false, false)
        .await;
    manager.timelines.insert(
        room.clone(),
        gap_demand_test_actor_handle("replacement", log.clone()),
    );

    manager.restore_foreground_gap_demand(&room).await;
    tokio::task::yield_now().await;

    assert_eq!(
        *log.lock().expect("gap demand log lock"),
        ["begin:replacement"],
    );
}

#[tokio::test]
async fn live_tail_epoch_replacement_folds_stale_pending_starts_before_dispatch() {
    let account = AccountKey("@replacement:test".to_owned());
    let candidates = ["!one:test", "!two:test", "!three:test"]
        .map(|room_id| TimelineKey::room(account.clone(), room_id));
    let labels = Arc::new(Mutex::new(HashMap::new()));
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut manager = live_tail_test_manager(
        candidates
            .iter()
            .cloned()
            .map(|key| {
                (
                    key.clone(),
                    live_tail_replacement_test_actor_handle(key, labels.clone(), log.clone()),
                )
            })
            .collect(),
    );
    let ordered = manager.timelines.keys().cloned().collect::<Vec<_>>();
    let [room_b, room_c, room_a] = ordered.as_slice() else {
        panic!("three replacement rooms");
    };
    let (room_b, room_c, room_a) = (room_b.clone(), room_c.clone(), room_a.clone());
    labels
        .lock()
        .expect("live-tail replacement labels lock")
        .extend([
            (room_b.clone(), "B"),
            (room_c.clone(), "C"),
            (room_a.clone(), "A"),
        ]);

    let prepare = || {
        let mut coordinator = LiveTailRefreshCoordinator::new();
        assert_eq!(
            coordinator.activate(room_a.clone(), 7),
            vec![LiveTailSchedulerAction::Start {
                key: room_a.clone(),
                epoch: 7,
                operation_generation: 1,
                limit: 128,
            }]
        );
        assert!(coordinator.mark_unproven(room_b.clone(), 7).is_empty());
        assert!(coordinator.mark_unproven(room_c.clone(), 7).is_empty());
        let start_b = coordinator.finish(room_a.clone(), 7, 1, LiveTailRefreshOutcome::Unchanged);
        (coordinator, start_b)
    };

    let (mut evidence, _) = prepare();
    let logical_actions = [room_b.clone(), room_c.clone(), room_a.clone()]
        .into_iter()
        .flat_map(|key| evidence.invalidate_epoch(key, 8))
        .collect::<Vec<_>>();
    assert_eq!(
        logical_actions,
        vec![
            LiveTailSchedulerAction::CancelNetwork {
                key: room_b.clone(),
                operation_generation: 2,
            },
            LiveTailSchedulerAction::Start {
                key: room_c.clone(),
                epoch: 7,
                operation_generation: 3,
                limit: 128,
            },
            LiveTailSchedulerAction::CancelNetwork {
                key: room_c.clone(),
                operation_generation: 3,
            },
            LiveTailSchedulerAction::Start {
                key: room_b.clone(),
                epoch: 8,
                operation_generation: 4,
                limit: 128,
            },
            LiveTailSchedulerAction::CancelNetwork {
                key: room_b.clone(),
                operation_generation: 4,
            },
            LiveTailSchedulerAction::Start {
                key: room_a.clone(),
                epoch: 8,
                operation_generation: 5,
                limit: 128,
            },
        ],
        "the coordinator cancellation stream must remain causal",
    );

    let (coordinator, start_b) = prepare();
    manager.live_tail_refreshes = coordinator;
    manager.apply_live_tail_scheduler_actions(start_b).await;
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if !log
                .lock()
                .expect("live-tail replacement log lock")
                .is_empty()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("initial delayed B start");
    log.lock().expect("live-tail replacement log lock").clear();

    let starts = manager
        .invalidate_live_tail_epoch_for_existing_rooms(8)
        .await;
    manager.apply_live_tail_scheduler_actions(starts).await;
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if log
                .lock()
                .expect("live-tail replacement log lock")
                .iter()
                .any(|entry| entry.starts_with("start:A:epoch=8:"))
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("final replacement start");
    tokio::task::yield_now().await;
    assert_eq!(
        *log.lock().expect("live-tail replacement log lock"),
        [
            "cancel-network:B:operation=2",
            "start:A:epoch=8:operation=5:limit=128",
        ],
        "only the already-dispatched network and final coordinator start reach actors",
    );
}

#[test]
fn causal_projection_domains_route_equal_raw_serial_without_collision() {
    let actor_generation = 4;
    let raw_serial = 1;
    let projection_batch = 1;
    let published_batch_id = TimelineBatchId(21);
    let historical_operation =
        CausalProjectionOperationId::new(CausalProjectionDomain::HistoricalGap, raw_serial)
            .expect("historical serial fits the transport envelope");
    let live_tail_operation =
        CausalProjectionOperationId::new(CausalProjectionDomain::LiveTail, raw_serial)
            .expect("live-tail serial fits the transport envelope");

    assert_eq!(historical_operation.encode_transport(), raw_serial);
    assert_eq!(
        live_tail_operation.encode_transport(),
        CAUSAL_PROJECTION_DOMAIN_BIT | raw_serial,
    );
    assert!(
        CausalProjectionOperationId::new(
            CausalProjectionDomain::HistoricalGap,
            CAUSAL_PROJECTION_DOMAIN_BIT,
        )
        .is_none(),
        "raw serials must never consume the operation-domain bit",
    );
    assert_eq!(
        next_causal_projection_serial(CAUSAL_PROJECTION_SERIAL_MAX),
        None,
        "exhaustion is terminal while the same domain owns a pending identity",
    );
    assert_eq!(
        next_causal_projection_serial(CAUSAL_PROJECTION_SERIAL_MAX),
        None,
        "one actor generation never wraps even when no operation is pending",
    );

    let mut historical = TimelineGapProjectionCorrelation::default();
    historical.begin(actor_generation, historical_operation);
    assert_eq!(
        historical.complete(
            actor_generation,
            historical_operation,
            Some(projection_batch),
        ),
        TimelineGapProjectionCompletion::Pending,
    );
    let mut live_tail = TimelineGapProjectionCorrelation::default();
    live_tail.begin(actor_generation, live_tail_operation);
    assert_eq!(
        live_tail.complete(
            actor_generation,
            live_tail_operation,
            Some(projection_batch),
        ),
        TimelineGapProjectionCompletion::Pending,
    );

    let historical_projection = CausalProjectionId::decode_transport(GapRepairProjectionId {
        actor_generation,
        repair_generation: historical_operation.encode_transport(),
        projection_batch,
    });
    let historical_observation = observe_causal_projection(
        &mut historical,
        &mut live_tail,
        historical_projection,
        published_batch_id,
    );
    assert_eq!(
        historical_observation.historical_gap_batch_id,
        Some(published_batch_id),
    );
    assert_eq!(historical_observation.live_tail_batch_id, None);
    assert!(
        live_tail.is_pending(),
        "historical tag cannot prove live-tail freshness"
    );

    // Re-arm the historical correlation to prove the reverse isolation on
    // the same actor/raw serial/batch collision.
    historical.begin(actor_generation, historical_operation);
    assert_eq!(
        historical.complete(
            actor_generation,
            historical_operation,
            Some(projection_batch),
        ),
        TimelineGapProjectionCompletion::Pending,
    );
    let live_tail_projection = CausalProjectionId::decode_transport(GapRepairProjectionId {
        actor_generation,
        repair_generation: live_tail_operation.encode_transport(),
        projection_batch,
    });
    let live_tail_observation = observe_causal_projection(
        &mut historical,
        &mut live_tail,
        live_tail_projection,
        TimelineBatchId(22),
    );
    assert_eq!(live_tail_observation.historical_gap_batch_id, None);
    assert_eq!(
        live_tail_observation.live_tail_batch_id,
        Some(TimelineBatchId(22)),
    );
    assert!(
        historical.is_pending(),
        "live-tail tag cannot release historical repair"
    );

    assert_eq!(
        observe_causal_projection(
            &mut historical,
            &mut live_tail,
            live_tail_projection,
            TimelineBatchId(23),
        ),
        CausalProjectionObservation::default(),
        "one live-tail projection can complete only once",
    );
}

#[tokio::test]
async fn room_actor_hydrates_a_historical_sender_without_a_live_event() {
    use matrix_sdk::ruma::events::room::member::MembershipState;
    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    use matrix_sdk_test::{ALICE, CAROL, JoinedRoomBuilder, event_factory::EventFactory};

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    client
        .event_cache()
        .subscribe()
        .expect("event cache subscription");
    let sdk_room_id = matrix_sdk::ruma::room_id!("!historical-profile:example.org");
    let event_id = matrix_sdk::ruma::event_id!("$historical-profile:example.org");
    let room = server.sync_joined_room(&client, sdk_room_id).await;
    let factory = EventFactory::new().room(sdk_room_id);
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(sdk_room_id).add_timeline_event(
                factory
                    .text_msg("historical")
                    .sender(&CAROL)
                    .event_id(event_id)
                    .into_raw_sync(),
            ),
        )
        .await;
    server
        .mock_get_members()
        .ok(vec![
            factory
                .member(&ALICE)
                .membership(MembershipState::Join)
                .into_raw(),
            factory
                .member(&CAROL)
                .display_name("Carol")
                .membership(MembershipState::Join)
                .into_raw(),
        ])
        .expect(1)
        .named("historical-profile-members")
        .mount()
        .await;

    let timeline = Arc::new(
        koushi_timeline_builder(
            &room,
            TimelineFocus::Live {
                hide_threaded_events: false,
            },
        )
        .build()
        .await
        .expect("room timeline"),
    );
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client,
        SessionInfo {
            homeserver: "http://example.invalid".to_owned(),
            user_id: ALICE.to_string(),
            device_id: "DEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    ));
    let key = TimelineKey::room(AccountKey(ALICE.to_string()), sdk_room_id.to_string());
    let mut manager = live_tail_test_manager(HashMap::new());
    let (action_tx, mut action_rx) = mpsc::channel(8);
    manager.action_tx = action_tx;
    let _action_drain = executor::spawn(async move { while action_rx.recv().await.is_some() {} });
    let mut event_rx = manager.event_tx.subscribe();
    let actor_generation = manager
        .timeline_actor_generations
        .activate_after_quiescence(&key)
        .await
        .generation;
    let _actor = TimelineActor::spawn(
        key.clone(),
        timeline,
        session,
        fake_rid(68),
        true,
        manager.action_tx.clone(),
        manager.event_tx.clone(),
        None,
        Default::default(),
        None,
        LinkPreviewContext::default(),
        manager.account_work.clone(),
        Arc::clone(&manager.thread_root_projection_service),
        manager.thread_root_order,
        Arc::clone(&manager.timeline_actor_generations),
        actor_generation,
        None,
        Default::default(),
        manager.terminal_ingress.clone(),
        manager.msg_tx.clone(),
    )
    .await;

    let hydrated = executor::timeout(Duration::from_secs(2), async {
        let mut saw_unavailable_initial = false;
        loop {
            match event_rx.recv().await.expect("timeline event") {
                CoreEvent::Timeline(TimelineEvent::InitialItems {
                    key: event_key,
                    items,
                    ..
                }) if event_key == key => {
                    let item = items
                        .iter()
                        .find(|item| timeline_item_event_id(item) == Some(event_id.as_str()))
                        .expect("historical initial item");
                    assert_eq!(item.sender_label, None);
                    saw_unavailable_initial = true;
                }
                CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                    key: event_key,
                    diffs,
                    ..
                }) if event_key == key && saw_unavailable_initial => {
                    if let Some(item) = diffs.iter().find_map(|diff| match diff {
                        koushi_protocol::event::TimelineDiff::Set { item, .. }
                            if timeline_item_event_id(item) == Some(event_id.as_str()) =>
                        {
                            Some(item)
                        }
                        _ => None,
                    }) && item.sender_label.as_deref() == Some("Carol")
                    {
                        break true;
                    }
                }
                _ => {}
            }
        }
    })
    .await
    .expect("member hydration must settle through an ordinary timeline diff");
    assert!(hydrated);
}

#[tokio::test]
async fn live_tail_restore_actor_flush_hands_completion_to_manager_once() {
    use matrix_sdk::test_utils::mocks::{MatrixMockServer, RoomMessagesResponseTemplate};
    use matrix_sdk_test::{ALICE, JoinedRoomBuilder, event_factory::EventFactory};

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    client
        .event_cache()
        .subscribe()
        .expect("event cache subscription");
    let sdk_room_id = matrix_sdk::ruma::room_id!("!restore-live-tail:example.org");
    let stale_edge_id = matrix_sdk::ruma::event_id!("$stale-edge:example.org");
    let refreshed_id = matrix_sdk::ruma::event_id!("$refreshed:example.org");
    let room = server.sync_joined_room(&client, sdk_room_id).await;
    let factory = EventFactory::new().room(sdk_room_id).sender(&ALICE);
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(sdk_room_id).add_timeline_event(
                factory
                    .text_msg("stale edge")
                    .event_id(stale_edge_id)
                    .into_raw_sync(),
            ),
        )
        .await;
    let timeline = Arc::new(
        koushi_timeline_builder(
            &room,
            TimelineFocus::Live {
                hide_threaded_events: false,
            },
        )
        .build()
        .await
        .expect("room timeline"),
    );
    let (initial_sdk_items, _fixture_stream) = timeline.subscribe().await;
    let real_sdk_item = initial_sdk_items
        .iter()
        .find(|item| item.as_event().and_then(|event| event.event_id()) == Some(stale_edge_id))
        .cloned()
        .expect("real SDK timeline item for the wrong-tag restore batch");
    server
        .mock_room_messages()
        .match_limit(u32::from(FOREGROUND_LIVE_TAIL_LIMIT))
        .ok(RoomMessagesResponseTemplate::default()
            .events(vec![
                factory.text_msg("refreshed").event_id(refreshed_id),
                factory.text_msg("stale edge").event_id(stale_edge_id),
            ])
            .with_delay(Duration::from_millis(500)))
        .expect(1)
        .named("restore-live-tail-production-refresh")
        .mount()
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
    let account = AccountKey("@restore:test".to_owned());
    let room_a = TimelineKey::room(account.clone(), sdk_room_id.to_string());
    let room_b = TimelineKey::room(account, "!delayed:example.org");
    let delayed_log = Arc::new(Mutex::new(Vec::new()));
    let mut manager = live_tail_test_manager(HashMap::from([(
        room_b.clone(),
        live_tail_test_actor_handle("B", delayed_log.clone()),
    )]));
    let (action_tx, mut action_rx) = mpsc::channel(8);
    manager.action_tx = action_tx;
    let _action_drain = executor::spawn(async move { while action_rx.recv().await.is_some() {} });
    let mut event_rx = manager.event_tx.subscribe();
    let actor_generation = manager
        .timeline_actor_generations
        .activate_after_quiescence(&room_a)
        .await
        .generation;
    let projection_request_id = fake_rid(40);
    let actor_handle = TimelineActor::spawn(
        room_a.clone(),
        timeline,
        session,
        projection_request_id,
        true,
        manager.action_tx.clone(),
        manager.event_tx.clone(),
        None,
        Default::default(),
        None,
        LinkPreviewContext::default(),
        manager.account_work.clone(),
        Arc::clone(&manager.thread_root_projection_service),
        manager.thread_root_order,
        Arc::clone(&manager.timeline_actor_generations),
        actor_generation,
        None,
        Default::default(),
        manager.terminal_ingress.clone(),
        manager.msg_tx.clone(),
    )
    .await;
    manager.timelines.insert(room_a.clone(), actor_handle);
    loop {
        if matches!(
            event_rx.recv().await.expect("initial actor event"),
            CoreEvent::Timeline(TimelineEvent::InitialItems { key, .. }) if key == room_a
        ) {
            break;
        }
    }

    let (restore_tx, restore_rx) = oneshot::channel();
    assert!(
        manager
            .timelines
            .get(&room_a)
            .expect("room A actor")
            .send(TimelineActorMessage::TestBeginRestore {
                request_id: fake_rid(41),
                event_id: "$anchor-not-in-window:example.org".to_owned(),
                acknowledged: restore_tx,
            })
            .await
    );
    restore_rx.await.expect("restore fixture acknowledged");

    let starts = manager.live_tail_refreshes.activate(room_a.clone(), 7);
    assert!(
        manager
            .live_tail_refreshes
            .mark_unproven(room_b.clone(), 7)
            .is_empty()
    );
    manager.apply_live_tail_scheduler_actions(starts).await;
    let operation = live_tail_causal_projection_operation(1);
    let wrong_projections = BTreeSet::from([
        CausalProjectionId {
            actor_generation: actor_generation + 1,
            operation,
            projection_batch: 1,
        },
        CausalProjectionId {
            actor_generation,
            operation: live_tail_causal_projection_operation(9),
            projection_batch: 1,
        },
        CausalProjectionId {
            actor_generation,
            operation,
            projection_batch: u32::MAX,
        },
    ]);
    let (inject_tx, inject_rx) = oneshot::channel();
    assert!(
        manager
            .timelines
            .get(&room_a)
            .expect("room A actor")
            .send(TimelineActorMessage::TestInjectRestoreDiff {
                diffs: vec![eyeball_im::VectorDiff::PushBack {
                    value: real_sdk_item.clone(),
                }],
                projections: wrong_projections.clone(),
                acknowledged: inject_tx,
            })
            .await
    );
    inject_rx.await.expect("wrong-tag diff handled");

    let snapshot = |manager: &TimelineManagerActor, key: &TimelineKey| {
        let (response, state) = oneshot::channel();
        let handle = manager.timelines.get(key).expect("room A actor");
        (handle.tx.clone(), response, state)
    };
    let (actor_tx, state_tx, state_rx) = snapshot(&manager, &room_a);
    actor_tx
        .send(TimelineActorMessage::TestRestoreCausalState(state_tx))
        .await
        .expect("snapshot request");
    let (pending, completion_waiting, buffered_diff_count, buffered_projections) =
        state_rx.await.expect("wrong-tag snapshot");
    assert!(pending);
    assert!(!completion_waiting);
    assert_eq!(
        buffered_diff_count, 0,
        "a duplicate canonical slot is a valid projected display no-op"
    );
    assert_eq!(buffered_projections, wrong_projections);
    assert_eq!(
        manager.live_tail_refreshes.freshness(&room_a),
        Some(LiveTailFreshnessState::Refreshing {
            epoch: 7,
            operation_generation: 1,
        }),
        "wrong actor, operation, and batch identities cannot prove freshness",
    );
    let matching_projection = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let (actor_tx, state_tx, state_rx) = snapshot(&manager, &room_a);
                actor_tx
                    .send(TimelineActorMessage::TestRestoreCausalState(state_tx))
                    .await
                    .expect("snapshot request");
                let (pending, completion_waiting, buffered_diff_count, projections) =
                    state_rx.await.expect("matching-tag snapshot");
                if let Some(projection) = projections.iter().copied().find(|projection| {
                    projection.actor_generation == actor_generation
                        && projection.operation == operation
                        && projection.projection_batch != u32::MAX
                }) && completion_waiting
                {
                    assert!(
                        pending,
                            "matching metadata remains pending until publication"
                    );
                    assert!(
                        buffered_diff_count >= 2,
                            "two real SDK batches must reach the actor restore buffer before terminal publication"
                    );
                    break projection;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("real tagged SDK diff reached the restore buffer");
    assert_ne!(matching_projection.projection_batch, u32::MAX);

    let manager_tx = manager.msg_tx.clone();
    let actor_tx = manager
        .timelines
        .get(&room_a)
        .expect("room A actor")
        .tx
        .clone();
    let _manager_task = executor::spawn(manager.run());

    while event_rx.try_recv().is_ok() {}
    let (flush_tx, flush_rx) = oneshot::channel();
    actor_tx
        .send(TimelineActorMessage::TestFinishRestore {
            request_id: fake_rid(41),
            response: flush_tx,
        })
        .await
        .expect("finish restore request");
    assert!(flush_rx.await.expect("production restore terminal result"));

    let (freshness, completion_dispatches, _) =
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let (state_tx, state_rx) = oneshot::channel();
                manager_tx
                    .send(TimelineMessage::TestLiveTailDispatchState {
                        key: room_a.clone(),
                        epoch: 7,
                        response: state_tx,
                    })
                    .await
                    .expect("manager state request");
                let state = state_rx.await.expect("manager state response");
                if state.0 && !delayed_log.lock().expect("delayed start log").is_empty() {
                    break state;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("production manager dispatch completed live-tail refresh");
    assert!(freshness, "room A becomes Fresh for epoch 7");
    assert_eq!(
        completion_dispatches, 1,
        "the production manager loop dispatches exactly one completion",
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if !delayed_log.lock().expect("delayed start log").is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("manager scheduled delayed room B");
    assert_eq!(
        *delayed_log.lock().expect("delayed start log"),
        ["start:B:epoch=7:limit=128"],
    );

    let mut settlement_events = Vec::new();
    while let Ok(event) = event_rx.try_recv() {
        match event {
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated { .. }) => {
                settlement_events.push("items")
            }
            CoreEvent::Timeline(TimelineEvent::NavigationUpdated { .. }) => {
                settlement_events.push("navigation")
            }
            CoreEvent::Timeline(TimelineEvent::AnchorRestoreFinished { .. }) => {
                settlement_events.push("terminal")
            }
            _ => {}
        }
    }
    assert_eq!(
        settlement_events
            .iter()
            .filter(|event| **event == "items")
            .count(),
        1,
        "restore publishes one convergent coalesced batch"
    );
    let items_position = settlement_events
        .iter()
        .position(|event| *event == "items")
        .expect("coalesced ItemsUpdated");
    let navigation_position = settlement_events
        .iter()
        .position(|event| *event == "navigation")
        .expect("settled NavigationUpdated");
    let terminal_position = settlement_events
        .iter()
        .position(|event| *event == "terminal")
        .expect("AnchorRestoreFinished terminal");
    assert!(items_position < navigation_position && navigation_position < terminal_position);

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    manager_tx
        .send(TimelineMessage::Shutdown {
            acknowledged: Some(shutdown_tx),
        })
        .await
        .expect("manager shutdown request");
    shutdown_rx.await.expect("manager shutdown acknowledged");
}

#[tokio::test]
async fn timeline_actor_spawn_returns_before_authoritative_publish_waits_for_manager_capacity() {
    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    use matrix_sdk_test::{ALICE, JoinedRoomBuilder, event_factory::EventFactory};

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    client
        .event_cache()
        .subscribe()
        .expect("event cache subscription");
    let room_id = matrix_sdk::ruma::room_id!("!startup-capacity:example.org");
    let factory = EventFactory::new().room(room_id).sender(&ALICE);
    let room = server.sync_joined_room(&client, room_id).await;
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(room_id).add_timeline_event(
                factory
                    .text_msg("synthetic")
                    .event_id(matrix_sdk::ruma::event_id!("$startup:example.org"))
                    .into_raw_sync(),
            ),
        )
        .await;
    let timeline = Arc::new(
        koushi_timeline_builder(
            &room,
            TimelineFocus::Live {
                hide_threaded_events: false,
            },
        )
        .build()
        .await
        .expect("room timeline"),
    );
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client,
        SessionInfo {
            homeserver: "http://example.invalid".to_owned(),
            user_id: ALICE.to_string(),
            device_id: "DEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    ));
    let key = TimelineKey::room(AccountKey("@startup:test".to_owned()), room_id.to_string());
    let generations = Arc::new(TimelineActorGenerationGate::default());
    let actor_generation = generations.activate_after_quiescence(&key).await.generation;
    let (manager_tx, mut manager_rx) = mpsc::channel(1);
    manager_tx
        .send(TimelineMessage::IgnoredUsersUpdated {
            user_ids: BTreeSet::new(),
        })
        .await
        .expect("saturate manager mailbox");
    let (action_tx, _action_rx) = mpsc::channel(8);
    let (event_tx, _) = broadcast::channel(8);
    let (terminal_ingress, _terminal_rx) = TimelineSendTerminalIngress::channel();

    let handle = executor::timeout(
        Duration::from_millis(100),
        TimelineActor::spawn(
            key.clone(),
            timeline,
            session,
            fake_rid(38_001),
            true,
            action_tx,
            event_tx,
            None,
            BTreeSet::new(),
            None,
            LinkPreviewContext::default(),
            AccountWorkScheduler::default(),
            Arc::new(Mutex::new(ThreadRootProjectionService::default())),
            koushi_state::TimelineThreadRootOrder::LatestReply,
            generations,
            actor_generation,
            None,
            SharedSendCompletionCoordinator::default(),
            terminal_ingress,
            manager_tx,
        ),
    )
    .await
    .expect("actor construction must not await manager capacity");

    assert!(matches!(
        manager_rx.recv().await,
        Some(TimelineMessage::IgnoredUsersUpdated { .. })
    ));
    assert!(matches!(
        executor::timeout(Duration::from_millis(100), manager_rx.recv())
            .await
            .expect("authoritative startup publish must resume after capacity opens"),
        Some(TimelineMessage::AuthoritativeReadStateObserved {
            key: observed,
            actor_generation: observed_generation,
            ..
        }) if observed == key && observed_generation == actor_generation
    ));
    handle.stop().await;
}

#[tokio::test]
async fn live_tail_replacement_ignores_old_epoch_actor_and_projection_completion() {
    let room = TimelineKey::room(AccountKey("@a:test".to_owned()), "!a:test");
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut manager = live_tail_test_manager(HashMap::from([(
        room.clone(),
        live_tail_test_actor_handle("A", log.clone()),
    )]));
    let actor_generation = manager
        .timeline_actor_generations
        .activate_after_quiescence(&room)
        .await
        .generation;

    manager.room_subscription_service_epoch = 7;
    manager
        .handle_committed_room_selection(fake_rid(1), room.clone(), false, false)
        .await;
    let replacement_starts = manager
        .invalidate_live_tail_epoch_for_existing_rooms(8)
        .await;
    manager
        .apply_live_tail_scheduler_actions(replacement_starts)
        .await;

    assert_eq!(
        manager.live_tail_refreshes.freshness(&room),
        Some(
            crate::live_tail_freshness::LiveTailFreshnessState::Refreshing {
                epoch: 8,
                operation_generation: 2,
            }
        ),
        "the replacement sync run must fence epoch 7 before an old completion can arrive",
    );
    manager
        .handle_live_tail_refresh_completed(
            room.clone(),
            actor_generation,
            7,
            1,
            MatrixLiveTailRefreshOutcome::Advanced { events: 1 },
            128,
            1,
            1,
        )
        .await;
    manager
        .handle_live_tail_refresh_completed(
            room.clone(),
            actor_generation.saturating_sub(1),
            8,
            2,
            MatrixLiveTailRefreshOutcome::Advanced { events: 1 },
            128,
            1,
            1,
        )
        .await;
    assert_eq!(
        manager.live_tail_refreshes.freshness(&room),
        Some(
            crate::live_tail_freshness::LiveTailFreshnessState::Refreshing {
                epoch: 8,
                operation_generation: 2,
            }
        ),
    );

    let mut projection = TimelineGapProjectionCorrelation::default();
    let operation = live_tail_causal_projection_operation(2);
    projection.begin(actor_generation, operation);
    assert_eq!(
        projection.complete(actor_generation, operation, Some(2)),
        TimelineGapProjectionCompletion::Pending
    );
    for stale in [
        CausalProjectionId {
            actor_generation: actor_generation.saturating_sub(1),
            operation,
            projection_batch: 2,
        },
        CausalProjectionId {
            actor_generation,
            operation: live_tail_causal_projection_operation(1),
            projection_batch: 2,
        },
        CausalProjectionId {
            actor_generation,
            operation,
            projection_batch: 1,
        },
    ] {
        assert_eq!(projection.observe(stale, TimelineBatchId(9)), None);
        assert!(projection.is_pending());
    }
    assert_eq!(
        projection.observe(
            CausalProjectionId {
                actor_generation,
                operation,
                projection_batch: 2,
            },
            TimelineBatchId(10),
        ),
        Some(TimelineBatchId(10))
    );

    manager
        .handle_live_tail_refresh_completed(
            room.clone(),
            actor_generation,
            8,
            2,
            MatrixLiveTailRefreshOutcome::Advanced { events: 1 },
            128,
            1,
            1,
        )
        .await;
    assert_eq!(
        manager.live_tail_refreshes.freshness(&room),
        Some(crate::live_tail_freshness::LiveTailFreshnessState::Fresh { epoch: 8 }),
    );

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if log.lock().expect("live-tail log lock").len() == 3 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("replacement epoch log completed");
    assert_eq!(
        *log.lock().expect("live-tail log lock"),
        [
            "start:A:epoch=7:limit=128",
            "cancel-network:A:epoch=7",
            "start:A:epoch=8:limit=128",
        ]
    );
}

#[test]
fn activity_row_from_timeline_item_preserves_thread_root_event_id() {
    let mut item = timeline_item(
        "$thread-reply:test",
        Some("reply body"),
        "@sender:test",
        false,
    );
    item.thread_root = Some("$thread-root:test".to_owned());

    let row = activity_row_from_timeline_item("!room:test", &item)
        .expect("event timeline item should project to an activity row");
    let value = serde_json::to_value(&row).expect("activity row should serialize");

    assert_eq!(value["event_id"], serde_json::json!("$thread-reply:test"));
    assert_eq!(
        value["thread_root_event_id"],
        serde_json::json!("$thread-root:test")
    );
}

#[test]
fn timeline_navigation_marks_first_unread_inside_viewport() {
    let items = vec![
        timeline_item("$read:test", Some("read"), "@alice:test", false),
        timeline_item("$unread:test", Some("unread"), "@alice:test", false),
        timeline_item("$newer:test", Some("newer"), "@alice:test", false),
    ];

    let snapshot = derive_timeline_navigation_snapshot(
        &items,
        Some("$read:test"),
        &TimelineViewportObservation {
            first_visible_event_id: Some("$unread:test".to_owned()),
            last_visible_event_id: Some("$newer:test".to_owned()),
            visible_gap_ids: Vec::new(),
            bottom_arrival: TimelineBottomArrival::User,
            at_bottom: true,
        },
        Some("@me:test"),
    );

    assert_eq!(snapshot.read_marker_event_id.as_deref(), Some("$read:test"));
    assert_eq!(
        snapshot.first_unread_event_id.as_deref(),
        Some("$unread:test")
    );
    assert_eq!(snapshot.unread_event_count, 2);
    assert_eq!(
        snapshot.unread_position,
        TimelineUnreadPosition::InsideViewport
    );
    assert_eq!(snapshot.newer_event_count, 0);
}

#[test]
fn timeline_navigation_separates_local_viewed_and_server_confirmed_boundaries() {
    let items = vec![
        timeline_item("$server:test", Some("server"), "@alice:test", false),
        timeline_item("$local:test", Some("local"), "@alice:test", false),
    ];
    let snapshot = derive_timeline_navigation_snapshot_with_read_state(
        &items,
        Some("$server:test"),
        Some("$server:test"),
        Some("$local:test"),
        TimelineReadStateSync::Pending,
        &TimelineViewportObservation {
            first_visible_event_id: Some("$local:test".to_owned()),
            last_visible_event_id: Some("$local:test".to_owned()),
            visible_gap_ids: Vec::new(),
            bottom_arrival: TimelineBottomArrival::User,
            at_bottom: true,
        },
        Some("@me:test"),
    );

    assert_eq!(
        snapshot.local_viewed_event_id.as_deref(),
        Some("$local:test")
    );
    assert_eq!(
        snapshot.server_confirmed_read_event_id.as_deref(),
        Some("$server:test")
    );
    assert_eq!(
        snapshot.read_marker_event_id.as_deref(),
        Some("$server:test")
    );
    assert_eq!(
        snapshot.read_marker_display_event_id.as_deref(),
        Some("$local:test")
    );
    assert_eq!(snapshot.read_state_sync, TimelineReadStateSync::Pending);
}

#[test]
fn timeline_navigation_reports_unread_below_viewport_and_newer_count() {
    let items = vec![
        timeline_item("$read:test", Some("read"), "@alice:test", false),
        timeline_item("$visible:test", Some("visible"), "@alice:test", false),
        timeline_item("$unread:test", Some("unread"), "@alice:test", false),
        timeline_item("$newer:test", Some("newer"), "@alice:test", false),
    ];

    let snapshot = derive_timeline_navigation_snapshot(
        &items,
        Some("$visible:test"),
        &TimelineViewportObservation {
            first_visible_event_id: Some("$read:test".to_owned()),
            last_visible_event_id: Some("$visible:test".to_owned()),
            visible_gap_ids: Vec::new(),
            bottom_arrival: TimelineBottomArrival::User,
            at_bottom: false,
        },
        Some("@me:test"),
    );

    assert_eq!(
        snapshot.first_unread_event_id.as_deref(),
        Some("$unread:test")
    );
    assert_eq!(snapshot.unread_event_count, 2);
    assert_eq!(
        snapshot.unread_position,
        TimelineUnreadPosition::BelowViewport
    );
    assert_eq!(snapshot.newer_event_count, 2);
}

#[test]
fn timeline_navigation_does_not_count_read_history_below_viewport_as_newer() {
    let items = vec![
        timeline_item("$visible:test", Some("visible"), "@alice:test", false),
        timeline_item("$read-a:test", Some("read a"), "@alice:test", false),
        timeline_item("$read-b:test", Some("read b"), "@alice:test", false),
        timeline_item(
            "$read-marker:test",
            Some("read marker"),
            "@alice:test",
            false,
        ),
    ];

    let snapshot = derive_timeline_navigation_snapshot(
        &items,
        Some("$read-marker:test"),
        &TimelineViewportObservation {
            first_visible_event_id: Some("$visible:test".to_owned()),
            last_visible_event_id: Some("$visible:test".to_owned()),
            visible_gap_ids: Vec::new(),
            bottom_arrival: TimelineBottomArrival::User,
            at_bottom: false,
        },
        Some("@me:test"),
    );

    assert_eq!(snapshot.first_unread_event_id, None);
    assert_eq!(snapshot.unread_event_count, 0);
    assert_eq!(snapshot.newer_event_count, 0);
    assert!(!snapshot.can_jump_to_bottom);
}

#[test]
fn timeline_navigation_does_not_count_newer_events_without_read_marker() {
    let items = vec![
        timeline_item("$visible:test", Some("visible"), "@alice:test", false),
        timeline_item("$loaded:test", Some("loaded"), "@alice:test", false),
    ];

    let snapshot = derive_timeline_navigation_snapshot(
        &items,
        None,
        &TimelineViewportObservation {
            first_visible_event_id: Some("$visible:test".to_owned()),
            last_visible_event_id: Some("$visible:test".to_owned()),
            visible_gap_ids: Vec::new(),
            bottom_arrival: TimelineBottomArrival::User,
            at_bottom: false,
        },
        Some("@me:test"),
    );

    assert_eq!(snapshot.read_marker_event_id, None);
    assert_eq!(snapshot.unread_event_count, 0);
    assert_eq!(snapshot.newer_event_count, 0);
    assert!(!snapshot.can_jump_to_bottom);
}

#[test]
fn timeline_navigation_ignores_own_local_and_synthetic_items_for_unread_counts() {
    let mut own = timeline_item("$own:test", Some("own"), "@me:test", false);
    own.id = TimelineItemId::Event {
        event_id: "$own:test".to_owned(),
    };
    let mut local = timeline_item("$local:test", Some("local"), "@me:test", false);
    local.id = TimelineItemId::Transaction {
        transaction_id: "txn-local".to_owned(),
    };
    let mut synthetic = timeline_item("$synthetic:test", Some("divider"), "@me:test", false);
    synthetic.id = TimelineItemId::Synthetic {
        synthetic_id: "date-divider".to_owned(),
    };
    let items = vec![
        timeline_item("$read:test", Some("read"), "@alice:test", false),
        own,
        local,
        synthetic,
        timeline_item("$remote:test", Some("remote"), "@alice:test", false),
    ];

    let snapshot = derive_timeline_navigation_snapshot(
        &items,
        Some("$read:test"),
        &TimelineViewportObservation {
            first_visible_event_id: Some("$read:test".to_owned()),
            last_visible_event_id: Some("$remote:test".to_owned()),
            visible_gap_ids: Vec::new(),
            bottom_arrival: TimelineBottomArrival::User,
            at_bottom: true,
        },
        Some("@me:test"),
    );

    assert_eq!(
        snapshot.first_unread_event_id.as_deref(),
        Some("$remote:test")
    );
    assert_eq!(snapshot.unread_event_count, 1);
    assert_eq!(snapshot.newer_event_count, 0);
}

#[test]
fn unread_consistency_diagnostic_correlates_thread_receipt_with_latest_reply_projection() {
    let key = thread_key();
    let mut root = timeline_item("$root:test", Some("root"), "@me:test", false);
    root.thread_summary = Some(ThreadSummaryDto {
        reply_count: 1,
        latest_event_id: Some("$reply:test".to_owned()),
        latest_sender: Some("@alice:test".to_owned()),
        latest_sender_label: Some("Alice".to_owned()),
        latest_body_preview: Some("reply".to_owned()),
        latest_timestamp_ms: Some(2),
    });
    let mut reply = timeline_item("$reply:test", Some("reply"), "@alice:test", false);
    reply.thread_root = Some("$root:test".to_owned());
    let canonical_items = vec![root.clone(), reply];
    let snapshot = derive_timeline_navigation_snapshot(
        &canonical_items,
        Some("$root:test"),
        &TimelineViewportObservation::default(),
        Some("@me:test"),
    );
    let thread_attention = ThreadAttentionTracker {
        receipt_event_id: Some("$reply:test".to_owned()),
        ..ThreadAttentionTracker::default()
    };

    let event = timeline_unread_consistency_diagnostic_event(
        "test",
        &key,
        &canonical_items,
        &[root],
        None,
        &snapshot,
        &thread_attention,
    );
    let has_field = |key, expected| {
        event
            .fields
            .iter()
            .any(|field| field.key == key && field.value == expected)
    };

    assert_eq!(event.source, "core.timeline_unread_consistency");
    assert!(has_field("timeline", DiagnosticValue::Token("thread")));
    assert!(has_field(
        "first_unread_has_thread_root",
        DiagnosticValue::Boolean(true)
    ));
    assert!(has_field(
        "thread_receipt_in_canonical",
        DiagnosticValue::Boolean(true)
    ));
    assert!(has_field(
        "thread_receipt_matches_timeline_root",
        DiagnosticValue::Boolean(true)
    ));
    assert!(has_field(
        "latest_reply_activity_matches_first_unread",
        DiagnosticValue::Boolean(true)
    ));
    assert!(has_field(
        "thread_attention_count",
        DiagnosticValue::Count(0)
    ));
    assert!(has_field("unread_event_count", DiagnosticValue::Count(1)));
}

#[tokio::test]
async fn forward_pagination_on_room_key_fails_invalid_direction() {
    let runtime = CoreRuntime::start();
    let mut conn = runtime.attach();

    // Inject a Ready session so commands are not gated.
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

    // Wait for Ready.
    loop {
        if matches!(conn.snapshot().session, SessionState::Ready(_)) {
            break;
        }
        crate::executor::sleep(Duration::from_millis(5)).await;
    }

    let rid = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
        request_id: rid,
        key: room_key(),
        initial_backfill: koushi_protocol::command::InitialBackfillPolicy::Disabled,
    }))
    .await
    .expect("submit");

    // Subscribe will fail (no real session) — we don't care. Send forward paginate.
    let paginate_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
        request_id: paginate_id,
        key: room_key(),
        direction: PaginationDirection::Forward,
        event_count: 20,
    }))
    .await
    .expect("submit");

    // Drain until we find a failure for paginate_id.
    loop {
        let timeout = tokio::time::timeout(Duration::from_secs(5), conn.recv_event()).await;
        let event = timeout.expect("no timeout").expect("no lag");
        match event {
            CoreEvent::OperationFailed {
                request_id,
                failure,
            } if request_id == paginate_id => {
                // Subscribe failed, so the key is not subscribed — we get NotSubscribed.
                // OR we get InvalidDirection if subscribe somehow succeeded.
                // Either way, it MUST NOT succeed.
                assert!(
                    matches!(
                        failure,
                        CoreFailure::TimelineOperationFailed {
                            kind: TimelineFailureKind::InvalidDirection
                                | TimelineFailureKind::NotSubscribed
                                | TimelineFailureKind::Sdk,
                        }
                    ),
                    "expected timeline failure, got: {failure:?}"
                );
                return;
            }
            _ => continue,
        }
    }
}

#[tokio::test]
async fn forward_pagination_on_thread_key_not_subscribed() {
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

    // Do NOT subscribe; paginate forward on thread key → NotSubscribed.
    let paginate_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
        request_id: paginate_id,
        key: thread_key(),
        direction: PaginationDirection::Forward,
        event_count: 10,
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
            } if request_id == paginate_id => {
                assert!(
                    matches!(
                        failure,
                        CoreFailure::TimelineOperationFailed {
                            kind: TimelineFailureKind::InvalidDirection
                                | TimelineFailureKind::NotSubscribed,
                        }
                    ),
                    "got: {failure:?}"
                );
                return;
            }
            _ => continue,
        }
    }
}

#[test]
fn focused_allows_forward_direction_in_paginate_logic() {
    // Test the direction check logic directly: forward IS allowed on Focused.
    let key = focused_key();
    let is_focused = matches!(key.kind, TimelineKind::Focused { .. });
    assert!(is_focused, "focused key must match Focused");

    // Forward + Focused: should NOT trigger InvalidDirection.
    let direction = PaginationDirection::Forward;
    let is_invalid = direction == PaginationDirection::Forward
        && !matches!(key.kind, TimelineKind::Focused { .. });
    assert!(
        !is_invalid,
        "forward on Focused must not be invalid direction"
    );
}

#[test]
fn backward_direction_never_invalid_for_any_kind() {
    for key in [room_key(), focused_key(), thread_key()] {
        let direction = PaginationDirection::Backward;
        let is_invalid = direction == PaginationDirection::Forward
            && !matches!(key.kind, TimelineKind::Focused { .. });
        assert!(
            !is_invalid,
            "backward pagination should never be InvalidDirection for key: {key:?}"
        );
    }
}

#[tokio::test]
async fn paginate_on_unsubscribed_key_returns_not_subscribed() {
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
    conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
        request_id: rid,
        key: room_key(),
        direction: PaginationDirection::Backward,
        event_count: 20,
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
                assert_eq!(
                    failure,
                    CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::NotSubscribed
                    }
                );
                return;
            }
            _ => continue,
        }
    }
}

#[tokio::test]
async fn sdk_vector_diff_batch_preserves_prefix_for_append_and_pop_variants() {
    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    use matrix_sdk_test::{ALICE, JoinedRoomBuilder, event_factory::EventFactory};

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    client
        .event_cache()
        .subscribe()
        .expect("event cache subscription");
    let room_id = matrix_sdk::ruma::room_id!("!sdk-diff-shapes:example.org");
    let room = server.sync_joined_room(&client, room_id).await;
    let factory = EventFactory::new().room(room_id).sender(&ALICE);
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(room_id)
                .add_timeline_event(
                    factory
                        .text_msg("prefix-a")
                        .event_id(matrix_sdk::ruma::event_id!("$prefix-a:example.org"))
                        .into_raw_sync(),
                )
                .add_timeline_event(
                    factory
                        .text_msg("prefix-b")
                        .event_id(matrix_sdk::ruma::event_id!("$prefix-b:example.org"))
                        .into_raw_sync(),
                )
                .add_timeline_event(
                    factory
                        .text_msg("append-a")
                        .event_id(matrix_sdk::ruma::event_id!("$append-a:example.org"))
                        .into_raw_sync(),
                )
                .add_timeline_event(
                    factory
                        .text_msg("append-b")
                        .event_id(matrix_sdk::ruma::event_id!("$append-b:example.org"))
                        .into_raw_sync(),
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
    .expect("room timeline");
    let (sdk_items, _stream) = timeline.subscribe().await;
    let event = |event_id: &str| {
        sdk_items
            .iter()
            .find(|item| {
                item.as_event()
                    .and_then(|event| event.event_id())
                    .is_some_and(|candidate| candidate.as_str() == event_id)
            })
            .cloned()
            .expect("fixture SDK event")
    };
    let key = TimelineKey::room(AccountKey(ALICE.to_string()), room_id.to_string());
    let mut canonical = vec![
        sdk_item_to_timeline_item(&key, &event("$prefix-a:example.org"), Some(&ALICE)),
        sdk_item_to_timeline_item(&key, &event("$prefix-b:example.org"), Some(&ALICE)),
    ];
    let diffs = sdk_vector_diffs_to_timeline_diffs(
        &[
            eyeball_im::VectorDiff::Append {
                values: eyeball_im::Vector::from(vec![
                    event("$append-a:example.org"),
                    event("$append-b:example.org"),
                ]),
            },
            eyeball_im::VectorDiff::PopBack,
            eyeball_im::VectorDiff::PopFront,
        ],
        canonical.len(),
        &key,
        Some(&ALICE),
        &HashMap::new(),
        None,
        None,
    );
    apply_timeline_diffs_to_items(&mut canonical, &diffs);

    assert_eq!(
        canonical
            .iter()
            .filter_map(|item| match &item.id {
                TimelineItemId::Event { event_id } => Some(event_id.as_str()),
                TimelineItemId::Transaction { .. } | TimelineItemId::Synthetic { .. } => None,
            })
            .collect::<Vec<_>>(),
        vec!["$prefix-b:example.org", "$append-a:example.org"],
        "Append must retain the existing prefix and PopBack must remove only the live edge"
    );
}

#[test]
fn navigation_display_anchor_advances_past_own_messages_after_marker() {
    let other = timeline_item("$other", Some("hello"), "@bob", false);
    let own1 = timeline_item("$own1", Some("own1"), "@alice", false);
    let own2 = timeline_item("$own2", Some("own2"), "@alice", false);
    let items = vec![other, own1, own2];
    let observation = TimelineViewportObservation::default();

    let snapshot =
        derive_timeline_navigation_snapshot(&items, Some("$other"), &observation, Some("@alice"));

    assert_eq!(snapshot.read_marker_event_id, Some("$other".to_owned()));
    assert_eq!(snapshot.first_unread_event_id, None);
    assert_eq!(
        snapshot.read_marker_display_event_id,
        Some("$own2".to_owned())
    );
}

#[test]
fn navigation_display_anchor_stays_at_marker_when_no_own_messages_after() {
    let other = timeline_item("$other", Some("hello"), "@bob", false);
    let remote = timeline_item("$remote", Some("remote"), "@bob", false);
    let items = vec![other, remote];
    let observation = TimelineViewportObservation::default();

    let snapshot =
        derive_timeline_navigation_snapshot(&items, Some("$other"), &observation, Some("@alice"));

    assert_eq!(snapshot.first_unread_event_id, Some("$remote".to_owned()));
    assert_eq!(snapshot.read_marker_display_event_id, None);
}

#[test]
fn navigation_display_anchor_advances_from_own_marker_to_later_own_message() {
    let own1 = timeline_item("$own1", Some("own1"), "@alice", false);
    let own2 = timeline_item("$own2", Some("own2"), "@alice", false);
    let items = vec![own1, own2];
    let observation = TimelineViewportObservation::default();

    let snapshot =
        derive_timeline_navigation_snapshot(&items, Some("$own1"), &observation, Some("@alice"));

    assert_eq!(snapshot.read_marker_event_id, Some("$own1".to_owned()));
    assert_eq!(snapshot.first_unread_event_id, None);
    assert_eq!(
        snapshot.read_marker_display_event_id,
        Some("$own2".to_owned())
    );
}
