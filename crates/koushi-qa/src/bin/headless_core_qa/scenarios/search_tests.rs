use super::{CRAWL_BUDGET_SECS, CRAWL_TIMEOUT_SECS, crawler_state_counts};
use crate::{SearchCrawlerFailureKind, SearchCrawlerRoomState};
use std::collections::BTreeMap;

/// Smallest per-room crawl budget the stage must leave the crawler after the
/// product's automatic-crawl startup hold. A focused stage that reaches the
/// crawler after the hold has elapsed completes one room in well under a
/// minute, so 60 s is a real margin rather than an arbitrary extension.
const MIN_CRAWL_BUDGET_SECS: u64 = 60;

// Compile-time half of the guard: a budget under the minimum leaves the crawler
// no room after the product hold, which is the #1198 failure mode.
const _: () = assert!(CRAWL_BUDGET_SECS >= MIN_CRAWL_BUDGET_SECS);

/// The `crawl_backfill` deadline must outlast the product's own automatic-crawl
/// startup hold plus a real crawl budget (#1198). A deadline that is only as
/// long as the hold expires with every room still `Queued` and no crawl ever
/// started; sizing it from `koushi_core::search::CRAWLER_STARTUP_DELAY` keeps
/// the waiter and the product from drifting apart again.
#[test]
fn crawl_backfill_deadline_covers_product_hold_and_minimum_budget() {
    let hold_secs = koushi_core::search::CRAWLER_STARTUP_DELAY.as_secs();
    assert!(
        CRAWL_TIMEOUT_SECS >= hold_secs + MIN_CRAWL_BUDGET_SECS,
        "crawl_backfill deadline {CRAWL_TIMEOUT_SECS}s must cover the {hold_secs}s product hold plus a {MIN_CRAWL_BUDGET_SECS}s crawl budget"
    );
}

#[test]
fn crawler_state_counts_bucket_every_room_state() {
    let mut rooms = BTreeMap::new();
    rooms.insert("a".to_owned(), SearchCrawlerRoomState::Idle);
    rooms.insert("b".to_owned(), SearchCrawlerRoomState::Queued);
    rooms.insert(
        "c".to_owned(),
        SearchCrawlerRoomState::Running {
            processed: 3,
            indexed: 2,
        },
    );
    rooms.insert(
        "d".to_owned(),
        SearchCrawlerRoomState::Completed { indexed: 4 },
    );
    rooms.insert(
        "e".to_owned(),
        SearchCrawlerRoomState::Failed {
            kind: SearchCrawlerFailureKind::RoomNotFound,
        },
    );

    assert_eq!(crawler_state_counts(&rooms), [1, 1, 1, 1, 1]);
    assert_eq!(crawler_state_counts(&BTreeMap::new()), [0, 0, 0, 0, 0]);
}

/// The `crawl_backfill_timeout` token reads the crawler's own token-only backlog
/// sample. It must report the latest `core.search` sample and ignore every other
/// source, so one crate's diagnostics cannot be misread as crawler state.
#[test]
fn search_crawl_backlog_summary_reads_only_the_latest_crawler_sample() {
    use koushi_diagnostics::{
        DiagnosticEvent, DiagnosticField, DiagnosticLevel, DiagnosticRecord, DiagnosticSnapshot,
        DiagnosticValue,
    };

    let sample = |source: &'static str, queued_index: u64, pending_retries: u64| DiagnosticRecord {
        timestamp_ms: 1,
        event: DiagnosticEvent {
            level: DiagnosticLevel::Debug,
            source,
            stage: "crawl_pump_held",
            fields: vec![
                DiagnosticField {
                    key: "queued_index",
                    value: DiagnosticValue::Count(queued_index),
                },
                DiagnosticField {
                    key: "pending_retries",
                    value: DiagnosticValue::Count(pending_retries),
                },
            ],
        },
    };

    let snapshot = DiagnosticSnapshot {
        records: vec![
            sample("core.search", 3, 2),
            sample("core.sync", 9, 9),
            sample("core.search", 1, 4),
        ],
        dropped_records: 0,
    };
    assert_eq!(
        crate::diagnostics::search_crawl_backlog_summary(&snapshot),
        Some((1, 4))
    );
    assert_eq!(
        crate::diagnostics::search_crawl_backlog_summary(&DiagnosticSnapshot {
            records: Vec::new(),
            dropped_records: 0,
        }),
        None
    );
}
