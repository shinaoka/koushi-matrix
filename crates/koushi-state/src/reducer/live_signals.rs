use crate::{
    effect::{AppEffect, UiEvent},
    state::AppState,
};

use super::{is_session_ready, session_user_id};

pub(crate) fn handle_live_room_receipts_updated(
    state: &mut AppState,
    room_id: String,
    receipts_by_event: Vec<crate::state::LiveEventReceipts>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let own_user_id = session_user_id(state).map(str::to_owned);
    let room = state.live_signals.rooms.entry(room_id).or_default();
    let normalized = crate::state::LiveRoomSignalUpdate {
        receipts_by_event,
        fully_read_event_id: None,
        typing_user_ids: Vec::new(),
    }
    .into_room_signals_with_profiles(&state.profile, own_user_id.as_deref());
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
    let normalized = crate::state::LiveRoomSignalUpdate {
        receipts_by_event,
        fully_read_event_id: None,
        typing_user_ids: Vec::new(),
    }
    .into_room_signals_with_profiles(&state.profile, own_user_id.as_deref());
    let room = state.live_signals.rooms.entry(room_id).or_default();
    for event_id in scoped_event_ids {
        room.receipts_by_event.remove(&event_id);
    }
    room.receipts_by_event.extend(normalized.receipts_by_event);
    vec![AppEffect::EmitUiEvent(UiEvent::LiveSignalsChanged)]
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

    let normalized = crate::state::LiveRoomSignalUpdate {
        receipts_by_event: Vec::new(),
        fully_read_event_id: None,
        typing_user_ids: user_ids,
    }
    .into_room_signals();
    state
        .live_signals
        .rooms
        .entry(room_id)
        .or_default()
        .typing_user_ids = normalized.typing_user_ids;
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
