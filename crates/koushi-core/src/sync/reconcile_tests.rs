use super::*;

#[tokio::test(start_paused = true)]
async fn slow_room_reconciliation_keeps_sync_alive_until_matching_ack() {
    let (room_tx, mut room_rx) = mpsc::channel(1);
    let reconciliation =
        tokio::spawn(async move { reconcile_committed_room_list(&room_tx, 7, 226).await });
    let RoomMessage::ReconcileCommittedRange { ack, .. } = room_rx.recv().await.unwrap() else {
        panic!("expected reconciliation request");
    };

    // Model a busy room observer after offline/sleep without waiting in real time.
    tokio::time::advance(ROOM_OBSERVATION_ACK_TIMEOUT + Duration::from_millis(500)).await;
    tokio::task::yield_now().await;
    assert!(
        !reconciliation.is_finished(),
        "slow live room projection must not terminate sync"
    );
    assert!(
        ack.send(RoomListReconcileAck::Projected {
            backend_generation: 7,
            room_generation: 1,
            response_sequence: 226,
        })
        .is_ok(),
        "late matching acknowledgement remains accepted"
    );
    assert_eq!(
        reconciliation.await.unwrap(),
        RoomListReconcileResult::Projected {
            response_sequence: 226
        }
    );
    assert!(room_rx.try_recv().is_err(), "do not enqueue duplicate work");
}

#[tokio::test(start_paused = true)]
async fn closed_or_invalid_ack_still_fails_after_delay() {
    for response in [
        None,
        Some(RoomListReconcileAck::Projected {
            backend_generation: 6,
            room_generation: 1,
            response_sequence: 226,
        }),
        Some(RoomListReconcileAck::Projected {
            backend_generation: 7,
            room_generation: 1,
            response_sequence: 225,
        }),
    ] {
        let (room_tx, mut room_rx) = mpsc::channel(1);
        let reconciliation =
            tokio::spawn(async move { reconcile_committed_room_list(&room_tx, 7, 226).await });
        let RoomMessage::ReconcileCommittedRange { ack, .. } = room_rx.recv().await.unwrap() else {
            panic!("expected reconciliation request");
        };
        tokio::time::advance(ROOM_OBSERVATION_ACK_TIMEOUT + Duration::from_secs(1)).await;
        match response {
            Some(response) => assert!(ack.send(response).is_ok()),
            None => drop(ack),
        }
        assert_eq!(
            reconciliation.await.unwrap(),
            RoomListReconcileResult::Failed
        );
    }
}
