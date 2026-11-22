use crate::{
    effect::{AppEffect, UiEvent},
    state::AppState,
};

use super::{is_session_ready, session_user_id};

pub(crate) fn handle_live_room_profiles_observed(
    state: &mut AppState,
    room_id: String,
    profiles: Vec<crate::state::UserProfile>,
) -> Vec<AppEffect> {
    // The reducer boundary is ready for room-local observations. Wiring the
    // TimelineActor producer is intentionally deferred to the next I6
    // milestone so this change remains local-only and bounded.
    if !is_session_ready(state) {
        return Vec::new();
    }

    let known_thumbnails = super::avatar::collect_known_avatar_thumbnails(state, false);
    let room_profiles = state.profile.room_users.entry(room_id).or_default();
    let mut changed = false;
    for mut profile in profiles {
        super::avatar::preserve_avatar_thumbnail(&known_thumbnails, &mut profile.avatar);
        if let Some(existing) = room_profiles.get_mut(&profile.user_id) {
            if profile.display_name.is_none() {
                profile.display_name = existing.display_name.clone();
            }
            if profile.avatar.is_none() {
                profile.avatar = existing.avatar.clone();
            }
            if existing != &profile {
                *existing = profile;
                changed = true;
            }
        } else {
            room_profiles.insert(profile.user_id.clone(), profile);
            changed = true;
        }
    }

    if changed {
        let own_user_id = session_user_id(state).map(str::to_owned);
        crate::state::refresh_live_receipt_display_projection(
            &mut state.live_signals,
            &state.profile,
            own_user_id.as_deref(),
        );
        vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
    } else {
        Vec::new()
    }
}

pub(crate) fn handle_live_room_receipt_summaries_updated(
    state: &mut AppState,
    room_id: String,
    scope: crate::state::ReceiptScope,
    scoped_event_ids: Vec<String>,
    receipts_by_event: Vec<crate::state::LiveEventReceiptSummaryUpdate>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let own_user_id = session_user_id(state).map(str::to_owned);
    let relevant_room_profiles = state.profile.room_users.get(&room_id);
    let previous_scope = state
        .live_signals
        .rooms
        .get(&room_id)
        .and_then(|room| room.receipts_for_scope(&scope));
    let mut summaries = Vec::with_capacity(receipts_by_event.len());
    for mut entry in receipts_by_event {
        let event_id = entry.event_id.clone();
        let previous_readers = previous_scope
            .and_then(|receipts| receipts.get(&event_id))
            .map(|summary| &summary.readers);
        for receipt in &mut entry.readers {
            if receipt.avatar.is_none() {
                receipt.avatar = previous_readers
                    .and_then(|readers| {
                        readers
                            .iter()
                            .find(|reader| reader.user_id == receipt.user_id)
                    })
                    .and_then(|reader| reader.avatar.clone());
            }
        }
        let summary = entry.into_summary_with_profiles(
            &state.profile,
            relevant_room_profiles,
            own_user_id.as_deref(),
        );
        summaries.push((event_id, summary));
    }

    let room = state.live_signals.rooms.entry(room_id).or_default();
    {
        let scoped = room.receipts_for_scope_mut(&scope);
        for event_id in scoped_event_ids {
            scoped.remove(&event_id);
        }
        for (event_id, summary) in summaries {
            scoped.insert(event_id, summary);
        }
    }
    vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
}

pub(crate) fn handle_live_room_receipts_window_reconciled(
    state: &mut AppState,
    room_id: String,
    scope: crate::state::ReceiptScope,
    scoped_event_ids: Vec<String>,
    receipts_by_event: Vec<crate::state::LiveEventReceipts>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    let own_user_id = session_user_id(state).map(str::to_owned);
    let relevant_room_profiles = state.profile.room_users.get(&room_id);
    let mut receipts_by_event = receipts_by_event;
    preserve_known_receipt_thumbnails(state, &mut receipts_by_event);
    let normalized = crate::state::LiveRoomSignalUpdate {
        receipts_by_event,
        fully_read_event_id: None,
        typing_user_ids: Vec::new(),
    }
    .into_room_signals_with_room_profiles(
        &state.profile,
        relevant_room_profiles,
        own_user_id.as_deref(),
    );
    let room = state.live_signals.rooms.entry(room_id).or_default();
    {
        let scoped = room.receipts_for_scope_mut(&scope);
        for event_id in scoped_event_ids {
            scoped.remove(&event_id);
        }
        scoped.extend(normalized.receipts_by_event);
    }
    vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
}

fn preserve_known_receipt_thumbnails(
    state: &AppState,
    receipts_by_event: &mut [crate::state::LiveEventReceipts],
) {
    let known_thumbnails = super::avatar::collect_known_avatar_thumbnails(state, false);
    for event_receipts in receipts_by_event {
        for receipt in &mut event_receipts.receipts {
            super::avatar::preserve_avatar_thumbnail(&known_thumbnails, &mut receipt.avatar);
        }
    }
}

pub(crate) fn handle_fully_read_marker_updated(
    state: &mut AppState,
    room_id: String,
    event_id: Option<String>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    state
        .live_signals
        .rooms
        .entry(room_id)
        .or_default()
        .fully_read_event_id = event_id;
    vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
}

/// #2030 B2: publish a confirmed main-room read boundary to the room list.
///
/// Core only sends this once it has proven the boundary is the newest event the
/// room's main timeline holds, so no unread event can follow it in this scope.
/// The boundary becomes the room's read position regardless of which room is in
/// the foreground, and the room-list reducer settles the counts it covers; a
/// list that has already moved past the boundary keeps them.
pub(crate) fn handle_confirmed_read_boundary_settled(
    state: &mut AppState,
    room_id: String,
    event_id: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    // The marker path owns this field; a settlement only fills it in when nothing
    // has published one yet, so a stale settlement cannot move the read divider
    // back. The stale-snapshot suppression only needs it to be present.
    let signals = state.live_signals.rooms.entry(room_id.clone()).or_default();
    let mut effects = Vec::new();
    if signals.fully_read_event_id.is_none() {
        signals.fully_read_event_id = Some(event_id.clone());
        effects.push(AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged));
    }
    // A room the projection does not hold yet has no counts to settle; the next
    // room-list update brings it back.
    if !super::room_exists(state, &room_id)
        || !super::room::settle_room_list_read_boundary(state, &room_id, &event_id)
    {
        return effects;
    }
    // #1803: the counts moved, so the published candidate and the badge are
    // recomputed in this same reduce instead of waiting for a room-list update.
    let (native_attention_changed, native_attention_diagnostic) =
        super::native_attention::recompute_native_attention_from_rooms(
            state,
            crate::state::NativeAttentionObservationKind::InitialSync,
        );
    effects.push(AppEffect::EmitUiEvent(UiEvent::RoomListChanged));
    effects.push(native_attention_diagnostic);
    if native_attention_changed {
        effects.push(AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged));
    }
    effects
}

pub(crate) fn handle_typing_users_updated(
    state: &mut AppState,
    room_id: String,
    user_ids: Vec<String>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let own_user_id = session_user_id(state).map(str::to_owned);
    let normalized = crate::state::LiveRoomSignalUpdate {
        receipts_by_event: Vec::new(),
        fully_read_event_id: None,
        typing_user_ids: user_ids,
    }
    .into_room_signals_with_profiles(&state.profile, own_user_id.as_deref());
    let room = state.live_signals.rooms.entry(room_id).or_default();
    room.typing_user_ids = normalized.typing_user_ids;
    room.typing_users = normalized.typing_users;
    vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
}

pub(crate) fn handle_presence_updated(
    state: &mut AppState,
    user_id: String,
    presence: crate::state::PresenceKind,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    if state.profile.ignored_user_ids.contains(&user_id) {
        return Vec::new();
    }

    state.live_signals.presence.insert(user_id, presence);
    vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
}
