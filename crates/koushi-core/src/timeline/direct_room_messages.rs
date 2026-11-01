//! Room timeline reads for out-of-band callers (#1530).
//!
//! The GUI reads history through a resident `TimelineActor`, which owns a
//! pagination window per room. An MCP caller must not depend on that
//! presentation lifetime: a room nobody opened has no actor at all, and
//! mounting one would change GUI state. This module pages the room straight
//! from the account session, so nothing is mounted, no app action is reduced,
//! and an open room's scroll position is untouched.
//!
//! `Room::messages` decrypts each page before it is returned, so a body here is
//! what this device can already read. A page that cannot be decrypted keeps the
//! envelope fields and no body.

use std::sync::Arc;
use std::time::Duration;

use koushi_protocol::event::{RoomMessageItem, RoomMessageKind, RoomMessagesPage};
use koushi_sdk::MatrixClientSession;
use matrix_sdk::RoomState;
use matrix_sdk::deserialized_responses::TimelineEvent;
use matrix_sdk::room::MessagesOptions;
use matrix_sdk::ruma::UInt;
use matrix_sdk::ruma::events::room::message::MessageType;
use matrix_sdk::ruma::events::{
    AnySyncMessageLikeEvent, AnySyncTimelineEvent, SyncMessageLikeEvent,
};

/// Bound on one page read, including decryption.
const READ_TIMEOUT: Duration = Duration::from_secs(20);

/// Largest page this layer will ask the server for.
const MAX_LIMIT: u32 = 100;

use crate::direct_operations::RoomMessagesFailure;

/// Reads one page of a room's recent messages, newest first.
pub(crate) async fn read_room_messages(
    session: Arc<MatrixClientSession>,
    room_id: &str,
    limit: u32,
    from: Option<String>,
) -> Result<RoomMessagesPage, RoomMessagesFailure> {
    let parsed_room =
        matrix_sdk::ruma::RoomId::parse(room_id).map_err(|_| RoomMessagesFailure::RoomNotFound)?;
    let room = session
        .client()
        .get_room(&parsed_room)
        .ok_or(RoomMessagesFailure::RoomNotFound)?;
    if room.state() != RoomState::Joined {
        return Err(RoomMessagesFailure::RoomNotFound);
    }

    let mut options = MessagesOptions::backward().from(from.as_deref());
    options.limit = UInt::from(limit.clamp(1, MAX_LIMIT));
    let page = crate::executor::timeout(READ_TIMEOUT, room.messages(options))
        .await
        .map_err(|_| RoomMessagesFailure::Unavailable)?
        .map_err(|_| RoomMessagesFailure::Unavailable)?;

    Ok(RoomMessagesPage {
        messages: page.chunk.iter().filter_map(item_for_event).collect(),
        next: page.end,
    })
}

pub(super) fn is_redacted(event: &TimelineEvent) -> bool {
    event
        .raw()
        .get_field::<serde_json::Value>("unsigned")
        .ok()
        .flatten()
        .and_then(|unsigned| unsigned.get("redacted_because").cloned())
        .is_some_and(|because| !because.is_null())
}

/// Projects one timeline event, or `None` when it is not a room message.
fn item_for_event(event: &TimelineEvent) -> Option<RoomMessageItem> {
    let parsed: AnySyncTimelineEvent = event.raw().deserialize().ok()?;
    let message = match parsed {
        AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomMessage(message)) => message,
        AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomEncrypted(message))
            if event.encryption_info().is_none() =>
        {
            // Match get_message: missing keys leave a readable envelope, not a missing event.
            return Some(RoomMessageItem {
                event_id: message.event_id().to_string(),
                sender_id: message.sender().to_string(),
                timestamp_ms: u64::from(message.origin_server_ts().0),
                body: None,
                content_available: false,
                kind: RoomMessageKind::Other,
                is_encrypted: true,
                is_redacted: is_redacted(event),
            });
        }
        _ => return None,
    };
    let sender_id = message.sender().to_string();
    let timestamp_ms = u64::from(message.origin_server_ts().0);
    let is_encrypted = event.encryption_info().is_some();
    match message {
        SyncMessageLikeEvent::Original(original) => {
            let (kind, body) = describe(&original.content.msgtype);
            Some(RoomMessageItem {
                event_id: original.event_id.to_string(),
                sender_id,
                timestamp_ms,
                content_available: body.is_some(),
                body,
                kind,
                is_encrypted,
                is_redacted: false,
            })
        }
        SyncMessageLikeEvent::Redacted(redacted) => Some(RoomMessageItem {
            event_id: redacted.event_id.to_string(),
            sender_id,
            timestamp_ms,
            body: None,
            content_available: false,
            kind: RoomMessageKind::Other,
            is_encrypted,
            is_redacted: true,
        }),
    }
}

/// The message kind and its readable text.
///
/// Media events carry a filename or a caption in `body`; keeping it lets a
/// reader tell "sent an image" from "sent `agent-scenario.png`". An event this
/// build does not model stays `Other` with no text.
pub(crate) fn message_kind(msgtype: &MessageType) -> RoomMessageKind {
    match msgtype {
        MessageType::Text(_) => RoomMessageKind::Text,
        MessageType::Emote(_) => RoomMessageKind::Emote,
        MessageType::Notice(_) => RoomMessageKind::Notice,
        MessageType::Image(_) => RoomMessageKind::Image,
        MessageType::Video(_) => RoomMessageKind::Video,
        MessageType::Audio(_) => RoomMessageKind::Audio,
        MessageType::File(_) => RoomMessageKind::File,
        _ => RoomMessageKind::Other,
    }
}

fn describe(msgtype: &MessageType) -> (RoomMessageKind, Option<String>) {
    let kind = message_kind(msgtype);
    let body = match msgtype {
        MessageType::Text(content) => Some(&content.body),
        MessageType::Emote(content) => Some(&content.body),
        MessageType::Notice(content) => Some(&content.body),
        MessageType::Image(content) => Some(&content.body),
        MessageType::Video(content) => Some(&content.body),
        MessageType::Audio(content) => Some(&content.body),
        MessageType::File(content) => Some(&content.body),
        _ => None,
    };
    let body = body
        .map(|body: &String| body.trim().to_owned())
        .filter(|body| !body.is_empty());
    (kind, body)
}

#[cfg(test)]
#[path = "direct_room_messages_tests.rs"]
mod pagination_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use koushi_state::SessionInfo;
    use serde_json::json;

    async fn synthetic_session() -> (Arc<MatrixClientSession>, tempfile::TempDir) {
        let server = matrix_sdk::test_utils::mocks::MatrixMockServer::new().await;
        let client = server.client_builder().build().await;
        let session = MatrixClientSession::from_client_for_testing(
            client.clone(),
            SessionInfo {
                homeserver: server.server().uri(),
                user_id: client.user_id().expect("user id").to_string(),
                device_id: client.device_id().expect("device id").to_string(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            },
        );
        (Arc::new(session), tempfile::tempdir().expect("data dir"))
    }

    fn plain_event(value: serde_json::Value) -> TimelineEvent {
        let raw = matrix_sdk::ruma::serde::Raw::new(&value).expect("raw event");
        TimelineEvent::from_plaintext(raw.cast_unchecked())
    }

    /// #1530: reading must fail closed for a room the account has not joined,
    /// even though no timeline actor is involved at all.
    #[tokio::test]
    async fn an_unjoined_room_fails_closed() {
        let (session, _data_dir) = synthetic_session().await;
        let failure = read_room_messages(session, "!not-joined:example.invalid", 20, None)
            .await
            .expect_err("an unjoined room must fail");
        assert_eq!(failure, RoomMessagesFailure::RoomNotFound);
    }

    #[test]
    fn a_text_message_carries_sender_time_and_body() {
        let event = plain_event(json!({
            "type": "m.room.message",
            "event_id": "$event:example.invalid",
            "sender": "@alice:example.invalid",
            "origin_server_ts": 1_700_000_123_000u64,
            "content": { "msgtype": "m.text", "body": "  hello  " }
        }));
        let item = item_for_event(&event).expect("message");
        assert_eq!(item.event_id, "$event:example.invalid");
        assert_eq!(item.sender_id, "@alice:example.invalid");
        assert_eq!(item.timestamp_ms, 1_700_000_123_000);
        assert_eq!(item.body.as_deref(), Some("hello"));
        assert_eq!(item.kind, RoomMessageKind::Text);
        assert!(!item.is_encrypted);
        assert!(!item.is_redacted);
    }

    #[test]
    fn non_message_events_are_skipped() {
        let event = plain_event(json!({
            "type": "m.room.member",
            "event_id": "$membership:example.invalid",
            "sender": "@alice:example.invalid",
            "origin_server_ts": 1_700_000_123_000u64,
            "state_key": "@alice:example.invalid",
            "content": { "membership": "join" }
        }));
        assert!(item_for_event(&event).is_none());
    }

    #[test]
    fn an_unknown_msgtype_keeps_the_envelope_only() {
        let event = plain_event(json!({
            "type": "m.room.message",
            "event_id": "$location:example.invalid",
            "sender": "@alice:example.invalid",
            "origin_server_ts": 1_700_000_123_000u64,
            "content": {
                "msgtype": "m.location",
                "body": "https://maps.example.invalid/1",
                "geo_uri": "geo:1,2"
            }
        }));
        let item = item_for_event(&event).expect("message");
        assert_eq!(item.kind, RoomMessageKind::Other);
        assert_eq!(item.body, None);
    }

    #[test]
    fn a_redacted_message_keeps_no_body() {
        let event = plain_event(json!({
            "type": "m.room.message",
            "event_id": "$redacted:example.invalid",
            "sender": "@alice:example.invalid",
            "origin_server_ts": 1_700_000_123_000u64,
            "content": {},
            "unsigned": { "redacted_because": {} }
        }));
        let item = item_for_event(&event).expect("message");
        assert_eq!(item.kind, RoomMessageKind::Other);
        assert_eq!(item.body, None);
        assert!(item.is_redacted);
    }
}
