//! #2092: opening a room at the live edge leaves the unread badge and the
//! "Jump to bottom" bar in place, and no read admission ever reaches the log.
//!
//! While the opening backward pagination is in flight the display window keeps
//! moving, so the observation the renderer settles with can name a window
//! revision the actor has already superseded. Dropping that observation leaves
//! nothing to re-attempt admission: the renderer has no reason to ask again, and
//! the room stays unread while it is displayed at the bottom with the newest row
//! visible.

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
    ruma::{OwnedEventId, user_id},
    test_utils::mocks::{MatrixMockServer, RoomMessagesResponseTemplate},
};
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};
use matrix_sdk_ui::timeline::TimelineFocus;
use tokio::sync::{broadcast, mpsc};

use super::super::{
    actor::{TimelineActor, TimelineActorHandle, TimelineActorMessage},
    display_projection::apply_timeline_diffs_to_display_items,
    manager::TimelineMessage,
    relay::koushi_timeline_builder,
    test_support::{fake_rid, live_tail_test_manager},
};
use crate::{executor, link_preview::LinkPreviewContext};

const LIVE_EVENTS: usize = 4;
const OLDER_EVENTS: usize = 3;

fn id(index: usize) -> OwnedEventId {
    OwnedEventId::try_from(format!("$m{index}:example.invalid")).expect("synthetic event id")
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
                self.items = items.clone()
            }
            CoreEvent::Timeline(TimelineEvent::ItemsUpdated { diffs, .. }) => {
                apply_timeline_diffs_to_display_items(&mut self.items, diffs);
            }
            CoreEvent::Timeline(TimelineEvent::NavigationUpdated { snapshot, .. }) => {
                self.window_revision = snapshot
                    .display_window
                    .as_ref()
                    .map(|window| window.revision);
            }
            _ => {}
        }
    }

    fn shown_event_ids(&self) -> Vec<String> {
        self.items
            .iter()
            .filter(|item| !item.is_hidden)
            .filter_map(|item| match &item.id {
                TimelineItemId::Event { event_id } => Some(event_id.clone()),
                _ => None,
            })
            .collect()
    }

    /// What the renderer reports while the view sits at the bottom, from the
    /// rows and the window revision it holds.
    fn at_bottom(&self) -> TimelineViewportObservation {
        let ids = self.shown_event_ids();
        TimelineViewportObservation {
            display_window_revision: self.window_revision,
            first_visible_event_id: ids.first().cloned(),
            last_visible_event_id: ids.last().cloned(),
            at_bottom: true,
            ..Default::default()
        }
    }
}

struct LiveEdge {
    actor: TimelineActorHandle,
    events: broadcast::Receiver<CoreEvent>,
    manager: super::super::manager::TimelineManagerActor,
    observed: Observed,
}

impl LiveEdge {
    /// A room timeline shown at the live edge with `LIVE_EVENTS` rows, holding an
    /// unanswered opening backward page.
    async fn open(room_name: &str) -> Self {
        let server = MatrixMockServer::new().await;
        let client = server.client_builder().build().await;
        let room_id = Box::leak(Box::new(
            matrix_sdk::ruma::RoomId::parse(room_name).expect("synthetic room id"),
        ));
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
        let _action_drain =
            executor::spawn(async move { while action_rx.recv().await.is_some() {} });
        manager.event_tx = broadcast::channel(1024).0;
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

        let mut edge = Self {
            actor,
            events,
            manager,
            observed: Observed::default(),
        };
        edge.wait_for_navigation().await;
        edge.drain_quiet(Duration::from_millis(250)).await;
        edge
    }

    async fn wait_for_navigation(&mut self) {
        let _ = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let event = self.events.recv().await.expect("actor event stream");
                self.observed.absorb(&event);
                if matches!(
                    event,
                    CoreEvent::Timeline(TimelineEvent::NavigationUpdated { .. })
                ) {
                    break;
                }
            }
        })
        .await;
    }

    async fn drain_quiet(&mut self, quiet: Duration) -> Vec<CoreEvent> {
        let mut seen = Vec::new();
        while let Ok(Ok(event)) = tokio::time::timeout(quiet, self.events.recv()).await {
            self.observed.absorb(&event);
            seen.push(event);
        }
        seen
    }

    /// The opening backward page lands, ahead of the renderer's observation.
    async fn prepend_opening_page(&mut self) {
        self.actor
            .send(TimelineActorMessage::Paginate {
                request_id: fake_rid(7),
                direction: PaginationDirection::Backward,
                event_count: 40,
            })
            .await;
        let _ = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let event = self.events.recv().await.expect("actor event stream");
                self.observed.absorb(&event);
                if let CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                    request_id: Some(request_id),
                    state: PaginationState::Idle | PaginationState::EndReached,
                    ..
                }) = &event
                    && *request_id == fake_rid(7)
                {
                    break;
                }
            }
        })
        .await;
        self.drain_quiet(Duration::from_millis(500)).await;
    }

    /// The read boundary the observation produced, if any.
    async fn observed_read_boundary(&mut self) -> Option<String> {
        self.drain_quiet(Duration::from_millis(500)).await;
        let mut target = None;
        while let Ok(message) = self.manager.msg_rx.try_recv() {
            if let TimelineMessage::LocalReadBoundaryObserved {
                target: boundary, ..
            } = message
            {
                target = Some(boundary.event_id().to_owned());
            }
        }
        target
    }
}

#[tokio::test]
async fn an_observation_superseded_by_a_prepend_still_reaches_the_read_boundary() {
    let mut edge = LiveEdge::open("!issue-2092-a:example.invalid").await;
    // The observation the renderer settles with if the page were not in flight.
    let settled = edge.observed.at_bottom();
    let revision_before = edge.observed.window_revision;
    assert!(
        revision_before.is_some(),
        "the initial projection must publish a display window"
    );
    let rows_before = edge.observed.shown_event_ids();

    edge.prepend_opening_page().await;
    assert_ne!(
        edge.observed.window_revision, revision_before,
        "the prepend must advance the display window revision"
    );
    assert!(
        edge.observed.shown_event_ids().len() > rows_before.len(),
        "the opening page must prepend older rows: before {rows_before:?}, after {:?}",
        edge.observed.shown_event_ids()
    );

    // The settled observation arrives naming the superseded revision. The reader
    // is still at the bottom and the newest row is the one it names, so the read
    // must still be admitted; nothing else will ask again.
    edge.actor
        .send(TimelineActorMessage::ObserveViewport {
            observation: settled,
        })
        .await;
    let read = edge.observed_read_boundary().await;
    assert!(
        read.is_some(),
        "#2092: the superseded observation never reached the read boundary; \
         the room stays unread at the bottom"
    );
    assert_eq!(
        read,
        edge.observed.shown_event_ids().last().cloned(),
        "#2092: the read must cover the newest row the reader is looking at"
    );
}
