use std::collections::BTreeMap;
use std::fmt;

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
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ThreadsListScope {
    Room { room_id: String },
    Home,
    Space { space_id: String },
}

// #1979: the scope names one room or space, so its Debug is a placeholder.
impl fmt::Debug for ThreadsListScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Room { .. } => "Room { room_id: RoomId(..) }",
            Self::Home => "Home",
            Self::Space { .. } => "Space { space_id: SpaceId(..) }",
        })
    }
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

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
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
    /// #1473: the thread timeline could not be loaded. Terminal until the user
    /// retries (`OpenThread`) or closes the pane; late subscription outcomes
    /// for the same root are ignored.
    Failed {
        room_id: String,
        root_event_id: String,
        intent: ThreadOpenIntent,
        failure_kind: OperationFailureKind,
    },
}

// #1985: `PinnedReply` carries the event the permalink pointed at.
fn intent_name(intent: &ThreadOpenIntent) -> &'static str {
    match intent {
        ThreadOpenIntent::ExistingThread => "ExistingThread",
        ThreadOpenIntent::NewThreadDraft => "NewThreadDraft",
        ThreadOpenIntent::PinnedReply { .. } => "PinnedReply",
    }
}

// #1985: every open variant carries a room and a thread root.
impl std::fmt::Debug for ThreadPaneState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closed => formatter.write_str("Closed"),
            Self::Opening { intent, .. } => formatter
                .debug_struct("Opening")
                .field("room_id", &"RoomId(..)")
                .field("root_event_id", &"EventId(..)")
                .field("intent", &intent_name(intent))
                .finish(),
            Self::Open {
                intent,
                is_subscribed,
                composer,
                staged_uploads,
                ..
            } => formatter
                .debug_struct("Open")
                .field("room_id", &"RoomId(..)")
                .field("root_event_id", &"EventId(..)")
                .field("intent", &intent_name(intent))
                .field("is_subscribed", is_subscribed)
                .field("composer", composer)
                .field("staged_uploads", &staged_uploads.len())
                .finish(),
            Self::Failed {
                intent,
                failure_kind,
                ..
            } => formatter
                .debug_struct("Failed")
                .field("room_id", &"RoomId(..)")
                .field("root_event_id", &"EventId(..)")
                .field("intent", &intent_name(intent))
                .field("failure_kind", failure_kind)
                .finish(),
        }
    }
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
        /// Roots with locally observed unread replies remain navigable even
        /// when the thread metadata service cannot load.
        #[serde(default)]
        items: Vec<ThreadsListItem>,
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

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
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

// #1985: a row carries a room, two events, two senders, their labels, and two
// body previews. This row is reachable from the whole-`AppState` diagnostic
// dump, so it renders presence and counts instead.
impl std::fmt::Debug for ThreadsListItem {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ThreadsListItem")
            .field("room_id", &"RoomId(..)")
            .field("root_event_id", &"EventId(..)")
            .field("root_sender", &"UserId(..)")
            .field(
                "root_sender_label",
                &self.root_sender_label.as_ref().map(|_| "SenderLabel(..)"),
            )
            .field(
                "root_body_preview",
                &self.root_body_preview.as_ref().map(|_| "MessageBody(..)"),
            )
            .field("root_timestamp_ms", &self.root_timestamp_ms)
            .field(
                "latest_event_id",
                &self.latest_event_id.as_ref().map(|_| "EventId(..)"),
            )
            .field(
                "latest_sender",
                &self.latest_sender.as_ref().map(|_| "UserId(..)"),
            )
            .field(
                "latest_sender_label",
                &self.latest_sender_label.as_ref().map(|_| "SenderLabel(..)"),
            )
            .field(
                "latest_body_preview",
                &self.latest_body_preview.as_ref().map(|_| "MessageBody(..)"),
            )
            .field("latest_timestamp_ms", &self.latest_timestamp_ms)
            .field("reply_count", &self.reply_count)
            .field("unread_count", &self.unread_count)
            .finish()
    }
}

/// #1238: one thread root's SDK thread-cache attention, as observed by the room
/// timeline actor.
///
/// `unread` and `highlight` are the thread cache's own `num_unread` /
/// `num_mentions`. The reducer sums these per room into
/// `RoomSummary.thread_unread_count` / `thread_highlight_count`. The shared
/// badge helper adds confirmed threaded unread to main-timeline unread; a
/// root's value never enters the main-only `RoomSummary.unread_count`.
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
