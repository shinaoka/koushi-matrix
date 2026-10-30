//! #1692: a room whose back-paginated pages each start (oldest event) with an
//! older reply of a thread whose newer reply is already loaded.
//!
//! The renderer's `underfilled` backfill paginates while the view is still at
//! the bottom, so every page is projected under the bounded live-edge context.
//! The incremental projection must show the same ordinary rows as a one-shot
//! rebuild of the same canonical history.

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
    actor::{TimelineActor, TimelineActorControl, TimelineActorMessage},
    display_projection::apply_timeline_diffs_to_display_items,
    relay::koushi_timeline_builder,
    test_support::{fake_rid, live_tail_test_manager},
};
use crate::{executor, link_preview::LinkPreviewContext};

fn id(name: &str) -> OwnedEventId {
    OwnedEventId::try_from(format!("${name}:example.invalid")).expect("synthetic event id")
}

/// Oldest first. `None` is an ordinary message, `Some(root)` a thread reply.
/// Every page except the oldest starts with an older reply of `$x`, whose
/// newest reply is already loaded when the page arrives. Thread `$t` has its
/// root inside the page that the projection loses.
const OLDEST_PAGE: &[(&str, Option<&str>)] =
    &[("m0", None), ("x", None), ("m1", None), ("x1", Some("x"))];
const MIDDLE_PAGE: &[(&str, Option<&str>)] =
    &[("x2", Some("x")), ("m2", None), ("t", None), ("m3", None)];
const NEWER_PAGE: &[(&str, Option<&str>)] = &[
    ("x3", Some("x")),
    ("m4", None),
    ("t1", Some("t")),
    ("m5", None),
];
/// The initial (synced) window: its oldest event is an ordinary message.
const LIVE_PAGE: &[(&str, Option<&str>)] = &[
    ("m6", None),
    ("x4", Some("x")),
    ("t2", Some("t")),
    ("m7", None),
];

const ORDINARY: &[&str] = &["m0", "x", "m1", "m2", "t", "m3", "m4", "m5", "m6", "m7"];

fn shown_event_ids(items: &[TimelineItem]) -> Vec<String> {
    items
        .iter()
        .filter(|item| !item.is_hidden)
        .filter_map(|item| match &item.id {
            TimelineItemId::Event { event_id } => Some(
                event_id
                    .trim_start_matches('$')
                    .trim_end_matches(":example.invalid")
                    .to_owned(),
            ),
            _ => None,
        })
        .collect()
}

fn rows(items: &[TimelineItem]) -> Vec<String> {
    items
        .iter()
        .filter(|item| !item.is_hidden)
        .map(|item| {
            let metadata = item.display_metadata.as_ref();
            let row = metadata.map_or_else(|| format!("{:?}", item.id), |m| m.row_id.clone());
            let row = row.replace(":example.invalid", "").replace('$', "");
            let ts = metadata.and_then(|m| m.display_timestamp_ms);
            format!(
                "{row}@{}",
                ts.map_or("-".to_owned(), |ts| (ts / 1_000).to_string())
            )
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

    /// What the renderer reports while the view sits at the bottom.
    fn at_bottom(&self) -> TimelineViewportObservation {
        let ids = self
            .items
            .iter()
            .filter(|item| !item.is_hidden)
            .filter_map(|item| match &item.id {
                TimelineItemId::Event { event_id } => Some(event_id.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        TimelineViewportObservation {
            display_window_revision: self.window_revision,
            first_visible_event_id: ids.first().cloned(),
            last_visible_event_id: ids.last().cloned(),
            at_bottom: true,
            ..Default::default()
        }
    }
}

async fn drain_until(
    events: &mut broadcast::Receiver<CoreEvent>,
    observed: &mut Observed,
    done: impl Fn(&CoreEvent) -> bool,
) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = events.recv().await.expect("actor event stream");
            observed.absorb(&event);
            if done(&event) {
                break;
            }
        }
    })
    .await
    .expect("actor event within the deadline");
}

/// Drain whatever is already queued without waiting for more.
fn drain_ready(events: &mut broadcast::Receiver<CoreEvent>, observed: &mut Observed) {
    while let Ok(event) = events.try_recv() {
        observed.absorb(&event);
    }
}

#[tokio::test]
async fn back_paginated_pages_behind_a_superseded_reply_stay_visible_at_the_live_edge() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    client.event_cache().subscribe().unwrap();
    let room_id = room_id!("!issue-1692:example.invalid");
    let alice = user_id!("@alice:example.invalid");
    let factory = EventFactory::new().room(room_id);
    let mut ts = 1_000_u64;
    let mut event = |name: &str, root: Option<&str>| {
        ts += 1_000;
        let builder = factory
            .text_msg(format!("Synthetic {name}"))
            .sender(alice)
            .event_id(&id(name))
            .server_ts(ts);
        match root {
            Some(root) => {
                let root = id(root);
                builder.in_thread(&root, &root)
            }
            None => builder,
        }
    };
    let mut page = |events: &[(&str, Option<&str>)]| {
        events
            .iter()
            .map(|(name, root)| event(name, *root))
            .collect::<Vec<_>>()
    };
    let oldest = page(OLDEST_PAGE);
    let middle = page(MIDDLE_PAGE);
    let newer = page(NEWER_PAGE);
    let live = page(LIVE_PAGE);

    let room = server.sync_joined_room(&client, room_id).await;
    let mut sync = JoinedRoomBuilder::new(room_id)
        .set_timeline_limited()
        .set_timeline_prev_batch("before-live");
    for event in live {
        sync = sync.add_timeline_event(event.into_raw_sync());
    }
    server.sync_room(&client, sync).await;
    // `/messages` returns each page newest first.
    for (from, page, end) in [
        ("before-live", newer, Some("before-newer")),
        ("before-newer", middle, Some("before-middle")),
        ("before-middle", oldest, None),
    ] {
        let mut response = RoomMessagesResponseTemplate::default()
            .events(page.into_iter().rev().collect::<Vec<_>>());
        if let Some(end) = end {
            response = response.end_token(end);
        }
        server
            .mock_room_messages()
            .match_from(from)
            .ok(response)
            .mount()
            .await;
    }

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
        // "Place threaded conversations at their latest reply" off.
        koushi_state::TimelineThreadRootOrder::RootEvent,
        false,
        manager.timeline_actor_generations.clone(),
        actor_generation,
        None,
        Default::default(),
        manager.terminal_ingress.clone(),
        manager.msg_tx.clone(),
    )
    .await;

    let mut observed = Observed::default();
    drain_until(&mut events, &mut observed, |event| {
        matches!(
            event,
            CoreEvent::Timeline(TimelineEvent::InitialItems { .. })
        )
    })
    .await;
    drain_ready(&mut events, &mut observed);
    eprintln!("initial: {:?}", shown_event_ids(&observed.items));

    // The renderer's `underfilled` backfill: the view stays at the bottom and
    // reports it before each request.
    let mut end_reached = false;
    for round in 0..8_u64 {
        actor
            .send(TimelineActorMessage::ObserveViewport {
                observation: observed.at_bottom(),
            })
            .await;
        actor
            .send(TimelineActorMessage::Paginate {
                request_id: fake_rid(100 + round),
                direction: PaginationDirection::Backward,
                event_count: 20,
            })
            .await;
        let mut state = None;
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let event = events.recv().await.expect("actor event stream");
                observed.absorb(&event);
                if let CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
                    state: s @ (PaginationState::Idle | PaginationState::EndReached),
                    prepend_expected,
                    ..
                }) = &event
                {
                    state = Some((s.clone(), *prepend_expected));
                    break;
                }
            }
        })
        .await
        .expect("pagination settles");
        tokio::time::sleep(Duration::from_millis(100)).await;
        drain_ready(&mut events, &mut observed);
        eprintln!(
            "round {round}: state={state:?} shown={:?}",
            shown_event_ids(&observed.items)
        );
        if end_reached {
            break;
        }
        end_reached = matches!(state, Some((PaginationState::EndReached, _)));
    }
    let incremental = shown_event_ids(&observed.items);

    // A room switch on the same actor: `ReplayInitialItems`.
    actor
        .send_control(TimelineActorControl::ReplayInitialItems {
            cause_request_id: fake_rid(900),
        })
        .await;
    drain_until(&mut events, &mut observed, |event| {
        matches!(
            event,
            CoreEvent::Timeline(TimelineEvent::InitialItems { .. })
        )
    })
    .await;
    let replay = shown_event_ids(&observed.items);
    eprintln!("incremental={incremental:?}\nreplay={replay:?}");
    eprintln!("replay rows: {:?}", rows(&observed.items));

    // Oracle: a one-shot rebuild of the same canonical history.
    actor
        .send(TimelineActorMessage::ReturnToLive {
            request_id: fake_rid(901),
        })
        .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    drain_ready(&mut events, &mut observed);
    let rebuilt = shown_event_ids(&observed.items);
    eprintln!("one-shot rebuild={rebuilt:?}");
    eprintln!("rebuild rows: {:?}", rows(&observed.items));

    let missing = |shown: &[String]| {
        ORDINARY
            .iter()
            .filter(|name| !shown.iter().any(|shown| shown == *name))
            .copied()
            .collect::<Vec<_>>()
    };
    assert!(
        missing(&rebuilt).is_empty(),
        "the one-shot rebuild oracle must show every ordinary row: {rebuilt:?}"
    );
    assert!(
        missing(&incremental).is_empty(),
        "#1692: incremental cached paging at the live edge hides ordinary rows {:?}: \
         shown {incremental:?}, one-shot rebuild {rebuilt:?}",
        missing(&incremental)
    );
    assert!(
        missing(&replay).is_empty(),
        "#1692: a room-switch replay on the retained actor keeps ordinary rows {:?} hidden: \
         shown {replay:?}",
        missing(&replay)
    );
}
