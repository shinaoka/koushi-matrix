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
use crate::composer_draft_lifecycle::ForwardedComposerDraftPermit;
use crate::executor;
use koushi_protocol::command::UploadMediaRequest;
use koushi_protocol::event::{
    CoreEvent, TimelineEvent, TimelineItem, TimelineItemId, TimelineSendState,
};
use koushi_protocol::failure::{CoreFailure, TimelineFailureKind};
use koushi_protocol::ids::{RequestId, TimelineKey, TimelineKind};

// BEGIN GENERATED SIBLING IMPORTS
use super::actor::{
    TimelineActor, TimelineActorCleanupIngress, TimelineActorMessage, emit_app_action_reliable,
};
use super::composer::{
    build_room_message_content_from_composer_document_with_options,
    build_room_message_content_without_relation_from_composer_document_with_options,
    media_caption_content_from_draft, ruma_mentions_from_intent,
};
use super::diagnostics::trace_timeline_items;
use super::display_projection::{DisplayProjectionContext, DisplayProjectionState};
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
use super::thread_projection::{ThreadAttentionBatchProvenance, ThreadAttentionCounters};
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
}

pub(super) struct TimelineSendFailureDelivery {
    pub(super) request_id: RequestId,
    pub(super) failure: CoreFailure,
}

/// Internal payload accepted only through the manager-owned terminal ingress.
/// Replaceable timeline actors cannot deliver reducer actions and completion
/// events independently.
pub(super) struct TimelineSendTerminalHandoff {
    pub(super) key: Option<TimelineKey>,
    hydration: Option<PendingSendHydration>,
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
    let client = context.session.client();
    let Some(own_user_id) = client.user_id().map(ToOwned::to_owned) else {
        return EncryptedSendDiagnosticSnapshot {
            room_encryption,
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
    handle: Option<matrix_sdk::send_queue::SendHandle>,
    media_queued: Option<MediaSendQueuedDelivery>,
}

impl SendEnqueueSuccess {
    fn terminal_only(sdk_transaction_id: String) -> Self {
        Self {
            sdk_transaction_id,
            handle: None,
            media_queued: None,
        }
    }
}

pub(super) struct SendEnqueueWorkerCompletion {
    pub(super) changed_key: Option<TimelineKey>,
    hydration: Option<(SendCorrelationKey, Option<TimelineItem>)>,
    deferred_refresh: Option<(TimelineKey, u64)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PendingSendPhase {
    Pending,
    FailedRecoverable,
    SentAwaitingRemote,
    HydratedSent,
}

#[derive(Clone)]
pub(super) struct PendingSendProjection {
    pub(super) key: TimelineKey,
    pub(super) sequence: u64,
    pub(super) client_txn_id: String,
    pub(super) item: TimelineItem,
    pub(super) sdk_transaction_id: Option<String>,
    pub(super) handle: Option<matrix_sdk::send_queue::SendHandle>,
    pub(super) terminal_event_id: Option<String>,
    pub(super) phase: PendingSendPhase,
}

#[derive(Clone)]
pub(super) struct PendingSendHydration {
    correlation: SendCorrelationKey,
    key: TimelineKey,
    event_id: String,
}

pub(super) fn pending_send_item(
    transaction_id: &str,
    body: &str,
    in_reply_to_event_id: Option<String>,
    thread_root: Option<String>,
    own_user_id: Option<&str>,
) -> TimelineItem {
    TimelineItem {
        id: TimelineItemId::Transaction {
            transaction_id: transaction_id.to_owned(),
        },
        sender: own_user_id.map(str::to_owned),
        sender_label: own_user_id.map(str::to_owned),
        sender_avatar: None,
        body: Some(body.to_owned()),
        notice_i18n: None,
        message_kind: Default::default(),
        spoiler_spans: Vec::new(),
        timestamp_ms: Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_millis().try_into().unwrap_or(u64::MAX))
                .unwrap_or_default(),
        ),
        in_reply_to_event_id,
        formatted: None,
        reply_quote: None,
        thread_root,
        thread_summary: None,
        media: None,
        link_previews: None,
        link_ranges: Vec::new(),
        reactions: Vec::new(),
        can_react: false,
        is_redacted: false,
        is_hidden: false,
        can_redact: false,
        is_edited: false,
        can_edit: false,
        unable_to_decrypt: None,
        request_state: None,
        actions: Default::default(),
        send_state: Some(TimelineSendState::Sending),
        display_metadata: None,
    }
}

type SendEnqueueWorkerFuture =
    Pin<Box<dyn Future<Output = SendEnqueueWorkerCompletion> + Send + 'static>>;

pub(super) type GlobalSendCompletionObserverFuture =
    Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

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
    deferred_projection_refreshes: HashSet<(TimelineKey, u64)>,
    terminal_ingress: TimelineSendTerminalIngress,
}

impl SendEnqueueWorkerSupervisor {
    pub(super) fn new(terminal_ingress: TimelineSendTerminalIngress) -> Self {
        Self {
            tasks: FuturesUnordered::new(),
            deferred_projection_refreshes: HashSet::new(),
            terminal_ingress,
        }
    }

    pub(super) fn cancel_all(&mut self) {
        self.tasks = FuturesUnordered::new();
        self.deferred_projection_refreshes.clear();
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
                handle,
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
            let changed_key = registration.bind_with_handle(sdk_transaction_id, handle);
            SendEnqueueWorkerCompletion {
                changed_key,
                hydration: None,
                deferred_refresh: None,
            }
        }
        Err(kind) => {
            registration.fail_known(kind);
            SendEnqueueWorkerCompletion {
                changed_key: None,
                hydration: None,
                deferred_refresh: None,
            }
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
        .map(|handle| SendEnqueueSuccess {
            sdk_transaction_id: handle.transaction_id().to_string(),
            handle: Some(handle),
            media_queued: None,
        })
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
        .map(|handle| SendEnqueueSuccess {
            sdk_transaction_id: handle.transaction_id().to_string(),
            handle: Some(handle),
            media_queued: None,
        })
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
        handle: Some(handle),
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

pub(super) const MAX_PENDING_SEND_PROJECTIONS: usize = 128;
const PENDING_SEND_PROJECTION_ACK_DEADLINE: Duration = Duration::from_secs(5);
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
    pub(super) async fn refresh_pending_send_projection(&mut self, key: &TimelineKey) {
        let Some(actor) = self.timelines.get(key) else {
            return;
        };
        let projections = self
            .send_completion
            .lock()
            .expect("send completion coordinator lock must not be poisoned")
            .projections_for_key(key);
        let actor_generation = self
            .timeline_actor_generations
            .current_generation(key)
            .unwrap_or_default();
        let actor_tx = actor.tx.clone();
        let (acknowledged, _acknowledgement) = oneshot::channel();
        let message = TimelineActorMessage::RefreshPendingSendProjection {
            actor_generation,
            projections,
            acknowledged,
        };
        match actor_tx.try_send(message) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                let refresh_id = (key.clone(), actor_generation);
                if !self
                    .send_enqueue_workers
                    .deferred_projection_refreshes
                    .insert(refresh_id.clone())
                {
                    koushi_diagnostics::increment_counter("pending_projection_refresh_coalesced");
                    return;
                }
                koushi_diagnostics::increment_counter("pending_projection_refresh_deferred");
                let coordinator = Arc::clone(&self.send_completion);
                let refresh_key = key.clone();
                self.send_enqueue_workers.tasks.push(Box::pin(async move {
                    if let Ok(permit) = actor_tx.reserve().await {
                        let projections = coordinator
                            .lock()
                            .expect("send completion coordinator lock must not be poisoned")
                            .projections_for_key(&refresh_key);
                        let (acknowledged, _acknowledgement) = oneshot::channel();
                        permit.send(TimelineActorMessage::RefreshPendingSendProjection {
                            actor_generation,
                            projections,
                            acknowledged,
                        });
                    }
                    SendEnqueueWorkerCompletion {
                        changed_key: None,
                        hydration: None,
                        deferred_refresh: Some(refresh_id),
                    }
                }));
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                koushi_diagnostics::increment_counter("pending_projection_refresh_closed");
            }
        }
    }

    #[cfg(test)]
    fn spawn_send_enqueue_future<F>(&mut self, registration: SendCompletionRegistration, enqueue: F)
    where
        F: Future<Output = Result<SendEnqueueSuccess, TimelineFailureKind>> + Send + 'static,
    {
        let event_tx = self.event_tx.clone();
        self.send_enqueue_workers.tasks.push(Box::pin(async move {
            // Spawned workers previously isolated enqueue panics at the JoinHandle boundary.
            // Keep that fail-closed isolation when the manager polls futures directly.
            let changed_key =
                AssertUnwindSafe(run_send_enqueue_future(registration, event_tx, enqueue))
                    .catch_unwind()
                    .await
                    .ok()
                    .and_then(|completion| completion.changed_key);
            SendEnqueueWorkerCompletion {
                changed_key,
                hydration: None,
                deferred_refresh: None,
            }
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
                let mut changed_key = None;
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
                            handle,
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
                        changed_key = registration.bind_with_handle(sdk_transaction_id, handle);
                    }
                    Err(kind) => {
                        registration.fail_known(kind);
                    }
                }
                changed_key
            };
            let changed_key = AssertUnwindSafe(worker).catch_unwind().await.ok().flatten();
            SendEnqueueWorkerCompletion {
                changed_key,
                hydration: None,
                deferred_refresh: None,
            }
        }));
        preflight_started_rx
    }
    pub(super) fn spawn_pending_send_hydration(&mut self, hydration: PendingSendHydration) {
        let Some(session) = self.session.clone() else {
            return;
        };
        if !self
            .send_completion
            .lock()
            .expect("send completion coordinator lock must not be poisoned")
            .begin_hydration(&hydration.correlation)
        {
            return;
        }
        self.send_enqueue_workers.tasks.push(Box::pin(async move {
            let correlation = hydration.correlation;
            let hydrated_item = super::thread_projection::load_exact_timeline_event_projection(
                &session,
                &hydration.key,
                &hydration.event_id,
            )
            .await
            .ok();
            SendEnqueueWorkerCompletion {
                changed_key: None,
                hydration: Some((correlation, hydrated_item)),
                deferred_refresh: None,
            }
        }));
    }

    pub(super) fn retry_pending_send_hydrations(&mut self, key: &TimelineKey) {
        let hydrations = self
            .send_completion
            .lock()
            .expect("send completion coordinator lock must not be poisoned")
            .pending_hydrations_for_key(key);
        for hydration in hydrations {
            self.spawn_pending_send_hydration(hydration);
        }
    }

    pub(super) async fn handle_send_enqueue_worker_completion(
        &mut self,
        completion: SendEnqueueWorkerCompletion,
    ) {
        let mut changed_key = completion.changed_key;
        if let Some(refresh_id) = completion.deferred_refresh {
            self.send_enqueue_workers
                .deferred_projection_refreshes
                .remove(&refresh_id);
        }
        if let Some((correlation, hydrated_item)) = completion.hydration {
            let mut coordinator = self
                .send_completion
                .lock()
                .expect("send completion coordinator lock must not be poisoned");
            if let Some(item) = hydrated_item {
                changed_key = coordinator
                    .mark_hydrated(&correlation, item)
                    .or(changed_key);
            } else {
                coordinator.finish_hydration_failure(&correlation);
            }
        }
        if let Some(key) = changed_key {
            self.refresh_pending_send_projection(&key).await;
        }
    }
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
                            self.handle_send_enqueue_worker_completion(completion).await;
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
                    self.handle_send_enqueue_worker_completion(completion).await;
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
            key,
            hydration,
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
            self.emit(CoreEvent::Timeline(TimelineEvent::SendCompleted {
                request_id: completion.request_id,
                key: completion.key,
                transaction_id: completion.transaction_id,
                event_id: completion.event_id,
            }));
        }
        if let Some(failure) = failure {
            self.emit(CoreEvent::OperationFailed {
                request_id: failure.request_id,
                failure: failure.failure,
            });
        }
        if let Some(key) = key {
            self.refresh_pending_send_projection(&key).await;
        }
        if let Some(hydration) = hydration {
            self.spawn_pending_send_hydration(hydration);
        }
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
        if self
            .send_completion
            .lock()
            .expect("send completion coordinator lock must not be poisoned")
            .pending_projection_count()
            >= MAX_PENDING_SEND_PROJECTIONS
        {
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
        let (permit_tx, permit_rx) = oneshot::channel();
        let (in_reply_to_event_id, thread_root) = match &payload {
            TimelineSendEnqueuePayload::Reply {
                in_reply_to_event_id,
                ..
            } => (
                Some(in_reply_to_event_id.clone()),
                match &key.kind {
                    TimelineKind::Thread { root_event_id, .. } => Some(root_event_id.clone()),
                    _ => None,
                },
            ),
            _ => (None, None),
        };
        let own_user_id = self
            .session
            .as_ref()
            .and_then(|session| session.client().user_id().map(|id| id.to_string()));
        let pending_item = pending_send_item(
            &transaction_id,
            &body,
            in_reply_to_event_id,
            thread_root,
            own_user_id.as_deref(),
        );
        let registration = SendCompletionRegistration::begin_with_projection(
            Arc::clone(&self.send_completion),
            self.terminal_ingress.clone(),
            key.clone(),
            transaction_id.clone(),
            Some(submission_id.clone()),
            request_id,
            true,
            Some(PendingSendProjection {
                key: key.clone(),
                sequence: 0,
                client_txn_id: transaction_id.clone(),
                item: pending_item,
                sdk_transaction_id: None,
                handle: None,
                terminal_event_id: None,
                phase: PendingSendPhase::Pending,
            }),
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
        let (acknowledged, acknowledgement) = oneshot::channel();
        let pending_projections = self
            .send_completion
            .lock()
            .expect("send completion coordinator lock must not be poisoned")
            .projections_for_key(key);
        let actor_generation = self
            .timeline_actor_generations
            .current_generation(key)
            .unwrap_or_default();
        let admitted = if let Some(actor) = self.timelines.get(key) {
            actor.try_send(TimelineActorMessage::RefreshPendingSendProjection {
                actor_generation,
                projections: pending_projections,
                acknowledged,
            }) && matches!(
                executor::timeout(PENDING_SEND_PROJECTION_ACK_DEADLINE, acknowledgement).await,
                Ok(Ok(true))
            )
        } else {
            false
        };
        if !admitted {
            self.send_completion
                .lock()
                .expect("send completion coordinator lock must not be poisoned")
                .cancel_registration(registration_id);
            self.accepted_submissions
                .reject(submission_id.clone(), key.clone());
            self.refresh_pending_send_projection(key).await;
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
                self.refresh_pending_send_projection(key).await;
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
        let retained = self
            .send_completion
            .lock()
            .expect("send completion coordinator lock must not be poisoned")
            .retained_handle(
                &self.key,
                &transaction_id,
                PendingSendPhase::FailedRecoverable,
            );
        let (sdk_transaction_id, handle, manager_owned) = if let Some((sdk_id, handle)) = retained {
            (sdk_id, handle, true)
        } else {
            if let Err(kind) = validate_retry_send(self.send_statuses.get(&transaction_id)) {
                self.emit_timeline_failure(request_id, kind);
                return;
            }
            let Some(handle) = self.send_handles.get(&transaction_id).cloned() else {
                self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendTarget);
                return;
            };
            (transaction_id.clone(), handle, false)
        };

        let Some(room) = self.sdk_room_for_key() else {
            self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendTarget);
            return;
        };
        room.send_queue().set_enabled(true);

        match handle.unwedge().await {
            Ok(()) => {
                self.send_statuses
                    .insert(sdk_transaction_id, TimelineSendState::Sending);
                if manager_owned {
                    self.send_completion
                        .lock()
                        .expect("send completion coordinator lock must not be poisoned")
                        .mark_retry_sending(&self.key, &transaction_id);
                    let projections = self
                        .send_completion
                        .lock()
                        .expect("send completion coordinator lock must not be poisoned")
                        .projections_for_key(&self.key);
                    self.refresh_pending_send_projection(self.actor_generation, projections)
                        .await;
                }
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
        let retained = {
            let coordinator = self
                .send_completion
                .lock()
                .expect("send completion coordinator lock must not be poisoned");
            coordinator
                .retained_handle(&self.key, &transaction_id, PendingSendPhase::Pending)
                .or_else(|| {
                    coordinator.retained_handle(
                        &self.key,
                        &transaction_id,
                        PendingSendPhase::FailedRecoverable,
                    )
                })
        };
        let (sdk_transaction_id, handle, manager_owned) = if let Some((sdk_id, handle)) = retained {
            (sdk_id, handle, true)
        } else {
            if let Err(kind) = validate_cancel_send(self.send_statuses.get(&transaction_id)) {
                self.emit_timeline_failure(request_id, kind);
                return;
            }
            let Some(handle) = self.send_handles.get(&transaction_id).cloned() else {
                self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendTarget);
                return;
            };
            (transaction_id.clone(), handle, false)
        };

        match handle.abort().await {
            Ok(true) => {
                self.send_statuses
                    .insert(sdk_transaction_id.clone(), TimelineSendState::Cancelled);
                self.send_handles.remove(&sdk_transaction_id);
                if let Some(room) = self.sdk_room_for_key() {
                    room.send_queue().set_enabled(true);
                }
                apply_send_completion_observation_and_handoff(
                    &self.send_completion,
                    &self.terminal_ingress,
                    self.key.room_id(),
                    SendCompletionObservation::Cancelled { sdk_transaction_id },
                );
                if manager_owned {
                    let projections = self
                        .send_completion
                        .lock()
                        .expect("send completion coordinator lock must not be poisoned")
                        .projections_for_key(&self.key);
                    self.refresh_pending_send_projection(self.actor_generation, projections)
                        .await;
                }
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
                {
                    let mut coordinator = self
                        .send_completion
                        .lock()
                        .expect("send completion coordinator lock must not be poisoned");
                    coordinator.stage_pending_send(
                        self.key.room_id(),
                        &sdk_transaction_id,
                        "local_echo_observed",
                    );
                    coordinator.reconcile_local_echo(self.key.room_id(), &sdk_transaction_id);
                }
                remember_local_echo(&mut self.send_statuses, &mut self.send_handles, &echo);
                let projections = self
                    .send_completion
                    .lock()
                    .expect("send completion coordinator lock must not be poisoned")
                    .projections_for_key(&self.key);
                self.refresh_pending_send_projection(self.actor_generation, projections)
                    .await;
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
        for item in &items {
            super::thread_projection::seed_thread_summary_item(
                &self.thread_root_projection_service,
                &self.key,
                item,
            );
        }
        let mut candidate_display_projection =
            DisplayProjectionState::from_canonical_window(&items, 0..items.len());
        let context = DisplayProjectionContext::for_timeline(
            &self.key.kind,
            &self.viewport_observation,
            false,
        )
        .with_thread_roots(
            self.thread_root_order,
            self.thread_root_projection_service
                .lock()
                .expect("thread-root projection service lock must not be poisoned")
                .display_data_for_room(self.key.room_id()),
        );
        candidate_display_projection.reproject(&context);
        let emitted_items = candidate_display_projection.display_items().to_vec();
        let _ = commit_prepared_initial_window_for_generation(
            &mut self.navigation_items,
            &mut self.display_projection,
            &self.event_tx,
            &self.timeline_actor_generations,
            &self.key,
            self.actor_generation,
            InitialItemsRequestIdentity::recovery(),
            self.generation,
            Vec::new(),
            PreparedInitialWindow {
                display_projection: candidate_display_projection,
                navigation_items: Some(items),
                emitted_items,
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
    retained_projections: HashMap<SendCorrelationKey, RetainedPendingProjection>,
    hydrated_projection_order: VecDeque<SendCorrelationKey>,
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

struct RetainedPendingProjection {
    projection: PendingSendProjection,
    lifecycle_trace: SendLifecycleTrace,
    hydration_in_flight: bool,
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
    projection: Option<PendingSendProjection>,
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
        Self::begin_with_projection(
            coordinator,
            terminal_ingress,
            key,
            client_txn_id,
            submission_id,
            request_id,
            settles_composer,
            None,
        )
    }

    pub(super) fn begin_with_projection(
        coordinator: SharedSendCompletionCoordinator,
        terminal_ingress: TimelineSendTerminalIngress,
        key: TimelineKey,
        client_txn_id: String,
        submission_id: Option<koushi_state::SubmissionId>,
        request_id: RequestId,
        settles_composer: bool,
        mut projection: Option<PendingSendProjection>,
    ) -> Self {
        let mut lifecycle_trace = SendLifecycleTrace::new(&key, settles_composer);
        lifecycle_trace.stage("accepted");
        if projection.is_some() {
            lifecycle_trace.stage("pending_projection_inserted");
        }
        let registration_id = {
            let mut coordinator = coordinator
                .lock()
                .expect("send completion coordinator lock must not be poisoned");
            coordinator.next_registration_id = coordinator
                .next_registration_id
                .checked_add(1)
                .expect("send registration id space exhausted");
            let registration_id = coordinator.next_registration_id;
            if let Some(projection) = projection.as_mut() {
                projection.sequence = registration_id;
            }
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
                    projection,
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
        let _ = self.bind_with_handle(sdk_transaction_id, None);
    }

    pub(super) fn bind_with_handle(
        &mut self,
        sdk_transaction_id: String,
        handle: Option<matrix_sdk::send_queue::SendHandle>,
    ) -> Option<TimelineKey> {
        let Some(registration_id) = self.registration_id.take() else {
            return None;
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
        let (key, handoffs) = coordinator.bind_registration(
            registration_id,
            sdk_transaction_id,
            handle,
            interactive_guard,
            lifecycle_trace,
        );
        for handoff in handoffs {
            let _admission = self.terminal_ingress.admit(handoff);
        }
        key
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
    pub(super) fn pending_projection_count(&self) -> usize {
        self.registrations
            .values()
            .chain(self.pending_sends.values())
            .filter(|pending| pending.projection.is_some())
            .count()
            .saturating_add(
                self.retained_projections
                    .values()
                    .filter(|retained| retained.projection.phase != PendingSendPhase::HydratedSent)
                    .count(),
            )
    }

    pub(super) fn projections_for_key(&self, key: &TimelineKey) -> Vec<PendingSendProjection> {
        let mut projections = self
            .registrations
            .values()
            .chain(self.pending_sends.values())
            .filter_map(|pending| {
                pending
                    .projection
                    .as_ref()
                    .filter(|projection| &projection.key == key)
                    .cloned()
            })
            .chain(
                self.retained_projections
                    .values()
                    .map(|retained| &retained.projection)
                    .filter(|projection| &projection.key == key)
                    .cloned(),
            )
            .collect::<Vec<_>>();
        projections.sort_by_key(|projection| projection.sequence);
        projections
    }

    pub(super) fn retained_handle(
        &self,
        key: &TimelineKey,
        render_id: &str,
        required_phase: PendingSendPhase,
    ) -> Option<(String, matrix_sdk::send_queue::SendHandle)> {
        self.pending_sends
            .iter()
            .find_map(|(correlation, pending)| {
                let projection = pending.projection.as_ref()?;
                if &projection.key == key
                    && projection.phase == required_phase
                    && (projection.client_txn_id == render_id
                        || projection.sdk_transaction_id.as_deref() == Some(render_id))
                {
                    Some((
                        correlation.sdk_transaction_id.clone(),
                        projection.handle.clone()?,
                    ))
                } else {
                    None
                }
            })
    }

    pub(super) fn mark_retry_sending(&mut self, key: &TimelineKey, render_id: &str) -> bool {
        let Some(pending) = self.pending_sends.values_mut().find(|pending| {
            pending.projection.as_ref().is_some_and(|projection| {
                &projection.key == key
                    && projection.phase == PendingSendPhase::FailedRecoverable
                    && (projection.client_txn_id == render_id
                        || projection.sdk_transaction_id.as_deref() == Some(render_id))
            })
        }) else {
            return false;
        };
        let Some(projection) = pending.projection.as_mut() else {
            return false;
        };
        projection.phase = PendingSendPhase::Pending;
        projection.item.send_state = Some(TimelineSendState::Sending);
        pending.failure_reported = false;
        true
    }

    fn begin_hydration(&mut self, correlation: &SendCorrelationKey) -> bool {
        let Some(retained) = self.retained_projections.get_mut(correlation) else {
            return false;
        };
        if retained.projection.phase != PendingSendPhase::SentAwaitingRemote
            || retained.hydration_in_flight
        {
            return false;
        }
        retained.hydration_in_flight = true;
        true
    }

    fn finish_hydration_failure(&mut self, correlation: &SendCorrelationKey) {
        if let Some(retained) = self.retained_projections.get_mut(correlation) {
            retained.hydration_in_flight = false;
        }
    }

    pub(super) fn pending_hydrations_for_key(
        &self,
        key: &TimelineKey,
    ) -> Vec<PendingSendHydration> {
        self.retained_projections
            .iter()
            .filter_map(|(correlation, retained)| {
                let projection = &retained.projection;
                let event_id = projection.terminal_event_id.clone()?;
                (&projection.key == key
                    && projection.phase == PendingSendPhase::SentAwaitingRemote
                    && !retained.hydration_in_flight)
                    .then(|| PendingSendHydration {
                        correlation: correlation.clone(),
                        key: projection.key.clone(),
                        event_id,
                    })
            })
            .collect()
    }

    fn mark_hydrated(
        &mut self,
        correlation: &SendCorrelationKey,
        mut item: TimelineItem,
    ) -> Option<TimelineKey> {
        let retained = self.retained_projections.get_mut(correlation)?;
        if retained.projection.phase != PendingSendPhase::SentAwaitingRemote {
            return None;
        }
        let expected_event_id = retained.projection.terminal_event_id.as_deref()?;
        if !matches!(
            &item.id,
            TimelineItemId::Event { event_id } if event_id == expected_event_id
        ) {
            retained.hydration_in_flight = false;
            return None;
        }
        item.send_state = Some(TimelineSendState::Sent);
        retained.projection.item = item;
        retained.projection.phase = PendingSendPhase::HydratedSent;
        retained.hydration_in_flight = false;
        let key = retained.projection.key.clone();
        self.hydrated_projection_order
            .push_back(correlation.clone());
        while self.hydrated_projection_order.len() > MAX_PENDING_SEND_PROJECTIONS {
            let Some(expired) = self.hydrated_projection_order.pop_front() else {
                break;
            };
            if self
                .retained_projections
                .get(&expired)
                .is_some_and(|entry| entry.projection.phase == PendingSendPhase::HydratedSent)
            {
                self.retained_projections.remove(&expired);
            }
        }
        Some(key)
    }

    pub(super) fn reconcile_remote_event(
        &mut self,
        room_id: &str,
        event_id: &str,
    ) -> Option<TimelineKey> {
        let key = self
            .pending_sends
            .iter()
            .find_map(|(correlation, pending)| {
                pending
                    .projection
                    .as_ref()
                    .filter(|projection| {
                        projection.key.room_id() == room_id
                            && projection.terminal_event_id.as_deref() == Some(event_id)
                    })
                    .map(|_| (correlation.clone(), pending.key.clone()))
            })
            .or_else(|| {
                self.retained_projections
                    .iter()
                    .find_map(|(correlation, retained)| {
                        let projection = &retained.projection;
                        (projection.key.room_id() == room_id
                            && projection.terminal_event_id.as_deref() == Some(event_id))
                        .then(|| (correlation.clone(), projection.key.clone()))
                    })
            });
        let Some((correlation, key)) = key else {
            return None;
        };
        self.pending_sends.remove(&correlation);
        if let Some(mut retained) = self.retained_projections.remove(&correlation) {
            retained.lifecycle_trace.stage_once("remote_echo_converged");
        }
        self.remember_settled(correlation);
        Some(key)
    }

    pub(super) fn settled_transaction_ids(&self, room_id: &str) -> HashSet<String> {
        self.settled_send_tombstones
            .iter()
            .filter(|correlation| correlation.room_id == room_id)
            .map(|correlation| correlation.sdk_transaction_id.clone())
            .collect()
    }

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

    pub(super) fn reconcile_local_echo(
        &mut self,
        room_id: &str,
        sdk_transaction_id: &str,
    ) -> Option<TimelineKey> {
        let pending = self.pending_sends.get_mut(&SendCorrelationKey {
            room_id: room_id.to_owned(),
            sdk_transaction_id: sdk_transaction_id.to_owned(),
        })?;
        if pending.projection.take().is_some() {
            pending.lifecycle_trace.stage_once("sdk_local_echo_merged");
            return Some(pending.key.clone());
        }
        None
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
        handle: Option<matrix_sdk::send_queue::SendHandle>,
        interactive_guard: Option<InteractiveWorkGuard>,
        lifecycle_trace: SendLifecycleTrace,
    ) -> (Option<TimelineKey>, Vec<TimelineSendTerminalHandoff>) {
        let Some(mut registration) = self.registrations.remove(&registration_id) else {
            return (None, Vec::new());
        };
        if !registration.active {
            self.purge_unmatched_for_inactive_room(registration.key.room_id());
            return (None, Vec::new());
        }
        let key = registration.key.clone();
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
            return (Some(key), handoffs);
        }
        if let Some(projection) = registration.projection.as_mut() {
            projection.sdk_transaction_id = Some(correlation.sdk_transaction_id.clone());
            projection.item.id = TimelineItemId::Transaction {
                transaction_id: correlation.sdk_transaction_id.clone(),
            };
            projection.handle = handle;
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
        (Some(key), handoffs)
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
                if let Some(projection) = pending.projection.as_mut() {
                    pending.lifecycle_trace.stage_once("sdk_local_echo_missing");
                    projection.terminal_event_id = Some(event_id.clone());
                    projection.phase = PendingSendPhase::SentAwaitingRemote;
                    projection.item.id = TimelineItemId::Event {
                        event_id: event_id.clone(),
                    };
                    projection.item.send_state = Some(TimelineSendState::Sent);
                    projection.handle = None;
                }
                let projection = pending.projection.take();
                if let Some(projection) = projection {
                    self.retained_projections.insert(
                        correlation.clone(),
                        RetainedPendingProjection {
                            projection,
                            lifecycle_trace: pending.lifecycle_trace.clone(),
                            hydration_in_flight: false,
                        },
                    );
                }
                self.remember_settled(correlation.clone());
                self.purge_unmatched_for_inactive_room(&correlation.room_id);
                let mut handoff = timeline_send_terminal_handoff(
                    &pending.key,
                    pending.client_txn_id.clone(),
                    pending.submission_id.clone(),
                    SendCompletionTerminal::Succeeded {
                        request_id: pending.request_id,
                        event_id: event_id.clone(),
                        settles_composer,
                    },
                );
                if self.retained_projections.contains_key(correlation) {
                    handoff.hydration = Some(PendingSendHydration {
                        correlation: correlation.clone(),
                        key: pending.key.clone(),
                        event_id,
                    });
                }
                Some(handoff)
            }
            ObservedSendTerminal::SendError { diagnostic } => {
                let recoverable = diagnostic.recoverable;
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
                if let Some(projection) = pending.projection.as_mut() {
                    if recoverable {
                        projection.phase = PendingSendPhase::FailedRecoverable;
                        projection.item.send_state = Some(TimelineSendState::NotSent {
                            reason: koushi_protocol::event::TimelineSendFailureReason::Recoverable,
                        });
                    } else {
                        pending.projection = None;
                    }
                }
                Some(timeline_send_terminal_handoff(
                    &pending.key,
                    pending.client_txn_id.clone(),
                    pending.submission_id.clone(),
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
        }),
        SendCompletionTerminal::Failed { .. } | SendCompletionTerminal::Cancelled { .. } => None,
    };
    TimelineSendTerminalHandoff {
        key: Some(key.clone()),
        hydration: None,
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
mod tests;
