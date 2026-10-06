//! Actor-level reply quote behaviour (#1120): restore-window ordering and
//! refreshing a dependent whose quote already resolved.
//!
//! Both cases need a real `TimelineActor` over a real SDK timeline, so they
//! share the mock-server fixture below.

use std::{sync::Arc, time::Duration};

use koushi_protocol::{
    event::{CoreEvent, TimelineEvent, TimelineItem, TimelineItemId},
    ids::{AccountKey, TimelineBatchId, TimelineKey},
};
use koushi_sdk::MatrixClientSession;
use koushi_state::{ReplyQuoteState, SessionInfo};
use matrix_sdk::{
    Client, Room,
    ruma::{RoomId, UserId, event_id, room_id, user_id},
    test_utils::mocks::MatrixMockServer,
};
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};
use matrix_sdk_ui::timeline::TimelineFocus;
use tokio::sync::{broadcast, mpsc, oneshot};

use super::super::{
    actor::{TimelineActor, TimelineActorHandle, TimelineActorMessage},
    display_projection::apply_timeline_diffs_to_items,
    outbound_send::{PendingSendPhase, PendingSendProjection, pending_send_item},
    reply_quote_hydration::placeholder_quote,
    test_support::{fake_rid, live_tail_test_manager},
};
use super::koushi_timeline_builder;
use crate::{executor, link_preview::LinkPreviewContext};

const ROOT: &str = "$root:example.invalid";
const REPLY: &str = "$reply:example.invalid";
const PENDING_TXN: &str = "pending-reply";
/// Liveness bound for waits that must succeed: a regression hangs until it,
/// while scheduler load alone must never reach it.
const EVENT_LIVENESS: Duration = Duration::from_secs(60);

/// A spawned room actor plus the event stream and the display items the test
/// has observed so far.
struct RoomActor {
    actor: TimelineActorHandle,
    events: broadcast::Receiver<CoreEvent>,
    items: Vec<TimelineItem>,
    key: TimelineKey,
    actor_generation: u64,
    // Kept alive for the actor's lifetime: the manager owns the receivers the
    // actor's cloned senders target.
    _manager: super::super::manager::TimelineManagerActor,
    _action_drain: executor::JoinHandle<()>,
}

impl RoomActor {
    async fn wait_for(
        &mut self,
        mut predicate: impl FnMut(&CoreEvent, &[TimelineItem]) -> bool,
    ) -> Result<(), tokio::time::error::Elapsed> {
        wait_for(&mut self.events, &mut self.items, &mut predicate).await
    }

    /// Settle a ledger entry from an authoritative observation, as a completed
    /// batch or an edit would, without an SDK lookup.
    async fn settle_original(&self, quote: koushi_state::ReplyQuote) {
        let (settle_tx, settle_rx) = oneshot::channel();
        assert!(
            self.actor
                .send(TimelineActorMessage::TestSettleReplyQuoteOriginal {
                    quote,
                    acknowledged: settle_tx,
                })
                .await
        );
        settle_rx.await.expect("settle acknowledged");
    }
}

async fn spawn_room_actor(
    room: &Room,
    client: &Client,
    room_id: &RoomId,
    alice: &UserId,
) -> RoomActor {
    let timeline = Arc::new(
        koushi_timeline_builder(
            room,
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
    let action_drain = executor::spawn(async move { while action_rx.recv().await.is_some() {} });
    manager.event_tx = broadcast::channel(128).0;
    let events = manager.event_tx.subscribe();
    let actor_generation = manager
        .timeline_actor_generations
        .activate_after_quiescence(&key)
        .await
        .generation;
    let actor = TimelineActor::spawn(
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
    RoomActor {
        actor,
        events,
        items: Vec::new(),
        key,
        actor_generation,
        _manager: manager,
        _action_drain: action_drain,
    }
}

async fn wait_for(
    events: &mut broadcast::Receiver<CoreEvent>,
    items: &mut Vec<TimelineItem>,
    mut predicate: impl FnMut(&CoreEvent, &[TimelineItem]) -> bool,
) -> Result<(), tokio::time::error::Elapsed> {
    tokio::time::timeout(EVENT_LIVENESS, async {
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

/// Drain every event the actor has already published once a barrier proves
/// its earlier turns finished. Batches whose ids precede `fence` were decided
/// before the restore began and only reach the receiver late; any batch at or
/// after the fence was published while the restore was buffering.
async fn expect_no_items_updated_since(fixture: &mut RoomActor, fence: TimelineBatchId) {
    let (barrier_tx, barrier_rx) = oneshot::channel();
    assert!(
        fixture
            .actor
            .send(TimelineActorMessage::Barrier(barrier_tx))
            .await
    );
    barrier_rx.await.expect("actor barrier acknowledged");
    loop {
        match fixture.events.try_recv() {
            Ok(CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                batch_id, diffs, ..
            })) => {
                assert!(
                    batch_id < fence,
                    "no item batch may overtake a buffered anchor restore: \
                     batch {batch_id:?} at or after restore fence {fence:?}: {diffs:?}"
                );
                apply_timeline_diffs_to_items(&mut fixture.items, &diffs);
            }
            Ok(_) => {}
            Err(broadcast::error::TryRecvError::Empty) => return,
            Err(error) => panic!("live actor event stream: {error:?}"),
        }
    }
}

fn event_quote_state(items: &[TimelineItem], event_id: &str) -> Option<ReplyQuoteState> {
    items.iter().find_map(|item| match &item.id {
        TimelineItemId::Event { event_id: id } if id == event_id => {
            item.reply_quote.as_ref().map(|quote| quote.state)
        }
        _ => None,
    })
}

fn event_quote_body(items: &[TimelineItem], event_id: &str) -> Option<String> {
    items.iter().find_map(|item| match &item.id {
        TimelineItemId::Event { event_id: id } if id == event_id => item
            .reply_quote
            .as_ref()
            .and_then(|quote| quote.body_preview.clone()),
        _ => None,
    })
}

fn pending_quote_state(items: &[TimelineItem]) -> Option<ReplyQuoteState> {
    items.iter().find_map(|item| match &item.id {
        TimelineItemId::Transaction { transaction_id } if transaction_id == PENDING_TXN => {
            item.reply_quote.as_ref().map(|quote| quote.state)
        }
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
    let mut fixture = spawn_room_actor(&room, &client, room_id, alice).await;

    fixture
        .wait_for(|event, _| {
            matches!(
                event,
                CoreEvent::Timeline(TimelineEvent::InitialItems { .. })
            )
        })
        .await
        .expect("initial items");
    // Let the actor's own lookup settle the quote before touching the restore,
    // so the only later change is the one this test injects.
    fixture
        .wait_for(|_, items| {
            event_quote_state(items, REPLY).is_some_and(|state| state != ReplyQuoteState::Loading)
        })
        .await
        .expect("the reply quote reaches a steady state");

    let (restore_tx, restore_rx) = oneshot::channel();
    assert!(
        fixture
            .actor
            .send(TimelineActorMessage::TestBeginRestore {
                request_id: fake_rid(41),
                event_id: "$anchor-absent:example.invalid".to_owned(),
                acknowledged: restore_tx,
            })
            .await
    );
    // Batches queued before this fence may still be in the receiver; only a
    // batch decided after the restore began can overtake it.
    let fence = restore_rx.await.expect("restore fixture acknowledged");

    // A live batch that arrives during the restore must be buffered, not
    // emitted: this is what the deferred republish must not overtake.
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(room_id).add_timeline_event(
                factory
                    .text_msg("Synthetic during restore")
                    .sender(alice)
                    .event_id(event_id!("$during:example.invalid"))
                    .into_raw_sync(),
            ),
        )
        .await;
    let buffered = tokio::time::timeout(EVENT_LIVENESS, async {
        loop {
            let (state_tx, state_rx) = oneshot::channel();
            assert!(
                fixture
                    .actor
                    .send(TimelineActorMessage::TestRestoreCausalState(state_tx))
                    .await
            );
            let (_live_tail_pending, _completion_waiting, buffered_diffs, _projections) =
                state_rx.await.expect("restore state");
            if buffered_diffs > 0 {
                return buffered_diffs;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("a live batch must reach the restore buffer");
    assert!(buffered > 0);

    // Settling the original now would republish the reply's resolved quote.
    let mut quote = placeholder_quote(ROOT, ReplyQuoteState::Ready);
    quote.sender = Some(alice.to_string());
    quote.body_preview = Some("Synthetic edited original".to_owned());
    fixture.settle_original(quote).await;

    expect_no_items_updated_since(&mut fixture, fence).await;

    // Ending the restore on an in-window anchor clears the anchor and publishes
    // the settlement (buffered items first, then the terminal); the deferred
    // republish runs at the loop tail of the same actor turn, after both.
    let mut restore_finished = false;
    let mut buffered_before_terminal = false;
    assert!(
        fixture
            .actor
            .send(TimelineActorMessage::RestoreTimelineAnchor {
                request_id: fake_rid(42),
                event_id: REPLY.to_owned(),
                max_batches: 1,
                event_count: 1,
            })
            .await
    );
    fixture
        .wait_for(|event, items| {
            if matches!(
                event,
                CoreEvent::Timeline(TimelineEvent::AnchorRestoreFinished { .. })
            ) {
                restore_finished = true;
                buffered_before_terminal = items.iter().any(|item| {
                    matches!(&item.id, TimelineItemId::Event { event_id } if event_id == "$during:example.invalid")
                        && !item.is_hidden
                });
            }
            event_quote_body(items, REPLY).as_deref() == Some("Synthetic edited original")
        })
        .await
        .expect("the deferred republish must run after the restore flush");
    assert!(
        restore_finished,
        "the deferred republish must not overtake the restore's terminal event"
    );
    assert!(
        buffered_before_terminal,
        "the buffered restore publication must precede the terminal event"
    );
    assert_eq!(
        event_quote_state(&fixture.items, REPLY),
        Some(ReplyQuoteState::Ready),
        "the refreshed quote keeps the settled state"
    );
}

#[tokio::test]
async fn redacting_a_canonical_original_refreshes_a_pending_only_reply() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    client.event_cache().subscribe().unwrap();
    let room_id = room_id!("!pending-refresh:example.invalid");
    let alice = user_id!("@alice:example.invalid");
    let factory = EventFactory::new().room(room_id);
    let room = server.sync_joined_room(&client, room_id).await;
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(room_id).add_timeline_event(
                factory
                    .text_msg("Synthetic original")
                    .sender(alice)
                    .event_id(event_id!("$root:example.invalid"))
                    .into_raw_sync(),
            ),
        )
        .await;
    let mut fixture = spawn_room_actor(&room, &client, room_id, alice).await;
    fixture
        .wait_for(|event, _| {
            matches!(
                event,
                CoreEvent::Timeline(TimelineEvent::InitialItems { .. })
            )
        })
        .await
        .expect("initial items");

    // A pending reply to an original that is already canonical resolves without
    // any lookup, so the ledger only learns that original from this overlay.
    let item = pending_send_item(
        PENDING_TXN,
        "Synthetic pending reply",
        Some(ROOT.to_owned()),
        None,
        Some(alice.as_str()),
    );
    let projection = PendingSendProjection {
        key: fixture.key.clone(),
        sequence: 1,
        client_txn_id: PENDING_TXN.to_owned(),
        item,
        sdk_transaction_id: None,
        handle: None,
        terminal_event_id: None,
        phase: PendingSendPhase::Pending,
    };
    let (acknowledged, accepted) = oneshot::channel();
    assert!(
        fixture
            .actor
            .send(TimelineActorMessage::RefreshPendingSendProjection {
                actor_generation: fixture.actor_generation,
                projections: vec![projection],
                acknowledged,
            })
            .await
    );
    assert!(accepted.await.expect("pending projection acknowledged"));
    fixture
        .wait_for(|_, items| pending_quote_state(items) == Some(ReplyQuoteState::Ready))
        .await
        .expect("the pending reply resolves from the canonical original");

    // Redacting that original must reach the already-resolved pending quote.
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(room_id).add_timeline_event(
                factory
                    .redaction(event_id!("$root:example.invalid"))
                    .sender(alice),
            ),
        )
        .await;
    fixture
        .wait_for(|_, items| pending_quote_state(items) == Some(ReplyQuoteState::Redacted))
        .await
        .expect("the pending quote must follow the redaction of its original");
}
