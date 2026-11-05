//! #1783: a reader whose receipt sits on a thread reply must still show in the
//! main pane once the thread root is drawn at that reply.
//!
//! The room timeline keeps thread replies (`hide_threaded_events: false`), so
//! the SDK places an unthreaded `m.read` on the reply event itself. Under
//! `LatestReply` ordering the display projection hides that reply and draws the
//! root at its position. The renderer reads a row's readers from the Main
//! receipt scope under the row's `content_event_id`, which for a moved root is
//! the root event, so the reader must be published there. The root event is
//! covered both inside the bounded window and outside it (#1657).

use std::{sync::Arc, time::Duration};

use koushi_protocol::{
    event::{CoreEvent, TimelineEvent, TimelineItem},
    ids::{AccountKey, TimelineKey},
};
use koushi_sdk::MatrixClientSession;
use koushi_state::{AppAction, AppState, SessionInfo, SessionState, reduce};
use matrix_sdk::{
    ruma::{
        OwnedEventId, UserId,
        events::receipt::{ReceiptThread, ReceiptType},
        room_id, user_id,
    },
    test_utils::mocks::MatrixMockServer,
};
use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};
use matrix_sdk_ui::timeline::TimelineFocus;
use tokio::sync::{broadcast, mpsc};

use super::super::{
    actor::TimelineActor,
    display_projection::apply_timeline_diffs_to_display_items,
    relay::koushi_timeline_builder,
    test_support::{fake_rid, live_tail_test_manager, timeline_item},
};
use crate::{executor, link_preview::LinkPreviewContext};

type ThreadRoots = Arc<std::sync::Mutex<crate::threads_list::ThreadRootProjectionService>>;

fn id(name: &str) -> OwnedEventId {
    OwnedEventId::try_from(format!("${name}:example.invalid")).expect("synthetic event id")
}

fn thread_roots() -> ThreadRoots {
    Arc::new(std::sync::Mutex::new(Default::default()))
}

fn session_info(user_id: &str) -> SessionInfo {
    SessionInfo {
        homeserver: "https://example.invalid".into(),
        user_id: user_id.to_owned(),
        device_id: "SYNTHETIC".into(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    }
}

fn apply(items: &mut Vec<TimelineItem>, event: &CoreEvent) {
    match event {
        CoreEvent::Timeline(TimelineEvent::InitialItems { items: initial, .. }) => {
            *items = initial.clone();
        }
        CoreEvent::Timeline(TimelineEvent::ItemsUpdated { diffs, .. }) => {
            apply_timeline_diffs_to_display_items(items, diffs);
        }
        _ => {}
    }
}

/// The rendered row whose content is `root` and whose activity is `activity`.
fn moved_root_row<'a>(
    items: &'a [TimelineItem],
    root: &str,
    activity: &str,
) -> Option<&'a TimelineItem> {
    items.iter().filter(|item| !item.is_hidden).find(|item| {
        item.display_metadata.as_ref().is_some_and(|metadata| {
            metadata.content_event_id.as_deref() == Some(root)
                && metadata.activity_event_id.as_deref() == Some(activity)
        })
    })
}

/// Main-scope event ids that list `reader`, as the renderer looks them up.
fn main_scope_events_with_reader(state: &AppState, room_id: &str, reader: &str) -> Vec<String> {
    let Some(room) = state.live_signals.rooms.get(room_id) else {
        return Vec::new();
    };
    let mut event_ids = room
        .receipts_by_event
        .iter()
        .filter(|(_, summary)| summary.readers.iter().any(|row| row.user_id == reader))
        .map(|(event_id, _)| event_id.clone())
        .collect::<Vec<_>>();
    event_ids.sort();
    event_ids
}

/// Readers the renderer draws for `event_id`'s row, sorted for comparison.
fn row_readers(state: &AppState, room_id: &str, event_id: &str) -> Vec<String> {
    let mut readers = state
        .live_signals
        .rooms
        .get(room_id)
        .and_then(|room| room.receipts_by_event.get(event_id))
        .map(|summary| {
            summary
                .readers
                .iter()
                .map(|row| row.user_id.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    readers.sort();
    readers
}

struct ActorFixture {
    actor: super::super::actor::TimelineActorHandle,
    events: broadcast::Receiver<CoreEvent>,
    actions: mpsc::UnboundedReceiver<AppAction>,
}

/// A room actor under `LatestReply` ordering, the setting that moves a thread
/// root to its latest reply.
async fn spawn_room_actor(
    client: &matrix_sdk::Client,
    room: &matrix_sdk::Room,
    owner: &str,
    thread_roots: ThreadRoots,
) -> ActorFixture {
    let timeline = Arc::new(
        koushi_timeline_builder(
            room,
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
        session_info(owner),
    ));
    let key = TimelineKey::room(AccountKey(owner.to_owned()), room.room_id().to_string());
    let mut manager = live_tail_test_manager(Default::default());
    let (action_tx, mut action_rx) = mpsc::channel(256);
    manager.action_tx = action_tx;
    // Forward every action so the actor never blocks on a full channel.
    let (actions_tx, actions) = mpsc::unbounded_channel::<AppAction>();
    let _action_forward = executor::spawn(async move {
        while let Some(batch) = action_rx.recv().await {
            for action in batch {
                let _ = actions_tx.send(action);
            }
        }
    });
    manager.event_tx = broadcast::channel(1024).0;
    let events = manager.event_tx.subscribe();
    manager.thread_root_projection_service = thread_roots;
    let actor_generation = manager
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
        koushi_state::TimelineThreadRootOrder::LatestReply,
        false,
        manager.timeline_actor_generations.clone(),
        actor_generation,
        None,
        Default::default(),
        manager.terminal_ingress.clone(),
        manager.msg_tx.clone(),
    )
    .await;
    ActorFixture {
        actor,
        events,
        actions,
    }
}

/// Absorbs actor events into the local display mirror until the root is drawn
/// at `activity`.
async fn wait_for_moved_root_row(
    events: &mut broadcast::Receiver<CoreEvent>,
    items: &mut Vec<TimelineItem>,
    root: &str,
    activity: &str,
) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = events.recv().await.expect("actor event stream");
            apply(items, &event);
            if moved_root_row(items, root, activity).is_some() {
                break;
            }
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "#1783 precondition: the root must be drawn at its latest reply; rows {:?}",
            items
                .iter()
                .filter(|item| !item.is_hidden)
                .map(|item| item.display_metadata.as_ref().map(|m| {
                    (
                        m.row_id.clone(),
                        m.content_event_id.clone(),
                        m.activity_event_id.clone(),
                    )
                }))
                .collect::<Vec<_>>()
        )
    });
}

/// Feeds actor actions into `state` until `reader` has a summary in the Main
/// scope, so the assertion tests where it lands rather than whether it arrived.
async fn reduce_until_reader_is_published(
    actions: &mut mpsc::UnboundedReceiver<AppAction>,
    state: &mut AppState,
    room_id: &str,
    reader: &str,
) {
    let settled = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let action = actions.recv().await.expect("actor action stream");
            reduce(state, action);
            if !main_scope_events_with_reader(state, room_id, reader).is_empty() {
                break;
            }
        }
    })
    .await;
    assert!(
        settled.is_ok(),
        "#1783 precondition: the reader's receipt must reach the Main receipt scope"
    );
    // Let any follow-up observation for the same receipt settle.
    while let Ok(Some(action)) =
        tokio::time::timeout(Duration::from_millis(500), actions.recv()).await
    {
        reduce(state, action);
    }
}

fn with_unthreaded_receipt(
    room: JoinedRoomBuilder,
    factory: &EventFactory,
    event_id: &OwnedEventId,
    reader: &UserId,
) -> JoinedRoomBuilder {
    room.add_receipt(
        factory
            .read_receipts()
            .add(
                event_id,
                reader,
                ReceiptType::Read,
                ReceiptThread::Unthreaded,
            )
            .into_event(),
    )
}

/// The moved root row is the only row that publishes these readers, under the
/// content event id the renderer looks up.
fn assert_moved_root_row_publishes(
    items: &[TimelineItem],
    state: &AppState,
    room_id: &str,
    root: &str,
    reply: &str,
    readers: &[&UserId],
) {
    assert!(
        moved_root_row(items, root, reply).is_some(),
        "#1783: the root must be drawn at its latest reply"
    );
    for reader in readers {
        assert_eq!(
            main_scope_events_with_reader(state, room_id, reader.as_str()),
            [root],
            "#1783: {reader} must be published only under the moved root row's \
             content event id"
        );
    }
    let published = row_readers(state, room_id, root);
    for reader in readers {
        assert!(
            published.contains(&reader.as_str().to_owned()),
            "#1783: the moved root row publishes the readers of both its own event \
             and the hidden reply it stands for, but holds {published:?}"
        );
    }
}

#[tokio::test]
async fn a_receipt_on_the_latest_thread_reply_shows_on_the_moved_root_row() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!issue-1783:example.invalid");
    let alice = user_id!("@alice:example.invalid");
    let bob = user_id!("@bob:example.invalid");
    let carol = user_id!("@carol:example.invalid");
    let dave = user_id!("@dave:example.invalid");
    let factory = EventFactory::new().room(room_id);
    let root = id("root");
    let main = id("main");
    let reply = id("reply");

    // Oldest first: the root, a later main message, then a reply in the root's
    // thread, so `LatestReply` moves the root below the main message.
    let room = server.sync_joined_room(&client, room_id).await;
    server
        .sync_room(
            &client,
            JoinedRoomBuilder::new(room_id)
                .add_timeline_event(
                    factory
                        .text_msg("Synthetic root")
                        .sender(bob)
                        .event_id(&root)
                        .server_ts(1_000_u64)
                        .into_raw_sync(),
                )
                .add_timeline_event(
                    factory
                        .text_msg("Synthetic main")
                        .sender(alice)
                        .event_id(&main)
                        .server_ts(2_000_u64)
                        .into_raw_sync(),
                )
                .add_timeline_event(
                    factory
                        .text_msg("Synthetic reply")
                        .sender(bob)
                        .event_id(&reply)
                        .server_ts(3_000_u64)
                        .in_thread(&root, &root)
                        .into_raw_sync(),
                ),
        )
        .await;

    let ActorFixture {
        actor,
        mut events,
        mut actions,
    } = spawn_room_actor(&client, &room, alice.as_str(), thread_roots()).await;
    let mut items = Vec::new();
    wait_for_moved_root_row(&mut events, &mut items, root.as_str(), reply.as_str()).await;

    let mut state = AppState {
        session: SessionState::Ready(session_info(alice.as_str())),
        ..AppState::default()
    };
    // Another client reads up to the reply and sends an unthreaded receipt, as
    // Element does when the newest event is a thread reply.
    server
        .sync_room(
            &client,
            with_unthreaded_receipt(JoinedRoomBuilder::new(room_id), &factory, &reply, carol),
        )
        .await;
    reduce_until_reader_is_published(&mut actions, &mut state, room_id.as_str(), carol.as_str())
        .await;
    assert_moved_root_row_publishes(
        &items,
        &state,
        room_id.as_str(),
        root.as_str(),
        reply.as_str(),
        &[carol],
    );

    // A later receipt on the root itself replaces the one summary the row owns,
    // so the update must carry the readers of the reply it stands for.
    server
        .sync_room(
            &client,
            with_unthreaded_receipt(JoinedRoomBuilder::new(room_id), &factory, &root, dave),
        )
        .await;
    reduce_until_reader_is_published(&mut actions, &mut state, room_id.as_str(), dave.as_str())
        .await;
    assert_moved_root_row_publishes(
        &items,
        &state,
        room_id.as_str(),
        root.as_str(),
        reply.as_str(),
        &[carol, dave],
    );
    drop(actor);
}

#[tokio::test]
async fn a_receipt_on_the_latest_thread_reply_shows_when_the_root_left_the_window() {
    let server = MatrixMockServer::new().await;
    let client = server.client_builder().build().await;
    let room_id = room_id!("!issue-1783-off-window:example.invalid");
    let alice = user_id!("@alice:example.invalid");
    let bob = user_id!("@bob:example.invalid");
    let carol = user_id!("@carol:example.invalid");
    let factory = EventFactory::new().room(room_id);
    let root = id("root");
    let main = id("main");
    let reply = id("reply");

    // The root itself was never loaded (#1657): the window holds only a later
    // main message and the thread reply, and the root's own event has no
    // receipts of its own either.
    let room = server.sync_joined_room(&client, room_id).await;
    let sync = JoinedRoomBuilder::new(room_id)
        .set_timeline_limited()
        .set_timeline_prev_batch("before-live")
        .add_timeline_event(
            factory
                .text_msg("Synthetic main")
                .sender(alice)
                .event_id(&main)
                .server_ts(2_000_u64)
                .into_raw_sync(),
        )
        .add_timeline_event(
            factory
                .text_msg("Synthetic reply")
                .sender(bob)
                .event_id(&reply)
                .server_ts(3_000_u64)
                .in_thread(&root, &root)
                .into_raw_sync(),
        );
    server
        .sync_room(
            &client,
            with_unthreaded_receipt(sync, &factory, &reply, carol),
        )
        .await;

    // The thread list still knows the trimmed-out root, which is what draws its
    // row at the retained activity.
    let mut root_item = timeline_item(root.as_str(), Some("Synthetic root"), bob.as_str(), false);
    root_item.timestamp_ms = Some(1_000);
    root_item.thread_summary = Some(koushi_protocol::event::ThreadSummaryDto {
        reply_count: 1,
        unread_count: 0,
        latest_event_id: Some(reply.to_string()),
        latest_sender: Some(bob.to_string()),
        latest_sender_label: None,
        latest_body_preview: Some("Synthetic reply".into()),
        latest_timestamp_ms: Some(3_000),
    });
    let thread_roots = thread_roots();
    thread_roots
        .lock()
        .expect("thread root projection service lock")
        .seed_canonical_root(room_id.as_str(), &root_item);

    let ActorFixture {
        actor,
        mut events,
        mut actions,
    } = spawn_room_actor(&client, &room, alice.as_str(), thread_roots).await;
    let mut items = Vec::new();
    wait_for_moved_root_row(&mut events, &mut items, root.as_str(), reply.as_str()).await;

    // The initial window observation already publishes this receipt.
    let mut state = AppState {
        session: SessionState::Ready(session_info(alice.as_str())),
        ..AppState::default()
    };
    reduce_until_reader_is_published(&mut actions, &mut state, room_id.as_str(), carol.as_str())
        .await;
    assert_moved_root_row_publishes(
        &items,
        &state,
        room_id.as_str(),
        root.as_str(),
        reply.as_str(),
        &[carol],
    );
    drop(actor);
}
