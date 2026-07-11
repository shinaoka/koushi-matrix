use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::errors::OperationFailureKind;
use super::settings::ThreadListOrder;
use super::timeline::ComposerState;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ThreadPaneState {
    Closed,
    Opening {
        room_id: String,
        root_event_id: String,
    },
    Open {
        room_id: String,
        root_event_id: String,
        is_subscribed: bool,
        composer: ComposerState,
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

/// The selected canonical reply currently representing a root in the bounded
/// Room timeline window. Reconciliation owns this selection, so it can move
/// backwards when the newest reply leaves the window.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ThreadRootProjectionActivity {
    pub root_event_id: String,
    pub activity_event_id: String,
    pub activity_timestamp_ms: Option<u64>,
}

/// Rust-owned record of bounded root hydration attempts, keyed by the exact
/// `(room_id, root_event_id)` pair. Failed entries are terminal for this room
/// timeline lifetime, so repeated reply diffs cannot start a fetch loop.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ThreadRootProjectionState {
    entries: BTreeMap<(String, String), ThreadRootProjectionStatus>,
    /// The current bounded canonical Room window per room. An absent room has
    /// not reported a window yet; this keeps direct state tests and startup
    /// actions conservative until the first reconciliation arrives.
    #[serde(default)]
    active_root_event_ids: BTreeMap<String, BTreeSet<String>>,
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
        if !self.is_active_or_unreported(&room_id, &root_event_id) {
            return false;
        }
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
                        failure_kind: failure_kind.clone(),
                    }
                }
            };
            if existing == updated {
                return false;
            }
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
    ) {
        if !self.is_active_or_unreported(&room_id, &root_event_id) {
            self.entries.remove(&(room_id.clone(), root_event_id));
            self.cleanup_empty_room_tracking(&room_id);
            return;
        }
        self.entries.insert(
            (room_id, root_event_id),
            ThreadRootProjectionStatus::Ready {
                activity_event_id,
                activity_timestamp_ms,
            },
        );
    }

    pub fn mark_failed(
        &mut self,
        room_id: String,
        root_event_id: String,
        activity_event_id: String,
        activity_timestamp_ms: Option<u64>,
        failure_kind: OperationFailureKind,
    ) {
        if !self.is_active_or_unreported(&room_id, &root_event_id) {
            self.entries.remove(&(room_id.clone(), root_event_id));
            self.cleanup_empty_room_tracking(&room_id);
            return;
        }
        self.entries.insert(
            (room_id, root_event_id),
            ThreadRootProjectionStatus::Failed {
                activity_event_id,
                activity_timestamp_ms,
                failure_kind,
            },
        );
    }

    /// Reconcile terminal records with the current bounded canonical Room
    /// window. Pending workers are allowed to settle, but ready/failed records
    /// disappear as soon as their reply root is no longer active.
    pub fn reconcile_room(
        &mut self,
        room_id: String,
        activities: Vec<ThreadRootProjectionActivity>,
    ) {
        let activities_by_root = activities
            .into_iter()
            .map(|activity| (activity.root_event_id.clone(), activity))
            .collect::<BTreeMap<_, _>>();
        let active = activities_by_root.keys().cloned().collect::<BTreeSet<_>>();
        self.active_root_event_ids
            .insert(room_id.clone(), active.clone());
        self.entries
            .retain(|(entry_room_id, root_event_id), status| {
                entry_room_id != &room_id
                    || active.contains(root_event_id)
                    || matches!(status, ThreadRootProjectionStatus::Pending { .. })
            });
        for (root_event_id, activity) in activities_by_root {
            let key = (room_id.clone(), root_event_id);
            let Some(existing) = self.entries.get(&key).cloned() else {
                continue;
            };
            let updated = match existing {
                ThreadRootProjectionStatus::Pending { .. } => ThreadRootProjectionStatus::Pending {
                    activity_event_id: activity.activity_event_id,
                    activity_timestamp_ms: activity.activity_timestamp_ms,
                },
                ThreadRootProjectionStatus::Ready { .. } => ThreadRootProjectionStatus::Ready {
                    activity_event_id: activity.activity_event_id,
                    activity_timestamp_ms: activity.activity_timestamp_ms,
                },
                ThreadRootProjectionStatus::Failed { failure_kind, .. } => {
                    ThreadRootProjectionStatus::Failed {
                        activity_event_id: activity.activity_event_id,
                        activity_timestamp_ms: activity.activity_timestamp_ms,
                        failure_kind,
                    }
                }
            };
            self.entries.insert(key, updated);
        }
        self.cleanup_empty_room_tracking(&room_id);
    }

    /// Forget all projection lifecycle state for an unsubscribed Room. This is
    /// deliberately distinct from reconciliation: inactive pending attempts
    /// are retained only while their Room actor remains subscribed.
    pub fn clear_room(&mut self, room_id: &str) -> bool {
        self.active_root_event_ids.remove(room_id);
        let before = self.entries.len();
        self.entries
            .retain(|(entry_room_id, _), _| entry_room_id != room_id);
        before != self.entries.len()
    }

    fn is_active_or_unreported(&self, room_id: &str, root_event_id: &str) -> bool {
        self.active_root_event_ids
            .get(room_id)
            .is_none_or(|active| active.contains(root_event_id))
    }

    fn cleanup_empty_room_tracking(&mut self, room_id: &str) {
        let has_pending_or_active_record = self
            .entries
            .keys()
            .any(|(entry_room_id, _)| entry_room_id == room_id);
        if self
            .active_root_event_ids
            .get(room_id)
            .is_some_and(BTreeSet::is_empty)
            && !has_pending_or_active_record
        {
            self.active_root_event_ids.remove(room_id);
        }
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
