use crate::SubmissionId;
use crate::{
    effect::{AppEffect, UiEvent},
    state::{
        AppError, AppState, ComposerMode, PendingComposerSendKind, StagedUploadCompressionChoice,
        ThreadPaneState,
    },
};

use super::{
    is_session_ready, refresh_timeline_media_gallery, refresh_timeline_scheduled_sends,
    refresh_timeline_upload_staging, room_exists,
};

const TIMELINE_SUBSCRIPTION_FAILED_MESSAGE: &str = "Matrix timeline subscription failed";

pub(crate) fn handle_timeline_subscribed(state: &mut AppState, room_id: String) -> Vec<AppEffect> {
    if !is_session_ready(state) || state.timeline.room_id.as_deref() != Some(room_id.as_str()) {
        return Vec::new();
    }

    state.timeline.is_subscribed = true;
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

pub(crate) fn handle_timeline_back_pagination_requested(
    state: &mut AppState,
    room_id: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state)
        || state.timeline.room_id.as_deref() != Some(room_id.as_str())
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
    if !is_session_ready(state)
        || state.timeline.room_id.as_deref() != Some(room_id.as_str())
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
    if !is_session_ready(state) || !room_exists(state, &item.room_id) {
        return Vec::new();
    }

    let room_id = item.room_id.clone();
    state.scheduled_sends.insert(item);
    state.composer_drafts.clear_room_draft(&room_id);
    if state.timeline.room_id.as_deref() == Some(room_id.as_str()) {
        state.timeline.composer = Default::default();
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
    send_at_ms: u64,
    handle: crate::state::ScheduledSendHandle,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let Some(item) = state
        .scheduled_sends
        .reschedule(&scheduled_id, send_at_ms, handle)
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
    room_id: String,
    items: Vec<crate::state::StagedUploadItem>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) || !room_exists(state, &room_id) {
        return Vec::new();
    }

    state.upload_staging.replace_room_items(&room_id, items);
    if state.timeline.room_id.as_deref() == Some(room_id.as_str()) {
        refresh_timeline_upload_staging(state);
        return vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })];
    }
    Vec::new()
}

pub(crate) fn handle_upload_staging_caption_changed(
    state: &mut AppState,
    staged_id: String,
    caption: Option<crate::FormattedMessageDraft>,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }

    let Some(item) = state.upload_staging.update_caption(&staged_id, caption) else {
        return Vec::new();
    };
    let room_id = item.room_id;
    if state.timeline.room_id.as_deref() == Some(room_id.as_str()) {
        refresh_timeline_upload_staging(state);
        return vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })];
    }
    Vec::new()
}

pub(crate) fn handle_upload_staging_compression_changed(
    state: &mut AppState,
    staged_id: String,
    compression_choice: StagedUploadCompressionChoice,
) -> Vec<AppEffect> {
    if !is_session_ready(state)
        || !staged_compression_choice_is_valid_for_item(
            state.upload_staging.items.get(&staged_id),
            compression_choice,
        )
    {
        return Vec::new();
    }

    let Some(item) = state
        .upload_staging
        .update_compression_choice(&staged_id, compression_choice)
    else {
        return Vec::new();
    };
    let room_id = item.room_id;
    if state.timeline.room_id.as_deref() == Some(room_id.as_str()) {
        refresh_timeline_upload_staging(state);
        return vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })];
    }
    Vec::new()
}

pub(crate) fn handle_upload_staging_cleared(
    state: &mut AppState,
    room_id: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state) || !state.upload_staging.clear_room(&room_id) {
        return Vec::new();
    }
    if state.timeline.room_id.as_deref() == Some(room_id.as_str()) {
        refresh_timeline_upload_staging(state);
        return vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })];
    }
    Vec::new()
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
    draft: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state) || state.timeline.room_id.as_deref() != Some(room_id.as_str()) {
        return Vec::new();
    }

    state.timeline.composer.draft = draft.clone();
    state.composer_drafts.set_room_draft(room_id.clone(), draft);
    vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
}

pub(crate) fn handle_send_text_submitted(
    state: &mut AppState,
    room_id: String,
    transaction_id: String,
    body: String,
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
    state.timeline.composer.draft.clear();
    state.composer_drafts.clear_room_draft(&room_id);
    vec![
        AppEffect::SendText {
            room_id: room_id.clone(),
            transaction_id,
            body,
        },
        AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id }),
    ]
}

pub(crate) fn handle_composer_submission_accepted(
    state: &mut AppState,
    submission_id: SubmissionId,
    room_id: String,
    transaction_id: String,
    body: String,
) -> Vec<AppEffect> {
    if !is_session_ready(state)
        || state.timeline.room_id.as_deref() != Some(room_id.as_str())
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
    state.timeline.composer.pending_submission_id = Some(submission_id);
    handle_send_text_submitted(state, room_id, transaction_id, body)
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
    {
        if state.timeline.composer.mode
            == (ComposerMode::Reply {
                in_reply_to_event_id,
            })
        {
            state.timeline.composer.mode = ComposerMode::Plain;
        }
    }
    vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
}

pub(crate) fn handle_send_text_failed(
    state: &mut AppState,
    room_id: String,
    transaction_id: String,
    message: String,
) -> Vec<AppEffect> {
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
