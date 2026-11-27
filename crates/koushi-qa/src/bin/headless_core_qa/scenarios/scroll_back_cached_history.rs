//! Scroll-back latency over history the local stores already hold.
//!
//! Users report a long blank wait when scrolling up in a room whose history
//! they have already seen. The SDK event cache persists that history and
//! back-pagination reads it from disk before it asks the server, so on a
//! loopback homeserver a page that the cache covers should settle quickly.
//!
//! The fixture seeds a disposable encrypted room with a bounded number of
//! synthetic messages while sync runs, so the event cache persists them. It
//! then restarts the runtime on the same encrypted stores, restores the
//! session, resubscribes the room, and walks back to the start of the room the
//! way the renderer does: one backward `Paginate` request of
//! [`RENDERER_PAGE_EVENT_COUNT`] events at a time, the next one only after the
//! previous one settles.
//!
//! What this measures, and what it cannot:
//!
//! - The SDK keeps only the chunk it reloaded last in memory and reads an older
//!   chunk from the store on demand. Whether one request is served from the
//!   store or goes to `/messages` therefore depends on where the SDK's chunk
//!   boundaries fall, which the seed's sequential sends do not control. Pages
//!   are classified afterwards from the forwarded request count instead of being
//!   arranged.
//! - No viewport observation is sent, so a request Core answers by sliding its
//!   bounded display window never reaches the SDK. Those pages are counted
//!   separately and take no part in the latency distribution: the claim this
//!   lane makes is about **SDK page latency**, not renderer demand.
//!
//! Per page it records, from content-free sources only:
//!
//! - the wall time from submitting the request to its terminal pagination
//!   state (what the user waits for),
//! - whether Core went to the SDK (`Paginating` seen) or only slid its bounded
//!   display window,
//! - the SDK pagination duration and the account work-gate wait from the
//!   `core.timeline` `paginate` diagnostic records,
//! - how many room `/messages` requests the QA proxy forwarded to the
//!   homeserver (zero on a page that reached the SDK means the local store
//!   served it).
//!
//! Gate policy: a wall-clock page budget is **reported, not enforced**. The CI
//! profile is `--profile=ci` (no optimisation, debug assertions) on a small
//! two-vCPU runner with a containerised homeserver, so an absolute page time
//! there measures the runner as much as the client. The lane fails only on a
//! runaway — a page that never settles, or one slower than
//! [`DEFAULT_RUNAWAY_PAGE_MS`] — and on losing the store path it exists to
//! prove: at least one page must have been served by the local store without a
//! forwarded `/messages` request, unless the walk forwarded none at all. The
//! measured distribution is printed every run so a regression is visible as a
//! distribution shift, and the recorded baseline lives in the worklog.
//!
//! The forwarded request count is a window delta over the proxy counter, not
//! attribution: a `/messages` the restore's backfill or a gap repair sent while
//! the page was waiting is charged to that page. A page can therefore read as
//! network when the store served it; the gate needs one clean page out of the
//! walk rather than every page clean, and the count is printed to make the
//! difference visible.
//!
//! No room or event identifiers, message bodies, tokens, or local paths are
//! printed; the emitted values are durations and counts.

use koushi_diagnostics::{
    DiagnosticRecord,
    test_support::{DetailCursor, detail_cursor, detail_records_since},
};

use super::cleanup::cleanup_logged_in_runtime;
use super::diagnostics::{QaTcpProxy, diagnostic_token_field};
use super::event_wait::{
    stop_sync_for_qa, subscribe_timeline_for_qa, wait_for_logged_in, wait_for_ready_snapshot,
    wait_for_send_flow_completion, wait_for_session_restored, wait_for_sync_started_and_running,
};
use super::fixtures::create_room_for_qa;
use super::participants::{complete_new_identity_gate_for_qa, qa_data_dir};
use super::registry::{EVENT_TIMEOUT, QaConfig};
use super::{
    AccountCommand, AccountKey, AuthSecret, CoreCommand, CoreConnection, CoreEvent, CoreRuntime,
    Duration, PaginationDirection, PaginationState, RequestId, SyncCommand, TimelineCommand,
    TimelineEvent, TimelineKey,
};

const LABEL: &str = "scroll_back_cached_history";
const ROOM_NAME: &str = "Synthetic Scroll Back History";

/// The renderer's backward page size (`TIMELINE_PAGE_EVENT_COUNT` in the
/// desktop timeline commands). `scripts/check-rust-test-structure.mjs` fails
/// when the two drift apart.
const RENDERER_PAGE_EVENT_COUNT: u16 = 100;

const DEFAULT_MESSAGES: usize = 300;
const MAX_MESSAGES: usize = 1_000;
/// Reported, never enforced: the page budget whose excess is printed as
/// `over_budget`. See the gate policy above.
const DEFAULT_PAGE_BUDGET_MS: u64 = 500;
const MAX_PAGE_BUDGET_MS: u64 = 30_000;
/// A page this slow is a hang rather than a slow page, and fails the lane.
const DEFAULT_RUNAWAY_PAGE_MS: u64 = 10_000;
const MAX_RUNAWAY_PAGE_MS: u64 = 120_000;
/// Quiet window with no timeline activity and no pagination in flight, which is
/// what "the restore has finished" means observably. A fixed sleep would race
/// the restore's own hydration and turn an in-flight pagination into a timeout.
const TIMELINE_IDLE_QUIET: Duration = Duration::from_millis(750);
/// How long a submitted page may stay silent before Core is presumed to have
/// refused it (it drops a `Paginate` while another pagination holds the
/// timeline, without answering the request), after which the page is retried
/// once the timeline is idle again.
const PAGE_SILENCE_PROBE: Duration = Duration::from_secs(2);

const ENV_MESSAGES: &str = "KOUSHI_QA_SCROLL_BACK_MESSAGES";
const ENV_PAGE_BUDGET_MS: &str = "KOUSHI_QA_SCROLL_BACK_PAGE_BUDGET_MS";
const ENV_RUNAWAY_PAGE_MS: &str = "KOUSHI_QA_SCROLL_BACK_RUNAWAY_MS";

fn bounded_env(name: &str, default: u64, max: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(|value| value.min(max))
        .unwrap_or(default)
}

fn seed_body(index: usize) -> String {
    format!("Synthetic scroll back body {index}")
}

/// How one backward request settled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PageTerminal {
    Idle,
    EndReached,
}

#[derive(Debug)]
struct PageSample {
    wall_ms: u64,
    /// Core reported `Paginating`, so the SDK ran a pagination.
    reached_sdk: bool,
    terminal: PageTerminal,
    sdk_ms: u64,
    gate_wait_ms: u64,
    room_messages_requests: usize,
    /// Core skipped the request because a gap repair held the timeline.
    skipped_for_gap_repair: bool,
    /// Submissions this page needed; more than one means Core refused an earlier
    /// one while another pagination held the timeline.
    attempts: usize,
}

impl PageSample {
    /// The page reached the SDK and forwarded no `/messages` request, so the
    /// local store served it.
    fn store_served(&self) -> bool {
        self.reached_sdk && self.room_messages_requests == 0
    }
}

pub(super) async fn run_scroll_back_cached_history_scenario(
    config: &QaConfig,
) -> Result<(), String> {
    let messages =
        (bounded_env(ENV_MESSAGES, DEFAULT_MESSAGES as u64, MAX_MESSAGES as u64) as usize).max(1);
    let page_budget_ms = bounded_env(
        ENV_PAGE_BUDGET_MS,
        DEFAULT_PAGE_BUDGET_MS,
        MAX_PAGE_BUDGET_MS,
    );
    let runaway_page_ms = bounded_env(
        ENV_RUNAWAY_PAGE_MS,
        DEFAULT_RUNAWAY_PAGE_MS,
        MAX_RUNAWAY_PAGE_MS,
    );

    let proxy = QaTcpProxy::start(&config.homeserver)
        .map_err(|_| format!("{LABEL}: proxy setup failed"))?;
    let proxied_config = config.with_homeserver(proxy.homeserver_url());
    let data_dir = qa_data_dir("scroll-back-cached-history");

    // 1. Seed: every message arrives through sync, so the event cache persists it.
    let runtime = CoreRuntime::start_with_data_dir(data_dir.clone());
    let mut conn = runtime.attach();
    let login_id = conn.next_request_id();
    conn.command(CoreCommand::Account(AccountCommand::LoginPassword {
        request_id: login_id,
        request: koushi_state::LoginRequest {
            homeserver: proxied_config.homeserver.clone(),
            username: proxied_config.user_a.clone(),
            password: AuthSecret::new(proxied_config.password_a.clone()),
            device_display_name: Some("Koushi Core QA Scroll Back".to_owned()),
        },
        platform: koushi_state::DisplayPlatform::Linux,
    }))
    .await
    .map_err(|e| format!("{LABEL}: submit login failed: {e}"))?;
    complete_new_identity_gate_for_qa(&mut conn, &proxied_config.password_a).await?;
    let account_key: AccountKey =
        wait_for_logged_in(&mut conn, login_id, "scroll back login").await?;
    wait_for_ready_snapshot(&mut conn, "scroll back session Ready").await?;
    start_sync(&mut conn, "scroll back seed sync").await?;

    let room_id = create_room_for_qa(&mut conn, ROOM_NAME, true, "scroll back create room").await?;
    let key = TimelineKey::room(account_key.clone(), room_id);
    subscribe_timeline_for_qa(&mut conn, &key, "scroll back seed subscribe").await?;
    // Each send completes -- with its own event id -- before the next one, so the
    // seed never overflows the send queue (the same discipline as
    // `timeline_stress`). That completion is the seed's evidence: the row has
    // already been projected, so waiting again for the last body afterwards races
    // the projection that carried it and can only time out.
    for index in 0..messages {
        let transaction_id = format!("scroll-back-{index}");
        let body = seed_body(index);
        let send_id = conn.next_request_id();
        conn.command_with_admission(CoreCommand::Timeline(TimelineCommand::SendText {
            explicit_mentions: None,
            request_id: send_id,
            key: key.clone(),
            transaction_id: transaction_id.clone(),
            document: koushi_state::ComposerDocument::from_plain_text(body.clone()),
        }))
        .await
        .map_err(|e| format!("{LABEL}: submit seed send failed: {e}"))?;
        wait_for_send_flow_completion(
            &mut conn,
            send_id,
            &key,
            &transaction_id,
            &body,
            "scroll back seed send",
        )
        .await?;
    }
    eprintln!("{LABEL}: seeded messages={messages}");
    println!("scroll_back_seeded=ok");

    // 2. Restart on the same encrypted stores, as after an app relaunch.
    stop_sync_for_qa(&mut conn, "scroll back stop before restart").await?;
    drop(conn);
    tokio::time::timeout(EVENT_TIMEOUT, runtime.shutdown())
        .await
        .map_err(|_| format!("{LABEL}: timed out shutting down the seeding runtime"))?;

    let restored_runtime = CoreRuntime::start_with_data_dir(data_dir);
    let mut restored = restored_runtime.attach();
    let restore_id = restored.next_request_id();
    restored
        .command(CoreCommand::Account(AccountCommand::RestoreSession {
            request_id: restore_id,
            account_key: account_key.clone(),
        }))
        .await
        .map_err(|e| format!("{LABEL}: submit restore failed: {e}"))?;
    wait_for_session_restored(
        &mut restored,
        restore_id,
        &account_key,
        "scroll back restore",
    )
    .await?;
    wait_for_ready_snapshot(&mut restored, "scroll back restored session Ready").await?;
    start_sync(&mut restored, "scroll back restored sync").await?;
    let initial = subscribe_timeline_for_qa(&mut restored, &key, "scroll back subscribe").await?;
    // The restore hydrates and backfills on its own; wait until that is
    // observably finished instead of sleeping for a guessed duration, so a slow
    // runner waits and a fast one does not waste the window.
    wait_for_timeline_idle(&mut restored, &key).await?;
    eprintln!("{LABEL}: restored initial_items={}", initial.len());
    println!("scroll_back_restored=ok");

    // 3. Walk back to the start of the room one renderer page at a time.
    let max_pages = messages / usize::from(RENDERER_PAGE_EVENT_COUNT) * 4 + 20;
    let mut samples: Vec<PageSample> = Vec::new();
    let walk_started = std::time::Instant::now();
    loop {
        // A page Core skipped because a gap repair held the timeline settles
        // instantly and answered nothing, so it is not progress and must not
        // spend the budget: a common gap repair would otherwise exhaust it.
        let skipped = samples.iter().filter(|s| s.skipped_for_gap_repair).count();
        let progressed = samples.len() - skipped;
        if progressed >= max_pages || skipped >= max_pages {
            print_samples(&samples);
            return Err(format!(
                "{LABEL}: no EndReached after {progressed} backward pages ({skipped} refused \
                 while a gap repair held the timeline)"
            ));
        }
        let sample = paginate_one_page(&mut restored, &proxy, &key).await?;
        let end = sample.terminal == PageTerminal::EndReached;
        let skipped_page = sample.skipped_for_gap_repair;
        samples.push(sample);
        if end {
            break;
        }
        if skipped_page {
            // Wait for the gap repair to finish, so the next request is not
            // skipped again immediately.
            wait_for_timeline_idle(&mut restored, &key).await?;
        }
    }
    let walk_ms = walk_started.elapsed().as_millis() as u64;
    print_samples(&samples);

    let sdk_pages: Vec<&PageSample> = samples.iter().filter(|s| s.reached_sdk).collect();
    let store_served_pages = samples.iter().filter(|s| s.store_served()).count();
    let network_pages = samples
        .iter()
        .filter(|s| s.room_messages_requests > 0)
        .count();
    let room_messages_requests: usize = samples.iter().map(|s| s.room_messages_requests).sum();
    let gap_repair_skips = samples.iter().filter(|s| s.skipped_for_gap_repair).count();
    let refused_attempts: usize = samples.iter().map(|s| s.attempts.saturating_sub(1)).sum();
    // The claim is about SDK pages; a window-only page is reported but never
    // mixed into it.
    let (wall_p50, wall_p95, wall_max) = percentiles(sdk_pages.iter().map(|s| s.wall_ms).collect());
    let (all_p50, all_p95, all_max) = percentiles(samples.iter().map(|s| s.wall_ms).collect());
    let (sdk_p50, sdk_p95, sdk_max) = percentiles(sdk_pages.iter().map(|s| s.sdk_ms).collect());
    let (gate_p50, gate_p95, gate_max) =
        percentiles(sdk_pages.iter().map(|s| s.gate_wait_ms).collect());
    let over_budget = samples
        .iter()
        .filter(|s| s.wall_ms > page_budget_ms)
        .count();
    let over_runaway = samples
        .iter()
        .filter(|s| s.wall_ms > runaway_page_ms)
        .count();
    eprintln!(
        "{LABEL}: pages={} sdk_pages={} window_only_pages={} store_served_pages={store_served_pages} \
         network_pages={network_pages} room_messages_requests={room_messages_requests} \
         gap_repair_skips={gap_repair_skips} refused_attempts={refused_attempts} \
         walk_ms={walk_ms} \
         sdk_page_wall p50={wall_p50}ms p95={wall_p95}ms max={wall_max}ms \
         sdk_page_sdk p50={sdk_p50}ms p95={sdk_p95}ms max={sdk_max}ms \
         sdk_page_gate_wait p50={gate_p50}ms p95={gate_p95}ms max={gate_max}ms \
         all_page_wall p50={all_p50}ms p95={all_p95}ms max={all_max}ms \
         budget={page_budget_ms}ms over_budget={over_budget} runaway={runaway_page_ms}ms",
        samples.len(),
        sdk_pages.len(),
        samples.len() - sdk_pages.len(),
    );
    println!("scroll_back_reached_start=ok");

    // The lane exists to prove the local store serves this history. Without a
    // store-served page (or a walk that never asked the network at all) a green
    // token would only say that pagination finished.
    if store_served_pages == 0 && room_messages_requests > 0 {
        return Err(format!(
            "{LABEL}: no page was served by the local store (pages={}, sdk_pages={}, \
             room_messages_requests={room_messages_requests}); the scroll-back cache path was \
             not exercised",
            samples.len(),
            sdk_pages.len(),
        ));
    }
    println!("scroll_back_store_served=ok");

    // A runaway (a page that never settled, or one slower than the runaway
    // threshold) is a real failure; the ordinary page budget is only reported.
    if over_runaway > 0 {
        return Err(format!(
            "{LABEL}: {over_runaway} of {} backward pages took longer than {runaway_page_ms} ms \
             (max {all_max} ms, network_pages={network_pages}, sdk_pages={})",
            samples.len(),
            sdk_pages.len(),
        ));
    }
    println!("scroll_back_no_runaway=ok");
    println!("scroll_back_cached_history=ok");

    cleanup_logged_in_runtime(
        restored,
        restored_runtime,
        account_key,
        "scroll back cleanup",
    )
    .await?;
    Ok(())
}

async fn start_sync(conn: &mut CoreConnection, label: &str) -> Result<(), String> {
    let sync_id = conn.next_request_id();
    conn.command(CoreCommand::Sync(SyncCommand::Start {
        request_id: sync_id,
    }))
    .await
    .map_err(|e| format!("{label}: submit sync start failed: {e}"))?;
    wait_for_sync_started_and_running(conn, sync_id, label).await
}

/// Wait until the timeline is observably idle: nothing is paginating and no
/// event for the key has arrived for [`TIMELINE_IDLE_QUIET`].
///
/// This replaces a fixed settlement sleep. The restore's hydration and backfill
/// paginations are the reason: Core drops a `Paginate` while one of them holds
/// the timeline and answers nothing, so a page submitted too early would wait
/// out the whole event timeout and look like a hang.
///
/// The in-flight count only sees paginations that start after the wait begins; a
/// pagination already running when it is called is covered by the caller's
/// retry, which is bounded by the page deadline and not by an attempt count.
async fn wait_for_timeline_idle(
    conn: &mut CoreConnection,
    key: &TimelineKey,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    let mut outstanding = 0usize;
    let mut last_activity = tokio::time::Instant::now();
    loop {
        let quiet_deadline = last_activity + TIMELINE_IDLE_QUIET;
        match tokio::time::timeout_at(quiet_deadline.min(deadline), conn.recv_event()).await {
            Ok(Ok(CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                key: event_key,
                state,
                ..
            }))) if event_key == *key => {
                match state {
                    PaginationState::Paginating => outstanding += 1,
                    _ => outstanding = outstanding.saturating_sub(1),
                }
                last_activity = tokio::time::Instant::now();
            }
            Ok(Ok(CoreEvent::Timeline(TimelineEvent::ItemsUpdated { key: event_key, .. })))
                if event_key == *key =>
            {
                last_activity = tokio::time::Instant::now();
            }
            Ok(Ok(_)) => {}
            Ok(Err(lag)) => {
                return Err(format!(
                    "{LABEL}: event stream lagged while waiting for the timeline to settle \
                     (skipped={})",
                    lag.skipped
                ));
            }
            Err(_) if tokio::time::Instant::now() >= deadline => {
                return Err(format!(
                    "{LABEL}: the timeline never settled after the restore \
                     (paginations_in_flight={outstanding})"
                ));
            }
            Err(_) if outstanding == 0 => return Ok(()),
            Err(_) => {
                // Quiet, but a pagination is still in flight: keep waiting for
                // the event that settles it.
                last_activity = tokio::time::Instant::now();
            }
        }
    }
}

/// Submit one backward request and wait for its own terminal state.
///
/// A request Core refuses (another pagination holds the timeline) produces no
/// event at all, so silence is resolved by retrying once the timeline is idle
/// again rather than by failing the page.
async fn paginate_one_page(
    conn: &mut CoreConnection,
    proxy: &QaTcpProxy,
    key: &TimelineKey,
) -> Result<PageSample, String> {
    let messages_before = proxy.room_messages_forwarded_count()?;
    let cursor = detail_cursor();
    let started = std::time::Instant::now();
    let first_request_id = conn.next_request_id();
    let mut attempts = 0usize;
    let mut reached_sdk = false;
    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    // Request ids are monotonic per connection and this page allocates none
    // besides its own, so a pagination state from this page's request ids is
    // distinguishable from the restore's own projection request, which predates
    // them.
    let accept = |id: RequestId| {
        id.connection_id.0 == first_request_id.connection_id.0
            && id.sequence >= first_request_id.sequence
    };
    // No attempt cap: a page waiting on the account work gate emits nothing
    // until it is admitted, and every further attempt is dropped as `in_flight`,
    // so a counted refusal says nothing about the queued one. The page deadline
    // bounds the loop instead.
    let terminal = 'page: loop {
        attempts += 1;
        let request_id = conn.next_request_id();
        conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
            request_id,
            key: key.clone(),
            direction: PaginationDirection::Backward,
            event_count: RENDERER_PAGE_EVENT_COUNT,
        }))
        .await
        .map_err(|e| format!("{LABEL}: submit pagination failed: {e}"))?;

        let mut attempt_saw_event = false;
        let mut silence_deadline = tokio::time::Instant::now() + PAGE_SILENCE_PROBE;
        loop {
            match tokio::time::timeout_at(silence_deadline.min(deadline), conn.recv_event()).await {
                Err(_) if tokio::time::Instant::now() >= deadline => {
                    return Err(format!(
                        "{LABEL}: timed out waiting for a terminal pagination state (sdk={reached_sdk})"
                    ));
                }
                Err(_) if attempt_saw_event => {
                    // The request is running, it just outlived the probe: wait
                    // for its own terminal state instead of resubmitting.
                    silence_deadline = deadline;
                }
                Err(_) => break,
                Ok(Err(lag)) => {
                    return Err(format!(
                        "{LABEL}: event stream lagged (skipped={})",
                        lag.skipped
                    ));
                }
                Ok(Ok(event)) => match event {
                    CoreEvent::OperationFailed {
                        request_id: failed,
                        failure,
                    } if accept(failed) => {
                        return Err(format!("{LABEL}: pagination operation failed: {failure:?}"));
                    }
                    CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                        request_id: Some(id),
                        key: event_key,
                        direction: PaginationDirection::Backward,
                        state,
                        ..
                    }) if event_key == *key && accept(id) => {
                        attempt_saw_event = true;
                        silence_deadline = tokio::time::Instant::now() + PAGE_SILENCE_PROBE;
                        match state {
                            PaginationState::Paginating => reached_sdk = true,
                            PaginationState::Idle => break 'page PageTerminal::Idle,
                            PaginationState::EndReached => break 'page PageTerminal::EndReached,
                            PaginationState::Failed { kind } => {
                                return Err(format!("{LABEL}: pagination failed: {kind:?}"));
                            }
                        }
                    }
                    _ => {}
                },
            }
        }
        // Nothing at all came back: Core dropped the request. Only retry once the
        // timeline is idle again, so the retry is not dropped as well.
        wait_for_timeline_idle(conn, key).await?;
    };
    let wall_ms = started.elapsed().as_millis() as u64;
    let records = paginate_records_since(cursor, first_request_id);
    let sdk_ms = stage_ms(&records, "sdk_finish", "duration");
    let gate_wait_ms = stage_ms(&records, "gate_acquired", "gate_wait");
    let skipped_for_gap_repair = records.iter().any(|record| {
        record.event.stage == "actor_paginate_skip"
            && diagnostic_token_field(&record.event, "outcome") == Some("gap_repair")
    });
    let room_messages_requests = proxy
        .room_messages_forwarded_count()?
        .saturating_sub(messages_before);
    Ok(PageSample {
        wall_ms,
        reached_sdk,
        terminal,
        sdk_ms,
        gate_wait_ms,
        room_messages_requests,
        skipped_for_gap_repair,
        attempts,
    })
}

/// The `core.timeline` `paginate` records this page produced: everything
/// recorded after the cursor for a request this page issued.
fn paginate_records_since(
    cursor: DetailCursor,
    first_request_id: RequestId,
) -> Vec<DiagnosticRecord> {
    detail_records_since(cursor)
        .into_iter()
        .filter(|record| {
            record.event.source == "core.timeline"
                && diagnostic_token_field(&record.event, "kind") == Some("paginate")
                && record_request_id(&record.event).is_some_and(|(connection_id, sequence)| {
                    connection_id == first_request_id.connection_id.0
                        && sequence >= first_request_id.sequence
                })
        })
        .collect()
}

fn record_request_id(event: &koushi_diagnostics::DiagnosticEvent) -> Option<(u64, u64)> {
    event.fields.iter().find_map(|field| match field.value {
        koushi_diagnostics::DiagnosticValue::RequestId {
            connection_id,
            sequence,
        } if field.key == "request_id" => Some((connection_id, sequence)),
        _ => None,
    })
}

fn stage_ms(records: &[DiagnosticRecord], stage: &str, field: &str) -> u64 {
    records
        .iter()
        .filter(|record| record.event.stage == stage)
        .filter_map(|record| {
            record.event.fields.iter().find_map(|f| match f.value {
                koushi_diagnostics::DiagnosticValue::Milliseconds(value) if f.key == field => {
                    Some(value)
                }
                _ => None,
            })
        })
        .max()
        .unwrap_or(0)
}

fn percentiles(mut samples: Vec<u64>) -> (u64, u64, u64) {
    if samples.is_empty() {
        return (0, 0, 0);
    }
    samples.sort_unstable();
    let p50 = samples[samples.len() / 2];
    let p95 = samples[((samples.len() * 95) / 100).min(samples.len() - 1)];
    (p50, p95, samples[samples.len() - 1])
}

fn print_samples(samples: &[PageSample]) {
    for (index, sample) in samples.iter().enumerate() {
        eprintln!(
            "{LABEL}: page={index} wall_ms={} sdk={} terminal={} sdk_ms={} gate_wait_ms={} \
             room_messages_requests={} store_served={} gap_repair_skip={} attempts={}",
            sample.wall_ms,
            sample.reached_sdk,
            match sample.terminal {
                PageTerminal::Idle => "idle",
                PageTerminal::EndReached => "end_reached",
            },
            sample.sdk_ms,
            sample.gate_wait_ms,
            sample.room_messages_requests,
            sample.store_served(),
            sample.skipped_for_gap_repair,
            sample.attempts,
        );
    }
}
