use super::*;

#[tauri::command]
pub async fn open_invite_workflow(
    room_id: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_open_invite_workflow_command(request_id, room_id),
    )
    .await?;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn close_invite_workflow(
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_close_invite_workflow_command(request_id),
    )
    .await?;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn search_invite_targets(
    room_id: String,
    query: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_search_invite_targets_command(request_id, room_id, query),
    )
    .await?;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn select_invite_target(
    room_id: String,
    user_id: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_select_invite_target_command(request_id, room_id, user_id),
    )
    .await?;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn remove_invite_target(
    user_id: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_remove_invite_target_command(request_id, user_id),
    )
    .await?;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn select_room_list_filter(
    filter: RoomListFilter,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::App(AppCommand::SelectRoomListFilter { request_id, filter }),
    )
    .await?;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn mark_room_as_read(
    room_id: String,
    event_id: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::Room(RoomCommand::MarkRoomAsRead {
            request_id,
            room_id,
            event_id,
        }),
    )
    .await?;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn mark_room_as_unread(
    room_id: String,
    unread: bool,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::Room(RoomCommand::MarkRoomAsUnread {
            request_id,
            room_id,
            unread,
        }),
    )
    .await?;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn set_room_notification_mode(
    room_id: String,
    mode: RoomNotificationMode,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::Room(RoomCommand::SetRoomNotificationMode {
            request_id,
            room_id,
            mode,
        }),
    )
    .await?;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn leave_room(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(state.inner(), build_leave_room_command(request_id, room_id)).await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn forget_room(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_forget_room_command(request_id, room_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn set_room_tag(
    room_id: String,
    tag: RoomTagKind,
    order: Option<f64>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_set_room_tag_command(request_id, room_id, tag, order),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn remove_room_tag(
    room_id: String,
    tag: RoomTagKind,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_remove_room_tag_command(request_id, room_id, tag),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn pin_event(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_pin_event_command(request_id, room_id, event_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn unpin_event(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_unpin_event_command(request_id, room_id, event_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn load_room_settings(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_load_room_settings_command(request_id, room_id))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_room_operation(
        &mut event_conn,
        request_id,
        ROOM_OPERATION_EVENT_TIMEOUT,
        |event, expected_request_id| {
            matches!(
                event,
                RoomEvent::RoomSettingsLoaded { request_id, .. } if *request_id == expected_request_id
            )
        },
        "room settings load did not complete",
        "room settings load failed",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn reshare_room_key(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_reshare_room_key_command(request_id, room_id))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_room_operation(
        &mut event_conn,
        request_id,
        ROOM_OPERATION_EVENT_TIMEOUT,
        |event, expected_request_id| {
            matches!(
                event,
                RoomEvent::RoomKeyReshared { request_id, .. } if *request_id == expected_request_id
            )
        },
        "room key reshare did not complete",
        "room key reshare failed",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn update_room_setting(
    room_id: String,
    change: RoomSettingChange,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_update_room_setting_command(
            request_id, room_id, change,
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_room_operation(
        &mut event_conn,
        request_id,
        ROOM_OPERATION_EVENT_TIMEOUT,
        |event, expected_request_id| {
            matches!(
                event,
                RoomEvent::RoomSettingUpdated { request_id, .. } if *request_id == expected_request_id
            )
        },
        "room setting update did not complete",
        "room setting update failed",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn moderate_room_member(
    room_id: String,
    target_user_id: String,
    action: RoomModerationAction,
    reason: Option<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_moderate_room_member_command(
            request_id,
            room_id,
            target_user_id,
            action,
            optional_non_blank(reason),
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_room_operation(
        &mut event_conn,
        request_id,
        ROOM_OPERATION_EVENT_TIMEOUT,
        |event, expected_request_id| {
            matches!(
                event,
                RoomEvent::RoomMemberModerated { request_id, .. } if *request_id == expected_request_id
            )
        },
        "room member moderation did not complete",
        "room member moderation failed",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn update_room_member_role(
    room_id: String,
    target_user_id: String,
    power_level: i64,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_update_room_member_role_command(
            request_id,
            room_id,
            target_user_id,
            power_level,
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_room_operation(
        &mut event_conn,
        request_id,
        ROOM_OPERATION_EVENT_TIMEOUT,
        |event, expected_request_id| {
            matches!(
                event,
                RoomEvent::RoomMemberRoleUpdated { request_id, .. } if *request_id == expected_request_id
            )
        },
        "room member role update did not complete",
        "room member role update failed",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn create_room(
    options: koushi_core::CreateRoomOptions,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_create_room_command(request_id, options))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_room_created(&mut event_conn, request_id, CREATE_EVENT_TIMEOUT).await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn create_space(
    name: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_create_space_command(request_id, name))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_space_created(&mut event_conn, request_id, CREATE_EVENT_TIMEOUT).await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn set_space_child(
    space_id: String,
    child_room_id: String,
    via_server: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_set_space_child_command(request_id, space_id, child_room_id, via_server),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn join_room(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    let Some(command) = build_join_room_command(request_id, room_id) else {
        update_qa_window_title_from_state(&app, state.inner()).await;
        return current_snapshot(state.inner()).await;
    };

    event_conn
        .command(command)
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_room_joined(&mut event_conn, request_id, ROOM_OPERATION_EVENT_TIMEOUT).await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn accept_invite(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_accept_invite_command(request_id, room_id))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_room_operation(
        &mut event_conn,
        request_id,
        ROOM_OPERATION_EVENT_TIMEOUT,
        |event, expected_request_id| {
            matches!(
                event,
                RoomEvent::InviteAccepted { request_id, .. } if *request_id == expected_request_id
            )
        },
        "invite acceptance did not complete",
        "invite acceptance failed",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn decline_invite(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_decline_invite_command(request_id, room_id))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_room_operation(
        &mut event_conn,
        request_id,
        ROOM_OPERATION_EVENT_TIMEOUT,
        |event, expected_request_id| {
            matches!(
                event,
                RoomEvent::InviteDeclined { request_id, .. } if *request_id == expected_request_id
            )
        },
        "invite decline did not complete",
        "invite decline failed",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn start_direct_message(
    user_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_start_direct_message_command(request_id, user_id))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_room_operation(
        &mut event_conn,
        request_id,
        ROOM_OPERATION_EVENT_TIMEOUT,
        |event, expected_request_id| {
            matches!(
                event,
                RoomEvent::DirectMessageStarted { request_id, .. } if *request_id == expected_request_id
            )
        },
        "direct message start did not complete",
        "direct message start failed",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn invite_user(
    room_id: String,
    user_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_invite_user_command(request_id, room_id, user_id))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_room_operation(
        &mut event_conn,
        request_id,
        ROOM_OPERATION_EVENT_TIMEOUT,
        |event, expected_request_id| {
            matches!(
                event,
                RoomEvent::UserInvited { request_id, .. } if *request_id == expected_request_id
            )
        },
        "user invite did not complete",
        "user invite failed",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn invite_targets(
    room_id: String,
    user_ids: Vec<String>,
    scope: InviteScopeSelection,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_invite_targets_command(
            request_id, room_id, user_ids, scope,
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_invite_batch_completed(&mut event_conn, request_id).await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

async fn wait_for_invite_batch_completed(
    event_conn: &mut CoreConnection,
    operation_request_id: RequestId,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + ROOM_OPERATION_EVENT_TIMEOUT;
    loop {
        let event = tokio::time::timeout_at(deadline, event_conn.recv_event())
            .await
            .map_err(|_| "invite batch did not complete".to_owned())?;
        match event {
            Ok(CoreEvent::Room(RoomEvent::InviteBatchCompleted { request_id, .. }))
                if request_id == operation_request_id =>
            {
                return Ok(());
            }
            Ok(CoreEvent::OperationFailed { request_id, .. })
                if request_id == operation_request_id =>
            {
                return Err("invite batch failed".to_owned());
            }
            Ok(_) => {}
            Err(_) => return Err("room operation event stream lagged".to_owned()),
        }
    }
}
