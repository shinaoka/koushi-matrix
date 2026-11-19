use std::collections::{BTreeMap, BTreeSet, HashMap};

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_state::{
    ActivityMarkReadTarget, ActivityResolutionState, ActivityRow, ActivityRowKind, ActivityState,
    ActivityStream, ActivityTab, AppAction, AppState, AvatarImage, AvatarThumbnailState,
    RoomNotificationMode,
};

use crate::room_profile::{RoomProfile, RoomProfileTable, RoomUnreadCounts};
use crate::unread_trace::UnreadTraceLedger;

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
    /// #1915: the per-room derived inputs of both streams. Only rooms whose
    /// profile revision changed have their rows rebuilt.
    room_profiles: RoomProfileTable,
    /// Unread-stream summary rows by room, rebuilt per changed profile.
    unread_rows_by_room: HashMap<String, ActivityRow>,
    /// Advances on every change to the cached rows or their avatar thumbnails.
    rows_revision: u64,
    /// The streams last built and the global inputs they were built from.
    emitted: Option<ActivityEmission>,
    /// Last recorded `activity_placeholder` unread trace per room.
    placeholder_traces: UnreadTraceLedger,
    stream_rebuilds: u64,
    last_rebuilt_room_rows: Vec<String>,
}

struct ActivityEmission {
    rows_revision: u64,
    own_user_id: Option<String>,
    recent: Vec<ActivityRow>,
    unread: Vec<ActivityRow>,
}

#[derive(Default)]
pub(super) struct ActivityMarkReadResult {
    pub(super) cleared_event_ids: Vec<String>,
    pub(super) cleared_placeholder_room_ids: Vec<String>,
}

impl ActivityProjection {
    /// #1915: the trace dedup state is per session, but this projection
    /// outlives a logout/login inside one runtime. Clearing it keeps the new
    /// session's first `activity_placeholder` records from being suppressed by
    /// the previous session's values.
    pub(super) fn forget_trace_history(&mut self) {
        self.placeholder_traces = Default::default();
    }

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
        if self.avatar_thumbnails_by_mxc.get(mxc_uri) == Some(thumbnail) {
            return;
        }
        self.avatar_thumbnails_by_mxc
            .insert(mxc_uri.to_owned(), thumbnail.clone());
        self.rows_revision = self.rows_revision.wrapping_add(1);
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
        self.settled_avatar(
            chosen_sender_avatar(state, own_user_id, room_id, sender_id, embedded).cloned(),
        )
    }

    /// Whether `current` is what [`Self::resolved_sender_avatar`] would
    /// return now, checked without cloning.
    fn sender_avatar_is_current(
        &self,
        state: &AppState,
        own_user_id: Option<&str>,
        row: &ActivityRow,
        embedded: Option<&AvatarImage>,
    ) -> bool {
        let chosen = chosen_sender_avatar(
            state,
            own_user_id,
            &row.room_id,
            row.sender_id.as_deref(),
            embedded,
        );
        match (chosen, row.sender_avatar.as_ref()) {
            (None, None) => true,
            (Some(chosen), Some(current)) => {
                chosen.mxc_uri == current.mxc_uri
                    && self
                        .avatar_thumbnails_by_mxc
                        .get(&chosen.mxc_uri)
                        .unwrap_or(&chosen.thumbnail)
                        == &current.thumbnail
            }
            _ => false,
        }
    }
}

/// The avatar source a row shows before thumbnail settlement: the live
/// profile when known, else the copy embedded in the row or summary.
fn chosen_sender_avatar<'a>(
    state: &'a AppState,
    own_user_id: Option<&str>,
    room_id: &str,
    sender_id: Option<&str>,
    embedded: Option<&'a AvatarImage>,
) -> Option<&'a AvatarImage> {
    let Some(sender_id) = sender_id else {
        return embedded;
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
    match (profile_avatar, embedded) {
        (Some(profile), Some(embedded))
            if profile.mxc_uri == embedded.mxc_uri
                && profile.thumbnail == AvatarThumbnailState::NotRequested
                && embedded.thumbnail != AvatarThumbnailState::NotRequested =>
        {
            Some(embedded)
        }
        (Some(profile), _) => Some(profile),
        (None, Some(embedded)) => Some(embedded),
        (None, None) => None,
    }
}

impl ActivityProjection {
    /// Compatibility path for older tests/injectors. Production timelines use
    /// `CanonicalActivityWindowReconciled`; this path replaces each observed
    /// room too, spilling only its over-bound detail into resolver provenance.
    pub(super) fn ingest(&mut self, rows: Vec<ActivityRow>) {
        self.rows_revision = self.rows_revision.wrapping_add(1);
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
        self.rows_revision = self.rows_revision.wrapping_add(1);
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
        self.rows_revision = self.rows_revision.wrapping_add(1);

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

    /// The rows update open Activity needs after an action, or `None` when
    /// no Activity input changed (#1915). Unchanged rooms cost one profile
    /// comparison; the streams are rebuilt only when a room profile, the
    /// cached rows, the session user, or a shown avatar changed, and the
    /// action is emitted only when the rebuilt rows differ from the state.
    pub(super) fn update_action_for_open_state(&mut self, state: &AppState) -> Option<AppAction> {
        let ActivityState::Open {
            recent: current_recent,
            unread: current_unread,
            ..
        } = &state.activity
        else {
            return None;
        };
        let rooms_changed = self.refresh_room_rows(state);
        let state_holds_emission = self.emitted.as_ref().is_some_and(|emitted| {
            emitted.recent == current_recent.rows && emitted.unread == current_unread.rows
        });
        if !rooms_changed && state_holds_emission && self.emitted_inputs_current(state) {
            return None;
        }
        let (recent, unread, excluded_room_ids) = self.build_streams(state);
        if recent.rows == current_recent.rows && unread.rows == current_unread.rows {
            return None;
        }
        let (recent, unread) = preserve_resolution(state, recent, unread);
        Some(AppAction::ActivityRowsUpdated {
            recent,
            unread,
            excluded_room_ids,
        })
    }

    /// Whether the global inputs of the last emission still hold: the cached
    /// rows, the session user, and every shown sender avatar.
    fn emitted_inputs_current(&self, state: &AppState) -> bool {
        let Some(emitted) = &self.emitted else {
            return false;
        };
        let own_user_id = crate::event_projection::timeline_projection_own_user_id(state);
        emitted.rows_revision == self.rows_revision
            && emitted.own_user_id.as_deref() == own_user_id
            && emitted.recent.iter().all(|row| {
                self.sender_avatar_is_current(state, own_user_id, row, self.embedded_avatar(row))
            })
    }

    /// The avatar copy a recent row was built from: its cached row, else the
    /// room's latest displayable event.
    fn embedded_avatar(&self, row: &ActivityRow) -> Option<&AvatarImage> {
        let event_id = row.event_id.as_deref()?;
        if let Some(cached) = self
            .canonical_rows_by_room
            .get(&row.room_id)
            .and_then(|rows| rows.get(event_id))
            .or_else(|| self.resolution_rows_by_event_id.get(event_id))
        {
            return cached.sender_avatar.as_ref();
        }
        self.room_profiles
            .get(&row.room_id)
            .and_then(|profile| profile.latest_display_event.as_ref())
            .filter(|latest| latest.event_id == event_id)
            .and_then(|latest| latest.sender_avatar.as_ref())
    }

    /// Refresh the room profile table and rebuild the unread summary row of
    /// every room whose profile changed. Returns whether any room changed.
    fn refresh_room_rows(&mut self, state: &AppState) -> bool {
        let changes = self.room_profiles.refresh(state);
        self.last_rebuilt_room_rows.clear();
        for room_id in &changes.removed {
            self.unread_rows_by_room.remove(room_id);
            self.placeholder_traces.forget(room_id);
        }
        for &index in &changes.updated {
            let room = &state.rooms[index];
            let Some(profile) = self.room_profiles.get(&room.room_id) else {
                continue;
            };
            self.last_rebuilt_room_rows.push(room.room_id.clone());
            // The SDK count is verified at room scope. Fully-read markers,
            // private receipts and the server notification counter can
            // disagree, so no individual event row may claim to be unread.
            if activity_excluded(profile) || profile.confirmed_unread_count() == 0 {
                self.unread_rows_by_room.remove(&room.room_id);
                self.placeholder_traces.forget(&room.room_id);
                continue;
            }
            crate::unread_trace::trace_activity_room(
                &mut self.placeholder_traces,
                "activity_placeholder",
                room,
                profile.notification_mode,
                true,
                "unread",
            );
            let row = ActivityRow {
                context_label: profile.context_label.clone(),
                ..ActivityRow::room_unread_placeholder(
                    room.room_id.clone(),
                    profile.display_label.clone(),
                    profile.activity_timestamp_ms,
                    false,
                )
            };
            self.unread_rows_by_room.insert(room.room_id.clone(), row);
        }
        !changes.is_empty()
    }

    /// A row refresh never owns the resolution lifecycle: keep the current
    /// generation so its settlement stays admissible.
    pub(super) fn snapshot_preserving_resolution(
        &mut self,
        state: &AppState,
    ) -> (ActivityStream, ActivityStream, Vec<String>) {
        let (recent, unread, excluded_room_ids) = self.snapshot(state);
        let (recent, unread) = preserve_resolution(state, recent, unread);
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

    #[cfg(test)]
    pub(super) fn stream_rebuilds(&self) -> u64 {
        self.stream_rebuilds
    }

    #[cfg(test)]
    pub(super) fn last_rebuilt_room_rows(&self) -> &[String] {
        &self.last_rebuilt_room_rows
    }

    #[cfg(test)]
    pub(super) fn room_profile_revision(&self, room_id: &str) -> Option<u64> {
        self.room_profiles.get(room_id).map(RoomProfile::revision)
    }

    pub(super) fn snapshot(
        &mut self,
        state: &AppState,
    ) -> (ActivityStream, ActivityStream, Vec<String>) {
        self.refresh_room_rows(state);
        self.build_streams(state)
    }

    /// Build both streams from the refreshed room profiles and cached rows,
    /// and remember them as the current emission.
    fn build_streams(&mut self, state: &AppState) -> (ActivityStream, ActivityStream, Vec<String>) {
        self.stream_rebuilds += 1;
        self.retain_projecting_avatars();
        let own_user_id = crate::event_projection::timeline_projection_own_user_id(state);
        let excluded_room_ids = state
            .rooms
            .iter()
            .filter(|room| {
                self.room_profiles
                    .get(&room.room_id)
                    .is_some_and(activity_excluded)
            })
            .map(|room| room.room_id.clone())
            .collect::<Vec<_>>();

        let mut recent = Vec::new();
        let mut recent_event_ids = BTreeSet::new();
        for row in self.effective_row_refs() {
            let Some(profile) = self.room_profiles.get(&row.room_id) else {
                continue;
            };
            if activity_excluded(profile)
                || !activity_recent_row_visible(
                    profile.notification_mode,
                    row.highlight,
                    activity_mode_unread_count(profile) > 0,
                )
            {
                continue;
            }
            let sender_avatar = self.resolved_sender_avatar(
                state,
                own_user_id,
                &row.room_id,
                row.sender_id.as_deref(),
                row.sender_avatar.as_ref(),
            );
            let row = ActivityRow {
                room_label: profile.display_label.clone(),
                sender_avatar,
                context_label: profile.context_label.clone(),
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
            let Some(profile) = self.room_profiles.get(&room.room_id) else {
                continue;
            };
            if activity_excluded(profile) {
                continue;
            }
            let Some(latest_event) = profile
                .latest_display_event
                .as_ref()
                .filter(|latest| !latest.event_id.trim().is_empty())
            else {
                continue;
            };
            if recent_event_ids.contains(&latest_event.event_id)
                || !activity_recent_row_visible(
                    profile.notification_mode,
                    false,
                    activity_mode_unread_count(profile) > 0,
                )
            {
                continue;
            }
            let mut row = ActivityRow::event(
                room.room_id.clone(),
                latest_event.event_id.clone(),
                latest_event.sender_id.clone(),
                profile.display_label.clone(),
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
            row.context_label = profile.context_label.clone();
            recent.push(row);
        }

        let mut unread: Vec<ActivityRow> = state
            .rooms
            .iter()
            .filter_map(|room| self.unread_rows_by_room.get(&room.room_id).cloned())
            .collect();
        koushi_state::sort_activity_rows(&mut recent);
        koushi_state::sort_activity_rows(&mut unread);

        let canonical_rows = self.canonical_row_ordinals.len();
        let resolver_rows = self.resolution_rows_by_event_id.len();
        if canonical_rows > MAX_CANONICAL_ROWS_GLOBAL || resolver_rows > ACTIVITY_RECENT_MAX_ROWS {
            let effective_rows = self.effective_rows();
            let marker_event_ids = state
                .live_signals
                .rooms
                .values()
                .filter_map(|signals| signals.fully_read_event_id.as_deref())
                .filter(|event_id| effective_rows.contains_key(*event_id))
                .count();
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

        self.emitted = Some(ActivityEmission {
            rows_revision: self.rows_revision,
            own_user_id: own_user_id.map(str::to_owned),
            recent: recent.clone(),
            unread: unread.clone(),
        });
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

    /// The effective cached rows by reference: canonical rows own a stable
    /// identity they share with a resolver row; redacted and hidden events
    /// are skipped.
    fn effective_row_refs(&self) -> impl Iterator<Item = &ActivityRow> {
        let visible = |room_id: &str, event_id: &str| {
            !self.redacted_event_ids.contains(event_id)
                && !self
                    .hidden_event_ids_by_room
                    .get(room_id)
                    .is_some_and(|ids| ids.contains(event_id))
        };
        let canonical = self
            .canonical_rows_by_room
            .iter()
            .flat_map(move |(room_id, rows)| {
                rows.iter()
                    .filter(move |(event_id, _)| visible(room_id, event_id))
                    .map(|(_, row)| row)
            });
        let resolution = self
            .resolution_rows_by_event_id
            .iter()
            .filter(move |(event_id, row)| {
                visible(&row.room_id, event_id)
                    && !self
                        .canonical_rows_by_room
                        .get(&row.room_id)
                        .is_some_and(|rows| rows.contains_key(*event_id))
            })
            .map(|(_, row)| row);
        canonical.chain(resolution)
    }
}

fn preserve_resolution(
    state: &AppState,
    mut recent: ActivityStream,
    mut unread: ActivityStream,
) -> (ActivityStream, ActivityStream) {
    if let ActivityState::Open {
        recent: current_recent,
        unread: current_unread,
        ..
    } = &state.activity
    {
        recent.resolution = current_recent.resolution;
        unread.resolution = current_unread.resolution;
    }
    (recent, unread)
}

fn activity_excluded(profile: &RoomProfile) -> bool {
    profile.low_priority || profile.is_muted()
}

fn activity_mode_unread_count(profile: &RoomProfile) -> u64 {
    mode_unread_count(&profile.counts, profile.notification_mode)
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

#[cfg(test)]
fn room_activity_unread_count_for_mode(
    room: &koushi_state::RoomSummary,
    mode: Option<RoomNotificationMode>,
) -> u64 {
    mode_unread_count(&crate::room_profile::RoomUnreadCounts::of(room), mode)
}

fn mode_unread_count(counts: &RoomUnreadCounts, mode: Option<RoomNotificationMode>) -> u64 {
    // Activity remains notification-filtered, but server-only notification
    // counts and manual flags cannot prove an unread message. Require the
    // client-observed main/thread count before applying the push-rule filter.
    let confirmed_unread_count = counts
        .unread_count
        .saturating_add(counts.thread_unread_count);
    if confirmed_unread_count == 0 {
        return 0;
    }
    let highlight_total = counts
        .highlight_count
        .saturating_add(counts.thread_highlight_count);
    if matches!(mode, Some(RoomNotificationMode::Mentions)) && highlight_total == 0 {
        0
    } else {
        counts
            .notification_count
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
mod incremental_tests;
#[cfg(test)]
mod tests;
