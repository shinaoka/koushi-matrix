//! #1125 RED stage: a fresh room subscription must reach a visible message
//! that sits behind more hidden state events than the initial hydration window.
//!
//! The shipped `hidden_state_acl` scenario re-subscribes the live actor, so the
//! Core-held replay keeps the message and the scenario stays green. This test
//! builds the fresh-subscription case instead and is ignored because it
//! currently fails: the initial hydration is one bounded
//! `INITIAL_EMPTY_ROOM_BACKFILL_EVENT_COUNT` pass, so a window that is entirely
//! hidden state events leaves the reader with no displayed row.

use std::{sync::Arc, time::Duration};

use koushi_protocol::{
    event::{CoreEvent, TimelineEvent, TimelineItemId},
    ids::{AccountKey, TimelineKey},
};
use koushi_sdk::MatrixClientSession;
use koushi_state::SessionInfo;
use matrix_sdk::{
    ruma::{OwnedEventId, event_id, room_id, user_id},
    test_utils::mocks::MatrixMockServer,
};
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};
use matrix_sdk_ui::timeline::TimelineFocus;
use tokio::sync::{broadcast, mpsc};

use super::super::{
    actor::TimelineActor,
    navigation::ROOM_REPLAY_INITIAL_ITEMS_MAX,
    relay::koushi_timeline_builder,
    test_support::{fake_rid, live_tail_test_manager},
};
use crate::{executor, link_preview::LinkPreviewContext};

const VISIBLE_EVENT_ID: &str = "$visible:example.invalid";

/// More hidden updates than the live-edge window (120 rows), so a fresh
/// subscription's window holds no displayed row at all.
const HIDDEN_UPDATE_COUNT: usize = ROOM_REPLAY_INITIAL_ITEMS_MAX + 5;

#[tokio::test]
#[ignore = "#1125: a fresh room subscription hydrates one bounded window, so a visible message behind more hidden state events than that window is not reached"]
async fn fresh_room_subscription_reaches_a_message_behind_hidden_state_events() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    client.event_cache().subscribe().unwrap();
    let room_id = room_id!("!fresh-hydration:example.invalid");
    let alice = user_id!("@alice:example.invalid");
    let factory = EventFactory::new().room(room_id);
    let room = server.sync_joined_room(&client, room_id).await;

    let mut sync = JoinedRoomBuilder::new(room_id).add_timeline_event(
        factory
            .text_msg("Synthetic visible message")
            .sender(alice)
            .event_id(event_id!("$visible:example.invalid"))
            .into_raw_sync(),
    );
    for index in 0..HIDDEN_UPDATE_COUNT {
        let event_id = OwnedEventId::try_from(format!("$acl-{index}:example.invalid"))
            .expect("synthetic ACL event id");
        sync = sync.add_timeline_event(
            factory
                .server_acl(
                    false,
                    vec!["*".to_owned()],
                    vec![format!("blocked-{index}.example.invalid")],
                )
                .sender(alice)
                .state_key("")
                .event_id(&event_id)
                .into_raw_sync(),
        );
    }
    server.sync_room(&client, sync).await;

    let timeline = Arc::new(
        koushi_timeline_builder(
            &room,
            TimelineFocus::Live {
                hide_threaded_events: false,
            },
        )
        .build()
        .await
        .expect("fresh room timeline"),
    );
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client.clone(),
        SessionInfo {
            homeserver: "https://example.invalid".into(),
            user_id: alice.to_string(),
            device_id: "SYNTHETIC".into(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    ));
    let key = TimelineKey::room(AccountKey(alice.to_string()), room_id.to_string());
    let mut manager = live_tail_test_manager(Default::default());
    let (action_tx, mut action_rx) = mpsc::channel(64);
    manager.action_tx = action_tx;
    let _action_drain = executor::spawn(async move { while action_rx.recv().await.is_some() {} });
    manager.event_tx = broadcast::channel(256).0;
    let mut events = manager.event_tx.subscribe();
    let actor_generation = manager
        .timeline_actor_generations
        .activate_after_quiescence(&key)
        .await
        .generation;
    let _actor = TimelineActor::spawn(
        key.clone(),
        timeline,
        session,
        fake_rid(1),
        true,
        manager.action_tx.clone(),
        manager.event_tx.clone(),
        None,
        Default::default(),
        None,
        LinkPreviewContext::default(),
        manager.account_work.clone(),
        manager.thread_root_projection_service.clone(),
        manager.thread_root_order,
        false,
        manager.timeline_actor_generations.clone(),
        actor_generation,
        None,
        Default::default(),
        manager.terminal_ingress.clone(),
        manager.msg_tx.clone(),
    )
    .await;

    let initial = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let CoreEvent::Timeline(TimelineEvent::InitialItems {
                key: event_key,
                items,
                ..
            }) = events
                .recv()
                .await
                .expect("fresh subscription event stream")
                && event_key == key
            {
                return items;
            }
        }
    })
    .await
    .expect("fresh subscription must publish its initial items");

    let hidden = initial.iter().filter(|item| item.is_hidden).count();
    let visible = initial.iter().any(|item| {
        matches!(&item.id, TimelineItemId::Event { event_id } if event_id == VISIBLE_EVENT_ID)
            && !item.is_hidden
    });
    assert!(
        visible,
        "#1125: the fresh subscription must reach the visible message \
         (items={}, hidden={hidden}, window={ROOM_REPLAY_INITIAL_ITEMS_MAX})",
        initial.len()
    );
}
