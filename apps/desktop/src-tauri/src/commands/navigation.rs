use super::*;

#[tauri::command]
pub async fn select_space(
    space_id: Option<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let started = std::time::Instant::now();
    let requested_space_id = space_id.clone();
    let request_id = next_request_id(state.inner()).await;
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "desktop.space.transition", "submit")
            .field(DiagnosticField::request_id(
                "request_id",
                request_id.connection_id.0,
                request_id.sequence,
            ))
            .field(DiagnosticField::boolean(
                "target_present",
                requested_space_id.is_some(),
            )),
    );
    submit_core_command(
        state.inner(),
        build_select_space_command(request_id, space_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    let snapshot = current_snapshot(state.inner()).await?;
    record(
        DiagnosticEvent::new(
            DiagnosticLevel::Debug,
            "desktop.space.transition",
            "snapshot",
        )
        .field(DiagnosticField::request_id(
            "request_id",
            request_id.connection_id.0,
            request_id.sequence,
        ))
        .field(DiagnosticField::milliseconds(
            "elapsed_ms",
            started.elapsed().as_millis(),
        ))
        .field(DiagnosticField::boolean(
            "active_space_selected",
            snapshot.state.ui.navigation.active_space_id.as_deref()
                == requested_space_id.as_deref(),
        ))
        .field(DiagnosticField::boolean(
            "active_room_present",
            snapshot.state.ui.navigation.active_room_id.is_some(),
        )),
    );
    Ok(snapshot)
}

#[tauri::command]
pub async fn reorder_spaces(
    space_ids: Vec<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_reorder_spaces_command(request_id, space_ids),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn select_room(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let selected_room_id = room_id.clone();
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    trace_tauri_timeline_command("submit", "select_room", request_id);
    event_conn
        .command(build_select_room_command(request_id, room_id))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_selected_room(
        &mut event_conn,
        request_id,
        &selected_room_id,
        SELECT_ROOM_EVENT_TIMEOUT,
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn open_activity_event(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    open_anchored_timeline(room_id, event_id, app, state).await
}

#[tauri::command]
pub async fn select_search_result(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    open_anchored_timeline(room_id, event_id, app, state).await
}

async fn open_anchored_timeline(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let selected_room_id = room_id.clone();
    let mut event_conn = state.runtime.attach();

    let close_request_id = event_conn.next_request_id();
    event_conn
        .command(CoreCommand::App(AppCommand::CloseFocusedContext {
            request_id: close_request_id,
        }))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_focused_context_closed(
        &mut event_conn,
        close_request_id,
        FOCUSED_CONTEXT_EVENT_TIMEOUT,
    )
    .await?;

    let select_request_id = event_conn.next_request_id();
    event_conn
        .command(build_select_room_command(
            select_request_id,
            room_id.clone(),
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_selected_room(
        &mut event_conn,
        select_request_id,
        &selected_room_id,
        SELECT_ROOM_EVENT_TIMEOUT,
    )
    .await?;

    let open_request_id = event_conn.next_request_id();
    event_conn
        .command(CoreCommand::App(AppCommand::OpenAnchoredTimeline {
            request_id: open_request_id,
            room_id: room_id.clone(),
            event_id: event_id.clone(),
        }))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_main_timeline_anchor(
        &mut event_conn,
        open_request_id,
        &room_id,
        &event_id,
        FOCUSED_CONTEXT_EVENT_TIMEOUT,
    )
    .await?;

    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn acknowledge_timeline_projection(
    projection_request_id: RequestId,
    key: TimelineKey,
    generation: TimelineGeneration,
    state: State<'_, CoreRuntimeState>,
) -> Result<(), String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::App(AppCommand::AcknowledgeTimelineProjection {
            request_id,
            projection_request_id,
            key,
            generation,
        }),
    )
    .await
}

#[tauri::command]
pub async fn acknowledge_timeline_batch_rendered(
    key: TimelineKey,
    actor_generation: u64,
    timeline_generation: TimelineGeneration,
    repair_generation: u64,
    batch_id: TimelineBatchId,
    state: State<'_, CoreRuntimeState>,
) -> Result<(), String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::App(AppCommand::AcknowledgeTimelineBatchRendered {
            request_id,
            key,
            actor_generation,
            timeline_generation,
            repair_generation,
            batch_id,
        }),
    )
    .await
}

#[tauri::command]
pub async fn close_focused_context(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(CoreCommand::App(AppCommand::CloseFocusedContext {
            request_id,
        }))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_focused_context_closed(&mut event_conn, request_id, FOCUSED_CONTEXT_EVENT_TIMEOUT)
        .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn open_timeline_at_timestamp(
    room_id: String,
    timestamp_ms: u64,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let focused_room_id = room_id.clone();
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_open_timeline_at_timestamp_command(
            request_id,
            room_id,
            timestamp_ms,
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_focused_context(
        &mut event_conn,
        request_id,
        &focused_room_id,
        FOCUSED_CONTEXT_EVENT_TIMEOUT,
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn update_navigation_scroll_anchor(
    room_id: String,
    anchor: koushi_state::TimelineScrollAnchor,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<(), String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_update_navigation_scroll_anchor_command(request_id, room_id, anchor),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(())
}

#[tauri::command]
pub async fn observe_timeline_viewport(
    room_id: String,
    first_visible_event_id: Option<String>,
    last_visible_event_id: Option<String>,
    visible_gap_ids: Vec<TimelineGapId>,
    at_bottom: bool,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<(), String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_observe_timeline_viewport_command(
            request_id,
            account_key,
            room_id,
            first_visible_event_id,
            last_visible_event_id,
            visible_gap_ids,
            at_bottom,
        ),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(())
}
