use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures_util::stream::FuturesUnordered;
use koushi_sdk::MatrixClientSession;
use koushi_state::AppAction;

use matrix_sdk::room::Receipts;
use matrix_sdk::ruma::api::client::receipt::create_receipt::v3::ReceiptType as SendReceiptType;
use matrix_sdk::ruma::events::receipt::ReceiptThread;
use matrix_sdk_ui::timeline::TimelineItem as SdkTimelineItem;
use tokio::sync::{mpsc, oneshot, watch};

use crate::event::{CoreEvent, LiveSignalsEvent, TimelineReadStateSync};
use crate::executor;
use crate::failure::{CoreFailure, ReadStateFailureKind, TimelineFailureKind};
use crate::ids::{RequestId, TimelineKey, TimelineKind};
use crate::read_state::{
    ReadAdmissionStatus, ReadCompletionDisposition, ReadNetworkFailure, ReadNetworkOutcome,
    ReadOperation, ReadOperationFence, ReadPersistenceSnapshot, ReadStateEngine, ReadStateKey,
    ReadTarget, ReadWaiterId, ReadWaiterTerminal, ReadWakeResult,
};

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
    collect_live_event_receipts_from_diff, is_attention_eligible_event, timeline_room_id,
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
        let (target_index, target_item) = self
            .navigation_items
            .iter()
            .enumerate()
            .rev()
            .find(|(_, item)| is_attention_eligible_event(item))?;
        let crate::event::TimelineItemId::Event { event_id } = &target_item.id else {
            return None;
        };
        if event_id != last_visible_event_id {
            return None;
        }
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
    pub(super) fn live_receipts_action_from_sdk_diffs(
        key: &TimelineKey,
        diffs: &[eyeball_im::VectorDiff<Arc<SdkTimelineItem>>],
    ) -> Option<AppAction> {
        let Some(room_id) = timeline_room_id(key) else {
            return None;
        };
        let mut receipts_by_event = Vec::new();
        for diff in diffs {
            collect_live_event_receipts_from_diff(diff, &mut receipts_by_event);
        }
        if receipts_by_event.is_empty() {
            return None;
        }
        Some(AppAction::LiveRoomReceiptsUpdated {
            room_id,
            receipts_by_event,
        })
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
mod tests {
    use super::super::test_source::item_body;
    use futures_util::{FutureExt, StreamExt};

    use std::collections::{HashMap, HashSet};

    use std::sync::Arc;

    use std::time::Duration;

    use koushi_sdk::MatrixClientSession;
    use koushi_sdk::MatrixUserProfile;

    use koushi_state::UserProfile;
    use koushi_state::{AppAction, LiveEventReceipts, LiveReadReceipt};

    use matrix_sdk_ui::timeline::TimelineFocus;
    use tokio::sync::{broadcast, mpsc, oneshot, watch};

    use crate::command::TimelineCommand;
    use crate::event::{CoreEvent, LiveSignalsEvent, TimelineReadStateSync};
    use crate::executor;
    use crate::failure::{CoreFailure, ReadStateFailureKind, TimelineFailureKind};
    #[cfg(any(test, feature = "test-hooks"))]
    use crate::ids::AccountKey;
    use crate::ids::{TimelineKey, TimelineKind};

    use crate::read_state::{
        ReadPersistenceSnapshot, ReadStateEngine, ReadStateKey, ReadTarget, ReadWaiterId,
    };

    use koushi_diagnostics::DiagnosticValue;
    use koushi_state::{SessionInfo, SessionState};

    use super::super::actor::{TimelineActorControl, TimelineActorHandle, TimelinePositionIndex};
    use super::super::diagnostics::{
        FullyReadReceiptContext, private_read_receipt_event_id_for_fully_read,
    };
    use super::super::item_projection::{
        build_live_receipt_observation_actions, collect_live_event_receipts_from_diff,
        emit_live_receipt_observation_actions, live_receipt_observation_actions_from_sdk_receipts,
    };
    use super::super::manager::TimelineMessage;
    use super::super::navigation::TimelineActorGenerationGate;

    use super::super::relay::koushi_timeline_builder;
    use super::{
        MAX_CONCURRENT_READ_WRITES, ReadActorApplyKind, ReadCommandKind, ReadNetworkFailure,
        ReadNetworkOutcome, ReadPersistenceIngress, ReadRetrySource, ReadWorkerCompletion,
        ReadWorkerSupervisor, read_retry_delay_for_attempt,
    };

    use super::super::test_support::{
        fake_rid, live_tail_test_manager, room_key, test_timeline_actor_handle,
    };

    #[test]
    fn set_fully_read_success_uses_private_read_receipt_before_clearing_room_unread_summary() {
        let source = include_str!("read_state.rs");
        let network = source
            .split("async fn perform_read_network_operation")
            .nth(1)
            .expect("manager read network worker should exist")
            .split("async fn run_send_enqueue_future")
            .next()
            .expect("send worker should follow read worker");
        let actor_success = source
            .split("async fn handle_read_success")
            .nth(1)
            .expect("actor read success handler should exist")
            .split("async fn handle_own_read_receipt_changed")
            .next()
            .expect("own receipt handler should follow actor success");
        let manager_settlement = source
            .split("async fn settle_read_operation")
            .nth(1)
            .expect("manager settlement should exist")
            .split("async fn route_to_actor_or_fail")
            .next()
            .expect("actor route should follow read settlement");

        assert!(
            network.contains("send_multiple_receipts"),
            "set_fully_read must use SDK read-marker batching so the marker and read receipt share one source of truth"
        );
        assert!(
            network.contains("room.send_multiple_receipts"),
            "manager worker must force the room read-marker API; stale server unread counts still need a fresh private receipt"
        );
        assert!(
            network.contains("fully_read_marker"),
            "set_fully_read must continue to update the fully-read marker"
        );
        assert!(
            network.contains("private_read_receipt"),
            "set_fully_read must include a private read receipt so SDK/server unread counts advance without publishing public receipts"
        );
        assert!(
            !network.contains("send_single_receipt(ReceiptType::FullyRead"),
            "fully-read alone must not be used as the persistent unread-count source of truth"
        );
        assert!(
            actor_success.contains("AppAction::FullyReadMarkerUpdated")
                && actor_success.contains("emit_action_reliable"),
            "actor control success must reliably update the fully-read marker before ACK"
        );
        assert!(
            manager_settlement.contains("AppAction::RoomMarkedAsReadSucceeded"),
            "ACKed fully-read success must clear RoomSummary unread counts so sidebar and Activity/Unread agree"
        );
    }

    #[test]
    fn private_read_receipt_target_advances_to_hidden_edit_notification() {
        let target = private_read_receipt_event_id_for_fully_read(FullyReadReceiptContext {
            visible_event_id: "$visible:test",
            latest_event_id: Some("$latest-edit:test"),
            latest_event_relation_type: Some("m.replace"),
            unread_messages: 0,
            notification_count: 1,
        });

        assert_eq!(target, "$latest-edit:test");

        for context in [
            FullyReadReceiptContext {
                visible_event_id: "$visible:test",
                latest_event_id: Some("$latest-message:test"),
                latest_event_relation_type: None,
                unread_messages: 0,
                notification_count: 1,
            },
            FullyReadReceiptContext {
                visible_event_id: "$visible:test",
                latest_event_id: Some("$latest-edit:test"),
                latest_event_relation_type: Some("m.replace"),
                unread_messages: 1,
                notification_count: 1,
            },
            FullyReadReceiptContext {
                visible_event_id: "$visible:test",
                latest_event_id: Some("$latest-edit:test"),
                latest_event_relation_type: Some("m.replace"),
                unread_messages: 0,
                notification_count: 0,
            },
            FullyReadReceiptContext {
                visible_event_id: "$visible:test",
                latest_event_id: None,
                latest_event_relation_type: Some("m.replace"),
                unread_messages: 0,
                notification_count: 1,
            },
        ] {
            assert_eq!(
                private_read_receipt_event_id_for_fully_read(context),
                "$visible:test"
            );
        }
    }

    #[test]
    fn private_read_receipt_target_advances_to_hidden_thread_notification() {
        let target = private_read_receipt_event_id_for_fully_read(FullyReadReceiptContext {
            visible_event_id: "$visible:test",
            latest_event_id: Some("$latest-thread:test"),
            latest_event_relation_type: Some("m.thread"),
            unread_messages: 0,
            notification_count: 1,
        });

        assert_eq!(target, "$latest-thread:test");
    }

    #[test]
    fn send_read_receipt_uses_threaded_receipt_for_thread_timelines() {
        let source = include_str!("read_state.rs");
        let worker = source
            .split("async fn perform_read_network_operation")
            .nth(1)
            .expect("manager read worker should exist")
            .split("async fn run_send_enqueue_future")
            .next()
            .expect("send worker should follow read worker");

        assert!(
            worker.contains("ReadStateKey::ThreadRead"),
            "thread timeline receipts must remain a distinct manager-owned operation"
        );
        assert!(
            worker.contains("ReceiptThread::Thread"),
            "thread timeline read receipts must use ReceiptThread::Thread(root)"
        );
        assert!(
            worker.contains("send_single_receipt"),
            "threaded read receipts must use the SDK single-receipt endpoint that accepts a thread"
        );
    }

    fn restored_read_snapshot(key: ReadStateKey, event_id: &str) -> ReadPersistenceSnapshot {
        let mut engine = ReadStateEngine::new(7);
        engine.admit(
            7,
            key,
            ReadTarget::new(event_id.to_owned()),
            ReadWaiterId::new(1),
        );
        engine.persistence_snapshot()
    }

    fn restored_public_read_snapshot(room_id: &str, event_id: &str) -> ReadPersistenceSnapshot {
        restored_read_snapshot(
            ReadStateKey::PublicUnthreaded {
                room_id: room_id.to_owned(),
            },
            event_id,
        )
    }

    #[tokio::test]
    async fn twenty_read_keys_never_exceed_four_concurrent_writes() {
        let (network_tx, mut network_rx) = mpsc::unbounded_channel();
        let mut supervisor = ReadWorkerSupervisor::synthetic(network_tx, Duration::from_secs(30));
        let keys = (0..20)
            .map(|index| ReadStateKey::PublicUnthreaded {
                room_id: format!("!dispatcher-{index}:example.invalid"),
            })
            .collect::<Vec<_>>();
        for (index, key) in keys.iter().enumerate() {
            supervisor.state.admit_background(
                1,
                key.clone(),
                ReadTarget::new(format!("$dispatcher-{index}:example.invalid")),
            );
            supervisor.enqueue_key(key.clone());
        }
        supervisor.dispatch_ready_reads();

        let mut started = Vec::new();
        for expected in 0..keys.len() {
            assert!(supervisor.state.active_operation_count() <= MAX_CONCURRENT_READ_WRITES);
            let request = next_synthetic_request(&mut supervisor, &mut network_rx).await;
            started.push(request.operation.target().event_id().to_owned());
            let operation = request.operation.clone();
            request
                .response
                .send(Ok(()))
                .expect("release dispatcher slot");
            let _completion = supervisor.tasks.next().await.expect("write completion");
            supervisor.state.complete(
                operation.key(),
                operation.fence(),
                ReadNetworkOutcome::Succeeded,
            );
            supervisor.dispatch_ready_reads();
            if expected < keys.len() - 1 {
                assert!(supervisor.state.active_operation_count() <= MAX_CONCURRENT_READ_WRITES);
            }
        }

        assert_eq!(started.len(), 20);
        assert_eq!(supervisor.state.active_operation_count(), 0);
        assert_eq!(started[0], "$dispatcher-0:example.invalid");
        assert_eq!(started[19], "$dispatcher-19:example.invalid");
    }

    #[test]
    fn synchronous_dispatch_failures_are_all_retained_for_settlement() {
        let (network_tx, network_rx) = mpsc::unbounded_channel();
        drop(network_rx);
        let mut supervisor = ReadWorkerSupervisor::synthetic(network_tx, Duration::from_secs(30));
        supervisor.network = None;
        for index in 0..20 {
            let key = ReadStateKey::PublicUnthreaded {
                room_id: format!("!dispatch-failure-{index}:example.invalid"),
            };
            supervisor.state.admit_background(
                1,
                key.clone(),
                ReadTarget::new(format!("$dispatch-failure-{index}:example.invalid")),
            );
            supervisor.enqueue_key(key);
        }

        supervisor.dispatch_ready_reads();

        assert_eq!(supervisor.take_dispatch_failures().len(), 20);
        assert_eq!(
            supervisor.state.persistence_snapshot().candidate_count(),
            20
        );
    }

    #[tokio::test(start_paused = true)]
    async fn fifo_peers_start_before_a_failed_key_retries() {
        let (network_tx, mut network_rx) = mpsc::unbounded_channel();
        let mut supervisor = ReadWorkerSupervisor::synthetic_with_retry(
            network_tx,
            Duration::from_secs(30),
            Duration::from_secs(1),
            Duration::from_secs(60),
        );
        let keys = (0..6)
            .map(|index| ReadStateKey::PublicUnthreaded {
                room_id: format!("!fair-{index}:example.invalid"),
            })
            .collect::<Vec<_>>();
        for (index, key) in keys.iter().enumerate() {
            supervisor.state.admit_background(
                1,
                key.clone(),
                ReadTarget::new(format!("$fair-{index}:example.invalid")),
            );
            supervisor.enqueue_key(key.clone());
        }
        supervisor.dispatch_ready_reads();

        let mut initial = Vec::new();
        for _ in 0..4 {
            initial.push(next_synthetic_request(&mut supervisor, &mut network_rx).await);
        }
        let failed_index = initial
            .iter()
            .position(|request| request.operation.target().event_id() == "$fair-0:example.invalid")
            .expect("first FIFO key is active");
        let failed_request = initial.remove(failed_index);
        let failed = failed_request.operation.clone();
        failed_request
            .response
            .send(Err(()))
            .expect("fail first FIFO request");
        let _completion = supervisor.tasks.next().await.expect("failed completion");
        supervisor.state.complete(
            failed.key(),
            failed.fence(),
            ReadNetworkOutcome::Failed(ReadNetworkFailure::new(ReadStateFailureKind::Sdk)),
        );
        supervisor.schedule_retry(&keys[0]);
        supervisor.dispatch_ready_reads();

        let peer = next_synthetic_request(&mut supervisor, &mut network_rx).await;
        assert_eq!(
            peer.operation.target().event_id(),
            "$fair-4:example.invalid"
        );
        let peer_operation = peer.operation.clone();
        peer.response.send(Ok(())).expect("complete queued peer");
        let _completion = supervisor.tasks.next().await.expect("peer completion");
        supervisor.state.complete(
            peer_operation.key(),
            peer_operation.fence(),
            ReadNetworkOutcome::Succeeded,
        );
        supervisor.dispatch_ready_reads();

        let peer = next_synthetic_request(&mut supervisor, &mut network_rx).await;
        assert_eq!(
            peer.operation.target().event_id(),
            "$fair-5:example.invalid"
        );
        let peer_operation = peer.operation.clone();
        peer.response
            .send(Ok(()))
            .expect("complete second queued peer");
        let _completion = supervisor
            .tasks
            .next()
            .await
            .expect("second peer completion");
        supervisor.state.complete(
            peer_operation.key(),
            peer_operation.fence(),
            ReadNetworkOutcome::Succeeded,
        );
        supervisor.dispatch_ready_reads();

        for peer in initial {
            let peer_operation = peer.operation.clone();
            peer.response.send(Ok(())).expect("complete initial peer");
            let _completion = supervisor
                .tasks
                .next()
                .await
                .expect("initial peer completion");
            supervisor.state.complete(
                peer_operation.key(),
                peer_operation.fence(),
                ReadNetworkOutcome::Succeeded,
            );
            supervisor.dispatch_ready_reads();
        }

        assert!(supervisor.retry_tasks.next().now_or_never().is_none());
        tokio::time::advance(Duration::from_secs(1)).await;
        let retry = supervisor.retry_tasks.next().await.expect("due FIFO retry");
        let ReadWorkerCompletion::RetryWake {
            key,
            generation,
            cancelled: false,
        } = retry
        else {
            panic!("expected due retry wake");
        };
        assert!(supervisor.accept_retry_wake(&key, generation));
        supervisor.enqueue_key(key);
        supervisor.dispatch_ready_reads();
        let retried = next_synthetic_request(&mut supervisor, &mut network_rx).await;
        assert_eq!(
            retried.operation.target().event_id(),
            "$fair-0:example.invalid"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limit_retry_after_is_the_exact_dispatch_delay() {
        let key = ReadStateKey::PublicUnthreaded {
            room_id: "!retry-after:example.invalid".to_owned(),
        };
        let (network_tx, mut network_rx) = mpsc::unbounded_channel();
        let mut supervisor = ReadWorkerSupervisor::synthetic_with_retry(
            network_tx,
            Duration::from_secs(30),
            Duration::from_secs(1),
            Duration::from_secs(60),
        );
        supervisor.state.admit_background(
            1,
            key.clone(),
            ReadTarget::new("$retry-after:example.invalid".to_owned()),
        );
        supervisor.enqueue_key(key.clone());
        supervisor.dispatch_ready_reads();
        let request = next_synthetic_request(&mut supervisor, &mut network_rx).await;
        let operation = request.operation.clone();
        request
            .response
            .send(Err(()))
            .expect("fail retry-after request");
        let _completion = supervisor
            .tasks
            .next()
            .await
            .expect("retry-after completion");
        supervisor.state.complete(
            operation.key(),
            operation.fence(),
            ReadNetworkOutcome::Failed(ReadNetworkFailure::with_retry_after(
                ReadStateFailureKind::RateLimited,
                Duration::from_secs(7),
            )),
        );
        supervisor.schedule_retry(&key);

        tokio::time::advance(Duration::from_secs(6) + Duration::from_millis(999)).await;
        assert!(supervisor.retry_tasks.next().now_or_never().is_none());
        tokio::time::advance(Duration::from_millis(1)).await;
        let retry = supervisor
            .retry_tasks
            .next()
            .await
            .expect("exact retry-after wake");
        let ReadWorkerCompletion::RetryWake {
            key: retry_key,
            generation,
            cancelled: false,
        } = retry
        else {
            panic!("expected retry-after wake");
        };
        assert_eq!(retry_key, key);
        assert!(supervisor.accept_retry_wake(&key, generation));
        supervisor.enqueue_key(key);
        supervisor.dispatch_ready_reads();
        let retry = next_synthetic_request(&mut supervisor, &mut network_rx).await;
        assert_eq!(
            retry.operation.target().event_id(),
            "$retry-after:example.invalid"
        );
    }

    #[tokio::test]
    async fn cancellation_keeps_a_dispatch_slot_until_cancelled_completion() {
        let (network_tx, mut network_rx) = mpsc::unbounded_channel();
        let mut supervisor = ReadWorkerSupervisor::synthetic(network_tx, Duration::from_secs(30));
        let keys = (0..5)
            .map(|index| ReadStateKey::PublicUnthreaded {
                room_id: format!("!cancel-slot-{index}:example.invalid"),
            })
            .collect::<Vec<_>>();
        for (index, key) in keys.iter().enumerate() {
            supervisor.state.admit_background(
                1,
                key.clone(),
                ReadTarget::new(format!("$cancel-slot-{index}:example.invalid")),
            );
            supervisor.enqueue_key(key.clone());
        }
        supervisor.dispatch_ready_reads();
        let mut first_four = Vec::new();
        for _ in 0..4 {
            first_four.push(next_synthetic_request(&mut supervisor, &mut network_rx).await);
        }
        let cancelled = first_four[0].operation.clone();
        supervisor.cancel(cancelled.fence());
        supervisor.dispatch_ready_reads();
        assert_eq!(supervisor.state.active_operation_count(), 4);
        assert!(network_rx.try_recv().is_err());

        let cancellation = supervisor.tasks.next().await.expect("cancelled completion");
        assert!(matches!(
            cancellation,
            ReadWorkerCompletion::Cancelled { ref operation }
                if operation.fence() == cancelled.fence()
        ));
        supervisor
            .state
            .complete_cancelled(&keys[0], cancelled.fence());
        supervisor.dispatch_ready_reads();
        let next = next_synthetic_request(&mut supervisor, &mut network_rx).await;
        assert_eq!(
            next.operation.target().event_id(),
            "$cancel-slot-4:example.invalid"
        );
    }

    #[tokio::test]
    async fn local_read_correlation_projects_lifecycle_and_fences_stale_b_before_new_c() {
        let key = room_key();
        let (actor_handle, mut control_rx) =
            actor_handle_with_positions(7, [("$local-b:test", 2), ("$local-c:test", 3)]);
        let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
        let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
        manager.read_workers =
            ReadWorkerSupervisor::synthetic(read_network_tx, Duration::from_secs(30));

        manager
            .handle_local_read_boundary_observed(
                key.clone(),
                7,
                ReadTarget::with_position(
                    "$local-b:test".to_owned(),
                    crate::read_state::ReadPositionEvidence {
                        generation: 7_u128 << 64,
                        rank: 2,
                    },
                ),
            )
            .await;
        assert_eq!(manager.read_workers.local_read_correlation_count(), 1);
        assert_eq!(
            manager.read_workers.local_read_sync(
                manager
                    .read_workers
                    .local_read_correlations
                    .get(&key)
                    .expect("local B correlation")
            ),
            TimelineReadStateSync::Pending
        );
        let _public_b =
            next_synthetic_request(&mut manager.read_workers, &mut read_network_rx).await;
        let fully_b = next_synthetic_request(&mut manager.read_workers, &mut read_network_rx).await;

        manager
            .handle_local_read_boundary_observed(
                key.clone(),
                7,
                ReadTarget::with_position(
                    "$local-c:test".to_owned(),
                    crate::read_state::ReadPositionEvidence {
                        generation: 7_u128 << 64,
                        rank: 3,
                    },
                ),
            )
            .await;
        let stale_operation = fully_b.operation.clone();
        manager
            .handle_read_worker_completion(ReadWorkerCompletion::Network {
                operation: stale_operation,
                outcome: ReadNetworkOutcome::Succeeded,
            })
            .await;
        let correlation = manager
            .read_workers
            .local_read_correlations
            .get(&key)
            .expect("new C correlation");
        assert_eq!(correlation.local_target.event_id(), "$local-c:test");
        assert_eq!(correlation.server_confirmed_read_event_id, None);
        assert!(
            manager
                .read_workers
                .state
                .has_candidate(fully_b.operation.key(), "$local-c:test")
        );
        assert!(
            manager
                .read_workers
                .state
                .active_operation(fully_b.operation.key())
                .is_some(),
            "stale completion must refill its dispatcher slot with desired C"
        );
        let replacement = loop {
            tokio::select! {
                request = read_network_rx.recv() => {
                    break request.expect("replacement C synthetic request");
                }
                completion = manager.read_workers.tasks.next() => {
                    manager
                        .handle_read_worker_completion(
                            completion.expect("cancelled B completion before replacement C"),
                        )
                        .await;
                }
            }
        };
        assert_eq!(replacement.operation.target().event_id(), "$local-c:test");

        while let Ok(control) = control_rx.try_recv() {
            assert!(
                !matches!(control, TimelineActorControl::ApplyReadSuccess { .. }),
                "stale B success must not reach the actor after desired C replaces it"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn local_read_correlation_reports_failed_then_synced_and_capacity_truthfully() {
        let key = room_key();
        let (actor_handle, mut control_rx) = actor_handle_with_positions(7, [("$local-b:test", 2)]);
        let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
        let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
        manager.read_workers = ReadWorkerSupervisor::synthetic_with_retry(
            read_network_tx,
            Duration::from_secs(30),
            Duration::from_secs(1),
            Duration::from_secs(60),
        );
        manager
            .handle_local_read_boundary_observed(
                key.clone(),
                7,
                ReadTarget::with_position(
                    "$local-b:test".to_owned(),
                    crate::read_state::ReadPositionEvidence {
                        generation: 7_u128 << 64,
                        rank: 2,
                    },
                ),
            )
            .await;
        let mut failed_requests = Vec::new();
        for _ in 0..2 {
            failed_requests.push(
                next_synthetic_request(&mut manager.read_workers, &mut read_network_rx).await,
            );
        }
        for request in failed_requests {
            let operation = request.operation.clone();
            request.response.send(Err(())).expect("fail local read");
            let _completion = manager
                .read_workers
                .tasks
                .next()
                .await
                .expect("failed read");
            manager
                .handle_read_worker_completion(ReadWorkerCompletion::Network {
                    operation,
                    outcome: ReadNetworkOutcome::Failed(ReadNetworkFailure::new(
                        ReadStateFailureKind::Transport,
                    )),
                })
                .await;
        }
        let correlation = manager
            .read_workers
            .local_read_correlations
            .get(&key)
            .expect("failed local correlation");
        assert_eq!(
            manager.read_workers.local_read_sync(correlation),
            TimelineReadStateSync::Failed {
                kind: ReadStateFailureKind::Transport
            }
        );

        assert!(
            manager
                .read_workers
                .retry_tasks
                .next()
                .now_or_never()
                .is_none()
        );
        tokio::time::advance(Duration::from_secs(1)).await;
        for _ in 0..2 {
            let wake = manager
                .read_workers
                .retry_tasks
                .next()
                .await
                .expect("local retry wake");
            manager.handle_read_worker_completion(wake).await;
        }
        let mut successful_requests = Vec::new();
        for _ in 0..2 {
            successful_requests.push(
                next_synthetic_request(&mut manager.read_workers, &mut read_network_rx).await,
            );
        }
        for request in successful_requests {
            let operation = request.operation.clone();
            request
                .response
                .send(Ok(()))
                .expect("successful local read");
            let completion = manager
                .read_workers
                .tasks
                .next()
                .await
                .expect("retry completion");
            assert_eq!(completion.fence(), Some(operation.fence()));
            assert_eq!(
                manager.read_workers.state.active_operation(operation.key()),
                Some(operation.fence())
            );
            if matches!(
                operation.key(),
                ReadStateKey::FullyReadAndPrivateUnthreaded { .. }
            ) {
                assert_eq!(
                    manager.read_timeline_key_for_operation(&operation),
                    Some(key.clone())
                );
            }
            manager.handle_read_worker_completion(completion).await;
            if matches!(
                operation.key(),
                ReadStateKey::FullyReadAndPrivateUnthreaded { .. }
            ) {
                let acknowledge = async {
                    loop {
                        match control_rx.recv().await.expect("fully-read apply control") {
                            TimelineActorControl::ApplyReadSuccess { acknowledged, .. } => {
                                acknowledged
                                    .send(true)
                                    .expect("acknowledge fully-read apply");
                                break;
                            }
                            TimelineActorControl::ReadStateProjection { .. } => {}
                            TimelineActorControl::ReadStatePolicyChanged { .. } => {}
                            TimelineActorControl::ReplayInitialItems { .. }
                            | TimelineActorControl::StartLiveTailRefresh { .. }
                            | TimelineActorControl::CancelLiveTailNetwork { .. }
                            | TimelineActorControl::BeginGapRepairDemand
                            | TimelineActorControl::EndGapRepairDemand => {}
                        }
                    }
                };
                let (apply_completion, ()) =
                    tokio::join!(manager.read_workers.tasks.next(), acknowledge);
                manager
                    .handle_read_worker_completion(
                        apply_completion.expect("fully-read apply completion"),
                    )
                    .await;
            }
        }
        let correlation = manager
            .read_workers
            .local_read_correlations
            .get(&key)
            .expect("synced local correlation");
        assert_eq!(
            manager.read_workers.local_read_sync(correlation),
            TimelineReadStateSync::Synced
        );
        assert_eq!(
            correlation.server_confirmed_read_event_id.as_deref(),
            Some("$local-b:test")
        );

        let capacity_key = TimelineKey::room(
            AccountKey("@capacity:example.invalid".to_owned()),
            "!capacity-room:example.invalid",
        );
        let (capacity_actor, _capacity_controls) =
            actor_handle_with_positions(8, [("$capacity:test", 1)]);
        let mut capacity_manager =
            live_tail_test_manager(HashMap::from([(capacity_key.clone(), capacity_actor)]));
        let (capacity_tx, _capacity_rx) = mpsc::unbounded_channel();
        capacity_manager.read_workers =
            ReadWorkerSupervisor::synthetic(capacity_tx, Duration::from_secs(30));
        for index in 0..crate::read_state::READ_STATE_OUTBOX_ENTRY_LIMIT {
            capacity_manager.read_workers.state.admit_background(
                1,
                ReadStateKey::PublicUnthreaded {
                    room_id: format!("!capacity-fill-{index}:example.invalid"),
                },
                ReadTarget::new(format!("$capacity-fill-{index}:example.invalid")),
            );
        }
        capacity_manager
            .handle_local_read_boundary_observed(
                capacity_key.clone(),
                8,
                ReadTarget::with_position(
                    "$capacity:test".to_owned(),
                    crate::read_state::ReadPositionEvidence {
                        generation: 8_u128 << 64,
                        rank: 1,
                    },
                ),
            )
            .await;
        let capacity_correlation = capacity_manager
            .read_workers
            .local_read_correlations
            .get(&capacity_key)
            .expect("capacity admission keeps correlation");
        assert_eq!(
            capacity_manager.read_workers.local_read_correlation_count(),
            1
        );
        assert_eq!(
            capacity_manager
                .read_workers
                .local_read_sync(capacity_correlation),
            TimelineReadStateSync::Failed {
                kind: ReadStateFailureKind::Capacity
            }
        );
    }

    #[tokio::test]
    async fn thread_read_policy_toggle_preserves_local_correlation_and_not_requested_state() {
        let room_id = "!policy-room:example.invalid";
        let key = TimelineKey {
            account_key: AccountKey("@policy:example.invalid".to_owned()),
            kind: TimelineKind::Thread {
                room_id: room_id.to_owned(),
                root_event_id: "$policy-root:example.invalid".to_owned(),
            },
        };
        let (actor_handle, _control_rx) = actor_handle_with_positions(9, [("$policy:test", 4)]);
        let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
        let (read_network_tx, _read_network_rx) = mpsc::unbounded_channel();
        manager.read_workers =
            ReadWorkerSupervisor::synthetic(read_network_tx, Duration::from_secs(30));
        let (persistence, mut persistence_rx) = ReadPersistenceIngress::channel();
        manager.read_workers.persistence = Some(persistence);
        manager
            .handle_local_read_boundary_observed(
                key.clone(),
                9,
                ReadTarget::with_position(
                    "$policy:test".to_owned(),
                    crate::read_state::ReadPositionEvidence {
                        generation: 9_u128 << 64,
                        rank: 4,
                    },
                ),
            )
            .await;
        assert_eq!(manager.read_workers.local_read_correlation_count(), 1);
        assert_eq!(
            manager.read_workers.local_read_sync(
                manager
                    .read_workers
                    .local_read_correlations
                    .get(&key)
                    .expect("thread policy correlation")
            ),
            TimelineReadStateSync::Pending
        );

        let _ = persistence_rx.borrow_and_update();
        manager.handle_read_state_policy_changed(1, false).await;
        persistence_rx
            .changed()
            .await
            .expect("privacy disable publishes the reduced outbox");
        let disabled_snapshot = persistence_rx
            .borrow_and_update()
            .as_ref()
            .expect("privacy disable persistence request")
            .snapshot()
            .clone();
        assert!(disabled_snapshot.is_empty());
        let (restored_network_tx, mut restored_network_rx) = mpsc::unbounded_channel();
        let (restored_persistence, _restored_persistence_rx) = ReadPersistenceIngress::channel();
        let mut restored = ReadWorkerSupervisor::synthetic_restored(
            restored_network_tx,
            disabled_snapshot,
            restored_persistence,
        );
        restored.send_read_receipts = false;
        restored.dispatch_ready_reads();
        assert!(restored_network_rx.try_recv().is_err());

        let stale_snapshot = restored_public_read_snapshot(room_id, "$stale-policy:test");
        let (stale_network_tx, mut stale_network_rx) = mpsc::unbounded_channel();
        let mut stale_supervisor =
            ReadWorkerSupervisor::synthetic(stale_network_tx, Duration::from_secs(30));
        stale_supervisor.state = ReadStateEngine::restore(1, stale_snapshot)
            .expect("stale privacy snapshot restores for defense-in-depth check");
        stale_supervisor.send_read_receipts = false;
        for read_key in stale_supervisor.desired_keys() {
            stale_supervisor.enqueue_key(read_key);
        }
        stale_supervisor.dispatch_ready_reads();
        assert!(stale_network_rx.try_recv().is_err());

        assert_eq!(
            manager.read_workers.local_read_sync(
                manager
                    .read_workers
                    .local_read_correlations
                    .get(&key)
                    .expect("disabled thread policy correlation")
            ),
            TimelineReadStateSync::NotRequested
        );
        assert_eq!(manager.read_workers.local_read_correlation_count(), 1);

        manager.handle_read_state_policy_changed(1, true).await;
        assert_eq!(
            manager.read_workers.local_read_sync(
                manager
                    .read_workers
                    .local_read_correlations
                    .get(&key)
                    .expect("re-enabled thread policy correlation")
            ),
            TimelineReadStateSync::Pending
        );
    }

    #[tokio::test]
    async fn actor_retirement_retires_its_read_keys_and_persistence() {
        let key = room_key();
        let (actor_handle, _control_rx) = actor_handle_with_positions(10, [("$retired:test", 1)]);
        let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
        let (read_network_tx, _read_network_rx) = mpsc::unbounded_channel();
        manager.read_workers =
            ReadWorkerSupervisor::synthetic(read_network_tx, Duration::from_secs(30));
        manager
            .handle_local_read_boundary_observed(
                key.clone(),
                10,
                ReadTarget::with_position(
                    "$retired:test".to_owned(),
                    crate::read_state::ReadPositionEvidence {
                        generation: 10_u128 << 64,
                        rank: 1,
                    },
                ),
            )
            .await;
        assert!(!manager.read_workers.state.persistence_snapshot().is_empty());

        manager.read_workers.remove_local_read_correlation(&key);

        assert_eq!(manager.read_workers.local_read_correlation_count(), 0);
        assert!(manager.read_workers.state.persistence_snapshot().is_empty());
        assert_eq!(manager.read_workers.state.active_operation_count(), 0);
    }

    async fn next_synthetic_request(
        supervisor: &mut ReadWorkerSupervisor,
        receiver: &mut mpsc::UnboundedReceiver<super::SyntheticReadNetworkRequest>,
    ) -> super::SyntheticReadNetworkRequest {
        let mut completion = Box::pin(supervisor.tasks.next());
        tokio::select! {
            request = receiver.recv() => request.expect("synthetic read request"),
            _ = &mut completion => panic!("synthetic worker completed before request was observed"),
        }
    }

    fn actor_handle_with_positions(
        actor_generation: u64,
        positions: impl IntoIterator<Item = (&'static str, u64)>,
    ) -> (TimelineActorHandle, mpsc::Receiver<TimelineActorControl>) {
        let (tx, _rx) = mpsc::channel(1);
        let (control_tx, control_rx) = mpsc::channel(32);
        let (_position_tx, position_rx) = watch::channel(Arc::new(TimelinePositionIndex {
            generation: u128::from(actor_generation) << 64,
            ranks: positions
                .into_iter()
                .map(|(event_id, rank)| (event_id.to_owned(), rank))
                .collect(),
        }));
        (
            TimelineActorHandle {
                tx,
                control_tx: Some(control_tx),
                position_rx: Some(position_rx),
                task: None,
                auxiliary_tasks: Vec::new(),
                subscription_generation: None,
                enqueue_context: None,
            },
            control_rx,
        )
    }

    #[tokio::test]
    async fn restored_read_waits_for_authoritative_reconciliation_before_retrying() {
        let key = room_key();
        let read_key = ReadStateKey::PublicUnthreaded {
            room_id: key.room_id().to_owned(),
        };
        let (ordinary_tx, _ordinary_rx) = mpsc::channel(1);
        let (control_tx, mut control_rx) = mpsc::channel(8);
        let (_position_tx, position_rx) = watch::channel(Arc::new(TimelinePositionIndex {
            generation: u128::from(7_u64) << 64,
            ranks: HashMap::from([("$desired:test".to_owned(), 5)]),
        }));
        let actor_handle = TimelineActorHandle {
            tx: ordinary_tx,
            control_tx: Some(control_tx),
            position_rx: Some(position_rx),
            task: None,
            auxiliary_tasks: Vec::new(),
            subscription_generation: None,
            enqueue_context: None,
        };
        let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
        let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
        let (persistence, mut persistence_rx) = ReadPersistenceIngress::channel();
        manager.read_workers = ReadWorkerSupervisor::synthetic_restored(
            read_network_tx,
            restored_public_read_snapshot(key.room_id(), "$desired:test"),
            persistence,
        );

        manager
            .wake_all_desired_reads(ReadRetrySource::Reconnect)
            .await;
        assert!(manager.read_workers.tasks.is_empty());
        assert!(read_network_rx.try_recv().is_err());

        manager
            .handle_authoritative_read_state_observed(&key, 7, read_key, None)
            .await;
        assert!(matches!(
            control_rx.recv().await,
            Some(TimelineActorControl::ReadStateProjection {
                local_viewed_event_id: Some(event_id),
                server_confirmed_read_event_id: None,
                sync: TimelineReadStateSync::Pending,
            }) if event_id == "$desired:test"
        ));
        let responder = async {
            let retry = read_network_rx
                .recv()
                .await
                .expect("server-behind reconciliation starts retry");
            assert_eq!(retry.operation.target().event_id(), "$desired:test");
            retry.response.send(Ok(())).expect("retry succeeds");
        };
        let (completion, ()) = tokio::join!(manager.read_workers.tasks.next(), responder);
        manager
            .handle_read_worker_completion(completion.expect("retry completion"))
            .await;
        persistence_rx
            .changed()
            .await
            .expect("successful retry publishes outbox removal");
        assert!(
            persistence_rx
                .borrow_and_update()
                .as_ref()
                .expect("persistence request")
                .snapshot()
                .is_empty()
        );
        assert!(matches!(
            control_rx.recv().await,
            Some(TimelineActorControl::ReadStateProjection {
                local_viewed_event_id: Some(local),
                server_confirmed_read_event_id: None,
                sync: TimelineReadStateSync::Synced,
            }) if local == "$desired:test"
        ));
    }

    #[tokio::test]
    async fn restored_fully_read_projects_pending_then_server_confirmed_after_apply() {
        let key = room_key();
        let read_key = ReadStateKey::FullyReadAndPrivateUnthreaded {
            room_id: key.room_id().to_owned(),
        };
        let (actor_handle, mut control_rx) =
            actor_handle_with_positions(7, [("$restored-fully:test", 5)]);
        let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
        let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
        let (persistence, mut persistence_rx) = ReadPersistenceIngress::channel();
        manager.read_workers = ReadWorkerSupervisor::synthetic_restored(
            read_network_tx,
            restored_read_snapshot(read_key.clone(), "$restored-fully:test"),
            persistence,
        );

        manager
            .handle_authoritative_read_state_observed(&key, 7, read_key, None)
            .await;
        assert!(matches!(
            control_rx.recv().await,
            Some(TimelineActorControl::ReadStateProjection {
                local_viewed_event_id: Some(event_id),
                server_confirmed_read_event_id: None,
                sync: TimelineReadStateSync::Pending,
            }) if event_id == "$restored-fully:test"
        ));

        let responder = async {
            let request = read_network_rx.recv().await.expect("restored retry starts");
            request
                .response
                .send(Ok(()))
                .expect("restored retry succeeds");
        };
        let (network_completion, ()) = tokio::join!(manager.read_workers.tasks.next(), responder);
        manager
            .handle_read_worker_completion(network_completion.expect("network completion"))
            .await;

        let acknowledge = async {
            loop {
                match control_rx.recv().await.expect("actor apply control") {
                    TimelineActorControl::ApplyReadSuccess {
                        kind: ReadActorApplyKind::FullyRead,
                        event_id,
                        acknowledged,
                    } => {
                        assert_eq!(event_id, "$restored-fully:test");
                        acknowledged.send(true).expect("acknowledge actor apply");
                        break;
                    }
                    TimelineActorControl::ReadStateProjection { .. } => {}
                    TimelineActorControl::ReadStatePolicyChanged { .. } => {}
                    TimelineActorControl::ReplayInitialItems { .. }
                    | TimelineActorControl::StartLiveTailRefresh { .. }
                    | TimelineActorControl::CancelLiveTailNetwork { .. }
                    | TimelineActorControl::BeginGapRepairDemand
                    | TimelineActorControl::EndGapRepairDemand => {}
                    TimelineActorControl::ApplyReadSuccess { .. } => {
                        panic!("unexpected actor apply kind")
                    }
                }
            }
        };
        let (apply_completion, ()) = tokio::join!(manager.read_workers.tasks.next(), acknowledge);
        manager
            .handle_read_worker_completion(apply_completion.expect("actor apply completion"))
            .await;

        persistence_rx
            .changed()
            .await
            .expect("successful restore publishes empty outbox");
        assert!(
            persistence_rx
                .borrow_and_update()
                .as_ref()
                .expect("persistence request")
                .snapshot()
                .is_empty()
        );
        assert!(matches!(
            control_rx.recv().await,
            Some(TimelineActorControl::ReadStateProjection {
                local_viewed_event_id: Some(local),
                server_confirmed_read_event_id: Some(server),
                sync: TimelineReadStateSync::Synced,
            }) if local == "$restored-fully:test" && server == "$restored-fully:test"
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn reconnect_preserves_a_bounded_reconciliation_wake_for_new_read_waiters() {
        let key = room_key();
        let read_key = ReadStateKey::PublicUnthreaded {
            room_id: key.room_id().to_owned(),
        };
        let (ordinary_tx, _ordinary_rx) = mpsc::channel(1);
        let (control_tx, _control_rx) = mpsc::channel(1);
        let (_position_tx, position_rx) = watch::channel(Arc::new(TimelinePositionIndex {
            generation: u128::from(7_u64) << 64,
            ranks: HashMap::new(),
        }));
        let actor_handle = TimelineActorHandle {
            tx: ordinary_tx,
            control_tx: Some(control_tx),
            position_rx: Some(position_rx),
            task: None,
            auxiliary_tasks: Vec::new(),
            subscription_generation: None,
            enqueue_context: None,
        };
        let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
        let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
        let (persistence, _persistence_rx) = ReadPersistenceIngress::channel();
        manager.read_workers = ReadWorkerSupervisor::synthetic_restored(
            read_network_tx,
            restored_public_read_snapshot(key.room_id(), "$restored:test"),
            persistence,
        );

        manager
            .wake_all_desired_reads(ReadRetrySource::Reconnect)
            .await;
        manager
            .route_read_command(
                fake_rid(29_601),
                key,
                "$new-waiter:test".to_owned(),
                ReadCommandKind::Receipt,
            )
            .await;

        assert!(
            manager
                .read_workers
                .scheduled_retries
                .contains_key(&read_key),
            "reconnect must not cancel the only bounded reconciliation wake"
        );
        tokio::time::advance(Duration::from_secs(1)).await;
        let completion = manager
            .read_workers
            .retry_tasks
            .next()
            .await
            .expect("bounded reconciliation wake");
        manager.handle_read_worker_completion(completion).await;
        let responder = async {
            let request = read_network_rx
                .recv()
                .await
                .expect("new waiter receives a network attempt after the bound");
            assert_eq!(request.operation.target().event_id(), "$new-waiter:test");
            request.response.send(Err(())).expect("settle retry");
        };
        let (completion, ()) = tokio::join!(manager.read_workers.tasks.next(), responder);
        manager
            .handle_read_worker_completion(completion.expect("network completion"))
            .await;
    }

    #[tokio::test]
    async fn invalidating_retry_actively_finishes_the_long_lived_sleeper() {
        let (network_tx, _network_rx) = mpsc::unbounded_channel();
        let mut supervisor = ReadWorkerSupervisor::synthetic_with_retry(
            network_tx,
            Duration::from_secs(30),
            Duration::from_secs(60),
            Duration::from_secs(60),
        );
        let key = ReadStateKey::PublicUnthreaded {
            room_id: "!retry-cancel:example.invalid".to_owned(),
        };
        supervisor.schedule_retry(&key);
        assert_eq!(supervisor.retry_tasks.len(), 1);
        assert_eq!(supervisor.scheduled_retries.len(), 1);

        supervisor.invalidate_retry(&key);

        assert!(supervisor.scheduled_retries.is_empty());
        let completion =
            executor::timeout(Duration::from_millis(25), supervisor.retry_tasks.next())
                .await
                .expect("retry invalidation must wake the sleeper promptly")
                .expect("cancelled retry completion");
        assert!(matches!(
            completion,
            ReadWorkerCompletion::RetryWake {
                key: observed,
                cancelled: true,
                ..
            } if observed == key
        ));
        assert!(
            supervisor.retry_tasks.is_empty(),
            "an invalidated retry must not leave a sixty-second task behind"
        );
    }

    #[tokio::test]
    async fn retry_serial_exhaustion_never_reuses_a_live_stale_token() {
        let (network_tx, _network_rx) = mpsc::unbounded_channel();
        let mut supervisor = ReadWorkerSupervisor::synthetic_with_retry(
            network_tx,
            Duration::from_secs(30),
            Duration::from_secs(60),
            Duration::from_secs(60),
        );
        let key = ReadStateKey::PublicUnthreaded {
            room_id: "!retry-token-exhaustion:example.invalid".to_owned(),
        };

        supervisor.retry_serial = u64::MAX;
        supervisor.schedule_retry(&key);
        let stale_generation = supervisor
            .scheduled_retries
            .get(&key)
            .map(|(generation, _)| generation.clone())
            .expect("stale retry token");
        supervisor.invalidate_retry(&key);

        // Model the manager-wide serial reaching exhaustion again while the
        // cancelled wake remains queued in `retry_tasks`.
        supervisor.retry_serial = u64::MAX;
        supervisor.schedule_retry(&key);
        let current_generation = supervisor
            .scheduled_retries
            .get(&key)
            .map(|(generation, _)| generation.clone())
            .expect("current retry token");

        let stale = executor::timeout(Duration::from_millis(25), supervisor.retry_tasks.next())
            .await
            .expect("cancelled stale wake must be ready")
            .expect("cancelled stale retry completion");
        assert!(matches!(
            stale,
            ReadWorkerCompletion::RetryWake {
                key: observed,
                generation: observed_generation,
                cancelled: true,
            } if observed == key && observed_generation == stale_generation
        ));
        assert!(
            !supervisor.accept_retry_wake(&key, stale_generation),
            "an exhausted stale token must not settle the current retry"
        );
        assert!(
            supervisor
                .scheduled_retries
                .get(&key)
                .is_some_and(|(generation, _)| generation == &current_generation),
            "the current retry must remain scheduled after the stale wake"
        );
    }

    #[tokio::test]
    async fn completed_retry_keys_do_not_accumulate_generation_bookkeeping() {
        let (network_tx, _network_rx) = mpsc::unbounded_channel();
        let mut supervisor = ReadWorkerSupervisor::synthetic_with_retry(
            network_tx,
            Duration::from_secs(30),
            Duration::from_secs(60),
            Duration::from_secs(60),
        );

        for index in 0..256 {
            let key = ReadStateKey::PublicUnthreaded {
                room_id: format!("!completed-retry-{index}:example.invalid"),
            };
            supervisor.schedule_retry(&key);
            let generation = supervisor
                .scheduled_retries
                .get(&key)
                .map(|(generation, _)| generation.clone())
                .expect("retry generation");

            supervisor.reset_retry(&key);
            let cancelled =
                executor::timeout(Duration::from_millis(25), supervisor.retry_tasks.next())
                    .await
                    .expect("retry cancellation must be bounded")
                    .expect("cancelled retry completion");
            assert!(matches!(
                cancelled,
                ReadWorkerCompletion::RetryWake {
                    key: observed,
                    generation: observed_generation,
                    cancelled: true,
                } if observed == key && observed_generation == generation
            ));
            assert!(
                !supervisor.accept_retry_wake(&key, generation),
                "a cancelled sleeper must remain stale after its key retires"
            );
        }

        assert_eq!(
            supervisor.retry_bookkeeping_key_count(),
            0,
            "completed historical keys must not remain in retry bookkeeping"
        );
    }

    #[tokio::test]
    async fn authoritative_server_ahead_clears_restored_read_without_network_retry() {
        let key = room_key();
        let read_key = ReadStateKey::PublicUnthreaded {
            room_id: key.room_id().to_owned(),
        };
        let (ordinary_tx, _ordinary_rx) = mpsc::channel(1);
        let (control_tx, _control_rx) = mpsc::channel(1);
        let (_position_tx, position_rx) = watch::channel(Arc::new(TimelinePositionIndex {
            generation: u128::from(7_u64) << 64,
            ranks: HashMap::from([
                ("$desired:test".to_owned(), 5),
                ("$server-ahead:test".to_owned(), 6),
            ]),
        }));
        let actor_handle = TimelineActorHandle {
            tx: ordinary_tx,
            control_tx: Some(control_tx),
            position_rx: Some(position_rx),
            task: None,
            auxiliary_tasks: Vec::new(),
            subscription_generation: None,
            enqueue_context: None,
        };
        let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
        let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
        let (persistence, mut persistence_rx) = ReadPersistenceIngress::channel();
        manager.read_workers = ReadWorkerSupervisor::synthetic_restored(
            read_network_tx,
            restored_public_read_snapshot(key.room_id(), "$desired:test"),
            persistence,
        );

        manager
            .handle_authoritative_read_state_observed(
                &key,
                7,
                read_key,
                Some("$server-ahead:test".to_owned()),
            )
            .await;

        assert!(read_network_rx.try_recv().is_err());
        assert!(manager.read_workers.tasks.is_empty());
        persistence_rx
            .changed()
            .await
            .expect("server-ahead reconciliation publishes removal");
        assert!(
            persistence_rx
                .borrow_and_update()
                .as_ref()
                .expect("persistence request")
                .snapshot()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn authoritative_reconciliation_keeps_unordered_remaining_candidate_pending() {
        let key = room_key();
        let read_key = ReadStateKey::PublicUnthreaded {
            room_id: key.room_id().to_owned(),
        };
        let mut restored = ReadStateEngine::new(7);
        restored.admit(
            7,
            read_key.clone(),
            ReadTarget::new("$positioned:test".to_owned()),
            ReadWaiterId::new(1),
        );
        restored.admit(
            7,
            read_key.clone(),
            ReadTarget::new("$outside-window:test".to_owned()),
            ReadWaiterId::new(2),
        );
        let (ordinary_tx, _ordinary_rx) = mpsc::channel(1);
        let (control_tx, mut control_rx) = mpsc::channel(1);
        let (_position_tx, position_rx) = watch::channel(Arc::new(TimelinePositionIndex {
            generation: u128::from(7_u64) << 64,
            ranks: HashMap::from([
                ("$positioned:test".to_owned(), 5),
                ("$server-ahead:test".to_owned(), 6),
            ]),
        }));
        let actor_handle = TimelineActorHandle {
            tx: ordinary_tx,
            control_tx: Some(control_tx),
            position_rx: Some(position_rx),
            task: None,
            auxiliary_tasks: Vec::new(),
            subscription_generation: None,
            enqueue_context: None,
        };
        let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
        let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
        let (persistence, _persistence_rx) = ReadPersistenceIngress::channel();
        manager.read_workers = ReadWorkerSupervisor::synthetic_restored(
            read_network_tx,
            restored.persistence_snapshot(),
            persistence,
        );

        manager
            .handle_authoritative_read_state_observed(
                &key,
                7,
                read_key.clone(),
                Some("$server-ahead:test".to_owned()),
            )
            .await;

        assert_eq!(manager.read_workers.state.candidate_count(&read_key), 1);
        assert!(manager.read_workers.reconciliation_pending(&read_key));
        assert!(manager.read_workers.tasks.is_empty());
        assert!(read_network_rx.try_recv().is_err());
        assert!(matches!(
            control_rx.recv().await,
            Some(TimelineActorControl::ReadStateProjection {
                local_viewed_event_id: None,
                sync: TimelineReadStateSync::Pending,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn stalled_read_receipt_worker_does_not_block_cached_subscription_replay() {
        let key = room_key();
        let read_request_id = fake_rid(28_480);
        let subscribe_request_id = fake_rid(28_481);
        let (ordinary_tx, _ordinary_rx) = mpsc::channel(1);
        let (control_tx, mut control_rx) = mpsc::channel(2);
        let (_position_tx, position_rx) = watch::channel(Arc::new(TimelinePositionIndex {
            generation: 11,
            ranks: HashMap::from([("$read-target:test".to_owned(), 7)]),
        }));
        let actor_handle = TimelineActorHandle {
            tx: ordinary_tx,
            control_tx: Some(control_tx),
            position_rx: Some(position_rx),
            task: None,
            auxiliary_tasks: Vec::new(),
            subscription_generation: None,
            enqueue_context: None,
        };
        let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
        let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
        manager.read_workers =
            ReadWorkerSupervisor::synthetic(read_network_tx, Duration::from_secs(30));
        let (manager_tx, manager_rx) = mpsc::channel(4);
        manager.msg_tx = manager_tx.clone();
        manager.msg_rx = manager_rx;
        let run = executor::spawn(manager.run());

        manager_tx
            .send(TimelineMessage::Command(TimelineCommand::SendReadReceipt {
                request_id: read_request_id,
                key: key.clone(),
                event_id: "$read-target:test".to_owned(),
            }))
            .await
            .expect("admit read command");
        let stalled = executor::timeout(Duration::from_millis(100), read_network_rx.recv())
            .await
            .expect("read worker must start")
            .expect("synthetic read request");

        manager_tx
            .send(TimelineMessage::Command(TimelineCommand::Subscribe {
                request_id: subscribe_request_id,
                key,
            }))
            .await
            .expect("queue cached subscribe");

        assert!(matches!(
            executor::timeout(Duration::from_millis(100), control_rx.recv())
                .await
                .expect("cached replay must not wait for read network"),
            Some(TimelineActorControl::ReplayInitialItems { cause_request_id })
                if cause_request_id == subscribe_request_id
        ));

        drop(stalled);
        let (acknowledged, acknowledgement) = oneshot::channel();
        manager_tx
            .send(TimelineMessage::Shutdown {
                acknowledged: Some(acknowledged),
            })
            .await
            .expect("shutdown manager");
        acknowledgement.await.expect("shutdown acknowledgement");
        run.await.expect("manager task");
    }

    #[tokio::test]
    async fn newer_positioned_read_target_cancels_stale_worker_and_settles_both_waiters_once() {
        let key = room_key();
        let older_request_id = fake_rid(28_482);
        let newer_request_id = fake_rid(28_483);
        let (ordinary_tx, _ordinary_rx) = mpsc::channel(1);
        let (control_tx, _control_rx) = mpsc::channel(2);
        let (_position_tx, position_rx) = watch::channel(Arc::new(TimelinePositionIndex {
            generation: 12,
            ranks: HashMap::from([
                ("$read-old:test".to_owned(), 7),
                ("$read-new:test".to_owned(), 8),
            ]),
        }));
        let actor_handle = TimelineActorHandle {
            tx: ordinary_tx,
            control_tx: Some(control_tx),
            position_rx: Some(position_rx),
            task: None,
            auxiliary_tasks: Vec::new(),
            subscription_generation: None,
            enqueue_context: None,
        };
        let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
        let (event_tx, mut event_rx) = broadcast::channel(8);
        manager.event_tx = event_tx;
        let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
        manager.read_workers =
            ReadWorkerSupervisor::synthetic(read_network_tx, Duration::from_secs(30));
        let (manager_tx, manager_rx) = mpsc::channel(4);
        manager.msg_tx = manager_tx.clone();
        manager.msg_rx = manager_rx;
        let run = executor::spawn(manager.run());

        for (request_id, event_id) in [
            (older_request_id, "$read-old:test"),
            (newer_request_id, "$read-new:test"),
        ] {
            manager_tx
                .send(TimelineMessage::Command(TimelineCommand::SendReadReceipt {
                    request_id,
                    key: key.clone(),
                    event_id: event_id.to_owned(),
                }))
                .await
                .expect("admit read command");
            if request_id == older_request_id {
                break;
            }
        }
        let older = executor::timeout(Duration::from_millis(100), read_network_rx.recv())
            .await
            .expect("older read worker must start")
            .expect("older synthetic read request");
        assert_eq!(older.operation.target().event_id(), "$read-old:test");

        manager_tx
            .send(TimelineMessage::Command(TimelineCommand::SendReadReceipt {
                request_id: newer_request_id,
                key: key.clone(),
                event_id: "$read-new:test".to_owned(),
            }))
            .await
            .expect("admit newer read command");
        let newer = executor::timeout(Duration::from_millis(100), read_network_rx.recv())
            .await
            .expect("newer read worker must start")
            .expect("newer synthetic read request");
        assert_eq!(newer.operation.target().event_id(), "$read-new:test");
        assert!(
            older.response.send(Ok(())).is_err(),
            "dominated worker must be cancelled before its late success"
        );
        newer.response.send(Ok(())).expect("complete newer target");

        let mut settled = HashSet::new();
        while settled.len() < 2 {
            let event = executor::timeout(Duration::from_millis(100), event_rx.recv())
                .await
                .expect("both waiters must settle")
                .expect("event stream");
            if let CoreEvent::LiveSignals(LiveSignalsEvent::ReadReceiptSent {
                request_id, ..
            }) = event
            {
                assert!(settled.insert(request_id), "duplicate waiter success");
            }
        }
        assert_eq!(settled, HashSet::from([older_request_id, newer_request_id]));
        assert!(
            executor::timeout(Duration::from_millis(25), event_rx.recv())
                .await
                .is_err(),
            "stale completion must not emit a second terminal"
        );

        let (acknowledged, acknowledgement) = oneshot::channel();
        manager_tx
            .send(TimelineMessage::Shutdown {
                acknowledged: Some(acknowledged),
            })
            .await
            .expect("shutdown manager");
        acknowledgement.await.expect("shutdown acknowledgement");
        run.await.expect("manager task");
    }

    #[tokio::test]
    async fn coalesced_read_timeout_fails_each_waiter_once_without_retry_storm() {
        let key = room_key();
        let request_ids = [fake_rid(28_484), fake_rid(28_485)];
        let (ordinary_tx, _ordinary_rx) = mpsc::channel(1);
        let (control_tx, _control_rx) = mpsc::channel(1);
        let actor_handle = TimelineActorHandle {
            tx: ordinary_tx,
            control_tx: Some(control_tx),
            position_rx: None,
            task: None,
            auxiliary_tasks: Vec::new(),
            subscription_generation: None,
            enqueue_context: None,
        };
        let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
        let (event_tx, mut event_rx) = broadcast::channel(8);
        manager.event_tx = event_tx;
        let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
        manager.read_workers =
            ReadWorkerSupervisor::synthetic(read_network_tx, Duration::from_millis(20));
        let (manager_tx, manager_rx) = mpsc::channel(4);
        manager.msg_tx = manager_tx.clone();
        manager.msg_rx = manager_rx;
        let run = executor::spawn(manager.run());

        for request_id in request_ids {
            manager_tx
                .send(TimelineMessage::Command(TimelineCommand::SendReadReceipt {
                    request_id,
                    key: key.clone(),
                    event_id: "$same-target:test".to_owned(),
                }))
                .await
                .expect("admit coalesced read");
        }
        let stalled = executor::timeout(Duration::from_millis(100), read_network_rx.recv())
            .await
            .expect("one network worker must start")
            .expect("synthetic read request");

        let mut failed = HashSet::new();
        while failed.len() < 2 {
            let event = executor::timeout(Duration::from_millis(100), event_rx.recv())
                .await
                .expect("timeout must settle both waiters")
                .expect("event stream");
            if let CoreEvent::OperationFailed {
                request_id,
                failure:
                    CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::Timeout,
                    },
            } = event
            {
                assert!(failed.insert(request_id), "duplicate waiter timeout");
            }
        }
        assert_eq!(failed, HashSet::from(request_ids));
        assert!(
            executor::timeout(Duration::from_millis(40), read_network_rx.recv())
                .await
                .is_err(),
            "timeout retains desired state but must not spin an immediate retry"
        );
        assert!(
            executor::timeout(Duration::from_millis(20), event_rx.recv())
                .await
                .is_err(),
            "each waiter receives exactly one timeout"
        );

        drop(stalled);
        let (acknowledged, acknowledgement) = oneshot::channel();
        manager_tx
            .send(TimelineMessage::Shutdown {
                acknowledged: Some(acknowledged),
            })
            .await
            .expect("shutdown manager");
        acknowledgement.await.expect("shutdown acknowledgement");
        run.await.expect("manager task");
    }

    #[tokio::test]
    async fn fully_read_success_waits_for_actor_control_ack_before_terminal_event() {
        let key = room_key();
        let request_id = fake_rid(28_486);
        let (ordinary_tx, _ordinary_rx) = mpsc::channel(1);
        let (control_tx, mut control_rx) = mpsc::channel(1);
        let actor_handle = TimelineActorHandle {
            tx: ordinary_tx,
            control_tx: Some(control_tx),
            position_rx: None,
            task: None,
            auxiliary_tasks: Vec::new(),
            subscription_generation: None,
            enqueue_context: None,
        };
        let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
        let (action_tx, mut action_rx) = mpsc::channel(4);
        let (event_tx, mut event_rx) = broadcast::channel(4);
        manager.action_tx = action_tx;
        manager.event_tx = event_tx;
        let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
        manager.read_workers =
            ReadWorkerSupervisor::synthetic(read_network_tx, Duration::from_secs(30));
        let (manager_tx, manager_rx) = mpsc::channel(4);
        manager.msg_tx = manager_tx.clone();
        manager.msg_rx = manager_rx;
        let run = executor::spawn(manager.run());

        manager_tx
            .send(TimelineMessage::Command(TimelineCommand::SetFullyRead {
                request_id,
                key: key.clone(),
                event_id: "$fully-read:test".to_owned(),
            }))
            .await
            .expect("admit fully-read command");
        let network = executor::timeout(Duration::from_millis(100), read_network_rx.recv())
            .await
            .expect("fully-read worker must start")
            .expect("synthetic read request");
        network.response.send(Ok(())).expect("SDK success");
        let control = executor::timeout(Duration::from_millis(100), control_rx.recv())
            .await
            .expect("success must enter actor control lane")
            .expect("actor apply control");
        assert!(
            event_rx.try_recv().is_err(),
            "success must wait for actor ACK"
        );
        let TimelineActorControl::ApplyReadSuccess {
            kind: ReadActorApplyKind::FullyRead,
            event_id,
            acknowledged,
        } = control
        else {
            panic!("expected fully-read actor control");
        };
        assert_eq!(event_id, "$fully-read:test");
        acknowledged.send(true).expect("ack actor state update");

        assert!(matches!(
            executor::timeout(Duration::from_millis(100), action_rx.recv())
                .await
                .expect("reducer action after ACK"),
            Some(actions)
                if matches!(actions.as_slice(), [AppAction::RoomMarkedAsReadSucceeded { request_id: sequence, .. }] if *sequence == request_id.sequence)
        ));
        assert!(matches!(
            executor::timeout(Duration::from_millis(100), event_rx.recv())
                .await
                .expect("success after ACK")
                .expect("event stream"),
            CoreEvent::LiveSignals(LiveSignalsEvent::FullyReadSet {
                request_id: settled,
                ..
            }) if settled == request_id
        ));

        let (acknowledged, acknowledgement) = oneshot::channel();
        manager_tx
            .send(TimelineMessage::Shutdown {
                acknowledged: Some(acknowledged),
            })
            .await
            .expect("shutdown manager");
        acknowledgement.await.expect("shutdown acknowledgement");
        run.await.expect("manager task");
    }

    #[tokio::test]
    async fn fully_read_success_after_actor_removal_fails_without_success_terminal() {
        let key = room_key();
        let request_id = fake_rid(28_487);
        let (ordinary_tx, _ordinary_rx) = mpsc::channel(1);
        let (control_tx, mut control_rx) = mpsc::channel(1);
        let actor_handle = TimelineActorHandle {
            tx: ordinary_tx,
            control_tx: Some(control_tx),
            position_rx: None,
            task: None,
            auxiliary_tasks: Vec::new(),
            subscription_generation: None,
            enqueue_context: None,
        };
        let mut manager = live_tail_test_manager(HashMap::from([(key.clone(), actor_handle)]));
        let (action_tx, _action_rx) = mpsc::channel(4);
        let (event_tx, mut event_rx) = broadcast::channel(4);
        manager.action_tx = action_tx;
        manager.event_tx = event_tx;
        let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
        manager.read_workers =
            ReadWorkerSupervisor::synthetic(read_network_tx, Duration::from_secs(30));
        let (manager_tx, manager_rx) = mpsc::channel(4);
        manager.msg_tx = manager_tx.clone();
        manager.msg_rx = manager_rx;
        let run = executor::spawn(manager.run());

        manager_tx
            .send(TimelineMessage::Command(TimelineCommand::SetFullyRead {
                request_id,
                key: key.clone(),
                event_id: "$fully-read:test".to_owned(),
            }))
            .await
            .expect("admit fully-read command");
        let network = executor::timeout(Duration::from_millis(100), read_network_rx.recv())
            .await
            .expect("fully-read worker must start")
            .expect("synthetic read request");
        manager_tx
            .send(TimelineMessage::Command(TimelineCommand::Unsubscribe {
                request_id: fake_rid(28_488),
                key: key.clone(),
            }))
            .await
            .expect("remove actor");
        assert!(
            executor::timeout(Duration::from_millis(100), control_rx.recv())
                .await
                .expect("actor control sender must close")
                .is_none()
        );
        network
            .response
            .send(Ok(()))
            .expect("late SDK success after actor removal");

        assert!(matches!(
            executor::timeout(Duration::from_millis(100), event_rx.recv())
                .await
                .expect("missing actor must fail waiter")
                .expect("event stream"),
            CoreEvent::OperationFailed {
                request_id: failed,
                failure: CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::Sdk,
                },
            } if failed == request_id
        ));
        assert!(
            executor::timeout(Duration::from_millis(20), event_rx.recv())
                .await
                .is_err(),
            "late network success must not emit a success terminal"
        );

        let (acknowledged, acknowledgement) = oneshot::channel();
        manager_tx
            .send(TimelineMessage::Shutdown {
                acknowledged: Some(acknowledged),
            })
            .await
            .expect("shutdown manager");
        acknowledgement.await.expect("shutdown acknowledgement");
        run.await.expect("manager task");
    }

    #[tokio::test]
    async fn read_admission_rejects_missing_session_actor_and_invalid_ids_immediately() {
        let key = room_key();
        let (event_tx, mut event_rx) = broadcast::channel(8);
        let mut manager =
            live_tail_test_manager(HashMap::from([(key.clone(), test_timeline_actor_handle())]));
        manager.event_tx = event_tx;

        manager
            .handle_command(TimelineCommand::SendReadReceipt {
                request_id: fake_rid(28_489),
                key: key.clone(),
                event_id: "$event:test".to_owned(),
            })
            .await;
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::OperationFailed {
                failure: CoreFailure::SessionRequired,
                ..
            })
        ));

        let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
        manager.read_workers =
            ReadWorkerSupervisor::synthetic(read_network_tx, Duration::from_secs(30));
        manager.timelines.clear();
        manager
            .handle_command(TimelineCommand::SendReadReceipt {
                request_id: fake_rid(28_490),
                key: key.clone(),
                event_id: "$event:test".to_owned(),
            })
            .await;
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::OperationFailed {
                failure: CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::NotSubscribed,
                },
                ..
            })
        ));

        manager
            .timelines
            .insert(key.clone(), test_timeline_actor_handle());
        manager.read_workers.send_read_receipts = false;
        manager
            .handle_command(TimelineCommand::SendReadReceipt {
                request_id: fake_rid(28_491),
                key: key.clone(),
                event_id: "$event:test".to_owned(),
            })
            .await;
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::OperationFailed {
                failure: CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::Forbidden,
                },
                ..
            })
        ));
        assert!(manager.read_workers.waiters.is_empty());
        assert!(manager.read_workers.tasks.is_empty());

        manager.read_workers.send_read_receipts = true;
        let flip_request_id = fake_rid(28_492);
        manager
            .handle_command(TimelineCommand::SendReadReceipt {
                request_id: flip_request_id,
                key: key.clone(),
                event_id: "$event:test".to_owned(),
            })
            .await;
        assert_eq!(manager.read_workers.waiters.len(), 1);
        manager.handle_read_state_policy_changed(1, false).await;
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::OperationFailed {
                request_id,
                failure: CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::Forbidden,
                },
            }) if request_id == flip_request_id
        ));
        assert!(manager.read_workers.waiters.is_empty());
        assert!(manager.read_workers.state.persistence_snapshot().is_empty());
        let cancelled = manager
            .read_workers
            .tasks
            .next()
            .await
            .expect("policy flip cancels the admitted worker");
        manager.handle_read_worker_completion(cancelled).await;
        assert!(event_rx.try_recv().is_err());
        assert!(read_network_rx.try_recv().is_err());

        manager
            .handle_command(TimelineCommand::SetFullyRead {
                request_id: fake_rid(28_493),
                key,
                event_id: "not-an-event-id".to_owned(),
            })
            .await;
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::OperationFailed {
                failure: CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::Sdk,
                },
                ..
            })
        ));
        assert!(manager.read_workers.tasks.is_empty());
        assert!(read_network_rx.try_recv().is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn failed_read_network_settles_waiter_once_then_retries_after_capped_backoff() {
        let key = room_key();
        let request_id = fake_rid(28_492);
        let (event_tx, mut event_rx) = broadcast::channel(4);
        let mut manager =
            live_tail_test_manager(HashMap::from([(key.clone(), test_timeline_actor_handle())]));
        manager.event_tx = event_tx;
        let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
        manager.read_workers = ReadWorkerSupervisor::synthetic_with_retry(
            read_network_tx,
            Duration::from_secs(30),
            Duration::from_secs(1),
            Duration::from_secs(4),
        );

        manager
            .handle_command(TimelineCommand::SendReadReceipt {
                request_id,
                key: key.clone(),
                event_id: "$event:test".to_owned(),
            })
            .await;
        let responder = async {
            let request = read_network_rx.recv().await.expect("read request");
            request
                .response
                .send(Err(()))
                .expect("fail network request");
        };
        let (completion, ()) = tokio::join!(manager.read_workers.tasks.next(), responder);
        manager
            .handle_read_worker_completion(completion.expect("worker completion"))
            .await;

        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::OperationFailed {
                request_id: failed,
                failure: CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::Sdk,
                },
            }) if failed == request_id
        ));
        assert!(event_rx.try_recv().is_err());
        assert!(read_network_rx.try_recv().is_err());

        assert!(
            manager
                .read_workers
                .retry_tasks
                .next()
                .now_or_never()
                .is_none(),
            "scheduled retry must begin pending"
        );
        tokio::time::advance(Duration::from_millis(999)).await;
        assert!(
            manager
                .read_workers
                .retry_tasks
                .next()
                .now_or_never()
                .is_none(),
            "retry must not run before the backoff deadline"
        );
        tokio::time::advance(Duration::from_millis(1)).await;
        let retry_wake = manager
            .read_workers
            .retry_tasks
            .next()
            .await
            .expect("backoff wake");
        manager.handle_read_worker_completion(retry_wake).await;
        let responder = async {
            let retried = read_network_rx.recv().await.expect("retry network request");
            assert_eq!(retried.operation.target().event_id(), "$event:test");
            retried.response.send(Ok(())).expect("retry succeeds");
        };
        let (completion, ()) = tokio::join!(manager.read_workers.tasks.next(), responder);
        manager
            .handle_read_worker_completion(completion.expect("retry completion"))
            .await;
        assert!(
            event_rx.try_recv().is_err(),
            "background retry must not emit a second user terminal"
        );
        assert!(!manager.read_workers.state.has_candidate(
            &ReadStateKey::PublicUnthreaded {
                room_id: key.room_id().to_owned(),
            },
            "$event:test",
        ));
    }

    #[test]
    fn read_retry_delay_is_exponential_and_capped() {
        assert_eq!(
            read_retry_delay_for_attempt(Duration::from_secs(1), Duration::from_secs(4), 0,),
            Duration::from_secs(1)
        );
        assert_eq!(
            read_retry_delay_for_attempt(Duration::from_secs(1), Duration::from_secs(4), 1,),
            Duration::from_secs(2)
        );
        assert_eq!(
            read_retry_delay_for_attempt(Duration::from_secs(1), Duration::from_secs(4), 64,),
            Duration::from_secs(4)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn sync_restart_preserves_failed_read_backoff_until_its_due_token() {
        let key = room_key();
        let request_id = fake_rid(28_493);
        let (event_tx, mut event_rx) = broadcast::channel(4);
        let mut manager =
            live_tail_test_manager(HashMap::from([(key.clone(), test_timeline_actor_handle())]));
        manager.event_tx = event_tx;
        let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
        manager.read_workers = ReadWorkerSupervisor::synthetic_with_retry(
            read_network_tx,
            Duration::from_secs(30),
            Duration::from_secs(30),
            Duration::from_secs(60),
        );

        manager
            .handle_command(TimelineCommand::SendReadReceipt {
                request_id,
                key,
                event_id: "$event:test".to_owned(),
            })
            .await;
        let responder = async {
            let first = read_network_rx.recv().await.expect("initial request");
            first.response.send(Err(())).expect("fail initial request");
        };
        let (completion, ()) = tokio::join!(manager.read_workers.tasks.next(), responder);
        manager
            .handle_read_worker_completion(completion.expect("initial completion"))
            .await;
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::OperationFailed {
                request_id: failed,
                ..
            }) if failed == request_id
        ));

        manager
            .wake_all_desired_reads(ReadRetrySource::Reconnect)
            .await;
        assert!(
            read_network_rx.try_recv().is_err(),
            "reconnect must not bypass the scheduled backoff"
        );
        tokio::time::advance(Duration::from_secs(30)).await;
        let retry_wake = manager
            .read_workers
            .retry_tasks
            .next()
            .await
            .expect("exact due token");
        manager.handle_read_worker_completion(retry_wake).await;
        let responder = async {
            let retry = read_network_rx.recv().await.expect("due retry");
            retry.response.send(Ok(())).expect("retry succeeds");
        };
        let (completion, ()) = tokio::join!(manager.read_workers.tasks.next(), responder);
        manager
            .handle_read_worker_completion(completion.expect("retry completion"))
            .await;
        tokio::time::advance(Duration::from_secs(60)).await;
        assert!(
            event_rx.try_recv().is_err(),
            "restart retry must not emit a second user terminal"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn room_subscription_checkpoint_preserves_failed_read_backoff() {
        let key = room_key();
        let request_id = fake_rid(28_494);
        let (event_tx, mut event_rx) = broadcast::channel(4);
        let mut manager =
            live_tail_test_manager(HashMap::from([(key.clone(), test_timeline_actor_handle())]));
        manager.event_tx = event_tx;
        manager.room_subscription_service_epoch = 9;
        let (read_network_tx, mut read_network_rx) = mpsc::unbounded_channel();
        manager.read_workers = ReadWorkerSupervisor::synthetic_with_retry(
            read_network_tx,
            Duration::from_secs(30),
            Duration::from_secs(30),
            Duration::from_secs(60),
        );

        manager
            .handle_command(TimelineCommand::SendReadReceipt {
                request_id,
                key: key.clone(),
                event_id: "$event:test".to_owned(),
            })
            .await;
        let responder = async {
            let first = read_network_rx.recv().await.expect("initial request");
            first.response.send(Err(())).expect("fail initial request");
        };
        let (completion, ()) = tokio::join!(manager.read_workers.tasks.next(), responder);
        manager
            .handle_read_worker_completion(completion.expect("initial completion"))
            .await;
        assert!(event_rx.try_recv().is_ok());

        manager
            .wake_desired_reads_for_room(key.room_id(), ReadRetrySource::Checkpoint)
            .await;
        assert!(
            read_network_rx.try_recv().is_err(),
            "checkpoint must not bypass the scheduled backoff"
        );
        tokio::time::advance(Duration::from_secs(30)).await;
        let retry_wake = manager
            .read_workers
            .retry_tasks
            .next()
            .await
            .expect("exact checkpoint retry token");
        manager.handle_read_worker_completion(retry_wake).await;
        let responder = async {
            let retry = read_network_rx.recv().await.expect("due checkpoint retry");
            retry
                .response
                .send(Ok(()))
                .expect("checkpoint retry succeeds");
        };
        let (completion, ()) = tokio::join!(manager.read_workers.tasks.next(), responder);
        manager
            .handle_read_worker_completion(completion.expect("checkpoint retry completion"))
            .await;
        assert!(
            event_rx.try_recv().is_err(),
            "checkpoint retry must not emit a second user terminal"
        );
    }

    #[test]
    fn manager_read_completion_lane_precedes_ordinary_mailbox() {
        let manager_run = item_body(include_str!("manager.rs"), "async fn run(mut self)");
        let read_completion = manager_run
            .find("completion = self.read_workers.tasks.next()")
            .expect("manager read completion lane");
        let ordinary_mailbox = manager_run
            .find("msg = self.msg_rx.recv()")
            .expect("manager ordinary mailbox");
        assert!(
            read_completion < ordinary_mailbox,
            "biased manager select must poll read completions before ordinary commands"
        );
    }

    #[test]
    fn replaying_thread_initial_items_preserves_semantic_attention_tracker() {
        let replay_helper = item_body(
            include_str!("navigation.rs"),
            "fn handle_replay_initial_items",
        );
        assert!(
            replay_helper.contains("ThreadAttentionObservation::Replay")
                && !replay_helper.contains("ThreadAttentionTracker::default()"),
            "thread replay must absorb history without resetting stable-ID deduplication or unread attention"
        );
    }

    #[test]
    fn timeline_builder_does_not_track_state_event_read_receipts() {
        let source = include_str!("relay.rs");
        let production = source.split("\nmod tests").next().unwrap_or(source);
        let builder_source = production
            .split("fn koushi_timeline_builder")
            .nth(1)
            .expect("timeline builder should exist")
            .split("struct PreparedRelayRecovery")
            .next()
            .expect("relay recovery structs should follow timeline builder");

        assert!(
            builder_source.contains("TimelineReadReceiptTracking::MessageLikeEvents"),
            "timeline read receipts should only track message-like events; state-event tracking exercises SDK event-cache ordering paths that are not needed by Koushi rows"
        );
        assert!(
            !builder_source.contains("TimelineReadReceiptTracking::AllEvents"),
            "do not restore AllEvents for the product timeline builder"
        );
    }

    #[tokio::test]
    async fn koushi_timeline_builder_projects_sdk_read_receipts() {
        use matrix_sdk::assert_next_with_timeout;
        use matrix_sdk::ruma::{event_id, room_id, user_id};
        use matrix_sdk::test_utils::mocks::MatrixMockServer;
        use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};

        let server = MatrixMockServer::new().await;
        let client = server.client_builder().build().await;
        let room_id = room_id!("!receipts:example.test");
        let room = server.sync_joined_room(&client, room_id).await;
        let timeline = koushi_timeline_builder(
            &room,
            TimelineFocus::Live {
                hide_threaded_events: false,
            },
        )
        .build()
        .await
        .expect("timeline");
        let (_initial_items, mut stream) = timeline.subscribe().await;

        let factory = EventFactory::new().room(room_id);
        server
            .sync_room(
                &client,
                JoinedRoomBuilder::new(room_id)
                    .add_timeline_event(
                        factory
                            .text_msg("first")
                            .event_id(event_id!("$first:example.test"))
                            .sender(user_id!("@alice:example.test"))
                            .into_raw_sync(),
                    )
                    .add_timeline_event(
                        factory
                            .text_msg("second")
                            .event_id(event_id!("$second:example.test"))
                            .sender(user_id!("@bob:example.test"))
                            .into_raw_sync(),
                    ),
            )
            .await;

        let diffs = assert_next_with_timeout!(stream);
        let mut receipts_by_event = Vec::new();
        for diff in &diffs {
            collect_live_event_receipts_from_diff(diff, &mut receipts_by_event);
        }

        let second = receipts_by_event
            .iter()
            .find(|entry| entry.event_id == "$second:example.test")
            .expect("Koushi timeline builder must opt in to SDK read receipt tracking");
        assert!(
            second
                .receipts
                .iter()
                .any(|receipt| receipt.user_id == "@bob:example.test")
        );
    }

    #[test]
    fn live_receipt_observation_action_builder_is_pure_and_orders_profiles_first() {
        let actions = build_live_receipt_observation_actions(
            "!room:example.test",
            vec![LiveEventReceipts {
                event_id: "$event:example.test".to_owned(),
                receipts: vec![LiveReadReceipt {
                    user_id: "@bob:example.test".to_owned(),
                    display_name: None,
                    original_display_label: String::new(),
                    avatar: None,
                    timestamp_ms: Some(1),
                }],
            }],
            vec![MatrixUserProfile {
                user_id: "@bob:example.test".to_owned(),
                display_name: Some("Bob".to_owned()),
                avatar_mxc_uri: None,
            }],
        );

        assert!(matches!(
            actions.as_slice(),
            [
                AppAction::LiveRoomProfilesObserved { profiles, .. },
                AppAction::UserProfilesUpdated { profiles: cached },
                AppAction::LiveRoomReceiptsUpdated { .. },
            ] if profiles[0].display_label == "Bob"
                && cached[0].display_label == "Bob"
        ));
    }

    #[tokio::test]
    async fn local_receipt_observation_helper_builds_profile_then_receipt_actions() {
        use koushi_state::{AppState, SessionInfo, SessionState, reduce};
        use matrix_sdk::assert_next_with_timeout;
        use matrix_sdk::ruma::{event_id, room_id, user_id};
        use matrix_sdk::test_utils::mocks::MatrixMockServer;
        use matrix_sdk_test::{ALICE, JoinedRoomBuilder, event_factory::EventFactory};

        let server = MatrixMockServer::new().await;
        let client = server.client_builder().build().await;
        let room_id = room_id!("!receipt-profiles:example.test");
        let bob = user_id!("@bob:example.test");
        let room = server.sync_joined_room(&client, room_id).await;
        server
            .sync_room(
                &client,
                JoinedRoomBuilder::new(room_id).add_state_event(
                    EventFactory::new()
                        .room(room_id)
                        .member(bob)
                        .display_name("Relevant room member")
                        .into_raw_sync_state(),
                ),
            )
            .await;

        let timeline = koushi_timeline_builder(
            &room,
            TimelineFocus::Live {
                hide_threaded_events: false,
            },
        )
        .build()
        .await
        .expect("timeline");
        let (_initial_items, mut stream) = timeline.subscribe().await;
        let factory = EventFactory::new().room(room_id);
        server
            .sync_room(
                &client,
                JoinedRoomBuilder::new(room_id)
                    .add_timeline_event(
                        factory
                            .text_msg("receipt source")
                            .event_id(event_id!("$receipt-source:example.test"))
                            .sender(bob)
                            .into_raw_sync(),
                    )
                    .add_timeline_event(
                        factory
                            .text_msg("second receipt source")
                            .event_id(event_id!("$receipt-source-two:example.test"))
                            .sender(bob)
                            .into_raw_sync(),
                    ),
            )
            .await;

        let diffs = assert_next_with_timeout!(stream);
        let mut receipts_by_event = Vec::new();
        for diff in &diffs {
            collect_live_event_receipts_from_diff(diff, &mut receipts_by_event);
        }
        let observed_receipts = receipts_by_event
            .iter()
            .find(|entry| {
                entry
                    .receipts
                    .iter()
                    .any(|receipt| receipt.user_id == bob.as_str())
            })
            .cloned()
            .expect("timeline diff should contain a real receipt for the member");

        let session = MatrixClientSession::from_client_for_testing(
            client,
            SessionInfo {
                homeserver: "http://example.invalid".to_owned(),
                user_id: ALICE.to_string(),
                device_id: "DEVICE".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            },
        );
        let mut state = AppState {
            session: SessionState::Ready(session.info.clone()),
            ..AppState::default()
        };
        reduce(
            &mut state,
            AppAction::LiveRoomReceiptsUpdated {
                room_id: room_id.to_string(),
                receipts_by_event: vec![observed_receipts.clone()],
            },
        );
        assert_eq!(
            state.live_signals.rooms[room_id.as_str()].receipts_by_event
                [&observed_receipts.event_id]
                .readers[0]
                .display_name
                .as_deref(),
            Some("Unknown user")
        );

        let action_batch = live_receipt_observation_actions_from_sdk_receipts(
            &session,
            room_id.as_str(),
            vec![observed_receipts.clone()],
        )
        .await;
        assert!(matches!(
            action_batch.first(),
            Some(AppAction::LiveRoomProfilesObserved {
                room_id: observed_room_id,
                profiles,
            }) if observed_room_id == room_id.as_str()
                && profiles.iter().any(|profile| {
                    profile.user_id == bob.as_str()
                        && profile.display_name.as_deref() == Some("Relevant room member")
                })
        ));
        assert!(matches!(
            action_batch.last(),
            Some(AppAction::LiveRoomReceiptsUpdated { room_id: observed_room_id, .. })
                if observed_room_id == room_id.as_str()
        ));

        for action in action_batch {
            reduce(&mut state, action);
        }

        assert_eq!(
            state.profile.room_users[room_id.as_str()][bob.as_str()]
                .display_name
                .as_deref(),
            Some("Relevant room member")
        );
        assert_eq!(
            state.profile.users[bob.as_str()].display_name.as_deref(),
            Some("Relevant room member")
        );
        assert_eq!(
            state.live_signals.rooms[room_id.as_str()].receipts_by_event
                [&observed_receipts.event_id]
                .readers[0]
                .display_name
                .as_deref(),
            Some("Relevant room member")
        );
    }

    #[tokio::test]
    async fn production_receipt_diff_delivery_refreshes_unknown_with_room_profile() {
        use koushi_state::{AppState, reduce};
        use matrix_sdk::ruma::{event_id, room_id, user_id};
        use matrix_sdk::test_utils::mocks::MatrixMockServer;
        use matrix_sdk_test::{ALICE, JoinedRoomBuilder, event_factory::EventFactory};

        let server = MatrixMockServer::new().await;
        let client = server.client_builder().build().await;
        let room_id = room_id!("!receipt-production:example.test");
        let bob = user_id!("@bob:example.test");
        server.sync_joined_room(&client, room_id).await;
        server
            .sync_room(
                &client,
                JoinedRoomBuilder::new(room_id).add_state_event(
                    EventFactory::new()
                        .room(room_id)
                        .member(bob)
                        .display_name("Relevant room member")
                        .into_raw_sync_state(),
                ),
            )
            .await;

        let session = Arc::new(MatrixClientSession::from_client_for_testing(
            client,
            SessionInfo {
                homeserver: "http://example.invalid".to_owned(),
                user_id: ALICE.to_string(),
                device_id: "DEVICE".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            },
        ));
        let receipts = vec![LiveEventReceipts {
            event_id: event_id!("$receipt-production:example.test").to_string(),
            receipts: vec![LiveReadReceipt {
                user_id: bob.to_string(),
                display_name: None,
                original_display_label: String::new(),
                avatar: None,
                timestamp_ms: Some(1),
            }],
        }];
        let mut state = AppState {
            session: SessionState::Ready(session.info.clone()),
            ..AppState::default()
        };
        reduce(
            &mut state,
            AppAction::LiveRoomReceiptsUpdated {
                room_id: room_id.to_string(),
                receipts_by_event: receipts.clone(),
            },
        );
        state.profile.users.insert(
            bob.to_string(),
            UserProfile {
                user_id: bob.to_string(),
                display_name: Some("Global cache".to_owned()),
                display_label: "Global cache".to_owned(),
                original_display_label: "Global cache".to_owned(),
                mention_search_terms: Vec::new(),
                avatar: None,
            },
        );
        assert_eq!(
            state.live_signals.rooms[room_id.as_str()].receipts_by_event[&receipts[0].event_id]
                .readers[0]
                .display_name
                .as_deref(),
            Some("Unknown user"),
            "the production batch must refresh an already-projected Unknown receipt"
        );

        let key = TimelineKey::room(AccountKey(ALICE.to_string()), room_id.to_string());
        let generations = Arc::new(TimelineActorGenerationGate::default());
        let actor_generation = generations.activate_after_quiescence(&key).await.generation;
        let (action_tx, mut action_rx) = mpsc::channel(1);
        assert!(
            emit_live_receipt_observation_actions(
                session.as_ref(),
                &action_tx,
                &generations,
                &key,
                actor_generation,
                room_id.as_str(),
                receipts.clone(),
            )
            .await
        );
        let action_batch = action_rx.recv().await.expect("receipt action batch");
        assert!(matches!(
            action_batch.as_slice(),
            [
                AppAction::LiveRoomProfilesObserved { profiles, .. },
                AppAction::UserProfilesUpdated { profiles: cached },
                AppAction::LiveRoomReceiptsUpdated { .. },
            ] if profiles.iter().any(|profile| {
                profile.user_id == bob.as_str()
                    && profile.display_name.as_deref() == Some("Relevant room member")
            }) && cached.iter().any(|profile| {
                profile.user_id == bob.as_str()
                    && profile.display_name.as_deref() == Some("Relevant room member")
            })
        ));

        for action in action_batch {
            reduce(&mut state, action);
        }
        assert_eq!(
            state.live_signals.rooms[room_id.as_str()].receipts_by_event[&receipts[0].event_id]
                .readers[0]
                .display_name
                .as_deref(),
            Some("Relevant room member"),
            "the relevant room profile must beat the global cache"
        );
    }

    #[tokio::test]
    async fn production_receipt_diff_delivery_uses_global_cache_when_local_lookup_misses() {
        use koushi_state::{AppState, reduce};
        use matrix_sdk::ruma::{event_id, room_id};
        use matrix_sdk::test_utils::mocks::MatrixMockServer;
        use matrix_sdk_test::ALICE;

        let server = MatrixMockServer::new().await;
        let client = server.client_builder().build().await;
        let room_id = room_id!("!receipt-cache-fallback:example.test");
        server.sync_joined_room(&client, room_id).await;
        let session = Arc::new(MatrixClientSession::from_client_for_testing(
            client,
            SessionInfo {
                homeserver: "http://example.invalid".to_owned(),
                user_id: ALICE.to_string(),
                device_id: "DEVICE".to_owned(),
                authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
            },
        ));
        let bob = "@bob:example.test";
        let receipts = vec![LiveEventReceipts {
            event_id: event_id!("$receipt-cache-fallback:example.test").to_string(),
            receipts: vec![LiveReadReceipt {
                user_id: bob.to_owned(),
                display_name: None,
                original_display_label: String::new(),
                avatar: None,
                timestamp_ms: Some(2),
            }],
        }];
        let mut state = AppState {
            session: SessionState::Ready(session.info.clone()),
            ..AppState::default()
        };
        state.profile.users.insert(
            bob.to_owned(),
            UserProfile {
                user_id: bob.to_owned(),
                display_name: Some("Global cache".to_owned()),
                display_label: "Global cache".to_owned(),
                original_display_label: "Global cache".to_owned(),
                mention_search_terms: Vec::new(),
                avatar: None,
            },
        );

        let key = TimelineKey::room(AccountKey(ALICE.to_string()), room_id.to_string());
        let generations = Arc::new(TimelineActorGenerationGate::default());
        let actor_generation = generations.activate_after_quiescence(&key).await.generation;
        let (action_tx, mut action_rx) = mpsc::channel(1);
        assert!(
            emit_live_receipt_observation_actions(
                session.as_ref(),
                &action_tx,
                &generations,
                &key,
                actor_generation,
                room_id.as_str(),
                receipts.clone(),
            )
            .await
        );
        let action_batch = action_rx.recv().await.expect("receipt fallback batch");
        assert!(matches!(
            action_batch.as_slice(),
            [AppAction::LiveRoomReceiptsUpdated { .. }]
        ));
        for action in action_batch {
            reduce(&mut state, action);
        }
        assert_eq!(
            state.live_signals.rooms[room_id.as_str()].receipts_by_event[&receipts[0].event_id]
                .readers[0]
                .display_name
                .as_deref(),
            Some("Global cache")
        );
    }

    #[tokio::test]
    async fn production_receipt_diff_delivery_sends_receipts_when_local_lookup_fails() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        use koushi_state::SessionAuthenticationMethod;
        use matrix_sdk::ruma::event_id;
        use matrix_sdk::test_utils::mocks::MatrixMockServer;
        use matrix_sdk_test::ALICE;

        let server = MatrixMockServer::new().await;
        let client = server.client_builder().build().await;
        let session = Arc::new(MatrixClientSession::from_client_for_testing(
            client,
            SessionInfo {
                homeserver: "http://example.invalid".to_owned(),
                user_id: ALICE.to_string(),
                device_id: "DEVICE".to_owned(),
                authentication_method: SessionAuthenticationMethod::Unknown,
            },
        ));
        let receipts = vec![LiveEventReceipts {
            event_id: event_id!("$receipt-lookup-failure:example.test").to_string(),
            receipts: vec![LiveReadReceipt {
                user_id: "@bob:example.test".to_owned(),
                display_name: None,
                original_display_label: String::new(),
                avatar: None,
                timestamp_ms: Some(3),
            }],
        }];
        let key = TimelineKey::room(
            AccountKey(ALICE.to_string()),
            "!receipt-failure:example.test",
        );
        let generations = Arc::new(TimelineActorGenerationGate::default());
        let actor_generation = generations.activate_after_quiescence(&key).await.generation;
        let (action_tx, mut action_rx) = mpsc::channel(1);
        let records_before = koushi_diagnostics::test_support::detail_snapshot()
            .records
            .len();
        assert!(
            emit_live_receipt_observation_actions(
                session.as_ref(),
                &action_tx,
                &generations,
                &key,
                actor_generation,
                "not-a-room-id",
                receipts,
            )
            .await
        );
        let action_batch = action_rx.recv().await.expect("failed lookup receipt batch");
        assert!(matches!(
            action_batch.as_slice(),
            [AppAction::LiveRoomReceiptsUpdated { .. }]
        ));
        assert!(
            koushi_diagnostics::test_support::detail_snapshot()
                .records
                .iter()
                .skip(records_before)
                .any(|record| {
                    record.event.source == "core.read_receipt_profile"
                        && record.event.stage == "local_lookup"
                        && record.event.fields.iter().any(|field| {
                            field.key == "lookup_outcome"
                                && field.value == DiagnosticValue::Token("failed")
                        })
                }),
            "lookup failures must record a sanitized outcome"
        );
    }

    #[tokio::test]
    async fn stale_production_receipt_diff_result_is_discarded_after_generation_replacement() {
        use koushi_state::SessionAuthenticationMethod;
        use matrix_sdk::ruma::event_id;
        use matrix_sdk::test_utils::mocks::MatrixMockServer;
        use matrix_sdk_test::ALICE;

        let server = MatrixMockServer::new().await;
        let client = server.client_builder().build().await;
        let session = Arc::new(MatrixClientSession::from_client_for_testing(
            client,
            SessionInfo {
                homeserver: "http://example.invalid".to_owned(),
                user_id: ALICE.to_string(),
                device_id: "DEVICE".to_owned(),
                authentication_method: SessionAuthenticationMethod::Unknown,
            },
        ));
        let receipts = vec![LiveEventReceipts {
            event_id: event_id!("$receipt-stale:example.test").to_string(),
            receipts: vec![LiveReadReceipt {
                user_id: "@bob:example.test".to_owned(),
                display_name: None,
                original_display_label: String::new(),
                avatar: None,
                timestamp_ms: Some(4),
            }],
        }];
        let key = TimelineKey::room(AccountKey(ALICE.to_string()), "!receipt-stale:example.test");
        let generations = Arc::new(TimelineActorGenerationGate::default());
        let stale_generation = generations.activate_after_quiescence(&key).await.generation;
        let (action_tx, mut action_rx) = mpsc::channel(1);
        action_tx
            .send(vec![AppAction::TypingUsersUpdated {
                room_id: "!occupied:example.test".to_owned(),
                user_ids: Vec::new(),
            }])
            .await
            .expect("fill action channel");

        let delivery = tokio::spawn({
            let session = Arc::clone(&session);
            let action_tx = action_tx.clone();
            let generations = Arc::clone(&generations);
            let key = key.clone();
            async move {
                emit_live_receipt_observation_actions(
                    session.as_ref(),
                    &action_tx,
                    &generations,
                    &key,
                    stale_generation,
                    "not-a-room-id",
                    receipts,
                )
                .await
            }
        });
        tokio::task::yield_now().await;
        let replacement_generation = generations.activate_after_quiescence(&key).await.generation;
        assert_ne!(replacement_generation, stale_generation);
        assert!(matches!(
            action_rx.recv().await,
            Some(actions) if matches!(
                actions.as_slice(),
                [AppAction::TypingUsersUpdated { room_id, .. }] if room_id == "!occupied:example.test"
            )
        ));
        assert!(!delivery.await.expect("stale delivery task"));
        assert!(
            action_rx.try_recv().is_err(),
            "a stale actor generation must not publish the receipt batch"
        );
    }

    #[test]
    fn production_receipt_diff_path_uses_fenced_ordered_observation_delivery() {
        let diff_handler = item_body(include_str!("relay.rs"), "async fn handle_diff_batch");
        let delivery = item_body(
            include_str!("item_projection.rs"),
            "async fn emit_receipt_observation_actions",
        );
        assert!(
            diff_handler.contains("emit_live_receipt_observation_actions"),
            "receipt diffs must use the production profile-observation delivery path"
        );
        assert!(
            delivery.contains("send_generation_fenced"),
            "receipt profile actions must use the actor-generation fence"
        );
        assert!(
            !diff_handler.contains("try_send(vec![action])"),
            "receipt action batches must not be dropped through try_send"
        );
    }

    #[test]
    fn initial_receipts_use_the_ordered_local_profile_observation_batch() {
        let source = include_str!("actor.rs");
        let startup = source
            .split("let initial_receipts = live_event_receipts_from_sdk_items")
            .nth(1)
            .expect("initial receipt projection exists")
            .split("let thread_attention = ThreadAttentionTracker::hydrate")
            .next()
            .expect("initial receipt publication precedes thread attention hydration");

        assert!(
            startup.contains("emit_receipt_observation_actions"),
            "initial receipts must use local profile observation and generation fencing"
        );
        assert!(
            !startup.contains("LiveRoomReceiptsUpdated {"),
            "initial receipts must not bypass the ordered profile/receipt batch"
        );
        assert!(
            !startup.contains("try_send(actions)"),
            "initial receipt publication must be reliable"
        );
    }

    #[test]
    fn authoritative_recovery_receipts_use_the_same_ordered_observation_batch() {
        let source = include_str!("relay.rs");
        let recovery = source
                .split("async fn handle_relay_overflow")
                .nth(1)
                .expect("authoritative recovery handler exists")
                .split("// ---------------------------------------------------------------------------\n// Relay task")
                .next()
                .expect("authoritative recovery handler boundary exists");

        assert!(
            recovery.contains("emit_receipt_observation_actions"),
            "authoritative recovery must use local profile observation and generation fencing"
        );
        assert!(
            !recovery.contains("if let Some(action) = receipts_action"),
            "authoritative recovery must not publish an unobserved receipt action directly"
        );
    }
}
