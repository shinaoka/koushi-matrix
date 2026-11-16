//! Per-room derived profile table (#1915).
//!
//! One entry per joined room holds the derived, presentation-ready data that
//! room-level projections read: labels, notification mode, unread counts and
//! the latest displayable event. Each entry carries a revision that changes
//! only when one of its inputs changes, so a consumer can skip rooms (or
//! whole rebuilds) whose revisions it has already projected.
//!
//! Activity is the only consumer today. The table deliberately knows nothing
//! about Activity rows so the room list and attention projections can adopt
//! it later without a second derivation of the same data.

use std::collections::HashMap;

use koushi_state::{
    AppState, AvatarImage, RoomLatestEventSummary, RoomNotificationMode, RoomSummary, SpaceSummary,
};

/// The latest event of a room that a room-level projection may display: not
/// redacted, not an edit or reaction, and in step with the room's
/// conversation activity.
pub(crate) fn latest_display_event(room: &RoomSummary) -> Option<&RoomLatestEventSummary> {
    let latest = room.latest_event.as_ref()?;
    if latest.is_redacted
        || matches!(
            latest.relation_type.as_deref(),
            Some("m.replace" | "m.annotation")
        )
    {
        return None;
    }
    if let Some(conversation_activity) = room.conversation_activity
        && conversation_activity.timestamp_ms != latest.timestamp_ms
    {
        return None;
    }
    // Older room-list injectors only provide the already-filtered latest event
    // and omit the separate conversation-activity projection. Production
    // projections keep that field in sync, while the producer-side filter in
    // koushi-sdk ensures state/profile events never reach this fallback.
    Some(latest)
}

/// A room's context label: `DM`, `<space> / <room>` for a room in a known
/// Space, otherwise the room label.
pub(crate) fn room_context_label(room: &RoomSummary, spaces: &[SpaceSummary]) -> String {
    if room.is_dm {
        return "DM".to_owned();
    }
    let parent_space = room
        .parent_space_ids
        .iter()
        .filter_map(|space_id| spaces.iter().find(|space| space.space_id == *space_id))
        .next()
        .or_else(|| {
            spaces.iter().find(|space| {
                space
                    .child_room_ids
                    .iter()
                    .any(|room_id| room_id == &room.room_id)
            })
        });
    if let Some(space) = parent_space {
        return format!("{} / {}", space.display_name, room.display_label);
    }
    room.display_label.clone()
}

/// The raw unread metrics of a room as the room list reported them.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) struct RoomUnreadCounts {
    pub(crate) unread_count: u64,
    pub(crate) notification_count: u64,
    pub(crate) highlight_count: u64,
    pub(crate) thread_unread_count: u64,
    pub(crate) thread_highlight_count: u64,
    pub(crate) marked_unread: bool,
}

impl RoomUnreadCounts {
    pub(crate) fn of(room: &RoomSummary) -> Self {
        Self {
            unread_count: room.unread_count,
            notification_count: room.notification_count,
            highlight_count: room.highlight_count,
            thread_unread_count: room.thread_unread_count,
            thread_highlight_count: room.thread_highlight_count,
            marked_unread: room.marked_unread,
        }
    }
}

/// The displayable latest event of a room, owned by its profile entry.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct RoomProfileLatestEvent {
    pub(crate) event_id: String,
    pub(crate) thread_root_event_id: Option<String>,
    pub(crate) sender_id: Option<String>,
    pub(crate) sender_label: Option<String>,
    /// The sender avatar embedded in the room-list summary. Consumers resolve
    /// the live profile avatar themselves; this is only the fallback.
    pub(crate) sender_avatar: Option<AvatarImage>,
    pub(crate) preview: Option<String>,
    pub(crate) timestamp_ms: u64,
}

impl RoomProfileLatestEvent {
    fn of(latest: &RoomLatestEventSummary) -> Self {
        Self {
            event_id: latest.event_id.clone(),
            thread_root_event_id: latest.thread_root_event_id.clone(),
            sender_id: latest.sender_id.clone(),
            sender_label: latest.sender_label.clone(),
            sender_avatar: latest.sender_avatar.clone(),
            preview: latest.preview.clone(),
            timestamp_ms: latest.timestamp_ms,
        }
    }

    fn matches(&self, latest: &RoomLatestEventSummary) -> bool {
        self.event_id == latest.event_id
            && self.thread_root_event_id == latest.thread_root_event_id
            && self.sender_id == latest.sender_id
            && self.sender_label == latest.sender_label
            && self.sender_avatar == latest.sender_avatar
            && self.preview == latest.preview
            && self.timestamp_ms == latest.timestamp_ms
    }
}

/// The derived profile of one room. Fields are read-only for consumers; the
/// table replaces an entry, with a new revision, when its inputs change.
pub(crate) struct RoomProfile {
    revision: u64,
    seen_epoch: u64,
    pub(crate) display_label: String,
    pub(crate) context_label: String,
    pub(crate) notification_mode: Option<RoomNotificationMode>,
    pub(crate) low_priority: bool,
    pub(crate) counts: RoomUnreadCounts,
    /// The timestamp of the room's newest conversation activity, falling back
    /// to the displayable latest event, or 0 when neither is known.
    pub(crate) activity_timestamp_ms: u64,
    pub(crate) latest_display_event: Option<RoomProfileLatestEvent>,
    // Inputs of `context_label` other than the label itself.
    is_dm: bool,
    parent_space_ids: Vec<String>,
    conversation_timestamp_ms: Option<u64>,
}

impl RoomProfile {
    fn of(
        room: &RoomSummary,
        mode: Option<RoomNotificationMode>,
        spaces: &[SpaceSummary],
        revision: u64,
        seen_epoch: u64,
    ) -> Self {
        let latest_display_event = latest_display_event(room);
        let conversation_timestamp_ms = room
            .conversation_activity
            .map(|activity| activity.timestamp_ms);
        Self {
            revision,
            seen_epoch,
            display_label: room.display_label.clone(),
            context_label: room_context_label(room, spaces),
            notification_mode: mode,
            low_priority: room.tags.low_priority.is_some(),
            counts: RoomUnreadCounts::of(room),
            activity_timestamp_ms: conversation_timestamp_ms
                .or_else(|| latest_display_event.map(|event| event.timestamp_ms))
                .unwrap_or_default(),
            latest_display_event: latest_display_event.map(RoomProfileLatestEvent::of),
            is_dm: room.is_dm,
            parent_space_ids: room.parent_space_ids.clone(),
            conversation_timestamp_ms,
        }
    }

    /// Whether this entry still describes `room` under `mode`, compared
    /// field by field without allocating.
    fn matches(&self, room: &RoomSummary, mode: Option<RoomNotificationMode>) -> bool {
        let latest_matches = match (
            self.latest_display_event.as_ref(),
            latest_display_event(room),
        ) {
            (None, None) => true,
            (Some(stored), Some(latest)) => stored.matches(latest),
            _ => false,
        };
        latest_matches
            && self.notification_mode == mode
            && self.counts == RoomUnreadCounts::of(room)
            && self.conversation_timestamp_ms
                == room
                    .conversation_activity
                    .map(|activity| activity.timestamp_ms)
            && self.low_priority == room.tags.low_priority.is_some()
            && self.display_label == room.display_label
            && self.is_dm == room.is_dm
            && self.parent_space_ids == room.parent_space_ids
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the per-entry revision is the table contract for the room-list and attention consumers to come; Activity consumes refresh change sets"
        )
    )]
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// The client-observed main plus thread unread count. Server-only
    /// notification counts and manual flags never prove an unread message.
    pub(crate) fn confirmed_unread_count(&self) -> u64 {
        self.counts
            .unread_count
            .saturating_add(self.counts.thread_unread_count)
    }

    pub(crate) fn is_muted(&self) -> bool {
        self.notification_mode == Some(RoomNotificationMode::Mute)
    }
}

/// The Space fields a context label reads; kept so a Space change can be
/// detected without retaining whole Space summaries.
struct SpaceLabelSource {
    space_id: String,
    display_name: String,
    child_room_ids: Vec<String>,
}

impl SpaceLabelSource {
    fn matches(&self, space: &SpaceSummary) -> bool {
        self.space_id == space.space_id
            && self.display_name == space.display_name
            && self.child_room_ids == space.child_room_ids
    }
}

/// What one [`RoomProfileTable::refresh`] changed.
#[derive(Default)]
pub(crate) struct RoomProfileChanges {
    /// Indices into `state.rooms` of rooms whose entry was added or replaced.
    pub(crate) updated: Vec<usize>,
    /// Rooms whose entry was dropped because they left the room list.
    pub(crate) removed: Vec<String>,
}

impl RoomProfileChanges {
    pub(crate) fn is_empty(&self) -> bool {
        self.updated.is_empty() && self.removed.is_empty()
    }
}

#[derive(Default)]
pub(crate) struct RoomProfileTable {
    entries: HashMap<String, RoomProfile>,
    spaces: Vec<SpaceLabelSource>,
    next_revision: u64,
    epoch: u64,
}

impl RoomProfileTable {
    pub(crate) fn get(&self, room_id: &str) -> Option<&RoomProfile> {
        self.entries.get(room_id)
    }

    /// Bring every entry in line with `state`. Unchanged rooms cost one
    /// field-by-field comparison and keep their revision.
    pub(crate) fn refresh(&mut self, state: &AppState) -> RoomProfileChanges {
        let mut changes = RoomProfileChanges::default();
        self.epoch = self.epoch.wrapping_add(1);
        let epoch = self.epoch;
        let spaces_changed = self.spaces.len() != state.spaces.len()
            || self
                .spaces
                .iter()
                .zip(&state.spaces)
                .any(|(stored, space)| !stored.matches(space));
        if spaces_changed {
            self.spaces = state
                .spaces
                .iter()
                .map(|space| SpaceLabelSource {
                    space_id: space.space_id.clone(),
                    display_name: space.display_name.clone(),
                    child_room_ids: space.child_room_ids.clone(),
                })
                .collect();
        }

        let mut seen = 0usize;
        for (index, room) in state.rooms.iter().enumerate() {
            let mode = state
                .room_notification_settings
                .get(&room.room_id)
                .map(|settings| settings.mode);
            match self.entries.get_mut(&room.room_id) {
                Some(entry) if entry.seen_epoch == epoch => {
                    // A duplicate room id: the first occurrence owns the entry.
                }
                Some(entry) if entry.matches(room, mode) => {
                    entry.seen_epoch = epoch;
                    seen += 1;
                    if spaces_changed {
                        let context_label = room_context_label(room, &state.spaces);
                        if context_label != entry.context_label {
                            entry.context_label = context_label;
                            entry.revision = next_revision(&mut self.next_revision);
                            changes.updated.push(index);
                        }
                    }
                }
                Some(entry) => {
                    let revision = next_revision(&mut self.next_revision);
                    *entry = RoomProfile::of(room, mode, &state.spaces, revision, epoch);
                    seen += 1;
                    changes.updated.push(index);
                }
                None => {
                    let revision = next_revision(&mut self.next_revision);
                    self.entries.insert(
                        room.room_id.clone(),
                        RoomProfile::of(room, mode, &state.spaces, revision, epoch),
                    );
                    seen += 1;
                    changes.updated.push(index);
                }
            }
        }
        if seen < self.entries.len() {
            self.entries.retain(|room_id, entry| {
                let keep = entry.seen_epoch == epoch;
                if !keep {
                    changes.removed.push(room_id.clone());
                }
                keep
            });
        }
        changes
    }
}

fn next_revision(next: &mut u64) -> u64 {
    *next = next.wrapping_add(1);
    *next
}

#[cfg(test)]
mod tests;
