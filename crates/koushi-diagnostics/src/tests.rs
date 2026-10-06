use super::*;
use std::sync::Arc;

fn event(stage: &'static str) -> DiagnosticEvent {
    DiagnosticEvent::new(DiagnosticLevel::Debug, "test", stage)
}

#[test]
fn keeps_latest_records_and_reports_drops() {
    let buffer = DiagnosticBuffer::new(2);
    buffer.record_at(1, event("one"));
    buffer.record_at(2, event("two"));
    buffer.record_at(3, event("three"));

    let snapshot = buffer.snapshot();
    assert_eq!(snapshot.dropped_records, 1);
    assert_eq!(
        snapshot
            .records
            .iter()
            .map(|record| record.event.stage)
            .collect::<Vec<_>>(),
        vec!["two", "three"]
    );
}

#[test]
fn records_concurrently_without_exceeding_capacity() {
    let buffer = Arc::new(DiagnosticBuffer::new(64));
    let workers = (0..8)
        .map(|_| {
            let buffer = Arc::clone(&buffer);
            std::thread::spawn(move || {
                for index in 0..100 {
                    buffer.record_at(index, event("concurrent"));
                }
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().unwrap();
    }
    let snapshot = buffer.snapshot();
    assert_eq!(snapshot.records.len(), 64);
    assert_eq!(snapshot.dropped_records, 736);
}

#[test]
fn batch_records_share_timestamp_and_preserve_order() {
    let buffer = DiagnosticBuffer::new(4);

    buffer.record_batch_at(
        42,
        [event("batch_one"), event("batch_two"), event("batch_three")],
    );

    let snapshot = buffer.snapshot();
    assert_eq!(snapshot.dropped_records, 0);
    assert_eq!(
        snapshot
            .records
            .iter()
            .map(|record| (record.timestamp_ms, record.event.stage))
            .collect::<Vec<_>>(),
        vec![(42, "batch_one"), (42, "batch_two"), (42, "batch_three")]
    );
}

#[test]
fn batch_keeps_latest_records_and_counts_every_drop() {
    let buffer = DiagnosticBuffer::new(2);
    buffer.record_at(1, event("existing"));

    buffer.record_batch_at(2, [event("one"), event("two"), event("three")]);

    let snapshot = buffer.snapshot();
    assert_eq!(snapshot.dropped_records, 2);
    assert_eq!(
        snapshot
            .records
            .iter()
            .map(|record| (record.timestamp_ms, record.event.stage))
            .collect::<Vec<_>>(),
        vec![(2, "two"), (2, "three")]
    );
}

#[test]
fn aggregate_counter_is_exported_outside_the_bounded_detail_ring() {
    let _guard = test_support::lock();
    reset_counter("synthetic_room_key_counter");
    increment_counter("synthetic_room_key_counter");
    increment_counter("synthetic_room_key_counter");

    let snapshot = super::snapshot();
    let summary = snapshot
        .records
        .iter()
        .find(|record| {
            record.event.source == "core.room_key_summary"
                && record.event.fields.iter().any(|field| {
                    field.key == "name"
                        && field.value == DiagnosticValue::Token("synthetic_room_key_counter")
                })
        })
        .expect("aggregate summary remains exportable independently of the detail ring");
    assert!(
        summary
            .event
            .fields
            .iter()
            .any(|field| { field.key == "count" && field.value == DiagnosticValue::Count(2) })
    );
    reset_counter("synthetic_room_key_counter");
}

#[test]
fn runtime_counter_contexts_reset_independently() {
    let first = DiagnosticCounterContext::new();
    let second = DiagnosticCounterContext::new();
    first.increment("runtime_counter");
    second.increment("runtime_counter");
    second.increment("runtime_counter");

    first.reset("runtime_counter");

    assert!(first.snapshot().records.is_empty());
    let second_snapshot = second.snapshot();
    assert!(second_snapshot.records.iter().any(|record| {
        record.event.fields.iter().any(|field| {
            field.key == "name" && field.value == DiagnosticValue::Token("runtime_counter")
        }) && record
            .event
            .fields
            .iter()
            .any(|field| field.key == "count" && field.value == DiagnosticValue::Count(2))
    }));
}

#[test]
fn concurrent_batches_remain_bounded_and_count_drops() {
    let buffer = Arc::new(DiagnosticBuffer::new(64));
    let workers = (0..8)
        .map(|worker| {
            let buffer = Arc::clone(&buffer);
            std::thread::spawn(move || {
                buffer.record_batch_at(worker, (0..100).map(|_| event("concurrent_batch")));
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().unwrap();
    }

    let snapshot = buffer.snapshot();
    assert_eq!(snapshot.records.len(), 64);
    assert_eq!(snapshot.dropped_records, 736);
    assert!(
        snapshot
            .records
            .windows(2)
            .all(|records| { records[0].timestamp_ms == records[1].timestamp_ms })
    );
}

#[test]
fn large_batch_retains_only_the_latest_capacity_without_timing_assumptions() {
    let buffer = DiagnosticBuffer::new(1_000);

    buffer.record_batch_at(7, (0..25_000).map(|_| event("large_batch")));

    let snapshot = buffer.snapshot();
    assert_eq!(snapshot.records.len(), 1_000);
    assert_eq!(snapshot.dropped_records, 24_000);
    assert!(
        snapshot
            .records
            .iter()
            .all(|record| record.timestamp_ms == 7)
    );
}

#[test]
fn formats_only_structured_fields() {
    let line = format_event(
        &DiagnosticEvent::new(DiagnosticLevel::Debug, "core.timeline", "actor_finish")
            .field(DiagnosticField::token("operation", "send_reaction"))
            .field(DiagnosticField::milliseconds("elapsed_ms", 42))
            .field(DiagnosticField::boolean("success", true)),
    );
    assert_eq!(
        line,
        "stage=actor_finish operation=send_reaction elapsed_ms=42 success=true"
    );
}

#[test]
fn recovers_after_records_mutex_poisoning() {
    let buffer = Arc::new(DiagnosticBuffer::new(1));
    let poisoned_buffer = Arc::clone(&buffer);
    let poisoner = std::thread::spawn(move || {
        let _records = poisoned_buffer.records.lock().unwrap();
        panic!("poison records mutex");
    });
    assert!(poisoner.join().is_err());

    buffer.record_at(7, event("after_records_poison"));

    let snapshot = buffer.snapshot();
    assert_eq!(snapshot.dropped_records, 0);
    assert_eq!(snapshot.records.len(), 1);
    assert_eq!(snapshot.records[0].event.stage, "after_records_poison");
}

#[test]
fn recovers_after_dropped_counter_mutex_poisoning() {
    let buffer = Arc::new(DiagnosticBuffer::new(1));
    buffer.record_at(1, event("first"));

    let poisoned_buffer = Arc::clone(&buffer);
    let poisoner = std::thread::spawn(move || {
        let _dropped_records = poisoned_buffer.dropped_records.lock().unwrap();
        panic!("poison dropped counter mutex");
    });
    assert!(poisoner.join().is_err());

    buffer.record_at(2, event("second"));

    let snapshot = buffer.snapshot();
    assert_eq!(snapshot.dropped_records, 1);
    assert_eq!(snapshot.records[0].event.stage, "second");
}

#[test]
fn clamps_pre_epoch_timestamp_to_zero() {
    let before_epoch = UNIX_EPOCH - std::time::Duration::from_millis(1);
    assert_eq!(timestamp_millis_at(before_epoch), 0);
}

#[test]
fn zero_capacity_drops_every_record() {
    let buffer = DiagnosticBuffer::new(0);
    buffer.record_at(1, event("dropped"));

    let snapshot = buffer.snapshot();
    assert!(snapshot.records.is_empty());
    assert_eq!(snapshot.dropped_records, 1);
}

#[test]
fn saturates_maximum_millisecond_duration() {
    assert_eq!(
        DiagnosticField::milliseconds("elapsed_ms", u128::MAX).value,
        DiagnosticValue::Milliseconds(u64::MAX)
    );
}

fn rotation_boundary(
    room_alias: u64,
    session_alias: u64,
    reason: &'static str,
) -> RotationBoundaryDiagnostic {
    RotationBoundaryDiagnostic {
        room_alias,
        previous_session_alias: None,
        new_session_alias: Some(session_alias),
        reason,
        creation_outcome: "created",
        first_share_outcome: "pending",
        first_send_correlation_present: false,
        discard_elapsed_ms: None,
        elapsed_ms: 3,
    }
}

#[test]
fn rotation_ledger_survives_general_ring_overflow_and_updates_one_session() {
    let detail = DiagnosticBuffer::new(1);
    let ledger = RotationDiagnosticLedger::new(2);
    ledger.record_at(10, rotation_boundary(1, 11, "expired_time"));
    ledger.record_at(11, rotation_boundary(2, 12, "explicit_discard"));
    for index in 0..10 {
        detail.record_at(index, event("churn"));
    }
    assert_eq!(detail.snapshot().dropped_records, 9);

    assert!(ledger.mark_first_send_correlation(11));
    let snapshot = ledger.snapshot();
    assert_eq!(snapshot.dropped_boundaries, 0);
    assert_eq!(snapshot.records.len(), 2);
    let first = &snapshot.records[0];
    let second = &snapshot.records[1];
    assert!(first.event.fields.iter().any(|field| {
        field.key == "first_send_correlation_present"
            && field.value == DiagnosticValue::Boolean(true)
    }));
    assert!(second.event.fields.iter().any(|field| {
        field.key == "first_send_correlation_present"
            && field.value == DiagnosticValue::Boolean(false)
    }));
}

#[test]
fn rotation_ledger_evicts_oldest_and_reset_clears_drop_count() {
    let ledger = RotationDiagnosticLedger::new(2);
    ledger.record_at(1, rotation_boundary(1, 1, "initial"));
    ledger.record_at(2, rotation_boundary(2, 2, "expired_message_count"));
    ledger.record_at(3, rotation_boundary(3, 3, "invalidated"));

    let snapshot = ledger.snapshot();
    assert_eq!(snapshot.dropped_boundaries, 1);
    assert_eq!(snapshot.records.len(), 2);
    assert!(snapshot.records.iter().all(|record| {
        !record.event.fields.iter().any(|field| {
            field.key == "new_session_alias"
                && field.value
                    == DiagnosticValue::OrdinalAlias {
                        kind: "session",
                        ordinal: 1,
                    }
        })
    }));

    ledger.reset();
    let reset = ledger.snapshot();
    assert!(reset.records.is_empty());
    assert_eq!(reset.dropped_boundaries, 0);
}

#[test]
fn exported_snapshot_includes_rotation_ledger_and_its_drop_counter() {
    let _guard = test_support::lock();
    reset_rotation_ledger();
    for session in 1..=129 {
        record_rotation_boundary(rotation_boundary(session, session, "expired_time"));
    }

    let snapshot = super::snapshot();
    assert!(snapshot.records.iter().any(|record| {
        record.event.source == "core.room_key_rotation"
            && record.event.fields.iter().any(|field| {
                field.key == "new_session_alias"
                    && field.value
                        == DiagnosticValue::OrdinalAlias {
                            kind: "session",
                            ordinal: 129,
                        }
            })
    }));
    assert!(snapshot.records.iter().any(|record| {
        record.event.source == "core.room_key_summary"
            && record.event.fields.iter().any(|field| {
                field.key == "name"
                    && field.value == DiagnosticValue::Token("rotation_boundaries_dropped")
            })
            && record
                .event
                .fields
                .iter()
                .any(|field| field.key == "count" && field.value == DiagnosticValue::Count(1))
    }));
    reset_rotation_ledger();
}

#[test]
fn rotation_ledger_exports_only_closed_private_data_free_fields() {
    let ledger = RotationDiagnosticLedger::new(1);
    ledger.record_at(1, rotation_boundary(4, 5, "membership_or_device_change"));
    let encoded = format!("{:?}", ledger.snapshot().records);
    for forbidden in [
        "room_id",
        "event_id",
        "session_id",
        "device_id",
        "user_id",
        "fingerprint",
        "ciphertext",
        "sender_key",
        "identity_key",
        "raw_error",
        "example.invalid",
    ] {
        assert!(!encoded.contains(forbidden), "privacy leak: {forbidden}");
    }
}

#[test]
fn cursor_window_survives_ring_eviction() {
    let buffer = DiagnosticBuffer::new(3);
    buffer.record(event("before-a"));
    buffer.record(event("before-b"));
    buffer.record(event("before-c"));
    // The ring is full: a snapshot length no longer moves, the cursor does.
    let cursor = buffer.end_cursor();
    assert_eq!(cursor, 3);
    buffer.record(event("after-a"));
    buffer.record(event("after-b"));
    assert_eq!(buffer.snapshot().records.len(), 3);
    let stages = buffer
        .records_since(cursor)
        .expect("nothing after the cursor was evicted")
        .into_iter()
        .map(|record| record.event.stage)
        .collect::<Vec<_>>();
    assert_eq!(stages, ["after-a", "after-b"]);
    assert!(
        buffer
            .records_since(buffer.end_cursor())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn cursor_window_reports_evicted_records_instead_of_hiding_them() {
    let buffer = DiagnosticBuffer::new(2);
    let cursor = buffer.end_cursor();
    buffer.record_batch([event("a"), event("b"), event("c")]);
    assert_eq!(buffer.records_since(cursor), Err(1));
    let retained = buffer.records_since(cursor + 1).unwrap();
    assert_eq!(retained.len(), 2);
}
