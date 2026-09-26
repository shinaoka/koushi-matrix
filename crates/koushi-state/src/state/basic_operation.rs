use serde::{Deserialize, Serialize};
use std::fmt;

use super::OperationFailureKind;

/// In-flight status of a basic room/space operation, modeled as a guarded state
/// machine (see `docs/architecture/state-machine.md`): only `Idle` accepts a new
/// request, and a pending operation can only be settled by a completion whose
/// `request_id` matches the one carried by the in-flight state. This mirrors the
/// composer's pending-transaction rule and search's `request_id` correlation.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BasicOperationState {
    #[default]
    Idle,
    CreatingRoom {
        request_id: u64,
        name: String,
    },
    CreatingSpace {
        request_id: u64,
        name: String,
    },
    LinkingSpaceChild {
        request_id: u64,
        space_id: String,
        child_room_id: String,
    },
}

impl BasicOperationState {
    /// Correlation id of the in-flight operation, or `None` when `Idle`.
    pub fn request_id(&self) -> Option<u64> {
        match self {
            BasicOperationState::Idle => None,
            BasicOperationState::CreatingRoom { request_id, .. }
            | BasicOperationState::CreatingSpace { request_id, .. }
            | BasicOperationState::LinkingSpaceChild { request_id, .. } => Some(*request_id),
        }
    }

    /// Whether no basic operation is currently in flight.
    pub fn is_idle(&self) -> bool {
        matches!(self, BasicOperationState::Idle)
    }
}

/// A requested basic operation: user intent, kept distinct from the resulting
/// state. The reducer pairs this with a correlation `request_id` to derive the
/// in-flight `BasicOperationState`; a request never names the target state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BasicOperationRequest {
    CreateRoom {
        name: String,
    },
    CreateSpace {
        name: String,
    },
    LinkSpaceChild {
        space_id: String,
        child_room_id: String,
    },
}

/// Latest settlement of parent-side Space child linking per `(Space, room)`
/// pair in the current account session (#1007).
///
/// Linking runs inside the basic-operation slot, either as its own
/// `LinkingSpaceChild` operation or as the parent-Space step of
/// `CreatingRoom`. A settlement is admitted only while its `request_id` is the
/// in-flight basic operation, so a stale or duplicate completion cannot
/// replace a newer result. `Linked` records the homeserver's acceptance of the
/// parent-side `m.space.child`, which the SDK room cache can lag behind.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SpaceChildLinkResults {
    pub entries: Vec<SpaceChildLinkResult>,
}

impl SpaceChildLinkResults {
    /// The latest settlement recorded for one pair.
    pub fn latest(&self, space_id: &str, child_room_id: &str) -> Option<&SpaceChildLinkResult> {
        self.entries
            .iter()
            .find(|entry| entry.space_id == space_id && entry.child_room_id == child_room_id)
    }

    pub(crate) fn record(&mut self, result: SpaceChildLinkResult) {
        self.entries.retain(|entry| {
            entry.space_id != result.space_id || entry.child_room_id != result.child_room_id
        });
        self.entries.push(result);
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct SpaceChildLinkResult {
    pub request_id: u64,
    pub space_id: String,
    pub child_room_id: String,
    pub outcome: SpaceChildLinkOutcome,
}

impl fmt::Debug for SpaceChildLinkResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SpaceChildLinkResult")
            .field("request_id", &self.request_id)
            .field("space_id", &"[redacted]")
            .field("child_room_id", &"[redacted]")
            .field("outcome", &self.outcome)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SpaceChildLinkOutcome {
    Linked,
    Failed { reason: OperationFailureKind },
}
