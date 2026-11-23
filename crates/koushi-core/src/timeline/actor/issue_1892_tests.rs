//! #1892: an observation that says the same thing as the last one must not
//! publish anything.
//!
//! The renderer now coalesces its asks to one per frame (#1892), but the actor
//! is the last line: an unchanged observation cannot move the display window,
//! the read boundary or the gaps, so re-publishing `NavigationUpdated` or an
//! empty `ItemsUpdated` for it becomes a whole-shell snapshot push on every
//! frame the reader keeps scrolling.

use std::{sync::Arc, time::Duration};

use koushi_protocol::{
    event::{CoreEvent, TimelineEvent, TimelineItem, TimelineItemId, TimelineViewportObservation},
    ids::{AccountKey, TimelineKey},
};
use koushi_sdk::MatrixClientSession;
use koushi_state::SessionInfo;
use matrix_sdk::{
    ruma::{OwnedEventId, room_id, user_id},
    test_utils::mocks::MatrixMockServer,
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

fn id(name: &str) -> OwnedEventId {
    OwnedEventId::try_from(format!("${name}:example.invalid")).expect("synthetic event id")
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

/// Wait, bounded, for the next navigation projection.
async fn wait_for_navigation(
    events: &mut broadcast::Receiver<CoreEvent>,
    observed: &mut Observed,
) -> bool {
    let outcome = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = events.recv().await.expect("actor event stream");
            observed.absorb(&event);
            if matches!(
                event,
                CoreEvent::Timeline(TimelineEvent::NavigationUpdated { .. })
            ) {
                return;
            }
        }
    })
    .await;
    outcome.is_ok()
}

/// Collect events until the actor has been silent for `quiet`.
async fn drain_quiet(
    events: &mut broadcast::Receiver<CoreEvent>,
    observed: &mut Observed,
    quiet: Duration,
) -> Vec<String> {
    let mut seen = Vec::new();
    while let Ok(Ok(event)) = tokio::time::timeout(quiet, events.recv()).await {
        observed.absorb(&event);
        seen.push(format!("{event:?}").chars().take(60).collect::<String>());
    }
    seen
}

#[tokio::test]
async fn a_repeated_viewport_observation_publishes_nothing() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!issue-1892:example.invalid");
    let alice = user_id!("@alice:example.invalid");
    let factory = EventFactory::new().room(room_id);
    let mut ts = 1_000_u64;
    let mut event = |name: &str| {
        ts += 1_000;
        factory
            .text_msg(format!("Synthetic {name}"))
            .sender(alice)
            .event_id(&id(name))
            .server_ts(ts)
    };

    let room = server.sync_joined_room(&client, room_id).await;
    let mut sync = JoinedRoomBuilder::new(room_id).set_timeline_limited();
    for name in ["first", "second", "third"] {
        sync = sync.add_timeline_event(event(name).into_raw_sync());
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
    // The initial projection publishes the window the renderer will report back.
    while let Ok(Ok(event)) = tokio::time::timeout(Duration::from_secs(10), events.recv()).await {
        observed.absorb(&event);
        if matches!(
            event,
            CoreEvent::Timeline(TimelineEvent::NavigationUpdated { .. })
        ) {
            break;
        }
    }
    // Let the initial projection finish before counting, so the round below is
    // the observation's own publication and not a leftover of the first render.
    let _ = drain_quiet(&mut events, &mut observed, Duration::from_millis(250)).await;
    let observation = observed.at_bottom();
    assert!(
        observation.display_window_revision.is_some(),
        "the initial projection must publish a display window"
    );

    actor
        .send(TimelineActorMessage::ObserveViewport {
            observation: observation.clone(),
        })
        .await;
    // The control: the same actor publishes for the first observation of a
    // freshly projected window (it moves `at_bottom` from unknown to true), so
    // the silence below is the actor declining to publish rather than the
    // harness missing events.
    assert!(
        wait_for_navigation(&mut events, &mut observed).await,
        "the first observation published no navigation; the control is hollow"
    );

    actor
        .send(TimelineActorMessage::ObserveViewport {
            observation: observation.clone(),
        })
        .await;
    let repeated = drain_quiet(&mut events, &mut observed, Duration::from_millis(250)).await;

    assert!(
        repeated.is_empty(),
        "a repeated observation published {} event(s): {repeated:?}",
        repeated.len()
    );
}
