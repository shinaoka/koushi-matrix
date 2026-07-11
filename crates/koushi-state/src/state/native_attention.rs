use std::{
    collections::{BTreeSet, HashMap},
    fmt,
};

use serde::{Deserialize, Serialize};

use crate::locale_profile::DisplayPlatform;

use super::errors::OperationFailureKind;
use super::room::{
    RoomAttentionKind, RoomSummary, room_activity_unread_count, room_attention_summary,
};
use super::settings::RoomNotificationMode;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct NativeAttentionState {
    pub summary: NativeAttentionSummary,
    pub dispatch: NativeAttentionDispatchState,
}

impl NativeAttentionState {
    pub fn kind(&self) -> &'static str {
        self.dispatch.kind()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct NativeAttentionSummary {
    pub unread_count: u64,
    pub highlight_count: u64,
    pub badge_count: u64,
    pub candidate: Option<NativeAttentionCandidate>,
    pub capabilities: NativeAttentionCapabilities,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct NativeAttentionCandidate {
    pub room_display_name: String,
    pub kind: RoomAttentionKind,
    pub unread_count: u64,
    pub highlight_count: u64,
}

impl fmt::Debug for NativeAttentionCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeAttentionCandidate")
            .field("room_display_name", &"RoomName(..)")
            .field("kind", &self.kind)
            .field("unread_count", &self.unread_count)
            .field("highlight_count", &self.highlight_count)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeAttentionObservationKind {
    Live,
    InitialSync,
    Backfill,
    SelfEvent,
}

#[derive(Clone, Copy, Debug)]
pub struct NativeAttentionProjectionInput<'a> {
    pub rooms: &'a [RoomSummary],
    pub active_room_id: Option<&'a str>,
    pub muted_room_ids: &'a [String],
    pub room_notification_modes: &'a HashMap<String, RoomNotificationMode>,
    pub ignored_user_ids: &'a BTreeSet<String>,
    pub window_focused: bool,
    pub observation: NativeAttentionObservationKind,
    pub previous_candidate: Option<&'a NativeAttentionCandidate>,
    pub capabilities: NativeAttentionCapabilities,
}

struct NativeAttentionCandidateEntry<'a> {
    room_id: &'a str,
    candidate: NativeAttentionCandidate,
}

pub fn native_attention_state_from_rooms(
    input: NativeAttentionProjectionInput<'_>,
) -> NativeAttentionState {
    let mut unread_count = 0;
    let mut highlight_count = 0;
    let mut candidates = Vec::new();

    for room in input.rooms {
        if room.tags.low_priority.is_some()
            || input
                .muted_room_ids
                .iter()
                .any(|room_id| room_id == &room.room_id)
            || (room.is_dm
                && room
                    .dm_user_ids
                    .iter()
                    .any(|user_id| input.ignored_user_ids.contains(user_id)))
        {
            continue;
        }

        let mode = input
            .room_notification_modes
            .get(&room.room_id)
            .copied()
            .unwrap_or_default();
        if mode == RoomNotificationMode::Mute {
            continue;
        }

        let activity_unread_count = room_activity_unread_count(room);
        let effective_unread_count =
            if mode == RoomNotificationMode::Mentions && room.highlight_count == 0 {
                0
            } else {
                activity_unread_count
            };
        let effective_notification_count = if mode == RoomNotificationMode::Mentions {
            0
        } else {
            room.notification_count
        };

        unread_count += effective_unread_count;
        highlight_count += room.highlight_count;

        if mode == RoomNotificationMode::Mentions && room.highlight_count == 0 {
            continue;
        }

        if let Some(summary) = room_attention_summary(
            room.display_label.clone(),
            room.is_dm,
            effective_notification_count,
            room.highlight_count,
            effective_unread_count,
        ) {
            candidates.push(NativeAttentionCandidateEntry {
                room_id: &room.room_id,
                candidate: NativeAttentionCandidate {
                    room_display_name: summary.room_display_name,
                    kind: summary.kind,
                    unread_count: summary.unread_count,
                    highlight_count: summary.highlight_count,
                },
            });
        }
    }

    candidates.sort_by(|left, right| {
        attention_kind_priority(right.candidate.kind)
            .cmp(&attention_kind_priority(left.candidate.kind))
            .then_with(|| {
                right
                    .candidate
                    .highlight_count
                    .cmp(&left.candidate.highlight_count)
            })
            .then_with(|| {
                right
                    .candidate
                    .unread_count
                    .cmp(&left.candidate.unread_count)
            })
            .then_with(|| {
                left.candidate
                    .room_display_name
                    .cmp(&right.candidate.room_display_name)
            })
    });

    let candidate_entry = candidates.first();
    let mut candidate = candidate_entry.map(|entry| entry.candidate.clone());
    let mut dispatch = NativeAttentionDispatchState::Idle;

    if let Some(entry) = candidate_entry {
        if let Some(reason) = native_attention_suppression_reason(input, entry) {
            candidate = None;
            dispatch = NativeAttentionDispatchState::Suppressed { reason };
        }
    }

    let badge_count = match input.capabilities.badge {
        NativeAttentionCapability::Unavailable => 0,
        NativeAttentionCapability::Available | NativeAttentionCapability::Unknown => unread_count,
    };

    NativeAttentionState {
        summary: NativeAttentionSummary {
            unread_count,
            highlight_count,
            badge_count,
            candidate,
            capabilities: input.capabilities,
        },
        dispatch,
    }
}

fn attention_kind_priority(kind: RoomAttentionKind) -> u8 {
    match kind {
        RoomAttentionKind::Mention => 3,
        RoomAttentionKind::Dm => 2,
        RoomAttentionKind::Message => 1,
    }
}

fn native_attention_suppression_reason(
    input: NativeAttentionProjectionInput<'_>,
    entry: &NativeAttentionCandidateEntry<'_>,
) -> Option<NativeAttentionSuppressionReason> {
    match input.observation {
        NativeAttentionObservationKind::InitialSync => {
            return Some(NativeAttentionSuppressionReason::InitialSync);
        }
        NativeAttentionObservationKind::Backfill => {
            return Some(NativeAttentionSuppressionReason::Backfill);
        }
        NativeAttentionObservationKind::SelfEvent => {
            return Some(NativeAttentionSuppressionReason::SelfMessage);
        }
        NativeAttentionObservationKind::Live => {}
    }

    if input.window_focused && input.active_room_id == Some(entry.room_id) {
        return Some(NativeAttentionSuppressionReason::WindowFocused);
    }

    if input.capabilities.notifications == NativeAttentionCapability::Unavailable {
        return Some(NativeAttentionSuppressionReason::CapabilityUnavailable);
    }

    if input.previous_candidate == Some(&entry.candidate) {
        return Some(NativeAttentionSuppressionReason::Duplicate);
    }

    None
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct NativeAttentionCapabilities {
    pub notifications: NativeAttentionCapability,
    pub badge: NativeAttentionCapability,
    pub overlay_icon: NativeAttentionCapability,
    pub sound: NativeAttentionCapability,
    pub tray: NativeAttentionCapability,
    pub activation: NativeAttentionCapability,
}

pub fn native_attention_capabilities_for_platform(
    platform: DisplayPlatform,
) -> NativeAttentionCapabilities {
    let badge = match platform {
        DisplayPlatform::Macos | DisplayPlatform::Windows => NativeAttentionCapability::Available,
        DisplayPlatform::Linux => NativeAttentionCapability::Unknown,
    };

    NativeAttentionCapabilities {
        notifications: NativeAttentionCapability::Available,
        badge,
        overlay_icon: match platform {
            DisplayPlatform::Windows => NativeAttentionCapability::Available,
            DisplayPlatform::Macos | DisplayPlatform::Linux => {
                NativeAttentionCapability::Unavailable
            }
        },
        sound: match platform {
            DisplayPlatform::Macos | DisplayPlatform::Windows => {
                NativeAttentionCapability::Available
            }
            DisplayPlatform::Linux => NativeAttentionCapability::Unavailable,
        },
        tray: NativeAttentionCapability::Unknown,
        activation: NativeAttentionCapability::Unknown,
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NativeAttentionCapability {
    Available,
    Unavailable,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum NativeAttentionDispatchState {
    #[default]
    Idle,
    Dispatching {
        request_id: u64,
    },
    Delivered {
        request_id: u64,
    },
    Suppressed {
        reason: NativeAttentionSuppressionReason,
    },
    Failed {
        request_id: u64,
        #[serde(rename = "failureKind")]
        kind: OperationFailureKind,
    },
}

impl NativeAttentionDispatchState {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Dispatching { .. } => "dispatching",
            Self::Delivered { .. } => "delivered",
            Self::Suppressed { .. } => "suppressed",
            Self::Failed { .. } => "failed",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NativeAttentionSuppressionReason {
    InitialSync,
    Backfill,
    SelfMessage,
    WindowFocused,
    RoomMuted,
    LowPriority,
    Duplicate,
    CapabilityUnavailable,
}
