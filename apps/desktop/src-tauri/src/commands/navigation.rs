use super::room::{
    ROOM_OPERATION_EVENT_TIMEOUT, build_refresh_pinned_events_command, wait_for_room_operation,
};
use super::timeline::{
    build_observe_timeline_viewport_command, build_open_timeline_at_timestamp_command,
    build_update_navigation_scroll_anchor_command,
};
use super::*;
use koushi_core::EventNavigationError;

#[tauri::command]
pub async fn update_navigation_preference(
    update: koushi_state::NavigationPreferenceUpdate,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_update_navigation_preference_command(request_id, update),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn select_space(
    space_id: Option<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
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
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_select_space_command(request_id, space_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    record(
        DiagnosticEvent::new(
            DiagnosticLevel::Debug,
            "desktop.space.transition",
            "admitted",
        )
        .field(DiagnosticField::request_id(
            "request_id",
            request_id.connection_id.0,
            request_id.sequence,
        ))
        .field(DiagnosticField::milliseconds(
            "elapsed_ms",
            started.elapsed().as_millis(),
        )),
    );
    Ok(admission)
}

#[tauri::command]
pub async fn reorder_spaces(
    space_ids: Vec<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_reorder_spaces_command(request_id, space_ids),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn select_room(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let selected_room_id = room_id.clone();
    let mut event_conn = state.runtime.attach();
    event_conn
        .select_room_and_wait(selected_room_id.clone(), SELECT_ROOM_EVENT_TIMEOUT)
        .await
        .map_err(invoke_error_from_select_room_error)?;
    let baseline = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline.state);
    let refresh_request_id = event_conn.next_request_id();
    event_conn
        .command(build_refresh_pinned_events_command(
            refresh_request_id,
            selected_room_id.clone(),
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    let snapshot = wait_for_room_operation(
        &mut event_conn,
        refresh_request_id,
        baseline.generation,
        account_key,
        selected_room_id,
        RoomOperationKind::PinnedEventsRefreshed,
        ROOM_OPERATION_EVENT_TIMEOUT,
        "pinned messages refresh",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(FrontendCommandSettlement::from_published_generation(
        snapshot.generation,
    ))
}

#[tauri::command]
pub async fn open_activity_event(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    navigate_to_event(
        room_id,
        event_id,
        koushi_state::EventNavigationSource::Activity,
        app,
        state,
    )
    .await
}

#[tauri::command]
pub async fn open_pinned_event(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    navigate_to_event(
        room_id,
        event_id,
        koushi_state::EventNavigationSource::Pinned,
        app,
        state,
    )
    .await
}

#[tauri::command]
pub async fn select_search_result(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    navigate_to_event(
        room_id,
        event_id,
        koushi_state::EventNavigationSource::Search,
        app,
        state,
    )
    .await
}

async fn navigate_to_event(
    room_id: String,
    event_id: String,
    source: koushi_state::EventNavigationSource,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let snapshot = event_conn
        .navigate_to_event_and_wait(
            room_id,
            event_id,
            source,
            event_navigation_policy(source),
            FOCUSED_CONTEXT_EVENT_TIMEOUT,
        )
        .await
        .map_err(invoke_error_from_event_navigation_error)?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(FrontendCommandSettlement::from_published_generation(
        snapshot.generation,
    ))
}

#[tauri::command]
pub async fn close_focused_context(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let baseline_snapshot = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline_snapshot.state);
    let room_id = baseline_snapshot.state.navigation.active_room_id.clone();
    let baseline_generation = baseline_snapshot.generation;
    let deadline = tokio::time::Instant::now() + FOCUSED_CONTEXT_EVENT_TIMEOUT;
    let request_id = event_conn.next_request_id();
    event_conn
        .command(CoreCommand::App(AppCommand::CloseFocusedContext {
            request_id,
        }))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    let snapshot = wait_for_focused_context_closed(
        &mut event_conn,
        request_id,
        account_key,
        room_id,
        baseline_generation,
        deadline,
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(FrontendCommandSettlement::from_published_generation(
        snapshot.generation,
    ))
}

#[tauri::command]
pub async fn open_timeline_at_timestamp(
    room_id: String,
    timestamp_ms: u64,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let baseline_snapshot = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline_snapshot.state);
    let baseline_generation = baseline_snapshot.generation;
    let deadline = tokio::time::Instant::now() + FOCUSED_CONTEXT_EVENT_TIMEOUT;
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_open_timeline_at_timestamp_command(
            request_id,
            room_id.clone(),
            timestamp_ms,
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    let snapshot = wait_for_focused_context(
        &mut event_conn,
        request_id,
        account_key,
        room_id,
        None,
        baseline_generation,
        deadline,
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(FrontendCommandSettlement::from_published_generation(
        snapshot.generation,
    ))
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

fn event_navigation_policy(
    source: koushi_state::EventNavigationSource,
) -> koushi_core::EventNavigationMissingTargetPolicy {
    match source {
        koushi_state::EventNavigationSource::Activity
        | koushi_state::EventNavigationSource::Search => {
            koushi_core::EventNavigationMissingTargetPolicy::LiveFallback
        }
        koushi_state::EventNavigationSource::Pinned => {
            koushi_core::EventNavigationMissingTargetPolicy::Fail
        }
    }
}

fn invoke_error_from_event_navigation_error(error: koushi_core::EventNavigationError) -> String {
    match error {
        EventNavigationError::CommandSubmit(_) => "event navigation command submit failed".to_owned(),
        EventNavigationError::Rejected => "event navigation rejected".to_owned(),
        EventNavigationError::Failed(kind) => match kind {
            koushi_state::EventNavigationFailureKind::TargetMissing => {
                "event navigation target unavailable".to_owned()
            }
            koushi_state::EventNavigationFailureKind::RoomUnavailable => {
                "event navigation room unavailable".to_owned()
            }
            koushi_state::EventNavigationFailureKind::SessionUnavailable => {
                "event navigation session unavailable".to_owned()
            }
            koushi_state::EventNavigationFailureKind::Timeline => {
                "event navigation failed".to_owned()
            }
        },
        EventNavigationError::EventStreamClosed | EventNavigationError::Timeout => {
            "event navigation did not complete".to_owned()
        }
    }
}

pub(super) fn invoke_error_from_select_room_error(error: koushi_core::SelectRoomError) -> String {
    match error {
        koushi_core::SelectRoomError::CommandSubmit(error) => {
            format!("command submit failed: {error}")
        }
        koushi_core::SelectRoomError::SessionNotReady => "session not ready".to_owned(),
        koushi_core::SelectRoomError::RoomNotInState => "room not yet loaded".to_owned(),
        koushi_core::SelectRoomError::FailedNoOp(IntentNoOpReason::TimelineTargetMissing) => {
            "timeline target not available".to_owned()
        }
        koushi_core::SelectRoomError::FailedNoOp(IntentNoOpReason::AlreadyActive) => {
            "room selection did not complete".to_owned()
        }
        koushi_core::SelectRoomError::FailedNoOp(
            IntentNoOpReason::SessionNotReady
            | IntentNoOpReason::RoomNotInState
            | IntentNoOpReason::Superseded,
        ) => "room selection did not complete".to_owned(),
        koushi_core::SelectRoomError::OperationFailed(failure) => {
            invoke_error_from_core_failure("room selection failed", failure)
        }
        koushi_core::SelectRoomError::EventStreamClosed | koushi_core::SelectRoomError::Timeout => {
            "room selection did not complete".to_owned()
        }
    }
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

async fn wait_for_focused_context_closed(
    event_conn: &mut CoreConnection,
    request_id: RequestId,
    account_key: AccountKey,
    room_id: Option<String>,
    baseline_generation: u64,
    deadline: tokio::time::Instant,
) -> Result<koushi_protocol::state_update::VersionedAppStateSnapshot, String> {
    let outcome = event_conn
        .wait_for_request_outcome(
            OutcomeCorrelation::Request(request_id),
            RequestOutcomeExpectation::FocusedContextClosed {
                request_id,
                account_key,
                room_id,
                allow_projection_only: true,
            },
            baseline_generation,
            deadline,
        )
        .await
        .map_err(|error| match error {
            RequestOutcomeError::OperationFailed { failure } => {
                invoke_error_from_core_failure("focused context close", failure)
            }
            error => invoke_error_from_request_outcome("focused context close", error),
        })?;
    match outcome {
        RequestOutcome::FocusedContext { snapshot } => Ok(snapshot),
        _ => Err("focused context close returned an invalid outcome".to_owned()),
    }
}

async fn wait_for_focused_context(
    event_conn: &mut CoreConnection,
    request_id: RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: Option<String>,
    baseline_generation: u64,
    deadline: tokio::time::Instant,
) -> Result<koushi_protocol::state_update::VersionedAppStateSnapshot, String> {
    let outcome = event_conn
        .wait_for_request_outcome(
            OutcomeCorrelation::Request(request_id),
            RequestOutcomeExpectation::FocusedContextOpened {
                request_id,
                account_key,
                room_id,
                event_id,
            },
            baseline_generation,
            deadline,
        )
        .await
        .map_err(|error| match error {
            RequestOutcomeError::OperationFailed { failure } => {
                invoke_error_from_core_failure("focused context open", failure)
            }
            error => invoke_error_from_request_outcome("focused context open", error),
        })?;
    match outcome {
        RequestOutcome::FocusedContext { snapshot } => Ok(snapshot),
        _ => Err("focused context open returned an invalid outcome".to_owned()),
    }
}

pub(super) fn build_update_navigation_preference_command(
    request_id: koushi_protocol::RequestId,
    update: koushi_state::NavigationPreferenceUpdate,
) -> CoreCommand {
    CoreCommand::App(AppCommand::UpdateNavigationPreference { request_id, update })
}

pub(super) fn build_select_space_command(
    request_id: koushi_protocol::RequestId,
    space_id: Option<String>,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::SelectSpace {
        request_id,
        space_id,
    })
}

pub(super) fn build_reorder_spaces_command(
    request_id: koushi_protocol::RequestId,
    space_ids: Vec<String>,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::ReorderSpaces {
        request_id,
        space_ids,
    })
}

#[cfg(test)]
pub(super) fn build_select_room_command(
    request_id: koushi_protocol::RequestId,
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

#[cfg(test)]
mod tests;
