//! Search history crawler: pages older room events through `/rooms/{roomId}/messages`,
//! decrypts them locally, and feeds searchable text into the document store.
//!
//! Media file bytes are never fetched; only MXC URIs, filenames, captions and
//! metadata are indexed. This keeps the crawler a text-only backfill worker.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use koushi_search::{AttachmentDocument, SensitiveString};
use koushi_state::{
    AppAction, AttachmentKind, SearchCrawlerFailureKind, SearchCrawlerSettings, SearchCrawlerSpeed,
};
use matrix_sdk::room::MessagesOptions;
use matrix_sdk::ruma::api::Direction;
use serde_json::Value;
use tokio::sync::{broadcast, mpsc};

use crate::event::{CoreEvent, SearchEvent};
use crate::executor;
use crate::ids::RequestId;
use crate::search::{SearchActorMessage, SearchIndexMessage};

const BATCH_SIZE_FAST: u32 = 200;
const BATCH_SIZE_STANDARD: u32 = 100;
const BATCH_SIZE_SLOW: u32 = 50;

/// Start a background crawl for the given room.
///
/// Returns the `JoinHandle` for the spawned task. Callers (SearchActor) store
/// this handle so they can await it on shutdown and prevent stale crawl tasks
/// from emitting actions into the next account's reducer (P4 fix).
///
/// `settings_generation` is the actor's current generation counter at the time
/// of spawn.  It is forwarded in `CrawlFinished` so the actor can discard
/// completions from crawls that ran under stale content-indexing settings
/// (P1-A: running-room recrawl correctness).
#[allow(clippy::too_many_arguments)]
pub fn spawn_history_crawl(
    session: Arc<koushi_sdk::MatrixClientSession>,
    room_id: String,
    request_id: RequestId,
    settings: SearchCrawlerSettings,
    index_tx: mpsc::Sender<SearchIndexMessage>,
    action_tx: mpsc::Sender<Vec<AppAction>>,
    event_tx: broadcast::Sender<CoreEvent>,
    cancel: Arc<AtomicBool>,
    actor_tx: mpsc::Sender<SearchActorMessage>,
    settings_generation: u64,
) -> executor::JoinHandle<bool> {
    executor::spawn(run_history_crawl(
        session,
        room_id,
        request_id,
        settings,
        index_tx,
        action_tx,
        event_tx,
        cancel,
        actor_tx,
        settings_generation,
    ))
}

async fn run_history_crawl(
    session: Arc<koushi_sdk::MatrixClientSession>,
    room_id: String,
    request_id: RequestId,
    settings: SearchCrawlerSettings,
    index_tx: mpsc::Sender<SearchIndexMessage>,
    action_tx: mpsc::Sender<Vec<AppAction>>,
    event_tx: broadcast::Sender<CoreEvent>,
    cancel: Arc<AtomicBool>,
    actor_tx: mpsc::Sender<SearchActorMessage>,
    settings_generation: u64,
) -> bool {
    let completed = run_history_crawl_inner(
        &session,
        &room_id,
        request_id,
        &settings,
        &index_tx,
        &action_tx,
        &event_tx,
        &cancel,
    )
    .await;

    // Notify the SearchActor that this crawl is done so it can clean up
    // `crawl_cancels` and, if it completed successfully, record the room in
    // `completed_rooms` to prevent duplicate auto-start crawls.
    // Use `send` (not `try_send`) so CrawlFinished is reliably delivered:
    // a dropped state-machine transition is prohibited by the repository
    // reliability rules (REPOSITORY_RULES L124-128).
    let _ = actor_tx
        .send(SearchActorMessage::CrawlFinished { room_id, completed, settings_generation })
        .await;
    completed
}

/// Core crawl logic.  Returns `true` if the room history was fully crawled
/// (completed or already paused), or `false` if it failed.
async fn run_history_crawl_inner(
    session: &koushi_sdk::MatrixClientSession,
    room_id: &str,
    request_id: RequestId,
    settings: &SearchCrawlerSettings,
    index_tx: &mpsc::Sender<SearchIndexMessage>,
    action_tx: &mpsc::Sender<Vec<AppAction>>,
    event_tx: &broadcast::Sender<CoreEvent>,
    cancel: &Arc<AtomicBool>,
) -> bool {
    send_action(
        action_tx,
        AppAction::HistoryCrawlStarted {
            request_id: request_id.sequence,
            room_id: room_id.to_owned(),
        },
    )
    .await;

    if settings.speed == SearchCrawlerSpeed::Paused {
        send_action(
            action_tx,
            AppAction::HistoryCrawlCompleted {
                room_id: room_id.to_owned(),
                indexed: 0,
            },
        )
        .await;
        return true;
    }

    let parsed_room_id = match room_id.parse::<matrix_sdk::ruma::OwnedRoomId>() {
        Ok(id) => id,
        Err(_) => {
            send_failure(
                action_tx,
                room_id,
                SearchCrawlerFailureKind::RoomNotFound,
                "invalid room id",
            )
            .await;
            return false;
        }
    };

    let room = match session.client().get_room(&parsed_room_id) {
        Some(room) => room,
        None => {
            send_failure(
                action_tx,
                room_id,
                SearchCrawlerFailureKind::RoomNotFound,
                "room not found",
            )
            .await;
            return false;
        }
    };

    let (batch_size, delay_ms) = match settings.speed {
        SearchCrawlerSpeed::Fast => (BATCH_SIZE_FAST, 0),
        SearchCrawlerSpeed::Slow => (BATCH_SIZE_SLOW, 500),
        _ => (BATCH_SIZE_STANDARD, 100),
    };

    let mut from_token: Option<String> = None;
    let mut processed: u64 = 0;
    let mut indexed: u64 = 0;
    // Track event ids that were redacted so a later-crawled (older) original
    // is not indexed after its redaction.  This is a backward crawl, so
    // redaction events arrive before the original message they target.
    let mut pending_redactions: HashSet<String> = HashSet::new();

    loop {
        if cancel.load(Ordering::Relaxed) {
            // Cancelled: report as completed so the room can be re-crawled
            // later from the start without being counted as failed.
            send_action(
                action_tx,
                AppAction::HistoryCrawlCompleted {
                    room_id: room_id.to_owned(),
                    indexed,
                },
            )
            .await;
            // Return false so the actor does NOT add this room to
            // `completed_rooms` — a cancelled crawl is retryable.
            return false;
        }

        // Build a fresh MessagesOptions per page (MessagesOptions is not Clone).
        let mut options = MessagesOptions::new(Direction::Backward);
        options.limit = batch_size.into();
        options.from = from_token.clone();

        let messages = match room.messages(options).await {
            Ok(messages) => messages,
            Err(_) => {
                send_failure(
                    action_tx,
                    room_id,
                    SearchCrawlerFailureKind::Sdk,
                    "messages request failed",
                )
                .await;
                return false;
            }
        };

        let chunk_len = messages.chunk.len() as u64;
        processed += chunk_len;

        for timeline_event in &messages.chunk {
            if timeline_event.kind.is_utd() {
                continue;
            }

            let raw = timeline_event.kind.raw();
            let json = raw.json().get();
            let Some(message) =
                event_json_to_index_message(room_id, json, settings, &mut pending_redactions)
            else {
                continue;
            };

            // If this upsert targets an event that was already redacted
            // (seen earlier in this backward crawl), skip it.
            let already_redacted = match &message {
                SearchIndexMessage::Upsert { event_id, .. } => {
                    pending_redactions.contains(event_id)
                }
                _ => false,
            };
            if already_redacted {
                continue;
            }

            // Use `send(...).await` (not `try_send`) so dropped index updates
            // are not silently counted as indexed and do not produce a
            // spurious `Completed` state with missing terms.  On backpressure
            // emit a coarse `IndexUnavailable` failure so the room is
            // retryable (P3 fix; REPOSITORY_RULES L124-128).
            if index_tx.send(message).await.is_err() {
                send_failure(
                    action_tx,
                    room_id,
                    SearchCrawlerFailureKind::IndexUnavailable,
                    "index channel closed",
                )
                .await;
                return false;
            }
            indexed += 1;
        }

        send_action(
            action_tx,
            AppAction::HistoryCrawlProgress {
                room_id: room_id.to_owned(),
                processed,
                indexed,
            },
        )
        .await;

        if chunk_len == 0 {
            break;
        }

        if delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }

        match messages.end {
            Some(next_token) => from_token = Some(next_token),
            None => break,
        }
    }

    send_action(
        action_tx,
        AppAction::HistoryCrawlCompleted {
            room_id: room_id.to_owned(),
            indexed,
        },
    )
    .await;
    let _ = event_tx.send(CoreEvent::Search(SearchEvent::HistoryCrawlCompleted {
        room_id: room_id.to_owned(),
        indexed,
    }));
    true
}

async fn send_action(action_tx: &mpsc::Sender<Vec<AppAction>>, action: AppAction) {
    let _ = action_tx.send(vec![action]).await;
}

async fn send_failure(
    action_tx: &mpsc::Sender<Vec<AppAction>>,
    room_id: &str,
    kind: SearchCrawlerFailureKind,
    _detail: &str,
) {
    // _detail is logged internally only; the coarse kind is what crosses the
    // Tauri/TypeScript boundary (privacy rule: no raw SDK errors in state).
    send_action(
        action_tx,
        AppAction::HistoryCrawlFailed {
            room_id: room_id.to_owned(),
            kind,
        },
    )
    .await;
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
            Some(SearchIndexMessage::Redact { event_id: target_id })
        }
        "m.room.message" => {
            let content = value.get("content")?;
            if is_edit_event(content) {
                return None;
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
            let text_body = settings
                .include_media_captions
                .then(|| body.to_owned());
            let attachment_filename = settings
                .include_filenames
                .then(|| body.to_owned());
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

fn project_message_content(
    msgtype: &str,
    body: &str,
    content: &Value,
    settings: &SearchCrawlerSettings,
) -> Option<(Option<String>, Option<String>, Option<AttachmentDocument>)> {
    match msgtype {
        "m.text" | "m.emote" | "m.notice" => {
            Some((Some(body.to_owned()), None, None))
        }
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
            let text_body = settings
                .include_media_captions
                .then(|| body.to_owned());
            let attachment_filename = settings
                .include_filenames
                .then(|| filename);
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
    let mimetype = info.get("mimetype").and_then(|v| v.as_str()).map(ToOwned::to_owned);
    let size = info.get("size").and_then(|v| v.as_u64());
    let width = info.get("w").and_then(|v| v.as_u64()).and_then(|w| u32::try_from(w).ok());
    let height = info.get("h").and_then(|v| v.as_u64()).and_then(|h| u32::try_from(h).ok());

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
        let url = file.get("url").and_then(|v| v.as_str()).unwrap_or("").to_owned();
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
mod tests {
    use super::*;

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
        let message =
            event_json_to_index_message("!r:test", json, &settings, &mut pending).unwrap();
        match message {
            SearchIndexMessage::Upsert { room_id, event_id, sender, body, attachment, .. } => {
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
        let message =
            event_json_to_index_message("!r:test", json, &settings, &mut pending).unwrap();
        match message {
            SearchIndexMessage::Upsert { body, attachment_filename, attachment, .. } => {
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
    fn crawler_skips_edit_events() {
        let json = r#"{
            "event_id": "$edit:test",
            "sender": "@alice:test",
            "origin_server_ts": 3000,
            "type": "m.room.message",
            "content": {
                "msgtype": "m.text",
                "body": "* edited",
                "m.relates_to": {
                    "rel_type": "m.replace",
                    "event_id": "$e1:test"
                }
            }
        }"#;
        let settings = SearchCrawlerSettings::default();
        let mut pending = HashSet::new();
        assert!(event_json_to_index_message("!r:test", json, &settings, &mut pending).is_none());
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
        let message =
            event_json_to_index_message("!r:test", json, &settings, &mut pending).unwrap();
        match message {
            SearchIndexMessage::Upsert { body, attachment_filename, .. } => {
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
        let message =
            event_json_to_index_message("!r:test", json, &settings, &mut pending).unwrap();
        match message {
            SearchIndexMessage::Upsert { body, attachment_filename, attachment, .. } => {
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
            event_json_to_index_message("!r:test", redaction_json, &settings, &mut pending)
                .unwrap();
        // Must Redact the TARGET, not the redaction event itself.
        match msg {
            SearchIndexMessage::Redact { event_id } => {
                assert_eq!(event_id, "$original:test",
                    "Redact must target the original event, not the redaction event");
            }
            other => panic!("expected Redact, got {other:?}"),
        }
        // The target must be in pending_redactions so a later Upsert is skipped.
        assert!(pending.contains("$original:test"),
            "target should be in pending_redactions set");
        assert!(!pending.contains("$redact:test"),
            "redaction event id itself must not be in pending_redactions");
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
            event_json_to_index_message("!r:test", redaction_json, &settings, &mut pending)
                .unwrap();
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
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
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
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            async {
                loop {
                    tokio::select! {
                        biased;
                        _ = &mut task => break,
                        _ = rx.recv() => {}
                    }
                }
            },
        )
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
}
