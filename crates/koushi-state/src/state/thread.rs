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
