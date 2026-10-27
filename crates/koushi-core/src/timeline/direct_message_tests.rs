//! Headless tests for the direct (display-independent) message read and
//! actions (#1628).
//!
//! These run against a matrix mock homeserver, so the bounded read, the
//! relations page, and the guards are exercised without a GUI and without a
//! timeline actor.

use super::*;
use koushi_state::SessionInfo;
use matrix_sdk::ruma::event_id;
use matrix_sdk::ruma::serde::Raw;
use matrix_sdk::test_utils::mocks::MatrixMockServer;
use serde_json::json;

async fn joined_session() -> (MatrixMockServer, Arc<MatrixClientSession>, String) {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = "!room:example.invalid".to_owned();
    let parsed = matrix_sdk::ruma::RoomId::parse(&room_id).expect("room id");
    server.sync_joined_room(&client, &parsed).await;
    let session = MatrixClientSession::from_client_for_testing(
        client.clone(),
        SessionInfo {
            homeserver: server.server().uri(),
            user_id: client.user_id().expect("user id").to_string(),
            device_id: client.device_id().expect("device id").to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    );
    (server, Arc::new(session), room_id)
}

fn plain(value: serde_json::Value) -> TimelineEvent {
    let raw = Raw::new(&value).expect("raw event");
    TimelineEvent::from_plaintext(raw.cast_unchecked())
}

fn raw_event(value: serde_json::Value) -> Raw<matrix_sdk::ruma::events::AnyTimelineEvent> {
    let raw = Raw::new(&value).expect("raw event");
    raw.cast_unchecked()
}

fn text_event(event_id: &str, sender: &str, timestamp: u64, body: &str) -> serde_json::Value {
    json!({
        "type": "m.room.message",
        "event_id": event_id,
        "sender": sender,
        "origin_server_ts": timestamp,
        "content": { "msgtype": "m.text", "body": body }
    })
}

/// #1628: a room this account has not joined fails closed, with no timeline
/// actor involved at all.
#[tokio::test]
async fn an_unjoined_room_fails_closed() {
    let (server, _session, _room_id) = joined_session().await;
    let client = server.client_builder().build().await;
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client.clone(),
        SessionInfo {
            homeserver: server.server().uri(),
            user_id: client.user_id().expect("user id").to_string(),
            device_id: client.device_id().expect("device id").to_string(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    ));
    let failure = read_direct_message(session, "!not-joined:example.invalid", "$event")
        .await
        .expect_err("an unjoined room must fail");
    assert_eq!(failure, DirectMessageFailure::RoomNotFound);
}

/// A state event is not a message: the caller asked for something this tool
/// cannot read or act on.
#[tokio::test]
async fn a_state_event_target_is_not_a_message() {
    let (server, session, room_id) = joined_session().await;
    let membership = plain(json!({
        "type": "m.room.member",
        "event_id": "$membership",
        "sender": "@alice:example.invalid",
        "state_key": "@alice:example.invalid",
        "origin_server_ts": 1_700_000_000_000u64,
        "content": { "membership": "join" }
    }));
    server
        .mock_room_event()
        .ok(membership)
        .mock_once()
        .mount()
        .await;

    let failure = read_direct_message(session, &room_id, "$membership")
        .await
        .expect_err("a state event is not a message");
    assert_eq!(failure, DirectMessageFailure::NotAMessage);
}

/// The body is the text the timeline would show: a reply's quoted fallback is
/// removed, so a summary does not repeat the quoted message.
#[tokio::test]
async fn a_reply_body_has_its_fallback_removed() {
    let (server, session, room_id) = joined_session().await;
    let reply = plain(json!({
        "type": "m.room.message",
        "event_id": "$reply",
        "sender": "@bob:example.invalid",
        "origin_server_ts": 1_700_000_500_000u64,
        "content": {
            "msgtype": "m.text",
            "body": "> <@alice:example.invalid> quoted root\n\nplain answer",
            "m.relates_to": { "m.in_reply_to": { "event_id": "$root" } }
        }
    }));
    server.mock_room_event().ok(reply).mock_once().mount().await;
    server
        .mock_room_relations()
        .ok(Default::default())
        .mock_once()
        .mount()
        .await;

    let view = read_direct_message(session, &room_id, "$reply")
        .await
        .expect("read");
    assert_eq!(view.body.as_deref(), Some("plain answer"));
    assert!(view.content_available);
    assert_eq!(view.reply_to_event_id.as_deref(), Some("$root"));
    assert_eq!(view.thread_root_event_id, None);
    assert!(!view.relations_truncated);
    assert!(view.reactions.is_empty());
}

/// Only the message's own sender may edit or delete it (#1628).
#[tokio::test]
async fn another_senders_message_cannot_be_edited_or_deleted() {
    let (server, session, room_id) = joined_session().await;
    let own = session.client().user_id().expect("user id").to_string();
    let event = plain(text_event(
        "$theirs",
        "@someone-else:example.invalid",
        1_700_000_100_000,
        "not mine",
    ));
    assert_ne!(own, "@someone-else:example.invalid");
    server.mock_room_event().ok(event.clone()).mount().await;
    server
        .mock_room_relations()
        .ok(Default::default())
        .mount()
        .await;

    let edit = edit_direct(
        Arc::clone(&session),
        &room_id,
        "$theirs",
        "changed",
        koushi_state::ComposerFormattingOptions::default(),
    )
    .await
    .expect_err("another sender's message cannot be edited");
    assert_eq!(edit, DirectMessageFailure::NotAuthor);

    let delete = delete_direct(session, &room_id, "$theirs")
        .await
        .expect_err("another sender's message cannot be deleted");
    assert_eq!(delete, DirectMessageFailure::NotAuthor);
}

/// Deleting an already redacted message is a success that sends nothing.
#[tokio::test]
async fn deleting_an_already_redacted_message_is_a_success() {
    let (server, session, room_id) = joined_session().await;
    let own = session.client().user_id().expect("user id").to_string();
    let redacted = plain(json!({
        "type": "m.room.message",
        "event_id": "$redacted",
        "sender": own,
        "origin_server_ts": 1_700_000_200_000u64,
        "content": {},
        "unsigned": { "redacted_because": { "type": "m.room.redaction" } }
    }));
    server
        .mock_room_event()
        .ok(redacted)
        .mock_once()
        .mount()
        .await;

    let outcome = delete_direct(session, &room_id, "$redacted")
        .await
        .expect("an already redacted message is a success");
    assert!(outcome.already_redacted);
    assert_eq!(outcome.redaction_event_id, None);
}

/// A reaction whose set cannot be established from one bounded relations page
/// fails instead of risking a duplicate (#1628).
#[tokio::test]
async fn a_truncated_relations_page_blocks_the_reaction_set_decision() {
    let (server, session, room_id) = joined_session().await;
    let event = plain(text_event(
        "$target",
        "@bob:example.invalid",
        1_700_000_300_000,
        "react to me",
    ));
    server.mock_room_event().ok(event).mock_once().mount().await;
    // A full page with a continuation token is the only uncertain shape; a
    // shorter page means the server had nothing more to return.
    let full_page: Vec<_> = (0..256)
        .map(|index| {
            raw_event(json!({
                "type": "m.reaction",
                "event_id": format!("$other-{index}"),
                "sender": format!("@user{index}:example.invalid"),
                "origin_server_ts": 1_700_000_300_000u64,
                "content": {
                    "m.relates_to": {
                        "rel_type": "m.annotation", "event_id": "$target", "key": "🎉"
                    }
                }
            }))
        })
        .collect();
    server
        .mock_room_relations()
        .ok(
            matrix_sdk::test_utils::mocks::RoomRelationsResponseTemplate::default()
                .events(full_page)
                .next_batch("more-relations-remain"),
        )
        .mock_once()
        .mount()
        .await;

    let failure = react_direct(session, &room_id, "$target", "👍")
        .await
        .expect_err("a full page cannot prove the reaction is absent");
    assert_eq!(failure, DirectMessageFailure::RelationsUnavailable);
}

/// The newest valid replacement decides the displayed text, and an invalid one
/// is ignored the way the timeline ignores it.
#[test]
fn the_latest_valid_edit_wins_and_an_invalid_one_is_ignored() {
    let target = plain(text_event(
        "$root",
        "@alice:example.invalid",
        1_700_000_000_000,
        "original",
    ));
    let valid = plain(json!({
        "type": "m.room.message",
        "event_id": "$edit-1",
        "sender": "@alice:example.invalid",
        "origin_server_ts": 1_700_000_100_000u64,
        "content": {
            "m.new_content": { "msgtype": "m.text", "body": "first edit" },
            "m.relates_to": { "rel_type": "m.replace", "event_id": "$root" }
        }
    }));
    let forged_sender = plain(json!({
        "type": "m.room.message",
        "event_id": "$edit-2",
        "sender": "@mallory:example.invalid",
        "origin_server_ts": 1_700_000_200_000u64,
        "content": {
            "m.new_content": { "msgtype": "m.text", "body": "forged edit" },
            "m.relates_to": { "rel_type": "m.replace", "event_id": "$root" }
        }
    }));

    let relations = [valid.clone(), forged_sender];
    let latest = latest_valid_edit(&target, &relations).expect("one valid replacement");
    assert_eq!(latest.event_id(), Some(event_id!("$edit-1")));

    let none = latest_valid_edit(&target, &[]);
    assert!(none.is_none(), "no replacement means no edit");
}

/// Reaction groups count every sender and mark this account's own, including a
/// duplicate own reaction that `unreact` has to remove.
#[test]
fn reaction_groups_count_senders_and_mark_mine() {
    fn annotation(event_id: &str, sender: &str, key: &str) -> serde_json::Value {
        json!({
            "type": "m.reaction",
            "event_id": event_id,
            "sender": sender,
            "origin_server_ts": 1_700_000_000_000u64,
            "content": {
                "m.relates_to": { "rel_type": "m.annotation", "event_id": "$target", "key": key }
            }
        })
    }
    let own = matrix_sdk::ruma::OwnedUserId::try_from("@me:example.invalid").expect("user id");
    let relations = vec![
        plain(annotation("$r1", "@bob:example.invalid", "👍")),
        plain(annotation("$r2", "@me:example.invalid", "👍")),
        plain(annotation("$r3", "@me:example.invalid", "👍")),
        plain(annotation("$r4", "@bob:example.invalid", "🎉")),
    ];

    let groups = reaction_groups(&relations, Some(&own));
    let thumbs = groups
        .iter()
        .find(|group| group.key == "👍")
        .expect("thumb");
    assert_eq!(
        thumbs.count, 2,
        "one sender counts once per key, like the timeline's own projection"
    );
    assert!(thumbs.mine);
    let party = groups
        .iter()
        .find(|group| group.key == "🎉")
        .expect("party");
    assert_eq!(party.count, 1);
    assert!(!party.mine);

    let (own_ids, unresolved) = own_reaction_event_ids(&relations, &own, "👍");
    assert!(!unresolved, "every relation here is readable");
    assert_eq!(
        own_ids.iter().map(ToString::to_string).collect::<Vec<_>>(),
        vec!["$r2", "$r3"],
        "every own reaction with the key has to be removable"
    );
}

/// A thread reply cannot be answered as if it were its thread's root.
#[tokio::test]
async fn a_thread_reply_target_is_not_a_thread_root() {
    let (server, session, room_id) = joined_session().await;
    let reply = plain(json!({
        "type": "m.room.message",
        "event_id": "$thread-reply",
        "sender": "@bob:example.invalid",
        "origin_server_ts": 1_700_000_400_000u64,
        "content": {
            "msgtype": "m.text",
            "body": "already in the thread",
            "m.relates_to": { "rel_type": "m.thread", "event_id": "$root" }
        }
    }));
    server.mock_room_event().ok(reply).mount().await;

    let failure = thread_reply_direct(
        session,
        &room_id,
        "$thread-reply",
        "another answer",
        koushi_state::ComposerFormattingOptions::default(),
    )
    .await
    .expect_err("a thread reply is not a thread root");
    assert_eq!(failure, DirectMessageFailure::NotThreadRoot);
}

/// A deleted thread root cannot be answered.
#[tokio::test]
async fn a_redacted_thread_root_is_refused() {
    let (server, session, room_id) = joined_session().await;
    let own = session.client().user_id().expect("user id").to_string();
    let redacted = plain(json!({
        "type": "m.room.message",
        "event_id": "$root",
        "sender": own,
        "origin_server_ts": 1_700_000_400_100u64,
        "content": {},
        "unsigned": { "redacted_because": { "type": "m.room.redaction" } }
    }));
    server.mock_room_event().ok(redacted).mount().await;

    let failure = thread_reply_direct(
        session,
        &room_id,
        "$root",
        "answer",
        koushi_state::ComposerFormattingOptions::default(),
    )
    .await
    .expect_err("a deleted root cannot be answered");
    assert_eq!(failure, DirectMessageFailure::Redacted);
}

/// A message this device cannot decrypt is a successful read with no body, not
/// "not a message".
#[tokio::test]
async fn an_undecryptable_message_reads_with_no_body() {
    let (server, session, room_id) = joined_session().await;
    let encrypted = plain(json!({
        "type": "m.room.encrypted",
        "event_id": "$encrypted",
        "sender": "@bob:example.invalid",
        "origin_server_ts": 1_700_000_600_000u64,
        "content": {
            "algorithm": "m.megolm.v1.aes-sha2",
            "ciphertext": "synthetic",
            "device_id": "SYNTHETIC",
            "sender_key": "synthetic",
            "session_id": "synthetic"
        }
    }));
    server
        .mock_room_event()
        .ok(encrypted)
        .mock_once()
        .mount()
        .await;
    server
        .mock_room_relations()
        .ok(Default::default())
        .mock_once()
        .mount()
        .await;

    let view = read_direct_message(session, &room_id, "$encrypted")
        .await
        .expect("an unreadable message is still a readable event");
    assert_eq!(view.body, None);
    assert!(!view.content_available);
    assert!(view.is_encrypted);
    assert!(!view.is_redacted);
    assert_eq!(view.sender_id, "@bob:example.invalid");
}
