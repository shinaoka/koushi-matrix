//! Pure manager-owned state for read-receipt and fully-read convergence.
//!
//! Matrix identifiers stay in [`ReadStateKey`], [`ReadTarget`], and
//! [`ReadOperation`], which are internal work values with redacted `Debug`
//! implementations. Admission and completion diagnostics expose only closed
//! enums, generation/count values, and waiter counts.

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

pub(crate) use crate::failure::ReadStateFailureKind;

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

    pub(crate) fn session_generation(self) -> u64 {
        self.session_generation
    }

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReadEvidenceStatus {
    Updated,
    IgnoredOlderEvidence,
    UnknownTarget,
    StaleSession,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReadEvidenceResult {
    status: ReadEvidenceStatus,
    superseded_operation: Option<ReadOperationFence>,
    candidate_count: usize,
    waiter_count: usize,
}

impl ReadEvidenceResult {
    pub(crate) fn status(self) -> ReadEvidenceStatus {
        self.status
    }

    pub(crate) fn updated(self) -> bool {
        self.status == ReadEvidenceStatus::Updated
    }

    pub(crate) fn superseded_operation(self) -> Option<ReadOperationFence> {
        self.superseded_operation
    }

    pub(crate) fn candidate_count(self) -> usize {
        self.candidate_count
    }

    pub(crate) fn waiter_count(self) -> usize {
        self.waiter_count
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

    pub(crate) fn failure_kind(&self) -> Option<ReadStateFailureKind> {
        self.failure_kind
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
    ) -> ReadEvidenceResult {
        if session_generation != self.session_generation {
            let (candidate_count, waiter_count) = self.counts(key);
            return ReadEvidenceResult {
                status: ReadEvidenceStatus::StaleSession,
                superseded_operation: None,
                candidate_count,
                waiter_count,
            };
        }
        let Some(state) = self.keys.get_mut(key) else {
            return ReadEvidenceResult {
                status: ReadEvidenceStatus::UnknownTarget,
                superseded_operation: None,
                candidate_count: 0,
                waiter_count: 0,
            };
        };
        let Some(desired) = state.desired.as_mut() else {
            return ReadEvidenceResult {
                status: ReadEvidenceStatus::UnknownTarget,
                superseded_operation: None,
                candidate_count: 0,
                waiter_count: 0,
            };
        };
        if desired.target.event_id != event_id {
            return ReadEvidenceResult {
                status: ReadEvidenceStatus::UnknownTarget,
                superseded_operation: None,
                candidate_count: 1,
                waiter_count: waiter_count(state),
            };
        }

        if desired
            .target
            .position
            .is_some_and(|known| evidence_is_older(evidence, known))
        {
            return ReadEvidenceResult {
                status: ReadEvidenceStatus::IgnoredOlderEvidence,
                superseded_operation: None,
                candidate_count: 1,
                waiter_count: waiter_count(state),
            };
        }

        desired.target.position = Some(evidence);
        ReadEvidenceResult {
            status: ReadEvidenceStatus::Updated,
            superseded_operation: None,
            candidate_count: 1,
            waiter_count: waiter_count(state),
        }
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

    pub(crate) fn desired_target(&self, key: &ReadStateKey) -> Option<&ReadTarget> {
        self.keys
            .get(key)
            .and_then(|state| state.desired.as_ref())
            .map(|desired| &desired.target)
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
mod tests {
    use std::collections::HashSet;

    use super::{
        READ_STATE_OUTBOX_ENTRY_LIMIT, READ_STATE_WAITER_LIMIT, ReadAdmissionRejection,
        ReadAdmissionStatus, ReadCompletionDisposition, ReadNetworkFailure, ReadNetworkOutcome,
        ReadOperationFence, ReadPositionEvidence, ReadStateEngine, ReadStateFailureKind,
        ReadStateKey, ReadTarget, ReadWaiterId, ReadWaiterTerminal, ReadWakeResult,
    };

    const SESSION: u64 = 7;

    fn public(room: &str) -> ReadStateKey {
        ReadStateKey::PublicUnthreaded {
            room_id: room.to_owned(),
        }
    }

    fn thread(room: &str, root: &str) -> ReadStateKey {
        ReadStateKey::ThreadRead {
            room_id: room.to_owned(),
            root_event_id: root.to_owned(),
        }
    }

    fn fully_read(room: &str) -> ReadStateKey {
        ReadStateKey::FullyReadAndPrivateUnthreaded {
            room_id: room.to_owned(),
        }
    }

    fn unordered(event: &str) -> ReadTarget {
        ReadTarget::new(event.to_owned())
    }

    fn positioned(event: &str, generation: u64, rank: u64) -> ReadTarget {
        ReadTarget::with_position(
            event.to_owned(),
            ReadPositionEvidence {
                generation: generation.into(),
                rank,
            },
        )
    }

    fn waiter(value: u64) -> ReadWaiterId {
        ReadWaiterId::new(value)
    }

    fn failed() -> ReadNetworkOutcome {
        ReadNetworkOutcome::Failed(ReadNetworkFailure::new(ReadStateFailureKind::Sdk))
    }

    #[test]
    fn read_state_keys_keep_public_thread_and_fully_read_bundles_distinct() {
        let room = "synthetic-room";
        let keys = HashSet::from([
            public(room),
            thread(room, "synthetic-root-a"),
            thread(room, "synthetic-root-b"),
            fully_read(room),
        ]);

        assert_eq!(keys.len(), 4);
    }

    #[test]
    fn position_evidence_coalesces_to_the_newest_candidate_and_keeps_waiters() {
        let key = public("synthetic-room");
        let mut engine = ReadStateEngine::new(SESSION);

        let first = engine.admit(
            SESSION,
            key.clone(),
            positioned("synthetic-event-10", 3, 10),
            waiter(1),
        );
        let newer = engine.admit(
            SESSION,
            key.clone(),
            positioned("synthetic-event-12", 3, 12),
            waiter(2),
        );
        let older = engine.admit(
            SESSION,
            key.clone(),
            positioned("synthetic-event-11", 3, 11),
            waiter(3),
        );

        assert_eq!(first.status(), ReadAdmissionStatus::Accepted);
        assert_eq!(engine.session_generation(), SESSION);
        assert_eq!(newer.status(), ReadAdmissionStatus::Accepted);
        assert_eq!(older.status(), ReadAdmissionStatus::Coalesced);
        assert_eq!(engine.candidate_count(&key), 1);
        assert_eq!(engine.waiter_count(&key), 3);
        assert!(engine.has_candidate(&key, "synthetic-event-12"));
    }

    #[test]
    fn unordered_latest_admission_is_the_only_desired_target() {
        let key = public("synthetic-room");
        let mut engine = ReadStateEngine::new(SESSION);
        engine.admit(
            SESSION,
            key.clone(),
            unordered("synthetic-event-a"),
            waiter(1),
        );
        engine.admit(
            SESSION,
            key.clone(),
            unordered("synthetic-event-b"),
            waiter(2),
        );

        assert_eq!(engine.candidate_count(&key), 1);
        assert_eq!(engine.waiter_count(&key), 2);
        assert!(!engine.has_candidate(&key, "synthetic-event-a"));
        assert!(engine.has_candidate(&key, "synthetic-event-b"));
    }

    #[test]
    fn new_visible_waiter_overtakes_restored_background_candidates() {
        let key = public("synthetic-room");
        let mut seed = ReadStateEngine::new(SESSION);
        seed.admit(
            SESSION,
            key.clone(),
            unordered("synthetic-restored-a"),
            waiter(1),
        );
        seed.admit(
            SESSION,
            key.clone(),
            unordered("synthetic-restored-b"),
            waiter(2),
        );
        let mut engine = ReadStateEngine::restore(SESSION, seed.persistence_snapshot())
            .expect("restore valid background candidates");
        engine.admit(
            SESSION,
            key.clone(),
            unordered("synthetic-visible"),
            waiter(3),
        );

        let ReadWakeResult::Start(operation) = engine.wake(&key) else {
            panic!("visible waiter must start");
        };
        assert_eq!(operation.target().event_id(), "synthetic-visible");
    }

    #[test]
    fn failed_unordered_background_target_is_retained_for_retry() {
        let key = public("synthetic-room");
        let mut seed = ReadStateEngine::new(SESSION);
        seed.admit(
            SESSION,
            key.clone(),
            unordered("synthetic-background-a"),
            waiter(1),
        );
        seed.admit(
            SESSION,
            key.clone(),
            unordered("synthetic-background-b"),
            waiter(2),
        );
        let mut engine = ReadStateEngine::restore(SESSION, seed.persistence_snapshot())
            .expect("restore valid background target");

        let ReadWakeResult::Start(first) = engine.wake(&key) else {
            panic!("background target must start");
        };
        assert_eq!(first.target().event_id(), "synthetic-background-b");
        engine.complete(&key, first.fence(), failed());

        assert_eq!(engine.candidate_count(&key), 1);
        assert!(engine.has_candidate(&key, "synthetic-background-b"));
    }

    #[test]
    fn candidates_from_different_position_generations_are_not_ordered() {
        let key = public("synthetic-room");
        let mut engine = ReadStateEngine::new(SESSION);
        engine.admit(
            SESSION,
            key.clone(),
            positioned("synthetic-event-a", 4, 100),
            waiter(1),
        );
        engine.admit(
            SESSION,
            key.clone(),
            positioned("synthetic-event-b", 5, 1),
            waiter(2),
        );

        assert_eq!(engine.candidate_count(&key), 1);
        assert!(engine.has_candidate(&key, "synthetic-event-b"));
    }

    #[test]
    fn key_limit_rejects_the_129th_key_without_eviction() {
        let mut engine = ReadStateEngine::new(SESSION);
        for index in 0..READ_STATE_OUTBOX_ENTRY_LIMIT {
            let key = public(&format!("synthetic-room-{index}"));
            let result = engine.admit(
                SESSION,
                key,
                unordered("synthetic-event"),
                waiter(index as u64),
            );
            assert_ne!(
                result.status(),
                ReadAdmissionStatus::Rejected(ReadAdmissionRejection::CandidateCapacity)
            );
        }

        let rejected = engine.admit(
            SESSION,
            public("synthetic-room-over-capacity"),
            unordered("synthetic-event-over-capacity"),
            waiter(1000),
        );

        assert_eq!(
            rejected.status(),
            ReadAdmissionStatus::Rejected(ReadAdmissionRejection::CandidateCapacity)
        );
        assert_eq!(
            engine.persistence_snapshot().entry_count(),
            READ_STATE_OUTBOX_ENTRY_LIMIT
        );
    }

    #[test]
    fn waiter_limit_rejects_the_thirty_third_request_without_eviction() {
        let key = public("synthetic-room");
        let mut engine = ReadStateEngine::new(SESSION);
        for index in 0..READ_STATE_WAITER_LIMIT {
            let result = engine.admit(
                SESSION,
                key.clone(),
                unordered("synthetic-event"),
                waiter(index as u64),
            );
            assert_ne!(
                result.status(),
                ReadAdmissionStatus::Rejected(ReadAdmissionRejection::WaiterCapacity)
            );
        }

        let rejected = engine.admit(
            SESSION,
            key.clone(),
            unordered("synthetic-event"),
            waiter(100),
        );

        assert_eq!(
            rejected.status(),
            ReadAdmissionStatus::Rejected(ReadAdmissionRejection::WaiterCapacity)
        );
        assert_eq!(engine.candidate_count(&key), 1);
        assert_eq!(engine.waiter_count(&key), READ_STATE_WAITER_LIMIT);
    }

    #[test]
    fn newer_candidate_supersedes_active_and_stale_completion_cannot_regress() {
        let key = public("synthetic-room");
        let mut engine = ReadStateEngine::new(SESSION);
        engine.admit(
            SESSION,
            key.clone(),
            positioned("synthetic-event-old", 8, 10),
            waiter(1),
        );
        let old_operation = match engine.wake(&key) {
            ReadWakeResult::Start(operation) => operation,
            other => panic!("expected a start, got {other:?}"),
        };
        assert_eq!(old_operation.key(), &key);
        assert_eq!(old_operation.target().event_id(), "synthetic-event-old");
        assert_eq!(
            old_operation.target().position(),
            Some(ReadPositionEvidence {
                generation: 8,
                rank: 10,
            })
        );
        assert_eq!(old_operation.fence().session_generation(), SESSION);

        let admission = engine.admit(
            SESSION,
            key.clone(),
            positioned("synthetic-event-new", 8, 20),
            waiter(2),
        );

        assert_eq!(
            admission.superseded_operation(),
            Some(old_operation.fence())
        );
        assert_eq!(engine.candidate_count(&key), 1);
        assert_eq!(engine.waiter_count(&key), 2);
        assert!(engine.has_candidate(&key, "synthetic-event-new"));
        assert_eq!(engine.active_operation(&key), Some(old_operation.fence()));

        let stale = engine.complete(&key, old_operation.fence(), ReadNetworkOutcome::Succeeded);
        assert_eq!(
            stale.disposition(),
            ReadCompletionDisposition::StaleDiscarded
        );
        assert!(stale.settlements().is_empty());
        assert!(engine.has_candidate(&key, "synthetic-event-new"));
        assert_eq!(engine.waiter_count(&key), 2);

        let new_operation = match engine.wake(&key) {
            ReadWakeResult::Start(operation) => operation,
            other => panic!("expected a replacement start, got {other:?}"),
        };
        assert!(
            new_operation.fence().operation_generation()
                > old_operation.fence().operation_generation()
        );
    }

    #[test]
    fn timeout_and_failure_settle_waiters_but_retain_desired_for_retry() {
        let key = public("synthetic-room");
        let mut engine = ReadStateEngine::new(SESSION);
        engine.admit(
            SESSION,
            key.clone(),
            unordered("synthetic-event"),
            waiter(1),
        );
        let first = match engine.wake(&key) {
            ReadWakeResult::Start(operation) => operation,
            other => panic!("expected a start, got {other:?}"),
        };
        let timed_out = engine.complete(&key, first.fence(), ReadNetworkOutcome::TimedOut);

        assert_eq!(timed_out.disposition(), ReadCompletionDisposition::TimedOut);
        assert_eq!(timed_out.settlements().len(), 1);
        assert_eq!(timed_out.settlements()[0].waiter().get(), 1);
        assert_eq!(
            timed_out.settlements()[0].terminal(),
            ReadWaiterTerminal::TimedOut
        );
        assert_eq!(engine.candidate_count(&key), 1);
        assert_eq!(engine.waiter_count(&key), 0);

        let second = match engine.wake(&key) {
            ReadWakeResult::Start(operation) => operation,
            other => panic!("expected a retry start, got {other:?}"),
        };
        let failed = engine.complete(&key, second.fence(), failed());

        assert_eq!(failed.disposition(), ReadCompletionDisposition::Failed);
        assert!(failed.settlements().is_empty());
        assert_eq!(engine.candidate_count(&key), 1);
        assert!(matches!(engine.wake(&key), ReadWakeResult::Start(_)));
    }

    #[test]
    fn duplicate_wake_does_not_allocate_another_operation() {
        let key = public("synthetic-room");
        let mut engine = ReadStateEngine::new(SESSION);
        engine.admit(
            SESSION,
            key.clone(),
            unordered("synthetic-event"),
            waiter(1),
        );
        let first = match engine.wake(&key) {
            ReadWakeResult::Start(operation) => operation,
            other => panic!("expected a start, got {other:?}"),
        };

        assert_eq!(engine.wake(&key), ReadWakeResult::AlreadyActive);
        assert_eq!(engine.active_operation(&key), Some(first.fence()));
        assert_eq!(
            engine.last_operation_generation(),
            first.fence().operation_generation()
        );
    }

    #[test]
    fn session_and_operation_generations_fence_stale_input() {
        let key = public("synthetic-room");
        let mut engine = ReadStateEngine::new(SESSION);
        let stale_admission = engine.admit(
            SESSION - 1,
            key.clone(),
            unordered("synthetic-event"),
            waiter(1),
        );
        assert_eq!(
            stale_admission.status(),
            ReadAdmissionStatus::Rejected(ReadAdmissionRejection::StaleSession)
        );
        assert_eq!(engine.candidate_count(&key), 0);

        engine.admit(
            SESSION,
            key.clone(),
            unordered("synthetic-event"),
            waiter(2),
        );
        let operation = match engine.wake(&key) {
            ReadWakeResult::Start(operation) => operation,
            other => panic!("expected a start, got {other:?}"),
        };
        let stale_fence =
            ReadOperationFence::new(SESSION + 1, operation.fence().operation_generation());
        let stale = engine.complete(&key, stale_fence, ReadNetworkOutcome::Succeeded);

        assert_eq!(
            stale.disposition(),
            ReadCompletionDisposition::StaleDiscarded
        );
        assert_eq!(engine.active_operation(&key), Some(operation.fence()));
        assert_eq!(engine.candidate_count(&key), 1);
        assert_eq!(engine.waiter_count(&key), 1);
    }

    #[test]
    fn successful_newer_candidate_settles_dominated_waiters_exactly_once() {
        let key = public("synthetic-room");
        let mut engine = ReadStateEngine::new(SESSION);
        engine.admit(
            SESSION,
            key.clone(),
            positioned("synthetic-event-old", 9, 5),
            waiter(1),
        );
        engine.admit(
            SESSION,
            key.clone(),
            positioned("synthetic-event-new", 9, 6),
            waiter(2),
        );
        let operation = match engine.wake(&key) {
            ReadWakeResult::Start(operation) => operation,
            other => panic!("expected a start, got {other:?}"),
        };
        let succeeded = engine.complete(&key, operation.fence(), ReadNetworkOutcome::Succeeded);

        assert_eq!(
            succeeded.disposition(),
            ReadCompletionDisposition::Succeeded
        );
        assert_eq!(succeeded.settlements().len(), 2);
        assert!(
            succeeded
                .settlements()
                .iter()
                .all(|settlement| settlement.terminal() == ReadWaiterTerminal::Converged)
        );
        assert_eq!(engine.candidate_count(&key), 0);
        assert_eq!(engine.waiter_count(&key), 0);

        let duplicate = engine.complete(&key, operation.fence(), ReadNetworkOutcome::Succeeded);
        assert_eq!(
            duplicate.disposition(),
            ReadCompletionDisposition::StaleDiscarded
        );
        assert!(duplicate.settlements().is_empty());
    }

    #[test]
    fn public_thread_and_fully_read_keys_can_each_own_one_active_operation() {
        let mut engine = ReadStateEngine::new(SESSION);
        let keys = [
            public("synthetic-room"),
            thread("synthetic-room", "synthetic-root"),
            fully_read("synthetic-room"),
        ];
        for (index, key) in keys.iter().enumerate() {
            engine.admit(
                SESSION,
                key.clone(),
                unordered(&format!("synthetic-event-{index}")),
                waiter(index as u64),
            );
        }

        let starts = keys
            .iter()
            .filter(|key| matches!(engine.wake(key), ReadWakeResult::Start(_)))
            .count();

        assert_eq!(starts, 3);
        assert_eq!(engine.active_operation_count(), 3);
    }

    #[test]
    fn persistence_snapshot_restores_only_desired_targets_without_waiters_or_positions() {
        let public_key = public("secret-room");
        let thread_key = thread("secret-room", "secret-root");
        let mut engine = ReadStateEngine::new(SESSION);
        engine.admit(
            SESSION,
            public_key.clone(),
            positioned("secret-public-event", 12, 8),
            waiter(1),
        );
        engine.admit(
            SESSION,
            thread_key.clone(),
            unordered("secret-thread-event"),
            waiter(2),
        );

        let snapshot = engine.persistence_snapshot();
        assert_eq!(snapshot.entry_count(), 2);
        let rendered = format!("{snapshot:?}");
        assert!(!rendered.contains("secret-room"));
        assert!(!rendered.contains("secret-root"));
        assert!(!rendered.contains("secret-public-event"));

        let restored = ReadStateEngine::restore(SESSION + 1, snapshot)
            .expect("bounded manager snapshot must restore");
        assert_eq!(restored.session_generation(), SESSION + 1);
        assert!(restored.has_candidate(&public_key, "secret-public-event"));
        assert!(restored.has_candidate(&thread_key, "secret-thread-event"));
        assert_eq!(restored.waiter_count(&public_key), 0);
        assert_eq!(restored.waiter_count(&thread_key), 0);
        assert_eq!(
            match restored.keys.get(&public_key) {
                Some(state) => state
                    .desired
                    .as_ref()
                    .expect("restored desired target")
                    .target
                    .position(),
                None => panic!("public desired target must restore"),
            },
            None,
            "the actor-owned position index is never serialized"
        );
    }

    #[test]
    fn persisted_receipt_policy_filters_public_and_thread_but_keeps_private_fully_read() {
        let public_key = public("policy-room");
        let thread_key = thread("policy-room", "policy-root");
        let fully_read_key = fully_read("policy-room");
        let mut engine = ReadStateEngine::new(SESSION);
        for (index, key) in [
            public_key.clone(),
            thread_key.clone(),
            fully_read_key.clone(),
        ]
        .into_iter()
        .enumerate()
        {
            engine.admit(
                SESSION,
                key,
                unordered(&format!("policy-event-{index}")),
                waiter(index as u64),
            );
        }

        let mut snapshot = engine.persistence_snapshot();
        assert!(snapshot.apply_receipt_policy(false));
        assert_eq!(snapshot.entry_count(), 1);
        assert_eq!(snapshot.entries()[0].key(), &fully_read_key);
        assert!(!snapshot.apply_receipt_policy(false));
        assert!(!snapshot.apply_receipt_policy(true));
    }

    #[test]
    fn authoritative_server_ahead_observation_converges_waiters_and_cancels_stale_worker() {
        let key = public("synthetic-room");
        let mut engine = ReadStateEngine::new(SESSION);
        engine.admit(
            SESSION,
            key.clone(),
            positioned("synthetic-old", 14, 4),
            waiter(1),
        );
        engine.admit(
            SESSION,
            key.clone(),
            positioned("synthetic-new", 14, 5),
            waiter(2),
        );
        let operation = match engine.wake(&key) {
            ReadWakeResult::Start(operation) => operation,
            other => panic!("expected an active retry, got {other:?}"),
        };

        let reconciliation = engine.confirm_authoritative(
            SESSION,
            &key,
            positioned("synthetic-server-ahead", 14, 6),
        );

        assert_eq!(
            reconciliation.superseded_operation(),
            Some(operation.fence())
        );
        assert_eq!(reconciliation.settlements().len(), 2);
        assert!(
            reconciliation
                .settlements()
                .iter()
                .all(|settlement| settlement.terminal() == ReadWaiterTerminal::Converged)
        );
        assert_eq!(engine.candidate_count(&key), 0);
        assert_eq!(engine.active_operation(&key), Some(operation.fence()));

        let stale = engine.complete(&key, operation.fence(), ReadNetworkOutcome::Succeeded);
        assert_eq!(
            stale.disposition(),
            ReadCompletionDisposition::StaleDiscarded
        );
        assert!(stale.settlements().is_empty());
    }

    #[test]
    fn diagnostic_views_and_debug_output_do_not_expose_identifiers() {
        let key = public("secret-room");
        let mut engine = ReadStateEngine::new(SESSION);
        let admission = engine.admit(SESSION, key.clone(), unordered("secret-event"), waiter(1));
        let operation = match engine.wake(&key) {
            ReadWakeResult::Start(operation) => operation,
            other => panic!("expected a start, got {other:?}"),
        };
        let completion = engine.complete(&key, operation.fence(), ReadNetworkOutcome::TimedOut);

        for rendered in [
            format!("{:?}", admission.diagnostic()),
            format!("{:?}", completion.diagnostic()),
            format!("{key:?}"),
            format!("{operation:?}"),
        ] {
            assert!(!rendered.contains("secret-room"));
            assert!(!rendered.contains("secret-event"));
        }
    }

    #[test]
    fn unordered_latest_admission_replaces_the_older_desired_target() {
        let key = public("synthetic-room");
        let mut engine = ReadStateEngine::new(SESSION);
        engine.admit(
            SESSION,
            key.clone(),
            unordered("synthetic-event-a"),
            waiter(1),
        );
        engine.admit(
            SESSION,
            key.clone(),
            unordered("synthetic-event-b"),
            waiter(2),
        );

        assert_eq!(engine.candidate_count(&key), 1);
        assert_eq!(engine.waiter_count(&key), 2);
        assert!(!engine.has_candidate(&key, "synthetic-event-a"));
        assert!(engine.has_candidate(&key, "synthetic-event-b"));
    }

    #[test]
    fn persistence_snapshot_writes_only_the_newest_unordered_target() {
        let key = public("synthetic-room");
        let mut engine = ReadStateEngine::new(SESSION);
        engine.admit(
            SESSION,
            key.clone(),
            unordered("synthetic-event-a"),
            waiter(1),
        );
        engine.admit(SESSION, key, unordered("synthetic-event-b"), waiter(2));

        let snapshot = engine.persistence_snapshot();
        assert_eq!(snapshot.candidate_count(), 1);
        assert_eq!(snapshot.entries()[0].event_ids(), ["synthetic-event-b"]);
    }

    #[test]
    fn failed_latest_target_is_retained_without_replaying_an_older_target() {
        let key = public("synthetic-room");
        let mut engine = ReadStateEngine::new(SESSION);
        engine.admit(
            SESSION,
            key.clone(),
            unordered("synthetic-event-a"),
            waiter(1),
        );
        engine.admit(
            SESSION,
            key.clone(),
            unordered("synthetic-event-b"),
            waiter(2),
        );

        let operation = match engine.wake(&key) {
            ReadWakeResult::Start(operation) => operation,
            other => panic!("expected a start, got {other:?}"),
        };
        assert_eq!(operation.target().event_id(), "synthetic-event-b");
        engine.complete(&key, operation.fence(), failed());

        assert_eq!(engine.candidate_count(&key), 1);
        assert!(engine.has_candidate(&key, "synthetic-event-b"));
        assert!(!engine.has_candidate(&key, "synthetic-event-a"));
    }
}
