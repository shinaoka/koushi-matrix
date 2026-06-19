use crate::{
    effect::{AppEffect, UiEvent},
    state::{
        AccountManagementCapabilities, AccountManagementState, AppState, AuthFailureKind,
        CapabilityState, DeviceSessionListState,
    },
};

use super::is_session_ready;

pub(crate) fn handle_device_sessions_load_requested(
    state: &mut AppState,
    request_id: u64,
) -> Vec<AppEffect> {
    if !is_session_ready(state)
        || matches!(
            state.device_sessions,
            DeviceSessionListState::Loading { .. }
        )
    {
        return Vec::new();
    }
    state.device_sessions = DeviceSessionListState::Loading { request_id };
    vec![AppEffect::EmitUiEvent(UiEvent::DeviceSessionsChanged)]
}

pub(crate) fn handle_device_sessions_loaded(
    state: &mut AppState,
    request_id: u64,
    devices: Vec<crate::state::DeviceSessionSummary>,
) -> Vec<AppEffect> {
    if !matches!(
        state.device_sessions,
        DeviceSessionListState::Loading {
            request_id: active
        } if active == request_id
    ) {
        return Vec::new();
    }
    state.device_sessions = DeviceSessionListState::Loaded { devices };
    vec![AppEffect::EmitUiEvent(UiEvent::DeviceSessionsChanged)]
}

pub(crate) fn handle_device_sessions_load_failed(
    state: &mut AppState,
    request_id: u64,
    kind: AuthFailureKind,
) -> Vec<AppEffect> {
    if !matches!(
        state.device_sessions,
        DeviceSessionListState::Loading {
            request_id: active
        } if active == request_id
    ) {
        return Vec::new();
    }
    state.device_sessions = DeviceSessionListState::Failed { request_id, kind };
    vec![AppEffect::EmitUiEvent(UiEvent::DeviceSessionsChanged)]
}

pub(crate) fn handle_account_management_requested(
    state: &mut AppState,
    request_id: u64,
    operation: crate::state::AccountManagementOperation,
) -> Vec<AppEffect> {
    if !is_session_ready(state)
        || matches!(
            state.account_management,
            AccountManagementState::Working { .. }
                | AccountManagementState::AwaitingUia { .. }
        )
    {
        return Vec::new();
    }
    state.account_management = AccountManagementState::Working {
        request_id,
        operation,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::AccountManagementChanged)]
}

pub(crate) fn handle_account_management_uia_required(
    state: &mut AppState,
    request_id: u64,
    flow_id: u64,
    operation: crate::state::AccountManagementOperation,
) -> Vec<AppEffect> {
    if !matches!(
        state.account_management,
        AccountManagementState::Working {
            request_id: active,
            operation: active_operation,
        } if active == request_id && active_operation == operation
    ) {
        return Vec::new();
    }
    state.account_management = AccountManagementState::AwaitingUia {
        request_id,
        flow_id,
        operation,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::AccountManagementChanged)]
}

pub(crate) fn handle_account_management_succeeded(
    state: &mut AppState,
    request_id: u64,
    operation: crate::state::AccountManagementOperation,
) -> Vec<AppEffect> {
    if !account_management_matches(&state.account_management, request_id, operation) {
        return Vec::new();
    }
    state.account_management = AccountManagementState::Succeeded {
        request_id,
        operation,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::AccountManagementChanged)]
}

pub(crate) fn handle_account_management_failed(
    state: &mut AppState,
    request_id: u64,
    operation: crate::state::AccountManagementOperation,
    kind: AuthFailureKind,
) -> Vec<AppEffect> {
    if !account_management_matches(&state.account_management, request_id, operation) {
        return Vec::new();
    }
    state.account_management = AccountManagementState::Failed {
        request_id,
        operation,
        kind,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::AccountManagementChanged)]
}

pub(crate) fn handle_account_management_auth_submitted(
    state: &mut AppState,
    request_id: u64,
    flow_id: u64,
) -> Vec<AppEffect> {
    let operation = match &state.account_management {
        AccountManagementState::AwaitingUia {
            request_id: active_request_id,
            flow_id: active_flow_id,
            operation,
        } if *active_request_id == request_id && *active_flow_id == flow_id => *operation,
        _ => return Vec::new(),
    };
    state.account_management = AccountManagementState::Working {
        request_id,
        operation,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::AccountManagementChanged)]
}

pub(crate) fn handle_account_management_capabilities_load_requested(
    state: &mut AppState,
) -> Vec<AppEffect> {
    state.account_management_capabilities = AccountManagementCapabilities::default();
    vec![AppEffect::EmitUiEvent(
        UiEvent::AccountManagementCapabilitiesChanged,
    )]
}

pub(crate) fn handle_account_management_capabilities_loaded(
    state: &mut AppState,
    change_password: bool,
) -> Vec<AppEffect> {
    state.account_management_capabilities.change_password = if change_password {
        CapabilityState::Enabled
    } else {
        CapabilityState::Disabled
    };
    vec![AppEffect::EmitUiEvent(
        UiEvent::AccountManagementCapabilitiesChanged,
    )]
}

pub(crate) fn handle_account_management_capabilities_load_failed(
    state: &mut AppState,
) -> Vec<AppEffect> {
    state.account_management_capabilities = AccountManagementCapabilities::default();
    vec![AppEffect::EmitUiEvent(
        UiEvent::AccountManagementCapabilitiesChanged,
    )]
}

// --- Private helpers ---

fn account_management_matches(
    state: &AccountManagementState,
    request_id: u64,
    operation: crate::state::AccountManagementOperation,
) -> bool {
    matches!(
        state,
        AccountManagementState::Working {
            request_id: active,
            operation: active_operation,
        }
        | AccountManagementState::AwaitingUia {
            request_id: active,
            operation: active_operation,
            ..
        } if *active == request_id && *active_operation == operation
    )
}
