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

pub(crate) fn handle_live_room_receipts_updated(
    state: &mut AppState,
    room_id: String,
    receipts_by_event: Vec<crate::state::LiveEventReceipts>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let own_user_id = session_user_id(state).map(str::to_owned);
    let relevant_room_profiles = state.profile.room_users.get(&room_id);
    let mut receipts_by_event = receipts_by_event;
    preserve_known_receipt_thumbnails(state, &mut receipts_by_event);
    let room = state.live_signals.rooms.entry(room_id).or_default();
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
    for (event_id, receipts) in normalized.receipts_by_event {
        room.receipts_by_event.insert(event_id, receipts);
    }
    vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
}

pub(crate) fn handle_live_room_receipts_window_reconciled(
    state: &mut AppState,
    room_id: String,
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
    for event_id in scoped_event_ids {
        room.receipts_by_event.remove(&event_id);
    }
    room.receipts_by_event.extend(normalized.receipts_by_event);
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
