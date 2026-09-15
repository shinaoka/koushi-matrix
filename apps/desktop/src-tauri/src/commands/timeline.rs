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
    request_id: koushi_protocol::RequestId,
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
        initial_backfill: koushi_protocol::command::InitialBackfillPolicy::Disabled,
    })
}

pub(super) fn build_paginate_timeline_backwards_command(
    request_id: koushi_protocol::RequestId,
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
    request_id: koushi_protocol::RequestId,
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
    request_id: koushi_protocol::RequestId,
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
    request_id: koushi_protocol::RequestId,
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
    request_id: koushi_protocol::RequestId,
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
    request_id: koushi_protocol::RequestId,
    account_key: AccountKey,
    room_id: String,
    first_visible_event_id: Option<String>,
    last_visible_event_id: Option<String>,
    visible_gap_ids: Vec<TimelineGapId>,
    at_bottom: bool,
    bottom_arrival: TimelineBottomArrival,
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
            bottom_arrival,
        },
    })
}

#[cfg(test)]
pub(super) fn build_send_text_command(
    request_id: koushi_protocol::RequestId,
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
    expected_account: koushi_protocol::SessionKeyId,
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
    request_id: koushi_protocol::RequestId,
    expected_account: koushi_protocol::SessionKeyId,
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

pub(super) fn build_cancel_scheduled_send_command(
    request_id: koushi_protocol::RequestId,
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
    request_id: koushi_protocol::RequestId,
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
    request_id: koushi_protocol::RequestId,
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
    request_id: koushi_protocol::RequestId,
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

pub(super) fn build_download_media_command(
    request_id: koushi_protocol::RequestId,
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
    request_id: koushi_protocol::RequestId,
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
    request_id: koushi_protocol::RequestId,
    account_key: AccountKey,
    room_id: String,
    event_id: String,
    origin: koushi_protocol::KeyRequestOrigin,
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
    request_id: koushi_protocol::RequestId,
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
    request_id: koushi_protocol::RequestId,
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
    request_id: koushi_protocol::RequestId,
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
    request_id: koushi_protocol::RequestId,
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
    request_id: koushi_protocol::RequestId,
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
    request_id: koushi_protocol::RequestId,
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
    request_id: koushi_protocol::RequestId,
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
    request_id: koushi_protocol::RequestId,
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
    request_id: koushi_protocol::RequestId,
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
    request_id: koushi_protocol::RequestId,
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
    request_id: koushi_protocol::RequestId,
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
const COMPOSER_DRAFT_ACCEPTANCE_TIMEOUT: Duration = Duration::from_secs(10);

async fn submit_required_admission(
    state: &CoreRuntimeState,
    command: Option<CoreCommand>,
    validation_error: &'static str,
) -> Result<FrontendCommandAdmission, String> {
    let command = command.ok_or_else(|| validation_error.to_owned())?;
    submit_core_command_with_admission(state, command).await
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

fn parse_composer_wire_tokens(
    renderer_generation: &str,
    lease_id: &str,
) -> Result<
    (
        koushi_core::composer_draft_lifecycle::ComposerRendererGeneration,
        koushi_core::composer_draft_lifecycle::ComposerDraftLeaseId,
    ),
    String,
> {
    let generation = koushi_core::composer_draft_lifecycle::ComposerRendererGeneration::parse_wire(
        renderer_generation,
    )
    .map_err(|_| "composer renderer generation is invalid".to_owned())?;
    let lease = koushi_core::composer_draft_lifecycle::ComposerDraftLeaseId::parse_wire(lease_id)
        .map_err(|_| "composer draft lease is invalid".to_owned())?;
    Ok((generation, lease))
}

fn acquire_terminal_composer_permit(
    connection: &CoreConnection,
    generation: koushi_core::composer_draft_lifecycle::ComposerRendererGeneration,
    lease_id: koushi_core::composer_draft_lifecycle::ComposerDraftLeaseId,
    account: &koushi_protocol::SessionKeyId,
    target: &koushi_state::ComposerTarget,
) -> Result<koushi_core::composer_draft_lifecycle::ComposerDraftCommandPermit, String> {
    connection
        .acquire_composer_draft_command_permit_for_active_target(
            account.clone(),
            target.clone(),
            generation,
            lease_id,
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
    connection
        .begin_composer_draft_renderer_generation()
        .map(|generation| generation.to_wire_string())
        .map_err(|_| "composer renderer generation unavailable".to_owned())
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
    let expected_account = koushi_protocol::SessionKeyId {
        homeserver: account_homeserver,
        user_id: account_user_id,
        device_id: account_device_id,
    };
    let generation = koushi_core::composer_draft_lifecycle::ComposerRendererGeneration::parse_wire(
        &renderer_generation,
    )
    .map_err(|_| "composer renderer generation is invalid".to_owned())?;
    let connection = state.connection.lock().await;
    let admission = connection
        .acquire_composer_draft_lease_for_active_target(expected_account, generation, target)
        .map_err(|_| "composer draft lease unavailable".to_owned())?;
    Ok(ComposerDraftLeaseResponse {
        renderer_generation: generation.to_wire_string(),
        lease_id: admission.lease_id.to_wire_string(),
        revision: admission.revision,
        last_accepted_clear_revision: admission.last_accepted_clear_revision,
        has_authoritative_content: admission.has_authoritative_content,
    })
}

#[tauri::command]
pub async fn release_composer_draft_lease(
    lease_id: String,
    renderer_generation: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<(), String> {
    let (generation, lease) = parse_composer_wire_tokens(&renderer_generation, &lease_id)?;
    state
        .connection
        .lock()
        .await
        .release_composer_draft_lease(generation, lease)
        .map_err(|_| "composer draft lease mismatch".to_owned())
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

async fn wait_for_composer_draft_acceptance(
    event_conn: &mut CoreConnection,
    request_id: RequestId,
    account_key: AccountKey,
    target: koushi_state::ComposerTarget,
    expected_revision: koushi_state::ComposerDraftRevision,
    baseline_generation: u64,
) -> Result<(koushi_state::ComposerDraftRevision, u64), String> {
    match event_conn
        .wait_for_request_outcome(
            OutcomeCorrelation::Request(request_id),
            RequestOutcomeExpectation::ComposerAccepted {
                request_id,
                account_key,
                target,
                expected_revision,
            },
            baseline_generation,
            tokio::time::Instant::now() + COMPOSER_DRAFT_ACCEPTANCE_TIMEOUT,
        )
        .await
        .map_err(|error| invoke_error_from_request_outcome("composer draft acceptance", error))?
    {
        RequestOutcome::ComposerAccepted {
            revision,
            generation,
            ..
        } => Ok((revision, generation)),
        _ => Err("composer draft acceptance: invalid request outcome".to_owned()),
    }
}

async fn wait_for_submission_settlement(
    event_conn: &mut CoreConnection,
    request_id: RequestId,
    account_key: AccountKey,
    target: koushi_state::ComposerTarget,
    submission_id: SubmissionId,
    baseline_generation: u64,
) -> Result<SubmissionResponse, SubmissionFailure> {
    let outcome = event_conn
        .wait_for_request_outcome(
            OutcomeCorrelation::Submission {
                request_id,
                submission_id: submission_id.clone(),
            },
            RequestOutcomeExpectation::Submission {
                request_id,
                account_key,
                target,
                submission_id: submission_id.clone(),
            },
            baseline_generation,
            tokio::time::Instant::now() + SUBMISSION_SETTLEMENT_TIMEOUT,
        )
        .await
        .map_err(submission_failure_from_outcome_error)?;
    let (outcome, transaction_id, generation) = match outcome {
        RequestOutcome::SubmissionAccepted {
            transaction_id,
            generation,
            ..
        } => (
            SubmissionOutcome::Accepted,
            Some(transaction_id),
            generation,
        ),
        RequestOutcome::SubmissionRejected {
            kind, generation, ..
        } => (SubmissionOutcome::Rejected { kind }, None, generation),
        _ => return Err(SubmissionFailure::SubmitFailed),
    };
    Ok(SubmissionResponse {
        outcome,
        submission_id,
        transaction_id,
        settlement: command_settlement(generation),
    })
}

fn submission_failure_from_outcome_error(error: RequestOutcomeError) -> SubmissionFailure {
    match error {
        RequestOutcomeError::TimedOut => SubmissionFailure::Timeout,
        RequestOutcomeError::Disconnected => SubmissionFailure::Disconnected,
        RequestOutcomeError::Lagged => SubmissionFailure::Lagged,
        RequestOutcomeError::OperationFailed { .. }
        | RequestOutcomeError::FailedNoOp { .. }
        | RequestOutcomeError::InvalidOutcome => SubmissionFailure::SubmitFailed,
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
) -> Result<FrontendCommandAdmission, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    trace_tauri_timeline_command("submit", "paginate_backwards", request_id);
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_paginate_timeline_backwards_command(request_id, account_key, room_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn restore_timeline_anchor(
    timeline_key: TimelineKey,
    event_id: String,
    max_batches: u16,
    event_count: u16,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
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
    Ok(admission)
}

#[tauri::command]
pub async fn ensure_timeline_subscribed(
    timeline_key: TimelineKey,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    trace_tauri_timeline_command("submit", "ensure_subscribed", request_id);
    let admission = submit_core_command_with_admission(
        state.inner(),
        CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id,
            key: TimelineKey {
                account_key,
                kind: timeline_key.kind,
            },
            initial_backfill: koushi_protocol::command::InitialBackfillPolicy::Disabled,
        }),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn paginate_thread_timeline_backwards(
    room_id: String,
    root_event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
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
    Ok(admission)
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

    let transaction_id = super::next_transaction_id("desktop");
    let expected_account = koushi_protocol::SessionKeyId {
        homeserver: account_homeserver,
        user_id: account_user_id,
        device_id: account_device_id,
    };
    let (generation, lease) = parse_composer_wire_tokens(&renderer_generation, &lease_id)
        .map_err(|_| SubmissionFailure::SubmitFailed)?;
    let mut event_conn = state.runtime.attach();
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
    let baseline_generation = event_conn.state_generation();
    let request_id = event_conn.next_request_id();
    let account_key = account_key_from_app_state(&event_conn.snapshot());
    let submission_id = SubmissionId::new(submission_id);
    if let Some(command) = build_submit_text_command(
        request_id,
        expected_account,
        submission_id.clone(),
        account_key.clone(),
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
    let response = wait_for_submission_settlement(
        &mut event_conn,
        request_id,
        account_key,
        target,
        submission_id,
        baseline_generation,
    )
    .await?;
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
    let (generation, lease) = parse_composer_wire_tokens(&renderer_generation, &lease_id)?;
    let mut event_conn = state.runtime.attach();
    let expected_account = koushi_protocol::SessionKeyId {
        homeserver: account_homeserver,
        user_id: account_user_id,
        device_id: account_device_id,
    };
    let expected_revision =
        next_composer_draft_acceptance_revision(&event_conn.snapshot(), &target, draft_revision)?;
    let account_key = account_key_from_app_state(&event_conn.snapshot());
    let baseline_generation = event_conn.state_generation();
    let _terminal_permit = acquire_terminal_composer_permit(
        &event_conn,
        generation,
        lease,
        &expected_account,
        &target,
    )?;
    let request_id = event_conn.next_request_id();
    let Some(command) = build_schedule_send_command(
        request_id,
        expected_account,
        target.clone(),
        body,
        send_at_ms,
        draft_revision,
    ) else {
        return Err("scheduled send body must not be blank".to_owned());
    };
    event_conn
        .command_with_composer_lease(generation, lease, command)
        .await
        .map_err(|error| format!("command submit failed: {error}"))?;
    let (accepted_revision, settled_snapshot) = wait_for_composer_draft_acceptance(
        &mut event_conn,
        request_id,
        account_key,
        target,
        expected_revision,
        baseline_generation,
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(ComposerDraftAcceptanceResponse {
        accepted_revision,
        settlement: command_settlement(settled_snapshot),
    })
}

#[tauri::command]
pub async fn stage_upload_bytes(
    target: koushi_state::ComposerTarget,
    items: Vec<StageUploadBytesInputItem>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let items = items
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
    let settled = state
        .runtime
        .attach()
        .stage_upload_bytes(target, items)
        .await
        .map_err(|error| error.to_string())?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(command_settlement(settled))
}

#[tauri::command]
pub async fn select_staged_upload_output(
    target: koushi_state::ComposerTarget,
    staged_id: String,
    selection: koushi_state::StagedUploadOutputSelection,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let settled = state
        .runtime
        .attach()
        .select_staged_upload_output(target, staged_id, selection)
        .await
        .map_err(|error| error.to_string())?;
    Ok(command_settlement(settled))
}

#[tauri::command]
pub async fn retry_staged_upload_preparation(
    target: koushi_state::ComposerTarget,
    staged_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let settled = state
        .runtime
        .attach()
        .retry_staged_upload_preparation(target, staged_id)
        .await
        .map_err(|error| error.to_string())?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(command_settlement(settled))
}

#[tauri::command]
pub async fn use_original_staged_upload(
    target: koushi_state::ComposerTarget,
    staged_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let settled = state
        .runtime
        .attach()
        .use_original_staged_upload(target, staged_id)
        .await
        .map_err(|error| error.to_string())?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(command_settlement(settled))
}

#[tauri::command]
pub async fn prepared_upload_preview(
    target: koushi_state::ComposerTarget,
    staged_id: String,
    variant_id: String,
    state: State<'_, CoreRuntimeState>,
) -> Result<Vec<u8>, String> {
    state
        .runtime
        .attach()
        .prepared_upload_preview(target, staged_id, variant_id)
        .await
        .map_err(|error| error.to_string())
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
    let expected_account = koushi_protocol::SessionKeyId {
        homeserver: account_homeserver,
        user_id: account_user_id,
        device_id: account_device_id,
    };
    let (generation, lease) = parse_composer_wire_tokens(&renderer_generation, &lease_id)?;
    let settled = state
        .runtime
        .attach()
        .send_prepared_uploads(expected_account, generation, lease, target, draft_revision)
        .await
        .map_err(|error| error.to_string())?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(ComposerDraftAcceptanceResponse {
        accepted_revision: settled.accepted_revision,
        settlement: command_settlement(settled.generation),
    })
}

#[tauri::command]
pub async fn update_staged_upload_caption(
    target: koushi_state::ComposerTarget,
    staged_id: String,
    document: Option<ComposerDocument>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let settled = state
        .runtime
        .attach()
        .update_staged_upload_caption(target, staged_id, document)
        .await
        .map_err(|error| error.to_string())?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(command_settlement(settled))
}

#[tauri::command]
pub async fn update_staged_upload_compression(
    target: koushi_state::ComposerTarget,
    staged_id: String,
    compression_choice: StagedUploadCompressionChoice,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let settled = state
        .runtime
        .attach()
        .update_staged_upload_compression(target, staged_id, compression_choice)
        .await
        .map_err(|error| error.to_string())?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(command_settlement(settled))
}

#[tauri::command]
pub async fn clear_upload_staging(
    target: koushi_state::ComposerTarget,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandSettlement, String> {
    let settled = state
        .runtime
        .attach()
        .clear_upload_staging(target)
        .await
        .map_err(|error| error.to_string())?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(command_settlement(settled))
}

#[tauri::command]
pub async fn cancel_scheduled_send(
    scheduled_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let Some(command) = build_cancel_scheduled_send_command(request_id, scheduled_id) else {
        return Err("scheduled send id must not be blank".to_owned());
    };
    let admission = submit_core_command_with_admission(state.inner(), command).await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn reschedule_scheduled_send(
    scheduled_id: String,
    body: String,
    send_at_ms: u64,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let Some(command) =
        build_reschedule_scheduled_send_command(request_id, scheduled_id, body, send_at_ms)
    else {
        return Err("scheduled send id and body must not be blank".to_owned());
    };
    let admission = submit_core_command_with_admission(state.inner(), command).await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn retry_send(
    room_id: String,
    transaction_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    let Some(command) = build_retry_send_command(request_id, account_key, room_id, transaction_id)
    else {
        return Err("send transaction id must not be blank".to_owned());
    };
    let admission = submit_core_command_with_admission(state.inner(), command).await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn cancel_send(
    room_id: String,
    transaction_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    let Some(command) = build_cancel_send_command(request_id, account_key, room_id, transaction_id)
    else {
        return Err("send transaction id must not be blank".to_owned());
    };
    let admission = submit_core_command_with_admission(state.inner(), command).await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn download_media(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<(), String> {
    if event_id.trim().is_empty() {
        return Err("media event id must not be blank".to_owned());
    }

    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    let Some(command) = build_download_media_command(request_id, account_key, room_id, event_id)
    else {
        return Err("media event id must not be blank".to_owned());
    };
    submit_core_command(state.inner(), command).await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(())
}

#[tauri::command]
pub async fn save_downloaded_media(
    source_url: String,
    destination_path: String,
) -> Result<(), String> {
    let cache_root = crate::app_data_dir()
        .map_err(|_| "media cache is unavailable".to_owned())?
        .join("media_downloads");
    let filesystem = crate::media_save::NativeMediaSaveFilesystem;
    koushi_core::save_downloaded_media(
        &filesystem,
        &cache_root,
        std::path::Path::new(source_url.trim()),
        std::path::Path::new(destination_path.trim()),
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn default_media_save_path(filename: String, app: AppHandle) -> Result<String, String> {
    let downloads_dir = app.path().download_dir().ok();
    Ok(
        koushi_core::default_media_save_path(&filename, downloads_dir.as_deref())
            .to_string_lossy()
            .into_owned(),
    )
}

#[tauri::command]
pub async fn load_message_source(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_required_admission(
        state.inner(),
        build_load_message_source_command(request_id, account_key, room_id, event_id),
        "message event id must not be blank",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn request_room_key(
    room_id: String,
    event_id: String,
    origin: Option<koushi_protocol::KeyRequestOrigin>,
    timeline_key: Option<TimelineKey>,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    // Only absent origin defaults to User; unknown wire values are rejected by
    // the typed deserializer instead of being silently coerced.
    let origin = origin.unwrap_or(koushi_protocol::KeyRequestOrigin::User);
    let admission = submit_required_admission(
        state.inner(),
        build_request_room_key_command(
            request_id,
            account_key,
            room_id,
            event_id,
            origin,
            timeline_key,
        ),
        "room key event id must not be blank",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
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
) -> Result<FrontendCommandAdmission, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_required_admission(
        state.inner(),
        build_request_late_decryption_command(request_id, account_key, room_id, timeline_key),
        "room id must not be blank",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn load_link_previews(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    trace_tauri_timeline_command("submit", "load_link_previews", request_id);
    let admission = submit_required_admission(
        state.inner(),
        build_load_link_previews_command(request_id, account_key, room_id, event_id),
        "link preview event id must not be blank",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn hide_link_preview(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_required_admission(
        state.inner(),
        build_hide_link_preview_command(request_id, account_key, room_id, event_id),
        "link preview event id must not be blank",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn forward_message(
    room_id: String,
    source_event_id: String,
    destination_room_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let transaction_id = super::next_transaction_id("desktop-forward");
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_required_admission(
        state.inner(),
        build_forward_message_command(
            request_id,
            account_key,
            room_id,
            source_event_id,
            destination_room_id,
            transaction_id,
        ),
        "forward source and destination must not be blank",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn edit_message(
    room_id: String,
    event_id: String,
    document: koushi_state::ComposerDocument,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    if document.plain_body().trim().is_empty() {
        return Err("message body must not be blank".to_owned());
    }
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_required_admission(
        state.inner(),
        build_edit_message_command(request_id, account_key, room_id, event_id, document),
        "message target must not be blank",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn redact_message(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        build_redact_message_command(request_id, account_key, room_id, event_id),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn toggle_reaction(
    room_id: String,
    event_id: String,
    reaction_key: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    if reaction_key.is_empty() {
        return Err("reaction key must not be blank".to_owned());
    }

    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_required_admission(
        state.inner(),
        build_toggle_reaction_command(request_id, account_key, room_id, event_id, reaction_key),
        "reaction target must not be blank",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn send_reaction(
    room_id: String,
    event_id: String,
    reaction_key: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    if reaction_key.trim().is_empty() || event_id.trim().is_empty() {
        return Err("reaction key and event id must not be blank".to_owned());
    }

    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    let trace_started = std::time::Instant::now();
    trace_tauri_timeline_command("submit", "send_reaction", request_id);
    let admission = submit_required_admission(
        state.inner(),
        build_send_reaction_command(request_id, account_key, room_id, event_id, reaction_key),
        "reaction target must not be blank",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    trace_tauri_timeline_command_elapsed(
        "done",
        "send_reaction",
        request_id,
        trace_started.elapsed().as_millis(),
    );
    Ok(admission)
}

#[tauri::command]
pub async fn redact_reaction(
    room_id: String,
    event_id: String,
    reaction_key: String,
    reaction_event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    if reaction_key.trim().is_empty()
        || event_id.trim().is_empty()
        || reaction_event_id.trim().is_empty()
    {
        return Err("reaction target must not be blank".to_owned());
    }

    let account_key = account_key_from_snapshot(state.inner()).await;
    let request_id = next_request_id(state.inner()).await;
    let trace_started = std::time::Instant::now();
    trace_tauri_timeline_command("submit", "redact_reaction", request_id);
    let admission = submit_required_admission(
        state.inner(),
        build_redact_reaction_command(
            request_id,
            account_key,
            room_id,
            event_id,
            reaction_key,
            reaction_event_id,
        ),
        "reaction target must not be blank",
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    trace_tauri_timeline_command_elapsed(
        "done",
        "redact_reaction",
        request_id,
        trace_started.elapsed().as_millis(),
    );
    Ok(admission)
}

#[tauri::command]
pub async fn set_composer_reply_target(
    room_id: String,
    event_id: String,
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        CoreCommand::App(AppCommand::SetComposerReplyTarget {
            request_id,
            room_id,
            event_id,
        }),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
}

#[tauri::command]
pub async fn cancel_composer_reply(
    app: AppHandle,
    state: State<'_, CoreRuntimeState>,
) -> Result<FrontendCommandAdmission, String> {
    let request_id = next_request_id(state.inner()).await;
    let admission = submit_core_command_with_admission(
        state.inner(),
        CoreCommand::App(AppCommand::CancelComposerReply { request_id }),
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(admission)
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
) -> Result<FrontendCommandAdmission, String> {
    let (generation, lease) = parse_composer_wire_tokens(&renderer_generation, &lease_id)?;
    let event_conn = state.runtime.attach();
    let expected_account = koushi_protocol::SessionKeyId {
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
    let admission = event_conn
        .command_with_composer_lease_and_admission(
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
    Ok(FrontendCommandAdmission::from_core(admission))
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
) -> Result<FrontendCommandAdmission, String> {
    let (generation, lease) = parse_composer_wire_tokens(&renderer_generation, &lease_id)?;
    let event_conn = state.runtime.attach();
    let expected_account = koushi_protocol::SessionKeyId {
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
    let admission = event_conn
        .command_with_composer_lease_and_admission(
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
    Ok(FrontendCommandAdmission::from_core(admission))
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

    let transaction_id = super::next_transaction_id("desktop");
    let expected_account = koushi_protocol::SessionKeyId {
        homeserver: account_homeserver,
        user_id: account_user_id,
        device_id: account_device_id,
    };
    let (generation, lease) = parse_composer_wire_tokens(&renderer_generation, &lease_id)
        .map_err(|_| SubmissionFailure::SubmitFailed)?;
    let mut event_conn = state.runtime.attach();
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
    let baseline_generation = event_conn.state_generation();
    let request_id = event_conn.next_request_id();
    let account_key = account_key_from_app_state(&event_conn.snapshot());
    let submission_id = SubmissionId::new(submission_id);
    if let Some(command) = build_submit_reply_command(
        request_id,
        expected_account,
        submission_id.clone(),
        account_key.clone(),
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
    let response = wait_for_submission_settlement(
        &mut event_conn,
        request_id,
        account_key,
        target,
        submission_id,
        baseline_generation,
    )
    .await?;
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

    let transaction_id = super::next_transaction_id("desktop");
    let expected_account = koushi_protocol::SessionKeyId {
        homeserver: account_homeserver,
        user_id: account_user_id,
        device_id: account_device_id,
    };
    let (generation, lease) = parse_composer_wire_tokens(&renderer_generation, &lease_id)
        .map_err(|_| SubmissionFailure::SubmitFailed)?;
    let mut event_conn = state.runtime.attach();
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
    let baseline_generation = event_conn.state_generation();
    let request_id = event_conn.next_request_id();
    let account_key = account_key_from_app_state(&event_conn.snapshot());
    let submission_id = SubmissionId::new(submission_id);
    if let Some(command) = build_submit_thread_reply_command(
        request_id,
        expected_account,
        submission_id.clone(),
        account_key.clone(),
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
    let response = wait_for_submission_settlement(
        &mut event_conn,
        request_id,
        account_key,
        target,
        submission_id,
        baseline_generation,
    )
    .await?;
    update_qa_window_title_from_state(&app, state.inner()).await;
    Ok(response)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ComposerDraftAcceptanceResponse {
    pub accepted_revision: ComposerDraftRevision,
    pub settlement: FrontendCommandSettlement,
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
        kind: koushi_protocol::TimelineFailureKind,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubmissionResponse {
    pub outcome: SubmissionOutcome,
    pub submission_id: SubmissionId,
    pub transaction_id: Option<String>,
    pub settlement: FrontendCommandSettlement,
}

#[cfg(test)]
pub(super) fn build_send_reply_command(
    request_id: koushi_protocol::RequestId,
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
    request_id: koushi_protocol::RequestId,
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
    request_id: koushi_protocol::RequestId,
    expected_account: koushi_protocol::SessionKeyId,
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
    request_id: koushi_protocol::RequestId,
    expected_account: koushi_protocol::SessionKeyId,
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
    expected_account: koushi_protocol::SessionKeyId,
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
    expected_account: koushi_protocol::SessionKeyId,
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

#[cfg(test)]
mod composer_wire_tests;
#[cfg(test)]
mod issue551_moved_tests;
#[cfg(test)]
mod outcome_delegation_tests;
