use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::errors::OperationFailureKind;
use super::settings::ThreadListOrder;
use super::timeline::{ComposerState, StagedUploadItem};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThreadOpenIntent {
    ExistingThread,
    NewThreadDraft,
    PinnedReply { event_id: String },
}

/// Scope used by the Threads panel. The reducer keeps the legacy `room_id`
/// snapshot field as a stable scope key (`home`, `space:<space_id>`, or the
/// room id) so older state consumers keep their correlation contract while
/// the actor can aggregate the resolved room ids in Rust.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ThreadsListScope {
    Room { room_id: String },
    Home,
    Space { space_id: String },
}

impl ThreadsListScope {
    pub fn scope_key(&self) -> String {
        match self {
            Self::Room { room_id } => room_id.clone(),
            Self::Home => "home".to_owned(),
            Self::Space { space_id } => format!("space:{space_id}"),
        }
    }

    pub fn from_scope_key(scope_key: &str) -> Self {
        if scope_key == "home" {
            Self::Home
        } else if let Some(space_id) = scope_key.strip_prefix("space:") {
            Self::Space {
                space_id: space_id.to_owned(),
            }
        } else {
            Self::Room {
                room_id: scope_key.to_owned(),
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[expect(
    clippy::large_enum_variant,
    reason = "pane state is held once and cloned rarely; a few hundred bytes does not justify boxing"
)]
pub enum ThreadPaneState {
    Closed,
    Opening {
        room_id: String,
        root_event_id: String,
        intent: ThreadOpenIntent,
    },
    Open {
        room_id: String,
        root_event_id: String,
        intent: ThreadOpenIntent,
        is_subscribed: bool,
        composer: ComposerState,
        staged_uploads: Vec<StagedUploadItem>,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ThreadAttentionState {
    #[default]
    Closed,
    Tracking {
        room_id: String,
        root_event_id: String,
        notification_count: u64,
        highlight_count: u64,
        live_event_marker_count: u64,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ThreadsListState {
    #[default]
    Closed,
    Loading {
        room_id: String,
        request_id: u64,
    },
    Open {
        room_id: String,
        request_id: u64,
        items: Vec<ThreadsListItem>,
        is_paginating: bool,
        end_reached: bool,
    },
    Failed {
        room_id: String,
        request_id: u64,
        failure_kind: OperationFailureKind,
    },
}

impl ThreadsListState {
    pub fn room_id(&self) -> Option<&str> {
        match self {
            Self::Closed => None,
            Self::Loading { room_id, .. }
            | Self::Open { room_id, .. }
            | Self::Failed { room_id, .. } => Some(room_id.as_str()),
        }
    }

    pub fn request_id(&self) -> Option<u64> {
        match self {
            Self::Closed => None,
            Self::Loading { request_id, .. }
            | Self::Open { request_id, .. }
            | Self::Failed { request_id, .. } => Some(*request_id),
        }
    }

    pub fn set_paginating(&mut self, value: bool) {
        if let Self::Open { is_paginating, .. } = self {
            *is_paginating = value;
        }
    }

    pub fn items(&self) -> &[ThreadsListItem] {
        match self {
            Self::Open { items, .. } => items,
            _ => &[],
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ThreadsListItem {
    pub room_id: String,
    pub root_event_id: String,
    pub root_sender: String,
    pub root_sender_label: Option<String>,
    pub root_body_preview: Option<String>,
    pub root_timestamp_ms: Option<u64>,
    pub latest_event_id: Option<String>,
    pub latest_sender: Option<String>,
    pub latest_sender_label: Option<String>,
    pub latest_body_preview: Option<String>,
    pub latest_timestamp_ms: Option<u64>,
    pub reply_count: u32,
    /// #1259: unread replies reported by the SDK thread cache for this root, the same
    /// value the room timeline chip shows.
    #[serde(default)]
    pub unread_count: u32,
}

/// #1238: one thread root's SDK thread-cache attention, as observed by the room
/// timeline actor.
///
/// `unread` and `highlight` are the thread cache's own `num_unread` /
/// `num_mentions`. The reducer sums these per room into
/// `RoomSummary.thread_unread_count` / `thread_highlight_count`, which the shared
/// badge helper maxes against the main-timeline counters; a root's value never
/// enters the main-only `RoomSummary.unread_count`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ThreadRootAttention {
    pub unread: u32,
    pub highlight: u32,
}

impl ThreadRootAttention {
    pub fn is_empty(self) -> bool {
        self.unread == 0 && self.highlight == 0
    }
}

/// Projection state for a root event which is outside the Room timeline's
/// canonical loaded window. This is deliberately separate from
/// [`ThreadsListState`]: opening/paginating the Threads panel must never
/// influence room-timeline root hydration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ThreadRootProjectionStatus {
    Pending {
        activity_event_id: String,
        activity_timestamp_ms: Option<u64>,
    },
    Ready {
        activity_event_id: String,
        activity_timestamp_ms: Option<u64>,
    },
    Failed {
        activity_event_id: String,
        activity_timestamp_ms: Option<u64>,
        failure_kind: OperationFailureKind,
    },
}

/// Rust-owned record of bounded root hydration attempts, keyed by the exact
/// `(room_id, root_event_id)` pair. Failed entries are terminal for this room
/// timeline lifetime, so repeated reply diffs cannot start a fetch loop.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ThreadRootProjectionState {
    entries: BTreeMap<(String, String), ThreadRootProjectionStatus>,
}

impl ThreadRootProjectionState {
    pub fn get(&self, room_id: &str, root_event_id: &str) -> Option<&ThreadRootProjectionStatus> {
        self.entries
            .get(&(room_id.to_owned(), root_event_id.to_owned()))
    }

    pub fn observe(
        &mut self,
        room_id: String,
        root_event_id: String,
        activity_event_id: String,
        activity_timestamp_ms: Option<u64>,
    ) -> bool {
        let key = (room_id, root_event_id);
        if let Some(existing) = self.entries.get(&key).cloned() {
            let (existing_activity_event_id, existing_activity_timestamp_ms) = match &existing {
                ThreadRootProjectionStatus::Pending {
                    activity_event_id,
                    activity_timestamp_ms,
                }
                | ThreadRootProjectionStatus::Ready {
                    activity_event_id,
                    activity_timestamp_ms,
                }
                | ThreadRootProjectionStatus::Failed {
                    activity_event_id,
                    activity_timestamp_ms,
                    ..
                } => (activity_event_id, *activity_timestamp_ms),
            };
            if !thread_root_projection_activity_is_newer(
                &activity_event_id,
                activity_timestamp_ms,
                existing_activity_event_id,
                existing_activity_timestamp_ms,
            ) {
                return false;
            }
            let updated = match &existing {
                ThreadRootProjectionStatus::Pending { .. } => ThreadRootProjectionStatus::Pending {
                    activity_event_id,
                    activity_timestamp_ms,
                },
                ThreadRootProjectionStatus::Ready { .. } => ThreadRootProjectionStatus::Ready {
                    activity_event_id,
                    activity_timestamp_ms,
                },
                ThreadRootProjectionStatus::Failed { failure_kind, .. } => {
                    ThreadRootProjectionStatus::Failed {
                        activity_event_id,
                        activity_timestamp_ms,
                        failure_kind: *failure_kind,
                    }
                }
            };
            self.entries.insert(key, updated);
            return true;
        }
        self.entries.insert(
            key,
            ThreadRootProjectionStatus::Pending {
                activity_event_id,
                activity_timestamp_ms,
            },
        );
        true
    }

    pub fn mark_ready(
        &mut self,
        room_id: String,
        root_event_id: String,
        activity_event_id: String,
        activity_timestamp_ms: Option<u64>,
    ) -> bool {
        let next = ThreadRootProjectionStatus::Ready {
            activity_event_id,
            activity_timestamp_ms,
        };
        let changed = self.entries.get(&(room_id.clone(), root_event_id.clone())) != Some(&next);
        self.entries.insert((room_id, root_event_id), next);
        changed
    }

    pub fn mark_failed(
        &mut self,
        room_id: String,
        root_event_id: String,
        activity_event_id: String,
        activity_timestamp_ms: Option<u64>,
        failure_kind: OperationFailureKind,
    ) -> bool {
        let next = ThreadRootProjectionStatus::Failed {
            activity_event_id,
            activity_timestamp_ms,
            failure_kind,
        };
        let changed = self.entries.get(&(room_id.clone(), root_event_id.clone())) != Some(&next);
        self.entries.insert((room_id, root_event_id), next);
        changed
    }

    pub fn clear(&mut self, room_id: &str, root_event_id: &str) -> bool {
        self.entries
            .remove(&(room_id.to_owned(), root_event_id.to_owned()))
            .is_some()
    }

    /// Forget all projection lifecycle state for an unsubscribed Room.
    pub fn clear_room(&mut self, room_id: &str) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|(entry_room_id, _), _| entry_room_id != room_id);
        before != self.entries.len()
    }
}

fn thread_root_projection_activity_is_newer(
    candidate_event_id: &str,
    candidate_timestamp_ms: Option<u64>,
    existing_event_id: &str,
    existing_timestamp_ms: Option<u64>,
) -> bool {
    candidate_timestamp_ms
        .unwrap_or(0)
        .cmp(&existing_timestamp_ms.unwrap_or(0))
        .then_with(|| candidate_event_id.cmp(existing_event_id))
        .is_gt()
}

/// Sort a threads-list projection according to the Rust-owned display-order
/// setting. The SDK timeline order stays canonical; this is a UI projection.
pub fn sort_threads_list_items(items: &mut [ThreadsListItem], order: ThreadListOrder) {
    match order {
        ThreadListOrder::LatestReply => {
            items.sort_by(|left, right| {
                let left_ts = left.latest_timestamp_ms.unwrap_or(0);
                let right_ts = right.latest_timestamp_ms.unwrap_or(0);
                right_ts
                    .cmp(&left_ts)
                    .then_with(|| left.root_event_id.cmp(&right.root_event_id))
            });
        }
        ThreadListOrder::RootChronology => {
            items.sort_by(|left, right| {
                let left_ts = left.root_timestamp_ms.unwrap_or(0);
                let right_ts = right.root_timestamp_ms.unwrap_or(0);
                left_ts
                    .cmp(&right_ts)
                    .then_with(|| left.root_event_id.cmp(&right.root_event_id))
            });
        }
    }
}
