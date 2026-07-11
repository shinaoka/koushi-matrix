use super::*;

const SUBMISSION_SETTLEMENT_TIMEOUT: Duration = Duration::from_secs(10);

trait SubmissionEventSource {
    fn snapshot(&self) -> koushi_state::AppState;
    fn recv_event(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Result<CoreEvent, EventStreamLag>> + Send + '_>>;
}

impl SubmissionEventSource for CoreConnection {
    fn snapshot(&self) -> koushi_state::AppState {
        CoreConnection::snapshot(self)
    }

    fn recv_event(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Result<CoreEvent, EventStreamLag>> + Send + '_>> {
        Box::pin(CoreConnection::recv_event(self))
    }
}

async fn wait_for_submission_settlement(
    event_conn: &mut CoreConnection,
    submission_id: SubmissionId,
) -> Result<SubmissionResponse, SubmissionFailure> {
    let (outcome, transaction_id) =
        wait_for_submission_outcome(event_conn, &submission_id, SUBMISSION_SETTLEMENT_TIMEOUT)
            .await?;
    let snapshot = event_conn.versioned_snapshot();
    Ok(SubmissionResponse {
        outcome,
        submission_id,
        transaction_id,
        snapshot: FrontendDesktopSnapshot::from_versioned(snapshot.state, snapshot.generation),
    })
}

async fn wait_for_submission_outcome<S: SubmissionEventSource>(
    source: &mut S,
    submission_id: &SubmissionId,
    timeout: Duration,
) -> Result<(SubmissionOutcome, Option<String>), SubmissionFailure> {
    let deadline = tokio::time::Instant::now() + timeout;
    let (outcome, transaction_id) = loop {
        let event = tokio::time::timeout_at(deadline, source.recv_event())
            .await
            .map_err(|_| SubmissionFailure::Timeout)?;
        match event {
            Ok(CoreEvent::Timeline(TimelineEvent::SubmissionAccepted {
                submission_id: accepted_id,
                transaction_id,
                ..
            })) if accepted_id == *submission_id => {
                break (SubmissionOutcome::Accepted, Some(transaction_id));
            }
            Ok(CoreEvent::Timeline(TimelineEvent::SubmissionRejected {
                submission_id: rejected_id,
                kind,
                ..
            })) if rejected_id == *submission_id => {
                break (SubmissionOutcome::Rejected { kind }, None);
            }
            Ok(_) => {}
            Err(EventStreamLag { skipped: 0 }) => return Err(SubmissionFailure::Disconnected),
            Err(_) => return Err(SubmissionFailure::Lagged),
        }
    };

    if matches!(outcome, SubmissionOutcome::Accepted) {
        loop {
            let snapshot = source.snapshot();
            if snapshot
                .timeline
                .submission_registry
                .accepted_submission_ids
                .contains(submission_id)
            {
                break;
            }
            tokio::time::timeout_at(deadline, source.recv_event())
                .await
                .map_err(|_| SubmissionFailure::Timeout)?
                .map_err(|lag| {
                    if lag.skipped == 0 {
                        SubmissionFailure::Disconnected
                    } else {
                        SubmissionFailure::Lagged
                    }
                })?;
        }
    }
    Ok((outcome, transaction_id))
}

#[tauri::command]
pub async fn resolve_composer_key_action(
    surface: ComposerSurface,
    key_event: ComposerKeyEvent,
    autocomplete_open: bool,
    send_enabled: bool,
    state: State<'_, CoreRuntimeState>,
) -> Result<ComposerResolvedAction, String> {
    let snapshot = state.connection.lock().await.snapshot();
    Ok(koushi_state::resolve_composer_key_action(
        key_event,
        ComposerResolverContext {
            surface,
            send_shortcut: snapshot.settings.values.keyboard.composer_send_shortcut,
            autocomplete_open,
            send_enabled,
        },
    ))
}

#[tauri::command]
pub async fn paginate_timeline_backwards(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    trace_tauri_timeline_command("submit", "paginate_backwards", request_id);
    submit_core_command(
        state.inner(),
        build_paginate_timeline_backwards_command(request_id, account_key, room_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn restore_timeline_anchor(
    timeline_key: TimelineKey,
    event_id: String,
    max_batches: u16,
    event_count: u16,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_restore_timeline_anchor_command(
            request_id,
            account_key,
            timeline_key,
            event_id,
            max_batches,
            event_count,
        ),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn ensure_timeline_subscribed(
    timeline_key: TimelineKey,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    trace_tauri_timeline_command("submit", "ensure_subscribed", request_id);
    submit_core_command(
        state.inner(),
        CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id,
            key: TimelineKey {
                account_key,
                kind: timeline_key.kind,
            },
        }),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn paginate_thread_timeline_backwards(
    room_id: String,
    root_event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_paginate_thread_timeline_backwards_command(
            request_id,
            account_key,
            room_id,
            root_event_id,
        ),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn send_text(
    submission_id: String,
    room_id: String,
    body: String,
    mentions: Option<koushi_state::MentionIntent>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<SubmissionResponse, SubmissionFailure> {
    if body.trim().is_empty() {
        return Err(SubmissionFailure::Invalid);
    }

    let transaction_id = format!(
        "desktop-{}",
        NEXT_TRANSACTION_ID.fetch_add(1, Ordering::Relaxed)
    );
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    let account_key = account_key_from_snapshot(state.inner()).await;
    let submission_id = SubmissionId::new(submission_id);
    if let Some(command) = build_submit_text_command(
        request_id,
        submission_id.clone(),
        account_key,
        room_id,
        transaction_id,
        body,
        mentions.unwrap_or_default(),
    ) {
        event_conn
            .command(command)
            .await
            .map_err(|_| SubmissionFailure::SubmitFailed)?;
    }
    let response = wait_for_submission_settlement(&mut event_conn, submission_id).await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(response)
}

#[tauri::command]
pub async fn schedule_send(
    room_id: String,
    body: String,
    send_at_ms: u64,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) = build_schedule_send_command(request_id, room_id, body, send_at_ms) {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn stage_uploads(
    room_id: String,
    items: Vec<StageUploadInputItem>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    if room_id.trim().is_empty() {
        return current_snapshot(state.inner()).await;
    }

    let room_id_for_wait = room_id.trim().to_owned();
    let expected_ids = items
        .iter()
        .filter(|item| !item.staged_id.trim().is_empty())
        .map(|item| item.staged_id.clone())
        .collect::<Vec<_>>();
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(build_set_upload_staging_command(request_id, room_id, items))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_upload_staging_snapshot(
        &mut event_conn,
        request_id,
        |snapshot| {
            snapshot.timeline.room_id.as_deref() == Some(room_id_for_wait.as_str())
                && snapshot.timeline.staged_uploads.len() == expected_ids.len()
                && expected_ids.iter().all(|expected_id| {
                    snapshot
                        .timeline
                        .staged_uploads
                        .iter()
                        .any(|item| item.staged_id == *expected_id)
                })
        },
        "upload staging did not update",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn update_staged_upload_caption(
    staged_id: String,
    caption: Option<String>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    if staged_id.trim().is_empty() {
        return current_snapshot(state.inner()).await;
    }

    let expected_caption = caption.as_ref().and_then(|body| {
        let trimmed = body.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    });
    let caption = caption.and_then(|body| {
        let trimmed = body.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(build_formatted_message_draft(
                trimmed.to_owned(),
                MentionIntent::default(),
            ))
        }
    });
    let staged_id_for_wait = staged_id.clone();
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(CoreCommand::App(AppCommand::UpdateStagedUploadCaption {
            request_id,
            staged_id,
            caption,
        }))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_upload_staging_snapshot(
        &mut event_conn,
        request_id,
        |snapshot| {
            snapshot
                .timeline
                .staged_uploads
                .iter()
                .find(|item| item.staged_id == staged_id_for_wait)
                .map(|item| {
                    item.caption
                        .as_ref()
                        .map(|caption| caption.plain_body.as_str())
                })
                == Some(expected_caption.as_deref())
        },
        "staged upload caption did not update",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn update_staged_upload_compression(
    staged_id: String,
    compression_choice: StagedUploadCompressionChoice,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    if staged_id.trim().is_empty() {
        return current_snapshot(state.inner()).await;
    }

    let staged_id_for_wait = staged_id.clone();
    let expected_choice = compression_choice;
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(CoreCommand::App(
            AppCommand::UpdateStagedUploadCompression {
                request_id,
                staged_id,
                compression_choice,
            },
        ))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_upload_staging_snapshot(
        &mut event_conn,
        request_id,
        |snapshot| {
            snapshot
                .timeline
                .staged_uploads
                .iter()
                .find(|item| item.staged_id == staged_id_for_wait)
                .map(|item| item.compression_choice)
                == Some(expected_choice)
        },
        "staged upload compression did not update",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn clear_upload_staging(
    room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    if room_id.trim().is_empty() {
        return current_snapshot(state.inner()).await;
    }

    let room_id_for_wait = room_id.trim().to_owned();
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(CoreCommand::App(AppCommand::ClearUploadStaging {
            request_id,
            room_id,
        }))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_upload_staging_snapshot(
        &mut event_conn,
        request_id,
        |snapshot| {
            snapshot.timeline.room_id.as_deref() == Some(room_id_for_wait.as_str())
                && snapshot.timeline.staged_uploads.is_empty()
        },
        "upload staging did not clear",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn cancel_scheduled_send(
    scheduled_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) = build_cancel_scheduled_send_command(request_id, scheduled_id) {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn reschedule_scheduled_send(
    scheduled_id: String,
    send_at_ms: u64,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) =
        build_reschedule_scheduled_send_command(request_id, scheduled_id, send_at_ms)
    {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn retry_send(
    room_id: String,
    transaction_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) =
        build_retry_send_command(request_id, account_key, room_id, transaction_id)
    {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn cancel_send(
    room_id: String,
    transaction_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) =
        build_cancel_send_command(request_id, account_key, room_id, transaction_id)
    {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn upload_media(
    room_id: String,
    filename: String,
    mime_type: String,
    bytes: Vec<u8>,
    caption: Option<String>,
    image_dimensions: Option<ImageUploadDimensions>,
    image_compression: Option<ImageUploadCompressionState>,
    thumbnail: Option<UploadMediaThumbnail>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    if bytes.is_empty() {
        return current_snapshot(state.inner()).await;
    }

    let transaction_id = format!(
        "desktop-media-{}",
        NEXT_TRANSACTION_ID.fetch_add(1, Ordering::Relaxed)
    );
    let account_key = account_key_from_snapshot(state.inner()).await;
    let (image_compression_mode, image_compression_policy) =
        image_upload_compression_contract_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) = build_upload_media_command(
        request_id,
        account_key,
        room_id,
        transaction_id,
        filename,
        mime_type,
        bytes,
        caption,
        image_compression_mode,
        image_compression_policy,
        image_dimensions,
        image_compression,
        thumbnail,
    ) {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn download_media(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    if event_id.trim().is_empty() {
        return current_snapshot(state.inner()).await;
    }

    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) = build_download_media_command(request_id, account_key, room_id, event_id)
    {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn save_downloaded_media(
    source_url: String,
    destination_path: String,
) -> Result<(), String> {
    let source_path = downloaded_media_source_path(&source_url)?;
    let destination = selected_save_destination_path(&destination_path)?;
    if let Some(parent) = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .map_err(|_| "media save destination could not be created".to_owned())?;
    }
    std::fs::copy(&source_path, &destination)
        .map(|_| ())
        .map_err(|_| "media file could not be saved".to_owned())
}

#[tauri::command]
pub async fn default_media_save_path(filename: String, app: AppHandle) -> Result<String, String> {
    let downloads_dir = app.path().download_dir().ok();
    Ok(
        default_media_save_path_for(&filename, downloads_dir.as_deref())
            .to_string_lossy()
            .into_owned(),
    )
}

fn default_media_save_path_for(filename: &str, downloads_dir: Option<&std::path::Path>) -> PathBuf {
    let safe_filename = safe_media_save_filename(filename);
    downloads_dir
        .map(|directory| directory.join(&safe_filename))
        .unwrap_or_else(|| PathBuf::from(safe_filename))
}

fn safe_media_save_filename(filename: &str) -> String {
    let trimmed = filename.trim();
    let candidate = if trimmed.is_empty() {
        "download"
    } else {
        trimmed
    };
    candidate
        .chars()
        .map(|character| match character {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        })
        .collect()
}

fn downloaded_media_source_path(source_url: &str) -> Result<PathBuf, String> {
    let source_path = local_media_source_path(source_url)?;
    let source_path = std::fs::canonicalize(&source_path)
        .map_err(|_| "media file could not be read".to_owned())?;
    let cache_root = std::fs::canonicalize(crate::app_data_dir()?.join("media_downloads"))
        .map_err(|_| "media cache is unavailable".to_owned())?;
    if !source_path.starts_with(&cache_root) {
        return Err("media file is outside the download cache".to_owned());
    }
    Ok(source_path)
}

fn local_media_source_path(source_url: &str) -> Result<PathBuf, String> {
    let trimmed = source_url.trim();
    if trimmed.is_empty() {
        return Err("media source is empty".to_owned());
    }
    if trimmed.contains("://") {
        return Err("media source must be a local cache path".to_owned());
    }
    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        return Err("media source must be an absolute cache path".to_owned());
    }
    Ok(path)
}

fn selected_save_destination_path(destination_path: &str) -> Result<PathBuf, String> {
    let trimmed = destination_path.trim();
    if trimmed.is_empty() {
        return Err("media save destination is empty".to_owned());
    }
    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        return Err("media save destination must be absolute".to_owned());
    }
    Ok(path)
}

#[tauri::command]
pub async fn load_message_source(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) =
        build_load_message_source_command(request_id, account_key, room_id, event_id)
    {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn request_room_key(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) =
        build_request_room_key_command(request_id, account_key, room_id, event_id)
    {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn load_link_previews(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    trace_tauri_timeline_command("submit", "load_link_previews", request_id);
    if let Some(command) =
        build_load_link_previews_command(request_id, account_key, room_id, event_id)
    {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn hide_link_preview(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) =
        build_hide_link_preview_command(request_id, account_key, room_id, event_id)
    {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn forward_message(
    room_id: String,
    source_event_id: String,
    destination_room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let transaction_id = format!(
        "desktop-forward-{}",
        NEXT_TRANSACTION_ID.fetch_add(1, Ordering::Relaxed)
    );
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) = build_forward_message_command(
        request_id,
        account_key,
        room_id,
        source_event_id,
        destination_room_id,
        transaction_id,
    ) {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn edit_message(
    room_id: String,
    event_id: String,
    body: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    if body.trim().is_empty() {
        return current_snapshot(state.inner()).await;
    }
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) =
        build_edit_message_command(request_id, account_key, room_id, event_id, body)
    {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn redact_message(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_redact_message_command(request_id, account_key, room_id, event_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn toggle_reaction(
    room_id: String,
    event_id: String,
    reaction_key: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    if reaction_key.is_empty() {
        return current_snapshot(state.inner()).await;
    }

    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) =
        build_toggle_reaction_command(request_id, account_key, room_id, event_id, reaction_key)
    {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn send_reaction(
    room_id: String,
    event_id: String,
    reaction_key: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    if reaction_key.trim().is_empty() || event_id.trim().is_empty() {
        return current_snapshot(state.inner()).await;
    }

    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    let trace_started = std::time::Instant::now();
    trace_tauri_timeline_command("submit", "send_reaction", request_id);
    if let Some(command) =
        build_send_reaction_command(request_id, account_key, room_id, event_id, reaction_key)
    {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    let snapshot = current_snapshot(state.inner()).await;
    trace_tauri_timeline_command_elapsed(
        "done",
        "send_reaction",
        request_id,
        trace_started.elapsed().as_millis(),
    );
    snapshot
}

#[tauri::command]
pub async fn redact_reaction(
    room_id: String,
    event_id: String,
    reaction_key: String,
    reaction_event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    if reaction_key.trim().is_empty()
        || event_id.trim().is_empty()
        || reaction_event_id.trim().is_empty()
    {
        return current_snapshot(state.inner()).await;
    }

    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    let trace_started = std::time::Instant::now();
    trace_tauri_timeline_command("submit", "redact_reaction", request_id);
    if let Some(command) = build_redact_reaction_command(
        request_id,
        account_key,
        room_id,
        event_id,
        reaction_key,
        reaction_event_id,
    ) {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    let snapshot = current_snapshot(state.inner()).await;
    trace_tauri_timeline_command_elapsed(
        "done",
        "redact_reaction",
        request_id,
        trace_started.elapsed().as_millis(),
    );
    snapshot
}

#[tauri::command]
pub async fn set_composer_reply_target(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::App(AppCommand::SetComposerReplyTarget {
            request_id,
            room_id,
            event_id,
        }),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn cancel_composer_reply(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::App(AppCommand::CancelComposerReply { request_id }),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn set_composer_draft(
    room_id: String,
    draft: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_set_composer_draft_command(request_id, room_id, draft),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn set_thread_composer_draft(
    room_id: String,
    root_event_id: String,
    draft: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        build_set_thread_composer_draft_command(request_id, room_id, root_event_id, draft),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn send_reply(
    submission_id: String,
    room_id: String,
    in_reply_to_event_id: String,
    body: String,
    mentions: Option<koushi_state::MentionIntent>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<SubmissionResponse, SubmissionFailure> {
    if body.trim().is_empty() {
        return Err(SubmissionFailure::Invalid);
    }

    let transaction_id = format!(
        "desktop-{}",
        NEXT_TRANSACTION_ID.fetch_add(1, Ordering::Relaxed)
    );
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    let account_key = account_key_from_snapshot(state.inner()).await;
    let submission_id = SubmissionId::new(submission_id);
    if let Some(command) = build_submit_reply_command(
        request_id,
        submission_id.clone(),
        account_key,
        room_id,
        transaction_id,
        in_reply_to_event_id,
        body,
        mentions.unwrap_or_default(),
    ) {
        event_conn
            .command(command)
            .await
            .map_err(|_| SubmissionFailure::SubmitFailed)?;
    }
    let response = wait_for_submission_settlement(&mut event_conn, submission_id).await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(response)
}

#[tauri::command]
pub async fn send_thread_reply(
    submission_id: String,
    room_id: String,
    root_event_id: String,
    body: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<SubmissionResponse, SubmissionFailure> {
    if body.trim().is_empty() {
        return Err(SubmissionFailure::Invalid);
    }

    let transaction_id = format!(
        "desktop-{}",
        NEXT_TRANSACTION_ID.fetch_add(1, Ordering::Relaxed)
    );
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    let account_key = account_key_from_snapshot(state.inner()).await;
    let submission_id = SubmissionId::new(submission_id);
    if let Some(command) = build_submit_thread_reply_command(
        request_id,
        submission_id.clone(),
        account_key,
        room_id,
        root_event_id,
        transaction_id,
        body,
    ) {
        event_conn
            .command(command)
            .await
            .map_err(|_| SubmissionFailure::SubmitFailed)?;
    }
    let response = wait_for_submission_settlement(&mut event_conn, submission_id).await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(response)
}

#[cfg(test)]
mod submission_settlement_tests {
    use std::collections::VecDeque;

    use super::*;

    struct ScriptedSource {
        state: koushi_state::AppState,
        events: VecDeque<(Result<CoreEvent, EventStreamLag>, Option<SubmissionId>)>,
        pending_on_empty: bool,
    }

    impl SubmissionEventSource for ScriptedSource {
        fn snapshot(&self) -> koushi_state::AppState {
            self.state.clone()
        }

        fn recv_event(
            &mut self,
        ) -> Pin<Box<dyn Future<Output = Result<CoreEvent, EventStreamLag>> + Send + '_>> {
            if let Some((event, accepted_id)) = self.events.pop_front() {
                if let Some(accepted_id) = accepted_id {
                    self.state
                        .timeline
                        .submission_registry
                        .accepted_submission_ids
                        .push_back(accepted_id);
                }
                Box::pin(async move { event })
            } else if self.pending_on_empty {
                Box::pin(std::future::pending())
            } else {
                Box::pin(async { Err(EventStreamLag { skipped: 0 }) })
            }
        }
    }

    fn accepted(id: SubmissionId, sequence: u64) -> CoreEvent {
        CoreEvent::Timeline(TimelineEvent::SubmissionAccepted {
            request_id: RequestId {
                connection_id: koushi_core::RuntimeConnectionId(1),
                sequence,
            },
            key: build_timeline_key(AccountKey("@u:test".to_owned()), "!r:test".to_owned()),
            submission_id: id,
            transaction_id: "txn".to_owned(),
        })
    }

    #[tokio::test]
    async fn waits_for_global_reducer_acceptance_after_active_room_switch() {
        let expected = SubmissionId::new("expected");
        let mut switched_state = koushi_state::AppState::default();
        switched_state.timeline.room_id = Some("!room-b:test".to_owned());
        let mut source = ScriptedSource {
            state: switched_state,
            events: VecDeque::from([
                (Ok(accepted(SubmissionId::new("other"), 1)), None),
                (Ok(accepted(expected.clone(), 2)), None),
                (
                    Ok(accepted(SubmissionId::new("after-accept"), 3)),
                    Some(expected.clone()),
                ),
            ]),
            pending_on_empty: false,
        };
        let result = wait_for_submission_outcome(&mut source, &expected, Duration::from_secs(1))
            .await
            .expect("accepted");
        assert_eq!(result.0, SubmissionOutcome::Accepted);
    }

    #[tokio::test]
    async fn matching_rejection_disconnect_lag_and_timeout_are_typed() {
        let expected = SubmissionId::new("expected");
        let rejected = CoreEvent::Timeline(TimelineEvent::SubmissionRejected {
            request_id: RequestId {
                connection_id: koushi_core::RuntimeConnectionId(1),
                sequence: 1,
            },
            key: build_timeline_key(AccountKey("@u:test".to_owned()), "!r:test".to_owned()),
            submission_id: expected.clone(),
            kind: koushi_core::TimelineFailureKind::NotSubscribed,
        });
        let mut source = ScriptedSource {
            state: koushi_state::AppState::default(),
            events: VecDeque::from([(Ok(rejected), None)]),
            pending_on_empty: false,
        };
        assert!(matches!(
            wait_for_submission_outcome(&mut source, &expected, Duration::from_secs(1)).await,
            Ok((
                SubmissionOutcome::Rejected {
                    kind: koushi_core::TimelineFailureKind::NotSubscribed
                },
                None
            ))
        ));
        let mut disconnected = ScriptedSource {
            state: koushi_state::AppState::default(),
            events: VecDeque::new(),
            pending_on_empty: false,
        };
        assert_eq!(
            wait_for_submission_outcome(&mut disconnected, &expected, Duration::from_secs(1)).await,
            Err(SubmissionFailure::Disconnected)
        );
        let mut lagged = ScriptedSource {
            state: koushi_state::AppState::default(),
            events: VecDeque::from([(Err(EventStreamLag { skipped: 1 }), None)]),
            pending_on_empty: false,
        };
        assert_eq!(
            wait_for_submission_outcome(&mut lagged, &expected, Duration::from_secs(1)).await,
            Err(SubmissionFailure::Lagged)
        );
        let mut timed_out = ScriptedSource {
            state: koushi_state::AppState::default(),
            events: VecDeque::new(),
            pending_on_empty: true,
        };
        assert_eq!(
            wait_for_submission_outcome(&mut timed_out, &expected, Duration::from_millis(1)).await,
            Err(SubmissionFailure::Timeout)
        );
    }
}

#[cfg(test)]
mod save_downloaded_media_tests {
    use super::*;

    #[test]
    fn default_media_save_path_prefers_downloads_directory() {
        let downloads = PathBuf::from("/tmp/koushi-downloads");

        assert_eq!(
            default_media_save_path_for(" report:name?.png ", Some(downloads.as_path())),
            downloads.join("report_name_.png")
        );
    }

    #[test]
    fn default_media_save_path_falls_back_to_safe_filename() {
        assert_eq!(
            default_media_save_path_for("   ", None),
            PathBuf::from("download")
        );
        assert_eq!(
            default_media_save_path_for("bad/path:name.txt", None),
            PathBuf::from("bad_path_name.txt")
        );
    }

    #[test]
    fn local_media_source_path_rejects_urls() {
        assert!(local_media_source_path("asset://localhost/file.png").is_err());
        assert!(local_media_source_path("https://example.invalid/file.png").is_err());
    }

    #[test]
    fn local_media_source_path_requires_absolute_path() {
        assert!(local_media_source_path("media_downloads/file.png").is_err());
    }

    #[test]
    fn selected_save_destination_path_rejects_empty_and_relative_paths() {
        assert!(selected_save_destination_path("").is_err());
        assert!(selected_save_destination_path("Downloads/file.png").is_err());
    }
}
