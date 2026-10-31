//! Synthetic missing-key and pagination regressions for #1707.

use super::*;
use koushi_state::SessionInfo;
use matrix_sdk::ruma::serde::Raw;
use matrix_sdk::test_utils::mocks::{MatrixMockServer, RoomMessagesResponseTemplate};
use serde_json::json;

fn encrypted_event(event_id: &str, timestamp: u64) -> serde_json::Value {
    json!({
        "type": "m.room.encrypted",
        "event_id": event_id,
        "sender": "@bob:example.invalid",
        "origin_server_ts": timestamp,
        "content": {
            "algorithm": "m.megolm.v1.aes-sha2",
            "ciphertext": "synthetic",
            "device_id": "SYNTHETIC",
            "sender_key": "synthetic",
            "session_id": "synthetic"
        }
    })
}

fn text_event(event_id: &str, timestamp: u64) -> serde_json::Value {
    json!({
        "type": "m.room.message",
        "event_id": event_id,
        "sender": "@alice:example.invalid",
        "origin_server_ts": timestamp,
        "content": { "msgtype": "m.text", "body": "synthetic text" }
    })
}

#[test]
fn an_undecryptable_message_keeps_its_envelope() {
    let raw = Raw::new(&encrypted_event("$encrypted", 1_700_000_600_000)).expect("raw");
    let event = TimelineEvent::from_plaintext(raw.cast_unchecked());
    let item = item_for_event(&event).expect("an unreadable message must not be dropped");
    assert_eq!(item.event_id, "$encrypted");
    assert_eq!(item.sender_id, "@bob:example.invalid");
    assert_eq!(item.timestamp_ms, 1_700_000_600_000);
    assert_eq!(item.body, None);
    assert_eq!(
        serde_json::to_value(&item).expect("item")["content_available"],
        false
    );
    assert_eq!(item.kind, RoomMessageKind::Other);
    assert!(item.is_encrypted);
    assert!(!item.is_redacted);
}

#[tokio::test]
async fn undecryptable_messages_preserve_page_count_order_and_next() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = matrix_sdk::ruma::room_id!("!room:example.invalid");
    server.sync_joined_room(&client, room_id).await;
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client.clone(),
        SessionInfo {
            homeserver: server.server().uri(),
            user_id: client.user_id().expect("user id").to_string(),
            device_id: client.device_id().expect("device id").to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    ));
    let membership = json!({
        "type": "m.room.member",
        "event_id": "$membership",
        "sender": "@alice:example.invalid",
        "origin_server_ts": 250,
        "state_key": "@alice:example.invalid",
        "content": { "membership": "join" }
    });
    for (from, next, events) in [
        (
            "page-start",
            Some("page-older"),
            vec![
                text_event("$newest", 300),
                membership,
                encrypted_event("$middle", 200),
                text_event("$oldest", 100),
            ],
        ),
        (
            "page-older",
            Some("page-end"),
            vec![
                encrypted_event("$older", 90),
                encrypted_event("$oldest-encrypted", 80),
            ],
        ),
        ("page-end", None, vec![]),
    ] {
        server
            .mock_room_messages()
            .match_limit(4)
            .match_from(from)
            .ok(RoomMessagesResponseTemplate {
                end: next.map(str::to_owned),
                chunk: events
                    .iter()
                    .map(|event| Raw::new(event).expect("raw").cast_unchecked())
                    .collect(),
                ..Default::default()
            })
            .mock_once()
            .mount()
            .await;
    }

    let page = read_room_messages(
        Arc::clone(&session),
        room_id.as_str(),
        4,
        Some("page-start".into()),
    )
    .await
    .expect("first page");
    assert_eq!(
        page.messages.len(),
        3,
        "only the membership event is skipped"
    );
    assert_eq!(
        page.messages
            .iter()
            .map(|item| (item.event_id.as_str(), item.timestamp_ms))
            .collect::<Vec<_>>(),
        vec![("$newest", 300), ("$middle", 200), ("$oldest", 100)]
    );
    assert_eq!(page.messages[0].body.as_deref(), Some("synthetic text"));
    assert_eq!(page.messages[1].body, None);
    assert!(page.messages[1].is_encrypted);
    assert_eq!(page.next.as_deref(), Some("page-older"));

    let older = read_room_messages(Arc::clone(&session), room_id.as_str(), 4, page.next)
        .await
        .expect("encrypted-only page");
    assert_eq!(older.messages.len(), 2);
    assert_eq!(
        older
            .messages
            .iter()
            .map(|item| item.event_id.as_str())
            .collect::<Vec<_>>(),
        vec!["$older", "$oldest-encrypted"]
    );
    assert!(
        older
            .messages
            .iter()
            .all(|item| item.body.is_none() && item.is_encrypted)
    );
    for item in page.messages.iter().chain(&older.messages) {
        assert_eq!(
            serde_json::to_value(item).expect("item")["content_available"],
            item.body.is_some()
        );
    }
    assert_eq!(older.next.as_deref(), Some("page-end"));

    let end = read_room_messages(session, room_id.as_str(), 4, older.next)
        .await
        .expect("end of history");
    assert!(end.messages.is_empty());
    assert_eq!(end.next, None);
}

#[test]
fn content_availability_matches_the_projected_body() {
    let mut empty = text_event("$empty", 100);
    empty["content"]["body"] = json!("  \n ");
    let unknown = json!({
        "type": "m.room.message", "event_id": "$unknown",
        "sender": "@alice:example.invalid", "origin_server_ts": 100,
        "content": { "msgtype": "m.location", "body": "synthetic", "geo_uri": "geo:1,2" }
    });
    let redacted = json!({
        "type": "m.room.message", "event_id": "$redacted",
        "sender": "@alice:example.invalid", "origin_server_ts": 100,
        "content": {}, "unsigned": { "redacted_because": {} }
    });
    for (value, available) in [
        (text_event("$text", 100), true),
        (empty, false),
        (unknown, false),
        (redacted, false),
    ] {
        let event = TimelineEvent::from_plaintext(Raw::new(&value).expect("raw").cast_unchecked());
        let item = item_for_event(&event).expect("message envelope");
        assert_eq!(item.body.is_some(), available);
        assert_eq!(
            serde_json::to_value(item).expect("item")["content_available"],
            available
        );
    }
}
