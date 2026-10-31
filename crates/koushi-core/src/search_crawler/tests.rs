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
    trace_crawler_page(
        DiagnosticLevel::Debug,
        "test_progress",
        CrawlHistorySource::Store,
        12,
        9,
        4,
    );
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
    let _session = Arc::new(MatrixClientSession::from_client_for_testing(
        client.clone(),
        session_info,
    ));
    let checkpoint = HistoryCrawlCheckpoint::new(
        room_id.to_string(),
        SearchCrawlerSettings::default(),
        1,
        true,
    );

    let (checkpoint, messages, _) = drain_crawl_pages(
        session_for(&client, &server),
        AccountWorkScheduler::default(),
        checkpoint,
        2,
    )
    .await;
    assert_eq!(
        checkpoint.history_source,
        CrawlHistorySource::Network,
        "the persisted walk found the gap and handed the rest to pagination"
    );
    // #996: the newest event arrived by sync and was already cached before the
    // crawl, so the first page indexes the cached events too. The persisted walk
    // covers the cached window and finds the gap behind it, which is why this
    // room continues through SDK back-pagination.
    let indexed_ids = upserted_ids(&messages);
    assert_eq!(
        indexed_ids.len(),
        3,
        "crawler should index the cached latest event and both historical events: {indexed_ids:?}"
    );
    assert!(indexed_ids.contains(&"$crawler-latest"));
    assert!(indexed_ids.contains(&"$crawler-older-1"));

    // History the store did not hold is fetched through pagination, which is the
    // one path that still inserts into the shared linked chunk.
    let room = client.get_room(room_id).expect("joined room");
    let (cache, _drop_handles) = room
        .client()
        .event_cache()
        .unwrap()
        .room(room.room_id())
        .await
        .expect("room event cache");
    let visible = cache.events().await.expect("cached timeline events");
    let visible_ids = visible
        .iter()
        .filter_map(|event| event.event_id().map(ToString::to_string))
        .collect::<Vec<_>>();
    assert!(visible_ids.iter().any(|id| id == "$crawler-older-1"));
    assert!(visible_ids.iter().any(|id| id == "$crawler-older-2"));
}

/// A room the local store already holds completely is crawled from persisted
/// chunks alone: no `/messages` request, and no growth of the resident room
/// timeline.
///
/// This is the property that keeps indexing from hydrating history. The room is
/// backfilled once through pagination, and the next crawl must complete from the
/// stored chunks without touching the network.
#[tokio::test]
async fn crawler_page_reads_persisted_history_without_hydrating_the_room_cache() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!store-only:example.invalid");
    let f = EventFactory::new()
        .room(room_id)
        .sender(user_id!("@crawler:test.invalid"));
    server
        .mock_sync()
        .ok_and_run(&client, |builder| {
            builder.add_joined_room(
                JoinedRoomBuilder::new(room_id)
                    .set_timeline_limited()
                    .set_timeline_prev_batch("store-prev")
                    .add_timeline_event(f.text_msg("newest").event_id(event_id!("$store-newest"))),
            );
        })
        .await;
    // Serves the one backfill page that closes the local gap. `mock_once` means a
    // crawl that asks for history a second time fails instead of completing.
    server
        .mock_room_messages()
        .match_from("store-prev")
        .ok(RoomMessagesResponseTemplate::default().events(vec![
            f.text_msg("older").event_id(event_id!("$store-older")),
        ]))
        .mock_once()
        .mount()
        .await;

    let session = session_for(&client, &server);
    let first = HistoryCrawlCheckpoint::new(
        room_id.to_string(),
        SearchCrawlerSettings::default(),
        1,
        true,
    );
    // A persisted page, then the paginated page that closes the gap.
    let (.., first_messages, _) =
        drain_crawl_pages(session.clone(), AccountWorkScheduler::default(), first, 2).await;
    let mut first_ids = upserted_ids(&first_messages);
    first_ids.sort_unstable();
    assert_eq!(first_ids, ["$store-newest", "$store-older"]);

    let room = client.get_room(room_id).expect("joined room");
    let (cache, _drop_handles) = room
        .client()
        .event_cache()
        .unwrap()
        .room(room.room_id())
        .await
        .expect("room event cache");
    let resident_before = client.event_cache().unwrap().memory_diagnostics().await;
    let visible_before = cache.events().await.expect("cached timeline events").len();

    let second = HistoryCrawlCheckpoint::new(
        room_id.to_string(),
        SearchCrawlerSettings::default(),
        1,
        true,
    );
    let (checkpoint, second_messages, second_completed) =
        drain_crawl_pages(session, AccountWorkScheduler::default(), second, 1).await;
    assert!(
        second_completed,
        "the stored chunks now hold the whole history, so the walk reaches its start"
    );
    assert_eq!(
        checkpoint.history_source,
        CrawlHistorySource::Store,
        "a complete local store must not need the network"
    );
    let mut second_ids = upserted_ids(&second_messages);
    second_ids.sort_unstable();
    assert_eq!(
        second_ids,
        ["$store-newest", "$store-older"],
        "the persisted walk covers the same history"
    );

    let resident_after = client.event_cache().unwrap().memory_diagnostics().await;
    let visible_after = cache.events().await.expect("cached timeline events").len();
    assert_eq!(
        resident_after.events, resident_before.events,
        "indexing must not add resident history to the room timeline"
    );
    assert_eq!(
        visible_after, visible_before,
        "indexing must not grow the resident room window"
    );
}

/// Stage A acceptance: the decided history is on disk but outside the resident
/// window, and indexing it adds nothing back to that window.
///
/// A crawl that also hydrated the chunks it read would pass the earlier
/// store-only test, because that fixture was small enough to stay resident. This
/// one spans several linked chunks (the SDK chunk capacity is 128), drops the
/// older chunks from the resident window through the public subscriber-release
/// path, and then requires the crawl to reach every stored document without the
/// network and without growing the resident counters.
#[tokio::test]
async fn crawler_page_indexes_history_that_is_not_resident() {
    const FETCHED_EVENTS: usize = 300;

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!nonresident:example.invalid");
    let f = EventFactory::new()
        .room(room_id)
        .sender(user_id!("@crawler:test.invalid"));
    let fetched = (0..FETCHED_EVENTS)
        .map(|index| {
            let id = matrix_sdk::ruma::OwnedEventId::try_from(format!(
                "$nonresident{index}:example.invalid"
            ))
            .expect("event id");
            f.text_msg(format!("history body {index}")).event_id(&id)
        })
        .collect::<Vec<_>>();
    server
        .mock_sync()
        .ok_and_run(&client, |builder| {
            builder.add_joined_room(
                JoinedRoomBuilder::new(room_id)
                    .set_timeline_limited()
                    .set_timeline_prev_batch("nonresident-prev")
                    .add_timeline_event(
                        f.text_msg("newest")
                            .event_id(event_id!("$nonresident-newest")),
                    ),
            );
        })
        .await;
    // `mock_once`: the second crawl must reach every document from the store, so
    // a second `/messages` request would fail it instead of completing.
    server
        .mock_room_messages()
        .match_from("nonresident-prev")
        .ok(RoomMessagesResponseTemplate::default().events(fetched))
        .mock_once()
        .mount()
        .await;

    let session = session_for(&client, &server);
    let first = HistoryCrawlCheckpoint::new(
        room_id.to_string(),
        SearchCrawlerSettings::default(),
        1,
        true,
    );
    let (.., first_messages, first_completed) =
        drain_crawl_pages(session.clone(), AccountWorkScheduler::default(), first, 4).await;
    assert!(
        first_completed,
        "the backfill crawl should reach the start of the room"
    );
    assert_eq!(
        upserted_ids(&first_messages).len(),
        FETCHED_EVENTS + 1,
        "the backfill indexes the persisted window and the paginated history"
    );

    // Release the room's resident window through the public subscriber path. The
    // store keeps every chunk; only the in-memory window shrinks to the newest
    // chunk, which is what makes the second crawl's evidence meaningful.
    let room = client.get_room(room_id).expect("joined room");
    let (cache, _drop_handles) = room
        .client()
        .event_cache()
        .unwrap()
        .room(room.room_id())
        .await
        .expect("room event cache");
    let (_events, subscriber) = cache.subscribe().await.expect("room subscription");
    drop(subscriber);
    let mut resident_after_release = client.event_cache().unwrap().memory_diagnostics().await;
    for _ in 0..100 {
        if resident_after_release.events < (FETCHED_EVENTS + 1) as u64 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        resident_after_release = client.event_cache().unwrap().memory_diagnostics().await;
    }
    assert!(
        resident_after_release.events < (FETCHED_EVENTS + 1) as u64,
        "the resident window should have shrunk below the stored history: {resident_after_release:?}"
    );
    let visible_before = cache.events().await.expect("cached timeline events").len();

    let second = HistoryCrawlCheckpoint::new(
        room_id.to_string(),
        SearchCrawlerSettings::default(),
        1,
        true,
    );
    let (checkpoint, second_messages, second_completed) =
        drain_crawl_pages(session, AccountWorkScheduler::default(), second, 4).await;
    assert!(
        second_completed,
        "the stored chunks hold the whole history, so the walk reaches its start"
    );
    assert_eq!(
        checkpoint.history_source,
        CrawlHistorySource::Store,
        "a complete local store must not need the network"
    );
    assert_eq!(
        upserted_ids(&second_messages).len(),
        FETCHED_EVENTS + 1,
        "every stored document is reachable from the persisted walk"
    );

    let resident_after = client.event_cache().unwrap().memory_diagnostics().await;
    let visible_after = cache.events().await.expect("cached timeline events").len();
    assert_eq!(
        resident_after.events, resident_after_release.events,
        "indexing must not add resident history to the room timeline"
    );
    assert_eq!(
        visible_after, visible_before,
        "indexing must not grow the resident room window"
    );
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

/// Drive one crawl for at most `max_pages` pages, accumulating what it indexed.
///
/// A room whose persisted history has a gap is crawled in two steps now: the
/// first page walks the persisted chunks, and the rest continues through SDK
/// back-pagination, which is the path that reaches `/messages`.
async fn drain_crawl_pages(
    session: Arc<MatrixClientSession>,
    scheduler: AccountWorkScheduler,
    checkpoint: HistoryCrawlCheckpoint,
    max_pages: usize,
) -> (HistoryCrawlCheckpoint, Vec<SearchIndexMessage>, bool) {
    let mut messages = Vec::new();
    let mut current = checkpoint;
    for _ in 0..max_pages {
        let result = run_history_crawl_page(session.clone(), scheduler.clone(), current).await;
        match result {
            HistoryCrawlPageResult::Success {
                checkpoint: next,
                messages: page_messages,
                completed,
                ..
            } => {
                messages.extend(page_messages);
                current = next;
                if completed {
                    return (current, messages, true);
                }
            }
            HistoryCrawlPageResult::Failed { checkpoint, .. } => {
                current = checkpoint;
                break;
            }
            HistoryCrawlPageResult::Preempted { checkpoint } => {
                current = checkpoint;
                break;
            }
        }
    }
    (current, messages, false)
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
        ..
    } = result
    else {
        panic!("catch-up should complete from the cache");
    };
    assert!(completed, "the boundary was in the cache");
    assert_eq!(upserted_ids(&messages), ["$catchup-new"]);
    assert_eq!((checkpoint.processed, checkpoint.indexed), (11, 8));
}

/// #996: a boundary the persisted walk only reaches across a gap is not proof of
/// catch-up. The events inside that gap were never delivered locally, so the walk
/// must not acknowledge completion; the crawl repairs the gap through
/// back-pagination and only then stops at the boundary.
#[tokio::test]
async fn catch_up_page_repairs_a_gap_before_accepting_the_boundary() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!catchup-gap:example.invalid");
    let f = EventFactory::new()
        .room(room_id)
        .sender(user_id!("@catchup:test.invalid"));
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
    let (checkpoint, messages, completed) = drain_crawl_pages(
        session_for(&client, &server),
        AccountWorkScheduler::default(),
        checkpoint,
        2,
    )
    .await;
    assert_eq!(
        checkpoint.history_source,
        CrawlHistorySource::Network,
        "a boundary behind a gap must hand the rest to back-pagination"
    );
    assert!(completed, "the fetched page holds the boundary");
    let mut ids = upserted_ids(&messages);
    ids.sort_unstable();
    assert_eq!(
        ids,
        ["$gap-middle", "$gap-newest"],
        "the gap's event is indexed and the boundary is not re-indexed"
    );
}

/// #996: when the boundary is older than anything the store holds, the catch-up
/// continues through back-pagination and stops on the page that holds it.
#[tokio::test]
async fn catch_up_page_continues_through_pagination_to_reach_the_boundary() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!catchup-network:example.invalid");
    let f = EventFactory::new()
        .room(room_id)
        .sender(user_id!("@catchup:test.invalid"));
    server
        .mock_sync()
        .ok_and_run(&client, |builder| {
            builder.add_joined_room(
                JoinedRoomBuilder::new(room_id)
                    .set_timeline_limited()
                    .set_timeline_prev_batch("net-token")
                    .add_timeline_event(f.text_msg("newest").event_id(event_id!("$net-newest"))),
            );
        })
        .await;
    server
        .mock_room_messages()
        .match_from("net-token")
        .ok(RoomMessagesResponseTemplate::default()
            .events(vec![
                f.text_msg("middle").event_id(event_id!("$net-middle")),
                f.text_msg("boundary").event_id(event_id!("$net-boundary")),
            ])
            .end_token("net-older"))
        .mock_once()
        .mount()
        .await;

    let checkpoint = HistoryCrawlCheckpoint::catch_up(
        room_id.to_string(),
        SearchCrawlerSettings::default(),
        1,
        "$net-boundary".to_owned(),
        0,
        0,
    );
    let (.., messages, completed) = drain_crawl_pages(
        session_for(&client, &server),
        AccountWorkScheduler::default(),
        checkpoint,
        2,
    )
    .await;
    assert!(
        completed,
        "the crawl reached the boundary in the fetched page"
    );
    let mut ids = upserted_ids(&messages);
    ids.sort_unstable();
    assert_eq!(
        ids,
        ["$net-middle", "$net-newest"],
        "boundary is not re-indexed"
    );
}

/// A page preempted by user-visible pagination keeps its checkpoint, so the
/// re-run resumes where it stopped and still honours the catch-up boundary.
#[tokio::test]
async fn preempted_page_keeps_its_checkpoint() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!catchup-preempt:example.invalid");
    let f = EventFactory::new()
        .room(room_id)
        .sender(user_id!("@catchup:test.invalid"));
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
    // The room has a gap, so the first page walks the persisted chunks and hands
    // the rest to back-pagination. Preemption is about a page that waits on the
    // shared gate, which is the paginated one.
    let HistoryCrawlPageResult::Success { checkpoint, .. } =
        run_history_crawl_page(session_for(&client, &server), scheduler.clone(), checkpoint).await
    else {
        panic!("the persisted page should succeed");
    };
    assert_eq!(checkpoint.history_source, CrawlHistorySource::Network);

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
    assert_eq!(
        checkpoint.history_source,
        CrawlHistorySource::Network,
        "a preempted page must not lose the source it had reached"
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
    let settings = SearchCrawlerSettings {
        include_media_captions: false,
        include_filenames: true,
        ..Default::default()
    };
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
    let settings = SearchCrawlerSettings {
        include_media_captions: true,
        include_filenames: false,
        ..Default::default()
    };
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
