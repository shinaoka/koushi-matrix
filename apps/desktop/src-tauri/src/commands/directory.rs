use super::navigation::{SELECT_ROOM_EVENT_TIMEOUT, invoke_error_from_select_room_error};
use super::room::ROOM_OPERATION_EVENT_TIMEOUT;
use super::*;
#[tauri::command]
pub async fn query_directory(
    term: Option<String>,
    server_name: Option<String>,
    limit: Option<u32>,
    since: Option<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let baseline = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline.state);
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_query_directory_command(
            request_id,
            term,
            server_name,
            limit,
            since,
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    let outcome = event_conn
        .wait_for_request_outcome(
            OutcomeCorrelation::Request(request_id),
            RequestOutcomeExpectation::DirectoryQuery {
                request_id,
                account_key,
            },
            baseline.generation,
            tokio::time::Instant::now() + ROOM_OPERATION_EVENT_TIMEOUT,
        )
        .await
        .map_err(|error| invoke_error_from_request_outcome("directory query", error))?;
    let RequestOutcome::Directory { generation, .. } = outcome else {
        return Err("directory query returned an invalid outcome".to_owned());
    };
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(FrontendCommandSettlement::from_published_generation(
        generation,
    ))
}

#[tauri::command]
pub async fn join_directory_room(
    room_id_or_alias: String,
    via_servers: Vec<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let baseline = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline.state);
    let request_id = event_conn.next_request_id();
    let Some(command) =
        build_join_directory_room_command(request_id, room_id_or_alias, via_servers)
    else {
        return Err("directory target must not be blank".to_owned());
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
                room_id: String::new(),
            },
            baseline.generation,
            tokio::time::Instant::now() + ROOM_OPERATION_EVENT_TIMEOUT,
        )
        .await
        .map_err(|error| invoke_error_from_request_outcome("room join", error))?;
    let RequestOutcome::RoomJoined {
        room_id: joined_room_id,
        ..
    } = outcome
    else {
        return Err("room join returned an invalid outcome".to_owned());
    };
    let selected_generation = event_conn
        .select_room_and_wait(joined_room_id.clone(), SELECT_ROOM_EVENT_TIMEOUT)
        .await
        .map_err(invoke_error_from_select_room_error)?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(FrontendCommandSettlement::from_published_generation(
        selected_generation,
    ))
}

#[tauri::command]
pub async fn preview_join_target(
    room_id_or_alias: String,
    via_servers: Vec<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let mut event_conn = state.runtime.attach();
    let baseline = event_conn.versioned_snapshot();
    let account_key = account_key_from_app_state(&baseline.state);
    let request_id = event_conn.next_request_id();
    let Some(command) =
        build_preview_join_target_command(request_id, room_id_or_alias, via_servers)
    else {
        return Err("directory target must not be blank".to_owned());
    };

    event_conn
        .command(command)
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    let outcome = event_conn
        .wait_for_request_outcome(
            OutcomeCorrelation::Request(request_id),
            RequestOutcomeExpectation::DirectoryPreview {
                request_id,
                account_key,
            },
            baseline.generation,
            tokio::time::Instant::now() + ROOM_OPERATION_EVENT_TIMEOUT,
        )
        .await
        .map_err(|error| invoke_error_from_request_outcome("directory preview", error))?;
    let RequestOutcome::Directory { generation, .. } = outcome else {
        return Err("directory preview returned an invalid outcome".to_owned());
    };
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(FrontendCommandSettlement::from_published_generation(
        generation,
    ))
}

#[tauri::command]
pub async fn dismiss_directory_preview(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_dismiss_directory_preview_command(request_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

/// Preview and join name the same target, so they normalize it the same way.
///
/// A blank server name is not a routing hint, and keeping duplicates would make
/// the homeserver retry the same server.
fn normalize_join_target(
    room_id_or_alias: String,
    via_servers: Vec<String>,
) -> Option<(String, Vec<String>)> {
    let room_id_or_alias = room_id_or_alias.trim().to_owned();
    if room_id_or_alias.is_empty() {
        return None;
    }
    let mut seen = std::collections::BTreeSet::new();
    let via_servers = via_servers
        .into_iter()
        .filter_map(|server| optional_non_blank(Some(server)))
        .filter(|server| seen.insert(server.clone()))
        .collect::<Vec<_>>();
    Some((room_id_or_alias, via_servers))
}

pub(super) fn build_create_room_command(
    request_id: koushi_protocol::RequestId,
    options: CreateRoomOptions,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::CreateRoom {
        request_id,
        options,
    })
}

pub(super) fn build_create_space_command(
    request_id: koushi_protocol::RequestId,
    name: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::CreateSpace { request_id, name })
}

pub(super) fn build_join_room_command(
    request_id: koushi_protocol::RequestId,
    room_id: String,
) -> Option<CoreCommand> {
    let room_id = room_id.trim().to_owned();
    if room_id.is_empty() {
        return None;
    }
    Some(CoreCommand::Room(RoomCommand::JoinRoom {
        request_id,
        room_id,
    }))
}

pub(super) fn build_set_space_child_command(
    request_id: koushi_protocol::RequestId,
    space_id: String,
    child_room_id: String,
    via_server: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::SetSpaceChild {
        request_id,
        space_id,
        child_room_id,
        via_server,
    })
}

pub(super) fn build_accept_invite_command(
    request_id: koushi_protocol::RequestId,
    room_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::AcceptInvite {
        request_id,
        room_id,
    })
}

pub(super) fn build_decline_invite_command(
    request_id: koushi_protocol::RequestId,
    room_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::DeclineInvite {
        request_id,
        room_id,
    })
}

pub(super) fn build_start_direct_message_command(
    request_id: koushi_protocol::RequestId,
    user_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::StartDirectMessage {
        request_id,
        user_id,
    })
}

pub(super) fn build_invite_user_command(
    request_id: koushi_protocol::RequestId,
    room_id: String,
    user_id: String,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::InviteUser {
        request_id,
        room_id,
        user_id,
    })
}

pub(super) fn build_invite_user_to_space_command(
    request_id: koushi_protocol::RequestId,
    space_id: String,
    user_id: String,
    generation: u64,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::InviteUserToSpace {
        request_id,
        space_id,
        user_id,
        generation,
    })
}

pub(super) fn build_cancel_space_invite_command(
    request_id: koushi_protocol::RequestId,
    space_id: String,
    user_id: String,
    generation: u64,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::CancelSpaceInvite {
        request_id,
        space_id,
        user_id,
        generation,
    })
}

pub(super) fn build_open_invite_workflow_command(
    request_id: koushi_protocol::RequestId,
    room_id: String,
) -> CoreCommand {
    CoreCommand::App(AppCommand::OpenInviteWorkflow {
        request_id,
        room_id,
    })
}

pub(super) fn build_close_invite_workflow_command(
    request_id: koushi_protocol::RequestId,
) -> CoreCommand {
    CoreCommand::App(AppCommand::CloseInviteWorkflow { request_id })
}

pub(super) fn build_search_invite_targets_command(
    request_id: koushi_protocol::RequestId,
    room_id: String,
    query: String,
) -> CoreCommand {
    CoreCommand::App(AppCommand::SearchInviteTargets {
        request_id,
        room_id,
        query,
    })
}

pub(super) fn build_set_invite_scope_command(
    request_id: koushi_protocol::RequestId,
    room_id: String,
    scope: InviteScopeSelection,
) -> CoreCommand {
    CoreCommand::App(AppCommand::SetInviteScope {
        request_id,
        room_id,
        scope,
    })
}

pub(super) fn build_select_invite_target_command(
    request_id: koushi_protocol::RequestId,
    room_id: String,
    user_id: String,
) -> CoreCommand {
    CoreCommand::App(AppCommand::SelectInviteTarget {
        request_id,
        room_id,
        user_id,
    })
}

pub(super) fn build_remove_invite_target_command(
    request_id: koushi_protocol::RequestId,
    user_id: String,
) -> CoreCommand {
    CoreCommand::App(AppCommand::RemoveInviteTarget {
        request_id,
        user_id,
    })
}

pub(super) fn build_invite_targets_command(
    request_id: koushi_protocol::RequestId,
    room_id: String,
    user_ids: Vec<String>,
    scope: InviteScopeSelection,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::InviteTargets {
        request_id,
        room_id,
        user_ids,
        scope,
    })
}

pub(super) fn build_query_directory_command(
    request_id: koushi_protocol::RequestId,
    term: Option<String>,
    server_name: Option<String>,
    limit: Option<u32>,
    since: Option<String>,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::QueryDirectory {
        request_id,
        query: DirectoryQuery {
            term: optional_non_blank(term),
            server_name: optional_non_blank(server_name),
            limit,
            since: optional_non_blank(since),
        },
    })
}

pub(super) fn build_preview_join_target_command(
    request_id: koushi_protocol::RequestId,
    room_id_or_alias: String,
    via_servers: Vec<String>,
) -> Option<CoreCommand> {
    let (room_id_or_alias, via_servers) = normalize_join_target(room_id_or_alias, via_servers)?;
    Some(CoreCommand::Room(RoomCommand::PreviewJoinTarget {
        request_id,
        room_id_or_alias,
        via_servers,
    }))
}

pub(super) fn build_dismiss_directory_preview_command(
    request_id: koushi_protocol::RequestId,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::DismissDirectoryPreview { request_id })
}

pub(super) fn build_join_directory_room_command(
    request_id: koushi_protocol::RequestId,
    room_id_or_alias: String,
    via_servers: Vec<String>,
) -> Option<CoreCommand> {
    let (room_id_or_alias, via_servers) = normalize_join_target(room_id_or_alias, via_servers)?;
    Some(CoreCommand::Room(RoomCommand::JoinDirectoryRoom {
        request_id,
        room_id_or_alias,
        via_servers,
    }))
}
