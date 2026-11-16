use std::collections::{BTreeMap, BTreeSet, HashMap};

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_state::{
    ActivityMarkReadTarget, ActivityResolutionState, ActivityRow, ActivityRowKind, ActivityState,
    ActivityStream, ActivityTab, AppAction, AppState, AvatarImage, AvatarThumbnailState,
    RoomLatestEventSummary, RoomNotificationMode, RoomSummary, SpaceSummary,
    room_activity_unread_count,
};

use super::{ActivityResolutionRequest, RequestId};

const MAX_ACTIVITY_RESOLUTION_ROOMS: usize = 16;
const MAX_CANONICAL_ROOM_SLOTS: usize = 512;
// Keep this aligned with the reviewed ROOM_REPLAY_INITIAL_ITEMS_MAX window.
const MAX_CANONICAL_ROWS_PER_ROOM: usize = 120;
const MAX_CANONICAL_ROWS_GLOBAL: usize = 2_048;
const MAX_ACTIVITY_REDACTION_TOMBSTONES: usize = 2_048;
const MAX_ACTIVITY_CLEARED_EVENTS: usize = 2_048;

pub const ACTIVITY_RECENT_MAX_ROWS: usize = 200;

pub(super) fn activity_tab_token(tab: ActivityTab) -> &'static str {
    match tab {
        ActivityTab::Recent => "recent",
        ActivityTab::Unread => "unread",
    }
}

pub(super) fn record_activity_transition(
    stage: &'static str,
    request_id: RequestId,
    outcome: &'static str,
    previous_tab: ActivityTab,
    selected_tab: ActivityTab,
) {
    record(
        DiagnosticEvent::new(DiagnosticLevel::Info, "core.activity", stage)
            .field(DiagnosticField::request_id(
                "request_id",
                request_id.connection_id.0,
                request_id.sequence,
            ))
            .field(DiagnosticField::token("outcome", outcome))
            .field(DiagnosticField::token(
                "previous_tab",
                activity_tab_token(previous_tab),
            ))
            .field(DiagnosticField::token(
                "selected_tab",
                activity_tab_token(selected_tab),
            )),
    );
}

#[derive(Default)]
pub(super) struct ActivityProjection {
    canonical_rows_by_room: BTreeMap<String, BTreeMap<String, ActivityRow>>,
    resolution_rows_by_event_id: BTreeMap<String, ActivityRow>,
    /// The exact fully-read marker against which resolver rows were selected.
    resolution_marker_by_room: BTreeMap<String, String>,
    /// Markers captured before dispatch, not markers observed when results return.
    requested_resolution_marker_by_room: BTreeMap<String, String>,
    redacted_event_ids: BTreeSet<String>,
    hidden_event_ids_by_room: BTreeMap<String, BTreeSet<String>>,
    invalidated_placeholder_room_ids: BTreeSet<String>,
    cleared_event_ids: BTreeSet<String>,
    room_ordinals: BTreeMap<String, u64>,
    canonical_row_ordinals: BTreeMap<(String, String), u64>,
    resolution_row_ordinals: BTreeMap<String, u64>,
    redaction_ordinals: BTreeMap<String, u64>,
    cleared_event_ordinals: BTreeMap<String, u64>,
    next_ordinal: u64,
    /// Room-unread candidates, keyed by room with their activity timestamp,
    /// that the latest resolution generation attempted. A live update only
    /// schedules resolution for a candidate absent here or newer (#1061), so
    /// a no-progress settlement stays retryable instead of looping.
    resolution_attempts: BTreeMap<String, u64>,
    /// Thumbnail states for avatars copied into cached rows. `ActivityRow`
    /// itself owns those copies, so the AppState reducer's
    /// `AvatarThumbnailUpdated` settlement cannot reach them without this
    /// projection-local record.
    avatar_thumbnails_by_mxc: BTreeMap<String, AvatarThumbnailState>,
    /// #1915: last recorded `activity_placeholder` unread trace per room, so a
    /// rebuild of an unchanged room does not append another record.
    placeholder_traces: crate::unread_trace::UnreadTraceLedger,
}

#[derive(Default)]
pub(super) struct ActivityMarkReadResult {
    pub(super) cleared_event_ids: Vec<String>,
    pub(super) cleared_placeholder_room_ids: Vec<String>,
}

fn activity_latest_display_event(room: &RoomSummary) -> Option<&RoomLatestEventSummary> {
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

fn activity_latest_display_event_id(room: &RoomSummary) -> Option<&str> {
    let latest = activity_latest_display_event(room)?;
    (!latest.event_id.trim().is_empty()).then_some(latest.event_id.as_str())
}

impl ActivityProjection {
    /// Keep a downloaded avatar visible when the only remaining copy lives in
    /// this projection's rows. Called for every `AvatarThumbnailUpdated`; the
    /// map stays bounded to avatars the cached rows still reference.
    pub(super) fn note_avatar_thumbnail(
        &mut self,
        mxc_uri: &str,
        thumbnail: &AvatarThumbnailState,
    ) {
        if !self.cached_rows_reference_avatar(mxc_uri) {
            return;
        }
        self.avatar_thumbnails_by_mxc
            .insert(mxc_uri.to_owned(), thumbnail.clone());
    }

    fn cached_rows_reference_avatar(&self, mxc_uri: &str) -> bool {
        self.cached_rows().any(|row| {
            row.sender_avatar
                .as_ref()
                .is_some_and(|avatar| avatar.mxc_uri == mxc_uri)
        })
    }

    fn cached_rows(&self) -> impl Iterator<Item = &ActivityRow> {
        self.canonical_rows_by_room
            .values()
            .flat_map(BTreeMap::values)
            .chain(self.resolution_rows_by_event_id.values())
    }

    fn retain_projecting_avatars(&mut self) {
        let mut thumbnails = std::mem::take(&mut self.avatar_thumbnails_by_mxc);
        {
            let referenced = self
                .cached_rows()
                .filter_map(|row| {
                    row.sender_avatar
                        .as_ref()
                        .map(|avatar| avatar.mxc_uri.as_str())
                })
                .collect::<BTreeSet<_>>();
            thumbnails.retain(|mxc_uri, _| referenced.contains(mxc_uri.as_str()));
        }
        self.avatar_thumbnails_by_mxc = thumbnails;
    }

    fn settled_avatar(&self, avatar: Option<AvatarImage>) -> Option<AvatarImage> {
        let mut avatar = avatar?;
        // The recorded action state is authoritative for this MXC: it also
        // carries the `NotRequested` downgrade that retires evicted bytes.
        if let Some(thumbnail) = self.avatar_thumbnails_by_mxc.get(&avatar.mxc_uri) {
            avatar.thumbnail = thumbnail.clone();
        }
        Some(avatar)
    }

    /// Resolve a sender avatar from the authoritative profile state first, so
    /// the projection never joins profile maps in React and never relies on a
    /// stale row copy when a live profile has the same source.
    fn resolved_sender_avatar(
        &self,
        state: &AppState,
        own_user_id: Option<&str>,
        room_id: &str,
        sender_id: Option<&str>,
        embedded: Option<&AvatarImage>,
    ) -> Option<AvatarImage> {
        let Some(sender_id) = sender_id else {
            return self.settled_avatar(embedded.cloned());
        };
        let own_profile_avatar = (own_user_id == Some(sender_id))
            .then_some(state.profile.own.avatar.as_ref())
            .flatten();
        let profile_avatar = own_profile_avatar
            .or_else(|| {
                state
                    .profile
                    .room_users
                    .get(room_id)
                    .and_then(|profiles| profiles.get(sender_id))
                    .and_then(|profile| profile.avatar.as_ref())
            })
            .or_else(|| {
                state
                    .profile
                    .users
                    .get(sender_id)
                    .and_then(|profile| profile.avatar.as_ref())
            });
        // A same-source row copy may already carry a settled thumbnail while
        // the profile entry is still NotRequested; keep the settled state.
        let avatar = match (profile_avatar, embedded) {
            (Some(profile), Some(embedded))
                if profile.mxc_uri == embedded.mxc_uri
                    && profile.thumbnail == AvatarThumbnailState::NotRequested
                    && embedded.thumbnail != AvatarThumbnailState::NotRequested =>
            {
                Some(embedded.clone())
            }
            (Some(profile), _) => Some(profile.clone()),
            (None, Some(embedded)) => Some(embedded.clone()),
            (None, None) => None,
        };
        self.settled_avatar(avatar)
    }

    /// Compatibility path for older tests/injectors. Production timelines use
    /// `CanonicalActivityWindowReconciled`; this path replaces each observed
    /// room too, spilling only its over-bound detail into resolver provenance.
    pub(super) fn ingest(&mut self, rows: Vec<ActivityRow>) {
        if rows.is_empty() {
            self.canonical_rows_by_room.clear();
            self.hidden_event_ids_by_room.clear();
            self.room_ordinals.clear();
            self.canonical_row_ordinals.clear();
            self.resolution_rows_by_event_id.clear();
            self.resolution_marker_by_room.clear();
            self.resolution_row_ordinals.clear();
            return;
        }

        let mut rows_by_room = BTreeMap::<String, Vec<ActivityRow>>::new();
        for row in rows {
            if let Some(row) = Self::sanitize_row(row) {
                rows_by_room
                    .entry(row.room_id.clone())
                    .or_default()
                    .push(row);
            }
        }
        for (room_id, rows) in rows_by_room {
            self.remove_resolution_rows_for_room(&room_id);
            let mut canonical_rows = rows;
            canonical_rows.sort_by(activity_row_newest_first);
            let overflow =
                canonical_rows.split_off(MAX_CANONICAL_ROWS_PER_ROOM.min(canonical_rows.len()));
            self.reconcile_canonical_window(room_id, canonical_rows, Vec::new(), Vec::new());
            for row in overflow {
                self.insert_resolution_row(row);
            }
        }
        self.enforce_resolution_bound();
    }

    pub(super) fn ingest_resolution_rows(&mut self, rows: Vec<ActivityRow>, state: &AppState) {
        for row in rows {
            let Some(row) = Self::sanitize_row(row) else {
                continue;
            };
            let current_marker = state
                .live_signals
                .rooms
                .get(&row.room_id)
                .and_then(|signals| signals.fully_read_event_id.as_ref());
            let Some(requested_marker) = self.requested_resolution_marker_by_room.get(&row.room_id)
            else {
                continue;
            };
            if current_marker != Some(requested_marker) {
                continue;
            }
            self.resolution_marker_by_room
                .insert(row.room_id.clone(), requested_marker.clone());
            self.insert_resolution_row(row);
        }
        self.enforce_resolution_bound();
    }

    pub(super) fn reconcile_canonical_window(
        &mut self,
        room_id: String,
        rows: Vec<ActivityRow>,
        redacted_event_ids: Vec<String>,
        hidden_event_ids: Vec<String>,
    ) {
        if room_id.trim().is_empty() {
            return;
        }

        self.remove_canonical_room(&room_id);
        self.hidden_event_ids_by_room.remove(&room_id);

        let hidden_event_ids = bounded_event_ids(hidden_event_ids);
        if !hidden_event_ids.is_empty() {
            self.hidden_event_ids_by_room
                .insert(room_id.clone(), hidden_event_ids);
        }

        for event_id in redacted_event_ids {
            if event_id.trim().is_empty() {
                continue;
            }
            self.invalidate_event(&room_id, &event_id);
        }

        let hidden = self
            .hidden_event_ids_by_room
            .get(&room_id)
            .cloned()
            .unwrap_or_default();
        let mut deduplicated = BTreeMap::new();
        for row in rows {
            let Some(row) = Self::sanitize_row(row) else {
                continue;
            };
            let Some(event_id) = row.event_id.as_ref() else {
                continue;
            };
            if self.redacted_event_ids.contains(event_id) || hidden.contains(event_id) {
                continue;
            }
            // BTreeMap::insert gives the last row in the accepted window
            // ownership of a duplicate stable identity.
            deduplicated.insert(event_id.clone(), row);
        }

        let mut canonical_rows = deduplicated.into_values().collect::<Vec<_>>();
        canonical_rows.sort_by(activity_row_newest_first);
        canonical_rows.truncate(MAX_CANONICAL_ROWS_PER_ROOM);
        let ordinal = self.next_ordinal();
        let mut stored_rows = BTreeMap::new();
        for row in canonical_rows {
            let event_id = row
                .event_id
                .as_ref()
                .expect("sanitized activity rows have an event id")
                .clone();
            self.canonical_row_ordinals
                .insert((room_id.clone(), event_id.clone()), ordinal);
            stored_rows.insert(event_id, row);
        }
        if !stored_rows.is_empty() {
            self.canonical_rows_by_room
                .insert(room_id.clone(), stored_rows);
        }

        if self
            .canonical_rows_by_room
            .get(&room_id)
            .is_some_and(|rows| !rows.is_empty())
            || self
                .hidden_event_ids_by_room
                .get(&room_id)
                .is_some_and(|ids| !ids.is_empty())
        {
            self.room_ordinals.insert(room_id.clone(), ordinal);
        } else {
            self.hidden_event_ids_by_room.remove(&room_id);
            self.room_ordinals.remove(&room_id);
        }

        self.enforce_room_slot_bound();
        self.enforce_canonical_global_bound();
        self.enforce_redaction_bound();
        self.enforce_cleared_event_bound();
    }

    fn sanitize_row(mut row: ActivityRow) -> Option<ActivityRow> {
        if row.kind != ActivityRowKind::Event
            || row
                .event_id
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
            || row.room_id.trim().is_empty()
            || row.preview.is_none()
        {
            return None;
        }
        row.room_label.clear();
        row.unread = false;
        Some(row)
    }

    fn next_ordinal(&mut self) -> u64 {
        let ordinal = self.next_ordinal;
        self.next_ordinal = self.next_ordinal.saturating_add(1);
        ordinal
    }

    fn remove_canonical_room(&mut self, room_id: &str) {
        self.canonical_rows_by_room.remove(room_id);
        self.canonical_row_ordinals
            .retain(|(stored_room_id, _), _| stored_room_id != room_id);
        self.room_ordinals.remove(room_id);
    }

    fn remove_resolution_rows_for_room(&mut self, room_id: &str) {
        self.resolution_marker_by_room.remove(room_id);
        let event_ids = self
            .resolution_rows_by_event_id
            .iter()
            .filter(|(_, row)| row.room_id == room_id)
            .map(|(event_id, _)| event_id.clone())
            .collect::<Vec<_>>();
        for event_id in event_ids {
            self.resolution_rows_by_event_id.remove(&event_id);
            self.resolution_row_ordinals.remove(&event_id);
        }
    }

    fn insert_resolution_row(&mut self, row: ActivityRow) {
        let Some(event_id) = row.event_id.as_ref() else {
            return;
        };
        if self.redacted_event_ids.contains(event_id)
            || self
                .hidden_event_ids_by_room
                .get(&row.room_id)
                .is_some_and(|ids| ids.contains(event_id))
        {
            return;
        }
        let event_id = event_id.clone();
        self.resolution_rows_by_event_id
            .insert(event_id.clone(), row);
        let ordinal = self.next_ordinal();
        self.resolution_row_ordinals.insert(event_id, ordinal);
    }

    fn invalidate_event(&mut self, room_id: &str, event_id: &str) {
        self.canonical_rows_by_room.values_mut().for_each(|rows| {
            rows.remove(event_id);
        });
        self.canonical_rows_by_room
            .retain(|_, rows| !rows.is_empty());
        self.canonical_row_ordinals
            .retain(|(_, stored_event_id), _| stored_event_id != event_id);
        self.resolution_rows_by_event_id.remove(event_id);
        self.resolution_row_ordinals.remove(event_id);
        self.cleared_event_ids.remove(event_id);
        self.cleared_event_ordinals.remove(event_id);
        self.invalidated_placeholder_room_ids.remove(room_id);
        self.redacted_event_ids.insert(event_id.to_owned());
        let ordinal = self.next_ordinal();
        self.redaction_ordinals.insert(event_id.to_owned(), ordinal);
    }

    fn active_event(&self, event_id: &str) -> bool {
        self.canonical_rows_by_room
            .values()
            .any(|rows| rows.contains_key(event_id))
            || self.resolution_rows_by_event_id.contains_key(event_id)
    }

    fn enforce_room_slot_bound(&mut self) {
        while self.room_ordinals.len() > MAX_CANONICAL_ROOM_SLOTS {
            let Some((room_id, _)) = self
                .room_ordinals
                .iter()
                .min_by_key(|(room_id, ordinal)| (**ordinal, (*room_id).clone()))
                .map(|(room_id, ordinal)| (room_id.clone(), *ordinal))
            else {
                break;
            };
            self.remove_canonical_room(&room_id);
            self.hidden_event_ids_by_room.remove(&room_id);
        }
    }

    fn enforce_canonical_global_bound(&mut self) {
        while self.canonical_row_ordinals.len() > MAX_CANONICAL_ROWS_GLOBAL {
            let Some((room_id, event_id)) = self
                .canonical_row_ordinals
                .iter()
                .min_by_key(|((room_id, event_id), ordinal)| {
                    (**ordinal, room_id.clone(), event_id.clone())
                })
                .map(|((room_id, event_id), _)| (room_id.clone(), event_id.clone()))
            else {
                break;
            };
            self.canonical_row_ordinals
                .remove(&(room_id.clone(), event_id.clone()));
            if let Some(rows) = self.canonical_rows_by_room.get_mut(&room_id) {
                rows.remove(&event_id);
                if rows.is_empty() {
                    self.canonical_rows_by_room.remove(&room_id);
                }
            }
            if !self.canonical_rows_by_room.contains_key(&room_id)
                && !self.hidden_event_ids_by_room.contains_key(&room_id)
            {
                self.room_ordinals.remove(&room_id);
            }
        }
    }

    fn enforce_resolution_bound(&mut self) {
        while self.resolution_rows_by_event_id.len() > ACTIVITY_RECENT_MAX_ROWS {
            let Some(event_id) = self
                .resolution_row_ordinals
                .iter()
                .min_by_key(|(event_id, ordinal)| (**ordinal, (*event_id).clone()))
                .map(|(event_id, _)| event_id.clone())
            else {
                break;
            };
            self.resolution_row_ordinals.remove(&event_id);
            self.resolution_rows_by_event_id.remove(&event_id);
        }
    }

    fn enforce_redaction_bound(&mut self) {
        while self.redacted_event_ids.len() > MAX_ACTIVITY_REDACTION_TOMBSTONES {
            let Some(event_id) = self
                .redaction_ordinals
                .iter()
                .filter(|(event_id, _)| !self.active_event(event_id))
                .min_by_key(|(event_id, ordinal)| (**ordinal, (*event_id).clone()))
                .map(|(event_id, _)| event_id.clone())
            else {
                break;
            };
            self.redacted_event_ids.remove(&event_id);
            self.redaction_ordinals.remove(&event_id);
        }
    }

    fn enforce_cleared_event_bound(&mut self) {
        while self.cleared_event_ids.len() > MAX_ACTIVITY_CLEARED_EVENTS {
            let Some(event_id) = self
                .cleared_event_ordinals
                .iter()
                .min_by_key(|(event_id, ordinal)| (**ordinal, (*event_id).clone()))
                .map(|(event_id, _)| event_id.clone())
            else {
                break;
            };
            self.cleared_event_ids.remove(&event_id);
            self.cleared_event_ordinals.remove(&event_id);
        }
    }

    fn effective_rows(&self) -> BTreeMap<String, ActivityRow> {
        let mut rows = self
            .resolution_rows_by_event_id
            .iter()
            .filter(|(event_id, row)| {
                !self.redacted_event_ids.contains(*event_id)
                    && !self
                        .hidden_event_ids_by_room
                        .get(&row.room_id)
                        .is_some_and(|ids| ids.contains(*event_id))
            })
            .map(|(event_id, row)| (event_id.clone(), row.clone()))
            .collect::<BTreeMap<_, _>>();
        for (room_id, room_rows) in &self.canonical_rows_by_room {
            let hidden = self
                .hidden_event_ids_by_room
                .get(room_id)
                .cloned()
                .unwrap_or_default();
            for (event_id, row) in room_rows {
                if self.redacted_event_ids.contains(event_id) || hidden.contains(event_id) {
                    continue;
                }
                // Canonical content owns duplicate stable identities.
                rows.insert(event_id.clone(), row.clone());
            }
        }
        rows
    }

    /// Activity Unread contains room summaries, not proven event targets.
    /// Reading is owned by the room/thread timeline after the user opens it.
    pub(super) fn mark_read(
        &mut self,
        _state: &AppState,
        _target: &ActivityMarkReadTarget,
    ) -> ActivityMarkReadResult {
        ActivityMarkReadResult::default()
    }

    pub(super) fn fully_read_marker_updates(
        &mut self,
        _state: &AppState,
        _target: &ActivityMarkReadTarget,
    ) -> Vec<(String, String)> {
        Vec::new()
    }

    pub(super) fn event_at_or_after(&self, room_id: &str, timestamp_ms: u64) -> Option<String> {
        let mut rows = self
            .effective_rows()
            .into_values()
            .filter(|row| row.room_id == room_id)
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| {
            left.timestamp_ms
                .cmp(&right.timestamp_ms)
                .then_with(|| left.event_id.cmp(&right.event_id))
        });

        rows.iter()
            .find(|row| row.timestamp_ms >= timestamp_ms)
            .or_else(|| rows.last())
            .filter(|row| row.kind == ActivityRowKind::Event)
            .and_then(|row| row.event_id.clone())
    }

    /// Current room-unread placeholders by room with their newest activity
    /// timestamp, or `None` when Activity is not open.
    pub(super) fn resolution_candidates(state: &AppState) -> Option<BTreeMap<String, u64>> {
        let ActivityState::Open { unread, .. } = &state.activity else {
            return None;
        };
        let mut candidates = BTreeMap::<String, u64>::new();
        for row in unread
            .rows
            .iter()
            .filter(|row| row.kind == ActivityRowKind::RoomUnread)
        {
            candidates
                .entry(row.room_id.clone())
                .and_modify(|timestamp_ms| *timestamp_ms = (*timestamp_ms).max(row.timestamp_ms))
                .or_insert(row.timestamp_ms);
        }
        Some(candidates)
    }

    pub(super) fn record_resolution_attempt(&mut self, candidates: BTreeMap<String, u64>) {
        self.resolution_attempts = candidates;
        self.requested_resolution_marker_by_room.clear();
    }

    pub(super) fn record_resolution_request_markers(
        &mut self,
        requests: &[ActivityResolutionRequest],
    ) {
        self.requested_resolution_marker_by_room = requests
            .iter()
            .filter_map(|request| {
                request
                    .fully_read_event_id
                    .as_ref()
                    .map(|marker| (request.room_id.clone(), marker.clone()))
            })
            .collect();
    }

    /// Whether live Activity has a room-unread candidate no resolution
    /// generation attempted yet. An in-flight generation is never preempted
    /// by live updates; its settlement re-evaluates the remaining candidates.
    pub(super) fn has_unattempted_resolution_candidate(&self, state: &AppState) -> bool {
        if matches!(
            &state.activity,
            ActivityState::Open { unread, .. }
                if matches!(unread.resolution, ActivityResolutionState::Resolving { .. })
        ) {
            return false;
        }
        Self::resolution_candidates(state).is_some_and(|candidates| {
            candidates.iter().any(|(room_id, timestamp_ms)| {
                self.resolution_attempts
                    .get(room_id)
                    .is_none_or(|attempted_ms| attempted_ms < timestamp_ms)
            })
        })
    }

    pub(super) fn update_action_for_open_state(&mut self, state: &AppState) -> Option<AppAction> {
        if !matches!(state.activity, ActivityState::Open { .. }) {
            return None;
        }
        let (recent, unread, excluded_room_ids) = self.snapshot_preserving_resolution(state);
        Some(AppAction::ActivityRowsUpdated {
            recent,
            unread,
            excluded_room_ids,
        })
    }

    /// A row refresh never owns the resolution lifecycle: keep the current
    /// generation so its settlement stays admissible.
    pub(super) fn snapshot_preserving_resolution(
        &mut self,
        state: &AppState,
    ) -> (ActivityStream, ActivityStream, Vec<String>) {
        let (mut recent, mut unread, excluded_room_ids) = self.snapshot(state);
        if let ActivityState::Open {
            recent: current_recent,
            unread: current_unread,
            ..
        } = &state.activity
        {
            recent.resolution = current_recent.resolution;
            unread.resolution = current_unread.resolution;
        }
        (recent, unread, excluded_room_ids)
    }

    pub(super) fn room_ids_without_remaining_unread(
        &mut self,
        state: &AppState,
        cleared_event_ids: &[String],
    ) -> Vec<String> {
        let affected_room_ids = cleared_event_ids
            .iter()
            .filter_map(|event_id| {
                self.effective_rows()
                    .get(event_id)
                    .map(|row| row.room_id.clone())
            })
            .collect::<BTreeSet<_>>();
        if affected_room_ids.is_empty() {
            return Vec::new();
        }

        let (_recent, unread, _excluded_room_ids) = self.snapshot(state);
        let remaining_unread_room_ids = unread
            .rows
            .into_iter()
            .map(|row| row.room_id)
            .collect::<BTreeSet<_>>();
        affected_room_ids
            .into_iter()
            .filter(|room_id| !remaining_unread_room_ids.contains(room_id))
            .collect()
    }

    pub(super) fn snapshot(
        &mut self,
        state: &AppState,
    ) -> (ActivityStream, ActivityStream, Vec<String>) {
        self.retain_projecting_avatars();
        let own_user_id = crate::event_projection::timeline_projection_own_user_id(state);
        let rooms_by_id: HashMap<&str, &RoomSummary> = state
            .rooms
            .iter()
            .map(|room| (room.room_id.as_str(), room))
            .collect();
        let excluded_room_ids = state
            .rooms
            .iter()
            .filter(|room| {
                room.tags.low_priority.is_some()
                    || state
                        .room_notification_settings
                        .get(&room.room_id)
                        .is_some_and(|settings| settings.mode == RoomNotificationMode::Mute)
            })
            .map(|room| room.room_id.clone())
            .collect::<Vec<_>>();
        let excluded: BTreeSet<&str> = excluded_room_ids.iter().map(String::as_str).collect();

        let mut recent = Vec::new();
        let mut recent_event_ids = BTreeSet::new();
        let effective_rows = self.effective_rows();
        for row in effective_rows.values() {
            if excluded.contains(row.room_id.as_str()) {
                continue;
            }
            let Some(room) = rooms_by_id.get(row.room_id.as_str()) else {
                continue;
            };
            let mode = state
                .room_notification_settings
                .get(&room.room_id)
                .map(|settings| settings.mode);
            let room_activity_unread = room_activity_unread_count_for_mode(room, mode) > 0;
            if !activity_recent_row_visible(mode, row.highlight, room_activity_unread) {
                continue;
            }
            let sender_avatar = self.resolved_sender_avatar(
                state,
                own_user_id,
                &row.room_id,
                row.sender_id.as_deref(),
                row.sender_avatar.as_ref(),
            );
            let context_label = activity_row_context_label(room, &state.spaces);
            let row = ActivityRow {
                room_label: room.display_label.clone(),
                sender_avatar,
                context_label,
                unread: false,
                highlight: row.highlight,
                ..row.clone()
            };
            if let Some(event_id) = row.event_id.clone() {
                recent_event_ids.insert(event_id);
            }
            recent.push(row);
        }

        for room in &state.rooms {
            if excluded.contains(room.room_id.as_str()) {
                continue;
            }
            let Some(display_event_id) = activity_latest_display_event_id(room) else {
                continue;
            };
            let Some(latest_event) = &room.latest_event else {
                continue;
            };
            if recent_event_ids.contains(display_event_id) {
                continue;
            }
            let mode = state
                .room_notification_settings
                .get(&room.room_id)
                .map(|settings| settings.mode);
            if !activity_recent_row_visible(
                mode,
                false,
                room_activity_unread_count_for_mode(room, mode) > 0,
            ) {
                continue;
            }
            let mut row = ActivityRow::event(
                room.room_id.clone(),
                display_event_id.to_owned(),
                latest_event.sender_id.clone(),
                room.display_label.clone(),
                latest_event.sender_label.clone(),
                latest_event.preview.clone(),
                latest_event.timestamp_ms,
                false,
                false,
            );
            row.sender_avatar = self.resolved_sender_avatar(
                state,
                own_user_id,
                &room.room_id,
                row.sender_id.as_deref(),
                latest_event.sender_avatar.as_ref(),
            );
            row.thread_root_event_id = latest_event.thread_root_event_id.clone();
            row.context_label = activity_row_context_label(room, &state.spaces);
            recent.push(row);
        }

        // The SDK count is verified at room scope. Fully-read markers, private
        // receipts and the server notification counter can disagree, so no
        // individual event row may claim to be unread here.
        // #1915: a room that is no longer unread has left the traced set, so its
        // next unread state is a new transition worth recording.
        for room in &state.rooms {
            if excluded.contains(room.room_id.as_str()) || room_activity_unread_count(room) == 0 {
                self.placeholder_traces.forget(&room.room_id);
            }
        }
        let mut unread: Vec<ActivityRow> = state
            .rooms
            .iter()
            .filter(|room| !excluded.contains(room.room_id.as_str()))
            .filter(|room| room_activity_unread_count(room) > 0)
            .map(|room| {
                crate::unread_trace::trace_activity_room(
                    &mut self.placeholder_traces,
                    "activity_placeholder",
                    room,
                    state
                        .room_notification_settings
                        .get(&room.room_id)
                        .map(|settings| settings.mode),
                    true,
                    "unread",
                );
                let timestamp_ms = room
                    .conversation_activity
                    .map(|activity| activity.timestamp_ms)
                    .or_else(|| activity_latest_display_event(room).map(|event| event.timestamp_ms))
                    .unwrap_or_default();
                ActivityRow {
                    context_label: activity_row_context_label(room, &state.spaces),
                    ..ActivityRow::room_unread_placeholder(
                        room.room_id.clone(),
                        room.display_label.clone(),
                        timestamp_ms,
                        false,
                    )
                }
            })
            .collect();
        sort_activity_rows(&mut recent);
        sort_activity_rows(&mut unread);

        let marker_event_ids = state
            .live_signals
            .rooms
            .values()
            .filter_map(|signals| signals.fully_read_event_id.as_deref())
            .filter(|event_id| effective_rows.contains_key(*event_id))
            .count();
        let canonical_rows = self.canonical_row_ordinals.len();
        let resolver_rows = self.resolution_rows_by_event_id.len();
        if canonical_rows > MAX_CANONICAL_ROWS_GLOBAL || resolver_rows > ACTIVITY_RECENT_MAX_ROWS {
            record(
                DiagnosticEvent::new(DiagnosticLevel::Debug, "core.activity", "projection_pruned")
                    .field(DiagnosticField::count(
                        "canonical_rows",
                        canonical_rows as u64,
                    ))
                    .field(DiagnosticField::count(
                        "resolver_rows",
                        resolver_rows as u64,
                    ))
                    .field(DiagnosticField::count(
                        "recent_returned",
                        recent.len().min(ACTIVITY_RECENT_MAX_ROWS) as u64,
                    ))
                    .field(DiagnosticField::count(
                        "unread_returned",
                        unread.len() as u64,
                    ))
                    .field(DiagnosticField::count(
                        "marker_retained",
                        marker_event_ids as u64,
                    )),
            );
        }
        recent.truncate(ACTIVITY_RECENT_MAX_ROWS);

        (
            ActivityStream {
                rows: recent,
                next_batch: None,
                resolution: Default::default(),
            },
            ActivityStream {
                rows: unread,
                next_batch: None,
                resolution: Default::default(),
            },
            excluded_room_ids,
        )
    }
}

fn activity_row_newest_first(left: &ActivityRow, right: &ActivityRow) -> std::cmp::Ordering {
    right
        .timestamp_ms
        .cmp(&left.timestamp_ms)
        .then_with(|| left.event_id.cmp(&right.event_id))
}

fn bounded_event_ids(ids: Vec<String>) -> BTreeSet<String> {
    let mut bounded = Vec::new();
    for id in ids {
        if id.trim().is_empty() || bounded.iter().any(|existing| existing == &id) {
            continue;
        }
        bounded.push(id);
    }
    if bounded.len() > MAX_CANONICAL_ROWS_PER_ROOM {
        bounded.drain(..bounded.len() - MAX_CANONICAL_ROWS_PER_ROOM);
    }
    bounded.into_iter().collect()
}

fn room_activity_unread_count_for_mode(
    room: &RoomSummary,
    mode: Option<RoomNotificationMode>,
) -> u64 {
    // Activity remains notification-filtered, but server-only notification
    // counts and manual flags cannot prove an unread message. Require the
    // client-observed main/thread count before applying the push-rule filter.
    let confirmed_unread_count = room_activity_unread_count(room);
    if confirmed_unread_count == 0 {
        return 0;
    }
    let highlight_total = room
        .highlight_count
        .saturating_add(room.thread_highlight_count);
    if matches!(mode, Some(RoomNotificationMode::Mentions)) && highlight_total == 0 {
        0
    } else {
        room.notification_count
            .max(highlight_total)
            .min(confirmed_unread_count)
    }
}

fn activity_recent_row_visible(
    mode: Option<RoomNotificationMode>,
    row_highlight: bool,
    room_activity_unread: bool,
) -> bool {
    !matches!(mode, Some(RoomNotificationMode::Mentions)) || row_highlight || room_activity_unread
}

fn activity_row_context_label(room: &RoomSummary, spaces: &[SpaceSummary]) -> String {
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

fn sort_activity_rows(rows: &mut [ActivityRow]) {
    rows.sort_by(|left, right| {
        right
            .timestamp_ms
            .cmp(&left.timestamp_ms)
            .then_with(|| left.room_id.cmp(&right.room_id))
            .then_with(|| left.event_id.cmp(&right.event_id))
    });
}

pub(super) fn guard_activity_resolution_completion(
    _state: &AppState,
    action: AppAction,
) -> AppAction {
    // Room summaries remain visible after hydration because no event-level
    // unread proof is claimed. Their presence is not a resolution failure.
    action
}

pub(super) fn normalize_activity_resolution_action(
    state: &AppState,
    action: AppAction,
) -> Option<AppAction> {
    let AppAction::ActivityResolutionRowsObserved { generation, rows } = action else {
        return Some(action);
    };
    let ActivityState::Open { unread, .. } = &state.activity else {
        return None;
    };
    if !matches!(
        unread.resolution,
        ActivityResolutionState::Resolving { generation: current, .. } if current == generation
    ) {
        return None;
    }
    Some(AppAction::ActivityResolutionRowsObserved { generation, rows })
}

pub(super) fn cap_activity_resolution_requests(
    mut requests: Vec<ActivityResolutionRequest>,
    generation: u64,
) -> Vec<ActivityResolutionRequest> {
    if requests.len() > MAX_ACTIVITY_RESOLUTION_ROOMS {
        let room_count = requests.len() as u64;
        let generation_offset = generation.saturating_sub(1) % room_count;
        let batch_width = MAX_ACTIVITY_RESOLUTION_ROOMS as u64 % room_count;
        let start = (generation_offset * batch_width % room_count) as usize;
        requests.rotate_left(start);
    }
    requests.truncate(MAX_ACTIVITY_RESOLUTION_ROOMS);
    requests
}

#[cfg(test)]
mod tests;
