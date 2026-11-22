//! #1916: a room whose first load holds more than the SDK's initial item budget
//! (20). The SDK then skips the oldest loaded items in its `subscribe()` view,
//! and the first backward pagination only lowers that skip count without a
//! network request. Koushi projects from the unskipped stream, so that reveal
//! adds nothing to its rows. The terminal must not promise a prepend that
//! never arrives: the renderer waits for a promised prepend before it may
//! request again, and a wrong promise stops back-pagination for good.

use std::{sync::Arc, time::Duration};

use koushi_protocol::{
    event::{
        CoreEvent, PaginationDirection, PaginationState, TimelineEvent, TimelineItem,
        TimelineItemId, TimelineViewportObservation,
    },
    ids::{AccountKey, TimelineKey},
};
use koushi_sdk::MatrixClientSession;
use koushi_state::SessionInfo;
use matrix_sdk::{
    ruma::{OwnedEventId, room_id, user_id},
    test_utils::mocks::{MatrixMockServer, RoomMessagesResponseTemplate},
};
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};
use matrix_sdk_ui::timeline::TimelineFocus;
use tokio::sync::{broadcast, mpsc};

use super::super::{
    actor::{TimelineActor, TimelineActorMessage},
    display_projection::apply_timeline_diffs_to_display_items,
    relay::koushi_timeline_builder,
    test_support::{fake_rid, live_tail_test_manager},
};
use crate::{executor, link_preview::LinkPreviewContext};

/// More than the SDK's `MAXIMUM_NUMBER_OF_INITIAL_ITEMS` (20), so the SDK's
/// `subscribe()` view starts with a non-zero skip count (about 11 here).
const LIVE_EVENTS: usize = 30;
const OLDER_EVENTS: usize = 10;
/// Below that skip count, so the first page only lowers the skip (no network),
/// as the renderer's 100-event pages do in a room with a large cached window.
const PAGE_EVENTS: u16 = 5;

fn id(index: usize) -> OwnedEventId {
    OwnedEventId::try_from(format!("$m{index}:example.invalid")).expect("synthetic event id")
}

fn shown_event_ids(items: &[TimelineItem]) -> Vec<String> {
    items
        .iter()
        .filter(|item| !item.is_hidden)
        .filter_map(|item| match &item.id {
            TimelineItemId::Event { event_id } => Some(event_id.clone()),
            _ => None,
        })
        .collect()
}

#[derive(Default)]
struct Observed {
    items: Vec<TimelineItem>,
    window_revision: Option<u64>,
}

impl Observed {
    fn absorb(&mut self, event: &CoreEvent) {
        match event {
            CoreEvent::Timeline(TimelineEvent::InitialItems { items, .. }) => {
                self.items = items.clone();
            }
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated { diffs, .. }) => {
                apply_timeline_diffs_to_display_items(&mut self.items, diffs);
            }
            CoreEvent::Timeline(TimelineEvent::NavigationUpdated { snapshot, .. }) => {
                if let Some(window) = &snapshot.display_window {
                    self.window_revision = Some(window.revision);
                }
            }
            _ => {}
        }
    }

    /// The reader scrolled to the top of what is shown.
    fn at_top(&self) -> TimelineViewportObservation {
        let ids = shown_event_ids(&self.items);
        TimelineViewportObservation {
            display_window_revision: self.window_revision,
            first_visible_event_id: ids.first().cloned(),
            last_visible_event_id: ids.get(ids.len().min(5).saturating_sub(1)).cloned(),
            at_bottom: false,
            ..Default::default()
        }
    }
}

async fn drain_for(
    events: &mut broadcast::Receiver<CoreEvent>,
    observed: &mut Observed,
    window: Duration,
) {
    let _ = tokio::time::timeout(window, async {
        loop {
            let event = events.recv().await.expect("actor event stream");
            observed.absorb(&event);
        }
    })
    .await;
}

#[tokio::test]
async fn a_backward_page_that_reveals_only_sdk_skipped_items_promises_no_prepend() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!issue-1916:example.invalid");
    let alice = user_id!("@alice:example.invalid");
    let factory = EventFactory::new().room(room_id);
    let event = |index: usize| {
        factory
            .text_msg(format!("Synthetic m{index}"))
            .sender(alice)
            .event_id(&id(index))
            .server_ts(1_000 + index as u64 * 1_000)
    };

    let room = server.sync_joined_room(&client, room_id).await;
    let mut sync = JoinedRoomBuilder::new(room_id)
        .set_timeline_limited()
        .set_timeline_prev_batch("before-live");
    for index in OLDER_EVENTS..OLDER_EVENTS + LIVE_EVENTS {
        sync = sync.add_timeline_event(event(index).into_raw_sync());
    }
    server.sync_room(&client, sync).await;
    // `/messages` returns the older page newest first, and it is the start.
    server
        .mock_room_messages()
        .match_from("before-live")
        .ok(RoomMessagesResponseTemplate::default()
            .events((0..OLDER_EVENTS).rev().map(event).collect::<Vec<_>>()))
        .mount()
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
        .expect("room timeline"),
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
    manager.event_tx = broadcast::channel(1024).0;
    let mut events = manager.event_tx.subscribe();
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
        koushi_state::TimelineThreadRootOrder::RootEvent,
        false,
        true,
        manager.timeline_actor_generations.clone(),
        actor_generation,
        None,
        Default::default(),
        manager.terminal_ingress.clone(),
        manager.msg_tx.clone(),
        None,
    )
    .await;

    let mut observed = Observed::default();
    drain_for(&mut events, &mut observed, Duration::from_millis(500)).await;
    let initial = shown_event_ids(&observed.items);
    assert_eq!(
        initial.len(),
        LIVE_EVENTS,
        "every synced event is projected (Koushi reads the unskipped stream)"
    );

    // Request older history like the renderer does: one request at a time,
    // the next only once the previous terminal and any promised prepend landed.
    let oldest = id(0).to_string();
    let mut reached_oldest = false;
    for round in 0..8_u64 {
        let before = shown_event_ids(&observed.items);
        actor
            .send(TimelineActorMessage::ObserveViewport {
                observation: observed.at_top(),
            })
            .await;
        let request_id = fake_rid(100 + round);
        actor
            .send(TimelineActorMessage::Paginate {
                request_id,
                direction: PaginationDirection::Backward,
                event_count: PAGE_EVENTS,
            })
            .await;
        let mut terminal = None;
        let _ = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let event = events.recv().await.expect("actor event stream");
                observed.absorb(&event);
                if let CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                    request_id: Some(id),
                    state: state @ (PaginationState::Idle | PaginationState::EndReached),
                    prepend_expected,
                    ..
                }) = &event
                    && *id == request_id
                {
                    terminal = Some((*state, *prepend_expected));
                    break;
                }
            }
        })
        .await;
        let (state, prepend_expected) = terminal.expect("each request reaches a terminal");
        // A promised prepend may follow its terminal; give it time to land.
        drain_for(&mut events, &mut observed, Duration::from_millis(500)).await;
        let after = shown_event_ids(&observed.items);
        let prepended = after.len() > before.len() && after.first() != before.first();
        assert!(
            prepend_expected != Some(true) || prepended,
            "#1916: round {round} promised a prepend (state {state:?}) but the rows \
             did not grow at the top: before {} rows, after {} rows",
            before.len(),
            after.len()
        );
        if after.contains(&oldest) {
            reached_oldest = true;
            break;
        }
    }
    assert!(
        reached_oldest,
        "#1916: repeated backward requests reach the older page; shown {:?}",
        shown_event_ids(&observed.items)
    );
}
