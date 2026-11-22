//! #2059: the account avatar reaches every timeline and only changes are pushed.

use super::*;
use crate::timeline::test_support::{
    live_tail_test_manager, room_key, test_timeline_actor_handle, thread_key,
};

fn recording_handle() -> (TimelineActorHandle, mpsc::Receiver<TimelineActorMessage>) {
    let mut handle = test_timeline_actor_handle();
    let (tx, rx) = mpsc::channel(4);
    handle.tx = tx;
    (handle, rx)
}

#[tokio::test]
async fn an_account_avatar_reaches_every_timeline_including_a_thread() {
    let room = room_key();
    let thread = thread_key();
    let (room_handle, mut room_rx) = recording_handle();
    let (thread_handle, mut thread_rx) = recording_handle();
    let mut manager = live_tail_test_manager(HashMap::from([
        (room, room_handle),
        (thread, thread_handle),
    ]));

    manager
        .handle_own_avatar_updated(Some("mxc://example.org/own".to_owned()))
        .await;

    for rx in [&mut room_rx, &mut thread_rx] {
        assert!(matches!(
            rx.recv().await,
            Some(TimelineActorMessage::OwnAvatarUpdated(Some(uri)))
                if uri == "mxc://example.org/own"
        ));
    }

    // A repeated profile observation is not a change, so nothing is republished.
    manager
        .handle_own_avatar_updated(Some("mxc://example.org/own".to_owned()))
        .await;
    assert!(room_rx.try_recv().is_err());
    assert!(thread_rx.try_recv().is_err());
}
