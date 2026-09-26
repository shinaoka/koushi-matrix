//! `search_crawler_catchup` (#996 ②): a room the history crawler has already
//! completed must be revisited when new messages reach it, without the reader
//! ever opening the room.
//!
//! Topology: sender B creates a private room and invites reader A. A never
//! subscribes to the room's timeline, so nothing but the crawler can feed A's
//! search index for it.
//!
//! Proofs (token-only, private-data-free):
//! - `crawl_catchup_live=ok`: after the room reached `Completed`, a message B
//!   posts while A is running becomes searchable (All rooms scope).
//! - `crawl_catchup_restart=ok`: messages B posts while A is stopped become
//!   searchable after A restarts, again without opening the room.
//! - `search_crawler_catchup=ok`: both proofs passed and both runtimes were
//!   cleaned up.

use super::cleanup::cleanup_logged_in_runtime;
use super::event_wait::{
    stop_sync_for_qa, subscribe_timeline_for_qa, wait_for_invite_in_snapshot,
    wait_for_ready_snapshot, wait_for_send_flow_completion, wait_for_session_restored,
    wait_for_sync_running_after_reconnect, wait_for_sync_started,
};
use super::fixtures::{accept_invite_for_qa, create_room_for_qa, invite_user_for_qa};
use super::participants::{QaParticipantLoginGate, login_synced_participant_for_qa, qa_data_dir};
use super::registry::{EVENT_TIMEOUT, QaConfig};
use super::{
    AccountCommand, AccountKey, CoreCommand, CoreConnection, CoreEvent, CoreRuntime, Duration,
    RoomCommand, SearchCommand, SearchCrawlerRoomState, SearchEvent, SearchScope, SyncCommand,
    TimelineCommand, TimelineKey, TimelineKind,
};

/// The crawler holds automatic work for a fixed startup delay (60 s) before
/// its first page, so every wait that spans a runtime start budgets for it.
const CRAWL_AFTER_START_TIMEOUT: Duration = Duration::from_secs(150);
/// A message that reaches an already-completed room must become searchable
/// well inside this window once the startup delay has passed.
const LIVE_CATCHUP_TIMEOUT: Duration = Duration::from_secs(45);

const SEED_BODIES: [&str; 3] = [
    "catchupseed alpha synthetic",
    "catchupseed bravo synthetic",
    "catchupseed charlie synthetic",
];
const LIVE_BODY: &str = "catchuplive delta synthetic";
const OFFLINE_BODIES: [&str; 4] = [
    "catchupoffline echo synthetic",
    "catchupoffline foxtrot synthetic",
    "catchupoffline golf synthetic",
    "catchupoffline hotel synthetic",
];

pub(super) async fn run_search_crawler_catchup_scenario(config: &QaConfig) -> Result<(), String> {
    let data_dir_a = qa_data_dir("search-catchup-a");
    let participant_a = login_synced_participant_for_qa(
        &config.homeserver,
        data_dir_a.clone(),
        &config.user_a,
        &config.password_a,
        "Koushi Search Catchup QA A",
        "search catch-up login A",
        "search catch-up gate A",
        QaParticipantLoginGate::BootstrapNewIdentity,
    )
    .await?;
    let participant_b = login_synced_participant_for_qa(
        &config.homeserver,
        qa_data_dir("search-catchup-b"),
        &config.user_b,
        &config.password_b,
        "Koushi Search Catchup QA B",
        "search catch-up login B",
        "search catch-up gate B",
        QaParticipantLoginGate::BootstrapNewIdentity,
    )
    .await?;
    let super::participants::QaParticipantLoginOutcome {
        runtime: runtime_a,
        conn: mut conn_a,
        account_key: account_key_a,
        ..
    } = participant_a;
    let super::participants::QaParticipantLoginOutcome {
        runtime: runtime_b,
        conn: mut conn_b,
        account_key: account_key_b,
        ..
    } = participant_b;
    let user_a_id = format!("@{}:{}", config.user_a, config.server_name);

    let live = run_live_catchup_flow(&mut conn_a, &mut conn_b, &account_key_b, &user_a_id).await;
    let (room_id, sender_key) = match live {
        Ok(flow) => flow,
        Err(flow) => {
            let _ = cleanup_logged_in_runtime(
                conn_b,
                runtime_b,
                account_key_b,
                "search catch-up failed cleanup B",
            )
            .await;
            let _ = cleanup_logged_in_runtime(
                conn_a,
                runtime_a,
                account_key_a,
                "search catch-up failed cleanup A",
            )
            .await;
            return Err(flow);
        }
    };
    println!("crawl_catchup_live=ok");

    // Quit A, then let B post while A is away.
    let stopped = stop_sync_for_qa(&mut conn_a, "search catch-up stop A").await;
    drop(conn_a);
    tokio::time::timeout(EVENT_TIMEOUT, runtime_a.shutdown())
        .await
        .map_err(|_| "search catch-up runtime shutdown timed out".to_owned())?;
    let offline = match stopped {
        Ok(()) => post_offline_messages(&mut conn_b, &sender_key).await,
        Err(error) => Err(error),
    };
    let cleanup_b = cleanup_logged_in_runtime(
        conn_b,
        runtime_b,
        account_key_b,
        "search catch-up cleanup B",
    )
    .await;
    let offline_event_ids = offline?;

    let restarted_runtime = CoreRuntime::start_with_data_dir(data_dir_a);
    let mut restarted_conn = restarted_runtime.attach();
    let restart = run_restart_catchup_flow(
        &mut restarted_conn,
        &account_key_a,
        &room_id,
        &offline_event_ids,
    )
    .await;
    let cleanup_a = cleanup_logged_in_runtime(
        restarted_conn,
        restarted_runtime,
        account_key_a,
        "search catch-up cleanup restarted A",
    )
    .await;
    restart?;
    println!("crawl_catchup_restart=ok");
    cleanup_b.and(cleanup_a)?;
    println!("search_crawler_catchup=ok");
    Ok(())
}

/// Seed the room, wait for A's crawler to complete it, then post one more
/// message while A is running and require it to become searchable.
async fn run_live_catchup_flow(
    conn_a: &mut CoreConnection,
    conn_b: &mut CoreConnection,
    account_key_b: &AccountKey,
    user_a_id: &str,
) -> Result<(String, TimelineKey), String> {
    // A sits in another room, as the reporter did: a selected room's timeline
    // is open and indexed live, which would hide the crawler gap.
    let decoy_room_id = create_room_for_qa(
        conn_a,
        "Synthetic Search Catchup Decoy",
        false,
        "search catch-up decoy room",
    )
    .await?;
    select_room(conn_a, &decoy_room_id).await?;
    let room_id = create_room_for_qa(
        conn_b,
        "Synthetic Search Catchup Room",
        false,
        "search catch-up room",
    )
    .await?;
    invite_user_for_qa(conn_b, &room_id, user_a_id, "search catch-up invite A").await?;
    wait_for_invite_in_snapshot(conn_a, &room_id, None, "search catch-up invite A").await?;
    accept_invite_for_qa(conn_a, &room_id, "search catch-up A accepts invite").await?;
    let sender_key = TimelineKey {
        account_key: account_key_b.clone(),
        kind: TimelineKind::Room {
            room_id: room_id.clone(),
        },
    };
    subscribe_timeline_for_qa(conn_b, &sender_key, "search catch-up subscribe sender").await?;

    let mut seed_event_ids = Vec::new();
    for (index, body) in SEED_BODIES.iter().enumerate() {
        seed_event_ids.push(
            send_text_and_wait_event(
                conn_b,
                &sender_key,
                &format!("search-catchup-seed-{index}"),
                body,
                "search catch-up seed",
            )
            .await?,
        );
    }

    // A never opens the room: only the crawler can index it.
    assert_room_not_open(conn_a, &room_id)?;
    wait_for_crawler_completed(conn_a, &room_id, CRAWL_AFTER_START_TIMEOUT).await?;
    wait_until_searchable(
        conn_a,
        "catchupseed",
        &seed_event_ids,
        CRAWL_AFTER_START_TIMEOUT,
        "search catch-up seed indexed",
    )
    .await?;

    let live_event_id = send_text_and_wait_event(
        conn_b,
        &sender_key,
        "search-catchup-live",
        LIVE_BODY,
        "search catch-up live",
    )
    .await?;
    wait_until_searchable(
        conn_a,
        "catchuplive",
        &[live_event_id],
        LIVE_CATCHUP_TIMEOUT,
        "search catch-up live message indexed in completed room",
    )
    .await?;
    assert_room_not_open(conn_a, &room_id)?;
    Ok((room_id, sender_key))
}

async fn select_room(conn: &mut CoreConnection, room_id: &str) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Room(RoomCommand::SelectRoom {
        request_id,
        room_id: room_id.to_owned(),
    }))
    .await
    .map_err(|e| format!("search catch-up: submit decoy select failed: {e}"))?;
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    while conn.snapshot().timeline.room_id.as_deref() != Some(room_id) {
        if tokio::time::Instant::now() >= deadline {
            return Err("search catch-up: timed out selecting the decoy room".to_owned());
        }
        let _ = tokio::time::timeout(Duration::from_secs(2), conn.recv_event()).await;
    }
    Ok(())
}

fn assert_room_not_open(conn: &CoreConnection, room_id: &str) -> Result<(), String> {
    if conn.snapshot().timeline.room_id.as_deref() == Some(room_id) {
        return Err("search catch-up: the probe room was opened by the reader".to_owned());
    }
    Ok(())
}

async fn post_offline_messages(
    conn_b: &mut CoreConnection,
    sender_key: &TimelineKey,
) -> Result<Vec<String>, String> {
    let mut event_ids = Vec::new();
    for (index, body) in OFFLINE_BODIES.iter().enumerate() {
        event_ids.push(
            send_text_and_wait_event(
                conn_b,
                sender_key,
                &format!("search-catchup-offline-{index}"),
                body,
                "search catch-up offline",
            )
            .await?,
        );
    }
    Ok(event_ids)
}

async fn run_restart_catchup_flow(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    room_id: &str,
    offline_event_ids: &[String],
) -> Result<(), String> {
    let restore_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::RestoreSession {
        request_id: restore_id,
        account_key: account_key.clone(),
    }))
    .await
    .map_err(|_| "search catch-up restore submission failed".to_owned())?;
    wait_for_session_restored(
        conn,
        restore_id,
        account_key,
        "search catch-up restored session",
    )
    .await?;
    wait_for_ready_snapshot(conn, "search catch-up restored Ready").await?;
    let sync_id = conn.next_request_id();
    conn.command(CoreCommand::Sync(SyncCommand::Start {
        request_id: sync_id,
    }))
    .await
    .map_err(|_| "search catch-up restarted sync submission failed".to_owned())?;
    wait_for_sync_started(conn, sync_id, "search catch-up restarted sync").await?;
    wait_for_sync_running_after_reconnect(conn, "search catch-up restarted sync running").await?;

    // Still without opening the room.
    assert_room_not_open(conn, room_id)?;
    wait_until_searchable(
        conn,
        "catchupoffline",
        offline_event_ids,
        CRAWL_AFTER_START_TIMEOUT,
        "search catch-up offline messages indexed after restart",
    )
    .await?;
    assert_room_not_open(conn, room_id)?;
    wait_for_crawler_completed(conn, room_id, CRAWL_AFTER_START_TIMEOUT).await
}

async fn wait_for_crawler_completed(
    conn: &mut CoreConnection,
    room_id: &str,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match conn.snapshot().search_crawler.rooms.get(room_id) {
            Some(SearchCrawlerRoomState::Completed { .. }) => return Ok(()),
            Some(SearchCrawlerRoomState::Failed { kind }) => {
                return Err(format!(
                    "search catch-up: room crawl failed with kind={kind:?}"
                ));
            }
            _ => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("search catch-up: timed out waiting for the room crawl".to_owned());
        }
        let _ = tokio::time::timeout(Duration::from_secs(2), conn.recv_event()).await;
    }
}

/// Poll an All-rooms search until every expected event id is returned.
async fn wait_until_searchable(
    conn: &mut CoreConnection,
    query: &str,
    expected_event_ids: &[String],
    timeout: Duration,
    label: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let request_id = conn.next_request_id();
        conn.command(CoreCommand::Search(SearchCommand::Query {
            request_id,
            query: query.to_owned(),
            scope: SearchScope::AllRooms,
            room_filter: koushi_state::SearchRoomFilter::AllRooms,
        }))
        .await
        .map_err(|e| format!("{label}: submit search query: {e}"))?;
        let found = latest_result_count(conn, request_id, expected_event_ids, label).await?;
        if found == expected_event_ids.len() {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "{label}: timed out with {found} of {} expected results",
                expected_event_ids.len()
            ));
        }
        tokio::time::sleep(Duration::from_millis(1_000)).await;
    }
}

/// Count how many expected events the query returned. The actor answers a
/// query with a local result and may follow with an SDK-supplemented one for
/// the same request id; keep the best of the answers seen in a short window.
async fn latest_result_count(
    conn: &mut CoreConnection,
    request_id: koushi_protocol::ids::RequestId,
    expected_event_ids: &[String],
    label: &str,
) -> Result<usize, String> {
    let mut best = None::<usize>;
    let window = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let remaining = window.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return best.ok_or_else(|| format!("{label}: no search results event"));
        }
        let event = match tokio::time::timeout(remaining, conn.recv_event()).await {
            Ok(Ok(event)) => event,
            Ok(Err(_)) => continue,
            Err(_) => return best.ok_or_else(|| format!("{label}: no search results event")),
        };
        match event {
            CoreEvent::Search(SearchEvent::Results {
                request_id: event_request_id,
                results,
            }) if event_request_id == request_id => {
                let found = expected_event_ids
                    .iter()
                    .filter(|expected| results.iter().any(|result| &result.event_id == *expected))
                    .count();
                best = Some(best.map_or(found, |previous| previous.max(found)));
                if found == expected_event_ids.len() {
                    return Ok(found);
                }
            }
            CoreEvent::OperationFailed {
                request_id: event_request_id,
                failure,
            } if event_request_id == request_id => {
                return Err(format!("{label}: search query failed: {failure:?}"));
            }
            _ => {}
        }
    }
}

async fn send_text_and_wait_event(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    transaction_id: &str,
    body: &str,
    label: &str,
) -> Result<String, String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
        request_id,
        key: key.clone(),
        transaction_id: transaction_id.to_owned(),
        document: koushi_state::ComposerDocument::from_plain_text(body.to_owned()),
    }))
    .await
    .map_err(|_| format!("{label}: send submission failed"))?;
    Ok(
        wait_for_send_flow_completion(conn, request_id, key, transaction_id, body, label)
            .await?
            .event_id,
    )
}
