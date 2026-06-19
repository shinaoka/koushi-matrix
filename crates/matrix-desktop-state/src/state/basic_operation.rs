use serde::{Deserialize, Serialize};

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
