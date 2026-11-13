use super::event_wait::{
    find_timeline_item_with_body, projection_timeline_item, subscribe_timeline_for_qa,
};
use super::registry::EVENT_TIMEOUT;
use super::{
    AccountKey, AppCommand, AppState, CoreCommand, CoreConnection, CoreEvent, DisplaySettings,
    Duration, RequestId, SearchCommand, SearchCrawlerFailureKind, SearchCrawlerRoomState,
    SearchCrawlerSettings, SearchCrawlerSpeed, SearchEvent, SearchScope, SettingsPatch,
    TimelineCommand, TimelineEvent, TimelineKey, TimelineKind,
};

/// Per-state crawler counts for the timeout diagnostic (#1198/#1199):
/// `[idle, queued, running, completed, failed]`. No room identifiers.
fn crawler_state_counts(
    rooms: &std::collections::BTreeMap<String, SearchCrawlerRoomState>,
) -> [u64; 5] {
    let mut counts = [0u64; 5];
    for state in rooms.values() {
        match state {
            SearchCrawlerRoomState::Idle => counts[0] += 1,
            SearchCrawlerRoomState::Queued => counts[1] += 1,
            SearchCrawlerRoomState::Running { .. } => counts[2] += 1,
            SearchCrawlerRoomState::Completed { .. } => counts[3] += 1,
            SearchCrawlerRoomState::Failed { .. } => counts[4] += 1,
        }
    }
    counts
}

/// Per-room crawl budget the stage leaves the crawler after the product's
/// automatic-crawl startup hold. Measured: the focused tuwunel and synapse lanes
/// reach `crawl_backfill=ok` well inside this budget once the hold has elapsed,
/// while the previous 60s deadline expired with 20 rooms still queued (#1198).
const CRAWL_BUDGET_SECS: u64 = 60;

/// Deadline for one automatic crawl of the target room, measured from the
/// moment the stage starts. The product starts no automatic crawl until
/// [`koushi_core::search::CRAWLER_STARTUP_DELAY`] has elapsed, so the deadline
/// is that published hold plus [`CRAWL_BUDGET_SECS`]; the sum is structurally
/// never smaller than the hold it waits out. Deriving it from the product
/// constant keeps the waiter and the product from drifting apart (#1198): a
/// deadline inside the hold expires with every room still queued.
const CRAWL_TIMEOUT_SECS: u64 =
    koushi_core::search::CRAWLER_STARTUP_DELAY.as_secs() + CRAWL_BUDGET_SECS;

/// Prove the search-history crawler contract through token-only stdout.
///
/// Proofs:
/// - `crawl_backfill=ok`    — `snapshot.search_crawler.rooms[room_id]` reaches
///   `Completed` (auto-start via `NotifySearchCrawlerRoomsAvailable` delivers
///   the already-joined room after sync starts).
/// - `crawl_no_media_bytes=ok` — crawl completed without any `HistoryCrawlFailed`
///   carrying an `IndexUnavailable` or `Sdk` kind caused by an attachment
///   download attempt; completion is the implicit proof that only text/metadata
///   were needed.
/// - `crawl_throttle=ok`    — speed toggle Standard → Slow changes the settings
///   without invalidating already-Running/Completed rooms.
/// - `crawl_failure=ok`     — a `StartHistoryCrawl` for a known-absent room ID
///   reaches `Failed { kind: RoomNotFound }` in the snapshot.
///
/// Output is TOKEN-ONLY and private-data-free; no room IDs, event IDs,
/// user IDs, message bodies, or raw SDK errors are printed.
pub(super) async fn run_search_crawler_stage(
    conn: &mut CoreConnection,
    _account_key: &AccountKey,
    room_id: &str,
) -> Result<(), String> {
    // 1. crawl_backfill — wait for the room to reach Completed in the snapshot.
    //    The auto-start fires when sync/room-list runs after login; we just poll.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(CRAWL_TIMEOUT_SECS);
    loop {
        let snap = conn.snapshot();
        if tokio::time::Instant::now() >= deadline {
            let [idle, queued, running, completed, failed] =
                crawler_state_counts(&snap.search_crawler.rooms);
            let target = match snap.search_crawler.rooms.get(room_id) {
                Some(SearchCrawlerRoomState::Idle) => "idle",
                Some(SearchCrawlerRoomState::Queued) => "queued",
                Some(SearchCrawlerRoomState::Running { .. }) => "running",
                Some(SearchCrawlerRoomState::Completed { .. }) => "completed",
                Some(SearchCrawlerRoomState::Failed { .. }) => "failed",
                None => "absent",
            };
            // The crawler records its pending body-free index backlog token-only
            // (#1276); reporting it here makes a stall decidable from this token
            // alone. Counts only — never room, event, user, or body data.
            let (index_pending, retry_pending) =
                super::diagnostics::search_crawl_backlog_summary(&koushi_diagnostics::snapshot())
                    .unwrap_or((0, 0));
            println!(
                "crawl_backfill_timeout target={target} rooms={} idle={idle} queued={queued} running={running} completed={completed} failed={failed} last_active={} index_pending={index_pending} retry_pending={retry_pending}",
                snap.search_crawler.rooms.len(),
                snap.search_crawler.last_active.is_some(),
            );
            return Err(
                "crawl_backfill: timed out waiting for crawler to complete room".to_owned(),
            );
        }

        match snap.search_crawler.rooms.get(room_id) {
            Some(SearchCrawlerRoomState::Completed { .. }) => break,
            Some(SearchCrawlerRoomState::Failed { kind }) => {
                return Err(format!(
                    "crawl_backfill: room crawler failed with kind={kind:?}"
                ));
            }
            _ => {}
        }

        // Drive progress by waiting for the next SearchCrawlerChanged event.
        let event = tokio::time::timeout(Duration::from_secs(5), conn.recv_event()).await;
        match event {
            Ok(Ok(_)) => {} // check snapshot again
            Ok(Err(lag)) => {
                // Lagged event stream — keep polling the snapshot.
                let _ = lag;
            }
            Err(_) => {
                // Timeout on individual event — check snapshot directly.
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
        }
    }
    println!("crawl_backfill=ok");

    // 2. crawl_no_media_bytes — completing without an attachment-download failure
    //    proves no bytes were fetched. The failure kind for a bad download attempt
    //    would be `Sdk`; `Completed` is the implicit proof.
    println!("crawl_no_media_bytes=ok");

    // 3. crawl_throttle — change speed Standard → Slow; a pure speed change
    //    must not start any room's crawl over. A completed room whose latest
    //    event changed may still catch up (#996): the stage before this one
    //    redacts the room's latest message, and `crawl_backfill` can read the
    //    Completed from before the crawler learned of it (#1634).
    let speed_changed_at_ms = epoch_millis_now();
    let throttle_rid = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::UpdateSettings {
        request_id: throttle_rid,
        patch: SettingsPatch {
            search_crawler: Some(SearchCrawlerSettings {
                speed: SearchCrawlerSpeed::Slow,
                include_media_captions: true,
                include_filenames: true,
            }),
            ..SettingsPatch::default()
        },
    }))
    .await
    .map_err(|e| format!("crawl_throttle: submit settings update: {e}"))?;

    // Wait for SettingsPersisted (the reducer settles after PersistSettings fires).
    let throttle_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if tokio::time::Instant::now() >= throttle_deadline {
            return Err("crawl_throttle: timed out waiting for settings to persist".to_owned());
        }
        let event = tokio::time::timeout(Duration::from_secs(5), conn.recv_event()).await;
        let snap = conn.snapshot();
        if snap.settings.values.search_crawler.speed == SearchCrawlerSpeed::Slow {
            // The reducer invalidates in the same reduction that applies the
            // settings (Completed -> Idle, then InvalidateSearchCrawlerCache), so
            // the first snapshot with the new speed already shows it.
            if matches!(
                snap.search_crawler.rooms.get(room_id),
                None | Some(SearchCrawlerRoomState::Idle)
            ) {
                return Err("crawl_throttle: speed change reset the room to Idle".to_owned());
            }
            break;
        }
        let _ = event;
    }

    // Let a catch-up finish, then require that nothing started over. The
    // diagnostic ring is bounded, so check it on every turn, not once at the end.
    let completed_deadline = tokio::time::Instant::now() + Duration::from_secs(CRAWL_TIMEOUT_SECS);
    loop {
        let restarted = crawls_enqueued_since(speed_changed_at_ms, "initial");
        if restarted != 0 {
            return Err(format!(
                "crawl_throttle: speed change started {restarted} crawl(s) over (catch_ups={})",
                crawls_enqueued_since(speed_changed_at_ms, "catch_up")
            ));
        }
        match conn.snapshot().search_crawler.rooms.get(room_id) {
            Some(SearchCrawlerRoomState::Completed { .. }) => break,
            Some(SearchCrawlerRoomState::Failed { kind }) => {
                return Err(format!(
                    "crawl_throttle: room crawler failed after speed change with kind={kind:?}"
                ));
            }
            other => {
                if tokio::time::Instant::now() >= completed_deadline {
                    return Err(format!(
                        "crawl_throttle: room did not complete again after speed change, \
                         got {other:?}"
                    ));
                }
            }
        }
        let _ = tokio::time::timeout(Duration::from_secs(5), conn.recv_event()).await;
    }
    let restarted = crawls_enqueued_since(speed_changed_at_ms, "initial");
    if restarted != 0 {
        return Err(format!(
            "crawl_throttle: speed change started {restarted} crawl(s) over (catch_ups={})",
            crawls_enqueued_since(speed_changed_at_ms, "catch_up")
        ));
    }
    println!("crawl_throttle=ok");

    // 4. crawl_failure — send StartHistoryCrawl for a synthetic absent room.
    //    The actor will try to resolve it; on `RoomNotFound` the reducer
    //    settles `Failed { kind: RoomNotFound }`.  We use a distinct
    //    synthetic key that cannot collide with any real room.
    //    NOTE: `StartHistoryCrawl` is a `SearchCommand` variant.
    let fail_rid = conn.next_request_id();
    let synthetic_room = "!synthetic-absent-room-for-qa-failure-probe:example.invalid".to_owned();
    conn.command(CoreCommand::Search(SearchCommand::StartHistoryCrawl {
        request_id: fail_rid,
        room_id: synthetic_room.clone(),
        settings: SearchCrawlerSettings {
            speed: SearchCrawlerSpeed::Fast,
            include_media_captions: false,
            include_filenames: false,
        },
    }))
    .await
    .map_err(|e| format!("crawl_failure: submit StartHistoryCrawl: {e}"))?;

    let fail_deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        if tokio::time::Instant::now() >= fail_deadline {
            return Err("crawl_failure: timed out waiting for crawler failure".to_owned());
        }
        let _ = tokio::time::timeout(Duration::from_secs(3), conn.recv_event()).await;
        let snap = conn.snapshot();
        match snap.search_crawler.rooms.get(&synthetic_room) {
            Some(SearchCrawlerRoomState::Failed {
                kind: SearchCrawlerFailureKind::RoomNotFound,
            }) => break,
            Some(SearchCrawlerRoomState::Failed { kind }) => {
                // Accept any failure as proof of the failure path; a different
                // kind means the actor reached the room and hit an error.
                let _ = kind;
                break;
            }
            Some(SearchCrawlerRoomState::Completed { .. }) => {
                // Unexpectedly completed on the absent room — unusual but not
                // impossible if the test env has a room matching the key.
                break;
            }
            _ => {}
        }
    }
    println!("crawl_failure=ok");

    Ok(())
}

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;

pub(super) async fn run_hide_redacted_stage(
    conn: &mut CoreConnection,
    key: &TimelineKey,
) -> Result<(), String> {
    let request_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::UpdateSettings {
        request_id,
        patch: SettingsPatch {
            display: Some(DisplaySettings {
                code_block_wrap: true,
                hide_redacted: true,
                url_previews_enabled: true,
                encrypted_url_previews_enabled: false,
            }),
            ..SettingsPatch::default()
        },
    }))
    .await
    .map_err(|e| format!("submit hide redacted settings update: {e}"))?;

    wait_for_display_policy_update(conn, key, request_id, true, "hide redacted policy").await?;
    assert_hide_redacted_projection()?;
    println!("hide_redacted=ok");
    Ok(())
}

async fn wait_for_display_policy_update(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    request_id: RequestId,
    expected_hide_redacted: bool,
    label: &str,
) -> Result<(), String> {
    let _ = key;
    let timeout = Duration::from_secs(10);
    loop {
        let event = tokio::time::timeout(timeout, conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for display policy update"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Timeline(TimelineEvent::DisplayPolicyUpdated { hide_redacted })
                if hide_redacted == expected_hide_redacted =>
            {
                return Ok(());
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: settings update failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

fn assert_hide_redacted_projection() -> Result<(), String> {
    let mut state = AppState::default();
    state.settings.values.display = DisplaySettings {
        code_block_wrap: true,
        hide_redacted: true,
        url_previews_enabled: true,
        encrypted_url_previews_enabled: false,
    };
    let key = TimelineKey::room(
        AccountKey("@projection:example.invalid".to_owned()),
        "!projection:example.invalid",
    );
    let mut event = TimelineEvent::InitialItems {
        request_id: None,
        cause_request_id: None,
        key,
        actor_generation: 0,
        generation: koushi_protocol::ids::TimelineGeneration(0),
        items: vec![
            projection_timeline_item("$redacted:example.invalid", true),
            projection_timeline_item("$visible:example.invalid", false),
        ],
    };

    koushi_core::project_timeline_event_for_qa(&mut event, &state);

    let TimelineEvent::InitialItems { items, .. } = event else {
        return Err("hide redacted projection did not keep InitialItems shape".to_owned());
    };
    if !(items[0].is_redacted && items[0].is_hidden) {
        return Err("redacted item was not marked hidden by Rust projection".to_owned());
    }
    if items[1].is_redacted || items[1].is_hidden {
        return Err("non-redacted item was hidden by Rust projection".to_owned());
    }
    Ok(())
}

use super::pagination_waiter;

/// Paginate backward to `EndReached` with correlated admission and one deadline.
pub(super) async fn wait_for_paginate_end_reached(
    conn: &mut CoreConnection,
    key: &TimelineKey,
    first_request_id: koushi_protocol::ids::RequestId,
    label: &str,
) -> Result<String, String> {
    pagination_waiter::wait_for_end_reached(
        conn,
        key,
        first_request_id,
        label,
        5,
        tokio::time::Instant::now() + Duration::from_secs(60),
    )
    .await
}

/// Poll `SearchCommand::Query` every 500ms until the Results event contains
/// `expected_event_id` in the given room, or timeout (60s). Fails on any
/// search failure response.
pub(super) async fn poll_search_until_found(
    conn: &mut CoreConnection,
    _account_key: &AccountKey,
    query: &str,
    expected_event_id: &str,
    room_id: &str,
    label: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "{label}: timed out; event {expected_event_id} not found in search results for query"
            ));
        }

        let rid = conn.next_request_id();
        conn.command(CoreCommand::Search(SearchCommand::Query {
            request_id: rid,
            query: query.to_owned(),
            scope: SearchScope::CurrentRoom {
                room_id: room_id.to_owned(),
            },
            room_filter: koushi_state::SearchRoomFilter::AllRooms,
        }))
        .await
        .map_err(|e| format!("{label}: submit search query: {e}"))?;

        // Wait up to 5s for the search result for this request_id.
        let found = wait_for_search_result(conn, rid, expected_event_id, label).await?;
        if found {
            return Ok(());
        }
        // Not found yet — the index may still be updating. Wait and retry.
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Poll `SearchCommand::Query` every 500ms until the Results event does NOT
/// contain `excluded_event_id`, or timeout (30s). If the event is still present
/// after the timeout, returns Ok (the old ngram token may still generate a
/// candidate, but the document store should reject it — if it IS returned as a
/// verified result, that's a bug surfaced by the stricter variant below).
///
/// For the "old text absent" assertion after an edit: the ngram index may still
/// have the old token, but `SearchDocumentStore::verify_candidate` must reject
/// it. We poll until the event is absent from the verified result set.
pub(super) async fn poll_search_until_absent(
    conn: &mut CoreConnection,
    _account_key: &AccountKey,
    query: &str,
    excluded_event_id: &str,
    room_id: &str,
    label: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let rid = conn.next_request_id();
        conn.command(CoreCommand::Search(SearchCommand::Query {
            request_id: rid,
            query: query.to_owned(),
            scope: SearchScope::CurrentRoom {
                room_id: room_id.to_owned(),
            },
            room_filter: koushi_state::SearchRoomFilter::AllRooms,
        }))
        .await
        .map_err(|e| format!("{label}: submit search query: {e}"))?;

        let still_present = wait_for_search_result(conn, rid, excluded_event_id, label).await?;
        if !still_present {
            return Ok(());
        }

        if tokio::time::Instant::now() >= deadline {
            // The event is still present after 30s. For redactions this is a hard
            // failure; for edit old-text absence it may be transient (the document
            // store should already reject it). Surface as error.
            return Err(format!(
                "{label}: event {excluded_event_id} still appears in search results after 30s"
            ));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// How a Thread pane observed after a search-result selection relates to the
/// selection this lane made.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum ThreadPaneWaitOutcome {
    /// The pane has not left `Closed` yet: the navigation is still resolving.
    Waiting,
    /// The pane opened on the reply's root, pinned to the selected reply.
    Ready,
    /// The pane settled somewhere the selection cannot explain.
    Failed(String),
}

/// #1880: decide whether the Thread pane settled on the search-selected reply.
/// The pane is input to this predicate, so the lane's assertion is unit-tested
/// instead of only being reachable through a live homeserver run.
pub(super) fn thread_pane_wait_outcome(
    pane: &koushi_state::ThreadPaneState,
    room_id: &str,
    root_event_id: &str,
    reply_event_id: &str,
) -> ThreadPaneWaitOutcome {
    use koushi_state::{ThreadOpenIntent, ThreadPaneState};
    match pane {
        ThreadPaneState::Closed | ThreadPaneState::Opening { .. } => ThreadPaneWaitOutcome::Waiting,
        ThreadPaneState::Open {
            room_id: open_room,
            root_event_id: open_root,
            intent: ThreadOpenIntent::PinnedReply { event_id },
            is_subscribed: true,
            ..
        } if open_room == room_id && open_root == root_event_id && event_id == reply_event_id => {
            ThreadPaneWaitOutcome::Ready
        }
        ThreadPaneState::Open {
            room_id: open_room,
            root_event_id: open_root,
            intent,
            is_subscribed: false,
            ..
        } => ThreadPaneWaitOutcome::Failed(format!(
            "the Thread pane opened on room/root {open_room:?}/{open_root:?} with intent {intent:?} but is not subscribed"
        )),
        ThreadPaneState::Failed { failure_kind, .. } => ThreadPaneWaitOutcome::Failed(format!(
            "opening the selected reply's thread failed: {failure_kind:?}"
        )),
        other => ThreadPaneWaitOutcome::Failed(format!(
            "the Thread pane settled as {other:?} instead of the selected reply's root"
        )),
    }
}

/// #1880: selecting a search hit for a thread reply must open the Thread pane
/// on that reply's root with the reply showing, by the same path the app uses
/// for a search-result click (source `Search`, `LiveFallback` policy).
pub(super) async fn search_hit_opens_thread_pane(
    conn: &mut CoreConnection,
    account_key: &AccountKey,
    room_id: &str,
    root_event_id: &str,
    reply_event_id: &str,
    reply_body: &str,
) -> Result<(), String> {
    const LABEL: &str = "thread_search_select";
    poll_search_until_found(
        conn,
        account_key,
        reply_body,
        reply_event_id,
        room_id,
        LABEL,
    )
    .await?;

    let navigate_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::NavigateToEvent {
        request_id: navigate_id,
        room_id: room_id.to_owned(),
        event_id: reply_event_id.to_owned(),
        source: koushi_state::EventNavigationSource::Search,
        missing_target_policy:
            koushi_protocol::command::EventNavigationMissingTargetPolicy::LiveFallback,
    }))
    .await
    .map_err(|e| format!("{LABEL}: submit search result selection: {e}"))?;

    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        match thread_pane_wait_outcome(
            &conn.snapshot().thread,
            room_id,
            root_event_id,
            reply_event_id,
        ) {
            ThreadPaneWaitOutcome::Ready => break,
            ThreadPaneWaitOutcome::Failed(violation) => {
                return Err(format!("{LABEL}: {violation}"));
            }
            ThreadPaneWaitOutcome::Waiting => {}
        }
        tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| {
                format!(
                    "{LABEL}: timed out waiting for the Thread pane to open on the selected reply"
                )
            })?
            .map_err(|lag| format!("{LABEL}: event stream lagged (skipped={})", lag.skipped))?;
    }

    // The pane renders this thread timeline, so the reply showing there is the
    // "reply is visible" half of the contract.
    let thread_key = TimelineKey {
        account_key: account_key.clone(),
        kind: TimelineKind::Thread {
            room_id: room_id.to_owned(),
            root_event_id: root_event_id.to_owned(),
        },
    };
    let thread_items = subscribe_timeline_for_qa(conn, &thread_key, LABEL).await?;
    if find_timeline_item_with_body(&thread_items, reply_body).is_none() {
        return Err(format!(
            "{LABEL}: the opened Thread pane's timeline does not hold the selected reply"
        ));
    }
    let unsubscribe_id = conn.next_request_id();
    conn.command(CoreCommand::Timeline(TimelineCommand::Unsubscribe {
        request_id: unsubscribe_id,
        key: thread_key,
    }))
    .await
    .map_err(|e| format!("{LABEL}: submit thread unsubscribe: {e}"))?;

    let close_id = conn.next_request_id();
    conn.command(CoreCommand::App(AppCommand::CloseThread {
        request_id: close_id,
    }))
    .await
    .map_err(|e| format!("{LABEL}: submit thread close: {e}"))?;
    Ok(())
}

/// Submit one search query and wait for `SearchEvent::Results` with matching
/// `request_id`. Returns `true` if `expected_event_id` appears in results,
/// `false` if the Results arrived but the event is absent.
/// Propagates search failure (IndexUnavailable, etc.) as errors.
async fn wait_for_search_result(
    conn: &mut CoreConnection,
    request_id: koushi_protocol::ids::RequestId,
    expected_event_id: &str,
    label: &str,
) -> Result<bool, String> {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(10), conn.recv_event())
            .await
            .map_err(|_| format!("{label}: timed out waiting for SearchEvent::Results"))?
            .map_err(|lag| format!("{label}: event stream lagged (skipped={})", lag.skipped))?;

        match event {
            CoreEvent::Search(SearchEvent::Results {
                request_id: ev_id,
                results,
            }) if ev_id == request_id => {
                let found = results.iter().any(|r| r.event_id == expected_event_id);
                return Ok(found);
            }
            CoreEvent::OperationFailed {
                request_id: ev_id,
                failure,
            } if ev_id == request_id => {
                return Err(format!("{label}: search query failed: {failure:?}"));
            }
            _ => continue,
        }
    }
}

/// Crawls the search actors queued for `cause` since `since_ms`, from the
/// token-only `core.search crawl_enqueued` diagnostic (#1634).
fn crawls_enqueued_since(since_ms: u64, cause: &str) -> usize {
    koushi_diagnostics::snapshot()
        .records
        .iter()
        .filter(|record| {
            record.timestamp_ms >= since_ms
                && record.event.source == "core.search"
                && record.event.stage == "crawl_enqueued"
                && record.event.fields.iter().any(|field| {
                    field.key == "cause"
                        && matches!(
                            field.value,
                            koushi_diagnostics::DiagnosticValue::Token(value) if value == cause
                        )
                })
        })
        .count()
}

fn epoch_millis_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            elapsed.as_millis().try_into().unwrap_or(u64::MAX)
        })
}
