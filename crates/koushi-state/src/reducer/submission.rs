use crate::{
    AppEffect, AppState, ComposerMode, ComposerSubmissionTarget, ComposerSubmissionTerminalOutcome,
    PendingComposerSendKind, SubmissionId, ThreadPaneState, UiEvent, state::AppError,
};

use super::is_session_ready;

pub(crate) fn handle_settled(
    state: &mut AppState,
    submission_id: SubmissionId,
    transaction_id: String,
    target: ComposerSubmissionTarget,
    outcome: ComposerSubmissionTerminalOutcome,
) -> Vec<AppEffect> {
    if !is_session_ready(state) {
        return Vec::new();
    }
    let changed = match target {
        ComposerSubmissionTarget::Main { room_id } => {
            if state.timeline.room_id.as_deref() != Some(room_id.as_str())
                || state.timeline.composer.pending_submission_id.as_ref() != Some(&submission_id)
                || state.timeline.composer.pending_transaction_id.as_deref()
                    != Some(transaction_id.as_str())
            {
                return Vec::new();
            }
            let pending_kind = state.timeline.composer.pending_send_kind.take();
            state.timeline.composer.pending_submission_id = None;
            state.timeline.composer.pending_transaction_id = None;
            if matches!(outcome, ComposerSubmissionTerminalOutcome::Succeeded) {
                if let Some(PendingComposerSendKind::Reply {
                    in_reply_to_event_id,
                }) = pending_kind
                {
                    if state.timeline.composer.mode
                        == (ComposerMode::Reply {
                            in_reply_to_event_id,
                        })
                    {
                        state.timeline.composer.mode = ComposerMode::Plain;
                    }
                }
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
            UiEvent::ThreadChanged
        }
    };
    let mut effects = vec![AppEffect::EmitUiEvent(changed)];
    if let ComposerSubmissionTerminalOutcome::Failed { message } = outcome {
        state.errors.push(AppError {
            code: "send_text_failed".to_owned(),
            message,
            recoverable: true,
        });
        effects.push(AppEffect::EmitUiEvent(UiEvent::ErrorChanged));
    }
    effects
}
