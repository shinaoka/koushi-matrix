use super::*;

pub(super) const TIMELINE_BACKWARDS_PAGE_EVENT_COUNT: u16 = 100;
#[cfg(test)]
pub(super) const TIMELINE_RESTORE_ANCHOR_MAX_BATCHES: u16 = 6;

pub(super) fn trace_tauri_timeline_command(
    stage: &'static str,
    kind: &'static str,
    request_id: RequestId,
) {
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "desktop.timeline", stage)
            .field(DiagnosticField::token("operation", kind))
            .field(DiagnosticField::request_id(
                "request_id",
                request_id.connection_id.0,
                request_id.sequence,
            )),
    );
}

pub(super) fn trace_tauri_timeline_command_elapsed(
    stage: &'static str,
    kind: &'static str,
    request_id: RequestId,
    elapsed_ms: u128,
) {
    record(
        DiagnosticEvent::new(DiagnosticLevel::Debug, "desktop.timeline", stage)
            .field(DiagnosticField::token("operation", kind))
            .field(DiagnosticField::request_id(
                "request_id",
                request_id.connection_id.0,
                request_id.sequence,
            ))
            .field(DiagnosticField::milliseconds("elapsed_ms", elapsed_ms)),
    );
}

async fn wait_for_upload_staging_snapshot(
    event_conn: &mut CoreConnection,
    request_id: RequestId,
    predicate: impl Fn(&koushi_state::AppState) -> bool,
    description: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + UPLOAD_STAGING_EVENT_TIMEOUT;

    loop {
        if predicate(&event_conn.snapshot()) {
            return Ok(());
        }

        let event = tokio::time::timeout_at(deadline, event_conn.recv_event())
            .await
            .map_err(|_| description.to_owned())?;
        match event {
            Ok(CoreEvent::StateChanged(snapshot)) if predicate(&snapshot) => return Ok(()),
            Ok(CoreEvent::OperationFailed {
                request_id: failed_request_id,
                failure,
            }) if failed_request_id == request_id => {
                return Err(invoke_error_from_core_failure(description, failure));
            }
            Ok(_) => {}
            Err(_) if predicate(&event_conn.snapshot()) => return Ok(()),
            Err(_) => continue,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageUploadInputItem {
    staged_id: String,
    position: u64,
    filename: String,
    mime_type: String,
    byte_count: u64,
    kind: StagedUploadKind,
    compression_choice: StagedUploadCompressionChoice,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageUploadBytesInputItem {
    staged_id: String,
    position: u64,
    filename: String,
    mime_type: String,
    bytes: Vec<u8>,
}

impl std::fmt::Debug for StageUploadBytesInputItem {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StageUploadBytesInputItem")
            .field("staged_id", &"StagedUploadId(..)")
            .field("position", &self.position)
            .field("filename", &"MediaFilename(..)")
            .field("mime_type", &self.mime_type)
            .field("byte_count", &self.bytes.len())
            .finish()
    }
}

pub(super) fn build_timeline_key(account_key: AccountKey, room_id: String) -> TimelineKey {
    TimelineKey {
        account_key,
        kind: TimelineKind::Room { room_id },
    }
}

#[cfg(test)]
pub(super) fn build_subscribe_focused_timeline_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
) -> CoreCommand {
    CoreCommand::Timeline(TimelineCommand::Subscribe {
        request_id,
        key: TimelineKey {
            account_key,
            kind: TimelineKind::Focused { room_id, event_id },
        },
    })
}

pub(super) fn build_paginate_timeline_backwards_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
) -> CoreCommand {
    CoreCommand::Timeline(TimelineCommand::Paginate {
        request_id,
        key: build_timeline_key(account_key, room_id),
        direction: PaginationDirection::Backward,
        event_count: TIMELINE_BACKWARDS_PAGE_EVENT_COUNT,
    })
}

pub(super) fn build_paginate_thread_timeline_backwards_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    root_event_id: String,
) -> CoreCommand {
    CoreCommand::Timeline(TimelineCommand::Paginate {
        request_id,
        key: TimelineKey {
            account_key,
            kind: TimelineKind::Thread {
                room_id,
                root_event_id,
            },
        },
        direction: PaginationDirection::Backward,
        event_count: TIMELINE_BACKWARDS_PAGE_EVENT_COUNT,
    })
}

pub(super) fn build_restore_timeline_anchor_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    timeline_key: TimelineKey,
    event_id: String,
    max_batches: u16,
    event_count: u16,
) -> CoreCommand {
    CoreCommand::Timeline(TimelineCommand::RestoreTimelineAnchor {
        request_id,
        key: TimelineKey {
            account_key,
            kind: timeline_key.kind,
        },
        event_id,
        max_batches,
        event_count,
    })
}

pub(super) fn build_open_timeline_at_timestamp_command(
    request_id: koushi_core::RequestId,
    room_id: String,
    timestamp_ms: u64,
) -> CoreCommand {
    CoreCommand::App(AppCommand::OpenTimelineAtTimestamp {
        request_id,
        room_id,
        timestamp_ms,
    })
}

pub(super) fn build_update_navigation_scroll_anchor_command(
    request_id: koushi_core::RequestId,
    room_id: String,
    anchor: TimelineScrollAnchor,
) -> CoreCommand {
    CoreCommand::App(AppCommand::TimelineScrollAnchorUpdated {
        request_id,
        room_id,
        anchor,
    })
}

pub(super) fn build_observe_timeline_viewport_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    first_visible_event_id: Option<String>,
    last_visible_event_id: Option<String>,
    visible_gap_ids: Vec<TimelineGapId>,
    at_bottom: bool,
    thread_root_event_id: Option<String>,
) -> CoreCommand {
    let key = match thread_root_event_id {
        Some(root_event_id) => TimelineKey {
            account_key,
            kind: TimelineKind::Thread {
                room_id,
                root_event_id,
            },
        },
        None => build_timeline_key(account_key, room_id),
    };
    CoreCommand::Timeline(TimelineCommand::ObserveViewport {
        request_id,
        key,
        observation: TimelineViewportObservation {
            first_visible_event_id,
            last_visible_event_id,
            visible_gap_ids,
            at_bottom,
        },
    })
}

#[cfg(test)]
pub(super) fn build_send_text_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    transaction_id: String,
    document: ComposerDocument,
) -> Option<CoreCommand> {
    if document.plain_body().trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::SendText {
        request_id,
        key: build_timeline_key(account_key, room_id),
        transaction_id,
        document,
    }))
}

pub(super) fn build_submit_text_command(
    request_id: RequestId,
    expected_account: koushi_key::SessionKeyId,
    submission_id: SubmissionId,
    account_key: AccountKey,
    room_id: String,
    transaction_id: String,
    document: ComposerDocument,
    draft_revision: ComposerDraftRevision,
) -> Option<CoreCommand> {
    if document.plain_body().trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::SubmitText {
        request_id,
        expected_account,
        submission_id,
        key: build_timeline_key(account_key, room_id),
        transaction_id,
        document,
        draft_revision,
    }))
}

pub(super) fn build_schedule_send_command(
    request_id: koushi_core::RequestId,
    expected_account: koushi_key::SessionKeyId,
    target: koushi_state::ComposerTarget,
    body: String,
    send_at_ms: u64,
    draft_revision: ComposerDraftRevision,
) -> Option<CoreCommand> {
    if body.trim().is_empty() {
        return None;
    }
    let (room_id, thread_root_event_id) = match target {
        koushi_state::ComposerTarget::Main { room_id } => (room_id, None),
        koushi_state::ComposerTarget::Thread {
            room_id,
            root_event_id,
        } => (room_id, Some(root_event_id)),
    };
    Some(CoreCommand::App(AppCommand::ScheduleSend {
        request_id,
        expected_account,
        room_id,
        thread_root_event_id,
        body,
        send_at_ms,
        draft_revision,
    }))
}

pub(super) fn build_set_upload_staging_command(
    request_id: koushi_core::RequestId,
    room_id: String,
    items: Vec<StageUploadInputItem>,
) -> CoreCommand {
    let room_id = room_id.trim().to_owned();
    let staged_items = items
        .into_iter()
        .filter(|item| !item.staged_id.trim().is_empty())
        .map(|item| StagedUploadItem {
            staged_id: item.staged_id,
            room_id: room_id.clone(),
            position: item.position,
            filename: match item.filename.trim() {
                "" => "attachment".to_owned(),
                value => value.to_owned(),
            },
            mime_type: match item.mime_type.trim() {
                "" => "application/octet-stream".to_owned(),
                value => value.to_owned(),
            },
            byte_count: item.byte_count,
            kind: item.kind,
            caption: None,
            compression_choice: item.compression_choice,
            preparation: Default::default(),
        })
        .collect();
    CoreCommand::App(AppCommand::SetUploadStaging {
        request_id,
        target: koushi_state::ComposerTarget::Main { room_id },
        items: staged_items,
    })
}

pub(super) fn build_cancel_scheduled_send_command(
    request_id: koushi_core::RequestId,
    scheduled_id: String,
) -> Option<CoreCommand> {
    if scheduled_id.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::App(AppCommand::CancelScheduledSend {
        request_id,
        scheduled_id,
    }))
}

pub(super) fn build_reschedule_scheduled_send_command(
    request_id: koushi_core::RequestId,
    scheduled_id: String,
    body: String,
    send_at_ms: u64,
) -> Option<CoreCommand> {
    if scheduled_id.trim().is_empty() || body.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::App(AppCommand::RescheduleScheduledSend {
        request_id,
        scheduled_id,
        body,
        send_at_ms,
    }))
}

pub(super) fn build_retry_send_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    transaction_id: String,
) -> Option<CoreCommand> {
    if transaction_id.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::RetrySend {
        request_id,
        key: build_timeline_key(account_key, room_id),
        transaction_id,
    }))
}

pub(super) fn build_cancel_send_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    transaction_id: String,
) -> Option<CoreCommand> {
    if transaction_id.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::CancelSend {
        request_id,
        key: build_timeline_key(account_key, room_id),
        transaction_id,
    }))
}

pub(super) fn build_upload_media_command(
    request_id: koushi_core::RequestId,
    expected_account: koushi_key::SessionKeyId,
    account_key: AccountKey,
    room_id: String,
    transaction_id: String,
    filename: String,
    mime_type: String,
    bytes: Vec<u8>,
    caption: Option<String>,
    image_compression_mode: ImageUploadCompressionMode,
    image_compression_policy: ImageUploadCompressionPolicy,
    image_dimensions: Option<ImageUploadDimensions>,
    image_compression: Option<ImageUploadCompressionState>,
    thumbnail: Option<UploadMediaThumbnail>,
) -> Option<CoreCommand> {
    if bytes.is_empty() {
        return None;
    }
    let filename = match filename.trim() {
        "" => "attachment".to_owned(),
        value => value.to_owned(),
    };
    let mime_type = match mime_type.trim() {
        "" => "application/octet-stream".to_owned(),
        value => value.to_owned(),
    };
    let is_image = mime_type.to_ascii_lowercase().starts_with("image/");
    let selected_byte_count = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let image_compression = if is_image {
        Some(normalize_image_upload_compression(
            image_compression_mode,
            image_compression_policy,
            mime_type.clone(),
            selected_byte_count,
            image_dimensions,
            image_compression,
            thumbnail.is_some(),
        ))
    } else {
        None
    };
    let selected_dimensions = image_compression
        .as_ref()
        .and_then(|compression| compression.selected.dimensions)
        .or(image_dimensions);
    let kind = if is_image {
        UploadMediaKind::Image {
            width: selected_dimensions.map(|dimensions| dimensions.width),
            height: selected_dimensions.map(|dimensions| dimensions.height),
        }
    } else {
        UploadMediaKind::File
    };

    Some(CoreCommand::Timeline(TimelineCommand::UploadAndSendMedia {
        request_id,
        expected_account,
        key: build_timeline_key(account_key, room_id),
        transaction_id,
        request: UploadMediaRequest {
            filename,
            mime_type,
            bytes,
            kind,
            compression: image_compression,
            thumbnail: if is_image { thumbnail } else { None },
            caption: media_caption_from_composer_body(caption),
        },
    }))
}

fn normalize_image_upload_compression(
    mode: ImageUploadCompressionMode,
    policy: ImageUploadCompressionPolicy,
    mime_type: String,
    selected_byte_count: u64,
    image_dimensions: Option<ImageUploadDimensions>,
    image_compression: Option<ImageUploadCompressionState>,
    thumbnail_present: bool,
) -> ImageUploadCompressionState {
    match image_compression {
        Some(mut compression) => {
            compression.mode = mode;
            compression.policy = policy;
            if compression.original.mime_type.trim().is_empty() {
                compression.original.mime_type = mime_type.clone();
            }
            if compression.selected.mime_type.trim().is_empty() {
                compression.selected.mime_type = mime_type;
            }
            compression.selected.byte_count = selected_byte_count;
            if compression.selected.dimensions.is_none() {
                compression.selected.dimensions = image_dimensions;
            }
            if compression.selected_variant == ImageUploadVariantKind::Original {
                compression.metadata_stripped = false;
            }
            if thumbnail_present {
                compression.thumbnail_refreshed = true;
            }
            compression
        }
        None => {
            let mut compression = ImageUploadCompressionState::original(
                mode,
                mime_type,
                selected_byte_count,
                image_dimensions,
            );
            compression.policy = policy;
            compression.skipped_small_image = policy.should_skip(&compression.original);
            compression
        }
    }
}

fn media_caption_from_composer_body(
    caption: Option<String>,
) -> Option<koushi_state::FormattedMessageDraft> {
    let caption = caption?.trim().to_owned();
    if caption.is_empty() {
        return None;
    }
    Some(build_formatted_message_draft(
        caption,
        MentionIntent::default(),
    ))
}

pub(super) fn build_download_media_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
) -> Option<CoreCommand> {
    if event_id.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::DownloadMedia {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
        selection: MediaDownloadSelection::File,
    }))
}

pub(super) fn build_load_message_source_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
) -> Option<CoreCommand> {
    if event_id.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::LoadMessageSource {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
    }))
}

pub(super) fn build_request_room_key_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
    origin: koushi_core::KeyRequestOrigin,
    timeline_key: Option<TimelineKey>,
) -> Option<CoreCommand> {
    if event_id.trim().is_empty() {
        return None;
    }
    let key = match timeline_key {
        Some(timeline_key) => TimelineKey {
            account_key,
            kind: timeline_key.kind,
        },
        None => build_timeline_key(account_key, room_id),
    };
    Some(CoreCommand::Timeline(TimelineCommand::RequestRoomKey {
        request_id,
        key,
        event_id,
        origin,
    }))
}

pub(super) fn build_request_late_decryption_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    timeline_key: Option<TimelineKey>,
) -> Option<CoreCommand> {
    let key = match timeline_key {
        Some(timeline_key) => TimelineKey {
            account_key,
            kind: timeline_key.kind,
        },
        None => build_timeline_key(account_key, room_id),
    };
    Some(CoreCommand::Timeline(
        TimelineCommand::RequestLateDecryption { request_id, key },
    ))
}

pub(super) fn build_load_link_previews_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
) -> Option<CoreCommand> {
    if event_id.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::LoadLinkPreviews {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
    }))
}

pub(super) fn build_hide_link_preview_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
) -> Option<CoreCommand> {
    if event_id.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::HideLinkPreview {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
    }))
}

pub(super) fn build_forward_message_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    source_event_id: String,
    destination_room_id: String,
    transaction_id: String,
) -> Option<CoreCommand> {
    if source_event_id.trim().is_empty()
        || destination_room_id.trim().is_empty()
        || transaction_id.trim().is_empty()
    {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::ForwardMessage {
        request_id,
        key: build_timeline_key(account_key, room_id),
        source_event_id,
        destination_room_id,
        transaction_id,
    }))
}

pub(super) fn build_edit_message_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
    document: ComposerDocument,
) -> Option<CoreCommand> {
    if document.plain_body().trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::EditText {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
        document,
    }))
}

pub(super) fn build_redact_message_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
) -> CoreCommand {
    CoreCommand::Timeline(TimelineCommand::Redact {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
    })
}

pub(super) fn build_toggle_reaction_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
    reaction_key: String,
) -> Option<CoreCommand> {
    if reaction_key.is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::ToggleReaction {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
        reaction_key,
    }))
}

pub(super) fn build_send_reaction_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
    reaction_key: String,
) -> Option<CoreCommand> {
    if event_id.trim().is_empty() || reaction_key.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::SendReaction {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
        reaction_key,
    }))
}

pub(super) fn build_redact_reaction_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
    reaction_key: String,
    reaction_event_id: String,
) -> Option<CoreCommand> {
    if event_id.trim().is_empty()
        || reaction_key.trim().is_empty()
        || reaction_event_id.trim().is_empty()
    {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::RedactReaction {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
        reaction_key,
        reaction_event_id,
    }))
}

pub(super) fn build_send_read_receipt_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
    thread_root_event_id: Option<String>,
) -> Option<CoreCommand> {
    if event_id.trim().is_empty() {
        return None;
    }
    let key = match thread_root_event_id.filter(|root_event_id| !root_event_id.trim().is_empty()) {
        Some(root_event_id) => TimelineKey {
            account_key,
            kind: TimelineKind::Thread {
                room_id,
                root_event_id,
            },
        },
        None => build_timeline_key(account_key, room_id),
    };
    Some(CoreCommand::Timeline(TimelineCommand::SendReadReceipt {
        request_id,
        key,
        event_id,
    }))
}

pub(super) fn build_set_fully_read_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
) -> Option<CoreCommand> {
    if event_id.trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::SetFullyRead {
        request_id,
        key: build_timeline_key(account_key, room_id),
        event_id,
    }))
}

const SUBMISSION_SETTLEMENT_TIMEOUT: Duration = Duration::from_secs(10);
const PREPARED_MEDIA_QUEUE_TIMEOUT: Duration = Duration::from_secs(10);
const COMPOSER_DRAFT_ACCEPTANCE_TIMEOUT: Duration = Duration::from_secs(10);

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

fn composer_draft_revision(
    state: &koushi_state::AppState,
    target: &koushi_state::ComposerTarget,
) -> koushi_state::ComposerDraftRevision {
    match target {
        koushi_state::ComposerTarget::Main { room_id } => {
            state.composer_drafts.room_revision(room_id)
        }
        koushi_state::ComposerTarget::Thread {
            room_id,
            root_event_id,
        } => state
            .composer_drafts
            .thread_revision(room_id, root_event_id),
    }
}

fn composer_draft_last_accepted_clear_revision(
    state: &koushi_state::AppState,
    target: &koushi_state::ComposerTarget,
) -> koushi_state::ComposerDraftRevision {
    match target {
        koushi_state::ComposerTarget::Main { room_id } => state
            .composer_drafts
            .room_last_accepted_clear_revisions
            .get(room_id)
            .copied()
            .unwrap_or_default(),
        koushi_state::ComposerTarget::Thread {
            room_id,
            root_event_id,
        } => state
            .composer_drafts
            .thread_last_accepted_clear_revisions
            .get(room_id)
            .and_then(|threads| threads.get(root_event_id))
            .copied()
            .unwrap_or_default(),
    }
}

fn composer_draft_has_content(
    state: &koushi_state::AppState,
    target: &koushi_state::ComposerTarget,
) -> bool {
    match target {
        koushi_state::ComposerTarget::Main { room_id } => state
            .composer_drafts
            .rooms
            .get(room_id)
            .is_some_and(|draft| !draft.is_empty()),
        koushi_state::ComposerTarget::Thread {
            room_id,
            root_event_id,
        } => state
            .composer_drafts
            .threads
            .get(room_id)
            .and_then(|threads| threads.get(root_event_id))
            .is_some_and(|draft| !draft.is_empty()),
    }
}

fn composer_transport_tokens(
    state: &CoreRuntimeState,
    renderer_generation: &str,
    lease_id: &str,
) -> Result<
    (
        koushi_core::composer_draft_lifecycle::ComposerRendererGeneration,
        koushi_core::composer_draft_lifecycle::ComposerDraftLeaseId,
    ),
    String,
> {
    let identities = state
        .composer_draft_transport
        .lock()
        .map_err(|_| "composer draft transport unavailable".to_owned())?;
    Ok((
        identities.generation(renderer_generation)?,
        identities.lease(renderer_generation, lease_id)?,
    ))
}

fn acquire_terminal_composer_permit(
    connection: &CoreConnection,
    generation: koushi_core::composer_draft_lifecycle::ComposerRendererGeneration,
    lease_id: koushi_core::composer_draft_lifecycle::ComposerDraftLeaseId,
    account: &koushi_key::SessionKeyId,
    target: &koushi_state::ComposerTarget,
) -> Result<koushi_core::composer_draft_lifecycle::ComposerDraftCommandPermit, String> {
    connection
        .acquire_composer_draft_command_permit(
            generation,
            lease_id,
            &koushi_core::composer_draft_lifecycle::ComposerDraftScope {
                account: account.clone(),
                target: target.clone(),
            },
        )
        .map_err(|_| "composer draft lease mismatch".to_owned())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposerDraftLeaseResponse {
    renderer_generation: String,
    lease_id: String,
    revision: koushi_state::ComposerDraftRevision,
    last_accepted_clear_revision: koushi_state::ComposerDraftRevision,
    has_authoritative_content: bool,
}

#[tauri::command]
pub async fn begin_composer_draft_renderer_generation(
    state: State<'_, CoreRuntimeState>,
) -> Result<String, String> {
    let connection = state.connection.lock().await;
    let generation = connection
        .begin_composer_draft_renderer_generation()
        .map_err(|_| "composer renderer generation unavailable".to_owned())?;
    state
        .composer_draft_transport
        .lock()
        .map_err(|_| "composer draft transport unavailable".to_owned())?
        .install_generation(generation)
}

#[tauri::command]
pub async fn acquire_composer_draft_lease(
    account_homeserver: String,
    account_user_id: String,
    account_device_id: String,
    target: koushi_state::ComposerTarget,
    renderer_generation: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<ComposerDraftLeaseResponse, String> {
    if account_homeserver.is_empty() || account_user_id.is_empty() || account_device_id.is_empty() {
        return Err("composer draft owner is incomplete".to_owned());
    }
    let expected_account = koushi_key::SessionKeyId {
        homeserver: account_homeserver,
        user_id: account_user_id,
        device_id: account_device_id,
    };
    let connection = state.connection.lock().await;
    let snapshot = connection.snapshot();
    if composer_draft_session_key(&snapshot).as_ref() != Some(&expected_account)
        || !composer_target_is_active(&snapshot, &target)
    {
        return Err("composer draft lease scope is inactive".to_owned());
    }
    let generation = state
        .composer_draft_transport
        .lock()
        .map_err(|_| "composer draft transport unavailable".to_owned())?
        .generation(&renderer_generation)?;
    let lease = connection
        .acquire_composer_draft_lease(
            generation,
            koushi_core::composer_draft_lifecycle::ComposerDraftScope {
                account: expected_account,
                target: target.clone(),
            },
        )
        .map_err(|_| "composer draft lease unavailable".to_owned())?;
    let lease_id = state
        .composer_draft_transport
        .lock()
        .map_err(|_| "composer draft transport unavailable".to_owned())?
        .install_lease(&renderer_generation, lease)?;
    Ok(ComposerDraftLeaseResponse {
        renderer_generation,
        lease_id,
        revision: composer_draft_revision(&snapshot, &target),
        last_accepted_clear_revision: composer_draft_last_accepted_clear_revision(
            &snapshot, &target,
        ),
        has_authoritative_content: composer_draft_has_content(&snapshot, &target),
    })
}

#[tauri::command]
pub async fn release_composer_draft_lease(
    lease_id: String,
    renderer_generation: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<(), String> {
    let (generation, lease) =
        composer_transport_tokens(state.inner(), &renderer_generation, &lease_id)?;
    state
        .connection
        .lock()
        .await
        .release_composer_draft_lease(generation, lease)
        .map_err(|_| "composer draft lease mismatch".to_owned())?;
    state
        .composer_draft_transport
        .lock()
        .map_err(|_| "composer draft transport unavailable".to_owned())?
        .remove_lease(&renderer_generation, &lease_id);
    Ok(())
}

fn composer_draft_session_key(state: &koushi_state::AppState) -> Option<koushi_key::SessionKeyId> {
    match &state.session {
        koushi_state::SessionState::Ready(info) => {
            Some(koushi_core::store::session_key_id_from_info(info))
        }
        _ => None,
    }
}

fn next_composer_draft_acceptance_revision(
    state: &koushi_state::AppState,
    target: &koushi_state::ComposerTarget,
    submitted_revision: koushi_state::ComposerDraftRevision,
) -> Result<koushi_state::ComposerDraftRevision, String> {
    koushi_state::ComposerDraftRevision::checked_successor(
        composer_draft_revision(state, target),
        submitted_revision,
    )
    .map_err(|_| "composer draft revision exhausted".to_owned())
}

async fn wait_for_composer_draft_acceptance<S: SubmissionEventSource>(
    source: &mut S,
    request_id: koushi_core::RequestId,
    target: &koushi_state::ComposerTarget,
    expected_revision: koushi_state::ComposerDraftRevision,
    timeout: Duration,
) -> Result<koushi_state::ComposerDraftRevision, String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let revision = composer_draft_revision(&source.snapshot(), target);
        if revision >= expected_revision {
            return Ok(revision);
        }
        let terminal_failure = match tokio::time::timeout_at(deadline, source.recv_event()).await {
            Ok(Ok(koushi_core::CoreEvent::OperationFailed {
                request_id: failed_request_id,
                ..
            })) if failed_request_id == request_id => {
                "composer draft acceptance was rejected".to_owned()
            }
            // Issue #450: schedule-time slash rejections are keyed events
            // carrying the request id; terminate the wait immediately.
            Ok(Ok(koushi_core::CoreEvent::Room(
                koushi_core::event::RoomEvent::ComposerSlashCommandRejected {
                    request_id: rejected_request_id,
                    ..
                },
            ))) if rejected_request_id == request_id => {
                "composer draft acceptance was rejected".to_owned()
            }
            Ok(Ok(_)) => continue,
            Ok(Err(lag)) if lag.skipped == 0 => "composer draft acceptance disconnected".to_owned(),
            Ok(Err(_)) => "composer draft acceptance event stream lagged".to_owned(),
            Err(_) => "composer draft acceptance did not settle".to_owned(),
        };
        // Broadcast delivery is only a wake-up hint. The reducer snapshot is
        // authoritative and may already contain the acceptance even when the
        // event was lagged, disconnected, or raced the deadline.
        let revision = composer_draft_revision(&source.snapshot(), target);
        if revision >= expected_revision {
            return Ok(revision);
        }
        return Err(terminal_failure);
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
            let registry = &snapshot.timeline.submission_registry;
            if registry.accepted_submission_ids.contains(submission_id)
                || registry.settled_submission_ids.contains(submission_id)
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

async fn wait_for_prepared_media_queue<S: SubmissionEventSource>(
    source: &mut S,
    request_id: RequestId,
    transaction_id: &str,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let event = tokio::time::timeout_at(deadline, source.recv_event())
            .await
            .map_err(|_| "prepared upload queue admission did not settle".to_owned())?;
        match event {
            Ok(CoreEvent::Timeline(TimelineEvent::MediaSendQueued {
                request_id: queued_request_id,
                transaction_id: queued_transaction_id,
                ..
            })) if queued_request_id == request_id && queued_transaction_id == transaction_id => {
                return Ok(());
            }
            Ok(CoreEvent::OperationFailed {
                request_id: failed_request_id,
                failure,
            }) if failed_request_id == request_id => {
                return Err(invoke_error_from_core_failure(
                    "prepared upload send failed",
                    failure,
                ));
            }
            Ok(_) => {}
            Err(EventStreamLag { skipped: 0 }) => {
                return Err("prepared upload send disconnected".to_owned());
            }
            Err(_) => return Err("prepared upload send event stream lagged".to_owned()),
        }
    }
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
    account_homeserver: String,
    account_user_id: String,
    account_device_id: String,
    lease_id: String,
    renderer_generation: String,
    submission_id: String,
    room_id: String,
    document: koushi_state::ComposerDocument,
    draft_revision: koushi_state::ComposerDraftRevision,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<SubmissionResponse, SubmissionFailure> {
    if document.plain_body().trim().is_empty() {
        return Err(SubmissionFailure::Invalid);
    }

    let transaction_id = format!(
        "desktop-{}",
        NEXT_TRANSACTION_ID.fetch_add(1, Ordering::Relaxed)
    );
    let expected_account = koushi_key::SessionKeyId {
        homeserver: account_homeserver,
        user_id: account_user_id,
        device_id: account_device_id,
    };
    let (generation, lease) =
        composer_transport_tokens(state.inner(), &renderer_generation, &lease_id)
            .map_err(|_| SubmissionFailure::SubmitFailed)?;
    let mut event_conn = state.runtime.attach();
    if composer_draft_session_key(&event_conn.snapshot()).as_ref() != Some(&expected_account) {
        return Err(SubmissionFailure::SubmitFailed);
    }
    let target = koushi_state::ComposerTarget::Main {
        room_id: room_id.clone(),
    };
    let _terminal_permit = acquire_terminal_composer_permit(
        &event_conn,
        generation,
        lease,
        &expected_account,
        &target,
    )
    .map_err(|_| SubmissionFailure::SubmitFailed)?;
    let request_id = event_conn.next_request_id();
    let account_key = account_key_from_app_state(&event_conn.snapshot());
    let submission_id = SubmissionId::new(submission_id);
    if let Some(command) = build_submit_text_command(
        request_id,
        expected_account,
        submission_id.clone(),
        account_key,
        room_id,
        transaction_id,
        document,
        draft_revision,
    ) {
        event_conn
            .command_with_composer_lease(generation, lease, command)
            .await
            .map_err(|_| SubmissionFailure::SubmitFailed)?;
    }
    let response = wait_for_submission_settlement(&mut event_conn, submission_id).await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(response)
}

#[tauri::command]
pub async fn schedule_send(
    account_homeserver: String,
    account_user_id: String,
    account_device_id: String,
    lease_id: String,
    renderer_generation: String,
    target: koushi_state::ComposerTarget,
    body: String,
    send_at_ms: u64,
    draft_revision: koushi_state::ComposerDraftRevision,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<ComposerDraftAcceptanceResponse, String> {
    let (generation, lease) =
        composer_transport_tokens(state.inner(), &renderer_generation, &lease_id)?;
    let mut event_conn = state.runtime.attach();
    let expected_account = koushi_key::SessionKeyId {
        homeserver: account_homeserver,
        user_id: account_user_id,
        device_id: account_device_id,
    };
    if composer_draft_session_key(&event_conn.snapshot()).as_ref() != Some(&expected_account) {
        return Err("composer operation owner changed".to_owned());
    }
    let expected_revision =
        next_composer_draft_acceptance_revision(&event_conn.snapshot(), &target, draft_revision)?;
    let _terminal_permit = acquire_terminal_composer_permit(
        &event_conn,
        generation,
        lease,
        &expected_account,
        &target,
    )?;
    let request_id = event_conn.next_request_id();
    let accepted_revision = if let Some(command) = build_schedule_send_command(
        request_id,
        expected_account,
        target.clone(),
        body,
        send_at_ms,
        draft_revision,
    ) {
        event_conn
            .command_with_composer_lease(generation, lease, command)
            .await
            .map_err(|error| format!("command submit failed: {error}"))?;
        Some(
            wait_for_composer_draft_acceptance(
                &mut event_conn,
                request_id,
                &target,
                expected_revision,
                COMPOSER_DRAFT_ACCEPTANCE_TIMEOUT,
            )
            .await?,
        )
    } else {
        None
    };
    update_qa_window_title_from_state(&app, state.inner()).await;
    let snapshot = event_conn.versioned_snapshot();
    Ok(ComposerDraftAcceptanceResponse {
        accepted_revision,
        snapshot: FrontendDesktopSnapshot::from_versioned(snapshot.state, snapshot.generation),
    })
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
pub async fn stage_upload_bytes(
    target: koushi_state::ComposerTarget,
    items: Vec<StageUploadBytesInputItem>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    const MAX_BATCH_BYTES: usize = 128 * 1024 * 1024;
    if items.is_empty()
        || items.len() > koushi_core::media_preparation::MAX_PREPARATION_BATCH_SIZE
        || items
            .iter()
            .try_fold(0usize, |total, item| total.checked_add(item.bytes.len()))
            .is_none_or(|total| total > MAX_BATCH_BYTES)
    {
        return Err("attachment batch is empty or exceeds the supported limit".to_owned());
    }
    let mut event_conn = state.runtime.attach();
    let initial_snapshot = event_conn.snapshot();
    if !composer_target_is_active(&initial_snapshot, &target) {
        return current_snapshot(state.inner()).await;
    }
    let initial_account = account_key_from_app_state(&initial_snapshot);
    let existing_items = staged_uploads_for_target(&initial_snapshot, &target)
        .unwrap_or_default()
        .to_vec();
    let expected_ids = items
        .iter()
        .map(|item| item.staged_id.clone())
        .collect::<Vec<_>>();
    let preparing_items = existing_items
        .iter()
        .cloned()
        .chain(items.iter().map(|item| StagedUploadItem {
            staged_id: item.staged_id.clone(),
            room_id: target.room_id().to_owned(),
            position: item.position,
            filename: item.filename.clone(),
            mime_type: normalized_attachment_mime(&item.mime_type),
            byte_count: u64::try_from(item.bytes.len()).unwrap_or(u64::MAX),
            kind: if item.mime_type.to_ascii_lowercase().starts_with("image/") {
                StagedUploadKind::Image {
                    width: None,
                    height: None,
                }
            } else {
                StagedUploadKind::File
            },
            caption: None,
            compression_choice: StagedUploadCompressionChoice::NotApplicable,
            preparation: koushi_state::StagedUploadPreparation::Preparing,
        }))
        .collect::<Vec<_>>();
    {
        let mut media = state.runtime.media_preparation().transition().await;
        media.reconcile_snapshot(&initial_snapshot);
        let preparing_request_id = event_conn.next_request_id();
        event_conn
            .command(CoreCommand::App(AppCommand::SetUploadStaging {
                request_id: preparing_request_id,
                target: target.clone(),
                items: preparing_items,
            }))
            .await
            .map_err(|error| format!("command submit failed: {error}"))?;
        wait_for_upload_staging_snapshot(
            &mut event_conn,
            preparing_request_id,
            |snapshot| {
                staged_uploads_for_target(snapshot, &target).is_some_and(|staged| {
                    staged.len() == existing_items.len() + expected_ids.len()
                        && expected_ids.iter().all(|expected_id| {
                            staged.iter().any(|item| {
                                item.staged_id == *expected_id
                                    && matches!(
                                        item.preparation,
                                        koushi_state::StagedUploadPreparation::Preparing
                                    )
                            })
                        })
                })
            },
            "upload staging did not enter preparing state",
        )
        .await?;
    }

    let snapshot = event_conn.snapshot();
    let policy = snapshot
        .settings
        .values
        .media
        .image_upload_compression_policy;
    let core_inputs = items
        .into_iter()
        .map(
            |item| koushi_core::media_preparation::StageUploadBytesInput {
                staged_id: item.staged_id,
                position: item.position,
                filename: item.filename,
                mime_type: item.mime_type,
                bytes: item.bytes,
            },
        )
        .collect();
    let preparation_target = target.clone();
    let preparation = tokio::task::spawn_blocking(move || {
        let mut registry = koushi_core::media_preparation::MediaPreparationRegistry::default();
        let items = registry.prepare_items(&preparation_target, core_inputs, policy);
        (registry, items)
    })
    .await;
    let (prepared_registry, new_prepared_items) =
        preparation.map_err(|_| "attachment preparation task did not complete".to_owned())?;
    let mut media = state.runtime.media_preparation().transition().await;
    let current = event_conn.snapshot();
    if account_key_from_app_state(&current) != initial_account
        || !composer_target_is_active(&current, &target)
    {
        return current_snapshot(state.inner()).await;
    }
    let mut prepared_by_id = new_prepared_items
        .into_iter()
        .map(|item| (item.staged_id.clone(), item))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut prepared_items = staged_uploads_for_target(&current, &target)
        .unwrap_or_default()
        .to_vec();
    for item in &mut prepared_items {
        if let Some(prepared) = prepared_by_id.remove(&item.staged_id) {
            *item = prepared;
        }
    }
    if !prepared_by_id.is_empty() {
        return current_snapshot(state.inner()).await;
    }
    media.merge_prepared(prepared_registry);

    let prepared_item_count = prepared_items.len();
    let ready_request_id = event_conn.next_request_id();
    event_conn
        .command(CoreCommand::App(AppCommand::SetUploadStaging {
            request_id: ready_request_id,
            target: target.clone(),
            items: prepared_items,
        }))
        .await
        .map_err(|error| format!("command submit failed: {error}"))?;
    wait_for_upload_staging_snapshot(
        &mut event_conn,
        ready_request_id,
        |snapshot| {
            staged_uploads_for_target(snapshot, &target).is_some_and(|staged| {
                staged.len() == prepared_item_count
                    && expected_ids.iter().all(|expected_id| {
                        staged.iter().any(|item| {
                            item.staged_id == *expected_id
                                && !matches!(
                                    item.preparation,
                                    koushi_state::StagedUploadPreparation::Preparing
                                )
                        })
                    })
            })
        },
        "upload preparation did not settle",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn select_staged_upload_output(
    target: koushi_state::ComposerTarget,
    staged_id: String,
    selection: koushi_state::StagedUploadOutputSelection,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let snapshot = state.runtime.attach().snapshot();
    if !composer_target_is_active(&snapshot, &target) {
        return current_snapshot(state.inner()).await;
    }
    let initial_account = account_key_from_app_state(&snapshot);
    let policy = snapshot
        .settings
        .values
        .media
        .image_upload_compression_policy;
    let variant_id =
        koushi_core::media_preparation::MediaPreparationRegistry::output_identity(selection);

    // Record the choice first: Rust owns which output uploads, and an
    // unprepared pair becomes `pending` under a fresh generation.
    let cached = {
        let mut media = state.runtime.media_preparation().transition().await;
        media.select_variant(&target, &staged_id, &variant_id)
    };
    let mut event_conn = state.runtime.attach();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(CoreCommand::App(AppCommand::SelectStagedUploadOutput {
            request_id,
            target: target.clone(),
            staged_id: staged_id.clone(),
            selection,
        }))
        .await
        .map_err(|error| format!("command submit failed: {error}"))?;
    let selection_matches = |snapshot: &koushi_state::AppState| {
        staged_uploads_for_target(snapshot, &target).is_some_and(|items| {
            items.iter().any(|item| {
                item.staged_id == staged_id
                    && matches!(
                        &item.preparation,
                        koushi_state::StagedUploadPreparation::Ready { selected, .. }
                            if selected == &selection
                    )
            })
        })
    };
    wait_for_upload_staging_snapshot(
        &mut event_conn,
        request_id,
        selection_matches,
        "staged upload output selection did not update",
    )
    .await?;
    if cached {
        return current_snapshot(state.inner()).await;
    }

    // Encode the newly requested pair. The generation captured here is what
    // makes a slower encode lose to a newer selection.
    let Some((generation, source)) =
        staged_output_generation(&event_conn.snapshot(), &target, &staged_id).zip(
            state
                .runtime
                .media_preparation()
                .transition()
                .await
                .source_input(&target, &staged_id),
        )
    else {
        return current_snapshot(state.inner()).await;
    };
    let encoded = tokio::task::spawn_blocking(move || {
        koushi_core::media_preparation::MediaPreparationRegistry::encode_output(
            &source, selection, policy,
        )
    })
    .await
    .map_err(|_| "attachment output encoding did not complete".to_owned())?;
    let Some((descriptor, bytes)) = encoded else {
        return current_snapshot(state.inner()).await;
    };

    let current = state.runtime.attach().snapshot();
    if account_key_from_app_state(&current) != initial_account
        || !composer_target_is_active(&current, &target)
    {
        return current_snapshot(state.inner()).await;
    }
    let Some(item) = staged_uploads_for_target(&current, &target).and_then(|items| {
        items
            .iter()
            .find(|item| item.staged_id == staged_id)
            .cloned()
    }) else {
        return current_snapshot(state.inner()).await;
    };
    // The state-owned fence decides whether this result is still wanted.
    let Some(replacement) = koushi_state::staged_upload_item_with_completed_output(
        &item,
        descriptor.clone(),
        generation,
    ) else {
        return current_snapshot(state.inner()).await;
    };
    state
        .runtime
        .media_preparation()
        .transition()
        .await
        .insert_prepared_output(&target, &staged_id, descriptor, bytes);
    replace_staged_upload_item(state.inner(), &target, &staged_id, replacement).await?;
    current_snapshot(state.inner()).await
}

/// Generation of the staged item's current output selection.
fn staged_output_generation(
    snapshot: &koushi_state::AppState,
    target: &koushi_state::ComposerTarget,
    staged_id: &str,
) -> Option<u64> {
    staged_uploads_for_target(snapshot, target)?
        .iter()
        .find(|item| item.staged_id == staged_id)
        .and_then(|item| match &item.preparation {
            koushi_state::StagedUploadPreparation::Ready { generation, .. } => Some(*generation),
            _ => None,
        })
}

#[tauri::command]
pub async fn retry_staged_upload_preparation(
    target: koushi_state::ComposerTarget,
    staged_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let snapshot = state.runtime.attach().snapshot();
    if !composer_target_is_active(&snapshot, &target) {
        return current_snapshot(state.inner()).await;
    }
    let initial_account = account_key_from_app_state(&snapshot);
    let policy = snapshot
        .settings
        .values
        .media
        .image_upload_compression_policy;
    let Some(source) = state
        .runtime
        .media_preparation()
        .transition()
        .await
        .source_input(&target, &staged_id)
    else {
        return current_snapshot(state.inner()).await;
    };
    let retry_target = target.clone();
    let retry = tokio::task::spawn_blocking(move || {
        let mut registry = koushi_core::media_preparation::MediaPreparationRegistry::default();
        let replacement = registry
            .prepare_items(&retry_target, vec![source], policy)
            .into_iter()
            .next();
        (registry, replacement)
    })
    .await;
    let (prepared_registry, replacement) =
        retry.map_err(|_| "attachment preparation task did not complete".to_owned())?;
    let mut media = state.runtime.media_preparation().transition().await;
    let current = state.runtime.attach().snapshot();
    if account_key_from_app_state(&current) != initial_account
        || !composer_target_is_active(&current, &target)
        || !staged_uploads_for_target(&current, &target).is_some_and(|items| {
            items.iter().any(|item| {
                item.staged_id == staged_id
                    && matches!(
                        item.preparation,
                        koushi_state::StagedUploadPreparation::Failed { .. }
                    )
            })
        })
    {
        return current_snapshot(state.inner()).await;
    }
    if let Some(replacement) = replacement {
        media.remove_item(&target, &staged_id);
        media.merge_prepared(prepared_registry);
        replace_staged_upload_item(state.inner(), &target, &staged_id, replacement).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn use_original_staged_upload(
    target: koushi_state::ComposerTarget,
    staged_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut media = state.runtime.media_preparation().transition().await;
    if !composer_target_is_active(&state.runtime.attach().snapshot(), &target) {
        return current_snapshot(state.inner()).await;
    }
    let replacement = media.use_original(&target, &staged_id);
    if let Some(replacement) = replacement {
        replace_staged_upload_item(state.inner(), &target, &staged_id, replacement).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

async fn replace_staged_upload_item(
    state: &CoreRuntimeState,
    target: &koushi_state::ComposerTarget,
    staged_id: &str,
    replacement: StagedUploadItem,
) -> Result<(), String> {
    let mut event_conn = state.runtime.attach();
    if !composer_target_is_active(&event_conn.snapshot(), target) {
        return Ok(());
    }
    let mut items = staged_uploads_for_target(&event_conn.snapshot(), target)
        .unwrap_or_default()
        .to_vec();
    let Some(item) = items.iter_mut().find(|item| item.staged_id == staged_id) else {
        return Ok(());
    };
    *item = replacement;
    let request_id = event_conn.next_request_id();
    event_conn
        .command(CoreCommand::App(AppCommand::SetUploadStaging {
            request_id,
            target: target.clone(),
            items,
        }))
        .await
        .map_err(|error| format!("command submit failed: {error}"))?;
    wait_for_upload_staging_snapshot(
        &mut event_conn,
        request_id,
        |snapshot| {
            staged_uploads_for_target(snapshot, target).is_some_and(|items| {
                items.iter().any(|item| {
                    item.staged_id == staged_id
                        && !matches!(
                            item.preparation,
                            koushi_state::StagedUploadPreparation::Preparing
                        )
                })
            })
        },
        "upload preparation recovery did not settle",
    )
    .await
}

async fn publish_staged_upload_items(
    event_conn: &mut CoreConnection,
    target: &koushi_state::ComposerTarget,
    items: Vec<StagedUploadItem>,
) -> Result<(), String> {
    let expected_ids = items
        .iter()
        .map(|item| item.staged_id.clone())
        .collect::<Vec<_>>();
    let request_id = event_conn.next_request_id();
    event_conn
        .command(CoreCommand::App(AppCommand::SetUploadStaging {
            request_id,
            target: target.clone(),
            items,
        }))
        .await
        .map_err(|error| format!("command submit failed: {error}"))?;
    wait_for_upload_staging_snapshot(
        event_conn,
        request_id,
        |snapshot| {
            staged_uploads_for_target(snapshot, target).is_some_and(|staged| {
                staged.len() == expected_ids.len()
                    && expected_ids
                        .iter()
                        .all(|expected_id| staged.iter().any(|item| item.staged_id == *expected_id))
            })
        },
        "prepared upload staging did not settle",
    )
    .await
}

#[tauri::command]
pub async fn prepared_upload_preview(
    target: koushi_state::ComposerTarget,
    staged_id: String,
    variant_id: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<Vec<u8>, String> {
    let media = state.runtime.media_preparation().transition().await;
    if !composer_target_is_active(&state.runtime.attach().snapshot(), &target) {
        return Err("prepared upload preview is unavailable".to_owned());
    }
    media
        .variant_bytes(&target, &staged_id, &variant_id)
        .ok_or_else(|| "prepared upload preview is unavailable".to_owned())
}

#[tauri::command]
pub async fn send_prepared_uploads(
    account_homeserver: String,
    account_user_id: String,
    account_device_id: String,
    lease_id: String,
    renderer_generation: String,
    target: koushi_state::ComposerTarget,
    draft_revision: koushi_state::ComposerDraftRevision,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<ComposerDraftAcceptanceResponse, String> {
    let snapshot = state.runtime.attach().snapshot();
    let expected_account = koushi_key::SessionKeyId {
        homeserver: account_homeserver,
        user_id: account_user_id,
        device_id: account_device_id,
    };
    if composer_draft_session_key(&snapshot).as_ref() != Some(&expected_account) {
        return Ok(ComposerDraftAcceptanceResponse {
            accepted_revision: None,
            snapshot: current_snapshot(state.inner()).await?,
        });
    }
    if !composer_target_is_active(&snapshot, &target) {
        return Ok(ComposerDraftAcceptanceResponse {
            accepted_revision: None,
            snapshot: current_snapshot(state.inner()).await?,
        });
    }
    let staged_items = staged_uploads_for_target(&snapshot, &target)
        .unwrap_or_default()
        .to_vec();
    if staged_items.is_empty() || !koushi_state::staged_uploads_are_sendable(&staged_items) {
        return Ok(ComposerDraftAcceptanceResponse {
            accepted_revision: None,
            snapshot: current_snapshot(state.inner()).await?,
        });
    }
    // Validate the eventual acceptance fence before the first upload. Revision
    // exhaustion must be a side-effect-free rejection: no Matrix upload, send,
    // or local prepared-item removal may happen before this check.
    let expected_revision =
        next_composer_draft_acceptance_revision(&snapshot, &target, draft_revision)?;
    let (generation, lease) =
        composer_transport_tokens(state.inner(), &renderer_generation, &lease_id)?;

    let account_key = account_key_from_app_state(&snapshot);
    let key = timeline_key_for_composer_target(account_key.clone(), &target);
    let mut event_conn = state.runtime.attach();
    let _terminal_permit = acquire_terminal_composer_permit(
        &event_conn,
        generation,
        lease,
        &expected_account,
        &target,
    )?;
    for item in &staged_items {
        let prepared = {
            state
                .runtime
                .media_preparation()
                .transition()
                .await
                .selected_upload(&target, &item.staged_id)
                .ok_or_else(|| "selected prepared upload bytes are unavailable".to_owned())?
        };
        let request_id = event_conn.next_request_id();
        let transaction_id = format!(
            "desktop-prepared-media-{}",
            NEXT_TRANSACTION_ID.fetch_add(1, Ordering::Relaxed)
        );
        let descriptor = prepared.descriptor;
        let kind = if descriptor.mime_type.starts_with("image/") {
            UploadMediaKind::Image {
                width: descriptor.width,
                height: descriptor.height,
            }
        } else {
            UploadMediaKind::File
        };
        event_conn
            .command(CoreCommand::Timeline(TimelineCommand::UploadAndSendMedia {
                request_id,
                expected_account: expected_account.clone(),
                key: key.clone(),
                transaction_id: transaction_id.clone(),
                request: UploadMediaRequest {
                    filename: descriptor.filename,
                    mime_type: descriptor.mime_type,
                    bytes: prepared.bytes,
                    kind,
                    compression: None,
                    thumbnail: None,
                    caption: item.caption.clone(),
                },
            }))
            .await
            .map_err(|error| format!("command submit failed: {error}"))?;
        wait_for_prepared_media_queue(
            &mut event_conn,
            request_id,
            &transaction_id,
            PREPARED_MEDIA_QUEUE_TIMEOUT,
        )
        .await?;
        let mut media = state.runtime.media_preparation().transition().await;
        media.remove_item(&target, &item.staged_id);
        let current = event_conn.snapshot();
        if composer_draft_session_key(&current).as_ref() != Some(&expected_account) {
            return Ok(ComposerDraftAcceptanceResponse {
                accepted_revision: None,
                snapshot: current_snapshot(state.inner()).await?,
            });
        }
        let mut remaining_items = staged_uploads_for_target(&current, &target)
            .unwrap_or_default()
            .to_vec();
        remaining_items.retain(|candidate| candidate.staged_id != item.staged_id);
        if composer_target_is_active(&current, &target) {
            publish_staged_upload_items(&mut event_conn, &target, remaining_items).await?;
        }
    }
    let request_id = event_conn.next_request_id();
    event_conn
        .command_with_composer_lease(
            generation,
            lease,
            CoreCommand::App(AppCommand::AcceptComposerDraft {
                request_id,
                expected_account,
                target: target.clone(),
                submitted_revision: draft_revision,
            }),
        )
        .await
        .map_err(|error| format!("command submit failed: {error}"))?;
    let accepted_revision = wait_for_composer_draft_acceptance(
        &mut event_conn,
        request_id,
        &target,
        expected_revision,
        COMPOSER_DRAFT_ACCEPTANCE_TIMEOUT,
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    let snapshot = event_conn.versioned_snapshot();
    Ok(ComposerDraftAcceptanceResponse {
        accepted_revision: Some(accepted_revision),
        snapshot: FrontendDesktopSnapshot::from_versioned(snapshot.state, snapshot.generation),
    })
}

fn timeline_key_for_composer_target(
    account_key: koushi_core::AccountKey,
    target: &koushi_state::ComposerTarget,
) -> koushi_core::TimelineKey {
    match target {
        koushi_state::ComposerTarget::Main { room_id } => {
            build_timeline_key(account_key, room_id.clone())
        }
        koushi_state::ComposerTarget::Thread {
            room_id,
            root_event_id,
        } => koushi_core::TimelineKey {
            account_key,
            kind: koushi_core::TimelineKind::Thread {
                room_id: room_id.clone(),
                root_event_id: root_event_id.clone(),
            },
        },
    }
}

fn normalized_attachment_mime(mime_type: &str) -> String {
    match mime_type.trim() {
        "" => "application/octet-stream".to_owned(),
        value => value.to_owned(),
    }
}

fn composer_target_is_active(
    snapshot: &koushi_state::AppState,
    target: &koushi_state::ComposerTarget,
) -> bool {
    match target {
        koushi_state::ComposerTarget::Main { room_id } => {
            snapshot.timeline.room_id.as_deref() == Some(room_id.as_str())
        }
        koushi_state::ComposerTarget::Thread {
            room_id,
            root_event_id,
        } => matches!(
            &snapshot.thread,
            koushi_state::ThreadPaneState::Open {
                room_id: open_room_id,
                root_event_id: open_root_event_id,
                ..
            } if open_room_id == room_id && open_root_event_id == root_event_id
        ),
    }
}

fn staged_uploads_for_target<'a>(
    snapshot: &'a koushi_state::AppState,
    target: &koushi_state::ComposerTarget,
) -> Option<&'a [StagedUploadItem]> {
    match target {
        koushi_state::ComposerTarget::Main { room_id }
            if snapshot.timeline.room_id.as_deref() == Some(room_id.as_str()) =>
        {
            Some(&snapshot.timeline.staged_uploads)
        }
        koushi_state::ComposerTarget::Thread {
            room_id,
            root_event_id,
        } => match &snapshot.thread {
            koushi_state::ThreadPaneState::Open {
                room_id: open_room_id,
                root_event_id: open_root_event_id,
                staged_uploads,
                ..
            } if open_room_id == room_id && open_root_event_id == root_event_id => {
                Some(staged_uploads)
            }
            _ => None,
        },
        _ => None,
    }
}

#[tauri::command]
pub async fn update_staged_upload_caption(
    target: koushi_state::ComposerTarget,
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
    if !composer_target_is_active(&event_conn.snapshot(), &target) {
        return current_snapshot(state.inner()).await;
    }
    let request_id = event_conn.next_request_id();
    event_conn
        .command(CoreCommand::App(AppCommand::UpdateStagedUploadCaption {
            request_id,
            target: target.clone(),
            staged_id,
            caption,
        }))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_upload_staging_snapshot(
        &mut event_conn,
        request_id,
        |snapshot| {
            staged_uploads_for_target(snapshot, &target)
                .unwrap_or_default()
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
    let Some(room_id) = event_conn.snapshot().timeline.room_id else {
        return current_snapshot(state.inner()).await;
    };
    event_conn
        .command(CoreCommand::App(
            AppCommand::UpdateStagedUploadCompression {
                request_id,
                target: koushi_state::ComposerTarget::Main { room_id },
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
    target: koushi_state::ComposerTarget,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let mut media = state.runtime.media_preparation().transition().await;
    let mut event_conn = state.runtime.attach();
    if !composer_target_is_active(&event_conn.snapshot(), &target) {
        return current_snapshot(state.inner()).await;
    }
    let request_id = event_conn.next_request_id();
    event_conn
        .command(CoreCommand::App(AppCommand::ClearUploadStaging {
            request_id,
            target: target.clone(),
        }))
        .await
        .map_err(|e| format!("command submit failed: {e}"))?;
    wait_for_upload_staging_snapshot(
        &mut event_conn,
        request_id,
        |snapshot| {
            staged_uploads_for_target(snapshot, &target).is_some_and(|items| items.is_empty())
        },
        "upload staging did not clear",
    )
    .await?;
    media.clear_target(&target);
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
    body: String,
    send_at_ms: u64,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) =
        build_reschedule_scheduled_send_command(request_id, scheduled_id, body, send_at_ms)
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
    let snapshot = state.runtime.attach().snapshot();
    let Some(expected_account) = composer_draft_session_key(&snapshot) else {
        return current_snapshot(state.inner()).await;
    };
    let account_key = account_key_from_app_state(&snapshot);
    let (image_compression_mode, image_compression_policy) =
        image_upload_compression_contract_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) = build_upload_media_command(
        request_id,
        expected_account,
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
    origin: Option<koushi_core::KeyRequestOrigin>,
    timeline_key: Option<TimelineKey>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    // Only absent origin defaults to User; unknown wire values are rejected by
    // the typed deserializer instead of being silently coerced.
    let origin = origin.unwrap_or(koushi_core::KeyRequestOrigin::User);
    if let Some(command) = build_request_room_key_command(
        request_id,
        account_key,
        room_id,
        event_id,
        origin,
        timeline_key,
    ) {
        submit_core_command(state.inner(), command).await?;
    }
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

/// Trigger the bounded local late-decryption retry for the given room's
/// visible timeline (issue #476). Requests no new keys and redistributes
/// nothing.
#[tauri::command]
pub async fn request_late_decryption(
    room_id: String,
    timeline_key: Option<TimelineKey>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) =
        build_request_late_decryption_command(request_id, account_key, room_id, timeline_key)
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
    document: koushi_state::ComposerDocument,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    if document.plain_body().trim().is_empty() {
        return current_snapshot(state.inner()).await;
    }
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    if let Some(command) =
        build_edit_message_command(request_id, account_key, room_id, event_id, document)
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
    account_homeserver: String,
    account_user_id: String,
    account_device_id: String,
    lease_id: String,
    renderer_generation: String,
    room_id: String,
    document: koushi_state::ComposerDocument,
    draft_revision: koushi_state::ComposerDraftRevision,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let (generation, lease) =
        composer_transport_tokens(state.inner(), &renderer_generation, &lease_id)?;
    let event_conn = state.runtime.attach();
    let expected_account = koushi_key::SessionKeyId {
        homeserver: account_homeserver,
        user_id: account_user_id,
        device_id: account_device_id,
    };
    let target = koushi_state::ComposerTarget::Main {
        room_id: room_id.clone(),
    };
    let _terminal_permit = acquire_terminal_composer_permit(
        &event_conn,
        generation,
        lease,
        &expected_account,
        &target,
    )?;
    let request_id = event_conn.next_request_id();
    event_conn
        .command_with_composer_lease(
            generation,
            lease,
            build_set_composer_draft_command(
                request_id,
                expected_account,
                room_id,
                document,
                draft_revision,
            ),
        )
        .await
        .map_err(|error| format!("command submit failed: {error}"))?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn set_thread_composer_draft(
    account_homeserver: String,
    account_user_id: String,
    account_device_id: String,
    lease_id: String,
    renderer_generation: String,
    room_id: String,
    root_event_id: String,
    document: koushi_state::ComposerDocument,
    draft_revision: koushi_state::ComposerDraftRevision,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendDesktopSnapshot, String> {
    let (generation, lease) =
        composer_transport_tokens(state.inner(), &renderer_generation, &lease_id)?;
    let event_conn = state.runtime.attach();
    let expected_account = koushi_key::SessionKeyId {
        homeserver: account_homeserver,
        user_id: account_user_id,
        device_id: account_device_id,
    };
    let target = koushi_state::ComposerTarget::Thread {
        room_id: room_id.clone(),
        root_event_id: root_event_id.clone(),
    };
    let _terminal_permit = acquire_terminal_composer_permit(
        &event_conn,
        generation,
        lease,
        &expected_account,
        &target,
    )?;
    let request_id = event_conn.next_request_id();
    event_conn
        .command_with_composer_lease(
            generation,
            lease,
            build_set_thread_composer_draft_command(
                request_id,
                expected_account,
                room_id,
                root_event_id,
                document,
                draft_revision,
            ),
        )
        .await
        .map_err(|error| format!("command submit failed: {error}"))?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    current_snapshot(state.inner()).await
}

#[tauri::command]
pub async fn send_reply(
    account_homeserver: String,
    account_user_id: String,
    account_device_id: String,
    lease_id: String,
    renderer_generation: String,
    submission_id: String,
    room_id: String,
    in_reply_to_event_id: String,
    document: koushi_state::ComposerDocument,
    draft_revision: koushi_state::ComposerDraftRevision,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<SubmissionResponse, SubmissionFailure> {
    if document.plain_body().trim().is_empty() {
        return Err(SubmissionFailure::Invalid);
    }

    let transaction_id = format!(
        "desktop-{}",
        NEXT_TRANSACTION_ID.fetch_add(1, Ordering::Relaxed)
    );
    let expected_account = koushi_key::SessionKeyId {
        homeserver: account_homeserver,
        user_id: account_user_id,
        device_id: account_device_id,
    };
    let (generation, lease) =
        composer_transport_tokens(state.inner(), &renderer_generation, &lease_id)
            .map_err(|_| SubmissionFailure::SubmitFailed)?;
    let mut event_conn = state.runtime.attach();
    if composer_draft_session_key(&event_conn.snapshot()).as_ref() != Some(&expected_account) {
        return Err(SubmissionFailure::SubmitFailed);
    }
    let target = koushi_state::ComposerTarget::Main {
        room_id: room_id.clone(),
    };
    let _terminal_permit = acquire_terminal_composer_permit(
        &event_conn,
        generation,
        lease,
        &expected_account,
        &target,
    )
    .map_err(|_| SubmissionFailure::SubmitFailed)?;
    let request_id = event_conn.next_request_id();
    let account_key = account_key_from_app_state(&event_conn.snapshot());
    let submission_id = SubmissionId::new(submission_id);
    if let Some(command) = build_submit_reply_command(
        request_id,
        expected_account,
        submission_id.clone(),
        account_key,
        room_id,
        transaction_id,
        in_reply_to_event_id,
        document,
        draft_revision,
    ) {
        event_conn
            .command_with_composer_lease(generation, lease, command)
            .await
            .map_err(|_| SubmissionFailure::SubmitFailed)?;
    }
    let response = wait_for_submission_settlement(&mut event_conn, submission_id).await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(response)
}

#[tauri::command]
pub async fn send_thread_reply(
    account_homeserver: String,
    account_user_id: String,
    account_device_id: String,
    lease_id: String,
    renderer_generation: String,
    submission_id: String,
    room_id: String,
    root_event_id: String,
    document: koushi_state::ComposerDocument,
    draft_revision: koushi_state::ComposerDraftRevision,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<SubmissionResponse, SubmissionFailure> {
    if document.plain_body().trim().is_empty() {
        return Err(SubmissionFailure::Invalid);
    }

    let transaction_id = format!(
        "desktop-{}",
        NEXT_TRANSACTION_ID.fetch_add(1, Ordering::Relaxed)
    );
    let expected_account = koushi_key::SessionKeyId {
        homeserver: account_homeserver,
        user_id: account_user_id,
        device_id: account_device_id,
    };
    let (generation, lease) =
        composer_transport_tokens(state.inner(), &renderer_generation, &lease_id)
            .map_err(|_| SubmissionFailure::SubmitFailed)?;
    let mut event_conn = state.runtime.attach();
    if composer_draft_session_key(&event_conn.snapshot()).as_ref() != Some(&expected_account) {
        return Err(SubmissionFailure::SubmitFailed);
    }
    let target = koushi_state::ComposerTarget::Thread {
        room_id: room_id.clone(),
        root_event_id: root_event_id.clone(),
    };
    let _terminal_permit = acquire_terminal_composer_permit(
        &event_conn,
        generation,
        lease,
        &expected_account,
        &target,
    )
    .map_err(|_| SubmissionFailure::SubmitFailed)?;
    let request_id = event_conn.next_request_id();
    let account_key = account_key_from_app_state(&event_conn.snapshot());
    let submission_id = SubmissionId::new(submission_id);
    if let Some(command) = build_submit_thread_reply_command(
        request_id,
        expected_account,
        submission_id.clone(),
        account_key,
        room_id,
        root_event_id,
        transaction_id,
        document,
        draft_revision,
    ) {
        event_conn
            .command_with_composer_lease(generation, lease, command)
            .await
            .map_err(|_| SubmissionFailure::SubmitFailed)?;
    }
    let response = wait_for_submission_settlement(&mut event_conn, submission_id).await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(response)
}

const UPLOAD_STAGING_EVENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ComposerDraftAcceptanceResponse {
    pub accepted_revision: Option<ComposerDraftRevision>,
    pub snapshot: FrontendDesktopSnapshot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SubmissionFailure {
    Invalid,
    SubmitFailed,
    Timeout,
    Disconnected,
    Lagged,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SubmissionOutcome {
    Accepted,
    Rejected {
        kind: koushi_core::TimelineFailureKind,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubmissionResponse {
    pub outcome: SubmissionOutcome,
    pub submission_id: SubmissionId,
    pub transaction_id: Option<String>,
    pub snapshot: FrontendDesktopSnapshot,
}

#[cfg(test)]
pub(super) fn build_send_reply_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    transaction_id: String,
    in_reply_to_event_id: String,
    document: ComposerDocument,
) -> Option<CoreCommand> {
    if document.plain_body().trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::SendReply {
        request_id,
        key: build_timeline_key(account_key, room_id),
        transaction_id,
        in_reply_to_event_id,
        document,
    }))
}

#[cfg(test)]
pub(super) fn build_send_thread_reply_command(
    request_id: koushi_core::RequestId,
    account_key: AccountKey,
    room_id: String,
    root_event_id: String,
    transaction_id: String,
    document: ComposerDocument,
) -> Option<CoreCommand> {
    if document.plain_body().trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::SendReply {
        request_id,
        key: TimelineKey {
            account_key,
            kind: TimelineKind::Thread {
                room_id,
                root_event_id: root_event_id.clone(),
            },
        },
        transaction_id,
        in_reply_to_event_id: root_event_id,
        document,
    }))
}

pub(super) fn build_set_composer_draft_command(
    request_id: koushi_core::RequestId,
    expected_account: koushi_key::SessionKeyId,
    room_id: String,
    document: ComposerDocument,
    revision: ComposerDraftRevision,
) -> CoreCommand {
    CoreCommand::App(AppCommand::SetComposerDraft {
        request_id,
        expected_account,
        room_id,
        document,
        revision,
    })
}

pub(super) fn build_set_thread_composer_draft_command(
    request_id: koushi_core::RequestId,
    expected_account: koushi_key::SessionKeyId,
    room_id: String,
    root_event_id: String,
    document: ComposerDocument,
    revision: ComposerDraftRevision,
) -> CoreCommand {
    CoreCommand::App(AppCommand::SetThreadComposerDraft {
        request_id,
        expected_account,
        room_id,
        root_event_id,
        document,
        revision,
    })
}

pub(super) fn build_submit_reply_command(
    request_id: RequestId,
    expected_account: koushi_key::SessionKeyId,
    submission_id: SubmissionId,
    account_key: AccountKey,
    room_id: String,
    transaction_id: String,
    in_reply_to_event_id: String,
    document: ComposerDocument,
    draft_revision: ComposerDraftRevision,
) -> Option<CoreCommand> {
    if document.plain_body().trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::SubmitReply {
        request_id,
        expected_account,
        submission_id,
        key: build_timeline_key(account_key, room_id),
        transaction_id,
        in_reply_to_event_id,
        document,
        draft_revision,
    }))
}

pub(super) fn build_submit_thread_reply_command(
    request_id: RequestId,
    expected_account: koushi_key::SessionKeyId,
    submission_id: SubmissionId,
    account_key: AccountKey,
    room_id: String,
    root_event_id: String,
    transaction_id: String,
    document: ComposerDocument,
    draft_revision: ComposerDraftRevision,
) -> Option<CoreCommand> {
    if document.plain_body().trim().is_empty() {
        return None;
    }
    Some(CoreCommand::Timeline(TimelineCommand::SubmitReply {
        request_id,
        expected_account,
        submission_id,
        key: TimelineKey {
            account_key,
            kind: TimelineKind::Thread {
                room_id,
                root_event_id: root_event_id.clone(),
            },
        },
        transaction_id,
        in_reply_to_event_id: root_event_id,
        document,
        draft_revision,
    }))
}

async fn image_upload_compression_contract_from_snapshot(
    state: &CoreRuntimeState,
) -> (ImageUploadCompressionMode, ImageUploadCompressionPolicy) {
    let media = state
        .connection
        .lock()
        .await
        .snapshot()
        .settings
        .values
        .media;
    (
        // #305 retired the stored mode. The direct upload path keeps the former
        // default so its behavior matches a user who never changed the setting.
        ImageUploadCompressionMode::default(),
        ImageUploadCompressionPolicy {
            threshold_bytes: media.image_upload_compression_policy.threshold_bytes,
            threshold_long_edge: media.image_upload_compression_policy.threshold_long_edge,
            target_long_edge: media.image_upload_compression_policy.target_long_edge,
            quality_percent: media.image_upload_compression_policy.quality_percent,
        },
    )
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

    struct DraftAcceptanceSource {
        state: koushi_state::AppState,
        target: koushi_state::ComposerTarget,
        submitted_revision: koushi_state::ComposerDraftRevision,
        pending_acceptance: bool,
        terminal_lag: Option<EventStreamLag>,
    }

    impl SubmissionEventSource for DraftAcceptanceSource {
        fn snapshot(&self) -> koushi_state::AppState {
            self.state.clone()
        }

        fn recv_event(
            &mut self,
        ) -> Pin<Box<dyn Future<Output = Result<CoreEvent, EventStreamLag>> + Send + '_>> {
            if self.pending_acceptance {
                self.pending_acceptance = false;
                match &self.target {
                    koushi_state::ComposerTarget::Main { room_id } => {
                        let _ = self
                            .state
                            .composer_drafts
                            .advance_room_revision(room_id, self.submitted_revision);
                    }
                    koushi_state::ComposerTarget::Thread {
                        room_id,
                        root_event_id,
                    } => {
                        let _ = self.state.composer_drafts.advance_thread_revision(
                            room_id,
                            root_event_id,
                            self.submitted_revision,
                        );
                    }
                }
                if let Some(lag) = self.terminal_lag.take() {
                    Box::pin(async move { Err(lag) })
                } else {
                    Box::pin(async { Ok(accepted(SubmissionId::new("draft-accept"), 99)) })
                }
            } else {
                Box::pin(std::future::pending())
            }
        }
    }

    fn accepted(id: SubmissionId, sequence: u64) -> CoreEvent {
        CoreEvent::Timeline(TimelineEvent::SubmissionAccepted {
            request_id: request_id(sequence),
            key: build_timeline_key(AccountKey("@u:test".to_owned()), "!r:test".to_owned()),
            submission_id: id,
            transaction_id: "txn".to_owned(),
        })
    }

    fn request_id(sequence: u64) -> RequestId {
        RequestId {
            connection_id: koushi_core::RuntimeConnectionId(1),
            sequence,
        }
    }

    fn media_send_queued(request_id: RequestId, transaction_id: &str) -> CoreEvent {
        CoreEvent::Timeline(TimelineEvent::MediaSendQueued {
            request_id,
            key: build_timeline_key(AccountKey("@u:test".to_owned()), "!r:test".to_owned()),
            transaction_id: transaction_id.to_owned(),
        })
    }

    #[tokio::test]
    async fn composer_acceptance_wait_is_target_keyed_after_ui_switch() {
        let targets = [
            koushi_state::ComposerTarget::Main {
                room_id: "!room-a:test".to_owned(),
            },
            koushi_state::ComposerTarget::Thread {
                room_id: "!room-a:test".to_owned(),
                root_event_id: "$root:test".to_owned(),
            },
        ];

        for target in targets {
            let mut state = koushi_state::AppState::default();
            state.timeline.room_id = Some("!room-b:test".to_owned());
            let expected_revision =
                next_composer_draft_acceptance_revision(&state, &target, 4.into())
                    .expect("revision");
            let mut source = DraftAcceptanceSource {
                state,
                target: target.clone(),
                submitted_revision: 4.into(),
                pending_acceptance: true,
                terminal_lag: None,
            };

            assert_eq!(
                wait_for_composer_draft_acceptance(
                    &mut source,
                    request_id(99),
                    &target,
                    expected_revision,
                    Duration::from_secs(1),
                )
                .await,
                Ok(expected_revision)
            );
        }
    }

    #[tokio::test]
    async fn composer_acceptance_wait_reconciles_terminal_snapshot_after_stream_failure() {
        for skipped in [0, 3] {
            let target = koushi_state::ComposerTarget::Main {
                room_id: "!room-a:test".to_owned(),
            };
            let state = koushi_state::AppState::default();
            let expected_revision =
                next_composer_draft_acceptance_revision(&state, &target, 7.into())
                    .expect("revision");
            let mut source = DraftAcceptanceSource {
                state,
                target: target.clone(),
                submitted_revision: 7.into(),
                pending_acceptance: true,
                terminal_lag: Some(EventStreamLag { skipped }),
            };

            assert_eq!(
                wait_for_composer_draft_acceptance(
                    &mut source,
                    request_id(99),
                    &target,
                    expected_revision,
                    Duration::from_secs(1),
                )
                .await,
                Ok(expected_revision)
            );
        }
    }

    #[tokio::test]
    async fn composer_acceptance_wait_stops_on_correlated_command_rejection() {
        let target = koushi_state::ComposerTarget::Main {
            room_id: "!room-a:test".to_owned(),
        };
        let rejected_request_id = request_id(99);
        let mut source = ScriptedSource {
            state: koushi_state::AppState::default(),
            events: VecDeque::from([(
                Ok(CoreEvent::OperationFailed {
                    request_id: rejected_request_id,
                    failure: koushi_core::CoreFailure::SessionRequired,
                }),
                None,
            )]),
            pending_on_empty: true,
        };

        assert_eq!(
            wait_for_composer_draft_acceptance(
                &mut source,
                rejected_request_id,
                &target,
                1.into(),
                Duration::from_secs(1),
            )
            .await,
            Err("composer draft acceptance was rejected".to_owned())
        );
    }

    #[tokio::test]
    async fn composer_acceptance_wait_stops_only_on_the_correlated_keyed_slash_rejection() {
        // Issue #450: the schedule waiter must ignore unrelated keyed
        // rejections and terminate on the matching request id.
        let target = koushi_state::ComposerTarget::Main {
            room_id: "!room-a:test".to_owned(),
        };
        let expected_request_id = request_id(42);
        let mut source = ScriptedSource {
            state: koushi_state::AppState::default(),
            events: VecDeque::from([
                (
                    Ok(CoreEvent::Room(
                        koushi_core::event::RoomEvent::ComposerSlashCommandRejected {
                            key: koushi_core::TimelineKey::room(
                                koushi_core::AccountKey("@a:test".to_owned()),
                                "!room-a:test",
                            ),
                            request_id: request_id(7),
                        },
                    )),
                    None,
                ),
                (
                    Ok(CoreEvent::Room(
                        koushi_core::event::RoomEvent::ComposerSlashCommandRejected {
                            key: koushi_core::TimelineKey::room(
                                koushi_core::AccountKey("@a:test".to_owned()),
                                "!room-a:test",
                            ),
                            request_id: expected_request_id,
                        },
                    )),
                    None,
                ),
            ]),
            pending_on_empty: true,
        };

        assert_eq!(
            wait_for_composer_draft_acceptance(
                &mut source,
                expected_request_id,
                &target,
                1.into(),
                Duration::from_secs(1),
            )
            .await,
            Err("composer draft acceptance was rejected".to_owned())
        );
        // Both events were consumed: the unrelated keyed rejection was skipped
        // (continue) and the matching one terminated the wait. If the waiter
        // terminated on ANY keyed rejection, this assertion fails.
        assert!(
            source.events.is_empty(),
            "waiter must consume the unrelated rejection before the matching one"
        );
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

    #[tokio::test]
    async fn prepared_media_wait_ignores_unrelated_queue_event_until_matching_admission() {
        let expected_request = RequestId {
            connection_id: koushi_core::RuntimeConnectionId(1),
            sequence: 8,
        };
        let unrelated_request = RequestId {
            connection_id: koushi_core::RuntimeConnectionId(1),
            sequence: 7,
        };
        let mut source = ScriptedSource {
            state: koushi_state::AppState::default(),
            events: VecDeque::from([
                (Ok(media_send_queued(unrelated_request, "other")), None),
                (Ok(media_send_queued(expected_request, "expected")), None),
            ]),
            pending_on_empty: false,
        };

        assert_eq!(
            wait_for_prepared_media_queue(
                &mut source,
                expected_request,
                "expected",
                Duration::from_secs(1),
            )
            .await,
            Ok(())
        );
    }

    #[tokio::test]
    async fn prepared_media_queue_wait_returns_matching_failure_before_cleanup() {
        let request_id = RequestId {
            connection_id: koushi_core::RuntimeConnectionId(1),
            sequence: 8,
        };
        let mut source = ScriptedSource {
            state: koushi_state::AppState::default(),
            events: VecDeque::from([(
                Ok(CoreEvent::OperationFailed {
                    request_id,
                    failure: koushi_core::CoreFailure::TimelineOperationFailed {
                        kind: koushi_core::TimelineFailureKind::Network,
                    },
                }),
                None,
            )]),
            pending_on_empty: false,
        };

        let failure = wait_for_prepared_media_queue(
            &mut source,
            request_id,
            "expected",
            Duration::from_secs(1),
        )
        .await
        .expect_err("matching failure must be terminal");
        assert!(failure.starts_with("prepared upload send failed"));
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

#[cfg(test)]
mod issue551_moved_tests {
    use super::*;
    use crate::commands::contracts::{fake_request_id, synthetic_session_key};
    use koushi_core::{AccountKey, CoreCommand, PaginationDirection, TimelineCommand};
    use koushi_state::ComposerDocument;
    fn commands_source() -> String {
        crate::commands::contracts::production_source()
    }
    #[test]
    fn acknowledge_timeline_batch_rendered_routes_every_generation_fence() {
        let source = include_str!("navigation.rs");
        let start = source
            .find("pub async fn acknowledge_timeline_batch_rendered")
            .expect("rendered batch acknowledgement command should exist");
        let command_source = &source[start..];

        for field in [
            "key: TimelineKey",
            "actor_generation: u64",
            "timeline_generation: TimelineGeneration",
            "repair_generation: u64",
            "batch_id: TimelineBatchId",
            "AppCommand::AcknowledgeTimelineBatchRendered",
        ] {
            assert!(command_source.contains(field), "missing {field}");
        }
    }

    #[test]
    fn build_subscribe_focused_timeline_command_routes_to_focused_timeline_kind() {
        let account_key = AccountKey("@alice:example.org".to_owned());
        let command = build_subscribe_focused_timeline_command(
            fake_request_id(21),
            account_key.clone(),
            "!room:example.org".to_owned(),
            "$event".to_owned(),
        );

        match command {
            CoreCommand::Timeline(TimelineCommand::Subscribe { request_id, key }) => {
                assert_eq!(request_id, fake_request_id(21));
                assert_eq!(key.account_key, account_key);
                assert_eq!(
                    key.kind,
                    koushi_core::TimelineKind::Focused {
                        room_id: "!room:example.org".to_owned(),
                        event_id: "$event".to_owned(),
                    }
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn composer_key_resolver_command_contract_is_present() {
        let commands_source = commands_source();
        let lib_source = include_str!("../lib.rs");
        let command_name = "pub async fn resolve_composer_key_action";
        let route_name = "koushi_state::resolve_composer_key_action";
        let settings_token = "settings.values.keyboard.composer_send_shortcut";
        let registration_name = "commands::timeline::resolve_composer_key_action";

        assert!(
            commands_source.contains(command_name),
            "Tauri command should expose resolve_composer_key_action"
        );
        assert!(
            commands_source.contains(route_name),
            "Tauri command should route through the Rust-owned resolver"
        );
        assert!(
            commands_source.contains(settings_token),
            "resolver should derive the send shortcut from Rust-owned settings"
        );
        assert!(
            lib_source.contains(registration_name),
            "Tauri command should be registered in generate_handler"
        );
    }

    #[test]
    fn hide_link_preview_tauri_command_contract_is_present() {
        let request_id = koushi_core::RequestId {
            connection_id: koushi_core::RuntimeConnectionId(1),
            sequence: 1,
        };
        let command = build_hide_link_preview_command(
            request_id,
            AccountKey("@u:example.test".to_owned()),
            "!room:example.test".to_owned(),
            "$event:example.test".to_owned(),
        );
        assert!(matches!(
            command,
            Some(CoreCommand::Timeline(
                TimelineCommand::HideLinkPreview { .. }
            ))
        ));
    }

    #[test]
    fn load_link_previews_tauri_command_contract_is_present() {
        let request_id = koushi_core::RequestId {
            connection_id: koushi_core::RuntimeConnectionId(1),
            sequence: 1,
        };
        let command = build_load_link_previews_command(
            request_id,
            AccountKey("@u:example.test".to_owned()),
            "!room:example.test".to_owned(),
            "$event:example.test".to_owned(),
        );
        assert!(matches!(
            command,
            Some(CoreCommand::Timeline(
                TimelineCommand::LoadLinkPreviews { .. }
            ))
        ));
    }

    #[test]
    fn reaction_tauri_command_contracts_are_present() {
        let commands_source = commands_source();
        let lib_source = include_str!("../lib.rs");
        for (command_name, route_name, registration_name, trace_kind) in [
            (
                "pub async fn send_reaction",
                "build_send_reaction_command",
                "commands::timeline::send_reaction",
                "send_reaction",
            ),
            (
                "pub async fn redact_reaction",
                "build_redact_reaction_command",
                "commands::timeline::redact_reaction",
                "redact_reaction",
            ),
        ] {
            assert!(
                commands_source.contains(command_name),
                "Tauri command should expose {command_name}"
            );
            assert!(
                commands_source.contains(route_name),
                "Tauri command should route through {route_name}"
            );
            assert!(
                lib_source.contains(registration_name),
                "Tauri command should register {registration_name}"
            );
            assert!(
                commands_source.contains(&format!(
                    "trace_tauri_timeline_command(\"submit\", \"{trace_kind}\""
                )),
                "Tauri command should trace submit for {trace_kind}"
            );
            assert!(
                commands_source.contains(&format!(
                    "trace_tauri_timeline_command_elapsed(\n        \"done\",\n        \"{trace_kind}\""
                )),
                "Tauri command should trace completion latency for {trace_kind}"
            );
        }
    }

    #[test]
    fn read_signal_tauri_commands_emit_latency_trace_tokens() {
        let commands_source = commands_source();
        for trace_kind in ["send_read_receipt", "set_fully_read"] {
            assert!(
                commands_source.contains(&format!(
                    "trace_tauri_timeline_command(\"submit\", \"{trace_kind}\""
                )),
                "read-signal command should trace submit for {trace_kind}"
            );
            assert!(
                commands_source.contains(&format!(
                    "trace_tauri_timeline_command_elapsed(\n        \"done\",\n        \"{trace_kind}\""
                )),
                "read-signal command should trace completion latency for {trace_kind}"
            );
        }
    }

    #[test]
    fn scheduled_send_tauri_command_contracts_are_present() {
        let commands_source = commands_source();
        let lib_source = include_str!("../lib.rs");
        for (command_name, route_name, registration_name) in [
            (
                "pub async fn schedule_send",
                "build_schedule_send_command",
                "commands::timeline::schedule_send",
            ),
            (
                "pub async fn cancel_scheduled_send",
                "build_cancel_scheduled_send_command",
                "commands::timeline::cancel_scheduled_send",
            ),
            (
                "pub async fn reschedule_scheduled_send",
                "build_reschedule_scheduled_send_command",
                "commands::timeline::reschedule_scheduled_send",
            ),
        ] {
            assert!(
                commands_source.contains(command_name),
                "Tauri command should expose {command_name}"
            );
            assert!(
                commands_source.contains(route_name),
                "Tauri command should route through {route_name}"
            );
            assert!(
                lib_source.contains(registration_name),
                "Tauri command should register {registration_name}"
            );
        }
    }

    #[test]
    fn send_queue_tauri_command_contracts_are_present() {
        let commands_source = commands_source();
        let lib_source = include_str!("../lib.rs");
        for (command_name, route_name, registration_name) in [
            (
                "pub async fn retry_send",
                "build_retry_send_command",
                "commands::timeline::retry_send",
            ),
            (
                "pub async fn cancel_send",
                "build_cancel_send_command",
                "commands::timeline::cancel_send",
            ),
        ] {
            assert!(
                commands_source.contains(command_name),
                "Tauri command should expose {command_name}"
            );
            assert!(
                commands_source.contains(route_name),
                "Tauri command should route through {route_name}"
            );
            assert!(
                lib_source.contains(registration_name),
                "Tauri command should register {registration_name}"
            );
        }
    }

    #[test]
    fn tauri_command_routes_blank_message_bodies_return_no_command() {
        let account_key = AccountKey("@alice:example.org".to_owned());
        let room_id = "!room:example.org".to_owned();

        assert!(
            build_send_text_command(
                fake_request_id(14),
                account_key.clone(),
                room_id.clone(),
                "desktop-14".to_owned(),
                ComposerDocument::from_plain_text("   "),
            )
            .is_none()
        );
        assert!(
            build_edit_message_command(
                fake_request_id(15),
                account_key,
                room_id,
                "$event".to_owned(),
                ComposerDocument::from_plain_text("\n\t "),
            )
            .is_none()
        );
        assert!(
            build_upload_media_command(
                fake_request_id(17),
                synthetic_session_key(),
                AccountKey("@alice:example.org".to_owned()),
                "!room:example.org".to_owned(),
                "desktop-media-empty".to_owned(),
                "empty.bin".to_owned(),
                "application/octet-stream".to_owned(),
                vec![],
                None,
                ImageUploadCompressionMode::Never,
                ImageUploadCompressionPolicy::default(),
                None,
                None,
                None,
            )
            .is_none()
        );
        assert!(
            build_download_media_command(
                fake_request_id(18),
                AccountKey("@alice:example.org".to_owned()),
                "!room:example.org".to_owned(),
                "\n\t ".to_owned(),
            )
            .is_none()
        );
        assert!(
            build_send_thread_reply_command(
                fake_request_id(16),
                AccountKey("@alice:example.org".to_owned()),
                "!room:example.org".to_owned(),
                "$root".to_owned(),
                "desktop-16".to_owned(),
                ComposerDocument::from_plain_text("\n\t "),
            )
            .is_none()
        );
    }

    #[test]
    fn thread_timeline_backwards_pagination_builder_targets_thread_key() {
        let account_key = AccountKey("@alice:example.org".to_owned());
        let room_id = "!room:example.org".to_owned();
        let root_event_id = "$thread-root".to_owned();

        match build_paginate_thread_timeline_backwards_command(
            fake_request_id(22),
            account_key.clone(),
            room_id.clone(),
            root_event_id.clone(),
        ) {
            CoreCommand::Timeline(TimelineCommand::Paginate {
                request_id,
                key,
                direction,
                event_count,
            }) => {
                assert_eq!(request_id, fake_request_id(22));
                assert_eq!(key.account_key, account_key);
                assert_eq!(
                    key.kind,
                    koushi_core::TimelineKind::Thread {
                        room_id,
                        root_event_id,
                    }
                );
                assert_eq!(direction, PaginationDirection::Backward);
                assert_eq!(event_count, 100);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn thread_timeline_backwards_pagination_contract_is_present() {
        let commands_source = commands_source();
        let lib_source = include_str!("../lib.rs");
        let helper_name = "build_paginate_thread_timeline_backwards_command";
        let command_name = "pub async fn paginate_thread_timeline_backwards";
        let registration_name = "commands::timeline::paginate_thread_timeline_backwards";

        let helper_offset = commands_source
            .find(helper_name)
            .expect("thread pagination builder helper should exist");
        let helper_source = &commands_source[helper_offset..];
        let helper_end = helper_source
            .find("fn build_send_text_command")
            .expect("thread pagination builder should live before send_text builder");
        let helper_source = &helper_source[..helper_end];

        assert!(
            commands_source.contains(command_name),
            "Tauri command should expose thread pagination"
        );
        assert!(
            lib_source.contains(registration_name),
            "Tauri command should be registered in generate_handler"
        );
        assert!(
            helper_source.contains("TimelineKind::Thread"),
            "thread pagination builder should use a thread timeline key"
        );
        assert!(
            helper_source.contains("PaginationDirection::Backward"),
            "thread pagination builder should request backward pagination"
        );
        assert!(
            helper_source.contains("event_count: TIMELINE_BACKWARDS_PAGE_EVENT_COUNT"),
            "thread pagination should keep the shared room pagination event count"
        );
    }
}
