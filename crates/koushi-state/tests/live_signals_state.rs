/// Unit tests for the live-signals receipt projection (normalize_receipts).
///
/// Verifies that:
///  - own_user_id is excluded from readers and counts.
///  - exclusion applies even when the own user read on a different device
///    (i.e. the receipt's user_id matches own_user_id regardless of origin).
///  - total_count and overflow_count are computed from the remaining readers
///    after own exclusion.
///  - other readers are unaffected.
use koushi_state::{LiveEventReceipts, LiveReadReceipt, LiveRoomSignalUpdate};

fn make_receipt(user_id: &str, ts: u64) -> LiveReadReceipt {
    LiveReadReceipt {
        user_id: user_id.to_owned(),
        display_name: Some(user_id.to_owned()),
        original_display_label: String::new(),
        avatar: None,
        timestamp_ms: Some(ts),
    }
}

fn signals_for(
    receipts: Vec<LiveReadReceipt>,
    own_user_id: Option<&str>,
) -> koushi_state::LiveEventReceiptSummary {
    let update = LiveRoomSignalUpdate {
        receipts_by_event: vec![LiveEventReceipts {
            event_id: "$ev:localhost".to_owned(),
            receipts,
        }],
        fully_read_event_id: None,
        typing_user_ids: vec![],
    };
    let room_signals =
        update.into_room_signals_with_profiles(&koushi_state::ProfileState::default(), own_user_id);
    room_signals
        .receipts_by_event
        .get("$ev:localhost")
        .cloned()
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// 1. Own receipt excluded from readers and count
// ---------------------------------------------------------------------------

#[test]
fn own_receipt_excluded_from_readers_and_count() {
    let own = "@self:localhost";
    let receipts = vec![
        make_receipt("@alice:localhost", 2000),
        make_receipt("@bob:localhost", 1000),
        make_receipt(own, 3000),
    ];

    let summary = signals_for(receipts, Some(own));

    // Own is absent from the readers list.
    assert!(
        summary.readers.iter().all(|r| r.user_id != own),
        "own user_id must not appear in readers"
    );
    // Only alice and bob remain.
    assert_eq!(summary.total_count, 2, "total_count must exclude own");
    assert_eq!(summary.overflow_count, 0);
    // Readers are sorted most-recent-first: alice (ts=2000) before bob (ts=1000).
    assert_eq!(summary.readers[0].user_id, "@alice:localhost");
    assert_eq!(summary.readers[1].user_id, "@bob:localhost");
}

// ---------------------------------------------------------------------------
// 2. Own receipt excluded when it is the only reader
// ---------------------------------------------------------------------------

#[test]
fn own_only_receipt_yields_empty_summary() {
    let own = "@self:localhost";
    let receipts = vec![make_receipt(own, 5000)];

    let summary = signals_for(receipts, Some(own));

    assert!(
        summary.readers.is_empty(),
        "readers must be empty when only own read"
    );
    assert_eq!(summary.total_count, 0);
    assert_eq!(summary.overflow_count, 0);
}

// ---------------------------------------------------------------------------
// 3. Own receipt excluded even when read "on another device"
//    (receipt user_id still matches own_user_id — multi-device scenario)
// ---------------------------------------------------------------------------

#[test]
fn own_read_on_another_device_still_excluded() {
    let own = "@self:localhost";
    // Two receipts for the same user_id (own) at different timestamps — as if
    // own read on two devices.  Both must be excluded.
    let receipts = vec![
        make_receipt(own, 1000), // device A
        make_receipt(own, 9000), // device B (newer)
        make_receipt("@carol:localhost", 500),
    ];

    let summary = signals_for(receipts, Some(own));

    assert!(
        summary.readers.iter().all(|r| r.user_id != own),
        "own must be excluded regardless of device count"
    );
    assert_eq!(summary.total_count, 1, "only carol remains");
    assert_eq!(summary.readers[0].user_id, "@carol:localhost");
}

// ---------------------------------------------------------------------------
// 4. When own_user_id is None, all receipts appear (no exclusion)
// ---------------------------------------------------------------------------

#[test]
fn no_exclusion_when_own_user_id_is_none() {
    let receipts = vec![
        make_receipt("@x:localhost", 100),
        make_receipt("@y:localhost", 200),
    ];

    let summary = signals_for(receipts, None);

    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.readers.len(), 2);
}
