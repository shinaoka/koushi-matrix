//! Search history crawler: pages older room events through `/rooms/{roomId}/messages`,
//! decrypts them locally, and feeds searchable text into the document store.
//!
//! Media file bytes are never fetched; only MXC URIs, filenames, captions and
//! metadata are indexed. This keeps the crawler a text-only backfill worker.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use matrix_desktop_search::{AttachmentDocument, AttachmentKind, SensitiveString};
use matrix_desktop_state::{
    AppAction, SearchCrawlerFailureKind, SearchCrawlerSettings, SearchCrawlerSpeed,
};
use matrix_sdk::room::{MessagesOptions, Room};
use matrix_sdk::ruma::api::client::direction::Direction;
use serde_json::Value;
use tokio::sync::{broadcast, mpsc};

use crate::event::{CoreEvent, SearchEvent};
use crate::ids::RequestId;
use crate::search::SearchIndexMessage;
use crate::executor;

const BATCH_SIZE_FAST: u32 = 200;
const BATCH_SIZE_STANDARD: u32 = 100;
const BATCH_SIZE_SLOW: u32 = 50;

/// Start a background crawl for the given room.
#[allow(clippy::too_many_arguments)]
pub fn spawn_history_crawl(
    session: Arc<matrix_desktop_sdk::MatrixClientSession>,
    room_id: String,
    request_id: RequestId,
    settings: SearchCrawlerSettings,
    index_tx: mpsc::Sender<SearchIndexMessage>,
    action_tx: mpsc::Sender<Vec<AppAction>>,
    event_tx: broadcast::Sender<CoreEvent>,
    cancel: Arc<AtomicBool>,
) {
    executor::spawn(run_history_crawl(
        session,
        room_id,
        request_id,
        settings,
        index_tx,
        action_tx,
        event_tx,
        cancel,
    ));
}

async fn run_history_crawl(
    session: Arc<matrix_desktop_sdk::MatrixClientSession>,
    room_id: String,
    request_id: RequestId,
    settings: SearchCrawlerSettings,
    index_tx: mpsc::Sender<SearchIndexMessage>,
    action_tx: mpsc::Sender<Vec<AppAction>>,
    event_tx: broadcast::Sender<CoreEvent>,
    cancel: Arc<AtomicBool>,
) {
    send_action(
        &action_tx,
        AppAction::HistoryCrawlStarted {
            request_id: request_id.sequence,
            room_id: room_id.clone(),
        },
    )
    .await;

    if settings.speed == SearchCrawlerSpeed::Paused {
        send_action(
            &action_tx,
            AppAction::HistoryCrawlCompleted {
                room_id,
                indexed: 0,
            },
        )
        .await;
        return;
    }

    let parsed_room_id = match room_id.parse::<matrix_sdk::ruma::OwnedRoomId>() {
        Ok(id) => id,
        Err(_) => {
            send_failure(
                &action_tx,
                &room_id,
                SearchCrawlerFailureKind::RoomNotFound,
                "invalid room id",
            )
            .await;
            return;
        }
    };

    let room = match session.client().get_room(&parsed_room_id) {
        Some(room) => room,
        None => {
            send_failure(
                &action_tx,
                &room_id,
                SearchCrawlerFailureKind::RoomNotFound,
                "room not found",
            )
            .await;
            return;
        }
    };

    let (batch_size, delay_ms) = match settings.speed {
        SearchCrawlerSpeed::Fast => (BATCH_SIZE_FAST, 0),
        SearchCrawlerSpeed::Slow => (BATCH_SIZE_SLOW, 500),
        _ => (BATCH_SIZE_STANDARD, 100),
    };

    let mut options = MessagesOptions::new(Direction::Backward);
    options.limit = batch_size.into();
    let mut processed: u64 = 0;
    let mut indexed: u64 = 0;

    loop {
        if cancel.load(Ordering::Relaxed) {
            send_action(
                &action_tx,
                AppAction::HistoryCrawlCompleted {
                    room_id: room_id.clone(),
                    indexed,
                },
            )
            .await;
            return;
        }

        let messages = match room.messages(options.clone()).await {
            Ok(messages) => messages,
            Err(_) => {
                send_failure(
                    &action_tx,
                    &room_id,
                    SearchCrawlerFailureKind::Sdk,
                    "messages request failed",
                )
                .await;
                return;
            }
        };

        let chunk_len = messages.chunk.len() as u64;
        processed += chunk_len;

        for timeline_event in &messages.chunk {
            if timeline_event.kind.is_utd() {
                continue;
            }

            let raw = timeline_event.kind.raw();
            let Some(json) = raw.json().ok() else { continue };
            let Some(message) = event_json_to_index_message(&room_id, json, &settings) else {
                continue;
            };
            indexed += 1;
            let _ = index_tx.try_send(message);
        }

        send_action(
            &action_tx,
            AppAction::HistoryCrawlProgress {
                room_id: room_id.clone(),
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
            Some(next_token) => options.from = Some(next_token),
            None => break,
        }
    }

    send_action(
        &action_tx,
        AppAction::HistoryCrawlCompleted {
            room_id: room_id.clone(),
            indexed,
        },
    )
    .await;
    let _ = event_tx.send(CoreEvent::Search(SearchEvent::HistoryCrawlCompleted {
        room_id,
        indexed,
    }));
}

async fn send_action(action_tx: &mpsc::Sender<Vec<AppAction>>, action: AppAction) {
    let _ = action_tx.send(vec![action]).await;
}

async fn send_failure(
    action_tx: &mpsc::Sender<Vec<AppAction>>,
    room_id: &str,
    kind: SearchCrawlerFailureKind,
    message: &str,
) {
    send_action(
        action_tx,
        AppAction::HistoryCrawlFailed {
            room_id: room_id.to_owned(),
            kind,
            message: message.to_owned(),
        },
    )
    .await;
}

fn event_json_to_index_message(
    room_id: &str,
    json: &str,
    settings: &SearchCrawlerSettings,
) -> Option<SearchIndexMessage> {
    let value: Value = serde_json::from_str(json).ok()?;
    let event_id = value.get("event_id")?.as_str()?.to_owned();
    let sender = value.get("sender")?.as_str()?.to_owned();
    let timestamp_ms = value.get("origin_server_ts")?.as_u64()?;

    let event_type = value.get("type")?.as_str()?;
    match event_type {
        "m.room.redaction" => Some(SearchIndexMessage::Redact { event_id }),
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
        let message = event_json_to_index_message("!r:test", json, &settings).unwrap();
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
        let message = event_json_to_index_message("!r:test", json, &settings).unwrap();
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
        assert!(event_json_to_index_message("!r:test", json, &settings).is_none());
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
        let message = event_json_to_index_message("!r:test", json, &settings).unwrap();
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
        let message = event_json_to_index_message("!r:test", json, &settings).unwrap();
        match message {
            SearchIndexMessage::Upsert { body, attachment_filename, attachment, .. } => {
                assert_eq!(body.as_deref(), Some("image.png"));
                assert!(attachment_filename.is_none());
                assert!(attachment.is_none());
            }
            other => panic!("expected Upsert, got {other:?}"),
        }
    }
}
