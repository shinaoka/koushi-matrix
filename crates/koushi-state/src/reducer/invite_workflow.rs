use crate::{
    effect::AppEffect,
    state::{
        AppState, InviteDestinationResult, InviteOperationState, InviteScopeSelection,
        InviteWorkflowState, OperationFailureKind, build_invite_scope_plan,
        build_invite_target_query_state, invite_notice_from_results, selected_target_from_query,
    },
};

pub(crate) fn handle_invite_workflow_opened(
    state: &mut AppState,
    room_id: String,
) -> Vec<AppEffect> {
    state.invite_workflow.scope_plan = Some(build_invite_scope_plan(state, room_id.clone()));
    state.invite_workflow.query.room_id = Some(room_id);
    Vec::new()
}

pub(crate) fn handle_invite_workflow_closed(state: &mut AppState) -> Vec<AppEffect> {
    state.invite_workflow = InviteWorkflowState::default();
    Vec::new()
}

pub(crate) fn handle_invite_target_query_changed(
    state: &mut AppState,
    room_id: String,
    query: String,
) -> Vec<AppEffect> {
    state.invite_workflow.scope_plan = Some(build_invite_scope_plan(state, room_id.clone()));
    state.invite_workflow.query = build_invite_target_query_state(state, room_id, query);
    Vec::new()
}

pub(crate) fn handle_invite_target_selected(
    state: &mut AppState,
    room_id: String,
    user_id: String,
) -> Vec<AppEffect> {
    if state
        .invite_workflow
        .selected_targets
        .iter()
        .any(|target| target.user_id == user_id)
    {
        return Vec::new();
    }

    if state.invite_workflow.query.room_id.as_deref() != Some(room_id.as_str()) {
        state.invite_workflow.query =
            build_invite_target_query_state(state, room_id.clone(), user_id.clone());
    }

    if let Some(target) = selected_target_from_query(&state.invite_workflow, &user_id) {
        state.invite_workflow.selected_targets.push(target);
        let query = state.invite_workflow.query.query.clone();
        state.invite_workflow.query = build_invite_target_query_state(state, room_id, query);
    }
    Vec::new()
}

pub(crate) fn handle_invite_target_removed(
    state: &mut AppState,
    user_id: String,
) -> Vec<AppEffect> {
    state
        .invite_workflow
        .selected_targets
        .retain(|target| target.user_id != user_id);
    if let Some(room_id) = state.invite_workflow.query.room_id.clone() {
        let query = state.invite_workflow.query.query.clone();
        state.invite_workflow.query = build_invite_target_query_state(state, room_id, query);
    }
    Vec::new()
}

pub(crate) fn handle_invite_batch_requested(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    user_ids: Vec<String>,
    scope: InviteScopeSelection,
) -> Vec<AppEffect> {
    state.invite_workflow.operation = InviteOperationState::Pending {
        request_id,
        room_id,
        user_ids,
        scope,
    };
    Vec::new()
}

pub(crate) fn handle_invite_batch_completed(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    results: Vec<InviteDestinationResult>,
) -> Vec<AppEffect> {
    if !matches!(
        &state.invite_workflow.operation,
        InviteOperationState::Pending {
            request_id: active,
            ..
        } if *active == request_id
    ) {
        return Vec::new();
    }

    state.invite_workflow.selected_targets.clear();
    state.invite_workflow.operation = InviteOperationState::Completed {
        request_id,
        room_id,
        notice: invite_notice_from_results(&results),
        results,
    };
    Vec::new()
}

pub(crate) fn handle_invite_batch_failed(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    kind: OperationFailureKind,
) -> Vec<AppEffect> {
    if !matches!(
        &state.invite_workflow.operation,
        InviteOperationState::Pending {
            request_id: active,
            ..
        } if *active == request_id
    ) {
        return Vec::new();
    }

    state.invite_workflow.operation = InviteOperationState::Failed {
        request_id,
        room_id,
        kind,
    };
    Vec::new()
}
