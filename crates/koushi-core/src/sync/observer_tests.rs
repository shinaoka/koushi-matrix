use super::*;
use eyeball::Observable;
use matrix_sdk::encryption::{
    EncryptionSyncReadinessSnapshot as Snapshot, EncryptionSyncReadinessState,
};
use matrix_sdk_ui::{room_list_service::CommittedAllRoomsResponse, sync_service::State};

struct Signals {
    state: Observable<State>,
    state_sub: eyeball::Subscriber<State>,
    committed: Observable<CommittedAllRoomsResponse>,
    committed_sub: eyeball::Subscriber<CommittedAllRoomsResponse>,
    encryption: tokio::sync::watch::Sender<Snapshot>,
    encryption_sub: tokio::sync::watch::Receiver<Snapshot>,
    stop: SyncObserverStop,
    pending: Option<PendingRoomReconciliation>,
}

impl Signals {
    fn new(room_tx: mpsc::Sender<RoomMessage>) -> Self {
        let state = Observable::new(State::Running);
        let state_sub = Observable::subscribe(&state);
        let committed = Observable::new(CommittedAllRoomsResponse::default());
        let committed_sub = Observable::subscribe(&committed);
        let (encryption, encryption_sub) = tokio::sync::watch::channel(Snapshot {
            generation: 1,
            state: EncryptionSyncReadinessState::Received,
        });
        Self {
            state,
            state_sub,
            committed,
            committed_sub,
            encryption,
            encryption_sub,
            stop: SyncObserverStop::default(),
            pending: Some(Box::pin(async move {
                (226, reconcile_committed_room_list(&room_tx, 7, 226).await)
            })),
        }
    }

    async fn next(&mut self) -> Signal {
        next_sync_observer_signal(
            &self.stop,
            &mut self.state_sub,
            &mut self.committed_sub,
            &mut self.encryption_sub,
            &mut self.pending,
        )
        .await
    }

    async fn poll_pending(&mut self) {
        let mut next = Box::pin(self.next());
        assert!(futures_util::poll!(next.as_mut()).is_pending());
    }
}

fn take_ack(room_rx: &mut mpsc::Receiver<RoomMessage>) -> oneshot::Sender<RoomListReconcileAck> {
    let RoomMessage::ReconcileCommittedRange { ack, .. } = room_rx.try_recv().unwrap() else {
        panic!("expected reconciliation request");
    };
    ack
}

#[tokio::test(start_paused = true)]
async fn stop_cancels_slow_ack_and_prevents_late_success() {
    let (room_tx, mut room_rx) = mpsc::channel(1);
    let mut signals = Signals::new(room_tx);
    signals.poll_pending().await;
    let ack = take_ack(&mut room_rx);
    tokio::time::advance(ROOM_OBSERVATION_ACK_TIMEOUT + Duration::from_secs(1)).await;
    signals.poll_pending().await;
    signals.stop.request();
    assert!(matches!(signals.next().await, Signal::Stopped));
    assert!(signals.pending.is_none());
    assert!(ack.is_closed());
}

#[tokio::test]
async fn stop_cancels_submission_waiting_for_mailbox_capacity() {
    let (room_tx, mut room_rx) = mpsc::channel(1);
    let (occupied_ack, _occupied_rx) = oneshot::channel();
    room_tx
        .send(RoomMessage::ReconcileCommittedRange {
            source: RoomListSource::Live,
            backend_generation: 7,
            response_sequence: 225,
            ack: occupied_ack,
        })
        .await
        .unwrap();
    let mut signals = Signals::new(room_tx);
    signals.poll_pending().await;
    signals.stop.request();
    assert!(matches!(signals.next().await, Signal::Stopped));
    assert!(signals.pending.is_none());
    let _ = take_ack(&mut room_rx);
    assert!(room_rx.try_recv().is_err());
}

#[tokio::test]
async fn connectivity_loss_wins_over_ready_ack_and_retires_wait() {
    for state in [
        State::Terminated,
        State::Offline(Arc::new(matrix_sdk_ui::sync_service::Error::Supervisor)),
        State::Error(Arc::new(matrix_sdk_ui::sync_service::Error::Supervisor)),
    ] {
        let (room_tx, mut room_rx) = mpsc::channel(1);
        let mut signals = Signals::new(room_tx);
        signals.poll_pending().await;
        let ack = take_ack(&mut room_rx);
        assert!(
            ack.send(RoomListReconcileAck::Projected {
                backend_generation: 7,
                room_generation: 1,
                response_sequence: 226,
            })
            .is_ok()
        );
        Observable::set(&mut signals.state, state);
        assert!(matches!(
            signals.next().await,
            Signal::State(State::Terminated | State::Offline(_) | State::Error(_))
        ));
        assert!(
            signals.pending.is_none(),
            "old acknowledgement must not survive connectivity loss"
        );
    }
}

#[tokio::test]
async fn encryption_progress_and_retained_commit_survive_pending_reconciliation() {
    let (room_tx, mut room_rx) = mpsc::channel(1);
    let mut signals = Signals::new(room_tx);
    signals.poll_pending().await;
    let ack = take_ack(&mut room_rx);
    Observable::set(&mut signals.committed, CommittedAllRoomsResponse::default());
    signals
        .encryption
        .send(Snapshot {
            generation: 2,
            state: EncryptionSyncReadinessState::Received,
        })
        .unwrap();
    assert!(matches!(
        signals.next().await,
        Signal::Encryption(Snapshot { generation: 2, .. })
    ));
    signals.poll_pending().await;
    assert!(
        room_rx.try_recv().is_err(),
        "only one reconciliation request"
    );
    assert!(
        ack.send(RoomListReconcileAck::Projected {
            backend_generation: 7,
            room_generation: 1,
            response_sequence: 226,
        })
        .is_ok()
    );
    assert!(matches!(
        signals.next().await,
        Signal::Reconciled(226, RoomListReconcileResult::Projected { .. })
    ));
    signals.pending = None;
    assert!(matches!(signals.next().await, Signal::Committed(_)));
}

#[tokio::test]
async fn new_encryption_generation_cannot_use_pending_old_room_ack() {
    let (room_tx, mut room_rx) = mpsc::channel(1);
    let mut signals = Signals::new(room_tx);
    signals.poll_pending().await;
    let old_ack = take_ack(&mut room_rx);
    let mut replacement = Some(ReplacementRecoveryProof::new(1, 225));
    assert!(!replacement.as_mut().unwrap().observe_encryption(Snapshot {
        generation: 2,
        state: EncryptionSyncReadinessState::Received,
    }));
    let mut pending_commit = Some(CommittedAllRoomsResponse::default());
    let mut last_sequence = 225;
    retire_room_recovery(
        &mut signals.pending,
        &mut pending_commit,
        &mut last_sequence,
        &mut replacement,
        226,
    );
    assert!(old_ack.is_closed());
    assert!(pending_commit.is_none());
    let proof = replacement.as_mut().unwrap();
    assert!(!proof.observe_encryption(Snapshot {
        generation: 3,
        state: EncryptionSyncReadinessState::Received,
    }));
    assert!(!proof.observe_room_response(226));
    assert!(proof.observe_room_response(227));
}

#[test]
fn repeated_network_loss_retires_partial_room_proof_and_cached_commit() {
    let mut replacement = Some(ReplacementRecoveryProof::new(1, 225));
    assert!(!replacement.as_mut().unwrap().observe_room_response(226));
    let mut pending = None;
    let mut pending_commit = Some(CommittedAllRoomsResponse::default());
    let mut last_sequence = 226;
    retire_room_recovery(
        &mut pending,
        &mut pending_commit,
        &mut last_sequence,
        &mut replacement,
        227,
    );
    assert!(pending_commit.is_none());
    let received = Snapshot {
        generation: 2,
        state: EncryptionSyncReadinessState::Received,
    };
    assert!(!replacement.as_mut().unwrap().observe_encryption(received));
    assert!(!replacement.as_mut().unwrap().observe_room_response(227));
    assert!(replacement.as_mut().unwrap().observe_room_response(228));
    retire_room_recovery(
        &mut pending,
        &mut pending_commit,
        &mut last_sequence,
        &mut replacement,
        229,
    );
    assert!(!replacement.as_mut().unwrap().observe_encryption(received));
    assert!(!replacement.as_mut().unwrap().observe_room_response(229));
    assert!(replacement.as_mut().unwrap().observe_room_response(230));
}
