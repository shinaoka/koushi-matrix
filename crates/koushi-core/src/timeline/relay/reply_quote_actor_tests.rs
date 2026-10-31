//! Actor-level reply quote behaviour (#1120): restore-window ordering and
//! refreshing a dependent whose quote already resolved.
//!
//! Both cases need a real `TimelineActor` over a real SDK timeline, so they
//! share the mock-server fixture below.

use std::{sync::Arc, time::Duration};

use koushi_protocol::{
    event::{CoreEvent, TimelineEvent, TimelineItem, TimelineItemId},
    ids::{AccountKey, TimelineKey},
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
    test_support::{fake_rid, live_tail_test_manager},
};
use super::koushi_timeline_builder;
use crate::{executor, link_preview::LinkPreviewContext};

const ROOT: &str = "$root:example.invalid";
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

fn pending_quote_state(items: &[TimelineItem]) -> Option<ReplyQuoteState> {
    items.iter().find_map(|item| match &item.id {
        TimelineItemId::Transaction { transaction_id } if transaction_id == PENDING_TXN => {
            item.reply_quote.as_ref().map(|quote| quote.state)
        }
        _ => None,
    })
}

#[tokio::test]
async fn redacting_a_canonical_original_refreshes_a_pending_only_reply() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
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
        lifecycle_trace: None,
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
