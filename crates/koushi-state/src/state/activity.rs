use std::{collections::BTreeSet, fmt};

use serde::{Deserialize, Serialize};

use super::errors::OperationFailureKind;
use super::profile::AvatarImage;

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ActivityState {
    #[default]
    Closed,
    Opening {
        request_id: u64,
        tab: ActivityTab,
    },
    Open {
        active_tab: ActivityTab,
        recent: ActivityStream,
        unread: ActivityStream,
        mark_read: ActivityMarkReadState,
    },
}

impl ActivityState {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Opening { .. } => "opening",
            Self::Open { .. } => "open",
        }
    }
}

impl fmt::Debug for ActivityState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => formatter.write_str("ActivityState::Closed"),
            Self::Opening { request_id, tab } => formatter
                .debug_struct("ActivityOpening")
                .field("request_id", request_id)
                .field("tab", tab)
                .finish(),
            Self::Open {
                active_tab,
                recent,
                unread,
                mark_read,
            } => formatter
                .debug_struct("ActivityOpen")
                .field("active_tab", active_tab)
                .field("recent", recent)
                .field("unread", unread)
                .field("mark_read", mark_read)
                .finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivityTab {
    #[default]
    Recent,
    Unread,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActivityStreamSummary {
    pub event_count: u32,
    pub room_count: u32,
    pub highlight_count: u32,
    pub unresolved_room_count: u32,
}

impl ActivityStreamSummary {
    pub fn from_rows(rows: &[ActivityRow]) -> Self {
        let mut room_ids = BTreeSet::new();
        let mut event_count = 0_u32;
        let mut highlight_count = 0_u32;
        let mut unresolved_room_count = 0_u32;

        for row in rows {
            room_ids.insert(row.room_id.as_str());
            if row.highlight {
                highlight_count = highlight_count.saturating_add(1);
            }
            match row.kind {
                ActivityRowKind::Event => event_count = event_count.saturating_add(1),
                ActivityRowKind::RoomUnread => {
                    unresolved_room_count = unresolved_room_count.saturating_add(1);
                }
            }
        }

        Self {
            event_count,
            room_count: room_ids.len().try_into().unwrap_or(u32::MAX),
            highlight_count,
            unresolved_room_count,
        }
    }
}

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActivityStream {
    pub rows: Vec<ActivityRow>,
    pub next_batch: Option<String>,
    #[serde(default)]
    pub summary: ActivityStreamSummary,
}

impl ActivityStream {
    pub fn new(rows: Vec<ActivityRow>, next_batch: Option<String>) -> Self {
        let summary = ActivityStreamSummary::from_rows(&rows);
        Self {
            rows,
            next_batch,
            summary,
        }
    }

    pub fn refresh_summary(&mut self) {
        self.summary = ActivityStreamSummary::from_rows(&self.rows);
    }
}

impl fmt::Debug for ActivityStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActivityStream")
            .field("rows", &format_args!("{} row(s)", self.rows.len()))
            .field(
                "next_batch",
                &self.next_batch.as_ref().map(|_| "PageToken(..)"),
            )
            .field("summary", &self.summary)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivityRowKind {
    #[default]
    Event,
    RoomUnread,
}

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActivityRow {
    #[serde(default)]
    pub kind: ActivityRowKind,
    pub room_id: String,
    pub event_id: Option<String>,
    #[serde(default)]
    pub sender_id: Option<String>,
    pub room_label: String,
    pub sender_label: Option<String>,
    #[serde(default)]
    pub sender_avatar: Option<AvatarImage>,
    pub preview: Option<String>,
    pub timestamp_ms: u64,
    pub unread: bool,
    pub highlight: bool,
    #[serde(default)]
    pub context_label: String,
}

impl ActivityRow {
    pub fn event(
        room_id: String,
        event_id: String,
        sender_id: Option<String>,
        room_label: String,
        sender_label: Option<String>,
        preview: Option<String>,
        timestamp_ms: u64,
        unread: bool,
        highlight: bool,
    ) -> Self {
        Self {
            kind: ActivityRowKind::Event,
            room_id,
            event_id: Some(event_id),
            sender_id,
            room_label,
            sender_label,
            sender_avatar: None,
            preview,
            timestamp_ms,
            unread,
            highlight,
            context_label: String::new(),
        }
    }

    pub fn room_unread_placeholder(
        room_id: String,
        room_label: String,
        timestamp_ms: u64,
        highlight: bool,
    ) -> Self {
        Self {
            kind: ActivityRowKind::RoomUnread,
            room_id,
            event_id: None,
            sender_id: None,
            room_label,
            sender_label: None,
            sender_avatar: None,
            preview: None,
            timestamp_ms,
            unread: true,
            highlight,
            context_label: String::new(),
        }
    }
}

impl fmt::Debug for ActivityRow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActivityRow")
            .field("kind", &self.kind)
            .field("room_id", &"RoomId(..)")
            .field("event_id", &self.event_id.as_ref().map(|_| "EventId(..)"))
            .field("sender_id", &self.sender_id.as_ref().map(|_| "UserId(..)"))
            .field("room_label", &"RoomLabel(..)")
            .field(
                "sender_label",
                &self.sender_label.as_ref().map(|_| "SenderLabel(..)"),
            )
            .field(
                "sender_avatar",
                &self.sender_avatar.as_ref().map(|_| "AvatarImage(..)"),
            )
            .field("preview", &self.preview.as_ref().map(|_| "Preview(..)"))
            .field("timestamp_ms", &self.timestamp_ms)
            .field("unread", &self.unread)
            .field("highlight", &self.highlight)
            .field(
                "context_label",
                &(!self.context_label.is_empty()).then_some("ContextLabel(..)"),
            )
            .finish()
    }
}

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ActivityMarkReadState {
    #[default]
    Idle,
    Pending {
        request_id: u64,
        target: ActivityMarkReadTarget,
    },
    Failed {
        target: ActivityMarkReadTarget,
        failure_kind: OperationFailureKind,
    },
}

impl fmt::Debug for ActivityMarkReadState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => formatter.write_str("ActivityMarkReadState::Idle"),
            Self::Pending { request_id, target } => formatter
                .debug_struct("ActivityMarkReadPending")
                .field("request_id", request_id)
                .field("target", target)
                .finish(),
            Self::Failed {
                target,
                failure_kind,
            } => formatter
                .debug_struct("ActivityMarkReadFailed")
                .field("target", target)
                .field("kind", failure_kind)
                .finish(),
        }
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ActivityMarkReadTarget {
    Room {
        room_id: String,
        up_to_event_id: String,
    },
    All,
}

impl fmt::Debug for ActivityMarkReadTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Room { .. } => formatter
                .debug_struct("ActivityMarkReadTarget::Room")
                .field("room_id", &"RoomId(..)")
                .field("up_to_event_id", &"EventId(..)")
                .finish(),
            Self::All => formatter.write_str("ActivityMarkReadTarget::All"),
        }
    }
}
