use crate::{ComposerDraftRevision, SubmissionId};
use crate::{
    effect::{AppEffect, UiEvent},
    state::{
        AppError, AppState, ComposerMode, PendingComposerSendKind, StagedUploadCompressionChoice,
        ThreadPaneState, TimelineContinuityInspection, TimelineContinuityState,
        TimelineGapRepairFailureKind,
    },
};

use super::{
    is_session_ready, refresh_timeline_media_gallery, refresh_timeline_scheduled_sends,
    refresh_timeline_upload_staging, room_exists, withdraw_scheduled_send_persistence_failure,
};

const TIMELINE_SUBSCRIPTION_FAILED_MESSAGE: &str = "Matrix timeline subscription failed";

/// Error code for a failed local scheduled-send save (#1159). The desktop
/// composer matches this code to show its localized notice; the Rust message
/// stays a coarse, identifier-free fallback.
pub(crate) const SCHEDULED_SEND_PERSISTENCE_FAILED: &str = "scheduled_send_persistence_failed";

pub(crate) fn handle_scheduled_send_persistence_failed(
    state: &mut AppState,
    message: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    // One notice per failure class: repeated failed writes must not grow the list.
    if state
        .errors
        .iter()
        .any(|error| error.code == SCHEDULED_SEND_PERSISTENCE_FAILED)
    {
        return Vec::new();
    }
    state.errors.push(AppError {
        code: SCHEDULED_SEND_PERSISTENCE_FAILED.to_owned(),
        message,
        recoverable: true,
    });
    vec![AppEffect::EmitUiEvent(UiEvent::ErrorChanged)]
}

pub(crate) fn handle_scheduled_send_persisted(state: &mut AppState) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    // A successful write saves the whole store, so the earlier failure is
    // resolved. Other codes are untouched (#1159).
    withdraw_scheduled_send_persistence_failure(state)
        .into_iter()
        .collect()
}

pub(crate) fn handle_timeline_subscribed(state: &mut AppState, room_id: String) -> Vec<AppEffect> {
    if !is_session_ready(state) || state.timeline.room_id.as_deref() != Some(room_id.as_str()) {
        return Vec::new();
    }

    state.timeline.is_subscribed = true;
    state.timeline.continuity = TimelineContinuityState::Unknown;
    vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
}

pub(crate) fn handle_timeline_subscription_failed(
    state: &mut AppState,
    room_id: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state) || state.timeline.room_id.as_deref() != Some(room_id.as_str()) {
        return Vec::new();
    }

    state.errors.push(AppError {
        code: "timeline_subscription_failed".to_owned(),
        message: TIMELINE_SUBSCRIPTION_FAILED_MESSAGE.to_owned(),
        recoverable: true,
    });
    vec![
        AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }),
        AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
    ]
}

fn continuity_gap_count(state: &TimelineContinuityState) -> u32 {
    match state {
        TimelineContinuityState::Inspecting {
            known_gap_count, ..
        } => *known_gap_count,
        TimelineContinuityState::Incomplete { gap_count, .. }
        | TimelineContinuityState::Repairing { gap_count, .. }
        | TimelineContinuityState::FailedIncomplete { gap_count, .. } => *gap_count,
        TimelineContinuityState::Unknown | TimelineContinuityState::Healthy { .. } => 0,
    }
}

fn continuity_generation(state: &TimelineContinuityState) -> u64 {
    match state {
        TimelineContinuityState::Unknown => 0,
        TimelineContinuityState::Inspecting { generation, .. }
        | TimelineContinuityState::Healthy { generation, .. }
        | TimelineContinuityState::Incomplete { generation, .. }
        | TimelineContinuityState::Repairing { generation, .. }
        | TimelineContinuityState::FailedIncomplete { generation, .. } => *generation,
    }
}

fn active_room_matches(state: &AppState, room_id: &str) -> bool {
    is_session_ready(state) && state.timeline.room_id.as_deref() == Some(room_id)
}

pub(crate) fn handle_timeline_continuity_inspection_started(
    state: &mut AppState,
    room_id: String,
    generation: u64,
) -> Vec<AppEffect> {
    if !active_room_matches(state, &room_id)
        || generation <= continuity_generation(&state.timeline.continuity)
    {
        return Vec::new();
    }
    let known_gap_count = continuity_gap_count(&state.timeline.continuity);
    state.timeline.continuity = TimelineContinuityState::Inspecting {
        generation,
        known_gap_count,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
}

pub(crate) fn handle_timeline_continuity_inspected(
    state: &mut AppState,
    room_id: String,
    generation: u64,
    inspection: TimelineContinuityInspection,
) -> Vec<AppEffect> {
    if !active_room_matches(state, &room_id)
        || !matches!(
            state.timeline.continuity,
            TimelineContinuityState::Inspecting { generation: active, .. } if active == generation
        )
    {
        return Vec::new();
    }
    state.timeline.continuity = match inspection {
        TimelineContinuityInspection::Unknown => TimelineContinuityState::Unknown,
        TimelineContinuityInspection::Gapped { gap_count } => TimelineContinuityState::Incomplete {
            generation,
            gap_count,
        },
        TimelineContinuityInspection::Complete => TimelineContinuityState::Healthy {
            generation,
            authoritative_start: true,
        },
    };
    vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
}

pub(crate) fn handle_timeline_gap_repair_started(
    state: &mut AppState,
    room_id: String,
    generation: u64,
    gap_count: u32,
) -> Vec<AppEffect> {
    if !active_room_matches(state, &room_id)
        || generation <= continuity_generation(&state.timeline.continuity)
    {
        return Vec::new();
    }
    state.timeline.continuity = TimelineContinuityState::Repairing {
        generation,
        gap_count,
        batches_processed: 0,
        minimum_batch_id: None,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
}

pub(crate) fn handle_timeline_gap_repair_progressed(
    state: &mut AppState,
    room_id: String,
    generation: u64,
    gap_count: u32,
    batches_processed: u32,
    minimum_batch_id: Option<u64>,
) -> Vec<AppEffect> {
    if !active_room_matches(state, &room_id)
        || !matches!(
            state.timeline.continuity,
            TimelineContinuityState::Repairing { generation: active, .. } if active == generation
        )
    {
        return Vec::new();
    }
    state.timeline.continuity = TimelineContinuityState::Repairing {
        generation,
        gap_count,
        batches_processed,
        minimum_batch_id,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
}

pub(crate) fn handle_timeline_gap_repair_failed(
    state: &mut AppState,
    room_id: String,
    generation: u64,
    gap_count: u32,
    batches_processed: u32,
    failure_kind: TimelineGapRepairFailureKind,
) -> Vec<AppEffect> {
    if !active_room_matches(state, &room_id)
        || !matches!(
            state.timeline.continuity,
            TimelineContinuityState::Repairing { generation: active, .. } if active == generation
        )
    {
        return Vec::new();
    }
    state.timeline.continuity = TimelineContinuityState::FailedIncomplete {
        generation,
        gap_count,
        batches_processed,
        failure_kind,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
}

pub(crate) fn handle_timeline_back_pagination_requested(
    state: &mut AppState,
    room_id: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    if state.timeline.room_id.as_deref() != Some(room_id.as_str())
        || state.timeline.is_paginating_backwards
    {
        return Vec::new();
    }

    state.timeline.is_paginating_backwards = true;
    vec![
        AppEffect::PaginateTimelineBackwards {
            room_id: room_id.clone(),
        },
        AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }),
    ]
}

pub(crate) fn handle_timeline_back_pagination_finished(
    state: &mut AppState,
    room_id: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    if state.timeline.room_id.as_deref() != Some(room_id.as_str())
        || !state.timeline.is_paginating_backwards
    {
        return Vec::new();
    }

    state.timeline.is_paginating_backwards = false;
    vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
}

pub(crate) fn handle_scheduled_send_capability_changed(
    state: &mut AppState,
    capability: crate::state::ScheduledSendCapability,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    if state.scheduled_sends.capability == capability {
        return Vec::new();
    }
    state.scheduled_sends.capability = capability;
    state.timeline.scheduled_send_capability = state.scheduled_sends.capability.clone();
    state
        .timeline
        .room_id
        .clone()
        .map(|room_id| vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })])
        .unwrap_or_default()
}

pub(crate) fn handle_scheduled_sends_loaded(
    state: &mut AppState,
    scheduled_sends: crate::state::ScheduledSendStore,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    if state.scheduled_sends == scheduled_sends {
        return Vec::new();
    }

    state.scheduled_sends = scheduled_sends;
    let Some(room_id) = state.timeline.room_id.clone() else {
        return Vec::new();
    };

    refresh_timeline_scheduled_sends(state);
    vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
}

pub(crate) fn handle_scheduled_send_created(
    state: &mut AppState,
    item: crate::state::ScheduledSendItem,
) -> Vec<AppEffect> {
    let draft_revision = if let Some(root_event_id) = item.thread_root_event_id.as_deref() {
        state
            .composer_drafts
            .thread_revision(&item.room_id, root_event_id)
    } else {
        state.composer_drafts.room_revision(&item.room_id)
    };
    handle_scheduled_send_created_at_revision(state, item, draft_revision)
}

pub(crate) fn handle_scheduled_send_created_at_revision(
    state: &mut AppState,
    item: crate::state::ScheduledSendItem,
    draft_revision: ComposerDraftRevision,
) -> Vec<AppEffect> {
    if !is_session_ready(state) || !room_exists(state, &item.room_id) {
        return Vec::new();
    }

    let room_id = item.room_id.clone();
    let thread_root_event_id = item.thread_root_event_id.clone();
    state.scheduled_sends.insert(item);
    if let Some(root_event_id) = thread_root_event_id.as_deref() {
        if state
            .composer_drafts
            .advance_thread_revision(&room_id, root_event_id, draft_revision)
            .is_err()
        {
            return Vec::new();
        }
        let mut effects = Vec::new();
        if let crate::state::ThreadPaneState::Open {
            room_id: open_room_id,
            root_event_id: open_root_event_id,
            composer,
            ..
        } = &mut state.thread
            && open_room_id == &room_id
            && open_root_event_id.as_str() == root_event_id
        {
            *composer = state
                .composer_drafts
                .composer_for_thread(&room_id, root_event_id);
            effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadChanged));
        }
        // #1159: the reservation belongs to this room's scheduled-send list
        // even though the thread composer, not the room composer, was cleared.
        // Refresh that projection here so an accepted reply is visible without
        // reselecting the room; the open thread stays open.
        if state.timeline.room_id.as_deref() == Some(room_id.as_str()) {
            refresh_timeline_scheduled_sends(state);
            effects.push(AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }));
        }
        return effects;
    }
    if state
        .composer_drafts
        .advance_room_revision(&room_id, draft_revision)
        .is_err()
    {
        return Vec::new();
    }
    if state.timeline.room_id.as_deref() == Some(room_id.as_str()) {
        state.timeline.composer = state.composer_drafts.composer_for_room(&room_id);
        refresh_timeline_scheduled_sends(state);
        return vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })];
    }
    Vec::new()
}

pub(crate) fn handle_scheduled_send_dispatch_started(
    state: &mut AppState,
    scheduled_id: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    state.scheduled_sends.start_local_dispatch(&scheduled_id);
    Vec::new()
}

pub(crate) fn handle_scheduled_send_dispatch_failed(
    state: &mut AppState,
    scheduled_id: String,
    retry_at_ms: u64,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let Some(item) = state
        .scheduled_sends
        .retry_local_dispatch(&scheduled_id, retry_at_ms)
    else {
        return Vec::new();
    };
    let room_id = item.room_id;
    if state.timeline.room_id.as_deref() == Some(room_id.as_str()) {
        refresh_timeline_scheduled_sends(state);
        return vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })];
    }
    Vec::new()
}

pub(crate) fn handle_scheduled_send_rescheduled(
    state: &mut AppState,
    scheduled_id: String,
    body: String,
    send_at_ms: u64,
    handle: crate::state::ScheduledSendHandle,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let Some(item) = state
        .scheduled_sends
        .reschedule(&scheduled_id, body, send_at_ms, handle)
    else {
        return Vec::new();
    };
    let room_id = item.room_id;
    if state.timeline.room_id.as_deref() == Some(room_id.as_str()) {
        refresh_timeline_scheduled_sends(state);
        return vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })];
    }
    Vec::new()
}

pub(crate) fn handle_scheduled_send_cancelled_or_dispatched(
    state: &mut AppState,
    scheduled_id: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let Some(item) = state.scheduled_sends.remove(&scheduled_id) else {
        return Vec::new();
    };
    let room_id = item.room_id;
    if state.timeline.room_id.as_deref() == Some(room_id.as_str()) {
        refresh_timeline_scheduled_sends(state);
        return vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })];
    }
    Vec::new()
}

pub(crate) fn handle_upload_staging_changed(
    state: &mut AppState,
    target: crate::ComposerTarget,
    items: Vec<crate::state::StagedUploadItem>,
) -> Vec<AppEffect> {
    if !is_session_ready(state)
        || !room_exists(state, target.room_id())
        || !composer_target_is_active(state, &target)
    {
        return Vec::new();
    }

    state
        .upload_staging
        .replace_target_items(target.clone(), items);
    refresh_and_emit_upload_target(state, &target)
}

pub(crate) fn handle_upload_staging_caption_changed(
    state: &mut AppState,
    target: crate::ComposerTarget,
    staged_id: String,
    caption: Option<crate::ComposerDocument>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) || !composer_target_is_active(state, &target) {
        return Vec::new();
    }

    let Some(_) = state
        .upload_staging
        .update_caption(&target, &staged_id, caption)
    else {
        return Vec::new();
    };
    refresh_and_emit_upload_target(state, &target)
}

pub(crate) fn handle_upload_staging_compression_changed(
    state: &mut AppState,
    target: crate::ComposerTarget,
    staged_id: String,
    compression_choice: StagedUploadCompressionChoice,
) -> Vec<AppEffect> {
    if !is_session_ready(state)
        || !composer_target_is_active(state, &target)
        || !staged_compression_choice_is_valid_for_item(
            state
                .upload_staging
                .items
                .get(&(target.clone(), staged_id.clone())),
            compression_choice,
        )
    {
        return Vec::new();
    }

    let Some(item) =
        state
            .upload_staging
            .update_compression_choice(&target, &staged_id, compression_choice)
    else {
        return Vec::new();
    };
    let _ = item;
    refresh_and_emit_upload_target(state, &target)
}

pub(crate) fn handle_upload_staging_cleared(
    state: &mut AppState,
    target: crate::ComposerTarget,
) -> Vec<AppEffect> {
    if !is_session_ready(state)
        || !composer_target_is_active(state, &target)
        || !state.upload_staging.clear_target(&target)
    {
        return Vec::new();
    }
    refresh_and_emit_upload_target(state, &target)
}

pub(crate) fn handle_upload_staging_output_selected(
    state: &mut AppState,
    target: crate::ComposerTarget,
    staged_id: String,
    selection: crate::state::StagedUploadOutputSelection,
) -> Vec<AppEffect> {
    if !is_session_ready(state) || !composer_target_is_active(state, &target) {
        return Vec::new();
    }
    if state
        .upload_staging
        .select_output(&target, &staged_id, selection)
        .is_none()
    {
        return Vec::new();
    }
    refresh_and_emit_upload_target(state, &target)
}

pub(crate) fn handle_media_gallery_updated(
    state: &mut AppState,
    room_id: String,
    items: Vec<crate::state::TimelineMediaGalleryItem>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) || !room_exists(state, &room_id) {
        return Vec::new();
    }

    state.media_gallery.replace_room_items(&room_id, items);
    if state.timeline.room_id.as_deref() == Some(room_id.as_str()) {
        refresh_timeline_media_gallery(state);
        return vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })];
    }
    Vec::new()
}

pub(crate) fn handle_media_download_updated(
    state: &mut AppState,
    room_id: String,
    event_id: String,
    download_state: crate::state::TimelineMediaDownloadState,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    if state.timeline.room_id.as_deref() != Some(room_id.as_str()) {
        return Vec::new();
    }
    let preserve_ready = matches!(
        state.timeline.media_downloads.get(&event_id),
        Some(crate::state::TimelineMediaDownloadState::Ready { .. })
    ) && matches!(
        download_state,
        crate::state::TimelineMediaDownloadState::Failed { .. }
    );
    if preserve_ready {
        return vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })];
    }
    state
        .timeline
        .media_downloads
        .insert(event_id, download_state);
    vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
}

pub(crate) fn handle_composer_drafts_loaded(
    state: &mut AppState,
    drafts: crate::state::ComposerDraftStore,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    state.composer_drafts = drafts;
    let mut effects = Vec::new();
    if let Some(room_id) = state.timeline.room_id.clone()
        && state.timeline.composer.pending_transaction_id.is_none()
        && state.timeline.composer.draft.is_empty()
    {
        let composer = state.composer_drafts.composer_for_room(&room_id);
        if state.timeline.composer != composer {
            state.timeline.composer = composer;
            effects.push(AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }));
        }
    }
    if let ThreadPaneState::Open {
        room_id,
        root_event_id,
        composer,
        ..
    } = &mut state.thread
        && composer.pending_transaction_id.is_none()
        && composer.draft.is_empty()
    {
        let hydrated = state
            .composer_drafts
            .composer_for_thread(room_id, root_event_id);
        if *composer != hydrated {
            *composer = hydrated;
            effects.push(AppEffect::EmitUiEvent(UiEvent::ThreadChanged));
        }
    }
    effects
}

pub(crate) fn handle_composer_draft_changed(
    state: &mut AppState,
    room_id: String,
    document: crate::ComposerDocument,
    revision: ComposerDraftRevision,
) -> Vec<AppEffect> {
    if !is_session_ready(state) || !state.rooms.iter().any(|room| room.room_id == room_id) {
        return Vec::new();
    }

    if !matches!(
        state
            .composer_drafts
            .apply_room_draft(room_id.clone(), document.clone(), revision),
        Ok(true)
    ) {
        return Vec::new();
    }
    if state.timeline.room_id.as_deref() == Some(room_id.as_str()) {
        state.timeline.composer.draft = document.plain_body();
        state.timeline.composer.document = document;
        state.timeline.composer.draft_revision = revision;
        vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
    } else {
        Vec::new()
    }
}

pub(crate) fn handle_composer_draft_accepted(
    state: &mut AppState,
    target: crate::ComposerTarget,
    submitted_revision: ComposerDraftRevision,
    consumes_draft: bool,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    match target {
        crate::ComposerTarget::Main { room_id } => {
            if !room_exists(state, &room_id) {
                return Vec::new();
            }
            // #1130: a staged-attachment send settles the draft it never
            // dispatched, so it advances the revision without clearing the text.
            let settled = if consumes_draft {
                state
                    .composer_drafts
                    .advance_room_revision(&room_id, submitted_revision)
            } else {
                state
                    .composer_drafts
                    .settle_room_revision(&room_id, submitted_revision)
            };
            if settled.is_err() {
                return Vec::new();
            }
            if state.timeline.room_id.as_deref() != Some(room_id.as_str()) {
                return Vec::new();
            }
            state.timeline.composer = state.composer_drafts.composer_for_room(&room_id);
            // #1037: only prepared-upload sends accept a main draft through
            // this action, after every upload was queued, so their pending
            // echoes are already in the live Room timeline.
            let navigation_effect = return_main_pane_to_live_for_accepted_send(state, &room_id);
            let mut effects = vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })];
            effects.extend(navigation_effect);
            effects
        }
        crate::ComposerTarget::Thread {
            room_id,
            root_event_id,
        } => {
            if !room_exists(state, &room_id) {
                return Vec::new();
            }
            let settled = if consumes_draft {
                state.composer_drafts.advance_thread_revision(
                    &room_id,
                    &root_event_id,
                    submitted_revision,
                )
            } else {
                state.composer_drafts.settle_thread_revision(
                    &room_id,
                    &root_event_id,
                    submitted_revision,
                )
            };
            if settled.is_err() {
                return Vec::new();
            }
            if let crate::state::ThreadPaneState::Open {
                room_id: open_room_id,
                root_event_id: open_root_event_id,
                composer,
                ..
            } = &mut state.thread
                && open_room_id == &room_id
                && open_root_event_id == &root_event_id
            {
                *composer = state
                    .composer_drafts
                    .composer_for_thread(&room_id, &root_event_id);
                return vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)];
            }
            Vec::new()
        }
    }
}

pub(crate) fn handle_send_text_submitted(
    state: &mut AppState,
    room_id: String,
    transaction_id: String,
    body: String,
    draft_revision: ComposerDraftRevision,
) -> Vec<AppEffect> {
    if !is_session_ready(state)
        || state.timeline.room_id.as_deref() != Some(room_id.as_str())
        || state.timeline.composer.pending_transaction_id.is_some()
    {
        return Vec::new();
    }

    state.timeline.composer.pending_transaction_id = Some(transaction_id.clone());
    state.timeline.composer.pending_send_kind = Some(match &state.timeline.composer.mode {
        ComposerMode::Plain => PendingComposerSendKind::Plain,
        ComposerMode::Reply {
            in_reply_to_event_id,
        } => PendingComposerSendKind::Reply {
            in_reply_to_event_id: in_reply_to_event_id.clone(),
        },
    });
    let Ok(accepted_revision) = state
        .composer_drafts
        .advance_room_revision(&room_id, draft_revision)
    else {
        return Vec::new();
    };
    let navigation_effect = return_main_pane_to_live_for_accepted_send(state, &room_id);
    let accepted_composer = state.composer_drafts.composer_for_room(&room_id);
    state.timeline.composer.draft = accepted_composer.draft;
    state.timeline.composer.document = accepted_composer.document;
    state.timeline.composer.draft_revision = accepted_revision;
    state.timeline.composer.last_accepted_clear_revision =
        accepted_composer.last_accepted_clear_revision;
    let mut effects = vec![
        AppEffect::SendText {
            room_id: room_id.clone(),
            transaction_id,
            body,
        },
        AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }),
    ];
    effects.extend(navigation_effect);
    effects
}

pub(crate) fn handle_composer_submission_accepted(
    state: &mut AppState,
    submission_id: SubmissionId,
    room_id: String,
    transaction_id: String,
    body: String,
    draft_revision: ComposerDraftRevision,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    if state
        .timeline
        .submission_registry
        .accepted_submission_ids
        .contains(&submission_id)
        || state
            .timeline
            .submission_registry
            .settled_submission_ids
            .contains(&submission_id)
    {
        return Vec::new();
    }
    state.timeline.submission_registry.remember_accepted(
        submission_id.clone(),
        transaction_id.clone(),
        crate::ComposerSubmissionTarget::Main {
            room_id: room_id.clone(),
        },
    );
    if ComposerDraftRevision::checked_successor(
        state.composer_drafts.room_revision(&room_id),
        draft_revision,
    )
    .is_err()
    {
        return Vec::new();
    }
    if state.timeline.room_id.as_deref() != Some(room_id.as_str())
        || state.timeline.composer.pending_submission_id.is_some()
        || state.timeline.composer.pending_transaction_id.is_some()
        || state
            .timeline
            .composer
            .accepted_submission_ids
            .contains(&submission_id)
    {
        return Vec::new();
    }
    state
        .timeline
        .composer
        .remember_accepted_submission(submission_id.clone());
    let navigation_effect = return_main_pane_to_live_for_accepted_send(state, &room_id);
    state.timeline.composer.pending_submission_id = Some(submission_id);
    state.timeline.composer.pending_transaction_id = Some(transaction_id.clone());
    state.timeline.composer.pending_send_kind = Some(match &state.timeline.composer.mode {
        ComposerMode::Plain => PendingComposerSendKind::Plain,
        ComposerMode::Reply {
            in_reply_to_event_id,
        } => PendingComposerSendKind::Reply {
            in_reply_to_event_id: in_reply_to_event_id.clone(),
        },
    });
    let accepted_composer = state.composer_drafts.composer_for_room(&room_id);
    state.timeline.composer.draft = accepted_composer.draft;
    state.timeline.composer.document = accepted_composer.document;
    state.timeline.composer.draft_revision = accepted_composer.draft_revision;
    state.timeline.composer.last_accepted_clear_revision =
        accepted_composer.last_accepted_clear_revision;
    let mut effects = vec![
        AppEffect::SendText {
            room_id: room_id.clone(),
            transaction_id,
            body,
        },
        AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }),
    ];
    effects.extend(navigation_effect);
    effects
}

pub(crate) fn handle_composer_submission_finished(
    state: &mut AppState,
    submission_id: SubmissionId,
    room_id: String,
    transaction_id: String,
) -> Vec<AppEffect> {
    if state.timeline.composer.pending_submission_id.as_ref() != Some(&submission_id) {
        return Vec::new();
    }
    let effects = handle_send_text_finished(state, room_id, transaction_id);
    if !effects.is_empty() {
        state.timeline.composer.pending_submission_id = None;
    }
    effects
}

pub(crate) fn handle_send_text_finished(
    state: &mut AppState,
    room_id: String,
    transaction_id: String,
) -> Vec<AppEffect> {
    if state.timeline.composer.pending_submission_id.is_some() {
        return Vec::new();
    }
    if !is_session_ready(state)
        || state.timeline.room_id.as_deref() != Some(room_id.as_str())
        || state.timeline.composer.pending_transaction_id.as_deref()
            != Some(transaction_id.as_str())
    {
        return Vec::new();
    }

    let pending_send_kind = state.timeline.composer.pending_send_kind.take();
    state.timeline.composer.pending_transaction_id = None;
    if let Some(PendingComposerSendKind::Reply {
        in_reply_to_event_id,
    }) = pending_send_kind
        && state.timeline.composer.mode
            == (ComposerMode::Reply {
                in_reply_to_event_id,
            })
    {
        state.timeline.composer.mode = ComposerMode::Plain;
    }
    vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
}

pub(crate) fn handle_send_text_failed(
    state: &mut AppState,
    room_id: String,
    transaction_id: String,
    message: String,
) -> Vec<AppEffect> {
    if state.timeline.composer.pending_submission_id.is_some() {
        return Vec::new();
    }
    if !is_session_ready(state)
        || state.timeline.room_id.as_deref() != Some(room_id.as_str())
        || state.timeline.composer.pending_transaction_id.as_deref()
            != Some(transaction_id.as_str())
    {
        return Vec::new();
    }

    state.timeline.composer.pending_transaction_id = None;
    state.timeline.composer.pending_send_kind = None;
    state.errors.push(AppError {
        code: "send_text_failed".to_owned(),
        message,
        recoverable: true,
    });
    vec![
        AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }),
        AppEffect::EmitUiEvent(UiEvent::ErrorChanged),
    ]
}

pub(crate) fn handle_composer_reply_target_selected(
    state: &mut AppState,
    room_id: String,
    event_id: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state) || state.timeline.room_id.as_deref() != Some(room_id.as_str()) {
        return Vec::new();
    }
    state.timeline.composer.mode = ComposerMode::Reply {
        in_reply_to_event_id: event_id,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
}

pub(crate) fn handle_composer_reply_cancelled(state: &mut AppState) -> Vec<AppEffect> {
    let Some(room_id) = state.timeline.room_id.clone() else {
        return Vec::new();
    };
    if state.timeline.composer.mode == ComposerMode::Plain {
        return Vec::new();
    }
    state.timeline.composer.mode = ComposerMode::Plain;
    vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
}

// --- Private helpers ---

/// #1037: pending outbound messages are projected into the live Room timeline,
/// so a main-composer send (text or attachments) accepted for the selected room
/// returns the main pane to live immediately on local acceptance (no server
/// acknowledgement or remote echo is awaited). Callers invoke this only after
/// every acceptance, revision, target-room, and duplicate guard has passed.
///
/// An anchored main pane reuses the focused-context close transition (clears
/// the main anchor and the room's stale scroll anchor; Core releases the
/// focused timeline because `focused_context` closed) and the return-to-live
/// transition (event navigation -> Idle, which lets Core release any in-flight
/// navigation owner). A live main pane keeps any independent right-panel
/// focused context. Either way the returned effect asks Core to cancel the
/// main-pane navigation it still owns for this room (a date jump or event
/// navigation awaiting its focused projection), which the reducer cannot see.
fn return_main_pane_to_live_for_accepted_send(
    state: &mut AppState,
    room_id: &str,
) -> Option<AppEffect> {
    if state.navigation.active_room_id.as_deref() != Some(room_id) {
        return None;
    }
    if state.navigation.main_timeline_anchor.is_some() {
        super::thread::handle_close_focused_context(state);
        super::navigation::handle_return_main_timeline_to_live(state, room_id.to_owned());
    }
    Some(AppEffect::CancelPendingMainTimelineNavigation {
        room_id: room_id.to_owned(),
    })
}

fn composer_target_is_active(state: &AppState, target: &crate::ComposerTarget) -> bool {
    match target {
        crate::ComposerTarget::Main { room_id } => {
            state.timeline.room_id.as_deref() == Some(room_id.as_str())
        }
        crate::ComposerTarget::Thread {
            room_id,
            root_event_id,
        } => matches!(
            &state.thread,
            ThreadPaneState::Open {
                room_id: open_room_id,
                root_event_id: open_root_event_id,
                ..
            } if open_room_id == room_id && open_root_event_id == root_event_id
        ),
    }
}

fn refresh_and_emit_upload_target(
    state: &mut AppState,
    target: &crate::ComposerTarget,
) -> Vec<AppEffect> {
    match target {
        crate::ComposerTarget::Main { room_id } => {
            refresh_timeline_upload_staging(state);
            vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged {
                room_id: room_id.clone(),
            })]
        }
        crate::ComposerTarget::Thread { .. } => {
            if let ThreadPaneState::Open { staged_uploads, .. } = &mut state.thread {
                *staged_uploads = state.upload_staging.items_for_target(target);
            }
            vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
        }
    }
}

fn staged_compression_choice_is_valid_for_item(
    item: Option<&crate::state::StagedUploadItem>,
    compression_choice: StagedUploadCompressionChoice,
) -> bool {
    match (item, compression_choice) {
        (Some(item), StagedUploadCompressionChoice::NotApplicable) => {
            matches!(item.kind, crate::state::StagedUploadKind::File)
        }
        (Some(item), StagedUploadCompressionChoice::Ask)
        | (Some(item), StagedUploadCompressionChoice::Original)
        | (Some(item), StagedUploadCompressionChoice::Compressed { .. }) => {
            matches!(item.kind, crate::state::StagedUploadKind::Image { .. })
        }
        (None, _) => false,
    }
}
