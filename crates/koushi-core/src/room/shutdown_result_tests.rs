use super::*;

#[tokio::test]
async fn panicked_room_owner_is_not_successful_shutdown() {
    let (action_tx, _action_rx) = mpsc::channel(16);
    let (event_tx, _) = broadcast::channel(16);
    let mut handle = RoomActor::spawn(action_tx, event_tx, Default::default());
    assert!(handle.shutdown().await);
    let (tx, mut rx) = mpsc::channel(1);
    handle.tx = tx;
    handle.task = Some(executor::spawn(async move {
        assert!(matches!(rx.recv().await, Some(RoomMessage::Shutdown)));
        panic!("synthetic room cleanup failure");
    }));
    assert!(
        !handle.shutdown().await,
        "a joined panic must not confirm cleanup"
    );
}
