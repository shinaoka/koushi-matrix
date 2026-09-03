use std::sync::Arc;

use koushi_state::{AppAction, TimelineThreadRootOrder};

use tokio::sync::{broadcast, mpsc};

use koushi_protocol::event::{
    CoreEvent, TimelineAnchorRestoreStatus, TimelineDiff, TimelineEvent, TimelineItem,
    TimelineItemId, TimelineMediaKind, TimelineSendState, TimelineViewportObservation,
};

use koushi_protocol::ids::{TimelineBatchId, TimelineGeneration};

use super::super::item_projection::timeline_item_event_id;
use super::super::navigation::{
    ROOM_REPLAY_INITIAL_ITEMS_MAX, RestoreSettlement, TimelineActorGenerationGate,
    derive_timeline_navigation_snapshot, publish_restore_settlement_for_generation,
};
use super::super::test_support::{
    fake_rid, focused_key, replacement_generation_fixture, room_key, thread_key, timeline_item,
    timeline_media_item,
};
use super::{
    DisplayProjectionBatch, DisplayProjectionContext, DisplayProjectionState,
    apply_timeline_diffs_to_display_items, apply_timeline_diffs_to_items,
    commit_sdk_batch_for_generation, project_sdk_batch,
};

#[test]
fn pending_send_converges_to_local_and_remote_echo_without_an_empty_projection() {
    let seed = timeline_item("$seed:test", Some("seed"), "@sender:test", false);
    let mut canonical_items = vec![seed];
    let mut projection = DisplayProjectionState::from_canonical_window(&canonical_items, 0..1);
    let mut pending = timeline_item(
        "$placeholder:test",
        Some("pending body"),
        "@sender:test",
        false,
    );
    pending.id = TimelineItemId::Transaction {
        transaction_id: "client-transaction".to_owned(),
    };
    pending.send_state = Some(TimelineSendState::Sending);

    let inserted = projection.replace_pending(
        vec![pending.clone()],
        Default::default(),
        &DisplayProjectionContext::bounded_live_edge(),
    );
    assert!(!inserted.is_empty());
    assert_eq!(
        projection.display_items().last().map(|item| &item.id),
        Some(&pending.id)
    );

    pending.id = TimelineItemId::Transaction {
        transaction_id: "sdk-transaction".to_owned(),
    };
    projection.set_pending_inputs(Vec::new(), Default::default());
    let local = project_sdk_batch(
        &mut canonical_items,
        &mut projection,
        &[TimelineDiff::PushBack {
            item: pending.clone(),
        }],
        &DisplayProjectionContext::bounded_live_edge(),
    );
    assert!(
        local
            .display_diffs
            .iter()
            .all(|diff| !matches!(diff, TimelineDiff::Remove { .. } | TimelineDiff::Clear))
    );
    assert_eq!(projection.display_items().len(), 2);
    assert_eq!(projection.display_items()[1].id, pending.id);

    let event_id = "$remote:test";
    let mut sent_fallback = pending;
    sent_fallback.id = TimelineItemId::Event {
        event_id: event_id.to_owned(),
    };
    sent_fallback.send_state = Some(TimelineSendState::Sent);
    let mut canonical_items = vec![timeline_item(
        "$seed:test",
        Some("seed"),
        "@sender:test",
        false,
    )];
    let mut projection = DisplayProjectionState::from_canonical_window(&canonical_items, 0..1);
    projection.replace_pending(
        vec![sent_fallback.clone()],
        ["sdk-transaction".to_owned()].into_iter().collect(),
        &DisplayProjectionContext::bounded_live_edge(),
    );
    let mut remote = sent_fallback;
    remote.body = Some("canonical remote body".to_owned());
    remote.send_state = None;
    projection.set_pending_inputs(Vec::new(), Default::default());
    let converged = project_sdk_batch(
        &mut canonical_items,
        &mut projection,
        &[TimelineDiff::PushBack {
            item: remote.clone(),
        }],
        &DisplayProjectionContext::bounded_live_edge(),
    );
    assert!(
        converged
            .display_diffs
            .iter()
            .all(|diff| !matches!(diff, TimelineDiff::Remove { .. } | TimelineDiff::Clear))
    );
    assert_eq!(projection.display_items().len(), 2);
    assert_eq!(projection.display_items()[1].id, remote.id);
    assert_eq!(projection.display_items()[1].body, remote.body);
    assert_eq!(projection.display_items()[1].send_state, None);

    let mut delayed_local = timeline_item(
        "$delayed-placeholder:test",
        Some("pending body"),
        "@sender:test",
        false,
    );
    delayed_local.id = TimelineItemId::Transaction {
        transaction_id: "sdk-transaction".to_owned(),
    };
    delayed_local.send_state = Some(TimelineSendState::Sending);
    projection.set_pending_inputs(
        Vec::new(),
        ["sdk-transaction".to_owned()].into_iter().collect(),
    );
    project_sdk_batch(
        &mut canonical_items,
        &mut projection,
        &[TimelineDiff::PushBack {
            item: delayed_local,
        }],
        &DisplayProjectionContext::bounded_live_edge(),
    );
    assert_eq!(projection.display_items().len(), 2);
    assert!(projection.display_items().iter().all(|item| {
        !matches!(
            &item.id,
            TimelineItemId::Transaction { transaction_id }
                if transaction_id == "sdk-transaction"
        )
    }));
}

#[test]
fn sdk_canonical_indices_project_to_bounded_display_and_converge_local_echo() {
    let mut canonical_items = synthetic_projection_items(9_039);
    let mut transaction = timeline_item(
        "$transaction-placeholder:test",
        Some("synthetic local echo"),
        "@sender:test",
        false,
    );
    transaction.id = TimelineItemId::Transaction {
        transaction_id: "transaction:test".to_owned(),
    };
    canonical_items.push(transaction);

    let window_start = canonical_items.len() - ROOM_REPLAY_INITIAL_ITEMS_MAX;
    let mut projection = DisplayProjectionState::from_canonical_window(
        &canonical_items,
        window_start..canonical_items.len(),
    );
    let mut desktop_model = projection.display_items().to_vec();
    let confirmed = timeline_item(
        "$confirmed:test",
        Some("synthetic confirmed event"),
        "@sender:test",
        false,
    );

    for canonical_diffs in [
        vec![TimelineDiff::Set {
            index: 9_039,
            item: confirmed.clone(),
        }],
        vec![
            TimelineDiff::Remove { index: 9_039 },
            TimelineDiff::PushBack {
                item: confirmed.clone(),
            },
        ],
    ] {
        let projected = project_sdk_batch(
            &mut canonical_items,
            &mut projection,
            &canonical_diffs,
            &DisplayProjectionContext::bounded_live_edge(),
        );
        apply_timeline_diffs_to_items(&mut desktop_model, &projected.display_diffs);

        assert!(!projected.used_reset_fallback);
        assert_eq!(desktop_model, projection.display_items());
        assert!(
            projection
                .display_items()
                .iter()
                .all(|item| !matches!(item.id, TimelineItemId::Transaction { .. }))
        );
        assert_eq!(
            projection
                .display_items()
                .iter()
                .filter(|item| timeline_item_event_id(item) == Some("$confirmed:test"))
                .count(),
            1
        );
    }
}

fn synthetic_projection_items(count: usize) -> Vec<TimelineItem> {
    (0..count)
        .map(|index| {
            timeline_item(
                &format!("$canonical-{index}:test"),
                Some("synthetic"),
                "@sender:test",
                false,
            )
        })
        .collect()
}

fn historical_display_projection_context() -> DisplayProjectionContext {
    DisplayProjectionContext {
        max_live_edge_items: None,
        include_prepend: true,
        include_append: true,
        project_thread_roots: true,
        thread_root_order: TimelineThreadRootOrder::RootEvent,
        thread_roots: Vec::new(),
    }
}

fn deep_display_projection_fixture() -> (Vec<TimelineItem>, DisplayProjectionState) {
    let canonical_items = synthetic_projection_items(9_040);
    let start = canonical_items.len() - ROOM_REPLAY_INITIAL_ITEMS_MAX;
    let state = DisplayProjectionState::from_canonical_window(
        &canonical_items,
        start..canonical_items.len(),
    );
    (canonical_items, state)
}

fn additive_display_payload_visit_bound(batch_len: usize) -> usize {
    ROOM_REPLAY_INITIAL_ITEMS_MAX
        .saturating_add(batch_len)
        .saturating_mul(2)
}

fn expected_log_display_structural_visit_bound(
    represented_width: usize,
    batch_len: usize,
) -> usize {
    let represented_nodes = represented_width
        .saturating_add(batch_len.saturating_mul(3))
        .saturating_add(2);
    let expected_log =
        usize::BITS.saturating_sub(represented_nodes.max(1).leading_zeros()) as usize;
    represented_width
        .saturating_mul(4)
        .saturating_add(
            batch_len
                .saturating_mul(expected_log.max(1))
                .saturating_mul(48),
        )
        .saturating_add(256)
}

fn assert_display_projection_converges(
    display_before: Vec<TimelineItem>,
    projection: &DisplayProjectionBatch,
) {
    let mut desktop_model = display_before;
    apply_timeline_diffs_to_items(&mut desktop_model, &projection.display_diffs);
    assert_eq!(desktop_model, projection.display_after);
}

fn displayed(item: &TimelineItem) -> TimelineItem {
    super::decorate_event_item(item).expect("test item must be displayable")
}

#[test]
fn display_projection_retains_duplicate_identity_until_its_last_owner_is_removed() {
    let first_owner = timeline_item("$duplicate:test", Some("first"), "@sender:test", false);
    let neighbor = timeline_item("$neighbor:test", Some("neighbor"), "@sender:test", false);
    let second_owner = timeline_item("$duplicate:test", Some("second"), "@sender:test", false);
    let mut canonical_items = vec![first_owner, neighbor.clone(), second_owner.clone()];
    let mut state =
        DisplayProjectionState::from_canonical_window(&canonical_items, 0..canonical_items.len());
    let display_before = state.display_items().to_vec();

    let projection = project_sdk_batch(
        &mut canonical_items,
        &mut state,
        &[TimelineDiff::Remove { index: 0 }],
        &historical_display_projection_context(),
    );

    assert!(!projection.used_reset_fallback);
    assert_display_projection_converges(display_before, &projection);
    assert_eq!(
        state.display_items(),
        &[displayed(&neighbor), displayed(&second_owner)]
    );
    assert_eq!(
        state
            .display_items()
            .iter()
            .filter(|item| timeline_item_event_id(item) == Some("$duplicate:test"))
            .count(),
        1
    );
}

#[test]
fn display_projection_media_duplicate_keeps_indexed_confirmation_in_display_space() {
    let owner = timeline_media_item(
        "$media-owner:test",
        "@sender:test",
        None,
        1,
        "owner.png",
        TimelineMediaKind::Image,
    );
    let duplicate = timeline_media_item(
        "$media-owner:test",
        "@sender:test",
        None,
        2,
        "duplicate.png",
        TimelineMediaKind::Image,
    );
    let neighbor = timeline_item("$neighbor:test", Some("neighbor"), "@sender:test", false);
    let mut transaction = timeline_media_item(
        "$transaction-placeholder:test",
        "@sender:test",
        None,
        3,
        "upload.png",
        TimelineMediaKind::Image,
    );
    transaction.id = TimelineItemId::Transaction {
        transaction_id: "media-transaction:test".to_owned(),
    };
    let confirmed = timeline_media_item(
        "$confirmed-media:test",
        "@sender:test",
        None,
        4,
        "confirmed.png",
        TimelineMediaKind::Image,
    );
    let mut canonical_items = vec![owner.clone(), neighbor.clone(), transaction];
    let mut state =
        DisplayProjectionState::from_canonical_window(&canonical_items, 0..canonical_items.len());
    let display_before = state.display_items().to_vec();

    let projection = project_sdk_batch(
        &mut canonical_items,
        &mut state,
        &[
            TimelineDiff::Insert {
                index: 1,
                item: duplicate,
            },
            TimelineDiff::Set {
                index: 3,
                item: confirmed.clone(),
            },
        ],
        &historical_display_projection_context(),
    );

    assert_display_projection_converges(display_before, &projection);
    assert_eq!(
        state.display_items(),
        &[
            displayed(&owner),
            displayed(&neighbor),
            displayed(&confirmed)
        ]
    );
    assert_eq!(
        state
            .display_items()
            .iter()
            .filter(|item| timeline_item_event_id(item) == Some("$media-owner:test"))
            .count(),
        1
    );
    assert!(
        state
            .display_items()
            .iter()
            .all(|item| !matches!(item.id, TimelineItemId::Transaction { .. }))
    );
    assert_eq!(
        state
            .display_items()
            .last()
            .and_then(|item| item.media.as_ref())
            .map(|media| media.filename.as_str()),
        Some("confirmed.png")
    );
}

#[test]
fn display_projection_ignores_out_of_window_index_mutations() {
    let mut canonical_items = synthetic_projection_items(200);
    let mut state = DisplayProjectionState::from_canonical_window(&canonical_items, 50..100);
    let display_before = state.display_items().to_vec();
    let replacement = timeline_item(
        "$replacement:test",
        Some("replacement"),
        "@sender:test",
        false,
    );
    let inserted = timeline_item("$inserted:test", Some("inserted"), "@sender:test", false);

    let projection = project_sdk_batch(
        &mut canonical_items,
        &mut state,
        &[
            TimelineDiff::Set {
                index: 10,
                item: replacement,
            },
            TimelineDiff::Remove { index: 10 },
            TimelineDiff::Insert {
                index: 10,
                item: inserted,
            },
            TimelineDiff::Truncate { length: 150 },
        ],
        &historical_display_projection_context(),
    );

    assert!(!projection.used_reset_fallback);
    assert_display_projection_converges(display_before.clone(), &projection);
    assert_eq!(projection.display_after, display_before);
}

#[test]
fn display_projection_includes_boundary_adjacent_insert() {
    let mut canonical_items = synthetic_projection_items(200);
    let mut state = DisplayProjectionState::from_canonical_window(&canonical_items, 50..100);
    let display_before = state.display_items().to_vec();
    let boundary = timeline_item("$boundary:test", Some("boundary"), "@sender:test", false);

    let projection = project_sdk_batch(
        &mut canonical_items,
        &mut state,
        &[TimelineDiff::Insert {
            index: 50,
            item: boundary.clone(),
        }],
        &historical_display_projection_context(),
    );

    assert!(!projection.used_reset_fallback);
    assert_display_projection_converges(display_before, &projection);
    assert_eq!(state.display_items().first(), Some(&displayed(&boundary)));
}

#[test]
fn display_projection_live_edge_push_back_stays_bounded() {
    let mut canonical_items = synthetic_projection_items(200);
    let mut state = DisplayProjectionState::from_canonical_window(&canonical_items, 80..200);
    let display_before = state.display_items().to_vec();
    let live = timeline_item("$live:test", Some("live"), "@sender:test", false);

    let projection = project_sdk_batch(
        &mut canonical_items,
        &mut state,
        &[TimelineDiff::PushBack { item: live.clone() }],
        &DisplayProjectionContext::bounded_live_edge(),
    );

    assert!(!projection.used_reset_fallback);
    assert_display_projection_converges(display_before, &projection);
    assert_eq!(state.display_items().len(), ROOM_REPLAY_INITIAL_ITEMS_MAX);
    assert_eq!(state.display_items().last(), Some(&displayed(&live)));
    assert_eq!(
        state
            .display_items()
            .first()
            .and_then(timeline_item_event_id),
        Some("$canonical-81:test")
    );
}

#[test]
fn display_projection_payload_work_does_not_rescan_window_per_prepend() {
    let (mut canonical_items, mut state) = deep_display_projection_fixture();
    let diffs = (0..512)
        .map(|index| TimelineDiff::PushFront {
            item: timeline_item(
                &format!("$older-{index}:test"),
                Some("older"),
                "@sender:test",
                false,
            ),
        })
        .collect::<Vec<_>>();

    let projection = project_sdk_batch(
        &mut canonical_items,
        &mut state,
        &diffs,
        &historical_display_projection_context(),
    );

    assert!(!projection.used_reset_fallback);
    assert!(
        projection.display_payload_visits <= additive_display_payload_visit_bound(diffs.len()),
        "visible payload work must stay within binding plus materialization passes"
    );
}

#[test]
fn display_projection_payload_work_does_not_rescan_window_per_indexed_diff() {
    let (mut canonical_items, mut state) = deep_display_projection_fixture();
    let mut diffs = Vec::new();
    for index in 0..128 {
        diffs.extend([
            TimelineDiff::Set {
                index: 10,
                item: timeline_item(
                    &format!("$outside-set-{index}:test"),
                    Some("outside"),
                    "@sender:test",
                    false,
                ),
            },
            TimelineDiff::Remove { index: 10 },
            TimelineDiff::Insert {
                index: 10,
                item: timeline_item(
                    &format!("$outside-insert-{index}:test"),
                    Some("outside"),
                    "@sender:test",
                    false,
                ),
            },
        ]);
    }
    let display_before = state.display_items().to_vec();

    let projection = project_sdk_batch(
        &mut canonical_items,
        &mut state,
        &diffs,
        &historical_display_projection_context(),
    );

    assert!(!projection.used_reset_fallback);
    assert_eq!(projection.display_after, display_before);
    assert!(
        projection.display_payload_visits <= additive_display_payload_visit_bound(diffs.len()),
        "indexed diffs must not rescan all visible payloads per operation"
    );
}

#[test]
fn uncapped_restore_structural_visits_stay_inside_expected_log_envelope() {
    let represented_width = 2_048;
    let mut canonical_items = synthetic_projection_items(represented_width);
    let mut state =
        DisplayProjectionState::from_canonical_window(&canonical_items, 0..canonical_items.len());
    let display_before = state.display_items().to_vec();
    let mut diffs = Vec::new();
    for serial in 0..128 {
        let index = (serial * 37) % represented_width;
        diffs.extend([
            TimelineDiff::Set {
                index,
                item: timeline_item(
                    &format!("$restore-set-{serial}:test"),
                    Some("restore"),
                    "@sender:test",
                    false,
                ),
            },
            TimelineDiff::Remove { index },
            TimelineDiff::Insert {
                index,
                item: timeline_item(
                    &format!("$restore-insert-{serial}:test"),
                    Some("restore"),
                    "@sender:test",
                    false,
                ),
            },
        ]);
    }
    let restore_context = DisplayProjectionContext::for_timeline(
        &room_key().kind,
        &TimelineViewportObservation {
            at_bottom: true,
            ..TimelineViewportObservation::default()
        },
        true,
    );
    assert_eq!(restore_context.max_live_edge_items, None);

    let projection = project_sdk_batch(&mut canonical_items, &mut state, &diffs, &restore_context);

    assert!(!projection.used_reset_fallback);
    assert_display_projection_converges(display_before, &projection);
    assert!(
        projection.structural_node_visits
            <= expected_log_display_structural_visit_bound(represented_width, diffs.len()),
        "uncapped restore structural work exceeded the deterministic expected-log envelope"
    );
}

#[test]
fn sparse_indexed_structural_envelope_is_independent_of_canonical_history_length() {
    let represented_width = 256;
    let batch_len = 256;
    let measure = |canonical_len: usize| {
        let mut canonical_items = synthetic_projection_items(canonical_len);
        let start = canonical_len - represented_width;
        let mut state = DisplayProjectionState::from_canonical_window(
            &canonical_items,
            start..canonical_items.len(),
        );
        let diffs = (0..batch_len)
            .map(|serial| {
                let hidden_width = canonical_len - represented_width;
                TimelineDiff::Set {
                    index: 1 + (serial * 7_919) % hidden_width.saturating_sub(1),
                    item: timeline_item(
                        &format!("$sparse-set-{serial}:test"),
                        Some("sparse"),
                        "@sender:test",
                        false,
                    ),
                }
            })
            .collect::<Vec<_>>();
        let projection = project_sdk_batch(
            &mut canonical_items,
            &mut state,
            &diffs,
            &historical_display_projection_context(),
        );
        assert!(!projection.used_reset_fallback);
        projection.structural_node_visits
    };
    let bound = expected_log_display_structural_visit_bound(represented_width, batch_len);

    for visits in [measure(4_096), measure(65_536)] {
        assert!(
            visits <= bound,
            "structural work must be bounded by represented W and B, not canonical N"
        );
    }
}

#[test]
fn display_projection_backward_push_front_prepends_historical_page() {
    let mut canonical_items = synthetic_projection_items(200);
    let mut state = DisplayProjectionState::from_canonical_window(&canonical_items, 80..200);
    let display_before = state.display_items().to_vec();
    let older = timeline_item("$older:test", Some("older"), "@sender:test", false);

    let projection = project_sdk_batch(
        &mut canonical_items,
        &mut state,
        &[TimelineDiff::PushFront {
            item: older.clone(),
        }],
        &historical_display_projection_context(),
    );

    assert!(!projection.used_reset_fallback);
    assert_display_projection_converges(display_before, &projection);
    assert_eq!(state.display_items().first(), Some(&displayed(&older)));
    assert_eq!(
        state.display_items().len(),
        ROOM_REPLAY_INITIAL_ITEMS_MAX + 1
    );
}

#[test]
fn display_projection_clear_and_reset_replace_authoritative_display() {
    let mut canonical_items = vec![
        timeline_item("$one:test", Some("one"), "@sender:test", false),
        timeline_item("$two:test", Some("two"), "@sender:test", false),
    ];
    let mut state =
        DisplayProjectionState::from_canonical_window(&canonical_items, 0..canonical_items.len());
    let clear_before = state.display_items().to_vec();
    let cleared = project_sdk_batch(
        &mut canonical_items,
        &mut state,
        &[TimelineDiff::Clear],
        &historical_display_projection_context(),
    );
    assert!(!cleared.used_reset_fallback);
    assert_display_projection_converges(clear_before, &cleared);
    assert!(state.display_items().is_empty());

    let reset_items = vec![
        timeline_item("$reset-one:test", Some("one"), "@sender:test", false),
        timeline_item("$reset-two:test", Some("two"), "@sender:test", false),
    ];
    let reset_before = state.display_items().to_vec();
    let reset = project_sdk_batch(
        &mut canonical_items,
        &mut state,
        &[TimelineDiff::Reset {
            items: reset_items.clone(),
        }],
        &historical_display_projection_context(),
    );
    assert!(!reset.used_reset_fallback);
    assert_display_projection_converges(reset_before, &reset);
    assert_eq!(
        state.display_items(),
        reset_items.iter().map(displayed).collect::<Vec<_>>()
    );
}

#[test]
fn display_projection_invalid_translation_uses_validated_reset_fallback() {
    let mut canonical_items = vec![timeline_item(
        "$one:test",
        Some("one"),
        "@sender:test",
        false,
    )];
    let mut state = DisplayProjectionState::from_canonical_window(&canonical_items, 0..1);
    let display_before = state.display_items().to_vec();

    let projection = project_sdk_batch(
        &mut canonical_items,
        &mut state,
        &[TimelineDiff::Remove { index: 9 }],
        &historical_display_projection_context(),
    );

    assert!(projection.used_reset_fallback);
    assert!(matches!(
        projection.display_diffs.as_slice(),
        [TimelineDiff::Reset { items }] if items == &projection.display_after
    ));
    assert_display_projection_converges(display_before, &projection);
}

#[tokio::test]
async fn restore_terminal_flush_publishes_two_projected_batches_once_then_rebounds_live_edge() {
    let key = room_key();
    let mut canonical_items = synthetic_projection_items(200);
    let mut state = DisplayProjectionState::from_canonical_window(&canonical_items, 80..200);
    let mut desktop_model = state.display_items().to_vec();
    let restore_context = DisplayProjectionContext::for_timeline(
        &key.kind,
        &TimelineViewportObservation {
            at_bottom: true,
            ..TimelineViewportObservation::default()
        },
        true,
    );
    let mut restore_emit_buffer = Vec::new();
    for event_id in ["$restore-1:test", "$restore-2:test"] {
        let projected = project_sdk_batch(
            &mut canonical_items,
            &mut state,
            &[TimelineDiff::PushFront {
                item: timeline_item(event_id, Some("restore"), "@sender:test", false),
            }],
            &restore_context,
        );
        assert!(!projected.used_reset_fallback);
        restore_emit_buffer.extend(projected.display_diffs);
    }
    let expected_buffer = restore_emit_buffer.clone();
    let (actor_generations, stale_generation, current_generation) =
        replacement_generation_fixture(&key).await;
    let (event_tx, mut event_rx) = broadcast::channel(8);
    let mut next_batch_id = TimelineBatchId(7);

    assert_eq!(
        publish_restore_settlement_for_generation(
            &mut restore_emit_buffer,
            false,
            &mut next_batch_id,
            &event_tx,
            &actor_generations,
            &key,
            stale_generation,
            TimelineGeneration(3),
            &canonical_items,
            state.display_items(),
            RestoreSettlement {
                navigation_snapshot: None,
                terminal: Some((fake_rid(70), TimelineAnchorRestoreStatus::Found)),
            },
        ),
        None
    );
    assert_eq!(restore_emit_buffer, expected_buffer);
    assert_eq!(next_batch_id, TimelineBatchId(7));
    assert!(matches!(
        event_rx.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));

    let navigation_snapshot = derive_timeline_navigation_snapshot(
        &canonical_items,
        None,
        &TimelineViewportObservation::default(),
        None,
    );
    assert_eq!(
        publish_restore_settlement_for_generation(
            &mut restore_emit_buffer,
            false,
            &mut next_batch_id,
            &event_tx,
            &actor_generations,
            &key,
            current_generation,
            TimelineGeneration(3),
            &canonical_items,
            state.display_items(),
            RestoreSettlement {
                navigation_snapshot: Some(navigation_snapshot.clone()),
                terminal: Some((fake_rid(71), TimelineAnchorRestoreStatus::Found)),
            },
        ),
        Some(true)
    );
    let CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
        batch_id, diffs, ..
    }) = event_rx.recv().await.expect("one terminal restore update")
    else {
        panic!("restore flush must publish ItemsUpdated");
    };
    assert_eq!(batch_id, TimelineBatchId(7));
    assert_eq!(next_batch_id, TimelineBatchId(8));
    assert!(restore_emit_buffer.is_empty());
    apply_timeline_diffs_to_items(&mut desktop_model, &diffs);
    assert_eq!(desktop_model, state.display_items());
    assert!(matches!(
        event_rx.recv().await,
        Ok(CoreEvent::Timeline(TimelineEvent::NavigationUpdated {
            snapshot,
            ..
        })) if snapshot == navigation_snapshot
    ));
    assert!(matches!(
        event_rx.recv().await,
        Ok(CoreEvent::Timeline(TimelineEvent::AnchorRestoreFinished {
            request_id,
            status: TimelineAnchorRestoreStatus::Found,
            ..
        })) if request_id == fake_rid(71)
    ));
    let live = timeline_item(
        "$live-after-restore:test",
        Some("live"),
        "@sender:test",
        false,
    );
    let live_projection = project_sdk_batch(
        &mut canonical_items,
        &mut state,
        &[TimelineDiff::PushBack { item: live.clone() }],
        &DisplayProjectionContext::bounded_live_edge(),
    );
    assert_display_projection_converges(desktop_model, &live_projection);
    assert_eq!(state.display_items().len(), ROOM_REPLAY_INITIAL_ITEMS_MAX);
    assert_eq!(state.display_items().last(), Some(&displayed(&live)));
}

#[tokio::test]
async fn sdk_batch_generation_fence_rejects_activity_and_state_together() {
    let key = room_key();
    let (generations, stale_generation, _current_generation) =
        replacement_generation_fixture(&key).await;
    let mut canonical_items = vec![timeline_item(
        "$before:test",
        Some("before"),
        "@sender:test",
        false,
    )];
    let canonical_before = canonical_items.clone();
    let mut state = DisplayProjectionState::from_canonical_window(&canonical_items, 0..1);
    let state_before = state.clone();
    let (action_tx, mut action_rx) = mpsc::channel(1);

    let committed = commit_sdk_batch_for_generation(
        &generations,
        &key,
        stale_generation,
        &mut canonical_items,
        &mut state,
        &[TimelineDiff::PushBack {
            item: timeline_item("$stale:test", Some("stale"), "@sender:test", false),
        }],
        &historical_display_projection_context(),
        |_lease, _projected, _canonical, _display| {
            action_tx
                .try_send(vec![AppAction::ActivityRowsObserved { rows: Vec::new() }])
                .expect("current batch publication");
        },
    );

    assert!(committed.is_none());
    assert_eq!(canonical_items, canonical_before);
    assert_eq!(state, state_before);
    assert!(matches!(
        action_rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}

#[test]
fn room_latest_reply_projection_emits_one_stable_root_and_suppresses_reply() {
    let key = room_key();
    let before = timeline_item("$before:test", Some("before"), "@a:test", false);
    let mut root = timeline_item("$root:test", Some("root"), "@a:test", false);
    root.timestamp_ms = Some(100);
    let between = timeline_item("$between:test", Some("between"), "@a:test", false);
    let mut reply = timeline_item("$reply:test", Some("reply"), "@b:test", false);
    reply.thread_root = Some("$root:test".to_owned());
    reply.timestamp_ms = Some(400);
    let after = timeline_item("$after:test", Some("after"), "@a:test", false);
    let canonical = vec![before, root.clone(), between, reply, after];
    let mut state = DisplayProjectionState::from_canonical_window(&canonical, 0..canonical.len());
    let context = DisplayProjectionContext::for_timeline(
        &key.kind,
        &TimelineViewportObservation::default(),
        false,
    )
    .with_thread_roots(
        TimelineThreadRootOrder::LatestReply,
        vec![crate::threads_list::ThreadRootDisplayData {
            root_event_id: "$root:test".to_owned(),
            activity_event_id: "$reply:test".to_owned(),
            activity_timestamp_ms: Some(400),
            item: Some(root),
            aggregate: crate::threads_list::AuthoritativeThreadAggregate {
                reply_count: 1,
                latest_event_id: Some("$reply:test".to_owned()),
                latest_sender: Some("@b:test".to_owned()),
                latest_sender_label: Some("B".to_owned()),
                latest_body_preview: Some("reply".to_owned()),
                latest_timestamp_ms: Some(400),
            },
            pending: false,
            failure_kind: None,
        }],
    );

    state.reproject(&context);
    let rows = state.display_items();
    assert_eq!(rows.len(), 4);
    assert_eq!(
        rows.iter()
            .filter_map(|item| item.display_metadata.as_ref())
            .filter(|metadata| metadata.row_id == "thread-root:$root:test")
            .count(),
        1
    );
    let projected_root = rows
        .iter()
        .find(|item| {
            item.display_metadata
                .as_ref()
                .is_some_and(|metadata| metadata.row_id == "thread-root:$root:test")
        })
        .expect("projected root");
    let metadata = projected_root.display_metadata.as_ref().unwrap();
    assert_eq!(metadata.content_event_id.as_deref(), Some("$root:test"));
    assert_eq!(metadata.activity_event_id.as_deref(), Some("$reply:test"));
    assert!(rows.iter().all(|item| {
        !matches!(&item.id, TimelineItemId::Event { event_id } if event_id == "$reply:test")
    }));
}

#[test]
fn reset_push_and_reordered_batches_converge_without_root_disappearance() {
    let key = room_key();
    let root = timeline_item("$root:test", Some("root"), "@a:test", false);
    let mut reply = timeline_item("$reply:test", Some("reply"), "@b:test", false);
    reply.thread_root = Some("$root:test".to_owned());
    reply.timestamp_ms = Some(400);
    let root_data = crate::threads_list::ThreadRootDisplayData {
        root_event_id: "$root:test".to_owned(),
        activity_event_id: "$reply:test".to_owned(),
        activity_timestamp_ms: Some(400),
        item: Some(root.clone()),
        aggregate: crate::threads_list::AuthoritativeThreadAggregate {
            reply_count: 1,
            latest_event_id: Some("$reply:test".to_owned()),
            latest_sender: None,
            latest_sender_label: None,
            latest_body_preview: Some("reply".to_owned()),
            latest_timestamp_ms: Some(400),
        },
        pending: false,
        failure_kind: None,
    };
    let cases = [
        vec![vec![TimelineDiff::Reset {
            items: vec![root.clone(), reply.clone()],
        }]],
        vec![
            vec![TimelineDiff::PushBack { item: root.clone() }],
            vec![TimelineDiff::PushBack {
                item: reply.clone(),
            }],
        ],
        vec![
            vec![TimelineDiff::Reset {
                items: vec![reply.clone()],
            }],
            vec![TimelineDiff::PushFront { item: root.clone() }],
        ],
    ];
    let mut final_rows = Vec::new();
    for batches in cases {
        let mut canonical = Vec::new();
        let mut state = DisplayProjectionState::from_canonical_window(&canonical, 0..0);
        let context = DisplayProjectionContext::for_timeline(
            &key.kind,
            &TimelineViewportObservation::default(),
            false,
        )
        .with_thread_roots(
            TimelineThreadRootOrder::LatestReply,
            vec![root_data.clone()],
        );
        for batch in batches {
            project_sdk_batch(&mut canonical, &mut state, &batch, &context);
            let present = state.display_items().iter().any(|item| {
                item.display_metadata
                    .as_ref()
                    .is_some_and(|metadata| metadata.row_id == "thread-root:$root:test")
            });
            assert!(
                present,
                "a retained root must be present in every projection"
            );
        }
        final_rows.push(
            state
                .display_items()
                .iter()
                .map(super::timeline_item_render_id)
                .collect::<Vec<_>>(),
        );
    }
    assert!(final_rows.windows(2).all(|pair| pair[0] == pair[1]));
    assert_eq!(final_rows[0], ["thread-root:$root:test"]);
}

#[test]
fn latest_reply_keeps_retained_root_visible_after_root_leaves_window() {
    let key = room_key();
    let root = timeline_item("$root:test", Some("root"), "@a:test", false);
    let mut reply = timeline_item("$reply:test", Some("reply"), "@b:test", false);
    reply.thread_root = Some("$root:test".to_owned());
    reply.timestamp_ms = Some(400);
    let context = DisplayProjectionContext::for_timeline(
        &key.kind,
        &TimelineViewportObservation::default(),
        false,
    )
    .with_thread_roots(
        TimelineThreadRootOrder::LatestReply,
        vec![crate::threads_list::ThreadRootDisplayData {
            root_event_id: "$root:test".to_owned(),
            activity_event_id: "$reply:test".to_owned(),
            activity_timestamp_ms: Some(400),
            item: Some(root.clone()),
            aggregate: crate::threads_list::AuthoritativeThreadAggregate {
                reply_count: 1,
                latest_event_id: Some("$reply:test".to_owned()),
                latest_sender: Some("@b:test".to_owned()),
                latest_sender_label: Some("B".to_owned()),
                latest_body_preview: Some("reply".to_owned()),
                latest_timestamp_ms: Some(400),
            },
            pending: false,
            failure_kind: None,
        }],
    );
    let mut canonical = vec![root, reply];
    let mut state = DisplayProjectionState::from_canonical_window(&canonical, 0..2);
    state.reproject(&context);
    assert_eq!(
        state
            .display_items()
            .iter()
            .map(super::timeline_item_render_id)
            .collect::<Vec<_>>(),
        ["thread-root:$root:test"]
    );

    project_sdk_batch(
        &mut canonical,
        &mut state,
        &[TimelineDiff::Remove { index: 0 }],
        &context,
    );
    assert_eq!(
        state
            .display_items()
            .iter()
            .map(super::timeline_item_render_id)
            .collect::<Vec<_>>(),
        ["thread-root:$root:test"]
    );
    let projected_root = &state.display_items()[0];
    assert_eq!(timeline_item_event_id(projected_root), Some("$root:test"));
    assert_eq!(
        projected_root
            .display_metadata
            .as_ref()
            .and_then(|metadata| metadata.activity_event_id.as_deref()),
        Some("$reply:test")
    );
}

#[test]
fn thread_timeline_preserves_ordinary_reply_rows_even_when_room_roots_exist() {
    let key = thread_key();
    let mut reply = timeline_item("$reply:test", Some("reply"), "@b:test", false);
    reply.thread_root = Some("$root:test".to_owned());
    let canonical = vec![reply.clone()];
    let mut state = DisplayProjectionState::from_canonical_window(&canonical, 0..1);
    let context = DisplayProjectionContext::for_timeline(
        &key.kind,
        &TimelineViewportObservation::default(),
        false,
    )
    .with_thread_roots(
        TimelineThreadRootOrder::LatestReply,
        vec![crate::threads_list::ThreadRootDisplayData {
            root_event_id: "$root:test".to_owned(),
            activity_event_id: "$reply:test".to_owned(),
            activity_timestamp_ms: Some(1),
            item: None,
            aggregate: crate::threads_list::AuthoritativeThreadAggregate::default(),
            pending: true,
            failure_kind: None,
        }],
    );
    state.reproject(&context);
    assert_eq!(state.display_items().len(), 1);
    assert_eq!(
        timeline_item_event_id(&state.display_items()[0]),
        Some("$reply:test")
    );
}

#[test]
fn metadata_only_thread_activity_change_emits_one_stable_set() {
    let key = room_key();
    let root = timeline_item("$root:test", Some("root"), "@a:test", false);
    let canonical = vec![root.clone()];
    let mut state = DisplayProjectionState::from_canonical_window(&canonical, 0..1);
    let display_data = |timestamp| crate::threads_list::ThreadRootDisplayData {
        root_event_id: "$root:test".to_owned(),
        activity_event_id: "$reply:test".to_owned(),
        activity_timestamp_ms: Some(timestamp),
        item: Some(root.clone()),
        aggregate: crate::threads_list::AuthoritativeThreadAggregate {
            reply_count: 1,
            latest_event_id: Some("$reply:test".to_owned()),
            latest_sender: None,
            latest_sender_label: None,
            latest_body_preview: Some("reply".to_owned()),
            latest_timestamp_ms: Some(timestamp),
        },
        pending: false,
        failure_kind: None,
    };
    let context = |timestamp| {
        DisplayProjectionContext::for_timeline(
            &key.kind,
            &TimelineViewportObservation::default(),
            false,
        )
        .with_thread_roots(
            TimelineThreadRootOrder::LatestReply,
            vec![display_data(timestamp)],
        )
    };
    state.reproject(&context(400));
    let diffs = state.reproject(&context(401));
    assert!(matches!(
        diffs.as_slice(),
        [TimelineDiff::Set { index: 0, item }]
            if item.display_metadata.as_ref().is_some_and(|metadata|
                metadata.row_id == "thread-root:$root:test"
                    && metadata.display_timestamp_ms == Some(401))
    ));
}

#[test]
fn display_diff_application_normalizes_duplicate_render_identities() {
    let mut before = timeline_item("$before:test", Some("before"), "@a:test", false);
    before.timestamp_ms = Some(200);
    let mut latest_reply = timeline_item("$latest:test", Some("reply"), "@b:test", false);
    latest_reply.timestamp_ms = Some(400);
    latest_reply.thread_root = Some("$known-root:test".to_owned());
    let mut root = timeline_item("$known-root:test", Some("root"), "@a:test", false);
    root.timestamp_ms = None;
    let mut transaction = timeline_item(
        "$transaction-placeholder:test",
        Some("txn"),
        "@a:test",
        false,
    );
    transaction.id = TimelineItemId::Transaction {
        transaction_id: "local-1".to_owned(),
    };
    transaction.timestamp_ms = Some(450);
    let mut synthetic = timeline_item(
        "$synthetic-placeholder:test",
        Some("synthetic"),
        "@a:test",
        false,
    );
    synthetic.id = TimelineItemId::Synthetic {
        synthetic_id: "divider-1".to_owned(),
    };
    synthetic.timestamp_ms = Some(500);
    let mut display_items = vec![
        before.clone(),
        latest_reply.clone(),
        root.clone(),
        transaction.clone(),
        synthetic.clone(),
    ];

    // Overlapping scrollback must not add a second event/transaction/
    // synthetic row, regardless of the Push or Insert operation.
    apply_timeline_diffs_to_display_items(
        &mut display_items,
        &[
            TimelineDiff::PushFront {
                item: latest_reply.clone(),
            },
            TimelineDiff::PushBack {
                item: transaction.clone(),
            },
            TimelineDiff::Insert {
                index: 1,
                item: synthetic.clone(),
            },
            TimelineDiff::PushBack { item: root.clone() },
        ],
    );
    assert_eq!(display_items.len(), 5);

    // A Set for an overlapping Core slot updates its already-rendered row
    // without replacing/moving the item currently at the raw index.
    let mut updated_latest_reply = latest_reply.clone();
    updated_latest_reply.body = Some("updated reply".to_owned());
    apply_timeline_diffs_to_display_items(
        &mut display_items,
        &[TimelineDiff::Set {
            index: 0,
            item: updated_latest_reply.clone(),
        }],
    );
    assert_eq!(
        timeline_item_event_id(&display_items[0]),
        Some("$before:test")
    );
    assert_eq!(
        display_items
            .iter()
            .find(|item| timeline_item_event_id(item) == Some("$latest:test"))
            .and_then(|item| item.body.as_deref()),
        Some("updated reply")
    );

    // Remove and Reset use the normalized sequence as the webview does;
    // Reset keeps the first occurrence of each render identity.
    apply_timeline_diffs_to_display_items(&mut display_items, &[TimelineDiff::Remove { index: 0 }]);
    assert_eq!(
        timeline_item_event_id(&display_items[0]),
        Some("$latest:test")
    );
    apply_timeline_diffs_to_display_items(
        &mut display_items,
        &[TimelineDiff::Reset {
            items: vec![
                latest_reply.clone(),
                updated_latest_reply,
                root.clone(),
                root,
                transaction.clone(),
                transaction,
                synthetic.clone(),
                synthetic,
            ],
        }],
    );
    assert_eq!(display_items.len(), 4);

    // Truncate and Clear operate on the same normalized sequence, so stale
    // IDs cannot survive in the display mirror.
    apply_timeline_diffs_to_display_items(
        &mut display_items,
        &[TimelineDiff::Truncate { length: 1 }],
    );
    assert_eq!(display_items.len(), 1);
    assert_eq!(
        timeline_item_event_id(&display_items[0]),
        Some("$latest:test")
    );
    apply_timeline_diffs_to_display_items(&mut display_items, &[TimelineDiff::Clear]);
    assert!(display_items.is_empty());
}
