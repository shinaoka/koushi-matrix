use super::room::{
    ROOM_OPERATION_EVENT_TIMEOUT, build_refresh_pinned_events_command, wait_for_room_operation,
};
use super::timeline::{
    build_observe_timeline_viewport_command, build_open_timeline_at_timestamp_command,
    build_update_navigation_scroll_anchor_command, trace_tauri_timeline_command,
};
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
    let refresh_request_id = event_conn.next_request_id();
    event_conn
        .command(build_refresh_pinned_events_command(
            refresh_request_id,
            selected_room_id.clone(),
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_room_operation(
        &mut event_conn,
        refresh_request_id,
        ROOM_OPERATION_EVENT_TIMEOUT,
        |event, _| {
            matches!(
                event,
                RoomEvent::PinnedEventsUpdated {
                    room_id: updated_room_id,
                    ..
                } if updated_room_id == &selected_room_id
            )
        },
        "pinned messages refresh did not complete",
        "pinned messages refresh failed",
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
    open_anchored_timeline(room_id, event_id, app, state, true).await
}

#[tauri::command]
pub async fn open_pinned_event(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    open_anchored_timeline(room_id, event_id, app, state, false).await
}

#[tauri::command]
pub async fn select_search_result(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    open_anchored_timeline(room_id, event_id, app, state, true).await
}

async fn open_anchored_timeline(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
    allow_live_fallback: bool,
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
            allow_live_fallback,
        }))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_main_timeline_anchor(
        &mut event_conn,
        open_request_id,
        &room_id,
        &event_id,
        allow_live_fallback,
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
    item_count: u64,
    target_present: bool,
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
            item_count,
            target_present,
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
    thread_root_event_id: Option<String>,
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
            thread_root_event_id,
        ),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(())
}

fn record_select_trace(
    stage: &'static str,
    outcome: &'static str,
    events: u32,
    state_changed: u32,
    state_delta: u32,
    active: &'static str,
) {
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "desktop.select", stage)
            .field(DiagnosticField::count("events", events.into()))
            .field(DiagnosticField::count(
                "state_changed",
                state_changed.into(),
            ))
            .field(DiagnosticField::count("state_delta", state_delta.into()))
            .field(DiagnosticField::token("outcome", outcome))
            .field(DiagnosticField::token("active", active)),
    );
}

fn record_select_intent_trace(
    stage: &'static str,
    outcome: &IntentOutcome,
    events: u32,
    state_changed: u32,
    state_delta: u32,
) {
    let (outcome_token, active) = match outcome {
        IntentOutcome::Committed => ("committed", "selected"),
        IntentOutcome::BenignNoOp(IntentNoOpReason::AlreadyActive) => {
            ("already_active", "selected")
        }
        IntentOutcome::BenignNoOp(IntentNoOpReason::TimelineTargetMissing) => {
            ("timeline_target_missing", "selected")
        }
        IntentOutcome::BenignNoOp(IntentNoOpReason::RoomNotInState) => {
            ("room_not_in_state", "unknown")
        }
        IntentOutcome::BenignNoOp(IntentNoOpReason::SessionNotReady) => {
            ("session_not_ready", "unknown")
        }
        IntentOutcome::FailedNoOp(IntentNoOpReason::RoomNotInState) => {
            ("room_not_in_state", "unknown")
        }
        IntentOutcome::FailedNoOp(IntentNoOpReason::SessionNotReady) => {
            ("session_not_ready", "unknown")
        }
        IntentOutcome::FailedNoOp(IntentNoOpReason::AlreadyActive) => {
            ("already_active", "selected")
        }
        IntentOutcome::FailedNoOp(IntentNoOpReason::TimelineTargetMissing) => {
            ("timeline_target_missing", "unknown")
        }
    };
    record_select_trace(
        stage,
        outcome_token,
        events,
        state_changed,
        state_delta,
        active,
    );
}

pub(super) trait SelectEventSource {
    fn snapshot(&self) -> koushi_state::AppState;
    fn recv_event(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Result<CoreEvent, EventStreamLag>> + Send + '_>>;
}

impl SelectEventSource for CoreConnection {
    fn snapshot(&self) -> koushi_state::AppState {
        CoreConnection::snapshot(self)
    }

    fn recv_event(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Result<CoreEvent, EventStreamLag>> + Send + '_>> {
        Box::pin(CoreConnection::recv_event(self))
    }
}

fn select_active_room_trace_label(
    snapshot: &koushi_state::AppState,
    selected_room_id: &str,
) -> &'static str {
    match snapshot.navigation.active_room_id.as_deref() {
        None => "none",
        Some(id) if id == selected_room_id => "match",
        Some(_) => "other",
    }
}

fn snapshot_has_focused_context(snapshot: &koushi_state::AppState, room_id: &str) -> bool {
    match &snapshot.focused_context {
        FocusedContextState::Opening {
            room_id: focused_room_id,
            ..
        }
        | FocusedContextState::Open {
            room_id: focused_room_id,
            ..
        } => focused_room_id == room_id,
        FocusedContextState::Closed => false,
    }
}

fn snapshot_has_no_focused_context(snapshot: &koushi_state::AppState) -> bool {
    snapshot.focused_context == FocusedContextState::Closed
        && snapshot.navigation.main_timeline_anchor.is_none()
}

fn snapshot_has_main_timeline_anchor(
    snapshot: &koushi_state::AppState,
    room_id: &str,
    event_id: &str,
) -> bool {
    snapshot.navigation.active_room_id.as_deref() == Some(room_id)
        && snapshot
            .navigation
            .main_timeline_anchor
            .as_ref()
            .is_some_and(|anchor| anchor.event_id == event_id)
}

fn snapshot_has_live_main_timeline(snapshot: &koushi_state::AppState, room_id: &str) -> bool {
    snapshot.navigation.active_room_id.as_deref() == Some(room_id)
        && snapshot.focused_context == FocusedContextState::Closed
        && snapshot.navigation.main_timeline_anchor.is_none()
}

fn snapshot_matches_main_timeline_settlement(
    snapshot: &koushi_state::AppState,
    room_id: &str,
    event_id: &str,
    settlement: Option<MainTimelineSettlement>,
) -> bool {
    match settlement {
        Some(MainTimelineSettlement::Anchor) | None => {
            snapshot_has_main_timeline_anchor(snapshot, room_id, event_id)
        }
        Some(MainTimelineSettlement::LiveFallback) => {
            snapshot_has_live_main_timeline(snapshot, room_id)
        }
    }
}

async fn wait_for_focused_context_closed(
    event_conn: &mut CoreConnection,
    request_id: RequestId,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        if snapshot_has_no_focused_context(&event_conn.snapshot()) {
            return Ok(());
        }

        let event = tokio::time::timeout_at(deadline, event_conn.recv_event())
            .await
            .map_err(|_| "focused context did not close".to_owned())?;
        match event {
            Ok(CoreEvent::StateChanged(snapshot)) if snapshot_has_no_focused_context(&snapshot) => {
                return Ok(());
            }
            Ok(CoreEvent::OperationFailed {
                request_id: failed_request_id,
                failure,
            }) if failed_request_id == request_id => {
                return Err(invoke_error_from_core_failure(
                    "focused context close failed",
                    failure,
                ));
            }
            Ok(_) => {}
            Err(_) if snapshot_has_no_focused_context(&event_conn.snapshot()) => {
                return Ok(());
            }
            Err(_) => continue,
        }
    }
}

async fn wait_for_focused_context(
    event_conn: &mut CoreConnection,
    request_id: RequestId,
    room_id: &str,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        if snapshot_has_focused_context(&event_conn.snapshot(), room_id) {
            return Ok(());
        }

        let event = tokio::time::timeout_at(deadline, event_conn.recv_event())
            .await
            .map_err(|_| "focused context did not open".to_owned())?;
        match event {
            Ok(CoreEvent::StateChanged(snapshot))
                if snapshot_has_focused_context(&snapshot, room_id) =>
            {
                return Ok(());
            }
            Ok(CoreEvent::OperationFailed {
                request_id: failed_request_id,
                failure,
            }) if failed_request_id == request_id => {
                return Err(invoke_error_from_core_failure(
                    "focused context open failed",
                    failure,
                ));
            }
            Ok(_) => {}
            Err(_) if snapshot_has_focused_context(&event_conn.snapshot(), room_id) => {
                return Ok(());
            }
            Err(_) => continue,
        }
    }
}

async fn wait_for_main_timeline_anchor(
    event_conn: &mut CoreConnection,
    request_id: RequestId,
    room_id: &str,
    event_id: &str,
    allow_live_fallback: bool,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut settlement = None;

    loop {
        let current = event_conn.snapshot();
        if snapshot_matches_main_timeline_settlement(&current, room_id, event_id, settlement) {
            return Ok(());
        }

        let event = tokio::time::timeout_at(deadline, event_conn.recv_event())
            .await
            .map_err(|_| "main timeline anchor did not open".to_owned())?;
        match event {
            Ok(CoreEvent::StateChanged(snapshot)) => {
                if snapshot_matches_main_timeline_settlement(
                    &snapshot, room_id, event_id, settlement,
                ) {
                    return Ok(());
                }
            }
            Ok(CoreEvent::IntentLifecycle {
                request_id: settled_request_id,
                outcome: IntentOutcome::Committed,
            }) if settled_request_id == request_id => {
                settlement = Some(MainTimelineSettlement::Anchor);
            }
            Ok(CoreEvent::IntentLifecycle {
                request_id: settled_request_id,
                outcome: IntentOutcome::BenignNoOp(IntentNoOpReason::TimelineTargetMissing),
            }) if settled_request_id == request_id => {
                if allow_live_fallback {
                    settlement = Some(MainTimelineSettlement::LiveFallback);
                } else {
                    return Err("pinned event is not available in the timeline".to_owned());
                }
            }
            Ok(CoreEvent::IntentLifecycle {
                request_id: settled_request_id,
                outcome: IntentOutcome::FailedNoOp(_),
            }) if settled_request_id == request_id => {
                return Err("main timeline anchor open failed".to_owned());
            }
            Ok(CoreEvent::OperationFailed {
                request_id: failed_request_id,
                failure,
            }) if failed_request_id == request_id => {
                return Err(invoke_error_from_core_failure(
                    "main timeline anchor open failed",
                    failure,
                ));
            }
            Ok(_) => {}
            Err(_) => {
                let current = event_conn.snapshot();
                if snapshot_matches_main_timeline_settlement(
                    &current, room_id, event_id, settlement,
                ) {
                    return Ok(());
                }
            }
        }
    }
}

pub(super) async fn wait_for_selected_room<S: SelectEventSource + ?Sized>(
    event_conn: &mut S,
    select_request_id: RequestId,
    selected_room_id: &str,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut events: u32 = 0;
    let mut state_changed: u32 = 0;
    let mut state_delta: u32 = 0;

    loop {
        if snapshot_has_active_room(&event_conn.snapshot(), selected_room_id) {
            record_select_trace(
                "ok_watch",
                "ok_watch",
                events,
                state_changed,
                state_delta,
                "selected",
            );
            return Ok(());
        }

        let event = match tokio::time::timeout_at(deadline, event_conn.recv_event()).await {
            Ok(event) => event,
            Err(_) => {
                let active =
                    select_active_room_trace_label(&event_conn.snapshot(), selected_room_id);
                record_select_trace(
                    "timeout",
                    "timeout",
                    events,
                    state_changed,
                    state_delta,
                    active,
                );
                return Err("room selection did not complete".to_owned());
            }
        };
        events += 1;
        match event {
            Ok(CoreEvent::StateChanged(snapshot)) => {
                state_changed += 1;
                if snapshot_has_active_room(&snapshot, selected_room_id) {
                    record_select_trace(
                        "ok_statechanged",
                        "ok_statechanged",
                        events,
                        state_changed,
                        state_delta,
                        "selected",
                    );
                    return Ok(());
                }
            }
            Ok(CoreEvent::StateDelta(_)) => {
                state_delta += 1;
            }
            Ok(CoreEvent::OperationFailed {
                request_id,
                failure,
            }) if request_id == select_request_id => {
                record_select_trace(
                    "op_failed",
                    "op_failed",
                    events,
                    state_changed,
                    state_delta,
                    "unknown",
                );
                return Err(invoke_error_from_core_failure(
                    "room selection failed",
                    failure,
                ));
            }
            // Telemetry-lane fast path: IntentLifecycle lets us fail fast with
            // a specific reason instead of waiting the full 10s timeout.
            Ok(CoreEvent::IntentLifecycle {
                request_id,
                outcome,
            }) if request_id == select_request_id => {
                match outcome {
                    IntentOutcome::Committed | IntentOutcome::BenignNoOp(_) => {
                        record_select_intent_trace(
                            "ok_intent",
                            &outcome,
                            events,
                            state_changed,
                            state_delta,
                        );
                        return Ok(());
                    }
                    IntentOutcome::FailedNoOp(IntentNoOpReason::RoomNotInState) => {
                        record_select_trace(
                            "failed_not_in_state",
                            "failed_not_in_state",
                            events,
                            state_changed,
                            state_delta,
                            "unknown",
                        );
                        return Err("room not yet loaded".to_owned());
                    }
                    IntentOutcome::FailedNoOp(IntentNoOpReason::SessionNotReady) => {
                        record_select_trace(
                            "failed_session_not_ready",
                            "failed_session_not_ready",
                            events,
                            state_changed,
                            state_delta,
                            "unknown",
                        );
                        return Err("session not ready".to_owned());
                    }
                    IntentOutcome::FailedNoOp(IntentNoOpReason::AlreadyActive) => {
                        // AlreadyActive is benign; this arm is unreachable per
                        // the classification logic but handle it defensively.
                        record_select_trace(
                            "ok_already_active",
                            "already_active",
                            events,
                            state_changed,
                            state_delta,
                            "selected",
                        );
                        return Ok(());
                    }
                    IntentOutcome::FailedNoOp(IntentNoOpReason::TimelineTargetMissing) => {
                        record_select_trace(
                            "failed_timeline_target_missing",
                            "timeline_target_missing",
                            events,
                            state_changed,
                            state_delta,
                            "unknown",
                        );
                        return Err("timeline target not available".to_owned());
                    }
                }
            }
            Ok(_) => {}
            Err(_) if snapshot_has_active_room(&event_conn.snapshot(), selected_room_id) => {
                record_select_trace(
                    "ok_after_lag",
                    "ok_after_lag",
                    events,
                    state_changed,
                    state_delta,
                    "selected",
                );
                return Ok(());
            }
            Err(_) => {
                record_select_trace("lag", "lag", events, state_changed, state_delta, "unknown");
                continue;
            }
        }
    }
}

fn snapshot_has_active_room(snapshot: &koushi_state::AppState, room_id: &str) -> bool {
    snapshot.navigation.active_room_id.as_deref() == Some(room_id)
}

pub(super) fn build_select_space_command(
    request_id: koushi_core::RequestId,
    space_id: Option<String>,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::SelectSpace {
        request_id,
        space_id,
    })
}

pub(super) fn build_reorder_spaces_command(
    request_id: koushi_core::RequestId,
    space_ids: Vec<String>,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::ReorderSpaces {
        request_id,
        space_ids,
    })
}

pub(super) fn build_select_room_command(
    request_id: koushi_core::RequestId,
    room_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::SelectRoom {
        request_id,
        room_id,
    })
}

pub(super) const SELECT_ROOM_EVENT_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(10);

const FOCUSED_CONTEXT_EVENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

#[derive(Clone, Copy)]
enum MainTimelineSettlement {
    Anchor,
    LiveFallback,
}

#[cfg(test)]
fn commands_source() -> String {
    crate::commands::contracts::production_source()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::contracts::fake_request_id;

    #[test]
    fn select_room_waits_for_core_selection_without_resubscribing_timeline() {
        let source = commands_source();
        let fn_name = concat!("pub async fn select", "_room");
        let select_token = concat!("build_select", "_room_command");
        let attach_token = concat!("state.runtime.", "attach");
        let wait_token = concat!("wait_for_selected", "_room");
        let subscribe_token = concat!("build_subscribe", "_timeline_command");
        let account_key_token = concat!("account_key", "_from_snapshot");
        let timeout_token = concat!("SELECT_ROOM", "_EVENT_TIMEOUT");
        let fn_offset = source
            .find(fn_name)
            .expect("select_room command should exist");
        let rest = &source[fn_offset..];
        let end = rest
            .find("pub async fn open_activity_event")
            .expect("next command should exist");
        let select_room_source = &rest[..end];
        let attach_offset = select_room_source
            .find(attach_token)
            .expect("select_room should attach an event connection before selecting");
        let select_offset = select_room_source
            .find(select_token)
            .expect("select_room should submit room selection");
        let wait_offset = select_room_source
            .find(wait_token)
            .expect("select_room should wait for selected-room state");

        assert!(
            attach_offset < select_offset,
            "event connection should be attached before room selection"
        );
        assert!(
            select_offset < wait_offset,
            "room selection state should be observed after submitting selection"
        );
        assert!(
            !select_room_source.contains(subscribe_token),
            "room selection reducers already emit the canonical timeline subscription"
        );
        assert!(
            !select_room_source.contains(account_key_token),
            "select_room should not derive an account key just to duplicate timeline subscription"
        );
        assert!(
            select_room_source.contains(timeout_token),
            "selected-room wait should be bounded"
        );
    }

    #[test]
    fn room_transition_and_backfill_commands_emit_submit_trace_tokens() {
        let source = commands_source();
        assert!(
            source.contains("fn trace_tauri_timeline_command"),
            "Tauri command layer must expose a private-data-free timeline trace helper"
        );
        assert!(
            source.contains("desktop.timeline"),
            "Tauri command traces must preserve the desktop.timeline source token"
        );
        let select_start = source
            .find("pub async fn select_room")
            .expect("select_room command should exist");
        let paginate_start = source
            .find("pub async fn paginate_timeline_backwards")
            .expect("paginate command should exist");
        let load_link_previews_start = source
            .find("pub async fn load_link_previews")
            .expect("load_link_previews command should exist");
        let select_source = &source[select_start..paginate_start];
        let paginate_source = &source[paginate_start..load_link_previews_start];
        let load_link_previews_source = &source[load_link_previews_start..];
        assert!(
            select_source.contains("trace_tauri_timeline_command(\"submit\", \"select_room\""),
            "select_room should trace the submitted room transition command"
        );
        assert!(
            paginate_source
                .contains("trace_tauri_timeline_command(\"submit\", \"paginate_backwards\""),
            "paginate_timeline_backwards should trace submitted backfill requests"
        );
        assert!(
            load_link_previews_source
                .contains("trace_tauri_timeline_command(\"submit\", \"load_link_previews\""),
            "load_link_previews should trace submitted preview expansion requests"
        );
    }

    #[test]
    fn select_search_result_selects_room_then_enters_anchored_timeline_without_room_resubscribe() {
        let source = commands_source();
        let fn_name = "pub async fn select_search_result";
        let helper_name = "async fn open_anchored_timeline";
        let close_token = "CloseFocusedContext";
        let open_token = "OpenAnchoredTimeline";
        let select_room_token = concat!("build_select", "_room_command");
        let subscribe_room_token = "build_subscribe_timeline_command";

        let fn_offset = source
            .find(fn_name)
            .expect("select_search_result command should exist");
        let helper_offset = source
            .find(helper_name)
            .expect("shared helper should exist");
        let helper_rest = &source[helper_offset..];
        let end = helper_rest
            .find("pub async fn acknowledge_timeline_projection")
            .expect("ack command should follow helper");
        let select_source = &helper_rest[..end];

        assert!(
            source[fn_offset..helper_offset].contains("open_anchored_timeline"),
            "select_search_result should use the shared anchored navigation path"
        );
        assert!(
            select_source.contains(close_token),
            "select_search_result should close the previous focused context"
        );
        assert!(
            select_source.contains(open_token),
            "select_search_result should subscribe the focused event timeline"
        );
        assert!(!select_source.contains("EnterAnchoredTimeline"));
        assert!(!select_source.contains("wait_for_focused_timeline_event"));
        assert!(
            select_source.contains(select_room_token),
            "select_search_result should select the room before opening the focused context"
        );
        assert!(
            !select_source.contains(subscribe_room_token),
            "select_search_result should rely on room selection reducers for room timeline subscription"
        );
        assert!(
            select_source.contains("wait_for_selected_room"),
            "select_search_result should wait for the selected room state"
        );
        assert!(
            select_source.contains("state.runtime.attach"),
            "select_search_result should attach a fresh core connection"
        );

        let select_offset = select_source
            .find(select_room_token)
            .expect("search result command should select the room");
        let wait_offset = select_source
            .find("wait_for_selected_room")
            .expect("search result command should wait for the selected room");
        let open_offset = select_source
            .find(open_token)
            .expect("search result command should open focused context");
        let anchor_offset = select_source
            .find("wait_for_main_timeline_anchor")
            .expect("search result command should wait for the acknowledged anchor");
        assert!(
            select_offset < wait_offset && wait_offset < open_offset && open_offset < anchor_offset,
            "focused event timeline should open and become the main anchored timeline only after the selected room state is observed"
        );
    }

    #[test]
    fn close_focused_context_command_routes_to_app_close_focused_context() {
        let source = commands_source();
        let fn_name = concat!("pub async fn close", "_focused_context");
        let command_token = concat!("Close", "FocusedContext");
        let submit_token = "submit_core_command";
        let title_token = "update_qa_window_title_from_state";
        let snapshot_token = "current_snapshot";

        let fn_offset = source
            .find(fn_name)
            .expect("close_focused_context command should exist");
        let rest = &source[fn_offset..];
        let end = rest
            .find("pub async fn paginate_timeline_backwards")
            .expect("next command should exist");
        let close_source = &rest[..end];

        assert!(
            close_source.contains(command_token),
            "close_focused_context should route through AppCommand::CloseFocusedContext"
        );
        assert!(
            close_source.contains(submit_token),
            "close_focused_context should submit the core command"
        );
        assert!(
            close_source.contains(title_token),
            "close_focused_context should refresh the QA title after state changes"
        );
        assert!(
            close_source.contains(snapshot_token),
            "close_focused_context should return the current snapshot"
        );
    }

    #[test]
    fn close_focused_context_command_waits_until_main_timeline_is_live() {
        let source = commands_source();
        let fn_name = concat!("pub async fn close", "_focused_context");
        let command_token = concat!("Close", "FocusedContext");

        let fn_offset = source
            .find(fn_name)
            .expect("close_focused_context command should exist");
        let rest = &source[fn_offset..];
        let end = rest
            .find("pub async fn paginate_timeline_backwards")
            .expect("next command should exist");
        let command_source = &rest[..end];

        let close_offset = command_source
            .find(command_token)
            .expect("close_focused_context should submit the close command");
        let wait_offset = command_source
            .find("wait_for_focused_context_closed")
            .expect("close_focused_context must wait before returning its snapshot");
        let snapshot_offset = command_source
            .find("current_snapshot")
            .expect("close_focused_context should return a snapshot");

        assert!(
            close_offset < wait_offset && wait_offset < snapshot_offset,
            "close_focused_context must return only after focused_context is closed and main_timeline_anchor is cleared"
        );
    }

    #[test]
    fn open_timeline_at_timestamp_command_routes_through_app_command() {
        let command = build_open_timeline_at_timestamp_command(
            fake_request_id(40),
            "!room:example.org".to_owned(),
            1_718_000_000_000,
        );

        match command {
            CoreCommand::App(AppCommand::OpenTimelineAtTimestamp {
                request_id,
                room_id,
                timestamp_ms,
            }) => {
                assert_eq!(request_id, fake_request_id(40));
                assert_eq!(room_id, "!room:example.org");
                assert_eq!(timestamp_ms, 1_718_000_000_000);
                let debug = format!(
                    "{:?}",
                    AppCommand::OpenTimelineAtTimestamp {
                        request_id,
                        room_id,
                        timestamp_ms,
                    }
                );
                assert!(!debug.contains("!room:example.org"), "{debug}");
                assert!(!debug.contains("1718000000000"), "{debug}");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn observe_timeline_viewport_command_routes_viewport_facts_only() {
        let account_key = AccountKey("@alice:example.org".to_owned());
        let command = build_observe_timeline_viewport_command(
            fake_request_id(41),
            account_key.clone(),
            "!room:example.org".to_owned(),
            Some("$first".to_owned()),
            Some("$last".to_owned()),
            vec![koushi_core::TimelineGapId {
                topology_revision: 7,
                ordinal: 2,
            }],
            false,
            None,
        );
        let debug = format!("{command:?}");
        assert!(!debug.contains("!room:example.org"), "{debug}");
        assert!(!debug.contains("$first"), "{debug}");
        assert!(!debug.contains("$last"), "{debug}");

        match command {
            CoreCommand::Timeline(TimelineCommand::ObserveViewport {
                request_id,
                key,
                observation,
            }) => {
                assert_eq!(request_id, fake_request_id(41));
                assert_eq!(key.account_key, account_key);
                assert_eq!(
                    key.kind,
                    koushi_core::TimelineKind::Room {
                        room_id: "!room:example.org".to_owned()
                    }
                );
                assert_eq!(
                    observation.first_visible_event_id.as_deref(),
                    Some("$first")
                );
                assert_eq!(observation.last_visible_event_id.as_deref(), Some("$last"));
                assert_eq!(
                    observation.visible_gap_ids,
                    vec![koushi_core::TimelineGapId {
                        topology_revision: 7,
                        ordinal: 2,
                    }]
                );
                assert!(!observation.at_bottom);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn observe_timeline_viewport_routes_thread_identity() {
        let command = build_observe_timeline_viewport_command(
            fake_request_id(43),
            AccountKey("@alice:example.org".to_owned()),
            "!room:example.org".to_owned(),
            Some("$reply".to_owned()),
            Some("$reply".to_owned()),
            Vec::new(),
            true,
            Some("$root".to_owned()),
        );
        let CoreCommand::Timeline(TimelineCommand::ObserveViewport { key, .. }) = command else {
            panic!("expected observe viewport command");
        };
        assert_eq!(
            key.kind,
            koushi_core::TimelineKind::Thread {
                room_id: "!room:example.org".to_owned(),
                root_event_id: "$root".to_owned(),
            }
        );
    }

    #[test]
    fn observe_timeline_viewport_parses_full_range_topology_revision() {
        let visible_gap_ids: Vec<koushi_core::TimelineGapId> =
            serde_json::from_value(serde_json::json!([{
                "topology_revision": "14695981039346656037",
                "ordinal": 0,
            }]))
            .expect("Tauri viewport gap ids parse from their JSON wire shape");

        let command = build_observe_timeline_viewport_command(
            fake_request_id(42),
            AccountKey("@alice:example.org".to_owned()),
            "!room:example.org".to_owned(),
            None,
            None,
            visible_gap_ids,
            false,
            None,
        );

        let CoreCommand::Timeline(TimelineCommand::ObserveViewport { observation, .. }) = command
        else {
            panic!("expected observe viewport command");
        };
        assert_eq!(
            observation.visible_gap_ids,
            vec![koushi_core::TimelineGapId {
                topology_revision: 14_695_981_039_346_656_037,
                ordinal: 0,
            }]
        );
    }

    #[test]
    fn wait_for_selected_room_observes_state_changed_failures_and_timeout() {
        let source = commands_source();
        let helper_name = concat!("async fn wait_for_selected", "_room");
        let helper_offset = source
            .find(helper_name)
            .expect("selected-room wait helper should exist");
        let rest = &source[helper_offset..];
        let end = rest
            .find("fn snapshot_has_active_room")
            .expect("active-room snapshot helper should follow selected-room wait");
        let helper_source = &rest[..end];

        assert!(helper_source.contains("timeout_at"));
        assert!(helper_source.contains("CoreEvent::StateChanged"));
        assert!(helper_source.contains(concat!("Operation", "Failed")));
        assert!(helper_source.contains(concat!("snapshot_has_active", "_room")));
    }

    #[test]
    fn select_diagnostics_keep_intent_outcomes_distinct() {
        super::record_select_intent_trace("ok_intent", &IntentOutcome::Committed, 0, 0, 0);
        super::record_select_intent_trace(
            "ok_intent",
            &IntentOutcome::BenignNoOp(IntentNoOpReason::AlreadyActive),
            0,
            0,
            0,
        );

        let records = koushi_diagnostics::snapshot().records;
        let select_records = records
            .iter()
            .filter(|record| record.event.source == "desktop.select")
            .rev()
            .take(2)
            .collect::<Vec<_>>();
        assert_eq!(select_records.len(), 2);
        let pairs = select_records
            .iter()
            .map(|record| {
                let fields = &record.event.fields;
                let outcome = fields
                    .iter()
                    .find(|field| field.key == "outcome")
                    .expect("intent record should include outcome");
                let active = fields
                    .iter()
                    .find(|field| field.key == "active")
                    .expect("intent record should include active");
                (outcome.value.clone(), active.value.clone())
            })
            .collect::<Vec<_>>();
        assert!(pairs.iter().any(|(outcome, active)| {
            matches!(
                (outcome, active),
                (
                    koushi_diagnostics::DiagnosticValue::Token("committed"),
                    koushi_diagnostics::DiagnosticValue::Token("selected")
                )
            )
        }));
        assert!(pairs.iter().any(|(outcome, active)| {
            matches!(
                (outcome, active),
                (
                    koushi_diagnostics::DiagnosticValue::Token("already_active"),
                    koushi_diagnostics::DiagnosticValue::Token("selected")
                )
            )
        }));
    }
}

#[cfg(test)]
mod issue551_moved_tests {
    use koushi_state::AppState;
    fn commands_source() -> String {
        crate::commands::contracts::production_source()
    }
    #[test]
    fn select_space_command_records_private_data_free_transition_trace() {
        let source = commands_source();
        let production_source = source
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("command production source should precede tests");
        let select_space_source = production_source
            .split("pub async fn select_space")
            .nth(1)
            .expect("select_space command should exist")
            .split("#[tauri::command]\npub async fn reorder_spaces")
            .next()
            .expect("reorder_spaces command should follow select_space");

        let submit_offset = select_space_source
            .find("\"desktop.space.transition\", \"submit\"")
            .expect("select_space should record submit trace");
        let command_offset = select_space_source
            .find("build_select_space_command")
            .expect("select_space should submit the SelectSpace command");
        let snapshot_offset = select_space_source
            .find("\"snapshot\"")
            .expect("select_space should record snapshot-return trace");

        assert!(submit_offset < command_offset);
        assert!(command_offset < snapshot_offset);
        assert!(select_space_source.contains("DiagnosticField::request_id"));
        assert!(select_space_source.contains("DiagnosticField::milliseconds"));
        assert!(select_space_source.contains("DiagnosticField::boolean"));
    }

    #[test]
    fn main_timeline_lifecycle_requires_the_matching_settled_snapshot() {
        let room_id = "!room:example.invalid";
        let event_id = "$event:example.invalid";
        let mut state = AppState::default();
        state.navigation.active_room_id = Some(room_id.to_owned());

        assert!(!super::snapshot_matches_main_timeline_settlement(
            &state,
            room_id,
            event_id,
            Some(super::MainTimelineSettlement::Anchor),
        ));
        state.navigation.main_timeline_anchor = Some(koushi_state::MainTimelineAnchor {
            event_id: event_id.to_owned(),
        });
        assert!(super::snapshot_matches_main_timeline_settlement(
            &state,
            room_id,
            event_id,
            Some(super::MainTimelineSettlement::Anchor),
        ));

        state.navigation.main_timeline_anchor = None;
        state.focused_context = koushi_state::FocusedContextState::Opening {
            room_id: room_id.to_owned(),
            event_id: event_id.to_owned(),
        };
        assert!(!super::snapshot_matches_main_timeline_settlement(
            &state,
            room_id,
            event_id,
            Some(super::MainTimelineSettlement::LiveFallback),
        ));
        state.focused_context = koushi_state::FocusedContextState::Closed;
        assert!(super::snapshot_matches_main_timeline_settlement(
            &state,
            room_id,
            event_id,
            Some(super::MainTimelineSettlement::LiveFallback),
        ));
    }
}
