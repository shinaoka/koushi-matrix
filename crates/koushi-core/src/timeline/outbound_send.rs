use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::task::Poll;
use std::time::{Duration, Instant};

use futures_util::{FutureExt, StreamExt, stream::FuturesUnordered};
use koushi_diagnostics::{DiagnosticEvent, DiagnosticField, DiagnosticLevel, record};
use koushi_sdk::MatrixClientSession;
use koushi_state::{AppAction, ComposerDocument, ComposerFormattingOptions, MediaTransferProgress};

use crate::send_diagnostics::{SendFailureDiagnostic, classify_send_failure};
use matrix_sdk::attachment::AttachmentConfig;
use matrix_sdk::room::reply::Reply;
use matrix_sdk::ruma::events::room::message::AddMentions;
use matrix_sdk::send_queue::{RoomSendQueueUpdate, SendQueueUpdate};
use matrix_sdk_ui::timeline::Timeline;
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::account_work::{AccountWorkKind, InteractiveWorkGuard};
use crate::command::UploadMediaRequest;
use crate::event::{CoreEvent, TimelineEvent, TimelineItem, TimelineItemId, TimelineSendState};
use crate::executor;
use crate::failure::{CoreFailure, TimelineFailureKind};
use crate::ids::{RequestId, TimelineKey, TimelineKind};
use crate::runtime::ForwardedComposerDraftPermit;

// BEGIN GENERATED SIBLING IMPORTS
use super::actor::{
    TimelineActor, TimelineActorCleanupIngress, TimelineActorMessage, emit_app_action_reliable,
};
use super::composer::{
    build_room_message_content_from_composer_document_with_options,
    build_room_message_content_without_relation_from_composer_document_with_options,
    media_caption_content_from_draft, ruma_mentions_from_intent,
};
use super::diagnostics::{
    OutboundSessionLookupDiagnostic, record_post_send_encryption_snapshot,
    record_send_diagnostic_snapshot_skipped, trace_timeline_items,
};
use super::display_projection::DisplayProjectionState;
use super::item_projection::{
    apply_ignored_sender_suppression, apply_link_previews_to_item, attachment_info_for_upload,
    attachment_reply_for_key, is_attention_eligible_event, remember_local_echo,
    reply_enforce_thread_for_key, sdk_item_to_timeline_item_with_send_states, send_failure_reason,
    thumbnail_for_upload, timeline_media_source_from_sdk, timeline_room_id, validate_cancel_send,
    validate_retry_send,
};
use super::manager::TimelineManagerActor;
use super::navigation::{
    InitialItemsRequestIdentity, PreparedInitialWindow,
    commit_prepared_initial_window_for_generation,
};
use super::room_key_recovery::RoomKeyReshareSchedule;
use super::thread_projection::{
    ThreadAttentionBatchProvenance, ThreadAttentionCounters,
    replay_known_candidates_for_display_items,
};
// END GENERATED SIBLING IMPORTS

/// One absolute deadline for the complete set of manager-owned enqueue workers.
/// This is deliberately not a per-worker timeout, so shutdown latency cannot
/// grow with the number of outstanding sends.

const SEND_ENQUEUE_WORKER_SHUTDOWN_DEADLINE: Duration = Duration::from_secs(5);

pub(super) struct TimelineSendCompletionDelivery {
    pub(super) request_id: RequestId,
    pub(super) key: TimelineKey,
    pub(super) transaction_id: String,
    pub(super) event_id: String,
    pub(super) diagnostic_correlation: Option<u64>,
}

pub(super) struct TimelineSendFailureDelivery {
    pub(super) request_id: RequestId,
    pub(super) failure: CoreFailure,
}

/// Internal payload accepted only through the manager-owned terminal ingress.
/// Replaceable timeline actors cannot deliver reducer actions and completion
/// events independently.
pub(super) struct TimelineSendTerminalHandoff {
    pub(super) submission_id: Option<koushi_state::SubmissionId>,
    pub(super) action: Option<AppAction>,
    pub(super) completion: Option<TimelineSendCompletionDelivery>,
    pub(super) failure: Option<TimelineSendFailureDelivery>,
}

#[derive(Clone)]
pub(super) struct TimelineSendTerminalIngress {
    tx: mpsc::UnboundedSender<TimelineSendTerminalHandoff>,
    accepting: Arc<std::sync::atomic::AtomicBool>,
}

pub(super) enum TimelineSendTerminalAdmission {
    Accepted,
    ClosedForShutdown,
}

impl TimelineSendTerminalIngress {
    pub(super) fn channel() -> (Self, mpsc::UnboundedReceiver<TimelineSendTerminalHandoff>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            Self {
                tx,
                accepting: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            },
            rx,
        )
    }

    pub(super) fn admit(
        &self,
        handoff: TimelineSendTerminalHandoff,
    ) -> TimelineSendTerminalAdmission {
        if !self.accepting.load(Ordering::Acquire) {
            return TimelineSendTerminalAdmission::ClosedForShutdown;
        }
        match self.tx.send(handoff) {
            Ok(()) => TimelineSendTerminalAdmission::Accepted,
            Err(_) => {
                debug_assert!(
                    !self.accepting.load(Ordering::Acquire),
                    "terminal ingress may close only during ordered manager shutdown"
                );
                TimelineSendTerminalAdmission::ClosedForShutdown
            }
        }
    }

    pub(super) fn close_for_shutdown(
        &self,
        receiver: &mut mpsc::UnboundedReceiver<TimelineSendTerminalHandoff>,
    ) {
        self.stop_accepting();
        receiver.close();
    }

    pub(super) fn stop_accepting(&self) {
        self.accepting.store(false, Ordering::Release);
    }
}

#[derive(Clone)]
pub(super) struct MatrixTimelineSendEnqueueContext {
    pub(super) key: TimelineKey,
    pub(super) timeline: Arc<Timeline>,
    pub(super) session: Arc<MatrixClientSession>,
    pub(super) cleanup: TimelineActorCleanupIngress,
    pub(super) diagnostic_trace: Option<SendLifecycleTrace>,
}

#[derive(Clone)]
pub(super) enum TimelineSendEnqueueContext {
    Matrix(MatrixTimelineSendEnqueueContext),
    #[cfg(test)]
    Synthetic {
        requests: mpsc::UnboundedSender<SyntheticSendEnqueueRequest>,
    },
    #[cfg(test)]
    CleanupProbe {
        cleanup: TimelineActorCleanupIngress,
    },
}

impl TimelineSendEnqueueContext {
    fn set_diagnostic_trace(&mut self, trace: Option<SendLifecycleTrace>) {
        match self {
            Self::Matrix(context) => context.diagnostic_trace = trace,
            #[cfg(test)]
            Self::Synthetic { .. } | Self::CleanupProbe { .. } => {}
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum RoomEncryptionDiagnosticState {
    Encrypted,
    NotEncrypted,
    Unknown,
}

impl RoomEncryptionDiagnosticState {
    pub(super) fn token(self) -> &'static str {
        match self {
            Self::Encrypted => "encrypted",
            Self::NotEncrypted => "not_encrypted",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy)]
enum OwnUserTrackingDiagnosticState {
    Tracked,
    Untracked,
    Unavailable,
}

impl OwnUserTrackingDiagnosticState {
    fn token(self) -> &'static str {
        match self {
            Self::Tracked => "tracked",
            Self::Untracked => "untracked",
            Self::Unavailable => "unavailable",
        }
    }
}

struct EncryptedSendDiagnosticSnapshot {
    room_encryption: RoomEncryptionDiagnosticState,
    outbound_session_present: Option<bool>,
    own_user_tracking: OwnUserTrackingDiagnosticState,
    own_device_present: Option<bool>,
    known_own_device_count: Option<usize>,
    known_own_other_device_count: Option<usize>,
    key_capable_own_other_device_count: Option<usize>,
    cross_signed_own_other_device_count: Option<usize>,
    dehydrated_own_other_device_count: Option<usize>,
    blacklisted_own_other_device_count: Option<usize>,
}

async fn encrypted_send_diagnostic_snapshot(
    context: &MatrixTimelineSendEnqueueContext,
) -> EncryptedSendDiagnosticSnapshot {
    let room_encryption = match context.timeline.room().encryption_state() {
        state if state.is_encrypted() => RoomEncryptionDiagnosticState::Encrypted,
        state if state.is_unknown() => RoomEncryptionDiagnosticState::Unknown,
        _ => RoomEncryptionDiagnosticState::NotEncrypted,
    };
    if !matches!(room_encryption, RoomEncryptionDiagnosticState::Encrypted) {
        return EncryptedSendDiagnosticSnapshot {
            room_encryption,
            outbound_session_present: None,
            own_user_tracking: OwnUserTrackingDiagnosticState::Unavailable,
            own_device_present: None,
            known_own_device_count: None,
            known_own_other_device_count: None,
            key_capable_own_other_device_count: None,
            cross_signed_own_other_device_count: None,
            dehydrated_own_other_device_count: None,
            blacklisted_own_other_device_count: None,
        };
    }
    let outbound_session_present =
        koushi_sdk::current_outbound_group_session_token(&context.session, context.key.room_id())
            .await
            .ok()
            .map(|session| session.is_some());

    let client = context.session.client();
    let Some(own_user_id) = client.user_id().map(ToOwned::to_owned) else {
        return EncryptedSendDiagnosticSnapshot {
            room_encryption,
            outbound_session_present,
            own_user_tracking: OwnUserTrackingDiagnosticState::Unavailable,
            own_device_present: None,
            known_own_device_count: None,
            known_own_other_device_count: None,
            key_capable_own_other_device_count: None,
            cross_signed_own_other_device_count: None,
            dehydrated_own_other_device_count: None,
            blacklisted_own_other_device_count: None,
        };
    };
    let own_device_id = client.device_id().map(ToOwned::to_owned);
    let own_user_tracking = match client.encryption().tracked_users().await {
        Ok(users) if users.contains(&own_user_id) => OwnUserTrackingDiagnosticState::Tracked,
        Ok(_) => OwnUserTrackingDiagnosticState::Untracked,
        Err(_) => OwnUserTrackingDiagnosticState::Unavailable,
    };
    let Ok(devices) = client.encryption().get_user_devices(&own_user_id).await else {
        return EncryptedSendDiagnosticSnapshot {
            room_encryption,
            outbound_session_present,
            own_user_tracking,
            own_device_present: None,
            known_own_device_count: None,
            known_own_other_device_count: None,
            key_capable_own_other_device_count: None,
            cross_signed_own_other_device_count: None,
            dehydrated_own_other_device_count: None,
            blacklisted_own_other_device_count: None,
        };
    };

    let known_own_device_count = devices.devices().count();
    let own_device_present = own_device_id
        .as_deref()
        .map(|own_device_id| devices.get(own_device_id).is_some());
    let mut known_own_other_device_count = 0;
    let mut key_capable_own_other_device_count = 0;
    let mut cross_signed_own_other_device_count = 0;
    let mut dehydrated_own_other_device_count = 0;
    let mut blacklisted_own_other_device_count = 0;
    for device in devices.devices() {
        if own_device_id
            .as_deref()
            .is_some_and(|own_device_id| device.device_id() == own_device_id)
        {
            continue;
        }
        known_own_other_device_count += 1;
        let cross_signed = device.is_cross_signed_by_owner();
        let dehydrated = device.is_dehydrated();
        let blacklisted = device.is_blacklisted();
        if device.curve25519_key().is_some() && !blacklisted {
            key_capable_own_other_device_count += 1;
        }
        if cross_signed {
            cross_signed_own_other_device_count += 1;
        }
        if dehydrated {
            dehydrated_own_other_device_count += 1;
        }
        if blacklisted {
            blacklisted_own_other_device_count += 1;
        }
    }

    EncryptedSendDiagnosticSnapshot {
        room_encryption,
        outbound_session_present,
        own_user_tracking,
        own_device_present,
        known_own_device_count: Some(known_own_device_count),
        known_own_other_device_count: Some(known_own_other_device_count),
        key_capable_own_other_device_count: Some(key_capable_own_other_device_count),
        cross_signed_own_other_device_count: Some(cross_signed_own_other_device_count),
        dehydrated_own_other_device_count: Some(dehydrated_own_other_device_count),
        blacklisted_own_other_device_count: Some(blacklisted_own_other_device_count),
    }
}

pub(super) enum TimelineSendEnqueuePayload {
    Text {
        document: ComposerDocument,
        formatting_options: ComposerFormattingOptions,
    },
    Reply {
        in_reply_to_event_id: String,
        document: ComposerDocument,
        formatting_options: ComposerFormattingOptions,
    },
    Media {
        request_id: RequestId,
        client_transaction_id: String,
        request: UploadMediaRequest,
    },
}

#[cfg(test)]
struct SyntheticSendEnqueueRequest {
    payload: TimelineSendEnqueuePayload,
    response: oneshot::Sender<Result<SendEnqueueSuccess, TimelineFailureKind>>,
}

struct MediaSendQueuedDelivery {
    request_id: RequestId,
    key: TimelineKey,
    transaction_id: String,
}

struct SendEnqueueSuccess {
    sdk_transaction_id: String,
    media_queued: Option<MediaSendQueuedDelivery>,
}

impl SendEnqueueSuccess {
    fn terminal_only(sdk_transaction_id: String) -> Self {
        Self {
            sdk_transaction_id,
            media_queued: None,
        }
    }
}

pub(super) struct SendEnqueueWorkerCompletion;

type SendEnqueueWorkerFuture =
    Pin<Box<dyn Future<Output = SendEnqueueWorkerCompletion> + Send + 'static>>;

type SendDiagnosticFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

pub(super) type GlobalSendCompletionObserverFuture =
    Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

pub(super) const MAX_CONCURRENT_SEND_DIAGNOSTICS: usize = 32;

pub(super) async fn poll_global_send_completion_observer(
    observer: &mut Option<GlobalSendCompletionObserverFuture>,
) {
    match observer.as_mut() {
        Some(observer) => observer.await,
        None => futures_util::future::pending().await,
    }
}

async fn poll_global_send_completion_observer_once(
    observer: &mut Option<GlobalSendCompletionObserverFuture>,
) -> bool {
    futures_util::future::poll_fn(|context| {
        let completed = observer
            .as_mut()
            .is_some_and(|observer| observer.as_mut().poll(context).is_ready());
        Poll::Ready(completed)
    })
    .await
}

pub(super) struct SendEnqueueWorkerSupervisor {
    pub(super) tasks: FuturesUnordered<SendEnqueueWorkerFuture>,
    pub(super) diagnostic_tasks: FuturesUnordered<SendDiagnosticFuture>,
    terminal_ingress: TimelineSendTerminalIngress,
    pub(super) room_key_reshares: HashMap<TimelineKey, RoomKeyReshareSchedule>,
}

impl SendEnqueueWorkerSupervisor {
    pub(super) fn new(terminal_ingress: TimelineSendTerminalIngress) -> Self {
        Self {
            tasks: FuturesUnordered::new(),
            diagnostic_tasks: FuturesUnordered::new(),
            terminal_ingress,
            room_key_reshares: HashMap::new(),
        }
    }

    pub(super) fn cancel_all(&mut self) {
        self.tasks = FuturesUnordered::new();
        self.cancel_diagnostics();
        self.room_key_reshares.clear();
    }

    pub(super) fn cancel_diagnostics(&mut self) {
        self.diagnostic_tasks = FuturesUnordered::new();
    }

    fn spawn_diagnostic<F>(&mut self, correlation: u64, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        if self.diagnostic_tasks.len() >= MAX_CONCURRENT_SEND_DIAGNOSTICS {
            record_send_diagnostic_snapshot_skipped(correlation);
            return;
        }
        self.diagnostic_tasks.push(Box::pin(future));
    }
}

impl Drop for SendEnqueueWorkerSupervisor {
    fn drop(&mut self) {
        // Enqueue futures are polled directly by the manager and must return
        // from each poll like every well-behaved async future. Closing terminal
        // admission before synchronously dropping the set makes every active
        // registration fail closed without a detached Tokio task.
        self.terminal_ingress.stop_accepting();
        self.cancel_all();
    }
}

#[cfg(test)]
pub(super) async fn run_send_enqueue_future<F>(
    mut registration: SendCompletionRegistration,
    event_tx: broadcast::Sender<CoreEvent>,
    enqueue: F,
) -> SendEnqueueWorkerCompletion
where
    F: Future<Output = Result<SendEnqueueSuccess, TimelineFailureKind>>,
{
    match enqueue.await {
        Ok(success) => {
            let SendEnqueueSuccess {
                sdk_transaction_id,
                media_queued,
            } = success;
            if let Some(media) = media_queued {
                let _ = event_tx.send(CoreEvent::Timeline(TimelineEvent::MediaSendQueued {
                    request_id: media.request_id,
                    key: media.key,
                    transaction_id: media.transaction_id,
                }));
            }
            // Binding can synchronously admit an SDK terminal retained before
            // enqueue completed. Publish the media queue acknowledgement first
            // so no terminal can overtake it at the manager ingress boundary.
            registration.bind(sdk_transaction_id);
            SendEnqueueWorkerCompletion
        }
        Err(kind) => {
            registration.fail_known(kind);
            SendEnqueueWorkerCompletion
        }
    }
}

async fn enqueue_document_send(
    context: MatrixTimelineSendEnqueueContext,
    document: ComposerDocument,
    formatting_options: ComposerFormattingOptions,
) -> Result<SendEnqueueSuccess, TimelineFailureKind> {
    let content = build_room_message_content_from_composer_document_with_options(
        document,
        formatting_options,
    )?;
    context
        .timeline
        .send(content.into())
        .await
        .map(|handle| SendEnqueueSuccess::terminal_only(handle.transaction_id().to_string()))
        .map_err(|error| classify_timeline_send_error(&error))
}

async fn enqueue_document_reply_send(
    context: MatrixTimelineSendEnqueueContext,
    in_reply_to_event_id: String,
    document: ComposerDocument,
    formatting_options: ComposerFormattingOptions,
) -> Result<SendEnqueueSuccess, TimelineFailureKind> {
    let reply_event_id = matrix_sdk::ruma::EventId::parse(&in_reply_to_event_id)
        .map_err(|_| TimelineFailureKind::Sdk)?;
    let content = build_room_message_content_without_relation_from_composer_document_with_options(
        document,
        formatting_options,
    )?;
    let reply = Reply {
        event_id: reply_event_id,
        enforce_thread: reply_enforce_thread_for_key(&context.key),
        add_mentions: AddMentions::Yes,
    };
    let content = context
        .timeline
        .room()
        .make_reply_event(content, reply)
        .await
        .map_err(|_| TimelineFailureKind::Sdk)?;
    context
        .timeline
        .send(content.into())
        .await
        .map(|handle| SendEnqueueSuccess::terminal_only(handle.transaction_id().to_string()))
        .map_err(|error| classify_timeline_send_error(&error))
}

async fn enqueue_media_send(
    context: MatrixTimelineSendEnqueueContext,
    request_id: RequestId,
    client_transaction_id: String,
    request: UploadMediaRequest,
) -> Result<SendEnqueueSuccess, TimelineFailureKind> {
    let room_id = matrix_sdk::ruma::RoomId::parse(context.key.room_id())
        .map_err(|_| TimelineFailureKind::Sdk)?;
    let room = context
        .session
        .client()
        .get_room(&room_id)
        .ok_or(TimelineFailureKind::Sdk)?;
    let mime_type = request
        .mime_type
        .parse()
        .map_err(|_| TimelineFailureKind::Sdk)?;
    let caption_mentions = request
        .caption
        .as_ref()
        .and_then(|caption| ruma_mentions_from_intent(&caption.mentions));
    let config = AttachmentConfig::new()
        .txn_id(matrix_sdk::ruma::OwnedTransactionId::from(
            client_transaction_id.clone(),
        ))
        .info(attachment_info_for_upload(&request))
        .thumbnail(thumbnail_for_upload(&request))
        .caption(
            request
                .caption
                .as_ref()
                .map(media_caption_content_from_draft),
        )
        .mentions(caption_mentions)
        .reply(attachment_reply_for_key(&context.key));
    let handle = room
        .send_queue()
        .send_attachment(request.filename, mime_type, request.bytes, config)
        .await
        .map_err(|error| classify_send_queue_error(&error))?;
    Ok(SendEnqueueSuccess {
        sdk_transaction_id: handle.transaction_id().to_string(),
        media_queued: Some(MediaSendQueuedDelivery {
            request_id,
            key: context.key,
            transaction_id: client_transaction_id,
        }),
    })
}

async fn enqueue_timeline_send(
    context: TimelineSendEnqueueContext,
    payload: TimelineSendEnqueuePayload,
) -> Result<SendEnqueueSuccess, TimelineFailureKind> {
    match context {
        TimelineSendEnqueueContext::Matrix(context) => {
            let diagnostic_context = context.clone();
            let diagnostic_trace = context.diagnostic_trace.clone();
            let diagnostic = async move {
                if let Some(trace) = diagnostic_trace {
                    let snapshot = encrypted_send_diagnostic_snapshot(&diagnostic_context).await;
                    trace.record_encryption_local_store_snapshot(&snapshot);
                }
            };
            let enqueue = async move {
                match payload {
                    TimelineSendEnqueuePayload::Text {
                        document,
                        formatting_options,
                    } => enqueue_document_send(context, document, formatting_options).await,
                    TimelineSendEnqueuePayload::Reply {
                        in_reply_to_event_id,
                        document,
                        formatting_options,
                    } => {
                        enqueue_document_reply_send(
                            context,
                            in_reply_to_event_id,
                            document,
                            formatting_options,
                        )
                        .await
                    }
                    TimelineSendEnqueuePayload::Media {
                        request_id,
                        client_transaction_id,
                        request,
                    } => {
                        enqueue_media_send(context, request_id, client_transaction_id, request)
                            .await
                    }
                }
            };
            tokio::pin!(diagnostic);
            tokio::pin!(enqueue);
            tokio::select! {
                biased;
                result = &mut enqueue => result,
                () = &mut diagnostic => enqueue.await,
            }
        }
        #[cfg(test)]
        TimelineSendEnqueueContext::Synthetic { requests } => {
            let (response, outcome) = oneshot::channel();
            requests
                .send(SyntheticSendEnqueueRequest { payload, response })
                .map_err(|_| TimelineFailureKind::QueueOverflow)?;
            outcome
                .await
                .unwrap_or(Err(TimelineFailureKind::QueueOverflow))
        }
        #[cfg(test)]
        TimelineSendEnqueueContext::CleanupProbe { .. } => Err(TimelineFailureKind::QueueOverflow),
    }
}

const MAX_SUBMISSION_TOMBSTONES: usize = 128;

#[derive(Default)]
pub(super) struct SubmissionAdmissionLedger {
    pub(super) active: HashMap<koushi_state::SubmissionId, (TimelineKey, String)>,
    tombstones: std::collections::VecDeque<(koushi_state::SubmissionId, TimelineKey, String)>,
    rejected: std::collections::VecDeque<(koushi_state::SubmissionId, TimelineKey)>,
}

impl SubmissionAdmissionLedger {
    pub(super) fn get(&self, id: &koushi_state::SubmissionId) -> Option<(&TimelineKey, &String)> {
        self.active
            .get(id)
            .map(|(key, txn)| (key, txn))
            .or_else(|| {
                self.tombstones
                    .iter()
                    .find(|(found, _, _)| found == id)
                    .map(|(_, key, txn)| (key, txn))
            })
    }

    pub(super) fn accept(
        &mut self,
        id: koushi_state::SubmissionId,
        key: TimelineKey,
        transaction_id: String,
    ) {
        self.active.insert(id, (key, transaction_id));
    }

    fn rejected(&self, id: &koushi_state::SubmissionId) -> Option<&TimelineKey> {
        self.rejected
            .iter()
            .find(|(found, _)| found == id)
            .map(|(_, key)| key)
    }

    fn reject(&mut self, id: koushi_state::SubmissionId, key: TimelineKey) {
        while self.rejected.len() >= MAX_SUBMISSION_TOMBSTONES {
            self.rejected.pop_front();
        }
        self.rejected.push_back((id, key));
    }

    pub(super) fn terminal(&mut self, id: &koushi_state::SubmissionId) {
        let Some((key, transaction_id)) = self.active.remove(id) else {
            return;
        };
        while self.tombstones.len() >= MAX_SUBMISSION_TOMBSTONES {
            self.tombstones.pop_front();
        }
        self.tombstones.push_back((id.clone(), key, transaction_id));
    }
}

impl TimelineManagerActor {
    #[cfg(test)]
    fn spawn_send_enqueue_future<F>(&mut self, registration: SendCompletionRegistration, enqueue: F)
    where
        F: Future<Output = Result<SendEnqueueSuccess, TimelineFailureKind>> + Send + 'static,
    {
        let event_tx = self.event_tx.clone();
        self.send_enqueue_workers.tasks.push(Box::pin(async move {
            // Spawned workers previously isolated enqueue panics at the JoinHandle boundary.
            // Keep that fail-closed isolation when the manager polls futures directly.
            let _ = AssertUnwindSafe(run_send_enqueue_future(registration, event_tx, enqueue))
                .catch_unwind()
                .await;
            SendEnqueueWorkerCompletion
        }));
    }
    fn spawn_send_enqueue(
        &mut self,
        mut context: TimelineSendEnqueueContext,
        mut registration: SendCompletionRegistration,
        admission: Option<oneshot::Receiver<()>>,
        payload: TimelineSendEnqueuePayload,
    ) -> oneshot::Receiver<()> {
        let (preflight_started_tx, preflight_started_rx) = oneshot::channel();
        let account_work = self.account_work.clone();
        let event_tx = self.event_tx.clone();
        self.send_enqueue_workers.tasks.push(Box::pin(async move {
            let worker = async move {
                let outcome = async {
                    if !await_submission_admission(admission).await {
                        return Err(TimelineFailureKind::QueueOverflow);
                    }
                    if let Some(trace) = registration.lifecycle_trace.as_mut() {
                        trace.stage("preflight_started");
                    }
                    let _ = preflight_started_tx.send(());
                    // Interactive: the guard never queues, so admission and the local
                    // echo stay immediate. Keep it attached to the send completion
                    // registration so background history work yields until the SDK
                    // terminal settles the send.
                    let interactive = account_work.begin_interactive(AccountWorkKind::MessageSend);
                    registration.hold_interactive_guard(interactive);
                    if let Some(trace) = registration.lifecycle_trace.as_mut() {
                        trace.stage("send_queue_worker_started");
                    }
                    if let Some(trace) = registration.lifecycle_trace.as_mut() {
                        trace.stage("sdk_enqueue_started");
                    }
                    context.set_diagnostic_trace(registration.lifecycle_trace.as_ref().cloned());
                    enqueue_timeline_send(context, payload).await
                }
                .await;
                match outcome {
                    Ok(success) => {
                        let SendEnqueueSuccess {
                            sdk_transaction_id,
                            media_queued,
                        } = success;
                        if let Some(media) = media_queued {
                            if let Some(trace) = registration.lifecycle_trace.as_mut() {
                                trace.stage("media_upload_queued");
                            }
                            let _ = event_tx.send(CoreEvent::Timeline(
                                TimelineEvent::MediaSendQueued {
                                    request_id: media.request_id,
                                    key: media.key,
                                    transaction_id: media.transaction_id,
                                },
                            ));
                        }
                        registration.bind(sdk_transaction_id);
                    }
                    Err(kind) => {
                        registration.fail_known(kind);
                    }
                }
            };
            let _ = AssertUnwindSafe(worker).catch_unwind().await;
            SendEnqueueWorkerCompletion
        }));
        preflight_started_rx
    }
    pub(super) fn handle_send_enqueue_worker_completion(&self, _: SendEnqueueWorkerCompletion) {}
    async fn drive_send_enqueue_until_preflight_started(
        &mut self,
        mut preflight_started: oneshot::Receiver<()>,
    ) {
        loop {
            tokio::select! {
                biased;
                _ = &mut preflight_started => break,
                worker = self.send_enqueue_workers.tasks.next(),
                    if !self.send_enqueue_workers.tasks.is_empty() => {
                    match worker {
                        Some(completion) => {
                            self.handle_send_enqueue_worker_completion(completion);
                        }
                        None => break,
                    }
                }
            }
        }
    }
    async fn drain_send_enqueue_workers_until(&mut self, deadline: executor::Instant) -> bool {
        enum DrainProgress {
            Worker(Option<SendEnqueueWorkerCompletion>),
            ObserverFinished,
        }

        while !self.send_enqueue_workers.tasks.is_empty() {
            let progress = executor::timeout_at(deadline, async {
                tokio::select! {
                    worker = self.send_enqueue_workers.tasks.next() => {
                        DrainProgress::Worker(worker)
                    }
                    _ = poll_global_send_completion_observer(
                        &mut self.global_send_completion_observer_future,
                    ) => DrainProgress::ObserverFinished,
                }
            })
            .await;
            match progress {
                Ok(DrainProgress::Worker(Some(completion))) => {
                    self.handle_send_enqueue_worker_completion(completion);
                }
                Ok(DrainProgress::Worker(None)) => break,
                Ok(DrainProgress::ObserverFinished) => {
                    self.global_send_completion_observer_future = None;
                }
                Err(_) => return false,
            }
        }
        if poll_global_send_completion_observer_once(
            &mut self.global_send_completion_observer_future,
        )
        .await
        {
            self.global_send_completion_observer_future = None;
        }
        true
    }
    pub(super) async fn join_send_enqueue_workers(&mut self) {
        self.join_send_enqueue_workers_with_grace_period(SEND_ENQUEUE_WORKER_SHUTDOWN_DEADLINE)
            .await;
    }
    async fn join_send_enqueue_workers_with_grace_period(&mut self, grace_period: Duration) {
        let graceful_deadline = executor::Instant::now() + grace_period;
        if self
            .drain_send_enqueue_workers_until(graceful_deadline)
            .await
        {
            return;
        }

        // Manager-owned futures are cancellation-safe at poll boundaries. Dropping the set
        // synchronously settles every registration while the terminal observer remains live.
        self.send_enqueue_workers.cancel_all();
        if poll_global_send_completion_observer_once(
            &mut self.global_send_completion_observer_future,
        )
        .await
        {
            self.global_send_completion_observer_future = None;
        }
    }
    pub(super) async fn handle_send_terminal_handoff(
        &mut self,
        handoff: TimelineSendTerminalHandoff,
    ) {
        let TimelineSendTerminalHandoff {
            submission_id,
            action,
            completion,
            failure,
        } = handoff;
        if let Some(action) = action
            && !deliver_submission_terminal_action(&self.action_tx, action).await
        {
            // A required reducer action that cannot be enqueued fails closed:
            // neither the admission ledger nor CoreEvent may claim settlement.
            if let Some(failure) = failure {
                self.emit(CoreEvent::OperationFailed {
                    request_id: failure.request_id,
                    failure: failure.failure,
                });
            }
            return;
        }
        if let Some(submission_id) = submission_id {
            self.accepted_submissions.terminal(&submission_id);
        }
        if let Some(completion) = completion {
            let key = completion.key.clone();
            let diagnostic_correlation = completion.diagnostic_correlation;
            self.emit(CoreEvent::Timeline(TimelineEvent::SendCompleted {
                request_id: completion.request_id,
                key: completion.key,
                transaction_id: completion.transaction_id,
                event_id: completion.event_id,
            }));
            self.spawn_post_send_encryption_diagnostics(&key, diagnostic_correlation);
            self.schedule_room_key_reshares(&key).await;
        }
        if let Some(failure) = failure {
            self.emit(CoreEvent::OperationFailed {
                request_id: failure.request_id,
                failure: failure.failure,
            });
        }
    }
    fn spawn_post_send_encryption_diagnostics(
        &mut self,
        key: &TimelineKey,
        diagnostic_correlation: Option<u64>,
    ) {
        let Some(correlation) = diagnostic_correlation else {
            return;
        };
        let Some(session) = self.session.as_ref().cloned() else {
            return;
        };
        let room_id = key.room_id().to_owned();
        self.send_enqueue_workers
            .spawn_diagnostic(correlation, async move {
                let client = session.client();
                let room_encryption = matrix_sdk::ruma::RoomId::parse(&room_id)
                    .ok()
                    .and_then(|room_id| client.get_room(&room_id))
                    .map(|room| match room.encryption_state() {
                        state if state.is_encrypted() => RoomEncryptionDiagnosticState::Encrypted,
                        state if state.is_unknown() => RoomEncryptionDiagnosticState::Unknown,
                        _ => RoomEncryptionDiagnosticState::NotEncrypted,
                    })
                    .unwrap_or(RoomEncryptionDiagnosticState::Unknown);
                let lookup =
                    if matches!(room_encryption, RoomEncryptionDiagnosticState::NotEncrypted) {
                        OutboundSessionLookupDiagnostic::NotApplicable
                    } else {
                        match koushi_sdk::current_outbound_group_session_token(&session, &room_id)
                            .await
                        {
                            Ok(Some(_)) => OutboundSessionLookupDiagnostic::Present,
                            Ok(None) => OutboundSessionLookupDiagnostic::Absent,
                            Err(error)
                                if error.failure_kind()
                                    == Some(koushi_sdk::MatrixRoomOperationFailureKind::Http) =>
                            {
                                OutboundSessionLookupDiagnostic::NetworkError
                            }
                            Err(_) => OutboundSessionLookupDiagnostic::SdkError,
                        }
                    };
                record_post_send_encryption_snapshot(correlation, room_encryption, lookup);
            });
    }
    pub(super) async fn route_send_to_worker_or_fail(
        &mut self,
        request_id: RequestId,
        key: &TimelineKey,
        transaction_id: String,
        body: String,
        projection: SendComposerProjection,
        payload: TimelineSendEnqueuePayload,
    ) {
        let Some(context) = self
            .timelines
            .get(key)
            .and_then(|handle| handle.enqueue_context.clone())
        else {
            self.emit_failure(
                request_id,
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::NotSubscribed,
                },
            );
            return;
        };

        if let Some(action) = send_submitted_action(key, projection, transaction_id.clone(), body) {
            if self.action_tx.send(vec![action]).await.is_err() {
                self.emit_failure(
                    request_id,
                    CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::QueueOverflow,
                    },
                );
                return;
            }
        }
        let mut registration = SendCompletionRegistration::begin(
            Arc::clone(&self.send_completion),
            self.terminal_ingress.clone(),
            key.clone(),
            transaction_id,
            None,
            request_id,
            true,
        );
        registration.activate();
        let preflight_started = self.spawn_send_enqueue(context, registration, None, payload);
        // Directly-owned futures are not independently scheduled Tokio tasks. Drive this
        // admitted worker through its permit to the start of payload-specific preflight before
        // returning to the command loop. This does not serialize later SDK queue insertion.
        self.drive_send_enqueue_until_preflight_started(preflight_started)
            .await;
    }
    pub(super) async fn route_media_send_to_worker_or_fail(
        &mut self,
        request_id: RequestId,
        key: &TimelineKey,
        transaction_id: String,
        payload: TimelineSendEnqueuePayload,
    ) {
        let Some(context) = self
            .timelines
            .get(key)
            .and_then(|handle| handle.enqueue_context.clone())
        else {
            self.emit_failure(
                request_id,
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::NotSubscribed,
                },
            );
            return;
        };
        let mut registration = SendCompletionRegistration::begin(
            Arc::clone(&self.send_completion),
            self.terminal_ingress.clone(),
            key.clone(),
            transaction_id,
            None,
            request_id,
            false,
        );
        registration.activate();
        let preflight_started = self.spawn_send_enqueue(context, registration, None, payload);
        self.drive_send_enqueue_until_preflight_started(preflight_started)
            .await;
    }
    pub(super) async fn route_submission_to_worker(
        &mut self,
        request_id: RequestId,
        submission_id: koushi_state::SubmissionId,
        key: &TimelineKey,
        transaction_id: String,
        body: String,
        draft_revision: koushi_state::ComposerDraftRevision,
        projection: SendComposerProjection,
        payload: TimelineSendEnqueuePayload,
        mut composer_permit: Option<ForwardedComposerDraftPermit>,
    ) {
        if let Some(rejected_key) = self.accepted_submissions.rejected(&submission_id) {
            self.emit(CoreEvent::Timeline(TimelineEvent::SubmissionRejected {
                request_id,
                key: rejected_key.clone(),
                submission_id,
                kind: TimelineFailureKind::QueueOverflow,
            }));
            return;
        }
        if let Some((accepted_key, accepted_transaction_id)) =
            self.accepted_submissions.get(&submission_id)
        {
            self.emit(CoreEvent::Timeline(TimelineEvent::SubmissionAccepted {
                request_id,
                key: accepted_key.clone(),
                submission_id,
                transaction_id: accepted_transaction_id.clone(),
            }));
            return;
        }
        let Some(context) = self
            .timelines
            .get(key)
            .and_then(|handle| handle.enqueue_context.clone())
        else {
            self.emit(CoreEvent::Timeline(TimelineEvent::SubmissionRejected {
                request_id,
                key: key.clone(),
                submission_id,
                kind: TimelineFailureKind::NotSubscribed,
            }));
            return;
        };
        let (permit_tx, permit_rx) = oneshot::channel();
        let registration = SendCompletionRegistration::begin(
            Arc::clone(&self.send_completion),
            self.terminal_ingress.clone(),
            key.clone(),
            transaction_id.clone(),
            Some(submission_id.clone()),
            request_id,
            true,
        );
        let registration_id = registration
            .registration_id()
            .expect("new send registration must own its id");
        // The stable manager owns the permit-blocked worker before it exposes
        // acceptance. Unsubscribe may now remove only presentation state.
        let preflight_started =
            self.spawn_send_enqueue(context, registration, Some(permit_rx), payload);
        if !self
            .send_completion
            .lock()
            .expect("send completion coordinator lock must not be poisoned")
            .activate_registration(registration_id)
        {
            self.emit(CoreEvent::Timeline(TimelineEvent::SubmissionRejected {
                request_id,
                key: key.clone(),
                submission_id,
                kind: TimelineFailureKind::QueueOverflow,
            }));
            return;
        }
        let action = match (projection, &key.kind) {
            (SendComposerProjection::Room, TimelineKind::Room { room_id }) => {
                Some(AppAction::ComposerSubmissionAcceptedAtRevision {
                    submission_id: submission_id.clone(),
                    room_id: room_id.clone(),
                    transaction_id: transaction_id.clone(),
                    body,
                    draft_revision,
                })
            }
            (
                SendComposerProjection::ThreadReply,
                TimelineKind::Thread {
                    room_id,
                    root_event_id,
                },
            ) => Some(AppAction::ThreadSubmissionAcceptedAtRevision {
                submission_id: submission_id.clone(),
                room_id: room_id.clone(),
                root_event_id: root_event_id.clone(),
                transaction_id: transaction_id.clone(),
                body,
                draft_revision,
            }),
            _ => send_submitted_action(key, projection, transaction_id.clone(), body),
        };
        if let Some(action) = action {
            if let Some(composer_permit) = composer_permit.as_mut() {
                composer_permit.acceptance_projection_reached();
            }
            if self.action_tx.send(vec![action]).await.is_err() {
                self.send_completion
                    .lock()
                    .expect("send completion coordinator lock must not be poisoned")
                    .cancel_registration(registration_id);
                self.accepted_submissions
                    .reject(submission_id.clone(), key.clone());
                self.emit(CoreEvent::Timeline(TimelineEvent::SubmissionRejected {
                    request_id,
                    key: key.clone(),
                    submission_id,
                    kind: TimelineFailureKind::QueueOverflow,
                }));
                return;
            }
            if let Some(composer_permit) = composer_permit.take() {
                composer_permit.acceptance_enqueued();
            }
        }
        self.accepted_submissions.accept(
            submission_id.clone(),
            key.clone(),
            transaction_id.clone(),
        );
        self.emit(CoreEvent::Timeline(TimelineEvent::SubmissionAccepted {
            request_id,
            key: key.clone(),
            submission_id,
            transaction_id,
        }));
        let _ = permit_tx.send(());
        self.drive_send_enqueue_until_preflight_started(preflight_started)
            .await;
    }
}

#[derive(Clone, Copy)]
pub(super) enum SendComposerProjection {
    Room,
    ThreadReply,
    None,
}

impl SendComposerProjection {
    pub(super) fn for_send_text(key: &TimelineKey) -> Self {
        match key.kind {
            TimelineKind::Room { .. } => Self::Room,
            TimelineKind::Thread { .. } | TimelineKind::Focused { .. } => Self::None,
        }
    }

    pub(super) fn for_send_reply(key: &TimelineKey) -> Self {
        match key.kind {
            TimelineKind::Room { .. } => Self::Room,
            TimelineKind::Thread { .. } => Self::ThreadReply,
            TimelineKind::Focused { .. } => Self::None,
        }
    }
}

fn send_submitted_action(
    key: &TimelineKey,
    projection: SendComposerProjection,
    transaction_id: String,
    body: String,
) -> Option<AppAction> {
    match (projection, &key.kind) {
        (SendComposerProjection::Room, TimelineKind::Room { room_id }) => {
            Some(AppAction::SendTextSubmitted {
                room_id: room_id.clone(),
                transaction_id,
                body,
            })
        }
        (
            SendComposerProjection::ThreadReply,
            TimelineKind::Thread {
                room_id,
                root_event_id,
            },
        ) => Some(AppAction::ThreadReplySubmitted {
            room_id: room_id.clone(),
            root_event_id: root_event_id.clone(),
            transaction_id,
            body,
        }),
        _ => None,
    }
}

fn send_finished_action(key: &TimelineKey, transaction_id: String) -> Option<AppAction> {
    match &key.kind {
        TimelineKind::Room { room_id } => Some(AppAction::SendTextFinished {
            room_id: room_id.clone(),
            transaction_id,
        }),
        TimelineKind::Thread {
            room_id,
            root_event_id,
        } => Some(AppAction::ThreadReplyFinished {
            room_id: room_id.clone(),
            root_event_id: root_event_id.clone(),
            transaction_id,
        }),
        TimelineKind::Focused { .. } => None,
    }
}

fn submission_target(key: &TimelineKey) -> Option<koushi_state::ComposerSubmissionTarget> {
    match &key.kind {
        TimelineKind::Room { room_id } => Some(koushi_state::ComposerSubmissionTarget::Main {
            room_id: room_id.clone(),
        }),
        TimelineKind::Thread {
            room_id,
            root_event_id,
        } => Some(koushi_state::ComposerSubmissionTarget::Thread {
            room_id: room_id.clone(),
            root_event_id: root_event_id.clone(),
        }),
        TimelineKind::Focused { .. } => None,
    }
}

fn send_failed_action(
    key: &TimelineKey,
    projection: SendComposerProjection,
    transaction_id: String,
    message: String,
) -> Option<AppAction> {
    match (projection, &key.kind) {
        (SendComposerProjection::Room, TimelineKind::Room { room_id }) => {
            Some(AppAction::SendTextFailed {
                room_id: room_id.clone(),
                transaction_id,
                message,
            })
        }
        (
            SendComposerProjection::ThreadReply,
            TimelineKind::Thread {
                room_id,
                root_event_id,
            },
        ) => Some(AppAction::ThreadReplyFailed {
            room_id: room_id.clone(),
            root_event_id: root_event_id.clone(),
            transaction_id,
            message,
        }),
        _ => None,
    }
}

pub(super) fn thread_attention_action(
    counts: ThreadAttentionCounters,
    key: &TimelineKey,
) -> Option<AppAction> {
    let TimelineKind::Thread {
        room_id,
        root_event_id,
    } = &key.kind
    else {
        return None;
    };

    Some(AppAction::ThreadAttentionUpdated {
        room_id: room_id.clone(),
        root_event_id: root_event_id.clone(),
        notification_count: counts.notification_count,
        highlight_count: counts.highlight_count,
        live_event_marker_count: counts.live_event_marker_count,
    })
}

pub(super) fn matching_remote_thread_reply_event_id<'a>(
    item: &'a TimelineItem,
    root_event_id: &str,
    own_user_id: Option<&str>,
) -> Option<&'a str> {
    if !is_attention_eligible_event(item) {
        return None;
    }
    let event_id = matching_thread_reply_event_id(item, root_event_id)?;
    if let (Some(sender), Some(own_user_id)) = (item.sender.as_deref(), own_user_id) {
        if sender == own_user_id {
            return None;
        }
    }
    Some(event_id)
}

pub(super) fn matching_thread_reply_event_id<'a>(
    item: &'a TimelineItem,
    root_event_id: &str,
) -> Option<&'a str> {
    let TimelineItemId::Event { event_id } = &item.id else {
        return None;
    };
    if item.thread_root.as_deref() != Some(root_event_id) {
        return None;
    }
    Some(event_id)
}

pub(super) fn thread_activity_observed_action(
    key: &TimelineKey,
    items: &[TimelineItem],
) -> Option<AppAction> {
    let TimelineKind::Thread {
        room_id,
        root_event_id,
    } = &key.kind
    else {
        return None;
    };
    items
        .iter()
        .any(|item| matching_thread_reply_event_id(item, root_event_id).is_some())
        .then(|| AppAction::ThreadActivityObserved {
            room_id: room_id.clone(),
            root_event_id: root_event_id.clone(),
        })
}

pub(super) fn thread_activity_observed_action_for_batch(
    key: &TimelineKey,
    items: &[TimelineItem],
    provenance: &ThreadAttentionBatchProvenance,
) -> Option<AppAction> {
    let TimelineKind::Thread {
        room_id,
        root_event_id,
    } = &key.kind
    else {
        return None;
    };
    items
        .iter()
        .filter_map(|item| matching_thread_reply_event_id(item, root_event_id))
        .any(|event_id| provenance.observation_for(event_id).is_some())
        .then(|| AppAction::ThreadActivityObserved {
            room_id: room_id.clone(),
            root_event_id: root_event_id.clone(),
        })
}

pub(super) fn newest_provable_receipt_event_id(
    items: &[TimelineItem],
    requested_event_id: &str,
    queried_event_id: Option<String>,
    current_event_id: Option<&str>,
) -> String {
    let positions = items
        .iter()
        .enumerate()
        .filter_map(|(position, item)| match &item.id {
            TimelineItemId::Event { event_id } => Some((event_id.as_str(), position)),
            TimelineItemId::Transaction { .. } | TimelineItemId::Synthetic { .. } => None,
        })
        .collect::<HashMap<_, _>>();
    let mut candidates = vec![requested_event_id.to_owned()];
    if let Some(queried_event_id) = queried_event_id {
        if !candidates.contains(&queried_event_id) {
            candidates.push(queried_event_id);
        }
    }
    if let Some(current_event_id) = current_event_id {
        if !candidates
            .iter()
            .any(|candidate| candidate == current_event_id)
        {
            candidates.push(current_event_id.to_owned());
        }
    }

    let newest_visible = candidates
        .iter()
        .filter(|candidate| positions.contains_key(candidate.as_str()))
        .max_by_key(|candidate| positions[candidate.as_str()])
        .cloned();
    if positions.contains_key(requested_event_id) {
        return newest_visible.unwrap_or_else(|| requested_event_id.to_owned());
    }
    if let Some(newest_visible) = newest_visible {
        return newest_visible;
    }

    current_event_id
        .map(str::to_owned)
        .or_else(|| candidates.get(1).cloned())
        .unwrap_or_else(|| requested_event_id.to_owned())
}

async fn await_submission_admission(admission: Option<oneshot::Receiver<()>>) -> bool {
    match admission {
        Some(permit) => permit.await.is_ok(),
        None => true,
    }
}

/// Composer terminals belong to the manager-owned submission ledger, not to
/// one replaceable timeline actor. The manager waits for reducer capacity and
/// only then tombstones the submission.
pub(super) async fn deliver_submission_terminal_action(
    action_tx: &mpsc::Sender<Vec<AppAction>>,
    action: AppAction,
) -> bool {
    emit_app_action_reliable(action_tx, action).await
}

impl TimelineActor {
    pub(super) async fn handle_retry_send(
        &mut self,
        request_id: RequestId,
        transaction_id: String,
    ) {
        if let Err(kind) = validate_retry_send(self.send_statuses.get(&transaction_id)) {
            self.emit_timeline_failure(request_id, kind);
            return;
        }

        let Some(handle) = self.send_handles.get(&transaction_id).cloned() else {
            self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendTarget);
            return;
        };

        let Some(room) = self.sdk_room_for_key() else {
            self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendTarget);
            return;
        };
        room.send_queue().set_enabled(true);

        match handle.unwedge().await {
            Ok(()) => {
                self.send_statuses
                    .insert(transaction_id, TimelineSendState::Sending);
            }
            Err(err) => {
                self.emit_timeline_failure(request_id, classify_send_queue_error(&err));
            }
        }
    }
    pub(super) async fn handle_cancel_send(
        &mut self,
        request_id: RequestId,
        transaction_id: String,
    ) {
        if let Err(kind) = validate_cancel_send(self.send_statuses.get(&transaction_id)) {
            self.emit_timeline_failure(request_id, kind);
            return;
        }

        let Some(handle) = self.send_handles.get(&transaction_id).cloned() else {
            self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendTarget);
            return;
        };

        match handle.abort().await {
            Ok(true) => {
                self.send_statuses
                    .insert(transaction_id.clone(), TimelineSendState::Cancelled);
                self.send_handles.remove(&transaction_id);
                if let Some(room) = self.sdk_room_for_key() {
                    room.send_queue().set_enabled(true);
                }
                apply_send_completion_observation_and_handoff(
                    &self.send_completion,
                    &self.terminal_ingress,
                    self.key.room_id(),
                    SendCompletionObservation::Cancelled {
                        sdk_transaction_id: transaction_id,
                    },
                );
            }
            Ok(false) => {
                self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendState);
            }
            Err(_) => {
                self.emit_timeline_failure(request_id, TimelineFailureKind::Sdk);
            }
        }
    }
    pub(super) async fn handle_send_queue_update(&mut self, update: RoomSendQueueUpdate) {
        match update {
            RoomSendQueueUpdate::NewLocalEvent(echo) => {
                let sdk_transaction_id = echo.transaction_id.to_string();
                self.send_completion
                    .lock()
                    .expect("send completion coordinator lock must not be poisoned")
                    .stage_pending_send(
                        self.key.room_id(),
                        &sdk_transaction_id,
                        "local_echo_observed",
                    );
                remember_local_echo(&mut self.send_statuses, &mut self.send_handles, &echo);
            }
            RoomSendQueueUpdate::CancelledLocalEvent { transaction_id } => {
                let sdk_txn_str = transaction_id.to_string();
                self.send_statuses
                    .insert(sdk_txn_str.clone(), TimelineSendState::Cancelled);
                self.send_handles.remove(&sdk_txn_str);
            }
            RoomSendQueueUpdate::ReplacedLocalEvent { transaction_id, .. } => {
                self.send_statuses
                    .insert(transaction_id.to_string(), TimelineSendState::Sending);
            }
            RoomSendQueueUpdate::SendError {
                transaction_id,
                is_recoverable,
                ..
            } => {
                let sdk_txn_str = transaction_id.to_string();
                self.send_statuses.insert(
                    sdk_txn_str.clone(),
                    TimelineSendState::NotSent {
                        reason: send_failure_reason(is_recoverable),
                    },
                );
            }
            RoomSendQueueUpdate::RetryEvent { transaction_id } => {
                let sdk_transaction_id = transaction_id.to_string();
                self.send_completion
                    .lock()
                    .expect("send completion coordinator lock must not be poisoned")
                    .stage_pending_send(self.key.room_id(), &sdk_transaction_id, "retry_scheduled");
                self.send_statuses
                    .insert(sdk_transaction_id, TimelineSendState::Sending);
            }
            RoomSendQueueUpdate::SentEvent {
                transaction_id,
                event_id,
            } => {
                // Presentation-only mirror: manager-global correlation owns the
                // request/client transaction terminal.
                let sdk_txn_str = transaction_id.to_string();
                self.send_statuses
                    .insert(sdk_txn_str.clone(), TimelineSendState::Sent);
                self.send_handles.remove(&sdk_txn_str);
                self.sent_event_txns
                    .insert(event_id.to_string(), transaction_id.clone());
            }
            RoomSendQueueUpdate::MediaUpload {
                related_to,
                file,
                index,
                progress,
            } => {
                let sdk_txn_str = related_to.to_string();
                self.send_statuses
                    .insert(sdk_txn_str.clone(), TimelineSendState::Sending);
                let (transaction_id, request_id) =
                    media_upload_progress_identity(&self.send_completion, &self.key, &sdk_txn_str);

                self.emit(CoreEvent::Timeline(TimelineEvent::MediaUploadProgress {
                    request_id,
                    key: self.key.clone(),
                    transaction_id,
                    index,
                    progress: MediaTransferProgress {
                        current: u64::try_from(progress.current).unwrap_or(u64::MAX),
                        total: u64::try_from(progress.total).unwrap_or(u64::MAX),
                    },
                    source: file.as_ref().map(timeline_media_source_from_sdk),
                }));
            }
        }
    }
    pub(super) async fn handle_send_queue_lagged(&mut self) {
        self.resync_send_queue_statuses().await;

        let (current_items, _) = self.timeline.subscribe().await;
        let link_preview_context = self.link_preview_policy.for_room(self.key.room_id());
        let items: Vec<TimelineItem> = current_items
            .iter()
            .map(|item| {
                sdk_item_to_timeline_item_with_send_states(
                    &self.key,
                    item,
                    self.own_user_id.as_deref(),
                    &self.send_statuses,
                    Some(&self.room_key_recovery),
                    Some(&self.key_request_states),
                    Some(&self.withheld_codes),
                )
            })
            .map(|mut item| {
                apply_ignored_sender_suppression(&mut item, &self.ignored_user_ids);
                item
            })
            .collect();
        let mut items = items;
        for item in &mut items {
            apply_link_previews_to_item(
                &mut *item,
                self.key.room_id(),
                &link_preview_context,
                &self.session,
            )
            .await;
        }
        trace_timeline_items("send_queue_lagged_initial", &self.key, &items);
        let candidate_display_projection =
            DisplayProjectionState::from_canonical_window(&items, 0..items.len());
        let replay_known_candidates = replay_known_candidates_for_display_items(
            &self.key,
            &items,
            candidate_display_projection.display_items(),
        );
        let _ = commit_prepared_initial_window_for_generation(
            &mut self.navigation_items,
            &mut self.display_projection,
            &self.event_tx,
            &self.replay_known_thread_root_projections,
            &self.thread_root_projection_service,
            &self.timeline_actor_generations,
            &self.key,
            self.actor_generation,
            InitialItemsRequestIdentity::recovery(),
            self.generation,
            Vec::new(),
            PreparedInitialWindow {
                display_projection: candidate_display_projection,
                navigation_items: Some(items.clone()),
                emitted_items: items,
                replay_known_candidates,
            },
        );
    }
    async fn resync_send_queue_statuses(&mut self) {
        let Some(room_id) = timeline_room_id(&self.key) else {
            return;
        };
        let Ok(room_id) = matrix_sdk::ruma::RoomId::parse(room_id) else {
            return;
        };
        let Some(room) = self.session.client().get_room(&room_id) else {
            return;
        };
        let Ok((local_echoes, _update_rx)) = room.send_queue().subscribe().await else {
            return;
        };

        self.send_statuses.clear();
        self.send_handles.clear();
        for echo in &local_echoes {
            remember_local_echo(&mut self.send_statuses, &mut self.send_handles, echo);
        }
    }
}

pub(super) async fn run_global_send_completion_observer(
    mut update_rx: broadcast::Receiver<SendQueueUpdate>,
    coordinator: SharedSendCompletionCoordinator,
    terminal_ingress: TimelineSendTerminalIngress,
) {
    loop {
        match update_rx.recv().await {
            Ok(SendQueueUpdate { room_id, update }) => {
                let observation = match update {
                    RoomSendQueueUpdate::SentEvent {
                        transaction_id,
                        event_id,
                    } => Some(SendCompletionObservation::Sent {
                        sdk_transaction_id: transaction_id.to_string(),
                        event_id: event_id.to_string(),
                    }),
                    RoomSendQueueUpdate::SendError {
                        transaction_id,
                        error,
                        is_recoverable,
                    } => Some(SendCompletionObservation::SendError {
                        sdk_transaction_id: transaction_id.to_string(),
                        diagnostic: classify_send_failure(error.as_ref(), is_recoverable),
                    }),
                    RoomSendQueueUpdate::CancelledLocalEvent { transaction_id } => {
                        Some(SendCompletionObservation::Cancelled {
                            sdk_transaction_id: transaction_id.to_string(),
                        })
                    }
                    RoomSendQueueUpdate::NewLocalEvent(_)
                    | RoomSendQueueUpdate::ReplacedLocalEvent { .. }
                    | RoomSendQueueUpdate::RetryEvent { .. }
                    | RoomSendQueueUpdate::MediaUpload { .. } => None,
                };
                if let Some(observation) = observation {
                    apply_send_completion_observation_and_handoff(
                        &coordinator,
                        &terminal_ingress,
                        room_id.as_str(),
                        observation,
                    );
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {
                // A global terminal broadcast gap is explicit observation loss,
                // never a guessed SDK SendError. Fail every active request once
                // with the private-safe queue-overflow contract while retaining
                // bound correlation for a later exact terminal.
                apply_send_completion_observation_loss_and_handoff(
                    &coordinator,
                    &terminal_ingress,
                    None,
                );
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

pub(super) async fn run_send_queue_monitor(
    actor_tx: mpsc::Sender<TimelineActorMessage>,
    mut update_rx: tokio::sync::broadcast::Receiver<RoomSendQueueUpdate>,
) {
    loop {
        match update_rx.recv().await {
            Ok(update) => {
                if actor_tx
                    .send(TimelineActorMessage::SendQueueUpdate(update))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                if actor_tx
                    .send(TimelineActorMessage::SendQueueLagged)
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                break;
            }
        }
    }
}

fn classify_timeline_send_error(err: &matrix_sdk_ui::timeline::Error) -> TimelineFailureKind {
    match err {
        matrix_sdk_ui::timeline::Error::SendQueueError(send_queue_error) => {
            classify_send_queue_error(send_queue_error)
        }
        _ => TimelineFailureKind::Sdk,
    }
}

fn classify_send_queue_error(
    err: &matrix_sdk::send_queue::RoomSendQueueError,
) -> TimelineFailureKind {
    use matrix_sdk::send_queue::RoomSendQueueError;
    match err {
        RoomSendQueueError::RoomNotJoined => TimelineFailureKind::Forbidden,
        RoomSendQueueError::RoomDisappeared => TimelineFailureKind::Sdk,
        RoomSendQueueError::StorageError(_) => TimelineFailureKind::Sdk,
        _ => TimelineFailureKind::Sdk,
    }
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct SendCorrelationKey {
    room_id: String,
    sdk_transaction_id: String,
}

pub(super) type SharedSendCompletionCoordinator = Arc<Mutex<SendCompletionCoordinator>>;

#[derive(Default)]
pub(super) struct SendCompletionCoordinator {
    next_registration_id: u64,
    registrations: std::collections::BTreeMap<u64, CoordinatedPendingSend>,
    pending_sends: HashMap<SendCorrelationKey, CoordinatedPendingSend>,
    unmatched_terminals: HashMap<SendCorrelationKey, VecDeque<ObservedSendTerminal>>,
    settled_send_tombstones: HashSet<SendCorrelationKey>,
    settled_send_order: VecDeque<SendCorrelationKey>,
}

static NEXT_SEND_DIAGNOSTIC_CORRELATION: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub(super) struct SendLifecycleTrace {
    state: Arc<Mutex<SendLifecycleTraceState>>,
}

impl SendLifecycleTrace {
    fn new(key: &TimelineKey, settles_composer: bool) -> Self {
        let now = Instant::now();
        Self {
            state: Arc::new(Mutex::new(SendLifecycleTraceState {
                correlation: NEXT_SEND_DIAGNOSTIC_CORRELATION.fetch_add(1, Ordering::Relaxed),
                kind: if !settles_composer {
                    "media"
                } else {
                    match key.kind {
                        TimelineKind::Thread { .. } => "thread",
                        TimelineKind::Room { .. } | TimelineKind::Focused { .. } => "text",
                    }
                },
                submitted_at: now,
                previous_stage_at: now,
                recorded_once: HashSet::new(),
            })),
        }
    }

    fn correlation(&self) -> u64 {
        self.state
            .lock()
            .map(|state| state.correlation)
            .unwrap_or_else(|poisoned| poisoned.into_inner().correlation)
    }

    fn stage(&mut self, stage: &'static str) {
        self.stage_internal(stage, None, None, None, false);
    }

    fn stage_once(&mut self, stage: &'static str) {
        self.stage_internal(stage, None, None, None, true);
    }

    fn stage_with_outcome(
        &mut self,
        stage: &'static str,
        outcome: Option<&'static str>,
        delivery_mode: Option<&'static str>,
    ) {
        self.stage_internal(stage, outcome, delivery_mode, None, false);
    }

    fn stage_with_outcome_once(
        &mut self,
        stage: &'static str,
        outcome: Option<&'static str>,
        delivery_mode: Option<&'static str>,
    ) {
        self.stage_internal(stage, outcome, delivery_mode, None, true);
    }

    fn stage_with_failure(
        &mut self,
        stage: &'static str,
        outcome: Option<&'static str>,
        delivery_mode: Option<&'static str>,
        failure: SendFailureDiagnostic,
    ) {
        self.stage_internal(stage, outcome, delivery_mode, Some(failure), false);
    }

    fn record_encryption_local_store_snapshot(&self, snapshot: &EncryptedSendDiagnosticSnapshot) {
        let Ok(state) = self.state.lock() else {
            return;
        };
        let now = Instant::now();
        let mut event = DiagnosticEvent::new(
            DiagnosticLevel::Info,
            "core.send",
            "encryption_local_store_snapshot",
        )
        .field(DiagnosticField::correlation(
            "correlation",
            state.correlation,
        ))
        .field(DiagnosticField::token("send_kind", state.kind))
        .field(DiagnosticField::token("queue", "room_send_queue"))
        .field(DiagnosticField::milliseconds(
            "elapsed_since_submission_ms",
            now.duration_since(state.submitted_at).as_millis(),
        ))
        .field(DiagnosticField::milliseconds(
            "elapsed_since_previous_ms",
            now.duration_since(state.previous_stage_at).as_millis(),
        ))
        .field(DiagnosticField::token(
            "room_encryption",
            snapshot.room_encryption.token(),
        ))
        .field(DiagnosticField::token("recipient_strategy", "all_devices"))
        .field(DiagnosticField::token(
            "snapshot_consistency",
            "best_effort_concurrent_local_store",
        ))
        .field(DiagnosticField::token(
            "own_user_tracking",
            snapshot.own_user_tracking.token(),
        ));
        if let Some(value) = snapshot.outbound_session_present {
            event = event.field(DiagnosticField::boolean("outbound_session_present", value));
        }
        if let Some(value) = snapshot.own_device_present {
            event = event.field(DiagnosticField::boolean("own_device_present", value));
        }
        for (key, value) in [
            ("known_own_device_count", snapshot.known_own_device_count),
            (
                "known_own_other_device_count",
                snapshot.known_own_other_device_count,
            ),
            (
                "key_capable_own_other_device_count",
                snapshot.key_capable_own_other_device_count,
            ),
            (
                "cross_signed_own_other_device_count",
                snapshot.cross_signed_own_other_device_count,
            ),
            (
                "dehydrated_own_other_device_count",
                snapshot.dehydrated_own_other_device_count,
            ),
            (
                "blacklisted_own_other_device_count",
                snapshot.blacklisted_own_other_device_count,
            ),
        ] {
            if let Some(value) = value {
                event = event.field(DiagnosticField::count(
                    key,
                    value.try_into().unwrap_or(u64::MAX),
                ));
            }
        }
        record(event);
    }

    fn stage_internal(
        &mut self,
        stage: &'static str,
        outcome: Option<&'static str>,
        delivery_mode: Option<&'static str>,
        failure: Option<SendFailureDiagnostic>,
        once: bool,
    ) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if once && !state.recorded_once.insert(stage) {
            return;
        }
        let now = Instant::now();
        let mut event = DiagnosticEvent::new(DiagnosticLevel::Info, "core.send", stage)
            .field(DiagnosticField::correlation(
                "correlation",
                state.correlation,
            ))
            .field(DiagnosticField::token("send_kind", state.kind))
            .field(DiagnosticField::token("queue", "room_send_queue"))
            .field(DiagnosticField::milliseconds(
                "elapsed_since_submission_ms",
                now.duration_since(state.submitted_at).as_millis(),
            ))
            .field(DiagnosticField::milliseconds(
                "elapsed_since_previous_ms",
                now.duration_since(state.previous_stage_at).as_millis(),
            ));
        if let Some(outcome) = outcome {
            event = event.field(DiagnosticField::token("outcome", outcome));
        }
        if let Some(delivery_mode) = delivery_mode {
            event = event.field(DiagnosticField::token("delivery_mode", delivery_mode));
        }
        if let Some(failure) = failure {
            event = event
                .field(DiagnosticField::token("reason", failure.reason))
                .field(DiagnosticField::boolean("recoverable", failure.recoverable));
        }
        record(event);
        state.previous_stage_at = now;
    }
}

struct SendLifecycleTraceState {
    correlation: u64,
    kind: &'static str,
    submitted_at: Instant,
    previous_stage_at: Instant,
    recorded_once: HashSet<&'static str>,
}

struct CoordinatedPendingSend {
    registration_id: u64,
    active: bool,
    key: TimelineKey,
    client_txn_id: String,
    submission_id: Option<koushi_state::SubmissionId>,
    request_id: RequestId,
    settles_composer: bool,
    failure_reported: bool,
    interactive_guard: Option<InteractiveWorkGuard>,
    lifecycle_trace: SendLifecycleTrace,
}

pub(super) enum SendCompletionObservation {
    Sent {
        sdk_transaction_id: String,
        event_id: String,
    },
    SendError {
        sdk_transaction_id: String,
        diagnostic: SendFailureDiagnostic,
    },
    Cancelled {
        sdk_transaction_id: String,
    },
}

enum ObservedSendTerminal {
    Sent { event_id: String },
    SendError { diagnostic: SendFailureDiagnostic },
    Cancelled,
}

pub(super) struct SendCompletionRegistration {
    coordinator: SharedSendCompletionCoordinator,
    terminal_ingress: TimelineSendTerminalIngress,
    registration_id: Option<u64>,
    interactive_guard: Option<InteractiveWorkGuard>,
    lifecycle_trace: Option<SendLifecycleTrace>,
}

impl SendCompletionRegistration {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn begin(
        coordinator: SharedSendCompletionCoordinator,
        terminal_ingress: TimelineSendTerminalIngress,
        key: TimelineKey,
        client_txn_id: String,
        submission_id: Option<koushi_state::SubmissionId>,
        request_id: RequestId,
        settles_composer: bool,
    ) -> Self {
        let mut lifecycle_trace = SendLifecycleTrace::new(&key, settles_composer);
        lifecycle_trace.stage("accepted");
        let registration_id = {
            let mut coordinator = coordinator
                .lock()
                .expect("send completion coordinator lock must not be poisoned");
            coordinator.next_registration_id = coordinator
                .next_registration_id
                .checked_add(1)
                .expect("send registration id space exhausted");
            let registration_id = coordinator.next_registration_id;
            coordinator.registrations.insert(
                registration_id,
                CoordinatedPendingSend {
                    registration_id,
                    active: false,
                    key,
                    client_txn_id,
                    submission_id,
                    request_id,
                    settles_composer,
                    failure_reported: false,
                    interactive_guard: None,
                    lifecycle_trace: lifecycle_trace.clone(),
                },
            );
            registration_id
        };
        Self {
            coordinator,
            terminal_ingress,
            registration_id: Some(registration_id),
            interactive_guard: None,
            lifecycle_trace: Some(lifecycle_trace),
        }
    }

    pub(super) fn activate(&mut self) {
        let Some(registration_id) = self.registration_id else {
            return;
        };
        self.coordinator
            .lock()
            .expect("send completion coordinator lock must not be poisoned")
            .activate_registration(registration_id);
    }

    fn registration_id(&self) -> Option<u64> {
        self.registration_id
    }

    fn hold_interactive_guard(&mut self, guard: InteractiveWorkGuard) {
        self.interactive_guard = Some(guard);
        if let Some(trace) = self.lifecycle_trace.as_mut() {
            trace.stage("guard_acquired");
        }
    }

    pub(super) fn bind(&mut self, sdk_transaction_id: String) {
        let Some(registration_id) = self.registration_id.take() else {
            return;
        };
        self.lifecycle_trace
            .as_mut()
            .expect("active send registration must own lifecycle trace")
            .stage("sdk_enqueue_finished");
        let lifecycle_trace = self
            .lifecycle_trace
            .take()
            .expect("active send registration must own lifecycle trace");
        let interactive_guard = self.interactive_guard.take();
        let mut coordinator = self
            .coordinator
            .lock()
            .expect("send completion coordinator lock must not be poisoned");
        let handoffs = coordinator.bind_registration(
            registration_id,
            sdk_transaction_id,
            interactive_guard,
            lifecycle_trace,
        );
        for handoff in handoffs {
            let _admission = self.terminal_ingress.admit(handoff);
        }
    }

    fn fail_known(&mut self, kind: TimelineFailureKind) {
        let Some(registration_id) = self.registration_id.take() else {
            return;
        };
        if let Some(trace) = self.lifecycle_trace.as_mut() {
            trace.stage_with_outcome("sdk_enqueue_finished", Some("failed"), None);
            trace.stage_with_outcome_once("terminal_applied", Some("failed"), None);
            trace.stage_once("guard_released");
        }
        let mut coordinator = self
            .coordinator
            .lock()
            .expect("send completion coordinator lock must not be poisoned");
        self.interactive_guard.take();
        if let Some(handoff) = coordinator.fail_registration(registration_id, kind) {
            let _admission = self.terminal_ingress.admit(handoff);
        }
    }
}

impl Drop for SendCompletionRegistration {
    fn drop(&mut self) {
        let Some(registration_id) = self.registration_id.take() else {
            return;
        };
        if let Some(trace) = self.lifecycle_trace.as_mut() {
            trace.stage_with_outcome_once("terminal_applied", Some("abandoned"), None);
            trace.stage_once("guard_released");
        }
        let mut coordinator = self
            .coordinator
            .lock()
            .expect("send completion coordinator lock must not be poisoned");
        self.interactive_guard.take();
        if let Some(handoff) = coordinator.abandon_registration(registration_id) {
            let _admission = self.terminal_ingress.admit(handoff);
        }
    }
}

impl SendCompletionCoordinator {
    fn pending_send(
        &self,
        room_id: &str,
        sdk_transaction_id: &str,
    ) -> Option<(&TimelineKey, &str, RequestId)> {
        self.pending_sends
            .get(&SendCorrelationKey {
                room_id: room_id.to_owned(),
                sdk_transaction_id: sdk_transaction_id.to_owned(),
            })
            .map(|pending| {
                (
                    &pending.key,
                    pending.client_txn_id.as_str(),
                    pending.request_id,
                )
            })
    }

    fn stage_pending_send(&mut self, room_id: &str, sdk_transaction_id: &str, stage: &'static str) {
        if let Some(pending) = self.pending_sends.get_mut(&SendCorrelationKey {
            room_id: room_id.to_owned(),
            sdk_transaction_id: sdk_transaction_id.to_owned(),
        }) {
            pending.lifecycle_trace.stage(stage);
        }
    }

    fn activate_registration(&mut self, registration_id: u64) -> bool {
        let Some(registration) = self.registrations.get_mut(&registration_id) else {
            return false;
        };
        registration.active = true;
        true
    }

    fn cancel_registration(&mut self, registration_id: u64) {
        let room_id = self
            .registrations
            .remove(&registration_id)
            .map(|mut registration| {
                registration.lifecycle_trace.stage_with_outcome_once(
                    "terminal_applied",
                    Some("cancelled"),
                    None,
                );
                registration.lifecycle_trace.stage_once("guard_released");
                registration.key.room_id().to_owned()
            });
        if let Some(room_id) = room_id {
            self.purge_unmatched_for_inactive_room(&room_id);
        }
    }

    fn fail_registration(
        &mut self,
        registration_id: u64,
        kind: TimelineFailureKind,
    ) -> Option<TimelineSendTerminalHandoff> {
        let mut registration = self.registrations.remove(&registration_id)?;
        registration.lifecycle_trace.stage_with_outcome_once(
            "terminal_applied",
            Some("failed"),
            None,
        );
        registration.lifecycle_trace.stage_once("guard_released");
        let room_id = registration.key.room_id().to_owned();
        let handoff = (!registration.failure_reported)
            .then(|| timeline_send_failure_handoff(&registration, kind));
        self.purge_unmatched_for_inactive_room(&room_id);
        handoff
    }

    fn abandon_registration(
        &mut self,
        registration_id: u64,
    ) -> Option<TimelineSendTerminalHandoff> {
        let mut registration = self.registrations.remove(&registration_id)?;
        registration.lifecycle_trace.stage_with_outcome_once(
            "terminal_applied",
            Some("abandoned"),
            None,
        );
        registration.lifecycle_trace.stage_once("guard_released");
        let room_id = registration.key.room_id().to_owned();
        let handoff = (registration.active && !registration.failure_reported)
            .then(|| timeline_send_observation_loss_handoff(&registration));
        self.purge_unmatched_for_inactive_room(&room_id);
        handoff
    }

    fn room_has_active_registration(&self, room_id: &str) -> bool {
        self.registrations
            .values()
            .any(|registration| registration.active && registration.key.room_id() == room_id)
            || self
                .pending_sends
                .values()
                .any(|pending| pending.key.room_id() == room_id)
    }

    fn room_unbound_capacity(&self, room_id: &str) -> usize {
        self.registrations
            .values()
            .filter(|registration| registration.active && registration.key.room_id() == room_id)
            .count()
    }

    fn purge_unmatched_for_inactive_room(&mut self, room_id: &str) {
        if self.room_has_active_registration(room_id) {
            return;
        }
        self.unmatched_terminals
            .retain(|correlation, _| correlation.room_id != room_id);
    }

    fn remember_settled(&mut self, correlation: SendCorrelationKey) {
        if !self.settled_send_tombstones.insert(correlation.clone()) {
            return;
        }
        self.settled_send_order.push_back(correlation);
        while self.settled_send_order.len() > MAX_SETTLED_SEND_TOMBSTONES {
            if let Some(expired) = self.settled_send_order.pop_front() {
                self.settled_send_tombstones.remove(&expired);
            }
        }
    }

    fn bind_registration(
        &mut self,
        registration_id: u64,
        sdk_transaction_id: String,
        interactive_guard: Option<InteractiveWorkGuard>,
        lifecycle_trace: SendLifecycleTrace,
    ) -> Vec<TimelineSendTerminalHandoff> {
        let Some(mut registration) = self.registrations.remove(&registration_id) else {
            return Vec::new();
        };
        if !registration.active {
            self.purge_unmatched_for_inactive_room(registration.key.room_id());
            return Vec::new();
        }
        registration.interactive_guard = interactive_guard;
        registration.lifecycle_trace = lifecycle_trace;
        registration.lifecycle_trace.stage_once("terminal_bound");
        let correlation = SendCorrelationKey {
            room_id: registration.key.room_id().to_owned(),
            sdk_transaction_id,
        };
        if self.settled_send_tombstones.contains(&correlation)
            || self.pending_sends.contains_key(&correlation)
        {
            let handoffs = (!registration.failure_reported)
                .then(|| timeline_send_observation_loss_handoff(&registration))
                .into_iter()
                .collect();
            self.purge_unmatched_for_inactive_room(&correlation.room_id);
            return handoffs;
        }
        self.pending_sends.insert(correlation.clone(), registration);
        let observed = self
            .unmatched_terminals
            .remove(&correlation)
            .unwrap_or_default();
        let mut handoffs = Vec::new();
        for terminal in observed {
            if let Some(handoff) =
                self.apply_terminal(&correlation, terminal, "retained_before_binding")
            {
                handoffs.push(handoff);
            }
        }
        handoffs
    }

    fn observe(
        &mut self,
        room_id: &str,
        observation: SendCompletionObservation,
    ) -> Vec<TimelineSendTerminalHandoff> {
        let (sdk_transaction_id, terminal) = match observation {
            SendCompletionObservation::Sent {
                sdk_transaction_id,
                event_id,
            } => (sdk_transaction_id, ObservedSendTerminal::Sent { event_id }),
            SendCompletionObservation::SendError {
                sdk_transaction_id,
                diagnostic,
            } => (
                sdk_transaction_id,
                ObservedSendTerminal::SendError { diagnostic },
            ),
            SendCompletionObservation::Cancelled { sdk_transaction_id } => {
                (sdk_transaction_id, ObservedSendTerminal::Cancelled)
            }
        };
        let correlation = SendCorrelationKey {
            room_id: room_id.to_owned(),
            sdk_transaction_id,
        };
        if self.settled_send_tombstones.contains(&correlation) {
            return Vec::new();
        }
        if self.pending_sends.contains_key(&correlation) {
            return self
                .apply_terminal(&correlation, terminal, "immediate")
                .into_iter()
                .collect();
        }
        let capacity = self.room_unbound_capacity(room_id);
        if capacity == 0 {
            return Vec::new();
        }
        if let Some(observed) = self.unmatched_terminals.get_mut(&correlation) {
            if observed.len() < 2 {
                observed.push_back(terminal);
                return Vec::new();
            }
            return self.observation_lost(Some(room_id));
        }
        let retained_for_room = self
            .unmatched_terminals
            .keys()
            .filter(|candidate| candidate.room_id == room_id)
            .count();
        if retained_for_room >= capacity {
            return self.observation_lost(Some(room_id));
        }
        self.unmatched_terminals
            .entry(correlation)
            .or_default()
            .push_back(terminal);
        Vec::new()
    }

    fn observation_lost(&mut self, room_id: Option<&str>) -> Vec<TimelineSendTerminalHandoff> {
        let mut registration_ids = self
            .registrations
            .values()
            .filter(|registration| {
                registration.active
                    && room_id.is_none_or(|room_id| registration.key.room_id() == room_id)
            })
            .map(|registration| registration.registration_id)
            .chain(
                self.pending_sends
                    .values()
                    .filter(|pending| {
                        room_id.is_none_or(|room_id| pending.key.room_id() == room_id)
                    })
                    .map(|pending| pending.registration_id),
            )
            .collect::<Vec<_>>();
        registration_ids.sort_unstable();
        let mut handoffs = Vec::new();
        for registration_id in registration_ids {
            let registration = self.registrations.get_mut(&registration_id).or_else(|| {
                self.pending_sends
                    .values_mut()
                    .find(|pending| pending.registration_id == registration_id)
            });
            let Some(registration) = registration else {
                continue;
            };
            if registration.failure_reported {
                continue;
            }
            registration.failure_reported = true;
            registration.lifecycle_trace.stage_with_outcome_once(
                "terminal_applied",
                Some("failed"),
                None,
            );
            registration.lifecycle_trace.stage_once("guard_released");
            registration.interactive_guard.take();
            handoffs.push(timeline_send_observation_loss_handoff(registration));
        }
        handoffs
    }

    fn apply_terminal(
        &mut self,
        correlation: &SendCorrelationKey,
        terminal: ObservedSendTerminal,
        delivery_mode: &'static str,
    ) -> Option<TimelineSendTerminalHandoff> {
        match terminal {
            ObservedSendTerminal::Sent { event_id } => {
                let mut pending = self.pending_sends.remove(correlation)?;
                let diagnostic_correlation = pending.lifecycle_trace.correlation();
                pending.lifecycle_trace.stage_with_outcome(
                    "sdk_terminal_observed",
                    Some("sent"),
                    Some(delivery_mode),
                );
                pending.lifecycle_trace.stage_with_outcome_once(
                    "terminal_applied",
                    Some("succeeded"),
                    Some(delivery_mode),
                );
                pending.lifecycle_trace.stage_once("guard_released");
                let _send_guard = pending.interactive_guard.take();
                let settles_composer = pending.settles_composer && !pending.failure_reported;
                self.remember_settled(correlation.clone());
                self.purge_unmatched_for_inactive_room(&correlation.room_id);
                Some(timeline_send_terminal_handoff(
                    &pending.key,
                    pending.client_txn_id,
                    pending.submission_id,
                    Some(diagnostic_correlation),
                    SendCompletionTerminal::Succeeded {
                        request_id: pending.request_id,
                        event_id,
                        settles_composer,
                    },
                ))
            }
            ObservedSendTerminal::SendError { diagnostic } => {
                let pending = self.pending_sends.get_mut(correlation)?;
                if pending.failure_reported {
                    return None;
                }
                pending.failure_reported = true;
                pending.lifecycle_trace.stage_with_failure(
                    "sdk_terminal_observed",
                    Some("failed"),
                    Some(delivery_mode),
                    diagnostic,
                );
                pending.lifecycle_trace.stage_with_outcome_once(
                    "terminal_applied",
                    Some("failed"),
                    Some(delivery_mode),
                );
                pending.lifecycle_trace.stage_once("guard_released");
                pending.interactive_guard.take();
                Some(timeline_send_terminal_handoff(
                    &pending.key,
                    pending.client_txn_id.clone(),
                    pending.submission_id.clone(),
                    None,
                    SendCompletionTerminal::Failed {
                        settles_composer: pending.settles_composer,
                    },
                ))
            }
            ObservedSendTerminal::Cancelled => {
                let mut pending = self.pending_sends.remove(correlation)?;
                pending.lifecycle_trace.stage_with_outcome(
                    "sdk_terminal_observed",
                    Some("cancelled"),
                    Some(delivery_mode),
                );
                pending.lifecycle_trace.stage_with_outcome_once(
                    "terminal_applied",
                    Some("cancelled"),
                    Some(delivery_mode),
                );
                pending.lifecycle_trace.stage_once("guard_released");
                let _send_guard = pending.interactive_guard.take();
                let settles_composer = pending.settles_composer && !pending.failure_reported;
                self.remember_settled(correlation.clone());
                self.purge_unmatched_for_inactive_room(&correlation.room_id);
                Some(timeline_send_terminal_handoff(
                    &pending.key,
                    pending.client_txn_id,
                    pending.submission_id,
                    None,
                    SendCompletionTerminal::Cancelled { settles_composer },
                ))
            }
        }
    }
}

fn media_upload_progress_identity(
    coordinator: &SharedSendCompletionCoordinator,
    actor_key: &TimelineKey,
    sdk_transaction_id: &str,
) -> (String, Option<RequestId>) {
    coordinator
        .lock()
        .expect("send completion coordinator lock must not be poisoned")
        .pending_send(actor_key.room_id(), sdk_transaction_id)
        .and_then(|(pending_key, client_transaction_id, request_id)| {
            (pending_key == actor_key).then(|| (client_transaction_id.to_owned(), Some(request_id)))
        })
        .unwrap_or_else(|| (sdk_transaction_id.to_owned(), None))
}

pub(super) fn apply_send_completion_observation_and_handoff(
    coordinator: &SharedSendCompletionCoordinator,
    terminal_ingress: &TimelineSendTerminalIngress,
    room_id: &str,
    observation: SendCompletionObservation,
) {
    let mut coordinator = coordinator
        .lock()
        .expect("send completion coordinator lock must not be poisoned");
    for handoff in coordinator.observe(room_id, observation) {
        let _admission = terminal_ingress.admit(handoff);
    }
}

pub(super) fn apply_send_completion_observation_loss_and_handoff(
    coordinator: &SharedSendCompletionCoordinator,
    terminal_ingress: &TimelineSendTerminalIngress,
    room_id: Option<&str>,
) {
    let mut coordinator = coordinator
        .lock()
        .expect("send completion coordinator lock must not be poisoned");
    for handoff in coordinator.observation_lost(room_id) {
        let _admission = terminal_ingress.admit(handoff);
    }
}

const MAX_SETTLED_SEND_TOMBSTONES: usize = 128;

enum SendCompletionTerminal {
    Succeeded {
        request_id: RequestId,
        event_id: String,
        settles_composer: bool,
    },
    Failed {
        settles_composer: bool,
    },
    Cancelled {
        settles_composer: bool,
    },
}

fn send_terminal_action(
    key: &TimelineKey,
    client_transaction_id: &str,
    submission_id: Option<&koushi_state::SubmissionId>,
    terminal: &SendCompletionTerminal,
) -> Option<AppAction> {
    let settles_composer = match terminal {
        SendCompletionTerminal::Succeeded {
            settles_composer, ..
        }
        | SendCompletionTerminal::Failed { settles_composer }
        | SendCompletionTerminal::Cancelled { settles_composer } => *settles_composer,
    };
    if !settles_composer {
        return None;
    }
    if let Some((submission_id, target)) = submission_id.zip(submission_target(key)) {
        let outcome = match terminal {
            SendCompletionTerminal::Succeeded { .. } => {
                koushi_state::ComposerSubmissionTerminalOutcome::Succeeded
            }
            SendCompletionTerminal::Failed { .. } => {
                koushi_state::ComposerSubmissionTerminalOutcome::Failed {
                    message: "send failed".to_owned(),
                }
            }
            SendCompletionTerminal::Cancelled { .. } => {
                koushi_state::ComposerSubmissionTerminalOutcome::Cancelled
            }
        };
        return Some(AppAction::ComposerSubmissionSettled {
            submission_id: submission_id.clone(),
            transaction_id: client_transaction_id.to_owned(),
            target,
            outcome,
        });
    }
    match terminal {
        SendCompletionTerminal::Succeeded { .. } | SendCompletionTerminal::Cancelled { .. } => {
            send_finished_action(key, client_transaction_id.to_owned())
        }
        SendCompletionTerminal::Failed { .. } => {
            let projection = match key.kind {
                TimelineKind::Room { .. } => SendComposerProjection::Room,
                TimelineKind::Thread { .. } => SendComposerProjection::ThreadReply,
                TimelineKind::Focused { .. } => SendComposerProjection::None,
            };
            send_failed_action(
                key,
                projection,
                client_transaction_id.to_owned(),
                "send failed".to_owned(),
            )
        }
    }
}

fn timeline_send_terminal_handoff(
    key: &TimelineKey,
    client_transaction_id: String,
    submission_id: Option<koushi_state::SubmissionId>,
    diagnostic_correlation: Option<u64>,
    terminal: SendCompletionTerminal,
) -> TimelineSendTerminalHandoff {
    let action = send_terminal_action(
        key,
        &client_transaction_id,
        submission_id.as_ref(),
        &terminal,
    );
    let ledger_submission_id = action.as_ref().and(submission_id);
    let completion = match terminal {
        SendCompletionTerminal::Succeeded {
            request_id,
            event_id,
            ..
        } => Some(TimelineSendCompletionDelivery {
            request_id,
            key: key.clone(),
            transaction_id: client_transaction_id,
            event_id,
            diagnostic_correlation,
        }),
        SendCompletionTerminal::Failed { .. } | SendCompletionTerminal::Cancelled { .. } => None,
    };
    TimelineSendTerminalHandoff {
        submission_id: ledger_submission_id,
        action,
        completion,
        failure: None,
    }
}

fn timeline_send_observation_loss_handoff(
    pending: &CoordinatedPendingSend,
) -> TimelineSendTerminalHandoff {
    timeline_send_failure_handoff(pending, TimelineFailureKind::QueueOverflow)
}

fn timeline_send_failure_handoff(
    pending: &CoordinatedPendingSend,
    kind: TimelineFailureKind,
) -> TimelineSendTerminalHandoff {
    let mut handoff = timeline_send_terminal_handoff(
        &pending.key,
        pending.client_txn_id.clone(),
        pending.submission_id.clone(),
        None,
        SendCompletionTerminal::Failed {
            settles_composer: pending.settles_composer,
        },
    );
    handoff.failure = Some(TimelineSendFailureDelivery {
        request_id: pending.request_id,
        failure: CoreFailure::TimelineOperationFailed { kind },
    });
    handoff
}

#[cfg(test)]
mod tests {
    use super::super::test_source::item_body;

    use std::collections::{BTreeMap, BTreeSet, HashMap};

    use std::sync::{Arc, Mutex, atomic::Ordering};
    use std::task::Poll;
    use std::time::Duration;

    use futures_util::{FutureExt, StreamExt};

    use koushi_state::{AppAction, ComposerDocument, ComposerFormattingOptions};

    use crate::send_diagnostics::SendFailureDiagnostic;

    use matrix_sdk::send_queue::{RoomSendQueueUpdate, SendQueueUpdate};

    use tokio::sync::{broadcast, mpsc, oneshot};

    use crate::account_work::AccountWorkScheduler;

    use crate::command::TimelineCommand;
    use crate::event::{CoreEvent, TimelineEvent};
    use crate::executor;
    use crate::failure::{CoreFailure, TimelineFailureKind};
    #[cfg(any(test, feature = "test-hooks"))]
    use crate::ids::AccountKey;
    use crate::ids::{TimelineKey, TimelineKind};
    use crate::link_preview::LinkPreviewContext;

    use crate::live_tail_freshness::LiveTailRefreshCoordinator;

    use crate::threads_list::ThreadRootProjectionService;

    use koushi_diagnostics::DiagnosticValue;
    use koushi_state::{SessionInfo, SessionState, SubmissionId};
    use std::sync::atomic::AtomicBool;

    use crate::command::CoreCommand;
    use crate::runtime::CoreRuntime;

    use super::super::actor::TimelineActorHandle;
    use super::super::diagnostics::{
        OutboundSessionLookupDiagnostic, record_post_send_encryption_snapshot,
    };

    use super::super::manager::{TimelineManagerActor, TimelineMessage};
    use super::super::navigation::{TimelineActorGenerationGate, send_generation_fenced};
    use super::super::read_state::ReadWorkerSupervisor;
    use super::super::test_support::{
        fake_rid, gap_demand_test_actor_handle, live_tail_test_manager, room_key,
        test_timeline_actor_handle,
    };
    use super::super::thread_projection::{
        ReplayKnownThreadRootProjectionRegistry, ThreadRootProjectionFetchRegistry,
    };
    use super::{
        EncryptedSendDiagnosticSnapshot, MAX_CONCURRENT_SEND_DIAGNOSTICS,
        MAX_SETTLED_SEND_TOMBSTONES, MAX_SUBMISSION_TOMBSTONES, MediaSendQueuedDelivery,
        OwnUserTrackingDiagnosticState, RoomEncryptionDiagnosticState,
        SEND_ENQUEUE_WORKER_SHUTDOWN_DEADLINE, SendCompletionObservation,
        SendCompletionRegistration, SendCorrelationKey, SendEnqueueSuccess,
        SendEnqueueWorkerCompletion, SendEnqueueWorkerSupervisor, SendLifecycleTrace,
        SharedSendCompletionCoordinator, SubmissionAdmissionLedger, SyntheticSendEnqueueRequest,
        TimelineSendCompletionDelivery, TimelineSendEnqueueContext, TimelineSendEnqueuePayload,
        TimelineSendFailureDelivery, TimelineSendTerminalAdmission, TimelineSendTerminalHandoff,
        TimelineSendTerminalIngress, apply_send_completion_observation_and_handoff,
        apply_send_completion_observation_loss_and_handoff, await_submission_admission,
        classify_timeline_send_error, media_upload_progress_identity,
        run_global_send_completion_observer,
    };

    #[test]
    fn send_enqueue_takes_the_interactive_guard_before_the_sdk_enqueue() {
        let source = include_str!("outbound_send.rs");
        let spawn_source = source
            .split("fn spawn_send_enqueue(")
            .nth(1)
            .and_then(|section| {
                section
                    .split("fn handle_send_enqueue_worker_completion")
                    .next()
            })
            .expect("send enqueue spawner should exist");
        let guard_offset = spawn_source
            .find("begin_interactive(AccountWorkKind::MessageSend)")
            .expect("send enqueue must take the interactive work guard");
        let enqueue_offset = spawn_source
            .find("enqueue_timeline_send(context, payload)")
            .expect("send enqueue must still call the SDK enqueue");
        assert!(
            guard_offset < enqueue_offset,
            "the interactive guard must be held across the SDK enqueue"
        );
        assert!(
            spawn_source
                .find("preflight_started_tx.send(())")
                .expect("admission must still be acknowledged before the guard")
                < guard_offset,
            "send admission and local echo must not wait for the scheduler"
        );
    }

    #[test]
    fn send_completion_keeps_the_interactive_guard_until_terminal() {
        let source = include_str!("outbound_send.rs");
        let pending_source = source
            .split(concat!("struct ", "CoordinatedPendingSend"))
            .nth(1)
            .and_then(|section| section.split("enum SendCompletionObservation").next())
            .expect("pending send state should exist");
        assert!(
            pending_source.contains("interactive_guard: Option<InteractiveWorkGuard>"),
            "a bound send must retain the interactive guard until its SDK terminal"
        );

        let spawn_source = source
            .split("fn spawn_send_enqueue(")
            .nth(1)
            .and_then(|section| {
                section
                    .split("fn handle_send_enqueue_worker_completion")
                    .next()
            })
            .expect("send enqueue spawner should exist");
        let guard_offset = spawn_source
            .find("begin_interactive(AccountWorkKind::MessageSend)")
            .expect("send enqueue must take the interactive work guard");
        let retain_offset = spawn_source
            .find("registration.hold_interactive_guard")
            .expect("send enqueue must hand the guard to the completion registration");
        let enqueue_offset = spawn_source
            .find("enqueue_timeline_send(context, payload)")
            .expect("send enqueue must still call the SDK enqueue");
        assert!(
            guard_offset < retain_offset && retain_offset < enqueue_offset,
            "the interactive guard must be retained before SDK enqueue can bind"
        );

        let terminal_source = source
            .split(concat!("fn ", "apply_terminal("))
            .nth(1)
            .and_then(|section| section.split("fn media_upload_progress_identity").next())
            .expect("send terminal applier should exist");
        assert!(
            terminal_source.contains("pending.interactive_guard.take()"),
            "terminal handling must explicitly release the retained send guard"
        );
    }

    fn test_session_key() -> koushi_key::SessionKeyId {
        koushi_key::SessionKeyId {
            homeserver: "https://example.test".to_owned(),
            user_id: "@a:test".to_owned(),
            device_id: "DEVICE".to_owned(),
        }
    }

    #[tokio::test]
    async fn generation_fenced_send_discards_a_continuation_replaced_during_capacity_await() {
        let key = room_key();
        let generations = Arc::new(TimelineActorGenerationGate::default());
        let old_generation = generations.activate_after_quiescence(&key).await.generation;
        let (tx, mut rx) = mpsc::channel(1);
        tx.send("occupied").await.expect("fill bounded channel");

        let send_task = tokio::spawn({
            let tx = tx.clone();
            let generations = Arc::clone(&generations);
            let key = key.clone();
            async move { send_generation_fenced(&tx, &generations, &key, old_generation, "stale").await }
        });
        tokio::task::yield_now().await;
        let replacement_generation = generations.activate_after_quiescence(&key).await.generation;
        assert_ne!(replacement_generation, old_generation);

        assert_eq!(rx.recv().await, Some("occupied"));
        assert!(!send_task.await.expect("fenced send task"));
        assert!(
            rx.try_recv().is_err(),
            "stale value must never be published"
        );
    }

    #[tokio::test]
    async fn send_terminal_handoff_survives_origin_abort_and_delivers_exactly_once() {
        let key = room_key();
        let sdk_transaction_id = "sdk-terminal-handoff".to_owned();
        let client_transaction_id = "client-terminal-handoff".to_owned();
        let event_id = "$event-terminal-handoff:test".to_owned();
        let request_id = fake_rid(775);
        let coordinator = SharedSendCompletionCoordinator::default();

        let (action_tx, mut action_rx) = mpsc::channel(1);
        action_tx
            .send(vec![AppAction::ThreadRootProjectionsCleared {
                room_id: "!occupied:test".to_owned(),
            }])
            .await
            .expect("fill reducer channel");
        let (event_tx, mut event_rx) = broadcast::channel(8);
        let manager = TimelineManagerActor::spawn(
            action_tx,
            event_tx,
            None,
            AccountWorkScheduler::default(),
            None,
        );
        let mut registration = SendCompletionRegistration::begin(
            Arc::clone(&coordinator),
            manager.terminal_sender(),
            key.clone(),
            client_transaction_id.clone(),
            None,
            request_id,
            true,
        );
        registration.activate();
        registration.bind(sdk_transaction_id.clone());

        let (settled_tx, settled_rx) = oneshot::channel();
        let origin = executor::spawn({
            let coordinator = Arc::clone(&coordinator);
            let terminal_tx = manager.terminal_sender();
            let key = key.clone();
            let sdk_transaction_id = sdk_transaction_id.clone();
            let event_id = event_id.clone();
            async move {
                apply_send_completion_observation_and_handoff(
                    &coordinator,
                    &terminal_tx,
                    key.room_id(),
                    SendCompletionObservation::Sent {
                        sdk_transaction_id,
                        event_id,
                    },
                );
                let _ = settled_tx.send(());
                std::future::pending::<()>().await;
            }
        });
        settled_rx.await.expect("origin settled SDK terminal");
        origin.abort();

        // Model a duplicate global/direct terminal observation. The manager
        // coordinator must suppress it before another handoff is scheduled.
        apply_send_completion_observation_and_handoff(
            &coordinator,
            &manager.terminal_sender(),
            key.room_id(),
            SendCompletionObservation::Sent {
                sdk_transaction_id,
                event_id: event_id.clone(),
            },
        );

        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        assert!(
            manager
                .send(TimelineMessage::Shutdown {
                    acknowledged: Some(shutdown_tx),
                })
                .await
        );
        assert!(matches!(
            shutdown_rx.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));

        let _occupied = action_rx.recv().await.expect("occupied reducer action");
        let delivered = tokio::time::timeout(Duration::from_secs(1), action_rx.recv())
            .await
            .expect("manager terminal action timeout")
            .expect("manager terminal action");
        assert!(matches!(
            delivered.as_slice(),
            [AppAction::SendTextFinished { transaction_id, .. }]
                if transaction_id == &client_transaction_id
        ));
        let completed = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
            .await
            .expect("manager SendCompleted timeout")
            .expect("manager SendCompleted");
        assert!(matches!(
            completed,
            CoreEvent::Timeline(TimelineEvent::SendCompleted {
                request_id: delivered_request_id,
                key: delivered_key,
                transaction_id,
                event_id: delivered_event_id,
            }) if delivered_request_id == request_id
                && delivered_key == key
                && transaction_id == client_transaction_id
                && delivered_event_id == event_id
        ));

        shutdown_rx.await.expect("manager shutdown barrier");
        assert!(
            matches!(
                action_rx.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
                    | Err(mpsc::error::TryRecvError::Disconnected)
            ),
            "duplicate terminal must not enqueue a second reducer action"
        );
        assert!(
            matches!(
                event_rx.try_recv(),
                Err(broadcast::error::TryRecvError::Empty)
                    | Err(broadcast::error::TryRecvError::Closed)
            ),
            "duplicate terminal must not survive the manager shutdown barrier"
        );
    }

    #[tokio::test]
    async fn media_enqueue_publishes_queued_before_a_prebind_terminal() {
        let key = room_key();
        let request_id = fake_rid(7751);
        let client_transaction_id = "client-media-order".to_owned();
        let sdk_transaction_id = "sdk-media-order".to_owned();
        let event_id = "$event-media-order:test".to_owned();
        let mut manager = live_tail_test_manager(HashMap::new());
        let mut event_rx = manager.event_tx.subscribe();
        let mut registration = SendCompletionRegistration::begin(
            Arc::clone(&manager.send_completion),
            manager.terminal_ingress.clone(),
            key.clone(),
            client_transaction_id.clone(),
            None,
            request_id,
            false,
        );
        registration.activate();

        apply_send_completion_observation_and_handoff(
            &manager.send_completion,
            &manager.terminal_ingress,
            key.room_id(),
            SendCompletionObservation::Sent {
                sdk_transaction_id: sdk_transaction_id.clone(),
                event_id: event_id.clone(),
            },
        );
        assert!(
            manager.terminal_rx.try_recv().is_err(),
            "the SDK terminal must remain held until the enqueue worker binds its transaction"
        );

        manager.spawn_send_enqueue_future(registration, {
            let key = key.clone();
            let client_transaction_id = client_transaction_id.clone();
            async move {
                Ok(SendEnqueueSuccess {
                    sdk_transaction_id,
                    media_queued: Some(MediaSendQueuedDelivery {
                        request_id,
                        key,
                        transaction_id: client_transaction_id,
                    }),
                })
            }
        });
        let manager_tx = manager.msg_tx.clone();
        let manager_task = executor::spawn(manager.run());

        let first = event_rx.recv().await.expect("first media lifecycle event");
        let second = event_rx.recv().await.expect("second media lifecycle event");
        assert!(matches!(
            first,
            CoreEvent::Timeline(TimelineEvent::MediaSendQueued {
                request_id: queued_request_id,
                key: queued_key,
                transaction_id: queued_transaction_id,
            }) if queued_request_id == request_id
                && queued_key == key
                && queued_transaction_id == client_transaction_id
        ));
        assert!(matches!(
            second,
            CoreEvent::Timeline(TimelineEvent::SendCompleted {
                request_id: completed_request_id,
                key: completed_key,
                transaction_id: completed_transaction_id,
                event_id: completed_event_id,
            }) if completed_request_id == request_id
                && completed_key == key
                && completed_transaction_id == client_transaction_id
                && completed_event_id == event_id
        ));

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        manager_tx
            .send(TimelineMessage::Shutdown {
                acknowledged: Some(shutdown_tx),
            })
            .await
            .expect("manager shutdown command");
        shutdown_rx.await.expect("manager shutdown acknowledgement");
        manager_task.await.expect("manager shutdown task");
    }

    #[tokio::test]
    async fn send_terminal_required_action_failure_suppresses_completion_and_shutdowns() {
        let key = room_key();
        let request_id = fake_rid(776);
        let submission_id = SubmissionId::new("closed-reducer-terminal");
        let transaction_id = "client-closed-reducer".to_owned();
        let mut manager = live_tail_test_manager(HashMap::new());
        manager.accepted_submissions.accept(
            submission_id.clone(),
            key.clone(),
            transaction_id.clone(),
        );
        let mut event_rx = manager.event_tx.subscribe();
        assert!(matches!(
            manager.terminal_ingress.admit(TimelineSendTerminalHandoff {
                submission_id: Some(submission_id.clone()),
                action: Some(AppAction::SendTextFinished {
                    room_id: key.room_id().to_owned(),
                    transaction_id: transaction_id.clone(),
                }),
                completion: Some(TimelineSendCompletionDelivery {
                    request_id,
                    key,
                    transaction_id,
                    event_id: "$event-closed-reducer:test".to_owned(),
                    diagnostic_correlation: None,
                }),
                failure: None,
            }),
            TimelineSendTerminalAdmission::Accepted
        ));
        let handoff = manager.terminal_rx.recv().await;
        manager
            .handle_send_terminal_handoff(handoff.expect("accepted terminal handoff"))
            .await;
        assert!(
            matches!(
                event_rx.try_recv(),
                Err(broadcast::error::TryRecvError::Empty)
            ),
            "SendCompleted must fail closed when its required reducer action cannot be enqueued"
        );
        assert!(
            manager
                .accepted_submissions
                .active
                .contains_key(&submission_id)
        );
        assert!(
            !manager
                .accepted_submissions
                .tombstones
                .iter()
                .any(|(settled, _, _)| settled == &submission_id),
            "the admission ledger must not claim a terminal whose reducer action was rejected"
        );

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let manager_tx = manager.msg_tx.clone();
        let manager_task = executor::spawn(manager.run());
        manager_tx
            .send(TimelineMessage::Shutdown {
                acknowledged: Some(shutdown_tx),
            })
            .await
            .expect("manager shutdown command");
        tokio::time::timeout(Duration::from_secs(1), shutdown_rx)
            .await
            .expect("manager shutdown timeout")
            .expect("manager shutdown acknowledgement");
        manager_task.await.expect("manager shutdown task");
    }

    #[tokio::test]
    async fn observation_loss_failure_survives_required_action_channel_shutdown() {
        let key = room_key();
        let request_id = fake_rid(777);
        let mut manager = live_tail_test_manager(HashMap::new());
        let mut event_rx = manager.event_tx.subscribe();

        manager
            .handle_send_terminal_handoff(TimelineSendTerminalHandoff {
                submission_id: None,
                action: Some(AppAction::SendTextFailed {
                    room_id: key.room_id().to_owned(),
                    transaction_id: "client-observation-loss".to_owned(),
                    message: "send failed".to_owned(),
                }),
                completion: None,
                failure: Some(TimelineSendFailureDelivery {
                    request_id,
                    failure: CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::QueueOverflow,
                    },
                }),
            })
            .await;

        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::OperationFailed {
                request_id: delivered_request_id,
                failure: CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::QueueOverflow,
                },
            }) if delivered_request_id == request_id
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    fn synthetic_send_timeline_actor_handle(
        requests: mpsc::UnboundedSender<SyntheticSendEnqueueRequest>,
    ) -> TimelineActorHandle {
        let mut handle = test_timeline_actor_handle();
        handle.enqueue_context = Some(TimelineSendEnqueueContext::Synthetic { requests });
        handle
    }

    async fn poll_manager_enqueue_workers_once(manager: &mut TimelineManagerActor) {
        std::future::poll_fn(|context| {
            if let Poll::Ready(Some(completion)) =
                manager.send_enqueue_workers.tasks.poll_next_unpin(context)
            {
                manager.handle_send_enqueue_worker_completion(completion);
            }
            Poll::Ready(())
        })
        .await;
    }

    #[tokio::test]
    async fn duplicate_submission_routes_one_manager_enqueue_worker() {
        let key = room_key();
        let (enqueue_tx, mut enqueue_rx) = mpsc::unbounded_channel();
        let (action_tx, mut action_rx) = mpsc::channel(4);
        let (event_tx, mut event_rx) = broadcast::channel(4);
        let (msg_tx, msg_rx) = mpsc::channel(1);
        let (terminal_ingress, terminal_rx) = TimelineSendTerminalIngress::channel();
        let mut manager = TimelineManagerActor {
            session: None,
            room_list_service: None,
            room_subscription_checkpoint_task: None,
            room_subscription_service_epoch: 0,
            current_core_generation: None,
            room_leave_states: BTreeMap::new(),
            #[cfg(feature = "test-hooks")]
            restored_room_subscription_probe: None,
            session_subscribed_rooms: BTreeSet::new(),
            subscribed_room_leases: BTreeMap::new(),
            subscription_room_seen: BTreeSet::new(),
            subscription_room_ordinals: BTreeMap::new(),
            next_subscription_room_ordinal: 0,
            global_response_commit: None,
            timelines: HashMap::from([(
                key.clone(),
                synthetic_send_timeline_actor_handle(enqueue_tx),
            )]),
            accepted_submissions: SubmissionAdmissionLedger::default(),
            send_completion: SharedSendCompletionCoordinator::default(),
            global_send_completion_observer_future: None,
            send_enqueue_workers: SendEnqueueWorkerSupervisor::new(terminal_ingress.clone()),
            read_workers: ReadWorkerSupervisor::unavailable(),
            action_tx,
            event_tx,
            msg_tx,
            msg_rx,
            control_rx: None,
            navigation_projection_rx: None,
            last_navigation_projection_generation: 0,
            terminal_ingress,
            terminal_rx,
            search_index_tx: None,
            ignored_user_ids: Default::default(),
            data_dir: None,
            link_preview_policy: LinkPreviewContext::default(),
            composer_formatting_options: ComposerFormattingOptions::default(),
            account_work: AccountWorkScheduler::default(),
            thread_root_projection_service: Arc::new(Mutex::new(
                ThreadRootProjectionService::default(),
            )),
            thread_root_projection_fetches: ThreadRootProjectionFetchRegistry::default(),
            replay_known_thread_root_projections: Arc::new(Mutex::new(
                ReplayKnownThreadRootProjectionRegistry::default(),
            )),
            timeline_actor_generations: Arc::new(TimelineActorGenerationGate::default()),
            live_tail_refreshes: LiveTailRefreshCoordinator::new(),
            test_session_available: true,
        };
        manager
            .send_enqueue_workers
            .tasks
            .push(Box::pin(async { SendEnqueueWorkerCompletion }));
        let submission_id = SubmissionId::new("opaque-submission");
        for request_id in [fake_rid(7300), fake_rid(7301)] {
            manager
                .handle_command(TimelineCommand::SubmitText {
                    request_id,
                    expected_account: test_session_key(),
                    submission_id: submission_id.clone(),
                    key: key.clone(),
                    transaction_id: "txn-once".to_owned(),
                    document: ComposerDocument::from_plain_text("body"),
                    draft_revision: 1.into(),
                })
                .await;
        }
        let request = tokio::time::timeout(Duration::from_secs(1), enqueue_rx.recv())
            .await
            .expect("manager enqueue worker must be driven")
            .expect("one manager enqueue worker");
        assert!(matches!(
            request.payload,
            TimelineSendEnqueuePayload::Text { ref document, .. } if document.plain_body() == "body"
        ));
        assert!(enqueue_rx.try_recv().is_err());
        assert!(matches!(
            action_rx.try_recv(),
            Ok(actions) if matches!(actions.as_slice(), [AppAction::ComposerSubmissionAcceptedAtRevision { submission_id: accepted, .. }] if accepted == &submission_id)
        ));
        assert!(action_rx.try_recv().is_err());
        assert!(
            request
                .response
                .send(Ok(SendEnqueueSuccess::terminal_only(
                    "sdk-transaction".to_owned(),
                )))
                .is_ok(),
            "complete synthetic enqueue"
        );
        manager.join_send_enqueue_workers().await;

        while event_rx.try_recv().is_ok() {}
        manager.timelines.remove(&key);
        let rejected_id = SubmissionId::new("unsubscribed-submission");
        manager
            .handle_command(TimelineCommand::SubmitText {
                request_id: fake_rid(7302),
                expected_account: test_session_key(),
                submission_id: rejected_id.clone(),
                key: key.clone(),
                transaction_id: "txn-rejected".to_owned(),
                document: ComposerDocument::from_plain_text("body"),
                draft_revision: 2.into(),
            })
            .await;
        assert!(action_rx.try_recv().is_err());
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::Timeline(TimelineEvent::SubmissionRejected {
                submission_id,
                ..
            })) if submission_id == rejected_id
        ));

        let failed_id = SubmissionId::new("reducer-closed-submission");
        let (enqueue_tx, mut enqueue_rx) = mpsc::unbounded_channel();
        manager.timelines.insert(
            key.clone(),
            synthetic_send_timeline_actor_handle(enqueue_tx),
        );
        let (closed_action_tx, closed_action_rx) = mpsc::channel(1);
        drop(closed_action_rx);
        manager.action_tx = closed_action_tx;
        manager
            .handle_command(TimelineCommand::SubmitText {
                request_id: fake_rid(7303),
                expected_account: test_session_key(),
                submission_id: failed_id.clone(),
                key: key.clone(),
                transaction_id: "txn-reducer-closed".to_owned(),
                document: ComposerDocument::from_plain_text("body"),
                draft_revision: 3.into(),
            })
            .await;
        manager.join_send_enqueue_workers().await;
        assert!(
            enqueue_rx.try_recv().is_err(),
            "a rejected reducer action never releases the SDK enqueue permit"
        );
        manager
            .handle_command(TimelineCommand::SubmitText {
                request_id: fake_rid(7304),
                expected_account: test_session_key(),
                submission_id: failed_id.clone(),
                key,
                transaction_id: "txn-replayed".to_owned(),
                document: ComposerDocument::from_plain_text("changed"),
                draft_revision: 3.into(),
            })
            .await;
        assert!(
            enqueue_rx.try_recv().is_err(),
            "rejected replay never reaches SDK actor"
        );
        assert!(matches!(
            event_rx.try_recv(),
            Ok(CoreEvent::Timeline(TimelineEvent::SubmissionRejected { submission_id, .. }))
                if submission_id == failed_id
        ));
    }

    #[tokio::test]
    async fn drive_send_enqueue_until_preflight_started_returns_when_sender_closes() {
        let mut manager = live_tail_test_manager(HashMap::new());
        let (preflight_started_tx, preflight_started_rx) = oneshot::channel();
        drop(preflight_started_tx);

        tokio::time::timeout(
            Duration::from_secs(1),
            manager.drive_send_enqueue_until_preflight_started(preflight_started_rx),
        )
        .await
        .expect("a dropped preflight-start sender must not stall the manager command loop");
    }

    #[tokio::test]
    async fn submission_admission_permit_blocks_until_reducer_acceptance_and_aborts_on_drop() {
        let (permit_tx, mut permit_rx) = tokio::sync::oneshot::channel();
        assert!(matches!(
            permit_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        permit_tx.send(()).expect("open admission permit");
        assert!(await_submission_admission(Some(permit_rx)).await);

        let (permit_tx, permit_rx) = tokio::sync::oneshot::channel::<()>();
        drop(permit_tx);
        assert!(
            !await_submission_admission(Some(permit_rx)).await,
            "dropped permit aborts actor SDK work"
        );
        assert!(
            await_submission_admission(None).await,
            "legacy sends need no permit"
        );
    }

    #[tokio::test]
    async fn shutdown_acknowledges_after_timeline_children_are_dropped() {
        let (action_tx, _action_rx) = mpsc::channel(1);
        let (event_tx, _) = broadcast::channel(1);
        let handle = TimelineManagerActor::spawn(
            action_tx,
            event_tx,
            None,
            AccountWorkScheduler::default(),
            None,
        );
        let (acknowledged, acknowledgement) = tokio::sync::oneshot::channel();
        assert!(
            handle
                .send(TimelineMessage::Shutdown {
                    acknowledged: Some(acknowledged),
                })
                .await
        );
        tokio::time::timeout(Duration::from_secs(1), acknowledgement)
            .await
            .expect("shutdown acknowledgement must not hang")
            .expect("timeline manager acknowledges shutdown");
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_deadline_aborts_stalled_enqueue_worker_before_stopping_terminal_observer() {
        struct ObserverDrop(Arc<AtomicBool>);
        impl Drop for ObserverDrop {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        struct WorkerDrop {
            alive: Arc<AtomicBool>,
            observer_alive: Arc<AtomicBool>,
            settled_before_observer_stop: Arc<AtomicBool>,
        }
        impl Drop for WorkerDrop {
            fn drop(&mut self) {
                self.settled_before_observer_stop
                    .store(self.observer_alive.load(Ordering::SeqCst), Ordering::SeqCst);
                self.alive.store(false, Ordering::SeqCst);
            }
        }

        let mut manager = live_tail_test_manager(HashMap::new());
        let observer_alive = Arc::new(AtomicBool::new(true));
        let worker_alive = Arc::new(AtomicBool::new(true));
        let settled_before_observer_stop = Arc::new(AtomicBool::new(false));
        let (observer_started, observer_ready) = oneshot::channel();
        manager.global_send_completion_observer_future = Some(Box::pin({
            let observer_alive = Arc::clone(&observer_alive);
            async move {
                let _drop = ObserverDrop(observer_alive);
                let _ = observer_started.send(());
                futures_util::future::pending::<()>().await;
            }
        }));
        let mut registration = SendCompletionRegistration::begin(
            Arc::clone(&manager.send_completion),
            manager.terminal_ingress.clone(),
            room_key(),
            "client-stalled-shutdown".to_owned(),
            None,
            fake_rid(7470),
            true,
        );
        registration.activate();
        let (worker_started, worker_ready) = oneshot::channel();
        manager.spawn_send_enqueue_future(registration, {
            let alive = Arc::clone(&worker_alive);
            let observer_alive = Arc::clone(&observer_alive);
            let settled_before_observer_stop = Arc::clone(&settled_before_observer_stop);
            async move {
                let _drop = WorkerDrop {
                    alive,
                    observer_alive,
                    settled_before_observer_stop,
                };
                let _ = worker_started.send(());
                futures_util::future::pending::<Result<SendEnqueueSuccess, TimelineFailureKind>>()
                    .await
            }
        });
        let msg_tx = manager.msg_tx.clone();
        let run = executor::spawn(manager.run());
        observer_ready.await.expect("terminal observer started");
        worker_ready.await.expect("enqueue worker started");
        let (ack_tx, mut ack_rx) = oneshot::channel();
        msg_tx
            .send(TimelineMessage::Shutdown {
                acknowledged: Some(ack_tx),
            })
            .await
            .expect("shutdown command");

        tokio::task::yield_now().await;
        tokio::time::advance(SEND_ENQUEUE_WORKER_SHUTDOWN_DEADLINE).await;
        tokio::task::yield_now().await;

        assert!(
            matches!(ack_rx.try_recv(), Ok(())),
            "a stalled SDK enqueue must not hold shutdown acknowledgement forever"
        );
        assert!(!worker_alive.load(Ordering::SeqCst));
        assert!(settled_before_observer_stop.load(Ordering::SeqCst));
        assert!(!observer_alive.load(Ordering::SeqCst));
        run.await.expect("bounded manager shutdown");
    }

    #[tokio::test]
    async fn shutdown_grace_polls_exact_terminal_observer_before_worker_quiescence() {
        let mut manager = live_tail_test_manager(HashMap::new());
        let key = room_key();
        let sdk_transaction_id = "sdk-shutdown-grace";
        let mut registration = SendCompletionRegistration::begin(
            Arc::clone(&manager.send_completion),
            manager.terminal_ingress.clone(),
            key.clone(),
            "client-shutdown-grace".to_owned(),
            None,
            fake_rid(7472),
            true,
        );
        registration.activate();

        let (updates_tx, updates_rx) = broadcast::channel(4);
        manager.global_send_completion_observer_future =
            Some(Box::pin(run_global_send_completion_observer(
                updates_rx,
                Arc::clone(&manager.send_completion),
                manager.terminal_ingress.clone(),
            )));
        let (release_tx, release_rx) = oneshot::channel();
        manager.spawn_send_enqueue_future(registration, async move {
            let _ = release_rx.await;
            Ok(SendEnqueueSuccess::terminal_only(
                sdk_transaction_id.to_owned(),
            ))
        });

        let queue_terminal = async move {
            tokio::task::yield_now().await;
            updates_tx
                .send(SendQueueUpdate {
                    room_id: matrix_sdk::ruma::OwnedRoomId::try_from(key.room_id())
                        .expect("room id"),
                    update: RoomSendQueueUpdate::SentEvent {
                        transaction_id: matrix_sdk::ruma::OwnedTransactionId::from(
                            sdk_transaction_id,
                        ),
                        event_id: matrix_sdk::ruma::OwnedEventId::try_from("$shutdown-grace:test")
                            .expect("event id"),
                    },
                })
                .expect("queue exact SDK terminal");
            let _ = release_tx.send(());
        };
        let ((), ()) = tokio::join!(manager.join_send_enqueue_workers(), queue_terminal);

        let terminal = manager
            .terminal_rx
            .try_recv()
            .expect("graceful drain observes the exact terminal before observer teardown");
        assert!(terminal.failure.is_none());
        assert!(matches!(
            terminal.completion,
            Some(TimelineSendCompletionDelivery { event_id, .. })
                if event_id == "$shutdown-grace:test"
        ));
    }

    #[tokio::test]
    async fn shutdown_cleans_captured_room_keys_before_acknowledging() {
        struct DropSignal(Option<oneshot::Sender<()>>);
        impl Drop for DropSignal {
            fn drop(&mut self) {
                if let Some(signal) = self.0.take() {
                    let _ = signal.send(());
                }
            }
        }

        let key = room_key();
        let generations = Arc::new(TimelineActorGenerationGate::default());
        generations.activate_after_quiescence(&key).await;
        let (dropped_tx, dropped_rx) = oneshot::channel();
        let child = executor::spawn(async move {
            let _signal = DropSignal(Some(dropped_tx));
            std::future::pending::<()>().await;
        });
        let (actor_tx, _actor_rx) = mpsc::channel(1);
        let (action_tx, mut action_rx) = mpsc::channel(4);
        let (event_tx, _) = broadcast::channel(4);
        let (msg_tx, msg_rx) = mpsc::channel(4);
        let (terminal_ingress, terminal_rx) = TimelineSendTerminalIngress::channel();
        let manager = TimelineManagerActor {
            session: None,
            room_list_service: None,
            room_subscription_checkpoint_task: None,
            room_subscription_service_epoch: 0,
            current_core_generation: None,
            room_leave_states: BTreeMap::new(),
            #[cfg(feature = "test-hooks")]
            restored_room_subscription_probe: None,
            session_subscribed_rooms: BTreeSet::new(),
            subscribed_room_leases: BTreeMap::new(),
            subscription_room_seen: BTreeSet::new(),
            subscription_room_ordinals: BTreeMap::new(),
            next_subscription_room_ordinal: 0,
            global_response_commit: None,
            timelines: HashMap::from([(
                key.clone(),
                TimelineActorHandle {
                    tx: actor_tx,
                    control_tx: None,
                    thread_summary_projection:
                        crate::timeline::actor::ThreadSummaryProjectionIngress::channel().0,
                    position_rx: None,
                    task: Some(child),
                    auxiliary_tasks: Vec::new(),
                    subscription_generation: None,
                    enqueue_context: None,
                },
            )]),
            accepted_submissions: SubmissionAdmissionLedger::default(),
            send_completion: SharedSendCompletionCoordinator::default(),
            global_send_completion_observer_future: None,
            send_enqueue_workers: SendEnqueueWorkerSupervisor::new(terminal_ingress.clone()),
            read_workers: ReadWorkerSupervisor::unavailable(),
            action_tx,
            event_tx,
            msg_tx: msg_tx.clone(),
            msg_rx,
            control_rx: None,
            navigation_projection_rx: None,
            last_navigation_projection_generation: 0,
            terminal_ingress,
            terminal_rx,
            search_index_tx: None,
            ignored_user_ids: Default::default(),
            data_dir: None,
            link_preview_policy: LinkPreviewContext::default(),
            composer_formatting_options: ComposerFormattingOptions::default(),
            account_work: AccountWorkScheduler::default(),
            thread_root_projection_service: Arc::new(Mutex::new(
                ThreadRootProjectionService::default(),
            )),
            thread_root_projection_fetches: ThreadRootProjectionFetchRegistry::default(),
            replay_known_thread_root_projections: Arc::new(Mutex::new(
                ReplayKnownThreadRootProjectionRegistry::default(),
            )),
            timeline_actor_generations: generations.clone(),
            live_tail_refreshes: LiveTailRefreshCoordinator::new(),
            test_session_available: true,
        };
        let run = executor::spawn(async move { manager.run().await });
        let (ack_tx, ack_rx) = oneshot::channel();
        msg_tx
            .send(TimelineMessage::Shutdown {
                acknowledged: Some(ack_tx),
            })
            .await
            .expect("shutdown command");
        ack_rx.await.expect("shutdown acknowledgement");
        dropped_rx
            .await
            .expect("child dropped before acknowledgement");
        assert!(matches!(
            action_rx.recv().await,
            Some(actions) if matches!(actions.as_slice(), [AppAction::ThreadRootProjectionsCleared { room_id }] if room_id == key.room_id())
        ));
        assert!(
            !generations
                .state
                .lock()
                .expect("generation gate")
                .entries
                .contains_key(&key)
        );
        run.await.expect("manager shutdown");
    }

    #[tokio::test]
    async fn manager_enqueue_worker_waits_for_reducer_acceptance_delivery() {
        let key = room_key();
        let (enqueue_tx, mut enqueue_rx) = mpsc::unbounded_channel();
        let (action_tx, mut action_rx) = mpsc::channel(1);
        action_tx
            .try_send(Vec::new())
            .expect("pause reducer delivery");
        let (event_tx, mut event_rx) = broadcast::channel(4);
        let (msg_tx, msg_rx) = mpsc::channel(1);
        let (terminal_ingress, terminal_rx) = TimelineSendTerminalIngress::channel();
        let mut manager = TimelineManagerActor {
            session: None,
            room_list_service: None,
            room_subscription_checkpoint_task: None,
            room_subscription_service_epoch: 0,
            current_core_generation: None,
            room_leave_states: BTreeMap::new(),
            #[cfg(feature = "test-hooks")]
            restored_room_subscription_probe: None,
            session_subscribed_rooms: BTreeSet::new(),
            subscribed_room_leases: BTreeMap::new(),
            subscription_room_seen: BTreeSet::new(),
            subscription_room_ordinals: BTreeMap::new(),
            next_subscription_room_ordinal: 0,
            global_response_commit: None,
            timelines: HashMap::from([(
                key.clone(),
                synthetic_send_timeline_actor_handle(enqueue_tx),
            )]),
            accepted_submissions: SubmissionAdmissionLedger::default(),
            send_completion: SharedSendCompletionCoordinator::default(),
            global_send_completion_observer_future: None,
            send_enqueue_workers: SendEnqueueWorkerSupervisor::new(terminal_ingress.clone()),
            read_workers: ReadWorkerSupervisor::unavailable(),
            action_tx,
            event_tx,
            msg_tx,
            msg_rx,
            control_rx: None,
            navigation_projection_rx: None,
            last_navigation_projection_generation: 0,
            terminal_ingress,
            terminal_rx,
            search_index_tx: None,
            ignored_user_ids: Default::default(),
            data_dir: None,
            link_preview_policy: LinkPreviewContext::default(),
            composer_formatting_options: ComposerFormattingOptions::default(),
            account_work: AccountWorkScheduler::default(),
            thread_root_projection_service: Arc::new(Mutex::new(
                ThreadRootProjectionService::default(),
            )),
            thread_root_projection_fetches: ThreadRootProjectionFetchRegistry::default(),
            replay_known_thread_root_projections: Arc::new(Mutex::new(
                ReplayKnownThreadRootProjectionRegistry::default(),
            )),
            timeline_actor_generations: Arc::new(TimelineActorGenerationGate::default()),
            live_tail_refreshes: LiveTailRefreshCoordinator::new(),
            test_session_available: true,
        };
        let submission_id = SubmissionId::new("paused-admission");
        let command_id = submission_id.clone();
        let registry = Arc::new(crate::composer_draft_lifecycle::ComposerDraftLeaseRegistry::new());
        let scope = crate::composer_draft_lifecycle::ComposerDraftScope {
            account: test_session_key(),
            target: koushi_state::ComposerTarget::Main {
                room_id: key.room_id().to_owned(),
            },
        };
        let renderer_generation = registry
            .begin_renderer_generation()
            .expect("begin renderer generation");
        let lease_id = registry
            .acquire(renderer_generation, scope.clone())
            .expect("acquire exact composer lease");
        let command_permit = registry
            .try_command_permit(renderer_generation, lease_id, &scope)
            .expect("admit exact composer command");
        let app_pending_permit = command_permit.clone();
        registry
            .release(renderer_generation, lease_id)
            .expect("release activation after command admission");
        let (rejected_tx, mut rejected_rx) = mpsc::unbounded_channel();
        let (acceptance_probe_tx, acceptance_probe_rx) = oneshot::channel();
        let forwarded_permit =
            crate::runtime::ForwardedComposerDraftPermit::new_with_acceptance_probe(
                fake_rid(7310),
                command_permit,
                rejected_tx,
                acceptance_probe_tx,
            );
        let route = tokio::spawn(async move {
            manager
                .handle_command_with_permit(
                    TimelineCommand::SubmitText {
                        request_id: fake_rid(7310),
                        expected_account: test_session_key(),
                        submission_id: command_id,
                        key,
                        transaction_id: "txn-paused".to_owned(),
                        document: ComposerDocument::from_plain_text("body"),
                        draft_revision: 4.into(),
                    },
                    Some(forwarded_permit),
                )
                .await;
            manager
        });
        acceptance_probe_rx
            .await
            .expect("timeline reached acceptance projection");
        assert_eq!(
            registry.protected_targets(&scope.account),
            BTreeSet::from([scope.target.clone()]),
            "the forwarded permit must protect the exact target while reducer delivery is blocked"
        );
        assert!(
            enqueue_rx.try_recv().is_err(),
            "the manager worker must stay permit-blocked before reducer acceptance"
        );
        assert!(event_rx.try_recv().is_err());
        assert!(action_rx.recv().await.expect("pause marker").is_empty());
        assert!(
            matches!(action_rx.recv().await, Some(actions) if matches!(actions.as_slice(), [AppAction::ComposerSubmissionAcceptedAtRevision { submission_id: accepted, .. }] if accepted == &submission_id))
        );
        let mut manager = route.await.expect("manager route");
        assert!(
            matches!(event_rx.try_recv(), Ok(CoreEvent::Timeline(TimelineEvent::SubmissionAccepted { submission_id: accepted, .. })) if accepted == submission_id)
        );
        assert_eq!(
            registry.protected_targets(&scope.account),
            BTreeSet::from([scope.target.clone()]),
            "the AppActor pending clone must outlive timeline acceptance enqueue"
        );
        let mut registry_changes = registry.subscribe();
        registry_changes.borrow_and_update();
        drop(app_pending_permit);
        registry_changes
            .changed()
            .await
            .expect("pending acceptance permit release notification");
        assert!(
            registry.protected_targets(&scope.account).is_empty(),
            "the exact target becomes eligible only after the matching reducer acceptance"
        );
        assert!(
            rejected_rx.try_recv().is_err(),
            "successful acceptance enqueue must disarm rejection cleanup"
        );
        let request = tokio::time::timeout(Duration::from_secs(1), enqueue_rx.recv())
            .await
            .expect("accepted enqueue worker must be driven")
            .expect("accepted submission releases manager enqueue worker");
        assert!(matches!(
            request.payload,
            TimelineSendEnqueuePayload::Text { ref document, .. } if document.plain_body() == "body"
        ));
        assert!(
            request
                .response
                .send(Ok(SendEnqueueSuccess::terminal_only(
                    "sdk-transaction".to_owned(),
                )))
                .is_ok(),
            "complete synthetic enqueue"
        );
        manager.join_send_enqueue_workers().await;
    }

    #[test]
    fn submission_admission_tombstones_are_bounded_and_active_is_retained() {
        let mut ledger = SubmissionAdmissionLedger::default();
        let key = room_key();
        let active = SubmissionId::new("active");
        ledger.accept(active.clone(), key.clone(), "active-txn".to_owned());
        for index in 0..=MAX_SUBMISSION_TOMBSTONES {
            let id = SubmissionId::new(format!("terminal-{index}"));
            ledger.accept(id.clone(), key.clone(), format!("txn-{index}"));
            ledger.terminal(&id);
        }
        assert_eq!(ledger.tombstones.len(), MAX_SUBMISSION_TOMBSTONES);
        assert!(ledger.active.contains_key(&active));
        assert!(ledger.get(&SubmissionId::new("terminal-0")).is_none());
    }

    #[test]
    fn send_submission_is_not_reduced_before_manager_worker_route_exists() {
        let source = include_str!("outbound_send.rs");
        let helper_source = source
            .split("async fn route_send_to_worker_or_fail")
            .nth(1)
            .expect("manager send-worker route helper should exist")
            .split("async fn route_media_send_to_worker_or_fail")
            .next()
            .expect("media worker route helper should follow text/reply routing");
        let route_lookup_offset = helper_source
            .find("handle.enqueue_context.clone()")
            .expect("send route helper should resolve manager enqueue context first");
        let submitted_offset = helper_source
            .find("send_submitted_action")
            .expect("send route helper should reduce submitted state through a projection helper");
        let worker_offset = helper_source
            .find("self.spawn_send_enqueue")
            .expect("manager route should spawn a supervised enqueue worker");

        assert!(
            route_lookup_offset < submitted_offset && submitted_offset < worker_offset,
            "submitted state must follow manager route resolution and precede SDK enqueue"
        );
        assert!(
            source.contains("AppAction::SendTextSubmitted"),
            "room send projection should reduce SendTextSubmitted"
        );
    }

    #[test]
    fn thread_reply_submission_is_not_reduced_before_manager_worker_route_exists() {
        let source = include_str!("outbound_send.rs");
        let helper_source = source
            .split("async fn route_send_to_worker_or_fail")
            .nth(1)
            .expect("send route helper should exist")
            .split("async fn route_media_send_to_worker_or_fail")
            .next()
            .expect("media worker route helper should follow text/reply routing");

        let route_lookup_offset = helper_source
            .find("handle.enqueue_context.clone()")
            .expect("send route helper should resolve manager enqueue context first");
        let submitted_offset = helper_source
            .find("send_submitted_action")
            .expect("send route helper should reduce submitted state through a projection helper");

        assert!(
            route_lookup_offset < submitted_offset,
            "submitted send state must not be reduced before the manager worker route exists"
        );
        assert!(
            source.contains("AppAction::ThreadReplySubmitted"),
            "thread send projection should reduce ThreadReplySubmitted"
        );
    }

    #[test]
    fn thread_timeline_keys_project_send_reply_to_thread_composer_actions() {
        let source = include_str!("outbound_send.rs");
        let helper_source = source
            .split("async fn route_send_to_worker_or_fail")
            .nth(1)
            .expect("send route helper should exist")
            .split("async fn route_media_send_to_worker_or_fail")
            .next()
            .expect("media worker route helper should follow text/reply routing");
        let projection_source = source
            .split("fn send_submitted_action")
            .nth(1)
            .expect("send submitted projection helper should exist")
            .split("fn send_finished_action")
            .next()
            .expect("send finished projection helper should follow submit helper");
        let _finished_projection_source = source
            .split("fn send_finished_action")
            .nth(1)
            .expect("send finished projection helper should exist")
            .split("fn send_failed_action")
            .next()
            .expect("send failed projection helper should follow finished helper");
        let _failed_projection_source = source
            .split("fn send_failed_action")
            .nth(1)
            .expect("send failed projection helper should exist")
            .split("// ---------------------------------------------------------------------------")
            .next()
            .expect("projection helper section should end");
        let terminal_action_source = source
            .split("fn send_terminal_action")
            .nth(1)
            .expect("send terminal action helper should exist")
            .split("fn timeline_send_terminal_handoff")
            .next()
            .expect("send terminal handoff builder should follow action helper");

        assert!(
            helper_source.contains("send_submitted_action")
                && projection_source.contains("TimelineKind::Thread")
                && projection_source.contains("ThreadReplySubmitted"),
            "thread SendReply routes must submit thread composer state"
        );
        assert!(
            source.contains("ThreadReplyFailed"),
            "thread SendReply route failures must clear thread composer pending state"
        );
        assert!(
            terminal_action_source.contains("ComposerSubmissionSettled")
                && terminal_action_source.contains("ComposerSubmissionTerminalOutcome")
                && terminal_action_source.contains("submission_target"),
            "manager-owned send terminals must settle thread composer state"
        );
        assert!(
            source.contains("TimelineKind::Focused { .. } => Self::None")
                && source.contains("TimelineKind::Focused { .. } => None"),
            "focused timelines must not own composer state"
        );
    }

    #[test]
    fn outbound_send_state_uses_sdk_truth_and_reliable_settles() {
        let outbound = include_str!("outbound_send.rs")
            .rsplit_once("\n#[cfg(test)]\nmod tests")
            .map(|(source, _)| source)
            .unwrap_or(include_str!("outbound_send.rs"));
        let manager = include_str!("manager.rs")
            .rsplit_once("\n#[cfg(test)]\nmod tests")
            .map(|(source, _)| source)
            .unwrap_or(include_str!("manager.rs"));
        let send_queue_monitor = item_body(outbound, "async fn run_send_queue_monitor");
        let send_state_projection = item_body(
            include_str!("item_projection.rs"),
            "fn sdk_item_to_timeline_item_with_send_states",
        );
        let terminal_boundary_source =
            item_body(outbound, "fn apply_send_completion_observation_and_handoff");
        let global_observer_source =
            item_body(outbound, "async fn run_global_send_completion_observer");
        let actor_update_source = item_body(outbound, "async fn handle_send_queue_update");
        let manager_delivery_source = item_body(outbound, "async fn handle_send_terminal_handoff");
        assert!(
            send_queue_monitor.contains("TimelineActorMessage::SendQueueLagged"),
            "send-queue broadcast lag must ask the actor to resync its send-state mirror"
        );
        assert!(
            !send_queue_monitor.contains("not critical for send completion tracking"),
            "lagged send-queue updates can contain terminal send states and must not be ignored"
        );
        let sdk = send_state_projection
            .find("timeline_send_state_from_sdk")
            .expect("projection should read SDK send state");
        let mirror = send_state_projection
            .find("send_statuses.get")
            .expect("projection should still use the actor mirror as fallback");
        assert!(
            sdk < mirror,
            "SDK timeline item send state must win over the actor mirror after relay gaps"
        );
        assert!(
            terminal_boundary_source.contains("terminal_ingress.admit(handoff)")
                && !terminal_boundary_source.contains(".await"),
            "tracker settlement must synchronously admit the manager-owned terminal handoff"
        );
        assert!(
            !terminal_boundary_source.contains("executor::spawn"),
            "terminal ownership transfer must not depend on a detached task reaching the manager mailbox"
        );
        assert!(
            global_observer_source.contains("SendQueueUpdate")
                && global_observer_source.contains("RecvError::Lagged")
                && global_observer_source
                    .contains("apply_send_completion_observation_loss_and_handoff"),
            "the session-global observer must own exact terminals and explicit lag failure"
        );
        assert!(
            !actor_update_source.contains("apply_send_completion_observation_and_handoff")
                && !actor_update_source.contains("SendCompleted"),
            "replaceable actor-local monitors are presentation-only terminal consumers"
        );
        assert!(
            manager_delivery_source.contains("deliver_submission_terminal_action")
                && manager_delivery_source.contains(".await")
                && !manager_delivery_source.contains("try_send"),
            "the stable manager must wait for reducer capacity before completion emission"
        );
        let manager_run_source = item_body(manager, "async fn run(mut self)");
        assert!(
            manager_run_source.contains("tokio::select!")
                && manager_run_source.contains("biased;")
                && manager_run_source.contains("terminal_rx.recv()"),
            "the manager must prioritize its owned terminal ingress"
        );
        assert_eq!(
            manager
                .matches("session.client().send_queue().subscribe()")
                .count(),
            1,
            "one client-global send terminal subscription belongs to the session manager"
        );
        assert!(
            !outbound.contains("SharedSendCompletionTracker")
                && !manager.contains("SharedSendCompletionTracker")
        );
    }

    #[test]
    fn outbound_sdk_enqueues_are_session_manager_owned_and_supervised() {
        let outbound = include_str!("outbound_send.rs")
            .rsplit_once("\n#[cfg(test)]\nmod tests")
            .map(|(source, _)| source)
            .unwrap_or(include_str!("outbound_send.rs"));
        let manager = include_str!("manager.rs")
            .rsplit_once("\n#[cfg(test)]\nmod tests")
            .map(|(source, _)| source)
            .unwrap_or(include_str!("manager.rs"));
        let submission_route = item_body(outbound, "async fn route_submission_to_worker");
        let spawn_worker = submission_route
            .find("self.spawn_send_enqueue")
            .expect("manager must own the permit-blocked enqueue worker");
        let activate = submission_route
            .find("activate_registration")
            .expect("registration activation should be explicit");
        let reducer_action = submission_route
            .find("self.action_tx.send")
            .expect("acceptance must enter the reliable reducer channel");
        let release_permit = submission_route
            .find("permit_tx.send")
            .expect("SDK enqueue should be released after acceptance");
        assert!(
            spawn_worker < activate && activate < reducer_action && reducer_action < release_permit,
            "the supervised worker must exist before acceptance and stay permit-blocked until reducer delivery"
        );
        for actor_variant in [
            "TimelineActorMessage::SendText",
            "TimelineActorMessage::SendReply",
            "TimelineActorMessage::UploadAndSendMedia",
        ] {
            assert!(
                !outbound.contains(actor_variant) && !manager.contains(actor_variant),
                "replaceable timeline actors must not own SDK enqueue: {actor_variant}"
            );
        }
        for route in [
            "route_send_to_worker_or_fail",
            "route_submission_to_worker",
            "route_media_send_to_worker_or_fail",
        ] {
            assert!(
                outbound.contains(route),
                "text, reply, and media commands must route through manager workers: {route}"
            );
        }
        assert!(
            outbound.contains("enqueue_timeline_send(context, payload).await"),
            "all enqueue payloads must pass through the common supervised future"
        );
        let manager_run = item_body(manager, "async fn run(mut self)");
        let worker_poll = manager_run
            .find("worker = self.send_enqueue_workers.tasks.next()")
            .expect("manager run loop should reap enqueue workers");
        let mailbox_poll = manager_run
            .find("msg = self.msg_rx.recv()")
            .expect("manager run loop should poll commands");
        let worker_join = manager_run
            .find("self.join_send_enqueue_workers().await")
            .expect("shutdown should join enqueue workers");
        let observer_stop = manager_run
            .find("self.global_send_completion_observer_future.take()")
            .expect("shutdown should stop the global observer explicitly");
        let actor_drain = manager_run
            .find("let timeline_actors = self")
            .expect("shutdown should stop replaceable actors explicitly");
        assert!(
            manager_run.contains("if !self.send_enqueue_workers.tasks.is_empty()")
                && worker_poll < mailbox_poll,
            "a guarded, biased worker branch must poll owned futures without empty-stream spinning"
        );
        assert!(
            worker_join < observer_stop && observer_stop < actor_drain,
            "shutdown must join enqueue workers while the global observer is live, then stop presentation actors"
        );
        let supervisor_impl = item_body(outbound, "impl SendEnqueueWorkerSupervisor");
        let supervisor_drop = item_body(outbound, "impl Drop for SendEnqueueWorkerSupervisor");
        assert!(
            supervisor_impl.contains("fn cancel_all(&mut self)")
                && supervisor_impl.contains("self.tasks = FuturesUnordered::new()")
                && supervisor_drop.contains("self.terminal_ingress.stop_accepting()")
                && supervisor_drop.contains("self.cancel_all()"),
            "abnormal manager drop must close terminal admission and synchronously drop every owned worker future"
        );
        let manager_drop = item_body(manager, "impl Drop for TimelineManagerActor");
        let admission_close = manager_drop
            .find("self.terminal_ingress.stop_accepting()")
            .expect("manager Drop closes terminal admission");
        let worker_cancel = manager_drop
            .find("self.send_enqueue_workers.cancel_all()")
            .expect("manager Drop synchronously cancels workers");
        let observer_drop = manager_drop
            .find("self.global_send_completion_observer_future.take()")
            .expect("manager Drop synchronously drops the observer");
        assert!(
            admission_close < worker_cancel && worker_cancel < observer_drop,
            "manager Drop must close admission, settle workers, then drop the observer"
        );
        let runner = item_body(outbound, "fn spawn_send_enqueue_future");
        assert!(
            runner.contains("AssertUnwindSafe") && runner.contains(".catch_unwind()"),
            "directly-polled enqueue futures must retain panic isolation"
        );
    }

    #[tokio::test]
    async fn send_without_authoritative_account_session_fails_closed() {
        let runtime = CoreRuntime::start();
        let mut conn = runtime.attach();

        runtime
            .inject_actions(vec![
                AppAction::RestoreSessionRequested,
                AppAction::RestoreSessionSucceeded(SessionInfo {
                    homeserver: "https://test.test".to_owned(),
                    user_id: "@a:test".to_owned(),
                    device_id: "DEV".to_owned(),
                    authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
                }),
                AppAction::CurrentDeviceTrustChanged(
                    koushi_state::CurrentDeviceTrustState::Verified,
                ),
            ])
            .await;
        loop {
            if matches!(conn.snapshot().session, SessionState::Ready(_)) {
                break;
            }
            crate::executor::sleep(Duration::from_millis(5)).await;
        }

        let rid = conn.next_request_id();
        conn.command(CoreCommand::Timeline(TimelineCommand::SendText {
            request_id: rid,
            key: room_key(),
            transaction_id: "txn-unsubscribed".to_owned(),
            document: koushi_state::ComposerDocument::from_plain_text("hello".to_owned()),
        }))
        .await
        .expect("submit");

        loop {
            let timeout = tokio::time::timeout(Duration::from_secs(5), conn.recv_event()).await;
            let event = timeout.expect("no timeout").expect("no lag");
            match event {
                CoreEvent::OperationFailed {
                    request_id,
                    failure,
                } if request_id == rid => {
                    assert_eq!(failure, CoreFailure::SessionRequired);
                    return;
                }
                _ => continue,
            }
        }
    }

    #[test]
    fn send_completion_trace_orders_terminal_before_and_after_binding() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        let coordinator = SharedSendCompletionCoordinator::default();
        let (ingress, _terminal_rx) = TimelineSendTerminalIngress::channel();
        let key = room_key();
        let mut owned_correlations = Vec::new();

        for (index, terminal_before_bind) in [true, false].into_iter().enumerate() {
            let mut registration = SendCompletionRegistration::begin(
                Arc::clone(&coordinator),
                ingress.clone(),
                key.clone(),
                format!("client-trace-{index}"),
                None,
                fake_rid(7400 + index as u64),
                true,
            );
            owned_correlations.push(
                registration
                    .lifecycle_trace
                    .as_ref()
                    .expect("registration owns a lifecycle trace")
                    .correlation(),
            );
            registration.activate();
            if terminal_before_bind {
                apply_send_completion_observation_and_handoff(
                    &coordinator,
                    &ingress,
                    key.room_id(),
                    SendCompletionObservation::Sent {
                        sdk_transaction_id: format!("sdk-trace-{index}"),
                        event_id: format!("$event-trace-{index}:test"),
                    },
                );
            }
            registration.bind(format!("sdk-trace-{index}"));
            if !terminal_before_bind {
                apply_send_completion_observation_and_handoff(
                    &coordinator,
                    &ingress,
                    key.room_id(),
                    SendCompletionObservation::Sent {
                        sdk_transaction_id: format!("sdk-trace-{index}"),
                        event_id: format!("$event-trace-{index}:test"),
                    },
                );
            }
        }

        let diagnostics = koushi_diagnostics::test_support::detail_snapshot();
        let records = diagnostics
            .records
            .iter()
            .filter(|record| {
                record.event.source == "core.send"
                    && record.event.fields.iter().any(|field| {
                        matches!(
                            field.value,
                            DiagnosticValue::Correlation(value)
                                if owned_correlations.contains(&value)
                        )
                    })
            })
            .collect::<Vec<_>>();
        let stages = records
            .iter()
            .map(|record| record.event.stage)
            .collect::<Vec<_>>();
        assert_eq!(
            stages,
            vec![
                "accepted",
                "sdk_enqueue_finished",
                "terminal_bound",
                "sdk_terminal_observed",
                "terminal_applied",
                "guard_released",
                "accepted",
                "sdk_enqueue_finished",
                "terminal_bound",
                "sdk_terminal_observed",
                "terminal_applied",
                "guard_released",
            ]
        );
        let correlations = records
            .chunks(6)
            .map(|trace| {
                trace
                    .iter()
                    .flat_map(|record| record.event.fields.iter())
                    .find(|field| field.key == "correlation")
                    .map(|field| field.value.clone())
            })
            .collect::<Vec<_>>();
        assert_eq!(correlations.len(), 2);
        assert!(correlations[0].is_some());
        assert!(correlations[1].is_some());
        assert_ne!(correlations[0], correlations[1]);
        for trace in records.chunks(6) {
            let trace_correlation = trace
                .iter()
                .flat_map(|record| record.event.fields.iter())
                .find(|field| field.key == "correlation")
                .map(|field| field.value.clone());
            assert!(trace.iter().all(|record| {
                record
                    .event
                    .fields
                    .iter()
                    .find(|field| field.key == "correlation")
                    .map(|field| field.value.clone())
                    == trace_correlation
            }));
            assert!(trace.iter().all(|record| {
                record.event.fields.iter().all(|field| {
                    !matches!(
                        field.key,
                        "room_id" | "event_id" | "user_id" | "transaction_id" | "request_id"
                    )
                })
            }));
        }
    }

    #[test]
    fn send_failure_trace_records_only_closed_failure_fields() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
            .records
            .len();
        let key = room_key();
        let mut trace = SendLifecycleTrace::new(&key, true);
        let correlation = trace.correlation();

        trace.stage_with_failure(
            "sdk_terminal_observed",
            Some("failed"),
            Some("immediate"),
            SendFailureDiagnostic {
                reason: "http",
                recoverable: true,
            },
        );

        let diagnostics = koushi_diagnostics::test_support::detail_snapshot();
        let event = &diagnostics.records[diagnostic_start..]
            .iter()
            .find(|record| {
                record.event.source == "core.send"
                    && record.event.stage == "sdk_terminal_observed"
                    && record.event.fields.iter().any(|field| {
                        field.key == "correlation"
                            && field.value == DiagnosticValue::Correlation(correlation)
                    })
            })
            .expect("send failure terminal diagnostic")
            .event;

        assert!(event.fields.iter().any(|field| {
            field.key == "reason" && field.value == DiagnosticValue::Token("http")
        }));
        assert!(event.fields.iter().any(|field| {
            field.key == "recoverable" && field.value == DiagnosticValue::Boolean(true)
        }));
        assert!(event.fields.iter().all(|field| {
            !matches!(
                field.key,
                "room_id"
                    | "event_id"
                    | "user_id"
                    | "device_id"
                    | "transaction_id"
                    | "endpoint"
                    | "error"
            )
        }));
    }

    #[test]
    fn encrypted_send_local_store_diagnostics_are_correlated_and_privacy_safe() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
            .records
            .len();
        let key = room_key();
        let trace = SendLifecycleTrace::new(&key, true);
        let correlation = trace.correlation();

        trace.record_encryption_local_store_snapshot(&EncryptedSendDiagnosticSnapshot {
            room_encryption: RoomEncryptionDiagnosticState::Encrypted,
            outbound_session_present: Some(true),
            own_user_tracking: OwnUserTrackingDiagnosticState::Tracked,
            own_device_present: Some(true),
            known_own_device_count: Some(4),
            known_own_other_device_count: Some(3),
            key_capable_own_other_device_count: Some(2),
            cross_signed_own_other_device_count: Some(2),
            dehydrated_own_other_device_count: Some(1),
            blacklisted_own_other_device_count: Some(1),
        });
        let diagnostics = koushi_diagnostics::test_support::detail_snapshot();
        let record = diagnostics.records[diagnostic_start..]
            .iter()
            .find(|record| {
                record.event.source == "core.send"
                    && record.event.stage == "encryption_local_store_snapshot"
                    && record.event.fields.iter().any(|field| {
                        field.key == "correlation"
                            && field.value == DiagnosticValue::Correlation(correlation)
                    })
            })
            .expect("encrypted-send snapshot diagnostic");

        for (key, value) in [
            ("room_encryption", DiagnosticValue::Token("encrypted")),
            ("recipient_strategy", DiagnosticValue::Token("all_devices")),
            (
                "snapshot_consistency",
                DiagnosticValue::Token("best_effort_concurrent_local_store"),
            ),
            ("outbound_session_present", DiagnosticValue::Boolean(true)),
            ("own_user_tracking", DiagnosticValue::Token("tracked")),
            ("own_device_present", DiagnosticValue::Boolean(true)),
            ("known_own_device_count", DiagnosticValue::Count(4)),
            ("known_own_other_device_count", DiagnosticValue::Count(3)),
            (
                "key_capable_own_other_device_count",
                DiagnosticValue::Count(2),
            ),
            (
                "cross_signed_own_other_device_count",
                DiagnosticValue::Count(2),
            ),
            (
                "dehydrated_own_other_device_count",
                DiagnosticValue::Count(1),
            ),
            (
                "blacklisted_own_other_device_count",
                DiagnosticValue::Count(1),
            ),
        ] {
            assert!(
                record
                    .event
                    .fields
                    .iter()
                    .any(|field| { field.key == key && field.value == value }),
                "missing {key}"
            );
        }
        assert!(record.event.fields.iter().all(|field| {
            !matches!(
                field.key,
                "room_id"
                    | "event_id"
                    | "user_id"
                    | "device_id"
                    | "session_id"
                    | "transaction_id"
                    | "request_id"
                    | "message"
                    | "key"
                    | "key_material"
            )
        }));
    }

    #[test]
    fn post_send_encryption_diagnostics_keep_unknown_state_and_session_evidence_separate() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
            .records
            .len();
        let correlation = 8_204;

        record_post_send_encryption_snapshot(
            correlation,
            RoomEncryptionDiagnosticState::Unknown,
            OutboundSessionLookupDiagnostic::Present,
        );

        let diagnostics = koushi_diagnostics::test_support::detail_snapshot();
        let record = diagnostics.records[diagnostic_start..]
            .iter()
            .find(|record| {
                record.event.source == "core.send"
                    && record.event.stage == "post_send_encryption_snapshot"
            })
            .expect("post-send encryption diagnostic");
        for (key, value) in [
            ("correlation", DiagnosticValue::Correlation(correlation)),
            (
                "room_encryption_cached_after_send",
                DiagnosticValue::Token("unknown"),
            ),
            ("outbound_session_lookup", DiagnosticValue::Token("present")),
            (
                "snapshot_consistency",
                DiagnosticValue::Token("best_effort_post_terminal_local_store"),
            ),
        ] {
            assert!(
                record
                    .event
                    .fields
                    .iter()
                    .any(|field| { field.key == key && field.value == value }),
                "missing {key}"
            );
        }
        assert!(record.event.fields.iter().all(|field| {
            !matches!(
                field.key,
                "room_id"
                    | "event_id"
                    | "user_id"
                    | "device_id"
                    | "session_id"
                    | "transaction_id"
                    | "request_id"
                    | "message"
                    | "key"
                    | "key_material"
            )
        }));
    }

    #[test]
    fn send_diagnostic_tasks_are_capacity_bounded_and_cancellable() {
        let _diagnostic_lock = koushi_diagnostics::test_support::lock();
        let diagnostic_start = koushi_diagnostics::test_support::detail_snapshot()
            .records
            .len();
        let (terminal_ingress, _terminal_rx) = TimelineSendTerminalIngress::channel();
        let mut supervisor = SendEnqueueWorkerSupervisor::new(terminal_ingress);

        for correlation in 1..=(MAX_CONCURRENT_SEND_DIAGNOSTICS as u64 + 1) {
            supervisor.spawn_diagnostic(correlation, futures_util::future::pending());
        }

        assert_eq!(
            supervisor.diagnostic_tasks.len(),
            MAX_CONCURRENT_SEND_DIAGNOSTICS
        );
        let diagnostics = koushi_diagnostics::test_support::detail_snapshot();
        assert!(
            diagnostics.records[diagnostic_start..]
                .iter()
                .any(|record| {
                    record.event.source == "core.send"
                        && record.event.stage == "diagnostic_snapshot_skipped"
                        && record.event.fields.iter().any(|field| {
                            field.key == "outcome"
                                && field.value == DiagnosticValue::Token("capacity_reached")
                        })
                })
        );

        supervisor.cancel_diagnostics();
        assert!(supervisor.diagnostic_tasks.is_empty());
    }

    #[tokio::test]
    async fn manager_coordinator_survives_unsubscribe_until_sdk_terminal() {
        let key = room_key();
        let mut manager = live_tail_test_manager(HashMap::from([(
            key.clone(),
            gap_demand_test_actor_handle("send-owner", Arc::new(Mutex::new(Vec::new()))),
        )]));
        let request_id = fake_rid(7410);
        let mut registration = SendCompletionRegistration::begin(
            Arc::clone(&manager.send_completion),
            manager.terminal_ingress.clone(),
            key.clone(),
            "client-unsubscribe-unit".to_owned(),
            None,
            request_id,
            true,
        );
        registration.activate();
        registration.bind("sdk-unsubscribe-unit".to_owned());

        manager
            .handle_command(TimelineCommand::Unsubscribe {
                request_id: fake_rid(7411),
                key: key.clone(),
            })
            .await;
        assert!(!manager.timelines.contains_key(&key));
        apply_send_completion_observation_and_handoff(
            &manager.send_completion,
            &manager.terminal_ingress,
            key.room_id(),
            SendCompletionObservation::Sent {
                sdk_transaction_id: "sdk-unsubscribe-unit".to_owned(),
                event_id: "$event-unsubscribe-unit:test".to_owned(),
            },
        );

        let handoff = manager
            .terminal_rx
            .recv()
            .await
            .expect("manager-owned completion after unsubscribe");
        assert!(matches!(
            handoff.completion,
            Some(TimelineSendCompletionDelivery {
                request_id: delivered_request_id,
                key: delivered_key,
                transaction_id,
                event_id,
                ..
            }) if delivered_request_id == request_id
                && delivered_key == key
                && transaction_id == "client-unsubscribe-unit"
                && event_id == "$event-unsubscribe-unit:test"
        ));
    }

    #[tokio::test]
    async fn manager_owned_prebind_enqueue_survives_room_and_thread_unsubscribe() {
        let account = AccountKey("@prebind-owner:test".to_owned());
        let keys = [
            TimelineKey::room(account.clone(), "!prebind-room:test"),
            TimelineKey {
                account_key: account,
                kind: TimelineKind::Thread {
                    room_id: "!prebind-room:test".to_owned(),
                    root_event_id: "$prebind-root:test".to_owned(),
                },
            },
        ];

        for (serial, key) in keys.into_iter().enumerate() {
            let mut manager = live_tail_test_manager(HashMap::from([(
                key.clone(),
                gap_demand_test_actor_handle("prebind", Arc::new(Mutex::new(Vec::new()))),
            )]));
            let request_id = fake_rid(7430 + serial as u64);
            let sdk_transaction_id = format!("sdk-prebind-{serial}");
            let mut registration = SendCompletionRegistration::begin(
                Arc::clone(&manager.send_completion),
                manager.terminal_ingress.clone(),
                key.clone(),
                format!("client-prebind-{serial}"),
                None,
                request_id,
                true,
            );
            registration.activate();
            let (durably_saved, saved) = oneshot::channel();
            let (release, released) = oneshot::channel();
            manager.spawn_send_enqueue_future(registration, async move {
                let _ = durably_saved.send(());
                let _ = released.await;
                Ok(SendEnqueueSuccess::terminal_only(sdk_transaction_id))
            });
            poll_manager_enqueue_workers_once(&mut manager).await;
            tokio::time::timeout(Duration::from_secs(1), saved)
                .await
                .expect("pre-bind enqueue worker must be driven")
                .expect("synthetic QueueStorage save committed");

            manager
                .handle_command(TimelineCommand::Unsubscribe {
                    request_id: fake_rid(7440 + serial as u64),
                    key: key.clone(),
                })
                .await;
            assert!(
                manager.terminal_rx.try_recv().is_err(),
                "actor removal must not abandon the manager-owned pre-bind registration"
            );

            let _ = release.send(());
            manager.join_send_enqueue_workers().await;
            apply_send_completion_observation_and_handoff(
                &manager.send_completion,
                &manager.terminal_ingress,
                key.room_id(),
                SendCompletionObservation::Sent {
                    sdk_transaction_id: format!("sdk-prebind-{serial}"),
                    event_id: format!("$event-prebind-{serial}:test"),
                },
            );
            let terminal = manager
                .terminal_rx
                .try_recv()
                .expect("correlated terminal after pre-bind unsubscribe");
            assert!(terminal.failure.is_none());
            assert!(matches!(
                terminal.completion,
                Some(TimelineSendCompletionDelivery {
                    request_id: completed_request_id,
                    key: completed_key,
                    ..
                }) if completed_request_id == request_id && completed_key == key
            ));
            assert!(manager.terminal_rx.try_recv().is_err());
        }
    }

    #[tokio::test]
    async fn manager_drop_aborts_owned_observer_and_send_enqueue_workers() {
        struct OwnedTaskDropFlag(Arc<AtomicBool>);

        impl Drop for OwnedTaskDropFlag {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }

        let mut manager = live_tail_test_manager(HashMap::new());
        let observer_alive = Arc::new(AtomicBool::new(true));
        manager.global_send_completion_observer_future = Some(Box::pin({
            let observer_drop = OwnedTaskDropFlag(Arc::clone(&observer_alive));
            async move {
                let _drop = observer_drop;
                futures_util::future::pending::<()>().await;
            }
        }));
        let mut registration = SendCompletionRegistration::begin(
            Arc::clone(&manager.send_completion),
            manager.terminal_ingress.clone(),
            room_key(),
            "client-manager-drop".to_owned(),
            None,
            fake_rid(7465),
            true,
        );
        registration.activate();
        let worker_alive = Arc::new(AtomicBool::new(true));
        manager.spawn_send_enqueue_future(registration, {
            let worker_drop = OwnedTaskDropFlag(Arc::clone(&worker_alive));
            async move {
                let _drop = worker_drop;
                futures_util::future::pending::<Result<SendEnqueueSuccess, TimelineFailureKind>>()
                    .await
            }
        });
        drop(manager);

        assert!(
            !observer_alive.load(Ordering::SeqCst),
            "dropping the manager must synchronously drop the owned observer future"
        );
        assert!(
            !worker_alive.load(Ordering::SeqCst),
            "dropping the manager must synchronously drop every owned enqueue future"
        );

        let quiesced = tokio::time::timeout(Duration::from_secs(1), async {
            while observer_alive.load(Ordering::SeqCst) || worker_alive.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await;
        assert!(
            quiesced.is_ok(),
            "unexpected manager drop must quiesce observer={} worker={}",
            !observer_alive.load(Ordering::SeqCst),
            !worker_alive.load(Ordering::SeqCst),
        );
    }

    #[tokio::test]
    async fn panicked_enqueue_future_is_fail_closed_without_stopping_manager_workers() {
        let mut manager = live_tail_test_manager(HashMap::new());
        let mut panicked_registration = SendCompletionRegistration::begin(
            Arc::clone(&manager.send_completion),
            manager.terminal_ingress.clone(),
            room_key(),
            "client-panicked-enqueue".to_owned(),
            None,
            fake_rid(7466),
            true,
        );
        panicked_registration.activate();
        manager.spawn_send_enqueue_future(panicked_registration, async move {
            panic!("synthetic enqueue panic");
            #[allow(unreachable_code)]
            Err(TimelineFailureKind::Sdk)
        });

        let completion = manager
            .send_enqueue_workers
            .tasks
            .next()
            .await
            .expect("caught panic still settles the supervised future");
        manager.handle_send_enqueue_worker_completion(completion);
        let panic_terminal = manager
            .terminal_rx
            .try_recv()
            .expect("registration drop emits one private-safe terminal");
        assert!(matches!(
            panic_terminal.failure,
            Some(TimelineSendFailureDelivery {
                failure: CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::QueueOverflow,
                },
                ..
            })
        ));

        let mut next_registration = SendCompletionRegistration::begin(
            Arc::clone(&manager.send_completion),
            manager.terminal_ingress.clone(),
            room_key(),
            "client-after-panic".to_owned(),
            None,
            fake_rid(7467),
            true,
        );
        next_registration.activate();
        manager.spawn_send_enqueue_future(next_registration, async move {
            Err(TimelineFailureKind::Sdk)
        });
        let completion = manager
            .send_enqueue_workers
            .tasks
            .next()
            .await
            .expect("manager continues polling workers after an isolated panic");
        manager.handle_send_enqueue_worker_completion(completion);
        let next_terminal = manager
            .terminal_rx
            .try_recv()
            .expect("later worker terminal is still delivered");
        assert!(matches!(
            next_terminal.failure,
            Some(TimelineSendFailureDelivery {
                failure: CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::Sdk,
                },
                ..
            })
        ));
        assert!(manager.terminal_rx.try_recv().is_err());
    }

    #[test]
    fn manager_coordinator_keeps_same_room_room_and_thread_keys_collision_safe() {
        let account = AccountKey("@send-owner:test".to_owned());
        let room_key = TimelineKey::room(account.clone(), "!shared-room:test");
        let thread_key = TimelineKey {
            account_key: account,
            kind: TimelineKind::Thread {
                room_id: "!shared-room:test".to_owned(),
                root_event_id: "$thread-root:test".to_owned(),
            },
        };
        let coordinator = SharedSendCompletionCoordinator::default();
        let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
        let mut room_registration = SendCompletionRegistration::begin(
            Arc::clone(&coordinator),
            ingress.clone(),
            room_key.clone(),
            "client-room".to_owned(),
            None,
            fake_rid(7420),
            true,
        );
        room_registration.activate();
        room_registration.bind("sdk-room".to_owned());
        let mut thread_registration = SendCompletionRegistration::begin(
            Arc::clone(&coordinator),
            ingress.clone(),
            thread_key.clone(),
            "client-thread".to_owned(),
            None,
            fake_rid(7421),
            true,
        );
        thread_registration.activate();
        thread_registration.bind("sdk-thread".to_owned());

        for (sdk_transaction_id, event_id) in [
            ("sdk-thread", "$event-thread:test"),
            ("sdk-room", "$event-room:test"),
        ] {
            apply_send_completion_observation_and_handoff(
                &coordinator,
                &ingress,
                "!shared-room:test",
                SendCompletionObservation::Sent {
                    sdk_transaction_id: sdk_transaction_id.to_owned(),
                    event_id: event_id.to_owned(),
                },
            );
        }

        let thread_handoff = terminal_rx.try_recv().expect("thread terminal first");
        let room_handoff = terminal_rx.try_recv().expect("room terminal second");
        assert!(matches!(
            thread_handoff.completion,
            Some(TimelineSendCompletionDelivery { key, .. }) if key == thread_key
        ));
        assert!(matches!(
            room_handoff.completion,
            Some(TimelineSendCompletionDelivery { key, .. }) if key == room_key
        ));
    }

    #[test]
    fn unmatched_terminal_cohort_overflow_fails_safe_once_without_unbounded_growth() {
        let key = room_key();
        let coordinator = SharedSendCompletionCoordinator::default();
        let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
        let mut registration = SendCompletionRegistration::begin(
            Arc::clone(&coordinator),
            ingress.clone(),
            key.clone(),
            "client-cohort".to_owned(),
            None,
            fake_rid(7430),
            true,
        );
        registration.activate();

        apply_send_completion_observation_and_handoff(
            &coordinator,
            &ingress,
            key.room_id(),
            SendCompletionObservation::Sent {
                sdk_transaction_id: "sdk-cohort-candidate".to_owned(),
                event_id: "$event-cohort-candidate:test".to_owned(),
            },
        );
        apply_send_completion_observation_and_handoff(
            &coordinator,
            &ingress,
            key.room_id(),
            SendCompletionObservation::Sent {
                sdk_transaction_id: "sdk-cohort-overflow".to_owned(),
                event_id: "$event-cohort-overflow:test".to_owned(),
            },
        );
        let overflow = terminal_rx.try_recv().expect("cohort overflow terminal");
        assert!(matches!(
            overflow.failure,
            Some(TimelineSendFailureDelivery {
                request_id,
                failure: CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::QueueOverflow,
                },
            }) if request_id == fake_rid(7430)
        ));
        assert_eq!(
            coordinator
                .lock()
                .expect("send completion coordinator")
                .unmatched_terminals
                .len(),
            1,
            "one active unbound registration admits only one unmatched transaction cohort"
        );

        apply_send_completion_observation_and_handoff(
            &coordinator,
            &ingress,
            key.room_id(),
            SendCompletionObservation::Sent {
                sdk_transaction_id: "sdk-cohort-overflow-again".to_owned(),
                event_id: "$event-cohort-overflow-again:test".to_owned(),
            },
        );
        assert!(
            terminal_rx.try_recv().is_err(),
            "cohort overflow failure must be reported once per active request"
        );

        registration.bind("sdk-cohort-candidate".to_owned());
        let completion = terminal_rx.try_recv().expect("retained exact terminal");
        assert!(completion.failure.is_none());
        assert!(matches!(
            completion.completion,
            Some(TimelineSendCompletionDelivery { key: delivered_key, .. })
                if delivered_key == key
        ));
    }

    #[test]
    fn known_enqueue_failure_and_active_registration_abort_have_distinct_terminals() {
        let key = room_key();
        let coordinator = SharedSendCompletionCoordinator::default();
        let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
        let mut known_failure = SendCompletionRegistration::begin(
            Arc::clone(&coordinator),
            ingress.clone(),
            key.clone(),
            "client-known-failure".to_owned(),
            None,
            fake_rid(7435),
            true,
        );
        known_failure.activate();
        known_failure.fail_known(TimelineFailureKind::Forbidden);
        drop(known_failure);
        assert!(matches!(
            terminal_rx.try_recv().expect("known failure").failure,
            Some(TimelineSendFailureDelivery {
                failure: CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::Forbidden,
                },
                ..
            })
        ));
        assert!(terminal_rx.try_recv().is_err());

        let mut abandoned = SendCompletionRegistration::begin(
            Arc::clone(&coordinator),
            ingress,
            key,
            "client-abandoned".to_owned(),
            None,
            fake_rid(7436),
            true,
        );
        abandoned.activate();
        drop(abandoned);
        assert!(matches!(
            terminal_rx.try_recv().expect("abandoned failure").failure,
            Some(TimelineSendFailureDelivery {
                failure: CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::QueueOverflow,
                },
                ..
            })
        ));
        assert!(terminal_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn global_send_observer_lag_fails_bound_and_unbound_in_registration_order() {
        let account = AccountKey("@lag-owner:test".to_owned());
        let first_key = TimelineKey::room(account.clone(), "!lag-room:test");
        let second_key = TimelineKey {
            account_key: account,
            kind: TimelineKind::Focused {
                room_id: "!lag-room:test".to_owned(),
                event_id: "$focus:test".to_owned(),
            },
        };
        let coordinator = SharedSendCompletionCoordinator::default();
        let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
        let mut first = SendCompletionRegistration::begin(
            Arc::clone(&coordinator),
            ingress.clone(),
            first_key,
            "client-lag-first".to_owned(),
            None,
            fake_rid(7440),
            true,
        );
        first.activate();
        first.bind("sdk-lag-first".to_owned());
        let mut second = SendCompletionRegistration::begin(
            Arc::clone(&coordinator),
            ingress.clone(),
            second_key,
            "client-lag-second".to_owned(),
            None,
            fake_rid(7441),
            false,
        );
        second.activate();

        let (updates_tx, updates_rx) = broadcast::channel(1);
        let room_id =
            matrix_sdk::ruma::OwnedRoomId::try_from("!lag-room:test").expect("lag room id");
        for transaction_id in ["sdk-overflow-one", "sdk-overflow-two"] {
            updates_tx
                .send(matrix_sdk::send_queue::SendQueueUpdate {
                    room_id: room_id.clone(),
                    update: RoomSendQueueUpdate::RetryEvent {
                        transaction_id: matrix_sdk::ruma::OwnedTransactionId::from(transaction_id),
                    },
                })
                .expect("queue lag update");
        }
        drop(updates_tx);
        run_global_send_completion_observer(updates_rx, Arc::clone(&coordinator), ingress.clone())
            .await;

        let first_failure = terminal_rx.try_recv().expect("first lag failure");
        let second_failure = terminal_rx.try_recv().expect("second lag failure");
        assert!(matches!(
            first_failure.failure,
            Some(TimelineSendFailureDelivery { request_id, .. }) if request_id == fake_rid(7440)
        ));
        assert!(first_failure.action.is_some());
        assert!(matches!(
            second_failure.failure,
            Some(TimelineSendFailureDelivery { request_id, .. }) if request_id == fake_rid(7441)
        ));
        assert!(second_failure.action.is_none());
        assert!(terminal_rx.try_recv().is_err());

        apply_send_completion_observation_loss_and_handoff(&coordinator, &ingress, None);
        assert!(
            terminal_rx.try_recv().is_err(),
            "a repeated lag notification must not report either request twice"
        );
        second.bind("sdk-lag-second".to_owned());
        apply_send_completion_observation_and_handoff(
            &coordinator,
            &ingress,
            "!lag-room:test",
            SendCompletionObservation::Sent {
                sdk_transaction_id: "sdk-lag-second".to_owned(),
                event_id: "$event-after-lag:test".to_owned(),
            },
        );
        let recovered = terminal_rx.try_recv().expect("exact terminal after lag");
        assert!(recovered.action.is_none());
        assert!(recovered.failure.is_none());
        assert!(recovered.completion.is_some());
    }

    #[tokio::test]
    async fn shutdown_joins_observer_then_actor_and_drains_registration_failure_before_ack() {
        struct OrderedDrop {
            label: &'static str,
            log: Arc<Mutex<Vec<&'static str>>>,
        }

        impl Drop for OrderedDrop {
            fn drop(&mut self) {
                self.log
                    .lock()
                    .expect("shutdown ordering log")
                    .push(self.label);
            }
        }

        let key = room_key();
        let mut manager = live_tail_test_manager(HashMap::new());
        let (action_tx, mut action_rx) = mpsc::channel(8);
        manager.action_tx = action_tx;
        let order = Arc::new(Mutex::new(Vec::new()));
        let (observer_started_tx, observer_started_rx) = oneshot::channel();
        manager.global_send_completion_observer_future = Some(Box::pin({
            let order = Arc::clone(&order);
            async move {
                let _drop = OrderedDrop {
                    label: "observer",
                    log: order,
                };
                let _ = observer_started_tx.send(());
                std::future::pending::<()>().await;
            }
        }));

        let mut registration = SendCompletionRegistration::begin(
            Arc::clone(&manager.send_completion),
            manager.terminal_ingress.clone(),
            key.clone(),
            "client-shutdown-order".to_owned(),
            None,
            fake_rid(7450),
            true,
        );
        registration.activate();
        let (actor_started_tx, actor_started_rx) = oneshot::channel();
        let (actor_tx, _actor_rx) = mpsc::channel(1);
        let actor_task = executor::spawn({
            let order = Arc::clone(&order);
            async move {
                let _drop = OrderedDrop {
                    label: "actor",
                    log: order,
                };
                let _registration = registration;
                let _ = actor_started_tx.send(());
                std::future::pending::<()>().await;
            }
        });
        manager.timelines.insert(
            key,
            TimelineActorHandle {
                tx: actor_tx,
                control_tx: None,
                thread_summary_projection:
                    crate::timeline::actor::ThreadSummaryProjectionIngress::channel().0,
                position_rx: None,
                task: Some(actor_task),
                auxiliary_tasks: Vec::new(),
                subscription_generation: None,
                enqueue_context: None,
            },
        );
        let manager_tx = manager.msg_tx.clone();
        let manager_task = executor::spawn(manager.run());
        observer_started_rx.await.expect("observer started");
        actor_started_rx.await.expect("actor started");

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        manager_tx
            .send(TimelineMessage::Shutdown {
                acknowledged: Some(shutdown_tx),
            })
            .await
            .expect("shutdown command");
        shutdown_rx.await.expect("shutdown acknowledgement");
        manager_task.await.expect("manager shutdown task");

        let observed_order = order.lock().expect("shutdown ordering log").clone();
        assert_eq!(
            observed_order,
            ["observer", "actor"],
            "the sole observer must stop before actor registration producers"
        );
        let mut send_failure_count = 0;
        while let Ok(actions) = action_rx.try_recv() {
            send_failure_count += actions
                .iter()
                .filter(|action| {
                    matches!(
                        action,
                        AppAction::SendTextFailed { transaction_id, .. }
                            if transaction_id == "client-shutdown-order"
                    )
                })
                .count();
        }
        assert_eq!(
            send_failure_count, 1,
            "shutdown must drain one fail-safe terminal"
        );
    }

    #[test]
    fn coordinator_maps_sdk_transaction_to_client_request_and_completion() {
        let key = room_key();
        let coordinator = SharedSendCompletionCoordinator::default();
        let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
        let mut registration = SendCompletionRegistration::begin(
            Arc::clone(&coordinator),
            ingress.clone(),
            key.clone(),
            "client-txn-42".to_owned(),
            None,
            fake_rid(42),
            true,
        );
        let diagnostic_correlation = registration
            .lifecycle_trace
            .as_ref()
            .expect("send registration lifecycle trace")
            .correlation();
        registration.activate();
        registration.bind("sdk-auto-generated-txn".to_owned());

        assert_eq!(
            coordinator
                .lock()
                .expect("send completion coordinator")
                .pending_send(key.room_id(), "sdk-auto-generated-txn"),
            Some((&key, "client-txn-42", fake_rid(42)))
        );
        apply_send_completion_observation_and_handoff(
            &coordinator,
            &ingress,
            key.room_id(),
            SendCompletionObservation::Sent {
                sdk_transaction_id: "sdk-auto-generated-txn".to_owned(),
                event_id: "$event-42:test".to_owned(),
            },
        );
        assert!(matches!(
            terminal_rx.try_recv().expect("mapped completion").completion,
            Some(TimelineSendCompletionDelivery {
                request_id,
                transaction_id,
                event_id,
                diagnostic_correlation: Some(delivered_correlation),
                ..
            }) if request_id == fake_rid(42)
                && transaction_id == "client-txn-42"
                && event_id == "$event-42:test"
                && delivered_correlation == diagnostic_correlation
        ));
    }

    #[test]
    fn send_completion_race_delivers_completion_when_sent_event_arrives_first() {
        let key = room_key();
        let coordinator = SharedSendCompletionCoordinator::default();
        let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
        let mut registration = SendCompletionRegistration::begin(
            Arc::clone(&coordinator),
            ingress.clone(),
            key.clone(),
            "client-race-txn".to_owned(),
            None,
            fake_rid(77),
            true,
        );
        registration.activate();
        apply_send_completion_observation_and_handoff(
            &coordinator,
            &ingress,
            key.room_id(),
            SendCompletionObservation::Sent {
                sdk_transaction_id: "sdk-race-txn".to_owned(),
                event_id: "$event-race:test".to_owned(),
            },
        );
        assert!(terminal_rx.try_recv().is_err());

        registration.bind("sdk-race-txn".to_owned());
        assert!(matches!(
            terminal_rx.try_recv().expect("early completion correlated").completion,
            Some(TimelineSendCompletionDelivery {
                request_id,
                transaction_id,
                event_id,
                ..
            }) if request_id == fake_rid(77)
                && transaction_id == "client-race-txn"
                && event_id == "$event-race:test"
        ));
    }

    #[test]
    fn replacement_owner_preserves_pending_send_completion_correlation() {
        let key = room_key();
        let current_owner = SharedSendCompletionCoordinator::default();
        let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
        let mut registration = SendCompletionRegistration::begin(
            Arc::clone(&current_owner),
            ingress.clone(),
            key.clone(),
            "client-owner-handoff-txn".to_owned(),
            None,
            fake_rid(773),
            true,
        );
        registration.activate();
        registration.bind("sdk-owner-handoff-txn".to_owned());

        let replacement_owner = Arc::clone(&current_owner);
        drop(registration);
        drop(current_owner);
        apply_send_completion_observation_and_handoff(
            &replacement_owner,
            &ingress,
            key.room_id(),
            SendCompletionObservation::Sent {
                sdk_transaction_id: "sdk-owner-handoff-txn".to_owned(),
                event_id: "$event-owner-handoff:test".to_owned(),
            },
        );
        assert!(matches!(
            terminal_rx.try_recv().expect("replacement completion").completion,
            Some(TimelineSendCompletionDelivery {
                request_id,
                transaction_id,
                event_id,
                ..
            }) if request_id == fake_rid(773)
                && transaction_id == "client-owner-handoff-txn"
                && event_id == "$event-owner-handoff:test"
        ));
    }

    #[test]
    fn duplicate_sent_event_after_completion_is_idempotent() {
        let key = room_key();
        let coordinator = SharedSendCompletionCoordinator::default();
        let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
        let mut registration = SendCompletionRegistration::begin(
            Arc::clone(&coordinator),
            ingress.clone(),
            key.clone(),
            "client-duplicate-txn".to_owned(),
            None,
            fake_rid(770),
            true,
        );
        registration.activate();
        registration.bind("sdk-duplicate-txn".to_owned());
        for _ in 0..2 {
            apply_send_completion_observation_and_handoff(
                &coordinator,
                &ingress,
                key.room_id(),
                SendCompletionObservation::Sent {
                    sdk_transaction_id: "sdk-duplicate-txn".to_owned(),
                    event_id: "$event-duplicate:test".to_owned(),
                },
            );
        }

        assert!(
            terminal_rx
                .try_recv()
                .expect("first completion")
                .completion
                .is_some()
        );
        assert!(
            terminal_rx.try_recv().is_err(),
            "an overlapping observer must not emit twice"
        );
    }

    #[test]
    fn sent_event_before_pending_race_remains_idempotent_after_settlement() {
        let key = room_key();
        let coordinator = SharedSendCompletionCoordinator::default();
        let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
        let mut registration = SendCompletionRegistration::begin(
            Arc::clone(&coordinator),
            ingress.clone(),
            key.clone(),
            "client-early-duplicate-txn".to_owned(),
            None,
            fake_rid(771),
            true,
        );
        registration.activate();
        apply_send_completion_observation_and_handoff(
            &coordinator,
            &ingress,
            key.room_id(),
            SendCompletionObservation::Sent {
                sdk_transaction_id: "sdk-early-duplicate-txn".to_owned(),
                event_id: "$event-early-duplicate:test".to_owned(),
            },
        );
        registration.bind("sdk-early-duplicate-txn".to_owned());
        assert!(
            terminal_rx
                .try_recv()
                .expect("early completion")
                .completion
                .is_some()
        );

        apply_send_completion_observation_and_handoff(
            &coordinator,
            &ingress,
            key.room_id(),
            SendCompletionObservation::Sent {
                sdk_transaction_id: "sdk-early-duplicate-txn".to_owned(),
                event_id: "$event-early-duplicate:test".to_owned(),
            },
        );
        assert!(terminal_rx.try_recv().is_err());
    }

    #[test]
    fn cancelled_completion_is_tombstoned_against_late_sent_event() {
        let key = room_key();
        let coordinator = SharedSendCompletionCoordinator::default();
        let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
        let mut registration = SendCompletionRegistration::begin(
            Arc::clone(&coordinator),
            ingress.clone(),
            key.clone(),
            "client-cancelled-txn".to_owned(),
            None,
            fake_rid(772),
            true,
        );
        registration.activate();
        registration.bind("sdk-cancelled-txn".to_owned());
        apply_send_completion_observation_and_handoff(
            &coordinator,
            &ingress,
            key.room_id(),
            SendCompletionObservation::Cancelled {
                sdk_transaction_id: "sdk-cancelled-txn".to_owned(),
            },
        );
        assert!(
            terminal_rx
                .try_recv()
                .expect("cancel terminal")
                .action
                .is_some()
        );

        apply_send_completion_observation_and_handoff(
            &coordinator,
            &ingress,
            key.room_id(),
            SendCompletionObservation::Sent {
                sdk_transaction_id: "sdk-cancelled-txn".to_owned(),
                event_id: "$late-event:test".to_owned(),
            },
        );
        assert!(terminal_rx.try_recv().is_err());
        assert!(
            coordinator
                .lock()
                .expect("send completion coordinator")
                .settled_send_tombstones
                .contains(&SendCorrelationKey {
                    room_id: key.room_id().to_owned(),
                    sdk_transaction_id: "sdk-cancelled-txn".to_owned(),
                })
        );
    }

    #[test]
    fn unmatched_early_send_completions_survive_beyond_tombstone_history_bound() {
        let key = room_key();
        let coordinator = SharedSendCompletionCoordinator::default();
        let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
        let observed = MAX_SETTLED_SEND_TOMBSTONES + 64;
        let mut registrations = Vec::with_capacity(observed);
        for index in 0..observed {
            let mut registration = SendCompletionRegistration::begin(
                Arc::clone(&coordinator),
                ingress.clone(),
                key.clone(),
                format!("client-early-{index}"),
                None,
                fake_rid(900 + index as u64),
                true,
            );
            registration.activate();
            registrations.push(registration);
        }
        for index in 0..observed {
            apply_send_completion_observation_and_handoff(
                &coordinator,
                &ingress,
                key.room_id(),
                SendCompletionObservation::Sent {
                    sdk_transaction_id: format!("sdk-early-{index}"),
                    event_id: format!("$event-early-{index}:test"),
                },
            );
        }
        assert_eq!(
            coordinator
                .lock()
                .expect("send completion coordinator")
                .unmatched_terminals
                .len(),
            observed,
            "active unmatched correlations are not tombstone history and must not be evicted"
        );

        registrations[0].bind("sdk-early-0".to_owned());
        assert!(matches!(
            terminal_rx.try_recv().expect("oldest early completion").completion,
            Some(TimelineSendCompletionDelivery {
                request_id,
                event_id,
                ..
            }) if request_id == fake_rid(900) && event_id == "$event-early-0:test"
        ));
    }

    #[test]
    fn settled_send_tombstones_are_bounded() {
        let key = room_key();
        let coordinator = SharedSendCompletionCoordinator::default();
        let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
        for index in 0..=MAX_SETTLED_SEND_TOMBSTONES {
            let sdk_transaction_id = format!("sdk-bounded-{index}");
            let mut registration = SendCompletionRegistration::begin(
                Arc::clone(&coordinator),
                ingress.clone(),
                key.clone(),
                format!("client-bounded-{index}"),
                None,
                fake_rid(1200 + index as u64),
                true,
            );
            registration.activate();
            registration.bind(sdk_transaction_id.clone());
            apply_send_completion_observation_and_handoff(
                &coordinator,
                &ingress,
                key.room_id(),
                SendCompletionObservation::Sent {
                    sdk_transaction_id,
                    event_id: format!("$event-bounded-{index}:test"),
                },
            );
            assert!(
                terminal_rx
                    .try_recv()
                    .expect("bounded completion")
                    .completion
                    .is_some()
            );
        }

        let first = SendCorrelationKey {
            room_id: key.room_id().to_owned(),
            sdk_transaction_id: "sdk-bounded-0".to_owned(),
        };
        let newest = SendCorrelationKey {
            room_id: key.room_id().to_owned(),
            sdk_transaction_id: format!("sdk-bounded-{MAX_SETTLED_SEND_TOMBSTONES}"),
        };
        let coordinator_guard = coordinator.lock().expect("send completion coordinator");
        assert_eq!(
            coordinator_guard.settled_send_tombstones.len(),
            MAX_SETTLED_SEND_TOMBSTONES
        );
        assert!(!coordinator_guard.settled_send_tombstones.contains(&first));
        assert!(coordinator_guard.settled_send_tombstones.contains(&newest));
        drop(coordinator_guard);

        apply_send_completion_observation_and_handoff(
            &coordinator,
            &ingress,
            key.room_id(),
            SendCompletionObservation::Sent {
                sdk_transaction_id: newest.sdk_transaction_id,
                event_id: "$duplicate:test".to_owned(),
            },
        );
        assert!(terminal_rx.try_recv().is_err());
    }

    #[test]
    fn send_completion_coordinator_preserves_submission_id_for_terminal_paths() {
        let key = room_key();
        let coordinator = SharedSendCompletionCoordinator::default();
        let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
        let submission_id = SubmissionId::new("submission-terminal");
        let mut registration = SendCompletionRegistration::begin(
            Arc::clone(&coordinator),
            ingress.clone(),
            key.clone(),
            "client-submission-terminal".to_owned(),
            Some(submission_id.clone()),
            fake_rid(7400),
            true,
        );
        registration.activate();
        registration.bind("sdk-submission-terminal".to_owned());

        apply_send_completion_observation_and_handoff(
            &coordinator,
            &ingress,
            key.room_id(),
            SendCompletionObservation::SendError {
                sdk_transaction_id: "sdk-submission-terminal".to_owned(),
                diagnostic: SendFailureDiagnostic {
                    reason: "http",
                    recoverable: true,
                },
            },
        );
        let failure = terminal_rx.try_recv().expect("submission send error");
        assert!(matches!(
            failure.action,
            Some(AppAction::ComposerSubmissionSettled {
                submission_id: found,
                ..
            }) if found == submission_id
        ));
        apply_send_completion_observation_and_handoff(
            &coordinator,
            &ingress,
            key.room_id(),
            SendCompletionObservation::Cancelled {
                sdk_transaction_id: "sdk-submission-terminal".to_owned(),
            },
        );
        let cancelled = terminal_rx.try_recv().expect("submission cancellation");
        assert!(cancelled.action.is_none());
        assert!(cancelled.completion.is_none());
    }

    #[test]
    fn media_pending_send_does_not_settle_text_composer() {
        let key = room_key();
        let coordinator = SharedSendCompletionCoordinator::default();
        let (ingress, mut terminal_rx) = TimelineSendTerminalIngress::channel();
        let mut registration = SendCompletionRegistration::begin(
            Arc::clone(&coordinator),
            ingress.clone(),
            key.clone(),
            "client-media-txn".to_owned(),
            None,
            fake_rid(78),
            false,
        );
        registration.activate();
        registration.bind("sdk-media-txn".to_owned());
        apply_send_completion_observation_and_handoff(
            &coordinator,
            &ingress,
            key.room_id(),
            SendCompletionObservation::Sent {
                sdk_transaction_id: "sdk-media-txn".to_owned(),
                event_id: "$event-media:test".to_owned(),
            },
        );
        let terminal = terminal_rx.try_recv().expect("media completion");
        assert!(terminal.action.is_none());
        assert!(terminal.completion.is_some());
    }

    #[test]
    fn timeline_send_error_classifies_not_joined_as_forbidden() {
        let error = matrix_sdk_ui::timeline::Error::SendQueueError(
            matrix_sdk::send_queue::RoomSendQueueError::RoomNotJoined,
        );

        assert_eq!(
            classify_timeline_send_error(&error),
            TimelineFailureKind::Forbidden
        );
    }

    #[test]
    fn same_room_thread_media_progress_does_not_borrow_room_request_correlation() {
        let account = AccountKey("@media-progress:test".to_owned());
        let room_key = TimelineKey::room(account.clone(), "!media-progress:test");
        let thread_key = TimelineKey {
            account_key: account,
            kind: TimelineKind::Thread {
                room_id: "!media-progress:test".to_owned(),
                root_event_id: "$media-root:test".to_owned(),
            },
        };
        let coordinator = SharedSendCompletionCoordinator::default();
        let (ingress, _terminal_rx) = TimelineSendTerminalIngress::channel();
        let mut registration = SendCompletionRegistration::begin(
            Arc::clone(&coordinator),
            ingress,
            room_key.clone(),
            "client-media-progress".to_owned(),
            None,
            fake_rid(7424),
            false,
        );
        registration.activate();
        registration.bind("sdk-media-progress".to_owned());

        assert_eq!(
            media_upload_progress_identity(&coordinator, &room_key, "sdk-media-progress"),
            ("client-media-progress".to_owned(), Some(fake_rid(7424)))
        );
        assert_eq!(
            media_upload_progress_identity(&coordinator, &thread_key, "sdk-media-progress"),
            ("sdk-media-progress".to_owned(), None),
            "same-room thread presentation must not borrow room request correlation"
        );
    }
}
