use std::{
    collections::{BTreeSet, HashMap},
    fmt,
};

use serde::{Deserialize, Serialize};

use crate::locale_profile::DisplayPlatform;

use super::errors::OperationFailureKind;
use super::room::{
    RoomAttentionKind, RoomLatestEventSummary, RoomSummary, room_activity_unread_count,
    room_attention_summary,
};

/// Longest preview the desktop adapter may render in an OS notification body.
///
/// The preview is remote content; a bound keeps one notification from turning
/// into an unbounded banner. Truncation happens on character boundaries.
const NATIVE_NOTIFICATION_PREVIEW_MAX_CHARS: usize = 200;
use super::settings::RoomNotificationMode;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeAttentionContext {
    pub window_focused: bool,
    pub window_focus_observation_generation: u64,
}

impl Default for NativeAttentionContext {
    fn default() -> Self {
        Self {
            window_focused: true,
            window_focus_observation_generation: 0,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct NativeAttentionState {
    pub summary: NativeAttentionSummary,
    pub dispatch: NativeAttentionDispatchState,
    /// Notification text and navigation target for the desktop adapter.
    ///
    /// Present only for a candidate that was selected and not suppressed. It is
    /// deliberately outside [`NativeAttentionSummary`]; the reducer uses its
    /// room target to keep the candidate and its title tied to the selected
    /// room, and the Rust desktop adapter handles notification delivery. It is never
    /// serialized, so the attention surface that reaches the webview stays
    /// free of message content and identifiers.
    #[serde(skip)]
    pub notification: Option<NativeNotificationPayload>,
}

#[derive(Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
pub struct NativeAttentionDispatchId {
    connection_id: u64,
    sequence: u64,
}

impl NativeAttentionDispatchId {
    pub fn new(connection_id: u64, sequence: u64) -> Self {
        Self {
            connection_id,
            sequence,
        }
    }
}

impl fmt::Debug for NativeAttentionDispatchId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NativeAttentionDispatchId(..)")
    }
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

/// Rust-owned text and target of one desktop notification.
///
/// Built in the same projection run as the selected candidate, so the
/// notification body and the navigation target can never describe different
/// events. `Debug` redacts every field: the title carries a room label and the
/// body may carry message content.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct NativeNotificationPayload {
    pub title: String,
    pub body: String,
    pub target: NativeNotificationTarget,
}

impl NativeNotificationPayload {
    pub(crate) fn refresh_title_for_candidate(
        &mut self,
        room_display_name: &str,
        kind: RoomAttentionKind,
    ) {
        self.title = native_notification_title(room_display_name, kind);
    }
}

impl fmt::Debug for NativeNotificationPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeNotificationPayload")
            .field("title", &"NotificationTitle(..)")
            .field("body", &"NotificationBody(..)")
            .field("target", &self.target)
            .finish()
    }
}

/// Navigation target of one desktop notification.
///
/// `room_id` is always present: when the room projection has not produced the
/// triggering event, the adapter still has to open the right room instead of
/// dropping the click. `thread_root_event_id` is set when the triggering event
/// is a thread reply, so the click opens the thread before anchoring the reply.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct NativeNotificationTarget {
    pub room_id: String,
    pub event_id: Option<String>,
    pub thread_root_event_id: Option<String>,
}

impl fmt::Debug for NativeNotificationTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeNotificationTarget")
            .field("room_id", &"RoomId(..)")
            .field("event_id", &self.event_id.as_ref().map(|_| "EventId(..)"))
            .field(
                "thread_root_event_id",
                &self.thread_root_event_id.as_ref().map(|_| "EventId(..)"),
            )
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
    /// Joined, non-space rooms only; spaces and invites are separate state.
    pub rooms: &'a [RoomSummary],
    pub active_room_id: Option<&'a str>,
    pub muted_room_ids: &'a [String],
    pub room_notification_modes: &'a HashMap<String, RoomNotificationMode>,
    pub ignored_user_ids: &'a BTreeSet<String>,
    pub window_focused: bool,
    pub observation: NativeAttentionObservationKind,
    pub previous_candidate: Option<&'a NativeAttentionCandidate>,
    /// `SettingsValues.notifications.message_previews`. Device-local and OFF
    /// by default; when OFF the notification body carries counts only.
    pub message_previews: bool,
    pub capabilities: NativeAttentionCapabilities,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeAttentionProjection {
    pub state: NativeAttentionState,
    pub active_room_match: bool,
    pub notification_count: u64,
    pub badge_room_count: u64,
    pub badge_excluded_room_count: u64,
}

struct NativeAttentionCandidateEntry<'a> {
    room_id: &'a str,
    candidate: NativeAttentionCandidate,
    latest_event: Option<&'a RoomLatestEventSummary>,
}

pub fn native_attention_state_from_rooms(
    input: NativeAttentionProjectionInput<'_>,
) -> NativeAttentionState {
    native_attention_projection_from_rooms(input).state
}

pub fn native_attention_projection_from_rooms(
    input: NativeAttentionProjectionInput<'_>,
) -> NativeAttentionProjection {
    let mut unread_count = 0;
    let mut highlight_count = 0;
    let mut notification_count = 0;
    let mut badge_count = 0;
    let mut badge_room_count = 0;
    let mut badge_excluded_room_count = 0;
    let mut seen_room_ids = BTreeSet::new();
    let mut candidates = Vec::new();

    for room in input.rooms {
        if !seen_room_ids.insert(room.room_id.as_str()) {
            badge_excluded_room_count += 1;
            continue;
        }
        let mode = input
            .room_notification_modes
            .get(&room.room_id)
            .copied()
            .unwrap_or_default();
        let explicitly_muted = input
            .muted_room_ids
            .iter()
            .any(|room_id| room_id == &room.room_id);
        if explicitly_muted || mode == RoomNotificationMode::Mute {
            badge_excluded_room_count += 1;
            continue;
        }
        // Low priority is excluded from the persistent Dock badge as well as
        // from candidates, matching the Home/Space aggregates
        // (state-machine.md, "Sidebar Sections And Low Priority").
        if room.tags.low_priority.is_some() {
            badge_excluded_room_count += 1;
            continue;
        }
        badge_room_count += 1;
        badge_count += room_activity_unread_count(room);

        // An ignored-user DM keeps its shared Dock contribution and is excluded
        // from transient candidates and attention totals only.
        let ignored_user_dm = room.is_dm
            && room
                .dm_user_ids
                .iter()
                .any(|user_id| input.ignored_user_ids.contains(user_id));
        if ignored_user_dm {
            continue;
        }

        let activity_unread_count = room_notification_unread_count(room);
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
        notification_count += effective_notification_count;
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
                latest_event: room.latest_event.as_ref(),
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
    let active_room_match =
        candidate_entry.is_some_and(|entry| input.active_room_id == Some(entry.room_id));
    let mut candidate = candidate_entry.map(|entry| entry.candidate.clone());
    let mut dispatch = NativeAttentionDispatchState::Idle;

    if let Some(entry) = candidate_entry
        && let Some(reason) = native_attention_suppression_reason(input, entry)
    {
        candidate = None;
        dispatch = NativeAttentionDispatchState::Suppressed { reason };
    }

    // A suppressed candidate must not produce OS notification text either, so
    // the payload is derived after suppression, from the same entry that won
    // the candidate selection.
    let notification = match (candidate_entry, candidate.as_ref()) {
        (Some(entry), Some(candidate)) => Some(native_notification_payload(
            entry,
            candidate,
            input.message_previews,
        )),
        _ => None,
    };

    let badge_count = match input.capabilities.badge {
        NativeAttentionCapability::Unavailable => 0,
        NativeAttentionCapability::Available | NativeAttentionCapability::Unknown => badge_count,
    };

    NativeAttentionProjection {
        state: NativeAttentionState {
            summary: NativeAttentionSummary {
                unread_count,
                highlight_count,
                badge_count,
                candidate,
                capabilities: input.capabilities,
            },
            dispatch,
            notification,
        },
        active_room_match,
        notification_count,
        badge_room_count,
        badge_excluded_room_count,
    }
}

/// Compose the OS notification the adapter may show for the selected candidate.
///
/// With previews OFF the body carries counts only and no message content ever
/// leaves Rust. With previews ON the body is the triggering event's sanitized
/// plain-text preview, which the room projection already reduced to
/// `Unable to decrypt message`, `Message deleted`, or the event type for
/// non-text and redacted events. A candidate without a usable preview falls
/// back to the count body, so enabling previews never turns a notification into
/// an empty banner.
fn native_notification_payload(
    entry: &NativeAttentionCandidateEntry<'_>,
    candidate: &NativeAttentionCandidate,
    message_previews: bool,
) -> NativeNotificationPayload {
    let latest_event = entry.latest_event;
    let body = if message_previews {
        latest_event
            .and_then(native_notification_preview)
            .unwrap_or_else(|| native_notification_count_body(candidate))
    } else {
        native_notification_count_body(candidate)
    };

    NativeNotificationPayload {
        title: native_notification_title(&candidate.room_display_name, candidate.kind),
        body,
        target: NativeNotificationTarget {
            room_id: entry.room_id.to_owned(),
            event_id: latest_event.map(|event| event.event_id.clone()),
            thread_root_event_id: latest_event.and_then(|event| event.thread_root_event_id.clone()),
        },
    }
}

fn native_notification_title(room_display_name: &str, kind: RoomAttentionKind) -> String {
    match kind {
        RoomAttentionKind::Mention => format!("Mention in {room_display_name}"),
        RoomAttentionKind::Dm => format!("Direct message in {room_display_name}"),
        RoomAttentionKind::Message => format!("Message in {room_display_name}"),
    }
}

fn native_notification_count_body(candidate: &NativeAttentionCandidate) -> String {
    let mut parts = Vec::new();
    if candidate.kind == RoomAttentionKind::Mention {
        parts.push(format_notification_count(
            candidate.highlight_count,
            "mention",
            "mentions",
        ));
    }
    parts.push(format_notification_count(
        candidate.unread_count,
        "unread",
        "unread",
    ));
    parts.retain(|part| !part.is_empty());
    parts.join(", ")
}

fn format_notification_count(count: u64, singular: &str, plural: &str) -> String {
    match count {
        0 => String::new(),
        1 => format!("1 {singular}"),
        count => format!("{count} {plural}"),
    }
}

/// Plain-text preview line for one room's latest event.
///
/// Returns `None` when the projection has no preview to show, which keeps
/// undecrypted or content-free events on the count body instead of an empty
/// notification.
fn native_notification_preview(latest_event: &RoomLatestEventSummary) -> Option<String> {
    let preview = latest_event.preview.as_deref()?;
    let collapsed = preview.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }

    let line = match latest_event
        .sender_label
        .as_deref()
        .map(str::trim)
        .filter(|sender| !sender.is_empty())
    {
        Some(sender) => format!("{sender}: {collapsed}"),
        None => collapsed,
    };
    Some(truncate_notification_preview(line))
}

fn truncate_notification_preview(text: String) -> String {
    if text.chars().count() <= NATIVE_NOTIFICATION_PREVIEW_MAX_CHARS {
        return text;
    }
    let mut truncated: String = text
        .chars()
        .take(NATIVE_NOTIFICATION_PREVIEW_MAX_CHARS)
        .collect();
    truncated.push('…');
    truncated
}

fn room_notification_unread_count(room: &RoomSummary) -> u64 {
    let count = room.notification_count.max(room.highlight_count);
    if count > 0 {
        count
    } else if room.marked_unread {
        1
    } else {
        0
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

impl NativeAttentionCapabilities {
    /// Resolve the tray capability observed by the platform adapter.
    ///
    /// `native_attention_capabilities_for_platform` is the platform-static
    /// baseline and cannot know whether a tray icon was actually created, so it
    /// leaves `tray` as `Unknown`. The adapter overwrites it with what it
    /// observed when it attempted the tray build (overview.md, "Desktop
    /// Attention Surfaces").
    #[must_use]
    pub fn with_tray(mut self, tray: NativeAttentionCapability) -> Self {
        self.tray = tray;
        self
    }
}

/// Platform-static native attention capability baseline.
///
/// Capabilities decided by the platform alone are resolved here. Capabilities
/// that depend on a runtime attempt stay `Unknown` and are resolved by the
/// adapter before the snapshot reaches the webview; `tray` is resolved through
/// [`NativeAttentionCapabilities::with_tray`].
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
        dispatch_id: NativeAttentionDispatchId,
    },
    Delivered {
        dispatch_id: NativeAttentionDispatchId,
    },
    Unsupported {
        dispatch_id: NativeAttentionDispatchId,
    },
    Suppressed {
        reason: NativeAttentionSuppressionReason,
    },
    Failed {
        dispatch_id: NativeAttentionDispatchId,
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
            Self::Unsupported { .. } => "unsupported",
            Self::Suppressed { .. } => "suppressed",
            Self::Failed { .. } => "failed",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NativeAttentionSoundOutcome {
    Played,
    Unsupported,
    Failed,
    Skipped,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{RoomSummary, RoomTags};

    fn unread_room() -> RoomSummary {
        RoomSummary {
            display_name_placeholder: None,
            display_label_placeholder: None,
            room_id: "!room:example.invalid".to_owned(),
            display_name: "Room".to_owned(),
            display_label: "Room".to_owned(),
            original_display_label: "Room".to_owned(),
            avatar: None,
            is_dm: false,
            dm_user_ids: Vec::new(),
            tags: RoomTags::default(),
            unread_count: 2,
            notification_count: 2,
            highlight_count: 0,
            marked_unread: false,
            recency_stamp: Some(42),
            conversation_activity: None,
            latest_event: None,
            parent_space_ids: Vec::new(),
            dm_space_ids: Vec::new(),
            is_encrypted: false,
            joined_members: 2,
        }
    }

    fn project(observation: NativeAttentionObservationKind) -> NativeAttentionProjection {
        let rooms = [unread_room()];
        native_attention_projection_from_rooms(NativeAttentionProjectionInput {
            rooms: &rooms,
            active_room_id: None,
            muted_room_ids: &[],
            room_notification_modes: &HashMap::new(),
            ignored_user_ids: &BTreeSet::new(),
            window_focused: false,
            observation,
            previous_candidate: None,
            message_previews: false,
            capabilities: NativeAttentionCapabilities::default(),
        })
    }

    #[test]
    fn native_attention_observation_suppresses_non_live_candidates() {
        for (observation, reason) in [
            (
                NativeAttentionObservationKind::InitialSync,
                NativeAttentionSuppressionReason::InitialSync,
            ),
            (
                NativeAttentionObservationKind::Backfill,
                NativeAttentionSuppressionReason::Backfill,
            ),
            (
                NativeAttentionObservationKind::SelfEvent,
                NativeAttentionSuppressionReason::SelfMessage,
            ),
        ] {
            let projection = project(observation);
            assert_eq!(projection.state.summary.unread_count, 2);
            assert_eq!(projection.state.summary.badge_count, 2);
            assert_eq!(projection.state.summary.candidate, None);
            assert_eq!(
                projection.state.dispatch,
                NativeAttentionDispatchState::Suppressed { reason }
            );
        }

        let live = project(NativeAttentionObservationKind::Live);
        assert_eq!(live.state.summary.unread_count, 2);
        assert_eq!(live.state.summary.badge_count, 2);
        assert!(live.state.summary.candidate.is_some());
        assert_eq!(live.state.dispatch, NativeAttentionDispatchState::Idle);
        assert!(!live.active_room_match);
    }

    #[test]
    fn native_badge_diagnostics_count_only_unique_non_muted_rooms() {
        let included = unread_room();
        let mut duplicate = included.clone();
        duplicate.unread_count = 9;
        let mut explicit_mute = unread_room();
        explicit_mute.room_id = "!explicit:example.invalid".to_owned();
        let mut mode_mute = unread_room();
        mode_mute.room_id = "!mode:example.invalid".to_owned();
        let rooms = [included, duplicate, explicit_mute, mode_mute];
        let modes = HashMap::from([(
            "!mode:example.invalid".to_owned(),
            RoomNotificationMode::Mute,
        )]);

        let projection = native_attention_projection_from_rooms(NativeAttentionProjectionInput {
            rooms: &rooms,
            active_room_id: None,
            muted_room_ids: &["!explicit:example.invalid".to_owned()],
            room_notification_modes: &modes,
            ignored_user_ids: &BTreeSet::new(),
            window_focused: false,
            observation: NativeAttentionObservationKind::Live,
            previous_candidate: None,
            message_previews: false,
            capabilities: NativeAttentionCapabilities::default(),
        });

        assert_eq!(projection.state.summary.badge_count, 2);
        assert_eq!(projection.badge_room_count, 1);
        assert_eq!(projection.badge_excluded_room_count, 3);
    }
}
