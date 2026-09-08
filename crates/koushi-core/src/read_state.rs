//! Pure manager-owned state for read-receipt and fully-read convergence.
//!
//! Matrix identifiers stay in [`ReadStateKey`], [`ReadTarget`], and
//! [`ReadOperation`], which are internal work values with redacted `Debug`
//! implementations. Admission and completion diagnostics expose only closed
//! enums, generation/count values, and waiter counts.

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

pub(crate) use koushi_protocol::failure::ReadStateFailureKind;

pub(crate) const READ_STATE_WAITER_LIMIT: usize = 32;
pub(crate) const READ_STATE_OUTBOX_ENTRY_LIMIT: usize = 128;
const READ_STATE_LEGACY_CANDIDATE_LIMIT: usize = 8;

#[derive(Clone, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub(crate) enum ReadStateKey {
    PublicUnthreaded {
        room_id: String,
    },
    ThreadRead {
        room_id: String,
        root_event_id: String,
    },
    FullyReadAndPrivateUnthreaded {
        room_id: String,
    },
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ReadPersistenceEntry {
    key: ReadStateKey,
    event_id: String,
}

impl ReadPersistenceEntry {
    pub(crate) fn key(&self) -> &ReadStateKey {
        &self.key
    }

    pub(crate) fn event_ids(&self) -> &[String] {
        std::slice::from_ref(&self.event_id)
    }

    #[cfg(test)]
    pub(crate) fn event_id(&self) -> &str {
        self.event_id.as_str()
    }
}

impl fmt::Debug for ReadPersistenceEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReadPersistenceEntry")
            .field("key", &self.key)
            .field("candidate_count", &1_usize)
            .finish()
    }
}

#[derive(Clone, Default, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ReadPersistenceSnapshot {
    entries: Vec<ReadPersistenceEntry>,
}

impl ReadPersistenceSnapshot {
    pub(crate) fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn candidate_count(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn entries(&self) -> &[ReadPersistenceEntry] {
        self.entries.as_slice()
    }

    pub(crate) fn apply_receipt_policy(&mut self, send_read_receipts: bool) -> bool {
        if send_read_receipts {
            return false;
        }
        let before = self.entries.len();
        self.entries.retain(|entry| {
            matches!(
                &entry.key,
                ReadStateKey::FullyReadAndPrivateUnthreaded { .. }
            )
        });
        self.entries.len() != before
    }

    pub(crate) fn from_legacy_entries(entries: Vec<(ReadStateKey, Vec<String>)>) -> Option<Self> {
        if entries.len() > READ_STATE_OUTBOX_ENTRY_LIMIT {
            return None;
        }
        let mut keys = HashMap::with_capacity(entries.len());
        for (key, event_ids) in entries {
            if event_ids.is_empty()
                || event_ids.len() > READ_STATE_LEGACY_CANDIDATE_LIMIT
                || event_ids.iter().any(String::is_empty)
                || event_ids
                    .iter()
                    .enumerate()
                    .any(|(index, event_id)| event_ids[..index].contains(event_id))
                || keys.contains_key(&key)
            {
                return None;
            }
            let event_id = event_ids.last()?.clone();
            keys.insert(key, event_id);
        }
        Some(Self {
            entries: keys
                .into_iter()
                .map(|(key, event_id)| ReadPersistenceEntry { key, event_id })
                .collect(),
        })
    }
}

impl fmt::Debug for ReadPersistenceSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReadPersistenceSnapshot")
            .field("entry_count", &self.entry_count())
            .field("candidate_count", &self.candidate_count())
            .finish()
    }
}

impl fmt::Debug for ReadStateKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PublicUnthreaded { .. } => "ReadStateKey::PublicUnthreaded",
            Self::ThreadRead { .. } => "ReadStateKey::ThreadRead",
            Self::FullyReadAndPrivateUnthreaded { .. } => {
                "ReadStateKey::FullyReadAndPrivateUnthreaded"
            }
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReadPositionEvidence {
    pub(crate) generation: u128,
    pub(crate) rank: u64,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct ReadTarget {
    event_id: String,
    position: Option<ReadPositionEvidence>,
}

impl ReadTarget {
    pub(crate) fn new(event_id: String) -> Self {
        Self {
            event_id,
            position: None,
        }
    }

    pub(crate) fn with_position(event_id: String, position: ReadPositionEvidence) -> Self {
        Self {
            event_id,
            position: Some(position),
        }
    }

    pub(crate) fn event_id(&self) -> &str {
        self.event_id.as_str()
    }

    pub(crate) fn position(&self) -> Option<ReadPositionEvidence> {
        self.position
    }
}

impl fmt::Debug for ReadTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReadTarget")
            .field("event_id", &"EventId(..)")
            .field("position", &self.position)
            .finish()
    }
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub(crate) struct ReadWaiterId(u64);

impl ReadWaiterId {
    pub(crate) fn new(value: u64) -> Self {
        Self(value)
    }

    #[cfg(test)]
    pub(crate) fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for ReadWaiterId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ReadWaiterId(..)")
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ReadOperationFence {
    session_generation: u64,
    operation_generation: u64,
}

impl ReadOperationFence {
    pub(crate) fn new(session_generation: u64, operation_generation: u64) -> Self {
        Self {
            session_generation,
            operation_generation,
        }
    }

    #[cfg(test)]
    pub(crate) fn session_generation(self) -> u64 {
        self.session_generation
    }

    #[cfg(test)]
    pub(crate) fn operation_generation(self) -> u64 {
        self.operation_generation
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReadOperation {
    key: ReadStateKey,
    target: ReadTarget,
    fence: ReadOperationFence,
}

impl ReadOperation {
    pub(crate) fn key(&self) -> &ReadStateKey {
        &self.key
    }

    pub(crate) fn target(&self) -> &ReadTarget {
        &self.target
    }

    pub(crate) fn fence(&self) -> ReadOperationFence {
        self.fence
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReadAdmissionRejection {
    StaleSession,
    CandidateCapacity,
    WaiterCapacity,
    DuplicateWaiter,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReadAdmissionStatus {
    Accepted,
    Coalesced,
    Rejected(ReadAdmissionRejection),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReadAdmissionDiagnostic {
    Accepted {
        candidate_count: usize,
        waiter_count: usize,
        superseded_operation_count: usize,
    },
    Coalesced {
        candidate_count: usize,
        waiter_count: usize,
        superseded_operation_count: usize,
    },
    Rejected {
        reason: ReadAdmissionRejection,
        candidate_count: usize,
        waiter_count: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReadAdmissionResult {
    status: ReadAdmissionStatus,
    superseded_operation: Option<ReadOperationFence>,
    candidate_count: usize,
    waiter_count: usize,
}

impl ReadAdmissionResult {
    pub(crate) fn status(self) -> ReadAdmissionStatus {
        self.status
    }

    pub(crate) fn superseded_operation(self) -> Option<ReadOperationFence> {
        self.superseded_operation
    }

    pub(crate) fn diagnostic(self) -> ReadAdmissionDiagnostic {
        match self.status {
            ReadAdmissionStatus::Accepted => ReadAdmissionDiagnostic::Accepted {
                candidate_count: self.candidate_count,
                waiter_count: self.waiter_count,
                superseded_operation_count: usize::from(self.superseded_operation.is_some()),
            },
            ReadAdmissionStatus::Coalesced => ReadAdmissionDiagnostic::Coalesced {
                candidate_count: self.candidate_count,
                waiter_count: self.waiter_count,
                superseded_operation_count: usize::from(self.superseded_operation.is_some()),
            },
            ReadAdmissionStatus::Rejected(reason) => ReadAdmissionDiagnostic::Rejected {
                reason,
                candidate_count: self.candidate_count,
                waiter_count: self.waiter_count,
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ReadWakeResult {
    Start(ReadOperation),
    AlreadyActive,
    NoDesired,
    OperationGenerationExhausted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReadNetworkFailure {
    pub(crate) kind: ReadStateFailureKind,
    pub(crate) retry_after: Option<std::time::Duration>,
}

impl ReadNetworkFailure {
    pub(crate) const fn new(kind: ReadStateFailureKind) -> Self {
        Self {
            kind,
            retry_after: None,
        }
    }

    #[cfg(test)]
    pub(crate) const fn with_retry_after(
        kind: ReadStateFailureKind,
        retry_after: std::time::Duration,
    ) -> Self {
        Self {
            kind,
            retry_after: Some(retry_after),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReadNetworkOutcome {
    Succeeded,
    Failed(ReadNetworkFailure),
    /// Kept as a distinct completion for retry diagnostics; the typed failure
    /// exposed to the engine is still `Timeout`.
    TimedOut,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReadCompletionDisposition {
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
    StaleDiscarded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReadWaiterTerminal {
    Converged,
    Failed,
    TimedOut,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) struct ReadWaiterSettlement {
    waiter: ReadWaiterId,
    terminal: ReadWaiterTerminal,
}

impl ReadWaiterSettlement {
    pub(crate) fn waiter(self) -> ReadWaiterId {
        self.waiter
    }

    pub(crate) fn terminal(self) -> ReadWaiterTerminal {
        self.terminal
    }
}

impl fmt::Debug for ReadWaiterSettlement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReadWaiterSettlement")
            .field("waiter", &"ReadWaiterId(..)")
            .field("terminal", &self.terminal)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReadCompletionDiagnostic {
    Succeeded {
        settled_waiter_count: usize,
        remaining_candidate_count: usize,
        remaining_waiter_count: usize,
    },
    Failed {
        settled_waiter_count: usize,
        remaining_candidate_count: usize,
        remaining_waiter_count: usize,
        failure_kind: ReadStateFailureKind,
    },
    TimedOut {
        settled_waiter_count: usize,
        remaining_candidate_count: usize,
        remaining_waiter_count: usize,
        failure_kind: ReadStateFailureKind,
    },
    StaleDiscarded {
        remaining_candidate_count: usize,
        remaining_waiter_count: usize,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReadCompletionResult {
    disposition: ReadCompletionDisposition,
    settlements: Vec<ReadWaiterSettlement>,
    remaining_candidate_count: usize,
    remaining_waiter_count: usize,
    failure_kind: Option<ReadStateFailureKind>,
}

pub(crate) struct ReadAuthoritativeConfirmation {
    settlements: Vec<ReadWaiterSettlement>,
    superseded_operation: Option<ReadOperationFence>,
}

impl ReadAuthoritativeConfirmation {
    pub(crate) fn settlements(&self) -> &[ReadWaiterSettlement] {
        self.settlements.as_slice()
    }

    pub(crate) fn superseded_operation(&self) -> Option<ReadOperationFence> {
        self.superseded_operation
    }
}

impl fmt::Debug for ReadAuthoritativeConfirmation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReadAuthoritativeConfirmation")
            .field("settled_waiter_count", &self.settlements.len())
            .field(
                "superseded_operation_count",
                &usize::from(self.superseded_operation.is_some()),
            )
            .finish()
    }
}

impl ReadCompletionResult {
    pub(crate) fn disposition(&self) -> ReadCompletionDisposition {
        self.disposition
    }

    pub(crate) fn settlements(&self) -> &[ReadWaiterSettlement] {
        self.settlements.as_slice()
    }

    pub(crate) fn diagnostic(&self) -> ReadCompletionDiagnostic {
        let settled_waiter_count = self.settlements.len();
        match self.disposition {
            ReadCompletionDisposition::Succeeded => ReadCompletionDiagnostic::Succeeded {
                settled_waiter_count,
                remaining_candidate_count: self.remaining_candidate_count,
                remaining_waiter_count: self.remaining_waiter_count,
            },
            ReadCompletionDisposition::Failed => ReadCompletionDiagnostic::Failed {
                settled_waiter_count,
                remaining_candidate_count: self.remaining_candidate_count,
                remaining_waiter_count: self.remaining_waiter_count,
                failure_kind: self.failure_kind.unwrap_or(ReadStateFailureKind::Sdk),
            },
            ReadCompletionDisposition::TimedOut => ReadCompletionDiagnostic::TimedOut {
                settled_waiter_count,
                remaining_candidate_count: self.remaining_candidate_count,
                remaining_waiter_count: self.remaining_waiter_count,
                failure_kind: self.failure_kind.unwrap_or(ReadStateFailureKind::Timeout),
            },
            ReadCompletionDisposition::Cancelled => ReadCompletionDiagnostic::StaleDiscarded {
                remaining_candidate_count: self.remaining_candidate_count,
                remaining_waiter_count: self.remaining_waiter_count,
            },
            ReadCompletionDisposition::StaleDiscarded => ReadCompletionDiagnostic::StaleDiscarded {
                remaining_candidate_count: self.remaining_candidate_count,
                remaining_waiter_count: self.remaining_waiter_count,
            },
        }
    }
}

struct ReadCandidate {
    target: ReadTarget,
    waiters: Vec<ReadWaiterId>,
}

struct ActiveReadOperation {
    event_id: String,
    fence: ReadOperationFence,
}

#[derive(Default)]
struct ReadKeyState {
    desired: Option<ReadCandidate>,
    active: Option<ActiveReadOperation>,
    /// The last failure belongs only to the current desired target. Replacing
    /// that target clears it; retrying the same target retains it for the
    /// closed diagnostic/status projection.
    last_failure: Option<ReadNetworkFailure>,
}

pub(crate) struct ReadStateEngine {
    session_generation: u64,
    operation_generation: u64,
    keys: HashMap<ReadStateKey, ReadKeyState>,
}

impl ReadStateEngine {
    pub(crate) fn new(session_generation: u64) -> Self {
        Self {
            session_generation,
            operation_generation: 0,
            keys: HashMap::new(),
        }
    }

    pub(crate) fn session_generation(&self) -> u64 {
        self.session_generation
    }

    #[cfg(test)]
    pub(crate) fn last_operation_generation(&self) -> u64 {
        self.operation_generation
    }

    pub(crate) fn restore(
        session_generation: u64,
        snapshot: ReadPersistenceSnapshot,
    ) -> Option<Self> {
        if snapshot.entries.len() > READ_STATE_OUTBOX_ENTRY_LIMIT {
            return None;
        }
        let mut keys = HashMap::with_capacity(snapshot.entries.len());
        for entry in snapshot.entries {
            if entry.event_id.is_empty() || keys.contains_key(&entry.key) {
                return None;
            }
            keys.insert(
                entry.key,
                ReadKeyState {
                    desired: Some(ReadCandidate {
                        target: ReadTarget::new(entry.event_id),
                        waiters: Vec::new(),
                    }),
                    active: None,
                    last_failure: None,
                },
            );
        }
        Some(Self {
            session_generation,
            operation_generation: 0,
            keys,
        })
    }

    pub(crate) fn persistence_snapshot(&self) -> ReadPersistenceSnapshot {
        let entries = self
            .keys
            .iter()
            .filter_map(|(key, state)| {
                state.desired.as_ref().map(|desired| ReadPersistenceEntry {
                    key: key.clone(),
                    event_id: desired.target.event_id.clone(),
                })
            })
            .take(READ_STATE_OUTBOX_ENTRY_LIMIT)
            .collect();
        ReadPersistenceSnapshot { entries }
    }

    pub(crate) fn admit(
        &mut self,
        session_generation: u64,
        key: ReadStateKey,
        target: ReadTarget,
        waiter: ReadWaiterId,
    ) -> ReadAdmissionResult {
        if session_generation != self.session_generation {
            return self.rejected_admission(&key, ReadAdmissionRejection::StaleSession);
        }
        if !self.keys.contains_key(&key) && self.keys.len() >= READ_STATE_OUTBOX_ENTRY_LIMIT {
            return self.rejected_admission(&key, ReadAdmissionRejection::CandidateCapacity);
        }
        let state = self.keys.entry(key).or_default();
        if state
            .desired
            .as_ref()
            .is_some_and(|candidate| candidate.waiters.contains(&waiter))
        {
            return admission_result(
                ReadAdmissionStatus::Rejected(ReadAdmissionRejection::DuplicateWaiter),
                None,
                state,
            );
        }
        if waiter_count(state) >= READ_STATE_WAITER_LIMIT {
            return admission_result(
                ReadAdmissionStatus::Rejected(ReadAdmissionRejection::WaiterCapacity),
                None,
                state,
            );
        }

        let (status, superseded_operation) = match state.desired.as_mut() {
            None => {
                state.desired = Some(ReadCandidate {
                    target,
                    waiters: vec![waiter],
                });
                (ReadAdmissionStatus::Accepted, None)
            }
            Some(desired) if desired.target.event_id == target.event_id => {
                desired.target.position =
                    preferred_same_event_position(desired.target.position, target.position);
                desired.waiters.push(waiter);
                (ReadAdmissionStatus::Coalesced, None)
            }
            Some(desired) if dominates(&desired.target, &target) => {
                desired.waiters.push(waiter);
                (ReadAdmissionStatus::Coalesced, None)
            }
            Some(desired) => {
                let old_event_id = desired.target.event_id.clone();
                let mut waiters = std::mem::take(&mut desired.waiters);
                waiters.push(waiter);
                desired.target = target;
                desired.waiters = waiters;
                state.last_failure = None;
                let superseded = state
                    .active
                    .as_ref()
                    .is_some_and(|active| active.event_id == old_event_id)
                    .then(|| state.active.as_ref().map(|active| active.fence))
                    .flatten();
                (ReadAdmissionStatus::Accepted, superseded)
            }
        };

        admission_result(status, superseded_operation, state)
    }

    pub(crate) fn admit_background(
        &mut self,
        session_generation: u64,
        key: ReadStateKey,
        target: ReadTarget,
    ) -> ReadAdmissionResult {
        if session_generation != self.session_generation {
            return self.rejected_admission(&key, ReadAdmissionRejection::StaleSession);
        }
        if !self.keys.contains_key(&key) && self.keys.len() >= READ_STATE_OUTBOX_ENTRY_LIMIT {
            return self.rejected_admission(&key, ReadAdmissionRejection::CandidateCapacity);
        }
        let state = self.keys.entry(key).or_default();
        let (status, superseded_operation) = match state.desired.as_mut() {
            None => {
                state.desired = Some(ReadCandidate {
                    target,
                    waiters: Vec::new(),
                });
                (ReadAdmissionStatus::Accepted, None)
            }
            Some(desired) if desired.target.event_id == target.event_id => {
                desired.target.position =
                    preferred_same_event_position(desired.target.position, target.position);
                (ReadAdmissionStatus::Coalesced, None)
            }
            Some(desired) if dominates(&desired.target, &target) => {
                (ReadAdmissionStatus::Coalesced, None)
            }
            Some(desired) => {
                let old_event_id = desired.target.event_id.clone();
                desired.target = target;
                state.last_failure = None;
                let superseded = state
                    .active
                    .as_ref()
                    .is_some_and(|active| active.event_id == old_event_id)
                    .then(|| state.active.as_ref().map(|active| active.fence))
                    .flatten();
                (ReadAdmissionStatus::Accepted, superseded)
            }
        };
        admission_result(status, superseded_operation, state)
    }

    pub(crate) fn observe_position(
        &mut self,
        session_generation: u64,
        key: &ReadStateKey,
        event_id: &str,
        evidence: ReadPositionEvidence,
    ) {
        if session_generation != self.session_generation {
            return;
        }
        let Some(state) = self.keys.get_mut(key) else {
            return;
        };
        let Some(desired) = state.desired.as_mut() else {
            return;
        };
        if desired.target.event_id != event_id {
            return;
        }

        if desired
            .target
            .position
            .is_some_and(|known| evidence_is_older(evidence, known))
        {
            return;
        }

        desired.target.position = Some(evidence);
    }

    pub(crate) fn wake(&mut self, key: &ReadStateKey) -> ReadWakeResult {
        let Some(state) = self.keys.get(key) else {
            return ReadWakeResult::NoDesired;
        };
        if state.active.is_some() {
            return ReadWakeResult::AlreadyActive;
        }
        let Some(desired) = state.desired.as_ref() else {
            return ReadWakeResult::NoDesired;
        };
        let target = desired.target.clone();
        let Some(operation_generation) = self.operation_generation.checked_add(1) else {
            return ReadWakeResult::OperationGenerationExhausted;
        };
        self.operation_generation = operation_generation;
        let fence = ReadOperationFence::new(self.session_generation, operation_generation);
        self.keys
            .get_mut(key)
            .expect("read state key must remain present while starting")
            .active = Some(ActiveReadOperation {
            event_id: target.event_id.clone(),
            fence,
        });

        ReadWakeResult::Start(ReadOperation {
            key: key.clone(),
            target,
            fence,
        })
    }

    pub(crate) fn complete(
        &mut self,
        key: &ReadStateKey,
        fence: ReadOperationFence,
        outcome: ReadNetworkOutcome,
    ) -> ReadCompletionResult {
        if fence.session_generation != self.session_generation {
            return self.stale_completion(key);
        }
        let Some(state) = self.keys.get_mut(key) else {
            return ReadCompletionResult {
                disposition: ReadCompletionDisposition::StaleDiscarded,
                settlements: Vec::new(),
                remaining_candidate_count: 0,
                remaining_waiter_count: 0,
                failure_kind: None,
            };
        };
        let Some(active) = state.active.take() else {
            return completion_result(
                ReadCompletionDisposition::StaleDiscarded,
                Vec::new(),
                state,
                None,
            );
        };
        if active.fence != fence {
            state.active = Some(active);
            return completion_result(
                ReadCompletionDisposition::StaleDiscarded,
                Vec::new(),
                state,
                None,
            );
        }

        let active_matches_desired = state
            .desired
            .as_ref()
            .is_some_and(|desired| desired.target.event_id == active.event_id);
        if !active_matches_desired {
            let result = completion_result(
                ReadCompletionDisposition::StaleDiscarded,
                Vec::new(),
                state,
                None,
            );
            if state.desired.is_none() {
                self.keys.remove(key);
            }
            return result;
        }

        let (disposition, settlements, failure_kind) = match outcome {
            ReadNetworkOutcome::Succeeded => {
                let candidate = state.desired.take().expect("desired target remains active");
                state.last_failure = None;
                let settlements = candidate
                    .waiters
                    .into_iter()
                    .map(|waiter| ReadWaiterSettlement {
                        waiter,
                        terminal: ReadWaiterTerminal::Converged,
                    })
                    .collect();
                (ReadCompletionDisposition::Succeeded, settlements, None)
            }
            ReadNetworkOutcome::Failed(failure) => {
                state.last_failure = Some(failure);
                let waiters = state
                    .desired
                    .as_mut()
                    .map(|desired| std::mem::take(&mut desired.waiters))
                    .unwrap_or_default();
                let settlements = waiters
                    .into_iter()
                    .map(|waiter| ReadWaiterSettlement {
                        waiter,
                        terminal: ReadWaiterTerminal::Failed,
                    })
                    .collect();
                (
                    ReadCompletionDisposition::Failed,
                    settlements,
                    Some(failure.kind),
                )
            }
            ReadNetworkOutcome::TimedOut => {
                let failure = ReadNetworkFailure::new(ReadStateFailureKind::Timeout);
                state.last_failure = Some(failure);
                let waiters = state
                    .desired
                    .as_mut()
                    .map(|desired| std::mem::take(&mut desired.waiters))
                    .unwrap_or_default();
                let settlements = waiters
                    .into_iter()
                    .map(|waiter| ReadWaiterSettlement {
                        waiter,
                        terminal: ReadWaiterTerminal::TimedOut,
                    })
                    .collect();
                (
                    ReadCompletionDisposition::TimedOut,
                    settlements,
                    Some(failure.kind),
                )
            }
        };
        let result = completion_result(disposition, settlements, state, failure_kind);
        if state.desired.is_none() {
            self.keys.remove(key);
        }
        result
    }

    pub(crate) fn complete_cancelled(
        &mut self,
        key: &ReadStateKey,
        fence: ReadOperationFence,
    ) -> ReadCompletionResult {
        if fence.session_generation != self.session_generation {
            return self.stale_completion(key);
        }
        let Some(state) = self.keys.get_mut(key) else {
            return self.stale_completion(key);
        };
        if state
            .active
            .as_ref()
            .is_none_or(|active| active.fence != fence)
        {
            return completion_result(
                ReadCompletionDisposition::StaleDiscarded,
                Vec::new(),
                state,
                None,
            );
        }
        state.active = None;
        let result = completion_result(
            ReadCompletionDisposition::Cancelled,
            Vec::new(),
            state,
            None,
        );
        if state.desired.is_none() {
            self.keys.remove(key);
        }
        result
    }

    pub(crate) fn confirm_authoritative(
        &mut self,
        session_generation: u64,
        key: &ReadStateKey,
        confirmed: ReadTarget,
    ) -> ReadAuthoritativeConfirmation {
        if session_generation != self.session_generation {
            return ReadAuthoritativeConfirmation {
                settlements: Vec::new(),
                superseded_operation: None,
            };
        }
        let Some(state) = self.keys.get_mut(key) else {
            return ReadAuthoritativeConfirmation {
                settlements: Vec::new(),
                superseded_operation: None,
            };
        };

        let desired_is_satisfied = state
            .desired
            .as_ref()
            .is_some_and(|desired| same_target_or_dominated(&confirmed, &desired.target));
        let superseded_operation = desired_is_satisfied
            .then(|| state.active.as_ref().map(|active| active.fence))
            .flatten();
        let settlements = if desired_is_satisfied {
            state.last_failure = None;
            state
                .desired
                .take()
                .expect("satisfied desired target")
                .waiters
                .into_iter()
                .map(|waiter| ReadWaiterSettlement {
                    waiter,
                    terminal: ReadWaiterTerminal::Converged,
                })
                .collect()
        } else {
            Vec::new()
        };
        if state.desired.is_none() && state.active.is_none() {
            self.keys.remove(key);
        }
        ReadAuthoritativeConfirmation {
            settlements,
            superseded_operation,
        }
    }

    pub(crate) fn candidate_count(&self, key: &ReadStateKey) -> usize {
        usize::from(
            self.keys
                .get(key)
                .is_some_and(|state| state.desired.is_some()),
        )
    }

    pub(crate) fn waiter_count(&self, key: &ReadStateKey) -> usize {
        self.keys.get(key).map_or(0, waiter_count)
    }

    pub(crate) fn active_operation(&self, key: &ReadStateKey) -> Option<ReadOperationFence> {
        self.keys
            .get(key)
            .and_then(|state| state.active.as_ref().map(|active| active.fence))
    }

    pub(crate) fn active_operation_count(&self) -> usize {
        self.keys
            .values()
            .filter(|state| state.active.is_some())
            .count()
    }

    pub(crate) fn has_candidate(&self, key: &ReadStateKey, event_id: &str) -> bool {
        self.keys
            .get(key)
            .and_then(|state| state.desired.as_ref())
            .is_some_and(|desired| desired.target.event_id == event_id)
    }

    pub(crate) fn last_failure(&self, key: &ReadStateKey) -> Option<ReadNetworkFailure> {
        self.keys.get(key).and_then(|state| state.last_failure)
    }

    /// Retire one automatic key at actor/session teardown. The caller cancels
    /// the returned active fence before dropping the worker; no late completion
    /// can recreate the removed key.
    pub(crate) fn retire(&mut self, key: &ReadStateKey) -> Option<ReadOperationFence> {
        self.keys
            .remove(key)
            .and_then(|state| state.active.map(|active| active.fence))
    }

    pub(crate) fn retire_with_waiters(
        &mut self,
        key: &ReadStateKey,
    ) -> (Option<ReadOperationFence>, Vec<ReadWaiterId>) {
        let Some(state) = self.keys.remove(key) else {
            return (None, Vec::new());
        };
        let active = state.active.map(|active| active.fence);
        let waiters = state
            .desired
            .map(|desired| desired.waiters)
            .unwrap_or_default();
        (active, waiters)
    }

    fn rejected_admission(
        &self,
        key: &ReadStateKey,
        rejection: ReadAdmissionRejection,
    ) -> ReadAdmissionResult {
        let (candidate_count, waiter_count) = self.counts(key);
        ReadAdmissionResult {
            status: ReadAdmissionStatus::Rejected(rejection),
            superseded_operation: None,
            candidate_count,
            waiter_count,
        }
    }

    fn stale_completion(&self, key: &ReadStateKey) -> ReadCompletionResult {
        let (remaining_candidate_count, remaining_waiter_count) = self.counts(key);
        ReadCompletionResult {
            disposition: ReadCompletionDisposition::StaleDiscarded,
            settlements: Vec::new(),
            remaining_candidate_count,
            remaining_waiter_count,
            failure_kind: None,
        }
    }

    fn counts(&self, key: &ReadStateKey) -> (usize, usize) {
        self.keys.get(key).map_or((0, 0), |state| {
            (usize::from(state.desired.is_some()), waiter_count(state))
        })
    }
}

fn admission_result(
    status: ReadAdmissionStatus,
    superseded_operation: Option<ReadOperationFence>,
    state: &ReadKeyState,
) -> ReadAdmissionResult {
    ReadAdmissionResult {
        status,
        superseded_operation,
        candidate_count: usize::from(state.desired.is_some()),
        waiter_count: waiter_count(state),
    }
}

fn completion_result(
    disposition: ReadCompletionDisposition,
    settlements: Vec<ReadWaiterSettlement>,
    state: &ReadKeyState,
    failure_kind: Option<ReadStateFailureKind>,
) -> ReadCompletionResult {
    ReadCompletionResult {
        disposition,
        settlements,
        remaining_candidate_count: usize::from(state.desired.is_some()),
        remaining_waiter_count: waiter_count(state),
        failure_kind,
    }
}

fn waiter_count(state: &ReadKeyState) -> usize {
    state
        .desired
        .as_ref()
        .map_or(0, |desired| desired.waiters.len())
}

fn dominates(left: &ReadTarget, right: &ReadTarget) -> bool {
    matches!(
        (left.position, right.position),
        (Some(left), Some(right))
            if left.generation == right.generation && left.rank >= right.rank
    )
}

fn same_target_or_dominated(confirmed: &ReadTarget, candidate: &ReadTarget) -> bool {
    confirmed.event_id == candidate.event_id || dominates(confirmed, candidate)
}

fn evidence_is_older(candidate: ReadPositionEvidence, known: ReadPositionEvidence) -> bool {
    candidate.generation < known.generation
        || (candidate.generation == known.generation && candidate.rank < known.rank)
}

fn preferred_same_event_position(
    left: Option<ReadPositionEvidence>,
    right: Option<ReadPositionEvidence>,
) -> Option<ReadPositionEvidence> {
    match (left, right) {
        (None, None) => None,
        (Some(position), None) | (None, Some(position)) => Some(position),
        (Some(left), Some(right)) => Some(if evidence_is_older(left, right) {
            right
        } else {
            left
        }),
    }
}

#[cfg(test)]
mod tests;
