use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_sdk::MatrixClientSession;
use koushi_sdk::MatrixUserProfile;
use koushi_search::{AttachmentDocument, SensitiveString};
use koushi_state::UserProfile;
use koushi_state::{
    AppAction, AttachmentKind, AvatarImage, AvatarThumbnailState, ComposerDocument, ComposerInline,
    LiveEventReceiptSummaryUpdate, LiveEventReceipts, LiveReadReceipt, MentionIntent,
    MentionTarget, ReplyQuote, ReplyQuoteCodeBlock, ReplyQuoteFormattedBody, ReplyQuoteState,
};

use matrix_sdk::attachment::{AttachmentInfo, BaseFileInfo, BaseImageInfo, Thumbnail};
use matrix_sdk::media::{MediaFormat, MediaRequestParameters, MediaThumbnailSettings};
use matrix_sdk::room::edit::EditedContent;
use matrix_sdk::room::reply::{EnforceThread, Reply};
use matrix_sdk::ruma::events::room::message::FormattedBody;
use matrix_sdk::ruma::events::room::message::{
    AddMentions, MessageFormat, MessageType, ReplyWithinThread,
    RoomMessageEventContentWithoutRelation,
};
use matrix_sdk::ruma::events::room::{MediaSource, ThumbnailInfo};
use matrix_sdk::ruma::events::{StateEventContentChange, room::name::RoomNameEventContent};
use matrix_sdk::ruma::html::{Html, SanitizerConfig};
use matrix_sdk::send_queue::{LocalEcho, LocalEchoContent, SendHandle};
use matrix_sdk_ui::timeline::{
    AnyOtherStateEventContentChange, EmbeddedEvent, EncryptedMessage,
    EventSendState as SdkEventSendState, EventTimelineItem, InReplyToDetails, MembershipChange,
    Profile, ReactionStatus, ReactionsByKeyBySender, Timeline, TimelineDetails,
    TimelineEventItemId, TimelineItem as SdkTimelineItem, TimelineItemContent, TimelineItemKind,
};
use tokio::sync::mpsc;

use crate::account_work::AccountWorkKind;
use crate::event_projection::{
    message_actions_for_timeline_item, message_source_for_timeline_item,
};
use crate::executor;
use crate::link_preview::{LinkPreviewContext, extract_link_ranges};
use crate::search::SearchIndexMessage;
use koushi_protocol::command::{MediaDownloadSelection, UploadMediaKind, UploadMediaRequest};
use koushi_protocol::event::{
    CoreEvent, LinkPreview, LinkPreviewState, ReactionSender, RoomKeyRequestStage,
    RoomKeyRequestStateDto, RoomKeyRequestWithheldCode, ThreadSummaryDto, TimelineDiff,
    TimelineEvent, TimelineItem, TimelineItemId, TimelineMedia, TimelineMediaKind,
    TimelineMediaSource, TimelineMediaThumbnail, TimelineMegolmSessionReason,
    TimelineMessageActions, TimelineMessageKind, TimelineMessageSource, TimelineNoticeI18n,
    TimelineNoticeI18nKey, TimelineSendFailureReason, TimelineSendState, TimelineSpoilerSpan,
    TimelineUnableToDecrypt, TimelineUnableToDecryptReason, TimelineViewportObservation,
};
use koushi_protocol::failure::{CoreFailure, TimelineFailureKind};
use koushi_protocol::ids::{RequestId, TimelineKey, TimelineKind};

// BEGIN GENERATED SIBLING IMPORTS
use super::actor::{
    TimelineActor, TimelineActorMessage, canonical_activity_window_action,
    reserve_canonical_activity_action,
};
use super::composer::ruma_mentions_from_intent;
use super::diagnostics::{
    trace_timeline_actor_operation, trace_timeline_actor_scan, trace_timeline_link_preview,
};
use super::manager::TimelineManagerActor;
use super::media::PrivateMediaEntry;
use super::navigation::{TimelineActorGenerationGate, send_generation_fenced};
use super::room_key_recovery::{DecryptRetryReason, KeyRequestUiState};
// END GENERATED SIBLING IMPORTS

pub(super) const REPLY_QUOTE_PREVIEW_MAX_CHARS: usize = 160;

impl TimelineManagerActor {
    pub(super) async fn handle_ignored_users_updated(
        &mut self,
        user_ids: std::collections::BTreeSet<String>,
    ) {
        self.ignored_user_ids = user_ids.clone();
        for handle in self.timelines.values() {
            let _ = handle
                .send(TimelineActorMessage::IgnoredUsersUpdated(user_ids.clone()))
                .await;
        }
    }
}

fn spawn_link_preview_fetch(
    session: Arc<MatrixClientSession>,
    msg_tx: mpsc::Sender<TimelineActorMessage>,
    request_id: RequestId,
    event_id: String,
    previews: Vec<LinkPreview>,
) -> executor::JoinHandle<()> {
    executor::spawn(async move {
        let started = std::time::Instant::now();
        let mut updated = Vec::with_capacity(previews.len());
        let mut pending_count = 0usize;
        let mut ready_count = 0usize;
        let mut failed_count = 0usize;

        for mut preview in previews {
            if preview.state != LinkPreviewState::Pending {
                updated.push(preview);
                continue;
            }

            pending_count += 1;
            match crate::link_preview::fetch_link_preview(&session, &preview.url).await {
                Ok(fetched) => {
                    updated.push(fetched);
                    ready_count += 1;
                }
                Err(_) => {
                    preview.state = LinkPreviewState::Failed;
                    updated.push(preview);
                    failed_count += 1;
                }
            }
        }

        let _ = msg_tx
            .send(TimelineActorMessage::LinkPreviewsFetched {
                request_id,
                event_id,
                previews: updated,
                pending_count,
                ready_count,
                failed_count,
                elapsed_ms: started.elapsed().as_millis(),
            })
            .await;
    })
}

fn spawn_reply_detail_fetch(
    timeline: Arc<Timeline>,
    msg_tx: mpsc::Sender<TimelineActorMessage>,
    event_id: String,
) -> executor::JoinHandle<()> {
    executor::spawn(async move {
        if let Ok(parsed_event_id) = matrix_sdk::ruma::EventId::parse(event_id.as_str()) {
            let _ = timeline.fetch_details_for_event(&parsed_event_id).await;
        }
        let _ = msg_tx
            .send(TimelineActorMessage::ReplyDetailsFetchFinished { event_id })
            .await;
    })
}

struct ReactionTargetState {
    item_id: TimelineEventItemId,
    can_react: bool,
    my_reaction_event_id: Option<String>,
}

impl TimelineActor {
    pub(super) async fn handle_load_message_source(
        &mut self,
        request_id: RequestId,
        event_id: String,
    ) {
        let Some(source) = self.project_message_source_for_event(&event_id).await else {
            self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendTarget);
            return;
        };

        self.emit(CoreEvent::Timeline(TimelineEvent::MessageSourceLoaded {
            request_id,
            key: self.key.clone(),
            source,
        }));
    }
    pub(super) async fn handle_edit_text(
        &mut self,
        request_id: RequestId,
        event_id: String,
        document: ComposerDocument,
    ) {
        // Edits go through the SDK Timeline so the Set diff on the original
        // item is produced locally (send-queue local echo) instead of
        // depending on the server echoing the edit back through sync —
        // some Sliding Sync implementations do not deliver it reliably (Phase 5
        // review finding). Canon rule 1: relay the SDK.
        let candidates = self.item_ids_for_event(&event_id);
        if candidates.is_empty() {
            trace_message_edit_lifecycle("opened", "text", 0, 0, None, None);
            trace_message_edit_lifecycle("settled", "text", 0, 0, None, Some("failed"));
            self.emit_failure(
                request_id,
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::Sdk,
                },
            );
            return;
        }
        // The replacement shape is a Rust-owned decision: the GUI submits only
        // the new visible text, so core resolves what the target event actually
        // is before choosing between a caption edit and a text replacement.
        let items = self.timeline.items().await;
        let mut result = Ok(());
        let mut diagnostic_target = "text";
        let mut diagnostic_original_mention_count = 0;
        let mut diagnostic_final_mention_count = 0;
        let mut diagnostic_revision_mention_count = 0;
        for item_id in &candidates {
            let target = edit_target_msgtype(&items, item_id);
            diagnostic_target = if target.is_some_and(msgtype_carries_editable_caption) {
                "media_caption"
            } else {
                "text"
            };
            let original_mentions = mention_summary_for_message_type(target);
            diagnostic_original_mention_count =
                original_mentions.0.len() + usize::from(original_mentions.1);
            trace_message_edit_lifecycle(
                "opened",
                diagnostic_target,
                diagnostic_original_mention_count,
                0,
                None,
                None,
            );
            let content = edited_document_content_for_edit_target(target, &document);
            trace_message_edit_target(target, &content);
            let (final_mention_count, revision_mention_count) =
                mention_counts_for_edit(target, &content);
            diagnostic_final_mention_count = final_mention_count;
            diagnostic_revision_mention_count = revision_mention_count;
            trace_message_edit_lifecycle(
                "submitted",
                diagnostic_target,
                diagnostic_original_mention_count,
                final_mention_count,
                Some(revision_mention_count),
                None,
            );
            result = self.timeline.edit(item_id, content).await;
            match &result {
                Err(matrix_sdk_ui::timeline::Error::EventNotInTimeline(_)) => continue,
                _ => break,
            }
        }

        trace_message_edit_lifecycle(
            "settled",
            diagnostic_target,
            diagnostic_original_mention_count,
            diagnostic_final_mention_count,
            Some(diagnostic_revision_mention_count),
            Some(if result.is_ok() { "success" } else { "failed" }),
        );

        if result.is_err() {
            self.emit_failure(
                request_id,
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::Sdk,
                },
            );
        }
        // Edit success: the local-echo Set diff on the original item identity
        // arrives through the subscription; no dedicated EditCompleted event.
    }
    pub(super) async fn handle_redact(&mut self, request_id: RequestId, event_id: String) {
        // Same rationale as edits: redact through the SDK Timeline so the
        // diff is produced locally instead of waiting for the server echo.
        let candidates = self.item_ids_for_event(&event_id);
        if candidates.is_empty() {
            self.emit_failure(
                request_id,
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::Sdk,
                },
            );
            return;
        }
        let _interactive = self
            .account_work
            .begin_interactive(AccountWorkKind::MessageSend);
        let mut result = Ok(());
        for item_id in &candidates {
            result = self.timeline.redact(item_id, None).await;
            match &result {
                Err(matrix_sdk_ui::timeline::Error::EventNotInTimeline(_)) => continue,
                _ => break,
            }
        }

        if result.is_err() {
            self.emit_failure(
                request_id,
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::Sdk,
                },
            );
        }
        // Redact success: timeline diff reflects it (removal or redacted-state Set diff).
    }
    pub(super) async fn handle_toggle_reaction(
        &mut self,
        request_id: RequestId,
        event_id: String,
        reaction_key: String,
    ) {
        let candidates = self.item_ids_for_event(&event_id);
        if candidates.is_empty() {
            self.emit_failure(
                request_id,
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::Sdk,
                },
            );
            return;
        }

        let _interactive = self
            .account_work
            .begin_interactive(AccountWorkKind::MessageSend);
        let mut result: Result<(), matrix_sdk_ui::timeline::Error> = Ok(());
        for item_id in &candidates {
            result = self
                .timeline
                .toggle_reaction(item_id, &reaction_key)
                .await
                .map(|_| ());
            match &result {
                Err(matrix_sdk_ui::timeline::Error::EventNotInTimeline(_)) => continue,
                _ => break,
            }
        }

        if result.is_err() {
            self.emit_failure(
                request_id,
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::Sdk,
                },
            );
        }
    }
    pub(super) async fn handle_send_reaction(
        &mut self,
        request_id: RequestId,
        event_id: String,
        reaction_key: String,
    ) {
        let started = Instant::now();
        trace_timeline_actor_operation(
            "actor_start",
            "send_reaction",
            request_id,
            &self.key,
            None,
            None,
        );
        if reaction_key.trim().is_empty() {
            trace_timeline_actor_operation(
                "actor_finish",
                "send_reaction",
                request_id,
                &self.key,
                Some(started.elapsed().as_millis()),
                Some("invalid_target"),
            );
            self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidReactionTarget);
            return;
        }

        let Some(target) = self
            .reaction_target_state(request_id, "send_reaction", &event_id, &reaction_key)
            .await
        else {
            trace_timeline_actor_operation(
                "actor_finish",
                "send_reaction",
                request_id,
                &self.key,
                Some(started.elapsed().as_millis()),
                Some("target_missing"),
            );
            self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidReactionTarget);
            return;
        };
        if let Err(kind) =
            validate_send_reaction(target.can_react, target.my_reaction_event_id.as_deref())
        {
            trace_timeline_actor_operation(
                "actor_finish",
                "send_reaction",
                request_id,
                &self.key,
                Some(started.elapsed().as_millis()),
                Some("invalid_state"),
            );
            self.emit_timeline_failure(request_id, kind);
            return;
        }

        let sdk_started = Instant::now();
        match self
            .timeline
            .toggle_reaction(&target.item_id, &reaction_key)
            .await
        {
            Ok(true) => {
                trace_timeline_actor_operation(
                    "sdk_done",
                    "send_reaction",
                    request_id,
                    &self.key,
                    Some(sdk_started.elapsed().as_millis()),
                    Some("sent"),
                );
                trace_timeline_actor_operation(
                    "actor_finish",
                    "send_reaction",
                    request_id,
                    &self.key,
                    Some(started.elapsed().as_millis()),
                    Some("success"),
                );
            }
            Ok(false) => {
                trace_timeline_actor_operation(
                    "sdk_done",
                    "send_reaction",
                    request_id,
                    &self.key,
                    Some(sdk_started.elapsed().as_millis()),
                    Some("invalid_state"),
                );
                trace_timeline_actor_operation(
                    "actor_finish",
                    "send_reaction",
                    request_id,
                    &self.key,
                    Some(started.elapsed().as_millis()),
                    Some("invalid_state"),
                );
                self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidReactionState);
            }
            Err(error) => {
                trace_timeline_actor_operation(
                    "sdk_done",
                    "send_reaction",
                    request_id,
                    &self.key,
                    Some(sdk_started.elapsed().as_millis()),
                    Some("sdk_error"),
                );
                trace_timeline_actor_operation(
                    "actor_finish",
                    "send_reaction",
                    request_id,
                    &self.key,
                    Some(started.elapsed().as_millis()),
                    Some("sdk_error"),
                );
                self.emit_timeline_failure(request_id, classify_reaction_error(&error));
            }
        }
    }
    pub(super) async fn handle_redact_reaction(
        &mut self,
        request_id: RequestId,
        event_id: String,
        reaction_key: String,
        reaction_event_id: String,
    ) {
        let started = Instant::now();
        trace_timeline_actor_operation(
            "actor_start",
            "redact_reaction",
            request_id,
            &self.key,
            None,
            None,
        );
        if reaction_key.trim().is_empty() || reaction_event_id.trim().is_empty() {
            trace_timeline_actor_operation(
                "actor_finish",
                "redact_reaction",
                request_id,
                &self.key,
                Some(started.elapsed().as_millis()),
                Some("invalid_target"),
            );
            self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidReactionTarget);
            return;
        }

        let Some(target) = self
            .reaction_target_state(request_id, "redact_reaction", &event_id, &reaction_key)
            .await
        else {
            trace_timeline_actor_operation(
                "actor_finish",
                "redact_reaction",
                request_id,
                &self.key,
                Some(started.elapsed().as_millis()),
                Some("target_missing"),
            );
            self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidReactionTarget);
            return;
        };
        if let Err(kind) = validate_redact_reaction(
            target.can_react,
            target.my_reaction_event_id.as_deref(),
            &reaction_event_id,
        ) {
            trace_timeline_actor_operation(
                "actor_finish",
                "redact_reaction",
                request_id,
                &self.key,
                Some(started.elapsed().as_millis()),
                Some("invalid_state"),
            );
            self.emit_timeline_failure(request_id, kind);
            return;
        }

        let sdk_started = Instant::now();
        match self
            .timeline
            .toggle_reaction(&target.item_id, &reaction_key)
            .await
        {
            Ok(false) => {
                trace_timeline_actor_operation(
                    "sdk_done",
                    "redact_reaction",
                    request_id,
                    &self.key,
                    Some(sdk_started.elapsed().as_millis()),
                    Some("redacted"),
                );
                trace_timeline_actor_operation(
                    "actor_finish",
                    "redact_reaction",
                    request_id,
                    &self.key,
                    Some(started.elapsed().as_millis()),
                    Some("success"),
                );
            }
            Ok(true) => {
                trace_timeline_actor_operation(
                    "sdk_done",
                    "redact_reaction",
                    request_id,
                    &self.key,
                    Some(sdk_started.elapsed().as_millis()),
                    Some("invalid_state"),
                );
                trace_timeline_actor_operation(
                    "actor_finish",
                    "redact_reaction",
                    request_id,
                    &self.key,
                    Some(started.elapsed().as_millis()),
                    Some("invalid_state"),
                );
                self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidReactionState);
            }
            Err(error) => {
                trace_timeline_actor_operation(
                    "sdk_done",
                    "redact_reaction",
                    request_id,
                    &self.key,
                    Some(sdk_started.elapsed().as_millis()),
                    Some("sdk_error"),
                );
                trace_timeline_actor_operation(
                    "actor_finish",
                    "redact_reaction",
                    request_id,
                    &self.key,
                    Some(started.elapsed().as_millis()),
                    Some("sdk_error"),
                );
                self.emit_timeline_failure(request_id, classify_reaction_error(&error));
            }
        }
    }
    pub(super) async fn handle_ignored_users_updated(
        &mut self,
        user_ids: std::collections::BTreeSet<String>,
    ) {
        if self.ignored_user_ids == user_ids {
            return;
        }
        let activity_permit = reserve_canonical_activity_action(&self.action_tx, &self.key).await;
        let activity_commit_lease = if activity_permit.is_some() {
            self.timeline_actor_generations
                .try_acquire(&self.key, self.actor_generation)
        } else {
            None
        };
        if matches!(self.key.kind, TimelineKind::Room { .. })
            && (activity_permit.is_none() || activity_commit_lease.is_none())
        {
            return;
        }
        self.ignored_user_ids = user_ids;

        let mut core_diffs = Vec::new();
        for (index, item) in self.navigation_items.iter_mut().enumerate() {
            let was_hidden = item.is_hidden;
            apply_ignored_sender_suppression(item, &self.ignored_user_ids);
            if item.is_hidden != was_hidden {
                core_diffs.push(TimelineDiff::Set {
                    index,
                    item: item.clone(),
                });
            }
        }
        if core_diffs.is_empty() {
            drop(activity_commit_lease);
            drop(activity_permit);
            return;
        }

        self.emit_media_gallery_if_changed().await;

        if self.emit_non_sdk_item_sets(core_diffs) {
            if let Some(activity_permit) = activity_permit {
                activity_permit.send(vec![
                    canonical_activity_window_action(&self.key, &self.navigation_items)
                        .expect("room ignored-user Activity action"),
                ]);
            }
            self.emit_navigation_if_changed();
        }
        drop(activity_commit_lease);
    }
    fn emit_timeline_item_set(&mut self, index: usize) -> bool {
        let core_diffs = vec![TimelineDiff::Set {
            index,
            item: self.navigation_items[index].clone(),
        }];
        self.emit_non_sdk_item_sets(core_diffs)
    }
    pub(super) async fn handle_load_link_previews(
        &mut self,
        request_id: RequestId,
        event_id: String,
    ) {
        let trace_started = Some(std::time::Instant::now());
        let Some(index) = self.navigation_items.iter().position(
            |item| matches!(&item.id, TimelineItemId::Event { event_id: id } if id == &event_id),
        ) else {
            trace_timeline_link_preview(
                "lookup_miss",
                request_id,
                &self.key,
                0,
                0,
                0,
                trace_started.map(|started| started.elapsed().as_millis()),
                Some("lookup_miss"),
            );
            return;
        };

        let Some(previews) = self.navigation_items[index].link_previews.clone() else {
            trace_timeline_link_preview(
                "no_previews",
                request_id,
                &self.key,
                0,
                0,
                0,
                trace_started.map(|started| started.elapsed().as_millis()),
                Some("no_previews"),
            );
            return;
        };

        let pending_count = previews
            .iter()
            .filter(|preview| preview.state == LinkPreviewState::Pending)
            .count();
        trace_timeline_link_preview(
            "start",
            request_id,
            &self.key,
            pending_count,
            0,
            0,
            None,
            None,
        );

        if pending_count == 0 {
            trace_timeline_link_preview(
                "complete",
                request_id,
                &self.key,
                pending_count,
                0,
                0,
                trace_started.map(|started| started.elapsed().as_millis()),
                Some("unchanged"),
            );
            return;
        }

        let mut loading_previews = previews.clone();
        for preview in &mut loading_previews {
            if preview.state == LinkPreviewState::Pending {
                preview.state = LinkPreviewState::Loading;
            }
        }
        self.navigation_items[index].link_previews = Some(loading_previews);
        self.emit_timeline_item_set(index);

        let task = spawn_link_preview_fetch(
            self.session.clone(),
            self.msg_tx.clone(),
            request_id,
            event_id.clone(),
            previews,
        );
        if let Some(previous) = self.link_preview_fetches.insert(event_id, task) {
            previous.abort();
        }
    }
    pub(super) async fn handle_link_previews_fetched(
        &mut self,
        request_id: RequestId,
        event_id: String,
        previews: Vec<LinkPreview>,
        pending_count: usize,
        ready_count: usize,
        failed_count: usize,
        elapsed_ms: u128,
    ) {
        if self.link_preview_fetches.remove(&event_id).is_none() {
            trace_timeline_link_preview(
                "complete",
                request_id,
                &self.key,
                pending_count,
                ready_count,
                failed_count,
                Some(elapsed_ms),
                Some("discarded"),
            );
            return;
        }
        let Some(index) = self.navigation_items.iter().position(
            |item| matches!(&item.id, TimelineItemId::Event { event_id: id } if id == &event_id),
        ) else {
            trace_timeline_link_preview(
                "lookup_miss",
                request_id,
                &self.key,
                pending_count,
                ready_count,
                failed_count,
                Some(elapsed_ms),
                Some("lookup_miss"),
            );
            return;
        };

        let Some(current_previews) = self.navigation_items[index].link_previews.as_mut() else {
            trace_timeline_link_preview(
                "complete",
                request_id,
                &self.key,
                pending_count,
                ready_count,
                failed_count,
                Some(elapsed_ms),
                Some("discarded"),
            );
            return;
        };

        let fetched_by_url: HashMap<String, LinkPreview> = previews
            .into_iter()
            .map(|preview| (preview.url.clone(), preview))
            .collect();
        let mut changed = false;
        for current in current_previews {
            if current.state != LinkPreviewState::Pending
                && current.state != LinkPreviewState::Loading
            {
                continue;
            }
            if let Some(fetched) = fetched_by_url.get(&current.url) {
                if fetched.state == LinkPreviewState::Ready {
                    self.link_preview_policy
                        .cache
                        .insert(fetched.url.clone(), fetched.clone());
                }
                if current != fetched {
                    *current = fetched.clone();
                    changed = true;
                }
            }
        }

        if changed {
            self.emit_timeline_item_set(index);
        }
        trace_timeline_link_preview(
            "complete",
            request_id,
            &self.key,
            pending_count,
            ready_count,
            failed_count,
            Some(elapsed_ms),
            Some(if changed { "updated" } else { "discarded" }),
        );
    }
    pub(super) fn handle_cancel_link_previews(&mut self, request_id: RequestId) {
        let fetch_count = self.link_preview_fetches.len();
        if fetch_count == 0 {
            return;
        }

        for (_, task) in self.link_preview_fetches.drain() {
            task.abort();
        }

        let mut core_diffs = Vec::new();
        for (index, item) in self.navigation_items.iter_mut().enumerate() {
            if reset_loading_link_previews_to_pending(item) {
                core_diffs.push(TimelineDiff::Set {
                    index,
                    item: item.clone(),
                });
            }
        }

        trace_timeline_link_preview(
            "cancelled",
            request_id,
            &self.key,
            fetch_count,
            0,
            0,
            None,
            Some("cancelled"),
        );

        if core_diffs.is_empty() {
            return;
        }

        let _ = self.emit_non_sdk_item_sets(core_diffs);
    }
    pub(super) async fn handle_hide_link_preview(
        &mut self,
        _request_id: RequestId,
        event_id: String,
    ) {
        let mut context = self.link_preview_policy.for_room(self.key.room_id());
        if !context.hidden_event_ids.insert(event_id.clone()) {
            return;
        }
        self.link_preview_policy.hidden_event_ids = context.hidden_event_ids.clone();

        let mut core_diffs = Vec::new();
        for (index, item) in self.navigation_items.iter_mut().enumerate() {
            if matches!(&item.id, TimelineItemId::Event { event_id: id } if id == &event_id) {
                apply_link_previews_to_item(item, self.key.room_id(), &context, &self.session)
                    .await;
                core_diffs.push(TimelineDiff::Set {
                    index,
                    item: item.clone(),
                });
            }
        }

        if core_diffs.is_empty() {
            return;
        }

        let _ = self.emit_non_sdk_item_sets(core_diffs);
    }
    pub(super) async fn handle_link_preview_policy_changed(
        &mut self,
        unencrypted_global_enabled: bool,
        encrypted_global_enabled: bool,
        room_enabled: Option<bool>,
    ) {
        self.link_preview_policy.apply_policy_delta(
            unencrypted_global_enabled,
            encrypted_global_enabled,
            room_enabled,
        );
        let context = self.link_preview_policy.for_room(self.key.room_id());

        let mut core_diffs = Vec::new();
        for (index, item) in self.navigation_items.iter_mut().enumerate() {
            let old = item.link_previews.clone();
            apply_link_previews_to_item(item, self.key.room_id(), &context, &self.session).await;
            if item.link_previews != old {
                core_diffs.push(TimelineDiff::Set {
                    index,
                    item: item.clone(),
                });
            }
        }

        if core_diffs.is_empty() {
            return;
        }

        let _ = self.emit_non_sdk_item_sets(core_diffs);
    }
    pub(super) fn maybe_fetch_visible_reply_details(&mut self) {
        let event_ids = visible_missing_reply_detail_event_ids(
            &self.navigation_items,
            &self.viewport_observation,
            &self.reply_detail_fetch_attempted_event_ids,
        );
        for event_id in event_ids {
            if !self
                .reply_detail_fetch_attempted_event_ids
                .insert(event_id.clone())
            {
                continue;
            }
            let task = spawn_reply_detail_fetch(
                self.timeline.clone(),
                self.msg_tx.clone(),
                event_id.clone(),
            );
            if let Some(previous) = self.reply_detail_fetches.insert(event_id, task) {
                previous.abort();
            }
        }
    }
    /// Forward SDK diff mutations to the search index channel reliably.
    /// Redactions are privacy-sensitive removals and must not be silently
    /// dropped as freshness-only updates.
    async fn forward_diff_to_search(&self, diff: &eyeball_im::VectorDiff<Arc<SdkTimelineItem>>) {
        let _ = self
            .emit_search_messages_reliable(self.search_index_messages_for_diff(diff))
            .await;
    }
    pub(super) fn search_index_messages_for_diff(
        &self,
        diff: &eyeball_im::VectorDiff<Arc<SdkTimelineItem>>,
    ) -> Vec<SearchIndexMessage> {
        use eyeball_im::VectorDiff;

        let room_id = match &self.key.kind {
            TimelineKind::Room { room_id }
            | TimelineKind::Thread { room_id, .. }
            | TimelineKind::Focused { room_id, .. } => room_id.as_str(),
        };

        match diff {
            VectorDiff::PushFront { value }
            | VectorDiff::PushBack { value }
            | VectorDiff::Insert { value, .. }
            | VectorDiff::Set { value, .. } => self.search_index_messages_for_item(room_id, value),
            VectorDiff::Append { values } | VectorDiff::Reset { values } => values
                .iter()
                .flat_map(|item| self.search_index_messages_for_item(room_id, item))
                .collect(),
            VectorDiff::Remove { .. }
            | VectorDiff::Truncate { .. }
            | VectorDiff::Clear
            | VectorDiff::PopFront
            | VectorDiff::PopBack => {
                // Remove/truncate/clear: we don't know which event_ids are affected
                // without tracking the full timeline list; skip search forwarding.
                // Redactions arrive as Set-with-is_redacted=true before any Remove.
                Vec::new()
            }
        }
    }
    fn search_index_messages_for_item(
        &self,
        room_id: &str,
        item: &Arc<SdkTimelineItem>,
    ) -> Vec<SearchIndexMessage> {
        use matrix_sdk_ui::timeline::TimelineItemKind;
        let event_item = match item.kind() {
            TimelineItemKind::Event(e) => e,
            TimelineItemKind::Virtual(_) => return Vec::new(),
        };

        // Only remote events have a stable event_id we can index.
        let event_id = match event_item.event_id() {
            Some(id) => id.to_string(),
            None => return Vec::new(), // local-echo without confirmed event_id: skip
        };

        let sender = event_item.sender().to_string();
        let timestamp_ms: u64 = event_item.timestamp().0.into();

        // Redacted items: forward Redact so the document is removed.
        if event_item.content().is_redacted() {
            return vec![SearchIndexMessage::Redact { event_id }];
        }

        let (body, attachment_filename, attachment, edit_event_id) =
            if let Some(sticker) = event_item.content().as_sticker() {
                (
                    None,
                    Some(sticker.content().body.clone()),
                    Some(Self::attachment_document_from_sticker(sticker)),
                    None,
                )
            } else if let Some(message) = event_item.content().as_message() {
                let projection = message_projection_from_msgtype(message.msgtype(), message.body());

                // Detect edits: when is_edited() is true, the SDK ngram index will
                // index the edit event under the edit event_id (not the original).
                // We must register an alias so verify_candidate can resolve it back.
                // Extract the edit event_id from latest_edit_json if available.
                let edit_event_id = if message.is_edited() {
                    event_item
                        .latest_edit_json()
                        .and_then(|raw| {
                            raw.get_field::<matrix_sdk::ruma::OwnedEventId>("event_id")
                                .ok()
                                .flatten()
                        })
                        .map(|id| id.to_string())
                } else {
                    None
                };

                (
                    projection.body,
                    projection
                        .media
                        .as_ref()
                        .map(|media| media.filename.clone()),
                    projection.media.as_ref().and_then(|media| {
                        Self::attachment_document_from_timeline_media(media, event_item, message)
                    }),
                    edit_event_id,
                )
            } else {
                return Vec::new();
            };

        if body.is_none() && attachment_filename.is_none() {
            return Vec::new();
        }

        if let Some(edit_event_id) = edit_event_id {
            // Edited message: Upsert original with new canonical body, AND
            // forward Edit so the document store registers the alias
            // (edit_event_id → original_event_id) used by verify_candidate.
            vec![
                SearchIndexMessage::Upsert {
                    room_id: room_id.to_owned(),
                    event_id: event_id.clone(),
                    sender: sender.clone(),
                    timestamp_ms,
                    body: body.clone(),
                    attachment_filename: attachment_filename.clone(),
                    attachment: attachment.clone(),
                },
                SearchIndexMessage::Edit {
                    edit_event_id,
                    target_event_id: event_id,
                    sender,
                    timestamp_ms,
                    body,
                    attachment_filename,
                    attachment,
                },
            ]
        } else {
            // New (unedited) message: Upsert into document store.
            vec![SearchIndexMessage::Upsert {
                room_id: room_id.to_owned(),
                event_id,
                sender,
                timestamp_ms,
                body,
                attachment_filename,
                attachment,
            }]
        }
    }
    pub(super) async fn forward_initial_items_to_search(
        &self,
        items: impl IntoIterator<Item = Arc<SdkTimelineItem>>,
    ) {
        use eyeball_im::VectorDiff;

        for item in items {
            self.forward_diff_to_search(&VectorDiff::PushBack { value: item })
                .await;
        }
    }
    fn attachment_document_from_timeline_media(
        media: &TimelineMedia,
        event_item: &EventTimelineItem,
        message: &matrix_sdk_ui::timeline::Message,
    ) -> Option<AttachmentDocument> {
        let kind = match media.kind {
            koushi_protocol::event::TimelineMediaKind::Image => AttachmentKind::Image,
            koushi_protocol::event::TimelineMediaKind::Video => AttachmentKind::Video,
            koushi_protocol::event::TimelineMediaKind::Audio => AttachmentKind::Audio,
            koushi_protocol::event::TimelineMediaKind::File => AttachmentKind::File,
        };

        let msgtype = match media.kind {
            koushi_protocol::event::TimelineMediaKind::Image => "m.image",
            koushi_protocol::event::TimelineMediaKind::Video => "m.video",
            koushi_protocol::event::TimelineMediaKind::Audio => "m.audio",
            koushi_protocol::event::TimelineMediaKind::File => "m.file",
        };

        let thread_root = event_item.content().thread_root().map(|id| id.to_string());

        Some(AttachmentDocument {
            kind,
            msgtype: msgtype.to_owned(),
            mimetype: media.mimetype.clone(),
            size: media.size,
            source_mxc: media.source.mxc_uri.clone(),
            thumbnail_mxc: media
                .thumbnail
                .as_ref()
                .map(|thumbnail| thumbnail.source.mxc_uri.clone()),
            filename: SensitiveString::new(media.filename.clone()),
            thread_root,
            encrypted: media.source.encrypted,
            encryption_version: media.source.encryption_version.clone(),
            width: media.width.and_then(|w| u32::try_from(w).ok()),
            height: media.height.and_then(|h| u32::try_from(h).ok()),
            is_edited: message.is_edited(),
        })
    }
    fn attachment_document_from_sticker(
        sticker: &matrix_sdk_ui::timeline::Sticker,
    ) -> AttachmentDocument {
        use matrix_sdk::ruma::events::sticker::{StickerEventContent, StickerMediaSource};

        let content: &StickerEventContent = sticker.content();
        let info = &content.info;

        let source = match &content.source {
            StickerMediaSource::Plain(uri) => TimelineMediaSource {
                mxc_uri: uri.to_string(),
                encrypted: false,
                encryption_version: None,
            },
            StickerMediaSource::Encrypted(file) => TimelineMediaSource {
                mxc_uri: file.url.to_string(),
                encrypted: true,
                encryption_version: Some(file.info.version().to_owned()),
            },
            _ => TimelineMediaSource {
                mxc_uri: String::new(),
                encrypted: false,
                encryption_version: None,
            },
        };

        let thumbnail_mxc = info
            .thumbnail_source
            .as_ref()
            .map(|thumbnail_source| timeline_media_source_from_sdk(thumbnail_source).mxc_uri);

        AttachmentDocument {
            kind: AttachmentKind::Sticker,
            msgtype: "m.sticker".to_owned(),
            mimetype: info.mimetype.clone(),
            size: uint_to_u64(info.size.as_ref()),
            source_mxc: source.mxc_uri,
            thumbnail_mxc,
            filename: SensitiveString::new(content.body.clone()),
            thread_root: None,
            encrypted: source.encrypted,
            encryption_version: source.encryption_version,
            width: None,
            height: None,
            is_edited: false,
        }
    }
    /// Resolve the timeline item identity for `event_id`, falling back to the
    /// local-echo transaction identity for events this actor sent whose
    /// remote echo has not arrived.
    fn item_ids_for_event(&self, event_id: &str) -> Vec<TimelineEventItemId> {
        let mut ids = Vec::with_capacity(2);
        if let Ok(parsed) = matrix_sdk::ruma::EventId::parse(event_id) {
            ids.push(TimelineEventItemId::EventId(parsed));
        }
        if let Some(txn) = self.sent_event_txns.get(event_id) {
            ids.push(TimelineEventItemId::TransactionId(txn.clone()));
        }
        ids
    }
    pub(super) fn timeline_contains_event_id(&self, event_id: &str) -> bool {
        self.navigation_items.iter().any(
            |item| matches!(&item.id, TimelineItemId::Event { event_id: id } if id == event_id),
        )
    }
    pub(super) async fn project_message_source_for_event(
        &self,
        event_id: &str,
    ) -> Option<TimelineMessageSource> {
        let parsed_event_id = matrix_sdk::ruma::EventId::parse(event_id).ok()?;
        let items = self.timeline.items().await;
        for item in items.iter().rev() {
            let TimelineItemKind::Event(event_item) = item.kind() else {
                continue;
            };
            if !event_item
                .event_id()
                .map(|candidate| candidate.as_str() == parsed_event_id.as_str())
                .unwrap_or(false)
            {
                continue;
            }

            let projected = sdk_item_to_timeline_item(&self.key, item, self.own_user_id.as_deref());
            let mut source = message_source_for_timeline_item(&projected)?;
            let encryption_info = event_item.encryption_info();
            let session_id = encryption_info
                .and_then(|info| info.session_id())
                .filter(|session_id| !session_id.is_empty())
                .map(str::to_owned);
            let sent_by_current_device = encryption_info
                .and_then(|info| info.sender_device.as_deref())
                .is_some_and(|device_id| device_id.as_str() == self.session.info.device_id);
            source.megolm_session_fingerprint =
                session_id.as_deref().map(megolm_session_fingerprint);
            if let Some(session_id) = session_id.as_deref() {
                let reason = if sent_by_current_device {
                    koushi_sdk::room_key_rotation_reason(
                        &self.session,
                        self.key.room_id(),
                        session_id,
                    )
                    .await
                } else {
                    None
                };
                source.megolm_session_rotation_reason =
                    project_local_megolm_rotation_reason(sent_by_current_device, reason);
            }
            source.original_json = original_json_for_event_item(event_item);
            source.megolm_message_index = source
                .original_json
                .as_ref()
                .and_then(megolm_message_index_from_original_json);
            return Some(source);
        }
        None
    }
    async fn reaction_target_state(
        &self,
        request_id: RequestId,
        trace_kind: &'static str,
        event_id: &str,
        reaction_key: &str,
    ) -> Option<ReactionTargetState> {
        let started = Instant::now();
        let parsed_event_id = match matrix_sdk::ruma::EventId::parse(event_id) {
            Ok(event_id) => event_id,
            Err(_) => {
                trace_timeline_actor_scan(
                    "target_scan",
                    trace_kind,
                    request_id,
                    &self.key,
                    0,
                    started.elapsed().as_millis(),
                    false,
                );
                return None;
            }
        };
        let items = self.timeline.items().await;
        let item_count = items.len();
        for item in items.iter().rev() {
            let TimelineItemKind::Event(event_item) = item.kind() else {
                continue;
            };
            if !event_item
                .event_id()
                .map(|candidate| candidate.as_str() == parsed_event_id.as_str())
                .unwrap_or(false)
            {
                continue;
            }

            let projected = sdk_item_to_timeline_item(&self.key, item, self.own_user_id.as_deref());
            let my_reaction_event_id = projected
                .reactions
                .iter()
                .find(|reaction| reaction.key == reaction_key)
                .and_then(|reaction| reaction.my_reaction_event_id.clone());
            trace_timeline_actor_scan(
                "target_scan",
                trace_kind,
                request_id,
                &self.key,
                item_count,
                started.elapsed().as_millis(),
                true,
            );
            return Some(ReactionTargetState {
                item_id: TimelineEventItemId::EventId(parsed_event_id),
                can_react: projected.can_react,
                my_reaction_event_id,
            });
        }
        trace_timeline_actor_scan(
            "target_scan",
            trace_kind,
            request_id,
            &self.key,
            item_count,
            started.elapsed().as_millis(),
            false,
        );
        None
    }
}

pub(super) fn timeline_room_id(key: &TimelineKey) -> Option<String> {
    match &key.kind {
        TimelineKind::Room { room_id }
        | TimelineKind::Thread { room_id, .. }
        | TimelineKind::Focused { room_id, .. } => Some(room_id.clone()),
    }
}

pub(super) fn apply_ignored_sender_suppression(
    item: &mut TimelineItem,
    ignored_user_ids: &std::collections::BTreeSet<String>,
) {
    if !matches!(&item.id, TimelineItemId::Event { .. }) {
        return;
    }
    let sender_ignored = item
        .sender
        .as_deref()
        .is_some_and(|sender| ignored_user_ids.contains(sender));
    // Recompute from projected content, not the previous ignored result. This
    // keeps ignore→unignore reversible while retaining the normal bodyless
    // suppression baseline.
    item.is_hidden = (!has_user_visible_content(item) && !item.is_redacted) || sender_ignored;
}

pub(super) fn apply_ignored_sender_suppression_to_diff(
    diff: &mut TimelineDiff,
    ignored_user_ids: &std::collections::BTreeSet<String>,
) {
    match diff {
        TimelineDiff::PushFront { item }
        | TimelineDiff::PushBack { item }
        | TimelineDiff::Insert { item, .. }
        | TimelineDiff::Set { item, .. } => {
            apply_ignored_sender_suppression(item, ignored_user_ids);
        }
        TimelineDiff::Reset { items } => {
            for item in items {
                apply_ignored_sender_suppression(item, ignored_user_ids);
            }
        }
        TimelineDiff::Remove { .. } | TimelineDiff::Truncate { .. } | TimelineDiff::Clear => {}
    }
}

pub(super) async fn apply_link_previews_to_item(
    item: &mut TimelineItem,
    room_id: &str,
    context: &LinkPreviewContext,
    session: &Arc<MatrixClientSession>,
) {
    let TimelineItemId::Event { event_id } = &item.id else {
        return;
    };

    let is_encrypted = match matrix_sdk::ruma::RoomId::parse(room_id) {
        Ok(room_id) => match session.client().get_room(&room_id) {
            Some(room) => match room.latest_encryption_state().await {
                Ok(state) => state.is_encrypted(),
                Err(_) => false,
            },
            None => false,
        },
        Err(_) => false,
    };

    item.link_previews = crate::link_preview::link_previews_for_message(
        item.body.as_deref(),
        item.formatted.as_ref(),
        event_id,
        is_encrypted,
        context,
    );
}

fn reset_loading_link_previews_to_pending(item: &mut TimelineItem) -> bool {
    let Some(previews) = item.link_previews.as_mut() else {
        return false;
    };
    let mut changed = false;
    for preview in previews {
        if preview.state == LinkPreviewState::Loading {
            preview.state = LinkPreviewState::Pending;
            changed = true;
        }
    }
    changed
}

pub(super) fn eligible_activity_preview(item: &TimelineItem) -> Option<String> {
    let source = item
        .formatted
        .as_ref()
        .map(|formatted| formatted.plain_text.as_str())
        .or(item.body.as_deref())
        .or_else(|| item.media.as_ref().map(|media| media.filename.as_str()))?;
    collapsed_preview(source, REPLY_QUOTE_PREVIEW_MAX_CHARS)
}

pub(super) fn is_attention_eligible_event(item: &TimelineItem) -> bool {
    matches!(item.id, TimelineItemId::Event { .. })
        && !item.is_redacted
        && !item.is_hidden
        && eligible_activity_preview(item).is_some()
}

pub(super) fn is_unread_navigation_item(item: &TimelineItem, own_user_id: Option<&str>) -> bool {
    if !is_attention_eligible_event(item) {
        return false;
    }
    if own_user_id.is_some_and(|own| item.sender.as_deref() == Some(own)) {
        return false;
    }
    true
}

pub(super) fn has_user_visible_content(item: &TimelineItem) -> bool {
    timeline_content_is_renderable(
        item.body.as_deref(),
        item.media.as_ref(),
        item.formatted.as_ref(),
    )
}

pub(super) fn timeline_content_is_renderable(
    body: Option<&str>,
    media: Option<&TimelineMedia>,
    formatted: Option<&koushi_protocol::event::TimelineFormattedBody>,
) -> bool {
    body.is_some_and(|body| !body.trim().is_empty())
        || media.is_some()
        || formatted.is_some_and(timeline_formatted_body_is_renderable)
}

pub(super) fn timeline_formatted_body_is_renderable(
    formatted: &koushi_protocol::event::TimelineFormattedBody,
) -> bool {
    !formatted.plain_text.trim().is_empty()
        || formatted
            .code_blocks
            .iter()
            .any(|block| !block.body.trim().is_empty())
}

pub(super) fn timeline_sender_label_from_profile(
    profile: &TimelineDetails<Profile>,
) -> Option<String> {
    match profile {
        TimelineDetails::Ready(profile) => profile.display_name.clone(),
        TimelineDetails::Unavailable | TimelineDetails::Pending | TimelineDetails::Error(_) => None,
    }
}

pub(super) fn timeline_sender_avatar_from_profile(
    profile: &TimelineDetails<Profile>,
) -> Option<AvatarImage> {
    let TimelineDetails::Ready(profile) = profile else {
        return None;
    };
    let avatar_url = profile.avatar_url.as_ref()?;
    Some(AvatarImage {
        mxc_uri: avatar_url.to_string(),
        thumbnail: AvatarThumbnailState::NotRequested,
    })
}

fn original_json_for_event_item(event_item: &EventTimelineItem) -> Option<serde_json::Value> {
    event_item
        .original_json()
        .and_then(|raw| serde_json::from_str(raw.json().get()).ok())
}

fn megolm_message_index_from_original_json(original_json: &serde_json::Value) -> Option<u32> {
    if original_json.get("type")?.as_str()? != "m.room.encrypted" {
        return None;
    }
    let content = original_json.get("content")?;
    if content.get("algorithm")?.as_str()? != "m.megolm.v1.aes-sha2" {
        return None;
    }
    let ciphertext = content.get("ciphertext")?.as_str()?;
    vodozemac::megolm::MegolmMessage::from_base64(ciphertext)
        .ok()
        .map(|message| message.message_index())
}

fn project_local_megolm_rotation_reason(
    sent_by_current_device: bool,
    reason: Option<koushi_sdk::MatrixRoomKeyRotationReason>,
) -> Option<TimelineMegolmSessionReason> {
    use koushi_sdk::MatrixRoomKeyRotationReason as Reason;
    if !sent_by_current_device {
        return None;
    }
    Some(match reason {
        Some(Reason::Initial) => TimelineMegolmSessionReason::Initial,
        Some(Reason::ExpiredTime) => TimelineMegolmSessionReason::ExpiredTime,
        Some(Reason::ExpiredMessageCount) => TimelineMegolmSessionReason::ExpiredMessageCount,
        Some(Reason::MembershipOrDeviceChange) => {
            TimelineMegolmSessionReason::MembershipOrDeviceChange
        }
        Some(Reason::EncryptionSettingsChanged) => {
            TimelineMegolmSessionReason::EncryptionSettingsChanged
        }
        Some(Reason::ExplicitDiscard) => TimelineMegolmSessionReason::ExplicitDiscard,
        Some(Reason::FullMemberListReload) => TimelineMegolmSessionReason::FullMemberListReload,
        Some(Reason::RoomSubscription) => TimelineMegolmSessionReason::RoomSubscription,
        Some(Reason::LimitedSyncResponse) => TimelineMegolmSessionReason::LimitedSyncResponse,
        Some(Reason::KeyShareFailure) => TimelineMegolmSessionReason::KeyShareFailure,
        Some(Reason::StoreMissing) => TimelineMegolmSessionReason::StoreMissing,
        Some(Reason::Invalidated) => TimelineMegolmSessionReason::Invalidated,
        Some(Reason::Unknown) => TimelineMegolmSessionReason::Unknown,
        None => TimelineMegolmSessionReason::NotRetained,
    })
}

pub(super) fn megolm_session_fingerprint(session_id: &str) -> String {
    // Matrix Megolm session IDs are random base64 strings. A 12-character
    // prefix is compact while providing enough entropy to distinguish session
    // rotation without exposing the complete identifier in the UI.
    session_id.chars().take(12).collect()
}

fn effective_message_content(raw: &serde_json::Value) -> Option<&serde_json::Value> {
    let content = raw.get("content")?;
    Some(
        content
            .get("m.relates_to")
            .and_then(|relation| {
                (relation.get("rel_type")?.as_str() == Some("m.replace"))
                    .then(|| relation.get("m.new_content"))
            })
            .flatten()
            .unwrap_or(content),
    )
}

pub(super) fn mentioned_user_ids_from_event_json(raw: &serde_json::Value) -> Vec<String> {
    mention_intent_from_event_json(raw)
        .map(|intent| {
            intent
                .targets
                .into_iter()
                .filter_map(|target| match target {
                    MentionTarget::User { user_id, .. } => Some(user_id),
                    MentionTarget::Room { .. } | MentionTarget::RoomMention { .. } => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn mention_intent_from_event_json(raw: &serde_json::Value) -> Option<MentionIntent> {
    let effective_content = effective_message_content(raw)?;
    let mentions = effective_content.get("m.mentions")?;
    let mut targets = mentions
        .get("user_ids")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .filter(|user_id| !user_id.trim().is_empty())
        .map(|user_id| MentionTarget::User {
            user_id: user_id.to_owned(),
            // The renderer replaces this safe fallback with the current room
            // candidate's display label before opening the editor.
            display_label: user_id.trim_start_matches('@').to_owned(),
        })
        .collect::<Vec<_>>();
    if mentions
        .get("room")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        targets.push(MentionTarget::RoomMention {
            display_label: "room".to_owned(),
        });
    }
    (!targets.is_empty()).then_some(MentionIntent { targets })
}

fn composer_document_from_event_json(raw: &serde_json::Value) -> Option<ComposerDocument> {
    let content = effective_message_content(raw)?;
    let body = content.get("body")?.as_str()?;
    let mentions = mention_intent_from_event_json(raw).unwrap_or_default();
    let formatted_body = content.get("formatted_body")?.as_str()?;
    let html = Html::parse(formatted_body);
    let mut parsed = Vec::new();
    collect_composer_inlines_from_nodes(html.children(), &mentions, &mut parsed);
    let mut inlines = Vec::new();
    let mut remaining = body;
    for inline in parsed {
        let ComposerInline::Mention {
            target,
            display_label,
        } = inline
        else {
            continue;
        };
        let needle = format!("@{display_label}");
        let offset = remaining.find(&needle)?;
        if offset > 0 {
            inlines.push(ComposerInline::Text {
                text: remaining[..offset].to_owned(),
            });
        }
        inlines.push(ComposerInline::Mention {
            target,
            display_label,
        });
        remaining = &remaining[offset + needle.len()..];
    }
    if !remaining.is_empty() {
        inlines.push(ComposerInline::Text {
            text: remaining.to_owned(),
        });
    }
    let document = ComposerDocument::new(inlines);
    (!document.mention_intent().targets.is_empty()).then_some(document)
}

fn collect_composer_inlines_from_nodes(
    nodes: impl Iterator<Item = matrix_sdk::ruma::html::NodeRef>,
    mentions: &MentionIntent,
    out: &mut Vec<ComposerInline>,
) {
    for node in nodes {
        if let Some(text) = node.as_text() {
            out.push(ComposerInline::Text {
                text: text.borrow().to_string(),
            });
            continue;
        }
        let Some(element) = node.as_element() else {
            continue;
        };
        let attrs = element.attrs.borrow();
        let href = attrs
            .iter()
            .find_map(|attr| (attr.name.local.as_ref() == "href").then(|| attr.value.to_string()));
        let room_mention = attrs
            .iter()
            .any(|attr| attr.name.local.as_ref() == "data-mx-mention");
        drop(attrs);

        let mut label = String::new();
        collect_plain_text_from_nodes(node.children(), &mut label);
        let display_label = label.strip_prefix('@').unwrap_or(&label).to_owned();
        if let Some(target) = href
            .as_deref()
            .and_then(matrix_to_mention_target)
            .and_then(|target| allowed_editable_mention_target(target, mentions, &display_label))
        {
            out.push(ComposerInline::Mention {
                target,
                display_label,
            });
        } else if room_mention && mentions.mentions_room() {
            out.push(ComposerInline::Mention {
                target: MentionTarget::RoomMention {
                    display_label: display_label.clone(),
                },
                display_label,
            });
        } else {
            collect_composer_inlines_from_nodes(node.children(), mentions, out);
        }
    }
}

fn matrix_to_mention_target(href: &str) -> Option<MentionTarget> {
    let url = url::Url::parse(href).ok()?;
    if url.scheme() != "https" || url.host_str()? != "matrix.to" {
        return None;
    }
    let encoded = url.fragment()?.strip_prefix('/')?.split('?').next()?;
    let identifier = percent_decode_matrix_identifier(encoded)?;
    if identifier.starts_with('@') {
        Some(MentionTarget::User {
            user_id: identifier,
            display_label: String::new(),
        })
    } else if identifier.starts_with('!') || identifier.starts_with('#') {
        Some(MentionTarget::Room {
            room_id: identifier,
            display_label: String::new(),
        })
    } else {
        None
    }
}

fn allowed_editable_mention_target(
    target: MentionTarget,
    mentions: &MentionIntent,
    display_label: &str,
) -> Option<MentionTarget> {
    match target {
        MentionTarget::User { user_id, .. } => mentions
            .targets
            .iter()
            .any(|target| matches!(target, MentionTarget::User { user_id: existing, .. } if existing == &user_id))
            .then(|| MentionTarget::User {
                user_id,
                display_label: display_label.to_owned(),
            }),
        MentionTarget::Room { room_id, .. } => Some(MentionTarget::Room {
            room_id,
            display_label: display_label.to_owned(),
        }),
        MentionTarget::RoomMention { .. } => None,
    }
}

fn percent_decode_matrix_identifier(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = hex_value(*bytes.get(index + 1)?)?;
            let low = hex_value(*bytes.get(index + 2)?)?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn timeline_item_should_be_hidden(has_renderable_content: bool, is_redacted: bool) -> bool {
    !has_renderable_content && !is_redacted
}

pub(super) fn timeline_item_should_be_hidden_for_key(
    _key: &TimelineKey,
    has_renderable_content: bool,
    is_redacted: bool,
    _thread_root: Option<&str>,
) -> bool {
    timeline_item_should_be_hidden(has_renderable_content, is_redacted)
}

/// Koushi threads are linear, so a thread-keyed reply command is always an
/// ordinary thread message: the relation is threaded and the target event is
/// never promoted to a rich reply. The product UI offers no thread-pane reply
/// action, and this projection keeps a thread rich reply unreachable even if a
/// caller passes a non-root target.
pub(super) fn reply_enforce_thread_for_key(key: &TimelineKey) -> EnforceThread {
    match key.kind {
        TimelineKind::Thread { .. } => EnforceThread::Threaded(ReplyWithinThread::No),
        TimelineKind::Room { .. } | TimelineKind::Focused { .. } => EnforceThread::MaybeThreaded,
    }
}

pub(super) fn attachment_reply_for_key(key: &TimelineKey) -> Option<Reply> {
    let TimelineKind::Thread { root_event_id, .. } = &key.kind else {
        return None;
    };
    Some(Reply {
        event_id: matrix_sdk::ruma::EventId::parse(root_event_id).ok()?,
        enforce_thread: EnforceThread::Threaded(ReplyWithinThread::No),
        add_mentions: AddMentions::No,
    })
}

pub(super) fn thread_root_from_original_json(original_json: &serde_json::Value) -> Option<String> {
    let relates_to = original_json.get("content")?.get("m.relates_to")?;
    if relates_to.get("rel_type")?.as_str()? != "m.thread" {
        return None;
    }
    let event_id = relates_to.get("event_id")?.as_str()?.trim();
    (!event_id.is_empty()).then(|| event_id.to_owned())
}

pub(super) fn item_index_for_event_id(items: &[TimelineItem], event_id: &str) -> Option<usize> {
    items
        .iter()
        .position(|item| timeline_item_event_id(item) == Some(event_id))
}

fn visible_missing_reply_detail_event_ids(
    items: &[TimelineItem],
    observation: &TimelineViewportObservation,
    already_requested_event_ids: &HashSet<String>,
) -> Vec<String> {
    let Some(first_visible_event_id) = observation.first_visible_event_id.as_deref() else {
        return Vec::new();
    };
    let Some(last_visible_event_id) = observation.last_visible_event_id.as_deref() else {
        return Vec::new();
    };
    let Some(first_visible_index) = item_index_for_event_id(items, first_visible_event_id) else {
        return Vec::new();
    };
    let Some(last_visible_index) = item_index_for_event_id(items, last_visible_event_id) else {
        return Vec::new();
    };

    let start = first_visible_index.min(last_visible_index);
    let end = first_visible_index.max(last_visible_index);
    items[start..=end]
        .iter()
        .filter_map(|item| {
            let event_id = timeline_item_event_id(item)?;
            if already_requested_event_ids.contains(event_id) {
                return None;
            }
            let quote = item.reply_quote.as_ref()?;
            (quote.state == ReplyQuoteState::Missing).then(|| event_id.to_owned())
        })
        .collect()
}

pub(super) fn timeline_item_event_id(item: &TimelineItem) -> Option<&str> {
    match &item.id {
        TimelineItemId::Event { event_id } => Some(event_id.as_str()),
        TimelineItemId::Transaction { .. } | TimelineItemId::Synthetic { .. } => None,
    }
}

pub(super) fn live_event_receipts_from_sdk_items<'a>(
    items: impl IntoIterator<Item = &'a Arc<SdkTimelineItem>>,
) -> Vec<LiveEventReceipts> {
    items
        .into_iter()
        .filter_map(live_event_receipts_from_sdk_item)
        .collect()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ReceiptObservationTarget {
    Live,
    Authoritative { scoped_event_ids: Vec<String> },
}

fn profile_actions(profiles: Vec<MatrixUserProfile>) -> Vec<UserProfile> {
    profiles
        .into_iter()
        .map(|profile| {
            let display_label = profile
                .display_name
                .as_deref()
                .map(str::trim)
                .filter(|label| !label.is_empty())
                .unwrap_or("Unknown user")
                .to_owned();
            UserProfile {
                user_id: profile.user_id,
                display_name: profile.display_name,
                display_label: display_label.clone(),
                original_display_label: display_label,
                mention_search_terms: Vec::new(),
                avatar: profile.avatar_mxc_uri.map(|mxc_uri| AvatarImage {
                    mxc_uri,
                    thumbnail: AvatarThumbnailState::NotRequested,
                }),
            }
        })
        .collect()
}

fn profile_actions_to_actions(
    room_id: &str,
    profiles: Vec<MatrixUserProfile>,
) -> (Vec<AppAction>, bool) {
    let profile_actions = profile_actions(profiles);
    let has_profiles = !profile_actions.is_empty();
    let mut actions = Vec::with_capacity(usize::from(has_profiles) * 2);
    if has_profiles {
        actions.push(AppAction::LiveRoomProfilesObserved {
            room_id: room_id.to_owned(),
            profiles: profile_actions.clone(),
        });
        actions.push(AppAction::UserProfilesUpdated {
            profiles: profile_actions,
        });
    }
    (actions, has_profiles)
}

fn build_receipt_observation_actions(
    room_id: &str,
    receipts_by_event: Vec<LiveEventReceipts>,
    profiles: Vec<MatrixUserProfile>,
    scoped_event_ids: Vec<String>,
) -> Vec<AppAction> {
    let (mut actions, has_profiles) = profile_actions_to_actions(room_id, profiles);
    actions.reserve(usize::from(has_profiles) + 1);
    actions.push(AppAction::LiveRoomReceiptsWindowReconciled {
        room_id: room_id.to_owned(),
        scoped_event_ids,
        receipts_by_event,
    });
    actions
}

fn compact_live_receipt_summaries(
    receipts_by_event: Vec<LiveEventReceipts>,
    own_user_id: Option<&str>,
) -> Vec<LiveEventReceiptSummaryUpdate> {
    receipts_by_event
        .into_iter()
        .map(|entry| {
            let mut by_user = BTreeMap::new();
            for receipt in entry.receipts {
                if own_user_id.is_some_and(|own| own == receipt.user_id) {
                    continue;
                }
                by_user
                    .entry(receipt.user_id.clone())
                    .and_modify(|existing: &mut LiveReadReceipt| {
                        if receipt.timestamp_ms.unwrap_or_default()
                            >= existing.timestamp_ms.unwrap_or_default()
                        {
                            *existing = receipt.clone();
                        }
                    })
                    .or_insert(receipt);
            }
            let mut readers = by_user.into_values().collect::<Vec<_>>();
            readers.sort_by(|left, right| {
                right
                    .timestamp_ms
                    .unwrap_or_default()
                    .cmp(&left.timestamp_ms.unwrap_or_default())
                    .then_with(|| left.user_id.cmp(&right.user_id))
            });
            let total_count = readers.len() as u64;
            readers.truncate(3);
            LiveEventReceiptSummaryUpdate {
                event_id: entry.event_id,
                readers,
                total_count,
            }
        })
        .collect()
}

fn build_live_receipt_summary_actions(
    room_id: &str,
    receipts_by_event: Vec<LiveEventReceiptSummaryUpdate>,
    profiles: Vec<MatrixUserProfile>,
) -> Vec<AppAction> {
    let (mut actions, has_profiles) = profile_actions_to_actions(room_id, profiles);
    actions.reserve(usize::from(has_profiles) + 1);
    actions.push(AppAction::LiveRoomReceiptSummariesUpdated {
        room_id: room_id.to_owned(),
        receipts_by_event,
    });
    actions
}

#[cfg(test)]
pub(super) fn build_live_receipt_observation_actions(
    room_id: &str,
    receipts_by_event: Vec<LiveEventReceipts>,
    profiles: Vec<MatrixUserProfile>,
) -> Vec<AppAction> {
    build_live_receipt_summary_actions(
        room_id,
        compact_live_receipt_summaries(receipts_by_event, None),
        profiles,
    )
}

#[cfg(test)]
pub(super) async fn live_receipt_observation_actions_from_sdk_receipts(
    session: &MatrixClientSession,
    room_id: &str,
    receipts_by_event: Vec<LiveEventReceipts>,
) -> Vec<AppAction> {
    receipt_observation_actions_from_sdk_receipts(
        session,
        room_id,
        receipts_by_event,
        ReceiptObservationTarget::Live,
    )
    .await
}

async fn receipt_observation_actions_from_sdk_receipts(
    session: &MatrixClientSession,
    room_id: &str,
    receipts_by_event: Vec<LiveEventReceipts>,
    target: ReceiptObservationTarget,
) -> Vec<AppAction> {
    if matches!(&target, ReceiptObservationTarget::Live) {
        let own_user_id = session.client().user_id().map(|user_id| user_id.to_owned());
        let summaries = compact_live_receipt_summaries(
            receipts_by_event,
            own_user_id.as_ref().map(|user_id| user_id.as_str()),
        );
        let lookup_user_ids = summaries
            .iter()
            .flat_map(|entry| entry.readers.iter())
            .map(|receipt| receipt.user_id.clone())
            .collect::<Vec<_>>();
        let receipt_count = summaries
            .iter()
            .map(|entry| entry.total_count as usize)
            .sum();
        let profiles =
            receipt_profiles_for_users(session, room_id, &lookup_user_ids, receipt_count).await;
        return build_live_receipt_summary_actions(room_id, summaries, profiles);
    }

    let user_ids = receipts_by_event
        .iter()
        .flat_map(|entry| entry.receipts.iter())
        .map(|receipt| receipt.user_id.clone())
        .collect::<Vec<_>>();
    let receipt_count = receipts_by_event
        .iter()
        .map(|entry| entry.receipts.len())
        .sum();
    let profiles = receipt_profiles_for_users(session, room_id, &user_ids, receipt_count).await;
    let ReceiptObservationTarget::Authoritative { scoped_event_ids } = target else {
        unreachable!("live receipt observation returns through the bounded branch");
    };
    build_receipt_observation_actions(room_id, receipts_by_event, profiles, scoped_event_ids)
}

pub(super) async fn prepare_receipt_window_profiles(
    session: &MatrixClientSession,
    room_id: &str,
    window: &mut super::receipt_endpoints::RawReceiptWindow,
    reply: &mut tokio::sync::oneshot::Sender<
        Result<super::receipt_endpoints::RawReceiptWindow, crate::view_scope_lifecycle::ScopeError>,
    >,
) -> bool {
    let users: Vec<_> = window
        .receipts
        .iter()
        .map(|receipt| receipt.user_id.clone())
        .collect();
    window.profiles = tokio::select! {
        biased;
        _ = reply.closed() => return false,
        profiles = receipt_profiles_for_users(session, room_id, &users, window.total_count as usize) => profiles,
    };
    true
}

async fn receipt_profiles_for_users(
    session: &MatrixClientSession,
    room_id: &str,
    user_ids: &[String],
    receipt_count: usize,
) -> Vec<MatrixUserProfile> {
    let requested_user_count = user_ids.iter().collect::<BTreeSet<_>>().len();
    let (profiles, lookup_outcome) = match session
        .room_member_profiles_no_sync(room_id, user_ids)
        .await
    {
        Ok(profiles) if profiles.is_empty() => (profiles, "miss"),
        Ok(profiles) => (profiles, "observed"),
        Err(_) => (Vec::new(), "failed"),
    };
    record_live_receipt_profile_lookup(
        receipt_count,
        requested_user_count,
        profiles.len(),
        lookup_outcome,
    );
    profiles
}

pub(super) async fn emit_receipt_observation_actions(
    session: &MatrixClientSession,
    action_tx: &mpsc::Sender<Vec<AppAction>>,
    timeline_actor_generations: &Arc<TimelineActorGenerationGate>,
    key: &TimelineKey,
    actor_generation: u64,
    room_id: &str,
    receipts_by_event: Vec<LiveEventReceipts>,
    target: ReceiptObservationTarget,
) -> bool {
    let actions =
        receipt_observation_actions_from_sdk_receipts(session, room_id, receipts_by_event, target)
            .await;
    send_generation_fenced(
        action_tx,
        timeline_actor_generations,
        key,
        actor_generation,
        actions,
    )
    .await
}

pub(super) async fn emit_live_receipt_observation_actions(
    session: &MatrixClientSession,
    action_tx: &mpsc::Sender<Vec<AppAction>>,
    timeline_actor_generations: &Arc<TimelineActorGenerationGate>,
    key: &TimelineKey,
    actor_generation: u64,
    room_id: &str,
    receipts_by_event: Vec<LiveEventReceipts>,
) -> bool {
    emit_receipt_observation_actions(
        session,
        action_tx,
        timeline_actor_generations,
        key,
        actor_generation,
        room_id,
        receipts_by_event,
        ReceiptObservationTarget::Live,
    )
    .await
}

fn record_live_receipt_profile_lookup(
    receipt_count: usize,
    requested_user_count: usize,
    observed_profile_count: usize,
    lookup_outcome: &'static str,
) {
    record(
        DiagnosticEvent::new(
            DiagnosticLevel::Debug,
            "core.read_receipt_profile",
            "local_lookup",
        )
        .field(DiagnosticField::token("lookup_outcome", lookup_outcome))
        .field(DiagnosticField::count(
            "receipt_count",
            receipt_count as u64,
        ))
        .field(DiagnosticField::count(
            "requested_user_count",
            requested_user_count as u64,
        ))
        .field(DiagnosticField::count(
            "observed_profile_count",
            observed_profile_count as u64,
        ))
        .field(DiagnosticField::boolean("network_lookup_attempted", false)),
    );
}

pub(super) fn live_event_receipts_from_endpoint_changes(
    changes: std::collections::BTreeMap<String, super::receipt_endpoints::ReceiptEndpointChange>,
    endpoints: &super::receipt_endpoints::ReceiptEndpointMirror,
) -> Vec<LiveEventReceipts> {
    changes
        .into_iter()
        .filter_map(|(event_id, change)| {
            let index = endpoints.readers(&event_id)?;
            if change.before.is_none() && index.total(None) == 0 {
                return None;
            }
            Some(live_event_receipts_from_readers(
                event_id,
                index.window(0, usize::MAX, None),
            ))
        })
        .collect()
}

fn live_event_receipts_from_sdk_item(item: &Arc<SdkTimelineItem>) -> Option<LiveEventReceipts> {
    use matrix_sdk_ui::timeline::TimelineItemKind;

    let event_item = match item.kind() {
        TimelineItemKind::Event(event_item) => event_item,
        TimelineItemKind::Virtual(_) => return None,
    };
    let event_id = event_item.event_id()?.to_string();
    let snapshot = event_item.read_receipt_snapshot();
    if snapshot.is_empty() {
        return None;
    }
    Some(live_event_receipts_from_readers(event_id, snapshot.iter()))
}

fn live_event_receipts_from_readers<'a>(
    event_id: String,
    readers: impl Iterator<
        Item = (
            &'a matrix_sdk::ruma::OwnedUserId,
            &'a matrix_sdk::ruma::events::receipt::Receipt,
        ),
    >,
) -> LiveEventReceipts {
    let receipts = readers
        .map(|(user_id, receipt)| LiveReadReceipt {
            user_id: user_id.to_string(),
            display_name: None,
            original_display_label: String::new(),
            avatar: None,
            timestamp_ms: receipt.ts.map(|timestamp| timestamp.0.into()),
        })
        .collect::<Vec<_>>();

    LiveEventReceipts { event_id, receipts }
}

/// Convert a single SDK `TimelineItem` to our `TimelineItem` DTO.
pub fn sdk_item_to_timeline_item(
    key: &TimelineKey,
    item: &Arc<SdkTimelineItem>,
    own_user_id: Option<&matrix_sdk::ruma::UserId>,
) -> TimelineItem {
    sdk_item_to_timeline_item_with_send_states(
        key,
        item,
        own_user_id,
        &HashMap::new(),
        None,
        None,
        None,
    )
}

/// Build the closed room-key request presentation state for an event
/// (issue #460). Only closed tokens cross the wire.
fn request_state_for_item(
    key_request_states: Option<&std::collections::BTreeMap<String, KeyRequestUiState>>,
    withheld_codes: Option<&std::collections::BTreeMap<(String, String), &'static str>>,
    key: &TimelineKey,
    event_item: &matrix_sdk_ui::timeline::EventTimelineItem,
) -> Option<RoomKeyRequestStateDto> {
    let event_id = event_item.event_id()?.to_string();
    let state = key_request_states?.get(&event_id)?;
    let withheld_code = state
        .withheld_code
        .and_then(key_request_withheld_code_token)
        .or_else(|| {
            let session = event_item
                .content()
                .as_unable_to_decrypt()
                .and_then(|utd| {
                    let matrix_sdk_ui::timeline::EncryptedMessage::MegolmV1AesSha2 {
                        session_id,
                        ..
                    } = utd
                    else {
                        return None;
                    };
                    Some(session_id.as_str())
                })?;
            withheld_codes?
                .get(&(key.room_id().to_owned(), session.to_owned()))
                .copied()
                .and_then(key_request_withheld_code_token)
        });
    Some(RoomKeyRequestStateDto {
        stage: key_request_stage_token(state.stage),
        withheld_code,
    })
}

/// Map an internal stage literal to the closed wire token. Internal stages
/// are compile-time literals only; the fallback keeps the wire contract closed
/// even if a future literal is added before the mapping is extended.
pub(super) fn key_request_stage_token(stage: &str) -> RoomKeyRequestStage {
    match stage {
        "sent" => RoomKeyRequestStage::Sent,
        "automatic" => RoomKeyRequestStage::Automatic,
        "still_waiting" => RoomKeyRequestStage::StillWaiting,
        "withheld" => RoomKeyRequestStage::Withheld,
        "decryption_recovered" => RoomKeyRequestStage::DecryptionRecovered,
        "send_failed" => RoomKeyRequestStage::SendFailed,
        _ => RoomKeyRequestStage::StillWaiting,
    }
}

/// Map an internal withheld-code literal to the closed wire token.
pub(super) fn key_request_withheld_code_token(code: &str) -> Option<RoomKeyRequestWithheldCode> {
    match code {
        "blacklisted" => Some(RoomKeyRequestWithheldCode::Blacklisted),
        "unverified" => Some(RoomKeyRequestWithheldCode::Unverified),
        "unauthorised" => Some(RoomKeyRequestWithheldCode::Unauthorised),
        "unavailable" => Some(RoomKeyRequestWithheldCode::Unavailable),
        _ => None,
    }
}

pub(super) fn sdk_item_to_timeline_item_with_send_states(
    key: &TimelineKey,
    item: &Arc<SdkTimelineItem>,
    own_user_id: Option<&matrix_sdk::ruma::UserId>,
    send_statuses: &HashMap<String, TimelineSendState>,
    recovery: Option<&std::collections::BTreeMap<String, super::recovery_model::RecoveryOperation>>,
    key_request_states: Option<&std::collections::BTreeMap<String, KeyRequestUiState>>,
    withheld_codes: Option<&std::collections::BTreeMap<(String, String), &'static str>>,
) -> TimelineItem {
    use matrix_sdk_ui::timeline::{TimelineItemKind, VirtualTimelineItem};

    match &item.kind() {
        TimelineItemKind::Event(event_item) => {
            // Stable identity: remote event_id when known, otherwise transaction_id.
            let transaction_id = event_item.transaction_id().map(|txn_id| txn_id.to_string());
            let id = if let Some(event_id) = event_item.event_id() {
                TimelineItemId::Event {
                    event_id: event_id.to_string(),
                }
            } else if let Some(txn_id) = transaction_id.as_ref() {
                TimelineItemId::Transaction {
                    transaction_id: txn_id.clone(),
                }
            } else {
                // Fallback: use the internal unique_id as a synthetic id.
                TimelineItemId::Synthetic {
                    synthetic_id: item.unique_id().0.clone(),
                }
            };

            let sender = Some(event_item.sender().to_string());
            let sender_profile = event_item.sender_profile();
            let sender_label = timeline_sender_label_from_profile(sender_profile);
            let sender_avatar = timeline_sender_avatar_from_profile(sender_profile);
            let timestamp_ms = Some(event_item.timestamp().0.into());

            let content = event_item.content();
            let message_projection = Some(message_projection_from_timeline_content(content));
            let body = message_projection
                .as_ref()
                .and_then(|projection| projection.body.clone());
            let notice_i18n = message_projection
                .as_ref()
                .and_then(|projection| projection.notice_i18n.clone());
            let actionable_body = message_projection
                .as_ref()
                .filter(|projection| projection.body_is_user_content)
                .and_then(|projection| projection.body.as_deref());
            let message_kind = message_projection
                .as_ref()
                .map(|projection| projection.message_kind)
                .unwrap_or_default();
            let spoiler_spans = message_projection
                .as_ref()
                .map(|projection| projection.spoiler_spans.clone())
                .unwrap_or_default();
            let media = message_projection
                .as_ref()
                .and_then(|projection| projection.media.clone());
            let formatted = message_projection
                .as_ref()
                .and_then(|projection| projection.formatted.clone());
            let has_renderable_content =
                timeline_content_is_renderable(body.as_deref(), media.as_ref(), formatted.as_ref());
            let is_redacted = content.is_redacted();
            let can_hold_reactions = content.reactions().is_some();
            let can_react = timeline_item_can_react(
                event_item.event_id().is_some(),
                can_hold_reactions,
                is_redacted,
                has_renderable_content,
            );
            let can_redact = timeline_item_can_redact(
                event_item.event_id().is_some(),
                own_user_id
                    .map(|user_id| event_item.sender().as_str() == user_id.as_str())
                    .unwrap_or(false),
                is_redacted,
                has_renderable_content,
            );
            let can_edit = timeline_item_can_edit(
                event_item.event_id().is_some(),
                own_user_id
                    .map(|user_id| event_item.sender().as_str() == user_id.as_str())
                    .unwrap_or(false),
                is_redacted,
                actionable_body.is_some(),
            );
            let in_reply_to = content.in_reply_to();
            let in_reply_to_event_id = in_reply_to
                .as_ref()
                .map(|details| details.event_id.to_string());
            let reply_quote = in_reply_to.as_ref().map(reply_quote_from_details);
            let thread_root = event_item
                .content()
                .thread_root()
                .map(|event_id| event_id.to_string())
                .or_else(|| {
                    content
                        .is_unable_to_decrypt()
                        .then(|| original_json_for_event_item(event_item))
                        .flatten()
                        .and_then(|original_json| thread_root_from_original_json(&original_json))
                });
            let thread_summary = event_item
                .content()
                .thread_summary()
                .map(thread_summary_from_sdk);
            let reactions = event_item
                .content()
                .reactions()
                .map(|reactions| reaction_groups_from_sdk(reactions, own_user_id))
                .unwrap_or_default();
            let is_edited = content
                .as_message()
                .map(|message| message.is_edited())
                .unwrap_or(false);
            let send_state = timeline_send_state_from_sdk(event_item.send_state()).or_else(|| {
                transaction_id
                    .as_deref()
                    .and_then(|txn_id| send_statuses.get(txn_id).cloned())
            });
            let mut unable_to_decrypt = unable_to_decrypt_from_content(content);
            if let Some(utd) = unable_to_decrypt.as_mut() {
                utd.can_request_keys = event_item.original_json().is_some();
                if let Some(session_id) = utd.session_id.as_deref()
                    && let Some(op) = recovery.and_then(|map| map.get(session_id))
                {
                    utd.recovery_stage =
                        Some(super::recovery_model::stage_token(op.stage()).to_owned());
                    utd.recovery_guidance = op
                        .guidance()
                        .map(super::recovery_model::guidance_token)
                        .map(ToOwned::to_owned);
                }
            }
            let mut actions = message_actions_for_timeline_item(
                key.room_id(),
                &id,
                actionable_body,
                media.is_some(),
                is_redacted,
            );
            let mut mentioned_user_ids = Vec::new();
            if let Some(raw) = original_json_for_event_item(event_item) {
                actions.editable_document = composer_document_from_event_json(&raw);
                mentioned_user_ids = mentioned_user_ids_from_event_json(&raw);
            }
            let is_hidden = timeline_item_should_be_hidden_for_key(
                key,
                has_renderable_content,
                is_redacted,
                thread_root.as_deref(),
            );
            let link_ranges =
                link_ranges_for_message_projection(body.as_deref(), formatted.as_ref());

            TimelineItem {
                request_state: request_state_for_item(
                    key_request_states,
                    withheld_codes,
                    key,
                    event_item,
                ),
                id,
                sender,
                sender_label,
                sender_avatar,
                body,
                notice_i18n,
                message_kind,
                spoiler_spans,
                timestamp_ms,
                in_reply_to_event_id,
                formatted,
                reply_quote,
                thread_root,
                thread_summary,
                media,
                link_previews: None,
                link_ranges,
                mentioned_user_ids,
                reactions,
                can_react,
                is_redacted,
                is_hidden,
                can_redact,
                is_edited,
                can_edit,
                unable_to_decrypt,
                actions,
                send_state,
                display_metadata: None,
            }
        }
        TimelineItemKind::Virtual(virtual_item) => {
            let (synthetic_id, timestamp_ms, is_hidden) = match virtual_item {
                VirtualTimelineItem::DateDivider(ts) => {
                    (format!("date-divider-{}", ts.0), Some(ts.0.into()), false)
                }
                VirtualTimelineItem::ReadMarker => ("read-marker".to_owned(), None, true),
                VirtualTimelineItem::TimelineStart => ("timeline-start".to_owned(), None, true),
            };
            TimelineItem {
                request_state: None,
                id: TimelineItemId::Synthetic { synthetic_id },
                sender: None,
                sender_label: None,
                sender_avatar: None,
                body: None,
                notice_i18n: None,
                message_kind: TimelineMessageKind::default(),
                spoiler_spans: Vec::new(),
                timestamp_ms,
                in_reply_to_event_id: None,
                formatted: None,
                reply_quote: None,
                thread_root: None,
                thread_summary: None,
                media: None,
                link_previews: None,
                link_ranges: Vec::new(),
                mentioned_user_ids: Vec::new(),
                reactions: Vec::new(),
                can_react: false,
                is_redacted: false,
                is_hidden,
                can_redact: false,
                is_edited: false,
                can_edit: false,
                unable_to_decrypt: None,
                actions: TimelineMessageActions::default(),
                send_state: None,
                display_metadata: None,
            }
        }
    }
}

/// Event id of a requestable UTD event for automatic key requests (issue
/// #460): decryptable-retry eligible (session known) and the original JSON is
/// available to re-request from.
pub(super) fn thread_auto_requestable_event_id(item: &Arc<SdkTimelineItem>) -> Option<String> {
    let event_item = item.as_event()?;
    let event_id = event_item.event_id()?.to_string();
    let requestable = event_item.content().is_unable_to_decrypt()
        && event_item.original_json().is_some()
        && unable_to_decrypt_from_content(event_item.content())
            .and_then(|utd| utd.session_id)
            .is_some();
    requestable.then_some(event_id)
}

/// Whether a late withheld observation should update/publish a presentation
/// state (issue #460): terminal stages are never regressed, and a stage
/// already settled `withheld` by a diff still gains the typed code when the
/// independent observation arrives later with a different code.
pub(super) fn withheld_update_should_publish(
    stage: &str,
    current_code: Option<&str>,
    new_code: &str,
) -> bool {
    !matches!(stage, "decryption_recovered" | "send_failed")
        && (stage != "withheld" || current_code != Some(new_code))
}

pub(super) fn unable_to_decrypt_from_content(
    content: &TimelineItemContent,
) -> Option<TimelineUnableToDecrypt> {
    let encrypted = content.as_unable_to_decrypt()?;
    let session_id = match encrypted {
        EncryptedMessage::MegolmV1AesSha2 { session_id, .. } => Some(session_id.clone()),
        EncryptedMessage::OlmV1Curve25519AesSha2 { .. } | EncryptedMessage::Unknown => None,
    };
    Some(TimelineUnableToDecrypt {
        reason: if session_id.is_some() {
            TimelineUnableToDecryptReason::MissingRoomKey
        } else {
            TimelineUnableToDecryptReason::Unknown
        },
        session_id,
        can_request_keys: false,
        recovery_stage: None,
        recovery_guidance: None,
    })
}

pub(super) fn decrypt_retry_reason_from_content(
    content: &TimelineItemContent,
) -> DecryptRetryReason {
    unable_to_decrypt_from_content(content)
        .map(|unable_to_decrypt| match unable_to_decrypt.reason {
            TimelineUnableToDecryptReason::MissingRoomKey => DecryptRetryReason::MissingRoomKey,
            TimelineUnableToDecryptReason::Withheld => DecryptRetryReason::Withheld,
            TimelineUnableToDecryptReason::Malformed => DecryptRetryReason::Malformed,
            TimelineUnableToDecryptReason::Unknown => DecryptRetryReason::Unknown,
        })
        .unwrap_or(DecryptRetryReason::Unknown)
}

pub(super) fn thread_summary_from_sdk(
    summary: matrix_sdk_ui::timeline::ThreadSummary,
) -> ThreadSummaryDto {
    let mut dto = ThreadSummaryDto {
        reply_count: summary.num_replies,
        latest_event_id: None,
        latest_sender: None,
        latest_sender_label: None,
        latest_body_preview: None,
        latest_timestamp_ms: None,
    };

    if let matrix_sdk_ui::timeline::TimelineDetails::Ready(latest_event) = summary.latest_event {
        dto.latest_event_id = match &latest_event.identifier {
            TimelineEventItemId::EventId(event_id) => Some(event_id.to_string()),
            TimelineEventItemId::TransactionId(_) => None,
        };
        dto.latest_sender = Some(latest_event.sender.to_string());
        dto.latest_sender_label = None;
        dto.latest_body_preview = latest_event
            .content
            .as_message()
            .map(|message| message.body().to_owned());
        dto.latest_timestamp_ms = Some(latest_event.timestamp.0.into());
    }

    dto
}

pub(super) struct MessageProjection {
    pub(super) body: Option<String>,
    pub(super) notice_i18n: Option<TimelineNoticeI18n>,
    pub(super) body_is_user_content: bool,
    pub(super) message_kind: TimelineMessageKind,
    pub(super) spoiler_spans: Vec<TimelineSpoilerSpan>,
    pub(super) media: Option<TimelineMedia>,
    pub(super) formatted: Option<koushi_protocol::event::TimelineFormattedBody>,
}

pub(super) fn link_ranges_for_message_projection(
    body: Option<&str>,
    formatted: Option<&koushi_protocol::event::TimelineFormattedBody>,
) -> Vec<koushi_protocol::event::TimelineLinkRange> {
    let source = formatted
        .map(|formatted_body| formatted_body.plain_text.as_str())
        .or(body)
        .unwrap_or("");
    extract_link_ranges(source)
}

fn reply_quote_from_details(details: &InReplyToDetails) -> ReplyQuote {
    match &details.event {
        TimelineDetails::Ready(event) => reply_quote_from_embedded_event(details, event),
        TimelineDetails::Unavailable | TimelineDetails::Pending | TimelineDetails::Error(_) => {
            ReplyQuote {
                event_id: details.event_id.to_string(),
                sender: None,
                sender_label: None,
                body_preview: None,
                formatted: None,
                state: ReplyQuoteState::Missing,
            }
        }
    }
}

fn reply_quote_from_embedded_event(
    details: &InReplyToDetails,
    event: &EmbeddedEvent,
) -> ReplyQuote {
    let sender = Some(event.sender.to_string());
    if event.content.is_redacted() {
        return ReplyQuote {
            event_id: details.event_id.to_string(),
            sender,
            sender_label: None,
            body_preview: None,
            formatted: None,
            state: ReplyQuoteState::Redacted,
        };
    }

    let projection = event
        .content
        .as_message()
        .map(|msg| message_projection_from_msgtype(msg.msgtype(), msg.body()));
    reply_quote_from_message_projection(&details.event_id.to_string(), sender, projection)
}

fn reply_quote_from_message_projection(
    event_id: &str,
    sender: Option<String>,
    projection: Option<MessageProjection>,
) -> ReplyQuote {
    let body_preview = projection
        .as_ref()
        .and_then(reply_quote_preview_from_message_projection);
    let formatted = projection
        .as_ref()
        .and_then(|projection| projection.formatted.as_ref())
        .map(reply_quote_formatted_body_from_timeline);
    let state = if body_preview.is_some() || formatted.is_some() {
        ReplyQuoteState::Ready
    } else {
        ReplyQuoteState::Unsupported
    };

    ReplyQuote {
        event_id: event_id.to_owned(),
        sender,
        sender_label: None,
        body_preview,
        formatted,
        state,
    }
}

fn reply_quote_formatted_body_from_timeline(
    formatted: &koushi_protocol::event::TimelineFormattedBody,
) -> ReplyQuoteFormattedBody {
    ReplyQuoteFormattedBody {
        html: formatted.html.clone(),
        plain_text: formatted.plain_text.clone(),
        code_blocks: formatted
            .code_blocks
            .iter()
            .map(|block| ReplyQuoteCodeBlock {
                language: block.language.clone(),
                body: block.body.clone(),
            })
            .collect(),
    }
}

fn reply_quote_preview_from_message_projection(projection: &MessageProjection) -> Option<String> {
    let source = projection.body.as_deref().or_else(|| {
        projection
            .media
            .as_ref()
            .map(|media| media.filename.as_str())
    })?;
    collapsed_preview(&source, REPLY_QUOTE_PREVIEW_MAX_CHARS)
}

fn message_projection_from_timeline_content(content: &TimelineItemContent) -> MessageProjection {
    if let Some(message) = content.as_message() {
        return message_projection_from_msgtype(message.msgtype(), message.body());
    }

    match content {
        TimelineItemContent::MembershipChange(change) => {
            return membership_change_projection(
                &change
                    .display_name()
                    .unwrap_or_else(|| change.user_id().to_string()),
                change.change(),
            );
        }
        TimelineItemContent::ProfileChange(change) => {
            return profile_change_projection(change);
        }
        TimelineItemContent::OtherState(state) => {
            if let AnyOtherStateEventContentChange::RoomName(change) = state.content() {
                return room_name_notice_projection(change);
            }
        }
        _ => {}
    }

    if let Some(sticker) = content.as_sticker() {
        return sticker_projection_from_body(&sticker.content().body);
    }

    if content.is_unable_to_decrypt() {
        return non_user_content_projection("Unable to decrypt message");
    }

    if content.is_poll() {
        return non_user_content_projection("Poll message");
    }

    if content.is_redacted() {
        return MessageProjection {
            body: None,
            notice_i18n: None,
            body_is_user_content: false,
            message_kind: TimelineMessageKind::Text,
            spoiler_spans: Vec::new(),
            media: None,
            formatted: None,
        };
    }

    let event_type = content
        .event_type_str()
        .unwrap_or_else(|| "unsupported Matrix event".to_owned());
    state_event_notice_projection(&event_type)
}

pub(super) fn sticker_projection_from_body(body: &str) -> MessageProjection {
    let body = body.trim();
    MessageProjection {
        body: (!body.is_empty()).then(|| body.to_owned()),
        notice_i18n: None,
        body_is_user_content: true,
        message_kind: TimelineMessageKind::Text,
        spoiler_spans: Vec::new(),
        media: None,
        formatted: None,
    }
}

fn state_event_notice_projection(event_type: &str) -> MessageProjection {
    MessageProjection {
        body: Some(state_event_notice_body(event_type).into_owned()),
        notice_i18n: state_event_notice_i18n(event_type).map(|key| TimelineNoticeI18n {
            key,
            old_name: None,
            new_name: None,
        }),
        body_is_user_content: false,
        message_kind: TimelineMessageKind::Notice,
        spoiler_spans: Vec::new(),
        media: None,
        formatted: None,
    }
}

fn state_event_notice_body(event_type: &str) -> Cow<'_, str> {
    match event_type {
        "m.room.create" => Cow::Borrowed("created the room"),
        "m.room.power_levels" => Cow::Borrowed("updated room permissions"),
        "m.room.guest_access" => Cow::Borrowed("updated guest access"),
        "m.room.encryption" => Cow::Borrowed("enabled room encryption"),
        "m.space.parent" => Cow::Borrowed("updated the parent space"),
        "m.room.join_rules" => Cow::Borrowed("updated join rules"),
        "m.room.history_visibility" => Cow::Borrowed("updated history visibility"),
        "m.room.pinned_events" => Cow::Borrowed("updated pinned messages"),
        _ => Cow::Owned(format!("Unsupported event: {event_type}")),
    }
}

fn state_event_notice_i18n(event_type: &str) -> Option<TimelineNoticeI18nKey> {
    match event_type {
        "m.room.create" => Some(TimelineNoticeI18nKey::RoomCreate),
        "m.room.power_levels" => Some(TimelineNoticeI18nKey::RoomPowerLevels),
        "m.room.guest_access" => Some(TimelineNoticeI18nKey::RoomGuestAccess),
        "m.room.encryption" => Some(TimelineNoticeI18nKey::RoomEncryption),
        "m.space.parent" => Some(TimelineNoticeI18nKey::SpaceParent),
        "m.room.join_rules" => Some(TimelineNoticeI18nKey::RoomJoinRules),
        "m.room.history_visibility" => Some(TimelineNoticeI18nKey::RoomHistoryVisibility),
        "m.room.pinned_events" => Some(TimelineNoticeI18nKey::RoomPinnedEvents),
        _ => None,
    }
}

fn room_name_notice_projection(
    change: &StateEventContentChange<RoomNameEventContent>,
) -> MessageProjection {
    let (body, notice_i18n) = match change {
        StateEventContentChange::Original {
            content,
            prev_content,
        } => {
            let current_name = content.name.as_str();
            let previous_name = prev_content
                .as_ref()
                .and_then(|content| content.name.as_deref())
                .filter(|name| !name.trim().is_empty());

            if current_name.trim().is_empty() {
                (
                    "removed the room name".to_owned(),
                    TimelineNoticeI18n {
                        key: TimelineNoticeI18nKey::RoomNameRemoved,
                        old_name: None,
                        new_name: None,
                    },
                )
            } else if previous_name.is_some_and(|previous| previous != current_name) {
                let previous_name = previous_name.expect("previous name was checked");
                (
                    format!("changed the room name from {previous_name} to {current_name}"),
                    TimelineNoticeI18n {
                        key: TimelineNoticeI18nKey::RoomNameChanged,
                        old_name: Some(previous_name.to_owned()),
                        new_name: Some(current_name.to_owned()),
                    },
                )
            } else {
                (
                    format!("set the room name to {current_name}"),
                    TimelineNoticeI18n {
                        key: TimelineNoticeI18nKey::RoomNameSet,
                        old_name: None,
                        new_name: Some(current_name.to_owned()),
                    },
                )
            }
        }
        StateEventContentChange::Redacted(_) => (
            "changed the room name".to_owned(),
            TimelineNoticeI18n {
                key: TimelineNoticeI18nKey::RoomNameChangedGeneric,
                old_name: None,
                new_name: None,
            },
        ),
    };

    MessageProjection {
        body: Some(body),
        notice_i18n: Some(notice_i18n),
        body_is_user_content: false,
        message_kind: TimelineMessageKind::Notice,
        spoiler_spans: Vec::new(),
        media: None,
        formatted: None,
    }
}

fn membership_change_projection(
    display_name: &str,
    change: Option<MembershipChange>,
) -> MessageProjection {
    let action = match change {
        Some(MembershipChange::Joined) | Some(MembershipChange::InvitationAccepted) => {
            "joined the room"
        }
        Some(MembershipChange::Left) => "left the room",
        Some(MembershipChange::Banned) => "was banned",
        Some(MembershipChange::Unbanned) => "was unbanned",
        Some(MembershipChange::Kicked) => "was kicked",
        Some(MembershipChange::Invited) => "was invited",
        Some(MembershipChange::InvitationRejected) => "rejected the invite",
        Some(MembershipChange::InvitationRevoked) => "had their invite revoked",
        Some(MembershipChange::Knocked) => "knocked",
        Some(MembershipChange::KnockAccepted) => "had their knock accepted",
        Some(MembershipChange::KnockRetracted) => "retracted their knock",
        Some(MembershipChange::KnockDenied) => "had their knock denied",
        Some(MembershipChange::KickedAndBanned) => "was kicked and banned",
        Some(MembershipChange::None) => "had a membership update",
        Some(MembershipChange::Error) | Some(MembershipChange::NotImplemented) | None => {
            "had a membership change"
        }
    };
    non_user_content_projection(&format!("{display_name} {action}"))
}

fn profile_change_projection(
    change: &matrix_sdk_ui::timeline::MemberProfileChange,
) -> MessageProjection {
    let body = match (
        change.displayname_change().is_some(),
        change.avatar_url_change().is_some(),
    ) {
        (false, true) => "changed their profile picture",
        (true, false) => "changed their display name",
        (true, true) => "changed their display name and profile picture",
        (false, false) => "updated their room profile",
    };
    non_user_content_projection(body)
}

pub(super) fn non_user_content_projection(body: &str) -> MessageProjection {
    MessageProjection {
        body: Some(body.to_owned()),
        notice_i18n: None,
        body_is_user_content: false,
        message_kind: TimelineMessageKind::Notice,
        spoiler_spans: Vec::new(),
        media: None,
        formatted: None,
    }
}

pub(super) fn collapsed_preview(value: &str, max_chars: usize) -> Option<String> {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }

    if collapsed.chars().count() <= max_chars {
        return Some(collapsed);
    }

    let mut preview = collapsed.chars().take(max_chars).collect::<String>();
    preview.push_str("...");
    Some(preview)
}

pub(super) fn message_projection_from_msgtype(
    msgtype: &MessageType,
    fallback_body: &str,
) -> MessageProjection {
    match msgtype {
        MessageType::Audio(content) => message_projection_from_body_and_formatted(
            content.caption(),
            content.formatted_caption(),
            TimelineMessageKind::Text,
            Some(timeline_media_from_audio(content)),
        ),
        MessageType::Emote(content) => message_projection_from_body_and_formatted(
            Some(fallback_body),
            content.formatted.as_ref(),
            TimelineMessageKind::Emote,
            None,
        ),
        MessageType::File(content) => message_projection_from_body_and_formatted(
            content.caption(),
            content.formatted_caption(),
            TimelineMessageKind::Text,
            Some(timeline_media_from_file(content)),
        ),
        MessageType::Image(content) => message_projection_from_body_and_formatted(
            content.caption(),
            content.formatted_caption(),
            TimelineMessageKind::Text,
            Some(timeline_media_from_image(content)),
        ),
        MessageType::Notice(content) => message_projection_from_body_and_formatted(
            Some(fallback_body),
            content.formatted.as_ref(),
            TimelineMessageKind::Notice,
            None,
        ),
        MessageType::Text(content) => message_projection_from_body_and_formatted(
            Some(fallback_body),
            content.formatted.as_ref(),
            TimelineMessageKind::Text,
            None,
        ),
        MessageType::Video(content) => message_projection_from_body_and_formatted(
            content.caption(),
            content.formatted_caption(),
            TimelineMessageKind::Text,
            Some(timeline_media_from_video(content)),
        ),
        _ => MessageProjection {
            body: Some(fallback_body.to_owned()),
            notice_i18n: None,
            body_is_user_content: true,
            message_kind: TimelineMessageKind::Text,
            spoiler_spans: Vec::new(),
            media: None,
            formatted: None,
        },
    }
}

fn message_projection_from_body_and_formatted(
    body: Option<&str>,
    formatted_body: Option<&FormattedBody>,
    message_kind: TimelineMessageKind,
    media: Option<TimelineMedia>,
) -> MessageProjection {
    let formatted = formatted_body.and_then(project_formatted_body);
    let spoiler_spans = formatted
        .as_ref()
        .map(|projection| projection.spoiler_spans.clone())
        .unwrap_or_default();
    let formatted = formatted.map(|projection| projection.formatted);
    let (body, spoiler_spans) = match (body, formatted.is_some()) {
        (Some(body), false) => {
            let projection = project_plain_body_with_spoilers(body);
            (Some(projection.body), projection.spoiler_spans)
        }
        (Some(body), true) => (Some(body.to_owned()), spoiler_spans),
        (None, _) => (None, spoiler_spans),
    };

    MessageProjection {
        body,
        notice_i18n: None,
        body_is_user_content: true,
        message_kind,
        spoiler_spans,
        media,
        formatted,
    }
}

struct PlainBodyProjection {
    body: String,
    spoiler_spans: Vec<TimelineSpoilerSpan>,
}

fn project_plain_body_with_spoilers(body: &str) -> PlainBodyProjection {
    let mut rendered = String::with_capacity(body.len());
    let mut spoiler_spans = Vec::new();
    let mut index = 0;

    while index < body.len() {
        let rest = &body[index..];
        if let Some(after) = rest.strip_prefix("||")
            && let Some(end) = after.find("||")
        {
            let start_utf16 = rendered.encode_utf16().count();
            rendered.push_str(&after[..end]);
            let end_utf16 = rendered.encode_utf16().count();
            if start_utf16 < end_utf16 {
                spoiler_spans.push(TimelineSpoilerSpan {
                    start_utf16,
                    end_utf16,
                    reason: None,
                });
            }
            index += 2 + end + 2;
            continue;
        }

        let ch = rest
            .chars()
            .next()
            .expect("rest is non-empty while projecting plain body");
        rendered.push(ch);
        index += ch.len_utf8();
    }

    PlainBodyProjection {
        body: rendered,
        spoiler_spans,
    }
}

struct FormattedBodyProjection {
    formatted: koushi_protocol::event::TimelineFormattedBody,
    spoiler_spans: Vec<TimelineSpoilerSpan>,
}

fn project_formatted_body(formatted_body: &FormattedBody) -> Option<FormattedBodyProjection> {
    if !matches!(&formatted_body.format, MessageFormat::Html) {
        return None;
    }

    let html = Html::parse(&formatted_body.body);
    html.sanitize_with(
        &SanitizerConfig::compat()
            .remove_reply_fallback()
            .remove_elements(["script", "style"]),
    );
    let sanitized_body = html.to_string();

    if sanitized_body.trim().is_empty() {
        return None;
    }

    let html = Html::parse(&sanitized_body);
    let plain_text = plain_text_from_html(&html);
    let code_blocks = code_blocks_from_html(&html);
    if plain_text.trim().is_empty() && code_blocks.iter().all(|block| block.body.trim().is_empty())
    {
        return None;
    }
    let spoiler_spans = spoiler_spans_from_html(&html);

    Some(FormattedBodyProjection {
        formatted: koushi_protocol::event::TimelineFormattedBody {
            html: sanitized_body,
            plain_text,
            code_blocks,
        },
        spoiler_spans,
    })
}

fn plain_text_from_html(html: &Html) -> String {
    let mut text = String::new();
    collect_plain_text_from_nodes(html.children(), &mut text);
    text
}

fn collect_plain_text_from_nodes(
    nodes: impl Iterator<Item = matrix_sdk::ruma::html::NodeRef>,
    out: &mut String,
) {
    for node in nodes {
        if let Some(text) = node.as_text() {
            out.push_str(&text.borrow());
            continue;
        }

        if node.as_element().is_some() {
            collect_plain_text_from_nodes(node.children(), out);
        }
    }
}

fn spoiler_spans_from_html(html: &Html) -> Vec<TimelineSpoilerSpan> {
    let mut spans = Vec::new();
    let mut offset_utf16 = 0;
    collect_spoiler_spans_from_nodes(html.children(), &mut offset_utf16, &mut spans);
    spans.sort_by_key(|span| (span.start_utf16, span.end_utf16));
    spans
}

fn collect_spoiler_spans_from_nodes(
    nodes: impl Iterator<Item = matrix_sdk::ruma::html::NodeRef>,
    offset_utf16: &mut usize,
    spans: &mut Vec<TimelineSpoilerSpan>,
) {
    for node in nodes {
        if let Some(text) = node.as_text() {
            *offset_utf16 += text.borrow().encode_utf16().count();
            continue;
        }

        let spoiler_reason = node.as_element().and_then(|element| {
            element.attrs.borrow().iter().find_map(|attr| {
                if attr.name.local.as_ref() != "data-mx-spoiler" {
                    return None;
                }
                let reason = attr.value.trim();
                Some((!reason.is_empty()).then(|| reason.to_owned()))
            })
        });

        let start_utf16 = *offset_utf16;
        collect_spoiler_spans_from_nodes(node.children(), offset_utf16, spans);
        if let Some(reason) = spoiler_reason {
            let end_utf16 = *offset_utf16;
            if start_utf16 < end_utf16 {
                spans.push(TimelineSpoilerSpan {
                    start_utf16,
                    end_utf16,
                    reason,
                });
            }
        }
    }
}

fn code_blocks_from_html(html: &Html) -> Vec<koushi_protocol::event::TimelineCodeBlock> {
    let mut blocks = Vec::new();
    collect_code_blocks_from_nodes(html.children(), &mut blocks);
    blocks
}

fn collect_code_blocks_from_nodes(
    nodes: impl Iterator<Item = matrix_sdk::ruma::html::NodeRef>,
    out: &mut Vec<koushi_protocol::event::TimelineCodeBlock>,
) {
    for node in nodes {
        let Some(element) = node.as_element() else {
            continue;
        };
        if element.name.local.as_ref() != "pre" {
            collect_code_blocks_from_nodes(node.children(), out);
            continue;
        }

        for child in node.children() {
            let Some(code_element) = child.as_element() else {
                continue;
            };
            if code_element.name.local.as_ref() != "code" {
                continue;
            }

            let language = code_element.attrs.borrow().iter().find_map(|attr| {
                if attr.name.local.as_ref() != "class" {
                    return None;
                }

                attr.value
                    .split_ascii_whitespace()
                    .find_map(|class_name| class_name.strip_prefix("language-"))
                    .map(|language| language.to_owned())
            });
            let mut body = String::new();
            collect_plain_text_from_nodes(child.children(), &mut body);

            out.push(koushi_protocol::event::TimelineCodeBlock { language, body });
            break;
        }

        collect_code_blocks_from_nodes(node.children(), out);
    }
}

fn timeline_media_from_audio(
    content: &matrix_sdk::ruma::events::room::message::AudioMessageEventContent,
) -> TimelineMedia {
    let info = content.info.as_deref();
    TimelineMedia {
        kind: TimelineMediaKind::Audio,
        filename: content.filename().to_owned(),
        source: timeline_media_source_from_sdk(&content.source),
        mimetype: info.and_then(|info| info.mimetype.clone()),
        size: info.and_then(|info| uint_to_u64(info.size.as_ref())),
        width: None,
        height: None,
        thumbnail: None,
    }
}

fn timeline_media_from_image(
    content: &matrix_sdk::ruma::events::room::message::ImageMessageEventContent,
) -> TimelineMedia {
    let info = content.info.as_deref();
    TimelineMedia {
        kind: TimelineMediaKind::Image,
        filename: content.filename().to_owned(),
        source: timeline_media_source_from_sdk(&content.source),
        mimetype: info.and_then(|info| info.mimetype.clone()),
        size: info.and_then(|info| uint_to_u64(info.size.as_ref())),
        width: info.and_then(|info| uint_to_u64(info.width.as_ref())),
        height: info.and_then(|info| uint_to_u64(info.height.as_ref())),
        thumbnail: info.and_then(|info| {
            timeline_media_thumbnail_from_sdk(
                info.thumbnail_source.as_ref(),
                info.thumbnail_info.as_deref(),
            )
        }),
    }
}

fn timeline_media_from_file(
    content: &matrix_sdk::ruma::events::room::message::FileMessageEventContent,
) -> TimelineMedia {
    let info = content.info.as_deref();
    TimelineMedia {
        kind: TimelineMediaKind::File,
        filename: content.filename().to_owned(),
        source: timeline_media_source_from_sdk(&content.source),
        mimetype: info.and_then(|info| info.mimetype.clone()),
        size: info.and_then(|info| uint_to_u64(info.size.as_ref())),
        width: None,
        height: None,
        thumbnail: info.and_then(|info| {
            timeline_media_thumbnail_from_sdk(
                info.thumbnail_source.as_ref(),
                info.thumbnail_info.as_deref(),
            )
        }),
    }
}

fn timeline_media_from_video(
    content: &matrix_sdk::ruma::events::room::message::VideoMessageEventContent,
) -> TimelineMedia {
    let info = content.info.as_deref();
    TimelineMedia {
        kind: TimelineMediaKind::Video,
        filename: content.filename().to_owned(),
        source: timeline_media_source_from_sdk(&content.source),
        mimetype: info.and_then(|info| info.mimetype.clone()),
        size: info.and_then(|info| uint_to_u64(info.size.as_ref())),
        width: info.and_then(|info| uint_to_u64(info.width.as_ref())),
        height: info.and_then(|info| uint_to_u64(info.height.as_ref())),
        thumbnail: info.and_then(|info| {
            timeline_media_thumbnail_from_sdk(
                info.thumbnail_source.as_ref(),
                info.thumbnail_info.as_deref(),
            )
        }),
    }
}

pub(super) fn timeline_media_source_from_sdk(source: &MediaSource) -> TimelineMediaSource {
    match source {
        MediaSource::Plain(uri) => TimelineMediaSource {
            mxc_uri: uri.to_string(),
            encrypted: false,
            encryption_version: None,
        },
        MediaSource::Encrypted(file) => TimelineMediaSource {
            mxc_uri: file.url.to_string(),
            encrypted: true,
            encryption_version: Some(file.info.version().to_owned()),
        },
    }
}

fn timeline_media_thumbnail_from_sdk(
    source: Option<&MediaSource>,
    info: Option<&ThumbnailInfo>,
) -> Option<TimelineMediaThumbnail> {
    source.map(|source| TimelineMediaThumbnail {
        source: timeline_media_source_from_sdk(source),
        mimetype: info.and_then(|info| info.mimetype.clone()),
        size: info.and_then(|info| uint_to_u64(info.size.as_ref())),
        width: info.and_then(|info| uint_to_u64(info.width.as_ref())),
        height: info.and_then(|info| uint_to_u64(info.height.as_ref())),
    })
}

pub(super) fn timeline_send_state_from_sdk(
    state: Option<&SdkEventSendState>,
) -> Option<TimelineSendState> {
    match state {
        Some(SdkEventSendState::NotSentYet { .. }) => Some(TimelineSendState::Sending),
        Some(SdkEventSendState::SendingFailed { is_recoverable, .. }) => {
            Some(TimelineSendState::NotSent {
                reason: send_failure_reason(*is_recoverable),
            })
        }
        Some(SdkEventSendState::Sent { .. }) => Some(TimelineSendState::Sent),
        None => None,
    }
}

pub(super) fn send_failure_reason(is_recoverable: bool) -> TimelineSendFailureReason {
    if is_recoverable {
        TimelineSendFailureReason::Recoverable
    } else {
        TimelineSendFailureReason::Unrecoverable
    }
}

pub(super) fn remember_local_echo(
    statuses: &mut HashMap<String, TimelineSendState>,
    handles: &mut HashMap<String, SendHandle>,
    echo: &LocalEcho,
) {
    let transaction_id = echo.transaction_id.to_string();
    if let LocalEchoContent::Event {
        send_handle,
        send_error,
        ..
    } = &echo.content
    {
        let state = if send_error.is_some() {
            TimelineSendState::NotSent {
                reason: TimelineSendFailureReason::Unrecoverable,
            }
        } else {
            TimelineSendState::Sending
        };
        statuses.insert(transaction_id.clone(), state);
        handles.insert(transaction_id, send_handle.clone());
    }
}

fn private_media_entry_from_msgtype(msgtype: &MessageType) -> Option<PrivateMediaEntry> {
    match msgtype {
        MessageType::Image(content) => {
            let info = content.info.as_deref();
            Some(PrivateMediaEntry {
                source: content.source.clone(),
                thumbnail_source: info.and_then(|info| info.thumbnail_source.clone()),
                mimetype: info.and_then(|info| info.mimetype.clone()),
                size: info
                    .and_then(|info| uint_to_u64(info.size.as_ref()))
                    .unwrap_or(0),
                width: info.and_then(|info| uint_to_u64(info.width.as_ref())),
                height: info.and_then(|info| uint_to_u64(info.height.as_ref())),
            })
        }
        MessageType::File(content) => {
            let info = content.info.as_deref();
            Some(PrivateMediaEntry {
                source: content.source.clone(),
                thumbnail_source: info.and_then(|info| info.thumbnail_source.clone()),
                mimetype: info.and_then(|info| info.mimetype.clone()),
                size: info
                    .and_then(|info| uint_to_u64(info.size.as_ref()))
                    .unwrap_or(0),
                width: None,
                height: None,
            })
        }
        MessageType::Audio(content) => {
            let info = content.info.as_deref();
            Some(PrivateMediaEntry {
                source: content.source.clone(),
                thumbnail_source: None,
                mimetype: info.and_then(|info| info.mimetype.clone()),
                size: info
                    .and_then(|info| uint_to_u64(info.size.as_ref()))
                    .unwrap_or(0),
                width: None,
                height: None,
            })
        }
        MessageType::Video(content) => {
            let info = content.info.as_deref();
            Some(PrivateMediaEntry {
                source: content.source.clone(),
                thumbnail_source: info.and_then(|info| info.thumbnail_source.clone()),
                mimetype: info.and_then(|info| info.mimetype.clone()),
                size: info
                    .and_then(|info| uint_to_u64(info.size.as_ref()))
                    .unwrap_or(0),
                width: info.and_then(|info| uint_to_u64(info.width.as_ref())),
                height: info.and_then(|info| uint_to_u64(info.height.as_ref())),
            })
        }
        _ => None,
    }
}

pub(super) fn cache_sdk_item_media_source(
    cache: &mut HashMap<String, PrivateMediaEntry>,
    item: &Arc<SdkTimelineItem>,
) {
    use matrix_sdk_ui::timeline::TimelineItemKind;

    let TimelineItemKind::Event(event_item) = item.kind() else {
        return;
    };
    let Some(event_id) = event_item.event_id() else {
        return;
    };
    let Some(message) = event_item.content().as_message() else {
        return;
    };
    let Some(entry) = private_media_entry_from_msgtype(message.msgtype()) else {
        return;
    };

    cache.insert(event_id.to_string(), entry);
}

pub(super) fn attachment_info_for_upload(request: &UploadMediaRequest) -> AttachmentInfo {
    let size = u64::try_from(request.bytes.len())
        .ok()
        .and_then(uint_from_u64);

    match request.kind {
        UploadMediaKind::Image { width, height } => AttachmentInfo::Image(BaseImageInfo {
            width: width.and_then(uint_from_u64),
            height: height.and_then(uint_from_u64),
            size,
            ..Default::default()
        }),
        UploadMediaKind::File => AttachmentInfo::File(BaseFileInfo { size }),
    }
}

pub(super) fn thumbnail_for_upload(request: &UploadMediaRequest) -> Option<Thumbnail> {
    let thumbnail = request.thumbnail.as_ref()?;
    Some(Thumbnail {
        data: thumbnail.bytes.clone(),
        content_type: thumbnail.mime_type.parse().ok()?,
        height: uint_from_u64(thumbnail.height)?,
        width: uint_from_u64(thumbnail.width)?,
        size: uint_from_u64(u64::try_from(thumbnail.bytes.len()).ok()?)?,
    })
}

pub(super) fn media_request_for_download(
    entry: &PrivateMediaEntry,
    selection: &MediaDownloadSelection,
) -> Option<MediaRequestParameters> {
    match selection {
        MediaDownloadSelection::File => Some(MediaRequestParameters {
            source: entry.source.clone(),
            format: MediaFormat::File,
        }),
        MediaDownloadSelection::Thumbnail { width, height } => {
            if let Some(source) = entry.thumbnail_source.clone() {
                return Some(MediaRequestParameters {
                    source,
                    format: MediaFormat::File,
                });
            }
            Some(MediaRequestParameters {
                source: entry.source.clone(),
                format: MediaFormat::Thumbnail(MediaThumbnailSettings::new(
                    uint_from_u64(*width)?,
                    uint_from_u64(*height)?,
                )),
            })
        }
    }
}

/// Produce a path-safe hex string from a Matrix identifier.
///
/// Matrix room ids and event ids contain `!`, `$`, `#`, `:`, `.`, and `/`
/// which are illegal or ambiguous in file-system path components on Windows
/// and some POSIX contexts.  We hash the identifier to a fixed-length hex
/// string so the path component is always safe.  The original identifier is
/// never written to the filesystem; it is only used as the hash input.
pub(super) fn sanitize_matrix_id_for_path(id: &str) -> String {
    use std::hash::Hash;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut hasher);
    std::hash::Hasher::finish(&hasher)
        .to_string()
        .chars()
        .take(16)
        .collect()
}

fn uint_to_u64(value: Option<&matrix_sdk::ruma::UInt>) -> Option<u64> {
    value.map(|value| (*value).into())
}

fn uint_from_u64(value: u64) -> Option<matrix_sdk::ruma::UInt> {
    matrix_sdk::ruma::UInt::try_from(value).ok()
}

pub(crate) fn timeline_item_can_react(
    is_event_backed: bool,
    can_hold_reactions: bool,
    is_redacted: bool,
    has_renderable_content: bool,
) -> bool {
    is_event_backed && can_hold_reactions && !is_redacted && has_renderable_content
}

pub(crate) fn validate_send_reaction(
    can_react: bool,
    my_reaction_event_id: Option<&str>,
) -> Result<(), TimelineFailureKind> {
    if !can_react {
        return Err(TimelineFailureKind::InvalidReactionTarget);
    }
    if my_reaction_event_id.is_some() {
        return Err(TimelineFailureKind::InvalidReactionState);
    }
    Ok(())
}

pub(crate) fn validate_redact_reaction(
    can_react: bool,
    my_reaction_event_id: Option<&str>,
    reaction_event_id: &str,
) -> Result<(), TimelineFailureKind> {
    if !can_react {
        return Err(TimelineFailureKind::InvalidReactionTarget);
    }
    match my_reaction_event_id {
        Some(current) if current == reaction_event_id => Ok(()),
        _ => Err(TimelineFailureKind::InvalidReactionState),
    }
}

pub(crate) fn timeline_item_can_redact(
    is_event_backed: bool,
    is_own_message: bool,
    is_redacted: bool,
    has_renderable_content: bool,
) -> bool {
    is_event_backed && is_own_message && !is_redacted && has_renderable_content
}

pub(crate) fn timeline_item_can_edit(
    is_event_backed: bool,
    is_own_message: bool,
    is_redacted: bool,
    has_editable_body: bool,
) -> bool {
    is_event_backed && is_own_message && !is_redacted && has_editable_body
}

/// Shape of a media message, used for the edit decision and its diagnostics.
///
/// The presence of a shape is what marks a message type as media; it is the
/// single list both `msgtype_carries_editable_caption` and the edit diagnostics
/// read, so the two cannot drift apart.
#[derive(Clone, Copy)]
struct MediaMessageShape {
    encrypted: bool,
    has_info: bool,
    has_caption: bool,
}

fn msgtype_media_shape(msgtype: &MessageType) -> Option<MediaMessageShape> {
    fn shape(source: &MediaSource, has_info: bool, has_caption: bool) -> Option<MediaMessageShape> {
        Some(MediaMessageShape {
            encrypted: matches!(source, MediaSource::Encrypted(_)),
            has_info,
            has_caption,
        })
    }

    match msgtype {
        MessageType::Audio(content) => shape(
            &content.source,
            content.info.is_some(),
            content.caption().is_some(),
        ),
        MessageType::File(content) => shape(
            &content.source,
            content.info.is_some(),
            content.caption().is_some(),
        ),
        MessageType::Image(content) => shape(
            &content.source,
            content.info.is_some(),
            content.caption().is_some(),
        ),
        MessageType::Video(content) => shape(
            &content.source,
            content.info.is_some(),
            content.caption().is_some(),
        ),
        _ => None,
    }
}

/// Whether the SDK can edit this message type's caption in place.
///
/// Media message types carry the attachment in the same `m.room.message`
/// content as the caption, and the caption-preserving SDK path supports exactly
/// the types that `msgtype_media_shape` recognises.
fn msgtype_carries_editable_caption(msgtype: &MessageType) -> bool {
    msgtype_media_shape(msgtype).is_some()
}

/// Resolve the SDK message type behind an edit target.
///
/// This mirrors the SDK's own `rfind_event_by_item_id`, which `Timeline::edit`
/// uses to locate the item: last match wins, an event id matches any item that
/// carries it (including a local echo the server has already accepted), and a
/// transaction id matches the local echo that owns it. Comparing
/// `EventTimelineItem::identifier()` instead would miss a sent local echo looked
/// up by transaction id, because that item reports its event id.
///
/// One SDK case stays out of reach: a remote item's originating transaction id
/// has no public accessor. Such an item always carries an event id, and
/// `item_ids_for_event` tries the event id first, so the caption decision still
/// sees it.
///
/// Returns `None` when the target is absent from the timeline or is not an
/// `m.room.message` (state events, polls, stickers).
fn edit_target_msgtype<'items>(
    items: &'items eyeball_im::Vector<Arc<SdkTimelineItem>>,
    item_id: &TimelineEventItemId,
) -> Option<&'items MessageType> {
    use matrix_sdk_ui::timeline::TimelineItemKind;

    items.iter().rev().find_map(|item| {
        let TimelineItemKind::Event(event_item) = item.kind() else {
            return None;
        };
        let is_target = match item_id {
            TimelineEventItemId::EventId(event_id) => {
                event_item.event_id() == Some(event_id.as_ref())
            }
            TimelineEventItemId::TransactionId(transaction_id) => {
                event_item.transaction_id() == Some(transaction_id.as_ref())
            }
        };
        if !is_target {
            return None;
        }
        event_item
            .content()
            .as_message()
            .map(|message| message.msgtype())
    })
}

/// Choose the replacement content for an edit of `body`.
///
/// A media message keeps its attachment (`url`/`file`/`info`/`filename`) in the
/// same content as its caption, so replacing the event with `m.text` drops the
/// attachment and reads as data loss in the timeline (issue #328). Media rows
/// therefore edit the caption in place; everything else keeps the plain-text
/// replacement. This decision stays in core because the GUI submits only the new
/// visible text and never sees the original Matrix content.
fn edited_document_content_for_edit_target(
    msgtype: Option<&MessageType>,
    document: &ComposerDocument,
) -> EditedContent {
    let body = document.plain_body();
    let formatted_body = document.formatted_body();
    let mentions = document.mention_intent();
    if msgtype.is_some_and(msgtype_carries_editable_caption) {
        return EditedContent::MediaCaption {
            caption: Some(body),
            formatted_caption: formatted_body.map(FormattedBody::html),
            mentions: ruma_mentions_from_intent(&mentions),
        };
    }

    let mut content = match (msgtype, formatted_body) {
        (Some(MessageType::Emote(_)), Some(formatted)) => {
            RoomMessageEventContentWithoutRelation::emote_html(body, formatted)
        }
        (Some(MessageType::Notice(_)), Some(formatted)) => {
            RoomMessageEventContentWithoutRelation::notice_html(body, formatted)
        }
        (_, Some(formatted)) => RoomMessageEventContentWithoutRelation::text_html(body, formatted),
        (Some(MessageType::Emote(_)), None) => {
            RoomMessageEventContentWithoutRelation::emote_plain(body)
        }
        (Some(MessageType::Notice(_)), None) => {
            RoomMessageEventContentWithoutRelation::notice_plain(body)
        }
        (_, None) => RoomMessageEventContentWithoutRelation::text_plain(body),
    };
    if let Some(mentions) = ruma_mentions_from_intent(&mentions) {
        content = content.add_mentions(mentions);
    }
    EditedContent::RoomMessage(content)
}

#[cfg(test)]
fn edited_content_for_edit_target(
    msgtype: Option<&MessageType>,
    body: &str,
    mentions: &MentionIntent,
) -> EditedContent {
    if msgtype.is_some_and(msgtype_carries_editable_caption) {
        return EditedContent::MediaCaption {
            caption: Some(body.to_owned()),
            formatted_caption: None,
            mentions: ruma_mentions_from_intent(mentions),
        };
    }

    let mut content = match msgtype {
        Some(MessageType::Emote(_)) => RoomMessageEventContentWithoutRelation::emote_plain(body),
        Some(MessageType::Notice(_)) => RoomMessageEventContentWithoutRelation::notice_plain(body),
        _ => RoomMessageEventContentWithoutRelation::text_plain(body),
    };
    if let Some(mentions) = ruma_mentions_from_intent(mentions) {
        content = content.add_mentions(mentions);
    }
    EditedContent::RoomMessage(content)
}

fn message_edit_target_token(msgtype: Option<&MessageType>) -> &'static str {
    match msgtype {
        None => "unresolved",
        Some(MessageType::Audio(_)) => "audio",
        Some(MessageType::Emote(_)) => "emote",
        Some(MessageType::File(_)) => "file",
        Some(MessageType::Image(_)) => "image",
        Some(MessageType::Notice(_)) => "notice",
        Some(MessageType::Text(_)) => "text",
        Some(MessageType::Video(_)) => "video",
        Some(_) => "other",
    }
}

/// Record which replacement shape an edit chose for its target.
///
/// Private-data-free: message type tokens and presence booleans only, never
/// bodies, captions, filenames, MXC URIs, event ids, or raw SDK errors.
fn trace_message_edit_target(msgtype: Option<&MessageType>, content: &EditedContent) {
    let media = msgtype.and_then(msgtype_media_shape);
    koushi_diagnostics::record(
        DiagnosticEvent::new(
            DiagnosticLevel::Info,
            "core.timeline_edit",
            "replacement_selected",
        )
        .field(DiagnosticField::token(
            "target",
            message_edit_target_token(msgtype),
        ))
        .field(DiagnosticField::token(
            "replacement",
            match content {
                EditedContent::MediaCaption { .. } => "media_caption",
                _ => "text",
            },
        ))
        .field(DiagnosticField::boolean(
            "has_url",
            media.is_some_and(|shape| !shape.encrypted),
        ))
        .field(DiagnosticField::boolean(
            "has_file",
            media.is_some_and(|shape| shape.encrypted),
        ))
        .field(DiagnosticField::boolean(
            "has_info",
            media.is_some_and(|shape| shape.has_info),
        ))
        .field(DiagnosticField::boolean(
            "has_caption",
            media.is_some_and(|shape| shape.has_caption),
        )),
    );
}

fn mention_summary_for_message_type(msgtype: Option<&MessageType>) -> (HashSet<String>, bool) {
    let Some(msgtype) = msgtype else {
        return (HashSet::new(), false);
    };
    let Ok(raw) = serde_json::to_value(msgtype) else {
        return (HashSet::new(), false);
    };
    let Some(mentions) = raw.get("m.mentions") else {
        return (HashSet::new(), false);
    };
    let users = mentions
        .get("user_ids")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::to_owned)
        .collect();
    let room = mentions
        .get("room")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    (users, room)
}

fn mention_counts_for_edit(
    original: Option<&MessageType>,
    edited: &EditedContent,
) -> (usize, usize) {
    let old = mention_summary_for_message_type(original);
    let next = match edited {
        EditedContent::RoomMessage(content) => content
            .mentions
            .as_ref()
            .map(|mentions| {
                (
                    mentions.user_ids.iter().map(ToString::to_string).collect(),
                    mentions.room,
                )
            })
            .unwrap_or_default(),
        EditedContent::MediaCaption { mentions, .. } => mentions
            .as_ref()
            .map(|mentions| {
                (
                    mentions.user_ids.iter().map(ToString::to_string).collect(),
                    mentions.room,
                )
            })
            .unwrap_or_default(),
        EditedContent::PollStart { .. } => (HashSet::new(), false),
    };
    let final_count = next.0.len() + usize::from(next.1);
    let revision_count = next.0.difference(&old.0).count() + usize::from(next.1 && !old.1);
    (final_count, revision_count)
}

fn trace_message_edit_lifecycle(
    stage: &'static str,
    target: &'static str,
    original_mention_count: usize,
    final_mention_count: usize,
    revision_mention_count: Option<usize>,
    outcome: Option<&'static str>,
) {
    let mut event = DiagnosticEvent::new(DiagnosticLevel::Info, "core.timeline_edit", stage)
        .field(DiagnosticField::token("target", target))
        .field(DiagnosticField::count(
            "original_mention_count",
            original_mention_count.try_into().unwrap_or(u64::MAX),
        ))
        .field(DiagnosticField::count(
            "final_mention_count",
            final_mention_count.try_into().unwrap_or(u64::MAX),
        ));
    if let Some(count) = revision_mention_count {
        event = event.field(DiagnosticField::count(
            "revision_mention_count",
            count.try_into().unwrap_or(u64::MAX),
        ));
    }
    if let Some(outcome) = outcome {
        event = event.field(DiagnosticField::token("outcome", outcome));
    }
    koushi_diagnostics::record(event);
}

pub(crate) fn validate_retry_send(
    state: Option<&TimelineSendState>,
) -> Result<(), TimelineFailureKind> {
    match state {
        Some(TimelineSendState::NotSent { .. }) => Ok(()),
        Some(
            TimelineSendState::Sending | TimelineSendState::Cancelled | TimelineSendState::Sent,
        ) => Err(TimelineFailureKind::InvalidSendState),
        None => Err(TimelineFailureKind::InvalidSendTarget),
    }
}

pub(crate) fn validate_cancel_send(
    state: Option<&TimelineSendState>,
) -> Result<(), TimelineFailureKind> {
    match state {
        Some(TimelineSendState::Sending | TimelineSendState::NotSent { .. }) => Ok(()),
        Some(TimelineSendState::Cancelled | TimelineSendState::Sent) => {
            Err(TimelineFailureKind::InvalidSendState)
        }
        None => Err(TimelineFailureKind::InvalidSendTarget),
    }
}

pub(crate) fn reaction_groups_from_sdk(
    reactions: &ReactionsByKeyBySender,
    own_user_id: Option<&matrix_sdk::ruma::UserId>,
) -> Vec<koushi_protocol::event::ReactionGroup> {
    reactions
        .iter()
        .map(|(key, senders)| koushi_protocol::event::ReactionGroup {
            key: key.clone(),
            count: senders.len().min(u32::MAX as usize) as u32,
            reacted_by_me: own_user_id
                .map(|user_id| {
                    senders
                        .keys()
                        .any(|sender| sender.as_str() == user_id.as_str())
                })
                .unwrap_or(false),
            my_reaction_event_id: own_user_id.and_then(|user_id| {
                senders.iter().find_map(|(sender, info)| {
                    if sender.as_str() == user_id.as_str() {
                        match &info.status {
                            ReactionStatus::RemoteToRemote(event_id) => Some(event_id.to_string()),
                            ReactionStatus::LocalToLocal(_) | ReactionStatus::LocalToRemote(_) => {
                                None
                            }
                        }
                    } else {
                        None
                    }
                })
            }),
            sender_preview: senders
                .keys()
                .take(3)
                .map(|sender| ReactionSender {
                    user_id: sender.to_string(),
                    display_label: None,
                })
                .collect(),
        })
        .collect()
}

/// Convert one ordered SDK diff batch while tracking the evolving canonical
/// length. `PopBack` has no explicit index and `Append` has no equivalent wire
/// variant, so both must be expanded with the batch's actual preceding state.
pub(super) fn sdk_vector_diffs_to_timeline_diffs(
    diffs: &[eyeball_im::VectorDiff<Arc<SdkTimelineItem>>],
    initial_canonical_len: usize,
    key: &TimelineKey,
    own_user_id: Option<&matrix_sdk::ruma::UserId>,
    send_statuses: &HashMap<String, TimelineSendState>,
    key_request_states: Option<&std::collections::BTreeMap<String, KeyRequestUiState>>,
    withheld_codes: Option<&std::collections::BTreeMap<(String, String), &'static str>>,
) -> Vec<TimelineDiff> {
    let mut canonical_len = initial_canonical_len;
    let mut converted = Vec::with_capacity(diffs.len());
    for diff in diffs {
        match diff {
            eyeball_im::VectorDiff::PushFront { value } => {
                converted.push(TimelineDiff::PushFront {
                    item: sdk_item_to_timeline_item_with_send_states(
                        key,
                        value,
                        own_user_id,
                        send_statuses,
                        None,
                        key_request_states,
                        withheld_codes,
                    ),
                });
                canonical_len += 1;
            }
            eyeball_im::VectorDiff::PushBack { value } => {
                converted.push(TimelineDiff::PushBack {
                    item: sdk_item_to_timeline_item_with_send_states(
                        key,
                        value,
                        own_user_id,
                        send_statuses,
                        None,
                        key_request_states,
                        withheld_codes,
                    ),
                });
                canonical_len += 1;
            }
            eyeball_im::VectorDiff::Insert { index, value } => {
                converted.push(TimelineDiff::Insert {
                    index: *index,
                    item: sdk_item_to_timeline_item_with_send_states(
                        key,
                        value,
                        own_user_id,
                        send_statuses,
                        None,
                        key_request_states,
                        withheld_codes,
                    ),
                });
                canonical_len += 1;
            }
            eyeball_im::VectorDiff::Set { index, value } => {
                converted.push(TimelineDiff::Set {
                    index: *index,
                    item: sdk_item_to_timeline_item_with_send_states(
                        key,
                        value,
                        own_user_id,
                        send_statuses,
                        None,
                        key_request_states,
                        withheld_codes,
                    ),
                });
            }
            eyeball_im::VectorDiff::Remove { index } => {
                converted.push(TimelineDiff::Remove { index: *index });
                if *index < canonical_len {
                    canonical_len -= 1;
                }
            }
            eyeball_im::VectorDiff::Truncate { length } => {
                converted.push(TimelineDiff::Truncate { length: *length });
                canonical_len = canonical_len.min(*length);
            }
            eyeball_im::VectorDiff::Clear => {
                converted.push(TimelineDiff::Clear);
                canonical_len = 0;
            }
            eyeball_im::VectorDiff::Reset { values } => {
                converted.push(TimelineDiff::Reset {
                    items: values
                        .iter()
                        .map(|value| {
                            sdk_item_to_timeline_item_with_send_states(
                                key,
                                value,
                                own_user_id,
                                send_statuses,
                                None,
                                None,
                                None,
                            )
                        })
                        .collect(),
                });
                canonical_len = values.len();
            }
            eyeball_im::VectorDiff::PopFront => {
                if canonical_len > 0 {
                    converted.push(TimelineDiff::Remove { index: 0 });
                    canonical_len -= 1;
                }
            }
            eyeball_im::VectorDiff::PopBack => {
                if canonical_len > 0 {
                    canonical_len -= 1;
                    converted.push(TimelineDiff::Remove {
                        index: canonical_len,
                    });
                }
            }
            eyeball_im::VectorDiff::Append { values } => {
                converted.extend(values.iter().map(|value| TimelineDiff::PushBack {
                    item: sdk_item_to_timeline_item_with_send_states(
                        key,
                        value,
                        own_user_id,
                        send_statuses,
                        None,
                        key_request_states,
                        withheld_codes,
                    ),
                }));
                canonical_len += values.len();
            }
        }
    }
    converted
}

fn classify_reaction_error(err: &matrix_sdk_ui::timeline::Error) -> TimelineFailureKind {
    match err {
        matrix_sdk_ui::timeline::Error::EventNotInTimeline(_) => {
            TimelineFailureKind::InvalidReactionTarget
        }
        _ => TimelineFailureKind::Sdk,
    }
}

#[cfg(test)]
mod tests;
