use super::*;
#[tauri::command]
pub async fn set_display_name(
    display_name: Option<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_set_display_name_command(request_id, display_name),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn set_local_user_alias(
    user_id: String,
    alias: Option<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_set_local_user_alias_command(request_id, user_id, alias),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn ignore_user(
    user_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_ignore_user_command(request_id, user_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn unignore_user(
    user_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_unignore_user_command(request_id, user_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn report_user(
    user_id: String,
    reason: Option<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_report_user_command(request_id, user_id, optional_non_blank(reason)),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn report_content(
    room_id: String,
    event_id: String,
    reason: Option<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_report_content_command(request_id, room_id, event_id, optional_non_blank(reason)),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn report_room(
    room_id: String,
    reason: Option<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_report_room_command(request_id, room_id, optional_non_blank(reason)),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn set_avatar(
    mime_type: String,
    bytes: Vec<u8>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    if bytes.is_empty() {
        return Err("avatar bytes must not be empty".to_owned());
    }
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_set_avatar_command(request_id, mime_type, bytes),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn download_avatar_thumbnail(
    mxc_uri: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<String, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_download_avatar_thumbnail_command(request_id, mxc_uri),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(request_id.sequence.to_string())
}

#[tauri::command]
pub async fn cancel_avatar_thumbnail(
    mxc_uri: String,
    request_sequence: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<(), String> {
    let request_id = next_request_id(state.inner()).await;
    let target_sequence = request_sequence
        .parse::<u64>()
        .map_err(|_| "avatar request sequence is invalid".to_owned())?;
    let target_request_id = koushi_protocol::RequestId {
        connection_id: request_id.connection_id,
        sequence: target_sequence,
    };
    submit_core_command(
        state.inner(),
        build_cancel_avatar_thumbnail_command(request_id, target_request_id, mxc_uri),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(())
}

pub(super) fn build_set_display_name_command(
    request_id: koushi_protocol::RequestId,
    display_name: Option<String>,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::SetDisplayName {
        request_id,
        display_name,
    })
}

pub(super) fn build_set_local_user_alias_command(
    request_id: koushi_protocol::RequestId,
    user_id: String,
    alias: Option<String>,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::SetLocalUserAlias {
        request_id,
        user_id,
        alias,
    })
}

pub(super) fn build_ignore_user_command(
    request_id: koushi_protocol::RequestId,
    user_id: String,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::IgnoreUser {
        request_id,
        user_id,
    })
}

pub(super) fn build_unignore_user_command(
    request_id: koushi_protocol::RequestId,
    user_id: String,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::UnignoreUser {
        request_id,
        user_id,
    })
}

pub(super) fn build_report_user_command(
    request_id: koushi_protocol::RequestId,
    user_id: String,
    reason: Option<String>,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::ReportUser {
        request_id,
        user_id,
        reason: reason.unwrap_or_default(),
    })
}

pub(super) fn build_report_content_command(
    request_id: koushi_protocol::RequestId,
    room_id: String,
    event_id: String,
    reason: Option<String>,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::ReportContent {
        request_id,
        room_id,
        event_id,
        reason,
    })
}

pub(super) fn build_report_room_command(
    request_id: koushi_protocol::RequestId,
    room_id: String,
    reason: Option<String>,
) -> CoreCommand {
    CoreCommand::Room(RoomCommand::ReportRoom {
        request_id,
        room_id,
        reason: reason.unwrap_or_default(),
    })
}

pub(super) fn build_set_avatar_command(
    request_id: koushi_protocol::RequestId,
    mime_type: String,
    bytes: Vec<u8>,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::SetAvatar {
        request_id,
        request: SetAvatarRequest { mime_type, bytes },
    })
}

pub(super) fn build_download_avatar_thumbnail_command(
    request_id: koushi_protocol::RequestId,
    mxc_uri: String,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::DownloadAvatarThumbnail {
        request_id,
        mxc_uri,
    })
}

pub(super) fn build_cancel_avatar_thumbnail_command(
    request_id: koushi_protocol::RequestId,
    target_request_id: koushi_protocol::RequestId,
    mxc_uri: String,
) -> CoreCommand {
    CoreCommand::Account(AccountCommand::CancelAvatarThumbnail {
        request_id,
        target_request_id,
        mxc_uri,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_avatar_thumbnail_command_keeps_target_request_identity() {
        let request_id = koushi_protocol::RequestId {
            connection_id: koushi_protocol::RuntimeConnectionId(3),
            sequence: 8,
        };
        let target_request_id = koushi_protocol::RequestId {
            connection_id: koushi_protocol::RuntimeConnectionId(3),
            sequence: 7,
        };
        assert!(matches!(
            build_cancel_avatar_thumbnail_command(
                request_id,
                target_request_id,
                "mxc://example.invalid/avatar".to_owned()
            ),
            CoreCommand::Account(AccountCommand::CancelAvatarThumbnail {
                request_id: actual_request_id,
                target_request_id: actual_target_request_id,
                mxc_uri
            }) if actual_request_id == request_id
                && actual_target_request_id == target_request_id
                && mxc_uri == "mxc://example.invalid/avatar"
        ));
    }
}
