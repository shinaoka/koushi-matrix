use crate::{
    effect::{AppEffect, UiEvent},
    state::{AppState, LocalEncryptionState},
};

pub(crate) fn handle_local_encryption_probe_requested(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    let next = LocalEncryptionState::Probing { request_id };
    if state.local_encryption == next {
        return Vec::new();
    }

    state.local_encryption = next;
    vec![AppEffect::EmitUiEvent(UiEvent::LocalEncryptionChanged)]
}

pub(crate) fn handle_local_encryption_health_changed(
    state: &mut AppState,
    request_id: u64,
    health: crate::state::LocalEncryptionHealth,
) -> Vec<AppEffect> {
    let LocalEncryptionState::Probing {
        request_id: current_request_id,
    } = state.local_encryption
    else {
        return Vec::new();
    };
    if current_request_id != request_id {
        return Vec::new();
    }

    let next = LocalEncryptionState::from(health);
    if state.local_encryption == next {
        return Vec::new();
    }

    state.local_encryption = next;
    vec![AppEffect::EmitUiEvent(UiEvent::LocalEncryptionChanged)]
}

pub(crate) fn handle_reset_local_data_requested(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    if !matches!(
        state.local_encryption,
        LocalEncryptionState::MissingCredential | LocalEncryptionState::ResetRequired
    ) {
        return Vec::new();
    }

    state.local_encryption = LocalEncryptionState::Resetting { request_id };
    vec![AppEffect::EmitUiEvent(UiEvent::LocalEncryptionChanged)]
}

pub(crate) fn handle_reset_local_data_completed(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    let LocalEncryptionState::Resetting {
        request_id: current_request_id,
    } = state.local_encryption
    else {
        return Vec::new();
    };
    if current_request_id != request_id {
        return Vec::new();
    }

    state.local_encryption = LocalEncryptionState::Unknown;
    vec![AppEffect::EmitUiEvent(UiEvent::LocalEncryptionChanged)]
}

pub(crate) fn handle_reset_local_data_failed(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    let LocalEncryptionState::Resetting {
        request_id: current_request_id,
    } = state.local_encryption
    else {
        return Vec::new();
    };
    if current_request_id != request_id {
        return Vec::new();
    }

    state.local_encryption = LocalEncryptionState::ResetRequired;
    vec![AppEffect::EmitUiEvent(UiEvent::LocalEncryptionChanged)]
}
