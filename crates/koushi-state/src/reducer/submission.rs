use crate::{
    AppEffect, AppState, ComposerDraftRevision, ComposerMode, ComposerSubmissionTarget,
    ComposerSubmissionTerminalOutcome, PendingComposerSendKind, SubmissionId, ThreadPaneState,
    UiEvent, state::AppError,
};

use super::is_session_ready;

/// SDK enqueue is the durable acceptance point. Release only the matching
/// composer; the submission registry continues to track its remote terminal.
pub(crate) fn handle_queued(
    state: &mut AppState,
    submission_id: SubmissionId,
    transaction_id: String,
    target: ComposerSubmissionTarget,
    draft_revision: ComposerDraftRevision,
) -> Vec<AppEffect> {
    if !state
        .timeline
        .submission_registry
        .active_matches(&submission_id, &transaction_id, &target)
    {
        return Vec::new();
    }
    state
        .timeline
        .submission_registry
        .remember_queued(submission_id.clone());
    match target {
        ComposerSubmissionTarget::Main { room_id } => {
            let was_cleared = state.composer_drafts.room_revision(&room_id) <= draft_revision;
            if state
                .composer_drafts
                .advance_room_revision(&room_id, draft_revision)
                .is_err()
            {
                return Vec::new();
            }
            if state.timeline.room_id.as_deref() != Some(room_id.as_str())
                || state.timeline.composer.pending_submission_id.as_ref() != Some(&submission_id)
                || state.timeline.composer.pending_transaction_id.as_deref()
                    != Some(transaction_id.as_str())
            {
                return Vec::new();
            }
            let accepted_composer = state.composer_drafts.composer_for_room(&room_id);
            let pending_kind = state.timeline.composer.pending_send_kind.take();
            state.timeline.composer.pending_submission_id = None;
            state.timeline.composer.pending_transaction_id = None;
            state.timeline.composer.draft = accepted_composer.draft;
            state.timeline.composer.document = accepted_composer.document;
            state.timeline.composer.draft_revision = accepted_composer.draft_revision;
            state.timeline.composer.last_accepted_clear_revision =
                accepted_composer.last_accepted_clear_revision;
            if let Some(PendingComposerSendKind::Reply {
                in_reply_to_event_id,
            }) = pending_kind
                && was_cleared
                && state.timeline.composer.mode
                    == (ComposerMode::Reply {
                        in_reply_to_event_id,
                    })
            {
                state.timeline.composer.mode = ComposerMode::Plain;
            }
            vec![AppEffect::EmitUiEvent(UiEvent::TimelineChanged { room_id })]
        }
        ComposerSubmissionTarget::Thread {
            room_id,
            root_event_id,
        } => {
            if state
                .composer_drafts
                .advance_thread_revision(&room_id, &root_event_id, draft_revision)
                .is_err()
            {
                return Vec::new();
            }
            let accepted_composer = state
                .composer_drafts
                .composer_for_thread(&room_id, &root_event_id);
            let ThreadPaneState::Open {
                room_id: open_room_id,
                root_event_id: open_root_event_id,
                composer,
                ..
            } = &mut state.thread
            else {
                return Vec::new();
            };
            if open_room_id != &room_id
                || open_root_event_id != &root_event_id
                || composer.pending_submission_id.as_ref() != Some(&submission_id)
                || composer.pending_transaction_id.as_deref() != Some(transaction_id.as_str())
            {
                return Vec::new();
            }
            composer.pending_submission_id = None;
            composer.pending_transaction_id = None;
            composer.pending_send_kind = None;
            composer.draft = accepted_composer.draft;
            composer.document = accepted_composer.document;
            composer.draft_revision = accepted_composer.draft_revision;
            composer.last_accepted_clear_revision = accepted_composer.last_accepted_clear_revision;
            vec![AppEffect::EmitUiEvent(UiEvent::ThreadChanged)]
        }
    }
}

pub(crate) fn handle_settled(
    state: &mut AppState,
    submission_id: SubmissionId,
    transaction_id: String,
    target: ComposerSubmissionTarget,
    outcome: ComposerSubmissionTerminalOutcome,
) -> Vec<AppEffect> {
    if !state
        .timeline
        .submission_registry
        .active_matches(&submission_id, &transaction_id, &target)
    {
        return Vec::new();
    }
    state
        .timeline
        .submission_registry
        .remember_settled(submission_id.clone());
    if !is_session_ready(state) {
        return Vec::new();
    }
    let changed = match target {
        ComposerSubmissionTarget::Main { room_id } => {
            if state.timeline.room_id.as_deref() != Some(room_id.as_str()) {
                return Vec::new();
            }
            if state.timeline.composer.pending_submission_id.as_ref() != Some(&submission_id)
                || state.timeline.composer.pending_transaction_id.as_deref()
                    != Some(transaction_id.as_str())
            {
                return Vec::new();
            }
            let pending_kind = state.timeline.composer.pending_send_kind.take();
            state.timeline.composer.pending_submission_id = None;
            state.timeline.composer.pending_transaction_id = None;
            if matches!(outcome, ComposerSubmissionTerminalOutcome::Succeeded)
                && let Some(PendingComposerSendKind::Reply {
                    in_reply_to_event_id,
                }) = pending_kind
                && state.timeline.composer.mode
                    == (ComposerMode::Reply {
                        in_reply_to_event_id,
                    })
            {
                state.timeline.composer.mode = ComposerMode::Plain;
            }
            UiEvent::TimelineChanged { room_id }
        }
        ComposerSubmissionTarget::Thread {
            room_id,
            root_event_id,
        } => {
            let ThreadPaneState::Open {
                room_id: open_room_id,
                root_event_id: open_root_event_id,
                composer,
                ..
            } = &mut state.thread
            else {
                return Vec::new();
            };
            if open_room_id != &room_id || open_root_event_id != &root_event_id {
                return Vec::new();
            }
            if composer.pending_submission_id.as_ref() != Some(&submission_id)
                || composer.pending_transaction_id.as_deref() != Some(transaction_id.as_str())
            {
                return Vec::new();
            }
            composer.pending_submission_id = None;
            composer.pending_transaction_id = None;
            composer.pending_send_kind = None;
            UiEvent::ThreadChanged
        }
    };
    let mut effects = vec![AppEffect::EmitUiEvent(changed)];
    if let ComposerSubmissionTerminalOutcome::Failed { message } = outcome {
        state.errors.push(AppError {
            code: "send_text_failed".to_owned(),
            message,
            recoverable: true,
            reason: None,
        });
        effects.push(AppEffect::EmitUiEvent(UiEvent::ErrorChanged));
    }
    effects
}
