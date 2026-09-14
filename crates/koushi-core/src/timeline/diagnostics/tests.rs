use super::super::test_source::item_body;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use std::sync::{Arc, Mutex};

use koushi_state::{AppAction, ComposerFormattingOptions};

use tokio::sync::{broadcast, mpsc};

use crate::account_work::AccountWorkScheduler;

use crate::executor;
use crate::link_preview::LinkPreviewContext;
use koushi_protocol::command::TimelineCommand;
use koushi_protocol::event::{PaginationDirection, TimelineDiff};
use koushi_protocol::failure::{CoreFailure, TimelineFailureKind};
#[cfg(any(test, feature = "test-hooks"))]
use koushi_protocol::ids::AccountKey;
use koushi_protocol::ids::{TimelineBatchId, TimelineGeneration, TimelineKey, TimelineKind};

use crate::live_tail_freshness::LiveTailRefreshCoordinator;
use crate::read_state::ReadStateKey;

use crate::threads_list::ThreadRootProjectionService;

use koushi_diagnostics::DiagnosticValue;

use super::super::actor::TimelineActorHandle;
use super::super::manager::TimelineManagerActor;
use super::super::navigation::TimelineActorGenerationGate;
use super::super::outbound_send::{
    SendCompletionObservation, SendCompletionRegistration, SendEnqueueWorkerSupervisor,
    SharedSendCompletionCoordinator, SubmissionAdmissionLedger, TimelineSendCompletionDelivery,
    TimelineSendFailureDelivery, TimelineSendTerminalIngress,
    apply_send_completion_observation_and_handoff,
};
use super::super::read_state::{ReadRetrySource, ReadWorkerSupervisor};
use super::super::test_support::{fake_rid, room_key, timeline_item};
use super::super::thread_projection::ThreadRootProjectionFetchRegistry;
use super::{
    event_cache_diff_batch_diagnostic_event, event_cache_item_diagnostic_event, record_read_retry,
    record_thread_projection, record_thread_summary_reconciliation,
    timeline_diff_batch_diagnostic_event, trace_event_cache_diff_without_item,
    trace_event_cache_diffs, trace_timeline_actor_operation, trace_timeline_actor_scan,
    trace_timeline_diffs, trace_timeline_items, trace_timeline_link_preview,
    trace_timeline_paginate, trace_timeline_route,
};

#[test]
fn event_cache_structured_fields_include_relation_presence_without_ids() {
    let key = room_key();
    let item = matrix_sdk_base::event_cache::Event::from_plaintext(
        matrix_sdk::ruma::serde::Raw::new(&serde_json::json!({
                "type": "m.room.message",
                "event_id": "$private-cache-event:test",
                "room_id": "!private-room:test",
                "sender": "@private-sender:test",
                "origin_server_ts": 1_783_076_820_000_u64,
                "content": {
                    "msgtype": "m.text",
                    "body": "private body",
                    "m.relates_to": {
                        "rel_type": "m.thread",
                        "event_id": "$private-thread-root:test",
                        "m.in_reply_to": { "event_id": "$private-reply:test" }
                }
            }
        }))
        .expect("synthetic cache event")
        .cast_unchecked(),
    );

    let event = event_cache_item_diagnostic_event("cache_initial", &key, "item", Some(4), &item);

    assert_eq!(
        event
            .fields
            .iter()
            .map(|field| (field.key, field.value.clone()))
            .collect::<Vec<_>>(),
        vec![
            ("kind", DiagnosticValue::Token("item")),
            ("timeline", DiagnosticValue::Token("room")),
            ("count", DiagnosticValue::Count(1)),
            ("index", DiagnosticValue::Count(4)),
            ("index_present", DiagnosticValue::Boolean(true)),
            ("event_id_present", DiagnosticValue::Boolean(true)),
            ("sender_present", DiagnosticValue::Boolean(true)),
            (
                "timestamp_minute",
                DiagnosticValue::Count(1_783_076_820_000 / 60_000),
            ),
            ("timestamp_present", DiagnosticValue::Boolean(true)),
            ("push_actions_present", DiagnosticValue::Boolean(false)),
            ("push_notify", DiagnosticValue::Boolean(false)),
            ("push_highlight", DiagnosticValue::Boolean(false)),
            ("relation", DiagnosticValue::Token("m.thread")),
            ("relates_to_present", DiagnosticValue::Boolean(true)),
            ("relation_event_present", DiagnosticValue::Boolean(true)),
            ("reply_present", DiagnosticValue::Boolean(true)),
            ("thread_root_present", DiagnosticValue::Boolean(true)),
        ]
    );
    let mut notifying_item = item.clone();
    notifying_item.set_push_actions(vec![matrix_sdk::ruma::push::Action::Notify]);
    let notifying = event_cache_item_diagnostic_event("cache_initial", &key, "item", Some(4), &notifying_item);
    assert!(notifying.fields.iter().any(|field| field.key == "push_notify" && field.value == DiagnosticValue::Boolean(true)));
    let serialized = serde_json::to_string(&event).expect("diagnostic event serializes");
    for private_value in [
        "$private-cache-event:test",
        "!private-room:test",
        "@private-sender:test",
        "$private-thread-root:test",
        "$private-reply:test",
        "private body",
    ] {
        assert!(!serialized.contains(private_value));
    }
}

#[test]
fn timeline_diagnostic_helpers_collect_typed_records_without_trace_env() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let key = room_key();
    let request_id = fake_rid(7001);
    record_read_retry(
        &ReadStateKey::PublicUnthreaded {
            room_id: "!private-read-room:test".to_owned(),
        },
        ReadRetrySource::Reconnect,
        2,
        1,
    );

    trace_timeline_actor_operation(
        "actor_finish",
        "send_reaction",
        request_id,
        &key,
        Some(12),
        Some("success"),
    );
    trace_timeline_actor_scan("target_scan", "send_reaction", request_id, &key, 3, 4, true);
    trace_timeline_route("manager_received", "send_reaction", request_id, &key);
    trace_timeline_paginate(
        "sdk_finish",
        request_id,
        &key,
        PaginationDirection::Backward,
        8,
        Some(15),
        Some(2),
        Some("success"),
    );
    trace_timeline_link_preview(
        "complete",
        request_id,
        &key,
        1,
        2,
        3,
        Some(9),
        Some("success"),
    );
    trace_timeline_items(
        "initial",
        &key,
        &[timeline_item(
            "$private-event:test",
            Some("private body"),
            "@private-sender:test",
            true,
        )],
    );
    trace_event_cache_diff_without_item("cache_diff", &key, "append", None, Some(2));

    trace_timeline_diffs(
        "diff_batch",
        &key,
        &[TimelineDiff::Remove { index: 2 }, TimelineDiff::Clear],
    );
    let cache_item = matrix_sdk_base::event_cache::Event::from_plaintext(
        matrix_sdk::ruma::serde::Raw::new(&serde_json::json!({
                "type": "m.room.message",
                "event_id": "$private-cache-event:test",
                "room_id": "!private-room:test",
                "sender": "@private-sender:test",
                "origin_server_ts": 1,
                "content": {"msgtype": "m.text", "body": "private body"}
        }))
        .expect("synthetic cache event")
        .cast_unchecked(),
    );
    trace_event_cache_diffs(
        "cache_update",
        &key,
        &matrix_sdk::event_cache::EventsOrigin::Cache,
        &[
            eyeball_im::VectorDiff::PushBack { value: cache_item },
            eyeball_im::VectorDiff::Remove { index: 2 },
            eyeball_im::VectorDiff::Clear,
        ],
    );

    let records = koushi_diagnostics::test_support::detail_snapshot().records;
    let expected = [
        ("core.timeline", "actor_finish"),
        ("core.timeline", "target_scan"),
        ("core.timeline", "manager_received"),
        ("core.timeline", "sdk_finish"),
        ("core.timeline", "complete"),
        ("core.timeline_item", "initial"),
        ("core.event_cache", "cache_diff"),
        ("core.read_state", "retry_wake"),
    ];
    for (source, stage) in expected {
        let event = records
            .iter()
            .find(|record| record.event.source == source && record.event.stage == stage)
            .map(|record| &record.event)
            .unwrap_or_else(|| panic!("missing {source}/{stage}"));
        assert!(event.fields.iter().any(|field| field.key == "kind"));
        assert!(event.fields.iter().any(|field| {
            matches!(field.key, "duration" | "count" | "request_id")
                || matches!(field.value, DiagnosticValue::Count(_))
        }));
        let serialized = serde_json::to_string(event).expect("diagnostic event serializes");
        for private_value in [
            "!r:test",
            "!private-read-room:test",
            "$private-event:test",
            "@private-sender:test",
            "private body",
        ] {
            assert!(
                !serialized.contains(private_value),
                "leaked {private_value}"
            );
        }
    }

    let records = koushi_diagnostics::test_support::detail_snapshot().records;
    for (source, stage, field_key) in [
        ("core.timeline_item", "diff_batch", "remove_count"),
        ("core.timeline_item", "diff_batch", "clear_count"),
        ("core.event_cache", "cache_update", "remove_count"),
        ("core.event_cache", "cache_update", "clear_count"),
        ("core.event_cache", "cache_update", "push_back_count"),
    ] {
        assert!(
            records.iter().any(|record| {
                record.event.source == source
                    && record.event.stage == stage
                    && record.event.fields.iter().any(|field| {
                        field.key == field_key
                            && field.value == koushi_diagnostics::DiagnosticValue::Count(1)
                    })
            }),
            "missing {source}/{stage}/{field_key}"
        );
    }

    for kind in [
        "subscribe",
        "ensure_subscribed",
        "unsubscribe",
        "cancel_pagination",
        "cancel_link_previews",
        "load_link_previews",
    ] {
        trace_timeline_route("manager_received", kind, request_id, &key);
    }
    for outcome in [
        "end_reached",
        "idle",
        "failed",
        "in_flight",
        "invalid_event",
        "invalid_private_receipt",
        "invalid_thread_root",
        "redacted",
        "unchanged",
        "discarded",
        "updated",
    ] {
        trace_timeline_actor_operation(
            "actor_finish",
            "send_reaction",
            request_id,
            &key,
            Some(1),
            Some(outcome),
        );
    }
    let records = koushi_diagnostics::test_support::detail_snapshot().records;
    for record in records
        .iter()
        .filter(|record| record.event.source == "core.timeline")
    {
        for field in &record.event.fields {
            if matches!(
                field.value,
                koushi_diagnostics::DiagnosticValue::Token("other")
            ) {
                panic!("live timeline diagnostic collapsed to other: {record:?}");
            }
        }
    }
}

#[test]
fn timeline_diff_batch_emits_one_count_only_summary() {
    let event = timeline_diff_batch_diagnostic_event(
        "diff_batch",
        &room_key(),
        &[TimelineDiff::Remove { index: 2 }, TimelineDiff::Clear],
    );

    assert!(
        event.fields.iter().any(|field| {
            field.key == "remove_count" && field.value == DiagnosticValue::Count(1)
        })
    );
    assert!(
        event.fields.iter().any(|field| {
            field.key == "clear_count" && field.value == DiagnosticValue::Count(1)
        })
    );
}

#[test]
fn event_cache_diff_batch_emits_one_count_only_summary() {
    let event = event_cache_diff_batch_diagnostic_event(
        "cache_update",
        &room_key(),
        &matrix_sdk::event_cache::EventsOrigin::Cache,
        &[
            eyeball_im::VectorDiff::Remove { index: 2 },
            eyeball_im::VectorDiff::Clear,
        ],
    );

    assert!(
        event.fields.iter().any(|field| {
            field.key == "remove_count" && field.value == DiagnosticValue::Count(1)
        })
    );
    assert!(
        event.fields.iter().any(|field| {
            field.key == "clear_count" && field.value == DiagnosticValue::Count(1)
        })
    );
}

#[test]
fn timeline_items_record_batch_only_by_default() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let key = room_key();
    let baseline = koushi_diagnostics::test_support::detail_snapshot()
        .records
        .len();
    trace_timeline_items(
        "replay_initial",
        &key,
        &[
            timeline_item("$one:test", Some("first body"), "@a:test", false),
            timeline_item("$two:test", Some("second body"), "@b:test", true),
        ],
    );

    let records = koushi_diagnostics::test_support::detail_snapshot().records;
    let appended = records[baseline..]
        .iter()
        .filter(|record| {
            record.event.source == "core.timeline_item" && record.event.stage == "replay_initial"
        })
        .collect::<Vec<_>>();
    assert_eq!(appended.len(), 1);
    let event = &appended[0].event;
    assert!(
        event
            .fields
            .iter()
            .any(|field| { field.key == "kind" && field.value == DiagnosticValue::Token("batch") })
    );
    assert!(
        event
            .fields
            .iter()
            .any(|field| { field.key == "count" && field.value == DiagnosticValue::Count(2) })
    );
    assert!(
        event
            .fields
            .iter()
            .any(|field| { field.key == "hidden" && field.value == DiagnosticValue::Count(1) })
    );
}

#[test]
fn thread_projection_diagnostic_records_only_thread_batches() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let baseline = koushi_diagnostics::test_support::detail_snapshot()
        .records
        .len();
    let thread_key = TimelineKey {
        account_key: AccountKey("@a:test".to_owned()),
        kind: TimelineKind::Thread {
            room_id: "!r:test".to_owned(),
            root_event_id: "$root:test".to_owned(),
        },
    };

    record_thread_projection(
        &thread_key,
        5,
        TimelineGeneration(3),
        TimelineBatchId(7),
        2,
        1,
        11,
    );
    record_thread_projection(
        &room_key(),
        5,
        TimelineGeneration(3),
        TimelineBatchId(8),
        2,
        1,
        11,
    );

    let records = koushi_diagnostics::test_support::detail_snapshot().records;
    let appended = records[baseline..]
        .iter()
        .filter(|record| record.event.source == "core.thread_timeline")
        .collect::<Vec<_>>();
    assert_eq!(appended.len(), 1);
    let event = &appended[0].event;
    assert_eq!(event.stage, "projected");
    for key in [
        "actor_generation",
        "timeline_generation",
        "batch_id",
        "input_diffs",
        "projected_diffs",
        "items",
    ] {
        assert!(event.fields.iter().any(|field| field.key == key));
    }
}

#[test]
fn thread_summary_diagnostic_is_closed_and_private_data_free() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let baseline = koushi_diagnostics::test_support::detail_snapshot()
        .records
        .len();
    for (source, relation, decision) in [
        ("rehydration", "missing", "advance"),
        ("live_reply", "different", "advance"),
        ("edit", "same", "repair"),
        ("redaction", "different", "remove"),
        ("sdk_summary", "missing", "no_op"),
    ] {
        record_thread_summary_reconciliation(
            (7, 9),
            source,
            relation,
            decision,
            "normal",
            1,
            2,
            true,
        );
    }

    let records = koushi_diagnostics::test_support::detail_snapshot().records;
    let events = records[baseline..]
        .iter()
        .filter(|record| {
            record.event.source == "core.thread_summary" && record.event.stage == "reconciled"
        })
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 5);
    for record in events {
        let event = &record.event;
        let keys = event
            .fields
            .iter()
            .map(|field| field.key)
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            vec![
                "room_ordinal",
                "root_ordinal",
                "source",
                "identity_relation",
                "decision",
                "merge_reason",
                "count_before",
                "count_after",
                "dto_changed",
            ]
        );
        let serialized = serde_json::to_string(event).expect("diagnostic serializes");
        for private_value in [
            "!private-room:example.invalid",
            "$private-root:example.invalid",
            "$private-reply:example.invalid",
            "@private:example.invalid",
            "private label",
            "private body",
        ] {
            assert!(
                !serialized.contains(private_value),
                "leaked {private_value}"
            );
        }
        assert!(event.fields.iter().any(|field| {
            field.key == "room_ordinal"
                && matches!(
                    &field.value,
                    DiagnosticValue::OrdinalAlias { kind, ordinal }
                        if *kind == "room" && *ordinal == 7
                )
        }));
    }
}

#[tokio::test]
async fn subscribe_replay_path_records_subscribed_done_stage() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let key = room_key();
    let (actor_tx, mut actor_rx) = mpsc::channel(1);
    let actor_task = executor::spawn(async move {
        let _ = actor_rx.recv().await;
    });
    let (action_tx, _action_rx) = mpsc::channel(1);
    let (event_tx, _event_rx) = broadcast::channel(1);
    let (manager_tx, manager_rx) = mpsc::channel(1);
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
        )]),
        accepted_submissions: SubmissionAdmissionLedger::default(),
        send_completion: SharedSendCompletionCoordinator::default(),
        global_send_completion_observer_future: None,
        send_enqueue_workers: SendEnqueueWorkerSupervisor::new(terminal_ingress.clone()),
        read_workers: ReadWorkerSupervisor::unavailable(),
        action_tx,
        event_tx,
        msg_tx: manager_tx.clone(),
        msg_rx: manager_rx,
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

    manager
        .handle_subscribe(
            fake_rid(7100),
            key,
            false,
            true,
            koushi_protocol::command::InitialBackfillPolicy::Disabled,
        )
        .await;
    drop(manager_tx);

    let event = koushi_diagnostics::test_support::detail_snapshot()
        .records
        .into_iter()
        .rev()
        .find(|record| {
            record.event.source == "core.timeline" && record.event.stage == "subscribed_done"
        })
        .expect("replay subscribe path should record subscribed_done");
    assert!(event.event.fields.iter().any(|field| {
        field.key == "kind"
            && field.value == koushi_diagnostics::DiagnosticValue::Token("subscribe")
    }));
}

#[test]
fn diagnostics_producer_paths_run_in_env_unset_child_process() {
    let child = std::process::Command::new(
        std::env::current_exe().expect("current test executable should be available"),
    )
    .arg("--exact")
    .arg(concat!(
        "timeline::tests::",
        "diagnostics_producer_paths_run_without_trace_environment"
    ))
    .arg("--ignored")
    .arg("--nocapture")
    .env_remove("KOUSHI_SUBSCRIBE_TRACE")
    .env_remove("KOUSHI_TIMELINE_ITEM_TRACE")
    .env_remove("KOUSHI_UNREAD_TRACE")
    .env_remove("KOUSHI_STARTUP_TRACE")
    .status()
    .expect("env-unset diagnostics child should start");
    assert!(
        child.success(),
        "env-unset diagnostics child failed: {child}"
    );
}

#[tokio::test]
#[ignore]
async fn diagnostics_producer_paths_run_without_trace_environment() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    for variable in [
        "KOUSHI_SUBSCRIBE_TRACE",
        "KOUSHI_TIMELINE_ITEM_TRACE",
        "KOUSHI_UNREAD_TRACE",
        "KOUSHI_STARTUP_TRACE",
    ] {
        assert!(
            std::env::var_os(variable).is_none(),
            "child environment unexpectedly contains {variable}"
        );
    }

    let key = room_key();
    trace_timeline_diffs(
        "diff_batch",
        &key,
        &[TimelineDiff::Remove { index: 2 }, TimelineDiff::Clear],
    );
    let cache_item = matrix_sdk_base::event_cache::Event::from_plaintext(
        matrix_sdk::ruma::serde::Raw::new(&serde_json::json!({
                "type": "m.room.message",
                "event_id": "$private-cache-event:test",
                "room_id": "!private-room:test",
                "sender": "@private-sender:test",
                "origin_server_ts": 1,
                "content": {"msgtype": "m.text", "body": "private body"}
        }))
        .expect("synthetic cache event")
        .cast_unchecked(),
    );
    trace_event_cache_diffs(
        "cache_update",
        &key,
        &matrix_sdk::event_cache::EventsOrigin::Cache,
        &[
            eyeball_im::VectorDiff::PushBack { value: cache_item },
            eyeball_im::VectorDiff::Remove { index: 2 },
            eyeball_im::VectorDiff::Clear,
        ],
    );

    let diff_records = koushi_diagnostics::test_support::detail_snapshot().records;
    for (source, stage) in [
        ("core.timeline_item", "diff_batch"),
        ("core.event_cache", "cache_update"),
    ] {
        let batch = diff_records
            .iter()
            .find(|record| record.event.source == source && record.event.stage == stage)
            .unwrap_or_else(|| panic!("missing {source}/{stage} batch"));
        assert!(batch.event.fields.iter().any(|field| {
            field.key == "kind"
                && field.value == koushi_diagnostics::DiagnosticValue::Token("batch")
        }));
    }
    for (source, stage, field_key) in [
        ("core.timeline_item", "diff_batch", "remove_count"),
        ("core.timeline_item", "diff_batch", "clear_count"),
        ("core.event_cache", "cache_update", "push_back_count"),
        ("core.event_cache", "cache_update", "remove_count"),
        ("core.event_cache", "cache_update", "clear_count"),
    ] {
        assert!(
            diff_records.iter().any(|record| {
                record.event.source == source
                    && record.event.stage == stage
                    && record.event.fields.iter().any(|field| {
                        field.key == field_key
                            && field.value == koushi_diagnostics::DiagnosticValue::Count(1)
                    })
            }),
            "missing {source}/{stage}/{field_key}"
        );
    }
    for record in diff_records.iter().filter(|record| {
        matches!(
            (record.event.source, record.event.stage),
            ("core.timeline_item", "diff_batch") | ("core.event_cache", "cache_update")
        )
    }) {
        let serialized = serde_json::to_string(&record.event).expect("diagnostic serializes");
        for private_value in [
            "!private-room:test",
            "$private-cache-event:test",
            "@private-sender:test",
            "private body",
        ] {
            assert!(
                !serialized.contains(private_value),
                "leaked {private_value}: {serialized}"
            );
        }
    }

    let (actor_tx, mut actor_rx) = mpsc::channel(8);
    let actor_task = executor::spawn(async move { while actor_rx.recv().await.is_some() {} });
    let (action_tx, _action_rx) = mpsc::channel(8);
    let (event_tx, _event_rx) = broadcast::channel(8);
    let (_manager_tx, manager_rx) = mpsc::channel(1);
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
        )]),
        accepted_submissions: SubmissionAdmissionLedger::default(),
        send_completion: SharedSendCompletionCoordinator::default(),
        global_send_completion_observer_future: None,
        send_enqueue_workers: SendEnqueueWorkerSupervisor::new(terminal_ingress.clone()),
        read_workers: ReadWorkerSupervisor::unavailable(),
        action_tx,
        event_tx,
        msg_tx: _manager_tx,
        msg_rx: manager_rx,
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

    manager
        .handle_subscribe(
            fake_rid(7199),
            key.clone(),
            false,
            true,
            koushi_protocol::command::InitialBackfillPolicy::Disabled,
        )
        .await;

    let commands = [
        TimelineCommand::SendReaction {
            request_id: fake_rid(7200),
            key: key.clone(),
            event_id: "$event:test".to_owned(),
            reaction_key: "👍".to_owned(),
        },
        TimelineCommand::RedactReaction {
            request_id: fake_rid(7201),
            key: key.clone(),
            event_id: "$event:test".to_owned(),
            reaction_key: "👍".to_owned(),
            reaction_event_id: "$reaction:test".to_owned(),
        },
        TimelineCommand::SendReadReceipt {
            request_id: fake_rid(7202),
            key: key.clone(),
            event_id: "$event:test".to_owned(),
        },
        TimelineCommand::SetFullyRead {
            request_id: fake_rid(7203),
            key: key.clone(),
            event_id: "$event:test".to_owned(),
        },
    ];
    for command in commands {
        manager.handle_command(command).await;
    }

    let records = koushi_diagnostics::test_support::detail_snapshot().records;
    for kind in [
        "send_reaction",
        "redact_reaction",
        "send_read_receipt",
        "set_fully_read",
    ] {
        assert!(
            records.iter().any(|record| {
                record.event.source == "core.timeline"
                    && record.event.stage == "manager_received"
                    && record.event.fields.iter().any(|field| {
                        field.key == "kind"
                            && field.value == koushi_diagnostics::DiagnosticValue::Token(kind)
                    })
            }),
            "missing actual route diagnostic for {kind}"
        );
    }
    assert!(records.iter().any(|record| {
        record.event.source == "core.timeline" && record.event.stage == "subscribed_done"
    }));
    for record in records {
        assert!(
            !record.event.fields.iter().any(|field| {
                field.value == koushi_diagnostics::DiagnosticValue::Token("other")
            }),
            "live diagnostic collapsed to other: {record:?}"
        );
    }
}

#[test]
fn reaction_and_read_signal_collector_fields_are_typed_and_private() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let key = room_key();
    let request_id = fake_rid(7002);
    trace_timeline_actor_operation(
        "actor_start",
        "redact_reaction",
        request_id,
        &key,
        None,
        None,
    );
    trace_timeline_actor_operation(
        "actor_finish",
        "send_read_receipt",
        request_id,
        &key,
        Some(6),
        Some("sdk_error"),
    );
    trace_timeline_actor_operation(
        "actor_finish",
        "set_fully_read",
        request_id,
        &key,
        Some(7),
        Some("success"),
    );
    let records = koushi_diagnostics::test_support::detail_snapshot().records;
    for kind in ["redact_reaction", "send_read_receipt", "set_fully_read"] {
        let event = records
            .iter()
            .find(|record| {
                record.event.source == "core.timeline"
                    && record.event.fields.iter().any(|field| {
                        field.key == "kind"
                            && field.value == koushi_diagnostics::DiagnosticValue::Token(kind)
                    })
            })
            .expect("typed reaction/read diagnostic");
        assert!(
            event
                .event
                .fields
                .iter()
                .any(|field| field.key == "request_id")
        );
        assert!(event.event.fields.iter().all(|field| {
            !matches!(
                field.value,
                koushi_diagnostics::DiagnosticValue::Token(value)
                    if value.contains("private") || value.contains("!r")
            )
        }));
    }
}

#[test]
fn manager_coordinator_fails_new_registration_on_exact_correlation_collision() {
    let key = room_key();
    let coordinator = SharedSendCompletionCoordinator::default();
    let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
    let mut first = SendCompletionRegistration::begin(
        Arc::clone(&coordinator),
        ingress.clone(),
        key.clone(),
        "client-collision-first".to_owned(),
        None,
        fake_rid(7422),
        true,
    );
    first.activate();
    first.bind("sdk-collision".to_owned());
    let mut second = SendCompletionRegistration::begin(
        Arc::clone(&coordinator),
        ingress.clone(),
        key.clone(),
        "client-collision-second".to_owned(),
        None,
        fake_rid(7423),
        true,
    );
    second.activate();
    second.bind("sdk-collision".to_owned());

    let collision = terminal_rx
        .try_recv()
        .expect("exact correlation collision must fail safe");
    assert!(matches!(
        collision.failure,
        Some(TimelineSendFailureDelivery {
            request_id,
            failure: CoreFailure::TimelineOperationFailed {
                kind: TimelineFailureKind::QueueOverflow,
            },
        }) if request_id == fake_rid(7423)
    ));
    assert!(matches!(
        collision.action,
        Some(AppAction::SendTextFailed { transaction_id, .. })
            if transaction_id == "client-collision-second"
    ));

    apply_send_completion_observation_and_handoff(
        &coordinator,
        &ingress,
        key.room_id(),
        SendCompletionObservation::Sent {
            sdk_transaction_id: "sdk-collision".to_owned(),
            event_id: "$event-collision-first:test".to_owned(),
        },
    );
    let first_completion = terminal_rx
        .try_recv()
        .expect("the original correlation owner must remain pending");
    assert!(matches!(
        first_completion.completion,
        Some(TimelineSendCompletionDelivery { request_id, .. })
            if request_id == fake_rid(7422)
    ));
}

#[tokio::test]
async fn read_receipt_repair_uses_local_notification_count() {
    use matrix_sdk::ruma::room_id;
    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room = server
        .sync_joined_room(&client, room_id!("!local-count:example.test"))
        .await;
    room.update_room_info(|mut info| {
        info.set_read_receipts(matrix_sdk_base::read_receipts::ReadReceipts {
            num_unread: 0,
            num_notifications: 1,
            num_mentions: 1,
            ..Default::default()
        });
        (
            info,
            matrix_sdk_base::RoomInfoNotableUpdateReasons::READ_RECEIPT,
        )
    })
    .await;
    assert_eq!(
        u64::from(room.unread_notification_counts().notification_count),
        0
    );
    assert_eq!(room.num_unread_notifications(), 1);
    let context = super::room_latest_receipt_context(&room);
    assert_eq!(context.notification_count, 1);
}
