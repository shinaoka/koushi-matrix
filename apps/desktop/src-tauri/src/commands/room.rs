use super::directory::{
    build_accept_invite_command, build_cancel_space_invite_command,
    build_close_invite_workflow_command, build_create_room_command, build_create_space_command,
    build_decline_invite_command, build_invite_targets_command, build_invite_user_command,
    build_invite_user_to_space_command, build_join_room_command,
    build_open_invite_workflow_command, build_remove_invite_target_command,
    build_search_invite_targets_command, build_select_invite_target_command,
    build_set_invite_scope_command, build_set_space_child_command,
    build_start_direct_message_command,
};
use super::navigation::SELECT_ROOM_EVENT_TIMEOUT;
use super::*;

const INVITE_WORKFLOW_CONVERGENCE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

async fn submit_invite_workflow_command(
    state: &CoreRuntimeState,
    request_id: RequestId,
    command: CoreCommand,
    room_id: Option<String>,
    context: &'static str,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let baseline = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline.state);
    let room_id = room_id
        .or_else(|| baseline.state.invite_workflow.query.room_id.clone())
        .unwrap_or_default();
    let query = baseline.state.invite_workflow.query.query.clone();
    submit_core_command(state, command).await?;
    let outcome = event_conn
        .wait_for_request_outcome(
            OutcomeCorrelation::Request(request_id),
            RequestOutcomeExpectation::InviteWorkflow {
                request_id,
                account_key,
                room_id,
                query,
                closed: false,
            },
            baseline.generation,
            tokio::time::Instant::now() + INVITE_WORKFLOW_CONVERGENCE_TIMEOUT,
        )
        .await
        .map_err(|error| invoke_error_from_request_outcome(context, error))?;
    let RequestOutcome::InviteWorkflow { generation, .. } = outcome else {
        return Err(format!("{context} returned an invalid outcome"));
    };
    Ok(command_settlement(generation))
}

async fn submit_room_operation(
    state: &CoreRuntimeState,
    request_id: RequestId,
    command: CoreCommand,
    room_id: String,
    operation: RoomOperationKind,
    context: &'static str,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let baseline = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline.state);
    submit_core_command(state, command).await?;
    let generation = wait_for_room_operation(
        &mut event_conn,
        request_id,
        baseline.generation,
        account_key,
        room_id,
        operation,
        ROOM_OPERATION_EVENT_TIMEOUT,
        context,
    )
    .await?;
    Ok(command_settlement(generation))
}

#[tauri::command]
pub async fn open_invite_workflow(
    room_id: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let baseline = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline.state);
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_open_invite_workflow_command(
            request_id,
            room_id.clone(),
        ))
        .await
        .map_err(|error| format!("command submit failed: {error}"))?;
    let outcome = event_conn
        .wait_for_request_outcome(
            OutcomeCorrelation::Request(request_id),
            RequestOutcomeExpectation::InviteWorkflow {
                request_id,
                account_key,
                room_id,
                query: String::new(),
                closed: false,
            },
            baseline.generation,
            tokio::time::Instant::now() + INVITE_WORKFLOW_CONVERGENCE_TIMEOUT,
        )
        .await
        .map_err(|error| invoke_error_from_request_outcome("invite workflow open", error))?;
    let RequestOutcome::InviteWorkflow { generation, .. } = outcome else {
        return Err("invite workflow open returned an invalid outcome".to_owned());
    };
    Ok(command_settlement(generation))
}

#[tauri::command]
pub async fn close_invite_workflow(
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let baseline = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline.state);
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_close_invite_workflow_command(request_id))
        .await
        .map_err(|error| format!("command submit failed: {error}"))?;
    let outcome = event_conn
        .wait_for_request_outcome(
            OutcomeCorrelation::Request(request_id),
            RequestOutcomeExpectation::InviteWorkflow {
                request_id,
                account_key,
                room_id: String::new(),
                query: String::new(),
                closed: true,
            },
            baseline.generation,
            tokio::time::Instant::now() + INVITE_WORKFLOW_CONVERGENCE_TIMEOUT,
        )
        .await
        .map_err(|error| invoke_error_from_request_outcome("invite workflow close", error))?;
    let RequestOutcome::InviteWorkflow { generation, .. } = outcome else {
        return Err("invite workflow close returned an invalid outcome".to_owned());
    };
    Ok(command_settlement(generation))
}

#[tauri::command]
pub async fn search_invite_targets(
    room_id: String,
    query: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let baseline = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline.state);
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_search_invite_targets_command(
            request_id,
            room_id.clone(),
            query.clone(),
        ))
        .await
        .map_err(|error| format!("command submit failed: {error}"))?;
    let outcome = event_conn
        .wait_for_request_outcome(
            OutcomeCorrelation::Request(request_id),
            RequestOutcomeExpectation::InviteWorkflow {
                request_id,
                account_key,
                room_id,
                query,
                closed: false,
            },
            baseline.generation,
            tokio::time::Instant::now() + INVITE_WORKFLOW_CONVERGENCE_TIMEOUT,
        )
        .await
        .map_err(|error| invoke_error_from_request_outcome("invite target search", error))?;
    let RequestOutcome::InviteWorkflow { generation, .. } = outcome else {
        return Err("invite target search returned an invalid outcome".to_owned());
    };
    Ok(command_settlement(generation))
}

#[tauri::command]
pub async fn set_invite_scope(
    room_id: String,
    scope: InviteScopeSelection,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let request_id = next_request_id(state.inner()).await;
    let expected_room_id = room_id.clone();
    submit_invite_workflow_command(
        state.inner(),
        request_id,
        build_set_invite_scope_command(request_id, room_id, scope),
        Some(expected_room_id),
        "invite scope update",
    )
    .await
}

#[tauri::command]
pub async fn select_invite_target(
    room_id: String,
    user_id: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let request_id = next_request_id(state.inner()).await;
    let expected_room_id = room_id.clone();
    submit_invite_workflow_command(
        state.inner(),
        request_id,
        build_select_invite_target_command(request_id, room_id, user_id),
        Some(expected_room_id),
        "invite target selection",
    )
    .await
}

#[tauri::command]
pub async fn remove_invite_target(
    user_id: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_invite_workflow_command(
        state.inner(),
        request_id,
        build_remove_invite_target_command(request_id, user_id),
        None,
        "invite target removal",
    )
    .await
}

#[tauri::command]
pub async fn select_room_list_filter(
    filter: RoomListFilter,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        CoreCommand::App(AppCommand::SelectRoomListFilter { request_id, filter }),
    )
    .await?;
    Ok(admission)
}

#[tauri::command]
pub async fn mark_room_as_read(
    room_id: String,
    event_id: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        CoreCommand::Room(RoomCommand::MarkRoomAsRead {
            request_id,
            room_id,
            event_id,
        }),
    )
    .await?;
    Ok(admission)
}

#[tauri::command]
pub async fn mark_room_as_unread(
    room_id: String,
    unread: bool,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        CoreCommand::Room(RoomCommand::MarkRoomAsUnread {
            request_id,
            room_id,
            unread,
        }),
    )
    .await?;
    Ok(admission)
}

#[tauri::command]
pub async fn force_rotate_outbound_session(
    room_id: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_room_operation(
        state.inner(),
        request_id,
        build_force_rotate_outbound_session_command(request_id, room_id.clone()),
        room_id,
        RoomOperationKind::OutboundSessionRotationForced,
        "outbound session rotation",
    )
    .await
}

#[tauri::command]
pub async fn set_room_notification_mode(
    room_id: String,
    mode: RoomNotificationMode,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        CoreCommand::Room(RoomCommand::SetRoomNotificationMode {
            request_id,
            room_id,
            mode,
        }),
    )
    .await?;
    Ok(admission)
}

#[tauri::command]
pub async fn leave_room(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_leave_room_command(request_id, room_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn forget_room(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_forget_room_command(request_id, room_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn set_room_tag(
    room_id: String,
    tag: RoomTagKind,
    order: Option<f64>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let request_id = next_request_id(state.inner()).await;
    let settlement = submit_room_operation(
        state.inner(),
        request_id,
        build_set_room_tag_command(request_id, room_id.clone(), tag.clone(), order),
        room_id,
        RoomOperationKind::RoomTagSet { tag },
        "room tag update",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(settlement)
}

#[tauri::command]
pub async fn remove_room_tag(
    room_id: String,
    tag: RoomTagKind,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let request_id = next_request_id(state.inner()).await;
    let settlement = submit_room_operation(
        state.inner(),
        request_id,
        build_remove_room_tag_command(request_id, room_id.clone(), tag.clone()),
        room_id,
        RoomOperationKind::RoomTagRemoved { tag },
        "room tag removal",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(settlement)
}

#[tauri::command]
pub async fn pin_event(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let request_id = next_request_id(state.inner()).await;
    let settlement = submit_room_operation(
        state.inner(),
        request_id,
        build_pin_event_command(request_id, room_id.clone(), event_id),
        room_id,
        RoomOperationKind::PinnedEventsRefreshed,
        "event pin",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(settlement)
}

#[tauri::command]
pub async fn unpin_event(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let request_id = next_request_id(state.inner()).await;
    let settlement = submit_room_operation(
        state.inner(),
        request_id,
        build_unpin_event_command(request_id, room_id.clone(), event_id),
        room_id,
        RoomOperationKind::PinnedEventsRefreshed,
        "event unpin",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(settlement)
}

#[tauri::command]
pub async fn refresh_pinned_events(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let baseline = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline.state);
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_refresh_pinned_events_command(
            request_id,
            room_id.clone(),
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    let generation = wait_for_room_operation(
        &mut event_conn,
        request_id,
        baseline.generation,
        account_key,
        room_id.clone(),
        RoomOperationKind::PinnedEventsRefreshed,
        ROOM_OPERATION_EVENT_TIMEOUT,
        "pinned messages refresh",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(command_settlement(generation))
}

#[tauri::command]
pub async fn load_room_settings(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let baseline = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline.state);
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_load_room_settings_command(
            request_id,
            room_id.clone(),
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    let generation = wait_for_room_operation(
        &mut event_conn,
        request_id,
        baseline.generation,
        account_key,
        room_id,
        RoomOperationKind::RoomSettingsLoaded,
        ROOM_OPERATION_EVENT_TIMEOUT,
        "room settings load",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(command_settlement(generation))
}

#[tauri::command]
pub async fn load_space_members(
    space_id: String,
    generation: u64,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let baseline = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline.state);
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_load_space_members_command(
            request_id,
            space_id.clone(),
            generation,
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    let generation = wait_for_room_operation(
        &mut event_conn,
        request_id,
        baseline.generation,
        account_key,
        space_id,
        RoomOperationKind::SpaceMembersLoaded { generation },
        ROOM_OPERATION_EVENT_TIMEOUT,
        "Space member load",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(command_settlement(generation))
}

#[tauri::command]
pub async fn query_mention_candidates(
    room_id: String,
    surface: MentionSurface,
    query: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<(), String> {
    let request_id = next_request_id(state.inner()).await;
    let account_key = account_key_from_snapshot(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::Room(RoomCommand::QueryMentionCandidates {
            request_id,
            account_key,
            room_id,
            surface,
            query,
        }),
    )
    .await
}

#[tauri::command]
pub async fn repair_room_timeline(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_repair_room_timeline_command(request_id, room_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn update_room_setting(
    room_id: String,
    change: RoomSettingChange,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let baseline = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline.state);
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_update_room_setting_command(
            request_id,
            room_id.clone(),
            change,
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    let generation = wait_for_room_operation(
        &mut event_conn,
        request_id,
        baseline.generation,
        account_key,
        room_id,
        RoomOperationKind::RoomSettingUpdated,
        ROOM_OPERATION_EVENT_TIMEOUT,
        "room setting update",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(command_settlement(generation))
}

#[tauri::command]
pub async fn moderate_room_member(
    room_id: String,
    target_user_id: String,
    action: RoomModerationAction,
    reason: Option<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let baseline = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline.state);
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_moderate_room_member_command(
            request_id,
            room_id.clone(),
            target_user_id.clone(),
            action,
            optional_non_blank(reason),
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    let generation = wait_for_room_operation(
        &mut event_conn,
        request_id,
        baseline.generation,
        account_key,
        room_id,
        RoomOperationKind::MemberModerated {
            target_user_id,
            action,
        },
        ROOM_OPERATION_EVENT_TIMEOUT,
        "room member moderation",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(command_settlement(generation))
}

#[tauri::command]
pub async fn update_room_member_role(
    room_id: String,
    target_user_id: String,
    power_level: i64,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let baseline = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline.state);
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_update_room_member_role_command(
            request_id,
            room_id.clone(),
            target_user_id.clone(),
            power_level,
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    let generation = wait_for_room_operation(
        &mut event_conn,
        request_id,
        baseline.generation,
        account_key,
        room_id,
        RoomOperationKind::MemberRoleUpdated { target_user_id },
        ROOM_OPERATION_EVENT_TIMEOUT,
        "room member role update",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(command_settlement(generation))
}

#[tauri::command]
pub fn preview_room_address(
    name: String,
    alias_localpart: Option<String>,
    state: State<'_, CoreRuntimeState>,
) -> koushi_state::RoomAddressPreview {
    state
        .runtime
        .attach()
        .preview_room_address(&name, alias_localpart.as_deref())
}

#[tauri::command]
pub async fn create_room(
    options: koushi_protocol::CreateRoomOptions,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let baseline = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline.state);
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_create_room_command(request_id, options))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    let outcome = event_conn
        .wait_for_request_outcome(
            OutcomeCorrelation::Request(request_id),
            RequestOutcomeExpectation::RoomCreated {
                request_id,
                account_key,
            },
            baseline.generation,
            tokio::time::Instant::now() + CREATE_EVENT_TIMEOUT,
        )
        .await
        .map_err(|error| invoke_error_from_request_outcome("room creation", error))?;
    let RequestOutcome::RoomCreated { generation, .. } = outcome else {
        return Err("room creation returned an invalid outcome".to_owned());
    };
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(command_settlement(generation))
}

#[tauri::command]
pub async fn create_space(
    name: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let baseline = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline.state);
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_create_space_command(request_id, name))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    let outcome = event_conn
        .wait_for_request_outcome(
            OutcomeCorrelation::Request(request_id),
            RequestOutcomeExpectation::SpaceCreated {
                request_id,
                account_key,
            },
            baseline.generation,
            tokio::time::Instant::now() + CREATE_EVENT_TIMEOUT,
        )
        .await
        .map_err(|error| invoke_error_from_request_outcome("space creation", error))?;
    let RequestOutcome::SpaceCreated { generation, .. } = outcome else {
        return Err("space creation returned an invalid outcome".to_owned());
    };
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(command_settlement(generation))
}

#[tauri::command]
pub async fn set_space_child(
    space_id: String,
    child_room_id: String,
    via_server: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_set_space_child_command(request_id, space_id, child_room_id, via_server),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn join_room(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let baseline = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline.state);
    let request_id = event_conn.next_request_id();
    let Some(command) = build_join_room_command(request_id, room_id.clone()) else {
        return Err("room id must not be blank".to_owned());
    };

    event_conn
        .command(command)
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    let outcome = event_conn
        .wait_for_request_outcome(
            OutcomeCorrelation::Request(request_id),
            RequestOutcomeExpectation::RoomJoined {
                request_id,
                account_key,
                room_id,
            },
            baseline.generation,
            tokio::time::Instant::now() + ROOM_OPERATION_EVENT_TIMEOUT,
        )
        .await
        .map_err(|error| invoke_error_from_request_outcome("room join", error))?;
    let RequestOutcome::RoomJoined { generation, .. } = outcome else {
        return Err("room join returned an invalid outcome".to_owned());
    };
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(command_settlement(generation))
}

#[tauri::command]
pub async fn accept_invite(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let baseline = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline.state);
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_accept_invite_command(request_id, room_id.clone()))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    let generation = wait_for_room_operation(
        &mut event_conn,
        request_id,
        baseline.generation,
        account_key,
        room_id,
        RoomOperationKind::InviteAccepted,
        ROOM_OPERATION_EVENT_TIMEOUT,
        "invite acceptance",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(command_settlement(generation))
}

#[tauri::command]
pub async fn decline_invite(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let baseline = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline.state);
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_decline_invite_command(request_id, room_id.clone()))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    let generation = wait_for_room_operation(
        &mut event_conn,
        request_id,
        baseline.generation,
        account_key,
        room_id,
        RoomOperationKind::InviteDeclined,
        ROOM_OPERATION_EVENT_TIMEOUT,
        "invite decline",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(command_settlement(generation))
}

#[tauri::command]
pub async fn start_direct_message(
    user_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let baseline = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline.state);
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_start_direct_message_command(request_id, user_id))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    // Core settles the event payload and the same-account room projection as
    // one composite outcome before the room is selected (#368).
    let outcome = event_conn
        .wait_for_request_outcome(
            OutcomeCorrelation::Request(request_id),
            RequestOutcomeExpectation::DirectMessageStarted {
                request_id,
                account_key,
            },
            baseline.generation,
            tokio::time::Instant::now() + ROOM_OPERATION_EVENT_TIMEOUT,
        )
        .await
        .map_err(|error| invoke_error_from_request_outcome("direct message start", error))?;
    let RequestOutcome::DirectMessageStarted { room_id, .. } = outcome else {
        return Err("direct message start returned an invalid outcome".to_owned());
    };
    let selected_generation = event_conn
        .select_room_and_wait(room_id.clone(), SELECT_ROOM_EVENT_TIMEOUT)
        .await
        .map_err(super::navigation::invoke_error_from_select_room_error)?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(command_settlement(selected_generation))
}

#[tauri::command]
pub async fn invite_user(
    room_id: String,
    user_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let baseline = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline.state);
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_invite_user_command(
            request_id,
            room_id.clone(),
            user_id.clone(),
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    let generation = wait_for_room_operation(
        &mut event_conn,
        request_id,
        baseline.generation,
        account_key,
        room_id,
        RoomOperationKind::UserInvited { user_id },
        ROOM_OPERATION_EVENT_TIMEOUT,
        "user invite",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(command_settlement(generation))
}

#[tauri::command]
pub async fn invite_user_to_space(
    space_id: String,
    user_id: String,
    generation: u64,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let baseline = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline.state);
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_invite_user_to_space_command(
            request_id,
            space_id.clone(),
            user_id.clone(),
            generation,
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    let generation = wait_for_room_operation(
        &mut event_conn,
        request_id,
        baseline.generation,
        account_key,
        space_id,
        RoomOperationKind::SpaceMemberInviteSettled {
            target_user_id: user_id,
            generation,
        },
        ROOM_OPERATION_EVENT_TIMEOUT,
        "Space member invite",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(command_settlement(generation))
}

#[tauri::command]
pub async fn update_space_member_role(
    space_id: String,
    user_id: String,
    generation: u64,
    expected_power_levels_revision: Option<String>,
    expected_power_level: i64,
    power_level: i64,
    confirmed: bool,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let baseline = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline.state);
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_update_space_member_role_command(
            request_id,
            space_id.clone(),
            user_id.clone(),
            generation,
            expected_power_levels_revision,
            expected_power_level,
            power_level,
            confirmed,
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    let generation = wait_for_room_operation(
        &mut event_conn,
        request_id,
        baseline.generation,
        account_key,
        space_id,
        RoomOperationKind::SpaceMemberRoleUpdated {
            target_user_id: user_id,
            generation,
        },
        ROOM_OPERATION_EVENT_TIMEOUT,
        "Space member role update",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(command_settlement(generation))
}

#[tauri::command]
pub async fn cancel_space_invite(
    space_id: String,
    user_id: String,
    generation: u64,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let baseline = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline.state);
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_cancel_space_invite_command(
            request_id,
            space_id.clone(),
            user_id.clone(),
            generation,
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    let generation = wait_for_room_operation(
        &mut event_conn,
        request_id,
        baseline.generation,
        account_key,
        space_id,
        RoomOperationKind::SpaceMemberInviteCancellationSettled {
            target_user_id: user_id,
            generation,
        },
        ROOM_OPERATION_EVENT_TIMEOUT,
        "Space member invite cancellation",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(command_settlement(generation))
}

#[tauri::command]
pub async fn invite_targets(
    room_id: String,
    user_ids: Vec<String>,
    scope: InviteScopeSelection,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let baseline = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline.state);
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_invite_targets_command(
            request_id,
            room_id.clone(),
            user_ids.clone(),
            scope.clone(),
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    let generation = wait_for_room_operation(
        &mut event_conn,
        request_id,
        baseline.generation,
        account_key,
        room_id,
        RoomOperationKind::InviteBatch { user_ids, scope },
        ROOM_OPERATION_EVENT_TIMEOUT,
        "invite batch",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(command_settlement(generation))
}

pub(super) async fn wait_for_room_operation(
    event_conn: &mut CoreConnection,
    operation_request_id: RequestId,
    baseline_generation: u64,
    account_key: AccountKey,
    room_id: String,
    operation: RoomOperationKind,
    timeout: std::time::Duration,
    context: &'static str,
) -> Result<u64, String> {
    let outcome = event_conn
        .wait_for_request_outcome(
            OutcomeCorrelation::Request(operation_request_id),
            RequestOutcomeExpectation::RoomOperation {
                request_id: operation_request_id,
                account_key,
                room_id,
                operation,
            },
            baseline_generation,
            tokio::time::Instant::now() + timeout,
        )
        .await
        .map_err(|error| invoke_error_from_request_outcome(context, error))?;
    let RequestOutcome::RoomOperation { generation, .. } = outcome else {
        return Err(format!("{context} returned an invalid outcome"));
    };
    Ok(generation)
}

pub(super) fn build_update_space_member_role_command(
    request_id: koushi_protocol::RequestId,
    space_id: String,
    user_id: String,
    generation: u64,
    expected_power_levels_revision: Option<String>,
    expected_power_level: i64,
    power_level: i64,
    confirmed: bool,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::UpdateSpaceMemberRole {
        request_id,
        space_id,
        user_id,
        generation,
        expected_power_levels_revision,
        expected_power_level,
        power_level,
        confirmed,
    })
}

pub(super) fn build_leave_room_command(
    request_id: koushi_protocol::RequestId,
    room_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::LeaveRoom {
        request_id,
        room_id,
    })
}

pub(super) fn build_forget_room_command(
    request_id: koushi_protocol::RequestId,
    room_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::ForgetRoom {
        request_id,
        room_id,
    })
}

pub(super) fn build_set_room_tag_command(
    request_id: koushi_protocol::RequestId,
    room_id: String,
    tag: RoomTagKind,
    order: Option<f64>,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::SetTag {
        request_id,
        room_id,
        tag,
        order,
    })
}

pub(super) fn build_remove_room_tag_command(
    request_id: koushi_protocol::RequestId,
    room_id: String,
    tag: RoomTagKind,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::RemoveTag {
        request_id,
        room_id,
        tag,
    })
}

pub(super) fn build_pin_event_command(
    request_id: koushi_protocol::RequestId,
    room_id: String,
    event_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::PinEvent {
        request_id,
        room_id,
        event_id,
    })
}

pub(super) fn build_unpin_event_command(
    request_id: koushi_protocol::RequestId,
    room_id: String,
    event_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::UnpinEvent {
        request_id,
        room_id,
        event_id,
    })
}

pub(super) fn build_refresh_pinned_events_command(
    request_id: koushi_protocol::RequestId,
    room_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::RefreshPinnedEvents {
        request_id,
        room_id,
    })
}

pub(super) fn build_load_room_settings_command(
    request_id: koushi_protocol::RequestId,
    room_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::LoadRoomSettings {
        request_id,
        room_id,
    })
}

pub(super) fn build_load_space_members_command(
    request_id: koushi_protocol::RequestId,
    space_id: String,
    generation: u64,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::LoadSpaceMembers {
        request_id,
        space_id,
        generation,
    })
}

pub(super) fn build_repair_room_timeline_command(
    request_id: koushi_protocol::RequestId,
    room_id: String,
) -> CoreCommand {
    CoreCommand::App(AppCommand::RepairRoomTimeline {
        request_id,
        room_id,
    })
}

pub(super) fn build_force_rotate_outbound_session_command(
    request_id: RequestId,
    room_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::ForceRotateOutboundSession {
        request_id,
        room_id,
    })
}

pub(super) fn build_update_room_setting_command(
    request_id: koushi_protocol::RequestId,
    room_id: String,
    change: RoomSettingChange,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::UpdateRoomSetting {
        request_id,
        room_id,
        change,
    })
}

pub(super) fn build_moderate_room_member_command(
    request_id: koushi_protocol::RequestId,
    room_id: String,
    target_user_id: String,
    action: RoomModerationAction,
    reason: Option<String>,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::ModerateRoomMember {
        request_id,
        room_id,
        target_user_id,
        action,
        reason,
    })
}

pub(super) fn build_update_room_member_role_command(
    request_id: koushi_protocol::RequestId,
    room_id: String,
    target_user_id: String,
    power_level: i64,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::UpdateRoomMemberRole {
        request_id,
        room_id,
        target_user_id,
        power_level,
    })
}

pub(super) const ROOM_OPERATION_EVENT_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(60);

const CREATE_EVENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

#[cfg(test)]
mod tests;
