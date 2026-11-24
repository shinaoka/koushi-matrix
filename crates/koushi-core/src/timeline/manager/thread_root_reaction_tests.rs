//! #2080: the room timeline owns a thread root row's reaction groups, so it is
//! the actor that wakes a thread pane showing that root.

use std::collections::HashMap;

use tokio::sync::mpsc;

use crate::executor;
use koushi_protocol::ids::{AccountKey, TimelineKey, TimelineKind};

use super::super::actor::{
    ThreadSummaryProjectionIngress, TimelineActorControl, TimelineActorHandle, TimelineActorMessage,
};
use super::super::test_support::live_tail_test_manager;
use super::TimelineManagerActor;

/// An actor handle whose mailbox this test owns, so a control message is
/// observable instead of consumed by an actor loop.
fn recording_actor_handle() -> (TimelineActorHandle, mpsc::Receiver<TimelineActorMessage>) {
    let (tx, rx) = mpsc::channel(8);
    let drain = tx.clone();
    let task = executor::spawn(async move { drop(drain) });
    (
        TimelineActorHandle {
            tx,
            control_tx: None,
            thread_summary_projection: ThreadSummaryProjectionIngress::channel().0,
            position_rx: None,
            task: Some(task),
            auxiliary_tasks: Vec::new(),
            subscription_generation: None,
            enqueue_context: None,
        },
        rx,
    )
}

fn thread_key(room_id: &str, root_event_id: &str) -> TimelineKey {
    TimelineKey {
        account_key: AccountKey("@alice:example.test".to_owned()),
        kind: TimelineKind::Thread {
            room_id: room_id.to_owned(),
            root_event_id: root_event_id.to_owned(),
        },
    }
}

async fn next_control(rx: &mut mpsc::Receiver<TimelineActorMessage>) -> TimelineActorMessage {
    executor::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("the matching thread actor is woken")
        .expect("the actor mailbox stays open")
}

#[tokio::test]
async fn a_root_reaction_repaint_reaches_the_thread_actor_that_shows_that_root() {
    let matching = thread_key("!room:example.test", "$root:example.test");
    let other_root = thread_key("!room:example.test", "$other-root:example.test");
    let other_room = thread_key("!other-room:example.test", "$root:example.test");
    let (matching_handle, mut matching_rx) = recording_actor_handle();
    let (other_root_handle, mut other_root_rx) = recording_actor_handle();
    let (other_room_handle, mut other_room_rx) = recording_actor_handle();
    let room = TimelineKey::room(
        AccountKey("@alice:example.test".to_owned()),
        "!room:example.test",
    );

    let mut manager: TimelineManagerActor = live_tail_test_manager(HashMap::from([
        (matching.clone(), matching_handle),
        (other_root, other_root_handle),
        (other_room, other_room_handle),
        (room, recording_actor_handle().0),
    ]));

    manager
        .handle_thread_root_reactions_changed(
            AccountKey("@alice:example.test".to_owned()),
            "!room:example.test".to_owned(),
            "$root:example.test".to_owned(),
        )
        .await;

    assert!(
        matches!(
            next_control(&mut matching_rx).await,
            TimelineActorMessage::RefreshThreadRootReactions
        ),
        "the thread pane showing the root repaints"
    );
    assert!(
        executor::timeout(std::time::Duration::from_millis(200), other_root_rx.recv())
            .await
            .is_err(),
        "a thread pane showing another root is not woken"
    );
    assert!(
        executor::timeout(std::time::Duration::from_millis(200), other_room_rx.recv())
            .await
            .is_err(),
        "the same root id in another room is not woken"
    );
}

#[tokio::test]
async fn a_root_reaction_repaint_for_an_unretained_thread_is_a_no_op() {
    let mut manager: TimelineManagerActor = live_tail_test_manager(HashMap::new());
    manager
        .handle_thread_root_reactions_changed(
            AccountKey("@alice:example.test".to_owned()),
            "!room:example.test".to_owned(),
            "$root:example.test".to_owned(),
        )
        .await;
}

#[tokio::test]
async fn the_thread_control_maps_onto_the_repaint_message() {
    assert!(matches!(
        TimelineActorMessage::from(TimelineActorControl::RefreshThreadRootReactions),
        TimelineActorMessage::RefreshThreadRootReactions
    ));
}
