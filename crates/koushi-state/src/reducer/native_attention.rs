use crate::{
    effect::{AppEffect, UiEvent},
    state::AppState,
};

use super::is_session_ready;

pub(crate) fn handle_dispatch_started(
    state: &mut AppState,
    dispatch_id: crate::state::NativeAttentionDispatchId,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    if state.native_attention.summary.candidate.is_none() {
        return Vec::new();
    }
    if matches!(
        state.native_attention.dispatch,
        crate::state::NativeAttentionDispatchState::Dispatching { .. }
            | crate::state::NativeAttentionDispatchState::Suppressed { .. }
    ) {
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
    if !state.settings.values.notifications.badges {
        attention.summary.badge_count = 0;
    }
    attention.dispatch = state.native_attention.dispatch.clone();
    if state.native_attention == attention {
        return Vec::new();
    }

    state.native_attention = attention;
    vec![AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged)]
}

pub(crate) fn apply_badge_setting(state: &mut AppState) -> bool {
    let next = if state.settings.values.notifications.badges
        && state.native_attention.summary.capabilities.badge
            != crate::state::NativeAttentionCapability::Unavailable
    {
        state.native_attention.summary.unread_count
    } else {
        0
    };
    if state.native_attention.summary.badge_count == next {
        return false;
    }
    state.native_attention.summary.badge_count = next;
    true
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
