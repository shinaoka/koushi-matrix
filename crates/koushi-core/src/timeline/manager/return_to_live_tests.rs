use super::*;
use crate::timeline::test_support::{
    fake_rid, live_tail_test_manager, room_key, test_timeline_actor_handle,
};

#[tokio::test]
async fn return_to_live_routes_to_the_existing_room_actor() {
    let key = room_key();
    let request_id = fake_rid(71);
    let mut handle = test_timeline_actor_handle();
    let (tx, mut rx) = mpsc::channel(1);
    handle.tx = tx;
    let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), handle)]));
    manager
        .handle_command(TimelineCommand::ReturnToLive { request_id, key })
        .await;
    assert!(matches!(
        rx.try_recv().unwrap(),
        TimelineActorMessage::ReturnToLive { request_id: routed } if routed == request_id
    ));
}

#[tokio::test]
async fn return_to_live_without_a_subscription_reports_correlated_failure() {
    let request_id = fake_rid(72);
    let mut manager = live_tail_test_manager(HashMap::new());
    let mut events = manager.event_tx.subscribe();
    manager
        .handle_command(TimelineCommand::ReturnToLive {
            request_id,
            key: room_key(),
        })
        .await;
    assert!(
        matches!(events.try_recv().unwrap(), CoreEvent::OperationFailed {
        request_id: failed,
        failure: CoreFailure::TimelineOperationFailed { kind: TimelineFailureKind::NotSubscribed },
    } if failed == request_id)
    );
}

#[test]
fn return_to_live_retains_request_identity_and_redacts_its_key() {
    let request_id = fake_rid(73);
    let command = TimelineCommand::ReturnToLive {
        request_id,
        key: room_key(),
    };
    let debug = format!("{command:?}");
    assert!(!debug.contains("@a:test"));
    assert!(!debug.contains("!r:test"));
    assert_eq!(
        koushi_protocol::command::CoreCommand::Timeline(command).request_id(),
        request_id
    );
}
