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
                (
                    226,
                    reconcile_committed_room_list(
                        &room_tx,
                        crate::SlidingSyncDiagnostics::default().owner(),
                        7,
                        226,
                    )
                    .await,
                )
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

// #1755: the first-response watchdog is instrumentation only. These tests pin
// the bounded window, the phase vocabulary and the one-record-per-episode
// contract without driving an SDK run.
fn first_response_records(
    cursor: koushi_diagnostics::test_support::DetailCursor,
) -> Vec<koushi_diagnostics::DiagnosticRecord> {
    koushi_diagnostics::test_support::detail_records_since(cursor)
        .into_iter()
        .filter(|record| record.event.source == "core.sync")
        .filter(|record| record.event.stage.starts_with("first_response_stalled"))
        .collect()
}

fn field_token<'a>(record: &'a koushi_diagnostics::DiagnosticRecord, key: &str) -> Option<&'a str> {
    record
        .event
        .fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| match field.value {
            koushi_diagnostics::DiagnosticValue::Token(token) => Some(token),
            _ => None,
        })
}

#[tokio::test]
async fn first_response_watchdog_reports_one_bounded_stall_and_clears_once() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock_async().await;
    let cursor = koushi_diagnostics::test_support::detail_cursor();
    let mut watchdog = FirstResponseWatchdog::new(Instant::now());
    assert_eq!(
        watchdog.deadline(),
        Some(watchdog.started_at + FIRST_RESPONSE_STALL_FIRST_WINDOW)
    );

    // Phases only move forward; the last observation wins.
    watchdog.advance(FirstResponsePhase::SyncRunning);
    watchdog.advance(FirstResponsePhase::EncryptionSyncRequested);
    watchdog.advance(FirstResponsePhase::SyncRunning);
    assert_eq!(watchdog.phase, FirstResponsePhase::EncryptionSyncRequested);

    watchdog.window_elapsed(7);
    let stalled = first_response_records(cursor);
    assert_eq!(stalled.len(), 1, "one record for the first window");
    assert_eq!(stalled[0].event.stage, "first_response_stalled");
    assert_eq!(
        stalled[0].event.level,
        koushi_diagnostics::DiagnosticLevel::Warn
    );
    assert_eq!(
        field_token(&stalled[0], "phase"),
        Some("encryption_sync_requested")
    );
    assert_eq!(
        stalled[0]
            .event
            .fields
            .iter()
            .find(|field| field.key == "generation")
            .map(|field| field.value.clone()),
        Some(koushi_diagnostics::DiagnosticValue::Count(7))
    );
    assert!(
        stalled[0]
            .event
            .fields
            .iter()
            .any(|field| field.key == "elapsed_ms"
                && matches!(
                    field.value,
                    koushi_diagnostics::DiagnosticValue::Milliseconds(_)
                )),
        "the stall record names its elapsed time"
    );

    // The window doubles, so an immediate second deadline is not due yet.
    assert!(
        watchdog.deadline() > Some(Instant::now() + FIRST_RESPONSE_STALL_FIRST_WINDOW),
        "the repeated window must back off"
    );
    assert_eq!(watchdog.window, FIRST_RESPONSE_STALL_FIRST_WINDOW * 2);

    watchdog.response_committed(7);
    assert_eq!(watchdog.deadline(), None);
    watchdog.response_committed(7);
    let records = first_response_records(cursor);
    assert_eq!(records.len(), 2, "one stall and one clear per episode");
    assert_eq!(records[1].event.stage, "first_response_stalled_cleared");
    assert_eq!(
        records[1].event.level,
        koushi_diagnostics::DiagnosticLevel::Debug
    );
    assert_eq!(
        field_token(&records[1], "phase"),
        Some("encryption_sync_requested")
    );
}

#[test]
fn first_response_stall_window_backs_off_to_its_cap() {
    let windows = std::iter::successors(Some(FIRST_RESPONSE_STALL_FIRST_WINDOW), |window| {
        Some(next_first_response_stall_window(*window))
    })
    .take(8)
    .collect::<Vec<_>>();
    assert_eq!(
        windows[1],
        Duration::from_secs(60),
        "the repeat waits longer"
    );
    assert!(
        windows.windows(2).all(|pair| pair[0] <= pair[1]),
        "the stall window never shrinks"
    );
    assert_eq!(
        *windows.last().expect("window sequence"),
        FIRST_RESPONSE_STALL_MAX_WINDOW,
        "a long stall repeats at the capped window instead of flooding"
    );
}

#[tokio::test]
async fn first_response_watchdog_is_silent_without_a_stall() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock_async().await;
    let cursor = koushi_diagnostics::test_support::detail_cursor();
    let mut watchdog = FirstResponseWatchdog::new(Instant::now());
    watchdog.advance(FirstResponsePhase::SyncRunning);
    watchdog.response_committed(3);
    assert_eq!(watchdog.deadline(), None);
    watchdog.window_elapsed(3);
    assert!(
        first_response_records(cursor).is_empty(),
        "a committed first response reports neither a stall nor a clear"
    );
    assert_eq!(
        first_response_diagnostic_event(
            "first_response_stalled",
            koushi_diagnostics::DiagnosticLevel::Warn,
            FirstResponsePhase::SyncStarted,
            Duration::from_millis(30_000),
            9,
        )
        .fields
        .len(),
        3,
        "phase, elapsed_ms and generation are the whole record"
    );
}
