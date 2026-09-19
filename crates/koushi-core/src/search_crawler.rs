//! Search history crawler: pages older room events through `/rooms/{roomId}/messages`,
//! decrypts them locally, and feeds searchable text into the document store.
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

use crate::account_work::{AccountWorkKind, AccountWorkScheduler};
use crate::executor;
use crate::search::SearchIndexMessage;
use crate::startup_trace::{self, StartupPhase};

const BATCH_SIZE_FAST: u32 = 200;
const BATCH_SIZE_STANDARD: u32 = 100;
const BATCH_SIZE_SLOW: u32 = 50;

#[derive(Clone)]
pub(crate) struct HistoryCrawlCheckpoint {
    pub room_id: String,
    pub processed: u64,
    pub indexed: u64,
    pub pending_redactions: HashSet<String>,
    pub settings: SearchCrawlerSettings,
    pub settings_generation: u64,
    pub manual: bool,
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
        }
    }
}

pub(crate) enum HistoryCrawlPageResult {
    Success {
        checkpoint: HistoryCrawlCheckpoint,
        messages: Vec<SearchIndexMessage>,
        completed: bool,
    },
    Failed {
        checkpoint: HistoryCrawlCheckpoint,
        kind: SearchCrawlerFailureKind,
    },
    /// The gate cancelled this page so a user-visible pagination could run.
    /// The checkpoint is unchanged and must be re-queued (no progress lost).
    Preempted { checkpoint: HistoryCrawlCheckpoint },
}

fn trace_crawler_page(
    level: DiagnosticLevel,
    outcome: &'static str,
    processed: u64,
    indexed: u64,
    page_items: u64,
) {
    record(
        DiagnosticEvent::new(level, "core.startup", "crawler_page")
            .field(DiagnosticField::token("outcome", outcome))
            .field(DiagnosticField::token("history_source", "sdk_event_cache"))
            .field(DiagnosticField::token(
                "timeline_reuse",
                "shared_pagination",
            ))
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

    let (batch_size, delay_ms) = crawl_batch_and_delay(checkpoint.settings.speed);
    // Search history must use the SDK-owned room event cache. Calling
    // `Room::messages` here would populate only the search index and leave
    // linked chunks untouched, forcing the normal timeline to fetch the same
    // history again when the room is opened after restart.
    let messages = {
        let permit = account_work.acquire(AccountWorkKind::SearchCrawl).await;
        let page_started = Some(startup_trace::now());
        let page_result = tokio::select! {
            biased;
            // A waiting timeline pagination cancels the crawler: yield the gate
            // immediately and re-queue this checkpoint (no progress lost).
            _ = permit.cancelled() => {
                startup_trace::trace_crawler_preempted();
                return HistoryCrawlPageResult::Preempted { checkpoint };
            }
            result = async {
                let (event_cache, _drop_handles) = room.event_cache().await
                    .map_err(|_| ())?;
                event_cache
                    .pagination()
                    .run_backwards_once(batch_size as u16)
                    .await
                    .map_err(|_| ())
            } => result,
        };
        startup_trace::trace_phase(StartupPhase::CrawlerPage, page_started);
        match page_result {
            Ok(messages) => messages,
            Err(_) => {
                trace_crawler_page(
                    DiagnosticLevel::Warn,
                    "failed",
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

    let chunk_len = messages.events.len() as u64;
    checkpoint.processed += chunk_len;

    let mut index_messages = Vec::new();
    for timeline_event in &messages.events {
        if timeline_event.kind.is_utd() {
            continue;
        }

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

    let completed = messages.reached_start || chunk_len == 0;
    trace_crawler_page(
        DiagnosticLevel::Debug,
        if completed { "completed" } else { "progress" },
        checkpoint.processed,
        checkpoint.indexed,
        chunk_len,
    );

    if !completed && delay_ms > 0 {
        executor::sleep(std::time::Duration::from_millis(delay_ms)).await;
    }

    HistoryCrawlPageResult::Success {
        checkpoint,
        messages: index_messages,
        completed,
    }
}

fn crawl_batch_and_delay(speed: SearchCrawlerSpeed) -> (u32, u64) {
    match speed {
        SearchCrawlerSpeed::Fast => (BATCH_SIZE_FAST, 0),
        SearchCrawlerSpeed::Slow => (BATCH_SIZE_SLOW, 500),
        SearchCrawlerSpeed::Paused | SearchCrawlerSpeed::Standard => (BATCH_SIZE_STANDARD, 100),
    }
}

fn event_json_to_index_message(
    room_id: &str,
    json: &str,
    settings: &SearchCrawlerSettings,
    pending_redactions: &mut HashSet<String>,
) -> Option<SearchIndexMessage> {
    let value: Value = serde_json::from_str(json).ok()?;
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
                if text_body.is_none() && attachment_filename.is_none() {
                    return None;
                }
                return Some(SearchIndexMessage::Edit {
                    edit_event_id: event_id,
                    target_event_id,
                    sender,
                    timestamp_ms,
                    body: text_body,
                    attachment_filename,
                    attachment,
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
            })
        }
        _ => None,
    }
}

fn is_edit_event(content: &Value) -> bool {
    content
        .get("m.relates_to")
        .or_else(|| content.get("relates_to"))
        .and_then(|rel| rel.get("rel_type"))
        .and_then(|v| v.as_str())
        == Some("m.replace")
}

fn edit_target_event_id(content: &Value) -> Option<String> {
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
            let attachment_filename = settings.include_filenames.then(|| filename);
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
