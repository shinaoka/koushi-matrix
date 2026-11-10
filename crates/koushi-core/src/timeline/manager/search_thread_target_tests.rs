//! A Search (or Activity / reply-quote) navigation whose target is a reply
//! inside a thread lands unreliably. The main pane opens a `Focused` key on
//! the reply, the production focus asks the SDK for
//! `TimelineEventFocusThreadMode::Automatic`, and the SDK silently switches
//! that event-focused timeline into thread mode (`Timeline::is_threaded()`):
//! the main pane receives only the thread's events, without the room messages
//! the server returned around the target. Forward pagination then ends at the
//! thread's last reply, Core reports that reply as the focused live tail, and
//! the renderer's auto return-to-live drops the reader into the live room
//! timeline, which hides thread replies, so the target disappears.
//!
//! Element X routes a permalink into a thread to the thread view. Core must
//! never present a thread-filtered projection as the main pane. Everything here
//! is synthetic.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use koushi_protocol::command::{InitialBackfillPolicy, TimelineCommand};
use koushi_protocol::event::{CoreEvent, TimelineEvent, TimelineItem};
use koushi_protocol::ids::{AccountKey, RequestId, RuntimeConnectionId, TimelineKey, TimelineKind};
use koushi_sdk::MatrixClientSession;
use koushi_state::SessionInfo;
use tokio::sync::{broadcast, mpsc};

use super::super::item_projection::timeline_item_event_id;
use super::super::navigation::NavigationProjectionIngress;
use super::super::test_support::live_tail_test_manager;
use super::TimelineMessage;
use crate::executor;

const ROOM: &str = "!search-thread:example.test";
const ROOT: &str = "$thread-root:example.test";
const ROOM_BEFORE: &str = "$room-before:example.test";
const TARGET: &str = "$thread-reply-target:example.test";
const ROOM_AFTER: &str = "$room-after:example.test";

fn request() -> RequestId {
    RequestId {
        connection_id: RuntimeConnectionId(1),
        sequence: 1,
    }
}

fn event_ids(items: &[TimelineItem]) -> Vec<&str> {
    items.iter().filter_map(timeline_item_event_id).collect()
}

#[tokio::test]
async fn main_pane_focus_on_a_thread_reply_keeps_the_surrounding_room_messages() {
    use matrix_sdk::ruma::{event_id, room_id, user_id};
    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    use matrix_sdk_base::ThreadingSupport;
    use matrix_sdk_test::event_factory::EventFactory;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, ResponseTemplate};

    let server = MatrixMockServer::new().await;
    // Production enables thread support (koushi-sdk client_session).
    let client = server
        .client_builder()
        .on_builder(|builder| {
            builder.with_threading_support(ThreadingSupport::Enabled {
                with_subscriptions: true,
            })
        })
        .build()
        .await;
    let room_id = room_id!("!search-thread:example.test");
    server.sync_joined_room(&client, room_id).await;
    let root = event_id!("$thread-root:example.test");
    let factory = EventFactory::new()
        .room(room_id)
        .sender(user_id!("@alice:example.test"));

    // Chronological room history: thread root, an ordinary room message, the
    // search hit (a reply in the thread), another ordinary room message.
    let root_event = factory
        .text_msg("synthetic root")
        .event_id(root)
        .into_event();
    let room_before = factory
        .text_msg("synthetic room message before")
        .event_id(event_id!("$room-before:example.test"))
        .into_event();
    let target = factory
        .text_msg("synthetic search hit")
        .event_id(event_id!("$thread-reply-target:example.test"))
        .in_thread(root, root)
        .into_event();
    let room_after = factory
        .text_msg("synthetic room message after")
        .event_id(event_id!("$room-after:example.test"))
        .into_event();
    Mock::given(method("GET"))
        .and(path_regex(r"/context/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "event": target.into_raw().json(),
            // `/context` lists earlier events newest first.
            "events_before": [room_before.into_raw().json(), root_event.into_raw().json()],
            "events_after": [room_after.into_raw().json()],
            "state": [],
        })))
        .mount(server.server())
        .await;

    let session = Arc::new(MatrixClientSession::from_client_for_testing(
        client,
        SessionInfo {
            homeserver: server.uri(),
            user_id: "@alice:example.test".to_owned(),
            device_id: "SYNTHETIC".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        },
    ));
    let mut manager = live_tail_test_manager(HashMap::new());
    manager.session = Some(session);
    let (action_tx, mut action_rx) = mpsc::channel(64);
    manager.action_tx = action_tx;
    let _action_drain = executor::spawn(async move { while action_rx.recv().await.is_some() {} });
    let (event_tx, mut event_rx) = broadcast::channel(256);
    manager.event_tx = event_tx;
    let (_ingress, navigation_rx) = NavigationProjectionIngress::channel();
    manager.navigation_projection_rx = Some(navigation_rx);
    let msg_tx = manager.msg_tx.clone();
    let _manager_task = executor::spawn(manager.run());

    // What `NavigateToEvent { source: Search, .. }` subscribes for the main
    // pane once the target is located.
    let key = TimelineKey {
        account_key: AccountKey("@alice:example.test".to_owned()),
        kind: TimelineKind::Focused {
            room_id: ROOM.to_owned(),
            event_id: TARGET.to_owned(),
        },
    };
    msg_tx
        .send(TimelineMessage::Command(TimelineCommand::Subscribe {
            request_id: request(),
            key: key.clone(),
            initial_backfill: InitialBackfillPolicy::Disabled,
        }))
        .await
        .expect("manager mailbox");

    let items = executor::timeout(Duration::from_secs(10), async {
        loop {
            match event_rx.recv().await {
                Ok(CoreEvent::Timeline(TimelineEvent::InitialItems {
                    key: event_key,
                    items,
                    ..
                })) if event_key == key => return items,
                Ok(CoreEvent::OperationFailed {
                    request_id,
                    failure,
                }) if request_id == request() => {
                    panic!("the focused subscribe failed: {failure:?}")
                }
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => panic!("event channel closed"),
            }
        }
    })
    .await
    .expect("the main-pane focused projection must settle");

    let ids = event_ids(&items);
    assert!(
        ids.contains(&TARGET),
        "precondition: the search hit itself is projected; got {ids:?}"
    );
    assert!(
        ids.contains(&ROOM_BEFORE) && ids.contains(&ROOM_AFTER),
        "the main pane received a thread-only projection for a search hit inside a thread \
         (root {ROOT}); the room messages around the hit are gone, so the view is not the \
         room and its live tail is the thread's last reply. Got {ids:?}"
    );
}
