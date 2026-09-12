use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures_util::stream::FuturesUnordered;
use koushi_sdk::MatrixClientSession;
use koushi_state::{AppAction, LiveEventReceipts};

use matrix_sdk::room::Receipts;
use matrix_sdk::ruma::api::client::receipt::create_receipt::v3::ReceiptType as SendReceiptType;
use matrix_sdk::ruma::events::receipt::ReceiptThread;
use tokio::sync::{mpsc, oneshot, watch};

use crate::executor;
use crate::read_state::{
    ReadAdmissionStatus, ReadCompletionDisposition, ReadNetworkFailure, ReadNetworkOutcome,
    ReadOperation, ReadOperationFence, ReadPersistenceSnapshot, ReadStateEngine, ReadStateKey,
    ReadTarget, ReadWaiterId, ReadWaiterTerminal, ReadWakeResult,
};
use koushi_protocol::event::{CoreEvent, LiveSignalsEvent, TimelineItem, TimelineReadStateSync};
use koushi_protocol::failure::{CoreFailure, ReadStateFailureKind, TimelineFailureKind};
use koushi_protocol::ids::{RequestId, TimelineKey, TimelineKind};

// BEGIN GENERATED SIBLING IMPORTS
use super::actor::{
    TimelineActor, TimelineActorControl, TimelineActorHandle, TimelineActorMessage,
    TimelinePositionIndex,
};
use super::diagnostics::{
    private_read_receipt_event_id_from_room_for_fully_read, read_state_key_for_command,
    read_state_room_id, record_read_admission, record_read_completion, record_read_retry,
    record_read_retry_scheduled, timeline_key_matches_read_state_key,
};
use super::item_projection::{
    is_attention_eligible_event, live_event_receipts_from_endpoint_changes, timeline_item_event_id,
    timeline_room_id,
};
use super::manager::{TimelineManagerActor, TimelineMessage};
use super::navigation::{derive_timeline_navigation_snapshot, record_timeline_unread_consistency};
use super::outbound_send::newest_provable_receipt_event_id;
// END GENERATED SIBLING IMPORTS

const READ_NETWORK_TIMEOUT: Duration = Duration::from_secs(30);

const READ_RETRY_BASE_DELAY: Duration = Duration::from_secs(1);

const READ_RETRY_MAX_DELAY: Duration = Duration::from_secs(60);
pub(super) const MAX_CONCURRENT_READ_WRITES: usize = 4;

#[derive(Clone)]
pub(crate) struct ReadPersistenceIngress {
    tx: watch::Sender<Option<ReadPersistenceRequest>>,
}

#[derive(Clone)]
pub(crate) struct ReadPersistenceRequest {
    session_generation: u64,
    save_generation: u64,
    snapshot: ReadPersistenceSnapshot,
}

impl ReadPersistenceRequest {
    pub(crate) fn new(
        session_generation: u64,
        save_generation: u64,
        snapshot: ReadPersistenceSnapshot,
    ) -> Self {
        Self {
            session_generation,
            save_generation,
            snapshot,
        }
    }

    pub(crate) fn session_generation(&self) -> u64 {
        self.session_generation
    }

    pub(crate) fn save_generation(&self) -> u64 {
        self.save_generation
    }

    pub(crate) fn snapshot(&self) -> &ReadPersistenceSnapshot {
        &self.snapshot
    }
}

impl std::fmt::Debug for ReadPersistenceRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReadPersistenceRequest")
            .field("session_generation", &self.session_generation)
            .field("save_generation", &self.save_generation)
            .field("entry_count", &self.snapshot.entry_count())
            .field("candidate_count", &self.snapshot.candidate_count())
            .finish()
    }
}

impl ReadPersistenceIngress {
    pub(crate) fn channel() -> (Self, watch::Receiver<Option<ReadPersistenceRequest>>) {
        let (tx, rx) = watch::channel(None);
        (Self { tx }, rx)
    }

    pub(crate) fn publish(&self, request: ReadPersistenceRequest) {
        self.tx.send_replace(Some(request));
    }
}

#[derive(Clone)]
enum ReadNetworkContext {
    Matrix(Arc<MatrixClientSession>),
    #[cfg(test)]
    Synthetic {
        requests: mpsc::UnboundedSender<SyntheticReadNetworkRequest>,
    },
}

#[cfg(test)]
struct SyntheticReadNetworkRequest {
    operation: ReadOperation,
    response: oneshot::Sender<Result<(), ()>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReadCommandKind {
    Receipt,
    FullyRead,
}

#[derive(Clone, Copy)]
pub(super) enum ReadRetrySource {
    Backoff,
    Reconnect,
    Checkpoint,
    AuthoritativeReceipt,
    SyncReconciliation,
}

impl ReadRetrySource {
    pub(super) fn token(self) -> &'static str {
        match self {
            Self::Backoff => "backoff",
            Self::Reconnect => "reconnect",
            Self::Checkpoint => "checkpoint",
            Self::AuthoritativeReceipt => "authoritative_receipt",
            Self::SyncReconciliation => "sync_reconciliation",
        }
    }
}

pub(super) struct ReadCommandWaiter {
    pub(super) request_id: RequestId,
    key: TimelineKey,
    event_id: String,
    kind: ReadCommandKind,
}

struct LocalReadCorrelation {
    actor_generation: u64,
    local_target: ReadTarget,
    server_confirmed_read_event_id: Option<String>,
    required_keys: std::collections::BTreeMap<ReadStateKey, ReadTarget>,
    admission_failure: Option<ReadStateFailureKind>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReadActorApplyKind {
    ThreadReceipt,
    FullyRead,
}

#[derive(Clone)]
pub(super) struct ReadRetryToken {
    epoch: Arc<()>,
    serial: u64,
}

impl PartialEq for ReadRetryToken {
    fn eq(&self, other: &Self) -> bool {
        self.serial == other.serial && Arc::ptr_eq(&self.epoch, &other.epoch)
    }
}

impl Eq for ReadRetryToken {}

pub(super) enum ReadWorkerCompletion {
    Network {
        operation: ReadOperation,
        outcome: ReadNetworkOutcome,
    },
    ActorApplied {
        operation: ReadOperation,
        applied: bool,
    },
    Cancelled {
        operation: ReadOperation,
    },
    RetryWake {
        key: ReadStateKey,
        generation: ReadRetryToken,
        cancelled: bool,
    },
}

impl ReadWorkerCompletion {
    fn fence(&self) -> Option<ReadOperationFence> {
        match self {
            Self::Network { operation, .. }
            | Self::ActorApplied { operation, .. }
            | Self::Cancelled { operation } => Some(operation.fence()),
            Self::RetryWake { .. } => None,
        }
    }
}

type ReadWorkerFuture = Pin<Box<dyn Future<Output = ReadWorkerCompletion> + Send + 'static>>;

pub(super) struct ReadWorkerSupervisor {
    state: ReadStateEngine,
    network: Option<ReadNetworkContext>,
    network_timeout: Duration,
    pub(super) tasks: FuturesUnordered<ReadWorkerFuture>,
    pub(super) retry_tasks: FuturesUnordered<ReadWorkerFuture>,
    cancellations: HashMap<ReadOperationFence, oneshot::Sender<()>>,
    pub(super) waiters: HashMap<ReadWaiterId, ReadCommandWaiter>,
    next_waiter_id: u64,
    retry_base_delay: Duration,
    retry_max_delay: Duration,
    retry_attempts: HashMap<ReadStateKey, u32>,
    /// Manager-wide token for distinguishing a current retry from cancelled
    /// sleepers without retaining one generation entry per historical key.
    retry_epoch: Arc<()>,
    retry_serial: u64,
    scheduled_retries: HashMap<ReadStateKey, (ReadRetryToken, oneshot::Sender<()>)>,
    ready: VecDeque<ReadStateKey>,
    queued: HashSet<ReadStateKey>,
    dispatch_failures: Vec<(ReadStateKey, crate::read_state::ReadCompletionResult)>,
    local_read_correlations: HashMap<TimelineKey, LocalReadCorrelation>,
    send_read_receipts: bool,
    reconciliation_pending: HashSet<ReadStateKey>,
    persistence: Option<ReadPersistenceIngress>,
    save_generation: u64,
}

impl ReadWorkerSupervisor {
    fn new(
        session_generation: u64,
        network: Option<ReadNetworkContext>,
        network_timeout: Duration,
    ) -> Self {
        Self {
            state: ReadStateEngine::new(session_generation),
            network,
            network_timeout,
            tasks: FuturesUnordered::new(),
            retry_tasks: FuturesUnordered::new(),
            cancellations: HashMap::new(),
            waiters: HashMap::new(),
            next_waiter_id: 0,
            retry_base_delay: READ_RETRY_BASE_DELAY,
            retry_max_delay: READ_RETRY_MAX_DELAY,
            retry_attempts: HashMap::new(),
            retry_epoch: Arc::new(()),
            retry_serial: 0,
            scheduled_retries: HashMap::new(),
            ready: VecDeque::new(),
            queued: HashSet::new(),
            dispatch_failures: Vec::new(),
            local_read_correlations: HashMap::new(),
            send_read_receipts: true,
            reconciliation_pending: HashSet::new(),
            persistence: None,
            save_generation: 0,
        }
    }

    pub(super) fn unavailable() -> Self {
        Self::new(0, None, READ_NETWORK_TIMEOUT)
    }

    pub(super) fn matrix(
        session: Arc<MatrixClientSession>,
        session_generation: u64,
        mut restored: ReadPersistenceSnapshot,
        persistence: ReadPersistenceIngress,
        send_read_receipts: bool,
    ) -> Self {
        let policy_removed_entries = restored.apply_receipt_policy(send_read_receipts);
        let reconciliation_pending = restored
            .entries()
            .iter()
            .map(|entry| entry.key().clone())
            .collect();
        let state = ReadStateEngine::restore(session_generation, restored)
            .unwrap_or_else(|| ReadStateEngine::new(session_generation));
        let mut supervisor = Self {
            state,
            network: Some(ReadNetworkContext::Matrix(session)),
            network_timeout: READ_NETWORK_TIMEOUT,
            tasks: FuturesUnordered::new(),
            retry_tasks: FuturesUnordered::new(),
            cancellations: HashMap::new(),
            waiters: HashMap::new(),
            next_waiter_id: 0,
            retry_base_delay: READ_RETRY_BASE_DELAY,
            retry_max_delay: READ_RETRY_MAX_DELAY,
            retry_attempts: HashMap::new(),
            retry_epoch: Arc::new(()),
            retry_serial: 0,
            scheduled_retries: HashMap::new(),
            ready: VecDeque::new(),
            queued: HashSet::new(),
            dispatch_failures: Vec::new(),
            local_read_correlations: HashMap::new(),
            send_read_receipts,
            reconciliation_pending,
            persistence: Some(persistence),
            save_generation: 0,
        };
        if policy_removed_entries {
            supervisor.publish_persistence();
        }
        for key in supervisor.reconciliation_pending.clone() {
            supervisor.schedule_retry(&key);
        }
        supervisor
    }

    #[cfg(test)]
    fn synthetic(
        requests: mpsc::UnboundedSender<SyntheticReadNetworkRequest>,
        timeout: Duration,
    ) -> Self {
        Self::new(1, Some(ReadNetworkContext::Synthetic { requests }), timeout)
    }

    #[cfg(test)]
    fn synthetic_with_retry(
        requests: mpsc::UnboundedSender<SyntheticReadNetworkRequest>,
        timeout: Duration,
        retry_base_delay: Duration,
        retry_max_delay: Duration,
    ) -> Self {
        let mut supervisor =
            Self::new(1, Some(ReadNetworkContext::Synthetic { requests }), timeout);
        supervisor.retry_base_delay = retry_base_delay;
        supervisor.retry_max_delay = retry_max_delay;
        supervisor
    }

    #[cfg(test)]
    fn synthetic_restored(
        requests: mpsc::UnboundedSender<SyntheticReadNetworkRequest>,
        restored: ReadPersistenceSnapshot,
        persistence: ReadPersistenceIngress,
    ) -> Self {
        let reconciliation_pending = restored
            .entries()
            .iter()
            .map(|entry| entry.key().clone())
            .collect();
        let mut supervisor = Self {
            state: ReadStateEngine::restore(7, restored)
                .expect("synthetic restored read state must be valid"),
            network: Some(ReadNetworkContext::Synthetic { requests }),
            network_timeout: Duration::from_secs(30),
            tasks: FuturesUnordered::new(),
            retry_tasks: FuturesUnordered::new(),
            cancellations: HashMap::new(),
            waiters: HashMap::new(),
            next_waiter_id: 0,
            retry_base_delay: Duration::from_secs(1),
            retry_max_delay: Duration::from_secs(4),
            retry_attempts: HashMap::new(),
            retry_epoch: Arc::new(()),
            retry_serial: 0,
            scheduled_retries: HashMap::new(),
            ready: VecDeque::new(),
            queued: HashSet::new(),
            dispatch_failures: Vec::new(),
            local_read_correlations: HashMap::new(),
            send_read_receipts: true,
            reconciliation_pending,
            persistence: Some(persistence),
            save_generation: 0,
        };
        for key in supervisor.reconciliation_pending.clone() {
            supervisor.schedule_retry(&key);
        }
        supervisor
    }

    fn allocate_waiter(&mut self) -> Option<ReadWaiterId> {
        let next = self.next_waiter_id.checked_add(1)?;
        self.next_waiter_id = next;
        Some(ReadWaiterId::new(next))
    }

    fn spawn_network(&mut self, operation: ReadOperation) -> bool {
        let Some(network) = self.network.clone() else {
            return false;
        };
        let timeout = self.network_timeout;
        let fence = operation.fence();
        let cancelled_operation = operation.clone();
        let (cancel, mut cancelled) = oneshot::channel();
        self.cancellations.insert(fence, cancel);
        self.tasks.push(Box::pin(async move {
            tokio::select! {
                biased;
                _ = &mut cancelled => ReadWorkerCompletion::Cancelled {
                    operation: cancelled_operation,
                },
                outcome = executor::timeout(timeout, perform_read_network_operation(
                    network,
                    &operation,
                )) => ReadWorkerCompletion::Network {
                    operation,
                    outcome: match outcome {
                        Ok(Ok(())) => ReadNetworkOutcome::Succeeded,
                        Ok(Err(failure)) => ReadNetworkOutcome::Failed(failure),
                        Err(_) => ReadNetworkOutcome::TimedOut,
                    },
                },
            }
        }));
        true
    }

    fn spawn_actor_apply<F>(&mut self, operation: ReadOperation, apply: F)
    where
        F: Future<Output = bool> + Send + 'static,
    {
        let timeout = self.network_timeout;
        let fence = operation.fence();
        let cancelled_operation = operation.clone();
        let (cancel, mut cancelled) = oneshot::channel();
        self.cancellations.insert(fence, cancel);
        self.tasks.push(Box::pin(async move {
            tokio::select! {
                biased;
                _ = &mut cancelled => ReadWorkerCompletion::Cancelled {
                    operation: cancelled_operation,
                },
                applied = executor::timeout(timeout, apply) => ReadWorkerCompletion::ActorApplied {
                    operation,
                    applied: applied.unwrap_or(false),
                },
            }
        }));
    }

    fn enqueue_key(&mut self, key: ReadStateKey) {
        if !self.send_read_receipts
            && matches!(
                &key,
                ReadStateKey::PublicUnthreaded { .. } | ReadStateKey::ThreadRead { .. }
            )
        {
            return;
        }
        if self.reconciliation_pending.contains(&key)
            || self.scheduled_retries.contains_key(&key)
            || self.state.active_operation(&key).is_some()
            || self.state.candidate_count(&key) == 0
        {
            return;
        }
        if self.queued.insert(key.clone()) {
            self.ready.push_back(key);
        }
    }

    /// The sole path that turns a desired key into an active operation. The
    /// queue is FIFO and the engine's active state is retained until the exact
    /// network/actor/cancel completion arrives, so cancellation cannot exceed
    /// the four-slot cap.
    fn dispatch_ready_reads(&mut self) {
        while self.state.active_operation_count() < MAX_CONCURRENT_READ_WRITES {
            let Some(key) = self.ready.pop_front() else {
                break;
            };
            self.queued.remove(&key);
            if self.reconciliation_pending.contains(&key)
                || self.scheduled_retries.contains_key(&key)
                || self.state.active_operation(&key).is_some()
                || self.state.candidate_count(&key) == 0
            {
                continue;
            }
            let ReadWakeResult::Start(operation) = self.state.wake(&key) else {
                continue;
            };
            if !self.spawn_network(operation.clone()) {
                let completion = self.state.complete(
                    operation.key(),
                    operation.fence(),
                    ReadNetworkOutcome::Failed(ReadNetworkFailure::new(ReadStateFailureKind::Sdk)),
                );
                self.dispatch_failures.push((key, completion));
            }
        }
    }

    fn take_dispatch_failures(
        &mut self,
    ) -> Vec<(ReadStateKey, crate::read_state::ReadCompletionResult)> {
        std::mem::take(&mut self.dispatch_failures)
    }

    fn cancel(&mut self, fence: ReadOperationFence) {
        if let Some(cancel) = self.cancellations.remove(&fence) {
            let _ = cancel.send(());
        }
    }

    fn finish(&mut self, completion: &ReadWorkerCompletion) {
        if let Some(fence) = completion.fence() {
            self.cancellations.remove(&fence);
        }
    }

    fn schedule_retry(&mut self, key: &ReadStateKey) {
        if self.scheduled_retries.contains_key(key) {
            return;
        }
        let attempt = self.retry_attempts.entry(key.clone()).or_default();
        let retry_after = self
            .state
            .last_failure(key)
            .and_then(|failure| failure.retry_after);
        let delay = read_retry_delay_for_attempt_with_retry_after(
            self.retry_base_delay,
            self.retry_max_delay,
            *attempt,
            retry_after,
        );
        let attempt_number = attempt.saturating_add(1);
        *attempt = attempt_number;
        record_read_retry_scheduled(
            key,
            attempt_number,
            self.queued.len(),
            self.state.active_operation_count(),
            delay,
        );
        self.retry_serial = match self.retry_serial.checked_add(1) {
            Some(serial) => serial,
            None => {
                // A stale retry future can still own the previous serial.
                // Rotate allocation identity before restarting the scalar so
                // no live stale token can compare equal to a fresh retry.
                self.retry_epoch = Arc::new(());
                1
            }
        };
        let generation = ReadRetryToken {
            epoch: self.retry_epoch.clone(),
            serial: self.retry_serial,
        };
        let cancelled_generation = generation.clone();
        let (cancel, mut cancelled) = oneshot::channel();
        self.scheduled_retries
            .insert(key.clone(), (generation.clone(), cancel));
        let key = key.clone();
        self.retry_tasks.push(Box::pin(async move {
            tokio::select! {
                _ = executor::sleep(delay) => ReadWorkerCompletion::RetryWake {
                    key,
                    generation,
                    cancelled: false,
                },
                _ = &mut cancelled => ReadWorkerCompletion::RetryWake {
                    key,
                    generation: cancelled_generation,
                    cancelled: true,
                },
            }
        }));
    }

    fn accept_retry_wake(&mut self, key: &ReadStateKey, generation: ReadRetryToken) -> bool {
        if self
            .scheduled_retries
            .get(key)
            .is_none_or(|(scheduled, _)| scheduled != &generation)
        {
            return false;
        }
        self.scheduled_retries.remove(key);
        true
    }

    fn invalidate_retry(&mut self, key: &ReadStateKey) {
        if let Some((_, cancel)) = self.scheduled_retries.remove(key) {
            let _ = cancel.send(());
        }
    }

    fn reset_retry(&mut self, key: &ReadStateKey) {
        self.invalidate_retry(key);
        self.retry_attempts.remove(key);
    }

    fn desired_keys(&self) -> Vec<ReadStateKey> {
        self.state
            .persistence_snapshot()
            .entries()
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    fn reconciliation_pending(&self, key: &ReadStateKey) -> bool {
        self.reconciliation_pending.contains(key)
    }

    fn finish_reconciliation(&mut self, key: &ReadStateKey) {
        self.reconciliation_pending.remove(key);
    }

    pub(super) fn publish_persistence(&mut self) {
        let Some(persistence) = self.persistence.as_ref() else {
            return;
        };
        self.save_generation = self.save_generation.wrapping_add(1).max(1);
        persistence.publish(ReadPersistenceRequest::new(
            self.state.session_generation(),
            self.save_generation,
            self.state.persistence_snapshot(),
        ));
    }

    pub(super) fn cancel_all(&mut self) {
        for (_, cancel) in self.cancellations.drain() {
            let _ = cancel.send(());
        }
        for (_, (_, cancel)) in self.scheduled_retries.drain() {
            let _ = cancel.send(());
        }
        self.tasks = FuturesUnordered::new();
        self.retry_tasks = FuturesUnordered::new();
        self.ready.clear();
        self.queued.clear();
        self.dispatch_failures.clear();
        self.retry_attempts.clear();
        self.local_read_correlations.clear();
    }

    fn remove_background_key(&mut self, key: &ReadStateKey) -> Vec<ReadWaiterId> {
        let (active, waiters) = self.state.retire_with_waiters(key);
        if let Some(fence) = active {
            self.cancel(fence);
        }
        self.invalidate_retry(key);
        self.retry_attempts.remove(key);
        self.queued.remove(key);
        self.ready.retain(|queued| queued != key);
        self.reconciliation_pending.remove(key);
        waiters
    }

    fn local_read_sync(&self, correlation: &LocalReadCorrelation) -> TimelineReadStateSync {
        if correlation.required_keys.is_empty() {
            return TimelineReadStateSync::NotRequested;
        }

        let mut pending = false;
        let mut desired = false;
        let mut failure = correlation.admission_failure;
        for key in correlation.required_keys.keys() {
            desired |= self.state.candidate_count(key) != 0;
            if let Some(candidate_failure) = self.state.last_failure(key) {
                failure = Some(select_read_failure(failure, candidate_failure.kind));
            }
            pending |= self.state.active_operation(key).is_some()
                || self.queued.contains(key)
                || self.reconciliation_pending.contains(key);
        }
        if pending {
            TimelineReadStateSync::Pending
        } else if let Some(kind) = failure {
            TimelineReadStateSync::Failed { kind }
        } else if desired
            || correlation
                .required_keys
                .keys()
                .any(|key| self.scheduled_retries.contains_key(key))
        {
            TimelineReadStateSync::Pending
        } else {
            TimelineReadStateSync::Synced
        }
    }

    pub(super) fn remove_local_read_correlation(&mut self, key: &TimelineKey) {
        let Some(correlation) = self.local_read_correlations.remove(key) else {
            return;
        };
        for read_key in correlation.required_keys.keys() {
            if let Some(active) = self.state.retire(read_key) {
                self.cancel(active);
            }
            self.queued.remove(read_key);
            self.ready.retain(|queued| queued != read_key);
            self.invalidate_retry(read_key);
            self.reconciliation_pending.remove(read_key);
            self.retry_attempts.remove(read_key);
        }
        self.publish_persistence();
        self.dispatch_ready_reads();
    }

    pub(super) fn send_read_receipts_enabled(&self) -> bool {
        self.send_read_receipts
    }

    #[cfg(test)]
    fn local_read_correlation_count(&self) -> usize {
        self.local_read_correlations.len()
    }

    #[cfg(test)]
    fn retry_bookkeeping_key_count(&self) -> usize {
        self.retry_attempts
            .keys()
            .chain(self.scheduled_retries.keys())
            .collect::<HashSet<_>>()
            .len()
    }
}

#[cfg(test)]
fn read_retry_delay_for_attempt(base: Duration, cap: Duration, attempt: u32) -> Duration {
    read_retry_delay_for_attempt_with_retry_after(base, cap, attempt, None)
}

fn select_read_failure(
    current: Option<ReadStateFailureKind>,
    candidate: ReadStateFailureKind,
) -> ReadStateFailureKind {
    fn priority(kind: ReadStateFailureKind) -> u8 {
        match kind {
            ReadStateFailureKind::Authentication => 5,
            ReadStateFailureKind::RateLimited => 4,
            ReadStateFailureKind::Timeout => 3,
            ReadStateFailureKind::Transport => 2,
            ReadStateFailureKind::Server => 1,
            ReadStateFailureKind::Capacity => 1,
            ReadStateFailureKind::Sdk => 0,
        }
    }

    current.map_or(candidate, |current| {
        if priority(candidate) > priority(current) {
            candidate
        } else {
            current
        }
    })
}

fn read_retry_delay_for_attempt_with_retry_after(
    base: Duration,
    cap: Duration,
    attempt: u32,
    retry_after: Option<Duration>,
) -> Duration {
    let multiplier = 1_u32.checked_shl(attempt.min(31)).unwrap_or(u32::MAX);
    let exponential = base.saturating_mul(multiplier).min(cap);
    match retry_after {
        Some(server_delay) if server_delay > cap => server_delay,
        Some(server_delay) => exponential.max(server_delay),
        None => exponential,
    }
}

impl Drop for ReadWorkerSupervisor {
    fn drop(&mut self) {
        self.cancel_all();
    }
}

async fn perform_read_network_operation(
    network: ReadNetworkContext,
    operation: &ReadOperation,
) -> Result<(), ReadNetworkFailure> {
    match network {
        ReadNetworkContext::Matrix(session) => {
            let room_id = matrix_sdk::ruma::RoomId::parse(match operation.key() {
                ReadStateKey::PublicUnthreaded { room_id }
                | ReadStateKey::ThreadRead { room_id, .. }
                | ReadStateKey::FullyReadAndPrivateUnthreaded { room_id } => room_id.as_str(),
            })
            .map_err(|_| ReadNetworkFailure::new(ReadStateFailureKind::Sdk))?;
            let event_id = matrix_sdk::ruma::EventId::parse(operation.target().event_id())
                .map_err(|_| ReadNetworkFailure::new(ReadStateFailureKind::Sdk))?;
            let room = session
                .client()
                .get_room(&room_id)
                .ok_or_else(|| ReadNetworkFailure::new(ReadStateFailureKind::Sdk))?;
            match operation.key() {
                ReadStateKey::PublicUnthreaded { .. } => room
                    .send_multiple_receipts(Receipts::new().public_read_receipt(event_id))
                    .await
                    .map_err(|error| classify_read_network_error(&error)),
                ReadStateKey::ThreadRead { root_event_id, .. } => {
                    let root_event_id = matrix_sdk::ruma::EventId::parse(root_event_id)
                        .map_err(|_| ReadNetworkFailure::new(ReadStateFailureKind::Sdk))?;
                    room.send_single_receipt(
                        SendReceiptType::Read,
                        ReceiptThread::Thread(root_event_id),
                        event_id,
                    )
                    .await
                    .map_err(|error| classify_read_network_error(&error))
                }
                ReadStateKey::FullyReadAndPrivateUnthreaded { .. } => {
                    let private_event_id = private_read_receipt_event_id_from_room_for_fully_read(
                        &room,
                        operation.target().event_id(),
                    );
                    let private_event_id = matrix_sdk::ruma::EventId::parse(private_event_id)
                        .map_err(|_| ReadNetworkFailure::new(ReadStateFailureKind::Sdk))?;
                    room.send_multiple_receipts(
                        Receipts::new()
                            .fully_read_marker(event_id)
                            .private_read_receipt(private_event_id),
                    )
                    .await
                    .map_err(|error| classify_read_network_error(&error))
                }
            }
        }
        #[cfg(test)]
        ReadNetworkContext::Synthetic { requests } => {
            let (response, outcome) = oneshot::channel();
            requests
                .send(SyntheticReadNetworkRequest {
                    operation: operation.clone(),
                    response,
                })
                .map_err(|_| ReadNetworkFailure::new(ReadStateFailureKind::Transport))?;
            match outcome.await.unwrap_or(Err(())) {
                Ok(()) => Ok(()),
                Err(()) => Err(ReadNetworkFailure::new(ReadStateFailureKind::Sdk)),
            }
        }
    }
}

fn classify_read_network_error(error: &matrix_sdk::Error) -> ReadNetworkFailure {
    match error {
        matrix_sdk::Error::Timeout => ReadNetworkFailure::new(ReadStateFailureKind::Timeout),
        matrix_sdk::Error::AuthenticationRequired => {
            ReadNetworkFailure::new(ReadStateFailureKind::Authentication)
        }
        matrix_sdk::Error::Http(http_error) => classify_http_error(http_error),
        _ => ReadNetworkFailure::new(ReadStateFailureKind::Sdk),
    }
}

fn classify_http_error(error: &matrix_sdk::HttpError) -> ReadNetworkFailure {
    use matrix_sdk::ruma::api::error::{ErrorKind, RetryAfter};

    if let Some(kind) = error.client_api_error_kind() {
        return match kind {
            ErrorKind::LimitExceeded(limit) => ReadNetworkFailure {
                kind: ReadStateFailureKind::RateLimited,
                retry_after: limit
                    .retry_after
                    .as_ref()
                    .and_then(|retry_after| match retry_after {
                        RetryAfter::Delay(duration) => Some(*duration),
                        RetryAfter::DateTime(_) => None,
                    }),
            },
            ErrorKind::MissingToken
            | ErrorKind::UnknownToken { .. }
            | ErrorKind::Unauthorized
            | ErrorKind::Forbidden => ReadNetworkFailure::new(ReadStateFailureKind::Authentication),
            _ => {
                let status = error
                    .as_client_api_error()
                    .map(|api_error| api_error.status_code.as_u16());
                if status.is_some_and(|status| (500..=599).contains(&status)) {
                    ReadNetworkFailure::new(ReadStateFailureKind::Server)
                } else {
                    ReadNetworkFailure::new(ReadStateFailureKind::Sdk)
                }
            }
        };
    }

    match error {
        matrix_sdk::HttpError::Reqwest(error) if error.is_timeout() => {
            ReadNetworkFailure::new(ReadStateFailureKind::Timeout)
        }
        matrix_sdk::HttpError::Reqwest(error)
            if error.status().is_some_and(|status| status.as_u16() == 429) =>
        {
            ReadNetworkFailure::new(ReadStateFailureKind::RateLimited)
        }
        matrix_sdk::HttpError::Reqwest(error)
            if error
                .status()
                .is_some_and(|status| status.is_server_error()) =>
        {
            ReadNetworkFailure::new(ReadStateFailureKind::Server)
        }
        matrix_sdk::HttpError::Reqwest(_) => {
            ReadNetworkFailure::new(ReadStateFailureKind::Transport)
        }
        matrix_sdk::HttpError::Cached(error) => classify_http_error(error),
        _ => ReadNetworkFailure::new(ReadStateFailureKind::Sdk),
    }
}

impl TimelineManagerActor {
    pub(super) async fn route_read_command(
        &mut self,
        request_id: RequestId,
        key: TimelineKey,
        event_id: String,
        kind: ReadCommandKind,
    ) {
        if self.read_workers.network.is_none() {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        }
        let Some(handle) = self.timelines.get(&key) else {
            self.emit_failure(
                request_id,
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::NotSubscribed,
                },
            );
            return;
        };
        if matrix_sdk::ruma::RoomId::parse(key.room_id()).is_err()
            || matrix_sdk::ruma::EventId::parse(event_id.as_str()).is_err()
            || matches!(
                &key.kind,
                TimelineKind::Thread { root_event_id, .. }
                    if matrix_sdk::ruma::EventId::parse(root_event_id.as_str()).is_err()
            )
        {
            self.emit_failure(
                request_id,
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::Sdk,
                },
            );
            return;
        }

        let read_key = read_state_key_for_command(&key, kind);
        if !self.read_workers.send_read_receipts
            && matches!(
                &read_key,
                ReadStateKey::PublicUnthreaded { .. } | ReadStateKey::ThreadRead { .. }
            )
        {
            self.emit_failure(
                request_id,
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::Forbidden,
                },
            );
            return;
        }
        let target = match handle.read_position(&event_id) {
            Some(position) => ReadTarget::with_position(event_id.clone(), position),
            None => ReadTarget::new(event_id.clone()),
        };
        let Some(waiter) = self.read_workers.allocate_waiter() else {
            self.emit_failure(
                request_id,
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::QueueOverflow,
                },
            );
            return;
        };
        let admission = self.read_workers.state.admit(
            self.read_workers.state.session_generation(),
            read_key.clone(),
            target,
            waiter,
        );
        record_read_admission(&read_key, admission.diagnostic());
        match admission.status() {
            ReadAdmissionStatus::Accepted | ReadAdmissionStatus::Coalesced => {
                self.read_workers.waiters.insert(
                    waiter,
                    ReadCommandWaiter {
                        request_id,
                        key,
                        event_id,
                        kind,
                    },
                );
            }
            ReadAdmissionStatus::Rejected(_) => {
                self.emit_failure(
                    request_id,
                    CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::QueueOverflow,
                    },
                );
                return;
            }
        }
        if let Some(superseded) = admission.superseded_operation() {
            self.read_workers.cancel(superseded);
        }
        self.read_workers.publish_persistence();
        self.wake_read_operation(&read_key).await;
    }

    async fn wake_read_operation(&mut self, key: &ReadStateKey) {
        if self.read_workers.reconciliation_pending(key) {
            self.read_workers.schedule_retry(key);
            return;
        }
        self.read_workers.enqueue_key(key.clone());
        self.read_workers.dispatch_ready_reads();
        self.drain_read_dispatch_failures().await;
    }

    async fn drain_read_dispatch_failures(&mut self) {
        for (key, completion) in self.read_workers.take_dispatch_failures() {
            record_read_completion(&key, completion.diagnostic());
            self.settle_read_waiters(completion.settlements().to_vec())
                .await;
            if matches!(
                completion.disposition(),
                ReadCompletionDisposition::Failed | ReadCompletionDisposition::TimedOut
            ) {
                self.read_workers.schedule_retry(&key);
            }
            self.read_workers.publish_persistence();
        }
    }

    pub(super) async fn wake_all_desired_reads(&mut self, source: ReadRetrySource) {
        for key in self.read_workers.desired_keys() {
            record_read_retry(
                &key,
                source,
                self.read_workers.state.candidate_count(&key),
                self.read_workers.state.waiter_count(&key),
            );
            if self.read_workers.reconciliation_pending(&key) {
                self.read_workers.schedule_retry(&key);
                continue;
            }
            self.wake_read_operation(&key).await;
        }
    }
    pub(super) async fn wake_desired_reads_for_room(
        &mut self,
        room_id: &str,
        source: ReadRetrySource,
    ) {
        let keys = self
            .read_workers
            .desired_keys()
            .into_iter()
            .filter(|key| read_state_room_id(key) == room_id)
            .collect::<Vec<_>>();
        for key in keys {
            record_read_retry(
                &key,
                source,
                self.read_workers.state.candidate_count(&key),
                self.read_workers.state.waiter_count(&key),
            );
            if self.read_workers.reconciliation_pending(&key) {
                self.read_workers.schedule_retry(&key);
                continue;
            }
            self.wake_read_operation(&key).await;
        }
    }
    pub(super) async fn handle_authoritative_read_state_observed(
        &mut self,
        timeline_key: &TimelineKey,
        actor_generation: u64,
        read_key: ReadStateKey,
        event_id: Option<String>,
    ) {
        let Some(position_index) = self
            .timelines
            .get(timeline_key)
            .and_then(TimelineActorHandle::read_position_index)
        else {
            return;
        };
        if position_index.actor_generation() != actor_generation
            || !timeline_key_matches_read_state_key(timeline_key, &read_key)
        {
            return;
        }
        self.ensure_restored_local_read_correlation(
            timeline_key,
            actor_generation,
            &position_index,
        );
        self.update_local_server_confirmation(
            timeline_key,
            actor_generation,
            &read_key,
            event_id.as_deref(),
        );
        self.project_unproven_restored_pending(
            timeline_key,
            actor_generation,
            &position_index,
            &read_key,
            event_id.as_deref(),
        )
        .await;
        let restored_entries = self.read_workers.state.persistence_snapshot();
        if let Some(entry) = restored_entries
            .entries()
            .iter()
            .find(|entry| entry.key() == &read_key)
        {
            for desired_event_id in entry.event_ids() {
                let Some(position) = position_index.evidence(desired_event_id) else {
                    continue;
                };
                self.read_workers.state.observe_position(
                    self.read_workers.state.session_generation(),
                    &read_key,
                    desired_event_id,
                    position,
                );
            }
        }
        let Some(event_id) = event_id else {
            self.read_workers.finish_reconciliation(&read_key);
            record_read_retry(
                &read_key,
                ReadRetrySource::SyncReconciliation,
                self.read_workers.state.candidate_count(&read_key),
                self.read_workers.state.waiter_count(&read_key),
            );
            self.read_workers.invalidate_retry(&read_key);
            self.wake_read_operation(&read_key).await;
            self.project_local_read_correlation(timeline_key).await;
            return;
        };
        let confirmed_position = position_index.evidence(&event_id);
        let confirmed = confirmed_position
            .map(|position| ReadTarget::with_position(event_id.clone(), position))
            .unwrap_or_else(|| ReadTarget::new(event_id));
        let confirmation = self.read_workers.state.confirm_authoritative(
            self.read_workers.state.session_generation(),
            &read_key,
            confirmed,
        );
        if let Some(superseded) = confirmation.superseded_operation() {
            self.read_workers.cancel(superseded);
        }
        self.settle_read_waiters(confirmation.settlements().to_vec())
            .await;
        let remaining = self.read_workers.state.candidate_count(&read_key);
        if remaining == 0 {
            self.read_workers.finish_reconciliation(&read_key);
            self.read_workers.reset_retry(&read_key);
        } else if self.read_workers.reconciliation_pending(&read_key)
            && (confirmed_position.is_none()
                || self
                    .read_workers
                    .state
                    .persistence_snapshot()
                    .entries()
                    .iter()
                    .find(|entry| entry.key() == &read_key)
                    .is_some_and(|entry| {
                        entry
                            .event_ids()
                            .iter()
                            .any(|event_id| position_index.evidence(event_id).is_none())
                    }))
        {
            // Different targets outside one current canonical position index
            // cannot be ordered safely. Keep the restored intent pending until
            // a later projection or receipt update supplies proof.
        } else {
            self.read_workers.finish_reconciliation(&read_key);
            record_read_retry(
                &read_key,
                ReadRetrySource::AuthoritativeReceipt,
                remaining,
                self.read_workers.state.waiter_count(&read_key),
            );
            self.read_workers.invalidate_retry(&read_key);
            self.wake_read_operation(&read_key).await;
        }
        self.read_workers.publish_persistence();
        self.project_local_read_correlation(timeline_key).await;
    }
    pub(super) async fn handle_read_worker_completion(&mut self, completion: ReadWorkerCompletion) {
        self.read_workers.finish(&completion);
        match completion {
            ReadWorkerCompletion::RetryWake {
                key,
                generation,
                cancelled,
            } => {
                if !cancelled && self.read_workers.accept_retry_wake(&key, generation) {
                    self.read_workers.finish_reconciliation(&key);
                    record_read_retry(
                        &key,
                        ReadRetrySource::Backoff,
                        self.read_workers.state.candidate_count(&key),
                        self.read_workers.state.waiter_count(&key),
                    );
                    self.wake_read_operation(&key).await;
                }
            }
            ReadWorkerCompletion::Cancelled { operation } => {
                self.settle_cancelled_read_operation(operation).await;
            }
            ReadWorkerCompletion::Network { operation, outcome } => {
                if outcome == ReadNetworkOutcome::Succeeded
                    && self.read_workers.state.active_operation(operation.key())
                        == Some(operation.fence())
                {
                    if !self
                        .read_workers
                        .state
                        .has_candidate(operation.key(), operation.target().event_id())
                    {
                        self.settle_read_operation(operation, outcome).await;
                        return;
                    }
                    match operation.key() {
                        ReadStateKey::PublicUnthreaded { .. } => {
                            let actor_is_current = self
                                .read_timeline_key_for_operation(&operation)
                                .is_some_and(|key| self.timelines.contains_key(&key));
                            if !actor_is_current {
                                self.settle_read_operation(
                                    operation,
                                    ReadNetworkOutcome::Failed(ReadNetworkFailure::new(
                                        ReadStateFailureKind::Sdk,
                                    )),
                                )
                                .await;
                                return;
                            }
                        }
                        ReadStateKey::ThreadRead { .. }
                        | ReadStateKey::FullyReadAndPrivateUnthreaded { .. } => {
                            if self.spawn_read_actor_apply(operation.clone()) {
                                return;
                            }
                            self.settle_read_operation(
                                operation,
                                ReadNetworkOutcome::Failed(ReadNetworkFailure::new(
                                    ReadStateFailureKind::Sdk,
                                )),
                            )
                            .await;
                            return;
                        }
                    }
                }
                self.settle_read_operation(operation, outcome).await;
            }
            ReadWorkerCompletion::ActorApplied { operation, applied } => {
                self.settle_read_operation(
                    operation,
                    if applied {
                        ReadNetworkOutcome::Succeeded
                    } else {
                        ReadNetworkOutcome::Failed(ReadNetworkFailure::new(
                            ReadStateFailureKind::Sdk,
                        ))
                    },
                )
                .await;
            }
        }
    }
    fn spawn_read_actor_apply(&mut self, operation: ReadOperation) -> bool {
        let apply_kind = match operation.key() {
            ReadStateKey::PublicUnthreaded { .. } => return false,
            ReadStateKey::ThreadRead { .. } => ReadActorApplyKind::ThreadReceipt,
            ReadStateKey::FullyReadAndPrivateUnthreaded { .. } => ReadActorApplyKind::FullyRead,
        };
        let Some(timeline_key) = self.read_timeline_key_for_operation(&operation) else {
            return false;
        };
        let Some(handle) = self.timelines.get(&timeline_key) else {
            return false;
        };
        let Some(control_tx) = handle.control_tx.clone() else {
            return false;
        };
        let event_id = operation.target().event_id().to_owned();
        self.read_workers.spawn_actor_apply(operation, async move {
            let (acknowledged, acknowledgement) = oneshot::channel();
            if control_tx
                .send(TimelineActorControl::ApplyReadSuccess {
                    kind: apply_kind,
                    event_id,
                    acknowledged,
                })
                .await
                .is_err()
            {
                return false;
            }
            acknowledgement.await.unwrap_or(false)
        });
        true
    }
    fn read_timeline_key_for_operation(&self, operation: &ReadOperation) -> Option<TimelineKey> {
        self.read_workers
            .waiters
            .values()
            .find_map(|waiter| {
                (waiter.event_id == operation.target().event_id()
                    && read_state_key_for_command(&waiter.key, waiter.kind) == *operation.key())
                .then(|| waiter.key.clone())
            })
            .or_else(|| {
                self.read_workers
                    .local_read_correlations
                    .keys()
                    .find(|key| timeline_key_matches_read_state_key(key, operation.key()))
                    .cloned()
            })
            .or_else(|| {
                self.timelines
                    .keys()
                    .find(|key| timeline_key_matches_read_state_key(key, operation.key()))
                    .cloned()
            })
    }
    async fn settle_cancelled_read_operation(&mut self, operation: ReadOperation) {
        let read_key = operation.key().clone();
        let completion = self
            .read_workers
            .state
            .complete_cancelled(&read_key, operation.fence());
        record_read_completion(&read_key, completion.diagnostic());
        if completion.disposition() == ReadCompletionDisposition::Cancelled {
            self.wake_read_operation(&read_key).await;
            self.read_workers.publish_persistence();
        }
    }

    async fn settle_read_operation(
        &mut self,
        operation: ReadOperation,
        outcome: ReadNetworkOutcome,
    ) {
        let read_key = operation.key().clone();
        let completion = self
            .read_workers
            .state
            .complete(&read_key, operation.fence(), outcome);
        let disposition = completion.disposition();
        if disposition == ReadCompletionDisposition::Succeeded {
            if let Some(timeline_key) = self.read_timeline_key_for_operation(&operation) {
                let actor_generation = self
                    .read_workers
                    .local_read_correlations
                    .get(&timeline_key)
                    .map_or(0, |correlation| correlation.actor_generation);
                self.update_local_server_confirmation(
                    &timeline_key,
                    actor_generation,
                    &read_key,
                    Some(operation.target().event_id()),
                );
            }
        }
        record_read_completion(&read_key, completion.diagnostic());
        let settlements = completion.settlements().to_vec();
        self.settle_read_waiters(settlements).await;
        match disposition {
            ReadCompletionDisposition::Succeeded => {
                self.read_workers.reset_retry(&read_key);
                self.wake_read_operation(&read_key).await;
            }
            ReadCompletionDisposition::Failed | ReadCompletionDisposition::TimedOut => {
                self.read_workers.schedule_retry(&read_key);
            }
            ReadCompletionDisposition::StaleDiscarded => {
                self.wake_read_operation(&read_key).await;
            }
            ReadCompletionDisposition::Cancelled => {}
        }
        if !matches!(
            disposition,
            ReadCompletionDisposition::StaleDiscarded | ReadCompletionDisposition::Cancelled
        ) {
            self.read_workers.publish_persistence();
        }
        if let Some(timeline_key) = self.read_timeline_key_for_operation(&operation) {
            self.project_local_read_correlation(&timeline_key).await;
        }
    }
    async fn settle_read_waiters(
        &mut self,
        settlements: Vec<crate::read_state::ReadWaiterSettlement>,
    ) {
        for settlement in settlements {
            let Some(waiter) = self.read_workers.waiters.remove(&settlement.waiter()) else {
                continue;
            };
            match settlement.terminal() {
                ReadWaiterTerminal::Converged => {
                    if waiter.kind == ReadCommandKind::FullyRead {
                        let room_id = waiter.key.room_id().to_owned();
                        if !self
                            .emit_action_reliable(AppAction::RoomMarkedAsReadSucceeded {
                                request_id: waiter.request_id.sequence,
                                room_id,
                            })
                            .await
                        {
                            self.emit_failure(
                                waiter.request_id,
                                CoreFailure::TimelineOperationFailed {
                                    kind: TimelineFailureKind::Sdk,
                                },
                            );
                            continue;
                        }
                        self.emit(CoreEvent::LiveSignals(LiveSignalsEvent::FullyReadSet {
                            request_id: waiter.request_id,
                            key: waiter.key,
                            event_id: waiter.event_id,
                        }));
                    } else {
                        self.emit(CoreEvent::LiveSignals(LiveSignalsEvent::ReadReceiptSent {
                            request_id: waiter.request_id,
                            key: waiter.key,
                            event_id: waiter.event_id,
                        }));
                    }
                }
                ReadWaiterTerminal::Failed | ReadWaiterTerminal::TimedOut => {
                    self.emit_failure(
                        waiter.request_id,
                        CoreFailure::TimelineOperationFailed {
                            kind: if settlement.terminal() == ReadWaiterTerminal::TimedOut {
                                TimelineFailureKind::Timeout
                            } else {
                                TimelineFailureKind::Sdk
                            },
                        },
                    );
                }
            }
        }
    }
    fn local_server_confirmation_key(timeline_key: &TimelineKey, read_key: &ReadStateKey) -> bool {
        match &timeline_key.kind {
            TimelineKind::Room { room_id } => {
                matches!(
                    read_key,
                    ReadStateKey::FullyReadAndPrivateUnthreaded { room_id: key_room }
                        if key_room == room_id
                )
            }
            TimelineKind::Thread { room_id, .. } => {
                matches!(
                    read_key,
                    ReadStateKey::ThreadRead { room_id: key_room, .. }
                        if key_room == room_id
                )
            }
            TimelineKind::Focused { .. } => false,
        }
    }

    async fn project_unproven_restored_pending(
        &mut self,
        timeline_key: &TimelineKey,
        actor_generation: u64,
        position_index: &TimelinePositionIndex,
        read_key: &ReadStateKey,
        authoritative_event_id: Option<&str>,
    ) {
        if self
            .read_workers
            .local_read_correlations
            .contains_key(timeline_key)
        {
            return;
        }
        let snapshot = self.read_workers.state.persistence_snapshot();
        let unproven = snapshot
            .entries()
            .iter()
            .find(|entry| entry.key() == read_key)
            .is_some_and(|entry| {
                entry
                    .event_ids()
                    .iter()
                    .any(|event_id| position_index.evidence(event_id).is_none())
            });
        if !unproven {
            return;
        }
        let Some(handle) = self.timelines.get(timeline_key) else {
            return;
        };
        if handle
            .read_position_index()
            .is_none_or(|index| index.actor_generation() != actor_generation)
        {
            return;
        }
        let server_confirmed_read_event_id =
            Self::local_server_confirmation_key(timeline_key, read_key)
                .then(|| authoritative_event_id.map(ToOwned::to_owned))
                .flatten();
        let _ = handle
            .send_control(TimelineActorControl::ReadStateProjection {
                local_viewed_event_id: None,
                server_confirmed_read_event_id,
                sync: TimelineReadStateSync::Pending,
            })
            .await;
    }

    fn ensure_restored_local_read_correlation(
        &mut self,
        timeline_key: &TimelineKey,
        actor_generation: u64,
        position_index: &TimelinePositionIndex,
    ) {
        if self
            .read_workers
            .local_read_correlations
            .contains_key(timeline_key)
            || self.read_workers.local_read_correlations.len()
                >= crate::read_state::READ_STATE_OUTBOX_ENTRY_LIMIT
        {
            return;
        }

        let snapshot = self.read_workers.state.persistence_snapshot();
        let mut required_keys = std::collections::BTreeMap::new();
        let mut local_target: Option<ReadTarget> = None;
        for entry in snapshot.entries() {
            let eligible = match (&timeline_key.kind, entry.key()) {
                (
                    TimelineKind::Room { room_id },
                    ReadStateKey::PublicUnthreaded { room_id: key_room },
                ) => self.read_workers.send_read_receipts && room_id == key_room,
                (
                    TimelineKind::Room { room_id },
                    ReadStateKey::FullyReadAndPrivateUnthreaded { room_id: key_room },
                ) => room_id == key_room,
                (
                    TimelineKind::Thread {
                        room_id,
                        root_event_id,
                    },
                    ReadStateKey::ThreadRead {
                        room_id: key_room,
                        root_event_id: key_root,
                    },
                ) => {
                    self.read_workers.send_read_receipts
                        && room_id == key_room
                        && root_event_id == key_root
                }
                _ => false,
            };
            if !eligible {
                continue;
            }
            let Some(event_id) = entry.event_ids().first() else {
                continue;
            };
            let Some(position) = position_index.evidence(event_id) else {
                continue;
            };
            let target = ReadTarget::with_position(event_id.clone(), position);
            if local_target
                .as_ref()
                .and_then(ReadTarget::position)
                .is_none_or(|current| position.rank > current.rank)
            {
                local_target = Some(target.clone());
            }
            required_keys.insert(entry.key().clone(), target);
        }
        let Some(local_target) = local_target else {
            return;
        };
        self.read_workers.local_read_correlations.insert(
            timeline_key.clone(),
            LocalReadCorrelation {
                actor_generation,
                local_target,
                server_confirmed_read_event_id: None,
                required_keys,
                admission_failure: None,
            },
        );
    }

    async fn project_local_read_correlation(&mut self, key: &TimelineKey) {
        let Some((actor_generation, local_viewed_event_id, server_confirmed_read_event_id, sync)) =
            self.read_workers
                .local_read_correlations
                .get(key)
                .map(|correlation| {
                    (
                        correlation.actor_generation,
                        correlation.local_target.event_id().to_owned(),
                        correlation.server_confirmed_read_event_id.clone(),
                        self.read_workers.local_read_sync(correlation),
                    )
                })
        else {
            return;
        };
        let Some(handle) = self.timelines.get(key) else {
            self.read_workers.local_read_correlations.remove(key);
            return;
        };
        if handle
            .read_position_index()
            .is_none_or(|index| index.actor_generation() != actor_generation)
        {
            return;
        }
        let _ = handle
            .send_control(TimelineActorControl::ReadStateProjection {
                local_viewed_event_id: Some(local_viewed_event_id),
                server_confirmed_read_event_id,
                sync,
            })
            .await;
    }

    fn update_local_server_confirmation(
        &mut self,
        timeline_key: &TimelineKey,
        actor_generation: u64,
        read_key: &ReadStateKey,
        event_id: Option<&str>,
    ) {
        if !Self::local_server_confirmation_key(timeline_key, read_key) {
            return;
        }
        let Some(correlation) = self
            .read_workers
            .local_read_correlations
            .get_mut(timeline_key)
        else {
            return;
        };
        if correlation.actor_generation == actor_generation
            && correlation.required_keys.contains_key(read_key)
            && event_id.is_some()
        {
            correlation.server_confirmed_read_event_id = event_id.map(ToOwned::to_owned);
        }
    }

    pub(super) async fn handle_local_read_boundary_observed(
        &mut self,
        key: TimelineKey,
        actor_generation: u64,
        target: ReadTarget,
    ) {
        if !matches!(
            key.kind,
            TimelineKind::Room { .. } | TimelineKind::Thread { .. }
        ) {
            return;
        }
        let Some(position_index) = self
            .timelines
            .get(&key)
            .and_then(TimelineActorHandle::read_position_index)
        else {
            return;
        };
        let Some(position) = target.position() else {
            return;
        };
        if position_index.actor_generation() != actor_generation
            || position_index.evidence(target.event_id()) != Some(position)
        {
            return;
        }

        if !self.read_workers.local_read_correlations.contains_key(&key)
            && self.read_workers.local_read_correlations.len()
                >= crate::read_state::READ_STATE_OUTBOX_ENTRY_LIMIT
        {
            return;
        }
        let previous_server_confirmed_read_event_id = self
            .read_workers
            .local_read_correlations
            .get(&key)
            .filter(|correlation| correlation.actor_generation == actor_generation)
            .and_then(|correlation| correlation.server_confirmed_read_event_id.clone());
        let mut required_keys = std::collections::BTreeMap::new();
        match &key.kind {
            TimelineKind::Room { room_id } => {
                if self.read_workers.send_read_receipts {
                    required_keys.insert(
                        ReadStateKey::PublicUnthreaded {
                            room_id: room_id.clone(),
                        },
                        target.clone(),
                    );
                }
                required_keys.insert(
                    ReadStateKey::FullyReadAndPrivateUnthreaded {
                        room_id: room_id.clone(),
                    },
                    target.clone(),
                );
            }
            TimelineKind::Thread {
                room_id,
                root_event_id,
            } if self.read_workers.send_read_receipts => {
                required_keys.insert(
                    ReadStateKey::ThreadRead {
                        room_id: room_id.clone(),
                        root_event_id: root_event_id.clone(),
                    },
                    target.clone(),
                );
            }
            TimelineKind::Thread { .. } => {}
            TimelineKind::Focused { .. } => return,
        }

        let mut admission_failure = None;
        let session_generation = self.read_workers.state.session_generation();
        let required = required_keys
            .iter()
            .map(|(read_key, read_target)| (read_key.clone(), read_target.clone()))
            .collect::<Vec<_>>();
        for (read_key, read_target) in &required {
            let admission = self.read_workers.state.admit_background(
                session_generation,
                read_key.clone(),
                read_target.clone(),
            );
            record_read_admission(read_key, admission.diagnostic());
            if let Some(superseded) = admission.superseded_operation() {
                self.read_workers.cancel(superseded);
            }
            if matches!(admission.status(), ReadAdmissionStatus::Rejected(_)) {
                admission_failure = Some(ReadStateFailureKind::Capacity);
            }
        }
        self.read_workers.local_read_correlations.insert(
            key.clone(),
            LocalReadCorrelation {
                actor_generation,
                local_target: target,
                server_confirmed_read_event_id: previous_server_confirmed_read_event_id,
                required_keys,
                admission_failure,
            },
        );
        self.read_workers.publish_persistence();
        self.project_local_read_correlation(&key).await;
        for (read_key, _) in required {
            self.wake_read_operation(&read_key).await;
        }
        self.project_local_read_correlation(&key).await;
    }

    pub(super) async fn handle_read_state_policy_changed(
        &mut self,
        session_generation: u64,
        send_read_receipts: bool,
    ) {
        if self.read_workers.state.session_generation() != session_generation {
            return;
        }
        self.read_workers.send_read_receipts = send_read_receipts;
        if !send_read_receipts {
            let blocked_keys = self
                .read_workers
                .desired_keys()
                .into_iter()
                .filter(|key| {
                    matches!(
                        key,
                        ReadStateKey::PublicUnthreaded { .. } | ReadStateKey::ThreadRead { .. }
                    )
                })
                .collect::<Vec<_>>();
            for blocked_key in blocked_keys {
                for waiter_id in self.read_workers.remove_background_key(&blocked_key) {
                    if let Some(waiter) = self.read_workers.waiters.remove(&waiter_id) {
                        self.emit_failure(
                            waiter.request_id,
                            CoreFailure::TimelineOperationFailed {
                                kind: TimelineFailureKind::Forbidden,
                            },
                        );
                    }
                }
            }
            self.read_workers.publish_persistence();
        }
        let keys = self
            .read_workers
            .local_read_correlations
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for key in &keys {
            let toggle_key = match &key.kind {
                TimelineKind::Room { room_id } => Some(ReadStateKey::PublicUnthreaded {
                    room_id: room_id.clone(),
                }),
                TimelineKind::Thread {
                    room_id,
                    root_event_id,
                } => Some(ReadStateKey::ThreadRead {
                    room_id: room_id.clone(),
                    root_event_id: root_event_id.clone(),
                }),
                TimelineKind::Focused { .. } => None,
            };
            let Some(toggle_key) = toggle_key else {
                continue;
            };
            if send_read_receipts {
                let target = self
                    .read_workers
                    .local_read_correlations
                    .get(key)
                    .map(|correlation| correlation.local_target.clone());
                if let Some(target) = target {
                    let mut should_admit = false;
                    if let Some(correlation) =
                        self.read_workers.local_read_correlations.get_mut(key)
                    {
                        should_admit = correlation
                            .required_keys
                            .insert(toggle_key.clone(), target.clone())
                            .is_none();
                        correlation.admission_failure = None;
                    }
                    if should_admit {
                        let admission = self.read_workers.state.admit_background(
                            session_generation,
                            toggle_key.clone(),
                            target,
                        );
                        record_read_admission(&toggle_key, admission.diagnostic());
                        if let Some(superseded) = admission.superseded_operation() {
                            self.read_workers.cancel(superseded);
                        }
                        if matches!(admission.status(), ReadAdmissionStatus::Rejected(_)) {
                            if let Some(correlation) =
                                self.read_workers.local_read_correlations.get_mut(key)
                            {
                                correlation.admission_failure =
                                    Some(ReadStateFailureKind::Capacity);
                            }
                        }
                    }
                }
            } else if let Some(correlation) = self.read_workers.local_read_correlations.get_mut(key)
            {
                correlation.required_keys.remove(&toggle_key);
                correlation.admission_failure = None;
            }
            self.read_workers.publish_persistence();
            if let Some(handle) = self.timelines.get(key) {
                let _ = handle
                    .send_control(TimelineActorControl::ReadStatePolicyChanged {
                        send_read_receipts,
                    })
                    .await;
            }
            self.project_local_read_correlation(key).await;
            if let Some(correlation) = self.read_workers.local_read_correlations.get(key) {
                let required = correlation
                    .required_keys
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>();
                for read_key in required {
                    self.wake_read_operation(&read_key).await;
                }
            }
            self.project_local_read_correlation(key).await;
        }
    }
}

/// Issue #872: the newest canonical item a settled viewport can claim the user
/// read, together with its navigation index.
///
/// A Room timeline hides thread replies behind their root row and reports the
/// root row's *activity* event id as the visible identity, so the newest
/// eligible canonical item beside the bottom row is usually a reply the room
/// never renders. Advancing the unthreaded viewed boundary onto it marks replies
/// read that the user never saw — and consumes the room's own unread badge with
/// them. Only an event that is itself a displayed row may be read.
fn viewed_boundary_target<'a>(
    kind: &TimelineKind,
    navigation_items: &'a [TimelineItem],
    display_items: &'a [TimelineItem],
    last_visible_event_id: &str,
) -> Option<(usize, &'a TimelineItem)> {
    let (target_index, target_item) = navigation_items
        .iter()
        .enumerate()
        .rev()
        .find(|(_, item)| is_attention_eligible_event(item))?;
    let koushi_protocol::event::TimelineItemId::Event { event_id } = &target_item.id else {
        return None;
    };
    if event_id != last_visible_event_id {
        return None;
    }
    if matches!(kind, TimelineKind::Room { .. })
        && !display_items
            .iter()
            .any(|item| timeline_item_event_id(item) == Some(event_id.as_str()))
    {
        return None;
    }
    Some((target_index, target_item))
}

impl TimelineActor {
    pub(super) fn observe_local_viewed_boundary(&mut self) -> Option<ReadTarget> {
        if !matches!(
            self.key.kind,
            TimelineKind::Room { .. } | TimelineKind::Thread { .. }
        ) || !self.viewport_observation.at_bottom
        {
            return None;
        }
        let last_visible_event_id = self.viewport_observation.last_visible_event_id.as_deref()?;
        let (target_index, target_item) = viewed_boundary_target(
            &self.key.kind,
            &self.navigation_items,
            self.display_projection.display_items(),
            last_visible_event_id,
        )?;
        let koushi_protocol::event::TimelineItemId::Event { event_id } = &target_item.id else {
            return None;
        };
        for visible_gap_id in &self.viewport_observation.visible_gap_ids {
            let Some((gap_index, _)) = self
                .gap_repair
                .projected_gaps
                .iter()
                .find(|(_, gap)| gap.id == *visible_gap_id)
            else {
                return None;
            };
            if *gap_index >= target_index {
                return None;
            }
        }
        let position = self.position_tx.borrow().evidence(event_id)?;
        let event_id = event_id.clone();
        if self.local_viewed_boundary.as_ref().is_some_and(|boundary| {
            boundary.event_id == event_id
                || (boundary.position.generation == position.generation
                    && boundary.position.rank >= position.rank)
        }) {
            return None;
        }
        self.local_viewed_boundary = Some(crate::timeline::actor::LocalViewedBoundary {
            event_id: event_id.clone(),
            position,
        });
        self.read_state_sync =
            if matches!(self.key.kind, TimelineKind::Thread { .. }) && !self.send_read_receipts {
                TimelineReadStateSync::NotRequested
            } else {
                TimelineReadStateSync::Pending
            };
        self.emit_navigation_if_changed();
        Some(ReadTarget::with_position(event_id, position))
    }

    pub(super) fn handle_read_state_projection(
        &mut self,
        local_viewed_event_id: Option<String>,
        server_confirmed_read_event_id: Option<String>,
        sync: TimelineReadStateSync,
    ) {
        if let Some(event_id) = local_viewed_event_id
            && let Some(position) = self.position_tx.borrow().evidence(&event_id)
            && self.local_viewed_boundary.as_ref().is_none_or(|boundary| {
                boundary.event_id != event_id
                    && (boundary.position.generation != position.generation
                        || boundary.position.rank < position.rank)
            })
        {
            self.local_viewed_boundary =
                Some(crate::timeline::actor::LocalViewedBoundary { event_id, position });
        }
        if server_confirmed_read_event_id.is_some() {
            self.server_confirmed_read_event_id = server_confirmed_read_event_id;
        }
        self.read_state_sync = sync;
        self.emit_navigation_if_changed();
    }

    pub(super) async fn handle_read_success(
        &mut self,
        kind: ReadActorApplyKind,
        event_id: String,
    ) -> bool {
        match kind {
            ReadActorApplyKind::ThreadReceipt => {
                if !matches!(self.key.kind, TimelineKind::Thread { .. }) {
                    return false;
                }
                let authoritative_event_id = newest_provable_receipt_event_id(
                    &self.navigation_items,
                    &event_id,
                    None,
                    self.thread_attention.receipt_event_id.as_deref(),
                );
                if let Some(action) = self.thread_attention.acknowledge(
                    &self.key,
                    &self.navigation_items,
                    authoritative_event_id.clone(),
                ) && !self.emit_action_reliable(action).await
                {
                    return false;
                }
                let snapshot = derive_timeline_navigation_snapshot(
                    &self.navigation_items,
                    self.fully_read_event_id.as_deref(),
                    &self.viewport_observation,
                    self.own_user_id.as_ref().map(|user_id| user_id.as_str()),
                );
                record_timeline_unread_consistency(
                    "thread_receipt_applied",
                    &self.key,
                    &self.navigation_items,
                    self.display_projection.display_items(),
                    self.last_navigation_snapshot.as_ref(),
                    &snapshot,
                    &self.thread_attention,
                );
                self.server_confirmed_read_event_id = Some(authoritative_event_id);
                self.emit_navigation_if_changed();
                true
            }
            ReadActorApplyKind::FullyRead => {
                let Some(room_id) = timeline_room_id(&self.key) else {
                    return false;
                };
                if !self
                    .emit_action_reliable(AppAction::FullyReadMarkerUpdated {
                        room_id,
                        event_id: Some(event_id.clone()),
                    })
                    .await
                {
                    return false;
                }
                self.fully_read_event_id = Some(event_id.clone());
                self.server_confirmed_read_event_id = Some(event_id);
                self.emit_navigation_if_changed();
                true
            }
        }
    }
    pub(super) async fn handle_own_read_receipt_changed(&mut self) {
        let Some(own_user_id) = self.own_user_id.as_deref() else {
            return;
        };
        let Some(event_id) = self
            .timeline
            .latest_user_read_receipt_timeline_event_id(own_user_id)
            .await
            .map(|event_id| event_id.to_string())
        else {
            return;
        };
        if let Some(action) =
            self.thread_attention
                .acknowledge(&self.key, &self.navigation_items, event_id.clone())
        {
            let _ = self.emit_action_reliable(action).await;
        }
        self.publish_authoritative_read_observation(
            read_state_key_for_command(&self.key, ReadCommandKind::Receipt),
            Some(event_id),
        )
        .await;
    }
    pub(super) async fn publish_authoritative_read_state(&self) {
        let receipt_event_id = if let Some(own_user_id) = self.own_user_id.as_deref() {
            self.timeline
                .latest_user_read_receipt_timeline_event_id(own_user_id)
                .await
                .map(|event_id| event_id.to_string())
        } else {
            None
        };
        self.publish_authoritative_read_observation(
            read_state_key_for_command(&self.key, ReadCommandKind::Receipt),
            receipt_event_id,
        )
        .await;
        let fully_read_event_id = matrix_sdk::ruma::RoomId::parse(self.key.room_id())
            .ok()
            .and_then(|room_id| self.session.client().get_room(&room_id))
            .and_then(|room| {
                room.fully_read_event_id()
                    .map(|event_id| event_id.to_string())
            });
        self.publish_authoritative_read_observation(
            read_state_key_for_command(&self.key, ReadCommandKind::FullyRead),
            fully_read_event_id,
        )
        .await;
    }
    async fn publish_authoritative_read_observation(
        &self,
        read_key: ReadStateKey,
        event_id: Option<String>,
    ) {
        let _ = self
            .manager_tx
            .send(TimelineMessage::AuthoritativeReadStateObserved {
                key: self.key.clone(),
                actor_generation: self.actor_generation,
                read_key,
                event_id,
            })
            .await;
    }
    pub(super) async fn handle_set_typing(&mut self, request_id: RequestId, is_typing: bool) {
        match self.timeline.room().typing_notice(is_typing).await {
            Ok(()) => {
                self.emit(CoreEvent::LiveSignals(LiveSignalsEvent::TypingSet {
                    request_id,
                    key: self.key.clone(),
                    is_typing,
                }));
            }
            Err(_) => {
                self.emit_failure(
                    request_id,
                    CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::Sdk,
                    },
                );
            }
        }
    }
    pub(super) fn live_receipts_from_endpoints(
        &self,
        changes: std::collections::BTreeMap<
            String,
            super::receipt_endpoints::ReceiptEndpointChange,
        >,
    ) -> Option<(String, Vec<LiveEventReceipts>)> {
        let Some(room_id) = timeline_room_id(&self.key) else {
            return None;
        };
        let receipts_by_event =
            live_event_receipts_from_endpoint_changes(changes, &self.receipt_endpoints);
        if receipts_by_event.is_empty() {
            return None;
        }
        Some((room_id, receipts_by_event))
    }
}

pub(super) async fn run_typing_notifications(
    actor_tx: mpsc::Sender<TimelineActorMessage>,
    _guard: matrix_sdk::event_handler::EventHandlerDropGuard,
    mut typing_rx: tokio::sync::broadcast::Receiver<Vec<matrix_sdk::ruma::OwnedUserId>>,
) {
    loop {
        match typing_rx.recv().await {
            Ok(user_ids) => {
                let user_ids = user_ids
                    .into_iter()
                    .map(|user_id| user_id.to_string())
                    .collect();
                if actor_tx
                    .send(TimelineActorMessage::TypingUsersUpdated(user_ids))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
}

#[cfg(test)]
mod tests;
