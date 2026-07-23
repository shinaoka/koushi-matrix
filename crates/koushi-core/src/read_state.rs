//! Pure manager-owned state for read-receipt and fully-read convergence.
//!
//! Matrix identifiers stay in [`ReadStateKey`], [`ReadTarget`], and
//! [`ReadOperation`], which are internal work values with redacted `Debug`
//! implementations. Admission and completion diagnostics expose only closed
//! enums, generation/count values, and waiter counts.

use std::collections::HashMap;
use std::fmt;

pub(crate) const READ_STATE_CANDIDATE_LIMIT: usize = 8;
pub(crate) const READ_STATE_WAITER_LIMIT: usize = 32;

#[derive(Clone, Eq, Hash, PartialEq)]
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
    pub(crate) generation: u64,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
pub(crate) enum ReadNetworkOutcome {
    Succeeded,
    Failed,
    TimedOut,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReadCompletionDisposition {
    Succeeded,
    Failed,
    TimedOut,
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
    },
    TimedOut {
        settled_waiter_count: usize,
        remaining_candidate_count: usize,
        remaining_waiter_count: usize,
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
            },
            ReadCompletionDisposition::TimedOut => ReadCompletionDiagnostic::TimedOut {
                settled_waiter_count,
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
    candidates: Vec<ReadCandidate>,
    active: Option<ActiveReadOperation>,
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

        let state = self.keys.entry(key).or_default();
        if state
            .candidates
            .iter()
            .any(|candidate| candidate.waiters.contains(&waiter))
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

        let coalesces_without_capacity = state.candidates.iter().any(|candidate| {
            candidate.target.event_id == target.event_id
                || dominates(&candidate.target, &target)
                || dominates(&target, &candidate.target)
        });
        if state.candidates.len() >= READ_STATE_CANDIDATE_LIMIT && !coalesces_without_capacity {
            return admission_result(
                ReadAdmissionStatus::Rejected(ReadAdmissionRejection::CandidateCapacity),
                None,
                state,
            );
        }

        let coalesced = state.candidates.iter().any(|candidate| {
            candidate.target.event_id == target.event_id || dominates(&candidate.target, &target)
        });
        state.candidates.push(ReadCandidate {
            target,
            waiters: vec![waiter],
        });
        let superseded_operation = coalesce_candidates(state);

        admission_result(
            if coalesced {
                ReadAdmissionStatus::Coalesced
            } else {
                ReadAdmissionStatus::Accepted
            },
            superseded_operation,
            state,
        )
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
        let Some(candidate) = state
            .candidates
            .iter_mut()
            .find(|candidate| candidate.target.event_id == event_id)
        else {
            return ReadEvidenceResult {
                status: ReadEvidenceStatus::UnknownTarget,
                superseded_operation: None,
                candidate_count: state.candidates.len(),
                waiter_count: waiter_count(state),
            };
        };

        if candidate
            .target
            .position
            .is_some_and(|known| evidence_is_older(evidence, known))
        {
            return ReadEvidenceResult {
                status: ReadEvidenceStatus::IgnoredOlderEvidence,
                superseded_operation: None,
                candidate_count: state.candidates.len(),
                waiter_count: waiter_count(state),
            };
        }

        candidate.target.position = Some(evidence);
        let superseded_operation = coalesce_candidates(state);
        ReadEvidenceResult {
            status: ReadEvidenceStatus::Updated,
            superseded_operation,
            candidate_count: state.candidates.len(),
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
        let Some(candidate) = state.candidates.first() else {
            return ReadWakeResult::NoDesired;
        };
        let target = candidate.target.clone();
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
            };
        };
        let matches_active = state
            .active
            .as_ref()
            .is_some_and(|active| active.fence == fence);
        if !matches_active {
            return completion_result(ReadCompletionDisposition::StaleDiscarded, Vec::new(), state);
        }
        let active = state
            .active
            .take()
            .expect("matching read operation must remain active");
        let Some(active_index) = state
            .candidates
            .iter()
            .position(|candidate| candidate.target.event_id == active.event_id)
        else {
            return completion_result(ReadCompletionDisposition::StaleDiscarded, Vec::new(), state);
        };

        let (disposition, settlements) = match outcome {
            ReadNetworkOutcome::Succeeded => {
                let confirmed = state.candidates[active_index].target.clone();
                let mut settled_waiters = Vec::new();
                let mut index = 0;
                while index < state.candidates.len() {
                    if same_target_or_dominated(&confirmed, &state.candidates[index].target) {
                        let candidate = state.candidates.remove(index);
                        settled_waiters.extend(candidate.waiters.into_iter().map(|waiter| {
                            ReadWaiterSettlement {
                                waiter,
                                terminal: ReadWaiterTerminal::Converged,
                            }
                        }));
                    } else {
                        index += 1;
                    }
                }
                (ReadCompletionDisposition::Succeeded, settled_waiters)
            }
            ReadNetworkOutcome::Failed | ReadNetworkOutcome::TimedOut => {
                let terminal = match outcome {
                    ReadNetworkOutcome::Failed => ReadWaiterTerminal::Failed,
                    ReadNetworkOutcome::TimedOut => ReadWaiterTerminal::TimedOut,
                    ReadNetworkOutcome::Succeeded => unreachable!(),
                };
                let settlements = state.candidates[active_index]
                    .waiters
                    .drain(..)
                    .map(|waiter| ReadWaiterSettlement { waiter, terminal })
                    .collect();
                (
                    match outcome {
                        ReadNetworkOutcome::Failed => ReadCompletionDisposition::Failed,
                        ReadNetworkOutcome::TimedOut => ReadCompletionDisposition::TimedOut,
                        ReadNetworkOutcome::Succeeded => unreachable!(),
                    },
                    settlements,
                )
            }
        };
        let result = completion_result(disposition, settlements, state);
        if state.candidates.is_empty() {
            self.keys.remove(key);
        }
        result
    }

    pub(crate) fn candidate_count(&self, key: &ReadStateKey) -> usize {
        self.keys.get(key).map_or(0, |state| state.candidates.len())
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
        self.keys.get(key).is_some_and(|state| {
            state
                .candidates
                .iter()
                .any(|candidate| candidate.target.event_id == event_id)
        })
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
        }
    }

    fn counts(&self, key: &ReadStateKey) -> (usize, usize) {
        self.keys.get(key).map_or((0, 0), |state| {
            (state.candidates.len(), waiter_count(state))
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
        candidate_count: state.candidates.len(),
        waiter_count: waiter_count(state),
    }
}

fn completion_result(
    disposition: ReadCompletionDisposition,
    settlements: Vec<ReadWaiterSettlement>,
    state: &ReadKeyState,
) -> ReadCompletionResult {
    ReadCompletionResult {
        disposition,
        settlements,
        remaining_candidate_count: state.candidates.len(),
        remaining_waiter_count: waiter_count(state),
    }
}

fn waiter_count(state: &ReadKeyState) -> usize {
    state
        .candidates
        .iter()
        .map(|candidate| candidate.waiters.len())
        .sum()
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

fn coalesce_candidates(state: &mut ReadKeyState) -> Option<ReadOperationFence> {
    let mut superseded_operation = None;
    loop {
        let mut pair = None;
        'search: for left_index in 0..state.candidates.len() {
            for right_index in (left_index + 1)..state.candidates.len() {
                let left = &state.candidates[left_index].target;
                let right = &state.candidates[right_index].target;
                if left.event_id == right.event_id {
                    pair = Some((left_index, right_index, true));
                    break 'search;
                }
                if dominates(left, right) {
                    pair = Some((left_index, right_index, false));
                    break 'search;
                }
                if dominates(right, left) {
                    pair = Some((right_index, left_index, false));
                    break 'search;
                }
            }
        }

        let Some((winner_index, loser_index, same_event)) = pair else {
            break;
        };
        let winner_event_id = state.candidates[winner_index].target.event_id.clone();
        let loser_event_id = state.candidates[loser_index].target.event_id.clone();
        let merged_position = same_event.then(|| {
            preferred_same_event_position(
                state.candidates[winner_index].target.position,
                state.candidates[loser_index].target.position,
            )
        });
        let loser = state.candidates.remove(loser_index);
        let adjusted_winner_index = if loser_index < winner_index {
            winner_index - 1
        } else {
            winner_index
        };
        let winner = &mut state.candidates[adjusted_winner_index];
        if let Some(position) = merged_position {
            winner.target.position = position;
        }
        winner.waiters.extend(loser.waiters);

        let active_is_loser = state
            .active
            .as_ref()
            .is_some_and(|active| active.event_id == loser_event_id);
        if active_is_loser && winner_event_id != loser_event_id {
            superseded_operation = state.active.take().map(|active| active.fence);
        }
    }
    superseded_operation
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{
        READ_STATE_CANDIDATE_LIMIT, READ_STATE_WAITER_LIMIT, ReadAdmissionRejection,
        ReadAdmissionStatus, ReadCompletionDisposition, ReadNetworkOutcome, ReadOperationFence,
        ReadPositionEvidence, ReadStateEngine, ReadStateKey, ReadTarget, ReadWaiterId,
        ReadWaiterTerminal, ReadWakeResult,
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
        ReadTarget::with_position(event.to_owned(), ReadPositionEvidence { generation, rank })
    }

    fn waiter(value: u64) -> ReadWaiterId {
        ReadWaiterId::new(value)
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
    fn unordered_candidates_remain_distinct_until_position_evidence_orders_them() {
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

        assert_eq!(engine.candidate_count(&key), 2);

        let first = engine.observe_position(
            SESSION,
            &key,
            "synthetic-event-a",
            ReadPositionEvidence {
                generation: 4,
                rank: 20,
            },
        );
        let second = engine.observe_position(
            SESSION,
            &key,
            "synthetic-event-b",
            ReadPositionEvidence {
                generation: 4,
                rank: 21,
            },
        );

        assert!(first.updated());
        assert_eq!(first.status(), super::ReadEvidenceStatus::Updated);
        assert_eq!(first.superseded_operation(), None);
        assert!(second.updated());
        assert_eq!(second.candidate_count(), 1);
        assert_eq!(second.waiter_count(), 2);
        assert_eq!(engine.candidate_count(&key), 1);
        assert_eq!(engine.waiter_count(&key), 2);
        assert!(engine.has_candidate(&key, "synthetic-event-b"));
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

        assert_eq!(engine.candidate_count(&key), 2);
    }

    #[test]
    fn candidate_limit_rejects_the_ninth_unordered_target_without_eviction() {
        let key = public("synthetic-room");
        let mut engine = ReadStateEngine::new(SESSION);
        for index in 0..READ_STATE_CANDIDATE_LIMIT {
            let result = engine.admit(
                SESSION,
                key.clone(),
                unordered(&format!("synthetic-event-{index}")),
                waiter(index as u64),
            );
            assert_ne!(
                result.status(),
                ReadAdmissionStatus::Rejected(ReadAdmissionRejection::CandidateCapacity)
            );
        }

        let rejected = engine.admit(
            SESSION,
            key.clone(),
            unordered("synthetic-event-over-capacity"),
            waiter(100),
        );

        assert_eq!(
            rejected.status(),
            ReadAdmissionStatus::Rejected(ReadAdmissionRejection::CandidateCapacity)
        );
        assert_eq!(engine.candidate_count(&key), READ_STATE_CANDIDATE_LIMIT);
        assert_eq!(engine.waiter_count(&key), READ_STATE_CANDIDATE_LIMIT);
        for index in 0..READ_STATE_CANDIDATE_LIMIT {
            assert!(engine.has_candidate(&key, &format!("synthetic-event-{index}")));
        }
        assert!(!engine.has_candidate(&key, "synthetic-event-over-capacity"));
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
        assert_eq!(engine.active_operation(&key), None);

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
        let failed = engine.complete(&key, second.fence(), ReadNetworkOutcome::Failed);

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
}
