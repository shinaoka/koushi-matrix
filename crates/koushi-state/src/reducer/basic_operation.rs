use crate::{
    effect::{AppEffect, UiEvent},
    state::{AppError, AppState, BasicOperationRequest, BasicOperationState},
};

use super::is_session_ready;

pub(crate) fn handle_clear_error(state: &mut AppState, code: String) -> Vec<AppEffect> {
    state.errors.retain(|error| error.code != code);
    vec![AppEffect::EmitUiEvent(UiEvent::ErrorChanged)]
}

pub(crate) fn handle_basic_operation_requested(
    state: &mut AppState,
    request_id: u64,
    request: BasicOperationRequest,
) -> Vec<AppEffect> {
    if !is_session_ready(state) || !state.basic_operation.is_idle() {
        return Vec::new();
    }
    state.basic_operation = match request {
        BasicOperationRequest::CreateRoom { name } => {
            BasicOperationState::CreatingRoom { request_id, name }
        }
        BasicOperationRequest::CreateSpace { name } => {
            BasicOperationState::CreatingSpace { request_id, name }
        }
        BasicOperationRequest::LinkSpaceChild {
            space_id,
            child_room_id,
        } => BasicOperationState::LinkingSpaceChild {
            request_id,
            space_id,
            child_room_id,
        },
    };
    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
}

pub(crate) fn handle_basic_operation_succeeded(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    if state.basic_operation.request_id() != Some(request_id) {
        return Vec::new();
    }
    state.basic_operation = BasicOperationState::Idle;
    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
}

pub(crate) fn handle_basic_operation_failed(
    state: &mut AppState,
    request_id: u64,
    message: String,
) -> Vec<AppEffect> {
    if state.basic_operation.request_id() != Some(request_id) {
        return Vec::new();
    }
    state.basic_operation = BasicOperationState::Idle;
    state.errors.push(AppError {
        code: "basic_operation_failed".to_owned(),
        message,
        recoverable: true,
    });
    vec![
        AppEffect::EmitUiEvent(UiEvent::RoomListChanged),
        AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
    ]
}
