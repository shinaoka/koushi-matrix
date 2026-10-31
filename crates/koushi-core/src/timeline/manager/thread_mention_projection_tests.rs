//! #1328: the thread-focused projection keeps an Element mention's facts.
//!
//! Element writes a user mention as a `matrix.to` anchor (not percent-encoded,
//! no leading `@`) plus `m.mentions.user_ids`. The thread pane renders the pill
//! only when both the sanitized anchor and the mentioned user id reach its DTO.
//! This module lives outside `manager`'s inline test module, which is at the
//! inline test-size limit.

use std::collections::HashMap;

use matrix_sdk::{
    ruma::{event_id, events::Mentions, room_id, user_id},
    test_utils::mocks::MatrixMockServer,
};
use matrix_sdk_base::{
    ThreadingSupport,
    linked_chunk::{ChunkIdentifier, LinkedChunkId, Position, Update},
};
use matrix_sdk_test::event_factory::EventFactory;
use matrix_sdk_ui::timeline::TimelineFocus;

use super::super::item_projection::sdk_item_to_timeline_item_with_send_states;
use super::super::test_support::thread_key;

/// #1328: an Element-authored thread reply keeps the mention facts the
/// renderer needs. Element writes the mention as a `matrix.to` user anchor
/// (not percent-encoded, no leading `@`) plus `m.mentions.user_ids`, and the
/// thread-focused projection must carry both the sanitized HTML anchor and
/// the mentioned user id into the thread pane's DTO.
#[tokio::test]
async fn thread_focus_projection_preserves_element_mention_anchors() {
    let server = MatrixMockServer::new().await;
    let client = server
        .client_builder()
        .on_builder(|b| {
            b.with_threading_support(ThreadingSupport::Enabled {
                with_subscriptions: true,
            })
        })
        .build()
        .await;
    let room_id = room_id!("!thread:example.test");
    let room = server.sync_joined_room(&client, room_id).await;
    let root = event_id!("$root:example.test");
    let reply = event_id!("$reply:example.test");
    let mentioned = user_id!("@alice:example.test");
    let factory = EventFactory::new()
        .room(room_id)
        .sender(user_id!("@bob:example.test"));
    let items = vec![
        factory.text_msg("root").event_id(root).into_event(),
        factory
            .text_html(
                "Alice: reply text",
                "<a href=\"https://matrix.to/#/@alice:example.test\">Alice</a>: reply text",
            )
            .event_id(reply)
            .mentions(Mentions::with_user_ids(vec![mentioned.to_owned()]))
            .in_thread(root, root)
            .into_event(),
    ];
    let updates = vec![
        Update::NewItemsChunk {
            previous: None,
            new: ChunkIdentifier::new(0),
            next: None,
        },
        Update::PushItems {
            at: Position::new(ChunkIdentifier::new(0), 0),
            items,
        },
    ];
    client
        .event_cache_store()
        .lock()
        .await
        .unwrap()
        .as_clean()
        .unwrap()
        .handle_linked_chunk_updates(LinkedChunkId::Thread(room_id, root), updates)
        .await
        .unwrap();
    let timeline = super::koushi_timeline_builder(
        &room,
        TimelineFocus::Thread {
            thread_id: root.to_owned(),
        },
    )
    .build()
    .await
    .unwrap();
    let _ = timeline.subscribe().await;
    super::hydrate_initial_thread(&timeline, &client, room_id, root)
        .await
        .unwrap();
    let (sdk_items, _stream) = timeline.subscribe().await;
    let key = thread_key();
    let mut reply_projection = None;
    for item in sdk_items.iter() {
        let Some(event) = item.as_event() else {
            continue;
        };
        if event.event_id() != Some(reply) {
            continue;
        }
        reply_projection = Some(sdk_item_to_timeline_item_with_send_states(
            &key,
            item,
            Some(mentioned),
            &HashMap::new(),
            None,
            None,
            None,
        ));
    }
    let projected = reply_projection.expect("the thread reply is projected");
    assert!(
        !projected.is_hidden,
        "a reply must be visible in its own thread"
    );
    assert_eq!(
        projected.mentioned_user_ids,
        vec![mentioned.to_string()],
        "the reply's m.mentions must reach the thread pane DTO"
    );
    let formatted = projected.formatted.expect("the reply keeps its HTML body");
    assert!(
        formatted
            .html
            .contains("href=\"https://matrix.to/#/@alice:example.test\""),
        "the Element-style anchor must survive sanitization: {}",
        formatted.html
    );
}
