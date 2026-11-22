//! #2059: the account avatar reaches an already published self-authored row.
//!
//! The account-side profile owner pushes the avatar to the manager, which pushes
//! it to every timeline. The actor re-derives its rows from the display
//! projection, so the row gains the avatar in place instead of waiting for the
//! timeline to be reopened.

use std::{sync::Arc, time::Duration};

use koushi_protocol::{
    event::{CoreEvent, TimelineEvent},
    ids::{AccountKey, TimelineKey},
};
use koushi_sdk::MatrixClientSession;
use matrix_sdk::{ruma::OwnedUserId, test_utils::mocks::MatrixMockServer};
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};
use matrix_sdk_ui::timeline::TimelineFocus;
use tokio::sync::{broadcast, mpsc};

use super::super::{
    actor::{TimelineActor, TimelineActorMessage},
    relay::koushi_timeline_builder,
    test_support::{fake_rid, live_tail_test_manager},
};
use crate::{executor, link_preview::LinkPreviewContext};

const OWN_AVATAR: &str = "mxc://example.org/own";

fn session_info(user_id: &str) -> koushi_state::SessionInfo {
    koushi_state::SessionInfo {
        homeserver: "https://example.invalid".into(),
        user_id: user_id.to_owned(),
        device_id: "SYNTHETIC".into(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    }
}

/// The avatar the published rows carry for `sender`, if any.
fn published_sender_avatar(
    items: &[koushi_protocol::event::TimelineItem],
    sender: &str,
) -> Option<String> {
    items
        .iter()
        .filter(|item| !item.is_hidden)
        .find(|item| item.sender.as_deref() == Some(sender))
        .and_then(|item| item.sender_avatar.as_ref())
        .map(|avatar| avatar.mxc_uri.clone())
}

/// Spawns a room actor whose only message is authored by the session user, and
/// returns the handle, the event stream and the avatar-free initial rows.
async fn spawn_room_with_own_message(
    own_avatar_mxc_uri: Option<&str>,
) -> (
    super::super::actor::TimelineActorHandle,
    broadcast::Receiver<CoreEvent>,
    Vec<koushi_protocol::event::TimelineItem>,
    Arc<MatrixMockServer>,
    matrix_sdk::Client,
    matrix_sdk::ruma::OwnedRoomId,
) {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = matrix_sdk::ruma::room_id!("!room:example.invalid").to_owned();
    // The row must be authored by the session user: the fallback is keyed on it.
    let own_user_id: OwnedUserId = client
        .user_id()
        .expect("the mock client is signed in")
        .to_owned();
    let room = server.sync_joined_room(&client, &room_id).await;
    let factory = EventFactory::new().room(&room_id).sender(&own_user_id);
    // The SDK has no sender profile for the session user, so the row has no
    // sender avatar of its own.
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(&room_id).add_timeline_event(
                factory
                    .text_msg("own message")
                    .event_id(matrix_sdk::ruma::event_id!("$own:example.invalid"))
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
        .expect("room timeline"),
    );
    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client.clone(),
        session_info(own_user_id.as_str()),
    ));
    let key = TimelineKey::room(AccountKey(own_user_id.to_string()), room_id.to_string());
    let mut manager = live_tail_test_manager(Default::default());
    let (action_tx, _action_rx) = mpsc::channel(64);
    manager.action_tx = action_tx;
    let (event_tx, _) = broadcast::channel(64);
    manager.event_tx = event_tx.clone();
    let mut events = event_tx.subscribe();
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
        true,
        manager.timeline_actor_generations.clone(),
        actor_generation,
        None,
        Default::default(),
        manager.terminal_ingress.clone(),
        manager.msg_tx.clone(),
        own_avatar_mxc_uri.map(str::to_owned),
    )
    .await;

    let mut items = Vec::new();
    let started = executor::timeout(Duration::from_secs(10), async {
        loop {
            let event = events.recv().await.expect("actor event stream");
            if let CoreEvent::Timeline(TimelineEvent::InitialItems { items: initial, .. }) = &event
            {
                items = initial.clone();
                break;
            }
        }
    })
    .await;
    started.expect("the actor publishes its initial rows");
    (actor, events, items, Arc::new(server), client, room_id)
}

/// Absorbs events into the local mirror until `predicate` holds.
async fn drain_until(
    events: &mut broadcast::Receiver<CoreEvent>,
    items: &mut Vec<koushi_protocol::event::TimelineItem>,
    sender: &str,
    predicate: impl Fn(Option<&str>) -> bool,
) -> bool {
    let settled = executor::timeout(Duration::from_secs(10), async {
        loop {
            let event = events.recv().await.expect("actor event stream");
            let CoreEvent::Timeline(TimelineEvent::ItemsUpdated { diffs, .. }) = &event else {
                continue;
            };
            for diff in diffs {
                super::super::display_projection::apply_timeline_diffs_to_display_items(
                    items,
                    std::slice::from_ref(diff),
                );
            }
            if predicate(published_sender_avatar(items, sender).as_deref()) {
                break;
            }
        }
    })
    .await;
    settled.is_ok()
}

/// The reported shape: the actor started without an account avatar, so the own
/// row rendered initials; the profile then hydrated.
#[tokio::test]
async fn a_late_account_avatar_updates_the_published_own_row() {
    let (actor, mut events, mut items, _server, client, _room_id) =
        spawn_room_with_own_message(None).await;
    let sender = client.user_id().expect("signed in").to_string();
    assert_eq!(
        published_sender_avatar(&items, &sender),
        None,
        "precondition: the SDK reported no sender avatar for the own row"
    );

    assert!(
        actor
            .send(TimelineActorMessage::OwnAvatarUpdated(Some(
                OWN_AVATAR.to_owned(),
            )))
            .await,
        "the actor accepts the account avatar"
    );

    assert!(
        drain_until(&mut events, &mut items, &sender, |avatar| avatar
            == Some(OWN_AVATAR))
        .await,
        "the already published own row must gain the account avatar without a reopen"
    );
}

/// The avatar was known when the room opened, so the first projection must
/// already carry it — no initials-then-swap on every room switch.
#[tokio::test]
async fn a_known_account_avatar_is_in_the_first_projection() {
    let (_actor, _events, items, _server, client, _room_id) =
        spawn_room_with_own_message(Some(OWN_AVATAR)).await;
    let sender = client.user_id().expect("signed in").to_string();
    assert_eq!(
        published_sender_avatar(&items, &sender).as_deref(),
        Some(OWN_AVATAR),
        "the initial rows must carry the account avatar the actor was seeded with"
    );
}
