//! Restore-window ordering for reply quote republishes (#1120).
//!
//! A republish requested while an anchor restore is buffering its coalesced
//! emission must not emit items ahead of `restore_emit_buffer`. It is deferred
//! and runs at the end of the actor loop, after the restore has flushed.

use std::{sync::Arc, time::Duration};

use koushi_protocol::{
    event::{CoreEvent, TimelineEvent, TimelineItem, TimelineItemId},
    ids::{AccountKey, TimelineKey},
};
use koushi_sdk::MatrixClientSession;
use koushi_state::{ReplyQuoteState, SessionInfo};
use matrix_sdk::{
    ruma::{event_id, room_id, user_id},
    test_utils::mocks::MatrixMockServer,
};
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};
use matrix_sdk_ui::timeline::TimelineFocus;
use tokio::sync::{broadcast, mpsc, oneshot};

use super::super::{
    actor::{TimelineActor, TimelineActorMessage},
    display_projection::apply_timeline_diffs_to_items,
    reply_quote_hydration::placeholder_quote,
    test_support::{fake_rid, live_tail_test_manager},
};
use super::koushi_timeline_builder;
use crate::{executor, link_preview::LinkPreviewContext};

const ROOT: &str = "$root:example.invalid";
const REPLY: &str = "$reply:example.invalid";

async fn wait_for(
    events: &mut broadcast::Receiver<CoreEvent>,
    items: &mut Vec<TimelineItem>,
    predicate: impl Fn(&CoreEvent, &[TimelineItem]) -> bool,
) -> Result<(), tokio::time::error::Elapsed> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.expect("live actor event stream");
            match &event {
                CoreEvent::Timeline(TimelineEvent::InitialItems { items: initial, .. }) => {
                    *items = initial.clone()
                }
                CoreEvent::Timeline(TimelineEvent::ItemsUpdated { diffs, .. }) => {
                    apply_timeline_diffs_to_items(items, diffs)
                }
                _ => {}
            }
            if predicate(&event, items) {
                break;
            }
        }
    })
    .await
}

async fn expect_no_items_updated(events: &mut broadcast::Receiver<CoreEvent>, window: Duration) {
    let deadline = tokio::time::Instant::now() + window;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return;
        }
        match tokio::time::timeout(remaining, events.recv()).await {
            Err(_) => return,
            Ok(Ok(CoreEvent::Timeline(TimelineEvent::ItemsUpdated { .. }))) => {
                panic!("no item batch may overtake a buffered anchor restore")
            }
            Ok(Ok(_)) => {}
            Ok(Err(_)) => return,
        }
    }
}

fn reply_quote_state(items: &[TimelineItem]) -> Option<ReplyQuoteState> {
    items.iter().find_map(|item| match &item.id {
        TimelineItemId::Event { event_id } if event_id == REPLY => {
            item.reply_quote.as_ref().map(|quote| quote.state)
        }
        _ => None,
    })
}

fn reply_quote_body(items: &[TimelineItem]) -> Option<String> {
    items.iter().find_map(|item| match &item.id {
        TimelineItemId::Event { event_id } if event_id == REPLY => item
            .reply_quote
            .as_ref()
            .and_then(|quote| quote.body_preview.clone()),
        _ => None,
    })
}

#[tokio::test]
async fn reply_quote_republish_is_deferred_until_the_restore_flush() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    client.event_cache().subscribe().unwrap();
    let room_id = room_id!("!reply-restore:example.invalid");
    let alice = user_id!("@alice:example.invalid");
    let factory = EventFactory::new().room(room_id);
    let room = server.sync_joined_room(&client, room_id).await;
    // The reply's original is never in this room's timeline, so the quote starts
    // `Loading`; the actor's own lookup settles it from this route, which keeps
    // the only later change the one this test injects.
    server
        .mock_room_event()
        .match_event_id()
        .ok(factory
            .text_msg("Synthetic original")
            .sender(alice)
            .event_id(event_id!("$root:example.invalid"))
            .into())
        .mount()
        .await;
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(room_id).add_timeline_event(
                factory
                    .text_msg("Synthetic reply")
                    .sender(alice)
                    .reply_to(event_id!("$root:example.invalid"))
                    .event_id(event_id!("$reply:example.invalid"))
                    .into_raw_sync(),
            ),
        )
        .await;
    let timeline = Arc::new(
        koushi_timeline_builder(
            &room,
            TimelineFocus::Live {
                hide_threaded_events: false,
            },
        )
        .build()
        .await
        .unwrap(),
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
    let _drain = executor::spawn(async move { while action_rx.recv().await.is_some() {} });
    manager.event_tx = broadcast::channel(128).0;
    let mut events = manager.event_tx.subscribe();
    let generation = manager
        .timeline_actor_generations
        .activate_after_quiescence(&key)
        .await
        .generation;
    let actor = TimelineActor::spawn(
        key,
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
        generation,
        None,
        Default::default(),
        manager.terminal_ingress.clone(),
        manager.msg_tx.clone(),
    )
    .await;

    let mut items = Vec::new();
    wait_for(&mut events, &mut items, |event, _| {
        matches!(
            event,
            CoreEvent::Timeline(TimelineEvent::InitialItems { .. })
        )
    })
    .await
    .expect("initial items");
    // Let the actor's own lookup settle the quote before touching the restore,
    // so the only later change is the one this test injects.
    let steady = wait_for(&mut events, &mut items, |_, items| {
        reply_quote_state(items).is_some_and(|state| state != ReplyQuoteState::Loading)
    })
    .await;
    if let Err(error) = steady {
        let described = items
            .iter()
            .map(|item| {
                format!(
                    "{:?}={:?}",
                    item.id,
                    item.reply_quote.as_ref().map(|quote| quote.state)
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        panic!("the reply quote reaches a steady state: {error:?} items=[{described}]");
    }

    let (restore_tx, restore_rx) = oneshot::channel();
    assert!(
        actor
            .send(TimelineActorMessage::TestBeginRestore {
                request_id: fake_rid(41),
                event_id: "$anchor-absent:example.invalid".to_owned(),
                acknowledged: restore_tx,
            })
            .await
    );
    restore_rx.await.expect("restore fixture acknowledged");

    // Settling the original now would republish the reply's resolved quote.
    let mut quote = placeholder_quote(ROOT, ReplyQuoteState::Ready);
    quote.sender = Some(alice.to_string());
    quote.body_preview = Some("Synthetic edited original".to_owned());
    let (settle_tx, settle_rx) = oneshot::channel();
    assert!(
        actor
            .send(TimelineActorMessage::TestSettleReplyQuoteOriginal {
                quote,
                acknowledged: settle_tx,
            })
            .await
    );
    settle_rx.await.expect("settle acknowledged");

    expect_no_items_updated(&mut events, Duration::from_millis(300)).await;

    // Ending the restore on an in-window anchor clears the anchor and publishes
    // the settlement; the deferred republish runs in the same actor turn.
    assert!(
        actor
            .send(TimelineActorMessage::RestoreTimelineAnchor {
                request_id: fake_rid(42),
                event_id: REPLY.to_owned(),
                max_batches: 1,
                event_count: 1,
            })
            .await
    );
    wait_for(&mut events, &mut items, |_, items| {
        reply_quote_body(items).as_deref() == Some("Synthetic edited original")
    })
    .await
    .expect("the deferred republish must run after the restore flush");
    assert_eq!(
        reply_quote_state(&items),
        Some(ReplyQuoteState::Ready),
        "the refreshed quote keeps the settled state"
    );
}
