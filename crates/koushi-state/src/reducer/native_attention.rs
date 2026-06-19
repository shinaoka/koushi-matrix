use crate::{
    effect::{AppEffect, UiEvent},
    state::AppState,
};

pub(crate) fn handle_native_attention_updated(
    state: &mut AppState,
    attention: crate::state::NativeAttentionState,
) -> Vec<AppEffect> {
    if state.native_attention == attention {
        return Vec::new();
    }

    state.native_attention = attention;
    vec![AppEffect::EmitUiEvent(UiEvent::NativeAttentionChanged)]
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
