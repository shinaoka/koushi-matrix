use super::{
    MatrixConversationActivity, MatrixConversationActivitySource, MatrixRoomTagInfo,
    MatrixRoomTags, SdkUnreadTrace, SpaceMemberLookupStatus, classify_space_member_ids,
    matrix_conversation_activity_from_timeline_event, matrix_conversation_activity_source,
    matrix_room_latest_event_projection, matrix_room_list_room_from_counts,
    matrix_timeline_event_is_redacted, newest_conversation_activity, people_scope_diagnostic_event,
    space_members_scope_diagnostic_event, trace_sdk_unread_snapshot,
};

use koushi_diagnostics::DiagnosticValue;
use std::collections::BTreeMap;
#[test]
fn people_scope_diagnostic_distinguishes_direct_space_members_from_child_room_aggregate() {
    let event = people_scope_diagnostic_event(true, 7, 3);

    assert_eq!(event.source, "sdk.people_scope");
    assert_eq!(event.stage, "member_snapshot");
    let field = |key| {
        event
            .fields
            .iter()
            .find(|field| field.key == key)
            .map(|field| &field.value)
    };
    assert_eq!(
        field("source"),
        Some(&koushi_diagnostics::DiagnosticValue::Token(
            "direct_space_members"
        ))
    );
    assert_eq!(
        field("direct_member_count"),
        Some(&koushi_diagnostics::DiagnosticValue::Count(7))
    );
    assert_eq!(
        field("child_room_count"),
        Some(&koushi_diagnostics::DiagnosticValue::Count(3))
    );
    assert_eq!(
        field("child_room_members_included"),
        Some(&koushi_diagnostics::DiagnosticValue::Boolean(false))
    );
}
#[test]
fn conversation_activity_classifies_messages_encryption_and_threads_only() {
    use MatrixConversationActivitySource::{EncryptedMessage, Message, ThreadReply};

    let cases = [
        ("m.room.message", None, Some(Message)),
        ("m.room.message", Some("m.in_reply_to"), Some(Message)),
        ("m.room.message", Some("m.thread"), Some(ThreadReply)),
        ("m.room.encrypted", None, Some(EncryptedMessage)),
        ("m.room.encrypted", Some("m.thread"), Some(ThreadReply)),
        ("m.room.message", Some("m.replace"), None),
        ("m.room.message", Some("m.annotation"), None),
        ("m.room.redaction", None, None),
        ("m.room.member", None, None),
        ("m.room.name", None, None),
        ("m.reaction", Some("m.annotation"), None),
        ("m.receipt", None, None),
        ("m.typing", None, None),
        ("m.presence", None, None),
    ];

    for (event_type, relation_type, expected) in cases {
        assert_eq!(
            matrix_conversation_activity_source(event_type, relation_type),
            expected,
            "unexpected classification for {event_type} / {relation_type:?}"
        );
    }
}
#[test]
fn redacted_raw_latest_is_classified_without_private_diagnostics() {
    let event = matrix_sdk::deserialized_responses::TimelineEvent::from_plaintext(
        matrix_sdk::ruma::serde::Raw::from_json_string(
            serde_json::json!({
                    "content": {"body": "redacted body", "msgtype": "m.text"},
                    "event_id": "$redacted:example.invalid",
                    "origin_server_ts": 42,
                    "sender": "@sender:example.invalid",
                    "type": "m.room.message",
                    "unsigned": {"redacted_because": {"type": "m.room.redaction"}}
            })
            .to_string(),
        )
        .expect("redacted synthetic timeline event"),
    );

    assert!(matrix_timeline_event_is_redacted(&event));
}

#[test]
fn malformed_unsigned_latest_fails_closed_as_redacted() {
    let event = matrix_sdk::deserialized_responses::TimelineEvent::from_plaintext(
        matrix_sdk::ruma::serde::Raw::from_json_string(
            serde_json::json!({
                    "content": {"body": "private body", "msgtype": "m.text"},
                    "event_id": "$private:example.invalid",
                    "origin_server_ts": 42,
                    "sender": "@private:example.invalid",
                    "type": "m.room.message",
                    "unsigned": "malformed"
            })
            .to_string(),
        )
        .expect("malformed unsigned synthetic timeline event"),
    );

    assert!(matrix_timeline_event_is_redacted(&event));
    assert!(matrix_conversation_activity_from_timeline_event(&event).is_none());
}

#[test]
fn redacted_cached_latest_does_not_create_conversation_activity() {
    let event = matrix_sdk::deserialized_responses::TimelineEvent::from_plaintext(
        matrix_sdk::ruma::serde::Raw::from_json_string(
            serde_json::json!({
                    "content": {"body": "redacted body", "msgtype": "m.text"},
                    "event_id": "$redacted:example.invalid",
                    "origin_server_ts": 42,
                    "sender": "@sender:example.invalid",
                    "type": "m.room.message",
                    "unsigned": {"redacted_because": {"type": "m.room.redaction"}}
            })
            .to_string(),
        )
        .expect("redacted synthetic timeline event"),
    );

    assert!(matrix_conversation_activity_from_timeline_event(&event).is_none());
}

#[tokio::test]
async fn remote_replacement_latest_keeps_original_identity_and_order() {
    use matrix_sdk::ruma::{event_id, room_id, user_id};
    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};

    let room_id = room_id!("!room:example.invalid");
    let sender = user_id!("@sender:example.invalid");
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    client
        .event_cache()
        .subscribe()
        .expect("event cache subscription");

    let original_factory = EventFactory::new().room(room_id).sender(sender);
    let replacement_factory = EventFactory::new().room(room_id).sender(sender);
    let original = original_factory
        .server_ts(42)
        .text_msg("before")
        .event_id(event_id!("$original:example.invalid"))
        .into_raw_sync();
    let replacement = replacement_factory
        .server_ts(99)
        .text_msg("fallback edit body")
        .event_id(event_id!("$edit:example.invalid"))
        .edit(
            event_id!("$original:example.invalid"),
            matrix_sdk::ruma::events::room::message::RoomMessageEventContent::text_plain("after")
                .into(),
        )
        .into_raw_sync();

    let room = server
        .sync_room(&client, JoinedRoomBuilder::new(room_id))
        .await;
    let mut latest_events = client
        .latest_events()
        .await
        .listen_and_subscribe_to_room(room_id)
        .await
        .expect("latest event subscription")
        .expect("latest event stream");
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(room_id).add_timeline_bulk(vec![original, replacement]),
        )
        .await;
    tokio::time::timeout(std::time::Duration::from_secs(1), latest_events.next())
        .await
        .expect("latest event update must be event-driven");

    let (latest, activity) = matrix_room_latest_event_projection(&room).await;
    let latest = latest.expect("latest replacement projection");
    assert_eq!(latest.event_id, "$original:example.invalid");
    assert_eq!(latest.sender_id.as_deref(), Some("@sender:example.invalid"));
    assert_eq!(latest.timestamp_ms, 42);
    assert_eq!(latest.relation_type, None);
    assert_eq!(latest.relation_event_id, None);
    assert_eq!(latest.preview.as_deref(), Some("after"));
    assert_eq!(activity.map(|activity| activity.timestamp_ms), Some(42));
}

/// #997: a cold room's Activity row is built from this latest-event summary.
/// Its thread root decides whether the row opens the Thread panel, so it must
/// come from the event's own `m.thread` relation (the SDK's raw
/// `TimelineItemContent::from_event` never fills it in).
#[tokio::test]
async fn thread_reply_latest_event_carries_its_thread_root() {
    use matrix_sdk::ruma::{event_id, room_id, user_id};
    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};

    let room_id = room_id!("!threads:example.invalid");
    let sender = user_id!("@sender:example.invalid");
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    client
        .event_cache()
        .subscribe()
        .expect("event cache subscription");

    let root = EventFactory::new()
        .room(room_id)
        .sender(sender)
        .server_ts(42)
        .text_msg("root")
        .event_id(event_id!("$root:example.invalid"))
        .into_raw_sync();
    let reply = EventFactory::new()
        .room(room_id)
        .sender(sender)
        .server_ts(99)
        .text_msg("reply in thread")
        .in_thread(
            event_id!("$root:example.invalid"),
            event_id!("$root:example.invalid"),
        )
        .event_id(event_id!("$reply:example.invalid"))
        .into_raw_sync();

    let room = server
        .sync_room(&client, JoinedRoomBuilder::new(room_id))
        .await;
    let mut latest_events = client
        .latest_events()
        .await
        .listen_and_subscribe_to_room(room_id)
        .await
        .expect("latest event subscription")
        .expect("latest event stream");
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(room_id).add_timeline_bulk(vec![root, reply]),
        )
        .await;
    tokio::time::timeout(std::time::Duration::from_secs(1), latest_events.next())
        .await
        .expect("latest event update must be event-driven");

    let (latest, _) = matrix_room_latest_event_projection(&room).await;
    let latest = latest.expect("latest thread reply projection");
    assert_eq!(latest.event_id, "$reply:example.invalid");
    assert_eq!(latest.relation_type.as_deref(), Some("m.thread"));
    assert_eq!(
        latest.thread_root_event_id.as_deref(),
        Some("$root:example.invalid")
    );
}

#[test]
fn conversation_activity_keeps_the_newest_cache_or_local_candidate() {
    let cached = super::MatrixConversationActivity {
        timestamp_ms: 41,
        source: MatrixConversationActivitySource::EncryptedMessage,
    };
    let local = super::MatrixConversationActivity {
        timestamp_ms: 42,
        source: MatrixConversationActivitySource::Message,
    };

    assert_eq!(
        newest_conversation_activity(Some(cached), Some(local)),
        Some(local)
    );
    assert_eq!(
        newest_conversation_activity(Some(cached), None),
        Some(cached)
    );
    assert_eq!(newest_conversation_activity(None, None), None);
}
#[test]
fn conversation_activity_debug_hides_raw_timestamp() {
    let activity = MatrixConversationActivity {
        timestamp_ms: 42,
        source: MatrixConversationActivitySource::ThreadReply,
    };

    let debug = format!("{activity:?}");

    assert!(debug.contains("ThreadReply"), "{debug}");
    assert!(!debug.contains("42"), "{debug}");
}
#[test]
fn space_member_facts_separate_join_invite_and_child_only() {
    let facts = classify_space_member_ids(
        ["joined", "both"],
        ["invited"],
        [
            ("child-a", ["child-only", "both"]),
            ("child-b", ["child-only", "second-only"]),
        ],
    );

    assert_eq!(facts.space_joined_ids, vec!["both", "joined"]);
    assert_eq!(facts.space_invited_ids, vec!["invited"]);
    assert_eq!(facts.child_room_only_ids, vec!["child-only", "second-only"]);
    assert_eq!(facts.child_join_union_count, 3);
    assert_eq!(facts.duplicate_child_membership_count, 1);
    assert_eq!(
        facts.child_room_ids.get("child-only"),
        Some(&vec!["child-a".to_owned(), "child-b".to_owned()])
    );
}
#[test]
fn space_members_scope_diagnostic_is_private_data_free() {
    let event = space_members_scope_diagnostic_event(
        "observed",
        SpaceMemberLookupStatus::Observed(2),
        SpaceMemberLookupStatus::Observed(1),
        Some(3),
        Some(2),
        Some(1),
        Some(2),
        Some(1),
        Some(4),
        Some(4),
        Some(1),
        Some(2),
        1,
        0,
    );

    assert_eq!(event.source, "sdk.space_members_scope");
    let rendered = format!("{event:?}");
    for forbidden in [
        "!space:example.invalid",
        "!child:example.invalid",
        "@alice:example.invalid",
        "Alice",
        "mxc://example.invalid/avatar",
        "raw sdk error",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "diagnostic leaked private data: {forbidden}"
        );
    }
}
#[test]
fn failed_space_member_lookup_does_not_report_zero_counts() {
    let event = space_members_scope_diagnostic_event(
        "observed",
        SpaceMemberLookupStatus::Failed,
        SpaceMemberLookupStatus::NotAttempted,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        0,
        1,
    );

    assert!(event.fields.iter().any(|field| {
        field.key == "space_join_lookup_outcome"
            && field.value == DiagnosticValue::Token("lookup_failed")
    }));
    assert!(event.fields.iter().any(|field| {
        field.key == "space_join_count_availability"
            && field.value == DiagnosticValue::Token("counts_unavailable")
    }));
    assert!(event.fields.iter().any(|field| {
        field.key == "space_invite_lookup_outcome"
            && field.value == DiagnosticValue::Token("not_attempted")
    }));
    for field in &event.fields {
        if matches!(
            field.key,
            "space_joined_count"
                | "space_invited_count"
                | "child_room_count"
                | "child_room_only_count"
                | "input_count"
                | "output_count"
        ) {
            assert_ne!(
                field.value,
                DiagnosticValue::Count(0),
                "unobserved Space counts must not be fabricated as zero"
            );
        }
    }
}
#[test]
fn direct_account_data_targets_are_indexed_by_room() {
    use matrix_sdk::ruma::{
        OwnedRoomId, OwnedUserId,
        events::direct::{DirectEventContent, OwnedDirectUserIdentifier},
    };

    let alice: OwnedUserId = "@alice:example.invalid".try_into().unwrap();
    let bob: OwnedUserId = "@bob:example.invalid".try_into().unwrap();
    let dm_room: OwnedRoomId = "!dm:example.invalid".try_into().unwrap();
    let other_room: OwnedRoomId = "!other:example.invalid".try_into().unwrap();
    let mut content = DirectEventContent::default();
    content.insert(
        OwnedDirectUserIdentifier::from(alice),
        vec![dm_room.clone(), other_room.clone()],
    );
    content.insert(OwnedDirectUserIdentifier::from(bob), vec![dm_room.clone()]);

    let by_room = super::direct_account_data_targets_by_room(&content);

    assert_eq!(
        by_room.get(dm_room.as_str()),
        Some(&vec![
            "@alice:example.invalid".to_owned(),
            "@bob:example.invalid".to_owned()
        ])
    );
    assert_eq!(
        by_room.get(other_room.as_str()),
        Some(&vec!["@alice:example.invalid".to_owned()])
    );
}
#[test]
fn direct_account_data_targets_are_sorted_deduplicated_and_indexed_by_room() {
    use matrix_sdk::ruma::{
        OwnedRoomId, OwnedUserId,
        events::direct::{DirectEventContent, OwnedDirectUserIdentifier},
    };

    let alice: OwnedUserId = "@alice:example.invalid".try_into().unwrap();
    let bob: OwnedUserId = "@bob:example.invalid".try_into().unwrap();
    let room: OwnedRoomId = "!dm:example.invalid".try_into().unwrap();
    let mut content = DirectEventContent::default();
    content.insert(OwnedDirectUserIdentifier::from(bob), vec![room.clone()]);
    content.insert(OwnedDirectUserIdentifier::from(alice), vec![room.clone()]);

    assert_eq!(
        super::direct_account_data_targets_by_room(&content),
        BTreeMap::from([(
            room.to_string(),
            vec![
                "@alice:example.invalid".to_owned(),
                "@bob:example.invalid".to_owned(),
            ],
        )]),
    );
}
#[tokio::test]
async fn explicit_empty_direct_map_overrides_cached_room_direct_targets() {
    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    use matrix_sdk_test::JoinedRoomBuilder;
    use serde_json::json;

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let target = matrix_sdk::ruma::user_id!("@dm-target:example.org");
    let room_id = matrix_sdk::ruma::room_id!("!stale-dm:example.org");

    server
        .mock_sync()
        .ok_and_run(&client, |builder| {
            builder.add_custom_global_account_data(json!({
                    "type": "m.direct",
                    "content": { target: [room_id] }
            }));
            builder.add_joined_room(JoinedRoomBuilder::new(room_id));
        })
        .await;

    let room = client.get_room(room_id).expect("joined test room");
    assert!(
        !room.direct_targets().is_empty(),
        "test room must have cached direct targets"
    );

    let direct_targets_by_room = BTreeMap::new();
    let snapshot = super::room_list_snapshot_from_sdk_rooms_with_direct_targets(
        std::iter::once(room),
        Some(&direct_targets_by_room),
    )
    .await;

    assert_eq!(snapshot.rooms.len(), 1);
    assert!(!snapshot.rooms[0].is_dm);
}
/// #1176: a cold-start room list caches a single event per room, so the SDK's
/// client-side counters are a lower bound while no own read receipt is anchored
/// in (and no receipt is left unmatched against) the loaded timeline. The
/// server counts that are already in hand must top the projection up.
async fn mock_room_with_unread_state(
    room_id: &matrix_sdk::ruma::RoomId,
    receipts: matrix_sdk_base::read_receipts::ReadReceipts,
    server_counts: matrix_sdk_base::sync::UnreadNotificationsCount,
) -> (
    matrix_sdk::test_utils::mocks::MatrixMockServer,
    matrix_sdk::Room,
) {
    use matrix_sdk::test_utils::mocks::MatrixMockServer;

    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room = server.sync_joined_room(&client, room_id).await;
    room.update_room_info(|mut info| {
        info.set_read_receipts(receipts);
        info.update_notification_count(server_counts);
        (
            info,
            matrix_sdk_base::RoomInfoNotableUpdateReasons::READ_RECEIPT,
        )
    })
    .await;
    (server, room)
}

async fn projected_single_room(room: matrix_sdk::Room) -> super::MatrixRoomListRoom {
    super::room_list_snapshot_from_sdk_rooms(vec![room])
        .await
        .rooms
        .into_iter()
        .next()
        .expect("one joined room must project into the room list")
}

fn read_receipts(
    num_unread: u64,
    num_notifications: u64,
    num_mentions: u64,
    latest_active: Option<&str>,
) -> matrix_sdk_base::read_receipts::ReadReceipts {
    matrix_sdk_base::read_receipts::ReadReceipts {
        num_unread,
        num_notifications,
        num_mentions,
        latest_active: latest_active.map(|event_id| {
            matrix_sdk_base::read_receipts::LatestReadReceipt {
                event_id: matrix_sdk::ruma::OwnedEventId::try_from(event_id)
                    .expect("test receipt event id"),
            }
        }),
        ..Default::default()
    }
}

fn server_counts(
    notification_count: u64,
    highlight_count: u64,
) -> matrix_sdk_base::sync::UnreadNotificationsCount {
    matrix_sdk_base::sync::UnreadNotificationsCount {
        notification_count,
        highlight_count,
    }
}

#[tokio::test]
async fn cold_start_unread_lower_bound_is_topped_up_by_server_counts() {
    use matrix_sdk::ruma::room_id;

    let (_server, room) = mock_room_with_unread_state(
        room_id!("!cold-unread:example.invalid"),
        read_receipts(1, 1, 0, None),
        server_counts(3, 0),
    )
    .await;

    let projected = projected_single_room(room).await;
    assert_eq!(projected.unread_count, 3);
    assert_eq!(projected.notification_count, 3);
    assert_eq!(projected.highlight_count, 0);
}

#[tokio::test]
async fn unmatched_pending_receipts_also_take_the_server_counts() {
    use matrix_sdk::ruma::room_id;

    let mut receipts = read_receipts(1, 0, 0, Some("$anchored:example.invalid"));
    receipts
        .pending
        .push(matrix_sdk::ruma::OwnedEventId::try_from("$unmatched:example.invalid").unwrap());
    let (_server, room) = mock_room_with_unread_state(
        room_id!("!pending-receipt:example.invalid"),
        receipts,
        server_counts(2, 0),
    )
    .await;

    let projected = projected_single_room(room).await;
    assert_eq!(projected.unread_count, 2);
}

#[tokio::test]
async fn anchored_own_receipt_keeps_client_counts_over_higher_server_counts() {
    use matrix_sdk::ruma::room_id;

    // The server values are deliberately higher: an anchored client receipt must
    // not be topped up at all, or a stale server count would inflate a room the
    // client has already accounted for.
    let (_server, room) = mock_room_with_unread_state(
        room_id!("!anchored-receipt:example.invalid"),
        read_receipts(3, 3, 1, Some("$anchored:example.invalid")),
        server_counts(9, 9),
    )
    .await;

    let projected = projected_single_room(room).await;
    assert_eq!(projected.unread_count, 3);
    assert_eq!(projected.notification_count, 3);
    assert_eq!(projected.highlight_count, 1);
}

#[tokio::test]
async fn client_counts_above_the_server_keep_the_client_values() {
    use matrix_sdk::ruma::room_id;

    let (_server, room) = mock_room_with_unread_state(
        room_id!("!client-above-server:example.invalid"),
        read_receipts(4, 4, 2, None),
        server_counts(2, 1),
    )
    .await;

    let projected = projected_single_room(room).await;
    assert_eq!(projected.unread_count, 4);
    assert_eq!(projected.notification_count, 4);
    assert_eq!(projected.highlight_count, 2);
}

/// #1176 ordering: the server top-up runs before the read-marker suppression,
/// so a stale positive server count cannot outlive a marker that covers the
/// projected latest event.
#[tokio::test]
async fn stale_server_counts_still_lose_to_a_matching_read_marker() {
    use matrix_sdk::ruma::{event_id, room_id, serde::Raw, user_id};
    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};

    let room_id = room_id!("!stale-server-counts:example.invalid");
    let sender = user_id!("@sender:example.invalid");
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    client
        .event_cache()
        .subscribe()
        .expect("event cache subscription");
    let room = server
        .sync_room(&client, JoinedRoomBuilder::new(room_id))
        .await;

    let message = EventFactory::new()
        .room(room_id)
        .sender(sender)
        .server_ts(42)
        .text_msg("already read")
        .event_id(event_id!("$read-msg:example.invalid"))
        .into_raw_sync();
    let fully_read: Raw<matrix_sdk::ruma::events::AnyRoomAccountDataEvent> = Raw::from_json(
        serde_json::value::to_raw_value(&serde_json::json!({
            "type": "m.fully_read",
            "content": { "event_id": "$read-msg:example.invalid" }
        }))
        .expect("raw fully read marker"),
    );
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(room_id)
                .add_timeline_bulk(vec![message])
                .add_account_data(fully_read),
        )
        .await;
    // Wait for the fixture itself, so a broken marker/event setup fails as that,
    // not as an unrelated count mismatch.
    let mut projected_event_id = None;
    for _ in 0..100 {
        projected_event_id = super::matrix_room_latest_event_projection(&room)
            .await
            .0
            .map(|latest| latest.event_id);
        if projected_event_id.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(
        projected_event_id.as_deref(),
        Some("$read-msg:example.invalid")
    );
    assert_eq!(
        super::matrix_room_fully_read_event_id(&room)
            .await
            .as_deref(),
        Some("$read-msg:example.invalid")
    );

    room.update_room_info(|mut info| {
        info.set_read_receipts(read_receipts(1, 1, 0, None));
        info.update_notification_count(server_counts(5, 0));
        (
            info,
            matrix_sdk_base::RoomInfoNotableUpdateReasons::READ_RECEIPT,
        )
    })
    .await;

    let projected = projected_single_room(room).await;
    assert_eq!(projected.unread_count, 0);
    assert_eq!(projected.notification_count, 0);
    assert_eq!(projected.highlight_count, 0);
}

#[tokio::test]
async fn dummy_zero_server_counts_leave_client_counts_alone() {
    use matrix_sdk::ruma::room_id;

    let (_server, room) = mock_room_with_unread_state(
        room_id!("!synapse-dummy-counts:example.invalid"),
        read_receipts(2, 2, 1, None),
        server_counts(0, 0),
    )
    .await;

    let projected = projected_single_room(room).await;
    assert_eq!(projected.unread_count, 2);
    assert_eq!(projected.notification_count, 2);
    assert_eq!(projected.highlight_count, 1);
}

#[tokio::test]
async fn server_highlight_count_is_projected_when_client_accounting_is_incomplete() {
    use matrix_sdk::ruma::room_id;

    let (_server, room) = mock_room_with_unread_state(
        room_id!("!server-highlight:example.invalid"),
        read_receipts(0, 0, 0, None),
        server_counts(0, 2),
    )
    .await;

    let projected = projected_single_room(room).await;
    assert_eq!(projected.unread_count, 0);
    assert_eq!(projected.notification_count, 0);
    assert_eq!(projected.highlight_count, 2);
}

#[test]
fn room_list_room_from_counts_keeps_manual_unread_over_a_covered_marker() {
    // The server fallback passes nonzero counts into the constructor; a manual
    // mark must keep them instead of being zeroed by the marker suppression.
    let room = matrix_room_list_room_from_counts(
        "!room:example.invalid".to_owned(),
        "Room".to_owned(),
        None,
        false,
        Vec::new(),
        MatrixRoomTags::default(),
        5,
        0,
        5,
        true,
        None,
        None,
        Some(test_latest_event("$event:example.invalid")),
        Some("$event:example.invalid".to_owned()),
        None,
        vec![],
        false,
        0,
    );

    assert_eq!(room.unread_count, 5);
    assert_eq!(room.notification_count, 5);
    assert!(room.marked_unread);
}

#[test]
fn room_list_room_from_counts_carries_notification_metadata() {
    let room = matrix_room_list_room_from_counts(
        "!room:example.invalid".to_owned(),
        "Room".to_owned(),
        None,
        true,
        vec!["@alice:example.invalid".to_owned()],
        MatrixRoomTags::default(),
        4,
        2,
        2,
        false,
        None,
        None,
        None,
        None,
        None,
        vec!["!space:example.invalid".to_owned()],
        false,
        2,
    );

    assert_eq!(room.notification_count, 4);
    assert_eq!(room.highlight_count, 2);
    assert_eq!(room.unread_count, 2);
    assert_eq!(room.joined_members, 2);
    assert!(room.is_dm);
}
#[test]
fn room_list_room_from_counts_does_not_turn_manual_unread_into_messages() {
    let room = matrix_room_list_room_from_counts(
        "!room:example.invalid".to_owned(),
        "Room".to_owned(),
        None,
        false,
        Vec::new(),
        MatrixRoomTags::default(),
        0,
        0,
        0,
        true,
        None,
        None,
        None,
        None,
        None,
        vec![],
        false,
        0,
    );

    assert_eq!(room.unread_count, 0);
    assert!(room.marked_unread);
}
#[test]
fn room_list_room_from_counts_carries_room_tags() {
    let tags = MatrixRoomTags {
        favourite: Some(MatrixRoomTagInfo {
            order: Some("0.25".to_owned()),
        }),
        low_priority: None,
    };

    let room = matrix_room_list_room_from_counts(
        "!room:example.invalid".to_owned(),
        "Room".to_owned(),
        None,
        false,
        Vec::new(),
        tags.clone(),
        0,
        0,
        0,
        false,
        None,
        None,
        None,
        None,
        None,
        vec![],
        false,
        0,
    );

    assert_eq!(room.tags, tags);
}
#[test]
fn unread_diagnostic_snapshot_rejects_private_synthetic_inputs() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let latest_event = Some(crate::MatrixRoomLatestEventSummary {
        event_id: "$event:example.invalid".to_owned(),
        sender_id: Some("@user:example.invalid".to_owned()),
        sender_label: None,
        sender_avatar_mxc_uri: None,
        preview: Some("secret message".to_owned()),
        timestamp_ms: 42,
        event_type: Some("m.room.message".to_owned()),
        relation_type: None,
        relation_event_id: None,
        thread_root_event_id: None,
        content_converted: true,
        is_threaded: false,
        is_reply: false,
        has_thread_summary: false,
        has_reactions: false,
        is_redacted: false,
    });
    trace_sdk_unread_snapshot(SdkUnreadTrace {
        unread_messages: 2,
        unread_count: 2,
        notification_count: 1,
        highlight_count: 1,
        marked_unread: true,
        latest_event: &latest_event,
        fully_read_event_id: Some("$event:example.invalid"),
        private_read_receipt_event_id: None,
        recency_stamp_present: true,
        conversation_activity: Some(MatrixConversationActivity {
            timestamp_ms: 3,
            source: MatrixConversationActivitySource::Message,
        }),
    });
    let serialized = serde_json::to_string(&koushi_diagnostics::snapshot()).unwrap();
    assert!(serialized.contains("conversation_activity_source"));
    for forbidden in [
        "!room:example.invalid",
        "@user:example.invalid",
        "$event:example.invalid",
        "/Users/alice/private",
        "secret message",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "serialized diagnostics leaked {forbidden}"
        );
    }
    assert!(serialized.contains("unread_messages"));
    assert!(serialized.contains("latest_event_present"));
}
fn test_latest_event(event_id: &str) -> crate::MatrixRoomLatestEventSummary {
    crate::MatrixRoomLatestEventSummary {
        event_id: event_id.to_owned(),
        sender_id: None,
        sender_label: None,
        sender_avatar_mxc_uri: None,
        preview: None,
        timestamp_ms: 42,
        event_type: Some("m.room.message".to_owned()),
        relation_type: None,
        relation_event_id: None,
        thread_root_event_id: None,
        content_converted: true,
        is_threaded: false,
        is_reply: false,
        has_thread_summary: false,
        has_reactions: false,
        is_redacted: false,
    }
}
#[test]
fn room_list_room_from_counts_rejects_redacted_latest_as_read_marker_target() {
    let mut latest = test_latest_event("$latest:example.invalid");
    latest.is_redacted = true;
    let room = matrix_room_list_room_from_counts(
        "!room:example.invalid".to_owned(),
        "Room".to_owned(),
        None,
        false,
        Vec::new(),
        MatrixRoomTags::default(),
        2,
        1,
        2,
        false,
        Some(42),
        None,
        Some(latest),
        Some("$latest:example.invalid".to_owned()),
        None,
        vec![],
        false,
        2,
    );

    assert_eq!(room.unread_count, 2);
    assert_eq!(room.notification_count, 2);
    assert_eq!(room.highlight_count, 1);
}

#[test]
fn room_list_room_from_counts_suppresses_stale_unread_when_fully_read_matches_latest_event() {
    let room = matrix_room_list_room_from_counts(
        "!room:example.invalid".to_owned(),
        "Room".to_owned(),
        None,
        false,
        Vec::new(),
        MatrixRoomTags::default(),
        2,
        1,
        2,
        false,
        Some(42),
        None,
        Some(test_latest_event("$latest:example.invalid")),
        Some("$latest:example.invalid".to_owned()),
        None,
        vec![],
        false,
        2,
    );

    assert_eq!(room.unread_count, 0);
    assert_eq!(room.notification_count, 0);
    assert_eq!(room.highlight_count, 0);
    assert!(!room.marked_unread);
}
#[test]
fn room_list_room_from_counts_preserves_unread_when_read_marker_differs_from_latest_event() {
    let room = matrix_room_list_room_from_counts(
        "!room:example.invalid".to_owned(),
        "Room".to_owned(),
        None,
        false,
        Vec::new(),
        MatrixRoomTags::default(),
        2,
        1,
        2,
        false,
        Some(42),
        None,
        Some(test_latest_event("$latest:example.invalid")),
        Some("$older:example.invalid".to_owned()),
        None,
        vec![],
        false,
        2,
    );

    assert_eq!(room.unread_count, 2);
    assert_eq!(room.notification_count, 2);
    assert_eq!(room.highlight_count, 1);
}

#[test]
fn room_name_placeholder_maps_only_the_sdk_empty_room_names() {
    use koushi_state::RoomNamePlaceholder;
    use matrix_sdk::RoomDisplayName;

    // #1050: the SDK renders these as English "Empty Room" text.
    assert_eq!(
        super::matrix_room_name_placeholder(&RoomDisplayName::Empty),
        Some(RoomNamePlaceholder::Empty)
    );
    assert_eq!(
        super::matrix_room_name_placeholder(&RoomDisplayName::EmptyWas("Bob".to_owned())),
        Some(RoomNamePlaceholder::EmptyWas {
            previous_names: "Bob".to_owned()
        })
    );
    // A room explicitly named "Empty Room" is caller data, not a placeholder.
    for name in [
        RoomDisplayName::Named("Empty Room".to_owned()),
        RoomDisplayName::Aliased("#empty:example.invalid".to_owned()),
        RoomDisplayName::Calculated("Alice and Bob".to_owned()),
    ] {
        assert_eq!(super::matrix_room_name_placeholder(&name), None);
    }
}
