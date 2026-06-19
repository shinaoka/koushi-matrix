//! Core identity types (overview.md "Core identity types are concrete and stable").

use serde::{Deserialize, Serialize};

/// Assigned by the runtime to each attached consumer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct RuntimeConnectionId(pub u64);

/// Unique on the shared event stream: connection id plus a sequence the
/// attached connection allocates. Callers never hand-build these; see
/// [`crate::CoreConnection::next_request_id`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct RequestId {
    pub connection_id: RuntimeConnectionId,
    pub sequence: u64,
}

/// Stable key for one account/device runtime.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct AccountKey(pub String);

/// Addresses one timeline. Always includes the account so late events from a
/// previous account switch can be rejected.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct TimelineKey {
    pub account_key: AccountKey,
    pub kind: TimelineKind,
}

impl TimelineKey {
    pub fn room(account_key: AccountKey, room_id: impl Into<String>) -> Self {
        Self {
            account_key,
            kind: TimelineKind::Room {
                room_id: room_id.into(),
            },
        }
    }

    pub fn room_id(&self) -> &str {
        match &self.kind {
            TimelineKind::Room { room_id }
            | TimelineKind::Thread { room_id, .. }
            | TimelineKind::Focused { room_id, .. } => room_id,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum TimelineKind {
    Room {
        room_id: String,
    },
    Thread {
        room_id: String,
        root_event_id: String,
    },
    Focused {
        room_id: String,
        event_id: String,
    },
}

/// Monotonic per timeline subscription; bumped on every reset/resync so the
/// UI can discard diffs from older generations.
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
pub struct TimelineGeneration(pub u64);

/// FIFO position of a diff batch within a generation.
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
pub struct TimelineBatchId(pub u64);
