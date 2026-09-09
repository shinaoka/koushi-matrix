//! Poll local reconciliation without blocking the sync owner's supervision.

use super::{ReplacementRecoveryProof, RoomListReconcileResult, SyncObserverStop};
use matrix_sdk::encryption::EncryptionSyncReadinessSnapshot;
use matrix_sdk_ui::{room_list_service::CommittedAllRoomsResponse, sync_service::State};

pub(super) type PendingRoomReconciliation =
    futures_util::future::BoxFuture<'static, (u64, RoomListReconcileResult)>;

pub(super) enum SyncObserverSignal {
    State(State),
    Committed(CommittedAllRoomsResponse),
    Encryption(EncryptionSyncReadinessSnapshot),
    Reconciled(u64, RoomListReconcileResult),
    Stopped,
    Closed(&'static str),
}

pub(super) fn retire_room_recovery(
    reconciliation: &mut Option<PendingRoomReconciliation>,
    pending_commit: &mut Option<CommittedAllRoomsResponse>,
    last_committed_sequence: &mut u64,
    replacement: &mut Option<ReplacementRecoveryProof>,
    latest_sequence: u64,
) {
    *reconciliation = None;
    *pending_commit = None;
    *last_committed_sequence = (*last_committed_sequence).max(latest_sequence);
    if let Some(proof) = replacement {
        proof.room_response_baseline = *last_committed_sequence;
        proof.latest_room_response_sequence = *last_committed_sequence;
        proof.room_response_committed = false;
    }
}

pub(super) async fn next_sync_observer_signal(
    stop: &SyncObserverStop,
    state: &mut eyeball::Subscriber<State>,
    committed: &mut eyeball::Subscriber<CommittedAllRoomsResponse>,
    encryption: &mut tokio::sync::watch::Receiver<EncryptionSyncReadinessSnapshot>,
    reconciliation: &mut Option<PendingRoomReconciliation>,
) -> SyncObserverSignal {
    if stop.is_requested() {
        *reconciliation = None;
        return SyncObserverSignal::Stopped;
    }
    let can_consume_commit = reconciliation.is_none();
    let signal = tokio::select! {
        biased;
        _ = stop.notify.notified() => SyncObserverSignal::Stopped,
        state = state.next() => match state {
            Some(state) => SyncObserverSignal::State(state),
            None => SyncObserverSignal::Closed("state_subscription_closed"),
        },
        changed = encryption.changed() => match changed {
            Ok(()) => SyncObserverSignal::Encryption(*encryption.borrow_and_update()),
            Err(_) => SyncObserverSignal::Closed("encryption_readiness_subscription_closed"),
        },
        (sequence, result) = async {
            match reconciliation.as_mut() {
                Some(wait) => wait.await,
                None => std::future::pending().await,
            }
        } => SyncObserverSignal::Reconciled(sequence, result),
        response = committed.next(), if can_consume_commit => match response {
            Some(response) => SyncObserverSignal::Committed(response),
            None => SyncObserverSignal::Closed("response_subscription_closed"),
        },
    };
    // Retire the receiver before exposing a terminal owner signal: a late ack
    // cannot satisfy the next SDK owner's recovery. Stop also cancels a send
    // waiting for mailbox capacity, since that send is part of this future.
    if matches!(
        signal,
        SyncObserverSignal::Stopped
            | SyncObserverSignal::State(State::Terminated | State::Offline(_) | State::Error(_))
    ) {
        *reconciliation = None;
    }
    signal
}
