use crate::{
    effect::{AppEffect, UiEvent},
    state::{
        AppState, InviteDestinationResult, InviteOperationState, InviteScopeSelection,
        InviteWorkflowState, OperationFailureKind, SessionState, build_invite_history_policy,
        build_invite_scope_plan, build_invite_target_query_state, invite_notice_from_results,
        selected_target_from_query,
    },
};

fn destination_exists(state: &AppState, room_id: &str) -> bool {
    state.rooms.iter().any(|room| room.room_id == room_id)
        || state.spaces.iter().any(|space| space.space_id == room_id)
}

fn operation_is_pending(state: &AppState) -> bool {
    matches!(
        state.invite_workflow.operation,
        InviteOperationState::Pending { .. }
    )
}

fn active_destination(state: &AppState) -> Option<&str> {
    state.invite_workflow.query.room_id.as_deref()
}

fn has_history_disclosure_context(state: &AppState) -> bool {
    matches!(
        &state.session,
        SessionState::Ready(_)
            | SessionState::AwaitingVerification { .. }
            | SessionState::Verifying { .. }
            | SessionState::AwaitingBootstrapConfirmation { .. }
            | SessionState::Locked(_)
    )
}

fn pending_operation_matches(state: &AppState, request_id: u64, room_id: &str) -> bool {
    matches!(
        &state.invite_workflow.operation,
        InviteOperationState::Pending {
            request_id: active_request_id,
            room_id: active_room_id,
            ..
        } if *active_request_id == request_id && active_room_id.as_str() == room_id
    )
}

fn refresh_invite_projection(state: &mut AppState, room_id: &str) {
    let plan = build_invite_scope_plan(state, room_id.to_owned());
    let selected_scope = state
        .invite_workflow
        .selected_scope
        .clone()
        .filter(|scope| plan.options.iter().any(|option| option.scope == *scope))
        .or_else(|| Some(plan.default_scope.clone()));
    state.invite_workflow.scope_plan = Some(plan);
    state.invite_workflow.selected_scope = selected_scope;
    state.invite_workflow.history_policy = Some(build_invite_history_policy(state, room_id));
}

pub(crate) fn handle_invite_workflow_opened(
    state: &mut AppState,
    room_id: String,
) -> Vec<AppEffect> {
    if !has_history_disclosure_context(state)
        || !destination_exists(state, &room_id)
        || operation_is_pending(state)
    {
        return Vec::new();
    }

    refresh_invite_projection(state, &room_id);
    state.invite_workflow.query.room_id = Some(room_id);
    vec![AppEffect::EmitUiEvent(UiEvent::InviteWorkflowChanged)]
}

pub(crate) fn handle_invite_workflow_closed(state: &mut AppState) -> Vec<AppEffect> {
    state.invite_workflow = InviteWorkflowState::default();
    vec![AppEffect::EmitUiEvent(UiEvent::InviteWorkflowChanged)]
}

pub(crate) fn handle_invite_target_query_changed(
    state: &mut AppState,
    room_id: String,
    query: String,
) -> Vec<AppEffect> {
    if !matches!(&state.session, SessionState::Ready(_))
        || !destination_exists(state, &room_id)
        || operation_is_pending(state)
        || active_destination(state).is_some_and(|active| active != room_id)
    {
        return Vec::new();
    }

    refresh_invite_projection(state, &room_id);
    state.invite_workflow.query = build_invite_target_query_state(state, room_id, query);
    vec![AppEffect::EmitUiEvent(UiEvent::InviteWorkflowChanged)]
}

pub(crate) fn handle_invite_scope_selected(
    state: &mut AppState,
    room_id: String,
    scope: InviteScopeSelection,
) -> Vec<AppEffect> {
    if !matches!(&state.session, SessionState::Ready(_))
        || operation_is_pending(state)
        || active_destination(state) != Some(room_id.as_str())
        || !destination_exists(state, &room_id)
    {
        return Vec::new();
    }
    let Some(plan) = state.invite_workflow.scope_plan.as_ref() else {
        return Vec::new();
    };
    if plan.room_id != room_id || !plan.options.iter().any(|option| option.scope == scope) {
        return Vec::new();
    }

    state.invite_workflow.selected_scope = Some(scope);
    vec![AppEffect::EmitUiEvent(UiEvent::InviteWorkflowChanged)]
}

pub(crate) fn handle_invite_target_selected(
    state: &mut AppState,
    room_id: String,
    user_id: String,
) -> Vec<AppEffect> {
    if !matches!(&state.session, SessionState::Ready(_))
        || operation_is_pending(state)
        || active_destination(state) != Some(room_id.as_str())
        || !destination_exists(state, &room_id)
        || state
            .invite_workflow
            .selected_targets
            .iter()
            .any(|target| target.user_id == user_id)
    {
        return Vec::new();
    }

    let Some(target) = selected_target_from_query(&state.invite_workflow, &user_id) else {
        return Vec::new();
    };
    state.invite_workflow.selected_targets.push(target);
    let query = state.invite_workflow.query.query.clone();
    state.invite_workflow.query = build_invite_target_query_state(state, room_id, query);
    vec![AppEffect::EmitUiEvent(UiEvent::InviteWorkflowChanged)]
}

pub(crate) fn handle_invite_target_removed(
    state: &mut AppState,
    user_id: String,
) -> Vec<AppEffect> {
    if !matches!(&state.session, SessionState::Ready(_)) || operation_is_pending(state) {
        return Vec::new();
    }
    let Some(room_id) = active_destination(state).map(str::to_owned) else {
        return Vec::new();
    };
    if !destination_exists(state, &room_id)
        || !state
            .invite_workflow
            .selected_targets
            .iter()
            .any(|target| target.user_id == user_id)
    {
        return Vec::new();
    }

    state
        .invite_workflow
        .selected_targets
        .retain(|target| target.user_id != user_id);
    let query = state.invite_workflow.query.query.clone();
    state.invite_workflow.query = build_invite_target_query_state(state, room_id, query);
    vec![AppEffect::EmitUiEvent(UiEvent::InviteWorkflowChanged)]
}

pub(crate) fn handle_invite_batch_requested(
    state: &mut AppState,
    request_id: u64,
    room_id: String,
    user_ids: Vec<String>,
    scope: InviteScopeSelection,
) -> Vec<AppEffect> {
    if !matches!(&state.session, SessionState::Ready(_))
        || operation_is_pending(state)
        || active_destination(state) != Some(room_id.as_str())
        || !destination_exists(state, &room_id)
        || user_ids.is_empty()
    {
        return Vec::new();
    }

    let Some(plan) = state.invite_workflow.scope_plan.as_ref() else {
        return Vec::new();
    };
    if plan.room_id != room_id || !plan.options.iter().any(|option| option.scope == scope) {
        return Vec::new();
    }
    let effective_scope = state
        .invite_workflow
        .selected_scope
        .as_ref()
        .unwrap_or(&plan.default_scope);
    if effective_scope != &scope {
        return Vec::new();
    }

    let selected_ids = state
        .invite_workflow
        .selected_targets
        .iter()
        .map(|target| target.user_id.as_str())
        .collect::<Vec<_>>();
    if selected_ids.len() != user_ids.len()
        || selected_ids
            .iter()
            .zip(&user_ids)
            .any(|(selected, requested)| *selected != requested.as_str())
    {
        return Vec::new();
    }

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
    if !pending_operation_matches(state, request_id, &room_id) {
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
    if !pending_operation_matches(state, request_id, &room_id) {
        return Vec::new();
    }

    state.invite_workflow.operation = InviteOperationState::Failed {
        request_id,
        room_id,
        kind,
    };
    Vec::new()
}
