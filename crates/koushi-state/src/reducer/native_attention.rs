use crate::{
    effect::{AppEffect, UiEvent},
    state::{AppState, NativeAttentionObservationKind},
};

use super::is_session_ready;

pub(crate) fn handle_dispatch_started(
    state: &mut AppState,
    dispatch_id: crate::state::NativeAttentionDispatchId,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    // Transient sound requires the same Rust-owned eligible candidate as banners.
    // Raw unread/badge growth alone must never bypass room notification policy.
    if state.native_attention.summary.candidate.is_none()
        || !state.settings.values.notifications.sound
        || !matches!(
            state.native_attention.dispatch,
            crate::state::NativeAttentionDispatchState::Idle
        )
    {
        return Vec::new();
    }
    state.native_attention.dispatch =
        crate::state::NativeAttentionDispatchState::Dispatching { dispatch_id };
    vec![AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged)]
}

pub(crate) fn handle_dispatch_settled(
    state: &mut AppState,
    dispatch_id: crate::state::NativeAttentionDispatchId,
    outcome: crate::state::NativeAttentionSoundOutcome,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    if state.native_attention.dispatch
        != (crate::state::NativeAttentionDispatchState::Dispatching { dispatch_id })
    {
        return Vec::new();
    }
    state.native_attention.dispatch = match outcome {
        crate::state::NativeAttentionSoundOutcome::Played => {
            crate::state::NativeAttentionDispatchState::Delivered { dispatch_id }
        }
        crate::state::NativeAttentionSoundOutcome::Unsupported => {
            crate::state::NativeAttentionDispatchState::Unsupported { dispatch_id }
        }
        crate::state::NativeAttentionSoundOutcome::Failed => {
            crate::state::NativeAttentionDispatchState::Failed {
                dispatch_id,
                kind: crate::state::OperationFailureKind::Sdk,
            }
        }
        crate::state::NativeAttentionSoundOutcome::Skipped => return Vec::new(),
    };
    vec![AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged)]
}

pub(crate) fn handle_native_attention_updated(
    state: &mut AppState,
    mut attention: crate::state::NativeAttentionState,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    // #1806: the stateless projection stamps `alert_id: None`, so a caller that submits
    // its output over a published candidate would drop the identity the webview dedupes
    // on. Carry the published identity forward when the submission is the alert that is
    // already out: the room the published alert targets (#1816), compared through the
    // handle that survives a restore (#1818). Reading a missing identity as "new alert"
    // is the double banner #1802 removed, and minting here would add a second mint rule
    // beside the room-identity one in
    // `recompute_native_attention_from_rooms_for_live_room`, so a submission for another
    // room carries none rather than borrowing a published one. An identity the caller
    // does submit wins, and a submitted clear keeps clearing: an identity without a
    // candidate would be a banner the webview can never show.
    if attention.summary.candidate.is_some() {
        let submitted_alert_room = attention
            .notification
            .as_ref()
            .map(|payload| crate::state::native_attention_room_handle(&payload.target.room_id));
        if attention.summary.alert_id.is_none()
            && submitted_alert_room.is_some()
            && submitted_alert_room == state.native_attention.summary.alert_room
        {
            attention.summary.alert_id = state.native_attention.summary.alert_id;
        }
        // The handle follows the identity: an alert that has one names a room, either
        // the one the caller supplied with it or the one this submission's payload
        // targets, and an alert that has none names no room.
        attention.summary.alert_room = match attention.summary.alert_id {
            Some(_) => submitted_alert_room.or(attention.summary.alert_room),
            None => None,
        };
    } else {
        // A submitted clear clears the room with the identity.
        attention.summary.alert_room = None;
    }
    if !state.settings.values.notifications.badges {
        attention.summary.badge_count = 0;
    }
    if state.is_agent_account() {
        silence_owner_alerts(&mut attention);
    }
    attention.dispatch = state.native_attention.dispatch.clone();
    if state.native_attention == attention {
        return Vec::new();
    }

    state.native_attention = attention;
    vec![AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged)]
}

fn projected_native_attention_from_rooms(
    state: &AppState,
    observation: NativeAttentionObservationKind,
    live_room_id: Option<&str>,
    preview_room_id: Option<&str>,
) -> crate::state::NativeAttentionProjection {
    let room_notification_modes = crate::state::attention_room_notification_modes(state);
    let previous_candidate = state.native_attention.summary.candidate.as_ref();
    let mut projection = crate::state::native_attention_projection_from_rooms_with_live_room(
        crate::state::NativeAttentionProjectionInput {
            rooms: &state.rooms,
            active_room_id: state.navigation.active_room_id.as_deref(),
            muted_room_ids: &[],
            room_notification_modes: &room_notification_modes,
            ignored_user_ids: &state.profile.ignored_user_ids,
            window_focused: state.native_attention_context.window_focused,
            observation,
            previous_candidate,
            message_previews: state.settings.values.notifications.message_previews,
            capabilities: state.native_attention.summary.capabilities,
            locale: crate::resolve_catalog_locale(&state.settings.values.locale),
        },
        live_room_id,
        preview_room_id,
        true,
    );
    if !state.settings.values.notifications.badges {
        projection.state.summary.badge_count = 0;
    }
    if state.is_agent_account() {
        silence_owner_alerts(&mut projection.state);
    }
    projection
}

/// #2110: an agent account interrupts nobody. The unread counts stay (the agent
/// and the owner can still read them); the badge, the notification candidate and
/// the alert identity that would raise a banner or a sound do not exist.
fn silence_owner_alerts(attention: &mut crate::state::NativeAttentionState) {
    attention.summary.badge_count = 0;
    attention.summary.candidate = None;
    attention.summary.alert_id = None;
    attention.summary.alert_room = None;
    attention.notification = None;
}

pub(crate) fn recompute_native_attention_from_rooms(
    state: &mut AppState,
    observation: NativeAttentionObservationKind,
) -> (bool, AppEffect) {
    recompute_native_attention_from_rooms_for_live_room(state, observation, None, None)
}

/// The published candidate and the room its payload targets.
fn published_candidate(
    state: &AppState,
) -> Option<(&str, &crate::state::NativeAttentionCandidate)> {
    let candidate = state.native_attention.summary.candidate.as_ref()?;
    let room_id = state
        .native_attention
        .notification
        .as_ref()
        .map(|payload| payload.target.room_id.as_str())?;
    Some((room_id, candidate))
}

/// Whether `room_id` still holds the attention that justified the published candidate.
///
/// Answered by the production projection over that one room, so this lifetime rule cannot
/// drift from the eligibility rules that raise a candidate.
fn published_room_still_holds_attention(state: &AppState, room_id: &str) -> bool {
    if state.is_agent_account() {
        return false;
    }
    let Some(room) = state.rooms.iter().find(|room| room.room_id == room_id) else {
        return false;
    };
    let room_notification_modes = crate::state::attention_room_notification_modes(state);
    crate::state::native_attention_projection_from_rooms_with_live_room(
        crate::state::NativeAttentionProjectionInput {
            rooms: std::slice::from_ref(room),
            // Focus does not take part: a focus change clears through its own action.
            active_room_id: None,
            muted_room_ids: &[],
            room_notification_modes: &room_notification_modes,
            ignored_user_ids: &state.profile.ignored_user_ids,
            window_focused: false,
            observation: NativeAttentionObservationKind::Live,
            previous_candidate: None,
            message_previews: false,
            capabilities: state.native_attention.summary.capabilities,
            locale: crate::resolve_catalog_locale(&state.settings.values.locale),
        },
        Some(room_id),
        None,
        true,
    )
    .state
    .summary
    .candidate
    .is_some()
}

pub(crate) fn recompute_native_attention_from_rooms_for_live_room(
    state: &mut AppState,
    observation: NativeAttentionObservationKind,
    live_room_id: Option<&str>,
    preview_room_id: Option<&str>,
) -> (bool, AppEffect) {
    let mut projection =
        projected_native_attention_from_rooms(state, observation, live_room_id, preview_room_id);
    // #1799: the adapter reads the candidate asynchronously, so an observation that raises
    // nothing (no live room, or a live room with no eligible attention) must not consume the
    // published candidate. It keeps the payload and the dispatch it was published with, so
    // the webview sees no change and cannot show the same banner or sound twice. A newer
    // live candidate replaces it; read, mute, and focus changes still clear it.
    if projection.state.summary.candidate.is_none()
        && projection.state.dispatch == crate::state::NativeAttentionDispatchState::Idle
        && let Some((room_id, candidate)) = published_candidate(state)
        && published_room_still_holds_attention(state, room_id)
    {
        projection.state.summary.candidate = Some(candidate.clone());
        projection.state.notification = state.native_attention.notification.clone();
        projection.state.dispatch = state.native_attention.dispatch.clone();
    }
    // #1802/#1816: the alert identity is minted here, where the candidate is raised, and
    // only here. It belongs to the room that holds the attention, not to the observation
    // that recomputed it: a candidate for the room the published alert already targets
    // relabels that alert even when this observation itself raises it (an unread increase
    // reports the room as live, `room.rs`), so the webview keeps seeing one alert for one
    // unread conversation until a canonical clear ends it. A candidate for another room
    // is a new alert.
    //
    // #1818: the published room is compared through the serialized handle rather than
    // through the payload's room id, so a summary that came back from a restore relabels
    // the alert it published instead of minting a second one for the same conversation.
    let published_alert_room = state.native_attention.summary.alert_room;
    let raised_alert_room = projection
        .state
        .notification
        .as_ref()
        .map(|payload| crate::state::native_attention_room_handle(&payload.target.room_id));
    let previous_alert_id = state.native_attention.summary.alert_id;
    let relabelled_alert_id = published_alert_room
        .filter(|handle| Some(*handle) == raised_alert_room)
        .and(previous_alert_id);
    projection.state.summary.alert_room = if projection.state.summary.candidate.is_none() {
        None
    } else {
        raised_alert_room
    };
    projection.state.summary.alert_id = if projection.state.summary.candidate.is_none() {
        None
    } else if let Some(alert_id) = relabelled_alert_id {
        Some(alert_id)
    } else {
        // #1802: the summary is serialized but this sequence is process-local, so a
        // restore can bring back a published identity with the counter at its default.
        // Mint above everything already published, so a value is never issued twice.
        let published = previous_alert_id.unwrap_or(0);
        state.native_attention_context.alert_sequence =
            state.native_attention_context.alert_sequence.max(published) + 1;
        Some(state.native_attention_context.alert_sequence)
    };
    let suppression = match projection.state.dispatch {
        crate::state::NativeAttentionDispatchState::Suppressed { reason } => Some(reason),
        _ => None,
    };
    let diagnostic = AppEffect::RecordNativeAttentionRecomputed {
        observation,
        unread_count: projection.state.summary.unread_count,
        notification_count: projection.notification_count,
        badge_count: projection.state.summary.badge_count,
        badge_room_count: projection.badge_room_count,
        badge_excluded_room_count: projection.badge_excluded_room_count,
        candidate: projection
            .state
            .summary
            .candidate
            .as_ref()
            .map(|candidate| candidate.kind),
        suppression,
        window_focused: state.native_attention_context.window_focused,
        active_room_match: projection.active_room_match,
    };
    let changed = state.native_attention != projection.state;
    if changed {
        state.native_attention = projection.state;
    }
    (changed, diagnostic)
}

pub(crate) fn handle_native_window_focus_changed(
    state: &mut AppState,
    focused: bool,
    observation_generation: u64,
) -> Vec<AppEffect> {
    if observation_generation
        <= state
            .native_attention_context
            .window_focus_observation_generation
    {
        return Vec::new();
    }
    state
        .native_attention_context
        .window_focus_observation_generation = observation_generation;
    if state.native_attention_context.window_focused == focused {
        return Vec::new();
    }
    state.native_attention_context.window_focused = focused;

    let mut next = projected_native_attention_from_rooms(
        state,
        NativeAttentionObservationKind::InitialSync,
        None,
        None,
    );
    next.state.summary.candidate = None;
    next.state.summary.alert_id = None;
    next.state.summary.alert_room = None;
    next.state.dispatch = crate::state::NativeAttentionDispatchState::Idle;
    if state.native_attention == next.state {
        return Vec::new();
    }

    state.native_attention = next.state;
    vec![AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged)]
}

pub(crate) fn apply_badge_setting(state: &mut AppState) -> bool {
    let next = projected_native_attention_from_rooms(
        state,
        NativeAttentionObservationKind::InitialSync,
        None,
        None,
    )
    .state
    .summary
    .badge_count;
    let mut changed = state.native_attention.summary.badge_count != next;
    state.native_attention.summary.badge_count = next;
    // Turning the agent role on withdraws an alert that was already published.
    if state.is_agent_account()
        && (state.native_attention.summary.candidate.is_some()
            || state.native_attention.notification.is_some()
            || state.native_attention.summary.alert_id.is_some())
    {
        silence_owner_alerts(&mut state.native_attention);
        changed = true;
    }
    changed
}

pub(crate) fn handle_japanese_catalog_profile_changed(
    state: &mut AppState,
    profile: crate::state::JapaneseCatalogProfile,
) -> Vec<AppEffect> {
    if state.cjk_text_policy.japanese_catalog == profile {
        return Vec::new();
    }

    state.cjk_text_policy.japanese_catalog = profile;
    vec![AppEffect::EmitUiEvent(UiEvent::CjkTextPolicyChanged)]
}
