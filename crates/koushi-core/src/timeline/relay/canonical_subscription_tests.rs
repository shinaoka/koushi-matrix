use super::{koushi_timeline_builder, koushi_timeline_subscribe};
use futures_util::StreamExt;
use matrix_sdk::{
    ruma::{OwnedEventId, event_id, room_id, user_id},
    test_utils::mocks::MatrixMockServer,
};
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};
use matrix_sdk_ui::timeline::TimelineFocus;
use std::{sync::Arc, time::Duration};

fn hidden_tail(factory: &EventFactory, room: &matrix_sdk::ruma::RoomId) -> JoinedRoomBuilder {
    let alice = user_id!("@reader:example.invalid");
    let mut sync = JoinedRoomBuilder::new(room).add_timeline_event(
        factory
            .text_msg("Synthetic canonical message")
            .sender(alice)
            .event_id(event_id!("$canonical-message:example.invalid"))
            .into_raw_sync(),
    );
    for index in 0..125 {
        let id = OwnedEventId::try_from(format!("$canonical-acl-{index}:example.invalid")).unwrap();
        sync = sync.add_timeline_event(
            factory
                .server_acl(
                    false,
                    vec!["*".to_owned()],
                    vec![format!("blocked-{index}.example.invalid")],
                )
                .sender(alice)
                .state_key("")
                .event_id(&id)
                .into_raw_sync(),
        );
    }
    sync
}

#[tokio::test]
async fn canonical_subscription_keeps_the_message_before_a_hidden_initial_tail() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!canonical-subscription:example.invalid");
    let room = server.sync_joined_room(&client, room_id).await;
    let (cache, _cache_lifetime) = client.event_cache().unwrap().room(room_id).await.unwrap();
    let (_cached, _subscriber) = cache.subscribe().await.unwrap();
    let factory = EventFactory::new().room(room_id);
    server
        .sync_room(&client, hidden_tail(&factory, room_id))
        .await;
    let timeline = koushi_timeline_builder(
        &room,
        TimelineFocus::Live {
            hide_threaded_events: true,
        },
    )
    .build()
    .await
    .unwrap();
    let raw = timeline.items().await;
    assert!(
        raw.iter().any(|item| item
            .as_event()
            .and_then(|item| item.event_id())
            .is_some_and(|id| id.as_str() == "$canonical-message:example.invalid")),
        "fixture must retain the message in raw SDK items"
    );
    let (initial, _stream) = koushi_timeline_subscribe(Arc::new(timeline)).await;
    assert_eq!(
        initial.len(),
        raw.len(),
        "Core canonical snapshot must not apply SDK presentation skip"
    );
    assert_eq!(
        initial
            .iter()
            .map(|item| item.unique_id().clone())
            .collect::<Vec<_>>(),
        raw.iter()
            .map(|item| item.unique_id().clone())
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn canonical_subscription_live_reset_diffs_match_the_raw_sdk_window() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!canonical-reset:example.invalid");
    let room = server.sync_joined_room(&client, room_id).await;
    let factory = EventFactory::new().room(room_id);
    let timeline = koushi_timeline_builder(
        &room,
        TimelineFocus::Live {
            hide_threaded_events: true,
        },
    )
    .build()
    .await
    .unwrap();
    let timeline = Arc::new(timeline);
    let (mut items, stream) = koushi_timeline_subscribe(timeline.clone()).await;
    futures_util::pin_mut!(stream);
    server
        .sync_room(
            &client,
            hidden_tail(&factory, room_id)
                .set_timeline_limited()
                .set_timeline_prev_batch("synthetic-gap"),
        )
        .await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let diffs = stream.next().await.expect("live canonical stream");
            for diff in diffs {
                diff.apply(&mut items);
            }
            if items.iter().any(|item| {
                item.as_event()
                    .and_then(|item| item.event_id())
                    .is_some_and(|id| id.as_str() == "$canonical-acl-124:example.invalid")
            }) {
                break;
            }
        }
    })
    .await
    .unwrap();
    let raw = timeline.items().await;
    assert_eq!(
        items.len(),
        raw.len(),
        "Core reset diffs must describe the same raw domain as items()"
    );
    assert_eq!(
        items
            .iter()
            .map(|item| item.unique_id().clone())
            .collect::<Vec<_>>(),
        raw.iter()
            .map(|item| item.unique_id().clone())
            .collect::<Vec<_>>()
    );
    assert!(items.iter().any(|item| {
        item.as_event()
            .and_then(|item| item.event_id())
            .is_some_and(|id| id.as_str() == "$canonical-message:example.invalid")
    }));
}
