use super::*;
use matrix_sdk::ruma::{MilliSecondsSinceUnixEpoch, events::receipt::Receipt, user_id};

fn endpoint(event: &str, timestamp: u32) -> Option<ReceiptEndpoint> {
    Some(ReceiptEndpoint {
        event_id: event.to_owned(),
        receipts: ReadReceiptSnapshot::from([(
            user_id!("@reader:example.org").to_owned(),
            Receipt::new(MilliSecondsSinceUnixEpoch(timestamp.into())),
        )]),
    })
}

#[test]
fn reader_initials_preserve_ascii_policy_and_valid_unicode() {
    for (label, expected) in [
        ("Current alias", "CU"),
        ("日a本b", "AB"),
        ("日本語", "日本"),
        ("😀😀", "😀😀"),
        ("علي", "عل"),
    ] {
        assert_eq!(reader_initials(label), expected);
    }
}

#[tokio::test]
async fn source_removal_waits_for_commit_and_invalidates_upgraded_witness() {
    let key = koushi_protocol::TimelineKey::room(
        koushi_protocol::AccountKey("account".into()),
        "!r:example.org",
    );
    let gate = Arc::new(super::super::navigation::TimelineActorGenerationGate::default());
    let generation = gate.activate_after_quiescence(&key).await.generation;
    let index = ReceiptReaderIndex::new(ReadReceiptSnapshot::default());
    let mut raw = RawReceiptWindow {
        total_count: 0,
        start: 0,
        receipts: vec![],
        profiles: vec![],
        owner: None,
        epoch: index.window_epoch(),
    };
    raw.bind_owner(&gate, &serde_json::from_value(serde_json::json!({"key": key, "projection_request_id": {"connection_id":"1", "sequence":"1"}, "generation":"1", "event_id":"$event"})).unwrap(), generation);
    let window = raw.into_resolved(koushi_state::CatalogLocale::En);
    let upgraded = window.epoch.upgrade().unwrap();
    let (attempted_tx, attempted_rx) = std::sync::mpsc::channel();
    let (finished_tx, finished_rx) = std::sync::mpsc::channel();
    std::thread::scope(|threads| {
        assert_eq!(
            window.commit_if_current(|| {
                threads.spawn(move || {
                    attempted_tx.send(()).unwrap();
                    drop(index);
                    finished_tx.send(()).unwrap();
                });
                attempted_rx
                    .recv_timeout(std::time::Duration::from_secs(1))
                    .unwrap();
                assert!(matches!(
                    finished_rx.try_recv(),
                    Err(std::sync::mpsc::TryRecvError::Empty)
                ));
                42
            }),
            Some(42)
        );
        finished_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
    });
    assert!(!upgraded.lock().unwrap().valid);
    assert!(
        window
            .commit_if_current(|| panic!("retired source committed"))
            .is_none()
    );
}

#[tokio::test]
async fn raw_window_does_not_keep_replaced_actor_authority() {
    let key = koushi_protocol::TimelineKey::room(
        koushi_protocol::AccountKey("account".into()),
        "!room:example.org",
    );
    let gate = Arc::new(super::super::navigation::TimelineActorGenerationGate::default());
    let generation = gate.activate_after_quiescence(&key).await.generation;
    let mut window = RawReceiptWindow {
        total_count: 0,
        start: 0,
        receipts: Vec::new(),
        profiles: Vec::new(),
        owner: None,
        epoch: std::sync::Weak::new(),
    };
    let epoch = Arc::new(std::sync::Mutex::new(ReceiptEpoch {
        valid: true,
        revision: Some(1),
    }));
    window.epoch = Arc::downgrade(&epoch);
    window.bind_owner(&gate, &serde_json::from_value(serde_json::json!({"key": key, "projection_request_id": {"connection_id":"1", "sequence":"1"}, "generation":"1", "event_id":"$event"})).unwrap(), generation);
    assert!(window.acquire_source().is_some());
    assert_eq!(window.commit_if_current(|| 42), Some(42));
    epoch.lock().unwrap().valid = false;
    assert!(
        window
            .commit_if_current(|| panic!("invalid epoch committed"))
            .is_none()
    );
    epoch.lock().unwrap().valid = true;
    gate.invalidate_and_quiesce(&key).await;
    assert!(window.acquire_source().is_none());
    assert!(
        window
            .commit_if_current(|| panic!("retired actor committed"))
            .is_none()
    );
}

#[test]
fn window_read_rejects_stale_sources_and_missing_events() {
    use koushi_protocol::view::{ReaderWindowLimit, ReceiptSourceRef, TimelineViewSource};
    use koushi_protocol::{
        AccountKey, RequestId, RuntimeConnectionId, TimelineGeneration, TimelineKey,
    };
    let current = TimelineViewSource {
        key: TimelineKey::room(AccountKey("account".into()), "!room:example.org"),
        projection_request_id: RequestId {
            connection_id: RuntimeConnectionId(1),
            sequence: 2,
        },
        generation: TimelineGeneration(3),
    };
    let mut source = ReceiptSourceRef {
        timeline: current.clone(),
        event_id: "$a".into(),
    };
    let mut mirror = ReceiptEndpointMirror::from_entries([endpoint("$a", 1)].into());
    let limit = ReaderWindowLimit::try_from(1).unwrap();
    let window = mirror
        .read_window(&source, &current, 0, limit, None)
        .unwrap();
    assert_eq!(window.total_count, 1);
    assert_eq!(window.receipts.len(), 1);
    source.timeline.generation = TimelineGeneration(2);
    assert!(
        mirror
            .read_window(&source, &current, 0, limit, None)
            .is_none()
    );
    source.timeline = current.clone();
    source.timeline.projection_request_id.sequence += 1;
    assert!(
        mirror
            .read_window(&source, &current, 0, limit, None)
            .is_none()
    );
    source.timeline = current.clone();
    source.timeline.key.kind = koushi_protocol::TimelineKind::Thread {
        room_id: "!room:example.org".into(),
        root_event_id: "$root".into(),
    };
    assert!(
        mirror
            .read_window(&source, &current, 0, limit, None)
            .is_none()
    );
    source.timeline = current.clone();
    source.event_id = "$absent".into();
    assert!(
        mirror
            .read_window(&source, &current, 0, limit, None)
            .is_none()
    );
    source.event_id = "$a".into();
    let _upgraded_old_epoch = window.epoch.upgrade().unwrap();
    mirror.apply_endpoint_batch([VectorDiff::Set {
        index: 0,
        value: Some(ReceiptEndpoint {
            event_id: "$a".into(),
            receipts: ReadReceiptSnapshot::default(),
        }),
    }]);
    assert!(!window.source_is_current());
    let empty = mirror
        .read_window(&source, &current, 0, limit, None)
        .unwrap();
    assert_eq!(empty.total_count, 0);
    assert!(empty.receipts.is_empty());
    assert!(empty.source_is_current());
    let _upgraded_removed_epoch = empty.epoch.upgrade().unwrap();
    mirror.apply_endpoint_batch([VectorDiff::Remove { index: 0 }]);
    assert!(!empty.source_is_current());
    assert!(
        mirror
            .read_window(&source, &current, 0, limit, None)
            .is_none()
    );
}

#[test]
fn repeated_sets_and_remove_reinsert_produce_net_endpoints() {
    let mut mirror =
        ReceiptEndpointMirror::from_entries([endpoint("$a", 1), None, endpoint("$b", 2)].into());
    let changes = mirror.apply_endpoint_batch([
        VectorDiff::Set {
            index: 0,
            value: endpoint("$a", 3),
        },
        VectorDiff::Set {
            index: 0,
            value: endpoint("$a", 4),
        },
        VectorDiff::Remove { index: 2 },
        VectorDiff::PushBack {
            value: endpoint("$b", 5),
        },
        VectorDiff::PushBack {
            value: endpoint("$temporary", 6),
        },
        VectorDiff::PopBack,
    ]);
    assert_eq!(changes.len(), 2);
    let reader = user_id!("@reader:example.org");
    for (event, old, new) in [("$a", 1_u32, 4_u32), ("$b", 2, 5)] {
        let change = &changes[event];
        assert_eq!(
            change.before.as_ref().unwrap().get(reader).unwrap().ts,
            Some(MilliSecondsSinceUnixEpoch(old.into()))
        );
        assert_eq!(
            change.after.as_ref().unwrap().get(reader).unwrap().ts,
            Some(MilliSecondsSinceUnixEpoch(new.into()))
        );
    }
    assert_eq!(mirror.entries.len(), 3);
    assert!(mirror.entries[1].is_none());
    assert!(
        mirror
            .apply_endpoint_batch([VectorDiff::Set {
                index: 0,
                value: endpoint("$a", 4)
            },])
            .is_empty()
    );
    let latest = mirror.entries[0]
        .as_ref()
        .unwrap()
        .receipts
        .get(reader)
        .unwrap();
    let indexed = mirror
        .readers("$a")
        .unwrap()
        .window(0, 1, None)
        .next()
        .unwrap()
        .1;
    assert!(
        std::ptr::eq(latest, indexed),
        "logical no-op still adopts the latest shared endpoint for the next diff"
    );
}

#[test]
fn reset_captures_removed_added_and_surviving_events() {
    let mut mirror =
        ReceiptEndpointMirror::from_entries([endpoint("$a", 1), endpoint("$b", 2)].into());
    let changes = mirror.apply_endpoint_batch([VectorDiff::Reset {
        values: [None, endpoint("$b", 3), endpoint("$c", 4)].into(),
    }]);
    assert_eq!(changes.len(), 3);
    assert!(changes["$a"].before.is_some());
    assert!(changes["$a"].after.is_none());
    assert!(changes["$b"].before.is_some());
    assert!(changes["$b"].after.is_some());
    assert!(changes["$c"].before.is_none());
    assert!(changes["$c"].after.is_some());
    assert_eq!(mirror.entries.len(), 3);
    assert!(mirror.readers("$a").is_none());
    assert_eq!(mirror.readers("$b").unwrap().total(None), 1);
    let removals = mirror.apply_endpoint_batch([
        VectorDiff::PushFront {
            value: endpoint("$temporary", 5),
        },
        VectorDiff::Insert {
            index: 2,
            value: None,
        },
        VectorDiff::Append {
            values: [endpoint("$appended", 6)].into(),
        },
        VectorDiff::PopFront,
        VectorDiff::Truncate { length: 3 },
        VectorDiff::Clear,
    ]);
    assert_eq!(removals.len(), 2);
    assert!(removals["$b"].after.is_none());
    assert!(removals["$c"].after.is_none());
    assert!(mirror.entries.is_empty());
    assert!(mirror.readers.is_empty());
}
