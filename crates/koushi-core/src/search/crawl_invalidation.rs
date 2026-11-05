//! Crawl-invalidation coalescing (#1781).
//!
//! A lag report invalidates every available room, and a reopen used to discard
//! the room's progress, cancel the active page and publish a new enqueue, so one
//! signal cost the joined-room count (3,070 enqueues in 97 s on a fresh install).
//! The incident itself is in
//! `docs/worklogs/2026-11-05-issue1781-crawl-invalidation-coalescing.md`.
//!
//! These tests drive the real `SearchActor::run` mailbox, or one actor method
//! with hand-built state where a precise state is not reachable through a
//! message. Page progress is held deterministically by an interactive
//! account-work guard, and every wait is "progress or silence" on the actor's
//! own channels — no fixed sleep decides an outcome.

use std::sync::Arc;

use super::*;
use crate::account_work::AccountWorkKind;
use koushi_protocol::ids::{RequestId, RuntimeConnectionId};
use koushi_state::{SessionAuthenticationMethod, SessionInfo};
use matrix_sdk::test_utils::mocks::MatrixMockServer;
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};

const ACTIVE_ROOM: &str = "!invalidation-active:example.invalid";
const PARTIAL_ROOM: &str = "!invalidation-partial:example.invalid";
const FRESH_ROOM: &str = "!invalidation-fresh:example.invalid";
const SETTLED_ROOM: &str = "!invalidation-settled:example.invalid";
const SETTLED_LATEST: &str = "$invalidation-settled:example.invalid";
const RACE_ROOM: &str = "!invalidation-race:example.invalid";
const RESTART_ROOM: &str = "!invalidation-restart:example.invalid";

/// Bound on "progress or silence": a burst that publishes nothing must settle
/// as a timeout, never as a longer wait for an expected signal.
const SETTLE_WINDOW: Duration = Duration::from_secs(20);

fn active_settings() -> SearchCrawlerSettings {
    SearchCrawlerSettings {
        speed: SearchCrawlerSpeed::Standard,
        include_media_captions: true,
        include_filenames: true,
    }
}

fn paused_settings() -> SearchCrawlerSettings {
    SearchCrawlerSettings {
        speed: SearchCrawlerSpeed::Paused,
        ..active_settings()
    }
}

fn session_info(client: &matrix_sdk::Client, server: &MatrixMockServer) -> SessionInfo {
    SessionInfo {
        homeserver: server.server().uri(),
        user_id: client.user_id().expect("mock user id").to_string(),
        device_id: client.device_id().expect("mock device id").to_string(),
        authentication_method: SessionAuthenticationMethod::Unknown,
    }
}

fn notification(
    rooms: &[&str],
    latest: &[(&str, &str)],
    settings: &SearchCrawlerSettings,
) -> CrawlerRoomsNotification {
    CrawlerRoomsNotification {
        room_ids: rooms.iter().map(|room| (*room).to_owned()).collect(),
        latest_event_ids: latest
            .iter()
            .map(|(room, event)| ((*room).to_owned(), (*event).to_owned()))
            .collect(),
        settings: settings.clone(),
    }
}

fn settled_progress(settings: &SearchCrawlerSettings, rooms: &[(&str, &str)]) -> SearchProgress {
    settled_progress_for("instance", settings, rooms)
}

fn settled_progress_for(
    index_instance: &str,
    settings: &SearchCrawlerSettings,
    rooms: &[(&str, &str)],
) -> SearchProgress {
    let mut progress = SearchProgress::new(SearchProgressIdentity {
        index_contract_version: crate::store::SEARCH_INDEX_CONTRACT_VERSION,
        index_instance: index_instance.to_owned(),
        include_media_captions: settings.include_media_captions,
        include_filenames: settings.include_filenames,
    });
    for (room_id, latest) in rooms {
        let room = progress.room_mut(room_id);
        room.backfill_complete = true;
        room.latest_event_id = Some((*latest).to_owned());
        room.processed = 4;
        room.indexed = 4;
    }
    progress
}

fn queued_checkpoint(
    actor: &SearchActor,
    room_id: &str,
    read_history: bool,
) -> HistoryCrawlCheckpoint {
    let mut checkpoint = HistoryCrawlCheckpoint::new(
        room_id.to_owned(),
        active_settings(),
        actor.crawl_settings_generation,
        false,
    );
    if read_history {
        // A page of this crawl already ran: its position may be past a change
        // that arrives now.
        checkpoint.first_page = false;
        checkpoint.processed = 40;
        checkpoint.indexed = 39;
    }
    checkpoint
}

fn started_rooms(actions: &[AppAction]) -> Vec<String> {
    actions
        .iter()
        .filter_map(|action| match action {
            AppAction::HistoryCrawlStarted { room_id, .. } => Some(room_id.clone()),
            _ => None,
        })
        .collect()
}

fn completed_rooms(actions: &[AppAction]) -> Vec<String> {
    actions
        .iter()
        .filter_map(|action| match action {
            AppAction::HistoryCrawlCompleted { room_id, .. } => Some(room_id.clone()),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Run-loop harness
// ---------------------------------------------------------------------------

struct Seeded {
    settings: SearchCrawlerSettings,
    /// Rooms the mock client knows, so their crawl page reaches the shared work
    /// permit instead of failing on an unknown room.
    rooms: Vec<&'static str>,
    /// Rooms the durable record already committed, with their latest event.
    settled: Vec<(&'static str, &'static str)>,
    /// Rooms queued before the test starts, and whether their crawl already
    /// read history.
    queued: Vec<(&'static str, bool)>,
}

struct Harness {
    /// Kept alive for the session's lifetime; the mock server owns the client's
    /// connection state.
    _server: MatrixMockServer,
    _client: matrix_sdk::Client,
    handle: SearchActorHandle,
    actions: mpsc::Receiver<Vec<AppAction>>,
    scheduler: AccountWorkScheduler,
    barrier: u64,
}

async fn harness(seeded: Seeded) -> Harness {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    if !seeded.rooms.is_empty() {
        let rooms = seeded.rooms.clone();
        server
            .mock_sync()
            .ok_and_run(&client, |builder| {
                for room_id in rooms {
                    let parsed =
                        matrix_sdk::ruma::RoomId::parse(room_id).expect("synthetic room ids parse");
                    builder.add_joined_room(JoinedRoomBuilder::new(&parsed));
                }
            })
            .await;
    }
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client.clone(),
        session_info(&client, &server),
    ));
    // Large enough that the pre-fix amplification cannot deadlock the actor on
    // a full channel instead of failing an assertion.
    let (action_tx, actions) = mpsc::channel(16_384);
    let (event_tx, _events) = broadcast::channel(64);
    let (tx, msg_rx) = mpsc::channel(16_384);
    let (index_tx, index_rx) = mpsc::channel(SEARCH_INDEX_MUTATION_QUEUE);
    let scheduler = AccountWorkScheduler::default();
    let mut actor = SearchActor::new(session, action_tx, event_tx, msg_rx, scheduler.clone());
    // The product startup hold is measured elsewhere; these tests exercise the
    // pump, not the hold.
    actor.crawl_delay_elapsed = true;
    if !seeded.settled.is_empty() {
        let progress = settled_progress(&seeded.settings, &seeded.settled);
        actor.adopt_loaded_progress(progress, "instance".to_owned());
    }
    for (room_id, read_history) in &seeded.queued {
        let checkpoint = queued_checkpoint(&actor, room_id, *read_history);
        actor.queued_crawl_rooms.insert((*room_id).to_owned());
        actor.crawl_queue.push_back(checkpoint);
    }
    let task = executor::spawn(actor.run(index_rx));
    Harness {
        _server: server,
        _client: client,
        handle: SearchActorHandle {
            tx,
            index_tx,
            task: Some(task),
            report_task: None,
        },
        actions,
        scheduler,
        barrier: 0,
    }
}

impl Harness {
    async fn send(&self, message: SearchActorMessage) {
        self.handle
            .tx
            .send(message)
            .await
            .expect("the actor inbox accepts a message");
    }

    async fn notify(&self, notification: CrawlerRoomsNotification) {
        self.send(SearchActorMessage::RoomsAvailable(notification))
            .await;
    }

    /// Handle every earlier message, then collect what the actor published.
    ///
    /// The empty-query command is an ordinary foreground command: its success
    /// action settles only after the mailbox ahead of it was handled, so it is
    /// both the barrier and the proof that foreground work still advances while
    /// the burst is pending.
    async fn settle(&mut self) -> Vec<AppAction> {
        self.barrier += 1;
        self.send(SearchActorMessage::Query {
            request_id: RequestId {
                connection_id: RuntimeConnectionId(1),
                sequence: self.barrier,
            },
            query: String::new(),
            scope: SearchScope::AllRooms,
            room_filter: SearchRoomFilter::AllRooms,
            content_policy: None,
            enqueued_at: Instant::now(),
        })
        .await;
        let mut published = Vec::new();
        loop {
            let batch = crate::executor::timeout(SETTLE_WINDOW, self.actions.recv())
                .await
                .expect("the foreground barrier command must settle")
                .expect("the actor is still alive");
            let settled = batch.iter().any(|action| {
                matches!(
                    action,
                    AppAction::SearchSucceeded { .. } | AppAction::SearchFailed { .. }
                )
            });
            published.extend(batch);
            if settled {
                return published;
            }
        }
    }

    /// One burst iteration: every signal kind a room can emit.
    async fn burst(&self, rounds: usize) {
        for _ in 0..rounds {
            for room_id in [ACTIVE_ROOM, PARTIAL_ROOM, FRESH_ROOM, SETTLED_ROOM] {
                self.send(SearchActorMessage::LateDecryptionResolved {
                    room_id: room_id.to_owned(),
                })
                .await;
            }
            self.send(SearchActorMessage::LateDecryptionLagging).await;
        }
    }
}

// ---------------------------------------------------------------------------
// Direct-actor fixture
// ---------------------------------------------------------------------------

async fn direct_actor(
    settings: SearchCrawlerSettings,
) -> (SearchActor, mpsc::Receiver<Vec<AppAction>>) {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let session = MatrixClientSession::from_client_for_testing(
        client.clone(),
        session_info(&client, &server),
    );
    let (action_tx, action_rx) = mpsc::channel(256);
    let (event_tx, _events) = broadcast::channel(32);
    let (_, msg_rx) = mpsc::channel(32);
    let mut actor = SearchActor::new(
        Arc::new(session),
        action_tx,
        event_tx,
        msg_rx,
        AccountWorkScheduler::default(),
    );
    actor.crawl_delay_elapsed = true;
    actor.crawler_settings = settings;
    (actor, action_rx)
}

fn drain(actions: &mut mpsc::Receiver<Vec<AppAction>>) -> Vec<AppAction> {
    let mut drained = Vec::new();
    while let Ok(batch) = actions.try_recv() {
        drained.extend(batch);
    }
    drained
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The measured amplification (#1781): a burst that never lets a page advance
/// must enqueue at most one crawl per distinct room — independently of how many
/// signals arrive — and a repeat over unchanged pending work must publish
/// nothing at all.
#[tokio::test]
async fn a_late_decryption_burst_enqueues_at_most_one_crawl_per_room() {
    let settings = active_settings();
    let mut harness = harness(Seeded {
        settings: settings.clone(),
        rooms: vec![ACTIVE_ROOM, PARTIAL_ROOM, FRESH_ROOM, SETTLED_ROOM],
        settled: vec![(SETTLED_ROOM, SETTLED_LATEST)],
        // The front of the queue starts and holds the pump; the second entry
        // already read history.
        queued: vec![(ACTIVE_ROOM, false), (PARTIAL_ROOM, true)],
    })
    .await;
    let hold = harness
        .scheduler
        .begin_interactive(AccountWorkKind::MessageSend);
    harness
        .notify(notification(
            &[ACTIVE_ROOM, PARTIAL_ROOM, FRESH_ROOM, SETTLED_ROOM],
            &[(SETTLED_ROOM, SETTLED_LATEST)],
            &settings,
        ))
        .await;

    // Setup: the notification itself queues the one room that is neither
    // settled nor already queued.
    let setup = harness.settle().await;
    assert_eq!(
        started_rooms(&setup),
        [FRESH_ROOM.to_owned()],
        "only the unknown room is enqueued by the room-list notification"
    );

    // A lag report on its own must cover every joined room, so it reaches the
    // settled room without any per-room signal.
    for _ in 0..100 {
        harness
            .send(SearchActorMessage::LateDecryptionLagging)
            .await;
    }
    let lag_only = harness.settle().await;
    let lag_only_enqueues = started_rooms(&lag_only);
    assert!(
        lag_only_enqueues == [SETTLED_ROOM.to_owned()],
        "100 lag reports enqueued {} crawls for 4 rooms; only the settled room needs one",
        lag_only_enqueues.len()
    );

    // First half of the burst, then a foreground command; the page still cannot
    // advance because the interactive hold is up.
    harness.burst(100).await;
    let first = harness.settle().await;

    // Second half over the same unchanged pending work.
    harness.burst(100).await;
    let second = harness.settle().await;
    drop(hold);

    let rooms = 4;
    assert!(
        started_rooms(&lag_only).len() + started_rooms(&first).len() <= rooms,
        "600 signals enqueued {} crawls for {rooms} rooms",
        started_rooms(&lag_only).len() + started_rooms(&first).len()
    );
    assert_eq!(
        started_rooms(&second),
        Vec::<String>::new(),
        "repeated signals over unchanged pending work must not publish another enqueue"
    );
    assert_eq!(
        completed_rooms(&lag_only).len()
            + completed_rooms(&first).len()
            + completed_rooms(&second).len(),
        0,
        "no room may report completion while its page is held"
    );
}

/// A change that arrives while a page is running is not covered by that page.
/// Its stale completion must not settle the room; one follow-up scan is owed,
/// and only that scan settles it.
#[tokio::test]
async fn a_page_result_that_raced_a_resolution_does_not_settle_the_room() {
    let (mut actor, mut actions) = direct_actor(active_settings()).await;
    actor.available_crawl_rooms.insert(RACE_ROOM.to_owned());
    // The room was reopened earlier, so its record owes a reread and the
    // reopened scan is the one running now.
    let mut progress = settled_progress(&active_settings(), &[(RACE_ROOM, "$race-latest")]);
    progress.room_mut(RACE_ROOM).needs_reread = true;
    actor.adopt_loaded_progress(progress, "instance".to_owned());
    let running = queued_checkpoint(&actor, RACE_ROOM, true);
    actor.active_crawl_checkpoint = Some(running.clone());

    // A second resolution lands while that page is in flight.
    actor
        .handle_actor_message(SearchActorMessage::LateDecryptionResolved {
            room_id: RACE_ROOM.to_owned(),
        })
        .await;
    let during = drain(&mut actions);
    assert_eq!(
        started_rooms(&during),
        Vec::<String>::new(),
        "a change that lands during a running page is one pending follow-up, not a new crawl"
    );

    // The running page now finishes and reports that it read the whole room.
    actor
        .handle_history_crawl_page_result(HistoryCrawlPageResult::Success {
            checkpoint: running,
            messages: Vec::new(),
            completed: true,
            work_permit: None,
        })
        .await;
    let stale = drain(&mut actions);

    assert_eq!(
        completed_rooms(&stale),
        Vec::<String>::new(),
        "a page that started before the change must not settle the room"
    );
    assert!(
        !actor.completed_rooms.contains_key(RACE_ROOM),
        "the room is not settled by the stale page"
    );
    assert!(
        actor
            .search_progress
            .as_ref()
            .and_then(|progress| progress.room(RACE_ROOM))
            .is_some_and(|room| room.needs_reread),
        "the newer obligation stays durable"
    );
    assert_eq!(
        actor.crawl_queue.len(),
        1,
        "exactly one follow-up scan is queued"
    );
    let follow_up = actor
        .crawl_queue
        .pop_front()
        .expect("the follow-up scan is queued");
    assert!(
        follow_up.first_page && follow_up.processed == 0 && follow_up.catch_up_after.is_none(),
        "the follow-up must read the room from its newest event, not resume behind the change"
    );
    assert_eq!(
        started_rooms(&stale),
        [RACE_ROOM.to_owned()],
        "the follow-up is published once"
    );

    // Serving the follow-up settles the room, and only then.
    actor.queued_crawl_rooms.remove(RACE_ROOM);
    actor
        .handle_history_crawl_page_result(HistoryCrawlPageResult::Success {
            checkpoint: follow_up,
            messages: Vec::new(),
            completed: true,
            work_permit: None,
        })
        .await;
    let settled = drain(&mut actions);
    assert_eq!(
        completed_rooms(&settled),
        [RACE_ROOM.to_owned()],
        "the follow-up scan settles the room"
    );
    assert!(actor.completed_rooms.contains_key(RACE_ROOM));
}

/// A queued continuation already passed part of the room, so it cannot cover a
/// change: it becomes a fresh scan instead of resuming behind it — and repeated
/// signals do not grow the queue.
#[tokio::test]
async fn a_queued_continuation_becomes_one_fresh_scan() {
    let (mut actor, mut actions) = direct_actor(active_settings()).await;
    actor.available_crawl_rooms.insert(PARTIAL_ROOM.to_owned());
    let continuation = queued_checkpoint(&actor, PARTIAL_ROOM, true);
    actor
        .enqueue_history_crawl(continuation, 0, CrawlEnqueueCause::Initial)
        .await;
    let setup = drain(&mut actions);
    assert_eq!(started_rooms(&setup), [PARTIAL_ROOM.to_owned()]);

    for _ in 0..10 {
        actor
            .handle_actor_message(SearchActorMessage::LateDecryptionResolved {
                room_id: PARTIAL_ROOM.to_owned(),
            })
            .await;
    }
    let published = drain(&mut actions);

    assert!(
        actor.crawl_queue.len() <= 1,
        "a burst keeps at most one entry for the room"
    );
    let queued = actor
        .crawl_queue
        .front()
        .or(actor.active_crawl_checkpoint.as_ref())
        .expect("the room is still queued or running");
    assert!(
        queued.starts_over(),
        "the continuation is replaced by a scan that starts over"
    );
    assert_eq!(
        started_rooms(&published),
        Vec::<String>::new(),
        "replacing a queued checkpoint is not a new enqueue"
    );
}

/// Pause stops the crawler: a signal while paused records the durable
/// obligation but starts no work, and resuming queues it again.
#[tokio::test]
async fn pausing_holds_reopened_work_until_the_crawler_resumes() {
    let settings = active_settings();
    let mut harness = harness(Seeded {
        settings: settings.clone(),
        rooms: vec![SETTLED_ROOM],
        settled: vec![(SETTLED_ROOM, SETTLED_LATEST)],
        queued: Vec::new(),
    })
    .await;
    harness
        .notify(notification(
            &[SETTLED_ROOM],
            &[(SETTLED_ROOM, SETTLED_LATEST)],
            &settings,
        ))
        .await;
    harness.settle().await;

    harness
        .notify(notification(
            &[SETTLED_ROOM],
            &[(SETTLED_ROOM, SETTLED_LATEST)],
            &paused_settings(),
        ))
        .await;
    harness.settle().await;

    for _ in 0..5 {
        harness
            .send(SearchActorMessage::LateDecryptionResolved {
                room_id: SETTLED_ROOM.to_owned(),
            })
            .await;
        harness
            .send(SearchActorMessage::LateDecryptionLagging)
            .await;
    }
    let paused = harness.settle().await;
    assert_eq!(
        started_rooms(&paused),
        Vec::<String>::new(),
        "a paused crawler must not start reopened work"
    );

    harness
        .notify(notification(
            &[SETTLED_ROOM],
            &[(SETTLED_ROOM, SETTLED_LATEST)],
            &settings,
        ))
        .await;
    let resumed = harness.settle().await;
    assert_eq!(
        started_rooms(&resumed),
        [SETTLED_ROOM.to_owned()],
        "resuming queues the room whose reread is still owed"
    );
}

/// A restart must re-crawl a room whose reread is still owed, so a pending
/// follow-up cannot be lost when the session ends: the obligation has to reach
/// the stored record, not only this session's memory.
#[tokio::test]
async fn a_restart_re_crawls_a_room_that_still_owes_a_reread() {
    use crate::store::test_support::{file_store_actor, make_key_id};

    let data_dir = tempfile::tempdir().expect("tempdir");
    let cred_dir = tempfile::tempdir().expect("tempdir");
    let key_id = make_key_id();
    let store = file_store_actor(&data_dir, &cred_dir);
    let instance = store.search_index_instance(&key_id).expect("instance");
    let settings = active_settings();
    let settled = settled_progress_for(&instance, &settings, &[(RESTART_ROOM, "$restart-latest")]);
    store
        .save_search_progress(&key_id, &settled)
        .expect("the previous session committed the room");

    let (mut actor, _actions) = direct_actor(settings.clone()).await;
    actor.search_progress_writer = Some((store.clone(), key_id.clone()));
    actor.available_crawl_rooms.insert(RESTART_ROOM.to_owned());
    actor.adopt_loaded_progress(settled, instance.clone());
    actor
        .handle_rooms_available(notification(
            &[RESTART_ROOM],
            &[(RESTART_ROOM, "$restart-latest")],
            &settings,
        ))
        .await;
    assert!(actor.completed_rooms.contains_key(RESTART_ROOM));

    // The change lands while the room's scan is running: the obligation is
    // pending, not yet served.
    actor.active_crawl_checkpoint = Some(queued_checkpoint(&actor, RESTART_ROOM, true));
    actor
        .handle_actor_message(SearchActorMessage::LateDecryptionResolved {
            room_id: RESTART_ROOM.to_owned(),
        })
        .await;

    let durable = store
        .load_search_progress(&key_id)
        .expect("load")
        .expect("the session wrote the record");
    assert!(
        durable
            .room(RESTART_ROOM)
            .is_some_and(|room| room.needs_reread),
        "the outstanding reread reached the store"
    );

    // A new session loads that record and must crawl the room again.
    let (mut restarted, mut actions) = direct_actor(settings.clone()).await;
    restarted.adopt_loaded_progress(durable, instance);
    restarted
        .handle_rooms_available(notification(
            &[RESTART_ROOM],
            &[(RESTART_ROOM, "$restart-latest")],
            &settings,
        ))
        .await;
    assert!(
        !restarted.completed_rooms.contains_key(RESTART_ROOM),
        "a room that still owes a reread is not seeded as settled"
    );
    assert_eq!(
        started_rooms(&drain(&mut actions)),
        [RESTART_ROOM.to_owned()],
        "the restarted session queues the outstanding reread"
    );
}

/// Product-level guard: a room reopened while its page was held settles and the
/// event that arrived with the signal is searchable.
///
/// The mock SDK also indexes synced events on its own, so this does not prove
/// which path indexed the event; the crawler's obligation is proven by the
/// actor-level cases above.
#[tokio::test]
async fn a_room_reopened_while_its_page_was_held_settles_and_stays_searchable() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = matrix_sdk::ruma::room_id!("!invalidation-index:example.invalid");
    let factory = EventFactory::new()
        .room(room_id)
        .sender(matrix_sdk::ruma::user_id!("@member:example.invalid"));
    server
        .mock_sync()
        .ok_and_run(&client, |builder| {
            builder.add_joined_room(
                JoinedRoomBuilder::new(room_id).add_timeline_event(
                    factory
                        .text_msg("synthetic first payload")
                        .event_id(matrix_sdk::ruma::event_id!("$invalidation-first")),
                ),
            );
        })
        .await;
    let settings = active_settings();
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client.clone(),
        session_info(&client, &server),
    ));
    let (action_tx, mut actions) = mpsc::channel(4096);
    let (event_tx, _events) = broadcast::channel(64);
    let (tx, msg_rx) = mpsc::channel(4096);
    let (index_tx, index_rx) = mpsc::channel(SEARCH_INDEX_MUTATION_QUEUE);
    let scheduler = AccountWorkScheduler::default();
    let mut actor = SearchActor::new(session, action_tx, event_tx, msg_rx, scheduler.clone());
    actor.crawl_delay_elapsed = true;
    let task = executor::spawn(actor.run(index_rx));
    let handle = SearchActorHandle {
        tx: tx.clone(),
        index_tx,
        task: Some(task),
        report_task: None,
    };

    let hold = scheduler.begin_interactive(AccountWorkKind::MessageSend);
    tx.send(SearchActorMessage::RoomsAvailable(notification(
        &[room_id.as_str()],
        &[(room_id.as_str(), "$invalidation-first")],
        &settings,
    )))
    .await
    .expect("the actor inbox accepts the notification");

    // The page cannot advance while the hold is up; the decrypted event and its
    // signal arrive in that window.
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(room_id).add_timeline_event(
                factory
                    .text_msg("synthetic needle")
                    .event_id(matrix_sdk::ruma::event_id!("$invalidation-decrypted")),
            ),
        )
        .await;
    for _ in 0..3 {
        tx.send(SearchActorMessage::LateDecryptionResolved {
            room_id: room_id.to_string(),
        })
        .await
        .expect("the actor inbox accepts the resolution");
    }
    drop(hold);

    let completed = crate::executor::timeout(SETTLE_WINDOW, async {
        while let Some(batch) = actions.recv().await {
            if batch.iter().any(|action| {
                matches!(action, AppAction::HistoryCrawlCompleted { room_id: done, .. } if done == room_id.as_str())
            }) {
                return true;
            }
        }
        false
    })
    .await
    .expect("the room settles eventually");
    assert!(completed, "the room reports completion");

    let room = client.get_room(room_id).expect("the room is known");
    let page = room
        .search_literal_page("needle", 10, None)
        .await
        .expect("the index answers");
    assert_eq!(
        page.len(),
        1,
        "the event that arrived with the signal is searchable"
    );

    drop(handle);
}
