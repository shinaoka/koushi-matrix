//! Scroll-back latency over history the local stores already hold.
//!
//! Users report a long blank wait when scrolling up in a room whose history
//! they have already seen. The SDK event cache persists that history and
//! back-pagination reads it from disk before it asks the server, so on a
//! loopback homeserver every page should settle quickly. This scenario
//! measures where the time actually goes.
//!
//! The fixture seeds a disposable encrypted room with a bounded number of
//! synthetic messages while sync runs, so the event cache persists them.
//! It then restarts the runtime on the same encrypted stores, restores the
//! session, resubscribes the room, and walks back to the start of the room the
//! way the renderer does: one backward `Paginate` request of
//! [`RENDERER_PAGE_EVENT_COUNT`] events at a time, the next one only after the
//! previous one settles.
//!
//! For each page it records, from content-free sources only:
//!
//! - the wall time from submitting the request to its terminal pagination
//!   state (what the user waits for),
//! - whether Core went to the SDK (`Paginating` seen) or only slid its bounded
//!   display window,
//! - the SDK pagination duration and the account work-gate wait from the
//!   `core.timeline` `paginate` diagnostic records,
//! - how many room `/messages` requests the QA proxy forwarded to the
//!   homeserver (zero means the page came from the local stores).
//!
//! It fails when any page takes longer than the per-page budget, so it stays
//! red until scrolling back through cached history is fast. No room or event
//! identifiers, message bodies, tokens, or local paths are printed; the
//! emitted values are durations and counts.

use koushi_diagnostics::{
    DiagnosticRecord,
    test_support::{DetailCursor, detail_cursor, detail_records_since},
};

use super::cleanup::cleanup_logged_in_runtime;
use super::diagnostics::{QaTcpProxy, diagnostic_token_field};
use super::event_wait::{
    stop_sync_for_qa, subscribe_timeline_for_qa, wait_for_event_item_with_body, wait_for_logged_in,
    wait_for_ready_snapshot, wait_for_send_flow_completion, wait_for_session_restored,
    wait_for_sync_started_and_running,
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
/// desktop timeline commands).
const RENDERER_PAGE_EVENT_COUNT: u16 = 100;

const DEFAULT_MESSAGES: usize = 300;
const MAX_MESSAGES: usize = 1_000;
/// Default per-page budget. A loopback homeserver answers `/messages` in tens
/// of milliseconds and a disk-served page needs no request at all, so a page
/// that takes longer than this is spent inside the client.
const DEFAULT_PAGE_BUDGET_MS: u64 = 500;
const MAX_PAGE_BUDGET_MS: u64 = 30_000;
/// Settlement after the restored subscription, so the first page is not
/// measured against the restore's own start-up work.
const RESTORE_SETTLEMENT: Duration = Duration::from_secs(5);

const ENV_MESSAGES: &str = "KOUSHI_QA_SCROLL_BACK_MESSAGES";
const ENV_PAGE_BUDGET_MS: &str = "KOUSHI_QA_SCROLL_BACK_PAGE_BUDGET_MS";

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
    // Each send completes before the next one, so the seed never overflows
    // the send queue (the same discipline as `timeline_stress`).
    for index in 0..messages {
        let transaction_id = format!("scroll-back-{index}");
        let body = seed_body(index);
        let send_id = conn.next_request_id();
        conn.command_with_admission(CoreCommand::Timeline(TimelineCommand::SendText {
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
    wait_for_event_item_with_body(
        &mut conn,
        &key,
        &seed_body(messages - 1),
        "scroll back last seed body",
    )
    .await?;
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
    drain_for(&mut restored, RESTORE_SETTLEMENT).await?;
    eprintln!("{LABEL}: restored initial_items={}", initial.len());
    println!("scroll_back_restored=ok");

    // 3. Walk back to the start of the room one renderer page at a time.
    let max_pages = messages / usize::from(RENDERER_PAGE_EVENT_COUNT) * 4 + 20;
    let mut samples = Vec::new();
    let walk_started = std::time::Instant::now();
    loop {
        if samples.len() >= max_pages {
            print_samples(&samples);
            return Err(format!(
                "{LABEL}: no EndReached after {max_pages} backward pages"
            ));
        }
        let sample = paginate_one_page(&mut restored, &proxy, &key).await?;
        let end = sample.terminal == PageTerminal::EndReached;
        samples.push(sample);
        if end {
            break;
        }
    }
    let walk_ms = walk_started.elapsed().as_millis() as u64;
    print_samples(&samples);

    let sdk_pages: Vec<&PageSample> = samples.iter().filter(|s| s.reached_sdk).collect();
    let network_pages = samples
        .iter()
        .filter(|s| s.room_messages_requests > 0)
        .count();
    let room_messages_requests: usize = samples.iter().map(|s| s.room_messages_requests).sum();
    let gap_repair_skips = samples.iter().filter(|s| s.skipped_for_gap_repair).count();
    let (wall_p50, wall_p95, wall_max) = percentiles(samples.iter().map(|s| s.wall_ms).collect());
    let (sdk_p50, sdk_p95, sdk_max) = percentiles(sdk_pages.iter().map(|s| s.sdk_ms).collect());
    let (gate_p50, gate_p95, gate_max) =
        percentiles(sdk_pages.iter().map(|s| s.gate_wait_ms).collect());
    eprintln!(
        "{LABEL}: pages={} sdk_pages={} window_only_pages={} network_pages={network_pages} \
         room_messages_requests={room_messages_requests} gap_repair_skips={gap_repair_skips} \
         walk_ms={walk_ms} wall p50={wall_p50}ms p95={wall_p95}ms max={wall_max}ms \
         sdk p50={sdk_p50}ms p95={sdk_p95}ms max={sdk_max}ms \
         gate_wait p50={gate_p50}ms p95={gate_p95}ms max={gate_max}ms budget={page_budget_ms}ms",
        samples.len(),
        sdk_pages.len(),
        samples.len() - sdk_pages.len(),
    );
    println!("scroll_back_reached_start=ok");

    let over_budget = samples
        .iter()
        .filter(|s| s.wall_ms > page_budget_ms)
        .count();
    if over_budget > 0 {
        return Err(format!(
            "{LABEL}: {over_budget} of {} backward pages took longer than {page_budget_ms} ms \
             (max {wall_max} ms, network_pages={network_pages}, gate_wait max {gate_max} ms)",
            samples.len()
        ));
    }
    println!("scroll_back_page_budget=ok");
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

/// Consume events for a fixed settlement window without asserting on them.
async fn drain_for(conn: &mut CoreConnection, window: Duration) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + window;
    loop {
        match tokio::time::timeout_at(deadline, conn.recv_event()).await {
            Err(_) => return Ok(()),
            Ok(Ok(_)) => {}
            Ok(Err(lag)) => {
                return Err(format!(
                    "{LABEL}: event stream lagged during settlement (skipped={})",
                    lag.skipped
                ));
            }
        }
    }
}

/// Submit one backward request and wait for its own terminal state.
async fn paginate_one_page(
    conn: &mut CoreConnection,
    proxy: &QaTcpProxy,
    key: &TimelineKey,
) -> Result<PageSample, String> {
    let messages_before = proxy.room_messages_forwarded_count()?;
    let cursor = detail_cursor();
    let request_id = conn.next_request_id();
    let started = std::time::Instant::now();
    conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
        request_id,
        key: key.clone(),
        direction: PaginationDirection::Backward,
        event_count: RENDERER_PAGE_EVENT_COUNT,
    }))
    .await
    .map_err(|e| format!("{LABEL}: submit pagination failed: {e}"))?;

    let deadline = tokio::time::Instant::now() + EVENT_TIMEOUT;
    let mut reached_sdk = false;
    let terminal = loop {
        let event = tokio::time::timeout_at(deadline, conn.recv_event())
            .await
            .map_err(|_| {
                format!(
                    "{LABEL}: timed out waiting for a terminal pagination state (sdk={reached_sdk})"
                )
            })?
            .map_err(|lag| format!("{LABEL}: event stream lagged (skipped={})", lag.skipped))?;
        match event {
            CoreEvent::OperationFailed {
                request_id: failed,
                failure,
            } if failed == request_id => {
                return Err(format!("{LABEL}: pagination operation failed: {failure:?}"));
            }
            CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                request_id: Some(id),
                key: event_key,
                direction: PaginationDirection::Backward,
                state,
                ..
            }) if id == request_id && event_key == *key => match state {
                PaginationState::Paginating => reached_sdk = true,
                PaginationState::Idle => break PageTerminal::Idle,
                PaginationState::EndReached => break PageTerminal::EndReached,
                PaginationState::Failed { kind } => {
                    return Err(format!("{LABEL}: pagination failed: {kind:?}"));
                }
            },
            _ => {}
        }
    };
    let wall_ms = started.elapsed().as_millis() as u64;
    let records = paginate_records_since(cursor, request_id);
    let sdk_ms = stage_ms(&records, "sdk_finish", "duration");
    let gate_wait_ms = stage_ms(&records, "gate_acquired", "gate_wait");
    let skipped_for_gap_repair = records.iter().any(|record| {
        record.event.stage == "actor_paginate_skip"
            && diagnostic_token_field(&record.event, "outcome") == Some("other")
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
    })
}

/// The `core.timeline` `paginate` records this request produced.
fn paginate_records_since(cursor: DetailCursor, request_id: RequestId) -> Vec<DiagnosticRecord> {
    detail_records_since(cursor)
        .into_iter()
        .filter(|record| {
            record.event.source == "core.timeline"
                && diagnostic_token_field(&record.event, "kind") == Some("paginate")
                && record_request_id(&record.event)
                    == Some((request_id.connection_id.0, request_id.sequence))
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
             room_messages_requests={} gap_repair_skip={}",
            sample.wall_ms,
            sample.reached_sdk,
            match sample.terminal {
                PageTerminal::Idle => "idle",
                PageTerminal::EndReached => "end_reached",
            },
            sample.sdk_ms,
            sample.gate_wait_ms,
            sample.room_messages_requests,
            sample.skipped_for_gap_repair,
        );
    }
}
