use super::*;

use std::sync::Arc;

use koushi_sdk::MatrixClientSession;
use koushi_state::{SessionAuthenticationMethod, SessionInfo};
use matrix_sdk::{
    ruma::{event_id, room_id, user_id},
    test_utils::mocks::{MatrixMockServer, RoomMessagesResponseTemplate},
};
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};

#[test]
fn crawler_page_producer_records_typed_progress_without_environment_switch() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    trace_crawler_page(DiagnosticLevel::Debug, "test_progress", 12, 9, 4);
    let record = koushi_diagnostics::snapshot()
        .records
        .into_iter()
        .rev()
        .find(|record| {
            record.event.source == "core.startup" && record.event.stage == "crawler_page"
        })
        .expect("crawler producer should record");
    assert!(
        record
            .event
            .fields
            .iter()
            .any(|field| field.key == "processed")
    );
    assert!(
        record
            .event
            .fields
            .iter()
            .any(|field| field.key == "indexed")
    );
    for key in ["history_source", "timeline_reuse", "persistence"] {
        assert!(
            record.event.fields.iter().any(|field| field.key == key),
            "crawler diagnostic must identify shared history persistence: {key}"
        );
    }
}

#[tokio::test]
async fn crawler_page_is_visible_to_the_normal_room_event_cache() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!crawler-reuse:example.invalid");
    let event_factory = EventFactory::new()
        .room(room_id)
        .sender(user_id!("@crawler:test.invalid"));

    // The sync timeline establishes the SDK-owned pagination boundary. The
    // crawler must consume this boundary through RoomEventCache pagination so
    // its result is available to an ordinary room subscription.
    client
        .event_cache()
        .subscribe()
        .expect("event cache subscription");
    server
        .mock_sync()
        .ok_and_run(&client, |builder| {
            builder.add_joined_room(
                JoinedRoomBuilder::new(room_id)
                    .set_timeline_limited()
                    .set_timeline_prev_batch("crawler-prev")
                    .add_timeline_event(
                        event_factory
                            .text_msg("latest")
                            .event_id(event_id!("$crawler-latest")),
                    ),
            );
        })
        .await;

    server
        .mock_room_messages()
        .match_from("crawler-prev")
        .ok(RoomMessagesResponseTemplate::default()
            .events(vec![
                event_factory
                    .text_msg("older second")
                    .event_id(event_id!("$crawler-older-2")),
                event_factory
                    .text_msg("older first")
                    .event_id(event_id!("$crawler-older-1")),
            ])
            .end_token("crawler-start"))
        .mock_once()
        .mount()
        .await;

    let session_info = SessionInfo {
        homeserver: server.server().uri(),
        user_id: client.user_id().expect("mock client user id").to_string(),
        device_id: client
            .device_id()
            .expect("mock client device id")
            .to_string(),
        authentication_method: SessionAuthenticationMethod::Unknown,
    };
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client.clone(),
        session_info,
    ));
    let checkpoint = HistoryCrawlCheckpoint::new(
        room_id.to_string(),
        SearchCrawlerSettings::default(),
        1,
        true,
    );

    let result = run_history_crawl_page(session, AccountWorkScheduler::default(), checkpoint).await;
    let HistoryCrawlPageResult::Success { messages, .. } = result else {
        panic!("crawler page should complete through SDK pagination");
    };
    // #996: the newest event arrived by sync and was already cached before the
    // crawl; backward pagination never returns it, so the first page must
    // index the cached events too.
    let indexed_ids = messages
        .iter()
        .filter_map(|message| match message {
            SearchIndexMessage::Upsert { event_id, .. } => Some(event_id.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        indexed_ids.len(),
        3,
        "crawler should index the cached latest event and both historical events: {indexed_ids:?}"
    );
    assert!(indexed_ids.contains(&"$crawler-latest"));
    assert!(indexed_ids.contains(&"$crawler-older-1"));

    let room = client.get_room(room_id).expect("joined room");
    let (cache, _drop_handles) = room.event_cache().await.expect("room event cache");
    let visible = cache.events().await.expect("cached timeline events");
    let visible_ids = visible
        .iter()
        .filter_map(|event| event.event_id().map(ToString::to_string))
        .collect::<Vec<_>>();
    assert!(visible_ids.iter().any(|id| id == "$crawler-older-1"));
    assert!(visible_ids.iter().any(|id| id == "$crawler-older-2"));
}

fn session_for(client: &matrix_sdk::Client, server: &MatrixMockServer) -> Arc<MatrixClientSession> {
    let session_info = SessionInfo {
        homeserver: server.server().uri(),
        user_id: client.user_id().expect("mock client user id").to_string(),
        device_id: client
            .device_id()
            .expect("mock client device id")
            .to_string(),
        authentication_method: SessionAuthenticationMethod::Unknown,
    };
    Arc::new(MatrixClientSession::from_client_for_testing(
        client.clone(),
        session_info,
    ))
}

fn upserted_ids(messages: &[SearchIndexMessage]) -> Vec<&str> {
    messages
        .iter()
        .filter_map(|message| match message {
            SearchIndexMessage::Upsert { event_id, .. } => Some(event_id.as_str()),
            _ => None,
        })
        .collect()
}

/// #996: a completed room whose cache still holds the recorded boundary is
/// caught up from the cache alone: only newer events, no `/messages` page.
#[tokio::test]
async fn catch_up_page_indexes_only_events_newer_than_the_boundary_from_the_cache() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!catchup-cache:example.invalid");
    let f = EventFactory::new()
        .room(room_id)
        .sender(user_id!("@catchup:test.invalid"));
    client
        .event_cache()
        .subscribe()
        .expect("event cache subscription");
    server
        .mock_sync()
        .ok_and_run(&client, |builder| {
            builder.add_joined_room(
                JoinedRoomBuilder::new(room_id)
                    .add_timeline_event(f.text_msg("old").event_id(event_id!("$catchup-old")))
                    .add_timeline_event(
                        f.text_msg("boundary")
                            .event_id(event_id!("$catchup-boundary")),
                    )
                    .add_timeline_event(f.text_msg("new").event_id(event_id!("$catchup-new"))),
            );
        })
        .await;
    // No `/messages` mock is mounted: a page request would fail the crawl.
    let checkpoint = HistoryCrawlCheckpoint::catch_up(
        room_id.to_string(),
        SearchCrawlerSettings::default(),
        1,
        "$catchup-boundary".to_owned(),
        10,
        7,
    );

    let result = run_history_crawl_page(
        session_for(&client, &server),
        AccountWorkScheduler::default(),
        checkpoint,
    )
    .await;
    let HistoryCrawlPageResult::Success {
        checkpoint,
        messages,
        completed,
    } = result
    else {
        panic!("catch-up should complete from the cache");
    };
    assert!(completed, "the boundary was in the cache");
    assert_eq!(upserted_ids(&messages), ["$catchup-new"]);
    assert_eq!((checkpoint.processed, checkpoint.indexed), (11, 8));
}

/// #996: after a limited sync the cache holds only the newest event behind a
/// gap; the catch-up pages backwards and stops at the page holding the
/// boundary instead of walking the whole room.
#[tokio::test]
async fn catch_up_page_resolves_a_limited_sync_gap_and_stops_at_the_boundary() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!catchup-gap:example.invalid");
    let f = EventFactory::new()
        .room(room_id)
        .sender(user_id!("@catchup:test.invalid"));
    client
        .event_cache()
        .subscribe()
        .expect("event cache subscription");
    server
        .mock_sync()
        .ok_and_run(&client, |builder| {
            builder.add_joined_room(
                JoinedRoomBuilder::new(room_id).add_timeline_event(
                    f.text_msg("boundary").event_id(event_id!("$gap-boundary")),
                ),
            );
        })
        .await;
    server
        .mock_sync()
        .ok_and_run(&client, |builder| {
            builder.add_joined_room(
                JoinedRoomBuilder::new(room_id)
                    .set_timeline_limited()
                    .set_timeline_prev_batch("gap-token")
                    .add_timeline_event(f.text_msg("newest").event_id(event_id!("$gap-newest"))),
            );
        })
        .await;
    server
        .mock_room_messages()
        .match_from("gap-token")
        .ok(RoomMessagesResponseTemplate::default()
            .events(vec![
                f.text_msg("middle").event_id(event_id!("$gap-middle")),
                f.text_msg("boundary").event_id(event_id!("$gap-boundary")),
            ])
            .end_token("gap-older"))
        .mock_once()
        .mount()
        .await;

    let checkpoint = HistoryCrawlCheckpoint::catch_up(
        room_id.to_string(),
        SearchCrawlerSettings::default(),
        1,
        "$gap-boundary".to_owned(),
        0,
        0,
    );
    let result = run_history_crawl_page(
        session_for(&client, &server),
        AccountWorkScheduler::default(),
        checkpoint,
    )
    .await;
    let HistoryCrawlPageResult::Success {
        messages,
        completed,
        ..
    } = result
    else {
        panic!("catch-up should page through the gap");
    };
    assert!(completed, "the page reached the boundary");
    let mut ids = upserted_ids(&messages);
    ids.sort_unstable();
    assert_eq!(
        ids,
        ["$gap-middle", "$gap-newest"],
        "boundary is not re-indexed"
    );
}

/// A page preempted by user-visible pagination keeps its first-page work, so
/// the re-run still indexes cached events and honours the catch-up boundary.
#[tokio::test]
async fn preempted_first_page_keeps_its_first_page_work() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!catchup-preempt:example.invalid");
    let f = EventFactory::new()
        .room(room_id)
        .sender(user_id!("@catchup:test.invalid"));
    client
        .event_cache()
        .subscribe()
        .expect("event cache subscription");
    server
        .mock_sync()
        .ok_and_run(&client, |builder| {
            builder.add_joined_room(
                JoinedRoomBuilder::new(room_id)
                    .set_timeline_limited()
                    .set_timeline_prev_batch("slow-token")
                    .add_timeline_event(f.text_msg("newest").event_id(event_id!("$slow-newest"))),
            );
        })
        .await;
    server
        .mock_room_messages()
        .match_from("slow-token")
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_secs(10))
                .set_body_json(serde_json::json!({ "start": "slow-token", "chunk": [] })),
        )
        .mount()
        .await;

    let scheduler = AccountWorkScheduler::default();
    let checkpoint = HistoryCrawlCheckpoint::catch_up(
        room_id.to_string(),
        SearchCrawlerSettings::default(),
        1,
        "$slow-boundary".to_owned(),
        0,
        0,
    );
    let page = tokio::spawn(run_history_crawl_page(
        session_for(&client, &server),
        scheduler.clone(),
        checkpoint,
    ));
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let waiting_pagination = {
        let scheduler = scheduler.clone();
        tokio::spawn(async move {
            let _permit = scheduler.acquire(AccountWorkKind::ExplicitPagination).await;
        })
    };

    let result = tokio::time::timeout(std::time::Duration::from_secs(5), page)
        .await
        .expect("preemption must end the page promptly")
        .expect("page task");
    let HistoryCrawlPageResult::Preempted { checkpoint } = result else {
        panic!("a waiting timeline pagination should preempt the crawler page");
    };
    assert!(
        checkpoint.first_page,
        "preemption must not consume the first page"
    );
    assert_eq!(checkpoint.catch_up_after.as_deref(), Some("$slow-boundary"));
    waiting_pagination.await.expect("pagination admitted");
}

#[test]
fn crawler_indexes_text_message_without_attachment_bytes() {
    let json = r#"{
            "event_id": "$e1:test",
            "sender": "@alice:test",
            "origin_server_ts": 1000,
            "type": "m.room.message",
            "content": {
                "msgtype": "m.text",
                "body": "hello historical world"
            }
        }"#;
    let settings = SearchCrawlerSettings {
        speed: SearchCrawlerSpeed::Standard,
        include_media_captions: true,
        include_filenames: true,
    };
    let mut pending = HashSet::new();
    let message = event_json_to_index_message("!r:test", json, &settings, &mut pending).unwrap();
    match message {
        SearchIndexMessage::Upsert {
            room_id,
            event_id,
            sender,
            body,
            attachment,
            ..
        } => {
            assert_eq!(room_id, "!r:test");
            assert_eq!(event_id, "$e1:test");
            assert_eq!(sender, "@alice:test");
            assert_eq!(body.as_deref(), Some("hello historical world"));
            assert!(attachment.is_none());
        }
        other => panic!("expected Upsert, got {other:?}"),
    }
}

#[test]
fn crawler_indexes_image_metadata_and_filename_not_bytes() {
    let json = r#"{
            "event_id": "$e2:test",
            "sender": "@bob:test",
            "origin_server_ts": 2000,
            "type": "m.room.message",
            "content": {
                "msgtype": "m.image",
                "body": "sunset.png",
                "url": "mxc://example/a",
                "info": {
                    "mimetype": "image/png",
                    "size": 12345,
                    "w": 800,
                    "h": 600,
                    "thumbnail_url": "mxc://example/t"
                }
            }
        }"#;
    let settings = SearchCrawlerSettings::default();
    let mut pending = HashSet::new();
    let message = event_json_to_index_message("!r:test", json, &settings, &mut pending).unwrap();
    match message {
        SearchIndexMessage::Upsert {
            body,
            attachment_filename,
            attachment,
            ..
        } => {
            assert_eq!(body.as_deref(), Some("sunset.png"));
            assert_eq!(attachment_filename.as_deref(), Some("sunset.png"));
            let attachment = attachment.expect("attachment metadata should be indexed");
            assert_eq!(attachment.source_mxc, "mxc://example/a");
            assert_eq!(attachment.thumbnail_mxc.as_deref(), Some("mxc://example/t"));
            assert_eq!(attachment.mimetype.as_deref(), Some("image/png"));
            assert_eq!(attachment.size, Some(12345));
            assert_eq!(attachment.width, Some(800));
            assert_eq!(attachment.height, Some(600));
        }
        other => panic!("expected Upsert, got {other:?}"),
    }
}

#[test]
fn crawler_does_not_index_edit_wrapper_as_standalone_message() {
    let json = r#"{
            "event_id": "$edit:test",
            "sender": "@alice:test",
            "origin_server_ts": 3000,
            "type": "m.room.message",
            "content": {
                "msgtype": "m.text",
                "body": "* wrapper body",
                "m.new_content": {
                    "msgtype": "m.text",
                    "body": "edited body"
                },
                "m.relates_to": {
                    "rel_type": "m.replace",
                    "event_id": "$e1:test"
                }
            }
        }"#;
    let settings = SearchCrawlerSettings::default();
    let mut pending = HashSet::new();
    let message = event_json_to_index_message("!r:test", json, &settings, &mut pending).unwrap();
    assert!(
        matches!(message, SearchIndexMessage::Edit { .. }),
        "edit wrapper must become a target mutation, not an Upsert"
    );
}

#[test]
fn crawler_indexes_edit_events_as_replacement_mutations() {
    let json = r#"{
            "event_id": "$edit:test",
            "sender": "@alice:test",
            "origin_server_ts": 3000,
            "type": "m.room.message",
            "content": {
                "msgtype": "m.text",
                "body": "* wrapper body must not be indexed",
                "m.new_content": {
                    "msgtype": "m.text",
                    "body": "edited historical body"
                },
                "m.relates_to": {
                    "rel_type": "m.replace",
                    "event_id": "$e1:test"
                }
            }
        }"#;
    let settings = SearchCrawlerSettings::default();
    let mut pending = HashSet::new();
    let message = event_json_to_index_message("!r:test", json, &settings, &mut pending).unwrap();

    match message {
        SearchIndexMessage::Edit {
            edit_event_id,
            target_event_id,
            sender,
            body,
            ..
        } => {
            assert_eq!(edit_event_id, "$edit:test");
            assert_eq!(target_event_id, "$e1:test");
            assert_eq!(sender, "@alice:test");
            assert_eq!(body.as_deref(), Some("edited historical body"));
        }
        other => panic!("expected Edit, got {other:?}"),
    }
}

#[test]
fn crawler_respects_include_media_captions_setting() {
    let json = r#"{
            "event_id": "$e3:test",
            "sender": "@bob:test",
            "origin_server_ts": 4000,
            "type": "m.room.message",
            "content": {
                "msgtype": "m.image",
                "body": "image.png",
                "url": "mxc://example/b",
                "info": { "mimetype": "image/png" }
            }
        }"#;
    let mut settings = SearchCrawlerSettings::default();
    settings.include_media_captions = false;
    settings.include_filenames = true;
    let mut pending = HashSet::new();
    let message = event_json_to_index_message("!r:test", json, &settings, &mut pending).unwrap();
    match message {
        SearchIndexMessage::Upsert {
            body,
            attachment_filename,
            ..
        } => {
            assert!(body.is_none());
            assert_eq!(attachment_filename.as_deref(), Some("image.png"));
        }
        other => panic!("expected Upsert, got {other:?}"),
    }
}

#[test]
fn crawler_respects_include_filenames_setting() {
    let json = r#"{
            "event_id": "$e4:test",
            "sender": "@bob:test",
            "origin_server_ts": 5000,
            "type": "m.room.message",
            "content": {
                "msgtype": "m.image",
                "body": "image.png",
                "url": "mxc://example/c",
                "info": { "mimetype": "image/png" }
            }
        }"#;
    let mut settings = SearchCrawlerSettings::default();
    settings.include_media_captions = true;
    settings.include_filenames = false;
    let mut pending = HashSet::new();
    let message = event_json_to_index_message("!r:test", json, &settings, &mut pending).unwrap();
    match message {
        SearchIndexMessage::Upsert {
            body,
            attachment_filename,
            attachment,
            ..
        } => {
            assert_eq!(body.as_deref(), Some("image.png"));
            assert!(attachment_filename.is_none());
            assert!(attachment.is_none());
        }
        other => panic!("expected Upsert, got {other:?}"),
    }
}

#[test]
fn crawler_redaction_targets_redacts_field_not_event_id() {
    // A backward crawl sees the redaction first (newer), then the original
    // (older).  The redaction must remove the TARGET event id, not itself,
    // and must record the target in `pending_redactions` so a subsequent
    // Upsert for the original is suppressed.
    let redaction_json = r#"{
            "event_id": "$redact:test",
            "sender": "@alice:test",
            "origin_server_ts": 9000,
            "type": "m.room.redaction",
            "redacts": "$original:test"
        }"#;
    let settings = SearchCrawlerSettings::default();
    let mut pending = HashSet::new();
    let msg =
        event_json_to_index_message("!r:test", redaction_json, &settings, &mut pending).unwrap();
    // Must Redact the TARGET, not the redaction event itself.
    match msg {
        SearchIndexMessage::Redact { event_id } => {
            assert_eq!(
                event_id, "$original:test",
                "Redact must target the original event, not the redaction event"
            );
        }
        other => panic!("expected Redact, got {other:?}"),
    }
    // The target must be in pending_redactions so a later Upsert is skipped.
    assert!(
        pending.contains("$original:test"),
        "target should be in pending_redactions set"
    );
    assert!(
        !pending.contains("$redact:test"),
        "redaction event id itself must not be in pending_redactions"
    );
}

#[test]
fn crawler_redaction_via_content_field() {
    // MSC2174: some servers nest `redacts` inside `content`.
    let redaction_json = r#"{
            "event_id": "$redact2:test",
            "sender": "@alice:test",
            "origin_server_ts": 9001,
            "type": "m.room.redaction",
            "content": {
                "redacts": "$original2:test",
                "reason": "spam"
            }
        }"#;
    let settings = SearchCrawlerSettings::default();
    let mut pending = HashSet::new();
    let msg =
        event_json_to_index_message("!r:test", redaction_json, &settings, &mut pending).unwrap();
    match msg {
        SearchIndexMessage::Redact { event_id } => {
            assert_eq!(event_id, "$original2:test");
        }
        other => panic!("expected Redact, got {other:?}"),
    }
    assert!(pending.contains("$original2:test"));
}

// -----------------------------------------------------------------------
// P1-B: Shutdown drain — channel backpressure must not cause deadlock
// -----------------------------------------------------------------------

/// Verifies the drain-while-await pattern used in `SearchActor::run`'s
/// Shutdown arm.  A task that is blocked on `channel.send().await` must
/// be able to complete after the receiver resumes draining, and the whole
/// sequence must finish within a bounded time (no deadlock).
///
/// This is a pure channel-level test; it does not require the full
/// SDK/actor infrastructure and runs without a network connection.
#[tokio::test]
async fn shutdown_drain_completes_within_bounded_time_when_channel_was_full() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::mpsc;

    // Capacity 2: the task can queue 2 messages without blocking, but
    // the 3rd send will block until the receiver drains one slot.
    let (tx, mut rx) = mpsc::channel::<u32>(2);
    let done = Arc::new(AtomicBool::new(false));
    let done_clone = done.clone();

    let tx_clone = tx.clone();
    let task = tokio::spawn(async move {
        // Fill to capacity without blocking.
        tx_clone.send(1).await.ok();
        tx_clone.send(2).await.ok();
        // This send blocks until the receiver drains at least one slot.
        let _ = tx_clone.send(3).await;
        done_clone.store(true, Ordering::Relaxed);
    });
    tokio::pin!(task);

    // Simulate the actor's Shutdown drain loop: drain the receiver while
    // awaiting the task handle.  Without draining, the task would be
    // stuck on the blocked send forever.
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            tokio::select! {
                biased;
                _ = &mut task => break,
                _ = rx.recv() => {}
            }
        }
    })
    .await;

    assert!(
        result.is_ok(),
        "shutdown drain must complete within 5 s — timed out (deadlock regression)"
    );
    assert!(
        done.load(Ordering::Relaxed),
        "task must have signalled completion after drain unblocked it"
    );
}
