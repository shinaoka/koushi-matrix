//! Search history crawler: pages older room events through the room's persisted
//! event chunks, decrypts them locally, indexes them in the encrypted ngram
//! index, and forwards Files metadata to the body-free document store.
//!
//! Reading persisted chunks instead of driving shared cache pagination is what
//! keeps indexing from hydrating history: `run_backwards_once` inserts every
//! page it reads into the shared room-timeline linked chunk, which left every
//! crawled room's whole history resident. A room whose persisted history has a
//! gap still falls back to SDK back-pagination, which is the path that reaches
//! `/messages`, because that history is not local yet.
//!
//! Each page awaits its index commit before being reported successful. A crawl
//! checkpoint is in-session only: what survives a restart is the committed
//! *completion* of a room (see `crate::store::search_progress`), not an
//! interrupted walk's position.
//!
//! Media file bytes are never fetched; only MXC URIs, filenames, captions and
//! metadata are indexed. This keeps the crawler a text-only backfill worker.

use std::collections::HashSet;
use std::sync::Arc;

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_search::{AttachmentDocument, SensitiveString};
use koushi_state::{
    AttachmentKind, SearchCrawlerFailureKind, SearchCrawlerSettings, SearchCrawlerSpeed,
};
use serde_json::Value;

use crate::account_work::{AccountWorkKind, AccountWorkPermit, AccountWorkScheduler};
use crate::executor;
use crate::search::SearchIndexMessage;
use crate::startup_trace::{self, StartupPhase};

const BATCH_SIZE_FAST: u32 = 200;
const BATCH_SIZE_STANDARD: u32 = 100;
const BATCH_SIZE_SLOW: u32 = 50;

/// Where one crawl reads its history from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CrawlHistorySource {
    /// The room's persisted event chunks, walked newest chunk first. Never
    /// inserts into the shared room-timeline linked chunk and never reaches the
    /// network.
    Store,
    /// SDK back-pagination. Reaches `/messages` at a gap and inserts what it
    /// reads into the shared linked chunk, which is the only way to index
    /// history the local store does not hold.
    Network,
}

#[derive(Clone)]
pub(crate) struct HistoryCrawlCheckpoint {
    pub room_id: String,
    pub processed: u64,
    pub indexed: u64,
    pub pending_redactions: HashSet<String>,
    pub settings: SearchCrawlerSettings,
    pub settings_generation: u64,
    pub manual: bool,
    /// True until the crawl's first page has run.
    pub first_page: bool,
    /// Catch-up crawl of a completed room (#996): the latest event id recorded
    /// when the room last completed. The crawl indexes only what is newer and
    /// completes once it reaches this event.
    pub catch_up_after: Option<String>,
    /// The room's latest event id (from the room list) when the first page
    /// started; recorded as the room's catch-up boundary on completion.
    pub latest_event_id_at_start: Option<String>,
    /// Where this crawl is reading from.
    pub history_source: CrawlHistorySource,
    /// Next persisted chunk to read, while the source is
    /// [`CrawlHistorySource::Store`].
    pub persisted_cursor: Option<koushi_sdk::MatrixPersistedEventCursor>,
    /// Whether the store walk has crossed a gap chunk, i.e. the local store does
    /// not hold the whole history between the newest event and the oldest chunk
    /// walked so far.
    pub crossed_store_gap: bool,
}

impl HistoryCrawlCheckpoint {
    pub fn new(
        room_id: String,
        settings: SearchCrawlerSettings,
        settings_generation: u64,
        manual: bool,
    ) -> Self {
        Self {
            room_id,
            processed: 0,
            indexed: 0,
            pending_redactions: HashSet::new(),
            settings,
            settings_generation,
            manual,
            first_page: true,
            catch_up_after: None,
            latest_event_id_at_start: None,
            history_source: CrawlHistorySource::Store,
            persisted_cursor: None,
            crossed_store_gap: false,
        }
    }

    /// Catch-up checkpoint for a completed room whose latest event changed
    /// (#996). Counts carry over so the room row keeps its totals.
    pub fn catch_up(
        room_id: String,
        settings: SearchCrawlerSettings,
        settings_generation: u64,
        after_event_id: String,
        processed: u64,
        indexed: u64,
    ) -> Self {
        Self {
            processed,
            indexed,
            catch_up_after: Some(after_event_id),
            ..Self::new(room_id, settings, settings_generation, false)
        }
    }
}

pub(crate) enum HistoryCrawlPageResult {
    Success {
        checkpoint: HistoryCrawlCheckpoint,
        messages: Vec<SearchIndexMessage>,
        completed: bool,
        work_permit: Option<AccountWorkPermit>,
    },
    Failed {
        checkpoint: HistoryCrawlCheckpoint,
        kind: SearchCrawlerFailureKind,
    },
    /// The gate cancelled this page so a user-visible pagination could run.
    /// The checkpoint is unchanged and must be re-queued (no progress lost).
    Preempted { checkpoint: HistoryCrawlCheckpoint },
}

/// One page of history plus what the caller needs to continue the crawl.
struct CrawlPage {
    /// Events the crawl's first page reads from the resident cache (#996); empty
    /// after the first page.
    cached: Vec<matrix_sdk::deserialized_responses::TimelineEvent>,
    /// Events the source returned, newest first.
    events: Vec<matrix_sdk::deserialized_responses::TimelineEvent>,
    /// Persisted cursor to resume from, when the source walks the store.
    next: Option<koushi_sdk::MatrixPersistedEventCursor>,
    /// Position in `events` at which the persisted walk crossed a gap chunk,
    /// i.e. where the page stops being contiguous with the previous one.
    gap_at: Option<usize>,
    /// Whether the source reached the start of the room's known history.
    reached_start: bool,
}

fn trace_crawler_page(
    level: DiagnosticLevel,
    outcome: &'static str,
    source: CrawlHistorySource,
    processed: u64,
    indexed: u64,
    page_items: u64,
) {
    let (history_source, timeline_reuse) = match source {
        CrawlHistorySource::Store => ("persisted_chunks", "not_hydrated"),
        CrawlHistorySource::Network => ("sdk_back_pagination", "shared_pagination"),
    };
    record(
        DiagnosticEvent::new(level, "core.startup", "crawler_page")
            .field(DiagnosticField::token("outcome", outcome))
            .field(DiagnosticField::token("history_source", history_source))
            .field(DiagnosticField::token("timeline_reuse", timeline_reuse))
            .field(DiagnosticField::token("persistence", "sdk_linked_chunks"))
            .field(DiagnosticField::count("processed", processed))
            .field(DiagnosticField::count("indexed", indexed))
            .field(DiagnosticField::count("page_items", page_items)),
    );
}

pub(crate) fn spawn_history_crawl_page(
    session: Arc<koushi_sdk::MatrixClientSession>,
    account_work: AccountWorkScheduler,
    checkpoint: HistoryCrawlCheckpoint,
) -> executor::JoinHandle<HistoryCrawlPageResult> {
    executor::spawn(run_history_crawl_page(session, account_work, checkpoint))
}

async fn run_history_crawl_page(
    session: Arc<koushi_sdk::MatrixClientSession>,
    account_work: AccountWorkScheduler,
    mut checkpoint: HistoryCrawlCheckpoint,
) -> HistoryCrawlPageResult {
    if checkpoint.settings.speed == SearchCrawlerSpeed::Paused {
        return HistoryCrawlPageResult::Success {
            checkpoint,
            messages: Vec::new(),
            completed: true,
            work_permit: None,
        };
    }

    let parsed_room_id = match checkpoint.room_id.parse::<matrix_sdk::ruma::OwnedRoomId>() {
        Ok(id) => id,
        Err(_) => {
            return HistoryCrawlPageResult::Failed {
                checkpoint,
                kind: SearchCrawlerFailureKind::RoomNotFound,
            };
        }
    };

    let room = match session.client().get_room(&parsed_room_id) {
        Some(room) => room,
        None => {
            return HistoryCrawlPageResult::Failed {
                checkpoint,
                kind: SearchCrawlerFailureKind::RoomNotFound,
            };
        }
    };

    let batch_size = crawl_batch_size(checkpoint.settings.speed);
    // Cleared only once a page succeeds: a preempted or failed page must run
    // its first-page work (cached events, catch-up boundary) again.
    let first_page = checkpoint.first_page;
    let catch_up_after = checkpoint.catch_up_after.clone();
    let history_source = checkpoint.history_source;
    let persisted_cursor = checkpoint.persisted_cursor;
    let crossed_store_gap = checkpoint.crossed_store_gap;
    // History comes from the room's persisted chunks, which the SDK owns and
    // which the encrypted store keeps on disk. Reading them does not insert
    // anything into the shared linked chunk, so indexing does not leave the
    // room's history resident. History the local store does not hold (a gap) is
    // still fetched through SDK back-pagination, which is what reaches
    // `/messages`.
    let work_permit = account_work.acquire(AccountWorkKind::SearchCrawl).await;
    let page = {
        let page_started = Some(startup_trace::now());
        let page_result = tokio::select! {
            biased;
            // A waiting timeline pagination cancels the crawler: yield the gate
            // immediately and re-queue this checkpoint (no progress lost).
            _ = work_permit.cancelled() => {
                startup_trace::trace_crawler_preempted();
                return HistoryCrawlPageResult::Preempted { checkpoint };
            }
            result = async {
                // #996: events that sync already put in the cache are never
                // returned by backward pagination, so the first page of a crawl
                // indexes them too. Reading them does not load anything new.
                let mut cached = if first_page {
                    let (event_cache, _drop_handles) = room
                        .client()
                        .event_cache()
                        .ok_or(())?
                        .room(room.room_id())
                        .await
                        .map_err(|_| ())?;
                    event_cache.events().await.map_err(|_| ())?
                } else {
                    Vec::new()
                };
                // The resident read above cannot prove the boundary is
                // contiguous with the newest event: it flattens the linked
                // chunk, so a gap between resident chunks is invisible, and it
                // cannot be sliced safely for a catch-up. The persisted page
                // reports that topology, so a catch-up decides from pages only.
                if catch_up_after.is_some() {
                    cached.clear();
                }
                match history_source {
                    CrawlHistorySource::Network => {
                        let (event_cache, _drop_handles) = room
                            .client()
                            .event_cache()
                            .ok_or(())?
                            .room(room.room_id())
                            .await
                            .map_err(|_| ())?;
                        let page = event_cache
                            .pagination()
                            .run_backwards_once(batch_size as u16)
                            .await
                            .map_err(|_| ())?;
                        Ok::<_, ()>(CrawlPage {
                            cached,
                            events: page.events,
                            next: None,
                            gap_at: None,
                            reached_start: page.reached_start,
                        })
                    }
                    CrawlHistorySource::Store => {
                        let page = koushi_sdk::persisted_room_event_page(
                            &session,
                            &checkpoint.room_id,
                            persisted_cursor,
                            batch_size as usize,
                        )
                        .await
                        .map_err(|_| ())?;
                        // The persisted walk is complete only when it reaches
                        // the oldest stored chunk without crossing a gap. A gap
                        // proves the store does not hold the history in between,
                        // so the crawl must continue through the network.
                        let gap_seen = crossed_store_gap || page.gap_at.is_some();
                        let reached_start = page.next.is_none() && !gap_seen;
                        Ok::<_, ()>(CrawlPage {
                            cached,
                            events: page.events,
                            next: page.next,
                            gap_at: page.gap_at,
                            reached_start,
                        })
                    }
                }
            } => result,
        };
        startup_trace::trace_phase(StartupPhase::CrawlerPage, page_started);
        match page_result {
            Ok(page) => page,
            Err(_) => {
                trace_crawler_page(
                    DiagnosticLevel::Warn,
                    "failed",
                    history_source,
                    checkpoint.processed,
                    checkpoint.indexed,
                    0,
                );
                return HistoryCrawlPageResult::Failed {
                    checkpoint,
                    kind: SearchCrawlerFailureKind::Sdk,
                };
            }
        }
    };

    let CrawlPage {
        cached,
        events: page_events_owned,
        next,
        gap_at,
        reached_start,
    } = page;
    checkpoint.first_page = false;
    if checkpoint.history_source == CrawlHistorySource::Store {
        checkpoint.persisted_cursor = next;
        checkpoint.crossed_store_gap = crossed_store_gap || gap_at.is_some();
    }
    let page_events = page_events_owned.as_slice();
    let chunk_len = page_events.len() as u64;
    // Catch-up completes when a page holds the boundary and the range from the
    // newest event down to it is continuous. Page events are newest first, so
    // the boundary and everything older were indexed before.
    let boundary_in_page = catch_up_after.as_deref().and_then(|boundary| {
        page_events
            .iter()
            .position(|event| event.event_id().is_some_and(|id| id.as_str() == boundary))
    });
    // A boundary found in a page is only proof of catch-up when the range from
    // the tail down to it is continuous. A persisted page can cross a gap chunk,
    // in which case the events inside that gap were never delivered locally and
    // must not be skipped: the crawl continues through back-pagination, which
    // repairs the gap, instead of acknowledging catch-up. A fetched page is
    // continuous by construction.
    let boundary_is_continuous = match boundary_in_page {
        None => false,
        Some(position) => {
            if checkpoint.history_source == CrawlHistorySource::Network {
                true
            } else {
                !crossed_store_gap && gap_at.is_none_or(|gap| position < gap)
            }
        }
    };
    let reached_catch_up_boundary = catch_up_after.is_some() && boundary_is_continuous;
    let page_events = match boundary_in_page {
        Some(position) => &page_events[..position],
        None => page_events,
    };
    let completed = reached_catch_up_boundary || reached_start;
    if !completed && checkpoint.history_source == CrawlHistorySource::Network && chunk_len == 0 {
        // A fetched page with no events that did not report the start of the
        // room is not evidence of completion: the SDK can hold that token and
        // still have older history behind it. Marking the room done here would
        // skip that history durably.
        trace_crawler_page(
            DiagnosticLevel::Warn,
            "empty_page",
            checkpoint.history_source,
            checkpoint.processed,
            checkpoint.indexed,
            chunk_len,
        );
        return HistoryCrawlPageResult::Failed {
            checkpoint,
            kind: SearchCrawlerFailureKind::Sdk,
        };
    }
    let boundary_blocks_completion =
        catch_up_after.is_some() && boundary_in_page.is_some() && !reached_catch_up_boundary;
    if !completed
        && checkpoint.history_source == CrawlHistorySource::Store
        && (next.is_none() || boundary_blocks_completion)
    {
        // Either the local store is exhausted with a gap behind it, or the
        // catch-up boundary sits beyond a gap: both mean the rest of this crawl
        // must come from back-pagination, which is the path that repairs gaps.
        checkpoint.history_source = CrawlHistorySource::Network;
    }
    let mut seen_event_ids = HashSet::new();
    let events = cached
        .iter()
        .chain(page_events.iter())
        .filter(|event| {
            event
                .event_id()
                .is_none_or(|event_id| seen_event_ids.insert(event_id.to_string()))
        })
        .collect::<Vec<_>>();
    checkpoint.processed += events.len() as u64;

    let mut index_messages = Vec::new();
    let mut index_events = Vec::new();
    for timeline_event in events {
        if timeline_event.kind.is_utd() {
            continue;
        }
        // The same page is written to the persistent index below; keep the
        // owned events for that write.
        index_events.push(timeline_event.clone());

        let raw = timeline_event.kind.raw();
        let json = raw.json().get();
        let Some(message) = event_json_to_index_message(
            &checkpoint.room_id,
            json,
            &checkpoint.settings,
            &mut checkpoint.pending_redactions,
        ) else {
            continue;
        };

        let already_redacted = match &message {
            SearchIndexMessage::Upsert { event_id, .. } => {
                checkpoint.pending_redactions.contains(event_id)
            }
            _ => false,
        };
        if already_redacted {
            continue;
        }

        checkpoint.indexed += 1;
        index_messages.push(message);
    }

    trace_crawler_page(
        DiagnosticLevel::Debug,
        if completed { "completed" } else { "progress" },
        checkpoint.history_source,
        checkpoint.processed,
        checkpoint.indexed,
        chunk_len,
    );

    // A completed crawl may only advance its in-session checkpoint once the index
    // has committed this page. The SDK's own indexing runs in a background
    // subscriber that can lag or drop failures, so a room marked done without
    // this acknowledgement can lose its history for search permanently.
    if let Err(kind) = index_page_events(&session, &checkpoint.room_id, index_events).await {
        trace_crawler_page(
            DiagnosticLevel::Warn,
            "index_failed",
            checkpoint.history_source,
            checkpoint.processed,
            checkpoint.indexed,
            chunk_len,
        );
        return HistoryCrawlPageResult::Failed { checkpoint, kind };
    }

    HistoryCrawlPageResult::Success {
        checkpoint,
        messages: index_messages,
        completed,
        work_permit: Some(work_permit),
    }
}

/// Commit one crawled page's events to the persistent index, or say why not.
///
/// The caller must not advance its in-session crawl checkpoint unless this returns
/// `Ok`.
async fn index_page_events(
    session: &Arc<koushi_sdk::MatrixClientSession>,
    room_id: &str,
    events: Vec<matrix_sdk::deserialized_responses::TimelineEvent>,
) -> Result<(), SearchCrawlerFailureKind> {
    if events.is_empty() {
        return Ok(());
    }

    koushi_sdk::index_room_events_now(session, room_id, events)
        .await
        .map_err(|error| match error {
            koushi_sdk::MatrixSearchError::IndexUnavailable => {
                SearchCrawlerFailureKind::IndexUnavailable
            }
            koushi_sdk::MatrixSearchError::Query => SearchCrawlerFailureKind::RoomNotFound,
            koushi_sdk::MatrixSearchError::Internal => SearchCrawlerFailureKind::Sdk,
        })
}

/// The shared scheduler controls work rate; this controls one admitted page's
/// bounded size.
fn crawl_batch_size(speed: SearchCrawlerSpeed) -> u32 {
    match speed {
        SearchCrawlerSpeed::Fast => BATCH_SIZE_FAST,
        SearchCrawlerSpeed::Slow => BATCH_SIZE_SLOW,
        SearchCrawlerSpeed::Paused | SearchCrawlerSpeed::Standard => BATCH_SIZE_STANDARD,
    }
}

pub(crate) fn event_json_to_index_message(
    room_id: &str,
    json: &str,
    settings: &SearchCrawlerSettings,
    pending_redactions: &mut HashSet<String>,
) -> Option<SearchIndexMessage> {
    let value: Value = serde_json::from_str(json).ok()?;
    if value.get("state_key").is_some() {
        return None; // State events are not room messages or replacements.
    }
    let event_id = value.get("event_id")?.as_str()?.to_owned();
    let sender = value.get("sender")?.as_str()?.to_owned();
    let timestamp_ms = value.get("origin_server_ts")?.as_u64()?;

    let event_type = value.get("type")?.as_str()?;
    match event_type {
        "m.room.redaction" => {
            // The `redacts` field names the TARGET event to remove from the
            // index, not the redaction event itself.  In a backward crawl the
            // redaction arrives before the original, so record the target in
            // `pending_redactions` to suppress it when the original is seen.
            let target_id = value
                .get("redacts")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    // MSC2174 / newer servers nest it inside content.
                    value
                        .get("content")
                        .and_then(|c| c.get("redacts"))
                        .and_then(|v| v.as_str())
                })
                .map(|s| s.to_owned())?;
            pending_redactions.insert(target_id.clone());
            Some(SearchIndexMessage::Redact {
                event_id: target_id,
            })
        }
        "m.room.message" => {
            let content = value.get("content")?;
            if is_edit_event(content) {
                let target_event_id = edit_target_event_id(content)?;
                let replacement_content = content.get("m.new_content")?;
                let msgtype = replacement_content.get("msgtype")?.as_str()?;
                let body = replacement_content.get("body")?.as_str()?;
                let (text_body, attachment_filename, attachment) =
                    project_message_content(msgtype, body, replacement_content, settings)?;
                // Even an opted-out text replacement can remove an existing
                // Files attachment. Only identity/provenance is forwarded then.
                return Some(SearchIndexMessage::Edit {
                    room_id: room_id.to_owned(),
                    edit_event_id: event_id,
                    target_event_id,
                    sender,
                    timestamp_ms,
                    body: text_body,
                    attachment_filename,
                    attachment,
                    // A crawl reports what history showed, not the current
                    // visible content.
                    canonical: false,
                });
            }
            let msgtype = content.get("msgtype")?.as_str()?;
            let body = content.get("body")?.as_str()?;
            let (text_body, attachment_filename, attachment) =
                project_message_content(msgtype, body, content, settings)?;
            if text_body.is_none() && attachment_filename.is_none() {
                return None;
            }
            Some(SearchIndexMessage::Upsert {
                room_id: room_id.to_owned(),
                event_id,
                sender,
                timestamp_ms,
                body: text_body,
                attachment_filename,
                attachment,
                canonical: false,
                edit: None,
            })
        }
        "m.sticker" => {
            let content = value.get("content")?;
            let body = content.get("body")?.as_str()?;
            let text_body = settings.include_media_captions.then(|| body.to_owned());
            let attachment_filename = settings.include_filenames.then(|| body.to_owned());
            let attachment = settings
                .include_filenames
                .then(|| build_attachment_document("m.sticker", content))
                .flatten();
            if text_body.is_none() && attachment_filename.is_none() {
                return None;
            }
            Some(SearchIndexMessage::Upsert {
                room_id: room_id.to_owned(),
                event_id,
                sender,
                timestamp_ms,
                body: text_body,
                attachment_filename,
                attachment,
                canonical: false,
                edit: None,
            })
        }
        _ => None,
    }
}

pub(crate) fn is_edit_event(content: &Value) -> bool {
    content
        .get("m.relates_to")
        .or_else(|| content.get("relates_to"))
        .and_then(|rel| rel.get("rel_type"))
        .and_then(|v| v.as_str())
        == Some("m.replace")
}

pub(crate) fn edit_target_event_id(content: &Value) -> Option<String> {
    content
        .get("m.relates_to")
        .or_else(|| content.get("relates_to"))
        .and_then(|rel| rel.get("event_id"))
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
}

fn project_message_content(
    msgtype: &str,
    body: &str,
    content: &Value,
    settings: &SearchCrawlerSettings,
) -> Option<(Option<String>, Option<String>, Option<AttachmentDocument>)> {
    match msgtype {
        "m.text" | "m.emote" | "m.notice" => Some((Some(body.to_owned()), None, None)),
        "m.image" | "m.video" | "m.audio" | "m.file" => {
            let filename = if msgtype == "m.file" {
                content
                    .get("filename")
                    .and_then(|v| v.as_str())
                    .unwrap_or(body)
                    .to_owned()
            } else {
                body.to_owned()
            };
            let text_body = settings.include_media_captions.then(|| body.to_owned());
            let attachment_filename = settings.include_filenames.then_some(filename);
            let attachment = settings
                .include_filenames
                .then(|| build_attachment_document(msgtype, content))
                .flatten();
            Some((text_body, attachment_filename, attachment))
        }
        _ => None,
    }
}

fn build_attachment_document(msgtype: &str, content: &Value) -> Option<AttachmentDocument> {
    let info = content.get("info").cloned().unwrap_or_default();
    let kind = attachment_kind(msgtype)?;
    let (source_url, encrypted, encryption_version) = media_source(content);
    let thumbnail_url = thumbnail_source(&info);
    let mimetype = info
        .get("mimetype")
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned);
    let size = info.get("size").and_then(|v| v.as_u64());
    let width = info
        .get("w")
        .and_then(|v| v.as_u64())
        .and_then(|w| u32::try_from(w).ok());
    let height = info
        .get("h")
        .and_then(|v| v.as_u64())
        .and_then(|h| u32::try_from(h).ok());

    Some(AttachmentDocument {
        kind,
        msgtype: msgtype.to_owned(),
        mimetype,
        size,
        source_mxc: source_url,
        thumbnail_mxc: thumbnail_url,
        filename: SensitiveString::new(
            content
                .get("filename")
                .and_then(|v| v.as_str())
                .or_else(|| content.get("body").and_then(|v| v.as_str()))
                .unwrap_or("")
                .to_owned(),
        ),
        thread_root: None,
        encrypted,
        encryption_version,
        width,
        height,
        is_edited: false,
    })
}

fn attachment_kind(msgtype: &str) -> Option<AttachmentKind> {
    match msgtype {
        "m.image" => Some(AttachmentKind::Image),
        "m.video" => Some(AttachmentKind::Video),
        "m.audio" => Some(AttachmentKind::Audio),
        "m.file" => Some(AttachmentKind::File),
        "m.sticker" => Some(AttachmentKind::Sticker),
        _ => None,
    }
}

fn media_source(content: &Value) -> (String, bool, Option<String>) {
    if let Some(file) = content.get("file") {
        let url = file
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();
        let version = file
            .get("v")
            .or_else(|| file.get("version"))
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned);
        (url, true, version)
    } else if let Some(url) = content.get("url").and_then(|v| v.as_str()) {
        (url.to_owned(), false, None)
    } else {
        (String::new(), false, None)
    }
}

fn thumbnail_source(info: &Value) -> Option<String> {
    if let Some(file) = info.get("thumbnail_file") {
        file.get("url")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned)
    } else {
        info.get("thumbnail_url")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned)
    }
}

#[cfg(test)]
mod tests;
