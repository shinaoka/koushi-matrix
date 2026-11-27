use super::*;

const PROFILE_ID: &str = "prof_00000000000000000000000000000001";

fn profile() -> GrantSubject {
    GrantSubject::profile(crate::consent::ProfileId::parse(PROFILE_ID).expect("a profile id"))
}

fn caller() -> GrantSubject {
    GrantSubject::client("caller")
}
use koushi_core::CoreRuntime;
use koushi_state::{ActivityRow, SessionInfo};

fn query(limit: usize) -> Query {
    Query::parse(&json!({"account_id":"account:synthetic","limit":limit})).unwrap()
}
fn current(count: usize) -> RecentActivitySnapshot {
    RecentActivitySnapshot {
        session: SessionInfo {
            homeserver: "https://example.test".into(),
            user_id: "@synthetic:example.test".into(),
            device_id: "SYNTHETIC".into(),
            authentication_method: Default::default(),
        },
        session_revision: 1,
        session_epoch: 0,
        notification_modes: Default::default(),
        rows: (0..count)
            .map(|index| {
                ActivityRow::event(
                    "!room:example.test".into(),
                    format!("$event-{index}"),
                    None,
                    "Room".into(),
                    None,
                    Some(format!("Preview {index}")),
                    (1000 - index) as u64,
                    false,
                    false,
                )
            })
            .collect(),
    }
}
fn eligible() -> BTreeSet<String> {
    BTreeSet::from(["!room:example.test".into()])
}
fn cursor(value: &Value) -> &str {
    value["cursor"].as_str().unwrap()
}
fn assert_reason(result: Result<Value, WireError>, reason: &str) {
    let error = result.unwrap_err();
    assert_eq!(error.code, ErrorCode::ResyncRequired);
    assert_eq!(error.reason.as_deref(), Some(reason));
}
fn runtime() -> (CoreRuntime, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let runtime = CoreRuntime::start_with_data_dir(directory.path().into());
    (runtime, directory)
}

#[test]
fn query_validates_bounds_and_normalizes_room_filter() {
    for args in [
        json!({"account_id":"a","limit":0}),
        json!({"account_id":"a","limit":101}),
        json!({"account_id":"a","limit":1.5}),
        json!({"account_id":"a","from_timestamp_ms":-1}),
        json!({"account_id":"a","from_timestamp_ms":5,"to_timestamp_ms":5}),
        json!({"account_id":"a","from_timestamp_ms":6,"to_timestamp_ms":5}),
        json!({"account_id":"a","room_ids":[""]}),
        json!({"account_id":"a","room_ids":vec!["room";65]}),
        json!({"account_id":"a","room_ids":["x".repeat(1025)]}),
    ] {
        assert!(Query::parse(&args).is_err());
    }
    assert_eq!(
        Query::parse(&json!({"account_id":"a","room_ids":["b","a","b"]}))
            .unwrap()
            .rooms,
        vec!["a", "b"]
    );
    assert_eq!(Query::parse(&json!({"account_id":"a"})).unwrap().limit, 50);
    assert_eq!(query(100).limit, 100);
}

#[tokio::test]
async fn continuation_is_immutable_when_new_current_rows_arrive() {
    let (runtime, _directory) = runtime();
    let pages = ActivityPages::default();
    let first = pages
        .read(
            &caller(),
            "account",
            query(2),
            None,
            runtime.attach(),
            current(5),
            &eligible(),
            &std::collections::HashMap::new(),
        )
        .unwrap();
    let mut newer = current(6);
    newer.rows[0].event_id = Some("$newest-added-later".into());
    let second = pages
        .read(
            &caller(),
            "account",
            query(2),
            Some(cursor(&first)),
            runtime.attach(),
            newer,
            &eligible(),
            &std::collections::HashMap::new(),
        )
        .unwrap();
    assert_eq!(second["items"][0]["event_id"], "$event-2");
    assert_eq!(
        second["snapshot_timestamp_ms"],
        first["snapshot_timestamp_ms"]
    );
    let third = pages
        .read(
            &caller(),
            "account",
            query(2),
            Some(cursor(&second)),
            runtime.attach(),
            current(0),
            &eligible(),
            &std::collections::HashMap::new(),
        )
        .unwrap();
    assert_eq!(third["items"].as_array().unwrap().len(), 1);
    assert!(third["cursor"].is_null());
    drop(pages);
    runtime.shutdown().await;
}

#[tokio::test]
async fn cursors_bind_caller_account_query_and_limit() {
    let (runtime, _directory) = runtime();
    let pages = ActivityPages::default();
    let first = pages
        .read(
            &caller(),
            "account",
            query(1),
            None,
            runtime.attach(),
            current(3),
            &eligible(),
            &std::collections::HashMap::new(),
        )
        .unwrap();
    for (subject, account, query) in [
        (GrantSubject::client("other"), "account", query(1)),
        // A profile never replays a client's cursor, even one whose name is
        // the profile's own id.
        (profile(), "account", query(1)),
        (GrantSubject::client(PROFILE_ID), "account", query(1)),
        (caller(), "other", query(1)),
        (caller(), "account", query(2)),
        (
            caller(),
            "account",
            Query {
                from: Some(1),
                ..query(1)
            },
        ),
        (
            caller(),
            "account",
            Query {
                rooms: vec!["!room:example.test".into()],
                ..query(1)
            },
        ),
    ] {
        assert_reason(
            pages.read(
                &subject,
                account,
                query,
                Some(cursor(&first)),
                runtime.attach(),
                current(3),
                &eligible(),
                &std::collections::HashMap::new(),
            ),
            "query_mismatch",
        );
    }
    drop(pages);
    runtime.shutdown().await;
}

#[tokio::test]
async fn malformed_unknown_and_expired_cursors_require_resync() {
    let (runtime, _directory) = runtime();
    let pages = ActivityPages::default();
    for malformed in ["", "not-a-cursor", &"g".repeat(64)] {
        assert_reason(
            pages.read(
                &caller(),
                "account",
                query(1),
                Some(malformed),
                runtime.attach(),
                current(3),
                &eligible(),
                &std::collections::HashMap::new(),
            ),
            "invalid_cursor",
        );
    }
    assert_reason(
        pages.read(
            &caller(),
            "account",
            query(1),
            Some(&"a".repeat(64)),
            runtime.attach(),
            current(3),
            &eligible(),
            &std::collections::HashMap::new(),
        ),
        "expired_cursor",
    );
    let first = pages
        .read(
            &caller(),
            "account",
            query(1),
            None,
            runtime.attach(),
            current(3),
            &eligible(),
            &std::collections::HashMap::new(),
        )
        .unwrap();
    pages.snapshots.lock().unwrap().front_mut().unwrap().created =
        Instant::now() - TTL - Duration::from_secs(1);
    assert_reason(
        pages.read(
            &caller(),
            "account",
            query(1),
            Some(cursor(&first)),
            runtime.attach(),
            current(3),
            &eligible(),
            &std::collections::HashMap::new(),
        ),
        "expired_cursor",
    );
    drop(pages);
    runtime.shutdown().await;
}

#[tokio::test]
async fn session_revision_identity_and_runtime_replacement_invalidate_cursors() {
    let (runtime, _directory) = runtime();
    let (replacement, _replacement_directory) = self::runtime();
    let pages = ActivityPages::default();
    for variant in 0..4 {
        let first = pages
            .read(
                &caller(),
                "account",
                query(1),
                None,
                runtime.attach(),
                current(3),
                &eligible(),
                &std::collections::HashMap::new(),
            )
            .unwrap();
        let mut next = current(3);
        if variant == 0 {
            next.session_revision += 1;
        }
        if variant == 3 {
            next.session_epoch += 1;
        }
        if variant == 1 {
            next.session.device_id = "OTHER_DEVICE".into();
        }
        let connection = if variant == 2 {
            replacement.attach()
        } else {
            runtime.attach()
        };
        assert_reason(
            pages.read(
                &caller(),
                "account",
                query(1),
                Some(cursor(&first)),
                connection,
                next,
                &eligible(),
                &std::collections::HashMap::new(),
            ),
            "session_changed",
        );
    }
    drop(pages);
    runtime.shutdown().await;
    replacement.shutdown().await;
}

#[tokio::test]
async fn room_left_or_newly_muted_invalidates_retained_cursor() {
    let (runtime, _directory) = runtime();
    let pages = ActivityPages::default();
    let first = pages
        .read(
            &caller(),
            "account",
            query(1),
            None,
            runtime.attach(),
            current(3),
            &eligible(),
            &std::collections::HashMap::new(),
        )
        .unwrap();
    assert_reason(
        pages.read(
            &caller(),
            "account",
            query(1),
            Some(cursor(&first)),
            runtime.attach(),
            current(3),
            &BTreeSet::new(),
            &std::collections::HashMap::new(),
        ),
        "room_access_changed",
    );
    let error = pages
        .read(
            &caller(),
            "account",
            Query {
                rooms: vec!["!left:example.test".into()],
                ..query(1)
            },
            None,
            runtime.attach(),
            current(3),
            &eligible(),
            &std::collections::HashMap::new(),
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::RoomNotFound);
    drop(pages);
    runtime.shutdown().await;
}

#[tokio::test]
async fn initial_reads_exclude_current_rows_outside_eligibility() {
    let (runtime, _directory) = runtime();
    let pages = ActivityPages::default();
    let mut snapshot = current(2);
    snapshot.rows[0].room_id = "!muted:example.test".into();
    let result = pages
        .read(
            &caller(),
            "account",
            query(2),
            None,
            runtime.attach(),
            snapshot,
            &eligible(),
            &std::collections::HashMap::new(),
        )
        .unwrap();
    assert!(
        result["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["room_id"] == "!room:example.test")
    );
    drop(pages);
    runtime.shutdown().await;
}

#[tokio::test]
async fn unicode_preview_truncation_and_unknown_content_are_explicit() {
    let (runtime, _directory) = runtime();
    let pages = ActivityPages::default();
    let mut snapshot = current(2);
    snapshot.rows[0].preview = Some("🦀界".repeat(3000));
    snapshot.rows[0].unread = true;
    snapshot.rows[1].preview = None;
    let result = pages
        .read(
            &caller(),
            "account",
            query(2),
            None,
            runtime.attach(),
            snapshot,
            &eligible(),
            &std::collections::HashMap::new(),
        )
        .unwrap();
    assert_eq!(
        result["items"][0]["preview"]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        4096
    );
    assert_eq!(result["items"][0]["preview_truncated"], true);
    assert_eq!(result["items"][0]["read_state"], "unknown");
    assert_eq!(result["items"][1]["content_available"], false);
    assert!(result["items"][1]["preview"].is_null());
    assert_eq!(result["items"][1]["read_state"], "unknown");
    assert_eq!(result["truncated"], true);
    assert_eq!(result["history_complete"], false);
    drop(pages);
    runtime.shutdown().await;
}

#[tokio::test]
async fn retention_enforces_snapshot_count_byte_budget_and_two_hundred_rows() {
    let (runtime, _directory) = runtime();
    let pages = ActivityPages::default();
    let first = pages
        .read(
            &caller(),
            "account",
            query(100),
            None,
            runtime.attach(),
            current(205),
            &eligible(),
            &std::collections::HashMap::new(),
        )
        .unwrap();
    for _ in 0..16 {
        pages
            .read(
                &caller(),
                "account",
                query(100),
                None,
                runtime.attach(),
                current(205),
                &eligible(),
                &std::collections::HashMap::new(),
            )
            .unwrap();
    }
    assert_reason(
        pages.read(
            &caller(),
            "account",
            query(100),
            Some(cursor(&first)),
            runtime.attach(),
            current(205),
            &eligible(),
            &std::collections::HashMap::new(),
        ),
        "expired_cursor",
    );
    assert_eq!(pages.snapshots.lock().unwrap().len(), 16);
    assert!(
        pages
            .snapshots
            .lock()
            .unwrap()
            .iter()
            .all(|s| s.rows.len() <= 200)
    );
    let mut snapshot = current(200);
    for row in &mut snapshot.rows {
        row.sender_id = Some("x".repeat(100_000));
    }
    for _ in 0..2 {
        pages
            .read(
                &caller(),
                "account",
                query(100),
                None,
                runtime.attach(),
                snapshot.clone(),
                &eligible(),
                &std::collections::HashMap::new(),
            )
            .unwrap();
    }
    {
        let retained = pages.snapshots.lock().unwrap();
        assert!(retained.iter().map(|s| s.bytes).sum::<usize>() <= MAX_BYTES);
        assert!(retained.iter().all(|s| s.rows.len() <= 200));
        assert!(retained.back().unwrap().truncated);
    }
    drop(pages);
    runtime.shutdown().await;
}

#[tokio::test]
async fn worst_case_unicode_pages_stay_wire_bounded_and_resume_without_row_loss() {
    let (runtime, _directory) = runtime();
    let pages = ActivityPages::default();
    let mut snapshot = current(100);
    for row in &mut snapshot.rows {
        row.preview = Some("🦀".repeat(4096));
        row.room_label = "界".repeat(2000);
        row.sender_label = Some("🦀".repeat(2000));
    }
    let mut result = pages
        .read(
            &caller(),
            "account",
            query(100),
            None,
            runtime.attach(),
            snapshot.clone(),
            &eligible(),
            &std::collections::HashMap::new(),
        )
        .unwrap();
    let mut events = Vec::new();
    loop {
        assert!(serde_json::to_vec(&result).unwrap().len() <= 512 * 1024);
        for row in result["items"].as_array().unwrap() {
            assert!(row["room_label"].as_str().unwrap().chars().count() <= 256);
            assert!(row["sender_label"].as_str().unwrap().chars().count() <= 256);
            assert_eq!(row["preview"].as_str().unwrap().chars().count(), 4096);
            events.push(row["event_id"].as_str().unwrap().to_owned());
        }
        let Some(next) = result["cursor"].as_str().map(str::to_owned) else {
            break;
        };
        result = pages
            .read(
                &caller(),
                "account",
                query(100),
                Some(&next),
                runtime.attach(),
                snapshot.clone(),
                &eligible(),
                &std::collections::HashMap::new(),
            )
            .unwrap();
    }
    assert_eq!(
        events,
        (0..100)
            .map(|index| format!("$event-{index}"))
            .collect::<Vec<_>>()
    );
    drop(pages);
    runtime.shutdown().await;
}

#[tokio::test]
async fn individually_oversized_rows_are_dropped_with_explicit_truncation() {
    let (runtime, _directory) = runtime();
    let pages = ActivityPages::default();
    let mut snapshot = current(2);
    snapshot.rows[0].event_id = Some("x".repeat(MAX_PAGE_BYTES + 1));
    let result = pages
        .read(
            &caller(),
            "account",
            query(100),
            None,
            runtime.attach(),
            snapshot,
            &eligible(),
            &std::collections::HashMap::new(),
        )
        .unwrap();
    assert_eq!(result["items"].as_array().unwrap().len(), 1);
    assert_eq!(result["items"][0]["event_id"], "$event-1");
    assert_eq!(result["truncated"], true);
    assert!(serde_json::to_vec(&result).unwrap().len() <= 512 * 1024);
    drop(pages);
    runtime.shutdown().await;
}

#[tokio::test]
async fn escaped_previews_fit_actual_tool_result_and_both_transport_envelopes() {
    use koushi_mcp::wire::{MAX_FRAME_BYTES, ServerMessage, encode_line};

    let (runtime, _directory) = runtime();
    for scalar in ["🦀", "\\", "\"", "\0"] {
        let pages = ActivityPages::default();
        let mut snapshot = current(100);
        for row in &mut snapshot.rows {
            row.preview = Some(scalar.repeat(4096));
            row.room_label = scalar.repeat(2000);
            row.sender_label = Some(scalar.repeat(2000));
        }
        let mut next = None;
        let mut events = Vec::new();
        loop {
            let result = pages
                .read(
                    &caller(),
                    "account",
                    query(100),
                    next.as_deref(),
                    runtime.attach(),
                    snapshot.clone(),
                    &eligible(),
                    &std::collections::HashMap::new(),
                )
                .unwrap();
            let tool_result = crate::handlers::tool_result(Ok(result.clone()));
            let rpc = json!({"jsonrpc":"2.0", "id":u64::MAX, "result":tool_result});
            assert!(serde_json::to_vec(&rpc).unwrap().len() < MAX_FRAME_BYTES);
            let ipc = ServerMessage::success(u64::MAX, tool_result);
            assert!(encode_line(&ipc).unwrap().len() <= MAX_FRAME_BYTES);
            for row in result["items"].as_array().unwrap() {
                assert_eq!(row["preview"].as_str().unwrap().chars().count(), 4096);
                events.push(row["event_id"].as_str().unwrap().to_owned());
            }
            next = result["cursor"].as_str().map(str::to_owned);
            if next.is_none() {
                break;
            }
        }
        assert_eq!(
            events,
            (0..100)
                .map(|index| format!("$event-{index}"))
                .collect::<Vec<_>>()
        );
        drop(pages);
    }
    runtime.shutdown().await;
}

#[tokio::test]
async fn changed_notification_mode_requires_resync_without_comparing_live_rows() {
    use koushi_state::{RoomNotificationMode, RoomNotificationSettings};
    let (runtime, _directory) = runtime();
    let pages = ActivityPages::default();
    let first = pages
        .read(
            &caller(),
            "account",
            query(1),
            None,
            runtime.attach(),
            current(3),
            &eligible(),
            &std::collections::HashMap::new(),
        )
        .unwrap();
    let changed = std::collections::HashMap::from([(
        "!room:example.test".into(),
        RoomNotificationSettings {
            mode: RoomNotificationMode::Mentions,
            ..Default::default()
        },
    )]);
    assert_reason(
        pages.read(
            &caller(),
            "account",
            query(1),
            Some(cursor(&first)),
            runtime.attach(),
            current(0),
            &eligible(),
            &changed,
        ),
        "filter_changed",
    );
    drop(pages);
    runtime.shutdown().await;
}
