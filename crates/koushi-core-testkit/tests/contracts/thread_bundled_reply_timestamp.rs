//! #1626: a session that first sees a thread after its replies exist dates the
//! latest reply at the root.
//!
//! The server bundles a thread's latest reply in the root's
//! `unsigned.m.relations.m.thread.latest_event`. Deriving that bundled copy used
//! the *root's* own `origin_server_ts` as the anti-future ceiling, and the room
//! event cache persists the bundled copy, overwriting the reply's correctly
//! dated copy already in the cache. Every later read then dates the reply at the
//! root: the SDK's thread relation aggregate, and the
//! `ThreadSummaryDto.latest_timestamp_ms` that `threads_list` projects from it.

use std::time::Duration;

use matrix_sdk::deserialized_responses::TimelineEvent;
use matrix_sdk::event_cache::{RoomEventCacheUpdate, Subscriber, TimelineVectorDiffs};
use matrix_sdk::ruma::events::AnySyncTimelineEvent;
use matrix_sdk::ruma::events::relation::RelationType;
use matrix_sdk::ruma::serde::Raw;
use matrix_sdk::ruma::{EventId, event_id, room_id};
use matrix_sdk::test_utils::mocks::MatrixMockServer;
use matrix_sdk_test::JoinedRoomBuilder;
use matrix_sdk_ui::eyeball_im::VectorDiff;
use matrix_sdk_ui::timeline::thread_list_service::resolve_thread_relation_aggregate;
use serde_json::{Value, json};

const ROOT_ID: &str = "$thread-root:example.invalid";
const REPLY_ID: &str = "$thread-reply:example.invalid";
const ROOT_TS: u64 = 1_700_000_000_000;
const REPLY_TS: u64 = 1_700_000_100_000;

const CACHE_TIMEOUT: Duration = Duration::from_secs(10);

fn reply_json() -> Value {
    json!({
        "event_id": REPLY_ID,
        "sender": "@bob:example.invalid",
        "origin_server_ts": REPLY_TS,
        "type": "m.room.message",
        "content": {
            "msgtype": "m.text",
            "body": "latest reply",
            "m.relates_to": { "rel_type": "m.thread", "event_id": ROOT_ID },
        },
    })
}

/// The root as a homeserver sends it: the thread's latest reply is bundled in
/// the unsigned thread summary, so a client that syncs after the replies exist
/// learns the reply from here.
fn root_json() -> Value {
    json!({
        "event_id": ROOT_ID,
        "sender": "@alice:example.invalid",
        "origin_server_ts": ROOT_TS,
        "type": "m.room.message",
        "content": { "msgtype": "m.text", "body": "thread root" },
        "unsigned": {
            "m.relations": {
                "m.thread": {
                    "latest_event": reply_json(),
                    "count": 1,
                    "current_user_participated": false,
                },
            },
        },
    })
}

fn raw_event(event: Value) -> Raw<AnySyncTimelineEvent> {
    serde_json::from_value(event).expect("valid sync timeline event")
}

fn reply_id() -> &'static EventId {
    event_id!("$thread-reply:example.invalid")
}

fn diff_carries(diff: &VectorDiff<TimelineEvent>, event_id: &EventId) -> bool {
    match diff {
        VectorDiff::PushFront { value }
        | VectorDiff::PushBack { value }
        | VectorDiff::Insert { value, .. }
        | VectorDiff::Set { value, .. } => value.event_id() == Some(event_id),
        VectorDiff::Append { values } | VectorDiff::Reset { values } => values
            .iter()
            .any(|value| value.event_id() == Some(event_id)),
        VectorDiff::PopFront
        | VectorDiff::PopBack
        | VectorDiff::Remove { .. }
        | VectorDiff::Clear
        | VectorDiff::Truncate { .. } => false,
    }
}

/// Waits for the batch that carries `event_id`. A batch is persisted and
/// post-processed (which is where a bundled reply is saved) before its diffs are
/// published, so this is also the barrier for the persisted copy being final.
async fn wait_for_batch(updates: &mut Subscriber<RoomEventCacheUpdate>, event_id: &EventId) {
    tokio::time::timeout(CACHE_TIMEOUT, async {
        loop {
            let RoomEventCacheUpdate::UpdateTimelineEvents(TimelineVectorDiffs { diffs, .. }) =
                updates.recv().await.expect("room event cache update")
            else {
                continue;
            };
            if diffs.iter().any(|diff| diff_carries(diff, event_id)) {
                break;
            }
        }
    })
    .await
    .expect("the sync batch reaches the room event cache");
}

#[tokio::test]
async fn a_bundled_thread_summary_keeps_the_reply_timestamp() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;

    let room_id = room_id!("!thread-timestamps:example.invalid");
    server.sync_joined_room(&client, room_id).await;
    let room = client.get_room(room_id).expect("the synced room");
    let (cache, _handles) = room
        .client()
        .event_cache()
        .unwrap()
        .room(room.room_id())
        .await
        .expect("room event cache");
    let (_, mut updates) = cache
        .subscribe()
        .await
        .expect("room event cache subscription");

    // The reply reaches this session first, dated by the server.
    server
        .mock_sync()
        .ok_and_run(&client, |builder| {
            builder.add_joined_room(
                JoinedRoomBuilder::new(room_id).add_timeline_event(raw_event(reply_json())),
            );
        })
        .await;
    wait_for_batch(&mut updates, reply_id()).await;

    // The thread root arrives later with the same reply bundled as the latest
    // event of its thread summary.
    server
        .mock_sync()
        .ok_and_run(&client, |builder| {
            builder.add_joined_room(
                JoinedRoomBuilder::new(room_id).add_timeline_event(raw_event(root_json())),
            );
        })
        .await;
    wait_for_batch(&mut updates, event_id!("$thread-root:example.invalid")).await;

    // The persisted copy, read the way the thread resolver reads it: the
    // relation index goes straight to the store, where the bundled copy replaced
    // the reply. (In-memory lookups still hold the earlier batch's copy.)
    let persisted = cache
        .find_event_relations(
            event_id!("$thread-root:example.invalid"),
            Some(vec![RelationType::Thread]),
        )
        .await
        .expect("persisted relations")
        .into_iter()
        .find(|event| event.event_id() == Some(reply_id()))
        .expect("the latest reply is persisted");
    assert_eq!(
        u64::from(persisted.timestamp().expect("persisted reply timestamp").0),
        REPLY_TS,
        "the persisted reply must keep its own origin_server_ts, not the root's"
    );

    // The value the thread summary projection consumes: `threads_list` copies
    // this aggregate's `latest_event.timestamp` into
    // `ThreadSummaryDto.latest_timestamp_ms`.
    let aggregate =
        resolve_thread_relation_aggregate(&room, event_id!("$thread-root:example.invalid"))
            .await
            .expect("thread relation aggregate");
    let latest = aggregate
        .latest_event
        .expect("the thread has a latest reply");
    assert_eq!(latest.event_id, reply_id());
    assert_eq!(
        u64::from(latest.timestamp.0),
        REPLY_TS,
        "the thread summary must show the reply's own time, not the root's"
    );
}
