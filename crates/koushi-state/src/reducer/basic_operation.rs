use crate::{
    effect::{AppEffect, UiEvent},
    state::{
        AppError, AppState, BasicOperationRequest, BasicOperationState, SpaceChildLinkOutcome,
        SpaceChildLinkResult,
    },
};

use crate::room_address::{
    RoomAddressAvailability, RoomAddressAvailabilityState, RoomAddressSuggestion,
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

pub(crate) fn handle_space_child_link_settled(
    state: &mut AppState,
    request_id: u64,
    space_id: String,
    child_room_id: String,
    outcome: SpaceChildLinkOutcome,
) -> Vec<AppEffect> {
    if state.basic_operation.request_id() != Some(request_id) {
        return Vec::new();
    }
    if outcome == SpaceChildLinkOutcome::Linked
        && let Some(space) = state
            .spaces
            .iter_mut()
            .find(|space| space.space_id == space_id)
        && !space.child_room_ids.contains(&child_room_id)
    {
        // Project the homeserver-accepted parent-side child now; the SDK room
        // cache can lag the state event until the next sync replaces the list.
        space.child_room_ids.push(child_room_id.clone());
        space.child_room_ids.sort();
    }
    state.space_child_links.record(SpaceChildLinkResult {
        request_id,
        space_id,
        child_room_id,
        outcome,
    });
    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
}

pub(crate) fn handle_room_address_availability_requested(
    state: &mut AppState,
    request_id: u64,
    full_alias: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    state.room_address_availability = RoomAddressAvailabilityState::Checking {
        request_id,
        full_alias,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
}

pub(crate) fn handle_room_address_availability_settled(
    state: &mut AppState,
    request_id: u64,
    full_alias: String,
    availability: RoomAddressAvailability,
    suggestion: Option<RoomAddressSuggestion>,
) -> Vec<AppEffect> {
    let matches = matches!(
        &state.room_address_availability,
        RoomAddressAvailabilityState::Checking {
            request_id: pending,
            full_alias: pending_alias,
        } if *pending == request_id && *pending_alias == full_alias
    );
    if !matches {
        return Vec::new();
    }
    state.room_address_availability = RoomAddressAvailabilityState::Checked {
        request_id,
        full_alias,
        availability,
        suggestion: suggestion.filter(|_| availability == RoomAddressAvailability::InUse),
    };
    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
}

pub(crate) fn handle_room_address_availability_cleared(state: &mut AppState) -> Vec<AppEffect> {
    if state.room_address_availability == RoomAddressAvailabilityState::Idle {
        return Vec::new();
    }
    state.room_address_availability = RoomAddressAvailabilityState::Idle;
    vec![AppEffect::EmitUiEvent(UiEvent::RoomListChanged)]
}
