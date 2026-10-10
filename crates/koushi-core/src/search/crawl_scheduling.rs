//! Actor-scheduling contract for the automatic history-crawl pump (#1276).
//!
//! These tests drive the real `SearchActor::run` loop through its channels, so
//! the `tokio::select!` arm ordering is exercised rather than approximated by
//! calling actor methods in a hand-picked order. The crawl room is listed in the
//! room-availability notification but is unknown to the SDK client, so a crawl
//! page that starts fails fast with `RoomNotFound`; that failure action is the
//! observable proof that the pump moved.

use std::time::Duration;

use super::*;
use koushi_state::{AttachmentKind, SessionAuthenticationMethod, SessionInfo};
use matrix_sdk::test_utils::mocks::MatrixMockServer;

/// Automatic crawl target. Deliberately absent from the SDK client.
const CRAWL_ROOM: &str = "!crawl:example.invalid";
/// A second room the client also does not know; index work for it cannot be
/// proven against the event cache, so it stays a body-free retry.
const UNPROVEN_ROOM: &str = "!media:example.invalid";
const UNPROVEN_EVENT: &str = "$media:example.invalid";

fn active_crawler_settings() -> SearchCrawlerSettings {
    SearchCrawlerSettings {
        speed: SearchCrawlerSpeed::Standard,
        include_media_captions: true,
        include_filenames: true,
    }
}

fn crawl_notification() -> CrawlerRoomsNotification {
    CrawlerRoomsNotification {
        room_ids: vec![CRAWL_ROOM.to_owned()],
        latest_event_ids: Default::default(),
        settings: active_crawler_settings(),
    }
}

/// Attachment metadata for a room the SDK client cannot resolve, which makes
/// the actor's mutation proof fail and retain the message body-free.
fn unprovable_attachment_message() -> SearchIndexMessage {
    SearchIndexMessage::Upsert {
        room_id: UNPROVEN_ROOM.to_owned(),
        event_id: UNPROVEN_EVENT.to_owned(),
        sender: "@member:example.invalid".to_owned(),
        timestamp_ms: 1,
        body: Some("synthetic caption".to_owned()),
        attachment_filename: Some("synthetic.bin".to_owned()),
        attachment: Some(AttachmentDocument {
            kind: AttachmentKind::File,
            msgtype: "m.file".into(),
            mimetype: None,
            size: None,
            source_mxc: "mxc://example.invalid/source".into(),
            thumbnail_mxc: None,
            filename: SensitiveString::new("synthetic.bin"),
            thread_root: None,
            encrypted: false,
            encryption_version: None,
            width: None,
            height: None,
            is_edited: false,
        }),
        canonical: true,
        edit: None,
    }
}

/// The test's watch window must outlast the product's startup hold: under
/// paused time the runtime advances to the earliest deadline, so a shorter
/// window would expire before the hold the test is waiting out.
fn crawl_watch_window() -> Duration {
    CRAWLER_STARTUP_DELAY + Duration::from_secs(5)
}

fn crawl_page_ran(actions: &[AppAction]) -> bool {
    actions
        .iter()
        .any(|action| matches!(action, AppAction::HistoryCrawlFailed { .. }))
}

async fn spawn_fixture() -> (
    MatrixMockServer,
    SearchActorHandle,
    mpsc::Receiver<Vec<AppAction>>,
) {
    let (server, session) = session_fixture().await;
    let (action_tx, action_rx) = mpsc::channel(64);
    let (event_tx, _) = broadcast::channel(32);
    let handle = SearchActor::spawn(
        session,
        action_tx,
        event_tx,
        AccountWorkScheduler::default(),
    );
    (server, handle, action_rx)
}

async fn session_fixture() -> (MatrixMockServer, Arc<MatrixClientSession>) {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    client.event_cache().subscribe().unwrap();
    let session = MatrixClientSession::from_client_for_testing(
        client.clone(),
        SessionInfo {
            homeserver: server.server().uri(),
            user_id: client.user_id().unwrap().to_string(),
            device_id: client.device_id().unwrap().to_string(),
            authentication_method: SessionAuthenticationMethod::Unknown,
        },
    );
    (server, Arc::new(session))
}

/// The pump serializes the crawler's own work: a completed page whose body-free
/// index queue has not been applied still holds the next page, while an
/// unresolved Files-admission retry must not starve the crawl (#1276).
#[tokio::test(start_paused = true)]
async fn a_pending_page_index_queue_holds_the_next_page_but_a_retry_does_not() {
    let (_server, session) = session_fixture().await;
    let (action_tx, _action_rx) = mpsc::channel(64);
    let (event_tx, _) = broadcast::channel(32);
    let (_, msg_rx) = mpsc::channel(32);
    let mut actor = SearchActor::new(
        session,
        action_tx,
        event_tx,
        msg_rx,
        AccountWorkScheduler::default(),
    );
    actor.crawler_settings = active_crawler_settings();
    actor.available_crawl_rooms.insert(CRAWL_ROOM.to_owned());
    actor.crawl_delay_elapsed = true;
    actor
        .enqueue_history_crawl(
            crate::search_crawler::HistoryCrawlCheckpoint::new(
                CRAWL_ROOM.to_owned(),
                active_crawler_settings(),
                0,
                false,
            ),
            0,
        )
        .await;

    // A completed page's body-free index queue still holds the next page.
    actor.queue_crawl_messages(vec![unprovable_attachment_message()]);
    assert!(!actor.queued_crawl_index.is_empty());
    actor.start_next_history_crawl_page();
    assert!(
        actor.active_crawl_page.is_none(),
        "the pending page index queue must hold the next page"
    );

    // Once that queue is applied, an unresolved Files-admission retry alone must
    // not keep the crawler from starting.
    actor.queued_crawl_index.clear();
    actor
        .attachment_retries
        .push_back(unprovable_attachment_message());
    actor.start_next_history_crawl_page();
    assert!(
        actor.active_crawl_page.is_some(),
        "an unresolved Files-admission retry must not starve the crawl"
    );
}

#[tokio::test(start_paused = true)]
async fn automatic_crawl_starts_without_pending_index_work() {
    let (_server, handle, mut action_rx) = spawn_fixture().await;
    handle
        .try_notify_rooms_available(crawl_notification())
        .expect("first room-availability notification is accepted");
    let started = tokio::time::timeout(crawl_watch_window(), async {
        while let Some(actions) = action_rx.recv().await {
            if crawl_page_ran(&actions) {
                return true;
            }
        }
        false
    })
    .await;
    assert_eq!(started, Ok(true), "quiet-session crawl never started");
}

/// The nightly aggregate stall (#1276): an automatic crawl is queued while the
/// startup hold is running, the hold elapses while index work leaves an
/// unprovable attachment retry pending, and the crawl must still start once the
/// index lane goes quiet.
#[tokio::test(start_paused = true)]
async fn automatic_crawl_starts_after_index_work_goes_quiet() {
    let (_server, handle, mut action_rx) = spawn_fixture().await;

    // (a) Automatic crawl work is queued while the startup hold has not elapsed.
    handle
        .try_notify_rooms_available(crawl_notification())
        .expect("first room-availability notification is accepted");
    // (b) Index work that cannot be proven leaves a body-free attachment retry.
    handle
        .index_sender()
        .send(unprovable_attachment_message())
        .await
        .expect("index channel accepts one message");

    // (c) The startup hold elapses and (d) the crawl page must start. The page
    // fails fast because the crawl room is unknown to the client, so receiving
    // that failure proves the pump ran.
    let started = tokio::time::timeout(crawl_watch_window(), async {
        while let Some(actions) = action_rx.recv().await {
            if crawl_page_ran(&actions) {
                return true;
            }
        }
        false
    })
    .await;

    assert_eq!(
        started,
        Ok(true),
        "the automatic crawl never started after index work went quiet"
    );
}
