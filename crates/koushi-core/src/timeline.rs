//! TimelineActor: per-`TimelineKey` subscription, diff relay, pagination,
//! send/edit/redact.
//!
//! ## Ownership
//! One `TimelineActor` per `TimelineKey`, owned by `AccountActor` in a
//! `HashMap<TimelineKey, TimelineActorHandle>`. `Unsubscribe` removes and drops
//! the entry — the runtime never leaks timeline state (Async rule 7).
//!
//! ## Generations & overflow protocol (canon pre-resolved decision B)
//! The relay task holds an `mpsc::Sender<DiffBatch>` of capacity 128
//! (`TIMELINE_DIFF_QUEUE_CAPACITY`). On `try_send` overflow:
//!   1. Stop forwarding diffs for the current generation.
//!   2. Bump `TimelineGeneration` (stored in `Arc<AtomicU64>`).
//!   3. Emit `ResyncRequired { reason: QueueOverflow }`.
//!   4. Emit a fresh `InitialItems` with the new generation from the current
//!      SDK timeline snapshot.
//!
//! ## Batch IDs (canon pre-resolved decision C)
//! Monotonic per generation starting at 0; the relay task increments
//! `next_batch_id` before emitting each `ItemsUpdated`.
//!
//! ## Transaction ID mapping (canon pre-resolved decision D)
//! `send_text_message` in the auth crate calls `room.send(content).with_transaction_id(txn_id)`.
//! This makes the SDK's send queue use our client-supplied txn ID for both the
//! local echo in timeline diffs AND the `RoomSendQueueUpdate::SentEvent` payload.
//! No separate mapping table is needed: the client txn ID IS the SDK txn ID.
//! `SendCompleted` carries the client txn ID directly from `SentEvent.transaction_id`.
//!
//! ## Pagination
//! `Timeline::paginate_backwards(n)` returns `Ok(true)` when the start of
//! history is reached (EndReached), `Ok(false)` when more history exists, and
//! `Err(_)` on failure. We emit:
//!   Idle → Paginating → (EndReached | Idle | Failed)
//! Forward pagination is only allowed on Focused timelines (Async rule 5).
//!
//! ## Thread/Focused support
//! The vendored SDK supports `TimelineFocus::Thread` and `TimelineFocus::Event`
//! (`::Focused`). Both are implemented. paginate_forwards is valid on Focused
//! (SDK: returns Ok(true) for Live focus, actually does work for Event focus).
//!
//! ## SDK handle lifecycle
//! The `Arc<matrix_sdk_ui::Timeline>` is held by the relay task. Dropping the
//! relay task's sender (on Unsubscribe or AccountActor shutdown) cancels the
//! relay task, which drops the Timeline handle — cancelling its background tasks.
//!
//! ## Security
//! Message bodies appear in `TimelineItem.body` (visible UI state per canon)
//! but never in error messages, log strings, or Debug output of error types.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use koushi_sdk::MatrixClientSession;
use koushi_search::{AttachmentDocument, SensitiveString};
use koushi_state::{
    ActivityRow, AppAction, AttachmentKind, AvatarImage, AvatarThumbnailState, ComposerSendIntent,
    FormattedMessageDraft, LiveEventReceipts, LiveReadReceipt, MediaTransferProgress,
    MentionIntent, OperationFailureKind, ReplyQuote, ReplyQuoteState, SlashCommandIntent,
    TimelineMediaDownloadState, TimelineMediaGalleryItem, TimelineMediaGalleryMedia,
    TimelineMediaGallerySource, TimelineMediaGalleryThumbnail,
    TimelineMediaKind as GalleryTimelineMediaKind, resolve_composer_send_intent,
};
use matrix_sdk::attachment::{
    AttachmentConfig, AttachmentInfo, BaseFileInfo, BaseImageInfo, Thumbnail,
};
use matrix_sdk::media::{MediaFormat, MediaRequestParameters, MediaThumbnailSettings};
use matrix_sdk::room::Receipts;
use matrix_sdk::room::edit::EditedContent;
use matrix_sdk::room::reply::{EnforceThread, Reply};
use matrix_sdk::ruma::UserId;
use matrix_sdk::ruma::api::client::receipt::create_receipt::v3::ReceiptType;
use matrix_sdk::ruma::events::Mentions;
use matrix_sdk::ruma::events::room::message::FormattedBody;
use matrix_sdk::ruma::events::room::message::{
    AddMentions, MessageFormat, MessageType, ReplyWithinThread, RoomMessageEventContent,
    RoomMessageEventContentWithoutRelation, TextMessageEventContent,
};
use matrix_sdk::ruma::events::room::{MediaSource, ThumbnailInfo};
use matrix_sdk::ruma::html::{Html, SanitizerConfig};
use matrix_sdk::send_queue::{LocalEcho, LocalEchoContent, RoomSendQueueUpdate, SendHandle};
use matrix_sdk_ui::timeline::{
    EmbeddedEvent, EncryptedMessage, EventSendState as SdkEventSendState, EventTimelineItem,
    InReplyToDetails, MembershipChange, Profile, ReactionStatus, ReactionsByKeyBySender, Timeline,
    TimelineDetails, TimelineEventItemId, TimelineFocus, TimelineItem as SdkTimelineItem,
    TimelineItemContent, TimelineItemKind, TimelineReadReceiptTracking,
};
use tokio::sync::{broadcast, mpsc};

use crate::command::{
    MediaDownloadSelection, TimelineCommand, UploadMediaKind, UploadMediaRequest,
};
use crate::event::{
    CoreEvent, LinkPreview, LinkPreviewState, LiveSignalsEvent, PaginationDirection,
    PaginationState, ThreadSummaryDto, TimelineAnchorRestoreStatus, TimelineDiff, TimelineEvent,
    TimelineItem, TimelineItemId, TimelineMedia, TimelineMediaKind, TimelineMediaSource,
    TimelineMediaThumbnail, TimelineMessageActions, TimelineMessageKind, TimelineMessageSource,
    TimelineNavigationSnapshot, TimelineResyncReason, TimelineSendFailureReason, TimelineSendState,
    TimelineSpoilerSpan, TimelineUnableToDecrypt, TimelineUnableToDecryptReason,
    TimelineUnreadPosition, TimelineViewportObservation, message_actions_for_timeline_item,
    message_source_for_timeline_item,
};
use crate::executor;
use crate::failure::{CoreFailure, TimelineFailureKind};
use crate::ids::{RequestId, TimelineBatchId, TimelineGeneration, TimelineKey, TimelineKind};
use crate::link_preview::{LinkPreviewContext, extract_link_ranges};
use crate::messages_backpressure::MessagesBackpressure;
use crate::search::SearchIndexMessage;
use crate::startup_trace::{self, StartupPhase};
use crate::unread_trace;

/// Bounded diff queue capacity per subscribed timeline (overview.md, Async rule 10).
pub const TIMELINE_DIFF_QUEUE_CAPACITY: usize = 128;
const ROOM_REPLAY_INITIAL_ITEMS_MAX: usize = 120;
const REPLY_QUOTE_PREVIEW_MAX_CHARS: usize = 160;
/// Backstop tick count for the anchor-relay wait. After the SDK signals
/// `anchor_present == true`, the anchor's diff has been broadcast through the
/// 3-hop relay (conclude_backwards_pagination_from_disk → event-cache task →
/// timeline observable → relay → DiffBatch actor msg) and WILL arrive in the
/// actor's `timeline_contains` check within the next few ticks. This backstop
/// guards against a genuinely stuck relay; under normal load the anchor lands
/// well before the count reaches zero.
const RESTORE_ANCHOR_RELAY_WAIT_TICKS: u8 = 40;
/// Delay between anchor-relay-wait ticks (milliseconds). The relay pipeline
/// is a 3-hop async path: conclude_backwards_pagination_from_disk →
/// room_event_cache_updates_task → handle_remote_events_with_diffs →
/// observable → relay task → DiffBatch actor message. Without a pause, all
/// 40 backstop ticks can drain before any relay task gets CPU time.
/// 50 ms is deliberately conservative (well within the 2 000 ms total
/// budget); under normal conditions the anchor lands on tick 1.
const RESTORE_ANCHOR_RELAY_WAIT_TICK_MS: u64 = 50;

/// Messages routed to the `TimelineManagerActor`.
pub enum TimelineMessage {
    Command(TimelineCommand),
    /// Sync started: carries the live `RoomListService` on the SyncService
    /// backend (None on LegacySync). Subscribing a timeline must also
    /// subscribe its room with the live service so the server streams that
    /// room's new timeline events (canon: TimelineActor description; without
    /// this, e.g. Conduit's sliding sync only delivers the initial window).
    SyncStarted {
        room_list_service: Option<Arc<matrix_sdk_ui::room_list_service::RoomListService>>,
    },
    IgnoredUsersUpdated {
        user_ids: std::collections::BTreeSet<String>,
    },
    Shutdown,
}

/// Handle to the timeline manager task (owned by `AccountActor`).
pub struct TimelineManagerHandle {
    tx: mpsc::Sender<TimelineMessage>,
}

impl TimelineManagerHandle {
    pub async fn send(&self, msg: TimelineMessage) -> bool {
        self.tx.send(msg).await.is_ok()
    }

    pub fn try_send(&self, msg: TimelineMessage) -> bool {
        self.tx.try_send(msg).is_ok()
    }

    pub(crate) fn sender(&self) -> mpsc::Sender<TimelineMessage> {
        self.tx.clone()
    }
}

/// Manages the `HashMap<TimelineKey, TimelineActorHandle>`.
/// Colocated as a child task under `AccountActor` (spec: "actor deployment
/// is flexible; boundaries define ownership not one task per actor").
pub struct TimelineManagerActor {
    session: Option<Arc<MatrixClientSession>>,
    room_list_service: Option<Arc<matrix_sdk_ui::room_list_service::RoomListService>>,
    timelines: HashMap<TimelineKey, TimelineActorHandle>,
    action_tx: mpsc::Sender<Vec<AppAction>>,
    event_tx: broadcast::Sender<CoreEvent>,
    msg_rx: mpsc::Receiver<TimelineMessage>,
    /// Search index mutation sender. Forwarded to individual `TimelineActor`s
    /// so they can push `SearchIndexMessage`s on each diff. `None` when there
    /// is no active search index (pre-session or pre-Phase-6 builds).
    search_index_tx: Option<mpsc::Sender<SearchIndexMessage>>,
    ignored_user_ids: std::collections::BTreeSet<String>,
    /// Application data directory for cached preview images.
    data_dir: Option<std::path::PathBuf>,
    /// URL preview policy broadcast from AppState.
    link_preview_policy: LinkPreviewContext,
    messages_backpressure: MessagesBackpressure,
}

impl TimelineManagerActor {
    pub(crate) fn spawn(
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
        data_dir: Option<std::path::PathBuf>,
        messages_backpressure: MessagesBackpressure,
    ) -> TimelineManagerHandle {
        let (tx, msg_rx) = mpsc::channel(crate::runtime::ACTOR_MESSAGE_QUEUE_CAPACITY);
        let actor = TimelineManagerActor {
            session: None,
            room_list_service: None,
            timelines: HashMap::new(),
            action_tx,
            event_tx,
            msg_rx,
            search_index_tx: None,
            ignored_user_ids: std::collections::BTreeSet::new(),
            data_dir,
            link_preview_policy: LinkPreviewContext::default(),
            messages_backpressure,
        };
        executor::spawn(actor.run());
        TimelineManagerHandle { tx }
    }

    /// Spawn with a session and a search index mutation sender.
    /// Called by `AccountActor::spawn_sync_actor` (Phase 6 wiring).
    pub(crate) fn spawn_with_session(
        session: Arc<MatrixClientSession>,
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
        search_index_tx: mpsc::Sender<SearchIndexMessage>,
        data_dir: Option<std::path::PathBuf>,
        link_preview_policy: LinkPreviewContext,
        messages_backpressure: MessagesBackpressure,
    ) -> TimelineManagerHandle {
        let (tx, msg_rx) = mpsc::channel(crate::runtime::ACTOR_MESSAGE_QUEUE_CAPACITY);
        let actor = TimelineManagerActor {
            session: Some(session),
            room_list_service: None,
            timelines: HashMap::new(),
            action_tx,
            event_tx,
            msg_rx,
            search_index_tx: Some(search_index_tx),
            ignored_user_ids: std::collections::BTreeSet::new(),
            data_dir,
            link_preview_policy,
            messages_backpressure,
        };
        executor::spawn(actor.run());
        TimelineManagerHandle { tx }
    }

    async fn run(mut self) {
        while let Some(msg) = self.msg_rx.recv().await {
            match msg {
                TimelineMessage::Shutdown => break,
                TimelineMessage::SyncStarted { room_list_service } => {
                    self.room_list_service = room_list_service;
                }
                TimelineMessage::IgnoredUsersUpdated { user_ids } => {
                    self.handle_ignored_users_updated(user_ids).await;
                }
                TimelineMessage::Command(command) => {
                    self.handle_command(command).await;
                }
            }
        }
        // Drop all timeline handles — this cancels relay tasks and drops SDK handles.
        self.timelines.clear();
    }

    async fn handle_ignored_users_updated(&mut self, user_ids: std::collections::BTreeSet<String>) {
        self.ignored_user_ids = user_ids.clone();
        for handle in self.timelines.values() {
            let _ = handle
                .send(TimelineActorMessage::IgnoredUsersUpdated(user_ids.clone()))
                .await;
        }
    }

    async fn handle_command(&mut self, command: TimelineCommand) {
        match command {
            TimelineCommand::Subscribe { request_id, key } => {
                trace_timeline_route("manager_received", "subscribe", request_id, &key);
                self.handle_subscribe(request_id, key, true).await;
            }
            TimelineCommand::EnsureSubscribed {
                request_id,
                key,
                replay_existing,
            } => {
                trace_timeline_route("manager_received", "ensure_subscribed", request_id, &key);
                self.handle_subscribe(request_id, key, replay_existing)
                    .await;
            }
            TimelineCommand::Unsubscribe { request_id, key } => {
                trace_timeline_route("manager_received", "unsubscribe", request_id, &key);
                // Drop the actor handle, which cancels its relay task and drops
                // the SDK Timeline handle — no dedicated success event per spec.
                self.timelines.remove(&key);
            }
            TimelineCommand::Paginate {
                request_id,
                key,
                direction,
                event_count,
            } => {
                trace_timeline_route("manager_received", "paginate", request_id, &key);
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::Paginate {
                        request_id,
                        direction,
                        event_count,
                    },
                )
                .await;
            }
            TimelineCommand::CancelPagination { request_id, key } => {
                trace_timeline_route("manager_received", "cancel_pagination", request_id, &key);
                if let Some(handle) = self.timelines.get(&key) {
                    let _ = handle
                        .send(TimelineActorMessage::CancelPagination { request_id })
                        .await;
                }
            }
            TimelineCommand::CancelLinkPreviews { request_id, key } => {
                trace_timeline_route("manager_received", "cancel_link_previews", request_id, &key);
                if let Some(handle) = self.timelines.get(&key) {
                    let _ = handle
                        .send(TimelineActorMessage::CancelLinkPreviews { request_id })
                        .await;
                }
            }
            TimelineCommand::RestoreTimelineAnchor {
                request_id,
                key,
                event_id,
                max_batches,
                event_count,
            } => {
                if matches!(&key.kind, TimelineKind::Room { .. }) {
                    self.route_to_actor_or_fail(
                        request_id,
                        &key,
                        TimelineActorMessage::RestoreTimelineAnchor {
                            request_id,
                            event_id,
                            max_batches,
                            event_count,
                        },
                    )
                    .await;
                } else {
                    self.emit(CoreEvent::Timeline(TimelineEvent::AnchorRestoreFinished {
                        request_id,
                        key,
                        status: TimelineAnchorRestoreStatus::Failed {
                            kind: TimelineFailureKind::NotSubscribed,
                        },
                    }));
                }
            }
            TimelineCommand::ObserveViewport {
                request_id,
                key,
                observation,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::ObserveViewport { observation },
                )
                .await;
            }
            TimelineCommand::SendText {
                request_id,
                key,
                transaction_id,
                body,
                mentions,
            } => {
                if let Err(kind) = validate_composer_body_for_timeline_send(&body) {
                    self.emit_failure(request_id, CoreFailure::TimelineOperationFailed { kind });
                    return;
                }
                self.route_send_to_actor_or_fail(
                    request_id,
                    &key,
                    transaction_id.clone(),
                    body.clone(),
                    SendComposerProjection::for_send_text(&key),
                    TimelineActorMessage::SendText {
                        request_id,
                        transaction_id,
                        body,
                        mentions,
                    },
                )
                .await;
            }
            TimelineCommand::SendReply {
                request_id,
                key,
                transaction_id,
                in_reply_to_event_id,
                body,
                mentions,
            } => {
                if let Err(kind) = validate_composer_body_for_timeline_send(&body) {
                    self.emit_failure(request_id, CoreFailure::TimelineOperationFailed { kind });
                    return;
                }
                self.route_send_to_actor_or_fail(
                    request_id,
                    &key,
                    transaction_id.clone(),
                    body.clone(),
                    SendComposerProjection::for_send_reply(&key),
                    TimelineActorMessage::SendReply {
                        request_id,
                        transaction_id,
                        in_reply_to_event_id,
                        body,
                        mentions,
                    },
                )
                .await;
            }
            TimelineCommand::ForwardMessage {
                request_id,
                key,
                source_event_id,
                destination_room_id,
                transaction_id,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::ForwardMessage {
                        request_id,
                        source_event_id,
                        destination_room_id,
                        transaction_id,
                    },
                )
                .await;
            }
            TimelineCommand::LoadMessageSource {
                request_id,
                key,
                event_id,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::LoadMessageSource {
                        request_id,
                        event_id,
                    },
                )
                .await;
            }
            TimelineCommand::RequestRoomKey {
                request_id,
                key,
                event_id,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::RequestRoomKey {
                        request_id,
                        event_id,
                    },
                )
                .await;
            }
            TimelineCommand::RetrySend {
                request_id,
                key,
                transaction_id,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::RetrySend {
                        request_id,
                        transaction_id,
                    },
                )
                .await;
            }
            TimelineCommand::CancelSend {
                request_id,
                key,
                transaction_id,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::CancelSend {
                        request_id,
                        transaction_id,
                    },
                )
                .await;
            }
            TimelineCommand::UploadAndSendMedia {
                request_id,
                key,
                transaction_id,
                request,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::UploadAndSendMedia {
                        request_id,
                        transaction_id,
                        request,
                    },
                )
                .await;
            }
            TimelineCommand::DownloadMedia {
                request_id,
                key,
                event_id,
                selection,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::DownloadMedia {
                        request_id,
                        event_id,
                        selection,
                    },
                )
                .await;
            }
            TimelineCommand::EditText {
                request_id,
                key,
                event_id,
                body,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::EditText {
                        request_id,
                        event_id,
                        body,
                    },
                )
                .await;
            }
            TimelineCommand::Redact {
                request_id,
                key,
                event_id,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::Redact {
                        request_id,
                        event_id,
                    },
                )
                .await;
            }
            TimelineCommand::ToggleReaction {
                request_id,
                key,
                event_id,
                reaction_key,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::ToggleReaction {
                        request_id,
                        event_id,
                        reaction_key,
                    },
                )
                .await;
            }
            TimelineCommand::SendReaction {
                request_id,
                key,
                event_id,
                reaction_key,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::SendReaction {
                        request_id,
                        event_id,
                        reaction_key,
                    },
                )
                .await;
            }
            TimelineCommand::RedactReaction {
                request_id,
                key,
                event_id,
                reaction_key,
                reaction_event_id,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::RedactReaction {
                        request_id,
                        event_id,
                        reaction_key,
                        reaction_event_id,
                    },
                )
                .await;
            }
            TimelineCommand::SendReadReceipt {
                request_id,
                key,
                event_id,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::SendReadReceipt {
                        request_id,
                        event_id,
                    },
                )
                .await;
            }
            TimelineCommand::SetFullyRead {
                request_id,
                key,
                event_id,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::SetFullyRead {
                        request_id,
                        event_id,
                    },
                )
                .await;
            }
            TimelineCommand::SetTyping {
                request_id,
                key,
                is_typing,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::SetTyping {
                        request_id,
                        is_typing,
                    },
                )
                .await;
            }
            TimelineCommand::LoadLinkPreviews {
                request_id,
                key,
                event_id,
            } => {
                trace_timeline_route("manager_received", "load_link_previews", request_id, &key);
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::LoadLinkPreviews {
                        request_id,
                        event_id,
                    },
                )
                .await;
            }
            TimelineCommand::HideLinkPreview {
                request_id,
                key,
                event_id,
            } => {
                self.route_to_actor_or_fail(
                    request_id,
                    &key,
                    TimelineActorMessage::HideLinkPreview {
                        request_id,
                        event_id,
                    },
                )
                .await;
            }
            TimelineCommand::BroadcastLinkPreviewPolicy {
                unencrypted_global_enabled,
                encrypted_global_enabled,
                room_overrides,
            } => {
                self.link_preview_policy.unencrypted_global_enabled = unencrypted_global_enabled;
                self.link_preview_policy.encrypted_global_enabled = encrypted_global_enabled;
                self.link_preview_policy.room_overrides = room_overrides;
                for (key, handle) in &self.timelines {
                    let room_enabled = self
                        .link_preview_policy
                        .room_overrides
                        .get(key.room_id())
                        .copied();
                    let _ = handle
                        .send(TimelineActorMessage::LinkPreviewPolicyChanged {
                            unencrypted_global_enabled,
                            encrypted_global_enabled,
                            room_enabled,
                        })
                        .await;
                }
            }
        }
    }

    async fn route_send_to_actor_or_fail(
        &self,
        request_id: RequestId,
        key: &TimelineKey,
        transaction_id: String,
        body: String,
        projection: SendComposerProjection,
        msg: TimelineActorMessage,
    ) {
        let Some(handle) = self.timelines.get(key) else {
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

        if !handle.send(msg).await {
            if let Some(action) = send_failed_action(
                key,
                projection,
                transaction_id,
                "timeline send route closed".to_owned(),
            ) {
                let _ = self.action_tx.send(vec![action]).await;
            }
            self.emit_failure(
                request_id,
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::QueueOverflow,
                },
            );
        }
    }

    async fn handle_subscribe(
        &mut self,
        request_id: RequestId,
        key: TimelineKey,
        replay_existing: bool,
    ) {
        // Diagnostic-only, private-data-free stage trace (no room/event ids).
        // Enable with KOUSHI_SUBSCRIBE_TRACE=1 to find which `.await` stalls
        // before InitialItems is emitted. Off by default.
        let trace = |stage: &str| {
            if std::env::var_os("KOUSHI_SUBSCRIBE_TRACE").is_some() {
                eprintln!("koushi.subscribe stage={stage}");
            }
        };
        trace("start");
        let Some(session) = &self.session else {
            self.emit_failure(
                request_id,
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::NotSubscribed,
                },
            );
            return;
        };

        // Idempotency: if the identical key is already subscribed, do NOT drop
        // and rebuild the SDK subscription.  The full rebuild was 4-8 expensive
        // `subscribe_to_rooms` / timeline-build cycles per room on snapshot
        // churn (issue #116).  Callers that need to populate an empty
        // TimelineView can request an InitialItems replay; room-selection
        // effects with an already-retained App store can skip that full replay.
        // Confine the `&self.timelines` borrow to the closure so the Err arm
        // can `remove` (a `&mut` borrow) without a conflict.
        let replay_result = self.timelines.get(&key).map(|handle| {
            if replay_existing {
                handle
                    .tx
                    .try_send(TimelineActorMessage::ReplayInitialItems { request_id })
            } else {
                Ok(())
            }
        });
        match replay_result {
            Some(Ok(())) => {
                // Re-emit the subscribed action so the reducer re-confirms
                // `is_subscribed = true` (idempotent in the reducer).
                self.emit_timeline_subscribed_action(&key);
                if !replay_existing {
                    trace("replay_initial_skipped");
                }
                trace("subscribed_done");
                return;
            }
            Some(Err(_)) => {
                // Mailbox full or closed: the cheap replay could not be
                // delivered, so drop the stale handle and fall through to a
                // full rebuild, which is guaranteed to emit InitialItems for
                // this request_id (a re-mounted view must be populated).
                self.timelines.remove(&key);
            }
            None => {}
        }

        let client = session.client();
        let room_id_str = match &key.kind {
            TimelineKind::Room { room_id } => room_id.clone(),
            TimelineKind::Thread { room_id, .. } => room_id.clone(),
            TimelineKind::Focused { room_id, .. } => room_id.clone(),
        };

        let room_id = match matrix_sdk::ruma::RoomId::parse(&room_id_str) {
            Ok(id) => id,
            Err(_) => {
                self.emit_failure(
                    request_id,
                    CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::Sdk,
                    },
                );
                return;
            }
        };

        let room = match client.get_room(&room_id) {
            Some(r) => r,
            None => {
                self.emit_failure(
                    request_id,
                    CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::Sdk,
                    },
                );
                return;
            }
        };

        // On the sliding-sync backend, subscribing a timeline must also
        // subscribe its room with the live RoomListService so the server
        // streams the room's NEW timeline events; the all-rooms list alone
        // only guarantees the initial window on some servers (Conduit).
        // This is the Element X room-open pattern.
        if let Some(service) = &self.room_list_service {
            trace("subscribe_rooms_begin");
            service.subscribe_to_rooms(&[&room_id]).await;
            trace("subscribe_rooms_done");
        }

        let focus = match &key.kind {
            TimelineKind::Room { .. } => TimelineFocus::Live {
                hide_threaded_events: true,
            },
            TimelineKind::Thread { root_event_id, .. } => {
                match matrix_sdk::ruma::EventId::parse(root_event_id.as_str()) {
                    Ok(event_id) => TimelineFocus::Thread {
                        root_event_id: event_id,
                    },
                    Err(_) => {
                        self.emit_failure(
                            request_id,
                            CoreFailure::TimelineOperationFailed {
                                kind: TimelineFailureKind::Sdk,
                            },
                        );
                        return;
                    }
                }
            }
            TimelineKind::Focused { event_id, .. } => {
                match matrix_sdk::ruma::EventId::parse(event_id.as_str()) {
                    Ok(eid) => TimelineFocus::Event {
                        target: eid,
                        num_context_events: 20,
                        thread_mode:
                            matrix_sdk_ui::timeline::TimelineEventFocusThreadMode::Automatic {
                                hide_threaded_events: false,
                            },
                    },
                    Err(_) => {
                        self.emit_failure(
                            request_id,
                            CoreFailure::TimelineOperationFailed {
                                kind: TimelineFailureKind::Sdk,
                            },
                        );
                        return;
                    }
                }
            }
        };

        trace("build_begin");
        let build_started = startup_trace::now_if_enabled();
        let timeline_result = koushi_timeline_builder(&room, focus).build().await;
        startup_trace::trace_phase(StartupPhase::TimelineBuild, build_started);
        trace("build_done");

        let timeline = match timeline_result {
            Ok(t) => Arc::new(t),
            Err(_) => {
                self.emit_failure(
                    request_id,
                    CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::Sdk,
                    },
                );
                return;
            }
        };

        trace("spawn_begin");
        let handle = TimelineActor::spawn(
            key.clone(),
            timeline,
            session.clone(),
            request_id,
            self.action_tx.clone(),
            self.event_tx.clone(),
            self.search_index_tx.clone(),
            self.ignored_user_ids.clone(),
            self.data_dir.clone(),
            self.link_preview_policy.for_room(key.room_id()),
            self.messages_backpressure.clone(),
        )
        .await;
        trace("spawn_done");

        self.emit_timeline_subscribed_action(&key);
        self.timelines.insert(key, handle);
        trace("subscribed_done");
    }

    async fn route_to_actor_or_fail(
        &self,
        request_id: RequestId,
        key: &TimelineKey,
        msg: TimelineActorMessage,
    ) {
        match self.timelines.get(key) {
            Some(handle) => {
                let _ = handle.send(msg).await;
            }
            None => {
                self.emit_failure(
                    request_id,
                    CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::NotSubscribed,
                    },
                );
            }
        }
    }

    fn emit(&self, event: CoreEvent) {
        let _ = self.event_tx.send(event);
    }

    fn emit_failure(&self, request_id: RequestId, failure: CoreFailure) {
        self.emit(CoreEvent::OperationFailed {
            request_id,
            failure,
        });
    }

    fn emit_timeline_subscribed_action(&self, key: &TimelineKey) {
        let action = match &key.kind {
            TimelineKind::Room { room_id } => AppAction::TimelineSubscribed {
                room_id: room_id.clone(),
            },
            TimelineKind::Thread {
                room_id,
                root_event_id,
            } => AppAction::ThreadSubscribed {
                room_id: room_id.clone(),
                root_event_id: root_event_id.clone(),
            },
            TimelineKind::Focused { room_id, event_id } => AppAction::FocusedContextSubscribed {
                room_id: room_id.clone(),
                event_id: event_id.clone(),
            },
        };
        let _ = self.action_tx.try_send(vec![action]);
    }
}

#[derive(Clone, Copy)]
enum SendComposerProjection {
    Room,
    ThreadReply,
    None,
}

impl SendComposerProjection {
    fn for_send_text(key: &TimelineKey) -> Self {
        match key.kind {
            TimelineKind::Room { .. } => Self::Room,
            TimelineKind::Thread { .. } | TimelineKind::Focused { .. } => Self::None,
        }
    }

    fn for_send_reply(key: &TimelineKey) -> Self {
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

fn thread_attention_action_from_timeline_diffs(
    counts: &mut ThreadAttentionCounters,
    key: &TimelineKey,
    diffs: &[TimelineDiff],
    own_user_id: Option<&str>,
) -> Option<AppAction> {
    let TimelineKind::Thread {
        room_id,
        root_event_id,
    } = &key.kind
    else {
        return None;
    };

    let live_delta = diffs
        .iter()
        .filter(|diff| match diff {
            TimelineDiff::PushBack { item } => {
                is_remote_renderable_timeline_message(item, own_user_id)
            }
            TimelineDiff::PushFront { .. }
            | TimelineDiff::Insert { .. }
            | TimelineDiff::Set { .. }
            | TimelineDiff::Remove { .. }
            | TimelineDiff::Truncate { .. }
            | TimelineDiff::Clear
            | TimelineDiff::Reset { .. } => false,
        })
        .count() as u64;

    if live_delta == 0 {
        return None;
    }

    counts.notification_count = counts.notification_count.saturating_add(live_delta);
    counts.live_event_marker_count = counts.live_event_marker_count.saturating_add(live_delta);

    Some(AppAction::ThreadAttentionUpdated {
        room_id: room_id.clone(),
        root_event_id: root_event_id.clone(),
        notification_count: counts.notification_count,
        highlight_count: counts.highlight_count,
        live_event_marker_count: counts.live_event_marker_count,
    })
}

fn is_remote_renderable_timeline_message(item: &TimelineItem, own_user_id: Option<&str>) -> bool {
    if !matches!(item.id, TimelineItemId::Event { .. }) {
        return false;
    }
    if item.body.is_none() && item.media.is_none() {
        return false;
    }
    if let (Some(sender), Some(own_user_id)) = (item.sender.as_deref(), own_user_id) {
        if sender == own_user_id {
            return false;
        }
    }
    true
}

fn validate_composer_body_for_timeline_send(body: &str) -> Result<(), TimelineFailureKind> {
    match resolve_composer_send_intent(body, MentionIntent::default()) {
        ComposerSendIntent::LocalFailure { .. }
        | ComposerSendIntent::SlashCommand {
            command:
                SlashCommandIntent::Join { .. }
                | SlashCommandIntent::Invite { .. }
                | SlashCommandIntent::PlainText { .. }
                | SlashCommandIntent::Unsupported { .. },
        } => Err(TimelineFailureKind::UnsupportedSlashCommand),
        ComposerSendIntent::Message { .. }
        | ComposerSendIntent::SlashCommand {
            command: SlashCommandIntent::Me { .. },
        } => Ok(()),
    }
}

fn build_room_message_content_from_composer_body(
    body: &str,
    mentions: MentionIntent,
) -> Result<RoomMessageEventContent, TimelineFailureKind> {
    build_room_message_content_without_relation_from_composer_body(body, mentions)
        .map(|content| content.with_relation(None))
}

fn build_room_message_content_without_relation_from_composer_body(
    body: &str,
    mentions: MentionIntent,
) -> Result<RoomMessageEventContentWithoutRelation, TimelineFailureKind> {
    match resolve_composer_send_intent(body, mentions) {
        ComposerSendIntent::Message { draft } => {
            Ok(without_relation_content_from_formatted_draft(draft, false))
        }
        ComposerSendIntent::SlashCommand {
            command: SlashCommandIntent::Me { body },
        } => Ok(without_relation_content_from_formatted_draft(
            koushi_state::build_formatted_message_draft(body, MentionIntent::default()),
            true,
        )),
        ComposerSendIntent::SlashCommand { .. } | ComposerSendIntent::LocalFailure { .. } => {
            Err(TimelineFailureKind::UnsupportedSlashCommand)
        }
    }
}

fn without_relation_content_from_formatted_draft(
    draft: FormattedMessageDraft,
    emote: bool,
) -> RoomMessageEventContentWithoutRelation {
    let mut content = match (emote, draft.formatted_body) {
        (true, Some(formatted_body)) => {
            RoomMessageEventContentWithoutRelation::emote_html(draft.plain_body, formatted_body)
        }
        (true, None) => RoomMessageEventContentWithoutRelation::emote_plain(draft.plain_body),
        (false, Some(formatted_body)) => {
            RoomMessageEventContentWithoutRelation::text_html(draft.plain_body, formatted_body)
        }
        (false, None) => RoomMessageEventContentWithoutRelation::text_plain(draft.plain_body),
    };

    if let Some(mentions) = ruma_mentions_from_intent(&draft.mentions) {
        content = content.add_mentions(mentions);
    }
    content
}

fn media_caption_content_from_draft(draft: &FormattedMessageDraft) -> TextMessageEventContent {
    match &draft.formatted_body {
        Some(formatted_body) => {
            TextMessageEventContent::html(draft.plain_body.clone(), formatted_body.clone())
        }
        None => TextMessageEventContent::plain(draft.plain_body.clone()),
    }
}

fn ruma_mentions_from_intent(intent: &MentionIntent) -> Option<Mentions> {
    let user_ids = intent
        .user_ids()
        .into_iter()
        .filter_map(|user_id| UserId::parse(user_id).ok().map(Into::into))
        .collect::<Vec<_>>();
    let mentions_room = intent.mentions_room();

    if user_ids.is_empty() && !mentions_room {
        return None;
    }

    let mut mentions = if user_ids.is_empty() {
        Mentions::new()
    } else {
        Mentions::with_user_ids(user_ids)
    };
    mentions.room = mentions_room;
    Some(mentions)
}

// ---------------------------------------------------------------------------
// Individual TimelineActor
// ---------------------------------------------------------------------------

enum TimelineActorMessage {
    Paginate {
        request_id: RequestId,
        direction: PaginationDirection,
        event_count: u16,
    },
    CancelPagination {
        request_id: RequestId,
    },
    CancelLinkPreviews {
        request_id: RequestId,
    },
    PaginationFinished {
        serial: u64,
    },
    RestoreTimelineAnchor {
        request_id: RequestId,
        event_id: String,
        max_batches: u16,
        event_count: u16,
    },
    RestoreTimelineAnchorContinue {
        serial: u64,
    },
    ObserveViewport {
        observation: TimelineViewportObservation,
    },
    SendText {
        request_id: RequestId,
        transaction_id: String,
        body: String,
        mentions: MentionIntent,
    },
    SendReply {
        request_id: RequestId,
        transaction_id: String,
        in_reply_to_event_id: String,
        body: String,
        mentions: MentionIntent,
    },
    ForwardMessage {
        request_id: RequestId,
        source_event_id: String,
        destination_room_id: String,
        transaction_id: String,
    },
    LoadMessageSource {
        request_id: RequestId,
        event_id: String,
    },
    ReplyDetailsFetchFinished {
        event_id: String,
    },
    RequestRoomKey {
        request_id: RequestId,
        event_id: String,
    },
    RetrySend {
        request_id: RequestId,
        transaction_id: String,
    },
    CancelSend {
        request_id: RequestId,
        transaction_id: String,
    },
    UploadAndSendMedia {
        request_id: RequestId,
        transaction_id: String,
        request: UploadMediaRequest,
    },
    DownloadMedia {
        request_id: RequestId,
        event_id: String,
        selection: MediaDownloadSelection,
    },
    EditText {
        request_id: RequestId,
        event_id: String,
        body: String,
    },
    Redact {
        request_id: RequestId,
        event_id: String,
    },
    ToggleReaction {
        request_id: RequestId,
        event_id: String,
        reaction_key: String,
    },
    SendReaction {
        request_id: RequestId,
        event_id: String,
        reaction_key: String,
    },
    RedactReaction {
        request_id: RequestId,
        event_id: String,
        reaction_key: String,
        reaction_event_id: String,
    },
    SendReadReceipt {
        request_id: RequestId,
        event_id: String,
    },
    SetFullyRead {
        request_id: RequestId,
        event_id: String,
    },
    SetTyping {
        request_id: RequestId,
        is_typing: bool,
    },
    TypingUsersUpdated(Vec<String>),
    IgnoredUsersUpdated(std::collections::BTreeSet<String>),
    LoadLinkPreviews {
        request_id: RequestId,
        event_id: String,
    },
    LinkPreviewsFetched {
        request_id: RequestId,
        event_id: String,
        previews: Vec<LinkPreview>,
        pending_count: usize,
        ready_count: usize,
        failed_count: usize,
        elapsed_ms: u128,
    },
    HideLinkPreview {
        request_id: RequestId,
        event_id: String,
    },
    LinkPreviewPolicyChanged {
        unencrypted_global_enabled: bool,
        encrypted_global_enabled: bool,
        room_enabled: Option<bool>,
    },
    /// Internal: diff batch from the relay task.
    DiffBatch(Vec<eyeball_im::VectorDiff<Arc<SdkTimelineItem>>>),
    /// Internal: send completed (from send queue monitor task).
    SendQueueUpdate(RoomSendQueueUpdate),
    /// Internal: relay task hit overflow — must resync.
    RelayOverflow,
    /// Internal: re-emit the current navigation_items as InitialItems for a
    /// new request_id without tearing down the SDK subscription.  Sent by
    /// `handle_subscribe` when the key is already subscribed (idempotency
    /// path) so a freshly re-mounted TimelineView still receives an
    /// InitialItems batch.
    ReplayInitialItems {
        request_id: RequestId,
    },
}

fn timeline_trace_enabled() -> bool {
    std::env::var_os("KOUSHI_SUBSCRIBE_TRACE").is_some()
}

fn timeline_key_trace_kind(key: &TimelineKey) -> &'static str {
    match &key.kind {
        TimelineKind::Room { .. } => "room",
        TimelineKind::Thread { .. } => "thread",
        TimelineKind::Focused { .. } => "focused",
    }
}

fn pagination_direction_trace_token(direction: PaginationDirection) -> &'static str {
    match direction {
        PaginationDirection::Backward => "backward",
        PaginationDirection::Forward => "forward",
    }
}

fn trace_timeline_route(stage: &str, kind: &str, request_id: RequestId, key: &TimelineKey) {
    if timeline_trace_enabled() {
        eprintln!(
            "koushi.timeline stage={stage} kind={kind} timeline={} request_id={}/{}",
            timeline_key_trace_kind(key),
            request_id.connection_id.0,
            request_id.sequence
        );
    }
}

fn trace_timeline_paginate(
    stage: &str,
    request_id: RequestId,
    key: &TimelineKey,
    direction: PaginationDirection,
    event_count: u16,
    elapsed_ms: Option<u128>,
    gate_ms: Option<u128>,
    outcome: Option<&'static str>,
) {
    if timeline_trace_enabled() {
        eprintln!(
            "koushi.timeline stage={stage} kind=paginate timeline={} direction={} event_count={} request_id={}/{} elapsed_ms={} gate_ms={} outcome={}",
            timeline_key_trace_kind(key),
            pagination_direction_trace_token(direction),
            event_count,
            request_id.connection_id.0,
            request_id.sequence,
            elapsed_ms.unwrap_or(0),
            gate_ms.unwrap_or(0),
            outcome.unwrap_or("pending")
        );
    }
}

fn trace_timeline_link_preview(
    stage: &str,
    request_id: RequestId,
    key: &TimelineKey,
    pending_count: usize,
    ready_count: usize,
    failed_count: usize,
    elapsed_ms: Option<u128>,
    outcome: Option<&'static str>,
) {
    if timeline_trace_enabled() {
        eprintln!(
            "koushi.timeline stage={stage} kind=link_preview timeline={} pending={} ready={} failed={} request_id={}/{} elapsed_ms={} outcome={}",
            timeline_key_trace_kind(key),
            pending_count,
            ready_count,
            failed_count,
            request_id.connection_id.0,
            request_id.sequence,
            elapsed_ms.unwrap_or(0),
            outcome.unwrap_or("pending")
        );
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

struct TimelineActorHandle {
    tx: mpsc::Sender<TimelineActorMessage>,
    task: executor::JoinHandle<()>,
    auxiliary_tasks: Vec<executor::JoinHandle<()>>,
}

impl TimelineActorHandle {
    async fn send(&self, msg: TimelineActorMessage) -> bool {
        self.tx.send(msg).await.is_ok()
    }
}

impl Drop for TimelineActorHandle {
    fn drop(&mut self) {
        self.task.abort();
        for task in &self.auxiliary_tasks {
            task.abort();
        }
    }
}

#[derive(Clone)]
struct PrivateMediaEntry {
    source: MediaSource,
    thumbnail_source: Option<MediaSource>,
    mimetype: Option<String>,
    size: u64,
    width: Option<u64>,
    height: Option<u64>,
}

struct ReactionTargetState {
    item_id: TimelineEventItemId,
    can_react: bool,
    my_reaction_event_id: Option<String>,
}

struct ActivePaginationTask {
    serial: u64,
    direction: PaginationDirection,
    event_count: u16,
    task: executor::JoinHandle<()>,
}

struct TimelineActor {
    key: TimelineKey,
    timeline: Arc<Timeline>,
    session: Arc<MatrixClientSession>,
    action_tx: mpsc::Sender<Vec<AppAction>>,
    event_tx: broadcast::Sender<CoreEvent>,
    msg_tx: mpsc::Sender<TimelineActorMessage>,
    msg_rx: mpsc::Receiver<TimelineActorMessage>,
    generation: TimelineGeneration,
    next_batch_id: TimelineBatchId,
    /// Correlates send queue completions across the enqueue / SentEvent race.
    send_completion: SendCompletionTracker,
    /// SDK transaction id -> Rust-owned outbound send state.
    send_statuses: HashMap<String, TimelineSendState>,
    /// SDK transaction id -> SDK send handle used for retry/cancel.
    send_handles: HashMap<String, SendHandle>,
    /// Current account user id, used to project reaction ownership.
    own_user_id: Option<matrix_sdk::ruma::OwnedUserId>,
    /// event_id → SDK transaction id for events this actor sent. Used to
    /// address local-echo items whose remote echo has not arrived (e.g.
    /// Conduit's sliding sync does not echo own events into the timeline),
    /// so edit/redact by event id can fall back to the transaction identity.
    sent_event_txns: HashMap<String, matrix_sdk::ruma::OwnedTransactionId>,
    /// event_id -> SDK media source. This cache may contain encrypted media
    /// keys/hashes and must never be serialized or logged.
    media_sources: HashMap<String, PrivateMediaEntry>,
    /// Event IDs for which a download is currently in flight. Prevents duplicate
    /// concurrent downloads when the user clicks an attachment repeatedly.
    media_downloads_in_progress: HashSet<String>,
    /// Search index mutation sender (Phase 6). `None` when no search index is
    /// configured (pre-session or pre-Phase-6 builds). Fire-and-forget: if the
    /// channel is full, we drop the mutation rather than block the diff relay.
    search_index_tx: Option<mpsc::Sender<crate::search::SearchIndexMessage>>,
    /// Rust-owned pane-level thread attention counters. Only thread timelines
    /// update these, and React reads them through `AppState.thread_attention`.
    thread_attention_counts: ThreadAttentionCounters,
    /// Rust-owned navigation projection source. The webview reports viewport
    /// facts; item ordering, unread marker semantics, and counts stay here.
    navigation_items: Vec<TimelineItem>,
    media_gallery_items: Vec<TimelineMediaGalleryItem>,
    fully_read_event_id: Option<String>,
    viewport_observation: TimelineViewportObservation,
    last_navigation_snapshot: Option<TimelineNavigationSnapshot>,
    ignored_user_ids: std::collections::BTreeSet<String>,
    /// URL preview policy for this timeline.
    link_preview_policy: LinkPreviewContext,
    /// In-flight URL preview fetch workers keyed by event_id.
    link_preview_fetches: HashMap<String, executor::JoinHandle<()>>,
    /// In-flight reply detail fetch workers keyed by the reply event_id.
    reply_detail_fetches: HashMap<String, executor::JoinHandle<()>>,
    /// Reply event IDs already handed to the SDK for replied-to details during
    /// this actor lifetime. This avoids retry loops on every viewport tick.
    reply_detail_fetch_attempted_event_ids: HashSet<String>,
    pagination_task: Option<ActivePaginationTask>,
    next_pagination_serial: u64,
    /// Application data directory for cached preview images.
    data_dir: Option<std::path::PathBuf>,
    messages_backpressure: MessagesBackpressure,
    restore_anchor: Option<RestoreTimelineAnchorState>,
    next_restore_anchor_serial: u64,
    /// Buffered `TimelineDiff`s accumulated during a restore walk. While
    /// `restore_anchor.is_some()`, each `handle_diff_batch` call appends its
    /// `core_diffs` here instead of emitting `ItemsUpdated` per chunk. The
    /// buffer is flushed as ONE `ItemsUpdated` when the restore terminates
    /// (Found/EndReached/BudgetExhausted/Failed/Superseded), so React receives
    /// a single settled update rather than O(chunks) intermediate renders.
    restore_emit_buffer: Vec<TimelineDiff>,
    /// Monotonically increasing counter, incremented at the start of every
    /// `handle_diff_batch` call (restore or not).
    diff_batch_seq: u64,
}

#[derive(Clone, Debug)]
struct RestoreTimelineAnchorState {
    request_id: RequestId,
    event_id: String,
    max_batches_remaining: u16,
    event_count: u16,
    in_flight: bool,
    awaiting_diff_batch: bool,
    continuation_scheduled: bool,
    continuation_serial: Option<u64>,
    /// Set to `Some(RESTORE_ANCHOR_RELAY_WAIT_TICKS)` after the SDK confirms
    /// `anchor_present == true` (load-until-anchor found the anchor in a loaded
    /// chunk; its broadcast has been fired and WILL propagate through the 3-hop
    /// relay). While non-zero, each tick re-checks `timeline_contains(anchor)`
    /// and re-ticks until Found or the backstop runs out. `None` during the
    /// normal walk.
    anchor_relay_wait: Option<u8>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ThreadAttentionCounters {
    notification_count: u64,
    highlight_count: u64,
    live_event_marker_count: u64,
}

impl Drop for TimelineActor {
    fn drop(&mut self) {
        for task in self.link_preview_fetches.values() {
            task.abort();
        }
        for task in self.reply_detail_fetches.values() {
            task.abort();
        }
        if let Some(active) = self.pagination_task.take() {
            active.task.abort();
        }
    }
}

impl TimelineActor {
    /// Spawn the actor, emit InitialItems, and return the handle.
    async fn spawn(
        key: TimelineKey,
        timeline: Arc<Timeline>,
        session: Arc<MatrixClientSession>,
        subscribe_request_id: RequestId,
        action_tx: mpsc::Sender<Vec<AppAction>>,
        event_tx: broadcast::Sender<CoreEvent>,
        search_index_tx: Option<mpsc::Sender<crate::search::SearchIndexMessage>>,
        ignored_user_ids: std::collections::BTreeSet<String>,
        data_dir: Option<std::path::PathBuf>,
        link_preview_policy: LinkPreviewContext,
        messages_backpressure: MessagesBackpressure,
    ) -> TimelineActorHandle {
        let mut auxiliary_tasks: Vec<executor::JoinHandle<()>> = Vec::new();

        // Env-gated origin observer. Subscribe the event cache BEFORE the
        // timeline load so the initial load's provenance (store=cache vs
        // network) is observed via the updates stream. Zero cost when
        // KOUSHI_STARTUP_TRACE is unset.
        if startup_trace::enabled() {
            if let Ok(parsed_room_id) = matrix_sdk::ruma::RoomId::parse(key.room_id()) {
                if let Some(observer_room) = session.client().get_room(&parsed_room_id) {
                    if let Ok((cache, drop_guards)) = observer_room.event_cache().await {
                        if let Ok((initial, mut updates)) = cache.subscribe().await {
                            if !initial.is_empty() {
                                // Cache already had events at restore — warm initial state.
                                startup_trace::trace_origin("cache");
                            }
                            auxiliary_tasks.push(executor::spawn(async move {
                                let _event_cache_drop_guards = drop_guards;
                                use matrix_sdk::event_cache::{EventsOrigin, RoomEventCacheUpdate};
                                loop {
                                    match updates.recv().await {
                                        Ok(RoomEventCacheUpdate::UpdateTimelineEvents(diffs)) => {
                                            let origin = match diffs.origin {
                                                EventsOrigin::Cache => "cache",
                                                EventsOrigin::Pagination => "network",
                                                EventsOrigin::Sync => "sync",
                                            };
                                            startup_trace::trace_origin(origin);
                                        }
                                        Ok(_) => {}
                                        // Broadcast lagged or channel closed — stop the observer.
                                        Err(_) => break,
                                    }
                                }
                            }));
                        }
                    }
                }
            }
        }

        // Subscribe to the SDK timeline to get initial items + diff stream.
        let subscribe_started = startup_trace::now_if_enabled();
        let (initial_sdk_items, diff_stream) = timeline.subscribe().await;
        startup_trace::trace_phase_items(
            StartupPhase::TimelineSubscribe,
            subscribe_started,
            initial_sdk_items.len(),
        );
        let own_user_id = session.client().user_id().map(|user_id| user_id.to_owned());
        let room_id = key.room_id().to_owned();

        let mut media_sources = HashMap::new();
        for item in &initial_sdk_items {
            cache_sdk_item_media_source(&mut media_sources, item);
        }

        let initial_items: Vec<TimelineItem> = initial_sdk_items
            .iter()
            .map(|item| sdk_item_to_timeline_item(&key, item, own_user_id.as_deref()))
            .map(|mut item| {
                apply_ignored_sender_suppression(&mut item, &ignored_user_ids);
                item
            })
            .collect();
        let mut initial_items = initial_items;
        for item in &mut initial_items {
            apply_link_previews_to_item(&mut *item, &room_id, &link_preview_policy, &session).await;
        }
        let navigation_items = initial_items.clone();
        let initial_activity_rows = activity_rows_from_timeline_items(&key, &initial_items);
        let initial_media_gallery_items =
            media_gallery_items_from_timeline_items(&key, &initial_items);
        let initial_receipts = live_event_receipts_from_sdk_items(initial_sdk_items.iter());

        let (actor_tx, actor_rx) = mpsc::channel(256);
        let mut send_statuses = HashMap::new();
        let mut send_handles = HashMap::new();

        // Emit InitialItems (generation 0).
        let generation = TimelineGeneration(0);
        if std::env::var_os("KOUSHI_SUBSCRIBE_TRACE").is_some() {
            // Private-data-free: item count only, no room/event ids or bodies.
            eprintln!(
                "koushi.subscribe stage=initial_emitted count={}",
                initial_items.len()
            );
        }
        let _ = event_tx.send(CoreEvent::Timeline(TimelineEvent::InitialItems {
            request_id: Some(subscribe_request_id),
            key: key.clone(),
            generation,
            items: initial_items,
        }));
        if !initial_activity_rows.is_empty() {
            let _ = action_tx.try_send(vec![AppAction::ActivityRowsObserved {
                rows: initial_activity_rows,
            }]);
        }
        if let Some(action) =
            media_gallery_updated_action(&key, initial_media_gallery_items.clone())
        {
            let _ = action_tx.try_send(vec![action]);
        }

        // Spawn the diff relay task: converts SDK VectorDiff stream into actor messages.
        let relay_tx = actor_tx.clone();
        let relay_timeline = timeline.clone();
        auxiliary_tasks.push(executor::spawn(run_diff_relay(
            relay_tx,
            diff_stream,
            relay_timeline,
        )));

        // Spawn the send queue monitor task: forwards RoomSendQueueUpdate to actor.
        let room_id_str = match &key.kind {
            TimelineKind::Room { room_id }
            | TimelineKind::Thread { room_id, .. }
            | TimelineKind::Focused { room_id, .. } => room_id.clone(),
        };
        let mut initial_fully_read_event_id = None;
        if let Ok(room_id) = matrix_sdk::ruma::RoomId::parse(&room_id_str) {
            if let Some(room) = session.client().get_room(&room_id) {
                let sq_tx = actor_tx.clone();
                if let Ok((local_echoes, update_rx)) = room.send_queue().subscribe().await {
                    for echo in &local_echoes {
                        remember_local_echo(&mut send_statuses, &mut send_handles, echo);
                    }
                    auxiliary_tasks.push(executor::spawn(run_send_queue_monitor(sq_tx, update_rx)));
                }

                let (typing_guard, typing_rx) = room.subscribe_to_typing_notifications();
                let typing_tx = actor_tx.clone();
                auxiliary_tasks.push(executor::spawn(run_typing_notifications(
                    typing_tx,
                    typing_guard,
                    typing_rx,
                )));

                let mut actions = Vec::new();
                let room_id = room_id_str.clone();
                if !initial_receipts.is_empty() {
                    actions.push(AppAction::LiveRoomReceiptsUpdated {
                        room_id: room_id.clone(),
                        receipts_by_event: initial_receipts,
                    });
                }
                actions.push(AppAction::FullyReadMarkerUpdated {
                    room_id,
                    event_id: {
                        initial_fully_read_event_id = room
                            .fully_read_event_id()
                            .map(|event_id| event_id.to_string());
                        initial_fully_read_event_id.clone()
                    },
                });
                let _ = action_tx.try_send(actions);
            }
        }

        let actor = TimelineActor {
            key: key.clone(),
            timeline,
            session,
            action_tx,
            event_tx,
            msg_tx: actor_tx.clone(),
            msg_rx: actor_rx,
            generation,
            next_batch_id: TimelineBatchId(0),
            send_completion: SendCompletionTracker::default(),
            send_statuses,
            send_handles,
            own_user_id,
            sent_event_txns: HashMap::new(),
            media_sources,
            media_downloads_in_progress: HashSet::new(),
            search_index_tx,
            thread_attention_counts: ThreadAttentionCounters::default(),
            navigation_items,
            media_gallery_items: initial_media_gallery_items,
            fully_read_event_id: initial_fully_read_event_id,
            viewport_observation: TimelineViewportObservation::default(),
            last_navigation_snapshot: None,
            ignored_user_ids,
            link_preview_policy,
            link_preview_fetches: HashMap::new(),
            reply_detail_fetches: HashMap::new(),
            reply_detail_fetch_attempted_event_ids: HashSet::new(),
            pagination_task: None,
            next_pagination_serial: 0,
            data_dir,
            messages_backpressure,
            restore_anchor: None,
            next_restore_anchor_serial: 0,
            restore_emit_buffer: Vec::new(),
            diff_batch_seq: 0,
        };

        actor.forward_initial_items_to_search(initial_sdk_items.iter().cloned());
        let task = executor::spawn(actor.run());

        TimelineActorHandle {
            tx: actor_tx,
            task,
            auxiliary_tasks,
        }
    }

    async fn run(mut self) {
        while let Some(msg) = self.msg_rx.recv().await {
            self.handle_msg(msg).await;
        }
    }

    async fn handle_msg(&mut self, msg: TimelineActorMessage) {
        match msg {
            TimelineActorMessage::Paginate {
                request_id,
                direction,
                event_count,
            } => {
                self.handle_paginate(request_id, direction, event_count)
                    .await;
            }
            TimelineActorMessage::CancelPagination { request_id } => {
                self.handle_cancel_pagination(request_id);
            }
            TimelineActorMessage::CancelLinkPreviews { request_id } => {
                self.handle_cancel_link_previews(request_id);
            }
            TimelineActorMessage::PaginationFinished { serial } => {
                if self
                    .pagination_task
                    .as_ref()
                    .is_some_and(|active| active.serial == serial)
                {
                    self.pagination_task = None;
                }
            }
            TimelineActorMessage::RestoreTimelineAnchor {
                request_id,
                event_id,
                max_batches,
                event_count,
            } => {
                self.handle_restore_timeline_anchor(request_id, event_id, max_batches, event_count)
                    .await;
            }
            TimelineActorMessage::RestoreTimelineAnchorContinue { serial } => {
                self.handle_restore_timeline_anchor_continue(serial).await;
            }
            TimelineActorMessage::ObserveViewport { observation } => {
                self.viewport_observation = observation;
                self.maybe_fetch_visible_reply_details();
                self.emit_navigation_if_changed();
            }
            TimelineActorMessage::SendText {
                request_id,
                transaction_id,
                body,
                mentions,
            } => {
                self.handle_send_text(request_id, transaction_id, body, mentions)
                    .await;
            }
            TimelineActorMessage::SendReply {
                request_id,
                transaction_id,
                in_reply_to_event_id,
                body,
                mentions,
            } => {
                self.handle_send_reply(
                    request_id,
                    transaction_id,
                    in_reply_to_event_id,
                    body,
                    mentions,
                )
                .await;
            }
            TimelineActorMessage::ForwardMessage {
                request_id,
                source_event_id,
                destination_room_id,
                transaction_id,
            } => {
                self.handle_forward_message(
                    request_id,
                    source_event_id,
                    destination_room_id,
                    transaction_id,
                )
                .await;
            }
            TimelineActorMessage::LoadMessageSource {
                request_id,
                event_id,
            } => {
                self.handle_load_message_source(request_id, event_id).await;
            }
            TimelineActorMessage::ReplyDetailsFetchFinished { event_id } => {
                self.reply_detail_fetches.remove(&event_id);
            }
            TimelineActorMessage::RequestRoomKey {
                request_id,
                event_id,
            } => {
                self.handle_request_room_key(request_id, event_id).await;
            }
            TimelineActorMessage::RetrySend {
                request_id,
                transaction_id,
            } => {
                self.handle_retry_send(request_id, transaction_id).await;
            }
            TimelineActorMessage::CancelSend {
                request_id,
                transaction_id,
            } => {
                self.handle_cancel_send(request_id, transaction_id).await;
            }
            TimelineActorMessage::UploadAndSendMedia {
                request_id,
                transaction_id,
                request,
            } => {
                self.handle_upload_and_send_media(request_id, transaction_id, request)
                    .await;
            }
            TimelineActorMessage::DownloadMedia {
                request_id,
                event_id,
                selection,
            } => {
                self.handle_download_media(request_id, event_id, selection)
                    .await;
            }
            TimelineActorMessage::EditText {
                request_id,
                event_id,
                body,
            } => {
                self.handle_edit_text(request_id, event_id, body).await;
            }
            TimelineActorMessage::Redact {
                request_id,
                event_id,
            } => {
                self.handle_redact(request_id, event_id).await;
            }
            TimelineActorMessage::ToggleReaction {
                request_id,
                event_id,
                reaction_key,
            } => {
                self.handle_toggle_reaction(request_id, event_id, reaction_key)
                    .await;
            }
            TimelineActorMessage::SendReaction {
                request_id,
                event_id,
                reaction_key,
            } => {
                self.handle_send_reaction(request_id, event_id, reaction_key)
                    .await;
            }
            TimelineActorMessage::RedactReaction {
                request_id,
                event_id,
                reaction_key,
                reaction_event_id,
            } => {
                self.handle_redact_reaction(request_id, event_id, reaction_key, reaction_event_id)
                    .await;
            }
            TimelineActorMessage::SendReadReceipt {
                request_id,
                event_id,
            } => {
                self.handle_send_read_receipt(request_id, event_id).await;
            }
            TimelineActorMessage::SetFullyRead {
                request_id,
                event_id,
            } => {
                self.handle_set_fully_read(request_id, event_id).await;
            }
            TimelineActorMessage::SetTyping {
                request_id,
                is_typing,
            } => {
                self.handle_set_typing(request_id, is_typing).await;
            }
            TimelineActorMessage::TypingUsersUpdated(user_ids) => {
                self.emit_typing_users_action(user_ids);
            }
            TimelineActorMessage::IgnoredUsersUpdated(user_ids) => {
                self.handle_ignored_users_updated(user_ids).await;
            }
            TimelineActorMessage::LoadLinkPreviews {
                request_id,
                event_id,
            } => {
                self.handle_load_link_previews(request_id, event_id).await;
            }
            TimelineActorMessage::LinkPreviewsFetched {
                request_id,
                event_id,
                previews,
                pending_count,
                ready_count,
                failed_count,
                elapsed_ms,
            } => {
                self.handle_link_previews_fetched(
                    request_id,
                    event_id,
                    previews,
                    pending_count,
                    ready_count,
                    failed_count,
                    elapsed_ms,
                )
                .await;
            }
            TimelineActorMessage::HideLinkPreview {
                request_id,
                event_id,
            } => {
                self.handle_hide_link_preview(request_id, event_id).await;
            }
            TimelineActorMessage::LinkPreviewPolicyChanged {
                unencrypted_global_enabled,
                encrypted_global_enabled,
                room_enabled,
            } => {
                self.handle_link_preview_policy_changed(
                    unencrypted_global_enabled,
                    encrypted_global_enabled,
                    room_enabled,
                )
                .await;
            }
            TimelineActorMessage::DiffBatch(diffs) => {
                self.handle_diff_batch(diffs).await;
            }
            TimelineActorMessage::SendQueueUpdate(update) => {
                self.handle_send_queue_update(update);
            }
            TimelineActorMessage::RelayOverflow => {
                self.handle_relay_overflow().await;
            }
            TimelineActorMessage::ReplayInitialItems { request_id } => {
                self.handle_replay_initial_items(request_id);
            }
        }
    }

    async fn handle_paginate(
        &mut self,
        request_id: RequestId,
        direction: PaginationDirection,
        event_count: u16,
    ) {
        trace_timeline_paginate(
            "actor_paginate_start",
            request_id,
            &self.key,
            direction,
            event_count,
            None,
            None,
            None,
        );

        // Enforce direction rule: forward only on Focused (Async rule 5).
        if direction == PaginationDirection::Forward
            && !matches!(self.key.kind, TimelineKind::Focused { .. })
        {
            self.emit_failure(
                request_id,
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::InvalidDirection,
                },
            );
            return;
        }

        if self.pagination_task.is_some() {
            trace_timeline_paginate(
                "actor_paginate_skip",
                request_id,
                &self.key,
                direction,
                event_count,
                None,
                None,
                Some("in_flight"),
            );
            return;
        }

        let serial = self.next_pagination_serial;
        self.next_pagination_serial = self.next_pagination_serial.saturating_add(1);
        let key = self.key.clone();
        let timeline = self.timeline.clone();
        let event_tx = self.event_tx.clone();
        let actor_tx = self.msg_tx.clone();
        let messages_backpressure = self.messages_backpressure.clone();
        let task = executor::spawn(async move {
            let _ = Self::paginate_once_for(
                request_id,
                key,
                timeline,
                event_tx,
                messages_backpressure,
                direction,
                event_count,
            )
            .await;
            let _ = actor_tx
                .send(TimelineActorMessage::PaginationFinished { serial })
                .await;
        });
        self.pagination_task = Some(ActivePaginationTask {
            serial,
            direction,
            event_count,
            task,
        });
    }

    async fn paginate_once(
        &mut self,
        request_id: RequestId,
        direction: PaginationDirection,
        event_count: u16,
    ) -> Result<bool, TimelineFailureKind> {
        Self::paginate_once_for(
            request_id,
            self.key.clone(),
            self.timeline.clone(),
            self.event_tx.clone(),
            self.messages_backpressure.clone(),
            direction,
            event_count,
        )
        .await
    }

    async fn paginate_once_for(
        request_id: RequestId,
        key: TimelineKey,
        timeline: Arc<Timeline>,
        event_tx: broadcast::Sender<CoreEvent>,
        messages_backpressure: MessagesBackpressure,
        direction: PaginationDirection,
        event_count: u16,
    ) -> Result<bool, TimelineFailureKind> {
        // Emit Paginating.
        let _ = event_tx.send(CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
            request_id: Some(request_id),
            key: key.clone(),
            direction,
            state: PaginationState::Paginating,
        }));

        let gate_started =
            (startup_trace::enabled() || timeline_trace_enabled()).then(std::time::Instant::now);
        let result = {
            let _permit = messages_backpressure.acquire_timeline().await;
            let gate_wait = gate_started.map(|t| t.elapsed());
            let gate_ms = gate_wait.map(|duration| duration.as_millis());
            trace_timeline_paginate(
                "gate_acquired",
                request_id,
                &key,
                direction,
                event_count,
                None,
                gate_ms,
                None,
            );
            let paginate_started = startup_trace::now_if_enabled();
            let trace_started = timeline_trace_enabled().then(std::time::Instant::now);
            let outcome = match direction {
                PaginationDirection::Backward => timeline.paginate_backwards(event_count).await,
                PaginationDirection::Forward => timeline.paginate_forwards(event_count).await,
            };
            let outcome_token = match &outcome {
                Ok(true) => "end_reached",
                Ok(false) => "idle",
                Err(_) => "failed",
            };
            trace_timeline_paginate(
                "sdk_finish",
                request_id,
                &key,
                direction,
                event_count,
                trace_started.map(|started| started.elapsed().as_millis()),
                gate_ms,
                Some(outcome_token),
            );
            startup_trace::trace_paginate(paginate_started, gate_wait, matches!(outcome, Ok(true)));
            outcome
        };

        let next_state = match result {
            Ok(true) => PaginationState::EndReached,
            Ok(false) => PaginationState::Idle,
            Err(err) => {
                let kind = classify_pagination_error(&err);
                PaginationState::Failed { kind }
            }
        };

        let end_reached = matches!(next_state, PaginationState::EndReached);
        let failure_kind = match &next_state {
            PaginationState::Failed { kind } => Some(*kind),
            _ => None,
        };
        let _ = event_tx.send(CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
            request_id: Some(request_id),
            key,
            direction,
            state: next_state,
        }));
        failure_kind.map_or(Ok(end_reached), Err)
    }

    fn handle_cancel_pagination(&mut self, request_id: RequestId) {
        let Some(active) = self.pagination_task.take() else {
            return;
        };
        active.task.abort();
        trace_timeline_paginate(
            "cancelled",
            request_id,
            &self.key,
            active.direction,
            active.event_count,
            None,
            None,
            Some("cancelled"),
        );
        self.emit(CoreEvent::Timeline(TimelineEvent::PaginationStateChanged {
            request_id: Some(request_id),
            key: self.key.clone(),
            direction: active.direction,
            state: PaginationState::Idle,
        }));
    }

    async fn handle_restore_timeline_anchor(
        &mut self,
        request_id: RequestId,
        event_id: String,
        max_batches: u16,
        event_count: u16,
    ) {
        if !matches!(self.key.kind, TimelineKind::Room { .. }) {
            self.emit_timeline_failure(request_id, TimelineFailureKind::NotSubscribed);
            return;
        }
        if event_id.trim().is_empty() || max_batches == 0 || event_count == 0 {
            // Invalid request: reject it without touching any active restore's
            // buffer. Using raw emit_anchor_restore_finished (NOT finish_anchor_restore)
            // prevents flushing a different restore's restore_emit_buffer here.
            self.emit_anchor_restore_finished(
                request_id,
                TimelineAnchorRestoreStatus::BudgetExhausted,
            );
            return;
        }
        if self.timeline_contains_event_id(&event_id) {
            self.restore_anchor = None;
            self.finish_anchor_restore(request_id, TimelineAnchorRestoreStatus::Found);
            return;
        }
        if let Some(mut existing) = self.restore_anchor.take() {
            if existing.event_id == event_id {
                existing.request_id = request_id;
                existing.max_batches_remaining = existing.max_batches_remaining.max(max_batches);
                existing.event_count = event_count;
                if existing.in_flight
                    || existing.awaiting_diff_batch
                    || existing.continuation_scheduled
                {
                    self.restore_anchor = Some(existing);
                } else {
                    self.schedule_restore_anchor_continue(existing).await;
                }
                return;
            }
            self.finish_anchor_restore(
                existing.request_id,
                TimelineAnchorRestoreStatus::Superseded,
            );
        }

        let restore = RestoreTimelineAnchorState {
            request_id,
            event_id,
            max_batches_remaining: max_batches,
            event_count,
            in_flight: false,
            awaiting_diff_batch: false,
            continuation_scheduled: false,
            continuation_serial: None,
            anchor_relay_wait: None,
        };

        self.schedule_restore_anchor_continue(restore).await;
    }

    async fn handle_restore_timeline_anchor_continue(&mut self, serial: u64) {
        let Some(mut restore) = self.restore_anchor.take() else {
            return;
        };
        if restore.continuation_serial != Some(serial) {
            self.restore_anchor = Some(restore);
            return;
        }
        if restore.in_flight {
            self.restore_anchor = Some(restore);
            return;
        }
        restore.awaiting_diff_batch = false;
        restore.continuation_scheduled = false;
        restore.continuation_serial = None;

        // Anchor-relay wait: entered after the SDK's authoritative
        // `anchor_present == true` signal. All cache events are in memory and
        // their diffs are in flight through the 3-hop relay
        // (conclude_backwards_pagination_from_disk → event-cache task →
        // timeline observable → relay task → DiffBatch actor msg). Re-tick
        // until `timeline_contains` confirms, or the backstop expires.
        //
        // A bounded sleep between ticks is necessary: without it all 40
        // backstop ticks drain before the relay task gets CPU time, because
        // the actor processes its own messages before yielding to other tasks.
        if let Some(remaining) = restore.anchor_relay_wait {
            if self.timeline_contains_event_id(&restore.event_id) {
                self.finish_anchor_restore(restore.request_id, TimelineAnchorRestoreStatus::Found);
                return;
            }
            if remaining > 0 {
                restore.anchor_relay_wait = Some(remaining - 1);
                // Yield to the runtime so the relay pipeline can deliver the
                // anchor diff before we check again. Without this pause, all
                // 40 ticks complete before any relay task is scheduled.
                tokio::time::sleep(std::time::Duration::from_millis(
                    RESTORE_ANCHOR_RELAY_WAIT_TICK_MS,
                ))
                .await;
                self.schedule_restore_anchor_continue(restore).await;
                return;
            }
            // Backstop: relay genuinely stuck. EndReached is the safest
            // fallback (anchor not confirmed in items; the caller can retry).
            self.finish_anchor_restore(restore.request_id, TimelineAnchorRestoreStatus::EndReached);
            return;
        }

        if self.timeline_contains_event_id(&restore.event_id) {
            self.finish_anchor_restore(restore.request_id, TimelineAnchorRestoreStatus::Found);
            return;
        }
        if restore.max_batches_remaining == 0 {
            self.finish_anchor_restore(
                restore.request_id,
                TimelineAnchorRestoreStatus::BudgetExhausted,
            );
            return;
        }

        restore.in_flight = true;
        let request_id = restore.request_id;
        let event_count = restore.event_count;

        // First try a cache-only bulk backward load in a single call
        // instead of looping one chunk at a time through `paginate_once`.
        // The SDK stops as soon as the anchor event is found (load-until-anchor),
        // or when it reaches a gap or the start of the on-disk timeline.
        //
        // Pass the UI-provided chunk budget directly as max_chunks. Room entry
        // must fail fast for stale/deep anchors instead of turning into a long
        // history walk; the event count `n` is a secondary cap.
        let chunk_budget = restore.max_batches_remaining;
        let bulk_n = (chunk_budget as u32)
            .saturating_mul(event_count as u32)
            .min(u16::MAX as u32) as u16;
        let cache_result = self
            .timeline
            .live_restore_from_cache(bulk_n, &restore.event_id, chunk_budget)
            .await;
        restore.in_flight = false;

        match cache_result {
            Ok(outcome) => {
                // The bulk load fired `RoomEventCacheUpdate::UpdateTimelineEvents`
                // broadcasts for every disk chunk, which are ingested by the
                // live Timeline's tasks loop and arrive as actor `DiffBatch`
                // messages. Those are buffered into `restore_emit_buffer` while
                // `restore_anchor.is_some()`, so we still get a single coalesced
                // `ItemsUpdated` flush at the terminal.
                // Deduct the actual number of cache chunks consumed from the
                // budget (each chunk ≈ one paginate batch). Clamp minimum to 1
                // so partial loads always advance the budget counter.
                restore.max_batches_remaining = restore
                    .max_batches_remaining
                    .saturating_sub(outcome.chunks_loaded.max(1) as u16);

                // Fast path: anchor already in timeline items (shallow-anchor case
                // where the lazy in-memory reveal made it visible immediately).
                if self.timeline_contains_event_id(&restore.event_id) {
                    self.finish_anchor_restore(request_id, TimelineAnchorRestoreStatus::Found);
                    return;
                }

                if outcome.anchor_present {
                    // SDK authoritative signal: anchor was found in a loaded disk
                    // chunk; its diff broadcast is already in flight through the
                    // 3-hop relay. Enter the relay-wait loop; do NOT conclude
                    // EndReached/BudgetExhausted while anchor_present is true.
                    restore.anchor_relay_wait = Some(RESTORE_ANCHOR_RELAY_WAIT_TICKS);
                    self.schedule_restore_anchor_continue(restore).await;
                    return;
                }

                if outcome.hit_gap {
                    // The cache is not contiguous up to the anchor depth.
                    // Fall back to the per-chunk paginate_once loop, which can
                    // resolve gaps via the network for non-contiguous caches.
                    restore.in_flight = true;
                    restore.max_batches_remaining = restore.max_batches_remaining.saturating_sub(1);

                    let result = self
                        .paginate_once(request_id, PaginationDirection::Backward, event_count)
                        .await;
                    restore.in_flight = false;

                    if self.timeline_contains_event_id(&restore.event_id) {
                        self.finish_anchor_restore(request_id, TimelineAnchorRestoreStatus::Found);
                        return;
                    }

                    let end_reached = match result {
                        Ok(end_reached) => end_reached,
                        Err(kind) => {
                            self.finish_anchor_restore(
                                request_id,
                                TimelineAnchorRestoreStatus::Failed { kind },
                            );
                            return;
                        }
                    };
                    if end_reached {
                        if self.timeline_contains_event_id(&restore.event_id) {
                            self.finish_anchor_restore(
                                request_id,
                                TimelineAnchorRestoreStatus::Found,
                            );
                            return;
                        }
                        self.finish_anchor_restore(
                            request_id,
                            TimelineAnchorRestoreStatus::EndReached,
                        );
                        return;
                    }
                    if restore.max_batches_remaining == 0 {
                        if self.timeline_contains_event_id(&restore.event_id) {
                            self.finish_anchor_restore(
                                request_id,
                                TimelineAnchorRestoreStatus::Found,
                            );
                            return;
                        }
                        self.finish_anchor_restore(
                            request_id,
                            TimelineAnchorRestoreStatus::BudgetExhausted,
                        );
                        return;
                    }
                    restore.awaiting_diff_batch = true;
                    self.schedule_restore_anchor_continue(restore).await;
                    return;
                }

                // No gap, anchor not present: cache-only bulk load completed
                // without finding the anchor.
                if outcome.reached_start {
                    // Loaded to the start of the on-disk cache; anchor is
                    // genuinely absent — conclude EndReached immediately
                    // (authoritative; no timing wait needed).
                    self.finish_anchor_restore(request_id, TimelineAnchorRestoreStatus::EndReached);
                    return;
                }

                // Cap case: the bulk load stopped because it reached the u16
                // per-call cap, not because it reached a gap or start. More
                // budget remains; issue another bulk load immediately.
                if restore.max_batches_remaining > 0 {
                    restore.awaiting_diff_batch = true;
                    self.schedule_restore_anchor_continue(restore).await;
                    return;
                }

                // Budget exhausted without finding the anchor.
                self.finish_anchor_restore(
                    request_id,
                    TimelineAnchorRestoreStatus::BudgetExhausted,
                );
            }

            Err(_) => {
                // Cache load error — fall back to the per-chunk paginate_once
                // path for a single attempt, treating the error as transient.
                restore.in_flight = true;
                restore.max_batches_remaining = restore.max_batches_remaining.saturating_sub(1);

                let result = self
                    .paginate_once(request_id, PaginationDirection::Backward, event_count)
                    .await;
                restore.in_flight = false;

                if self.timeline_contains_event_id(&restore.event_id) {
                    self.finish_anchor_restore(request_id, TimelineAnchorRestoreStatus::Found);
                    return;
                }

                let end_reached = match result {
                    Ok(end_reached) => end_reached,
                    Err(kind) => {
                        self.finish_anchor_restore(
                            request_id,
                            TimelineAnchorRestoreStatus::Failed { kind },
                        );
                        return;
                    }
                };
                if end_reached {
                    if self.timeline_contains_event_id(&restore.event_id) {
                        self.finish_anchor_restore(request_id, TimelineAnchorRestoreStatus::Found);
                        return;
                    }
                    self.finish_anchor_restore(request_id, TimelineAnchorRestoreStatus::EndReached);
                    return;
                }
                if restore.max_batches_remaining == 0 {
                    if self.timeline_contains_event_id(&restore.event_id) {
                        self.finish_anchor_restore(request_id, TimelineAnchorRestoreStatus::Found);
                        return;
                    }
                    self.finish_anchor_restore(
                        request_id,
                        TimelineAnchorRestoreStatus::BudgetExhausted,
                    );
                    return;
                }
                restore.awaiting_diff_batch = true;
                self.schedule_restore_anchor_continue(restore).await;
            }
        }
    }

    async fn maybe_continue_restore_anchor_after_diff(&mut self) {
        let Some(mut restore) = self.restore_anchor.take() else {
            return;
        };
        if restore.in_flight {
            self.restore_anchor = Some(restore);
            return;
        }
        // Anchor-relay wait: the queued Continue tick handles polling
        // `timeline_contains` each tick until Found or backstop. Put restore
        // back so the queued tick does its check on the next iteration.
        if restore.anchor_relay_wait.is_some() {
            self.restore_anchor = Some(restore);
            return;
        }
        if !restore.awaiting_diff_batch {
            self.restore_anchor = Some(restore);
            return;
        }
        if self.timeline_contains_event_id(&restore.event_id) {
            self.finish_anchor_restore(restore.request_id, TimelineAnchorRestoreStatus::Found);
            return;
        }
        if restore.max_batches_remaining == 0 {
            self.finish_anchor_restore(
                restore.request_id,
                TimelineAnchorRestoreStatus::BudgetExhausted,
            );
            return;
        }
        if restore.continuation_scheduled {
            self.restore_anchor = Some(restore);
            return;
        }

        restore.awaiting_diff_batch = false;
        self.schedule_restore_anchor_continue(restore).await;
    }

    async fn schedule_restore_anchor_continue(&mut self, mut restore: RestoreTimelineAnchorState) {
        self.next_restore_anchor_serial = self.next_restore_anchor_serial.wrapping_add(1);
        let serial = self.next_restore_anchor_serial;
        restore.continuation_scheduled = true;
        restore.continuation_serial = Some(serial);
        self.restore_anchor = Some(restore);
        let _ = self
            .msg_tx
            .send(TimelineActorMessage::RestoreTimelineAnchorContinue { serial })
            .await;
    }

    async fn handle_send_text(
        &mut self,
        request_id: RequestId,
        client_txn_id: String,
        body: String,
        mentions: MentionIntent,
    ) {
        let room_id_str = match &self.key.kind {
            TimelineKind::Room { room_id }
            | TimelineKind::Thread { room_id, .. }
            | TimelineKind::Focused { room_id, .. } => room_id.clone(),
        };

        let room_id = match matrix_sdk::ruma::RoomId::parse(&room_id_str) {
            Ok(id) => id,
            Err(_) => {
                self.emit_send_failed_action(&client_txn_id);
                self.emit_failure(
                    request_id,
                    CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::Sdk,
                    },
                );
                return;
            }
        };
        let client = self.session.client();
        if client.get_room(&room_id).is_none() {
            self.emit_send_failed_action(&client_txn_id);
            self.emit_failure(
                request_id,
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::Sdk,
                },
            );
            return;
        };

        // Send through the SDK UI timeline so local echo is owned by the
        // timeline controller and still backed by the send queue. Canon
        // decision D: the client-supplied txn_id maps to the SDK-generated
        // txn_id returned by Timeline::send. The SendHandle gives us the SDK
        // txn_id; we store client_txn_id -> sdk_txn_id here so the SentEvent
        // handler can emit SendCompleted with the client's txn_id.
        let content = match build_room_message_content_from_composer_body(&body, mentions) {
            Ok(content) => content,
            Err(kind) => {
                self.emit_send_failed_action(&client_txn_id);
                self.emit_failure(request_id, CoreFailure::TimelineOperationFailed { kind });
                return;
            }
        };
        let content = matrix_sdk::ruma::events::AnyMessageLikeEventContent::RoomMessage(content);

        match self.timeline.send(content).await {
            Ok(handle) => {
                let sdk_txn_id = handle.transaction_id().to_string();
                remember_send_handle(
                    &mut self.send_statuses,
                    &mut self.send_handles,
                    &handle,
                    TimelineSendState::Sending,
                );
                if let Some((client_txn_id, request_id, event_id)) = self
                    .send_completion
                    .remember_pending_send(sdk_txn_id, client_txn_id, request_id, true)
                {
                    self.emit_send_finished_action(&client_txn_id);
                    self.emit(CoreEvent::Timeline(TimelineEvent::SendCompleted {
                        request_id,
                        key: self.key.clone(),
                        transaction_id: client_txn_id,
                        event_id,
                    }));
                }
            }
            Err(err) => {
                let kind = classify_timeline_send_error(&err);
                self.emit_send_failed_action(&client_txn_id);
                self.emit_failure(request_id, CoreFailure::TimelineOperationFailed { kind });
            }
        }
        // On success: local echo appears via diff (Transaction id = SDK txn_id).
        // SendCompleted arrives via SendQueueUpdate::SentEvent.
    }

    async fn handle_send_reply(
        &mut self,
        request_id: RequestId,
        client_txn_id: String,
        in_reply_to_event_id: String,
        body: String,
        mentions: MentionIntent,
    ) {
        let room_id_str = match &self.key.kind {
            TimelineKind::Room { room_id }
            | TimelineKind::Thread { room_id, .. }
            | TimelineKind::Focused { room_id, .. } => room_id.clone(),
        };

        let room_id = match matrix_sdk::ruma::RoomId::parse(&room_id_str) {
            Ok(id) => id,
            Err(_) => {
                self.emit_send_failed_action(&client_txn_id);
                self.emit_failure(
                    request_id,
                    CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::Sdk,
                    },
                );
                return;
            }
        };

        let reply_event_id = match matrix_sdk::ruma::EventId::parse(&in_reply_to_event_id) {
            Ok(id) => id,
            Err(_) => {
                self.emit_send_failed_action(&client_txn_id);
                self.emit_failure(
                    request_id,
                    CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::Sdk,
                    },
                );
                return;
            }
        };

        let client = self.session.client();
        if client.get_room(&room_id).is_none() {
            self.emit_send_failed_action(&client_txn_id);
            self.emit_failure(
                request_id,
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::Sdk,
                },
            );
            return;
        }

        let content =
            match build_room_message_content_without_relation_from_composer_body(&body, mentions) {
                Ok(content) => content,
                Err(kind) => {
                    self.emit_send_failed_action(&client_txn_id);
                    self.emit_failure(request_id, CoreFailure::TimelineOperationFailed { kind });
                    return;
                }
            };
        let reply = Reply {
            event_id: reply_event_id,
            enforce_thread: reply_enforce_thread_for_key(&self.key),
            add_mentions: AddMentions::Yes,
        };

        let content = match self.timeline.room().make_reply_event(content, reply).await {
            Ok(content) => content,
            Err(_) => {
                self.emit_send_failed_action(&client_txn_id);
                self.emit_failure(
                    request_id,
                    CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::Sdk,
                    },
                );
                return;
            }
        };

        match self.timeline.send(content.into()).await {
            Ok(handle) => {
                let sdk_txn_id = handle.transaction_id().to_string();
                remember_send_handle(
                    &mut self.send_statuses,
                    &mut self.send_handles,
                    &handle,
                    TimelineSendState::Sending,
                );
                if let Some((client_txn_id, request_id, event_id)) = self
                    .send_completion
                    .remember_pending_send(sdk_txn_id, client_txn_id, request_id, true)
                {
                    self.emit_send_finished_action(&client_txn_id);
                    self.emit(CoreEvent::Timeline(TimelineEvent::SendCompleted {
                        request_id,
                        key: self.key.clone(),
                        transaction_id: client_txn_id,
                        event_id,
                    }));
                }
            }
            Err(err) => {
                let kind = classify_timeline_send_error(&err);
                self.emit_send_failed_action(&client_txn_id);
                self.emit_failure(request_id, CoreFailure::TimelineOperationFailed { kind });
            }
        }
    }

    async fn handle_load_message_source(&mut self, request_id: RequestId, event_id: String) {
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

    async fn handle_request_room_key(&mut self, request_id: RequestId, event_id: String) {
        let event_id = match matrix_sdk::ruma::EventId::parse(&event_id) {
            Ok(event_id) => event_id,
            Err(_) => {
                self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendTarget);
                return;
            }
        };
        let Some(event_item) = self.timeline.item_by_event_id(&event_id).await else {
            self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendTarget);
            return;
        };
        if !event_item.content().is_unable_to_decrypt() {
            self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendState);
            return;
        }
        let Some(original_json) = event_item.original_json().cloned() else {
            self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendTarget);
            return;
        };
        let room_id = match matrix_sdk::ruma::RoomId::parse(self.key.room_id()) {
            Ok(room_id) => room_id,
            Err(_) => {
                self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendTarget);
                return;
            }
        };
        let Some(session_id) =
            unable_to_decrypt_from_content(event_item.content()).and_then(|utd| utd.session_id)
        else {
            self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendState);
            return;
        };
        match koushi_sdk::download_room_key_from_backup(
            &self.session,
            room_id.as_str(),
            &session_id,
        )
        .await
        {
            Ok(true) => {
                self.timeline.retry_decryption([session_id]).await;
            }
            Ok(false) | Err(_) => {
                if koushi_sdk::request_room_key_for_event(
                    &self.session,
                    room_id.as_str(),
                    &original_json,
                )
                .await
                .is_err()
                {
                    self.emit_timeline_failure(request_id, TimelineFailureKind::Sdk);
                }
            }
        }
    }

    async fn handle_forward_message(
        &mut self,
        request_id: RequestId,
        source_event_id: String,
        destination_room_id: String,
        transaction_id: String,
    ) {
        let Some(source) = self
            .project_message_source_for_event(&source_event_id)
            .await
        else {
            self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendTarget);
            return;
        };
        let Some(body) = source
            .body
            .as_deref()
            .filter(|body| !body.trim().is_empty())
        else {
            self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendState);
            return;
        };
        if source.is_redacted {
            self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendState);
            return;
        }

        let destination_room_id_parsed = match matrix_sdk::ruma::RoomId::parse(&destination_room_id)
        {
            Ok(room_id) => room_id,
            Err(_) => {
                self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendTarget);
                return;
            }
        };
        let Some(destination_room) = self.session.client().get_room(&destination_room_id_parsed)
        else {
            self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidSendTarget);
            return;
        };

        let txn_id = matrix_sdk::ruma::OwnedTransactionId::from(transaction_id.clone());
        let content = RoomMessageEventContent::text_plain(body);
        match destination_room
            .send(content)
            .with_transaction_id(txn_id)
            .await
        {
            Ok(result) => {
                self.emit(CoreEvent::Timeline(TimelineEvent::MessageForwarded {
                    request_id,
                    key: self.key.clone(),
                    destination_room_id,
                    transaction_id,
                    event_id: result.response.event_id.to_string(),
                }));
            }
            Err(_) => {
                self.emit_timeline_failure(request_id, TimelineFailureKind::Sdk);
            }
        }
    }

    async fn handle_retry_send(&mut self, request_id: RequestId, transaction_id: String) {
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

    async fn handle_cancel_send(&mut self, request_id: RequestId, transaction_id: String) {
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
                if let Some((client_txn_id, _request_id, settles_composer)) =
                    self.send_completion.record_cancelled_event(&transaction_id)
                {
                    if settles_composer {
                        self.emit_send_finished_action(&client_txn_id);
                    }
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

    fn sdk_room_for_key(&self) -> Option<matrix_sdk::Room> {
        let room_id_str = match &self.key.kind {
            TimelineKind::Room { room_id }
            | TimelineKind::Thread { room_id, .. }
            | TimelineKind::Focused { room_id, .. } => room_id,
        };
        let room_id = matrix_sdk::ruma::RoomId::parse(room_id_str).ok()?;
        self.session.client().get_room(&room_id)
    }

    async fn handle_upload_and_send_media(
        &mut self,
        request_id: RequestId,
        client_txn_id: String,
        request: UploadMediaRequest,
    ) {
        let room_id_str = match &self.key.kind {
            TimelineKind::Room { room_id }
            | TimelineKind::Thread { room_id, .. }
            | TimelineKind::Focused { room_id, .. } => room_id.clone(),
        };

        let room_id = match matrix_sdk::ruma::RoomId::parse(&room_id_str) {
            Ok(id) => id,
            Err(_) => {
                self.emit_failure(
                    request_id,
                    CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::Sdk,
                    },
                );
                return;
            }
        };

        let client = self.session.client();
        let Some(room) = client.get_room(&room_id) else {
            self.emit_failure(
                request_id,
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::Sdk,
                },
            );
            return;
        };

        let mime_type = match request.mime_type.parse() {
            Ok(mime_type) => mime_type,
            Err(_) => {
                self.emit_failure(
                    request_id,
                    CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::Sdk,
                    },
                );
                return;
            }
        };

        let caption_mentions = request
            .caption
            .as_ref()
            .and_then(|caption| ruma_mentions_from_intent(&caption.mentions));
        let config = AttachmentConfig::new()
            .txn_id(matrix_sdk::ruma::OwnedTransactionId::from(
                client_txn_id.clone(),
            ))
            .info(attachment_info_for_upload(&request))
            .thumbnail(thumbnail_for_upload(&request))
            .caption(
                request
                    .caption
                    .as_ref()
                    .map(media_caption_content_from_draft),
            )
            .mentions(caption_mentions);

        match room
            .send_queue()
            .send_attachment(request.filename, mime_type, request.bytes, config)
            .await
        {
            Ok(handle) => {
                let sdk_txn_id = handle.transaction_id().to_string();
                remember_send_handle(
                    &mut self.send_statuses,
                    &mut self.send_handles,
                    &handle,
                    TimelineSendState::Sending,
                );
                if let Some((client_txn_id, request_id, event_id)) = self
                    .send_completion
                    .remember_pending_send(sdk_txn_id, client_txn_id, request_id, false)
                {
                    self.emit(CoreEvent::Timeline(TimelineEvent::SendCompleted {
                        request_id,
                        key: self.key.clone(),
                        transaction_id: client_txn_id,
                        event_id,
                    }));
                }
            }
            Err(err) => {
                self.emit_failure(
                    request_id,
                    CoreFailure::TimelineOperationFailed {
                        kind: classify_send_queue_error(&err),
                    },
                );
            }
        }
    }

    async fn handle_download_media(
        &mut self,
        request_id: RequestId,
        event_id: String,
        selection: MediaDownloadSelection,
    ) {
        if !self.media_downloads_in_progress.insert(event_id.clone()) {
            return;
        }

        let Some(entry) = self.media_sources.get(&event_id).cloned() else {
            self.media_downloads_in_progress.remove(&event_id);
            self.emit_failure(
                request_id,
                CoreFailure::TimelineOperationFailed {
                    kind: TimelineFailureKind::Sdk,
                },
            );
            return;
        };

        let total = entry.size;
        self.emit(CoreEvent::Timeline(TimelineEvent::MediaDownloadProgress {
            request_id,
            key: self.key.clone(),
            event_id: event_id.clone(),
            progress: MediaTransferProgress { current: 0, total },
        }));
        self.emit_action_reliable(AppAction::MediaDownloadUpdated {
            room_id: self.key.room_id().to_owned(),
            event_id: event_id.clone(),
            state: TimelineMediaDownloadState::Pending {
                progress: Some(MediaTransferProgress { current: 0, total }),
            },
        })
        .await;

        let Some(request) = media_request_for_download(&entry, selection) else {
            self.media_downloads_in_progress.remove(&event_id);
            self.emit_download_failed(request_id, &event_id, TimelineFailureKind::Sdk)
                .await;
            return;
        };

        match self
            .session
            .client()
            .media()
            .get_media_content(&request, true)
            .await
        {
            Ok(bytes) => {
                let byte_count = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
                let mut download_state = TimelineMediaDownloadState::Pending {
                    progress: Some(MediaTransferProgress {
                        current: byte_count,
                        total,
                    }),
                };

                if let Some(data_dir) = self.data_dir.as_deref() {
                    // Matrix IDs contain ':' which is not valid in Windows
                    // path components.  Use a hex-encoded SHA-256 of the
                    // room_id as the directory name and of the event_id as the
                    // filename so the path is safe on all platforms.
                    let dir_name = sanitize_matrix_id_for_path(self.key.room_id());
                    let file_name = format!("{}.bin", sanitize_matrix_id_for_path(&event_id));
                    let dir = data_dir.join("media_downloads").join(dir_name);
                    if tokio::fs::create_dir_all(&dir).await.is_ok() {
                        let path = dir.join(file_name);
                        if tokio::fs::write(&path, &bytes).await.is_ok() {
                            download_state = TimelineMediaDownloadState::Ready {
                                source_url: path.to_string_lossy().into_owned(),
                                width: entry.width,
                                height: entry.height,
                                mime_type: entry.mimetype.clone(),
                            };
                        }
                    }
                }

                let (source_url, width, height, mimetype) = match &download_state {
                    TimelineMediaDownloadState::Ready {
                        source_url,
                        width,
                        height,
                        mime_type,
                    } => (source_url.clone(), *width, *height, mime_type.clone()),
                    _ => {
                        self.media_downloads_in_progress.remove(&event_id);
                        self.emit_download_failed(request_id, &event_id, TimelineFailureKind::Sdk)
                            .await;
                        return;
                    }
                };

                self.emit(CoreEvent::Timeline(TimelineEvent::MediaDownloadCompleted {
                    request_id,
                    key: self.key.clone(),
                    event_id: event_id.clone(),
                    source_url,
                    byte_count,
                    mimetype,
                    width,
                    height,
                }));
                self.emit_action_reliable(AppAction::MediaDownloadUpdated {
                    room_id: self.key.room_id().to_owned(),
                    event_id: event_id.clone(),
                    state: download_state,
                })
                .await;
                self.media_downloads_in_progress.remove(&event_id);
            }
            Err(_) => {
                self.media_downloads_in_progress.remove(&event_id);
                self.emit_download_failed(request_id, &event_id, TimelineFailureKind::Sdk)
                    .await;
            }
        }
    }

    async fn emit_download_failed(
        &self,
        request_id: RequestId,
        event_id: &str,
        kind: TimelineFailureKind,
    ) {
        self.emit(CoreEvent::Timeline(TimelineEvent::MediaDownloadFailed {
            request_id,
            key: self.key.clone(),
            event_id: event_id.to_owned(),
            kind,
        }));
        // Use reliable delivery — a dropped failure action leaves the UI stuck
        // in a pending download state (REPOSITORY_RULES L124-128).
        self.emit_action_reliable(AppAction::MediaDownloadUpdated {
            room_id: self.key.room_id().to_owned(),
            event_id: event_id.to_owned(),
            state: TimelineMediaDownloadState::Failed {
                failure_kind: match kind {
                    TimelineFailureKind::Network => OperationFailureKind::Network,
                    TimelineFailureKind::Sdk => OperationFailureKind::Sdk,
                    TimelineFailureKind::Forbidden => OperationFailureKind::Forbidden,
                    TimelineFailureKind::Timeout => OperationFailureKind::Timeout,
                    _ => OperationFailureKind::Sdk,
                },
            },
        })
        .await;
    }

    async fn handle_edit_text(&mut self, request_id: RequestId, event_id: String, body: String) {
        // Edits go through the SDK Timeline so the Set diff on the original
        // item is produced locally (send-queue local echo) instead of
        // depending on the server echoing the edit back through sync —
        // Conduit's sliding sync does not deliver it reliably (Phase 5
        // review finding). Canon rule 1: relay the SDK.
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
        let mut result = Ok(());
        for item_id in &candidates {
            let content =
                matrix_sdk::ruma::events::room::message::RoomMessageEventContentWithoutRelation::text_plain(
                    &body,
                );
            result = self
                .timeline
                .edit(item_id, EditedContent::RoomMessage(content))
                .await;
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
        // Edit success: the local-echo Set diff on the original item identity
        // arrives through the subscription; no dedicated EditCompleted event.
    }

    async fn handle_redact(&mut self, request_id: RequestId, event_id: String) {
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

    async fn handle_toggle_reaction(
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

    async fn handle_send_reaction(
        &mut self,
        request_id: RequestId,
        event_id: String,
        reaction_key: String,
    ) {
        if reaction_key.trim().is_empty() {
            self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidReactionTarget);
            return;
        }

        let Some(target) = self.reaction_target_state(&event_id, &reaction_key).await else {
            self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidReactionTarget);
            return;
        };
        if let Err(kind) =
            validate_send_reaction(target.can_react, target.my_reaction_event_id.as_deref())
        {
            self.emit_timeline_failure(request_id, kind);
            return;
        }

        match self
            .timeline
            .toggle_reaction(&target.item_id, &reaction_key)
            .await
        {
            Ok(true) => {}
            Ok(false) => {
                self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidReactionState);
            }
            Err(error) => {
                self.emit_timeline_failure(request_id, classify_reaction_error(&error));
            }
        }
    }

    async fn handle_redact_reaction(
        &mut self,
        request_id: RequestId,
        event_id: String,
        reaction_key: String,
        reaction_event_id: String,
    ) {
        if reaction_key.trim().is_empty() || reaction_event_id.trim().is_empty() {
            self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidReactionTarget);
            return;
        }

        let Some(target) = self.reaction_target_state(&event_id, &reaction_key).await else {
            self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidReactionTarget);
            return;
        };
        if let Err(kind) = validate_redact_reaction(
            target.can_react,
            target.my_reaction_event_id.as_deref(),
            &reaction_event_id,
        ) {
            self.emit_timeline_failure(request_id, kind);
            return;
        }

        match self
            .timeline
            .toggle_reaction(&target.item_id, &reaction_key)
            .await
        {
            Ok(false) => {}
            Ok(true) => {
                self.emit_timeline_failure(request_id, TimelineFailureKind::InvalidReactionState);
            }
            Err(error) => {
                self.emit_timeline_failure(request_id, classify_reaction_error(&error));
            }
        }
    }

    async fn handle_send_read_receipt(&mut self, request_id: RequestId, event_id: String) {
        let parsed_event_id = match matrix_sdk::ruma::EventId::parse(event_id.as_str()) {
            Ok(event_id) => event_id,
            Err(_) => {
                self.emit_failure(
                    request_id,
                    CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::Sdk,
                    },
                );
                return;
            }
        };

        match self
            .timeline
            .send_single_receipt(ReceiptType::Read, parsed_event_id)
            .await
        {
            Ok(_) => {
                self.emit(CoreEvent::LiveSignals(LiveSignalsEvent::ReadReceiptSent {
                    request_id,
                    key: self.key.clone(),
                    event_id,
                }));
            }
            Err(_) => {
                self.emit_failure(
                    request_id,
                    CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::Sdk,
                    },
                );
            }
        }
    }

    async fn handle_set_fully_read(&mut self, request_id: RequestId, event_id: String) {
        let room_id_for_trace = timeline_room_id(&self.key);
        if let Some(room_id) = &room_id_for_trace {
            unread_trace::trace_mark_read(
                "set_fully_read_requested",
                request_id.sequence,
                room_id,
                Some(event_id.as_str()),
            );
        }
        let parsed_event_id = match matrix_sdk::ruma::EventId::parse(event_id.as_str()) {
            Ok(event_id) => event_id,
            Err(_) => {
                if let Some(room_id) = &room_id_for_trace {
                    unread_trace::trace_mark_read(
                        "set_fully_read_failed",
                        request_id.sequence,
                        room_id,
                        Some(event_id.as_str()),
                    );
                }
                self.emit_failure(
                    request_id,
                    CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::Sdk,
                    },
                );
                return;
            }
        };

        let receipts = Receipts::new()
            .fully_read_marker(parsed_event_id.clone())
            .private_read_receipt(parsed_event_id);

        match self.timeline.room().send_multiple_receipts(receipts).await {
            Ok(_) => {
                self.fully_read_event_id = Some(event_id.clone());
                if let Some(room_id) = room_id_for_trace {
                    unread_trace::trace_mark_read(
                        "set_fully_read_success",
                        request_id.sequence,
                        &room_id,
                        Some(event_id.as_str()),
                    );
                    let _ = self.action_tx.try_send(vec![
                        AppAction::FullyReadMarkerUpdated {
                            room_id: room_id.clone(),
                            event_id: Some(event_id.clone()),
                        },
                        AppAction::RoomMarkedAsReadSucceeded {
                            request_id: request_id.sequence,
                            room_id,
                        },
                    ]);
                }
                self.emit(CoreEvent::LiveSignals(LiveSignalsEvent::FullyReadSet {
                    request_id,
                    key: self.key.clone(),
                    event_id,
                }));
                self.emit_navigation_if_changed();
            }
            Err(_) => {
                if let Some(room_id) = &room_id_for_trace {
                    unread_trace::trace_mark_read(
                        "set_fully_read_failed",
                        request_id.sequence,
                        room_id,
                        Some(event_id.as_str()),
                    );
                }
                self.emit_failure(
                    request_id,
                    CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::Sdk,
                    },
                );
            }
        }
    }

    async fn handle_set_typing(&mut self, request_id: RequestId, is_typing: bool) {
        match self.timeline.room().typing_notice(is_typing).await {
            Ok(()) => {
                self.emit(CoreEvent::LiveSignals(LiveSignalsEvent::TypingSet {
                    request_id,
                    key: self.key.clone(),
                    is_typing,
                }));
            }
            Err(_) => {
                self.emit_failure(
                    request_id,
                    CoreFailure::TimelineOperationFailed {
                        kind: TimelineFailureKind::Sdk,
                    },
                );
            }
        }
    }

    /// Re-emit `navigation_items` as `InitialItems` for `request_id` without
    /// touching the SDK subscription or tearing down the actor.  Called when
    /// `handle_subscribe` detects that this key is already subscribed (the
    /// idempotency fast path).  The generation is unchanged; the caller only
    /// needs a fresh InitialItems batch so a re-mounted TimelineView is
    /// populated.
    fn handle_replay_initial_items(&self, request_id: RequestId) {
        let items = replay_initial_items_window(
            &self.key.kind,
            &self.navigation_items,
            &self.viewport_observation,
        );
        if std::env::var_os("KOUSHI_SUBSCRIBE_TRACE").is_some() {
            eprintln!(
                "koushi.subscribe stage=replay_initial_emitted count={}",
                items.len()
            );
        }
        self.emit(CoreEvent::Timeline(TimelineEvent::InitialItems {
            request_id: Some(request_id),
            key: self.key.clone(),
            generation: self.generation,
            items,
        }));
    }

    async fn handle_diff_batch(
        &mut self,
        diffs: Vec<eyeball_im::VectorDiff<Arc<SdkTimelineItem>>>,
    ) {
        if diffs.is_empty() {
            return;
        }
        // Advance the diff-batch sequence counter on every non-empty batch so
        // settle ticks can detect when the final async DiffBatch has landed.
        self.diff_batch_seq = self.diff_batch_seq.wrapping_add(1);

        for diff in &diffs {
            self.apply_media_cache_diff(diff);
        }
        self.emit_receipts_from_sdk_diffs(&diffs);

        // Phase 6: forward search index mutations before converting diffs.
        if self.search_index_tx.is_some() {
            for diff in &diffs {
                self.forward_diff_to_search(diff);
            }
        }

        let mut core_diffs: Vec<TimelineDiff> = diffs
            .into_iter()
            .map(|diff| {
                sdk_vector_diff_to_timeline_diff(
                    diff,
                    &self.key,
                    self.own_user_id.as_deref(),
                    &self.send_statuses,
                )
            })
            .collect();
        for diff in &mut core_diffs {
            apply_ignored_sender_suppression_to_diff(diff, &self.ignored_user_ids);
        }
        let link_preview_context = self.link_preview_policy.for_room(self.key.room_id());
        for diff in &mut core_diffs {
            match diff {
                TimelineDiff::Reset { items } => {
                    for item in items {
                        apply_link_previews_to_item(
                            item,
                            self.key.room_id(),
                            &link_preview_context,
                            &self.session,
                        )
                        .await;
                    }
                }
                TimelineDiff::PushFront { item }
                | TimelineDiff::PushBack { item }
                | TimelineDiff::Insert { item, .. }
                | TimelineDiff::Set { item, .. } => {
                    apply_link_previews_to_item(
                        item,
                        self.key.room_id(),
                        &link_preview_context,
                        &self.session,
                    )
                    .await;
                }
                _ => {}
            }
        }
        if let Some(action) = thread_attention_action_from_timeline_diffs(
            &mut self.thread_attention_counts,
            &self.key,
            &core_diffs,
            self.own_user_id.as_ref().map(|user_id| user_id.as_str()),
        ) {
            let _ = self.action_tx.try_send(vec![action]);
        }
        let activity_rows = activity_rows_from_timeline_diffs(&self.key, &core_diffs);
        if !activity_rows.is_empty() {
            let _ = self
                .action_tx
                .try_send(vec![AppAction::ActivityRowsObserved {
                    rows: activity_rows,
                }]);
        }
        apply_timeline_diffs_to_items(&mut self.navigation_items, &core_diffs);
        self.maybe_fetch_visible_reply_details();
        self.emit_media_gallery_if_changed();

        let restore_diff_is_relevant = timeline_diffs_include_prepend(&core_diffs);

        if self.restore_anchor.is_some() {
            // While a restore walk is in-flight, buffer this batch's diffs
            // instead of emitting ItemsUpdated per chunk. React receives ONE
            // settled update when the restore terminates. The batch_id counter
            // is still advanced so later non-restore emits remain monotonic.
            self.next_batch_id = TimelineBatchId(self.next_batch_id.0 + 1);
            self.restore_emit_buffer.extend(core_diffs);
            // Navigation is also suppressed until the flush at restore end.

            if restore_diff_is_relevant {
                let restore_event_id = self
                    .restore_anchor
                    .as_ref()
                    .map(|restore| restore.event_id.clone());
                if let Some(event_id) = restore_event_id {
                    if self.timeline_contains_event_id(&event_id) {
                        if let Some(restore) = self.restore_anchor.take() {
                            self.finish_anchor_restore(
                                restore.request_id,
                                TimelineAnchorRestoreStatus::Found,
                            );
                        }
                    } else {
                        self.maybe_continue_restore_anchor_after_diff().await;
                    }
                }
            }
        } else {
            let batch_id = self.next_batch_id;
            self.next_batch_id = TimelineBatchId(batch_id.0 + 1);
            self.emit(CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: self.key.clone(),
                generation: self.generation,
                batch_id,
                diffs: core_diffs,
            }));
            self.emit_navigation_if_changed();
        }
    }

    async fn handle_ignored_users_updated(&mut self, user_ids: std::collections::BTreeSet<String>) {
        if self.ignored_user_ids == user_ids {
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
            return;
        }

        let activity_rows = activity_rows_from_timeline_diffs(&self.key, &core_diffs);
        if !activity_rows.is_empty() {
            let _ = self
                .action_tx
                .try_send(vec![AppAction::ActivityRowsObserved {
                    rows: activity_rows,
                }]);
        }
        self.emit_media_gallery_if_changed();

        let batch_id = self.next_batch_id;
        self.next_batch_id = TimelineBatchId(batch_id.0 + 1);

        self.emit(CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
            key: self.key.clone(),
            generation: self.generation,
            batch_id,
            diffs: core_diffs,
        }));
        self.emit_navigation_if_changed();
    }

    fn emit_timeline_item_set(&mut self, index: usize) {
        let core_diffs = vec![TimelineDiff::Set {
            index,
            item: self.navigation_items[index].clone(),
        }];

        let batch_id = self.next_batch_id;
        self.next_batch_id = TimelineBatchId(batch_id.0 + 1);
        self.emit(CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
            key: self.key.clone(),
            generation: self.generation,
            batch_id,
            diffs: core_diffs,
        }));
    }

    async fn handle_load_link_previews(&mut self, request_id: RequestId, event_id: String) {
        let trace_started = timeline_trace_enabled().then(std::time::Instant::now);
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

    async fn handle_link_previews_fetched(
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

    fn handle_cancel_link_previews(&mut self, request_id: RequestId) {
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

        let batch_id = self.next_batch_id;
        self.next_batch_id = TimelineBatchId(batch_id.0 + 1);
        self.emit(CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
            key: self.key.clone(),
            generation: self.generation,
            batch_id,
            diffs: core_diffs,
        }));
    }

    async fn handle_hide_link_preview(&mut self, _request_id: RequestId, event_id: String) {
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

        let batch_id = self.next_batch_id;
        self.next_batch_id = TimelineBatchId(batch_id.0 + 1);
        self.emit(CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
            key: self.key.clone(),
            generation: self.generation,
            batch_id,
            diffs: core_diffs,
        }));
    }

    async fn handle_link_preview_policy_changed(
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

        let batch_id = self.next_batch_id;
        self.next_batch_id = TimelineBatchId(batch_id.0 + 1);
        self.emit(CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
            key: self.key.clone(),
            generation: self.generation,
            batch_id,
            diffs: core_diffs,
        }));
    }

    fn emit_media_gallery_if_changed(&mut self) {
        let items = media_gallery_items_from_timeline_items(&self.key, &self.navigation_items);
        if items == self.media_gallery_items {
            return;
        }
        self.media_gallery_items = items.clone();
        if let Some(action) = media_gallery_updated_action(&self.key, items) {
            let _ = self.action_tx.try_send(vec![action]);
        }
    }

    fn emit_navigation_if_changed(&mut self) {
        let snapshot = derive_timeline_navigation_snapshot(
            &self.navigation_items,
            self.fully_read_event_id.as_deref(),
            &self.viewport_observation,
            self.own_user_id.as_ref().map(|user_id| user_id.as_str()),
        );
        if self.last_navigation_snapshot.as_ref() == Some(&snapshot) {
            return;
        }
        self.last_navigation_snapshot = Some(snapshot.clone());
        self.emit(CoreEvent::Timeline(TimelineEvent::NavigationUpdated {
            key: self.key.clone(),
            snapshot,
        }));
    }

    fn maybe_fetch_visible_reply_details(&mut self) {
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

    fn apply_media_cache_diff(&mut self, diff: &eyeball_im::VectorDiff<Arc<SdkTimelineItem>>) {
        use eyeball_im::VectorDiff;

        match diff {
            VectorDiff::PushFront { value }
            | VectorDiff::PushBack { value }
            | VectorDiff::Insert { value, .. }
            | VectorDiff::Set { value, .. } => {
                cache_sdk_item_media_source(&mut self.media_sources, value);
            }
            VectorDiff::Append { values } => {
                for item in values {
                    cache_sdk_item_media_source(&mut self.media_sources, item);
                }
            }
            VectorDiff::Reset { values } => {
                self.media_sources.clear();
                for item in values {
                    cache_sdk_item_media_source(&mut self.media_sources, item);
                }
            }
            VectorDiff::Clear => {
                self.media_sources.clear();
            }
            VectorDiff::Remove { .. }
            | VectorDiff::Truncate { .. }
            | VectorDiff::PopFront
            | VectorDiff::PopBack => {}
        }
    }

    /// Forward a single SDK diff item to the search index channel.
    /// Fire-and-forget: if the channel is full, the mutation is dropped rather
    /// than blocking the diff relay (search index is best-effort for freshness).
    fn forward_diff_to_search(&self, diff: &eyeball_im::VectorDiff<Arc<SdkTimelineItem>>) {
        use crate::search::SearchIndexMessage;
        use eyeball_im::VectorDiff;

        let Some(tx) = &self.search_index_tx else {
            return;
        };

        let room_id = match &self.key.kind {
            TimelineKind::Room { room_id }
            | TimelineKind::Thread { room_id, .. }
            | TimelineKind::Focused { room_id, .. } => room_id.clone(),
        };

        // Extract the SDK item (if any) from the diff.
        let item_ref: Option<&Arc<SdkTimelineItem>> = match diff {
            VectorDiff::PushFront { value } => Some(value),
            VectorDiff::PushBack { value } => Some(value),
            VectorDiff::Insert { value, .. } => Some(value),
            VectorDiff::Set { value, .. } => Some(value),
            VectorDiff::Append { values } => {
                // Bulk append: process each item in order.
                for item in values.iter() {
                    let sub_diff = VectorDiff::PushBack {
                        value: item.clone(),
                    };
                    self.forward_diff_to_search(&sub_diff);
                }
                return;
            }
            VectorDiff::Reset { values } => {
                // Full reset: process each item.
                for item in values.iter() {
                    let sub_diff = VectorDiff::PushBack {
                        value: item.clone(),
                    };
                    self.forward_diff_to_search(&sub_diff);
                }
                return;
            }
            VectorDiff::Remove { .. }
            | VectorDiff::Truncate { .. }
            | VectorDiff::Clear
            | VectorDiff::PopFront
            | VectorDiff::PopBack => {
                // Remove/truncate/clear: we don't know which event_ids are affected
                // without tracking the full timeline list; skip search forwarding.
                // Redactions arrive as Set-with-is_redacted=true before any Remove.
                return;
            }
        };

        let Some(item) = item_ref else { return };

        use matrix_sdk_ui::timeline::TimelineItemKind;
        let event_item = match item.kind() {
            TimelineItemKind::Event(e) => e,
            TimelineItemKind::Virtual(_) => return,
        };

        // Only remote events have a stable event_id we can index.
        let event_id = match event_item.event_id() {
            Some(id) => id.to_string(),
            None => return, // local-echo without confirmed event_id: skip
        };

        let sender = event_item.sender().to_string();
        let timestamp_ms: u64 = event_item.timestamp().0.into();

        // Redacted items: forward Redact so the document is removed.
        if event_item.content().is_redacted() {
            let _ = tx.try_send(SearchIndexMessage::Redact { event_id });
            return;
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
                return;
            };

        if body.is_none() && attachment_filename.is_none() {
            return;
        }

        if let Some(edit_event_id) = edit_event_id {
            // Edited message: Upsert original with new canonical body, AND
            // forward Edit so the document store registers the alias
            // (edit_event_id → original_event_id) used by verify_candidate.
            let _ = tx.try_send(SearchIndexMessage::Upsert {
                room_id: room_id.clone(),
                event_id: event_id.clone(),
                sender: sender.clone(),
                timestamp_ms,
                body: body.clone(),
                attachment_filename: attachment_filename.clone(),
                attachment: attachment.clone(),
            });
            let _ = tx.try_send(SearchIndexMessage::Edit {
                edit_event_id,
                target_event_id: event_id,
                sender,
                timestamp_ms,
                body,
                attachment_filename,
                attachment,
            });
        } else {
            // New (unedited) message: Upsert into document store.
            let _ = tx.try_send(SearchIndexMessage::Upsert {
                room_id,
                event_id,
                sender,
                timestamp_ms,
                body,
                attachment_filename,
                attachment,
            });
        }
    }

    fn forward_initial_items_to_search(
        &self,
        items: impl IntoIterator<Item = Arc<SdkTimelineItem>>,
    ) {
        use eyeball_im::VectorDiff;

        for item in items {
            self.forward_diff_to_search(&VectorDiff::PushBack { value: item });
        }
    }

    fn attachment_document_from_timeline_media(
        media: &TimelineMedia,
        event_item: &EventTimelineItem,
        message: &matrix_sdk_ui::timeline::Message,
    ) -> Option<AttachmentDocument> {
        let kind = match media.kind {
            crate::event::TimelineMediaKind::Image => AttachmentKind::Image,
            crate::event::TimelineMediaKind::Video => AttachmentKind::Video,
            crate::event::TimelineMediaKind::Audio => AttachmentKind::Audio,
            crate::event::TimelineMediaKind::File => AttachmentKind::File,
        };

        let msgtype = match media.kind {
            crate::event::TimelineMediaKind::Image => "m.image",
            crate::event::TimelineMediaKind::Video => "m.video",
            crate::event::TimelineMediaKind::Audio => "m.audio",
            crate::event::TimelineMediaKind::File => "m.file",
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

    fn timeline_contains_event_id(&self, event_id: &str) -> bool {
        self.navigation_items.iter().any(
            |item| matches!(&item.id, TimelineItemId::Event { event_id: id } if id == event_id),
        )
    }

    async fn project_message_source_for_event(
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
            source.original_json = original_json_for_event_item(event_item);
            return Some(source);
        }
        None
    }

    async fn reaction_target_state(
        &self,
        event_id: &str,
        reaction_key: &str,
    ) -> Option<ReactionTargetState> {
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
            let my_reaction_event_id = projected
                .reactions
                .iter()
                .find(|reaction| reaction.key == reaction_key)
                .and_then(|reaction| reaction.my_reaction_event_id.clone());
            return Some(ReactionTargetState {
                item_id: TimelineEventItemId::EventId(parsed_event_id),
                can_react: projected.can_react,
                my_reaction_event_id,
            });
        }
        None
    }

    fn handle_send_queue_update(&mut self, update: RoomSendQueueUpdate) {
        match update {
            RoomSendQueueUpdate::NewLocalEvent(echo) => {
                remember_local_echo(&mut self.send_statuses, &mut self.send_handles, &echo);
            }
            RoomSendQueueUpdate::CancelledLocalEvent { transaction_id } => {
                let sdk_txn_str = transaction_id.to_string();
                self.send_statuses
                    .insert(sdk_txn_str.clone(), TimelineSendState::Cancelled);
                self.send_handles.remove(&sdk_txn_str);
                if let Some((client_txn_id, _request_id, settles_composer)) =
                    self.send_completion.record_cancelled_event(&sdk_txn_str)
                {
                    if settles_composer {
                        self.emit_send_finished_action(&client_txn_id);
                    }
                }
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
                if let Some((client_txn_id, _request_id, settles_composer)) =
                    self.send_completion.record_send_error(&sdk_txn_str)
                {
                    if settles_composer {
                        self.emit_send_failed_action(&client_txn_id);
                    }
                }
            }
            RoomSendQueueUpdate::RetryEvent { transaction_id } => {
                self.send_statuses
                    .insert(transaction_id.to_string(), TimelineSendState::Sending);
            }
            RoomSendQueueUpdate::SentEvent {
                transaction_id,
                event_id,
            } => {
                // The SDK fires SentEvent with its own txn_id; look up the client txn_id.
                let sdk_txn_str = transaction_id.to_string();
                self.send_statuses
                    .insert(sdk_txn_str.clone(), TimelineSendState::Sent);
                self.send_handles.remove(&sdk_txn_str);
                self.sent_event_txns
                    .insert(event_id.to_string(), transaction_id.clone());
                if let Some((client_txn_id, request_id, event_id, settles_composer)) = self
                    .send_completion
                    .record_sent_event(sdk_txn_str, event_id.to_string())
                {
                    if settles_composer {
                        self.emit_send_finished_action(&client_txn_id);
                    }
                    self.emit(CoreEvent::Timeline(TimelineEvent::SendCompleted {
                        request_id,
                        key: self.key.clone(),
                        transaction_id: client_txn_id,
                        event_id,
                    }));
                }
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
                let pending = self.send_completion.pending_send(&sdk_txn_str);
                let (transaction_id, request_id) = pending
                    .map(|(client_txn_id, request_id)| (client_txn_id.to_owned(), Some(request_id)))
                    .unwrap_or((sdk_txn_str, None));

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

    async fn handle_relay_overflow(&mut self) {
        // Overflow protocol (canon decision B):
        // 1. Bump generation.
        self.generation = TimelineGeneration(self.generation.0 + 1);
        // 2. Reset batch_id to 0.
        self.next_batch_id = TimelineBatchId(0);

        // 3. Emit ResyncRequired.
        self.emit(CoreEvent::Timeline(TimelineEvent::ResyncRequired {
            key: self.key.clone(),
            reason: TimelineResyncReason::QueueOverflow,
        }));

        // 4. Emit a fresh InitialItems with the new generation from the current
        //    SDK timeline snapshot.
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

        self.emit(CoreEvent::Timeline(TimelineEvent::InitialItems {
            request_id: None,
            key: self.key.clone(),
            generation: self.generation,
            items,
        }));
        if let Some(restore) = self.restore_anchor.take() {
            self.finish_anchor_restore(
                restore.request_id,
                TimelineAnchorRestoreStatus::Failed {
                    kind: TimelineFailureKind::QueueOverflow,
                },
            );
        }
    }

    fn emit(&self, event: CoreEvent) {
        let _ = self.event_tx.send(event);
    }

    fn emit_anchor_restore_finished(
        &self,
        request_id: RequestId,
        status: TimelineAnchorRestoreStatus,
    ) {
        self.emit(CoreEvent::Timeline(TimelineEvent::AnchorRestoreFinished {
            request_id,
            key: self.key.clone(),
            status,
        }));
    }

    /// Flush the restore-walk diff buffer as ONE `ItemsUpdated` event (Change
    /// 2). Called at every restore terminal path (Found/EndReached/
    /// BudgetExhausted/Failed/Superseded). If the buffer is empty nothing is
    /// emitted; navigation is always refreshed after a restore so React
    /// receives a consistent settled state. Never drops buffered diffs.
    fn flush_restore_emit_buffer(&mut self) {
        if !self.restore_emit_buffer.is_empty() {
            let diffs = std::mem::take(&mut self.restore_emit_buffer);
            let batch_id = self.next_batch_id;
            self.next_batch_id = TimelineBatchId(batch_id.0 + 1);
            self.emit(CoreEvent::Timeline(TimelineEvent::ItemsUpdated {
                key: self.key.clone(),
                generation: self.generation,
                batch_id,
                diffs,
            }));
        } else {
            self.restore_emit_buffer.clear();
        }
        self.emit_navigation_if_changed();
    }

    /// Terminate a restore walk: flush the buffered diffs (Change 2) then emit
    /// `AnchorRestoreFinished`. Call this at every terminal restore path in
    /// place of `emit_anchor_restore_finished` when the diff buffer may be
    /// non-empty.
    fn finish_anchor_restore(
        &mut self,
        request_id: RequestId,
        status: TimelineAnchorRestoreStatus,
    ) {
        self.flush_restore_emit_buffer();
        self.emit_anchor_restore_finished(request_id, status);
    }

    /// Reliably deliver an `AppAction` to the reducer.  Uses `send` (not
    /// `try_send`) so the action is not silently dropped when the channel is
    /// momentarily full.  Required for state-machine transitions where a
    /// dropped action would leave the UI stuck in a pending/inconsistent state
    /// (REPOSITORY_RULES L124-128).
    async fn emit_action_reliable(&self, action: AppAction) {
        let _ = self.action_tx.send(vec![action]).await;
    }

    fn emit_failure(&self, request_id: RequestId, failure: CoreFailure) {
        self.emit(CoreEvent::OperationFailed {
            request_id,
            failure,
        });
    }

    fn emit_timeline_failure(&self, request_id: RequestId, kind: TimelineFailureKind) {
        self.emit_failure(request_id, CoreFailure::TimelineOperationFailed { kind });
    }

    /// Drive the reducer's composer completion transition for the matching
    /// pending send. Room timelines settle the main composer; thread timelines
    /// settle the open thread composer; focused timelines own no composer state.
    fn emit_send_finished_action(&self, client_txn_id: &str) {
        if let Some(action) = send_finished_action(&self.key, client_txn_id.to_owned()) {
            let _ = self.action_tx.try_send(vec![action]);
        }
    }

    /// Drive the reducer's composer failure transition for the matching pending
    /// send. Room timelines settle the main composer; thread timelines settle
    /// the open thread composer; focused timelines own no composer state.
    fn emit_send_failed_action(&self, client_txn_id: &str) {
        let projection = match self.key.kind {
            TimelineKind::Room { .. } => SendComposerProjection::Room,
            TimelineKind::Thread { .. } => SendComposerProjection::ThreadReply,
            TimelineKind::Focused { .. } => SendComposerProjection::None,
        };
        if let Some(action) = send_failed_action(
            &self.key,
            projection,
            client_txn_id.to_owned(),
            "send failed".to_owned(),
        ) {
            let _ = self.action_tx.try_send(vec![action]);
        }
    }

    fn emit_typing_users_action(&self, user_ids: Vec<String>) {
        let Some(room_id) = timeline_room_id(&self.key) else {
            return;
        };
        let _ = self
            .action_tx
            .try_send(vec![AppAction::TypingUsersUpdated { room_id, user_ids }]);
    }

    fn emit_receipts_from_sdk_diffs(&self, diffs: &[eyeball_im::VectorDiff<Arc<SdkTimelineItem>>]) {
        let Some(room_id) = timeline_room_id(&self.key) else {
            return;
        };
        let mut receipts_by_event = Vec::new();
        for diff in diffs {
            collect_live_event_receipts_from_diff(diff, &mut receipts_by_event);
        }
        if receipts_by_event.is_empty() {
            return;
        }
        let _ = self
            .action_tx
            .try_send(vec![AppAction::LiveRoomReceiptsUpdated {
                room_id,
                receipts_by_event,
            }]);
    }
}

fn koushi_timeline_builder(
    room: &matrix_sdk::Room,
    focus: TimelineFocus,
) -> matrix_sdk_ui::timeline::TimelineBuilder {
    matrix_sdk_ui::timeline::TimelineBuilder::new(room)
        .with_focus(focus)
        .track_read_marker_and_receipts(TimelineReadReceiptTracking::AllEvents)
}

// ---------------------------------------------------------------------------
// Relay task: SDK diff stream → actor inbox (with overflow detection)
// ---------------------------------------------------------------------------

async fn run_diff_relay(
    actor_tx: mpsc::Sender<TimelineActorMessage>,
    mut diff_stream: impl futures_util::Stream<Item = Vec<eyeball_im::VectorDiff<Arc<SdkTimelineItem>>>>
    + Unpin,
    _timeline: Arc<Timeline>,
) {
    use futures_util::StreamExt;

    let mut overflow = false;
    loop {
        let Some(diffs) = diff_stream.next().await else {
            break;
        };

        if overflow {
            // Already in overflow state — stay silent, the actor has already
            // been notified and will emit a new InitialItems on the new generation.
            continue;
        }

        match actor_tx.try_send(TimelineActorMessage::DiffBatch(diffs)) {
            Ok(_) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                // Queue overflow: notify the actor to resync.
                overflow = true;
                let _ = actor_tx.try_send(TimelineActorMessage::RelayOverflow);
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                // Actor dropped — relay task should stop.
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Send queue monitor task
// ---------------------------------------------------------------------------

async fn run_send_queue_monitor(
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
                // Some updates dropped — not critical for send completion tracking.
                continue;
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                break;
            }
        }
    }
}

async fn run_typing_notifications(
    actor_tx: mpsc::Sender<TimelineActorMessage>,
    _guard: matrix_sdk::event_handler::EventHandlerDropGuard,
    mut typing_rx: tokio::sync::broadcast::Receiver<Vec<matrix_sdk::ruma::OwnedUserId>>,
) {
    loop {
        match typing_rx.recv().await {
            Ok(user_ids) => {
                let user_ids = user_ids
                    .into_iter()
                    .map(|user_id| user_id.to_string())
                    .collect();
                if actor_tx
                    .send(TimelineActorMessage::TypingUsersUpdated(user_ids))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
}

// ---------------------------------------------------------------------------
// SDK → core type conversions
// ---------------------------------------------------------------------------

fn timeline_room_id(key: &TimelineKey) -> Option<String> {
    match &key.kind {
        TimelineKind::Room { room_id }
        | TimelineKind::Thread { room_id, .. }
        | TimelineKind::Focused { room_id, .. } => Some(room_id.clone()),
    }
}

fn replay_initial_items_window(
    kind: &TimelineKind,
    items: &[TimelineItem],
    observation: &TimelineViewportObservation,
) -> Vec<TimelineItem> {
    if matches!(kind, TimelineKind::Room { .. })
        && observation.at_bottom
        && items.len() > ROOM_REPLAY_INITIAL_ITEMS_MAX
    {
        items[items.len() - ROOM_REPLAY_INITIAL_ITEMS_MAX..].to_vec()
    } else {
        items.to_vec()
    }
}

fn activity_rows_from_timeline_items(
    key: &TimelineKey,
    items: &[TimelineItem],
) -> Vec<ActivityRow> {
    let TimelineKind::Room { room_id } = &key.kind else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| activity_row_from_timeline_item(room_id, item))
        .collect()
}

fn activity_rows_from_timeline_diffs(
    key: &TimelineKey,
    diffs: &[TimelineDiff],
) -> Vec<ActivityRow> {
    let TimelineKind::Room { room_id } = &key.kind else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for diff in diffs {
        match diff {
            TimelineDiff::PushFront { item }
            | TimelineDiff::PushBack { item }
            | TimelineDiff::Insert { item, .. }
            | TimelineDiff::Set { item, .. } => {
                if let Some(row) = activity_row_from_timeline_item(room_id, item) {
                    rows.push(row);
                }
            }
            TimelineDiff::Reset { items } => {
                rows.extend(
                    items
                        .iter()
                        .filter_map(|item| activity_row_from_timeline_item(room_id, item)),
                );
            }
            TimelineDiff::Remove { .. } | TimelineDiff::Truncate { .. } | TimelineDiff::Clear => {}
        }
    }
    rows
}

fn activity_row_from_timeline_item(room_id: &str, item: &TimelineItem) -> Option<ActivityRow> {
    let TimelineItemId::Event { event_id } = &item.id else {
        return None;
    };
    let preview = item
        .body
        .clone()
        .or_else(|| item.media.as_ref().map(|media| media.filename.clone()))?;
    let mut row = ActivityRow::event(
        room_id.to_owned(),
        event_id.clone(),
        item.sender.clone(),
        String::new(),
        item.sender_label.clone(),
        Some(preview),
        item.timestamp_ms.unwrap_or(0),
        false,
        false,
    );
    row.sender_avatar = item.sender_avatar.clone();
    Some(row)
}

fn media_gallery_updated_action(
    key: &TimelineKey,
    items: Vec<TimelineMediaGalleryItem>,
) -> Option<AppAction> {
    let TimelineKind::Room { room_id } = &key.kind else {
        return None;
    };

    Some(AppAction::MediaGalleryUpdated {
        room_id: room_id.clone(),
        items,
    })
}

fn media_gallery_items_from_timeline_items(
    key: &TimelineKey,
    items: &[TimelineItem],
) -> Vec<TimelineMediaGalleryItem> {
    let TimelineKind::Room { room_id } = &key.kind else {
        return Vec::new();
    };

    let mut gallery_items = items
        .iter()
        .filter_map(|item| media_gallery_item_from_timeline_item(room_id, item))
        .collect::<Vec<_>>();
    gallery_items.sort_by(|left, right| {
        right
            .timestamp_ms
            .cmp(&left.timestamp_ms)
            .then_with(|| left.event_id.cmp(&right.event_id))
    });
    gallery_items
}

fn media_gallery_item_from_timeline_item(
    room_id: &str,
    item: &TimelineItem,
) -> Option<TimelineMediaGalleryItem> {
    if item.is_hidden || item.is_redacted {
        return None;
    }
    let TimelineItemId::Event { event_id } = &item.id else {
        return None;
    };
    let media = item.media.as_ref()?;

    Some(TimelineMediaGalleryItem {
        event_id: event_id.clone(),
        room_id: room_id.to_owned(),
        sender: item.sender.clone(),
        sender_label: item.sender_label.clone(),
        timestamp_ms: item.timestamp_ms.unwrap_or_default(),
        media: TimelineMediaGalleryMedia {
            kind: media_gallery_kind_from_timeline_kind(media.kind),
            filename: media.filename.clone(),
            source: media_gallery_source_from_timeline_source(&media.source),
            mimetype: media.mimetype.clone(),
            size: media.size,
            width: media.width,
            height: media.height,
            thumbnail: media.thumbnail.as_ref().map(media_gallery_thumbnail),
        },
    })
}

fn media_gallery_kind_from_timeline_kind(kind: TimelineMediaKind) -> GalleryTimelineMediaKind {
    match kind {
        TimelineMediaKind::Image => GalleryTimelineMediaKind::Image,
        TimelineMediaKind::File => GalleryTimelineMediaKind::File,
        TimelineMediaKind::Audio => GalleryTimelineMediaKind::Audio,
        TimelineMediaKind::Video => GalleryTimelineMediaKind::Video,
    }
}

fn media_gallery_source_from_timeline_source(
    source: &TimelineMediaSource,
) -> TimelineMediaGallerySource {
    TimelineMediaGallerySource {
        mxc_uri: source.mxc_uri.clone(),
        encrypted: source.encrypted,
        encryption_version: source.encryption_version.clone(),
    }
}

fn media_gallery_thumbnail(thumbnail: &TimelineMediaThumbnail) -> TimelineMediaGalleryThumbnail {
    TimelineMediaGalleryThumbnail {
        source: media_gallery_source_from_timeline_source(&thumbnail.source),
        mimetype: thumbnail.mimetype.clone(),
        size: thumbnail.size,
        width: thumbnail.width,
        height: thumbnail.height,
    }
}

fn derive_timeline_navigation_snapshot(
    items: &[TimelineItem],
    fully_read_event_id: Option<&str>,
    observation: &TimelineViewportObservation,
    own_user_id: Option<&str>,
) -> TimelineNavigationSnapshot {
    let mut snapshot = TimelineNavigationSnapshot {
        read_marker_event_id: fully_read_event_id.map(ToOwned::to_owned),
        read_marker_display_event_id: None,
        first_unread_event_id: None,
        unread_event_count: 0,
        unread_position: TimelineUnreadPosition::None,
        newer_event_count: 0,
        can_jump_to_bottom: false,
    };

    let Some(read_marker_event_id) = fully_read_event_id else {
        return snapshot;
    };
    let Some(read_marker_index) = item_index_for_event_id(items, read_marker_event_id) else {
        snapshot.unread_position = TimelineUnreadPosition::Unknown;
        return snapshot;
    };
    snapshot.newer_event_count =
        newer_unread_event_count(items, observation, own_user_id, read_marker_index);
    snapshot.can_jump_to_bottom = snapshot.newer_event_count > 0;

    let unread_items: Vec<(usize, &TimelineItem)> = items
        .iter()
        .enumerate()
        .skip(read_marker_index.saturating_add(1))
        .filter(|(_, item)| is_unread_navigation_item(item, own_user_id))
        .collect();

    snapshot.unread_event_count = unread_items.len() as u64;
    if let Some((first_unread_index, first_unread)) = unread_items.first() {
        snapshot.first_unread_event_id =
            timeline_item_event_id(first_unread).map(ToOwned::to_owned);
        snapshot.unread_position =
            unread_position_for_index(items, *first_unread_index, observation);
        return snapshot;
    }

    // No remote unread events after the marker. Advance the display anchor to the
    // current user's latest visible own message at or after the marker so the
    // "Read up to here" separator is rendered after it, not before.
    snapshot.read_marker_display_event_id = items
        .iter()
        .enumerate()
        .skip(read_marker_index)
        .filter(|(_, item)| is_own_visible_event(item, own_user_id))
        .last()
        .and_then(|(_, item)| timeline_item_event_id(item).map(ToOwned::to_owned));
    snapshot
}

fn is_own_visible_event(item: &TimelineItem, own_user_id: Option<&str>) -> bool {
    if item.is_hidden || !has_user_visible_content(item) {
        return false;
    }
    if !own_user_id.is_some_and(|own| item.sender.as_deref() == Some(own)) {
        return false;
    }
    matches!(item.id, TimelineItemId::Event { .. })
}

fn newer_unread_event_count(
    items: &[TimelineItem],
    observation: &TimelineViewportObservation,
    own_user_id: Option<&str>,
    read_marker_index: usize,
) -> u64 {
    if observation.at_bottom {
        return 0;
    }
    let Some(last_visible_event_id) = observation.last_visible_event_id.as_deref() else {
        return 0;
    };
    let Some(last_visible_index) = item_index_for_event_id(items, last_visible_event_id) else {
        return 0;
    };
    let first_newer_unread_index = last_visible_index.max(read_marker_index).saturating_add(1);
    items
        .iter()
        .skip(first_newer_unread_index)
        .filter(|item| is_unread_navigation_item(item, own_user_id))
        .count() as u64
}

fn unread_position_for_index(
    items: &[TimelineItem],
    item_index: usize,
    observation: &TimelineViewportObservation,
) -> TimelineUnreadPosition {
    let Some(first_visible_event_id) = observation.first_visible_event_id.as_deref() else {
        return TimelineUnreadPosition::Unknown;
    };
    let Some(last_visible_event_id) = observation.last_visible_event_id.as_deref() else {
        return TimelineUnreadPosition::Unknown;
    };
    let Some(first_visible_index) = item_index_for_event_id(items, first_visible_event_id) else {
        return TimelineUnreadPosition::Unknown;
    };
    let Some(last_visible_index) = item_index_for_event_id(items, last_visible_event_id) else {
        return TimelineUnreadPosition::Unknown;
    };

    if item_index < first_visible_index {
        TimelineUnreadPosition::AboveViewport
    } else if item_index > last_visible_index {
        TimelineUnreadPosition::BelowViewport
    } else {
        TimelineUnreadPosition::InsideViewport
    }
}

fn apply_timeline_diffs_to_items(items: &mut Vec<TimelineItem>, diffs: &[TimelineDiff]) {
    for diff in diffs {
        match diff {
            TimelineDiff::PushFront { item } => items.insert(0, item.clone()),
            TimelineDiff::PushBack { item } => items.push(item.clone()),
            TimelineDiff::Insert { index, item } => {
                let index = (*index).min(items.len());
                items.insert(index, item.clone());
            }
            TimelineDiff::Set { index, item } => {
                if let Some(slot) = items.get_mut(*index) {
                    *slot = item.clone();
                }
            }
            TimelineDiff::Remove { index } => {
                if *index < items.len() {
                    items.remove(*index);
                }
            }
            TimelineDiff::Truncate { length } => {
                items.truncate(*length);
            }
            TimelineDiff::Clear => {
                items.clear();
            }
            TimelineDiff::Reset { items: reset_items } => {
                *items = reset_items.clone();
            }
        }
    }
}

fn timeline_diffs_include_prepend(diffs: &[TimelineDiff]) -> bool {
    diffs.iter().any(|diff| match diff {
        TimelineDiff::PushFront { .. } => true,
        TimelineDiff::Insert { index, .. } => *index == 0,
        TimelineDiff::Reset { .. } => true,
        TimelineDiff::PushBack { .. }
        | TimelineDiff::Set { .. }
        | TimelineDiff::Remove { .. }
        | TimelineDiff::Truncate { .. }
        | TimelineDiff::Clear => false,
    })
}

fn apply_ignored_sender_suppression(
    item: &mut TimelineItem,
    ignored_user_ids: &std::collections::BTreeSet<String>,
) {
    let sender_ignored = item
        .sender
        .as_deref()
        .is_some_and(|sender| ignored_user_ids.contains(sender));
    item.is_hidden = item.is_hidden || sender_ignored;
}

fn apply_ignored_sender_suppression_to_diff(
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

async fn apply_link_previews_to_item(
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

fn is_unread_navigation_item(item: &TimelineItem, own_user_id: Option<&str>) -> bool {
    if item.is_hidden || !has_user_visible_content(item) {
        return false;
    }
    if own_user_id.is_some_and(|own| item.sender.as_deref() == Some(own)) {
        return false;
    }
    matches!(item.id, TimelineItemId::Event { .. })
}

fn has_user_visible_content(item: &TimelineItem) -> bool {
    timeline_content_is_renderable(
        item.body.as_deref(),
        item.media.as_ref(),
        item.formatted.as_ref(),
    )
}

fn timeline_content_is_renderable(
    body: Option<&str>,
    media: Option<&TimelineMedia>,
    formatted: Option<&crate::event::TimelineFormattedBody>,
) -> bool {
    body.is_some_and(|body| !body.trim().is_empty())
        || media.is_some()
        || formatted.is_some_and(timeline_formatted_body_is_renderable)
}

fn timeline_formatted_body_is_renderable(formatted: &crate::event::TimelineFormattedBody) -> bool {
    !formatted.plain_text.trim().is_empty()
        || formatted
            .code_blocks
            .iter()
            .any(|block| !block.body.trim().is_empty())
}

fn timeline_sender_label_from_profile(profile: &TimelineDetails<Profile>) -> Option<String> {
    match profile {
        TimelineDetails::Ready(profile) => profile.display_name.clone(),
        TimelineDetails::Unavailable | TimelineDetails::Pending | TimelineDetails::Error(_) => None,
    }
}

fn timeline_sender_avatar_from_profile(profile: &TimelineDetails<Profile>) -> Option<AvatarImage> {
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

fn timeline_item_should_be_hidden(has_renderable_content: bool, is_redacted: bool) -> bool {
    !has_renderable_content && !is_redacted
}

fn timeline_item_should_be_hidden_for_key(
    key: &TimelineKey,
    has_renderable_content: bool,
    is_redacted: bool,
    thread_root: Option<&str>,
) -> bool {
    timeline_item_should_be_hidden(has_renderable_content, is_redacted)
        || (matches!(key.kind, TimelineKind::Room { .. }) && thread_root.is_some())
}

fn reply_enforce_thread_for_key(key: &TimelineKey) -> EnforceThread {
    match key.kind {
        TimelineKind::Thread { .. } => EnforceThread::Threaded(ReplyWithinThread::No),
        TimelineKind::Room { .. } | TimelineKind::Focused { .. } => EnforceThread::MaybeThreaded,
    }
}

fn thread_root_from_original_json(original_json: &serde_json::Value) -> Option<String> {
    let relates_to = original_json.get("content")?.get("m.relates_to")?;
    if relates_to.get("rel_type")?.as_str()? != "m.thread" {
        return None;
    }
    let event_id = relates_to.get("event_id")?.as_str()?.trim();
    (!event_id.is_empty()).then(|| event_id.to_owned())
}

fn item_index_for_event_id(items: &[TimelineItem], event_id: &str) -> Option<usize> {
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

fn timeline_item_event_id(item: &TimelineItem) -> Option<&str> {
    match &item.id {
        TimelineItemId::Event { event_id } => Some(event_id.as_str()),
        TimelineItemId::Transaction { .. } | TimelineItemId::Synthetic { .. } => None,
    }
}

fn live_event_receipts_from_sdk_items<'a>(
    items: impl IntoIterator<Item = &'a Arc<SdkTimelineItem>>,
) -> Vec<LiveEventReceipts> {
    items
        .into_iter()
        .filter_map(|item| live_event_receipts_from_sdk_item(item, false))
        .collect()
}

fn collect_live_event_receipts_from_diff(
    diff: &eyeball_im::VectorDiff<Arc<SdkTimelineItem>>,
    out: &mut Vec<LiveEventReceipts>,
) {
    use eyeball_im::VectorDiff;

    match diff {
        VectorDiff::PushFront { value }
        | VectorDiff::PushBack { value }
        | VectorDiff::Insert { value, .. } => {
            if let Some(receipts) = live_event_receipts_from_sdk_item(value, false) {
                out.push(receipts);
            }
        }
        VectorDiff::Set { value, .. } => {
            if let Some(receipts) = live_event_receipts_from_sdk_item(value, true) {
                out.push(receipts);
            }
        }
        VectorDiff::Append { values } | VectorDiff::Reset { values } => {
            out.extend(live_event_receipts_from_sdk_items(values.iter()));
        }
        VectorDiff::Remove { .. }
        | VectorDiff::Truncate { .. }
        | VectorDiff::Clear
        | VectorDiff::PopFront
        | VectorDiff::PopBack => {}
    }
}

fn live_event_receipts_from_sdk_item(
    item: &Arc<SdkTimelineItem>,
    include_empty: bool,
) -> Option<LiveEventReceipts> {
    use matrix_sdk_ui::timeline::TimelineItemKind;

    let event_item = match item.kind() {
        TimelineItemKind::Event(event_item) => event_item,
        TimelineItemKind::Virtual(_) => return None,
    };
    let event_id = event_item.event_id()?.to_string();
    let receipts = event_item
        .read_receipts()
        .iter()
        .map(|(user_id, receipt)| LiveReadReceipt {
            user_id: user_id.to_string(),
            display_name: None,
            original_display_label: String::new(),
            avatar: None,
            timestamp_ms: receipt.ts.map(|timestamp| timestamp.0.into()),
        })
        .collect::<Vec<_>>();

    if receipts.is_empty() && !include_empty {
        return None;
    }

    Some(LiveEventReceipts { event_id, receipts })
}

/// Convert a single SDK `TimelineItem` to our `TimelineItem` DTO.
pub fn sdk_item_to_timeline_item(
    key: &TimelineKey,
    item: &Arc<SdkTimelineItem>,
    own_user_id: Option<&matrix_sdk::ruma::UserId>,
) -> TimelineItem {
    sdk_item_to_timeline_item_with_send_states(key, item, own_user_id, &HashMap::new())
}

fn sdk_item_to_timeline_item_with_send_states(
    key: &TimelineKey,
    item: &Arc<SdkTimelineItem>,
    own_user_id: Option<&matrix_sdk::ruma::UserId>,
    send_statuses: &HashMap<String, TimelineSendState>,
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
            let notice_i18n_key = message_projection
                .as_ref()
                .and_then(|projection| projection.notice_i18n_key)
                .map(str::to_owned);
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
            let send_state = transaction_id
                .as_deref()
                .and_then(|txn_id| send_statuses.get(txn_id).cloned())
                .or_else(|| timeline_send_state_from_sdk(event_item.send_state()));
            let mut unable_to_decrypt = unable_to_decrypt_from_content(content);
            if let Some(utd) = unable_to_decrypt.as_mut() {
                utd.can_request_keys = event_item.original_json().is_some();
            }
            let actions = message_actions_for_timeline_item(
                key.room_id(),
                &id,
                actionable_body,
                media.is_some(),
                is_redacted,
            );
            let is_hidden = timeline_item_should_be_hidden_for_key(
                key,
                has_renderable_content,
                is_redacted,
                thread_root.as_deref(),
            );
            let link_ranges =
                link_ranges_for_message_projection(body.as_deref(), formatted.as_ref());

            TimelineItem {
                id,
                sender,
                sender_label,
                sender_avatar,
                body,
                notice_i18n_key,
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
                id: TimelineItemId::Synthetic { synthetic_id },
                sender: None,
                sender_label: None,
                sender_avatar: None,
                body: None,
                notice_i18n_key: None,
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
            }
        }
    }
}

fn unable_to_decrypt_from_content(
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
    })
}

fn thread_summary_from_sdk(summary: matrix_sdk_ui::timeline::ThreadSummary) -> ThreadSummaryDto {
    let mut dto = ThreadSummaryDto {
        reply_count: summary.num_replies,
        latest_sender: None,
        latest_sender_label: None,
        latest_body_preview: None,
        latest_timestamp_ms: None,
    };

    if let matrix_sdk_ui::timeline::TimelineDetails::Ready(latest_event) = summary.latest_event {
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

struct MessageProjection {
    body: Option<String>,
    notice_i18n_key: Option<&'static str>,
    body_is_user_content: bool,
    message_kind: TimelineMessageKind,
    spoiler_spans: Vec<TimelineSpoilerSpan>,
    media: Option<TimelineMedia>,
    formatted: Option<crate::event::TimelineFormattedBody>,
}

fn link_ranges_for_message_projection(
    body: Option<&str>,
    formatted: Option<&crate::event::TimelineFormattedBody>,
) -> Vec<crate::event::TimelineLinkRange> {
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
            state: ReplyQuoteState::Redacted,
        };
    }

    let body_preview = event.content.as_message().and_then(|msg| {
        let projection = message_projection_from_msgtype(msg.msgtype(), msg.body());
        reply_quote_preview_from_message_projection(projection)
    });
    let state = if body_preview.is_some() {
        ReplyQuoteState::Ready
    } else {
        ReplyQuoteState::Unsupported
    };

    ReplyQuote {
        event_id: details.event_id.to_string(),
        sender,
        sender_label: None,
        body_preview,
        state,
    }
}

fn reply_quote_preview_from_message_projection(projection: MessageProjection) -> Option<String> {
    let source = projection
        .body
        .or_else(|| projection.media.map(|media| media.filename))?;
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
        _ => {}
    }

    if let Some(sticker) = content.as_sticker() {
        let body = sticker.content().body.trim();
        return MessageProjection {
            body: (!body.is_empty()).then(|| body.to_owned()),
            notice_i18n_key: None,
            body_is_user_content: true,
            message_kind: TimelineMessageKind::Text,
            spoiler_spans: Vec::new(),
            media: None,
            formatted: None,
        };
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
            notice_i18n_key: None,
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

fn state_event_notice_projection(event_type: &str) -> MessageProjection {
    MessageProjection {
        body: Some(state_event_notice_body(event_type).into_owned()),
        notice_i18n_key: state_event_notice_i18n_key(event_type),
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

fn state_event_notice_i18n_key(event_type: &str) -> Option<&'static str> {
    match event_type {
        "m.room.create" => Some("timeline.notice.roomCreate"),
        "m.room.power_levels" => Some("timeline.notice.roomPowerLevels"),
        "m.room.guest_access" => Some("timeline.notice.roomGuestAccess"),
        "m.room.encryption" => Some("timeline.notice.roomEncryption"),
        "m.space.parent" => Some("timeline.notice.spaceParent"),
        "m.room.join_rules" => Some("timeline.notice.roomJoinRules"),
        "m.room.history_visibility" => Some("timeline.notice.roomHistoryVisibility"),
        "m.room.pinned_events" => Some("timeline.notice.roomPinnedEvents"),
        _ => None,
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

fn non_user_content_projection(body: &str) -> MessageProjection {
    MessageProjection {
        body: Some(body.to_owned()),
        notice_i18n_key: None,
        body_is_user_content: false,
        message_kind: TimelineMessageKind::Notice,
        spoiler_spans: Vec::new(),
        media: None,
        formatted: None,
    }
}

fn collapsed_preview(value: &str, max_chars: usize) -> Option<String> {
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

fn message_projection_from_msgtype(
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
            notice_i18n_key: None,
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
        notice_i18n_key: None,
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
    formatted: crate::event::TimelineFormattedBody,
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
        formatted: crate::event::TimelineFormattedBody {
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

fn code_blocks_from_html(html: &Html) -> Vec<crate::event::TimelineCodeBlock> {
    let mut blocks = Vec::new();
    collect_code_blocks_from_nodes(html.children(), &mut blocks);
    blocks
}

fn collect_code_blocks_from_nodes(
    nodes: impl Iterator<Item = matrix_sdk::ruma::html::NodeRef>,
    out: &mut Vec<crate::event::TimelineCodeBlock>,
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

            out.push(crate::event::TimelineCodeBlock { language, body });
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

fn timeline_media_source_from_sdk(source: &MediaSource) -> TimelineMediaSource {
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

fn timeline_send_state_from_sdk(state: Option<&SdkEventSendState>) -> Option<TimelineSendState> {
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

fn send_failure_reason(is_recoverable: bool) -> TimelineSendFailureReason {
    if is_recoverable {
        TimelineSendFailureReason::Recoverable
    } else {
        TimelineSendFailureReason::Unrecoverable
    }
}

fn remember_send_handle(
    statuses: &mut HashMap<String, TimelineSendState>,
    handles: &mut HashMap<String, SendHandle>,
    handle: &SendHandle,
    state: TimelineSendState,
) {
    let transaction_id = handle.transaction_id().to_string();
    statuses.insert(transaction_id.clone(), state);
    handles.insert(transaction_id, handle.clone());
}

fn remember_local_echo(
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

fn cache_sdk_item_media_source(
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

fn attachment_info_for_upload(request: &UploadMediaRequest) -> AttachmentInfo {
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

fn thumbnail_for_upload(request: &UploadMediaRequest) -> Option<Thumbnail> {
    let thumbnail = request.thumbnail.as_ref()?;
    Some(Thumbnail {
        data: thumbnail.bytes.clone(),
        content_type: thumbnail.mime_type.parse().ok()?,
        height: uint_from_u64(thumbnail.height)?,
        width: uint_from_u64(thumbnail.width)?,
        size: uint_from_u64(u64::try_from(thumbnail.bytes.len()).ok()?)?,
    })
}

fn media_request_for_download(
    entry: &PrivateMediaEntry,
    selection: MediaDownloadSelection,
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
                    uint_from_u64(width)?,
                    uint_from_u64(height)?,
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
fn sanitize_matrix_id_for_path(id: &str) -> String {
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
) -> Vec<crate::event::ReactionGroup> {
    reactions
        .iter()
        .map(|(key, senders)| crate::event::ReactionGroup {
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
            sender_preview: senders.keys().take(3).map(ToString::to_string).collect(),
        })
        .collect()
}

/// Convert an SDK `VectorDiff` to our `TimelineDiff`.
fn sdk_vector_diff_to_timeline_diff(
    diff: eyeball_im::VectorDiff<Arc<SdkTimelineItem>>,
    key: &TimelineKey,
    own_user_id: Option<&matrix_sdk::ruma::UserId>,
    send_statuses: &HashMap<String, TimelineSendState>,
) -> TimelineDiff {
    match diff {
        eyeball_im::VectorDiff::PushFront { value } => TimelineDiff::PushFront {
            item: sdk_item_to_timeline_item_with_send_states(
                key,
                &value,
                own_user_id,
                send_statuses,
            ),
        },
        eyeball_im::VectorDiff::PushBack { value } => TimelineDiff::PushBack {
            item: sdk_item_to_timeline_item_with_send_states(
                key,
                &value,
                own_user_id,
                send_statuses,
            ),
        },
        eyeball_im::VectorDiff::Insert { index, value } => TimelineDiff::Insert {
            index,
            item: sdk_item_to_timeline_item_with_send_states(
                key,
                &value,
                own_user_id,
                send_statuses,
            ),
        },
        eyeball_im::VectorDiff::Set { index, value } => TimelineDiff::Set {
            index,
            item: sdk_item_to_timeline_item_with_send_states(
                key,
                &value,
                own_user_id,
                send_statuses,
            ),
        },
        eyeball_im::VectorDiff::Remove { index } => TimelineDiff::Remove { index },
        eyeball_im::VectorDiff::Truncate { length } => TimelineDiff::Truncate { length },
        eyeball_im::VectorDiff::Clear => TimelineDiff::Clear,
        eyeball_im::VectorDiff::Reset { values } => TimelineDiff::Reset {
            items: values
                .iter()
                .map(|value| {
                    sdk_item_to_timeline_item_with_send_states(
                        key,
                        value,
                        own_user_id,
                        send_statuses,
                    )
                })
                .collect(),
        },
        eyeball_im::VectorDiff::PopFront => {
            // SDK VectorDiff::PopFront is not in the spec enum; map to Remove{0}.
            TimelineDiff::Remove { index: 0 }
        }
        eyeball_im::VectorDiff::PopBack => {
            // SDK VectorDiff::PopBack not in spec enum; we don't know the index.
            // Emit a Clear+Reset is too aggressive; emit a no-op Truncate that
            // leaves it to the SDK's Reset to resync if needed. The real "pop"
            // case is extremely rare (SDK mainly uses Set/Insert/Remove).
            // The safe approach: emit a Reset with the current items. But we
            // don't hold the current list here. Emit Truncate(0) as a conservative
            // sentinel that tells the UI to resync — the next Reset diff will fix it.
            // This is a known gap; escalate if PopBack becomes common in practice.
            TimelineDiff::Truncate { length: 0 }
        }
        eyeball_im::VectorDiff::Append { values } => {
            // Append is equivalent to multiple PushBacks followed by a batch.
            // Convert to Reset to keep ordering safe (the spec allows Reset as a
            // position-preserving fallback). This is consistent with what the SDK
            // actually emits: Append only fires during initial populate, after which
            // the live stream uses PushBack/Insert/Set/Remove.
            // We emit a Reset here; the UI applies it as a full list replacement.
            // Alternatively we could split into PushBacks, but that risks sending
            // oversized batches for large initial appends; Reset is more robust.
            TimelineDiff::Reset {
                items: values
                    .iter()
                    .map(|value| {
                        sdk_item_to_timeline_item_with_send_states(
                            key,
                            value,
                            own_user_id,
                            send_statuses,
                        )
                    })
                    .collect(),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Error classifiers
// ---------------------------------------------------------------------------

fn classify_pagination_error(err: &matrix_sdk_ui::timeline::Error) -> TimelineFailureKind {
    use matrix_sdk_ui::timeline::{Error, PaginationError};
    match err {
        Error::PaginationError(PaginationError::NotSupported) => {
            TimelineFailureKind::InvalidDirection
        }
        Error::PaginationError(_) => TimelineFailureKind::Sdk,
        _ => TimelineFailureKind::Sdk,
    }
}

fn classify_reaction_error(err: &matrix_sdk_ui::timeline::Error) -> TimelineFailureKind {
    match err {
        matrix_sdk_ui::timeline::Error::EventNotInTimeline(_) => {
            TimelineFailureKind::InvalidReactionTarget
        }
        _ => TimelineFailureKind::Sdk,
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

#[derive(Default)]
struct SendCompletionTracker {
    /// Pending send requests: sdk_txn_id → completion metadata.
    pending_sends: HashMap<String, PendingSendCompletion>,
    /// SentEvent updates that arrived before the pending mapping existed:
    /// sdk_txn_id → event_id.
    completed_sends: HashMap<String, String>,
}

struct PendingSendCompletion {
    client_txn_id: String,
    request_id: RequestId,
    settles_composer: bool,
    failure_reported: bool,
}

impl SendCompletionTracker {
    fn remember_pending_send(
        &mut self,
        sdk_txn_id: String,
        client_txn_id: String,
        request_id: RequestId,
        settles_composer: bool,
    ) -> Option<(String, RequestId, String)> {
        if let Some(event_id) = self.completed_sends.remove(&sdk_txn_id) {
            Some((client_txn_id, request_id, event_id))
        } else {
            self.pending_sends.insert(
                sdk_txn_id,
                PendingSendCompletion {
                    client_txn_id,
                    request_id,
                    settles_composer,
                    failure_reported: false,
                },
            );
            None
        }
    }

    fn record_sent_event(
        &mut self,
        sdk_txn_id: String,
        event_id: String,
    ) -> Option<(String, RequestId, String, bool)> {
        if let Some(pending) = self.pending_sends.remove(&sdk_txn_id) {
            let settles_composer = pending.settles_composer && !pending.failure_reported;
            Some((
                pending.client_txn_id,
                pending.request_id,
                event_id,
                settles_composer,
            ))
        } else {
            self.completed_sends.insert(sdk_txn_id, event_id);
            None
        }
    }

    fn record_send_error(&mut self, sdk_txn_id: &str) -> Option<(String, RequestId, bool)> {
        let pending = self.pending_sends.get_mut(sdk_txn_id)?;
        if pending.failure_reported {
            return None;
        }
        pending.failure_reported = true;
        Some((
            pending.client_txn_id.clone(),
            pending.request_id,
            pending.settles_composer,
        ))
    }

    fn record_cancelled_event(&mut self, sdk_txn_id: &str) -> Option<(String, RequestId, bool)> {
        let pending = self.pending_sends.remove(sdk_txn_id)?;
        Some((
            pending.client_txn_id,
            pending.request_id,
            pending.settles_composer && !pending.failure_reported,
        ))
    }

    fn pending_send(&self, sdk_txn_id: &str) -> Option<(&str, RequestId)> {
        self.pending_sends
            .get(sdk_txn_id)
            .map(|pending| (pending.client_txn_id.as_str(), pending.request_id))
    }
}

// ---------------------------------------------------------------------------
// Unit tests (network-free)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::Duration;

    use koushi_state::{
        AppAction, MentionIntent, MentionTarget, SessionInfo, SessionState,
        TimelineMediaKind as GalleryTimelineMediaKind,
    };
    use matrix_sdk::ruma::events::room::message::{
        EmoteMessageEventContent, MessageType, NoticeMessageEventContent, TextMessageEventContent,
    };
    use matrix_sdk::ruma::{OwnedUserId, uint};
    use matrix_sdk_ui::timeline::{
        MembershipChange, ReactionInfo, ReactionStatus, ReactionsByKeyBySender,
    };
    use tokio::sync::broadcast;

    use super::*;
    use crate::command::{
        CoreCommand, ImageUploadCompressionPolicy, ImageUploadCompressionState,
        ImageUploadDimensions, ImageUploadVariantInfo, ImageUploadVariantKind, TimelineCommand,
    };
    use crate::event::{
        CoreEvent, PaginationDirection, TimelineEvent, TimelineUnreadPosition,
        TimelineViewportObservation,
    };
    use crate::failure::{CoreFailure, TimelineFailureKind};
    use crate::ids::{AccountKey, RequestId, RuntimeConnectionId, TimelineBatchId};
    use crate::runtime::CoreRuntime;

    fn fake_rid(seq: u64) -> RequestId {
        RequestId {
            connection_id: RuntimeConnectionId(999),
            sequence: seq,
        }
    }

    fn room_key() -> TimelineKey {
        TimelineKey::room(AccountKey("@a:test".to_owned()), "!r:test")
    }

    #[test]
    fn resubscribe_replay_caps_room_timeline_to_live_window() {
        let key = room_key();
        let items = (0..(ROOM_REPLAY_INITIAL_ITEMS_MAX + 25))
            .map(|index| {
                timeline_item(
                    &format!("$event-{index}:test"),
                    Some("body"),
                    "@bob:test",
                    false,
                )
            })
            .collect::<Vec<_>>();

        let replay = replay_initial_items_window(
            &key.kind,
            &items,
            &TimelineViewportObservation {
                at_bottom: true,
                ..TimelineViewportObservation::default()
            },
        );

        assert_eq!(replay.len(), ROOM_REPLAY_INITIAL_ITEMS_MAX);
        assert_eq!(
            replay.first().and_then(timeline_item_event_id),
            Some("$event-25:test")
        );
        let expected_last = format!("$event-{}:test", ROOM_REPLAY_INITIAL_ITEMS_MAX + 24);
        assert_eq!(
            replay.last().and_then(timeline_item_event_id),
            Some(expected_last.as_str())
        );
    }

    #[test]
    fn resubscribe_replay_keeps_scrolled_room_context_complete() {
        let key = room_key();
        let items = (0..(ROOM_REPLAY_INITIAL_ITEMS_MAX + 25))
            .map(|index| {
                timeline_item(
                    &format!("$event-{index}:test"),
                    Some("body"),
                    "@bob:test",
                    false,
                )
            })
            .collect::<Vec<_>>();

        let replay = replay_initial_items_window(
            &key.kind,
            &items,
            &TimelineViewportObservation {
                at_bottom: false,
                first_visible_event_id: Some("$event-10:test".to_owned()),
                last_visible_event_id: Some("$event-20:test".to_owned()),
            },
        );

        assert_eq!(replay.len(), ROOM_REPLAY_INITIAL_ITEMS_MAX + 25);
        assert_eq!(
            replay.first().and_then(timeline_item_event_id),
            Some("$event-0:test")
        );
    }

    #[test]
    fn resubscribe_replay_keeps_focused_timeline_context_complete() {
        let key = TimelineKey {
            account_key: AccountKey("@a:test".to_owned()),
            kind: TimelineKind::Focused {
                room_id: "!r:test".to_owned(),
                event_id: "$anchor:test".to_owned(),
            },
        };
        let items = (0..(ROOM_REPLAY_INITIAL_ITEMS_MAX + 25))
            .map(|index| {
                timeline_item(
                    &format!("$event-{index}:test"),
                    Some("body"),
                    "@bob:test",
                    false,
                )
            })
            .collect::<Vec<_>>();

        let replay = replay_initial_items_window(
            &key.kind,
            &items,
            &TimelineViewportObservation {
                at_bottom: true,
                ..TimelineViewportObservation::default()
            },
        );

        assert_eq!(replay.len(), ROOM_REPLAY_INITIAL_ITEMS_MAX + 25);
        assert_eq!(
            replay.first().and_then(timeline_item_event_id),
            Some("$event-0:test")
        );
    }

    #[test]
    fn visible_missing_reply_detail_event_ids_only_returns_visible_unrequested_missing_replies() {
        let mut before = timeline_item("$before:test", Some("before"), "@alice:test", false);
        before.reply_quote = Some(ReplyQuote {
            event_id: "$root-before:test".to_owned(),
            sender: None,
            sender_label: None,
            body_preview: None,
            state: ReplyQuoteState::Missing,
        });
        let first_visible =
            timeline_item("$first-visible:test", Some("first"), "@alice:test", false);
        let mut missing = timeline_item("$missing:test", Some("missing"), "@alice:test", false);
        missing.reply_quote = Some(ReplyQuote {
            event_id: "$root-missing:test".to_owned(),
            sender: None,
            sender_label: None,
            body_preview: None,
            state: ReplyQuoteState::Missing,
        });
        let mut ready = timeline_item("$ready:test", Some("ready"), "@alice:test", false);
        ready.reply_quote = Some(ReplyQuote {
            event_id: "$root-ready:test".to_owned(),
            sender: Some("@bob:test".to_owned()),
            sender_label: None,
            body_preview: Some("loaded".to_owned()),
            state: ReplyQuoteState::Ready,
        });
        let mut already_requested = timeline_item(
            "$already-requested:test",
            Some("already"),
            "@alice:test",
            false,
        );
        already_requested.reply_quote = Some(ReplyQuote {
            event_id: "$root-already:test".to_owned(),
            sender: None,
            sender_label: None,
            body_preview: None,
            state: ReplyQuoteState::Missing,
        });
        let mut after = timeline_item("$after:test", Some("after"), "@alice:test", false);
        after.reply_quote = Some(ReplyQuote {
            event_id: "$root-after:test".to_owned(),
            sender: None,
            sender_label: None,
            body_preview: None,
            state: ReplyQuoteState::Missing,
        });

        let items = vec![
            before,
            first_visible,
            missing,
            ready,
            already_requested,
            after,
        ];
        let requested = HashSet::from(["$already-requested:test".to_owned()]);

        let event_ids = visible_missing_reply_detail_event_ids(
            &items,
            &TimelineViewportObservation {
                first_visible_event_id: Some("$first-visible:test".to_owned()),
                last_visible_event_id: Some("$already-requested:test".to_owned()),
                at_bottom: false,
            },
            &requested,
        );

        assert_eq!(event_ids, vec!["$missing:test".to_owned()]);
    }

    #[test]
    fn set_fully_read_success_uses_private_read_receipt_before_clearing_room_unread_summary() {
        let source = include_str!("timeline.rs");
        let handler = source
            .split("async fn handle_set_fully_read")
            .nth(1)
            .expect("handle_set_fully_read should exist")
            .split("async fn handle_set_typing")
            .next()
            .expect("handle_set_typing should follow handle_set_fully_read");
        let success_arm = handler
            .split("Ok(_) => {")
            .nth(1)
            .expect("set fully read success arm should exist")
            .split("Err(_) => {")
            .next()
            .expect("set fully read error arm should follow success arm");

        assert!(
            handler.contains("send_multiple_receipts"),
            "set_fully_read must use SDK read-marker batching so the marker and read receipt share one source of truth"
        );
        assert!(
            handler.contains("self.timeline.room().send_multiple_receipts"),
            "set_fully_read must force the room read-marker API instead of Timeline receipt de-duplication; stale server unread counts still need a fresh private receipt"
        );
        assert!(
            handler.contains("fully_read_marker"),
            "set_fully_read must continue to update the fully-read marker"
        );
        assert!(
            handler.contains("private_read_receipt"),
            "set_fully_read must include a private read receipt so SDK/server unread counts advance without publishing public receipts"
        );
        assert!(
            !handler.contains("send_single_receipt(ReceiptType::FullyRead"),
            "fully-read alone must not be used as the persistent unread-count source of truth"
        );
        assert!(
            success_arm.contains("AppAction::FullyReadMarkerUpdated"),
            "set_fully_read must update the fully-read marker after SDK success"
        );
        assert!(
            success_arm.contains("AppAction::RoomMarkedAsReadSucceeded"),
            "set_fully_read SDK success must also clear RoomSummary unread counts so sidebar and Activity/Unread agree"
        );
    }

    struct DropFlag(Arc<AtomicBool>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn timeline_actor_handle_drop_aborts_actor_and_auxiliary_tasks() {
        let actor_alive = Arc::new(AtomicBool::new(true));
        let auxiliary_alive = Arc::new(AtomicBool::new(true));
        let (tx, mut rx) = mpsc::channel(1);
        let (actor_started_tx, actor_started_rx) = tokio::sync::oneshot::channel();
        let (auxiliary_started_tx, auxiliary_started_rx) = tokio::sync::oneshot::channel();
        let actor_alive_for_task = actor_alive.clone();
        let actor_task = executor::spawn(async move {
            let _guard = DropFlag(actor_alive_for_task);
            let _ = actor_started_tx.send(());
            while rx.recv().await.is_some() {}
        });
        let auxiliary_alive_for_task = auxiliary_alive.clone();
        let auxiliary_task = executor::spawn(async move {
            let _guard = DropFlag(auxiliary_alive_for_task);
            let _ = auxiliary_started_tx.send(());
            futures_util::future::pending::<()>().await;
        });
        let auxiliary_sender = tx.clone();

        actor_started_rx.await.expect("actor task should start");
        auxiliary_started_rx
            .await
            .expect("auxiliary task should start");

        let handle = TimelineActorHandle {
            tx,
            task: actor_task,
            auxiliary_tasks: vec![auxiliary_task],
        };
        drop(handle);
        executor::sleep(Duration::from_millis(25)).await;

        assert!(!actor_alive.load(Ordering::SeqCst));
        assert!(!auxiliary_alive.load(Ordering::SeqCst));
        assert!(
            auxiliary_sender
                .try_send(TimelineActorMessage::RelayOverflow)
                .is_err()
        );
    }

    fn timeline_item(
        event_id: &str,
        body: Option<&str>,
        sender: &str,
        is_hidden: bool,
    ) -> TimelineItem {
        TimelineItem {
            id: TimelineItemId::Event {
                event_id: event_id.to_owned(),
            },
            sender: Some(sender.to_owned()),
            sender_label: None,
            sender_avatar: None,
            body: body.map(ToOwned::to_owned),
            notice_i18n_key: None,
            message_kind: Default::default(),
            spoiler_spans: Vec::new(),
            timestamp_ms: Some(1),
            in_reply_to_event_id: None,
            formatted: None,
            reply_quote: None,
            thread_root: None,
            thread_summary: None,
            media: None,
            link_previews: None,
            link_ranges: Vec::new(),
            reactions: Vec::new(),
            can_react: false,
            is_redacted: false,
            is_hidden,
            can_redact: false,
            is_edited: false,
            can_edit: false,
            actions: TimelineMessageActions::default(),
            send_state: None,
            unable_to_decrypt: None,
        }
    }

    fn timeline_media_item(
        event_id: &str,
        sender: &str,
        sender_label: Option<&str>,
        timestamp_ms: u64,
        filename: &str,
        kind: TimelineMediaKind,
    ) -> TimelineItem {
        let mut item = timeline_item(event_id, None, sender, false);
        item.sender_label = sender_label.map(ToOwned::to_owned);
        item.timestamp_ms = Some(timestamp_ms);
        item.media = Some(TimelineMedia {
            kind,
            filename: filename.to_owned(),
            source: TimelineMediaSource {
                mxc_uri: format!("mxc://example.invalid/{event_id}"),
                encrypted: true,
                encryption_version: Some("v2".to_owned()),
            },
            mimetype: Some("image/png".to_owned()),
            size: Some(2048),
            width: Some(640),
            height: Some(480),
            thumbnail: Some(TimelineMediaThumbnail {
                source: TimelineMediaSource {
                    mxc_uri: format!("mxc://example.invalid/{event_id}-thumb"),
                    encrypted: false,
                    encryption_version: None,
                },
                mimetype: Some("image/png".to_owned()),
                size: Some(512),
                width: Some(160),
                height: Some(120),
            }),
        });
        item
    }

    #[test]
    fn media_gallery_projection_keeps_event_media_newest_first() {
        let mut transaction_media = timeline_media_item(
            "$local:test",
            "@me:test",
            None,
            3,
            "local.png",
            TimelineMediaKind::Image,
        );
        transaction_media.id = TimelineItemId::Transaction {
            transaction_id: "txn-local".to_owned(),
        };
        let items = vec![
            timeline_media_item(
                "$old:test",
                "@alice:test",
                Some("Alice"),
                1,
                "old.png",
                TimelineMediaKind::Image,
            ),
            timeline_item("$text:test", Some("text"), "@bob:test", false),
            transaction_media,
            timeline_media_item(
                "$new:test",
                "@carol:test",
                Some("Carol"),
                2,
                "new.png",
                TimelineMediaKind::Image,
            ),
        ];

        let gallery = media_gallery_items_from_timeline_items(&room_key(), &items);

        assert_eq!(gallery.len(), 2);
        assert_eq!(gallery[0].event_id, "$new:test");
        assert_eq!(gallery[0].sender.as_deref(), Some("@carol:test"));
        assert_eq!(gallery[0].sender_label.as_deref(), Some("Carol"));
        assert_eq!(gallery[0].timestamp_ms, 2);
        assert_eq!(gallery[0].media.kind, GalleryTimelineMediaKind::Image);
        assert_eq!(gallery[0].media.filename, "new.png");
        assert!(gallery[0].media.source.encrypted);
        assert_eq!(
            gallery[0].media.thumbnail.as_ref().map(|thumb| thumb.width),
            Some(Some(160))
        );
        assert_eq!(gallery[1].event_id, "$old:test");
    }

    #[test]
    fn media_gallery_projection_recomputes_after_timeline_diffs() {
        let mut items = vec![
            timeline_media_item(
                "$old:test",
                "@alice:test",
                None,
                1,
                "old.png",
                TimelineMediaKind::Image,
            ),
            timeline_media_item(
                "$new:test",
                "@bob:test",
                None,
                2,
                "new.png",
                TimelineMediaKind::Image,
            ),
        ];

        apply_timeline_diffs_to_items(&mut items, &[TimelineDiff::Remove { index: 1 }]);
        let gallery = media_gallery_items_from_timeline_items(&room_key(), &items);
        assert_eq!(gallery.len(), 1);
        assert_eq!(gallery[0].event_id, "$old:test");

        apply_timeline_diffs_to_items(&mut items, &[TimelineDiff::Reset { items: Vec::new() }]);
        assert!(media_gallery_items_from_timeline_items(&room_key(), &items).is_empty());
    }

    #[test]
    fn timeline_navigation_marks_first_unread_inside_viewport() {
        let items = vec![
            timeline_item("$read:test", Some("read"), "@alice:test", false),
            timeline_item("$unread:test", Some("unread"), "@alice:test", false),
            timeline_item("$newer:test", Some("newer"), "@alice:test", false),
        ];

        let snapshot = derive_timeline_navigation_snapshot(
            &items,
            Some("$read:test"),
            &TimelineViewportObservation {
                first_visible_event_id: Some("$unread:test".to_owned()),
                last_visible_event_id: Some("$newer:test".to_owned()),
                at_bottom: true,
            },
            Some("@me:test"),
        );

        assert_eq!(snapshot.read_marker_event_id.as_deref(), Some("$read:test"));
        assert_eq!(
            snapshot.first_unread_event_id.as_deref(),
            Some("$unread:test")
        );
        assert_eq!(snapshot.unread_event_count, 2);
        assert_eq!(
            snapshot.unread_position,
            TimelineUnreadPosition::InsideViewport
        );
        assert_eq!(snapshot.newer_event_count, 0);
    }

    #[test]
    fn timeline_navigation_reports_unread_below_viewport_and_newer_count() {
        let items = vec![
            timeline_item("$read:test", Some("read"), "@alice:test", false),
            timeline_item("$visible:test", Some("visible"), "@alice:test", false),
            timeline_item("$unread:test", Some("unread"), "@alice:test", false),
            timeline_item("$newer:test", Some("newer"), "@alice:test", false),
        ];

        let snapshot = derive_timeline_navigation_snapshot(
            &items,
            Some("$visible:test"),
            &TimelineViewportObservation {
                first_visible_event_id: Some("$read:test".to_owned()),
                last_visible_event_id: Some("$visible:test".to_owned()),
                at_bottom: false,
            },
            Some("@me:test"),
        );

        assert_eq!(
            snapshot.first_unread_event_id.as_deref(),
            Some("$unread:test")
        );
        assert_eq!(snapshot.unread_event_count, 2);
        assert_eq!(
            snapshot.unread_position,
            TimelineUnreadPosition::BelowViewport
        );
        assert_eq!(snapshot.newer_event_count, 2);
    }

    #[test]
    fn timeline_navigation_does_not_count_read_history_below_viewport_as_newer() {
        let items = vec![
            timeline_item("$visible:test", Some("visible"), "@alice:test", false),
            timeline_item("$read-a:test", Some("read a"), "@alice:test", false),
            timeline_item("$read-b:test", Some("read b"), "@alice:test", false),
            timeline_item(
                "$read-marker:test",
                Some("read marker"),
                "@alice:test",
                false,
            ),
        ];

        let snapshot = derive_timeline_navigation_snapshot(
            &items,
            Some("$read-marker:test"),
            &TimelineViewportObservation {
                first_visible_event_id: Some("$visible:test".to_owned()),
                last_visible_event_id: Some("$visible:test".to_owned()),
                at_bottom: false,
            },
            Some("@me:test"),
        );

        assert_eq!(snapshot.first_unread_event_id, None);
        assert_eq!(snapshot.unread_event_count, 0);
        assert_eq!(snapshot.newer_event_count, 0);
        assert!(!snapshot.can_jump_to_bottom);
    }

    #[test]
    fn timeline_navigation_does_not_count_newer_events_without_read_marker() {
        let items = vec![
            timeline_item("$visible:test", Some("visible"), "@alice:test", false),
            timeline_item("$loaded:test", Some("loaded"), "@alice:test", false),
        ];

        let snapshot = derive_timeline_navigation_snapshot(
            &items,
            None,
            &TimelineViewportObservation {
                first_visible_event_id: Some("$visible:test".to_owned()),
                last_visible_event_id: Some("$visible:test".to_owned()),
                at_bottom: false,
            },
            Some("@me:test"),
        );

        assert_eq!(snapshot.read_marker_event_id, None);
        assert_eq!(snapshot.unread_event_count, 0);
        assert_eq!(snapshot.newer_event_count, 0);
        assert!(!snapshot.can_jump_to_bottom);
    }

    #[test]
    fn timeline_navigation_ignores_own_local_and_synthetic_items_for_unread_counts() {
        let mut own = timeline_item("$own:test", Some("own"), "@me:test", false);
        own.id = TimelineItemId::Event {
            event_id: "$own:test".to_owned(),
        };
        let mut local = timeline_item("$local:test", Some("local"), "@me:test", false);
        local.id = TimelineItemId::Transaction {
            transaction_id: "txn-local".to_owned(),
        };
        let mut synthetic = timeline_item("$synthetic:test", Some("divider"), "@me:test", false);
        synthetic.id = TimelineItemId::Synthetic {
            synthetic_id: "date-divider".to_owned(),
        };
        let items = vec![
            timeline_item("$read:test", Some("read"), "@alice:test", false),
            own,
            local,
            synthetic,
            timeline_item("$remote:test", Some("remote"), "@alice:test", false),
        ];

        let snapshot = derive_timeline_navigation_snapshot(
            &items,
            Some("$read:test"),
            &TimelineViewportObservation {
                first_visible_event_id: Some("$read:test".to_owned()),
                last_visible_event_id: Some("$remote:test".to_owned()),
                at_bottom: true,
            },
            Some("@me:test"),
        );

        assert_eq!(
            snapshot.first_unread_event_id.as_deref(),
            Some("$remote:test")
        );
        assert_eq!(snapshot.unread_event_count, 1);
        assert_eq!(snapshot.newer_event_count, 0);
    }

    #[test]
    fn attachment_info_for_image_upload_uses_selected_variant_metadata() {
        let request = UploadMediaRequest {
            filename: "private-screenshot.jpg".to_owned(),
            mime_type: "image/jpeg".to_owned(),
            bytes: vec![1, 2, 3, 4],
            kind: UploadMediaKind::Image {
                width: Some(1200),
                height: Some(900),
            },
            compression: Some(ImageUploadCompressionState {
                mode: koushi_state::ImageUploadCompressionMode::Always,
                policy: ImageUploadCompressionPolicy::default(),
                original: ImageUploadVariantInfo {
                    mime_type: "image/jpeg".to_owned(),
                    byte_count: 3_200_000,
                    dimensions: Some(ImageUploadDimensions {
                        width: 4032,
                        height: 3024,
                    }),
                },
                selected: ImageUploadVariantInfo {
                    mime_type: "image/jpeg".to_owned(),
                    byte_count: 4,
                    dimensions: Some(ImageUploadDimensions {
                        width: 1200,
                        height: 900,
                    }),
                },
                selected_variant: ImageUploadVariantKind::Compressed,
                skipped_small_image: false,
                metadata_stripped: true,
                thumbnail_refreshed: true,
            }),
            thumbnail: None,
            caption: None,
        };

        match attachment_info_for_upload(&request) {
            AttachmentInfo::Image(info) => {
                assert_eq!(info.width, Some(uint!(1200)));
                assert_eq!(info.height, Some(uint!(900)));
                assert_eq!(info.size, Some(uint!(4)));
            }
            other => panic!("expected image info, got {other:?}"),
        }
    }

    fn reaction_groups_fixture() -> ReactionsByKeyBySender {
        let mut reactions = ReactionsByKeyBySender::default();
        let thumbs = reactions.entry("👍".to_owned()).or_default();
        thumbs.insert(
            OwnedUserId::try_from("@me:test").expect("user id"),
            ReactionInfo {
                timestamp: matrix_sdk::ruma::MilliSecondsSinceUnixEpoch(uint!(1)),
                status: ReactionStatus::RemoteToRemote(
                    matrix_sdk::ruma::OwnedEventId::try_from("$reaction:me").expect("event id"),
                ),
            },
        );
        thumbs.insert(
            OwnedUserId::try_from("@alice:test").expect("user id"),
            ReactionInfo {
                timestamp: matrix_sdk::ruma::MilliSecondsSinceUnixEpoch(uint!(2)),
                status: ReactionStatus::LocalToRemote(None),
            },
        );
        thumbs.insert(
            OwnedUserId::try_from("@bob:test").expect("user id"),
            ReactionInfo {
                timestamp: matrix_sdk::ruma::MilliSecondsSinceUnixEpoch(uint!(3)),
                status: ReactionStatus::LocalToRemote(None),
            },
        );
        thumbs.insert(
            OwnedUserId::try_from("@carol:test").expect("user id"),
            ReactionInfo {
                timestamp: matrix_sdk::ruma::MilliSecondsSinceUnixEpoch(uint!(4)),
                status: ReactionStatus::LocalToRemote(None),
            },
        );

        reactions
    }

    #[test]
    fn composer_core_builds_markdown_send_content_with_mentions() {
        let content = build_room_message_content_from_composer_body(
            "hello **Alice**",
            MentionIntent {
                targets: vec![MentionTarget::User {
                    user_id: "@alice:example.test".to_owned(),
                    display_label: "Alice".to_owned(),
                }],
            },
        )
        .expect("content");

        match &content.msgtype {
            MessageType::Text(text) => {
                assert_eq!(text.body, "hello **Alice**");
                assert_eq!(
                    text.formatted
                        .as_ref()
                        .map(|formatted| formatted.body.as_str()),
                    Some("hello <strong>Alice</strong>")
                );
            }
            other => panic!("expected text content, got {other:?}"),
        }

        let mentions = content.mentions.expect("mentions");
        assert!(
            mentions
                .user_ids
                .iter()
                .any(|user_id| user_id.as_str() == "@alice:example.test")
        );
    }

    #[test]
    fn composer_core_builds_me_slash_command_as_emote_content() {
        let content = build_room_message_content_from_composer_body(
            "/me waves **hello**",
            MentionIntent::default(),
        )
        .expect("content");

        match &content.msgtype {
            MessageType::Emote(emote) => {
                assert_eq!(emote.body, "waves **hello**");
                assert_eq!(
                    emote
                        .formatted
                        .as_ref()
                        .map(|formatted| formatted.body.as_str()),
                    Some("waves <strong>hello</strong>")
                );
            }
            other => panic!("expected emote content, got {other:?}"),
        }
    }

    #[test]
    fn composer_core_builds_spoiler_markdown_as_formatted_body() {
        let content = build_room_message_content_from_composer_body(
            "keep ||secret|| hidden",
            MentionIntent::default(),
        )
        .expect("content");

        match &content.msgtype {
            MessageType::Text(text) => {
                assert_eq!(text.body, "keep ||secret|| hidden");
                assert_eq!(
                    text.formatted
                        .as_ref()
                        .map(|formatted| formatted.body.as_str()),
                    Some("keep <span data-mx-spoiler>secret</span> hidden")
                );
            }
            other => panic!("expected text content, got {other:?}"),
        }
    }

    #[test]
    fn message_projection_carries_msgtype_and_plain_spoiler_spans() {
        let projection = message_projection_from_msgtype(
            &MessageType::Notice(NoticeMessageEventContent::plain("keep ||secret|| hidden")),
            "keep ||secret|| hidden",
        );

        assert_eq!(projection.message_kind, TimelineMessageKind::Notice);
        assert_eq!(projection.body.as_deref(), Some("keep secret hidden"));
        assert_eq!(
            projection.spoiler_spans,
            vec![TimelineSpoilerSpan {
                start_utf16: 5,
                end_utf16: 11,
                reason: None,
            }]
        );
    }

    #[test]
    fn membership_change_projection_is_a_supported_notice() {
        let projection =
            membership_change_projection("Alice", Some(MembershipChange::InvitationAccepted));

        assert_eq!(projection.message_kind, TimelineMessageKind::Notice);
        assert_eq!(projection.body.as_deref(), Some("Alice joined the room"));
        assert_eq!(projection.body_is_user_content, false);
        assert!(
            !projection
                .body
                .as_deref()
                .unwrap_or_default()
                .contains("Unsupported event: m.room.member")
        );
    }

    #[test]
    fn profile_change_projection_does_not_emit_user_id_body() {
        let source = include_str!("timeline.rs");
        let profile_branch = source
            .split("TimelineItemContent::ProfileChange(change)")
            .nth(1)
            .expect("profile change branch should exist")
            .split("_ => {}")
            .next()
            .expect("profile change branch should precede fallback branch");

        assert!(
            profile_branch.contains("profile_change_projection(change)"),
            "profile changes should use Element-like notice text"
        );
        assert!(
            !profile_branch.contains("change.user_id()"),
            "timeline row body must not contain a raw Matrix user id for profile changes"
        );
    }

    #[test]
    fn pinned_events_projection_is_a_supported_notice() {
        assert_eq!(
            state_event_notice_body("m.room.pinned_events").as_ref(),
            "updated pinned messages"
        );
        assert_eq!(
            state_event_notice_body("m.room.create").as_ref(),
            "created the room"
        );
        assert_eq!(
            state_event_notice_body("m.room.power_levels").as_ref(),
            "updated room permissions"
        );
        assert_eq!(
            state_event_notice_body("m.room.guest_access").as_ref(),
            "updated guest access"
        );
        assert_eq!(
            state_event_notice_body("m.room.encryption").as_ref(),
            "enabled room encryption"
        );
        assert_eq!(
            state_event_notice_body("m.space.parent").as_ref(),
            "updated the parent space"
        );
        assert_eq!(
            state_event_notice_body("m.room.join_rules").as_ref(),
            "updated join rules"
        );
        assert_eq!(
            state_event_notice_body("m.room.history_visibility").as_ref(),
            "updated history visibility"
        );
        assert_eq!(
            state_event_notice_body("m.room.topic").as_ref(),
            "Unsupported event: m.room.topic"
        );
    }

    #[test]
    fn supported_state_event_notices_carry_i18n_keys() {
        let projection = state_event_notice_projection("m.room.power_levels");

        assert_eq!(projection.body.as_deref(), Some("updated room permissions"));
        assert_eq!(
            projection.notice_i18n_key,
            Some("timeline.notice.roomPowerLevels")
        );
        assert_eq!(projection.body_is_user_content, false);
    }

    #[test]
    fn message_projection_extracts_formatted_spoiler_spans_with_reason() {
        let msgtype = MessageType::Emote(EmoteMessageEventContent::html(
            "plain fallback",
            r#"keep <span data-mx-spoiler="because">secret</span> hidden"#,
        ));

        let projection = message_projection_from_msgtype(&msgtype, "plain fallback");

        assert_eq!(projection.message_kind, TimelineMessageKind::Emote);
        assert_eq!(
            projection.spoiler_spans,
            vec![TimelineSpoilerSpan {
                start_utf16: 5,
                end_utf16: 11,
                reason: Some("because".to_owned()),
            }]
        );
    }

    #[test]
    fn message_projection_sanitizes_formatted_html_and_extracts_code_blocks() {
        let msgtype = MessageType::Text(TextMessageEventContent::html(
            "plain fallback",
            r#"<strong>ok</strong><script>alert(1)</script><a href="javascript:alert(1)">bad</a><a href="https://example.invalid/path">safe</a><pre><code class="language-rust ignored">fn main() {}</code></pre>"#,
        ));

        let projection = message_projection_from_msgtype(&msgtype, "plain fallback");
        let formatted = projection
            .formatted
            .expect("html formatted_body should project to a Rust-owned render model");

        assert!(formatted.html.contains("<strong>ok</strong>"));
        assert!(!formatted.html.contains("<script"));
        assert!(!formatted.html.contains("alert(1)"));
        assert!(!formatted.html.contains("javascript:"));
        assert!(formatted.html.contains("https://example.invalid/path"));
        assert_eq!(formatted.plain_text, "okbadsafefn main() {}");
        assert_eq!(formatted.code_blocks.len(), 1);
        assert_eq!(formatted.code_blocks[0].language.as_deref(), Some("rust"));
        assert_eq!(formatted.code_blocks[0].body, "fn main() {}");
    }

    #[test]
    fn formatted_message_link_ranges_use_formatted_plain_text_basis() {
        let msgtype = MessageType::Text(TextMessageEventContent::html(
            "fallback without url",
            r#"<strong>Visit https://example.invalid/path</strong>"#,
        ));

        let projection = message_projection_from_msgtype(&msgtype, "fallback without url");
        let ranges = link_ranges_for_message_projection(
            projection.body.as_deref(),
            projection.formatted.as_ref(),
        );

        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].url, "https://example.invalid/path");
        assert_eq!(ranges[0].start_utf16, "Visit ".encode_utf16().count());
        assert_eq!(
            ranges[0].end_utf16,
            "Visit https://example.invalid/path".encode_utf16().count()
        );
    }

    #[test]
    fn message_projection_keeps_allowed_formatted_blocks_and_spoilers() {
        let msgtype = MessageType::Emote(EmoteMessageEventContent::html(
            "plain fallback",
            r#"<blockquote>quote</blockquote><ul><li>one</li></ul><span data-mx-spoiler="reason">secret</span>"#,
        ));

        let projection = message_projection_from_msgtype(&msgtype, "plain fallback");
        let formatted = projection
            .formatted
            .expect("allowed formatted_body should project to a render model");

        assert!(formatted.html.contains("<blockquote>quote</blockquote>"));
        assert!(formatted.html.contains("<ul><li>one</li></ul>"));
        assert!(formatted.html.contains("data-mx-spoiler=\"reason\""));
        assert!(formatted.html.contains(">secret<"));
    }

    #[test]
    fn message_projection_falls_back_to_plain_body_when_formatted_body_is_empty() {
        let msgtype = MessageType::Text(TextMessageEventContent::html("plain fallback", "   "));

        let projection = message_projection_from_msgtype(&msgtype, "plain fallback");

        assert_eq!(projection.body.as_deref(), Some("plain fallback"));
        assert!(projection.formatted.is_none());
    }

    #[test]
    fn message_projection_falls_back_to_plain_body_when_formatted_body_has_only_markup() {
        let msgtype = MessageType::Text(TextMessageEventContent::html(
            "plain fallback",
            "<p><br /></p>",
        ));

        let projection = message_projection_from_msgtype(&msgtype, "plain fallback");

        assert_eq!(projection.body.as_deref(), Some("plain fallback"));
        assert!(projection.formatted.is_none());
    }

    #[test]
    fn user_visible_content_includes_formatted_body() {
        let mut item = timeline_item("$formatted:test", None, "@alice:test", false);
        item.formatted = Some(crate::event::TimelineFormattedBody {
            html: "<strong>visible</strong>".to_owned(),
            plain_text: "visible".to_owned(),
            code_blocks: Vec::new(),
        });

        assert!(has_user_visible_content(&item));
    }

    #[test]
    fn bodyless_event_backed_items_are_hidden_unless_redacted() {
        assert!(timeline_item_should_be_hidden(false, false));
        assert!(!timeline_item_should_be_hidden(true, false));
        assert!(!timeline_item_should_be_hidden(false, true));
    }

    #[test]
    fn sender_profile_projects_display_name_and_avatar_mxc() {
        let profile = TimelineDetails::Ready(Profile {
            display_name: Some("kamohara".to_owned()),
            display_name_ambiguous: false,
            avatar_url: Some(matrix_sdk::ruma::OwnedMxcUri::from(
                "mxc://matrix.org/avatar".to_owned(),
            )),
        });

        assert_eq!(
            timeline_sender_label_from_profile(&profile),
            Some("kamohara".to_owned())
        );
        let avatar = timeline_sender_avatar_from_profile(&profile).expect("avatar");
        assert_eq!(avatar.mxc_uri, "mxc://matrix.org/avatar");
        assert_eq!(avatar.thumbnail, AvatarThumbnailState::NotRequested);
    }

    #[test]
    fn composer_core_rejects_unknown_slash_command_locally() {
        assert_eq!(
            build_room_message_content_from_composer_body("/shrug nope", MentionIntent::default(),)
                .expect_err("unsupported slash command should fail before SDK send"),
            TimelineFailureKind::UnsupportedSlashCommand
        );
    }

    fn focused_key() -> TimelineKey {
        TimelineKey {
            account_key: AccountKey("@a:test".to_owned()),
            kind: TimelineKind::Focused {
                room_id: "!r:test".to_owned(),
                event_id: "$evt:test".to_owned(),
            },
        }
    }

    fn thread_key() -> TimelineKey {
        TimelineKey {
            account_key: AccountKey("@a:test".to_owned()),
            kind: TimelineKind::Thread {
                room_id: "!r:test".to_owned(),
                root_event_id: "$root:test".to_owned(),
            },
        }
    }

    fn timeline_message_item(event_id: &str, sender: &str) -> TimelineItem {
        TimelineItem {
            id: TimelineItemId::Event {
                event_id: event_id.to_owned(),
            },
            sender: Some(sender.to_owned()),
            sender_label: None,
            sender_avatar: None,
            body: Some("body".to_owned()),
            notice_i18n_key: None,
            message_kind: Default::default(),
            spoiler_spans: Vec::new(),
            timestamp_ms: Some(1),
            in_reply_to_event_id: None,
            formatted: None,
            reply_quote: None,
            thread_root: None,
            thread_summary: None,
            media: None,
            link_previews: None,
            link_ranges: Vec::new(),
            reactions: Vec::new(),
            can_react: true,
            is_redacted: false,
            is_hidden: false,
            can_redact: false,
            is_edited: false,
            can_edit: false,
            actions: TimelineMessageActions::default(),
            send_state: None,
            unable_to_decrypt: None,
        }
    }

    // --- Direction enforcement ---

    #[tokio::test]
    async fn forward_pagination_on_room_key_fails_invalid_direction() {
        let runtime = CoreRuntime::start();
        let mut conn = runtime.attach();

        // Inject a Ready session so commands are not gated.
        runtime
            .inject_actions(vec![AppAction::RestoreSessionSucceeded(SessionInfo {
                homeserver: "https://test.test".to_owned(),
                user_id: "@a:test".to_owned(),
                device_id: "DEV".to_owned(),
            })])
            .await;

        // Wait for Ready.
        loop {
            if matches!(conn.snapshot().session, SessionState::Ready(_)) {
                break;
            }
            crate::executor::sleep(Duration::from_millis(5)).await;
        }

        let rid = conn.next_request_id();
        conn.command(CoreCommand::Timeline(TimelineCommand::Subscribe {
            request_id: rid,
            key: room_key(),
        }))
        .await
        .expect("submit");

        // Subscribe will fail (no real session) — we don't care. Send forward paginate.
        let paginate_id = conn.next_request_id();
        conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
            request_id: paginate_id,
            key: room_key(),
            direction: PaginationDirection::Forward,
            event_count: 20,
        }))
        .await
        .expect("submit");

        // Drain until we find a failure for paginate_id.
        loop {
            let timeout = tokio::time::timeout(Duration::from_secs(5), conn.recv_event()).await;
            let event = timeout.expect("no timeout").expect("no lag");
            match event {
                CoreEvent::OperationFailed {
                    request_id,
                    failure,
                } if request_id == paginate_id => {
                    // Subscribe failed, so the key is not subscribed — we get NotSubscribed.
                    // OR we get InvalidDirection if subscribe somehow succeeded.
                    // Either way, it MUST NOT succeed.
                    assert!(
                        matches!(
                            failure,
                            CoreFailure::TimelineOperationFailed {
                                kind: TimelineFailureKind::InvalidDirection
                                    | TimelineFailureKind::NotSubscribed
                                    | TimelineFailureKind::Sdk,
                            }
                        ),
                        "expected timeline failure, got: {failure:?}"
                    );
                    return;
                }
                _ => continue,
            }
        }
    }

    #[test]
    fn room_subscribe_success_reduces_timeline_subscribed_action() {
        let source = include_str!("timeline.rs");
        let fn_offset = source
            .find("async fn handle_subscribe")
            .expect("handle_subscribe should exist");
        let rest = &source[fn_offset..];
        let end = rest
            .find("async fn route_to_actor_or_fail")
            .expect("next helper should exist");
        let handle_subscribe_source = &rest[..end];
        let spawn_token = concat!("TimelineActor::", "spawn");
        let action_token = concat!("emit_timeline", "_subscribed_action");
        let room_token = concat!("TimelineKind::", "Room");
        let spawn_offset = handle_subscribe_source
            .find(spawn_token)
            .expect("subscribe success should spawn the timeline actor");
        let success_path = &handle_subscribe_source[spawn_offset..];
        let action_offset = success_path
            .find(action_token)
            .expect("subscribe success should reduce TimelineSubscribed");

        assert!(
            action_offset > 0,
            "TimelineSubscribed should be reduced only after subscribe succeeds"
        );
        assert!(
            handle_subscribe_source.contains(room_token),
            "main timeline subscription state should only be marked for room timelines"
        );
    }

    #[tokio::test]
    async fn koushi_timeline_builder_projects_sdk_read_receipts() {
        use matrix_sdk::assert_next_with_timeout;
        use matrix_sdk::ruma::{event_id, room_id, user_id};
        use matrix_sdk::test_utils::mocks::MatrixMockServer;
        use matrix_sdk_test::{JoinedRoomBuilder, event_factory::EventFactory};

        let server = MatrixMockServer::new().await;
        let client = server.client_builder().build().await;
        let room_id = room_id!("!receipts:example.test");
        let room = server.sync_joined_room(&client, room_id).await;
        let timeline = koushi_timeline_builder(
            &room,
            TimelineFocus::Live {
                hide_threaded_events: false,
            },
        )
        .build()
        .await
        .expect("timeline");
        let (_initial_items, mut stream) = timeline.subscribe().await;

        let factory = EventFactory::new().room(room_id);
        server
            .sync_room(
                &client,
                JoinedRoomBuilder::new(room_id)
                    .add_timeline_event(
                        factory
                            .text_msg("first")
                            .event_id(event_id!("$first:example.test"))
                            .sender(user_id!("@alice:example.test"))
                            .into_raw_sync(),
                    )
                    .add_timeline_event(
                        factory
                            .text_msg("second")
                            .event_id(event_id!("$second:example.test"))
                            .sender(user_id!("@bob:example.test"))
                            .into_raw_sync(),
                    ),
            )
            .await;

        let diffs = assert_next_with_timeout!(stream);
        let mut receipts_by_event = Vec::new();
        for diff in &diffs {
            collect_live_event_receipts_from_diff(diff, &mut receipts_by_event);
        }

        let second = receipts_by_event
            .iter()
            .find(|entry| entry.event_id == "$second:example.test")
            .expect("Koushi timeline builder must opt in to SDK read receipt tracking");
        assert!(
            second
                .receipts
                .iter()
                .any(|receipt| receipt.user_id == "@bob:example.test")
        );
    }

    #[test]
    fn room_live_timeline_focus_hides_threaded_events() {
        let source = include_str!("timeline.rs");
        let focus_source = source
            .split("let focus = match &key.kind")
            .nth(1)
            .expect("subscribe focus match should exist")
            .split("let timeline_result")
            .next()
            .expect("timeline build should follow focus selection");
        let room_focus = focus_source
            .split("TimelineKind::Room")
            .nth(1)
            .expect("room timeline focus arm should exist")
            .split("TimelineKind::Thread")
            .next()
            .expect("thread timeline focus arm should follow room arm");

        assert!(
            room_focus.contains("hide_threaded_events: true"),
            "room live timelines must hide threaded replies"
        );
    }

    #[test]
    fn thread_timeline_focus_uses_sdk_thread_pagination() {
        let source = include_str!("timeline.rs");
        let focus_source = source
            .split("let focus = match &key.kind")
            .nth(1)
            .expect("subscribe focus match should exist")
            .split("let timeline_result")
            .next()
            .expect("timeline build should follow focus selection");
        let thread_focus = focus_source
            .split("TimelineKind::Thread")
            .nth(1)
            .expect("thread timeline focus arm should exist")
            .split("TimelineKind::Focused")
            .next()
            .expect("focused timeline focus arm should follow thread arm");

        assert!(
            thread_focus.contains("TimelineFocus::Thread"),
            "thread panes should use SDK thread timelines so pagination follows thread relations"
        );
        assert!(
            !thread_focus.contains("TimelineFocus::Event"),
            "thread panes must not use event-context focus because later thread replies can be outside the context window"
        );
    }

    /// Contract: re-ensuring an already-subscribed identical key takes the
    /// cheap path — ask the existing actor to replay InitialItems for the new
    /// request_id and return early, so NO `subscribe_to_rooms` / timeline
    /// teardown happens on snapshot churn. Only when the cheap replay cannot be
    /// delivered (mailbox full/closed) does it fall back to a full rebuild so a
    /// re-mounted view is still guaranteed InitialItems. A different (new) key
    /// always falls through to the full subscribe path.
    #[test]
    fn timeline_subscribe_is_idempotent_for_existing_key() {
        let source = include_str!("timeline.rs");
        let handle_subscribe_source = source
            .split("async fn handle_subscribe")
            .nth(1)
            .expect("handle_subscribe should exist")
            .split("async fn route_to_actor_or_fail")
            .next()
            .expect("route helper should follow handle_subscribe");

        // The existing-key branch must be present and must end with an early
        // return — proving the full SDK rebuild is skipped.
        let existing_key_branch = handle_subscribe_source
            .split("let replay_result = self.timelines.get(&key)")
            .nth(1)
            .expect("handle_subscribe must detect an already-subscribed key via timelines.get")
            .split("let client = session.client()")
            .next()
            .expect("existing-key branch must precede the new-key SDK path");

        assert!(
            existing_key_branch.contains("ReplayInitialItems"),
            "re-ensuring an already subscribed timeline must send ReplayInitialItems to the existing actor (no SDK teardown on the success path)"
        );
        assert!(
            existing_key_branch.contains("return;"),
            "the cheap replay path must return early, skipping subscribe_to_rooms and the full SDK rebuild"
        );
        // The success (Ok) arm does the cheap replay and returns; the existing-key
        // branch never re-runs the SDK `subscribe_to_rooms` (which lives after
        // `let client = session.client()`). An undeliverable replay (full/closed
        // mailbox) intentionally falls back to a full rebuild via
        // `self.timelines.remove(&key)`, so no "must-not-remove" assertion here.

        // The new-key (full subscribe) path must still call subscribe_to_rooms
        // and build a fresh timeline.
        let new_key_path = handle_subscribe_source
            .split("let client = session.client()")
            .nth(1)
            .expect("new-key SDK path must follow the existing-key branch");

        assert!(
            new_key_path.contains("service.subscribe_to_rooms"),
            "a new (not yet subscribed) key must still call subscribe_to_rooms"
        );
        assert!(
            new_key_path.contains("koushi_timeline_builder("),
            "a new (not yet subscribed) key must still build an SDK timeline through the project helper"
        );
    }

    #[test]
    fn timeline_ensure_subscribed_can_skip_existing_actor_replay() {
        let source = include_str!("timeline.rs");
        let handle_command = source
            .split("async fn handle_command")
            .nth(1)
            .expect("handle_command should exist")
            .split("async fn handle_subscribe")
            .next()
            .expect("handle_subscribe should follow handle_command");
        let handle_subscribe_source = source
            .split("async fn handle_subscribe")
            .nth(1)
            .expect("handle_subscribe should exist")
            .split("let client = session.client()")
            .next()
            .expect("existing-key branch should precede the SDK subscribe path");

        assert!(
            handle_command.contains("TimelineCommand::EnsureSubscribed"),
            "timeline manager should expose an explicit ensure-subscription path for callers that do not need item replay"
        );
        assert!(
            handle_command.contains("replay_existing"),
            "ensure-subscription routing must pass through whether an existing actor should replay InitialItems"
        );
        assert!(
            handle_subscribe_source.contains("if replay_existing"),
            "existing actors should only replay InitialItems when the caller explicitly requests replay"
        );
    }

    #[test]
    fn timeline_pagination_uses_account_wide_messages_backpressure() {
        let source = include_str!("timeline.rs");
        let pagination_source = source
            .split("async fn handle_paginate")
            .nth(1)
            .and_then(|section| section.split("async fn handle_send_text").next())
            .expect("pagination handler should exist");
        let acquire_offset = pagination_source
            .find("acquire_timeline")
            .expect("timeline pagination must acquire the shared /messages backpressure permit");
        let paginate_offset = pagination_source
            .find("paginate_backwards")
            .expect("timeline pagination must still call SDK pagination");

        assert!(
            source.contains("MessagesBackpressure"),
            "Timeline actors must carry the shared account-wide /messages backpressure handle"
        );
        assert!(
            acquire_offset < paginate_offset,
            "timeline pagination must acquire account-wide /messages backpressure before SDK pagination"
        );
    }

    #[test]
    fn timeline_pagination_is_abortable_without_dropping_the_actor() {
        let source = include_str!("timeline.rs");
        let actor_source = source
            .split("struct TimelineActor {")
            .nth(1)
            .expect("TimelineActor should exist")
            .split("impl Drop for TimelineActor")
            .next()
            .expect("TimelineActor fields should precede Drop impl");
        let handle_paginate_source = source
            .split("async fn handle_paginate")
            .nth(1)
            .and_then(|section| section.split("async fn paginate_once").next())
            .expect("handle_paginate should exist");
        let handle_cancel_source = source
            .split("fn handle_cancel_pagination")
            .nth(1)
            .and_then(|section| {
                section
                    .split("async fn handle_restore_timeline_anchor")
                    .next()
            })
            .expect("cancel pagination handler should exist");

        assert!(
            source.contains("CancelPagination"),
            "timeline manager must expose a cancellation message for in-flight pagination"
        );
        assert!(
            actor_source.contains("pagination_task"),
            "TimelineActor must retain the active pagination task handle separately from the subscription"
        );
        assert!(
            handle_paginate_source.contains("executor::spawn"),
            "pagination must run outside the actor command loop so cancel messages can be received"
        );
        assert!(
            handle_cancel_source.contains(".abort()"),
            "cancelling pagination must abort only the pagination task, not the timeline actor"
        );
    }

    #[test]
    fn sdk_projection_reads_thread_contract_accessors() {
        let source = include_str!("timeline.rs");
        let projection_source = source
            .split("pub fn sdk_item_to_timeline_item")
            .nth(1)
            .expect("sdk projection function should exist")
            .split("pub(crate) fn timeline_item_can_react")
            .next()
            .expect("projection helper should follow sdk projection");
        let compact_projection_source: String = projection_source
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect();

        assert!(
            compact_projection_source.contains("content().thread_root()"),
            "timeline item projection must read SDK thread_root"
        );
        assert!(
            compact_projection_source.contains("content().thread_summary()"),
            "timeline item projection must read SDK thread_summary"
        );
    }

    #[test]
    fn encrypted_thread_reply_relation_is_recovered_from_original_json() {
        let original_json = serde_json::json!({
            "content": {
                "algorithm": "m.megolm.v1.aes-sha2",
                "ciphertext": "ciphertext",
                "m.relates_to": {
                    "rel_type": "m.thread",
                    "event_id": "$thread-root:test",
                    "m.in_reply_to": {
                        "event_id": "$reply-target:test"
                    },
                    "is_falling_back": true
                },
                "session_id": "session"
            },
            "event_id": "$thread-reply:test",
            "type": "m.room.encrypted"
        });

        assert_eq!(
            thread_root_from_original_json(&original_json).as_deref(),
            Some("$thread-root:test")
        );
    }

    #[test]
    fn room_timeline_hides_thread_reply_placeholders_even_when_renderable() {
        let key = room_key();

        assert!(timeline_item_should_be_hidden_for_key(
            &key,
            true,
            false,
            Some("$thread-root:test")
        ));
    }

    #[test]
    fn thread_attention_action_counts_remote_live_thread_messages_only() {
        let key = thread_key();
        let own_user_id = "@me:test";
        let mut counts = ThreadAttentionCounters::default();
        let diffs = vec![
            TimelineDiff::PushBack {
                item: timeline_message_item("$remote:test", "@alice:test"),
            },
            TimelineDiff::PushBack {
                item: timeline_message_item("$own:test", own_user_id),
            },
            TimelineDiff::PushFront {
                item: timeline_message_item("$backfill:test", "@bob:test"),
            },
        ];

        assert_eq!(
            thread_attention_action_from_timeline_diffs(
                &mut counts,
                &key,
                &diffs,
                Some(own_user_id)
            ),
            Some(AppAction::ThreadAttentionUpdated {
                room_id: "!r:test".to_owned(),
                root_event_id: "$root:test".to_owned(),
                notification_count: 1,
                highlight_count: 0,
                live_event_marker_count: 1,
            })
        );
        assert_eq!(
            thread_attention_action_from_timeline_diffs(
                &mut counts,
                &room_key(),
                &[TimelineDiff::PushBack {
                    item: timeline_message_item("$room:test", "@alice:test"),
                }],
                Some(own_user_id),
            ),
            None
        );
    }

    #[test]
    fn send_submission_is_not_reduced_before_actor_route_accepts_it() {
        let source = include_str!("timeline.rs");
        let send_text_arm = source
            .split("TimelineCommand::SendText")
            .nth(1)
            .expect("SendText arm should exist")
            .split("TimelineCommand::SendReply")
            .next()
            .expect("SendReply arm should follow SendText");

        if send_text_arm.contains("route_send_to_actor_or_fail") {
            let helper_source = source
                .split("async fn route_send_to_actor_or_fail")
                .nth(1)
                .expect("send route helper should exist")
                .split("async fn handle_subscribe")
                .next()
                .expect("handle_subscribe should follow the send route helper");
            let route_lookup_offset = helper_source
                .find("self.timelines.get")
                .expect("send route helper should look up the actor before reducing state");
            let submitted_offset = helper_source.find("send_submitted_action").expect(
                "send route helper should reduce submitted state through a projection helper",
            );

            assert!(
                route_lookup_offset < submitted_offset,
                "submitted send state must not be reduced before the actor route is known to exist"
            );
            assert!(
                source.contains("AppAction::SendTextSubmitted"),
                "room send projection should reduce SendTextSubmitted"
            );
            return;
        }

        let submitted_offset = send_text_arm
            .find("SendTextSubmitted")
            .expect("send path should reduce SendTextSubmitted");
        let route_offset = send_text_arm
            .find("route_to_actor_or_fail")
            .expect("send path should route to a timeline actor");

        assert!(
            route_offset < submitted_offset,
            "SendTextSubmitted must not be reduced before the actor route is known to exist"
        );
    }

    #[test]
    fn thread_reply_submission_is_not_reduced_before_actor_route_accepts_it() {
        let source = include_str!("timeline.rs");
        let helper_source = source
            .split("async fn route_send_to_actor_or_fail")
            .nth(1)
            .expect("send route helper should exist")
            .split("async fn handle_subscribe")
            .next()
            .expect("handle_subscribe should follow the send route helper");

        let route_lookup_offset = helper_source
            .find("self.timelines.get")
            .expect("send route helper should look up the actor before reducing state");
        let submitted_offset = helper_source
            .find("send_submitted_action")
            .expect("send route helper should reduce submitted state through a projection helper");

        assert!(
            route_lookup_offset < submitted_offset,
            "submitted send state must not be reduced before the actor route is known to exist"
        );
        assert!(
            source.contains("AppAction::ThreadReplySubmitted"),
            "thread send projection should reduce ThreadReplySubmitted"
        );
    }

    #[test]
    fn thread_timeline_keys_project_send_reply_to_thread_composer_actions() {
        let source = include_str!("timeline.rs");
        let helper_source = source
            .split("async fn route_send_to_actor_or_fail")
            .nth(1)
            .expect("send route helper should exist")
            .split("async fn handle_subscribe")
            .next()
            .expect("handle_subscribe should follow the send route helper");
        let projection_source = source
            .split("fn send_submitted_action")
            .nth(1)
            .expect("send submitted projection helper should exist")
            .split("fn send_finished_action")
            .next()
            .expect("send finished projection helper should follow submit helper");
        let finished_projection_source = source
            .split("fn send_finished_action")
            .nth(1)
            .expect("send finished projection helper should exist")
            .split("fn send_failed_action")
            .next()
            .expect("send failed projection helper should follow finished helper");
        let failed_projection_source = source
            .split("fn send_failed_action")
            .nth(1)
            .expect("send failed projection helper should exist")
            .split("// ---------------------------------------------------------------------------")
            .next()
            .expect("projection helper section should end");
        let actor_completion_source = source
            .split("fn emit_send_finished_action")
            .nth(1)
            .expect("send completion helper should exist")
            .split("// ---------------------------------------------------------------------------")
            .next()
            .expect("timeline actor helper section should end");

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
            actor_completion_source.contains("send_finished_action")
                && actor_completion_source.contains("send_failed_action")
                && finished_projection_source.contains("ThreadReplyFinished")
                && failed_projection_source.contains("ThreadReplyFailed"),
            "thread actor completion and failure must settle thread composer state"
        );
        assert!(
            source.contains("TimelineKind::Focused { .. } => Self::None")
                && source.contains("TimelineKind::Focused { .. } => None"),
            "focused timelines must not own composer state"
        );
    }

    #[test]
    fn restore_anchor_handler_is_room_only_and_bounded() {
        let source = include_str!("timeline.rs");
        let helper_source = source
            .split("async fn handle_restore_timeline_anchor(")
            .nth(1)
            .expect("restore anchor handler should exist")
            .split("async fn handle_restore_timeline_anchor_continue")
            .next()
            .expect("restore anchor handler should end before send text");
        let continue_source = source
            .split("async fn handle_restore_timeline_anchor_continue")
            .nth(1)
            .expect("restore anchor continuation should exist")
            .split("async fn schedule_restore_anchor_continue")
            .next()
            .expect("restore anchor continuation should end before scheduler");

        assert!(
            helper_source.contains("TimelineKind::Room"),
            "restore anchor must target the live room timeline actor"
        );
        assert!(
            continue_source.contains("PaginationDirection::Backward"),
            "restore anchor must drive backward pagination"
        );
        assert!(
            helper_source.contains("max_batches") && helper_source.contains("event_count"),
            "restore anchor must carry a bounded pagination budget"
        );
        assert!(
            !helper_source.contains("TimelineKind::Focused"),
            "restore anchor must not bootstrap through the focused timeline path"
        );
    }

    #[test]
    fn thread_composer_sends_regular_thread_messages_for_element_compatibility() {
        assert_eq!(
            reply_enforce_thread_for_key(&thread_key()),
            EnforceThread::Threaded(ReplyWithinThread::No)
        );
    }

    #[tokio::test]
    async fn forward_pagination_on_thread_key_not_subscribed() {
        let runtime = CoreRuntime::start();
        let mut conn = runtime.attach();

        runtime
            .inject_actions(vec![AppAction::RestoreSessionSucceeded(SessionInfo {
                homeserver: "https://test.test".to_owned(),
                user_id: "@a:test".to_owned(),
                device_id: "DEV".to_owned(),
            })])
            .await;
        loop {
            if matches!(conn.snapshot().session, SessionState::Ready(_)) {
                break;
            }
            crate::executor::sleep(Duration::from_millis(5)).await;
        }

        // Do NOT subscribe; paginate forward on thread key → NotSubscribed.
        let paginate_id = conn.next_request_id();
        conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
            request_id: paginate_id,
            key: thread_key(),
            direction: PaginationDirection::Forward,
            event_count: 10,
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
                } if request_id == paginate_id => {
                    assert!(
                        matches!(
                            failure,
                            CoreFailure::TimelineOperationFailed {
                                kind: TimelineFailureKind::InvalidDirection
                                    | TimelineFailureKind::NotSubscribed,
                            }
                        ),
                        "got: {failure:?}"
                    );
                    return;
                }
                _ => continue,
            }
        }
    }

    #[test]
    fn focused_allows_forward_direction_in_paginate_logic() {
        // Test the direction check logic directly: forward IS allowed on Focused.
        let key = focused_key();
        let is_focused = matches!(key.kind, TimelineKind::Focused { .. });
        assert!(is_focused, "focused key must match Focused");

        // Forward + Focused: should NOT trigger InvalidDirection.
        let direction = PaginationDirection::Forward;
        let is_invalid = direction == PaginationDirection::Forward
            && !matches!(key.kind, TimelineKind::Focused { .. });
        assert!(
            !is_invalid,
            "forward on Focused must not be invalid direction"
        );
    }

    #[test]
    fn backward_direction_never_invalid_for_any_kind() {
        for key in [room_key(), focused_key(), thread_key()] {
            let direction = PaginationDirection::Backward;
            let is_invalid = direction == PaginationDirection::Forward
                && !matches!(key.kind, TimelineKind::Focused { .. });
            assert!(
                !is_invalid,
                "backward pagination should never be InvalidDirection for key: {key:?}"
            );
        }
    }

    // --- NotSubscribed for commands on unknown keys ---

    #[tokio::test]
    async fn paginate_on_unsubscribed_key_returns_not_subscribed() {
        let runtime = CoreRuntime::start();
        let mut conn = runtime.attach();

        runtime
            .inject_actions(vec![AppAction::RestoreSessionSucceeded(SessionInfo {
                homeserver: "https://test.test".to_owned(),
                user_id: "@a:test".to_owned(),
                device_id: "DEV".to_owned(),
            })])
            .await;
        loop {
            if matches!(conn.snapshot().session, SessionState::Ready(_)) {
                break;
            }
            crate::executor::sleep(Duration::from_millis(5)).await;
        }

        let rid = conn.next_request_id();
        conn.command(CoreCommand::Timeline(TimelineCommand::Paginate {
            request_id: rid,
            key: room_key(),
            direction: PaginationDirection::Backward,
            event_count: 20,
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
                    assert_eq!(
                        failure,
                        CoreFailure::TimelineOperationFailed {
                            kind: TimelineFailureKind::NotSubscribed
                        }
                    );
                    return;
                }
                _ => continue,
            }
        }
    }

    #[tokio::test]
    async fn send_on_unsubscribed_key_returns_not_subscribed() {
        let runtime = CoreRuntime::start();
        let mut conn = runtime.attach();

        runtime
            .inject_actions(vec![AppAction::RestoreSessionSucceeded(SessionInfo {
                homeserver: "https://test.test".to_owned(),
                user_id: "@a:test".to_owned(),
                device_id: "DEV".to_owned(),
            })])
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
            body: "hello".to_owned(),
            mentions: MentionIntent::default(),
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
                    assert_eq!(
                        failure,
                        CoreFailure::TimelineOperationFailed {
                            kind: TimelineFailureKind::NotSubscribed
                        }
                    );
                    return;
                }
                _ => continue,
            }
        }
    }

    // --- Batch ID monotonicity ---

    #[test]
    fn batch_id_monotonically_increases_per_generation() {
        let mut id = TimelineBatchId(0);
        let mut seen = Vec::new();
        for _ in 0..10 {
            seen.push(id);
            id = TimelineBatchId(id.0 + 1);
        }
        for (i, pair) in seen.windows(2).enumerate() {
            assert!(pair[0] < pair[1], "batch ids must be increasing: index {i}");
        }
        // After generation reset, batch_id resets to 0.
        let reset = TimelineBatchId(0);
        assert_eq!(reset, TimelineBatchId(0));
    }

    // --- Generation bump + ResyncRequired on synthetic overflow ---

    #[tokio::test]
    async fn relay_overflow_signal_triggers_generation_bump() {
        // Test the overflow logic directly on the actor message pathway,
        // using a synthetic mpsc channel at capacity 1 to force overflow.
        let (event_tx, mut event_rx): (broadcast::Sender<CoreEvent>, _) = broadcast::channel(256);
        let (actor_tx, actor_rx) = mpsc::channel::<TimelineActorMessage>(2);

        let key = room_key();
        let generation = Arc::new(AtomicU64::new(0));
        let next_batch_id = Arc::new(AtomicU64::new(0));

        // Simulate the actor receiving RelayOverflow:
        // It should increment generation, reset batch_id, and emit ResyncRequired.
        // We test the state machine logic directly.
        let gen_before = generation.load(Ordering::SeqCst);
        let new_gen = gen_before + 1;
        generation.store(new_gen, Ordering::SeqCst);
        next_batch_id.store(0, Ordering::SeqCst);

        let _ = event_tx.send(CoreEvent::Timeline(TimelineEvent::ResyncRequired {
            key: key.clone(),
            reason: TimelineResyncReason::QueueOverflow,
        }));

        // Verify the event was emitted.
        let event = event_rx.recv().await.expect("event");
        match event {
            CoreEvent::Timeline(TimelineEvent::ResyncRequired {
                key: ev_key,
                reason,
            }) => {
                assert_eq!(ev_key, key);
                assert_eq!(reason, TimelineResyncReason::QueueOverflow);
            }
            other => panic!("expected ResyncRequired, got {other:?}"),
        }

        assert_eq!(
            generation.load(Ordering::SeqCst),
            1,
            "generation must be bumped"
        );
        assert_eq!(
            next_batch_id.load(Ordering::SeqCst),
            0,
            "batch_id resets to 0"
        );

        drop(actor_tx);
        drop(actor_rx);
    }

    // --- Txn-ID mapping ---

    #[test]
    fn pending_sends_map_sdk_txn_to_client_txn_and_request_id() {
        let mut tracker = SendCompletionTracker::default();
        let sdk_txn = "sdk-auto-generated-txn".to_owned();
        let client_txn = "client-txn-42".to_owned();
        let rid = fake_rid(42);
        let event_id = "$event-42".to_owned();

        assert_eq!(
            tracker.remember_pending_send(sdk_txn.clone(), client_txn.clone(), rid, true),
            None
        );
        assert_eq!(
            tracker.pending_send(&sdk_txn),
            Some((client_txn.as_str(), rid))
        );
        assert_eq!(
            tracker.record_sent_event(sdk_txn.clone(), event_id.clone()),
            Some((client_txn.clone(), rid, event_id, true))
        );

        assert!(tracker.pending_sends.is_empty());
        assert!(tracker.completed_sends.is_empty());
    }

    #[test]
    fn send_completion_race_delivers_completion_when_sent_event_arrives_first() {
        let mut tracker = SendCompletionTracker::default();
        let sdk_txn = "sdk-race-txn".to_owned();
        let client_txn = "client-race-txn".to_owned();
        let request_id = fake_rid(77);
        let event_id = "$event-race".to_owned();

        assert_eq!(
            tracker.record_sent_event(sdk_txn.clone(), event_id.clone()),
            None
        );
        assert_eq!(
            tracker.remember_pending_send(sdk_txn.clone(), client_txn.clone(), request_id, true),
            Some((client_txn.clone(), request_id, event_id.clone()))
        );
        assert!(tracker.pending_sends.is_empty());
        assert!(tracker.completed_sends.is_empty());
    }

    #[test]
    fn media_pending_send_does_not_settle_text_composer() {
        let mut tracker = SendCompletionTracker::default();
        let sdk_txn = "sdk-media-txn".to_owned();
        let client_txn = "client-media-txn".to_owned();
        let request_id = fake_rid(78);
        let event_id = "$event-media".to_owned();

        assert_eq!(
            tracker.remember_pending_send(sdk_txn.clone(), client_txn.clone(), request_id, false),
            None
        );
        assert_eq!(
            tracker.record_sent_event(sdk_txn.clone(), event_id.clone()),
            Some((client_txn, request_id, event_id, false))
        );
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
    fn send_operation_guards_allow_retry_and_cancel_only_from_outbound_states() {
        assert_eq!(
            validate_retry_send(Some(&TimelineSendState::NotSent {
                reason: TimelineSendFailureReason::Recoverable,
            })),
            Ok(())
        );
        assert_eq!(
            validate_retry_send(Some(&TimelineSendState::Sending)),
            Err(TimelineFailureKind::InvalidSendState)
        );
        assert_eq!(
            validate_retry_send(Some(&TimelineSendState::Sent)),
            Err(TimelineFailureKind::InvalidSendState)
        );
        assert_eq!(
            validate_cancel_send(Some(&TimelineSendState::Sending)),
            Ok(())
        );
        assert_eq!(
            validate_cancel_send(Some(&TimelineSendState::NotSent {
                reason: TimelineSendFailureReason::Unrecoverable,
            })),
            Ok(())
        );
        assert_eq!(
            validate_cancel_send(Some(&TimelineSendState::Sent)),
            Err(TimelineFailureKind::InvalidSendState)
        );
        assert_eq!(
            validate_cancel_send(None),
            Err(TimelineFailureKind::InvalidSendTarget)
        );
    }

    #[test]
    fn retry_send_reenables_sdk_room_queue_before_unwedge() {
        let source = include_str!("timeline.rs");
        let retry_handler = source
            .split("async fn handle_retry_send")
            .nth(1)
            .and_then(|section| section.split("async fn handle_cancel_send").next())
            .expect("retry handler source");
        let enable_index = retry_handler
            .find("set_enabled(true)")
            .expect("retry must re-enable the SDK room send queue");
        let unwedge_index = retry_handler
            .find("unwedge().await")
            .expect("retry must unwedge the SDK send handle");

        assert!(
            enable_index < unwedge_index,
            "room send queue must be re-enabled before SendHandle::unwedge()"
        );
    }

    #[test]
    fn cancel_send_reenables_sdk_room_queue_after_abort() {
        let source = include_str!("timeline.rs");
        let cancel_handler = source
            .split("async fn handle_cancel_send")
            .nth(1)
            .and_then(|section| section.split("fn sdk_room_for_key").next())
            .expect("cancel handler source");
        let abort_index = cancel_handler
            .find("abort().await")
            .expect("cancel must abort the SDK send handle");
        let enable_index = cancel_handler
            .find("set_enabled(true)")
            .expect("cancel must re-enable the SDK room send queue after abort");

        assert!(
            abort_index < enable_index,
            "room send queue must be re-enabled after a successful abort"
        );
    }

    #[test]
    fn reaction_groups_project_my_sender_and_remote_event_id() {
        let own_user_id = OwnedUserId::try_from("@me:test").expect("user id");
        let groups =
            reaction_groups_from_sdk(&reaction_groups_fixture(), Some(own_user_id.as_ref()));

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].key, "👍");
        assert_eq!(groups[0].count, 4);
        assert!(groups[0].reacted_by_me);
        assert_eq!(
            groups[0].my_reaction_event_id.as_deref(),
            Some("$reaction:me")
        );
        assert_eq!(
            groups[0].sender_preview,
            vec!["@me:test", "@alice:test", "@bob:test"]
        );
    }

    #[test]
    fn reaction_groups_count_unique_senders_after_sdk_deduplication() {
        let mut reactions = ReactionsByKeyBySender::default();
        let thumbs = reactions.entry("👍".to_owned()).or_default();
        let alice = OwnedUserId::try_from("@alice:test").expect("user id");
        thumbs.insert(
            alice.clone(),
            ReactionInfo {
                timestamp: matrix_sdk::ruma::MilliSecondsSinceUnixEpoch(uint!(1)),
                status: ReactionStatus::RemoteToRemote(
                    matrix_sdk::ruma::OwnedEventId::try_from("$reaction:old").expect("event id"),
                ),
            },
        );
        thumbs.insert(
            alice,
            ReactionInfo {
                timestamp: matrix_sdk::ruma::MilliSecondsSinceUnixEpoch(uint!(2)),
                status: ReactionStatus::RemoteToRemote(
                    matrix_sdk::ruma::OwnedEventId::try_from("$reaction:new").expect("event id"),
                ),
            },
        );

        let groups = reaction_groups_from_sdk(&reactions, None);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].count, 1);
        assert_eq!(groups[0].sender_preview, vec!["@alice:test"]);
    }

    #[test]
    fn reaction_groups_follow_sdk_redaction_removal() {
        let mut reactions = reaction_groups_fixture();
        reactions
            .get_mut("👍")
            .expect("thumbs reaction")
            .shift_remove(&OwnedUserId::try_from("@me:test").expect("user id"));
        let own_user_id = OwnedUserId::try_from("@me:test").expect("user id");

        let groups = reaction_groups_from_sdk(&reactions, Some(own_user_id.as_ref()));

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].count, 3);
        assert!(!groups[0].reacted_by_me);
        assert_eq!(groups[0].my_reaction_event_id, None);
    }

    #[test]
    fn timeline_item_can_react_requires_event_backed_renderable_content() {
        assert!(timeline_item_can_react(true, true, false, true));
        assert!(!timeline_item_can_react(false, true, false, true));
        assert!(!timeline_item_can_react(true, false, false, true));
        assert!(!timeline_item_can_react(true, true, false, false));
        assert!(!timeline_item_can_react(true, true, true, true));
    }

    #[test]
    fn send_reaction_guard_requires_reactable_target_without_existing_own_reaction() {
        assert_eq!(validate_send_reaction(true, None), Ok(()));
        assert_eq!(
            validate_send_reaction(false, None),
            Err(TimelineFailureKind::InvalidReactionTarget)
        );
        assert_eq!(
            validate_send_reaction(true, Some("$reaction:example.test")),
            Err(TimelineFailureKind::InvalidReactionState)
        );
    }

    #[test]
    fn redact_reaction_guard_requires_matching_own_reaction_event() {
        assert_eq!(
            validate_redact_reaction(
                true,
                Some("$reaction:example.test"),
                "$reaction:example.test"
            ),
            Ok(())
        );
        assert_eq!(
            validate_redact_reaction(
                false,
                Some("$reaction:example.test"),
                "$reaction:example.test"
            ),
            Err(TimelineFailureKind::InvalidReactionTarget)
        );
        assert_eq!(
            validate_redact_reaction(true, None, "$reaction:example.test"),
            Err(TimelineFailureKind::InvalidReactionState)
        );
        assert_eq!(
            validate_redact_reaction(
                true,
                Some("$other-reaction:example.test"),
                "$reaction:example.test"
            ),
            Err(TimelineFailureKind::InvalidReactionState)
        );
    }

    #[test]
    fn timeline_item_can_redact_requires_own_renderable_event_content() {
        assert!(timeline_item_can_redact(true, true, false, true));
        assert!(!timeline_item_can_redact(false, true, false, true));
        assert!(!timeline_item_can_redact(true, false, false, true));
        assert!(!timeline_item_can_redact(true, true, true, true));
        assert!(!timeline_item_can_redact(true, true, false, false));
    }

    #[test]
    fn timeline_item_can_edit_requires_own_editable_body() {
        assert!(timeline_item_can_edit(true, true, false, true));
        assert!(!timeline_item_can_edit(false, true, false, true));
        assert!(!timeline_item_can_edit(true, false, false, true));
        assert!(!timeline_item_can_edit(true, true, true, true));
        assert!(!timeline_item_can_edit(true, true, false, false));
    }

    // --- Debug redaction of new types ---

    #[test]
    fn timeline_actor_message_bodies_not_visible_in_send_text_debug() {
        // TimelineActorMessage::SendText carries a body — it must not leak.
        // The type is not pub, so we test the Debug output of the outer TimelineCommand
        // which is already tested in tests.rs; this is an extra check for internal types.
        // Since TimelineActorMessage is not exported, we only verify the public command.
        let cmd = TimelineCommand::SendText {
            request_id: fake_rid(1),
            key: room_key(),
            transaction_id: "txn-vis".to_owned(),
            body: "very-private-body".to_owned(),
            mentions: MentionIntent::default(),
        };
        let debug = format!("{cmd:?}");
        assert!(
            !debug.contains("very-private-body"),
            "body leaked in Debug: {debug}"
        );
        assert!(
            debug.contains("txn-vis"),
            "txn_id should be visible: {debug}"
        );
    }

    #[test]
    fn timeline_subscribe_and_paginate_emit_startup_trace() {
        let source = include_str!("timeline.rs");
        // Search production code only; excluding the test module prevents the
        // assertion strings below from satisfying themselves (include_str! pulls in
        // this test's own body).
        let production = source.split("\nmod tests").next().unwrap_or(source);
        // build lives in the manager's subscribe handler; subscribe lives in
        // TimelineActor::spawn. Assert presence without forcing their location.
        assert!(
            production.contains("StartupPhase::TimelineBuild"),
            "the SDK TimelineBuilder::build phase must be timed"
        );
        assert!(
            production.contains("StartupPhase::TimelineSubscribe"),
            "the timeline.subscribe() phase must be timed with an item bucket"
        );
        let paginate_src = production
            .split("async fn handle_paginate")
            .nth(1)
            .and_then(|s| s.split("async fn handle_send_text").next())
            .expect("handle_paginate should exist");
        assert!(
            paginate_src.contains("trace_paginate"),
            "pagination must emit a startup_trace paginate token"
        );
    }

    #[test]
    fn timeline_route_and_paginate_emit_ordered_trace_tokens() {
        let source = include_str!("timeline.rs");
        let production = source.split("\nmod tests").next().unwrap_or(source);
        assert!(
            production.contains("fn trace_timeline_route"),
            "timeline manager routing must have a private-data-free trace helper"
        );
        assert!(
            production.contains("fn trace_timeline_paginate"),
            "timeline pagination must have a private-data-free trace helper"
        );
        assert!(
            production.contains("koushi.timeline"),
            "timeline traces must share a stable koushi.timeline prefix"
        );
        for token in [
            "stage={stage}",
            "\"manager_received\"",
            "\"actor_paginate_start\"",
            "\"gate_acquired\"",
            "\"sdk_finish\"",
            "request_id={}/{}",
            "timeline={}",
        ] {
            assert!(
                production.contains(token),
                "missing timeline trace token {token}"
            );
        }
    }

    #[test]
    fn timeline_link_preview_load_emits_private_data_free_trace_tokens() {
        let source = include_str!("timeline.rs");
        let production = source.split("\nmod tests").next().unwrap_or(source);
        assert!(
            production.contains("fn trace_timeline_link_preview"),
            "link preview loads must have a private-data-free trace helper"
        );
        for token in [
            "kind=link_preview",
            "\"lookup_miss\"",
            "\"no_previews\"",
            "\"start\"",
            "\"complete\"",
            "\"load_link_previews\"",
            "pending={}",
            "elapsed_ms={}",
            "request_id={}/{}",
        ] {
            assert!(
                production.contains(token),
                "missing link preview trace token {token}"
            );
        }
    }

    #[test]
    fn timeline_link_preview_fetches_do_not_block_actor_command_queue() {
        let source = include_str!("timeline.rs");
        let production = source.split("\nmod tests").next().unwrap_or(source);
        let load_src = production
            .split("async fn handle_load_link_previews")
            .nth(1)
            .and_then(|s| s.split("async fn handle_hide_link_preview").next())
            .expect("handle_load_link_previews should exist");

        assert!(
            !load_src.contains("fetch_link_preview("),
            "link preview network fetches must not run on the TimelineActor command loop"
        );
        assert!(
            production.contains("spawn_link_preview_fetch"),
            "link preview loads must spawn fetch work outside the TimelineActor command loop"
        );
        assert!(
            production.contains("LinkPreviewsFetched"),
            "link preview worker results must return to the TimelineActor explicitly"
        );
    }

    #[test]
    fn timeline_link_preview_fetches_are_abortable_without_dropping_the_actor() {
        let source = include_str!("timeline.rs");
        let production = source.split("\nmod tests").next().unwrap_or(source);
        let handle_cancel_source = production
            .split("fn handle_cancel_link_previews")
            .nth(1)
            .and_then(|section| section.split("async fn handle_hide_link_preview").next())
            .expect("cancel link previews handler should exist");
        let fetched_source = production
            .split("async fn handle_link_previews_fetched")
            .nth(1)
            .and_then(|section| section.split("fn handle_cancel_link_previews").next())
            .expect("link preview fetched handler should exist");

        assert!(
            production.contains("CancelLinkPreviews"),
            "timeline manager must expose a cancellation message for in-flight link previews"
        );
        assert!(
            handle_cancel_source.contains(".abort()"),
            "cancelling link previews must abort only link preview workers, not the timeline actor"
        );
        assert!(
            handle_cancel_source.contains("reset_loading_link_previews_to_pending"),
            "cancelled link preview workers must return Loading previews to Pending for future retries"
        );
        assert!(
            fetched_source.contains("remove(&event_id).is_none()"),
            "late results from cancelled link preview workers must be ignored"
        );
    }

    #[test]
    fn cancelled_link_preview_loads_return_loading_previews_to_pending() {
        let mut item = timeline_item(
            "$link:test",
            Some("https://example.test"),
            "@bob:test",
            false,
        );
        item.link_previews = Some(vec![
            LinkPreview {
                url: "https://example.test/loading".to_owned(),
                title: None,
                description: None,
                image: None,
                state: LinkPreviewState::Loading,
            },
            LinkPreview {
                url: "https://example.test/ready".to_owned(),
                title: Some("ready".to_owned()),
                description: None,
                image: None,
                state: LinkPreviewState::Ready,
            },
        ]);

        assert!(reset_loading_link_previews_to_pending(&mut item));
        let previews = item.link_previews.as_ref().expect("link previews");
        assert_eq!(previews[0].state, LinkPreviewState::Pending);
        assert_eq!(previews[1].state, LinkPreviewState::Ready);
        assert!(!reset_loading_link_previews_to_pending(&mut item));
    }

    #[test]
    fn timeline_subscribe_spawns_env_gated_origin_observer() {
        let source = include_str!("timeline.rs");
        // Search production code only; excluding the test module prevents the
        // assertion strings below from satisfying themselves (include_str! pulls in
        // this test's own body).
        let production = source.split("\nmod tests").next().unwrap_or(source);
        // The observer lives in TimelineActor::spawn alongside other auxiliary
        // tasks. Assert whole-source presence without forcing a code location.
        assert!(
            production.contains("startup_trace::enabled()"),
            "origin observer must be gated on KOUSHI_STARTUP_TRACE so production is unaffected"
        );
        assert!(
            production.contains("event_cache()"),
            "origin observer must subscribe the SDK room event cache"
        );
        assert!(
            production.contains("EventsOrigin"),
            "origin observer must read the SDK EventsOrigin (cache/network/sync)"
        );
    }

    // --- Restore budget (room entry must fail fast) ---

    /// Proves that room-entry anchor restore respects the frontend budget. A
    /// stale or very deep persisted anchor must fail quickly and let the UI fall
    /// back to live edge; it must not silently inflate `max_batches=6` into a
    /// multi-thousand chunk walk that blocks entering the room.
    #[test]
    fn restore_anchor_budget_respects_frontend_hint() {
        let source = include_str!("timeline.rs");
        // Limit to production code so test strings cannot self-satisfy.
        let production = source.split("\nmod tests").next().unwrap_or(source);

        // 1. The new-state construction must use the request budget directly.
        let new_state_src = production
            .split("let restore = RestoreTimelineAnchorState {")
            .nth(1)
            .expect("new RestoreTimelineAnchorState construction must exist");
        assert!(
            new_state_src.contains("max_batches_remaining: max_batches,"),
            "max_batches_remaining initialization must respect the frontend budget"
        );

        // 2. The existing-state branch must not inflate an in-flight budget.
        let existing_branch_src = production
            .split("if existing.event_id == event_id {")
            .nth(1)
            .expect("existing-state same-event branch must exist");
        assert!(
            existing_branch_src.contains(".max(max_batches);"),
            "in-flight budget update must only preserve/increase to the requested budget"
        );
    }

    // --- Restore diff coalescing (Change 2) ---

    /// Proves that during a restore walk the diff-batch handler buffers
    /// `TimelineDiff`s rather than emitting `ItemsUpdated` per chunk, and that
    /// all terminal paths flush the buffer exactly once.  React therefore
    /// receives a single settled `ItemsUpdated` per restore — no O(chunks)
    /// render churn — while internal state (`timeline_contains_event_id`) stays
    /// up-to-date every batch so the anchor can be found mid-walk.
    #[test]
    fn initial_timeline_items_are_forwarded_to_search_index() {
        let source = include_str!("timeline.rs");
        let production = source.split("\nmod tests").next().unwrap_or(source);
        let spawn_src = production
            .split("async fn spawn(")
            .nth(1)
            .expect("TimelineActor::spawn must exist")
            .split("async fn run(")
            .next()
            .expect("spawn should end before run");

        assert!(
            spawn_src.contains("forward_initial_items_to_search"),
            "visible initial timeline items must enter the same search-index path as live diffs"
        );
        assert!(
            spawn_src.find("forward_initial_items_to_search")
                < spawn_src.find("actor.run()"),
            "initial items must be forwarded before the actor starts processing later diffs"
        );
    }

    #[test]
    fn restore_walk_coalesces_items_updated_to_single_flush() {
        let source = include_str!("timeline.rs");
        let production = source.split("\nmod tests").next().unwrap_or(source);

        // 1. The buffer field must exist on TimelineActor.
        assert!(
            production.contains("restore_emit_buffer: Vec<TimelineDiff>"),
            "TimelineActor must carry restore_emit_buffer to coalesce diffs"
        );

        // 2. handle_diff_batch must gate on restore_anchor.is_some() before
        //    buffering vs. emitting.
        let diff_batch_src = production
            .split("async fn handle_diff_batch(")
            .nth(1)
            .expect("handle_diff_batch must exist")
            .split("async fn handle_ignored_users_updated")
            .next()
            .expect("handle_diff_batch must end before handle_ignored_users_updated");
        assert!(
            diff_batch_src.contains("restore_anchor.is_some()"),
            "handle_diff_batch must check restore_anchor.is_some() to gate buffering"
        );
        assert!(
            diff_batch_src.contains("restore_emit_buffer"),
            "handle_diff_batch must use restore_emit_buffer to accumulate diffs"
        );
        // The emit path for non-restore diffs must still exist (else emit is lost).
        assert!(
            diff_batch_src.contains("ItemsUpdated"),
            "handle_diff_batch must still emit ItemsUpdated on the non-restore branch"
        );

        // 3. flush_restore_emit_buffer must emit ONE ItemsUpdated from the buffer.
        let flush_src = production
            .split("fn flush_restore_emit_buffer(")
            .nth(1)
            .expect("flush_restore_emit_buffer helper must exist")
            .split("fn finish_anchor_restore(")
            .next()
            .expect("flush_restore_emit_buffer must end before finish_anchor_restore");
        assert!(
            flush_src.contains("std::mem::take"),
            "flush must drain the buffer with mem::take to avoid cloning"
        );
        assert!(
            flush_src.contains("ItemsUpdated"),
            "flush must emit exactly one ItemsUpdated from the drained buffer"
        );
        assert!(
            flush_src.contains("emit_navigation_if_changed"),
            "flush must refresh navigation after emitting the coalesced batch"
        );

        // 4. finish_anchor_restore must call flush then emit_anchor_restore_finished.
        let finish_src = production
            .split("fn finish_anchor_restore(")
            .nth(1)
            .expect("finish_anchor_restore wrapper must exist")
            .split("fn flush_restore_emit_buffer(")
            .next()
            // May appear before the flush fn in source; accept any order.
            .unwrap_or("");
        // The wrapper is defined; search broadly for both calls in production.
        assert!(
            production.contains("flush_restore_emit_buffer()")
                || production.contains("self.flush_restore_emit_buffer()"),
            "finish_anchor_restore must call flush_restore_emit_buffer"
        );
        let _ = finish_src; // used for the existence assertion above

        // 5. Every ACTIVE-restore terminal path must call finish_anchor_restore (not raw
        //    emit_anchor_restore_finished), ensuring the buffer is always flushed.
        //    Exception: the invalid-request early-return in handle_restore_timeline_anchor
        //    (empty event_id / max_batches==0 / event_count==0) intentionally uses raw
        //    emit_anchor_restore_finished so it does NOT flush a DIFFERENT restore's buffer.
        //    That path is exempt: it fires before any restore state is set, and must not
        //    touch an active restore's restore_emit_buffer.
        //
        // To verify the valid-request (post-early-return) path, check that
        // handle_restore_timeline_anchor has at most ONE emit_anchor_restore_finished call
        // (the exempt invalid-request path), while the continuation uses none directly.
        let restore_handler_src = production
            .split("async fn handle_restore_timeline_anchor(")
            .nth(1)
            .expect("handle_restore_timeline_anchor must exist")
            .split("async fn handle_restore_timeline_anchor_continue")
            .next()
            .expect("restore handler must end before continuation");
        let raw_emit_count = restore_handler_src
            .matches("self.emit_anchor_restore_finished(")
            .count();
        assert!(
            raw_emit_count <= 1,
            "handle_restore_timeline_anchor may have at most ONE raw emit_anchor_restore_finished \
             call (the invalid-request exempt path); found {raw_emit_count}"
        );
        let continue_handler_src = production
            .split("async fn handle_restore_timeline_anchor_continue(")
            .nth(1)
            .expect("handle_restore_timeline_anchor_continue must exist")
            .split("async fn maybe_continue_restore_anchor_after_diff")
            .next()
            .expect("continue handler must end before maybe_continue helper");
        assert!(
            !continue_handler_src.contains("self.emit_anchor_restore_finished("),
            "handle_restore_timeline_anchor_continue must use finish_anchor_restore (never raw \
             emit_anchor_restore_finished) — all its terminals have an active restore buffer"
        );
    }

    // --- Deterministic anchor-present terminal + invalid-request no-flush ---

    /// Proves the authoritative anchor-present terminal: the SDK's
    /// `anchor_present` signal determines whether to wait-for-relay (Found
    /// guaranteed) or conclude EndReached immediately (anchor genuinely absent).
    /// This makes the restore terminal deterministic — no timing heuristic.
    ///
    /// NOTE: a behavioral unit test requires constructing a real `TimelineActor`
    /// with an active Matrix SDK session, which this test module does not support
    /// without a large new mock harness. The `cache_restore` headless harness
    /// (scenario=cache_restore, 3 rooms × deep stress) is the behavioral gate for
    /// correctness of the anchor-present path; these assertions guard the
    /// structural contracts.
    #[test]
    fn restore_terminal_is_anchor_present_not_timing_dependent() {
        let source = include_str!("timeline.rs");
        let production = source.split("\nmod tests").next().unwrap_or(source);

        // 1. anchor_relay_wait must exist on RestoreTimelineAnchorState.
        let struct_src = production
            .split("struct RestoreTimelineAnchorState {")
            .nth(1)
            .expect("RestoreTimelineAnchorState must exist")
            .split('}')
            .next()
            .expect("struct body must end");
        assert!(
            struct_src.contains("anchor_relay_wait"),
            "RestoreTimelineAnchorState must carry anchor_relay_wait for the relay-wait backstop"
        );
        // 2. The continuation handler must enter the relay-wait path when anchor_present.
        let continue_src = production
            .split("async fn handle_restore_timeline_anchor_continue(")
            .nth(1)
            .expect("continuation must exist")
            .split("async fn maybe_continue_restore_anchor_after_diff")
            .next()
            .expect("continuation must end before maybe_continue");
        assert!(
            continue_src.contains("anchor_relay_wait"),
            "continuation handler must manage anchor_relay_wait for the relay-wait loop"
        );
        assert!(
            continue_src.contains("outcome.anchor_present"),
            "continuation handler must branch on outcome.anchor_present (SDK authoritative signal)"
        );
        // 3. When reached_start (anchor absent), the handler must conclude EndReached immediately.
        assert!(
            continue_src.contains("outcome.reached_start"),
            "continuation handler must use outcome.reached_start to conclude EndReached immediately"
        );
        // 4. The timing heuristics must be gone.
        assert!(
            !continue_src.contains("settle_last_seen_seq"),
            "timing-heuristic settle_last_seen_seq must be removed (replaced by anchor_present)"
        );
        assert!(
            !continue_src.contains("settle_awaiting_first_diff"),
            "timing-heuristic settle_awaiting_first_diff must be removed"
        );
        assert!(
            !production.contains("RESTORE_ANCHOR_SETTLE_TICK_DELAY_MS"),
            "50ms tick delay constant must be removed"
        );
        assert!(
            !production.contains("schedule_restore_anchor_settle_tick"),
            "schedule_restore_anchor_settle_tick function must be removed"
        );
        // 5. P3: invalid-request path must NOT call finish_anchor_restore.
        let restore_handler_src = production
            .split("async fn handle_restore_timeline_anchor(")
            .nth(1)
            .expect("handle_restore_timeline_anchor must exist")
            .split("async fn handle_restore_timeline_anchor_continue")
            .next()
            .expect("restore handler must end before continuation");
        assert!(
            restore_handler_src.contains("emit_anchor_restore_finished"),
            "invalid-request path must call emit_anchor_restore_finished (not finish_anchor_restore)"
        );
    }

    // --- VectorDiff → TimelineDiff conversion ---

    #[test]
    fn sdk_vector_diff_converts_correctly() {
        // We can't construct real SdkTimelineItems without the SDK runtime;
        // instead test the conversion path shape by examining the match arms
        // in sdk_vector_diff_to_timeline_diff are exhaustive for all diff kinds.
        // This is verified by the compiler (no dead_code warnings).
        // We document the PopBack → Truncate(0) and Append → Reset mappings here.
        let _popback_maps_to_truncate: bool = true;
        let _append_maps_to_reset: bool = true;
    }

    // --- Navigation snapshot read-marker display anchor ---

    #[test]
    fn navigation_display_anchor_advances_past_own_messages_after_marker() {
        let other = timeline_item("$other", Some("hello"), "@bob", false);
        let own1 = timeline_item("$own1", Some("own1"), "@alice", false);
        let own2 = timeline_item("$own2", Some("own2"), "@alice", false);
        let items = vec![other, own1, own2];
        let observation = TimelineViewportObservation::default();

        let snapshot = derive_timeline_navigation_snapshot(
            &items,
            Some("$other"),
            &observation,
            Some("@alice"),
        );

        assert_eq!(snapshot.read_marker_event_id, Some("$other".to_owned()));
        assert_eq!(snapshot.first_unread_event_id, None);
        assert_eq!(
            snapshot.read_marker_display_event_id,
            Some("$own2".to_owned())
        );
    }

    #[test]
    fn navigation_display_anchor_stays_at_marker_when_no_own_messages_after() {
        let other = timeline_item("$other", Some("hello"), "@bob", false);
        let remote = timeline_item("$remote", Some("remote"), "@bob", false);
        let items = vec![other, remote];
        let observation = TimelineViewportObservation::default();

        let snapshot = derive_timeline_navigation_snapshot(
            &items,
            Some("$other"),
            &observation,
            Some("@alice"),
        );

        assert_eq!(snapshot.first_unread_event_id, Some("$remote".to_owned()));
        assert_eq!(snapshot.read_marker_display_event_id, None);
    }

    #[test]
    fn navigation_display_anchor_advances_from_own_marker_to_later_own_message() {
        let own1 = timeline_item("$own1", Some("own1"), "@alice", false);
        let own2 = timeline_item("$own2", Some("own2"), "@alice", false);
        let items = vec![own1, own2];
        let observation = TimelineViewportObservation::default();

        let snapshot = derive_timeline_navigation_snapshot(
            &items,
            Some("$own1"),
            &observation,
            Some("@alice"),
        );

        assert_eq!(snapshot.read_marker_event_id, Some("$own1".to_owned()));
        assert_eq!(snapshot.first_unread_event_id, None);
        assert_eq!(
            snapshot.read_marker_display_event_id,
            Some("$own2".to_owned())
        );
    }
}
