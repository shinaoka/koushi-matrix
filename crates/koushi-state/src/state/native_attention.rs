use std::{
    collections::{BTreeSet, HashMap},
    fmt,
};

use serde::{Deserialize, Serialize};

use crate::locale_profile::{CatalogLocale, DisplayPlatform};
use crate::notification_text::{NotificationText, notification_counts, notification_text};

use super::AppState;
use super::errors::OperationFailureKind;
use super::notification_delivery::{
    BoundedNotificationDeliveryText, bounded_notification_delivery_message_source,
};
#[cfg(test)]
use super::room::RoomLatestEventSummary;
use super::room::{
    RoomAttentionKind, RoomSummary, room_activity_unread_count, room_attention_summary,
};
use super::settings::RoomNotificationMode;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeAttentionContext {
    pub window_focused: bool,
    pub window_focus_observation_generation: u64,
    /// Process-local sequence that mints alert identities (#1802).
    ///
    /// Monotonic, never time-derived: a value handed out once cannot come back
    /// for a later alert, so the webview can dedupe on identity alone.
    pub alert_sequence: u64,
}

impl Default for NativeAttentionContext {
    fn default() -> Self {
        Self {
            window_focused: true,
            window_focus_observation_generation: 0,
            alert_sequence: 0,
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
    /// Identity of the alert this summary publishes (#1802).
    ///
    /// Minted in Rust where a candidate is raised, kept while the same alert is
    /// relabelled, and cleared with the candidate. The webview dedupes on this
    /// token alone instead of on the candidate's contents.
    #[serde(default)]
    pub alert_id: Option<u64>,
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
        locale: CatalogLocale,
    ) {
        self.title = native_notification_title(room_display_name, kind, locale);
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
    /// Device setting (#994): when on, a verified live alert carries the triggering
    /// event's text instead of the count body.
    pub message_previews: bool,
    pub capabilities: NativeAttentionCapabilities,
    /// The catalog locale the banner text is written in, resolved from the
    /// current settings when the projection runs (#1476).
    pub locale: CatalogLocale,
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
    room: &'a RoomSummary,
    /// Whether this room is the one whose main unread count rose in this live
    /// observation, which is what lets its latest event stand for the alert (#994).
    is_verified_live_room: bool,
    candidate: NativeAttentionCandidate,
}

pub fn native_attention_state_from_rooms(
    input: NativeAttentionProjectionInput<'_>,
) -> NativeAttentionState {
    native_attention_projection_from_rooms(input).state
}

pub fn native_attention_projection_from_rooms(
    input: NativeAttentionProjectionInput<'_>,
) -> NativeAttentionProjection {
    native_attention_projection_from_rooms_with_live_room(input, None, None, false)
}

/// Production projection: a transient alert may only target the room whose
/// SDK-local unread count increased in this room-list observation.
///
/// `preview_room_id` is that room again when the reducer also saw its latest event
/// change to one another user sent; only then may the event's text stand for the
/// alert (#994). Otherwise the alert carries the count body.
pub(crate) fn native_attention_projection_from_rooms_with_live_room(
    input: NativeAttentionProjectionInput<'_>,
    live_room_id: Option<&str>,
    preview_room_id: Option<&str>,
    require_live_room: bool,
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

        let highlight_total = room
            .highlight_count
            .saturating_add(room.thread_highlight_count);
        let activity_unread_count = room_activity_unread_count(room);
        let effective_unread_count =
            if mode == RoomNotificationMode::Mentions && highlight_total == 0 {
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
        highlight_count += highlight_total;

        if activity_unread_count == 0
            // A room-level highlight counter cannot identify which unread
            // event is the mention, so Mentions-only cannot raise a banner.
            || mode == RoomNotificationMode::Mentions
            || (require_live_room && live_room_id != Some(room.room_id.as_str()))
            || !room
                .latest_event
                .as_ref()
                .is_some_and(|event| !event.event_id.is_empty() && !event.is_redacted)
        {
            continue;
        }

        if let Some(summary) = room_attention_summary(
            room.display_label.clone(),
            room.is_dm,
            0,
            0,
            effective_unread_count,
        ) {
            candidates.push(NativeAttentionCandidateEntry {
                room_id: &room.room_id,
                room,
                is_verified_live_room: require_live_room
                    && live_room_id == Some(room.room_id.as_str())
                    && preview_room_id == Some(room.room_id.as_str()),
                candidate: NativeAttentionCandidate {
                    room_display_name: summary.room_display_name,
                    kind: summary.kind,
                    unread_count: summary.unread_count,
                    highlight_count: 0,
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
            input.message_previews && input.observation == NativeAttentionObservationKind::Live,
            input.locale,
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
                // A stateless projection has no sequence to mint from; the reducer
                // assigns the identity where it raises the candidate (#1802).
                alert_id: None,
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

/// The other member of a direct message, for naming it in a delivered digest (#1492).
///
/// Only a direct message with exactly one other member has one. The user id is the
/// room list's own direct target; the alias is this device's personal local alias for
/// that user, when one is set.
#[derive(Clone, Eq, PartialEq)]
pub struct DirectPartner {
    pub user_id: String,
    pub local_alias: Option<String>,
}

/// One room the attention projection counts as holding unread attention (#1427).
///
/// The device-local delivery channel needs the same rooms the badge counts, but for
/// every one of them rather than only the single room a transient alert may target. It
/// also needs the mention count, which the candidate deliberately discards. Both come
/// from this one function so the delivery channel cannot drift from the badge.
#[derive(Clone, Eq, PartialEq)]
pub struct RoomAttentionCandidate {
    pub room_id: String,
    /// The safe display label, the same one the candidate and the sidebar row use.
    pub display_label: String,
    pub kind: RoomAttentionKind,
    /// Main-timeline plus thread unread, after the room's notification mode is applied.
    pub unread: u32,
    /// Main-timeline plus thread highlights, which is what makes a room a mention.
    pub mentions: u32,
    /// The spaces this room is a child of in the room list right now, so a watch list naming
    /// a space covers a room that joined it after the list was written.
    pub parent_space_ids: Vec<String>,
    /// The display names of the first parent spaces, for the digest's room line
    /// (#1460). Filled by [`attention_room_candidates_for_state`], which owns the space
    /// list; empty when the room is in no space.
    pub parent_space_labels: Vec<String>,
    /// The body of the room's latest conversation event, when it is a plain text message
    /// this device can already read and [`bounded_notification_delivery_message_source`]
    /// has bounded it (#1427, #1460).
    ///
    /// It is the *latest* event, which may be newer than the unread event that made the
    /// room qualify: a room can be unread because of an older mention while a newer plain
    /// message sits at the top, and the room list previews that same newer message. It is
    /// message text, so it is carried here and never serialized, logged, or rendered
    /// except by `digest` under a per-sink consent.
    pub newest_message_text: Option<BoundedNotificationDeliveryText>,
    /// The other member when the room is a one-to-one direct message (#1492). The alias
    /// is filled by [`attention_room_candidates_for_state`], which owns the alias list.
    pub direct_partner: Option<DirectPartner>,
}

impl fmt::Debug for RoomAttentionCandidate {
    /// Redacts the message text: the candidate travels through failure output and
    /// diagnostics, and a body is the one field here that is the sender's words.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RoomAttentionCandidate")
            .field("room_id", &"RoomId(..)")
            .field("display_label", &"DisplayLabel(..)")
            .field("kind", &self.kind)
            .field("unread", &self.unread)
            .field("mentions", &self.mentions)
            .field("parent_space_ids", &self.parent_space_ids.len())
            .field("parent_space_labels", &self.parent_space_labels.len())
            .field(
                "newest_message_text",
                &self.newest_message_text.as_ref().map(|_| "MessageText(..)"),
            )
            .field(
                "direct_partner",
                &self.direct_partner.as_ref().map(|_| "DirectPartner(..)"),
            )
            .finish()
    }
}

/// Every room with unread attention, for the device-local delivery channel.
///
/// The read/unread and mute semantics are exactly
/// [`native_attention_projection_from_rooms_with_live_room`]'s: the same deduplication,
/// the same explicit-mute and `Mute` mode exclusions, the same low-priority exclusion,
/// the same ignored-user direct-message exclusion, the same `Mentions`-mode collapse,
/// and the same display label. Two things the transient alert needs are deliberately not
/// required here, because a digest is not a banner: a room does not have to be the live
/// room, and it does not have to carry a usable latest event.
pub fn attention_room_candidates(
    input: NativeAttentionProjectionInput<'_>,
) -> Vec<RoomAttentionCandidate> {
    let mut seen_room_ids = BTreeSet::new();
    let mut candidates = Vec::new();

    for room in input.rooms {
        if !seen_room_ids.insert(room.room_id.as_str()) {
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
            continue;
        }
        if room.tags.low_priority.is_some() {
            continue;
        }
        // An ignored user's direct message keeps its shared Dock contribution, but it is
        // not attention: it stays out of the totals and out of the candidates.
        if room.is_dm
            && room
                .dm_user_ids
                .iter()
                .any(|user_id| input.ignored_user_ids.contains(user_id))
        {
            continue;
        }

        let highlight_total = room
            .highlight_count
            .saturating_add(room.thread_highlight_count);
        let activity_unread_count = room_activity_unread_count(room);
        // `Mentions` only reports what was highlighted; without a highlight the room is
        // read as far as this projection is concerned.
        let effective_unread_count =
            if mode == RoomNotificationMode::Mentions && highlight_total == 0 {
                0
            } else {
                activity_unread_count
            };
        if effective_unread_count == 0 {
            continue;
        }
        let Some(summary) = room_attention_summary(
            room.display_label.clone(),
            room.is_dm,
            if mode == RoomNotificationMode::Mentions {
                0
            } else {
                room.notification_count
            },
            highlight_total,
            effective_unread_count,
        ) else {
            continue;
        };
        candidates.push(RoomAttentionCandidate {
            room_id: room.room_id.clone(),
            display_label: summary.room_display_name,
            kind: summary.kind,
            unread: u32::try_from(effective_unread_count).unwrap_or(u32::MAX),
            mentions: u32::try_from(highlight_total).unwrap_or(u32::MAX),
            parent_space_ids: room.parent_space_ids.clone(),
            parent_space_labels: Vec::new(),
            newest_message_text: room
                .latest_event
                .as_ref()
                .filter(|event| !event.is_redacted)
                .and_then(|event| event.message_text.as_deref())
                .and_then(bounded_notification_delivery_message_source),
            direct_partner: match room.dm_user_ids.as_slice() {
                [user_id] if room.is_dm => Some(DirectPartner {
                    user_id: user_id.clone(),
                    local_alias: None,
                }),
                _ => None,
            },
        });
    }

    candidates.sort_by(|left, right| {
        attention_kind_priority(right.kind)
            .cmp(&attention_kind_priority(left.kind))
            .then_with(|| right.mentions.cmp(&left.mentions))
            .then_with(|| right.unread.cmp(&left.unread))
            .then_with(|| left.display_label.cmp(&right.display_label))
            .then_with(|| left.room_id.cmp(&right.room_id))
    });
    candidates
}

/// Every room with unread attention in this state, for the device-local delivery channel.
///
/// The input assembly mirrors `crate::reducer::native_attention`'s production projection -
/// the same room list, the same room notification modes, the same ignored users - and shares
/// [`attention_room_notification_modes`] with it, so the delivery channel reads exactly the
/// attention state the badge is built from. The fields the candidate rule does not read
/// (observation kind, the previous candidate, the platform capabilities) are filled with the
/// neutral value, because this function answers one question about rooms only.
pub fn attention_room_candidates_for_state(state: &AppState) -> Vec<RoomAttentionCandidate> {
    let room_notification_modes = attention_room_notification_modes(state);
    let mut candidates = attention_room_candidates(NativeAttentionProjectionInput {
        rooms: &state.rooms,
        active_room_id: state.navigation.active_room_id.as_deref(),
        muted_room_ids: &[],
        room_notification_modes: &room_notification_modes,
        ignored_user_ids: &state.profile.ignored_user_ids,
        window_focused: state.native_attention_context.window_focused,
        observation: NativeAttentionObservationKind::InitialSync,
        previous_candidate: state.native_attention.summary.candidate.as_ref(),
        message_previews: state.settings.values.notifications.message_previews,
        capabilities: state.native_attention.summary.capabilities,
        locale: crate::resolve_catalog_locale(&state.settings.values.locale),
    });
    for candidate in &mut candidates {
        if let Some(partner) = candidate.direct_partner.as_mut() {
            partner.local_alias = state
                .profile
                .local_aliases
                .get(&partner.user_id)
                .map(|alias| alias.trim().to_owned())
                .filter(|alias| !alias.is_empty());
        }
        candidate.parent_space_labels = candidate
            .parent_space_ids
            .iter()
            .filter_map(|space_id| {
                state
                    .spaces
                    .iter()
                    .find(|space| &space.space_id == space_id)
                    .map(|space| space.display_name.clone())
            })
            .filter(|label| !label.trim().is_empty())
            .take(MAX_ATTENTION_PARENT_SPACE_LABELS)
            .collect();
    }
    candidates
}

/// A digest names at most this many parent spaces per room; a room in more spaces is
/// still covered by every watched space, the line only names the first ones.
pub const MAX_ATTENTION_PARENT_SPACE_LABELS: usize = 2;

/// The per-room notification modes the attention projection reads, keyed by room id.
///
/// One function, so the badge's projection and the delivery channel's candidates can never
/// read two different mute tables (#1427).
pub fn attention_room_notification_modes(
    state: &AppState,
) -> HashMap<String, RoomNotificationMode> {
    state
        .room_notification_settings
        .iter()
        .map(|(room_id, settings)| (room_id.clone(), settings.mode))
        .collect()
}

/// Compose the OS notification the adapter may show for the selected candidate.
///
/// A room-level unread count proves the number, but does not prove that the
/// room's latest event is itself unread. The alert navigates to the room, never to
/// an unverified event or thread. Its body is the count body unless the user turned
/// message previews on and [`verified_native_preview`] can stand the latest event
/// for the alert (#994).
fn native_notification_payload(
    entry: &NativeAttentionCandidateEntry<'_>,
    candidate: &NativeAttentionCandidate,
    previews_allowed: bool,
    locale: CatalogLocale,
) -> NativeNotificationPayload {
    let preview = previews_allowed
        .then(|| verified_native_preview(entry))
        .flatten();
    NativeNotificationPayload {
        title: native_notification_title(&candidate.room_display_name, candidate.kind, locale),
        body: preview.unwrap_or_else(|| native_notification_count_body(candidate, locale)),
        target: NativeNotificationTarget {
            room_id: entry.room_id.to_owned(),
            event_id: None,
            thread_root_event_id: None,
        },
    }
}

/// The longest OS preview, in grapheme clusters. A banner shows a line or two; the
/// rest of the message stays in the app.
pub const MAX_NATIVE_NOTIFICATION_PREVIEW_GRAPHEMES: usize = 120;

/// The latest event's text, when it may stand for the alert (#994).
///
/// Verified means: the room is the one whose unread count rose in this live
/// observation, its latest event changed in that same observation and was sent by
/// another user, it is neither redacted nor an edit or reaction, and it carries
/// decrypted plain text. A thread reply qualifies because it is the latest
/// event. Anything else falls back to the count body.
fn verified_native_preview(entry: &NativeAttentionCandidateEntry<'_>) -> Option<String> {
    if !entry.is_verified_live_room {
        return None;
    }
    let event = entry.room.latest_event.as_ref()?;
    if event.is_redacted
        || event.event_id.is_empty()
        || matches!(
            event.relation_type.as_deref(),
            Some("m.replace" | "m.annotation")
        )
    {
        return None;
    }
    let flattened = event
        .message_text
        .as_deref()?
        .split(|character: char| {
            character.is_whitespace() || character.is_control() || character == '\u{2028}'
        })
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if flattened.is_empty() {
        return None;
    }
    use unicode_segmentation::UnicodeSegmentation;
    // The ellipsis counts toward the bound, so a cut preview is exactly the maximum.
    if flattened
        .graphemes(true)
        .nth(MAX_NATIVE_NOTIFICATION_PREVIEW_GRAPHEMES)
        .is_none()
    {
        return Some(flattened);
    }
    let bounded: String = flattened
        .graphemes(true)
        .take(MAX_NATIVE_NOTIFICATION_PREVIEW_GRAPHEMES - 1)
        .collect();
    Some(format!("{bounded}…"))
}

fn native_notification_title(
    room_display_name: &str,
    kind: RoomAttentionKind,
    locale: CatalogLocale,
) -> String {
    let place = room_display_name;
    notification_text(
        locale,
        match kind {
            RoomAttentionKind::Mention => NotificationText::MentionIn { place },
            RoomAttentionKind::Dm => NotificationText::DirectMessageIn { place },
            RoomAttentionKind::Message => NotificationText::MessageIn { place },
        },
    )
}

/// The count body: the mention count for a mention, then the unread count,
/// zero counts left out.
fn native_notification_count_body(
    candidate: &NativeAttentionCandidate,
    locale: CatalogLocale,
) -> String {
    let mentions = if candidate.kind == RoomAttentionKind::Mention {
        candidate.highlight_count
    } else {
        0
    };
    notification_counts(locale, mentions, candidate.unread_count)
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
#[path = "native_attention/preview_tests.rs"]
mod preview_tests;
#[cfg(test)]
#[path = "native_attention/tests.rs"]
mod tests;
